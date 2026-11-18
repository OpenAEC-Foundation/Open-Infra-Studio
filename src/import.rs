//! Import operations shared by the workbench window and the command line:
//! fetch with the engine, convert with `emit`, write through `bridge`.

use ocs_plugin_api::host::codec::Color;

use crate::bridge::{self, Target};
use crate::emit;
use crate::gis::caps::{WfsCaps, WfsType, WmsCaps, WmsLayer};
use crate::gis::raster::{self, Options as RasterOptions};
use crate::gis::servers::{self, MapServer, ServerKind};
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature, Options as VectorOptions};
use crate::gis::{caps, osm, wfs};

fn target() -> Result<Target, String> {
    bridge::target().ok_or_else(|| "geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad".into())
}

pub fn wms(site: &Site, server: &MapServer, caps: &WmsCaps, layer: &WmsLayer, opt: &RasterOptions, fade: u8, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = target()?;
    let placed = raster::import_wms(site, caps, layer, opt, &bridge::image_dir(), progress)?;
    let layer_name = emit::layer_name("GIS-WMS", &layer.title);
    bridge::ensure_layer(&layer_name, Color::from_index(8));
    let handles = bridge::add_imported_map(&format!("GIS WMS {}", layer.title), emit::images(site, &placed, &layer_name, t.unit_factor, fade))?;
    let n = handles.len();
    bridge::zoom_extents();
    Ok(format!("{n} kaartdeel/delen van {} op laag {layer_name} ({}).", server.name, server.attribution))
}

