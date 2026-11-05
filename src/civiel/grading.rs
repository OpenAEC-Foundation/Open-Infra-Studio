//! Grading: 3D feature lines whose heights come from the surface, from a start level and a grade per
//! segment, or from fixed levels, with edit commands; plateaus at a level
//! with a fall that slope to the existing ground with separate cut and fill
//! slopes (the daylight line); slopes from an open feature line to the
//! ground, over a width or to a level. Every grading joins one design
//! surface ("bouwrijp", OG-TRIANGULATIE status N) built with `terrein::Tin`
//! faces, so contours, analysis and volumes work on it. And the exact
//! TIN-to-TIN earthwork with bulking and compaction factors and a cut/fill
//! map in colour bands.
//!
//! Inside, coordinates and heights are drawing units; arguments are metres.

use std::collections::HashMap;

use ocs_plugin_api::host::codec::entities::{Polyline3D, TextHorizontalAlignment, TextVerticalAlignment, Vertex3DPolyline};
use ocs_plugin_api::host::codec::{EntityType, Handle, Line};

use super::as_profiel::Seg;
use super::terrein::{self, area_centroid, clip_half, clip_to_triangle, minus_triangle, Plane, Tin, P3};
use super::{api, record, v3, Ctx, Maaiveld, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::nlcs::{Element, LayerSpec, Status};

/// A text in the interface language: a Dutch and an English format string
/// with the same arguments.
macro_rules! tf {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}
pub(crate) use tf;

fn tr(nl: &'static str, en: &'static str) -> &'static str {
    crate::lang::tr(nl, en)
}

fn dutch_ui() -> bool {
    crate::lang::current() == crate::lang::Lang::Nl
}

/// A number with `d` decimals: a decimal comma in Dutch, a point in English.
pub fn dec(v: f64, d: usize) -> String {
    let s = format!("{v:.d$}");
    if dutch_ui() { s.replace('.', ",") } else { s }
}

/// A level with its sign: "+1,50" (Dutch) or "+1.50".
pub fn lvl(z: f64) -> String {
    let s = format!("{z:+.2}");
    if dutch_ui() { s.replace('.', ",") } else { s }
}

/// A whole number in thousands: "1.234.567" (Dutch) or "1,234,567".
pub fn whole(v: f64) -> String {
    let s = terrein::m3(v.abs());
    let s = if dutch_ui() { s } else { s.replace('.', ",") };
    if v < -0.5 { format!("-{s}") } else { s }
}

// ── Lines and heights ──────────────────────────────────────────────────────

/// Where a grading triangle lies: on the plateau itself, or on a slope that
/// runs down from it (fill) or up from it (cut).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Core,
    Fill,
    Cut,
}

impl Part {
    pub fn code(self) -> &'static str {
        match self {
            Part::Core => "kern",
            Part::Fill => "ophoging",
            Part::Cut => "ontgraving",
        }
    }

    pub fn from_code(s: &str) -> Option<Part> {
        [Part::Core, Part::Fill, Part::Cut].into_iter().find(|p| p.code() == s)
    }
}

/// A design triangle and what it belongs to.
pub type Tri = ([P3; 3], Part);

/// A line read from the drawing: its corners with heights (drawing units),
/// arcs as chords of at most 5°.
#[derive(Clone, Debug, PartialEq)]
pub struct Line3 {
    pub pts: Vec<P3>,
    pub closed: bool,
    /// Whether the line carries heights (a 3D polyline or an elevation).
    pub has_z: bool,
}

fn chords(v: &[(XY, f64, f64)], closed: bool) -> Vec<P3> {
    let n = v.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    let mut out = Vec::new();
    for i in 0..n {
        let (p, bulge, z) = v[i];
        out.push([p[0], p[1], z]);
        if i < segs && bulge.abs() > 1e-9 {
            let (q, _, z2) = v[(i + 1) % n];
            let seg = Seg::from_bulge(p, q, bulge);
            let k = ((4.0 * bulge.atan()).abs() / 5f64.to_radians()).ceil().max(1.0) as usize;
            for j in 1..k {
                let t = j as f64 / k as f64;
                let (c, _) = seg.at(seg.length() * t);
                out.push([c[0], c[1], z + (z2 - z) * t]);
            }
        }
    }
    out
}

/// The line of a 3D polyline, polyline, or line.
pub fn line_of(e: &EntityType) -> Option<Line3> {
    let (mut pts, mut closed, has_z) = match e {
        EntityType::Polyline3D(p) => {
            let v: Vec<P3> = p.vertices.iter().map(|v| [v.position.x, v.position.y, v.position.z]).collect();
            let z = v.iter().any(|p| p[2] != 0.0);
            (v, p.flags.closed, z)
        }
        EntityType::LwPolyline(l) => {
            let v: Vec<(XY, f64, f64)> = l.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge, l.elevation)).collect();
            (chords(&v, l.is_closed), l.is_closed, l.elevation != 0.0)
        }
        EntityType::Polyline2D(p) => {
            let v: Vec<(XY, f64, f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge, v.location.z)).collect();
            let z = v.iter().any(|p| p.2 != 0.0);
            (chords(&v, p.flags.is_closed()), p.flags.is_closed(), z)
        }
        EntityType::Line(l) => (vec![[l.start.x, l.start.y, l.start.z], [l.end.x, l.end.y, l.end.z]], false, l.start.z != 0.0 || l.end.z != 0.0),
        _ => return None,
    };
    pts.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9);
    // A closing point repeated at the end.
    if pts.len() > 3 {
        let (a, b) = (pts[0], pts[pts.len() - 1]);
        if (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9 {
            pts.pop();
            closed = true;
        }
    }
    (pts.len() >= 2).then_some(Line3 { pts, closed, has_z })
}

fn plan(a: P3, b: P3) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Plan length from the first corner to each corner.
pub fn cumulative(pts: &[P3]) -> Vec<f64> {
    let mut out = vec![0.0];
    for w in pts.windows(2) {
        out.push(out[out.len() - 1] + plan(w[0], w[1]));
    }
    out
}

/// Grade per segment (%, rising along the line > 0), the closing one too.
pub fn grades(pts: &[P3], closed: bool) -> Vec<f64> {
    let n = pts.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    (0..segs)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let l = plan(a, b);
            if l > 0.0 { (b[2] - a[2]) / l * 100.0 } else { 0.0 }
        })
        .collect()
}

/// Heights from a start level and a grade per segment (%; the last repeats).
pub fn by_grades(pts: &[P3], start: f64, grade: &[f64]) -> Vec<f64> {
    let mut z = vec![start];
    for (i, w) in pts.windows(2).enumerate() {
        let g = grade.get(i).or(grade.last()).copied().unwrap_or(0.0);
        z.push(z[i] + g / 100.0 * plan(w[0], w[1]));
    }
    z
}

/// Fixed levels at some corners; linear by length in between, level before
/// the first and after the last.
pub fn by_levels(pts: &[P3], levels: &[Option<f64>]) -> Result<Vec<f64>, String> {
    let known: Vec<usize> = (0..pts.len()).filter(|i| levels.get(*i).copied().flatten().is_some()).collect();
    let (Some(&first), Some(&last)) = (known.first(), known.last()) else {
        return Err(tr("geen enkel peil gegeven", "no level given").into());
    };
    let s = cumulative(pts);
    let z = |i: usize| levels[i].unwrap();
    let mut out = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        out.push(if i <= first {
            z(first)
        } else if i >= last {
            z(last)
        } else {
            let a = *known.iter().rev().find(|k| **k <= i).unwrap();
            let b = *known.iter().find(|k| **k >= i).unwrap();
            if a == b || s[b] - s[a] <= 0.0 { z(a) } else { z(a) + (z(b) - z(a)) * (s[i] - s[a]) / (s[b] - s[a]) }
        });
    }
    Ok(out)
}

/// A number with a decimal point or comma.
pub fn number(t: &str) -> Result<f64, String> {
    let t = t.trim();
    t.parse::<f64>().or_else(|_| t.replace(',', ".").parse::<f64>()).map_err(|_| tf!("geen getal: '{t}'", "not a number: '{t}'"))
}

/// Levels at corners: "2.5;;;3.0" (one per corner, empty = interpolate) or
/// "1:2.5;4:3.0" (corner number from 1, level).
pub fn parse_levels(s: &str, n: usize) -> Result<Vec<Option<f64>>, String> {
    let mut out = vec![None; n];
    if s.contains(':') {
        for t in s.split(';').filter(|t| !t.trim().is_empty()) {
            let (i, z) = t.split_once(':').ok_or_else(|| tf!("verwacht punt:peil, kreeg '{t}'", "expected point:level, got '{t}'"))?;
            let i = number(i)? as usize;
            if i == 0 || i > n {
                return Err(tf!("punt {i} bestaat niet (1 tot {n})", "point {i} does not exist (1 to {n})"));
            }
            out[i - 1] = Some(number(z)?);
        }
    } else {
        let parts: Vec<&str> = s.split(';').collect();
        if parts.len() > n {
            return Err(tf!("{} peilen voor {n} punten", "{} levels for {n} points", parts.len()));
        }
        for (i, t) in parts.iter().enumerate() {
            if !t.trim().is_empty() {
                out[i] = Some(number(t)?);
            }
        }
    }
    Ok(out)
}

/// Douglas–Peucker on (distance, height): the indices to keep.
fn simplify(sz: &[(f64, f64)], tol: f64) -> Vec<usize> {
    let mut keep = vec![false; sz.len()];
    if sz.is_empty() {
        return Vec::new();
    }
    keep[0] = true;
    keep[sz.len() - 1] = true;
    let mut stack = vec![(0usize, sz.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (s0, z0, s1, z1) = (sz[a].0, sz[a].1, sz[b].0, sz[b].1);
        let mut worst = (0.0, a);
        for (i, p) in sz.iter().enumerate().take(b).skip(a + 1) {
            let z = if s1 > s0 { z0 + (z1 - z0) * (p.0 - s0) / (s1 - s0) } else { z0 };
            let d = (p.1 - z).abs();
            if d > worst.0 {
                worst = (d, i);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    (0..sz.len()).filter(|i| keep[*i]).collect()
}

/// A line draped on a surface: its corners, and with `step` > 0 also points
/// in between where the surface's grade changes (within `tol`). Corners
/// without a height take their neighbours'.
pub fn drape(pts: &[P3], closed: bool, step: f64, tol: f64, h: &mut dyn FnMut(XY) -> Option<f64>) -> Result<Vec<P3>, String> {
    let n = pts.len();
    let segs = if closed { n } else { n - 1 };
    let mut out: Vec<P3> = Vec::new();
    let mut levels: Vec<Option<f64>> = Vec::new();
    for i in 0..n {
        let z = h([pts[i][0], pts[i][1]]);
        out.push([pts[i][0], pts[i][1], z.unwrap_or(0.0)]);
        levels.push(z);
        if i >= segs || step <= 0.0 {
            continue;
        }
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = plan(a, b);
        let k = (len / step).ceil() as usize;
        // (distance, height) along the segment, the corners at both ends.
        let mut sz: Vec<(f64, f64, XY)> = Vec::new();
        if let Some(z) = z {
            sz.push((0.0, z, [a[0], a[1]]));
        }
        for j in 1..k {
            let t = j as f64 / k as f64;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if let Some(z) = h(p) {
                sz.push((len * t, z, p));
            }
        }
        if let Some(zb) = h([b[0], b[1]]) {
            sz.push((len, zb, [b[0], b[1]]));
        }
        let plain: Vec<(f64, f64)> = sz.iter().map(|p| (p.0, p.1)).collect();
        for i in simplify(&plain, tol) {
            let (s, z, p) = sz[i];
            if s > 1e-12 && s < len - 1e-12 {
                out.push([p[0], p[1], z]);
                levels.push(Some(z));
            }
        }
    }
    let z = by_levels(&out, &levels).map_err(|_| tr("geen hoogtes langs de lijn: ligt hij buiten het terrein?", "no heights along the line: is it outside the terrain?").to_string())?;
    Ok(out.iter().zip(z).map(|(p, z)| [p[0], p[1], z]).collect())
}

/// Which corner an edit works on.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// Corner number (from 0).
    Corner(usize),
    /// The corner nearest to this point.
    Near(XY),
}

/// A change to a feature line (heights in drawing units).
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// One corner to a level.
    Level { at: Pick, z: f64 },
    /// Everything up (> 0) or down.
    Raise(f64),
    /// A new height point on the line, at a level or interpolated.
    Insert { at: XY, z: Option<f64> },
    /// Constant grade between two corners (from < to, from 0): at this grade
    /// (rise per run) from the first, or straight between the two.
    Grade { from: usize, to: usize, grade: Option<f64> },
}

fn corner(pts: &[P3], at: &Pick) -> Result<usize, String> {
    match at {
        Pick::Corner(i) if *i < pts.len() => Ok(*i),
        Pick::Corner(i) => Err(tf!("punt {} bestaat niet (1 tot {})", "point {} does not exist (1 to {})", i + 1, pts.len())),
        Pick::Near(p) => Ok((0..pts.len()).min_by(|a, b| plan(pts[*a], [p[0], p[1], 0.0]).total_cmp(&plan(pts[*b], [p[0], p[1], 0.0]))).unwrap_or(0)),
    }
}

/// Apply an edit; a short description back (corners numbered from 1).
pub fn apply(pts: &mut Vec<P3>, closed: bool, edit: &Edit, f: f64) -> Result<String, String> {
    match edit {
        Edit::Level { at, z } => {
            let i = corner(pts, at)?;
            pts[i][2] = *z;
            Ok(tf!("punt {} op {} m", "point {} at {} m", i + 1, lvl(z / f)))
        }
        Edit::Raise(dz) => {
            for p in pts.iter_mut() {
                p[2] += dz;
            }
            Ok(tf!("{} m {}", "{} m {}", dec((dz / f).abs(), 3), if *dz >= 0.0 { tr("omhoog", "up") } else { tr("omlaag", "down") }))
        }
        Edit::Insert { at, z } => {
            let n = pts.len();
            let segs = if closed { n } else { n - 1 };
            let mut best = (f64::MAX, 0usize, 0.0f64);
            for i in 0..segs {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l2 = (dx * dx + dy * dy).max(1e-24);
                let t = (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
                let d = (a[0] + dx * t - at[0]).hypot(a[1] + dy * t - at[1]);
                if d < best.0 {
                    best = (d, i, t);
                }
            }
            let (_, i, t) = best;
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let len = plan(a, b);
            if t * len < 1e-3 * f || (1.0 - t) * len < 1e-3 * f {
                let k = if t * len < 1e-3 * f { i } else { (i + 1) % n };
                return match z {
                    Some(z) => {
                        pts[k][2] = *z;
                        Ok(tf!("punt {} (bestaand) op {} m", "point {} (existing) at {} m", k + 1, lvl(z / f)))
                    }
                    None => Err(tf!("het punt valt op hoekpunt {}", "the point falls on corner {}", k + 1)),
                };
            }
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, z.unwrap_or(a[2] + (b[2] - a[2]) * t)];
            pts.insert(i + 1, p);
            Ok(tf!("hoogtepunt {} ingevoegd op {} m", "height point {} inserted at {} m", i + 2, lvl(p[2] / f)))
        }
        Edit::Grade { from, to, grade } => {
            let (a, b) = (*from, *to);
            if a >= b || b >= pts.len() {
                return Err(tf!("kies twee punten van 1 tot {}, de eerste vóór de tweede", "choose two points from 1 to {}, the first before the second", pts.len()));
            }
            let s = cumulative(&pts[a..=b]);
            let (za, zb) = (pts[a][2], pts[b][2]);
            let g = grade.unwrap_or(if s[s.len() - 1] > 0.0 { (zb - za) / s[s.len() - 1] } else { 0.0 });
            for (k, sk) in s.iter().enumerate().skip(1) {
                pts[a + k][2] = za + g * sk;
            }
            Ok(tf!("helling {} % van punt {} tot {}", "grade {} % from point {} to {}", dec(g * 100.0, 2), a + 1, b + 1))
        }
    }
}

// ── Grading: slopes from an edge to a target ───────────────────────────────

/// What a slope runs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// The existing ground (daylight): cut slope 1:`cut`, fill slope
    /// 1:`fill`, at most `reach` far.
    Ground { cut: f64, fill: f64, reach: f64 },
    /// A fixed width at a grade (rise per run, < 0 runs down).
    Width { width: f64, grade: f64 },
    /// A level, at slope 1:`n`.
    Level { z: f64, n: f64 },
}

/// How a slope ray ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// On the ground: a daylight point.
    Daylight,
    /// At the width or level it was given.
    Target,
    /// Where another part of the edge is nearer (an inside corner).
    Clipped,
    /// At the reach without finding the ground.
    Reach,
    /// Where the ground has no height.
    NoGround,
}

