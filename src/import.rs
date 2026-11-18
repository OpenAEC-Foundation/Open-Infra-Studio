//! Import operations shared by the workbench window and the command line:
//! fetch with the engine, convert with `emit`, write through `bridge`.

use ocs_plugin_api::host::codec::{Color, Handle};

use crate::bridge::{self, Target};
use crate::emit;
use crate::gis::caps::{WfsCaps, WfsType, WmsCaps, WmsLayer};
use crate::gis::raster::{self, Options as RasterOptions};
use crate::gis::servers::{self, MapServer, ServerKind};
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature};
use crate::gis::{caps, osm, wfs};

fn target() -> Result<Target, String> {
    bridge::target().ok_or_else(|| "geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad".into())
}

pub fn wms(site: &Site, server: &MapServer, caps: &WmsCaps, layer: &WmsLayer, opt: &RasterOptions, fade: u8, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = target()?;
    let placed = raster::import_wms(site, caps, layer, opt, &bridge::image_dir(), progress)?;
    let layer_name = emit::layer_name("GIS-WMS", &layer.title);
    let n = place_map(site, &placed, &layer_name, t.unit_factor, fade, &format!("GIS WMS {}", layer.title))?;
    bridge::zoom_extents();
    Ok(format!("{n} kaartdeel/delen van {} op laag {layer_name} ({}).", server.name, server.attribution))
}

pub fn xyz(site: &Site, server: &MapServer, opt: &RasterOptions, fade: u8, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = target()?;
    let placed = raster::import_xyz(site, server, opt, &bridge::image_dir(), progress)?;
    let layer_name = emit::layer_name("GIS", &server.name);
    place_map(site, &placed, &layer_name, t.unit_factor, fade, &format!("GIS {}", server.name))?;
    bridge::zoom_extents();
    Ok(format!("{} op laag {layer_name} ({}).", server.name, server.attribution))
}

pub struct VectorStyle {
    pub aci: i16,
    pub label_field: Option<String>,
    /// Text height in metres.
    pub text_m: f64,
    pub clip: bool,
}

fn place_map(site: &Site, images: &[raster::PlacedImage], layer: &str, unit_factor: f64, fade: u8, label: &str) -> Result<usize, String> {
    bridge::with_import_undo(label, || {
        let handles = write_map(site, images, layer, unit_factor, fade, label)?;
        // DRAWORDER can close the held step, so it must be the last mutation.
        bridge::image_to_back(&handles);
        Ok(handles.len())
    })
}

fn write_map(site: &Site, images: &[raster::PlacedImage], layer: &str, unit_factor: f64, fade: u8, label: &str) -> Result<Vec<Handle>, String> {
    let entities = emit::images(site, images, layer, unit_factor, fade);
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    bridge::ensure_layer(layer, Color::from_index(8));
    bridge::add_entities_handles(label, entities)
}

fn place_features(site: &Site, features: &[Feature], data_crs: &str, layer: &str, label: &str, style: &VectorStyle, with_data: bool) -> Result<usize, String> {
    let t = target()?;
    let to_site = vector::data_to_site(site, data_crs, features)?;
    let text_h = style.text_m * t.unit_factor;
    // Preserve the draw order: polylines, point markers, then labels.
    let mut polylines = Vec::new();
    let mut markers = Vec::new();
    let mut labels = Vec::new();
    for feature in features {
        let (lines, points) = vector::feature_to_drawing(site, feature, &*to_site, t.unit_factor, style.clip);
        if lines.is_empty() && points.is_empty() {
            continue;
        }
        let data = with_data.then(|| emit::feature_data(feature)).filter(|record| !record.is_empty());
        let data = data.as_ref();
        polylines.extend(lines.iter().map(|(pts, closed)| emit::polyline(pts, *closed, false, layer, 1.0, data)));
        markers.extend(points.iter().map(|&point| emit::marker(point, layer, text_h * 0.25, data)));
        if let Some(text) = style.label_field.as_ref().and_then(|key| feature.props.get(key)).and_then(vector::prop_text) {
            let anchor = lines.first().into_iter().flat_map(|(pts, _)| pts).chain(&points);
            let (x, y, n) = anchor.fold((0.0, 0.0, 0usize), |(x, y, n), p| (x + p[0], y + p[1], n + 1));
            let at = [x / n as f64, y / n as f64];
            labels.push(emit::text(at, &text, text_h, 0.0, layer, data));
        }
    }
    polylines.append(&mut markers);
    polylines.append(&mut labels);
    if polylines.is_empty() {
        return Ok(0);
    }
    bridge::with_import_undo(label, || {
        bridge::ensure_layer(layer, Color::from_index(style.aci));
        Ok(bridge::add_entities_with_object_data(label, polylines)?.len())
    })
}

