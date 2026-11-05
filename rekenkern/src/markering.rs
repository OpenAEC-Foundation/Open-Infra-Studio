//! Road marking geometry along an axis, issue #70: longitudinal lines
//! (continuous, dashed, double), shark teeth, arrows and zebra crossings as
//! closed polygons with a kind label, ready to be hatched on a layer.
//!
//! Lines follow a polyline (the caller approximates arcs by chords). A dash
//! pattern is laid out along the *arc length* of the offset line, so a
//! stripe that straddles a vertex bends around it (mitred) and the pattern
//! stays continuous through corners; `phase` shifts the pattern start.
//!
//! Dutch line types are named `stripe-gap` in metres with a width, as in
//! Dutch drawing legends: e.g. "onderbroken streep 9-3 0.15 m breed,
//! 1-3 0.45 m breed, 1-1 0.10/0.15 m breed, 3-3 0.15 m breed, 1-2 0.10 m
//! breed" (legend of exb-2020-32786, officielebekendmakingen.nl) and
//! "3-9 streep, 15cm breed; 1-3 streep, 15/30/45cm breed; 0.30-2.70 streep;
//! 0.50-0.50 streep" (legend of exb-2019-6447). The governing guideline is
//! CROW, *Richtlijn Markeringen en bebakening*; the general line widths
//! "0,15 m outside / 0,10 m inside built-up areas" are quoted from Beleidsnota
//! Wegenbeheer module A12 (CVDR93542). Which type goes where (asstreep,
//! deelstreep, waarschuwingsstreep, blokmarkering) is the designer's choice
//! per CROW; this module only draws them. Other countries define their own
//! [`RegionProfile`].
//!
//! Shark teeth (*haaientanden*): legends list "haaientanden 0.30-0.30,
//! lengte 0.30m; 0.50-0.50, lengte 0.50m; 0.50-0.70, lengte 0.70m"
//! (exb-2019-6447), read here as base–height with the gap equal to the base
//! (CROW 207 detail "a, b en c = 0,5 m" as cited in municipal specifications).
//! That reading is an adjustable default.
//!
//! Zebra crossing: "witte strepen met een breedte van 0,4 tot 0,6 m en
//! vergelijkbare tussenliggende afstand", "breedte van ten minste 4 m"
//! (BABW requirement as quoted in Gemeente Leudal, *Beleidskader zichtbare
//! zebra's*, gmb-2026-156730). Default 0.50 / 0.50 / 4.0 m.
//!
//! Arrow dimensions are **not** verified against a guideline: the defaults in
//! [`ArrowStyle::default`] are adjustable placeholders.

use crate::P2;
use serde::{Deserialize, Serialize};

/// Layer label of a marking polygon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkingKind {
    Continuous,
    Dashed,
    SharkTooth,
    Arrow,
    Zebra,
}

/// A closed ring (first point not repeated) with its kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marking {
    pub kind: MarkingKind,
    pub ring: Vec<P2>,
}

/// A longitudinal line type. `gap == 0` means continuous.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineType {
    pub name: String,
    /// Line width, m.
    pub width: f64,
    /// Stripe length, m.
    pub stripe: f64,
    /// Gap length, m.
    pub gap: f64,
}

impl LineType {
    pub fn new(name: &str, width: f64, stripe: f64, gap: f64) -> Self {
        Self {
            name: name.into(),
            width,
            stripe,
            gap,
        }
    }
    pub fn is_continuous(&self) -> bool {
        self.gap <= 0.0
    }
}

/// Shark teeth: isosceles triangles along a give-way line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SharkTeeth {
    pub name: String,
    /// Base along the line, m.
    pub base: f64,
    /// Height (base to apex), m.
    pub height: f64,
    /// Clear gap between bases, m.
    pub gap: f64,
}

/// Zebra crossing stripes (parallel to the road axis).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zebra {
    /// Stripe width across the road, m.
    pub stripe_width: f64,
    /// Gap between stripes, m.
    pub gap: f64,
    /// Stripe length along the road (= width of the crossing), m.
    pub length: f64,
}

