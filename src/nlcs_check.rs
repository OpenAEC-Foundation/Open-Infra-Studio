//! NLCS validation of a drawing: layer names,
//! layer colour/lineweight/linetype per drawing status, element properties
//! ByLayer, elements on layer 0, text heights and empty layers. Findings
//! that can be repaired carry their repair.

use std::collections::{BTreeMap, HashMap};

use ocs_plugin_api::host::codec::{CadDocument, Color, EntityType, Handle, LineWeight};

use crate::nlcs::{self, LayerSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Fout,
    Waarschuwing,
    Info,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Fout => "fout",
            Severity::Waarschuwing => "waarschuwing",
            Severity::Info => "info",
        }
    }
}

/// The rules, in report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rule {
    LayerName,
    LayerObject,
    LayerElement,
    LayerColor,
    LayerLineweight,
    LayerLinetype,
    EntityColor,
    EntityLineweight,
    EntityLinetype,
    LayerZero,
    TextHeight,
    EmptyLayer,
}

impl Rule {
    pub fn code(self) -> &'static str {
        match self {
            Rule::LayerName => "L1",
            Rule::LayerObject => "L2",
            Rule::LayerElement => "L3",
            Rule::LayerColor => "L4",
            Rule::LayerLineweight => "L5",
            Rule::LayerLinetype => "L6",
            Rule::EntityColor => "E1",
            Rule::EntityLineweight => "E2",
            Rule::EntityLinetype => "E3",
            Rule::LayerZero => "E4",
            Rule::TextHeight => "E5",
            Rule::EmptyLayer => "L7",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Rule::LayerName => "Laagnaam niet volgens NLCS",
            Rule::LayerObject => "Object bestaat niet in de hoofdgroep",
            Rule::LayerElement => "Elementtype niet toegestaan voor het object",
            Rule::LayerColor => "Laagkleur wijkt af van NLCS",
            Rule::LayerLineweight => "Laaglijndikte wijkt af van NLCS",
            Rule::LayerLinetype => "Laaglijntype wijkt af van NLCS",
            Rule::EntityColor => "Kleur van elementen niet DuLaag (ByLayer)",
            Rule::EntityLineweight => "Lijndikte van elementen niet DuLaag (ByLayer)",
            Rule::EntityLinetype => "Lijntype van elementen niet DuLaag (ByLayer)",
            Rule::LayerZero => "Elementen op laag 0",
            Rule::TextHeight => "Teksthoogte geen NLCS-tekstklasse",
            Rule::EmptyLayer => "Lege NLCS-laag",
        }
    }
    pub fn severity(self) -> Severity {
        match self {
            Rule::LayerName | Rule::LayerObject | Rule::LayerElement | Rule::LayerColor | Rule::LayerLineweight | Rule::LayerLinetype => Severity::Fout,
            Rule::EntityColor | Rule::EntityLineweight | Rule::EntityLinetype | Rule::LayerZero | Rule::TextHeight => Severity::Waarschuwing,
            Rule::EmptyLayer => Severity::Info,
        }
    }
}

/// A repair the plugin can carry out.
#[derive(Clone, Debug)]
pub enum Repair {
    /// Set the layer's colour, lineweight and linetype to the NLCS values.
    LayerStyle(LayerSpec),
    /// Set colour, lineweight and linetype of these elements to ByLayer.
    ByLayer(Vec<Handle>),
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub rule: Rule,
    pub layer: String,
    pub message: String,
    /// The elements concerned (for selecting them in the drawing).
    pub handles: Vec<Handle>,
    pub repair: Option<Repair>,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// Plot scale denominator, for text heights in paper millimetres.
    pub scale: f64,
    /// Drawing units per metre.
    pub unit_factor: f64,
    pub text_heights: bool,
    pub empty_layers: bool,
    /// Layer colours, lineweights and linetypes are known (not so for a very
    /// large drawing, read from the host's shared view): check them.
    pub layer_props: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { scale: 500.0, unit_factor: 1.0, text_heights: true, empty_layers: true, layer_props: true }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub layers: usize,
    pub nlcs_layers: usize,
    pub entities: usize,
    /// Elements on a valid NLCS layer whose layer and own properties conform.
    pub conform_entities: usize,
    /// Layer colours, lineweights and linetypes were checked.
    pub layer_props: bool,
}

