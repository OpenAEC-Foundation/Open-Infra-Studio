//! Closing the earthwork balance (#68): the plateau level where cut and fill
//! balance, and a design profile whose levels are shifted, within their grade
//! limits, so that little soil leaves or enters the site and little is hauled
//! along it. `civiel::grading` (CIV_PLATEAU, CIV_GRONDBALANS) and
//! `civiel::massa` compute the volumes and the mass haul of a given design;
//! this searches the design.
//!
//! Factor: `factor` is the in-situ cut needed per m³ of compacted fill, so
//! balance means `cut = factor · fill`. With the bulking factor (loose m³ per
//! m³ in situ) and compaction factor (loose m³ per m³ compacted fill) of
//! `civiel::grading`, `factor = compaction / bulking`; with the volume factor
//! of `civiel::massa` (cut × factor goes into fill), `factor = 1 / that`.
//! Default 1.0.
//!
//! Balance level: cut − factor·fill falls monotonically as the level rises,
//! so the level is found by Brent's bracketing root finder (Brent 1973,
//! *Algorithms for Minimization without Derivatives*, ch. 4; as `zbrent` in
//! Press et al., *Numerical Recipes*, §9.3). The volumes come from a closure
//! (e.g. a TIN-to-TIN volume by the caller) or from [`Plateau`], a grid of
//! existing heights in an outline with optional side slopes.
//!
//! Profile: PVIs joined by straight grades (no vertical curves). The
//! objective is |net| + w·haul, with net = cut − factor·fill (m³) and haul =
//! ∫|M(s)| ds in m³·km, M the mass ordinate (cumulative cut − factor·fill
//! from the start: the area between the mass haul curve and the balance
//! line, the classic mass haul diagram measure; massa.rs computes the same
//! curve). The search is a compass (pattern) search — Kolda, Lewis & Torczon
//! (2003), "Optimization by direct search: new perspectives on some classical
//! and modern methods", SIAM Review 45(3) — over three kinds of move: one
//! PVI, all PVIs together, and one PVI with an opposite shift of all that
//! keeps the mean level (and so roughly the net) the same, which lets the
//! haul fall without upsetting the balance. Every trial profile is projected
//! onto the level bounds and grade limits; the step halves when no move
//! helps. Robust and derivative-free (the objective has kinks), not globally
//! optimal.
//!
//! The weight w (m³ of net balance per m³·km of haul, default 0.1) is a
//! cost ratio to take from the unit prices of transport off site and of
//! haul on site; it is not a norm value.

use serde::{Deserialize, Serialize};

use crate::bui::Grid;
use crate::P2;

/// Cut and fill volumes (m³, in situ and compacted).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CutFill {
    pub cut: f64,
    pub fill: f64,
}

/// A balanced plateau level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelBalance {
    pub level: f64,
    pub cut: f64,
    pub fill: f64,
    /// cut − factor · fill (m³): > 0 surplus, < 0 shortage.
    pub net: f64,
    pub factor: f64,
    /// Volume evaluations used.
    pub evaluations: usize,
}

/// No balance within the bounds, with the best bound when there is one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unbalanced {
    pub message: String,
    pub best: Option<LevelBalance>,
}

impl std::fmt::Display for Unbalanced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<Unbalanced> for String {
    fn from(u: Unbalanced) -> String {
        u.message
    }
}

fn invalid(message: String) -> Unbalanced {
    Unbalanced { message, best: None }
}

