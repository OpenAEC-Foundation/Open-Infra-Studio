//! Checking an IFC model against a client's IDS (buildingSMART Information
//! Delivery Specification 1.0). Issue #65.
//!
//! An IDS (XML, namespace `http://standards.buildingsmart.org/IDS`, schema
//! `ids.xsd` 1.0) lists specifications: an *applicability* (which elements)
//! and *requirements* (what they must have), each made of facets: entity,
//! attribute, property, classification, material and partOf. Values are a
//! `simpleValue` or an `xs:restriction` (enumeration, pattern, bounds,
//! lengths). This module parses the IDS with roxmltree and checks a model
//! read by [`crate::step`], following the IDS 1.0 implementers' rules
//! (github.com/buildingSMART/IDS, Documentation/ImplementersDocumentation):
//!
//! - entity: exact uppercase class (no subtypes); PredefinedType from the
//!   occurrence, else its type; `USERDEFINED` resolves to ObjectType (or the
//!   type's ElementType).
//! - attribute: by name, from a table of attribute positions (below); null,
//!   empty strings, empty lists and logical UNKNOWN never satisfy.
//! - property: IfcPropertySet / IfcElementQuantity through
//!   IfcRelDefinesByProperties, plus the type's HasPropertySets (occurrence
//!   wins per property); single, enumerated, list, bounded and table values;
//!   `dataType` must equal the value's IFC type (quantities: their measure).
//!   Length/area/volume measures are converted to SI from the project's
//!   IfcSIUnit prefixes and conversion-based units before comparing.
//! - classification: IfcRelAssociatesClassification → reference chain →
//!   IfcClassification.Name; a value matches the reference or any parent
//!   reference; type classifications apply for systems the occurrence lacks;
//!   IfcExternalReferenceRelationship covers resources such as materials.
//! - material: IfcRelAssociatesMaterial (occurrence, else type): material,
//!   layer/profile/constituent set names, their item names, materials and
//!   categories; material lists; set usages.
//! - partOf: IfcRelAggregates and IfcRelNests (any ancestor),
//!   IfcRelContainedInSpatialStructure (container and its ancestors),
//!   IfcRelAssignsToGroup, IfcRelFillsElement+IfcRelVoidsElement (host).
//!
//! Cardinality: `required` must pass; `optional` passes when the item is
//! absent; `prohibited` is the opposite of required. A specification fails
//! when an applicable element fails or the number of applicable elements is
//! outside minOccurs/maxOccurs on `<applicability>` (defaults: minOccurs 1
//! per xs:occurs; maxOccurs unbounded, as ifctester does, where xs:occurs
//! would say 1). maxOccurs 0 means no element may be applicable.
//! Reals compare with the IDS tolerance |a − b| ≤ 1e-6·|b| + 1e-6.
//!
//! Patterns use a small XSD regex engine (§F of XML Schema Part 2): literals,
//! `.`, classes with ranges and negation, `\d \w \s \i \c` (and capitals),
//! groups, `|`, `? * + {n} {n,} {n,m}`; anchored to the whole string. Not
//! supported, reported as "not checked": `\p{…}` categories and class
//! subtraction.
//!
//! Attribute positions known (IFC2X3/IFC4/IFC4X3 agree on these):
//! IfcRoot GlobalId 0, OwnerHistory 1, Name 2, Description 3; IfcObject
//! ObjectType 4; IfcProduct ObjectPlacement 5, Representation 6; IfcElement
//! Tag 7; IfcSpatialElement LongName 7, CompositionType 8; IfcContext
//! ObjectType 4, LongName 5, Phase 6; IfcTypeObject ApplicableOccurrence 4,
//! HasPropertySets 5; IfcTypeProduct RepresentationMaps 6, Tag 7;
//! IfcElementType ElementType 8; PredefinedType per class (8 for most
//! elements, 9 for element types, with a table of exceptions); IfcMaterial
//! Name/Description/Category; IfcClassification and IfcClassificationReference
//! names; IfcSurfaceStyleRefraction. Classes are recognised by name lists and
//! by the 22-character GlobalId, not by a full EXPRESS schema. The IFC2X3
//! type-mapping table and unit-carrying attributes are not handled.

use crate::step::{Entity, StepModel, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const IDS_NS: &str = "http://standards.buildingsmart.org/IDS";

// ------------------------------------------------------------------- model

/// An IDS value: one literal or an XSD restriction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdsValue {
    Simple(String),
    Restriction(Restriction),
}

/// `xs:restriction` facets; all given kinds must hold, values within one
/// kind (enumeration, pattern) are alternatives.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Restriction {
    /// `xs:string`, `xs:double`, …
    pub base: Option<String>,
    pub enumeration: Vec<String>,
    pub patterns: Vec<String>,
    pub min_inclusive: Option<f64>,
    pub max_inclusive: Option<f64>,
    pub min_exclusive: Option<f64>,
    pub max_exclusive: Option<f64>,
    pub length: Option<usize>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

/// Facet cardinality in requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cardinality {
    Required,
    Optional,
    Prohibited,
}

/// One facet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FacetKind {
    Entity {
        name: IdsValue,
        predefined_type: Option<IdsValue>,
    },
    Attribute {
        name: IdsValue,
        value: Option<IdsValue>,
    },
    Property {
        property_set: IdsValue,
        base_name: IdsValue,
        value: Option<IdsValue>,
        data_type: Option<String>,
    },
    Classification {
        system: Option<IdsValue>,
        value: Option<IdsValue>,
    },
    Material {
        value: Option<IdsValue>,
    },
    PartOf {
        relation: Option<String>,
        name: IdsValue,
        predefined_type: Option<IdsValue>,
    },
}

/// A facet with its cardinality (always `Required` in applicability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Facet {
    pub kind: FacetKind,
    pub cardinality: Cardinality,
    pub instructions: Option<String>,
}

/// One `<specification>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Specification {
    pub name: String,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    /// `IFC2X3`, `IFC4`, `IFC4X3_ADD2`.
    pub ifc_versions: Vec<String>,
    pub min_occurs: u32,
    /// `None` = unbounded.
    pub max_occurs: Option<u32>,
    pub applicability: Vec<Facet>,
    pub requirements: Vec<Facet>,
}

/// A parsed IDS document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ids {
    pub title: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub specifications: Vec<Specification>,
}

// ------------------------------------------------------------------- parse

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text_of(n: roxmltree::Node, name: &str) -> Option<String> {
    child(n, name)
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn parse_value(n: roxmltree::Node) -> Result<IdsValue, String> {
    if let Some(s) = child(n, "simpleValue") {
        return Ok(IdsValue::Simple(s.text().unwrap_or("").to_string()));
    }
    let r = child(n, "restriction").ok_or_else(|| {
        format!(
            "<{}> without simpleValue or xs:restriction",
            n.tag_name().name()
        )
    })?;
    let mut out = Restriction {
        base: r.attribute("base").map(String::from),
        ..Default::default()
    };
    for f in r.children().filter(|c| c.is_element()) {
        let v = f.attribute("value").unwrap_or("").to_string();
        let num = || {
            v.trim()
                .parse::<f64>()
                .map_err(|_| format!("bad number '{v}' in xs:{}", f.tag_name().name()))
        };
        let len = || {
            v.trim()
                .parse::<usize>()
                .map_err(|_| format!("bad length '{v}'"))
        };
        match f.tag_name().name() {
            "enumeration" => out.enumeration.push(v.clone()),
            "pattern" => out.patterns.push(v.clone()),
            "minInclusive" => out.min_inclusive = Some(num()?),
            "maxInclusive" => out.max_inclusive = Some(num()?),
            "minExclusive" => out.min_exclusive = Some(num()?),
            "maxExclusive" => out.max_exclusive = Some(num()?),
            "length" => out.length = Some(len()?),
            "minLength" => out.min_length = Some(len()?),
            "maxLength" => out.max_length = Some(len()?),
            other => return Err(format!("unsupported restriction facet xs:{other}")),
        }
    }
    Ok(IdsValue::Restriction(out))
}

fn opt_value(n: roxmltree::Node, name: &str) -> Result<Option<IdsValue>, String> {
    child(n, name).map(parse_value).transpose()
}

fn parse_facet(n: roxmltree::Node, in_requirements: bool) -> Result<Facet, String> {
    let name = n.tag_name().name();
    let req = |c: &str| {
        child(n, c)
            .ok_or_else(|| format!("<{name}> without <{c}>"))
            .and_then(parse_value)
    };
    let kind = match name {
        "entity" => FacetKind::Entity {
            name: req("name")?,
            predefined_type: opt_value(n, "predefinedType")?,
        },
        "attribute" => FacetKind::Attribute {
            name: req("name")?,
            value: opt_value(n, "value")?,
        },
        "property" => FacetKind::Property {
            property_set: req("propertySet")?,
            base_name: req("baseName")?,
            value: opt_value(n, "value")?,
            data_type: n.attribute("dataType").map(|s| s.to_ascii_uppercase()),
        },
        "classification" => FacetKind::Classification {
            system: opt_value(n, "system")?,
            value: opt_value(n, "value")?,
        },
        "material" => FacetKind::Material {
            value: opt_value(n, "value")?,
        },
        "partOf" => {
            let e = child(n, "entity").ok_or("<partOf> without <entity>")?;
            FacetKind::PartOf {
                relation: n.attribute("relation").map(|s| s.to_ascii_uppercase()),
                name: child(e, "name")
                    .ok_or_else(|| "<partOf><entity> without <name>".to_string())
                    .and_then(parse_value)?,
                predefined_type: opt_value(e, "predefinedType")?,
            }
        }
        other => return Err(format!("unknown facet <{other}>")),
    };
    let cardinality = match (in_requirements, n.attribute("cardinality")) {
        (false, _) | (_, None | Some("required")) => Cardinality::Required,
        (true, Some("optional")) => Cardinality::Optional,
        (true, Some("prohibited")) => Cardinality::Prohibited,
        (_, Some(c)) => return Err(format!("unknown cardinality '{c}'")),
    };
    Ok(Facet {
        kind,
        cardinality,
        instructions: n.attribute("instructions").map(String::from),
    })
}

impl Ids {
    /// Parses IDS 1.0 XML.
    pub fn parse(xml: &str) -> Result<Ids, String> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| format!("IDS XML: {e}"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "ids" {
            return Err("root element is not <ids>".into());
        }
        if root.tag_name().namespace() != Some(IDS_NS) {
            return Err(format!("<ids> is not in namespace {IDS_NS}"));
        }
        let info = child(root, "info");
        let get = |k: &str| info.and_then(|i| text_of(i, k));
        let mut specifications = Vec::new();
        let specs = child(root, "specifications").ok_or("IDS without <specifications>")?;
        for s in specs
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "specification")
        {
            let name = s.attribute("name").unwrap_or("").to_string();
            let ctx = |e: String| format!("specification '{name}': {e}");
            let app = child(s, "applicability").ok_or_else(|| ctx("no <applicability>".into()))?;
            let applicability = app
                .children()
                .filter(|c| c.is_element())
                .map(|c| parse_facet(c, false))
                .collect::<Result<Vec<_>, _>>()
                .map_err(ctx)?;
            let requirements = match child(s, "requirements") {
                Some(r) => r
                    .children()
                    .filter(|c| c.is_element())
                    .map(|c| parse_facet(c, true))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(ctx)?,
                None => Vec::new(),
            };
            let min_occurs = app
                .attribute("minOccurs")
                .map(|v| v.parse().map_err(|_| ctx(format!("bad minOccurs '{v}'"))))
                .transpose()?
                .unwrap_or(1);
            let max_occurs = match app.attribute("maxOccurs") {
                None | Some("unbounded") => None,
                Some(v) => Some(v.parse().map_err(|_| ctx(format!("bad maxOccurs '{v}'")))?),
            };
            specifications.push(Specification {
                identifier: s.attribute("identifier").map(String::from),
                description: s.attribute("description").map(String::from),
                instructions: s.attribute("instructions").map(String::from),
                ifc_versions: s
                    .attribute("ifcVersion")
                    .unwrap_or("")
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
                min_occurs,
                max_occurs,
                applicability,
                requirements,
                name,
            });
        }
        Ok(Ids {
            title: get("title").unwrap_or_default(),
            version: get("version"),
            author: get("author"),
            description: get("description"),
            specifications,
        })
    }

    /// Checks `model` against every specification.
    pub fn check(&self, model: &StepModel) -> IdsReport {
        check(self, model)
    }
}

