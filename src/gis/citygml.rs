//! CityGML reader (2.0 and 3.0; IMGeo is CityGML 2.0 with an extension):
//! the city objects of a file with their surfaces, for import as 3D meshes.
//!
//! Per object: its theme from the element name (building, transport,
//! vegetation, water, land use, relief, bridge or tunnel), its surfaces as
//! rings of 2D or 3D points, the semantic part of each surface (roof, wall,
//! ground) where the file says so, and the year of construction and measured
//! height when given. Only the highest LoD of an object is kept, so a file
//! with LoD1 and LoD2 does not give each building twice.

use roxmltree::{Document, Node};
use quick_xml::{events::Event, Reader};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Theme {
    Building,
    Transport,
    Vegetation,
    Water,
    LandUse,
    Relief,
    Bridge,
    Other,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Theme::Building => "gebouwen",
            Theme::Transport => "wegen",
            Theme::Vegetation => "groen",
            Theme::Water => "water",
            Theme::LandUse => "terrein",
            Theme::Relief => "reliëf",
            Theme::Bridge => "kunstwerken",
            Theme::Other => "overig",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Roof,
    Wall,
    Ground,
    /// No semantics (an LoD1 solid, a 2D surface).
    Any,
}

/// Rings of a surface: the outer ring first, then holes. Points are x, y, z
/// (z = NaN for 2D data).
pub type Rings = Vec<Vec<[f64; 3]>>;

#[derive(Clone, Debug)]
pub struct CityObject {
    pub id: String,
    pub theme: Theme,
    /// The element name: `Building`, `TrafficArea`, `Waterdeel`, …
    pub class: String,
    pub year: Option<i64>,
    pub height: Option<f64>,
    pub surfaces: Vec<(Part, Rings)>,
    /// Function, class and physical appearance (CityGML function/class,
    /// IMGeo bgt-/plus-fysiekVoorkomen and -functie), lower case: for colours.
    pub look: String,
    /// All points 2D: drape on the ground.
    pub flat: bool,
}

#[derive(Debug, Default)]
pub struct CityModel {
    /// `EPSG:28992`, … from the file's srsName.
    pub crs: Option<String>,
    /// The objects kept.
    pub objects: Vec<CityObject>,
    /// All objects with geometry in the file, kept or not.
    pub seen: usize,
    /// Objects or sequence records skipped while preserving readable ones.
    pub warnings: Vec<String>,
}

impl CityModel {
    pub fn report(&self, message: String) -> String {
        if self.warnings.is_empty() { message } else { format!("{message} {}", self.warnings.join(" ")) }
    }
}

/// EPSG code of a GML srsName (`urn:ogc:def:crs:EPSG::28992`, `EPSG:28992`,
/// `http://www.opengis.net/def/crs/EPSG/0/7415`). RD + NAP (7415) is RD.
pub fn epsg(srs: &str) -> Option<String> {
    let up = srs.to_ascii_uppercase();
    // German AdV names: urn:adv:crs:ETRS89_UTM32*DE_DHHN2016_NH, DE_DHDN_3GK3.
    if let Some(i) = up.find("ETRS89_UTM") {
        let zone: String = up[i + 10..].chars().take_while(|c| c.is_ascii_digit()).collect();
        return (!zone.is_empty()).then(|| format!("EPSG:258{zone}"));
    }
    if let Some(i) = up.find("DHDN_3GK") {
        return up[i + 8..].chars().next().and_then(|c| c.to_digit(10)).map(|z| format!("EPSG:{}", 31464 + z));
    }
    let digits: String = srs.rsplit(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty())?.to_string();
    let code = match digits.as_str() {
        "7415" => "28992".to_string(),
        d => d.to_string(),
    };
    up.contains("EPSG").then(|| format!("EPSG:{code}"))
}

pub(crate) fn theme_of(name: &str) -> Theme {
    match name {
        "Building" | "BuildingPart" | "Pand" => Theme::Building,
        "Road" | "Railway" | "Track" | "Square" | "TransportSquare" | "TrafficArea" | "AuxiliaryTrafficArea" | "TransportationComplex" | "Wegdeel" | "OndersteunendWegdeel" | "Spoor" => Theme::Transport,
        "PlantCover" | "SolitaryVegetationObject" | "BegroeidTerreindeel" | "Vegetatieobject" => Theme::Vegetation,
        "WaterBody" | "Waterway" | "WaterSurface" | "WaterGroundSurface" | "Waterdeel" | "OndersteunendWaterdeel" => Theme::Water,
        "LandUse" | "OnbegroeidTerreindeel" => Theme::LandUse,
        "ReliefFeature" | "TINRelief" | "MassPointRelief" => Theme::Relief,
        "Bridge" | "BridgePart" | "Tunnel" | "TunnelPart" | "Overbruggingsdeel" | "Kunstwerkdeel" | "Tunneldeel" => Theme::Bridge,
        _ => Theme::Other,
    }
}

fn local<'a, 'i>(n: &Node<'a, 'i>) -> &'i str {
    n.tag_name().name()
}

fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

/// The dimension of coordinates at `n`: its own or an ancestor's srsDimension.
fn dimension(n: Node, fallback: usize) -> usize {
    n.ancestors().find_map(|a| a.attribute("srsDimension").and_then(|v| v.trim().parse().ok())).unwrap_or(fallback)
}

