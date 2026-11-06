//! The dependency graph of the drawing's derived products: which length
//! profiles and cross sections were made from which axis and which ground,
//! and for cross sections also the BGT surfaces, KLIC lines and survey
//! points they cut, with a fingerprint of each source as it was.
//! `CIV_BIJWERKEN` compares the fingerprints with the sources now (for cross
//! sections: what their section lines cut now) and makes again only what a changed
//! source touches, at the same place in the drawing (with the current
//! settings); `CIV_BIJWERKEN CONTROLE` only reports. A product whose axis is
//! gone is taken out of the graph and its drawing removed.
//!
//! Open CAD Studio has no reactors, so this is the "only the affected part"
//! of the roadmap without watching every edit. Labels keep their own sources
//! the same way (`CIV_LABELS_BIJWERKEN`).
//!
//! Kept in the drawing as JSON (XRecord `OPENINFRA/GRAAF`).

use ocs_plugin_api::host::codec::{CadDocument, Handle};
use serde::{Deserialize, Serialize};

use super::as_profiel::Alignment;
use super::doorsnede::{self, Cuttable};
use super::{api, CivielOptions};
use crate::bridge;
use crate::gis::crs::XY;
use crate::lang::tr;

const RECORD: &str = "GRAAF";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Product {
    /// `lengteprofiel` or `dwarsprofielen`.
    pub soort: String,
    /// The axis (hex handle).
    pub as_handle: String,
    /// Where it was drawn (upper-left corner).
    pub plaats: Option<XY>,
    /// (source, fingerprint): `as:<handle>` and `maaiveld`; for cross
    /// sections also `bgt`, `klic` and `meetpunten`.
    pub bronnen: Vec<(String, String)>,
    /// Its elements (hex handles).
    pub gemaakt: Vec<String>,
    /// Cross sections: where they cut in plan, to find what they cut now.
    /// Absent for length profiles and in graphs stored by earlier versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snijlijnen: Option<Snijlijnen>,
}

/// How cross sections were laid out along their axis, as `cross_sections`
/// lays them out: one section every `afstand` m and at every tangent point,
/// from `links` m left of the axis to `rechts` m right of it, in a drawing
/// with `eenheid` drawing units per metre.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snijlijnen {
    pub afstand: f64,
    pub links: f64,
    pub rechts: f64,
    pub eenheid: f64,
}

impl Snijlijnen {
    /// The stations of the sections along the axis (drawing units).
    pub fn stations(&self, al: &Alignment) -> Vec<f64> {
        al.stations(self.afstand * self.eenheid, true)
    }

    /// The section line at station `s` in plan (drawing units), from left
    /// to right looking ahead.
    pub fn line(&self, al: &Alignment, s: f64) -> (XY, XY) {
        let f = self.eenheid;
        let (p, dir) = al.at(s);
        let right_n = [dir.sin(), -dir.cos()];
        let plan_at = |d: f64| [p[0] + right_n[0] * d * f, p[1] + right_n[1] * d * f];
        (plan_at(-self.links), plan_at(self.rechts))
    }

    /// All section lines in plan, as `cross_sections` draws them.
    pub fn lines(&self, al: &Alignment) -> Vec<(XY, XY)> {
        self.stations(al).into_iter().map(|s| self.line(al, s)).collect()
    }

    /// Where what the sections cut lies (drawing units, `[x0, y0, x1, y1]`):
    /// around the axis, as wide as the sections plus 2 m (survey points up
    /// to 1 m beside a section).
    pub fn area(&self, al: &Alignment) -> [f64; 4] {
        let (b, m) = (al.bbox(), (self.links.max(self.rechts) + 2.0) * self.eenheid);
        [b[0] - m, b[1] - m, b[2] + m, b[3] + m]
    }
}

fn hex(h: Handle) -> String {
    format!("{:X}", u64::from(h))
}

fn handle(s: &str) -> Option<Handle> {
    u64::from_str_radix(s, 16).ok().map(Handle::from)
}