// ------------------------------------------------------------------- values

/// A value from the IFC model as IDS compares it.
#[derive(Debug, Clone, PartialEq)]
enum Actual {
    Str(String),
    Num(f64),
    Bool(bool),
    /// Logical `.U.`.
    Unknown,
    /// Instance reference or aggregate: not comparable.
    Object,
}

impl Actual {
    fn show(&self) -> String {
        match self {
            Actual::Str(s) => format!("'{s}'"),
            Actual::Num(n) => format!("{n}"),
            Actual::Bool(b) => format!("{b}"),
            Actual::Unknown => "UNKNOWN".into(),
            Actual::Object => "an object".into(),
        }
    }
    /// Empty strings and UNKNOWN never satisfy a facet.
    fn is_empty(&self) -> bool {
        matches!(self, Actual::Unknown) || matches!(self, Actual::Str(s) if s.is_empty())
    }
}

fn actual(v: &Value) -> Option<Actual> {
    Some(match v.inner() {
        Value::Null | Value::Derived => return None,
        Value::Int(i) => Actual::Num(*i as f64),
        Value::Real(r) => Actual::Num(*r),
        Value::Str(s) => Actual::Str(s.clone()),
        Value::Enum(e) if e == "T" => Actual::Bool(true),
        Value::Enum(e) if e == "F" => Actual::Bool(false),
        Value::Enum(e) if e == "U" => Actual::Unknown,
        Value::Enum(e) => Actual::Str(e.clone()),
        Value::Binary(b) => Actual::Str(b.clone()),
        Value::Ref(_) | Value::List(_) | Value::Typed(..) => Actual::Object,
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs() + 1e-6
}

fn simple_matches(s: &str, a: &Actual) -> bool {
    match a {
        Actual::Str(x) => x == s,
        Actual::Num(x) => s.trim().parse::<f64>().is_ok_and(|v| close(*x, v)),
        Actual::Bool(b) => s == if *b { "true" } else { "false" },
        Actual::Unknown | Actual::Object => false,
    }
}

impl IdsValue {
    /// Whether `a` satisfies this value; `Err` when it cannot be checked.
    fn matches(&self, a: &Actual) -> Result<bool, String> {
        match self {
            IdsValue::Simple(s) => Ok(simple_matches(s, a)),
            IdsValue::Restriction(r) => r.matches(a),
        }
    }
    fn matches_str(&self, s: &str) -> Result<bool, String> {
        self.matches(&Actual::Str(s.to_string()))
    }
    /// The literal when this is a simple value.
    pub fn simple(&self) -> Option<&str> {
        match self {
            IdsValue::Simple(s) => Some(s),
            _ => None,
        }
    }
    /// Short human text.
    pub fn describe(&self) -> String {
        match self {
            IdsValue::Simple(s) => format!("'{s}'"),
            IdsValue::Restriction(r) => {
                let mut p = Vec::new();
                if !r.enumeration.is_empty() {
                    p.push(format!("one of [{}]", r.enumeration.join(", ")));
                }
                for pat in &r.patterns {
                    p.push(format!("pattern /{pat}/"));
                }
                let b = [
                    (r.min_inclusive, ">="),
                    (r.min_exclusive, ">"),
                    (r.max_inclusive, "<="),
                    (r.max_exclusive, "<"),
                ];
                for (v, op) in b {
                    if let Some(v) = v {
                        p.push(format!("{op} {v}"));
                    }
                }
                for (v, what) in [
                    (r.length, "length ="),
                    (r.min_length, "length >="),
                    (r.max_length, "length <="),
                ] {
                    if let Some(v) = v {
                        p.push(format!("{what} {v}"));
                    }
                }
                p.join(" and ")
            }
        }
    }
}

impl Restriction {
    fn matches(&self, a: &Actual) -> Result<bool, String> {
        if !self.enumeration.is_empty() && !self.enumeration.iter().any(|e| simple_matches(e, a)) {
            return Ok(false);
        }
        if !self.patterns.is_empty() {
            let Actual::Str(s) = a else { return Ok(false) };
            let mut any = false;
            for p in &self.patterns {
                let re = XsdRegex::new(p).map_err(|e| format!("pattern /{p}/ not checked: {e}"))?;
                any |= re.is_match(s);
            }
            if !any {
                return Ok(false);
            }
        }
        let bounds = [
            self.min_inclusive,
            self.max_inclusive,
            self.min_exclusive,
            self.max_exclusive,
        ];
        if bounds.iter().any(Option::is_some) {
            let Actual::Num(x) = *a else { return Ok(false) };
            let ok = self.min_inclusive.is_none_or(|m| x >= m || close(x, m))
                && self.max_inclusive.is_none_or(|m| x <= m || close(x, m))
                && self.min_exclusive.is_none_or(|m| x > m && !close(x, m))
                && self.max_exclusive.is_none_or(|m| x < m && !close(x, m));
            if !ok {
                return Ok(false);
            }
        }
        if self.length.is_some() || self.min_length.is_some() || self.max_length.is_some() {
            let Actual::Str(s) = a else { return Ok(false) };
            let n = s.chars().count();
            if self.length.is_some_and(|l| n != l)
                || self.min_length.is_some_and(|l| n < l)
                || self.max_length.is_some_and(|l| n > l)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

// --------------------------------------------------------------- XSD regex

/// A regular expression in the XML Schema dialect (implicitly anchored).
#[derive(Debug, Clone)]
pub struct XsdRegex {
    alt: Vec<Vec<Piece>>,
}

#[derive(Debug, Clone)]
struct Piece {
    atom: Atom,
    min: usize,
    max: Option<usize>,
}

#[derive(Debug, Clone)]
enum Atom {
    Char(char),
    Any,
    Class { neg: bool, items: Vec<ClassItem> },
    Group(Vec<Vec<Piece>>),
}

#[derive(Debug, Clone)]
enum ClassItem {
    Range(char, char),
    /// `d`, `D`, `w`, `W`, `s`, `S`, `i`, `I`, `c`, `C`.
    Esc(char),
}

fn esc_matches(e: char, c: char) -> bool {
    let base = match e.to_ascii_lowercase() {
        'd' => c.is_ascii_digit() || (!c.is_ascii() && c.is_numeric()),
        's' => matches!(c, ' ' | '\t' | '\n' | '\r'),
        // \w = not punctuation, separator or other (XSD F.1.1); ASCII symbols count as word characters.
        'w' => {
            if c.is_ascii() {
                c.is_ascii_alphanumeric() || "$+<=>^`|~".contains(c)
            } else {
                c.is_alphanumeric()
            }
        }
        'i' => c.is_alphabetic() || c == '_' || c == ':',
        'c' => c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '-' | '\u{B7}'),
        _ => false,
    };
    base != e.is_ascii_uppercase()
}

struct ReParser {
    c: Vec<char>,
    i: usize,
}

impl ReParser {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }
    fn alt(&mut self, depth: usize) -> Result<Vec<Vec<Piece>>, String> {
        if depth > 50 {
            return Err("pattern nested too deeply".into());
        }
        let mut alt = vec![Vec::new()];
        while let Some(ch) = self.peek() {
            match ch {
                '|' => {
                    self.i += 1;
                    alt.push(Vec::new());
                }
                ')' => break,
                _ => {
                    let atom = self.atom(depth)?;
                    let (min, max) = self.quantifier()?;
                    if let Some(b) = alt.last_mut() {
                        b.push(Piece { atom, min, max });
                    }
                }
            }
        }
        Ok(alt)
    }
    fn atom(&mut self, depth: usize) -> Result<Atom, String> {
        let ch = self.peek().ok_or("unexpected end")?;
        self.i += 1;
        Ok(match ch {
            '(' => {
                if self.peek() == Some('?') {
                    return Err("'(?' is not XSD syntax".into());
                }
                let g = self.alt(depth + 1)?;
                if self.peek() != Some(')') {
                    return Err("missing ')'".into());
                }
                self.i += 1;
                Atom::Group(g)
            }
            '[' => self.class()?,
            '.' => Atom::Any,
            '\\' => match self.escape()? {
                ClassItem::Esc(e) => Atom::Class {
                    neg: false,
                    items: vec![ClassItem::Esc(e)],
                },
                ClassItem::Range(c, _) => Atom::Char(c),
            },
            '*' | '+' | '?' | '{' => return Err(format!("nothing to repeat before '{ch}'")),
            c => Atom::Char(c),
        })
    }
    fn escape(&mut self) -> Result<ClassItem, String> {
        let e = self.peek().ok_or("trailing backslash")?;
        self.i += 1;
        Ok(match e {
            'n' => ClassItem::Range('\n', '\n'),
            'r' => ClassItem::Range('\r', '\r'),
            't' => ClassItem::Range('\t', '\t'),
            'd' | 'D' | 'w' | 'W' | 's' | 'S' | 'i' | 'I' | 'c' | 'C' => ClassItem::Esc(e),
            'p' | 'P' => return Err("Unicode categories \\p{..} are not supported".into()),
            c if "\\|.-^?*+{}()[]$/".contains(c) => ClassItem::Range(c, c),
            c => return Err(format!("unknown escape \\{c}")),
        })
    }
    fn class(&mut self) -> Result<Atom, String> {
        let mut neg = false;
        if self.peek() == Some('^') {
            neg = true;
            self.i += 1;
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let ch = self.peek().ok_or("missing ']'")?;
            if ch == ']' && !first {
                self.i += 1;
                break;
            }
            first = false;
            self.i += 1;
            if ch == '-' && self.peek() == Some('[') {
                return Err("character class subtraction is not supported".into());
            }
            let lo = if ch == '\\' {
                match self.escape()? {
                    ClassItem::Esc(e) => {
                        items.push(ClassItem::Esc(e));
                        continue;
                    }
                    ClassItem::Range(c, _) => c,
                }
            } else {
                ch
            };
            if self.peek() == Some('-')
                && !matches!(self.c.get(self.i + 1), Some(']') | Some('[') | None)
            {
                self.i += 1;
                let h = self.peek().ok_or("missing ']'")?;
                self.i += 1;
                let hi = if h == '\\' {
                    match self.escape()? {
                        ClassItem::Range(c, _) => c,
                        ClassItem::Esc(_) => return Err("class escape as range end".into()),
                    }
                } else {
                    h
                };
                if hi < lo {
                    return Err(format!("bad range {lo}-{hi}"));
                }
                items.push(ClassItem::Range(lo, hi));
            } else {
                items.push(ClassItem::Range(lo, lo));
            }
        }
        Ok(Atom::Class { neg, items })
    }
    fn number(&mut self) -> Option<usize> {
        let s = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.c[s..self.i].iter().collect::<String>().parse().ok()
    }
    fn quantifier(&mut self) -> Result<(usize, Option<usize>), String> {
        let q = match self.peek() {
            Some('?') => (0, Some(1)),
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('{') => {
                self.i += 1;
                let n = self.number().ok_or("bad {n,m}")?;
                let m = if self.peek() == Some(',') {
                    self.i += 1;
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.number().ok_or("bad {n,m}")?)
                    }
                } else {
                    Some(n)
                };
                if self.peek() != Some('}') {
                    return Err("missing '}'".into());
                }
                if m.is_some_and(|m| m < n) {
                    return Err("{n,m} with m < n".into());
                }
                (n, m)
            }
            _ => return Ok((1, Some(1))),
        };
        self.i += 1;
        Ok(q)
    }
}

impl XsdRegex {
    /// Compiles a pattern.
    pub fn new(pattern: &str) -> Result<XsdRegex, String> {
        let mut p = ReParser {
            c: pattern.chars().collect(),
            i: 0,
        };
        let alt = p.alt(0)?;
        if p.i != p.c.len() {
            return Err("unbalanced ')'".into());
        }
        Ok(XsdRegex { alt })
    }
    /// Whole-string match.
    pub fn is_match(&self, s: &str) -> bool {
        let c: Vec<char> = s.chars().collect();
        if c.len() > 4096 {
            return false;
        }
        m_alt(&self.alt, &c, 0, &|j| j == c.len())
    }
}

fn m_alt(alt: &[Vec<Piece>], s: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    alt.iter().any(|b| m_seq(b, s, i, k))
}

fn m_seq(b: &[Piece], s: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    match b.split_first() {
        None => k(i),
        Some((p, rest)) => m_rep(p, 0, s, i, &|j| m_seq(rest, s, j, k)),
    }
}

fn m_rep(p: &Piece, n: usize, s: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    if p.max.is_none_or(|m| n < m)
        && m_atom(&p.atom, s, i, &|j| {
            !(j == i && n >= p.min) && m_rep(p, n + 1, s, j, k)
        })
    {
        return true;
    }
    n >= p.min && k(i)
}

fn m_atom(a: &Atom, s: &[char], i: usize, k: &dyn Fn(usize) -> bool) -> bool {
    if let Atom::Group(g) = a {
        return m_alt(g, s, i, k);
    }
    let Some(&c) = s.get(i) else { return false };
    let ok = match a {
        Atom::Char(x) => c == *x,
        Atom::Any => c != '\n' && c != '\r',
        Atom::Class { neg, items } => {
            items.iter().any(|it| match it {
                ClassItem::Range(lo, hi) => (*lo..=*hi).contains(&c),
                ClassItem::Esc(e) => esc_matches(*e, c),
            }) != *neg
        }
        Atom::Group(_) => false,
    };
    ok && k(i + 1)
}

// ------------------------------------------------------------- IFC access

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    NotRooted,
    /// IfcRelationship, IfcPropertyDefinition and other rooted non-objects.
    RootOnly,
    Context,
    /// IfcObject that is not a product (group, system, process, resource…).
    Object,
    /// IfcProduct that is not an element or spatial element.
    Product,
    Spatial,
    SpatialStructure,
    Element,
    TypeObject,
    TypeProduct,
}

