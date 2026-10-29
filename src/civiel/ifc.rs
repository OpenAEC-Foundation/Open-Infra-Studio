//! IFC 4.3 export (IFC4X3_ADD2, ISO 16739-1:2024) for open BIM: the
//! project with SI units and its georeferencing (IfcProjectedCRS and
//! IfcMapConversion), the site, the alignments after the IFC 4.3 alignment
//! concept templates (business logic nested in IfcAlignmentHorizontal and
//! IfcAlignmentVertical, geometry as IfcCompositeCurve and IfcGradientCurve
//! of IfcCurveSegments, stationing by an IfcReferent), the terrain as
//! IfcGeographicElement TERRAIN with an IfcTriangulatedIrregularNetwork,
//! the corridors as IfcRoad with pavement, berms, slopes and earthworks,
//! and the 3D BAG buildings. Written with the small STEP writer in `step.rs`.
//!
//! Coordinates are metres relative to the site's base point; the map
//! conversion adds the base point back (Eastings, Northings) to reach the
//! site's CRS. Heights are metres in the vertical datum (NAP).
//!
//! The mapping of alignment segments to IfcCurveSegments follows the
//! ifcopenshell alignment API (IfcOpenShell, LGPL-3.0-or-later,
//! `ifcopenshell/api/alignment/_map_alignment_*_segment.py`): parent curves
//! at the origin, the segment placed at its start point and direction.

use std::collections::HashMap;

use super::as_profiel::{Alignment, Pvi, Seg};
use super::step::{self, refs, Id, Step, V};
use super::terrein::P3;
use super::uitwisseling::{Corridor, LineKind, Model, Surface};
use crate::gis::crs::XY;
use crate::lang::tr;

pub const SCHEMA: &str = "IFC4X3_ADD2";

// ── Pure geometry: alignment segments, earthworks ──────────────────────────

/// The parent curve of a horizontal curve segment.
#[derive(Clone, Debug, PartialEq)]
pub enum Parent {
    Line,
    /// A circle of this radius around the origin.
    Circle { r: f64 },
    /// An IfcClothoid with constant `a`; the segment starts `start` along it.
    #[cfg_attr(not(test), allow(dead_code))]
    Clothoid { a: f64, start: f64 },
}

/// One horizontal segment: the design parameters (IfcAlignmentHorizontalSegment)
/// and what its geometry (IfcCurveSegment) needs. Points are local (m).
#[derive(Clone, Debug, PartialEq)]
pub struct HSeg {
    pub kind: &'static str,
    pub start: XY,
    /// Direction at the start (radians, counter-clockwise from the x axis).
    pub dir: f64,
    /// Signed radii (left/counter-clockwise > 0; 0 = straight).
    pub r0: f64,
    pub r1: f64,
    pub length: f64,
    pub parent: Parent,
    /// The curve segment's SegmentLength: negative runs clockwise on a circle.
    pub curve_length: f64,
    pub end: XY,
    pub end_dir: f64,
}

/// The clothoid parent curve of a spiral from radius `r0` to `r1` (signed,
/// 0 = straight) over `len`: its constant A and where the segment starts on
/// it. After ifcopenshell's `_map_clothoid` (IfcOpenShell, LGPL-3.0-or-later).
#[cfg_attr(not(test), allow(dead_code))]
pub fn clothoid_parent(r0: f64, r1: f64, len: f64) -> Parent {
    let f = (if r1 == 0.0 { 0.0 } else { len / r1 }) - (if r0 == 0.0 { 0.0 } else { len / r0 });
    let a = len / f.abs().sqrt() * f.signum();
    let start = if (r0.abs() < r1.abs() && r0 != 0.0) || r1 == 0.0 {
        -len - if r1 != 0.0 { len * r0 / (r1 - r0) } else { 0.0 }
    } else if r0 != 0.0 {
        len * r1 / (r0 - r1)
    } else {
        0.0
    };
    Parent::Clothoid { a, start }
}

/// One piece of an alignment as an IFC horizontal segment (`base` is the
/// local origin in project coordinates).
///
/// `Seg::Spiral` (stream as-spoor, a clothoid) becomes: `kind`
/// "CLOTHOID"; `start` and `dir` its start point and direction; `r0`, `r1`
/// its signed start and end radii (left > 0, 0 for a straight end, which is
/// IfcAlignmentHorizontalSegment's StartRadiusOfCurvature and
/// EndRadiusOfCurvature); `length` and `curve_length` its length
/// (SegmentLength); `parent: clothoid_parent(r0, r1, length)`, which writes
/// the IfcClothoid parent curve; `end` and `end_dir` from `Seg::at`.
pub fn horizontal_segment(seg: &Seg, base: XY) -> HSeg {
    let local = |p: XY| [p[0] - base[0], p[1] - base[1]];
    let (p0, d0) = seg.at(0.0);
    let (p1, d1) = seg.at(seg.length());
    match seg {
        Seg::Line { .. } => HSeg { kind: "LINE", start: local(p0), dir: d0, r0: 0.0, r1: 0.0, length: seg.length(), parent: Parent::Line, curve_length: seg.length(), end: local(p1), end_dir: d1 },
        Seg::Arc { r, sweep, .. } => {
            let signed = r * sweep.signum();
            HSeg {
                kind: "CIRCULARARC",
                start: local(p0),
                dir: d0,
                r0: signed,
                r1: signed,
                length: seg.length(),
                parent: Parent::Circle { r: *r },
                curve_length: seg.length() * sweep.signum(),
                end: local(p1),
                end_dir: d1,
            }
        }
    }
}

/// One vertical segment (IfcAlignmentVerticalSegment): distance along the
/// horizontal alignment (m from its start), horizontal length, height and
/// gradients at both ends.
#[derive(Clone, Debug, PartialEq)]
pub struct VSeg {
    pub kind: &'static str,
    pub s0: f64,
    pub length: f64,
    pub z0: f64,
    pub g0: f64,
    pub g1: f64,
}

impl VSeg {
    pub fn end(&self) -> (f64, f64) {
        (self.s0 + self.length, self.z0 + (self.g0 + self.g1) / 2.0 * self.length)
    }

    /// Curvature at the start and end in the (distance, height) plane.
    fn curvature(&self) -> (f64, f64) {
        if self.kind == "PARABOLICARC" && self.length > 0.0 {
            let c2 = (self.g1 - self.g0) / self.length;
            (c2 / (1.0 + self.g0 * self.g0).powf(1.5), c2 / (1.0 + self.g1 * self.g1).powf(1.5))
        } else {
            (0.0, 0.0)
        }
    }
}

/// The design profile as constant grades and parabolic vertical curves.
pub fn vertical_segments(pvis: &[Pvi]) -> Vec<VSeg> {
    let mut out = Vec::new();
    if pvis.len() < 2 {
        return out;
    }
    let grade = |a: &Pvi, b: &Pvi| (b.z - a.z) / (b.s - a.s).max(1e-9);
    // From the end of the previous curve (or the first PVI).
    let (mut s, mut z) = (pvis[0].s, pvis[0].z);
    for j in 1..pvis.len() {
        let p = &pvis[j];
        let g_in = grade(&pvis[j - 1], p);
        let curve = j + 1 < pvis.len() && p.l > 0.0;
        let bvc = if curve { p.s - p.l / 2.0 } else { p.s };
        if bvc - s > 1e-6 {
            out.push(VSeg { kind: "CONSTANTGRADIENT", s0: s, length: bvc - s, z0: z, g0: g_in, g1: g_in });
        }
        if curve {
            let g_out = grade(p, &pvis[j + 1]);
            let z_bvc = p.z - g_in * p.l / 2.0;
            out.push(VSeg { kind: "PARABOLICARC", s0: bvc, length: p.l, z0: z_bvc, g0: g_in, g1: g_out });
            s = p.s + p.l / 2.0;
            z = p.z + g_out * p.l / 2.0;
        } else {
            s = p.s;
            z = p.z;
        }
    }
    out
}

