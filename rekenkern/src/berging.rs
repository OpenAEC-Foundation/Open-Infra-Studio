//! Storage facilities (issue #64): wadis, ponds and crate systems with a
//! stage–storage curve, outlets, and level-pool routing of a storm.
//!
//! What: a [`StageStorage`] curve (level → plan area → storage) from a table,
//! from an area function (e.g. sampled from a TIN or grid), or from a
//! standard shape; [`Outlet`]s (orifice / *knijp*, weir / *drempel*,
//! infiltration); and [`route`], which routes an inflow series through the
//! facility and reports levels, storage, outflow per outlet, overflow and
//! drain time.
//!
//! Why this scheme: level-pool routing with the storage-indication
//! (modified Puls) equation, Chow, Maidment & Mays, *Applied Hydrology*
//! (1988), §8.2:
//!
//! S(h₁) + Δt/2·O(h₁) = S(h₀) + I·Δt − Δt/2·O(h₀)
//!
//! Instead of interpolating an S–O table, the new level is found by bisection
//! on the monotone left-hand side, so any mix of outlets works and the step
//! is unconditionally stable. The outflow volume per outlet in a step is the
//! trapezoid Δt/2·(O₀ + O₁); the new storage is then taken from the volume
//! balance, so inflow = Δstorage + Σ outflow holds to rounding.
//!
//! Coefficients (Stichting RIONED, *Leidraad Riolering* C2100, August 2004,
//! appendix 2): g = 9.81 m/s²; weir (§2.4, *volkomen korte overlaat*)
//! Q = m·b·⅔·√(⅔·g)·H^1.5 ≈ 1.7·m·b·H^1.5 with default m = 0.8; orifice
//! (§2.5, *doorlaat*) contraction coefficient μ = 0.6 for a round opening,
//! used here as Q = μ·a·√(2gH) for free outflow.

use serde::{Deserialize, Serialize};

use crate::buien::Hyetograph;
use crate::P2;

/// Gravitational acceleration (m/s²), as in Leidraad Riolering C2100.
pub const G: f64 = 9.81;
/// Default orifice (*knijp*) discharge coefficient μ for a round opening,
/// Leidraad Riolering C2100 appendix 2 §2.5.
pub const ORIFICE_CD: f64 = 0.6;
/// Default weir discharge coefficient m for a free short weir, Leidraad
/// Riolering C2100 appendix 2 §2.4.
pub const WEIR_M: f64 = 0.8;
/// Default void ratio of infiltration/storage crates: 0.95. Manufacturer data
/// give 95–96 % (Wavin Q-Bic Plus datasheet), 95.5 % (Pipelife Stormbox),
/// ≥ 95 % (DYKA Rainbox). Adjust to the product used.
pub const CRATE_POROSITY: f64 = 0.95;
/// Number of intervals used when a curve is sampled from a shape or function.
pub const SAMPLE_STEPS: usize = 100;

/// How volume is integrated between two (level, area) points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolumeMethod {
    /// Area linear in level: ΔV = Δh·(A₁ + A₂)/2.
    AverageEndArea,
    /// √area linear in level (frustum of similar shapes):
    /// ΔV = Δh/3·(A₁ + A₂ + √(A₁A₂)).
    Conic,
}

/// Stage–storage curve: plan area against level, with cumulative volume.
///
/// Areas are gross plan areas; storage = porosity × gross volume, so a crate
/// or gravel body keeps its footprint for infiltration. Below the bottom the
/// storage is 0; above the top the curve continues with vertical walls at the
/// top area (reported as `above_top` by [`route`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageStorage {
    /// Levels (m), strictly increasing; the first is the bottom.
    pub levels: Vec<f64>,
    /// Gross plan area at each level (m²).
    pub areas: Vec<f64>,
    /// Cumulative gross volume from the bottom at each level (m³).
    pub volumes: Vec<f64>,
    /// Integration and interpolation method between points.
    pub method: VolumeMethod,
    /// Fraction of the gross volume that stores water (1 for open water).
    pub porosity: f64,
}