/// One slope line, square to the edge (or fanned round an outside corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub from: P3,
    pub to: P3,
    pub part: Part,
    pub end: End,
}

/// Edge points every `step`; outside corners fanned every `fan` radians;
/// the ground searched every `res`; `tol` is a length that counts as zero.
#[derive(Clone, Copy, Debug)]
pub struct Sampling {
    pub step: f64,
    pub fan: f64,
    pub res: f64,
    pub tol: f64,
}

impl Sampling {
    /// For a drawing with `f` units per metre: points every `step` m.
    pub fn metres(f: f64, step: f64) -> Sampling {
        Sampling { step: step.max(0.1) * f, fan: 10f64.to_radians(), res: 0.25 * f, tol: 1e-6 * f }
    }
}

pub struct Grading {
    pub rays: Vec<Ray>,
    /// The design surface of this grading: slopes, and the plateau when asked.
    pub tris: Vec<Tri>,
}

fn seg_dist(p: XY, a: P3, b: P3) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (a[0] + dx * t - p[0]).hypot(a[1] + dy * t - p[1])
}

fn tri_area(t: &[P3; 3]) -> f64 {
    ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])) / 2.0
}

/// Signed plan area of a ring (counter-clockwise > 0).
pub fn ring_area(pts: &[P3]) -> f64 {
    let xy: Vec<XY> = pts.iter().map(|p| [p[0], p[1]]).collect();
    area_centroid(&xy).0
}

/// Slopes from `edge` to `target` on one side (`side` +1 right of the
/// direction of travel, −1 left; outside of a counter-clockwise ring is
/// right). Every point of the slope belongs to the nearest point of the
/// edge, so inside corners need no clean-up and outside corners become
/// cones. With `core` (closed edges) the plateau inside is triangulated too.
pub fn grade(edge: &[P3], closed: bool, side: f64, target: Target, s: Sampling, ground: &mut dyn FnMut(XY) -> Option<f64>, core: bool) -> Result<Grading, String> {
    let mut pts: Vec<P3> = edge.to_vec();
    pts.dedup_by(|a, b| plan(*a, *b) <= s.tol);
    if closed && pts.len() > 2 && plan(pts[0], pts[pts.len() - 1]) <= s.tol {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 || (closed && n < 3) {
        return Err(tr("te weinig punten voor een talud", "too few points for a slope").into());
    }
    let segs = if closed { n } else { n - 1 };
    let seg = |i: usize| (pts[i], pts[(i + 1) % n]);
    let normal = |i: usize| -> XY {
        let (a, b) = seg(i);
        let l = plan(a, b).max(1e-300);
        [side * (b[1] - a[1]) / l, -side * (b[0] - a[0]) / l]
    };
    // Ray starts in order along the edge: (point, direction or None for none).
    let mut starts: Vec<(P3, Option<XY>)> = Vec::new();
    for (i, &corner_pt) in pts.iter().enumerate() {
        let prev = if closed { Some((i + n - 1) % n) } else { i.checked_sub(1) };
        let next = (i < segs).then_some(i);
        match (prev, next) {
            (Some(a), Some(b)) => {
                let (na, nb) = (normal(a), normal(b));
                let turn = (na[0] * nb[1] - na[1] * nb[0]).atan2(na[0] * nb[0] + na[1] * nb[1]);
                if turn.abs() < 1e-9 {
                    starts.push((corner_pt, Some(na)));
                } else if side * turn > 0.0 {
                    // Outside corner: a fan from one normal to the other.
                    let k = (turn.abs() / s.fan).ceil().max(1.0) as usize;
                    for j in 0..=k {
                        let a = turn * j as f64 / k as f64;
                        let (c, sn) = (a.cos(), a.sin());
                        starts.push((corner_pt, Some([na[0] * c - na[1] * sn, na[0] * sn + na[1] * c])));
                    }
                } else {
                    // Inside corner: the slopes of both sides meet on the bisector.
                    starts.push((corner_pt, None));
                }
            }
            (None, Some(b)) => starts.push((corner_pt, Some(normal(b)))),
            (Some(a), None) => starts.push((corner_pt, Some(normal(a)))),
            (None, None) => {}
        }
        if let Some(i) = next {
            let (a, b) = seg(i);
            let k = (plan(a, b) / s.step).ceil().max(1.0) as usize;
            for j in 1..k {
                let t = j as f64 / k as f64;
                starts.push(([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t], Some(normal(i))));
            }
        }
    }
    let edge_segs: Vec<(P3, P3)> = (0..segs).map(seg).collect();
    let nearest = |p: XY| edge_segs.iter().map(|(a, b)| seg_dist(p, *a, *b)).fold(f64::MAX, f64::min);
    let mut rays = Vec::with_capacity(starts.len());
    for (q, dir) in starts {
        rays.push(match dir {
            None => Ray { from: q, to: q, part: Part::Fill, end: End::Clipped },
            Some(u) => cast(q, u, target, s, ground, &nearest),
        });
    }
    // Strips between neighbouring rays.
    let mut tris: Vec<Tri> = Vec::new();
    let count = if closed { rays.len() } else { rays.len() - 1 };
    let small = s.tol * s.tol;
    for i in 0..count {
        let (r, t) = (rays[i], rays[(i + 1) % rays.len()]);
        let mut push = |tri: [P3; 3], part: Part| {
            if tri_area(&tri).abs() > small {
                tris.push((tri, part));
            }
        };
        if plan(r.from, t.from) <= s.tol {
            push([r.from, t.to, r.to], r.part);
        } else if plan(r.from, r.to) <= s.tol && plan(t.from, t.to) <= s.tol {
            continue;
        } else if plan(r.from, t.to) <= plan(t.from, r.to) {
            push([r.from, t.from, t.to], t.part);
            push([r.from, t.to, r.to], r.part);
        } else {
            push([r.from, t.from, r.to], r.part);
            push([t.from, t.to, r.to], t.part);
        }
    }
    if core && closed {
        let mut ring: Vec<P3> = rays.iter().map(|r| r.from).collect();
        ring.dedup_by(|a, b| plan(*a, *b) <= s.tol);
        if ring.len() > 3 && plan(ring[0], ring[ring.len() - 1]) <= s.tol {
            ring.pop();
        }
        let o = ring[0];
        let flat: Vec<f64> = ring.iter().flat_map(|p| [p[0] - o[0], p[1] - o[1]]).collect();
        let idx = earcutr::earcut(&flat, &[], 2).map_err(|_| tr("het plateau is niet te trianguleren: snijdt de omtrek zichzelf?", "the plateau cannot be triangulated: does its outline cross itself?").to_string())?;
        for t in idx.as_chunks::<3>().0 {
            let tri = [ring[t[0]], ring[t[1]], ring[t[2]]];
            if tri_area(&tri).abs() > small {
                tris.push((tri, Part::Core));
            }
        }
    }
    Ok(Grading { rays, tris })
}

/// One slope ray from `q` in direction `u` (a unit vector).
fn cast(q: P3, u: XY, target: Target, s: Sampling, ground: &mut dyn FnMut(XY) -> Option<f64>, nearest: &dyn Fn(XY) -> f64) -> Ray {
    let at = |t: f64| [q[0] + u[0] * t, q[1] + u[1] * t];
    let (grade, part, tmax, g0) = match target {
        Target::Ground { cut, fill, reach } => {
            let Some(g0) = ground([q[0], q[1]]) else {
                return Ray { from: q, to: q, part: Part::Fill, end: End::NoGround };
            };
            let filled = q[2] >= g0;
            let grade = if filled { -1.0 / fill.max(0.01) } else { 1.0 / cut.max(0.01) };
            (grade, if filled { Part::Fill } else { Part::Cut }, reach, Some(g0))
        }
        Target::Width { width, grade } => (grade, if grade < 0.0 { Part::Fill } else { Part::Cut }, width, None),
        Target::Level { z, n } => {
            let dz = z - q[2];
            let grade = dz.signum() / n.max(0.01);
            (grade, if dz < 0.0 { Part::Fill } else { Part::Cut }, dz.abs() * n.max(0.01), None)
        }
    };
    let point = |t: f64| {
        let p = at(t);
        [p[0], p[1], q[2] + grade * t]
    };
    let ray = |t: f64, end: End| Ray { from: q, to: point(t), part, end };
    if tmax <= s.tol {
        return ray(0.0, End::Target);
    }
    let mut d_prev = g0.map(|g| q[2] - g).unwrap_or(0.0);
    if g0.is_some() && d_prev.abs() <= s.tol {
        return ray(0.0, End::Daylight);
    }
    let clipped = |t: f64| nearest(at(t)) < t - s.tol.max(t * 1e-9);
    let mut t_prev = 0.0;
    loop {
        let t = (t_prev + s.res).min(tmax);
        let mut t_end = t;
        let mut cut_short = false;
        if clipped(t) {
            // Where another part of the edge becomes nearer.
            let (mut lo, mut hi) = (t_prev, t);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.0;
                if clipped(mid) { hi = mid } else { lo = mid }
            }
            t_end = lo;
            cut_short = true;
        }
        if g0.is_some() {
            let Some(g) = ground(at(t_end)) else {
                return ray(t_prev, End::NoGround);
            };
            let d = q[2] + grade * t_end - g;
            if d == 0.0 || d.signum() != d_prev.signum() {
                let tc = t_prev + (t_end - t_prev) * d_prev / (d_prev - d);
                return ray(tc, End::Daylight);
            }
            d_prev = d;
        }
        if cut_short {
            return ray(t_end, End::Clipped);
        }
        if t >= tmax {
            return ray(tmax, if g0.is_some() { End::Reach } else { End::Target });
        }
        t_prev = t;
    }
}

// ── One design surface from several gradings ───────────────────────────────

