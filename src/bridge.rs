//! Talking to Open CAD Studio from worker and window threads.
//!
//! `dispatch` (the host's call into the plugin) hands us a
//! `PluginRequestSender` for the active document tab; the workbench window and
//! import workers use it to add layers and entities while the host stays
//! responsive.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ocs_plugin_api::host::{
    codec::{Color, EntityType, Handle, LineWeight},
    CommandRequest, CommandStep, ExtendedDataRecord, HostApi, HostNotification, InteractiveCommand, LayerConfig, PluginRequestError, PluginRequestSender, PreviewWire, TableOperation, XDataValue, XRecordEntry, XRecordValue,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use ocs_plugin_api::host::codec::tables::normalize_name;

use crate::gis::site::{unit_factor_for_insunits, Site};

/// The document the workbench currently works on.
#[derive(Clone)]
pub struct Target {
    pub sender: Arc<dyn PluginRequestSender>,
    pub tab_id: u64,
    pub unit_factor: f64,
    pub doc_path: Option<PathBuf>,
}

struct DrawingSender {
    sender: Box<dyn PluginRequestSender>,
    closed: AtomicBool,
}

impl PluginRequestSender for DrawingSender {
    fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(PluginRequestError(crate::lang::tr(
                "de doeltekening is gesloten",
                "the target drawing is closed",
            ).into()));
        }
        self.sender.request(req)
    }
}

#[cfg(test)]
#[test]
fn closed_drawing_sender_refuses_and_does_not_forward_a_request() {
    struct NeverSend;
    impl PluginRequestSender for NeverSend {
        fn request(&self, _: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
            panic!("a request for a closed drawing reached CAD");
        }
    }

    let closed = AtomicBool::new(true);
    let sender = DrawingSender { sender: Box::new(NeverSend), closed };
    assert!(sender.request(PluginRequest::PushInfo("probe".into())).is_err());
}

struct CommandContext { target: Option<Target>, site: Option<Site>, reply: Option<String> }
thread_local! { static COMMAND_CONTEXT: std::cell::RefCell<Option<CommandContext>> = const { std::cell::RefCell::new(None) }; }
thread_local! { static IMPORT_UNDO_OPEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
thread_local! { static WRITES_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Bind a queued worker to its originating document and result ID, even if the active tab changes.
pub fn with_command_context<R>(target: Target, site: Option<Site>, reply: Option<String>, f: impl FnOnce() -> R) -> R {
    with_context(Some(target), site, reply, f)
}

/// Keep a frame or worker bound to its snapshot even when there was no drawing.
pub(crate) fn with_workbench_document<R>(target: Option<Target>, site: Option<Site>, f: impl FnOnce() -> R) -> R {
    with_context(target, site, None, f)
}

fn with_context<R>(target: Option<Target>, site: Option<Site>, reply: Option<String>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<CommandContext>);
    impl Drop for Restore { fn drop(&mut self) { COMMAND_CONTEXT.with(|c| *c.borrow_mut() = self.0.take()); } }
    let old = COMMAND_CONTEXT.with(|c| c.replace(Some(CommandContext { target, site, reply })));
    let _restore = Restore(old);
    f()
}

/// Start command work with the originating drawing, site and script reply.
/// A worker without an originating drawing must not attach to a later one.
pub fn spawn_command(f: impl FnOnce() + Send + 'static) {
    let Some(target) = target() else { return };
    let site = site();
    let reply = take_reply_id();
    std::thread::spawn(move || with_command_context(target, site, reply, f));
}

pub fn take_reply_id() -> Option<String> {
    let bound = COMMAND_CONTEXT.with(|c| c.borrow_mut().as_mut().map(|v| v.reply.take()));
    bound.unwrap_or_else(|| pending_reply().lock().unwrap().take())
}

/// The host keeps interactive commands per tab without dispatching again
/// on their later clicks. Keep that command's context until its final worker.
pub fn start_interactive(host: &mut dyn HostApi, command: Box<dyn InteractiveCommand>) {
    let Some(target) = target() else { return };
    let context = CommandContext { target: Some(target), site: site(), reply: take_reply_id() };
    host.start_interactive(Box::new(BoundInteractive { command, context }));
}

struct BoundInteractive {
    command: Box<dyn InteractiveCommand>,
    context: CommandContext,
}

impl BoundInteractive {
    fn call<R>(&mut self, f: impl FnOnce(&mut dyn InteractiveCommand) -> R) -> R {
        with_context(self.context.target.clone(), self.context.site.clone(), self.context.reply.take(), || {
            let out = f(self.command.as_mut());
            COMMAND_CONTEXT.with(|c| {
                let mut c = c.borrow_mut();
                let active = c.as_mut().unwrap();
                self.context.site = active.site.clone();
                self.context.reply = active.reply.take();
            });
            out
        })
    }
}

impl InteractiveCommand for BoundInteractive {
    fn prompt(&self) -> String { self.command.prompt() }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep { self.call(|c| c.on_point(pt)) }
    fn on_enter(&mut self) -> CommandStep { self.call(|c| c.on_enter()) }
    fn needs_object_pick(&self) -> bool { self.command.needs_object_pick() }
    fn on_object_pick(&mut self, h: Handle, pt: [f64; 3]) -> CommandStep { self.call(|c| c.on_object_pick(h, pt)) }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> { self.call(|c| c.on_cursor_move(pt)) }
    fn is_selection_gathering(&self) -> bool { self.command.is_selection_gathering() }
    fn accepts_drag_selection(&self) -> bool { self.command.accepts_drag_selection() }
    fn on_selection_complete(&mut self, handles: Vec<Handle>) -> CommandStep { self.call(|c| c.on_selection_complete(handles)) }
    fn selection_rejects_locked_layers(&self) -> bool { self.command.selection_rejects_locked_layers() }
    fn modal_dialog(&self) -> Option<ocs_plugin_api::dialog::DialogDef> { self.command.modal_dialog() }
    fn on_dialog_response(&mut self, response: ocs_plugin_api::dialog::DialogResponse) -> CommandStep { self.call(|c| c.on_dialog_response(response)) }
    fn take_pending_panel_updates(&mut self) -> Vec<(String, std::collections::HashMap<String, ocs_plugin_api::dialog::DialogValue>)> {
        self.call(|c| c.take_pending_panel_updates())
    }
}

#[derive(Default)]
struct Shared {
    target: Option<Target>,
    site: Option<Site>,
    drawing_senders: HashMap<u64, Arc<DrawingSender>>,
    /// The NLCS layers imports drew on, per tab, for the legend.
    nlcs_layers: Vec<(u64, crate::nlcs::LayerSpec)>,
    /// Data sources of the imports, per tab, for the title block.
    sources: Vec<(u64, String)>,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(Default::default)
}

const SITE_DICT: &[&str] = &["OPENINFRA"];
const SITE_RECORD: &str = "SITE";

/// Called from `dispatch`: remember the active tab and read its site.
pub fn attach(host: &mut dyn HostApi) {
    let Some(sender) = host.plugin_request_sender() else { return };
    let tab_id = host.tab_id();
    let stored = host
        .read_xrecord(SITE_DICT, SITE_RECORD)
        .ok()
        .and_then(|entries| {
            entries.into_iter().find_map(|e| match e.value {
                XRecordValue::String(s) if e.code == 1 => serde_json::from_str::<Site>(&s).ok(),
                _ => None,
            })
        });
    let doc_path = host.document_path(tab_id);
    // The V4 runner can wake for dispatch with earlier notifications still
    // queued separately. Apply drawing changes before registering this
    // sender, since DocumentOpened can replace the contents of the same tab.
    while let Some((_, notification)) = host.try_recv_notification() {
        handle_document_notification(notification);
    }
    {
        let mut s = shared().lock().unwrap();
        let same_tab = s.target.as_ref().is_some_and(|t| t.tab_id == tab_id);
        let unit_factor = s.target.as_ref().filter(|_| same_tab).map(|t| t.unit_factor).unwrap_or(1.0);
        // Requests from this runner to one CAD tab use the same connection.
        // Keep the sender's identity stable until the drawing is closed.
        let sender = s.drawing_senders.entry(tab_id).or_insert_with(|| {
            Arc::new(DrawingSender { sender, closed: AtomicBool::new(false) })
        }).clone();
        s.target = Some(Target { sender, tab_id, unit_factor, doc_path });
        match stored {
            Some(site) => {
                site.register_crs();
                s.site = Some(site);
            }
            // A new drawing without a site keeps nothing from another drawing.
            None if !same_tab => s.site = None,
            None => {}
        }
    }
    // Unit discovery can write the UNITS record. Refuse an outside target
    // before that write; dispatch reports the same refusal to its caller.
    if require_work_folder().is_err() { return; }
    // With the target in place the units can be asked for without the drawing.
    let f = units(host, tab_id, false);
    if let Some(t) = shared().lock().unwrap().target.as_mut() {
        t.unit_factor = f;
    }
}

/// Handle drawing lifecycle notifications for both the GIS and Civiel runners.
pub fn handle_document_notification(notification: HostNotification) {
    if let HostNotification::DocumentTabClosed { tab_id } | HostNotification::DocumentOpened { tab_id } = notification {
        drawing_closed(tab_id);
    }
}

/// Invalidate the previous drawing's senders and clear the workbench's
/// active target if it closed or was replaced in its tab.
pub fn drawing_closed(tab_id: u64) {
    let mut state = shared().lock().unwrap();
    if let Some(sender) = state.drawing_senders.remove(&tab_id) {
        sender.closed.store(true, Ordering::Release);
    }
    if state.target.as_ref().is_some_and(|target| target.tab_id == tab_id) {
        state.target = None;
        state.site = None;
    }
    drop(state);
    crate::ui::send(crate::ui::UiMsg::DrawingClosed(tab_id));
}

/// Capture a bound command's drawing or read the current target and saved
/// location together when there is no command context.
pub(crate) fn workbench_document() -> (Option<Target>, Option<Site>) {
    if let Some(document) = COMMAND_CONTEXT.with(|ctx| {
        ctx.borrow().as_ref().map(|ctx| (ctx.target.clone(), ctx.site.clone()))
    }) {
        return document;
    }
    let state = shared().lock().unwrap();
    (state.target.clone(), state.site.clone())
}

pub(crate) fn workbench_target_is_open(target: &Target) -> bool {
    shared().lock().unwrap().drawing_senders.get(&target.tab_id).is_some_and(|sender| {
        std::ptr::addr_eq(Arc::as_ptr(sender), Arc::as_ptr(&target.sender))
    })
}

#[cfg(test)]
pub(crate) fn workbench_test_target(tab_id: u64, sender: Box<dyn PluginRequestSender>) -> Target {
    let sender = Arc::new(DrawingSender { sender, closed: AtomicBool::new(false) });
    shared().lock().unwrap().drawing_senders.insert(tab_id, sender.clone());
    Target { sender, tab_id, unit_factor: 1.0, doc_path: None }
}

/// Drawing units per metre for the tab. The current host exposes INSUNITS
/// without a drawing snapshot. Only when it is unavailable do we fall back
/// to the tab cache, OPENINFRA/UNITS, or a small drawing's header.
/// `fresh` bypasses the cached/recorded value before trying the header.
pub fn units(host: &mut dyn HostApi, tab_id: u64, fresh: bool) -> f64 {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<u64, f64>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let f = host.system_variable("INSUNITS").and_then(unit_factor_from_variable);
    if f.is_none() && !fresh {
        if let Some(f) = cache.lock().unwrap().get(&tab_id) {
            return *f;
        }
    }
    let recorded = || -> Option<f64> {
        host.read_xrecord(SITE_DICT, UNITS_RECORD).ok()?.into_iter().find_map(|e| match e.value {
            XRecordValue::String(s) => s.trim().parse::<f64>().ok().filter(|f| *f > 0.0),
            _ => None,
        })
    };
    let f = match f.or_else(|| if fresh { None } else { recorded() }) {
        Some(f) => f,
        // The header, but only from a drawing small enough to send whole.
        None => match view_entities(|_| false) {
            Some((_, _, size)) if size < SNAPSHOT_SAFE => unit_factor_for_insunits(host.document().header.insertion_units as i64),
            _ => recorded().unwrap_or(1.0),
        },
    };
    cache.lock().unwrap().insert(tab_id, f);
    if recorded().is_none_or(|r| (r - f).abs() > 1e-12) {
        record_units(f);
    }
    f
}

fn unit_factor_from_variable(value: ocs_plugin_api::host::HostSettingValue) -> Option<f64> {
    use ocs_plugin_api::host::HostSettingValue;
    match value {
        HostSettingValue::Number(n) if n.is_finite() => Some(unit_factor_for_insunits(n as i64)),
        HostSettingValue::Text(t) => t.trim().parse().ok().map(unit_factor_for_insunits),
        _ => None,
    }
}

const UNITS_RECORD: &str = "UNITS";

/// Keep the unit factor in the drawing, for when it is too large to read.
fn record_units(f: f64) {
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let entries = vec![XRecordEntry::string(1, format!("{f}"))];
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: UNITS_RECORD.into(), entries: entries.clone() };
    if !matches!(request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        let _ = request(PluginRequest::CreateXRecord { dictionary_path: path, name: UNITS_RECORD.into(), entries });
    }
}

pub fn target() -> Option<Target> {
    if let Some(t) = COMMAND_CONTEXT.with(|c| c.borrow().as_ref().map(|v| v.target.clone())) { return t; }
    shared().lock().unwrap().target.clone()
}

pub fn site() -> Option<Site> {
    if let Some(s) = COMMAND_CONTEXT.with(|c| c.borrow().as_ref().map(|v| v.site.clone())) { return s; }
    shared().lock().unwrap().site.clone()
}