impl StageStorage {
    /// A curve from (level, area) pairs; levels strictly increasing, areas ≥ 0.
    pub fn from_table(pairs: &[(f64, f64)], method: VolumeMethod) -> Result<Self, String> {
        if pairs.len() < 2 {
            return Err("a stage–storage curve needs at least two levels".into());
        }
        let mut volumes = vec![0.0];
        for w in pairs.windows(2) {
            let ((h1, a1), (h2, a2)) = (w[0], w[1]);
            if !(h1.is_finite() && h2.is_finite() && h2 > h1) {
                return Err(format!("levels must increase strictly ({h1} → {h2})"));
            }
            if !(a1.is_finite() && a2.is_finite() && a1 >= 0.0 && a2 >= 0.0) {
                return Err("areas must be ≥ 0".into());
            }
            let dv = match method {
                VolumeMethod::AverageEndArea => (h2 - h1) * (a1 + a2) / 2.0,
                VolumeMethod::Conic => (h2 - h1) / 3.0 * (a1 + a2 + (a1 * a2).sqrt()),
            };
            volumes.push(volumes.last().unwrap() + dv);
        }
        Ok(StageStorage {
            levels: pairs.iter().map(|p| p.0).collect(),
            areas: pairs.iter().map(|p| p.1).collect(),
            volumes,
            method,
            porosity: 1.0,
        })
    }

    /// A curve sampled from an area function (e.g. a TIN or grid query that
    /// returns the flooded plan area below a level) in `steps` equal intervals.
    pub fn from_area_fn<F: Fn(f64) -> f64>(
        area: F,
        bottom: f64,
        top: f64,
        steps: usize,
        method: VolumeMethod,
    ) -> Result<Self, String> {
        if top.partial_cmp(&bottom) != Some(std::cmp::Ordering::Greater) || steps == 0 {
            return Err("top must be above bottom and steps ≥ 1".into());
        }
        let pairs: Vec<(f64, f64)> = (0..=steps)
            .map(|i| {
                let h = bottom + (top - bottom) * i as f64 / steps as f64;
                (h, area(h))
            })
            .collect();
        StageStorage::from_table(&pairs, method)
    }

    /// A prismatic tank: constant `area` from `bottom` over `height`.
    pub fn prismatic(bottom: f64, area: f64, height: f64) -> Result<Self, String> {
        if !(area > 0.0 && height > 0.0) {
            return Err("area and height must be > 0".into());
        }
        StageStorage::from_table(
            &[(bottom, area), (bottom + height, area)],
            VolumeMethod::AverageEndArea,
        )
    }

    /// A crate system (*kratten*): footprint × height with a void ratio
    /// (`CRATE_POROSITY` is the usual default).
    pub fn crates(bottom: f64, footprint: f64, height: f64, porosity: f64) -> Result<Self, String> {
        if !(porosity > 0.0 && porosity <= 1.0) {
            return Err("porosity must be in (0, 1]".into());
        }
        let mut s = StageStorage::prismatic(bottom, footprint, height)?;
        s.porosity = porosity;
        Ok(s)
    }

    /// A wadi or pond with a polygonal bottom and side slopes 1:`slope_n`
    /// (horizontal n per vertical 1).
    ///
    /// The plan area at height z is the mitred outward offset of the bottom
    /// polygon by r = n·z: A(z) = A₀ + P₀·r + K·r², with K = Σ tan(θᵢ/2) over
    /// the turning angles θᵢ (K = 4 for a rectangle). Exact for convex bottoms
    /// and for concave ones as long as the offset does not self-intersect.
    /// The curve is sampled in `SAMPLE_STEPS` intervals with average-end-area
    /// (relative volume error below 1e-4 for usual slopes).
    pub fn basin(
        bottom_polygon: &[P2],
        bottom: f64,
        depth: f64,
        slope_n: f64,
    ) -> Result<Self, String> {
        let n = bottom_polygon.len();
        if n < 3 {
            return Err("a bottom polygon needs at least three points".into());
        }
        if !(depth > 0.0 && slope_n >= 0.0) {
            return Err("depth must be > 0 and the side slope ≥ 0".into());
        }
        let mut area2 = 0.0;
        let mut perimeter = 0.0;
        for i in 0..n {
            let (p, q) = (bottom_polygon[i], bottom_polygon[(i + 1) % n]);
            area2 += p[0] * q[1] - q[0] * p[1];
            perimeter += ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
        }
        if area2.abs() < 1e-12 {
            return Err("the bottom polygon has no area".into());
        }
        let orient = area2.signum();
        let mut k = 0.0;
        for i in 0..n {
            let a = bottom_polygon[(i + n - 1) % n];
            let b = bottom_polygon[i];
            let c = bottom_polygon[(i + 1) % n];
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let turn = (orient * (u[0] * v[1] - u[1] * v[0])).atan2(u[0] * v[0] + u[1] * v[1]);
            k += (turn / 2.0).tan();
        }
        let a0 = area2.abs() / 2.0;
        StageStorage::from_area_fn(
            |h| {
                let r = slope_n * (h - bottom);
                a0 + perimeter * r + k * r * r
            },
            bottom,
            bottom + depth,
            SAMPLE_STEPS,
            VolumeMethod::AverageEndArea,
        )
    }