/// A uniform grid over triangles, for "which triangles are near here".
struct Grid {
    o: XY,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

fn bbox3(t: &[P3; 3]) -> (XY, XY) {
    let lo = [t[0][0].min(t[1][0]).min(t[2][0]), t[0][1].min(t[1][1]).min(t[2][1])];
    let hi = [t[0][0].max(t[1][0]).max(t[2][0]), t[0][1].max(t[1][1]).max(t[2][1])];
    (lo, hi)
}

impl Grid {
    fn new(tris: &[Tri]) -> Grid {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (t, _) in tris {
            let (a, b) = bbox3(t);
            lo = [lo[0].min(a[0]), lo[1].min(a[1])];
            hi = [hi[0].max(b[0]), hi[1].max(b[1])];
        }
        if tris.is_empty() {
            return Grid { o: [0.0, 0.0], cell: 1.0, cols: 1, rows: 1, cells: vec![Vec::new()] };
        }
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-9);
        let cell = size / (tris.len() as f64).sqrt().ceil().max(1.0);
        let cols = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut g = Grid { o: lo, cell, cols, rows, cells: vec![Vec::new(); cols * rows] };
        for (i, (t, _)) in tris.iter().enumerate() {
            let (a, b) = bbox3(t);
            let (c0, r0, c1, r1) = g.range(a, b);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    g.cells[r * g.cols + c].push(i as u32);
                }
            }
        }
        g
    }

    fn range(&self, lo: XY, hi: XY) -> (usize, usize, usize, usize) {
        let c = |v: f64, o: f64, n: usize| (((v - o) / self.cell).floor().max(0.0) as usize).min(n - 1);
        (c(lo[0], self.o[0], self.cols), c(lo[1], self.o[1], self.rows), c(hi[0], self.o[0], self.cols), c(hi[1], self.o[1], self.rows))
    }

    fn near(&self, lo: XY, hi: XY) -> Vec<usize> {
        let (c0, r0, c1, r1) = self.range(lo, hi);
        let mut out = Vec::new();
        for r in r0..=r1 {
            for c in c0..=c1 {
                out.extend(self.cells[r * self.cols + c].iter().map(|i| *i as usize));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn xy_of(t: &[P3; 3]) -> [XY; 3] {
    t.map(|p| [p[0], p[1]])
}

/// How two overlapping parts combine.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rule {
    Newest,
    Highest,
    Lowest,
}

/// Fill slopes raise what they overlap (the highest wins), cut slopes lower
/// it (the lowest wins); between two plateaus, or a fill and a cut slope,
/// the newest grading wins.
fn rule(new: Part, old: Part) -> Rule {
    match (new, old) {
        (Part::Core, Part::Core) | (Part::Fill, Part::Cut) | (Part::Cut, Part::Fill) => Rule::Newest,
        (Part::Fill, _) | (_, Part::Fill) => Rule::Highest,
        (Part::Cut, _) | (_, Part::Cut) => Rule::Lowest,
    }
}

/// A convex piece with its plane.
type Piece = (Vec<XY>, Plane, Part);

/// An older design surface and a new grading as one surface (see [`rule`]),
/// without T-junctions.
pub fn combine(old: &[Tri], new: &[Tri], tol: f64) -> Vec<Tri> {
    let min_area = tol * tol;
    let (og, ng) = (Grid::new(old), Grid::new(new));
    let mut pieces: Vec<Piece> = Vec::new();
    let poly = |t: &[P3; 3]| -> Vec<XY> {
        let xy = xy_of(t);
        if tri_area(t) >= 0.0 { xy.to_vec() } else { vec![xy[0], xy[2], xy[1]] }
    };
    for (tn, kn) in new {
        let pn = Plane::of(*tn);
        let p = poly(tn);
        let mut outside = vec![p.clone()];
        let (lo, hi) = bbox3(tn);
        for o in og.near(lo, hi) {
            let (to, ko) = &old[o];
            let inter = clip_to_triangle(&p, xy_of(to), tol);
            if area_centroid(&inter).0 <= min_area {
                continue;
            }
            let po = Plane::of(*to);
            match rule(*kn, *ko) {
                Rule::Newest => pieces.push((inter, pn, *kn)),
                r => {
                    let up = |q: XY| pn.z(q) - po.z(q);
                    let (hi_new, lo_new) = (clip_half(&inter, tol, up), clip_half(&inter, tol, |q| -up(q)));
                    let (win_hi, win_lo) = if r == Rule::Highest { ((pn, *kn), (po, *ko)) } else { ((po, *ko), (pn, *kn)) };
                    if area_centroid(&hi_new).0 > min_area {
                        pieces.push((hi_new, win_hi.0, win_hi.1));
                    }
                    if area_centroid(&lo_new).0 > min_area {
                        pieces.push((lo_new, win_lo.0, win_lo.1));
                    }
                }
            }
            outside = outside.iter().flat_map(|q| minus_triangle(q, xy_of(to), tol, min_area)).collect();
        }
        pieces.extend(outside.into_iter().map(|q| (q, pn, *kn)));
    }
    for (to, ko) in old {
        let p = poly(to);
        let mut outside = vec![p.clone()];
        let (lo, hi) = bbox3(to);
        for i in ng.near(lo, hi) {
            let tn = xy_of(&new[i].0);
            if area_centroid(&clip_to_triangle(&p, tn, tol)).0 > min_area {
                outside = outside.iter().flat_map(|q| minus_triangle(q, tn, tol, min_area)).collect();
            }
        }
        let po = Plane::of(*to);
        pieces.extend(outside.into_iter().map(|q| (q, po, *ko)));
    }
    let mut tris = Vec::new();
    for (q, pl, part) in pieces {
        for k in 1..q.len().saturating_sub(1) {
            let t = [q[0], q[k], q[k + 1]].map(|p| [p[0], p[1], pl.z(p)]);
            if tri_area(&t).abs() > min_area {
                tris.push((t, part));
            }
        }
    }
    conform(tris, tol)
}

/// Triangles with no corner of another lying inside one of their edges: a
/// triangle with such points on its edges becomes a fan round its centroid
/// (heights from its own plane), so the faces share their edges again.
pub fn conform(tris: Vec<Tri>, tol: f64) -> Vec<Tri> {
    if tris.is_empty() {
        return tris;
    }
    let key = |p: XY| ((p[0] / tol).round() as i64, (p[1] / tol).round() as i64);
    let mut verts: HashMap<(i64, i64), XY> = HashMap::new();
    let mut total = 0.0;
    for (t, _) in &tris {
        for p in t {
            verts.entry(key([p[0], p[1]])).or_insert([p[0], p[1]]);
        }
        total += tri_area(t).abs();
    }
    let pts: Vec<XY> = verts.into_values().collect();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &pts {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let cell = (total / pts.len() as f64).sqrt().max((hi[0] - lo[0]).max(hi[1] - lo[1]) / 2000.0).max(tol * 10.0);
    let ck = |p: XY| (((p[0] - lo[0]) / cell).floor() as i64, ((p[1] - lo[1]) / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        grid.entry(ck(*p)).or_default().push(i);
    }
    let mut out = Vec::with_capacity(tris.len());
    for (t, part) in tris {
        let mut ring: Vec<P3> = Vec::new();
        let mut split = false;
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            ring.push(a);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            if len <= tol {
                continue;
            }
            let (c0, c1) = (ck([a[0].min(b[0]), a[1].min(b[1])]), ck([a[0].max(b[0]), a[1].max(b[1])]));
            let mut on: Vec<(f64, XY)> = Vec::new();
            for cx in c0.0..=c1.0 {
                for cy in c0.1..=c1.1 {
                    let Some(list) = grid.get(&(cx, cy)) else { continue };
                    for &i in list {
                        let p = pts[i];
                        let s = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len;
                        if s <= tol || s >= len - tol {
                            continue;
                        }
                        let d = (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / len;
                        if d.abs() <= tol {
                            on.push((s, p));
                        }
                    }
                }
            }
            if !on.is_empty() {
                split = true;
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                on.dedup_by(|x, y| (x.0 - y.0).abs() <= tol);
                for (s, p) in on {
                    let z = a[2] + (b[2] - a[2]) * s / len;
                    ring.push([p[0], p[1], z]);
                }
            }
        }
        if !split {
            out.push((t, part));
            continue;
        }
        let pl = Plane::of(t);
        let c = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0];
        let c = [c[0], c[1], pl.z(c)];
        for k in 0..ring.len() {
            let f = [c, ring[k], ring[(k + 1) % ring.len()]];
            if tri_area(&f).abs() > tol * tol {
                out.push((f, part));
            }
        }
    }
    out
}

/// The faces of a design surface as a TIN (for contours, heights, volumes).
pub fn tin_of(tris: &[Tri]) -> Option<Tin> {
    Tin::from_faces(&tris.iter().map(|t| t.0).collect::<Vec<_>>())
}

// ── In the drawing ──────────────────────────────────────────────────────────

/// Where a command's line comes from.
#[derive(Clone, Debug)]
pub enum Source {
    Handle(Handle),
    /// Project coordinates x,y or x,y,z (metres).
    Points(Vec<Vec<f64>>),
    Selection,
}

impl Source {
    fn handle(&self) -> Vec<Handle> {
        match self {
            Source::Handle(h) => vec![*h],
            _ => Vec::new(),
        }
    }
}

/// The line of a source, and its entity when it is one in the drawing.
fn read_source(ctx: &Ctx, src: &Source, closed: Option<bool>) -> Result<(Line3, Option<EntityType>), String> {
    let (line, e) = match src {
        Source::Points(p) => {
            if p.len() < 2 {
                return Err(tr("geef minstens twee punten: PUNTEN=x,y;x,y;…", "give at least two points: PUNTEN=x,y;x,y;…").into());
            }
            let has_z = p.iter().all(|c| c.len() > 2);
            let pts = p.iter().map(|c| {
                let q = ctx.to_drawing([c[0], c[1]]);
                [q[0], q[1], c.get(2).copied().unwrap_or(0.0) * ctx.f]
            }).collect();
            (Line3 { pts, closed: false, has_z }, None)
        }
        Source::Handle(_) | Source::Selection => {
            let h = match src {
                Source::Handle(h) => *h,
                _ => {
                    let sel = bridge::selection()?;
                    match sel.len() {
                        0 => return Err(tr("selecteer eerst een lijn of polylijn, of geef LIJN=<handle>", "select a line or polyline first, or give LIJN=<handle>").into()),
                        1 => sel[0],
                        n => return Err(tf!("{n} elementen geselecteerd: selecteer er één", "{n} entities selected: select one")),
                    }
                }
            };
            let e = bridge::entities(&[h])?.into_iter().next().flatten().ok_or_else(|| tf!("geen element met handle {}", "no entity with handle {}", api::handle_text(h)))?;
            let line = line_of(&e).ok_or_else(|| tf!("{} is geen lijn, polylijn of 3D-polylijn", "{} is not a line, polyline or 3D polyline", api::handle_text(h)))?;
            (line, Some(e))
        }
    };
    let mut line = line;
    if let Some(c) = closed {
        line.closed = line.closed || (c && line.pts.len() > 2);
    }
    Ok((line, e))
}

/// The settings with drawing status N (a design) unless the arguments chose one.
pub fn design_options(opt: &super::CivielOptions, keep_status: bool) -> super::CivielOptions {
    let mut o = opt.clone();
    if !keep_status {
        o.nlcs.status = Status::N;
    }
    o
}

/// "Kantlijn 3": one more than the highest number under this index kind.
fn next_name(kind: &str, prefix: &str) -> String {
    let n = bridge::index_read()
        .into_iter()
        .filter(|(k, _, _)| k == kind)
        .filter_map(|(_, key, _)| key.strip_prefix(prefix).and_then(|n| n.trim().parse::<usize>().ok()))
        .max()
        .unwrap_or(0);
    format!("{prefix} {}", n + 1)
}

fn poly3(pts: &[P3], closed: bool, spec: &LayerSpec, k: f64, data: Option<&ocs_plugin_api::host::ExtendedDataRecord>) -> EntityType {
    let mut p = Polyline3D::from_points(pts.iter().map(|p| v3([p[0], p[1]], p[2])).collect());
    p.flags.closed = closed;
    let mut e = EntityType::Polyline3D(p);
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    c.linetype_scale = k;
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

fn line3(a: P3, b: P3, spec: &LayerSpec) -> EntityType {
    let mut e = EntityType::Line(Line::from_points(v3([a[0], a[1]], a[2]), v3([b[0], b[1]], b[2])));
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    e
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

/// A plain tool layer (no NLCS object) in a colour index.
fn tool_layer(name: &str, aci: u8) -> LayerSpec {
    LayerSpec { name: name.to_string(), color: aci, rgb: crate::nlcs::color_rgb(aci), true_color: None, lineweight: None, linetype: "CONTINUOUS".into(), object_id: 0 }
}

fn pct(v: f64) -> String {
    dec(v, 2)
}

fn zs_m(pts: &[P3], f: f64) -> (f64, f64) {
    pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[2] / f), a.1.max(p[2] / f)))
}

/// The object data of a feature line.
fn feature_data(name: &str, source: &str, pts: &[P3], closed: bool, f: f64) -> ocs_plugin_api::host::ExtendedDataRecord {
    let s = cumulative(pts);
    let mut len = s[s.len() - 1];
    if closed {
        len += plan(pts[pts.len() - 1], pts[0]);
    }
    let (lo, hi) = zs_m(pts, f);
    let g: Vec<String> = grades(pts, closed).iter().map(|g| format!("{g:.3}")).collect();
    record(&[
        ("kantlijn", name.to_string()),
        ("bron", source.to_string()),
        ("punten", pts.len().to_string()),
        ("lengte_m", format!("{:.3}", len / f)),
        ("z_min", format!("{lo:.3}")),
        ("z_max", format!("{hi:.3}")),
        ("hellingen", g.join(";")),
    ])
}

/// Labels of a feature line: the level at each corner and the grade of
/// each segment with an arrow down the slope (fewer on long, dense lines).
fn feature_labels(ctx: &Ctx, pts: &[P3], closed: bool, spec: &LayerSpec) -> Vec<EntityType> {
    let (f, k) = (ctx.f, ctx.k);
    let n = pts.len();
    let mut out = Vec::new();
    if !ctx.opt.nlcs.labels {
        return out;
    }
    let dense = n > 30;
    for (i, p) in pts.iter().enumerate() {
        if dense && i != 0 && i != n - 1 {
            continue;
        }
        let t = ctx.text([p[0] + 1.0 * k, p[1] + 1.0 * k], &lvl(p[2] / f), 1.8, 0.0, spec, None);
        out.push(aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Bottom));
    }
    let segs = if closed { n } else { n - 1 };
    for (i, g) in grades(pts, closed).into_iter().enumerate().take(segs) {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let len = plan(a, b);
        if len < 12.0 * k || g.abs() < 0.005 {
            continue;
        }
        let dir = (b[1] - a[1]).atan2(b[0] - a[0]);
        let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let nrm = [-dir.sin(), dir.cos()];
        let at = [m[0] + nrm[0] * 1.5 * k, m[1] + nrm[1] * 1.5 * k];
        let z = (a[2] + b[2]) / 2.0;
        let t = ctx.text(at, &format!("{} %", pct(g.abs())), 1.8, crate::emit::readable(dir), spec, None);
        out.push(aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom));
        // An arrow pointing down the slope, under the text.
        let down = if g < 0.0 { 1.0 } else { -1.0 };
        let u = [dir.cos() * down, dir.sin() * down];
        let (h, w) = (4.0 * k, 0.8 * k);
        let tail = [m[0] - u[0] * h + nrm[0] * 0.5 * k, m[1] - u[1] * h + nrm[1] * 0.5 * k];
        let tip = [m[0] + u[0] * h + nrm[0] * 0.5 * k, m[1] + u[1] * h + nrm[1] * 0.5 * k];
        let back = [tip[0] - u[0] * 1.5 * k, tip[1] - u[1] * 1.5 * k];
        out.push(line3([tail[0], tail[1], z], [tip[0], tip[1], z], spec));
        out.push(line3([back[0] + nrm[0] * w, back[1] + nrm[1] * w, z], [tip[0], tip[1], z], spec));
        out.push(line3([back[0] - nrm[0] * w, back[1] - nrm[1] * w, z], [tip[0], tip[1], z], spec));
    }
    out
}

