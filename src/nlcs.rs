//! NLCS — Nederlandse CAD Standaard (GWW) 5.02.
//!
//! Layer names are built as `STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT`
//! (e.g. `B-XX-VH-GESLOTENVERHARDING_ASFALT-G`); every object carries its own
//! colour, lineweight and linetype per drawing status. The tables come from
//! the official publication (assets/nlcs, CC BY 4.0, DigiGO / Stichting NLCS).

use std::collections::HashMap;
use std::sync::OnceLock;

const OBJECTS: &str = include_str!("../assets/nlcs/objects.tsv");
const COLORS: &str = include_str!("../assets/nlcs/colors.tsv");
const BGT: &str = include_str!("../assets/nlcs/bgt.tsv");
const LINETYPES: &str = include_str!("../assets/nlcs/linetypes.tsv");
const DISCIPLINES: &str = include_str!("../assets/nlcs/disciplines.tsv");
const GROUPS: &str = include_str!("../assets/nlcs/groups.tsv");

/// Drawing status (tekenstatus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Bestaand werk.
    B,
    /// Nieuw werk.
    N,
    /// Tijdelijk werk.
    T,
    /// Verwijderd / vervallen werk.
    V,
}

impl Status {
    pub fn code(self) -> &'static str {
        match self {
            Status::B => "B",
            Status::N => "N",
            Status::T => "T",
            Status::V => "V",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
    pub const ALL: [Status; 4] = [Status::B, Status::N, Status::T, Status::V];
    pub fn label(self) -> &'static str {
        match self {
            Status::B => "B — bestaand",
            Status::N => "N — nieuw",
            Status::T => "T — tijdelijk",
            Status::V => "V — te verwijderen",
        }
    }
}

/// The description of a main group ("KL" → "Kabels en Leidingen").
pub fn group_name(code: &str) -> &'static str {
    GROUPS.lines().skip(1).find_map(|l| l.split_once('\t').filter(|(c, _)| *c == code).map(|(_, d)| d.trim())).unwrap_or("")
}

/// An object name for people: "GESLOTENVERHARDING_ASFALT" → "Geslotenverharding asfalt".
pub fn humanize(name: &str) -> String {
    let lower = name.replace('_', " ").to_lowercase();
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

impl Status {
    pub fn from_code(code: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.code() == code)
    }
}

/// A layer name taken apart: `STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT`.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerName {
    pub status: Status,
    pub discipline: String,
    pub group: String,
    /// May itself contain hyphens (e.g. `WATERGANG-WERK`).
    pub object: String,
    pub element: String,
}

/// Why a layer name is not an NLCS name, or its parts.
pub fn parse_layer(name: &str) -> Result<LayerName, String> {
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() < 5 {
        return Err("geen NLCS-laagnaam (STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT)".into());
    }
    let status = Status::from_code(parts[0]).ok_or_else(|| format!("tekenstatus '{}' bestaat niet (B, N, T of V)", parts[0]))?;
    let (discipline, group) = (parts[1], parts[2]);
    if !disciplines().iter().any(|(c, _)| *c == discipline) {
        return Err(format!("discipline '{discipline}' bestaat niet"));
    }
    if group_name(group).is_empty() {
        return Err(format!("hoofdgroep '{group}' bestaat niet"));
    }
    let element = parts[parts.len() - 1].to_string();
    let object = parts[3..parts.len() - 1].join("-");
    Ok(LayerName { status, discipline: discipline.into(), group: group.into(), object, element })
}

/// Element codes for annotation, allowed on every object: text, dimensions,
/// leaders.
pub const ANNOTATION_ELEMENTS: &[&str] = &["T", "M", "L"];

/// Whether `element` may be used for the object (the table's G/S/A, plus
/// annotation).
pub fn element_allowed(obj: &Object, element: &str) -> bool {
    ANNOTATION_ELEMENTS.contains(&element) || obj.element.split('/').any(|e| e.trim() == element) || (obj.element.trim().is_empty() && element == "G")
}

