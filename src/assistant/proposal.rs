//! A pending proposal: an assistant's change the user accepts or rejects.
//!
//! The proposed entities stay on their own layers (later steps build on
//! them: contours on a proposed terrain, a corridor on a proposed axis), and
//! a frame with a label on [`MARKER_LAYER`] shows where the proposal is.
//! The drawing keeps, per proposal, two XRecords under `OPENINFRA/PROPOSALS`:
//!
//! - `P<n>`: the record (JSON): who, what, the handles added, modified and
//!   removed, the markers, and a fingerprint of the proposed entities;
//! - `P<n>-STATE`: the entities as they were before the change (bincode,
//!   deflate, base64), to restore them on reject.
//!
//! Reject removes what was added and puts back what was modified or removed,
//! with their own handles, from that stored state: it does not depend on the
//! host's undo, so the user's other edits in between stay. A proposal whose
//! entities changed since (the fingerprint differs) is stale: it can only be
//! accepted or dismissed.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use base64::Engine as _;
use ocs_plugin_api::host::codec::{Color, EntityType, Handle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::activity::{self, Entry, State};
use super::capture::{Change, Finished};
use crate::bridge;
use crate::civiel::api::{handle_text, parse_handle};
use crate::lang::tr;

/// Dictionary of the proposals in the drawing.
pub const DICT: &[&str] = &["OPENINFRA", "PROPOSALS"];
/// Layer of the frames and labels that mark pending proposals.
pub const MARKER_LAYER: &str = "OI-ASSISTANT-PROPOSAL";
/// Stored state above this (characters of base64) is not kept: such a
/// proposal can be accepted or dismissed, not rejected.
pub const MAX_STATE: usize = 16_000_000;
const STATE_SUFFIX: &str = "-STATE";
/// The last number given (an XRecord beside the proposals).
const COUNTER: &str = "COUNTER";
const FORMAT: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub created: String,
    pub client: String,
    pub tool: String,
    #[serde(default)]
    pub class: String,
    pub command: String,
    #[serde(default)]
    pub arguments: Value,
    pub title_nl: String,
    pub title_en: String,
    pub message: String,
    pub ok: bool,
    /// Handles (hex) the change added, modified and removed.
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    /// The frame and label on [`MARKER_LAYER`].
    pub markers: Vec<String>,
    /// Fingerprint (hex) of the added and modified entities as proposed.
    pub fingerprint: String,
    /// Extent in drawing coordinates: [xmin, ymin, xmax, ymax].
    pub bbox: Option<[f64; 4]>,
    /// The state before is stored: reject is possible.
    pub restorable: bool,
    /// What the change did to the object index, to undo on reject.
    #[serde(default)]
    pub index: Vec<IndexChange>,
    #[serde(default)]
    pub format: u32,
}

/// One key of the object index: the handles a change added and removed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexChange {
    pub kind: String,
    pub key: String,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

impl Proposal {
    pub fn title(&self) -> &str {
        tr(&self.title_nl, &self.title_en)
    }

    /// What the command's answer carries about it.
    pub fn summary(&self) -> Value {
        json!({
            "id": self.id, "state": "pending", "title": self.title_en, "added": self.added.len(),
            "modified": self.modified.len(), "removed": self.removed.len(), "restorable": self.restorable,
            "handles": self.added,
        })
    }

    fn handles(list: &[String]) -> Vec<Handle> {
        list.iter().filter_map(|h| parse_handle(h).ok()).collect()
    }

    pub fn added_handles(&self) -> Vec<Handle> {
        Self::handles(&self.added)
    }

    pub fn modified_handles(&self) -> Vec<Handle> {
        Self::handles(&self.modified)
    }

    pub fn removed_handles(&self) -> Vec<Handle> {
        Self::handles(&self.removed)
    }

    pub fn marker_handles(&self) -> Vec<Handle> {
        Self::handles(&self.markers)
    }

    pub fn index_diff(&self) -> bridge::IndexDiff {
        self.index.iter().map(|c| (c.kind.clone(), c.key.clone(), Self::handles(&c.added), Self::handles(&c.removed))).collect()
    }

    /// The entities whose state the fingerprint covers: added and modified.
    pub fn watched(&self) -> Vec<Handle> {
        let mut v = self.added_handles();
        v.extend(self.modified_handles());
        v
    }
}

// ── Fingerprint ─────────────────────────────────────────────────────────────