/// Where a feature line's heights come from.
#[derive(Clone, Debug)]
pub enum Heights {
    /// From a surface: 'B' the existing ground (TIN, else AHN), 'N' the design.
    Surface(char),
    /// Start level (m) and grades per segment (%).
    Grades { start: f64, grades: Vec<f64> },
    /// Levels at corners, as `PEILEN=` gives them.
    Levels(String),
    /// The 3D line's own heights.
    Own,
}

pub struct FeatureRequest {
    pub source: Source,
    pub heights: Heights,
    /// With a surface: points in between every `step` m where the grade
    /// changes (0 = the corners only).
    pub step: f64,
    pub name: Option<String>,
    pub closed: Option<bool>,
}

/// `CIV_KANTLIJN`: a 3D feature line on OG-BREUKLIJN with object data.
pub fn feature_line(opt: &super::CivielOptions, req: FeatureRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let (line, _) = read_source(&ctx, &req.source, req.closed)?;
    let n = line.pts.len();
    let (pts, source): (Vec<P3>, String) = match &req.heights {
        Heights::Surface(c) => {
            progress(tr("Hoogtes van het oppervlak…", "Heights from the surface…").into());
            let doc = super::document(&["tin"], &[])?;
            if *c == 'N' {
                let tin = Tin::of_status(&doc, 'N').ok_or(tr("geen ontwerpoppervlak (N-…-OG-TRIANGULATIE) in de tekening", "no design surface (N-…-OG-TRIANGULATIE) in the drawing"))?;
                let pts = drape(&line.pts, line.closed, req.step * f, 0.005 * f, &mut |p| tin.height(p))?;
                (pts, tr("ontwerpoppervlak (N)", "design surface (N)").into())
            } else {
                let mut ground = Maaiveld::find(&ctx, &doc)?;
                let label = ground.label();
                let pts = drape(&line.pts, line.closed, req.step * f, 0.005 * f, &mut |p| ground.height(p, f).map(|z| z * f)).map_err(|e| ground.error().unwrap_or(e))?;
                ground.note_source();
                (pts, label)
            }
        }
        Heights::Grades { start, grades: g } => {
            let z = by_grades(&line.pts, start * f, g);
            (line.pts.iter().zip(z).map(|(p, z)| [p[0], p[1], z]).collect(), tf!("beginpeil {} m, hellingen {} %", "start level {} m, grades {} %", lvl(*start), g.iter().map(|v| pct(*v)).collect::<Vec<_>>().join("; ")))
        }
        Heights::Levels(raw) => {
            let lv: Vec<Option<f64>> = parse_levels(raw, n)?.into_iter().map(|z| z.map(|z| z * f)).collect();
            let z = by_levels(&line.pts, &lv)?;
            (line.pts.iter().zip(z).map(|(p, z)| [p[0], p[1], z]).collect(), tr("vaste peilen", "fixed levels").into())
        }
        Heights::Own => {
            if !line.has_z {
                return Err(tr("de lijn heeft geen hoogtes: geef HOOGTE=MAAIVELD, START= met HELLING=, of PEILEN=", "the line has no heights: give HOOGTE=MAAIVELD, START= with HELLING=, or PEILEN=").into());
            }
            (line.pts.clone(), tr("eigen hoogtes", "own heights").into())
        }
    };
    let name = req.name.unwrap_or_else(|| next_name("kantlijn", "Kantlijn"));
    let spec = ctx.layer("OG", "BREUKLIJN", Element::G)?;
    let spec_t = ctx.layer("OG", "BREUKLIJN", Element::T)?;
    let data = feature_data(&name, &source, &pts, line.closed, f);
    let mut out = Out::default();
    out.put(&spec, [poly3(&pts, line.closed, &spec, ctx.k, Some(&data))]);
    let labels = feature_labels(&ctx, &pts, line.closed, &spec_t);
    out.put(&spec_t, labels);
    let handles = out.write_handles(&name)?;
    let h = *handles.first().ok_or(tr("de kantlijn is niet getekend", "the feature line was not drawn"))?;
    bridge::index_add("kantlijn", &name, &[h], true);
    bridge::index_add("kantlijnlabel", &api::handle_text(h), &handles[1..], true);
    let (lo, hi) = zs_m(&pts, f);
    let s = cumulative(&pts);
    let len = (s[s.len() - 1] + if line.closed { plan(pts[pts.len() - 1], pts[0]) } else { 0.0 }) / f;
    api::note("kantlijn", api::handle_text(h));
    api::note("naam", name.clone());
    api::note("punten", pts.len());
    api::note("lengte_m", len);
    api::note("z_min", lo);
    api::note("z_max", hi);
    api::note("hoogtes", pts.iter().map(|p| p[2] / f).collect::<Vec<_>>());
    api::note("hellingen_pct", grades(&pts, line.closed));
    api::note("laag", spec.name.clone());
    Ok(tf!("{name}: {} punten, {} m, {} tot {} m ({source}) — op {}.", "{name}: {} points, {} m, {} to {} m ({source}) — on {}.", pts.len(), dec(len, 1), lvl(lo), lvl(hi), spec.name))
}

/// An edit as the arguments give it: metres and project coordinates.
#[derive(Clone, Debug)]
pub enum EditArgs {
    Level { corner: Option<usize>, near: Option<XY>, z: f64 },
    Raise(f64),
    Insert { at: XY, z: Option<f64> },
    /// Corners from 1; grade in %.
    Grade { from: usize, to: usize, grade: Option<f64> },
}

fn edit_units(ctx: &Ctx, e: &EditArgs) -> Result<Edit, String> {
    let f = ctx.f;
    Ok(match e {
        EditArgs::Level { corner, near, z } => {
            let at = match (corner, near) {
                (Some(i), _) if *i >= 1 => Pick::Corner(i - 1),
                (_, Some(p)) => Pick::Near(ctx.to_drawing(*p)),
                _ => return Err(tr("geef PUNT=<nummer, vanaf 1> of BIJ=x,y", "give PUNT=<number, from 1> or BIJ=x,y").into()),
            };
            Edit::Level { at, z: z * f }
        }
        EditArgs::Raise(dz) => Edit::Raise(dz * f),
        EditArgs::Insert { at, z } => Edit::Insert { at: ctx.to_drawing(*at), z: z.map(|z| z * f) },
        EditArgs::Grade { from, to, grade } => {
            if *from == 0 || *to == 0 {
                return Err(tr("punten tellen vanaf 1", "points count from 1").into());
            }
            Edit::Grade { from: from - 1, to: to - 1, grade: grade.map(|g| g / 100.0) }
        }
    })
}

/// The values of an object data record as key/value pairs.
fn data_pairs(e: &EntityType) -> Vec<(String, String)> {
    let Some(r) = e.common().extended_data.get_record(bridge::XDATA_APP) else { return Vec::new() };
    r.values
        .iter()
        .filter_map(|v| match v {
            ocs_plugin_api::host::XDataValue::String(s) => s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())),
            _ => None,
        })
        .collect()
}

/// `CIV_KANTLIJN_PEIL`, `_VERHOOG`, `_PUNT`, `_HELLING`: change a feature
/// line in place (same handle), with its labels.
pub fn edit_feature(opt: &super::CivielOptions, handle: Option<Handle>, edit: EditArgs) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let src = match handle {
        Some(h) => Source::Handle(h),
        None => Source::Selection,
    };
    let (line, e) = read_source(&ctx, &src, None)?;
    let Some(EntityType::Polyline3D(old)) = e else {
        return Err(tr("geen kantlijn: kies een 3D-polylijn (CIV_KANTLIJN maakt er een)", "not a feature line: choose a 3D polyline (CIV_KANTLIJN makes one)").into());
    };
    let h = old.common.handle;
    let mut pts = line.pts.clone();
    let what = apply(&mut pts, line.closed, &edit_units(&ctx, &edit)?, f)?;
    let mut p = old.clone();
    if pts.len() == p.vertices.len() {
        for (v, q) in p.vertices.iter_mut().zip(&pts) {
            v.position.z = q[2];
        }
    } else {
        // One point inserted: keep the other vertices (and their handles).
        let at = (0..pts.len()).find(|i| *i >= p.vertices.len() || plan(pts[*i], [p.vertices[*i].position.x, p.vertices[*i].position.y, 0.0]) > 1e-9).unwrap_or(pts.len() - 1);
        let mut v = Vertex3DPolyline::new(v3([pts[at][0], pts[at][1]], pts[at][2]));
        if let Some(first) = p.vertices.first() {
            v.layer = first.layer.clone();
        }
        p.vertices.insert(at, v);
        for (v, q) in p.vertices.iter_mut().zip(&pts) {
            v.position.z = q[2];
        }
    }
    let mut ent = EntityType::Polyline3D(p);
    let old_pairs = data_pairs(&ent);
    let name = old_pairs.iter().find(|(k, _)| k == "kantlijn").map(|(_, v)| v.clone()).unwrap_or_else(|| "Kantlijn".into());
    let source = old_pairs.iter().find(|(k, _)| k == "bron").map(|(_, v)| v.clone()).unwrap_or_default();
    let mut rec = feature_data(&name, &source, &pts, line.closed, f);
    for (k, v) in &old_pairs {
        if !["kantlijn", "bron", "punten", "lengte_m", "z_min", "z_max", "hellingen", "bewerkt"].contains(&k.as_str()) {
            rec.add_value(ocs_plugin_api::host::XDataValue::String(format!("{k}={v}")));
        }
    }
    rec.add_value(ocs_plugin_api::host::XDataValue::String(format!("bewerkt={what}")));
    ent.common_mut().extended_data.upsert_record(rec);
    let label = tf!("{name} bewerken", "Edit {name}");
    let new_handle = match bridge::update_entities(&label, vec![ent.clone()]) {
        Ok(_) => h,
        Err(_) => {
            // The host would not take the change in place: draw it anew.
            bridge::remove_entities(&label, &[h])?;
            ent.common_mut().handle = Handle::NULL;
            let hs = bridge::add_entities_handles(&label, vec![ent])?;
            let nh = *hs.first().ok_or(tr("de kantlijn is niet opnieuw getekend", "the feature line was not redrawn"))?;
            bridge::index_add("kantlijn", &name, &[nh], true);
            nh
        }
    };
    // New labels for the new heights.
    let old_labels: Vec<Handle> = bridge::index_read().into_iter().filter(|(k, key, _)| k == "kantlijnlabel" && *key == api::handle_text(h)).flat_map(|(_, _, hs)| hs).collect();
    let _ = bridge::remove_entities(tr("Kantlijnlabels", "Feature line labels"), &old_labels);
    let spec_t = ctx.layer("OG", "BREUKLIJN", Element::T)?;
    let layer = old.common.layer.clone();
    let spec_t = if layer.starts_with(|c: char| "BNTV".contains(c)) {
        LayerSpec { name: format!("{}{}", &layer[..layer.len().saturating_sub(1)], "T"), ..spec_t }
    } else {
        spec_t
    };
    let mut out = Out::default();
    out.put(&spec_t, feature_labels(&ctx, &pts, line.closed, &spec_t));
    let labels = if out.is_empty() { Vec::new() } else { out.write_handles(tr("Kantlijnlabels", "Feature line labels"))? };
    bridge::index_add("kantlijnlabel", &api::handle_text(new_handle), &labels, true);
    let (lo, hi) = zs_m(&pts, f);
    api::note("kantlijn", api::handle_text(new_handle));
    api::note("punten", pts.len());
    api::note("hoogtes", pts.iter().map(|p| p[2] / f).collect::<Vec<_>>());
    api::note("hellingen_pct", grades(&pts, line.closed));
    api::note("z_min", lo);
    api::note("z_max", hi);
    Ok(tf!("{name}: {what}; nu {} punten, {} tot {} m.", "{name}: {what}; now {} points, {} to {} m.", pts.len(), lvl(lo), lvl(hi)))
}

