//! NLCS++ XML: read deliveries of every version (V11 with an operator root
//! such as `NLCSnetbeheerAlliander` and a `VersieNummer` element; V12 and
//! V12.1 with `NLCSnetbeheer` and an `AprojectReferentie`), write V12.1.
//!
//! Objects are read generically, so a field the model does not know is kept
//! (and reported by [`crate::check`]). Geometry keeps its Z.

use crate::model::{self, model};

#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Point([f64; 3]),
    Line(Vec<[f64; 3]>),
    /// Outer ring first, then holes.
    Polygon(Vec<Vec<[f64; 3]>>),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Object {
    /// The element name: `LSkabel`, `Gleiding`, `AprojectReferentie`, …
    pub kind: String,
    /// Fields in file order (empty values left out).
    pub attrs: Vec<(String, String)>,
    pub geom: Option<Geom>,
    /// `EPSG:28992`, `EPSG:7415`, … from the geometry's `srsName`.
    pub srs: Option<String>,
}

impl Object {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        match self.attrs.iter_mut().find(|(a, _)| a == k) {
            Some(slot) => slot.1 = v,
            None => self.attrs.push((k.to_string(), v)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Delivery {
    /// The root element's name.
    pub root: String,
    /// `11`, `12`, `12.1`.
    pub version: Option<String>,
    /// `Alliander`, `Enexis`, `Stedin` when the file tells.
    pub operator: Option<String>,
    pub project: Option<Object>,
    pub objects: Vec<Object>,
}

fn coords(text: &str, dim: usize) -> Vec<[f64; 3]> {
    let v: Vec<f64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    let dim = dim.clamp(2, 3);
    v.chunks_exact(dim).map(|c| [c[0], c[1], if dim == 3 { c[2] } else { 0.0 }]).collect()
}

fn dim_of(n: roxmltree::Node) -> usize {
    n.ancestors().find_map(|a| a.attribute("srsDimension")).and_then(|d| d.parse().ok()).unwrap_or(2)
}

fn geometry(n: roxmltree::Node) -> Option<(Geom, Option<String>)> {
    let srs = n.descendants().find_map(|d| d.attribute("srsName")).map(|s| {
        let code = s.rsplit([':', '/']).next().unwrap_or(s);
        format!("EPSG:{code}")
    });
    let g = n.descendants().filter(|d| d.is_element()).find(|d| matches!(d.tag_name().name(), "Point" | "LineString" | "Curve" | "Polygon" | "Surface" | "MultiCurve" | "MultiSurface" | "MultiPoint"))?;
    let lists = |root: roxmltree::Node| -> Vec<Vec<[f64; 3]>> {
        root.descendants()
            .filter(|d| d.is_element() && matches!(d.tag_name().name(), "posList" | "pos"))
            .map(|d| coords(d.text().unwrap_or(""), dim_of(d)))
            .collect()
    };
    let geom = match g.tag_name().name() {
        "Point" | "MultiPoint" => Geom::Point(*lists(g).first()?.first()?),
        "LineString" | "Curve" | "MultiCurve" => {
            let pts: Vec<[f64; 3]> = lists(g).into_iter().flatten().collect();
            (pts.len() >= 2).then_some(())?;
            Geom::Line(pts)
        }
        _ => {
            let mut rings = Vec::new();
            for side in ["exterior", "interior"] {
                for r in g.descendants().filter(|d| d.is_element() && d.tag_name().name() == side) {
                    let ring: Vec<[f64; 3]> = lists(r).into_iter().flatten().collect();
                    if ring.len() >= 3 {
                        rings.push(ring);
                    }
                }
            }
            (!rings.is_empty()).then_some(())?;
            Geom::Polygon(rings)
        }
    };
    Some((geom, srs))
}

fn object(n: roxmltree::Node) -> Object {
    let mut o = Object { kind: n.tag_name().name().to_string(), ..Default::default() };
    for c in n.children().filter(|c| c.is_element()) {
        let has_gml = c.descendants().any(|d| d.is_element() && d.tag_name().namespace() == Some("http://www.opengis.net/gml/3.2"));
        if has_gml {
            if let Some((g, srs)) = geometry(c) {
                o.geom = Some(g);
                o.srs = srs;
            }
            continue;
        }
        let v = c.text().unwrap_or("").trim();
        if !v.is_empty() {
            o.attrs.push((c.tag_name().name().to_string(), v.to_string()));
        }
    }
    o
}

/// The deepest element nesting of `text`, counted without parsing it.
/// roxmltree reads nested elements recursively, so this bounds the stack
/// the parse needs. Comments, CDATA, processing instructions and quoted
/// attribute values do not count. An entity in an internal DTD subset can
/// expand to markup, entity in entity up to ten levels deep, so each `<` in
/// that subset counts ten times on top of the body's nesting.
fn nesting(text: &str) -> usize {
    let b = text.as_bytes();
    // The index just past `pat` from `at`, or the end of the text.
    let past = |at: usize, pat: &[u8]| b.get(at..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)).map_or(b.len(), |p| at + p + pat.len());
    // The index just past the `>` that ends a tag, outside quoted values.
    let tag_end = |mut i: usize| {
        let mut quote = None;
        while let Some(&c) = b.get(i) {
            i += 1;
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (None, b'"' | b'\'') => quote = Some(c),
                (None, b'>') => break,
                _ => {}
            }
        }
        i
    };
    let (mut i, mut depth, mut deepest, mut dtd) = (0, 0usize, 0usize, 0usize);
    while let Some(p) = b.get(i..).and_then(|r| r.iter().position(|&c| c == b'<')) {
        let at = i + p;
        let rest = &b[at..];
        if rest.starts_with(b"<!--") {
            i = past(at + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = past(at + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = past(at + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            // A DOCTYPE: skip it with its internal subset (declarations
            // with quoted values, comments and processing instructions).
            let (mut j, mut quote, mut subset) = (at + 2, None, false);
            while let Some(&c) = b.get(j) {
                j += 1;
                match (quote, c) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), b'<') => dtd += 1,
                    (Some(_), _) => {}
                    (None, b'"' | b'\'') => quote = Some(c),
                    (None, b'<') if b[j..].starts_with(b"!--") => j = past(j + 3, b"-->"),
                    (None, b'<') if b[j..].starts_with(b"?") => j = past(j + 1, b"?>"),
                    (None, b'<') => dtd += 1,
                    (None, b'[') => subset = true,
                    (None, b']') => subset = false,
                    (None, b'>') if !subset => break,
                    _ => {}
                }
            }
            i = j;
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = tag_end(at + 2);
        } else {
            // A start tag; an empty element is parsed one level down too.
            i = tag_end(at + 1);
            deepest = deepest.max(depth + 1);
            if !b[..i].ends_with(b"/>") {
                depth += 1;
            }
        }
    }
    deepest.saturating_add(dtd.saturating_mul(10))
}

/// Parses `text`, on a thread with enough stack for its nesting: deep but
/// valid XML still reads, and too deep XML gives an error instead of
/// aborting the process with a stack overflow.
fn document(text: &str) -> Result<roxmltree::Document<'_>, String> {
    let parse = || {
        let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        roxmltree::Document::parse_with_options(text, opt).map_err(|e| format!("NLCS++: {e}"))
    };
    let depth = nesting(text);
    if depth <= 64 {
        return parse();
    }
    let too_deep = || format!("NLCS++: XML te diep genest ({depth} niveaus)");
    let stack = depth.checked_mul(32 * 1024).and_then(|s| s.checked_add(2 << 20)).ok_or_else(too_deep)?;
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new().stack_size(stack).spawn_scoped(scope, parse).map_err(|_| too_deep())?;
        handle.join().map_err(|_| "NLCS++: XML-parser gestopt".to_string())?
    })
}

