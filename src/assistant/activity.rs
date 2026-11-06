//! The activity log in the drawing (`OPENINFRA/ACTIVITY`): what assistants
//! did and what the user decided, in plain words. The newest
//! [`MAX_ENTRIES`] lines are kept.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::bridge;
use crate::lang::tr;

const NAME: &str = "ACTIVITY";
const DICT: &[&str] = &["OPENINFRA"];
pub const MAX_ENTRIES: usize = 500;

/// Where a line stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Changed the drawing at once (apply mode).
    Applied,
    /// A proposal waiting for the user.
    Pending,
    Accepted,
    Rejected,
    /// A stale proposal the user closed without restoring.
    Dismissed,
    /// Finished without changing the drawing (an export, a save).
    #[default]
    Done,
    /// An edit that found nothing to change.
    NoChange,
    Failed,
}

impl State {
    pub fn label_nl(self) -> &'static str {
        match self {
            State::Applied => "Toegepast",
            State::Pending => "Wacht op beoordeling",
            State::Accepted => "Geaccepteerd",
            State::Rejected => "Afgewezen",
            State::Dismissed => "Vervallen",
            State::Done => "Uitgevoerd",
            State::NoChange => "Niets gewijzigd",
            State::Failed => "Mislukt",
        }
    }

    pub fn label_en(self) -> &'static str {
        match self {
            State::Applied => "Applied",
            State::Pending => "Awaiting review",
            State::Accepted => "Accepted",
            State::Rejected => "Rejected",
            State::Dismissed => "Dismissed",
            State::Done => "Done",
            State::NoChange => "No change",
            State::Failed => "Failed",
        }
    }

    pub fn label(self) -> &'static str {
        tr(self.label_nl(), self.label_en())
    }
}

/// One line of the log.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// RFC 3339 with the local offset.
    pub at: String,
    /// The MCP client (name and version from its `initialize`), or "user".
    pub who: String,
    /// The MCP tool, or accept / reject / dismiss for the user's review.
    pub tool: String,
    /// read, edit, export, save or review.
    pub class: String,
    pub title_nl: String,
    pub title_en: String,
    /// The command line it ran (without `ID=`).
    pub command: String,
    /// The command's own answer.
    pub message: String,
    pub ok: bool,
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
    pub state: State,
    pub proposal: Option<String>,
}

impl Entry {
    pub fn title(&self) -> &str {
        tr(&self.title_nl, &self.title_en)
    }

    pub fn who_label(&self) -> &str {
        if self.who == "user" { tr("gebruiker", "user") } else { &self.who }
    }

    /// "+3 ~1 −0", empty when nothing changed.
    pub fn counts(&self) -> String {
        if self.added + self.modified + self.removed == 0 {
            String::new()
        } else {
            format!("+{} ~{} −{}", self.added, self.modified, self.removed)
        }
    }

    /// The line in plain words, in the interface language.
    pub fn plain(&self) -> String {
        let mut s = format!("{} · {} · {}", super::short_time(&self.at), self.who_label(), self.title());
        let c = self.counts();
        if !c.is_empty() {
            s.push_str(&format!(" ({c})"));
        }
        s.push_str(&format!(" — {}", self.state.label()));
        if let Some(p) = &self.proposal {
            s.push_str(&format!(" [{p}]"));
        }
        s
    }
}

/// Add a line, keeping the newest `max`.
pub fn push_capped(list: &mut Vec<Entry>, e: Entry, max: usize) {
    list.push(e);
    if list.len() > max {
        let extra = list.len() - max;
        list.drain(..extra);
    }
}

/// The proposal's own line (the assistant's) gets its new state.
pub fn set_state_in(list: &mut [Entry], proposal: &str, state: State) -> bool {
    match list.iter_mut().rev().find(|e| e.proposal.as_deref() == Some(proposal) && e.class != "review") {
        Some(e) => {
            e.state = state;
            true
        }
        None => false,
    }
}

static LOCK: Mutex<()> = Mutex::new(());

/// The log of the active drawing, oldest first.
pub fn read() -> Vec<Entry> {
    bridge::xrecord_strings(DICT, NAME).and_then(|c| serde_json::from_str(&c.concat()).ok()).unwrap_or_default()
}

fn write(list: &[Entry]) {
    if let Ok(json) = serde_json::to_string(list) {
        let _ = bridge::xrecord_write_strings(DICT, NAME, bridge::text_chunks(&json, 2000));
    }
}

pub fn append(e: Entry) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read();
    push_capped(&mut list, e, MAX_ENTRIES);
    write(&list);
}

pub fn set_state(proposal: &str, state: State) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = read();
    if set_state_in(&mut list, proposal, state) {
        write(&list);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize, proposal: Option<&str>) -> Entry {
        Entry { at: "2026-11-05T14:03:00+01:00".into(), who: "claude-code 2.1".into(), tool: "civ_tin".into(), class: "edit".into(), title_nl: format!("Stap {n}"), title_en: format!("Step {n}"), added: n, state: State::Pending, proposal: proposal.map(str::to_string), ..Default::default() }
    }

    #[test]
    fn keeps_the_newest_lines() {
        let mut list = Vec::new();
        for n in 0..7 {
            push_capped(&mut list, entry(n, None), 5);
        }
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].title_en, "Step 2");
        assert_eq!(list[4].title_en, "Step 6");
    }

    #[test]
    fn a_review_updates_the_proposals_own_line() {
        let mut list = vec![entry(1, Some("P1")), entry(2, Some("P2"))];
        let mut review = entry(3, Some("P1"));
        review.class = "review".into();
        list.push(review);
        assert!(set_state_in(&mut list, "P1", State::Accepted));
        assert_eq!(list[0].state, State::Accepted);
        assert_eq!(list[2].state, State::Pending, "the review line keeps its own state");
        assert!(!set_state_in(&mut list, "P9", State::Rejected));
    }

    #[test]
    fn plain_line() {
        let e = entry(3, Some("P4"));
        let s = e.plain();
        assert!(s.starts_with("2026-11-05 14:03 · claude-code 2.1 · "), "{s}");
        assert!(s.contains("(+3 ~0 −0)") && s.ends_with("[P4]"), "{s}");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["state"], "pending");
        assert_eq!(serde_json::to_value(State::NoChange).unwrap(), "no_change");
    }
}