/// "1 fout", "3 fouten".
pub fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Report {
    pub fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.rule.severity() == s).count()
    }
    /// Share of the elements that conform, in percent.
    /// The score as shown: rounded down, so never "100 %" while an element
    /// is off (99,97 % reads 99,9 %).
    pub fn score_label(&self) -> String {
        if self.conform_entities >= self.entities {
            "100 %".into()
        } else {
            format!("{:.1} %", (self.score() * 10.0).floor() / 10.0).replace('.', ",")
        }
    }

    pub fn score(&self) -> f64 {
        if self.entities == 0 { 100.0 } else { self.conform_entities as f64 * 100.0 / self.entities as f64 }
    }
    pub fn by_rule(&self) -> BTreeMap<Rule, Vec<&Finding>> {
        let mut m: BTreeMap<Rule, Vec<&Finding>> = BTreeMap::new();
        for f in &self.findings {
            m.entry(f.rule).or_default().push(f);
        }
        m
    }
    pub fn repairs(&self) -> Vec<&Repair> {
        self.findings.iter().filter_map(|f| f.repair.as_ref()).collect()
    }
}

/// Layers left out of the check: the CAD system's own and external references.
fn exempt(name: &str) -> bool {
    name == "0" || name.eq_ignore_ascii_case("Defpoints") || name.contains('|')
}

fn color_index(c: &Color) -> Option<u8> {
    match c {
        Color::Index(i) => Some(*i),
        _ => None,
    }
}

fn lineweight_value(l: &LineWeight) -> Option<i16> {
    match l {
        LineWeight::Value(v) => Some(*v),
        _ => None,
    }
}

/// Text height of an element in drawing units, if it is a text.
fn text_height(e: &EntityType) -> Option<f64> {
    match e {
        EntityType::Text(t) => Some(t.height),
        EntityType::MText(t) => Some(t.height),
        _ => None,
    }
}

/// The NLCS text classes in paper millimetres.
const TEXT_CLASSES: &[f64] = &[1.8, 2.5, 3.0, 3.5, 5.0, 7.0];

