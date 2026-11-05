//! Design problems: one list of what every check finds, with fixes the
//! program computes (`CIV_PROBLEMEN`, `CIV_PROBLEEM_HERSTEL`,
//! `CIV_PROBLEEM_ACCEPTEER`, the Problems page of the window).
//!
//! Each check keeps its own logic and reports through a small adapter in
//! its module: the alignment layout and the design profile
//! ([`super::as_profiel`]), the design criteria ([`super::ontwerpcriteria`]),
//! the sewer check ([`super::riool`]), gradings ([`super::grading`]),
//! corridors ([`super::corridor`]), out-of-date profiles ([`super::graaf`])
//! and the NLCS check ([`crate::nlcs_check`]). They all give [`Problem`]s:
//! a stable code, a severity, a message in Dutch and English, what is
//! required against what is found, the object it is about, where it is,
//! and zero or more [`Fix`]es, each applied as one undo step.
//!
//! A problem can be accepted with a reason. The acceptance is kept in the
//! drawing (XRecord `OPENINFRA/PROBLEMEN`) with a fingerprint of the object
//! as it was; when the object changes the problem is open again.

use std::collections::HashSet;

use ocs_plugin_api::host::codec::{CadDocument, Circle, EntityType, Handle, Vector3};
use serde::{Deserialize, Serialize};

use super::{api, CivielOptions, Ctx, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::LayerSpec;

// ── The problem record ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn label(self) -> Text {
        match self {
            Severity::Error => Text::new("fout", "error"),
            Severity::Warning => Text::new("waarschuwing", "warning"),
            Severity::Info => Text::new("info", "info"),
        }
    }
}

/// A text in Dutch and English.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub nl: String,
    pub en: String,
}

impl Text {
    pub fn new(nl: impl Into<String>, en: impl Into<String>) -> Text {
        Text { nl: nl.into(), en: en.into() }
    }

    /// The text in the interface language.
    pub fn get(&self) -> &str {
        tr(&self.nl, &self.en)
    }
}

/// A value with its unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub value: f64,
    pub unit: String,
}

/// The object a problem is about: its entities (hex handles), what kind of
/// object it is and its name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct About {
    pub handles: Vec<String>,
    pub kind: String,
    pub name: String,
}

/// Where: project coordinates (m) and, along an alignment, the station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station: Option<f64>,
}

/// What a fix does. Lengths and radii in metres; handles in hex.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Draw the axis again from its layout with these radii, one per PI
    /// (0 at the ends and at corners).
    AxisRadii { axis: String, radii: Vec<f64> },
    /// Give the design profile these curve lengths of their own, one per
    /// PVI (`None`: the profile's curve length).
    CurveLengths { profile: String, lengths: Vec<Option<f64>> },
    /// `CIV_RIOOL_BIJWERKEN RIOOL=<sewer> [PUTNR=<manhole> BOB=AUTO]`.
    SewerUpdate {
        sewer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manhole: Option<String>,
        #[serde(default)]
        invert_auto: bool,
    },
    /// The NLCS colour, lineweight and linetype for this layer.
    NlcsLayer { layer: String },
    /// Colour, lineweight and linetype of these elements ByLayer.
    NlcsByLayer { handles: Vec<String> },
    /// `CIV_BIJWERKEN`: length profiles and cross sections made again.
    UpdateProducts {},
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fix {
    /// Short and stable within its problem: `FIX=<id>` (or its number).
    pub id: String,
    pub label: Text,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Open,
    Accepted,
}

/// An accepted problem, as kept in the drawing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acceptance {
    pub code: String,
    pub object: String,
    pub fingerprint: String,
    pub reason: String,
    pub date: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    /// Stable, dotted: `alignment.curve_overlap`, `criteria.radius_min`, …
    pub code: String,
    pub severity: Severity,
    /// The check that found it: alignment, criteria, profile, sewer, grading,
    /// corridor, products, nlcs.
    pub check: String,
    /// The object reference, `<kind>:<key>[/<part>]`: with `code` it names
    /// the problem (`OBJECT=` of the commands).
    pub object: String,
    /// A short title and the full message.
    pub title: Text,
    pub message: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Measure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<Measure>,
    pub about: About,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// The source of the limit (design guidelines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub fixes: Vec<Fix>,
    /// Why there is no fix, when the program looked for one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_fix: Option<Text>,
    #[serde(default)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<Acceptance>,
    /// Accepted before, but the object has changed since.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub acceptance_outdated: bool,
    #[serde(default)]
    pub fingerprint: String,
    /// The marker entities in the drawing (hex handles).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
}

impl Problem {
    pub fn new(code: &str, severity: Severity, check: &str, object: impl Into<String>, title: Text, message: Text) -> Problem {
        Problem {
            code: code.into(),
            severity,
            check: check.into(),
            object: object.into(),
            title,
            message,
            required: None,
            found: None,
            about: About::default(),
            location: None,
            source: None,
            fixes: Vec::new(),
            no_fix: None,
            status: Status::Open,
            accepted: None,
            acceptance_outdated: false,
            fingerprint: String::new(),
            markers: Vec::new(),
        }
    }

    pub fn required(mut self, value: f64, unit: &str) -> Problem {
        self.required = Some(Measure { value, unit: unit.into() });
        self
    }

