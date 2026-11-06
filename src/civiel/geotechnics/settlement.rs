//! Settlement of a fill on the soil model: the stress increase below the
//! fill (Boussinesq, integrated over rectangles), primary and secondary
//! compression per layer (Koppejan, or Terzaghi with Cc/(1+e0)), and the
//! time it takes (Terzaghi's one-dimensional consolidation).
//!
//! - **Stress below a rectangle corner** (Boussinesq 1885, integrated by
//!   Newmark 1935, tabulated by Fadum 1948): Δσz = q·I(m, n), m = B/z,
//!   n = L/z, with
//!   I = 1/(4π)·[2mn√(m²+n²+1)/(m²+n²+m²n²+1)·(m²+n²+2)/(m²+n²+1) +
//!   atan2(2mn√(m²+n²+1), m²+n²+1−m²n²)].
//!   A fill is a grid of rectangles of uniform load q = γ·h; under any
//!   point the four-corner superposition of every rectangle gives the exact
//!   stress of that stepped load.
//! - **Koppejan** (1948): ε = (1/Cp + log₁₀(t/t₀)/Cs)·ln(σ'p/σ'0) +
//!   (1/C'p + log₁₀(t/t₀)/C's)·ln(σ'e/σ'p), t₀ = 1 day, σ'p = σ'0 + POP,
//!   σ'e = σ'0 + Δσ (the first term only up to σ'e when σ'e < σ'p).
//! - **Terzaghi** (compression ratios): ε = RR·log₁₀(σ'p/σ'0) +
//!   CR·log₁₀(σ'e/σ'p), CR = Cc/(1+e0), RR = Csw/(1+e0); secondary
//!   compression Cα·log₁₀(t/t_p) after the end of primary consolidation t_p
//!   (taken at U = 95 %), after Mesri (1973).
//! - **Consolidation** (Terzaghi 1925): U(T) = 1 − Σ 2/M²·exp(−M²T),
//!   M = π(2k+1)/2, T = cv·t/H_dr². Consecutive clay, loam and peat layers
//!   between sand layers (or the surface) consolidate together; their
//!   thickness is transformed to that of the reference layer (the thickest),
//!   H' = Σ Hᵢ·√(cv,ref/cv,ᵢ). The drainage length is H'/2 with sand below,
//!   H' when the package rests on an impermeable base.
//!
//! In Koppejan's method the secular (log t) part is taken to develop with
//! the degree of consolidation: s(t) = U(t)·s_K(t); for Terzaghi
//! s(t) = U(t)·s_p + s_s(t).

use serde::Serialize;

use super::model::Band;
use super::soil::{Params, GAMMA_W};

/// Influence factor for the vertical stress below the corner of a uniformly
/// loaded rectangle B × L at depth z (m = B/z, n = L/z).
pub fn corner_influence(m: f64, n: f64) -> f64 {
    if m <= 0.0 || n <= 0.0 {
        return 0.0;
    }
    let (m2, n2) = (m * m, n * n);
    let a = m2 + n2 + 1.0;
    let s = a.sqrt();
    let t1 = 2.0 * m * n * s / (m2 + n2 + m2 * n2 + 1.0) * (m2 + n2 + 2.0) / a;
    let t2 = (2.0 * m * n * s).atan2(a - m2 * n2);
    (t1 + t2) / (4.0 * std::f64::consts::PI)
}

/// A rectangle of uniform load q (kPa) from (x0, y0) to (x1, y1).
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub q: f64,
}

/// The vertical stress increase (kPa) at depth z (m) below (px, py) from a
/// set of loaded rectangles (four-corner superposition).
pub fn stress(rects: &[Rect], px: f64, py: f64, z: f64) -> f64 {
    let z = z.max(1e-3);
    let f = |a: f64, b: f64| -> f64 { a.signum() * b.signum() * corner_influence(a.abs() / z, b.abs() / z) };
    rects
        .iter()
        .map(|r| {
            let (a0, a1, b0, b1) = (r.x0 - px, r.x1 - px, r.y0 - py, r.y1 - py);
            r.q * (f(a1, b1) - f(a0, b1) - f(a1, b0) + f(a0, b0))
        })
        .sum()
}