const SPATIAL: &[&str] = &[
    "IFCSITE",
    "IFCBUILDING",
    "IFCBUILDINGSTOREY",
    "IFCSPACE",
    "IFCFACILITY",
    "IFCBRIDGE",
    "IFCROAD",
    "IFCRAILWAY",
    "IFCMARINEFACILITY",
    "IFCTUNNEL",
    "IFCFACILITYPART",
    "IFCFACILITYPARTCOMMON",
    "IFCBRIDGEPART",
    "IFCROADPART",
    "IFCRAILWAYPART",
    "IFCMARINEFACILITYPART",
    "IFCTUNNELPART",
];
const NON_PRODUCT_OBJECTS: &[&str] = &[
    "IFCGROUP",
    "IFCZONE",
    "IFCINVENTORY",
    "IFCASSET",
    "IFCSTRUCTURALANALYSISMODEL",
    "IFCSTRUCTURALLOADGROUP",
    "IFCSTRUCTURALLOADCASE",
    "IFCSTRUCTURALRESULTGROUP",
    "IFCTASK",
    "IFCEVENT",
    "IFCPROCEDURE",
    "IFCWORKPLAN",
    "IFCWORKSCHEDULE",
    "IFCWORKCALENDAR",
    "IFCCOSTITEM",
    "IFCCOSTSCHEDULE",
    "IFCPERMIT",
    "IFCACTIONREQUEST",
    "IFCPROJECTORDER",
    "IFCPERFORMANCEHISTORY",
    "IFCACTOR",
    "IFCOCCUPANT",
    "IFCCONTROL",
    "IFCPROCESS",
    "IFCRESOURCE",
];
const NON_ELEMENT_PRODUCTS: &[&str] = &[
    "IFCGRID",
    "IFCANNOTATION",
    "IFCALIGNMENT",
    "IFCPORT",
    "IFCDISTRIBUTIONPORT",
    "IFCREFERENT",
    "IFCPROXY",
    "IFCLINEARPOSITIONINGELEMENT",
    "IFCPOSITIONINGELEMENT",
    "IFCSPATIALZONE",
    "IFCEXTERNALSPATIALELEMENT",
    "IFCSTRUCTURALCURVEMEMBER",
    "IFCSTRUCTURALSURFACEMEMBER",
    "IFCSTRUCTURALPOINTCONNECTION",
    "IFCSTRUCTURALCURVECONNECTION",
    "IFCSTRUCTURALSURFACECONNECTION",
    "IFCSTRUCTURALPOINTACTION",
    "IFCSTRUCTURALCURVEACTION",
    "IFCSTRUCTURALSURFACEACTION",
    "IFCSTRUCTURALLINEARACTION",
    "IFCSTRUCTURALPLANARACTION",
    "IFCSTRUCTURALPOINTREACTION",
    "IFCSTRUCTURALCURVEREACTION",
    "IFCSTRUCTURALSURFACEREACTION",
    "IFCSTRUCTURALCURVEMEMBERVARYING",
    "IFCSTRUCTURALSURFACEMEMBERVARYING",
];

