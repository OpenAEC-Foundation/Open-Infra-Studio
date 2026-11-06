//! The assistant's commands, for the ribbon, scripts and the MCP server:
//!
//! - `CIV_ASSISTENT` (`CIV_ASSISTANT`, `GIS_ASSISTENT`): the Assistant page;
//! - `CIV_VOORSTELLEN` (`CIV_PROPOSALS`): the pending proposals with their status;
//! - `CIV_VOORSTEL_ACCEPTEREN VOORSTEL=P3` (`CIV_PROPOSAL_ACCEPT`),
//!   `CIV_VOORSTEL_AFWIJZEN` (`CIV_PROPOSAL_REJECT`),
//!   `CIV_VOORSTEL_VERVALLEN` (`CIV_PROPOSAL_DISMISS`): the user's review;
//! - `CIV_ACTIVITEIT [MAX=50]` (`CIV_ACTIVITY_LOG`): the activity log;
//! - `CIV_ACTIVITEIT_NOTEER` (`CIV_ACTIVITY_NOTE`): a line in the log for a
//!   step without its own command (the MCP server's save and open).

use ocs_plugin_api::host::HostApi;
use serde_json::json;

use super::capture::Request;
use super::{activity, proposal};
use crate::bridge;
use crate::civiel::api::{self, Args};
use crate::lang::tr;

/// The review verbs: an assistant's own tools never run them.
pub const REVIEW_VERBS: &[&str] = &[
    "CIV_VOORSTEL_ACCEPTEREN",
    "CIV_PROPOSAL_ACCEPT",
    "CIV_VOORSTEL_AFWIJZEN",
    "CIV_PROPOSAL_REJECT",
    "CIV_VOORSTEL_VERVALLEN",
    "CIV_PROPOSAL_DISMISS",
];

/// accept, reject or dismiss: (proposal, who) → message.
type Review = fn(&str, &str) -> Result<String, String>;

/// Who reviews: the user, or the MCP client its operator allowed.
fn reviewer(request: &Option<Request>) -> String {
    match request {
        Some(r) if r.review && !r.client.trim().is_empty() => format!("{} (OI_MCP_REVIEW)", r.client),
        _ => "user".into(),
    }
}

/// On a worker: the answer goes to the command line and, with `ID=`, to
/// its JSON file.
fn work(host: &mut dyn HostApi, label: &str, id: Option<String>, f: impl FnOnce() -> Result<String, String> + Send + 'static) {
    host.push_info(&format!("[Open Infra] {label} …"));
    let label = label.to_string();
    std::thread::spawn(move || {
        let _ = api::take();
        let r = super::capture::without(|| std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| Err(crate::panic_text(p.as_ref()))));
        let data = api::take();
        let (ok, msg) = match r {
            Ok(m) => (true, m),
            Err(e) => (false, format!("{label}: {e}")),
        };
        let _ = bridge::request(if ok {
            ocs_plugin_api::ipc::protocol::PluginRequest::PushInfo(format!("[Open Infra] {msg}"))
        } else {
            ocs_plugin_api::ipc::protocol::PluginRequest::PushError(format!("[Open Infra] {msg}"))
        });
        if let Some(id) = id {
            api::write(&id, ok, &msg, data);
        }
    });
}

fn proposal_arg(args: &Args) -> Result<String, String> {
    args.str("VOORSTEL").or(args.str("PROPOSAL")).or(args.str("NR")).map(str::to_string).or_else(|| args.words.first().cloned()).ok_or_else(|| tr("geef VOORSTEL=<P1, P2, …> (CIV_VOORSTELLEN geeft de lijst)", "give VOORSTEL=<P1, P2, …> (CIV_VOORSTELLEN lists them)").to_string())
}

