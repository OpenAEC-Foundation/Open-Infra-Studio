//! Layer standards: the CAD layer set is a setting.
//!
//! Open Infra names its drawn content by NLCS object (group + name, e.g.
//! `RI DWA_RIOOLLEIDING_PVC_250`): that is the internal semantic key. NLCS is
//! one layer standard; the others are data, one TSV per standard under
//! `assets/laagstandaarden/` (built in) or `<config>/OpenCADStudio/open-infra/
//! laagstandaarden/` (the user's own, read at start). A file maps semantic
//! keys to the standard's layer names and pens; see docs/laagstandaarden.md.
//!
//! The choice is `layer_standard` in settings.json; without it the default
//! region decides (Netherlands → NLCS, United States → NCS, else ISO 13567).
//! `nlcs::layer` asks [`current`] for every layer it makes, and
//! `nlcs::parse_layer` asks [`recognise`] for names that are not NLCS.

use crate::nlcs::{self, Element, LayerSpec, Object, Status};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// The built-in standards, in the order they are listed.
const BUILTIN: &[&str] = &[
    include_str!("../assets/laagstandaarden/nlcs.tsv"),
    include_str!("../assets/laagstandaarden/ncs-us.tsv"),
    include_str!("../assets/laagstandaarden/uk-iso13567.tsv"),
    include_str!("../assets/laagstandaarden/au-iso13567.tsv"),
    include_str!("../assets/laagstandaarden/iso13567.tsv"),
];

/// Linetypes the standards' files may name besides the NLCS ones
/// (the usual ISO 128 and drafting dash patterns, paper millimetres).
pub const LINETYPES: &str = include_str!("../assets/laagstandaarden/linetypes.tsv");

pub const NLCS: &str = "nlcs";

/// One mapping row: semantic key → layer template and pen.
#[derive(Clone, Debug)]
pub struct Rule {
    /// NLCS main group, or `*` for any.
    pub group: String,
    /// Object name, `PREFIX*`, or `*` for any object of the group.
    pub pattern: String,
    /// Layer name with `{status}` / `{element}` placeholders; `{-status}`
    /// puts the hyphen (any one character) only before a non-empty code.
    pub layer: String,
    /// The template for text (T) and hatching (A), when the standard keeps
    /// them on a layer of their own (NCS: `C-STRM-IDEN`); empty: `layer`.
    pub annotation: String,
    /// ACI colour number, and the true colour when the file gives an RGB
    /// value no ACI colour has.
    pub color: Option<(u8, Option<(u8, u8, u8)>)>,
    pub linetype: String,
    pub lineweight_mm: Option<f64>,
}

impl Rule {
    /// How closely the rule matches `obj`: `None` when it does not; higher
    /// is more specific (exact name, then longer prefixes, then the group,
    /// then the catch-all).
    fn specificity(&self, obj: &Object) -> Option<usize> {
        let group = self.group == "*" || self.group == obj.group;
        if !group {
            return None;
        }
        let g = if self.group == "*" { 0 } else { 1 };
        match self.pattern.strip_suffix('*') {
            Some("") => Some(g),
            Some(prefix) => obj.name.starts_with(prefix).then_some(2 + prefix.len()),
            None => (self.pattern == obj.name).then_some(10_000),
        }
    }

    /// The catch-all row (`*` `*`): what an object no other row knows gets.
    pub fn is_fallback(&self) -> bool {
        self.group == "*" && self.pattern == "*"
    }
}

#[derive(Debug)]
pub struct LayerStandard {
    pub id: String,
    pub name: String,
    /// Region code the standard belongs to (`NL`, `US`, `UK`, `AU`, `international`).
    pub region: String,
    pub sources: Vec<String>,
    /// NLCS itself: the layers come from the NLCS tables (`nlcs::nlcs_layer`).
    pub native_nlcs: bool,
    /// Status codes for B, N, T, V.
    status: [String; 4],
    /// Element codes for G, S, T, A.
    element: [String; 4],
    pub rules: Vec<Rule>,
    /// Read from the user's folder.
    pub user: bool,
    reverse: OnceLock<HashMap<String, (u32, Status, Element)>>,
}

const ELEMENTS: [Element; 4] = [Element::G, Element::S, Element::T, Element::A];

fn element_index(e: Element) -> usize {
    ELEMENTS.iter().position(|x| *x == e).unwrap_or(0)
}

fn status_index(s: Status) -> usize {
    Status::ALL.iter().position(|x| *x == s).unwrap_or(0)
}