fn is_guid(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| {
        s.len() == 22
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
    })
}

fn classify(e: &Entity) -> Class {
    let n = e.name.as_str();
    let rooted = is_guid(e.arg(0))
        && e.arg(1)
            .is_some_and(|v| v.is_null() || v.as_ref().is_some())
        && e.args.len() >= 4;
    if !rooted {
        return Class::NotRooted;
    }
    if n.starts_with("IFCREL")
        || n.starts_with("IFCPROPERTY")
        || n.ends_with("PROPERTIES")
        || n == "IFCELEMENTQUANTITY"
        || n.ends_with("TEMPLATE")
    {
        return Class::RootOnly;
    }
    if n == "IFCPROJECT" || n == "IFCPROJECTLIBRARY" {
        return Class::Context;
    }
    if n.ends_with("TYPE") || n.ends_with("STYLE") {
        let process = ["TASKTYPE", "PROCEDURETYPE", "EVENTTYPE", "RESOURCETYPE"]
            .iter()
            .any(|s| n.ends_with(s));
        return if process {
            Class::TypeObject
        } else {
            Class::TypeProduct
        };
    }
    if SPATIAL.contains(&n) {
        return Class::SpatialStructure;
    }
    if n == "IFCSPATIALZONE" || n == "IFCEXTERNALSPATIALELEMENT" {
        return Class::Spatial;
    }
    if NON_ELEMENT_PRODUCTS.contains(&n) {
        return Class::Product;
    }
    if NON_PRODUCT_OBJECTS.contains(&n)
        || n.ends_with("SYSTEM")
        || n.ends_with("RESOURCE")
        || n.ends_with("CIRCUIT")
    {
        return Class::Object;
    }
    Class::Element
}

/// Position of PredefinedType, where known.
fn predefined_index(e: &Entity, class: Class) -> Option<usize> {
    let n = e.name.as_str();
    let table: &[(&str, usize)] = &[
        ("IFCELEMENTASSEMBLY", 9),
        ("IFCDOOR", 10),
        ("IFCWINDOW", 10),
        ("IFCREINFORCINGBAR", 12),
        ("IFCREINFORCINGMESH", 17),
        ("IFCTENDON", 9),
        ("IFCTENDONANCHOR", 9),
        ("IFCTENDONCONDUIT", 9),
        ("IFCSPACE", 9),
        ("IFCBRIDGE", 9),
        ("IFCROAD", 9),
        ("IFCRAILWAY", 9),
        ("IFCMARINEFACILITY", 9),
        ("IFCTUNNEL", 9),
        ("IFCBRIDGEPART", 10),
        ("IFCROADPART", 10),
        ("IFCRAILWAYPART", 10),
        ("IFCMARINEFACILITYPART", 10),
        ("IFCTUNNELPART", 10),
        ("IFCFACILITYPARTCOMMON", 10),
        ("IFCSPATIALZONE", 8),
        ("IFCEXTERNALSPATIALELEMENT", 8),
        ("IFCALIGNMENT", 7),
        ("IFCANNOTATION", 7),
        ("IFCREFERENT", 7),
        ("IFCGRID", 10),
        ("IFCDISTRIBUTIONPORT", 8),
        ("IFCDISTRIBUTIONSYSTEM", 6),
        ("IFCDISTRIBUTIONCIRCUIT", 6),
        ("IFCBUILDINGSYSTEM", 5),
        ("IFCBUILTSYSTEM", 5),
        ("IFCTASK", 12),
        ("IFCSTRUCTURALANALYSISMODEL", 5),
        ("IFCOPENINGELEMENT", 8),
    ];
    if let Some(&(_, i)) = table.iter().find(|(t, _)| *t == n) {
        return Some(i);
    }
    match class {
        Class::Element => Some(8),
        Class::TypeProduct | Class::TypeObject => Some(9),
        _ => None,
    }
}

/// Position of a named attribute for this instance.
fn attr_index(e: &Entity, class: Class, name: &str) -> Option<usize> {
    use Class::*;
    let rooted = class != NotRooted;
    let object = matches!(
        class,
        Context | Object | Product | Spatial | SpatialStructure | Element
    );
    let product = matches!(class, Product | Spatial | SpatialStructure | Element);
    let typ = matches!(class, TypeObject | TypeProduct);
    let i = match name {
        "GlobalId" if rooted => 0,
        "OwnerHistory" if rooted => 1,
        "Name" if rooted => 2,
        "Description" if rooted => 3,
        "ObjectType" if object => 4,
        "ObjectPlacement" if product => 5,
        "Representation" if product => 6,
        "Tag" if class == Element || class == TypeProduct => 7,
        "LongName" if matches!(class, Spatial | SpatialStructure) => 7,
        "LongName" if class == Context => 5,
        "Phase" if class == Context => 6,
        "CompositionType" if class == SpatialStructure => 8,
        "ApplicableOccurrence" if typ => 4,
        "HasPropertySets" if typ => 5,
        "RepresentationMaps" if class == TypeProduct => 6,
        "ElementType" if class == TypeProduct => 8,
        "PredefinedType" => return predefined_index(e, class),
        _ if !rooted => {
            let table: &[(&str, &str, usize)] = &[
                ("IFCMATERIAL", "Name", 0),
                ("IFCMATERIAL", "Description", 1),
                ("IFCMATERIAL", "Category", 2),
                ("IFCMATERIALLAYERSET", "LayerSetName", 1),
                ("IFCMATERIALLAYERSET", "Description", 2),
                ("IFCMATERIALLAYER", "LayerThickness", 1),
                ("IFCMATERIALLAYER", "Name", 3),
                ("IFCMATERIALLAYER", "Category", 5),
                ("IFCMATERIALPROFILESET", "Name", 0),
                ("IFCMATERIALCONSTITUENTSET", "Name", 0),
                ("IFCCLASSIFICATION", "Source", 0),
                ("IFCCLASSIFICATION", "Edition", 1),
                ("IFCCLASSIFICATION", "Name", 3),
                ("IFCCLASSIFICATIONREFERENCE", "Location", 0),
                ("IFCCLASSIFICATIONREFERENCE", "Identification", 1),
                ("IFCCLASSIFICATIONREFERENCE", "Name", 2),
                ("IFCSURFACESTYLEREFRACTION", "RefractionIndex", 0),
                ("IFCSURFACESTYLEREFRACTION", "DispersionFactor", 1),
            ];
            return table
                .iter()
                .find(|(t, a, _)| *t == e.name && *a == name)
                .map(|t| t.2);
        }
        _ => return None,
    };
    Some(i)
}

/// Attribute names offered for pattern matching on names.
const KNOWN_ATTRIBUTES: &[&str] = &[
    "GlobalId",
    "OwnerHistory",
    "Name",
    "Description",
    "ObjectType",
    "ObjectPlacement",
    "Representation",
    "Tag",
    "LongName",
    "Phase",
    "CompositionType",
    "ApplicableOccurrence",
    "HasPropertySets",
    "RepresentationMaps",
    "ElementType",
    "PredefinedType",
    "Category",
    "LayerSetName",
    "LayerThickness",
    "Source",
    "Edition",
    "Location",
    "Identification",
    "RefractionIndex",
    "DispersionFactor",
];

#[derive(Debug, Clone)]
enum PropVals {
    /// (IFC type name, value) pairs.
    Values(Vec<(Option<String>, Actual)>),
    Empty,
    /// Reference or complex property: not checkable.
    Object,
}

#[derive(Debug, Clone)]
struct Prop {
    name: String,
    vals: PropVals,
}

type Pset = (String, Vec<Prop>);

/// Relationship indexes over a model.
struct Ctx<'a> {
    m: &'a StepModel,
    class: HashMap<u64, Class>,
    type_of: HashMap<u64, u64>,
    defs: HashMap<u64, Vec<u64>>,
    classes: HashMap<u64, Vec<u64>>,
    mats: HashMap<u64, Vec<u64>>,
    agg_parent: HashMap<u64, u64>,
    nest_parent: HashMap<u64, u64>,
    container: HashMap<u64, u64>,
    groups: HashMap<u64, Vec<u64>>,
    fill_opening: HashMap<u64, u64>,
    void_host: HashMap<u64, u64>,
    /// Scale to SI for length, area and volume measures.
    scale: [f64; 3],
}

fn si_prefix(p: Option<&str>) -> f64 {
    match p {
        Some("EXA") => 1e18,
        Some("PETA") => 1e15,
        Some("TERA") => 1e12,
        Some("GIGA") => 1e9,
        Some("MEGA") => 1e6,
        Some("KILO") => 1e3,
        Some("HECTO") => 1e2,
        Some("DECA") => 1e1,
        Some("DECI") => 1e-1,
        Some("CENTI") => 1e-2,
        Some("MILLI") => 1e-3,
        Some("MICRO") => 1e-6,
        Some("NANO") => 1e-9,
        Some("PICO") => 1e-12,
        Some("FEMTO") => 1e-15,
        Some("ATTO") => 1e-18,
        _ => 1.0,
    }
}

