//! NLCS design → BGT mutation as IMGeo-GML 2.1.1 (issue #10).
//!
//! Reads the NLCS layers of the drawing, maps them back to BGT object types
//! with the official NLCS-BGT 5.0 mapping, and writes the new, changed and
//! ended objects as IMGeo-GML 2.1.1 (CityGML 2.0 ADE): the file a bronhouder's
//! BGT package reads. The bronhouder delivers to the LV-BGT as StUF-Geo IMGeo
//! (through BRAVO); that step is theirs.
//!
//! Status of the layer decides what an object is:
//! - **N** (nieuw): plan topography (IMGeo plus-status `plan`), a new object;
//!   with `as_built` it counts as built (status R);
//! - **R** (revisie): as built: a changed object when it carries a BGT
//!   identification (`lokaal_id` from the BGT import), else a new one;
//! - **V** (te verwijderen): the BGT object ends (terminationDate);
//! - **B** (bestaand): unchanged; it only serves to fill in attributes and
//!   to check that the new objects fit;
//! - **T** and **X**: not for the BGT.
//!
//! An NLCS layer often stands for several BGT types and functions (asphalt is
//! a wegdeel, an ondersteunend wegdeel or an onbegroeid terreindeel). The
//! attributes come from, in this order: the object data of the BGT import,
//! object data given by hand (`objecttype`, `functie`, `fysiek_voorkomen`,
//! `type`, …), the existing BGT object the new surface covers most, and the
//! mapping itself where it is unambiguous. What stays ambiguous goes in the
//! report, not in the file.

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;
use std::fmt::Write as _;

use ocs_plugin_api::host::codec::entities::BoundaryEdge;
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use crate::gis::crs::{self, XY};
use crate::gis::site::Site;
use crate::nlcs;

pub struct Options {
    /// Bronhouder code (`G0505`); empty: the one of the BGT objects in the drawing.
    pub bronhouder: String,
    /// N layers are built (as R), not plan topography.
    pub as_built: bool,
    pub unit_factor: f64,
    /// Only these entities (the selection).
    pub only: Option<std::collections::HashSet<Handle>>,
    /// Date and time of the mutation: (`2026-10-28`, `2026-10-28T10:15:00`).
    pub now: (String, String),
}

impl Options {
    pub fn now() -> (String, String) {
        let t = chrono::Local::now();
        (t.format("%Y-%m-%d").to_string(), t.format("%Y-%m-%dT%H:%M:%S").to_string())
    }
}

// ── IMGeo classes ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
pub enum Class {
    Wegdeel,
    OndersteunendWegdeel,
    BegroeidTerreindeel,
    OnbegroeidTerreindeel,
    Waterdeel,
    OndersteunendWaterdeel,
    Pand,
    Scheiding,
    Paal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Surface,
    Point,
    /// Scheiding: a line under 30 cm wide, else a surface.
    LineOrSurface,
}

impl Class {
    const ALL: [Class; 9] = [
        Class::Wegdeel,
        Class::OndersteunendWegdeel,
        Class::BegroeidTerreindeel,
        Class::OnbegroeidTerreindeel,
        Class::Waterdeel,
        Class::OndersteunendWaterdeel,
        Class::Pand,
        Class::Scheiding,
        Class::Paal,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Wegdeel => "Wegdeel",
            Class::OndersteunendWegdeel => "OndersteunendWegdeel",
            Class::BegroeidTerreindeel => "BegroeidTerreindeel",
            Class::OnbegroeidTerreindeel => "OnbegroeidTerreindeel",
            Class::Waterdeel => "Waterdeel",
            Class::OndersteunendWaterdeel => "OndersteunendWaterdeel",
            Class::Pand => "Pand",
            Class::Scheiding => "Scheiding",
            Class::Paal => "Paal",
        }
    }

    /// The object name in the NLCS-BGT mapping.
    fn mapping(self) -> &'static str {
        match self {
            Class::Wegdeel => "WEGDEEL",
            Class::OndersteunendWegdeel => "ONDERSTEUNEND WEGDEEL",
            Class::BegroeidTerreindeel => "BEGROEID TERREINDEEL",
            Class::OnbegroeidTerreindeel => "ONBEGROEID TERREINDEEL",
            Class::Waterdeel => "WATERDEEL",
            Class::OndersteunendWaterdeel => "ONDERSTEUNEND WATERDEEL",
            Class::Pand => "PAND",
            Class::Scheiding => "SCHEIDING",
            Class::Paal => "PAAL",
        }
    }

    fn from_mapping(name: &str) -> Option<Class> {
        Class::ALL.into_iter().find(|c| c.mapping() == name)
    }

    /// From a PDOK collection (`wegdeel`, `scheiding_lijn`) or a class name.
    fn from_name(s: &str) -> Option<Class> {
        let base = s.split('_').next().unwrap_or(s).replace([' ', '-'], "").to_lowercase();
        Class::ALL.into_iter().find(|c| c.name().to_lowercase() == base)
    }

    fn shape(self) -> Shape {
        match self {
            Class::Scheiding => Shape::LineOrSurface,
            Class::Paal => Shape::Point,
            _ => Shape::Surface,
        }
    }

    /// Takes part in the level-0 partition of the BGT (vlakdekkend).
    fn partition(self) -> bool {
        matches!(self, Class::Wegdeel | Class::OndersteunendWegdeel | Class::BegroeidTerreindeel | Class::OnbegroeidTerreindeel | Class::Waterdeel | Class::OndersteunendWaterdeel | Class::Pand)
    }
}

// ── Value lists (IMGeo waardelijsten 2020-07-01) ────────────────────────────

const FUNCTIE_WEG: &[&str] = &["OV-baan", "overweg", "spoorbaan", "baan voor vliegverkeer", "rijbaan autosnelweg", "rijbaan autoweg", "rijbaan regionale weg", "rijbaan lokale weg", "fietspad", "voetpad", "voetpad op trap", "ruiterpad", "parkeervlak", "voetgangersgebied", "inrit", "woonerf"];
const FUNCTIE_WEG_PLUS: &[&str] = &["verbindingsweg", "calamiteitendoorsteek", "verkeersdrempel"];
const FYSIEK_WEG: &[&str] = &["gesloten verharding", "open verharding", "half verhard", "onverhard"];
const FYSIEK_WEG_PLUS: &[&str] = &["asfalt", "cementbeton", "betonstraatstenen", "gebakken klinkers", "tegels", "sierbestrating", "beton element", "grasklinkers", "schelpen", "puin", "grind", "gravel", "boomschors", "zand"];
const FUNCTIE_OWG: &[&str] = &["verkeerseiland", "berm"];
const GROEN_PLUS: &[&str] = &["bosplantsoen", "gras- en kruidachtigen", "planten", "struikrozen", "heesters", "bodembedekkers"];
const FYSIEK_BEGROEID: &[&str] = &["loofbos", "gemengd bos", "naaldbos", "heide", "struiken", "houtwal", "duin", "grasland overig", "moeras", "rietland", "kwelder", "fruitteelt", "boomteelt", "bouwland", "grasland agrarisch", "groenvoorziening"];
const FYSIEK_BEGROEID_PLUS: &[&str] = &["griend en hakhout", "open duinvegetatie", "gesloten duinvegetatie", "laagstam boomgaarden", "hoogstam boomgaarden", "wijngaarden", "klein fruit", "akkerbouw", "braakliggend", "vollegrondsteelt", "bollenteelt"];
const FYSIEK_ONBEGROEID: &[&str] = &["erf", "gesloten verharding", "open verharding", "half verhard", "onverhard", "zand"];
const FYSIEK_ONBEGROEID_PLUS: &[&str] = &["kunststof", "strand en strandwal", "zandverstuiving"];
const TYPE_WATER: &[&str] = &["zee", "waterloop", "watervlakte", "greppel, droge sloot"];
const TYPE_WATER_PLUS: &[&str] = &["rivier", "sloot", "kanaal", "beek", "gracht", "bron", "haven", "meer, plas, ven, vijver"];
const TYPE_OWT: &[&str] = &["oever, slootkant", "slik"];
const TYPE_SCHEIDING: &[&str] = &["muur", "kademuur", "geluidsscherm", "damwand", "walbescherming", "hek", "niet-bgt"];
const TYPE_SCHEIDING_PLUS: &[&str] = &["draadraster", "faunaraster"];
const TYPE_PAAL_PLUS: &[&str] = &["lichtmast", "telpaal", "portaal", "verkeersregelinstallatiepaal", "verkeersbordpaal", "poller", "haltepaal", "vlaggenmast", "afsluitpaal", "praatpaal", "hectometerpaal", "dijkpaal", "drukknoppaal", "grensmarkering", "sirene"];

/// An attribute of an object, with its value lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord, Hash)]
enum Attr {
    Functie,
    PlusFunctie,
    Fysiek,
    PlusFysiek,
    Type,
    PlusType,
}

impl Attr {
    const ALL: [Attr; 6] = [Attr::Functie, Attr::PlusFunctie, Attr::Fysiek, Attr::PlusFysiek, Attr::Type, Attr::PlusType];