/// Check the path captured from the host at dispatch, before any worker starts.
/// A native Save As can change that path without changing the tab number.
pub fn require_work_folder() -> Result<(), String> {
    if std::env::var("OPENAEC_BEHEERD").as_deref() != Ok("1") { return Ok(()) }
    let root = std::env::var_os("OPENAEC_MAP").map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_dir())
        .and_then(|p| p.canonicalize().ok())
        .ok_or("beheerde modus zonder bestaande absolute werkmap")?;
    let drawing = target().and_then(|t| t.doc_path).ok_or("sla de tekening eerst op in de werkmap")?;
    let path = drawing.canonicalize().map_err(|_| "de tekening staat niet in de werkmap")?;
    let parent = drawing.parent().and_then(|p| p.canonicalize().ok()).ok_or("de tekening staat niet in de werkmap")?;
    if !path.starts_with(&root) || !parent.starts_with(&root) {
        return Err("de tekening staat niet in de werkmap".into());
    }
    Ok(())
}

/// Store the site in memory and in the drawing: the NOD XRecord (leading,
/// round-trips DWG) and the drawing's GEODATA object, in one undo step, so
/// other CAD programs open the saved drawing georeferenced. GEODATA is made
/// anew each time: the host keeps an existing object's unit scale, which a
/// change of INSUNITS would make wrong, and a refused CRS must not leave the
/// previous location behind. A refusal keeps the stored site and says so once.
pub fn save_site(site: &Site) -> Result<(), String> {
    let t = target().ok_or("geen tekening actief")?;
    let tab = t.tab_id;
    let mut written = site_writes().lock().unwrap();
    let sender = t.sender.clone();
    let label = crate::lang::tr("Projectlocatie", "Project location").to_string();
    let grouped = matches!(sender.request(PluginRequest::BeginUndoGroup { label }), Ok(PluginResponse::Ok));
    let stored = store_site(site, t.clone());
    let geodata = stored.is_ok().then(|| {
        let ask = |req: PluginRequest| traced(trace_on().then(|| kind(&req)), || sender.request(req)).map_err(|e| e.to_string());
        // Window and retained command targets can predate an INSUNITS change.
        // Ask this drawing's sender rather than reusing their cached factor.
        let f = match ask(PluginRequest::GetSystemVariable { name: "INSUNITS".into() })? {
            PluginResponse::SystemVariable(value) => value.and_then(unit_factor_from_variable).ok_or("INSUNITS niet beschikbaar")?,
            PluginResponse::Error(e) => return Err(e),
            other => return Err(format!("onverwacht antwoord: {other:?}")),
        };
        COMMAND_CONTEXT.with(|c| {
            if let Some(target) = c.borrow_mut().as_mut()
                .and_then(|ctx| ctx.target.as_mut()).filter(|target| target.tab_id == tab)
            {
                target.unit_factor = f;
            }
        });
        if let Some(current) = shared().lock().unwrap().target.as_mut().filter(|t| t.tab_id == tab) { current.unit_factor = f; }
        match ask(PluginRequest::RemoveGeoLocation)? {
            PluginResponse::GeoLocationRemoved(r) => r.map(|_| ()),
            PluginResponse::Error(e) => Err(e),
            other => Err(format!("onverwacht antwoord: {other:?}")),
        }?;
        match ask(PluginRequest::SetGeoLocation { location: crate::gis::geodata::location_for(site, f) })? {
            PluginResponse::GeoLocationSet(r) => r.map(|_| ()),
            PluginResponse::Error(e) => Err(e),
            other => Err(format!("onverwacht antwoord: {other:?}")),
        }
    });
    if grouped {
        let _ = sender.request(PluginRequest::EndUndoGroup);
    }
    stored?;
    written.insert(tab);
    if let Some(Err(e)) = geodata {
        crate::civiel::api::note("geodata_geweigerd", e.clone());
        let _ = sender.request(PluginRequest::PushError(format!(
            "[Open Infra] {} {}: {e}. {}",
            crate::lang::tr("GEODATA niet gezet voor", "GEODATA not set for"),
            site.crs,
            crate::lang::tr(
                "De projectlocatie is opgeslagen. GEODATA kon niet worden bijgewerkt; andere CAD-programma's kunnen daardoor een oude of geen locatie lezen. Kies bij een onbekend stelsel een EPSG-code.",
                "The project location is stored. GEODATA could not be updated; other CAD programs may read an old location or none. For an unknown coordinate system, choose an EPSG code."
            )
        )));
    }
    Ok(())
}

fn site_writes() -> &'static Mutex<HashSet<u64>> {
    static WRITTEN: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    WRITTEN.get_or_init(Default::default)
}

fn store_site(site: &Site, t: Target) -> Result<(), String> {
    let json = serde_json::to_string(site).map_err(|e| e.to_string())?;
    let entries = vec![XRecordEntry::string(1, json)];
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: SITE_RECORD.into(), entries: entries.clone() };
    if !matches!(traced(trace_on().then(|| "UpdateXRecord SITE".into()), || t.sender.request(update)), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        // Not there yet: create it (and the OPENINFRA dictionary).
        match traced(trace_on().then(|| "CreateXRecord SITE".into()), || t.sender.request(PluginRequest::CreateXRecord { dictionary_path: path, name: SITE_RECORD.into(), entries })) {
            Ok(PluginResponse::XRecordCreateResult(Ok(_))) => Ok(()),
            Ok(PluginResponse::XRecordCreateResult(Err(e))) | Ok(PluginResponse::Error(e)) => Err(e),
            Ok(other) => Err(format!("onverwacht antwoord: {other:?}")),
            Err(e) => Err(e.to_string()),
        }?;
    }
    COMMAND_CONTEXT.with(|c| {
        if let Some(c) = c.borrow_mut().as_mut() { c.site = Some(site.clone()); }
    });
    let mut s = shared().lock().unwrap();
    if s.target.as_ref().is_some_and(|current| current.tab_id == t.tab_id) {
        s.site = Some(site.clone());
    }
    Ok(())
}

/// Store a location found in GEODATA. The caller has no explicit command reply.
pub fn adopt_site(site: &Site) -> Result<bool, String> {
    let t = target().ok_or("geen tekening actief")?;
    let tab = t.tab_id;
    // Check and store under the same lock as explicit saves. Adoption only
    // initializes a tab; it may never replace a location already saved here.
    let mut written = site_writes().lock().unwrap();
    if written.contains(&tab) { return Ok(false); }
    store_site(site, t)?;
    written.insert(tab);
    Ok(true)
}

/// Request tracing for diagnosis: on with `OI_BRIDGE_TRACE` or a file
/// `<config>/OpenCADStudio/open-infra/bridge-trace`; every host request is
/// written before and after, with its thread, to `bridge-trace.log` beside it.
fn trace_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("OI_BRIDGE_TRACE").is_some() || crate::gis::servers::config_dir().join("bridge-trace").exists();
        if on {
            crate::ui::allow_debugger();
        }
        on
    })
}

fn trace_line(line: &str) {
    use std::io::Write;
    static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = crate::gis::servers::config_dir().join("bridge-trace.log");
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(Mutex::new)
    });
    if let Some(f) = file {
        let _ = writeln!(f.lock().unwrap_or_else(|e| e.into_inner()), "{line}");
    }
}

fn traced<R>(what: Option<String>, f: impl FnOnce() -> R) -> R {
    let Some(what) = what else { return f() };
    let th = std::thread::current();
    let name = th.name().unwrap_or("?").to_string();
    let t0 = std::time::Instant::now();
    trace_line(&format!("{:?} {name} → {what}", std::time::SystemTime::now()));
    let r = f();
    trace_line(&format!("{:?} {name} ← {what} ({} ms)", std::time::SystemTime::now(), t0.elapsed().as_millis()));
    r
}

/// The variant name of a request (its Debug output up to the first field).
fn kind(req: &PluginRequest) -> String {
    let d = format!("{req:?}");
    d[..d.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(d.len())].to_string()
}

pub(crate) fn request(req: PluginRequest) -> Result<PluginResponse, String> {
    let t = target().ok_or("geen tekening actief — klik eerst in Open CAD Studio op het GIS-tabblad")?;
    match traced(trace_on().then(|| kind(&req)), || t.sender.request(req)) {
        Ok(PluginResponse::Error(e)) => Err(e),
        Ok(r) => Ok(r),
        Err(e) => Err(e.to_string()),
    }
}

pub fn info(msg: &str) {
    push_info(msg);
    reply(true, msg);
}

/// A line on the host's command line, without answering a waiting script.
pub(crate) fn push_info(msg: &str) {
    let _ = request(PluginRequest::PushInfo(format!("[Open Infra] {msg}")));
}

pub fn error(msg: &str) {
    let _ = request(PluginRequest::PushError(format!("[Open Infra] {msg}")));
    reply(false, msg);
}

fn pending_reply() -> &'static Mutex<Option<String>> {
    static R: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

/// A script's command carried `ID=<name>`: the command's final message (the
/// next `info` or `error`) is also written to `<config>/api/<name>.json`.
/// One command at a time, as a script runs them.
pub fn expect_reply(id: Option<String>) {
    *pending_reply().lock().unwrap() = id;
}

/// A script waits for this command's answer (it carried `ID=`).
pub fn scripted() -> bool {
    COMMAND_CONTEXT.with(|c| c.borrow().as_ref().is_some_and(|v| v.reply.is_some())) || pending_reply().lock().unwrap().is_some()
}

/// Answer the waiting script now (argument errors, commands that finish at once).
pub fn reply(ok: bool, msg: &str) {
    let bound = COMMAND_CONTEXT.with(|c| c.borrow_mut().as_mut().map(|v| v.reply.take()));
    if let Some(id) = bound.unwrap_or_else(|| pending_reply().lock().unwrap().take()) {
        crate::civiel::api::write(&id, ok, msg, crate::civiel::api::take());
    }
}

/// `ID=<name>` out of a command line: the command without it, and the name.
pub fn split_id(cmd: &str) -> (String, Option<String>) {
    let mut id = None;
    let rest: Vec<&str> = cmd
        .split_whitespace()
        .filter(|w| match w.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("ID") => {
                id = crate::civiel::api::Args::parse(&format!("ID={v}")).id();
                false
            }
            _ => true,
        })
        .collect();
    (rest.join(" "), id)
}

pub fn ensure_layer(name: &str, color: Color) {
    adopt_dashed_layer(name);
    let _ = request(PluginRequest::AddLayer(LayerConfig { name: name.into(), color: Some(color), ..Default::default() }));
}

/// Until #160 `emit::layer_name` turned a comma into '-'. Rename the layer of
/// such an earlier import, with its entities, so a re-import does not leave
/// a second layer. Without that layer, or with the new one already there,
/// the host refuses and nothing changes.
fn adopt_dashed_layer(name: &str) {
    if name.contains(',') {
        let operation = TableOperation::LayerRename { from: name.replace(',', "-"), to: name.into() };
        let _ = request(PluginRequest::TableOperation { operation });
    }
}

/// Application name of the object data (XDATA) on imported entities.
pub const XDATA_APP: &str = "OPENINFRA";

/// Create an NLCS layer with its colour, lineweight and linetype. The NLCS
/// linetype is defined in paper millimetres when the drawing lacks it (shape
/// and text elements left out); an undefinable one leaves the layer
/// Continuous.
pub fn ensure_nlcs_layer(spec: &crate::nlcs::LayerSpec) {
    let Some(tab) = target().map(|t| t.tab_id) else { return };
    {
        let mut s = shared().lock().unwrap();
        if !s.nlcs_layers.iter().any(|(t, layer)| *t == tab && layer.name == spec.name) {
            s.nlcs_layers.push((tab, spec.clone()));
        }
    }
    // Undo can remove a layer or linetype while the plugin stays loaded.
    let linetype = ensure_linetype(&spec.linetype);
    let config = |linetype: Option<String>| LayerConfig {
        name: spec.name.clone(),
        color: Some(nlcs_color(spec)),
        linetype,
        lineweight: spec.lineweight.map(LineWeight::from_value),
        description: Some(format!("NLCS object {}", spec.object_id)),
        ..Default::default()
    };
    let made = matches!(request(PluginRequest::AddLayer(config(linetype.clone()))), Ok(PluginResponse::OptHandle(Some(_))));
    if !made && linetype.is_some() {
        let _ = request(PluginRequest::AddLayer(config(None)));
    }
}

/// The colour of an NLCS layer: its true colour, else its NLCS colour.
pub fn nlcs_color(spec: &crate::nlcs::LayerSpec) -> Color {
    match spec.true_color {
        Some((r, g, b)) => Color::Rgb { r, g, b },
        None => Color::Index(spec.color.max(1)),
    }
}

/// Define an NLCS linetype in the drawing when it lacks it (shape and text
/// elements left out). `None` for CONTINUOUS and names that cannot be made.
fn ensure_linetype(name: &str) -> Option<String> {
    let pattern = crate::nlcs::linetype_pattern(name)?;
    let description = if crate::nlcs::linetype_is_complex(name) { format!("NLCS {name} (vereenvoudigd, zonder symbolen)") } else { format!("NLCS {name}") };
    let op = TableOperation::LinetypeCreate { name: name.to_string(), description, pattern };
    match request(PluginRequest::TableOperation { operation: op }) {
        Ok(PluginResponse::TableResult(Ok(_))) => Some(name.to_string()),
        // Already in the drawing (an NLCS template, an earlier import).
        Ok(PluginResponse::TableResult(Err(e))) if e.contains("already exists") => Some(name.to_string()),
        _ => None,
    }
}

