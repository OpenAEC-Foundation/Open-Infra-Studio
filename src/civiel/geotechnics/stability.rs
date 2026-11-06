//! Slope stability with circular slip surfaces: Bishop's simplified method
//! and the ordinary method of slices (Fellenius), in effective stresses,
//! with pore pressures from a phreatic line, free water, a surcharge strip
//! and a search for the critical circle.
//!
//! - **Ordinary method of slices** (Fellenius 1936):
//!   F = Σ [c'·b/cos α + max(0, W cos α − u·b/cos α)·tan φ'] / Σ W sin α.
//! - **Bishop simplified** (Bishop 1955, Géotechnique 5(1), 7–17):
//!   F = Σ [c'·b + (W − u·b)·tan φ'] / m_α / Σ W sin α, with
//!   m_α = cos α + sin α·tan φ' / F, solved by iteration from the
//!   Fellenius value. A circle with m_α < 0.2 in any slice is rejected
//!   (Whitman & Bailey 1967, J. Soil Mech. Found. Div. ASCE 93(SM4)).
//!
//! W is the slice weight (γ above the phreatic level, γsat below, plus free
//! water standing on the surface and the surcharge), b the slice width, α
//! the base inclination, u = γw·(phreatic level − base level) at the base
//! (hydrostatic below the phreatic line, the usual simplification).
//!
//! **Search.** A grid of centres above the slope, each with a range of
//! radii: circles tangent to levels from just below the lowest ground down
//! to the model base, to six levels within the slope height, and to every
//! layer boundary in that range.
//! The best circle is then refined by a compass (pattern) search on centre
//! and tangent level until the steps are below 2 cm. Circles must enter and
//! leave the ground inside the section and reach at least `min_depth`.

use serde::Serialize;

use super::soil::{Params, GAMMA_W};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Method {
    Bishop,
    Fellenius,
}

impl Method {
    pub fn parse(s: &str) -> Option<Method> {
        match s.to_ascii_lowercase().as_str() {
            "bishop" | "b" => Some(Method::Bishop),
            "fellenius" | "f" | "ordinary" | "omsl" | "zweeds" => Some(Method::Fellenius),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Method::Bishop => "Bishop",
            Method::Fellenius => "Fellenius",
        }
    }
}

/// A soil band in a section column: top and bottom (m), unit index.
#[derive(Clone, Debug)]
pub struct SBand {
    pub top: f64,
    pub bottom: f64,
    pub unit: usize,
}

/// A surcharge q (kPa) on the surface from x `from` to `to` (m).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Load {
    pub q: f64,
    pub from: f64,
    pub to: f64,
}

/// A cross section: x along the section (m), z level (m).
#[derive(Clone, Debug)]
pub struct Section {
    /// Ground or design surface, x increasing.
    pub surface: Vec<[f64; 2]>,
    /// Soil columns at increasing x; a slice takes the nearest one.
    pub columns: Vec<(f64, Vec<SBand>)>,
    pub units: Vec<Params>,
    /// The phreatic line, x increasing (constant beyond its ends).
    pub phreatic: Vec<[f64; 2]>,
    pub load: Option<Load>,
    /// No slip surface below this level.
    pub base: f64,
}

fn interp(line: &[[f64; 2]], x: f64) -> Option<f64> {
    if line.is_empty() {
        return None;
    }
    if x <= line[0][0] {
        return Some(line[0][1]);
    }
    if x >= line[line.len() - 1][0] {
        return Some(line[line.len() - 1][1]);
    }
    // The segment by binary search (x increasing).
    let i = line.partition_point(|p| p[0] < x).clamp(1, line.len() - 1);
    let (a, b) = (line[i - 1], line[i]);
    let d = b[0] - a[0];
    Some(if d.abs() < 1e-12 { b[1] } else { a[1] + (b[1] - a[1]) * (x - a[0]) / d })
}

