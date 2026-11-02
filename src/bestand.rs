//! Local GIS files into the drawing (GeoJSON, Shapefile, GeoPackage,
//! KML/KMZ, GPX, CSV), with each feature's attributes as object data
//! (XDATA `OPENINFRA`), like the base data imports.

use std::path::Path;

use ocs_plugin_api::host::codec::{Color, EntityType};

use crate::bridge;
use crate::emit;
use crate::gis::files::{self, FileData};
use crate::gis::site::Site;
use crate::gis::vector;
use crate::lang::tr;

#[derive(Clone, Debug)]
pub struct FileOptions {
    /// The data's CRS (`EPSG:xxxx`).
    pub crs: String,
    /// Base layer name; with `split_by`, one layer per value of that attribute.
    pub layer: String,
    pub split_by: Option<String>,
    pub label_field: Option<String>,
    pub aci: i16,
    /// Text height in metres.
    pub text_m: f64,
    /// Clip to the site's import area.
    pub clip: bool,
}

impl FileOptions {
    pub fn for_file(path: &Path, data: &FileData, site: &Site) -> Self {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        FileOptions {
            crs: data.crs.clone().unwrap_or_else(|| site.crs.clone()),
            layer: emit::layer_name("GIS", stem),
            split_by: None,
            label_field: None,
            aci: 3,
            text_m: 1.0,
            clip: false,
        }
    }
}

fn text_of(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) if s.trim().is_empty() => None,
        serde_json::Value::String(s) => Some(s.trim().to_string()),
        other => Some(other.to_string()),
    }
}

/// Read a file and draw it.
pub fn import_path(site: &Site, path: &Path, crs: Option<&str>, progress: &dyn Fn(String)) -> Result<String, String> {
    progress(format!("{} {}…", tr("Lezen", "Reading"), path.display()));
    let data = files::read(path, None)?;
    let mut opt = FileOptions::for_file(path, &data, site);
    if let Some(c) = crs {
        opt.crs = c.to_ascii_uppercase();
    }
    place(site, &data, &opt, &path.display().to_string(), progress)
}

/// Draw file features: polylines and rings, points as small circles, labels,
/// all with their attributes as object data.
pub fn place(site: &Site, data: &FileData, opt: &FileOptions, source: &str, progress: &dyn Fn(String)) -> Result<String, String> {
    if data.format == "IMRO" {
        if opt.clip { return Err("IMRO-vlakken worden met volledige binnenringen ingelezen; zet bijsnijden uit.".into()); }
        let mut plans = crate::gis::planning::Data::from_file_data(data)?;
        plans.crs = opt.crs.clone();
        return crate::plannen::place_with_text(site, &plans, source, opt.text_m, progress);
    }
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad", "no drawing active — click a button of the GIS tab in Open CAD Studio").to_string())?;
    let f = t.unit_factor;
    crate::gis::crs::ensure(&opt.crs)?;
    let to_site = vector::data_to_site(site, &opt.crs, &data.features)?;
    let text_h = opt.text_m * f;
    let mut layers: Vec<String> = Vec::new();
    let mut out: Vec<EntityType> = Vec::new();
    let mut first_record = None;
    let mut n_features = 0;
    for (i, feat) in data.features.iter().enumerate() {
        if i % 2000 == 0 {
            progress(format!("{} {}/{}…", tr("Omzetten", "Converting"), i, data.features.len()));
        }
        let (lines, points) = vector::feature_to_drawing(site, feat, &*to_site, f, opt.clip);
        if lines.is_empty() && points.is_empty() {
            continue;
        }
        n_features += 1;
        let layer = match opt.split_by.as_ref().and_then(|k| feat.props.get(k)).and_then(text_of) {
            Some(v) => emit::layer_name(&opt.layer, &v),
            None => opt.layer.clone(),
        };
        if !layers.contains(&layer) {
            layers.push(layer.clone());
        }
        // Object data: up to 40 attributes, `key=value`.
        let record = bridge::object_data(feat.props.iter().filter_map(|(k, v)| text_of(v).map(|s| (k.as_str(), s))).take(40));
        if first_record.is_none() {
            first_record = Some(record.clone());
        }
        out.extend(emit::polylines(&lines, &layer, 1.0, Some(&record)));
        out.extend(emit::markers(&points, &layer, text_h * 0.25, Some(&record)));
        if let Some(text) = opt.label_field.as_ref().and_then(|k| feat.props.get(k)).and_then(text_of) {
            let anchor = points
                .first()
                .copied()
                .or_else(|| lines.iter().find(|(_, closed)| *closed).and_then(|(r, _)| vector::label_point(r)))
                .or_else(|| lines.first().and_then(|(l, _)| emit::label_spot(l).map(|(p, _)| p)));
            if let Some(at) = anchor {
                let rot = if points.is_empty() && lines.iter().all(|(_, c)| !c) { lines.first().and_then(|(l, _)| emit::label_spot(l)).map(|(_, a)| emit::readable(a)).unwrap_or(0.0) } else { 0.0 };
                out.push(emit::text(at, &text, text_h, rot, &layer, Some(&record)));
            }
        }
    }
    if out.is_empty() {
        return Err(if opt.clip {
            tr("geen objecten binnen het importgebied (zet 'bijsnijden' uit of controleer het coördinatenstelsel)", "no features inside the import area (turn clipping off or check the coordinate system)").into()
        } else {
            tr("geen tekenbare objecten in het bestand", "no drawable features in the file").into()
        });
    }
    for l in &layers {
        bridge::ensure_layer(l, Color::from_index(opt.aci));
    }
    let handles = bridge::add_entities_handles(&format!("GIS {}", source), out)?;
    if let (Some(h), Some(r)) = (handles.first(), first_record) {
        bridge::register_object_data(*h, r);
    }
    bridge::note_source(source);
    bridge::zoom_extents();
    Ok(format!(
        "{} {n_features} {} ({}, {}) — {} {} {}.",
        data.format,
        tr("objecten", "features"),
        opt.crs,
        source,
        handles.len(),
        tr("elementen op", "entities on"),
        if layers.len() == 1 { layers[0].clone() } else { format!("{} {}", layers.len(), tr("lagen", "layers")) }
    ))
}