/// Below this a full snapshot is safe: it travels in one message, and the
/// host's messages may not exceed 64 MB.
const SNAPSHOT_SAFE: usize = 40 * 1024 * 1024;

static PARTIAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The host's shared-memory view of the drawing: the entities and the size
/// of their encoding, or `None` when the host cannot share it (a drawing
/// larger than its view segment, 16 MB unless Open CAD Studio is started
/// with OCS_V4_SNAPSHOT_SEGMENT_SIZE). The entities and layer names are
/// read only when `decode` says so for that size: a caller that needs the
/// size alone skips the work.
fn view_entities(decode: impl FnOnce(usize) -> bool) -> Option<(Vec<EntityType>, Vec<String>, usize)> {
    use ocs_plugin_api::shm::{DocumentViewDataV4, SharedDocumentReader};
    // A refused view costs the host a full serialisation: after a refusal,
    // do not ask again for a while (drawings rarely shrink).
    static REFUSED: Mutex<Option<(u64, std::time::Instant)>> = Mutex::new(None);
    let t = target()?;
    if let Some((tab, at)) = *REFUSED.lock().unwrap() {
        if tab == t.tab_id && at.elapsed() < std::time::Duration::from_secs(600) {
            return None;
        }
    }
    let Ok(PluginResponse::DocumentViewV4 { path, .. }) = request(PluginRequest::OpenDocumentViewV4 { tab_id: t.tab_id }) else {
        *REFUSED.lock().unwrap() = Some((t.tab_id, std::time::Instant::now()));
        // The store was made before publishing failed: close it, or the
        // host keeps trying on every change.
        let _ = request(PluginRequest::CloseDocumentViewV4 { tab_id: t.tab_id });
        return None;
    };
    let read = || -> Option<(Vec<EntityType>, Vec<String>, usize)> {
        let reader = SharedDocumentReader::<DocumentViewDataV4>::open(std::path::Path::new(&path)).ok()?;
        let p = reader.payload()?;
        let size: usize = p.entities.iter().map(|e| e.data.len()).sum();
        if !decode(size) {
            return Some((Vec::new(), Vec::new(), size));
        }
        let layers = p.layers.iter().map(|l| l.name.as_str().to_string()).collect();
        let entities = p.entities.iter().filter_map(|e| bincode::deserialize::<EntityType>(e.data.as_slice()).ok()).collect();
        Some((entities, layers, size))
    };
    let out = read();
    // Closed again: an open view is republished on every change.
    let _ = request(PluginRequest::CloseDocumentViewV4 { tab_id: t.tab_id });
    out
}

/// A document of these entities (their own handles, in model space) and layers.
fn document_of(entities: Vec<EntityType>, layers: &[String]) -> ocs_plugin_api::host::codec::CadDocument {
    use ocs_plugin_api::host::codec::{tables::Layer, CadDocument};
    let mut doc = CadDocument::new();
    for l in layers {
        let _ = doc.layers.add(Layer::new(l.as_str()));
    }
    for mut e in entities {
        e.common_mut().owner_handle = Handle::NULL;
        if !doc.layers.iter().any(|l| l.name == e.common().layer) {
            let _ = doc.layers.add(Layer::new(e.common().layer.as_str()));
        }
        let _ = doc.add_entity(e);
    }
    doc
}

/// The whole drawing, if the host can share it (any size up to its view
/// segment); layers carry names only.
pub fn view() -> Option<ocs_plugin_api::host::codec::CadDocument> {
    let (entities, layers, _) = view_entities(|_| true)?;
    Some(document_of(entities, &layers))
}

/// How long a full snapshot may take. OCS sends no answer at all to one
/// that does not fit in a message (64 MB), and the drawing's tables and
/// records do not count in the size of the shared view: without a limit,
/// such a request would wait forever.
const SNAPSHOT_WAIT: std::time::Duration = std::time::Duration::from_secs(if cfg!(test) { 1 } else { 60 });

/// The full snapshot of the active tab, or an error when the host does not
/// send it within [`SNAPSHOT_WAIT`]. Ask it only for a drawing whose shared
/// view is below [`SNAPSHOT_SAFE`].
fn full_snapshot() -> Result<Box<ocs_plugin_api::host::Document>, String> {
    let t = target().ok_or("geen tekening actief — klik eerst in Open CAD Studio op het GIS-tabblad")?;
    let (tx, rx) = std::sync::mpsc::channel();
    // A request that never gets its answer keeps this thread, not the command.
    std::thread::spawn(move || {
        let _ = tx.send(with_command_context(t, None, None, || request(PluginRequest::DocumentSnapshot)));
    });
    match rx.recv_timeout(SNAPSHOT_WAIT) {
        Ok(Ok(PluginResponse::Document(doc))) => Ok(doc),
        Ok(Ok(other)) => Err(format!("onverwacht antwoord op de tekeningopvraag: {other:?}")),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(crate::lang::tr(
            "Open CAD Studio stuurde de tekening niet: ze is waarschijnlijk te groot voor één bericht (64 MB)",
            "Open CAD Studio did not send the drawing: it is probably too large for one message (64 MB)",
        )
        .into()),
    }
}

/// The drawing's layer table, from a full snapshot: `None` for a drawing
/// too large for that. The shared view is only measured, not decoded.
pub fn layer_table() -> Option<ocs_plugin_api::host::codec::tables::Table<ocs_plugin_api::host::codec::tables::Layer>> {
    view_entities(|_| false).filter(|(_, _, size)| *size < SNAPSHOT_SAFE)?;
    let doc: ocs_plugin_api::host::codec::CadDocument = (*full_snapshot().ok()?).into();
    Some(doc.layers)
}

/// The whole drawing of the active tab, read from a worker or window thread.
/// A small drawing comes as a full snapshot (with the layer table); a large
/// one from the shared view, whose layers have names only (see [`partial`]).
/// A drawing too large for both gives an error, never a stalled request.
pub fn snapshot() -> Result<ocs_plugin_api::host::codec::CadDocument, String> {
    match view_entities(|size| size >= SNAPSHOT_SAFE) {
        Some((_, _, size)) if size < SNAPSHOT_SAFE => {
            PARTIAL.store(false, std::sync::atomic::Ordering::Relaxed);
            full_snapshot().map(|doc| (*doc).into())
        }
        Some((entities, layers, _)) => {
            PARTIAL.store(true, std::sync::atomic::Ordering::Relaxed);
            Ok(document_of(entities, &layers))
        }
        None => Err("de tekening is te groot om in zijn geheel te lezen (Open CAD Studio deelt standaard tot 16 MB). \
             Start Open CAD Studio met OCS_V4_SNAPSHOT_SEGMENT_SIZE=268435456 voor tekeningen tot 256 MB"
            .into()),
    }
}

/// Whether the last snapshot came from the shared view (a large drawing):
/// its layers have names only, no colours, lineweights or linetypes.
pub fn partial() -> bool {
    PARTIAL.load(std::sync::atomic::Ordering::Relaxed)
}

/// A document of just these entities, fetched by handle (`per_request` at a
/// time: few for meshes, many for lines), for drawings of any size.
pub fn partial_document(handles: &[Handle], per_request: usize) -> Result<ocs_plugin_api::host::codec::CadDocument, String> {
    let mut all = Vec::new();
    for chunk in handles.chunks(per_request.max(1)) {
        all.extend(entities(chunk)?.into_iter().flatten());
    }
    Ok(document_of(all, &[]))
}

/// The entities matching a host query (the JSON of the control API's
/// `query` operation, API v14), all pages, for drawings of any size.
pub fn query(mut query: serde_json::Value) -> Result<Vec<EntityType>, String> {
    let mut out = Vec::new();
    let mut offset = 0u64;
    loop {
        // Pages small enough for one host message (64 MiB).
        query["offset"] = offset.into();
        query["limit"] = 500.into();
        match request(PluginRequest::QueryEntities { query: query.to_string() })? {
            PluginResponse::EntityPage(Ok(page)) => {
                out.extend(page.entities);
                match page.next_offset {
                    Some(next) if next > offset => offset = next,
                    _ => return Ok(out),
                }
            }
            PluginResponse::EntityPage(Err(e)) => return Err(e),
            other => return Err(format!("onverwacht antwoord op de opvraag: {other:?}")),
        }
    }
}

// ── Layout of generated views ───────────────────────────────────────────────

/// A free place for a view of `size` (width, height): "onder" stacks views
/// below `area`, "rechts" puts them side by side to the right of it. The
/// cursor is kept in the drawing (NOD OPENINFRA/LAYOUT), so views of later
/// commands never overlap. Returns the lower-left corner ("onder") or the
/// upper-left corner ("rechts").
pub fn layout_spot(side: &str, size: [f64; 2], area: [f64; 4], gap: f64) -> [f64; 2] {
    let _one = index_lock().lock().unwrap_or_else(|e| e.into_inner());
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let mut cursors: Vec<(String, f64, f64)> = match request(PluginRequest::ReadXRecord { dictionary_path: path.clone(), name: "LAYOUT".into() }) {
        Ok(PluginResponse::XRecordEntries(Ok(entries))) => entries
            .into_iter()
            .filter_map(|e| match e.value {
                XRecordValue::String(s) => {
                    let mut p = s.split(' ');
                    Some((p.next()?.to_string(), p.next()?.parse().ok()?, p.next()?.parse().ok()?))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let (x, y) = match cursors.iter().find(|c| c.0 == side) {
        Some(c) => (c.1, c.2),
        None if side == "rechts" => (area[2] + gap, area[3]),
        None => (area[0], area[1] - gap),
    };
    let (spot, next) = if side == "rechts" {
        ([x, y], (x + size[0] + gap, y))
    } else {
        ([x, y - size[1]], (x, y - size[1] - gap))
    };
    cursors.retain(|c| c.0 != side);
    cursors.push((side.to_string(), next.0, next.1));
    let entries: Vec<XRecordEntry> = cursors.iter().map(|(k, a, b)| XRecordEntry::string(1, format!("{k} {a} {b}"))).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: "LAYOUT".into(), entries: entries.clone() };
    if !matches!(request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        let _ = request(PluginRequest::CreateXRecord { dictionary_path: path, name: "LAYOUT".into(), entries });
    }
    spot
}

// ── Object index ────────────────────────────────────────────────────────────

const INDEX_RECORD: &str = "INDEX";

fn index_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// The Open Infra objects of the drawing by kind and key (axis name,
/// terrain status, …), kept in the drawing (NOD OPENINFRA/INDEX), so the
/// civil commands fetch what they need by handle and never depend on the
/// whole drawing, which may be too large to send.
/// A JSON document kept in the drawing (NOD XRecord `OPENINFRA/<name>`), in
/// pieces of at most 2000 characters; `None` when there is none.
pub fn json_record_read(name: &str) -> Option<String> {
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let Ok(PluginResponse::XRecordEntries(Ok(entries))) = request(PluginRequest::ReadXRecord { dictionary_path: path, name: name.into() }) else {
        return None;
    };
    let text: String = entries.into_iter().filter_map(|e| match e.value {
        XRecordValue::String(s) => Some(s),
        _ => None,
    }).collect();
    (!text.is_empty()).then_some(text)
}

/// Writes a JSON document into the drawing (see [`json_record_read`]).
pub fn json_record_write(name: &str, json: &str) -> Result<(), String> {
    let chars: Vec<char> = json.chars().collect();
    let entries: Vec<XRecordEntry> = chars.chunks(2000).map(|c| XRecordEntry::string(1, c.iter().collect::<String>())).collect();
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: name.into(), entries: entries.clone() };
    if matches!(request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        return Ok(());
    }
    match request(PluginRequest::CreateXRecord { dictionary_path: path, name: name.into(), entries })? {
        PluginResponse::XRecordCreateResult(Ok(_)) => Ok(()),
        PluginResponse::XRecordCreateResult(Err(e)) => Err(e),
        _ => Ok(()),
    }
}

pub fn index_read() -> Vec<(String, String, Vec<Handle>)> {
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let Ok(PluginResponse::XRecordEntries(Ok(entries))) = request(PluginRequest::ReadXRecord { dictionary_path: path, name: INDEX_RECORD.into() }) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, Vec<Handle>)> = Vec::new();
    for e in entries {
        let XRecordValue::String(s) = e.value else { continue };
        let mut parts = s.splitn(3, '\t');
        let (Some(kind), Some(key), Some(list)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let hs: Vec<Handle> = list.split(' ').filter_map(|h| u64::from_str_radix(h, 16).ok()).map(Handle::from).collect();
        match out.iter_mut().find(|(k, n, _)| k == kind && n == key) {
            Some((_, _, v)) => v.extend(hs),
            None => out.push((kind.to_string(), key.to_string(), hs)),
        }
    }
    out
}

/// Add handles to the index under kind and key (`replace` drops the old ones).
pub fn index_add(kind: &str, key: &str, handles: &[Handle], replace: bool) {
    if handles.is_empty() {
        return;
    }
    let _one = index_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut all = index_read();
    match all.iter_mut().find(|(k, n, _)| k == kind && n == key) {
        Some((_, _, v)) => {
            if replace {
                v.clear();
            }
            v.extend_from_slice(handles);
        }
        None => all.push((kind.to_string(), key.to_string(), handles.to_vec())),
    }
    // Lines of at most ~2000 characters.
    let mut entries = Vec::new();
    for (k, n, hs) in &all {
        let mut seen = HashSet::new();
        let list: Vec<String> = hs.iter().filter(|h| seen.insert(**h)).map(|h| format!("{:X}", u64::from(*h))).collect();
        for chunk in list.chunks(200) {
            entries.push(XRecordEntry::string(1, format!("{k}\t{n}\t{}", chunk.join(" "))));
        }
    }
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: INDEX_RECORD.into(), entries: entries.clone() };
    if !matches!(request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        let _ = request(PluginRequest::CreateXRecord { dictionary_path: path, name: INDEX_RECORD.into(), entries });
    }
}

/// Set many keys of one kind in a single write: each key gets exactly these
/// handles, and a key with none is dropped from the index (labels that
/// follow their source, `civiel::labels`).
pub fn index_set_many(kind: &str, keys: &[(String, Vec<Handle>)]) {
    if keys.is_empty() {
        return;
    }
    let _one = index_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut all = index_read();
    for (key, handles) in keys {
        all.retain(|(k, n, _)| !(k == kind && n == key));
        if !handles.is_empty() {
            all.push((kind.to_string(), key.clone(), handles.clone()));
        }
    }
    let mut entries = Vec::new();
    for (k, n, hs) in &all {
        let mut seen = HashSet::new();
        let list: Vec<String> = hs.iter().filter(|h| seen.insert(**h)).map(|h| format!("{:X}", u64::from(*h))).collect();
        for chunk in list.chunks(200) {
            entries.push(XRecordEntry::string(1, format!("{k}\t{n}\t{}", chunk.join(" "))));
        }
    }
    if entries.is_empty() {
        // An XRecord keeps at least one entry.
        entries.push(XRecordEntry::string(1, "-\t-\t".to_string()));
    }
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: INDEX_RECORD.into(), entries: entries.clone() };
    if !matches!(request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        let _ = request(PluginRequest::CreateXRecord { dictionary_path: path, name: INDEX_RECORD.into(), entries });
    }
}

