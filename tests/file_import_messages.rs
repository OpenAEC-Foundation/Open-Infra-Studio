//! De echte bestandsimport bewaart goede objecten en meldt overgeslagen invoer.
use std::path::Path;
use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{codec::{EntityType, Handle}, PluginRequestError, PluginRequestSender};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use open_infra::{bestand, bridge, gis::{files, site::Site}, lang::tr};

#[derive(Default)]
struct Drawing {
    entities: Mutex<Vec<EntityType>>,
    undo: Mutex<Vec<String>>,
    unexpected: Mutex<Vec<String>>,
}

impl PluginRequestSender for Drawing {
    fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        Ok(match request {
            PluginRequest::AddLayer(_) => PluginResponse::OptHandle(Some(Handle::from(1000u64))),
            PluginRequest::PushUndo { label } => {
                self.undo.lock().unwrap().push(label);
                PluginResponse::Ok
            }
            PluginRequest::AddEntities(entities) => {
                let mut stored = self.entities.lock().unwrap();
                let handles = (stored.len()..stored.len() + entities.len()).map(|i| Handle::from(i as u64 + 1)).collect();
                stored.extend(entities);
                PluginResponse::Handles(handles)
            }
            PluginRequest::WriteRecord { .. } | PluginRequest::SetDirty | PluginRequest::BumpGeometry => PluginResponse::Ok,
            PluginRequest::ExecuteCommand(command) if command == "ZOOMEXTENTS" => PluginResponse::Bool(true),
            other => {
                let error = format!("onverwacht hostverzoek: {other:?}");
                self.unexpected.lock().unwrap().push(error.clone());
                return Err(PluginRequestError(error));
            }
        })
    }
}

fn place(data: &files::FileData, tab: u64) -> (Result<String, String>, Arc<Drawing>) {
    let site = Site { crs: "EPSG:2056".into(), origin_e: 2600000.0, origin_n: 1200000.0, ..Site::default() };
    let drawing = Arc::new(Drawing::default());
    let target = bridge::Target { sender: drawing.clone(), tab_id: tab, unit_factor: 1.0, doc_path: None };
    let options = bestand::FileOptions::for_file(Path::new("partial.gml"), data, &site);
    let result = bridge::with_command_context(target, Some(site.clone()), None, || {
        bestand::place(&site, data, &options, "partial.gml", &|_| {})
    });
    assert!(drawing.unexpected.lock().unwrap().is_empty());
    (result, drawing)
}

#[test]
fn partial_gml_import_draws_the_good_object_and_returns_its_read_warning() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gml/mixed.gml");
    let data = files::read(&path, None).unwrap();
    assert_eq!((data.features.len(), data.skipped, data.warnings.len()), (1, 1, 1));
    assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
    let (result, drawing) = place(&data, 74101);
    assert!(!drawing.entities.lock().unwrap().is_empty());
    assert_eq!(drawing.undo.lock().unwrap().len(), 1);
    let message = result.unwrap();
    assert!(message.starts_with(&format!("GML 1 {}", tr("objecten", "features"))), "{message}");
    assert!(message.ends_with(&format!("\n{}", data.warnings[0])), "{message}");
}

#[test]
fn unreadable_gml_import_returns_the_object_and_reason_without_drawing() {
    let data = files::FileData { format: "GML", crs: Some("EPSG:2056".into()), warnings: vec!["GML: object broken-line: bad coordinate".into()], skipped: 1, ..Default::default() };
    let (result, drawing) = place(&data, 74102);
    assert!(drawing.entities.lock().unwrap().is_empty());
    assert!(drawing.undo.lock().unwrap().is_empty());
    assert_eq!(result.unwrap_err(), format!("{}\n{}", tr("geen tekenbare objecten in het bestand", "no drawable features in the file"), data.warnings[0]));
}