    /// The object data key (as the BGT import writes it).
    fn key(self) -> &'static str {
        match self {
            Attr::Functie => "functie",
            Attr::PlusFunctie => "plus_functie",
            Attr::Fysiek => "fysiek_voorkomen",
            Attr::PlusFysiek => "plus_fysiek_voorkomen",
            Attr::Type => "type",
            Attr::PlusType => "plus_type",
        }
    }

    /// The attribute a column of the NLCS-BGT mapping fills, per class.
    fn of_column(class: Class, column: &str) -> Option<Attr> {
        Some(match (class, column) {
            (_, "function") => Attr::Functie,
            (_, "plus-functieWegdeel") => Attr::PlusFunctie,
            (Class::Wegdeel | Class::OndersteunendWegdeel, "surfaceMaterial") => Attr::Fysiek,
            (_, "bgt-fysiekVoorkomen") => Attr::Fysiek,
            (Class::BegroeidTerreindeel, "class") => Attr::Fysiek,
            (Class::Waterdeel | Class::OndersteunendWaterdeel, "class") => Attr::Type,
            (_, "plus-fysiekVoorkomenWegdeel" | "plus-fysiekVoorkomenOndersteunendWegdeel" | "plus-fysiekVoorkomen") => Attr::PlusFysiek,
            (_, "bgt-type") => Attr::Type,
            (_, "plus-type") => Attr::PlusType,
            _ => return None,
        })
    }

    /// The values the attribute may take for `class`; empty: not used there.
    fn values(self, class: Class) -> Vec<&'static str> {
        let join = |a: &[&'static str], b: &[&'static str]| a.iter().chain(b).copied().collect::<Vec<_>>();
        match (class, self) {
            (Class::Wegdeel, Attr::Functie) => FUNCTIE_WEG.to_vec(),
            (Class::Wegdeel, Attr::PlusFunctie) => FUNCTIE_WEG_PLUS.to_vec(),
            (Class::Wegdeel, Attr::Fysiek) => FYSIEK_WEG.to_vec(),
            (Class::Wegdeel, Attr::PlusFysiek) => FYSIEK_WEG_PLUS.to_vec(),
            (Class::OndersteunendWegdeel, Attr::Functie) => FUNCTIE_OWG.to_vec(),
            (Class::OndersteunendWegdeel, Attr::Fysiek) => join(FYSIEK_WEG, &["groenvoorziening"]),
            (Class::OndersteunendWegdeel, Attr::PlusFysiek) => join(FYSIEK_WEG_PLUS, GROEN_PLUS),
            (Class::BegroeidTerreindeel, Attr::Fysiek) => FYSIEK_BEGROEID.to_vec(),
            (Class::BegroeidTerreindeel, Attr::PlusFysiek) => join(FYSIEK_BEGROEID_PLUS, GROEN_PLUS),
            (Class::OnbegroeidTerreindeel, Attr::Fysiek) => FYSIEK_ONBEGROEID.to_vec(),
            (Class::OnbegroeidTerreindeel, Attr::PlusFysiek) => join(FYSIEK_WEG_PLUS, FYSIEK_ONBEGROEID_PLUS),
            (Class::Waterdeel, Attr::Type) => TYPE_WATER.to_vec(),
            (Class::Waterdeel, Attr::PlusType) => TYPE_WATER_PLUS.to_vec(),
            (Class::OndersteunendWaterdeel, Attr::Type) => TYPE_OWT.to_vec(),
            (Class::Scheiding, Attr::Type) => TYPE_SCHEIDING.to_vec(),
            (Class::Scheiding, Attr::PlusType) => TYPE_SCHEIDING_PLUS.to_vec(),
            (Class::Paal, Attr::Type) => vec!["niet-bgt"],
            (Class::Paal, Attr::PlusType) => TYPE_PAAL_PLUS.to_vec(),
            _ => Vec::new(),
        }
    }

    /// Required in the BGT (a bgt attribute, not an IMGeo plus one).
    fn required(self, class: Class) -> bool {
        match class {
            Class::Wegdeel | Class::OndersteunendWegdeel => matches!(self, Attr::Functie | Attr::Fysiek),
            Class::BegroeidTerreindeel | Class::OnbegroeidTerreindeel => self == Attr::Fysiek,
            Class::Waterdeel | Class::OndersteunendWaterdeel | Class::Scheiding | Class::Paal => self == Attr::Type,
            Class::Pand => false,
        }
    }

    /// The code list (codeSpace fragment) of the attribute for `class`.
    fn list(self, class: Class) -> &'static str {
        match (class, self) {
            (Class::Wegdeel, Attr::Functie) => "FunctieWeg",
            (Class::Wegdeel, Attr::PlusFunctie) => "FunctieWegPlus",
            (Class::Wegdeel, Attr::Fysiek) => "FysiekVoorkomenWeg",
            (Class::Wegdeel, Attr::PlusFysiek) => "FysiekVoorkomenWegPlus",
            (Class::OndersteunendWegdeel, Attr::Functie) => "FunctieOndersteunendWegdeel",
            (Class::OndersteunendWegdeel, Attr::PlusFunctie) => "FunctieOndersteunendWegdeelPlus",
            (Class::OndersteunendWegdeel, Attr::Fysiek) => "FysiekVoorkomenOndersteunendWegdeel",
            (Class::OndersteunendWegdeel, Attr::PlusFysiek) => "FysiekVoorkomenOndersteunendWegdeelPlus",
            (Class::BegroeidTerreindeel, Attr::Fysiek) => "FysiekVoorkomenBegroeidTerrein",
            (Class::BegroeidTerreindeel, Attr::PlusFysiek) => "FysiekVoorkomenBegroeidTerreinPlus",
            (Class::OnbegroeidTerreindeel, Attr::Fysiek) => "FysiekVoorkomenOnbegroeidTerrein",
            (Class::OnbegroeidTerreindeel, Attr::PlusFysiek) => "FysiekVoorkomenOnbegroeidTerreinPlus",
            (Class::Waterdeel, Attr::Type) => "TypeWater",
            (Class::Waterdeel, Attr::PlusType) => "TypeWaterPlus",
            (Class::OndersteunendWaterdeel, Attr::Type) => "TypeOndersteunendWaterdeel",
            (Class::OndersteunendWaterdeel, Attr::PlusType) => "TypeOndersteunendWaterdeelPlus",
            (Class::Scheiding, Attr::Type) => "TypeScheiding",
            (Class::Scheiding, Attr::PlusType) => "TypeScheidingPlus",
            (Class::Paal, Attr::Type) => "TypePaal",
            (Class::Paal, Attr::PlusType) => "TypePaalPlus",
            _ => "",
        }
    }
}