/// NLCS disciplines: (code, short description).
pub fn disciplines() -> Vec<(&'static str, &'static str)> {
    DISCIPLINES
        .lines()
        .skip(1)
        .filter_map(|l| {
            let (code, desc) = l.split_once('\t')?;
            let short = desc.split([',', '.', ':']).next().unwrap_or(desc).trim();
            Some((code, short))
        })
        .collect()
}

/// Element type, the last part of the layer name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Element {
    /// Geometrie.
    G,
    /// Symbool.
    S,
    /// Tekst.
    T,
    /// Arcering.
    A,
}

impl Element {
    pub fn code(self) -> &'static str {
        match self {
            Element::G => "G",
            Element::S => "S",
            Element::T => "T",
            Element::A => "A",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Style {
    /// Lineweight in mm.
    pub lineweight: Option<f64>,
    /// NLCS colour number (0–255).
    pub color: Option<u8>,
    pub linetype: String,
}

#[derive(Clone, Debug)]
pub struct Object {
    pub id: u32,
    pub group: String,
    pub name: String,
    pub parent: Option<u32>,
    /// Allowed element types, e.g. "G/S".
    pub element: String,
    pub discipline: String,
    /// Styles for status B, N, T, V.
    pub styles: [Style; 4],
}

struct Tables {
    by_id: HashMap<u32, Object>,
    by_name: HashMap<(String, String), u32>,
    colors: Vec<(u8, u8, u8)>,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();
        for line in OBJECTS.lines().skip(1) {
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() < 18 {
                continue;
            }
            let Ok(id) = c[0].parse::<u32>() else { continue };
            let style = |i: usize| Style {
                lineweight: c[i].replace(',', ".").parse().ok(),
                color: c[i + 1].parse().ok(),
                linetype: c[i + 2].to_string(),
            };
            let obj = Object {
                id,
                group: c[1].to_string(),
                name: c[2].to_string(),
                parent: c[3].parse().ok(),
                element: c[4].to_string(),
                discipline: c[5].to_string(),
                styles: [style(6), style(9), style(12), style(15)],
            };
            by_name.entry((obj.group.clone(), obj.name.clone())).or_insert(id);
            by_id.insert(id, obj);
        }
        let mut colors = vec![(255u8, 255u8, 255u8); 256];
        for line in COLORS.lines().skip(1) {
            let mut it = line.split('\t');
            if let (Some(n), Some(hex)) = (it.next().and_then(|n| n.parse::<usize>().ok()), it.next()) {
                let h = hex.trim_start_matches('#');
                if let (Ok(r), Ok(g), Ok(b), true) = (
                    u8::from_str_radix(h.get(0..2).unwrap_or(""), 16),
                    u8::from_str_radix(h.get(2..4).unwrap_or(""), 16),
                    u8::from_str_radix(h.get(4..6).unwrap_or(""), 16),
                    n < 256,
                ) {
                    colors[n] = (r, g, b);
                }
            }
        }
        Tables { by_id, by_name, colors }
    })
}

pub fn object(id: u32) -> Option<&'static Object> {
    tables().by_id.get(&id)
}

/// Find an object by main group and name, e.g. ("KL", "ET_LS").
pub fn object_by_name(group: &str, name: &str) -> Option<&'static Object> {
    tables().by_name.get(&(group.to_string(), name.to_string())).and_then(|id| object(*id))
}

pub fn color_rgb(n: u8) -> (u8, u8, u8) {
    tables().colors[n as usize]
}

/// Everything needed to create an NLCS layer.
#[derive(Clone, Debug)]
pub struct LayerSpec {
    pub name: String,
    /// NLCS colour number, which is the ACI colour index.
    pub color: u8,
    pub rgb: (u8, u8, u8),
    /// A true colour in place of the NLCS colour (the IMKL display option).
    pub true_color: Option<(u8, u8, u8)>,
    /// Lineweight in hundredths of a mm (DWG convention).
    pub lineweight: Option<i16>,
    pub linetype: String,
    pub object_id: u32,
}

