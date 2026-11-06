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

/// The positions of an object's attribute values in its layer name. The
/// positions in a symbol's name (`symboolnaam_posities`) are not layer parts.
fn positions(kind: &str) -> Vec<(&'static str, usize)> {
    model().laagnaam_posities.get(kind).map(|p| p.iter().map(|(k, v)| (k.as_str(), *v)).collect()).unwrap_or_default()
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

/// The sub-theme a kind's name starts with (`MSstation` → `MS`,
/// `OVLoverdrachtspunt` → `OV`) and the length of that prefix.
fn sub_theme(kind: &str) -> Option<(&'static str, usize)> {
    ["LHS", "LS", "MS", "HS"].into_iter().find(|p| kind.starts_with(p)).map(|p| (p, p.len())).or(kind.starts_with("OVL").then_some(("OV", 3)))
}

/// The one layer of a kind that carries no attribute values, as parts
/// (without status and toestand): a symbol's, or that of a kind without
/// layer positions. Learned from the examples, else for a symbol the
/// template layer named after its kind (`MSmof` → `ET_MS_MOF`).
fn fixed_layer(kind: &str, element: &str, operator: Option<&str>) -> Option<Vec<String>> {
    if element != "S" && !positions(kind).is_empty() {
        return None;
    }
    if let Some(base) = learned().vaste_lagen.get(kind).and_then(|m| m.get(element)) {
        return Some(base.split('_').map(str::to_string).collect());
    }
    let (sub, n) = sub_theme(kind).filter(|_| element == "S")?;
    let parts = vec!["ET".to_string(), sub.to_string(), kind[n..].to_ascii_uppercase()];
    templates(operator).iter().any(|l| l.element == "S" && l.parts == parts).then_some(parts)
}

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
        if let Some((sub, _)) = sub_theme(&o.kind) {
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
    if let Some(parts) = fixed_layer(&o.kind, element, operator) {
        return Some(LayerName { status, parts, toestand, haakjes: false, element: element.into() }.compose());
    }
    let pos = if element == "S" { Vec::new() } else { positions(&o.kind) };
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
    let sub = sub_theme(&o.kind).map(|s| s.0);
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
/// for. A fixed layer (a symbol's, a cabinet's) names its kind and no
/// values; a symbol layer names nothing else, as a symbol's values are in
/// its symbol name. Otherwise every object whose layer positions, filled
/// from the name, give the name back; the one with the most positions wins.
pub fn for_layer(name: &str, operator: Option<&str>) -> Option<(String, Vec<(String, String)>)> {
    let l = parse(name)?;
    let m = model();
    let body = l.parts.join("_");
    if let Some(kind) = m.objecten.keys().find(|k| fixed_layer(k, &l.element, operator).is_some_and(|p| p.join("_").eq_ignore_ascii_case(&body))) {
        return Some((kind.clone(), Vec::new()));
    }
    if l.element == "S" {
        return None;
    }
    let mut best: Option<(usize, String, Vec<(String, String)>)> = None;
    for kind in m.objecten.keys() {
        let pos = positions(kind);
        if pos.is_empty() || pos.iter().any(|(_, p)| *p >= l.parts.len()) {
            continue;
        }
        let mut o = Object { kind: kind.clone(), ..Default::default() };
        for (attr, p) in &pos {
            o.set(attr, l.parts[*p].clone());
        }
        o.set("Status", match l.status.as_str() { "N" => "NIEUW", "R" => "REVISIE", "V" => "VERWIJDERD", _ => "BESTAAND" });
        o.geom = Some(match l.element.as_str() {
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

    /// A symbol layer names one kind and no attribute values: those are in
    /// the symbol's name (`symboolnaam_posities`), not in its layer.
    #[test]
    fn symbol_layers_name_their_own_kind() {
        let cases = [
            ("B-OI-KL-ET_LS_AANSLUITPUNT-S", "LSoverdrachtspunt"),
            ("B-OI-KL-ET_MS_AANSLUITPUNT-S", "MSoverdrachtspunt"),
            ("B-OI-KL-ET_OV_AANSLUITPUNT-S", "OVLoverdrachtspunt"),
            ("B-OI-KL-ET_LS_MOF-S", "LSmof"),
            ("B-OI-KL-ET_MS_MOF-S", "MSmof"),
            ("B-OI-KL-ET_HS_MOF-S", "HSmof"),
            ("N-OI-KL-ET_LS_MOF-VERLATEN-S", "LSmof"),
        ];
        for op in [Some("Alliander"), Some("Enexis"), None] {
            for (name, kind) in cases {
                assert_eq!(for_layer(name, op), Some((kind.to_string(), Vec::new())), "{name} ({op:?})");
            }
            // The other way, each symbol lands on its own layer.
            for (name, kind) in cases.iter().take(6) {
                let o = Object { kind: kind.to_string(), geom: Some(Geom::Point([0.0; 3])), ..Default::default() };
                assert_eq!(for_object(&o, op).as_deref(), Some(*name), "{kind} ({op:?})");
            }
            // A layer that is no kind's symbol layer is not guessed from its parts.
            for name in ["B-OI-KL-ET_HS_AARDING-S", "B-OI-KL-ET_AANLEGTECHNIEK_GESTUURDE BORING-S"] {
                assert_eq!(for_layer(name, op), None, "{name} ({op:?})");
            }
        }
    }

    /// Every fixed layer belongs to one kind only, and gives it back.
    #[test]
    fn fixed_layers_are_unique() {
        for op in [Some("Alliander"), Some("Enexis")] {
            let mut seen: HashMap<String, &str> = HashMap::new();
            for kind in model().objecten.keys() {
                for element in ELEMENTS {
                    if let Some(parts) = fixed_layer(kind, element, op) {
                        let name = format!("B-OI-KL-{}-{element}", parts.join("_"));
                        assert!(seen.insert(name.clone(), kind).is_none(), "{name} twice ({op:?})");
                        assert_eq!(for_layer(&name, op), Some((kind.clone(), Vec::new())), "{name} ({op:?})");
                    }
                }
            }
            assert!(seen.len() >= 7, "{seen:?}");
        }
    }
}