/// The level in `[min, max]` where `cut = factor · fill`, to within `tol`
/// (m), with `volumes(level)` giving cut and fill at a level.
pub fn balance_level(mut volumes: impl FnMut(f64) -> CutFill, min: f64, max: f64, factor: f64, tol: f64) -> Result<LevelBalance, Unbalanced> {
    if !(min.is_finite() && max.is_finite() && min <= max) {
        return Err(invalid(format!("bounds {min}..{max}: need finite min ≤ max")));
    }
    if !(factor.is_finite() && factor > 0.0) {
        return Err(invalid(format!("factor {factor} must be > 0")));
    }
    if tol.is_nan() || tol <= 0.0 {
        return Err(invalid(format!("tolerance {tol} must be > 0")));
    }
    let mut evaluations = 0;
    let mut eval = |l: f64| {
        evaluations += 1;
        let v = volumes(l);
        (v, v.cut - factor * v.fill)
    };
    let at = |l: f64, v: CutFill, g: f64, e: usize| LevelBalance { level: l, cut: v.cut, fill: v.fill, net: g, factor, evaluations: e };
    let (va, ga) = eval(min);
    let (vb, gb) = eval(max);
    if !(ga.is_finite() && gb.is_finite()) {
        return Err(invalid("the volumes are not finite".into()));
    }
    if ga == 0.0 {
        return Ok(at(min, va, ga, evaluations));
    }
    if gb == 0.0 {
        return Ok(at(max, vb, gb, evaluations));
    }
    if ga < 0.0 {
        return Err(Unbalanced {
            message: format!("even at the lowest level {min:.3} m the fill needs {:.1} m³ more cut", -ga),
            best: Some(at(min, va, ga, evaluations)),
        });
    }
    if gb > 0.0 {
        return Err(Unbalanced {
            message: format!("even at the highest level {max:.3} m {gb:.1} m³ of cut is left over"),
            best: Some(at(max, vb, gb, evaluations)),
        });
    }
    let level = brent(|l| eval(l).1, min, max, ga, gb, tol);
    let (v, g) = eval(level);
    Ok(at(level, v, g, evaluations))
}

/// Brent's root finder on `[a, b]` with f(a), f(b) of opposite sign.
fn brent(mut f: impl FnMut(f64) -> f64, mut a: f64, mut b: f64, mut fa: f64, mut fb: f64, tol: f64) -> f64 {
    let (mut c, mut fc) = (b, fb);
    let (mut d, mut e) = (b - a, b - a);
    for _ in 0..200 {
        if (fb > 0.0) == (fc > 0.0) {
            (c, fc) = (a, fa);
            d = b - a;
            e = d;
        }
        if fc.abs() < fb.abs() {
            (a, b, c) = (b, c, b);
            (fa, fb, fc) = (fb, fc, fb);
        }
        let tol1 = 2.0 * f64::EPSILON * b.abs() + 0.5 * tol;
        let xm = 0.5 * (c - b);
        if xm.abs() <= tol1 || fb == 0.0 {
            return b;
        }
        if e.abs() >= tol1 && fa.abs() > fb.abs() {
            // Inverse quadratic interpolation, or the secant.
            let s = fb / fa;
            let (mut p, mut q);
            if a == c {
                p = 2.0 * xm * s;
                q = 1.0 - s;
            } else {
                let (qq, r) = (fa / fc, fb / fc);
                p = s * (2.0 * xm * qq * (qq - r) - (b - a) * (r - 1.0));
                q = (qq - 1.0) * (r - 1.0) * (s - 1.0);
            }
            if p > 0.0 {
                q = -q;
            }
            p = p.abs();
            if 2.0 * p < (3.0 * xm * q - (tol1 * q).abs()).min((e * q).abs()) {
                e = d;
                d = p / q;
            } else {
                d = xm;
                e = d;
            }
        } else {
            d = xm;
            e = d;
        }
        (a, fa) = (b, fb);
        b += if d.abs() > tol1 { d } else { tol1.copysign(xm) };
        fb = f(b);
    }
    b
}

/// Side slopes 1:n (horizontal per vertical) from the plateau edge to the
/// ground, for cut (rising) and fill (falling); `reach` (m, 0 = no limit) is
/// how far out they may go.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slopes {
    pub cut: f64,
    pub fill: f64,
    pub reach: f64,
}

