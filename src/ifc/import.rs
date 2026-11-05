//! Reading IFC (issue #66): alignments, terrain and element geometry from a
//! STEP file parsed by `rekenkern::step`, as plain data in metres. The
//! drawing side is `civiel::ifc_import` (`CIV_IFC_IMPORT`).
//!
//! What is read (IFC 4.3 ADD2 attribute positions; IFC4 and IFC2X3 agree
//! for everything but the alignments):
//!
//! - units: IfcSIUnit prefixes and IfcConversionBasedUnit for length and
//!   plane angle (the project's IfcUnitAssignment);
//! - georeference: IfcMapConversion (and …Scaled) with its IfcProjectedCRS:
//!   Eastings, Northings, OrthogonalHeight, the rotation from
//!   XAxisAbscissa/XAxisOrdinate and Scale (applied in plan);
//! - placements: IfcLocalPlacement chains of IfcAxis2Placement3D/2D;
//!   IfcLinearPlacement by its CartesianPosition;
//! - IfcAlignment → IfcRelNests → IfcAlignmentHorizontal / Vertical →
//!   IfcRelNests → IfcAlignmentSegment.DesignParameters: horizontal LINE,
//!   CIRCULARARC and CLOTHOID (other transition curves become a clothoid
//!   with the same radii and length, reported), vertical CONSTANTGRADIENT and
//!   PARABOLICARC (a CIRCULARARC or CLOTHOID becomes a parabola between the
//!   same grades); the start station from the Pset_Stationing of the first
//!   IfcReferent;
//! - geometry: IfcTriangulatedFaceSet, IfcTriangulatedIrregularNetwork,
//!   IfcPolygonalFaceSet, IfcFacetedBrep, IfcShellBasedSurfaceModel,
//!   IfcFaceBasedSurfaceModel, IfcMappedItem, IfcExtrudedAreaSolid
//!   (rectangle, circle and polyline profiles, with voids), IfcSweptDiskSolid
//!   along a polyline, and of a Boolean result its first operand; colours
//!   from IfcStyledItem and IfcIndexedColourMap;
//! - property sets and quantities of each element (and its type).
//!
//! Not read, and counted per kind: other swept and advanced solids
//! (IfcSectionedSolidHorizontal, IfcRevolvedAreaSolid, NURBS/IfcAdvancedBrep,
//! CSG primitives) and curve-only representations.

use std::collections::{BTreeMap, HashSet};
use std::f64::consts::{FRAC_PI_2, TAU};

use rekenkern::step::{Entity, StepModel, Value};

use super::alignment::{HSeg, VKind, VSeg};
use crate::civiel::as_profiel::{Pvi, Seg};

pub type P3 = [f64; 3];
type XY = [f64; 2];

// ── Transforms ──────────────────────────────────────────────────────────────

/// An affine transform: three columns (x, y, z axes) and a translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat {
    pub x: P3,
    pub y: P3,
    pub z: P3,
    pub t: P3,
}

impl Mat {
    pub const IDENTITY: Mat = Mat { x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0], t: [0.0, 0.0, 0.0] };

    pub fn apply(&self, p: P3) -> P3 {
        [0, 1, 2].map(|k| self.x[k] * p[0] + self.y[k] * p[1] + self.z[k] * p[2] + self.t[k])
    }

    fn dir(&self, v: P3) -> P3 {
        [0, 1, 2].map(|k| self.x[k] * v[0] + self.y[k] * v[1] + self.z[k] * v[2])
    }

    /// `self` after `inner`: first `inner`, then `self`.
    pub fn then_of(&self, inner: &Mat) -> Mat {
        Mat { x: self.dir(inner.x), y: self.dir(inner.y), z: self.dir(inner.z), t: self.apply(inner.t) }
    }

    /// The rotation about z of the x axis (radians).
    pub fn plan_rotation(&self) -> f64 {
        self.x[1].atan2(self.x[0])
    }
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: P3, b: P3) -> P3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn unit(a: P3) -> Option<P3> {
    let n = dot(a, a).sqrt();
    (n > 1e-12).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

// ── Units and georeference ──────────────────────────────────────────────────

/// Metres per length unit and radians per angle unit of the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Units {
    pub length: f64,
    pub angle: f64,
}

fn prefix(p: Option<&str>) -> f64 {
    match p.unwrap_or("") {
        "EXA" => 1e18,
        "PETA" => 1e15,
        "TERA" => 1e12,
        "GIGA" => 1e9,
        "MEGA" => 1e6,
        "KILO" => 1e3,
        "HECTO" => 1e2,
        "DECA" => 1e1,
        "DECI" => 1e-1,
        "CENTI" => 1e-2,
        "MILLI" => 1e-3,
        "MICRO" => 1e-6,
        "NANO" => 1e-9,
        _ => 1.0,
    }
}

/// The size of a unit in SI (metre, radian): IfcSIUnit with its prefix, or
/// an IfcConversionBasedUnit (foot, degree) through its measure.
fn unit_size(m: &StepModel, e: &Entity, depth: usize) -> Option<(String, f64)> {
    match e.name.as_str() {
        // IfcSIUnit(Dimensions, UnitType, Prefix, Name)
        "IFCSIUNIT" => Some((e.enum_at(1)?.to_string(), prefix(e.enum_at(2)))),
        // IfcConversionBasedUnit(Dimensions, UnitType, Name, ConversionFactor)
        "IFCCONVERSIONBASEDUNIT" | "IFCCONVERSIONBASEDUNITWITHOFFSET" if depth < 4 => {
            let kind = e.enum_at(1)?.to_string();
            let measure = m.get(e.ref_at(3)?)?;
            // IfcMeasureWithUnit(ValueComponent, UnitComponent)
            let v = measure.arg(0)?.as_f64()?;
            let base = m.get(measure.ref_at(1)?).and_then(|u| unit_size(m, u, depth + 1)).map(|u| u.1).unwrap_or(1.0);
            Some((kind, v * base))
        }
        _ => None,
    }
}

/// The project's length and angle units (metre and radian by default).
pub fn units(m: &StepModel) -> Units {
    let mut u = Units { length: 1.0, angle: 1.0 };
    let assignment = m.by_type("IFCPROJECT").first().and_then(|p| p.ref_at(8)).and_then(|r| m.get(r)).or_else(|| m.by_type("IFCUNITASSIGNMENT").into_iter().next());
    let Some(a) = assignment else { return u };
    for r in a.refs_at(0) {
        if let Some((kind, size)) = m.get(r).and_then(|e| unit_size(m, e, 0)) {
            match kind.as_str() {
                "LENGTHUNIT" => u.length = size,
                "PLANEANGLEUNIT" => u.angle = size,
                _ => {}
            }
        }
    }
    u
}

/// IfcMapConversion: local metres → map coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct MapConversion {
    pub eastings: f64,
    pub northings: f64,
    pub height: f64,
    /// Rotation of the local x axis from grid east (radians, counter-clockwise).
    pub rotation: f64,
    pub scale: f64,
    /// The target CRS name ("EPSG:28992").
    pub crs: Option<String>,
}

impl MapConversion {
    /// Map coordinates of a local point (metres).
    pub fn apply(&self, p: P3) -> P3 {
        let (s, c) = self.rotation.sin_cos();
        let k = self.scale;
        [self.eastings + k * (c * p[0] - s * p[1]), self.northings + k * (s * p[0] + c * p[1]), self.height + p[2]]
    }
}

/// A CRS name in the "EPSG:n" form when it is an EPSG code in any spelling.
pub fn crs_name(name: &str) -> String {
    let t = name.trim();
    let digits = t.trim_start_matches(|c: char| !c.is_ascii_digit());
    if t.to_ascii_uppercase().contains("EPSG") && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return format!("EPSG:{digits}");
    }
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        return format!("EPSG:{t}");
    }
    t.to_string()
}

