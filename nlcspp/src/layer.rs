//! The NLCS++ layer-name grammar, both ways:
//!
//! `<Status>-OI-KL-<part>_<part>_…[-<Bedrijfstoestand>|-(<Bedrijfstoestand>)]-<element>`
//!
//! with element `G` (lines), `GV`/`V` (areas) or `S` (symbols), for example
//! `B-OI-KL-ET_LS_AANSLUITNET_400 V_4x10Cu as10-G`. Which attribute value
//! stands at which part follows the information model ("Deel van NLCS
//! laagnaam"); the fixed parts (`ET`, `MOF`, `STATION`, …) come from the
//! layers of the operators' DWG templates, so names are only made as the
//! templates have them.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::model::{model, template_layers};
use crate::xml::{Geom, Object};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerName {
    /// `B`, `N`, `R`, `V`, …
    pub status: String,
    pub parts: Vec<String>,
    /// `RESERVE`, `VERLATEN`, … (`IN BEDRIJF` is left out of names).
    pub toestand: Option<String>,
    /// Written in brackets: `-(RESERVE)`.
    pub haakjes: bool,
    pub element: String,
}

const ELEMENTS: [&str; 5] = ["GV", "G", "V", "SO", "S"];

/// Splits a layer name; `None` when it is no NLCS++ cables-and-pipes layer.
pub fn parse(name: &str) -> Option<LayerName> {
    let (status, rest) = name.split_once("-OI-KL-")?;
    if status.is_empty() || status.len() > 2 {
        return None;
    }
    let (body, element) = rest.rsplit_once('-')?;
    if !ELEMENTS.contains(&element) {
        return None;
    }
    let toestanden = model().list(None, "Bedrijfstoestand").unwrap_or(&[]);
    let (body, toestand, haakjes) = match body.rsplit_once('-') {
        Some((b, t)) if t.starts_with('(') && t.ends_with(')') => (b, Some(t[1..t.len() - 1].to_string()), true),
        Some((b, t)) if toestanden.iter().any(|x| x == t) => (b, Some(t.to_string()), false),
        _ => (body, None, false),
    };
    Some(LayerName { status: status.to_string(), parts: body.split('_').map(str::to_string).collect(), toestand, haakjes, element: element.to_string() })
}

impl LayerName {
    pub fn compose(&self) -> String {
        let mut s = format!("{}-OI-KL-{}", self.status, self.parts.join("_"));
        if let Some(t) = &self.toestand {
            if self.haakjes {
                s.push_str(&format!("-({t})"));
            } else {
                s.push_str(&format!("-{t}"));
            }
        }
        s.push('-');
        s.push_str(&self.element);
        s
    }
}

/// The status letter of a Status value.
pub fn status_letter(status: &str) -> &'static str {
    match status.to_ascii_uppercase().as_str() {
        "NIEUW" => "N",
        "REVISIE" => "R",
        "VERWIJDERD" => "V",
        _ => "B",
    }
}

/// The element of a geometry: lines `G`, areas `GV`, points `S`.
pub fn element_of(g: Option<&Geom>) -> &'static str {
    match g {
        Some(Geom::Point(_)) => "S",
        Some(Geom::Polygon(_)) => "GV",
        _ => "G",
    }
}

/// The template layers of an operator, parsed, the toestand-free ones only.
fn templates(operator: Option<&str>) -> &'static [LayerName] {
    static CACHE: OnceLock<HashMap<&'static str, Vec<LayerName>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let mut m = HashMap::new();
        for op in ["Alliander", "Enexis"] {
            let mut v: Vec<LayerName> = template_layers(Some(op)).iter().filter_map(|n| parse(n)).filter(|l| l.toestand.is_none()).collect();
            v.sort_by(|a, b| a.parts.cmp(&b.parts).then(a.element.cmp(&b.element)));
            v.dedup_by(|a, b| a.parts == b.parts && a.element == b.element);
            m.insert(op, v);
        }
        m
    });
    cache.get(if operator == Some("Alliander") { "Alliander" } else { "Enexis" }).map(Vec::as_slice).unwrap_or(&[])
}

/// The positions of an object's attribute values in its layer name (or, for
/// symbols, in its symbol name).
fn positions(kind: &str, symbol: bool) -> Vec<(&'static str, usize)> {
    let m = model();
    let src = if symbol { m.symboolnaam_posities.get(kind).filter(|p| !p.is_empty()) } else { None }.or_else(|| m.laagnaam_posities.get(kind));
    src.map(|p| p.iter().map(|(k, v)| (k.as_str(), *v)).collect()).unwrap_or_default()
}

#[derive(serde::Deserialize)]
struct Learned {
    /// Per kind: part position → the fixed value there.
    vaste_delen: HashMap<String, HashMap<String, String>>,
    /// Per kind: element → the whole fixed name (symbols, cabinets).
    vaste_lagen: HashMap<String, HashMap<String, String>>,
}

