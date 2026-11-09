//! NLCS++ Netbeheer in the drawing, on top of the host-independent `nlcspp`
//! crate:
//!
//! - import: an NLCS++ XML (V11–V12.1) drawn on its layers, with Z, every
//!   field as object data and `nlcspp_object` for the kind;
//! - export: the drawing as a V12.1 delivery (`Handle` is the CAD handle,
//!   `ID` follows from it) with a project reference over the project area;
//!   objects are found by their object data, else by their layer name;
//!   references (`AssetObjectID`, …) follow their objects to the new IDs;
//! - objects without geometry (relation objects such as an `AbestandBijlage`
//!   without a location) are kept in the drawing's data on import
//!   ([`RELATIONS`]) and delivered again on export;
//! - check: the schema rules per operator plus whether each object lies on
//!   the layer its attributes ask for, as a report.

use std::collections::BTreeMap;

use nlcspp::{check, layer, model, xml, Delivery, Finding, Geom, Object};
use ocs_plugin_api::host::codec::entities::{Point, Polyline3D};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Vector3};

use crate::gis::crs::{Transformer, XY};
use crate::gis::site::Site;
use crate::lang::tr;
use crate::nlcs::LayerSpec;

/// Whether a file is an NLCS++ delivery (its root element), without reading all of it.
pub fn is_delivery(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    String::from_utf8_lossy(&head[..n]).contains("<NLCSnetbeheer")
}

/// A layer spec for an NLCS++ layer: the NLCS colour of its theme when known.
fn spec(name: &str) -> LayerSpec {
    let parts = layer::parse(name).map(|l| l.parts).unwrap_or_default();
    let obj = (parts.len() >= 2).then(|| crate::nlcs::object_by_name("KL", &format!("{}_{}", parts[0], parts[1]))).flatten();
    let (color, rgb) = obj.map(|o| {
        let c = crate::nlcs::layer(o, crate::nlcs::Status::B, "OI", crate::nlcs::Element::G).color;
        (c, crate::nlcs::color_rgb(c))
    }).unwrap_or((7, (255, 255, 255)));
    LayerSpec { name: name.to_string(), color, rgb, true_color: None, lineweight: None, linetype: "Continuous".into(), object_id: 0 }
}

/// Draws a delivery read from `path` in the project area (RD coordinates).
pub fn import(site: &Site, path: &std::path::Path, unit_factor: f64) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let d = xml::read(&text)?;
    let op = d.operator.clone();
    let to_site = crate::gis::vector::data_to_site(site, "EPSG:28992", &[])?;
    let f = unit_factor;
    let to = |p: &[f64; 3]| -> Option<Vector3> {
        let q = to_site([p[0], p[1]])?;
        let d = site.to_drawing(q, f);
        Some(Vector3::new(d[0], d[1], p[2] * f))
    };
    let mut batch = crate::basisdata::Batch::new();
    let (mut drawn, mut no_rule) = (0usize, 0usize);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut kept: Vec<&Object> = Vec::new();
    // References name objects of the same import: two deliveries may use the same IDs.
    let import = import_key();
    for o in &d.objects {
        let Some(g) = &o.geom else {
            kept.push(o);
            *kinds.entry(o.kind.clone()).or_insert(0) += 1;
            continue;
        };
        let name = match layer::for_object(o, op.as_deref()) {
            Some(n) => n,
            None => {
                no_rule += 1;
                format!("{}-OI-KL-{}-{}", layer::status_letter(o.get("Status").unwrap_or("BESTAAND")), o.kind.to_uppercase(), layer::element_of(Some(g)))
            }
        };
        let s = spec(&name);
        let mut pairs: Vec<(&str, String)> = vec![("bron", "NLCS++".into()), ("nlcspp_object", o.kind.clone()), (IMPORT, import.clone())];
        pairs.extend(o.attrs.iter().map(|(k, v)| (k.as_str(), v.clone())));
        let data = crate::bridge::object_data(pairs);
        let mut ents: Vec<EntityType> = Vec::new();
        match g {
            Geom::Point(p) => {
                if let Some(v) = to(p) {
                    ents.push(EntityType::Point(Point::from_coords(v.x, v.y, v.z)));
                    ents.extend(crate::emit::markers(&[[v.x, v.y]], &name, 0.25 * f, Some(&data)));
                }
            }
            Geom::Line(pts) => {
                let v: Vec<Vector3> = pts.iter().filter_map(to).collect();
                if v.len() >= 2 {
                    ents.push(EntityType::Polyline3D(Polyline3D::from_points(v)));
                }
            }
            Geom::Polygon(rings) => {
                for r in rings {
                    let v: Vec<Vector3> = r.iter().filter_map(to).collect();
                    if v.len() >= 3 {
                        let mut pl = Polyline3D::from_points(v);
                        pl.flags.closed = true;
                        ents.push(EntityType::Polyline3D(pl));
                    }
                }
            }
        }
        if ents.is_empty() {
            continue;
        }
        for e in &mut ents {
            let c = e.common_mut();
            c.layer = name.clone();
            if c.extended_data.get_record(crate::bridge::XDATA_APP).is_none() {
                c.extended_data.add_record(data.clone());
            }
        }
        batch.push(&s, ents, Some(&data));
        drawn += 1;
        *kinds.entry(o.kind.clone()).or_insert(0) += 1;
    }
    let (n, layers) = batch.write("NLCS++")?;
    // After the entities, so the record can join their undo step.
    keep_relations(&import, &kept)?;
    drawn += kept.len();
    crate::bridge::note_source(&format!("NLCS++ {}", path.file_name().and_then(|n| n.to_str()).unwrap_or("")));
    let top: Vec<String> = {
        let mut v: Vec<(&String, &usize)> = kinds.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        v.iter().take(5).map(|(k, n)| format!("{n} {k}")).collect()
    };
    let mut msg = format!(
        "NLCS++ {} (V{}{}): {drawn} {} → {n} elementen op {layers} lagen ({}).",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        d.version.as_deref().unwrap_or("?"),
        op.as_deref().map(|o| format!(", {o}")).unwrap_or_default(),
        tr("objecten", "objects"),
        top.join(", ")
    );
    if no_rule > 0 {
        msg.push_str(&format!(" {no_rule} {}", tr("objecten zonder laagregel: op een laag met hun objectnaam.", "objects without a layer rule: on a layer named after them.")));
    }
    Ok(msg)
}

