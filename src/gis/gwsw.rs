//! GWSW — Gegevenswoordenboek Stedelijk Water: the sewer networks Dutch
//! municipalities and water boards publish, served by PDOK as OGC API
//! Features (`beheer-stedelijk-watersystemen-gwsw`, CC0), and their mapping
//! to the NLCS main group RI (riolering).

use super::ogcapi;
use super::site::Site;
use super::vector::Feature;

pub const API: &str = "https://api.pdok.nl/rioned/beheer-stedelijk-watersystemen-gwsw/ogc/v1";

/// The collections, in drawing order (pipes under the manholes).
pub const COLLECTIONS: &[(&str, &str)] = &[
    ("beheerleiding", "Leidingen"),
    ("aansluitingleiding", "Aansluitleidingen"),
    ("beheerput", "Putten"),
    ("beheerbouwwerk", "Bouwwerken (gemalen, uitlaten)"),
    ("beheerpomp", "Pompen"),
    ("aansluitingpunt", "Aansluitpunten"),
];

/// The last part of a GWSW URI (`http://data.gwsw.nl/1.5/totaal/Beton` → `Beton`).
pub fn term(v: Option<&serde_json::Value>) -> String {
    v.and_then(|v| v.as_str()).map(|s| s.rsplit(['/', '#']).next().unwrap_or(s).to_string()).unwrap_or_default()
}

/// NLCS system prefix in group RI for a GWSW pipe type or system type.
pub fn system(kind: &str, stelseltype: &str) -> &'static str {
    let k = format!("{} {}", kind, stelseltype).to_ascii_lowercase();
    if k.contains("drain") {
        "DRAINAGE"
    } else if k.contains("druk") || k.contains("pers") {
        "PERSLEIDING"
    } else if k.contains("vacuum") {
        "VACUUMLEIDING"
    } else if k.contains("infiltr") {
        "ITR"
    } else if k.contains("hemelwater") || k.contains("regenwater") || k.contains("hwa") {
        "HWA"
    } else if k.contains("vuilwater") || k.contains("afvalwater") || k.contains("dwa") {
        "DWA"
    } else {
        // Gemengd riool, overstort- and bergingsleidingen, unknown.
        "GWA"
    }
}

fn material(m: &str) -> Option<&'static str> {
    let m = m.to_ascii_lowercase();
    Some(match () {
        _ if m.contains("beton") => "BETON",
        _ if m.contains("gres") => "GRES",
        _ if m.contains("pvc") => "PVC",
        _ if m == "pe" || m.contains("polyethyleen") || m.starts_with("hdpe") => "PE",
        _ if m == "pp" || m.contains("polypropyleen") => "PP",
        _ if m.contains("gietijzer") => "GIETIJZER",
        _ if m.contains("staal") => "STAAL",
        _ if m.contains("metsel") => "METSELWERK",
        _ if m.contains("asbest") => "ASBESTCEMENT",
        _ => return None,
    })
}

/// NLCS object names to try, most specific first, for a feature of
/// `collection`. The caller takes the first one the NLCS table has.
pub fn nlcs_candidates(collection: &str, f: &Feature) -> Vec<String> {
    let kind = term(f.props.get("type"));
    let stelsel = term(f.props.get("stelseltype"));
    let sys = system(&kind, &stelsel);
    let mat = material(&term(f.props.get("materiaal_leiding")));
    let width = f.props.get("breedte_leiding").and_then(|v| v.as_f64()).filter(|w| *w > 0.0).map(|w| w.round() as i64);
    let mut out = Vec::new();
    match collection {
        "beheerleiding" => {
            let base = match sys {
                "DRAINAGE" => "DRAINAGE_DRAIN".to_string(),
                "PERSLEIDING" | "VACUUMLEIDING" => sys.to_string(),
                s => format!("{s}_RIOOLLEIDING"),
            };
            if let Some(m) = mat {
                if let Some(w) = width {
                    out.push(format!("{base}_{m}_{w}"));
                }
                out.push(format!("{base}_{m}"));
            }
            out.push(base);
        }
        "aansluitingleiding" => {
            if kind.to_ascii_lowercase().contains("kolk") {
                out.push(format!("{sys}_KOLKAANSLUITLEIDING"));
            }
            out.push(format!("{sys}_PERCEELAANSLUITLEIDING"));
            out.push(format!("{sys}_KOLKAANSLUITLEIDING"));
        }
        "beheerput" => {
            let k = kind.to_ascii_lowercase();
            let sub = if k.contains("blinde") { "BLINDEPUT" } else if k.contains("pomp") { "POMPPUT" } else if k.contains("overstort") { "OVERSTORTPUT" } else { "INSPECTIEPUT" };
            out.push(format!("{sys}_RIOOLPUT_{sub}"));
            out.push(format!("{sys}_RIOOLPUT"));
        }
        "beheerbouwwerk" | "beheerpomp" => {
            let k = kind.to_ascii_lowercase();
            if k.contains("uitlaat") || k.contains("overstort") {
                out.push(format!("{sys}_UITLAATCONSTRUCTIE"));
            } else {
                out.push(format!("{sys}_RIOOLPUT_POMPPUT"));
                out.push("PERSLEIDING_GEMAAL".into());
            }
        }
        _ => {
            out.push(format!("{sys}_PERCEELAANSLUITPUNT"));
            out.push(format!("{sys}_AANSLUITPUT"));
        }
    }
    out.push(format!("{sys}_RIOOLLEIDING"));
    out.push("GWA_RIOOLLEIDING".into());
    out
}

