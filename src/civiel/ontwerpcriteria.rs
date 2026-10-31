//! Design criteria (`CIV_ONTWERPCRITERIA`,
//! `CIV_AS_TOETS`): per guideline, road category and design speed the
//! minimum radius, clothoid parameter, grades, vertical curve radii,
//! superelevation and stopping sight distance, from the tables in
//! `assets/criteria/*.json` (every value with its source, estimates marked)
//! and from `<config>/OpenCADStudio/open-infra/criteria/*.json` (own tables
//! in the same form). The check runs the axis and its design profile
//! against one choice and reports each violation with its stations.

use std::collections::BTreeMap;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};
use serde::{Deserialize, Serialize};

use super::as_profiel::{self, Alignment, Pvi, Seg};
use super::{record, station, Ctx, Out};
use crate::gis::crs::XY;
use crate::nlcs::Element;

/// The built-in tables.
const BUILT_IN: &[(&str, &str)] = &[
    ("roa", include_str!("../../assets/criteria/roa.json")),
    ("crow", include_str!("../../assets/criteria/crow.json")),
    ("noa", include_str!("../../assets/criteria/noa.json")),
    ("aashto", include_str!("../../assets/criteria/aashto.json")),
];

/// One design value and where it comes from.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Value {
    pub v: f64,
    /// Key into the guideline's `bronnen`.
    pub bron: String,
    /// Not verified against the source: an estimate.
    #[serde(default)]
    pub schatting: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub opmerking: String,
}

/// The values for one design speed.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Limits {
    /// Design speed (km/h).
    pub v: f64,
    /// Minimum horizontal radius (m).
    #[serde(default)]
    pub r_min: Option<Value>,
    /// Minimum clothoid parameter A (m).
    #[serde(default)]
    pub a_min: Option<Value>,
    /// Curves below this radius (m) need clothoids towards straights.
    #[serde(default)]
    pub r_overgang: Option<Value>,
    /// Maximum and minimum longitudinal grade (%).
    #[serde(default)]
    pub helling_max: Option<Value>,
    #[serde(default)]
    pub helling_min: Option<Value>,
    /// Minimum vertical curve radius, crest and sag (m).
    #[serde(default)]
    pub r_top_min: Option<Value>,
    #[serde(default)]
    pub r_voet_min: Option<Value>,
    /// Maximum superelevation (%).
    #[serde(default)]
    pub verkanting_max: Option<Value>,
    /// Stopping sight distance (m).
    #[serde(default)]
    pub stopzicht: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Category {
    pub id: String,
    pub naam: String,
    pub snelheden: Vec<Limits>,
}

/// One guideline: its categories and sources.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Guideline {
    pub id: String,
    pub naam: String,
    #[serde(default)]
    pub land: String,
    /// Eye and object height for sight distance (m).
    #[serde(default)]
    pub ooghoogte: Option<Value>,
    #[serde(default)]
    pub objecthoogte: Option<Value>,
    /// The clothoid rule A ≥ R / n: n.
    #[serde(default)]
    pub a_r_deler: Option<Value>,
    pub categorieen: Vec<Category>,
    /// Source key → full citation.
    pub bronnen: BTreeMap<String, String>,
}

impl Guideline {
    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categorieen.iter().find(|c| c.id.eq_ignore_ascii_case(id))
    }

    /// The citation of a value: "ROA 2019, tabel 4.3" (with "schatting" when so).
    pub fn cite(&self, v: &Value) -> String {
        let src = self.bronnen.get(&v.bron).cloned().unwrap_or_else(|| v.bron.clone());
        if v.schatting { format!("{src} (schatting)") } else { src }
    }
}

/// Every table: the built-in ones, then the user's own (same id replaces).
pub fn guidelines() -> Vec<Guideline> {
    let mut out: Vec<Guideline> = BUILT_IN.iter().filter_map(|(_, t)| serde_json::from_str(t).ok()).collect();
    let dir = crate::gis::servers::config_dir().join("criteria");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) {
                if let Some(g) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Guideline>(&t).ok()) {
                    out.retain(|o| o.id != g.id);
                    out.push(g);
                }
            }
        }
    }
    out
}

/// A choice of guideline, category and design speed.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub guideline: String,
    pub category: String,
    pub v: f64,
}