/// Arc length of y = A + Bx + Cx² over [0, L] (closed form, as ifcopenshell's
/// `_polynomial_length`).
fn parabola_length(b: f64, c: f64, l: f64) -> f64 {
    if c.abs() < 1e-12 {
        return l * (1.0 + b * b).sqrt();
    }
    let f = |x: f64| {
        let d = b + 2.0 * c * x;
        let q = (1.0 + d * d).sqrt();
        (d * q + (d + q).ln()) / (4.0 * c)
    };
    f(l) - f(0.0)
}

/// How a curve segment continues into the next: position, direction and
/// curvature at the joint.
fn transition(end: (XY, f64, f64), next: (XY, f64, f64)) -> &'static str {
    let same_pos = (end.0[0] - next.0[0]).hypot(end.0[1] - next.0[1]) < 1e-3;
    let dd = ((next.1 - end.1 + std::f64::consts::PI).rem_euclid(2.0 * std::f64::consts::PI) - std::f64::consts::PI).abs();
    if !same_pos {
        "DISCONTINUOUS"
    } else if dd > 1e-5 {
        "CONTINUOUS"
    } else if (end.2 - next.2).abs() > 1e-6 {
        "CONTSAMEGRADIENT"
    } else {
        "CONTSAMEGRADIENTSAMECURVATURE"
    }
}

fn curvature(r: f64) -> f64 {
    if r == 0.0 { 0.0 } else { 1.0 / r }
}

/// A triangulated body: points, counter-clockwise faces seen from outside.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub points: Vec<P3>,
    pub triangles: Vec<[usize; 3]>,
    /// Every edge used by exactly two faces, in opposite directions.
    pub closed: bool,
    /// Enclosed volume (m³), when closed.
    pub volume: f64,
}

#[derive(Default)]
struct Points {
    points: Vec<P3>,
    index: HashMap<(i64, i64, i64), usize>,
}

impl Points {
    fn at(&mut self, p: P3) -> usize {
        let key = ((p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64, (p[2] * 1e4).round() as i64);
        *self.index.entry(key).or_insert_with(|| {
            self.points.push(p);
            self.points.len() - 1
        })
    }
}

fn finish(points: Vec<P3>, triangles: Vec<[usize; 3]>) -> Body {
    let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
    for t in &triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    let closed = !triangles.is_empty() && directed.iter().all(|(&(a, b), &n)| n == 1 && directed.get(&(b, a)) == Some(&1));
    let volume = if closed {
        let o = points[triangles[0][0]];
        let r = |p: P3| [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| r(points[i]));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
            })
            .sum()
    } else {
        0.0
    };
    Body { points, triangles, closed, volume }
}

/// The earth between a design surface and the ground under it: where the
/// design lies above the ground (`fill`) or below it (cut). Top and bottom
/// are the two surfaces cut at the line where they meet (there the body has
/// no thickness); along the open border of the design surface walls close
/// it. `ground` is the height under each design point (None: no ground).
pub fn earth_body(points: &[P3], ground: &[Option<f64>], triangles: &[[usize; 3]], fill: bool) -> Body {
    // Signed thickness, top and bottom height at each design point.
    let sign = if fill { 1.0 } else { -1.0 };
    let info = |i: usize| -> (XY, f64, f64, f64) {
        let p = points[i];
        let g = ground.get(i).copied().flatten().unwrap_or(p[2]);
        let (top, bottom) = if fill { (p[2], g) } else { (g, p[2]) };
        ([p[0], p[1]], sign * (p[2] - g), top, bottom)
    };
    // Edges on the border of the design surface.
    let mut uses: HashMap<(usize, usize), usize> = HashMap::new();
    for t in triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let eps = 1e-6;
    let mut pts = Points::default();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let push = |t: [usize; 3], tris: &mut Vec<[usize; 3]>| {
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            tris.push(t);
        }
    };
    // A vertex of the clipped polygon: plan point, top and bottom height.
    type Vx = (XY, f64, f64);
    let cross = |a: &(XY, f64, f64, f64), b: &(XY, f64, f64, f64)| -> Vx {
        let t = a.1 / (a.1 - b.1);
        let xy = [a.0[0] + (b.0[0] - a.0[0]) * t, a.0[1] + (b.0[1] - a.0[1]) * t];
        let z = a.2 + (b.2 - a.2) * t;
        (xy, z, z)
    };
    for t in triangles {
        let v = t.map(info);
        if v.iter().all(|x| x.1 <= eps) {
            continue;
        }
        // Sutherland–Hodgman against thickness > 0.
        let mut poly: Vec<Vx> = Vec::with_capacity(4);
        for k in 0..3 {
            let (a, b) = (&v[k], &v[(k + 1) % 3]);
            if a.1 > eps {
                poly.push((a.0, a.2, a.3));
            }
            if (a.1 > eps) != (b.1 > eps) {
                poly.push(cross(a, b));
            }
        }
        let top: Vec<usize> = poly.iter().map(|p| pts.at([p.0[0], p.0[1], p.1])).collect();
        let bottom: Vec<usize> = poly.iter().map(|p| pts.at([p.0[0], p.0[1], p.2])).collect();
        for k in 1..poly.len().saturating_sub(1) {
            push([top[0], top[k], top[k + 1]], &mut tris);
            push([bottom[0], bottom[k + 1], bottom[k]], &mut tris);
        }
        // Walls on the border: outward is to the right of a counter-clockwise edge.
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            if uses[&(i.min(j), i.max(j))] != 1 {
                continue;
            }
            let (a, b) = (&v[k], &v[(k + 1) % 3]);
            let (u, w): (Vx, Vx) = match (a.1 > eps, b.1 > eps) {
                (true, true) => ((a.0, a.2, a.3), (b.0, b.2, b.3)),
                (true, false) => ((a.0, a.2, a.3), cross(a, b)),
                (false, true) => (cross(a, b), (b.0, b.2, b.3)),
                (false, false) => continue,
            };
            let (tu, bu) = (pts.at([u.0[0], u.0[1], u.1]), pts.at([u.0[0], u.0[1], u.2]));
            let (tw, bw) = (pts.at([w.0[0], w.0[1], w.1]), pts.at([w.0[0], w.0[1], w.2]));
            push([tu, bu, bw], &mut tris);
            push([tu, bw, tw], &mut tris);
        }
    }
    finish(pts.points, tris)
}

/// Which part of a corridor a design triangle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Pavement,
    Shoulder,
    Crest,
    Slope,
}

/// Each triangle of the corridor's design surface by the offset of its
/// centre from the axis: lanes up to the pavement edge, then the shoulder
/// (or a dike's crest), then the slopes.
pub fn classify(c: &Corridor, s: &Surface) -> Vec<Part> {
    let al = Alignment { name: c.axis.clone(), start: 0.0, segs: c.axis_segs.clone() };
    let (lane, berm, crest) = (c.edge(LineKind::Pavement), c.edge(LineKind::Shoulder), c.edge(LineKind::Crest));
    s.triangles
        .iter()
        .map(|t| {
            if al.segs.is_empty() {
                return Part::Slope;
            }
            let [a, b, cc] = t.map(|i| s.points[i]);
            let m = [(a[0] + b[0] + cc[0]) / 3.0, (a[1] + b[1] + cc[1]) / 3.0];
            let o = al.station_offset(m).1.abs();
            let within = |e: Option<f64>| e.is_some_and(|e| o <= e + 0.01);
            if within(lane) {
                Part::Pavement
            } else if within(berm) {
                Part::Shoulder
            } else if within(crest) {
                Part::Crest
            } else {
                Part::Slope
            }
        })
        .collect()
}