/// Arrow outline dimensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArrowStyle {
    /// Overall length along the lane, m.
    pub length: f64,
    pub shaft_width: f64,
    pub head_length: f64,
    pub head_width: f64,
}

impl Default for ArrowStyle {
    /// Adjustable placeholder (not verified against CROW).
    fn default() -> Self {
        Self {
            length: 5.0,
            shaft_width: 0.15,
            head_length: 1.5,
            head_width: 0.6,
        }
    }
}

/// Arrow shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArrowKind {
    Straight,
    Left,
    Right,
}

/// A country's or road authority's marking dimensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionProfile {
    pub name: String,
    pub line_types: Vec<LineType>,
    pub shark_teeth: Vec<SharkTeeth>,
    pub zebra: Zebra,
    pub arrow: ArrowStyle,
}

impl RegionProfile {
    pub fn line(&self, name: &str) -> Option<&LineType> {
        self.line_types.iter().find(|l| l.name == name)
    }
    pub fn teeth(&self, name: &str) -> Option<&SharkTeeth> {
        self.shark_teeth.iter().find(|t| t.name == name)
    }
}

/// Dutch profile; sources in the module docs. Names are `stripe-gap (width)`.
pub fn profile_nl() -> RegionProfile {
    let l = LineType::new;
    RegionProfile {
        name: "NL".into(),
        line_types: vec![
            l("doorgetrokken (0.10)", 0.10, 0.0, 0.0),
            l("doorgetrokken (0.15)", 0.15, 0.0, 0.0),
            l("doorgetrokken (0.30)", 0.30, 0.0, 0.0),
            l("3-9 (0.10)", 0.10, 3.0, 9.0),
            l("3-9 (0.15)", 0.15, 3.0, 9.0),
            l("9-3 (0.10)", 0.10, 9.0, 3.0),
            l("9-3 (0.15)", 0.15, 9.0, 3.0),
            l("3-3 (0.15)", 0.15, 3.0, 3.0),
            l("1-1 (0.10)", 0.10, 1.0, 1.0),
            l("1-1 (0.15)", 0.15, 1.0, 1.0),
            l("1-2 (0.10)", 0.10, 1.0, 2.0),
            l("1-3 (0.15)", 0.15, 1.0, 3.0),
            l("1-3 (0.30)", 0.30, 1.0, 3.0),
            l("1-3 (0.45)", 0.45, 1.0, 3.0),
            l("0.30-2.70 (0.10)", 0.10, 0.30, 2.70),
            l("0.50-0.50 (0.10)", 0.10, 0.50, 0.50),
            l("0.50-0.50 (0.50)", 0.50, 0.50, 0.50),
        ],
        shark_teeth: vec![
            SharkTeeth {
                name: "0.30-0.30".into(),
                base: 0.30,
                height: 0.30,
                gap: 0.30,
            },
            SharkTeeth {
                name: "0.50-0.50".into(),
                base: 0.50,
                height: 0.50,
                gap: 0.50,
            },
            SharkTeeth {
                name: "0.50-0.70".into(),
                base: 0.50,
                height: 0.70,
                gap: 0.50,
            },
        ],
        zebra: Zebra {
            stripe_width: 0.50,
            gap: 0.50,
            length: 4.0,
        },
        arrow: ArrowStyle::default(),
    }
}

// ---------------------------------------------------------------- geometry

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P2, k: f64) -> P2 {
    [a[0] * k, a[1] * k]
}
fn len(a: P2) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: P2) -> P2 {
    let l = len(a);
    if l > 0.0 {
        mul(a, 1.0 / l)
    } else {
        [0.0, 0.0]
    }
}
/// Left normal of a direction.
fn left(d: P2) -> P2 {
    [-d[1], d[0]]
}