impl<'a> Ctx<'a> {
    fn new(m: &'a StepModel) -> Ctx<'a> {
        let mut c = Ctx {
            m,
            class: m.entities().iter().map(|e| (e.id, classify(e))).collect(),
            type_of: HashMap::new(),
            defs: HashMap::new(),
            classes: HashMap::new(),
            mats: HashMap::new(),
            agg_parent: HashMap::new(),
            nest_parent: HashMap::new(),
            container: HashMap::new(),
            groups: HashMap::new(),
            fill_opening: HashMap::new(),
            void_host: HashMap::new(),
            scale: [1.0; 3],
        };
        let many = |name: &str, objs: usize, target: usize| -> Vec<(u64, Vec<u64>)> {
            m.by_type(name)
                .iter()
                .map(|r| (r.ref_at(target).unwrap_or(0), r.refs_at(objs)))
                .collect()
        };
        for (t, objs) in many("IFCRELDEFINESBYTYPE", 4, 5) {
            objs.into_iter().for_each(|o| {
                c.type_of.insert(o, t);
            });
        }
        for r in m.by_type("IFCRELDEFINESBYPROPERTIES") {
            for o in r.refs_at(4) {
                c.defs.entry(o).or_default().extend(r.refs_at(5));
            }
        }
        for (t, objs) in many("IFCRELASSOCIATESCLASSIFICATION", 4, 5) {
            objs.into_iter()
                .for_each(|o| c.classes.entry(o).or_default().push(t));
        }
        for (t, objs) in many("IFCEXTERNALREFERENCERELATIONSHIP", 3, 2) {
            objs.into_iter()
                .for_each(|o| c.classes.entry(o).or_default().push(t));
        }
        for (t, objs) in many("IFCRELASSOCIATESMATERIAL", 4, 5) {
            objs.into_iter()
                .for_each(|o| c.mats.entry(o).or_default().push(t));
        }
        for (p, objs) in many("IFCRELAGGREGATES", 5, 4) {
            objs.into_iter().for_each(|o| {
                c.agg_parent.insert(o, p);
            });
        }
        for (p, objs) in many("IFCRELNESTS", 5, 4) {
            objs.into_iter().for_each(|o| {
                c.nest_parent.insert(o, p);
            });
        }
        for (p, objs) in many("IFCRELCONTAINEDINSPATIALSTRUCTURE", 4, 5) {
            objs.into_iter().for_each(|o| {
                c.container.insert(o, p);
            });
        }
        for name in ["IFCRELASSIGNSTOGROUP", "IFCRELASSIGNSTOGROUPBYFACTOR"] {
            for (g, objs) in many(name, 4, 6) {
                objs.into_iter()
                    .for_each(|o| c.groups.entry(o).or_default().push(g));
            }
        }
        for (op, objs) in many("IFCRELFILLSELEMENT", 5, 4) {
            objs.into_iter().for_each(|o| {
                c.fill_opening.insert(o, op);
            });
        }
        for (host, objs) in many("IFCRELVOIDSELEMENT", 5, 4) {
            objs.into_iter().for_each(|o| {
                c.void_host.insert(o, host);
            });
        }
        // Units: IfcProject.UnitsInContext → IfcUnitAssignment.Units.
        let units = m
            .by_type("IFCPROJECT")
            .first()
            .and_then(|p| p.ref_at(8))
            .and_then(|u| m.get(u))
            .map(|u| u.refs_at(0))
            .unwrap_or_default();
        for u in units.iter().filter_map(|&u| m.get(u)) {
            let k = match u.enum_at(1) {
                Some("LENGTHUNIT") => 0,
                Some("AREAUNIT") => 1,
                Some("VOLUMEUNIT") => 2,
                _ => continue,
            };
            if let Some(s) = c.unit_scale(u, k as i32 + 1, 0) {
                c.scale[k] = s;
            }
        }
        c
    }

    /// SI factor of a unit with dimension power `dim` (1 length, 2 area, 3 volume).
    fn unit_scale(&self, u: &Entity, dim: i32, depth: usize) -> Option<f64> {
        if depth > 4 {
            return None;
        }
        match u.name.as_str() {
            "IFCSIUNIT" => Some(si_prefix(u.enum_at(2)).powi(dim)),
            "IFCCONVERSIONBASEDUNIT" | "IFCCONVERSIONBASEDUNITWITHOFFSET" => {
                let mw = self.m.get(u.ref_at(3)?)?;
                let v = mw.f64_at(0)?;
                let base = self.m.get(mw.ref_at(1)?)?;
                Some(v * self.unit_scale(base, dim, depth + 1)?)
            }
            _ => None,
        }
    }

    fn class_of(&self, id: u64) -> Class {
        self.class.get(&id).copied().unwrap_or(Class::NotRooted)
    }

    /// Resolved PredefinedType (USERDEFINED → ObjectType/ElementType), from
    /// the occurrence or else its type.
    fn predefined_type(&self, e: &Entity) -> Option<String> {
        let own = self.own_predefined(e);
        if own.is_some() {
            return own;
        }
        let t = self.m.get(*self.type_of.get(&e.id)?)?;
        self.own_predefined(t)
    }

    fn own_predefined(&self, e: &Entity) -> Option<String> {
        let class = self.class_of(e.id);
        let i = predefined_index(e, class)?;
        match e.enum_at(i)? {
            "NOTDEFINED" => None,
            "USERDEFINED" => {
                // ObjectType of occurrences; ElementType/ProcessType/ResourceType of types.
                let k = if matches!(class, Class::TypeProduct | Class::TypeObject) {
                    8
                } else {
                    4
                };
                e.str_at(k).filter(|s| !s.is_empty()).map(String::from)
            }
            v => Some(v.to_string()),
        }
    }

    fn prop_value(&self, v: &Value, forced_type: Option<&str>) -> Option<(Option<String>, Actual)> {
        let t = forced_type
            .map(String::from)
            .or_else(|| v.type_name().map(String::from));
        let mut a = actual(v)?;
        if let (Actual::Num(x), Some(t)) = (&mut a, t.as_deref()) {
            let k = match t {
                "IFCLENGTHMEASURE" | "IFCPOSITIVELENGTHMEASURE" | "IFCNONNEGATIVELENGTHMEASURE" => {
                    Some(0)
                }
                "IFCAREAMEASURE" => Some(1),
                "IFCVOLUMEMEASURE" => Some(2),
                _ => None,
            };
            if let Some(k) = k {
                *x *= self.scale[k];
            }
        }
        Some((t, a))
    }

    fn prop(&self, p: &Entity) -> Option<Prop> {
        let name = p.str_at(0)?.to_string();
        let vals_of = |vs: &[&Value], forced: Option<&str>| {
            let v: Vec<_> = vs
                .iter()
                .filter_map(|v| self.prop_value(v, forced))
                .collect();
            if v.is_empty() {
                PropVals::Empty
            } else {
                PropVals::Values(v)
            }
        };
        let list = |i: usize| -> Vec<&Value> {
            p.list_at(i).map(|l| l.iter().collect()).unwrap_or_default()
        };
        let measure = match p.name.as_str() {
            "IFCQUANTITYLENGTH" => Some("IFCLENGTHMEASURE"),
            "IFCQUANTITYAREA" => Some("IFCAREAMEASURE"),
            "IFCQUANTITYVOLUME" => Some("IFCVOLUMEMEASURE"),
            "IFCQUANTITYCOUNT" => Some("IFCCOUNTMEASURE"),
            "IFCQUANTITYWEIGHT" => Some("IFCMASSMEASURE"),
            "IFCQUANTITYTIME" => Some("IFCTIMEMEASURE"),
            "IFCQUANTITYNUMBER" => Some("IFCNUMERICMEASURE"),
            _ => None,
        };
        let vals = match p.name.as_str() {
            "IFCPROPERTYSINGLEVALUE" => vals_of(&p.arg(2).into_iter().collect::<Vec<_>>(), None),
            "IFCPROPERTYENUMERATEDVALUE" | "IFCPROPERTYLISTVALUE" => vals_of(&list(2), None),
            "IFCPROPERTYBOUNDEDVALUE" => vals_of(
                &[2, 3, 5]
                    .iter()
                    .filter_map(|&i| p.arg(i))
                    .collect::<Vec<_>>(),
                None,
            ),
            "IFCPROPERTYTABLEVALUE" => {
                let mut v = list(2);
                v.extend(list(3));
                vals_of(&v, None)
            }
            _ if measure.is_some() => vals_of(&p.arg(3).into_iter().collect::<Vec<_>>(), measure),
            _ => PropVals::Object,
        };
        Some(Prop { name, vals })
    }

    fn pset(&self, id: u64) -> Option<Pset> {
        let e = self.m.get(id)?;
        let items = match e.name.as_str() {
            "IFCPROPERTYSET" => e.refs_at(4),
            "IFCELEMENTQUANTITY" => e.refs_at(5),
            _ => return None,
        };
        let name = e.str_at(2)?.to_string();
        Some((
            name,
            items
                .iter()
                .filter_map(|&i| self.m.get(i))
                .filter_map(|p| self.prop(p))
                .collect(),
        ))
    }

    /// Property sets of an element, type sets overridden per property by the
    /// occurrence's.
    fn psets(&self, e: &Entity) -> Vec<Pset> {
        let mut out: Vec<Pset> = Vec::new();
        let mut merge = |sets: Vec<Pset>| {
            for (name, props) in sets {
                match out.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, have)) => {
                        for p in props {
                            match have.iter_mut().find(|h| h.name == p.name) {
                                Some(h) => *h = p,
                                None => have.push(p),
                            }
                        }
                    }
                    None => out.push((name, props)),
                }
            }
        };
        let type_sets = |t: &Entity| -> Vec<Pset> {
            let mut ids = t.refs_at(5);
            ids.extend(self.defs.get(&t.id).cloned().unwrap_or_default());
            ids.iter().filter_map(|&i| self.pset(i)).collect()
        };
        if matches!(self.class_of(e.id), Class::TypeObject | Class::TypeProduct) {
            merge(type_sets(e));
            return out;
        }
        if let Some(t) = self.type_of.get(&e.id).and_then(|&t| self.m.get(t)) {
            merge(type_sets(t));
        }
        merge(
            self.defs
                .get(&e.id)
                .map(|d| d.iter().filter_map(|&i| self.pset(i)).collect())
                .unwrap_or_default(),
        );
        out
    }

    /// (system, identifications along the reference chain) pairs.
    fn classifications(&self, e: &Entity) -> Vec<(Option<String>, Vec<String>)> {
        let of = |id: u64| -> Vec<(Option<String>, Vec<String>)> {
            self.classes
                .get(&id)
                .map(|v| v.iter().filter_map(|&c| self.classification(c)).collect())
                .unwrap_or_default()
        };
        let mut out = of(e.id);
        if let Some(&t) = self.type_of.get(&e.id) {
            for (sys, ids) in of(t) {
                if !out.iter().any(|(s, _)| *s == sys) {
                    out.push((sys, ids));
                }
            }
        }
        out
    }

    fn classification(&self, id: u64) -> Option<(Option<String>, Vec<String>)> {
        let mut ids = Vec::new();
        let mut cur = self.m.get(id)?;
        for _ in 0..32 {
            match cur.name.as_str() {
                "IFCCLASSIFICATION" => return Some((cur.str_at(3).map(String::from), ids)),
                "IFCCLASSIFICATIONREFERENCE" => {
                    if let Some(i) = cur.str_at(1) {
                        ids.push(i.to_string());
                    }
                    match cur.ref_at(3).and_then(|r| self.m.get(r)) {
                        Some(next) => cur = next,
                        None => return Some((None, ids)),
                    }
                }
                _ => return None,
            }
        }
        None
    }

    /// Names and categories of the element's materials (else its type's);
    /// `None` without a material association.
    fn materials(&self, e: &Entity) -> Option<Vec<String>> {
        let rels = self
            .mats
            .get(&e.id)
            .or_else(|| self.type_of.get(&e.id).and_then(|t| self.mats.get(t)))?;
        let mut out = Vec::new();
        for &r in rels {
            self.material_names(r, &mut out, 0);
        }
        Some(out)
    }

    fn material_names(&self, id: u64, out: &mut Vec<String>, depth: usize) {
        let Some(e) = self.m.get(id) else { return };
        if depth > 8 {
            return;
        }
        let mut push = |i: usize| {
            if let Some(s) = e.str_at(i).filter(|s| !s.is_empty()) {
                out.push(s.to_string());
            }
        };
        let (names, children): (&[usize], Vec<u64>) = match e.name.as_str() {
            "IFCMATERIAL" => (&[0, 2], vec![]),
            "IFCMATERIALLAYERSET" => (&[1], e.refs_at(0)),
            "IFCMATERIALLAYER" | "IFCMATERIALLAYERWITHOFFSETS" => {
                (&[3, 5], e.ref_at(0).into_iter().collect())
            }
            "IFCMATERIALPROFILESET" => (&[0], e.refs_at(2)),
            "IFCMATERIALPROFILE" | "IFCMATERIALPROFILEWITHOFFSETS" => {
                (&[0, 5], e.ref_at(2).into_iter().collect())
            }
            "IFCMATERIALCONSTITUENTSET" => (&[0], e.refs_at(2)),
            "IFCMATERIALCONSTITUENT" => (&[0, 4], e.ref_at(2).into_iter().collect()),
            "IFCMATERIALLIST" => (&[], e.refs_at(0)),
            "IFCMATERIALLAYERSETUSAGE"
            | "IFCMATERIALPROFILESETUSAGE"
            | "IFCMATERIALPROFILESETUSAGETAPERING" => (&[], e.ref_at(0).into_iter().collect()),
            _ => (&[], vec![]),
        };
        names.iter().for_each(|&i| push(i));
        for c in children {
            self.material_names(c, out, depth + 1);
        }
    }

    /// Wholes the element is part of through `relation` (`None` = any).
    fn wholes(&self, id: u64, relation: Option<&str>) -> Vec<u64> {
        let chain = |map: &HashMap<u64, u64>| {
            let mut v = Vec::new();
            let mut cur = id;
            while let Some(&p) = map.get(&cur) {
                if v.contains(&p) || v.len() > 64 {
                    break;
                }
                v.push(p);
                cur = p;
            }
            v
        };
        let contained = || {
            let mut v = Vec::new();
            let mut cur = id;
            let mut seen = HashSet::new();
            while seen.insert(cur) {
                if let Some(&c) = self.container.get(&cur) {
                    v.push(c);
                    cur = c;
                } else if let Some(&p) = self.agg_parent.get(&cur) {
                    if v.is_empty() {
                        cur = p; // a part is contained where its whole is
                    } else {
                        v.push(p);
                        cur = p;
                    }
                } else {
                    break;
                }
            }
            v
        };
        let host = || {
            self.fill_opening
                .get(&id)
                .and_then(|o| self.void_host.get(o))
                .copied()
                .into_iter()
                .collect::<Vec<_>>()
        };
        let groups = || self.groups.get(&id).cloned().unwrap_or_default();
        match relation {
            Some("IFCRELAGGREGATES") => chain(&self.agg_parent),
            Some("IFCRELNESTS") => chain(&self.nest_parent),
            Some("IFCRELCONTAINEDINSPATIALSTRUCTURE") => contained(),
            Some("IFCRELASSIGNSTOGROUP") => groups(),
            Some(r) if r.contains("IFCRELVOIDSELEMENT") || r.contains("IFCRELFILLSELEMENT") => {
                host()
            }
            Some(_) => Vec::new(),
            None => {
                let mut v = chain(&self.agg_parent);
                v.extend(chain(&self.nest_parent));
                v.extend(contained());
                v.extend(groups());
                v.extend(host());
                v.sort_unstable();
                v.dedup();
                v
            }
        }
    }
}