/// 64-bit FNV-1a.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= u64::from(*x);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Fields that say where an entity sits in the file, not what it is: they
/// change when the drawing is saved and opened again.
fn bookkeeping(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("handle")
        || k.starts_with("owner")
        || k.starts_with("xdictionary")
        || matches!(k.as_str(), "reactors" | "raw_record" | "r12_metadata" | "graphic_data" | "entity_mode" | "material_flags" | "shadow_flags" | "plotstyle_flags" | "extended_data")
}

fn walk(v: &Value, h: &mut Fnv) {
    walk_in(v, "", h)
}

/// `parent`: the key this value sits under. A sub-entity's own layer (the
/// vertices of a 3D polyline) follows its entity's and is not saved as such.
fn walk_in(v: &Value, parent: &str, h: &mut Fnv) {
    match v {
        Value::Null => h.bytes(b"n"),
        Value::Bool(b) => h.bytes(if *b { b"t" } else { b"f" }),
        Value::Number(n) => {
            // A micro-unit grid: the same coordinates read back from a file.
            let r = (n.as_f64().unwrap_or(0.0) * 1e6).round();
            let r = if r == 0.0 { 0.0 } else { r };
            h.bytes(b"#");
            h.bytes(&r.to_le_bytes());
        }
        Value::String(s) => {
            h.bytes(b"s");
            h.bytes(s.as_bytes());
            h.bytes(&[0]);
        }
        Value::Array(a) => {
            h.bytes(b"[");
            h.bytes(&(a.len() as u64).to_le_bytes());
            for x in a {
                walk_in(x, parent, h);
            }
        }
        Value::Object(m) => {
            h.bytes(b"{");
            for (k, x) in m {
                if bookkeeping(k) || (k == "layer" && parent != "common") {
                    continue;
                }
                h.bytes(k.as_bytes());
                h.bytes(b":");
                walk_in(x, k, h);
            }
            h.bytes(b"}");
        }
    }
}

/// What an entity is, independent of its handle and of how the file stores
/// it: kind, layer, style, geometry, and Open Infra's object data.
pub fn fingerprint(e: &EntityType) -> u64 {
    let mut h = Fnv::new();
    walk(&serde_json::to_value(e).unwrap_or(Value::Null), &mut h);
    if let Some(r) = e.common().extended_data.get_record(bridge::XDATA_APP) {
        walk(&serde_json::to_value(r).unwrap_or(Value::Null), &mut h);
    }
    h.0
}

/// One fingerprint for a set of entities, by handle; a missing one counts.
pub fn combined(items: &[(Handle, Option<u64>)]) -> u64 {
    let mut sorted: Vec<&(Handle, Option<u64>)> = items.iter().collect();
    sorted.sort_by_key(|(h, _)| u64::from(*h));
    let mut h = Fnv::new();
    for (handle, fp) in sorted {
        h.bytes(&u64::from(*handle).to_le_bytes());
        match fp {
            Some(f) => h.bytes(&f.to_le_bytes()),
            None => h.bytes(b"missing"),
        }
    }
    h.0
}

// ── Stored state ────────────────────────────────────────────────────────────

/// The entities as text pieces for an XRecord: bincode, deflate, base64.
pub fn encode_state(entities: &[EntityType]) -> Result<Vec<String>, String> {
    let raw = bincode::serialize(entities).map_err(|e| e.to_string())?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).map_err(|e| e.to_string())?;
    let packed = z.finish().map_err(|e| e.to_string())?;
    let text = base64::engine::general_purpose::STANDARD.encode(packed);
    Ok(bridge::text_chunks(&text, 2000))
}

pub fn decode_state(chunks: &[String]) -> Result<Vec<EntityType>, String> {
    let text: String = chunks.concat();
    let packed = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(packed.as_slice()).read_to_end(&mut raw).map_err(|e| e.to_string())?;
    bincode::deserialize(&raw).map_err(|e| e.to_string())
}

/// The next free proposal number.
pub fn next_id(existing: &[String]) -> String {
    let n = existing.iter().filter_map(|s| s.strip_prefix('P').and_then(|r| r.split('-').next()).and_then(|r| r.parse::<u64>().ok())).max().unwrap_or(0);
    format!("P{}", n + 1)
}

// ── Restore ─────────────────────────────────────────────────────────────────

/// What a reject sends to the host.
#[derive(Debug, Default)]
pub struct RestorePlan {
    /// The added entities and the markers that are still there.
    pub remove: Vec<Handle>,
    /// Removed entities, back with their own handles.
    pub add: Vec<EntityType>,
    /// Modified entities, back in their state before.
    pub update: Vec<EntityType>,
}