/// Drop consecutive duplicate points.
fn clean(line: &[P2]) -> Vec<P2> {
    let mut out: Vec<P2> = Vec::with_capacity(line.len());
    for &p in line {
        if out.last().is_none_or(|q| len(sub(p, *q)) > 1e-9) {
            out.push(p);
        }
    }
    out
}

/// Polyline length.
pub fn length(line: &[P2]) -> f64 {
    line.windows(2).map(|w| len(sub(w[1], w[0]))).sum()
}

/// Signed area of a ring (positive counter-clockwise).
pub fn ring_area(ring: &[P2]) -> f64 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0
}

/// Offset a polyline sideways by `d` (positive = left of the direction of
/// travel) with mitred joins; the mitre is capped at 4·|d| for very sharp
/// angles.
pub fn offset_polyline(line: &[P2], d: f64) -> Vec<P2> {
    let line = clean(line);
    let n = line.len();
    if n < 2 || d == 0.0 {
        return line;
    }
    let dirs: Vec<P2> = line.windows(2).map(|w| unit(sub(w[1], w[0]))).collect();
    (0..n)
        .map(|i| {
            if i == 0 {
                add(line[0], mul(left(dirs[0]), d))
            } else if i == n - 1 {
                add(line[i], mul(left(dirs[i - 1]), d))
            } else {
                let (n1, n2) = (left(dirs[i - 1]), left(dirs[i]));
                let m = unit(add(n1, n2));
                let c = m[0] * n1[0] + m[1] * n1[1];
                let k = if c > 0.25 { d / c } else { 4.0 * d };
                add(line[i], mul(m, k))
            }
        })
        .collect()
}

/// Point and unit tangent at arc length `s` (clamped to the line).
pub fn point_at(line: &[P2], s: f64) -> (P2, P2) {
    let line = clean(line);
    if line.len() < 2 {
        return (line.first().copied().unwrap_or([0.0, 0.0]), [1.0, 0.0]);
    }
    let mut rest = s.max(0.0);
    for w in line.windows(2) {
        let l = len(sub(w[1], w[0]));
        let u = unit(sub(w[1], w[0]));
        if rest <= l {
            return (add(w[0], mul(u, rest)), u);
        }
        rest -= l;
    }
    let k = line.len();
    (line[k - 1], unit(sub(line[k - 1], line[k - 2])))
}

/// The part of a polyline between arc lengths `s0 < s1`, keeping vertices.
pub fn sub_line(line: &[P2], s0: f64, s1: f64) -> Vec<P2> {
    let line = clean(line);
    let mut out = Vec::new();
    let mut acc = 0.0;
    for w in line.windows(2) {
        let l = len(sub(w[1], w[0]));
        let (a, b) = (acc, acc + l);
        if b >= s0 && a <= s1 && l > 0.0 {
            let u = unit(sub(w[1], w[0]));
            let p0 = add(w[0], mul(u, (s0 - a).max(0.0)));
            let p1 = add(w[0], mul(u, (s1 - a).min(l)));
            if out.last().is_none_or(|q: &P2| len(sub(*q, p0)) > 1e-9) {
                out.push(p0);
            }
            out.push(p1);
        }
        acc = b;
        if acc > s1 {
            break;
        }
    }
    clean(&out)
}

/// A strip of `width` centred on a polyline as one closed ring.
pub fn strip(line: &[P2], width: f64) -> Vec<P2> {
    let mut ring = offset_polyline(line, -width / 2.0);
    let mut l = offset_polyline(line, width / 2.0);
    l.reverse();
    ring.extend(l);
    ring
}

// ------------------------------------------------------------------- lines

/// A continuous line of `width` at `offset` from the axis.
pub fn continuous_line(axis: &[P2], width: f64, offset: f64) -> Vec<Marking> {
    let c = offset_polyline(axis, offset);
    if c.len() < 2 {
        return vec![];
    }
    vec![Marking {
        kind: MarkingKind::Continuous,
        ring: strip(&c, width),
    }]
}