    /// A rectangular wadi or pond: bottom `length` × `width`, side slopes 1:n.
    pub fn basin_rect(
        bottom: f64,
        length: f64,
        width: f64,
        depth: f64,
        slope_n: f64,
    ) -> Result<Self, String> {
        StageStorage::basin(
            &[[0.0, 0.0], [length, 0.0], [length, width], [0.0, width]],
            bottom,
            depth,
            slope_n,
        )
    }

    /// Bottom level (m).
    pub fn bottom(&self) -> f64 {
        self.levels[0]
    }

    /// Top level of the curve (m).
    pub fn top(&self) -> f64 {
        *self.levels.last().unwrap()
    }

    /// Storage at the top of the curve (m³).
    pub fn max_storage(&self) -> f64 {
        self.porosity * self.volumes.last().unwrap()
    }

    /// Interval index and fraction t ∈ [0, 1] of level `h` inside the curve.
    fn locate(&self, h: f64) -> (usize, f64) {
        let i = self
            .levels
            .partition_point(|&l| l <= h)
            .clamp(1, self.levels.len() - 1)
            - 1;
        let t = (h - self.levels[i]) / (self.levels[i + 1] - self.levels[i]);
        (i, t.clamp(0.0, 1.0))
    }

    /// Gross plan area (m²) at level `h`; 0 below the bottom, top area above.
    pub fn area(&self, h: f64) -> f64 {
        if h < self.bottom() {
            return 0.0;
        }
        if h >= self.top() {
            return *self.areas.last().unwrap();
        }
        let (i, t) = self.locate(h);
        let (a1, a2) = (self.areas[i], self.areas[i + 1]);
        match self.method {
            VolumeMethod::AverageEndArea => a1 + (a2 - a1) * t,
            VolumeMethod::Conic => {
                let s = a1.sqrt() + (a2.sqrt() - a1.sqrt()) * t;
                s * s
            }
        }
    }

    /// Storage (m³) at level `h`, consistent with the interpolated area.
    pub fn storage(&self, h: f64) -> f64 {
        if h <= self.bottom() {
            return 0.0;
        }
        let gross = if h >= self.top() {
            self.volumes.last().unwrap() + self.areas.last().unwrap() * (h - self.top())
        } else {
            let (i, t) = self.locate(h);
            let dh = self.levels[i + 1] - self.levels[i];
            let (a1, a2) = (self.areas[i], self.areas[i + 1]);
            self.volumes[i]
                + dh * match self.method {
                    VolumeMethod::AverageEndArea => a1 * t + (a2 - a1) * t * t / 2.0,
                    VolumeMethod::Conic => {
                        let (s1, s2) = (a1.sqrt(), a2.sqrt());
                        s1 * s1 * t + s1 * (s2 - s1) * t * t + (s2 - s1).powi(2) * t.powi(3) / 3.0
                    }
                }
        };
        self.porosity * gross
    }

    /// Level (m) at which the storage equals `storage` (inverse of [`Self::storage`]).
    pub fn level(&self, storage: f64) -> f64 {
        if storage <= 0.0 {
            return self.bottom();
        }
        let gross = storage / self.porosity;
        let v_top = *self.volumes.last().unwrap();
        if gross >= v_top {
            let a_top = *self.areas.last().unwrap();
            return if a_top > 0.0 {
                self.top() + (gross - v_top) / a_top
            } else {
                self.top()
            };
        }
        let i = self
            .volumes
            .partition_point(|&v| v <= gross)
            .clamp(1, self.volumes.len() - 1)
            - 1;
        let (mut lo, mut hi) = (self.levels[i], self.levels[i + 1]);
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if self.storage(mid) < storage {
                lo = mid;
            } else {
                hi = mid;
            }
            if hi - lo < 1e-12 {
                break;
            }
        }
        0.5 * (lo + hi)
    }
}