    pub fn found(mut self, value: f64, unit: &str) -> Problem {
        self.found = Some(Measure { value, unit: unit.into() });
        self
    }

    pub fn about(mut self, handles: &[Handle], kind: &str, name: &str) -> Problem {
        self.about = About { handles: handles.iter().map(|h| api::handle_text(*h)).collect(), kind: kind.into(), name: name.into() };
        self
    }

    /// At a project point (m), with the station along an alignment.
    pub fn at(mut self, p: XY, station: Option<f64>) -> Problem {
        self.location = Some(Location { x: p[0], y: p[1], station });
        self
    }

    pub fn source(mut self, s: impl Into<String>) -> Problem {
        self.source = Some(s.into());
        self
    }

    pub fn fix(mut self, id: &str, label: Text, action: Action) -> Problem {
        self.fixes.push(Fix { id: id.into(), label, action });
        self
    }

    pub fn no_fix(mut self, why: Text) -> Problem {
        self.no_fix = Some(why);
        self
    }

    /// The object's key: the reference without its part (`as:E9E`).
    pub fn key(&self) -> &str {
        object_key(&self.object)
    }

    /// The fix by id or by number (1 = the first); the first without one.
    pub fn find_fix(&self, which: Option<&str>) -> Option<&Fix> {
        match which.map(str::trim).filter(|w| !w.is_empty()) {
            None => self.fixes.first(),
            Some(w) => self.fixes.iter().find(|f| f.id.eq_ignore_ascii_case(w)).or_else(|| w.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| self.fixes.get(i))),
        }
    }
}

/// `as:E9E/PI1-PI2` → `as:E9E`.
pub fn object_key(object: &str) -> &str {
    object.split_once('/').map(|(k, _)| k).unwrap_or(object)
}

/// A number as Dutch (decimal comma) and English text.
pub fn nl(v: f64, d: usize) -> String {
    format!("{v:.d$}").replace('.', ",")
}

pub fn en(v: f64, d: usize) -> String {
    format!("{v:.d$}")
}

// ── Fingerprints and acceptances ────────────────────────────────────────────

/// FNV-1a, 64 bit, as hex.
pub fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The fingerprint of a problem: its code and object, what is required and
/// found, and `state`, the state of the objects it is about (their geometry
/// and object data). The same while nothing changes.
pub fn fingerprint(p: &Problem, state: &str) -> String {
    let m = |x: &Option<Measure>| x.as_ref().map(|m| format!("{:.4}{}", m.value, m.unit)).unwrap_or_default();
    hash(&format!("{}|{}|{}|{}|{state}", p.code, p.object, m(&p.required), m(&p.found)))
}

/// Mark the accepted problems: an acceptance of the same code and object
/// with the same fingerprint. One with another fingerprint (the object
/// changed) leaves the problem open, marked `acceptance_outdated`.
pub fn apply_acceptances(problems: &mut [Problem], store: &[Acceptance]) {
    for p in problems.iter_mut() {
        p.status = Status::Open;
        p.accepted = None;
        p.acceptance_outdated = false;
        if let Some(a) = store.iter().find(|a| a.code == p.code && a.object == p.object) {
            if a.fingerprint == p.fingerprint {
                p.status = Status::Accepted;
                p.accepted = Some(a.clone());
            } else {
                p.acceptance_outdated = true;
            }
        }
    }
}

/// Accept `p` (replacing an earlier acceptance of the same problem).
pub fn accept_into(store: &mut Vec<Acceptance>, p: &Problem, reason: &str, date: &str, author: &str) -> Acceptance {
    store.retain(|a| !(a.code == p.code && a.object == p.object));
    let a = Acceptance { code: p.code.clone(), object: p.object.clone(), fingerprint: p.fingerprint.clone(), reason: reason.trim().into(), date: date.into(), author: author.trim().into() };
    store.push(a.clone());
    a
}

/// Drop the acceptances of problems that are no longer there, for objects
/// whose checks ran (`covered`); how many went.
pub fn prune(store: &mut Vec<Acceptance>, problems: &[Problem], covered: &dyn Fn(&str) -> bool) -> usize {
    let before = store.len();
    store.retain(|a| !covered(&a.object) || problems.iter().any(|p| p.code == a.code && p.object == a.object));
    before - store.len()
}

