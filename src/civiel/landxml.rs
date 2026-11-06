//! LandXML 1.x in (Road "LandXML Import"): survey points (CgPoints),
//! surfaces (TIN) and alignments (lines, curves, spirals). LandXML writes
//! northing before easting.

use std::f64::consts::PI;
use std::collections::HashMap;

use ocs_plugin_api::host::codec::EntityType;

use super::as_profiel::{Alignment, Seg};
use super::landmeten::SurveyPoint;
use super::terrein::{Tin, P3};
use super::{record, Ctx, Out};
use ocs_plugin_api::host::codec::Handle;
use crate::gis::crs::XY;
use crate::nlcs::Element;

#[derive(Debug, Default)]
pub struct LandXml {
    /// Metres per file unit.
    pub unit: f64,
    pub points: Vec<SurveyPoint>,
    /// Name and triangles (easting, northing, height in metres).
    pub surfaces: Vec<(String, Vec<[P3; 3]>)>,
    pub alignments: Vec<XmlAlignment>,
    /// Objects that could not be read; unrelated objects remain available.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct XmlAlignment {
    pub name: String,
    pub start: f64,
    /// Pieces in project metres (easting, northing).
    pub segs: Vec<Seg>,
    pub spirals: usize,
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && local(c) == name)
}

fn children<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &'static str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> {
    n.children().filter(move |c| c.is_element() && local(c) == name)
}

fn numbers(s: &str, unit: f64) -> Result<Vec<f64>, String> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).map(|t| {
        let value = t.parse::<f64>().map_err(|_| format!("LandXML: ongeldig getal '{t}'"))? * unit;
        if !value.is_finite() { return Err(format!("LandXML: niet-eindig getal '{t}' na eenhedenconversie")); }
        Ok(value)
    }).collect()
}

fn point_values<'a, 'i>(n: roxmltree::Node<'a, 'i>, unit: f64, points: &HashMap<String, roxmltree::Node<'a, 'i>>, depth: usize) -> Result<Vec<f64>, String> {
    // PointType permits text and pntRef together. The text supplies the
    // location; consult the reference only when no coordinate text exists.
    let text = n.text().unwrap_or("");
    if !text.trim().is_empty() { return numbers(text, unit); }
    if depth > 64 { return Err("LandXML: puntverwijzing te diep of cyclisch".into()); }
    if let Some(reference) = n.attribute("pntRef") {
        let reference = reference.trim();
        let target = points.get(reference).ok_or_else(|| format!("LandXML: onbekende puntverwijzing '{reference}'"))?;
        return point_values(*target, unit, points, depth + 1);
    }
    numbers(text, unit)
}

/// "N E" or a point reference → [E, N] in metres.
fn en<'a, 'i>(n: Option<roxmltree::Node<'a, 'i>>, unit: f64, points: &HashMap<String, roxmltree::Node<'a, 'i>>) -> Result<Option<XY>, String> {
    let Some(n) = n else { return Ok(None) };
    let v = point_values(n, unit, points, 0)?;
    if !(2..=3).contains(&v.len()) { return Err(format!("LandXML: {} verwacht twee of drie coördinaten", local(&n))); }
    Ok(Some([v[1], v[0]]))
}

fn number_attr(n: roxmltree::Node, name: &str, unit: f64) -> Result<Option<f64>, String> {
    n.attribute(name).map(|text| {
        let value = text.trim().parse::<f64>().map_err(|_| format!("LandXML: ongeldig {name}: '{text}'"))? * unit;
        if !value.is_finite() { return Err(format!("LandXML: niet-eindig {name}")); }
        Ok(value)
    }).transpose()
}

fn unit_of(name: &str) -> f64 {
    match name.to_ascii_lowercase().as_str() {
        "millimeter" => 0.001,
        "centimeter" => 0.01,
        "decimeter" => 0.1,
        "kilometer" => 1000.0,
        "foot" | "internationalfoot" => 0.3048,
        "ussurveyfoot" => 1200.0 / 3937.0,
        _ => 1.0,
    }
}