/// The triangles of `s` of one part, with their own point list.
fn part_mesh(s: &Surface, parts: &[Part], want: &[Part]) -> (Vec<P3>, Vec<[usize; 3]>) {
    let mut pts = Points::default();
    let mut tris = Vec::new();
    for (t, p) in s.triangles.iter().zip(parts) {
        if want.contains(p) {
            tris.push(t.map(|i| pts.at(s.points[i])));
        }
    }
    (pts.points, tris)
}

// ── Writing ─────────────────────────────────────────────────────────────────

/// What the export wrote, for the result.
#[derive(Debug, Default)]
pub struct Summary {
    pub alignments: Vec<(String, f64, f64, usize, usize)>,
    pub terrains: usize,
    pub roads: usize,
    /// Corridor name, fill and cut volume (m³), and whether both bodies are closed.
    pub earthworks: Vec<(String, f64, f64, bool)>,
    pub buildings: usize,
    pub instances: u64,
}

struct W {
    s: Step,
    ns: String,
    base: XY,
    body: Id,
    axis: Id,
    origin: Id,
    styles: HashMap<&'static str, Id>,
}

impl W {
    fn guid(&self, key: &str) -> V {
        V::Str(step::guid(&self.ns, key))
    }

    fn add(&mut self, entity: &str, attrs: Vec<V>) -> Id {
        self.s.add(entity, attrs)
    }

    fn pt2(&mut self, p: XY) -> Id {
        self.add("IfcCartesianPoint", vec![step::reals(&p)])
    }

    fn dir2(&mut self, angle: f64) -> Id {
        self.add("IfcDirection", vec![step::reals(&[angle.cos(), angle.sin()])])
    }

    fn placement2(&mut self, p: XY, angle: f64) -> Id {
        let (l, d) = (self.pt2(p), self.dir2(angle));
        self.add("IfcAxis2Placement2D", vec![l.into(), d.into()])
    }

    fn local_placement(&mut self, rel: Option<Id>) -> Id {
        let o = self.origin;
        self.add("IfcLocalPlacement", vec![rel.into(), o.into()])
    }

    fn local(&self, p: P3) -> P3 {
        [p[0] - self.base[0], p[1] - self.base[1], p[2]]
    }

    fn style(&mut self, name: &'static str, rgb: (u8, u8, u8), transparency: f64) -> Id {
        if let Some(id) = self.styles.get(name) {
            return *id;
        }
        let c = self.add("IfcColourRgb", vec![V::Null, V::Real(rgb.0 as f64 / 255.0), V::Real(rgb.1 as f64 / 255.0), V::Real(rgb.2 as f64 / 255.0)]);
        let t = if transparency > 0.0 { V::Real(transparency) } else { V::Null };
        let sh = self.add("IfcSurfaceStyleShading", vec![c.into(), t]);
        let st = self.add("IfcSurfaceStyle", vec![name.into(), V::Enum("BOTH"), refs(&[sh])]);
        self.styles.insert(name, st);
        st
    }

    /// A triangulated body (local coordinates) as a product shape.
    fn shape(&mut self, pts: &[P3], tris: &[[usize; 3]], closed: Option<bool>, tin: bool, style: Option<Id>) -> Id {
        let coords = V::List(pts.iter().map(|p| step::reals(&self.local(*p))).collect());
        let list = self.add("IfcCartesianPointList3D", vec![coords, V::Null]);
        let index = V::List(tris.iter().map(|t| V::List(t.iter().map(|i| V::Int(*i as i64 + 1)).collect())).collect());
        let closed = closed.map_or(V::Null, V::Bool);
        let item = if tin {
            let flags = V::List(vec![V::Int(0); tris.len()]);
            self.add("IfcTriangulatedIrregularNetwork", vec![list.into(), V::Null, V::Bool(false), index, V::Null, flags])
        } else {
            self.add("IfcTriangulatedFaceSet", vec![list.into(), V::Null, closed, index, V::Null])
        };
        if let Some(st) = style {
            self.add("IfcStyledItem", vec![item.into(), refs(&[st]), V::Null]);
        }
        let body = self.body;
        let rep = self.add("IfcShapeRepresentation", vec![body.into(), "Body".into(), "Tessellation".into(), refs(&[item])]);
        self.add("IfcProductDefinitionShape", vec![V::Null, V::Null, refs(&[rep])])
    }

    fn rel(&mut self, entity: &str, key: &str, a: V, b: V) -> Id {
        let g = self.guid(key);
        self.add(entity, vec![g, V::Null, V::Null, V::Null, a, b])
    }

    fn pset(&mut self, key: &str, name: &str, objects: &[Id], props: Vec<(&str, V)>) {
        let values: Vec<Id> = props.into_iter().map(|(n, v)| self.add("IfcPropertySingleValue", vec![n.into(), V::Null, v, V::Null])).collect();
        let g = self.guid(&format!("{key}/pset"));
        let ps = self.add("IfcPropertySet", vec![g, V::Null, name.into(), V::Null, refs(&values)]);
        self.rel("IfcRelDefinesByProperties", &format!("{key}/pset-rel"), refs(objects), ps.into());
    }

    /// A built element with a body: `entity(GlobalId, …, Tag, PredefinedType)`.
    #[allow(clippy::too_many_arguments)]
    fn element(&mut self, entity: &str, key: &str, name: &str, desc: &str, object_type: &str, placement: Id, shape: Option<Id>, kind: &'static str) -> Id {
        let g = self.guid(key);
        self.add(entity, vec![g, V::Null, name.into(), step::opt_str(desc), step::opt_str(object_type), placement.into(), shape.into(), V::Null, V::Enum(kind)])
    }
}

fn label(t: &str) -> V {
    V::Typed("IFCLABEL", Box::new(V::Str(t.to_string())))
}

fn text(t: &str) -> V {
    V::Typed("IFCTEXT", Box::new(V::Str(t.to_string())))
}

/// An angle as IfcCompoundPlaneAngleMeasure: degrees, minutes, seconds,
/// millionths of a second, all with the sign of the angle.
fn compound(angle: f64) -> V {
    let sign = if angle < 0.0 { -1 } else { 1 };
    let mut us = (angle.abs() * 3600.0 * 1e6).round() as i64;
    let d = us / 3_600_000_000;
    us -= d * 3_600_000_000;
    let m = us / 60_000_000;
    us -= m * 60_000_000;
    let s = us / 1_000_000;
    us -= s * 1_000_000;
    V::List([d, m, s, us].iter().map(|v| V::Int(sign * v)).collect())
}

