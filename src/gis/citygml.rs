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
    n.ancestors().find_map(|a| a.attribute("srsDimension").and_then(|v| v.parse().ok())).unwrap_or(fallback)
}

/// The points of one ring element (posList, a series of pos, or coordinates).
fn ring(r: Node, fallback_dim: usize) -> Vec<[f64; 3]> {
    if let Some(pl) = r.descendants().find(|d| local(d) == "posList") {
        let v = numbers(pl.text().unwrap_or(""));
        let dim = dimension(pl, fallback_dim).clamp(2, 3);
        return v.chunks_exact(dim).map(|c| [c[0], c[1], if dim == 3 { c[2] } else { f64::NAN }]).collect();
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
fn first_attribute<'t>(xml: &'t str, name: &str) -> Option<&'t str> {
    let pat = format!("{name}=\"");
    let i = xml.find(&pat)? + pat.len();
    xml[i..].find('"').map(|j| &xml[i..i + j])
}

/// The CRS of a CityGML text, without parsing it.
pub fn crs_of(xml: &str) -> Option<String> {
    first_attribute(xml, "srsName").and_then(epsg)
}

/// The members of a CityGML text one by one: (start, end) of every
/// `<…cityObjectMember …>…</…cityObjectMember>` (or featureMember).
fn members(xml: &str) -> Vec<(usize, usize)> {
    let pat = if xml.contains("cityObjectMember") { "cityObjectMember" } else { "featureMember" };
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = xml[pos..].find(pat).map(|i| i + pos) {
        let after = xml[i + pat.len()..].chars().next();
        // Walk back over a prefix to the '<' of an opening tag.
        let head = &xml[..i];
        let start = head.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
        let start = start.strip_suffix(':').map(|s| s.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')).unwrap_or(start);
        let opening = start.ends_with('<') && !start.ends_with("</") && matches!(after, Some('>') | Some(' ') | Some('\n') | Some('\r') | Some('\t') | Some('/'));
        if !opening {
            pos = i + pat.len();
            continue;
        }
        let s = start.len() - 1;
        let qname = &xml[s + 1..i + pat.len()];
        let close = format!("</{qname}>");
        match xml[i..].find(&close) {
            Some(j) => {
                let e = i + j + close.len();
                out.push((s, e));
                pos = e;
            }
            None => break,
        }
    }
    out
}

/// Read a CityGML text member by member, so a file of a whole city does
/// not have to fit in memory as one XML tree. `keep` decides per object
/// (e.g. only those in the project area).
pub fn parse_with(xml: &str, keep: &dyn Fn(&CityObject) -> bool, progress: &dyn Fn(String)) -> Result<CityModel, String> {
    let mut model = CityModel { crs: crs_of(xml), ..Default::default() };
    let fallback_dim = first_attribute(xml, "srsDimension").and_then(|v| v.parse().ok()).unwrap_or(3);
    // The namespace declarations of the root element, for every member.
    let root_start = xml.match_indices('<').map(|(i, _)| i).find(|&i| !matches!(xml.as_bytes().get(i + 1), Some(b'?') | Some(b'!'))).ok_or("CityGML: geen XML")?;
    let root_end = xml[root_start..].find('>').map(|j| root_start + j).ok_or("CityGML: geen XML")?;
    let root_tag = xml[root_start + 1..root_end].trim_end_matches('/');
    let attrs = root_tag.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or("");
    let list = members(xml);
    let mut seen = 0usize;
    for (k, (s, e)) in list.iter().enumerate() {
        if k % 2000 == 0 && k > 0 {
            progress(format!("CityGML: {k} van {} objecten gelezen…", list.len()));
        }
        let chunk = format!("<wrap {attrs}>{}</wrap>", &xml[*s..*e]);
        let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
        let doc = match Document::parse_with_options(&chunk, opts) {
            Ok(d) => d,
            Err(_) => continue,
        };
        for member in doc.root_element().children().filter(|c| c.is_element()) {
            for o in member.children().filter(|c| c.is_element()) {
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
        return Err("CityGML: geen stadsobjecten met geometrie gevonden".into());
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