/// The points of one ring element (posList, a series of pos, or coordinates).
fn ring(r: Node, fallback_dim: usize) -> Vec<[f64; 3]> {
    if let Some(pl) = r.descendants().find(|d| local(d) == "posList") {
        let v = numbers(pl.text().unwrap_or(""));
        let dim = dimension(pl, fallback_dim);
        return v.chunks_exact(dim).map(|c| [c[0], c[1], if dim >= 3 { c[2] } else { f64::NAN }]).collect();
    }
    let pos: Vec<Node> = r.descendants().filter(|d| local(d) == "pos").collect();
    if !pos.is_empty() {
        return pos
            .iter()
            .filter_map(|p| {
                let v = numbers(p.text().unwrap_or(""));
                (v.len() >= 2).then(|| [v[0], v[1], v.get(2).copied().unwrap_or(f64::NAN)])
            })
            .collect();
    }
    if let Some(cs) = r.descendants().find(|d| local(d) == "coordinates") {
        // x,y[,z] x,y[,z] …
        return cs
            .text()
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|t| {
                let v: Vec<f64> = t.split(',').filter_map(|s| s.parse().ok()).collect();
                (v.len() >= 2).then(|| [v[0], v[1], v.get(2).copied().unwrap_or(f64::NAN)])
            })
            .collect();
    }
    Vec::new()
}