/// The first IfcMapConversion; its offsets in metres (map unit, else the
/// project's length unit).
pub fn map_conversion(m: &StepModel, u: &Units) -> Option<MapConversion> {
    let e = m.by_type("IFCMAPCONVERSION").into_iter().chain(m.by_type("IFCMAPCONVERSIONSCALED")).next()?;
    let crs = e.ref_at(1).and_then(|r| m.get(r));
    // IfcProjectedCRS(Name, Description, GeodeticDatum, VerticalDatum, MapProjection, MapZone, MapUnit)
    let map_unit = crs.and_then(|c| c.ref_at(6)).and_then(|r| m.get(r)).and_then(|x| unit_size(m, x, 0)).map(|x| x.1).unwrap_or(u.length);
    let a = e.f64_at(5).unwrap_or(1.0);
    let o = e.f64_at(6).unwrap_or(0.0);
    Some(MapConversion {
        eastings: e.f64_at(2).unwrap_or(0.0) * map_unit,
        northings: e.f64_at(3).unwrap_or(0.0) * map_unit,
        height: e.f64_at(4).unwrap_or(0.0) * map_unit,
        rotation: if a == 0.0 && o == 0.0 { 0.0 } else { o.atan2(a) },
        scale: e.f64_at(7).filter(|s| *s > 0.0).unwrap_or(1.0),
        crs: crs.and_then(|c| c.str_at(0)).map(crs_name),
    })
}

// ── Placements ──────────────────────────────────────────────────────────────

/// Reads entities in metres with the file's units.
pub struct Reader<'a> {
    pub m: &'a StepModel,
    pub u: Units,
}

impl<'a> Reader<'a> {
    pub fn new(m: &'a StepModel) -> Reader<'a> {
        Reader { m, u: units(m) }
    }

    fn get(&self, id: u64) -> Option<&'a Entity> {
        self.m.get(id)
    }

    /// A list of numbers.
    fn nums(v: Option<&Value>) -> Vec<f64> {
        v.and_then(Value::as_list).map(|l| l.iter().filter_map(Value::as_f64).collect()).unwrap_or_default()
    }

    /// IfcCartesianPoint in metres (2D points get z = 0).
    fn point(&self, id: u64) -> Option<P3> {
        let c = Self::nums(self.get(id)?.arg(0));
        let k = self.u.length;
        match c.len() {
            2 => Some([c[0] * k, c[1] * k, 0.0]),
            3 => Some([c[0] * k, c[1] * k, c[2] * k]),
            _ => None,
        }
    }

    fn direction(&self, id: Option<u64>) -> Option<P3> {
        let c = Self::nums(self.get(id?)?.arg(0));
        unit(match c.len() {
            2 => [c[0], c[1], 0.0],
            3 => [c[0], c[1], c[2]],
            _ => return None,
        })
    }

    /// IfcAxis2Placement3D / 2D as a transform.
    pub fn axis_placement(&self, id: u64) -> Mat {
        let Some(e) = self.get(id) else { return Mat::IDENTITY };
        let t = e.ref_at(0).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
        let (z, xref) = match e.name.as_str() {
            "IFCAXIS2PLACEMENT3D" => (self.direction(e.ref_at(1)).unwrap_or([0.0, 0.0, 1.0]), self.direction(e.ref_at(2)).unwrap_or([1.0, 0.0, 0.0])),
            "IFCAXIS2PLACEMENT2D" => ([0.0, 0.0, 1.0], self.direction(e.ref_at(1)).unwrap_or([1.0, 0.0, 0.0])),
            _ => return Mat::IDENTITY,
        };
        let x = unit(sub(xref, { let d = dot(xref, z); [z[0] * d, z[1] * d, z[2] * d] })).unwrap_or_else(|| if z[0].abs() < 0.9 { unit(cross([0.0, 1.0, 0.0], z)).unwrap_or([1.0, 0.0, 0.0]) } else { unit(cross(z, [0.0, 0.0, 1.0])).unwrap_or([0.0, 1.0, 0.0]) });
        Mat { x, y: cross(z, x), z, t }
    }

    /// An object placement: an IfcLocalPlacement chain to the world.
    pub fn placement(&self, id: Option<u64>) -> Mat {
        let mut out = Mat::IDENTITY;
        let mut cur = id;
        let mut seen = HashSet::new();
        while let Some(i) = cur {
            if !seen.insert(i) || seen.len() > 64 {
                break;
            }
            let Some(e) = self.get(i) else { break };
            let local = match e.name.as_str() {
                "IFCLOCALPLACEMENT" => e.ref_at(1).map(|r| self.axis_placement(r)).unwrap_or(Mat::IDENTITY),
                // IfcLinearPlacement(PlacementRelTo, RelativePlacement, CartesianPosition)
                "IFCLINEARPLACEMENT" => e.ref_at(2).map(|r| self.axis_placement(r)).unwrap_or(Mat::IDENTITY),
                _ => Mat::IDENTITY,
            };
            out = local.then_of(&out);
            cur = e.ref_at(0);
        }
        out
    }
}

// ── Alignments ──────────────────────────────────────────────────────────────

/// An IfcAlignment in local metres (placement applied, not yet the map
/// conversion).
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub id: u64,
    pub name: String,
    pub global_id: Option<String>,
    /// Station at the start (m).
    pub start: f64,
    pub h: Vec<HSeg>,
    pub v: Vec<VSeg>,
    /// What was approximated.
    pub warnings: Vec<String>,
}

impl Axis {
    pub fn length(&self) -> f64 {
        self.h.iter().map(|s| s.length).sum()
    }
}

/// The objects nested under `id` (IfcRelNests, in order).
fn nested<'a>(m: &'a StepModel, id: u64) -> Vec<&'a Entity> {
    m.referencing_of_type(id, "IFCRELNESTS").into_iter().filter(|r| r.ref_at(4) == Some(id)).flat_map(|r| r.refs_at(5)).filter_map(|i| m.get(i)).collect()
}

/// The design parameters of the segments nested under a layout.
fn segments<'a>(m: &'a StepModel, layout: u64) -> Vec<&'a Entity> {
    nested(m, layout).into_iter().filter(|e| e.name == "IFCALIGNMENTSEGMENT").filter_map(|s| s.ref_at(7).and_then(|r| m.get(r))).collect()
}

/// The single values of an object's property sets: (set, name, value).
pub fn properties(m: &StepModel, id: u64) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut sets: Vec<u64> = m.referencing_of_type(id, "IFCRELDEFINESBYPROPERTIES").into_iter().filter(|r| r.refs_at(4).contains(&id)).flat_map(|r| r.refs_at(5)).collect();
    // The type's sets, for what the occurrence does not say itself.
    let types: Vec<u64> = m.referencing_of_type(id, "IFCRELDEFINESBYTYPE").into_iter().filter(|r| r.refs_at(4).contains(&id)).filter_map(|r| r.ref_at(5)).collect();
    let own = sets.len();
    for t in types {
        if let Some(te) = m.get(t) {
            sets.extend(te.refs_at(5));
        }
    }
    let mut seen = HashSet::new();
    for (k, s) in sets.into_iter().enumerate() {
        let Some(set) = m.get(s) else { continue };
        let set_name = set.str_at(2).unwrap_or("").to_string();
        let props = match set.name.as_str() {
            "IFCPROPERTYSET" => set.refs_at(4),
            "IFCELEMENTQUANTITY" => set.refs_at(5),
            _ => continue,
        };
        for p in props.into_iter().filter_map(|p| m.get(p)) {
            let Some(name) = p.str_at(0) else { continue };
            let value = match p.name.as_str() {
                "IFCPROPERTYSINGLEVALUE" => p.arg(2).and_then(value_text),
                "IFCPROPERTYENUMERATEDVALUE" | "IFCPROPERTYLISTVALUE" => p.list_at(2).map(|l| l.iter().filter_map(value_text).collect::<Vec<_>>().join(";")),
                n if n.starts_with("IFCQUANTITY") => p.arg(3).and_then(value_text),
                _ => None,
            };
            let Some(value) = value.filter(|v| !v.is_empty()) else { continue };
            // The occurrence wins over its type.
            if k >= own && seen.contains(&(set_name.clone(), name.to_string())) {
                continue;
            }
            seen.insert((set_name.clone(), name.to_string()));
            out.push((set_name.clone(), name.to_string(), value));
        }
    }
    out
}