pub fn parse(text: &str) -> Result<LandXml, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("geen geldige XML: {e}"))?;
    let root = doc.root_element();
    if local(&root) != "LandXML" {
        return Err("geen LandXML-bestand".into());
    }
    let mut out = LandXml { unit: 1.0, ..Default::default() };
    if let Some(units) = child(root, "Units") {
        if let Some(u) = units.children().find(|c| c.is_element()) {
            out.unit = unit_of(u.attribute("linearUnit").unwrap_or("meter"));
        }
    }
    let unit = out.unit;
    // Index all point definitions first, including those after a reference.
    let survey_points: Vec<_> = children(root, "CgPoints").flat_map(|set| set.descendants().filter(|n| n.is_element() && local(n) == "CgPoint")).collect();
    let mut points = HashMap::new();
    for p in &survey_points {
        if let Some(name) = p.attribute("name").or(p.attribute("oID")) {
            points.entry(name.trim().to_string()).or_insert(*p);
        }
    }
    // Points. A group's unnamed reference is membership, not another point.
    for p in survey_points {
        let parsed = (|| -> Result<(), String> {
        let v = point_values(p, unit, &points, 0)?;
        if !(2..=3).contains(&v.len()) {
            return Err("LandXML: CgPoint verwacht twee of drie coördinaten".into());
        }
        let name = p.attribute("name").or(p.attribute("oID")).unwrap_or("").to_string();
        if p.attribute("pntRef").is_some() && name.is_empty() && p.text().is_none_or(|text| text.trim().is_empty()) { return Ok(()); }
        let code = p.attribute("code").or(p.attribute("desc")).unwrap_or("").to_string();
        out.points.push(SurveyPoint {
            name: if name.is_empty() { (out.points.len() + 1).to_string() } else { name },
            e: v[1],
            n: v[0],
            z: v.get(2).copied(),
            code,
        });
        Ok(())
        })();
        if let Err(error) = parsed { out.warnings.push(format!("LandXML: CgPoint {} overgeslagen: {error}", p.attribute("name").or(p.attribute("pntRef")).unwrap_or("zonder naam"))); }
    }
    // Surfaces: points by id, faces by three ids; i="1" faces are invisible.
    for surf in root.descendants().filter(|n| n.is_element() && local(n) == "Surface") {
        let Some(def) = child(surf, "Definition") else { continue };
        let parsed = (|| -> Result<(), String> {
        let mut ids = std::collections::HashMap::new();
        if let Some(pnts) = child(def, "Pnts") {
            for p in children(pnts, "P") {
                let v = point_values(p, unit, &points, 0)?;
                if v.len() != 3 { return Err("LandXML: TIN-punt verwacht drie coördinaten".into()); }
                if let Some(id) = p.attribute("id") {
                    ids.insert(id.to_string(), [v[1], v[0], v[2]]);
                }
            }
        }
        let mut faces = Vec::new();
        if let Some(fs) = child(def, "Faces") {
            for f in children(fs, "F") {
                if f.attribute("i") == Some("1") {
                    continue;
                }
                let t: Vec<&str> = f.text().unwrap_or("").split_whitespace().collect();
                if t.len() >= 3 {
                    if let (Some(a), Some(b), Some(c)) = (ids.get(t[0]), ids.get(t[1]), ids.get(t[2])) {
                        faces.push([*a, *b, *c]);
                    }
                }
            }
        }
        if !faces.is_empty() {
            out.surfaces.push((surf.attribute("name").unwrap_or("Oppervlak").to_string(), faces));
        }
        Ok(())
        })();
        if let Err(error) = parsed { out.warnings.push(format!("LandXML: Surface {} overgeslagen: {error}", surf.attribute("name").unwrap_or("zonder naam"))); }
    }
    // Alignments.
    for al in root.descendants().filter(|n| n.is_element() && local(n) == "Alignment") {
        let Some(geom) = child(al, "CoordGeom") else { continue };
        let parsed = (|| -> Result<(), String> {
        let mut segs = Vec::new();
        let mut spirals = 0;
        for g in geom.children().filter(|c| c.is_element()) {
            if !matches!(local(&g), "Line" | "Curve" | "Spiral") { continue; }
            let start = en(child(g, "Start"), unit, &points)?;
            let end = en(child(g, "End"), unit, &points)?;
            if start.is_none() || end.is_none() { return Err(format!("LandXML: {} zonder Start of End", local(&g))); }
            match (local(&g), start, end) {
                ("Line", Some(a), Some(b)) => segs.push(Seg::Line { a, b }),
                ("Curve", Some(a), Some(b)) => {
                    let c = en(child(g, "Center"), unit, &points)?.ok_or("LandXML: Curve zonder Center")?;
                    let r = (a[0] - c[0]).hypot(a[1] - c[1]);
                    if !r.is_finite() || r <= 0.0 { return Err("LandXML: ongeldige boogstraal".into()); }
                    let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
                    let a1 = (b[1] - c[1]).atan2(b[0] - c[0]);
                    let ccw = g.attribute("rot") != Some("cw");
                    let mut sweep = a1 - a0;
                    if ccw && sweep <= 0.0 {
                        sweep += 2.0 * PI;
                    } else if !ccw && sweep >= 0.0 {
                        sweep -= 2.0 * PI;
                    }
                    segs.push(Seg::Arc { c, r, a0, sweep });
                }
                ("Spiral", Some(a), Some(b)) => {
                    // From the start towards the PI; curvature from radiusStart
                    // to radiusEnd ("INF" and "-INF" are straight), signed by the rotation.
                    let len = number_attr(g, "length", unit)?.unwrap_or_else(|| (b[0] - a[0]).hypot(b[1] - a[1]));
                    if !len.is_finite() || len <= 0.0 { return Err("LandXML: ongeldige spiraallengte".into()); }
                    let left = if g.attribute("rot") == Some("cw") { -1.0 } else { 1.0 };
                    let k = |attr: &str| -> Result<f64, String> {
                        match g.attribute(attr).map(str::trim) {
                            None => Ok(0.0),
                            Some(v) if v.eq_ignore_ascii_case("INF") || v.eq_ignore_ascii_case("-INF") || v.eq_ignore_ascii_case("+INF") => Ok(0.0),
                            Some(_) => Ok(number_attr(g, attr, unit)?.filter(|r| r.abs() > 1e-9).map(|r| left / r).unwrap_or(0.0)),
                        }
                    };
                    // An exact clothoid (Seg::Spiral), heading from the PI.
                    let (k0, k1) = (k("radiusStart")?, k("radiusEnd")?);
                    let dir = match en(child(g, "PI"), unit, &points)? {
                        Some(pi) if (pi[0] - a[0]).hypot(pi[1] - a[1]) > 1e-9 => (pi[1] - a[1]).atan2(pi[0] - a[0]),
                        // Without a PI: the direction that makes the chord fit.
                        _ => {
                            let l = super::as_profiel::clothoid_local(k0, (k1 - k0) / len, len);
                            (b[1] - a[1]).atan2(b[0] - a[0]) - l[1].atan2(l[0])
                        }
                    };
                    if !dir.is_finite() || !k0.is_finite() || !k1.is_finite() { return Err("LandXML: niet-eindige spiraalgegevens".into()); }
                    segs.push(Seg::Spiral { a, dir, len, k0, k1 });
                    spirals += 1;
                }
                _ => {}
            }
        }
        if !segs.is_empty() {
            out.alignments.push(XmlAlignment {
                name: al.attribute("name").unwrap_or("As").to_string(),
                start: number_attr(al, "staStart", unit)?.unwrap_or(0.0),
                segs,
                spirals,
            });
        }
        Ok(())
        })();
        if let Err(error) = parsed { out.warnings.push(format!("LandXML: Alignment {} overgeslagen: {error}", al.attribute("name").unwrap_or("zonder naam"))); }
    }
    if out.points.is_empty() && out.surfaces.is_empty() && out.alignments.is_empty() {
        return Err("geen punten, oppervlakken of assen in dit LandXML-bestand".into());
    }
    Ok(out)
}