pub fn check(doc: &CadDocument, opt: &Options) -> Report {
    let mut report = Report { layers: doc.layers.iter().count(), layer_props: opt.layer_props, ..Default::default() };
    // Elements per layer.
    let mut per_layer: HashMap<String, Vec<&EntityType>> = HashMap::new();
    for e in doc.model_space_entities() {
        if matches!(e, EntityType::Viewport(_) | EntityType::AttributeDefinition(_)) {
            continue;
        }
        per_layer.entry(e.common().layer.clone()).or_default().push(e);
        report.entities += 1;
    }
    let mut layers: Vec<_> = doc.layers.iter().collect();
    layers.sort_by(|a, b| a.name.cmp(&b.name));
    let paper = opt.scale / 1000.0 * opt.unit_factor;

    for layer in layers {
        let name = layer.name.clone();
        let elements = per_layer.get(&name).map(Vec::as_slice).unwrap_or(&[]);
        let handles = || elements.iter().map(|e| e.common().handle).collect::<Vec<_>>();
        if name == "0" {
            if !elements.is_empty() {
                report.findings.push(Finding { rule: Rule::LayerZero, layer: name, message: format!("{} element(en) op laag 0", elements.len()), handles: handles(), repair: None });
            }
            continue;
        }
        if exempt(&name) || layer.flags.xref_dependent {
            report.conform_entities += elements.len();
            continue;
        }
        let parsed = match nlcs::parse_layer(&name) {
            Ok(p) => p,
            Err(why) => {
                report.findings.push(Finding { rule: Rule::LayerName, layer: name, message: format!("{why} — {} element(en)", elements.len()), handles: handles(), repair: None });
                continue;
            }
        };
        let Some(obj) = nlcs::object_by_name(&parsed.group, &parsed.object) else {
            report.findings.push(Finding {
                rule: Rule::LayerObject,
                layer: name,
                message: format!("object '{}' staat niet in hoofdgroep {} ({})", parsed.object, parsed.group, nlcs::group_name(&parsed.group)),
                handles: handles(),
                repair: None,
            });
            continue;
        };
        report.nlcs_layers += 1;
        let mut layer_ok = true;
        if !nlcs::element_allowed(obj, &parsed.element) {
            layer_ok = false;
            report.findings.push(Finding {
                rule: Rule::LayerElement,
                layer: name.clone(),
                message: format!("element '{}' niet toegestaan; {} kent {} (en T, M, L)", parsed.element, obj.name, if obj.element.is_empty() { "G" } else { obj.element.as_str() }),
                handles: handles(),
                repair: None,
            });
        }
        // Layer properties against the table for this status.
        let element = match parsed.element.as_str() {
            "S" => nlcs::Element::S,
            "A" => nlcs::Element::A,
            "T" => nlcs::Element::T,
            _ => nlcs::Element::G,
        };
        let spec = nlcs::layer(obj, parsed.status, &parsed.discipline, element);
        let spec = LayerSpec { name: name.clone(), ..spec };
        let mut style = Vec::new();
        if opt.layer_props && color_index(&layer.color) != Some(spec.color.max(1)) {
            style.push(Rule::LayerColor);
            report.findings.push(Finding {
                rule: Rule::LayerColor,
                layer: name.clone(),
                message: format!("kleur {} — NLCS {}", describe_color(&layer.color), spec.color),
                handles: Vec::new(),
                repair: None,
            });
        }
        if let Some(lw) = spec.lineweight.filter(|_| opt.layer_props) {
            if lineweight_value(&layer.line_weight) != Some(lw) {
                style.push(Rule::LayerLineweight);
                report.findings.push(Finding {
                    rule: Rule::LayerLineweight,
                    layer: name.clone(),
                    message: format!("lijndikte {} — NLCS {:.2} mm", describe_lineweight(&layer.line_weight), lw as f64 / 100.0),
                    handles: Vec::new(),
                    repair: None,
                });
            }
        }
        if opt.layer_props && !layer.line_type.eq_ignore_ascii_case(&spec.linetype) {
            style.push(Rule::LayerLinetype);
            report.findings.push(Finding {
                rule: Rule::LayerLinetype,
                layer: name.clone(),
                message: format!("lijntype {} — NLCS {}", layer.line_type, spec.linetype),
                handles: Vec::new(),
                repair: None,
            });
        }
        if !style.is_empty() {
            layer_ok = false;
            // One repair for the layer, on its first style finding.
            if let Some(f) = report.findings.iter_mut().rev().find(|f| f.layer == name && style.contains(&f.rule)) {
                f.repair = Some(Repair::LayerStyle(spec.clone()));
            }
        }
        // Element properties: ByLayer, except the text class pen of texts.
        let mut colour = Vec::new();
        let mut weight = Vec::new();
        let mut ltype = Vec::new();
        let mut heights = Vec::new();
        let mut entity_bad = 0usize;
        for e in elements {
            let c = e.common();
            let mm = text_height(e).map(|h| h / paper);
            let class = mm.and_then(nlcs::text_class);
            let pen_ok = |col: &Color, lw: &LineWeight| class.is_some_and(|(cc, cl)| color_index(col) == Some(cc) && lineweight_value(lw).is_none_or(|v| v == cl));
            let mut bad = false;
            if !matches!(c.color, Color::ByLayer) && !pen_ok(&c.color, &c.line_weight) {
                colour.push(c.handle);
                bad = true;
            }
            if !matches!(c.line_weight, LineWeight::ByLayer) && !pen_ok(&c.color, &c.line_weight) {
                weight.push(c.handle);
                bad = true;
            }
            if !(c.linetype.is_empty() || c.linetype.eq_ignore_ascii_case("ByLayer")) {
                ltype.push(c.handle);
                bad = true;
            }
            if opt.text_heights {
                if let Some(mm) = mm {
                    if !TEXT_CLASSES.iter().any(|k| (k - mm).abs() < 0.05) {
                        heights.push((c.handle, mm));
                    }
                }
            }
            if bad {
                entity_bad += 1;
            }
        }
        for (rule, list) in [(Rule::EntityColor, &colour), (Rule::EntityLineweight, &weight), (Rule::EntityLinetype, &ltype)] {
            if !list.is_empty() {
                report.findings.push(Finding {
                    rule,
                    layer: name.clone(),
                    message: format!("{} van {} element(en)", list.len(), elements.len()),
                    handles: list.clone(),
                    repair: Some(Repair::ByLayer(list.clone())),
                });
            }
        }
        if !heights.is_empty() {
            let mut sizes: Vec<String> = heights.iter().map(|(_, mm)| format!("{mm:.2}")).collect();
            sizes.sort();
            sizes.dedup();
            report.findings.push(Finding {
                rule: Rule::TextHeight,
                layer: name.clone(),
                message: format!("{} tekst(en) van {} mm op papier bij 1:{} (klassen 1,8 · 2,5 · 3,5 · 5 · 7 mm)", heights.len(), sizes.join(" / "), opt.scale),
                handles: heights.iter().map(|(h, _)| *h).collect(),
                repair: None,
            });
        }
        if elements.is_empty() && opt.empty_layers {
            report.findings.push(Finding { rule: Rule::EmptyLayer, layer: name.clone(), message: "geen elementen".into(), handles: Vec::new(), repair: None });
        }
        if layer_ok {
            report.conform_entities += elements.len() - entity_bad;
        }
    }
    report.findings.sort_by(|a, b| (a.rule, &a.layer).cmp(&(b.rule, &b.layer)));
    report
}

