//! IFC 4.3 export (IFC4X3_ADD2) of the drawing, for BIM and infra software.
//!
//! Every object becomes the IFC entity of what it is, read from its NLCS
//! layer and object data: terrain (TIN, photo terrain, water level) as
//! IfcGeographicElement, 3D BAG and CityGML buildings as IfcBuilding,
//! pavements, green, water, structures, cables and pipes, sewers, earthworks,
//! CPTs and boreholes (IfcBorehole with IfcGeotechnicalStratum), and the rest
//! as IfcAnnotation. Axes become IfcAlignments with their horizontal layout
//! and, when the axis has a design profile, the vertical layout. The project
//! location gives the georeference (IfcProjectedCRS + IfcMapConversion);
//! object data and the NLCS layer go along as property sets, layers as
//! IfcPresentationLayerAssignment, colours as styles.

pub mod alignment;
// A corridor's surface as road parts and earthworks (stream uitwisseling).
pub mod corridor;
pub mod step;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use ocs_plugin_api::host::codec::{CadDocument, Color, EntityType, Handle};
use ocs_plugin_api::host::XDataValue;

use crate::civiel::as_profiel::{Alignment, Pvi};
use crate::gis::site::Site;
use crate::nlcs::{self, LayerName};
use alignment::{HSeg, VKind, VSeg};
use step::{coord, coord_list, index_list, opt_text, real, refs, text, Id, Step};

pub const SCHEMA: &str = "IFC4X3_ADD2";

/// What to export.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Only these entities (the selection); `None` exports everything.
    pub only: Option<HashSet<Handle>>,
    /// Leave texts out (labels, heights, parcel numbers).
    pub no_texts: bool,
}

/// The drawing to export.
pub struct Source<'a> {
    pub doc: &'a CadDocument,
    /// Drawing units per metre.
    pub f: f64,
    pub site: Option<&'a Site>,
    /// The drawing's file name: names the project and seeds the GlobalIds.
    pub name: String,
}

/// What was written.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub path: PathBuf,
    /// Objects per IFC class.
    pub classes: BTreeMap<String, usize>,
    /// Entities left out, per reason.
    pub skipped: BTreeMap<String, usize>,
    pub alignments: usize,
    pub instances: u64,
    /// The CRS of the georeference, when the drawing has a project location.
    pub crs: Option<String>,
}

impl Report {
    pub fn objects(&self) -> usize {
        self.classes.values().sum()
    }

    pub fn summary(&self) -> String {
        let tr = crate::lang::tr;
        let classes: Vec<String> = self.classes.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let mut s = format!(
            "IFC 4x3: {} {} ({}), {} {}, {} → {}",
            self.objects(),
            tr("objecten", "objects"),
            classes.join(", "),
            self.alignments,
            tr("as(sen)", "alignment(s)"),
            match &self.crs {
                Some(c) => format!("{} {c}", tr("georeferentie", "georeferenced in")),
                None => tr("zonder georeferentie (geen projectlocatie)", "not georeferenced (no project location)").to_string(),
            },
            self.path.display()
        );
        if !self.skipped.is_empty() {
            let sk: Vec<String> = self.skipped.iter().map(|(k, v)| format!("{k} {v}")).collect();
            s.push_str(&format!("; {}: {}", tr("niet in IFC", "not in IFC"), sk.join(", ")));
        }
        s
    }
}

/// Write the drawing as IFC 4.3 to `path`.
pub fn write(src: &Source, opt: &Options, path: &Path) -> Result<Report, String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut report = Report { path: path.to_path_buf(), ..Report::default() };
    export(src, opt, BufWriter::with_capacity(1 << 20, file), &mut report).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(report)
}

/// Write the drawing as IFC 4.3 into any writer (tests).
pub fn export<W: Write>(src: &Source, opt: &Options, out: W, report: &mut Report) -> io::Result<W> {
    let mut x = Exporter::new(src, out)?;
    x.run(opt, report)?;
    report.instances = x.step.count();
    x.step.finish()
}

// ── Coordinates ─────────────────────────────────────────────────────────────

/// Drawing → IFC coordinates: metres from the project's base point (the
/// map conversion holds the base point's map coordinates).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub f: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Frame {
    pub fn new(f: f64, site: Option<&Site>) -> (Frame, [f64; 2]) {
        let f = if f > 0.0 { f } else { 1.0 };
        match site {
            Some(s) => {
                let o = s.from_drawing([0.0, 0.0], f);
                let base = [s.origin_e, s.origin_n];
                (Frame { f, dx: o[0] - base[0], dy: o[1] - base[1] }, base)
            }
            None => (Frame { f, dx: 0.0, dy: 0.0 }, [0.0, 0.0]),
        }
    }

    pub fn p(&self, x: f64, y: f64, z: f64) -> [f64; 3] {
        [x / self.f + self.dx, y / self.f + self.dy, z / self.f]
    }
}

// ── Geometry of one entity ──────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum Geo {
    Mesh { verts: Vec<[f64; 3]>, tris: Vec<[usize; 3]> },
    /// Points, and per segment an arc's middle point (`None`: straight).
    Curve { pts: Vec<[f64; 3]>, mids: Vec<Option<[f64; 3]>>, closed: bool },
    Point([f64; 3]),
    Text { at: [f64; 3], value: String, rotation: f64 },
}

impl Geo {
    fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut add = |p: &[f64; 3]| {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        };
        match self {
            Geo::Mesh { verts, .. } => verts.iter().for_each(&mut add),
            Geo::Curve { pts, mids, .. } => {
                pts.iter().for_each(&mut add);
                mids.iter().flatten().for_each(&mut add);
            }
            Geo::Point(p) | Geo::Text { at: p, .. } => add(p),
        }
        b
    }

    fn is_3d(&self) -> bool {
        match self {
            Geo::Curve { pts, mids, .. } => pts.iter().chain(mids.iter().flatten()).any(|p| p[2].abs() > 1e-9),
            _ => true,
        }
    }
}

fn mesh(verts: Vec<[f64; 3]>, faces: impl IntoIterator<Item = Vec<usize>>) -> Result<Geo, &'static str> {
    let n = verts.len();
    let mut tris = Vec::new();
    for f in faces {
        if f.len() < 3 || f.iter().any(|i| *i >= n) {
            continue;
        }
        for k in 1..f.len() - 1 {
            let t = [f[0], f[k], f[k + 1]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                tris.push(t);
            }
        }
    }
    if tris.is_empty() {
        return Err("lege mesh");
    }
    Ok(Geo::Mesh { verts, tris })
}

/// A polyline with DXF bulges: an arc's middle point lies on the right of
/// the chord for a counter-clockwise arc (bulge > 0).
fn bulge_curve(v: &[([f64; 3], f64)], closed: bool) -> Result<Geo, &'static str> {
    if v.len() < 2 {
        return Err("lege lijn");
    }
    let pts: Vec<[f64; 3]> = v.iter().map(|(p, _)| *p).collect();
    let n = if closed { v.len() } else { v.len() - 1 };
    let mids = (0..n)
        .map(|i| {
            let (a, b) = (v[i].0, v[(i + 1) % v.len()].0);
            let bulge = v[i].1;
            (bulge.abs() > 1e-9).then(|| {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                [(a[0] + b[0]) / 2.0 + bulge * dy / 2.0, (a[1] + b[1]) / 2.0 - bulge * dx / 2.0, (a[2] + b[2]) / 2.0]
            })
        })
        .collect();
    Ok(Geo::Curve { pts, mids, closed })
}

fn polyline(pts: Vec<[f64; 3]>, closed: bool) -> Result<Geo, &'static str> {
    if pts.len() < 2 {
        return Err("lege lijn");
    }
    let n = if closed { pts.len() } else { pts.len() - 1 };
    Ok(Geo::Curve { pts, mids: vec![None; n], closed })
}

/// MTEXT without its formatting codes.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => {}
            '\\' => match chars.next() {
                Some('P') | Some('n') => out.push(' '),
                Some('~') => out.push(' '),
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                // \f…; \H…; \C…; and the like: skip to the semicolon.
                Some(_) => {
                    for d in chars.by_ref() {
                        if d == ';' {
                            break;
                        }
                    }
                }
                None => {}
            },
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

fn geometry(e: &EntityType, fr: &Frame) -> Result<Geo, &'static str> {
    use ocs_plugin_api::host::codec::entities::TextHorizontalAlignment as H;
    let p = |v: &ocs_plugin_api::host::codec::Vector3| fr.p(v.x, v.y, v.z);
    // A mesh without faces but with points (a column drawn as two points)
    // still marks where its object is: as a line through them.
    let or_line = |g: Result<Geo, &'static str>, verts: Vec<[f64; 3]>| match g {
        Err(_) if verts.len() >= 2 => polyline(verts, false),
        other => other,
    };
    match e {
        EntityType::Mesh(m) => {
            let verts: Vec<[f64; 3]> = m.vertices.iter().map(p).collect();
            or_line(mesh(verts.clone(), m.faces.iter().map(|f| f.vertices.clone())), verts)
        }
        EntityType::PolyfaceMesh(m) => {
            let verts: Vec<[f64; 3]> = m.vertices.iter().map(|v| p(&v.location)).collect();
            let faces = m.faces.iter().map(|f| [f.index1, f.index2, f.index3, f.index4].iter().filter(|i| **i != 0).map(|i| i.unsigned_abs() as usize - 1).collect());
            or_line(mesh(verts.clone(), faces), verts)
        }
        EntityType::Face3D(f) => {
            let mut c = vec![p(&f.first_corner), p(&f.second_corner), p(&f.third_corner)];
            let d = p(&f.fourth_corner);
            if (0..3).any(|k| (d[k] - c[2][k]).abs() > 1e-9) {
                c.push(d);
            }
            let face: Vec<usize> = (0..c.len()).collect();
            mesh(c, [face])
        }
        // A 2D solid fill: its corners run 1-2-4-3.
        EntityType::Solid(s) => {
            let c = vec![p(&s.first_corner), p(&s.second_corner), p(&s.fourth_corner), p(&s.third_corner)];
            mesh(c, [vec![0, 1, 2, 3]])
        }
        EntityType::Line(l) => polyline(vec![p(&l.start), p(&l.end)], false),
        EntityType::Arc(a) => {
            let mut sweep = a.end_angle - a.start_angle;
            if sweep <= 0.0 {
                sweep += std::f64::consts::TAU;
            }
            let at = |t: f64| fr.p(a.center.x + a.radius * t.cos(), a.center.y + a.radius * t.sin(), a.center.z);
            Ok(Geo::Curve { pts: vec![at(a.start_angle), at(a.end_angle)], mids: vec![Some(at(a.start_angle + sweep / 2.0))], closed: false })
        }
        EntityType::Circle(c) => {
            let at = |t: f64| fr.p(c.center.x + c.radius * t.cos(), c.center.y + c.radius * t.sin(), c.center.z);
            use std::f64::consts::{FRAC_PI_2, PI};
            Ok(Geo::Curve { pts: vec![at(0.0), at(PI)], mids: vec![Some(at(FRAC_PI_2)), Some(at(PI + FRAC_PI_2))], closed: true })
        }
        EntityType::LwPolyline(l) => {
            let v: Vec<([f64; 3], f64)> = l.vertices.iter().map(|v| (fr.p(v.location.x, v.location.y, l.elevation), v.bulge)).collect();
            bulge_curve(&v, l.is_closed)
        }
        EntityType::Polyline2D(pl) => {
            let v: Vec<([f64; 3], f64)> = pl.vertices.iter().map(|v| (p(&v.location), v.bulge)).collect();
            bulge_curve(&v, pl.flags.is_closed())
        }
        EntityType::Polyline3D(pl) => polyline(pl.vertices.iter().map(|v| p(&v.position)).collect(), pl.flags.closed),
        EntityType::Spline(s) => {
            let src = if s.fit_points.len() >= 2 { &s.fit_points } else { &s.control_points };
            polyline(src.iter().map(p).collect(), false)
        }
        EntityType::Ellipse(el) => {
            let (mx, my) = (el.major_axis.x, el.major_axis.y);
            let (nx, ny) = (-my * el.minor_axis_ratio, mx * el.minor_axis_ratio);
            let mut t1 = el.end_parameter;
            if t1 <= el.start_parameter {
                t1 += std::f64::consts::TAU;
            }
            let full = (t1 - el.start_parameter - std::f64::consts::TAU).abs() < 1e-9;
            let steps = 48;
            let pts = (0..=steps)
                .take(if full { steps } else { steps + 1 })
                .map(|i| {
                    let t = el.start_parameter + (t1 - el.start_parameter) * i as f64 / steps as f64;
                    fr.p(el.center.x + mx * t.cos() + nx * t.sin(), el.center.y + my * t.cos() + ny * t.sin(), el.center.z)
                })
                .collect();
            polyline(pts, full)
        }
        EntityType::Point(pt) => Ok(Geo::Point(p(&pt.location))),
        EntityType::Text(t) => {
            let at = match (t.horizontal_alignment, t.alignment_point) {
                (H::Left, _) | (_, None) => t.insertion_point,
                (_, Some(a)) => a,
            };
            let value = t.value.trim().to_string();
            if value.is_empty() {
                return Err("lege tekst");
            }
            Ok(Geo::Text { at: p(&at), value, rotation: t.rotation })
        }
        EntityType::MText(t) => {
            let value = plain(&t.value);
            if value.is_empty() {
                return Err("lege tekst");
            }
            Ok(Geo::Text { at: p(&t.insertion_point), value, rotation: t.rotation })
        }
        EntityType::RasterImage(_) => Err("kaartbeeld (raster)"),
        EntityType::Hatch(_) => Err("arcering"),
        // A survey symbol (placed from a point code) stands for its object at
        // the measured point; other blocks (vehicles, sheets) stay out.
        EntityType::Insert(i) if crate::civiel::data_value(e, "code").is_some() => Ok(Geo::Point(p(&i.insert_point))),
        EntityType::Insert(_) => Err("blokverwijzing"),
        EntityType::Viewport(_) => Err("viewport"),
        EntityType::Dimension(_) => Err("maatvoering"),
        _ => Err("ander DWG-type"),
    }
}

// ── What an object is ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// One element per entity, or per object id when it has one.
    Element,
    /// All faces of a layer in one triangulated surface.
    Terrain,
    Building,
    /// A soil layer of a CPT or borehole.
    Stratum,
    Axis,
}

#[derive(Clone, Debug, PartialEq)]
struct Class {
    ifc: &'static str,
    predefined: Option<&'static str>,
    object_type: Option<String>,
    role: Role,
}

fn class(ifc: &'static str, predefined: Option<&'static str>, object_type: Option<String>) -> Class {
    Class { ifc, predefined, object_type, role: Role::Element }
}

fn annotation(kind: &'static str, object_type: Option<String>) -> Class {
    class("IfcAnnotation", Some(kind), object_type)
}

/// Object data as key/value pairs, in their order.
type Data = Vec<(String, String)>;

fn data_of(e: &EntityType) -> Data {
    let Some(r) = e.common().extended_data.get_record(crate::bridge::XDATA_APP) else { return Vec::new() };
    r.values
        .iter()
        .filter_map(|v| match v {
            XDataValue::String(s) => s.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())),
            _ => None,
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

fn get<'a>(d: &'a Data, key: &str) -> Option<&'a str> {
    d.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// The road (IfcRoad) an object belongs to: its corridor, junction or roundabout.
fn road_of(d: &Data) -> Option<&str> {
    get(d, "corridor").or_else(|| get(d, "kruispunt")).or_else(|| get(d, "rotonde"))
}