/// Which area infiltrates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfiltrationArea {
    /// Plan area at the water level (bottom plus the flooded side slopes in
    /// projection).
    Wetted,
    /// Bottom area only (plan area at the bottom level).
    Bottom,
}

/// An outlet of a storage facility. All outlets discharge only while the
/// facility holds water (level above the bottom).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Outlet {
    /// Round orifice (*knijp*) with free outflow: Q = μ·a·√(2g(h − h_c)),
    /// h_c = invert + d/2. When only partly submerged it acts as a weir,
    /// Q = Q(full)·f^1.5 with f the submerged fraction of the diameter, which
    /// is continuous at f = 1 (a simplification of the circular segment).
    Orifice {
        /// Diameter (m).
        diameter: f64,
        /// Invert level, the bottom of the opening (m).
        invert: f64,
        /// Discharge coefficient μ (default `ORIFICE_CD`).
        cd: f64,
    },
    /// Free short weir / overflow (*drempel*): Q = m·b·⅔·√(⅔g)·H^1.5, H the
    /// level above the crest. Counted as overflow in the summary.
    Weir {
        /// Crest level (m).
        crest: f64,
        /// Crest width (m).
        width: f64,
        /// Discharge coefficient m (default `WEIR_M`).
        m: f64,
    },
    /// Infiltration at a constant rate over an area: Q = rate·A.
    Infiltration {
        /// Infiltration rate (mm/h); a design value from a soil test, with
        /// any safety factor already applied.
        rate_mm_h: f64,
        /// Area that infiltrates.
        area: InfiltrationArea,
    },
}

impl Outlet {
    /// An orifice with the default discharge coefficient.
    pub fn orifice(diameter: f64, invert: f64) -> Outlet {
        Outlet::Orifice {
            diameter,
            invert,
            cd: ORIFICE_CD,
        }
    }

    /// A weir with the default discharge coefficient.
    pub fn weir(crest: f64, width: f64) -> Outlet {
        Outlet::Weir {
            crest,
            width,
            m: WEIR_M,
        }
    }

    /// Infiltration at `rate_mm_h` over `area`.
    pub fn infiltration(rate_mm_h: f64, area: InfiltrationArea) -> Outlet {
        Outlet::Infiltration { rate_mm_h, area }
    }

    /// Whether this outlet counts as overflow (a weir).
    pub fn is_overflow(&self) -> bool {
        matches!(self, Outlet::Weir { .. })
    }

    fn validate(&self) -> Result<(), String> {
        let ok = match *self {
            Outlet::Orifice {
                diameter,
                invert,
                cd,
            } => diameter > 0.0 && cd > 0.0 && invert.is_finite(),
            Outlet::Weir { crest, width, m } => width > 0.0 && m > 0.0 && crest.is_finite(),
            Outlet::Infiltration { rate_mm_h, .. } => rate_mm_h >= 0.0 && rate_mm_h.is_finite(),
        };
        if ok {
            Ok(())
        } else {
            Err(format!("invalid outlet {self:?}"))
        }
    }

    /// Discharge (m³/s) at water level `h` of facility `curve`.
    pub fn discharge(&self, h: f64, curve: &StageStorage) -> f64 {
        if h <= curve.bottom() {
            return 0.0;
        }
        match *self {
            Outlet::Orifice {
                diameter,
                invert,
                cd,
            } => {
                let hu = h - invert;
                if hu <= 0.0 {
                    return 0.0;
                }
                let a = std::f64::consts::PI * diameter * diameter / 4.0;
                if hu >= diameter {
                    cd * a * (2.0 * G * (hu - diameter / 2.0)).sqrt()
                } else {
                    cd * a * (G * diameter).sqrt() * (hu / diameter).powf(1.5)
                }
            }
            Outlet::Weir { crest, width, m } => {
                let head = h - crest;
                if head <= 0.0 {
                    0.0
                } else {
                    m * width * (2.0 / 3.0) * (2.0 / 3.0 * G).sqrt() * head.powf(1.5)
                }
            }
            Outlet::Infiltration { rate_mm_h, area } => {
                let a = match area {
                    InfiltrationArea::Wetted => curve.area(h),
                    InfiltrationArea::Bottom => curve.areas[0],
                };
                rate_mm_h / 3.6e6 * a
            }
        }
    }
}