fn norm(v: &str) -> String {
    v.trim().to_lowercase().replace(['-', '_'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The official spelling of `value` in the attribute's list (case, hyphens),
/// or None when it is not in the list.
fn canonical(class: Class, attr: Attr, value: &str) -> Option<&'static str> {
    // A typo in the NLCS-BGT mapping.
    let value = if norm(value) == "gras en kruidachtingen" { "gras- en kruidachtigen" } else { value };
    attr.values(class).into_iter().find(|v| norm(v) == norm(value))
}

// ── What the drawing holds ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum Kind {
    Nieuw,
    Gewijzigd,
    Beeindigd,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Nieuw => "nieuw",
            Kind::Gewijzigd => "gewijzigd",
            Kind::Beeindigd => "beëindigd",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Geometry {
    Surface { outer: Vec<XY>, holes: Vec<Vec<XY>> },
    /// Several surfaces (a pand of more parts).
    Surfaces(Vec<(Vec<XY>, Vec<Vec<XY>>)>),
    Line(Vec<XY>),
    Point(XY),
}

impl Geometry {
    fn area(&self) -> f64 {
        let one = |o: &[XY], h: &[Vec<XY>]| ring_area(o).abs() - h.iter().map(|r| ring_area(r).abs()).sum::<f64>();
        match self {
            Geometry::Surface { outer, holes } => one(outer, holes),
            Geometry::Surfaces(parts) => parts.iter().map(|(o, h)| one(o, h)).sum(),
            _ => 0.0,
        }
    }

    fn parts(&self) -> Vec<(&Vec<XY>, &Vec<Vec<XY>>)> {
        match self {
            Geometry::Surface { outer, holes } => vec![(outer, holes)],
            Geometry::Surfaces(parts) => parts.iter().map(|(o, h)| (o, h)).collect(),
            _ => Vec::new(),
        }
    }

    fn bbox(&self) -> [f64; 4] {
        let pts: Vec<XY> = match self {
            Geometry::Surface { outer, .. } => outer.clone(),
            Geometry::Surfaces(parts) => parts.iter().flat_map(|(o, _)| o.clone()).collect(),
            Geometry::Line(l) => l.clone(),
            Geometry::Point(p) => vec![*p],
        };
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in pts {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        b
    }

    fn contains(&self, p: XY) -> bool {
        self.parts().iter().any(|(o, h)| point_in_ring(p, o) && !h.iter().any(|r| point_in_ring(p, r)))
    }
}

/// One BGT object of the mutation.
#[derive(Clone, Debug)]
pub struct Feature {
    pub kind: Kind,
    pub class: Class,
    pub lokaal_id: String,
    pub plan: bool,
    attrs: BTreeMap<Attr, String>,
    pub hoogteligging: i32,
    pub op_talud: bool,
    pub bag_pnd: Option<String>,
    /// objectBeginTijd of an existing object (yyyy-mm-dd).
    pub creation: Option<String>,
    pub geometry: Geometry,
    pub handles: Vec<Handle>,
    pub layer: String,
    /// Where the attributes came from.
    pub source: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Fout,
    Waarschuwing,
    Melding,
}

#[derive(Clone, Debug)]
pub struct Issue {
    pub severity: Severity,
    pub layer: String,
    pub handles: Vec<Handle>,
    pub message: String,
}

#[derive(Default)]
pub struct Mutation {
    pub features: Vec<Feature>,
    pub issues: Vec<Issue>,
    pub bronhouder: String,
    /// Existing BGT objects left as they are (status B).
    pub unchanged: usize,
    /// Entities not on an NLCS layer or not for the BGT (T, X, annotation).
    pub skipped: usize,
}

impl Mutation {
    pub fn bronhouder_ok(&self) -> bool {
        let b = &self.bronhouder;
        b.len() == 5 && b.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && b[1..].chars().all(|c| c.is_ascii_digit())
    }
    pub fn count(&self, k: Kind) -> usize {
        self.features.iter().filter(|f| f.kind == k).count()
    }
    pub fn errors(&self) -> usize {
        self.issues.iter().filter(|i| i.severity == Severity::Fout).count()
    }
}

/// A piece of drawing: rings, a line or a point, with what it says about itself.
struct Piece {
    handle: Handle,
    layer: String,
    status: char,
    nlcs: u32,
    data: HashMap<String, String>,
    rings: Vec<Vec<XY>>,
    lines: Vec<Vec<XY>>,
    points: Vec<XY>,
}

fn object_data(e: &EntityType) -> HashMap<String, String> {
    let mut m = HashMap::new();
    if let Some(r) = e.common().extended_data.get_record(crate::bridge::XDATA_APP) {
        for v in &r.values {
            if let ocs_plugin_api::host::XDataValue::String(s) = v {
                if let Some((k, v)) = s.split_once('=') {
                    m.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
    }
    m
}

/// Status letter and NLCS object of a layer. NLCS 5 also has R (revisie)
/// and X; the rest of the name is checked as usual.
/// Other layer standards' names are taken back to their NLCS object.
fn layer_object(layer: &str) -> Option<(char, u32, String)> {
    let status = layer.chars().next()?;
    if "BNTVRX".contains(status) && layer.chars().nth(1) == Some('-') {
        if let Some(obj) = nlcs::parse_nlcs_layer(&format!("B{}", &layer[1..])).ok().and_then(|p| Some((nlcs::object_by_name(&p.group, &p.object)?, p.element))) {
            return Some((status, obj.0.id, obj.1));
        }
    }
    let (obj, status, element) = crate::laagstandaard::recognise(layer)?;
    Some((status.code().chars().next()?, obj.id, element.code().into()))
}

/// Arc points of a bulge segment, without the start, with the end.
fn bulge_points(a: XY, b: XY, bulge: f64, out: &mut Vec<XY>) {
    if bulge.abs() < 1e-9 {
        out.push(b);
        return;
    }
    let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
    let theta = 4.0 * bulge.atan();
    let r = chord / (2.0 * (theta / 2.0).sin());
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let d = r * (theta / 2.0).cos();
    let (ux, uy) = ((b[0] - a[0]) / chord, (b[1] - a[1]) / chord);
    let c = [mid[0] - uy * d, mid[1] + ux * d];
    let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
    // At most 1 cm off the arc.
    let step = 2.0 * (1.0 - 0.01 / r.abs().max(0.02)).clamp(-1.0, 1.0).acos();
    let n = ((theta.abs() / step.max(1e-3)).ceil() as usize).clamp(2, 256);
    for k in 1..n {
        let t = a0 + theta * k as f64 / n as f64;
        out.push([c[0] + r.abs() * t.cos(), c[1] + r.abs() * t.sin()]);
    }
    out.push(b);
}

fn bulged(v: &[(XY, f64)], closed: bool) -> Vec<XY> {
    let mut out = Vec::with_capacity(v.len());
    let Some(first) = v.first() else { return out };
    out.push(first.0);
    for i in 1..v.len() {
        bulge_points(v[i - 1].0, v[i].0, v[i - 1].1, &mut out);
    }
    if closed && v.len() > 2 {
        bulge_points(v[v.len() - 1].0, v[0].0, v[v.len() - 1].1, &mut out);
        out.pop();
    }
    out
}

fn circle(c: XY, r: f64) -> Vec<XY> {
    let n = 32;
    (0..n).map(|k| {
        let t = 2.0 * PI * k as f64 / n as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    }).collect()
}

/// The geometry of an entity, in drawing units. On a symbol layer (S) a
/// circle is a point object (the BGT import draws those as small circles);
/// elsewhere it is a round surface (a tree pit, a planter).
fn shapes(e: &EntityType, symbol: bool) -> (Vec<Vec<XY>>, Vec<Vec<XY>>, Vec<XY>) {
    let (mut rings, mut lines, mut points) = (Vec::new(), Vec::new(), Vec::new());
    match e {
        EntityType::LwPolyline(p) => {
            let v: Vec<(XY, f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
            let closed = p.is_closed || (v.len() > 3 && v[0].0 == v[v.len() - 1].0);
            let pts = bulged(&v, p.is_closed);
            if closed { rings.push(pts) } else { lines.push(pts) }
        }
        EntityType::Polyline2D(p) => {
            let v: Vec<(XY, f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
            let pts = bulged(&v, p.flags.is_closed());
            if p.flags.is_closed() { rings.push(pts) } else { lines.push(pts) }
        }
        EntityType::Polyline3D(p) => {
            let pts: Vec<XY> = p.vertices.iter().map(|v| [v.position.x, v.position.y]).collect();
            if p.flags.closed { rings.push(pts) } else { lines.push(pts) }
        }
        EntityType::Line(l) => lines.push(vec![[l.start.x, l.start.y], [l.end.x, l.end.y]]),
        EntityType::Arc(a) => {
            let mut sweep = a.end_angle - a.start_angle;
            if sweep <= 0.0 {
                sweep += 2.0 * PI;
            }
            let n = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
            lines.push((0..=n).map(|k| {
                let t = a.start_angle + sweep * k as f64 / n as f64;
                [a.center.x + a.radius * t.cos(), a.center.y + a.radius * t.sin()]
            }).collect());
        }
        EntityType::Circle(c) if symbol => points.push([c.center.x, c.center.y]),
        EntityType::Circle(c) => rings.push(circle([c.center.x, c.center.y], c.radius)),
        EntityType::Point(p) => points.push([p.location.x, p.location.y]),
        EntityType::Insert(i) => points.push([i.insert_point.x, i.insert_point.y]),
        EntityType::Hatch(h) => {
            for path in &h.paths {
                let mut pts: Vec<(XY, f64)> = Vec::new();
                for edge in &path.edges {
                    match edge {
                        BoundaryEdge::Polyline(p) => pts.extend(p.vertices.iter().map(|v| ([v.x, v.y], v.z))),
                        BoundaryEdge::Line(l) => {
                            if pts.last().is_none_or(|q| (q.0[0] - l.start.x).abs() > 1e-9 || (q.0[1] - l.start.y).abs() > 1e-9) {
                                pts.push(([l.start.x, l.start.y], 0.0));
                            }
                            pts.push(([l.end.x, l.end.y], 0.0));
                        }
                        BoundaryEdge::CircularArc(a) => {
                            let mut sweep = a.end_angle - a.start_angle;
                            if sweep <= 0.0 {
                                sweep += 2.0 * PI;
                            }
                            let n = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
                            for k in 0..=n {
                                let t = a.start_angle + sweep * k as f64 / n as f64;
                                let t = if a.counter_clockwise { t } else { -t };
                                pts.push(([a.center.x + a.radius * t.cos(), a.center.y + a.radius * t.sin()], 0.0));
                            }
                        }
                        _ => {}
                    }
                }
                let mut ring = bulged(&pts, true);
                if ring.len() > 1 && ring[0] == ring[ring.len() - 1] {
                    ring.pop();
                }
                if ring.len() >= 3 {
                    rings.push(ring);
                }
            }
        }
        _ => {}
    }
    (rings, lines, points)
}

// ── Geometry helpers (RD metres) ────────────────────────────────────────────

fn ring_area(r: &[XY]) -> f64 {
    let n = r.len();
    (0..n).map(|i| {
        let (a, b) = (r[i], r[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>() / 2.0
}

fn point_in_ring(p: XY, r: &[XY]) -> bool {
    let mut inside = false;
    let n = r.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (r[i], r[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn mm(v: f64) -> f64 {
    // Rounded half up to the millimetre (catalogue §3.4).
    (v * 1000.0 + 0.5).floor() / 1000.0
}

/// Rounded to mm, repeated points out; closing point not repeated.
fn clean(pts: &[XY], ring: bool) -> Vec<XY> {
    let mut out: Vec<XY> = Vec::with_capacity(pts.len());
    for p in pts {
        let q = [mm(p[0]), mm(p[1])];
        if out.last().is_none_or(|l| (l[0] - q[0]).abs() > 0.0005 || (l[1] - q[1]).abs() > 0.0005) {
            out.push(q);
        }
    }
    if ring && out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).abs() < 0.0005 && (out[0][1] - out[out.len() - 1][1]).abs() < 0.0005 {
        out.pop();
    }
    out
}

fn segments_cross(a: XY, b: XY, c: XY, d: XY) -> bool {
    let o = |p: XY, q: XY, r: XY| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
    ((d1 > 1e-9 && d2 < -1e-9) || (d1 < -1e-9 && d2 > 1e-9)) && ((d3 > 1e-9 && d4 < -1e-9) || (d3 < -1e-9 && d4 > 1e-9))
}

/// Where a ring or line crosses itself (the first crossing found).
fn self_crossing(pts: &[XY], ring: bool) -> Option<XY> {
    let n = pts.len();
    let segs = if ring { n } else { n.saturating_sub(1) };
    for i in 0..segs {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        for j in (i + 2)..segs {
            if ring && i == 0 && j == segs - 1 {
                continue;
            }
            let (c, d) = (pts[j], pts[(j + 1) % n]);
            if segments_cross(a, b, c, d) {
                return Some([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
            }
        }
    }
    None
}

/// Surface two geometries share, estimated on a grid.
fn overlap(a: &Geometry, b: &Geometry) -> f64 {
    let (p, q) = (a.bbox(), b.bbox());
    let x0 = p[0].max(q[0]);
    let y0 = p[1].max(q[1]);
    let x1 = p[2].min(q[2]);
    let y1 = p[3].min(q[3]);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let step = (((x1 - x0) * (y1 - y0)) / 6000.0).sqrt().max(0.05);
    let mut hits = 0usize;
    let mut y = y0 + step / 2.0;
    while y < y1 {
        let mut x = x0 + step / 2.0;
        while x < x1 {
            if a.contains([x, y]) && b.contains([x, y]) {
                hits += 1;
            }
            x += step;
        }
        y += step;
    }
    hits as f64 * step * step
}

/// Rings into surfaces, by index: a ring inside another (and not inside one
/// of its holes) is a hole; an island in a hole is a surface again.
fn nest_ids(rings: &[Vec<XY>]) -> Vec<(usize, Vec<usize>)> {
    let mut order: Vec<usize> = (0..rings.len()).filter(|&i| rings[i].len() >= 3).collect();
    order.sort_by(|&a, &b| ring_area(&rings[b]).abs().total_cmp(&ring_area(&rings[a]).abs()));
    let mut out: Vec<(usize, Vec<usize>)> = Vec::new();
    for i in order {
        // A point of this ring that is not on the other's boundary: the
        // middle of its first edge, nudged inward.
        let r = &rings[i];
        let probe = interior_point(r);
        let parent = out.iter().rposition(|(o, h)| point_in_ring(probe, &rings[*o]) && !h.iter().any(|x| point_in_ring(probe, &rings[*x])));
        match parent {
            Some(k) => out[k].1.push(i),
            None => out.push((i, Vec::new())),
        }
    }
    out
}

/// A point just inside a ring (near its first edge).
fn interior_point(r: &[XY]) -> XY {
    let ccw = ring_area(r) > 0.0;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len < 1e-6 {
            continue;
        }
        // Left of the edge is inside for a counter-clockwise ring.
        let (nx, ny) = if ccw { (-(b[1] - a[1]) / len, (b[0] - a[0]) / len) } else { ((b[1] - a[1]) / len, -(b[0] - a[0]) / len) };
        let d = (len * 1e-3).min(1e-3);
        let p = [(a[0] + b[0]) / 2.0 + nx * d, (a[1] + b[1]) / 2.0 + ny * d];
        if point_in_ring(p, r) {
            return p;
        }
    }
    r[0]
}

/// Outer ring counter-clockwise, holes clockwise.
fn oriented(mut outer: Vec<XY>, mut holes: Vec<Vec<XY>>) -> (Vec<XY>, Vec<Vec<XY>>) {
    if ring_area(&outer) < 0.0 {
        outer.reverse();
    }
    for h in holes.iter_mut() {
        if ring_area(h) > 0.0 {
            h.reverse();
        }
    }
    (outer, holes)
}

fn nest(rings: Vec<Vec<XY>>) -> Vec<(Vec<XY>, Vec<Vec<XY>>)> {
    nest_ids(&rings).into_iter().map(|(o, h)| oriented(rings[o].clone(), h.iter().map(|&i| rings[i].clone()).collect())).collect()
}

/// A new lokaalID: bronhouder code and 32 random hex digits (UUID v4).
fn new_id(bronhouder: &str) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut bytes = [0u8; 16];
    for half in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        bytes[half * 8..half * 8 + 8].copy_from_slice(&h.finish().to_be_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{bronhouder}.{hex}")
}

// ── From drawing to mutation ────────────────────────────────────────────────

/// What the mapping says about an NLCS object: per class, the value of each
/// attribute when all rows of that class agree.
struct Candidates {
    classes: Vec<Class>,
    rows: Vec<(Class, BTreeMap<Attr, String>)>,
    other: Vec<String>,
}

fn candidates(nlcs_object: u32) -> Candidates {
    let mut c = Candidates { classes: Vec::new(), rows: Vec::new(), other: Vec::new() };
    for row in nlcs::bgt_rows(nlcs_object) {
        let Some(class) = Class::from_mapping(&row.object) else {
            if !c.other.contains(&row.object) {
                c.other.push(row.object.clone());
            }
            continue;
        };
        let attrs = row.values.iter().filter_map(|(col, v)| Some((Attr::of_column(class, col)?, v.clone()))).collect();
        if !c.classes.contains(&class) {
            c.classes.push(class);
        }
        c.rows.push((class, attrs));
    }
    c
}

impl Candidates {
    /// The value of `attr` the rows of `class` (and, when given, with this
    /// functie) agree on.
    fn agreed(&self, class: Class, functie: Option<&str>, attr: Attr) -> Result<Option<String>, Vec<String>> {
        let rows: Vec<&BTreeMap<Attr, String>> = self.rows.iter().filter(|(c, a)| *c == class && functie.is_none_or(|f| a.get(&Attr::Functie).is_none_or(|g| norm(g) == norm(f)))).map(|(_, a)| a).collect();
        let mut seen: Vec<String> = Vec::new();
        for r in &rows {
            if let Some(v) = r.get(&attr) {
                if !seen.iter().any(|s| norm(s) == norm(v)) {
                    seen.push(v.clone());
                }
            }
        }
        match seen.len() {
            0 => Ok(None),
            1 if rows.iter().all(|r| r.contains_key(&attr)) => Ok(seen.pop()),
            // Some rows leave it open: a general rule beside specific ones.
            1 => Ok(None),
            _ => Err(seen),
        }
    }
}

struct Existing {
    lokaal_id: String,
    class: Class,
    data: HashMap<String, String>,
    geometry: Geometry,
    status: char,
}

/// Turn the drawing into a BGT mutation.
pub fn build(doc: &CadDocument, site: &Site, opt: &Options) -> Mutation {
    let mut m = Mutation::default();
    let f = opt.unit_factor.max(1e-9);
    let to_rd = |p: XY| -> Option<XY> {
        let q = site.from_drawing(p, f);
        if site.crs == "EPSG:28992" { Some(q) } else { crs::transform(&site.crs, "EPSG:28992", q).ok() }
    };

    // 1. Pieces on NLCS layers.
    let mut pieces: Vec<Piece> = Vec::new();
    let mut outside: Vec<Handle> = Vec::new();
    // The project area in RD, a little wider.
    let area: Vec<f64> = {
        let r = site.drawing_rect(f);
        let corners: Vec<XY> = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].iter().filter_map(|p| to_rd(*p)).collect();
        if corners.len() == 4 && site.width > 0.0 && site.height > 0.0 {
            let margin = 0.1 * site.width.max(site.height);
            vec![
                corners.iter().map(|p| p[0]).fold(f64::MAX, f64::min) - margin,
                corners.iter().map(|p| p[1]).fold(f64::MAX, f64::min) - margin,
                corners.iter().map(|p| p[0]).fold(f64::MIN, f64::max) + margin,
                corners.iter().map(|p| p[1]).fold(f64::MIN, f64::max) + margin,
            ]
        } else {
            Vec::new()
        }
    };
    for e in doc.model_space_entities() {
        let c = e.common();
        if opt.only.as_ref().is_some_and(|s| !s.contains(&c.handle)) {
            continue;
        }
        let Some((status, nlcs_id, element)) = layer_object(&c.layer) else {
            m.skipped += 1;
            continue;
        };
        if nlcs::ANNOTATION_ELEMENTS.contains(&element.as_str()) || matches!(status, 'T' | 'X') {
            m.skipped += 1;
            continue;
        }
        let data = object_data(e);
        if data.get("bron").is_some_and(|b| b == "tekenblad") {
            m.skipped += 1;
            continue;
        }
        let (rings, lines, points) = shapes(e, element == "S");
        let conv = |v: Vec<Vec<XY>>| -> Vec<Vec<XY>> { v.into_iter().map(|r| r.into_iter().filter_map(|p| to_rd(p)).collect()).collect() };
        let (rings, lines, points) = (conv(rings), conv(lines), points.into_iter().filter_map(|p| to_rd(p)).collect::<Vec<XY>>());
        // Without object data and wholly outside the project area: a legend
        // sample of an older drawing sheet, a detail beside the plan.
        if data.is_empty() && !area.is_empty() {
            let all: Vec<XY> = rings.iter().chain(&lines).flatten().copied().chain(points.iter().copied()).collect();
            if !all.is_empty() && all.iter().all(|p| p[0] < area[0] || p[0] > area[2] || p[1] < area[1] || p[1] > area[3]) {
                outside.push(c.handle);
                continue;
            }
        }
        pieces.push(Piece {
            handle: c.handle,
            layer: c.layer.clone(),
            status,
            nlcs: nlcs_id,
            data,
            rings,
            lines,
            points,
        });
    }
    if !outside.is_empty() {
        m.issues.push(Issue { severity: Severity::Melding, layer: String::new(), handles: outside.clone(), message: format!("{} element(en) zonder objectgegevens buiten het projectgebied (legenda, details) overgeslagen.", outside.len()) });
        m.skipped += outside.len();
    }

    // 2. Bronhouder.
    m.bronhouder = if opt.bronhouder.trim().is_empty() {
        let mut count: HashMap<String, usize> = HashMap::new();
        for p in &pieces {
            if let Some(b) = p.data.get("bronhouder").cloned().or_else(|| p.data.get("lokaal_id").and_then(|l| l.split('.').next().map(str::to_string))) {
                *count.entry(b).or_default() += 1;
            }
        }
        count.into_iter().max_by_key(|(_, n)| *n).map(|(b, _)| b).unwrap_or_default()
    } else {
        opt.bronhouder.trim().to_uppercase()
    };
    if !m.bronhouder_ok() {
        m.issues.push(Issue {
            severity: Severity::Fout,
            layer: String::new(),
            handles: Vec::new(),
            message: if m.bronhouder.is_empty() {
                "Geen bronhouder: geef de code van de gemeente (bijv. G0505 voor Dordrecht) mee, of importeer eerst de BGT.".into()
            } else {
                format!("Bronhoudercode '{}' klopt niet: een letter en vier cijfers (bijv. G0505).", m.bronhouder)
            },
        });
    }

    // 3. Existing BGT objects in the drawing (by lokaal_id).
    let mut by_id: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, p) in pieces.iter().enumerate() {
        if p.data.get("bron").is_some_and(|b| b == "BGT") {
            if let Some(id) = p.data.get("lokaal_id") {
                by_id.entry(id.clone()).or_default().push(i);
            }
        }
    }
    let mut existing: Vec<Existing> = Vec::new();
    let mut used = vec![false; pieces.len()];
    for (id, idx) in &by_id {
        let first = &pieces[idx[0]];
        let class = first.data.get("objecttype").and_then(|t| Class::from_name(t));
        let geometry = geometry_of(idx.iter().map(|&i| &pieces[i]), class.map(Class::shape));
        for &i in idx {
            used[i] = true;
        }
        let Some(class) = class else {
            m.skipped += idx.len();
            continue;
        };
        let Some(geometry) = geometry else { continue };
        existing.push(Existing { lokaal_id: id.clone(), class, data: first.data.clone(), geometry, status: first.status });
    }

    // 4. Existing objects that change or end.
    let mut on_new: Vec<Handle> = Vec::new();
    for ex in &existing {
        let idx = &by_id[&ex.lokaal_id];
        let handles: Vec<Handle> = idx.iter().map(|&i| pieces[i].handle).collect();
        let layer = pieces[idx[0]].layer.clone();
        let kind = match ex.status {
            'V' => Kind::Beeindigd,
            'R' => Kind::Gewijzigd,
            'N' if opt.as_built => Kind::Gewijzigd,
            'N' => {
                on_new.extend(handles);
                continue;
            }
            _ => {
                m.unchanged += 1;
                continue;
            }
        };
        let mut attrs = BTreeMap::new();
        for a in Attr::ALL {
            if let Some(v) = ex.data.get(a.key()).filter(|v| !v.is_empty()) {
                attrs.insert(a, canonical(ex.class, a, v).map(str::to_string).unwrap_or_else(|| v.clone()));
            }
        }
        m.features.push(Feature {
            kind,
            class: ex.class,
            lokaal_id: ex.lokaal_id.clone(),
            plan: false,
            attrs,
            hoogteligging: ex.data.get("relatieve_hoogteligging").and_then(|v| v.parse().ok()).unwrap_or(0),
            op_talud: ex.data.get("op_talud").is_some_and(|v| v == "true"),
            bag_pnd: ex.data.get("bag_pnd").cloned(),
            creation: ex.data.get("creation_date").map(|d| dutch_date(d)),
            geometry: ex.geometry.clone(),
            handles,
            layer,
            source: "BGT-objectgegevens".into(),
        });
    }

    if !on_new.is_empty() {
        m.issues.push(Issue { severity: Severity::Melding, layer: String::new(), handles: on_new.clone(), message: format!("{} BGT-object(en) staan op lagen met status N; die blijven zoals ze zijn. Zet ze op R om ze te wijzigen of op V om ze te beëindigen.", on_new.len()) });
    }

    // 5. New objects: everything on N/R layers without a BGT identification.
    let mut groups: BTreeMap<(String, char), Vec<usize>> = BTreeMap::new();
    for (i, p) in pieces.iter().enumerate() {
        if used[i] {
            continue;
        }
        match p.status {
            'N' | 'R' => groups.entry((p.layer.clone(), p.status)).or_default().push(i),
            'V' => m.issues.push(Issue { severity: Severity::Melding, layer: p.layer.clone(), handles: vec![p.handle], message: "Te verwijderen element zonder BGT-identificatie: niet in de mutatie.".into() }),
            _ => {}
        }
    }
    for ((layer, status), idx) in groups {
        let nlcs_id = pieces[idx[0]].nlcs;
        let cand = candidates(nlcs_id);
        let plan = status == 'N' && !opt.as_built;
        // One object per surface; lines and points one by one.
        let owners: Vec<(Handle, &HashMap<String, String>)> = idx.iter().flat_map(|&i| pieces[i].rings.iter().map(move |_| i)).map(|i| (pieces[i].handle, &pieces[i].data)).collect();
        let rings: Vec<Vec<XY>> = idx.iter().flat_map(|&i| pieces[i].rings.iter().cloned()).collect();
        let mut items: Vec<(Geometry, Vec<Handle>, HashMap<String, String>)> = Vec::new();
        for (o, h) in nest_ids(&rings) {
            let mut handles = vec![owners[o].0];
            handles.extend(h.iter().map(|&i| owners[i].0).filter(|x| *x != owners[o].0));
            let (outer, holes) = oriented(rings[o].clone(), h.iter().map(|&i| rings[i].clone()).collect());
            items.push((Geometry::Surface { outer, holes }, handles, owners[o].1.clone()));
        }
        for &i in &idx {
            for l in &pieces[i].lines {
                items.push((Geometry::Line(l.clone()), vec![pieces[i].handle], pieces[i].data.clone()));
            }
            for p in &pieces[i].points {
                items.push((Geometry::Point(*p), vec![pieces[i].handle], pieces[i].data.clone()));
            }
        }
        for (geometry, handles, data) in items {
            if let Some(feature) = new_feature(&mut m, &cand, &existing, &layer, plan, geometry, handles, &data) {
                m.features.push(feature);
            }
        }
    }

    // 6. Checks.
    check(&mut m, &existing);
    m
}

/// The geometry of an existing object from its pieces.
fn geometry_of<'a>(pieces: impl Iterator<Item = &'a Piece>, shape: Option<Shape>) -> Option<Geometry> {
    let (mut rings, mut lines, mut points) = (Vec::new(), Vec::new(), Vec::new());
    for p in pieces {
        rings.extend(p.rings.iter().cloned());
        lines.extend(p.lines.iter().cloned());
        points.extend(p.points.iter().copied());
    }
    match shape {
        Some(Shape::Point) => points.first().map(|p| Geometry::Point(*p)),
        _ if !rings.is_empty() => {
            let mut parts = nest(rings);
            if parts.len() == 1 {
                let (outer, holes) = parts.pop()?;
                Some(Geometry::Surface { outer, holes })
            } else {
                Some(Geometry::Surfaces(parts))
            }
        }
        _ => lines.into_iter().next().map(Geometry::Line).or_else(|| points.first().map(|p| Geometry::Point(*p))),
    }
}

#[allow(clippy::too_many_arguments)]
fn new_feature(m: &mut Mutation, cand: &Candidates, existing: &[Existing], layer: &str, plan: bool, geometry: Geometry, handles: Vec<Handle>, data: &HashMap<String, String>) -> Option<Feature> {
    let issue = |m: &mut Mutation, sev: Severity, msg: String| m.issues.push(Issue { severity: sev, layer: layer.to_string(), handles: handles.clone(), message: msg });
    if cand.classes.is_empty() {
        if cand.other.is_empty() {
            m.skipped += 1;
        } else {
            issue(m, Severity::Melding, format!("{} wordt (nog) niet als BGT-mutatie uitgevoerd.", cand.other.join(", ").to_lowercase()));
        }
        return None;
    }
    let is_surface = matches!(geometry, Geometry::Surface { .. });
    let fits = |c: &Class| match c.shape() {
        Shape::Surface => is_surface,
        Shape::Point => matches!(geometry, Geometry::Point(_)),
        Shape::LineOrSurface => !matches!(geometry, Geometry::Point(_)),
    };
    let classes: Vec<Class> = cand.classes.iter().copied().filter(fits).collect();
    if classes.is_empty() {
        // An open line on a surface layer, a hatch symbol, …
        let what = match geometry {
            Geometry::Line(_) => "een open lijn",
            Geometry::Point(_) => "een punt of symbool",
            _ => "een vlak",
        };
        issue(m, Severity::Waarschuwing, format!("{what} op een laag voor {}: niet in de mutatie (sluit de lijn, of teken het op de juiste laag).", cand.classes.iter().map(|c| c.name()).collect::<Vec<_>>().join("/")));
        return None;
    }

    // Attributes given by hand win.
    let given_class = data.get("objecttype").and_then(|t| Class::from_name(t)).filter(|c| classes.contains(c));
    let mut source = Vec::new();
    // The existing BGT object this surface covers most.
    let area = geometry.area();
    let beneath = if is_surface && area > 0.0 {
        existing
            .iter()
            .filter(|e| classes.contains(&e.class) && matches!(e.geometry, Geometry::Surface { .. } | Geometry::Surfaces(_)))
            .map(|e| (e, overlap(&geometry, &e.geometry) / area))
            .filter(|(_, share)| *share >= 0.2)
            .max_by(|a, b| a.1.total_cmp(&b.1))
    } else {
        None
    };
    let class = match (given_class, beneath, classes.as_slice()) {
        (Some(c), _, _) => {
            source.push("objectgegevens".to_string());
            c
        }
        (None, Some((e, share)), _) => {
            source.push(format!("{} ({:.0} % overlap)", e.lokaal_id, share * 100.0));
            e.class
        }
        (None, None, [only]) => *only,
        (None, None, many) => {
            issue(m, Severity::Fout, format!("Niet eenduidig: {} kan {} zijn. Geef objectgegevens 'objecttype' mee, of teken het over het bestaande BGT-object.", nlcs::humanize(layer.split('-').nth(3).unwrap_or(layer)), many.iter().map(|c| c.name()).collect::<Vec<_>>().join(", ")));
            return None;
        }
    };

    let mut attrs: BTreeMap<Attr, String> = BTreeMap::new();
    let mut open: Vec<String> = Vec::new();
    // Functie first: the other values of a row may depend on it.
    let mut order = Attr::ALL.to_vec();
    order.sort_by_key(|a| *a != Attr::Functie);
    for a in order {
        if a.values(class).is_empty() {
            continue;
        }
        let functie = attrs.get(&Attr::Functie).cloned();
        let value = if let Some(v) = data.get(a.key()).filter(|v| !v.is_empty()) {
            Some((v.clone(), "objectgegevens"))
        } else {
            match cand.agreed(class, functie.as_deref(), a) {
                Ok(Some(v)) => Some((v, "NLCS-laag")),
                Ok(None) | Err(_) => beneath.and_then(|(e, _)| e.data.get(a.key()).filter(|v| !v.is_empty()).map(|v| (v.clone(), "bestaand object"))).or_else(|| match cand.agreed(class, functie.as_deref(), a) {
                    Err(options) => {
                        if a.required(class) {
                            open.push(format!("{}: {}", a.key(), options.iter().map(|o| o.to_lowercase()).collect::<Vec<_>>().join(" | ")));
                        }
                        None
                    }
                    _ => None,
                }),
            }
        };
        if let Some((v, from)) = value {
            match canonical(class, a, &v) {
                Some(c) => {
                    attrs.insert(a, c.to_string());
                    if from != "NLCS-laag" && !source.iter().any(|s| s == from) && from != "bestaand object" {
                        source.push(from.to_string());
                    }
                }
                None => issue(m, Severity::Waarschuwing, format!("{} '{}' staat niet in de IMGeo-waardelijst {}: weggelaten.", a.key(), v, a.list(class))),
            }
        }
    }
    if class == Class::Paal {
        attrs.entry(Attr::Type).or_insert_with(|| "niet-bgt".into());
    }
    let missing: Vec<&str> = Attr::ALL.iter().filter(|a| a.required(class) && !attrs.contains_key(a)).map(|a| a.key()).collect();
    if !missing.is_empty() {
        let hint = if open.is_empty() { String::new() } else { format!(" (keuze: {})", open.join("; ")) };
        issue(m, Severity::Fout, format!("{}: {} onbekend{hint}. Geef het mee in de objectgegevens, of teken over het bestaande BGT-object.", class.name(), missing.join(" en ")));
        return None;
    }
    if source.is_empty() {
        source.push("NLCS-laag".into());
    }
    Some(Feature {
        kind: Kind::Nieuw,
        class,
        lokaal_id: new_id(&m.bronhouder),
        plan,
        attrs,
        hoogteligging: data.get("relatieve_hoogteligging").and_then(|v| v.parse().ok()).unwrap_or(0),
        op_talud: data.get("op_talud").is_some_and(|v| v == "true"),
        bag_pnd: data.get("bag_pnd").cloned(),
        creation: None,
        geometry,
        handles,
        layer: layer.to_string(),
        source: source.join(", "),
    })
}

/// Geometry rules of the LV-BGT and the level-0 partition.
fn check(m: &mut Mutation, existing: &[Existing]) {
    let mut issues = Vec::new();
    for f in &mut m.features {
        let mut say = |sev: Severity, msg: String| issues.push(Issue { severity: sev, layer: f.layer.clone(), handles: f.handles.clone(), message: format!("{} {}: {msg}", f.class.name(), short(&f.lokaal_id)) });
        match &mut f.geometry {
            Geometry::Surface { outer, holes } => {
                *outer = clean(outer, true);
                for h in holes.iter_mut() {
                    *h = clean(h, true);
                }
                holes.retain(|h| h.len() >= 3);
                if outer.len() < 3 {
                    say(Severity::Fout, "minder dan drie punten.".into());
                    continue;
                }
                let n: usize = outer.len() + holes.iter().map(Vec::len).sum::<usize>();
                if n > 5000 {
                    say(Severity::Fout, format!("{n} punten; de BGT staat er hoogstens 5000 toe. Deel het object op."));
                }
                for r in std::iter::once(&*outer).chain(holes.iter()) {
                    if let Some(p) = self_crossing(r, true) {
                        say(Severity::Fout, format!("de rand snijdt zichzelf bij X {:.3} Y {:.3}.", p[0], p[1]));
                        break;
                    }
                }
                let a = ring_area(outer).abs();
                if a < 1.0 {
                    say(Severity::Waarschuwing, format!("heel klein vlak ({a:.2} m²)."));
                }
            }
            Geometry::Line(l) => {
                *l = clean(l, false);
                if l.len() < 2 {
                    say(Severity::Fout, "lijn zonder lengte.".into());
                } else if let Some(p) = self_crossing(l, false) {
                    say(Severity::Fout, format!("de lijn snijdt zichzelf bij X {:.3} Y {:.3}.", p[0], p[1]));
                }
            }
            Geometry::Point(p) => *p = [mm(p[0]), mm(p[1])],
            Geometry::Surfaces(parts) => {
                for (o, h) in parts.iter_mut() {
                    *o = clean(o, true);
                    for x in h.iter_mut() {
                        *x = clean(x, true);
                    }
                }
            }
        }
        if f.class == Class::Pand && f.bag_pnd.is_none() && !f.plan {
            say(Severity::Waarschuwing, "geen BAG-pandidentificatie (bag_pnd); vul die in het BGT-pakket aan.".into());
        }
    }
    m.issues.extend(issues);

    // Level 0: no overlaps among the new and changed surfaces, nor with
    // existing objects that stay. Plan topography stays outside.
    let live: Vec<&Feature> = m.features.iter().filter(|f| f.kind != Kind::Beeindigd && !f.plan && f.hoogteligging == 0 && f.class.partition()).collect();
    for (i, a) in live.iter().enumerate() {
        for b in &live[i + 1..] {
            let o = overlap(&a.geometry, &b.geometry);
            if o > 0.1 {
                m.issues.push(Issue { severity: Severity::Fout, layer: a.layer.clone(), handles: a.handles.iter().chain(&b.handles).copied().collect(), message: format!("{} {} en {} {} overlappen (± {o:.1} m²): op maaiveld mogen vlakken niet overlappen.", a.class.name(), short(&a.lokaal_id), b.class.name(), short(&b.lokaal_id)) });
            }
        }
    }
    let touched: Vec<&str> = m.features.iter().map(|f| f.lokaal_id.as_str()).collect();
    for a in &live {
        for e in existing.iter().filter(|e| !touched.contains(&e.lokaal_id.as_str()) && e.class.partition() && e.data.get("relatieve_hoogteligging").is_none_or(|v| v == "0")) {
            let o = overlap(&a.geometry, &e.geometry);
            if o > 0.1 {
                m.issues.push(Issue { severity: Severity::Waarschuwing, layer: a.layer.clone(), handles: a.handles.clone(), message: format!("{} {} overlapt het bestaande {} {} (± {o:.1} m²): neem dat ook mee (laag R om te wijzigen, V om te beëindigen).", a.class.name(), short(&a.lokaal_id), e.class.name(), short(&e.lokaal_id)) });
            }
        }
    }
    m.issues.sort_by(|a, b| a.severity.cmp(&b.severity));
}

/// The date of a PDOK timestamp in Dutch legal time: objectBeginTijd
/// "2013-12-15T23:00:00Z" is 16 December. Summer time from the last Sunday
/// of March to the last Sunday of October, 01:00 UTC.
fn dutch_date(utc: &str) -> String {
    use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};
    let Ok(t) = NaiveDateTime::parse_from_str(utc.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S") else {
        return utc.chars().take(10).collect();
    };
    let last_sunday = |month: u32| {
        let mut d = NaiveDate::from_ymd_opt(t.year(), month + 1, 1).unwrap_or_default() - Duration::days(1);
        while d.weekday() != Weekday::Sun {
            d -= Duration::days(1);
        }
        d.and_hms_opt(1, 0, 0).unwrap_or_default()
    };
    let summer = t >= last_sunday(3) && t < last_sunday(10);
    (t + Duration::hours(if summer { 2 } else { 1 })).format("%Y-%m-%d").to_string()
}

fn short(id: &str) -> String {
    match id.split_once('.') {
        Some((b, rest)) if rest.len() > 8 => format!("{b}.{}…", &rest[..8]),
        _ => id.to_string(),
    }
}

// ── IMGeo-GML 2.1.1 ─────────────────────────────────────────────────────────

const DEF: &str = "http://www.geostandaarden.nl/imgeo/def/2.1#";
const SRS: &str = "urn:ogc:def:crs:EPSG::28992";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn pos_list(r: &[XY], close: bool) -> String {
    let mut s = String::new();
    for p in r.iter().chain(close.then(|| &r[0])) {
        let _ = write!(s, "{:.3} {:.3} ", p[0], p[1]);
    }
    s.trim_end().to_string()
}

fn polygon(outer: &[XY], holes: &[Vec<XY>]) -> String {
    let mut s = format!("<gml:Polygon srsName=\"{SRS}\"><gml:exterior><gml:LinearRing><gml:posList srsDimension=\"2\">{}</gml:posList></gml:LinearRing></gml:exterior>", pos_list(outer, true));
    for h in holes {
        let _ = write!(s, "<gml:interior><gml:LinearRing><gml:posList srsDimension=\"2\">{}</gml:posList></gml:LinearRing></gml:interior>", pos_list(h, true));
    }
    s.push_str("</gml:Polygon>");
    s
}

fn geometry_xml(g: &Geometry, multi: bool) -> String {
    match g {
        Geometry::Surface { outer, holes } if multi => format!("<gml:MultiSurface srsName=\"{SRS}\"><gml:surfaceMember>{}</gml:surfaceMember></gml:MultiSurface>", polygon(outer, holes)),
        Geometry::Surface { outer, holes } => polygon(outer, holes),
        Geometry::Surfaces(parts) => {
            let mut s = format!("<gml:MultiSurface srsName=\"{SRS}\">");
            for (o, h) in parts {
                let _ = write!(s, "<gml:surfaceMember>{}</gml:surfaceMember>", polygon(o, h));
            }
            s + "</gml:MultiSurface>"
        }
        Geometry::Line(l) => format!("<gml:LineString srsName=\"{SRS}\"><gml:posList srsDimension=\"2\">{}</gml:posList></gml:LineString>", pos_list(l, false)),
        Geometry::Point(p) => format!("<gml:Point srsName=\"{SRS}\"><gml:pos srsDimension=\"2\">{:.3} {:.3}</gml:pos></gml:Point>", p[0], p[1]),
    }
}

/// A coded value, or the void value when it is not known.
fn code(tag: &str, class: Class, attr: Attr, f: &Feature) -> String {
    match f.attrs.get(&attr) {
        Some(v) => format!("<{tag} codeSpace=\"{DEF}{}\">{}</{tag}>", attr.list(class), esc(v)),
        None => format!("<{tag} codeSpace=\"{DEF}VoidReasonValue\">waardeOnbekend</{tag}>"),
    }
}

fn member(f: &Feature, bronhouder: &str, now: &(String, String)) -> String {
    let c = f.class;
    let element = match c {
        Class::Wegdeel => "tra:TrafficArea",
        Class::OndersteunendWegdeel => "tra:AuxiliaryTrafficArea",
        Class::BegroeidTerreindeel => "veg:PlantCover",
        Class::OnbegroeidTerreindeel => "imgeo:OnbegroeidTerreindeel",
        Class::Waterdeel => "imgeo:Waterdeel",
        Class::OndersteunendWaterdeel => "imgeo:OndersteunendWaterdeel",
        Class::Pand => "bui:BuildingPart",
        Class::Scheiding => "imgeo:Scheiding",
        Class::Paal => "imgeo:Paal",
    };
    let gml_id = format!("{}_{}", f.lokaal_id.replace(['-', ':'], ""), now.1.replace(['-', ':', 'T'], ""));
    let mut s = String::new();
    let _ = writeln!(s, "  <cit:cityObjectMember>\n    <!-- {} {}{} -->", f.kind.label(), c.name(), if f.plan { " (plantopografie)" } else { "" });
    let _ = write!(s, "    <{element} gml:id=\"{}\">", esc(&gml_id));
    let creation = match f.kind {
        Kind::Nieuw => now.0.clone(),
        _ => f.creation.clone().unwrap_or_else(|| now.0.clone()),
    };
    let _ = write!(s, "<cit:creationDate>{creation}</cit:creationDate>");
    if f.kind == Kind::Beeindigd {
        let _ = write!(s, "<cit:terminationDate>{}</cit:terminationDate>", now.0);
    }
    let _ = write!(s, "<imgeo:identificatie><imgeo:NEN3610ID><imgeo:namespace>NL.IMGeo</imgeo:namespace><imgeo:lokaalID>{}</imgeo:lokaalID></imgeo:NEN3610ID></imgeo:identificatie>", esc(&f.lokaal_id));
    let _ = write!(s, "<imgeo:tijdstipRegistratie>{}</imgeo:tijdstipRegistratie>", now.1);
    if f.kind == Kind::Beeindigd {
        let _ = write!(s, "<imgeo:eindRegistratie>{}</imgeo:eindRegistratie>", now.1);
    }
    let _ = write!(s, "<imgeo:bronhouder>{}</imgeo:bronhouder><imgeo:inOnderzoek>false</imgeo:inOnderzoek><imgeo:relatieveHoogteligging>{}</imgeo:relatieveHoogteligging>", esc(bronhouder), f.hoogteligging);
    if f.plan {
        let _ = write!(s, "<imgeo:bgt-status codeSpace=\"{DEF}VoidReasonValue\">geenWaarde</imgeo:bgt-status><imgeo:plus-status codeSpace=\"{DEF}StatusPlus\">plan</imgeo:plus-status>");
    } else {
        let _ = write!(s, "<imgeo:bgt-status codeSpace=\"{DEF}Status\">bestaand</imgeo:bgt-status><imgeo:plus-status codeSpace=\"{DEF}VoidReasonValue\">geenWaarde</imgeo:plus-status>");
    }
    let geom = |tag: &str, multi: bool| format!("<imgeo:{tag}>{}</imgeo:{tag}>", geometry_xml(&f.geometry, multi));
    let talud = |tag: &str| format!("<imgeo:{tag}>{}</imgeo:{tag}>", f.op_talud);
    let opt_code = |tag: &str, attr: Attr| if f.attrs.contains_key(&attr) || !attr.list(c).is_empty() { code(tag, c, attr, f) } else { String::new() };
    match c {
        Class::Wegdeel => {
            s += &code("tra:function", c, Attr::Functie, f);
            s += &code("tra:surfaceMaterial", c, Attr::Fysiek, f);
            s += &opt_code("imgeo:plus-functieWegdeel", Attr::PlusFunctie);
            s += &opt_code("imgeo:plus-fysiekVoorkomenWegdeel", Attr::PlusFysiek);
            s += &talud("wegdeelOpTalud");
            s += &geom("geometrie2dWegdeel", false);
        }
        Class::OndersteunendWegdeel => {
            s += &code("tra:function", c, Attr::Functie, f);
            s += &code("tra:surfaceMaterial", c, Attr::Fysiek, f);
            s += &opt_code("imgeo:plus-functieOndersteunendWegdeel", Attr::PlusFunctie);
            s += &opt_code("imgeo:plus-fysiekVoorkomenOndersteunendWegdeel", Attr::PlusFysiek);
            s += &talud("ondersteunendWegdeelOpTalud");
            s += &geom("geometrie2dOndersteunendWegdeel", false);
        }
        Class::BegroeidTerreindeel => {
            s += &code("veg:class", c, Attr::Fysiek, f);
            s += &geom("geometrie2dBegroeidTerreindeel", false);
            s += &talud("begroeidTerreindeelOpTalud");
            s += &opt_code("imgeo:plus-fysiekVoorkomen", Attr::PlusFysiek);
        }
        Class::OnbegroeidTerreindeel => {
            s += &code("imgeo:bgt-fysiekVoorkomen", c, Attr::Fysiek, f);
            s += &talud("onbegroeidTerreindeelOpTalud");
            s += &opt_code("imgeo:plus-fysiekVoorkomen", Attr::PlusFysiek);
            s += &geom("geometrie2dOnbegroeidTerreindeel", false);
        }
        Class::Waterdeel => {
            s += &code("wat:class", c, Attr::Type, f);
            s += &opt_code("imgeo:plus-type", Attr::PlusType);
            s += &geom("geometrie2dWaterdeel", false);
        }
        Class::OndersteunendWaterdeel => {
            s += &code("wat:class", c, Attr::Type, f);
            s += &opt_code("imgeo:plus-type", Attr::PlusType);
            s += &geom("geometrie2dOndersteunendWaterdeel", false);
        }
        Class::Pand => {
            if let Some(b) = &f.bag_pnd {
                let _ = write!(s, "<imgeo:identificatieBAGPND>{}</imgeo:identificatieBAGPND>", esc(b));
            }
            s += &geom("geometrie2dGrondvlak", true);
        }
        Class::Scheiding => {
            s += &geom("geometrie2dOverigeConstructie", false);
            s += &code("imgeo:bgt-type", c, Attr::Type, f);
            s += &opt_code("imgeo:plus-type", Attr::PlusType);
        }
        Class::Paal => {
            s += &code("cif:function", c, Attr::Type, f);
            s += &opt_code("imgeo:plus-type", Attr::PlusType);
            s += &geom("geometrie2dPaal", false);
        }
    }
    let _ = write!(s, "</{element}>\n  </cit:cityObjectMember>\n");
    s
}

/// The mutation as one IMGeo-GML 2.1.1 file (CityModel with the objects).
pub fn gml(m: &Mutation, now: &(String, String)) -> String {
    let mut s = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!-- BGT-mutatie uit een NLCS-tekening (Open Infra). IMGeo 2.1.1; aanleveren aan de LV-BGT doet de bronhouder (StUF-Geo IMGeo, via BRAVO). -->\n",
        "<cit:CityModel xmlns:cit=\"http://www.opengis.net/citygml/2.0\" xmlns:imgeo=\"http://www.geostandaarden.nl/imgeo/2.1\"",
        " xmlns:tra=\"http://www.opengis.net/citygml/transportation/2.0\" xmlns:veg=\"http://www.opengis.net/citygml/vegetation/2.0\"",
        " xmlns:wat=\"http://www.opengis.net/citygml/waterbody/2.0\" xmlns:bui=\"http://www.opengis.net/citygml/building/2.0\"",
        " xmlns:cif=\"http://www.opengis.net/citygml/cityfurniture/2.0\" xmlns:gml=\"http://www.opengis.net/gml\"",
        " xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
        " xsi:schemaLocation=\"http://www.geostandaarden.nl/imgeo/2.1 http://schemas.geonovum.nl/imgeo/2.1/imgeo-2.1.1.xsd\">\n",
    ));
    for f in &m.features {
        s += &member(f, &m.bronhouder, now);
    }
    s.push_str("</cit:CityModel>\n");
    s
}

// ── Command ─────────────────────────────────────────────────────────────────

/// Read the active drawing, write `<drawing>_bgt-mutatie.gml` and `.md`
/// beside it, and say what is in them. For `GIS_BGT_MUTATIE` and the window.
pub fn export(bronhouder: &str, as_built: bool, only: Option<std::collections::HashSet<Handle>>) -> Result<String, String> {
    let site = crate::bridge::site().ok_or("stel eerst de projectlocatie in (GIS → Locatie)")?;
    let t = crate::bridge::target().ok_or("geen tekening actief")?;
    let doc = crate::bridge::snapshot()?;
    let opt = Options { bronhouder: bronhouder.into(), as_built, unit_factor: t.unit_factor, only, now: Options::now() };
    let m = build(&doc, &site, &opt);
    if !m.bronhouder_ok() {
        return Err(m.issues.first().map(|i| i.message.clone()).unwrap_or_default());
    }
    let gml_path = crate::bridge::beside_drawing("bgt-mutatie.gml");
    let md_path = crate::bridge::beside_drawing("bgt-mutatie.md");
    let gml_name = gml_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if m.features.is_empty() {
        let _ = std::fs::remove_file(&gml_path);
    } else {
        std::fs::write(&gml_path, gml(&m, &opt.now)).map_err(|e| format!("{}: {e}", gml_path.display()))?;
    }
    std::fs::write(&md_path, markdown(&m, &crate::bridge::drawing_name(), &gml_name, &opt.now)).map_err(|e| format!("{}: {e}", md_path.display()))?;
    crate::civiel::api::note("bronhouder", m.bronhouder.clone());
    crate::civiel::api::note("nieuw", m.count(Kind::Nieuw));
    crate::civiel::api::note("gewijzigd", m.count(Kind::Gewijzigd));
    crate::civiel::api::note("beeindigd", m.count(Kind::Beeindigd));
    crate::civiel::api::note("fouten", m.errors());
    crate::civiel::api::note("meldingen", m.issues.iter().map(|i| i.message.clone()).collect::<Vec<_>>());
    if !m.features.is_empty() {
        crate::civiel::api::note("gml", gml_path.display().to_string());
    }
    crate::civiel::api::note("rapport", md_path.display().to_string());
    if m.features.is_empty() {
        return Ok(format!("BGT-mutatie: niets om te leveren ({}). Rapport: {}", summary(&m), md_path.display()));
    }
    Ok(format!("BGT-mutatie: {}. GML: {} — rapport: {}", summary(&m), gml_path.display(), md_path.display()))
}

// ── Report ──────────────────────────────────────────────────────────────────

pub fn summary(m: &Mutation) -> String {
    format!(
        "{} nieuw, {} gewijzigd, {} beëindigd; {} fout(en), {} waarschuwing(en)",
        m.count(Kind::Nieuw),
        m.count(Kind::Gewijzigd),
        m.count(Kind::Beeindigd),
        m.errors(),
        m.issues.iter().filter(|i| i.severity == Severity::Waarschuwing).count()
    )
}

pub fn markdown(m: &Mutation, drawing: &str, gml_name: &str, now: &(String, String)) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# BGT-mutatie — {drawing}\n");
    let _ = writeln!(s, "{} · bronhouder {} · {}\n", now.1.replace('T', " "), if m.bronhouder.is_empty() { "onbekend" } else { &m.bronhouder }, summary(m));
    let _ = writeln!(s, "Bestand: `{gml_name}` (IMGeo-GML 2.1.1). Het BGT-pakket van de bronhouder leest dit in; aanleveren aan de LV-BGT gaat als StUF-Geo IMGeo via BRAVO.\n");
    let plan = m.features.iter().filter(|f| f.plan).count();
    if plan > 0 {
        let _ = writeln!(s, "{plan} object(en) uit lagen met status N zijn **plantopografie** (plus-status `plan`): toekomstige topografie hoort niet in de BGT zelf. Is het werk zo aangelegd, maak de mutatie dan opnieuw als gerealiseerd.\n");
    }
    if !m.issues.is_empty() {
        let _ = writeln!(s, "## Meldingen\n\n| Ernst | Laag | Handles | Melding |\n|---|---|---|---|");
        for i in &m.issues {
            let sev = match i.severity {
                Severity::Fout => "Fout",
                Severity::Waarschuwing => "Waarschuwing",
                Severity::Melding => "Melding",
            };
            let h: Vec<String> = i.handles.iter().take(6).map(|h| format!("{:X}", u64::from(*h))).collect();
            let _ = writeln!(s, "| {sev} | {} | {} | {} |", i.layer, h.join(" "), i.message.replace('|', "/"));
        }
        s.push('\n');
    }
    let _ = writeln!(s, "## Objecten\n\n| Mutatie | Klasse | lokaalID | Functie / type | Fysiek voorkomen | m² | Bron van de kenmerken |\n|---|---|---|---|---|---|---|");
    for f in &m.features {
        let ft = [Attr::Functie, Attr::Type].iter().filter_map(|a| f.attrs.get(a)).cloned().chain([Attr::PlusFunctie, Attr::PlusType].iter().filter_map(|a| f.attrs.get(a)).map(|v| format!("({v})"))).collect::<Vec<_>>().join(" ");
        let fy = [Attr::Fysiek, Attr::PlusFysiek].iter().filter_map(|a| f.attrs.get(a)).cloned().collect::<Vec<_>>().join(" / ");
        let area = f.geometry.area();
        let _ = writeln!(s, "| {}{} | {} | {} | {} | {} | {} | {} |", f.kind.label(), if f.plan { " (plan)" } else { "" }, f.class.name(), f.lokaal_id, ft, fy, if area > 0.0 { format!("{area:.1}") } else { String::new() }, f.source);
    }
    let _ = writeln!(s, "\n## Niet gecontroleerd\n\n- Gaten in de maaiveldbedekking: alleen de bronhouder ziet de hele omgeving. Neem aangrenzende objecten mee waarvan de rand verandert.\n- Gedeelde hoekpunten met buurobjecten (een hoekpunt van de buur op de rand moet ook in de eigen rand zitten).\n- Plaatsbepalingspunten (PBP) en WAS=WAS: die voegt het BGT-pakket van de bronhouder toe bij het maken van het StUF-bericht.\n- {} element(en) niet op een NLCS-laag voor de BGT (tijdelijk, annotatie of geen NLCS-laag); {} bestaand(e) BGT-object(en) ongewijzigd.", m.skipped, m.unchanged);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{LwPolyline, Vector2};

    fn rect(layer: &str, x0: f64, y0: f64, x1: f64, y1: f64, data: &[(&str, &str)]) -> EntityType {
        let mut p = LwPolyline::new();
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            p.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        p.is_closed = true;
        let mut e = EntityType::LwPolyline(p);
        e.common_mut().layer = layer.into();
        if !data.is_empty() {
            e.common_mut().extended_data.add_record(crate::bridge::object_data(data.iter().map(|(k, v)| (*k, v.to_string()))));
        }
        e
    }

    /// RD in the drawing, no project area (no filter on it).
    fn site() -> Site {
        let mut s = Site::default();
        s.crs = "EPSG:28992".into();
        s.absolute = true;
        s.width = 0.0;
        s.height = 0.0;
        s
    }

    fn opt() -> Options {
        Options { bronhouder: String::new(), as_built: false, unit_factor: 1.0, only: None, now: ("2026-10-28".into(), "2026-10-28T10:15:00".into()) }
    }

    const ROAD: &[(&str, &str)] = &[("bron", "BGT"), ("objecttype", "wegdeel"), ("lokaal_id", "G0505.047fc0f61802c003e0532b00500a66e1"), ("bronhouder", "G0505"), ("functie", "rijbaan lokale weg"), ("fysiek_voorkomen", "open verharding"), ("relatieve_hoogteligging", "0"), ("creation_date", "2013-12-15T23:00:00Z")];

    #[test]
    fn mapping_backwards_is_ambiguous_for_asphalt() {
        let c = candidates(nlcs::object_by_name("VH", "GESLOTENVERHARDING_ASFALT").unwrap().id);
        assert!(c.classes.contains(&Class::Wegdeel) && c.classes.contains(&Class::OndersteunendWegdeel));
        // The material is the same in every wegdeel row; the function is not.
        assert_eq!(c.agreed(Class::Wegdeel, None, Attr::PlusFysiek).unwrap().as_deref(), Some("ASFALT"));
        assert!(c.agreed(Class::Wegdeel, None, Attr::Functie).is_err());
        assert_eq!(canonical(Class::Wegdeel, Attr::Functie, "OV-BAAN"), Some("OV-baan"));
        assert_eq!(canonical(Class::OndersteunendWegdeel, Attr::PlusFysiek, "GRAS- EN KRUIDACHTINGEN"), Some("gras- en kruidachtigen"));
    }

    #[test]
    fn new_asphalt_takes_the_road_beneath() {
        let mut doc = CadDocument::new();
        let _ = doc.add_entity(rect("B-WE-VH-OPENVERHARDING-G", 104000.0, 425000.0, 104040.0, 425010.0, ROAD));
        let _ = doc.add_entity(rect("R-WE-VH-GESLOTENVERHARDING_ASFALT-G", 104005.0, 425002.0, 104030.0, 425008.0, &[]));
        let m = build(&doc, &site(), &opt());
        assert_eq!(m.bronhouder, "G0505");
        let f = m.features.iter().find(|f| f.kind == Kind::Nieuw).expect("new road part");
        assert_eq!(f.class, Class::Wegdeel);
        assert!(!f.plan);
        assert_eq!(f.attrs.get(&Attr::Functie).map(String::as_str), Some("rijbaan lokale weg"));
        assert_eq!(f.attrs.get(&Attr::Fysiek).map(String::as_str), Some("gesloten verharding"));
        assert_eq!(f.attrs.get(&Attr::PlusFysiek).map(String::as_str), Some("asfalt"));
        assert!(f.lokaal_id.starts_with("G0505.") && f.lokaal_id.len() == 38, "{}", f.lokaal_id);
        assert!(f.source.contains("overlap"), "{}", f.source);
        // The old road part stays (status B) under the new one: the report says to take it along.
        assert!(m.issues.iter().any(|i| i.severity == Severity::Waarschuwing && i.message.contains("overlapt het bestaande")), "{:?}", m.issues);
    }

    #[test]
    fn plan_on_n_layers_and_ambiguous_ones_in_the_report() {
        let mut doc = CadDocument::new();
        let _ = doc.add_entity(rect("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", 0.0, 0.0, 10.0, 10.0, &[("objecttype", "wegdeel"), ("functie", "fietspad")]));
        let _ = doc.add_entity(rect("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", 20.0, 0.0, 30.0, 10.0, &[]));
        let mut o = opt();
        o.bronhouder = "G0505".into();
        let m = build(&doc, &site(), &o);
        assert_eq!(m.features.len(), 1);
        assert!(m.features[0].plan);
        assert_eq!(m.features[0].attrs.get(&Attr::Functie).map(String::as_str), Some("fietspad"));
        assert!(m.issues.iter().any(|i| i.severity == Severity::Fout && i.message.contains("Niet eenduidig")), "{:?}", m.issues);
    }

    #[test]
    fn ended_and_changed_objects_keep_their_id() {
        let mut doc = CadDocument::new();
        let _ = doc.add_entity(rect("V-WE-VH-OPENVERHARDING-G", 0.0, 0.0, 10.0, 10.0, ROAD));
        let mut other: Vec<(&str, &str)> = ROAD.to_vec();
        other[2] = ("lokaal_id", "G0505.00000000000000000000000000000002");
        let _ = doc.add_entity(rect("R-WE-VH-OPENVERHARDING-G", 10.0, 0.0, 20.0, 10.0, &other));
        let m = build(&doc, &site(), &opt());
        assert_eq!(m.count(Kind::Beeindigd), 1);
        assert_eq!(m.count(Kind::Gewijzigd), 1);
        let g = gml(&m, &opt().now);
        let doc = roxmltree::Document::parse(&g).expect("well-formed GML");
        let ns = "http://www.geostandaarden.nl/imgeo/2.1";
        let ids: Vec<&str> = doc.descendants().filter(|n| n.has_tag_name((ns, "lokaalID"))).filter_map(|n| n.text()).collect();
        assert!(ids.contains(&"G0505.047fc0f61802c003e0532b00500a66e1"));
        assert!(g.contains("<cit:terminationDate>2026-10-28</cit:terminationDate>"));
        // The changed one keeps its objectBeginTijd (23:00 UTC is midnight in winter).
        assert!(g.contains("<cit:creationDate>2013-12-16</cit:creationDate>"), "{g}");
        assert_eq!(dutch_date("2014-07-20T22:00:00Z"), "2014-07-21");
        assert!(g.contains("codeSpace=\"http://www.geostandaarden.nl/imgeo/def/2.1#FunctieWeg\">rijbaan lokale weg</tra:function>"));
    }

    /// Every class once; with OI_GML_OUT=<file> the GML is written there (for
    /// a check against the IMGeo 2.1.1 schema).
    #[test]
    fn all_classes_to_gml() {
        let mut doc = CadDocument::new();
        let mut add = |e: EntityType| {
            let _ = doc.add_entity(e);
        };
        add(rect("R-WE-VH-GESLOTENVERHARDING_ASFALT-G", 0.0, 0.0, 10.0, 10.0, &[("objecttype", "wegdeel"), ("functie", "rijbaan lokale weg")]));
        add(rect("R-WE-VH-GESLOTENVERHARDING_ASFALT-G", 10.0, 0.0, 12.0, 10.0, &[("objecttype", "ondersteunendwegdeel"), ("functie", "berm")]));
        add(rect("R-WE-GR-GROENVOORZIENING-G", 12.0, 0.0, 20.0, 10.0, &[("objecttype", "begroeidterreindeel")]));
        add(rect("R-WE-OG-TERREIN_ERF-G", 20.0, 0.0, 30.0, 10.0, &[]));
        add(rect("R-WE-OG-WATER_BEEK-G", 30.0, 0.0, 40.0, 10.0, &[]));
        add(rect("R-WE-OG-BEBOUWING-G", 40.0, 0.0, 50.0, 10.0, &[("bag_pnd", "0505100000013372")]));
        let mut fence = ocs_plugin_api::host::codec::LwPolyline::new();
        fence.add_point_with_bulge(Vector2::new(0.0, 12.0), 0.0);
        fence.add_point_with_bulge(Vector2::new(20.0, 12.0), 0.0);
        let mut fence = EntityType::LwPolyline(fence);
        fence.common_mut().layer = "R-WE-IE-TERREINAFSCHEIDING_KUNSTMATIG_HEKWERK-G".into();
        add(fence);
        let mut pole = EntityType::Circle(ocs_plugin_api::host::codec::Circle::from_center_radius(ocs_plugin_api::host::codec::Vector3::new(5.0, 11.0, 0.0), 0.2));
        pole.common_mut().layer = "R-WE-IE-MEUBILAIR-S".into();
        pole.common_mut().extended_data.add_record(crate::bridge::object_data([("objecttype", "paal".to_string()), ("plus_type", "lichtmast".to_string())]));
        add(pole);
        add(rect("V-WE-VH-OPENVERHARDING-G", 0.0, 20.0, 10.0, 30.0, ROAD));
        let mut other: Vec<(&str, &str)> = ROAD.to_vec();
        other[2] = ("lokaal_id", "G0505.00000000000000000000000000000002");
        add(rect("R-WE-VH-OPENVERHARDING-G", 10.0, 20.0, 20.0, 30.0, &other));
        let mut o = opt();
        o.bronhouder = "G0505".into();
        let m = build(&doc, &site(), &o);
        let g = gml(&m, &o.now);
        roxmltree::Document::parse(&g).expect("well-formed GML");
        if let Ok(path) = std::env::var("OI_GML_OUT") {
            std::fs::write(path, &g).unwrap();
            std::fs::write(std::env::var("OI_GML_OUT").unwrap() + ".md", markdown(&m, "test", "test.gml", &o.now)).unwrap();
        }
        let classes: Vec<Class> = m.features.iter().map(|f| f.class).collect();
        for c in [Class::Wegdeel, Class::OndersteunendWegdeel, Class::BegroeidTerreindeel, Class::OnbegroeidTerreindeel, Class::Waterdeel, Class::Pand, Class::Scheiding, Class::Paal] {
            assert!(classes.contains(&c), "{c:?} missing; {:?}", m.issues);
        }
    }

    #[test]
    fn legend_and_far_away_items_are_left_out() {
        let mut s = site();
        s.origin_e = 0.0;
        s.origin_n = 0.0;
        s.width = 100.0;
        s.height = 100.0;
        let mut doc = CadDocument::new();
        let road = [("objecttype", "wegdeel"), ("functie", "fietspad")];
        let _ = doc.add_entity(rect("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", 0.0, 0.0, 10.0, 10.0, &road));
        // A legend sample of the drawing sheet, inside the area.
        let _ = doc.add_entity(rect("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", 20.0, 0.0, 30.0, 10.0, &[("bron", "tekenblad")]));
        // Something far outside the project area, without object data.
        let _ = doc.add_entity(rect("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", 900.0, 900.0, 910.0, 910.0, &[]));
        let mut o = opt();
        o.bronhouder = "G0505".into();
        let m = build(&doc, &s, &o);
        assert_eq!(m.features.len(), 1, "{:?}", m.issues);
        assert!(m.issues.iter().any(|i| i.message.contains("buiten het projectgebied")), "{:?}", m.issues);
    }

    #[test]
    fn geometry_rules() {
        // A bow tie crosses itself.
        let bow = [[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]];
        assert!(self_crossing(&bow, true).is_some());
        assert!(self_crossing(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true).is_none());
        // Rounded half up to mm, repeated points out.
        assert_eq!(clean(&[[1.0004, 2.0005], [1.0004, 2.0005], [3.0, 4.0], [1.0004, 2.0005]], true), vec![[1.0, 2.001], [3.0, 4.0]]);
        // A ring inside another is a hole, oriented clockwise.
        let parts = nest(vec![vec![[2.0, 2.0], [4.0, 2.0], [4.0, 4.0], [2.0, 4.0]], vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0]]]);
        assert_eq!(parts.len(), 1);
        assert!(ring_area(&parts[0].0) > 0.0 && ring_area(&parts[0].1[0]) < 0.0);
        let a = Geometry::Surface { outer: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], holes: vec![] };
        let b = Geometry::Surface { outer: vec![[5.0, 0.0], [15.0, 0.0], [15.0, 10.0], [5.0, 10.0]], holes: vec![] };
        assert!((overlap(&a, &b) - 50.0).abs() < 1.5, "{}", overlap(&a, &b));
    }
}