/// The sewer values of a pipe or manhole (CIV_RIOOL, GWSW), under English
/// names: invert levels (BOB) in m above the datum (NAP), sizes in mm.
fn sewer_props(d: &Data) -> Data {
    let pick = |keys: &[&str]| keys.iter().find_map(|k| get(d, k)).map(str::to_string);
    let mut out = Data::new();
    for (name, keys) in [
        ("InvertLevelStart", &["bob_van", "bob_beginpunt_leiding"][..]),
        ("InvertLevelEnd", &["bob_naar", "bob_eindpunt_leiding"]),
        ("NominalDiameter", &["diameter", "breedte_leiding"]),
        ("InnerDiameter", &["binnendiameter"]),
        ("WallThickness", &["wanddikte"]),
        ("Material", &["materiaal", "materiaal_leiding"]),
        ("Length", &["lengte", "lengte_leiding"]),
        ("Slope", &["verhang"]),
        ("System", &["stelsel", "stelseltype"]),
        ("FromManhole", &["van", "beginpunt"]),
        ("ToManhole", &["naar", "eindpunt"]),
        ("ManholeNumber", &["put"]),
        ("GroundLevel", &["maaiveld", "maaiveldhoogte"]),
        ("BottomLevel", &["bob_laagst"]),
        ("Depth", &["diepte"]),
        ("ManholeSize", &["afmeting", "breedte_put"]),
    ] {
        if let Some(v) = pick(keys) {
            out.push((name.to_string(), v));
        }
    }
    // Only sewer objects: something with an invert level or a manhole bottom.
    if out.iter().any(|(k, _)| k.starts_with("InvertLevel") || k == "BottomLevel" || (k == "GroundLevel" && get(d, "rol") == Some("put"))) { out } else { Data::new() }
}

/// KLIC cables and pipes, after the NLCS objects of `nlcs::klic_object`.
/// `symbol`: the NLCS symbol element (S), e.g. the hydrant itself rather
/// than the fire main that leads to it.
fn klic(o: &str, ot: Option<String>, symbol: bool) -> Class {
    let electric = o.starts_with("ET_") || o.starts_with("DATA_");
    if o.contains("BRANDHYDRANT") && symbol {
        return class("IfcFireSuppressionTerminal", Some("FIREHYDRANT"), ot);
    }
    if o.contains("STRAATPOT") {
        return class("IfcPipeFitting", Some("NOTDEFINED"), ot);
    }
    if o.contains("MANTELBUIS") || o.contains("KABELBED") {
        return class("IfcCableCarrierSegment", Some("CONDUITSEGMENT"), ot);
    }
    if o.contains("HULPSTUK") {
        return if electric { class("IfcCableFitting", Some("NOTDEFINED"), ot) } else { class("IfcPipeFitting", Some("NOTDEFINED"), ot) };
    }
    if electric {
        return class("IfcCableSegment", Some(if o.starts_with("DATA_") { "NOTDEFINED" } else { "CABLESEGMENT" }), ot);
    }
    class("IfcPipeSegment", Some("NOTDEFINED"), ot)
}

fn classify(ln: Option<&LayerName>, layer: &str, data: &Data, geo: &Geo) -> Result<Class, &'static str> {
    let is_mesh = matches!(geo, Geo::Mesh { .. });
    if matches!(geo, Geo::Text { .. }) {
        return Ok(annotation("TEXT", None));
    }
    if get(data, "bron") == Some("waterpeil") {
        return Ok(Class { role: Role::Terrain, ..class("IfcGeographicElement", Some("USERDEFINED"), Some("Waterpeil".into())) });
    }
    // Civil analyses without an NLCS object, by their object data.
    for (key, what) in [("stroomgebied", "Stroomgebied"), ("plas", "Plas"), ("waterdruppel", "Waterdruppel"), ("vak", "Parkeervak")] {
        if get(data, key).is_some() && ln.is_none() {
            return Ok(annotation("USERDEFINED", Some(what.into())));
        }
    }
    let Some(ln) = ln else {
        // Not an NLCS layer: GIS imports (OpenStreetMap, WFS, files) and the rest.
        let ot = Some(layer.to_string());
        return Ok(if is_mesh { class("IfcBuildingElementProxy", Some("USERDEFINED"), ot) } else { annotation("USERDEFINED", ot) });
    };
    let (g, o) = (ln.group.as_str(), ln.object.as_str());
    let ot = Some(nlcs::humanize(o));
    match ln.element.as_str() {
        "T" => return Ok(annotation("TEXT", ot)),
        "M" => return Ok(annotation("DIMENSION", ot)),
        "L" => return Ok(annotation("LEADER", ot)),
        "A" => return Err("arcering"),
        _ => {}
    }
    let generic = |ot: Option<String>| if is_mesh { class("IfcBuildingElementProxy", Some("USERDEFINED"), ot) } else { annotation("USERDEFINED", ot) };
    Ok(match g {
        "AL" if o == "VERWIJZING_DOORSNEDE" => annotation("USERDEFINED", ot),
        "AL" => return Err("tekenblad en profielkader"),
        "AM" if o.starts_with("AS")
            && !is_mesh
            && matches!(geo, Geo::Curve { .. })
            && ln.element == "G"
            && ["verschuiving_m", "corridor", "ontwerpprofiel"].iter().all(|k| get(data, k).is_none()) =>
        {
            Class { role: Role::Axis, ..class("IfcAlignment", None, ot) }
        }
        "AM" | "ZZ" | "KG" => annotation("USERDEFINED", ot),
        "OG" => match o {
            "TRIANGULATIE" | "DTM" => Class { role: Role::Terrain, ..class("IfcGeographicElement", Some("TERRAIN"), ot) },
            _ if o.starts_with("HOOGTELIJN") => annotation("CONTOURLINE", ot),
            "HOOGTEPUNT" => annotation("SURVEY", ot),
            _ if o.starts_with("WATER") => class("IfcGeographicElement", Some("USERDEFINED"), ot),
            _ if o.starts_with("TERREIN") => class("IfcGeographicElement", Some("TERRAIN"), ot),
            "WEG" => class("IfcPavement", Some("NOTDEFINED"), ot),
            "BEBOUWING" => Class { role: Role::Building, ..class("IfcBuilding", None, ot) },
            _ if o.starts_with("BEBOUWING") => class("IfcBuildingElementProxy", Some("USERDEFINED"), ot),
            _ if is_mesh => class("IfcGeographicElement", Some("USERDEFINED"), ot),
            _ => annotation("USERDEFINED", ot),
        },
        // A corridor's pavement layers as bodies: one course per material.
        "VH" if is_mesh && get(data, "laag").is_some() => {
            class("IfcCourse", Some(if get(data, "rol") == Some("verharding") { "PAVEMENT" } else { "USERDEFINED" }), get(data, "laag").map(str::to_string))
        }
        // A corridor's formation and subgrade surfaces.
        "VH" | "GW" if is_mesh && get(data, "oppervlak").is_some() => Class { role: Role::Terrain, ..class("IfcGeographicElement", Some("USERDEFINED"), get(data, "oppervlak").map(str::to_string)) },
        // A ditch or watercourse body; its lines (water line, toe) stay annotation.
        "GW" if o.starts_with("WATERGANG") && is_mesh => class("IfcGeographicElement", Some("USERDEFINED"), ot),
        "VH" => match o {
            _ if o.starts_with("ONVERHARD") => class("IfcGeographicElement", Some("TERRAIN"), ot),
            _ if o.starts_with("KANTOPSLUITING") => class("IfcKerb", Some("NOTDEFINED"), ot),
            "ROOSTER" => class("IfcCivilElement", None, ot),
            _ if o.contains("ASFALT") => class("IfcPavement", Some("FLEXIBLE"), ot),
            _ if o.contains("BETON") && o.starts_with("GESLOTEN") => class("IfcPavement", Some("RIGID"), ot),
            _ => class("IfcPavement", Some("NOTDEFINED"), ot),
        },
        "GR" => class("IfcGeographicElement", Some("VEGETATION"), ot),
        "WH" | "OB" => class("IfcGeographicElement", Some("USERDEFINED"), ot),
        "KL" => klic(o, ot, ln.element == "S"),
        "RI" => match o {
            // Flow arrows of a sewer design are symbols, not pipes.
            _ if o.contains("STROMINGSRICHTING") => annotation("SYMBOL", ot),
            _ if o.contains("PUT") => class("IfcDistributionChamberElement", Some("MANHOLE"), ot),
            _ if o.contains("KOLK") => class("IfcDistributionChamberElement", Some("SUMP"), ot),
            _ if o.contains("GOOT") => class("IfcPipeSegment", Some("GUTTER"), ot),
            _ => class("IfcPipeSegment", Some("NOTDEFINED"), ot),
        },
        "MO" if o.starts_with("ONDERZOEK_") && is_mesh => Class { role: Role::Stratum, ..class("IfcGeotechnicalStratum", Some("SOLID"), ot) },
        "MO" => annotation(if ln.element == "S" { "SYMBOL" } else { "USERDEFINED" }, ot),
        // Sight lines of the sight distance check: drawing aids, not structures.
        "KW" if o == "ZICHTRUIMTE" => annotation("USERDEFINED", ot),
        "KW" => class("IfcCivilElement", None, ot),
        "GW" => match o {
            _ if o.starts_with("ONTGRAVING") => class("IfcEarthworksCut", Some("EXCAVATION"), ot),
            _ if o.starts_with("OPHOGING") => class("IfcEarthworksFill", Some("EMBANKMENT"), ot),
            _ if is_mesh => class("IfcEarthworksFill", Some("USERDEFINED"), ot),
            _ => annotation("USERDEFINED", ot),
        },
        "IE" => match o {
            _ if o.contains("HEKWERK") => class("IfcRailing", Some("FENCE"), ot),
            _ if o.starts_with("BALUSTRADE") => class("IfcRailing", Some("BALUSTRADE"), ot),
            _ if o.contains("MUUR") => class("IfcWall", Some("NOTDEFINED"), ot),
            _ => class("IfcCivilElement", None, ot),
        },
        "BV" => class("IfcRailing", Some("GUARDRAIL"), ot),
        "GK" => class("IfcWall", Some("RETAININGWALL"), ot),
        "VW" | "VV" if o.contains("MARKERING") => class("IfcSurfaceFeature", Some("PAVEMENTSURFACEMARKING"), ot),
        "VW" | "VV" if o.contains("BORD") => class("IfcSign", Some("NOTDEFINED"), ot),
        "VW" | "VV" | "OV" | "IW" | "IV" | "FV" => class("IfcCivilElement", None, ot),
        _ => generic(ot),
    })
}

/// The attributes after `Representation` (`Tag`, `PredefinedType`).
fn tail(ifc: &str) -> (bool, bool) {
    match ifc {
        "IfcAnnotation" => (false, true),
        "IfcCivilElement" | "IfcBorehole" | "IfcBuiltElement" | "IfcFurnishingElement" => (true, false),
        _ => (true, true),
    }
}

// ── Grouping ────────────────────────────────────────────────────────────────

/// A mesh assembled from many entities, with one colour per triangle.
#[derive(Default)]
struct Merged {
    verts: Vec<[f64; 3]>,
    tris: Vec<[usize; 3]>,
    face_colour: Vec<usize>,
    colours: Vec<(u8, u8, u8)>,
    index: HashMap<(i64, i64, i64), usize>,
    colour_index: HashMap<(u8, u8, u8), usize>,
}

impl Merged {
    fn add(&mut self, verts: &[[f64; 3]], tris: &[[usize; 3]], rgb: (u8, u8, u8)) {
        let c = *self.colour_index.entry(rgb).or_insert_with(|| {
            self.colours.push(rgb);
            self.colours.len() - 1
        });
        let ids: Vec<usize> = verts
            .iter()
            .map(|p| {
                let key = ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64);
                *self.index.entry(key).or_insert_with(|| {
                    self.verts.push(*p);
                    self.verts.len() - 1
                })
            })
            .collect();
        for t in tris {
            let t = [ids[t[0]], ids[t[1]], ids[t[2]]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                self.tris.push(t);
                self.face_colour.push(c);
            }
        }
    }
}

struct Group {
    class: Class,
    name: String,
    handles: Vec<Handle>,
    layer: String,
    ln: Option<LayerName>,
    data: Data,
    /// (identifier, type) → representation items.
    reps: BTreeMap<(&'static str, &'static str), Vec<Id>>,
    merged: Option<Merged>,
    /// The CPT or borehole of a stratum.
    parent: Option<String>,
    /// The corridor (IfcRoad) the object belongs to.
    road: Option<String>,
    /// Plan extent [min x, min y, max x, max y].
    bbox: [f64; 4],
}

// ── The exporter ────────────────────────────────────────────────────────────

struct Common {
    z_axis: Id,
    identity2: Id,
    line2: Id,
    model: Id,
    body: Id,
    axis: Id,
    footprint: Id,
    annotation: Id,
    reference: Id,
    placement: Id,
    metre: Id,
}

struct Exporter<'a, W: Write> {
    src: &'a Source<'a>,
    step: Step<W>,
    fr: Frame,
    base: [f64; 2],
    seed: String,
    guids: HashSet<String>,
    c: Common,
    surface_styles: HashMap<(u8, u8, u8, u8), Id>,
    curve_styles: HashMap<(u8, u8, u8), Id>,
    layer_colours: HashMap<String, (u8, u8, u8)>,
    layer_items: BTreeMap<String, Vec<Id>>,
    project: Id,
    site: Id,
}

fn is_line_index(mids: &[Option<[f64; 3]>]) -> bool {
    mids.iter().all(Option::is_none)
}

impl<'a, W: Write> Exporter<'a, W> {
    fn new(src: &'a Source<'a>, out: W) -> io::Result<Self> {
        let (fr, base) = Frame::new(src.f, src.site);
        let time = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let system = format!("Open Infra Studio {}", env!("CARGO_PKG_VERSION"));
        let mut step = Step::new(out, "Alignment-basedReferenceView", &src.name, &time, "", "OpenAEC Foundation", &system, SCHEMA)?;
        let origin3 = step.add("IFCCARTESIANPOINT", "(0.,0.,0.)")?;
        let z_axis = step.add("IFCDIRECTION", "(0.,0.,1.)")?;
        let x_axis = step.add("IFCDIRECTION", "(1.,0.,0.)")?;
        let world = step.add("IFCAXIS2PLACEMENT3D", &format!("{origin3},{z_axis},{x_axis}"))?;
        let origin2 = step.add("IFCCARTESIANPOINT", "(0.,0.)")?;
        let x_axis2 = step.add("IFCDIRECTION", "(1.,0.)")?;
        let identity2 = step.add("IFCAXIS2PLACEMENT2D", &format!("{origin2},{x_axis2}"))?;
        let north = step.add("IFCDIRECTION", "(0.,1.)")?;
        let unit_vector = step.add("IFCVECTOR", &format!("{x_axis2},1."))?;
        let line2 = step.add("IFCLINE", &format!("{origin2},{unit_vector}"))?;
        let model = step.add("IFCGEOMETRICREPRESENTATIONCONTEXT", &format!("$,'Model',3,1.E-05,{world},{north}"))?;
        let sub = |step: &mut Step<W>, id: &str, kind: &str, view: &str| step.add("IFCGEOMETRICREPRESENTATIONSUBCONTEXT", &format!("'{id}','{kind}',*,*,*,*,{model},$,.{view}.,$"));
        let body = sub(&mut step, "Body", "Model", "MODEL_VIEW")?;
        let axis = sub(&mut step, "Axis", "Model", "MODEL_VIEW")?;
        let footprint = sub(&mut step, "FootPrint", "Model", "PLAN_VIEW")?;
        let annotation = sub(&mut step, "Annotation", "Plan", "PLAN_VIEW")?;
        let reference = sub(&mut step, "Reference", "Model", "MODEL_VIEW")?;
        let metre = step.add("IFCSIUNIT", "*,.LENGTHUNIT.,$,.METRE.")?;
        let area = step.add("IFCSIUNIT", "*,.AREAUNIT.,$,.SQUARE_METRE.")?;
        let volume = step.add("IFCSIUNIT", "*,.VOLUMEUNIT.,$,.CUBIC_METRE.")?;
        let angle = step.add("IFCSIUNIT", "*,.PLANEANGLEUNIT.,$,.RADIAN.")?;
        let units = step.add("IFCUNITASSIGNMENT", &refs(&[metre, area, volume, angle]))?;
        let seed = src.name.clone();
        let mut guids = HashSet::new();
        let mut g = |key: &str| unique(&mut guids, &seed, key);
        let project_name = src.site.map(|s| s.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| src.name.clone());
        let project = step.add(
            "IFCPROJECT",
            &format!("{},$,{},{},$,$,$,({model}),{units}", text(&g("project")), text(&project_name), text(&format!("Open Infra Studio — {}", src.name))),
        )?;
        let site_placement = step.add("IFCLOCALPLACEMENT", &format!("$,{world}"))?;
        let placement = step.add("IFCLOCALPLACEMENT", &format!("{site_placement},{world}"))?;
        let (lat, lon) = match src.site {
            Some(s) => (compound(s.lat), compound(s.lon)),
            None => ("$".into(), "$".into()),
        };
        let site = step.add(
            "IFCSITE",
            &format!("{},$,{},$,$,{site_placement},$,$,.ELEMENT.,{lat},{lon},$,$,$", text(&g("site")), text(&project_name)),
        )?;
        step.add("IFCRELAGGREGATES", &format!("{},$,$,$,{project},({site})", text(&g("rel:project-site"))))?;
        let c = Common { z_axis, identity2, line2, model, body, axis, footprint, annotation, reference, placement, metre };
        let mut x = Exporter {
            src,
            step,
            fr,
            base,
            seed,
            guids,
            c,
            surface_styles: HashMap::new(),
            curve_styles: HashMap::new(),
            layer_colours: HashMap::new(),
            layer_items: BTreeMap::new(),
            project,
            site,
        };
        x.layer_colours = layer_colours(src.doc);
        Ok(x)
    }

