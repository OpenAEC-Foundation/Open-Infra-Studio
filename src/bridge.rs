//! Talking to Open CAD Studio from worker and window threads.
//!
//! `dispatch` (the host's call into the plugin) hands us a
//! `PluginRequestSender` for the active document tab; the workbench window and
//! import workers use it to add layers and entities while the host stays
//! responsive. Each import is one undo step.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use ocs_plugin_api::host::{
    codec::{Color, EntityType, Handle, LineWeight},
    ExtendedDataRecord, HostApi, LayerConfig, PluginRequestSender, TableOperation, XDataValue, XRecordEntry, XRecordValue,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};

use crate::gis::site::{unit_factor_for_insunits, Site};

/// The document the workbench currently works on.
#[derive(Clone)]
pub struct Target {
    pub sender: Arc<dyn PluginRequestSender>,
    pub tab_id: u64,
    pub unit_factor: f64,
    pub doc_path: Option<PathBuf>,
}

#[derive(Default)]
struct Shared {
    target: Option<Target>,
    site: Option<Site>,
    /// NLCS layers and linetypes already set up, per tab.
    made: HashSet<(u64, String)>,
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
    {
        let mut s = shared().lock().unwrap();
        let same_tab = s.target.as_ref().is_some_and(|t| t.tab_id == tab_id);
        let unit_factor = s.target.as_ref().filter(|_| same_tab).map(|t| t.unit_factor).unwrap_or(1.0);
        s.target = Some(Target { sender: Arc::from(sender), tab_id, unit_factor, doc_path });
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
    // With the target in place the units can be asked for without the drawing.
    let f = units(host, tab_id, false);
    if let Some(t) = shared().lock().unwrap().target.as_mut() {
        t.unit_factor = f;
    }
}

/// Drawing units → metres factor for the tab. Open CAD Studio's plugin API
/// does not hand out INSUNITS (v2026.43 knows CLAYER, SNAPANG and CTAB) and
/// the drawing header only comes with the whole drawing, which a large
/// drawing cannot send. So: a cache per tab; Open Infra's own record in the
/// drawing (OPENINFRA/UNITS); the header only when the drawing is known to be
/// small; metres otherwise. `fresh` reads again (a new project location).
pub fn units(host: &mut dyn HostApi, tab_id: u64, fresh: bool) -> f64 {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<u64, f64>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if !fresh {
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
    let f = match host.system_variable("INSUNITS") {
        Some(ocs_plugin_api::host::HostSettingValue::Number(n)) => Some(unit_factor_for_insunits(n as i64)),
        Some(ocs_plugin_api::host::HostSettingValue::Text(t)) => Some(unit_factor_for_insunits(t.trim().parse().unwrap_or(0))),
        None => None,
    };
    let f = match f.or_else(|| if fresh { None } else { recorded() }) {
        Some(f) => f,
        // The header, but only from a drawing small enough to send whole.
        None => match view_entities() {
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
    shared().lock().unwrap().target.clone()
}

pub fn site() -> Option<Site> {
    shared().lock().unwrap().site.clone()
}

/// Store the site in memory and in the drawing (NOD XRecord, round-trips DWG).
pub fn save_site(site: &Site) -> Result<(), String> {
    shared().lock().unwrap().site = Some(site.clone());
    let t = target().ok_or("geen tekening actief")?;
    let json = serde_json::to_string(site).map_err(|e| e.to_string())?;
    let entries = vec![XRecordEntry::string(1, json)];
    let path: Vec<String> = SITE_DICT.iter().map(|s| s.to_string()).collect();
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: SITE_RECORD.into(), entries: entries.clone() };
    if let Ok(PluginResponse::XRecordUpdateResult(Ok(_))) = t.sender.request(update) {
        return Ok(());
    }
    // Not there yet: create it (and the OPENINFRA dictionary).
    match t.sender.request(PluginRequest::CreateXRecord { dictionary_path: path, name: SITE_RECORD.into(), entries }) {
        Ok(PluginResponse::XRecordCreateResult(Ok(_))) => Ok(()),
        Ok(PluginResponse::XRecordCreateResult(Err(e))) | Ok(PluginResponse::Error(e)) => Err(e),
        Ok(other) => Err(format!("onverwacht antwoord: {other:?}")),
        Err(e) => Err(e.to_string()),
    }
}

fn request(req: PluginRequest) -> Result<PluginResponse, String> {
    let t = target().ok_or("geen tekening actief — klik eerst in Open CAD Studio op het GIS-tabblad")?;
    match t.sender.request(req) {
        Ok(PluginResponse::Error(e)) => Err(e),
        Ok(r) => Ok(r),
        Err(e) => Err(e.to_string()),
    }
}

pub fn info(msg: &str) {
    let _ = request(PluginRequest::PushInfo(format!("[Open Infra] {msg}")));
    reply(true, msg);
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

/// Answer the waiting script now (argument errors, commands that finish at once).
pub fn reply(ok: bool, msg: &str) {
    if let Some(id) = pending_reply().lock().unwrap().take() {
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
    let _ = request(PluginRequest::AddLayer(LayerConfig { name: name.into(), color: Some(color), ..Default::default() }));
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
        if !s.made.insert((tab, format!("LA:{}", spec.name))) {
            return;
        }
        s.nlcs_layers.push((tab, spec.clone()));
    }
    let linetype = ensure_linetype(tab, &spec.linetype);
    let config = |linetype: Option<String>| LayerConfig {
        name: spec.name.clone(),
        color: Some(match spec.true_color {
            Some((r, g, b)) => Color::Rgb { r, g, b },
            None => Color::Index(spec.color.max(1)),
        }),
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

/// Define an NLCS linetype in the drawing when it lacks it (shape and text
/// elements left out). `None` for CONTINUOUS and names that cannot be made.
fn ensure_linetype(tab: u64, name: &str) -> Option<String> {
    let pattern = crate::nlcs::linetype_pattern(name)?;
    if !shared().lock().unwrap().made.insert((tab, format!("LT:{name}"))) {
        return Some(name.to_string());
    }
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
/// with OCS_V4_SNAPSHOT_SEGMENT_SIZE).
fn view_entities() -> Option<(Vec<EntityType>, Vec<String>, usize)> {
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
    let (entities, layers, _) = view_entities()?;
    Some(document_of(entities, &layers))
}

/// The whole drawing of the active tab, read from a worker or window thread.
/// A small drawing comes as a full snapshot (with the layer table); a large
/// one from the shared view, whose layers have names only (see [`partial`]).
/// A drawing too large for both gives an error, never a stalled request.
pub fn snapshot() -> Result<ocs_plugin_api::host::codec::CadDocument, String> {
    match view_entities() {
        Some((_, _, size)) if size < SNAPSHOT_SAFE => {
            PARTIAL.store(false, std::sync::atomic::Ordering::Relaxed);
            match request(PluginRequest::DocumentSnapshot)? {
                PluginResponse::Document(doc) => Ok(*doc),
                other => Err(format!("onverwacht antwoord op de tekeningopvraag: {other:?}")),
            }
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
    let tab = target().map(|t| t.tab_id).ok_or("geen tekening actief")?;
    let linetype = ensure_linetype(tab, &spec.linetype).or_else(|| Some("Continuous".into()));
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
    request(PluginRequest::PushUndo { label: label.into() })?;
    let mut n = 0;
    for h in handles {
        if request(PluginRequest::RemoveEntity { handle: *h }).is_ok() {
            n += 1;
        }
    }
    let _ = request(PluginRequest::SetDirty);
    let _ = request(PluginRequest::BumpGeometry);
    Ok(n)
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

/// Like [`add_entities`], returning the new handles.
pub fn add_entities_handles(label: &str, entities: Vec<EntityType>) -> Result<Vec<Handle>, String> {
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    // Imports run on their own threads: one at a time keeps each its own
    // undo step.
    static WRITING: Mutex<()> = Mutex::new(());
    let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    request(PluginRequest::PushUndo { label: label.into() })?;
    let mut handles = Vec::with_capacity(entities.len());
    // Batches well under the host's 64 MB message limit: meshes carry a
    // full record per vertex and face, so count bytes, not entities.
    let mut chunk: Vec<EntityType> = Vec::new();
    let mut bytes = 0usize;
    let send = |chunk: &mut Vec<EntityType>, handles: &mut Vec<Handle>| -> Result<(), String> {
        if !chunk.is_empty() {
            if let PluginResponse::Handles(h) = request(PluginRequest::AddEntities(std::mem::take(chunk)))? {
                handles.extend(h);
            }
        }
        Ok(())
    };
    for e in entities {
        let size = estimated_size(&e);
        if !chunk.is_empty() && (bytes + size > BATCH_BYTES || chunk.len() >= 2000) {
            send(&mut chunk, &mut handles)?;
            bytes = 0;
        }
        bytes += size;
        chunk.push(e);
    }
    send(&mut chunk, &mut handles)?;
    let _ = request(PluginRequest::SetDirty);
    // Many fast additions can leave the host's draw cache partly stale
    // (seen with an image underlay hiding newer lines): force a rebuild.
    let _ = request(PluginRequest::BumpGeometry);
    Ok(handles)
}

/// Map images are underlays: send them behind the vector data.
pub fn send_to_back(handles: &[Handle]) {
    if handles.is_empty() {
        return;
    }
    if request(PluginRequest::SetSelection { handles: handles.to_vec() }).is_ok() {
        let _ = request(PluginRequest::ExecuteCommand("DRAWORDER_BACK".into()));
        let _ = request(PluginRequest::SetSelection { handles: Vec::new() });
        let _ = request(PluginRequest::BumpGeometry);
    }
}

/// A new map image behind the drawing's vectors, but on top of the images
/// already there (newest map on top: a cadastre overlay over the photo).
pub fn image_to_back(new: &[Handle]) {
    send_to_back(new);
    let older: Vec<Handle> = view()
        .map(|d| {
            d.model_space_entities()
                .filter(|e| matches!(e, EntityType::RasterImage(_)))
                .map(|e| e.common().handle)
                .filter(|h| !new.contains(h))
                .collect()
        })
        .unwrap_or_default();
    send_to_back(&older);
}

/// Run a command line in the active drawing (tokens answer its prompts).
pub fn execute(command: &str) -> Result<(), String> {
    match request(PluginRequest::ExecuteCommand(command.into()))? {
        PluginResponse::Bool(false) => Err(format!("Open CAD Studio voerde '{}' niet uit", command.split_whitespace().next().unwrap_or(""))),
        _ => Ok(()),
    }
}

pub fn zoom_extents() {
    let _ = request(PluginRequest::ExecuteCommand("ZOOMEXTENTS".into()));
}

/// A file beside the drawing (`<tekening>_<suffix>`), or in the plugin's
/// config folder while the drawing has not been saved.
pub fn beside_drawing(suffix: &str) -> PathBuf {
    match target().and_then(|t| t.doc_path) {
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