/// The pending proposals as JSON, with their status.
pub fn proposals_json() -> (Vec<serde_json::Value>, String) {
    let list = proposal::overview();
    let mut lines = Vec::new();
    let rows = list
        .iter()
        .map(|(p, st)| {
            let (stale, missing, by, error) = match st {
                Ok(s) => (s.stale, s.missing, s.changed_by.clone(), None),
                Err(e) => (true, 0, Vec::new(), Some(e.clone())),
            };
            lines.push(format!("{} {}{}", p.id, p.title(), if stale { tr(" (verouderd)", " (stale)") } else { "" }));
            json!({
                "id": p.id, "created": p.created, "client": p.client, "tool": p.tool, "class": p.class, "command": p.command,
                "arguments": p.arguments, "title": p.title_en, "title_nl": p.title_nl, "message": p.message,
                "added": p.added, "modified": p.modified, "removed": p.removed, "markers": p.markers,
                "bbox": p.bbox, "restorable": p.restorable && !stale, "stale": stale, "missing": missing,
                "changed_by": by, "error": error,
            })
        })
        .collect();
    let msg = if lines.is_empty() {
        tr("Geen openstaande voorstellen.", "No pending proposals.").to_string()
    } else {
        format!("{} {}: {}", lines.len(), tr("openstaande voorstellen", "pending proposals"), lines.join("; "))
    };
    (rows, msg)
}

/// Run one of the assistant's verbs; `false` when `verb` is not one.
pub fn run(host: &mut dyn HostApi, verb: &str, args: &Args, id: Option<String>, request: Option<Request>) -> Result<bool, String> {
    match verb {
        "CIV_ASSISTENT" | "CIV_ASSISTANT" | "GIS_ASSISTENT" | "GIS_ASSISTANT" => {
            crate::ui::open(crate::ui::Tab::Assistant)?;
            crate::ui::send(crate::ui::UiMsg::AssistantChanged);
            if let Some(id) = id {
                api::write(&id, true, tr("Pagina Assistent geopend", "Assistant page opened"), Default::default());
            }
        }
        "CIV_VOORSTELLEN" | "CIV_PROPOSALS" => work(host, tr("Voorstellen", "Proposals"), id, || {
            let (rows, msg) = proposals_json();
            api::note("count", rows.len());
            api::note("proposals", rows);
            Ok(msg)
        }),
        "CIV_VOORSTEL_ACCEPTEREN" | "CIV_PROPOSAL_ACCEPT" | "CIV_VOORSTEL_AFWIJZEN" | "CIV_PROPOSAL_REJECT" | "CIV_VOORSTEL_VERVALLEN" | "CIV_PROPOSAL_DISMISS" => {
            let which = proposal_arg(args)?;
            let who = reviewer(&request);
            let (label, f): (&str, Review) = match verb {
                "CIV_VOORSTEL_ACCEPTEREN" | "CIV_PROPOSAL_ACCEPT" => (tr("Voorstel accepteren", "Accept proposal"), proposal::accept),
                "CIV_VOORSTEL_AFWIJZEN" | "CIV_PROPOSAL_REJECT" => (tr("Voorstel afwijzen", "Reject proposal"), proposal::reject),
                _ => (tr("Voorstel laten vervallen", "Dismiss proposal"), proposal::dismiss),
            };
            work(host, label, id, move || f(&which, &who));
        }
        "CIV_ACTIVITEIT" | "CIV_ACTIVITY_LOG" | "CIV_ACTIVITY" => {
            let max = args.f64("MAX")?.map(|m| m.max(1.0) as usize).unwrap_or(50);
            work(host, tr("Activiteitenlog", "Activity log"), id, move || {
                let list = activity::read();
                let n = list.len();
                let recent: Vec<&activity::Entry> = list.iter().rev().take(max).collect();
                let lines: Vec<String> = recent.iter().take(5).map(|e| e.plain()).collect();
                api::note("count", n);
                api::note("entries", serde_json::to_value(&recent).unwrap_or_default());
                Ok(format!("{} {}{}{}", n, tr("regels in het activiteitenlog", "lines in the activity log"), if lines.is_empty() { "" } else { ": " }, lines.join(" | ")))
            });
        }
        "CIV_ACTIVITEIT_NOTEER" | "CIV_ACTIVITY_NOTE" => {
            // With a request file the capture logs it; a user's own note here.
            let text = args.str("TEKST").or(args.str("TEXT")).map(str::to_string).unwrap_or_else(|| args.words.join(" "));
            let own = request.is_none() && !text.trim().is_empty();
            work(host, tr("Notitie", "Note"), id, move || {
                if own {
                    activity::append(activity::Entry { at: super::now(), who: "user".into(), tool: "note".into(), class: "note".into(), title_nl: text.clone(), title_en: text.clone(), ok: true, ..Default::default() });
                }
                Ok(tr("Genoteerd in het activiteitenlog", "Noted in the activity log").to_string())
            });
        }
        _ => return Ok(false),
    }
    Ok(true)
}