/// An inflow series: mean inflow (m³/s) per interval of `dt_s`, from t = 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inflow {
    /// Interval length (s); also the routing time step.
    pub dt_s: f64,
    /// Mean inflow per interval (m³/s).
    pub q: Vec<f64>,
}

impl Inflow {
    /// Total inflow volume (m³).
    pub fn volume(&self) -> f64 {
        self.q.iter().sum::<f64>() * self.dt_s
    }
}

/// Inflow from a storm on a contributing area: Q = C·A·i, with `area_m2`
/// the contributing (e.g. paved, connected) area and `runoff_coefficient` C
/// ∈ [0, 1] the fraction that reaches the facility. No routing delay over the
/// surface (conservative for storage).
pub fn inflow_from_storm(
    storm: &Hyetograph,
    area_m2: f64,
    runoff_coefficient: f64,
    dt_s: f64,
) -> Result<Inflow, String> {
    if !(area_m2 >= 0.0 && (0.0..=1.0).contains(&runoff_coefficient)) {
        return Err("area must be ≥ 0 and the runoff coefficient in [0, 1]".into());
    }
    let q = storm
        .resample(dt_s)?
        .into_iter()
        .map(|i| runoff_coefficient * area_m2 * i / 3.6e6)
        .collect();
    Ok(Inflow { dt_s, q })
}

/// Options for [`route`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingOptions {
    /// Stop at this time even if not drained (s). Default 7 days.
    pub max_time_s: f64,
    /// The facility counts as empty below this fraction of the maximum
    /// storage (for the drain time). Default 0.01, an adjustable choice.
    pub empty_fraction: f64,
    /// Starting level (m); `None` starts empty.
    pub initial_level: Option<f64>,
}

impl Default for RoutingOptions {
    fn default() -> Self {
        RoutingOptions {
            max_time_s: 7.0 * 86400.0,
            empty_fraction: 0.01,
            initial_level: None,
        }
    }
}

/// Totals and key values of a routing run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingSummary {
    /// Highest level reached (m).
    pub max_level: f64,
    /// Highest storage (m³).
    pub max_storage: f64,
    /// Time of the highest storage (s).
    pub time_of_max_s: f64,
    /// Storage at the start (m³).
    pub initial_storage: f64,
    /// Storage at the end of the run (m³).
    pub final_storage: f64,
    /// Total inflow volume (m³).
    pub inflow_volume: f64,
    /// Outflow volume per outlet, in the order given (m³).
    pub outlet_volumes: Vec<f64>,
    /// Whether any weir discharged.
    pub overflows: bool,
    /// Volume over the weirs (m³).
    pub overflow_volume: f64,
    /// Whether the level rose above the top of the curve.
    pub above_top: bool,
    /// Time from the peak until storage fell below `empty_fraction` of the
    /// peak (s); `None` if that did not happen within `max_time_s`.
    pub drain_time_s: Option<f64>,
    /// (inflow + initial − final − Σ outflow) / (inflow + initial).
    pub mass_balance_error: f64,
}

/// Time series of a routing run, sampled every time step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingResult {
    /// Sample times (s).
    pub time_s: Vec<f64>,
    /// Water level (m).
    pub level: Vec<f64>,
    /// Storage (m³).
    pub storage: Vec<f64>,
    /// Mean inflow over the step starting at the sample (m³/s).
    pub inflow: Vec<f64>,
    /// Discharge per outlet at the sample, `outflow[outlet][sample]` (m³/s).
    pub outflow: Vec<Vec<f64>>,
    /// Totals and key values.
    pub summary: RoutingSummary,
}