/// Reads an NLCS++ XML delivery.
pub fn read(text: &str) -> Result<Delivery, String> {
    let text = text.trim_start_matches('\u{feff}');
    let doc = document(text)?;
    let root = doc.root_element();
    let name = root.tag_name().name();
    if !name.starts_with("NLCSnetbeheer") {
        return Err(format!("NLCS++: <{name}> is geen NLCSnetbeheer-levering"));
    }
    let mut d = Delivery { root: name.to_string(), ..Default::default() };
    d.version = root.attribute("VersieNummer").map(str::to_string);
    let schema = root.attributes().find(|a| a.name() == "schemaLocation").map(|a| a.value().to_string()).unwrap_or_default();
    d.operator = model::operator(&name["NLCSnetbeheer".len()..]).or_else(|| model::operator(&schema)).map(str::to_string);
    for c in root.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "VersieNummer" => d.version = c.text().map(|t| t.trim().to_string()),
            "featureMember" | "featureMembers" => {
                for f in c.children().filter(|f| f.is_element()) {
                    d.objects.push(object(f));
                }
            }
            "AprojectReferentie" => {
                let p = object(c);
                if d.operator.is_none() {
                    d.operator = p.get("Netbeheerder").and_then(model::operator).map(str::to_string);
                }
                d.project = Some(p);
            }
            _ => d.objects.push(object(c)),
        }
    }
    Ok(d)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// An `xs:ID`: an NCName (a letter first, then letters, digits, `_`, `-`, `.`).