impl Choice {
    /// "roa;autosnelweg;120", as kept on an axis.
    pub fn parse(s: &str) -> Option<Choice> {
        let mut p = s.split(';');
        let (g, c, v) = (p.next()?.trim(), p.next()?.trim(), p.next()?.trim().replace(',', ".").parse().ok()?);
        Some(Choice { guideline: g.to_ascii_lowercase(), category: c.to_ascii_lowercase(), v })
    }

    pub fn text(&self) -> String {
        format!("{};{};{}", self.guideline, self.category, self.v)
    }

    /// The guideline and the limits for this choice; an error lists what there is.
    pub fn resolve(&self, all: &[Guideline]) -> Result<(Guideline, Category, Limits), String> {
        let g = all
            .iter()
            .find(|g| g.id.eq_ignore_ascii_case(&self.guideline))
            .ok_or_else(|| {
                let ids = all.iter().map(|g| g.id.as_str()).collect::<Vec<_>>().join(", ");
                tx(format!("onbekende richtlijn '{}'; kies {ids}", self.guideline), format!("unknown guideline '{}'; choose {ids}", self.guideline))
            })?;
        let c = g
            .category(&self.category)
            .ok_or_else(|| {
                let ids = g.categorieen.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ");
                tx(format!("{} kent geen categorie '{}'; kies {ids}", g.id, self.category), format!("{} has no category '{}'; choose {ids}", g.id, self.category))
            })?;
        let l = c.snelheden.iter().find(|l| (l.v - self.v).abs() < 1e-6).ok_or_else(|| {
            let vs = c.snelheden.iter().map(|l| format!("{}", l.v)).collect::<Vec<_>>().join(", ");
            tx(format!("{} {} heeft geen ontwerpsnelheid {} km/h; kies {vs}", g.id, c.id, self.v), format!("{} {} has no design speed {} km/h; choose {vs}", g.id, c.id, self.v))
        })?;
        Ok((g.clone(), c.clone(), l.clone()))
    }
}

/// The choice of this session (`CIV_ONTWERPCRITERIA` without an axis).
pub fn session() -> Option<Choice> {
    store().lock().unwrap().clone()
}

fn store() -> &'static std::sync::Mutex<Option<Choice>> {
    static S: std::sync::OnceLock<std::sync::Mutex<Option<Choice>>> = std::sync::OnceLock::new();
    S.get_or_init(|| std::sync::Mutex::new(None))
}

/// The object data key of an axis's design criteria.
pub const KEY: &str = "ontwerpcriteria";

/// The choice for an axis: the arguments, else what the axis carries, else
/// the session's.
pub fn choice_for(arg: Option<Choice>, axis: Option<&EntityType>) -> Option<Choice> {
    arg.or_else(|| axis.and_then(|e| super::data_value(e, KEY)).and_then(|s| Choice::parse(&s))).or_else(session)
}

/// One violation: where (m from the start of the axis), what, the value,
/// the limit and its source.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub s0: f64,
    pub s1: f64,
    /// boogstraal, clothoide, clothoide_r, overgangsboog, helling_max,
    /// helling_min, topboog, voetboog, verkanting.
    pub check: &'static str,
    pub value: f64,
    pub limit: f64,
    pub source: String,
    /// "fout" (violation) or "let op" (advice, such as a flat grade).
    pub severity: &'static str,
    pub text: String,
}

fn comma(v: f64, d: usize) -> String {
    format!("{v:.d$}").replace('.', ",")
}

