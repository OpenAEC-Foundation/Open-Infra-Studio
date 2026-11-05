//! Checks of an NLCS++ delivery against the information model, in the
//! levels the sector uses: **Fout** (the netbeheerder rejects it: a missing
//! required field, a value outside the code list, too long, a wrong type,
//! wrong geometry), **Waarschuwing** (accepted, but probably not meant: an
//! unknown field, a layer that does not fit the attributes) and
//! **Informerend** (worth knowing: a code list the operator leaves empty).
//! The operators' rule R.5 (GisId on existing objects) is included as Fout.
//!
//! These are the schema rules (XSD), done without an XML schema processor.
//! The operators' content rules (Schematron, mijnaansluiting/Inhoudelijke-
//! validaties) need Saxon; `tools/nlcspp_schematron.sh` runs them.

use crate::model::{model, Field};
use crate::xml::{Delivery, Geom, Object};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Fout,
    Waarschuwing,
    Informerend,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Fout => "Fout",
            Level::Waarschuwing => "Waarschuwing",
            Level::Informerend => "Informerend",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub level: Level,
    /// `LSkabel 561D` (kind and handle) or `AprojectReferentie`.
    pub object: String,
    pub field: String,
    pub message: String,
}

fn label(o: &Object) -> String {
    match o.get("Handle").or_else(|| o.get("ID")) {
        Some(h) => format!("{} {h}", o.kind),
        None => o.kind.clone(),
    }
}

fn value_ok(kind: &str, v: &str) -> bool {
    match kind {
        "integer" | "int" | "long" | "nonNegativeInteger" | "positiveInteger" => v.parse::<i64>().is_ok(),
        "decimal" | "double" | "float" => v.replace(',', ".").parse::<f64>().is_ok() && !v.contains(','),
        "date" => v.len() == 10 && v.as_bytes()[4] == b'-' && v.as_bytes()[7] == b'-' && v[..4].parse::<u32>().is_ok(),
        "dateTime" => v.len() >= 19 && v.as_bytes()[10] == b'T',
        "boolean" => matches!(v, "true" | "false" | "1" | "0"),
        _ => true,
    }
}

fn check_fields(o: &Object, fields: &[Field], operator: Option<&str>, out: &mut Vec<Finding>) {
    let m = model();
    let who = label(o);
    let mut add = |level, field: &str, message: String| out.push(Finding { level, object: who.clone(), field: field.to_string(), message });
    for f in fields {
        if f.kind == "geometry" {
            if f.required && o.geom.is_none() {
                add(Level::Fout, &f.name, "geometrie ontbreekt".into());
            }
            continue;
        }
        // The ID is made on export from the handle.
        if f.name == "ID" {
            continue;
        }
        let Some(v) = o.get(&f.name) else {
            if f.required {
                add(Level::Fout, &f.name, "verplicht veld ontbreekt".into());
            }
            continue;
        };
        if let Some(max) = f.max_length {
            if v.chars().count() > max {
                add(Level::Fout, &f.name, format!("{} tekens, hooguit {max}", v.chars().count()));
            }
        }
        if let Some(list) = &f.list {
            match m.list(operator, list) {
                Some(values) if !values.iter().any(|x| x == v) => {
                    let near: Vec<&String> = values.iter().filter(|x| x.eq_ignore_ascii_case(v)).collect();
                    let hint = match near.first() {
                        Some(n) => format!(" (bedoeld: \"{n}\"?)"),
                        None => String::new(),
                    };
                    add(Level::Fout, &f.name, format!("\"{v}\" staat niet in keuzelijst {list}{}{hint}", operator.map(|o| format!(" van {o}")).unwrap_or_default()));
                }
                None => add(Level::Informerend, &f.name, format!("keuzelijst {list} is leeg voor deze netbeheerder; \"{v}\" niet getoetst")),
                _ => {}
            }
        }
        if let Some(values) = &f.values {
            if !values.iter().any(|x| x == v) {
                add(Level::Fout, &f.name, format!("\"{v}\" is geen toegestane waarde ({})", values.join(", ")));
            }
        }
        if !value_ok(&f.kind, v) {
            add(Level::Fout, &f.name, format!("\"{v}\" is geen {}", f.kind));
        }
    }
    for (k, _) in &o.attrs {
        if !fields.iter().any(|f| &f.name == k) {
            add(Level::Waarschuwing, k, "veld bestaat niet in NLCS++ V12.1 en wordt niet geleverd".into());
        }
    }
}

fn geometry_ok(kind: Option<&str>, g: &Geom) -> bool {
    match (kind, g) {
        (Some("point" | "multipoint"), Geom::Point(_)) => true,
        (Some("curve" | "multicurve"), Geom::Line(_)) => true,
        (Some("surface" | "multisurface"), Geom::Polygon(_)) => true,
        (Some("any") | None, _) => true,
        _ => false,
    }
}