/// The model as an IFC 4.3 file. `file_name` goes in the header.
pub fn write(m: &Model, file_name: &str, time_stamp: &str) -> (String, Summary) {
    let mut sum = Summary::default();
    let base = m.crs.as_ref().map(|c| c.base).unwrap_or([0.0, 0.0]);
    let ns = format!("{}|{:.3}|{:.3}", m.name, base[0], base[1]);
    let mut w = W { s: Step::new(), ns, base, body: Id(0), axis: Id(0), origin: Id(0), styles: HashMap::new() };

    // Project, units, contexts.
    let units: Vec<Id> = [("LENGTHUNIT", "METRE"), ("AREAUNIT", "SQUARE_METRE"), ("VOLUMEUNIT", "CUBIC_METRE"), ("PLANEANGLEUNIT", "RADIAN")]
        .iter()
        .map(|(t, n)| w.add("IfcSIUnit", vec![V::Derived, V::Enum(t), V::Null, V::Enum(n)]))
        .collect();
    let metre = units[0];
    let unit_assignment = w.add("IfcUnitAssignment", vec![refs(&units)]);
    let o3 = w.add("IfcCartesianPoint", vec![step::reals(&[0.0, 0.0, 0.0])]);
    w.origin = w.add("IfcAxis2Placement3D", vec![o3.into(), V::Null, V::Null]);
    let origin = w.origin;
    let ctx = w.add("IfcGeometricRepresentationContext", vec![V::Null, "Model".into(), V::Int(3), V::Real(1e-5), origin.into(), V::Null]);
    w.body = w.add("IfcGeometricRepresentationSubContext", vec!["Body".into(), "Model".into(), V::Derived, V::Derived, V::Derived, V::Derived, ctx.into(), V::Null, V::Enum("MODEL_VIEW"), V::Null]);
    w.axis = w.add("IfcGeometricRepresentationSubContext", vec!["Axis".into(), "Model".into(), V::Derived, V::Derived, V::Derived, V::Derived, ctx.into(), V::Null, V::Enum("MODEL_VIEW"), V::Null]);
    let name = if m.name.is_empty() { "Open Infra".to_string() } else { m.name.clone() };
    let g = w.guid("project");
    let project = w.add("IfcProject", vec![g, V::Null, name.clone().into(), V::Null, V::Null, V::Null, V::Null, refs(&[ctx]), unit_assignment.into()]);

    // Georeferencing: the site's CRS, the base point as false origin.
    if let Some(c) = m.crs.as_ref().filter(|c| !c.geographic) {
        let crs = w.add("IfcProjectedCRS", vec![c.code.clone().into(), c.name.clone().into(), V::Null, c.vertical.clone().into(), V::Null, V::Null, metre.into()]);
        w.add("IfcMapConversion", vec![ctx.into(), crs.into(), V::Real(c.base[0]), V::Real(c.base[1]), V::Real(0.0), V::Real(1.0), V::Real(0.0), V::Real(1.0)]);
    }

    // Site.
    let site_pl = w.local_placement(None);
    let g = w.guid("site");
    let (lat, lon) = m.crs.as_ref().map(|c| (compound(c.lat), compound(c.lon))).unwrap_or((V::Null, V::Null));
    let site = w.add("IfcSite", vec![g, V::Null, name.clone().into(), V::Null, V::Null, site_pl.into(), V::Null, V::Null, V::Enum("ELEMENT"), lat, lon, V::Real(0.0), V::Null, V::Null]);
    let mut in_site: Vec<Id> = Vec::new();
    let mut site_parts: Vec<Id> = Vec::new();

    // Terrain: every TIN that is not a corridor's design surface.
    let mut terrain_b: Option<Id> = None;
    for (i, s) in m.surfaces.iter().enumerate().filter(|(_, s)| s.corridor.is_none() && !s.triangles.is_empty()) {
        let existing = s.status != 'N';
        let st = if existing { w.style(tr("Maaiveld bestaand", "Existing ground"), (128, 156, 96), 0.0) } else { w.style(tr("Maaiveld ontwerp", "Design surface"), (186, 170, 120), 0.0) };
        let shape = w.shape(&s.points, &s.triangles, None, true, Some(st));
        let pl = w.local_placement(Some(site_pl));
        let desc = if existing { tr("bestaand maaiveld (B)", "existing ground (B)") } else { tr("ontwerpoppervlak (N)", "design surface (N)") };
        let e = w.element("IfcGeographicElement", &format!("terrein/{i}/{}", s.name), &s.name, desc, "", pl, Some(shape), "TERRAIN");
        w.pset(&format!("terrein/{i}"), "OpenInfra_Terrain", &[e], vec![("Status", label(if existing { "B" } else { "N" })), ("Source", text(&s.source))]);
        if existing && terrain_b.is_none() {
            terrain_b = Some(e);
        }
        in_site.push(e);
        sum.terrains += 1;
    }

    // Alignments.
    let mut by_axis: HashMap<String, Id> = HashMap::new();
    let mut alignments: Vec<Id> = Vec::new();
    for (ai, a) in m.axes.iter().enumerate() {
        if a.segs.is_empty() {
            continue;
        }
        let key = format!("as/{ai}/{}", a.name);
        let pl = w.local_placement(None);
        // Horizontal: business logic and geometry.
        let hs: Vec<super::ifc::HSeg> = a.segs.iter().map(|s| horizontal_segment(s, base)).collect();
        let mut h_segments = Vec::new();
        let mut h_curves = Vec::new();
        for (i, h) in hs.iter().enumerate() {
            let next = hs.get(i + 1).map(|n| (n.start, n.dir, curvature(n.r0))).unwrap_or((h.end, h.end_dir, 0.0));
            let tr = transition((h.end, h.end_dir, curvature(h.r1)), next);
            let (seg, cs) = write_horizontal(&mut w, &format!("{key}/h{i}"), h, tr, pl);
            h_segments.push(seg);
            h_curves.push(cs);
        }
        // The closing zero-length segment.
        let last = hs.last().unwrap();
        let zero = HSeg { kind: "LINE", start: last.end, dir: last.end_dir, r0: 0.0, r1: 0.0, length: 0.0, parent: Parent::Line, curve_length: 0.0, end: last.end, end_dir: last.end_dir };
        let (seg, cs) = write_horizontal(&mut w, &format!("{key}/h-end"), &zero, "DISCONTINUOUS", pl);
        h_segments.push(seg);
        h_curves.push(cs);
        let composite = w.add("IfcCompositeCurve", vec![refs(&h_curves), V::Bool(false)]);
        // Vertical: the design profile.
        let vs = a.design.as_deref().map(vertical_segments).unwrap_or_default();
        let mut v_segments = Vec::new();
        let mut gradient = None;
        if !vs.is_empty() {
            let mut v_curves = Vec::new();
            for (i, v) in vs.iter().enumerate() {
                let (e, c) = v.curvature();
                let next = vs.get(i + 1).map(|n| ([n.s0, n.z0], n.g0.atan(), n.curvature().0)).unwrap_or(([v.end().0, v.end().1], v.g1.atan(), 0.0));
                let tr = transition(([v.end().0, v.end().1], v.g1.atan(), c), next);
                let _ = e;
                let (seg, cs) = write_vertical(&mut w, &format!("{key}/v{i}"), v, tr, pl);
                v_segments.push(seg);
                v_curves.push(cs);
            }
            let lv = vs.last().unwrap();
            let (s1, z1) = lv.end();
            let zero = VSeg { kind: "CONSTANTGRADIENT", s0: s1, length: 0.0, z0: z1, g0: lv.g1, g1: lv.g1 };
            let (seg, cs) = write_vertical(&mut w, &format!("{key}/v-end"), &zero, "DISCONTINUOUS", pl);
            v_segments.push(seg);
            v_curves.push(cs);
            gradient = Some(w.add("IfcGradientCurve", vec![refs(&v_curves), V::Bool(false), composite.into(), V::Null]));
        }
        let axis_ctx = w.axis;
        let reps = match gradient {
            Some(gc) => vec![
                w.add("IfcShapeRepresentation", vec![axis_ctx.into(), "FootPrint".into(), "Curve2D".into(), refs(&[composite])]),
                w.add("IfcShapeRepresentation", vec![axis_ctx.into(), "Axis".into(), "Curve3D".into(), refs(&[gc])]),
            ],
            None => vec![w.add("IfcShapeRepresentation", vec![axis_ctx.into(), "Axis".into(), "Curve2D".into(), refs(&[composite])])],
        };
        let shape = w.add("IfcProductDefinitionShape", vec![V::Null, V::Null, refs(&reps)]);
        let g = w.guid(&key);
        let al = w.add("IfcAlignment", vec![g, V::Null, a.name.clone().into(), V::Null, V::Null, pl.into(), shape.into(), V::Enum("NOTDEFINED")]);
        let g = w.guid(&format!("{key}/horizontal"));
        let hor = w.add("IfcAlignmentHorizontal", vec![g, V::Null, format!("{} {}", a.name, tr("horizontaal", "horizontal")).into(), V::Null, V::Null, V::Null, V::Null]);
        w.rel("IfcRelNests", &format!("{key}/h-nest"), hor.into(), refs(&h_segments));
        let mut layouts = vec![hor];
        if !v_segments.is_empty() {
            let g = w.guid(&format!("{key}/vertical"));
            let ver = w.add("IfcAlignmentVertical", vec![g, V::Null, format!("{} {}", a.name, tr("verticaal", "vertical")).into(), V::Null, V::Null, V::Null, V::Null]);
            w.rel("IfcRelNests", &format!("{key}/v-nest"), ver.into(), refs(&v_segments));
            layouts.push(ver);
        }
        w.rel("IfcRelNests", &format!("{key}/layouts"), al.into(), refs(&layouts));
        // Stationing: a referent at the start with Pset_Stationing.
        let d = w.add("IfcPointByDistanceExpression", vec![V::Typed("IFCLENGTHMEASURE", Box::new(V::Real(0.0))), V::Null, V::Null, V::Null, composite.into()]);
        let lin = w.add("IfcAxis2PlacementLinear", vec![d.into(), V::Null, V::Null]);
        let first = &hs[0];
        let p0 = w.add("IfcCartesianPoint", vec![step::reals(&[first.start[0], first.start[1], 0.0])]);
        let up = w.add("IfcDirection", vec![step::reals(&[0.0, 0.0, 1.0])]);
        let x = w.add("IfcDirection", vec![step::reals(&[first.dir.cos(), first.dir.sin(), 0.0])]);
        let fallback = w.add("IfcAxis2Placement3D", vec![p0.into(), up.into(), x.into()]);
        let lp = w.add("IfcLinearPlacement", vec![V::Null, lin.into(), fallback.into()]);
        let g = w.guid(&format!("{key}/station"));
        let referent = w.add("IfcReferent", vec![g, V::Null, super::station(a.start).into(), V::Null, V::Null, lp.into(), V::Null, V::Enum("STATION")]);
        w.pset(&format!("{key}/station"), "Pset_Stationing", &[referent], vec![("Station", V::Typed("IFCLENGTHMEASURE", Box::new(V::Real(a.start))))]);
        w.rel("IfcRelNests", &format!("{key}/stationing"), al.into(), refs(&[referent]));
        by_axis.insert(a.name.clone(), al);
        alignments.push(al);
        sum.alignments.push((a.name.clone(), a.length(), a.start, h_segments.len(), v_segments.len()));
    }

    // Corridors: a road (or for a dike, earthworks in the site) with the
    // pavement, berms, slopes and the fill and cut bodies.
    for (ci, c) in m.corridors.iter().enumerate() {
        let Some(s) = c.surface.and_then(|i| m.surfaces.get(i)) else { continue };
        let key = format!("corridor/{ci}/{}", c.name);
        let parts = classify(c, s);
        let road = c.edge(LineKind::Pavement).is_some();
        let (container, container_pl) = if road {
            let pl = w.local_placement(Some(site_pl));
            let g = w.guid(&key);
            let r = w.add("IfcRoad", vec![g, V::Null, c.name.clone().into(), step::opt_str(&format!("sjabloon {}", c.template)), V::Null, pl.into(), V::Null, V::Null, V::Enum("ELEMENT"), V::Enum("NOTDEFINED")]);
            site_parts.push(r);
            if let Some(al) = by_axis.get(&c.axis) {
                w.rel("IfcRelPositions", &format!("{key}/positions"), (*al).into(), refs(&[r]));
            }
            sum.roads += 1;
            (r, pl)
        } else {
            (site, site_pl)
        };
        let mut road_parts = Vec::new();
        let mut contained: Vec<Id> = Vec::new();
        // A road part (or the container itself) holding one element.
        let mut part = |w: &mut W, id: &str, part_name: &str, kind: &'static str, element: Id, contained: &mut Vec<Id>| {
            if road {
                let pl = w.local_placement(Some(container_pl));
                let g = w.guid(&format!("{key}/part/{id}"));
                let p = w.add("IfcRoadPart", vec![g, V::Null, part_name.into(), V::Null, V::Null, pl.into(), V::Null, V::Null, V::Null, V::Enum("LONGITUDINAL"), V::Enum(kind)]);
                w.rel("IfcRelContainedInSpatialStructure", &format!("{key}/part/{id}/contains"), refs(&[element]), p.into());
                road_parts.push(p);
            } else {
                contained.push(element);
            }
        };
        let surface_element = |w: &mut W, want: &[Part], entity: &str, id: &str, name: &str, kind: &'static str, style: Id| -> Option<Id> {
            let (pts, tris) = part_mesh(s, &parts, want);
            if tris.is_empty() {
                return None;
            }
            let shape = w.shape(&pts, &tris, Some(false), false, Some(style));
            let pl = w.local_placement(Some(container_pl));
            Some(w.element(entity, &format!("{key}/{id}"), name, "", "", pl, Some(shape), kind))
        };
        let asphalt = w.style(tr("Verharding", "Pavement"), (72, 72, 78), 0.0);
        if let Some(course) = surface_element(&mut w, &[Part::Pavement], "IfcCourse", "deklaag", tr("Verharding bovenvlak", "Pavement surface"), "PAVEMENT", asphalt) {
            let pl = w.local_placement(Some(container_pl));
            let desc = tr("rijstroken (alleen het bovenvlak; de opbouw staat niet in het sjabloon)", "lanes (top surface only; the assembly has no layer build-up)");
            let pavement = w.element("IfcPavement", &format!("{key}/verharding"), tr("Verharding", "Pavement"), desc, "", pl, None, "FLEXIBLE");
            w.rel("IfcRelAggregates", &format!("{key}/verharding/lagen"), pavement.into(), refs(&[course]));
            part(&mut w, "rijbaan", tr("Rijbaan", "Carriageway"), "CARRIAGEWAY", pavement, &mut contained);
        }
        let grass = w.style(tr("Berm", "Shoulder"), (112, 158, 82), 0.0);
        let berm_name = if road { tr("Berm", "Shoulder") } else { tr("Kruin", "Crest") };
        if let Some(e) = surface_element(&mut w, &[Part::Shoulder, Part::Crest], "IfcCourse", "berm", berm_name, "PROTECTION", grass) {
            part(&mut w, "berm", tr("Berm", "Shoulder"), "SOFTSHOULDER", e, &mut contained);
        }
        let slope = w.style(tr("Talud", "Slope"), (136, 176, 96), 0.0);
        if let Some(e) = surface_element(&mut w, &[Part::Slope], "IfcCourse", "talud", tr("Talud", "Slope"), "PROTECTION", slope) {
            part(&mut w, "talud", tr("Talud", "Slope"), "ROADSIDE", e, &mut contained);
        }
        // Earthworks between the design and the existing ground.
        let (mut fill_v, mut cut_v, mut closed) = (0.0, 0.0, true);
        if c.ground.len() == s.points.len() && c.ground.iter().any(Option::is_some) {
            let fill = earth_body(&s.points, &c.ground, &s.triangles, true);
            if !fill.triangles.is_empty() {
                let st = w.style(tr("Ophoging", "Fill"), (170, 128, 86), 0.4);
                let shape = w.shape(&fill.points, &fill.triangles, Some(fill.closed), false, Some(st));
                let pl = w.local_placement(Some(container_pl));
                let name = if road { tr("Ophoging", "Fill") } else { tr("Dijklichaam", "Dike body") };
                let desc = format!("{} {}", tr("tussen ontwerp en", "between the design and"), c.ground_label);
                let e = w.element("IfcEarthworksFill", &format!("{key}/ophoging"), name, &desc, "", pl, Some(shape), "EMBANKMENT");
                w.pset(&format!("{key}/ophoging"), "OpenInfra_Earthworks", &[e], vec![("Volume", V::Typed("IFCVOLUMEMEASURE", Box::new(V::Real(fill.volume)))), ("Closed", V::Typed("IFCBOOLEAN", Box::new(V::Bool(fill.closed))))]);
                contained.push(e);
                fill_v = fill.volume;
                closed &= fill.closed;
            }
            // A cut is a subtraction: it voids the existing terrain.
            if let Some(terrain) = terrain_b {
                let cut = earth_body(&s.points, &c.ground, &s.triangles, false);
                if !cut.triangles.is_empty() {
                    let st = w.style(tr("Ontgraving", "Cut"), (204, 120, 64), 0.4);
                    let shape = w.shape(&cut.points, &cut.triangles, Some(cut.closed), false, Some(st));
                    let pl = w.local_placement(Some(site_pl));
                    let desc = format!("{} {} {}", tr("tussen", "between"), c.ground_label, tr("en het ontwerp", "and the design"));
                    let e = w.element("IfcEarthworksCut", &format!("{key}/ontgraving"), tr("Ontgraving", "Cut"), &desc, "", pl, Some(shape), "CUT");
                    w.pset(&format!("{key}/ontgraving"), "OpenInfra_Earthworks", &[e], vec![("Volume", V::Typed("IFCVOLUMEMEASURE", Box::new(V::Real(cut.volume)))), ("Closed", V::Typed("IFCBOOLEAN", Box::new(V::Bool(cut.closed))))]);
                    w.rel("IfcRelVoidsElement", &format!("{key}/ontgraving/voids"), terrain.into(), e.into());
                    cut_v = cut.volume;
                    closed &= cut.closed;
                }
            }
            sum.earthworks.push((c.name.clone(), fill_v, cut_v, closed));
        }
        if road {
            if !road_parts.is_empty() {
                w.rel("IfcRelAggregates", &format!("{key}/parts"), container.into(), refs(&road_parts));
            }
            if !contained.is_empty() {
                w.rel("IfcRelContainedInSpatialStructure", &format!("{key}/contains"), refs(&contained), container.into());
            }
        } else {
            in_site.extend(contained);
        }
    }

    // 3D BAG buildings: roof and walls in a building.
    for (bi, b) in m.buildings.iter().enumerate() {
        let key = format!("pand/{}", b.id);
        let pl = w.local_placement(Some(site_pl));
        let g = w.guid(&key);
        let building = w.add("IfcBuilding", vec![g, V::Null, b.id.clone().into(), "3D BAG".into(), V::Null, pl.into(), V::Null, V::Null, V::Enum("ELEMENT"), V::Null, V::Null, V::Null]);
        let mut elements = Vec::new();
        for (id, what, tris, entity, rgb) in [("dak", tr("Dak", "Roof"), &b.roof, "IfcRoof", (150, 82, 64)), ("gevels", tr("Gevels", "Walls"), &b.walls, "IfcWall", (214, 202, 186))] {
            if tris.is_empty() {
                continue;
            }
            let mut pts = Points::default();
            let idx: Vec<[usize; 3]> = tris.iter().map(|t| t.map(|p| pts.at(p))).collect();
            let st = w.style(what, rgb, 0.0);
            let shape = w.shape(&pts.points, &idx, None, false, Some(st));
            let epl = w.local_placement(Some(pl));
            elements.push(w.element(entity, &format!("{key}/{id}"), what, "", "", epl, Some(shape), "NOTDEFINED"));
        }
        if !elements.is_empty() {
            w.rel("IfcRelContainedInSpatialStructure", &format!("{key}/contains"), refs(&elements), building.into());
        }
        let mut props = vec![("BuildingID", V::Typed("IFCIDENTIFIER", Box::new(V::Str(b.id.clone()))))];
        if !b.year.is_empty() {
            props.push(("YearOfConstruction", label(&b.year)));
        }
        w.pset(&format!("{key}/{bi}"), "Pset_BuildingCommon", &[building], props);
        site_parts.push(building);
        sum.buildings += 1;
    }

    // The structure: project ⊃ site and alignments; site ⊃ roads and
    // buildings; the site contains the terrain; it references the alignments.
    let mut top = vec![site];
    top.extend(&alignments);
    w.rel("IfcRelAggregates", "project/parts", project.into(), refs(&top));
    if !site_parts.is_empty() {
        w.rel("IfcRelAggregates", "site/parts", site.into(), refs(&site_parts));
    }
    if !in_site.is_empty() {
        w.rel("IfcRelContainedInSpatialStructure", "site/contains", refs(&in_site), site.into());
    }
    if !alignments.is_empty() {
        w.rel("IfcRelReferencedInSpatialStructure", "site/alignments", refs(&alignments), site.into());
    }
    sum.instances = w.s.count();
    let header = step::Header {
        description: vec!["ViewDefinition [ReferenceView]".into()],
        name: file_name.to_string(),
        time_stamp: time_stamp.to_string(),
        author: String::new(),
        organization: String::new(),
        preprocessor: format!("Open Infra {}", env!("CARGO_PKG_VERSION")),
        originating_system: format!("Open Infra {} (Open CAD Studio)", env!("CARGO_PKG_VERSION")),
        schema: SCHEMA.into(),
    };
    (w.s.finish(&header), sum)
}

