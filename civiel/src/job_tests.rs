//! Een puntenexport blijft bij zijn tekening wanneer het actieve tabblad wisselt.
use super::*;
use std::any::Any;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use ocs_plugin_api::host::{
    codec::{entities::Point, Document, EntityType, Handle},
    DocumentReader, ExtendedDataRecord, PluginRequestError, PluginRequestSender,
    XRecordEntry,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};

#[derive(Clone)]
struct Drawing {
    point: Point,
    pause: Arc<Mutex<Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>>>,
    done: mpsc::Sender<Result<String, String>>,
}

impl PluginRequestSender for Drawing {
    fn request(&self, req: Q) -> Result<A, PluginRequestError> {
        Ok(match req {
            Q::OpenDocumentViewV4 { .. } => return Err(PluginRequestError("geen gedeelde weergave".into())),
            Q::GetSelection => A::Handles(vec![Handle::from(1u64)]),
            Q::ReadXRecord { .. } => A::XRecordEntries(Ok(Vec::new())),
            Q::GetEntitiesBatch { handles } => {
                let pause = self.pause.lock().unwrap().take();
                if let Some((entered, resume)) = pause {
                    entered.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                A::GetEntitiesBatchResult(Ok(handles.iter().map(|h| {
                    (*h == Handle::from(1u64)).then(|| EntityType::Point(self.point.clone()))
                }).collect()))
            }
            Q::PushInfo(msg) => { self.done.send(Ok(msg)).unwrap(); A::Ok }
            Q::PushError(msg) => { self.done.send(Err(msg)).unwrap(); A::Ok }
            other => return Err(PluginRequestError(format!("onverwacht verzoek: {other:?}"))),
        })
    }
}

struct Host { tab: u64, path: PathBuf, drawing: Drawing }

impl HostApi for Host {
    fn tab_index(&self) -> usize { self.tab as usize }
    fn document(&self) -> &Document { panic!("geen directe documenttoegang") }
    fn document_mut(&mut self) -> &mut Document { panic!("geen directe documenttoegang") }
    fn document_reader(&self) -> Box<dyn DocumentReader + '_> { panic!("geen directe documenttoegang") }
    fn add_entity(&mut self, _: EntityType) -> Handle { panic!("export schrijft geen geometrie") }
    fn bump_geometry(&mut self) { panic!("export schrijft geen geometrie") }
    fn read_record(&self, _: Handle, _: &str) -> Option<&ExtendedDataRecord> { None }
    fn write_record(&mut self, _: Handle, _: ExtendedDataRecord) -> bool { false }
    fn remove_record(&mut self, _: Handle, _: &str) -> bool { false }
    fn push_undo(&mut self, _: &str) { panic!("export schrijft geen geometrie") }
    fn set_dirty(&mut self) { panic!("export schrijft geen geometrie") }
    fn push_info(&mut self, _: &str) {}
    fn push_output(&mut self, _: &str) {}
    fn push_error(&mut self, msg: &str) { panic!("{msg}") }
    fn start_interactive(&mut self, _: Box<dyn InteractiveCommand>) { panic!("geen interactieve opdracht") }
    fn plugin_state_any(&self, _: &str) -> Option<&(dyn Any + Send + Sync)> { None }
    fn plugin_state_any_mut(&mut self, _: &str) -> Option<&mut (dyn Any + Send + Sync)> { None }
    fn ensure_plugin_state_any(&mut self, _: &'static str, _: &mut dyn FnMut() -> Box<dyn Any + Send + Sync>) -> Option<&mut (dyn Any + Send + Sync)> { None }
    fn plugin_request_sender(&self) -> Option<Box<dyn PluginRequestSender>> { Some(Box::new(self.drawing.clone())) }
    fn document_path(&self, _: u64) -> Option<PathBuf> { Some(self.path.clone()) }
    fn read_xrecord(&self, _: &[&str], name: &str) -> Result<Vec<XRecordEntry>, String> {
        let value = if name == "SITE" {
            serde_json::to_string(&open_infra::gis::site::Site::default()).unwrap()
        } else { "1".into() };
        Ok(vec![XRecordEntry::string(1, value)])
    }
}

#[test]
fn points_export_keeps_its_folder_after_the_active_drawing_changes() {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("oi-job-{}-{nonce}", std::process::id()));
    let work = root.join("werkmap");
    let outside = root.join("buiten");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let (entered, started) = mpsc::sync_channel(0);
    let (resume, paused) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let mut point = Point::from_coords(1.0, 2.0, 3.5);
    point.common.handle = Handle::from(1u64);
    let a = Drawing { point: point.clone(), pause: Arc::new(Mutex::new(Some((entered, paused)))), done: done.clone() };
    let b = Drawing { point, pause: Arc::new(Mutex::new(None)), done };
    let mut first = Host { tab: 103001, path: work.join("a.dwg"), drawing: a };
    let mut second = Host { tab: 103002, path: outside.join("b.dwg"), drawing: b };
    bridge::attach(&mut first);
    let opt = civiel::CivielOptions::default();
    job(&mut first, "Punten exporteren", None, move |_| landmeten::export(&opt, None));
    started.recv_timeout(Duration::from_secs(5)).expect("export leest punten van A");
    bridge::attach(&mut second);
    resume.send(()).unwrap();
    let result = finished.recv_timeout(Duration::from_secs(5)).expect("export afgerond");
    assert!(result.is_ok(), "{result:?}");
    assert!(work.join("a_punten.csv").exists(), "export hoort bij A: {result:?}");
    assert!(!outside.join("b_punten.csv").exists(), "export mag niet naar B verhuizen");
    assert!(std::fs::read_to_string(work.join("a_punten.csv")).unwrap().contains("3.5"));
    assert_eq!(bridge::target().unwrap().tab_id, second.tab);
    std::fs::remove_dir_all(root).unwrap();
}
