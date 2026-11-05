//! Steady water surface profiles in a watercourse (waterlijnberekening):
//! one-dimensional gradually varied flow by the standard step method with
//! Manning, from a downstream boundary upstream (subcritical flow), with
//! culverts (duikers) and weirs (stuwen) between cross sections.
//!
//! The method follows the HEC-RAS *Hydraulic Reference Manual* (USACE,
//! version 4.1, 2010; here "HRM"): the energy equation between sections
//! (HRM eq. 2-1/2-2), the friction slope by average conveyance (HRM eq.
//! 2-13, the HEC-RAS default), contraction and expansion losses on the
//! difference in velocity heads (eq. 2-2), Manning conveyance K = A R^⅔ / n
//! per wet sub-area (eq. 2-5 in SI), the velocity coefficient α from those
//! conveyances (eq. 2-7), and critical depth as the minimum of the specific
//! energy (HRM ch. 2, "Critical Depth Determination"). Where no subcritical
//! solution exists, the level is set to critical depth and flagged, as
//! HEC-RAS does (HRM ch. 2). See also Chow, *Open-Channel Hydraulics* (1959).
//!
//! Limitations: subcritical only (no hydraulic jumps, no mixed regime);
//! one Manning n per section; every wet part of a section conveys (no levees
//! or ineffective areas); the ends of a section are extended as vertical
//! walls when the water rises above them (flagged); the reach length is the
//! station difference; structures are taken as lying between the two sections
//! around them, without friction over that reach.

use serde::{Deserialize, Serialize};

use crate::riool::{circle_section, G};
use crate::P2;

// ── Design values ───────────────────────────────────────────────────────────

/// Contraction loss coefficient for gradual transitions in subcritical flow: HRM table 3-3.
pub const CONTRACTION: f64 = 0.1;
/// Expansion loss coefficient for gradual transitions: HRM table 3-3 (0.3/0.5
/// at bridges, 0.6/0.8 for abrupt transitions).
pub const EXPANSION: f64 = 0.3;
/// Culvert entrance loss coefficient, headwall with square edge: HRM table 6-3 (after FHWA 1985).
pub const ENTRANCE: f64 = 0.5;
/// Culvert exit loss coefficient for a sudden expansion: HRM eq. 6-12 (FHWA
/// 1985; 0.3–1.0 for less abrupt outlets).
pub const EXIT: f64 = 1.0;
/// Broad-crested weir coefficient C in Q = C·b·H^1.5 (m^½/s). Critical flow
/// on the crest gives C = (2/3)^1.5·√g ≈ 1.70; HRM table 8-1 lists 2.6–3.1
/// ft^½/s for broad crests, 1.44–1.71 in SI (factor 0.552, HRM ch. 8).
pub const WEIR_C: f64 = 1.70;
/// Villemonte (1947) submergence: Q = Q_free·(1 − (H₂/H₁)^1.5)^0.385, the
/// exponent 1.5 for a rectangular weir. HEC-RAS applies Villemonte to sharp
/// crests and its own curve to broad crests (HRM ch. 8); here it is used for
/// both, a simplification.
pub const VILLEMONTE_N: f64 = 1.5;
pub const VILLEMONTE_M: f64 = 0.385;

fn contraction() -> f64 {
    CONTRACTION
}
fn expansion() -> f64 {
    EXPANSION
}
fn entrance() -> f64 {
    ENTRANCE
}
fn exit() -> f64 {
    EXIT
}
fn weir_c() -> f64 {
    WEIR_C
}

// ── Cross sections ──────────────────────────────────────────────────────────

/// A cross section at a station along the axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossSection {
    /// Station along the axis (m).
    pub station: f64,
    /// (offset, z) from left to right looking downstream; offsets non-decreasing.
    pub points: Vec<P2>,
    /// Manning n (s/m^⅓).
    pub n: f64,
    /// Coefficients for the reach downstream of this section (HEC-RAS puts
    /// them on the upstream section); `None` takes the profile's.
    #[serde(default)]
    pub contraction: Option<f64>,
    #[serde(default)]
    pub expansion: Option<f64>,
}

/// Wetted geometry at a water level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    pub area: f64,
    pub perimeter: f64,
    pub top_width: f64,
    pub hydraulic_radius: f64,
    /// Conveyance K = Σ Aᵢ Rᵢ^⅔ / n over the wet sub-areas.
    pub conveyance: f64,
    /// Velocity coefficient α = A² Σ(Kᵢ³/Aᵢ²) / K³.
    pub alpha: f64,
    /// Offsets of the outer waterlines (the inundation boundary).
    pub left: f64,
    pub right: f64,
    /// Offset intervals of each wet sub-area, left to right.
    pub wet: Vec<[f64; 2]>,
    /// The water is above an end of the section (vertical wall assumed).
    pub walls: bool,
}

