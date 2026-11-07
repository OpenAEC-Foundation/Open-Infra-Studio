//! Eigen GeoJSON via de echte bestandsimport opnieuw exporteren zonder attribuutverlies.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use ocs_plugin_api::host::{codec::{CadDocument, EntityType, Handle}, PluginRequestError, PluginRequestSender};
use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};
use open_infra::{bestand, bridge, geoexport::{self, GeoFeature, Geom, Options}, gis::site::Site};

#[derive(Default)]
struct Drawing(Mutex<Vec<EntityType>>);
impl PluginRequestSender for Drawing {
    fn request(&self, req: Q) -> Result<A, PluginRequestError> {
        Ok(match req {
            Q::AddLayer(_) => A::OptHandle(Some(Handle::from(1000u64))),
            Q::PushUndo { .. } | Q::SetDirty | Q::BumpGeometry => A::Ok,
            Q::AddEntities(entities) => {
                let mut all = self.0.lock().unwrap();
                let mut handles = Vec::new();
                for mut e in entities {
                    let handle = Handle::from(all.len() as u64 + 1);
                    e.common_mut().handle = handle;
                    handles.push(handle);
                    all.push(e);
                }
                A::Handles(handles)
            }
            // Imported entities already carry the same object data; this
            // second write registers its application with the real host.
            Q::WriteRecord { handle, .. } => {
                assert!(self.0.lock().unwrap().iter().any(|e| e.common().handle == handle));
                A::Bool(true)
            }
            Q::ExecuteCommand(cmd) => { assert_eq!(cmd, "ZOOMEXTENTS"); A::Ok }
            other => return Err(PluginRequestError(format!("onverwacht verzoek: {other:?}"))),
        })
    }
}

#[test]
fn own_geojson_import_can_be_exported_again_without_replacing_its_source_heights() {
    let mut site = Site::default();
    site.crs = "EPSG:4326".into();
    site.absolute = true;
    let input = GeoFeature { geom: Geom::Line(vec![[4.0, 52.0, -2.25], [4.1, 52.1, -2.15]]),
        props: BTreeMap::from([("naam".into(), "originele lijn".into())]) };
    let first = geoexport::geojson(&[input], &site).unwrap();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("oi-geojson-{}-{nonce}.geojson", std::process::id()));
    std::fs::write(&path, &first).unwrap();
    let drawing = Arc::new(Drawing::default());
    let target = bridge::Target { sender: drawing.clone(), tab_id: 119001, unit_factor: 1.0, doc_path: None };
    bridge::with_command_context(target, Some(site.clone()), None, || {
        bestand::import_path(&site, &path, None, &|_| {}).unwrap();
    });
    let _ = std::fs::remove_file(&path);
    let mut doc = CadDocument::new();
    for e in drawing.0.lock().unwrap().iter() { doc.add_entity(e.clone()).unwrap(); }
    let imported = geoexport::features(&doc, &site, 1.0, &Options::default());
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].props["openaec:project_z"], "[-2.25,-2.15]");
    let second: serde_json::Value = serde_json::from_str(&geoexport::geojson(&imported, &site).unwrap()).unwrap();
    assert_eq!(second["openaec:project_z_property"], "openaec:project_z_1");
    let props = &second["features"][0]["properties"];
    assert_eq!(props["naam"], "originele lijn");
    assert_eq!(props["openaec:project_z"], "[-2.25,-2.15]", "eerdere hoogten blijven behouden");
    // The existing reader imports XY geometry. Its current zero heights
    // are distinct from the source height attribute, never substituted for it.
    assert_eq!(props["openaec:project_z_1"], serde_json::json!([0.0, 0.0]));
    assert!(second["features"][0]["geometry"]["coordinates"].as_array().unwrap().iter().all(|p| p.as_array().unwrap().len() == 2));
}
