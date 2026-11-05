//! Curve fitting for the design problems (`CIV_PROBLEMEN`): why curves do
//! not fit, and the radius or curve length that does.
//!
//! Horizontal: an axis by layout is its PIs with a radius (and clothoids)
//! at each inner PI. The curve at a PI takes its tangent length T from the
//! straights on both sides ([`as_profiel::tangent_length`]). Two curves
//! overlap when their tangents on the straight between their PIs add up to
//! more than the straight: T₁ + T₂ > L. The fixes: the largest radius at
//! either PI that still fits with the other one kept, or both scaled by one
//! factor until they meet. T grows with R, but with clothoids of a fixed
//! parameter A not everywhere, so every search scans from the asked value
//! down and then bisects.
//!
//! Vertical: a parabolic curve of length L at a PVI runs L/2 either side;
//! it fits when half of it and half of each neighbour's curve lie within
//! the distance to that neighbour ([`as_profiel::longest_curve`]). The
//! rate of curvature K = L / A, with A = |g₂ − g₁| in percent (m per %),
//! and the radius R = L / |g₂ − g₁| = 100·K.

use ocs_plugin_api::host::codec::EntityType;

use super::as_profiel::{tangent_length, Alignment, Seg, Transition};
use crate::gis::crs::XY;

/// An axis by layout: PIs (drawing units), and per PI the radius (drawing
/// units, 0 at the ends and at a corner) and the transition curves.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pis: Vec<XY>,
    pub radii: Vec<f64>,
    pub transitions: Vec<Transition>,
}

/// The straight between two PIs on which the curves at its ends do not fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Misfit {
    /// The straight from PI `leg` to PI `leg + 1`.
    pub leg: usize,
    /// Its length and the tangent lengths its curves take (drawing units).
    pub length: f64,
    pub used: f64,
    /// The inner PIs at its ends that carry a curve.
    pub curves: Vec<usize>,
}

/// The largest `x` in [`lo`, `hi`] for which `ok(x)`: `hi` itself when it
/// fits, else scanned down in 400 steps and bisected to the boundary.
pub fn largest_fit(lo: f64, hi: f64, ok: impl Fn(f64) -> bool) -> Option<f64> {
    if ok(hi) {
        return Some(hi);
    }
    let n = 400;
    let mut above = hi;
    for k in (0..n).rev() {
        let x = lo + (hi - lo) * k as f64 / n as f64;
        if ok(x) {
            let (mut a, mut b) = (x, above);
            for _ in 0..100 {
                let m = (a + b) / 2.0;
                if ok(m) {
                    a = m;
                } else {
                    b = m;
                }
            }
            return Some(a);
        }
        above = x;
    }
    None
}

impl Layout {
    /// One radius and transition per PI from per-curve lists, as
    /// [`Alignment::from_layout`] reads them (the last value repeats).
    pub fn new(pis: Vec<XY>, radii: &[f64], transitions: &[Transition]) -> Layout {
        let n = pis.len();
        let mut r = vec![0.0; n];
        let mut t = vec![Transition::None; n];
        for i in 1..n.saturating_sub(1) {
            r[i] = radii.get(i - 1).or(radii.last()).copied().unwrap_or(0.0).max(0.0);
            t[i] = transitions.get(i - 1).or(transitions.last()).copied().unwrap_or(Transition::None);
        }
        Layout { pis, radii: r, transitions: t }
    }

    pub fn len(&self) -> usize {
        self.pis.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pis.is_empty()
    }

    /// The deflection angle at PI `i` (radians, absolute; 0 at the ends).
    pub fn deflection(&self, i: usize) -> f64 {
        if i == 0 || i + 1 >= self.pis.len() {
            return 0.0;
        }
        let (a, p, b) = (self.pis[i - 1], self.pis[i], self.pis[i + 1]);
        let d_in = (p[1] - a[1]).atan2(p[0] - a[0]);
        let d_out = (b[1] - p[1]).atan2(b[0] - p[0]);
        ((d_out - d_in + std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI) - std::f64::consts::PI).abs()
    }

    /// The straight from PI `j` to PI `j + 1`.
    pub fn leg(&self, j: usize) -> f64 {
        let (a, b) = (self.pis[j], self.pis[j + 1]);
        (b[0] - a[0]).hypot(b[1] - a[1])
    }