/// The restore of proposal `p` from its stored state; `exists` tells which
/// handles are in the drawing now.
pub fn restore_plan(p: &Proposal, originals: Vec<EntityType>, exists: &dyn Fn(Handle) -> bool) -> Result<RestorePlan, String> {
    let removed: HashSet<Handle> = p.removed_handles().into_iter().collect();
    let modified: HashSet<Handle> = p.modified_handles().into_iter().collect();
    let mut plan = RestorePlan::default();
    let mut seen = HashSet::new();
    for e in originals {
        let h = e.common().handle;
        seen.insert(h);
        if removed.contains(&h) {
            if exists(h) {
                return Err(format!("{} {}", tr("de handle is weer in gebruik:", "the handle is in use again:"), handle_text(h)));
            }
            plan.add.push(e);
        } else if modified.contains(&h) {
            // Deleted since: it comes back as it was.
            if exists(h) { plan.update.push(e) } else { plan.add.push(e) }
        }
    }
    if let Some(h) = removed.iter().chain(modified.iter()).find(|h| !seen.contains(h)) {
        return Err(format!("{} {}", tr("de opgeslagen toestand mist", "the stored state lacks"), handle_text(*h)));
    }
    plan.remove = p.added_handles().into_iter().chain(p.marker_handles()).filter(|h| exists(*h)).collect();
    Ok(plan)
}

// ── Status ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Status {
    /// The proposed entities changed since (or are gone).
    pub stale: bool,
    pub missing: usize,
    /// Later pending proposals that changed these entities.
    pub changed_by: Vec<String>,
}

/// The status of `p` from the current state of its watched entities.
pub fn status_of(p: &Proposal, current: &[(Handle, Option<u64>)], later: &[Proposal]) -> Status {
    let missing = current.iter().filter(|(_, f)| f.is_none()).count();
    let stale = format!("{:016X}", combined(current)) != p.fingerprint;
    let watched: HashSet<Handle> = p.watched().into_iter().collect();
    let changed_by = if stale {
        later
            .iter()
            .filter(|q| q.id != p.id && number(&q.id) > number(&p.id))
            .filter(|q| q.modified_handles().into_iter().chain(q.removed_handles()).any(|h| watched.contains(&h)))
            .map(|q| q.id.clone())
            .collect()
    } else {
        Vec::new()
    };
    Status { stale, missing, changed_by }
}

fn number(id: &str) -> u64 {
    id.trim_start_matches('P').parse().unwrap_or(0)
}

/// The fingerprints of these entities as they are now.
fn current(handles: &[Handle]) -> Result<Vec<(Handle, Option<u64>)>, String> {
    let mut out = Vec::with_capacity(handles.len());
    bridge::for_each_entity_batch(handles, 16, &mut |hs, es| {
        out.extend(hs.iter().copied().zip(es.iter().map(|e| e.as_ref().map(fingerprint))));
    })?;
    Ok(out)
}

/// The status of a stored proposal (reads its entities).
pub fn status(p: &Proposal, all: &[Proposal]) -> Result<Status, String> {
    Ok(status_of(p, &current(&p.watched())?, all))
}

// ── Store ───────────────────────────────────────────────────────────────────

/// The pending proposals of the drawing, oldest first.
pub fn list() -> Vec<Proposal> {
    let mut out: Vec<Proposal> = bridge::xrecord_names(DICT)
        .into_iter()
        .filter(|n| !n.ends_with(STATE_SUFFIX) && n != COUNTER)
        .filter_map(|n| bridge::xrecord_strings(DICT, &n))
        .filter_map(|chunks| serde_json::from_str(&chunks.concat()).ok())
        .collect();
    out.sort_by_key(|p| number(&p.id));
    out
}

pub fn load(id: &str) -> Result<Proposal, String> {
    let id = normalise(id)?;
    let chunks = bridge::xrecord_strings(DICT, &id).ok_or_else(|| format!("{} {id}", tr("geen openstaand voorstel", "no pending proposal")))?;
    serde_json::from_str(&chunks.concat()).map_err(|e| format!("{id}: {e}"))
}

/// "p3", "3" or "P3" → "P3".
pub fn normalise(id: &str) -> Result<String, String> {
    let t = id.trim().trim_start_matches(['P', 'p']);
    t.parse::<u64>().map(|n| format!("P{n}")).map_err(|_| format!("{} '{id}' (P1, P2, …)", tr("geen voorstelnummer:", "not a proposal number:")))
}

