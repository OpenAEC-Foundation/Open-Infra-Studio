//! What an assistant's command changes, recorded where the bridge writes:
//! the entities it adds, and the state before of the entities it modifies or
//! removes. One assistant command at a time, as the MCP server sends them
//! (it waits for each answer); the capture ends when the command answers.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use ocs_plugin_api::host::codec::{EntityType, Handle};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Mode;
use crate::bridge;
use crate::lang::tr;

/// What the MCP server says about a command it sends for an assistant
/// (`<config>/OpenCADStudio/open-infra/api/<id>.request.json`, written just
/// before the command).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The client's name and version from the MCP `initialize` (clientInfo).
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub tool: String,
    /// read, edit, export or save.
    #[serde(default)]
    pub class: String,
    /// The mode the server applies; the settings file when absent.
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub arguments: Value,
    /// A review (accept, reject, dismiss) the server's operator allowed
    /// (`OI_MCP_REVIEW=1`); never set for an assistant's own tools.
    #[serde(default)]
    pub review: bool,
    /// A line for the activity log of a step without its own command
    /// (saving, opening).
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Modified,
    Removed,
}

/// The bookkeeping of one command's changes.
#[derive(Debug, Default)]
pub struct Changes {
    /// New entities, in the order they came.
    pub added: Vec<Handle>,
    added_set: HashSet<Handle>,
    /// Entities that were there before: what happened to them and their
    /// first state (`None` when it could not be read or was not kept).
    pub before: Vec<(Handle, Change, Option<EntityType>)>,
    at: HashMap<Handle, usize>,
    /// What the command changed in the object index (OPENINFRA/INDEX).
    pub index: bridge::IndexDiff,
}

impl Changes {
    pub fn add(&mut self, handles: &[Handle]) {
        for h in handles {
            if !h.is_null() && self.added_set.insert(*h) {
                self.added.push(*h);
            }
        }
    }

    /// The handles whose state has to be read before they change: not added
    /// by this command and not changed by it already.
    pub fn unknown(&self, handles: &[Handle]) -> Vec<Handle> {
        let mut seen = HashSet::new();
        handles.iter().copied().filter(|h| !h.is_null() && !self.added_set.contains(h) && !self.at.contains_key(h) && seen.insert(*h)).collect()
    }

    /// A change to an entity: its first state is kept; one this command
    /// added stays simply added.
    pub fn modify(&mut self, h: Handle, original: Option<EntityType>) {
        if self.added_set.contains(&h) || self.at.contains_key(&h) {
            return;
        }
        self.at.insert(h, self.before.len());
        self.before.push((h, Change::Modified, original));
    }

