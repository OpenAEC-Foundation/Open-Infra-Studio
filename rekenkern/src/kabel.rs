//! Cable routes, issue #72: underground cover depth and joints, overhead
//! conductor catenaries, sag–tension change of state and clearances.
//!
//! Underground. Cover (*dekking*) = ground level − top of cable. Default
//! minimum covers follow Stedin, *Criteria kabels en leidingen*, Tabel 1
//! "Dekking van kabels en leidingen", which refers to NEN 7171-1 and
//! NPR 7171-2: low voltage (LS) 60 cm, medium voltage (MS) 80 cm, gas 80 cm,
//! operator data cables 80 cm. These are operator defaults; the municipality
//! or operator may require other values (e.g. ≥ 1.00 m under watercourses,
//! Raalte, *Nadere regels kabels en leidingen*), so pass your own minimum.
//! Joints (*moffen*): a cable section cannot exceed the usable drum length;
//! without preferred positions the count is `ceil(L / drum) − 1`.
//!
//! Overhead. A conductor of weight `w` (N/m) under horizontal tension `H` (N)
//! hangs as a catenary with parameter `a = H / w`:
//! `z(x) = z₀ + a·(cosh((x − x₀)/a) − 1)`. For a level span the sag is
//! `a·(cosh(L/2a) − 1) ≈ w·L²/(8H)`. Temperature and load changes follow the
//! parabolic change-of-state equation (Kiessling, Nefzger, Nolasco & Kaintzyk,
//! *Overhead Power Lines*, Springer 2003, sag–tension chapter):
//! `w₂²L²/(24H₂²) − w₁²L²/(24H₁²) = α(θ₂ − θ₁) + (H₂ − H₁)/(E·A)`,
//! valid for the (ruling) span `L` with small sags. EN 50341-1 requires sag
//! and clearance checks for its load cases; its clearance values depend on
//! voltage and national annex (NL: EN 50341-2-15) and are **not** built in:
//! pass the minimum clearance yourself.

use crate::{P2, P3};
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------ underground

/// Cable or pipe class with its default minimum cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CableClass {
    LowVoltage,
    MediumVoltage,
    Gas,
    Data,
}

impl CableClass {
    /// Default minimum cover in m (Stedin Tabel 1, after NEN 7171-1).
    pub fn min_cover_nl(self) -> f64 {
        match self {
            CableClass::LowVoltage => 0.60,
            CableClass::MediumVoltage | CableClass::Gas | CableClass::Data => 0.80,
        }
    }
}

/// Cover at one station of the route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverSample {
    /// Distance along the route in plan, m.
    pub station: f64,
    /// Cable axis point.
    pub point: P3,
    pub ground_z: f64,
    /// Ground − top of cable, m.
    pub cover: f64,
}

/// A stretch where the cover is below the minimum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoverViolation {
    pub from_station: f64,
    pub to_station: f64,
    pub least_cover: f64,
}

fn d2(a: P2, b: P2) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}
fn d3(a: P3, b: P3) -> f64 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
}
fn lerp3(a: P3, b: P3, t: f64) -> P3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Plan length of a 3D or 2D route.
pub fn plan_length(route: &[P3]) -> f64 {
    route
        .windows(2)
        .map(|w| d2([w[0][0], w[0][1]], [w[1][0], w[1][1]]))
        .sum()
}

/// True 3D length of a route.
pub fn length_3d(route: &[P3]) -> f64 {
    route.windows(2).map(|w| d3(w[0], w[1])).sum()
}

/// Lay a 2D route at a constant depth: cable axis z = ground − depth_to_top
/// − diameter/2, sampled at the vertices and every `step` m.
pub fn drape(
    route: &[P2],
    ground: &dyn Fn(P2) -> Option<f64>,
    depth_to_top: f64,
    diameter: f64,
    step: f64,
) -> Result<Vec<P3>, String> {
    if route.len() < 2 || step <= 0.0 {
        return Err("route needs two points and a positive step".into());
    }
    let mut out = Vec::new();
    for (i, w) in route.windows(2).enumerate() {
        let l = d2(w[0], w[1]);
        let n = (l / step).ceil().max(1.0) as usize;
        let from = if i == 0 { 0 } else { 1 };
        for k in from..=n {
            let t = k as f64 / n as f64;
            let p = [
                w[0][0] + (w[1][0] - w[0][0]) * t,
                w[0][1] + (w[1][1] - w[0][1]) * t,
            ];
            let g = ground(p).ok_or_else(|| format!("no ground level at {:?}", p))?;
            out.push([p[0], p[1], g - depth_to_top - diameter / 2.0]);
        }
    }
    Ok(out)
}