/// The horizontal alignment against the limits (`f` drawing units per m).
pub fn check_plan(al: &Alignment, f: f64, g: &Guideline, l: &Limits) -> Vec<Finding> {
    let mut out = Vec::new();
    let starts = al.seg_starts();
    for (i, seg) in al.segs.iter().enumerate() {
        let (s0, s1) = (starts[i] / f, (starts[i] + seg.length()) / f);
        match seg {
            Seg::Arc { r, .. } => {
                let r = r / f;
                if let Some(min) = &l.r_min {
                    if r < min.v - 1e-6 {
                        out.push(Finding { s0, s1, check: "boogstraal", value: r, limit: min.v, source: g.cite(min), severity: "fout", text: format!("R = {} m < {} m", comma(r, 1), comma(min.v, 0)) });
                    }
                }
                // A straight right against a curve that needs clothoids.
                if let Some(req) = l.r_overgang.as_ref().filter(|v| r < v.v) {
                    let straight = |j: Option<usize>| j.and_then(|j| al.segs.get(j)).is_some_and(|s| matches!(s, Seg::Line { .. }));
                    if straight(i.checked_sub(1)) || straight(Some(i + 1)) {
                        let text = if nl() { format!("boog R = {} m zonder overgangsboog (nodig onder R = {} m)", comma(r, 0), comma(req.v, 0)) } else { format!("curve R = {} m without transition curve (needed below R = {} m)", comma(r, 0), comma(req.v, 0)) };
                        out.push(Finding { s0, s1, check: "overgangsboog", value: r, limit: req.v, source: g.cite(req), severity: "fout", text });
                    }
                }
            }
            Seg::Spiral { .. } => {
                let Some(a) = seg.spiral_a().map(|a| a / f) else { continue };
                if let Some(min) = &l.a_min {
                    if a < min.v - 1e-6 {
                        out.push(Finding { s0, s1, check: "clothoide", value: a, limit: min.v, source: g.cite(min), severity: "fout", text: format!("A = {} m < {} m", comma(a, 1), comma(min.v, 0)) });
                    }
                }
                // A ≥ R / n, with R the sharpest radius the clothoid reaches.
                if let Some(n) = &g.a_r_deler {
                    let kmax = seg.curvature(0.0).abs().max(seg.curvature(seg.length()).abs());
                    if kmax > 0.0 && n.v > 0.0 {
                        let r = 1.0 / kmax / f;
                        if a < r / n.v - 1e-6 {
                            out.push(Finding {
                                s0,
                                s1,
                                check: "clothoide_r",
                                value: a,
                                limit: r / n.v,
                                source: g.cite(n),
                                severity: "fout",
                                text: format!("A = {} m < R/{} = {} m", comma(a, 1), comma(n.v, 0), comma(r / n.v, 1)),
                            });
                        }
                    }
                }
            }
            Seg::Line { .. } => {}
        }
    }
    out
}

fn grade(a: &Pvi, b: &Pvi) -> f64 {
    (b.z - a.z) / (b.s - a.s).max(1e-9)
}

/// The design profile against the limits (stations in m from the start).
pub fn check_profile(pvis: &[Pvi], g: &Guideline, l: &Limits) -> Vec<Finding> {
    let mut out = Vec::new();
    for w in pvis.windows(2) {
        let gr = grade(&w[0], &w[1]) * 100.0;
        let (s0, s1) = (w[0].s, w[1].s);
        if let Some(max) = &l.helling_max {
            if gr.abs() > max.v + 1e-9 {
                out.push(Finding { s0, s1, check: "helling_max", value: gr.abs(), limit: max.v, source: g.cite(max), severity: "fout", text: tx(format!("helling {} % > {} %", comma(gr.abs(), 2), comma(max.v, 1)), format!("grade {} % > {} %", comma(gr.abs(), 2), comma(max.v, 1))) });
            }
        }
        if let Some(min) = &l.helling_min {
            if gr.abs() < min.v - 1e-9 {
                out.push(Finding { s0, s1, check: "helling_min", value: gr.abs(), limit: min.v, source: g.cite(min), severity: "let op", text: tx(format!("helling {} % < {} % (afwatering)", comma(gr.abs(), 2), comma(min.v, 1)), format!("grade {} % < {} % (drainage)", comma(gr.abs(), 2), comma(min.v, 1))) });
            }
        }
    }
    for j in 1..pvis.len().saturating_sub(1) {
        let p = &pvis[j];
        let (g1, g2) = (grade(&pvis[j - 1], p), grade(p, &pvis[j + 1]));
        let dg = (g2 - g1).abs();
        if dg < 1e-9 {
            continue;
        }
        let crest = g2 < g1;
        // No curve at all is a radius of zero.
        let r = p.l / dg;
        let (min, check, kind) = if crest { (&l.r_top_min, "topboog", crate::lang::tr("topboog", "crest curve")) } else { (&l.r_voet_min, "voetboog", crate::lang::tr("voetboog", "sag curve")) };
        if let Some(min) = min {
            if r < min.v - 1e-6 {
                out.push(Finding {
                    s0: p.s - p.l / 2.0,
                    s1: p.s + p.l / 2.0,
                    check,
                    value: r,
                    limit: min.v,
                    source: g.cite(min),
                    severity: "fout",
                    text: format!("{kind} R = {} m < {} m", comma(r, 0), comma(min.v, 0)),
                });
            }
        }
    }
    out
}

