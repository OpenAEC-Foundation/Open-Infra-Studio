//! AI assistant changes as proposals, and a plain-language activity log.
//!
//! The MCP server (`mcp/open_infra_mcp.py`) marks every command it sends for
//! an assistant with a small request file beside the command's answer
//! (`<config>/OpenCADStudio/open-infra/api/<id>.request.json`): the client
//! that asked, the tool, its class and arguments, and the mode. While such a
//! command runs, the bridge records what it adds, and the state of what it
//! modifies or removes ([`capture`]). When the command answers:
//!
//! - in **propose** mode a change becomes a pending [`proposal`]: kept in the
//!   drawing (XRecords under `OPENINFRA/PROPOSALS`) with a fingerprint of the
//!   proposed entities and the state to restore, and marked by a frame and a
//!   label on the layer [`proposal::MARKER_LAYER`] until the user accepts or
//!   rejects it;
//! - in **apply** mode it stays as it is.
//!
//! Either way the [`activity`] log in the drawing (`OPENINFRA/ACTIVITY`) gets
//! a line: when, which client, which tool, what, how many entities, and the
//! proposal's state. The user's accepts and rejects are logged too.

pub mod activity;
pub mod capture;
pub mod commands;
pub mod describe;
pub mod proposal;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use activity::{Entry, State};

/// What an edit by an assistant becomes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A pending proposal the user accepts or rejects (the default).
    #[default]
    Propose,
    /// A done fact, as before; undo is the safety net.
    Apply,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "propose" | "voorstel" | "voorstellen" => Some(Mode::Propose),
            "apply" | "toepassen" | "direct" => Some(Mode::Apply),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Mode::Propose => "propose",
            Mode::Apply => "apply",
        }
    }
}

fn settings_file() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("settings.json")
}

/// The mode from `assistant` in `<config>/OpenCADStudio/open-infra/settings.json`
/// (read each time, so a change in the window counts at once).
pub fn mode() -> Mode {
    std::fs::read_to_string(settings_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("assistant").and_then(Value::as_str).and_then(Mode::parse))
        .unwrap_or_default()
}

/// Store the mode in the settings file, keeping its other keys.
pub fn set_mode(mode: Mode) -> Result<(), String> {
    let path = settings_file();
    let mut v: Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.to_string()),
    };
    if !v.is_object() {
        return Err(format!("{}: not a JSON object", path.display()));
    }
    v["assistant"] = Value::String(mode.code().into());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The time now, with the local offset (RFC 3339, seconds).
pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// "2026-11-05 14:03" from [`now`]'s form.
pub fn short_time(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339).map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|_| rfc3339.to_string())
}

/// At most `n` characters, on one line.
pub fn clip(s: &str, n: usize) -> String {
    let one: String = s.replace(['\n', '\r'], " ");
    if one.chars().count() <= n {
        return one;
    }
    let mut out: String = one.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The end of an assistant's command: a proposal (propose mode, something
/// changed) and a line in the activity log. Runs on its own thread, before
/// the command's answer is written (see [`capture::finish`]).
fn record(a: capture::Finished, ok: bool, message: &str, data: &mut Map<String, Value>) {
    let (added, modified, removed) = a.changes.counts();
    let (title_nl, title_en) = match &a.request.note {
        Some(n) => (n.clone(), n.clone()),
        None => describe::title(&a.command),
    };
    let mut entry = Entry {
        at: now(),
        who: if a.request.client.trim().is_empty() { "assistant".into() } else { a.request.client.clone() },
        tool: a.request.tool.clone(),
        class: a.request.class.clone(),
        title_nl,
        title_en,
        command: clip(&a.command, 400),
        message: clip(message, 300),
        ok,
        added,
        modified,
        removed,
        state: State::Done,
        proposal: None,
    };
    // Without entity changes (an export, a save, the project location) the
    // step is done as it is.
    entry.state = if a.changes.is_empty() {
        if ok { State::Done } else { State::Failed }
    } else if !a.keep {
        State::Applied
    } else {
        match proposal::create(&a, ok, message, &entry) {
            Ok(p) => {
                entry.proposal = Some(p.id.clone());
                data.insert("proposal".into(), p.summary());
                State::Pending
            }
            Err(e) => {
                data.insert("proposal_error".into(), Value::String(e));
                State::Applied
            }
        }
    };
    data.insert(
        "activity".into(),
        json!({"state": entry.state, "proposal": entry.proposal, "added": added, "modified": modified, "removed": removed, "mode": if a.keep { "propose" } else { "apply" }}),
    );
    activity::append(entry);
    crate::ui::send(crate::ui::UiMsg::AssistantChanged);
}