/// Entities by handle, without fetching the drawing (API 12).
pub fn entities(handles: &[Handle]) -> Result<Vec<Option<EntityType>>, String> {
    match request(PluginRequest::GetEntitiesBatch { handles: handles.to_vec() })? {
        PluginResponse::GetEntitiesBatchResult(r) => r,
        other => Err(format!("onverwacht antwoord: {other:?}")),
    }
}

/// The handles selected in the drawing.
pub fn selection() -> Result<Vec<Handle>, String> {
    match request(PluginRequest::GetSelection)? {
        PluginResponse::Selection(h) => Ok(h),
        other => Err(format!("onverwacht antwoord op de selectie: {other:?}")),
    }
}

/// Select elements in the drawing (an empty list clears the selection).
pub fn select(handles: &[Handle]) -> Result<(), String> {
    request(PluginRequest::SetSelection { handles: handles.to_vec() }).map(|_| ())?;
    let _ = request(PluginRequest::BumpGeometry);
    Ok(())
}

/// Give an existing layer the NLCS colour, lineweight and linetype.
pub fn restyle_layer(spec: &crate::nlcs::LayerSpec) -> Result<(), String> {
    let linetype = ensure_linetype(&spec.linetype).or_else(|| Some("Continuous".into()));
    let config = LayerConfig {
        name: spec.name.clone(),
        color: Some(Color::Index(spec.color.max(1))),
        lineweight: spec.lineweight.map(LineWeight::from_value),
        linetype,
        ..Default::default()
    };
    request(PluginRequest::ModifyLayer(config)).map(|_| ())
}

/// Remove elements, as one undo step.
pub fn remove_entities(label: &str, handles: &[Handle]) -> Result<usize, String> {
    if handles.is_empty() {
        return Ok(0);
    }
    with_import_undo(label, || {
        let mut n = 0;
        for h in handles {
            if matches!(request(PluginRequest::RemoveEntity { handle: *h }), Ok(PluginResponse::Bool(true))) {
                n += 1;
            }
        }
        let _ = request(PluginRequest::SetDirty);
        let _ = request(PluginRequest::BumpGeometry);
        Ok(n)
    })
}

/// Replace elements in one undo step (an atomic transaction in the host).
pub fn update_entities(label: &str, entities: Vec<EntityType>) -> Result<usize, String> {
    let n = entities.len();
    if n == 0 {
        return Ok(0);
    }
    match request(PluginRequest::UpdateEntitiesTransaction { label: label.into(), entities })? {
        PluginResponse::EntityTransactionResult(Ok(())) => {
            let _ = request(PluginRequest::SetDirty);
            let _ = request(PluginRequest::BumpGeometry);
            Ok(n)
        }
        PluginResponse::EntityTransactionResult(Err(e)) => Err(e),
        other => Err(format!("onverwacht antwoord: {other:?}")),
    }
}

/// The NLCS layers set up in the active drawing during this session.
pub fn nlcs_layers() -> Vec<crate::nlcs::LayerSpec> {
    let Some(tab) = target().map(|t| t.tab_id) else { return Vec::new() };
    shared().lock().unwrap().nlcs_layers.iter().filter(|(t, _)| *t == tab).map(|(_, s)| s.clone()).collect()
}

/// Record a data source of an import into the active drawing.
pub fn note_source(source: &str) {
    let Some(tab) = target().map(|t| t.tab_id) else { return };
    let mut s = shared().lock().unwrap();
    if !s.sources.iter().any(|(t, v)| *t == tab && v == source) {
        s.sources.push((tab, source.to_string()));
    }
}

pub fn sources() -> Vec<String> {
    let Some(tab) = target().map(|t| t.tab_id) else { return Vec::new() };
    shared().lock().unwrap().sources.iter().filter(|(t, _)| *t == tab).map(|(_, v)| v.clone()).collect()
}

/// An XDATA record with `key=value` strings (each at most 255 characters).
pub fn object_data<'a>(pairs: impl IntoIterator<Item = (&'a str, String)>) -> ExtendedDataRecord {
    let mut r = ExtendedDataRecord::new(XDATA_APP);
    for (k, v) in pairs {
        let v = v.replace(['\n', '\r'], " ");
        if v.trim().is_empty() {
            continue;
        }
        let mut s = format!("{k}={}", v.trim());
        if s.len() > 255 {
            let mut cut = 255;
            while !s.is_char_boundary(cut) {
                cut -= 1;
            }
            s.truncate(cut);
        }
        r.add_value(XDataValue::String(s));
    }
    r
}

/// The host registers an XDATA application name only through WriteRecord:
/// rewrite one entity's record so the drawing carries the APPID.
pub fn register_object_data(handle: Handle, record: ExtendedDataRecord) {
    let _ = request(PluginRequest::WriteRecord { handle, record });
}

const BATCH_BYTES: usize = 12 * 1024 * 1024;

/// Hold API 23's history step across worker requests. Nested placement
/// functions join the outer import. Keep fetches outside this scope.
pub(crate) fn with_import_undo<R>(label: &str, write: impl FnOnce() -> Result<R, String>) -> Result<R, String> {
    if IMPORT_UNDO_OPEN.get() {
        return write();
    }
    if COMMAND_CONTEXT.with(|c| c.borrow().is_none()) {
        let t = target().ok_or("geen tekening actief")?;
        return with_command_context(t, site(), None, || with_import_undo(label, write));
    }
    with_writes_held(|| {
        begin_layer_undo(label)?;
        struct CloseUndo;
        impl Drop for CloseUndo {
            fn drop(&mut self) {
                if IMPORT_UNDO_OPEN.replace(false) {
                    let _ = request(PluginRequest::EndUndoGroup);
                }
            }
        }
        IMPORT_UNDO_OPEN.set(true);
        let _close = CloseUndo;
        let result = write();
        let closed = request(PluginRequest::EndUndoGroup).and_then(|response| match response {
            PluginResponse::Ok => Ok(()),
            other => Err(format!("EndUndoGroup: {other:?}")),
        });
        IMPORT_UNDO_OPEN.set(false);
        result.and_then(|value| closed.map(|()| value))
    })
}

/// Hold the write lock that serialises plug-in writes, without an undo
/// step: for what must happen together with a write but outside its undo
/// group, such as switching to the layout it writes in (a switch with the
/// block editor open closes an open undo group). Nested calls, and
/// [`with_import_undo`] inside it, join the lock.
pub(crate) fn with_writes_held<R>(f: impl FnOnce() -> R) -> R {
    if WRITES_HELD.get() {
        return f();
    }
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            WRITES_HELD.set(false);
        }
    }
    WRITES_HELD.set(true);
    let _release = Release;
    f()
}

/// A generous guess of an entity's size on the wire.
fn estimated_size(e: &EntityType) -> usize {
    match e {
        EntityType::Mesh(m) => 40 * m.vertices.len() + 48 * m.faces.len() + 2000,
        EntityType::PolyfaceMesh(m) => 400 * (m.vertices.len() + m.faces.len()) + 2000,
        EntityType::Polyline3D(p) => 300 * p.vertices.len() + 2000,
        EntityType::LwPolyline(p) => 64 * p.vertices.len() + 2000,
        EntityType::Hatch(_) => 20_000,
        _ => 2000,
    }
}

/// Add entities as one undo step, in chunks that stay well under the IPC
/// frame limit.
pub fn add_entities(label: &str, entities: Vec<EntityType>) -> Result<usize, String> {
    if entities.is_empty() {
        return Ok(0);
    }
    Ok(add_entities_handles(label, entities)?.len())
}

// Serialize worker writes so one import cannot relock a layer while another
// is still writing to it, or close another operation's undo group.
static WRITING: Mutex<()> = Mutex::new(());

/// Like [`add_entities`], returning the new handles.
pub fn add_entities_handles(label: &str, entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    with_import_undo(label, || {
        let mut handles = Vec::with_capacity(entities.len());
        entity_batches(entities, |chunk| {
            handles.extend(confirmed_entities(send_entities(chunk)?)?);
            Ok(())
        })?;
        let _ = request(PluginRequest::SetDirty);
        let _ = request(PluginRequest::BumpGeometry);
        Ok(handles)
    })
}

fn entity_batches(
    entities: Vec<EntityType>,
    mut send: impl FnMut(Vec<EntityType>) -> Result<(), String>,
) -> Result<(), String> {
    let mut chunk = Vec::new();
    let mut bytes = 0;
    for entity in entities {
        let size = estimated_size(&entity);
        if !chunk.is_empty() && (bytes + size > BATCH_BYTES || chunk.len() >= 2000) {
            send(std::mem::take(&mut chunk))?;
            bytes = 0;
        }
        bytes += size;
        chunk.push(entity);
    }
    if !chunk.is_empty() {
        send(chunk)?;
    }
    Ok(())
}

fn change_layer_lock(name: &str, locked: bool) -> Result<(), String> {
    let config = LayerConfig { name: name.into(), locked: Some(locked), ..Default::default() };
    match request(PluginRequest::ModifyLayer(config))? {
        PluginResponse::Bool(true) => Ok(()),
        other => Err(format!("{} {name}: {other:?}", crate::lang::tr(
            "OCS kon het laagslot niet wijzigen voor", "OCS could not change the lock for layer",
        ))),
    }
}

/// Track only layers touched by this write. The host is the source of truth:
/// users can change locks in OCS between imports, including after reopening.
pub(crate) struct ImportLayers {
    writable: HashSet<String>,
    relock: BTreeMap<String, String>,
    auto_lock: bool,
    registered: bool,
}

impl ImportLayers {
    pub(crate) fn unlock(&mut self, layers: impl IntoIterator<Item = String>) -> Result<(), String> {
        use ocs_plugin_api::host::codec::Point;

        for name in layers {
            let key = normalize_name(&name);
            if self.writable.contains(&key) {
                continue;
            }
            // API 23 has no layer-state read. A valid point probes just this
            // layer. OCS refuses it on a locked layer. Remove an accepted point
            // inside the same undo group before proceeding with the import.
            let mut point = Point::default();
            point.common.layer = name.clone();
            match request(PluginRequest::AddEntity(EntityType::Point(point)))? {
                PluginResponse::Handle(handle) if handle.is_null() => {
                    self.relock.insert(key.clone(), name.clone());
                    change_layer_lock(&name, false)?;
                }
                PluginResponse::Handle(handle) => match request(PluginRequest::RemoveEntity { handle })? {
                    PluginResponse::Bool(true) => {}
                    other => return Err(format!("RemoveEntity: {other:?}")),
                },
                other => return Err(format!("AddEntity: {other:?}")),
            }
            self.writable.insert(key);
        }
        Ok(())
    }

    pub(crate) fn mark_written(&mut self, layer: &str) {
        if self.auto_lock {
            self.relock.insert(normalize_name(layer), layer.to_owned());
        }
    }

    fn add(&mut self, chunk: Vec<EntityType>, added: &mut Vec<Handle>) -> Result<(), String> {
        let mut handles = send_entities(chunk.clone())?;
        let start = added.len();
        added.extend(handles.iter().copied().filter(|h| !h.is_null()));
        let skipped: Vec<_> = handles.iter().enumerate()
            .filter_map(|(i, h)| h.is_null().then_some(i)).collect();
        for (entity, handle) in chunk.iter().zip(&handles) {
            if !handle.is_null() {
                self.mark_written(&entity.common().layer);
                self.writable.insert(normalize_name(&entity.common().layer));
            }
        }
        if !skipped.is_empty() {
            let retry: Vec<_> = skipped.into_iter()
                .filter(|&i| !self.writable.contains(&normalize_name(&chunk[i].common().layer)))
                .collect();
            if !retry.is_empty() {
                self.unlock(retry.iter().map(|&i| chunk[i].common().layer.clone()))?;
                let entities = retry.iter().map(|&i| chunk[i].clone()).collect();
                for (i, handle) in retry.into_iter().zip(send_entities(entities)?) {
                    handles[i] = handle;
                    if !handle.is_null() {
                        self.mark_written(&chunk[i].common().layer);
                    }
                }
                // Keep the original input order, including the successful retries.
                added.truncate(start);
                added.extend(handles.iter().copied().filter(|h| !h.is_null()));
            }
        }
        // WriteRecord is refused on a locked layer as well. Register the
        // drawing's APPID while an accepted entity is still writable.
        if !self.registered {
            let data = chunk.iter().zip(&handles).filter(|(_, handle)| !handle.is_null())
                .find_map(|(entity, &handle)| {
                    entity.common().extended_data.get_record(XDATA_APP).cloned().map(|record| (handle, record))
                });
            if let Some((handle, record)) = data {
                match request(PluginRequest::WriteRecord { handle, record })? {
                    PluginResponse::Bool(true) => self.registered = true,
                    _ => return Err(crate::lang::tr(
                        "OCS kon de geïmporteerde objectgegevens niet registreren",
                        "OCS could not register the imported object data",
                    ).into()),
                }
            }
        }
        confirmed_entities(handles).map(drop)
    }
}