/// A plateau on a grid of existing heights: the cells whose centre lies in
/// the outline, and with side slopes the cells around it that they reach.
///
/// The design surface of a cell at horizontal distance d outside the outline
/// is the ground clamped to [level − d/n_fill, level + d/n_cut]: the slopes
/// rise and fall from the edge until they meet the ground. Exact for a
/// convex outline over ground that the slope meets only once; round convex
/// corners the slopes form cones, as built.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plateau {
    /// Cell area (m²).
    pub cell_area: f64,
    /// Ground level and distance to the outline (0 inside) of each cell used.
    pub z: Vec<f64>,
    pub d: Vec<f64>,
    pub slopes: Option<Slopes>,
    /// Cells inside the outline.
    pub inside: usize,
}

impl Plateau {
    pub fn new(grid: &Grid, outline: &[P2], slopes: Option<Slopes>) -> Result<Plateau, String> {
        grid.check()?;
        let mut ring: Vec<P2> = outline.to_vec();
        if ring.len() > 1 && ring[0] == ring[ring.len() - 1] {
            ring.pop();
        }
        if ring.len() < 3 {
            return Err("plateau: the outline needs at least three points".into());
        }
        if let Some(s) = slopes {
            if !(s.cut > 0.0 && s.fill > 0.0 && s.reach >= 0.0) {
                return Err("plateau: slopes 1:n need n > 0 and reach ≥ 0".into());
            }
        }
        let n = grid.nx * grid.ny;
        let mut inside = vec![false; n];
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (i, f) in inside.iter_mut().enumerate() {
            let z = grid.z[i];
            if z.is_finite() && contains(&ring, grid.centre(i)) {
                *f = true;
                lo = lo.min(z);
                hi = hi.max(z);
            }
        }
        let count = inside.iter().filter(|v| **v).count();
        if count == 0 {
            return Err("plateau: no cell with a height inside the outline".into());
        }
        // How far the slopes can reach: the full height range over the
        // flattest slope, or the given reach.
        let reach = match slopes {
            None => 0.0,
            Some(s) => {
                let (zl, zh) = grid.z.iter().filter(|v| v.is_finite()).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
                let r = (zh - zl) * s.cut.max(s.fill);
                if s.reach > 0.0 { r.min(s.reach) } else { r }
            }
        };
        let (mut z, mut d) = (Vec::new(), Vec::new());
        for (i, &inn) in inside.iter().enumerate() {
            let zi = grid.z[i];
            if !zi.is_finite() {
                continue;
            }
            if inn {
                z.push(zi);
                d.push(0.0);
            } else if reach > 0.0 {
                let di = distance(&ring, grid.centre(i));
                if di <= reach {
                    z.push(zi);
                    d.push(di);
                }
            }
        }
        Ok(Plateau { cell_area: grid.area(), z, d, slopes, inside: count })
    }

    /// Cut and fill (m³) at a plateau level.
    pub fn cut_fill(&self, level: f64) -> CutFill {
        let (mut cut, mut fill) = (0.0, 0.0);
        for (&z, &d) in self.z.iter().zip(&self.d) {
            let design = match self.slopes {
                Some(s) if d > 0.0 => z.clamp(level - d / s.fill, level + d / s.cut),
                _ => level,
            };
            if z > design {
                cut += z - design;
            } else {
                fill += design - z;
            }
        }
        CutFill { cut: cut * self.cell_area, fill: fill * self.cell_area }
    }

    /// Lowest and highest ground among the cells used: the balance lies
    /// between them.
    pub fn range(&self) -> (f64, f64) {
        self.z.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)))
    }
}

/// The balanced level of a plateau (to 1 mm).
pub fn balance_plateau(p: &Plateau, factor: f64) -> Result<LevelBalance, Unbalanced> {
    let (lo, hi) = p.range();
    balance_level(|l| p.cut_fill(l), lo, hi, factor, 0.001)
}

/// Point in polygon (crossing number).
fn contains(ring: &[P2], p: P2) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from `p` to the closed ring.
fn distance(ring: &[P2], p: P2) -> f64 {
    let mut best = f64::INFINITY;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let (ux, uy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = ux * ux + uy * uy;
        let t = if l2 > 0.0 { (((p[0] - a[0]) * ux + (p[1] - a[1]) * uy) / l2).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((p[0] - a[0] - t * ux).hypot(p[1] - a[1] - t * uy));
    }
    best
}

// ── Profile ─────────────────────────────────────────────────────────────────

/// A point of vertical intersection: station (m), level (m) and the bounds
/// of its level (min = max fixes it, e.g. a tie-in to an existing road).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pvi {
    pub station: f64,
    pub level: f64,
    pub min: f64,
    pub max: f64,
}