/// A property value as text: numbers plainly, booleans as true/false.
pub fn value_text(v: &Value) -> Option<String> {
    match v.inner() {
        Value::Str(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Real(r) => Some(if r.fract() == 0.0 && r.abs() < 1e15 { format!("{}", *r as i64) } else { format!("{r}") }),
        Value::Enum(e) => Some(match e.as_str() {
            "T" => "true".into(),
            "F" => "false".into(),
            other => other.to_string(),
        }),
        _ => None,
    }
}

impl Reader<'_> {
    fn horizontal(&self, e: &Entity, place: &Mat, warnings: &mut Vec<String>) -> Option<HSeg> {
        // IfcAlignmentHorizontalSegment(StartTag, EndTag, StartPoint, StartDirection,
        // StartRadiusOfCurvature, EndRadiusOfCurvature, SegmentLength, GravityCenterLineHeight, PredefinedType)
        let p = self.point(e.ref_at(2)?)?;
        let p = place.apply(p);
        let k = self.u.length;
        let dir = e.f64_at(3)? * self.u.angle + place.plan_rotation();
        let r0 = e.f64_at(4).unwrap_or(0.0) * k;
        let r1 = e.f64_at(5).unwrap_or(r0 / k) * k;
        let len = e.f64_at(6)?.abs() * k;
        let kind = e.enum_at(8).unwrap_or("LINE");
        let (radius, end_radius) = match kind {
            "LINE" => (0.0, 0.0),
            "CIRCULARARC" => (r0, r0),
            "CLOTHOID" => (r0, r1),
            other => {
                warnings.push(format!("{other} → clothoid"));
                (r0, r1)
            }
        };
        Some(HSeg { start: [p[0], p[1]], dir, radius, end_radius, length: len })
    }

    fn vertical(&self, e: &Entity, warnings: &mut Vec<String>) -> Option<VSeg> {
        // IfcAlignmentVerticalSegment(StartTag, EndTag, StartDistAlong, HorizontalLength,
        // StartHeight, StartGradient, EndGradient, RadiusOfCurvature, PredefinedType)
        let k = self.u.length;
        let (start, length, z) = (e.f64_at(2)? * k, e.f64_at(3)?.abs() * k, e.f64_at(4)? * k);
        let g0 = e.f64_at(5).unwrap_or(0.0);
        let g1 = e.f64_at(6).unwrap_or(g0);
        let kind = match e.enum_at(8).unwrap_or("CONSTANTGRADIENT") {
            "CONSTANTGRADIENT" => VKind::Gradient,
            "PARABOLICARC" => VKind::Parabola,
            other => {
                warnings.push(format!("{other} → parabola"));
                VKind::Parabola
            }
        };
        Some(VSeg { kind, start, length, z, g0, g1: if kind == VKind::Gradient { g0 } else { g1 } })
    }

    /// The IfcAlignments with their horizontal and vertical layouts.
    pub fn alignments(&self) -> Vec<Axis> {
        let m = self.m;
        let mut out = Vec::new();
        for a in m.by_type("IFCALIGNMENT") {
            let place = self.placement(a.ref_at(5));
            let mut warnings = Vec::new();
            let mut h = Vec::new();
            let mut v = Vec::new();
            let mut start = None;
            for layout in nested(m, a.id) {
                match layout.name.as_str() {
                    "IFCALIGNMENTHORIZONTAL" => h.extend(segments(m, layout.id).into_iter().filter_map(|s| self.horizontal(s, &place, &mut warnings))),
                    "IFCALIGNMENTVERTICAL" => v.extend(segments(m, layout.id).into_iter().filter_map(|s| self.vertical(s, &mut warnings))),
                    "IFCREFERENT" if start.is_none() => {
                        start = properties(m, layout.id).into_iter().find(|(s, n, _)| s == "Pset_Stationing" && n == "Station").and_then(|(_, _, v)| v.parse::<f64>().ok()).map(|st| {
                            // Less the referent's distance along the alignment.
                            let along = layout.ref_at(5).and_then(|p| m.get(p)).filter(|p| p.name == "IFCLINEARPLACEMENT").and_then(|p| p.ref_at(1)).and_then(|r| m.get(r)).and_then(|l| l.ref_at(0)).and_then(|r| m.get(r)).and_then(|d| d.f64_at(0)).unwrap_or(0.0);
                            (st - along) * self.u.length
                        });
                    }
                    _ => {}
                }
            }
            h.retain(|s| s.length > 1e-9);
            v.retain(|s| s.length > 1e-9);
            if h.is_empty() {
                continue;
            }
            warnings.sort();
            warnings.dedup();
            out.push(Axis { id: a.id, name: a.str_at(2).unwrap_or("").to_string(), global_id: a.str_at(0).map(String::from), start: start.unwrap_or(0.0), h, v, warnings });
        }
        out
    }
}

/// The pieces of an axis from its horizontal segments.
pub fn segs(h: &[HSeg]) -> Vec<Seg> {
    h.iter()
        .map(|s| {
            let (sn, cs) = s.dir.sin_cos();
            if s.is_clothoid() {
                let k = |r: f64| if r == 0.0 { 0.0 } else { 1.0 / r };
                Seg::Spiral { a: s.start, dir: s.dir, len: s.length, k0: k(s.radius), k1: k(s.end_radius) }
            } else if s.is_line() {
                Seg::Line { a: s.start, b: [s.start[0] + s.length * cs, s.start[1] + s.length * sn] }
            } else {
                let sign = s.radius.signum();
                let r = s.radius.abs();
                let a0 = s.dir - sign * FRAC_PI_2;
                Seg::Arc { c: [s.start[0] - r * a0.cos(), s.start[1] - r * a0.sin()], r, a0: a0.rem_euclid(TAU), sweep: sign * s.length / r }
            }
        })
        .collect()
}

/// A piece of axis through a similarity: points by `to`, directions turned
/// by `rot`, lengths times `scale`.
pub fn map_seg(s: &Seg, to: &dyn Fn(XY) -> XY, rot: f64, scale: f64) -> Seg {
    match s {
        Seg::Line { a, b } => Seg::Line { a: to(*a), b: to(*b) },
        Seg::Arc { c, r, a0, sweep } => Seg::Arc { c: to(*c), r: r * scale, a0: (a0 + rot).rem_euclid(TAU), sweep: *sweep },
        Seg::Spiral { a, dir, len, k0, k1 } => Seg::Spiral { a: to(*a), dir: dir + rot, len: len * scale, k0: k0 / scale, k1: k1 / scale },
    }
}

/// The PVIs of a vertical layout: the ends, the tangent intersection of
/// every vertical curve (with the curve's length) and every kink between
/// two grades. Distances from the start of the horizontal alignment.
pub fn pvis(v: &[VSeg]) -> Vec<Pvi> {
    let Some(first) = v.first() else { return Vec::new() };
    let mut out = vec![Pvi { s: first.start, z: first.z, l: 0.0 }];
    let mut grade: Option<f64> = None;
    for s in v {
        match s.kind {
            VKind::Gradient => {
                if let Some(g) = grade {
                    if (g - s.g0).abs() > 1e-9 && (out.last().unwrap().s - s.start).abs() > 1e-6 {
                        out.push(Pvi { s: s.start, z: s.z, l: 0.0 });
                    }
                }
                grade = Some(s.g0);
            }
            VKind::Parabola => {
                let half = s.length / 2.0;
                out.push(Pvi { s: s.start + half, z: s.z + s.g0 * half, l: s.length });
                grade = Some(s.g1);
            }
        }
    }
    let last = v.last().unwrap();
    let end = last.start + last.length;
    out.push(Pvi { s: end, z: last.end_z(), l: 0.0 });
    // A curve at either end has no tangents to meet: it starts at the end.
    out.dedup_by(|b, a| (b.s - a.s).abs() < 1e-6);
    if let Some(p) = out.first_mut() {
        p.l = 0.0;
    }
    if let Some(p) = out.last_mut() {
        p.l = 0.0;
    }
    out
}

// ── Geometry ────────────────────────────────────────────────────────────────

/// Triangles of one colour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Part {
    pub tris: Vec<[P3; 3]>,
    pub colour: Option<(u8, u8, u8)>,
}

/// An element with geometry, in local metres (placement applied).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Product {
    pub id: u64,
    /// IFC class as written ("IFCWALL").
    pub class: String,
    pub global_id: Option<String>,
    pub name: Option<String>,
    pub object_type: Option<String>,
    pub predefined: Option<String>,
    pub parts: Vec<Part>,
    /// (property set, property, value).
    pub props: Vec<(String, String, String)>,
}

