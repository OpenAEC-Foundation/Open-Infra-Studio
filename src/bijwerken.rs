//! Base data brought up to date instead of imported again (issue #11).
//!
//! Every BGT entity carries its source attributes as object data, among them
//! `lokaal_id`. Updating fetches the current BGT for the collections that
//! are in the drawing and compares object by object: a new object is drawn,
//! a changed one (other geometry) is redrawn, an object that left the BGT is
//! moved to a separate layer and never silently deleted. Drawing work without
//! Open Infra object data is not touched.

use std::collections::{BTreeMap, HashMap, HashSet};

use ocs_plugin_api::host::codec::{Color, EntityType, Handle};

use crate::basisdata::{self, NlcsOptions};
use crate::bridge;
use crate::gis::bgt;
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature};
use crate::lang::tr;

/// Layer for objects that are no longer in the BGT.
pub fn vanished_layer() -> &'static str {
    tr("OI-BGT-VERVALLEN", "OI-BGT-REMOVED")
}

/// `key=value` pairs of an entity's Open Infra object data.
fn object_data(e: &EntityType) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(r) = e.common().extended_data.get_record(bridge::XDATA_APP) {
        for v in &r.values {
            if let ocs_plugin_api::host::XDataValue::String(s) = v {
                if let Some((k, val)) = s.split_once('=') {
                    out.insert(k.to_string(), val.to_string());
                }
            }
        }
    }
    out
}

/// A geometry fingerprint in metres, to the millimetre: polyline vertices
/// and point positions, sorted (texts left out: labels follow their object).
fn fingerprint(points: impl IntoIterator<Item = [f64; 2]>, f: f64) -> Vec<(i64, i64)> {
    let mut v: Vec<(i64, i64)> = points.into_iter().map(|p| ((p[0] / f * 1000.0).round() as i64, (p[1] / f * 1000.0).round() as i64)).collect();
    v.sort_unstable();
    v
}

fn entity_points(e: &EntityType) -> Vec<[f64; 2]> {
    match e {
        EntityType::LwPolyline(l) => l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect(),
        EntityType::Circle(c) => vec![[c.center.x, c.center.y]],
        _ => Vec::new(),
    }
}

#[derive(Default)]
struct Drawn {
    handles: Vec<Handle>,
    points: Vec<[f64; 2]>,
    collection: String,
}