/// Project metres → drawing, for a piece of axis (translation and scale only).
fn seg_to_drawing(ctx: &Ctx, s: &Seg) -> Seg {
    match s {
        Seg::Line { a, b } => Seg::Line { a: ctx.to_drawing(*a), b: ctx.to_drawing(*b) },
        Seg::Arc { c, r, a0, sweep } => Seg::Arc { c: ctx.to_drawing(*c), r: r * ctx.f, a0: *a0, sweep: *sweep },
        Seg::Spiral { a, dir, len, k0, k1 } => Seg::Spiral { a: ctx.to_drawing(*a), dir: *dir, len: len * ctx.f, k0: k0 / ctx.f, k1: k1 / ctx.f },
    }
}

/// `GIS_LANDXML <file>`: points, surfaces and alignments in one undo step.
pub fn import(path: &std::path::Path, opt: &super::CivielOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    progress("LandXML lezen…".into());
    let x = parse(&text)?;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Out::default();
    let mut parts = Vec::new();
    let mut n_points = 0;
    if !x.points.is_empty() {
        n_points = super::landmeten::draw(&ctx, &x.points, &file, &mut out)?;
        parts.push(format!("{} punten", x.points.len()));
    }
    let mut surfaces: Vec<(String, usize)> = Vec::new();
    for (name, faces) in &x.surfaces {
        let f = ctx.f;
        let dfaces: Vec<[P3; 3]> = faces.iter().map(|t| t.map(|p| {
            let q = ctx.to_drawing([p[0], p[1]]);
            [q[0], q[1], p[2] * f]
        })).collect();
        let Some(tin) = Tin::from_faces(&dfaces) else { continue };
        let (spec, meshes) = super::terrein::draw(&ctx, &tin, name, &file, 0.0, &mut out)?;
        surfaces.push((spec.name.chars().next().unwrap_or('B').to_string(), meshes));
        parts.push(format!("oppervlak {name} ({} driehoeken)", tin.triangles.len()));
    }
    let axis = ctx.layer("AM", "AS", Element::G)?;
    for a in &x.alignments {
        let al = Alignment { name: a.name.clone(), start: a.start, segs: a.segs.iter().map(|s| seg_to_drawing(&ctx, s)).collect() };
        // The clothoids as arcs, restored exactly from the object data.
        let (poly, spiral_data) = al.to_polyline_data(0.001 * ctx.f);
        let mut pairs = vec![("as", a.name.clone()), ("beginstation", format!("{:.3}", a.start)), ("lengte_m", format!("{:.3}", al.length() / ctx.f)), ("bron", file.clone())];
        pairs.extend(spiral_data.iter().map(|(k, v)| (k.as_str(), v.clone())));
        let d = record(&pairs);
        let mut e = EntityType::LwPolyline(poly);
        let c = e.common_mut();
        c.layer = axis.name.clone();
        c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        c.linetype_scale = ctx.k;
        c.extended_data.add_record(d);
        out.put(&axis, [e]);
        parts.push(format!(
            "as {} ({:.0} m{})",
            a.name,
            al.length() / ctx.f,
            if a.spirals > 0 { format!(", {} overgangsbogen", a.spirals) } else { String::new() }
        ));
    }
    let handles = out.write_handles(&format!("LandXML {file}"))?;
    let n = handles.len();
    super::api::note("punten", x.points.len());
    super::api::note("oppervlakken", x.surfaces.iter().map(|(n, f)| serde_json::json!({"naam": n, "driehoeken": f.len()})).collect::<Vec<_>>());
    // The axes were put last, on their own layer: their handles end the list.
    let axes = &handles[handles.len().saturating_sub(x.alignments.len())..];
    // Into the drawing's object index: points first, then the surfaces'
    // meshes (each surface's outline follows on its own layer), axes last.
    super::landmeten::index_points(&handles, n_points);
    if let Some((status, _)) = surfaces.first() {
        // Point labels vary in number: find the meshes among the new entities.
        let mesh_total: usize = surfaces.iter().map(|s| s.1).sum();
        let found = crate::bridge::entities(&handles).unwrap_or_default();
        let meshes: Vec<Handle> = handles.iter().zip(found).filter(|(_, e)| matches!(e, Some(ocs_plugin_api::host::codec::EntityType::Mesh(_)))).map(|(h, _)| *h).take(mesh_total).collect();
        crate::bridge::index_add("tin", status, &meshes, false);
    }
    for (a, h) in x.alignments.iter().zip(axes) {
        crate::bridge::index_add("as", &a.name, &[*h], true);
    }
    super::api::note("assen", x.alignments.iter().zip(axes).map(|(a, h)| serde_json::json!({"naam": a.name, "handle": super::api::handle_text(*h), "beginstation": a.start})).collect::<Vec<_>>());
    let hint = if x.alignments.is_empty() { "" } else { " Selecteer een as en klik op As voor de stationering." };
    Ok(format!("{file}: {} — {n} elementen.{hint}", parts.join(", ")))
}

// ── Export: LandXML 1.2 ─────────────────────────────────────────────────────

/// Date and time for the file's header.
pub struct Stamp {
    /// "2026-10-29".
    pub date: String,
    /// "14:05:00".
    pub time: String,
}

impl Stamp {
    pub fn now() -> Stamp {
        let t = chrono::Local::now();
        Stamp { date: t.format("%Y-%m-%d").to_string(), time: t.format("%H:%M:%S").to_string() }
    }
}