fn describe_color(c: &Color) -> String {
    match c {
        Color::Index(i) => i.to_string(),
        Color::Rgb { r, g, b } => format!("RGB {r},{g},{b}"),
        Color::ByLayer => "DuLaag".into(),
        Color::ByBlock => "DuBlok".into(),
        Color::None => "geen".into(),
    }
}

fn describe_lineweight(l: &LineWeight) -> String {
    match l {
        LineWeight::Value(v) => format!("{:.2} mm", *v as f64 / 100.0),
        LineWeight::ByLayer => "DuLaag".into(),
        LineWeight::ByBlock => "DuBlok".into(),
        LineWeight::Default => "standaard".into(),
    }
}

/// Carry out repairs in the drawing: layer styles, then element properties
/// to ByLayer (one undo step).
pub fn repair(doc: &CadDocument, repairs: &[&Repair]) -> Result<String, String> {
    let mut layers = 0;
    let mut wanted: std::collections::HashSet<Handle> = std::collections::HashSet::new();
    for r in repairs {
        match r {
            Repair::LayerStyle(spec) => {
                crate::bridge::restyle_layer(spec)?;
                layers += 1;
            }
            Repair::ByLayer(h) => wanted.extend(h.iter().copied()),
        }
    }
    let mut changed: Vec<EntityType> = Vec::new();
    for e in doc.model_space_entities() {
        if wanted.contains(&e.common().handle) {
            let mut e = e.clone();
            let c = e.common_mut();
            c.color = Color::ByLayer;
            c.line_weight = LineWeight::ByLayer;
            c.linetype = String::new();
            changed.push(e);
        }
    }
    let n = crate::bridge::update_entities("NLCS: elementen DuLaag", changed)?;
    Ok(format!("NLCS hersteld: {layers} laag/lagen kregen de NLCS-kleur, -lijndikte en -lijntype; {n} element(en) staan nu op DuLaag."))
}

