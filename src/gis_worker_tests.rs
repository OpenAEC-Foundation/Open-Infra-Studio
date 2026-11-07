//! Antwoord-ID van de gevraagde opdracht blijft buiten automatische GEODATA-adoptie.
use super::*;
use std::any::Any;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use ocs_plugin_api::host::{
    codec::{Document, EntityType, Handle}, DocumentReader, ExtendedDataRecord,
    PluginRequestError, PluginRequestSender,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};

#[derive(Default)]
struct IdleHost { command: Option<Box<dyn InteractiveCommand>> }
impl HostApi for IdleHost {
    fn tab_index(&self) -> usize { 120003 }
    fn document(&self) -> &Document { panic!("geen directe documenttoegang") }
    fn document_mut(&mut self) -> &mut Document { panic!("geen directe documenttoegang") }
    fn document_reader(&self) -> Box<dyn DocumentReader + '_> { panic!("geen directe documenttoegang") }
    fn add_entity(&mut self, _: EntityType) -> Handle { panic!("geen geometrie in deze proef") }
    fn bump_geometry(&mut self) { panic!("geen geometrie in deze proef") }
    fn read_record(&self, _: Handle, _: &str) -> Option<&ExtendedDataRecord> { None }
    fn write_record(&mut self, _: Handle, _: ExtendedDataRecord) -> bool { false }
    fn remove_record(&mut self, _: Handle, _: &str) -> bool { false }
    fn push_undo(&mut self, _: &str) { panic!("geen geometrie in deze proef") }
    fn set_dirty(&mut self) { panic!("geen geometrie in deze proef") }
    fn push_info(&mut self, _: &str) {}
    fn push_output(&mut self, _: &str) {}
    fn push_error(&mut self, msg: &str) { panic!("{msg}") }
    fn start_interactive(&mut self, command: Box<dyn InteractiveCommand>) { self.command = Some(command); }
    fn plugin_state_any(&self, _: &str) -> Option<&(dyn Any + Send + Sync)> { None }
    fn plugin_state_any_mut(&mut self, _: &str) -> Option<&mut (dyn Any + Send + Sync)> { None }
    fn ensure_plugin_state_any(&mut self, _: &'static str, _: &mut dyn FnMut() -> Box<dyn Any + Send + Sync>) -> Option<&mut (dyn Any + Send + Sync)> { None }
}

#[test]
fn geodata_adoption_keeps_the_first_commands_reply_id() {
    let _one = bridge::COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    struct NoGeodata(mpsc::Sender<()>);
    impl PluginRequestSender for NoGeodata {
        fn request(&self, req: Q) -> Result<A, PluginRequestError> {
            match req {
                Q::OpenDocumentViewV4 { .. } => Err(PluginRequestError("geen GEODATA beschikbaar".into())),
                Q::CloseDocumentViewV4 { .. } => { self.0.send(()).unwrap(); Ok(A::Ok) }
                other => panic!("onverwacht verzoek: {other:?}"),
            }
        }
    }
    let (done, finished) = mpsc::channel();
    let target = bridge::Target { sender: Arc::new(NoGeodata(done)), tab_id: 120003, unit_factor: 1.0, doc_path: None };
    bridge::with_command_context(target, None, None, || {
        // This is dispatch's ordering before the first GIS_SITE worker starts.
        bridge::expect_reply(Some("eerste".into()));
        adopt_geodata(&mut IdleHost::default());
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let reply = bridge::take_reply_id();
    assert_eq!(reply.as_deref(), Some("eerste"), "alleen de gevraagde opdracht mag haar antwoord-ID innemen");
}

#[test]
fn an_area_started_in_a_keeps_its_site_after_callbacks_resume_with_b_active() {
    let _one = bridge::COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    struct Writer { sites: Arc<std::sync::Mutex<Vec<Site>>>, done: mpsc::Sender<()> }
    impl PluginRequestSender for Writer {
        fn request(&self, req: Q) -> Result<A, PluginRequestError> {
            Ok(match req {
                Q::UpdateXRecord { name, entries, .. } => {
                    assert_eq!(name, "SITE");
                    for entry in entries {
                        if let ocs_plugin_api::host::XRecordValue::String(value) = entry.value {
                            self.sites.lock().unwrap().push(serde_json::from_str(&value).unwrap());
                        }
                    }
                    A::XRecordUpdateResult(Ok(Handle::from(1u64)))
                }
                Q::PushInfo(_) => { self.done.send(()).unwrap(); A::Ok }
                other => panic!("onverwacht verzoek: {other:?}"),
            })
        }
    }
    let (done, finished) = mpsc::channel();
    let first = Arc::new(std::sync::Mutex::new(Vec::new()));
    let second = Arc::new(std::sync::Mutex::new(Vec::new()));
    let drawing = |tab_id, sites| bridge::Target { sender: Arc::new(Writer { sites, done: done.clone() }), tab_id, unit_factor: 1.0, doc_path: None };
    let mut site = Site::default();
    site.label = "Project A".into();
    let mut host = IdleHost::default();
    bridge::with_command_context(drawing(120010, first.clone()), Some(site.clone()), None, || {
        bridge::start_interactive(&mut host, Box::new(AreaCmd { first: None, site, unit_factor: 1.0 }));
    });
    let mut command = host.command.take().unwrap();
    assert!(matches!(command.on_point([0.0, 0.0, 0.0]), CommandStep::NeedPoint));
    bridge::with_command_context(drawing(120011, second.clone()), None, None, || {
        // The host resumes A's retained command without another dispatch.
        assert!(matches!(command.on_point([100.0, 50.0, 0.0]), CommandStep::Done));
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(bridge::target().unwrap().tab_id, 120011, "callback herstelt de context van B");
    });
    let written = first.lock().unwrap();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].label, "Project A");
    assert_eq!(written[0].width, 100.0);
    assert_eq!(written[0].height, 50.0);
    assert!(second.lock().unwrap().is_empty(), "A's gebied kwam in B");
}
