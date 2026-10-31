//! The layout of an IfcAlignment: horizontal segments from an Open Infra
//! axis (lines and arcs), vertical segments from its design profile (grades
//! and parabolic curves), and the transition codes between them. Lengths in
//! metres; the mapping to curve segments follows IFC 4.3 ADD2 as
//! IfcOpenShell evaluates it.

use std::f64::consts::{FRAC_PI_2, TAU};

use crate::civiel::as_profiel::{design_z, Alignment, Pvi, Seg};

/// One horizontal segment (IfcAlignmentHorizontalSegment).
#[derive(Clone, Debug, PartialEq)]
pub struct HSeg {
    pub start: [f64; 2],
    /// Start direction, radians from the x axis (counter-clockwise).
    pub dir: f64,
    /// Signed radius: > 0 turns left (counter-clockwise), 0 for a line.
    pub radius: f64,
    pub length: f64,
}

impl HSeg {
    pub fn is_line(&self) -> bool {
        self.radius == 0.0
    }

    pub fn kind(&self) -> &'static str {
        if self.is_line() { "LINE" } else { "CIRCULARARC" }
    }

    fn curvature(&self) -> f64 {
        if self.is_line() { 0.0 } else { 1.0 / self.radius }
    }

    /// End point and direction.
    pub fn end(&self) -> ([f64; 2], f64) {
        if self.is_line() {
            ([self.start[0] + self.length * self.dir.cos(), self.start[1] + self.length * self.dir.sin()], self.dir)
        } else {
            let r = self.radius;
            let c = [self.start[0] - r * self.dir.sin(), self.start[1] + r * self.dir.cos()];
            let d1 = self.dir + self.length / r;
            ([c[0] + r * d1.sin(), c[1] - r * d1.cos()], d1)
        }
    }
}

/// The horizontal layout of an axis in drawing units, in metres from the
/// drawing origin (`f` drawing units per metre).
pub fn horizontal(al: &Alignment, f: f64) -> Vec<HSeg> {
    al.segs
        .iter()
        .map(|s| match s {
            Seg::Line { a, b } => HSeg {
                start: [a[0] / f, a[1] / f],
                dir: (b[1] - a[1]).atan2(b[0] - a[0]),
                radius: 0.0,
                length: (b[0] - a[0]).hypot(b[1] - a[1]) / f,
            },
            Seg::Arc { c, r, a0, sweep } => {
                let sign = sweep.signum();
                HSeg {
                    start: [(c[0] + r * a0.cos()) / f, (c[1] + r * a0.sin()) / f],
                    dir: a0 + sign * FRAC_PI_2,
                    radius: sign * r / f,
                    length: r * sweep.abs() / f,
                }
            }
        })
        .filter(|s| s.length > 1e-9)
        .collect()
}

/// How segment `a` runs into segment `b` (IfcTransitionCode).
pub fn transition(a: &HSeg, b: &HSeg) -> &'static str {
    let (p, d) = a.end();
    if (p[0] - b.start[0]).hypot(p[1] - b.start[1]) > 1e-4 {
        return "DISCONTINUOUS";
    }
    let turn = (b.dir - d).rem_euclid(TAU);
    if turn.min(TAU - turn) > 1e-5 {
        return "CONTINUOUS";
    }
    if (a.curvature() - b.curvature()).abs() < 1e-9 { "CONTSAMEGRADIENTSAMECURVATURE" } else { "CONTSAMEGRADIENT" }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VKind {
    Gradient,
    Parabola,
}

/// One vertical segment (IfcAlignmentVerticalSegment).
#[derive(Clone, Debug, PartialEq)]
pub struct VSeg {
    pub kind: VKind,
    /// Distance along the horizontal alignment (m).
    pub start: f64,
    pub length: f64,
    pub z: f64,
    pub g0: f64,
    pub g1: f64,
}

impl VSeg {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            VKind::Gradient => "CONSTANTGRADIENT",
            VKind::Parabola => "PARABOLICARC",
        }
    }

    /// Radius of a parabolic curve at its apex: positive in a sag (the
    /// grade increases), negative on a crest.
    pub fn radius(&self) -> Option<f64> {
        (self.kind == VKind::Parabola && (self.g1 - self.g0).abs() > 1e-12).then(|| self.length / (self.g1 - self.g0))
    }

    pub fn end_z(&self) -> f64 {
        self.z + (self.g0 + self.g1) / 2.0 * self.length
    }
}

fn grade(a: &Pvi, b: &Pvi) -> f64 {
    (b.z - a.z) / (b.s - a.s).max(1e-9)
}

/// The vertical layout of a design profile: grades between the curves and a
/// parabolic curve of length `l` at every inner PVI.
pub fn vertical(pvis: &[Pvi]) -> Vec<VSeg> {
    let n = pvis.len();
    if n < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = pvis[0].s;
    for j in 1..n {
        let g = grade(&pvis[j - 1], &pvis[j]);
        let inner = j + 1 < n;
        let half = if inner { pvis[j].l / 2.0 } else { 0.0 };
        let end = pvis[j].s - half;
        if end - cursor > 1e-6 {
            out.push(VSeg { kind: VKind::Gradient, start: cursor, length: end - cursor, z: design_z(pvis, cursor), g0: g, g1: g });
        }
        cursor = end.max(cursor);
        if inner && half > 1e-6 {
            let g1 = grade(&pvis[j], &pvis[j + 1]);
            out.push(VSeg { kind: VKind::Parabola, start: cursor, length: 2.0 * half, z: design_z(pvis, cursor), g0: g, g1 });
            cursor += 2.0 * half;
        }
    }
    out
}

