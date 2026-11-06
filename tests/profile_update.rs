//! Bijwerken via de echte opdracht, met een tekening aan de hostgrens.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{
    codec::{entities::Face3D, CadDocument, EntityType, Handle, LwPolyline, Vector2, Vector3},
    PluginRequestError, PluginRequestSender, XRecordEntry,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use open_infra::{bridge, civiel::{graaf, CivielOptions}};

#[derive(Clone)]
struct State {
    entities: Vec<EntityType>,
    records: HashMap<String, Vec<XRecordEntry>>,
}

struct Drawing {
    state: Mutex<State>,
    undo: Mutex<Vec<State>>,
    grouped: Mutex<bool>,
    writes: Mutex<Vec<&'static str>>,
    fail_add: bool,
}

impl PluginRequestSender for Drawing {
    fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        let mut state = self.state.lock().unwrap();
        Ok(match request {
            PluginRequest::ReadXRecord { name, .. } => PluginResponse::XRecordEntries(Ok(state.records.get(&name).cloned().unwrap_or_default())),
            PluginRequest::UpdateXRecord { name, entries, .. } | PluginRequest::CreateXRecord { name, entries, .. } => {
                self.writes.lock().unwrap().push("record");
                state.records.insert(name, entries);
                PluginResponse::XRecordUpdateResult(Ok(Handle::from(9000u64)))
            }
            PluginRequest::OpenDocumentViewV4 { .. } => return Err(PluginRequestError("geen gedeelde weergave".into())),
            PluginRequest::CloseDocumentViewV4 { .. } => PluginResponse::Ok,
            PluginRequest::GetSelection => PluginResponse::Handles(vec![]),
            PluginRequest::GetEntitiesBatch { handles } => PluginResponse::GetEntitiesBatchResult(Ok(handles.iter().map(|h| state.entities.iter().find(|e| e.common().handle == *h).cloned()).collect())),
            PluginRequest::BeginUndoGroup { .. } => {
                assert!(!*self.grouped.lock().unwrap());
                self.undo.lock().unwrap().push(state.clone());
                *self.grouped.lock().unwrap() = true;
                PluginResponse::Ok
            }
            PluginRequest::EndUndoGroup => {
                assert!(*self.grouped.lock().unwrap());
                *self.grouped.lock().unwrap() = false;
                PluginResponse::Ok
            }
            PluginRequest::PushUndo { .. } => {
                if !*self.grouped.lock().unwrap() { self.undo.lock().unwrap().push(state.clone()); }
                PluginResponse::Ok
            }
            PluginRequest::AddEntities(entities) => {
                self.writes.lock().unwrap().push("add");
                if self.fail_add { return Err(PluginRequestError("toevoegen geweigerd".into())); }
                let mut next = state.entities.iter().map(|e| u64::from(e.common().handle)).max().unwrap_or(0) + 1;
                let handles = entities.into_iter().map(|mut e| {
                    let h = Handle::from(next);
                    next += 1;
                    e.common_mut().handle = h;
                    state.entities.push(e);
                    h
                }).collect();
                PluginResponse::Handles(handles)
            }
            PluginRequest::RemoveEntity { handle } => {
                self.writes.lock().unwrap().push("remove");
                state.entities.retain(|e| e.common().handle != handle);
                PluginResponse::Ok
            }
            PluginRequest::AddLayer(_) => PluginResponse::OptHandle(Some(Handle::from(9001u64))),
            PluginRequest::TableOperation { .. } => PluginResponse::TableResult(Ok(Handle::from(9002u64))),
            PluginRequest::WriteRecord { .. } | PluginRequest::SetDirty | PluginRequest::BumpGeometry => PluginResponse::Ok,
            other => return Err(PluginRequestError(format!("onverwacht verzoek: {other:?}"))),
        })
    }
}