/// An alignment segment with its business logic and its curve segment
/// (also its 'Axis'/'Segment' representation).
fn write_horizontal(w: &mut W, key: &str, h: &HSeg, transition: &'static str, pl: Id) -> (Id, Id) {
    let start = w.pt2(h.start);
    let dp = w.add(
        "IfcAlignmentHorizontalSegment",
        vec![V::Null, V::Null, start.into(), V::Real(h.dir), V::Real(h.r0), V::Real(h.r1), V::Real(h.length), V::Null, V::Enum(h.kind)],
    );
    let parent = match &h.parent {
        Parent::Line => {
            let o = w.pt2([0.0, 0.0]);
            let d = w.dir2(0.0);
            let v = w.add("IfcVector", vec![d.into(), V::Real(1.0)]);
            w.add("IfcLine", vec![o.into(), v.into()])
        }
        Parent::Circle { r } => {
            let p = w.placement2([0.0, 0.0], 0.0);
            w.add("IfcCircle", vec![p.into(), V::Real(*r)])
        }
        Parent::Clothoid { a, .. } => {
            let p = w.placement2([0.0, 0.0], 0.0);
            w.add("IfcClothoid", vec![p.into(), V::Real(*a)])
        }
    };
    let seg_start = match &h.parent {
        Parent::Clothoid { start, .. } => *start,
        Parent::Line | Parent::Circle { .. } => 0.0,
    };
    let place = w.placement2(h.start, h.dir);
    let len = |x: f64| V::Typed("IFCLENGTHMEASURE", Box::new(V::Real(x)));
    let cs = w.add("IfcCurveSegment", vec![V::Enum(transition), place.into(), len(seg_start), len(h.curve_length), parent.into()]);
    let seg = segment(w, key, cs, dp, pl);
    (seg, cs)
}