pub fn wfs_fetch(site: &Site, caps: &WfsCaps, ft: &WfsType, limit: usize, progress: &dyn Fn(String)) -> Result<wfs::WfsResult, String> {
    wfs::fetch(site, caps, ft, limit, progress)
}

pub fn wfs_place(site: &Site, ft: &WfsType, result: &wfs::WfsResult, style: &VectorStyle) -> Result<String, String> {
    let message = write_wfs(site, ft, result, style)?;
    bridge::zoom_extents();
    Ok(message)
}

fn write_wfs(site: &Site, ft: &WfsType, result: &wfs::WfsResult, style: &VectorStyle) -> Result<String, String> {
    let layer = emit::layer_name("GIS", ft.name.rsplit(':').next().unwrap_or(&ft.name));
    let n = place_features(site, &result.features, &result.data_crs, &layer, &format!("GIS WFS {}", ft.title), style, true)?;
    Ok(format!("{n} objecten van {} op laag {layer}.", ft.title))
}

/// One layer of any service (service browser, `GIS_SERVICE_LOAD`) for the
/// project area: WMS and tiles as images, WFS as features with their
/// attributes. Capabilities are fetched when not given; `layer` defaults to
/// the server's preset layer, then the first.
#[allow(clippy::too_many_arguments)]
pub fn service_layer(site: &Site, server: &MapServer, wms_caps: Option<&WmsCaps>, wfs_caps: Option<&WfsCaps>, layer: Option<&str>, resolution: f64, limit: usize, progress: &dyn Fn(String)) -> Result<String, String> {
    let wanted = layer.filter(|l| !l.is_empty()).or((!server.default_layer.is_empty()).then_some(server.default_layer.as_str()));
    match server.kind {
        ServerKind::Xyz => xyz(site, server, &RasterOptions { resolution, format: "image/png".into() }, 0, progress),
        ServerKind::Wms => {
            let fetched;
            let c = match wms_caps {
                Some(c) => c,
                None => {
                    progress("Capabilities ophalen…".into());
                    fetched = caps::fetch_wms(&server.url)?;
                    &fetched
                }
            };
            let l = c.layers.iter().find(|l| Some(l.name.as_str()) == wanted).or(c.layers.first()).ok_or("de dienst heeft geen lagen")?;
            // Keep the image within the pixel limit for large areas.
            let res = resolution.max((site.width * site.height / raster::MAX_TOTAL_PX).sqrt());
            wms(site, server, c, l, &RasterOptions { resolution: res, format: raster::default_format(c, &l.name) }, 0, progress)
        }
        ServerKind::Wfs => {
            let fetched;
            let c = match wfs_caps {
                Some(c) => c,
                None => {
                    progress("Capabilities ophalen…".into());
                    fetched = caps::fetch_wfs(&server.url)?;
                    &fetched
                }
            };
            let t = c.types.iter().find(|t| Some(t.name.as_str()) == wanted).or(c.types.first()).ok_or("de dienst heeft geen objecttypen")?;
            let res = wfs_fetch(site, c, t, limit, progress)?;
            let names = vector::property_names(&res.features);
            const HINTS: &[&str] = &["perceelnummer", "flurstueckskennzeichen", "flurstuecksnummer", "nationalCadastralReference", "label", "huisnummer", "naam", "name"];
            let label = HINTS.iter().find(|h| names.iter().any(|n| n == *h)).map(|h| h.to_string());
            wfs_place(site, t, &res, &VectorStyle { aci: 2, label_field: label, text_m: 1.0, clip: true })
        }
    }
}