/// Cover along a 3D cable axis, sampled at the vertices and every `step` m
/// of plan length. Points without ground level are skipped.
pub fn cover_along(
    route: &[P3],
    diameter: f64,
    ground: &dyn Fn(P2) -> Option<f64>,
    step: f64,
) -> Vec<CoverSample> {
    let mut out = Vec::new();
    if route.len() < 2 || step <= 0.0 {
        return out;
    }
    let mut st = 0.0;
    for (i, w) in route.windows(2).enumerate() {
        let l = d2([w[0][0], w[0][1]], [w[1][0], w[1][1]]);
        let n = (l / step).ceil().max(1.0) as usize;
        let from = if i == 0 { 0 } else { 1 };
        for k in from..=n {
            let t = k as f64 / n as f64;
            let p = lerp3(w[0], w[1], t);
            if let Some(g) = ground([p[0], p[1]]) {
                out.push(CoverSample {
                    station: st + l * t,
                    point: p,
                    ground_z: g,
                    cover: g - (p[2] + diameter / 2.0),
                });
            }
        }
        st += l;
    }
    out
}

/// Stretches of consecutive samples with cover below `min_cover`.
pub fn cover_violations(samples: &[CoverSample], min_cover: f64) -> Vec<CoverViolation> {
    let mut out: Vec<CoverViolation> = Vec::new();
    let mut open = false;
    for s in samples {
        if s.cover < min_cover {
            if open {
                let v = out.last_mut().unwrap();
                v.to_station = s.station;
                v.least_cover = v.least_cover.min(s.cover);
            } else {
                out.push(CoverViolation {
                    from_station: s.station,
                    to_station: s.station,
                    least_cover: s.cover,
                });
                open = true;
            }
        } else {
            open = false;
        }
    }
    out
}

/// Joint stations along a cable of `length` with usable `drum` length.
/// Each joint goes at the furthest preferred position (e.g. a joint pit)
/// within reach of the previous one, otherwise at the full drum length.
pub fn joint_positions(length: f64, drum: f64, preferred: &[f64]) -> Result<Vec<f64>, String> {
    if drum.is_nan() || drum <= 0.0 || length < 0.0 {
        return Err("drum length must be positive".into());
    }
    let mut pref: Vec<f64> = preferred
        .iter()
        .copied()
        .filter(|s| *s > 0.0 && *s < length)
        .collect();
    pref.sort_by(|a, b| a.total_cmp(b));
    let mut out = Vec::new();
    let mut last = 0.0;
    let eps = 1e-9;
    while length - last > drum + eps {
        let reach = last + drum;
        let next = pref
            .iter()
            .copied()
            .rfind(|s| *s > last + eps && *s <= reach + eps)
            .unwrap_or(reach);
        out.push(next);
        last = next;
    }
    Ok(out)
}

/// A cable laid along a route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CableRun {
    pub cable_type: String,
    pub route: Vec<P3>,
    /// Usable drum length for this type, m.
    pub drum_length: f64,
    /// Extra length per run (slack, risers, connections), m.
    pub extra: f64,
}

/// Quantities per cable type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeQuantity {
    pub cable_type: String,
    /// Cable length incl. extra, m.
    pub length: f64,
    pub joints: usize,
    pub runs: usize,
}

/// Bill of quantities for a set of runs and trenches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quantities {
    pub per_type: Vec<TypeQuantity>,
    /// Plan length of all trenches, m.
    pub trench_length: f64,
}

/// Cable length (3D + extra) and joints (no preferred positions) per type;
/// trench length from the given trench centre lines.
pub fn quantities(runs: &[CableRun], trenches: &[Vec<P2>]) -> Result<Quantities, String> {
    let mut per_type: Vec<TypeQuantity> = Vec::new();
    for r in runs {
        let l = length_3d(&r.route) + r.extra;
        let j = joint_positions(l, r.drum_length, &[])?.len();
        match per_type.iter_mut().find(|q| q.cable_type == r.cable_type) {
            Some(q) => {
                q.length += l;
                q.joints += j;
                q.runs += 1;
            }
            None => per_type.push(TypeQuantity {
                cable_type: r.cable_type.clone(),
                length: l,
                joints: j,
                runs: 1,
            }),
        }
    }
    let trench_length = trenches
        .iter()
        .map(|t| t.windows(2).map(|w| d2(w[0], w[1])).sum::<f64>())
        .sum();
    Ok(Quantities {
        per_type,
        trench_length,
    })
}