    fn guid(&mut self, key: &str) -> String {
        text(&unique(&mut self.guids, &self.seed, key))
    }

    /// IfcProjectedCRS + IfcMapConversion from the project location.
    fn georeference(&mut self, report: &mut Report) -> io::Result<()> {
        let Some(site) = self.src.site else { return Ok(()) };
        let vertical = if site.crs == "EPSG:28992" { "'EPSG:5709'" } else { "$" };
        let crs = self.step.add(
            "IFCPROJECTEDCRS",
            &format!("{},{},$,{vertical},$,$,{}", text(&site.crs), opt_text(Some(&site.crs_name)), self.c.metre),
        )?;
        self.step.add(
            "IFCMAPCONVERSION",
            &format!("{},{crs},{},{},0.,1.,0.,$", self.c.model, real(self.base[0]), real(self.base[1])),
        )?;
        report.crs = Some(site.crs.clone());
        Ok(())
    }

    fn colour(&self, e: &EntityType) -> (u8, u8, u8) {
        match e.common().color {
            Color::Rgb { r, g, b } => (r, g, b),
            c @ Color::Index(_) => c.rgb().unwrap_or((128, 128, 128)),
            _ => self.layer_colours.get(&e.common().layer).copied().unwrap_or((160, 160, 160)),
        }
    }

    fn surface_style(&mut self, rgb: (u8, u8, u8), transparency: u8) -> io::Result<Id> {
        if let Some(id) = self.surface_styles.get(&(rgb.0, rgb.1, rgb.2, transparency)) {
            return Ok(*id);
        }
        let colour = self.step.add("IFCCOLOURRGB", &format!("$,{}", rgb_args(rgb)))?;
        let t = if transparency == 0 { "$".to_string() } else { real((transparency as f64 / 255.0 * 1000.0).round() / 1000.0) };
        let shading = self.step.add("IFCSURFACESTYLESHADING", &format!("{colour},{t}"))?;
        let style = self.step.add("IFCSURFACESTYLE", &format!("$,.BOTH.,({shading})"))?;
        self.surface_styles.insert((rgb.0, rgb.1, rgb.2, transparency), style);
        Ok(style)
    }

    fn curve_style(&mut self, rgb: (u8, u8, u8)) -> io::Result<Id> {
        if let Some(id) = self.curve_styles.get(&rgb) {
            return Ok(*id);
        }
        let colour = self.step.add("IFCCOLOURRGB", &format!("$,{}", rgb_args(rgb)))?;
        let style = self.step.add("IFCCURVESTYLE", &format!("$,$,$,{colour},.T."))?;
        self.curve_styles.insert(rgb, style);
        Ok(style)
    }

    fn style(&mut self, item: Id, style: Id) -> io::Result<()> {
        self.step.add("IFCSTYLEDITEM", &format!("{item},({style}),$")).map(|_| ())
    }

    fn face_set(&mut self, verts: &[[f64; 3]], tris: &[[usize; 3]]) -> io::Result<Id> {
        let pts = self.step.add("IFCCARTESIANPOINTLIST3D", &format!("{},$", coord_list(verts)))?;
        self.step.add("IFCTRIANGULATEDFACESET", &format!("{pts},$,$,{},$", index_list(tris)))
    }

    /// IfcIndexedPolyCurve: straight pieces as IfcLineIndex, arcs as IfcArcIndex.
    fn poly_curve(&mut self, pts: &[[f64; 3]], mids: &[Option<[f64; 3]>], closed: bool, d3: bool) -> io::Result<Id> {
        let mut all: Vec<[f64; 3]> = pts.to_vec();
        let mut segments: Vec<String> = Vec::new();
        let mut run: Vec<usize> = Vec::new();
        let flush = |run: &mut Vec<usize>, segments: &mut Vec<String>| {
            if run.len() >= 2 {
                segments.push(format!("IFCLINEINDEX(({}))", run.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(",")));
            }
            run.clear();
        };
        for (i, m) in mids.iter().enumerate() {
            let (a, b) = (i, if i + 1 < pts.len() { i + 1 } else { 0 });
            match m {
                None => {
                    if run.last() != Some(&a) {
                        flush(&mut run, &mut segments);
                        run.push(a);
                    }
                    run.push(b);
                }
                Some(mid) => {
                    flush(&mut run, &mut segments);
                    all.push(*mid);
                    segments.push(format!("IFCARCINDEX(({},{},{}))", a + 1, all.len(), b + 1));
                }
            }
        }
        flush(&mut run, &mut segments);
        let list = if d3 {
            self.step.add("IFCCARTESIANPOINTLIST3D", &format!("{},$", coord_list(&all)))?
        } else {
            let flat: Vec<[f64; 2]> = all.iter().map(|p| [p[0], p[1]]).collect();
            self.step.add("IFCCARTESIANPOINTLIST2D", &format!("{},$", coord_list(&flat)))?
        };
        // A plain open polyline needs no segment list; a closed one returns to its first point.
        let segs = if is_line_index(mids) && !closed { "$".to_string() } else { format!("({})", segments.join(",")) };
        self.step.add("IFCINDEXEDPOLYCURVE", &format!("{list},{segs},.F."))
    }

