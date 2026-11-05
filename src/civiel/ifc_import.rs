//! `CIV_IFC_IMPORT <bestand.ifc> [STATUS=B|N]` (issue #66): an IFC model
//! from a partner as a base. Alignments become Open Infra axes (as
//! `CIV_AS` draws them: stationing, tangent points, radii, clothoids) with
//! their design profile as `CIV_ONTWERPPROFIEL` stores it (object data
//! `pvi`, `boog_m`, and the curve length per PVI in `bogen_m`), drawn as the
//! axis in 3D; terrain becomes a TIN on OG-TRIANGULATIE (status B, or N with
//! `STATUS=N`); every other element with surfaces becomes meshes on its NLCS
//! layer with its property sets as object data. IfcMapConversion places
//! everything in the project's coordinate system. The reading is
//! `ifc::import`; this module places and draws.

use std::path::Path;

use ocs_plugin_api::host::codec::entities::Polyline3D;
use ocs_plugin_api::host::codec::{Color, EntityType, Handle};
use rekenkern::mesh3d::Mesh;
use rekenkern::step::StepModel;

use super::as_profiel::{design_z, Alignment, Pvi};
use super::terrein::Tin;
use super::{record, v3, Ctx, Out};
use crate::gis::site::Site;
use crate::ifc::import::{self as ifc, Axis, Content, MapConversion, Product, P3};
use crate::lang::tr;
use crate::nlcs::{self, Element, LayerSpec, Status};

/// A message in the interface language, with positional arguments.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

/// Object data keys that would make an imported mesh pose as an Open Infra
/// object it is not (an axis, a profile, a corridor, a sheet).
const RESERVED: &[&str] = &["as", "ontwerpprofiel", "lengteprofiel", "corridor", "bladen", "profieltype", "label", "vingerafdruk", "terrein", "riool", "rioolprofiel", "verschuiving_m"];

/// Most object data values on one entity.
const MAX_VALUES: usize = 120;

/// Local IFC metres → project metres → drawing.
pub struct Plaatsing {
    pub map: Option<MapConversion>,
    pub site: Option<Site>,
    /// Drawing units per metre.
    pub f: f64,
    /// From the file's CRS to the project's, when they differ.
    pub reproject: Option<crate::gis::crs::Transformer>,
}

impl Plaatsing {
    /// Project coordinates of a local point: through the map conversion, or
    /// (without one) from the project's base point.
    pub fn project(&self, p: P3) -> P3 {
        match &self.map {
            Some(m) => {
                let q = m.apply(p);
                match &self.reproject {
                    Some(t) => t.apply([q[0], q[1]]).map(|r| [r[0], r[1], q[2]]).unwrap_or(q),
                    None => q,
                }
            }
            None => {
                let b = self.site.as_ref().map(|s| [s.origin_e, s.origin_n]).unwrap_or([0.0; 2]);
                [b[0] + p[0], b[1] + p[1], p[2]]
            }
        }
    }

    /// Drawing coordinates of a project point.
    pub fn drawing(&self, p: P3) -> P3 {
        let q = match &self.site {
            Some(s) => s.to_drawing([p[0], p[1]], self.f),
            None => [p[0] * self.f, p[1] * self.f],
        };
        [q[0], q[1], p[2] * self.f]
    }

    fn rotation(&self) -> f64 {
        self.map.as_ref().map(|m| m.rotation).unwrap_or(0.0)
    }

    fn scale(&self) -> f64 {
        self.map.as_ref().map(|m| m.scale).unwrap_or(1.0)
    }

    /// The axis in drawing units.
    pub fn axis(&self, a: &Axis, name: &str) -> Alignment {
        let to_project = |p: [f64; 2]| {
            let q = self.project([p[0], p[1], 0.0]);
            [q[0], q[1]]
        };
        let to_drawing = |p: [f64; 2]| {
            let q = self.drawing([p[0], p[1], 0.0]);
            [q[0], q[1]]
        };
        let segs = ifc::segs(&a.h).iter().map(|s| ifc::map_seg(&ifc::map_seg(s, &to_project, self.rotation(), self.scale()), &to_drawing, 0.0, self.f)).collect();
        Alignment { name: name.to_string(), start: a.start * self.scale(), segs }
    }

