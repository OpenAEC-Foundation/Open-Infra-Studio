//! GWSW sewer networks drawn conform NLCS (main group RI): pipes on their
//! system and material layer, manholes as small circles with their name,
//! every object with its GWSW attributes as object data.

use ocs_plugin_api::host::codec::EntityType;
use ocs_plugin_api::host::ExtendedDataRecord;

use crate::basisdata::NlcsOptions;
use crate::bridge;
use crate::emit;
use crate::gis::gwsw;
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature};
use crate::lang::tr;
use crate::nlcs::{self, Element, LayerSpec};

fn spec(name: &str, opt: &NlcsOptions, el: Element) -> Option<LayerSpec> {
    let obj = nlcs::object_by_name("RI", name)?;
    Some(nlcs::layer(obj, opt.status, &opt.discipline, el))
}

fn text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.trim().is_empty() => Some(gwsw::term(Some(v)).trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn record(f: &Feature) -> ExtendedDataRecord {
    bridge::object_data(f.props.iter().filter(|(k, _)| !matches!(k.as_str(), "dataset" | "uri")).filter_map(|(k, v)| text(v).map(|s| (k.as_str(), s))).take(40))
}

/// Fetch the GWSW collections and draw them.
pub fn import(site: &Site, collections: &[&str], opt: &NlcsOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let data = gwsw::fetch(site, collections, progress)?;
    progress(tr("Riolering tekenen…", "Drawing the sewers…").into());
    place(site, &data, opt)
}

pub fn place(site: &Site, data: &[(String, Vec<Feature>)], opt: &NlcsOptions) -> Result<String, String> {
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let f = t.unit_factor;
    let to_site = vector::data_to_site(site, "EPSG:28992", &[])?;
    let k = opt.scale / 1000.0 * f;
    let mut entities: Vec<EntityType> = Vec::new();
    let mut layers: Vec<LayerSpec> = Vec::new();
    let mut counts: Vec<(String, usize)> = Vec::new();
    let ensure = |s: &LayerSpec, layers: &mut Vec<LayerSpec>| {
        if !layers.iter().any(|layer| layer.name == s.name) {
            layers.push(s.clone());
        }
    };
    for (collection, features) in data {
        let mut n = 0;
        for feat in features {
            let Some(name) = gwsw::nlcs_candidates(collection, feat).into_iter().find(|c| nlcs::object_by_name("RI", c).is_some()) else { continue };
            let (lines, points) = vector::feature_to_drawing(site, feat, &*to_site, f, opt.clip);
            if lines.is_empty() && points.is_empty() {
                continue;
            }
            let d = opt.object_data.then(|| record(feat));
            if !lines.is_empty() {
                if let Some(g) = spec(&name, opt, Element::G) {
                    ensure(&g, &mut layers);
                    entities.extend(emit::polylines(&lines, &g.name, k, d.as_ref()));
                }
            }
            if !points.is_empty() {
                if let Some(s) = spec(&name, opt, Element::S).or_else(|| spec(&name, opt, Element::G)) {
                    ensure(&s, &mut layers);
                    entities.extend(emit::markers(&points, &s.name, 0.6 * k, d.as_ref()));
                }
            }
            // Manhole names on the PUTNUMMER object of the system.
            if opt.labels && collection == "beheerput" {
                let sys = gwsw::system(&gwsw::term(feat.props.get("type")), &gwsw::term(feat.props.get("stelseltype")));
                if let (Some(label), Some(at)) = (feat.props.get("naam").and_then(text), points.first()) {
                    if let Some(ts) = spec(&format!("{sys}_PUTNUMMER"), opt, Element::T) {
                        ensure(&ts, &mut layers);
                        entities.push(emit::text([at[0] + 1.2 * k, at[1] + 1.2 * k], &label, 1.8 * k, 0.0, &ts.name, d.as_ref()));
                    }
                }
            }
            n += 1;
        }
        let title = gwsw::COLLECTIONS.iter().find(|(c, _)| c == collection).map(|(_, t)| *t).unwrap_or(collection.as_str());
        counts.push((title.to_string(), n));
    }
    if entities.is_empty() {
        return Err(tr("geen GWSW-riolering in het importgebied (niet elke gemeente publiceert haar stelsel)", "no GWSW sewer data in the import area (not every municipality publishes its network)").into());
    }
    let label = tr("GWSW-riolering", "GWSW sewers");
    let handles = bridge::with_import_undo(label, || {
        for spec in &layers {
            bridge::ensure_nlcs_layer(spec);
        }
        bridge::add_imported_entities(label, entities)
    })?;
    bridge::note_source("GWSW (Stichting RIONED / PDOK, CC0)");
    bridge::zoom_extents();
    let parts: Vec<String> = counts.iter().filter(|(_, n)| *n > 0).map(|(t, n)| format!("{t}: {n}")).collect();
    Ok(format!("GWSW — {} — {} {} {} {}.", parts.join(", "), handles.len(), tr("elementen op", "entities on"), layers.len(), tr("NLCS-lagen", "NLCS layers")))
}