fn write_vertical(w: &mut W, key: &str, v: &VSeg, transition: &'static str, pl: Id) -> (Id, Id) {
    let radius = if v.kind == "PARABOLICARC" && (v.g1 - v.g0).abs() > 1e-12 { V::Real(v.length / (v.g1 - v.g0)) } else { V::Null };
    let dp = w.add("IfcAlignmentVerticalSegment", vec![V::Null, V::Null, V::Real(v.s0), V::Real(v.length), V::Real(v.z0), V::Real(v.g0), V::Real(v.g1), radius, V::Enum(v.kind)]);
    let (parent, length) = if v.kind == "PARABOLICARC" {
        let p = w.placement2([0.0, 0.0], 0.0);
        let c = (v.g1 - v.g0) / (2.0 * v.length.max(1e-12));
        let poly = w.add("IfcPolynomialCurve", vec![p.into(), step::reals(&[0.0, 1.0]), step::reals(&[v.z0, v.g0, c]), V::Null]);
        (poly, parabola_length(v.g0, c, v.length))
    } else {
        let o = w.pt2([0.0, 0.0]);
        let d = w.dir2(0.0);
        let vec = w.add("IfcVector", vec![d.into(), V::Real(1.0)]);
        (w.add("IfcLine", vec![o.into(), vec.into()]), v.length * (1.0 + v.g0 * v.g0).sqrt())
    };
    let place = w.placement2([v.s0, v.z0], v.g0.atan());
    let len = |x: f64| V::Typed("IFCLENGTHMEASURE", Box::new(V::Real(x)));
    let cs = w.add("IfcCurveSegment", vec![V::Enum(transition), place.into(), len(0.0), len(length), parent.into()]);
    let seg = segment(w, key, cs, dp, pl);
    (seg, cs)
}