// ------------------------------------------------------------------- check

/// Outcome of one facet on one element, before cardinality.
enum Outcome {
    Pass,
    Fail(String),
    Absent(String),
}

fn value_check(v: &Option<IdsValue>, a: &Actual, what: &str) -> Outcome {
    let Some(v) = v else { return Outcome::Pass };
    match v.matches(a) {
        Ok(true) => Outcome::Pass,
        Ok(false) => Outcome::Fail(format!("{what} is {}, required {}", a.show(), v.describe())),
        Err(e) => Outcome::Fail(format!("{what}: {e}")),
    }
}

fn entity_check(ctx: &Ctx, e: &Entity, name: &IdsValue, pt: &Option<IdsValue>) -> Outcome {
    match name.matches_str(&e.name) {
        Ok(true) => {}
        Ok(false) => return Outcome::Fail(format!("entity {} is not {}", e.name, name.describe())),
        Err(err) => return Outcome::Fail(format!("entity: {err}")),
    }
    let Some(pt) = pt else { return Outcome::Pass };
    match ctx.predefined_type(e) {
        None => Outcome::Fail(format!(
            "{} has no PredefinedType, required {}",
            e.name,
            pt.describe()
        )),
        Some(p) => value_check(&Some(pt.clone()), &Actual::Str(p), "PredefinedType"),
    }
}