    /// Representation items of one entity, styled and on its layer.
    fn items(&mut self, e: &EntityType, geo: &Geo, element: bool, closed_footprint: bool) -> io::Result<Vec<((&'static str, &'static str), Id)>> {
        let rgb = self.colour(e);
        let layer = e.common().layer.clone();
        let mut out = Vec::new();
        match geo {
            Geo::Mesh { verts, tris } => {
                let item = self.face_set(verts, tris)?;
                let t = match e.common().transparency {
                    ocs_plugin_api::host::codec::Transparency::Explicit(a) => a,
                    _ => 0,
                };
                let s = self.surface_style(rgb, t)?;
                self.style(item, s)?;
                out.push((("Body", "Tessellation"), item));
            }
            Geo::Curve { pts, mids, closed } => {
                let d3 = geo.is_3d();
                let item = self.poly_curve(pts, mids, *closed, d3)?;
                let s = self.curve_style(rgb)?;
                self.style(item, s)?;
                let ty = if d3 { "Curve3D" } else { "Curve2D" };
                let id = if !element { "Annotation" } else if *closed && closed_footprint { "FootPrint" } else { "Axis" };
                out.push(((id, ty), item));
            }
            Geo::Point(p) => {
                let item = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(p[0]), coord(p[1]), coord(p[2])))?;
                out.push(((if element { "Reference" } else { "Annotation" }, "Point"), item));
            }
            Geo::Text { at, value, rotation } => {
                let pt = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(at[0]), coord(at[1]), coord(at[2])))?;
                let dir = self.step.add("IFCDIRECTION", &format!("({},{},0.)", real(rotation.cos()), real(rotation.sin())))?;
                let place = self.step.add("IFCAXIS2PLACEMENT3D", &format!("{pt},{},{dir}", self.c.z_axis))?;
                let item = self.step.add("IFCTEXTLITERAL", &format!("{},{place},.RIGHT.", text(value)))?;
                let s = self.curve_style(rgb)?;
                self.style(item, s)?;
                out.push((("Annotation", "Annotation2D"), item));
            }
        }
        for (_, id) in &out {
            self.layer_items.entry(layer.clone()).or_default().push(*id);
        }
        Ok(out)
    }

    fn context_of(&self, identifier: &str) -> Id {
        match identifier {
            "Body" => self.c.body,
            "Axis" => self.c.axis,
            "FootPrint" => self.c.footprint,
            "Reference" => self.c.reference,
            _ => self.c.annotation,
        }
    }

    /// IfcProductDefinitionShape for these items, or `$`.
    fn shape(&mut self, reps: &BTreeMap<(&'static str, &'static str), Vec<Id>>) -> io::Result<String> {
        if reps.is_empty() {
            return Ok("$".into());
        }
        let mut ids = Vec::new();
        // Body before the others: viewers show the first.
        let mut keys: Vec<_> = reps.keys().copied().collect();
        keys.sort_by_key(|(id, _)| if *id == "Body" { 0 } else { 1 });
        for key in keys {
            let ctx = self.context_of(key.0);
            ids.push(self.step.add("IFCSHAPEREPRESENTATION", &format!("{ctx},'{}','{}',{}", key.0, key.1, refs(&reps[&key])))?);
        }
        Ok(format!("{}", self.step.add("IFCPRODUCTDEFINITIONSHAPE", &format!("$,$,{}", refs(&ids)))?))
    }

    /// A sewer pipe or manhole in 3D from its object data (CIV_RIOOL or a
    /// GWSW import): the pipe as a hollow tube along its invert levels, the
    /// manhole as a shaft from its bottom to the ground. `None` without the
    /// levels.
    fn sewer_body(&mut self, e: &EntityType, geo: &Geo, data: &Data) -> io::Result<Option<Id>> {
        let num = |k: &str| get(data, k).and_then(|v| v.replace(',', ".").parse::<f64>().ok()).filter(|v| v.is_finite());
        let item = match (geo, e) {
            (Geo::Curve { pts, closed: false, .. }, _) if pts.len() >= 2 => {
                let (Some(b0), Some(b1)) = (num("bob_van").or_else(|| num("bob_beginpunt_leiding")), num("bob_eindpunt_leiding").or_else(|| num("bob_naar"))) else { return Ok(None) };
                let inner = num("binnendiameter").or_else(|| num("breedte_leiding")).filter(|d| *d > 10.0).map(|d| d / 1000.0);
                let Some(d) = inner else { return Ok(None) };
                let t = num("wanddikte").unwrap_or(0.0).max(0.0) / 1000.0;
                let (a, b) = (pts[0], pts[pts.len() - 1]);
                let pa = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(a[0]), coord(a[1]), coord(b0 + d / 2.0)))?;
                let pb = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(b[0]), coord(b[1]), coord(b1 + d / 2.0)))?;
                let line = self.step.add("IFCPOLYLINE", &format!("({pa},{pb})"))?;
                let inner_r = if t > 0.0 { real(d / 2.0) } else { "$".into() };
                self.step.add("IFCSWEPTDISKSOLID", &format!("{line},{},{inner_r},$,$", real(d / 2.0 + t)))?
            }
            (_, EntityType::Circle(c)) => {
                let top = num("maaiveld").or_else(|| num("maaiveldhoogte"));
                // A GWSW manhole has its height instead of its bottom.
                let bottom = num("bob_laagst").or_else(|| top.zip(num("hoogte_put")).map(|(t, h)| t - h / 1000.0));
                let (Some(top), Some(bottom)) = (top, bottom) else { return Ok(None) };
                if top <= bottom {
                    return Ok(None);
                }
                let at = self.fr.p(c.center.x, c.center.y, 0.0);
                // A GWSW manhole is drawn as a marker: its width gives the size.
                let r = num("breedte_put").filter(|w| *w > 100.0).map(|w| w / 2000.0).unwrap_or(c.radius / self.fr.f);
                let o = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(at[0]), coord(at[1]), coord(bottom)))?;
                let place = self.step.add("IFCAXIS2PLACEMENT3D", &format!("{o},$,$"))?;
                let profile = self.step.add("IFCCIRCLEPROFILEDEF", &format!(".AREA.,$,$,{}", real(r)))?;
                self.step.add("IFCEXTRUDEDAREASOLID", &format!("{profile},{place},{},{}", self.c.z_axis, real(top - bottom)))?
            }
            _ => return Ok(None),
        };
        let rgb = self.colour(e);
        let s = self.surface_style(rgb, 0)?;
        self.style(item, s)?;
        self.layer_items.entry(e.common().layer.clone()).or_default().push(item);
        Ok(Some(item))
    }

    fn psets(&mut self, key: &str, owner: Id, layer: &str, ln: Option<&LayerName>, data: &Data) -> io::Result<()> {
        let mut nlcs_props: Data = vec![("Laag".into(), layer.to_string())];
        if let Some(ln) = ln {
            nlcs_props.push(("Status".into(), format!("{} ({})", ln.status.code(), ln.status.label())));
            nlcs_props.push(("Discipline".into(), ln.discipline.clone()));
            nlcs_props.push(("Hoofdgroep".into(), format!("{} ({})", ln.group, nlcs::group_name(&ln.group))));
            nlcs_props.push(("Object".into(), ln.object.clone()));
            nlcs_props.push(("Element".into(), ln.element.clone()));
        }
        self.pset(&format!("{key}:nlcs"), "OpenInfra_NLCS", &nlcs_props, owner)?;
        if !data.is_empty() {
            self.pset(&format!("{key}:data"), "OpenInfra_ObjectData", data, owner)?;
        }
        Ok(())
    }

    fn pset(&mut self, key: &str, name: &str, props: &Data, owner: Id) -> io::Result<()> {
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for (k, v) in props {
            if v.is_empty() || !seen.insert(k.clone()) {
                continue;
            }
            ids.push(self.step.add("IFCPROPERTYSINGLEVALUE", &format!("{},$,{},$", text(k), value(v)))?);
        }
        if ids.is_empty() {
            return Ok(());
        }
        let g = self.guid(&format!("pset:{key}"));
        let set = self.step.add("IFCPROPERTYSET", &format!("{g},$,{},$,{}", text(name), refs(&ids)))?;
        let g = self.guid(&format!("rel:{key}"));
        self.step.add("IFCRELDEFINESBYPROPERTIES", &format!("{g},$,$,$,({owner}),{set}")).map(|_| ())
    }

    /// An element of any class, after its class's attribute list.
    fn element(&mut self, key: &str, class: &Class, name: &str, description: &str, tag: Option<&str>, shape: &str) -> io::Result<Id> {
        let g = self.guid(&format!("el:{key}"));
        let (has_tag, has_type) = tail(class.ifc);
        let mut args = format!(
            "{g},$,{},{},{},{},{shape}",
            text(name),
            opt_text(Some(description)),
            opt_text(class.object_type.as_deref()),
            self.c.placement
        );
        if has_tag {
            args.push_str(&format!(",{}", opt_text(tag)));
        }
        if has_type {
            args.push_str(&format!(",{}", class.predefined.map(|p| format!(".{p}.")).unwrap_or_else(|| "$".into())));
        }
        self.step.add(&class.ifc.to_ascii_uppercase(), &args)
    }

    fn run(&mut self, opt: &Options, report: &mut Report) -> io::Result<()> {
        self.georeference(report)?;
        let doc = self.src.doc;
        let fr = self.fr;
        // Length profiles and cross sections are views in their own frame
        // (AL-PROFIELBALK), not objects in the world: leave their contents out.
        // A frame is a closed polyline, or one that returns to its start.
        let frames: Vec<[f64; 4]> = doc
            .model_space_entities()
            .filter(|e| matches!(e, EntityType::LwPolyline(l) if l.is_closed || is_ring(l)))
            .filter(|e| nlcs::parse_layer(&e.common().layer).is_ok_and(|ln| ln.group == "AL" && ln.object == "PROFIELBALK"))
            .filter_map(|e| geometry(e, &fr).ok().map(|g| g.bbox()))
            .collect();
        // Views lie beside the project area: lines and texts wholly outside
        // it and near a frame (a design line above the ground's height
        // range, a PVI tick) belong to the view. Views hold no meshes.
        let project = self.src.site.map(|s| {
            let r = s.drawing_rect(fr.f);
            let (a, b) = (fr.p(r[0], r[1], 0.0), fr.p(r[2], r[3], 0.0));
            [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
        });
        let inside = |b: &[f64; 4], mesh: bool| {
            let within = frames.iter().any(|r| b[0] >= r[0] - 1e-6 && b[1] >= r[1] - 1e-6 && b[2] <= r[2] + 1e-6 && b[3] <= r[3] + 1e-6);
            let beside = |a: &[f64; 4]| b[2] < a[0] || b[0] > a[2] || b[3] < a[1] || b[1] > a[3];
            let near = || {
                let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                frames.iter().any(|r| {
                    let (mx, my) = ((r[2] - r[0]) * 0.25, (r[3] - r[1]) * 0.25);
                    c[0] >= r[0] - mx && c[0] <= r[2] + mx && c[1] >= r[1] - my && c[1] <= r[3] + my
                })
            };
            within || (!mesh && project.as_ref().is_some_and(beside) && near())
        };

        let mut groups: Vec<Group> = Vec::new();
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut axes: Vec<(&EntityType, Data)> = Vec::new();
        let mut skip = |r: &str| *report.skipped.entry(r.to_string()).or_insert(0) += 1;
        for e in doc.model_space_entities() {
            let h = e.common().handle;
            if opt.only.as_ref().is_some_and(|s| !s.contains(&h)) {
                continue;
            }
            let geo = match geometry(e, &fr) {
                Ok(g) => g,
                Err(r) => {
                    skip(r);
                    continue;
                }
            };
            if opt.no_texts && matches!(geo, Geo::Text { .. }) {
                skip("tekst");
                continue;
            }
            let layer = e.common().layer.clone();
            let ln = nlcs::parse_layer(&layer).ok();
            let data = data_of(e);
            // The design profile is drawn in its length profile; the alignment carries it.
            if get(&data, "ontwerpprofiel").is_some() {
                skip("profiel- en dwarsprofielweergave");
                continue;
            }
            // Sheet sets (`civiel::bladen`): view frames, match lines, sheet
            // profiles and paper space are drawing sheets, not objects.
            if get(&data, "bladen").is_some() {
                skip("tekenblad en profielkader");
                continue;
            }
            // Labels (`civiel::labels`) are annotation as a whole: without
            // texts their ticks, crosses and arrows go too.
            if opt.no_texts && get(&data, "label").is_some() && get(&data, "vingerafdruk").is_some() {
                skip("tekst");
                continue;
            }
            // An assembly drawn as a template section is no object.
            if get(&data, "profieltype").is_some() {
                skip("profieltype");
                continue;
            }
            let mut cl = match classify(ln.as_ref(), &layer, &data, &geo) {
                Ok(c) => c,
                Err(r) => {
                    skip(r);
                    continue;
                }
            };
            if !frames.is_empty() && !(ln.as_ref().is_some_and(|l| l.group == "AL")) && inside(&geo.bbox(), matches!(geo, Geo::Mesh { .. })) {
                skip("profiel- en dwarsprofielweergave");
                continue;
            }
            // A symbol stands for its object: a circle marks a point.
            let geo = match (&geo, ln.as_ref().map(|l| l.element.as_str()), e) {
                (Geo::Curve { .. }, Some("S"), EntityType::Circle(c)) => Geo::Point(fr.p(c.center.x, c.center.y, c.center.z)),
                _ => geo,
            };
            if cl.role == Role::Axis {
                if Alignment::from_entity(e).is_some() {
                    axes.push((e, data));
                    continue;
                }
                cl = annotation("USERDEFINED", cl.object_type.clone());
            }
            let id = get(&data, "identificatie").or_else(|| get(&data, "lokaal_id")).map(str::to_string);
            let bro = get(&data, "bro_id").map(str::to_string);
            let key = match cl.role {
                Role::Terrain => format!("terrain:{layer}:{}:{}", get(&data, "bron").unwrap_or(""), road_of(&data).unwrap_or("")),
                Role::Building => format!("building:{}", id.clone().unwrap_or_else(|| format!("{h:?}"))),
                // A point cloud mesh (CIV_MESH) can span several MESH entities: one object.
                _ if cl.ifc != "IfcAnnotation" && get(&data, "mesh").is_some() => format!("mesh:{layer}:{}", get(&data, "mesh").unwrap_or("")),
                _ if cl.ifc != "IfcAnnotation" && id.is_some() => format!("{}:{}", cl.ifc, id.clone().unwrap_or_default()),
                _ => format!("entity:{}", handle_text(h)),
            };
            let gi = match by_key.get(&key) {
                Some(i) => *i,
                None => {
                    let name = match cl.role {
                        Role::Building => format!("{} {}", crate::lang::tr("Pand", "Building"), id.clone().unwrap_or_default()).trim().to_string(),
                        Role::Stratum => get(&data, "grondsoort").or_else(|| get(&data, "grondsoort_nen")).map(str::to_string).unwrap_or_else(|| cl.object_type.clone().unwrap_or_default()),
                        _ => get(&data, "naam")
                            .or_else(|| get(&data, "name"))
                            // A designed sewer (CIV_RIOOL): manhole number, pipe "from-to".
                            .or_else(|| get(&data, "put"))
                            .or_else(|| get(&data, "leiding"))
                            .map(str::to_string)
                            .or_else(|| match &geo {
                                Geo::Text { value, .. } => Some(value.clone()),
                                _ => None,
                            })
                            .unwrap_or_else(|| cl.object_type.clone().unwrap_or_else(|| layer.clone())),
                    };
                    let road = road_of(&data).map(str::to_string);
                    groups.push(Group {
                        class: cl.clone(),
                        name,
                        handles: Vec::new(),
                        layer: layer.clone(),
                        ln: ln.clone(),
                        data: data.clone(),
                        reps: BTreeMap::new(),
                        merged: (cl.role == Role::Terrain).then(Merged::default),
                        parent: (cl.role == Role::Stratum).then(|| bro.clone().unwrap_or_else(|| handle_text(h))),
                        road,
                        bbox: [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
                    });
                    by_key.insert(key, groups.len() - 1);
                    groups.len() - 1
                }
            };
            groups[gi].handles.push(h);
            let b = geo.bbox();
            let gb = &mut groups[gi].bbox;
            *gb = [gb[0].min(b[0]), gb[1].min(b[1]), gb[2].max(b[2]), gb[3].max(b[3])];
            if let (Some(m), Geo::Mesh { verts, tris }) = (groups[gi].merged.as_mut(), &geo) {
                let rgb = self.colour(e);
                m.add(verts, tris, rgb);
                continue;
            }
            let element = groups[gi].class.ifc != "IfcAnnotation";
            for (k, item) in self.items(e, &geo, element, true)? {
                groups[gi].reps.entry(k).or_default().push(item);
            }
            if matches!(groups[gi].class.ifc, "IfcPipeSegment" | "IfcDistributionChamberElement") {
                if let Some(item) = self.sewer_body(e, &geo, &data)? {
                    groups[gi].reps.entry(("Body", "SweptSolid")).or_default().push(item);
                }
            }
        }

        // Axes, each with its design profile.
        let mut alignments = Vec::new();
        let mut alignment_ids: HashMap<String, Id> = HashMap::new();
        for (e, data) in &axes {
            let Some(al) = Alignment::from_entity(e) else { continue };
            let pvis = if al.name.is_empty() { None } else { crate::civiel::corridor::profile_of(doc, &al.name) };
            if let Some(id) = self.alignment(e, &al, data, pvis.as_deref())? {
                alignments.push(id);
                alignment_ids.insert(al.name.clone(), id);
            }
        }
        report.alignments = alignments.len();

        // An excavation (IfcEarthworksCut) is an opening: it cuts the existing
        // ground and is not itself in the spatial structure. Without a ground
        // surface it stays a geographic element.
        // Empty terrain groups are skipped below. An outline, or a mesh whose
        // faces collapse during vertex merging, cannot host an excavation.
        let has_ground = groups.iter().any(|g| {
            g.class.role == Role::Terrain && g.class.predefined == Some("TERRAIN")
                && g.merged.as_ref().is_some_and(|m| !m.tris.is_empty())
        });
        if !has_ground {
            for g in groups.iter_mut().filter(|g| g.class.ifc == "IfcEarthworksCut") {
                g.class = class("IfcGeographicElement", Some("USERDEFINED"), g.class.object_type.clone());
            }
        }
        let mut ground: Option<Id> = None;
        let mut cuts: Vec<Id> = Vec::new();
        // A marking (IfcSurfaceFeature) adheres to the pavement it lies on;
        // with no pavement beneath it is a civil element.
        let hosts: Vec<Option<usize>> = groups
            .iter()
            .map(|g| {
                if g.class.ifc != "IfcSurfaceFeature" {
                    return None;
                }
                let c = [(g.bbox[0] + g.bbox[2]) / 2.0, (g.bbox[1] + g.bbox[3]) / 2.0];
                groups
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.class.ifc == "IfcPavement" && c[0] >= p.bbox[0] && c[0] <= p.bbox[2] && c[1] >= p.bbox[1] && c[1] <= p.bbox[3])
                    .min_by(|a, b| area(&a.1.bbox).total_cmp(&area(&b.1.bbox)))
                    .map(|(i, _)| i)
            })
            .collect();
        for (g, host) in groups.iter_mut().zip(&hosts) {
            if g.class.ifc == "IfcSurfaceFeature" && host.is_none() {
                g.class = class("IfcCivilElement", None, g.class.object_type.clone());
            }
        }
        let mut written: Vec<Option<Id>> = vec![None; groups.len()];
        let mut features: Vec<(usize, Id)> = Vec::new();

        // Elements, buildings, boreholes and roads.
        let mut contained: Vec<Id> = Vec::new();
        let mut buildings: Vec<Id> = Vec::new();
        let mut strata: BTreeMap<String, (Vec<Id>, String)> = BTreeMap::new();
        let mut roads: BTreeMap<String, Vec<Id>> = BTreeMap::new();
        // A corridor's design surface split into road parts and earthworks.
        // Feature lines that run through a corridor's design surface belong
        // to its road (the corridor gives only its axis line object data).
        let mut joins: Vec<(usize, String)> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            if g.road.is_some() || g.handles.len() != 1 {
                continue;
            }
            let Some(EntityType::Polyline3D(p)) = doc.get_entity(g.handles[0]) else { continue };
            let Some(v) = p.vertices.first() else { continue };
            let q = fr.p(v.position.x, v.position.y, v.position.z);
            let key = ((q[0] * 1000.0).round() as i64, (q[1] * 1000.0).round() as i64, (q[2] * 1000.0).round() as i64);
            let on = groups.iter().find(|s| s.class.role == Role::Terrain && s.road.is_some() && s.merged.as_ref().is_some_and(|m| m.index.contains_key(&key)));
            if let Some(r) = on.and_then(|s| s.road.clone()) {
                joins.push((gi, r));
            }
        }
        for (gi, r) in joins {
            groups[gi].road = Some(r);
        }
        // The cut voids the existing ground: only with a ground surface in the file.
        let ground_tin = groups
            .iter()
            .any(|g| g.class.role == Role::Terrain && g.class.predefined == Some("TERRAIN") && g.road.is_none())
            .then(|| crate::civiel::terrein::Tin::of_status(doc, 'B'))
            .flatten();
        let mut road_parts: BTreeMap<String, RoadParts> = BTreeMap::new();
        for (gi, gr) in groups.iter_mut().enumerate() {
            if let (Role::Terrain, Some(road), Some(m)) = (gr.class.role, gr.road.clone(), gr.merged.as_ref()) {
                if let Some(rp) = self.corridor_parts(gr, m, ground_tin.as_ref(), report)? {
                    cuts.extend(rp.cut);
                    road_parts.insert(road, rp);
                    continue;
                }
            }
            if let Some(m) = gr.merged.take() {
                if m.tris.is_empty() {
                    continue;
                }
                let item = self.face_set(&m.verts, &m.tris)?;
                if m.colours.len() == 1 {
                    let s = self.surface_style(m.colours[0], 0)?;
                    self.style(item, s)?;
                } else {
                    let list: Vec<String> = m.colours.iter().map(|c| format!("({})", rgb_args(*c))).collect();
                    let colours = self.step.add("IFCCOLOURRGBLIST", &format!("({})", list.join(",")))?;
                    let index: Vec<String> = m.face_colour.iter().map(|c| (c + 1).to_string()).collect();
                    self.step.add("IFCINDEXEDCOLOURMAP", &format!("{item},$,{colours},({})", index.join(",")))?;
                    // A plain style too, for viewers without colour maps.
                    let s = self.surface_style(m.colours[0], 0)?;
                    self.style(item, s)?;
                }
                self.layer_items.entry(gr.layer.clone()).or_default().push(item);
                gr.reps.entry(("Body", "Tessellation")).or_default().push(item);
            }
            let shape = self.shape(&gr.reps)?;
            let key = format!("{}:{}", gr.class.ifc, gr.handles.first().map(|h| handle_text(*h)).unwrap_or_default());
            let tag = gr.handles.first().map(|h| handle_text(*h));
            let description = format!("DWG {}", gr.handles.iter().take(8).map(|h| handle_text(*h)).collect::<Vec<_>>().join(" "));
            let id = match gr.class.role {
                Role::Building => {
                    let g = self.guid(&format!("el:{key}"));
                    let terrain = get(&gr.data, "maaiveld_m").and_then(|v| v.parse::<f64>().ok()).map(real).unwrap_or_else(|| "$".into());
                    let id = self.step.add(
                        "IFCBUILDING",
                        &format!("{g},$,{},{},{},{},{shape},$,.ELEMENT.,$,{terrain},$", text(&gr.name), text(&description), opt_text(gr.class.object_type.as_deref()), self.c.placement),
                    )?;
                    buildings.push(id);
                    id
                }
                _ => {
                    let id = self.element(&key, &gr.class, &gr.name, &description, tag.as_deref(), &shape)?;
                    if gr.class.role == Role::Terrain && gr.class.predefined == Some("TERRAIN") && (ground.is_none() || gr.ln.as_ref().is_some_and(|l| l.status == nlcs::Status::B)) {
                        ground = Some(id);
                    }
                    written[gi] = Some(id);
                    match (&gr.class.role, &gr.parent, &gr.road) {
                        _ if gr.class.ifc == "IfcEarthworksCut" => cuts.push(id),
                        _ if gr.class.ifc == "IfcSurfaceFeature" => features.push((hosts[gi].unwrap_or(0), id)),
                        (Role::Stratum, Some(p), _) => strata.entry(p.clone()).or_insert_with(|| (Vec::new(), gr.class.object_type.clone().unwrap_or_default())).0.push(id),
                        (_, _, Some(r)) => roads.entry(r.clone()).or_default().push(id),
                        _ => contained.push(id),
                    }
                    id
                }
            };
            *report.classes.entry(if gr.class.role == Role::Building { "IfcBuilding".into() } else { gr.class.ifc.to_string() }).or_insert(0) += 1;
            let (layer, ln, data) = (gr.layer.clone(), gr.ln.clone(), std::mem::take(&mut gr.data));
            self.psets(&key, id, &layer, ln.as_ref(), &data)?;
            let sewer = sewer_props(&data);
            if !sewer.is_empty() {
                self.pset(&format!("{key}:riool"), "OpenInfra_Sewer", &sewer, id)?;
            }
        }
        if let Some(ground) = ground {
            for (i, cut) in cuts.iter().enumerate() {
                let g = self.guid(&format!("rel:voids:{i}"));
                self.step.add("IFCRELVOIDSELEMENT", &format!("{g},$,$,$,{ground},{cut}"))?;
            }
        }
        let mut by_host: BTreeMap<usize, Vec<Id>> = BTreeMap::new();
        for (host, id) in features {
            by_host.entry(host).or_default().push(id);
        }
        for (host, members) in by_host {
            if let Some(pavement) = written[host] {
                let g = self.guid(&format!("rel:adheres:{host}"));
                self.step.add("IFCRELADHERESTOELEMENT", &format!("{g},$,$,$,{pavement},{}", refs(&members)))?;
            }
        }
        for (bro, (members, kind)) in &strata {
            let g = self.guid(&format!("borehole:{bro}"));
            let name = format!("{} {bro}", if kind.to_ascii_lowercase().contains("sondering") { "Sondering" } else { "Boring" });
            let hole = self.step.add("IFCBOREHOLE", &format!("{g},$,{},$,{},{},$,{}", text(&name), opt_text(Some(kind)), self.c.placement, text(bro)))?;
            let g = self.guid(&format!("rel:borehole:{bro}"));
            self.step.add("IFCRELAGGREGATES", &format!("{g},$,$,$,{hole},{}", refs(members)))?;
            contained.push(hole);
            *report.classes.entry("IfcBorehole".into()).or_insert(0) += 1;
        }
        let mut facilities = buildings.clone();
        let mut names: Vec<&String> = roads.keys().chain(road_parts.keys()).collect();
        names.sort();
        names.dedup();
        for name in names {
            let g = self.guid(&format!("road:{name}"));
            let road = self.step.add("IFCROAD", &format!("{g},$,{},$,$,{},$,$,.ELEMENT.,$", text(name), self.c.placement))?;
            let mut members = roads.get(name).cloned().unwrap_or_default();
            if let Some(rp) = road_parts.get(name) {
                members.extend(&rp.members);
                self.road_parts(name, road, rp, &alignment_ids, report)?;
            }
            if !members.is_empty() {
                let g = self.guid(&format!("rel:road:{name}"));
                self.contain(&g, &members, road)?;
            }
            facilities.push(road);
            *report.classes.entry("IfcRoad".into()).or_insert(0) += 1;
        }
        if !facilities.is_empty() {
            let g = self.guid("rel:site-facilities");
            self.step.add("IFCRELAGGREGATES", &format!("{g},$,$,$,{},{}", self.site, refs(&facilities)))?;
        }
        if !contained.is_empty() {
            let site = self.site;
            for (i, chunk) in contained.chunks(20_000).enumerate() {
                let g = self.guid(&format!("rel:site-contains:{i}"));
                self.contain(&g, chunk, site)?;
            }
        }
        if !alignments.is_empty() {
            let g = self.guid("rel:project-alignments");
            self.step.add("IFCRELAGGREGATES", &format!("{g},$,$,$,{},{}", self.project, refs(&alignments)))?;
            let g = self.guid("rel:site-alignments");
            self.step.add("IFCRELREFERENCEDINSPATIALSTRUCTURE", &format!("{g},$,$,$,{},{}", refs(&alignments), self.site))?;
        }
        let layers = std::mem::take(&mut self.layer_items);
        for (name, items) in layers {
            self.step.add("IFCPRESENTATIONLAYERASSIGNMENT", &format!("{},$,{},$", text(&name), refs(&items)))?;
        }
        if report.alignments > 0 {
            *report.classes.entry("IfcAlignment".into()).or_insert(0) += report.alignments;
        }
        Ok(())
    }

    fn contain(&mut self, guid: &str, members: &[Id], structure: Id) -> io::Result<()> {
        self.step.add("IFCRELCONTAINEDINSPATIALSTRUCTURE", &format!("{guid},$,$,$,{},{structure}", refs(members))).map(|_| ())
    }

    fn point2(&mut self, p: [f64; 2]) -> io::Result<Id> {
        self.step.add("IFCCARTESIANPOINT", &format!("({},{})", coord(p[0]), coord(p[1])))
    }

    fn placement2(&mut self, p: [f64; 2], dir: f64) -> io::Result<Id> {
        let pt = self.point2(p)?;
        let d = self.step.add("IFCDIRECTION", &format!("({},{})", real(dir.cos()), real(dir.sin())))?;
        self.step.add("IFCAXIS2PLACEMENT2D", &format!("{pt},{d}"))
    }

    fn curve_segment(&mut self, transition: &str, place: Id, length: f64, parent: Id) -> io::Result<Id> {
        self.step.add("IFCCURVESEGMENT", &format!(".{transition}.,{place},IFCLENGTHMEASURE(0.),IFCLENGTHMEASURE({}),{parent}", real(length)))
    }

    /// IfcAlignment with its horizontal layout and, with a design profile,
    /// the vertical one; the axis geometry as IfcCompositeCurve and
    /// IfcGradientCurve; a station referent at the start.
    fn alignment(&mut self, e: &EntityType, al: &Alignment, data: &Data, pvis: Option<&[Pvi]>) -> io::Result<Option<Id>> {
        let mut h: Vec<HSeg> = alignment::horizontal(al, self.fr.f);
        if h.is_empty() {
            return Ok(None);
        }
        for s in h.iter_mut() {
            s.start = [s.start[0] + self.fr.dx, s.start[1] + self.fr.dy];
        }
        let v: Vec<VSeg> = pvis.map(alignment::vertical).unwrap_or_default();
        let handle = handle_text(e.common().handle);
        let key = format!("alignment:{handle}");

        // Geometry: the horizontal composite curve …
        let mut segs = Vec::new();
        for (i, s) in h.iter().enumerate() {
            let next = h.get(i + 1).cloned().unwrap_or_else(|| {
                let (p, d) = s.end();
                HSeg::new(p, d, 0.0, 0.0)
            });
            let place = self.placement2(s.start, s.dir)?;
            if let Some((a, t0)) = s.clothoid() {
                // A clothoid: the part of an IfcClothoid from parameter t0 on.
                let parent = self.step.add("IFCCLOTHOID", &format!("{},{}", self.c.identity2, real(a)))?;
                segs.push(self.step.add(
                    "IFCCURVESEGMENT",
                    &format!(".{}.,{place},IFCLENGTHMEASURE({}),IFCLENGTHMEASURE({}),{parent}", alignment::transition(s, &next), real(t0), real(s.length)),
                )?);
                continue;
            }
            let (parent, length) = if s.is_line() {
                (self.c.line2, s.length)
            } else {
                (self.step.add("IFCCIRCLE", &format!("{},{}", self.c.identity2, real(s.radius.abs())))?, s.length * s.radius.signum())
            };
            segs.push(self.curve_segment(alignment::transition(s, &next), place, length, parent)?);
        }
        let (end, end_dir) = h.last().unwrap().end();
        let place = self.placement2(end, end_dir)?;
        segs.push(self.curve_segment("DISCONTINUOUS", place, 0.0, self.c.line2)?);
        let composite = self.step.add("IFCCOMPOSITECURVE", &format!("{},.F.", refs(&segs)))?;

        // … and the vertical gradient curve over it.
        let mut reps = Vec::new();
        if v.is_empty() {
            reps.push(self.step.add("IFCSHAPEREPRESENTATION", &format!("{},'Axis','Curve2D',({composite})", self.c.axis))?);
        } else {
            let mut vsegs = Vec::new();
            for (i, s) in v.iter().enumerate() {
                let transition = v.get(i + 1).map(|n| alignment::vertical_transition(s, n)).unwrap_or("CONTSAMEGRADIENT");
                let angle = s.g0.atan();
                let place = self.placement2([s.start, s.z], angle)?;
                let (parent, length) = match s.kind {
                    VKind::Gradient => (self.c.line2, s.length / angle.cos()),
                    VKind::Parabola => {
                        let c = (s.g1 - s.g0) / (2.0 * s.length);
                        let curve = self.step.add("IFCPOLYNOMIALCURVE", &format!("{},(0.,1.),{},$", self.c.identity2, step::reals(&[s.z, s.g0, c])))?;
                        (curve, alignment::parabola_length(s.g0, c, s.length))
                    }
                };
                vsegs.push(self.curve_segment(transition, place, length, parent)?);
            }
            let last = v.last().unwrap();
            let place = self.placement2([last.start + last.length, last.end_z()], last.g1.atan())?;
            vsegs.push(self.curve_segment("DISCONTINUOUS", place, 0.0, self.c.line2)?);
            let gradient = self.step.add("IFCGRADIENTCURVE", &format!("{},.F.,{composite},$", refs(&vsegs)))?;
            reps.push(self.step.add("IFCSHAPEREPRESENTATION", &format!("{},'FootPrint','Curve2D',({composite})", self.c.axis))?);
            reps.push(self.step.add("IFCSHAPEREPRESENTATION", &format!("{},'Axis','Curve3D',({gradient})", self.c.axis))?);
        }
        let shape = self.step.add("IFCPRODUCTDEFINITIONSHAPE", &format!("$,$,{}", refs(&reps)))?;
        let name = if al.name.is_empty() { format!("{} {handle}", crate::lang::tr("As", "Alignment")) } else { al.name.clone() };
        let g = self.guid(&format!("el:{key}"));
        let alignment = self.step.add("IFCALIGNMENT", &format!("{g},$,{},{},$,{},{shape},$", text(&name), text(&format!("DWG {handle}")), self.c.placement))?;

        // Semantics: the layouts and their segments.
        let g = self.guid(&format!("{key}:horizontal"));
        let horizontal = self.step.add("IFCALIGNMENTHORIZONTAL", &format!("{g},$,$,$,$,$,$"))?;
        let mut layouts = vec![horizontal];
        let mut hs = Vec::new();
        for (i, s) in h.iter().chain(std::iter::once(&HSeg::new(end, end_dir, 0.0, 0.0))).enumerate() {
            let p = self.point2(s.start)?;
            let dp = self.step.add(
                "IFCALIGNMENTHORIZONTALSEGMENT",
                &format!("$,$,{p},{},{},{},{},$,.{}.", real(s.dir.rem_euclid(std::f64::consts::TAU)), real(s.radius), real(s.end_radius), real(s.length), s.kind()),
            )?;
            let g = self.guid(&format!("{key}:h{i}"));
            hs.push(self.step.add("IFCALIGNMENTSEGMENT", &format!("{g},$,$,$,$,$,$,{dp}"))?);
        }
        let g = self.guid(&format!("{key}:hnest"));
        self.step.add("IFCRELNESTS", &format!("{g},$,$,$,{horizontal},{}", refs(&hs)))?;
        if !v.is_empty() {
            let g = self.guid(&format!("{key}:vertical"));
            let vertical = self.step.add("IFCALIGNMENTVERTICAL", &format!("{g},$,$,$,$,$,$"))?;
            layouts.push(vertical);
            let last = v.last().unwrap().clone();
            let end_seg = VSeg { kind: VKind::Gradient, start: last.start + last.length, length: 0.0, z: last.end_z(), g0: last.g1, g1: last.g1 };
            let mut vs = Vec::new();
            for (i, s) in v.iter().chain(std::iter::once(&end_seg)).enumerate() {
                let radius = s.radius().map(real).unwrap_or_else(|| "$".into());
                let dp = self.step.add(
                    "IFCALIGNMENTVERTICALSEGMENT",
                    &format!("$,$,{},{},{},{},{},{radius},.{}.", real(s.start), real(s.length), real(s.z), real(s.g0), real(s.g1), s.kind_name()),
                )?;
                let g = self.guid(&format!("{key}:v{i}"));
                vs.push(self.step.add("IFCALIGNMENTSEGMENT", &format!("{g},$,$,$,$,$,$,{dp}"))?);
            }
            let g = self.guid(&format!("{key}:vnest"));
            self.step.add("IFCRELNESTS", &format!("{g},$,$,$,{vertical},{}", refs(&vs)))?;
        }
        // The cant of a rail track (IfcAlignmentCant), from its object data:
        // the outer rail raised, constant in curves, linear along clothoids.
        let cant = get(data, "verkanting_mm").map(|c| crate::civiel::spoor::cant_segments(al, self.fr.f, c)).unwrap_or_default();
        if let Some(last) = cant.last().cloned() {
            let g = self.guid(&format!("{key}:cant"));
            let layout = self.step.add("IFCALIGNMENTCANT", &format!("{g},$,$,$,$,$,$,{}", real(crate::civiel::spoor::RAIL_HEAD_DISTANCE)))?;
            layouts.push(layout);
            let end_seg = crate::civiel::spoor::CantSeg { s0: last.s0 + last.len, len: 0.0, d0: last.d1, d1: last.d1, side: last.side };
            let mut cs = Vec::new();
            for (i, c) in cant.iter().chain(std::iter::once(&end_seg)).enumerate() {
                // Left and right rail (m): the rail on the outside of the curve goes up.
                let rails = |d: f64| if c.side > 0.0 { (0.0, d / 1000.0) } else if c.side < 0.0 { (d / 1000.0, 0.0) } else { (0.0, 0.0) };
                let ((l0, r0), (l1, r1)) = (rails(c.d0), rails(c.d1));
                let linear = (c.d1 - c.d0).abs() > 1e-9;
                let ends = if linear { (real(l1), real(r1)) } else { ("$".to_string(), "$".to_string()) };
                let dp = self.step.add(
                    "IFCALIGNMENTCANTSEGMENT",
                    &format!("$,$,{},{},{},{},{},{},.{}.", real(c.s0), real(c.len), real(l0), ends.0, real(r0), ends.1, if linear { "LINEARTRANSITION" } else { "CONSTANTCANT" }),
                )?;
                let g = self.guid(&format!("{key}:c{i}"));
                cs.push(self.step.add("IFCALIGNMENTSEGMENT", &format!("{g},$,$,$,$,$,$,{dp}"))?);
            }
            let g = self.guid(&format!("{key}:cnest"));
            self.step.add("IFCRELNESTS", &format!("{g},$,$,$,{layout},{}", refs(&cs)))?;
        }
        let g = self.guid(&format!("{key}:layouts"));
        self.step.add("IFCRELNESTS", &format!("{g},$,$,$,{alignment},{}", refs(&layouts)))?;

        // Stationing: a referent at the start.
        let along = self.step.add("IFCPOINTBYDISTANCEEXPRESSION", &format!("IFCLENGTHMEASURE(0.),$,$,$,{composite}"))?;
        let linear = self.step.add("IFCAXIS2PLACEMENTLINEAR", &format!("{along},$,$"))?;
        let s0 = h[0].start;
        let z0 = v.first().map(|s| s.z).unwrap_or(0.0);
        let p0 = self.step.add("IFCCARTESIANPOINT", &format!("({},{},{})", coord(s0[0]), coord(s0[1]), coord(z0)))?;
        let d0 = self.step.add("IFCDIRECTION", &format!("({},{},0.)", real(h[0].dir.cos()), real(h[0].dir.sin())))?;
        let cart = self.step.add("IFCAXIS2PLACEMENT3D", &format!("{p0},{},{d0}", self.c.z_axis))?;
        let lp = self.step.add("IFCLINEARPLACEMENT", &format!("$,{linear},{cart}"))?;
        let g = self.guid(&format!("{key}:referent"));
        let referent = self.step.add("IFCREFERENT", &format!("{g},$,{},$,$,{lp},$,.STATION.", text(&crate::civiel::station(al.start))))?;
        let station = self.step.add("IFCPROPERTYSINGLEVALUE", &format!("'Station',$,IFCLENGTHMEASURE({}),$", real(al.start)))?;
        let g = self.guid(&format!("{key}:stationing"));
        let pset = self.step.add("IFCPROPERTYSET", &format!("{g},$,'Pset_Stationing',$,({station})"))?;
        let g = self.guid(&format!("{key}:stationing-rel"));
        self.step.add("IFCRELDEFINESBYPROPERTIES", &format!("{g},$,$,$,({referent}),{pset}"))?;
        let g = self.guid(&format!("{key}:referents"));
        self.step.add("IFCRELNESTS", &format!("{g},$,$,$,{alignment},({referent})"))?;

        let layer = e.common().layer.clone();
        let ln = nlcs::parse_layer(&layer).ok();
        self.psets(&key, alignment, &layer, ln.as_ref(), data)?;
        self.layer_items.entry(layer).or_default().push(composite);
        Ok(Some(alignment))
    }
}