/// How vertical segment `a` runs into `b`.
pub fn vertical_transition(a: &VSeg, b: &VSeg) -> &'static str {
    if (a.start + a.length - b.start).abs() > 1e-4 || (a.end_z() - b.z).abs() > 1e-4 {
        return "DISCONTINUOUS";
    }
    if (a.g1 - b.g0).abs() > 1e-9 {
        return "CONTINUOUS";
    }
    let k = |s: &VSeg| if s.kind == VKind::Parabola { (s.g1 - s.g0) / s.length } else { 0.0 };
    if (k(a) - k(b)).abs() < 1e-12 { "CONTSAMEGRADIENTSAMECURVATURE" } else { "CONTSAMEGRADIENT" }
}

/// Length along a parabola `y = A + Bx + Cx²` from 0 to `l` (closed form).
pub fn parabola_length(b: f64, c: f64, l: f64) -> f64 {
    if c.abs() < 1e-15 {
        return l * (1.0 + b * b).sqrt();
    }
    // ∫ √(1 + (B + 2Cx)²) dx with u = B + 2Cx.
    let f = |u: f64| (u * (1.0 + u * u).sqrt() + (u + (1.0 + u * u).sqrt()).ln()) / 2.0;
    (f(b + 2.0 * c * l) - f(b)) / (2.0 * c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::as_profiel::fit_curves;

    #[test]
    fn line_then_left_arc() {
        let al = Alignment {
            name: "A".into(),
            start: 0.0,
            segs: vec![
                Seg::Line { a: [0.0, 0.0], b: [100.0, 0.0] },
                // Centre above the end of the line: turns left.
                Seg::Arc { c: [100.0, 200.0], r: 200.0, a0: -FRAC_PI_2, sweep: 0.5 },
            ],
        };
        let h = horizontal(&al, 1.0);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].kind(), "LINE");
        assert!((h[1].radius - 200.0).abs() < 1e-9);
        assert!((h[1].start[0] - 100.0).abs() < 1e-9 && h[1].start[1].abs() < 1e-9);
        assert!(h[1].dir.abs() < 1e-12);
        assert_eq!(transition(&h[0], &h[1]), "CONTSAMEGRADIENT");
        let (p, d) = h[1].end();
        assert!((p[0] - (100.0 + 200.0 * 0.5f64.sin())).abs() < 1e-9);
        assert!((p[1] - 200.0 * (1.0 - 0.5f64.cos())).abs() < 1e-9);
        assert!((d - 0.5).abs() < 1e-12);
    }

    #[test]
    fn millimetre_drawings_scale_to_metres() {
        let al = Alignment { name: "A".into(), start: 0.0, segs: vec![Seg::Line { a: [0.0, 0.0], b: [5000.0, 0.0] }] };
        assert!((horizontal(&al, 1000.0)[0].length - 5.0).abs() < 1e-12);
    }

    #[test]
    fn a_kink_is_continuous_only() {
        let a = HSeg { start: [0.0, 0.0], dir: 0.0, radius: 0.0, length: 10.0 };
        let b = HSeg { start: [10.0, 0.0], dir: 0.3, radius: 0.0, length: 10.0 };
        assert_eq!(transition(&a, &b), "CONTINUOUS");
        let c = HSeg { start: [10.0, 0.0], dir: 0.0, radius: 0.0, length: 10.0 };
        assert_eq!(transition(&a, &c), "CONTSAMEGRADIENTSAMECURVATURE");
    }

    #[test]
    fn vertical_layout_follows_the_design_profile() {
        let pvis = fit_curves(&[(0.0, 10.0), (100.0, 12.0), (200.0, 11.0)], 40.0);
        let v = vertical(&pvis);
        assert_eq!(v.iter().map(|s| s.kind).collect::<Vec<_>>(), [VKind::Gradient, VKind::Parabola, VKind::Gradient]);
        assert!((v[1].start - 80.0).abs() < 1e-9 && (v[1].length - 40.0).abs() < 1e-9);
        assert!((v[1].z - 11.6).abs() < 1e-9);
        assert!((v[1].g0 - 0.02).abs() < 1e-12 && (v[1].g1 + 0.01).abs() < 1e-12);
        // Crest: the grade decreases, the radius is negative.
        assert!((v[1].radius().unwrap() + 40.0 / 0.03).abs() < 1e-6);
        for w in v.windows(2) {
            assert_eq!(vertical_transition(&w[0], &w[1]), "CONTSAMEGRADIENT");
            assert!((w[0].end_z() - w[1].z).abs() < 1e-9);
        }
        assert!((v[2].end_z() - 11.0).abs() < 1e-9);
    }

    #[test]
    fn parabola_length_matches_a_numeric_sum() {
        let (b, c, l) = (0.02, -0.000375, 40.0);
        let n = 100_000;
        let mut sum = 0.0;
        for i in 0..n {
            let x = (i as f64 + 0.5) * l / n as f64;
            sum += (1.0 + (b + 2.0 * c * x).powi(2)).sqrt() * l / n as f64;
        }
        assert!((parabola_length(b, c, l) - sum).abs() < 1e-6);
    }
}