/// The next proposal number of the drawing: never one used before, also
/// when the earlier ones were accepted or rejected.
fn take_number() -> String {
    let mut known = bridge::xrecord_names(DICT);
    if let Some(n) = bridge::xrecord_strings(DICT, COUNTER).and_then(|c| c.concat().trim().parse::<u64>().ok()) {
        known.push(format!("P{n}"));
    }
    let id = next_id(&known);
    let _ = bridge::xrecord_write_strings(DICT, COUNTER, vec![number(&id).to_string()]);
    id
}

fn save(p: &Proposal) -> Result<(), String> {
    let json = serde_json::to_string(p).map_err(|e| e.to_string())?;
    bridge::xrecord_write_strings(DICT, &p.id, bridge::text_chunks(&json, 2000))
}

fn forget(id: &str) {
    bridge::xrecord_delete(DICT, &format!("{id}{STATE_SUFFIX}"));
    bridge::xrecord_delete(DICT, id);
}

// ── Making one ──────────────────────────────────────────────────────────────

fn grow(b: &mut Option<[f64; 4]>, e: &EntityType) {
    let bb = e.as_entity().bounding_box();
    let (lo, hi) = (bb.min, bb.max);
    if ![lo.x, lo.y, hi.x, hi.y].iter().all(|v| v.is_finite()) || (lo.x == 0.0 && lo.y == 0.0 && hi.x == 0.0 && hi.y == 0.0) {
        return;
    }
    *b = Some(match *b {
        None => [lo.x, lo.y, hi.x, hi.y],
        Some(c) => [c[0].min(lo.x), c[1].min(lo.y), c[2].max(hi.x), c[3].max(hi.y)],
    });
}

/// The frame and label of a proposal, in drawing units.
pub fn marker_entities(id: &str, title: &str, counts: (usize, usize, usize), bbox: [f64; 4], k: f64) -> Vec<EntityType> {
    let size = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]);
    let pad = (0.02 * size).max(2.0 * k);
    let (x0, y0, x1, y1) = (bbox[0] - pad, bbox[1] - pad, bbox[2] + pad, bbox[3] + pad);
    let data = bridge::object_data([("assistant_proposal", id.to_string())]);
    let mut out = crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], MARKER_LAYER, 1.0, Some(&data));
    let label = format!(
        "{id} · {} · {title} (+{} ~{} −{})",
        tr("voorstel van de assistent, wacht op beoordeling", "assistant proposal, awaiting review"),
        counts.0,
        counts.1,
        counts.2
    );
    // As wide as the frame, so it reads at the zoom that shows the frame;
    // at least 3 mm on paper.
    let chars = label.chars().count().max(1) as f64;
    let text_h = ((x1 - x0) / (chars * 0.6)).min(0.03 * size.max(1.0)).max(3.0 * k);
    let w = chars * text_h * 0.6;
    out.push(crate::emit::text([x0 + w / 2.0, y1 + text_h], &label, text_h, 0.0, MARKER_LAYER, Some(&data)));
    out
}