// ── Corridors: road parts and earthworks (stream uitwisseling) ───────────────

/// What a corridor's design surface became.
struct RoadParts {
    /// Road parts: name, IfcRoadPartTypeEnum, the elements they contain.
    parts: Vec<(String, &'static str, Vec<Id>)>,
    /// Elements of the road itself (the fill body; for a dike everything).
    members: Vec<Id>,
    /// The cut body: it voids the existing ground.
    cut: Option<Id>,
    /// The corridor's axis, which positions the road.
    axis: String,
}

impl<W: Write> Exporter<'_, W> {
    /// The corridor's design surface as road parts and earthworks: each
    /// triangle by the offset of its centre from the axis (pavement up to the
    /// asphalt edge, then the shoulder or crest, then the slopes), and the
    /// fill and cut bodies against the existing ground (`ground`, drawing
    /// units). `None` when the corridor's axis is not in the drawing.
    fn corridor_parts(&mut self, gr: &Group, m: &Merged, ground: Option<&crate::civiel::terrein::Tin>, report: &mut Report) -> io::Result<Option<RoadParts>> {
        use corridor::{Edges, Part};
        let tr = crate::lang::tr;
        let (doc, fr) = (self.src.doc, self.fr);
        let Some(axis_name) = get(&gr.data, "as").map(str::to_string) else { return Ok(None) };
        let road = gr.road.clone().unwrap_or_default();
        let al = doc
            .model_space_entities()
            .filter(|e| matches!(e, EntityType::LwPolyline(_)) && e.common().layer.contains("-AM-AS-"))
            .filter(|e| {
                let d = data_of(e);
                get(&d, "as") == Some(axis_name.as_str()) && ["verschuiving_m", "corridor", "ontwerpprofiel", "lengteprofiel"].iter().all(|k| get(&d, k).is_none())
            })
            .find_map(Alignment::from_entity);
        let Some(al) = al else { return Ok(None) };
        let to_drawing = |p: &[f64; 3]| [(p[0] - fr.dx) * fr.f, (p[1] - fr.dy) * fr.f];
        // The parts' edges: the feature lines that run through this surface.
        let mut edges = Edges::default();
        for e in doc.model_space_entities() {
            let EntityType::Polyline3D(p) = e else { continue };
            let Some(v) = p.vertices.first() else { continue };
            let q = fr.p(v.position.x, v.position.y, v.position.z);
            let key = ((q[0] * 1000.0).round() as i64, (q[1] * 1000.0).round() as i64, (q[2] * 1000.0).round() as i64);
            if m.index.contains_key(&key) {
                edges.add(&e.common().layer, al.station_offset([v.position.x, v.position.y]).1 / fr.f);
            }
        }
        let mut tris = m.tris.clone();
        corridor::upward(&m.verts, &mut tris);
        let parts: Vec<Part> = tris
            .iter()
            .map(|t| {
                let c = t.iter().fold([0.0; 3], |s, i| [s[0] + m.verts[*i][0] / 3.0, s[1] + m.verts[*i][1] / 3.0, 0.0]);
                edges.part(al.station_offset(to_drawing(&c)).1 / fr.f)
            })
            .collect();
        let is_road = edges.pavement.is_some();
        let key = format!("corridor:{road}");
        let layer = gr.layer.clone();
        let mut out = RoadParts { parts: Vec::new(), members: Vec::new(), cut: None, axis: axis_name };
        // A triangulated body as a product shape, styled and on the corridor's layer.
        let body = |x: &mut Self, pts: &[[f64; 3]], tris: &[[usize; 3]], closed: Option<bool>, rgb: (u8, u8, u8), transparency: u8| -> io::Result<String> {
            let list = x.step.add("IFCCARTESIANPOINTLIST3D", &format!("{},$", coord_list(pts)))?;
            let closed = closed.map(|c| if c { ".T." } else { ".F." }).unwrap_or("$");
            let item = x.step.add("IFCTRIANGULATEDFACESET", &format!("{list},$,{closed},{},$", index_list(tris)))?;
            let s = x.surface_style(rgb, transparency)?;
            x.style(item, s)?;
            x.layer_items.entry(layer.clone()).or_default().push(item);
            let mut reps = BTreeMap::new();
            reps.insert(("Body", "Tessellation"), vec![item]);
            x.shape(&reps)
        };
        // The surface parts as courses: parts, key, name, course type, colour, road part.
        struct Course {
            want: &'static [Part],
            id: &'static str,
            name: &'static str,
            kind: &'static str,
            rgb: (u8, u8, u8),
            part_kind: &'static str,
        }
        let courses = [
            Course { want: &[Part::Pavement], id: "deklaag", name: tr("Verharding bovenvlak", "Pavement surface"), kind: "PAVEMENT", rgb: (72, 72, 78), part_kind: "CARRIAGEWAY" },
            Course {
                want: &[Part::Shoulder, Part::Crest],
                id: "berm",
                name: if is_road { tr("Berm", "Shoulder") } else { tr("Kruin", "Crest") },
                kind: "PROTECTION",
                rgb: (112, 158, 82),
                part_kind: "SOFTSHOULDER",
            },
            Course { want: &[Part::Slope], id: "talud", name: tr("Talud", "Slope"), kind: "PROTECTION", rgb: (136, 176, 96), part_kind: "ROADSIDE" },
        ];
        for Course { want, id, name, kind, rgb, part_kind } in courses {
            let (pts, t) = corridor::part_mesh(&m.verts, &tris, &parts, want);
            if t.is_empty() {
                continue;
            }
            let shape = body(self, &pts, &t, Some(false), rgb, 0)?;
            let course = self.element(&format!("{key}:{id}"), &class("IfcCourse", Some(kind), None), name, &road, None, &shape)?;
            *report.classes.entry("IfcCourse".into()).or_insert(0) += 1;
            let element = if kind == "PAVEMENT" {
                // The course is the pavement's top: the assembly has no build-up.
                let desc = tr("rijstroken (alleen het bovenvlak; het sjabloon heeft geen opbouw)", "lanes (top surface only; the assembly has no layer build-up)");
                let pavement = self.element(&format!("{key}:verharding"), &class("IfcPavement", Some("FLEXIBLE"), None), tr("Verharding", "Pavement"), desc, None, "$")?;
                let g = self.guid(&format!("rel:{key}:verharding"));
                self.step.add("IFCRELAGGREGATES", &format!("{g},$,$,$,{pavement},({course})"))?;
                *report.classes.entry("IfcPavement".into()).or_insert(0) += 1;
                self.psets(&format!("{key}:verharding"), pavement, &gr.layer, gr.ln.as_ref(), &gr.data)?;
                pavement
            } else {
                course
            };
            if is_road {
                let part_name = match part_kind {
                    "CARRIAGEWAY" => tr("Rijbaan", "Carriageway"),
                    "SOFTSHOULDER" => tr("Berm", "Shoulder"),
                    _ => tr("Talud", "Slope"),
                };
                out.parts.push((part_name.to_string(), part_kind, vec![element]));
            } else {
                out.members.push(element);
            }
        }
        // Earthworks between the design and the existing ground.
        if let Some(tin) = ground {
            let under: Vec<Option<f64>> = m.verts.iter().map(|p| tin.height(to_drawing(p)).map(|z| z / fr.f)).collect();
            for fill in [true, false] {
                let b = corridor::earth_body(&m.verts, &under, &tris, fill);
                if b.triangles.is_empty() {
                    continue;
                }
                let (id, name, ifc, kind, rgb) = if fill {
                    ("ophoging", if is_road { tr("Ophoging", "Fill") } else { tr("Dijklichaam", "Dike body") }, "IfcEarthworksFill", "EMBANKMENT", (170, 128, 86))
                } else {
                    ("ontgraving", tr("Ontgraving", "Cut"), "IfcEarthworksCut", "CUT", (204, 120, 64))
                };
                let shape = body(self, &b.points, &b.triangles, Some(b.closed), rgb, 100)?;
                let desc = tr("tussen het ontwerp en het bestaande maaiveld", "between the design and the existing ground");
                let e = self.element(&format!("{key}:{id}"), &class(ifc, Some(kind), None), name, desc, None, &shape)?;
                let props: Data = vec![("Volume".into(), format!("{:.3}", b.volume)), ("Closed".into(), if b.closed { "true".into() } else { "false".into() })];
                self.pset(&format!("{key}:{id}:earthworks"), "OpenInfra_Earthworks", &props, e)?;
                *report.classes.entry(ifc.into()).or_insert(0) += 1;
                if fill {
                    out.members.push(e);
                } else {
                    out.cut = Some(e);
                }
            }
        }
        Ok(Some(out))
    }