/// The problem with this code and object: exactly, or the only one with
/// this code whose object starts with `object` (a handle or key alone).
pub fn find<'a>(problems: &'a [Problem], code: &str, object: &str) -> Result<&'a Problem, String> {
    let same = |p: &&Problem| p.code.eq_ignore_ascii_case(code);
    if let Some(p) = problems.iter().filter(same).find(|p| p.object == object) {
        return Ok(p);
    }
    let loose: Vec<&Problem> = problems.iter().filter(same).filter(|p| p.object.starts_with(object) || p.key().ends_with(&format!(":{object}")) || p.object.contains(&format!(":{object}/"))).collect();
    match loose.len() {
        1 => Ok(loose[0]),
        0 => Err(format!("{} {code} {} {object}", tr("geen probleem", "no problem"), tr("bij", "at"))),
        n => Err(format!("{n} {} {code}: {}", tr("problemen", "problems"), loose.iter().map(|p| p.object.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// The record in the drawing that keeps the acceptances.
pub const RECORD: &str = "PROBLEMEN";

pub fn read_store() -> Vec<Acceptance> {
    bridge::json_record_read(RECORD).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_store(store: &[Acceptance]) -> Result<(), String> {
    bridge::json_record_write(RECORD, &serde_json::to_string(store).map_err(|e| e.to_string())?)
}

/// The author name for acceptances: `author` in Open Infra's settings.json.
pub fn author() -> String {
    let path = crate::gis::servers::config_dir().join("settings.json");
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v.get("author").and_then(|a| a.as_str()).map(str::to_string)).unwrap_or_default()
}

/// Keep the author name for acceptances in settings.json (empty removes it).
pub fn save_author(name: &str) -> Result<(), String> {
    let path = crate::gis::servers::config_dir().join("settings.json");
    let mut v: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    if name.trim().is_empty() {
        if let Some(o) = v.as_object_mut() {
            o.remove("author");
        }
    } else {
        v["author"] = serde_json::Value::String(name.trim().into());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

// ── Running the checks ──────────────────────────────────────────────────────

/// Which checks to run.
#[derive(Clone, Debug, PartialEq)]
pub struct Checks {
    pub alignment: bool,
    pub criteria: bool,
    pub profile: bool,
    pub sewer: bool,
    pub grading: bool,
    pub corridor: bool,
    pub products: bool,
    pub nlcs: bool,
}

impl Checks {
    /// Every check; NLCS with the Dutch modules (or when asked for).
    pub fn all() -> Checks {
        Checks { alignment: true, criteria: true, profile: true, sewer: true, grading: true, corridor: true, products: true, nlcs: crate::lang::dutch() }
    }

    pub const NAMES: [&'static str; 8] = ["alignment", "criteria", "profile", "sewer", "grading", "corridor", "products", "nlcs"];

    /// `CONTROLES=alignment;criteria;…` (Dutch names too).
    pub fn parse(list: &str) -> Result<Checks, String> {
        let mut c = Checks { alignment: false, criteria: false, profile: false, sewer: false, grading: false, corridor: false, products: false, nlcs: false };
        for w in list.split([';', ',']).map(|w| w.trim().to_ascii_lowercase()).filter(|w| !w.is_empty()) {
            match w.as_str() {
                "alles" | "all" => c = Checks { nlcs: true, ..Checks::all() },
                "as" | "alignment" | "layout" => c.alignment = true,
                "criteria" | "ontwerpcriteria" => c.criteria = true,
                "profiel" | "profile" | "ontwerpprofiel" => c.profile = true,
                "riool" | "sewer" | "riolering" => c.sewer = true,
                "grading" | "grondwerk" => c.grading = true,
                "corridor" => c.corridor = true,
                "producten" | "products" | "bijwerken" => c.products = true,
                "nlcs" => c.nlcs = true,
                other => return Err(format!("CONTROLES: {} '{other}'; {}: {}", tr("onbekend", "unknown"), tr("kies uit", "choose from"), Checks::NAMES.join(", "))),
            }
        }
        Ok(c)
    }

    /// The checks an object kind needs (`as:`, `profiel:`, `riool:`, …).
    fn for_key(key: &str) -> Checks {
        let mut c = Checks { alignment: false, criteria: false, profile: false, sewer: false, grading: false, corridor: false, products: false, nlcs: false };
        match key.split_once(':').map(|(k, _)| k).unwrap_or("") {
            "as" | "profiel" => {
                c.alignment = true;
                c.criteria = true;
                c.profile = true;
            }
            "riool" => c.sewer = true,
            "grading" => c.grading = true,
            "corridor" => c.corridor = true,
            "product" => c.products = true,
            "laag" => c.nlcs = true,
            _ => c = Checks::all(),
        }
        c
    }

    /// Whether these checks look at the object `object` refers to.
    pub fn covers(&self, object: &str) -> bool {
        let n = Checks::for_key(object_key(object));
        let pairs = [(n.alignment, self.alignment), (n.criteria, self.criteria), (n.profile, self.profile), (n.sewer, self.sewer), (n.grading, self.grading), (n.corridor, self.corridor), (n.products, self.products), (n.nlcs, self.nlcs)];
        pairs.iter().all(|(need, have)| !need || *have)
    }

    fn enabled(&self) -> Vec<&'static str> {
        let on = [self.alignment, self.criteria, self.profile, self.sewer, self.grading, self.corridor, self.products, self.nlcs];
        Checks::NAMES.iter().zip(on).filter(|(_, o)| *o).map(|(n, _)| *n).collect()
    }
}

/// Which objects a run covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    All,
    /// The objects of the selected entities.
    Selection,
    /// Objects by key (`as:E9E`, `riool:Riool 1`).
    Objects(Vec<String>),
}

/// What a run found.
pub struct Found {
    pub problems: Vec<Problem>,
    /// Remarks: checks that could not run, and why.
    pub notes: Vec<String>,
    pub checks: Vec<&'static str>,
}

/// The axes of a drawing: polylines on AM-AS that are not offsets,
/// profiles or tracks.
pub fn axes(doc: &CadDocument) -> Vec<&EntityType> {
    doc.model_space_entities()
        .filter(|e| {
            matches!(e, EntityType::LwPolyline(_))
                && e.common().layer.contains("-AM-AS-")
                && super::data_value(e, "as").is_some()
                && super::data_value(e, "verschuiving_m").is_none()
                && super::data_value(e, "ontwerpprofiel").is_none()
                && super::data_value(e, "lengteprofiel").is_none()
        })
        .collect()
}

/// The newest design profile line of the axis named `axis`.
pub fn design_of<'a>(doc: &'a CadDocument, axis: &str) -> Option<&'a EntityType> {
    doc.model_space_entities().filter(|e| super::data_value(e, "ontwerpprofiel").is_some() && super::data_value(e, "as").as_deref() == Some(axis)).last()
}

/// Run the checks over the drawing.
pub fn collect(opt: &CivielOptions, scope: &Scope, checks: &Checks, progress: &dyn Fn(String)) -> Result<Found, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let mut checks = checks.clone();
    if let Scope::Objects(keys) = scope {
        // Only what the objects need.
        let mut c = Checks { alignment: false, criteria: false, profile: false, sewer: false, grading: false, corridor: false, products: false, nlcs: false };
        for k in keys {
            let n = Checks::for_key(k);
            c.alignment |= n.alignment;
            c.criteria |= n.criteria;
            c.profile |= n.profile;
            c.sewer |= n.sewer;
            c.grading |= n.grading;
            c.corridor |= n.corridor;
            c.products |= n.products;
            c.nlcs |= n.nlcs;
        }
        checks = c;
    }
    let kinds: HashSet<String> = bridge::index_read().into_iter().map(|(k, _, _)| k).collect();
    let mut needs = vec!["as", "ontwerpprofiel", "lengteprofiel", "corridor", "grading"];
    if checks.sewer && kinds.contains("riool") {
        needs.extend(["riool", "riooltoets", "tin"]);
    }
    // Out-of-date profiles compare the ground too.
    if checks.products && !needs.contains(&"tin") && !super::graaf::read().is_empty() {
        needs.push("tin");
    }
    progress(tr("Tekening lezen…", "Reading the drawing…").into());
    let doc = super::document(&needs, &[])?;
    let mut out: Vec<Problem> = Vec::new();
    let mut notes = Vec::new();
    if checks.alignment || checks.criteria || checks.profile {
        progress(tr("Assen en profielen…", "Alignments and profiles…").into());
        for e in axes(&doc) {
            let Some(al) = super::as_profiel::Alignment::from_entity(e) else { continue };
            let h = e.common().handle;
            let stored = super::curve_fit::Layout::from_entity(e, f);
            let layout = stored.clone().or_else(|| super::curve_fit::Layout::from_alignment(&al));
            let design = design_of(&doc, &al.name);
            if checks.alignment {
                if let Some(lay) = &stored {
                    out.extend(super::as_profiel::layout_problems(&ctx, e, &al, lay));
                }
            }
            if checks.criteria {
                out.extend(super::ontwerpcriteria::problems(&ctx, e, &al, layout.as_ref(), design));
            }
            if checks.profile {
                if let Some(d) = design {
                    out.extend(super::as_profiel::profile_problems(&ctx, h, &al, d));
                }
            }
        }
    }
    if checks.sewer && kinds.contains("riool") {
        progress(tr("Riolering…", "Sewers…").into());
        match super::riool::problems(opt, &ctx, &doc, progress) {
            Ok(p) => out.extend(p),
            Err(e) => notes.push(format!("{}: {e}", tr("riolering", "sewers"))),
        }
    }
    if checks.grading {
        out.extend(super::grading::problems(&ctx, &doc));
    }
    if checks.corridor {
        out.extend(super::corridor::problems(&ctx, &doc));
    }
    if checks.products {
        out.extend(super::graaf::problems(&ctx, &doc));
    }
    let mut full: Option<CadDocument> = None;
    if checks.nlcs {
        progress(tr("NLCS-controle…", "NLCS check…").into());
        match bridge::snapshot() {
            Ok(d) => {
                let o = crate::nlcs_check::Options { unit_factor: f, layer_props: !bridge::partial(), scale: opt.nlcs.scale, ..Default::default() };
                out.extend(crate::nlcs_check::problems(&crate::nlcs_check::check(&d, &o)));
                full = Some(d);
            }
            Err(e) => notes.push(format!("NLCS: {e}")),
        }
    }
    // The scope.
    match scope {
        Scope::All => {}
        Scope::Selection => {
            let sel: HashSet<String> = bridge::selection()?.into_iter().map(api::handle_text).collect();
            out.retain(|p| p.about.handles.iter().any(|h| sel.contains(h)));
        }
        Scope::Objects(keys) => out.retain(|p| keys.iter().any(|k| p.key() == k.as_str())),
    }
    // Fingerprints of the objects as they are now.
    for p in out.iter_mut() {
        let source = full.as_ref().unwrap_or(&doc);
        let state: Vec<String> = if p.about.handles.len() <= 200 {
            p.about.handles.iter().filter_map(|h| api::parse_handle(h).ok()).filter_map(|h| source.get_entity(h)).map(super::labels::entity_fingerprint).collect()
        } else {
            vec![format!("{}", p.about.handles.len())]
        };
        p.fingerprint = fingerprint(p, &state.join(";"));
    }
    apply_acceptances(&mut out, &read_store());
    out.sort_by(|a, b| (a.status, a.severity, &a.about.name, &a.object, &a.code).cmp(&(b.status, b.severity, &b.about.name, &b.object, &b.code)));
    Ok(Found { problems: out, notes, checks: checks.enabled() })
}

// ── Markers ─────────────────────────────────────────────────────────────────

/// The layers of the markers: errors, warnings and info, accepted.
pub const LAYERS: [(&str, u8); 3] = [("OI-PROBLEMEN", 1), ("OI-PROBLEMEN-LET-OP", 30), ("OI-PROBLEMEN-GEACCEPTEERD", 8)];
const MARKERS: &str = "problemen";

fn marker_layer(p: &Problem) -> LayerSpec {
    let (name, aci) = match (p.status, p.severity) {
        (Status::Accepted, _) => LAYERS[2],
        (_, Severity::Error) => LAYERS[0],
        _ => LAYERS[1],
    };
    LayerSpec { name: name.into(), color: aci, rgb: crate::nlcs::color_rgb(aci), true_color: None, lineweight: Some(35), linetype: "CONTINUOUS".into(), object_id: 0 }
}

/// Draw the markers of `problems` on the problem layers, in place of the old
/// ones of the same scope, in one undo step; their handles go into the
/// problems. Problems without a place get none.
pub fn draw_markers(ctx: &Ctx, problems: &mut [Problem], scope: &Scope) -> Result<usize, String> {
    let k = ctx.k;
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(kind, _, _)| kind == MARKERS).flat_map(|(_, _, h)| h).collect();
    // Which old markers this run replaces.
    let (gone, kept): (Vec<Handle>, Vec<Handle>) = match scope {
        Scope::All => (old, Vec::new()),
        _ => {
            let sel: HashSet<String> = if *scope == Scope::Selection { bridge::selection().unwrap_or_default().into_iter().map(api::handle_text).collect() } else { HashSet::new() };
            let mut gone = Vec::new();
            let mut kept = Vec::new();
            for chunk in old.chunks(200) {
                for (h, e) in chunk.iter().zip(bridge::entities(chunk).unwrap_or_default()) {
                    let Some(e) = e else { continue };
                    let object = super::data_value(&e, "probleem_object").unwrap_or_default();
                    let hs = super::data_value(&e, "probleem_handles").unwrap_or_default();
                    let hit = match scope {
                        Scope::Objects(keys) => keys.iter().any(|key| object_key(&object) == key.as_str()),
                        _ => hs.split(' ').any(|x| sel.contains(x)),
                    };
                    if hit { gone.push(*h) } else { kept.push(*h) }
                }
            }
            (gone, kept)
        }
    };
    let mut out = Out::default();
    let mut owners: Vec<usize> = Vec::new();
    for (i, p) in problems.iter().enumerate() {
        let Some(loc) = &p.location else { continue };
        let spec = marker_layer(p);
        let c = ctx.to_drawing([loc.x, loc.y]);
        let data = super::record(&[
            ("probleem", p.code.clone()),
            ("probleem_object", p.object.clone()),
            ("probleem_status", if p.status == Status::Accepted { "accepted".into() } else { "open".into() }),
            ("probleem_handles", p.about.handles.iter().take(12).cloned().collect::<Vec<_>>().join(" ")),
        ]);
        let mut circle = EntityType::Circle(Circle::from_center_radius(Vector3::new(c[0], c[1], 0.0), 4.0 * k));
        let cm = circle.common_mut();
        cm.layer = spec.name.clone();
        cm.color = ocs_plugin_api::host::codec::Color::ByLayer;
        cm.extended_data.add_record(data.clone());
        let mark = match (p.status, p.severity) {
            (Status::Accepted, _) => "✓",
            (_, Severity::Error) => "✗",
            _ => "!",
        };
        let label = format!("{mark} {}", p.title.get());
        let t = ctx.text([c[0] + 5.0 * k, c[1] + 2.5 * k], &label, 2.5, 0.0, &spec, Some(&data));
        owners.push(i);
        out.put(&spec, [circle, t]);
    }
    // Write by problem order: `Out` keeps the order per layer, so put each
    // problem's pair on its layer and read the handles back by layer.
    let mut by_layer: Vec<(String, Vec<usize>)> = Vec::new();
    for &i in &owners {
        let name = marker_layer(&problems[i]).name;
        match by_layer.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => v.push(i),
            None => by_layer.push((name, vec![i])),
        }
    }
    let n_new = owners.len() * 2;
    let handles = bridge::undo_group(tr("Problemen", "Problems"), || {
        if !gone.is_empty() {
            bridge::remove_entities(tr("Problemen", "Problems"), &gone)?;
        }
        if out.is_empty() { Ok(Vec::new()) } else { out.write_handles(tr("Problemen", "Problems")) }
    })?;
    let mut at = 0;
    for (_, list) in &by_layer {
        for i in list {
            problems[*i].markers = handles.get(at..at + 2).map(|s| s.iter().map(|h| api::handle_text(*h)).collect()).unwrap_or_default();
            at += 2;
        }
    }
    let mut all = kept;
    all.extend(&handles);
    bridge::index_set_many(MARKERS, &[("markers".to_string(), all)]);
    Ok(n_new.min(handles.len()))
}