impl Section {
    pub fn ground(&self, x: f64) -> f64 {
        interp(&self.surface, x).unwrap_or(0.0)
    }

    pub fn water(&self, x: f64) -> f64 {
        interp(&self.phreatic, x).unwrap_or(f64::MIN)
    }

    fn column(&self, x: f64) -> &[SBand] {
        let i = self.columns.partition_point(|c| c.0 < x);
        let pick = match (i.checked_sub(1), self.columns.get(i)) {
            (Some(a), Some(b)) => if x - self.columns[a].0 <= b.0 - x { a } else { i },
            (Some(a), None) => a,
            (None, _) => 0,
        };
        self.columns.get(pick).map(|c| c.1.as_slice()).unwrap_or(&[])
    }

    /// The unit at level z under x (the top unit above the column, the
    /// bottom one below it).
    pub fn unit_at(&self, x: f64, z: f64) -> usize {
        let c = self.column(x);
        c.iter().find(|b| z <= b.top + 1e-9 && z >= b.bottom - 1e-9).or_else(|| if c.first().is_some_and(|b| z > b.top) { c.first() } else { c.last() }).map(|b| b.unit).unwrap_or(0)
    }

    /// Weight per m² of plan of the soil between `z_base` and `z_top` at x
    /// (kN/m²): γ above the phreatic level, γsat below.
    fn column_weight(&self, x: f64, z_base: f64, z_top: f64) -> f64 {
        let zw = self.water(x);
        let c = self.column(x);
        let mut total = 0.0;
        let part = |lo: f64, hi: f64, p: &Params| -> f64 {
            if hi <= lo {
                return 0.0;
            }
            let wet = (zw.min(hi) - lo).max(0.0);
            let dry = (hi - lo) - wet;
            wet * p.gamma_sat + dry * p.gamma
        };
        let mut covered_top = f64::MIN;
        let mut covered_bottom = f64::MAX;
        for b in c {
            let (lo, hi) = (b.bottom.max(z_base), b.top.min(z_top));
            total += part(lo, hi, &self.units[b.unit]);
            covered_top = covered_top.max(b.top);
            covered_bottom = covered_bottom.min(b.bottom);
        }
        // Above or below the column: its top or bottom unit.
        if let (Some(first), Some(last)) = (c.first(), c.last()) {
            if z_top > covered_top {
                total += part(covered_top.max(z_base), z_top, &self.units[first.unit]);
            }
            if z_base < covered_bottom {
                total += part(z_base, covered_bottom.min(z_top), &self.units[last.unit]);
            }
        }
        total
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Circle {
    pub xc: f64,
    pub zc: f64,
    pub r: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Slice {
    /// Middle and width (m).
    pub x: f64,
    pub b: f64,
    pub z_top: f64,
    pub z_base: f64,
    /// Base inclination (degrees), positive where the base rises in the
    /// direction the soil slides from.
    pub alpha: f64,
    /// Weight including free water and surcharge (kN/m).
    pub w: f64,
    /// Pore pressure at the base (kPa).
    pub u: f64,
    pub c: f64,
    pub phi: f64,
    pub unit: usize,
}

/// A circle cut into slices: where it enters and leaves the ground and the
/// greatest depth of the slip surface below the ground.
#[derive(Clone, Debug)]
pub struct Cut {
    pub slices: Vec<Slice>,
    pub entry: f64,
    pub exit: f64,
    pub depth: f64,
}

/// The slices of a circle; `None` when it is not a valid slip surface.
pub fn slices(sec: &Section, c: &Circle, n: usize) -> Option<Cut> {
    let (x0, x1) = (sec.surface.first()?[0], sec.surface.last()?[0]);
    let lo = (c.xc - c.r).max(x0);
    let hi = (c.xc + c.r).min(x1);
    if hi - lo < 0.5 {
        return None;
    }
    let arc = |x: f64| c.zc - (c.r * c.r - (x - c.xc).powi(2)).max(0.0).sqrt();
    let f = |x: f64| sec.ground(x) - arc(x);
    // Sign changes on a fine sampling (and at the surface vertices).
    let steps = 240;
    let mut xs: Vec<f64> = (0..=steps).map(|i| lo + (hi - lo) * i as f64 / steps as f64).collect();
    xs.extend(sec.surface.iter().map(|p| p[0]).filter(|x| *x > lo && *x < hi));
    xs.sort_by(f64::total_cmp);
    let root = |mut a: f64, mut b: f64| {
        for _ in 0..50 {
            let m = 0.5 * (a + b);
            if (f(a) > 0.0) == (f(m) > 0.0) { a = m } else { b = m }
        }
        0.5 * (a + b)
    };
    let mut runs: Vec<(f64, f64, bool, bool)> = Vec::new();
    let mut start: Option<(f64, bool)> = None;
    for w in xs.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (fa, fb) = (f(a) > 0.0, f(b) > 0.0);
        if start.is_none() && fa {
            start = Some((a, true));
        }
        if !fa && fb {
            start = Some((root(a, b), false));
        }
        if fa && !fb {
            if let Some((s, open)) = start.take() {
                runs.push((s, root(a, b), open, false));
            }
        }
    }
    if let Some((s, open)) = start {
        runs.push((s, hi, open, true));
    }
    let (xa, xb, open_a, open_b) = runs.into_iter().max_by(|p, q| (p.1 - p.0).total_cmp(&(q.1 - q.0)))?;
    // Out of the section, or the circle's ends under ground.
    if open_a || open_b || xb - xa < 0.5 {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    let bw = (xb - xa) / n as f64;
    let mut depth = 0.0f64;
    for i in 0..n {
        let x = xa + (i as f64 + 0.5) * bw;
        let z_top = sec.ground(x);
        let z_base = arc(x);
        if z_base < sec.base - 1e-6 {
            return None;
        }
        depth = depth.max(z_top - z_base);
        let alpha = ((x - c.xc) / c.r).clamp(-1.0, 1.0).asin();
        let zw = sec.water(x);
        let mut w = sec.column_weight(x, z_base, z_top.max(z_base)) * bw;
        if zw > z_top {
            w += GAMMA_W * (zw - z_top) * bw;
        }
        if let Some(l) = sec.load {
            if x >= l.from.min(l.to) && x <= l.from.max(l.to) {
                w += l.q * bw;
            }
        }
        let u = GAMMA_W * (zw - z_base).max(0.0);
        let unit = sec.unit_at(x, z_base + 1e-6);
        let p = &sec.units[unit];
        out.push(Slice { x, b: bw, z_top, z_base, alpha: alpha.to_degrees(), w, u, c: p.c, phi: p.phi, unit });
    }
    Some(Cut { slices: out, entry: xa, exit: xb, depth })
}

/// Factor of safety of a set of slices; also the sliding direction (+1 when
/// the soil moves towards smaller x).
pub fn factor(slices: &[Slice], method: Method) -> Option<(f64, f64)> {
    let drive: f64 = slices.iter().map(|s| s.w * s.alpha.to_radians().sin()).sum();
    if drive.abs() < 1e-9 {
        return None;
    }
    let dir = drive.signum();
    let d = drive.abs();
    let fell = || -> f64 {
        slices
            .iter()
            .map(|s| {
                let a = dir * s.alpha.to_radians();
                let l = s.b / a.cos();
                s.c * l + (s.w * a.cos() - s.u * l).max(0.0) * s.phi.to_radians().tan()
            })
            .sum::<f64>()
            / d
    };
    let f0 = fell();
    match method {
        Method::Fellenius => Some((f0, dir)),
        Method::Bishop => {
            let mut f = f0.max(0.05);
            for _ in 0..200 {
                let mut num = 0.0;
                for s in slices {
                    let a = dir * s.alpha.to_radians();
                    let t = s.phi.to_radians().tan();
                    let m = a.cos() + a.sin() * t / f;
                    if m < 0.2 {
                        return None;
                    }
                    num += (s.c * s.b + (s.w - s.u * s.b).max(0.0) * t) / m;
                }
                let next = num / d;
                if (next - f).abs() < 1e-7 {
                    return Some((next, dir));
                }
                f = next;
            }
            Some((f, dir))
        }
    }
}

/// One evaluated circle.
#[derive(Clone, Debug, Serialize)]
pub struct Trial {
    pub circle: Circle,
    pub f: f64,
    /// +1: the soil slides towards smaller x; −1 towards larger x.
    pub direction: f64,
    pub entry: f64,
    pub exit: f64,
}

/// Search settings.
#[derive(Clone, Copy, Debug)]
pub struct Search {
    pub method: Method,
    pub nx: usize,
    pub nz: usize,
    pub tangents: usize,
    pub slices: usize,
    pub min_depth: f64,
}

impl Default for Search {
    fn default() -> Self {
        Search { method: Method::Bishop, nx: 21, nz: 15, tangents: 16, slices: 50, min_depth: 0.5 }
    }
}

pub fn evaluate(sec: &Section, c: &Circle, s: &Search, method: Method) -> Option<Trial> {
    if c.r < 0.5 {
        return None;
    }
    let cut = slices(sec, c, s.slices)?;
    if cut.depth < s.min_depth {
        return None;
    }
    let (f, dir) = factor(&cut.slices, method)?;
    (f.is_finite() && f > 0.0).then_some(Trial { circle: *c, f, direction: dir, entry: cut.entry, exit: cut.exit })
}

/// The outcome of a search: the critical circle, the grid of centres (each
/// with its lowest F) and how many circles were tried.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    pub critical: Trial,
    pub grid: Vec<[f64; 3]>,
    pub grid_box: [f64; 4],
    pub tried: usize,
}

/// The critical circle of a section.
pub fn search(sec: &Section, s: &Search) -> Option<Outcome> {
    let (x0, x1) = (sec.surface.first()?[0], sec.surface.last()?[0]);
    let (zmin, zmax) = sec.surface.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    let h = (zmax - zmin).max(1.0);
    let width = x1 - x0;
    let (gx0, gx1) = (x0 + 0.05 * width, x1 - 0.05 * width);
    let (gz0, gz1) = (zmax + 0.2 * h, zmax + 2.5 * h + 2.0);
    let deepest = (zmin - 2.0 * h - 5.0).max(sec.base + 0.05);
    // Tangent levels below the lowest ground, and some within the slope
    // height (slips in the slope face).
    let mut levels: Vec<f64> = (0..s.tangents.max(2)).map(|i| zmin - 0.1 - (zmin - 0.1 - deepest) * i as f64 / (s.tangents.max(2) - 1) as f64).collect();
    let face_top = zmax - s.min_depth;
    if face_top > zmin {
        levels.extend((0..6).map(|i| zmin + (face_top - zmin) * (i as f64 + 0.5) / 6.0));
    }
    for (_, col) in &sec.columns {
        for b in col {
            if b.bottom < zmax && b.bottom > deepest {
                levels.push(b.bottom - 0.01);
            }
        }
    }
    levels.sort_by(|a, b| b.total_cmp(a));
    levels.dedup_by(|a, b| (*a - *b).abs() < 0.02);
    let mut best: Option<Trial> = None;
    let mut tried = 0usize;
    let mut grid = Vec::new();
    let keep = |t: Option<Trial>, best: &mut Option<Trial>| {
        if let Some(t) = t {
            if best.as_ref().is_none_or(|b| t.f < b.f) {
                *best = Some(t);
            }
        }
    };
    for i in 0..s.nx {
        let xc = gx0 + (gx1 - gx0) * i as f64 / (s.nx - 1).max(1) as f64;
        for j in 0..s.nz {
            let zc = gz0 + (gz1 - gz0) * j as f64 / (s.nz - 1).max(1) as f64;
            let mut here: Option<Trial> = None;
            for zt in &levels {
                tried += 1;
                keep(evaluate(sec, &Circle { xc, zc, r: zc - zt }, s, s.method), &mut here);
            }
            if let Some(t) = &here {
                grid.push([xc, zc, t.f]);
            }
            keep(here, &mut best);
        }
    }
    let mut cur = best?;
    // Compass search on (xc, zc, tangent level).
    let mut steps = [(gx1 - gx0) / (s.nx - 1).max(1) as f64, (gz1 - gz0) / (s.nz - 1).max(1) as f64, (zmin - deepest) / s.tangents.max(2) as f64];
    while steps.iter().any(|d| *d > 0.02) {
        let mut moved = false;
        for k in 0..3 {
            for sign in [-1.0, 1.0] {
                let c = cur.circle;
                let zt = c.zc - c.r;
                let (mut xc, mut zc, mut zt2) = (c.xc, c.zc, zt);
                match k {
                    0 => xc += sign * steps[0],
                    1 => zc += sign * steps[1],
                    _ => zt2 += sign * steps[2],
                }
                tried += 1;
                if let Some(t) = evaluate(sec, &Circle { xc, zc, r: zc - zt2 }, s, s.method) {
                    if t.f < cur.f - 1e-9 {
                        cur = t;
                        moved = true;
                    }
                }
            }
        }
        if !moved {
            for d in steps.iter_mut() {
                *d *= 0.5;
            }
        }
    }
    Some(Outcome { critical: cur, grid, grid_box: [gx0, gz0, gx1, gz1], tried })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::geotechnics::soil;

    fn homogeneous(surface: Vec<[f64; 2]>, base: f64, c: f64, phi: f64, gamma: f64) -> Section {
        let mut p = soil::row("klei_schoon_matig").unwrap().params();
        p.c = c;
        p.phi = phi;
        p.gamma = gamma;
        p.gamma_sat = gamma;
        Section { surface, columns: vec![(0.0, vec![SBand { top: 1000.0, bottom: base, unit: 0 }])], units: vec![p], phreatic: vec![[0.0, base - 100.0]], load: None, base }
    }

    /// ACADS test problem 1(a), the simple homogeneous slope of Giam &
    /// Donald (1989), "Example problems for testing soil slope stability
    /// programs", Civil Engineering Research Report No. 8/1989, Monash
    /// University: slope 2:1 from (30, 25) to (50, 35), base at 20 m,
    /// c' = 3 kPa, φ' = 19.6°, γ = 20 kN/m³, no water. The referee factor
    /// of safety is 1.00 (also in Donald 1994, Proc. XIII ICSMFE New Delhi,
    /// "The extended CRISS method", Example 1, where a Bishop circular
    /// analysis gives 1.004). Published runs of the problem give about 0.99
    /// for Bishop and about 0.95 for the ordinary method of slices.
    #[test]
    fn acads_simple_slope() {
        let sec = homogeneous(vec![[20.0, 25.0], [30.0, 25.0], [50.0, 35.0], [70.0, 35.0]], 20.0, 3.0, 19.6, 20.0);
        let b = search(&sec, &Search::default()).unwrap();
        let f = search(&sec, &Search { method: Method::Fellenius, ..Search::default() }).unwrap();
        let (fb, ff) = (b.critical.f, f.critical.f);
        println!("ACADS 1(a): Bishop {fb:.4} at {:?}, Fellenius {ff:.4} at {:?}", b.critical.circle, f.critical.circle);
        assert!((0.97..=1.01).contains(&fb), "Bishop {fb:.3} at {:?}", b.critical.circle);
        assert!((0.92..=0.97).contains(&ff), "Fellenius {ff:.3}");
        assert!(ff < fb);
        // The soil slides down to the toe (towards smaller x).
        assert_eq!(b.critical.direction, 1.0);
        assert!(b.critical.entry < 31.0 && b.critical.exit > 48.0, "{:?}", b.critical);
    }

    /// φ = 0: both methods give the exact circular solution, which for a
    /// 60° slope is a toe circle with Taylor's stability number
    /// Ns = c/(F γ H) = 0.191 (Taylor 1937, "Stability of earth slopes",
    /// J. Boston Soc. Civil Eng. 24(3), 197–246, chart for φ = 0).
    #[test]
    fn taylor_undrained_toe_circle() {
        let h = 10.0;
        let run = h / 60f64.to_radians().tan();
        let sec = homogeneous(vec![[0.0, 0.0], [30.0, 0.0], [30.0 + run, h], [80.0, h]], -30.0, 20.0, 0.0, 20.0);
        for m in [Method::Bishop, Method::Fellenius] {
            let o = search(&sec, &Search { method: m, ..Search::default() }).unwrap();
            let ns = 20.0 / (o.critical.f * 20.0 * h);
            println!("Taylor φ = 0, 60°: {m:?} F {:.4}, Ns {ns:.4}", o.critical.f);
            assert!((ns - 0.191).abs() < 0.004, "{m:?}: Ns {ns:.4} at {:?}", o.critical);
        }
    }

    /// A dry cohesionless slope fails in a shallow slip parallel to its face:
    /// F → tan φ' / tan β (the infinite slope, e.g. Craig's Soil Mechanics).
    #[test]
    fn cohesionless_slope_tends_to_the_infinite_slope() {
        let sec = homogeneous(vec![[0.0, 0.0], [20.0, 0.0], [40.0, 10.0], [60.0, 10.0]], -10.0, 0.0, 30.0, 18.0);
        let o = search(&sec, &Search { min_depth: 0.2, ..Search::default() }).unwrap();
        let inf = 30f64.to_radians().tan() / 0.5;
        println!("infinite slope: F {:.4} against tan φ'/tan β {inf:.4}", o.critical.f);
        assert!(o.critical.f >= inf - 0.01 && o.critical.f < inf * 1.06, "{} vs {inf}", o.critical.f);
    }

    #[test]
    fn water_lowers_the_factor_and_a_load_too() {
        let mut sec = homogeneous(vec![[0.0, 0.0], [10.0, 0.0], [16.0, 3.0], [40.0, 3.0]], -10.0, 5.0, 25.0, 18.0);
        sec.units[0].gamma_sat = 19.0;
        let dry = search(&sec, &Search::default()).unwrap().critical.f;
        sec.phreatic = vec![[0.0, -0.5], [40.0, 2.0]];
        let wet = search(&sec, &Search::default()).unwrap().critical.f;
        sec.load = Some(Load { q: 20.0, from: 16.0, to: 22.0 });
        let loaded = search(&sec, &Search::default()).unwrap().critical.f;
        assert!(wet < dry && loaded < wet, "{dry} {wet} {loaded}");
        // A slope the other way round slides the other way.
        let mirror = homogeneous(vec![[0.0, 3.0], [24.0, 3.0], [30.0, 0.0], [40.0, 0.0]], -10.0, 5.0, 25.0, 18.0);
        let o = search(&mirror, &Search::default()).unwrap();
        assert_eq!(o.critical.direction, -1.0);
        assert!((o.critical.f - dry).abs() < 0.02, "{} vs {dry}", o.critical.f);
    }

    #[test]
    fn bishop_equals_fellenius_without_friction() {
        let sec = homogeneous(vec![[0.0, 0.0], [10.0, 0.0], [20.0, 5.0], [30.0, 5.0]], -10.0, 15.0, 0.0, 18.0);
        let c = Circle { xc: 16.0, zc: 12.0, r: 14.0 };
        let sl = slices(&sec, &c, 40).unwrap().slices;
        let b = factor(&sl, Method::Bishop).unwrap().0;
        let f = factor(&sl, Method::Fellenius).unwrap().0;
        assert!((b - f).abs() < 1e-9);
    }
}