/// Grade limits as fractions (0.04 = 4 %): every grade between PVIs has
/// `min ≤ |grade| ≤ max` (min > 0 for drainage on kerbed roads, 0 = none).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradeLimits {
    pub min: f64,
    pub max: f64,
}

/// Cut and fill (m³) between two stations.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub from: f64,
    pub to: f64,
    pub cut: f64,
    pub fill: f64,
}

/// Search settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Optimise {
    /// In-situ cut per m³ of fill.
    pub factor: f64,
    /// m³ of net balance that weigh as much as 1 m³·km of haul.
    pub haul_weight: f64,
    /// First step (m) and the step at which to stop (m).
    pub step: f64,
    pub tol: f64,
    pub max_evaluations: usize,
}

impl Default for Optimise {
    fn default() -> Self {
        Optimise { factor: 1.0, haul_weight: 0.1, step: 1.0, tol: 0.001, max_evaluations: 20_000 }
    }
}

/// The volumes of one profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileState {
    pub levels: Vec<f64>,
    pub grades: Vec<f64>,
    pub cut: f64,
    pub fill: f64,
    /// cut − factor · fill (m³).
    pub net: f64,
    /// ∫|mass ordinate| ds (m³·km).
    pub haul: f64,
    /// |net| + w · haul.
    pub objective: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileResult {
    pub before: ProfileState,
    pub after: ProfileState,
    pub evaluations: usize,
}

/// Cut, fill, net (cut − factor·fill) and haul (m³·km) of these intervals,
/// in station order: the mass ordinate is linear within an interval (its
/// volume spread evenly), so ∫|M| ds is exact per piece, also where M
/// changes sign.
pub fn mass_haul(intervals: &[Interval], factor: f64) -> (f64, f64, f64, f64) {
    let (mut cut, mut fill, mut m, mut area) = (0.0, 0.0, 0.0, 0.0);
    for iv in intervals {
        cut += iv.cut;
        fill += iv.fill;
        let m1 = m + iv.cut - factor * iv.fill;
        let len = (iv.to - iv.from).abs();
        area += if m * m1 >= 0.0 { 0.5 * (m.abs() + m1.abs()) * len } else { 0.5 * (m * m + m1 * m1) / (m.abs() + m1.abs()) * len };
        m = m1;
    }
    (cut, fill, cut - factor * fill, area / 1000.0)
}

/// Grades between the PVIs.
fn grades_of(st: &[f64], z: &[f64]) -> Vec<f64> {
    st.windows(2).zip(z.windows(2)).map(|(s, z)| (z[1] - z[0]) / (s[1] - s[0])).collect()
}

/// Move `z` onto the level bounds and grade limits (alternating projections);
/// None if they cannot all hold.
fn project(st: &[f64], lo: &[f64], hi: &[f64], lim: &GradeLimits, z: &mut [f64]) -> Option<()> {
    const EPS: f64 = 1e-9;
    let n = z.len();
    for _ in 0..200 {
        for k in 0..n {
            z[k] = z[k].clamp(lo[k], hi[k]);
        }
        let mut ok = true;
        for j in 0..n - 1 {
            let len = st[j + 1] - st[j];
            let g = (z[j + 1] - z[j]) / len;
            let target = if g.abs() > lim.max {
                lim.max.copysign(g)
            } else if g.abs() < lim.min {
                if g >= 0.0 { lim.min } else { -lim.min }
            } else {
                continue;
            };
            if (g - target).abs() <= EPS {
                continue;
            }
            ok = false;
            // Change the rise by `need`, split over the ends that can move.
            let need = (target - g) * len;
            let (fa, fb) = ((hi[j] > lo[j]) as u8 as f64, (hi[j + 1] > lo[j + 1]) as u8 as f64);
            let share = fa + fb;
            if share == 0.0 {
                return None;
            }
            z[j] -= need * fa / share;
            z[j + 1] += need * fb / share;
        }
        if ok && (0..n).all(|k| z[k] >= lo[k] - EPS && z[k] <= hi[k] + EPS) {
            return Some(());
        }
    }
    let feasible = (0..n).all(|k| z[k] >= lo[k] - 1e-6 && z[k] <= hi[k] + 1e-6)
        && grades_of(st, z).iter().all(|g| g.abs() <= lim.max + 1e-6 && g.abs() >= lim.min - 1e-6);
    feasible.then_some(())
}