/// Read a standard from its TSV text.
pub fn parse(text: &str, user: bool) -> Result<LayerStandard, String> {
    let mut s = LayerStandard {
        id: String::new(),
        name: String::new(),
        region: "international".into(),
        sources: Vec::new(),
        native_nlcs: false,
        status: ["B", "N", "T", "V"].map(String::from),
        element: ["G", "S", "T", "A"].map(String::from),
        rules: Vec::new(),
        user,
        reverse: OnceLock::new(),
    };
    let codes = |cells: &[&str], keys: [&str; 4], out: &mut [String; 4]| -> Result<(), String> {
        for c in cells {
            let (k, v) = c.split_once('=').ok_or_else(|| format!("'{c}': expected KEY=code"))?;
            let i = keys.iter().position(|x| *x == k.trim()).ok_or_else(|| format!("'{k}': expected one of {}", keys.join(", ")))?;
            out[i] = v.trim().to_string();
        }
        Ok(())
    };
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        let at = |e: String| format!("line {}: {e}", n + 1);
        if let Some(key) = cells[0].strip_prefix('#') {
            let rest = &cells[1..];
            let value = rest.first().map(|v| v.trim().to_string()).unwrap_or_default();
            match key {
                "id" => s.id = value,
                "name" => s.name = value,
                "region" => s.region = value,
                "source" => s.sources.push(value),
                "kind" => s.native_nlcs = value == "nlcs",
                "status" => codes(rest, ["B", "N", "T", "V"], &mut s.status).map_err(at)?,
                "element" => codes(rest, ["G", "S", "T", "A"], &mut s.element).map_err(at)?,
                // Comments.
                _ => {}
            }
            continue;
        }
        if cells[0] == "group" {
            continue;
        }
        let cell = |i: usize| cells.get(i).map(|v| v.trim()).unwrap_or("");
        // group, name, layer, annotation, color, linetype, lineweight_mm, note
        if cell(1).is_empty() || cell(2).is_empty() {
            return Err(at("a row needs group, name and layer".into()));
        }
        let color = match cell(4) {
            "" => None,
            c => Some(parse_color(c).ok_or_else(|| at(format!("colour '{c}' is not an ACI number 1–255 or R,G,B")))?),
        };
        let lineweight_mm = match cell(6) {
            "" => None,
            w => Some(w.replace(',', ".").parse::<f64>().map_err(|_| at(format!("lineweight '{w}' is not a number of mm")))?),
        };
        s.rules.push(Rule { group: cell(0).into(), pattern: cell(1).into(), layer: cell(2).into(), annotation: cell(3).into(), color, linetype: cell(5).into(), lineweight_mm });
    }
    if s.id.is_empty() || !s.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("#id missing or not made of letters, digits, - and _".into());
    }
    if s.name.is_empty() {
        s.name = s.id.clone();
    }
    if !s.native_nlcs && !s.rules.iter().any(Rule::is_fallback) {
        return Err(format!("{}: the catch-all row (* *) is missing", s.id));
    }
    Ok(s)
}

/// An ACI number, or `R,G,B` as the nearest ACI colour plus the true colour
/// when that differs.
fn parse_color(c: &str) -> Option<(u8, Option<(u8, u8, u8)>)> {
    if let Ok(n) = c.parse::<u8>() {
        return (n >= 1).then_some((n, None));
    }
    let v: Vec<u8> = c.split(',').map(|x| x.trim().parse::<u8>()).collect::<Result<_, _>>().ok()?;
    let [r, g, b] = v[..] else { return None };
    let d = |(x, y, z): (u8, u8, u8)| (x as i32 - r as i32).pow(2) + (y as i32 - g as i32).pow(2) + (z as i32 - b as i32).pow(2);
    let aci = (1..=255u8).min_by_key(|n| d(nlcs::color_rgb(*n)))?;
    Some((aci, (nlcs::color_rgb(aci) != (r, g, b)).then_some((r, g, b))))
}

/// The template a rule uses for an element.
fn template(rule: &Rule, element: Element) -> &str {
    if matches!(element, Element::T | Element::A) && !rule.annotation.is_empty() {
        &rule.annotation
    } else {
        &rule.layer
    }
}

/// Fill in `{status}` / `{element}`, with an optional separator before the key.
fn expand(template: &str, status: &str, element: &str) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &rest[open + 1..open + close];
        let value = [("status", status), ("element", element)].into_iter().find_map(|(k, v)| inner.strip_suffix(k).map(|sep| (sep, v)));
        match value {
            Some((sep, v)) if !v.is_empty() => {
                out.push_str(sep);
                out.push_str(v);
            }
            Some(_) => {}
            None => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out
}

impl LayerStandard {
    pub fn is_nlcs(&self) -> bool {
        self.native_nlcs
    }

    pub fn status_code(&self, s: Status) -> &str {
        &self.status[status_index(s)]
    }

    pub fn element_code(&self, e: Element) -> &str {
        &self.element[element_index(e)]
    }

    /// The rules that match `obj`, most specific first.
    pub fn matches(&self, obj: &Object) -> Vec<&Rule> {
        let mut m: Vec<(usize, usize, &Rule)> = self.rules.iter().enumerate().filter_map(|(i, r)| r.specificity(obj).map(|s| (s, i, r))).collect();
        // Most specific first; between equals the earlier row.
        m.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        m.into_iter().map(|(_, _, r)| r).collect()
    }

    /// The row that decides `obj`'s layer name.
    pub fn rule(&self, obj: &Object) -> Option<&Rule> {
        self.matches(obj).into_iter().next()
    }