/// The fingerprints of a product's sources now: the axis's geometry and
/// data, and the ground (the TIN in the drawing, else AHN or Terrain Tiles).
pub fn fingerprints(doc: &CadDocument, axis: Handle) -> Option<Vec<(String, String)>> {
    let e = doc.get_entity(axis)?;
    let ground = match super::terrein::Tin::from_document(doc) {
        Some(t) => super::labels::tin_fingerprint(&t),
        None => "ahn".into(),
    };
    Some(vec![(format!("as:{}", hex(axis)), super::labels::entity_fingerprint(e)), ("maaiveld".into(), ground)])
}

/// The record of a product just made from `axis`. Cross sections pass
/// their layout and the objects they were cut from: the `bgt`, `klic` and
/// `meetpunten` sources are what the section lines cut of those.
fn product(doc: &CadDocument, soort: &str, axis: Handle, plaats: Option<XY>, cut: Option<(Snijlijnen, &[Cuttable])>, made: &[Handle]) -> Option<Product> {
    // JSON writes `inf` (OM=inf) as null, which would make the whole graph
    // unreadable: such cross sections stay unrecorded, so always stale.
    let cut = cut.filter(|(s, _)| [s.afstand, s.links, s.rechts, s.eenheid].iter().all(|v| v.is_finite()));
    let mut bronnen = fingerprints(doc, axis)?;
    if let Some((sn, things)) = cut {
        let lines = doc.get_entity(axis).and_then(Alignment::from_entity).map(|al| sn.lines(&al)).unwrap_or_default();
        bronnen.extend(doorsnede::cut_fingerprints(things, &lines, sn.eenheid));
    }
    Some(Product { soort: soort.into(), as_handle: hex(axis), plaats, bronnen, gemaakt: made.iter().map(|h| hex(*h)).collect(), snijlijnen: cut.map(|(s, _)| s) })
}

/// The name of a source in a report.
fn source_name(key: &str) -> String {
    match key {
        "maaiveld" => tr("maaiveld", "ground").into(),
        "bgt" => "BGT".into(),
        "klic" => "KLIC".into(),
        "meetpunten" => tr("meetpunten", "survey points").into(),
        _ => tr("as", "axis").into(),
    }
}