    /// The tangent length at PI `i` with radius `r` (its own transition).
    pub fn tangent(&self, i: usize, r: f64) -> Option<f64> {
        if i == 0 || i + 1 >= self.pis.len() {
            return Some(0.0);
        }
        tangent_length(r, self.deflection(i), self.transitions[i])
    }

    /// The axis with these radii (one per PI); an error when they do not fit.
    pub fn alignment(&self, radii: &[f64]) -> Result<Alignment, String> {
        let n = self.pis.len();
        let inner: Vec<f64> = (1..n.saturating_sub(1)).map(|i| radii.get(i).copied().unwrap_or(0.0)).collect();
        let tr: Vec<Transition> = (1..n.saturating_sub(1)).map(|i| self.transitions[i]).collect();
        Alignment::from_layout(&self.pis, &inner, &tr)
    }

    /// The straights on which the curves with `radii` do not fit.
    pub fn misfits(&self, radii: &[f64]) -> Vec<Misfit> {
        let n = self.pis.len();
        let mut out = Vec::new();
        for j in 0..n.saturating_sub(1) {
            let ends = [j, j + 1];
            let used: f64 = ends.iter().map(|&i| self.tangent(i, radii[i]).unwrap_or(0.0)).sum();
            let length = self.leg(j);
            if used > length * (1.0 + 1e-9) + 1e-6 {
                let curves = ends.iter().copied().filter(|&i| i > 0 && i + 1 < n && radii[i] > 0.0 && self.deflection(i) > 1e-9).collect();
                out.push(Misfit { leg: j, length, used, curves });
            }
        }
        out
    }

    /// The largest radius at PI `i`, at most `radii[i]`, whose curve fits
    /// on straight `leg` next to the curve at its other end (with `radii`).
    /// `None` when no radius fits (clothoids longer than the room).
    pub fn largest_radius_on(&self, i: usize, leg: usize, radii: &[f64]) -> Option<f64> {
        let other = if leg == i { leg + 1 } else { leg };
        let room = self.leg(leg) - self.tangent(other, radii[other]).unwrap_or(0.0);
        let hi = radii[i];
        if hi <= 0.0 || room <= 0.0 {
            return None;
        }
        largest_fit(hi * 1e-4, hi, |r| self.tangent(i, r).is_some_and(|t| t <= room))
    }

    /// The largest factor (≤ 1) by which both curves of straight `leg`
    /// shrink until they fit on it.
    pub fn both_scaled(&self, leg: usize, radii: &[f64]) -> Option<f64> {
        let (a, b) = (leg, leg + 1);
        let room = self.leg(leg);
        largest_fit(1e-4, 1.0, |k| match (self.tangent(a, radii[a] * k), self.tangent(b, radii[b] * k)) {
            (Some(ta), Some(tb)) => ta + tb <= room,
            _ => false,
        })
    }

    /// The room for the curve at PI `i` on both its straights, next to the
    /// neighbouring curves with `radii`: (smallest room, on which straight).
    pub fn room(&self, i: usize, radii: &[f64]) -> (f64, usize) {
        let left = self.leg(i - 1) - self.tangent(i - 1, radii[i - 1]).unwrap_or(0.0);
        let right = self.leg(i) - self.tangent(i + 1, radii[i + 1]).unwrap_or(0.0);
        if left <= right { (left, i - 1) } else { (right, i) }
    }

    /// Whether radius `r` at PI `i` fits on both its straights next to the
    /// neighbouring curves; else the tangent it needs and the room there is.
    pub fn fits(&self, i: usize, r: f64, radii: &[f64]) -> Result<(), (f64, f64)> {
        let (room, _) = self.room(i, radii);
        match self.tangent(i, r) {
            Some(t) if t <= room + 1e-9 => Ok(()),
            Some(t) => Err((t, room)),
            None => Err((f64::INFINITY, room)),
        }
    }

    /// The radii drawn when the asked ones do not fit: on every straight
    /// that is too short both curves shrink by one factor until they meet;
    /// a curve between two such straights takes the smaller factor.
    pub fn fitted(&self) -> Vec<f64> {
        let mut k = vec![1.0f64; self.pis.len()];
        for m in self.misfits(&self.radii) {
            let factor = if m.curves.len() == 2 {
                self.both_scaled(m.leg, &self.radii)
            } else if let Some(&i) = m.curves.first() {
                self.largest_radius_on(i, m.leg, &self.radii).map(|r| r / self.radii[i])
            } else {
                None
            };
            for &i in &m.curves {
                // A margin of a millionth keeps the rebuilt curves inside the straight.
                k[i] = k[i].min(factor.unwrap_or(0.0) * (1.0 - 1e-9));
            }
        }
        self.radii.iter().zip(&k).map(|(r, k)| r * k).collect()
    }

