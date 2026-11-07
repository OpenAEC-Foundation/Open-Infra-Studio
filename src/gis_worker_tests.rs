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

struct IdleHost;
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
    fn start_interactive(&mut self, _: Box<dyn InteractiveCommand>) { panic!("geen interactieve opdracht") }
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
        adopt_geodata(&mut IdleHost);
        let reply = bridge::take_reply_id();
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reply.as_deref(), Some("eerste"), "alleen de gevraagde opdracht mag haar antwoord-ID innemen");
    });
}