/// The NLCS layer for an object, drawing status, discipline and element type.
pub fn layer(obj: &Object, status: Status, discipline: &str, element: Element) -> LayerSpec {
    // A table that leaves the requested status empty falls back to nieuw
    // werk, then to the other statuses (the AL objects only fill bestaand).
    let order = [status.index(), Status::N.index(), Status::B.index(), Status::T.index(), Status::V.index()];
    let pick = |f: &dyn Fn(&Style) -> bool| order.iter().map(|i| &obj.styles[*i]).find(|s| f(s));
    let color = pick(&|s| s.color.is_some()).and_then(|s| s.color).unwrap_or(7);
    let lineweight = pick(&|s| s.lineweight.is_some()).and_then(|s| s.lineweight);
    let linetype = pick(&|s| !s.linetype.is_empty()).map(|s| s.linetype.clone()).unwrap_or_else(|| "CONTINUOUS".into());
    LayerSpec {
        name: format!("{}-{}-{}-{}-{}", status.code(), discipline, obj.group, obj.name, element.code()),
        color,
        rgb: color_rgb(color),
        true_color: None,
        lineweight: lineweight.map(|mm| (mm * 100.0).round() as i16),
        linetype,
        object_id: obj.id,
    }
}

/// The dash pattern of an NLCS linetype in paper millimetres (positive dash,
/// negative gap, zero dot), or `None` for CONTINUOUS and unknown names.
/// Complex linetypes lose their shape and text elements: the pattern keeps
/// their dashes and the gaps the symbols sat in.
pub fn linetype_pattern(name: &str) -> Option<Vec<f64>> {
    if name.is_empty() || name.eq_ignore_ascii_case("CONTINUOUS") {
        return None;
    }
    let def = LINETYPES.lines().skip(1).find_map(|l| {
        let mut c = l.split('\t');
        if c.next()?.eq_ignore_ascii_case(name) { c.nth(1) } else { None }
    })?;
    parse_pattern(def)
}

/// Whether the linetype carries shapes or text the simplified pattern drops.
pub fn linetype_is_complex(name: &str) -> bool {
    LINETYPES.lines().skip(1).any(|l| {
        let mut c = l.split('\t');
        c.next().is_some_and(|n| n.eq_ignore_ascii_case(name)) && c.nth(1).is_some_and(|d| d.contains('['))
    })
}

fn parse_pattern(def: &str) -> Option<Vec<f64>> {
    let body = def.trim().strip_prefix("A,").or_else(|| def.trim().strip_prefix("a,"))?;
    // Drop the bracketed complex elements (they hold commas of their own).
    // A symbol leaves a marker: between two dashes it becomes a short gap,
    // so a linetype of dashes and symbols only (GW-MAAIVELD-SO) stays valid.
    let mut plain = String::new();
    let mut depth = 0;
    for ch in body.chars() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    plain.push('S');
                }
            }
            _ if depth == 0 => plain.push(ch),
            _ => {}
        }
    }
    let mut out: Vec<f64> = Vec::new();
    for t in plain.split(',').map(str::trim) {
        if t == "S" {
            if out.last().is_none_or(|v| *v >= 0.0) {
                out.push(-1.0);
            }
            continue;
        }
        let Ok(v) = t.parse::<f64>() else { continue };
        match out.last_mut() {
            // Two gaps in a row (a symbol sat between them): one longer gap.
            Some(last) if *last < 0.0 && v < 0.0 => *last += v,
            _ => out.push(v),
        }
    }
    if out.len() > 1 && out[0] < 0.0 && out.last().is_some_and(|l| *l < 0.0) {
        let first = out.remove(0);
        *out.last_mut().unwrap() += first;
    }
    out.truncate(12);
    let ok = out.len() >= 2 && out.iter().any(|v| *v >= 0.0) && out.iter().any(|v| *v < 0.0);
    ok.then_some(out)
}