#[derive(Clone, Copy)]
struct Piece {
    a: f64,
    p: f64,
    t: f64,
    y0: f64,
    y1: f64,
}

impl CrossSection {
    /// Lowest bed level.
    pub fn z_min(&self) -> f64 {
        self.points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min)
    }

    /// Checks the points and n.
    pub fn validate(&self) -> Result<(), String> {
        if self.points.len() < 2 {
            return Err(format!("section {}: at least two points", self.station));
        }
        if self.points.windows(2).any(|w| w[1][0] < w[0][0]) {
            return Err(format!("section {}: offsets must run left to right", self.station));
        }
        if self.n <= 0.0 {
            return Err(format!("section {}: Manning n must be positive", self.station));
        }
        Ok(())
    }

    /// Geometry at water level `ws`: the section polyline clipped at the
    /// level, every wet stretch between dry points a sub-area.
    pub fn geometry(&self, ws: f64) -> Geometry {
        let pts = &self.points;
        let d = |i: usize| ws - pts[i][1];
        let mut pieces: Vec<Piece> = Vec::new();
        let mut cur: Option<Piece> = None;
        let mut walls = false;
        if d(0) > 0.0 {
            walls = true;
            cur = Some(Piece { a: 0.0, p: d(0), t: 0.0, y0: pts[0][0], y1: pts[0][0] });
        }
        for i in 0..pts.len() - 1 {
            let ([y0, z0], [y1, z1]) = (pts[i], pts[i + 1]);
            let (d0, d1) = (d(i), d(i + 1));
            if d0 <= 0.0 && d1 <= 0.0 {
                if let Some(c) = cur.take() {
                    pieces.push(c);
                }
                continue;
            }
            let len = (y1 - y0).hypot(z1 - z0);
            let (ya, yb, da, db, frac) = if d0 > 0.0 && d1 > 0.0 {
                (y0, y1, d0, d1, 1.0)
            } else {
                let t = d0 / (d0 - d1);
                let yc = y0 + t * (y1 - y0);
                if d0 > 0.0 { (y0, yc, d0, 0.0, t) } else { (yc, y1, 0.0, d1, 1.0 - t) }
            };
            let c = cur.get_or_insert(Piece { a: 0.0, p: 0.0, t: 0.0, y0: ya, y1: ya });
            c.a += (da + db) / 2.0 * (yb - ya);
            c.p += len * frac;
            c.t += yb - ya;
            c.y1 = yb;
            if d1 <= 0.0 {
                pieces.extend(cur.take());
            }
        }
        if let Some(mut c) = cur.take() {
            let last = d(pts.len() - 1);
            if last > 0.0 {
                walls = true;
                c.p += last;
            }
            pieces.push(c);
        }
        let mut g = Geometry { area: 0.0, perimeter: 0.0, top_width: 0.0, hydraulic_radius: 0.0, conveyance: 0.0, alpha: 1.0, left: 0.0, right: 0.0, wet: vec![], walls };
        let mut k3a2 = 0.0;
        for c in pieces.iter().filter(|c| c.a > 0.0) {
            let k = c.a / self.n * (c.a / c.p.max(1e-12)).powf(2.0 / 3.0);
            g.area += c.a;
            g.perimeter += c.p;
            g.top_width += c.t;
            g.conveyance += k;
            k3a2 += k * k * k / (c.a * c.a);
            g.wet.push([c.y0, c.y1]);
        }
        if let (Some(f), Some(l)) = (g.wet.first(), g.wet.last()) {
            g.left = f[0];
            g.right = l[1];
        }
        if g.perimeter > 0.0 {
            g.hydraulic_radius = g.area / g.perimeter;
        }
        if g.conveyance > 0.0 {
            g.alpha = g.area * g.area * k3a2 / g.conveyance.powi(3);
        }
        g
    }

    /// Energy level ws + αV²/2g for discharge `q`.
    pub fn energy(&self, ws: f64, q: f64) -> f64 {
        let g = self.geometry(ws);
        if g.area <= 0.0 {
            return f64::INFINITY;
        }
        ws + g.alpha * q * q / (2.0 * G * g.area * g.area)
    }
}