/// Terzaghi's average degree of consolidation at time factor T.
pub fn degree(t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    // Small T: the series converges slowly; √(4T/π) is exact to 1e-6 there.
    if t < 0.01 {
        return (4.0 * t / std::f64::consts::PI).sqrt();
    }
    let mut s = 0.0;
    for k in 0..100 {
        let m = std::f64::consts::PI * (2 * k + 1) as f64 / 2.0;
        let term = 2.0 / (m * m) * (-m * m * t).exp();
        s += term;
        if term < 1e-12 {
            break;
        }
    }
    (1.0 - s).clamp(0.0, 1.0)
}

/// The time factor for a degree of consolidation (bisection on [`degree`]).
pub fn time_factor(u: f64) -> f64 {
    let u = u.clamp(1e-6, 0.999_999);
    let (mut lo, mut hi) = (0.0, 10.0);
    for _ in 0..100 {
        let m = 0.5 * (lo + hi);
        if degree(m) < u { lo = m } else { hi = m }
    }
    0.5 * (lo + hi)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Method {
    Koppejan,
    Terzaghi,
}

impl Method {
    pub fn parse(s: &str) -> Option<Method> {
        match s.to_ascii_lowercase().as_str() {
            "koppejan" | "k" => Some(Method::Koppejan),
            "terzaghi" | "t" | "cc" | "ccc" => Some(Method::Terzaghi),
            _ => None,
        }
    }
}

pub const DAY: f64 = 86_400.0;
pub const YEAR: f64 = 365.25 * DAY;

/// Primary and secondary strain of one sublayer (Koppejan at time `t_days`,
/// or Terzaghi's primary strain with secondary 0 here: [`settlement`] adds
/// Cα after the end of primary consolidation).
pub fn strain(p: &Params, s0: f64, ds: f64, method: Method, t_days: f64) -> (f64, f64) {
    let s0 = s0.max(1.0);
    let se = s0 + ds.max(0.0);
    let sp = s0 + p.pop.max(0.0);
    match method {
        Method::Koppejan => {
            let lt = (t_days.max(1.0)).log10();
            let (below, above) = if se <= sp { ((se / s0).ln(), 0.0) } else { ((sp / s0).ln(), (se / sp).ln()) };
            let prim = below / p.cp_pre + above / p.cp;
            let sec = p.cs_pre.map(|c| lt / c * below).unwrap_or(0.0) + p.cs.map(|c| lt / c * above).unwrap_or(0.0);
            (prim, sec)
        }
        Method::Terzaghi => {
            let (below, above) = if se <= sp { ((se / s0).log10(), 0.0) } else { ((sp / s0).log10(), (se / sp).log10()) };
            (p.rr * below + p.cr * above, 0.0)
        }
    }
}

/// One layer at a point, for the report.
#[derive(Clone, Debug, Serialize)]
pub struct LayerResult {
    pub code: String,
    pub top: f64,
    pub bottom: f64,
    /// Effective stress before and stress increase at the middle (kPa).
    pub sigma0: f64,
    pub dsigma: f64,
    /// Primary and secondary settlement (m) at the end time.
    pub primary: f64,
    pub secondary: f64,
    /// Package of consolidating layers it belongs to (None: drains freely).
    pub package: Option<usize>,
}

/// A package of layers that consolidate together.
#[derive(Clone, Debug, Serialize)]
pub struct Package {
    pub top: f64,
    pub bottom: f64,
    /// Transformed thickness, drainage length (m) and reference cv (m²/s).
    pub h_eq: f64,
    pub drainage: f64,
    pub cv: f64,
    pub double: bool,
    /// Time to 50 and 90 % consolidation (days).
    pub t50: f64,
    pub t90: f64,
}

/// The settlement at one point.
#[derive(Clone, Debug, Serialize)]
pub struct PointResult {
    /// Final primary settlement (U = 1) and total at the end time (m).
    pub primary: f64,
    pub total: f64,
    pub layers: Vec<LayerResult>,
    pub packages: Vec<Package>,
}

/// What a point needs: the bands below the ground (top first, the first
/// band's top is the ground level), the phreatic level, the parameters.
pub struct Column<'a> {
    pub bands: &'a [Band],
    pub water: f64,
    pub params: &'a dyn Fn(&str) -> Params,
}

/// Sublayers of at most this thickness (m).
const SUB: f64 = 0.5;