fn drawing(length: f64, kinds: &[&str], fail_add: bool) -> Arc<Drawing> {
    let mut doc = CadDocument::new();
    let mut axis = LwPolyline::new();
    axis.add_point_with_bulge(Vector2::new(0.0, 0.0), 0.0);
    axis.add_point_with_bulge(Vector2::new(10.0, 0.0), 0.0);
    let axis = doc.add_entity(EntityType::LwPolyline(axis)).unwrap();
    let terrain = doc.add_entity(EntityType::Face3D({
        let mut face = Face3D::new(Vector3::new(-100.0, -100.0, 1.5), Vector3::new(2000.0, -100.0, 1.5), Vector3::new(-100.0, 2000.0, 1.5), Vector3::new(-100.0, 2000.0, 1.5));
        face.common.layer = "B-OG-TRIANGULATIE-G".into();
        face
    })).unwrap();
    let products: Vec<_> = kinds.iter().enumerate().map(|(i, kind)| {
        let old = doc.add_entity(EntityType::LwPolyline(LwPolyline::new())).unwrap();
        graaf::Product { soort: (*kind).into(), as_handle: format!("{:X}", u64::from(axis)), plaats: Some([200.0, -200.0 - i as f64 * 100.0]), bronnen: graaf::fingerprints(&doc, axis).unwrap(), gemaakt: vec![format!("{:X}", u64::from(old))] }
    }).collect();
    if let EntityType::LwPolyline(p) = doc.get_entity_mut(axis).unwrap() { p.vertices[1].location.x = length; }
    let records = HashMap::from([
        ("GRAAF".into(), vec![XRecordEntry::string(1, serde_json::to_string(&products).unwrap())]),
        ("INDEX".into(), vec![XRecordEntry::string(1, format!("tin\tmaaiveld\t{:X}", u64::from(terrain)))]),
    ]);
    Arc::new(Drawing { state: Mutex::new(State { entities: doc.model_space_entities().cloned().collect(), records }), undo: Mutex::new(vec![]), grouped: Mutex::new(false), writes: Mutex::new(vec![]), fail_add })
}

fn update(drawing: &Arc<Drawing>, tab_id: u64, every: f64) -> Result<String, String> {
    let target = bridge::Target { sender: drawing.clone(), tab_id, unit_factor: 1.0, doc_path: None };
    bridge::with_command_context(target, None, None, || graaf::update(&CivielOptions { section_every: every, ..Default::default() }, false, &|_| {}))
}

fn encoded(state: &State) -> Vec<u8> { bincode::serialize(&(&state.entities, &state.records)).unwrap() }

#[test]
fn too_many_sections_preserves_all_profiles_and_returns_the_error() {
    // Het lengteprofiel is eerst aan de beurt: ook dat moet blijven staan
    // als de volgende berekening meer dan 400 doorsneden vraagt.
    let drawing = drawing(500.0, &["lengteprofiel", "dwarsprofielen"], false);
    let before = encoded(&drawing.state.lock().unwrap());
    let result = update(&drawing, 61001, 1.0);
    assert!(result.is_err(), "berekenfout moet worden doorgegeven: {result:?}");
    assert!(result.unwrap_err().contains("501"));
    assert_eq!(encoded(&drawing.state.lock().unwrap()), before, "profielen en graaf moeten blijven staan");
    assert!(drawing.writes.lock().unwrap().is_empty());
    assert!(drawing.undo.lock().unwrap().is_empty());
}

#[test]
fn replacement_and_graph_share_one_undo_step() {
    let drawing = drawing(20.0, &["lengteprofiel", "dwarsprofielen"], false);
    let before = encoded(&drawing.state.lock().unwrap());
    update(&drawing, 61002, 10.0).unwrap();
    assert_ne!(encoded(&drawing.state.lock().unwrap()), before);
    let mut undo = drawing.undo.lock().unwrap();
    assert_eq!(undo.len(), 1, "één undo herstelt alle profielen en de graaf");
    *drawing.state.lock().unwrap() = undo.pop().unwrap();
    assert_eq!(encoded(&drawing.state.lock().unwrap()), before);
    assert!(!*drawing.grouped.lock().unwrap());
}

#[test]
fn failed_add_keeps_old_profile_and_closes_undo_group() {
    let drawing = drawing(20.0, &["dwarsprofielen"], true);
    let before = encoded(&drawing.state.lock().unwrap());
    assert!(update(&drawing, 61003, 10.0).is_err());
    assert_eq!(encoded(&drawing.state.lock().unwrap()), before);
    assert!(!drawing.writes.lock().unwrap().contains(&"remove"));
    assert!(!*drawing.grouped.lock().unwrap());
}
