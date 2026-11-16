//! Assemblies (profieltypen) as JSON: a small cross-section composer.
//! An assembly lists parameterised parts per side, outward from the axis,
//! with simple conditions (only in cut, only in fill) and targets (a width to
//! an offset or a line, a level to the ground, a height or a 3D line).
//!
//! Presets live in `assets/assemblies/*.json` (built into the plugin); the
//! user's own in `<config>/OpenCADStudio/open-infra/assemblies/*.json`, where
//! a file with a preset's name replaces that preset. Named pavement
//! build-ups come from `assets/assemblies/opbouw.json`, and from an
//! `opbouw.json` beside the user's assemblies.
//!
//! The format (all lengths in metres, slopes in %, negative running down
//! away from the axis):
//!
//! ```json
//! { "naam": "stadsstraat", "omschrijving": "…", "bronnen": ["…"],
//!   "talud": 3, "ingraving": 2, "reikwijdte": 40, "verkanting": 0, "uitrol": 30,
//!   "beide": [ {"type": "rijstrook", "breedte": 3.0, "helling": -2.5, "lagen": "asfalt-stad"} ],
//!   "rechts": [ … ], "links": [ … ] }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use super::corridor::{material_nlcs, nlcs_id, num, trf, Assembly, Condition, Ditch, Layer, NlcsId, Part, PartKind, Target};
use crate::lang::tr;
use super::{Ctx, Out};
use crate::gis::crs::XY;
use crate::nlcs::{Element, Status};

/// The preset files, built in.
const PRESETS: &[&str] = &[
    include_str!("../../assets/assemblies/weg.json"),
    include_str!("../../assets/assemblies/fietspad.json"),
    include_str!("../../assets/assemblies/snelweg.json"),
    include_str!("../../assets/assemblies/dijk.json"),
    include_str!("../../assets/assemblies/stadsstraat.json"),
    include_str!("../../assets/assemblies/woonstraat.json"),
    include_str!("../../assets/assemblies/polderweg.json"),
    include_str!("../../assets/assemblies/2x2-middenberm.json"),
];