fn segment(w: &mut W, key: &str, cs: Id, dp: Id, pl: Id) -> Id {
    let axis = w.axis;
    let rep = w.add("IfcShapeRepresentation", vec![axis.into(), "Axis".into(), "Segment".into(), refs(&[cs])]);
    let shape = w.add("IfcProductDefinitionShape", vec![V::Null, V::Null, refs(&[rep])]);
    let g = w.guid(key);
    w.add("IfcAlignmentSegment", vec![g, V::Null, V::Null, V::Null, V::Null, pl.into(), shape.into(), dp.into()])
}

/// `CIV_IFC_EXPORT [BESTAND=<pad>] [ALLES]`: the model as IFC 4.3.
pub fn export(opt: &super::CivielOptions, path: Option<std::path::PathBuf>, all: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = super::Ctx::new(opt)?;
    let m = super::uitwisseling::gather(&ctx, all, true, progress)?;
    if m.is_empty() {
        return Err(tr("niets om te exporteren: geen TIN's, assen, corridors of 3D BAG-panden (in de selectie)", "nothing to export: no TINs, alignments, corridors or 3D BAG buildings (in the selection)").into());
    }
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing("ifc4x3.ifc"));
    progress(tr("IFC schrijven…", "Writing IFC…").into());
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (text, sum) = write(&m, &file, &chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string());
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    use serde_json::json;
    use super::api::note as api;
    api("bestand", path.to_string_lossy().into_owned());
    api("schema", SCHEMA);
    api("bytes", text.len());
    api("instanties", sum.instances);
    api("assen", sum.alignments.iter().map(|(n, l, s, h, v)| json!({"naam": n, "lengte_m": l, "beginstation": s, "horizontaal": h, "verticaal": v})).collect::<Vec<_>>());
    api("terreinen", sum.terrains);
    api("wegen", sum.roads);
    api("grondverzet", sum.earthworks.iter().map(|(n, f, c, closed)| json!({"corridor": n, "ophogen_m3": f, "afgraven_m3": c, "gesloten": closed})).collect::<Vec<_>>());
    api("panden", sum.buildings);
    api("crs", m.crs.as_ref().map(|c| json!({"code": c.code, "verticaal": c.vertical, "eastings": c.base[0], "northings": c.base[1]})).unwrap_or(serde_json::Value::Null));
    let mut parts = Vec::new();
    if !sum.alignments.is_empty() {
        parts.push(format!("{} {}", sum.alignments.len(), tr("as(sen)", "alignment(s)")));
    }
    if sum.terrains > 0 {
        parts.push(format!("{} {}", sum.terrains, tr("terrein(en)", "terrain(s)")));
    }
    if sum.roads > 0 || !sum.earthworks.is_empty() {
        let (f, c) = sum.earthworks.iter().fold((0.0, 0.0), |a, e| (a.0 + e.1, a.1 + e.2));
        parts.push(format!("{} corridor(s), {} {} m³, {} {} m³", m.corridors.len(), tr("ophogen", "fill"), super::terrein::m3(f), tr("afgraven", "cut"), super::terrein::m3(c)));
    }
    if sum.buildings > 0 {
        parts.push(format!("{} {}", sum.buildings, tr("panden", "buildings")));
    }
    Ok(format!(
        "IFC 4.3 ({SCHEMA}, {}): {} — {}.",
        m.crs.as_ref().map(|c| format!("{} + {}", c.code, c.vertical)).unwrap_or_else(|| tr("zonder georeferentie", "not georeferenced").into()),
        parts.join(", "),
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn horizontal_segments_of_an_axis() {
        // East 100 m, a quarter circle R 50 to the left, north.
        let al = Alignment::from_pis(&[[1000.0, 2000.0], [1100.0, 2000.0], [1100.0, 2100.0]], &[50.0]).unwrap();
        let hs: Vec<HSeg> = al.segs.iter().map(|s| horizontal_segment(s, [1000.0, 2000.0])).collect();
        assert_eq!(hs.iter().map(|h| h.kind).collect::<Vec<_>>(), ["LINE", "CIRCULARARC", "LINE"]);
        assert_eq!((hs[0].start, hs[0].dir, hs[0].length), ([0.0, 0.0], 0.0, 50.0));
        let arc = &hs[1];
        assert!((arc.r0 - 50.0).abs() < 1e-9 && arc.r0 == arc.r1, "left: positive radius");
        assert!((arc.length - 25.0 * PI).abs() < 1e-9 && arc.curve_length > 0.0);
        assert!((arc.end[0] - 100.0).abs() < 1e-9 && (arc.end[1] - 50.0).abs() < 1e-9);
        assert!((arc.end_dir - PI / 2.0).abs() < 1e-9);
        // Line into the tangent curve: same direction, other curvature.
        assert_eq!(transition((hs[0].end, hs[0].end_dir, 0.0), (arc.start, arc.dir, 1.0 / 50.0)), "CONTSAMEGRADIENT");
        assert_eq!(transition((hs[0].end, hs[0].end_dir, 0.0), (hs[0].end, hs[0].end_dir + 0.3, 0.0)), "CONTINUOUS");
        assert_eq!(transition((hs[0].end, 0.0, 0.0), ([60.0, 0.0], 0.0, 0.0)), "DISCONTINUOUS");
        // A right turn runs clockwise: negative radius and curve length.
        let right = Alignment::from_pis(&[[0.0, 0.0], [100.0, 0.0], [100.0, -100.0]], &[30.0]).unwrap();
        let h = horizontal_segment(&right.segs[1], [0.0, 0.0]);
        assert!(h.r0 < 0.0 && h.curve_length < 0.0);
    }

    #[test]
    fn a_design_profile_as_vertical_segments() {
        // Up 2 % for 100 m, down 1 % for 100 m, a 50 m crest curve.
        let pvis = as_profiel_fit(&[(0.0, 0.0), (100.0, 2.0), (200.0, 1.0)], 50.0);
        let vs = vertical_segments(&pvis);
        assert_eq!(vs.iter().map(|v| v.kind).collect::<Vec<_>>(), ["CONSTANTGRADIENT", "PARABOLICARC", "CONSTANTGRADIENT"]);
        assert_eq!((vs[0].s0, vs[0].length, vs[0].g0), (0.0, 75.0, 0.02));
        let c = &vs[1];
        assert_eq!((c.s0, c.length, c.g0, c.g1), (75.0, 50.0, 0.02, -0.01));
        assert!((c.z0 - 1.5).abs() < 1e-12);
        // The end of the curve meets the next grade, and the profile ends on the last PVI.
        assert!((c.end().1 - 1.75).abs() < 1e-12 && (vs[2].z0 - 1.75).abs() < 1e-12);
        assert!((vs[2].end().0 - 200.0).abs() < 1e-12 && (vs[2].end().1 - 1.0).abs() < 1e-12);
        // Heights agree with the design profile everywhere.
        for v in &vs {
            for t in [0.0, 0.3, 1.0] {
                let s = v.s0 + v.length * t;
                let z = v.z0 + v.g0 * (s - v.s0) + (v.g1 - v.g0) / (2.0 * v.length) * (s - v.s0).powi(2);
                assert!((z - super::super::as_profiel::design_z(&pvis, s)).abs() < 1e-9, "{s}");
            }
        }
        // Parabola length: nearly the chord for gentle grades.
        let l = parabola_length(0.02, -0.0003, 50.0);
        assert!(l > 50.0 && l < 50.02, "{l}");
        assert!((parabola_length(0.5, 0.0, 10.0) - 10.0 * 1.25f64.sqrt()).abs() < 1e-12);
    }

    fn as_profiel_fit(pts: &[(f64, f64)], l: f64) -> Vec<Pvi> {
        super::super::as_profiel::fit_curves(pts, l)
    }

    #[test]
    fn clothoid_parent_curves() {
        // Straight into R 200 to the left over 60 m: A = sqrt(R L), from
        // the clothoid's origin.
        match clothoid_parent(0.0, 200.0, 60.0) {
            Parent::Clothoid { a, start } => {
                assert!((a - (200.0f64 * 60.0).sqrt()).abs() < 1e-9);
                assert!(start.abs() < 1e-9, "{start}");
            }
            p => panic!("{p:?}"),
        }
        // From R 200 out to straight: the 60 m before the origin.
        match clothoid_parent(200.0, 0.0, 60.0) {
            Parent::Clothoid { a, start } => assert!(a < 0.0 && (start + 60.0).abs() < 1e-9, "{a} {start}"),
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn earthworks_are_closed_bodies() {
        // A 20 × 10 m design plane at 1 m over ground at 0, sloping down to
        // the ground on the east side (x = 20): a wedge of 20 × 10 × 1 / 2.
        let pts: Vec<P3> = vec![[0.0, 0.0, 1.0], [20.0, 0.0, 0.0], [20.0, 10.0, 0.0], [0.0, 10.0, 1.0]];
        let ground = vec![Some(0.0); 4];
        let tris = vec![[0, 1, 2], [0, 2, 3]];
        let fill = earth_body(&pts, &ground, &tris, true);
        assert!(fill.closed, "{:?}", fill.triangles);
        assert!((fill.volume - 100.0).abs() < 1e-9, "{}", fill.volume);
        assert!(earth_body(&pts, &ground, &tris, false).triangles.is_empty(), "no cut");
        // Ground at 0.5: fill west of x = 10, cut east of it.
        let ground = vec![Some(0.5); 4];
        let fill = earth_body(&pts, &ground, &tris, true);
        let cut = earth_body(&pts, &ground, &tris, false);
        assert!(fill.closed && cut.closed);
        assert!((fill.volume - 25.0).abs() < 1e-9 && (cut.volume - 25.0).abs() < 1e-9, "{} {}", fill.volume, cut.volume);
    }

    #[test]
    fn writes_a_georeferenced_ifc() {
        let mut m = super::super::uitwisseling::sample_model();
        // A corridor surface over the axis's first 40 m, 1 m above the
        // ground on the left and 0.5 m below it on the right.
        let al = m.axes[0].segs.clone();
        let o = [104_885.0, 425_320.0];
        let pts: Vec<P3> = vec![[o[0] - 80.0, o[1] - 14.0, 1.6], [o[0] - 40.0, o[1] - 14.0, 1.6], [o[0] - 40.0, o[1] - 26.0, 0.4], [o[0] - 80.0, o[1] - 26.0, 0.4]];
        m.surfaces.push(Surface { name: "Corridor As 1".into(), status: 'N', source: String::new(), points: pts, triangles: vec![[0, 2, 1], [0, 3, 2]], corridor: Some("Corridor As 1".into()) });
        m.corridors[0].surface = Some(1);
        m.corridors[0].axis_segs = al;
        m.corridors[0].ground = vec![Some(0.8); 4];
        m.corridors[0].ground_label = "Terrein 1".into();
        let (text, sum) = write(&m, "test.ifc", "2026-10-29T12:00:00");
        // For tests/e2e/uitwisseling_check.py: OI_EXPORT_SAMPLES=<dir>.
        if let Ok(dir) = std::env::var("OI_EXPORT_SAMPLES") {
            let _ = std::fs::write(format!("{dir}/sample.ifc"), &text);
            let stamp = super::super::landxml::Stamp { date: "2026-10-29".into(), time: "12:00:00".into() };
            let _ = std::fs::write(format!("{dir}/sample.xml"), super::super::landxml::write(&m, &stamp));
        }
        assert!(text.contains("FILE_SCHEMA(('IFC4X3_ADD2'));"));
        assert!(text.contains("IFCPROJECTEDCRS('EPSG:28992','Amersfoort / RD New',$,'NAP',$,$,#1);"));
        assert!(text.contains(",104885.,425320.,0.,1.,0.,1.);"), "map conversion with the base point");
        assert!(text.contains("IFCALIGNMENTHORIZONTALSEGMENT($,$,"));
        assert!(text.contains(".PARABOLICARC.") && text.contains("IFCGRADIENTCURVE("));
        assert!(text.contains("IFCREFERENT(") && text.contains("'Pset_Stationing'"));
        assert!(text.contains(".TERRAIN.") && text.contains("IFCTRIANGULATEDIRREGULARNETWORK("));
        assert!(text.contains("IFCROAD(") && text.contains("IFCEARTHWORKSFILL(") && text.contains("IFCEARTHWORKSCUT(") && text.contains("IFCRELVOIDSELEMENT("));
        assert_eq!(sum.alignments.len(), 1);
        assert_eq!((sum.alignments[0].3, sum.alignments[0].4), (4, 4), "3 + 1 horizontal, 3 + 1 vertical with the zero-length ends");
        let (_, fill, cut, closed) = &sum.earthworks[0];
        assert!(*closed && *fill > 0.0 && *cut > 0.0, "{:?}", sum.earthworks);
        // Every instance is referenced by number and defined once.
        let defined: Vec<u64> = text.lines().filter_map(|l| l.strip_prefix('#')).filter_map(|l| l.split('=').next()?.parse().ok()).collect();
        assert_eq!(defined.len() as u64, sum.instances);
        assert!(defined.windows(2).all(|w| w[1] == w[0] + 1));
    }
}