    /// This standard's layer for an object; `nlcs` is the NLCS layer of the
    /// same object, status and element (NLCS itself returns it as it is).
    /// A pen cell left empty takes the next less specific row's, and
    /// without any the NLCS pen of the object.
    pub fn spec(&self, obj: &Object, status: Status, element: Element, nlcs: LayerSpec) -> LayerSpec {
        if self.native_nlcs {
            return nlcs;
        }
        let rules = self.matches(obj);
        let Some(first) = rules.first() else { return nlcs };
        let (color, true_color) = rules.iter().find_map(|r| r.color).unwrap_or((nlcs.color, None));
        let linetype = rules.iter().map(|r| r.linetype.as_str()).find(|l| !l.is_empty()).map(str::to_string).unwrap_or(nlcs.linetype);
        let lineweight = rules.iter().find_map(|r| r.lineweight_mm).map(|mm| (mm * 100.0).round() as i16).or(nlcs.lineweight);
        LayerSpec {
            name: expand(template(first, element), self.status_code(status), self.element_code(element)),
            color,
            rgb: nlcs::color_rgb(color),
            true_color: nlcs.true_color.or(true_color),
            lineweight,
            linetype,
            object_id: obj.id,
        }
    }

    /// The layer name only.
    pub fn layer_name(&self, obj: &Object, status: Status, element: Element) -> String {
        self.spec(obj, status, element, nlcs::nlcs_layer(obj, status, "XX", element)).name
    }

    /// Layer name → (object, status, element), for every name this standard
    /// makes. A name many objects share stands for the one with the most
    /// specific row (then the shortest name): its group and kind are right,
    /// the exact object is in the object data.
    fn reverse(&self) -> &HashMap<String, (u32, Status, Element)> {
        self.reverse.get_or_init(|| {
            let mut best: HashMap<String, ((usize, usize, u32), (u32, Status, Element))> = HashMap::new();
            for obj in nlcs::objects() {
                let Some(rule) = self.rule(obj) else { continue };
                let rank = (usize::MAX - rule.specificity(obj).unwrap_or(0), obj.name.len(), obj.id);
                for (si, s) in Status::ALL.into_iter().enumerate() {
                    for (ei, e) in ELEMENTS.into_iter().enumerate() {
                        let name = expand(template(rule, e), &self.status[si], &self.element[ei]);
                        // Between statuses or elements sharing a code: the first.
                        let rank = (rank.0, rank.1 * 16 + si * 4 + ei, rank.2);
                        let entry = best.entry(name.to_ascii_uppercase()).or_insert((rank, (obj.id, s, e)));
                        if rank < entry.0 {
                            *entry = (rank, (obj.id, s, e));
                        }
                    }
                }
            }
            best.into_iter().map(|(k, (_, v))| (k, v)).collect()
        })
    }

    /// The semantic object, status and element of one of this standard's
    /// layer names (case-insensitive).
    pub fn lookup(&self, layer: &str) -> Option<(&'static Object, Status, Element)> {
        if self.native_nlcs {
            let p = nlcs::parse_nlcs_layer(layer).ok()?;
            let obj = nlcs::object_by_name(&p.group, &p.object)?;
            let e = ELEMENTS.into_iter().find(|e| e.code() == p.element).unwrap_or(Element::G);
            return Some((obj, p.status, e));
        }
        let (id, s, e) = *self.reverse().get(&layer.to_ascii_uppercase())?;
        Some((nlcs::object(id)?, s, e))
    }
}

fn load() -> Vec<LayerStandard> {
    let mut out: Vec<LayerStandard> = BUILTIN.iter().map(|t| parse(t, false).expect("built-in layer standard")).collect();
    #[cfg(not(test))]
    for (path, text) in user_files() {
        match parse(&text, true) {
            Ok(s) => match out.iter().position(|o| o.id == s.id) {
                // A user's file with a built-in id replaces it.
                Some(i) => out[i] = s,
                None => out.push(s),
            },
            Err(e) => eprintln!("[Open Infra] {}: {e}", path.display()),
        }
    }
    out
}

/// The folder for the user's own standards.
pub fn user_dir() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("laagstandaarden")
}

#[cfg(not(test))]
fn user_files() -> Vec<(std::path::PathBuf, String)> {
    let Ok(dir) = std::fs::read_dir(user_dir()) else { return Vec::new() };
    let mut files: Vec<_> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tsv"))).collect();
    files.sort();
    files.into_iter().filter_map(|p| std::fs::read_to_string(&p).ok().map(|t| (p, t))).collect()
}

/// Every standard: the built-in ones, then the user's.
pub fn all() -> &'static [LayerStandard] {
    static ALL: OnceLock<Vec<LayerStandard>> = OnceLock::new();
    ALL.get_or_init(load)
}

pub fn by_id(id: &str) -> Option<&'static LayerStandard> {
    all().iter().find(|s| s.id.eq_ignore_ascii_case(id.trim()))
}

fn nlcs_standard() -> &'static LayerStandard {
    by_id(NLCS).expect("NLCS standard")
}