/// Stripe intervals `[s0, s1]` of a dash pattern over `[0, total]`.
/// `phase` is how far into the pattern the line starts (0 = with a stripe);
/// stripes cut by the ends are clipped.
pub fn dash_intervals(total: f64, stripe: f64, gap: f64, phase: f64) -> Vec<[f64; 2]> {
    let period = stripe + gap;
    if !(period > 0.0 && stripe > 0.0) {
        return vec![];
    }
    let mut out = Vec::new();
    let mut s = -phase.rem_euclid(period);
    while s < total {
        let (a, b) = (s.max(0.0), (s + stripe).min(total));
        if b - a > 1e-9 {
            out.push([a, b]);
        }
        s += period;
    }
    out
}

/// A line of a given type at `offset` from the axis, dashed per its pattern.
pub fn line_marking(axis: &[P2], lt: &LineType, offset: f64, phase: f64) -> Vec<Marking> {
    if lt.is_continuous() {
        return continuous_line(axis, lt.width, offset);
    }
    let c = offset_polyline(axis, offset);
    dash_intervals(length(&c), lt.stripe, lt.gap, phase)
        .into_iter()
        .map(|[a, b]| Marking {
            kind: MarkingKind::Dashed,
            ring: strip(&sub_line(&c, a, b), lt.width),
        })
        .collect()
}

/// Double line: `left_type` and `right_type` with a clear `spacing` between
/// them, centred on `offset`.
pub fn double_line(
    axis: &[P2],
    left_type: &LineType,
    right_type: &LineType,
    spacing: f64,
    offset: f64,
    phase: f64,
) -> Vec<Marking> {
    let mut out = line_marking(
        axis,
        left_type,
        offset + spacing / 2.0 + left_type.width / 2.0,
        phase,
    );
    out.extend(line_marking(
        axis,
        right_type,
        offset - spacing / 2.0 - right_type.width / 2.0,
        phase,
    ));
    out
}

// ----------------------------------------------------------------- symbols

/// Shark teeth along the give-way line `a → b`, centred over its length.
/// The apexes point to the left of `a → b` (towards the approaching
/// traffic): choose the order of `a`, `b` accordingly.
pub fn shark_teeth(a: P2, b: P2, t: &SharkTeeth) -> Vec<Marking> {
    let w = len(sub(b, a));
    let pitch = t.base + t.gap;
    if !(w > 0.0 && t.base > 0.0 && pitch > 0.0) {
        return vec![];
    }
    let n = ((w + t.gap) / pitch + 1e-9).floor() as usize;
    let used = n as f64 * t.base + n.saturating_sub(1) as f64 * t.gap;
    let u = unit(sub(b, a));
    let nrm = left(u);
    (0..n)
        .map(|i| {
            let s = (w - used) / 2.0 + i as f64 * pitch;
            let p0 = add(a, mul(u, s));
            let p1 = add(a, mul(u, s + t.base));
            let apex = add(add(a, mul(u, s + t.base / 2.0)), mul(nrm, t.height));
            Marking {
                kind: MarkingKind::SharkTooth,
                ring: vec![p0, p1, apex],
            }
        })
        .collect()
}