// --------------------------------------------------------------- overhead

/// Catenary of one span in its vertical plane: supports at horizontal
/// stations 0 and `span`, the second `dz` higher than the first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Catenary {
    pub span: f64,
    pub dz: f64,
    /// Conductor weight per metre, N/m.
    pub weight: f64,
    /// Horizontal tension, N.
    pub tension: f64,
    /// Catenary parameter a = H / w, m.
    pub parameter: f64,
    /// Station of the lowest point (may lie outside the span), m.
    pub low_station: f64,
    /// Height of the lowest point relative to the first support, m.
    pub low_z: f64,
}

/// Results of [`Catenary::summary`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatenarySummary {
    /// Largest vertical distance chord − conductor, m.
    pub sag: f64,
    pub sag_station: f64,
    /// Lowest conductor point within the span: station, relative z.
    pub lowest_in_span: [f64; 2],
    /// Conductor length, m.
    pub conductor_length: f64,
    /// Largest tension (at the higher support), N.
    pub max_tension: f64,
}

impl Catenary {
    /// Exact catenary through both supports.
    pub fn new(span: f64, dz: f64, weight: f64, tension: f64) -> Result<Self, String> {
        if !(span > 0.0 && weight > 0.0 && tension > 0.0) {
            return Err("span, weight and tension must be positive".into());
        }
        let a = tension / weight;
        let half = span / (2.0 * a);
        let x0 = span / 2.0 - a * (dz / (2.0 * a * half.sinh())).asinh();
        let low_z = -a * ((-x0 / a).cosh() - 1.0);
        Ok(Self {
            span,
            dz,
            weight,
            tension,
            parameter: a,
            low_station: x0,
            low_z,
        })
    }

    /// Horizontal tension giving `sag` (bisection on the exact curve).
    pub fn from_sag(span: f64, dz: f64, weight: f64, sag: f64) -> Result<Self, String> {
        if sag.is_nan() || sag <= 0.0 {
            return Err("sag must be positive".into());
        }
        let s = |h: f64| Catenary::new(span, dz, weight, h).map(|c| c.summary().sag);
        // Parabolic estimate, then bracket.
        let est = weight * span * span / (8.0 * sag);
        let (mut lo, mut hi) = (est / 10.0, est * 10.0);
        if s(lo)? < sag || s(hi)? > sag {
            return Err("sag out of range".into());
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if s(mid)? > sag {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Catenary::new(span, dz, weight, 0.5 * (lo + hi))
    }

    /// Conductor height relative to the first support at station `x`.
    pub fn z(&self, x: f64) -> f64 {
        let a = self.parameter;
        self.low_z + a * (((x - self.low_station) / a).cosh() - 1.0)
    }

    /// Sag, lowest point, length and maximum tension.
    pub fn summary(&self) -> CatenarySummary {
        let a = self.parameter;
        let xs = self.low_station + a * (self.dz / self.span).asinh();
        let sag = self.dz * xs / self.span - self.z(xs);
        let xl = self.low_station.clamp(0.0, self.span);
        let half = self.span / (2.0 * a);
        let conductor_length = self.dz.hypot(2.0 * a * half.sinh());
        let z_high = self.dz.max(0.0);
        CatenarySummary {
            sag,
            sag_station: xs,
            lowest_in_span: [xl, self.z(xl)],
            conductor_length,
            max_tension: self.tension + self.weight * (z_high - self.low_z),
        }
    }

    /// Conductor curve between supports `a` and `b` in 3D, `n` segments.
    /// The span and height difference are taken from `a`, `b`.
    pub fn points_between(
        a: P3,
        b: P3,
        weight: f64,
        tension: f64,
        n: usize,
    ) -> Result<Vec<P3>, String> {
        let span = d2([a[0], a[1]], [b[0], b[1]]);
        let c = Catenary::new(span, b[2] - a[2], weight, tension)?;
        let n = n.max(1);
        Ok((0..=n)
            .map(|i| {
                let t = i as f64 / n as f64;
                let x = t * span;
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + c.z(x),
                ]
            })
            .collect())
    }
}

/// Conductor properties for the change-of-state equation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conductor {
    /// Elastic modulus × cross-section E·A, N.
    pub ea: f64,
    /// Coefficient of thermal expansion, 1/K.
    pub alpha: f64,
}