/// Settlement at one point at time `t_end` (s), with `ds(depth)` the stress
/// increase (kPa) at a depth below the ground.
pub fn settlement(col: &Column, ds: &dyn Fn(f64) -> f64, method: Method, t_end: f64) -> PointResult {
    let ground = col.bands.first().map(|b| b.top).unwrap_or(0.0);
    let params: Vec<Params> = col.bands.iter().map(|b| (col.params)(&b.code)).collect();
    // Packages: consecutive layers with a cv, between drains.
    let mut package_of: Vec<Option<usize>> = vec![None; col.bands.len()];
    let mut packages: Vec<Package> = Vec::new();
    let mut i = 0;
    while i < col.bands.len() {
        if params[i].cv.is_none() {
            i += 1;
            continue;
        }
        let start = i;
        while i < col.bands.len() && params[i].cv.is_some() {
            i += 1;
        }
        let members = start..i;
        let reference = members.clone().max_by(|a, b| col.bands[*a].thickness().total_cmp(&col.bands[*b].thickness())).unwrap_or(start);
        let cv_ref = params[reference].cv.unwrap_or(1e-7);
        let h_eq: f64 = members.clone().map(|k| col.bands[k].thickness() * (cv_ref / params[k].cv.unwrap_or(cv_ref)).sqrt()).sum();
        let double = i < col.bands.len();
        let drainage = if double { h_eq / 2.0 } else { h_eq };
        let t_of = |u: f64| time_factor(u) * drainage * drainage / cv_ref / DAY;
        for k in members {
            package_of[k] = Some(packages.len());
        }
        packages.push(Package { top: col.bands[start].top, bottom: col.bands[i - 1].bottom, h_eq, drainage, cv: cv_ref, double, t50: t_of(0.5), t90: t_of(0.9) });
    }
    let t_days = t_end / DAY;
    let mut layers = Vec::new();
    let (mut primary, mut total) = (0.0, 0.0);
    let mut sigma = 0.0;
    for (k, b) in col.bands.iter().enumerate() {
        let p = &params[k];
        let h = b.thickness();
        if h <= 0.0 {
            continue;
        }
        let n = (h / SUB).ceil().max(1.0) as usize;
        let dh = h / n as f64;
        let (mut lp, mut ls) = (0.0, 0.0);
        let (mut mid_s0, mut mid_ds) = (0.0, 0.0);
        let u_end = package_of[k].map(|j| degree(packages[j].cv * t_end / packages[j].drainage.powi(2))).unwrap_or(1.0);
        let t_p = package_of[k].map(|j| time_factor(0.95) * packages[j].drainage.powi(2) / packages[j].cv / DAY).unwrap_or(1.0);
        for s in 0..n {
            let top = b.top - s as f64 * dh;
            let mid = top - dh / 2.0;
            // σ'0 at the middle: the weight above, less the pore pressure.
            let s_above = sigma + weight(p, top, mid, col.water);
            let u = GAMMA_W * (col.water - mid).max(0.0);
            let s0 = (s_above - u).max(1.0);
            let d = ds(ground - mid);
            let (ep, es) = strain(p, s0, d, method, t_days);
            let es = match method {
                Method::Koppejan => es,
                // Mesri: Cα·log(t/t_p) after the end of primary consolidation.
                Method::Terzaghi if d > 0.0 && t_days > t_p => p.ca * (t_days / t_p).log10(),
                Method::Terzaghi => 0.0,
            };
            lp += ep * dh;
            ls += es * dh;
            if s == n / 2 {
                mid_s0 = s0;
                mid_ds = d;
            }
            sigma += weight(p, top, top - dh, col.water);
        }
        primary += lp;
        total += match method {
            Method::Koppejan => u_end * (lp + ls),
            Method::Terzaghi => u_end * lp + ls,
        };
        layers.push(LayerResult { code: b.code.clone(), top: b.top, bottom: b.bottom, sigma0: mid_s0, dsigma: mid_ds, primary: lp, secondary: ls, package: package_of[k] });
    }
    PointResult { primary, total, layers, packages }
}

/// The settlement over time at one point: (days, m), log-spaced from one
/// day to `t_end` (s), recomputed at each time (Koppejan's log t grows).
pub fn curve(col: &Column, ds: &dyn Fn(f64) -> f64, method: Method, t_end: f64, points: usize) -> Vec<(f64, f64)> {
    let days = (t_end / DAY).max(2.0);
    (0..points.max(2))
        .map(|i| {
            let d = 10f64.powf(days.log10() * i as f64 / (points.max(2) - 1) as f64);
            (d, settlement(col, ds, method, d * DAY).total)
        })
        .collect()
}