// ── The design surface of all gradings ("bouwrijp") ────────────────────────

const BOUWRIJP: &str = "Bouwrijp";

/// Triangles of a mesh entity (faces fanned from their first corner).
fn mesh_tris(e: &EntityType) -> Vec<[P3; 3]> {
    let EntityType::Mesh(m) = e else { return Vec::new() };
    let p = |i: usize| m.vertices.get(i).map(|v| [v.x, v.y, v.z]);
    let mut out = Vec::new();
    for fc in &m.faces {
        for k in 1..fc.vertices.len().saturating_sub(1) {
            if let (Some(a), Some(b), Some(c)) = (p(fc.vertices[0]), p(fc.vertices[k]), p(fc.vertices[k + 1])) {
                out.push([a, b, c]);
            }
        }
    }
    out
}

/// The combined design surface in the drawing: its triangles, mesh handles
/// and the names of the gradings in it.
/// The design surface's triangles, its mesh handles and the gradings in it.
pub type Bouwrijp = (Vec<Tri>, Vec<Handle>, Vec<String>);

pub fn read_bouwrijp() -> Result<Bouwrijp, String> {
    let hs: Vec<Handle> = bridge::index_read().into_iter().filter(|(k, _, _)| k == "bouwrijp").flat_map(|(_, _, h)| h).collect();
    let mut tris = Vec::new();
    let mut found = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for chunk in hs.chunks(4) {
        for e in bridge::entities(chunk)?.into_iter().flatten() {
            let part = super::data_value(&e, "deel").and_then(|d| Part::from_code(&d)).unwrap_or(Part::Core);
            for n in super::data_value(&e, "gradingen").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
                if !names.iter().any(|x| x == n) {
                    names.push(n.to_string());
                }
            }
            tris.extend(mesh_tris(&e).into_iter().map(|t| (t, part)));
            found.push(e.common().handle);
        }
    }
    Ok((tris, found, names))
}

/// Draw the combined design surface (one mesh per part, status N) in place
/// of the old one, and keep the object index up to date.
fn write_bouwrijp(ctx: &Ctx, tris: &[Tri], names: &[String], old: &[Handle], label: &str) -> Result<usize, String> {
    if !old.is_empty() {
        bridge::remove_entities(label, old)?;
    }
    let spec = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let mut out = Out::default();
    for part in [Part::Core, Part::Fill, Part::Cut] {
        let mut mb = super::model3d::MeshBuilder::default();
        for (t, p) in tris {
            if *p == part {
                mb.tri(t[0], t[1], t[2]);
            }
        }
        if mb.is_empty() {
            continue;
        }
        let d = record(&[("terrein", BOUWRIJP.to_string()), ("grading", "bouwrijp".into()), ("deel", part.code().into()), ("gradingen", names.join(";")), ("driehoeken", tris.len().to_string())]);
        out.put(&spec, mb.finish(&spec.name, None, Some(&d)));
    }
    let handles = out.write_handles(label)?;
    bridge::index_add("bouwrijp", "N", &handles, true);
    let mut status_n: Vec<Handle> = bridge::index_read().into_iter().filter(|(k, key, _)| k == "tin" && key == "N").flat_map(|(_, _, h)| h).filter(|h| !old.contains(h)).collect();
    status_n.extend(&handles);
    bridge::index_add("tin", "N", &status_n, true);
    Ok(handles.len())
}

/// Add a grading's triangles to the design surface (or draw them as a
/// surface of their own); the number of triangles of the result.
fn add_to_design(ctx: &Ctx, name: &str, tris: &[Tri], join: bool, progress: &dyn Fn(String)) -> Result<(usize, usize), String> {
    let tol = 1e-5 * ctx.f;
    if !join {
        let all = conform(tris.to_vec(), tol);
        let spec = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
        let mut mb = super::model3d::MeshBuilder::default();
        for (t, _) in &all {
            mb.tri(t[0], t[1], t[2]);
        }
        let d = record(&[("terrein", name.to_string()), ("grading", name.to_string())]);
        let mut out = Out::default();
        out.put(&spec, mb.finish(&spec.name, None, Some(&d)));
        let hs = out.write_handles(name)?;
        bridge::index_add("tin", "N", &hs, false);
        return Ok((all.len(), 1));
    }
    let (old, handles, mut names) = read_bouwrijp()?;
    progress(tf!("Ontwerpoppervlak samenvoegen ({} + {} driehoeken)…", "Joining the design surface ({} + {} triangles)…", old.len(), tris.len()));
    let all = if old.is_empty() { conform(tris.to_vec(), tol) } else { combine(&old, tris, tol) };
    names.retain(|n| n != name);
    names.push(name.to_string());
    write_bouwrijp(ctx, &all, &names, &handles, BOUWRIJP)?;
    Ok((all.len(), names.len()))
}

/// Daylight (or target) lines of a grading in runs of fill (toe) and cut
/// (crest), skipping rays cut short in inside corners.
fn daylight_runs(rays: &[Ray], closed: bool) -> Vec<(Vec<P3>, Part)> {
    let usable = |r: &Ray| matches!(r.end, End::Daylight | End::Target | End::Reach);
    let mut order: Vec<usize> = (0..rays.len()).collect();
    if closed {
        // Start where a run breaks, so no run is split at the seam.
        if let Some(b) = (0..rays.len()).find(|i| rays[*i].end == End::NoGround || (usable(&rays[*i]) && rays[(*i + rays.len() - 1) % rays.len()].part != rays[*i].part && usable(&rays[(*i + rays.len() - 1) % rays.len()]))) {
            order.rotate_left(b);
        }
    }
    let mut runs: Vec<(Vec<P3>, Part)> = Vec::new();
    let mut cur: Vec<P3> = Vec::new();
    let mut part: Option<Part> = None;
    for &i in &order {
        let r = rays[i];
        if r.end == End::NoGround {
            if cur.len() > 1 {
                runs.push((std::mem::take(&mut cur), part.unwrap()));
            }
            cur.clear();
            part = None;
            continue;
        }
        if !usable(&r) {
            continue;
        }
        if part.is_some_and(|p| p != r.part) {
            cur.push(r.to);
            if cur.len() > 1 {
                runs.push((std::mem::take(&mut cur), part.unwrap()));
            }
            cur = vec![r.to];
        } else {
            cur.push(r.to);
        }
        part = Some(r.part);
    }
    if cur.len() > 1 {
        let p = part.unwrap();
        // One run all round: close it.
        if closed && runs.is_empty() {
            cur.push(cur[0]);
        }
        runs.push((cur, p));
    }
    for r in runs.iter_mut() {
        r.0.dedup_by(|a, b| plan(*a, *b) < 1e-9);
    }
    runs.retain(|r| r.0.len() > 1);
    runs
}

/// The lines of a grading: edge, daylight (toe or crest) and slope hatching
/// (long and short ticks from the top of the slope down). Returns the
/// daylight length in drawing units.
fn draw_grading(ctx: &Ctx, g: &Grading, edge: Option<(&[P3], bool)>, data: &ocs_plugin_api::host::ExtendedDataRecord, out: &mut Out) -> Result<f64, String> {
    let k = ctx.k;
    if let Some((pts, closed)) = edge {
        let spec = ctx.layer("OG", "BREUKLIJN", Element::G)?;
        out.put(&spec, [poly3(pts, closed, &spec, k, Some(data))]);
    }
    let toe = ctx.layer("GW", "TALUD_TEEN", Element::G)?;
    let crest = ctx.layer("GW", "TALUD_KRUIN", Element::G)?;
    let closed = edge.map(|e| e.1).unwrap_or(false);
    let mut length = 0.0;
    for (run, part) in daylight_runs(&g.rays, closed) {
        let l = if part == Part::Cut { &crest } else { &toe };
        length += cumulative(&run).last().copied().unwrap_or(0.0);
        out.put(l, [poly3(&run, false, l, k, Some(data))]);
    }
    // Hatching every 2.5 mm on paper along the top of the slope.
    let long = ctx.layer("GW", "TALUDARCERING_LANG", Element::G)?;
    let short = ctx.layer("GW", "TALUDARCERING_KORT", Element::G)?;
    let gap = 2.5 * k;
    let mut last: Option<Ray> = None;
    let mut n = 0usize;
    for r in &g.rays {
        if plan(r.from, r.to) < 1.5 * k || r.end == End::NoGround {
            continue;
        }
        let top_is_edge = r.to[2] <= r.from[2];
        let (top, bottom) = if top_is_edge { (r.from, r.to) } else { (r.to, r.from) };
        if let Some(p) = last {
            let (pt, _) = if p.to[2] <= p.from[2] { (p.from, p.to) } else { (p.to, p.from) };
            if plan(pt, top) < gap && plan(p.to, r.to) < gap {
                continue;
            }
        }
        last = Some(*r);
        if n.is_multiple_of(2) {
            out.put(&long, [line3(top, bottom, &long)]);
        } else {
            let t = 1.0 / 3.0;
            out.put(&short, [line3(top, [top[0] + (bottom[0] - top[0]) * t, top[1] + (bottom[1] - top[1]) * t, top[2] + (bottom[2] - top[2]) * t], &short)]);
        }
        n += 1;
    }
    Ok(length)
}

/// This grading against the ground alone: (cut, fill) m³, exact on a TIN,
/// on a 0.5 m grid over AHN.
fn grading_volume(ctx: &Ctx, ground: &mut Maaiveld, tris: &[Tri]) -> Option<(f64, f64, bool)> {
    let tin = tin_of(tris)?;
    let f = ctx.f;
    match ground {
        Maaiveld::Tin(t) => {
            let v = terrein::volume_between(t, &tin, f);
            Some((v.cut, v.fill, true))
        }
        // AHN or a point-cloud strip: sampled on a 0.5 m grid.
        Maaiveld::Bron(_) | Maaiveld::Puntenwolk(_) => {
            let w = terrein::earthwork(tin.extent(), 0.5 * f, f, |p| ground.height(p, f).map(|z| z * f), |p| tin.height(p));
            Some((w.cut, w.fill, false))
        }
    }
}

fn bearing_name(b: f64) -> String {
    let names = ["N", "NO", "O", "ZO", "Z", "ZW", "W", "NW"];
    let d = b.to_degrees().rem_euclid(360.0);
    if (d / 45.0 - (d / 45.0).round()).abs() < 1e-6 {
        names[((d / 45.0).round() as usize) % 8].to_string()
    } else {
        format!("{d:.0}°")
    }
}

/// A compass direction: N, NO, O, ZO, Z, ZW, W, NW (also NE/E/SE/S/SW in
/// English) or degrees clockwise from north. Radians back.
pub fn parse_bearing(s: &str) -> Result<f64, String> {
    let t = s.trim().to_ascii_uppercase();
    let d = match t.as_str() {
        "N" => 0.0,
        "NO" | "NE" => 45.0,
        "O" | "E" => 90.0,
        "ZO" | "SE" => 135.0,
        "Z" | "S" => 180.0,
        "ZW" | "SW" => 225.0,
        "W" => 270.0,
        "NW" => 315.0,
        _ => number(&t).map_err(|_| tf!("RICHTING: N, NO, O, ZO, Z, ZW, W, NW of graden vanaf noord, kreeg '{s}'", "RICHTING: N, NE, E, SE, S, SW, W, NW or degrees from north, got '{s}'"))?,
    };
    Ok(d.to_radians())
}

/// A slope ratio "1:3", "3" or "1:2,5": the n of 1:n.
pub fn parse_ratio(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let n = match t.split_once(':') {
        Some((a, b)) => number(b)? / number(a)?.max(1e-9),
        None => number(t)?,
    };
    if n <= 0.0 {
        return Err(tf!("talud {s}: verwacht 1:n met n > 0", "slope {s}: expected 1:n with n > 0"));
    }
    Ok(n)
}

pub struct PlateauRequest {
    pub source: Source,
    /// Level (m) at the reference point.
    pub level: Option<f64>,
    /// The level when none is given and the line has no heights (the
    /// ribbon: the level of the settings).
    pub default_level: Option<f64>,
    /// Fall (%) and the direction it falls to (radians from north).
    pub fall: Option<(f64, f64)>,
    /// Where the level applies (project coordinates); the centroid if none.
    pub reference: Option<XY>,
    /// Cut and fill slopes 1:n, and how far they may reach (m).
    pub cut: f64,
    pub fill: f64,
    pub reach: f64,
    /// Points along the edge every `step` m.
    pub step: f64,
    pub name: Option<String>,
    /// Join the design surface of the other gradings.
    pub join: bool,
}