fn facet_outcome(ctx: &Ctx, e: &Entity, f: &FacetKind) -> Outcome {
    match f {
        FacetKind::Entity {
            name,
            predefined_type,
        } => entity_check(ctx, e, name, predefined_type),
        FacetKind::Attribute { name, value } => {
            let class = ctx.class_of(e.id);
            let names: Vec<&str> = match name {
                IdsValue::Simple(s) => vec![s.as_str()],
                r => KNOWN_ATTRIBUTES
                    .iter()
                    .copied()
                    .filter(|n| r.matches_str(n).unwrap_or(false))
                    .collect(),
            };
            let mut present = 0;
            for n in &names {
                let Some(i) = attr_index(e, class, n) else {
                    continue;
                };
                let raw = e.arg(i).unwrap_or(&Value::Null);
                let Some(a) = actual(raw) else { continue };
                present += 1;
                if a.is_empty() || raw.as_list().is_some_and(|l| l.is_empty()) {
                    return Outcome::Fail(format!("attribute {n} is empty"));
                }
                if value.is_some() && a == Actual::Object {
                    return Outcome::Fail(format!(
                        "attribute {n} is an object or list; its value cannot be compared"
                    ));
                }
                if let o @ Outcome::Fail(_) = value_check(value, &a, &format!("attribute {n}")) {
                    return o;
                }
            }
            if present == 0 {
                Outcome::Absent(format!(
                    "attribute {} has no value on {}",
                    name.describe(),
                    e.name
                ))
            } else {
                Outcome::Pass
            }
        }
        FacetKind::Property {
            property_set,
            base_name,
            value,
            data_type,
        } => {
            let psets = ctx.psets(e);
            let matched: Vec<&Pset> = psets
                .iter()
                .filter(|(n, _)| property_set.matches_str(n).unwrap_or(false))
                .collect();
            if matched.is_empty() {
                return Outcome::Absent(format!("no property set {}", property_set.describe()));
            }
            let mut found = 0;
            let mut fails = Vec::new();
            for (pn, props) in &matched {
                let hits: Vec<&Prop> = props
                    .iter()
                    .filter(|p| base_name.matches_str(&p.name).unwrap_or(false))
                    .collect();
                if hits.is_empty() {
                    fails.push(format!("{pn} has no property {}", base_name.describe()));
                }
                for p in hits {
                    found += 1;
                    let what = format!("{pn}.{}", p.name);
                    match &p.vals {
                        PropVals::Empty => fails.push(format!("{what} has no value")),
                        PropVals::Object => fails.push(format!(
                            "{what} is a reference or complex property (not supported)"
                        )),
                        PropVals::Values(vs) => {
                            if let Some(dt) = data_type {
                                if let Some((t, _)) =
                                    vs.iter().find(|(t, _)| t.as_deref() != Some(dt.as_str()))
                                {
                                    fails.push(format!(
                                        "{what} has data type {}, required {dt}",
                                        t.as_deref().unwrap_or("none")
                                    ));
                                    continue;
                                }
                            }
                            if vs.iter().all(|(_, a)| a.is_empty()) {
                                fails.push(format!("{what} is empty or unknown"));
                                continue;
                            }
                            if let Some(v) = value {
                                let mut ok = false;
                                let mut err = None;
                                for (_, a) in vs.iter().filter(|(_, a)| !a.is_empty()) {
                                    match v.matches(a) {
                                        Ok(true) => ok = true,
                                        Ok(false) => {}
                                        Err(e) => err = Some(e),
                                    }
                                }
                                if !ok {
                                    let shown: Vec<String> =
                                        vs.iter().map(|(_, a)| a.show()).collect();
                                    fails.push(err.unwrap_or_else(|| {
                                        format!(
                                            "{what} is {}, required {}",
                                            shown.join(", "),
                                            v.describe()
                                        )
                                    }));
                                }
                            }
                        }
                    }
                }
            }
            if found == 0 {
                Outcome::Absent(format!(
                    "no property {} in {}",
                    base_name.describe(),
                    property_set.describe()
                ))
            } else if fails.is_empty() {
                Outcome::Pass
            } else {
                Outcome::Fail(fails.join("; "))
            }
        }
        FacetKind::Classification { system, value } => {
            let cs = ctx.classifications(e);
            if cs.is_empty() {
                return Outcome::Absent("no classification".into());
            }
            let hit = cs.iter().any(|(sys, ids)| {
                let s_ok = match system {
                    None => true,
                    Some(s) => sys
                        .as_deref()
                        .is_some_and(|x| s.matches_str(x).unwrap_or(false)),
                };
                let v_ok = match value {
                    None => true,
                    Some(v) => ids.iter().any(|i| v.matches_str(i).unwrap_or(false)),
                };
                s_ok && v_ok
            });
            if hit {
                Outcome::Pass
            } else {
                let shown: Vec<String> = cs
                    .iter()
                    .map(|(s, ids)| {
                        format!(
                            "{} {}",
                            s.as_deref().unwrap_or("?"),
                            ids.first().map(String::as_str).unwrap_or("")
                        )
                    })
                    .collect();
                let want = format!(
                    "{}{}",
                    system
                        .as_ref()
                        .map(|s| format!("system {}", s.describe()))
                        .unwrap_or_default(),
                    value
                        .as_ref()
                        .map(|v| format!(" value {}", v.describe()))
                        .unwrap_or_default()
                );
                Outcome::Fail(format!(
                    "classification [{}] does not match {}",
                    shown.join(", ").trim(),
                    want.trim()
                ))
            }
        }
        FacetKind::Material { value } => match ctx.materials(e) {
            None => Outcome::Absent("no material".into()),
            Some(names) => match value {
                None => Outcome::Pass,
                Some(v) if names.iter().any(|n| v.matches_str(n).unwrap_or(false)) => Outcome::Pass,
                Some(v) => Outcome::Fail(format!(
                    "materials [{}] do not match {}",
                    names.join(", "),
                    v.describe()
                )),
            },
        },
        FacetKind::PartOf {
            relation,
            name,
            predefined_type,
        } => {
            let wholes = ctx.wholes(e.id, relation.as_deref());
            let rel = relation.as_deref().unwrap_or("any relation");
            if wholes.is_empty() {
                return Outcome::Absent(format!("not part of anything via {rel}"));
            }
            let ok = wholes
                .iter()
                .filter_map(|&w| ctx.m.get(w))
                .any(|w| matches!(entity_check(ctx, w, name, predefined_type), Outcome::Pass));
            if ok {
                Outcome::Pass
            } else {
                let shown: Vec<String> = wholes
                    .iter()
                    .filter_map(|&w| ctx.m.get(w))
                    .map(|w| format!("#{} {}", w.id, w.name))
                    .collect();
                Outcome::Fail(format!(
                    "part of [{}] via {rel}, none is {}",
                    shown.join(", "),
                    name.describe()
                ))
            }
        }
    }
}

fn apply_cardinality(c: Cardinality, o: Outcome) -> Result<(), String> {
    match (c, o) {
        (Cardinality::Required, Outcome::Pass) => Ok(()),
        (Cardinality::Required, Outcome::Fail(r) | Outcome::Absent(r)) => Err(r),
        (Cardinality::Optional, Outcome::Pass | Outcome::Absent(_)) => Ok(()),
        (Cardinality::Optional, Outcome::Fail(r)) => Err(r),
        (Cardinality::Prohibited, Outcome::Pass) => Err("prohibited, but present".into()),
        (Cardinality::Prohibited, _) => Ok(()),
    }
}

fn facet_label(f: &Facet) -> String {
    let k = match &f.kind {
        FacetKind::Entity { name, .. } => format!("entity {}", name.describe()),
        FacetKind::Attribute { name, .. } => format!("attribute {}", name.describe()),
        FacetKind::Property {
            property_set,
            base_name,
            ..
        } => format!(
            "property {}.{}",
            property_set.describe(),
            base_name.describe()
        ),
        FacetKind::Classification { system, .. } => format!(
            "classification {}",
            system.as_ref().map(IdsValue::describe).unwrap_or_default()
        ),
        FacetKind::Material { .. } => "material".into(),
        FacetKind::PartOf { relation, .. } => {
            format!("partOf {}", relation.as_deref().unwrap_or(""))
        }
    };
    k.trim().to_string()
}

// ------------------------------------------------------------------ report

/// Specification outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Pass,
    Fail,
}

/// One applicable element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementResult {
    pub id: u64,
    pub entity: String,
    pub global_id: Option<String>,
    pub name: Option<String>,
    pub pass: bool,
    /// Failure reasons, one per failing facet.
    pub reasons: Vec<String>,
}

/// One specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecResult {
    pub name: String,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub status: Status,
    pub applicable: usize,
    pub passed: usize,
    pub failed: usize,
    /// Occurrence and schema remarks.
    pub messages: Vec<String>,
    pub elements: Vec<ElementResult>,
}

/// Totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub specifications: usize,
    pub specifications_passed: usize,
    pub specifications_failed: usize,
    pub checks: usize,
    pub checks_failed: usize,
}

/// The whole check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdsReport {
    pub title: String,
    pub schema: Option<String>,
    pub specifications: Vec<SpecResult>,
    pub summary: Summary,
}

fn schema_version(s: &str) -> &'static str {
    let s = s.to_ascii_uppercase();
    if s.starts_with("IFC4X3") {
        "IFC4X3_ADD2"
    } else if s.starts_with("IFC4") {
        "IFC4"
    } else if s.starts_with("IFC2X3") {
        "IFC2X3"
    } else {
        "unknown"
    }
}