pub fn read() -> Vec<Product> {
    bridge::json_record_read(RECORD).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(all: &[Product]) -> Result<(), String> {
    bridge::json_record_write(RECORD, &serde_json::to_string(all).map_err(|e| e.to_string())?)
}

/// Records a product just made (replacing the one of the same kind and
/// axis). Cross sections pass their layout and the objects they cut from.
pub fn register(doc: &CadDocument, soort: &str, axis: Handle, plaats: XY, cut: Option<(Snijlijnen, &[Cuttable])>, made: &[Handle]) -> Result<(), String> {
    let Some(new) = product(doc, soort, axis, Some(plaats), cut, made) else { return Ok(()) };
    let mut all = read();
    all.retain(|p| !(p.soort == soort && p.as_handle == new.as_handle));
    all.push(new);
    write(&all)
}

/// What changed for each product: `None` up to date, `Some(reason)` stale
/// (an empty reason: the axis is gone). Cross sections recorded without
/// their layout are stale: what they cut was not recorded. Cut objects come
/// from `doc` only; [`stale_reading`] also reads them around the axis.
pub fn stale(doc: &CadDocument, products: &[Product]) -> Vec<Option<String>> {
    stale_reading(doc, products, &|_| Ok(CadDocument::new())).unwrap_or_default()
}

/// [`stale`] for a drawing that may be too large to share: `around` gives
/// the objects in an area that `doc` does not hold (`super::read_area`), as
/// `cross_sections` reads them.
fn stale_reading(doc: &CadDocument, products: &[Product], around: &dyn Fn([f64; 4]) -> Result<CadDocument, String>) -> Result<Vec<Option<String>>, String> {
    let base = if products.iter().any(|p| p.snijlijnen.is_some()) { doorsnede::cuttables(doc) } else { Vec::new() };
    products
        .iter()
        .map(|p| {
            let Some(h) = handle(&p.as_handle) else { return Ok(Some(String::new())) };
            let Some(mut now) = fingerprints(doc, h) else { return Ok(Some(String::new())) };
            if let Some(sn) = &p.snijlijnen {
                let (lines, extra) = match doc.get_entity(h).and_then(Alignment::from_entity) {
                    Some(al) => (sn.lines(&al), doorsnede::cuttables(&around(sn.area(&al))?)),
                    None => (Vec::new(), Vec::new()),
                };
                now.extend(doorsnede::cut_fingerprints(base.iter().chain(&extra), &lines, sn.eenheid));
            }
            Ok(reasons(p, &now))
        })
        .collect()
}

/// Why a product is stale, from its sources now: `None` when it is not.
fn reasons(p: &Product, now: &[(String, String)]) -> Option<String> {
    let changed: Vec<String> = now
        .iter()
        .filter(|(k, v)| p.bronnen.iter().find(|(pk, _)| pk == k).is_none_or(|(_, pv)| pv != v))
        .map(|(k, _)| source_name(k))
        .collect();
    let mut out = Vec::new();
    if !changed.is_empty() {
        out.push(format!("{} {}", changed.join(" + "), tr("gewijzigd", "changed")));
    }
    if p.soort == "dwarsprofielen" && p.snijlijnen.is_none() {
        out.push(tr("gesneden objecten niet vastgelegd", "cut objects not recorded").to_string());
    }
    (!out.is_empty()).then(|| out.join(", "))
}

/// `CIV_BIJWERKEN [CONTROLE]`.
pub fn update(opt: &CivielOptions, only_check: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    let products = read();
    if products.is_empty() {
        return Ok(tr("Geen lengte- of dwarsprofielen in de afhankelijkheidsgraaf van deze tekening.", "No length profiles or cross sections in this drawing's dependency graph.").into());
    }
    let axes: Vec<Handle> = products.iter().filter_map(|p| handle(&p.as_handle)).collect();
    let doc = std::sync::Arc::new(super::document(&["tin"], &axes)?);
    // As `cross_sections` reads them: the BGT, KLIC and survey points
    // around each axis that a drawing too large to share leaves out.
    let state = stale_reading(&doc, &products, &|area| {
        super::read_area(&doc, area).map_err(|e| format!("{} ({e})", tr("gesneden objecten langs de as zijn niet te lezen", "the cut objects along the axis cannot be read")))
    })?;
    let n_stale = state.iter().filter(|s| s.is_some()).count();
    api::note("producten", products.len());
    api::note("verouderd", n_stale);
    let list: Vec<String> = products
        .iter()
        .zip(&state)
        .filter_map(|(p, s)| s.as_ref().map(|r| format!("{} {} ({})", p.soort, p.as_handle, if r.is_empty() { tr("as verwijderd", "axis removed") } else { r.as_str() })))
        .collect();
    if only_check || n_stale == 0 {
        return Ok(if n_stale == 0 {
            format!("{} {}", products.len(), tr("producten, alle actueel.", "products, all up to date."))
        } else {
            format!("{n_stale}/{} {}: {}", products.len(), tr("verouderd", "out of date"), list.join("; "))
        });
    }
    // Length profiles first: the order the products are made in.
    let mut order: Vec<usize> = (0..products.len()).filter(|i| state[*i].is_some()).collect();
    order.sort_by_key(|i| (products[*i].soort != "lengteprofiel") as u8);
    // Calculate every replacement first. A later calculation error must
    // leave even the earlier products and removed-axis products untouched.
    let mut replacements = Vec::new();
    for &i in &order {
        let p = &products[i];
        progress(format!("{} {} {}…", tr("Bijwerken:", "Updating:"), p.soort, p.as_handle));
        if state[i].as_deref() == Some("") {
            continue;
        }
        let axis = handle(&p.as_handle);
        let prepared = match p.soort.as_str() {
            "lengteprofiel" => super::as_profiel::prepare_length_profile(opt, doc.clone(), axis, p.plaats, progress),
            _ => super::as_profiel::prepare_cross_sections(opt, doc.clone(), axis, p.plaats, progress),
        }?;
        replacements.push((i, prepared));
    }

    use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
    bridge::request(PluginRequest::BeginUndoGroup { label: tr("Profielen bijwerken", "Update profiles").into() })?;
    let result = (|| {
        let mut done = Vec::new();
        // Add all replacements before removing any old geometry. Failed
        // host writes are errors too; the old geometry remains available.
        for (i, prepared) in replacements {
            let p = &products[i];
            done.push(format!("{} {}: {}", p.soort, p.as_handle, prepared.write()?));
        }
        let mut gone = 0;
        for i in order {
            let p = &products[i];
            let old: Vec<Handle> = p.gemaakt.iter().filter_map(|h| handle(h)).collect();
            for e in bridge::entities(&old)?.into_iter().flatten() {
                let h = e.common().handle;
                match bridge::request(PluginRequest::RemoveEntity { handle: h })? {
                    PluginResponse::Bool(true) => {}
                    _ => return Err(format!("{} {}", tr("Profielelement niet verwijderd:", "Profile element not removed:"), hex(h))),
                }
            }
            if state[i].as_deref() == Some("") {
                gone += 1;
                let mut all = read();
                all.retain(|q| !(q.soort == p.soort && q.as_handle == p.as_handle));
                write(&all)?;
            }
        }
        Ok(format!(
            "{} {} {}{}. {}",
            done.len(),
            tr("van", "of"),
            products.len(),
            tr(" producten opnieuw gemaakt", " products made again"),
            if gone > 0 { format!("{gone} {}", tr("met een verwijderde as weggehaald.", "with a removed axis taken out.")) } else { String::new() }
        ) + &format!(" {}", done.join(" | ")))
    })();
    // Close the group on both success and error, including all graph and
    // index writes in the same undo step as the geometry.
    let closed = bridge::request(PluginRequest::EndUndoGroup);
    let summary = result?;
    closed?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{EntityType, LwPolyline, Vector2};

    #[test]
    fn changed_axis_is_stale_and_unchanged_is_not() {
        let mut doc = CadDocument::new();
        let mut p = LwPolyline::new();
        p.add_point_with_bulge(Vector2::new(0.0, 0.0), 0.0);
        p.add_point_with_bulge(Vector2::new(100.0, 0.0), 0.0);
        let h = doc.add_entity(EntityType::LwPolyline(p)).unwrap();
        let product = Product { soort: "lengteprofiel".into(), as_handle: hex(h), plaats: None, bronnen: fingerprints(&doc, h).unwrap(), gemaakt: vec![], snijlijnen: None };
        assert_eq!(stale(&doc, std::slice::from_ref(&product)), vec![None]);
        if let Some(EntityType::LwPolyline(p)) = doc.get_entity_mut(h) {
            p.vertices[1].location.x = 120.0;
        }
        let s = stale(&doc, std::slice::from_ref(&product));
        assert!(matches!(s[0].as_deref(), Some("as gewijzigd" | "axis changed")), "{s:?}");
        let gone = Product { as_handle: "FFFFFF".into(), ..product };
        assert_eq!(stale(&doc, &[gone]), vec![Some(String::new())]);
    }

    /// An entity with Open Infra object data.
    fn with_data(doc: &mut CadDocument, mut e: EntityType, layer: &str, pairs: &[(&str, &str)]) -> Handle {
        e.common_mut().layer = layer.into();
        e.common_mut().extended_data.add_record(super::super::record(&pairs.iter().map(|(k, v)| (*k, v.to_string())).collect::<Vec<_>>()));
        doc.add_entity(e).unwrap()
    }

    fn line(a: [f64; 2], b: [f64; 2]) -> EntityType {
        let mut p = LwPolyline::new();
        p.add_point_with_bulge(Vector2::new(a[0], a[1]), 0.0);
        p.add_point_with_bulge(Vector2::new(b[0], b[1]), 0.0);
        EntityType::LwPolyline(p)
    }

    fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> EntityType {
        let mut p = LwPolyline::new();
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            p.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        p.is_closed = true;
        EntityType::LwPolyline(p)
    }

    fn move_by(doc: &mut CadDocument, h: Handle, dy: f64) {
        if let Some(EntityType::LwPolyline(p)) = doc.get_entity_mut(h) {
            for v in &mut p.vertices {
                v.location.y += dy;
            }
        }
    }

    /// A 100 m axis along y = 0 with a cable 5 m left of it, a far cable,
    /// a BGT surface and a survey point at the section at 40 m, and one
    /// survey point between two sections.
    struct Site {
        doc: CadDocument,
        axis: Handle,
        cable: Handle,
        far: Handle,
        surface: Handle,
        point: Handle,
        between: Handle,
    }

    fn site() -> Site {
        use ocs_plugin_api::host::codec::Point;
        let mut doc = CadDocument::new();
        let axis = doc.add_entity(line([0.0, 0.0], [100.0, 0.0])).unwrap();
        let klic = [("bron", "KLIC"), ("thema", "laagspanning")];
        let cable = with_data(&mut doc, line([-5.0, 5.0], [105.0, 5.0]), "B-OI-KL-ET_LS-G", &klic);
        let far = with_data(&mut doc, line([-5.0, 50.0], [105.0, 50.0]), "B-OI-KL-ET_LS-G", &klic);
        let surface = with_data(&mut doc, rectangle(35.0, -3.0, 45.0, 3.0), "B-XX-VH-GESLOTENVERHARDING_ASFALT-G", &[("bron", "BGT")]);
        let at = |x: f64, y: f64| EntityType::Point(Point::from_coords(x, y, 0.0));
        let point = with_data(&mut doc, at(40.3, 2.0), "N-XX-MEETPUNT-S", &[("puntnummer", "12"), ("code", "KABEL"), ("z", "1.25")]);
        let between = with_data(&mut doc, at(30.0, 2.0), "N-XX-MEETPUNT-S", &[("puntnummer", "13"), ("z", "1.10")]);
        Site { doc, axis, cable, far, surface, point, between }
    }

    /// The record `register` makes of cross sections every 20 m, 10 m left
    /// and right of the axis.
    fn cross_sections_of(doc: &CadDocument, axis: Handle) -> Product {
        let layout = Snijlijnen { afstand: 20.0, links: 10.0, rechts: 10.0, eenheid: 1.0 };
        product(doc, "dwarsprofielen", axis, None, Some((layout, doorsnede::cuttables(doc).as_slice())), &[]).unwrap()
    }

    /// The record `register` makes of a length profile.
    fn length_profile_of(doc: &CadDocument, axis: Handle) -> Product {
        product(doc, "lengteprofiel", axis, None, None, &[]).unwrap()
    }

    fn set_z(doc: &mut CadDocument, h: Handle, z: &str) {
        let e = doc.get_entity_mut(h).unwrap();
        let mut r = e.common().extended_data.get_record(bridge::XDATA_APP).unwrap().clone();
        for v in &mut r.values {
            if let ocs_plugin_api::host::XDataValue::String(s) = v {
                if s.starts_with("z=") {
                    *s = format!("z={z}");
                }
            }
        }
        e.common_mut().extended_data.add_record(r);
    }

    #[test]
    fn what_the_sections_cut_is_a_source() {
        let s = site();
        let product = cross_sections_of(&s.doc, s.axis);
        assert_eq!(stale(&s.doc, std::slice::from_ref(&product)), vec![None]);
        let check = |doc: &CadDocument, what: &str| {
            let r = stale(doc, std::slice::from_ref(&product));
            assert!(r[0].is_some(), "{what}: still up to date");
            r[0].clone().unwrap()
        };

        // The cable moves 1 m but still crosses every section.
        let mut doc = s.doc.clone();
        move_by(&mut doc, s.cable, 1.0);
        assert!(check(&doc, "moved cable").contains("KLIC"));

        // A new cable across the section at 40 m.
        let mut doc = s.doc.clone();
        with_data(&mut doc, line([38.0, -20.0], [42.0, 20.0]), "B-OI-KL-ET_LS-G", &[("bron", "KLIC"), ("thema", "water")]);
        assert!(check(&doc, "new cable").contains("KLIC"));

        // The BGT surface the section at 40 m runs through.
        let mut doc = s.doc.clone();
        move_by(&mut doc, s.surface, 1.0);
        assert!(check(&doc, "moved surface").contains("BGT"));

        // A survey height corrected.
        let mut doc = s.doc.clone();
        set_z(&mut doc, s.point, "1.40");
        let r = check(&doc, "survey height");
        assert!(r.contains("meetpunten") || r.contains("survey points"), "{r}");

        // What no section cuts stays out of it.
        let mut doc = s.doc.clone();
        move_by(&mut doc, s.far, 1.0);
        set_z(&mut doc, s.between, "1.30");
        assert_eq!(stale(&doc, std::slice::from_ref(&product)), vec![None], "far cable, point between sections");

        // A length profile does not cut anything.
        let length = length_profile_of(&s.doc, s.axis);
        let mut doc = s.doc.clone();
        move_by(&mut doc, s.cable, 1.0);
        assert_eq!(stale(&doc, &[length]), vec![None]);
    }

    #[test]
    fn cross_sections_recorded_without_their_cut_objects_are_stale() {
        // As the graph of version 2026.45.4 stored them: axis and ground only.
        let s = site();
        let axis = super::super::labels::entity_fingerprint(s.doc.get_entity(s.axis).unwrap());
        let json = format!(r#"[{{"soort":"dwarsprofielen","as_handle":"{}","plaats":[0.0,-50.0],"bronnen":[["as:{}","{axis}"],["maaiveld","ahn"]],"gemaakt":["1A"]}},{{"soort":"lengteprofiel","as_handle":"{}","plaats":[0.0,50.0],"bronnen":[["as:{}","{axis}"],["maaiveld","ahn"]],"gemaakt":["2B"]}}]"#, hex(s.axis), hex(s.axis), hex(s.axis), hex(s.axis));
        let old: Vec<Product> = serde_json::from_str(&json).expect("an old graph still reads");
        assert_eq!(old.len(), 2);
        assert_eq!(old[0].gemaakt, vec!["1A".to_string()]);
        let r = stale(&s.doc, &old);
        assert!(r[0].is_some(), "cross sections without their cut objects: {r:?}");
        assert!(matches!(r[0].as_deref(), Some("gesneden objecten niet vastgelegd" | "cut objects not recorded")), "{r:?}");
        assert_eq!(r[1], None, "a length profile needs nothing new");
    }

    #[test]
    fn an_infinite_layout_keeps_the_graph_readable() {
        // OM=inf gives cross sections at the ends only; JSON has no inf.
        let s = site();
        let layout = Snijlijnen { afstand: f64::INFINITY, links: 10.0, rechts: 10.0, eenheid: 1.0 };
        let p = product(&s.doc, "dwarsprofielen", s.axis, None, Some((layout, doorsnede::cuttables(&s.doc).as_slice())), &[]).unwrap();
        let back: Vec<Product> = serde_json::from_str(&serde_json::to_string(&[p]).unwrap()).expect("the graph still reads");
        assert_eq!(back.len(), 1);
        assert!(stale(&s.doc, &back)[0].is_some());
    }
}