/// The standard a region starts with.
pub fn default_for(region: crate::localisation::Region) -> &'static str {
    use crate::localisation::Region;
    match region {
        Region::Netherlands => NLCS,
        Region::UnitedStates => "ncs-us",
        Region::International => "iso13567",
    }
}

fn settings_file() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("settings.json")
}

/// The standard settings.json names, else the default region's.
pub fn configured() -> &'static LayerStandard {
    let v: serde_json::Value = std::fs::read_to_string(settings_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    from_settings(&v)
}

fn from_settings(v: &serde_json::Value) -> &'static LayerStandard {
    v.get("layer_standard")
        .and_then(|s| s.as_str())
        .and_then(by_id)
        .or_else(|| by_id(default_for(crate::localisation::from_settings(v))))
        .unwrap_or_else(nlcs_standard)
}

static CURRENT: RwLock<Option<&'static LayerStandard>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<&'static LayerStandard>> = const { std::cell::Cell::new(None) };
}

/// The active standard. Tests get NLCS unless they use [`with`].
pub fn current() -> &'static LayerStandard {
    #[cfg(test)]
    {
        OVERRIDE.with(|o| o.get()).unwrap_or_else(nlcs_standard)
    }
    #[cfg(not(test))]
    {
        if let Some(s) = *CURRENT.read().unwrap() {
            return s;
        }
        let s = configured();
        *CURRENT.write().unwrap() = Some(s);
        s
    }
}

/// Run `f` with standard `id` active on this thread (tests).
#[cfg(test)]
pub fn with<R>(id: &str, f: impl FnOnce() -> R) -> R {
    let s = by_id(id).unwrap_or_else(|| panic!("layer standard {id}"));
    let before = OVERRIDE.with(|o| o.replace(Some(s)));
    let r = f();
    OVERRIDE.with(|o| o.set(before));
    r
}

/// Make `id` the active standard and store it in settings.json.
pub fn set(id: &str) -> Result<&'static LayerStandard, String> {
    let s = by_id(id).ok_or_else(|| format!("{} '{id}' ({}: {})", crate::lang::tr("onbekende laagstandaard", "unknown layer standard"), crate::lang::tr("kies uit", "choose from"), ids().join(", ")))?;
    let path = settings_file();
    let mut v: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.to_string()),
    };
    if !v.is_object() {
        return Err("settings.json must be a JSON object".into());
    }
    v["layer_standard"] = serde_json::Value::String(s.id.clone());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    *CURRENT.write().unwrap() = Some(s);
    Ok(s)
}

pub fn ids() -> Vec<&'static str> {
    all().iter().map(|s| s.id.as_str()).collect()
}

/// The semantic object behind a layer name that is not NLCS: the active
/// standard first, then the others (a drawing made under another setting).
pub fn recognise(layer: &str) -> Option<(&'static Object, Status, Element)> {
    let cur = current();
    std::iter::once(cur).chain(all().iter().filter(|s| !std::ptr::eq(*s, cur))).filter(|s| !s.native_nlcs).find_map(|s| s.lookup(layer))
}

/// Why the NLCS check does not run: another standard is active. `None`
/// under NLCS.
pub fn nlcs_check_note() -> Option<String> {
    let s = current();
    (!s.is_nlcs()).then(|| {
        format!(
            "{} {} ({}). {}",
            crate::lang::tr("De NLCS-controle toetst NLCS-lagen; de actieve laagstandaard is", "The NLCS check checks NLCS layers; the active layer standard is"),
            s.name,
            s.id,
            crate::lang::tr("Kies NLCS bij Localisatie of met GIS_LAAGSTANDAARD nlcs om te controleren.", "Choose NLCS under Localisation or with GIS_LAAGSTANDAARD nlcs to check.")
        )
    })
}