    /// A removal: one this command added is as if it never was; a changed
    /// one is removed with its first state.
    pub fn remove(&mut self, h: Handle, original: Option<EntityType>) {
        if self.added_set.remove(&h) {
            self.added.retain(|x| *x != h);
            return;
        }
        match self.at.get(&h) {
            Some(&i) => self.before[i].1 = Change::Removed,
            None => {
                self.at.insert(h, self.before.len());
                self.before.push((h, Change::Removed, original));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.before.is_empty()
    }

    /// Add an index change; one that undoes an earlier one cancels it.
    pub fn index(&mut self, diff: bridge::IndexDiff) {
        for (k, n, add, rem) in diff {
            let i = match self.index.iter().position(|(a, b, ..)| *a == k && *b == n) {
                Some(i) => i,
                None => {
                    self.index.push((k, n, Vec::new(), Vec::new()));
                    self.index.len() - 1
                }
            };
            let e = &mut self.index[i];
            for h in add {
                match e.3.iter().position(|x| *x == h) {
                    Some(j) => {
                        e.3.remove(j);
                    }
                    None if !e.2.contains(&h) => e.2.push(h),
                    None => {}
                }
            }
            for h in rem {
                match e.2.iter().position(|x| *x == h) {
                    Some(j) => {
                        e.2.remove(j);
                    }
                    None if !e.3.contains(&h) => e.3.push(h),
                    None => {}
                }
            }
        }
        self.index.retain(|e| !e.2.is_empty() || !e.3.is_empty());
    }

    /// (added, modified, removed).
    pub fn counts(&self) -> (usize, usize, usize) {
        let m = self.before.iter().filter(|b| b.1 == Change::Modified).count();
        (self.added.len(), m, self.before.len() - m)
    }

    pub fn with(&self, change: Change) -> Vec<Handle> {
        self.before.iter().filter(|b| b.1 == change).map(|b| b.0).collect()
    }
}

/// The command being recorded.
struct Active {
    id: String,
    request: Request,
    command: String,
    keep: bool,
    changes: Changes,
}

/// A finished capture, handed to the proposal and the log.
pub struct Finished {
    pub request: Request,
    pub command: String,
    /// Propose mode: the state before was kept.
    pub keep: bool,
    pub changes: Changes,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<Active>> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

thread_local! {
    /// The user's own review runs on this thread: not part of a capture.
    static SUSPENDED: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` without recording its writes (accept, reject, the bookkeeping
/// of a proposal itself).
pub fn without<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SUSPENDED.with(|s| s.set(self.0));
        }
    }
    let _restore = Restore(SUSPENDED.with(|s| s.replace(true)));
    f()
}

fn suspended() -> bool {
    SUSPENDED.with(|s| s.get())
}

/// The verbs of the user's review: never run on an assistant's behalf.
pub fn is_review(verb: &str) -> bool {
    super::commands::REVIEW_VERBS.contains(&verb.to_ascii_uppercase().as_str())
}

/// The request file of a command with `ID=<id>`, read and removed.
pub fn take_request(id: &str) -> Result<Option<Request>, String> {
    let path = crate::civiel::api::result_dir().join(format!("{id}.request.json"));
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {e}", path.display()))
}

/// Called by both plugins' dispatch before a command runs: a command an
/// assistant sent (its request file is there) is recorded until it answers.
/// A review verb sent for an assistant is refused; one the server's operator
/// allowed comes back with its request, for the log.
pub fn begin(id: Option<&str>, command: &str) -> Result<Option<Request>, String> {
    let Some(id) = id else { return Ok(None) };
    let Some(request) = take_request(id)? else { return Ok(None) };
    let verb = command.split_whitespace().next().unwrap_or("");
    if is_review(verb) {
        if !request.review {
            return Err(tr(
                "een assistent beoordeelt zijn eigen voorstellen niet: de gebruiker accepteert of wijst ze af op de pagina Assistent",
                "an assistant does not review its own proposals: the user accepts or rejects them on the Assistant page",
            )
            .into());
        }
        return Ok(Some(request));
    }
    let mode = request.mode.unwrap_or_else(super::mode);
    let old = lock().replace(Active { id: id.to_string(), request: request.clone(), command: command.trim().to_string(), keep: mode == Mode::Propose, changes: Changes::default() });
    // An earlier command that never answered: keep what it did.
    if let Some(old) = old {
        let note = tr("geen antwoord van het commando", "the command did not answer").to_string();
        spawn_record(old, false, note, Map::new(), false);
    }
    Ok(Some(request))
}

/// The state of entities before a write, read while an assistant's command
/// runs (propose mode); `None` otherwise.
pub struct Before {
    states: Vec<(Handle, Option<EntityType>)>,
}

pub fn prepare(handles: &[Handle]) -> Option<Before> {
    if suspended() {
        return None;
    }
    let (keep, unknown) = {
        let g = lock();
        let a = g.as_ref()?;
        (a.keep, a.changes.unknown(handles))
    };
    let mut states = Vec::with_capacity(unknown.len());
    if keep && !unknown.is_empty() {
        let read = bridge::for_each_entity_batch(&unknown, 16, &mut |hs, es| states.extend(hs.iter().copied().zip(es)));
        if read.is_err() {
            // Not restorable, but still counted.
            states = unknown.iter().map(|h| (*h, None)).collect();
        }
    } else {
        states = unknown.iter().map(|h| (*h, None)).collect();
    }
    Some(Before { states })
}

/// The entities of [`prepare`] were modified.
pub fn modified(before: Option<Before>) {
    let Some(b) = before else { return };
    if let Some(a) = lock().as_mut() {
        for (h, e) in b.states {
            a.changes.modify(h, e);
        }
    }
}

/// Of the entities of [`prepare`], these were removed.
pub fn removed(before: Option<Before>, gone: &[Handle]) {
    let Some(b) = before else { return };
    let mut states: HashMap<Handle, Option<EntityType>> = b.states.into_iter().collect();
    if let Some(a) = lock().as_mut() {
        for h in gone {
            a.changes.remove(*h, states.remove(h).flatten());
        }
    }
}

/// New entities.
pub fn added(handles: &[Handle]) {
    if suspended() {
        return;
    }
    if let Some(a) = lock().as_mut() {
        a.changes.add(handles);
    }
}

/// The object index changed (propose mode keeps it, to undo on reject).
pub fn index_changed(diff: bridge::IndexDiff) {
    if suspended() || diff.is_empty() {
        return;
    }
    if let Some(a) = lock().as_mut() {
        if a.keep {
            a.changes.index(diff);
        }
    }
}

/// Whether an assistant's command is being recorded.
pub fn active() -> bool {
    lock().is_some()
}

/// The answer of command `id` (from [`crate::civiel::api::write`]): when it
/// was an assistant's, its proposal and log line are made on a worker and
/// the answer is written after them (`None`); otherwise the data comes back
/// to be written at once.
pub fn finish(id: &str, ok: bool, message: &str, data: Map<String, Value>) -> Option<Map<String, Value>> {
    let a = {
        let mut g = lock();
        if g.as_ref().is_some_and(|a| a.id == id) { g.take() } else { None }
    };
    let Some(a) = a else { return Some(data) };
    spawn_record(a, ok, message.to_string(), data, true);
    None
}

fn spawn_record(a: Active, ok: bool, message: String, data: Map<String, Value>, answer: bool) {
    // The command's own document, also when the user switched tabs since.
    let (target, site) = (bridge::target(), bridge::site());
    let id = a.id.clone();
    let done = Finished { request: a.request, command: a.command, keep: a.keep, changes: a.changes };
    // Off the calling thread: the host's dispatch may not wait on itself.
    std::thread::spawn(move || {
        let mut data = data;
        let run = || {
            without(|| {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::record(done, ok, &message, &mut data)));
                if let Err(p) = r {
                    data.insert("proposal_error".into(), Value::String(crate::panic_text(p.as_ref())));
                }
            })
        };
        match target {
            Some(t) => bridge::with_command_context(t, site, None, run),
            None => run(),
        }
        if answer {
            crate::civiel::api::write_file(&id, ok, &message, data);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{entities::Point, Vector3};

    fn h(n: u64) -> Handle {
        Handle::from(n)
    }

    fn point(n: u64, x: f64) -> EntityType {
        let mut p = Point::new();
        p.location = Vector3::new(x, 0.0, 0.0);
        let mut e = EntityType::Point(p);
        e.common_mut().handle = h(n);
        e
    }

    #[test]
    fn added_then_removed_is_no_change() {
        let mut c = Changes::default();
        c.add(&[h(10), h(11)]);
        c.remove(h(10), None);
        assert_eq!(c.added, vec![h(11)]);
        assert_eq!(c.counts(), (1, 0, 0));
        c.remove(h(11), None);
        assert!(c.is_empty());
    }

    #[test]
    fn the_first_state_of_a_changed_entity_is_kept() {
        let mut c = Changes::default();
        c.modify(h(5), Some(point(5, 1.0)));
        // A second change keeps the state from before the command.
        assert!(c.unknown(&[h(5)]).is_empty());
        c.modify(h(5), Some(point(5, 2.0)));
        assert_eq!(c.before.len(), 1);
        assert!(matches!(&c.before[0].2, Some(EntityType::Point(p)) if p.location.x == 1.0));
        // Modified, then removed: removed with that first state.
        c.remove(h(5), Some(point(5, 3.0)));
        assert_eq!(c.counts(), (0, 0, 1));
        assert!(matches!(&c.before[0], (_, Change::Removed, Some(EntityType::Point(p))) if p.location.x == 1.0));
    }

    #[test]
    fn own_entities_need_no_state_and_stay_added() {
        let mut c = Changes::default();
        c.add(&[h(20)]);
        assert_eq!(c.unknown(&[h(20), h(21), h(21), Handle::NULL]), vec![h(21)]);
        c.modify(h(20), Some(point(20, 0.0)));
        assert_eq!(c.counts(), (1, 0, 0));
        assert_eq!(c.with(Change::Modified), Vec::<Handle>::new());
    }

    #[test]
    fn index_changes_are_kept_and_undone() {
        let s = |v: &[u64]| v.iter().map(|n| h(*n)).collect::<Vec<_>>();
        let before = vec![("kantlijnlabel".to_string(), "A1".to_string(), s(&[1, 2])), ("tin".to_string(), "B".to_string(), s(&[7]))];
        let after = vec![("kantlijnlabel".to_string(), "A1".to_string(), s(&[3, 4])), ("tin".to_string(), "B".to_string(), s(&[7])), ("as".to_string(), "Rondweg".to_string(), s(&[9]))];
        let diff = bridge::index_diff(&before, &after);
        assert_eq!(diff.len(), 2);
        let mut c = Changes::default();
        c.index(diff);
        // A later step of the same command that takes one back cancels out.
        c.index(vec![("as".to_string(), "Rondweg".to_string(), Vec::new(), s(&[9]))]);
        assert_eq!(c.index, vec![("kantlijnlabel".to_string(), "A1".to_string(), s(&[3, 4]), s(&[1, 2]))]);
        // Undone on an index the user changed since: the other keys stay.
        let now = vec![("kantlijnlabel".to_string(), "A1".to_string(), s(&[3, 4, 5])), ("tin".to_string(), "B".to_string(), s(&[7, 8]))];
        let back = bridge::index_reverted(now, &c.index);
        assert_eq!(back, vec![("kantlijnlabel".to_string(), "A1".to_string(), s(&[5, 1, 2])), ("tin".to_string(), "B".to_string(), s(&[7, 8]))]);
        // A key whose handles all go is dropped.
        let gone = bridge::index_reverted(vec![("as".to_string(), "X".to_string(), s(&[9]))], &vec![("as".to_string(), "X".to_string(), s(&[9]), Vec::new())]);
        assert!(gone.is_empty());
    }

    #[test]
    fn a_review_verb_is_known() {
        assert!(is_review("civ_voorstel_accepteren"));
        assert!(is_review("CIV_PROPOSAL_REJECT"));
        assert!(!is_review("CIV_TIN"));
    }
}