pub fn xyz(site: &Site, server: &MapServer, opt: &RasterOptions, fade: u8, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = target()?;
    let placed = raster::import_xyz(site, server, opt, &bridge::image_dir(), progress)?;
    let layer_name = emit::layer_name("GIS", &server.name);
    bridge::ensure_layer(&layer_name, Color::from_index(8));
    bridge::add_imported_map(&format!("GIS {}", server.name), emit::images(site, &placed, &layer_name, t.unit_factor, fade))?;
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

pub fn place_features(site: &Site, features: &[Feature], data_crs: &str, layer: &str, label: &str, style: &VectorStyle) -> Result<usize, String> {
    let t = target()?;
    let to_site = vector::data_to_site(site, data_crs, features)?;
    let geom = vector::to_drawing(
        site,
        features,
        &*to_site,
        &VectorOptions { unit_factor: t.unit_factor, clip: style.clip, label_field: style.label_field.clone() },
    );
    if geom.is_empty() {
        return Ok(0);
    }
    bridge::ensure_layer(layer, Color::from_index(style.aci));
    bridge::add_imported_entities(label, emit::geometry(&geom, layer, style.text_m * t.unit_factor)).map(|h| h.len())
}

pub fn wfs_fetch(site: &Site, caps: &WfsCaps, ft: &WfsType, limit: usize, progress: &dyn Fn(String)) -> Result<wfs::WfsResult, String> {
    wfs::fetch(site, caps, ft, limit, progress)
}

pub fn wfs_place(site: &Site, ft: &WfsType, result: &wfs::WfsResult, style: &VectorStyle) -> Result<String, String> {
    let layer = emit::layer_name("GIS", ft.name.rsplit(':').next().unwrap_or(&ft.name));
    let n = place_features(site, &result.features, &result.data_crs, &layer, &format!("GIS WFS {}", ft.title), style)?;
    bridge::zoom_extents();
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
    let mut summary = Vec::new();
    for (id, features) in theme_ids.iter().zip(per_theme) {
        let Some(theme) = osm::THEMES.iter().find(|t| t.id == *id) else { continue };
        let style = VectorStyle { aci: theme.aci, label_field: labels.then(|| theme.label_tag.map(str::to_string)).flatten(), text_m, clip };
        let n = place_features(site, features, "EPSG:4326", theme.layer, &format!("GIS OSM {}", theme.label), &style)?;
        summary.push(format!("{}: {n}", theme.label));
    }
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
    bridge::ensure_layer(layer, Color::from_index(1));
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
    bridge::add_entities("GIS basispunt", ents)?;
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

/// Import one catalogued dataset with sensible defaults.
pub fn run_dataset(site: &Site, data: &Dataset, progress: &dyn Fn(String)) -> Result<String, String> {
    match data {
        Dataset::Raster { server } => {
            let srv = servers::find(server).ok_or_else(|| format!("server {server} onbekend"))?;
            // Large areas: no more pixels than a 4096-wide image (a quick
            // import of 30 km at 25 cm would be billions of pixels).
            let cap = |fine: f64| fine.max(site.width.max(site.height) / 4096.0);
            if srv.kind == ServerKind::Xyz {
                return xyz(site, &srv, &RasterOptions { resolution: cap(0.5), format: "image/png".into() }, 0, progress);
            }
            let caps = caps::fetch_wms(&srv.url)?;
            let layer = caps.layers.iter().find(|l| l.name == srv.default_layer).or(caps.layers.first()).cloned().ok_or("geen lagen")?;
            let opt = RasterOptions { resolution: cap(0.25), format: raster::default_format(&caps, &layer.name) };
            wms(site, &srv, &caps, &layer, &opt, 0, progress)
        }
        Dataset::Wfs { server, typename, label, aci } => {
            let srv = servers::find(server).ok_or_else(|| format!("server {server} onbekend"))?;
            let caps = caps::fetch_wfs(&srv.url)?;
            let ft = if typename.is_empty() { caps.types.first().cloned() } else { caps.types.iter().find(|t| t.name.eq_ignore_ascii_case(typename)).cloned() }
                .ok_or_else(|| format!("objecttype {typename} niet gevonden"))?;
            let res = wfs_fetch(site, &caps, &ft, 20_000, progress)?;
            let style = VectorStyle { aci: *aci, label_field: label.map(str::to_string), text_m: 1.0, clip: true };
            wfs_place(site, &ft, &res, &style)
        }
    }
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
pub fn aerial(site: &Site, progress: &dyn Fn(String)) -> Result<String, String> {
    let in_nl = site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7;
    if in_nl {
        return run_dataset(site, &Dataset::Raster { server: "pdok-luchtfoto" }, progress);
    }
    let srv = servers::find("eox-s2-wms").ok_or("server eox-s2-wms onbekend")?;
    let caps = caps::fetch_wms(&srv.url)?;
    let layer = caps.layers.iter().find(|l| l.name == srv.default_layer).or(caps.layers.first()).cloned().ok_or("geen lagen")?;
    let res = (site.width.max(site.height) / 2048.0).max(5.0);
    wms(site, &srv, &caps, &layer, &RasterOptions { resolution: res, format: "image/jpeg".into() }, 0, progress)
}

#[cfg(test)]
mod import_layer_tests {
    use super::*;
    use ocs_plugin_api::host::{codec::{CadDocument, Handle}, PluginRequestError, PluginRequestSender};
    use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};
    use std::sync::{Arc, Mutex};

    struct Drawing(Mutex<CadDocument>);

    impl PluginRequestSender for Drawing {
        fn request(&self, req: Q) -> Result<A, PluginRequestError> {
            let mut doc = self.0.lock().unwrap();
            Ok(match req {
                Q::DocumentSnapshot => A::Document(Box::new(doc.clone().into())),
                Q::AddLayer(config) => {
                    let mut layer = ocs_plugin_api::host::codec::tables::Layer::new(&config.name);
                    layer.flags.locked = config.locked.unwrap_or_default();
                    let handle = doc.allocate_handle();
                    layer.handle = handle;
                    A::OptHandle(doc.layers.add(layer).ok().map(|()| handle))
                }
                Q::ModifyLayer(config) => {
                    let layer = doc.layers.get_mut(&config.name).unwrap();
                    layer.flags.locked = config.locked.unwrap();
                    A::Bool(true)
                }
                Q::AddEntities(entities) => A::Handles(entities.into_iter().map(|e| {
                    if doc.layers.get(&e.common().layer).is_some_and(|l| l.is_locked()) || e.common().layer == "GIS-REFUSED" {
                        Handle::NULL
                    } else {
                        doc.add_entity(e).unwrap()
                    }
                }).collect()),
                Q::PushUndo { .. } | Q::BeginUndoGroup { .. } | Q::EndUndoGroup | Q::SetDirty | Q::BumpGeometry => A::Ok,
                other => panic!("unexpected host request: {other:?}"),
            })
        }
    }

    fn import_point(layer: &str) -> Result<usize, String> {
        let site = Site::default();
        let feature = Feature { points: vec![[site.lon, site.lat]], ..Default::default() };
        let style = VectorStyle { aci: 7, label_field: None, text_m: 1.0, clip: false };
        place_features(&site, &[feature], "EPSG:4326", layer, "WFS", &style)
    }

    #[test]
    fn vector_import_locks_layer_and_can_append_to_it() {
        crate::settings::with_test_file(|| {
            let drawing = Arc::new(Drawing(Mutex::new(CadDocument::new())));
            let target = Target { sender: drawing.clone(), tab_id: 136, unit_factor: 1.0, doc_path: None };
            bridge::with_command_context(target, None, None, || {
                for expected in 1..=2 {
                    assert_eq!(import_point("GIS-TEST").unwrap(), 1);
                    let doc = drawing.0.lock().unwrap();
                    assert!(doc.layers.get("GIS-TEST").unwrap().is_locked(), "the GIS layer must be locked after import");
                    assert_eq!(doc.model_space_entities().count(), expected);
                }
            });
        });
    }

    #[test]
    fn import_layer_preference_off_preserves_existing_locks() {
        use crate::settings;
        settings::with_test_file(|| {
            settings::update(&settings::path(), |s| { s.insert("language".into(), "nl".into()); }).unwrap();
            assert!(settings::lock_imported_layers());
            settings::save_lock_imported_layers(false).unwrap();
            assert!(!settings::lock_imported_layers());
            let settings: serde_json::Value = serde_json::from_slice(&std::fs::read(settings::path()).unwrap()).unwrap();
            assert_eq!(settings["language"], "nl");
            let drawing = Arc::new(Drawing(Mutex::new(CadDocument::new())));
            let target = Target { sender: drawing.clone(), tab_id: 137, unit_factor: 1.0, doc_path: None };
            bridge::with_command_context(target, None, None, || {
                assert_eq!(import_point("GIS-OPEN").unwrap(), 1);
                assert!(!drawing.0.lock().unwrap().layers.get("GIS-OPEN").unwrap().is_locked());
                drawing.0.lock().unwrap().layers.get_mut("GIS-OPEN").unwrap().flags.locked = true;
                assert_eq!(import_point("gis-open").unwrap(), 1);
                let doc = drawing.0.lock().unwrap();
                assert!(doc.layers.get("GIS-OPEN").unwrap().is_locked());
                assert_eq!(doc.model_space_entities().count(), 2);
            });
        });
    }

    #[test]
    fn refused_import_restores_the_previous_layer_lock() {
        crate::settings::with_test_file(|| {
            let mut doc = CadDocument::new();
            let mut layer = ocs_plugin_api::host::codec::tables::Layer::new("GIS-REFUSED");
            layer.flags.locked = true;
            doc.layers.add(layer).unwrap();
            let drawing = Arc::new(Drawing(Mutex::new(doc)));
            let target = Target { sender: drawing.clone(), tab_id: 138, unit_factor: 1.0, doc_path: None };
            bridge::with_command_context(target, None, None, || {
                assert!(import_point("GIS-REFUSED").is_err());
                let doc = drawing.0.lock().unwrap();
                assert!(doc.layers.get("GIS-REFUSED").unwrap().is_locked());
                assert_eq!(doc.model_space_entities().count(), 0);
            });
        });
    }
}