/// The values for the project reference (`AprojectReferentie`).
#[derive(Clone, Debug, Default)]
pub struct Project {
    pub nummer: String,
    pub aannemer: String,
    pub tekeningtype: String,
    pub volgnummer: Option<u32>,
    pub toelichting: String,
}

/// The object data key with the import an object came with.
const IMPORT: &str = "nlcspp_levering";

/// A key unique per import.
fn import_key() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{t:x}-{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// The XRecord (`OPENINFRA/NLCSPP`) with the imported objects that have no
/// geometry: relation objects the schema allows without a location, each
/// with its import. Nothing in the drawing shows them; the export delivers
/// them again.
const RELATIONS: &str = "NLCSPP";

fn relations() -> Result<Vec<(String, Object)>, String> {
    let Some(text) = crate::bridge::json_record_read(RELATIONS) else { return Ok(Vec::new()) };
    let v: Vec<(String, String, Vec<(String, String)>)> = serde_json::from_str(&text).map_err(|e| format!("OPENINFRA/{RELATIONS}: {e}"))?;
    Ok(v.into_iter().map(|(import, kind, attrs)| (import, Object { kind, attrs, ..Default::default() })).collect())
}

/// Adds the objects of one import to [`RELATIONS`].
fn keep_relations(import: &str, new: &[&Object]) -> Result<(), String> {
    if new.is_empty() {
        return Ok(());
    }
    // Imports run on their own threads: one read-and-write at a time.
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let mut all: Vec<(String, String, Vec<(String, String)>)> = relations()?.into_iter().map(|(i, o)| (i, o.kind, o.attrs)).collect();
    all.extend(new.iter().map(|o| (import.to_string(), o.kind.clone(), o.attrs.clone())));
    crate::bridge::json_record_write(RELATIONS, &serde_json::to_string(&all).map_err(|e| e.to_string())?)
}

fn take(o: &mut Object, k: &str) -> Option<String> {
    let i = o.attrs.iter().position(|(a, _)| a == k)?;
    Some(o.attrs.remove(i).1)
}

/// The number of references (IDREF fields) an object has.
fn references(o: &Object) -> usize {
    model::model().object(&o.kind).map_or(0, |d| d.velden.iter().filter(|f| f.kind == "IDREF" && o.get(&f.name).is_some()).count())
}

/// Gives the objects of a delivery their IDs (a drawn object's follows
/// from its CAD handle, a kept relation is numbered) and points their
/// references at them. A kept relation whose references all name objects
/// that are not delivered is left out, until no more drop out. Returns the
/// objects and the findings about references.
fn with_ids(drawn: Vec<Object>, kept: Vec<(String, Object)>) -> (Vec<Object>, Vec<Finding>) {
    let mut objs = Vec::with_capacity(drawn.len() + kept.len());
    let mut source = Vec::with_capacity(drawn.len() + kept.len());
    for mut o in drawn {
        // Drawn before imports kept their import: one shared one.
        let import = take(&mut o, IMPORT).unwrap_or_default();
        source.push(take(&mut o, "ID").map(|id| (import, id)));
        let id = xml::xml_id("H", o.get("Handle").unwrap_or(""));
        o.set("ID", id);
        objs.push(o);
    }
    let first_kept = objs.len();
    for (i, (import, mut o)) in kept.into_iter().enumerate() {
        source.push(take(&mut o, "ID").map(|id| (import, id)));
        // Its handle in the source drawing: nothing in this drawing.
        take(&mut o, "Handle");
        o.set("ID", format!("R{}", i + 1));
        objs.push(o);
    }
    let refs: Vec<usize> = objs.iter().map(references).collect();
    let mut gone = vec![false; objs.len()];
    let (delivered, dangling) = loop {
        let left: Vec<usize> = (0..objs.len()).filter(|&i| !gone[i]).collect();
        let mut trial: Vec<Object> = left.iter().map(|&i| objs[i].clone()).collect();
        let dangling = xml::relink(&mut trial, &left.iter().map(|&i| source[i].clone()).collect::<Vec<_>>());
        let more: Vec<usize> = left.iter().enumerate().filter(|&(j, &i)| i >= first_kept && refs[i] > 0 && dangling.iter().filter(|d| d.object == j).count() == refs[i]).map(|(_, &i)| i).collect();
        if more.is_empty() {
            break (trial, dangling);
        }
        for i in more {
            gone[i] = true;
        }
    };
    let label = |o: &Object| format!("{} {}", o.kind, o.get("Handle").or(o.get("ID")).unwrap_or(""));
    let mut findings = Vec::new();
    for d in &dangling {
        findings.push(Finding { level: nlcspp::Level::Waarschuwing, object: label(&delivered[d.object]), field: d.field.clone(), message: format!("verwijst naar ID {}, dat niet in de tekening staat; de verwijzing wordt niet geleverd", d.id) });
    }
    for (o, _) in objs.iter().zip(&gone).filter(|(_, g)| **g) {
        findings.push(Finding { level: nlcspp::Level::Waarschuwing, object: label(o), field: String::new(), message: "object zonder geometrie uit een import; de objecten waar het naar verwijst worden niet geleverd, dus dit object ook niet".into() });
    }
    (delivered, findings)
}

/// The NLCS++ objects of a drawing (by object data, else by layer name)
/// with their entity layer, in RD. `ID` is an imported object's ID in its
/// source, [`IMPORT`] the import it came with.
pub fn objects(doc: &CadDocument, site: &Site, unit_factor: f64, operator: &str) -> Result<Vec<(Object, String)>, String> {
    let feats = crate::geoexport::features(doc, site, unit_factor, &crate::geoexport::Options { annotation: false, ..Default::default() });
    let rd = (site.crs != "EPSG:28992").then(|| Transformer::new(&site.crs, "EPSG:28992")).transpose()?;
    let to_rd = |p: &[f64; 3]| -> [f64; 3] {
        match &rd {
            Some(t) => t.apply([p[0], p[1]]).map(|q| [q[0], q[1], p[2]]).unwrap_or(*p),
            None => *p,
        }
    };
    let m = model::model();
    let mut out: Vec<(Object, String)> = Vec::new();
    let mut seen_handles = std::collections::HashSet::new();
    for f in feats {
        let lay = f.props.get("laag").cloned().unwrap_or_default();
        if !lay.contains("-OI-KL-") && !f.props.contains_key("nlcspp_object") {
            continue;
        }
        let (kind, from_layer) = match f.props.get("nlcspp_object") {
            Some(k) => (k.clone(), Vec::new()),
            None => match layer::for_layer(&lay, Some(operator)) {
                Some((k, a)) => (k, a),
                None => continue,
            },
        };
        let Some(def) = m.object(&kind) else { continue };
        // The marker circles of an imported symbol carry its data too: one object per handle chain.
        let handle = f.props.get("cad_handle").cloned().unwrap_or_default();
        if f.props.get("cad_type").map(String::as_str) == Some("cirkel") && def.geometrie.as_deref() == Some("point") {
            continue;
        }
        if !seen_handles.insert(handle.clone()) {
            continue;
        }
        let mut o = Object { kind: kind.clone(), srs: Some("EPSG:28992".into()), ..Default::default() };
        o.set("Handle", handle);
        // An imported object keeps all its fields, so the check can warn about
        // the ones V12.1 no longer has (they are not written); otherwise only
        // the model's fields count.
        let imported = f.props.contains_key("nlcspp_object");
        const OURS: [&str; 6] = ["bron", "nlcspp_object", "laag", "cad_type", "cad_handle", "tekst"];
        for (k, v) in &f.props {
            let model_field = def.velden.iter().any(|fd| &fd.name == k);
            let internal = OURS.contains(&k.as_str()) || k.starts_with("nlcs_") || k == "Handle" || k == "blok";
            if !internal && (model_field || imported) {
                o.set(k, v.clone());
            }
        }
        for (k, v) in from_layer {
            if o.get(&k).is_none() {
                o.set(&k, v);
            }
        }
        if o.get("Status").is_none() {
            let st = layer::parse(&lay).map(|l| match l.status.as_str() { "N" => "NIEUW", "R" => "REVISIE", "V" => "VERWIJDERD", _ => "BESTAAND" }).unwrap_or("BESTAAND");
            o.set("Status", st);
        }
        if o.get("Bedrijfstoestand").is_none() {
            o.set("Bedrijfstoestand", layer::parse(&lay).and_then(|l| l.toestand).unwrap_or_else(|| "IN BEDRIJF".into()));
        }
        o.geom = Some(match (&f.geom, def.geometrie.as_deref()) {
            (crate::geoexport::Geom::Point(p), _) => Geom::Point(to_rd(p)),
            (crate::geoexport::Geom::Line(v), Some("point")) => Geom::Point(to_rd(&v[0])),
            (crate::geoexport::Geom::Line(v), _) => Geom::Line(v.iter().map(to_rd).collect()),
            (crate::geoexport::Geom::Polygon(r), Some("curve")) => Geom::Line(r[0].iter().map(to_rd).collect()),
            (crate::geoexport::Geom::Polygon(r), _) => Geom::Polygon(r.iter().map(|ring| ring.iter().map(to_rd).collect()).collect()),
        });
        out.push((o, lay));
    }
    Ok(out)
}

/// The drawing as a V12.1 delivery for `operator`, with all findings
/// (schema rules and layers). Returns (xml, findings, objects).
pub fn delivery(doc: &CadDocument, site: &Site, unit_factor: f64, operator: &str, project: &Project) -> Result<(String, Vec<Finding>, usize), String> {
    let found = objects(doc, site, unit_factor, operator)?;
    let mut d = Delivery { root: "NLCSnetbeheer".into(), version: Some("12.1".into()), operator: Some(operator.to_string()), ..Default::default() };
    let mut p = Object { kind: "AprojectReferentie".into(), ..Default::default() };
    p.set("ID", "PROJECT");
    for (k, v) in [("Projectnummer", project.nummer.as_str()), ("NaamAannemer", project.aannemer.as_str()), ("Tekeningtype", project.tekeningtype.as_str()), ("Toelichting", project.toelichting.as_str())] {
        if !v.trim().is_empty() {
            p.set(k, v.trim());
        }
    }
    p.set("Netbeheerder", model::netbeheerder_value(operator));
    if let Some(n) = project.volgnummer {
        p.set("Volgnummer", n.to_string());
    }
    // The project area, in RD.
    let b = site.bbox();
    let corners = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
    let rd: Vec<[f64; 3]> = corners
        .iter()
        .map(|c| if site.crs == "EPSG:28992" { Ok([c[0], c[1], 0.0]) } else { crate::gis::crs::transform(&site.crs, "EPSG:28992", *c).map(|q: XY| [q[0], q[1], 0.0]) })
        .collect::<Result<_, _>>()?;
    p.geom = Some(Geom::Polygon(vec![rd]));
    d.project = Some(p);
    let mut findings = Vec::new();
    for (o, lay) in &found {
        if let Some(w) = check::check_layer(o, lay, Some(operator)) {
            findings.push(w);
        }
    }
    let (objects, refs) = with_ids(found.into_iter().map(|(o, _)| o).collect(), relations()?);
    let n = objects.len();
    d.objects = objects;
    findings.extend(refs);
    findings.extend(check::check(&d, Some(operator)));
    findings.sort_by(|a, b| a.level.cmp(&b.level).then(a.object.cmp(&b.object)));
    Ok((xml::write(&d, operator, "EPSG:28992"), findings, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_files_are_recognised() {
        let dir = std::env::temp_dir().join(format!("oi-nlcspp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.xml"), dir.join("b.xml"));
        std::fs::write(&a, "<?xml version=\"1.0\"?>\n<NLCSnetbeheerEnexis xmlns=\"x\"/>").unwrap();
        std::fs::write(&b, "<?xml version=\"1.0\"?>\n<GebiedsinformatieLevering/>").unwrap();
        assert!(is_delivery(&a) && !is_delivery(&b));
        let _ = std::fs::remove_dir_all(dir);
    }
}