    /// The design profile in project metres: distances along the axis as
    /// drawn, heights above the datum.
    pub fn pvis(&self, a: &Axis) -> Vec<Pvi> {
        let (k, dz) = (self.scale(), self.map.as_ref().map(|m| m.height).unwrap_or(0.0));
        ifc::pvis(&a.v).into_iter().map(|p| Pvi { s: p.s * k, z: p.z + dz, l: p.l * k }).collect()
    }

    /// Triangles in drawing coordinates.
    pub fn triangles(&self, tris: &[[P3; 3]]) -> Vec<[P3; 3]> {
        tris.iter().map(|t| t.map(|p| self.drawing(self.project(p)))).collect()
    }
}

/// The object data of a design profile for `pvi`, `boog_m` and `bogen_m`.
pub fn design_data(name: &str, pvis: &[Pvi]) -> Vec<(String, String)> {
    let max = pvis.iter().map(|p| p.l).fold(0.0, f64::max);
    vec![
        ("ontwerpprofiel".into(), format!("Ontwerpprofiel {name}")),
        ("as".into(), name.to_string()),
        ("pvi".into(), pvis.iter().map(|p| format!("{:.3}:{:.3}", p.s, p.z)).collect::<Vec<_>>().join(";")),
        ("boog_m".into(), format!("{max}")),
        ("bogen_m".into(), pvis.iter().map(|p| format!("{:.3}", p.l)).collect::<Vec<_>>().join(";")),
    ]
}

/// The design line in 3D along the axis (drawing units): every metre or so
/// and at the ends of each vertical curve.
pub fn design_line(al: &Alignment, pvis: &[Pvi], f: f64) -> Vec<P3> {
    let len = al.length() / f;
    let (Some(a), Some(b)) = (pvis.first(), pvis.last()) else { return Vec::new() };
    let (s0, s1) = (a.s.clamp(0.0, len), b.s.clamp(0.0, len));
    if s1 <= s0 {
        return Vec::new();
    }
    let step = (len / 2000.0).max(1.0);
    let mut ss: Vec<f64> = Vec::new();
    let mut s = s0;
    while s < s1 {
        ss.push(s);
        s += step;
    }
    ss.push(s1);
    ss.extend(pvis.iter().flat_map(|p| [p.s - p.l / 2.0, p.s + p.l / 2.0]).filter(|s| *s > s0 && *s < s1));
    ss.sort_by(f64::total_cmp);
    ss.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    ss.iter()
        .map(|s| {
            let (p, _) = al.at(s * f);
            [p[0], p[1], design_z(pvis, *s) * f]
        })
        .collect()
}