/// A design superelevation (%) against the maximum.
pub fn check_superelevation(e: f64, len_m: f64, g: &Guideline, l: &Limits) -> Vec<Finding> {
    match &l.verkanting_max {
        Some(max) if e > max.v + 1e-9 => vec![Finding {
            s0: 0.0,
            s1: len_m,
            check: "verkanting",
            value: e,
            limit: max.v,
            source: g.cite(max),
            severity: "fout",
            text: tx(format!("verkanting {} % > {} %", comma(e, 1), comma(max.v, 1)), format!("superelevation {} % > {} %", comma(e, 1), comma(max.v, 1))),
        }],
        _ => Vec::new(),
    }
}

/// The values of a choice as JSON (each with its source).
pub fn limits_json(g: &Guideline, c: &Category, l: &Limits) -> serde_json::Value {
    let v = |x: &Option<Value>| x.as_ref().map(|x| serde_json::json!({"waarde": x.v, "bron": g.cite(x), "schatting": x.schatting})).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "richtlijn": g.naam,
        "categorie": c.naam,
        "ontwerpsnelheid": l.v,
        "r_min_m": v(&l.r_min),
        "a_min_m": v(&l.a_min),
        "helling_max_pct": v(&l.helling_max),
        "helling_min_pct": v(&l.helling_min),
        "r_top_min_m": v(&l.r_top_min),
        "r_voet_min_m": v(&l.r_voet_min),
        "verkanting_max_pct": v(&l.verkanting_max),
        "stopzicht_m": v(&l.stopzicht),
        "ooghoogte_m": v(&g.ooghoogte),
        "objecthoogte_m": v(&g.objecthoogte),
        "overgangsboog_onder_r_m": v(&l.r_overgang),
    })
}

/// A short line of the values: "Rmin 450 m · Amin 150 m · …".
pub fn limits_text(l: &Limits) -> String {
    let mut parts = Vec::new();
    let mut add = |name: &str, v: &Option<Value>, unit: &str| {
        if let Some(v) = v {
            parts.push(format!("{name} {}{unit}{}", comma(v.v, if v.v.fract().abs() > 1e-9 { 1 } else { 0 }), if v.schatting { "*" } else { "" }));
        }
    };
    add("Rmin", &l.r_min, " m");
    add("Amin", &l.a_min, " m");
    add(crate::lang::tr("helling max", "grade max"), &l.helling_max, " %");
    add(crate::lang::tr("helling min", "grade min"), &l.helling_min, " %");
    add("Rtop", &l.r_top_min, " m");
    add("Rvoet", &l.r_voet_min, " m");
    add(crate::lang::tr("verkanting max", "superelevation max"), &l.verkanting_max, " %");
    add(crate::lang::tr("stopzicht", "stopping sight"), &l.stopzicht, " m");
    parts.join(" · ")
}