/// A depth above the bed where `ok` holds, by doubling from 0.1 m.
fn bracket(z: f64, mut ok: impl FnMut(f64) -> bool) -> Result<f64, String> {
    let mut h = 0.1;
    for _ in 0..40 {
        if ok(z + h) {
            return Ok(h);
        }
        h *= 2.0;
    }
    Err("no water level found".into())
}

fn bisect(mut a: f64, mut b: f64, mut below: impl FnMut(f64) -> bool) -> f64 {
    for _ in 0..100 {
        let m = (a + b) / 2.0;
        if below(m) {
            a = m;
        } else {
            b = m;
        }
    }
    (a + b) / 2.0
}

/// Water level of uniform flow: K(ws)·√S = Q (Manning).
pub fn normal_level(sec: &CrossSection, q: f64, slope: f64) -> Result<f64, String> {
    sec.validate()?;
    if slope <= 0.0 || q <= 0.0 {
        return Err("normal depth needs a positive slope and discharge".into());
    }
    let z = sec.z_min();
    let h = bracket(z, |ws| sec.geometry(ws).conveyance * slope.sqrt() >= q)?;
    Ok(bisect(z, z + h, |ws| sec.geometry(ws).conveyance * slope.sqrt() < q))
}

/// Critical water level: the minimum of the specific energy ws + αQ²/(2gA²)
/// (HRM ch. 2, critical depth), by a scan and golden-section refinement.
pub fn critical_level(sec: &CrossSection, q: f64) -> Result<f64, String> {
    sec.validate()?;
    if q <= 0.0 {
        return Err("critical depth needs a positive discharge".into());
    }
    let z = sec.z_min();
    let froude2 = |ws: f64| {
        let g = sec.geometry(ws);
        if g.area <= 0.0 || g.top_width <= 0.0 {
            return f64::INFINITY;
        }
        g.alpha * q * q * g.top_width / (G * g.area.powi(3))
    };
    let h = 2.0 * bracket(z, |ws| froude2(ws) < 1.0)?;
    let n = 400;
    let e = |ws: f64| sec.energy(ws, q);
    let step = h / n as f64;
    let best = (1..=n).min_by(|&a, &b| e(z + a as f64 * step).total_cmp(&e(z + b as f64 * step))).unwrap_or(1);
    let (mut a, mut b) = (z + (best as f64 - 1.0).max(1e-3) * step, z + (best as f64 + 1.0) * step);
    let r = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..100 {
        let (x1, x2) = (b - r * (b - a), a + r * (b - a));
        if e(x1) < e(x2) {
            b = x2;
        } else {
            a = x1;
        }
    }
    Ok((a + b) / 2.0)
}

/// The subcritical level with energy `e`, or critical depth (true) when `e`
/// is below the critical energy.
pub fn level_from_energy(sec: &CrossSection, q: f64, e: f64) -> Result<(f64, bool), String> {
    let wc = critical_level(sec, q)?;
    if e <= sec.energy(wc, q) {
        return Ok((wc, true));
    }
    Ok((bisect(wc, e, |ws| sec.energy(ws, q) < e), false))
}

// ── Structures ──────────────────────────────────────────────────────────────

/// Barrel shape of a culvert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CulvertShape {
    Circular { diameter: f64 },
    Rectangular { width: f64, height: f64 },
}

impl CulvertShape {
    pub fn height(&self) -> f64 {
        match self {
            CulvertShape::Circular { diameter } => *diameter,
            CulvertShape::Rectangular { height, .. } => *height,
        }
    }

    /// (area, wetted perimeter, top width) at depth `y`; full at the height
    /// (the roof counted in the perimeter).
    pub fn wetted(&self, y: f64) -> (f64, f64, f64) {
        let h = self.height();
        match self {
            CulvertShape::Circular { diameter } => {
                let c = circle_section(*diameter, y);
                (c.area, c.perimeter, c.top_width)
            }
            CulvertShape::Rectangular { width, height } => {
                let y = y.clamp(0.0, *height);
                let roof = if y >= h { *width } else { 0.0 };
                (width * y, width + 2.0 * y + roof, if y >= h { 0.0 } else { *width })
            }
        }
    }

    /// Critical depth in the barrel, `None` when it would reach the roof.
    pub fn critical_depth(&self, q: f64) -> Option<f64> {
        let h = self.height();
        let fr2 = |y: f64| {
            let (a, _, t) = self.wetted(y);
            q * q * t / (G * a.powi(3))
        };
        if fr2(0.999 * h) > 1.0 {
            return None;
        }
        Some(bisect(1e-9, 0.999 * h, |y| fr2(y) > 1.0))
    }
}