/// Fixed parts and fixed layers per object kind, derived from the official
/// example deliveries (`assets/uit-voorbeelden.json`).
fn learned() -> &'static Learned {
    static L: OnceLock<Learned> = OnceLock::new();
    L.get_or_init(|| serde_json::from_str(include_str!("../assets/uit-voorbeelden.json")).expect("NLCS++ examples"))
}

const SUBTHEMES: [&str; 5] = ["LS", "MS", "HS", "LHS", "OV"];

/// The fixed parts of a kind's layer: learned from the examples, else from
/// its name (`MSstation` → `ET`, `MS`); the object's `Thema` (`ET_LS`,
/// `GAS_LD`) overrides the theme family and a sub-theme.
fn fixed_parts(o: &Object) -> Vec<(usize, String)> {
    let mut fixed: Vec<(usize, String)> = learned()
        .vaste_delen
        .get(&o.kind)
        .map(|m| m.iter().filter_map(|(p, v)| p.parse().ok().map(|p: usize| (p, v.clone()))).collect())
        .unwrap_or_default();
    if fixed.is_empty() {
        let k = &o.kind;
        let sub = ["LHS", "LS", "MS", "HS"].into_iter().find(|p| k.starts_with(p)).or(k.starts_with("OVL").then_some("OV"));
        if let Some(sub) = sub {
            fixed = vec![(0, "ET".into()), (1, sub.into())];
        }
    }
    if let Some(theme) = o.get("Thema").filter(|t| *t != "OVERIG") {
        let tp: Vec<&str> = theme.split('_').collect();
        for (i, v) in tp.iter().enumerate().take(2) {
            match fixed.iter_mut().find(|(p, _)| *p == i) {
                Some(slot) if i == 0 || SUBTHEMES.contains(&slot.1.as_str()) => slot.1 = v.to_string(),
                None if i == 0 => fixed.push((0, v.to_string())),
                _ => {}
            }
        }
    }
    fixed.sort();
    fixed
}

/// The layer of an object: the template layer whose parts carry its values
/// at their positions and the kind's fixed parts (a value the template does
/// not have becomes `OVERIG`, as the templates do), with its status letter
/// and toestand. Symbols and cabinets have one fixed layer per kind.
pub fn for_object(o: &Object, operator: Option<&str>) -> Option<String> {
    let element = element_of(o.geom.as_ref());
    let toestand = o.get("Bedrijfstoestand").filter(|t| !matches!(*t, "IN BEDRIJF" | "GEEN")).map(str::to_string);
    let status: String = status_letter(o.get("Status").unwrap_or("BESTAAND")).into();
    let attr_pos = positions(&o.kind, false);
    if let Some(base) = learned().vaste_lagen.get(&o.kind).and_then(|m| m.get(element)) {
        if element == "S" || attr_pos.is_empty() {
            return Some(LayerName { status, parts: base.split('_').map(str::to_string).collect(), toestand, haakjes: false, element: element.into() }.compose());
        }
    }
    let pos = if element == "S" { Vec::new() } else { attr_pos };
    let fixed = fixed_parts(o);
    let mut wanted: Vec<(usize, String)> = pos.iter().filter_map(|(attr, p)| o.get(attr).map(|v| (*p, v.to_string()))).collect();
    if wanted.is_empty() && fixed.is_empty() {
        return None;
    }
    let max = pos.iter().map(|p| p.1).chain(fixed.iter().map(|f| f.0)).max().unwrap_or(0);
    wanted.retain(|(p, _)| !fixed.iter().any(|f| f.0 == *p));
    let n_attrs = wanted.len();
    wanted.extend(fixed);
    let fits = |l: &LayerName, w: &[(usize, String)], exact_len: bool| {
        l.element == element && (!exact_len || l.parts.len() == max + 1) && w.iter().all(|(p, v)| l.parts.get(*p).is_some_and(|x| x.eq_ignore_ascii_case(v)))
    };
    let pool = templates(operator);
    let pick = |w: &[(usize, String)]| -> Option<&LayerName> {
        pool.iter().find(|l| fits(l, w, true)).or_else(|| pool.iter().filter(|l| fits(l, w, false)).min_by_key(|l| l.parts.len()))
    };
    // A missing attribute: OVERIG where the template has it, else the kind's
    // sub-theme (an LS cable without Functie is LS), as the examples do.
    let missing: Vec<usize> = pos.iter().filter(|(attr, p)| o.get(attr).is_none() && !wanted.iter().any(|w| w.0 == *p)).map(|(_, p)| *p).collect();
    let sub = ["LHS", "LS", "MS", "HS"].into_iter().find(|p| o.kind.starts_with(p)).or(o.kind.starts_with("OVL").then_some("OV"));
    for p in missing {
        for guess in std::iter::once("OVERIG").chain(sub) {
            let mut trial = wanted.clone();
            trial.push((p, guess.to_string()));
            if pick(&trial).is_some() {
                wanted = trial;
                break;
            }
        }
    }
    // Attribute values the template lacks become OVERIG, the last ones first.
    let mut w = wanted.clone();
    let mut found = pick(&w);
    let mut i = n_attrs;
    while found.is_none() && i > 0 {
        i -= 1;
        w[i].1 = "OVERIG".into();
        found = pick(&w);
    }
    let l = found?;
    Some(LayerName { status, parts: l.parts.clone(), toestand, haakjes: false, element: l.element.clone() }.compose())
}