/// Triangles welded into one mesh (shared corners within 0.1 mm).
pub fn weld(tris: &[[P3; 3]]) -> Mesh {
    let mut m = Mesh::new("ifc", Vec::new(), Vec::new(), [1.0; 4]);
    let mut ids: std::collections::HashMap<(i64, i64, i64), u32> = std::collections::HashMap::new();
    for t in tris {
        let idx = t.map(|p| {
            let key = ((p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64, (p[2] * 1e4).round() as i64);
            *ids.entry(key).or_insert_with(|| {
                m.positions.push(p);
                (m.positions.len() - 1) as u32
            })
        });
        m.indices.push(idx);
    }
    m
}

/// The object data of an element: its IFC class, GlobalId, name and type,
/// our own object data back under its own keys, other property sets as
/// "Set.Property".
pub fn element_data(p: &Product, file: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![("ifc".into(), p.class.clone()), ("bron".into(), file.to_string())];
    if let Some(g) = &p.global_id {
        out.push(("ifc_guid".into(), g.clone()));
    }
    if let Some(n) = &p.name {
        out.push(("naam".into(), n.clone()));
    }
    if let Some(t) = &p.predefined {
        out.push(("ifc_type".into(), t.clone()));
    }
    if let Some(o) = &p.object_type {
        out.push(("ifc_objecttype".into(), o.clone()));
    }
    for (set, name, value) in &p.props {
        let key = match set.as_str() {
            "OpenInfra_ObjectData" if !RESERVED.contains(&name.as_str()) && !name.starts_with("spiraal") => name.clone(),
            "OpenInfra_ObjectData" | "OpenInfra_NLCS" | "OpenInfra_Sewer" => continue,
            _ => format!("{set}.{name}"),
        };
        if key.is_empty() || out.iter().any(|(k, _)| *k == key) {
            continue;
        }
        out.push((key.replace(['=', '\n'], "_"), value.clone()));
        if out.len() >= MAX_VALUES {
            break;
        }
    }
    out
}

/// The status of an Open Infra layer name.
fn layer_status(layer: Option<&str>) -> Option<Status> {
    layer.and_then(|l| nlcs::parse_layer(l).ok()).map(|l| l.status)
}

/// Whether a terrain product becomes a TIN: from our own TIN layer, or
/// without an Open Infra layer at all.
fn is_tin(p: &Product) -> bool {
    p.prop("OpenInfra_NLCS", "Laag").is_none_or(|l| l.contains("-OG-TRIANGULATIE-"))
}

/// The layer of an element: its Open Infra layer, or the NLCS object of its class.
fn element_layer(ctx: &Ctx, p: &Product) -> Result<LayerSpec, String> {
    if let Some(l) = p.prop("OpenInfra_NLCS", "Laag") {
        return super::uitwisseling3d::layer_spec(ctx, Some(l));
    }
    let (g, o) = ifc::nlcs_object(&p.class, p.predefined.as_deref());
    ctx.layer(g, o, Element::G)
}

/// The layers of an axis: its own AM layer (a track keeps AS_SPOOR_…), else AM-AS.
fn axis_layers(ctx: &Ctx, own: Option<&str>) -> Result<(LayerSpec, LayerSpec), String> {
    if let Some(ln) = own.and_then(|l| nlcs::parse_layer(l).ok()).filter(|l| l.group == "AM" && l.object.starts_with("AS")) {
        if let Some(obj) = nlcs::object_by_name("AM", &ln.object) {
            return Ok((nlcs::layer(obj, ln.status, &ln.discipline, Element::G), nlcs::layer(obj, ln.status, &ln.discipline, Element::T)));
        }
    }
    Ok((ctx.layer("AM", "AS", Element::G)?, ctx.layer("AM", "AS", Element::T)?))
}

/// A name that no axis in the drawing has yet.
fn free_name(wanted: &str, taken: &mut Vec<String>) -> String {
    let base = if wanted.trim().is_empty() { "As IFC".to_string() } else { wanted.trim().to_string() };
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t == &name) {
        name = format!("{base} ({n})");
        n += 1;
    }
    taken.push(name.clone());
    name
}