/// Fetch the GWSW collections in the site extent (in RD).
pub fn fetch(site: &Site, collections: &[&str], progress: &dyn Fn(String)) -> Result<Vec<(String, Vec<Feature>)>, String> {
    let b = if site.crs == "EPSG:28992" {
        site.bbox()
    } else {
        super::site::envelope(site.bbox(), &super::crs::Transformer::new(&site.crs, "EPSG:28992")?)?
    };
    collections
        .iter()
        .map(|c| ogcapi::fetch_items(API, c, b, 28992, None, 50_000, progress).map(|(f, _)| (c.to_string(), f)))
        .collect()
}

// ── GWSW-OroX: export and reading back ──────────────────────────────────────
//
// GWSW-OroX is RDF Turtle on the GWSW ontology of Stichting RIONED, here
// version 1.7 (data.gwsw.nl, namespace http://data.gwsw.nl/1.7/totaal/),
// after the official example dataset `GwswDataset__Voorbeeldstelsel_v1.7.orox.ttl`
// and the handbook "GWSW.orox Opbouw dataset" (apps.gwsw.nl/doc). Objects
// carry their values as aspects (`gwsw:hasAspect [ a gwsw:X ; gwsw:hasValue v ]`);
// geometry is a GML literal on the orientation of a manhole (Putorientatie,
// Punt) or pipe (Leidingorientatie, Lijn), in RD with NAP heights; the BOB
// sits on the begin and end points of the pipe, which connect to the
// orientations of their manholes.

pub const OROX_VERSIE: &str = "1.7";
pub const OROX_NS: &str = "http://data.gwsw.nl/1.7/totaal/";
const GML_NS: &str = "http://www.opengis.net/gml/3.2";

/// A system (stelsel) in an OroX file.
pub struct OroxStelsel {
    /// GWSW class, e.g. `Vuilwaterstelsel`.
    pub klasse: String,
    pub label: String,
}

pub struct OroxPut {
    pub naam: String,
    /// GWSW class, e.g. `Inspectieput`.
    pub klasse: String,
    pub stelsel: usize,
    /// RD (EPSG:28992).
    pub xy: [f64; 2],
    /// m NAP.
    pub maaiveld: Option<f64>,
    /// mm: width and length (equal for a round manhole), height.
    pub breedte: f64,
    pub lengte: f64,
    pub hoogte: Option<f64>,
    pub rond: bool,
    pub materiaal: Option<String>,
}

pub struct OroxLeiding {
    pub naam: String,
    /// GWSW class, e.g. `Vuilwaterriool`.
    pub klasse: String,
    pub stelsel: usize,
    pub van: usize,
    pub naar: usize,
    /// RD (EPSG:28992).
    pub punten: Vec<[f64; 2]>,
    /// mm.
    pub breedte: f64,
    /// GWSW material individual, e.g. `PVC`, `Beton`.
    pub materiaal: String,
    /// m.
    pub lengte: f64,
    /// m NAP.
    pub bob_van: f64,
    pub bob_naar: f64,
    /// A design value (WijzeVanInwinning Plan_Ontwerp) rather than measured.
    pub ontwerp: bool,
}

pub struct Orox {
    /// The dataset name: the file name without `.orox.ttl`.
    pub naam: String,
    pub app: String,
    pub datum: String,
    pub stelsels: Vec<OroxStelsel>,
    pub putten: Vec<OroxPut>,
    pub leidingen: Vec<OroxLeiding>,
}

/// A Turtle string literal.
fn ttl_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// A local name: only `[A-Za-z0-9_]`, as the handbook asks.
pub fn orox_local(s: &str) -> String {
    let v: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if v.is_empty() { "x".into() } else { v }
}

fn gml_point(p: [f64; 2]) -> String {
    ttl_str(&format!("<gml:Point xmlns:gml=\"{GML_NS}\"><gml:pos>{:.3} {:.3}</gml:pos></gml:Point>", p[0], p[1])) + "^^geo:gmlLiteral"
}