// ── Applying a fix ──────────────────────────────────────────────────────────

/// Apply a fix: one undo step.
pub fn apply(opt: &CivielOptions, action: &Action, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    match action {
        Action::AxisRadii { axis, radii } => {
            let h = api::parse_handle(axis)?;
            let e = bridge::entities(&[h])?.into_iter().next().flatten().ok_or_else(|| format!("{} {axis}", tr("geen element met handle", "no entity with handle")))?;
            let al = super::as_profiel::Alignment::from_entity(&e).ok_or_else(|| format!("{axis}: {}", tr("geen as", "not an alignment")))?;
            let layout = super::curve_fit::Layout::from_entity(&e, f)
                .or_else(|| super::curve_fit::Layout::from_alignment(&al))
                .ok_or(tr("de PI's van deze as zijn niet bekend", "the PIs of this alignment are not known"))?;
            if layout.len() != radii.len() {
                return Err(tr("de as is veranderd: voer CIV_PROBLEMEN opnieuw uit", "the alignment has changed: run CIV_PROBLEMEN again").into());
            }
            let r: Vec<f64> = radii.iter().map(|r| r * f).collect();
            super::as_profiel::rebuild_axis(opt, h, &layout, &r)
        }
        Action::CurveLengths { profile, lengths } => {
            let h = api::parse_handle(profile)?;
            let e = bridge::entities(&[h])?.into_iter().next().flatten().ok_or_else(|| format!("{} {profile}", tr("geen element met handle", "no entity with handle")))?;
            let mut pd = super::as_profiel::ProfileData::of(&e).ok_or(tr("het ontwerpprofiel mist zijn PVI's", "the design profile lacks its PVIs"))?;
            if pd.points.len() != lengths.len() {
                return Err(tr("het ontwerpprofiel is veranderd: voer CIV_PROBLEMEN opnieuw uit", "the design profile has changed: run CIV_PROBLEMEN again").into());
            }
            pd.own = lengths.clone();
            let axis = super::data_value(&e, "as").unwrap_or_default();
            let doc = super::document(&["lengteprofiel"], &[])?;
            let fr = super::as_profiel::ProfileFrame::newest(&doc, &axis).ok_or_else(|| format!("{} {axis}", tr("geen lengteprofiel van", "no length profile of")))?;
            super::as_profiel::draw_design(&ctx, &fr, &pd, Some(h))
        }
        Action::SewerUpdate { sewer, manhole, invert_auto } => {
            let mut o = opt.clone();
            o.nlcs.status = crate::nlcs::Status::N;
            let w = super::riool::Wijziging { put: manhole.clone(), bob: invert_auto.then_some(None), ..Default::default() };
            bridge::undo_group(&format!("{} {sewer}", tr("Riool bijwerken", "Update sewer")), || super::riool::bijwerken(&o, Some(sewer.clone()), w, progress))
        }
        Action::NlcsLayer { layer } => {
            let doc = bridge::snapshot()?;
            let o = crate::nlcs_check::Options { unit_factor: f, layer_props: !bridge::partial(), ..Default::default() };
            let rep = crate::nlcs_check::check(&doc, &o);
            let repairs: Vec<&crate::nlcs_check::Repair> = rep.findings.iter().filter(|x| x.layer == *layer).filter_map(|x| x.repair.as_ref()).filter(|r| matches!(r, crate::nlcs_check::Repair::LayerStyle(_))).collect();
            if repairs.is_empty() {
                return Err(format!("{layer}: {}", tr("niets te herstellen", "nothing to repair")));
            }
            crate::nlcs_check::repair(&doc, &repairs)
        }
        Action::NlcsByLayer { handles } => {
            let hs: Vec<Handle> = handles.iter().filter_map(|h| api::parse_handle(h).ok()).collect();
            let doc = bridge::partial_document(&hs, 500)?;
            crate::nlcs_check::repair(&doc, &[&crate::nlcs_check::Repair::ByLayer(hs.clone())])
        }
        Action::UpdateProducts {} => super::graaf::update(opt, false, progress),
    }
}