pub fn xml_id(prefix: &str, s: &str) -> String {
    let body: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' }).collect();
    format!("{prefix}{body}")
}

fn pos(p: &[f64; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

fn gml(g: &Geom, srs: &str, id: &str) -> String {
    let attrs = format!("gml:id=\"{id}\" srsName=\"{srs}\" srsDimension=\"3\"");
    match g {
        Geom::Point(p) => format!("<gml:Point {attrs}><gml:pos>{}</gml:pos></gml:Point>", pos(p)),
        Geom::Line(v) => format!("<gml:LineString {attrs}><gml:posList>{}</gml:posList></gml:LineString>", v.iter().map(pos).collect::<Vec<_>>().join(" ")),
        Geom::Polygon(r) => {
            let ring = |side: &str, ring: &Vec<[f64; 3]>| {
                let mut ring = ring.clone();
                if ring.first() != ring.last() {
                    ring.push(ring[0]);
                }
                format!("<gml:{side}><gml:LinearRing><gml:posList>{}</gml:posList></gml:LinearRing></gml:{side}>", ring.iter().map(pos).collect::<Vec<_>>().join(" "))
            };
            let mut s = format!("<gml:Polygon {attrs}>");
            for (i, rg) in r.iter().enumerate() {
                s.push_str(&ring(if i == 0 { "exterior" } else { "interior" }, rg));
            }
            s.push_str("</gml:Polygon>");
            s
        }
    }
}

/// One object as V12.1 XML, its fields in the order of the model (unknown
/// fields are left out: the schema would reject them).
fn write_object(o: &Object, fields: &[model::Field], srs: &str, n: usize) -> String {
    let id = o.get("ID").map(str::to_string).unwrap_or_else(|| xml_id("H", o.get("Handle").unwrap_or(&n.to_string())));
    let mut s = format!("\t<{} gml:id=\"{}\">\n", o.kind, esc(&xml_id("f_", &id)));
    for f in fields {
        if f.kind == "geometry" {
            if let Some(g) = &o.geom {
                s.push_str(&format!("\t\t<{}>{}</{}>\n", f.name, gml(g, o.srs.as_deref().unwrap_or(srs), &xml_id("g_", &id)), f.name));
            }
            continue;
        }
        let v = if f.name == "ID" { Some(id.as_str()) } else { o.get(&f.name) };
        if let Some(v) = v {
            s.push_str(&format!("\t\t<{0}>{1}</{0}>\n", f.name, esc(v)));
        }
    }
    s.push_str(&format!("\t</{}>\n", o.kind));
    s
}

/// A V12.1 delivery for `operator`, objects in drawing order, coordinates
/// in `srs` (EPSG:28992 or EPSG:7415). Objects of kinds the model does not
/// know are skipped (and counted by the check).
pub fn write(d: &Delivery, operator: &str, srs: &str) -> String {
    let m = model();
    let op = m.netbeheerders.get(operator);
    let xsd = op.map(|o| o.xsd.as_str()).unwrap_or("NLCS_Netbeheer.xsd");
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    s.push_str(&format!(
        "<{root} xmlns=\"{ns}\" xmlns:gml=\"http://www.opengis.net/gml/3.2\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"{ns} https://schemas.mijnaansluiting.nl/V{v}/{xsd}\" gml:id=\"NLCSnetbeheer\">\n",
        root = m.root,
        ns = m.namespace,
        v = m.versie,
    ));
    if let Some(p) = &d.project {
        let mut p = p.clone();
        p.kind = "AprojectReferentie".into();
        if p.get("ID").is_none() {
            p.set("ID", "PROJECT");
        }
        s.push_str(&write_object(&p, &m.project, srs, 0));
    }
    for (i, o) in d.objects.iter().enumerate() {
        if let Some(def) = m.object(&o.kind) {
            s.push_str(&write_object(o, &def.velden, srs, i + 1));
        }
    }
    s.push_str(&format!("</{}>\n", m.root));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One object whose child is nested `depth` elements deep.
    fn nested(depth: usize) -> String {
        format!("<NLCSnetbeheer><LSkabel>{}{}</LSkabel></NLCSnetbeheer>", "<a>".repeat(depth), "</a>".repeat(depth))
    }

    #[test]
    fn nesting_counts_elements_only() {
        assert_eq!(nesting("<a><b/><c><d></d></c></a>"), 3);
        assert_eq!(nesting("<a x='>' y=\"/>\"><b/></a>"), 2);
        assert_eq!(nesting("<?xml version=\"1.0\"?><a><!-- <b><c> --><![CDATA[<b><c>]]><?pi <b> ?></a>"), 1);
        assert_eq!(nesting("<!DOCTYPE a SYSTEM \"a.dtd\"><a><b/></a>"), 2);
        // Markup in entities can expand anywhere: each `<` in the subset counts ten times.
        assert_eq!(nesting("<!DOCTYPE a [<!-- ]> --><!ENTITY e \"<b><c/></b>\">]><a>&e;</a>"), 1 + 10 * 4);
        assert_eq!(nesting("<!DOCTYPE a [<!ENTITY x 'it''s'><?pi ]> ?>]><a><b><c/></b></a>"), 3 + 10);
        assert_eq!(nesting(&nested(1000)), 1002);
    }

    #[test]
    fn deep_markup_from_an_entity_gets_an_answer() {
        let e = format!("{}{}", "<a>".repeat(20_000), "</a>".repeat(20_000));
        let text = format!("<!DOCTYPE NLCSnetbeheer [<!ENTITY e \"{e}\">]><NLCSnetbeheer><LSkabel>&e;</LSkabel></NLCSnetbeheer>");
        // Either a delivery or an error message; no abort.
        let objects = std::thread::Builder::new().stack_size(2 << 20).spawn(move || read(&text).map(|d| d.objects.len())).unwrap().join().unwrap();
        assert!(objects.as_ref().map_or_else(|e| e.contains("te diep"), |n| *n == 1), "{objects:?}");
    }

    #[test]
    fn deeply_nested_xml_does_not_overflow_the_stack() {
        // Host commands run on threads with the default 2 MiB stack. Without
        // a guard, roxmltree's recursion aborts the process here (SIGABRT).
        let text = nested(50_000);
        let objects = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || read(&text).map(|d| d.objects.len()))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(objects, Ok(1));
    }

    const V11: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<NLCSnetbeheerAlliander xmlns:gml="http://www.opengis.net/gml/3.2" xmlns="NS_NLCSnetbeheerAlliander">
  <VersieNummer>11</VersieNummer>
  <LSkabel>
    <Handle>561D</Handle>
    <Uitvoering>4x95Al 4x6Cu as35Cu</Uitvoering>
    <Status>BESTAAND</Status>
    <Bedrijfstoestand>IN BEDRIJF</Bedrijfstoestand>
    <Functie>LSOV</Functie>
    <GisId>0864322</GisId>
    <Geometry><gml:LineString srsDimension="3" srsName="EPSG:28992"><gml:posList>162047.1 498526.2 -0.6 162204.9 498648.7 -0.7</gml:posList></gml:LineString></Geometry>
  </LSkabel>
  <LSmof><Handle>5630</Handle><Status>NIEUW</Status><Bedrijfstoestand>IN BEDRIJF</Bedrijfstoestand>
    <Geometry><gml:Point srsName="EPSG:28992"><gml:pos>162204.9 498648.7</gml:pos></gml:Point></Geometry></LSmof>
</NLCSnetbeheerAlliander>"#;

    #[test]
    fn reads_v11() {
        let d = read(V11).unwrap();
        assert_eq!((d.version.as_deref(), d.operator.as_deref(), d.objects.len()), (Some("11"), Some("Alliander"), 2));
        let k = &d.objects[0];
        assert_eq!((k.kind.as_str(), k.get("Handle"), k.get("Functie")), ("LSkabel", Some("561D"), Some("LSOV")));
        assert_eq!(k.geom, Some(Geom::Line(vec![[162047.1, 498526.2, -0.6], [162204.9, 498648.7, -0.7]])));
        assert_eq!(k.srs.as_deref(), Some("EPSG:28992"));
        assert_eq!(d.objects[1].geom, Some(Geom::Point([162204.9, 498648.7, 0.0])));
    }

    #[test]
    fn writes_v12_1_and_reads_it_back() {
        let mut d = read(V11).unwrap();
        let mut p = Object { kind: "AprojectReferentie".into(), ..Default::default() };
        for (k, v) in [("Projectnummer", "P-1"), ("Netbeheerder", "Liander"), ("NaamAannemer", "Test"), ("Tekeningtype", "DEFINITIEF ONTWERP")] {
            p.set(k, v);
        }
        p.geom = Some(Geom::Polygon(vec![vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]]]));
        d.project = Some(p);
        let x = write(&d, "Alliander", "EPSG:28992");
        assert!(x.contains("NLCS_NetbeheerAllianderV12.1Import_Keuzelijst.xsd"));
        assert!(x.contains("<ID>H561D</ID>") && x.contains("gml:id=\"f_H561D\""));
        // The fields come in the order of the schema: ID after Handle, Uitvoering before Status.
        let k = x.find("<Handle>561D").unwrap();
        assert!(k < x.find("<ID>H561D").unwrap() && x.find("<Uitvoering>").unwrap() < x.find("<Status>BESTAAND").unwrap());
        let back = read(&x).unwrap();
        assert_eq!(back.version, None);
        assert_eq!(back.objects.len(), 2);
        assert_eq!(back.objects[0].geom, d.objects[0].geom, "Z kept");
        assert_eq!(back.project.unwrap().get("Projectnummer"), Some("P-1"));
        assert_eq!(back.operator.as_deref(), Some("Alliander"), "from the project's Netbeheerder");
    }

    #[test]
    fn ids_are_ncnames() {
        assert_eq!(xml_id("H", "5{6}D"), "H5_6_D");
    }
}
