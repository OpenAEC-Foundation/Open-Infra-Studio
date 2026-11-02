//! LandXML 1.x in (Road "LandXML Import"): survey points (CgPoints),
//! surfaces (TIN) and alignments (lines, curves, spirals). LandXML writes
//! northing before easting.

use std::f64::consts::PI;

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

/// "N E" in a node's text → [E, N] in metres.
fn en(n: Option<roxmltree::Node>, unit: f64) -> Result<Option<XY>, String> {
    let Some(n) = n else { return Ok(None) };
    let v = numbers(n.text().unwrap_or(""), unit)?;
    if !(2..=3).contains(&v.len()) { return Err(format!("LandXML: {} verwacht twee of drie coördinaten", local(&n))); }
    Ok(Some([v[1], v[0]]))
}

fn number_attr(n: roxmltree::Node, name: &str, unit: f64) -> Result<Option<f64>, String> {
    n.attribute(name).map(|text| {
        let value = text.parse::<f64>().map_err(|_| format!("LandXML: ongeldig {name}: '{text}'"))? * unit;
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

/// Points of a clothoid from `a` heading `dir`, curvature changing linearly
/// from `k0` to `k1` (1/m, left > 0) over `len`: every metre or so.
fn spiral_points(a: XY, dir: f64, len: f64, k0: f64, k1: f64) -> Vec<XY> {
    let n = (len.ceil() as usize).clamp(16, 2000);
    let h = len / n as f64;
    let mut pts = vec![a];
    let (mut p, mut s) = (a, 0.0);
    for _ in 0..n {
        // Midpoint rule on the heading; fine for chords of about a metre.
        let sm = s + h / 2.0;
        let th = dir + k0 * sm + (k1 - k0) * sm * sm / (2.0 * len);
        p = [p[0] + h * th.cos(), p[1] + h * th.sin()];
        s += h;
        pts.push(p);
    }
    pts
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
    // Points.
    for set in children(root, "CgPoints") {
        for p in set.descendants().filter(|n| n.is_element() && local(n) == "CgPoint") {
            let v = numbers(p.text().unwrap_or(""), unit)?;
            if !(2..=3).contains(&v.len()) {
                return Err("LandXML: CgPoint verwacht twee of drie coördinaten".into());
            }
            let name = p.attribute("name").or(p.attribute("oID")).unwrap_or("").to_string();
            let code = p.attribute("code").or(p.attribute("desc")).unwrap_or("").to_string();
            out.points.push(SurveyPoint {
                name: if name.is_empty() { (out.points.len() + 1).to_string() } else { name },
                e: v[1],
                n: v[0],
                z: v.get(2).copied(),
                code,
            });
        }
    }
    // Surfaces: points by id, faces by three ids; i="1" faces are invisible.
    for surf in root.descendants().filter(|n| n.is_element() && local(n) == "Surface") {
        let Some(def) = child(surf, "Definition") else { continue };
        let mut ids = std::collections::HashMap::new();
        if let Some(pnts) = child(def, "Pnts") {
            for p in children(pnts, "P") {
                let v = numbers(p.text().unwrap_or(""), unit)?;
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
    }
    // Alignments.
    for al in root.descendants().filter(|n| n.is_element() && local(n) == "Alignment") {
        let Some(geom) = child(al, "CoordGeom") else { continue };
        let mut segs = Vec::new();
        let mut spirals = 0;
        for g in geom.children().filter(|c| c.is_element()) {
            if !matches!(local(&g), "Line" | "Curve" | "Spiral") { continue; }
            let start = en(child(g, "Start"), unit)?;
            let end = en(child(g, "End"), unit)?;
            if start.is_none() || end.is_none() { return Err(format!("LandXML: {} zonder Start of End", local(&g))); }
            match (local(&g), start, end) {
                ("Line", Some(a), Some(b)) => segs.push(Seg::Line { a, b }),
                ("Curve", Some(a), Some(b)) => {
                    let c = en(child(g, "Center"), unit)?.ok_or("LandXML: Curve zonder Center")?;
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
                    // to radiusEnd ("INF" is straight).
                    let len = number_attr(g, "length", unit)?.unwrap_or_else(|| (b[0] - a[0]).hypot(b[1] - a[1]));
                    if !len.is_finite() || len <= 0.0 { return Err("LandXML: ongeldige spiraallengte".into()); }
                    let pi = en(child(g, "PI"), unit)?.unwrap_or(b);
                    let dir = (pi[1] - a[1]).atan2(pi[0] - a[0]);
                    let left = if g.attribute("rot") == Some("cw") { -1.0 } else { 1.0 };
                    let k = |attr: &str| -> Result<f64, String> {
                        match g.attribute(attr).map(str::trim) {
                            None => Ok(0.0),
                            Some(v) if v.eq_ignore_ascii_case("INF") => Ok(0.0),
                            Some(_) => Ok(number_attr(g, attr, unit)?.filter(|r| r.abs() > 1e-9).map(|r| left / r).unwrap_or(0.0)),
                        }
                    };
                    let pts = spiral_points(a, dir, len, k("radiusStart")?, k("radiusEnd")?);
                    // Close the small integration gap on the given end point.
                    let gap = [b[0] - pts[pts.len() - 1][0], b[1] - pts[pts.len() - 1][1]];
                    let n = pts.len() - 1;
                    let pts: Vec<XY> = pts.iter().enumerate().map(|(i, p)| [p[0] + gap[0] * i as f64 / n as f64, p[1] + gap[1] * i as f64 / n as f64]).collect();
                    if !pts.iter().flatten().all(|v| v.is_finite()) { return Err("LandXML: niet-eindige spiraalcoördinaten".into()); }
                    segs.extend(pts.windows(2).map(|w| Seg::Line { a: w[0], b: w[1] }));
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
        let d = record(&[("as", a.name.clone()), ("beginstation", format!("{:.3}", a.start)), ("lengte_m", format!("{:.3}", al.length() / ctx.f)), ("bron", file.clone())]);
        let mut e = EntityType::LwPolyline(al.to_polyline());
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
            if a.spirals > 0 { format!(", {} overgangsbogen als koorden", a.spirals) } else { String::new() }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = include_str!("../../tests/fixtures/landxml/site.xml");

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
            assert!(parse(&SITE.replace(from, to)).is_err(), "{from} -> {to}");
        }
    }

    #[test]
    fn rejects_unit_overflow_and_nonfinite_stationing() {
        let overflow = SITE.replace("linearUnit=\"meter\"", "linearUnit=\"kilometer\"").replace("200 100 5", "1e308 100 5");
        assert!(parse(&overflow).is_err());
        for value in ["NaN", "inf", "bad", "1e999"] {
            assert!(parse(&SITE.replace("staStart=\"10\"", &format!("staStart=\"{value}\""))).is_err(), "{value}");
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
            assert!(parse(&SAMPLE.replace(from, to)).is_err(), "{from} -> {to}");
        }
    }

    #[test]
    fn rejects_empty_wrong_root_and_truncated_xml() {
        for text in ["", "<Other/>", "<LandXML/>", "<LandXML>", &SITE[..SITE.len() / 2]] {
            assert!(parse(text).is_err());
        }
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
      <Spiral length="30" radiusStart="INF" radiusEnd="200" rot="cw"><Start>50 150</Start><PI>60 150</PI><End>79.9 149.2</End></Spiral>
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
        // The spiral ends exactly on its End.
        match a.segs.last().unwrap() {
            Seg::Line { b, .. } => assert!((b[0] - 149.2).abs() < 1e-9 && (b[1] - 79.9).abs() < 1e-9),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn a_clothoid_bends_gradually() {
        // 100 m from straight to R 100 to the left: the end heading turns
        // L / (2 R) = 0.5 rad.
        let p = spiral_points([0.0, 0.0], 0.0, 100.0, 0.0, 0.01);
        let n = p.len();
        let dir = (p[n - 1][1] - p[n - 2][1]).atan2(p[n - 1][0] - p[n - 2][0]);
        assert!((dir - 0.5).abs() < 0.01, "{dir}");
        // The series with θ = 0.5: x = L(1 − θ²/10 + θ⁴/216), y = L(θ/3 − θ³/42 + θ⁵/1320).
        assert!((p[n - 1][0] - 97.529).abs() < 0.01 && (p[n - 1][1] - 16.371).abs() < 0.01, "{:?}", p[n - 1]);
    }

    #[test]
    fn units_and_errors() {
        assert!((unit_of("USSurveyFoot") - 0.3048006).abs() < 1e-6);
        assert!(parse("<Other/>").is_err());
        assert!(parse("<LandXML/>").is_err());
    }
}