// ── The commands ────────────────────────────────────────────────────────────

fn counts(problems: &[Problem]) -> (usize, usize, usize, usize) {
    let open = |s: Severity| problems.iter().filter(|p| p.status == Status::Open && p.severity == s).count();
    (open(Severity::Error), open(Severity::Warning), open(Severity::Info), problems.iter().filter(|p| p.status == Status::Accepted).count())
}

fn note_found(found: &Found, markers: usize) {
    let (e, w, i, a) = counts(&found.problems);
    api::note("problems", serde_json::to_value(&found.problems).unwrap_or_default());
    api::note("open", e + w + i);
    api::note("errors", e);
    api::note("warnings", w);
    api::note("info", i);
    api::note("accepted", a);
    api::note("checks", found.checks.clone());
    api::note("notes", found.notes.clone());
    api::note("markers", markers);
    api::note("layers", LAYERS.iter().map(|l| l.0).collect::<Vec<_>>());
}

fn summary(found: &Found) -> String {
    let (e, w, i, a) = counts(&found.problems);
    let notes = if found.notes.is_empty() { String::new() } else { format!(" ({})", found.notes.join("; ")) };
    format!(
        "{}: {e} {}, {w} {}, {i} info, {a} {}{notes}.",
        tr("Problemen", "Problems"),
        tr("fout(en)", "error(s)"),
        tr("waarschuwing(en)", "warning(s)"),
        tr("geaccepteerd", "accepted")
    )
}