fn level_text(z: f64) -> String {
    format!("{} m", lvl(z))
}

/// `CIV_PLATEAU`: a plateau at a level (with a fall), sloping to the ground.
pub fn plateau(opt: &super::CivielOptions, req: PlateauRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let (line, e) = read_source(&ctx, &req.source, Some(true))?;
    if !line.closed || line.pts.len() < 3 {
        return Err(tr("een plateau heeft een gesloten omtrek nodig: sluit de polylijn (of geef PUNTEN= met minstens drie punten)", "a plateau needs a closed outline: close the polyline (or give PUNTEN= with at least three points)").into());
    }
    let from_feature = e.as_ref().and_then(|e| super::data_value(e, "kantlijn"));
    let xy: Vec<XY> = line.pts.iter().map(|p| [p[0], p[1]]).collect();
    let (area, centroid) = area_centroid(&xy);
    let reference = req.reference.map(|p| ctx.to_drawing(p)).unwrap_or(centroid);
    let mut edge = line.pts.clone();
    let mut how = String::new();
    let level = req.level.or(if line.has_z { None } else { req.default_level });
    match level {
        Some(level) => {
            let (g, b) = req.fall.unwrap_or((0.0, 0.0));
            let u = [b.sin(), b.cos()];
            for p in edge.iter_mut() {
                p[2] = level * f - g / 100.0 * ((p[0] - reference[0]) * u[0] + (p[1] - reference[1]) * u[1]);
            }
            how = tf!("peil {}", "level {}", level_text(level));
            if g != 0.0 {
                how.push_str(&tf!(", afschot {} % naar {}", ", fall {} % to {}", pct(g), bearing_name(b)));
            }
        }
        None if line.has_z => how.push_str(tr("hoogtes van de kantlijn", "heights of the feature line")),
        None => return Err(tr("geef het peil van het plateau: PEIL=<m NAP>", "give the level of the plateau: PEIL=<m>").into()),
    }
    progress(tr("Maaiveld rond het plateau…", "Ground around the plateau…").into());
    let doc = super::document(&["tin"], &req.source.handle())?;
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let side = if area > 0.0 { 1.0 } else { -1.0 };
    let target = Target::Ground { cut: req.cut, fill: req.fill, reach: req.reach * f };
    progress(tr("Taluds naar het maaiveld…", "Slopes to the ground…").into());
    let g = grade(&edge, true, side, target, Sampling::metres(f, req.step), &mut |p| ground.height(p, f).map(|z| z * f), true)?;
    if g.rays.iter().all(|r| r.end == End::NoGround) {
        return Err(ground.error().unwrap_or_else(|| tf!("geen maaiveld rond het plateau ({})", "no ground around the plateau ({})", ground.label())));
    }
    let name = req.name.unwrap_or_else(|| next_name("grading", "Plateau"));
    let missing = g.rays.iter().filter(|r| matches!(r.end, End::Reach | End::NoGround)).count();
    let data = record(&[
        ("grading", name.clone()),
        ("soort", "plateau".into()),
        ("peil", level.map(|z| format!("{z:.3}")).unwrap_or_default()),
        ("afschot_pct", req.fall.map(|v| format!("{:.3}", v.0)).unwrap_or_default()),
        ("afschot_richting", req.fall.map(|v| bearing_name(v.1)).unwrap_or_default()),
        ("talud_af", format!("1:{}", req.cut)),
        ("talud_op", format!("1:{}", req.fill)),
    ]);
    let mut out = Out::default();
    let daylight = draw_grading(&ctx, &g, from_feature.is_none().then_some((&edge[..], true)), &data, &mut out)?;
    // The level, and an arrow down the fall.
    let spec_t = ctx.layer("GW", "HOOGTE_AFWERKHOOGTE", Element::T)?;
    let zc = edge.iter().map(|p| p[2]).sum::<f64>() / edge.len() as f64;
    let ztext = level.map(|z| z * f).unwrap_or(zc);
    let t = ctx.text([reference[0], reference[1] + 1.5 * k], &format!("{name} {}", lvl(ztext / f)), 2.5, 0.0, &spec_t, Some(&data));
    out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
    if let Some((gr, b)) = req.fall.filter(|v| v.0 != 0.0) {
        let u = [b.sin(), b.cos()];
        let (a0, a1) = ([reference[0], reference[1] - 1.5 * k], [reference[0] + u[0] * 12.0 * k, reference[1] - 1.5 * k + u[1] * 12.0 * k]);
        let side_v = [-u[1], u[0]];
        let back = [a1[0] - u[0] * 2.0 * k, a1[1] - u[1] * 2.0 * k];
        for (p, q) in [(a0, a1), ([back[0] + side_v[0] * k, back[1] + side_v[1] * k], a1), ([back[0] - side_v[0] * k, back[1] - side_v[1] * k], a1)] {
            out.put(&spec_t, [line3([p[0], p[1], ztext], [q[0], q[1], ztext], &spec_t)]);
        }
        let mid = [(a0[0] + a1[0]) / 2.0 + side_v[0] * 1.2 * k, (a0[1] + a1[1]) / 2.0 + side_v[1] * 1.2 * k];
        let t = ctx.text(mid, &format!("{} %", pct(gr)), 1.8, crate::emit::readable(u[1].atan2(u[0])), &spec_t, None);
        out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
    }
    let handles = out.write_handles(&name)?;
    bridge::index_add("grading", &name, &handles, true);
    let vol = grading_volume(&ctx, &mut ground, &g.tris);
    let (tris, gradings) = add_to_design(&ctx, &name, &g.tris, req.join, progress)?;
    ground.note_source();
    let core_area: f64 = g.tris.iter().filter(|t| t.1 == Part::Core).map(|t| tri_area(&t.0).abs()).sum::<f64>() / (f * f);
    let slope_area: f64 = g.tris.iter().filter(|t| t.1 != Part::Core).map(|t| tri_area(&t.0).abs()).sum::<f64>() / (f * f);
    api::note("plateau", name.clone());
    api::note("rand", handles.first().map(|h| api::handle_text(*h)).unwrap_or_default());
    api::note("peil", level);
    api::note("oppervlak_plateau_m2", core_area);
    api::note("oppervlak_talud_m2", slope_area);
    api::note("insteek_teen_m", daylight / f);
    api::note("taludlijnen", g.rays.len());
    api::note("zonder_aansluiting", missing);
    api::note("driehoeken", g.tris.len());
    api::note("ontwerp_driehoeken", tris);
    api::note("gradings_in_ontwerp", gradings);
    if let Some((c, fl, exact)) = vol {
        api::note("afgraven_m3", c);
        api::note("ophogen_m3", fl);
        api::note("volume_exact", exact);
    }
    Ok(tf!(
        "{name}: {how}; plateau {} m², taluds {} m² (1:{} af, 1:{} op), aansluiting {} m{}{} — ontwerpoppervlak {} driehoeken ({} grading(s)).",
        "{name}: {how}; plateau {} m², slopes {} m² (cut 1:{}, fill 1:{}), daylight {} m{}{} — design surface {} triangles ({} grading(s)).",
        m3(core_area),
        m3(slope_area),
        fmt_n(req.cut),
        fmt_n(req.fill),
        dec(daylight / f, 1),
        vol.map(|(c, fl, _)| tf!("; afgraven {} m³, ophogen {} m³", "; cut {} m³, fill {} m³", m3(c), m3(fl))).unwrap_or_default(),
        if missing > 0 { tf!(" ({missing} taludlijnen zonder aansluiting op het maaiveld)", " ({missing} slope lines do not reach the ground)") } else { String::new() },
        tris,
        gradings
    ))
}

fn m3(v: f64) -> String {
    whole(v)
}

/// A number with at most two decimals, trailing zeros dropped.
pub fn fmt_n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if dutch_ui() { s.replace('.', ",") } else { s.to_string() }
}

/// What a slope from an open line runs to (metres).
#[derive(Clone, Copy, Debug)]
pub enum SlopeTo {
    Ground { cut: f64, fill: f64, reach: f64 },
    /// A width at slope 1:n, up or down.
    Width { width: f64, n: f64, up: bool },
    Level { z: f64, n: f64 },
}

pub struct SlopeRequest {
    pub source: Source,
    /// +1 right of the line's direction, −1 left.
    pub side: f64,
    pub to: SlopeTo,
    /// The edge's level when the line has no heights of its own (m); else
    /// its corners take the ground's height.
    pub level: Option<f64>,
    pub step: f64,
    pub name: Option<String>,
    pub join: bool,
}

/// `CIV_TALUD`: a slope from a (feature) line to the ground, over a width or
/// to a level.
pub fn slope(opt: &super::CivielOptions, req: SlopeRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let (line, e) = read_source(&ctx, &req.source, None)?;
    let from_feature = e.as_ref().and_then(|e| super::data_value(e, "kantlijn"));
    let doc = super::document(&["tin"], &req.source.handle())?;
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let mut edge = line.pts.clone();
    let how = if line.has_z {
        tr("hoogtes van de lijn", "heights of the line").to_string()
    } else if let Some(z) = req.level {
        for p in edge.iter_mut() {
            p[2] = z * f;
        }
        tf!("rand op {}", "edge at {}", level_text(z))
    } else {
        edge = drape(&edge, line.closed, 0.0, 0.0, &mut |p| ground.height(p, f).map(|z| z * f)).map_err(|e| ground.error().unwrap_or(e))?;
        tf!("rand op het maaiveld ({})", "edge on the ground ({})", ground.label())
    };
    let (target, what) = match req.to {
        SlopeTo::Ground { cut, fill, reach } => (Target::Ground { cut, fill, reach: reach * f }, tf!("naar het maaiveld (1:{} af, 1:{} op)", "to the ground (cut 1:{}, fill 1:{})", fmt_n(cut), fmt_n(fill))),
        SlopeTo::Width { width, n, up } => (Target::Width { width: width * f, grade: if up { 1.0 } else { -1.0 } / n.max(0.01) }, tf!("{} m breed, 1:{} {}", "{} m wide, 1:{} {}", fmt_n(width), fmt_n(n), if up { tr("omhoog", "up") } else { tr("omlaag", "down") })),
        SlopeTo::Level { z, n } => (Target::Level { z: z * f, n }, tf!("naar peil {} (1:{})", "to level {} (1:{})", level_text(z), fmt_n(n))),
    };
    progress(tr("Talud…", "Slope…").into());
    let g = grade(&edge, line.closed, req.side, target, Sampling::metres(f, req.step), &mut |p| ground.height(p, f).map(|z| z * f), false)?;
    if g.tris.is_empty() {
        return Err(match target {
            Target::Ground { .. } => ground.error().unwrap_or_else(|| tr("het talud vindt geen maaiveld: ligt de lijn op het maaiveld, of buiten het terrein?", "the slope finds no ground: does the line lie on the ground, or outside the terrain?").into()),
            _ => tr("het talud heeft geen breedte: ligt de rand al op dat peil?", "the slope has no width: is the edge already at that level?").into(),
        });
    }
    let name = req.name.unwrap_or_else(|| next_name("grading", "Talud"));
    let data = record(&[
        ("grading", name.clone()),
        ("soort", "talud".into()),
        ("zijde", if req.side > 0.0 { "rechts" } else { "links" }.into()),
        ("naar", what.clone()),
    ]);
    let mut out = Out::default();
    let length = draw_grading(&ctx, &g, from_feature.is_none().then_some((&edge[..], line.closed)), &data, &mut out)?;
    let handles = out.write_handles(&name)?;
    bridge::index_add("grading", &name, &handles, true);
    let vol = grading_volume(&ctx, &mut ground, &g.tris);
    let (tris, gradings) = add_to_design(&ctx, &name, &g.tris, req.join, progress)?;
    ground.note_source();
    let slope_area: f64 = g.tris.iter().map(|t| tri_area(&t.0).abs()).sum::<f64>() / (f * f);
    let missing = g.rays.iter().filter(|r| matches!(r.end, End::Reach | End::NoGround)).count();
    api::note("talud", name.clone());
    api::note("lijnen", handles.iter().map(|h| api::handle_text(*h)).take(3).collect::<Vec<_>>());
    api::note("oppervlak_talud_m2", slope_area);
    api::note("teen_kruin_m", length / f);
    api::note("zonder_aansluiting", missing);
    api::note("driehoeken", g.tris.len());
    api::note("ontwerp_driehoeken", tris);
    if let Some((c, fl, exact)) = vol {
        api::note("afgraven_m3", c);
        api::note("ophogen_m3", fl);
        api::note("volume_exact", exact);
    }
    Ok(tf!(
        "{name}: {how}, {what}, {}; {} m², teen/kruin {} m{}{} — ontwerpoppervlak {tris} driehoeken ({gradings} grading(s)).",
        "{name}: {how}, {what}, {}; {} m², toe/crest {} m{}{} — design surface {tris} triangles ({gradings} grading(s)).",
        if req.side > 0.0 { tr("rechts", "right") } else { tr("links", "left") },
        m3(slope_area),
        dec(length / f, 1),
        vol.map(|(c, fl, _)| tf!("; afgraven {} m³, ophogen {} m³", "; cut {} m³, fill {} m³", m3(c), m3(fl))).unwrap_or_default(),
        if missing > 0 { tf!(" ({missing} taludlijnen zonder aansluiting)", " ({missing} slope lines without daylight)") } else { String::new() },
    ))
}