    /// The PI whose curve an arc of the axis belongs to: the inner PI with a
    /// radius nearest to the middle of the arc.
    pub fn pi_of(&self, seg: &Seg) -> Option<usize> {
        let (m, _) = seg.at(seg.length() / 2.0);
        (1..self.pis.len().saturating_sub(1))
            .filter(|&i| self.radii[i] > 0.0)
            .min_by(|&a, &b| {
                let da = (self.pis[a][0] - m[0]).hypot(self.pis[a][1] - m[1]);
                let db = (self.pis[b][0] - m[0]).hypot(self.pis[b][1] - m[1]);
                da.total_cmp(&db)
            })
    }

    /// The layout as object data of the axis: `pi<n>=x,y[,R[,L<len>|A<a>]]`
    /// (drawing coordinates; radius and transition in metres, `f` drawing
    /// units per metre).
    pub fn to_data(&self, f: f64) -> Vec<(String, String)> {
        let n = self.pis.len();
        (0..n)
            .map(|i| {
                let p = self.pis[i];
                let mut v = format!("{:.4},{:.4}", p[0], p[1]);
                if i > 0 && i + 1 < n {
                    v.push_str(&format!(",{}", num(self.radii[i] / f)));
                    match self.transitions[i] {
                        Transition::None => {}
                        Transition::Length(l) => v.push_str(&format!(",L{}", num(l / f))),
                        Transition::Parameter(a) => v.push_str(&format!(",A{}", num(a / f))),
                    }
                }
                (format!("{PI_KEY}{i}"), v)
            })
            .collect()
    }

    /// The layout kept on an axis ([`Layout::to_data`]), when it still
    /// describes the axis drawn (not moved or edited since).
    pub fn from_entity(e: &EntityType, f: f64) -> Option<Layout> {
        let al = Alignment::from_entity(e)?;
        let lay = Layout::from_data(&|k| super::data_value(e, k), f)?;
        lay.matches(&al).then_some(lay)
    }

    pub fn from_data(get: &dyn Fn(&str) -> Option<String>, f: f64) -> Option<Layout> {
        let mut pis = Vec::new();
        let mut radii = Vec::new();
        let mut transitions = Vec::new();
        while let Some(v) = get(&format!("{PI_KEY}{}", pis.len())) {
            let parts: Vec<&str> = v.split(',').map(str::trim).collect();
            let x: f64 = parts.first()?.parse().ok()?;
            let y: f64 = parts.get(1)?.parse().ok()?;
            pis.push([x, y]);
            radii.push(parts.get(2).and_then(|r| r.parse::<f64>().ok()).unwrap_or(0.0) * f);
            transitions.push(match parts.get(3) {
                Some(t) if t.starts_with('L') => Transition::Length(t[1..].parse::<f64>().ok()? * f),
                Some(t) if t.starts_with('A') => Transition::Parameter(t[1..].parse::<f64>().ok()? * f),
                _ => Transition::None,
            });
        }
        (pis.len() >= 2).then_some(Layout { pis, radii, transitions })
    }

    /// Whether the layout, built as it would be drawn, is the axis `al`.
    pub fn matches(&self, al: &Alignment) -> bool {
        let Ok(built) = self.alignment(&self.fitted()) else { return false };
        let tol = 1e-4 * al.length().max(1.0);
        let same = |a: XY, b: XY| (a[0] - b[0]).hypot(a[1] - b[1]) <= tol;
        (built.length() - al.length()).abs() <= tol && (0..=8).all(|k| same(built.at(built.length() * k as f64 / 8.0).0, al.at(al.length() * k as f64 / 8.0).0))
    }