/// Checks `model` against `ids`.
pub fn check(ids: &Ids, model: &StepModel) -> IdsReport {
    let ctx = Ctx::new(model);
    let schema = model.schema();
    let mut specs = Vec::new();
    for s in &ids.specifications {
        let mut messages = Vec::new();
        if let Some(sc) = &schema {
            let v = schema_version(sc);
            if !s.ifc_versions.is_empty() && !s.ifc_versions.iter().any(|x| x == v) {
                messages.push(format!(
                    "written for {}, model is {sc}; checked anyway",
                    s.ifc_versions.join(" ")
                ));
            }
        }
        // Candidates: by class when the applicability names one, else all.
        let candidates: Vec<&Entity> = match s.applicability.iter().find_map(|f| match &f.kind {
            FacetKind::Entity {
                name: IdsValue::Simple(n),
                ..
            } => Some(n),
            _ => None,
        }) {
            Some(n) => model.by_type(n),
            None => model.entities().iter().collect(),
        };
        let applicable: Vec<&Entity> = candidates
            .into_iter()
            .filter(|e| {
                s.applicability
                    .iter()
                    .all(|f| matches!(facet_outcome(&ctx, e, &f.kind), Outcome::Pass))
            })
            .collect();
        let mut elements = Vec::new();
        if s.max_occurs != Some(0) {
            for e in &applicable {
                let reasons: Vec<String> = s
                    .requirements
                    .iter()
                    .filter_map(|f| {
                        apply_cardinality(f.cardinality, facet_outcome(&ctx, e, &f.kind))
                            .err()
                            .map(|r| format!("{}: {r}", facet_label(f)))
                    })
                    .collect();
                let rooted = ctx.class_of(e.id) != Class::NotRooted;
                elements.push(ElementResult {
                    id: e.id,
                    entity: e.name.clone(),
                    global_id: if rooted {
                        e.str_at(0).map(String::from)
                    } else {
                        None
                    },
                    name: if rooted {
                        e.str_at(2).map(String::from)
                    } else {
                        None
                    },
                    pass: reasons.is_empty(),
                    reasons,
                });
            }
        }
        let n = applicable.len();
        let passed = elements.iter().filter(|r| r.pass).count();
        let mut ok = passed == elements.len();
        if n < s.min_occurs as usize {
            ok = false;
            messages.push(format!(
                "{n} applicable elements, at least {} required",
                s.min_occurs
            ));
        }
        if let Some(max) = s.max_occurs {
            if n > max as usize {
                ok = false;
                messages.push(if max == 0 {
                    format!("{n} applicable elements, none allowed")
                } else {
                    format!("{n} applicable elements, at most {max} allowed")
                });
            }
        }
        specs.push(SpecResult {
            name: s.name.clone(),
            identifier: s.identifier.clone(),
            description: s.description.clone(),
            status: if ok { Status::Pass } else { Status::Fail },
            applicable: n,
            passed,
            failed: elements.len() - passed,
            messages,
            elements,
        });
    }
    let summary = Summary {
        specifications: specs.len(),
        specifications_passed: specs.iter().filter(|s| s.status == Status::Pass).count(),
        specifications_failed: specs.iter().filter(|s| s.status == Status::Fail).count(),
        checks: specs.iter().map(|s| s.elements.len()).sum(),
        checks_failed: specs.iter().map(|s| s.failed).sum(),
    };
    IdsReport {
        title: ids.title.clone(),
        schema,
        specifications: specs,
        summary,
    }
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

impl IdsReport {
    /// True when every specification passes.
    pub fn passed(&self) -> bool {
        self.summary.specifications_failed == 0
    }
    /// Pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    /// Markdown: a summary table, then the failing elements per specification.
    pub fn to_markdown(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(s, "# IDS check: {}\n", md_cell(&self.title));
        if let Some(sc) = &self.schema {
            let _ = writeln!(s, "Model schema: {sc}\n");
        }
        let su = &self.summary;
        let _ = writeln!(
            s,
            "{} of {} specifications pass; {} of {} element checks fail.\n",
            su.specifications_passed, su.specifications, su.checks_failed, su.checks
        );
        let _ = writeln!(s, "| Specification | Status | Applicable | Pass | Fail | Remarks |\n|---|---|---:|---:|---:|---|");
        for r in &self.specifications {
            let st = if r.status == Status::Pass {
                "pass"
            } else {
                "FAIL"
            };
            let _ = writeln!(
                s,
                "| {} | {st} | {} | {} | {} | {} |",
                md_cell(&r.name),
                r.applicable,
                r.passed,
                r.failed,
                md_cell(&r.messages.join("; "))
            );
        }
        for r in self.specifications.iter().filter(|r| r.failed > 0) {
            let _ = writeln!(
                s,
                "\n## {}\n\n| Element | GlobalId | Name | Reason |\n|---|---|---|---|",
                md_cell(&r.name)
            );
            for e in r.elements.iter().filter(|e| !e.pass) {
                let _ = writeln!(
                    s,
                    "| #{} {} | {} | {} | {} |",
                    e.id,
                    e.entity,
                    e.global_id.as_deref().unwrap_or(""),
                    md_cell(e.name.as_deref().unwrap_or("")),
                    md_cell(&e.reasons.join("; "))
                );
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> String {
        let p = format!("{}/tests/fixtures/ids/{rel}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"))
    }

    #[test]
    fn regex_subset() {
        let ok = |p: &str, s: &str| XsdRegex::new(p).unwrap().is_match(s);
        assert!(ok("[A-Z]{2}[0-9]{2}", "WT01"));
        assert!(!ok("[A-Z]{2}[0-9]{2}", "WT011"));
        assert!(!ok("[A-Z]{2}[0-9]{2}", "wt01"));
        assert!(ok(r"AC \d+ (surf|bin|base)", "AC 16 surf"));
        assert!(!ok(r"AC \d+ (surf|bin|base)", "AC 16 top"));
        assert!(ok(".*", ""));
        assert!(ok(r"\w+", "Foobar"));
        assert!(!ok(r"\w+", "Foo bar"));
        assert!(ok("[^0-9]+", "abc"));
        assert!(ok(r"[\-a]+\.x?", "-a-."));
        assert!(ok("(ab)*c{1,2}", "ababcc"));
        assert!(!ok("(ab)*c{1,2}", "abaccc"));
        assert!(ok("(a|)+b", "aab"));
        assert!(ok(r"\i\c*", "IfcWall_1"));
        assert!(XsdRegex::new(r"\p{Lu}").is_err());
        assert!(XsdRegex::new("[a-z-[aeiou]]").is_err());
        assert!(XsdRegex::new("a)").is_err());
    }

    #[test]
    fn values_and_restrictions() {
        let r = Restriction {
            min_inclusive: Some(0.04),
            max_exclusive: Some(1.0),
            ..Default::default()
        };
        assert!(r.matches(&Actual::Num(0.04)).unwrap());
        assert!(
            r.matches(&Actual::Num(0.0399999999)).unwrap(),
            "IDS tolerance"
        );
        assert!(!r.matches(&Actual::Num(1.0)).unwrap());
        assert!(
            !r.matches(&Actual::Str("0.5".into())).unwrap(),
            "bounds only on numbers"
        );
        let p = Restriction {
            patterns: vec!["[0-9]+".into()],
            ..Default::default()
        };
        assert!(
            !p.matches(&Actual::Num(12.0)).unwrap(),
            "patterns only on strings"
        );
        let l = Restriction {
            min_length: Some(2),
            max_length: Some(3),
            ..Default::default()
        };
        assert!(l.matches(&Actual::Str("ëë".into())).unwrap());
        assert!(!l.matches(&Actual::Str("abcd".into())).unwrap());
        let e = Restriction {
            enumeration: vec!["A".into(), "B".into()],
            ..Default::default()
        };
        assert!(e.matches(&Actual::Str("B".into())).unwrap());
        assert!(!e.matches(&Actual::Str("b".into())).unwrap());
        assert!(simple_matches("true", &Actual::Bool(true)));
        assert!(!simple_matches("TRUE", &Actual::Bool(true)));
        assert!(simple_matches("2", &Actual::Num(2.0000000001)));
    }

    #[test]
    fn parses_ids() {
        let ids = Ids::parse(&fixture("infra.ids")).unwrap();
        assert_eq!(ids.title, "Open Infra test: verharding");
        assert!(ids.specifications.len() >= 14);
        let s = &ids.specifications[0];
        assert_eq!(s.ifc_versions, vec!["IFC4X3_ADD2"]);
        assert!(
            matches!(&s.requirements[0].kind, FacetKind::Entity { predefined_type: Some(IdsValue::Simple(p)), .. } if p == "PAVEMENT")
        );
        assert!(Ids::parse("<ids/>").is_err());
    }

    #[test]
    fn infra_fixture_pass_and_fail_per_facet() {
        let ids = Ids::parse(&fixture("infra.ids")).unwrap();
        let model = StepModel::parse_str(&fixture("infra.ifc")).unwrap();
        let rep = ids.check(&model);
        let spec = |name: &str| {
            rep.specifications
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("no spec {name}"))
        };
        let elem = |name: &str, id: u64| {
            spec(name)
                .elements
                .iter()
                .find(|e| e.id == id)
                .unwrap_or_else(|| panic!("{name}: no #{id}"))
                .clone()
        };
        // (spec, pass?, element #20 passes?, element #21 passes?)
        let cases: &[(&str, bool, Option<bool>, Option<bool>)] = &[
            ("entity predefined type", false, Some(true), Some(false)),
            ("attribute Tag", false, Some(true), Some(false)),
            (
                "property thickness with units",
                false,
                Some(true),
                Some(false),
            ),
            ("property optional pattern", true, Some(true), Some(true)),
            ("property from type", false, Some(true), Some(false)),
            (
                "classification with parent value",
                false,
                Some(true),
                Some(false),
            ),
            ("material layer", false, Some(true), Some(false)),
            ("partOf indirect containment", true, Some(true), Some(true)),
            ("classification prohibited", true, None, None),
            ("entity user defined", true, None, None),
            ("no proxies", true, None, None),
            ("quantity volume", false, Some(true), Some(false)),
            ("applicability by property", true, Some(true), Some(true)),
            ("road part", true, None, None),
            ("bridges required", false, None, None),
        ];
        for &(name, pass, e20, e21) in cases {
            let s = spec(name);
            assert_eq!(s.status == Status::Pass, pass, "{name}: {:#?}", s);
            if let Some(p) = e20 {
                assert_eq!(
                    elem(name, 20).pass,
                    p,
                    "{name} #20: {:?}",
                    elem(name, 20).reasons
                );
            }
            if let Some(p) = e21 {
                assert_eq!(
                    elem(name, 21).pass,
                    p,
                    "{name} #21: {:?}",
                    elem(name, 21).reasons
                );
            }
        }
        assert_eq!(spec("classification prohibited").applicable, 1);
        assert_eq!(spec("entity user defined").passed, 1);
        assert_eq!(spec("road part").applicable, 1);
        let r = &elem("property thickness with units", 21).reasons[0];
        assert!(r.contains("0.03") && r.contains(">= 0.04"), "{r}");
        assert!(!rep.passed());
        assert_eq!(rep.summary.specifications, cases.len());
        let md = rep.to_markdown();
        assert!(md.contains("| entity predefined type | FAIL |"), "{md}");
        assert!(md.contains("#21 IFCCOURSE"));
        let js: serde_json::Value = serde_json::from_str(&rep.to_json()).unwrap();
        assert_eq!(js["summary"]["specifications"], cases.len());
    }

    /// buildingSMART IDS 1.0 implementers' test cases (CC BY-ND 4.0,
    /// unmodified copies; see tests/fixtures/ids/buildingsmart/SOURCE.txt).
    /// The file name prefix gives the expected result.
    #[test]
    fn buildingsmart_test_cases() {
        let dir = format!(
            "{}/tests/fixtures/ids/buildingsmart",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".ids"))
            .collect();
        names.sort();
        assert!(names.len() >= 10, "{names:?}");
        let mut wrong = Vec::new();
        for n in &names {
            let stem = n.trim_end_matches(".ids");
            let ids = Ids::parse(&fixture(&format!("buildingsmart/{stem}.ids"))).unwrap();
            let model =
                StepModel::parse_str(&fixture(&format!("buildingsmart/{stem}.ifc"))).unwrap();
            let rep = ids.check(&model);
            let expect = stem.split('-').nth(1) == Some("pass");
            if rep.passed() != expect {
                wrong.push(format!(
                    "{stem}: got {}\n{}",
                    rep.passed(),
                    rep.to_markdown()
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}
