//! Parkeer- en veegpadopdrachten blijven op A wanneer B tijdens het werk actief wordt.
use std::any::Any;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use ocs_plugin_api::host::{
    codec::{Document, EntityType, Handle, Line, Vector3}, DocumentReader, ExtendedDataRecord,
    HostApi, InteractiveCommand, PluginRequestError, PluginRequestSender, XRecordEntry,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};
use open_infra::{bridge, library};

struct Drawing {
    drawn: Mutex<Vec<EntityType>>,
    pause: Mutex<Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>>,
    pause_on_read: bool,
    done: mpsc::Sender<bool>,
}
impl PluginRequestSender for Drawing {
    fn request(&self, req: Q) -> Result<A, PluginRequestError> {
        let stop = if self.pause_on_read { matches!(&req, Q::GetEntitiesBatch { .. }) } else { matches!(&req, Q::AddLayer(_)) };
        if stop {
            let pause = self.pause.lock().unwrap().take();
            if let Some((entered, resume)) = pause {
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
        Ok(match req {
            Q::GetEntitiesBatch { handles } => A::GetEntitiesBatchResult(Ok(handles.iter().map(|_| {
                Some(EntityType::Line(Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0))))
            }).collect())),
            Q::AddLayer(_) => A::OptHandle(Some(Handle::from(1000u64))),
            Q::PushUndo { .. } | Q::SetDirty | Q::BumpGeometry => A::Ok,
            Q::AddEntities(entities) => {
                let mut all = self.drawn.lock().unwrap();
                let handles = (all.len()..all.len() + entities.len()).map(|i| Handle::from(i as u64 + 1)).collect();
                all.extend(entities);
                A::Handles(handles)
            }
            Q::WriteRecord { .. } => A::Bool(true),
            Q::PushInfo(_) => { self.done.send(true).unwrap(); A::Ok }
            Q::PushError(_) => { self.done.send(false).unwrap(); A::Ok }
            other => return Err(PluginRequestError(format!("onverwacht verzoek: {other:?}"))),
        })
    }
}
struct Sender(Arc<Drawing>);
impl PluginRequestSender for Sender {
    fn request(&self, req: Q) -> Result<A, PluginRequestError> { self.0.request(req) }
}
struct Host { tab: u64, path: PathBuf, drawing: Arc<Drawing>, selected: bool }
impl HostApi for Host {
    fn tab_index(&self) -> usize { self.tab as usize }
    fn document(&self) -> &Document { panic!("geen directe documenttoegang") }
    fn document_mut(&mut self) -> &mut Document { panic!("geen directe documenttoegang") }
    fn document_reader(&self) -> Box<dyn DocumentReader + '_> { panic!("geen directe documenttoegang") }
    fn add_entity(&mut self, _: EntityType) -> Handle { panic!("gebruik de workergrens") }
    fn bump_geometry(&mut self) { panic!("gebruik de workergrens") }
    fn read_record(&self, _: Handle, _: &str) -> Option<&ExtendedDataRecord> { None }
    fn write_record(&mut self, _: Handle, _: ExtendedDataRecord) -> bool { false }
    fn remove_record(&mut self, _: Handle, _: &str) -> bool { false }
    fn push_undo(&mut self, _: &str) { panic!("gebruik de workergrens") }
    fn set_dirty(&mut self) { panic!("gebruik de workergrens") }
    fn push_info(&mut self, _: &str) {}
    fn push_output(&mut self, _: &str) {}
    fn push_error(&mut self, msg: &str) { panic!("{msg}") }
    fn start_interactive(&mut self, _: Box<dyn InteractiveCommand>) { panic!("opdracht heeft al een lijn") }
    fn plugin_state_any(&self, _: &str) -> Option<&(dyn Any + Send + Sync)> { None }
    fn plugin_state_any_mut(&mut self, _: &str) -> Option<&mut (dyn Any + Send + Sync)> { None }
    fn ensure_plugin_state_any(&mut self, _: &'static str, _: &mut dyn FnMut() -> Box<dyn Any + Send + Sync>) -> Option<&mut (dyn Any + Send + Sync)> { None }
    fn plugin_request_sender(&self) -> Option<Box<dyn PluginRequestSender>> { Some(Box::new(Sender(self.drawing.clone()))) }
    fn document_path(&self, _: u64) -> Option<PathBuf> { Some(self.path.clone()) }
    fn selection(&self) -> Vec<Handle> { if self.selected { vec![Handle::from(1u64)] } else { Vec::new() } }
    fn read_xrecord(&self, _: &[&str], name: &str) -> Result<Vec<XRecordEntry>, String> {
        if name == "UNITS" { Ok(vec![XRecordEntry::string(1, "1")]) } else { Err("geen projectlocatie".into()) }
    }
}

#[test]
fn parking_and_swept_path_keep_their_drawing_and_reply_on_explicit_and_selected_lines() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("oi-library-workers-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let previous = std::env::var_os("APPDATA");
    std::env::set_var("APPDATA", &root);
    let vehicle = library::vehicles().iter().find(|v| v.category == "car" && v.units.len() == 1).unwrap();
    let cases = [
        ("parking-path", "GIS_PARKING LIJN=0,0;100,0".to_string(), false),
        ("parking-selection", "GIS_PARKING".to_string(), true),
        ("swept-path", format!("GIS_SWEPT_PATH {} PAD=0,0;100,0", vehicle.id), false),
        ("swept-selection", format!("GIS_SWEPT_PATH {}", vehicle.id), true),
    ];
    for (i, (name, command, selected)) in cases.iter().enumerate() {
        let (started, entered) = mpsc::sync_channel(0);
        let (resume, paused) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let first = Arc::new(Drawing { drawn: Mutex::new(Vec::new()), pause: Mutex::new(Some((started, paused))), pause_on_read: *selected, done: done.clone() });
        let second = Arc::new(Drawing { drawn: Mutex::new(Vec::new()), pause: Mutex::new(None), pause_on_read: false, done });
        let mut a = Host { tab: 120100 + i as u64 * 2, path: root.join("a.dwg"), drawing: first.clone(), selected: *selected };
        let mut b = Host { tab: a.tab + 1, path: root.join("b.dwg"), drawing: second.clone(), selected: false };
        bridge::attach(&mut a);
        let id = format!("{name}-{nonce}");
        bridge::expect_reply(Some(id.clone()));
        if command.starts_with("GIS_PARKING") { library::parking::command(&mut a, command); }
        else { library::place::swept_command(&mut a, command); }
        entered.recv_timeout(Duration::from_secs(5)).expect("worker bereikt de hostgrens");
        bridge::attach(&mut b);
        bridge::expect_reply(None); // A later user command in B owns no script reply.
        resume.send(()).unwrap();
        assert!(finished.recv_timeout(Duration::from_secs(5)).unwrap(), "{name} mislukt");
        assert!(!first.drawn.lock().unwrap().is_empty(), "{name}: A kreeg geen geometrie");
        assert!(second.drawn.lock().unwrap().is_empty(), "{name}: geometrie kwam in B");
        let result = open_infra::civiel::api::result_dir().join(format!("{id}.json"));
        for _ in 0..500 {
            if result.exists() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        let reply: serde_json::Value = serde_json::from_slice(&std::fs::read(&result).expect("oorspronkelijk antwoord-ID bewaard")).unwrap();
        assert_eq!(reply["ok"], true);
        assert!(reply["data"]["elementen"].as_u64().unwrap() > 0);
    }
    if let Some(value) = previous { std::env::set_var("APPDATA", value); } else { std::env::remove_var("APPDATA"); }
    std::fs::remove_dir_all(root).unwrap();
}