/// Keep a finished capture as a pending proposal: fingerprint, stored
/// state, markers, records. Runs without capture (see [`super::record`]).
pub fn create(a: &Finished, ok: bool, message: &str, entry: &Entry) -> Result<Proposal, String> {
    let c = &a.changes;
    let modified = c.with(Change::Modified);
    let mut watched = c.added.clone();
    watched.extend(modified.iter().copied());
    // The proposed state, read back as the host keeps it.
    let mut bbox = None;
    let mut prints = Vec::with_capacity(watched.len());
    bridge::for_each_entity_batch(&watched, 16, &mut |hs, es| {
        for (h, e) in hs.iter().zip(es) {
            if let Some(e) = &e {
                grow(&mut bbox, e);
            }
            prints.push((*h, e.as_ref().map(fingerprint)));
        }
    })?;
    let originals: Vec<EntityType> = c.before.iter().filter_map(|b| b.2.clone()).collect();
    for e in &originals {
        grow(&mut bbox, e);
    }
    let complete = originals.len() == c.before.len();
    let state = if complete { encode_state(&originals).ok().filter(|s| s.iter().map(String::len).sum::<usize>() <= MAX_STATE) } else { None };
    let id = take_number();
    let mut p = Proposal {
        id: id.clone(),
        created: entry.at.clone(),
        client: entry.who.clone(),
        tool: a.request.tool.clone(),
        class: a.request.class.clone(),
        command: entry.command.clone(),
        arguments: a.request.arguments.clone(),
        title_nl: entry.title_nl.clone(),
        title_en: entry.title_en.clone(),
        message: super::clip(message, 300),
        ok,
        added: c.added.iter().map(|h| handle_text(*h)).collect(),
        modified: modified.iter().map(|h| handle_text(*h)).collect(),
        removed: c.with(Change::Removed).iter().map(|h| handle_text(*h)).collect(),
        markers: Vec::new(),
        fingerprint: format!("{:016X}", combined(&prints)),
        bbox,
        restorable: state.is_some(),
        index: c.index.iter().map(|(kind, key, added, removed)| IndexChange { kind: kind.clone(), key: key.clone(), added: added.iter().map(|h| handle_text(*h)).collect(), removed: removed.iter().map(|h| handle_text(*h)).collect() }).collect(),
        format: FORMAT,
    };
    // Markers, state and record as one undo step after the command's own.
    let _group = bridge::UndoGroup::begin(&format!("{} {id}", tr("Assistent: voorstel", "Assistant: proposal")));
    if let Some(b) = bbox {
        let k = crate::civiel::settings().nlcs.scale / 1000.0 * bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
        bridge::ensure_layer(MARKER_LAYER, Color::Index(30));
        let title = p.title().to_string();
        let marks = marker_entities(&id, &title, (p.added.len(), p.modified.len(), p.removed.len()), b, k);
        p.markers = bridge::add_entities_handles(&format!("{} {id}", tr("Voorstel", "Proposal")), marks)?.iter().map(|h| handle_text(*h)).collect();
    }
    if let Some(s) = state {
        bridge::xrecord_write_strings(DICT, &format!("{id}{STATE_SUFFIX}"), s)?;
    }
    save(&p)?;
    // Not selected: many commands act on the selection when they get no
    // handle, so a selection left behind would steer the assistant's next step.
    Ok(p)
}

// ── Review ──────────────────────────────────────────────────────────────────

fn review_entry(p: &Proposal, who: &str, state: State, ok: bool, message: String) -> Entry {
    Entry {
        at: super::now(),
        who: who.to_string(),
        tool: match state {
            State::Accepted => "accept",
            State::Rejected => "reject",
            _ => "dismiss",
        }
        .into(),
        class: "review".into(),
        title_nl: format!("{} {}: {}", state.label_nl(), p.id, p.title_nl),
        title_en: format!("{} {}: {}", state.label_en(), p.id, p.title_en),
        command: String::new(),
        message,
        ok,
        added: 0,
        modified: 0,
        removed: 0,
        state,
        proposal: Some(p.id.clone()),
    }
}

/// Keep the change and clear the record (`state`: accepted or dismissed).
fn close(id: &str, who: &str, state: State) -> Result<String, String> {
    let p = load(id)?;
    let _group = bridge::UndoGroup::begin(&format!("{} {}", state.label(), p.id));
    let _ = bridge::remove_entities(tr("Voorstelmarkering", "Proposal marker"), &p.marker_handles());
    forget(&p.id);
    let msg = format!("{} {}: {}", state.label(), p.id, p.title());
    activity::set_state(&p.id, state);
    activity::append(review_entry(&p, who, state, true, msg.clone()));
    crate::civiel::api::note("proposal", json!({"id": p.id, "state": state}));
    crate::ui::send(crate::ui::UiMsg::AssistantChanged);
    Ok(msg)
}

/// Accept: the change stays, the markers and the record go.
pub fn accept(id: &str, who: &str) -> Result<String, String> {
    close(id, who, State::Accepted)
}

/// Dismiss: like accept, for a stale proposal the user settled by hand.
pub fn dismiss(id: &str, who: &str) -> Result<String, String> {
    close(id, who, State::Dismissed)
}