/// All findings of a delivery for `operator` (None: the general lists only),
/// worst first.
pub fn check(d: &Delivery, operator: Option<&str>) -> Vec<Finding> {
    let m = model();
    let mut out = Vec::new();
    match &d.project {
        Some(p) => check_fields(p, &m.project, operator, &mut out),
        None => out.push(Finding { level: Level::Fout, object: "AprojectReferentie".into(), field: String::new(), message: "projectreferentie ontbreekt (verplicht in V12 en V12.1)".into() }),
    }
    for o in &d.objects {
        let Some(def) = m.object(&o.kind) else {
            out.push(Finding { level: Level::Fout, object: label(o), field: String::new(), message: format!("onbekend objecttype {}", o.kind) });
            continue;
        };
        check_fields(o, &def.velden, operator, &mut out);
        if let Some(g) = &o.geom {
            if !geometry_ok(def.geometrie.as_deref(), g) {
                out.push(Finding { level: Level::Fout, object: label(o), field: "Geometry".into(), message: format!("verkeerde geometrie: {} verwacht", def.geometrie.as_deref().unwrap_or("?")) });
            }
        }
        // Rule R.5 of the operators' content validations.
        if matches!(o.get("Status"), Some("BESTAAND" | "REVISIE" | "VERWIJDERD")) && o.get("GisId").is_none() {
            out.push(Finding { level: Level::Fout, object: label(o), field: "GisId".into(), message: "GisId ontbreekt bij een object met status BESTAAND, REVISIE of VERWIJDERD (regel R.5 van de netbeheerders)".into() });
        }
    }
    out.sort_by(|a, b| a.level.cmp(&b.level).then(a.object.cmp(&b.object)));
    out
}

/// A layer that does not fit an object's attributes (only where the
/// grammar can make one): a warning with the layer it should be on.
pub fn check_layer(o: &Object, layer: &str, operator: Option<&str>) -> Option<Finding> {
    let want = crate::layer::for_object(o, operator)?;
    (!want.eq_ignore_ascii_case(layer)).then(|| Finding { level: Level::Waarschuwing, object: label(o), field: "laag".into(), message: format!("ligt op {layer}, de attributen horen bij {want}") })
}

/// Counts per level: (fout, waarschuwing, informerend).
pub fn counts(f: &[Finding]) -> (usize, usize, usize) {
    f.iter().fold((0, 0, 0), |(a, b, c), x| match x.level {
        Level::Fout => (a + 1, b, c),
        Level::Waarschuwing => (a, b + 1, c),
        Level::Informerend => (a, b, c + 1),
    })
}

/// The findings as a Markdown report.
pub fn markdown(f: &[Finding], title: &str) -> String {
    let (a, b, c) = counts(f);
    let mut s = format!("# NLCS++-controle: {title}\n\n{a} fout(en), {b} waarschuwing(en), {c} informerend.\n\n| Niveau | Object | Veld | Bevinding |\n|---|---|---|---|\n");
    for x in f {
        s.push_str(&format!("| {} | {} | {} | {} |\n", x.level.label(), x.object, x.field, x.message.replace('|', "/")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::read;

    #[test]
    fn levels() {
        let x = r#"<NLCSnetbeheerEnexis xmlns:gml="http://www.opengis.net/gml/3.2" xmlns="NS">
  <LSkabel><Handle>1</Handle><Uitvoering>9x999Zz</Uitvoering><Status>BESTAAND</Status><Bedrijfstoestand>IN BEDRIJF</Bedrijfstoestand><Kleur>rood</Kleur><DatumAanleg>10-10-2023</DatumAanleg>
    <Geometry><gml:Point srsName="EPSG:28992"><gml:pos>1 2</gml:pos></gml:Point></Geometry></LSkabel>
  <LSmof><Handle>2</Handle><Status>NIEUW</Status></LSmof>
  <Robot><Handle>3</Handle></Robot>
</NLCSnetbeheerEnexis>"#;
        let d = read(x).unwrap();
        let f = check(&d, d.operator.as_deref());
        let has = |level, obj: &str, field: &str| f.iter().any(|x| x.level == level && x.object == obj && x.field == field);
        assert!(has(Level::Fout, "AprojectReferentie", ""));
        assert!(has(Level::Fout, "LSkabel 1", "Uitvoering"), "{f:#?}");
        assert!(has(Level::Fout, "LSkabel 1", "DatumAanleg"));
        assert!(has(Level::Fout, "LSkabel 1", "Geometry"), "a point for a cable");
        assert!(has(Level::Waarschuwing, "LSkabel 1", "Kleur"));
        assert!(has(Level::Fout, "LSkabel 1", "GisId"), "rule R.5");
        assert!(has(Level::Fout, "LSmof 2", "Bedrijfstoestand"));
        assert!(has(Level::Fout, "LSmof 2", "Geometry"));
        assert!(has(Level::Fout, "Robot 3", ""));
        assert_eq!(f[0].level, Level::Fout);
        assert!(markdown(&f, "test").contains("| Fout | LSkabel 1 | Uitvoering |"));
    }
}