/// A culvert (duiker).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Culvert {
    pub shape: CulvertShape,
    /// Barrel length (m).
    pub length: f64,
    pub invert_up: f64,
    pub invert_down: f64,
    /// Manning n of the barrel.
    pub n: f64,
    #[serde(default = "entrance")]
    pub entrance: f64,
    #[serde(default = "exit")]
    pub exit: f64,
}

/// What a culvert does to the flow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CulvertResult {
    /// Energy level upstream (m).
    pub energy_up: f64,
    /// Barrel velocity used for the losses (m/s).
    pub velocity: f64,
    /// Entrance, friction and exit loss (m).
    pub entrance_loss: f64,
    pub friction_loss: f64,
    pub exit_loss: f64,
    /// Upstream energy by a simple inlet-control check, `None` when the
    /// critical depth reaches the roof (the inlet is then submerged).
    pub inlet_control: Option<f64>,
    /// "outlet, full", "outlet, part-full" or "inlet".
    pub regime: String,
}

/// Upstream energy level of a culvert passing `q`, from the downstream water
/// level and channel velocity. Outlet control (HRM ch. 6, after FHWA 1985):
/// E_up = WS_out + V₂²/2g + k_ex(V² − V₂²)/2g + h_f + k_en V²/2g, with the
/// barrel full when the tailwater is above its roof, otherwise flowing at the
/// tailwater depth but no less than critical depth (a free outfall); h_f by
/// Manning over the whole length at that one depth. Inlet control, checked
/// simply: critical depth at the inlet plus the entrance loss,
/// E = z_in + y_c + (1 + k_en) V_c²/2g, valid for an unsubmerged inlet only
/// (no FHWA HDS-5 nomographs). The larger governs.
pub fn culvert(c: &Culvert, q: f64, level_down: f64, v_down: f64) -> Result<CulvertResult, String> {
    let h = c.shape.height();
    if q <= 0.0 || h <= 0.0 || c.length <= 0.0 || c.n <= 0.0 {
        return Err("culvert: positive discharge, size, length and n expected".into());
    }
    let yc = c.shape.critical_depth(q);
    let tail = level_down - c.invert_down;
    let full = tail >= h;
    let y = if full { h } else { tail.max(yc.unwrap_or(h)).min(h) };
    let ws_out = if full { level_down } else { c.invert_down + y };
    let (a, p, _) = c.shape.wetted(y);
    let v = q / a;
    let r = a / p;
    let friction_loss = c.length * (q * c.n / (a * r.powf(2.0 / 3.0))).powi(2);
    let exit_loss = c.exit * (v * v - v_down * v_down).max(0.0) / (2.0 * G);
    let entrance_loss = c.entrance * v * v / (2.0 * G);
    let outlet = ws_out + v_down * v_down / (2.0 * G) + exit_loss + friction_loss + entrance_loss;
    let inlet_control = yc.map(|yc| {
        let vc = q / c.shape.wetted(yc).0;
        c.invert_up + yc + (1.0 + c.entrance) * vc * vc / (2.0 * G)
    });
    let (energy_up, regime) = match inlet_control {
        Some(e) if e > outlet => (e, "inlet"),
        _ => (outlet, if full { "outlet, full" } else { "outlet, part-full" }),
    };
    Ok(CulvertResult { energy_up, velocity: v, entrance_loss, friction_loss, exit_loss, inlet_control, regime: regime.into() })
}

/// A weir (stuw) with a horizontal crest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Weir {
    /// Crest level (m).
    pub crest: f64,
    /// Crest width (m).
    pub width: f64,
    /// Discharge coefficient C (m^½/s).
    #[serde(default = "weir_c")]
    pub c: f64,
}

/// What a weir does to the flow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeirResult {
    /// Upstream energy level (m).
    pub energy_up: f64,
    /// Upstream energy head above the crest H₁ (m).
    pub head: f64,
    pub submerged: bool,
}

/// Weir discharge for upstream energy head `h1` and downstream head `h2`
/// above the crest: C·b·H₁^1.5, reduced by Villemonte when `h2` > 0.
pub fn weir_discharge(w: &Weir, h1: f64, h2: f64) -> f64 {
    if h1 <= 0.0 || h2 >= h1 {
        return 0.0;
    }
    let free = w.c * w.width * h1.powf(1.5);
    if h2 <= 0.0 { free } else { free * (1.0 - (h2 / h1).powf(VILLEMONTE_N)).powf(VILLEMONTE_M) }
}