/// Reject: remove what was added, put back what was modified or removed.
pub fn reject(id: &str, who: &str) -> Result<String, String> {
    let p = load(id)?;
    let all = list();
    let st = status(&p, &all)?;
    if st.stale {
        let by = if st.changed_by.is_empty() {
            String::new()
        } else {
            format!(" ({} {})", tr("gewijzigd door het latere voorstel", "changed by the later proposal"), st.changed_by.join(", "))
        };
        return Err(format!(
            "{}: {}{by}",
            p.id,
            tr("de voorgestelde elementen zijn sindsdien bewerkt; accepteer of laat vervallen", "the proposed entities were edited since; accept or dismiss it")
        ));
    }
    if !p.restorable {
        return Err(format!("{}: {}", p.id, tr("de vorige toestand is niet bewaard (te groot of onleesbaar); accepteer of laat vervallen", "the previous state was not kept (too large or unreadable); accept or dismiss it")));
    }
    let originals = if p.modified.is_empty() && p.removed.is_empty() {
        Vec::new()
    } else {
        let chunks = bridge::xrecord_strings(DICT, &format!("{}{STATE_SUFFIX}", p.id)).ok_or_else(|| tr("de opgeslagen toestand ontbreekt", "the stored state is missing").to_string())?;
        decode_state(&chunks)?
    };
    // Which handles are there now.
    let mut probe: Vec<Handle> = p.added_handles();
    probe.extend(p.marker_handles());
    probe.extend(p.modified_handles());
    probe.extend(p.removed_handles());
    let mut there = HashMap::new();
    bridge::for_each_entity_batch(&probe, 64, &mut |hs, es| {
        for (h, e) in hs.iter().zip(es) {
            there.insert(*h, e.is_some());
        }
    })?;
    let plan = restore_plan(&p, originals, &|h| there.get(&h).copied().unwrap_or(false))?;
    let label = format!("{} {}", tr("Voorstel afwijzen", "Reject proposal"), p.id);
    {
        let _group = bridge::UndoGroup::begin(&label);
        bridge::remove_entities(&label, &plan.remove)?;
        if !plan.add.is_empty() {
            let want: Vec<Handle> = plan.add.iter().map(|e| e.common().handle).collect();
            let got = bridge::add_entities_handles(&label, plan.add)?;
            if got != want {
                return Err(format!("{}: {}", p.id, tr("de host gaf de verwijderde elementen andere handles", "the host gave the removed entities other handles")));
            }
        }
        bridge::update_entities(&label, plan.update)?;
    }
    bridge::index_revert(&p.index_diff());
    forget(&p.id);
    let msg = format!(
        "{} {}: {} {} {}, {} {}.",
        State::Rejected.label(),
        p.id,
        p.added.len(),
        tr("elementen verwijderd,", "entities removed,"),
        p.modified.len() + p.removed.len(),
        tr("hersteld", "restored"),
        p.title()
    );
    activity::set_state(&p.id, State::Rejected);
    activity::append(review_entry(&p, who, State::Rejected, true, msg.clone()));
    crate::civiel::api::note("proposal", json!({"id": p.id, "state": State::Rejected, "removed": p.added, "restored": p.modified.iter().chain(&p.removed).collect::<Vec<_>>()}));
    crate::ui::send(crate::ui::UiMsg::AssistantChanged);
    Ok(msg)
}

/// Select a proposal's entities and zoom to it.
pub fn zoom(id: &str) -> Result<String, String> {
    let p = load(id)?;
    let hs = p.added_handles();
    let _ = bridge::select(&hs);
    if let Some(b) = p.bbox {
        let pad = 0.1 * (b[2] - b[0]).max(b[3] - b[1]).max(1.0);
        bridge::execute(&format!("ZOOM W {},{} {},{}", b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad))?;
    }
    Ok(format!("{}: {}", p.id, p.title()))
}