    /// The layout of an axis drawn from straights and curves: the PIs where
    /// the straights around each curve meet. `None` for an axis that starts
    /// or ends in a curve, or has curves without a straight between them.
    pub fn from_alignment(al: &Alignment) -> Option<Layout> {
        let segs = &al.segs;
        let mut pis = vec![segs.first()?.at(0.0).0];
        let mut radii = vec![0.0];
        let mut transitions = vec![Transition::None];
        let mut last_line: Option<(XY, XY)> = None;
        let mut group: Vec<&Seg> = Vec::new();
        for g in segs {
            match g {
                Seg::Line { a, b } => {
                    match (last_line, group.is_empty()) {
                        // Two straights meet in a corner.
                        (Some((_, pb)), true) => {
                            pis.push(pb);
                            radii.push(0.0);
                            transitions.push(Transition::None);
                        }
                        (Some((la, lb)), false) => {
                            let arcs: Vec<f64> = group.iter().filter_map(|s| if let Seg::Arc { r, .. } = s { Some(*r) } else { None }).collect();
                            let spirals: Vec<f64> = group.iter().filter_map(|s| if let Seg::Spiral { len, .. } = s { Some(*len) } else { None }).collect();
                            if arcs.len() > 1 || (arcs.is_empty() && spirals.len() != 2) {
                                return None;
                            }
                            let r = match arcs.first() {
                                Some(r) => *r,
                                None => 1.0 / group.iter().map(|s| s.curvature(0.0).abs().max(s.curvature(s.length()).abs())).fold(0.0, f64::max).max(1e-300),
                            };
                            pis.push(intersect(la, lb, *a, *b)?);
                            radii.push(r);
                            transitions.push(spirals.first().map(|l| Transition::Length(*l)).unwrap_or(Transition::None));
                            group.clear();
                        }
                        (None, false) => return None,
                        (None, true) => {}
                    }
                    last_line = Some((*a, *b));
                }
                other => group.push(other),
            }
        }
        if !group.is_empty() {
            return None;
        }
        let end = segs.last()?;
        pis.push(end.at(end.length()).0);
        radii.push(0.0);
        transitions.push(Transition::None);
        Some(Layout { pis, radii, transitions })
    }
}

/// The object data key of a layout's PIs.
pub const PI_KEY: &str = "pi";