/// One line per standard, for the command line and the MCP tool.
pub fn overview() -> String {
    let cur = current();
    all()
        .iter()
        .map(|s| format!("{} {} — {}{}", if std::ptr::eq(s, cur) { "*" } else { " " }, s.id, s.name, if s.user { crate::lang::tr(" (eigen bestand)", " (user file)") } else { "" }))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Example layer names of the active standard, for the overview.
pub fn examples(s: &LayerStandard) -> Vec<(&'static str, String)> {
    EXAMPLES
        .iter()
        .filter_map(|(label, g, n, st, el)| nlcs::object_by_name(g, n).map(|o| (*label, if s.native_nlcs { nlcs::nlcs_layer(o, *st, "XX", *el).name } else { s.layer_name(o, *st, *el) })))
        .collect()
}

/// Typical objects: (label, group, name, status, element).
pub const EXAMPLES: &[(&str, &str, &str, Status, Element)] = &[
    ("foul sewer pipe", "RI", "DWA_RIOOLLEIDING_PVC_250", Status::N, Element::G),
    ("manhole", "RI", "DWA_RIOOLPUT_INSPECTIEPUT", Status::N, Element::S),
    ("road centre line", "AM", "AS_WEG", Status::N, Element::G),
    ("contour", "OG", "HOOGTELIJN", Status::B, Element::G),
    ("low-voltage cable (KLIC)", "KL", "ET_LS", Status::B, Element::G),
    ("survey point", "OG", "HOOGTEPUNT", Status::B, Element::S),
];

/// The `GIS_LAAGSTANDAARD [<id>]` command: show the standards, or switch.
pub fn command(arg: &str) -> Result<String, String> {
    let arg = arg.trim();
    if !arg.is_empty() {
        let s = set(arg)?;
        return Ok(format!("{}: {} — {}", crate::lang::tr("Laagstandaard", "Layer standard"), s.id, s.name));
    }
    let cur = current();
    let ex: Vec<String> = examples(cur).into_iter().map(|(l, n)| format!("{l}: {n}")).collect();
    Ok(format!(
        "{}\n{}: {}\n{}: {}",
        overview(),
        crate::lang::tr("Voorbeelden", "Examples"),
        ex.join("; "),
        crate::lang::tr("Eigen standaarden (TSV)", "Own standards (TSV)"),
        user_dir().display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(g: &str, n: &str) -> &'static Object {
        nlcs::object_by_name(g, n).unwrap_or_else(|| panic!("{g} {n}"))
    }

    #[test]
    fn templates_expand_with_optional_separators() {
        assert_eq!(expand("C-SSWR-PIPE{-element}{-status}", "E", ""), "C-SSWR-PIPE-E");
        assert_eq!(expand("C-SSWR-PIPE{-element}{-status}", "N", "IDEN"), "C-SSWR-PIPE-IDEN-N");
        assert_eq!(expand("C-Ss_50_30_04-{element}{status}", "N", "M"), "C-Ss_50_30_04-MN");
        assert_eq!(expand("A{x}B", "N", "M"), "A{x}B");
    }

    #[test]
    fn builtin_standards_load_and_nlcs_is_unchanged() {
        assert_eq!(ids(), vec!["nlcs", "ncs-us", "uk-iso13567", "au-iso13567", "iso13567"]);
        let s = by_id("NLCS").unwrap();
        assert!(s.is_nlcs());
        let o = obj("RI", "DWA_RIOOLLEIDING_PVC_250");
        assert_eq!(nlcs::layer(o, Status::N, "OI", Element::G).name, "N-OI-RI-DWA_RIOOLLEIDING_PVC_250-G");
        for st in all() {
            assert!(!st.sources.is_empty() || st.user, "{}: sources", st.id);
        }
    }

    #[test]
    fn default_follows_the_region() {
        use serde_json::json;
        assert_eq!(from_settings(&json!({})).id, "nlcs");
        assert_eq!(from_settings(&json!({"region": "US"})).id, "ncs-us");
        assert_eq!(from_settings(&json!({"region": "international"})).id, "iso13567");
        assert_eq!(from_settings(&json!({"region": "NL", "layer_standard": "uk-iso13567"})).id, "uk-iso13567");
        assert_eq!(from_settings(&json!({"region": "US", "layer_standard": "nope"})).id, "ncs-us");
    }

    /// Golden names, checked against the sources in docs/laagstandaarden.md.
    #[test]
    fn golden_layer_names() {
        let golden: &[(&str, [&str; 6])] = &[
            ("nlcs", ["N-XX-RI-DWA_RIOOLLEIDING_PVC_250-G", "N-XX-RI-DWA_RIOOLPUT_INSPECTIEPUT-S", "N-XX-AM-AS_WEG-G", "B-XX-OG-HOOGTELIJN-G", "B-XX-KL-ET_LS-G", "B-XX-OG-HOOGTEPUNT-S"]),
            ("ncs-us", GOLDEN_US),
            ("uk-iso13567", GOLDEN_UK),
            ("au-iso13567", GOLDEN_AU),
            ("iso13567", GOLDEN_ISO),
        ];
        for (id, names) in golden {
            let s = by_id(id).unwrap();
            let got: Vec<String> = examples(s).into_iter().map(|(_, n)| n).collect();
            assert_eq!(got, names.to_vec(), "{id}");
        }
    }

    // A/E/C R6 layers (C-SSWR-MAIN-PIPE, C-SSWR-JBOX, C-ROAD-CNTR,
    // C-TOPO-MINR, E-SECD-UGND, V-TOPO-SPOT) with the NCS status field.
    const GOLDEN_US: [&str; 6] = ["C-SSWR-MAIN-PIPE-N", "C-SSWR-JBOX-N", "C-ROAD-CNTR-N", "C-TOPO-MINR-E", "E-SECD-UGND-E", "V-TOPO-SPOT-E"];
    // BS 1192 / AEC (UK): role, Uniclass 2015 or Zz code, presentation, description.
    const GOLDEN_UK: [&str; 6] = [
        "D-Ss_50_35_08_30-M_FoulSewer",
        "D-Ss_50_35_06-M_FoulManhole",
        "D-Zz_35_20-M_RoadCentreline",
        "G-Zz_40_15-M_ExistingContour",
        "E-Ss_70_30_45_45-M_ExistingLVCable",
        "G-Zz_40_50-M_ExistingLevelIndicator",
    ];
    const GOLDEN_AU: [&str; 6] = GOLDEN_UK;
    // ISO 13567-2 positions 1–20, then the user-defined description.
    const GOLDEN_ISO: [&str; 6] = [
        "D-Ss5035E-N---------FoulSewer",
        "D-Ss5035E-N---------FoulManhole",
        "D-Zz3520E-N---------RoadCentreline",
        "G-Zz4015E-E---------Contour",
        "E-Ss7030E-E---------LVCable",
        "G-Zz4050E-E---------LevelIndicator",
    ];

    /// ISO 13567 names keep the mandatory and optional fields in place.
    #[test]
    fn iso_fields_sit_at_their_positions() {
        let s = by_id("iso13567").unwrap();
        for o in nlcs::objects() {
            let n = s.layer_name(o, Status::V, Element::T);
            assert!(n.len() > 20 && n.is_ascii(), "{n}");
            assert!(matches!(&n[8..10], "T-" | "B-" | "D-" | "U-" | "G-"), "presentation of {n}");
            assert!(matches!(&n[10..11], "R" | "-"), "status of {n}");
            assert_eq!(&n[11..20], "---------", "{n}");
        }
    }

    /// A layer that holds a cable, pipe or sewer object holds nothing of
    /// another main group: KLIC crossings and sewer imports recognise them
    /// by the mapping.
    #[test]
    fn utility_and_sewer_layers_are_not_shared() {
        for s in all().iter().filter(|s| !s.native_nlcs) {
            let mut groups: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
            for o in nlcs::objects() {
                for st in Status::ALL {
                    for el in ELEMENTS {
                        groups.entry(s.layer_name(o, st, el)).or_default().insert(o.group.clone());
                    }
                }
            }
            for (name, g) in &groups {
                if g.contains("KL") || g.contains("RI") || g.contains("AM") {
                    assert_eq!(g.len(), 1, "{}: {name} holds {g:?}", s.id);
                }
            }
        }
    }

    /// Every NLCS object maps to a row of its own group or a more specific
    /// one in every standard: none falls through to the catch-all.
    #[test]
    fn no_object_falls_through_to_the_catch_all() {
        for s in all().iter().filter(|s| !s.native_nlcs) {
            let fallen: Vec<String> = nlcs::objects().filter(|o| s.rule(o).is_none_or(Rule::is_fallback)).map(|o| format!("{} {}", o.group, o.name)).collect();
            assert!(fallen.is_empty(), "{}: {} objects only on the catch-all: {:?}", s.id, fallen.len(), &fallen[..fallen.len().min(20)]);
        }
    }

    /// The objects the code draws (literals and the names helpers compose)
    /// have a row more specific than their group's in every standard.
    #[test]
    fn objects_the_code_draws_have_their_own_row() {
        for s in all().iter().filter(|s| !s.native_nlcs) {
            let mut generic = Vec::new();
            for (g, n) in USED {
                let o = obj(g, n);
                let r = s.rule(o).unwrap();
                if r.pattern == "*" && !GROUP_LEVEL_OK.contains(g) {
                    generic.push(format!("{g} {n}"));
                }
            }
            assert!(generic.is_empty(), "{}: only a group row for {generic:?}", s.id);
        }
    }

    /// Groups whose objects the standards keep on one layer per group (the
    /// standards have no finer class for them).
    const GROUP_LEVEL_OK: &[&str] = &[];

    /// NLCS objects the code names: grep over src/ for `ctx.layer(…)`,
    /// `object_by_name(…)`, `klic_object` and the sewer helpers.
    pub(crate) const USED: &[(&str, &str)] = &[
        ("AL", "AANKLEDING"), ("AL", "BLADVERWIJZING_AS"), ("AL", "LEGENDA"), ("AL", "ORIENTATIE"), ("AL", "PROFIELBALK"),
        ("AL", "TEKENBLAD_DEELKADER"), ("AL", "TEKENBLADGRENS"), ("AL", "TEKENBLAD_KADER"), ("AL", "TEKENBLAD_VERKLARING"),
        ("AL", "TEKENBLAD_VIEWPORT"), ("AL", "VERWIJZING_DOORSNEDE"),
        ("AM", "AS"), ("AM", "AS_SPOOR_TREIN"), ("AM", "KILOMETRERING"), ("AM", "STRING_HULP"), ("AM", "TANGENTPUNT"), ("AM", "AS_WEG"),
        ("GR", "BOOM"), ("GR", "GRAS"), ("GR", "GRAS_BERM"),
        ("GW", "CUNET_BODEM"), ("GW", "DIJK_KRUIN"), ("GW", "HOOGTE_AFWERKHOOGTE"), ("GW", "MAAIVELD"), ("GW", "ONTGRAVINGSVAK-WERK"),
        ("GW", "OPHOGING-WERK"), ("GW", "TALUDARCERING_KORT"), ("GW", "TALUDARCERING_LANG"), ("GW", "TALUD_KRUIN"), ("GW", "TALUD_TEEN"),
        ("GW", "WATERGANG_SLOOT"), ("GW", "WATERGANG_SLOOT_BODEM"), ("GW", "WATERGANG_SLOOT_INSTEEK"), ("GW", "WATERGANG_WATERLIJN"),
        ("KG", "GRENS_PERCEEL"),
        ("KL", "CHEMIE_HULPSTUK"), ("KL", "CHEMIE_LEIDING"), ("KL", "CHEMIE_MANTELBUIS"), ("KL", "DATA_ALGEMEEN"), ("KL", "DATA_HULPSTUK"),
        ("KL", "DATA_MANTELBUIS"), ("KL", "DATA_TELECOM"), ("KL", "ET_HS"), ("KL", "ET_HULPSTUK"), ("KL", "ET_LS"), ("KL", "ET_MANTELBUIS"),
        ("KL", "ET_MS"), ("KL", "ET_ZHS"), ("KL", "GAS_HD"), ("KL", "GAS_HULPSTUK"), ("KL", "GAS_LD"), ("KL", "GAS_MANTELBUIS"),
        ("KL", "OLIE_LEIDING"), ("KL", "WARMTENET_HULPSTUK"), ("KL", "WARMTENET_MANTELBUIS"), ("KL", "WARMTENET_TRANSPORTLEIDING"),
        ("KL", "WATER_BRANDLEIDING_BRANDHYDRANT"), ("KL", "WATER_HULPSTUK"), ("KL", "WATER_MANTELBUIS"), ("KL", "WATER_TRANSPORTLEIDING"),
        ("KW", "KUNSTWERK_BRUG"), ("KW", "ZICHTRUIMTE"),
        ("MO", "ONDERZOEK_SONDERING"), ("MO", "ONDERZOEK_BORING"),
        ("OG", "BEBOUWING"), ("OG", "BEBOUWING_HUISNUMMER"), ("OG", "BEBOUWING_STRAATNAAM"), ("OG", "BREUKLIJN"), ("OG", "DTM"), ("OG", "GRENS"),
        ("OG", "HOOGTELIJN"), ("OG", "HOOGTELIJN_MAJOR"), ("OG", "HOOGTELIJN_MINOR"), ("OG", "HOOGTEPUNT"), ("OG", "TRIANGULATIE"), ("OG", "WATER"),
        ("RI", "DWA_RIOOLLEIDING_PVC_250"), ("RI", "DWA_RIOOLPUT_INSPECTIEPUT"), ("RI", "DWA_STROMINGSRICHTING"), ("RI", "GWA"), ("RI", "GWA_KOLK"),
        ("RI", "GWA_RIOOLLEIDING"), ("RI", "GWA_RIOOLPUT"), ("RI", "OVERIG_LOZEPUT"), ("RI", "PERSLEIDING"),
        ("RI", "DWA_RIOOLLEIDING"), ("RI", "DWA_RIOOLPUT"), ("RI", "HWA_RIOOLLEIDING"), ("RI", "HWA_RIOOLPUT"), ("RI", "HWA_STROMINGSRICHTING"),
        ("RI", "GWA_STROMINGSRICHTING"), ("RI", "ITR_RIOOLLEIDING"), ("RI", "ITR_RIOOLPUT"), ("RI", "HWA_KOLK"),
        ("VH", "FUNDERING"), ("VH", "FUNDERING_GRANULAAT"), ("VH", "GESLOTENVERHARDING_ASFALT"), ("VH", "GESLOTENVERHARDING_ASFALT_DEKLAAG"),
        ("VH", "GESLOTENVERHARDING_ASFALT_ROOD"), ("VH", "GOOT_BSS"), ("VH", "KANTOPSLUITING_GAZONBAND"), ("VH", "KANTOPSLUITING_INRITBAND"),
        ("VH", "KANTOPSLUITING_OPSLUITBAND"), ("VH", "KANTOPSLUITING_RIJWIELPADBAND"), ("VH", "KANTOPSLUITING_ROTONDEBLOK"),
        ("VH", "KANTOPSLUITING_TROTTOIRBAND"), ("VH", "OPENVERHARDING_BETONSTRAATSTEEN"), ("VH", "OPENVERHARDING_TEGEL"), ("VH", "VERKANTING"),
        ("VW", "MARKERING"), ("VW", "MARKERING_DWARS_DRIEHOEK"),
        ("ZZ", "CONSTRUCTIELIJN"), ("ZZ", "GRENS_WERK"), ("ZZ", "HULPLIJN"), ("ZZ", "NAP"), ("ZZ", "GRENS"), ("ZZ", "DETAIL1"),
    ];

    /// The names the code recognises reverse to their own group (KLIC
    /// crossings, terrain, sewers), and the exact object where one row is
    /// one object.
    #[test]
    fn layer_names_reverse_to_their_object() {
        for s in all().iter().filter(|s| !s.native_nlcs) {
            for (g, n) in USED {
                let o = obj(g, n);
                for st in Status::ALL {
                    for el in ELEMENTS {
                        let name = s.layer_name(o, st, el);
                        let (back, bst, bel) = s.lookup(&name).unwrap_or_else(|| panic!("{}: {name}", s.id));
                        assert_eq!(s.layer_name(back, bst, bel), name, "{}: {g} {n}", s.id);
                    }
                }
            }
            let tin = obj("OG", "TRIANGULATIE");
            assert_eq!(s.lookup(&s.layer_name(tin, Status::B, Element::G)).map(|x| (x.0.id, x.1)), Some((tin.id, Status::B)), "{}", s.id);
            for (g, n) in [("KL", "ET_LS"), ("KL", "GAS_LD"), ("KL", "DATA_TELECOM"), ("RI", "GWA_RIOOLLEIDING")] {
                let back = s.lookup(&s.layer_name(obj(g, n), Status::B, Element::G)).unwrap().0;
                assert_eq!(back.group, g, "{}: {g} {n} → {} {}", s.id, back.group, back.name);
            }
        }
    }

    #[test]
    fn the_chokepoint_follows_the_active_standard() {
        let o = obj("RI", "DWA_RIOOLLEIDING_PVC_250");
        let us = with("ncs-us", || nlcs::layer(o, Status::B, "OI", Element::G));
        assert_eq!(us.name, "C-SSWR-MAIN-PIPE-E");
        // A/E/C R6 pen: 0.35 mm, RGB 0,255,0 (ACI 3).
        assert_eq!((us.color, us.true_color, us.lineweight), (3, None, Some(35)));
        let text = with("ncs-us", || nlcs::layer(o, Status::N, "OI", Element::T));
        assert_eq!(text.name, "C-SSWR-IDEN-N");
        assert_eq!(us.object_id, o.id);
        // Back to NLCS outside `with`.
        assert_eq!(nlcs::layer(o, Status::B, "OI", Element::G).name, "B-OI-RI-DWA_RIOOLLEIDING_PVC_250-G");
        // Recognised whatever is active, and parsed back to the semantic key.
        let p = nlcs::parse_layer("C-SSWR-MAIN-PIPE-E").unwrap();
        assert_eq!((p.group.as_str(), p.status), ("RI", Status::B));
        assert!(p.object.starts_with("DWA_"), "{}", p.object);
        assert!(nlcs::parse_nlcs_layer("C-SSWR-MAIN-PIPE-E").is_err());
        assert_eq!(nlcs::as_nlcs("C-SSWR-MAIN-PIPE-E").split('-').nth(2), Some("RI"));
        assert_eq!(nlcs::as_nlcs("B-XX-RI-GWA-G"), "B-XX-RI-GWA-G");
        // Terrain under another standard is still the existing ground.
        let tin = obj("OG", "TRIANGULATIE");
        let name = with("uk-iso13567", || nlcs::layer(tin, Status::B, "XX", Element::G).name);
        assert_eq!(name, "G-Zz_40-M_ExistingGroundModelTriangle");
        let mut memo = nlcs::LayerMemo::default();
        assert!(memo.is(&name, "OG", Some("TRIANGULATIE"), Some(Status::B)));
        assert!(!memo.is(&name, "OG", Some("TRIANGULATIE"), Some(Status::N)));
        assert!(memo.is("B-XX-OG-TRIANGULATIE-G", "OG", Some("TRIANGULATIE"), Some(Status::B)));
    }

    #[test]
    fn linetypes_of_the_files_have_patterns() {
        for s in all().iter().filter(|s| !s.native_nlcs) {
            for r in &s.rules {
                let lt = r.linetype.as_str();
                assert!(lt.is_empty() || lt.eq_ignore_ascii_case("CONTINUOUS") || nlcs::linetype_pattern(lt).is_some(), "{}: linetype {lt}", s.id);
            }
        }
    }

    #[test]
    fn rgb_colours_take_the_nearest_aci() {
        assert_eq!(parse_color("255,0,0"), Some((1, None)));
        let (aci, tc) = parse_color("105,0,0").unwrap();
        assert_eq!(tc, Some((105, 0, 0)));
        assert!(aci > 9);
        assert_eq!(parse_color("0"), None);
    }

    #[test]
    fn user_files_are_checked() {
        assert!(parse("#id\tx\ngroup\tname\tlayer\nRI\t*\tC-SEW", true).unwrap_err().contains("catch-all"));
        assert!(parse("#id\tx\n*\t*\tX-MISC\nRI\t*\tC-SEW\t\t300", true).unwrap_err().contains("colour"));
        let s = parse("#id\tmy-dot\n#status\tB=EX\tN=PR\n*\t*\tX-MISC{-status}\nRI\tDWA*\tSAN-{status}\t\t3", true).unwrap();
        assert_eq!(s.layer_name(obj("RI", "DWA_RIOOLLEIDING_PVC_250"), Status::B, Element::G), "SAN-EX");
        assert_eq!(s.layer_name(obj("VH", "FUNDERING"), Status::N, Element::G), "X-MISC-PR");
    }
}