// ── Exact earthwork between two TINs ────────────────────────────────────────

/// Which surface is the design in a TIN-to-TIN volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesignPick {
    /// The selected faces, else the design surface (N), else the level.
    Auto,
    /// Only the combined grading surface.
    Bouwrijp,
    /// A level (`PEIL=`).
    Level,
}

pub struct VolumeRequest {
    pub design: DesignPick,
    /// Bulking: loose m³ per m³ in situ (the cut factor).
    pub loose: f64,
    /// Compaction: loose m³ needed per m³ of compacted fill ("fill factor").
    pub compaction: f64,
    /// Colour bands of the map (half cut, half fill); 0 = no map.
    pub bands: usize,
    /// Also the grid volume (cell of the settings), for comparison.
    pub compare: bool,
}

/// Colours of the bands from near zero outwards: cut yellow to red, fill
/// cyan to blue (colour indices).
const CUT_ACI: [u8; 5] = [50, 40, 30, 20, 10];
const FILL_ACI: [u8; 5] = [130, 140, 150, 160, 170];

fn band_colour(ramp: &[u8; 5], i: usize, n: usize) -> u8 {
    ramp[if n <= 1 { 0 } else { i * (ramp.len() - 1) / (n - 1) }]
}

/// `CIV_VOLUME BASIS=TIN`: cut and fill between the design and the existing
/// ground from the overlay of both TINs (prismoidal, exact), with bulking
/// and compaction, and a cut/fill map in colour bands with a legend.
pub fn volume_tin(opt: &super::CivielOptions, req: VolumeRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["tin"], &[])?;
    let ground = Tin::of_status(&doc, 'B').ok_or("geen bestaand terrein (B-…-OG-TRIANGULATIE): maak eerst een TIN (CIV_TIN)")?;
    let selected = || -> Option<Tin> {
        let sel: std::collections::HashSet<Handle> = bridge::selection().unwrap_or_default().into_iter().collect();
        let mut memo = crate::nlcs::LayerMemo::default();
        let faces: std::collections::HashSet<Handle> = doc
            .model_space_entities()
            .filter(|e| matches!(e, EntityType::Face3D(_) | EntityType::Mesh(_)) && sel.contains(&e.common().handle))
            .filter(|e| {
                let l = &e.common().layer;
                !(l.starts_with("B-") || memo.other(l).is_some_and(|(s, _)| s == crate::nlcs::Status::B))
            })
            .map(|e| e.common().handle)
            .collect();
        (!faces.is_empty()).then(|| Tin::from_document_with(&doc, &faces)).flatten()
    };
    let level = || (terrein::level_tin(ground.extent(), opt.level * f), tf!("peil {}", "level {}", level_text(opt.level)));
    let (design, what) = match req.design {
        DesignPick::Level => level(),
        DesignPick::Bouwrijp => {
            let (tris, _, names) = read_bouwrijp()?;
            (tin_of(&tris), tf!("bouwrijp ontwerp ({})", "site grading design ({})", names.join(", ")))
        }
        DesignPick::Auto => match selected() {
            Some(t) => (Some(t), tr("ontwerp (selectie)", "design (selection)").to_string()),
            None => match Tin::of_status(&doc, 'N') {
                Some(t) => (Some(t), tr("ontwerpoppervlak (N)", "design surface (N)").to_string()),
                None => level(),
            },
        },
    };
    let design = design.ok_or(tr("geen ontwerpoppervlak gevonden", "no design surface found"))?;
    progress(tf!("TIN tegen TIN: {} × {} driehoeken…", "TIN against TIN: {} × {} triangles…", design.triangles.len(), ground.triangles.len()));
    let v = terrein::volume_between(&ground, &design, f);
    if v.area <= 0.0 {
        return Err(tr("het ontwerp ligt nergens boven het bestaande terrein", "the design lies nowhere over the existing ground").into());
    }
    let (cut_loose, fill_need) = (v.cut * req.loose, v.fill * req.compaction);
    let balance = cut_loose - fill_need;
    let mut out = Out::default();
    let text_l = ctx.layer("GW", "ONTGRAVINGSVAK-WERK", Element::T)?;
    let ext = design.extent();
    let mut band_rows = Vec::new();
    if req.bands >= 2 {
        progress(tr("Kaart in kleurbanden…", "Map in colour bands…").into());
        let mut biggest = 0.0f64;
        terrein::overlay_difference(&ground, &design, |poly, d| {
            for p in poly {
                biggest = biggest.max(d(*p).abs());
            }
        });
        let half = (req.bands / 2).clamp(1, 5);
        let step = super::nice_step((biggest / f / half as f64).max(0.01));
        let flat = 0.001 * f;
        // (from, to) in drawing units, layer, area and volume per band.
        let mut bands: Vec<(f64, f64, LayerSpec, f64, f64, super::model3d::MeshBuilder)> = Vec::new();
        for i in 0..half {
            let (a, b) = (i as f64 * step * f, (i + 1) as f64 * step * f);
            let last = i + 1 == half;
            bands.push((-b - if last { 1e30 } else { 0.0 }, -a.max(flat), tool_layer(&format!("OI-GRONDVERZET-AF-{:02}", i + 1), band_colour(&CUT_ACI, i, half)), 0.0, 0.0, Default::default()));
            bands.push((a.max(flat), b + if last { 1e30 } else { 0.0 }, tool_layer(&format!("OI-GRONDVERZET-OP-{:02}", i + 1), band_colour(&FILL_ACI, i, half)), 0.0, 0.0, Default::default()));
        }
        let eps = 1e-9 * f;
        terrein::overlay_difference(&ground, &design, |poly, d| {
            for (lo, hi, _, area, vol, mesh) in bands.iter_mut() {
                let part = clip_half(&clip_half(poly, eps, |p| d(p) - *lo), eps, |p| *hi - d(p));
                let (a, c) = area_centroid(&part);
                if a <= 0.0 {
                    continue;
                }
                *area += a;
                *vol += a * d(c).abs();
                for j in 1..part.len() - 1 {
                    mesh.tri([part[0][0], part[0][1], 0.0], [part[j][0], part[j][1], 0.0], [part[j + 1][0], part[j + 1][1], 0.0]);
                }
            }
        });
        // Legend to the right: fill bands on top, cut below.
        let (lx, top) = (ext[2] + 10.0 * k, ext[3]);
        let box_h = 5.0 * k;
        let mut order: Vec<usize> = (0..bands.len()).filter(|i| i % 2 == 1).rev().collect();
        order.extend((0..bands.len()).filter(|i| i % 2 == 0));
        for (row, &i) in order.iter().enumerate() {
            let (lo, hi, spec, area, vol, _) = &bands[i];
            let y = top - (row + 1) as f64 * (box_h + 1.5 * k);
            let pts = [[lx, y], [lx + 8.0 * k, y], [lx + 8.0 * k, y + box_h], [lx, y + box_h]];
            let mut sq = super::model3d::MeshBuilder::default();
            sq.quad([pts[0][0], pts[0][1], 0.0], [pts[1][0], pts[1][1], 0.0], [pts[2][0], pts[2][1], 0.0], [pts[3][0], pts[3][1], 0.0]);
            out.put(spec, sq.finish(&spec.name, None, None));
            let open = (hi / f).abs() > 1e20 || (lo / f).abs() > 1e20;
            let lim = half as f64 * step;
            let (a, b) = ((lo / f).clamp(-lim, lim), (hi / f).clamp(-lim, lim));
            let (from, to) = (a.abs().min(b.abs()), a.abs().max(b.abs()));
            let kind = if i % 2 == 0 { tr("afgraven", "cut") } else { tr("ophogen", "fill") };
            let range = if open { format!("> {}", pct(from)) } else { tf!("{} tot {}", "{} to {}", pct(from), pct(to)) };
            let label = format!("{kind} {range} m · {} m² · {} m³", m3(area / (f * f)), m3(vol / (f * f * f)));
            let t = ctx.text([lx + 10.0 * k, y + box_h / 2.0], &label, 1.8, 0.0, spec, None);
            out.put(spec, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
            band_rows.push(serde_json::json!({"soort": if i % 2 == 0 { "afgraven" } else { "ophogen" }, "van_m": from, "tot_m": if open { serde_json::Value::Null } else { to.into() }, "oppervlak_m2": area / (f * f), "volume_m3": vol / (f * f * f), "laag": spec.name}));
        }
        let t = ctx.text([lx, top + 3.0 * k], tr("Ontgraven en ophogen (ontwerp t.o.v. bestaand)", "Cut and fill (design against existing)"), 2.5, 0.0, &text_l, None);
        out.put(&text_l, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        for (_, _, spec, _, _, mesh) in bands {
            if !mesh.is_empty() {
                out.put(&spec, mesh.finish(&spec.name, None, None));
            }
        }
    }
    let grid = req.compare.then(|| {
        let cell = opt.volume_cell.max(0.1) * f;
        terrein::earthwork(ext, cell, f, |p| ground.height(p), |p| design.height(p))
    });
    let mut lines = vec![
        tf!("Grondverzet TIN-TIN — {what} t.o.v. bestaand terrein", "Earthwork TIN to TIN — {what} against the existing ground"),
        tf!("Afgraven {} m³ over {} m² · ophogen {} m³ over {} m²", "Cut {} m³ over {} m² · fill {} m³ over {} m²", m3(v.cut), m3(v.cut_area), m3(v.fill), m3(v.fill_area)),
        tf!("Losfactor {} → {} m³ los · verdichtingsfactor {} → {} m³ nodig", "Bulking factor {} → {} m³ loose · compaction factor {} → {} m³ needed", fmt_n(req.loose), m3(cut_loose), fmt_n(req.compaction), m3(fill_need)),
        tf!("Balans {} {} m³ (los)", "Balance: {} of {} m³ (loose)", if balance >= 0.0 { tr("overschot", "surplus") } else { tr("tekort", "shortage") }, m3(balance.abs())),
    ];
    if let Some(w) = grid {
        lines.push(tf!("Ter vergelijking raster {} m: afgraven {} m³, ophogen {} m³", "For comparison, grid of {} m: cut {} m³, fill {} m³", fmt_n(opt.volume_cell), m3(w.cut), m3(w.fill)));
    }
    for (i, l) in lines.iter().enumerate() {
        let t = ctx.text([ext[0], ext[3] + (6.0 + (lines.len() - 1 - i) as f64 * 4.0) * k], l, if i == 0 { 2.5 } else { 1.8 }, 0.0, &text_l, None);
        out.put(&text_l, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    let n = out.write(tr("Grondverzet TIN-TIN", "Earthwork TIN to TIN"))?;
    api::note("methode", "TIN-TIN (overlay, prismoïde)");
    api::note("vergelijking", what.clone());
    api::note("afgraven_m3", v.cut);
    api::note("ophogen_m3", v.fill);
    api::note("afgraven_m2", v.cut_area);
    api::note("ophogen_m2", v.fill_area);
    api::note("overlap_m2", v.area);
    api::note("ontwerp_m2", v.design_area);
    api::note("losfactor", req.loose);
    api::note("verdichtingsfactor", req.compaction);
    api::note("afgraven_los_m3", cut_loose);
    api::note("ophogen_nodig_m3", fill_need);
    api::note("balans_m3", balance);
    api::note("banden", band_rows);
    if let Some(w) = grid {
        api::note("raster_cel_m", opt.volume_cell);
        api::note("raster_afgraven_m3", w.cut);
        api::note("raster_ophogen_m3", w.fill);
    }
    Ok(tf!(
        "{}: afgraven {} m³, ophogen {} m³; met los- en verdichtingsfactor {} {} m³ — {n} elementen.",
        "{}: cut {} m³, fill {} m³; with bulking and compaction a {} of {} m³ — {n} entities.",
        lines[0],
        m3(v.cut),
        m3(v.fill),
        if balance >= 0.0 { tr("overschot", "surplus") } else { tr("tekort", "shortage") },
        m3(balance.abs())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(z: f64, lo: f64, hi: f64) -> Tin {
        let mut pts = Vec::new();
        for i in 0..=10 {
            for j in 0..=10 {
                pts.push([lo + (hi - lo) * i as f64 / 10.0, lo + (hi - lo) * j as f64 / 10.0, z]);
            }
        }
        Tin::build(&pts, 0.0).unwrap()
    }

    fn square(x0: f64, y0: f64, w: f64, z: f64) -> Vec<P3> {
        vec![[x0, y0, z], [x0 + w, y0, z], [x0 + w, y0 + w, z], [x0, y0 + w, z]]
    }

    #[test]
    fn heights_from_grades_and_levels() {
        let pts = vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 50.0, 0.0], [100.0, 150.0, 0.0]];
        let z = by_grades(&pts, 2.0, &[1.0, -2.0]);
        assert_eq!(z, vec![2.0, 3.0, 2.0, 0.0]);
        let lv = parse_levels("1:2;4:4,5", 4).unwrap();
        assert_eq!(lv, vec![Some(2.0), None, None, Some(4.5)]);
        let z = by_levels(&pts, &lv).unwrap();
        // 250 m from 2.0 to 4.5: 1 cm per metre.
        assert!((z[1] - 3.0).abs() < 1e-12 && (z[2] - 3.5).abs() < 1e-12);
        assert_eq!(parse_levels("1;;2,5", 4).unwrap(), vec![Some(1.0), None, Some(2.5), None]);
        assert!(parse_levels("5:1", 4).is_err());
        let g = grades(&[[0.0, 0.0, 1.0], [50.0, 0.0, 2.0], [50.0, 50.0, 1.5]], true);
        assert!((g[0] - 2.0).abs() < 1e-12 && (g[1] + 1.0).abs() < 1e-12);
        assert!((parse_ratio("1:2,5").unwrap() - 2.5).abs() < 1e-12 && parse_ratio("3").unwrap() == 3.0);
        assert!((parse_bearing("ZW").unwrap().to_degrees() - 225.0).abs() < 1e-9);
    }

    #[test]
    fn edits_of_a_feature_line() {
        let mut pts = vec![[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [20.0, 0.0, 1.0], [40.0, 0.0, 1.0]];
        apply(&mut pts, false, &Edit::Level { at: Pick::Near([19.0, 1.0]), z: 2.0 }, 1.0).unwrap();
        assert_eq!(pts[2][2], 2.0);
        apply(&mut pts, false, &Edit::Raise(-0.5), 1.0).unwrap();
        assert_eq!(pts.iter().map(|p| p[2]).collect::<Vec<_>>(), vec![0.5, 0.5, 1.5, 0.5]);
        // A point between corners 3 and 4, interpolated.
        apply(&mut pts, false, &Edit::Insert { at: [30.0, 3.0], z: None }, 1.0).unwrap();
        assert_eq!(pts.len(), 5);
        assert!((pts[3][2] - 1.0).abs() < 1e-12 && pts[3][0] == 30.0);
        // 2 % from corner 1 to corner 5 (40 m): +0.8 m at the end.
        apply(&mut pts, false, &Edit::Grade { from: 0, to: 4, grade: Some(0.02) }, 1.0).unwrap();
        assert!((pts[4][2] - 1.3).abs() < 1e-12 && (pts[2][2] - 0.9).abs() < 1e-12);
        // Straight between corners 2 and 4.
        pts[3][2] = 5.0;
        apply(&mut pts, false, &Edit::Grade { from: 1, to: 4, grade: None }, 1.0).unwrap();
        assert!((pts[2][2] - (0.7 + 0.6 * 10.0 / 30.0)).abs() < 1e-12);
        assert!(apply(&mut pts, false, &Edit::Grade { from: 3, to: 1, grade: None }, 1.0).is_err());
    }

    #[test]
    fn draping_keeps_grade_breaks_only() {
        // Ground z = |x − 50| / 10: one break at x = 50.
        let mut pts = Vec::new();
        for i in 0..=10 {
            for j in 0..=2 {
                let x = i as f64 * 10.0;
                pts.push([x, j as f64 * 10.0, (x - 50.0).abs() / 10.0]);
            }
        }
        let t = Tin::build(&pts, 0.0).unwrap();
        let line = vec![[0.0, 10.0, 0.0], [100.0, 10.0, 0.0]];
        let d = drape(&line, false, 1.0, 0.001, &mut |p| t.height(p)).unwrap();
        assert_eq!(d.len(), 3, "{d:?}");
        assert!((d[1][0] - 50.0).abs() < 1e-9 && d[1][2].abs() < 1e-9 && (d[2][2] - 5.0).abs() < 1e-9);
        let corners = drape(&line, false, 0.0, 0.0, &mut |p| t.height(p)).unwrap();
        assert_eq!(corners.len(), 2);
    }

    #[test]
    fn a_square_plateau_in_fill() {
        // 20 × 20 m at +2 on flat ground at 0, fill slopes 1:2: the toe lies
        // 4 m out all round. Volume: plateau 800, sides 4 × 80, corners a
        // cone of radius 4 m and height 2 m (a 36-sided pyramid here).
        let ground = flat(0.0, -20.0, 40.0);
        let edge = square(0.0, 0.0, 20.0, 2.0);
        let s = Sampling { step: 1.0, fan: 10f64.to_radians(), res: 0.25, tol: 1e-6 };
        let g = grade(&edge, true, 1.0, Target::Ground { cut: 2.0, fill: 2.0, reach: 30.0 }, s, &mut |p| ground.height(p), true).unwrap();
        assert!(g.rays.iter().all(|r| r.end == End::Daylight && r.part == Part::Fill));
        for r in &g.rays {
            assert!((plan(r.from, r.to) - 4.0).abs() < 1e-6 && r.to[2].abs() < 1e-6, "{r:?}");
        }
        let cone = 36.0 / 2.0 * 16.0 * (2.0 * std::f64::consts::PI / 36.0).sin() * 2.0 / 3.0;
        let tin = tin_of(&g.tris).unwrap();
        let v = terrein::volume_between(&ground, &tin, 1.0);
        assert!((v.fill - (800.0 + 320.0 + cone)).abs() < 1e-6, "{} vs {}", v.fill, 800.0 + 320.0 + cone);
        // The same plateau listed clockwise grades outwards too.
        let mut cw = edge.clone();
        cw.reverse();
        let g2 = grade(&cw, true, -1.0, Target::Ground { cut: 2.0, fill: 2.0, reach: 30.0 }, s, &mut |p| ground.height(p), true).unwrap();
        let v2 = terrein::volume_between(&ground, &tin_of(&g2.tris).unwrap(), 1.0);
        assert!((v2.fill - v.fill).abs() < 1e-6);
        // In cut (ground at +3): slopes 1:1 up to the ground, 1 m out.
        let high = flat(3.0, -20.0, 40.0);
        let g3 = grade(&edge, true, 1.0, Target::Ground { cut: 1.0, fill: 2.0, reach: 30.0 }, s, &mut |p| high.height(p), true).unwrap();
        assert!(g3.rays.iter().all(|r| r.part == Part::Cut && (plan(r.from, r.to) - 1.0).abs() < 1e-6));
    }

    #[test]
    fn an_inside_corner_needs_no_cleanup() {
        // An L-shaped plateau at +2 on flat ground, fill 1:2: the slope is the
        // distance to the outline, so the TIN must match z = 2 − d / 2 and
        // its volume a fine numerical integral of that.
        let ground = flat(0.0, -20.0, 60.0);
        let edge = vec![[0.0, 0.0, 2.0], [30.0, 0.0, 2.0], [30.0, 10.0, 2.0], [10.0, 10.0, 2.0], [10.0, 30.0, 2.0], [0.0, 30.0, 2.0]];
        let s = Sampling { step: 0.5, fan: 5f64.to_radians(), res: 0.1, tol: 1e-6 };
        let g = grade(&edge, true, 1.0, Target::Ground { cut: 2.0, fill: 2.0, reach: 30.0 }, s, &mut |p| ground.height(p), true).unwrap();
        let tin = tin_of(&conform(g.tris.clone(), 1e-6)).unwrap();
        let dist = |p: XY| -> f64 {
            let inside = {
                let mut c = false;
                for i in 0..edge.len() {
                    let (a, b) = (edge[i], edge[(i + 1) % edge.len()]);
                    if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                        c = !c;
                    }
                }
                c
            };
            if inside { 0.0 } else { (0..edge.len()).map(|i| seg_dist(p, edge[i], edge[(i + 1) % edge.len()])).fold(f64::MAX, f64::min) }
        };
        let expect = |p: XY| (2.0 - dist(p) / 2.0).max(0.0);
        // Heights round the inside corner (10, 10).
        for p in [[12.0, 12.0], [11.0, 13.5], [14.0, 11.0], [12.5, 12.5], [11.5, 11.0]] {
            let z = tin.height(p).unwrap();
            assert!((z - expect(p)).abs() < 0.02, "{p:?}: {z} vs {}", expect(p));
        }
        let mut numeric = 0.0;
        let h = 0.05;
        let mut y = -6.0 + h / 2.0;
        while y < 36.0 {
            let mut x = -6.0 + h / 2.0;
            while x < 36.0 {
                numeric += expect([x, y]) * h * h;
                x += h;
            }
            y += h;
        }
        let v = terrein::volume_between(&ground, &tin, 1.0);
        assert!((v.fill - numeric).abs() / numeric < 0.002, "{} vs {numeric}", v.fill);
    }

    #[test]
    fn slopes_to_a_width_and_a_level() {
        let ground = flat(0.0, -20.0, 60.0);
        let line = vec![[0.0, 0.0, 1.0], [20.0, 0.0, 1.0]];
        let s = Sampling { step: 1.0, fan: 10f64.to_radians(), res: 0.25, tol: 1e-6 };
        // 3 m wide, down 1:3, to the right (−y).
        let g = grade(&line, false, 1.0, Target::Width { width: 3.0, grade: -1.0 / 3.0 }, s, &mut |p| ground.height(p), false).unwrap();
        assert!(g.rays.iter().all(|r| (r.to[1] + 3.0).abs() < 1e-9 && (r.to[2] - 0.0).abs() < 1e-9 && r.part == Part::Fill));
        // Up to +2.5 at 1:2, to the left: 3 m out.
        let g = grade(&line, false, -1.0, Target::Level { z: 2.5, n: 2.0 }, s, &mut |p| ground.height(p), false).unwrap();
        assert!(g.rays.iter().all(|r| (r.to[1] - 3.0).abs() < 1e-9 && (r.to[2] - 2.5).abs() < 1e-9 && r.part == Part::Cut));
        let area: f64 = g.tris.iter().map(|t| tri_area(&t.0).abs()).sum();
        assert!((area - 60.0).abs() < 1e-9);
    }

    fn union_area(a: &Tin, b: &Tin) -> f64 {
        let mut both = 0.0;
        terrein::overlay(a, b, |p, _, _| both += area_centroid(p).0);
        let tot = |t: &Tin| (0..t.triangles.len()).map(|i| area_centroid(&xy_of(&t.corners(i))).0.abs()).sum::<f64>();
        tot(a) + tot(b) - both
    }

    #[test]
    fn two_plateaus_make_one_surface() {
        // Plateau A at +2 and plateau B at +3, 6 m apart on ground at 0,
        // fill 1:2: B's slope runs over A's plateau where it is higher.
        let ground = flat(0.0, -30.0, 80.0);
        let s = Sampling { step: 1.0, fan: 10f64.to_radians(), res: 0.25, tol: 1e-6 };
        let t = Target::Ground { cut: 2.0, fill: 2.0, reach: 30.0 };
        let a = grade(&square(0.0, 0.0, 20.0, 2.0), true, 1.0, t, s, &mut |p| ground.height(p), true).unwrap().tris;
        let b = grade(&square(26.0, 0.0, 20.0, 3.0), true, 1.0, t, s, &mut |p| ground.height(p), true).unwrap().tris;
        let all = combine(&conform(a.clone(), 1e-6), &b, 1e-6);
        let tin = tin_of(&all).unwrap();
        // On A, far from B: +2. On B: +3. Between: the higher slope.
        assert!((tin.height([5.0, 10.0]).unwrap() - 2.0).abs() < 1e-9);
        assert!((tin.height([30.0, 10.0]).unwrap() - 3.0).abs() < 1e-9);
        // At x = 19: A's plateau (2.0) vs B's slope (3 − 7 / 2 < 0, not there).
        assert!((tin.height([19.0, 10.0]).unwrap() - 2.0).abs() < 1e-9);
        // At x = 23: A's slope 2 − 3/2 = 0.5, B's slope 3 − 3/2 = 1.5: the highest.
        assert!((tin.height([23.0, 10.0]).unwrap() - 1.5).abs() < 1e-9);
        // No overlaps: the faces cover the union of both footprints once.
        let (ta, tb) = (tin_of(&a).unwrap(), tin_of(&b).unwrap());
        let covered: f64 = all.iter().map(|t| tri_area(&t.0).abs()).sum();
        assert!((covered - union_area(&ta, &tb)).abs() < 1e-6, "{covered} vs {}", union_area(&ta, &tb));
        // No T-junctions: no corner lies inside another face's edge.
        let mut corners: Vec<XY> = all.iter().flat_map(|t| xy_of(&t.0)).collect();
        corners.sort_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])));
        corners.dedup_by(|p, q| (p[0] - q[0]).abs() < 1e-7 && (p[1] - q[1]).abs() < 1e-7);
        for (t, _) in all.iter().take(400) {
            for e in 0..3 {
                let (p, q) = (t[e], t[(e + 1) % 3]);
                for c in &corners {
                    let d = seg_dist(*c, p, q);
                    let at_end = (c[0] - p[0]).hypot(c[1] - p[1]) < 1e-5 || (c[0] - q[0]).hypot(c[1] - q[1]) < 1e-5;
                    assert!(d > 1e-6 || at_end, "corner {c:?} on edge {p:?}–{q:?}");
                }
            }
        }
    }

    #[test]
    fn combination_rules() {
        assert_eq!(rule(Part::Core, Part::Core), Rule::Newest);
        assert_eq!(rule(Part::Fill, Part::Core), Rule::Highest);
        assert_eq!(rule(Part::Core, Part::Fill), Rule::Highest);
        assert_eq!(rule(Part::Cut, Part::Cut), Rule::Lowest);
        assert_eq!(rule(Part::Core, Part::Cut), Rule::Lowest);
        assert_eq!(rule(Part::Fill, Part::Cut), Rule::Newest);
    }
}