/// The pending proposals with their status, for the MCP tool and the window.
pub fn overview() -> Vec<(Proposal, Result<Status, String>)> {
    let all = list();
    all.iter().map(|p| (p.clone(), status(p, &all))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::entities::{LwPolyline, Mesh, Point, Polyline3D, Text};
    use ocs_plugin_api::host::codec::{CadDocument, DwgReader, DwgWriter, Vector2, Vector3};

    fn h(n: u64) -> Handle {
        Handle::from(n)
    }

    fn with_handle(mut e: EntityType, n: u64) -> EntityType {
        e.common_mut().handle = h(n);
        e
    }

    fn point(n: u64, x: f64) -> EntityType {
        let mut p = Point::new();
        p.location = Vector3::new(x, 2.0, 0.5);
        let mut e = with_handle(EntityType::Point(p), n);
        e.common_mut().layer = "B-XX-OG-HOOGTEPUNT-G".into();
        e
    }

    fn proposal(added: &[u64], modified: &[u64], removed: &[u64]) -> Proposal {
        Proposal {
            id: "P2".into(),
            added: added.iter().map(|n| handle_text(h(*n))).collect(),
            modified: modified.iter().map(|n| handle_text(h(*n))).collect(),
            removed: removed.iter().map(|n| handle_text(h(*n))).collect(),
            markers: vec![handle_text(h(90))],
            restorable: true,
            ..Default::default()
        }
    }

    /// The entities the civil tools draw: a TIN mesh, an axis, a feature
    /// line, a point and a text, each with Open Infra's object data.
    fn samples() -> Vec<EntityType> {
        let data = bridge::object_data([("as", "Rondweg".to_string()), ("bron", "AHN4".to_string())]);
        let mut mesh = Mesh::from_triangles(
            vec![Vector3::new(104900.0, 425300.0, 1.25), Vector3::new(104905.0, 425300.0, 1.5), Vector3::new(104900.0, 425305.0, 1.75), Vector3::new(104905.0, 425305.0, 2.0)],
            &[(0, 1, 2), (1, 3, 2)],
        );
        mesh.common.layer = "B-XX-OG-TRIANGULATIE-V".into();
        let mut lw = LwPolyline::new();
        lw.add_point_with_bulge(Vector2::new(104350.0, 424900.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(104800.123456, 424950.5), 0.2);
        lw.add_point_with_bulge(Vector2::new(105300.0, 424900.0), 0.0);
        lw.common.layer = "N-XX-AM-AS-G".into();
        let mut pl = Polyline3D::new();
        pl.add_vertex(Vector3::new(104000.0, 425000.0, 1.0));
        pl.add_vertex(Vector3::new(104010.0, 425000.0, 1.5));
        pl.common.layer = "N-XX-OG-BREUKLIJN-G".into();
        let mut t = Text::with_value("0+100".to_string(), Vector3::new(104500.0, 424920.0, 0.0)).with_height(1.25);
        t.common.layer = "N-XX-AM-KILOMETRERING-T".into();
        let mut out = vec![EntityType::Mesh(mesh), EntityType::LwPolyline(lw), EntityType::Polyline3D(pl), EntityType::Text(t), point(0, 104950.0)];
        for e in &mut out {
            e.common_mut().extended_data.add_record(data.clone());
            e.common_mut().handle = Handle::NULL;
        }
        out
    }

    #[test]
    fn fingerprint_ignores_handles_and_sees_edits() {
        let a = point(10, 1.0);
        let b = point(11, 1.0);
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut moved = a.clone();
        if let EntityType::Point(p) = &mut moved {
            p.location.x += 0.01;
        }
        assert_ne!(fingerprint(&a), fingerprint(&moved));
        let mut relayered = a.clone();
        relayered.common_mut().layer = "0".into();
        assert_ne!(fingerprint(&a), fingerprint(&relayered));
        let mut retagged = a.clone();
        retagged.common_mut().extended_data.upsert_record(bridge::object_data([("bron", "edited".to_string())]));
        assert_ne!(fingerprint(&a), fingerprint(&retagged));
        // A missing entity counts in the combined print, order does not.
        let one = combined(&[(h(1), Some(5)), (h(2), Some(6))]);
        assert_eq!(one, combined(&[(h(2), Some(6)), (h(1), Some(5))]));
        assert_ne!(one, combined(&[(h(1), Some(5)), (h(2), None)]));
    }

    #[test]
    fn fingerprint_survives_saving_and_opening_a_dwg() {
        let mut doc = CadDocument::new();
        // As the host does for a plugin's object data.
        let _ = doc.app_ids.add(ocs_plugin_api::host::codec::tables::AppId::new(bridge::XDATA_APP));
        let mut handles = Vec::new();
        for e in samples() {
            // As the host does: a plugin's layer goes into the layer table.
            if !doc.layers.iter().any(|l| l.name == e.common().layer) {
                let _ = doc.layers.add(ocs_plugin_api::host::codec::tables::Layer::new(e.common().layer.as_str()));
            }
            handles.push(doc.add_entity(e).unwrap());
        }
        let before: Vec<u64> = handles.iter().map(|h| fingerprint(doc.get_entity(*h).unwrap())).collect();
        let path = std::env::temp_dir().join(format!("oi-proposal-fp-{}.dwg", std::process::id()));
        DwgWriter::write_to_file(&path, &doc).unwrap();
        let back = DwgReader::from_file(&path).unwrap().read().unwrap();
        let _ = std::fs::remove_file(&path);
        for (h, fp) in handles.iter().zip(before) {
            let e = back.get_entity(*h).unwrap_or_else(|| panic!("{h:?} after the round trip"));
            let a = serde_json::to_value(doc.get_entity(*h).unwrap()).unwrap();
            let b = serde_json::to_value(e).unwrap();
            assert_eq!(fingerprint(e), fp, "differs: {}", differences(&a, &b, "").join("; "));
        }
    }

    /// The paths where two entities differ, bookkeeping left out.
    fn differences(a: &Value, b: &Value, path: &str) -> Vec<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => x
                .keys()
                .chain(y.keys())
                .filter(|k| !bookkeeping(k) && !(k.as_str() == "layer" && !path.ends_with("/common")))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .flat_map(|k| differences(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), &format!("{path}/{k}")))
                .collect(),
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x.iter().zip(y).enumerate().flat_map(|(i, (p, q))| differences(p, q, &format!("{path}/{i}"))).collect(),
            _ if a == b => Vec::new(),
            _ => vec![format!("{path}: {} → {}", a.to_string().chars().take(80).collect::<String>(), b.to_string().chars().take(80).collect::<String>())],
        }
    }

    #[test]
    fn state_round_trips_compressed() {
        let es: Vec<EntityType> = samples().into_iter().enumerate().map(|(i, e)| with_handle(e, 100 + i as u64)).collect();
        let chunks = encode_state(&es).unwrap();
        assert!(chunks.iter().all(|c| c.chars().count() <= 2000));
        let back = decode_state(&chunks).unwrap();
        assert_eq!(back.len(), es.len());
        for (a, b) in es.iter().zip(&back) {
            assert_eq!(a.common().handle, b.common().handle);
            assert_eq!(fingerprint(a), fingerprint(b));
        }
        assert!(decode_state(&["not base64!".to_string()]).is_err());
    }

    #[test]
    fn reject_removes_added_and_restores_with_the_old_handles() {
        let p = proposal(&[20, 21], &[5], &[6]);
        // 21 was deleted by the user since; 6 was removed by the proposal.
        let there: HashSet<u64> = [20, 5, 90].into_iter().collect();
        let plan = restore_plan(&p, vec![point(5, 1.0), point(6, 2.0)], &|x| there.contains(&u64::from(x))).unwrap();
        assert_eq!(plan.remove, vec![h(20), h(90)]);
        assert_eq!(plan.add.iter().map(|e| e.common().handle).collect::<Vec<_>>(), vec![h(6)]);
        assert_eq!(plan.update.iter().map(|e| e.common().handle).collect::<Vec<_>>(), vec![h(5)]);
        // A modified entity deleted since comes back too.
        let plan = restore_plan(&p, vec![point(5, 1.0), point(6, 2.0)], &|x| u64::from(x) == 20).unwrap();
        assert_eq!(plan.add.len(), 2);
        assert!(plan.update.is_empty());
    }

    #[test]
    fn reject_refuses_without_complete_state_or_on_a_taken_handle() {
        let p = proposal(&[20], &[5], &[6]);
        assert!(restore_plan(&p, vec![point(5, 1.0)], &|_| true).unwrap_err().contains('6'));
        assert!(restore_plan(&p, vec![point(5, 1.0), point(6, 1.0)], &|_| true).is_err());
        // Nothing modified: only the additions go.
        let q = proposal(&[20, 21], &[], &[]);
        assert_eq!(restore_plan(&q, Vec::new(), &|_| true).unwrap().remove.len(), 3);
    }

    #[test]
    fn stale_when_edited_and_who_changed_it() {
        let e = point(20, 1.0);
        let mut p = proposal(&[20], &[], &[]);
        p.fingerprint = format!("{:016X}", combined(&[(h(20), Some(fingerprint(&e)))]));
        assert!(!status_of(&p, &[(h(20), Some(fingerprint(&e)))], &[]).stale);
        let mut later = proposal(&[30], &[20], &[]);
        later.id = "P3".into();
        let edited = status_of(&p, &[(h(20), Some(fingerprint(&point(20, 9.0))))], &[p.clone(), later]);
        assert!(edited.stale);
        assert_eq!(edited.changed_by, vec!["P3".to_string()]);
        let gone = status_of(&p, &[(h(20), None)], &[]);
        assert!(gone.stale);
        assert_eq!(gone.missing, 1);
    }

    #[test]
    fn numbers_and_markers() {
        assert_eq!(next_id(&[]), "P1");
        assert_eq!(next_id(&["P1".into(), "P7-STATE".into(), "P3".into(), "x".into()]), "P8");
        assert_eq!(normalise("p12").unwrap(), "P12");
        assert_eq!(normalise(" 4 ").unwrap(), "P4");
        assert!(normalise("P").is_err());
        let m = marker_entities("P2", "Create an alignment", (3, 0, 1), [0.0, 0.0, 100.0, 50.0], 0.5);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|e| e.common().layer == MARKER_LAYER));
        let bb = m[0].as_entity().bounding_box();
        assert!(bb.min.x < 0.0 && bb.max.y > 50.0);
    }
}