/// `CIV_ONTWERPCRITERIA`: without a choice the tables there are; with one
/// its values, kept for the session and, with an axis, on the axis.
pub fn criteria(choice: Option<Choice>, axis_h: Option<Handle>) -> Result<String, String> {
    let all = guidelines();
    let Some(choice) = choice else {
        let list: Vec<serde_json::Value> = all
            .iter()
            .map(|g| {
                serde_json::json!({
                    "id": g.id,
                    "naam": g.naam,
                    "categorieen": g.categorieen.iter().map(|c| serde_json::json!({"id": c.id, "naam": c.naam, "snelheden": c.snelheden.iter().map(|l| l.v).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                })
            })
            .collect();
        super::api::note("richtlijnen", list);
        let text: Vec<String> = all
            .iter()
            .map(|g| format!("{} ({}): {}", g.id, g.naam, g.categorieen.iter().map(|c| format!("{} {}", c.id, c.snelheden.iter().map(|l| format!("{}", l.v)).collect::<Vec<_>>().join("/"))).collect::<Vec<_>>().join(", ")))
            .collect();
        let text = text.join(" | ");
        return Ok(tx(format!("Ontwerpcriteria — kies RICHTLIJN= CATEGORIE= V=: {text}"), format!("Design criteria — choose RICHTLIJN= CATEGORIE= V=: {text}")));
    };
    let (g, c, l) = choice.resolve(&all)?;
    *store().lock().unwrap() = Some(choice.clone());
    super::api::note("criteria", limits_json(&g, &c, &l));
    let mut on_axis = String::new();
    if let Some(h) = axis_h {
        let ht = super::api::handle_text(h);
        let e = crate::bridge::entities(&[h])?.into_iter().next().flatten().ok_or_else(|| tx(format!("geen element met handle {ht}"), format!("no entity with handle {ht}")))?;
        if Alignment::from_entity(&e).is_none() {
            return Err(tx(format!("{ht} is geen as"), format!("{ht} is not an alignment")));
        }
        // The axis's object data, with the choice added or replaced.
        let mut pairs: Vec<(String, String)> = e
            .common()
            .extended_data
            .get_record(crate::bridge::XDATA_APP)
            .map(|r| {
                r.values
                    .iter()
                    .filter_map(|v| match v {
                        ocs_plugin_api::host::XDataValue::String(s) => s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        pairs.retain(|(k, _)| k != KEY);
        pairs.push((KEY.to_string(), choice.text()));
        let refs: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        crate::bridge::register_object_data(h, record(&refs));
        let axis_name = super::data_value(&e, "as").unwrap_or_else(|| ht.clone());
        on_axis = tx(format!(" Vastgelegd op {axis_name}."), format!(" Stored on {axis_name}."));
    }
    Ok(format!("{} — {} {} km/h: {}.{on_axis}", g.naam, c.naam, l.v, limits_text(&l)))
}

/// Where the findings go in a length profile of the axis, when there is one:
/// the frame's origin, base height, exaggeration and units.
fn profile_frame(doc: &CadDocument, axis_name: &str) -> Option<(f64, f64, f64, f64, f64)> {
    let e = doc
        .model_space_entities()
        .filter(|e| super::data_value(e, "lengteprofiel").is_some() && super::data_value(e, "as").as_deref() == Some(axis_name))
        .last()?;
    let g = |k: &str| super::data_value(e, k).and_then(|v| v.parse::<f64>().ok());
    Some((g("x_nul")?, g("y_basis")?, g("basis")?, g("overhoging")?, g("eenheid")?))
}

/// `CIV_AS_TOETS`: the axis and its design profile against the design
/// criteria; every violation marked in plan (and in the length profile) on
/// AL-AANKLEDING, and listed with its stations.
pub fn check(opt: &super::CivielOptions, axis_h: Option<Handle>, choice: Option<Choice>, superelevation: Option<f64>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["as", "ontwerpprofiel", "lengteprofiel"], &axis_h.into_iter().collect::<Vec<_>>())?;
    let (al, h, _) = as_profiel::axis(&doc, axis_h)?;
    let axis_e = doc.get_entity(h);
    let choice = choice_for(choice, axis_e).ok_or_else(|| crate::lang::tr("kies eerst de ontwerpcriteria: CIV_ONTWERPCRITERIA RICHTLIJN= CATEGORIE= V= (of geef ze hier mee)", "choose the design criteria first: CIV_ONTWERPCRITERIA RICHTLIJN= CATEGORIE= V= (or give them here)").to_string())?;
    let (g, c, l) = choice.resolve(&guidelines())?;
    let f = ctx.f;
    let len_m = al.length() / f;
    let mut found = check_plan(&al, f, &g, &l);
    let pvis = super::corridor::profile_of(&doc, &al.name);
    if let Some(p) = &pvis {
        found.extend(check_profile(p, &g, &l));
    }
    if let Some(e) = superelevation {
        found.extend(check_superelevation(e, len_m, &g, &l));
    }
    found.sort_by(|a, b| a.s0.total_cmp(&b.s0));
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    // Markers: the stretch along the axis, a circle at its middle, the text beside it.
    let k = ctx.k;
    let spec = ctx.layer("AL", "AANKLEDING", Element::G)?;
    let spec_t = ctx.layer("AL", "AANKLEDING", Element::T)?;
    let mut out = Out::default();
    let frame = profile_frame(&doc, &al.name);
    for (i, fd) in found.iter().enumerate() {
        let data = record(&[("toets", name.clone()), ("controle", fd.check.to_string()), ("station", format!("{:.3}", al.start + fd.s0)), ("bron", fd.source.clone())]);
        let (a, b) = (fd.s0.clamp(0.0, len_m) * f, fd.s1.clamp(0.0, len_m) * f);
        let n = (((b - a) / (2.0 * f)).ceil() as usize).clamp(1, 500);
        let pts: Vec<XY> = (0..=n).map(|j| al.at(a + (b - a) * j as f64 / n as f64).0).collect();
        if b - a > 1e-6 {
            out.put(&spec, crate::emit::polylines(&[(pts, false)], &spec.name, k, Some(&data)));
        }
        let (p, dir) = al.at((a + b) / 2.0);
        // Stagger the markers so neighbours do not overlap.
        let off = (8.0 + 5.0 * (i % 3) as f64) * k;
        let at = [p[0] - dir.sin() * off, p[1] + dir.cos() * off];
        out.put(&spec, crate::emit::markers(&[p], &spec.name, 2.0 * k, Some(&data)));
        out.put(&spec, crate::emit::polylines(&[(vec![p, at], false)], &spec.name, k, None));
        let label = format!("{} {}–{}: {} ({})", if fd.severity == "fout" { "✗" } else { "!" }, station(al.start + fd.s0), station(al.start + fd.s1), fd.text, fd.source);
        let t = ctx.text(at, &label, 1.8, crate::emit::readable(dir), &spec_t, None);
        out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        // In the length profile: a mark over the stretch at the top of the line.
        if let (Some((x0, y0, base, v, fu)), Some(p)) = (frame, &pvis) {
            if matches!(fd.check, "helling_max" | "helling_min" | "topboog" | "voetboog") {
                let to = |s: f64| [x0 + s * fu, y0 + (as_profiel::design_z(p, s) - base) * v * fu + 3.0 * k];
                let m = ((fd.s1 - fd.s0) / 2.0).ceil().clamp(1.0, 500.0) as usize;
                let line: Vec<XY> = (0..=m).map(|j| to(fd.s0 + (fd.s1 - fd.s0) * j as f64 / m as f64)).collect();
                let mid = to((fd.s0 + fd.s1) / 2.0);
                out.put(&spec, crate::emit::polylines(&[(line, false)], &spec.name, k, Some(&data)));
                let t = ctx.text([mid[0], mid[1] + 3.0 * k], &format!("✗ {}", fd.text), 1.8, 0.0, &spec_t, None);
                out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
            }
        }
    }
    let written = if out.is_empty() { 0 } else { out.write(&format!("Toets {name}"))? };
    let errors = found.iter().filter(|f| f.severity == "fout").count();
    super::api::note("criteria", limits_json(&g, &c, &l));
    super::api::note("keuze", choice.text());
    super::api::note("ontwerpprofiel", pvis.is_some());
    super::api::note(
        "overtredingen",
        found
            .iter()
            .map(|fd| {
                serde_json::json!({
                    "station": al.start + fd.s0,
                    "eindstation": al.start + fd.s1,
                    "toets": fd.check,
                    "waarde": fd.value,
                    "grens": fd.limit,
                    "ernst": fd.severity,
                    "tekst": fd.text,
                    "bron": fd.source,
                })
            })
            .collect::<Vec<_>>(),
    );
    super::api::note("fouten", errors);
    let profile = if pvis.is_some() { "" } else { crate::lang::tr(" (geen ontwerpprofiel: alleen het horizontale alignement)", " (no design profile: the horizontal alignment only)") };
    let (gid, v) = (g.id.to_uppercase(), l.v);
    if found.is_empty() {
        return Ok(tx(format!("{name} voldoet aan {gid} {} {v} km/h{profile}.", c.naam), format!("{name} meets {gid} {} {v} km/h{profile}.", c.naam)));
    }
    let list: Vec<String> = found.iter().take(8).map(|fd| format!("{} {}", station(al.start + fd.s0), fd.text)).collect();
    let (cat, advice, more) = (&c.naam, found.len() - errors, if found.len() > 8 { " …" } else { "" });
    let list = list.join("; ");
    Ok(tx(
        format!("{name} tegen {gid} {cat} {v} km/h: {errors} overtreding(en), {advice} aandachtspunt(en){profile} — {list}{more} — {written} elementen."),
        format!("{name} against {gid} {cat} {v} km/h: {errors} violation(s), {advice} point(s) of attention{profile} — {list}{more} — {written} entities."),
    ))
}

/// Whether the interface is Dutch.
fn nl() -> bool {
    crate::lang::current() == crate::lang::Lang::Nl
}

/// The Dutch or the English text.
fn tx(nl_text: String, en_text: String) -> String {
    if nl() { nl_text } else { en_text }
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_load_and_cite_their_sources() {
        let all = guidelines();
        assert!(all.len() >= 4, "{}", all.len());
        for g in &all {
            for c in &g.categorieen {
                assert!(!c.snelheden.is_empty(), "{} {}", g.id, c.id);
                for l in &c.snelheden {
                    for v in [&l.r_min, &l.a_min, &l.r_overgang, &l.helling_max, &l.helling_min, &l.r_top_min, &l.r_voet_min, &l.verkanting_max, &l.stopzicht].into_iter().flatten() {
                        assert!(g.bronnen.contains_key(&v.bron), "{} {} {}: source '{}' is not listed", g.id, c.id, l.v, v.bron);
                    }
                }
            }
            for v in [&g.ooghoogte, &g.objecthoogte, &g.a_r_deler].into_iter().flatten() {
                assert!(g.bronnen.contains_key(&v.bron), "{}: source '{}'", g.id, v.bron);
            }
        }
        assert_eq!(Choice::parse("ROA; autosnelweg ;120").unwrap(), Choice { guideline: "roa".into(), category: "autosnelweg".into(), v: 120.0 });
        assert!(Choice { guideline: "xyz".into(), category: "a".into(), v: 1.0 }.resolve(&all).is_err());
    }

    fn table() -> (Guideline, Category, Limits) {
        let val = |v: f64| Some(Value { v, bron: "t".into(), schatting: false, opmerking: String::new() });
        let l = Limits { v: 80.0, r_min: val(250.0), a_min: val(100.0), r_overgang: val(1000.0), helling_max: val(5.0), helling_min: val(0.5), r_top_min: val(2500.0), r_voet_min: val(1500.0), verkanting_max: val(6.0), stopzicht: val(110.0) };
        let c = Category { id: "gow".into(), naam: "GOW".into(), snelheden: vec![l.clone()] };
        let g = Guideline { id: "test".into(), naam: "Test".into(), land: "NL".into(), ooghoogte: val(1.1), objecthoogte: val(0.2), a_r_deler: val(3.0), categorieen: vec![c.clone()], bronnen: [("t".to_string(), "Testtabel".to_string())].into_iter().collect() };
        (g, c, l)
    }

    #[test]
    fn violations_with_their_stations() {
        let (g, _, l) = table();
        // R 200 with A 90 (L = 40.5) at the first PI, R 400 without clothoids at the second.
        let pis = [[0.0, 0.0], [300.0, 0.0], [500.0, 200.0], [800.0, 200.0]];
        let al = as_profiel::Alignment::from_layout(&pis, &[200.0, 400.0], &[as_profiel::Transition::Parameter(90.0), as_profiel::Transition::None]).unwrap();
        let found = check_plan(&al, 1.0, &g, &l);
        let checks: Vec<&str> = found.iter().map(|f| f.check).collect();
        assert_eq!(checks, ["clothoide", "boogstraal", "clothoide", "overgangsboog"], "{found:#?}");
        let r = &found[1];
        assert!((r.value - 200.0).abs() < 1e-9 && r.limit == 250.0 && r.source == "Testtabel");
        let starts = al.seg_starts();
        assert!((r.s0 - starts[2]).abs() < 1e-9 && (r.s1 - starts[3]).abs() < 1e-9);
        // A ≥ R/3: A 105 against R 330 (110).
        let al = as_profiel::Alignment::from_layout(&pis[..3], &[330.0], &[as_profiel::Transition::Parameter(105.0)]).unwrap();
        let found = check_plan(&al, 1.0, &g, &l);
        assert_eq!(found.iter().map(|f| f.check).collect::<Vec<_>>(), ["clothoide_r", "clothoide_r"]);
        // Profile: 6 % up (too steep), a sharp crest, then 0,2 % (flat).
        let pvis = as_profiel::fit_curves(&[(0.0, 0.0), (100.0, 6.0), (300.0, 6.4)], 80.0);
        let found = check_profile(&pvis, &g, &l);
        let checks: Vec<&str> = found.iter().map(|f| f.check).collect();
        assert_eq!(checks, ["helling_max", "helling_min", "topboog"]);
        assert!((found[2].value - 80.0 / 0.058).abs() < 1e-6 && found[2].s0 == 60.0 && found[1].severity == "let op");
        assert_eq!(check_superelevation(7.0, 100.0, &g, &l).len(), 1);
        assert!(check_superelevation(5.0, 100.0, &g, &l).is_empty());
    }
}