/// The NLCS text class for a text height in paper millimetres (the ZZ
/// objects "(T1.8)" … "(T7.0)"): the colour number and lineweight (1/100 mm)
/// a text of that height is drawn with.
pub fn text_class(mm: f64) -> Option<(u8, i16)> {
    let classes = [(1.8, "(T1.8)"), (2.5, "(T2.5)"), (3.0, "(T3.0)"), (3.5, "(T3.5)"), (5.0, "(T5.0)"), (7.0, "(T7.0)")];
    let (_, name) = classes.iter().min_by(|a, b| (a.0 - mm).abs().total_cmp(&(b.0 - mm).abs()))?;
    let obj = object_by_name("ZZ", name)?;
    let st = &obj.styles[Status::B.index()];
    Some((st.color?, (st.lineweight? * 100.0).round() as i16))
}

/// NLCS objects for BGT texts: street names and house numbers.
pub fn bgt_label_object(collection: &str) -> Option<&'static Object> {
    match collection {
        "openbareruimtelabel" => object_by_name("OG", "BEBOUWING_STRAATNAAM"),
        "pand_nummeraanduiding" => object_by_name("OG", "BEBOUWING_HUISNUMMER"),
        _ => None,
    }
}

/// Fallback for a BGT feature no rule matches: the object's general rule
/// (the one with the fewest criteria).
pub fn bgt_default(collection: &str) -> Option<u32> {
    let object = bgt_object_name(collection)?;
    bgt_rules().iter().filter(|r| r.object == object).min_by_key(|r| r.criteria.len()).map(|r| r.nlcs_object)
}

// ── BGT → NLCS (official NLCS-BGT 5.0 mapping) ──────────────────────────────

struct BgtRule {
    object: String,
    /// IMGeo attribute values that must all occur on the feature (lowercase).
    criteria: Vec<String>,
    nlcs_object: u32,
    symbol: Option<u32>,
}

fn bgt_rules() -> &'static Vec<BgtRule> {
    static R: OnceLock<Vec<BgtRule>> = OnceLock::new();
    R.get_or_init(|| {
        BGT.lines()
            .skip(1)
            .filter_map(|line| {
                let c: Vec<&str> = line.split('\t').collect();
                if c.len() < 15 {
                    return None;
                }
                Some(BgtRule {
                    object: c[0].to_uppercase(),
                    criteria: c[1..13].iter().map(|v| norm(v)).filter(|v| !v.is_empty()).collect(),
                    nlcs_object: c[13].parse().ok()?,
                    symbol: c[14].parse().ok().filter(|s: &u32| *s > 0),
                })
            })
            .collect()
    })
}

fn norm(v: &str) -> String {
    v.trim().to_lowercase().replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The mapping's object name for a PDOK BGT collection (`wegdeel`, `kunstwerkdeel_vlak`, …).
pub fn bgt_object_name(collection: &str) -> Option<&'static str> {
    let base = collection.split('_').next().unwrap_or(collection);
    Some(match base {
        "wegdeel" => "WEGDEEL",
        "ondersteunendwegdeel" => "ONDERSTEUNEND WEGDEEL",
        "begroeidterreindeel" => "BEGROEID TERREINDEEL",
        "onbegroeidterreindeel" => "ONBEGROEID TERREINDEEL",
        "waterdeel" => "WATERDEEL",
        "ondersteunendwaterdeel" => "ONDERSTEUNEND WATERDEEL",
        "pand" => "PAND",
        "overigbouwwerk" => "OVERIG BOUWWERK",
        "overbruggingsdeel" => "OVERBRUGGINGSDEEL",
        "tunneldeel" => "TUNNELDEEL",
        "kunstwerkdeel" => "KUNSTWERKDEEL",
        "scheiding" => "SCHEIDING",
        "overigescheiding" => "OVERIGE SCHEIDING",
        "spoor" => "SPOOR",
        "straatmeubilair" => "STRAATMEUBILAIR",
        "paal" => "PAAL",
        "put" => "PUT",
        "kast" => "KAST",
        "bord" => "BORD",
        "mast" => "MAST",
        "sensor" => "SENSOR",
        "bak" => "BAK",
        "weginrichtingselement" => "WEGINRICHTINGSELEMENT",
        "waterinrichtingselement" => "WATERINRICHTINGSELEMENT",
        "vegetatieobject" => "VEGETATIEOBJECT",
        "gebouwinstallatie" => "GEBOUWINSTALLATIE",
        "installatie" => "INSTALLATIE",
        "openbareruimtelabel" => "OPENBARE RUIMTE LABEL",
        "functioneelgebied" => "FUNCTIONEEL GEBIED",
        "ongeclassificeerdobject" => "ONGECLASSIFICEERD OBJECT",
        _ => return None,
    })
}