/// `CIV_PROBLEMEN [ALLES|SELECTIE] [CONTROLES=] [MARKERINGEN=nee]`.
pub fn run_command(opt: &CivielOptions, scope: Scope, checks: Checks, markers: bool, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let mut found = collect(opt, &scope, &checks, progress)?;
    // A run over the whole drawing forgets acceptances of problems that are
    // gone, for the checks that ran without trouble.
    if scope == Scope::All && found.notes.is_empty() {
        let mut store = read_store();
        if prune(&mut store, &found.problems, &|o| checks.covers(o)) > 0 {
            let _ = write_store(&store);
        }
    }
    let n = if markers { draw_markers(&ctx, &mut found.problems, &scope)? } else { 0 };
    note_found(&found, n);
    let list: Vec<String> = found.problems.iter().filter(|p| p.status == Status::Open).take(6).map(|p| format!("{} {}", p.code, p.object)).collect();
    Ok(format!("{}{}", summary(&found), if list.is_empty() { String::new() } else { format!(" {}", list.join("; ")) }))
}

/// `CIV_PROBLEEM_HERSTEL CODE= OBJECT= [FIX=]`: check the object again, apply
/// the fix and check once more.
pub fn fix_command(opt: &CivielOptions, code: &str, object: &str, which: Option<&str>, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let key = object_key(object).to_string();
    let scope = if key.contains(':') { Scope::Objects(vec![key.clone()]) } else { Scope::All };
    let checks = Checks::all();
    let found = collect(opt, &scope, &checks, progress)?;
    let p = find(&found.problems, code, object)?.clone();
    let fix = p.find_fix(which).cloned().ok_or_else(|| match &p.no_fix {
        Some(why) => format!("{} {}: {}", p.code, p.object, why.get()),
        None => format!("{} {}: {}", p.code, p.object, tr("geen herstel bekend", "no fix known")),
    })?;
    progress(fix.label.get().to_string());
    // The fix and the markers of the object as it is then: one undo step.
    let scope = Scope::Objects(vec![p.key().to_string()]);
    let (done, after, n) = bridge::undo_group(&format!("{} {}", tr("Herstel", "Fix"), p.code), || {
        let done = apply(opt, &fix.action, progress)?;
        let mut after = collect(opt, &scope, &checks, progress)?;
        let n = draw_markers(&ctx, &mut after.problems, &scope)?;
        Ok((done, after, n))
    })?;
    let still = after.problems.iter().any(|q| q.code == p.code && q.object == p.object);
    api::note("problem", serde_json::to_value(&p).unwrap_or_default());
    api::note("fix", serde_json::to_value(&fix).unwrap_or_default());
    api::note("applied", done.clone());
    api::note("solved", !still);
    note_found(&after, n);
    Ok(format!(
        "{} {} — {}: {done} {}",
        p.code,
        p.object,
        fix.label.get(),
        if still { tr("Het probleem is er nog.", "The problem is still there.") } else { tr("Opgelost.", "Solved.") }
    ))
}