pub fn osm_fetch(site: &Site, theme_ids: &[&str], progress: &dyn Fn(String)) -> Result<Vec<Vec<Feature>>, String> {
    let themes: Vec<&osm::Theme> = osm::THEMES.iter().filter(|t| theme_ids.contains(&t.id)).collect();
    if themes.is_empty() {
        return Err("kies minstens één thema".into());
    }
    osm::fetch(site, &themes, progress)
}

/// Place OpenStreetMap features fetched for `theme_ids` (same order).
pub fn osm_place(site: &Site, theme_ids: &[&str], per_theme: &[Vec<Feature>], labels: bool, text_m: f64, clip: bool) -> Result<String, String> {
    target()?;
    let summary = bridge::with_import_undo("OpenStreetMap", || {
        let mut summary = Vec::new();
        for (id, features) in theme_ids.iter().zip(per_theme) {
            let Some(theme) = osm::THEMES.iter().find(|t| t.id == *id) else { continue };
            let style = VectorStyle { aci: theme.aci, label_field: labels.then(|| theme.label_tag.map(str::to_string)).flatten(), text_m, clip };
            let n = place_features(site, features, "EPSG:4326", theme.layer, &format!("GIS OSM {}", theme.label), &style, false)?;
            summary.push(format!("{}: {n}", theme.label));
        }
        Ok(summary)
    })?;
    bridge::zoom_extents();
    Ok(format!("OpenStreetMap — {} (© OpenStreetMap-bijdragers, ODbL).", summary.join(", ")))
}

pub fn osm(site: &Site, theme_ids: &[&str], labels: bool, text_m: f64, clip: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    target()?;
    let per_theme = osm_fetch(site, theme_ids, progress)?;
    osm_place(site, theme_ids, &per_theme, labels, text_m, clip)
}

/// A base point marker in the drawing (the GeoOrigin of the FreeCAD workbench):
/// cross, circle and the coordinates, on layer GIS-BASISPUNT.
pub fn geo_origin(site: &Site) -> Result<String, String> {
    use ocs_plugin_api::host::codec::{Circle, EntityType, Line, Text, Vector3};
    let t = target()?;
    let f = t.unit_factor;
    let p = site.to_drawing([site.origin_e, site.origin_n], f);
    let r = (site.width.max(site.height) / 100.0).clamp(0.5, 25.0) * f;
    let layer = "GIS-BASISPUNT";
    let mut ents = vec![
        EntityType::Line(Line::from_points(Vector3::new(p[0] - 2.0 * r, p[1], 0.0), Vector3::new(p[0] + 2.0 * r, p[1], 0.0))),
        EntityType::Line(Line::from_points(Vector3::new(p[0], p[1] - 2.0 * r, 0.0), Vector3::new(p[0], p[1] + 2.0 * r, 0.0))),
        EntityType::Circle(Circle::from_center_radius(Vector3::new(p[0], p[1], 0.0), r)),
        EntityType::Text(
            Text::with_value(format!("GIS-basispunt {}  X {:.3}  Y {:.3}", site.crs, site.origin_e, site.origin_n), Vector3::new(p[0] + 2.2 * r, p[1] + 0.4 * r, 0.0))
                .with_height(0.6 * r),
        ),
    ];
    for e in &mut ents {
        e.common_mut().layer = layer.into();
        e.common_mut().color = Color::ByLayer;
    }
    bridge::with_import_undo("GIS basispunt", || {
        bridge::ensure_layer(layer, Color::from_index(1));
        bridge::add_entities("GIS basispunt", ents)
    })?;
    Ok(format!("Basispunt geplaatst op laag {layer}."))
}