fn num(v: f64) -> String {
    let t = format!("{v:.4}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn intersect(a: XY, b: XY, c: XY, d: XY) -> Option<XY> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
    Some([a[0] + r[0] * t, a[1] + r[1] * t])
}

/// K = L / A: metres of curve per percent of grade change.
pub fn k_value(l: f64, g1: f64, g2: f64) -> f64 {
    let a = (g2 - g1).abs() * 100.0;
    if a < 1e-12 { f64::INFINITY } else { l / a }
}

/// The curve length that gives radius `r` for a grade change g₁ → g₂.
pub fn length_for_radius(r: f64, g1: f64, g2: f64) -> f64 {
    r * (g2 - g1).abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::as_profiel::{fit_curves, fit_curves_with, longest_curve};

    /// A U: north 200 m to PI 1, east 100 m to PI 2, south 200 m. Both
    /// corners turn 90°, so T = R·tan 45° = R.
    fn u(r1: f64, r2: f64) -> Layout {
        Layout::new(vec![[0.0, -200.0], [0.0, 0.0], [100.0, 0.0], [100.0, -200.0]], &[r1, r2], &[])
    }

    #[test]
    fn overlapping_curves_and_the_radii_that_fit() {
        let lay = u(80.0, 60.0);
        assert!((lay.deflection(1) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((lay.tangent(1, 80.0).unwrap() - 80.0).abs() < 1e-9);
        // T₁ + T₂ = 80 + 60 = 140 m on a straight of 100 m.
        let m = lay.misfits(&lay.radii);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].leg, m[0].curves.clone()), (1, vec![1, 2]));
        assert!((m[0].used - 140.0).abs() < 1e-9 && (m[0].length - 100.0).abs() < 1e-9);
        // Keep R 60 at PI 2: 100 − 60 = 40 m for PI 1, so R = 40.
        assert!((lay.largest_radius_on(1, 1, &lay.radii).unwrap() - 40.0).abs() < 1e-9);
        // Keep R 80 at PI 1: 20 m left, R = 20.
        assert!((lay.largest_radius_on(2, 1, &lay.radii).unwrap() - 20.0).abs() < 1e-9);
        // Both scaled: 140·k = 100, k = 5/7, R 57,14 and 42,86.
        let k = lay.both_scaled(1, &lay.radii).unwrap();
        assert!((k - 5.0 / 7.0).abs() < 1e-9, "{k}");
        let drawn = lay.fitted();
        assert!((drawn[1] - 400.0 / 7.0).abs() < 1e-6 && (drawn[2] - 300.0 / 7.0).abs() < 1e-6, "{drawn:?}");
        // The drawn axis is valid and its curves meet without a straight.
        let al = lay.alignment(&drawn).unwrap();
        assert!(lay.misfits(&drawn).is_empty());
        assert_eq!(al.segs.iter().filter(|s| matches!(s, Seg::Arc { .. })).count(), 2);
        assert!(lay.matches(&al));
        // As asked it is refused.
        assert!(lay.alignment(&lay.radii).is_err());
    }

    #[test]
    fn a_curve_too_large_for_its_end_straight() {
        // North 50 m, then east: R 80 needs T 80 on a straight of 50.
        let lay = Layout::new(vec![[0.0, -50.0], [0.0, 0.0], [300.0, 0.0]], &[80.0], &[]);
        let m = lay.misfits(&lay.radii);
        assert_eq!((m.len(), m[0].leg, m[0].curves.clone()), (1, 0, vec![1]));
        assert!((lay.largest_radius_on(1, 0, &lay.radii).unwrap() - 50.0).abs() < 1e-9);
        assert!((lay.fitted()[1] - 50.0).abs() < 1e-6);
    }

    #[test]
    fn the_minimum_radius_fits_or_says_why_not() {
        // R 30 at PI 1 next to R 60 at PI 2: 40 m of room, so R 50 (T 50) does not fit.
        let lay = u(30.0, 60.0);
        assert!(lay.misfits(&lay.radii).is_empty());
        let (room, leg) = lay.room(1, &lay.radii);
        assert!((room - 40.0).abs() < 1e-9 && leg == 1);
        let (need, room) = lay.fits(1, 50.0, &lay.radii).unwrap_err();
        assert!((need - 50.0).abs() < 1e-9 && (room - 40.0).abs() < 1e-9);
        // Next to R 40 there are 60 m: R 50 fits.
        let lay = u(30.0, 40.0);
        assert!(lay.fits(1, 50.0, &lay.radii).is_ok());
        // 60° at PI 1 (north to 30°), 30° at PI 2 (to east); T₂ = 60 m with
        // R₂ = 60/tan 15°. 40 m of room give R₁ = 40/tan 30° = 69,28.
        let c = 60f64.to_radians();
        let lay = Layout::new(vec![[0.0, -300.0], [0.0, 0.0], [100.0 * c.sin(), 100.0 * c.cos()], [100.0 * c.sin() + 300.0, 100.0 * c.cos()]], &[90.0, 60.0 / 15f64.to_radians().tan()], &[]);
        assert!((lay.deflection(1) - c).abs() < 1e-12 && (lay.deflection(2) - c / 2.0).abs() < 1e-12);
        assert!((lay.tangent(2, lay.radii[2]).unwrap() - 60.0).abs() < 1e-9);
        let r = lay.largest_radius_on(1, 1, &lay.radii).unwrap();
        assert!((r - 40.0 / 30f64.to_radians().tan()).abs() < 1e-7, "{r}");
    }

    #[test]
    fn tangents_with_clothoids_match_the_built_axis() {
        // East 300, then 60° left; R 200 with clothoids of 60 m.
        let c = std::f64::consts::PI / 3.0;
        let pis = vec![[0.0, 0.0], [300.0, 0.0], [300.0 + 300.0 * c.cos(), 300.0 * c.sin()]];
        let lay = Layout::new(pis, &[200.0], &[Transition::Length(60.0)]);
        let al = lay.alignment(&lay.radii).unwrap();
        // The first straight ends T before the PI.
        let t = lay.tangent(1, 200.0).unwrap();
        assert!((al.segs[0].length() - (300.0 - t)).abs() < 1e-9, "{t}");
        // A clothoid parameter A keeps A when the radius shrinks: L = A²/R.
        let lay_a = Layout { transitions: vec![Transition::None, Transition::Parameter(120.0), Transition::None], ..lay.clone() };
        let r = lay_a.largest_radius_on(1, 0, &[0.0, 600.0, 0.0]).unwrap();
        assert!(lay_a.tangent(1, r).unwrap() <= 300.0 + 1e-9 && lay_a.tangent(1, r * 1.0001).unwrap() > 300.0);
    }

    #[test]
    fn layout_as_object_data_and_from_geometry() {
        let lay = Layout::new(vec![[0.0, -200.0], [0.0, 0.0], [100.0, 0.0], [100.0, -200.0]], &[40.0, 30.0], &[Transition::None, Transition::Length(10.0)]);
        let data = lay.to_data(1.0);
        assert_eq!(data[1], ("pi1".to_string(), "0.0000,0.0000,40".to_string()));
        assert_eq!(data[2].1, "100.0000,0.0000,30,L10");
        let back = Layout::from_data(&|k| data.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()), 1.0).unwrap();
        assert_eq!(back, lay);
        // From the drawn geometry: the same PIs and radii.
        let plain = Layout::new(lay.pis.clone(), &[40.0, 30.0], &[]);
        let al = plain.alignment(&plain.radii).unwrap();
        let rebuilt = Layout::from_alignment(&al).unwrap();
        for (a, b) in rebuilt.pis.iter().zip(&plain.pis) {
            assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9, "{a:?} {b:?}");
        }
        assert!((rebuilt.radii[1] - 40.0).abs() < 1e-9 && (rebuilt.radii[2] - 30.0).abs() < 1e-9);
        let arcs: Vec<&Seg> = al.segs.iter().filter(|s| matches!(s, Seg::Arc { .. })).collect();
        assert_eq!((plain.pi_of(arcs[0]), plain.pi_of(arcs[1])), (Some(1), Some(2)));
    }

    #[test]
    fn vertical_curves_that_fit() {
        // PVIs 0, 40, 300, 400 m; BOOG 120. The rule of fit_curves halves
        // the first curve: L₁ = min(120, 40, 260) = 40, L₂ = min(120, 260, 100) = 100.
        let pts = [(0.0, 10.0), (40.0, 11.0), (300.0, 14.0), (400.0, 12.0)];
        let pvis = fit_curves(&pts, 120.0);
        assert_eq!((pvis[1].l, pvis[2].l), (40.0, 100.0));
        assert_eq!(fit_curves_with(&pts, 120.0, &[]), pvis);
        // Longest at PVI 1: 2·(40 − 0) = 80 to the start, 2·(260 − 100/2) = 420 to PVI 2.
        assert!((longest_curve(&pvis, &[], 1) - 80.0).abs() < 1e-9);
        // At PVI 2: 2·(260 − 40/2) = 480, 2·(100 − 0) = 200.
        assert!((longest_curve(&pvis, &[], 2) - 200.0).abs() < 1e-9);
        // K and R at PVI 1 with L = 80: g 2,5 % → 1,1538 %, A = 1,3462 %.
        let (g1, g2) = (1.0 / 40.0, 3.0 / 260.0);
        assert!((k_value(80.0, g1, g2) - 80.0 / 1.346_153_846_153_846).abs() < 1e-9);
        assert!((80.0 / (g2 - g1).abs() - 100.0 * k_value(80.0, g1, g2)).abs() < 1e-9);
        // An own length of 80 at PVI 1 is kept; PVI 2 still follows the rule.
        let own = [None, Some(80.0), None, None];
        let p = fit_curves_with(&pts, 120.0, &own);
        assert_eq!((p[1].l, p[2].l), (80.0, 100.0));
        // And 120 of its own at PVI 2 fits as well (60 + 40 ≤ 260, 60 ≤ 100).
        let own = [None, Some(80.0), Some(120.0), None];
        let p = fit_curves_with(&pts, 120.0, &own);
        assert_eq!((p[1].l, p[2].l), (80.0, 120.0));
        // Too long an own length is cut to what fits: 300 at PVI 2 → 200.
        let p = fit_curves_with(&pts, 120.0, &[None, Some(80.0), Some(300.0), None]);
        assert_eq!(p[2].l, 200.0);
        // The crest radius from a length and back.
        assert!((length_for_radius(2500.0, 0.02, -0.01) - 75.0).abs() < 1e-9);
    }

    #[test]
    fn largest_fit_scans_then_bisects() {
        // ok below 3: the boundary to 1e-9.
        let x = largest_fit(0.0, 10.0, |x| x <= 3.0).unwrap();
        assert!((x - 3.0).abs() < 1e-9);
        assert_eq!(largest_fit(0.0, 10.0, |x| x <= 20.0), Some(10.0));
        assert_eq!(largest_fit(1.0, 10.0, |_| false), None);
        // A dip: ok only in [2, 4]; the largest is 4.
        let x = largest_fit(0.0, 10.0, |x| (2.0..=4.0).contains(&x)).unwrap();
        assert!((x - 4.0).abs() < 1e-9);
    }
}