const BUILD_UPS: &str = include_str!("../../assets/assemblies/opbouw.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AsmFile {
    naam: String,
    #[serde(default)]
    omschrijving: String,
    #[serde(default)]
    bronnen: Vec<String>,
    talud: Option<f64>,
    ingraving: Option<f64>,
    reikwijdte: Option<f64>,
    verkanting: Option<f64>,
    uitrol: Option<f64>,
    beide: Option<Vec<PartFile>>,
    links: Option<Vec<PartFile>>,
    rechts: Option<Vec<PartFile>>,
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct PartFile {
    #[serde(rename = "type")]
    kind: String,
    naam: Option<String>,
    breedte: Option<f64>,
    helling: Option<f64>,
    opstand: Option<f64>,
    hoogte: Option<f64>,
    schuinte: Option<f64>,
    lagen: Option<LagenFile>,
    materiaal: Option<String>,
    nlcs: Option<String>,
    bodembreedte: Option<f64>,
    diepte: Option<f64>,
    bodempeil: Option<f64>,
    talud_binnen: Option<f64>,
    talud_buiten: Option<f64>,
    als: Option<AlsFile>,
    breedte_doel: Option<Value>,
    hoogte_doel: Option<Value>,
    helling_min: Option<f64>,
    helling_max: Option<f64>,
    /// Where the values come from (free text, kept for the reader).
    #[allow(dead_code)]
    bron: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum LagenFile {
    Named(String),
    List(Vec<LaagFile>),
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct LaagFile {
    materiaal: String,
    dikte: f64,
    rol: Option<String>,
    nlcs: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum AlsFile {
    Short(String),
    Full {
        #[serde(rename = "type")]
        kind: String,
        min: Option<f64>,
        max: Option<f64>,
    },
}

#[derive(Deserialize)]
struct BuildUpsFile {
    opbouw: BTreeMap<String, BuildUpFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildUpFile {
    #[serde(default)]
    omschrijving: String,
    #[allow(dead_code)]
    bron: Option<String>,
    lagen: Vec<LaagFile>,
}

/// Where the user's assemblies are.
pub fn user_dir() -> PathBuf {
    crate::gis::servers::config_dir().join("assemblies")
}

/// "VH-KANTOPSLUITING_OPSLUITBAND" or "VH KANTOPSLUITING_OPSLUITBAND".
fn parse_nlcs(s: &str) -> Result<NlcsId, String> {
    let (g, n) = s.trim().split_once(['-', ' ']).ok_or_else(|| trf!("nlcs '{}': schrijf HOOFDGROEP-OBJECT, bijv. VH-KANTOPSLUITING_OPSLUITBAND", "nlcs '{}': write GROUP-OBJECT, e.g. VH-KANTOPSLUITING_OPSLUITBAND", s))?;
    match nlcs_id(&g.to_ascii_uppercase(), &n.trim().to_ascii_uppercase()) {
        0 => Err(trf!("nlcs '{}': dat NLCS-object bestaat niet", "nlcs '{}': no such NLCS object", s)),
        id => Ok(id),
    }
}

fn layers(l: &[LaagFile]) -> Result<Vec<Layer>, String> {
    l.iter()
        .map(|l| {
            if !(l.dikte > 0.0 && l.dikte < 5.0) {
                return Err(trf!("laag '{}': dikte {} m (verwacht 0 – 5 m)", "layer '{}': dikte {} m (expected 0 – 5 m)", l.materiaal, l.dikte));
            }
            let mut layer = Layer::new(&l.materiaal, l.dikte);
            match l.rol.as_deref().map(str::to_lowercase).as_deref() {
                None => {}
                Some("verharding") => layer.bound = true,
                Some("fundering") => layer.bound = false,
                Some(r) => return Err(trf!("laag '{}': rol '{}' (verharding of fundering)", "layer '{}': rol '{}' (verharding or fundering)", l.materiaal, r)),
            }
            if let Some(n) = &l.nlcs {
                layer.nlcs = parse_nlcs(n)?;
            }
            Ok(layer)
        })
        .collect()
}

/// The named build-ups: built in, then the user's.
pub fn build_ups() -> BTreeMap<String, (String, Vec<Layer>)> {
    let mut out = BTreeMap::new();
    let mut add = |text: &str| -> Result<(), String> {
        let f: BuildUpsFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        for (k, v) in f.opbouw {
            out.insert(k.to_lowercase(), (v.omschrijving, layers(&v.lagen)?));
        }
        Ok(())
    };
    let _ = add(BUILD_UPS);
    if let Ok(text) = std::fs::read_to_string(user_dir().join("opbouw.json")) {
        let _ = add(&text);
    }
    out
}

fn condition(a: &AlsFile) -> Result<Condition, String> {
    let (kind, min, max) = match a {
        AlsFile::Short(k) => (k.as_str(), None, None),
        AlsFile::Full { kind, min, max } => (kind.as_str(), *min, *max),
    };
    let cut = match kind.to_lowercase().as_str() {
        "ingraving" | "afgraving" | "cut" => true,
        "ophoging" | "fill" => false,
        k => return Err(trf!("als '{}': ingraving of ophoging", "als '{}': ingraving (cut) or ophoging (fill)", k)),
    };
    Ok(Condition { cut, min: min.unwrap_or(0.0), max: max.unwrap_or(f64::INFINITY) })
}

fn target(v: &Value, level: bool) -> Result<Target, String> {
    match v {
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(0.0);
            Ok(if level { Target::Level(x) } else { Target::Offset(x) })
        }
        Value::String(s) if level && s.eq_ignore_ascii_case("maaiveld") => Ok(Target::Ground),
        Value::String(s) if !s.trim().is_empty() => Ok(Target::Line(s.trim().to_lowercase())),
        _ => Err(trf!(
            "{}: een getal of de naam van een doellijn{}",
            "{}: a number or the name of a target line{}",
            if level { "hoogte_doel" } else { "breedte_doel" },
            if level { tr(", of \"maaiveld\"", ", or \"maaiveld\" (the ground)") } else { "" }
        )),
    }
}

/// Widths and slopes when a file leaves them out (see docs/civiel-corridor.md
/// for their sources; estimates are marked there).
fn defaults(kind: PartKind) -> (Option<f64>, f64) {
    match kind {
        PartKind::Lane => (None, -2.5),
        PartKind::Crest => (Some(2.5), -2.0),
        PartKind::Shoulder => (Some(1.5), -4.0),
        PartKind::Kerb => (Some(0.15), 0.0),
        PartKind::Gutter => (Some(0.30), -2.5),
        PartKind::Footway => (Some(1.80), 2.0),
        PartKind::CyclePath => (Some(2.00), -2.0),
        PartKind::Parking => (Some(2.00), -2.5),
        PartKind::Median => (Some(4.00), 4.0),
        PartKind::Verge => (Some(1.50), -2.0),
        PartKind::Ditch => (Some(0.0), 0.0),
    }
}

fn part(p: &PartFile, ups: &BTreeMap<String, (String, Vec<Layer>)>, both: bool) -> Result<Part, String> {
    let kind = PartKind::parse(&p.kind).ok_or_else(|| {
        trf!("type '{}' bestaat niet; kies {}", "type '{}' does not exist; choose {}", p.kind, PartKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", "))
    })?;
    let (w0, s0) = defaults(kind);
    let width = p.breedte.or(w0).ok_or_else(|| trf!("{}: geef de breedte", "{}: give the width (breedte)", kind.name()))?;
    if !(0.0..=100.0).contains(&width) {
        return Err(trf!("{}: breedte {} m", "{}: breedte {} m (width)", kind.name(), width));
    }
    let mut out = Part::new(kind, width, p.helling.unwrap_or(s0) / 100.0);
    if kind == PartKind::Median && both {
        // In "beide" the central reserve's width is the whole: half a side.
        out.width = width / 2.0;
    }
    if let Some(n) = &p.naam {
        out.name = n.clone();
    }
    if kind == PartKind::Kerb {
        out.upstand = p.opstand.unwrap_or(0.12);
        out.height = p.hoogte.unwrap_or(0.25);
        out.batter = p.schuinte.unwrap_or(0.02);
        if out.height < out.upstand || out.height <= 0.0 {
            return Err(trf!("{}: hoogte {} m is lager dan de opstand {} m", "{}: hoogte {} m (height) is below the opstand {} m (upstand)", out.name, out.height, out.upstand));
        }
        let n = out.name.to_lowercase();
        out.surface = if n.contains("opsluit") {
            nlcs_id("VH", "KANTOPSLUITING_OPSLUITBAND")
        } else if n.contains("rotonde") {
            nlcs_id("VH", "KANTOPSLUITING_ROTONDEBLOK")
        } else if n.contains("inrit") {
            nlcs_id("VH", "KANTOPSLUITING_INRITBAND")
        } else if n.contains("rijwielpad") || n.contains("fietspad") {
            nlcs_id("VH", "KANTOPSLUITING_RIJWIELPADBAND")
        } else if n.contains("gazon") {
            nlcs_id("VH", "KANTOPSLUITING_GAZONBAND")
        } else {
            nlcs_id("VH", "KANTOPSLUITING_TROTTOIRBAND")
        };
    } else if p.opstand.is_some() || p.hoogte.is_some() || p.schuinte.is_some() {
        return Err(trf!("{}: opstand, hoogte en schuinte horen bij een band", "{}: opstand, hoogte and schuinte belong to a kerb (band)", kind.name()));
    }
    if kind == PartKind::Ditch {
        let d = Ditch {
            bottom: p.bodembreedte.unwrap_or(0.5),
            depth: p.diepte.unwrap_or(0.8),
            level: p.bodempeil,
            inner: p.talud_binnen.unwrap_or(1.5),
            outer: p.talud_buiten.unwrap_or(1.5),
        };
        if d.bottom < 0.0 || d.depth <= 0.0 || d.inner <= 0.0 || d.outer <= 0.0 {
            return Err(tr("sloot: bodembreedte ≥ 0, diepte en taluds > 0", "sloot (ditch): bodembreedte ≥ 0, diepte and taluds > 0").into());
        }
        out.ditch = Some(d);
    } else if p.bodembreedte.is_some() || p.diepte.is_some() || p.bodempeil.is_some() || p.talud_binnen.is_some() || p.talud_buiten.is_some() {
        return Err(trf!("{}: bodembreedte, diepte, bodempeil en taluds horen bij een sloot", "{}: bodembreedte, diepte, bodempeil and taluds belong to a ditch (sloot)", kind.name()));
    }
    out.layers = match &p.lagen {
        None => Vec::new(),
        Some(LagenFile::List(l)) => layers(l)?,
        Some(LagenFile::Named(n)) => ups.get(&n.to_lowercase()).map(|u| u.1.clone()).ok_or_else(|| trf!("{}: opbouw '{}' bestaat niet; kies {}", "{}: build-up (opbouw) '{}' does not exist; choose {}", kind.name(), n, ups.keys().cloned().collect::<Vec<_>>().join(", ")))?,
    };
    if kind != PartKind::Kerb {
        if let Some(m) = &p.materiaal {
            out.surface = material_nlcs(m);
        } else if let Some(l) = out.layers.first() {
            if !matches!(kind, PartKind::Shoulder | PartKind::Verge | PartKind::Median | PartKind::Crest) {
                // The surface by its top layer; asphalt courses as asphalt.
                let course = ["GESLOTENVERHARDING_ASFALT_DEKLAAG", "GESLOTENVERHARDING_ASFALT_TUSSENLAAG", "GESLOTENVERHARDING_ASFALT_ONDERLAAG"];
                out.surface = if course.iter().any(|c| nlcs_id("VH", c) == l.nlcs) { nlcs_id("VH", "GESLOTENVERHARDING_ASFALT") } else { l.nlcs };
            }
        }
    }
    if let Some(n) = &p.nlcs {
        out.surface = parse_nlcs(n)?;
    }
    out.condition = p.als.as_ref().map(condition).transpose()?;
    out.width_target = p.breedte_doel.as_ref().map(|v| target(v, false)).transpose()?;
    out.level_target = p.hoogte_doel.as_ref().map(|v| target(v, true)).transpose()?;
    if matches!(kind, PartKind::Kerb | PartKind::Ditch) && (out.width_target.is_some() || out.level_target.is_some()) {
        return Err(trf!("{}: een band of sloot heeft geen breedte- of hoogtedoel", "{}: a kerb or ditch takes no width or level target", kind.name()));
    }
    out.slope_min = p.helling_min.unwrap_or(-50.0) / 100.0;
    out.slope_max = p.helling_max.unwrap_or(50.0) / 100.0;
    if out.slope_min > out.slope_max {
        return Err(trf!("{}: helling_min is groter dan helling_max", "{}: helling_min is larger than helling_max", kind.name()));
    }
    Ok(out)
}

/// An assembly from JSON text.
pub fn parse(text: &str) -> Result<Assembly, String> {
    let f: AsmFile = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
    let ups = build_ups();
    let side = |list: Option<&Vec<PartFile>>, both: bool, which: &str| -> Result<Vec<Part>, String> {
        let list = list.ok_or_else(|| trf!("geen onderdelen {}: geef \"beide\", of \"links\" en \"rechts\"", "no parts {}: give \"beide\" (both), or \"links\" and \"rechts\"", which))?;
        list.iter().enumerate().map(|(i, p)| part(p, &ups, both).map_err(|e| trf!("{}, onderdeel {}: {}", "{}, part {}: {}", which, i + 1, e))).collect()
    };
    let left = match &f.links {
        Some(l) => side(Some(l), false, "links")?,
        None => side(f.beide.as_ref(), true, "beide")?,
    };
    let right = match &f.rechts {
        Some(r) => side(Some(r), false, "rechts")?,
        None => side(f.beide.as_ref(), true, "beide")?,
    };
    if left.is_empty() && right.is_empty() {
        return Err(tr("een profieltype heeft minstens één onderdeel", "an assembly needs at least one part").into());
    }
    let name = f.naam.trim().to_lowercase();
    if name.is_empty() || name.contains(['/', '\\', ' ']) {
        return Err(trf!("naam '{}': één woord, zonder / of spaties", "naam '{}': one word, without / or spaces", f.naam));
    }
    let mut a = Assembly::symmetric(&name, Vec::new());
    a.sides = [left, right];
    a.description = f.omschrijving;
    a.sources = f.bronnen;
    a.fill = f.talud.unwrap_or(3.0).max(0.1);
    a.cut = f.ingraving.unwrap_or(2.0).max(0.1);
    a.reach = f.reikwijdte.unwrap_or(60.0).max(1.0);
    a.superelevation = f.verkanting.unwrap_or(0.0) / 100.0;
    a.runoff = f.uitrol.unwrap_or(30.0).max(1.0);
    Ok(a)
}

/// One entry of the library.
pub struct Entry {
    pub name: String,
    /// None for a preset, else the user's file.
    pub file: Option<PathBuf>,
    pub assembly: Result<Assembly, String>,
    pub text: String,
}

fn user_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(user_dir())
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) && p.file_stem().is_some_and(|s| s != "opbouw")).collect())
        .unwrap_or_default();
    files.sort();
    files
}