/// Level-pool routing of `inflow` through a facility with `outlets`.
///
/// Runs with the inflow's time step until the inflow has ended and the
/// facility is empty (see `empty_fraction`), or until `max_time_s`.
pub fn route(
    curve: &StageStorage,
    outlets: &[Outlet],
    inflow: &Inflow,
    opts: &RoutingOptions,
) -> Result<RoutingResult, String> {
    let dt = inflow.dt_s;
    if !(dt.is_finite() && dt > 0.0) {
        return Err("inflow time step must be > 0".into());
    }
    if inflow.q.iter().any(|q| !q.is_finite() || *q < 0.0) {
        return Err("inflow must be finite and ≥ 0".into());
    }
    if !(0.0..1.0).contains(&opts.empty_fraction)
        || opts.max_time_s.is_nan()
        || opts.max_time_s <= 0.0
    {
        return Err("empty_fraction must be in [0, 1) and max_time_s > 0".into());
    }
    for o in outlets {
        o.validate()?;
    }
    let outflows = |h: f64| -> Vec<f64> { outlets.iter().map(|o| o.discharge(h, curve)).collect() };
    let lhs = |h: f64| curve.storage(h) + 0.5 * dt * outflows(h).iter().sum::<f64>();

    let mut s = opts.initial_level.map_or(0.0, |h| curve.storage(h));
    let mut h = curve.level(s);
    let initial_storage = s;
    let mut res = RoutingResult {
        time_s: Vec::new(),
        level: Vec::new(),
        storage: Vec::new(),
        inflow: Vec::new(),
        outflow: vec![Vec::new(); outlets.len()],
        summary: RoutingSummary {
            max_level: h,
            max_storage: s,
            time_of_max_s: 0.0,
            initial_storage,
            final_storage: 0.0,
            inflow_volume: inflow.volume(),
            outlet_volumes: vec![0.0; outlets.len()],
            overflows: false,
            overflow_volume: 0.0,
            above_top: false,
            drain_time_s: None,
            mass_balance_error: 0.0,
        },
    };
    let max_steps = (opts.max_time_s / dt).ceil() as usize;
    let mut o_now = outflows(h);
    let mut i_peak = 0;
    for k in 0..=max_steps {
        let t = k as f64 * dt;
        let q_in = inflow.q.get(k).copied().unwrap_or(0.0);
        res.time_s.push(t);
        res.level.push(h);
        res.storage.push(s);
        res.inflow.push(q_in);
        for (series, o) in res.outflow.iter_mut().zip(&o_now) {
            series.push(*o);
        }
        let sm = &mut res.summary;
        if s > sm.max_storage {
            sm.max_storage = s;
            sm.time_of_max_s = t;
            i_peak = k;
        }
        sm.max_level = sm.max_level.max(h);
        sm.above_top |= h > curve.top() + 1e-9;
        let empty = s <= opts.empty_fraction * sm.max_storage;
        if (k >= inflow.q.len() && empty) || k == max_steps {
            break;
        }

        // One storage-indication step.
        let v_in = q_in * dt;
        let rhs = s + v_in - 0.5 * dt * o_now.iter().sum::<f64>();
        let mut vols: Vec<f64>;
        if rhs <= 0.0 {
            vols = o_now.iter().map(|o| 0.5 * dt * o).collect();
        } else {
            let (mut lo, mut hi) = (curve.bottom(), curve.top().max(h) + 1.0);
            while lhs(hi) < rhs {
                hi = curve.bottom() + 2.0 * (hi - curve.bottom());
                if hi - curve.bottom() > 1e6 {
                    return Err("routing: level diverges; check the curve and inflow".into());
                }
            }
            for _ in 0..200 {
                let mid = 0.5 * (lo + hi);
                if lhs(mid) < rhs {
                    lo = mid;
                } else {
                    hi = mid;
                }
                if hi - lo < 1e-12 {
                    break;
                }
            }
            let o_new = outflows(hi);
            vols = o_now
                .iter()
                .zip(&o_new)
                .map(|(a, b)| 0.5 * dt * (a + b))
                .collect();
        }
        // Volume balance; scale outflow down if it would overdraw the storage.
        let available = s + v_in;
        let total: f64 = vols.iter().sum();
        let s_new = if total > available {
            let f = if total > 0.0 { available / total } else { 0.0 };
            vols.iter_mut().for_each(|v| *v *= f);
            0.0
        } else {
            available - total
        };
        for (acc, v) in res.summary.outlet_volumes.iter_mut().zip(&vols) {
            *acc += v;
        }
        s = s_new;
        h = curve.level(s);
        o_now = outflows(h);
    }

    // Drain time: from the peak until storage ≤ empty_fraction × peak.
    let sm = &mut res.summary;
    let threshold = opts.empty_fraction * sm.max_storage;
    if sm.max_storage > 0.0 {
        for i in i_peak + 1..res.storage.len() {
            if res.storage[i] <= threshold {
                let (s0, s1) = (res.storage[i - 1], res.storage[i]);
                let frac = if s0 > s1 {
                    (s0 - threshold) / (s0 - s1)
                } else {
                    1.0
                };
                let t_cross = res.time_s[i - 1] + frac * (res.time_s[i] - res.time_s[i - 1]);
                sm.drain_time_s = Some(t_cross - sm.time_of_max_s);
                break;
            }
        }
    }
    sm.final_storage = s;
    sm.overflow_volume = outlets
        .iter()
        .zip(&sm.outlet_volumes)
        .filter(|(o, _)| o.is_overflow())
        .map(|(_, v)| v)
        .sum();
    sm.overflows = sm.overflow_volume > 0.0;
    let supplied = sm.inflow_volume + sm.initial_storage;
    let out: f64 = sm.outlet_volumes.iter().sum();
    sm.mass_balance_error = if supplied > 0.0 {
        (supplied - sm.final_storage - out) / supplied
    } else {
        0.0
    };
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buien::{bui08, uniform_block, StresstestStorm};

    fn rel(a: f64, b: f64) -> f64 {
        (a - b).abs() / b.abs().max(1e-300)
    }

    #[test]
    fn prismatic_tank_without_outlet_stores_inflow() {
        let tank = StageStorage::prismatic(0.0, 50.0, 2.0).unwrap();
        let q = inflow_from_storm(&bui08(), 1000.0, 1.0, 60.0).unwrap();
        assert!(rel(q.volume(), 19.8) < 1e-12);
        let r = route(
            &tank,
            &[],
            &q,
            &RoutingOptions {
                max_time_s: 7200.0,
                ..Default::default()
            },
        )
        .unwrap();
        let sm = &r.summary;
        assert!(rel(sm.final_storage, 19.8) < 1e-9);
        assert!(rel(sm.max_level, 19.8 / 50.0) < 1e-9);
        assert!(!sm.overflows);
        assert_eq!(sm.drain_time_s, None);
        assert!(sm.mass_balance_error.abs() < 1e-12);
    }

    #[test]
    fn orifice_drain_time_matches_torricelli() {
        // Prismatic tank drained through a small orifice at the bottom, from
        // 1.0 m to the 10 % threshold (0.1 m), both above the opening:
        // t = A/(μa)·(√(2h₀/g) − √(2h₁/g)), heads to the orifice centre.
        let (area, d, cd) = (10.0, 0.05, ORIFICE_CD);
        let tank = StageStorage::prismatic(0.0, area, 2.0).unwrap();
        let outlets = [Outlet::orifice(d, 0.0)];
        let inflow = Inflow {
            dt_s: 0.5,
            q: vec![],
        };
        let opts = RoutingOptions {
            initial_level: Some(1.0),
            empty_fraction: 0.1,
            ..Default::default()
        };
        let r = route(&tank, &outlets, &inflow, &opts).unwrap();
        let a = std::f64::consts::PI * d * d / 4.0;
        let (h0, h1) = (1.0 - d / 2.0, 0.1 - d / 2.0);
        let t = area / (cd * a) * ((2.0 * h0 / G).sqrt() - (2.0 * h1 / G).sqrt());
        let got = r.summary.drain_time_s.unwrap();
        assert!(rel(got, t) < 1e-3, "drain time {got} s, Torricelli {t} s");
        assert!(r.summary.mass_balance_error.abs() < 1e-12);
    }

    #[test]
    fn wadi_mass_balance_closes() {
        // 30 × 10 m wadi, 0.5 m deep, slopes 1:3, orifice, overflow, infiltration;
        // stress-test storm 70 mm/h on 2000 m² paved area.
        let wadi = StageStorage::basin_rect(10.0, 30.0, 10.0, 0.5, 3.0).unwrap();
        let outlets = [
            Outlet::orifice(0.06, 10.05),
            Outlet::weir(10.30, 2.0),
            Outlet::infiltration(20.0, InfiltrationArea::Wetted),
        ];
        let storm = StresstestStorm::Local1hT100.hyetograph();
        let q = inflow_from_storm(&storm, 2000.0, 1.0, 30.0).unwrap();
        let r = route(&wadi, &outlets, &q, &RoutingOptions::default()).unwrap();
        let sm = &r.summary;
        let out: f64 = sm.outlet_volumes.iter().sum();
        let balance = (sm.inflow_volume - sm.final_storage - out) / sm.inflow_volume;
        assert!(balance.abs() < 1e-3, "mass balance error {balance}");
        assert!(sm.mass_balance_error.abs() < 1e-9);
        assert!(rel(sm.inflow_volume, 140.0) < 1e-12);
        assert!(sm.overflows && sm.overflow_volume > 0.0);
        assert!(sm.max_level > 10.30 && sm.max_level < 10.5);
        assert!(sm.drain_time_s.is_some());
        assert!(sm.outlet_volumes.iter().all(|v| *v > 0.0));
    }

    #[test]
    fn crate_porosity_and_infiltration_bottom() {
        let c = StageStorage::crates(-1.5, 20.0, 0.6, CRATE_POROSITY).unwrap();
        assert!(rel(c.max_storage(), 20.0 * 0.6 * 0.95) < 1e-12);
        assert!(rel(c.level(5.7), -1.5 + 0.3) < 1e-9);
        // 10 mm/h on 20 m² = 0.2 m³/h
        let inf = Outlet::infiltration(10.0, InfiltrationArea::Bottom);
        assert!(rel(inf.discharge(-1.4, &c), 0.2 / 3600.0) < 1e-12);
        assert_eq!(inf.discharge(-1.5, &c), 0.0);
    }

    #[test]
    fn shapes_match_analytic_volumes() {
        // Rectangular basin: V = LWh + (L+W)·n·h² + 4/3·n²·h³.
        let (l, w, d, n): (f64, f64, f64, f64) = (30.0, 10.0, 0.5, 3.0);
        let v = l * w * d + (l + w) * n * d * d + 4.0 / 3.0 * n * n * d.powi(3);
        let b = StageStorage::basin_rect(0.0, l, w, d, n).unwrap();
        assert!(rel(b.max_storage(), v) < 1e-4);
        assert!(rel(b.area(d), (l + 2.0 * n * d) * (w + 2.0 * n * d)) < 1e-12);
        // Square pyramid frustum: the conic method is exact.
        let f = StageStorage::from_table(&[(0.0, 4.0), (1.0, 16.0)], VolumeMethod::Conic).unwrap();
        assert!(rel(f.max_storage(), 1.0 / 3.0 * (4.0 + 16.0 + 8.0)) < 1e-12);
        assert!(rel(f.area(0.5), 9.0) < 1e-12);
        assert!(rel(f.storage(0.5), 0.5 / 3.0 * (4.0 + 9.0 + 6.0)) < 1e-12);
        // Level is the inverse of storage, also above the top.
        for h in [0.1, 0.37, 0.99, 1.4] {
            assert!((f.level(f.storage(h)) - h).abs() < 1e-9);
        }
        // From a closure: a cone of radius r = z (√A linear in z).
        let cone = StageStorage::from_area_fn(
            |z| std::f64::consts::PI * z * z,
            0.0,
            2.0,
            4,
            VolumeMethod::Conic,
        )
        .unwrap();
        assert!(rel(cone.max_storage(), std::f64::consts::PI * 8.0 / 3.0) < 1e-12);
    }

    #[test]
    fn outlets_formulas() {
        let tank = StageStorage::prismatic(0.0, 1.0, 5.0).unwrap();
        // weir: 1.7·m·b·H^1.5 (C2100)
        let wq = Outlet::weir(1.0, 2.0).discharge(1.5, &tank);
        assert!(
            rel(
                wq,
                0.8 * 2.0 * (2.0 / 3.0) * (2.0 / 3.0 * G).sqrt() * 0.5f64.powf(1.5)
            ) < 1e-12
        );
        assert!(rel((2.0 / 3.0) * (2.0 / 3.0 * G).sqrt(), 1.705) < 1e-3);
        // orifice continuous at the top of the opening
        let o = Outlet::orifice(0.1, 0.5);
        let (below, above) = (
            o.discharge(0.6 - 1e-9, &tank),
            o.discharge(0.6 + 1e-9, &tank),
        );
        assert!((below - above).abs() < 1e-8);
        assert_eq!(o.discharge(0.4, &tank), 0.0);
    }

    #[test]
    fn block_storm_inflow_volume() {
        let q = inflow_from_storm(&uniform_block(90.0, 60.0).unwrap(), 500.0, 0.8, 120.0).unwrap();
        assert!(rel(q.volume(), 0.09 * 500.0 * 0.8) < 1e-12);
        assert!(inflow_from_storm(&bui08(), 100.0, 1.5, 60.0).is_err());
    }
}
