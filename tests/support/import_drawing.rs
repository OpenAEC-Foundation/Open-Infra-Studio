// Kleine testhost voor de echte bestandsimport en de importpagina.
use std::sync::Mutex;
use ocs_plugin_api::host::{codec::{EntityType, Handle}, PluginRequestError, PluginRequestSender, TableOperation};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};

#[derive(Default)]
pub struct Drawing {
    pub entities: Mutex<Vec<EntityType>>,
    pub undo: Mutex<Vec<String>>,
    pub layers: Mutex<Vec<String>>,
    pub unexpected: Mutex<Vec<String>>,
}

impl PluginRequestSender for Drawing {
    fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        Ok(match request {
            PluginRequest::AddLayer(layer) => {
                self.layers.lock().unwrap().push(layer.name);
                PluginResponse::OptHandle(Some(Handle::from(1000u64)))
            }
            PluginRequest::TableOperation { operation: TableOperation::LinetypeCreate { .. } } => PluginResponse::TableResult(Ok(Handle::from(1001u64))),
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