fn preset_name(text: &str) -> String {
    serde_json::from_str::<Value>(text).ok().and_then(|v| v.get("naam").and_then(|n| n.as_str()).map(str::to_lowercase)).unwrap_or_default()
}

/// Every assembly: the user's (which replace presets of the same name), then
/// the presets.
pub fn list() -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for path in user_files() {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let assembly = parse(&text).map_err(|e| format!("{}: {e}", path.display()));
        let name = match &assembly {
            Ok(a) => a.name.clone(),
            Err(_) => path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default(),
        };
        out.push(Entry { name, file: Some(path), assembly, text });
    }
    for text in PRESETS {
        let name = preset_name(text);
        if !out.iter().any(|e| e.name == name) {
            out.push(Entry { name, file: None, assembly: parse(text), text: text.to_string() });
        }
    }
    out
}

/// An assembly by name (the user's first), or from a .json file. In the
/// SuperCloud (`OPENAEC_BEHEERD=1`) only by name: a path, also one stored
/// in a drawing, would reach files outside the work folder.
pub fn find(name: &str) -> Result<Assembly, String> {
    find_in(name, std::env::var("OPENAEC_BEHEERD").as_deref() == Ok("1"))
}

fn find_in(name: &str, managed: bool) -> Result<Assembly, String> {
    let n = name.trim();
    if n.to_lowercase().ends_with(".json") && !managed && Path::new(n).exists() {
        let text = std::fs::read_to_string(n).map_err(|e| format!("{n}: {e}"))?;
        return parse(&text).map_err(|e| format!("{n}: {e}"));
    }
    let all = list();
    match all.into_iter().find(|e| e.name == n.to_lowercase()) {
        Some(e) => e.assembly,
        None => Err(trf!("profieltype '{}' bestaat niet; kies {} (of zet een .json in {})", "assembly '{}' does not exist; choose {} (or put a .json in {})", n, names().join(", "), user_dir().display())),
    }
}