/// `CIV_PROBLEEM_ACCEPTEER CODE= OBJECT= REDEN=`.
pub fn accept_command(opt: &CivielOptions, code: &str, object: &str, reason: &str, progress: &dyn Fn(String)) -> Result<String, String> {
    if reason.trim().is_empty() {
        return Err(tr("geef een reden: REDEN=\"…\"", "give a reason: REDEN=\"…\"").into());
    }
    let ctx = Ctx::new(opt)?;
    let key = object_key(object).to_string();
    let scope = if key.contains(':') { Scope::Objects(vec![key]) } else { Scope::All };
    let mut found = collect(opt, &scope, &Checks::all(), progress)?;
    let p = find(&found.problems, code, object)?.clone();
    let mut store = read_store();
    let a = accept_into(&mut store, &p, reason, &today(), &author());
    apply_acceptances(&mut found.problems, &store);
    let scope = Scope::Objects(vec![p.key().to_string()]);
    found.problems.retain(|q| q.key() == p.key());
    // The acceptance and the markers: one undo step.
    let n = bridge::undo_group(&format!("{} {}", tr("Accepteren", "Accept"), p.code), || {
        write_store(&store)?;
        draw_markers(&ctx, &mut found.problems, &scope)
    })?;
    api::note("acceptance", serde_json::to_value(&a).unwrap_or_default());
    note_found(&found, n);
    Ok(format!("{} {} {} ({}{}).", p.code, p.object, tr("geaccepteerd", "accepted"), a.reason, if a.author.is_empty() { String::new() } else { format!(", {}", a.author) }))
}

/// Withdraw an acceptance (the window's "open again").
pub fn reopen(code: &str, object: &str) -> Result<String, String> {
    let mut store = read_store();
    let before = store.len();
    store.retain(|a| !(a.code == code && a.object == object));
    if store.len() == before {
        return Err(format!("{code} {object}: {}", tr("niet geaccepteerd", "not accepted")));
    }
    write_store(&store)?;
    Ok(format!("{code} {object}: {}", tr("weer open", "open again")))
}