/// Upstream energy level of a weir passing `q` with downstream water level
/// `level_down`; free flow gives H₁ = (Q/(C·b))^⅔.
pub fn weir(w: &Weir, q: f64, level_down: f64) -> Result<WeirResult, String> {
    if q <= 0.0 || w.width <= 0.0 || w.c <= 0.0 {
        return Err("weir: positive discharge, width and coefficient expected".into());
    }
    let h2 = level_down - w.crest;
    let head = if h2 <= 0.0 {
        (q / (w.c * w.width)).powf(2.0 / 3.0)
    } else {
        let hi = bracket(h2, |h1| weir_discharge(w, h1, h2) >= q)?;
        bisect(h2, h2 + hi, |h1| weir_discharge(w, h1, h2) < q)
    };
    Ok(WeirResult { energy_up: w.crest + head, head, submerged: h2 > 0.0 })
}

/// A structure between the two cross sections around its station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StructureKind {
    Culvert(Culvert),
    Weir(Weir),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    pub station: f64,
    pub kind: StructureKind,
}

// ── The profile ─────────────────────────────────────────────────────────────

/// Downstream boundary condition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Boundary {
    /// Known water level (m).
    Level(f64),
    /// Normal depth for this energy slope.
    NormalDepth(f64),
    CriticalDepth,
}

/// A steady profile computation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Discharge (m³/s), constant along the reach.
    pub q: f64,
    pub sections: Vec<CrossSection>,
    #[serde(default)]
    pub structures: Vec<Structure>,
    pub boundary: Boundary,
    /// Stations increase downstream; by default they increase upstream, as in HEC-RAS.
    #[serde(default)]
    pub stations_downstream: bool,
    #[serde(default = "contraction")]
    pub contraction: f64,
    #[serde(default = "expansion")]
    pub expansion: f64,
}