/// Arrow at `at` (the arrow's tail) pointing along `dir`.
pub fn arrow(at: P2, dir: P2, kind: ArrowKind, st: &ArrowStyle) -> Vec<Marking> {
    let f = unit(dir);
    if len(f) == 0.0 || st.length <= st.head_length {
        return vec![];
    }
    let l = left(f);
    let loc = |u: f64, v: f64| add(at, add(mul(f, u), mul(l, v)));
    let shaft_end = st.length - st.head_length;
    // Centre line in local (u along, v left) coordinates.
    let centre: Vec<P2> = match kind {
        ArrowKind::Straight => vec![[0.0, 0.0], [shaft_end, 0.0]],
        ArrowKind::Left | ArrowKind::Right => {
            let side = if kind == ArrowKind::Left { 1.0 } else { -1.0 };
            let bend = 0.45 * st.length;
            let diag = (st.length - bend - st.head_length * std::f64::consts::FRAC_1_SQRT_2)
                .max(st.head_length * 0.5);
            let k = diag * std::f64::consts::FRAC_1_SQRT_2;
            vec![[0.0, 0.0], [bend, 0.0], [bend + k, side * k]]
        }
    };
    let n = centre.len();
    let end_dir = unit(sub(centre[n - 1], centre[n - 2]));
    let end_left = left(end_dir);
    let right_side = offset_polyline(&centre, -st.shaft_width / 2.0);
    let left_side = offset_polyline(&centre, st.shaft_width / 2.0);
    let tip = centre[n - 1];
    let mut ring: Vec<P2> = right_side;
    ring.push(add(tip, mul(end_left, -st.head_width / 2.0)));
    ring.push(add(tip, mul(end_dir, st.head_length)));
    ring.push(add(tip, mul(end_left, st.head_width / 2.0)));
    ring.extend(left_side.into_iter().rev());
    let ring = ring.into_iter().map(|p| loc(p[0], p[1])).collect();
    vec![Marking {
        kind: MarkingKind::Arrow,
        ring,
    }]
}

/// Arrow at station `s` of a lane axis (`offset` sideways), pointing along it.
pub fn arrow_at_station(
    axis: &[P2],
    s: f64,
    offset: f64,
    kind: ArrowKind,
    st: &ArrowStyle,
) -> Vec<Marking> {
    let (p, t) = point_at(axis, s);
    arrow(add(p, mul(left(t), offset)), t, kind, st)
}

