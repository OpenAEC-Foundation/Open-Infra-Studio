//! The dependency graph of the drawing's derived products: which length
//! profiles and cross sections were made from which axis and which ground,
//! with a fingerprint of each source as it was. `CIV_BIJWERKEN` compares the
//! fingerprints with the sources now and makes again only what a changed
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
    /// (source, fingerprint): `as:<handle>` and `maaiveld`.
    pub bronnen: Vec<(String, String)>,
    /// Its elements (hex handles).
    pub gemaakt: Vec<String>,
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

pub fn read() -> Vec<Product> {
    bridge::json_record_read(RECORD).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(all: &[Product]) -> Result<(), String> {
    bridge::json_record_write(RECORD, &serde_json::to_string(all).map_err(|e| e.to_string())?)
}

/// Records a product just made (replacing the one of the same kind and axis).
pub fn register(doc: &CadDocument, soort: &str, axis: Handle, plaats: XY, made: &[Handle]) -> Result<(), String> {
    let Some(bronnen) = fingerprints(doc, axis) else { return Ok(()) };
    let mut all = read();
    let key = hex(axis);
    all.retain(|p| !(p.soort == soort && p.as_handle == key));
    all.push(Product { soort: soort.into(), as_handle: key, plaats: Some(plaats), bronnen, gemaakt: made.iter().map(|h| hex(*h)).collect() });
    write(&all)
}

/// What changed for each product: `None` up to date, `Some(reason)` stale
/// (an empty reason: the axis is gone).
pub fn stale(doc: &CadDocument, products: &[Product]) -> Vec<Option<String>> {
    products
        .iter()
        .map(|p| {
            let Some(now) = handle(&p.as_handle).and_then(|h| fingerprints(doc, h)) else { return Some(String::new()) };
            let changed: Vec<String> = now
                .iter()
                .filter(|(k, v)| p.bronnen.iter().find(|(pk, _)| pk == k).is_none_or(|(_, pv)| pv != v))
                .map(|(k, _)| if k.starts_with("as:") { tr("as", "axis").to_string() } else { tr("maaiveld", "ground").to_string() })
                .collect();
            (!changed.is_empty()).then(|| changed.join(" + "))
        })
        .collect()
}

/// `CIV_BIJWERKEN [CONTROLE]`.
pub fn update(opt: &CivielOptions, only_check: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    let products = read();
    if products.is_empty() {
        return Ok(tr("Geen lengte- of dwarsprofielen in de afhankelijkheidsgraaf van deze tekening.", "No length profiles or cross sections in this drawing's dependency graph.").into());
    }
    let axes: Vec<Handle> = products.iter().filter_map(|p| handle(&p.as_handle)).collect();
    let doc = std::sync::Arc::new(super::document(&["tin"], &axes)?);
    let state = stale(&doc, &products);
    let n_stale = state.iter().filter(|s| s.is_some()).count();
    api::note("producten", products.len());
    api::note("verouderd", n_stale);
    let list: Vec<String> = products
        .iter()
        .zip(&state)
        .filter_map(|(p, s)| s.as_ref().map(|r| format!("{} {} ({})", p.soort, p.as_handle, if r.is_empty() { tr("as verwijderd", "axis removed").to_string() } else { format!("{} {}", r, tr("gewijzigd", "changed")) })))
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
        let product = Product { soort: "lengteprofiel".into(), as_handle: hex(h), plaats: None, bronnen: fingerprints(&doc, h).unwrap(), gemaakt: vec![] };
        assert_eq!(stale(&doc, std::slice::from_ref(&product)), vec![None]);
        if let Some(EntityType::LwPolyline(p)) = doc.get_entity_mut(h) {
            p.vertices[1].location.x = 120.0;
        }
        let s = stale(&doc, std::slice::from_ref(&product));
        assert!(matches!(s[0].as_deref(), Some("as" | "axis")), "{s:?}");
        let gone = Product { as_handle: "FFFFFF".into(), ..product };
        assert_eq!(stale(&doc, &[gone]), vec![Some(String::new())]);
    }
}