impl Product {
    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.tris.len()).sum()
    }

    /// A property by set and name.
    pub fn prop(&self, set: &str, name: &str) -> Option<&str> {
        self.props.iter().find(|(s, n, _)| s == set && n == name).map(|(_, _, v)| v.as_str())
    }

    /// Terrain: an IfcGeographicElement TERRAIN, an IfcSite with a body, or
    /// a TIN item.
    pub fn is_terrain(&self, tin_item: bool) -> bool {
        tin_item || self.class == "IFCSITE" || (self.class == "IFCGEOGRAPHICELEMENT" && (self.predefined.as_deref() == Some("TERRAIN") || self.object_type.as_deref().is_some_and(|o| o.eq_ignore_ascii_case("terrain"))))
    }
}

/// Triangulates a planar polygon with holes in its own plane.
pub fn polygon(outer: &[P3], holes: &[Vec<P3>]) -> Vec<[P3; 3]> {
    let mut outer: Vec<P3> = outer.to_vec();
    if outer.len() > 1 && outer.first() == outer.last() {
        outer.pop();
    }
    if outer.len() < 3 {
        return Vec::new();
    }
    if outer.len() == 3 && holes.is_empty() {
        return vec![[outer[0], outer[1], outer[2]]];
    }
    // Newell's normal picks the projection plane.
    let mut n = [0.0; 3];
    for i in 0..outer.len() {
        let (a, b) = (outer[i], outer[(i + 1) % outer.len()]);
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let drop = (0..3).max_by(|&i, &j| n[i].abs().total_cmp(&n[j].abs())).unwrap_or(2);
    let (i, j) = match drop {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let mut all = outer.clone();
    let mut flat: Vec<f64> = outer.iter().flat_map(|p| [p[i], p[j]]).collect();
    let mut hole_idx = Vec::new();
    for h in holes {
        let mut h = h.clone();
        if h.len() > 1 && h.first() == h.last() {
            h.pop();
        }
        if h.len() < 3 {
            continue;
        }
        hole_idx.push(all.len());
        flat.extend(h.iter().flat_map(|p| [p[i], p[j]]));
        all.extend(h);
    }
    let fan = || (1..outer.len() - 1).map(|k| [outer[0], outer[k], outer[k + 1]]).collect::<Vec<_>>();
    let Ok(idx) = earcutr::earcut(&flat, &hole_idx, 2) else { return fan() };
    if idx.is_empty() {
        return fan();
    }
    // earcut's winding follows the projection; keep the polygon's own.
    let flip = n[drop] < 0.0;
    idx.chunks_exact(3)
        .map(|t| {
            let tri = [all[t[0]], all[t[1]], all[t[2]]];
            let tn = cross(sub(tri[1], tri[0]), sub(tri[2], tri[0]));
            if (tn[drop] < 0.0) != flip { [tri[0], tri[2], tri[1]] } else { tri }
        })
        .collect()
}

/// What an item turned into, before the product's placement.
#[derive(Default)]
struct Collector {
    parts: Vec<Part>,
    tin: bool,
    /// Representation items that could not be read, per IFC class.
    skipped: BTreeMap<String, usize>,
    /// Items read approximately, per kind.
    approximated: BTreeMap<String, usize>,
}

impl Collector {
    fn add(&mut self, tris: Vec<[P3; 3]>, colour: Option<(u8, u8, u8)>, at: &Mat) {
        if tris.is_empty() {
            return;
        }
        let tris = tris.into_iter().map(|t| t.map(|p| at.apply(p)));
        match self.parts.iter_mut().find(|p| p.colour == colour) {
            Some(p) => p.tris.extend(tris),
            None => self.parts.push(Part { tris: tris.collect(), colour }),
        }
    }
}

fn byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl Reader<'_> {
    /// The surface colour of a styled item (IfcStyledItem → IfcSurfaceStyle → shading).
    fn colour_of(&self, item: u64) -> Option<(u8, u8, u8)> {
        let m = self.m;
        for st in m.referencing_of_type(item, "IFCSTYLEDITEM") {
            let mut styles = st.refs_at(1);
            // IFC2X3: IfcPresentationStyleAssignment around the styles.
            let mut k = 0;
            while k < styles.len() {
                if let Some(e) = m.get(styles[k]) {
                    if e.name == "IFCPRESENTATIONSTYLEASSIGNMENT" {
                        styles.extend(e.refs_at(0));
                    }
                }
                k += 1;
            }
            for s in styles.into_iter().filter_map(|s| m.get(s)).filter(|s| s.name == "IFCSURFACESTYLE") {
                for el in s.refs_at(2).into_iter().filter_map(|r| m.get(r)) {
                    if matches!(el.name.as_str(), "IFCSURFACESTYLESHADING" | "IFCSURFACESTYLERENDERING") {
                        if let Some(c) = el.ref_at(0).and_then(|r| m.get(r)) {
                            return Some((byte(c.f64_at(1)?), byte(c.f64_at(2)?), byte(c.f64_at(3)?)));
                        }
                    }
                }
            }
        }
        None
    }

    fn point_list(&self, id: Option<u64>) -> Vec<P3> {
        let Some(e) = id.and_then(|i| self.get(i)) else { return Vec::new() };
        let k = self.u.length;
        e.list_at(0)
            .map(|l| {
                l.iter()
                    .map(|p| {
                        let c = Self::nums(Some(p));
                        [c.first().copied().unwrap_or(0.0) * k, c.get(1).copied().unwrap_or(0.0) * k, c.get(2).copied().unwrap_or(0.0) * k]
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 1-based indices, through PnIndex when there is one.
    fn index(list: &[Value], pn: &[usize], n: usize) -> Vec<usize> {
        list.iter()
            .filter_map(Value::as_i64)
            .filter_map(|i| {
                let i = (i as usize).checked_sub(1)?;
                let i = if pn.is_empty() { i } else { pn.get(i)?.checked_sub(1)? };
                (i < n).then_some(i)
            })
            .collect()
    }

    fn pn_index(e: &Entity, at: usize) -> Vec<usize> {
        e.list_at(at).map(|l| l.iter().filter_map(Value::as_i64).map(|i| i.max(0) as usize).collect()).unwrap_or_default()
    }

    /// A tessellated face set: its triangles, coloured per face by an
    /// IfcIndexedColourMap when it has one.
    fn face_set(&self, e: &Entity, colour: Option<(u8, u8, u8)>, at: &Mat, out: &mut Collector) {
        let pts = self.point_list(e.ref_at(0));
        let n = pts.len();
        let mut faces: Vec<Vec<[P3; 3]>> = Vec::new();
        if e.name == "IFCPOLYGONALFACESET" {
            let pn = Self::pn_index(e, 3);
            for f in e.refs_at(2).into_iter().filter_map(|f| self.get(f)) {
                let outer: Vec<P3> = Self::index(f.list_at(0).unwrap_or(&[]), &pn, n).into_iter().map(|i| pts[i]).collect();
                let holes: Vec<Vec<P3>> = if f.name == "IFCINDEXEDPOLYGONALFACEWITHVOIDS" {
                    f.list_at(1).unwrap_or(&[]).iter().filter_map(Value::as_list).map(|h| Self::index(h, &pn, n).into_iter().map(|i| pts[i]).collect()).collect()
                } else {
                    Vec::new()
                };
                faces.push(polygon(&outer, &holes));
            }
        } else {
            let pn = Self::pn_index(e, 4);
            for t in e.list_at(3).unwrap_or(&[]).iter().filter_map(Value::as_list) {
                let i = Self::index(t, &pn, n);
                faces.push(if i.len() == 3 { vec![[pts[i[0]], pts[i[1]], pts[i[2]]]] } else { Vec::new() });
            }
        }
        // IfcIndexedColourMap(MappedTo, Opacity, Colours, ColourIndex)
        let map = self.m.referencing_of_type(e.id, "IFCINDEXEDCOLOURMAP").into_iter().find(|c| c.ref_at(0) == Some(e.id));
        if let Some(map) = map {
            let list: Vec<(u8, u8, u8)> = map
                .ref_at(2)
                .and_then(|r| self.get(r))
                .and_then(|c| c.list_at(0))
                .map(|l| l.iter().map(|c| { let v = Self::nums(Some(c)); (byte(v.first().copied().unwrap_or(0.5)), byte(v.get(1).copied().unwrap_or(0.5)), byte(v.get(2).copied().unwrap_or(0.5))) }).collect())
                .unwrap_or_default();
            let index: Vec<usize> = Self::pn_index(map, 3);
            if index.len() == faces.len() && !list.is_empty() {
                for (f, c) in faces.into_iter().zip(index) {
                    out.add(f, c.checked_sub(1).and_then(|c| list.get(c)).copied().or(colour), at);
                }
                return;
            }
        }
        out.add(faces.into_iter().flatten().collect(), colour, at);
    }

    /// The faces of shells (IfcClosedShell, IfcOpenShell, IfcConnectedFaceSet).
    fn shell(&self, id: u64, colour: Option<(u8, u8, u8)>, at: &Mat, out: &mut Collector) {
        let Some(shell) = self.get(id) else { return };
        let mut tris = Vec::new();
        for f in shell.refs_at(0).into_iter().filter_map(|f| self.get(f)) {
            let mut outer: Option<Vec<P3>> = None;
            let mut holes = Vec::new();
            for b in f.refs_at(0).into_iter().filter_map(|b| self.get(b)) {
                let Some(lp) = b.ref_at(0).and_then(|r| self.get(r)) else { continue };
                if lp.name != "IFCPOLYLOOP" {
                    *out.skipped.entry(lp.name.clone()).or_insert(0) += 1;
                    continue;
                }
                let mut ring: Vec<P3> = lp.refs_at(0).into_iter().filter_map(|p| self.point(p)).collect();
                if b.arg(1).and_then(Value::as_bool) == Some(false) {
                    ring.reverse();
                }
                if b.name == "IFCFACEOUTERBOUND" && outer.is_none() {
                    outer = Some(ring);
                } else {
                    holes.push(ring);
                }
            }
            let outer = match outer {
                Some(o) => o,
                None if !holes.is_empty() => holes.remove(0),
                None => continue,
            };
            let mut t = polygon(&outer, &holes);
            if f.name == "IFCFACESURFACE" && f.arg(2).and_then(Value::as_bool) == Some(false) {
                t.iter_mut().for_each(|x| x.swap(1, 2));
            }
            tris.extend(t);
        }
        out.add(tris, colour, at);
    }

    /// A closed profile curve in its plane (metres): polyline, indexed poly
    /// curve (arcs by their three points) or circle.
    fn curve_points(&self, id: Option<u64>) -> Option<Vec<P3>> {
        let e = self.get(id?)?;
        match e.name.as_str() {
            "IFCPOLYLINE" => Some(e.refs_at(0).into_iter().filter_map(|p| self.point(p)).collect()),
            "IFCINDEXEDPOLYCURVE" => {
                let pts = self.point_list(e.ref_at(0));
                let Some(segs) = e.list_at(1) else { return Some(pts) };
                let mut out: Vec<P3> = Vec::new();
                for s in segs {
                    let idx: Vec<usize> = s.inner().as_list().unwrap_or(&[]).iter().filter_map(Value::as_i64).filter_map(|i| (i as usize).checked_sub(1)).filter(|i| *i < pts.len()).collect();
                    for i in idx {
                        if out.last() != Some(&pts[i]) {
                            out.push(pts[i]);
                        }
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// The outline (and holes) of a profile definition, in its plane.
    fn profile(&self, id: Option<u64>) -> Result<(Vec<P3>, Vec<Vec<P3>>), String> {
        let e = self.get(id.ok_or("geen profiel")?).ok_or("geen profiel")?;
        let pos = |at: usize| e.ref_at(at).map(|r| self.axis_placement(r)).unwrap_or(Mat::IDENTITY);
        let k = self.u.length;
        match e.name.as_str() {
            "IFCRECTANGLEPROFILEDEF" => {
                let (x, y) = (e.f64_at(3).ok_or("XDim")? * k / 2.0, e.f64_at(4).ok_or("YDim")? * k / 2.0);
                let p = pos(2);
                Ok(([[-x, -y, 0.0], [x, -y, 0.0], [x, y, 0.0], [-x, y, 0.0]].map(|q| p.apply(q)).to_vec(), Vec::new()))
            }
            "IFCCIRCLEPROFILEDEF" | "IFCCIRCLEHOLLOWPROFILEDEF" => {
                let r = e.f64_at(3).ok_or("Radius")? * k;
                let p = pos(2);
                let ring = |r: f64| (0..24).map(|i| { let a = TAU * i as f64 / 24.0; p.apply([r * a.cos(), r * a.sin(), 0.0]) }).collect::<Vec<_>>();
                let holes = match (e.name.as_str(), e.f64_at(4)) {
                    ("IFCCIRCLEHOLLOWPROFILEDEF", Some(t)) if t * k < r => vec![ring(r - t * k)],
                    _ => Vec::new(),
                };
                Ok((ring(r), holes))
            }
            "IFCARBITRARYCLOSEDPROFILEDEF" | "IFCARBITRARYPROFILEDEFWITHVOIDS" => {
                let outer = self.curve_points(e.ref_at(2)).ok_or_else(|| self.get(e.ref_at(2).unwrap_or(0)).map(|c| c.name.clone()).unwrap_or_else(|| "curve".into()))?;
                let holes = if e.name == "IFCARBITRARYPROFILEDEFWITHVOIDS" { e.refs_at(3).into_iter().filter_map(|c| self.curve_points(Some(c))).collect() } else { Vec::new() };
                Ok((outer, holes))
            }
            other => Err(other.to_string()),
        }
    }

    /// An extruded solid: both caps and the sides.
    fn extrusion(&self, e: &Entity) -> Result<Vec<[P3; 3]>, String> {
        let (outer, holes) = self.profile(e.ref_at(0))?;
        let place = e.ref_at(1).map(|r| self.axis_placement(r)).unwrap_or(Mat::IDENTITY);
        let d = self.direction(e.ref_at(2)).unwrap_or([0.0, 0.0, 1.0]);
        let depth = e.f64_at(3).ok_or("Depth")? * self.u.length;
        let v = [d[0] * depth, d[1] * depth, d[2] * depth];
        let top = |p: P3| [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
        let mut tris = Vec::new();
        for t in polygon(&outer, &holes) {
            tris.push([t[0], t[2], t[1]]);
            tris.push(t.map(top));
        }
        for ring in std::iter::once(&outer).chain(holes.iter()) {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if a == b {
                    continue;
                }
                tris.push([a, b, top(b)]);
                tris.push([a, top(b), top(a)]);
            }
        }
        Ok(tris.into_iter().map(|t| t.map(|p| place.apply(p))).collect())
    }

    /// A swept disk along a polyline: a tube of 12 sides, ends closed.
    fn swept_disk(&self, e: &Entity) -> Result<Vec<[P3; 3]>, String> {
        let line = self.curve_points(e.ref_at(0)).ok_or_else(|| self.get(e.ref_at(0).unwrap_or(0)).map(|c| c.name.clone()).unwrap_or_else(|| "curve".into()))?;
        let r = e.f64_at(1).ok_or("Radius")? * self.u.length;
        let mut tris = Vec::new();
        let n = 12;
        for w in line.windows(2) {
            let Some(t) = unit(sub(w[1], w[0])) else { continue };
            let a = unit(cross(t, if t[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] })).unwrap_or([1.0, 0.0, 0.0]);
            let b = cross(t, a);
            let ring = |c: P3| (0..n).map(|i| { let f = TAU * i as f64 / n as f64; [0, 1, 2].map(|k| c[k] + r * (f.cos() * a[k] + f.sin() * b[k])) }).collect::<Vec<_>>();
            let (r0, r1) = (ring(w[0]), ring(w[1]));
            for i in 0..n {
                let j = (i + 1) % n;
                tris.push([r0[i], r0[j], r1[j]]);
                tris.push([r0[i], r1[j], r1[i]]);
            }
            for i in 1..n - 1 {
                tris.push([w[0], r0[i + 1], r0[i]]);
                tris.push([w[1], r1[i], r1[i + 1]]);
            }
        }
        Ok(tris)
    }

    /// One representation item, placed by `at`.
    fn item(&self, id: u64, at: &Mat, inherited: Option<(u8, u8, u8)>, depth: usize, out: &mut Collector) {
        let Some(e) = self.get(id) else { return };
        let colour = self.colour_of(id).or(inherited);
        match e.name.as_str() {
            "IFCTRIANGULATEDIRREGULARNETWORK" => {
                out.tin = true;
                self.face_set(e, colour, at, out);
            }
            "IFCTRIANGULATEDFACESET" | "IFCPOLYGONALFACESET" => self.face_set(e, colour, at, out),
            "IFCFACETEDBREP" | "IFCFACETEDBREPWITHVOIDS" => {
                for s in std::iter::once(e.ref_at(0)).flatten().chain(e.refs_at(1)) {
                    self.shell(s, colour, at, out);
                }
            }
            "IFCSHELLBASEDSURFACEMODEL" | "IFCFACEBASEDSURFACEMODEL" => {
                for s in e.refs_at(0) {
                    self.shell(s, colour, at, out);
                }
            }
            "IFCMAPPEDITEM" if depth < 8 => {
                // IfcMappedItem(MappingSource → IfcRepresentationMap(MappingOrigin, MappedRepresentation), MappingTarget)
                let Some(map) = e.ref_at(0).and_then(|r| self.get(r)) else { return };
                let origin = map.ref_at(0).map(|r| self.axis_placement(r)).unwrap_or(Mat::IDENTITY);
                let target = e.ref_at(1).and_then(|r| self.get(r)).map(|t| self.operator(t)).unwrap_or(Mat::IDENTITY);
                let place = at.then_of(&target.then_of(&origin));
                if let Some(rep) = map.ref_at(1).and_then(|r| self.get(r)) {
                    for i in rep.refs_at(3) {
                        self.item(i, &place, colour, depth + 1, out);
                    }
                }
            }
            "IFCEXTRUDEDAREASOLID" => match self.extrusion(e) {
                Ok(t) => out.add(t, colour, at),
                Err(why) => *out.skipped.entry(format!("IFCEXTRUDEDAREASOLID ({why})")).or_insert(0) += 1,
            },
            "IFCSWEPTDISKSOLID" => match self.swept_disk(e) {
                Ok(t) => out.add(t, colour, at),
                Err(why) => *out.skipped.entry(format!("IFCSWEPTDISKSOLID ({why})")).or_insert(0) += 1,
            },
            "IFCBOOLEANRESULT" | "IFCBOOLEANCLIPPINGRESULT" if depth < 8 => {
                *out.approximated.entry(format!("{} ({})", e.name, crate::lang::tr("alleen eerste operand", "first operand only"))).or_insert(0) += 1;
                if let Some(first) = e.ref_at(1) {
                    self.item(first, at, colour, depth + 1, out);
                }
            }
            other if is_curve_like(other) => {}
            other => *out.skipped.entry(other.to_string()).or_insert(0) += 1,
        }
    }

    /// IfcCartesianTransformationOperator3D (uniform or not) as a transform.
    fn operator(&self, e: &Entity) -> Mat {
        // (Axis1, Axis2, LocalOrigin, Scale, Axis3[, Scale2, Scale3])
        let z = self.direction(e.ref_at(4)).unwrap_or([0.0, 0.0, 1.0]);
        let x0 = self.direction(e.ref_at(0)).unwrap_or([1.0, 0.0, 0.0]);
        let x = unit(sub(x0, { let d = dot(x0, z); [z[0] * d, z[1] * d, z[2] * d] })).unwrap_or([1.0, 0.0, 0.0]);
        let y = cross(z, x);
        let s = e.f64_at(3).unwrap_or(1.0);
        let (s2, s3) = (e.f64_at(5).unwrap_or(s), e.f64_at(6).unwrap_or(s));
        let t = e.ref_at(2).and_then(|p| self.point(p)).unwrap_or([0.0; 3]);
        Mat { x: x.map(|c| c * s), y: y.map(|c| c * s2), z: z.map(|c| c * s3), t }
    }
}

/// Curves, points and texts: representation items that are not meant to
/// have surfaces (axes, footprints, annotation).
fn is_curve_like(name: &str) -> bool {
    name.ends_with("CURVE") || name.ends_with("POINT") || name.starts_with("IFCTEXTLITERAL") || name.starts_with("IFCANNOTATION")
        || matches!(name, "IFCPOLYLINE" | "IFCLINE" | "IFCCIRCLE" | "IFCELLIPSE" | "IFCGEOMETRICSET" | "IFCGEOMETRICCURVESET" | "IFCPOINTBYDISTANCEEXPRESSION" | "IFCCARTESIANPOINTLIST2D" | "IFCCARTESIANPOINTLIST3D")
}

/// Everything read from a file.
#[derive(Debug, Default)]
pub struct Content {
    pub schema: Option<String>,
    pub units: Option<Units>,
    pub map: Option<MapConversion>,
    pub axes: Vec<Axis>,
    /// Elements with geometry; `terrain` marks the TINs.
    pub products: Vec<(Product, bool)>,
    /// Representation items not read, per IFC class.
    pub skipped: BTreeMap<String, usize>,
    pub approximated: BTreeMap<String, usize>,
    /// Products with a representation but no triangles.
    pub without_mesh: usize,
}

/// Whether an entity is a product: a 22-character GlobalId and a
/// product definition shape in Representation (attribute 7).
fn is_product(m: &StepModel, e: &Entity) -> bool {
    e.args.len() >= 7 && e.str_at(0).is_some_and(|g| g.len() == 22) && e.ref_at(6).and_then(|r| m.get(r)).is_some_and(|r| r.name == "IFCPRODUCTDEFINITIONSHAPE")
}

/// Reads alignments, terrain and elements.
pub fn read(m: &StepModel) -> Content {
    let r = Reader::new(m);
    let mut c = Content { schema: m.schema(), units: Some(r.u), map: map_conversion(m, &r.u), axes: r.alignments(), ..Content::default() };
    for e in m.entities() {
        if e.name == "IFCALIGNMENT" || !is_product(m, e) {
            continue;
        }
        let place = r.placement(e.ref_at(5));
        let mut col = Collector::default();
        let shape = m.get(e.ref_at(6).unwrap_or(0));
        let reps: Vec<&Entity> = shape.map(|s| s.refs_at(2)).unwrap_or_default().into_iter().filter_map(|i| m.get(i)).filter(|rep| rep.name == "IFCSHAPEREPRESENTATION" || rep.name == "IFCTOPOLOGYREPRESENTATION").collect();
        // The body; without one, any representation of surfaces or solids.
        let body: Vec<&&Entity> = reps.iter().filter(|rep| rep.str_at(1).is_some_and(|i| i.eq_ignore_ascii_case("Body") || i.eq_ignore_ascii_case("Facetation"))).collect();
        let chosen: Vec<&&Entity> = if body.is_empty() {
            reps.iter().filter(|rep| !rep.str_at(1).is_some_and(|i| ["Axis", "FootPrint", "Box", "Annotation", "Profile", "Reference", "Clearance", "Lighting", "CoG", "Surface"].iter().any(|k| i.eq_ignore_ascii_case(k)))).collect()
        } else {
            body
        };
        for rep in chosen {
            for i in rep.refs_at(3) {
                r.item(i, &place, None, 0, &mut col);
            }
        }
        for (k, v) in col.skipped {
            *c.skipped.entry(k).or_insert(0) += v;
        }
        for (k, v) in col.approximated {
            *c.approximated.entry(k).or_insert(0) += v;
        }
        if col.parts.iter().all(|p| p.tris.is_empty()) {
            c.without_mesh += 1;
            continue;
        }
        let predefined = e.args.iter().skip(7).rev().find_map(|a| a.as_enum().filter(|v| !matches!(*v, "T" | "F" | "U" | "ELEMENT" | "COMPLEX" | "PARTIAL")).map(String::from));
        let p = Product {
            id: e.id,
            class: e.name.clone(),
            global_id: e.str_at(0).map(String::from),
            name: e.str_at(2).map(String::from).filter(|s| !s.is_empty()),
            object_type: e.str_at(4).map(String::from).filter(|s| !s.is_empty()),
            predefined,
            parts: col.parts,
            props: properties(m, e.id),
        };
        let terrain = p.is_terrain(col.tin);
        c.products.push((p, terrain));
    }
    c
}

/// The NLCS object (hoofdgroep, object) for an IFC class without an
/// Open Infra layer of its own.
pub fn nlcs_object(class: &str, predefined: Option<&str>) -> (&'static str, &'static str) {
    match class {
        "IFCBUILDING" | "IFCBUILDINGSTOREY" | "IFCSPACE" | "IFCROOF" | "IFCDOOR" | "IFCWINDOW" | "IFCSTAIR" | "IFCSTAIRFLIGHT" | "IFCCURTAINWALL" | "IFCCOVERING" => ("OG", "BEBOUWING"),
        "IFCPAVEMENT" | "IFCCOURSE" => ("VH", "GESLOTENVERHARDING"),
        "IFCKERB" => ("VH", "KANTOPSLUITING"),
        "IFCEARTHWORKSFILL" => ("GW", "OPHOGING"),
        "IFCEARTHWORKSCUT" => ("GW", "ONTGRAVINGSVAK"),
        "IFCEARTHWORKSELEMENT" | "IFCREINFORCEDSOIL" | "IFCGEOTECHNICALSTRATUM" | "IFCGEOSLICE" | "IFCGEOMODEL" => ("GW", "GROND"),
        "IFCGEOGRAPHICELEMENT" if predefined == Some("VEGETATION") => ("GR", "BEPLANTING"),
        "IFCGEOGRAPHICELEMENT" | "IFCSITE" => ("OG", "TERREIN"),
        "IFCPIPESEGMENT" | "IFCPIPEFITTING" | "IFCDISTRIBUTIONCHAMBERELEMENT" | "IFCFLOWSEGMENT" | "IFCFLOWFITTING" => ("RI", "OVERIG"),
        "IFCRAILING" | "IFCFENCE" => ("IE", "TERREINAFSCHEIDING"),
        "IFCSIGN" | "IFCSIGNAL" => ("IE", "BORD"),
        "IFCFURNISHINGELEMENT" | "IFCFURNITURE" | "IFCSYSTEMFURNITUREELEMENT" => ("IE", "MEUBILAIR"),
        _ => ("KW", "KUNSTWERK"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::as_profiel::{design_z, Alignment};
    use std::f64::consts::PI;

    fn export_sample() -> String {
        let doc = crate::ifc::tests::sample();
        let site = crate::ifc::tests::sample_site();
        let src = crate::ifc::Source { doc: &doc, f: 1.0, site: Some(&site), name: "voorbeeld.dwg".into() };
        let mut report = crate::ifc::Report::default();
        String::from_utf8(crate::ifc::export(&src, &crate::ifc::Options::default(), Vec::new(), &mut report).unwrap()).unwrap()
    }

    /// Our own export read back: the same axes, design profile and triangles.
    #[test]
    fn round_trip_of_our_own_export() {
        let text = export_sample();
        let m = StepModel::parse_str(&text).unwrap();
        let c = read(&m);
        assert_eq!(c.schema.as_deref(), Some("IFC4X3_ADD2"));
        let map = c.map.clone().unwrap();
        assert_eq!((map.eastings, map.northings, map.rotation, map.scale), (105000.0, 425000.0, 0.0, 1.0));
        assert_eq!(map.crs.as_deref(), Some("EPSG:28992"));
        // The axes: the same lengths and pieces as drawn.
        let doc = crate::ifc::tests::sample();
        let drawn: Vec<Alignment> = doc.model_space_entities().filter(|e| crate::civiel::data_value(e, "as").is_some() && crate::civiel::data_value(e, "spiraal1").is_some()).filter_map(Alignment::from_entity).collect();
        assert_eq!(c.axes.len(), 2);
        for al in &drawn {
            let a = c.axes.iter().find(|a| a.name == al.name).unwrap();
            assert!((a.length() - al.length()).abs() < 1e-6, "{} {} {}", al.name, a.length(), al.length());
            let back = Alignment { name: a.name.clone(), start: a.start, segs: segs(&a.h).iter().map(|s| map_seg(s, &|p| { let q = map.apply([p[0], p[1], 0.0]); [q[0] - 105000.0, q[1] - 425000.0] }, map.rotation, map.scale)).collect() };
            assert_eq!(back.segs.len(), al.segs.len());
            assert!((back.length() - al.length()).abs() < 1e-6);
            for s in [0.0, al.length() / 3.0, al.length()] {
                let (p, d) = al.at(s);
                let (q, e) = back.at(s);
                assert!((p[0] - q[0]).hypot(p[1] - q[1]) < 1e-5 && ((d - e + PI).rem_euclid(TAU) - PI).abs() < 1e-6, "{} at {s}: {p:?} {q:?}", al.name);
            }
            assert!(a.warnings.is_empty());
        }
        // The design profile of A1: PVIs 0:10, 100:12, 180:11 with a 40 m curve.
        let a1 = c.axes.iter().find(|a| a.name == "A1").unwrap();
        assert_eq!(a1.start, 0.0);
        let p = pvis(&a1.v);
        assert_eq!(p.len(), 3);
        for (got, want) in p.iter().zip([(0.0, 10.0, 0.0), (100.0, 12.0, 40.0), (180.0, 11.0, 0.0)]) {
            assert!((got.s - want.0).abs() < 1e-9 && (got.z - want.1).abs() < 1e-9 && (got.l - want.2).abs() < 1e-9, "{got:?}");
        }
        let original = crate::civiel::as_profiel::fit_curves(&[(0.0, 10.0), (100.0, 12.0), (180.0, 11.0)], 40.0);
        for s in [0.0, 85.0, 100.0, 117.0, 180.0] {
            assert!((design_z(&p, s) - design_z(&original, s)).abs() < 1e-9);
        }
        // Terrain and elements: every triangle comes back.
        let tin: usize = c.products.iter().filter(|(p, t)| *t && p.prop("OpenInfra_NLCS", "Laag").is_some_and(|l| l.contains("-OG-TRIANGULATIE-"))).map(|(p, _)| p.triangles()).sum();
        assert_eq!(tin, 2);
        let building = c.products.iter().find(|(p, _)| p.class == "IFCBUILDING").unwrap();
        assert_eq!(building.0.triangles(), 3);
        assert_eq!(building.0.parts.len(), 2, "two colours");
        assert_eq!(building.0.prop("OpenInfra_ObjectData", "identificatie"), Some("NL.IMBAG.Pand.0505100000012345"));
        // The photo terrain's colour map gives its two colours back.
        let photo = c.products.iter().find(|(p, _)| p.prop("OpenInfra_NLCS", "Laag").is_some_and(|l| l.contains("-OG-DTM-"))).unwrap();
        let mut colours: Vec<_> = photo.0.parts.iter().map(|p| p.colour).collect();
        colours.sort();
        assert_eq!(colours, vec![Some((90, 120, 60)), Some((120, 120, 120))]);
        // Local coordinates: metres from the base point (the TIN corner 0,-20,1).
        let all: Vec<P3> = c.products.iter().flat_map(|(p, _)| p.parts.iter().flat_map(|q| q.tris.iter().flatten().copied())).collect();
        assert!(all.iter().any(|q| (q[0] - 0.0).abs() < 1e-9 && (q[1] + 20.0).abs() < 1e-9 && (q[2] - 1.0).abs() < 1e-9));
        // Sewer pipes in the sample are curves only; the excavation is a mesh.
        assert!(c.products.iter().any(|(p, _)| p.class == "IFCEARTHWORKSCUT" && p.triangles() == 1));
        let total: usize = c.products.iter().map(|(p, _)| p.triangles()).sum();
        // TIN 2 + building 3 + strata 2×2 + photo terrain 2 + excavation 1.
        assert_eq!(total, 12, "{:?}", c.products.iter().map(|(p, _)| (p.class.clone(), p.triangles())).collect::<Vec<_>>());
    }

    const HEADER: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\nFILE_NAME('t.ifc','',(''),(''),'','','');\nFILE_SCHEMA(('IFC4X3_ADD2'));\nENDSEC;\nDATA;\n";

    fn step(body: &str) -> StepModel {
        StepModel::parse_str(&format!("{HEADER}{body}ENDSEC;\nEND-ISO-10303-21;\n")).unwrap()
    }

    #[test]
    fn units_placements_and_map_conversion() {
        let m = step(
            "#1=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n\
             #2=IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.);\n\
             #3=IFCMEASUREWITHUNIT(IFCPLANEANGLEMEASURE(0.0174532925199433),#2);\n\
             #4=IFCCONVERSIONBASEDUNIT(#5,.PLANEANGLEUNIT.,'DEGREE',#3);\n\
             #5=IFCDIMENSIONALEXPONENTS(0,0,0,0,0,0,0);\n\
             #6=IFCUNITASSIGNMENT((#1,#4));\n\
             #7=IFCPROJECT('0YvctVUKr0kugbFTf53O9L',$,'P',$,$,$,$,$,#6);\n\
             #10=IFCCARTESIANPOINT((1000.,2000.,0.));\n\
             #11=IFCDIRECTION((0.,1.,0.));\n\
             #12=IFCAXIS2PLACEMENT3D(#10,$,#11);\n\
             #13=IFCLOCALPLACEMENT($,#12);\n\
             #14=IFCCARTESIANPOINT((500.,0.,300.));\n\
             #15=IFCAXIS2PLACEMENT3D(#14,$,$);\n\
             #16=IFCLOCALPLACEMENT(#13,#15);\n\
             #20=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n\
             #21=IFCPROJECTEDCRS('EPSG:28992',$,$,$,$,$,#20);\n\
             #22=IFCMAPCONVERSION(#99,#21,155000.,463000.,2.5,0.,1.,0.9996);\n",
        );
        let u = units(&m);
        assert!((u.length - 0.001).abs() < 1e-15 && (u.angle - PI / 180.0).abs() < 1e-12);
        let r = Reader::new(&m);
        // The child at (0.5, 0, 0.3) in a parent turned 90° and moved to (1, 2).
        let p = r.placement(Some(16)).apply([0.0; 3]);
        assert!((p[0] - 1.0).abs() < 1e-12 && (p[1] - 2.5).abs() < 1e-12 && (p[2] - 0.3).abs() < 1e-12, "{p:?}");
        let mc = map_conversion(&m, &u).unwrap();
        assert!((mc.rotation - FRAC_PI_2).abs() < 1e-12 && mc.scale == 0.9996 && mc.eastings == 155000.0);
        let q = mc.apply([10.0, 0.0, 1.0]);
        assert!((q[0] - 155000.0).abs() < 1e-9 && (q[1] - 463009.996).abs() < 1e-9 && (q[2] - 3.5).abs() < 1e-12);
        assert_eq!(crs_name("EPSG 28992"), "EPSG:28992");
        assert_eq!(crs_name("28992"), "EPSG:28992");
    }

    #[test]
    fn breps_extrusions_and_mapped_items() {
        let m = step(
            "#1=IFCCARTESIANPOINT((0.,0.,0.));\n#2=IFCCARTESIANPOINT((2.,0.,0.));\n#3=IFCCARTESIANPOINT((2.,2.,0.));\n#4=IFCCARTESIANPOINT((0.,2.,0.));\n\
             #5=IFCPOLYLOOP((#1,#2,#3,#4));\n#6=IFCFACEOUTERBOUND(#5,.T.);\n#7=IFCFACE((#6));\n#8=IFCCLOSEDSHELL((#7));\n#9=IFCFACETEDBREP(#8);\n\
             #20=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,2.,4.);\n#21=IFCDIRECTION((0.,0.,1.));\n#22=IFCEXTRUDEDAREASOLID(#20,$,#21,3.);\n\
             #30=IFCCARTESIANPOINT((0.,0.,0.));\n#31=IFCAXIS2PLACEMENT3D(#30,$,$);\n#32=IFCSHAPEREPRESENTATION(#99,'Body','SweptSolid',(#22));\n\
             #33=IFCREPRESENTATIONMAP(#31,#32);\n#34=IFCCARTESIANPOINT((10.,0.,0.));\n#35=IFCCARTESIANTRANSFORMATIONOPERATOR3D($,$,#34,2.,$);\n#36=IFCMAPPEDITEM(#33,#35);\n\
             #40=IFCSHAPEREPRESENTATION(#99,'Body','Brep',(#9,#36));\n#41=IFCPRODUCTDEFINITIONSHAPE($,$,(#40));\n\
             #42=IFCWALL('2O2Fr$t4X7Zf8NOew3FLOH',$,'Muur',$,$,$,#41,$,.SOLIDWALL.);\n\
             #50=IFCPROPERTYSINGLEVALUE('FireRating',$,IFCLABEL('EI60'),$);\n#51=IFCPROPERTYSET('3O2Fr$t4X7Zf8NOew3FLOH',$,'Pset_WallCommon',$,(#50));\n\
             #52=IFCRELDEFINESBYPROPERTIES('4O2Fr$t4X7Zf8NOew3FLOH',$,$,$,(#42),#51);\n",
        );
        let c = read(&m);
        assert_eq!(c.products.len(), 1);
        let (p, terrain) = &c.products[0];
        assert!(!terrain);
        assert_eq!(p.predefined.as_deref(), Some("SOLIDWALL"));
        assert_eq!(p.prop("Pset_WallCommon", "FireRating"), Some("EI60"));
        // Square face: 2 triangles; box: 2 caps × 2 + 4 sides × 2 = 12.
        assert_eq!(p.triangles(), 14);
        let all: Vec<P3> = p.parts.iter().flat_map(|q| q.tris.iter().flatten().copied()).collect();
        // The mapped box is scaled 2× and moved to x = 10: from 8 to 12, up to 6.
        let max_x = all.iter().map(|q| q[0]).fold(f64::MIN, f64::max);
        let max_z = all.iter().map(|q| q[2]).fold(f64::MIN, f64::max);
        assert!((max_x - 12.0).abs() < 1e-9 && (max_z - 6.0).abs() < 1e-9, "{max_x} {max_z}");
        // The extruded box is closed: its volume is 2·4·3·2³.
        let tris: Vec<[P3; 3]> = p.parts.iter().flat_map(|q| q.tris.iter().copied()).filter(|t| t.iter().all(|q| q[0] > 7.0)).collect();
        let vol: f64 = tris.iter().map(|t| dot(t[0], cross(t[1], t[2]))).sum::<f64>() / 6.0;
        assert!((vol - 192.0).abs() < 1e-6, "{vol}");
        assert_eq!(nlcs_object("IFCWALL", None), ("KW", "KUNSTWERK"));
    }

    #[test]
    fn polygons_with_holes_and_concave_outlines() {
        let outer = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 0.0], [0.0, 4.0, 0.0]];
        let hole = vec![[1.0, 1.0, 0.0], [1.0, 3.0, 0.0], [3.0, 3.0, 0.0], [3.0, 1.0, 0.0]];
        let t = polygon(&outer, &[hole]);
        let area: f64 = t.iter().map(|t| cross(sub(t[1], t[0]), sub(t[2], t[0]))[2] / 2.0).sum();
        assert!((area - 12.0).abs() < 1e-9, "{area}");
        // An L in a vertical plane keeps its winding.
        let l = [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [0.0, 1.0, 2.0], [0.0, 1.0, 1.0], [0.0, 2.0, 1.0], [0.0, 2.0, 0.0]];
        let t = polygon(&l, &[]);
        assert_eq!(t.len(), 4);
        let nx: f64 = t.iter().map(|t| cross(sub(t[1], t[0]), sub(t[2], t[0]))[0] / 2.0).sum();
        assert!((nx.abs() - 3.0).abs() < 1e-9 && nx < 0.0, "{nx}");
    }

    #[test]
    fn vertical_kinks_and_curves_become_pvis() {
        let v = vec![
            VSeg { kind: VKind::Gradient, start: 0.0, length: 50.0, z: 5.0, g0: 0.01, g1: 0.01 },
            VSeg { kind: VKind::Gradient, start: 50.0, length: 30.0, z: 5.5, g0: -0.02, g1: -0.02 },
            VSeg { kind: VKind::Parabola, start: 80.0, length: 20.0, z: 4.9, g0: -0.02, g1: 0.0 },
        ];
        let p = pvis(&v);
        assert_eq!(p.iter().map(|q| (q.s, q.l)).collect::<Vec<_>>(), vec![(0.0, 0.0), (50.0, 0.0), (90.0, 20.0), (100.0, 0.0)]);
        assert!((p[2].z - 4.7).abs() < 1e-12 && (p[3].z - 4.7).abs() < 1e-12);
    }
}