/// Result at one cross section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionResult {
    pub station: f64,
    pub level: f64,
    /// Above the lowest bed point.
    pub depth: f64,
    pub velocity: f64,
    pub froude: f64,
    pub top_width: f64,
    /// Outer waterline offsets.
    pub left: f64,
    pub right: f64,
    pub wet: Vec<[f64; 2]>,
    pub energy: f64,
    /// Set to critical depth (no subcritical solution).
    pub critical: bool,
    /// Water above an end of the section.
    pub walls: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructureResult {
    pub station: f64,
    pub level_down: f64,
    pub energy_up: f64,
    pub level_up: f64,
    /// Culvert regime, or "free" / "submerged" for a weir.
    pub regime: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileResult {
    /// In order of station.
    pub sections: Vec<SectionResult>,
    pub structures: Vec<StructureResult>,
}

fn result(sec: &CrossSection, q: f64, ws: f64, critical: bool) -> SectionResult {
    let g = sec.geometry(ws);
    let v = if g.area > 0.0 { q / g.area } else { 0.0 };
    let froude = if g.top_width > 0.0 { v / (G * g.area / g.top_width).sqrt() } else { 0.0 };
    SectionResult {
        station: sec.station,
        level: ws,
        depth: ws - sec.z_min(),
        velocity: v,
        froude,
        top_width: g.top_width,
        left: g.left,
        right: g.right,
        energy: ws + g.alpha * v * v / (2.0 * G),
        critical,
        walls: g.walls,
        wet: g.wet,
    }
}

/// One standard step: the level at `up` from the known level at `down`,
/// WS₂ + α₂V₂²/2g = WS₁ + α₁V₁²/2g + L·S̄f + C|α₂V₂²/2g − α₁V₁²/2g|,
/// S̄f = ((Q+Q)/(K₁+K₂))² (HRM eqs. 2-1, 2-2, 2-13); C is the contraction
/// coefficient when the velocity head grows downstream, else expansion.
pub fn step(down: &CrossSection, ws_down: f64, up: &CrossSection, q: f64, contraction: f64, expansion: f64) -> Result<(f64, bool), String> {
    let gd = down.geometry(ws_down);
    if gd.area <= 0.0 {
        return Err(format!("section {}: dry at level {:.3}", down.station, ws_down));
    }
    let hv1 = gd.alpha * q * q / (2.0 * G * gd.area * gd.area);
    let e1 = ws_down + hv1;
    let len = (up.station - down.station).abs();
    let f = |ws: f64| {
        let g = up.geometry(ws);
        if g.area <= 0.0 {
            return -f64::INFINITY;
        }
        let hv2 = g.alpha * q * q / (2.0 * G * g.area * g.area);
        let sf = (2.0 * q / (gd.conveyance + g.conveyance)).powi(2);
        let c = if hv1 > hv2 { contraction } else { expansion };
        ws + hv2 - e1 - len * sf - c * (hv2 - hv1).abs()
    };
    let wc = critical_level(up, q)?;
    if f(wc) >= 0.0 {
        return Ok((wc, true));
    }
    let hi = bracket(wc, |ws| f(ws) > 0.0)?;
    Ok((bisect(wc, wc + hi, |ws| f(ws) < 0.0), false))
}

/// The water surface profile from the downstream boundary upstream.
pub fn standard_step(p: &Profile) -> Result<ProfileResult, String> {
    if p.q <= 0.0 {
        return Err("positive discharge expected".into());
    }
    if p.sections.is_empty() {
        return Err("no cross sections".into());
    }
    for s in &p.sections {
        s.validate()?;
    }
    let mut idx: Vec<usize> = (0..p.sections.len()).collect();
    idx.sort_by(|&a, &b| p.sections[a].station.total_cmp(&p.sections[b].station));
    if p.stations_downstream {
        idx.reverse();
    }
    let first = &p.sections[idx[0]];
    let (ws0, crit0) = match p.boundary {
        Boundary::Level(h) => {
            let wc = critical_level(first, p.q)?;
            if h < wc { (wc, true) } else { (h, false) }
        }
        Boundary::NormalDepth(s) => (normal_level(first, p.q, s)?, false),
        Boundary::CriticalDepth => (critical_level(first, p.q)?, true),
    };
    let mut out = vec![result(first, p.q, ws0, crit0)];
    let mut structures = Vec::new();
    let mut used = vec![false; p.structures.len()];
    for w in idx.windows(2) {
        let (down, up) = (&p.sections[w[0]], &p.sections[w[1]]);
        let (lo, hi) = (down.station.min(up.station), down.station.max(up.station));
        let here: Vec<usize> = (0..p.structures.len()).filter(|&i| !used[i] && p.structures[i].station >= lo && p.structures[i].station <= hi).collect();
        if here.len() > 1 {
            return Err(format!("more than one structure between stations {} and {}", lo, hi));
        }
        let prev = out.last().expect("boundary result");
        let (ws, crit) = if let Some(&i) = here.first() {
            used[i] = true;
            let (e, regime) = match &p.structures[i].kind {
                StructureKind::Culvert(c) => {
                    let r = culvert(c, p.q, prev.level, prev.velocity)?;
                    (r.energy_up, r.regime)
                }
                StructureKind::Weir(wr) => {
                    let r = weir(wr, p.q, prev.level)?;
                    (r.energy_up, if r.submerged { "submerged" } else { "free" }.to_string())
                }
            };
            let e = e.max(prev.energy);
            let (ws, crit) = level_from_energy(up, p.q, e)?;
            structures.push(StructureResult { station: p.structures[i].station, level_down: prev.level, energy_up: e, level_up: ws, regime });
            (ws, crit)
        } else {
            step(down, prev.level, up, p.q, up.contraction.unwrap_or(p.contraction), up.expansion.unwrap_or(p.expansion))?
        };
        out.push(result(up, p.q, ws, crit));
    }
    if let Some(i) = used.iter().position(|u| !u) {
        return Err(format!("structure at station {} lies outside the cross sections", p.structures[i].station));
    }
    out.sort_by(|a, b| a.station.total_cmp(&b.station));
    Ok(ProfileResult { sections: out, structures })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{} != {} (tol {})", a, b, tol);
    }

    fn rect(station: f64, b: f64, z: f64, n: f64) -> CrossSection {
        CrossSection { station, points: vec![[0.0, z + 20.0], [0.0, z], [b, z], [b, z + 20.0]], n, contraction: None, expansion: None }
    }

    fn trapezoid(station: f64, b: f64, m: f64, z: f64, n: f64) -> CrossSection {
        let h = 10.0;
        CrossSection { station, points: vec![[-m * h, z + h], [0.0, z], [b, z], [b + m * h, z + h]], n, contraction: None, expansion: None }
    }

    /// Manning discharge of a trapezoid at depth y.
    fn manning_trapezoid(b: f64, m: f64, n: f64, s: f64, y: f64) -> f64 {
        let a = (b + m * y) * y;
        let p = b + 2.0 * y * (1.0 + m * m).sqrt();
        a / n * (a / p).powf(2.0 / 3.0) * s.sqrt()
    }

    #[test]
    fn geometry_of_a_trapezoid_and_sub_areas() {
        let t = trapezoid(0.0, 4.0, 2.0, 0.0, 0.03);
        let g = t.geometry(1.0);
        close(g.area, (4.0 + 2.0) * 1.0, 1e-12);
        close(g.perimeter, 4.0 + 2.0 * 5f64.sqrt(), 1e-12);
        close(g.top_width, 8.0, 1e-12);
        close(g.left, -2.0, 1e-12);
        close(g.right, 6.0, 1e-12);
        close(g.alpha, 1.0, 1e-12);
        // Two ditches separated by a dry ridge: two sub-areas, α > 1 when unequal.
        let s = CrossSection { station: 0.0, points: vec![[0.0, 2.0], [1.0, 0.0], [2.0, 2.0], [3.0, 1.0], [4.0, 2.0]], n: 0.03, contraction: None, expansion: None };
        let g = s.geometry(1.5);
        assert_eq!(g.wet.len(), 2);
        close(g.wet[0][0], 0.25, 1e-12);
        close(g.wet[1][1], 3.5, 1e-12);
        close(g.area, 0.5 * 1.5 * 1.5 + 0.5 * 1.0 * 0.5, 1e-12);
        assert!(g.alpha > 1.0);
        // Above the ends: vertical walls.
        assert!(s.geometry(2.5).walls);
    }

    #[test]
    fn normal_depth_trapezoid() {
        let t = trapezoid(0.0, 4.0, 2.0, 0.0, 0.03);
        let (q, s) = (12.0, 0.0005);
        let y = normal_level(&t, q, s).unwrap();
        close(manning_trapezoid(4.0, 2.0, 0.03, s, y), q, 1e-6);
    }

    #[test]
    fn critical_depth_rectangle() {
        let r = rect(0.0, 5.0, 0.0, 0.03);
        let q = 10.0;
        let yc = critical_level(&r, q).unwrap();
        close(yc, ((q / 5.0f64).powi(2) / G).cbrt(), 1e-6);
        // At critical depth the Froude number is 1.
        close(result(&r, q, yc, true).froude, 1.0, 1e-4);
    }

    fn channel(len: f64, dx: f64, s: f64) -> Vec<CrossSection> {
        let n = (len / dx) as usize;
        (0..=n).map(|i| rect(i as f64 * dx, 10.0, i as f64 * dx * s, 0.03)).collect()
    }

    #[test]
    fn backwater_converges_to_normal_depth() {
        let (q, s) = (20.0, 0.001);
        let yn = normal_level(&rect(0.0, 10.0, 0.0, 0.03), q, s).unwrap();
        close(manning_trapezoid(10.0, 0.0, 0.03, s, yn), q, 1e-6);
        // M1: high downstream level; M2: critical depth downstream.
        for boundary in [Boundary::Level(3.0), Boundary::CriticalDepth] {
            let p = Profile { q, sections: channel(10_000.0, 50.0, s), structures: vec![], boundary, stations_downstream: false, contraction: CONTRACTION, expansion: EXPANSION };
            let r = standard_step(&p).unwrap();
            let last = r.sections.last().unwrap();
            close(last.depth, yn, 0.005);
            assert!(r.sections.iter().skip(1).all(|x| !x.critical));
        }
        // Normal-depth boundary stays at normal depth.
        let p = Profile { q, sections: channel(2000.0, 50.0, s), structures: vec![], boundary: Boundary::NormalDepth(s), stations_downstream: false, contraction: CONTRACTION, expansion: EXPANSION };
        let r = standard_step(&p).unwrap();
        assert!(r.sections.iter().all(|x| (x.depth - yn).abs() < 0.002));
    }

    #[test]
    fn stations_may_run_downstream() {
        let (q, s) = (20.0, 0.001);
        let mut secs = channel(5000.0, 50.0, s);
        for c in &mut secs {
            c.station = 5000.0 - c.station;
        }
        let p = Profile { q, sections: secs, structures: vec![], boundary: Boundary::Level(3.0), stations_downstream: true, contraction: CONTRACTION, expansion: EXPANSION };
        let r = standard_step(&p).unwrap();
        close(r.sections.last().unwrap().level, 3.0, 1e-12);
        assert!(r.sections[0].depth < 2.0);
    }

    #[test]
    fn culvert_loss_when_submerged() {
        let c = Culvert { shape: CulvertShape::Circular { diameter: 1.0 }, length: 20.0, invert_up: 0.02, invert_down: 0.0, n: 0.013, entrance: ENTRANCE, exit: EXIT };
        let q = 1.0;
        let r = culvert(&c, q, 1.5, 0.0).unwrap();
        let a = std::f64::consts::PI / 4.0;
        let v = q / a;
        let hf = 20.0 * (q * 0.013 / (a * 0.25f64.powf(2.0 / 3.0))).powi(2);
        close(r.energy_up - 1.5, (0.5 + 1.0) * v * v / (2.0 * G) + hf, 1e-9);
        assert_eq!(r.regime, "outlet, full");
        // In a wide channel the upstream level rises by about that loss.
        let secs = vec![rect(0.0, 10.0, 0.0, 0.03), rect(10.0, 10.0, 0.0, 0.03)];
        let s = Structure { station: 5.0, kind: StructureKind::Culvert(c) };
        let p = Profile { q, sections: secs, structures: vec![s], boundary: Boundary::Level(1.5), stations_downstream: false, contraction: CONTRACTION, expansion: EXPANSION };
        let pr = standard_step(&p).unwrap();
        close(pr.sections[1].level - pr.sections[0].level, r.energy_up - 1.5, 1e-3);
        assert_eq!(pr.structures.len(), 1);
    }

    #[test]
    fn culvert_free_outfall_and_inlet_control() {
        // Low tailwater: the barrel runs at critical depth at its outlet.
        let c = Culvert { shape: CulvertShape::Rectangular { width: 2.0, height: 1.5 }, length: 10.0, invert_up: 0.5, invert_down: 0.0, n: 0.013, entrance: ENTRANCE, exit: EXIT };
        let q = 2.0;
        let yc = ((q / 2.0f64).powi(2) / G).cbrt();
        close(c.shape.critical_depth(q).unwrap(), yc, 1e-6);
        let r = culvert(&c, q, -1.0, 0.0).unwrap();
        // A steep, short barrel: inlet control at E = z + yc + 1.5·Vc²/2g = z + 1.75 yc.
        assert_eq!(r.regime, "inlet");
        close(r.energy_up, 0.5 + 1.75 * yc, 1e-6);
    }

    #[test]
    fn weir_free_and_submerged() {
        let w = Weir { crest: 2.0, width: 5.0, c: WEIR_C };
        let q = 3.0;
        let r = weir(&w, q, 1.0).unwrap();
        close(r.head, (q / (WEIR_C * 5.0)).powf(2.0 / 3.0), 1e-12);
        close(r.energy_up, 2.0 + r.head, 1e-12);
        assert!(!r.submerged);
        // Submerged: higher head, and the Villemonte discharge equals q.
        let s = weir(&w, q, 2.0 + 0.9 * r.head).unwrap();
        assert!(s.submerged && s.head > r.head);
        close(weir_discharge(&w, s.head, 0.9 * r.head), q, 1e-9);
        // In a channel: the upstream energy level is crest + head.
        let secs = vec![rect(0.0, 5.0, 0.0, 0.03), rect(10.0, 5.0, 0.0, 0.03)];
        let p = Profile { q, sections: secs, structures: vec![Structure { station: 5.0, kind: StructureKind::Weir(w) }], boundary: Boundary::Level(1.0), stations_downstream: false, contraction: CONTRACTION, expansion: EXPANSION };
        let pr = standard_step(&p).unwrap();
        close(pr.sections[1].energy, 2.0 + r.head, 1e-6);
        assert_eq!(pr.structures[0].regime, "free");
    }

    #[test]
    fn bad_input() {
        let p = Profile { q: 1.0, sections: vec![rect(0.0, 5.0, 0.0, 0.03)], structures: vec![Structure { station: 50.0, kind: StructureKind::Weir(Weir { crest: 1.0, width: 1.0, c: WEIR_C }) }], boundary: Boundary::CriticalDepth, stations_downstream: false, contraction: CONTRACTION, expansion: EXPANSION };
        assert!(standard_step(&p).unwrap_err().contains("outside"));
        let bad = CrossSection { station: 0.0, points: vec![[1.0, 0.0], [0.0, 1.0]], n: 0.03, contraction: None, expansion: None };
        assert!(normal_level(&bad, 1.0, 0.001).is_err());
    }
}