/// Weight (kN/m²) of a unit between two levels, γ above the water, γsat below.
fn weight(p: &Params, top: f64, bottom: f64, water: f64) -> f64 {
    let (hi, lo) = (top.max(bottom), top.min(bottom));
    let wet = (water.min(hi) - lo).max(0.0);
    wet * p.gamma_sat + (hi - lo - wet) * p.gamma
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::geotechnics::soil;

    /// The influence values of Fadum (1948), "Influence values for estimating
    /// stresses in elastic foundations", Proc. 2nd ICSMFE Rotterdam, vol. 3,
    /// pp. 77–84, from Newmark's (1935) integration of Boussinesq's solution.
    #[test]
    fn rectangle_corner_values() {
        for (m, n, i) in [(1.0, 1.0, 0.1752), (2.0, 2.0, 0.2325), (0.5, 0.5, 0.0840), (0.5, 1.0, 0.1202), (1.0, 2.0, 0.1999), (1e6, 1e6, 0.25)] {
            assert!((corner_influence(m, n) - i).abs() < 6e-5, "I({m},{n}) = {}", corner_influence(m, n));
        }
        // Under the centre of a 2 × 2 m square at 1 m: four corners of 1 × 1.
        let sq = [Rect { x0: -1.0, y0: -1.0, x1: 1.0, y1: 1.0, q: 100.0 }];
        assert!((stress(&sq, 0.0, 0.0, 1.0) - 4.0 * 100.0 * 0.1752).abs() < 0.05);
        // Against a numerical integration of Boussinesq's point load.
        let (px, py, z) = (1.7, 0.4, 1.3);
        let mut num = 0.0;
        let k = 400;
        for i in 0..k {
            for j in 0..k {
                let x = -1.0 + 2.0 * (i as f64 + 0.5) / k as f64;
                let y = -1.0 + 2.0 * (j as f64 + 0.5) / k as f64;
                let r2 = (x - px).powi(2) + (y - py).powi(2) + z * z;
                num += 3.0 * 100.0 * (2.0 / k as f64).powi(2) * z.powi(3) / (2.0 * std::f64::consts::PI * r2.powf(2.5));
            }
        }
        assert!((stress(&sq, px, py, z) - num).abs() < 0.01 * num, "{} vs {num}", stress(&sq, px, py, z));
        // A wide area: the full load at depth.
        let wide = [Rect { x0: -500.0, y0: -500.0, x1: 500.0, y1: 500.0, q: 27.0 }];
        assert!((stress(&wide, 0.0, 0.0, 5.0) - 27.0).abs() < 0.01);
    }

    /// Terzaghi's time factors: T = 0.197 for U = 50 %, 0.848 for 90 %
    /// (e.g. Das, Principles of Geotechnical Engineering, 7th ed., §11.11).
    /// Das's Example 11.13 (p. 356): T = 0.36 m²/month × 9 months / 3² m
    /// = 0.36 gives U ≈ 0.67.
    #[test]
    fn consolidation_degree() {
        assert!((degree(0.197) - 0.50).abs() < 0.001);
        assert!((degree(0.848) - 0.90).abs() < 0.001);
        assert!((time_factor(0.9) - 0.848).abs() < 0.001);
        let t = 0.36 * 9.0 / (3.0f64 * 3.0);
        assert!((degree(t) - 0.67).abs() < 0.01, "{}", degree(t));
        assert!((degree(0.005) - (4.0 * 0.005 / std::f64::consts::PI).sqrt()).abs() < 1e-6);
    }

    /// Das, Principles of Geotechnical Engineering, 7th ed. (Cengage, ISBN
    /// 978-1-133-10866-5), Example 11.4 (a), p. 323: a normally consolidated
    /// clay layer 4 m thick, Cc = 0.009(LL − 10) = 0.27 with LL = 40,
    /// e0 = 0.8, σ'0 = 2·14 + 4·(18 − 9.81) + 2·(19 − 9.81) = 79.14 kPa at
    /// its middle, Δσ = 100 kPa: Sc = Cc·H/(1+e0)·log₁₀((σ'0+Δσ)/σ'0)
    /// = 0.21 m.
    #[test]
    fn das_example_11_4_single_clay_layer() {
        let mut p = soil::row("klei_schoon_matig").unwrap().params();
        p.cr = 0.27 / 1.8;
        p.pop = 0.0;
        let s0 = 2.0 * 14.0 + 4.0 * (18.0 - 9.81) + 2.0 * (19.0 - 9.81);
        let (e, _) = strain(&p, s0, 100.0, Method::Terzaghi, 1.0);
        let sc = e * 4.0;
        assert!((sc - 0.213).abs() < 0.001, "{sc}");
        assert!((sc - 0.21).abs() < 0.005);
        // The same layer by sublayers through the full calculation: 2 m dry
        // sand (14), 4 m sand under water (18), the clay (19) below.
        let mut sand_dry = soil::row("zand_schoon_matig").unwrap().params();
        sand_dry.gamma = 14.0;
        sand_dry.gamma_sat = 18.0;
        sand_dry.cr = 0.0;
        sand_dry.rr = 0.0;
        let mut clay = p.clone();
        clay.gamma = 19.0;
        clay.gamma_sat = 19.0;
        let bands = vec![
            Band { top: 0.0, bottom: -6.0, code: "zand".into() },
            Band { top: -6.0, bottom: -10.0, code: "klei".into() },
        ];
        let lookup = move |c: &str| if c == "zand" { sand_dry.clone() } else { clay.clone() };
        let col = Column { bands: &bands, water: -2.0, params: &lookup };
        let r = settlement(&col, &|_| 100.0, Method::Terzaghi, 100.0 * YEAR);
        // Sublayers integrate the log over the layer: a little more.
        assert!((r.primary - 0.213).abs() < 0.01, "{}", r.primary);
        // The clay rests on the model base: drained at its top only.
        assert!(r.packages.len() == 1 && !r.packages[0].double);
    }

    /// Koppejan by hand: soft clay, C'p = 7, C's = 80, σ'0 = 20, Δσ = 27 kPa,
    /// t = 10 000 days: ε = (1/7 + 4/80)·ln(47/20).
    #[test]
    fn koppejan_by_hand() {
        let p = soil::row("klei_schoon_slap").unwrap().params();
        let (ep, es) = strain(&p, 20.0, 27.0, Method::Koppejan, 10_000.0);
        let r = (47.0f64 / 20.0).ln();
        assert!((ep - r / 7.0).abs() < 1e-12 && (es - 4.0 / 80.0 * r).abs() < 1e-12);
        // With a POP of 10 kPa the first 10 kPa follow Cp ≈ 3 C'p.
        let mut q = p.clone();
        q.pop = 10.0;
        let (ep2, _) = strain(&q, 20.0, 27.0, Method::Koppejan, 1.0);
        assert!(ep2 < ep && (ep2 - ((30.0f64 / 20.0).ln() / q.cp_pre + (47.0f64 / 30.0).ln() / 7.0)).abs() < 1e-12);
    }

    #[test]
    fn a_clay_layer_between_sands_consolidates_from_both_sides() {
        let sand = soil::row("zand_schoon_vast").unwrap().params();
        let mut clay = soil::row("klei_schoon_slap").unwrap().params();
        clay.cv = Some(1e-8);
        let bands = vec![
            Band { top: 0.0, bottom: -1.0, code: "zand".into() },
            Band { top: -1.0, bottom: -5.0, code: "klei".into() },
            Band { top: -5.0, bottom: -15.0, code: "zand".into() },
        ];
        let lookup = move |c: &str| if c == "zand" { sand.clone() } else { clay.clone() };
        let col = Column { bands: &bands, water: -0.5, params: &lookup };
        let r = settlement(&col, &|_| 30.0, Method::Koppejan, 30.0 * YEAR);
        let pk = &r.packages[0];
        assert!(pk.double && (pk.drainage - 2.0).abs() < 1e-9);
        // t90 = 0.848 · 2² / 1e-8 s.
        assert!((pk.t90 - 0.848 * 4.0 / 1e-8 / DAY).abs() / pk.t90 < 0.002);
        let c = curve(&col, &|_| 30.0, Method::Koppejan, 30.0 * YEAR, 12);
        assert!(c.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-12) && c[0].1 < c[11].1 * 0.2);
        assert!((c[11].1 - r.total).abs() < 1e-9);
    }
}