/// The other way: the object kind and attribute values a layer name stands
/// for. Every object whose positions, filled from the name, give the name
/// back; the first (fewest positions) wins.
pub fn for_layer(name: &str, operator: Option<&str>) -> Option<(String, Vec<(String, String)>)> {
    let l = parse(name)?;
    let symbol = l.element == "S";
    let m = model();
    let mut best: Option<(usize, String, Vec<(String, String)>)> = None;
    for kind in m.objecten.keys() {
        let pos = positions(kind, symbol);
        if pos.is_empty() || pos.iter().any(|(_, p)| *p >= l.parts.len()) {
            continue;
        }
        let mut o = Object { kind: kind.clone(), ..Default::default() };
        for (attr, p) in &pos {
            o.set(attr, l.parts[*p].clone());
        }
        o.set("Status", match l.status.as_str() { "N" => "NIEUW", "R" => "REVISIE", "V" => "VERWIJDERD", _ => "BESTAAND" });
        o.geom = Some(match l.element.as_str() {
            "S" => Geom::Point([0.0; 3]),
            "GV" | "V" => Geom::Polygon(vec![]),
            _ => Geom::Line(vec![]),
        });
        // Compared without the toestand: it is not part of the object's kind.
        let Some(back) = for_object(&o, operator) else { continue };
        let target = LayerName { toestand: None, haakjes: false, ..l.clone() }.compose();
        if back.eq_ignore_ascii_case(&target) && best.as_ref().is_none_or(|b| pos.len() > b.0) {
            let attrs = pos.iter().map(|(a, p)| (a.to_string(), l.parts[*p].clone())).filter(|(_, v)| v != "OVERIG").collect();
            best = Some((pos.len(), kind.clone(), attrs));
        }
    }
    best.map(|(_, k, a)| (k, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_compose() {
        for n in ["B-OI-KL-ET_LS_AANSLUITNET_400 V_4x10Cu as10-G", "N-OI-KL-DATA_TELECOM_AANSLUITNET-(RESERVE)-G", "B-OI-KL-ET_MS_STATION_NETSTATION-GV", "B-OI-KL-ET_LS_MOF-S", "B-OI-KL-ET_AANLEGTECHNIEK_OVERIG-VERLATEN-G"] {
            let l = parse(n).unwrap();
            assert_eq!(l.compose(), n);
        }
        let l = parse("B-OI-KL-ET_LS_AANSLUITNET_400 V_4x10Cu as10-G").unwrap();
        assert_eq!((l.status.as_str(), l.parts.len(), l.parts[3].as_str(), l.element.as_str()), ("B", 5, "400 V", "G"));
        assert_eq!(parse("N-OI-KL-DATA_TELECOM_AANSLUITNET-(RESERVE)-G").unwrap().toestand.as_deref(), Some("RESERVE"));
        assert!(parse("B-WE-VH-ASFALT-G").is_none());
    }

    fn cable(functie: &str, subnet: &str, spanning: &str, uitvoering: &str, status: &str) -> Object {
        let mut o = Object { kind: "LSkabel".into(), geom: Some(Geom::Line(vec![])), ..Default::default() };
        for (k, v) in [("Functie", functie), ("Subnettype", subnet), ("Spanningsniveau", spanning), ("Uitvoering", uitvoering), ("Status", status), ("Bedrijfstoestand", "IN BEDRIJF")] {
            o.set(k, v);
        }
        o
    }

    #[test]
    fn object_to_layer_and_back() {
        let o = cable("LS", "AANSLUITNET", "400 V", "4x10Cu as10", "NIEUW");
        let name = for_object(&o, Some("Alliander")).unwrap();
        assert_eq!(name, "N-OI-KL-ET_LS_AANSLUITNET_400 V_4x10Cu as10-G");
        let (kind, attrs) = for_layer(&name, Some("Alliander")).unwrap();
        assert_eq!(kind, "LSkabel");
        assert!(attrs.contains(&("Uitvoering".into(), "4x10Cu as10".into())));
        // A cable type the template does not know: OVERIG, as in the examples.
        let o = cable("LS", "AANSLUITNET", "400 V", "9x999Zz", "BESTAAND");
        assert_eq!(for_object(&o, Some("Alliander")).unwrap(), "B-OI-KL-ET_LS_AANSLUITNET_400 V_OVERIG-G");
    }
}