/// One row of the NLCS-BGT mapping, with the IMGeo attribute each value is
/// for (the table's column names: `function`, `surfaceMaterial`, `class`,
/// `bgt-type`, `plus-type`, …). For the way back: NLCS design → BGT.
pub struct BgtRow {
    /// The mapping's object name (`WEGDEEL`, `BEGROEID TERREINDEEL`, …).
    pub object: String,
    pub values: Vec<(String, String)>,
}

/// The mapping rows that lead to NLCS object `nlcs_object`.
pub fn bgt_rows(nlcs_object: u32) -> Vec<BgtRow> {
    let mut lines = BGT.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split('\t').collect();
    lines
        .filter_map(|line| {
            let c: Vec<&str> = line.split('\t').collect();
            if c.len() < 15 || c[13].parse::<u32>().ok()? != nlcs_object {
                return None;
            }
            let values = (1..13).filter(|&i| !c[i].trim().is_empty()).map(|i| (head[i].to_string(), c[i].trim().to_string())).collect();
            Some(BgtRow { object: c[0].trim().to_uppercase(), values })
        })
        .collect()
}

/// Most specific mapping rule whose attribute values all occur on the feature.
/// Returns (NLCS object id, NLCS symbol id).
pub fn bgt_lookup(collection: &str, props: &serde_json::Map<String, serde_json::Value>) -> Option<(u32, Option<u32>)> {
    let object = bgt_object_name(collection)?;
    let values: Vec<String> = props
        .iter()
        .filter(|(k, _)| !k.ends_with("_leeg") && !k.ends_with("_codespace"))
        .filter_map(|(_, v)| v.as_str())
        .map(norm)
        .filter(|v| !v.is_empty())
        .collect();
    bgt_rules()
        .iter()
        .filter(|r| r.object == object && r.criteria.iter().all(|c| values.iter().any(|v| v == c)))
        .max_by_key(|r| r.criteria.len())
        .map(|r| (r.nlcs_object, r.symbol))
}

// ── KLIC → NLCS (hoofdgroep KL / RI) ────────────────────────────────────────

/// NLCS object for a KLIC (IMKL) theme and element kind.
pub fn klic_object(thema: &str, kind: &str) -> Option<&'static Object> {
    let point = kind == "Appurtenance";
    let casing = kind == "Mantelbuis" || kind == "Kabelbed";
    let candidates: &[(&str, &str)] = match (thema, point, casing) {
        ("laagspanning", false, false) => &[("KL", "ET_LS")],
        ("middenspanning", false, false) => &[("KL", "ET_MS")],
        ("hoogspanning", false, false) => &[("KL", "ET_HS")],
        ("landelijkHoogspanningsnet", false, false) => &[("KL", "ET_ZHS"), ("KL", "ET_HS")],
        ("laagspanning" | "middenspanning" | "hoogspanning" | "landelijkHoogspanningsnet", true, _) => &[("KL", "ET_HULPSTUK")],
        ("laagspanning" | "middenspanning" | "hoogspanning" | "landelijkHoogspanningsnet", _, true) => &[("KL", "ET_MANTELBUIS")],
        ("datatransport", false, false) => &[("KL", "DATA_TELECOM"), ("KL", "DATA_ALGEMEEN")],
        ("datatransport", true, _) => &[("KL", "DATA_HULPSTUK")],
        ("datatransport", _, true) => &[("KL", "DATA_MANTELBUIS")],
        ("gasLageDruk", false, false) => &[("KL", "GAS_LD")],
        ("gasHogeDruk", false, false) => &[("KL", "GAS_HD")],
        ("gasLageDruk" | "gasHogeDruk", true, _) => &[("KL", "GAS_HULPSTUK")],
        ("gasLageDruk" | "gasHogeDruk", _, true) => &[("KL", "GAS_MANTELBUIS")],
        ("water", false, false) => &[("KL", "WATER_TRANSPORTLEIDING")],
        ("water", true, _) => &[("KL", "WATER_HULPSTUK")],
        ("water", _, true) => &[("KL", "WATER_MANTELBUIS")],
        ("warmte", false, false) => &[("KL", "WARMTENET_TRANSPORTLEIDING")],
        ("warmte", true, _) => &[("KL", "WARMTENET_HULPSTUK")],
        ("warmte", _, true) => &[("KL", "WARMTENET_MANTELBUIS")],
        ("petrochemie" | "buisleidingGevaarlijkeInhoud", false, false) => &[("KL", "CHEMIE_LEIDING"), ("KL", "OLIE_LEIDING")],
        ("petrochemie" | "buisleidingGevaarlijkeInhoud", true, _) => &[("KL", "CHEMIE_HULPSTUK")],
        ("petrochemie" | "buisleidingGevaarlijkeInhoud", _, true) => &[("KL", "CHEMIE_MANTELBUIS")],
        ("rioolVrijverval", false, _) => &[("RI", "GWA_RIOOLLEIDING"), ("RI", "GWA")],
        ("rioolOnderOverOfOnderdruk", false, _) => &[("RI", "PERSLEIDING_TRANSPORTLEIDING"), ("RI", "PERSLEIDING")],
        ("rioolVrijverval" | "rioolOnderOverOfOnderdruk", true, _) => &[("RI", "GWA_RIOOLPUT"), ("RI", "OVERIG_LOZEPUT")],
        (_, true, _) => &[("KL", "DATA_HULPSTUK")],
        _ => &[("KL", "DATA_ALGEMEEN")],
    };
    candidates.iter().find_map(|(g, n)| object_by_name(g, n))
}