/// A dataset that can be imported in one go (ribbon "Snel", PDOK (NL) page).
pub enum Dataset {
    Raster { server: &'static str },
    Wfs { server: &'static str, typename: &'static str, label: Option<&'static str>, aci: i16 },
}

pub struct Catalogued {
    pub id: &'static str,
    pub label: &'static str,
    pub label_en: &'static str,
    pub group: &'static str,
    pub group_en: &'static str,
    pub default_on: bool,
    pub data: Dataset,
}

pub const PDOK: &[Catalogued] = &[
    Catalogued { id: "luchtfoto", label: "Luchtfoto 8 cm (Beeldmateriaal)", label_en: "8 cm aerial imagery (Beeldmateriaal)", group: "Kaartbeelden", group_en: "Map imagery", default_on: true, data: Dataset::Raster { server: "pdok-luchtfoto" } },
    Catalogued { id: "kadasterkaart", label: "Kadastrale kaart", label_en: "Cadastral map", group: "Kaartbeelden", group_en: "Map imagery", default_on: false, data: Dataset::Raster { server: "pdok-kadastralekaart-wms" } },
    Catalogued { id: "ahn", label: "AHN hoogtekaart (DTM 0,5 m)", label_en: "AHN elevation map (0.5 m DTM)", group: "Kaartbeelden", group_en: "Map imagery", default_on: false, data: Dataset::Raster { server: "pdok-ahn-wms" } },
    Catalogued { id: "bag", label: "BAG panden", label_en: "BAG buildings", group: "Vectordata", group_en: "Vector data", default_on: true, data: Dataset::Wfs { server: "pdok-bag-wfs", typename: "bag:pand", label: None, aci: 30 } },
    Catalogued { id: "percelen", label: "Kadastrale percelen (met perceelnummer)", label_en: "Cadastral parcels (with parcel numbers)", group: "Vectordata", group_en: "Vector data", default_on: true, data: Dataset::Wfs { server: "pdok-kadastralekaart-wfs", typename: "kadastralekaart:Perceel", label: Some("perceelnummer"), aci: 3 } },
    Catalogued { id: "nwb", label: "NWB wegvakken (met straatnaam)", label_en: "NWB road sections (with street names)", group: "Infra", group_en: "Infrastructure", default_on: true, data: Dataset::Wfs { server: "pdok-nwbwegen-wfs", typename: "nwbwegen:wegvakken", label: Some("stt_naam"), aci: 2 } },
    Catalogued { id: "hecto", label: "NWB hectometerpunten", label_en: "NWB hectometre markers", group: "Infra", group_en: "Infrastructure", default_on: false, data: Dataset::Wfs { server: "pdok-nwbwegen-wfs", typename: "nwbwegen:hectopunten", label: Some("hectomtrng"), aci: 2 } },
    Catalogued { id: "spoor", label: "Spoorassen (ProRail)", label_en: "Railway centrelines (ProRail)", group: "Infra", group_en: "Infrastructure", default_on: false, data: Dataset::Wfs { server: "pdok-spoorwegen-wfs", typename: "spoorwegen:spooras", label: None, aci: 6 } },
    Catalogued { id: "spoorkm", label: "Spoorkilometrering (ProRail)", label_en: "Railway kilometre markers (ProRail)", group: "Infra", group_en: "Infrastructure", default_on: false, data: Dataset::Wfs { server: "pdok-spoorwegen-wfs", typename: "spoorwegen:kilometrering", label: Some("kmGeocodeT"), aci: 6 } },
    Catalogued { id: "vaarwegen", label: "NWB vaarwegen", label_en: "NWB waterways", group: "Infra", group_en: "Infrastructure", default_on: false, data: Dataset::Wfs { server: "pdok-nwbvaarwegen-wfs", typename: "", label: None, aci: 5 } },
];

pub(crate) enum PreparedDataset {
    Map { images: Vec<raster::PlacedImage>, layer: String, label: String, summary: String },
    Vector { features: wfs::WfsResult, feature_type: WfsType, style: VectorStyle },
}

impl PreparedDataset {
    fn write(self, site: &Site, rasters: &mut Vec<Handle>) -> Result<String, String> {
        match self {
            Self::Map { images, layer, label, summary } => {
                rasters.extend(write_map(site, &images, &layer, target()?.unit_factor, 0, &label)?);
                Ok(summary)
            }
            Self::Vector { features, feature_type, style } => write_wfs(site, &feature_type, &features, &style),
        }
    }
}

/// Fetch everything before asking the host to start recording the import.
pub(crate) fn prepare_dataset(site: &Site, data: &Dataset, progress: &dyn Fn(String)) -> Result<PreparedDataset, String> {
    match data {
        Dataset::Raster { server } => {
            let srv = servers::find(server).ok_or_else(|| format!("server {server} onbekend"))?;
            // Large areas: no more pixels than a 4096-wide image (a quick
            // import of 30 km at 25 cm would be billions of pixels).
            let cap = |fine: f64| fine.max(site.width.max(site.height) / 4096.0);
            if srv.kind == ServerKind::Xyz {
                let opt = RasterOptions { resolution: cap(0.5), format: "image/png".into() };
                let images = raster::import_xyz(site, &srv, &opt, &bridge::image_dir(), progress)?;
                let layer = emit::layer_name("GIS", &srv.name);
                let label = format!("GIS {}", srv.name);
                let summary = format!("{} op laag {layer} ({}).", srv.name, srv.attribution);
                return Ok(PreparedDataset::Map { images, layer, label, summary });
            }
            let caps = caps::fetch_wms(&srv.url)?;
            let layer = caps.layers.iter().find(|l| l.name == srv.default_layer).or(caps.layers.first()).cloned().ok_or("geen lagen")?;
            let opt = RasterOptions { resolution: cap(0.25), format: raster::default_format(&caps, &layer.name) };
            prepare_wms(site, &srv, &caps, &layer, &opt, progress)
        }
        Dataset::Wfs { server, typename, label, aci } => {
            let srv = servers::find(server).ok_or_else(|| format!("server {server} onbekend"))?;
            let caps = caps::fetch_wfs(&srv.url)?;
            let ft = if typename.is_empty() { caps.types.first().cloned() } else { caps.types.iter().find(|t| t.name.eq_ignore_ascii_case(typename)).cloned() }
                .ok_or_else(|| format!("objecttype {typename} niet gevonden"))?;
            let res = wfs_fetch(site, &caps, &ft, 20_000, progress)?;
            let style = VectorStyle { aci: *aci, label_field: label.map(str::to_string), text_m: 1.0, clip: true };
            Ok(PreparedDataset::Vector { features: res, feature_type: ft, style })
        }
    }
}

fn prepare_wms(site: &Site, server: &MapServer, caps: &WmsCaps, layer: &WmsLayer, opt: &RasterOptions, progress: &dyn Fn(String)) -> Result<PreparedDataset, String> {
    let images = raster::import_wms(site, caps, layer, opt, &bridge::image_dir(), progress)?;
    let name = emit::layer_name("GIS-WMS", &layer.title);
    let label = format!("GIS WMS {}", layer.title);
    let summary = format!("{} kaartdeel/delen van {} op laag {name} ({}).", images.len(), server.name, server.attribution);
    Ok(PreparedDataset::Map { images, layer: name, label, summary })
}

pub(crate) fn place_datasets(site: &Site, label: &str, origin: bool, datasets: Vec<PreparedDataset>) -> Result<Vec<String>, String> {
    if !origin && datasets.is_empty() {
        return Ok(Vec::new());
    }
    let messages = bridge::with_import_undo(label, || {
        let mut messages = Vec::new();
        let mut rasters = Vec::new();
        if origin {
            messages.push(geo_origin(site)?);
        }
        for dataset in datasets {
            messages.push(dataset.write(site, &mut rasters)?);
        }
        bridge::image_to_back(&rasters);
        Ok(messages)
    })?;
    bridge::zoom_extents();
    Ok(messages)
}

/// Import one catalogued dataset with sensible defaults.
pub fn run_dataset(site: &Site, data: &Dataset, progress: &dyn Fn(String)) -> Result<String, String> {
    target()?;
    let dataset = prepare_dataset(site, data, progress)?;
    let label = match &dataset {
        PreparedDataset::Map { label, .. } => label.clone(),
        PreparedDataset::Vector { feature_type, .. } => format!("GIS WFS {}", feature_type.title),
    };
    Ok(place_datasets(site, &label, false, vec![dataset])?.join(" "))
}

/// Import any server (preset, worldwide list or the user's own) by id: an
/// XYZ/tile server or WMS layer as image, a WFS feature type as vectors.
/// `layer` overrides the server's default layer; `res` the resolution (m/px).
pub fn run_server(site: &Site, id: &str, layer: Option<&str>, res: Option<f64>, progress: &dyn Fn(String)) -> Result<String, String> {
    let srv = servers::find(id).ok_or_else(|| format!("server {id} onbekend (zie de serverlijst in het werkvenster)"))?;
    // Large areas: no more pixels than a 4096-wide image.
    let auto = |fine: f64| res.unwrap_or(fine).max(site.width.max(site.height) / 4096.0);
    match srv.kind {
        ServerKind::Xyz => xyz(site, &srv, &RasterOptions { resolution: auto(0.5), format: "image/png".into() }, 0, progress),
        ServerKind::Wms => {
            let caps = caps::fetch_wms(&srv.url)?;
            let want = layer.unwrap_or(&srv.default_layer);
            // A list of layers (`a,b`) goes as one request, on the first's settings.
            let first = want.split(',').next().unwrap_or("").trim();
            let mut l = caps.layers.iter().find(|l| l.name == want || l.name == first).or(caps.layers.iter().find(|l| !l.name.is_empty())).cloned().ok_or("geen lagen")?;
            if want.contains(',') && l.name == first {
                l.name = want.to_string();
            }
            let opt = RasterOptions { resolution: auto(0.25), format: raster::default_format(&caps, &l.name) };
            wms(site, &srv, &caps, &l, &opt, 0, progress)
        }
        ServerKind::Wfs => {
            let caps = caps::fetch_wfs(&srv.url)?;
            let want = layer.unwrap_or(&srv.default_layer);
            let ft = caps.types.iter().find(|t| t.name.eq_ignore_ascii_case(want)).or(caps.types.first()).cloned().ok_or("geen objecttypen")?;
            let r = wfs_fetch(site, &caps, &ft, 20_000, progress)?;
            wfs_place(site, &ft, &r, &VectorStyle { aci: 3, label_field: None, text_m: 1.0, clip: true })
        }
    }
}

/// Aerial photo for the site: PDOK 8 cm in the Netherlands, Sentinel-2
/// (10 m) elsewhere — the "Import Aerialphoto" option of the location dialog.
pub(crate) fn prepare_aerial(site: &Site, progress: &dyn Fn(String)) -> Result<PreparedDataset, String> {
    let in_nl = site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7;
    if in_nl {
        return prepare_dataset(site, &Dataset::Raster { server: "pdok-luchtfoto" }, progress);
    }
    let srv = servers::find("eox-s2-wms").ok_or("server eox-s2-wms onbekend")?;
    let caps = caps::fetch_wms(&srv.url)?;
    let layer = caps.layers.iter().find(|l| l.name == srv.default_layer).or(caps.layers.first()).cloned().ok_or("geen lagen")?;
    let res = (site.width.max(site.height) / 2048.0).max(5.0);
    prepare_wms(site, &srv, &caps, &layer, &RasterOptions { resolution: res, format: "image/jpeg".into() }, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{EntityType, Handle};
    use ocs_plugin_api::host::{ExtendedDataRecord, PluginRequestError, PluginRequestSender, XDataValue};
    use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Drawing {
        entities: Mutex<Vec<EntityType>>,
        records_written: Mutex<Vec<(Handle, ExtendedDataRecord)>>,
    }

    impl PluginRequestSender for Drawing {
        fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
            Ok(match request {
                PluginRequest::AddEntities(entities) => {
                    let mut placed = self.entities.lock().unwrap();
                    let first = placed.len() as u64 + 1;
                    let handles = (first..first + entities.len() as u64).map(Handle::new).collect();
                    placed.extend(entities);
                    PluginResponse::Handles(handles)
                }
                PluginRequest::WriteRecord { handle, record } => {
                    self.records_written.lock().unwrap().push((handle, record));
                    PluginResponse::Bool(true)
                }
                _ => PluginResponse::Ok,
            })
        }
    }

    #[test]
    fn wfs_bag_pand_carries_source_attributes() {
        // Synthetic GetFeature response in RD, with the BAG field names.
        let response = serde_json::json!({
            "type": "FeatureCollection",
            "features": [{
                "type": "Feature",
                "properties": { "identificatie": "0503100000000001", "bouwjaar": 1984 },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[105000, 425000], [105010, 425000], [105010, 425010], [105000, 425000]]]
                }
            }]
        });
        let site = Site { origin_e: 105000.0, origin_n: 425000.0, ..Site::default() };
        let ft = WfsType { name: "bag:pand".into(), title: "Pand".into(), crs: vec![site.crs.clone()], bbox: None };
        let result = wfs::WfsResult { features: vector::parse_geojson(&response), data_crs: site.crs.clone(), truncated: false };
        let style = VectorStyle { aci: 30, label_field: None, text_m: 1.0, clip: true };
        let drawing = Arc::new(Drawing::default());
        let target = Target { sender: drawing.clone(), tab_id: 139, unit_factor: 1.0, doc_path: None };