/// A Markdown report for the project file.
pub fn markdown(r: &Report, drawing: &str, opt: &Options) -> String {
    let mut md = format!(
        "# NLCS-controle — {drawing}\n\nNLCS 5.02, tekstklassen bij 1:{}. {} lagen ({} NLCS), {} elementen; **{} conform**. {}, {}, {}.\n\n",
        opt.scale,
        r.layers,
        r.nlcs_layers,
        r.entities,
        r.score_label(),
        counted(r.count(Severity::Fout), "fout", "fouten"),
        counted(r.count(Severity::Waarschuwing), "waarschuwing", "waarschuwingen"),
        counted(r.count(Severity::Info), "melding", "meldingen")
    );
    if !r.layer_props {
        md.push_str("_Laagkleur, -lijndikte en -lijntype zijn niet gecontroleerd: deze grote tekening is via de gedeelde weergave van Open CAD Studio gelezen, die alleen laagnamen bevat._\n\n");
    }
    for (rule, list) in r.by_rule() {
        md.push_str(&format!("## {} {} ({}, {}×)\n\n| laag | melding |\n|---|---|\n", rule.code(), rule.title(), rule.severity().label(), list.len()));
        for f in list {
            md.push_str(&format!("| `{}` | {} |\n", f.layer, f.message.replace('|', "/")));
        }
        md.push('\n');
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{tables::Layer, Line, Text, Vector3};

    fn layer(doc: &mut CadDocument, name: &str, color: u8, lw: i16, lt: &str) {
        let mut l = Layer::new(name);
        l.color = Color::Index(color);
        l.line_weight = LineWeight::Value(lw);
        l.line_type = lt.into();
        let _ = doc.layers.add(l);
    }

    fn line(doc: &mut CadDocument, layer: &str, color: Color) {
        let mut e = EntityType::Line(Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)));
        e.common_mut().layer = layer.into();
        e.common_mut().color = color;
        let _ = doc.add_entity(e);
    }

    #[test]
    fn finds_and_repairs() {
        let mut doc = CadDocument::new();
        let good = nlcs::layer(nlcs::object_by_name("VH", "GESLOTENVERHARDING_ASFALT").unwrap(), nlcs::Status::B, "XX", nlcs::Element::G);
        layer(&mut doc, &good.name, good.color, good.lineweight.unwrap(), &good.linetype);
        line(&mut doc, &good.name, Color::ByLayer);
        // Wrong colour on the layer, an element with its own colour.
        layer(&mut doc, "B-XX-VH-OPENVERHARDING-G", 1, 18, "CONTINUOUS");
        line(&mut doc, "B-XX-VH-OPENVERHARDING-G", Color::Index(3));
        // Not NLCS, unknown object, wrong element.
        layer(&mut doc, "WEGEN", 7, 25, "CONTINUOUS");
        line(&mut doc, "WEGEN", Color::ByLayer);
        layer(&mut doc, "B-XX-VH-BESTAATNIET-G", 7, 25, "CONTINUOUS");
        layer(&mut doc, "B-XX-VH-GESLOTENVERHARDING_ASFALT-Q", 7, 25, "CONTINUOUS");
        line(&mut doc, "0", Color::ByLayer);
        // A text of 2.5 mm at 1:500 in the T2.5 pen is fine; 4 mm is not a class.
        for (h, c) in [(1.25, Color::Index(7)), (2.0, Color::ByLayer)] {
            let mut t = EntityType::Text(Text::with_value("x", Vector3::new(0.0, 0.0, 0.0)).with_height(h));
            t.common_mut().layer = good.name.clone();
            t.common_mut().color = c;
            if h == 1.25 {
                t.common_mut().line_weight = LineWeight::Value(25);
            }
            let _ = doc.add_entity(t);
        }
        let r = check(&doc, &Options::default());
        let rules: Vec<Rule> = r.findings.iter().map(|f| f.rule).collect();
        for expected in [Rule::LayerName, Rule::LayerObject, Rule::LayerElement, Rule::LayerColor, Rule::EntityColor, Rule::LayerZero, Rule::TextHeight] {
            assert!(rules.contains(&expected), "{expected:?} ontbreekt in {rules:?}");
        }
        // The well-made layer itself raises nothing but the text height.
        assert!(r.findings.iter().filter(|f| f.layer == good.name).all(|f| f.rule == Rule::TextHeight), "{:?}", r.findings);
        assert!(r.repairs().iter().any(|x| matches!(x, Repair::LayerStyle(s) if s.name == "B-XX-VH-OPENVERHARDING-G")));
        assert!(r.repairs().iter().any(|x| matches!(x, Repair::ByLayer(h) if h.len() == 1)));
        assert!(r.score() < 100.0 && r.score() > 0.0);
        assert!(markdown(&r, "test.dwg", &Options::default()).contains("L4 Laagkleur"));
    }
}