/// Rings of a gml:Polygon, PolygonPatch or Triangle, the closing point off.
fn polygon(p: Node, dim: usize) -> Rings {
    let mut out = Vec::new();
    for side in ["exterior", "outerBoundaryIs", "interior", "innerBoundaryIs"] {
        for b in p.children().filter(|c| local(c) == side) {
            let mut r = ring(b, dim);
            if r.len() > 1 && r[0][0] == r[r.len() - 1][0] && r[0][1] == r[r.len() - 1][1] {
                r.pop();
            }
            if r.len() >= 3 {
                if side.starts_with("ext") || side.starts_with("outer") {
                    out.insert(0, r);
                } else {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// LoD of a polygon: the first ancestor named lod<n>…, 0 for IMGeo's 2D.
fn lod_of(p: Node, stop: Node) -> u8 {
    for a in p.ancestors() {
        if a == stop {
            break;
        }
        let n = local(&a);
        if let Some(rest) = n.strip_prefix("lod") {
            if let Some(d) = rest.chars().next().and_then(|c| c.to_digit(10)) {
                return d as u8;
            }
        }
    }
    0
}

fn part_of(p: Node, stop: Node) -> Part {
    for a in p.ancestors() {
        if a == stop {
            break;
        }
        match local(&a) {
            "RoofSurface" => return Part::Roof,
            "WallSurface" | "ClosureSurface" | "OuterCeilingSurface" | "InteriorWallSurface" => return Part::Wall,
            "GroundSurface" | "OuterFloorSurface" | "FloorSurface" => return Part::Ground,
            _ => {}
        }
    }
    Part::Any
}

fn text_of(o: Node, name: &str) -> Option<String> {
    o.descendants().find(|d| local(d) == name).and_then(|d| d.text()).map(|t| t.trim().to_string())
}

/// One city object element (the child of a cityObjectMember).
fn object(o: Node, fallback_dim: usize) -> Option<CityObject> {
    let class = local(&o).to_string();
    let polys: Vec<Node> = o.descendants().filter(|d| matches!(local(d), "Polygon" | "PolygonPatch" | "Triangle")).collect();
    let top = polys.iter().map(|p| lod_of(*p, o)).max()?;
    let mut surfaces = Vec::new();
    for p in polys.iter().filter(|p| lod_of(**p, o) == top) {
        let rings = polygon(*p, fallback_dim);
        if !rings.is_empty() {
            surfaces.push((part_of(*p, o), rings));
        }
    }
    if surfaces.is_empty() {
        return None;
    }
    let flat = surfaces.iter().all(|(_, r)| r.iter().flatten().all(|p| p[2].is_nan()));
    let id = o.attributes().find(|a| a.name() == "id").map(|a| a.value().to_string()).or_else(|| text_of(o, "lokaalID")).unwrap_or_default();
    Some(CityObject {
        id,
        theme: theme_of(&class),
        year: text_of(o, "yearOfConstruction").and_then(|y| y.get(..4).and_then(|s| s.parse().ok())),
        height: text_of(o, "measuredHeight").and_then(|h| h.parse().ok()),
        look: o
            .descendants()
            .filter(|d| matches!(local(d), "function" | "class" | "usage" | "bgt-fysiekVoorkomen" | "plus-fysiekVoorkomen" | "bgt-functie" | "plus-functie" | "bgt-type"))
            .filter_map(|d| d.text())
            .map(|t| t.trim().to_lowercase())
            .collect::<Vec<_>>()
            .join(" "),
        class,
        surfaces,
        flat,
    })
}

/// The first value of an XML attribute anywhere in the text (`srsName`).
fn first_attribute(xml: &str, name: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) | Event::Empty(tag) => {
                for attr in tag.attributes() {
                    let attr = attr.ok()?;
                    if attr.key.as_ref() == name.as_bytes() {
                        return attr.decode_and_unescape_value(reader.decoder()).ok().map(|value| value.into_owned());
                    }
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The CRS of a CityGML text, without parsing it.
pub fn crs_of(xml: &str) -> Option<String> {
    first_attribute(xml, "srsName").as_deref().and_then(epsg)
}

/// The members of a CityGML text one by one: (start, end) of every
/// `<…cityObjectMember …>…</…cityObjectMember>` (or featureMember).
fn members(xml: &str) -> Result<(Vec<(usize, usize)>, &str), String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_comments = true;
    let mut out = Vec::new();
    let (mut depth, mut root_seen) = (0usize, false);
    let mut member: Option<(usize, usize)> = None;
    let mut attrs = "";
    loop {
        match reader.read_event().map_err(|e| format!("CityGML: ongeldige XML: {e}"))? {
            Event::Start(tag) => {
                let end = reader.buffer_position() as usize;
                let start = end - tag.len() - 2;
                for attr in tag.attributes() { attr.map_err(|e| format!("CityGML: ongeldig XML-attribuut: {e}"))?; }
                if depth == 0 {
                    if root_seen { return Err("CityGML: meerdere XML-hoofdelementen".into()); }
                    root_seen = true;
                    attrs = xml[start + 1..end - 1].split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or("");
                }
                depth += 1;
                if depth > 256 { return Err("CityGML: XML te diep genest".into()); }
                if member.is_none() && matches!(tag.local_name().as_ref(), b"cityObjectMember" | b"featureMember") {
                    member = Some((depth, start));
                }
            }
            Event::Empty(tag) => {
                for attr in tag.attributes() { attr.map_err(|e| format!("CityGML: ongeldig XML-attribuut: {e}"))?; }
                if depth == 0 {
                    if root_seen { return Err("CityGML: meerdere XML-hoofdelementen".into()); }
                    root_seen = true;
                }
            }
            Event::End(_) => {
                if member.is_some_and(|(d, _)| d == depth) {
                    let (_, start) = member.take().unwrap();
                    out.push((start, reader.buffer_position() as usize));
                }
                depth = depth.checked_sub(1).ok_or("CityGML: onverwachte XML-eindtag")?;
            }
            Event::Text(text) if depth == 0 && !text.as_ref().iter().all(u8::is_ascii_whitespace) => return Err("CityGML: tekst buiten XML-hoofdelement".into()),
            Event::CData(_) | Event::GeneralRef(_) if depth == 0 => return Err("CityGML: inhoud buiten XML-hoofdelement".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || depth != 0 { return Err("CityGML: leeg of afgebroken XML-document".into()); }
    Ok((out, attrs))
}

fn validate_coordinates(root: Node, fallback_dim: usize) -> Result<(), String> {
    let numbers = |text: &str| -> Result<Vec<f64>, String> {
        text.split_whitespace().map(|t| {
            let v = t.parse::<f64>().map_err(|_| format!("CityGML: ongeldig coördinaatgetal '{t}'"))?;
            if !v.is_finite() { return Err(format!("CityGML: niet-eindig coördinaatgetal '{t}'")); }
            Ok(v)
        }).collect()
    };
    for n in root.ancestors().chain(root.descendants()).filter(|n| n.is_element()) {
        if let Some(dim) = n.attribute("srsDimension") {
            if !dim.trim().parse::<usize>().is_ok_and(|d| d >= 2) { return Err(format!("CityGML: srsDimension '{dim}' kan niet naar XYZ worden gelezen")); }
        }
        match local(&n) {
            "posList" => {
                let values = numbers(n.text().unwrap_or(""))?;
                let dim = dimension(n, fallback_dim);
                if dim < 2 || values.is_empty() || values.len() % dim != 0 { return Err("CityGML: onvolledige posList".into()); }
            }
            "pos" => {
                let values = numbers(n.text().unwrap_or(""))?;
                let dim = n.ancestors().find_map(|a| a.attribute("srsDimension"));
                if values.len() < 2 || dim.is_some_and(|d| d.trim().parse::<usize>().ok() != Some(values.len())) { return Err("CityGML: onvolledige pos-coördinaat".into()); }
            }
            "coordinates" => {
                let mut dim = None;
                for tuple in n.text().unwrap_or("").split_whitespace() {
                    let values = numbers(&tuple.replace(',', " "))?;
                    let expected = *dim.get_or_insert(dimension(n, values.len()));
                    if expected < 2 || values.len() != expected || tuple.split(',').any(str::is_empty) { return Err("CityGML: onvolledige coordinates-tuple".into()); }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Read a CityGML text member by member, so a file of a whole city does
/// not have to fit in memory as one XML tree. `keep` decides per object
/// (e.g. only those in the project area).
pub fn parse_with(xml: &str, keep: &dyn Fn(&CityObject) -> bool, progress: &dyn Fn(String)) -> Result<CityModel, String> {
    let mut model = CityModel { crs: crs_of(xml), ..Default::default() };
    // The namespace declarations of the root element, for every member.
    let (list, attrs) = members(xml)?;
    // Only global attributes/bounds supply a fallback. A local dimension on
    // one city object must not change how its neighbours are interpreted.
    let global_end = list.first().map(|(s, _)| *s).unwrap_or(xml.len());
    let fallback_dim = match first_attribute(&xml[..global_end], "srsDimension") {
        Some(v) => v.trim().parse::<usize>().ok().filter(|d| *d >= 2)
            .ok_or_else(|| format!("CityGML: globale srsDimension '{v}' kan niet naar XYZ worden gelezen"))?,
        None => 3,
    };
    let mut seen = 0usize;
    for (k, (s, e)) in list.iter().enumerate() {
        if k % 2000 == 0 && k > 0 {
            progress(format!("CityGML: {k} van {} objecten gelezen…", list.len()));
        }
        let chunk = format!("<wrap {attrs}>{}</wrap>", &xml[*s..*e]);
        let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        let doc = match Document::parse_with_options(&chunk, opts) {
            Ok(doc) => doc,
            Err(error) => { model.warnings.push(format!("CityGML: member {} overgeslagen: onleesbare XML: {error}", k + 1)); continue; }
        };
        for member in doc.root_element().children().filter(|c| c.is_element()) {
            for o in member.children().filter(|c| c.is_element()) {
                if let Err(error) = validate_coordinates(o, fallback_dim) {
                    let id = o.attributes().find(|a| a.name() == "id").map(|a| a.value()).unwrap_or("zonder id");
                    model.warnings.push(format!("CityGML: object {id} overgeslagen: {error}"));
                    continue;
                }
                if let Some(obj) = object(o, fallback_dim) {
                    seen += 1;
                    if keep(&obj) {
                        model.objects.push(obj);
                    }
                }
            }
        }
    }
    model.seen = seen;
    if seen == 0 {
        return Err(model.report("CityGML: geen stadsobjecten met geometrie gevonden".into()));
    }
    Ok(model)
}

/// Read a CityGML document (all objects).
pub fn parse(xml: &str) -> Result<CityModel, String> {
    parse_with(xml, &|_| true, &|_| {})
}

/// Read a CityGML or CityJSON file (`.gml`, `.xml`, `.json`, `.city.json`,
/// CityJSONSeq `.jsonl`): the CRS first, then `keep(crs)` gives the filter.
pub fn read_file(path: &std::path::Path, keep: &dyn Fn(Option<&str>) -> Result<Box<dyn Fn(&CityObject) -> bool>, String>, progress: &dyn Fn(String)) -> Result<CityModel, String> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    progress(format!("{} lezen ({:.0} MB)…", path.display(), size as f64 / 1e6));
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json = text.trim_start().starts_with('{');
    let crs = if json { super::cityjson::crs_of(&text) } else { crs_of(&text) };
    let filter = keep(crs.as_deref())?;
    if json {
        super::cityjson::parse_with(&text, &*filter, progress)
    } else {
        parse_with(&text, &*filter, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILDING: &str = include_str!("../../tests/fixtures/citygml/building.gml");

    fn with_coordinates(element: &str) -> String {
        let doc = Document::parse(BUILDING).unwrap();
        let list = doc.descendants().find(|n| n.is_element() && local(n) == "posList").unwrap();
        let mut text = BUILDING.to_string();
        text.replace_range(list.range(), element);
        text
    }

    fn with_neighbour(member_source: &str) -> String {
        let doc = Document::parse(member_source).unwrap();
        let member = doc.descendants().find(|n| n.is_element() && local(n) == "cityObjectMember").unwrap();
        let neighbour = member_source[member.range()].replace("synthetic-building", "neighbour");
        BUILDING.replace("</core:CityModel>", &format!("{neighbour}</core:CityModel>"))
    }

    fn with_lod0_coordinates(element: &str) -> String {
        let footprint = format!("<bldg:lod0FootPrint><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing>{element}</gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod0FootPrint>");
        BUILDING.replace("<bldg:boundedBy>", &format!("{footprint}<bldg:boundedBy>"))
    }

    #[test]
    fn sol_lod0_semicolon_coordinates_preserve_lod2_roof() {
        let expected = parse(BUILDING).unwrap();
        let text = with_lod0_coordinates("<gml:coordinates cs=\";\">2600000;1200000;0 2600010;1200000;0 2600010;1200010;0 2600000;1200010;0 2600000;1200000;0</gml:coordinates>");
        let model = parse(&text).unwrap();
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].surfaces[0].1[0].len(), 4);
        assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
        assert!(model.warnings.is_empty());
    }

    #[test]
    fn sol_lod0_custom_decimal_and_tuple_separator_preserve_lod2_roof() {
        let expected = parse(BUILDING).unwrap();
        let text = with_lod0_coordinates("<gml:coordinates decimal=\",\" cs=\";\" ts=\"|\">2600000,5;1200000,5;0|2600010,5;1200000,5;0|2600010,5;1200010,5;0|2600000,5;1200010,5;0|2600000,5;1200000,5;0</gml:coordinates>");
        let model = parse(&text).unwrap();
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].surfaces[0].1[0].len(), 4);
        assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
        assert!(model.warnings.is_empty());
    }

    #[test]
    fn citygml_selected_gml2_coordinates_use_the_declared_separators() {
        let expected = parse(BUILDING).unwrap();
        let values = ["2600000 1200000 5", "2600010 1200000 5", "2600010 1200010 5", "2600000 1200010 5", "2600000 1200000 5"];
        for (attrs, cs, ts, decimal) in [("", ",", " ", "."), (" cs=\";\"", ";", " ", "."), (" decimal=\",\" cs=\";\" ts=\"|\"", ";", "|", ",")] {
            let text = values.iter().map(|p| p.split_whitespace().map(|v| if decimal == "," { format!("{v},0") } else { v.to_string() }).collect::<Vec<_>>().join(cs)).collect::<Vec<_>>().join(ts);
            let model = parse(&with_coordinates(&format!("<gml:coordinates{attrs}>{text}</gml:coordinates>"))).unwrap();
            assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces, "{attrs}");
        }
    }

    #[test]
    fn citygml_reads_every_text_part_around_comments_and_cdata() {
        let expected = parse(BUILDING).unwrap();
        for (from, to) in [
            ("2600010 1200010 5", "2600010 1200010 5<!--comment-->"),
            ("2600010 1200010 5", "26000<!--number-->10 1200010 5"),
            ("2600010 1200010 5", "<![CDATA[2600010 1200010 5]]>"),
        ] {
            let text = BUILDING.replace(from, to);
            let model = parse(&text).unwrap();
            assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces, "{to}");
        }
    }

    #[test]
    fn sol_roof_comment_after_third_tuple_keeps_four_corners() {
        let text = with_coordinates("<gml:posList>2600000 1200000 5 2600010 1200000 5 2600010 1200010 5<!--comment--> 2600000 1200010 5 2600000 1200000 5</gml:posList>");
        let model = parse(&text).unwrap();
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].surfaces[0].1[0].len(), 4);
        assert_eq!(model.objects[0].surfaces, parse(BUILDING).unwrap().objects[0].surfaces);
        assert!(model.warnings.is_empty());
    }

    #[test]
    fn sol_bad_after_roof_comment_rejects_only_that_object() {
        let bad = with_coordinates("<gml:posList>2600000 1200000 5 2600010 1200000 5 2600010 1200010 5<!--comment--> BAD 2600000 1200010 5 2600000 1200000 5</gml:posList>");
        assert!(parse(&bad).is_err(), "BAD achter commentaar mag geen gedeeltelijk dak opleveren");
        let model = parse(&with_neighbour(&bad)).unwrap();
        assert_eq!((model.seen, model.objects.len(), model.warnings.len()), (1, 1, 1));
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(model.objects[0].surfaces, parse(BUILDING).unwrap().objects[0].surfaces);
    }

    #[test]
    fn citygml_bad_xml_in_one_member_keeps_the_other_member() {
        let bad = BUILDING.replace("2600010 1200010 5", "2600010 &unknown; 5");
        // with_neighbour parses XML to find its boundaries, so insert the
        // unknown reference only after copying the well-formed member.
        let text = with_neighbour(BUILDING).replace("gml:id=\"neighbour\">", "gml:id=\"neighbour\"><bldg:name>&unknown;</bldg:name>");
        let model = parse(&text).unwrap();
        assert_eq!((model.objects.len(), model.warnings.len()), (1, 1));
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(parse(&bad).unwrap_err(), "CityGML: geen stadsobjecten met geometrie gevonden");
    }

    #[test]
    fn citygml_main_compatible_member_isolation() {
        let expected = parse(BUILDING).unwrap();
        let text = with_neighbour(BUILDING).replace("gml:id=\"neighbour\">", "gml:id=\"neighbour\"><bldg:name>&unknown;</bldg:name>");
        let model = parse(&text).unwrap();
        assert_eq!(model.objects.len(), 1);
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
    }

    #[test]
    fn citygml_legacy_comma_separated_poslist_keeps_main_coordinates() {
        let expected = parse(BUILDING).unwrap();
        let doc = Document::parse(BUILDING).unwrap();
        let list = doc.descendants().find(|n| n.is_element() && local(n) == "posList").unwrap();
        let text = with_coordinates(&format!("<gml:posList>{}</gml:posList>", list.text().unwrap().replace(' ', ",")));
        let model = parse(&text).unwrap();
        assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
    }

    #[test]
    fn sol_duplicate_gml_id_in_second_building_keeps_first() {
        let text = with_neighbour(BUILDING).replace("gml:id=\"neighbour\">", "gml:id=\"neighbour\" gml:id=\"duplicate\">");
        let model = parse(&text).unwrap();
        assert_eq!((model.objects.len(), model.warnings.len()), (1, 1));
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(model.objects[0].surfaces, parse(BUILDING).unwrap().objects[0].surfaces);
    }

    #[test]
    fn sol_missing_attribute_equals_in_second_building_keeps_first() {
        let text = with_neighbour(BUILDING).replace("gml:id=\"neighbour\"", "gml:id \"neighbour\"");
        assert!(text.contains("gml:id \"neighbour\""));
        let model = parse(&text).unwrap();
        assert_eq!((model.objects.len(), model.warnings.len()), (1, 1));
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(model.objects[0].surfaces, parse(BUILDING).unwrap().objects[0].surfaces);
    }

    #[test]
    fn citygml_file_route_keeps_valid_objects_and_internal_skip_details() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/citygml/mixed.gml");
        let progress = std::cell::RefCell::new(Vec::new());
        let model = read_file(&path, &|crs| {
            assert_eq!(crs, Some("EPSG:2056"));
            Ok(Box::new(|_| true))
        }, &|message| progress.borrow_mut().push(message)).unwrap();
        assert_eq!((model.seen, model.objects.len(), model.warnings.len()), (1, 1, 2));
        assert_eq!(model.objects[0].id, "synthetic-building");
        assert_eq!(model.objects[0].surfaces[0].1[0].len(), 4);
        assert!(model.warnings[0].contains("broken-coordinates"));
        assert!(model.warnings[1].contains("onleesbare XML"));
        // Only the existing file-reading progress is visible. Per-object
        // diagnostics stay in the result pending Ethan's wording decision.
        assert_eq!(progress.borrow().len(), 1);
        assert!(progress.borrow()[0].contains("lezen"));
    }

    #[test]
    fn citygml_document_errors_use_the_existing_empty_geometry_error() {
        for text in [
            BUILDING.replace("<core:cityObjectMember>", "&unknown;<core:cityObjectMember>"),
            format!("{BUILDING}<?xml version=\"1.0\"?>"),
        ] {
            assert_eq!(parse(&text).unwrap_err(), "CityGML: geen stadsobjecten met geometrie gevonden");
        }
    }

    #[test]
    fn citygml_declared_and_builtin_entities_outside_members_remain_valid() {
        let named = BUILDING.replace("<core:CityModel", "<!DOCTYPE core:CityModel [<!ENTITY outside \"metadata\">]><core:CityModel")
            .replace("<core:cityObjectMember>", "<gml:name>&outside; &amp; &#65;</gml:name><core:cityObjectMember>");
        assert_eq!(parse(&named).unwrap().objects.len(), 1);
    }

    #[test]
    fn citygml_whitespace_and_irrelevant_dimension_metadata_keep_the_roof() {
        let expected = parse(BUILDING).unwrap();
        for dimension in [" 3 ", "unknown", "bad", "0"] {
            let model = parse(&BUILDING.replace("srsDimension=\"3\"", &format!("srsDimension=\"{dimension}\""))).unwrap();
            assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
        }
    }

    #[test]
    fn citygml_outer_containers_do_not_impose_a_geometry_depth_limit() {
        let text = BUILDING.replace("<core:cityObjectMember>", &format!("{}<core:cityObjectMember>", "<group>".repeat(300)))
            .replace("</core:cityObjectMember>", &format!("</core:cityObjectMember>{}", "</group>".repeat(300)));
        let model = parse(&text).unwrap();
        assert_eq!(model.objects[0].id, "synthetic-building");
    }

    #[test]
    fn citygml_reads_comments_in_pos_and_coordinates() {
        let expected = parse(BUILDING).unwrap();
        let values = ["2600000 1200000 5", "2600010 1200000 5", "2600010 1200010 5", "2600000 1200010 5", "2600000 1200000 5"];
        let pos = values.iter().map(|v| format!("<gml:pos>{}</gml:pos>", v.replace("2600010", "26000<!--number-->10"))).collect::<String>();
        let coords = format!("<gml:coordinates>{}</gml:coordinates>", values.iter().map(|v| v.replace(' ', ",")).collect::<Vec<_>>().join(" ").replace("2600010,1200010,5", "2600010,1200010,5<!--comment-->"));
        for element in [pos, coords] {
            let model = parse(&with_coordinates(&element)).unwrap();
            assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
        }
    }

    #[test]
    fn citygml_bad_member_keeps_the_valid_adjacent_building() {
        let doc = Document::parse(BUILDING).unwrap();
        let member = doc.descendants().find(|n| n.is_element() && local(n) == "cityObjectMember").unwrap();
        let broken = BUILDING[member.range()].replace("synthetic-building", "broken-building").replace("2600010 1200010 5", "2600010 bad 5");
        let text = BUILDING.replace("</core:CityModel>", &format!("{broken}</core:CityModel>"));
        let data = parse(&text).unwrap();
        assert_eq!((data.seen, data.objects.len()), (1, 1));
        assert_eq!(data.objects[0].id, "synthetic-building");
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("broken-building overgeslagen"));
    }

    #[test]
    fn citygml_projects_complete_four_dimensional_tuples_to_xyz() {
        let doc = Document::parse(BUILDING).unwrap();
        let list = doc.descendants().find(|n| n.is_element() && local(n) == "posList").unwrap();
        let values = list.text().unwrap().split_whitespace().map(|v| v.parse::<f64>().unwrap()).collect::<Vec<_>>().chunks_exact(3).map(|p| format!("{} {} {} 42", p[0], p[1], p[2])).collect::<Vec<_>>().join(" ");
        let text = BUILDING.replace("srsDimension=\"3\"", "srsDimension=\"4\"").replace(list.text().unwrap(), &values)
            .replace("<gml:lowerCorner>2600000 1200000 0</gml:lowerCorner>", "<gml:lowerCorner>2600000 1200000 0 42</gml:lowerCorner>")
            .replace("<gml:upperCorner>2600010 1200010 5</gml:upperCorner>", "<gml:upperCorner>2600010 1200010 5 42</gml:upperCorner>");
        let data = parse(&text).unwrap();
        assert_eq!(data.objects[0].surfaces[0].1[0][2], [2600010.0, 1200010.0, 5.0]);
    }

    #[test]
    fn fixture_preserves_versions_crs_geometry_and_filtering() {
        for version in ["1.0", "2.0", "3.0"] {
            let text = BUILDING.replace("/2.0", &format!("/{version}"));
            let model = parse(&text).unwrap();
            assert_eq!(model.crs.as_deref(), Some("EPSG:2056"));
            assert_eq!((model.seen, model.objects.len()), (1, 1));
            assert_eq!(model.objects[0].surfaces[0].0, Part::Roof);
            assert_eq!(model.objects[0].surfaces[0].1[0].len(), 4);
            assert_eq!(model.objects[0].surfaces[0].1[0][2], [2600010.0, 1200010.0, 5.0]);
            let filtered = parse_with(&text, &|_| false, &|_| {}).unwrap();
            assert_eq!((filtered.seen, filtered.objects.len()), (1, 0));
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/citygml/building.gml");
        let model = read_file(&path, &|crs| {
            assert_eq!(crs, Some("EPSG:2056"));
            Ok(Box::new(|_| true))
        }, &|_| {}).unwrap();
        assert_eq!(model.objects[0].id, "synthetic-building");
    }

    #[test]
    fn citygml_rejects_broken_xml_instead_of_partial_success() {
        for text in [
            BUILDING.replace("</core:CityModel>", ""),
            BUILDING.replace("</core:CityModel>", "<core:cityObjectMember>"),
            BUILDING.replace("</core:CityModel>", "<core:cityObjectMember><bldg:Building></core:cityObjectMember></core:CityModel>"),
            format!("{BUILDING}<extra/>"),
            format!("{BUILDING}trailing text"),
        ] {
            assert!(parse(&text).unwrap_err().starts_with("CityGML:"));
        }
    }

    #[test]
    fn citygml_ignores_member_names_in_comments_and_cdata() {
        let commented = BUILDING.replace("cityObjectMember", "featureMember").replace("<gml:boundedBy>", "<!-- cityObjectMember -->\n<gml:boundedBy>");
        assert_eq!(parse(&commented).unwrap().objects.len(), 1);
        let cdata = BUILDING.replace("<bldg:boundedBy>", "<bldg:name><![CDATA[</core:cityObjectMember>]]></bldg:name><bldg:boundedBy>");
        assert_eq!(parse(&cdata).unwrap().objects.len(), 1);
    }

    #[test]
    fn citygml_accepts_single_quotes_and_whitespace_around_crs_attributes() {
        let text = BUILDING.replace("srsName=\"urn:ogc:def:crs:EPSG::2056\"", "srsName = 'urn:ogc:def:crs:EPSG::2056'");
        assert_eq!(crs_of(&text).as_deref(), Some("EPSG:2056"));
        assert_eq!(parse(&text).unwrap().crs.as_deref(), Some("EPSG:2056"));
    }

    #[test]
    fn citygml_rejects_invalid_numbers_and_coordinate_dimensions() {
        for (from, to) in [
            ("2600010 1200010 5", "2600010 bad 1200010 5"),
            ("2600010 1200010 5", "2600010 1200010"),
            ("2600010 1200010 5", "2600010 NaN 5"),
            ("2600010 1200010 5", "2600010 inf 5"),
        ] {
            assert!(parse(&BUILDING.replace(from, to)).is_err(), "{from} -> {to}");
        }
        for dimension in [0, 1] {
            let text = BUILDING.replace("<gml:posList>", &format!("<gml:posList srsDimension=\"{dimension}\">"));
            assert_eq!(parse(&text).unwrap_err(), "CityGML: geen stadsobjecten met geometrie gevonden");
        }
        let huge = parse(&BUILDING.replace("2600010", "1e100")).unwrap();
        assert_eq!(huge.objects[0].surfaces[0].1[0][2][0], 1e100);
    }

    #[test]
    fn citygml_reports_empty_and_truncated_input() {
        for text in ["", "<CityModel/>", "<", &BUILDING[..BUILDING.len() / 2]] {
            assert!(parse(text).is_err());
        }
    }

    #[test]
    fn citygml_unknown_bounds_and_optional_geometry_preserve_the_building() {
        let expected = parse(BUILDING).unwrap();
        let doc = Document::parse(BUILDING).unwrap();
        let bounds = doc.descendants().find(|n| n.is_element() && local(n) == "boundedBy" && n.tag_name().namespace() == Some("http://www.opengis.net/gml")).unwrap();
        let mut unknown_bounds = BUILDING.to_string();
        unknown_bounds.replace_range(bounds.range(), "<gml:boundedBy xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\" nilReason=\"unknown\"/>");
        assert_eq!(parse(&unknown_bounds).unwrap().crs, None);
        let unknown_lod = BUILDING.replace("<bldg:boundedBy>", "<bldg:lod1Solid nilReason=\"unknown\"/><bldg:boundedBy>");
        for text in [unknown_bounds, unknown_lod] {
            let model = parse(&text).unwrap();
            assert_eq!((model.seen, model.objects.len()), (1, 1));
            assert_eq!(model.objects[0].id, expected.objects[0].id);
            assert_eq!(model.objects[0].surfaces, expected.objects[0].surfaces);
            assert_eq!((model.objects[0].height, model.objects[0].year), (None, None));
        }
    }

    const LOD2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<core:CityModel xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:gml="http://www.opengis.net/gml">
 <gml:boundedBy><gml:Envelope srsName="urn:ogc:def:crs:EPSG::7415" srsDimension="3"><gml:lowerCorner>0 0 0</gml:lowerCorner><gml:upperCorner>10 10 9</gml:upperCorner></gml:Envelope></gml:boundedBy>
 <core:cityObjectMember>
  <bldg:Building gml:id="b1">
   <bldg:yearOfConstruction>1931</bldg:yearOfConstruction>
   <bldg:measuredHeight>9.0</bldg:measuredHeight>
   <bldg:lod1Solid><gml:Solid><gml:exterior><gml:CompositeSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 0 6 10 0 6 10 10 6 0 10 6 0 0 6</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:CompositeSurface></gml:exterior></gml:Solid></bldg:lod1Solid>
   <bldg:boundedBy><bldg:RoofSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>0 0 6 10 0 6 10 5 9 0 5 9 0 0 6</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:RoofSurface></bldg:boundedBy>
   <bldg:boundedBy><bldg:WallSurface><bldg:lod2MultiSurface><gml:MultiSurface><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:pos>0 0 0</gml:pos><gml:pos>10 0 0</gml:pos><gml:pos>10 0 6</gml:pos><gml:pos>0 0 6</gml:pos><gml:pos>0 0 0</gml:pos></gml:LinearRing></gml:exterior></gml:Polygon></gml:surfaceMember></gml:MultiSurface></bldg:lod2MultiSurface></bldg:WallSurface></bldg:boundedBy>
  </bldg:Building>
 </core:cityObjectMember>
</core:CityModel>"#;

    #[test]
    fn building_lod2_with_semantics() {
        let m = parse(LOD2).unwrap();
        assert_eq!(m.crs.as_deref(), Some("EPSG:28992"));
        assert_eq!(m.objects.len(), 1);
        let b = &m.objects[0];
        assert_eq!((b.theme, b.year, b.height, b.flat), (Theme::Building, Some(1931), Some(9.0), false));
        // LoD2 only (the LoD1 solid is left out), roof and wall.
        assert_eq!(b.surfaces.len(), 2);
        assert_eq!(b.surfaces[0].0, Part::Roof);
        assert_eq!(b.surfaces[1].0, Part::Wall);
        assert_eq!(b.surfaces[0].1[0].len(), 4);
    }

    #[test]
    fn imgeo_2d_and_srs_names() {
        let xml = r#"<cit:CityModel xmlns:cit="http://www.opengis.net/citygml/2.0" xmlns:tra="http://www.opengis.net/citygml/transportation/2.0" xmlns:imgeo="http://www.geostandaarden.nl/imgeo/2.1" xmlns:gml="http://www.opengis.net/gml">
<cit:cityObjectMember><tra:TrafficArea gml:id="w1"><imgeo:identificatie><imgeo:NEN3610ID><imgeo:lokaalID>G0505.abc</imgeo:lokaalID></imgeo:NEN3610ID></imgeo:identificatie>
<imgeo:geometrie2dWegdeel><gml:Polygon srsName="urn:ogc:def:crs:EPSG::28992"><gml:exterior><gml:LinearRing><gml:posList srsDimension="2">0 0 10 0 10 10 0 10 0 0</gml:posList></gml:LinearRing></gml:exterior>
<gml:interior><gml:LinearRing><gml:posList srsDimension="2">2 2 4 2 4 4 2 2</gml:posList></gml:LinearRing></gml:interior></gml:Polygon></imgeo:geometrie2dWegdeel></tra:TrafficArea></cit:cityObjectMember></cit:CityModel>"#;
        let m = parse(xml).unwrap();
        let w = &m.objects[0];
        assert_eq!((w.theme, w.flat, w.id.as_str()), (Theme::Transport, true, "w1"));
        assert_eq!(w.surfaces[0].1.len(), 2, "outer ring and a hole");
        assert_eq!(epsg("http://www.opengis.net/def/crs/EPSG/0/7415").as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg("EPSG:4326").as_deref(), Some("EPSG:4326"));
        assert_eq!(epsg("urn:adv:crs:ETRS89_UTM32*DE_DHHN2016_NH").as_deref(), Some("EPSG:25832"));
        assert_eq!(epsg("urn:adv:crs:DE_DHDN_3GK3*DE_DHHN92_NH").as_deref(), Some("EPSG:31467"));
        assert!(parse("<a/>").is_err());
    }

    /// Real files: `CITY_FILES=a.gml:b.city.json cargo test --release city_files -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn city_files() {
        for f in std::env::var("CITY_FILES").unwrap_or_default().split(':').filter(|f| !f.is_empty()) {
            let t = std::time::Instant::now();
            let m = read_file(std::path::Path::new(f), &|_| Ok(Box::new(|_| true)), &|_| {}).unwrap();
            let mut themes = std::collections::BTreeMap::new();
            let (mut surfaces, mut points, mut years) = (0, 0, 0);
            for o in &m.objects {
                *themes.entry(format!("{:?}", o.theme)).or_insert(0) += 1;
                surfaces += o.surfaces.len();
                points += o.surfaces.iter().flat_map(|(_, r)| r.iter()).map(Vec::len).sum::<usize>();
                years += o.year.is_some() as usize;
            }
            let roofs = m.objects.iter().flat_map(|o| &o.surfaces).filter(|(p, _)| *p == Part::Roof).count();
            println!("{f}: {:?}, {} objects ({themes:?}), {surfaces} surfaces ({roofs} roofs), {points} points, {years} with year, {:.1} s", m.crs, m.objects.len(), t.elapsed().as_secs_f64());
        }
    }
}