        bridge::with_command_context(target, None, None, || wfs_place(&site, &ft, &result, &style)).unwrap();

        let placed = drawing.entities.lock().unwrap();
        assert_eq!(placed.len(), 1);
        assert!(matches!(&placed[0], EntityType::LwPolyline(p) if p.is_closed));
        assert_eq!(placed[0].common().layer, "GIS-PAND");
        let record = placed[0].common().extended_data.get_record(bridge::XDATA_APP)
            .expect("GIS-PAND must carry OPENINFRA XDATA");
        assert!(record.values.contains(&XDataValue::String("bouwjaar=1984".into())));
        assert!(record.values.contains(&XDataValue::String("identificatie=0503100000000001".into())));
        assert_eq!(*drawing.records_written.lock().unwrap(), vec![(Handle::new(1), record.clone())]);
    }

    #[test]
    fn wfs_parts_keep_their_own_attributes_after_clipping() {
        let response = serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "properties": { "name": null },
                    "geometry": { "type": "LineString", "coordinates": [[1,1],[2,2]] }
                },
                {
                    "type": "Feature",
                    "properties": { "name": "outside" },
                    "geometry": { "type": "Point", "coordinates": [20,20] }
                },
                {
                    "type": "Feature",
                    "properties": { "name": "road" },
                    "geometry": { "type": "LineString", "coordinates": [[-1,3],[5,3],[11,3],[11,4],[5,4]] }
                },
                {
                    "type": "Feature",
                    "properties": { "name": "stops", "active": true },
                    "geometry": { "type": "MultiPoint", "coordinates": [[2,3],[4,3],[20,20]] }
                }
            ]
        });
        let site = Site { origin_e: 5.0, origin_n: 5.0, width: 10.0, height: 10.0, ..Site::default() };
        let ft = WfsType { name: "test:features".into(), title: "Test".into(), crs: vec![site.crs.clone()], bbox: None };
        let result = wfs::WfsResult { features: vector::parse_geojson(&response), data_crs: site.crs.clone(), truncated: false };
        let style = VectorStyle { aci: 2, label_field: Some("name".into()), text_m: 1.0, clip: true };
        let drawing = Arc::new(Drawing::default());
        let target = Target { sender: drawing.clone(), tab_id: 139, unit_factor: 1.0, doc_path: None };

        bridge::with_command_context(target, None, None, || wfs_place(&site, &ft, &result, &style)).unwrap();

        let placed = drawing.entities.lock().unwrap();
        assert!(matches!(placed.as_slice(), [
            EntityType::LwPolyline(_), EntityType::LwPolyline(_), EntityType::LwPolyline(_),
            EntityType::Circle(_), EntityType::Circle(_), EntityType::Text(_), EntityType::Text(_),
        ]));
        let records: Vec<_> = placed.iter().map(|entity| entity.common().extended_data.get_record(bridge::XDATA_APP)).collect();
        assert!(records[0].is_none());
        let expected = |name: &str| XDataValue::String(format!("name={name}"));
        for i in [1, 2, 5] {
            assert_eq!(records[i].unwrap().values, [expected("road")]);
        }
        for i in [3, 4, 6] {
            assert_eq!(records[i].unwrap().values, [XDataValue::String("active=true".into()), expected("stops")]);
        }
        assert_eq!(*drawing.records_written.lock().unwrap(), vec![(Handle::new(2), records[1].unwrap().clone())]);
    }
}