/// Zebra crossing centred at `centre` on a road running along `dir`,
/// stripes across a carriageway of `width`, centred.
pub fn zebra(centre: P2, dir: P2, width: f64, z: &Zebra) -> Vec<Marking> {
    let f = unit(dir);
    let pitch = z.stripe_width + z.gap;
    if len(f) == 0.0 || !(width > 0.0 && z.stripe_width > 0.0 && pitch > 0.0) {
        return vec![];
    }
    let l = left(f);
    let n = ((width + z.gap) / pitch + 1e-9).floor() as usize;
    let used = n as f64 * z.stripe_width + n.saturating_sub(1) as f64 * z.gap;
    let half = z.length / 2.0;
    (0..n)
        .map(|i| {
            let v0 = -width / 2.0 + (width - used) / 2.0 + i as f64 * pitch;
            let v1 = v0 + z.stripe_width;
            let p = |u: f64, v: f64| add(centre, add(mul(f, u), mul(l, v)));
            Marking {
                kind: MarkingKind::Zebra,
                ring: vec![p(-half, v0), p(half, v0), p(half, v1), p(-half, v1)],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_count_over_known_length() {
        let nl = profile_nl();
        let lt = nl.line("3-9 (0.15)").unwrap();
        let axis = [[0.0, 0.0], [120.0, 0.0]];
        let m = line_marking(&axis, lt, 0.0, 0.0);
        assert_eq!(m.len(), 10);
        for d in &m {
            assert!((ring_area(&d.ring).abs() - 3.0 * 0.15).abs() < 1e-9);
        }
        // A phase of 4 m clips the first stripe: none at the start, 10 total.
        let m = line_marking(&axis, lt, 0.0, 4.0);
        assert_eq!(m.len(), 10);
        assert!((m[0].ring[0][0] - 8.0).abs() < 1e-9);
    }

    #[test]
    fn corner_keeps_pattern_continuous() {
        let lt = LineType::new("1-3", 0.15, 1.0, 3.0);
        let axis = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        // Phase 2.5: stripes start at 1.5, 5.5, 9.5 (around the corner), …
        let m = line_marking(&axis, &lt, 0.0, 2.5);
        assert_eq!(m.len(), 5);
        // Every stripe, including the bent one, has stripe·width area.
        for d in &m {
            assert!(
                (ring_area(&d.ring).abs() - 0.15).abs() < 1e-9,
                "{:?}",
                d.ring
            );
        }
        // The corner stripe passes through the corner vertex region.
        assert!(m[2]
            .ring
            .iter()
            .any(|p| (p[0] - 10.0).abs() < 0.1 && p[1].abs() < 0.1));
        // Next stripe starts 4 m after the corner one along the line: at (10, 3.5).
        let (p, _) = point_at(&axis, 13.5);
        assert!(m[3]
            .ring
            .iter()
            .any(|q| len(sub(*q, add(p, [0.075, 0.0]))) < 1e-9));
    }

    #[test]
    fn offset_line_stays_parallel() {
        let axis = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        let o = offset_polyline(&axis, 1.0); // left = inside of the left turn
        assert_eq!(o, vec![[0.0, 1.0], [9.0, 1.0], [9.0, 10.0]]);
    }

    #[test]
    fn continuous_and_double_lines() {
        let axis = [[0.0, 0.0], [50.0, 0.0]];
        let c = continuous_line(&axis, 0.10, 0.0);
        assert!((ring_area(&c[0].ring).abs() - 5.0).abs() < 1e-9);
        let lt = LineType::new("cont", 0.10, 0.0, 0.0);
        let d = double_line(&axis, &lt, &lt, 0.10, 0.0, 0.0);
        assert_eq!(d.len(), 2);
        let ys: Vec<f64> = d
            .iter()
            .map(|m| m.ring.iter().map(|p| p[1]).sum::<f64>() / 4.0)
            .collect();
        assert!((ys[0] - 0.10).abs() < 1e-9 && (ys[1] + 0.10).abs() < 1e-9);
    }

    #[test]
    fn shark_teeth_across_width() {
        let nl = profile_nl();
        let t = nl.teeth("0.50-0.50").unwrap();
        // 3.50 m lane: (3.5 + 0.5) / 1.0 = 4 teeth.
        let m = shark_teeth([0.0, 0.0], [3.5, 0.0], t);
        assert_eq!(m.len(), 4);
        // Centred: first base starts at (3.5 − 3.5) / 2 = 0.
        assert!(m[0].ring[0][0].abs() < 1e-9);
        // Apex left of a→b, at height 0.5.
        assert!((m[0].ring[2][1] - 0.5).abs() < 1e-9);
        // 3.0 m: 3 teeth, centred with 0.25 m margin.
        let m = shark_teeth([0.0, 0.0], [3.0, 0.0], t);
        assert_eq!(m.len(), 3);
        assert!((m[0].ring[0][0] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn zebra_stripes() {
        let nl = profile_nl();
        let m = zebra([0.0, 0.0], [1.0, 0.0], 6.0, &nl.zebra);
        // (6.0 + 0.5) / 1.0 = 6 stripes of 0.5 × 4.0.
        assert_eq!(m.len(), 6);
        for s in &m {
            assert!((ring_area(&s.ring).abs() - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn arrows_are_simple_rings() {
        let st = ArrowStyle::default();
        for k in [ArrowKind::Straight, ArrowKind::Left, ArrowKind::Right] {
            let m = arrow([0.0, 0.0], [0.0, 1.0], k, &st);
            assert_eq!(m.len(), 1);
            assert!(ring_area(&m[0].ring).abs() > 0.5);
        }
        let s = arrow([0.0, 0.0], [0.0, 1.0], ArrowKind::Straight, &st);
        let tip = s[0].ring.iter().fold(f64::MIN, |a, p| a.max(p[1]));
        assert!((tip - st.length).abs() < 1e-9);
        let l = arrow([0.0, 0.0], [0.0, 1.0], ArrowKind::Left, &st);
        // Heading north, a left arrow ends to the west.
        assert!(l[0].ring.iter().any(|p| p[0] < -1.0));
    }
}