/// The IMKL theme colours of the PMKL "Handreiking visualisatie"
/// (Geonovum, version 3.0), for the optional "IMKL-themakleuren" display
/// instead of the NLCS colours.
pub fn imkl_rgb(thema: &str) -> (u8, u8, u8) {
    match thema {
        "datatransport" => (0, 255, 0),
        "gasLageDruk" => (255, 215, 80),
        "gasHogeDruk" => (255, 175, 60),
        "buisleidingGevaarlijkeInhoud" => (255, 127, 0),
        "hoogspanning" | "landelijkHoogspanningsnet" => (255, 0, 0),
        "middenspanning" => (200, 0, 0),
        "laagspanning" => (150, 0, 0),
        "petrochemie" => (182, 74, 0),
        "rioolVrijverval" => (186, 56, 168),
        "rioolOnderOverOfOnderdruk" => (128, 0, 128),
        "warmte" => (0, 128, 128),
        "water" => (0, 0, 255),
        "wees" => (145, 138, 111),
        // "overig" and anything else.
        _ => (111, 92, 16),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_load_and_build_layers() {
        let ls = object_by_name("KL", "ET_LS").expect("ET_LS");
        let l = layer(ls, Status::B, "XX", Element::G);
        assert_eq!(l.name, "B-XX-KL-ET_LS-G");
        assert!(l.lineweight.is_some());
        assert_eq!(color_rgb(1), (255, 0, 0));
        assert_eq!(color_rgb(30), (255, 127, 0));
    }

    #[test]
    fn bgt_mapping_picks_most_specific_rule() {
        let props: serde_json::Map<String, serde_json::Value> = serde_json::from_str(
            r#"{"functie":"rijbaan lokale weg","fysiek_voorkomen":"open verharding","plus_fysiek_voorkomen":"gebakken klinkers"}"#,
        )
        .unwrap();
        let (id, _) = bgt_lookup("wegdeel", &props).expect("mapping");
        let o = object(id).unwrap();
        assert_eq!(o.group, "VH");
        assert!(o.name.starts_with("OPENVERHARDING"), "{}", o.name);
        let empty = serde_json::Map::new();
        assert_eq!(bgt_lookup("wegdeel", &empty).map(|x| x.0), Some(1620));
    }

    #[test]
    fn imkl_colours_follow_the_pmkl_handreiking() {
        // Hex values of the PMKL Handreiking visualisatie 3.0.
        let table = [
            ("datatransport", "00ff00"),
            ("gasLageDruk", "ffd750"),
            ("gasHogeDruk", "ffaf3c"),
            ("buisleidingGevaarlijkeInhoud", "ff7f00"),
            ("landelijkHoogspanningsnet", "ff0000"),
            ("hoogspanning", "ff0000"),
            ("middenspanning", "c80000"),
            ("laagspanning", "960000"),
            ("petrochemie", "b64a00"),
            ("rioolVrijverval", "ba38a8"),
            ("rioolOnderOverOfOnderdruk", "800080"),
            ("warmte", "008080"),
            ("water", "0000ff"),
            ("wees", "918a6f"),
            ("overig", "6f5c10"),
        ];
        for (thema, hex) in table {
            let (r, g, b) = imkl_rgb(thema);
            assert_eq!(format!("{r:02x}{g:02x}{b:02x}"), hex, "{thema}");
        }
    }

    #[test]
    fn klic_themes_map_to_kl_objects() {
        for t in ["laagspanning", "middenspanning", "datatransport", "gasLageDruk", "water", "rioolVrijverval", "rioolOnderOverOfOnderdruk", "warmte"] {
            assert!(klic_object(t, "Elektriciteitskabel").is_some(), "{t}");
            assert!(klic_object(t, "Appurtenance").is_some(), "{t}");
        }
    }

    #[test]
    fn linetype_patterns_simplify_complex_elements() {
        assert_eq!(linetype_pattern("AM-AS-SO"), Some(vec![10.0, -2.0, 2.0, -2.0]));
        assert_eq!(linetype_pattern("CONTINUOUS"), None);
        // A,4,-1,[CIRC…],-1,4,-1,["H"…],-2 → dashes keep their place.
        assert_eq!(linetype_pattern("KL-GAS_HD-SO"), Some(vec![4.0, -2.0, 4.0, -3.0]));
        assert!(linetype_is_complex("KL-GAS_HD-SO"));
        assert!(!linetype_is_complex("AM-AS-SO"));
        let al = layer(object_by_name("AL", "TEKENBLAD_KADER").unwrap(), Status::N, "XX", Element::G);
        assert_eq!(al.lineweight, Some(70));
        assert!(bgt_label_object("openbareruimtelabel").is_some());
        assert!(bgt_label_object("pand_nummeraanduiding").is_some());
        assert!(bgt_default("kunstwerkdeel_lijn").is_some());
        assert_eq!(text_class(2.5), Some((7, 25)));
        assert_eq!(text_class(1.8), Some((10, 18)));
    }

    #[test]
    fn symbol_only_linetypes_get_a_pattern() {
        assert_eq!(linetype_pattern("GW-MAAIVELD-SO"), Some(vec![12.0, -1.0, 12.0]));
        // Symbols between gaps leave the gaps as they were.
        assert_eq!(linetype_pattern("MO-BODEM_CONTOUR_INTERVENTIEWAARDE-SO"), Some(vec![17.25, -14.0, 17.25]));
    }

    #[test]
    fn layer_names_are_parsed() {
        let l = parse_layer("B-XX-VH-GESLOTENVERHARDING_ASFALT-G").unwrap();
        assert_eq!((l.status, l.discipline.as_str(), l.group.as_str(), l.object.as_str(), l.element.as_str()), (Status::B, "XX", "VH", "GESLOTENVERHARDING_ASFALT", "G"));
        assert_eq!(parse_layer("N-WA-GW-WATERGANG-WERK-A").unwrap().object, "WATERGANG-WERK");
        assert!(parse_layer("0").is_err());
        assert!(parse_layer("X-XX-VH-WEG-G").unwrap_err().contains("tekenstatus"));
        assert!(parse_layer("B-QQ-VH-WEG-G").unwrap_err().contains("discipline"));
        let obj = object_by_name("VH", "GESLOTENVERHARDING_ASFALT").unwrap();
        assert!(element_allowed(obj, "G") && element_allowed(obj, "T") && !element_allowed(obj, "Q"));
    }
}