fn gml_line(pts: &[[f64; 2]]) -> String {
    let list: Vec<String> = pts.iter().map(|p| format!("{:.3} {:.3}", p[0], p[1])).collect();
    ttl_str(&format!("<gml:LineString xmlns:gml=\"{GML_NS}\"><gml:posList srsDimension=\"2\">{}</gml:posList></gml:LineString>", list.join(" "))) + "^^geo:gmlLiteral"
}

/// Write a GWSW-OroX dataset (Turtle).
pub fn write_orox(o: &Orox) -> String {
    let slug = orox_local(&o.naam);
    let mut t = String::new();
    t.push_str("@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n");
    t.push_str("@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
    t.push_str("@prefix owl: <http://www.w3.org/2002/07/owl#> .\n");
    t.push_str("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
    t.push_str("@prefix geo: <http://www.opengis.net/ont/geosparql#> .\n");
    t.push_str(&format!("@prefix gwsw: <{OROX_NS}> .\n"));
    t.push_str(&format!("@prefix : <http://sparql.gwsw.nl/bim/{slug}#> .\n\n"));
    t.push_str(&format!("<{}.orox.ttl> a owl:Ontology ;\n    owl:versionInfo {} .\n\n", o.naam, ttl_str(&format!("Export for GWSW {OROX_VERSIE} from {}, {}", o.app, o.datum))));
    for (i, s) in o.stelsels.iter().enumerate() {
        t.push_str(&format!(":Stelsel_{} a gwsw:{} ;\n    rdfs:label {} .\n\n", i + 1, s.klasse, ttl_str(&s.label)));
    }
    let design = "gwsw:hasAspect [ a gwsw:Inwinning ; gwsw:hasAspect [ a gwsw:WijzeVanInwinning ; gwsw:hasReference gwsw:Plan_Ontwerp ] ]";
    for (i, p) in o.putten.iter().enumerate() {
        let id = format!("{}_{}", p.klasse, i + 1);
        let mut aspects = vec![
            format!(":{id}_ori"),
            format!("[ a gwsw:BreedtePut ; gwsw:hasValue {:.0} ]", p.breedte),
            format!("[ a gwsw:LengtePut ; gwsw:hasValue {:.0} ]", p.lengte),
        ];
        if p.rond {
            aspects.push(format!("[ a gwsw:DiameterPut ; gwsw:hasValue {:.0} ]", p.breedte));
        }
        if let Some(h) = p.hoogte {
            aspects.push(format!("[ a gwsw:HoogtePut ; gwsw:hasValue {:.0} ]", h));
        }
        aspects.push(format!("[ a gwsw:VormPut ; gwsw:hasReference gwsw:{} ]", if p.rond { "Rond" } else { "Rechthoekig" }));
        if let Some(m) = &p.materiaal {
            aspects.push(format!("[ a gwsw:MateriaalPut ; gwsw:hasReference gwsw:{m} ]"));
        }
        t.push_str(&format!(":{id} a gwsw:{} ;\n    rdfs:label {} ;\n    gwsw:isPartOf :Stelsel_{} ;\n    gwsw:hasAspect {} .\n", p.klasse, ttl_str(&p.naam), p.stelsel + 1, aspects.join(" ,\n        ")));
        t.push_str(&format!(":{id}_ori a gwsw:Putorientatie ;\n    gwsw:hasAspect [ a gwsw:Punt ; gwsw:hasValue {} ]", gml_point(p.xy)));
        if let Some(z) = p.maaiveld {
            t.push_str(&format!(" ;\n    gwsw:hasConnection [ a gwsw:Maaiveldorientatie ; gwsw:hasAspect [ a gwsw:Maaiveldhoogte ; gwsw:hasValue {z:.3} ] ]"));
        }
        t.push_str(" .\n\n");
    }
    let put_id = |i: usize| format!("{}_{}", o.putten[i].klasse, i + 1);
    for (i, l) in o.leidingen.iter().enumerate() {
        let id = format!("{}_{}", l.klasse, i + 1);
        let aspects = [
            format!(":{id}_ori"),
            format!("[ a gwsw:BreedteLeiding ; gwsw:hasValue {:.0} ]", l.breedte),
            format!("[ a gwsw:HoogteLeiding ; gwsw:hasValue {:.0} ]", l.breedte),
            "[ a gwsw:VormLeiding ; gwsw:hasReference gwsw:Rond ]".to_string(),
            format!("[ a gwsw:MateriaalLeiding ; gwsw:hasReference gwsw:{} ]", l.materiaal),
            format!("[ a gwsw:LengteLeiding ; gwsw:hasValue {:.2} ]", l.lengte),
        ];
        t.push_str(&format!(":{id} a gwsw:{} ;\n    rdfs:label {} ;\n    gwsw:isPartOf :Stelsel_{} ;\n    gwsw:hasAspect {} .\n", l.klasse, ttl_str(&l.naam), l.stelsel + 1, aspects.join(" ,\n        ")));
        t.push_str(&format!(":{id}_ori a gwsw:Leidingorientatie ;\n    gwsw:hasAspect [ a gwsw:Lijn ; gwsw:hasValue {} ] ;\n    gwsw:hasPart :{id}_bgp , :{id}_enp .\n", gml_line(&l.punten)));
        let how = if l.ontwerp { format!(" ;\n        {design}") } else { String::new() };
        t.push_str(&format!(":{id}_bgp a gwsw:BeginpuntLeiding ;\n    gwsw:hasConnection :{}_ori ;\n    gwsw:hasAspect [ a gwsw:BobBeginpuntLeiding ; gwsw:hasValue {:.3}{how} ] .\n", put_id(l.van), l.bob_van));
        t.push_str(&format!(":{id}_enp a gwsw:EindpuntLeiding ;\n    gwsw:hasConnection :{}_ori ;\n    gwsw:hasAspect [ a gwsw:BobEindpuntLeiding ; gwsw:hasValue {:.3}{how} ] .\n\n", put_id(l.naar), l.bob_naar));
    }
    t
}

// A small Turtle reader: enough for GWSW-OroX (prefixes, IRIs, prefixed
// names, blank node property lists, literals with datatype or language).

#[derive(Clone, Debug, PartialEq)]
enum Term {
    Iri(String),
    Blank(String),
    Lit(String),
}

struct Ttl<'a> {
    s: &'a [u8],
    i: usize,
    prefixes: std::collections::HashMap<String, String>,
    blank: usize,
    triples: Vec<(Term, String, Term)>,
}

impl<'a> Ttl<'a> {
    fn err(&self, what: &str) -> String {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|c| **c == b'\n').count() + 1;
        format!("Turtle, {} {line}: {what}", crate::lang::tr("regel", "line"))
    }

    fn ws(&mut self) {
        while self.i < self.s.len() {
            match self.s[self.i] {
                b' ' | b'\t' | b'\n' | b'\r' => self.i += 1,
                b'#' => {
                    while self.i < self.s.len() && self.s[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("'{}' {}", c as char, crate::lang::tr("verwacht", "expected"))))
        }
    }

    fn word(&mut self) -> String {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() || matches!(c, b'<' | b'>' | b'"' | b'\'' | b'[' | b']' | b'(' | b')' | b';' | b',' | b'#') {
                break;
            }
            // A final '.' ends the statement, not the name.
            if c == b'.' && self.s.get(self.i + 1).is_none_or(|n| n.is_ascii_whitespace() || *n == b'#') {
                break;
            }
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn iri_ref(&mut self) -> Result<String, String> {
        self.eat(b'<')?;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != b'>' {
            self.i += 1;
        }
        let v = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.eat(b'>')?;
        Ok(v)
    }

    fn pname(&self, w: &str) -> Result<String, String> {
        let (p, local) = w.split_once(':').ok_or_else(|| self.err(&format!("{} '{w}'", crate::lang::tr("onbekende term", "unknown term"))))?;
        let ns = self.prefixes.get(p).ok_or_else(|| self.err(&format!("{} '{p}:'", crate::lang::tr("onbekend voorvoegsel", "unknown prefix"))))?;
        Ok(format!("{ns}{}", local.replace('\\', "")))
    }

    fn string(&mut self) -> Result<String, String> {
        let q = self.s[self.i];
        let long = self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..]);
        self.i += if long { 3 } else { 1 };
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return Err(self.err(crate::lang::tr("tekst niet afgesloten", "unterminated string"))) };
            if c == q && (!long || self.s.get(self.i..self.i + 3) == Some(&[q, q, q][..])) {
                self.i += if long { 3 } else { 1 };
                break;
            }
            if c == b'\\' {
                let e = self.s.get(self.i + 1).copied().unwrap_or(b'\\');
                out.push(match e {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    other => other,
                });
                self.i += 2;
                continue;
            }
            out.push(c);
            self.i += 1;
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn object(&mut self) -> Result<Term, String> {
        match self.peek() {
            Some(b'<') => Ok(Term::Iri(self.iri_ref()?)),
            Some(b'[') => self.blank_list(),
            Some(b'"') | Some(b'\'') => {
                let v = self.string()?;
                // Datatype or language: the value is what counts here.
                if self.s.get(self.i..self.i + 2) == Some(&b"^^"[..]) {
                    self.i += 2;
                    if self.peek() == Some(b'<') {
                        self.iri_ref()?;
                    } else {
                        self.word();
                    }
                } else if self.s.get(self.i) == Some(&b'@') {
                    self.word();
                }
                Ok(Term::Lit(v))
            }
            Some(b'(') => Err(self.err(crate::lang::tr("collecties worden niet gelezen", "collections are not read"))),
            Some(_) => {
                let w = self.word();
                if w.is_empty() {
                    return Err(self.err(crate::lang::tr("term verwacht", "term expected")));
                }
                if let Some(b) = w.strip_prefix("_:") {
                    return Ok(Term::Blank(b.to_string()));
                }
                let first = w.as_bytes()[0];
                if first.is_ascii_digit() || first == b'-' || first == b'+' || first == b'.' || w == "true" || w == "false" {
                    return Ok(Term::Lit(w));
                }
                Ok(Term::Iri(self.pname(&w)?))
            }
            None => Err(self.err(crate::lang::tr("onverwacht einde", "unexpected end"))),
        }
    }

    fn verb(&mut self) -> Result<String, String> {
        if self.peek() == Some(b'<') {
            return self.iri_ref();
        }
        let w = self.word();
        if w == "a" {
            return Ok(RDF_TYPE.to_string());
        }
        self.pname(&w)
    }

    fn blank_list(&mut self) -> Result<Term, String> {
        self.eat(b'[')?;
        self.blank += 1;
        let b = Term::Blank(format!("b{}", self.blank));
        if self.peek() != Some(b']') {
            self.predicates(&b)?;
        }
        self.eat(b']')?;
        Ok(b)
    }

    fn predicates(&mut self, subject: &Term) -> Result<(), String> {
        loop {
            let p = self.verb()?;
            loop {
                let o = self.object()?;
                self.triples.push((subject.clone(), p.clone(), o));
                if self.peek() == Some(b',') {
                    self.i += 1;
                    continue;
                }
                break;
            }
            if self.peek() == Some(b';') {
                while self.peek() == Some(b';') {
                    self.i += 1;
                }
                if matches!(self.peek(), Some(b'.') | Some(b']') | None) {
                    return Ok(());
                }
                continue;
            }
            return Ok(());
        }
    }

    fn parse(mut self) -> Result<Vec<(Term, String, Term)>, String> {
        while let Some(c) = self.peek() {
            if c == b'@' || self.s[self.i..].starts_with(b"PREFIX") || self.s[self.i..].starts_with(b"BASE") {
                let kw = self.word();
                let kw = kw.trim_start_matches('@').to_ascii_lowercase();
                if kw == "prefix" {
                    let p = self.word();
                    let ns = self.iri_ref()?;
                    self.prefixes.insert(p.trim_end_matches(':').to_string(), ns);
                } else {
                    self.iri_ref()?;
                }
                if self.peek() == Some(b'.') {
                    self.i += 1;
                }
                continue;
            }
            let subject = if c == b'[' { self.blank_list()? } else { self.object()? };
            if self.peek() != Some(b'.') {
                self.predicates(&subject)?;
            }
            self.eat(b'.')?;
        }
        Ok(self.triples)
    }
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

fn parse_ttl(text: &str) -> Result<Vec<(Term, String, Term)>, String> {
    Ttl { s: text.as_bytes(), i: 0, prefixes: Default::default(), blank: 0, triples: Vec::new() }.parse()
}

/// The points of a GML `posList` or `pos` (x, y; a third value is dropped).
fn gml_points(gml: &str) -> Vec<[f64; 2]> {
    // The element's attributes and its text.
    let element = |tag: &str| -> Option<(String, String)> {
        for prefix in ["gml:", ""] {
            let open = format!("<{prefix}{tag}");
            let mut from = 0;
            while let Some(i) = gml[from..].find(&open) {
                let rest = &gml[from + i + open.len()..];
                if rest.starts_with([' ', '>', '\t', '\n', '\r']) {
                    let close = rest.find('>')?;
                    return Some((rest[..close].to_string(), rest[close + 1..].split('<').next()?.to_string()));
                }
                from += i + open.len();
            }
        }
        None
    };
    let (attrs, text, list) = match element("posList") {
        Some((a, t)) => (a, t, true),
        None => match element("pos") {
            Some((a, t)) => (a, t, false),
            None => return Vec::new(),
        },
    };
    let nums: Vec<f64> = text.split_whitespace().filter_map(|v| v.parse().ok()).collect();
    let given = attrs.split("srsDimension=").nth(1).and_then(|r| r.trim_start_matches(['"', '\'']).split(['"', '\'']).next()).and_then(|d| d.parse::<usize>().ok());
    let dim = match (given, list) {
        (Some(d), _) => d.max(2),
        (None, true) => 2,
        (None, false) => nums.len().clamp(2, 3),
    };
    nums.chunks(dim).filter(|c| c.len() >= 2).map(|c| [c[0], c[1]]).collect()
}

/// Read a GWSW-OroX file into the collections and properties of the PDOK
/// GWSW service (`beheerleiding`, `beheerput`), so it is drawn by the same
/// route as `GIS_Q_GWSW` (geometry in RD).
pub fn read_orox(text: &str) -> Result<Vec<(String, Vec<Feature>)>, String> {
    use serde_json::{json, Value};
    use std::collections::HashMap;
    let triples = parse_ttl(text)?;
    let key = |t: &Term| match t {
        Term::Iri(s) => format!("<{s}>"),
        Term::Blank(b) => format!("_:{b}"),
        Term::Lit(v) => v.clone(),
    };
    let mut out: HashMap<String, Vec<(String, Term)>> = HashMap::new();
    let mut inv: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (s, p, o) in &triples {
        out.entry(key(s)).or_default().push((p.clone(), o.clone()));
        if !matches!(o, Term::Lit(_)) {
            inv.entry(key(o)).or_default().push((p.clone(), key(s)));
        }
    }
    let local = |iri: &str| iri.rsplit(['/', '#']).next().unwrap_or(iri).to_string();
    // GWSW terms of any version (1.5, 1.6, 1.7 …) by their local name.
    let gp = |p: &str, name: &str| p.starts_with("http://data.gwsw.nl/") && p.rsplit('/').next() == Some(name);
    let types = |n: &str| -> Vec<String> { out.get(n).map(|v| v.iter().filter(|(p, _)| p == RDF_TYPE).filter_map(|(_, o)| if let Term::Iri(i) = o { Some(i.clone()) } else { None }).collect()).unwrap_or_default() };
    let is = |n: &str, class: &str| types(n).iter().any(|t| local(t) == class);
    let objs = |n: &str, p: &str| -> Vec<String> { out.get(n).map(|v| v.iter().filter(|(q, _)| gp(q, p)).map(|(_, o)| key(o)).collect()).unwrap_or_default() };
    let lit = |n: &str, p: &str| -> Option<String> { out.get(n).and_then(|v| v.iter().find(|(q, _)| q == p || gp(q, p)).and_then(|(_, o)| if let Term::Lit(l) = o { Some(l.clone()) } else { None })) };
    // An aspect of a given class on a node: its value (or reference).
    let aspect = |n: &str, class: &str| -> Option<Term> {
        objs(n, "hasAspect").into_iter().find(|a| is(a, class)).and_then(|a| out.get(&a).and_then(|v| v.iter().find(|(p, _)| gp(p, "hasValue") || gp(p, "hasReference")).map(|(_, o)| o.clone())))
    };
    let num = |t: Option<Term>| -> Option<f64> { match t { Some(Term::Lit(v)) => v.parse().ok(), _ => None } };
    let refer = |t: Option<Term>| -> Option<String> { match t { Some(Term::Iri(i)) => Some(i), Some(Term::Lit(v)) => Some(v), _ => None } };
    let label = |n: &str| lit(n, RDFS_LABEL);
    // The system of an object: isPartOf, or the inverse hasPart.
    let stelsel = |n: &str| -> Option<String> {
        objs(n, "isPartOf").into_iter().chain(inv.get(n).map(|v| v.iter().filter(|(p, _)| gp(p, "hasPart")).map(|(_, s)| s.clone()).collect::<Vec<_>>()).unwrap_or_default()).find(|s| types(s).iter().any(|t| local(t).to_ascii_lowercase().contains("stelsel")))
    };
    let mut puts = Vec::new();
    let mut put_of_ori: HashMap<String, String> = HashMap::new();
    let mut pipes = Vec::new();
    for n in out.keys() {
        for a in objs(n, "hasAspect") {
            if is(&a, "Putorientatie") {
                put_of_ori.insert(a.clone(), n.clone());
                puts.push((n.clone(), a));
            } else if is(&a, "Leidingorientatie") {
                pipes.push((n.clone(), a));
            }
        }
    }
    puts.sort();
    pipes.sort();
    let mut features_put = Vec::new();
    for (n, ori) in &puts {
        let gml = objs(ori, "hasAspect").into_iter().find(|a| is(a, "Punt")).and_then(|a| lit(&a, "hasValue"));
        let Some(p) = gml.as_deref().map(gml_points).and_then(|v| v.first().copied()) else { continue };
        let mut f = Feature { points: vec![p], ..Default::default() };
        let mut props = serde_json::Map::new();
        props.insert("naam".into(), json!(label(n).unwrap_or_default()));
        if let Some(t) = types(n).first() {
            props.insert("type".into(), json!(t));
        }
        if let Some(s) = stelsel(n) {
            props.insert("stelsel".into(), json!(label(&s).unwrap_or_default()));
            if let Some(t) = types(&s).first() {
                props.insert("stelseltype".into(), json!(t));
            }
        }
        let mv = objs(ori, "hasConnection").into_iter().find(|c| is(c, "Maaiveldorientatie")).and_then(|c| num(aspect(&c, "Maaiveldhoogte")));
        if let Some(z) = mv {
            props.insert("maaiveldhoogte".into(), json!(z));
        }
        for (k, class) in [("breedte_put", "BreedtePut"), ("lengte_put", "LengtePut"), ("hoogte_put", "HoogtePut")] {
            if let Some(v) = num(aspect(n, class)) {
                props.insert(k.into(), json!(v));
            }
        }
        for (k, class) in [("vorm_put", "VormPut"), ("materiaal_put", "MateriaalPut")] {
            if let Some(v) = refer(aspect(n, class)) {
                props.insert(k.into(), Value::String(v));
            }
        }
        f.props = props;
        features_put.push(f);
    }
    let mut features_pipe = Vec::new();
    for (n, ori) in &pipes {
        let gml = objs(ori, "hasAspect").into_iter().find(|a| is(a, "Lijn")).and_then(|a| lit(&a, "hasValue"));
        let pts = gml.as_deref().map(gml_points).unwrap_or_default();
        if pts.len() < 2 {
            continue;
        }
        let mut f = Feature { lines: vec![(pts, false)], ..Default::default() };
        let mut props = serde_json::Map::new();
        props.insert("naam".into(), json!(label(n).unwrap_or_default()));
        if let Some(t) = types(n).first() {
            props.insert("type".into(), json!(t));
        }
        if let Some(s) = stelsel(n) {
            props.insert("stelsel".into(), json!(label(&s).unwrap_or_default()));
            if let Some(t) = types(&s).first() {
                props.insert("stelseltype".into(), json!(t));
            }
        }
        for part in objs(ori, "hasPart") {
            let (end, bob) = if is(&part, "BeginpuntLeiding") { ("beginpunt", "bob_beginpunt_leiding") } else if is(&part, "EindpuntLeiding") { ("eindpunt", "bob_eindpunt_leiding") } else { continue };
            let class = if end == "beginpunt" { "BobBeginpuntLeiding" } else { "BobEindpuntLeiding" };
            if let Some(v) = num(aspect(&part, class)) {
                props.insert(bob.into(), json!(v));
            }
            if let Some(name) = objs(&part, "hasConnection").into_iter().find_map(|c| put_of_ori.get(&c).and_then(|p| label(p))) {
                props.insert(end.into(), json!(name));
            }
        }
        for (k, class) in [("breedte_leiding", "BreedteLeiding"), ("hoogte_leiding", "HoogteLeiding"), ("lengte_leiding", "LengteLeiding")] {
            if let Some(v) = num(aspect(n, class)) {
                props.insert(k.into(), json!(v));
            }
        }
        if !props.contains_key("breedte_leiding") {
            if let Some(v) = num(aspect(n, "DiameterLeiding")) {
                props.insert("breedte_leiding".into(), json!(v));
            }
        }
        for (k, class) in [("materiaal_leiding", "MateriaalLeiding"), ("vorm_leiding", "VormLeiding")] {
            if let Some(v) = refer(aspect(n, class)) {
                props.insert(k.into(), Value::String(v));
            }
        }
        f.props = props;
        features_pipe.push(f);
    }
    if features_put.is_empty() && features_pipe.is_empty() {
        return Err(crate::lang::tr("geen GWSW-putten of -leidingen in het bestand (verwacht GWSW-OroX: Putorientatie / Leidingorientatie)", "no GWSW manholes or pipes in the file (expected GWSW-OroX: Putorientatie / Leidingorientatie)").into());
    }
    Ok(vec![("beheerleiding".to_string(), features_pipe), ("beheerput".to_string(), features_put)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(pairs: &[(&str, serde_json::Value)]) -> Feature {
        let mut f = Feature::default();
        for (k, v) in pairs {
            f.props.insert((*k).into(), v.clone());
        }
        f
    }

    #[test]
    fn pipes_map_to_the_most_specific_nlcs_object() {
        let f = feature(&[("type", "http://data.gwsw.nl/1.5/totaal/GemengdRiool".into()), ("materiaal_leiding", "http://data.gwsw.nl/1.5/totaal/Beton".into()), ("breedte_leiding", 800.into())]);
        let c = nlcs_candidates("beheerleiding", &f);
        assert_eq!(c[0], "GWA_RIOOLLEIDING_BETON_800");
        assert!(c.contains(&"GWA_RIOOLLEIDING".to_string()));
        let h = feature(&[("type", "Hemelwaterriool".into())]);
        assert_eq!(nlcs_candidates("beheerleiding", &h)[0], "HWA_RIOOLLEIDING");
        let p = feature(&[("type", "Drukleiding".into())]);
        assert_eq!(nlcs_candidates("beheerleiding", &p)[0], "PERSLEIDING");
        let put = feature(&[("type", "Inspectieput".into()), ("stelseltype", "Regenwaterstelsel".into())]);
        assert_eq!(nlcs_candidates("beheerput", &put)[0], "HWA_RIOOLPUT_INSPECTIEPUT");
    }

    #[test]
    fn orox_round_trip() {
        let o = Orox {
            naam: "riool_1".into(),
            app: "Open Infra test".into(),
            datum: "2026-10-29".into(),
            stelsels: vec![OroxStelsel { klasse: "Vuilwaterstelsel".into(), label: "Riool 1".into() }],
            putten: vec![
                OroxPut { naam: "P101".into(), klasse: "Inspectieput".into(), stelsel: 0, xy: [104900.0, 425300.0], maaiveld: Some(1.25), breedte: 800.0, lengte: 800.0, hoogte: Some(2150.0), rond: true, materiaal: None },
                OroxPut { naam: "P102".into(), klasse: "Inspectieput".into(), stelsel: 0, xy: [104945.2, 425300.0], maaiveld: Some(1.1), breedte: 800.0, lengte: 800.0, hoogte: None, rond: true, materiaal: None },
            ],
            leidingen: vec![OroxLeiding { naam: "P101-P102".into(), klasse: "Vuilwaterriool".into(), stelsel: 0, van: 0, naar: 1, punten: vec![[104900.0, 425300.0], [104945.2, 425300.0]], breedte: 250.0, materiaal: "PVC".into(), lengte: 45.2, bob_van: -0.9, bob_naar: -1.081, ontwerp: true }],
        };
        let ttl = write_orox(&o);
        assert!(ttl.contains("gwsw:BobBeginpuntLeiding ; gwsw:hasValue -0.900"), "{ttl}");
        let data = read_orox(&ttl).unwrap();
        let pipes = &data.iter().find(|(c, _)| c == "beheerleiding").unwrap().1;
        let puts = &data.iter().find(|(c, _)| c == "beheerput").unwrap().1;
        assert_eq!((pipes.len(), puts.len()), (1, 2));
        let p = &pipes[0].props;
        assert_eq!(p["naam"], "P101-P102");
        assert_eq!(p["beginpunt"], "P101");
        assert_eq!(p["eindpunt"], "P102");
        assert_eq!(p["bob_beginpunt_leiding"].as_f64(), Some(-0.9));
        assert_eq!(p["bob_eindpunt_leiding"].as_f64(), Some(-1.081));
        assert_eq!(p["breedte_leiding"].as_f64(), Some(250.0));
        assert_eq!(term(p.get("materiaal_leiding")), "PVC");
        assert_eq!(term(p.get("stelseltype")), "Vuilwaterstelsel");
        assert_eq!(pipes[0].lines[0].0[1], [104945.2, 425300.0]);
        // The existing GWSW drawing route puts it on the right NLCS objects.
        assert_eq!(nlcs_candidates("beheerleiding", &pipes[0])[0], "DWA_RIOOLLEIDING_PVC_250");
        let put = puts.iter().find(|f| f.props["naam"] == "P101").unwrap();
        assert_eq!(put.props["maaiveldhoogte"].as_f64(), Some(1.25));
        assert_eq!(put.points[0], [104900.0, 425300.0]);
        assert_eq!(nlcs_candidates("beheerput", put)[0], "DWA_RIOOLPUT_INSPECTIEPUT");
    }

    #[test]
    fn turtle_reader_takes_other_layouts() {
        let ttl = "@prefix gwsw: <http://data.gwsw.nl/1.6/totaal/> .\n@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
            PREFIX ex: <http://example.org/x#>\n\
            ex:p1 a gwsw:Inspectieput ; rdfs:label 'K1'@nl ; gwsw:hasAspect ex:o1 . # comment\n\
            ex:o1 a gwsw:Putorientatie ; gwsw:hasAspect [ a gwsw:Punt ; gwsw:hasValue \"<gml:Point xmlns:gml='http://www.opengis.net/gml'><gml:pos>1.5 2.5 -1.0</gml:pos></gml:Point>\"^^<http://www.opengis.net/ont/geosparql#gmlLiteral> ] .";
        let data = read_orox(ttl).unwrap();
        let puts = &data[1].1;
        assert_eq!(puts.len(), 1);
        assert_eq!(puts[0].points[0], [1.5, 2.5]);
        assert_eq!(puts[0].props["naam"], "K1");
    }

    #[test]
    fn the_first_candidates_exist_in_nlcs() {
        for (coll, kind) in [("beheerleiding", "GemengdRiool"), ("beheerleiding", "Vuilwaterriool"), ("beheerleiding", "Infiltratieriool"), ("beheerput", "Inspectieput"), ("aansluitingleiding", "Kolkaansluitleiding"), ("beheerbouwwerk", "Uitlaatconstructie"), ("beheerbouwwerk", "Rioolgemaal")] {
            let f = feature(&[("type", kind.into())]);
            let found = nlcs_candidates(coll, &f).iter().any(|n| crate::nlcs::object_by_name("RI", n).is_some());
            assert!(found, "{coll} {kind}: no NLCS object");
        }
    }
}