/// Text for an XML attribute or element.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// A number with at most four decimals, without trailing zeros.
fn n4(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// A number with at most six decimals (alignment geometry: tangency
/// survives the round trip), without trailing zeros.
fn n6(v: f64) -> String {
    let s = format!("{:.6}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// A plan point the LandXML way: northing first.
fn ne(p: XY) -> String {
    format!("{} {}", n6(p[1]), n6(p[0]))
}

/// A point with height: "N E Z".
fn nez(p: P3) -> String {
    format!("{} {} {}", n4(p[1]), n4(p[0]), n4(p[2]))
}

/// One piece of an alignment as a `CoordGeom` child, with the station at
/// its start.
///
/// `Seg::Spiral` (stream as-spoor) becomes
/// `<Spiral length=".." radiusStart=".." radiusEnd=".." rot="cw|ccw"
/// spiType="clothoid" staStart=".."><Start/><PI/><End/></Spiral>`: radii in
/// metres with "INF" for a straight end, `rot` the turning side, `PI` where
/// the tangents at both ends meet. The reader above takes exactly that.
fn coord_geom_segment(out: &mut String, seg: &Seg, station: f64) {
    match seg {
        Seg::Line { a, b } => {
            out.push_str(&format!(
                "        <Line staStart=\"{}\" length=\"{}\"><Start>{}</Start><End>{}</End></Line>\n",
                n4(station),
                n4(seg.length()),
                ne(*a),
                ne(*b)
            ));
        }
        Seg::Spiral { len, k0, k1, .. } => {
            let (start, end) = (seg.at(0.0), seg.at(*len));
            let radius = |k: f64| if k.abs() < 1e-12 { "INF".to_string() } else { n4(1.0 / k.abs()) };
            // The turning side: the curved end decides.
            let k = if k1.abs() >= k0.abs() { *k1 } else { *k0 };
            // Where the tangents at both ends meet.
            let (d0, d1) = ([start.1.cos(), start.1.sin()], [end.1.cos(), end.1.sin()]);
            let det = d0[0] * d1[1] - d0[1] * d1[0];
            let pi = (det.abs() > 1e-9).then(|| {
                let (dx, dy) = (end.0[0] - start.0[0], end.0[1] - start.0[1]);
                let t = (dx * d1[1] - dy * d1[0]) / det;
                [start.0[0] + t * d0[0], start.0[1] + t * d0[1]]
            });
            out.push_str(&format!(
                "        <Spiral length=\"{}\" radiusStart=\"{}\" radiusEnd=\"{}\" rot=\"{}\" spiType=\"clothoid\" staStart=\"{}\"><Start>{}</Start>{}<End>{}</End></Spiral>\n",
                n4(*len),
                radius(*k0),
                radius(*k1),
                if k > 0.0 { "ccw" } else { "cw" },
                n4(station),
                ne(start.0),
                pi.map(|p| format!("<PI>{}</PI>", ne(p))).unwrap_or_default(),
                ne(end.0)
            ));
        }
        Seg::Arc { c, r, sweep, .. } => {
            let start = seg.at(0.0);
            let end = seg.at(seg.length());
            let delta = sweep.abs();
            // Where the tangents meet; none for a half circle or more.
            let pi = (delta < PI - 1e-6).then(|| {
                let t = r * (delta / 2.0).tan();
                [start.0[0] + t * start.1.cos(), start.0[1] + t * start.1.sin()]
            });
            out.push_str(&format!(
                "        <Curve rot=\"{}\" crvType=\"arc\" radius=\"{}\" length=\"{}\" delta=\"{}\" chord=\"{}\" staStart=\"{}\"><Start>{}</Start><Center>{}</Center><End>{}</End>{}</Curve>\n",
                if *sweep > 0.0 { "ccw" } else { "cw" },
                n4(*r),
                n4(seg.length()),
                n4(delta.to_degrees()),
                n4((end.0[0] - start.0[0]).hypot(end.0[1] - start.0[1])),
                n4(station),
                ne(start.0),
                ne(*c),
                ne(end.0),
                pi.map(|p| format!("<PI>{}</PI>", ne(p))).unwrap_or_default()
            ));
        }
    }
}

/// The model as a LandXML 1.2 document (metres, northing before easting).
pub fn write(m: &super::uitwisseling::Model, stamp: &Stamp) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(&format!(
        "<LandXML xmlns=\"http://www.landxml.org/schema/LandXML-1.2\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:schemaLocation=\"http://www.landxml.org/schema/LandXML-1.2 http://www.landxml.org/schema/LandXML-1.2/LandXML-1.2.xsd\" \
         date=\"{}\" time=\"{}\" version=\"1.2\" readOnly=\"false\">\n",
        stamp.date, stamp.time
    ));
    x.push_str("  <Units><Metric areaUnit=\"squareMeter\" linearUnit=\"meter\" volumeUnit=\"cubicMeter\" temperatureUnit=\"celsius\" pressureUnit=\"milliBars\" angularUnit=\"decimal degrees\" directionUnit=\"decimal degrees\"/></Units>\n");
    if let Some(c) = &m.crs {
        let mut a = format!("name=\"{}\"", esc(&c.name));
        if let Some(code) = c.epsg {
            a.push_str(&format!(" epsgCode=\"{code}\""));
        }
        a.push_str(&format!(" horizontalCoordinateSystemName=\"{}\" verticalDatum=\"{}\"", esc(&c.code), esc(&c.vertical)));
        if c.vertical == "NAP" {
            a.push_str(" verticalCoordinateSystemName=\"NAP height (EPSG:5709)\"");
        }
        x.push_str(&format!("  <CoordinateSystem {a}/>\n"));
    }
    if !m.name.is_empty() {
        x.push_str(&format!("  <Project name=\"{}\"/>\n", esc(&m.name)));
    }
    x.push_str(&format!(
        "  <Application name=\"Open Infra\" desc=\"Civiel (Open CAD Studio)\" manufacturer=\"OpenAEC Foundation\" version=\"{}\" manufacturerURL=\"https://github.com/OpenAEC-Foundation/Open-Infra\" timeStamp=\"{}T{}\"/>\n",
        env!("CARGO_PKG_VERSION"),
        stamp.date,
        stamp.time
    ));
    // Points: names must be unique within the collection.
    if !m.points.is_empty() {
        x.push_str("  <CgPoints name=\"Meetpunten\">\n");
        let mut taken = std::collections::HashSet::new();
        for p in &m.points {
            let mut name = p.name.clone();
            let mut i = 2;
            while !taken.insert(name.clone()) {
                name = format!("{}-{i}", p.name);
                i += 1;
            }
            let code = if p.code.is_empty() { String::new() } else { format!(" code=\"{}\"", esc(&p.code)) };
            let at = match p.z {
                Some(z) => nez([p.e, p.n, z]),
                None => ne([p.e, p.n]),
            };
            x.push_str(&format!("    <CgPoint name=\"{}\"{code}>{at}</CgPoint>\n", esc(&name)));
        }
        x.push_str("  </CgPoints>\n");
    }
    // Surfaces: existing ground and designs; a corridor's design surface
    // carries its feature lines as breaklines.
    let surfaces: Vec<&super::uitwisseling::Surface> = m.surfaces.iter().filter(|s| s.points.len() >= 3 && !s.triangles.is_empty()).collect();
    if !surfaces.is_empty() {
        x.push_str("  <Surfaces>\n");
        for s in surfaces {
            let (lo, hi) = s.range();
            let state = if s.status == 'N' { "proposed" } else { "existing" };
            let desc = if s.source.is_empty() { String::new() } else { format!(" desc=\"{}\"", esc(&s.source)) };
            x.push_str(&format!("    <Surface name=\"{}\" state=\"{state}\"{desc}>\n", esc(&s.name)));
            let breaklines: Vec<&super::uitwisseling::FeatureLine> =
                m.corridors.iter().filter(|c| c.surface.is_some_and(|i| std::ptr::eq(&m.surfaces[i], s))).flat_map(|c| c.lines.iter()).filter(|l| l.pts.len() >= 2).collect();
            if !breaklines.is_empty() {
                x.push_str("      <SourceData><Breaklines>\n");
                for l in breaklines {
                    let pts: Vec<String> = l.pts.iter().map(|p| nez(*p)).collect();
                    x.push_str(&format!("        <Breakline name=\"{}\" brkType=\"standard\"><PntList3D>{}</PntList3D></Breakline>\n", esc(&l.name), pts.join(" ")));
                }
                x.push_str("      </Breaklines></SourceData>\n");
            }
            x.push_str(&format!("      <Definition surfType=\"TIN\" area2DSurf=\"{}\" elevMin=\"{}\" elevMax=\"{}\">\n        <Pnts>\n", n4(s.area()), n4(lo), n4(hi)));
            for (i, p) in s.points.iter().enumerate() {
                x.push_str(&format!("          <P id=\"{}\">{}</P>\n", i + 1, nez(*p)));
            }
            x.push_str("        </Pnts>\n        <Faces>\n");
            for t in &s.triangles {
                x.push_str(&format!("          <F>{} {} {}</F>\n", t[0] + 1, t[1] + 1, t[2] + 1));
            }
            x.push_str("        </Faces>\n      </Definition>\n    </Surface>\n");
        }
        x.push_str("  </Surfaces>\n");
    }
    // Alignments with their ground and design profiles.
    if !m.axes.is_empty() {
        x.push_str("  <Alignments name=\"Assen\">\n");
        for a in &m.axes {
            x.push_str(&format!("    <Alignment name=\"{}\" length=\"{}\" staStart=\"{}\">\n      <CoordGeom>\n", esc(&a.name), n4(a.length()), n4(a.start)));
            let mut station = a.start;
            for seg in &a.segs {
                coord_geom_segment(&mut x, seg, station);
                station += seg.length();
            }
            x.push_str("      </CoordGeom>\n");
            if a.ground.is_some() || a.design.as_ref().is_some_and(|p| p.len() >= 2) {
                x.push_str(&format!("      <Profile name=\"{}\" staStart=\"{}\">\n", esc(&a.name), n4(a.start)));
                if let Some((name, pts)) = &a.ground {
                    let list: Vec<String> = pts.iter().map(|(s, z)| format!("{} {}", n4(a.start + s), n4(*z))).collect();
                    x.push_str(&format!("        <ProfSurf name=\"{}\" state=\"existing\"><PntList2D>{}</PntList2D></ProfSurf>\n", esc(name), list.join(" ")));
                }
                if let Some(pvis) = a.design.as_ref().filter(|p| p.len() >= 2) {
                    x.push_str(&format!("        <ProfAlign name=\"{} {}\" state=\"proposed\">\n", crate::lang::tr("Ontwerpprofiel", "Design profile"), esc(&a.name)));
                    for (i, p) in pvis.iter().enumerate() {
                        let at = format!("{} {}", n4(a.start + p.s), n4(p.z));
                        if p.l > 0.0 && i > 0 && i + 1 < pvis.len() {
                            x.push_str(&format!("          <ParaCurve length=\"{}\">{at}</ParaCurve>\n", n4(p.l)));
                        } else {
                            x.push_str(&format!("          <PVI>{at}</PVI>\n"));
                        }
                    }
                    x.push_str("        </ProfAlign>\n");
                }
                x.push_str("      </Profile>\n");
            }
            x.push_str("    </Alignment>\n");
        }
        x.push_str("  </Alignments>\n");
    }
    // Corridor feature lines as plan features: 3D lines vertex to vertex.
    for c in &m.corridors {
        let lines: Vec<&super::uitwisseling::FeatureLine> = c.lines.iter().filter(|l| l.pts.len() >= 2).collect();
        if lines.is_empty() {
            continue;
        }
        x.push_str(&format!(
            "  <PlanFeatures name=\"{}\" desc=\"{}{}\">\n",
            esc(&c.name),
            crate::lang::tr("kantlijnen", "feature lines"),
            if c.template.is_empty() { String::new() } else { format!(", {} {}", crate::lang::tr("sjabloon", "assembly"), esc(&c.template)) }
        ));
        for l in lines {
            x.push_str(&format!("    <PlanFeature name=\"{}\" desc=\"{}\" state=\"proposed\">\n      <CoordGeom>\n", esc(&l.name), l.kind.label()));
            for w in l.pts.windows(2) {
                x.push_str(&format!("        <Line><Start>{}</Start><End>{}</End></Line>\n", nez(w[0]), nez(w[1])));
            }
            x.push_str("      </CoordGeom>\n    </PlanFeature>\n");
        }
        x.push_str("  </PlanFeatures>\n");
    }
    x.push_str("</LandXML>\n");
    x
}

/// `CIV_LANDXML_EXPORT [BESTAND=<pad>] [ALLES]`: points, surfaces (B and N),
/// alignments with their profiles and the corridors' feature lines; the
/// selection, or everything.
pub fn export(opt: &super::CivielOptions, path: Option<std::path::PathBuf>, all: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    use crate::lang::tr;
    let ctx = Ctx::new(opt)?;
    let m = super::uitwisseling::gather(&ctx, all, progress)?;
    if m.is_empty() {
        return Err(tr("niets om te exporteren: geen meetpunten, TIN's, assen of corridors (in de selectie)", "nothing to export: no survey points, TINs, alignments or corridors (in the selection)").into());
    }
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing("landxml.xml"));
    progress(tr("LandXML schrijven…", "Writing LandXML…").into());
    let text = write(&m, &Stamp::now());
    std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    use serde_json::json;
    use super::api::note as api;
    api("bestand", path.to_string_lossy().into_owned());
    api("bytes", text.len());
    api("punten", m.points.len());
    api("oppervlakken", m.surfaces.iter().map(|s| json!({"naam": s.name, "status": s.status.to_string(), "punten": s.points.len(), "driehoeken": s.triangles.len()})).collect::<Vec<_>>());
    api(
        "assen",
        m.axes
            .iter()
            .map(|a| json!({"naam": a.name, "lengte_m": a.length(), "beginstation": a.start, "segmenten": a.segs.len(), "maaiveldprofiel": a.ground.as_ref().map(|g| g.1.len()), "pvi": a.design.as_ref().map(|d| d.len())}))
            .collect::<Vec<_>>(),
    );
    api("corridors", m.corridors.iter().map(|c| json!({"naam": c.name, "kantlijnen": c.lines.len()})).collect::<Vec<_>>());
    api("crs", m.crs.as_ref().map(|c| json!({"code": c.code, "verticaal": c.vertical})).unwrap_or(serde_json::Value::Null));
    let mut parts = Vec::new();
    if !m.points.is_empty() {
        parts.push(format!("{} {}", m.points.len(), tr("punten", "points")));
    }
    if !m.surfaces.is_empty() {
        parts.push(format!("{} {}", m.surfaces.len(), tr("oppervlak(ken)", "surface(s)")));
    }
    if !m.axes.is_empty() {
        let profiles = m.axes.iter().filter(|a| a.ground.is_some() || a.design.is_some()).count();
        parts.push(format!("{} {}, {profiles} {}", m.axes.len(), tr("as(sen)", "alignment(s)"), tr("met profiel", "with a profile")));
    }
    if !m.corridors.is_empty() {
        parts.push(format!("{} corridor(s), {} {}", m.corridors.len(), m.corridors.iter().map(|c| c.lines.len()).sum::<usize>(), tr("kantlijnen", "feature lines")));
    }
    Ok(format!(
        "LandXML 1.2 ({}): {} — {}.",
        m.crs.as_ref().map(|c| c.code.clone()).unwrap_or_else(|| tr("tekeningcoördinaten", "drawing coordinates").into()),
        parts.join(", "),
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = include_str!("../../tests/fixtures/landxml/site.xml");

    #[test]
    fn sol_unreadable_landxml_returns_the_complete_original_error() {
        let text = "<LandXML><CgPoints><CgPoint name=\"bad\">bad</CgPoint></CgPoints></LandXML>";
        assert_eq!(parse(text).unwrap_err(), "geen punten, oppervlakken of assen in dit LandXML-bestand");
    }

    #[test]
    fn sol_start_coordinate_text_overrides_pntref() {
        let text = SITE.replace("<Start>200 100</Start>", "<Start pntRef=\"1\">205 105</Start>");
        assert_ne!(text, SITE);
        let data = parse(&text).unwrap();
        assert_eq!(data.alignments.len(), 1);
        assert_eq!(data.alignments[0].segs[0], Seg::Line { a: [105.0, 205.0], b: [110.0, 200.0] });
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn sol_named_cgpoint_coordinate_text_overrides_pntref() {
        let point = "<CgPoint name=\"2\" pntRef=\"1\">205 105 7</CgPoint>";
        let text = SITE.replace("</CgPoints>", &format!("{point}</CgPoints>"));
        let data = parse(&text).unwrap();
        assert_eq!(data.points.len(), 2);
        let point = &data.points[1];
        assert_eq!(point.name, "2");
        assert_eq!((point.e, point.n, point.z), (105.0, 205.0, Some(7.0)));
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn sol_unnamed_cgpoint_coordinate_text_overrides_pntref() {
        let point = "<CgPoint pntRef=\"1\">205 105 7</CgPoint>";
        let text = SITE.replace("</CgPoints>", &format!("{point}</CgPoints>"));
        let data = parse(&text).unwrap();
        assert_eq!(data.points.len(), 2);
        assert_eq!((data.points[1].e, data.points[1].n, data.points[1].z), (105.0, 205.0, Some(7.0)));
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn sol_negative_infinite_spiral_radius_keeps_zero_start_curvature() {
        let text = SAMPLE.replace("INF", "-INF");
        assert!(text.contains("radiusStart=\"-INF\""));
        let data = parse(&text).unwrap();
        assert_eq!(data.alignments.len(), 1);
        assert_eq!(data.alignments[0].spirals, 1);
        // INF means zero start curvature. The literal -INF substitution
        // must retain exactly the same axis, including all spiral chords.
        assert_eq!(data.alignments[0].segs, parse(SAMPLE).unwrap().alignments[0].segs);
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn one_bad_landxml_object_keeps_unrelated_points_surfaces_and_axes() {
        for (from, to, expected) in [
            ("<CgPoint name=\"1\">200 100 5</CgPoint>", "<CgPoint name=\"1\">200 bad 5</CgPoint>", (0, 1, 1)),
            ("<P id=\"2\">200 110 6</P>", "<P id=\"2\">200 bad 6</P>", (1, 0, 1)),
            ("<Start>200 100</Start>", "<Start>200 bad</Start>", (1, 1, 0)),
        ] {
            let text = SITE.replace(from, to);
            assert_ne!(text, SITE);
            let data = parse(&text).unwrap();
            assert_eq!((data.points.len(), data.surfaces.len(), data.alignments.len()), expected);
            assert_eq!(data.warnings.len(), 1);
            assert!(data.warnings[0].contains("overgeslagen"));
        }
    }

    #[test]
    fn fixture_reads_points_tin_and_axis_in_easting_northing_order() {
        let xml = parse(SITE).unwrap();
        assert_eq!((xml.points.len(), xml.surfaces.len(), xml.alignments.len()), (1, 1, 1));
        assert_eq!((xml.points[0].e, xml.points[0].n, xml.points[0].z), (100.0, 200.0, Some(5.0)));
        assert_eq!(xml.surfaces[0].1[0][2], [110.0, 210.0, 7.0]);
        assert_eq!(xml.alignments[0].start, 10.0);
        let huge = parse(&SITE.replace("200 100 5", "1e100 100 5")).unwrap();
        assert_eq!(huge.points[0].n, 1e100);
    }

    #[test]
    fn rejects_invalid_coordinates_instead_of_shifting_or_skipping_them() {
        for (from, to) in [
            ("200 100 5", "200 broken 100 5"),
            ("200 100 5", "200 NaN 5"),
            ("200 100 5", "200 inf 5"),
            ("200 110 6", "200 broken 110 6"),
            ("200 110 6", "200 110 1e999"),
            ("<Start>200 100</Start>", "<Start>200 broken 100</Start>"),
            ("<Start>200 100</Start>", "<Start>200 NaN</Start>"),
            ("<Start>200 100</Start>", ""),
            ("<CgPoint name=\"1\">200 100 5</CgPoint>", "<CgPoint name=\"1\">200</CgPoint>"),
            ("<P id=\"2\">200 110 6</P>", "<P id=\"2\">200 110</P>"),
        ] {
            let data = parse(&SITE.replace(from, to)).unwrap();
            assert!(!data.warnings.is_empty(), "{from} -> {to}");
        }
    }

    #[test]
    fn rejects_unit_overflow_and_nonfinite_stationing() {
        let overflow = SITE.replace("linearUnit=\"meter\"", "linearUnit=\"kilometer\"").replace("200 100 5", "1e308 100 5");
        assert!(!parse(&overflow).unwrap().warnings.is_empty());
        for value in ["NaN", "inf", "bad", "1e999"] {
            let data = parse(&SITE.replace("staStart=\"10\"", &format!("staStart=\"{value}\""))).unwrap();
            assert!(data.alignments.is_empty() && !data.warnings.is_empty(), "{value}");
        }
    }

    #[test]
    fn rejects_invalid_spiral_numbers_and_nonfinite_arc_radius() {
        for (from, to) in [
            ("length=\"30\"", "length=\"0\""),
            ("length=\"30\"", "length=\"NaN\""),
            ("length=\"30\"", "length=\"bad\""),
            ("radiusEnd=\"200\"", "radiusEnd=\"NaN\""),
            ("radiusEnd=\"200\"", "radiusEnd=\"bad\""),
            ("<Center>50 100</Center>", "<Center>-1.7e308 -1.7e308</Center>"),
        ] {
            assert_ne!(SAMPLE.replace(from, to), SAMPLE, "fixture substitution: {from}");
            let data = parse(&SAMPLE.replace(from, to)).unwrap();
            assert!(data.alignments.is_empty() && !data.warnings.is_empty(), "{from} -> {to}");
        }
    }

    #[test]
    fn rejects_empty_wrong_root_and_truncated_xml() {
        for text in ["", "<Other/>", "<LandXML/>", "<LandXML>", &SITE[..SITE.len() / 2]] {
            assert!(parse(text).is_err());
        }
    }

    #[test]
    fn point_group_reference_preserves_the_existing_point_without_duplicates() {
        for group in [
            "<CgPoints name=\"groep\"><CgPoint pntRef=\"1\"/></CgPoints>",
            "<CgPoints name=\"groep\"><CgPoint pntRef=\" 1 \"/></CgPoints>",
        ] {
            // Put the group first as well: references do not depend on order.
            let text = SITE.replace("<CgPoints>", &format!("{group}<CgPoints>"));
            let model = parse(&text).unwrap();
            assert_eq!((model.points.len(), model.surfaces.len(), model.alignments.len()), (1, 1, 1));
            assert_eq!((model.points[0].e, model.points[0].n, model.points[0].z), (100.0, 200.0, Some(5.0)));
        }
    }

    #[test]
    fn mixed_axis_geometry_resolves_point_references_before_unit_conversion() {
        let definitions = "<CgPoint name=\"axis-start\">0 0</CgPoint><CgPoint name=\"axis-center\">50 100</CgPoint><CgPoint name=\"axis-pi\">60 150</CgPoint><CgPoint name=\"axis-end\">79.9 149.2</CgPoint>";
        for unit in ["meter", "millimeter"] {
            let inline = SAMPLE.replace("linearUnit=\"meter\"", &format!("linearUnit=\"{unit}\""));
            let referenced = inline
                .replace("</CgPoints>", &format!("{definitions}</CgPoints>"))
                .replace("<Start>0 0</Start>", "<Start pntRef=\"axis-start\"/>")
                .replace("<Center>50 100</Center>", "<Center pntRef=\"axis-center\"/>")
                .replace("<PI>60 150</PI>", "<PI pntRef=\"axis-pi\"/>")
                .replace("<End>79.9 149.2</End>", "<End pntRef=\"axis-end\"/>");
            let expected = parse(&inline).unwrap();
            let model = parse(&referenced).unwrap();
            assert_eq!(model.alignments[0].segs, expected.alignments[0].segs, "{unit}");
            assert_eq!(model.alignments[0].spirals, 1);
            assert_eq!(model.alignments[0].start, expected.alignments[0].start);
        }
    }

    #[test]
    fn spiral_radius_allows_surrounding_whitespace() {
        let text = SAMPLE.replace("radiusEnd=\"200\"", "radiusEnd=\" 200 \"");
        assert_eq!(parse(&text).unwrap().alignments[0].segs, parse(SAMPLE).unwrap().alignments[0].segs);
    }

    #[test]
    fn missing_and_cyclic_point_references_return_errors() {
        let missing = SITE.replace("<Start>200 100</Start>", "<Start pntRef=\"missing\"/>");
        let data = parse(&missing).unwrap();
        assert!(data.alignments.is_empty() && !data.warnings.is_empty());
        let cycle = SITE.replace("<CgPoint name=\"1\">200 100 5</CgPoint>", "<CgPoint name=\"1\" pntRef=\"2\"/><CgPoint name=\"2\" pntRef=\"1\"/>");
        let data = parse(&cycle).unwrap();
        assert!(data.points.is_empty() && !data.warnings.is_empty());
    }

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <Units><Metric linearUnit="meter" areaUnit="squareMeter" volumeUnit="cubicMeter"/></Units>
  <CgPoints>
    <CgPoint name="101" code="PUT">425000.0 105000.0 1.25</CgPoint>
    <CgPoint name="102">425010.0 105020.0</CgPoint>
  </CgPoints>
  <Surfaces><Surface name="Maaiveld"><Definition surfType="TIN">
    <Pnts><P id="1">0 0 1</P><P id="2">0 10 2</P><P id="3">10 10 3</P><P id="4">10 0 4</P></Pnts>
    <Faces><F>1 2 3</F><F>1 3 4</F><F i="1">2 3 4</F></Faces>
  </Definition></Surface></Surfaces>
  <Alignments><Alignment name="Hoofdas" staStart="100" length="0">
    <CoordGeom>
      <Line><Start>0 0</Start><End>0 100</End></Line>
      <Curve rot="ccw" radius="50"><Start>0 100</Start><Center>50 100</Center><End>50 150</End></Curve>
      <Spiral length="30" radiusStart="INF" radiusEnd="200" rot="cw"><Start>50 150</Start><PI>60 150</PI><End>79.98313 150.74970</End></Spiral>
    </CoordGeom>
  </Alignment></Alignments>
</LandXML>"#;

    #[test]
    fn reads_points_surfaces_and_alignments() {
        let x = parse(SAMPLE).unwrap();
        assert_eq!(x.points.len(), 2);
        assert_eq!((x.points[0].e, x.points[0].n, x.points[0].z), (105000.0, 425000.0, Some(1.25)));
        assert_eq!(x.points[0].code, "PUT");
        assert_eq!(x.points[1].z, None);
        assert_eq!(x.surfaces[0].1.len(), 2, "the invisible face is left out");
        assert_eq!(x.surfaces[0].1[0][1], [10.0, 0.0, 2.0], "east before north");
        let a = &x.alignments[0];
        assert_eq!((a.name.as_str(), a.start, a.spirals), ("Hoofdas", 100.0, 1));
        // The line runs east (N stays 0).
        assert_eq!(a.segs[0], Seg::Line { a: [0.0, 0.0], b: [100.0, 0.0] });
        // The quarter curve: centre (100, 50), radius 50, counter-clockwise.
        match &a.segs[1] {
            Seg::Arc { c, r, sweep, .. } => {
                assert_eq!(*c, [100.0, 50.0]);
                assert!((r - 50.0).abs() < 1e-9 && (sweep - PI / 2.0).abs() < 1e-9);
            }
            s => panic!("{s:?}"),
        }
        // The spiral is a real clothoid: north from (150, 50), turning right
        // (cw) to R 200 over 30 m, ending on its End to well under a millimetre.
        match a.segs.last().unwrap() {
            g @ Seg::Spiral { len, k0, k1, .. } => {
                assert!((len - 30.0).abs() < 1e-12 && *k0 == 0.0 && (k1 + 1.0 / 200.0).abs() < 1e-12);
                let (end, dir) = g.at(30.0);
                assert!((end[0] - 150.7497).abs() < 1e-4 && (end[1] - 79.98313).abs() < 1e-4, "{end:?}");
                assert!((dir - (PI / 2.0 - 0.075)).abs() < 1e-12);
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn units_and_errors() {
        assert!((unit_of("USSurveyFoot") - 0.3048006).abs() < 1e-6);
        assert!(parse("<Other/>").is_err());
        assert!(parse("<LandXML/>").is_err());
    }

    #[test]
    fn exports_and_reads_back() {
        let m = super::super::uitwisseling::sample_model();
        let text = write(&m, &Stamp { date: "2026-10-29".into(), time: "12:00:00".into() });
        assert!(text.contains("epsgCode=\"28992\"") && text.contains("verticalDatum=\"NAP\""));
        assert!(text.contains("<Project name=\"Dordrecht &amp; co\"/>"));
        // Northing first.
        assert!(text.contains("<CgPoint name=\"101\" code=\"PUT\">425316.5 104886.25 1.2345</CgPoint>"), "{text}");
        assert!(text.contains("<CgPoint name=\"101-2\">425324 104887</CgPoint>"), "duplicate names get a suffix");
        assert!(text.contains("<ParaCurve length=\"30\">160 2.5</ParaCurve>"));
        assert!(text.contains("<ProfSurf name=\"Terrein 1\" state=\"existing\"><PntList2D>100 0.6 "));
        assert!(text.contains("<PlanFeature name=\"Corridor As 1 Kant verharding links\""));
        // Back in with the reader: the same points, faces and axis.
        let x = parse(&text).unwrap();
        assert_eq!(x.points.len(), 2);
        assert_eq!((x.points[0].e, x.points[0].n, x.points[0].z), (m.points[0].e, m.points[0].n, Some(1.2345)));
        assert_eq!(x.points[0].code, "PUT");
        assert_eq!(x.surfaces.len(), 1);
        assert_eq!(x.surfaces[0].0, "Terrein 1");
        let s = &m.surfaces[0];
        for (i, t) in s.triangles.iter().enumerate() {
            assert_eq!(x.surfaces[0].1[i], t.map(|k| s.points[k]));
        }
        assert_eq!(x.alignments.len(), 1);
        let (a, b) = (&x.alignments[0], &m.axes[0]);
        assert_eq!((a.name.as_str(), a.start, a.segs.len()), ("As 1", 100.0, b.segs.len()));
        for (p, q) in a.segs.iter().zip(&b.segs) {
            assert!((p.length() - q.length()).abs() < 1e-3, "{p:?} {q:?}");
            for t in [0.0, 0.5, 1.0] {
                let (u, v) = (p.at(p.length() * t), q.at(q.length() * t));
                assert!((u.0[0] - v.0[0]).abs() < 1e-3 && (u.0[1] - v.0[1]).abs() < 1e-3);
                assert!(((u.1 - v.1 + PI).rem_euclid(2.0 * PI) - PI).abs() < 1e-6, "{u:?} {v:?}");
            }
        }
        assert!(matches!(a.segs[1], Seg::Arc { sweep, .. } if sweep > 0.0), "a left turn stays counter-clockwise");
    }
}