pub fn names() -> Vec<String> {
    list().into_iter().map(|e| e.name).collect()
}

/// The parts of one side as short text: "rijstrook 3,00 · goot 0,30 · band".
pub fn summary(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| {
            let w = match p.kind {
                PartKind::Ditch => p.ditch.map(|d| format!(" b {} d {}", num(d.bottom, 2), d.level.map(|l| format!("NAP{}", num(l, 2))).unwrap_or(num(d.depth, 2)))).unwrap_or_default(),
                _ => format!(" {}", num(p.width, 2)),
            };
            let cond = match p.condition {
                Some(c) if c.cut => tr(" (ingraving)", " (in cut)"),
                Some(_) => tr(" (ophoging)", " (in fill)"),
                None => "",
            };
            let name = if p.name == p.kind.name() { p.kind.label().to_string() } else { format!("{} ({})", p.kind.label(), p.name) };
            format!("{name}{w}{cond}")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn describe(a: &Assembly) -> Value {
    let side = |parts: &[Part]| -> Vec<Value> {
        parts
            .iter()
            .map(|p| {
                serde_json::json!({
                    "type": p.kind.name(), "naam": p.name, "breedte": p.width, "helling_pct": p.slope * 100.0,
                    "opstand": (p.kind == PartKind::Kerb).then_some(p.upstand),
                    "lagen": p.layers.iter().map(|l| serde_json::json!({"materiaal": l.material, "dikte": l.thickness, "verharding": l.bound})).collect::<Vec<_>>(),
                    "als": p.condition.map(|c| if c.cut { "ingraving" } else { "ophoging" }),
                    "breedte_doel": p.width_target.as_ref().map(|t| format!("{t:?}")),
                    "hoogte_doel": p.level_target.as_ref().map(|t| format!("{t:?}")),
                })
            })
            .collect()
    };
    let width = |parts: &[Part]| parts.iter().filter(|p| p.condition.is_none()).map(Part::extent).sum::<f64>();
    serde_json::json!({
        "naam": a.name, "omschrijving": a.description, "bronnen": a.sources,
        "breedte_links": width(&a.sides[0]), "breedte_rechts": width(&a.sides[1]),
        "rijbaanrand_links": a.edge(0).0, "rijbaanrand_rechts": a.edge(1).0,
        "talud": a.fill, "ingraving": a.cut, "verkanting_pct": a.superelevation * 100.0,
        "voorwaarden": a.has_conditions(), "lagen": a.has_layers(),
        "links": side(&a.sides[0]), "rechts": side(&a.sides[1]),
    })
}

/// `CIV_PROFIELTYPEN`: the library, for the command line and scripts.
pub fn list_command() -> Result<String, String> {
    let all = list();
    let mut lines = Vec::new();
    let mut data = Vec::new();
    for e in &all {
        let from = match &e.file {
            Some(p) => format!("{} {}", tr("eigen:", "own:"), p.display()),
            None => tr("standaard", "preset").into(),
        };
        match &e.assembly {
            Ok(a) => {
                lines.push(trf!("{} ({}): links {} | rechts {}", "{} ({}): left {} | right {}", a.name, from, summary(&a.sides[0]), summary(&a.sides[1])));
                let mut d = describe(a);
                d["bron"] = Value::from(from);
                data.push(d);
            }
            Err(err) => {
                lines.push(trf!("{} ({}): FOUT {}", "{} ({}): ERROR {}", e.name, from, err));
                data.push(serde_json::json!({"naam": e.name, "bron": from, "fout": err}));
            }
        }
    }
    super::api::note("profieltypen", data);
    super::api::note("map", user_dir().to_string_lossy().into_owned());
    super::api::note("opbouw", build_ups().into_iter().map(|(k, (d, l))| serde_json::json!({"naam": k, "omschrijving": d, "dikte_m": l.iter().map(|l| l.thickness).sum::<f64>()})).collect::<Vec<_>>());
    Ok(trf!("{} profieltypen ({} eigen in {}): {}", "{} assemblies ({} own in {}): {}", all.len(), all.iter().filter(|e| e.file.is_some()).count(), user_dir().display(), lines.join(" ¶ ")))
}

/// `CIV_PROFIELTYPE NAAM=… [KOPIE=…] [PLAATS=x,y]`: one assembly in detail;
/// copied as a start for the user's own; drawn as a template section.
pub fn show(opt: &super::CivielOptions, name: &str, copy: Option<&str>, at: Option<XY>) -> Result<String, String> {
    let entry = list().into_iter().find(|e| e.name == name.trim().to_lowercase()).ok_or_else(|| trf!("profieltype '{}' bestaat niet; kies {}", "assembly '{}' does not exist; choose {}", name, names().join(", ")))?;
    let a = entry.assembly.clone()?;
    super::api::note("profieltype", describe(&a));
    let mut msg = trf!("Profieltype {}: {} — links {} | rechts {}", "Assembly {}: {} — left {} | right {}", a.name, a.description, summary(&a.sides[0]), summary(&a.sides[1]));
    if let Some(new) = copy {
        let new = new.trim().to_lowercase();
        if new.is_empty() || new.contains(['/', '\\', ' ', '.']) {
            return Err(trf!("KOPIE '{}': één woord, zonder / . of spaties", "KOPIE '{}': one word, without / . or spaces", new));
        }
        let mut v: Value = serde_json::from_str(&entry.text).map_err(|e| e.to_string())?;
        v["naam"] = Value::from(new.clone());
        let dir = user_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("{new}.json"));
        if path.exists() {
            return Err(trf!("{} bestaat al", "{} already exists", path.display()));
        }
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default() + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        super::api::note("bestand", path.to_string_lossy().into_owned());
        msg.push_str(&trf!(". Gekopieerd naar {}", ". Copied to {}", path.display()));
    }
    if let Some(at) = at {
        let n = draw(opt, &a, at)?;
        msg.push_str(&trf!(". Getekend: {} elementen", ". Drawn: {} entities", n));
    }
    Ok(msg)
}

/// The assembly drawn as a template section on flat ground at `at` (drawing
/// coordinates of the axis at design level): the top, the layers, the
/// ground, labels per part. With conditions: once in fill, once in cut.
pub fn draw(opt: &super::CivielOptions, a: &Assembly, at: XY) -> Result<usize, String> {
    let mut opt = opt.clone();
    opt.nlcs.status = Status::N;
    let ctx = Ctx::new(&opt)?;
    let f = ctx.f;
    let top_l = ctx.layer("GW", "HOOGTE_AFWERKHOOGTE", Element::G)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let axis_l = ctx.layer("AM", "AS", Element::G)?;
    let text_l = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let mut out = Out::default();
    let (in_fill, in_cut) = (tr("ophoging", "in fill"), tr("ingraving", "in cut"));
    let cases: Vec<(&str, f64)> = if a.has_conditions() { vec![(in_fill, -1.0), (in_cut, 0.6)] } else { vec![(in_fill, -0.5)] };
    let mut y = at[1];
    let data = super::record(&[("profieltype", a.name.clone())]);
    for (label, dz) in cases {
        let mut g = |_: f64| Some(dz);
        let s = super::corridor::section(a, 0.0, 0.0, None, &mut g);
        let pt = |o: f64, z: f64| [at[0] + o * f, y + z * f];
        let pl = |pts: Vec<XY>, closed: bool, layer: &str| crate::emit::polylines(&[(pts, closed)], layer, ctx.k, Some(&data));
        out.put(&top_l, pl(s.pts.iter().map(|p| pt(p.o, p.z)).collect(), false, &top_l.name));
        let (lo, hi) = (s.pts[0].o - 2.0, s.pts[s.pts.len() - 1].o + 2.0);
        out.put(&ground_l, pl(vec![pt(lo, dz), pt(hi, dz)], false, &ground_l.name));
        let zlo = s.bodies.iter().flat_map(|b| b.poly.iter().map(|p| p.1)).fold(dz.min(-0.2), f64::min);
        out.put(&axis_l, pl(vec![pt(0.0, zlo - 0.3), pt(0.0, 1.2)], false, &axis_l.name));
        for b in &s.bodies {
            let spec = super::corridor::nlcs_layer(&ctx, b.nlcs, Element::G)?;
            out.put(&spec, pl(b.poly.iter().map(|p| pt(p.0, p.1)).collect(), true, &spec.name));
        }
        // Labels: each part's name, width and slope, upright above it.
        for (si, parts) in a.sides.iter().enumerate() {
            let dir = if si == 0 { -1.0 } else { 1.0 };
            // This side's points, outward.
            let mut side_pts: Vec<&super::corridor::Pt> = s.pts.iter().filter(|p| p.o * dir > 0.0).collect();
            if si == 0 {
                side_pts.reverse();
            }
            let mut start = 0.0;
            let mut k = 0;
            for p in parts.iter() {
                // Parts a condition left out of this section have no points.
                let (code, span) = match p.kind {
                    PartKind::Kerb => ("BV", 1),
                    PartKind::Ditch => ("SL", 2),
                    _ => (p.kind.code(), 0),
                };
                let Some(found) = side_pts.iter().skip(k).position(|q| q.code == code) else { continue };
                let idx = k + found;
                let end = side_pts.get(idx + span).map(|q| q.o).unwrap_or(side_pts[idx].o);
                k = idx + span + 1;
                let mid = (start + end) / 2.0;
                let width = (end - start).abs();
                start = end;
                let name = if p.name == p.kind.name() { p.kind.label() } else { p.name.as_str() };
                let mut t = match p.kind {
                    PartKind::Kerb => trf!("{} {} · opstand {}", "{} {} · upstand {}", name, num(p.width, 2), num(p.upstand, 2)),
                    PartKind::Ditch => format!("{name} b {}", num(p.ditch.map(|d| d.bottom).unwrap_or(0.0), 2)),
                    _ => format!("{name} {} · {}%", num(width, 2), num(p.slope * 100.0, 1)),
                };
                if let Some(c) = p.condition {
                    t.push_str(if c.cut { tr(" (ingraving)", " (in cut)") } else { tr(" (ophoging)", " (in fill)") });
                }
                let mut e = crate::emit::text(pt(mid, 0.4), &t, 0.08 * f, std::f64::consts::FRAC_PI_2, &text_l.name, Some(&data));
                if let ocs_plugin_api::host::codec::EntityType::Text(tx) = &mut e {
                    tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
                }
                out.put(&text_l, [e]);
            }
        }
        let title = trf!("Profieltype {} — {} (maaiveld {} m)", "Assembly {} — {} (ground {} m)", a.name, label, num(dz, 1));
        out.put(&text_l, [crate::emit::text(pt(0.0, 3.2), &title, 0.25 * f, 0.0, &text_l.name, Some(&data))]);
        y -= (6.0 + (-zlo).max(1.0)) * f;
    }
    out.write(&trf!("Profieltype {}", "Assembly {}", a.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_mode_reads_no_assembly_file() {
        let dir = std::env::temp_dir().join(format!("oi-profieltype-beheerd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("buiten.json");
        std::fs::write(&file, PRESETS[0]).unwrap();
        let path = file.to_string_lossy().to_string();
        assert!(find_in(&path, false).is_ok());
        let err = find_in(&path, true).err().expect("managed mode read a file");
        assert!(err.contains("buiten.json"), "{err}");
        assert!(find_in("weg", true).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_preset_reads() {
        for text in PRESETS {
            let a = parse(text).unwrap_or_else(|e| panic!("{}: {e}", preset_name(text)));
            assert!(!a.sides[0].is_empty() && !a.sides[1].is_empty(), "{}", a.name);
        }
        let ups = build_ups();
        assert!(ups.len() >= 6, "{:?}", ups.keys().collect::<Vec<_>>());
        for (name, (_, l)) in &ups {
            assert!(!l.is_empty() && l.iter().all(|l| l.nlcs != 0), "{name}");
        }
        // The old templates keep their geometry.
        let weg = parse(PRESETS[0]).unwrap();
        assert_eq!(weg.name, "weg");
        assert!((weg.half_width() - 4.75).abs() < 1e-9 && (weg.superelevation - 0.04).abs() < 1e-12);
        let street = PRESETS.iter().map(|t| parse(t).unwrap()).find(|a| a.name == "stadsstraat").unwrap();
        assert!(street.sides[1].iter().any(|p| p.kind == PartKind::Ditch) && street.sides[0].iter().any(|p| p.kind == PartKind::Kerb));
    }

    #[test]
    fn a_file_with_conditions_and_targets() {
        let a = parse(
            r#"{"naam": "Test", "beide": [
                {"type": "middenberm", "breedte": 3.0},
                {"type": "rijstrook", "breedte": 3.1, "lagen": "asfalt-gow"},
                {"type": "band", "naam": "opsluitband 10/20", "opstand": 0.0, "hoogte": 0.20, "schuinte": 0},
                {"type": "trottoir", "breedte_doel": "rooilijn", "hoogte_doel": "maaiveld", "helling_min": -1, "helling_max": 4,
                 "lagen": [{"materiaal": "betontegels", "dikte": 0.05}, {"materiaal": "zand", "dikte": 0.2, "rol": "fundering"}]},
                {"type": "sloot", "als": {"type": "ingraving", "min": 0.2}, "bodembreedte": 0.6, "diepte": 0.9, "talud_buiten": 2},
                {"type": "berm", "als": "ophoging"}]}"#,
        )
        .unwrap();
        assert_eq!(a.name, "test");
        let r = &a.sides[1];
        assert!((r[0].width - 1.5).abs() < 1e-12, "half the central reserve");
        assert_eq!(r[1].layers.len(), 5);
        assert_eq!(r[2].surface, nlcs_id("VH", "KANTOPSLUITING_OPSLUITBAND"));
        assert_eq!(r[3].width_target, Some(Target::Line("rooilijn".into())));
        assert_eq!(r[3].level_target, Some(Target::Ground));
        assert!((r[3].slope_max - 0.04).abs() < 1e-12 && !r[3].layers[1].bound);
        assert_eq!(r[4].condition, Some(Condition { cut: true, min: 0.2, max: f64::INFINITY }));
        assert_eq!(r[4].ditch.unwrap().outer, 2.0);
        assert!(r[5].condition.is_some_and(|c| !c.cut));
        // Mistakes are named in Dutch with where they are.
        let e = parse(r#"{"naam": "x", "beide": [{"type": "rijstrook", "breedte": 3}, {"type": "stoep"}]}"#).unwrap_err();
        assert!(e.contains("onderdeel 2") || e.contains("part 2"), "{e}");
        assert!(e.contains("stoep"), "{e}");
        let e = parse(r#"{"naam": "x", "beide": [{"type": "rijstrook"}]}"#).unwrap_err();
        assert!(e.contains("breedte"), "{e}");
        let e = parse(r#"{"naam": "x", "beide": [{"type": "rijstrook", "breedte": 3, "lagen": "onbekend"}]}"#).unwrap_err();
        assert!(e.contains("'onbekend'"), "{e}");
        let e = parse(r#"{"naam": "x", "beide": [{"type": "band", "opstand": 0.3, "hoogte": 0.2}]}"#).unwrap_err();
        assert!(e.contains("opstand"), "{e}");
        let e = parse(r#"{"naam": "x", "beide": [{"type": "rijstrook", "breedte": 3, "bredte": 2}]}"#).unwrap_err();
        assert!(e.contains("bredte"), "{e}");
    }

    #[test]
    fn asymmetric_sides() {
        let a = parse(r#"{"naam": "x", "links": [{"type": "rijstrook", "breedte": 3}], "rechts": [{"type": "rijstrook", "breedte": 3}, {"type": "parkeerstrook"}]}"#).unwrap();
        assert_eq!((a.sides[0].len(), a.sides[1].len()), (1, 2));
        assert!((a.half_width() - 5.0).abs() < 1e-12);
        assert!((a.edge(1).0 - 5.0).abs() < 1e-12);
    }
}
