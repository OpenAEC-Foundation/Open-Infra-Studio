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
        let mut plans = crate::gis::planning::Data::from_file_data(data).map_err(|e| data.report(e))?;
        plans.crs = opt.crs.clone();
        return crate::plannen::place_with_text(site, &plans, source, opt.text_m, progress).map_err(|e| data.report(e));
    }
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad", "no drawing active — click a button of the GIS tab in Open CAD Studio").to_string())?;
    let f = t.unit_factor;
    crate::gis::crs::ensure(&opt.crs)?;
    let to_site = vector::data_to_site(site, &opt.crs, &data.features)?;
    let stem = Path::new(source).file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    // No spaces: a layer group cannot name such a layer (#143).
    let base = emit::clean_name(opt.layer.trim());
    // The name a drawing from before #143 has for it: the default layer
    // kept the spaces of the file's name.
    let old_base = if base == emit::layer_name("GIS", stem) { emit::spaced_layer_name("GIS", stem) } else { base.clone() };
    let text_h = opt.text_m * f;
    let mut layers: Vec<String> = Vec::new();
    let mut out: Vec<EntityType> = Vec::new();
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
        let value = opt.split_by.as_ref().and_then(|k| feat.props.get(k)).and_then(emit::attribute_text);
        let layer = value.as_deref().map_or_else(|| base.clone(), |v| emit::layer_name(&base, v));
        if !layers.contains(&layer) {
            // Once per layer: the one from before #143 takes this name.
            bridge::take_over_layer(&value.as_deref().map_or_else(|| old_base.clone(), |v| emit::spaced_layer_name(&old_base, v)), &layer);
            layers.push(layer.clone());
        }
        let record = emit::feature_data(feat);
        out.extend(emit::polylines(&lines, &layer, 1.0, Some(&record)));
        out.extend(emit::markers(&points, &layer, text_h * 0.25, Some(&record)));
        if let Some(text) = opt.label_field.as_ref().and_then(|k| feat.props.get(k)).and_then(emit::attribute_text) {
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
        return Err(data.report(if opt.clip {
            tr("geen objecten binnen het importgebied (zet 'bijsnijden' uit of controleer het coördinatenstelsel)", "no features inside the import area (turn clipping off or check the coordinate system)").into()
        } else {
            tr("geen tekenbare objecten in het bestand", "no drawable features in the file").into()
        }));
    }
    let label = format!("GIS {source}");
    let handles = bridge::with_import_undo(&label, || {
        for l in &layers {
            bridge::ensure_layer(l, Color::from_index(opt.aci));
        }
        // A file is its own source: its layers are named after it, so is its group.
        bridge::add_imported_entities(&label, &format!("Bestand-{stem}"), out)
    })?;
    bridge::note_source(source);
    bridge::zoom_extents();
    Ok(data.report(format!(
        "{} {n_features} {} ({}, {}) — {} {} {}.",
        data.format,
        tr("objecten", "features"),
        opt.crs,
        source,
        handles.len(),
        tr("elementen op", "entities on"),
        if layers.len() == 1 { layers[0].clone() } else { format!("{} {}", layers.len(), tr("lagen", "layers")) }
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ocs_plugin_api::host::codec::{tables::Layer, CadDocument, Point};

    use super::*;
    use crate::bridge::replacement_test_host::Drawing;

    /// One point at the site, with `props`.
    fn one_feature(props: &[(&str, &str)]) -> FileData {
        let props = props.iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
        let feature = vector::Feature { points: vec![[2600005.0, 1200005.0]], props, ..Default::default() };
        FileData { format: "GeoJSON", crs: Some("EPSG:2056".into()), features: vec![feature], ..Default::default() }
    }

    /// Import `file` with the dialog's options, as `change` sets them.
    fn import(drawing: &Arc<Drawing>, tab_id: u64, file: &str, data: &FileData, change: impl FnOnce(&mut FileOptions)) -> String {
        let site = Site { crs: "EPSG:2056".into(), origin_e: 2600000.0, origin_n: 1200000.0, ..Site::default() };
        let mut opt = FileOptions::for_file(Path::new(file), data, &site);
        change(&mut opt);
        let target = bridge::Target { sender: drawing.clone(), tab_id, unit_factor: 1.0, doc_path: None };
        let mut message = String::new();
        crate::settings::with_test_file(|| {
            message = bridge::with_command_context(target, Some(site.clone()), None, || place(&site, data, &opt, file, &|_| {}).unwrap());
        });
        message
    }

    fn groups(drawing: &Drawing) -> Vec<(String, Vec<String>)> {
        drawing.groups.lock().unwrap().clone()
    }

    fn group(name: &str, layers: &[&str]) -> (String, Vec<String>) {
        (name.into(), layers.iter().map(|l| l.to_string()).collect())
    }

    /// A file named like a fixed source keeps its own layer group, also when
    /// imported again: OCS compares group names without case, and `bgt.csv`
    /// is not BGT (#143).
    #[test]
    fn a_file_named_like_a_source_keeps_its_own_layer_group() {
        let drawing = Arc::new(Drawing::new(CadDocument::new(), false));
        drawing.groups.lock().unwrap().push(group("BGT", &["B-XX-WE-RIJBAAN-G"]));
        for _ in 0..2 {
            import(&drawing, 143_201, "bgt.csv", &one_feature(&[]), |_| {});
            assert_eq!(groups(&drawing), [group("BGT", &["B-XX-WE-RIJBAAN-G"]), group("Bestand-bgt", &["GIS-BGT"])]);
        }
    }

    /// A drawing from before #143 has a layer per value with the spaces of
    /// the file's name: a re-import draws on that layer, renamed, and its
    /// layer group holds it (#143).
    #[test]
    fn a_reimport_per_value_takes_over_the_layer_with_spaces_from_before() {
        let mut doc = CadDocument::new();
        doc.layers.add(Layer::new("GIS-WEGEN WEST-DICHT ASFALT")).unwrap();
        let mut point = Point::default();
        point.common.layer = "GIS-WEGEN WEST-DICHT ASFALT".into();
        let old = doc.add_entity(EntityType::Point(point)).unwrap();
        let drawing = Arc::new(Drawing::new(doc, false));
        import(&drawing, 143_202, "wegen west.gpkg", &one_feature(&[("soort", "dicht asfalt")]), |opt| opt.split_by = Some("soort".into()));
        assert_eq!(groups(&drawing), [group("Bestand-wegen_west", &["GIS-WEGEN_WEST-DICHT_ASFALT"])]);
        let doc = drawing.doc.lock().unwrap();
        assert!(!doc.layers.contains("GIS-WEGEN WEST-DICHT ASFALT"));
        assert_eq!(doc.get_entity(old).unwrap().common().layer, "GIS-WEGEN_WEST-DICHT_ASFALT");
    }

    /// A layer name typed with a space: the import draws on it without the
    /// space, in its layer group, and names that layer (#143).
    #[test]
    fn a_typed_layer_name_with_a_space_is_drawn_without_it() {
        let drawing = Arc::new(Drawing::new(CadDocument::new(), false));
        let message = import(&drawing, 143_203, "wegen.gpkg", &one_feature(&[]), |opt| opt.layer = "Ontwerp wegen".into());
        assert!(message.ends_with("Ontwerp_wegen."), "{message}");
        assert_eq!(groups(&drawing), [group("Bestand-wegen", &["Ontwerp_wegen"])]);
        assert!(!drawing.doc.lock().unwrap().layers.contains("Ontwerp wegen"));
    }
}