/// Optimise the PVI levels for |net| + w·haul with `volumes(levels)` giving
/// cut and fill per station interval.
pub fn optimise_profile(pvis: &[Pvi], limits: &GradeLimits, opt: &Optimise, mut volumes: impl FnMut(&[f64]) -> Vec<Interval>) -> Result<ProfileResult, String> {
    let n = pvis.len();
    if n < 2 {
        return Err("profile: at least two PVIs".into());
    }
    if pvis.windows(2).any(|w| w[1].station.partial_cmp(&w[0].station) != Some(std::cmp::Ordering::Greater)) {
        return Err("profile: stations must increase".into());
    }
    if pvis.iter().any(|p| !(p.min <= p.max && p.level.is_finite())) {
        return Err("profile: every PVI needs min ≤ max and a level".into());
    }
    if !(limits.max > 0.0 && limits.min >= 0.0 && limits.min <= limits.max) {
        return Err("profile: grade limits need 0 ≤ min ≤ max and max > 0".into());
    }
    if !(opt.factor > 0.0 && opt.haul_weight >= 0.0 && opt.step > 0.0 && opt.tol > 0.0) {
        return Err("profile: factor, step and tol must be > 0, haul weight ≥ 0".into());
    }
    let st: Vec<f64> = pvis.iter().map(|p| p.station).collect();
    let lo: Vec<f64> = pvis.iter().map(|p| p.min).collect();
    let hi: Vec<f64> = pvis.iter().map(|p| p.max).collect();
    let mut evaluations = 0usize;
    let mut state = |z: &[f64], evaluations: &mut usize| {
        *evaluations += 1;
        let (cut, fill, net, haul) = mass_haul(&volumes(z), opt.factor);
        ProfileState { levels: z.to_vec(), grades: grades_of(&st, z), cut, fill, net, haul, objective: net.abs() + opt.haul_weight * haul }
    };
    let z0: Vec<f64> = pvis.iter().map(|p| p.level).collect();
    let before = state(&z0, &mut evaluations);
    let mut z = z0.clone();
    project(&st, &lo, &hi, limits, &mut z).ok_or("profile: no levels satisfy both the level bounds and the grade limits")?;
    let mut best = state(&z, &mut evaluations);

    // Hat area of each PVI over the total length: the share of the mean
    // level that moving it by 1 m changes.
    let total = st[n - 1] - st[0];
    let hat: Vec<f64> = (0..n)
        .map(|k| {
            let left = if k > 0 { st[k] - st[k - 1] } else { 0.0 };
            let right = if k + 1 < n { st[k + 1] - st[k] } else { 0.0 };
            0.5 * (left + right) / total
        })
        .collect();
    let mut step = opt.step;
    let mut trial = vec![0.0; n];
    while step >= opt.tol && evaluations < opt.max_evaluations {
        let mut improved = false;
        // Moves: (PVI or None for all, compensate).
        let moves = (0..n).flat_map(|k| [(Some(k), false), (Some(k), true)]).chain([(None, false)]);
        for (k, comp) in moves {
            if k.is_some_and(|k| hi[k] <= lo[k]) {
                continue;
            }
            for dir in [1.0, -1.0] {
                let d = dir * step;
                trial.copy_from_slice(&best.levels);
                match k {
                    None => trial.iter_mut().for_each(|v| *v += d),
                    Some(k) => {
                        if comp {
                            let shift = -d * hat[k] / (1.0 - hat[k]).max(1e-9);
                            trial.iter_mut().for_each(|v| *v += shift);
                            trial[k] += d - shift;
                        } else {
                            trial[k] += d;
                        }
                    }
                }
                if project(&st, &lo, &hi, limits, &mut trial).is_none() {
                    continue;
                }
                let s = state(&trial, &mut evaluations);
                if s.objective < best.objective - 1e-12 * (1.0 + best.objective) {
                    best = s;
                    improved = true;
                    break;
                }
            }
            if evaluations >= opt.max_evaluations {
                break;
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    Ok(ProfileResult { before, after: best, evaluations })
}

/// Cut and fill of a template of `width` (m) along a ground profile
/// `(station, level)` under the design line through the PVIs: width ×
/// height difference, integrated exactly between the stations of both
/// (piecewise linear, split where they cross).
pub fn ground_volumes(ground: &[(f64, f64)], stations: &[f64], levels: &[f64], width: f64) -> Vec<Interval> {
    let (s0, s1) = (stations[0], stations[stations.len() - 1]);
    let mut at: Vec<f64> = ground.iter().map(|g| g.0).filter(|s| *s > s0 && *s < s1).chain(stations.iter().copied()).collect();
    at.sort_by(f64::total_cmp);
    at.dedup();
    let design = |s: f64| interp(stations, levels, s);
    let gs: Vec<f64> = ground.iter().map(|g| g.0).collect();
    let gz: Vec<f64> = ground.iter().map(|g| g.1).collect();
    let diff = |s: f64| interp(&gs, &gz, s) - design(s);
    at.windows(2)
        .map(|w| {
            let (a, b) = (diff(w[0]), diff(w[1]));
            let len = w[1] - w[0];
            // ∫max(d, 0) and ∫max(−d, 0) for a linear d.
            let (pos, neg) = if a >= 0.0 && b >= 0.0 {
                (0.5 * (a + b) * len, 0.0)
            } else if a <= 0.0 && b <= 0.0 {
                (0.0, -0.5 * (a + b) * len)
            } else {
                let t = a / (a - b);
                let (p, m) = if a > 0.0 { (a * t, -b * (1.0 - t)) } else { (b * (1.0 - t), -a * t) };
                (0.5 * p * len, 0.5 * m * len)
            };
            Interval { from: w[0], to: w[1], cut: width * pos, fill: width * neg }
        })
        .collect()
}

/// Linear interpolation in sorted `xs`, constant beyond the ends.
fn interp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let k = xs.partition_point(|v| *v < x);
    if k == 0 {
        return ys[0];
    }
    if k >= xs.len() {
        return ys[ys.len() - 1];
    }
    let t = (x - xs[k - 1]) / (xs[k] - xs[k - 1]);
    ys[k - 1] + t * (ys[k] - ys[k - 1])
}

/// [`optimise_profile`] with [`ground_volumes`]: a template of `width` m
/// over a ground profile that covers the PVIs.
pub fn optimise_on_ground(pvis: &[Pvi], limits: &GradeLimits, opt: &Optimise, ground: &[(f64, f64)], width: f64) -> Result<ProfileResult, String> {
    if ground.len() < 2 || ground.windows(2).any(|w| w[1].0.partial_cmp(&w[0].0) != Some(std::cmp::Ordering::Greater)) {
        return Err("ground profile: at least two points with increasing stations".into());
    }
    if pvis.len() < 2 || ground[0].0 > pvis[0].station + 1e-9 || ground[ground.len() - 1].0 < pvis[pvis.len() - 1].station - 1e-9 {
        return Err("ground profile: must cover the PVIs".into());
    }
    if width.is_nan() || width <= 0.0 {
        return Err("template width must be > 0".into());
    }
    let st: Vec<f64> = pvis.iter().map(|p| p.station).collect();
    optimise_profile(pvis, limits, opt, |z| ground_volumes(ground, &st, z, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 40×40 cells of 1 m over [−20, 20]², z = 3 + 0.05·x.
    fn tilted() -> Grid {
        let mut g = Grid { origin: [-20.0, -20.0], cell: 1.0, nx: 40, ny: 40, z: Vec::new() };
        g.z = (0..1600).map(|i| 3.0 + 0.05 * g.centre(i)[0]).collect();
        g
    }

    fn square(h: f64) -> Vec<P2> {
        vec![[-h, -h], [h, -h], [h, h], [-h, h]]
    }

    #[test]
    fn tilted_plane_balances_at_its_mean() {
        let p = Plateau::new(&tilted(), &square(10.0), None).unwrap();
        assert_eq!(p.inside, 400);
        let b = balance_plateau(&p, 1.0).unwrap();
        assert!((b.level - 3.0).abs() < 1e-3, "{b:?}");
        assert!(b.net.abs() < 0.5 && (b.cut - b.fill).abs() < 0.5, "{b:?}");
    }

    #[test]
    fn factor_above_one_lowers_the_level() {
        // z uniform on [3 − H, 3 + H]: cut = (3+H−L)²/4H, fill = (L−3+H)²/4H
        // per unit area, so cut = k·fill at L = 3 + H(1 − √k)/(1 + √k).
        let p = Plateau::new(&tilted(), &square(10.0), None).unwrap();
        let k: f64 = 1.25;
        let b = balance_plateau(&p, k).unwrap();
        let expect = 3.0 + 0.5 * (1.0 - k.sqrt()) / (1.0 + k.sqrt());
        assert!(b.level < 3.0 && (b.level - expect).abs() < 0.005, "{} vs {expect}", b.level);
        assert!((b.cut - k * b.fill).abs() < 0.01 * b.cut, "{b:?}");
    }

    #[test]
    fn side_slopes_keep_the_symmetric_balance() {
        let g = tilted();
        let flat = Plateau::new(&g, &square(10.0), None).unwrap();
        let s = Slopes { cut: 2.0, fill: 2.0, reach: 0.0 };
        let p = Plateau::new(&g, &square(10.0), Some(s)).unwrap();
        assert!(p.z.len() > flat.z.len());
        let b = balance_plateau(&p, 1.0).unwrap();
        assert!((b.level - 3.0).abs() < 1e-3, "{b:?}");
        assert!(p.cut_fill(3.0).cut > flat.cut_fill(3.0).cut);
        // East of the edge the ground is 3.5 + 0.05·d, the cut slope 3 + d/2:
        // they meet at d = 1.11 m, so the ring adds a strip of about
        // ½ · 0.5 m · 1.11 m per metre of edge, plus the corner cones.
        let extra = p.cut_fill(3.0).cut - flat.cut_fill(3.0).cut;
        assert!(extra > 0.0 && extra < 20.0 * 0.5 * 0.5 * 1.2 + 2.0, "{extra}");
    }

    #[test]
    fn no_balance_returns_the_best_bound() {
        let e = balance_level(|l| CutFill { cut: (5.0 - l).max(0.0) * 100.0, fill: 0.0 }, 0.0, 4.0, 1.0, 1e-3).unwrap_err();
        assert_eq!(e.best.as_ref().unwrap().level, 4.0);
        assert!(balance_level(|_| CutFill { cut: 1.0, fill: 1.0 }, 2.0, 1.0, 1.0, 1e-3).is_err());
        let s: String = e.into();
        assert!(s.contains("highest"));
    }

    #[test]
    fn brent_is_quick_on_a_smooth_balance() {
        let b = balance_level(|l| CutFill { cut: (10.0 - l).powi(2), fill: l * l }, 0.0, 10.0, 1.0, 1e-9).unwrap();
        assert!((b.level - 5.0).abs() < 1e-8 && b.evaluations < 20, "{b:?}");
    }

    #[test]
    fn mass_haul_of_a_simple_loop() {
        let iv = [Interval { from: 0.0, to: 100.0, cut: 100.0, fill: 0.0 }, Interval { from: 100.0, to: 200.0, cut: 0.0, fill: 100.0 }];
        let (cut, fill, net, haul) = mass_haul(&iv, 1.0);
        assert_eq!((cut, fill, net), (100.0, 100.0, 0.0));
        assert!((haul - 10.0).abs() < 1e-12);
        // Sign change inside an interval: M from 50 to −50 over 100 m.
        let (_, _, _, h) = mass_haul(&[Interval { from: 0.0, to: 100.0, cut: 50.0, fill: 0.0 }, Interval { from: 100.0, to: 200.0, cut: 0.0, fill: 100.0 }], 1.0);
        assert!((h - (2500.0 + 2500.0) / 1000.0).abs() < 1e-12, "{h}");
    }

    #[test]
    fn ground_volumes_are_exact_for_a_crossing() {
        // Ground 0 → 2 over 0..100, design flat at 1: 25 m² cut and fill.
        let iv = ground_volumes(&[(0.0, 0.0), (100.0, 2.0)], &[0.0, 100.0], &[1.0, 1.0], 2.0);
        let (c, f): (f64, f64) = iv.iter().fold((0.0, 0.0), |a, i| (a.0 + i.cut, a.1 + i.fill));
        assert!((c - 50.0).abs() < 1e-9 && (f - 50.0).abs() < 1e-9, "{c} {f}");
    }

    fn hill_and_valley() -> Vec<(f64, f64)> {
        (0..=80).map(|i| { let s = i as f64 * 5.0; (s, 2.0 * (std::f64::consts::TAU * s / 400.0).sin()) }).collect()
    }

    #[test]
    fn profile_over_hill_and_valley_balances_within_its_grades() {
        let ground = hill_and_valley();
        let pvis: Vec<Pvi> = (0..=4).map(|k| Pvi { station: 100.0 * k as f64, level: 1.5, min: -5.0, max: 5.0 }).collect();
        let lim = GradeLimits { min: 0.0, max: 0.015 };
        let r = optimise_on_ground(&pvis, &lim, &Optimise::default(), &ground, 10.0).unwrap();
        let (b, a) = (&r.before, &r.after);
        assert!(b.net < -1000.0, "{b:?}");
        assert!(a.net.abs() < 0.005 * (b.cut + b.fill), "{a:?}");
        assert!(a.objective < 0.5 * b.objective && a.haul <= b.haul, "{b:?} → {a:?}");
        assert!(a.grades.iter().all(|g| g.abs() <= 0.015 + 1e-9), "{:?}", a.grades);
        // The hill is cut and the valley filled: the line falls overall.
        assert!(a.levels[1] > a.levels[3], "{:?}", a.levels);
    }

    #[test]
    fn fixed_ends_and_a_minimum_grade_hold() {
        let ground = hill_and_valley();
        let mut pvis: Vec<Pvi> = (0..=4).map(|k| Pvi { station: 100.0 * k as f64, level: 0.0, min: -5.0, max: 5.0 }).collect();
        for k in [0, 4] {
            pvis[k] = Pvi { min: 0.0, max: 0.0, ..pvis[k] };
        }
        let lim = GradeLimits { min: 0.003, max: 0.03 };
        let r = optimise_on_ground(&pvis, &lim, &Optimise::default(), &ground, 10.0).unwrap();
        let a = &r.after;
        assert_eq!((a.levels[0], a.levels[4]), (0.0, 0.0));
        assert!(a.grades.iter().all(|g| g.abs() >= 0.003 - 1e-9 && g.abs() <= 0.03 + 1e-9), "{:?}", a.grades);
        assert!(a.objective <= r.before.objective);
        assert!(a.net.abs() < 5.0, "{a:?}");
        // Impossible: 10 m up in 100 m at 3 %.
        let bad = [Pvi { station: 0.0, level: 0.0, min: 0.0, max: 0.0 }, Pvi { station: 100.0, level: 10.0, min: 10.0, max: 10.0 }];
        assert!(optimise_on_ground(&bad, &lim, &Optimise::default(), &ground, 10.0).is_err());
    }
}