pub(crate) fn with_import_layers<R>(
    label: &str,
    write: impl FnOnce(&mut ImportLayers) -> Result<R, String>,
) -> Result<R, String> {
    with_layer_writes(label, crate::settings::lock_imported_layers(), write)
}

fn with_layer_writes<R>(
    label: &str,
    auto_lock: bool,
    write: impl FnOnce(&mut ImportLayers) -> Result<R, String>,
) -> Result<R, String> {
    with_import_undo(label, || {
        let mut layers = ImportLayers {
            writable: HashSet::new(),
            relock: BTreeMap::new(),
            auto_lock,
            registered: false,
        };
        let result = write(&mut layers);
        let mut errors = Vec::new();
        for name in layers.relock.into_values() {
            if let Err(error) = change_layer_lock(&name, true) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            return result;
        }
        if let Err(error) = result {
            errors.insert(0, error);
        }
        Err(errors.join("\n"))
    })
}

fn begin_layer_undo(label: &str) -> Result<(), String> {
    match request(PluginRequest::BeginUndoGroup { label: label.into() })? {
        PluginResponse::Ok => Ok(()),
        other => Err(format!("BeginUndoGroup: {other:?}")),
    }
}

pub(crate) fn add_imported_entities(label: &str, entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
    write_import(label, entities, false)
}

pub(crate) fn append_entities(label: &str, entities: Vec<EntityType>, originals: &[(Handle, String)]) -> Result<Vec<Handle>, String> {
    with_layer_writes(label, false, |layers| {
        let handles = layers.write_entities_inner(entities, Some(label))?;
        layers.remove_originals(originals)?;
        Ok(handles)
    })
}

/// [`append_entities`] for elements that belong in `space` (the block record
/// of model space, say). With the block editor open OCS adds to the block
/// being edited: what it put elsewhere is taken back while its layers are
/// still open, and the write fails with `elsewhere`.
pub(crate) fn append_in(label: &str, new: Vec<EntityType>, space: Handle, elsewhere: &str) -> Result<Vec<Handle>, String> {
    with_layer_writes(label, false, |layers| {
        let handles = layers.write_entities_inner(new, Some(label))?;
        let Some(&first) = handles.first() else { return Ok(handles) };
        match owner(first) {
            Ok(owner) if owner == space => Ok(handles),
            Ok(_) => Err(take_back(label, &handles, elsewhere.into())),
            Err(error) => Err(take_back(label, &handles, error)),
        }
    })
}

/// The block record that owns element `handle`.
pub(crate) fn owner(handle: Handle) -> Result<Handle, String> {
    entities(&[handle])?.into_iter().flatten().next().map(|e| e.common().owner_handle).ok_or_else(|| format!("{handle:?}?"))
}

/// Remove what a failed write added; `error` says why the write failed,
/// with a request to undo when OCS kept part of it.
fn take_back(label: &str, handles: &[Handle], error: String) -> String {
    match remove_entities(label, handles) {
        Ok(count) if count == handles.len() => error,
        _ => format!("{error}\n{}", crate::lang::tr(
            "OCS kon de reeds geplaatste kopieën niet allemaal opruimen. Gebruik Ongedaan maken.",
            "OCS could not clean up all newly added copies. Use Undo.",
        )),
    }
}

/// Remove `originals` (handle, layer) in one undo step, also from a locked
/// layer, which is locked again after it.
pub(crate) fn remove_replaced(label: &str, originals: &[(Handle, String)]) -> Result<(), String> {
    with_layer_writes(label, false, |layers| layers.remove_originals(originals))
}

fn send_entities(entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
    let count = entities.len();
    match request(PluginRequest::AddEntities(entities))? {
        PluginResponse::Handles(handles) if handles.len() == count => Ok(handles),
        _ => Err(crate::lang::tr("OCS bevestigde de toegevoegde elementen niet", "OCS did not confirm the added entities").into()),
    }
}

fn confirmed_entities(handles: Vec<Handle>) -> Result<Vec<Handle>, String> {
    if handles.iter().any(|h| h.is_null()) {
        return Err(crate::lang::tr(
            "OCS weigerde elementen. Controleer of de doellaag vergrendeld is.",
            "OCS rejected entities. Check whether the target layer is locked.",
        ).into());
    }
    Ok(handles)
}

pub(crate) fn add_imported_map(label: &str, entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
    write_import(label, entities, true)
}

fn write_import(label: &str, entities: Vec<EntityType>, map: bool) -> Result<Vec<Handle>, String> {
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    with_import_layers(label, |layers| {
        let handles = layers.write_entities(entities)?;
        if map {
            image_to_back(&handles, layers)?;
        }
        Ok(handles)
    })
}

impl ImportLayers {
    /// Remove `originals` (handle, layer), opening a locked layer for it. One
    /// that is gone already counts as removed. A replacement that must keep
    /// its originals until the new elements are in calls it after them
    /// ([`append_entities`]).
    pub(crate) fn remove_originals(&mut self, originals: &[(Handle, String)]) -> Result<(), String> {
        self.unlock(originals.iter().map(|(_, layer)| layer.clone()))?;
        for (handle, _) in originals {
            match request(PluginRequest::RemoveEntity { handle: *handle })? {
                PluginResponse::Bool(true) => {}
                // Gone already (a user deleted it meanwhile): removed.
                _ if entities(&[*handle])?.first().is_some_and(Option::is_none) => {}
                _ => return Err(crate::lang::tr(
                    "OCS kon een origineel object niet verwijderen.",
                    "OCS could not remove an original entity.",
                ).into()),
            }
        }
        if !originals.is_empty() {
            let _ = request(PluginRequest::SetDirty);
            let _ = request(PluginRequest::BumpGeometry);
        }
        Ok(())
    }

    pub(crate) fn write_entities(&mut self, entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
        self.write_entities_inner(entities, None)
    }

    fn write_entities_inner(&mut self, entities: Vec<EntityType>, rollback_label: Option<&str>) -> Result<Vec<Handle>, String> {
        let mut handles = Vec::with_capacity(entities.len());
        if let Err(error) = entity_batches(entities, |chunk| self.add(chunk, &mut handles)) {
            return Err(match rollback_label {
                Some(label) => take_back(label, &handles, error),
                None => error,
            });
        }
        let _ = request(PluginRequest::SetDirty);
        // Force a rebuild after a batch (an image can hide newer lines).
        let _ = request(PluginRequest::BumpGeometry);
        Ok(handles)
    }
}

const SCRIPT_COMMANDS_OFF: &str = "running commands from a script is turned off (SCRIPTCOMMANDS = 0); turn it on at the command line with SCRIPTCOMMANDS 1";

/// Return a notice if the host refuses draw order before running the command.
fn run_draw_order(handles: &[Handle], command: &str) -> Result<Option<&'static str>, String> {
    let select = |handles| {
        match request(PluginRequest::SetSelection { handles })? {
            PluginResponse::SelectionResult(result) => result,
            other => Err(format!("SetSelection: {other:?}")),
        }
    };
    let previous = selection()?;
    select(handles.to_vec())?;
    let moved = run_on_target(command);
    let restored = select(previous);
    let notice = match moved {
        Ok(()) => None,
        // API 23 returns these strings before running the command. The
        // actual import has already written its geometry.
        Err(error) => Some(match error.as_str() {
            "commands act on the active document; switch to it first" => crate::lang::tr(
                "Kaartbeelden geplaatst. De tekenvolgorde is overgeslagen omdat de tekening niet meer actief is.",
                "Map images were placed. Draw order was skipped because the drawing is no longer active.",
            ),
            "a command is already active; cancel it first" => crate::lang::tr(
                "Kaartbeelden geplaatst. De tekenvolgorde is overgeslagen omdat er al een tekenopdracht actief is.",
                "Map images were placed. Draw order was skipped because another drawing command is active.",
            ),
            SCRIPT_COMMANDS_OFF => crate::lang::tr(
                "Kaartbeelden geplaatst. De tekenvolgorde is overgeslagen omdat SCRIPTCOMMANDS uitstaat.",
                "Map images were placed. Draw order was skipped because SCRIPTCOMMANDS is off.",
            ),
            _ => return Err(error),
        }),
    };
    restored.map(|()| notice)
}