    /// The road's parts, each containing its element, and the alignment
    /// that positions the road.
    fn road_parts(&mut self, name: &str, road: Id, rp: &RoadParts, alignments: &HashMap<String, Id>, report: &mut Report) -> io::Result<()> {
        let mut parts = Vec::new();
        for (part_name, kind, members) in &rp.parts {
            let g = self.guid(&format!("roadpart:{name}:{kind}"));
            let part = self.step.add("IFCROADPART", &format!("{g},$,{},$,$,{},$,$,$,.LONGITUDINAL.,.{kind}.", text(part_name), self.c.placement))?;
            let g = self.guid(&format!("rel:roadpart:{name}:{kind}"));
            self.contain(&g, members, part)?;
            parts.push(part);
            *report.classes.entry("IfcRoadPart".into()).or_insert(0) += 1;
        }
        if !parts.is_empty() {
            let g = self.guid(&format!("rel:road-parts:{name}"));
            self.step.add("IFCRELAGGREGATES", &format!("{g},$,$,$,{road},{}", refs(&parts)))?;
        }
        if let Some(al) = alignments.get(&rp.axis) {
            let g = self.guid(&format!("rel:positions:{name}"));
            self.step.add("IFCRELPOSITIONS", &format!("{g},$,$,$,{al},({road})"))?;
        }
        Ok(())
    }
}

fn is_ring(l: &ocs_plugin_api::host::codec::LwPolyline) -> bool {
    let (Some(a), Some(b)) = (l.vertices.first(), l.vertices.last()) else { return false };
    l.vertices.len() >= 4 && (a.location.x - b.location.x).abs() < 1e-9 && (a.location.y - b.location.y).abs() < 1e-9
}

fn area(b: &[f64; 4]) -> f64 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn unique(seen: &mut HashSet<String>, seed: &str, key: &str) -> String {
    let mut g = step::guid(seed, key);
    let mut n = 1;
    while !seen.insert(g.clone()) {
        g = step::guid(seed, &format!("{key}#{n}"));
        n += 1;
    }
    g
}

fn handle_text(h: Handle) -> String {
    format!("{:X}", h.value())
}

fn rgb_args(c: (u8, u8, u8)) -> String {
    let f = |v: u8| real((v as f64 / 255.0 * 10000.0).round() / 10000.0);
    format!("{},{},{}", f(c.0), f(c.1), f(c.2))
}

/// A property value: a number as IfcReal, the rest as IfcLabel (ids with a
/// leading zero stay text).
fn value(v: &str) -> String {
    let t = v.replace(',', ".");
    let numeric = t.parse::<f64>().is_ok_and(f64::is_finite) && !(t.len() > 1 && t.starts_with('0') && !t.starts_with("0.")) && t.len() < 16 && !t.ends_with('.');
    if numeric {
        format!("IFCREAL({})", real(t.parse::<f64>().unwrap()))
    } else {
        format!("IFCLABEL({})", text(v))
    }
}

/// Degrees as IfcCompoundPlaneAngleMeasure (degrees, minutes, seconds,
/// millionths of a second, all with the sign of the angle).
fn compound(deg: f64) -> String {
    let sign = if deg < 0.0 { -1 } else { 1 };
    let total = (deg.abs() * 3600.0 * 1e6).round() as i64;
    let (d, rest) = (total / 3_600_000_000, total % 3_600_000_000);
    let (m, rest) = (rest / 60_000_000, rest % 60_000_000);
    let (s, micro) = (rest / 1_000_000, rest % 1_000_000);
    format!("({},{},{},{})", sign * d, sign * m, sign * s, sign * micro)
}

/// Layer colours: from the layer table, or from the NLCS table for a
/// drawing shared without one.
fn layer_colours(doc: &CadDocument) -> HashMap<String, (u8, u8, u8)> {
    let mut out = HashMap::new();
    for l in doc.layers.iter() {
        if let Some(rgb) = l.color.rgb() {
            out.insert(l.name.clone(), rgb);
        }
    }
    let names: HashSet<String> = doc.model_space_entities().map(|e| e.common().layer.clone()).collect();
    for name in names {
        if out.contains_key(&name) {
            continue;
        }
        if let Some(rgb) = nlcs_colour(&name) {
            out.insert(name, rgb);
        }
    }
    out
}

fn nlcs_colour(layer: &str) -> Option<(u8, u8, u8)> {
    let ln = nlcs::parse_layer(layer).ok()?;
    let obj = nlcs::object_by_name(&ln.group, &ln.object)?;
    let element = match ln.element.as_str() {
        "S" => nlcs::Element::S,
        "T" => nlcs::Element::T,
        "A" => nlcs::Element::A,
        _ => nlcs::Element::G,
    };
    let spec = nlcs::layer(obj, ln.status, &ln.discipline, element);
    Some(spec.true_color.unwrap_or(spec.rgb))
}

// ── The command ─────────────────────────────────────────────────────────────

/// The arguments of `GIS_IFC` and `CIV_IFC`: the file, selection only, texts.
pub fn args(a: &crate::civiel::api::Args) -> (Option<PathBuf>, bool, bool) {
    let path = a.str("BESTAND").or_else(|| a.str("PAD")).map(PathBuf::from);
    let selection = a.words.iter().any(|w| w.eq_ignore_ascii_case("SELECTIE") || w.eq_ignore_ascii_case("SELECTION"));
    let texts = !a.str("TEKST").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "nee" | "no" | "0" | "uit"));
    (path, selection, texts)
}