/// Zoom to a problem: its marker when it has one, else its object; the
/// object is selected afterwards.
pub fn zoom(p: &Problem) -> Result<(), String> {
    let hs = |v: &[String]| v.iter().filter_map(|h| api::parse_handle(h).ok()).collect::<Vec<Handle>>();
    let target = if p.markers.is_empty() { hs(&p.about.handles) } else { hs(&p.markers) };
    if target.is_empty() {
        return Err(tr("dit probleem heeft geen plek in de tekening", "this problem has no place in the drawing").into());
    }
    bridge::select(&target)?;
    bridge::execute("ZOOM OBJECT")?;
    let about = hs(&p.about.handles);
    if !about.is_empty() {
        bridge::select(&about)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Problem {
        Problem::new("criteria.radius_min", Severity::Error, "criteria", "as:E9E/PI3", Text::new("R < Rmin", "R < Rmin"), Text::new("boog", "curve"))
            .required(160.0, "m")
            .found(80.0, "m")
            .fix("rmin", Text::new("R = 160 m", "R = 160 m"), Action::AxisRadii { axis: "E9E".into(), radii: vec![0.0, 300.0, 160.0, 0.0] })
            .fix("other", Text::new("x", "x"), Action::UpdateProducts {})
    }

    #[test]
    fn fingerprints_follow_the_object() {
        let p = sample();
        let a = fingerprint(&p, "geometry-1");
        assert_eq!(a, fingerprint(&p, "geometry-1"));
        assert_eq!(a.len(), 16);
        // Another geometry, another value found, another limit: another print.
        assert_ne!(a, fingerprint(&p, "geometry-2"));
        assert_ne!(a, fingerprint(&p.clone().found(81.0, "m"), "geometry-1"));
        assert_ne!(a, fingerprint(&p.clone().required(250.0, "m"), "geometry-1"));
        // FNV-1a of the empty string is its offset basis.
        assert_eq!(hash(""), "cbf29ce484222325");
    }

    #[test]
    fn accepted_until_the_object_changes() {
        let mut p = sample();
        p.fingerprint = fingerprint(&p, "g1");
        let mut store = Vec::new();
        let a = accept_into(&mut store, &p, "  tight site, agreed with the client ", "2026-11-05", "C. Engineer");
        assert_eq!(a.reason, "tight site, agreed with the client");
        let mut list = vec![p.clone()];
        apply_acceptances(&mut list, &store);
        assert_eq!(list[0].status, Status::Accepted);
        assert_eq!(list[0].accepted.as_ref().unwrap().author, "C. Engineer");
        assert!(!list[0].acceptance_outdated);
        // The axis moved: open again, and marked as accepted before.
        let mut moved = p.clone();
        moved.fingerprint = fingerprint(&moved, "g2");
        let mut list = vec![moved];
        apply_acceptances(&mut list, &store);
        assert_eq!(list[0].status, Status::Open);
        assert!(list[0].acceptance_outdated && list[0].accepted.is_none());
        // Accepting again replaces the old acceptance.
        accept_into(&mut store, &list[0], "still fine", "2026-11-06", "");
        assert_eq!(store.len(), 1);
        assert_eq!(store[0].fingerprint, list[0].fingerprint);
        // Another problem on the same object is not accepted by it.
        let mut other = sample();
        other.code = "criteria.clothoid_min".into();
        other.fingerprint = fingerprint(&other, "g2");
        let mut list = vec![other];
        apply_acceptances(&mut list, &store);
        assert_eq!(list[0].status, Status::Open);
        // Gone problems drop out of the store, when their check ran.
        assert_eq!(prune(&mut store, &list, &|o| o.starts_with("riool:")), 0);
        assert_eq!(prune(&mut store, &list, &|o| o.starts_with("as:")), 1);
        assert!(store.is_empty());
        // The record survives JSON.
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"severity\":\"error\"") && json.contains("\"kind\":\"axis_radii\""));
        assert_eq!(serde_json::from_str::<Problem>(&json).unwrap(), p);
    }

    #[test]
    fn problems_and_fixes_by_reference() {
        let p = sample();
        assert_eq!(p.key(), "as:E9E");
        assert_eq!(p.find_fix(None).unwrap().id, "rmin");
        assert_eq!(p.find_fix(Some("2")).unwrap().id, "other");
        assert_eq!(p.find_fix(Some("RMIN")).unwrap().id, "rmin");
        assert!(p.find_fix(Some("3")).is_none());
        let list = vec![p.clone(), Problem { object: "as:E9E/PI4".into(), ..p.clone() }];
        assert_eq!(find(&list, "criteria.radius_min", "as:E9E/PI3").unwrap().object, "as:E9E/PI3");
        // A handle alone is ambiguous here, unique with one problem.
        assert!(find(&list, "criteria.radius_min", "E9E").is_err());
        assert_eq!(find(&list[..1], "criteria.radius_min", "E9E").unwrap().object, "as:E9E/PI3");
        assert!(find(&list, "criteria.grade_max", "as:E9E/PI3").is_err());
        assert_eq!(Checks::parse("as;riool").unwrap(), Checks { alignment: true, sewer: true, criteria: false, profile: false, grading: false, corridor: false, products: false, nlcs: false });
        assert!(Checks::parse("bogus").is_err());
    }
}