/// Horizontal tension in state 2 from a known state 1 (tension `h1`, load
/// `w1` N/m, temperature `t1`) for a (ruling) span `span`, load `w2` and
/// temperature `t2` (°C), solving the parabolic change-of-state equation.
pub fn change_of_state(
    c: &Conductor,
    span: f64,
    h1: f64,
    w1: f64,
    t1: f64,
    w2: f64,
    t2: f64,
) -> Result<f64, String> {
    if !(c.ea > 0.0 && span > 0.0 && h1 > 0.0 && w1 > 0.0 && w2 > 0.0) {
        return Err("positive EA, span, tension and loads required".into());
    }
    let l2 = span * span;
    let rhs = w1 * w1 * l2 / (24.0 * h1 * h1) + c.alpha * (t2 - t1);
    // f decreases monotonically from +∞ (H → 0) to −∞.
    let f = |h: f64| w2 * w2 * l2 / (24.0 * h * h) - (h - h1) / c.ea - rhs;
    let (mut lo, mut hi) = (1e-9 * h1, h1);
    while f(hi) > 0.0 {
        hi *= 2.0;
        if hi > 1e6 * h1 {
            return Err("no solution for the change of state".into());
        }
    }
    for _ in 0..300 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

/// An object to keep clear of (top point) with its own minimum distance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Obstacle {
    pub name: String,
    pub point: P3,
    pub min_clearance: f64,
}

/// A clearance shortfall.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClearanceIssue {
    /// "ground" or the obstacle name.
    pub what: String,
    /// Conductor point where it is closest.
    pub at: P3,
    pub clearance: f64,
    pub required: f64,
}

/// Check a sampled conductor curve: vertical clearance to the ground
/// (`min_ground`) and 3D distance to obstacles. Returns the worst point per
/// ground stretch and per obstacle that falls short.
pub fn clearance_check(
    curve: &[P3],
    ground: &dyn Fn(P2) -> Option<f64>,
    min_ground: f64,
    obstacles: &[Obstacle],
) -> Vec<ClearanceIssue> {
    let mut out: Vec<ClearanceIssue> = Vec::new();
    let mut open = false;
    for p in curve {
        match ground([p[0], p[1]]) {
            Some(g) if p[2] - g < min_ground => {
                let c = p[2] - g;
                if open {
                    let v = out.last_mut().unwrap();
                    if c < v.clearance {
                        v.clearance = c;
                        v.at = *p;
                    }
                } else {
                    out.push(ClearanceIssue {
                        what: "ground".into(),
                        at: *p,
                        clearance: c,
                        required: min_ground,
                    });
                    open = true;
                }
            }
            _ => open = false,
        }
    }
    for o in obstacles {
        let mut best = (f64::INFINITY, [0.0; 3]);
        for w in curve.windows(2) {
            let (d, q) = point_segment_3d(o.point, w[0], w[1]);
            if d < best.0 {
                best = (d, q);
            }
        }
        if best.0 < o.min_clearance {
            out.push(ClearanceIssue {
                what: o.name.clone(),
                at: best.1,
                clearance: best.0,
                required: o.min_clearance,
            });
        }
    }
    out
}