/// `GIS_IFC` / `CIV_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee]`: the drawing as IFC 4.3
/// beside it (`<tekening>_IFC4X3.ifc`).
pub fn command(path: Option<PathBuf>, selection: bool, texts: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = crate::bridge::target().ok_or("geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad")?;
    progress(crate::lang::tr("Tekening lezen…", "Reading the drawing…").into());
    let doc = crate::bridge::snapshot()?;
    let only = if selection {
        let s: HashSet<Handle> = crate::bridge::selection()?.into_iter().collect();
        if s.is_empty() {
            return Err(crate::lang::tr("selecteer eerst de objecten (of laat SELECTIE weg)", "select the objects first (or leave out SELECTIE)").into());
        }
        Some(s)
    } else {
        None
    };
    let site = crate::bridge::site();
    let name = t.doc_path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "tekening".into());
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing("IFC4X3.ifc"));
    progress(format!("IFC 4x3 → {}", path.display()));
    let src = Source { doc: &doc, f: t.unit_factor, site: site.as_ref(), name };
    let report = write(&src, &Options { only, no_texts: !texts }, &path)?;
    crate::civiel::api::note("bestand", path.to_string_lossy().into_owned());
    crate::civiel::api::note("klassen", serde_json::to_value(&report.classes).unwrap_or_default());
    crate::civiel::api::note("overgeslagen", serde_json::to_value(&report.skipped).unwrap_or_default());
    crate::civiel::api::note("assen", report.alignments);
    crate::civiel::api::note("instanties", report.instances);
    crate::civiel::api::note("crs", report.crs.clone().unwrap_or_default());
    Ok(report.summary())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::civiel::as_profiel::Transition;
    use crate::nlcs::{Element, Status};
    use ocs_plugin_api::host::codec::entities::{Mesh, MeshFace, Polyline3D, RasterImage};
    use ocs_plugin_api::host::codec::{LwPolyline, Point, Text, Vector2, Vector3};

    fn nl(group: &str, name: &str, el: Element, status: Status) -> String {
        nlcs::layer(nlcs::object_by_name(group, name).unwrap_or_else(|| panic!("{group} {name}")), status, "XX", el).name
    }

    fn on(mut e: EntityType, layer: &str, data: &[(&str, &str)]) -> EntityType {
        let c = e.common_mut();
        c.layer = layer.to_string();
        if !data.is_empty() {
            c.extended_data.add_record(crate::bridge::object_data(data.iter().map(|(k, v)| (*k, v.to_string()))));
        }
        e
    }

    fn lw(pts: &[[f64; 2]], closed: bool, elevation: f64) -> EntityType {
        let mut l = LwPolyline::new();
        for p in pts {
            l.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
        }
        l.is_closed = closed;
        l.elevation = elevation;
        EntityType::LwPolyline(l)
    }

    fn mesh(verts: &[[f64; 3]], faces: &[&[usize]], rgb: Option<(u8, u8, u8)>) -> EntityType {
        let mut m = Mesh::new();
        m.vertices = verts.iter().map(|p| Vector3::new(p[0], p[1], p[2])).collect();
        m.faces = faces.iter().map(|f| MeshFace::new(f.to_vec())).collect();
        if let Some((r, g, b)) = rgb {
            m.common.color = Color::Rgb { r, g, b };
        }
        EntityType::Mesh(m)
    }

    fn line3(pts: &[[f64; 3]]) -> EntityType {
        let mut p = Polyline3D::new();
        for q in pts {
            p.add_vertex(Vector3::new(q[0], q[1], q[2]));
        }
        EntityType::Polyline3D(p)
    }

    /// A drawing with one of everything Open Infra draws.
    pub fn sample() -> CadDocument {
        let mut doc = CadDocument::new();
        let mut add = |e: EntityType| {
            doc.add_entity(e).unwrap();
        };
        // Clothoids into and out of the curve; their records restore them exactly.
        let al = Alignment::from_layout(&[[0.0, 0.0], [100.0, 0.0], [180.0, 80.0]], &[60.0], &[Transition::Length(20.0)]).unwrap();
        let (poly, spirals) = al.to_polyline_data(0.001);
        let mut d: Vec<(&str, &str)> = vec![("as", "A1"), ("beginstation", "0.000")];
        d.extend(spirals.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        add(on(EntityType::LwPolyline(poly), &nl("AM", "AS", Element::G, Status::N), &d));
        // A rail track with cant (IfcAlignmentCant).
        let rail = Alignment::from_layout(&[[0.0, -200.0], [200.0, -200.0], [350.0, -120.0]], &[500.0], &[Transition::Length(40.0)]).unwrap();
        let (poly, spirals) = rail.to_polyline_data(0.001);
        let mut d: Vec<(&str, &str)> = vec![("as", "S1"), ("spoor", "S1"), ("snelheid_kmh", "80"), ("verkanting_mm", "100")];
        d.extend(spirals.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        add(on(EntityType::LwPolyline(poly), &nl("AM", "AS_SPOOR_TREIN", Element::G, Status::N), &d));
        // The length profile: a frame with the design profile and the ground inside.
        add(on(lw(&[[900.0, 990.0], [1200.0, 990.0], [1200.0, 1050.0], [900.0, 1050.0]], true, 0.0), &nl("AL", "PROFIELBALK", Element::G, Status::N), &[("lengteprofiel", "Lengteprofiel A1")]));
        add(on(
            lw(&[[910.0, 1000.0], [1010.0, 1020.0], [1090.0, 1010.0]], false, 0.0),
            &nl("GW", "HOOGTE_AFWERKHOOGTE", Element::G, Status::N),
            &[("ontwerpprofiel", "Ontwerpprofiel A1"), ("as", "A1"), ("pvi", "0.000:10.000;100.000:12.000;180.000:11.000"), ("boog_m", "40")],
        ));
        add(on(lw(&[[910.0, 995.0], [1090.0, 998.0]], false, 0.0), &nl("GW", "MAAIVELD", Element::G, Status::B), &[]));
        // A cross-section view: its frame returns to its start, open; the axis line inside.
        add(on(lw(&[[1300.0, 990.0], [1360.0, 990.0], [1360.0, 1020.0], [1300.0, 1020.0], [1300.0, 990.0]], false, 0.0), &nl("AL", "PROFIELBALK", Element::G, Status::N), &[]));
        add(on(lw(&[[1330.0, 995.0], [1330.0, 1018.0]], false, 0.0), &nl("AM", "AS", Element::G, Status::N), &[]));
        // A PVI tick just above the length profile's frame (the design lies higher than the ground).
        add(on(lw(&[[1010.0, 1052.0], [1010.0, 1056.0]], false, 0.0), &nl("AM", "AS", Element::G, Status::N), &[]));
        // The corridor's axis as a 3D feature line: part of the road, not a second alignment.
        add(on(line3(&[[0.0, 0.0, 10.0], [100.0, 0.0, 12.0]]), &nl("AM", "AS", Element::G, Status::N), &[("corridor", "Corridor A1"), ("as", "A1")]));
        add(on(mesh(&[[0.0, -20.0, 1.0], [200.0, -20.0, 1.5], [200.0, 100.0, 2.0], [0.0, 100.0, 1.2]], &[&[0, 1, 2], &[0, 2, 3]], None), &nl("OG", "TRIANGULATIE", Element::G, Status::B), &[("bron", "TIN")]));
        let pand = [("bron", "3D BAG LoD 2.2"), ("identificatie", "NL.IMBAG.Pand.0505100000012345"), ("bouwjaar", "1470"), ("maaiveld_m", "1.20")];
        let bebouwing = nl("OG", "BEBOUWING", Element::G, Status::B);
        add(on(mesh(&[[10.0, 10.0, 1.2], [20.0, 10.0, 1.2], [20.0, 10.0, 9.0], [10.0, 10.0, 9.0]], &[&[0, 1, 2, 3]], Some((200, 200, 190))), &bebouwing, &pand));
        add(on(mesh(&[[10.0, 10.0, 9.0], [20.0, 10.0, 9.0], [15.0, 15.0, 14.0]], &[&[0, 1, 2]], Some((160, 40, 30))), &bebouwing, &pand));
        add(on(line3(&[[0.0, 5.0, 0.4], [100.0, 5.0, 0.4]]), &nl("KL", "ET_LS", Element::G, Status::B), &[("thema", "laagspanning"), ("diameter", "0")]));
        add(on(lw(&[[0.0, 8.0], [100.0, 8.0]], false, 0.0), &nl("KL", "GAS_LD", Element::G, Status::B), &[("thema", "gasLageDruk"), ("materiaal", "PE")]));
        for (top, bottom, soil) in [(1.2, -3.0, "zand"), (-3.0, -8.5, "klei")] {
            add(on(
                mesh(&[[50.0, 50.0, top], [50.5, 50.0, top], [50.5, 50.0, bottom], [50.0, 50.0, bottom]], &[&[0, 1, 2, 3]], Some((230, 200, 80))),
                &nl("MO", "ONDERZOEK_SONDERING", Element::G, Status::B),
                &[("bron", "BRO CPT"), ("bro_id", "CPT000000012345"), ("grondsoort", soil)],
            ));
        }
        let mut pt = Point::new();
        pt.location = Vector3::new(30.0, 30.0, 1.25);
        add(on(EntityType::Point(pt), &nl("OG", "HOOGTEPUNT", Element::S, Status::B), &[("hoogte_nap", "1.250")]));
        add(on(lw(&[[0.0, 40.0], [60.0, 42.0], [120.0, 41.0]], false, 2.0), &nl("OG", "HOOGTELIJN_MAJOR", Element::G, Status::B), &[("hoogte_nap", "2.000")]));
        let label = Text::with_value("Grote Kerk \u{2014} Dordrecht".to_string(), Vector3::new(30.0, 32.0, 0.0)).with_height(2.0);
        add(on(EntityType::Text(label), &nl("OG", "HOOGTEPUNT", Element::T, Status::B), &[]));
        add(on(lw(&[[0.0, -2.0], [100.0, -2.0], [100.0, 2.0], [0.0, 2.0]], true, 0.0), &nl("VH", "GESLOTENVERHARDING_ASFALT", Element::G, Status::B), &[("bron", "BGT"), ("lokaal_id", "G0505.1234")]));
        add(on(lw(&[[40.0, -0.1], [45.0, -0.1], [45.0, 0.1], [40.0, 0.1]], true, 0.0), &nl("VW", "MARKERING", Element::G, Status::B), &[("bron", "BGT")]));
        let dtm = nl("OG", "DTM", Element::G, Status::B);
        add(on(mesh(&[[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [10.0, 10.0, 1.1]], &[&[0, 1, 2]], Some((90, 120, 60))), &dtm, &[("bron", "AHN + luchtfoto PDOK")]));
        add(on(mesh(&[[0.0, 0.0, 1.0], [10.0, 10.0, 1.1], [0.0, 10.0, 1.0]], &[&[0, 1, 2]], Some((120, 120, 120))), &dtm, &[("bron", "AHN + luchtfoto PDOK")]));
        add(on(lw(&[[0.0, 60.0], [50.0, 70.0]], false, 0.0), "GIS-OSM-WEGEN", &[("highway", "residential")]));
        add(on(EntityType::RasterImage(RasterImage::with_size("luchtfoto.png", Vector3::new(0.0, 0.0, 0.0), 100.0, 100.0, 100.0, 100.0)), "GIS-LUCHTFOTO", &[]));
        add(on(line3(&[[0.0, 12.0, 10.0], [100.0, 12.0, 11.9]]), &nl("GW", "TALUD_TEEN", Element::G, Status::N), &[("corridor", "Corridor A1"), ("as", "A1")]));
        add(on(mesh(&[[0.0, -4.0, 1.0], [100.0, -4.0, 1.0], [100.0, -8.0, 0.5]], &[&[0, 1, 2]], None), &nl("GW", "ONTGRAVINGSVAK-WERK", Element::G, Status::N), &[]));
        doc
    }

    pub fn sample_site() -> Site {
        Site {
            label: "Grote Kerk Dordrecht".into(),
            lon: 4.6612,
            lat: 51.8126,
            crs: "EPSG:28992".into(),
            crs_name: "Amersfoort / RD New".into(),
            origin_e: 105000.0,
            origin_n: 425000.0,
            absolute: false,
            ..Site::default()
        }
    }

    fn export_sample() -> (String, Report) {
        let doc = sample();
        let site = sample_site();
        let src = Source { doc: &doc, f: 1.0, site: Some(&site), name: "voorbeeld.dwg".into() };
        let mut report = Report::default();
        let out = export(&src, &Options::default(), Vec::new(), &mut report).unwrap();
        (String::from_utf8(out).unwrap(), report)
    }

    /// A corridor's bodies, a junction and an assembly drawing (corridor stream).
    #[test]
    fn corridor_parts_and_layers() {
        let mut doc = CadDocument::new();
        let mut add = |e: EntityType| {
            doc.add_entity(e).unwrap();
        };
        let road = [("corridor", "Corridor A1"), ("as", "A1")];
        let with = |extra: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> { road.iter().chain(extra).copied().collect() };
        let block = |x: f64, z: f64| mesh(&[[x, 0.0, z], [x + 10.0, 0.0, z], [x + 10.0, 3.0, z], [x, 3.0, z - 0.1]], &[&[0, 1, 2], &[0, 2, 3]], None);
        add(on(block(0.0, 1.0), &nl("VH", "GESLOTENVERHARDING_ASFALT_DEKLAAG", Element::G, Status::N), &with(&[("laag", "AC 11 surf (deklaag)"), ("rol", "verharding")])));
        add(on(block(0.0, 0.7), &nl("VH", "FUNDERING_GRANULAAT", Element::G, Status::N), &with(&[("laag", "menggranulaat (fundering)"), ("rol", "fundering")])));
        add(on(block(20.0, 1.1), &nl("VH", "KANTOPSLUITING_TROTTOIRBAND", Element::G, Status::N), &with(&[("band", "trottoirband 13/15x25")])));
        add(on(block(40.0, 0.2), &nl("GW", "WATERGANG_SLOOT", Element::G, Status::N), &with(&[("onderdeel", "sloot")])));
        add(on(block(60.0, 0.3), &nl("GW", "CUNET_BODEM", Element::G, Status::N), &with(&[("oppervlak", "onderkant verhardingsconstructie (subgrade)")])));
        add(on(line3(&[[0.0, 5.0, 1.0], [100.0, 5.0, 1.0]]), &nl("GW", "WATERGANG_SLOOT_BODEM", Element::G, Status::N), &[]));
        add(on(block(80.0, 1.0), &nl("OG", "TRIANGULATIE", Element::G, Status::N), &[("kruispunt", "Kruispunt A1 / B1")]));
        add(on(lw(&[[500.0, 0.0], [510.0, 0.0]], false, 0.0), &nl("VH", "GESLOTENVERHARDING_ASFALT_DEKLAAG", Element::G, Status::N), &[("profieltype", "stadsstraat")]));
        let src = Source { doc: &doc, f: 1.0, site: None, name: "corridor.dwg".into() };
        let mut r = Report::default();
        let out = String::from_utf8(export(&src, &Options::default(), Vec::new(), &mut r).unwrap()).unwrap();
        let n = |k: &str| r.classes.get(k).copied().unwrap_or(0);
        assert_eq!(n("IfcCourse"), 2, "{:?}", r.classes);
        assert_eq!(n("IfcKerb"), 1);
        assert_eq!(n("IfcGeographicElement"), 4, "ditch body and bottom line, subgrade, junction surface: {:?}", r.classes);
        assert_eq!(n("IfcRoad"), 2, "the corridor and the junction");
        assert_eq!(r.skipped.get("profieltype"), Some(&1), "{:?}", r.skipped);
        assert!(out.contains(".PAVEMENT.") && out.contains("'menggranulaat (fundering)'"), "courses: surfacing and a named foundation");
    }

    /// A designed sewer (CIV_RIOOL): pipes as hollow tubes at their invert
    /// levels, manholes as shafts, the BOB as properties, flow arrows as symbols.
    #[test]
    fn a_designed_sewer_becomes_pipes_and_manholes() {
        let mut doc = CadDocument::new();
        let pipe = nl("RI", "DWA_RIOOLLEIDING_PVC_250", Element::G, Status::N);
        let put = nl("RI", "DWA_RIOOLPUT_INSPECTIEPUT", Element::G, Status::N);
        doc.add_entity(on(
            lw(&[[0.0, 0.0], [40.0, 0.0]], false, 0.0),
            &pipe,
            &[("riool", "DWA1"), ("rol", "leiding"), ("leiding", "P101-P102"), ("van", "P101"), ("naar", "P102"), ("diameter", "250"), ("binnendiameter", "235.4"), ("wanddikte", "7.3"), ("materiaal", "PVC"), ("bob_van", "1.2000"), ("bob_naar", "1.0400"), ("verhang", "0.004000")],
        ))
        .unwrap();
        for (x, name, low) in [(0.0, "P101", "1.200"), (40.0, "P102", "1.040")] {
            let c = EntityType::Circle(ocs_plugin_api::host::codec::Circle::from_center_radius(Vector3::new(x, 0.0, 0.0), 0.4));
            doc.add_entity(on(c, &put, &[("riool", "DWA1"), ("rol", "put"), ("put", name), ("maaiveld", "2.900"), ("bob_laagst", low), ("afmeting", "800")])).unwrap();
        }
        doc.add_entity(on(lw(&[[19.0, 0.5], [20.0, 0.0], [19.0, -0.5]], false, 0.0), &nl("RI", "DWA_STROMINGSRICHTING", Element::G, Status::N), &[("riool", "DWA1"), ("rol", "label")])).unwrap();
        let site = sample_site();
        let src = Source { doc: &doc, f: 1.0, site: Some(&site), name: "riool.dwg".into() };
        let mut r = Report::default();
        let ifc = String::from_utf8(export(&src, &Options::default(), Vec::new(), &mut r).unwrap()).unwrap();
        assert_eq!(r.classes.get("IfcPipeSegment"), Some(&1), "{:?}", r.classes);
        assert_eq!(r.classes.get("IfcDistributionChamberElement"), Some(&2), "{:?}", r.classes);
        assert_eq!(r.classes.get("IfcAnnotation"), Some(&1), "the flow arrow: {:?}", r.classes);
        assert!(ifc.contains("IFCSWEPTDISKSOLID") && ifc.contains("IFCEXTRUDEDAREASOLID"), "3D bodies");
        assert!(ifc.contains("'OpenInfra_Sewer'") && ifc.contains("'InvertLevelStart'") && ifc.contains("'GroundLevel'"));
        assert!(ifc.contains("'P101-P102'") && ifc.contains("'P101'"), "names");
        // The tube's axis runs at the invert plus half the inner diameter.
        assert!(ifc.contains(&format!("{})", coord(1.2 + 0.1177))), "axis height");
    }

    #[test]
    fn every_part_gets_its_ifc_class() {
        let (ifc, r) = export_sample();
        let n = |k: &str| r.classes.get(k).copied().unwrap_or(0);
        assert_eq!(n("IfcAlignment"), 2, "road and rail: {:?}", r.classes);
        assert_eq!(n("IfcBuilding"), 1, "two meshes of one BAG building");
        assert_eq!(n("IfcBorehole"), 1);
        assert_eq!(n("IfcGeotechnicalStratum"), 2);
        assert_eq!(n("IfcGeographicElement"), 2, "TIN and photo terrain");
        assert_eq!(n("IfcCableSegment"), 1);
        assert_eq!(n("IfcPipeSegment"), 1);
        assert_eq!(n("IfcPavement"), 1);
        assert_eq!(n("IfcEarthworksCut"), 1);
        assert_eq!(n("IfcRoad"), 1);
        assert_eq!(n("IfcSurfaceFeature"), 1, "the marking on the asphalt");
        assert!(ifc.contains("IFCRELADHERESTOELEMENT("));
        assert!(ifc.contains("IFCRELVOIDSELEMENT("), "the excavation cuts the ground");
        assert!(n("IfcAnnotation") >= 4, "{:?}", r.classes);
        assert_eq!(r.skipped.get("kaartbeeld (raster)"), Some(&1));
        assert_eq!(r.skipped.get("profiel- en dwarsprofielweergave"), Some(&4), "{:?}", r.skipped);
        assert_eq!(r.crs.as_deref(), Some("EPSG:28992"));
        for needle in [
            "FILE_SCHEMA(('IFC4X3_ADD2'));",
            "IFCMAPCONVERSION(",
            "IFCPROJECTEDCRS('EPSG:28992'",
            ".PARABOLICARC.",
            ".CIRCULARARC.",
            ".CLOTHOID.",
            "IFCCLOTHOID(",
            "IFCALIGNMENTCANT(",
            ".LINEARTRANSITION.",
            "IFCGRADIENTCURVE(",
            "IFCINDEXEDCOLOURMAP(",
            "IFCPAVEMENT(",
            ".FLEXIBLE.",
            "'Pset_Stationing'",
            "'OpenInfra_NLCS'",
            "IFCLABEL('NL.IMBAG.Pand.0505100000012345')",
            "Grote Kerk \\X2\\2014\\X0\\ Dordrecht",
        ] {
            assert!(ifc.contains(needle), "missing {needle}");
        }
        // Local coordinates: metres from the base point; the base point in the map conversion.
        assert!(ifc.contains(",105000.,425000.,0.,1.,0.,$);"));
    }

    #[test]
    fn survey_symbols_parcels_and_water_analyses() {
        use ocs_plugin_api::host::codec::entities::Insert;
        let mut doc = CadDocument::new();
        let tree = on(EntityType::Insert(Insert::new("OI_SYMBOOL_BOOM", Vector3::new(10.0, 5.0, 1.5))), &nl("GR", "BOOM", Element::S, Status::B), &[("code", "SBM"), ("soort", "Eik")]);
        let gully = on(EntityType::Insert(Insert::new("OI_SYMBOOL_KOLK", Vector3::new(12.0, 5.0, 1.5))), &nl("RI", "GWA_KOLK", Element::S, Status::B), &[("code", "SSK")]);
        let hydrant = on(EntityType::Insert(Insert::new("OI_SYMBOOL_BRANDKRAAN", Vector3::new(14.0, 5.0, 1.5))), &nl("KL", "WATER_BRANDLEIDING_BRANDHYDRANT", Element::S, Status::B), &[("code", "SBKR")]);
        let vehicle = on(EntityType::Insert(Insert::new("OI_CAR_TOP", Vector3::new(0.0, 0.0, 0.0))), "OI-VOERTUIGEN", &[("vehicle", "car")]);
        let parcel = on(lw(&[[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]], true, 0.0), &nl("KG", "GRENS_PERCEEL", Element::G, Status::N), &[("perceel", "P1"), ("oppervlakte_m2", "400.00")]);
        let catchment = on(lw(&[[0.0, 0.0], [30.0, 0.0], [30.0, 30.0]], true, 0.0), "OI-STROOMGEBIED", &[("stroomgebied", "SG1"), ("oppervlakte_m2", "450.0")]);
        let pond = on(lw(&[[1.0, 1.0], [3.0, 1.0], [3.0, 3.0]], true, 0.0), "OI-PLAS", &[("plas", "1"), ("volume_m3", "0.40")]);
        let drop = on(line3(&[[0.0, 0.0, 2.0], [5.0, 0.0, 1.5]]), "OI-WATERDRUPPEL", &[("waterdruppel", "1")]);
        for e in [tree, gully, hydrant, vehicle, parcel, catchment, pond, drop] {
            doc.add_entity(e).unwrap();
        }
        let src = Source { doc: &doc, f: 1.0, site: None, name: "overig.dwg".into() };
        let mut r = Report::default();
        let out = String::from_utf8(export(&src, &Options::default(), Vec::new(), &mut r).unwrap()).unwrap();
        let n = |k: &str| r.classes.get(k).copied().unwrap_or(0);
        assert_eq!(n("IfcGeographicElement"), 1, "the tree: {:?}", r.classes);
        assert_eq!(n("IfcDistributionChamberElement"), 1, "the gully");
        assert_eq!(n("IfcFireSuppressionTerminal"), 1, "the hydrant");
        assert_eq!(n("IfcAnnotation"), 4, "parcel, catchment, pond and drop: {:?}", r.classes);
        assert_eq!(r.skipped.get("blokverwijzing"), Some(&1), "the vehicle stays out");
        for needle in [".VEGETATION.", ".SUMP.", ".FIREHYDRANT.", "'Stroomgebied'", "'Plas'", "'Waterdruppel'", "IFCLABEL('P1')", "IFCLABEL('SG1')"] {
            assert!(out.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn the_same_drawing_gives_the_same_ids() {
        let ids = |s: &str| s.lines().filter(|l| l.contains("=IFCBUILDING(") || l.contains("=IFCALIGNMENT(")).map(|l| l.split('\'').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
        let (a, _) = export_sample();
        let (b, _) = export_sample();
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(ids(&a).len(), 3);
    }

    fn check_excavation_without_exported_ground(terrain: EntityType, name: &str) {
        let mut doc = CadDocument::new();
        doc.add_entity(on(terrain, &nl("OG", "TRIANGULATIE", Element::G, Status::B), &[])).unwrap();
        doc.add_entity(on(
            lw(&[[0.0, 10.0], [5.0, 10.0], [5.0, 15.0], [0.0, 15.0]], true, 0.0),
            &nl("GW", "ONTGRAVINGSVAK-WERK", Element::G, Status::N), &[("controle_id", "ontgraving-behoud-017")],
        )).unwrap();
        let src = Source { doc: &doc, f: 1.0, site: None, name: format!("{name}.dwg") };
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ifc-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.ifc"));
        let report = write(&src, &Options::default(), &path).unwrap();
        let data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(report.classes.get("IfcEarthworksCut"), None, "an opening requires an exported ground host");
        assert_eq!(report.classes.get("IfcGeographicElement"), Some(&1), "preserve the excavation geometry as a geographic element");
        assert!(data.contains("IFCRELCONTAINEDINSPATIALSTRUCTURE("));
        assert!(!data.contains("IFCRELVOIDSELEMENT("));
    }

    #[test]
    fn excavation_with_collapsed_terrain_stays_geographic() {
        // Merged terrain vertices are deduplicated to millimetres. All three
        // vertices collapse, so this otherwise non-empty mesh is not emitted.
        let terrain = mesh(&[[0.0, 0.0, 0.0], [0.0001, 0.0, 0.0], [0.0, 0.0001, 0.0]], &[&[0, 1, 2]], None);
        check_excavation_without_exported_ground(terrain, "ontgraving-ingeklapt-terrein");
    }

    #[test]
    fn excavation_with_only_a_terrain_outline_stays_geographic() {
        // A terrain outline also creates a terrain group without mesh faces.
        let terrain = lw(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]], true, 0.0);
        check_excavation_without_exported_ground(terrain, "ontgraving-terreinomtrek");
    }

    #[test]
    fn values_and_angles() {
        assert_eq!(value("1470"), "IFCREAL(1470.)");
        assert_eq!(value("1,25"), "IFCREAL(1.25)");
        assert_eq!(value("0505100000012345"), "IFCLABEL('0505100000012345')");
        assert_eq!(value("zand"), "IFCLABEL('zand')");
        assert_eq!(compound(51.5), "(51,30,0,0)");
        assert_eq!(compound(-4.25), "(-4,-15,0,0)");
        assert_eq!(plain("{\\fArial|b1;Grote}\\PKerk"), "Grote Kerk");
    }

    #[test]
    fn sheets_stay_out_and_labels_follow_the_text_switch() {
        let mut doc = CadDocument::new();
        let sheet = [("bladen", "A1 A1")];
        let label = [("label", "stationering"), ("bron", "1A"), ("vingerafdruk", "0123456789abcdef")];
        let frame = lw(&[[0.0, 0.0], [600.0, 0.0], [600.0, 300.0], [0.0, 300.0]], true, 0.0);
        doc.add_entity(on(frame, &nl("AL", "TEKENBLADGRENS", Element::G, Status::N), &sheet)).unwrap();
        doc.add_entity(on(EntityType::Text(Text::with_value("BLAD 1 VAN 2".to_string(), Vector3::new(5.0, 290.0, 0.0))), &nl("AL", "TEKENBLADGRENS", Element::T, Status::N), &sheet)).unwrap();
        doc.add_entity(on(EntityType::Text(Text::with_value("AANSLUITLIJN 0+500".to_string(), Vector3::new(500.0, 50.0, 0.0))), &nl("AL", "BLADVERWIJZING_AS", Element::T, Status::N), &sheet)).unwrap();
        doc.add_entity(on(lw(&[[100.0, -2.5], [100.0, 2.5]], false, 0.0), &nl("AM", "KILOMETRERING", Element::G, Status::N), &label)).unwrap();
        doc.add_entity(on(EntityType::Text(Text::with_value("0+100".to_string(), Vector3::new(100.0, 4.0, 0.0))), &nl("AM", "KILOMETRERING", Element::T, Status::N), &label)).unwrap();
        let site = sample_site();
        let src = Source { doc: &doc, f: 1.0, site: Some(&site), name: "bladen.dwg".into() };
        let run = |opt: &Options| {
            let mut r = Report::default();
            export(&src, opt, Vec::new(), &mut r).unwrap();
            r
        };
        let r = run(&Options::default());
        assert_eq!(r.skipped.get("tekenblad en profielkader"), Some(&3), "{:?}", r.skipped);
        assert_eq!(r.classes.get("IfcAnnotation"), Some(&2), "{:?}", r.classes);
        let r = run(&Options { no_texts: true, ..Options::default() });
        // The three texts and the label's tick; the sheet frame stays a sheet part.
        assert_eq!(r.skipped.get("tekst"), Some(&4), "{:?}", r.skipped);
        assert_eq!(r.skipped.get("tekenblad en profielkader"), Some(&1), "{:?}", r.skipped);
        assert_eq!(r.classes.get("IfcAnnotation"), None, "{:?}", r.classes);
    }

    /// A road corridor over flat ground: the alignment, the existing TIN,
    /// the corridor's design surface and its feature lines.
    pub fn corridor_sample() -> CadDocument {
        let mut doc = CadDocument::new();
        let mut add = |e: EntityType| {
            doc.add_entity(e).unwrap();
        };
        let al = Alignment::from_pis(&[[0.0, 0.0], [100.0, 0.0]], &[]).unwrap();
        add(on(EntityType::LwPolyline(al.to_polyline()), &nl("AM", "AS", Element::G, Status::N), &[("as", "A2"), ("beginstation", "0.000")]));
        add(on(mesh(&[[-10.0, -20.0, 1.0], [110.0, -20.0, 1.0], [110.0, 20.0, 1.0], [-10.0, 20.0, 1.0]], &[&[0, 1, 2], &[0, 2, 3]], None), &nl("OG", "TRIANGULATIE", Element::G, Status::B), &[("terrein", "Terrein 1"), ("bron", "AHN")]));
        // The "weg" assembly 1 m above the ground, right of the axis is -y:
        // toe, shoulder, lane, axis, lane, shoulder, toe.
        let profile = [(-8.0, 1.0), (-4.75, 1.86), (-3.25, 1.92), (0.0, 2.0), (3.25, 1.92), (4.75, 1.86), (8.0, 1.0)];
        let mut verts = Vec::new();
        for x in [0.0, 50.0, 100.0] {
            for (o, z) in profile {
                verts.push([x, -o, z]);
            }
        }
        let n = profile.len();
        let mut faces: Vec<Vec<usize>> = Vec::new();
        for s in 0..2 {
            for j in 0..n - 1 {
                let (a, b) = (s * n + j, (s + 1) * n + j);
                faces.push(vec![a, a + 1, b + 1]);
                faces.push(vec![a, b + 1, b]);
            }
        }
        let faces: Vec<&[usize]> = faces.iter().map(|f| f.as_slice()).collect();
        let data = [("corridor", "Corridor A2"), ("as", "A2"), ("sjabloon", "weg")];
        add(on(mesh(&verts, &faces, None), &nl("OG", "TRIANGULATIE", Element::G, Status::N), &data));
        for (j, (o, z)) in profile.iter().enumerate() {
            let (group, object) = match j {
                0 | 6 => ("GW", "TALUD_TEEN"),
                1 | 5 => ("GR", "GRAS_BERM"),
                2 | 4 => ("VH", "GESLOTENVERHARDING_ASFALT"),
                _ => ("AM", "AS"),
            };
            let line = line3(&[[0.0, -o, *z], [50.0, -o, *z], [100.0, -o, *z]]);
            add(on(line, &nl(group, object, Element::G, Status::N), if j == 3 { &data[..] } else { &[] }));
        }
        doc
    }

    #[test]
    fn a_corridor_becomes_road_parts_and_earthworks() {
        let doc = corridor_sample();
        let site = sample_site();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ifc-test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = Source { doc: &doc, f: 1.0, site: Some(&site), name: "corridor.dwg".into() };
        let r = write(&src, &Options::default(), &dir.join("corridor.ifc")).unwrap();
        let n = |k: &str| r.classes.get(k).copied().unwrap_or(0);
        assert_eq!(n("IfcRoad"), 1, "{:?}", r.classes);
        assert_eq!(n("IfcRoadPart"), 3, "carriageway, shoulder, slope");
        assert_eq!(n("IfcCourse"), 3);
        assert_eq!(n("IfcEarthworksFill"), 1);
        assert_eq!(n("IfcEarthworksCut"), 0, "the road lies above the ground everywhere");
        let ifc = std::fs::read_to_string(dir.join("corridor.ifc")).unwrap();
        assert_eq!(ifc.matches(".TERRAIN.").count(), 1, "only the existing ground; the design surface became the road");
        // The road holds its parts and the feature lines that run through it.
        let road_rel = ifc.lines().find(|l| l.contains("=IFCRELCONTAINEDINSPATIALSTRUCTURE(") && l.contains(&format!(",{});", ifc.lines().find(|l| l.contains("=IFCROAD(")).unwrap().split('=').next().unwrap()))).unwrap();
        assert!(road_rel.matches('#').count() >= 7, "fill and six edge lines in the road: {road_rel}");
        for needle in [".CARRIAGEWAY.", ".SOFTSHOULDER.", ".ROADSIDE.", ".PAVEMENT.", ".EMBANKMENT.", "IFCRELPOSITIONS(", "'OpenInfra_Earthworks'"] {
            assert!(ifc.contains(needle), "missing {needle}");
        }
        // The fill: the cross section's area (trapezoids from the axis out)
        // over 100 m, as a closed body.
        let half = (1.0 + 0.92) / 2.0 * 3.25 + (0.92 + 0.86) / 2.0 * 1.5 + 0.86 / 2.0 * 3.25;
        let volume: f64 = ifc
            .lines()
            .find(|l| l.contains("'Volume'"))
            .and_then(|l| l.split("IFCREAL(").nth(1))
            .and_then(|v| v.split(')').next())
            .and_then(|v| v.parse().ok())
            .unwrap();
        assert!((volume - 2.0 * half * 100.0).abs() < 0.01, "{volume} vs {}", 2.0 * half * 100.0);
        assert!(ifc.contains("IFCREAL(1.)") || ifc.contains("IFCLABEL('true')"), "closed");
    }

    /// Writes target/ifc-test/voorbeeld.ifc for tests/ifc_validate.py.
    #[test]
    fn writes_the_sample_for_the_validator() {
        let doc = sample();
        let site = sample_site();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ifc-test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = Source { doc: &doc, f: 1.0, site: Some(&site), name: "voorbeeld.dwg".into() };
        let r = write(&src, &Options::default(), &dir.join("voorbeeld.ifc")).unwrap();
        assert!(r.instances > 100);
    }
}