/// `CIV_IFC_IMPORT <bestand.ifc> [STATUS=B|N]`.
pub fn import(opt: &super::CivielOptions, path: &Path, status: Option<Status>, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    progress(tr("IFC lezen…", "Reading the IFC…").into());
    let model = StepModel::parse(&bytes)?;
    let c: Content = ifc::read(&model);
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // A model in another CRS than the project: transformed point by point.
    let reproject = match (c.map.as_ref().and_then(|m| m.crs.clone()), ctx.site.as_ref()) {
        (Some(from), Some(s)) if from.starts_with("EPSG:") && from != s.crs => Some(crate::gis::crs::Transformer::new(&from, &s.crs)?),
        _ => None,
    };
    let pl = Plaatsing { map: c.map.clone(), site: ctx.site.clone(), f: ctx.f, reproject };
    let mut out = Out::default();
    let mut taken: Vec<String> = crate::bridge::index_read().into_iter().filter(|(k, _, _)| k == "as").map(|(_, n, _)| n).collect();

    // Axes first: their polylines lead their layer.
    progress(tr("Assen tekenen…", "Drawing alignments…").into());
    struct AxisOut {
        name: String,
        layer: String,
        nth: usize,
        design: Option<usize>,
        length: f64,
        pvis: usize,
        warnings: Vec<String>,
    }
    let mut axes: Vec<AxisOut> = Vec::new();
    for a in &c.axes {
        let name = free_name(&a.name, &mut taken);
        let al = pl.axis(a, &name);
        let own = ifc::properties(&model, a.id).into_iter().find(|(s, n, _)| s == "OpenInfra_NLCS" && n == "Laag").map(|(_, _, v)| v);
        let (axis, axis_t) = axis_layers(&ctx, own.as_deref())?;
        let mut data: Vec<(String, String)> = vec![("bron".into(), file.clone())];
        if let Some(g) = &a.global_id {
            data.push(("ifc_guid".into(), g.clone()));
        }
        let nth = out.count(&axis.name);
        super::as_profiel::draw_axis(&ctx, &al, &axis, &axis_t, true, &data, &mut out)?;
        let pvis = pl.pvis(a);
        let mut design = None;
        if pvis.len() >= 2 {
            let line = design_line(&al, &pvis, ctx.f);
            if line.len() >= 2 {
                let mut d = design_data(&name, &pvis);
                d.push(("bron".into(), file.clone()));
                let pairs: Vec<(&str, String)> = d.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                let mut e = EntityType::Polyline3D(Polyline3D::from_points(line.iter().map(|p| v3([p[0], p[1]], p[2])).collect()));
                let cm = e.common_mut();
                cm.layer = axis.name.clone();
                cm.color = Color::ByLayer;
                cm.extended_data.add_record(record(&pairs));
                design = Some(out.count(&axis.name));
                out.put(&axis, [e]);
            }
        }
        axes.push(AxisOut { name, layer: axis.name.clone(), nth, design, length: al.length() / ctx.f, pvis: pvis.len(), warnings: a.warnings.clone() });
    }

    // Terrain: TINs.
    progress(tr("Terrein en elementen tekenen…", "Drawing terrain and elements…").into());
    struct TinOut {
        status: Status,
        layer: String,
        nth: usize,
        meshes: usize,
        triangles: usize,
    }
    let mut tins: Vec<TinOut> = Vec::new();
    let mut elements = 0;
    let mut element_tris = 0;
    let mut classes: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (p, terrain) in &c.products {
        let tris: Vec<[P3; 3]> = p.parts.iter().flat_map(|q| pl.triangles(&q.tris)).collect();
        if *terrain && is_tin(p) {
            let Some(tin) = Tin::from_faces(&tris) else { continue };
            let st = status.or(layer_status(p.prop("OpenInfra_NLCS", "Laag"))).unwrap_or(Status::B);
            let mut o = ctx.opt.clone();
            o.nlcs.status = st;
            let tctx = Ctx { f: ctx.f, k: ctx.k, site: ctx.site.clone(), opt: o };
            let layer = tctx.layer("OG", "TRIANGULATIE", Element::G)?.name;
            let nth = out.count(&layer);
            let name = p.name.clone().unwrap_or_else(|| tr("Terrein (IFC)", "Terrain (IFC)").into());
            let (_, meshes) = super::terrein::draw(&tctx, &tin, &name, &file, 0.0, &mut out)?;
            tins.push(TinOut { status: st, layer, nth, meshes, triangles: tin.triangles.len() });
            continue;
        }
        let spec = element_layer(&ctx, p)?;
        let pairs = element_data(p, &file);
        let d = record(&pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect::<Vec<_>>());
        for part in &p.parts {
            let m = weld(&pl.triangles(&part.tris));
            let colour = part.colour.map(|(r, g, b)| Color::Rgb { r, g, b }).unwrap_or(Color::ByLayer);
            let ents: Vec<EntityType> = super::uitwisseling3d::cad_meshes(&m, |q| q)
                .into_iter()
                .map(|mesh| {
                    let mut e = EntityType::Mesh(mesh);
                    let cm = e.common_mut();
                    cm.layer = spec.name.clone();
                    cm.color = colour;
                    cm.extended_data.add_record(d.clone());
                    e
                })
                .collect();
            out.put(&spec, ents);
        }
        elements += 1;
        element_tris += p.triangles();
        *classes.entry(p.class.clone()).or_insert(0) += 1;
    }
    if out.is_empty() {
        return Err(t!(
            "{}: geen assen, terrein of elementen met vlakken gevonden ({} elementen zonder vlakken)",
            "{}: no alignments, terrain or elements with surfaces found ({} elements without surfaces)",
            file,
            c.without_mesh
        ));
    }
    // Where the axes, design lines and TIN meshes land among the new handles.
    let axis_at: Vec<(Option<usize>, Option<usize>)> = axes.iter().map(|a| (out.position(&a.layer, a.nth), a.design.and_then(|n| out.position(&a.layer, n)))).collect();
    let tin_at: Vec<Vec<usize>> = tins.iter().map(|t| (0..t.meshes).filter_map(|k| out.position(&t.layer, t.nth + k)).collect()).collect();
    let handles = out.write_handles(&format!("IFC {file}"))?;
    let handle = |i: Option<usize>| i.and_then(|i| handles.get(i).copied());
    let mut axis_notes = Vec::new();
    for (a, (line, design)) in axes.iter().zip(&axis_at) {
        let h = handle(*line);
        if let Some(h) = h {
            crate::bridge::index_add("as", &a.name, &[h], true);
        }
        if let Some(d) = handle(*design) {
            crate::bridge::index_add("ontwerpprofiel", &a.name, &[d], true);
        }
        axis_notes.push(serde_json::json!({"naam": a.name, "handle": h.map(super::api::handle_text), "lengte_m": a.length, "pvi": a.pvis, "benaderd": a.warnings}));
    }
    for (t, at) in tins.iter().zip(&tin_at) {
        let hs: Vec<Handle> = at.iter().filter_map(|i| handles.get(*i).copied()).collect();
        crate::bridge::index_add("tin", t.status.code(), &hs, false);
    }
    let tin_tris: usize = tins.iter().map(|t| t.triangles).sum();
    super::api::note("bestand", path.to_string_lossy().into_owned());
    super::api::note("schema", c.schema.clone().unwrap_or_default());
    super::api::note("georeferentie", c.map.as_ref().map(|m| serde_json::json!({"crs": m.crs, "eastings": m.eastings, "northings": m.northings, "hoogte": m.height, "rotatie_graden": m.rotation.to_degrees(), "schaal": m.scale})).unwrap_or(serde_json::Value::Null));
    super::api::note("assen", axis_notes);
    super::api::note("terreinen", tins.iter().map(|t| serde_json::json!({"status": t.status.code(), "driehoeken": t.triangles})).collect::<Vec<_>>());
    super::api::note("elementen_ifc", elements);
    super::api::note("driehoeken", element_tris + tin_tris);
    super::api::note("klassen", serde_json::to_value(&classes).unwrap_or_default());
    super::api::note("zonder_vlakken", c.without_mesh);
    super::api::note("niet_gelezen", serde_json::to_value(&c.skipped).unwrap_or_default());
    super::api::note("benaderd", serde_json::to_value(&c.approximated).unwrap_or_default());
    let mut parts = Vec::new();
    if !axes.is_empty() {
        let with_profile = axes.iter().filter(|a| a.pvis >= 2).count();
        parts.push(t!("{} as(sen), {} met ontwerpprofiel", "{} alignment(s), {} with design profile", axes.len(), with_profile));
    }
    for t in &tins {
        parts.push(t!("terrein {} ({} driehoeken)", "terrain {} ({} triangles)", t.status.code(), t.triangles));
    }
    if elements > 0 {
        parts.push(t!("{} elementen ({} driehoeken)", "{} elements ({} triangles)", elements, element_tris));
    }
    let georef = match &c.map {
        Some(m) => t!("geplaatst met IfcMapConversion ({})", "placed by IfcMapConversion ({})", m.crs.clone().unwrap_or_else(|| "?".into())),
        None => tr("zonder georeferentie: op het basispunt van het project", "no georeference: placed on the project base point").to_string(),
    };
    let mut msg = t!("{}: {}; {} — {} tekenelementen.", "{}: {}; {} — {} drawing entities.", file, parts.join(", "), georef, handles.len());
    let skipped: usize = c.skipped.values().sum();
    if skipped > 0 {
        let list: Vec<String> = c.skipped.iter().map(|(k, n)| format!("{k} {n}")).collect();
        msg.push_str(&t!(" Niet gelezen: {}.", " Not read: {}.", list.join(", ")));
    }
    let approx: usize = c.approximated.values().sum::<usize>() + axes.iter().map(|a| a.warnings.len()).sum::<usize>();
    if approx > 0 {
        msg.push_str(&t!(" Benaderd: {} onderdelen (zie de JSON).", " Approximated: {} parts (see the JSON).", approx));
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_sample() -> (StepModel, Content) {
        let doc = crate::ifc::tests::sample();
        let site = crate::ifc::tests::sample_site();
        let src = crate::ifc::Source { doc: &doc, f: 1.0, site: Some(&site), name: "voorbeeld.dwg".into() };
        let mut report = crate::ifc::Report::default();
        let text = String::from_utf8(crate::ifc::export(&src, &crate::ifc::Options::default(), Vec::new(), &mut report).unwrap()).unwrap();
        let m = StepModel::parse_str(&text).unwrap();
        let c = ifc::read(&m);
        (m, c)
    }

    /// Export → import into a drawing in millimetres: the axes come back
    /// where they were drawn, with their design profile; the TIN keeps its
    /// triangles.
    #[test]
    fn our_export_comes_back_in_the_drawing() {
        let (_, c) = read_sample();
        let doc = crate::ifc::tests::sample();
        let site = crate::ifc::tests::sample_site();
        for f in [1.0, 1000.0] {
            let pl = Plaatsing { map: c.map.clone(), site: Some(site.clone()), f, reproject: None };
            for e in doc.model_space_entities().filter(|e| super::super::data_value(e, "spiraal1").is_some()) {
                let al = Alignment::from_entity(e).unwrap();
                let a = c.axes.iter().find(|a| a.name == al.name).unwrap();
                let back = pl.axis(a, &al.name);
                assert!((back.length() / f - al.length()).abs() < 1e-6, "{f} {}", al.name);
                let (p, _) = al.at(al.length() * 0.7);
                let (q, _) = back.at(back.length() * 0.7);
                assert!((p[0] * f - q[0]).hypot(p[1] * f - q[1]) < 1e-5 * f, "{f} {p:?} {q:?}");
                // The drawn polyline keeps the clothoids (object data spiraal…).
                let (_, data) = back.to_polyline_data(0.001 * f);
                assert!(!data.is_empty());
            }
            let a1 = c.axes.iter().find(|a| a.name == "A1").unwrap();
            let pvis = pl.pvis(a1);
            let d = design_data("A1", &pvis);
            let get = |k: &str| d.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap();
            assert_eq!(get("pvi"), "0.000:10.000;100.000:12.000;180.000:11.000");
            let back = super::super::as_profiel::pvis_from_data(&get("pvi"), Some(&get("boog_m")), Some(&get("bogen_m"))).unwrap();
            assert_eq!(back, pvis);
            let al = pl.axis(a1, "A1");
            let line = design_line(&al, &pvis, f);
            assert!(line.len() > 100);
            assert!((line[0][2] - 10.0 * f).abs() < 1e-9 && (line.last().unwrap()[2] - 11.0 * f).abs() < 1e-9);
            // The TIN: two triangles at their drawn corners.
            let (tin, _) = c.products.iter().find(|(p, t)| *t && is_tin(p)).unwrap();
            let tris = pl.triangles(&tin.parts[0].tris);
            let t = Tin::from_faces(&tris).unwrap();
            assert_eq!(t.triangles.len(), 2);
            assert!(t.points.iter().any(|q| (q[0] - 200.0 * f).abs() < 1e-6 && (q[1] - 100.0 * f).abs() < 1e-6 && (q[2] - 2.0 * f).abs() < 1e-6));
        }
    }

    #[test]
    fn element_data_and_layers() {
        let (_, c) = read_sample();
        let (b, _) = c.products.iter().find(|(p, _)| p.class == "IFCBUILDING").unwrap();
        let d = element_data(b, "x.ifc");
        let get = |k: &str| d.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str());
        assert_eq!(get("ifc"), Some("IFCBUILDING"));
        assert_eq!(get("identificatie"), Some("NL.IMBAG.Pand.0505100000012345"));
        assert_eq!(get("bouwjaar"), Some("1470"));
        assert!(get("Laag").is_none() && get("OpenInfra_NLCS.Laag").is_none());
        assert_eq!(b.prop("OpenInfra_NLCS", "Laag").and_then(|l| layer_status(Some(l))), Some(Status::B));
        // Our own TIN layer becomes a TIN; the photo terrain stays meshes.
        let terrains: Vec<bool> = c.products.iter().filter(|(_, t)| *t).map(|(p, _)| is_tin(p)).collect();
        assert_eq!(terrains.iter().filter(|x| **x).count(), 1);
        assert!(terrains.iter().any(|x| !x));
        // Every class without a layer of its own maps to an existing NLCS object.
        for class in ["IFCWALL", "IFCPAVEMENT", "IFCKERB", "IFCEARTHWORKSFILL", "IFCEARTHWORKSCUT", "IFCGEOTECHNICALSTRATUM", "IFCSITE", "IFCPIPESEGMENT", "IFCRAILING", "IFCSIGN", "IFCFURNISHINGELEMENT", "IFCBUILDING", "IFCBRIDGE"] {
            for pre in [None, Some("VEGETATION")] {
                let (g, o) = ifc::nlcs_object(class, pre);
                assert!(nlcs::object_by_name(g, o).is_some(), "{g} {o}");
            }
        }
        let mut taken = vec!["A1".to_string()];
        assert_eq!(free_name("A1", &mut taken), "A1 (2)");
        assert_eq!(free_name("", &mut taken), "As IFC");
        // Welding shares corners.
        let m = weld(&[[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]]);
        assert_eq!((m.positions.len(), m.indices.len()), (4, 2));
    }
}