fn point_segment_3d(p: P3, a: P3, b: P3) -> (f64, P3) {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1] + (p[2] - a[2]) * ab[2]) / l2)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = lerp3(a, b, t);
    (d3(p, q), q)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_span_sag_matches_parabola_and_exact_formula() {
        let (l, w, h) = (300.0, 15.0, 25_000.0);
        let c = Catenary::new(l, 0.0, w, h).unwrap();
        let s = c.summary();
        let a = h / w;
        let exact = a * ((l / (2.0 * a)).cosh() - 1.0);
        assert!((s.sag - exact).abs() < 1e-9);
        let para = w * l * l / (8.0 * h);
        assert!((s.sag - para).abs() / para < 0.005, "{} vs {}", s.sag, para);
        assert!((s.sag_station - 150.0).abs() < 1e-9);
        assert!((c.low_station - 150.0).abs() < 1e-9);
    }

    #[test]
    fn inclined_span_passes_both_supports() {
        let c = Catenary::new(250.0, 20.0, 12.0, 20_000.0).unwrap();
        assert!(c.z(0.0).abs() < 1e-9);
        assert!((c.z(250.0) - 20.0).abs() < 1e-9);
        let s = c.summary();
        // Sag of an inclined span ≈ w·L²/(8H) as well (parabolic, chord-based).
        let para = 12.0 * 250.0 * 250.0 / (8.0 * 20_000.0);
        assert!((s.sag - para).abs() / para < 0.01);
        assert!(s.max_tension > 20_000.0);
    }

    #[test]
    fn tension_from_sag_round_trip() {
        let c = Catenary::new(200.0, 5.0, 10.0, 18_000.0).unwrap();
        let sag = c.summary().sag;
        let d = Catenary::from_sag(200.0, 5.0, 10.0, sag).unwrap();
        assert!((d.tension - 18_000.0).abs() < 1e-3);
    }

    #[test]
    fn change_of_state_reference_and_heating() {
        let c = Conductor {
            ea: 60.0e3 * 1e6 * 300e-6,
            alpha: 19.3e-6,
        };
        let h = change_of_state(&c, 300.0, 25_000.0, 15.0, 10.0, 15.0, 10.0).unwrap();
        assert!((h - 25_000.0).abs() < 1e-6);
        let hot = change_of_state(&c, 300.0, 25_000.0, 15.0, 10.0, 15.0, 60.0).unwrap();
        assert!(hot < 25_000.0);
        // Residual of the equation is zero at the solution.
        let l2 = 300.0f64 * 300.0;
        let r = 15.0f64.powi(2) * l2 / (24.0 * hot * hot)
            - 15.0f64.powi(2) * l2 / (24.0 * 25_000.0f64.powi(2))
            - c.alpha * 50.0
            - (hot - 25_000.0) / c.ea;
        assert!(r.abs() < 1e-9);
    }

    #[test]
    fn joints_follow_drum_rule() {
        for (l, drum) in [
            (1200.0, 500.0),
            (1000.0, 500.0),
            (499.0, 500.0),
            (2501.0, 250.0),
        ] {
            let j = joint_positions(l, drum, &[]).unwrap();
            assert_eq!(j.len(), (l / drum).ceil() as usize - 1, "{l} / {drum}");
        }
        // Preferred pits pull joints back.
        let j = joint_positions(1200.0, 500.0, &[450.0, 900.0]).unwrap();
        assert_eq!(j, vec![450.0, 900.0]);
        let q = quantities(
            &[CableRun {
                cable_type: "MS 3x240".into(),
                route: vec![[0.0, 0.0, -1.0], [1200.0, 0.0, -1.0]],
                drum_length: 500.0,
                extra: 0.0,
            }],
            &[vec![[0.0, 0.0], [1200.0, 0.0]]],
        )
        .unwrap();
        assert_eq!(q.per_type[0].joints, 2);
        assert!((q.trench_length - 1200.0).abs() < 1e-9);
    }

    #[test]
    fn cover_and_violations() {
        // Ground at z = 0 with a 0.5 m mound between x 40 and 60.
        let ground = |p: P2| {
            Some(if (40.0..=60.0).contains(&p[0]) {
                0.5
            } else {
                0.0
            })
        };
        let route = drape(&[[0.0, 0.0], [100.0, 0.0]], &|_| Some(0.0), 0.8, 0.05, 10.0).unwrap();
        let s = cover_along(&route, 0.05, &ground, 10.0);
        assert!(s
            .iter()
            .all(|c| (c.cover - 0.8).abs() < 1e-9 || (c.cover - 1.3).abs() < 1e-9));
        // Excavation: ground lowered to −0.3 between 20 and 30.
        let ground2 = |p: P2| {
            Some(if (20.0..=30.0).contains(&p[0]) {
                -0.3
            } else {
                0.0
            })
        };
        let s = cover_along(&route, 0.05, &ground2, 5.0);
        let v = cover_violations(&s, CableClass::MediumVoltage.min_cover_nl());
        assert_eq!(v.len(), 1);
        assert!((v[0].from_station - 20.0).abs() < 1e-9 && (v[0].to_station - 30.0).abs() < 1e-9);
        assert!((v[0].least_cover - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clearance_to_ground_and_obstacle() {
        let pts =
            Catenary::points_between([0.0, 0.0, 20.0], [300.0, 0.0, 20.0], 15.0, 25_000.0, 60)
                .unwrap();
        // Sag ≈ 6.75 m → lowest ≈ 13.25 m above ground 0.
        assert!(clearance_check(&pts, &|_| Some(0.0), 7.0, &[]).is_empty());
        let issues = clearance_check(&pts, &|_| Some(0.0), 14.0, &[]);
        assert_eq!(issues.len(), 1);
        assert!((issues[0].at[0] - 150.0).abs() < 1e-6);
        let ob = Obstacle {
            name: "tree".into(),
            point: [150.0, 2.0, 10.0],
            min_clearance: 4.0,
        };
        let issues = clearance_check(&pts, &|_| Some(0.0), 7.0, &[ob]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].what, "tree");
    }
}