/// Keep the new map below vector data and above older images. The host
/// excludes locked entities from DRAWORDER, so reopen the older image layers
/// for this part of the same import and restore their original lock state.
pub(crate) fn image_to_back(new: &[Handle], layers: &mut ImportLayers) -> Result<(), String> {
    if new.is_empty() {
        return Ok(());
    }
    let rasters = query(serde_json::json!({"type": "RASTERIMAGE"}))?;
    let imported: HashSet<Handle> = new.iter().copied().collect();
    let owner = rasters.iter().find(|e| imported.contains(&e.common().handle))
        .map(|e| e.common().owner_handle);
    let Some(owner) = owner else {
        return Err(crate::lang::tr("OCS gaf de geplaatste kaartbeelden niet terug", "OCS did not return the placed map images").into());
    };
    let older: Vec<_> = rasters.into_iter().filter_map(|e| {
        let c = e.common();
        (c.owner_handle == owner && !imported.contains(&c.handle))
            .then(|| (c.handle, c.layer.clone()))
    }).collect();
    let current = effective_draw_order(owner);
    let moves = raster_draw_order(older.iter().map(|(handle, _)| *handle), new, current.as_deref());
    if moves.is_empty() {
        return Ok(());
    }
    let moved_handles: HashSet<_> = moves.iter().flat_map(|step| step.handles.iter().copied()).collect();
    layers.unlock(older.iter().filter(|(handle, _)| moved_handles.contains(handle)).map(|(_, name)| name.clone()))?;
    let moved = (|| {
        for step in moves {
            let command = match step.under {
                Some(reference) => format!("DRAWORDER UNDER {:X}", reference.value()),
                None => "DRAWORDER_BACK".to_owned(),
            };
            if let Some(notice) = run_draw_order(&step.handles, &command)? {
                return Ok::<_, String>(Some(notice));
            }
        }
        Ok(None)
    })();
    if !layers.relock.is_empty() {
        // Native DRAWORDER can close the import group. Record the subsequent
        // lock changes together even when several image layers were reopened.
        let scope = begin_layer_undo(crate::lang::tr("Kaartlagen vergrendelen", "Lock map layers"));
        scope?;
    }
    if let Some(message) = moved? {
        crate::civiel::api::note("tekenvolgorde_overgeslagen", message);
        push_info(message);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct RasterMove {
    handles: Vec<Handle>,
    under: Option<Handle>,
}

fn raster_draw_order(
    older: impl IntoIterator<Item = Handle>,
    new: &[Handle],
    current: Option<&[(Handle, u64)]>,
) -> Vec<RasterMove> {
    let mut remaining: HashSet<_> = older.into_iter().collect();
    let mut order: Vec<_> = current.into_iter().flatten().map(|(handle, _)| *handle)
        .filter(|handle| remaining.remove(handle))
        .collect();
    // A large drawing may not fit in a snapshot. Use handle order for
    // rasters whose positions are unavailable, then append the new tiles.
    let mut unknown: Vec<_> = remaining.into_iter().collect();
    unknown.sort_unstable_by_key(|handle| handle.value());
    order.extend(unknown);
    let old_count = order.len();
    order.extend_from_slice(new);
    let back = |handles| vec![RasterMove { handles, under: None }];
    let Some(current) = current else { return back(order) };
    let current_handles = || current.iter().map(|(handle, _)| *handle);
    if current_handles().take(order.len()).eq(order.iter().copied()) {
        return Vec::new();
    }
    let rasters: HashSet<_> = order.iter().copied().collect();
    let Some((vector, sort_key)) = current.iter().find(|(handle, _)| !rasters.contains(handle)).copied() else {
        return back(order);
    };
    if sort_key > order.len() as u64 {
        return back(order);
    }

    // With no spare keys below the first vector, BACK can leapfrog vectors.
    // UNDER preserves the effective order of the selection and its siblings.
    let under = |handles| RasterMove { handles, under: Some(vector) };
    if current_handles().filter(|handle| rasters.contains(handle)).eq(order.iter().copied()) {
        return vec![under(order)];
    }
    let new_handles: HashSet<_> = new.iter().copied().collect();
    if current_handles().filter(|handle| !new_handles.contains(handle)).take(old_count).eq(order[..old_count].iter().copied()) {
        return vec![under(new.to_vec())];
    }
    vec![under(order[..old_count].to_vec()), under(new.to_vec())]
}

fn effective_draw_order(owner: Handle) -> Option<Vec<(Handle, u64)>> {
    use ocs_plugin_api::host::codec::objects::ObjectType;
    // Shared views have no sort table. Only request the full snapshot when
    // the shared view puts the drawing comfortably below the IPC frame limit.
    if !view_entities(|_| false).is_some_and(|(_, _, bytes)| bytes < SNAPSHOT_SAFE) {
        return None;
    }
    let Ok(drawing) = full_snapshot() else {
        return None;
    };
    let overrides: std::collections::HashMap<_, _> = drawing.objects.values().find_map(|object| match object {
        ObjectType::SortEntitiesTable(table) if table.block_owner_handle == owner => {
            Some(table.entries().map(|entry| (entry.entity_handle, entry.sort_handle.value())).collect())
        }
        _ => None,
    }).unwrap_or_default();
    let mut order: Vec<_> = drawing.entities().filter_map(|entity| {
        let c = entity.common();
        (c.owner_handle == owner || c.owner_handle.is_null()).then(|| {
            (c.handle, overrides.get(&c.handle).copied().unwrap_or_else(|| c.handle.value()))
        })
    }).collect();
    order.sort_unstable_by_key(|(handle, key)| (*key, handle.value()));
    Some(order)
}

fn run_on_target(line: &str) -> Result<(), String> {
    match request(PluginRequest::RunCommand { request: CommandRequest::Run { line: line.into() } })? {
        PluginResponse::CommandResult(result) => {
            let result = result?;
            if let Some(error) = result.error {
                return Err(error);
            }
            if result.status != "completed" {
                return Err(format!("Open CAD Studio heeft '{line}' niet voltooid"));
            }
            Ok(())
        }
        other => Err(format!("onverwacht antwoord op '{line}': {other:?}")),
    }
}

/// Attach a point cloud on its own layer. OCS's native command always uses
/// the current layer and creates the definition and reactor itself.
pub(crate) fn attach_imported_cloud(path: &str, layer: &str) -> Result<(), String> {
    use ocs_plugin_api::host::{CommandRequest, HostSettingValue};
    use crate::lang::tr;

    let label = tr("Puntenwolk importeren", "Import point cloud");
    with_import_layers(label, |layers| {
        ensure_layer(layer, Color::from_index(7));
        layers.unlock([layer.to_owned()])?;
        let get = PluginRequest::GetSystemVariable { name: "CLAYER".into() };
        let PluginResponse::SystemVariable(Some(HostSettingValue::Text(previous))) = request(get)? else {
            return Err(tr("OCS gaf de huidige laag niet terug", "OCS did not return the current layer").into());
        };
        let current = |name: &str| {
            match request(PluginRequest::SetSystemVariable {
                name: "CLAYER".into(), value: HostSettingValue::Text(name.into()),
            })? {
                PluginResponse::SystemVariableResult(Ok(_)) => Ok(()),
                PluginResponse::SystemVariableResult(Err(error)) => Err(error),
                other => Err(format!("CLAYER: {other:?}")),
            }
        };
        current(layer)?;
        let result = request(PluginRequest::RunCommand {
            request: CommandRequest::RunQuiet { line: format!("-POINTCLOUDATTACH {path} 0,0,0 1 0") },
        });
        let restore = current(&previous);
        match result? {
            PluginResponse::CommandResult(Ok(outcome)) if outcome.added == 1 && outcome.error.is_none() => {
                layers.mark_written(layer);
                restore
            }
            PluginResponse::CommandResult(Ok(outcome)) => {
                restore?;
                Err(outcome.error.unwrap_or_else(|| tr("OCS heeft de puntenwolk niet gekoppeld", "OCS did not attach the point cloud").into()))
            }
            PluginResponse::CommandResult(Err(error)) => {
                restore?;
                Err(error)
            }
            other => {
                restore?;
                Err(format!("POINTCLOUDATTACH: {other:?}"))
            }
        }
    })
}

/// Run a command line in the active drawing (tokens answer its prompts).
pub fn execute(command: &str) -> Result<(), String> {
    match request(PluginRequest::ExecuteCommand(command.into()))? {
        PluginResponse::Bool(false) => Err(format!("Open CAD Studio voerde '{}' niet uit", command.split_whitespace().next().unwrap_or(""))),
        _ => Ok(()),
    }
}

pub fn zoom_extents() {
    let _ = run_on_target("ZOOMEXTENTS");
}

/// A file beside the drawing (`<tekening>_<suffix>`), or in the plugin's
/// config folder while the drawing has not been saved.
pub fn beside_drawing(suffix: &str) -> PathBuf {
    file_beside(target().and_then(|t| t.doc_path), suffix)
}

/// `<folder of doc>/<stem>_<suffix>` as one file name: a name in the suffix
/// (an axis, a sewer, from the drawing or an agent) cannot add folders or
/// `..` to the path.
fn file_beside(doc: Option<PathBuf>, suffix: &str) -> PathBuf {
    let suffix: String = suffix.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let suffix = suffix.as_str();
    match doc {
        Some(p) => {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("tekening").to_string();
            p.parent().map(|d| d.join(format!("{stem}_{suffix}"))).unwrap_or_else(|| crate::gis::servers::config_dir().join(suffix))
        }
        None => crate::gis::servers::config_dir().join(format!("tekening_{suffix}")),
    }
}

/// The drawing's file name, for reports.
pub fn drawing_name() -> String {
    target().and_then(|t| t.doc_path).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "(niet opgeslagen tekening)".into())
}

/// Folder for downloaded map images: beside the drawing when it is saved,
/// otherwise in the plugin's config folder.
pub fn image_dir() -> PathBuf {
    let doc = target().and_then(|t| t.doc_path);
    match doc {
        Some(p) => {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("tekening").to_string();
            p.parent().map(|d| d.join(format!("{stem}_GIS"))).unwrap_or_else(|| crate::gis::servers::config_dir().join("kaarten"))
        }
        None => crate::gis::servers::config_dir().join("kaarten"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_import_preserves_existing_stacking() {
        let [older_bottom, older_top, vector, new_bottom, new_top] = [21, 15, 12, 30, 32].map(Handle::new);
        let older = [older_top, older_bottom];
        let new = [new_bottom, new_top];
        let wanted = [older_bottom, older_top, new_bottom, new_top];

        let current = [(older_bottom, 5), (older_top, 6), (vector, 12), (new_bottom, 30), (new_top, 32)];
        assert_eq!(raster_draw_order(older, &new, Some(&current)), [RasterMove { handles: wanted.to_vec(), under: None }]);

        let current = [(older_bottom, 1), (older_top, 2), (new_bottom, 3), (new_top, 4), (vector, 5)];
        assert!(raster_draw_order(older, &new, Some(&current)).is_empty());
        assert_eq!(
            raster_draw_order(older, &new, None),
            [RasterMove { handles: [older_top, older_bottom, new_bottom, new_top].to_vec(), under: None }],
        );
    }

    #[test]
    fn raster_import_uses_relative_order_when_back_would_collide() {
        let [older_bottom, older_top, vector, new] = [21, 15, 25, 30].map(Handle::new);
        let older = [older_top, older_bottom];
        let under_vector = |handles| RasterMove { handles, under: Some(vector) };

        let current = [(older_bottom, 1), (older_top, 2), (vector, 3), (new, 30)];
        assert_eq!(raster_draw_order(older, &[new], Some(&current)), [under_vector(vec![older_bottom, older_top, new])]);

        let current = [(older_bottom, 1), (new, 2), (older_top, 3), (vector, 3)];
        assert_eq!(raster_draw_order(older, &[new], Some(&current)), [under_vector(vec![new])]);

        let current = [(older_bottom, 1), (new, 2), (vector, 3), (older_top, 4)];
        assert_eq!(
            raster_draw_order(older, &[new], Some(&current)),
            [under_vector(vec![older_bottom, older_top]), under_vector(vec![new])],
        );
    }

    /// A layer from before #160 ("… 0-5M") is renamed to the new name before
    /// the import adds it; a name without a comma asks nothing extra.
    #[test]
    fn ensure_layer_takes_over_the_dashed_layer_of_an_earlier_import() {
        #[derive(Default)]
        struct Sender(Mutex<Vec<String>>);
        impl PluginRequestSender for Sender {
            fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
                let mut seen = self.0.lock().unwrap();
                Ok(match req {
                    PluginRequest::TableOperation { operation: TableOperation::LayerRename { from, to } } => {
                        seen.push(format!("rename {from} -> {to}"));
                        PluginResponse::TableResult(Ok(Handle::new(7)))
                    }
                    PluginRequest::AddLayer(config) => {
                        seen.push(format!("add {}", config.name));
                        PluginResponse::OptHandle(None)
                    }
                    other => panic!("unexpected request: {other:?}"),
                })
            }
        }
        let sender = Arc::new(Sender::default());
        let target = Target { sender: sender.clone(), tab_id: 160, unit_factor: 1.0, doc_path: None };
        with_command_context(target, None, None, || {
            ensure_layer("GIS-WMS-DTM 0,5M", Color::from_index(8));
            ensure_layer("GIS-WMS-LUCHTFOTO", Color::from_index(8));
        });
        assert_eq!(
            *sender.0.lock().unwrap(),
            ["rename GIS-WMS-DTM 0-5M -> GIS-WMS-DTM 0,5M", "add GIS-WMS-DTM 0,5M", "add GIS-WMS-LUCHTFOTO"]
        );
    }

    #[test]
    fn draw_order_restores_selection_and_reports_refusals() {
        use ocs_plugin_api::host::PluginRequestError;

        struct Sender {
            command: Result<(), &'static str>,
            restore: Result<(), &'static str>,
            selections: Mutex<Vec<Vec<Handle>>>,
        }
        impl PluginRequestSender for Sender {
            fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
                Ok(match req {
                    PluginRequest::GetSelection => PluginResponse::Selection(vec![Handle::new(1)]),
                    PluginRequest::SetSelection { handles } => {
                        let mut selections = self.selections.lock().unwrap();
                        selections.push(handles);
                        let result = if selections.len() == 1 { Ok(()) } else { self.restore };
                        PluginResponse::SelectionResult(result.map_err(str::to_string))
                    }
                    PluginRequest::RunCommand { request: CommandRequest::Run { line } } => {
                        assert_eq!(line, "DRAWORDER_BACK");
                        assert_eq!(*self.selections.lock().unwrap(), [vec![Handle::new(2)]]);
                        PluginResponse::CommandResult(self.command.map(|()| ocs_plugin_api::host::CommandOutcome {
                            status: "completed".into(),
                            ..Default::default()
                        }).map_err(str::to_string))
                    }
                    other => panic!("unexpected request: {other:?}"),
                })
            }
        }

        let run = |command, restore| {
            let sender = Arc::new(Sender { command, restore, selections: Mutex::default() });
            let target = Target { sender: sender.clone(), tab_id: 141, unit_factor: 1.0, doc_path: None };
            let result = with_command_context(target, None, None, || run_draw_order(&[Handle::new(2)], "DRAWORDER_BACK"));
            assert_eq!(*sender.selections.lock().unwrap(), [vec![Handle::new(2)], vec![Handle::new(1)]]);
            result
        };
        let inactive_tab = Err("commands act on the active document; switch to it first");
        let active_command = Err("a command is already active; cancel it first");
        assert_eq!(run(Ok(()), Ok(())), Ok(None));
        assert!(run(inactive_tab, Ok(())).unwrap().is_some());
        assert!(run(active_command, Ok(())).unwrap().is_some());
        assert!(run(Err(SCRIPT_COMMANDS_OFF), Ok(())).unwrap().is_some());
        assert_eq!(run(active_command, Err("restore failed")), Err("restore failed".into()));
        assert_eq!(run(Err("DRAWORDER failed"), Ok(())), Err("DRAWORDER failed".into()));
    }

    #[test]
    fn entity_chunks_share_one_undo_group() {
        use ocs_plugin_api::host::codec::entities::Point;
        use ocs_plugin_api::host::PluginRequestError;

        #[derive(Default)]
        struct Sender(Mutex<Vec<&'static str>>);
        impl PluginRequestSender for Sender {
            fn request(&self, req: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
                let (name, response) = match req {
                    PluginRequest::BeginUndoGroup { .. } => ("begin", PluginResponse::Ok),
                    PluginRequest::EndUndoGroup => ("end", PluginResponse::Ok),
                    PluginRequest::PushUndo { .. } => ("push", PluginResponse::Ok),
                    PluginRequest::AddEntities(ents) => {
                        let name = if ents.len() == 2000 { "first chunk" } else { "second chunk" };
                        (name, PluginResponse::Handles(vec![Handle::new(1); ents.len()]))
                    }
                    PluginRequest::SetDirty | PluginRequest::BumpGeometry => return Ok(PluginResponse::Ok),
                    other => panic!("unexpected request: {other:?}"),
                };
                self.0.lock().unwrap().push(name);
                Ok(response)
            }
        }

        let sender = Arc::new(Sender::default());
        let drawing = Target { sender: sender.clone(), tab_id: 141, unit_factor: 1.0, doc_path: None };
        let points = vec![EntityType::Point(Point::from_coords(1.0, 2.0, 0.0)); 2001];
        let count = with_command_context(drawing, None, None, || add_entities("Import", points)).unwrap();
        assert_eq!(count, 2001);
        assert_eq!(*sender.0.lock().unwrap(), ["begin", "first chunk", "second chunk", "end"]);
    }

    #[test]
    fn a_name_stays_one_file_beside_the_drawing() {
        let doc = PathBuf::from("/werkmap/tekening.dwg");
        assert_eq!(file_beside(Some(doc.clone()), "punten.csv"), PathBuf::from("/werkmap/tekening_punten.csv"));
        // An axis named "a/../../../outside/overwrite" (review of #103).
        let out = file_beside(Some(doc.clone()), "corridor_a/../../../outside/overwrite.csv");
        assert_eq!(out, PathBuf::from("/werkmap/tekening_corridor_a_.._.._.._outside_overwrite.csv"));
        assert_eq!(out.parent(), Some(std::path::Path::new("/werkmap")));
        assert_eq!(file_beside(Some(doc), "a\\..\\b\nc.csv"), PathBuf::from("/werkmap/tekening_a_.._b_c.csv"));
    }
}

#[cfg(test)]
pub(crate) static COMMAND_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) mod replacement_test_host {
    use super::*;
    use ocs_plugin_api::host::codec::{tables::Layer, CadDocument};

    pub(crate) struct Drawing {
        pub doc: Mutex<CadDocument>,
        reject_batch: bool,
        pub reject_profile: bool,
        pub index: Mutex<Vec<XRecordEntry>>,
        /// Undo groups and record writes, in order.
        pub log: Mutex<Vec<String>>,
        /// The current layout (CTAB). OCS adds to a sheet's layout while one
        /// is current, and to the block being edited while the block editor
        /// is open (CTAB `Model`, and a switch of layout does not close it
        /// while the block has unsaved changes): `outside` keeps what went
        /// outside model space.
        pub layout: Mutex<String>,
        pub outside: Mutex<Vec<Handle>>,
        /// The block editor is open on a block with unsaved changes: a
        /// switch of layout leaves it open.
        pub block_edit: bool,
        /// The block editor is open on a block without changes: a switch of
        /// layout closes it, then switches. Either way OCS first closes an
        /// open undo group (`block_edit_delta` → `finish_pending_history`).
        pub clean_block_edit: Mutex<bool>,
        /// An undo group is open (OCS ignores an end without one).
        open: Mutex<bool>,
        /// Two commands in step: the first two reads of a record wait for
        /// each other (at most a moment), as two commands started together.
        pub meet: Option<(Mutex<u32>, std::sync::Condvar)>,
        /// Elements OCS refuses to remove (as on a layer locked meanwhile).
        pub refuse: Mutex<Vec<Handle>>,
        /// A sheet's layout, once current, stays a moment (at most 300 ms)
        /// before the next request, so another command can switch away
        /// from it meanwhile.
        pub hold_sheet: bool,
        switched: std::sync::Condvar,
    }

    impl Drawing {
        pub(crate) fn new(doc: CadDocument, reject_batch: bool) -> Self {
            Self {
                doc: Mutex::new(doc), reject_batch, reject_profile: false, index: Mutex::new(Vec::new()), log: Mutex::new(Vec::new()),
                layout: Mutex::new("Model".into()), outside: Mutex::new(Vec::new()), block_edit: false, meet: None, refuse: Mutex::new(Vec::new()),
                hold_sheet: false, switched: std::sync::Condvar::new(), clean_block_edit: Mutex::new(false), open: Mutex::new(false),
            }
        }

        /// A new element, in the current layout or the block being edited.
        fn add(&self, doc: &mut CadDocument, mut entity: EntityType) -> Handle {
            let outside = self.block_edit || *self.clean_block_edit.lock().unwrap() || *self.layout.lock().unwrap() != "Model";
            if outside {
                entity.common_mut().owner_handle = doc.header.paper_space_block_handle;
            }
            let handle = doc.add_entity(entity).unwrap();
            if outside {
                self.outside.lock().unwrap().push(handle);
            }
            handle
        }

        fn meet(&self) {
            if let Some((count, met)) = &self.meet {
                let mut n = count.lock().unwrap();
                *n += 1;
                met.notify_all();
                let _ = met.wait_timeout_while(n, std::time::Duration::from_millis(300), |n| *n < 2).unwrap();
            }
        }
    }

    impl PluginRequestSender for Drawing {
        fn request(&self, req: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
            use PluginRequest as Q;
            use PluginResponse as A;

            if matches!(req, Q::ReadXRecord { .. }) {
                self.meet();
            }
            if let Q::TableOperation { operation: TableOperation::LayoutSetCurrent { name } } = &req {
                let mut clean = self.clean_block_edit.lock().unwrap();
                if (self.block_edit || *clean) && std::mem::take(&mut *self.open.lock().unwrap()) {
                    self.log.lock().unwrap().push("end".into());
                }
                *clean = false;
                drop(clean);
                if !self.block_edit {
                    let mut layout = self.layout.lock().unwrap();
                    layout.clone_from(name);
                    self.switched.notify_all();
                    if self.hold_sheet && name != "Model" {
                        let _ = self.switched.wait_timeout_while(layout, std::time::Duration::from_millis(300), |now| now == name).unwrap();
                    }
                }
                return Ok(A::TableResult(Ok(Handle::NULL)));
            }
            let mut doc = self.doc.lock().unwrap();
            let locked = |doc: &CadDocument, entity: &EntityType| {
                doc.layers.get(&entity.common().layer).is_some_and(|layer| layer.is_locked())
            };
            Ok(match req {
                Q::AddLayer(config) if doc.layers.contains(&config.name) => A::OptHandle(None),
                Q::AddLayer(config) => {
                    let mut layer = Layer::new(&config.name);
                    layer.handle = doc.allocate_handle();
                    let handle = layer.handle;
                    doc.layers.add(layer).unwrap();
                    A::OptHandle(Some(handle))
                }
                Q::ModifyLayer(config) => {
                    doc.layers.get_mut(&config.name).unwrap().flags.locked = config.locked.unwrap();
                    A::Bool(true)
                }
                Q::AddEntity(entity) => A::Handle(if locked(&doc, &entity) {
                    Handle::NULL
                } else {
                    self.add(&mut doc, entity)
                }),
                Q::AddEntities(entities) => A::Handles(entities.into_iter().map(|entity| {
                    if self.reject_batch || locked(&doc, &entity)
                        || (self.reject_profile && crate::civiel::data_value(&entity, "rioolprofiel").is_some()) {
                        Handle::NULL
                    } else {
                        self.add(&mut doc, entity)
                    }
                }).collect()),
                Q::RemoveEntity { handle } => {
                    let writable = doc.get_entity(handle).is_some_and(|e| !locked(&doc, e)) && !self.refuse.lock().unwrap().contains(&handle);
                    A::Bool(writable && doc.remove_entity(handle).is_some())
                }
                Q::WriteRecord { handle, .. } => A::Bool(doc.get_entity(handle).is_some_and(|e| !locked(&doc, e))),
                Q::TableOperation { .. } => A::TableResult(Ok(Handle::new(1001))),
                Q::OpenDocumentViewV4 { .. } => A::Error("no shared view in the test host".into()),
                Q::GetSelection => A::Selection(doc.model_space_entities().map(|e| e.common().handle).collect()),
                Q::GetEntitiesBatch { handles } => A::GetEntitiesBatchResult(Ok(handles.iter().map(|h| doc.get_entity(*h).cloned()).collect())),
                Q::ReadXRecord { .. } => A::XRecordEntries(Ok(self.index.lock().unwrap().clone())),
                Q::UpdateXRecord { entries, .. } => {
                    self.log.lock().unwrap().push("record".into());
                    *self.index.lock().unwrap() = entries;
                    A::XRecordUpdateResult(Ok(Handle::new(1002)))
                }
                Q::BeginUndoGroup { .. } => {
                    *self.open.lock().unwrap() = true;
                    self.log.lock().unwrap().push("begin".into());
                    A::Ok
                }
                Q::EndUndoGroup => {
                    if std::mem::take(&mut *self.open.lock().unwrap()) {
                        self.log.lock().unwrap().push("end".into());
                    }
                    A::Ok
                }
                Q::CloseDocumentViewV4 { .. } | Q::SetDirty | Q::BumpGeometry | Q::PushUndo { .. } => A::Ok,
                other => panic!("unexpected host request: {other:?}"),
            })
        }
    }
}

#[cfg(test)]
mod layer_table_tests {
    use super::*;
    use ocs_plugin_api::host::codec::{tables::Layer, CadDocument, Color, Point};
    use ocs_plugin_api::shm::{DocumentSnapshotStore, DocumentViewDataV4, EntityViewV4};

    /// A host that shares a small drawing. OCS sends no answer at all to a
    /// full snapshot too large for one message (its tables and records do
    /// not count in the shared view): `answers` false plays that.
    struct Host {
        view: DocumentSnapshotStore<DocumentViewDataV4>,
        doc: CadDocument,
        answers: bool,
    }

    impl Host {
        fn new(tab_id: u64, answers: bool) -> Arc<Host> {
            let mut doc = CadDocument::new();
            let mut layer = Layer::new("B-XX-VH-GESLOTENVERHARDING_ASFALT-G");
            layer.color = Color::Index(5);
            let _ = doc.layers.add(layer);
            let point = EntityType::Point(Point::default());
            let mut view = DocumentSnapshotStore::new(tab_id, 1 << 20).unwrap();
            view.publish(&DocumentViewDataV4 { layers: Vec::new(), app_ids: Vec::new(), entities: vec![EntityViewV4::from(&point)] }).unwrap();
            Arc::new(Host { view, doc, answers })
        }
    }

    impl PluginRequestSender for Host {
        fn request(&self, req: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
            Ok(match req {
                PluginRequest::OpenDocumentViewV4 { .. } => PluginResponse::DocumentViewV4 { path: self.view.path().display().to_string(), version: 1 },
                PluginRequest::CloseDocumentViewV4 { .. } => PluginResponse::Ok,
                PluginRequest::DocumentSnapshot if self.answers => PluginResponse::Document(Box::new(self.doc.clone().into())),
                PluginRequest::DocumentSnapshot => loop {
                    std::thread::park();
                },
                other => panic!("unexpected host request: {other:?}"),
            })
        }
    }

    fn on(host: Arc<Host>, tab_id: u64, f: impl FnOnce()) {
        with_command_context(Target { sender: host, tab_id, unit_factor: 1.0, doc_path: None }, None, None, f);
    }

    /// The legend of a sheet reads the pens of the drawing's layers (#142).
    #[test]
    fn the_layer_table_has_the_pens_of_the_drawing() {
        on(Host::new(142_001, true), 142_001, || {
            let table = layer_table().expect("a small drawing comes whole");
            assert_eq!(table.get("B-XX-VH-GESLOTENVERHARDING_ASFALT-G").unwrap().color, Color::Index(5));
        });
    }

    /// A drawing whose snapshot OCS cannot send: no table and no stalled
    /// command, so a sheet set never stops halfway (#142).
    #[test]
    fn a_drawing_too_large_for_one_message_gives_no_layer_table_without_waiting() {
        let (tx, rx) = std::sync::mpsc::channel();
        let host = Host::new(142_002, false);
        std::thread::spawn(move || {
            on(host, 142_002, || {
                let _ = tx.send((layer_table().is_none(), snapshot().is_err()));
            })
        });
        let read = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the read waits for an answer that never comes");
        assert_eq!(read, (true, true));
    }
}

#[cfg(test)]
mod command_worker_tests {
    use super::*;
    #[test]
    fn workbench_without_a_drawing_cannot_use_a_later_global_target() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        struct NeverSend;
        impl PluginRequestSender for NeverSend {
            fn request(&self, _: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
                panic!("the documentless workbench contacted CAD");
            }
        }
        struct Restore(Option<Target>, Option<Site>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let mut state = shared().lock().unwrap();
                state.target = self.0.take();
                state.site = self.1.take();
            }
        }

        let restore = {
            let mut state = shared().lock().unwrap();
            Restore(
                state.target.replace(Target {
                    sender: Arc::new(NeverSend),
                    tab_id: 130046,
                    unit_factor: 1.0,
                    doc_path: None,
                }),
                state.site.replace(Site::default()),
            )
        };
        std::thread::spawn(|| with_workbench_document(None, None, || {
            assert!(target().is_none());
            assert!(site().is_none());
            let (target, site) = workbench_document();
            assert!(target.is_none() && site.is_none());
            assert!(request(PluginRequest::PushInfo("probe".into())).is_err());
        })).join().unwrap();
        assert_eq!(target().unwrap().tab_id, 130046);
        drop(restore);
    }

    #[test]
    fn command_worker_keeps_the_originating_drawing_and_site() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::mpsc;
        use std::time::Duration;
        struct Sender;
        impl PluginRequestSender for Sender {
            fn request(&self, _: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
                Ok(PluginResponse::Ok)
            }
        }
        let drawing = |tab_id, name: &str| Target {
            sender: Arc::new(Sender), tab_id, unit_factor: 1.0,
            doc_path: Some(PathBuf::from(name)),
        };
        let mut first_site = Site::default();
        first_site.label = "Oorspronkelijk project".into();
        let mut other_site = Site::default();
        other_site.label = "Ander project".into();
        let (resume, paused) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        with_command_context(drawing(120001, "/werkmap/a.dwg"), Some(first_site), None, || {
            spawn_command(move || {
                paused.recv_timeout(Duration::from_secs(5)).unwrap();
                done.send((target().unwrap().tab_id, site().unwrap().label, beside_drawing("export.csv"))).unwrap();
            });
        });
        with_command_context(drawing(120002, "/buiten/b.dwg"), Some(other_site), None, || {
            resume.send(()).unwrap();
            let (tab, label, path) = finished.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(tab, 120001);
            assert_eq!(label, "Oorspronkelijk project");
            assert_eq!(path, PathBuf::from("/werkmap/a_export.csv"));
            assert_eq!(target().unwrap().tab_id, 120002);
        });
    }
    #[test]
    fn a_site_worker_does_not_change_a_later_empty_drawing() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::mpsc;
        use std::time::Duration;
        struct Restore(Option<Shared>);
        impl Drop for Restore {
            fn drop(&mut self) { *shared().lock().unwrap_or_else(|e| e.into_inner()) = self.0.take().unwrap(); }
        }
        let _restore = Restore(Some(std::mem::take(&mut *shared().lock().unwrap())));
        struct Writer(Arc<Mutex<Vec<String>>>);
        impl PluginRequestSender for Writer {
            fn request(&self, req: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
                match req {
                    PluginRequest::UpdateXRecord { name, entries, .. } => {
                        assert_eq!(name, "SITE");
                        for e in entries {
                            if let XRecordValue::String(value) = e.value { self.0.lock().unwrap().push(value); }
                        }
                        Ok(PluginResponse::XRecordUpdateResult(Ok(Handle::from(1u64))))
                    }
                    PluginRequest::BeginUndoGroup { .. } | PluginRequest::EndUndoGroup => Ok(PluginResponse::Ok),
                    PluginRequest::SetGeoLocation { .. } => Ok(PluginResponse::GeoLocationSet(Ok(Handle::from(2u64)))),
                    PluginRequest::RemoveGeoLocation => Ok(PluginResponse::GeoLocationRemoved(Ok(false))),
                    PluginRequest::GetSystemVariable { name } if name == "INSUNITS" => Ok(PluginResponse::SystemVariable(Some(ocs_plugin_api::host::HostSettingValue::Number(6.0)))),
                    other => panic!("onverwacht verzoek: {other:?}"),
                }
            }
        }
        let writes = Arc::new(Mutex::new(Vec::new()));
        let sender = Arc::new(Writer(writes.clone()));
        let drawing = |tab_id| Target { sender: sender.clone(), tab_id, unit_factor: 1.0, doc_path: None };
        {
            let mut s = shared().lock().unwrap();
            s.target = Some(drawing(120004));
            s.site = Some(Site::default());
        }
        let (resume, paused) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        spawn_command(move || {
            paused.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut updated = Site::default();
            updated.label = "Alleen project A".into();
            save_site(&updated).unwrap();
            done.send(site().unwrap().label).unwrap();
        });
        {
            let mut s = shared().lock().unwrap();
            s.target = Some(drawing(120005));
            s.site = None;
        }
        resume.send(()).unwrap();
        let worker_site = finished.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(writes.lock().unwrap().len(), 1, "A is opgeslagen");
        let (active_tab, active_site) = {
            let s = shared().lock().unwrap();
            (s.target.as_ref().unwrap().tab_id, s.site.clone())
        };
        assert_eq!(active_tab, 120005);
        assert!(active_site.is_none(), "lege tekening B kreeg A's projectlocatie");
        assert_eq!(worker_site, "Alleen project A", "de worker ziet zijn opgeslagen locatie");
    }

    #[test]
    fn an_earlier_adoption_cannot_replace_a_completed_explicit_site() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        use std::sync::mpsc;
        use std::time::Duration;
        struct Writer(Arc<Mutex<Vec<Site>>>);
        impl PluginRequestSender for Writer {
            fn request(&self, req: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
                match req {
                    PluginRequest::UpdateXRecord { entries, .. } => {
                        for e in entries {
                            if let XRecordValue::String(value) = e.value { self.0.lock().unwrap().push(serde_json::from_str(&value).unwrap()); }
                        }
                        Ok(PluginResponse::XRecordUpdateResult(Ok(Handle::from(1u64))))
                    }
                    PluginRequest::BeginUndoGroup { .. } | PluginRequest::EndUndoGroup => Ok(PluginResponse::Ok),
                    PluginRequest::SetGeoLocation { .. } => Ok(PluginResponse::GeoLocationSet(Ok(Handle::from(2u64)))),
                    PluginRequest::RemoveGeoLocation => Ok(PluginResponse::GeoLocationRemoved(Ok(false))),
                    PluginRequest::GetSystemVariable { name } if name == "INSUNITS" => Ok(PluginResponse::SystemVariable(Some(ocs_plugin_api::host::HostSettingValue::Number(6.0)))),
                    other => panic!("onverwacht verzoek: {other:?}"),
                }
            }
        }
        let writes = Arc::new(Mutex::new(Vec::new()));
        let target = Target { sender: Arc::new(Writer(writes.clone())), tab_id: 120006, unit_factor: 1.0, doc_path: None };
        let adoption_target = target.clone();
        let (started, entered) = mpsc::channel();
        let (resume, paused) = mpsc::channel();
        let worker = std::thread::spawn(move || with_command_context(adoption_target, None, None, || {
            let mut old = Site::default();
            old.label = "Oud GEODATA-project".into();
            started.send(()).unwrap();
            paused.recv_timeout(Duration::from_secs(5)).unwrap();
            adopt_site(&old).unwrap()
        }));
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        with_command_context(target.clone(), None, None, || {
            let mut requested = Site::default();
            requested.label = "Expliciet gekozen project".into();
            save_site(&requested).unwrap();
        });
        resume.send(()).unwrap();
        assert!(!worker.join().unwrap(), "late GEODATA-adoptie had de expliciete keuze moeten behouden");
        {
            let values = writes.lock().unwrap();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].label, "Expliciet gekozen project");
        }
        let mut fresh = target;
        fresh.tab_id = 120007;
        with_command_context(fresh, None, None, || {
            let mut automatic = Site::default();
            automatic.label = "Eerste automatische locatie".into();
            assert!(adopt_site(&automatic).unwrap(), "een nieuwe tekening mag GEODATA overnemen");
            assert!(!adopt_site(&automatic).unwrap(), "adoptie initialiseert een tekening maar eenmaal");
            let mut explicit = Site::default();
            explicit.label = "Nieuwe expliciete locatie".into();
            save_site(&explicit).unwrap();
            assert_eq!(site().unwrap().label, explicit.label);
        });
        assert_eq!(writes.lock().unwrap().len(), 3);
    }


    /// The GEODATA of a drawing as Open CAD Studio v2026.46 keeps it
    /// (`src/io/crs.rs`, `HostSession::set_geo_location`): a CRS outside its
    /// registry is refused before anything changes; a new object takes its
    /// unit scale from INSUNITS, an existing one keeps its scale.
    struct Drawing { unit_scale: f64, geodata: Option<(ocs_plugin_api::host::GeoLocation, f64)>, site: Option<Site> }
    /// Every request one drawing receives, answered from `drawing`.
    struct Recorder { seen: Arc<Mutex<Vec<PluginRequest>>>, drawing: Arc<Mutex<Drawing>> }
    impl PluginRequestSender for Recorder {
        fn request(&self, req: PluginRequest) -> Result<PluginResponse, ocs_plugin_api::host::PluginRequestError> {
            let mut d = self.drawing.lock().unwrap();
            let answer = match &req {
                PluginRequest::UpdateXRecord { dictionary_path, name, entries } => {
                    assert_eq!(dictionary_path, &["OPENINFRA"]);
                    assert_eq!(name, "SITE");
                    let XRecordValue::String(json) = &entries[0].value else { panic!("SITE is JSON") };
                    d.site = Some(serde_json::from_str(json).unwrap());
                    PluginResponse::XRecordUpdateResult(Ok(Handle::from(1u64)))
                }
                PluginRequest::SetGeoLocation { location } if location.coordinate_system != "EPSG:28992" => {
                    PluginResponse::GeoLocationSet(Err(format!("{} is not in the embedded CRS registry", location.coordinate_system)))
                }
                PluginRequest::SetGeoLocation { location } => {
                    let scale = d.geodata.as_ref().map(|(_, s)| *s).unwrap_or(d.unit_scale);
                    d.geodata = Some((location.clone(), scale));
                    PluginResponse::GeoLocationSet(Ok(Handle::from(7u64)))
                }
                PluginRequest::RemoveGeoLocation => PluginResponse::GeoLocationRemoved(Ok(d.geodata.take().is_some())),
                PluginRequest::GetSystemVariable { name } if name == "INSUNITS" => {
                    let insunits = if d.unit_scale == 0.001 { 4.0 } else { 6.0 };
                    PluginResponse::SystemVariable(Some(ocs_plugin_api::host::HostSettingValue::Number(insunits)))
                }
                PluginRequest::BeginUndoGroup { .. } | PluginRequest::EndUndoGroup | PluginRequest::PushError(_) => PluginResponse::Ok,
                other => panic!("onverwacht verzoek: {other:?}"),
            };
            self.seen.lock().unwrap().push(req);
            Ok(answer)
        }
    }

    fn rd_site(absolute: bool) -> Site {
        let mut site = Site::default();
        site.crs = "EPSG:28992".into();
        site.set_origin(104885.0, 425330.0);
        site.absolute = absolute;
        site
    }

    /// Save `site` in a drawing of `unit_factor` units per metre; the requests it sent.
    fn save_in(drawing: &Arc<Mutex<Drawing>>, tab_id: u64, unit_factor: f64, site: &Site) -> Vec<PluginRequest> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sender = Arc::new(Recorder { seen: seen.clone(), drawing: drawing.clone() });
        with_command_context(Target { sender, tab_id, unit_factor, doc_path: None }, None, None, || {
            save_site(site).expect("de projectlocatie blijft opgeslagen");
            assert_eq!(super::site().as_ref(), Some(site));
            assert_eq!(target().unwrap().unit_factor, 1.0 / drawing.lock().unwrap().unit_scale, "vervolgwerk gebruikt de actuele eenheden");
        });
        let requests = std::mem::take(&mut *seen.lock().unwrap());
        requests
    }

    #[test]
    fn a_saved_site_is_also_the_drawings_geodata() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (tab_id, absolute, unit_factor) in [(120020, true, 1000.0), (120021, false, 1.0)] {
            let drawing = Arc::new(Mutex::new(Drawing { unit_scale: 1.0 / unit_factor, geodata: None, site: None }));
            let seen = save_in(&drawing, tab_id, unit_factor, &rd_site(absolute));
            let kinds: Vec<String> = seen.iter().map(kind).collect();
            assert_eq!(kinds, ["BeginUndoGroup", "UpdateXRecord", "GetSystemVariable", "RemoveGeoLocation", "SetGeoLocation", "EndUndoGroup"], "één ongedaanmaakstap voor record en GEODATA");
            let design = if absolute { [104885.0 * unit_factor, 425330.0 * unit_factor, 0.0] } else { [0.0; 3] };
            let expected = ocs_plugin_api::host::GeoLocation { coordinate_system: "EPSG:28992".into(), design_point: design, reference_point: [104885.0, 425330.0, 0.0], north_direction: [0.0, 1.0] };
            assert_eq!(drawing.lock().unwrap().geodata, Some((expected, 1.0 / unit_factor)));
        }
    }

    #[test]
    fn geodata_follows_a_change_of_drawing_units() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let drawing = Arc::new(Mutex::new(Drawing { unit_scale: 0.001, geodata: None, site: None }));
        save_in(&drawing, 120023, 1000.0, &rd_site(false));
        // INSUNITS millimetres → metres, then the location is stored again.
        drawing.lock().unwrap().unit_scale = 1.0;
        save_in(&drawing, 120023, 1.0, &rd_site(false));
        let scale = drawing.lock().unwrap().geodata.as_ref().map(|(_, s)| *s);
        assert_eq!(scale, Some(1.0), "GEODATA hield de schaal van millimeters");
    }

    #[test]
    fn an_absolute_site_uses_current_units_even_with_a_stale_target() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let drawing = Arc::new(Mutex::new(Drawing { unit_scale: 0.001, geodata: None, site: None }));
        let site = rd_site(true);
        save_in(&drawing, 120024, 1000.0, &site);
        // The location window keeps the target cached at attach. Only the
        // host's INSUNITS changes from mm to m before its next OK click.
        drawing.lock().unwrap().unit_scale = 1.0;
        save_in(&drawing, 120024, 1000.0, &site);
        let d = drawing.lock().unwrap();
        let (location, scale) = d.geodata.as_ref().unwrap();
        assert_eq!(location.design_point, [104885.0, 425330.0, 0.0], "het basispunt gebruikt de verouderde cache");
        assert_eq!(*scale, 1.0);
        assert_eq!(d.site.as_ref(), Some(&site));
    }

    #[test]
    fn a_refused_geodata_keeps_the_site_and_leaves_no_old_location() {
        let _one = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let drawing = Arc::new(Mutex::new(Drawing { unit_scale: 1.0, geodata: None, site: None }));
        save_in(&drawing, 120022, 1.0, &rd_site(true));
        assert!(drawing.lock().unwrap().geodata.is_some(), "locatie A staat in GEODATA");
        let mut b = rd_site(true);
        b.crs = "EPSG:9999".into();
        b.label = "Weigering".into();
        let seen = save_in(&drawing, 120022, 1.0, &b);
        assert!(drawing.lock().unwrap().geodata.is_none(), "GEODATA van locatie A bleef staan naast SITE B");
        assert_eq!(drawing.lock().unwrap().site.as_ref(), Some(&b), "SITE B blijft volledig bewaard");
        let kinds: Vec<String> = seen.iter().map(kind).collect();
        assert_eq!(kinds, ["BeginUndoGroup", "UpdateXRecord", "GetSystemVariable", "RemoveGeoLocation", "SetGeoLocation", "EndUndoGroup", "PushError"]);
        let Some(PluginRequest::UpdateXRecord { entries, .. }) = seen.get(1) else { unreachable!() };
        assert!(matches!(&entries[0].value, XRecordValue::String(v) if v.contains("Weigering")), "SITE B is geschreven");
        let Some(PluginRequest::PushError(warning)) = seen.last() else { unreachable!() };
        assert!(warning.contains("EPSG:9999") && warning.contains("GEODATA"), "{warning}");
    }
}