/// Bring the BGT in the drawing up to date.
pub fn bgt_update(site: &Site, opt: &NlcsOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let f = t.unit_factor;
    progress(tr("Tekening lezen…", "Reading the drawing…").into());
    let doc = bridge::snapshot()?;
    let mut drawn: BTreeMap<String, Drawn> = BTreeMap::new();
    let mut entities: HashMap<Handle, EntityType> = HashMap::new();
    for e in doc.entities() {
        let d = object_data(e);
        if d.get("bron").map(String::as_str) != Some("BGT") {
            continue;
        }
        let Some(id) = d.get("lokaal_id") else { continue };
        let slot = drawn.entry(id.clone()).or_default();
        slot.handles.push(e.common().handle);
        slot.points.extend(entity_points(e));
        slot.collection = d.get("objecttype").cloned().unwrap_or_default();
        entities.insert(e.common().handle, e.clone());
    }
    if drawn.is_empty() {
        return Err(tr("geen BGT in de tekening om bij te werken: importeer eerst de BGT", "no BGT in the drawing to update: import the BGT first").into());
    }
    let collections: Vec<String> = drawn.values().map(|d| d.collection.clone()).filter(|c| !c.is_empty()).collect::<HashSet<_>>().into_iter().collect();
    let cols: Vec<&str> = collections.iter().map(String::as_str).collect();
    let fresh = bgt::fetch(site, &cols, progress)?;
    progress(tr("Vergelijken…", "Comparing…").into());
    let to_site = vector::data_to_site(site, "EPSG:28992", &[])?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut redraw: Vec<(String, Vec<Feature>)> = Vec::new();
    let mut remove: Vec<Handle> = Vec::new();
    let (mut added, mut changed, mut same) = (0, 0, 0);
    for (collection, features) in &fresh {
        let mut todo = Vec::new();
        for feat in features {
            let Some(id) = feat.props.get("lokaal_id").and_then(|v| v.as_str()) else { continue };
            seen.insert(id.to_string());
            let (lines, points) = vector::feature_to_drawing(site, feat, &*to_site, f, opt.clip);
            if lines.is_empty() && points.is_empty() {
                continue;
            }
            match drawn.get(id) {
                None => {
                    added += 1;
                    todo.push(feat.clone());
                }
                Some(d) => {
                    let new_fp = fingerprint(lines.iter().flat_map(|(l, _)| l.iter().copied()).chain(points.iter().copied()), f);
                    if bgt::is_label(collection) || new_fp == fingerprint(d.points.iter().copied(), f) {
                        same += 1;
                    } else {
                        changed += 1;
                        remove.extend(d.handles.iter().copied());
                        todo.push(feat.clone());
                    }
                }
            }
        }
        if !todo.is_empty() {
            redraw.push((collection.clone(), todo));
        }
    }
    // Objects of the fetched collections that the BGT no longer has.
    let vanished: Vec<Handle> = drawn.iter().filter(|(id, d)| !seen.contains(*id) && collections.contains(&d.collection)).flat_map(|(_, d)| d.handles.iter().copied()).collect();
    let n_vanished = drawn.iter().filter(|(id, d)| !seen.contains(*id) && collections.contains(&d.collection)).count();
    if !remove.is_empty() {
        let label = tr("BGT bijwerken: gewijzigde objecten", "BGT update: changed objects");
        bridge::with_import_layers(label, |layers| {
            layers.unlock(remove.iter().map(|h| entities[h].common().layer.clone()))?;
            let removed = bridge::remove_entities(label, &remove)?;
            if removed != remove.len() {
                return Err(tr("OCS kon de oude BGT-objecten niet allemaal verwijderen", "OCS could not remove all old BGT entities").into());
            }
            Ok(())
        })?;
    }
    let placed = if redraw.is_empty() { String::new() } else { basisdata::bgt_place(site, &redraw, opt)? };
    if !vanished.is_empty() {
        bridge::ensure_layer(vanished_layer(), Color::from_index(1));
        let moved: Vec<EntityType> = vanished
            .iter()
            .filter_map(|h| entities.get(h).cloned())
            .map(|mut e| {
                e.common_mut().layer = vanished_layer().to_string();
                e
            })
            .collect();
        let label = tr("BGT bijwerken: vervallen objecten", "BGT update: removed objects");
        bridge::with_import_layers(label, |layers| {
            layers.unlock(vanished.iter().map(|h| entities[h].common().layer.clone())
                .chain(std::iter::once(vanished_layer().to_owned())))?;
            if bridge::update_entities(label, moved)? > 0 {
                layers.mark_written(vanished_layer());
            }
            Ok(())
        })?;
    }
    let mut msg = format!(
        "{}: {added} {}, {changed} {}, {n_vanished} {} ({}), {same} {}.",
        tr("BGT bijgewerkt", "BGT updated"),
        tr("nieuw", "new"),
        tr("gewijzigd", "changed"),
        tr("vervallen", "removed"),
        vanished_layer(),
        tr("ongewijzigd", "unchanged")
    );
    if !placed.is_empty() {
        msg.push(' ');
        msg.push_str(&placed);
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_ignore_order_and_sub_millimetre_noise() {
        let a = fingerprint([[1.0, 2.0], [3.0, 4.0]], 1.0);
        let b = fingerprint([[3.0001, 4.0], [1.0, 2.0002]], 1.0);
        assert_eq!(a, b);
        assert_ne!(a, fingerprint([[1.0, 2.0], [3.0, 4.01]], 1.0));
        // A drawing in mm: 1000 units per metre.
        assert_eq!(fingerprint([[1000.0, 2000.0]], 1000.0), fingerprint([[1.0, 2.0]], 1.0));
    }
}
