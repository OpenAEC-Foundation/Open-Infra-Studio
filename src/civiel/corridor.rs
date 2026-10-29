//! Corridor (Road "Road"): an assembly — the template cross section — swept
//! along an axis and its design profile.
//!
//! An assembly lists its parts per side, outward from the axis: lanes,
//! gutters, kerbs, footways, cycle paths, parking
//! lanes, verges, a central reserve and ditches. Each part has a width, a
//! cross slope and a pavement build-up (layers). Lanes tilt in curves
//! (superelevation, with a runoff before and after). A part may carry a
//! condition (only in cut or only in fill, checked where the part starts)
//! and targets: its width to an offset or a line in the drawing, its outer
//! level to the ground, a height or a 3D line. At the outer edge a slope runs
//! down (fill) or up (cut) until it meets the existing ground (daylight); a
//! ditch as the last part daylights with its back slope.
//!
//! The result is:
//! - the design surface (OG-TRIANGULATIE, status N);
//! - the formation (top of the foundation, VH-FUNDERING) and the subgrade
//!   (underside of the build-up, GW-CUNET_BODEM) as surfaces;
//! - 3D feature lines along every part edge;
//! - cut and fill per section with average-end-area volumes, to the top and
//!   to the subgrade, and a mass balance (CSV);
//! - material quantities: volume per layer material, area per surface, kerb
//!   lengths (CSV);
//! - optionally section views with the layers.
//!
//! Assemblies are JSON files (presets and the user's own): see `profieltype`.

use std::collections::{BTreeMap, HashMap, HashSet};

use ocs_plugin_api::host::codec::entities::Polyline3D;
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use super::as_profiel::{self, Alignment, Pvi, Seg, SectionView};
use super::model3d::MeshBuilder;
use super::terrein::{Tin, P3};
use super::{record, station, v3, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::{self, Element, LayerSpec, Status};

/// A message in the interface language: `trf!("nl {}", "en {}", args…)`.
macro_rules! trf {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match $crate::lang::current() {
            $crate::lang::Lang::Nl => format!($nl $(, $a)*),
            $crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}
pub(crate) use trf;

/// A number with a decimal comma in Dutch.
pub fn num(v: f64, decimals: usize) -> String {
    let t = format!("{v:.decimals$}");
    if crate::lang::current() == crate::lang::Lang::Nl { t.replace('.', ",") } else { t }
}

/// An NLCS object (its id in the NLCS table): the layer of a surface, a
/// material or a line.
pub type NlcsId = u32;

/// The NLCS object `group`/`name`; 0 when the table lacks it.
pub fn nlcs_id(group: &str, name: &str) -> NlcsId {
    nlcs::object_by_name(group, name).map(|o| o.id).unwrap_or(0)
}

/// The layer of an NLCS object in the context's status and discipline.
pub fn nlcs_layer(ctx: &Ctx, id: NlcsId, el: Element) -> Result<LayerSpec, String> {
    match nlcs::object(id) {
        Some(o) => Ok(nlcs::layer(o, ctx.opt.nlcs.status, &ctx.opt.nlcs.discipline, el)),
        None => ctx.layer("VH", "FUNDERING", el),
    }
}

/// The NLCS object of a material or surface by its name: asphalt layers,
/// paving, foundations, grass. Unknown materials go on VH-FUNDERING.
pub fn material_nlcs(material: &str) -> NlcsId {
    let m = material.to_lowercase();
    let has = |k: &str| m.contains(k);
    let asphalt = has("asfalt") || m.starts_with("ac ") || has(" ac ") || has("surf") || has(" bin") || has(" base");
    let name = if has("zoab") {
        "GESLOTENVERHARDING_ASFALT_ZOAB"
    } else if has("granulaat") || has("agrac") {
        "FUNDERING_GRANULAAT"
    } else if has("zandcement") {
        "FUNDERING_ZANDCEMENTSTABILISATIE"
    } else if asphalt {
        if has("rood") {
            "GESLOTENVERHARDING_ASFALT_ROOD"
        } else if has("deklaag") || has("surf") {
            "GESLOTENVERHARDING_ASFALT_DEKLAAG"
        } else if has("tussenlaag") || has(" bin") {
            "GESLOTENVERHARDING_ASFALT_TUSSENLAAG"
        } else if has("onderlaag") || has(" base") {
            "GESLOTENVERHARDING_ASFALT_ONDERLAAG"
        } else {
            "GESLOTENVERHARDING_ASFALT"
        }
    } else if has("betonstraatsteen") || has("klinker") || has("bss") {
        "OPENVERHARDING_BETONSTRAATSTEEN"
    } else if has("straatbaksteen") || has("gebakken") {
        "OPENVERHARDING_STRAATBAKSTEEN"
    } else if has("tegel") {
        "OPENVERHARDING_TEGEL"
    } else if has("natuursteen") {
        "OPENVERHARDING_NATUURSTEEN"
    } else if has("beton") {
        "GESLOTENVERHARDING_BETON"
    } else if has("grind") {
        "HALFVERHARDING_GRIND"
    } else if has("gras") {
        return nlcs_id("GR", "GRAS_BERM");
    } else if has("zand") || has("straatlaag") {
        "FUNDERING_ZAND"
    } else {
        "FUNDERING"
    };
    nlcs_id("VH", name)
}

/// Whether a layer belongs to the surfacing (asphalt, paving with its
/// bedding) rather than the foundation (granulate, sand): the formation lies
/// under the surfacing.
pub fn bound_default(material: &str) -> bool {
    let m = material.to_lowercase();
    let foundation = m.contains("granulaat") || m.contains("agrac") || m.contains("zandcement") || m.contains("fundering") || m.contains("zandbed") || m.contains("cunet");
    let sand = m.contains("zand") && !m.contains("straat") && !m.contains("breker");
    !(foundation || sand)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartKind {
    Lane,
    Shoulder,
    Crest,
    Kerb,
    Gutter,
    Footway,
    CyclePath,
    Parking,
    Median,
    Verge,
    Ditch,
}

impl PartKind {
    pub const ALL: [PartKind; 11] = [
        PartKind::Lane,
        PartKind::Shoulder,
        PartKind::Crest,
        PartKind::Kerb,
        PartKind::Gutter,
        PartKind::Footway,
        PartKind::CyclePath,
        PartKind::Parking,
        PartKind::Median,
        PartKind::Verge,
        PartKind::Ditch,
    ];

    /// The point code of the part's outer edge.
    pub fn code(self) -> &'static str {
        match self {
            PartKind::Lane => "RS",
            PartKind::Shoulder => "BE",
            PartKind::Crest => "KR",
            PartKind::Kerb => "BA",
            PartKind::Gutter => "GO",
            PartKind::Footway => "TR",
            PartKind::CyclePath => "FP",
            PartKind::Parking => "PS",
            PartKind::Median => "MB",
            PartKind::Verge => "SB",
            PartKind::Ditch => "SL",
        }
    }

    /// The Dutch name, as in the JSON files.
    pub fn name(self) -> &'static str {
        match self {
            PartKind::Lane => "rijstrook",
            PartKind::Shoulder => "berm",
            PartKind::Crest => "kruin",
            PartKind::Kerb => "band",
            PartKind::Gutter => "goot",
            PartKind::Footway => "trottoir",
            PartKind::CyclePath => "fietspad",
            PartKind::Parking => "parkeerstrook",
            PartKind::Median => "middenberm",
            PartKind::Verge => "scheidingsberm",
            PartKind::Ditch => "sloot",
        }
    }

    /// The name in the interface language.
    pub fn label(self) -> &'static str {
        let tr = crate::lang::tr;
        match self {
            PartKind::Lane => tr("rijstrook", "lane"),
            PartKind::Shoulder => tr("berm", "shoulder"),
            PartKind::Crest => tr("kruin", "crest"),
            PartKind::Kerb => tr("band", "kerb"),
            PartKind::Gutter => tr("goot", "gutter"),
            PartKind::Footway => tr("trottoir", "footway"),
            PartKind::CyclePath => tr("fietspad", "cycle path"),
            PartKind::Parking => tr("parkeerstrook", "parking lane"),
            PartKind::Median => tr("middenberm", "central reserve"),
            PartKind::Verge => tr("scheidingsberm", "separating verge"),
            PartKind::Ditch => tr("sloot", "ditch"),
        }
    }

    pub fn parse(s: &str) -> Option<PartKind> {
        let s = s.trim().to_lowercase();
        let alias = match s.as_str() {
            "rijbaan" => "rijstrook",
            "kantopsluiting" | "trottoirband" | "opsluitband" => "band",
            "voetpad" => "trottoir",
            "parkeren" | "parkeerhaven" => "parkeerstrook",
            "watergang" | "greppel" => "sloot",
            "dijkkruin" => "kruin",
            other => other,
        };
        PartKind::ALL.into_iter().find(|k| k.name() == alias)
    }

    /// The surface's NLCS object when the part names none.
    pub fn default_surface(self) -> NlcsId {
        let (g, n) = match self {
            PartKind::Lane => ("VH", "GESLOTENVERHARDING_ASFALT"),
            PartKind::Shoulder | PartKind::Median | PartKind::Verge => ("GR", "GRAS_BERM"),
            PartKind::Crest => ("GW", "DIJK_KRUIN"),
            PartKind::Kerb => ("VH", "KANTOPSLUITING_TROTTOIRBAND"),
            PartKind::Gutter => ("VH", "GOOT_BSS"),
            PartKind::Footway => ("VH", "OPENVERHARDING_TEGEL"),
            PartKind::CyclePath => ("VH", "GESLOTENVERHARDING_ASFALT_ROOD"),
            PartKind::Parking => ("VH", "OPENVERHARDING_BETONSTRAATSTEEN"),
            PartKind::Ditch => ("GW", "WATERGANG_SLOOT_BODEM"),
        };
        nlcs_id(g, n)
    }

    /// Part of the carriageway: kerb returns at junctions follow its edge.
    pub fn carriageway(self) -> bool {
        matches!(self, PartKind::Lane | PartKind::Gutter | PartKind::Parking)
    }
}

/// One layer of a part's pavement build-up, from the top down.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub material: String,
    /// Thickness (m).
    pub thickness: f64,
    /// Surfacing (asphalt, paving and its bedding) rather than foundation.
    pub bound: bool,
    pub nlcs: NlcsId,
}

impl Layer {
    pub fn new(material: &str, thickness: f64) -> Layer {
        Layer { material: material.to_string(), thickness, bound: bound_default(material), nlcs: material_nlcs(material) }
    }
}

/// A ditch: bottom width, depth below its inner edge (or a bottom level),
/// and the side slopes 1:n.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ditch {
    pub bottom: f64,
    pub depth: f64,
    /// Bottom level (m NAP), in place of the depth.
    pub level: Option<f64>,
    pub inner: f64,
    pub outer: f64,
}

impl Default for Ditch {
    fn default() -> Ditch {
        Ditch { bottom: 0.5, depth: 0.8, level: None, inner: 1.5, outer: 1.5 }
    }
}

/// Use a part only in cut (ground above the design at the part's start) or
/// only in fill, when the depth lies between `min` and `max` (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    pub cut: bool,
    pub min: f64,
    pub max: f64,
}

/// What a part's width or outer level goes to.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Width: the outer edge at this distance from the axis (m).
    Offset(f64),
    /// Level: this height (m).
    Level(f64),
    /// Level: the existing ground at the outer edge.
    Ground,
    /// A named line in the drawing (`DOELEN=naam:handle`): its crossing with
    /// the section gives the width, its height the level.
    Line(String),
}

/// One subassembly, outward from the axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub kind: PartKind,
    /// A name for lists and labels ("trottoirband 13/15×25").
    pub name: String,
    pub width: f64,
    /// Cross slope (fraction; negative runs down away from the axis).
    pub slope: f64,
    /// Kerb: its top above the surface before it (m).
    pub upstand: f64,
    /// Kerb: the element's height (m).
    pub height: f64,
    /// Kerb: the horizontal run of its sloping front face (m).
    pub batter: f64,
    pub layers: Vec<Layer>,
    pub ditch: Option<Ditch>,
    pub condition: Option<Condition>,
    pub width_target: Option<Target>,
    pub level_target: Option<Target>,
    /// Limits of the slope a level target may give (fractions).
    pub slope_min: f64,
    pub slope_max: f64,
    /// The surface (and edge line) layer.
    pub surface: NlcsId,
}

impl Part {
    pub fn new(kind: PartKind, width: f64, slope: f64) -> Part {
        Part {
            kind,
            name: kind.name().to_string(),
            width,
            slope,
            upstand: 0.0,
            height: 0.0,
            batter: 0.0,
            layers: Vec::new(),
            ditch: (kind == PartKind::Ditch).then(Ditch::default),
            condition: None,
            width_target: None,
            level_target: None,
            slope_min: -0.5,
            slope_max: 0.5,
            surface: kind.default_surface(),
        }
    }

    /// The nominal width (a ditch: both slopes and its bottom).
    pub fn extent(&self) -> f64 {
        match (self.kind, self.ditch) {
            (PartKind::Ditch, Some(d)) => d.depth * (d.inner + d.outer) + d.bottom,
            _ => self.width,
        }
    }

    pub fn depth(&self) -> f64 {
        self.layers.iter().map(|l| l.thickness).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub name: String,
    pub description: String,
    pub sources: Vec<String>,
    /// Parts outward from the axis: left, right.
    pub sides: [Vec<Part>; 2],
    /// Fill and cut slopes 1:n (n metres across per metre height).
    pub fill: f64,
    pub cut: f64,
    /// How far a slope may run to find the ground (m).
    pub reach: f64,
    /// Superelevation in curves (fraction, 0 = none) and its runoff (m).
    pub superelevation: f64,
    pub runoff: f64,
}

impl Assembly {
    /// An assembly with the same parts on both sides.
    pub fn symmetric(name: &str, parts: Vec<Part>) -> Assembly {
        Assembly {
            name: name.to_string(),
            description: String::new(),
            sources: Vec::new(),
            sides: [parts.clone(), parts],
            fill: 3.0,
            cut: 2.0,
            reach: 60.0,
            superelevation: 0.0,
            runoff: 30.0,
        }
    }

    /// A preset or the user's own assembly by name (see `profieltype`).
    pub fn template(name: &str) -> Option<Assembly> {
        super::profieltype::find(name).ok()
    }

    /// The assembly with the arguments' changes, on both sides: RIJSTROOK,
    /// BERM (widths, m), HELLING, BERMHELLING (%, down > 0), TALUD,
    /// INGRAVING (1:n), KRUIN (half crest, m), VERKANTING (%), REIKWIJDTE (m).
    pub fn with_args(mut self, a: &super::api::Args) -> Result<Assembly, String> {
        let (lane_w, lane_s, berm_w, berm_s, crest_w) = (a.f64("RIJSTROOK")?, a.f64("HELLING")?, a.f64("BERM")?, a.f64("BERMHELLING")?, a.f64("KRUIN")?);
        for p in self.sides.iter_mut().flatten() {
            let (w, s) = match p.kind {
                PartKind::Lane => (lane_w, lane_s),
                PartKind::Shoulder => (berm_w, berm_s),
                PartKind::Crest => (crest_w, lane_s),
                _ => (None, None),
            };
            if let Some(w) = w {
                p.width = w;
            }
            if let Some(h) = s {
                p.slope = -h / 100.0;
            }
        }
        if let Some(v) = a.f64("TALUD")? {
            self.fill = v.max(0.1);
        }
        if let Some(v) = a.f64("INGRAVING")? {
            self.cut = v.max(0.1);
        }
        if let Some(v) = a.f64("VERKANTING")? {
            self.superelevation = v / 100.0;
        }
        if let Some(v) = a.f64("REIKWIJDTE")? {
            self.reach = v.max(1.0);
        }
        Ok(self)
    }

    pub const ARGS: [&'static str; 9] = ["RIJSTROOK", "BERM", "HELLING", "BERMHELLING", "TALUD", "INGRAVING", "KRUIN", "VERKANTING", "REIKWIJDTE"];

    /// The wider side's nominal width (m), without the daylight.
    pub fn half_width(&self) -> f64 {
        self.sides.iter().map(|s| s.iter().filter(|p| p.condition.is_none()).map(Part::extent).sum::<f64>()).fold(0.0, f64::max)
    }

    pub fn has_layers(&self) -> bool {
        self.sides.iter().flatten().any(|p| !p.layers.is_empty())
    }

    pub fn has_conditions(&self) -> bool {
        self.sides.iter().flatten().any(|p| p.condition.is_some())
    }

    /// The carriageway edge of a side (0 left, 1 right), as a distance from
    /// the axis (m): past a central reserve, over lanes, gutters and parking
    /// lanes up to the first other part. With the kerb found there, if any.
    pub fn edge(&self, side: usize) -> (f64, Option<&Part>) {
        let mut o = 0.0;
        let mut started = false;
        for p in &self.sides[side] {
            if p.condition.is_some() {
                continue;
            }
            if p.kind == PartKind::Median && !started {
                o += p.width;
                continue;
            }
            if p.kind.carriageway() {
                started = true;
                o += p.width;
                continue;
            }
            return (o, (p.kind == PartKind::Kerb).then_some(p));
        }
        (o, None)
    }
}

/// Where the design height along the axis comes from.
#[derive(Clone, Debug)]
pub enum Grade {
    /// The design profile of the axis (PVIs with vertical curves).
    Profile(Vec<Pvi>),
    /// A constant level (m), for a dike crest or a flat site.
    Level(f64),
    /// So many metres above the ground at the axis, smoothed over 100 m.
    AboveGround(f64),
}

/// The design profile drawn for an axis: PVIs from its object data.
pub fn profile_of(doc: &CadDocument, axis_name: &str) -> Option<Vec<Pvi>> {
    let e = doc
        .model_space_entities()
        .filter(|e| super::data_value(e, "ontwerpprofiel").is_some() && super::data_value(e, "as").as_deref() == Some(axis_name))
        .last()?;
    let pts: Vec<(f64, f64)> = super::data_value(e, "pvi")?
        .split(';')
        .filter_map(|t| {
            let (a, b) = t.split_once(':')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect();
    let l: f64 = super::data_value(e, "boog_m").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    (pts.len() >= 2).then(|| as_profiel::fit_curves(&pts, l))
}

/// The assembly the newest corridor on an axis was built with.
pub fn assembly_of(doc: &CadDocument, axis_name: &str) -> Option<String> {
    doc.model_space_entities()
        .filter(|e| super::data_value(e, "corridor").is_some() && super::data_value(e, "as").as_deref() == Some(axis_name))
        .filter_map(|e| super::data_value(e, "sjabloon"))
        .last()
}

/// One point of a section: offset (m, right > 0), height (m), the point code
/// and the NLCS object of the line along it.
#[derive(Clone, Debug, PartialEq)]
pub struct Pt {
    pub o: f64,
    pub z: f64,
    /// RS lane, GO gutter, BV/BA kerb front and back, TR footway, FP cycle
    /// path, PS parking, MB central reserve, SB verge, BE shoulder, KR crest,
    /// SL/SI ditch bottom and its outer edge, AS axis, TL daylight.
    pub code: &'static str,
    pub nlcs: NlcsId,
}

/// A piece of material in a section: a pavement layer or a kerb.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub material: String,
    pub nlcs: NlcsId,
    /// Outline (offset, height).
    pub poly: Vec<(f64, f64)>,
    /// Area (m²).
    pub area: f64,
    pub kerb: bool,
}

/// One computed cross section.
#[derive(Clone, Debug)]
pub struct Section {
    /// Distance along the axis (m).
    pub s: f64,
    pub z_axis: f64,
    /// The design (top) from the left daylight to the right one.
    pub pts: Vec<Pt>,
    /// Whether each side's daylight is in fill (true) or cut; None without ground.
    pub fill_side: [Option<bool>; 2],
    /// Cut and fill areas to the top (m²).
    pub cut: f64,
    pub fill: f64,
    /// Cut and fill areas to the subgrade (m²).
    pub cut_datum: f64,
    pub fill_datum: f64,
    /// The subgrade: the underside of the build-up, else the top.
    pub datum: Vec<(f64, f64)>,
    /// The formation (underside of the surfacing) per part: side, part
    /// index, inner and outer point.
    pub formation: Vec<(usize, usize, [(f64, f64); 2])>,
    pub bodies: Vec<Body>,
    /// Top width per surface (m).
    pub surfaces: Vec<(NlcsId, f64)>,
}

/// Named target lines crossing one section: offset (m, right > 0) and the
/// line's height there (m), if it has one.
#[derive(Clone, Debug, Default)]
pub struct Targets(HashMap<String, Vec<(f64, Option<f64>)>>);

impl Targets {
    pub fn add(&mut self, name: &str, offset: f64, z: Option<f64>) {
        self.0.entry(name.to_lowercase()).or_default().push((offset, z));
    }

    /// The nearest crossing on side `dir` at or beyond `from` (distance, m).
    fn offset(&self, name: &str, dir: f64, from: f64) -> Option<f64> {
        self.0.get(&name.to_lowercase())?.iter().map(|(t, _)| t * dir).filter(|t| *t >= from - 1e-6).min_by(f64::total_cmp)
    }

    /// The height of the crossing on side `dir` nearest to distance `at`.
    fn level(&self, name: &str, dir: f64, at: f64) -> Option<f64> {
        self.0.get(&name.to_lowercase())?.iter().filter(|(t, z)| t * dir > 0.0 && z.is_some()).min_by(|a, b| (a.0 * dir - at).abs().total_cmp(&(b.0 * dir - at).abs())).and_then(|x| x.1)
    }
}

/// Cross slopes of the lanes: [left, right], each side's crown on
/// straights, tilted towards the inside in curves.
fn lane_slopes(asm: &Assembly, curve: Option<f64>, crown: [f64; 2]) -> [f64; 2] {
    match curve {
        // `t` > 0: a left curve, the inside is left; |t| is the runoff share.
        Some(t) if asm.superelevation > 0.0 => {
            let share = t.abs().min(1.0);
            let inside = if t > 0.0 { 0 } else { 1 };
            let mut out = crown;
            let ei = asm.superelevation.max(-crown[inside]);
            out[inside] = crown[inside] + (-ei - crown[inside]) * share;
            let o = 1 - inside;
            let eo = asm.superelevation.max(-crown[o]);
            out[o] = crown[o] + (eo - crown[o]) * share;
            out
        }
        _ => crown,
    }
}

/// Signed share of full superelevation at `s` (m): +1 inside a left curve,
/// −1 in a right curve, going linearly to 0 over the runoff before and after.
fn curve_share(al: &Alignment, s_m: f64, f: f64, runoff: f64) -> Option<f64> {
    let mut run = 0.0;
    let mut best: Option<f64> = None;
    for seg in &al.segs {
        let l = seg.length() / f;
        if let Seg::Arc { sweep, .. } = seg {
            let sign = sweep.signum();
            let (a, b) = (run, run + l);
            let share = if s_m >= a && s_m <= b {
                1.0
            } else if s_m < a && a - s_m < runoff {
                1.0 - (a - s_m) / runoff
            } else if s_m > b && s_m - b < runoff {
                1.0 - (s_m - b) / runoff
            } else {
                0.0
            };
            if share > 0.0 && best.is_none_or(|v| share > v.abs()) {
                best = Some(sign * share);
            }
        }
        run += l;
    }
    best
}

/// Linear interpolation in a polyline of (offset, height), sorted by offset.
fn interp(pts: &[(f64, f64)], o: f64) -> Option<f64> {
    if pts.is_empty() || o < pts[0].0 - 1e-9 || o > pts[pts.len() - 1].0 + 1e-9 {
        return None;
    }
    for w in pts.windows(2) {
        if o <= w[1].0 + 1e-12 {
            let t = if (w[1].0 - w[0].0).abs() < 1e-12 { 0.0 } else { (o - w[0].0) / (w[1].0 - w[0].0) };
            return Some(w[0].1 + (w[1].1 - w[0].1) * t);
        }
    }
    pts.last().map(|p| p.1)
}

/// Area of a polygon (shoelace).
fn polygon_area(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum::<f64>().abs() / 2.0
}

/// Whether the condition holds for design height `z` over `ground`.
fn holds(c: &Condition, ground: Option<f64>, z: f64) -> bool {
    let Some(g) = ground else { return false };
    let d = if c.cut { g - z } else { z - g };
    d >= c.min - 1e-9 && d <= c.max + 1e-9
}

/// From distance `o` at height `z`, outward (`dir`) with `grade` (rise per
/// metre): where the line meets the ground within `reach`. Returns the
/// distance run, and whether the ground was met (else the last point with
/// ground).
fn meet(o: f64, z: f64, dir: f64, grade: f64, reach: f64, ground: &mut dyn FnMut(f64) -> Option<f64>) -> (f64, bool) {
    let Some(g0) = ground(dir * o) else { return (0.0, false) };
    let step = 0.25;
    let mut prev = (0.0, z - g0);
    if prev.1 == 0.0 {
        return (0.0, true);
    }
    let mut t = step;
    while t <= reach + 1e-9 {
        let Some(gt) = ground(dir * (o + t)) else { break };
        let d = z + grade * t - gt;
        if d == 0.0 || d.signum() != prev.1.signum() {
            return (prev.0 + (t - prev.0) * prev.1 / (prev.1 - d), true);
        }
        prev = (t, d);
        t += step;
    }
    (prev.0, false)
}

/// What one side gives, outward from the axis; offsets are distances from
/// the axis (m), made signed by the caller.
#[derive(Default)]
struct SideOut {
    pts: Vec<Pt>,
    fill: Option<bool>,
    datum: Vec<(f64, f64)>,
    formation: Vec<(usize, [(f64, f64); 2])>,
    bodies: Vec<Body>,
    surfaces: Vec<(NlcsId, f64)>,
}

/// One side (0 left, 1 right): the parts, then the slope to the ground.
fn side(asm: &Assembly, si: usize, z0: f64, lane_slope: f64, ground: &mut dyn FnMut(f64) -> Option<f64>, targets: &Targets) -> SideOut {
    let dir = if si == 0 { -1.0 } else { 1.0 };
    let parts = &asm.sides[si];
    let mut out = SideOut::default();
    let (mut o, mut z) = (0.0f64, z0);
    let mut done = false;
    let (bottom_id, edge_id) = (nlcs_id("GW", "WATERGANG_SLOOT_BODEM"), nlcs_id("GW", "WATERGANG_SLOOT_INSTEEK"));
    for (pi, p) in parts.iter().enumerate() {
        if let Some(c) = &p.condition {
            if !holds(c, ground(dir * o), z) {
                continue;
            }
        }
        let (o0, zs) = (o, z);
        let depth = p.depth();
        match p.kind {
            PartKind::Kerb => {
                let batter = p.batter.clamp(0.0, p.width);
                let top = z + p.upstand;
                o += batter;
                z = top;
                out.pts.push(Pt { o, z, code: "BV", nlcs: p.surface });
                let w = (p.width - batter).max(0.0);
                o += w;
                z += p.slope * w;
                out.pts.push(Pt { o, z, code: "BA", nlcs: p.surface });
                let (b0, b1) = (top - p.height, z - p.height);
                let poly = vec![(o0, b0), (o0, zs.max(b0)), (o0 + batter, top), (o, z), (o, b1)];
                out.bodies.push(Body { material: p.name.clone(), nlcs: p.surface, area: polygon_area(&poly), poly, kerb: true });
                let mut d = 0.0;
                for l in &p.layers {
                    let poly = vec![(o0, b0 - d), (o, b1 - d), (o, b1 - d - l.thickness), (o0, b0 - d - l.thickness)];
                    out.bodies.push(Body { material: l.material.clone(), nlcs: l.nlcs, area: (o - o0) * l.thickness, poly, kerb: false });
                    d += l.thickness;
                }
                out.datum.push((o0, b0 - depth));
                out.datum.push((o, b1 - depth));
            }
            PartKind::Ditch => {
                let d = p.ditch.unwrap_or_default();
                let dep = match d.level {
                    Some(l) => (z - l).max(0.05),
                    None => d.depth.max(0.05),
                };
                out.datum.push((o0, zs));
                o += dep * d.inner;
                z -= dep;
                out.pts.push(Pt { o, z, code: "SL", nlcs: bottom_id });
                out.datum.push((o, z));
                o += d.bottom;
                out.pts.push(Pt { o, z, code: "SL", nlcs: bottom_id });
                out.datum.push((o, z));
                // The last part that applies: its back slope is the daylight.
                let g0 = ground(dir * o0);
                let later = parts[pi + 1..].iter().any(|q| q.condition.as_ref().is_none_or(|c| holds(c, g0, zs)));
                let back = 1.0 / d.outer.max(0.1);
                if !later && ground(dir * o).is_some_and(|g| g > z) {
                    let (t, _) = meet(o, z, dir, back, asm.reach, ground);
                    o += t;
                    z += back * t;
                    out.pts.push(Pt { o, z, code: "TL", nlcs: 0 });
                    out.datum.push((o, z));
                    out.fill = Some(false);
                    done = true;
                } else {
                    o += (zs - z) * d.outer;
                    z = zs;
                    out.pts.push(Pt { o, z, code: "SI", nlcs: edge_id });
                    out.datum.push((o, z));
                }
            }
            _ => {
                let mut w = p.width;
                match &p.width_target {
                    Some(Target::Offset(v)) => w = (v.abs() - o).max(0.0),
                    Some(Target::Line(n)) => {
                        if let Some(t) = targets.offset(n, dir, o) {
                            w = (t - o).max(0.0);
                        }
                    }
                    _ => {}
                }
                let mut slope = if p.kind == PartKind::Lane { lane_slope } else { p.slope };
                if w > 1e-6 {
                    let zt = match &p.level_target {
                        Some(Target::Level(v)) => Some(*v),
                        Some(Target::Ground) => ground(dir * (o + w)),
                        Some(Target::Line(n)) => targets.level(n, dir, o + w),
                        _ => None,
                    };
                    if let Some(zt) = zt {
                        slope = ((zt - z) / w).clamp(p.slope_min, p.slope_max);
                    }
                }
                o += w;
                z += slope * w;
                out.pts.push(Pt { o, z, code: p.kind.code(), nlcs: p.surface });
                let mut d = 0.0;
                for l in &p.layers {
                    let poly = vec![(o0, zs - d), (o, z - d), (o, z - d - l.thickness), (o0, zs - d - l.thickness)];
                    out.bodies.push(Body { material: l.material.clone(), nlcs: l.nlcs, area: w * l.thickness, poly, kerb: false });
                    d += l.thickness;
                }
                let bound: f64 = p.layers.iter().take_while(|l| l.bound).map(|l| l.thickness).sum();
                if bound > 0.0 {
                    out.formation.push((pi, [(o0, zs - bound), (o, z - bound)]));
                }
                out.surfaces.push((p.surface, w));
                out.datum.push((o0, zs - depth));
                out.datum.push((o, z - depth));
            }
        }
        if done {
            return out;
        }
    }
    // Daylight: the fill or cut slope to the ground.
    out.datum.push((o, z));
    let Some(g0) = ground(dir * o) else {
        out.pts.push(Pt { o, z, code: "TL", nlcs: 0 });
        return out;
    };
    let fill = z >= g0;
    let grade = if fill { -1.0 / asm.fill.max(0.1) } else { 1.0 / asm.cut.max(0.1) };
    let (t, _) = meet(o, z, dir, grade, asm.reach, ground);
    o += t;
    z += grade * t;
    out.pts.push(Pt { o, z, code: "TL", nlcs: 0 });
    out.datum.push((o, z));
    out.fill = Some(fill);
    out
}

/// Cut and fill areas (m²) between a design line and the ground.
fn areas(design: &[(f64, f64)], ground: &mut dyn FnMut(f64) -> Option<f64>) -> (f64, f64) {
    let (a, b) = (design[0].0, design[design.len() - 1].0);
    let step = 0.1;
    let (mut cut, mut fill) = (0.0, 0.0);
    let mut o = a + step / 2.0;
    while o < b {
        if let (Some(d), Some(g)) = (interp(design, o), ground(o)) {
            let dz = d - g;
            if dz > 0.0 {
                fill += dz * step;
            } else {
                cut -= dz * step;
            }
        }
        o += step;
    }
    (cut, fill)
}

/// A cross section at `s` (m) for a design height `z0` at the axis.
pub fn section(asm: &Assembly, s: f64, z0: f64, curve: Option<f64>, ground: &mut dyn FnMut(f64) -> Option<f64>) -> Section {
    section_with(asm, s, z0, curve, ground, &Targets::default())
}

/// A cross section with target lines.
pub fn section_with(asm: &Assembly, s: f64, z0: f64, curve: Option<f64>, ground: &mut dyn FnMut(f64) -> Option<f64>, targets: &Targets) -> Section {
    let crown = |si: usize| asm.sides[si].iter().find(|p| p.kind == PartKind::Lane).map(|p| p.slope).unwrap_or(-0.025);
    let slopes = lane_slopes(asm, curve, [crown(0), crown(1)]);
    let l = side(asm, 0, z0, slopes[0], ground, targets);
    let r = side(asm, 1, z0, slopes[1], ground, targets);
    let mut pts: Vec<Pt> = l.pts.iter().rev().map(|p| Pt { o: -p.o, ..p.clone() }).collect();
    pts.push(Pt { o: 0.0, z: z0, code: "AS", nlcs: nlcs_id("AM", "AS") });
    pts.extend(r.pts.iter().cloned());
    let mut datum: Vec<(f64, f64)> = l.datum.iter().rev().map(|(o, z)| (-o, *z)).collect();
    datum.extend(r.datum.iter().cloned());
    let line: Vec<(f64, f64)> = pts.iter().map(|p| (p.o, p.z)).collect();
    let (cut, fill) = areas(&line, ground);
    let (cut_datum, fill_datum) = if asm.has_layers() { areas(&datum, ground) } else { (cut, fill) };
    let mirror = |b: &Body| Body { poly: b.poly.iter().map(|(o, z)| (-o, *z)).collect(), ..b.clone() };
    let mut bodies: Vec<Body> = l.bodies.iter().map(mirror).collect();
    bodies.extend(r.bodies.iter().cloned());
    let mut formation: Vec<(usize, usize, [(f64, f64); 2])> = l.formation.iter().map(|(pi, [a, b])| (0, *pi, [(-a.0, a.1), (-b.0, b.1)])).collect();
    formation.extend(r.formation.iter().map(|(pi, ab)| (1, *pi, *ab)));
    let mut surfaces = l.surfaces;
    surfaces.extend(r.surfaces);
    Section { s, z_axis: z0, pts, fill_side: [l.fill, r.fill], cut, fill, cut_datum, fill_datum, datum, formation, bodies, surfaces }
}

/// Average end areas: (cut, fill) volume between consecutive sections and
/// the running balance (fill − cut), m³.
pub fn volumes(sections: &[Section]) -> Vec<(f64, f64, f64)> {
    volumes_of(sections, |s| (s.cut, s.fill))
}

fn volumes_of(sections: &[Section], area: impl Fn(&Section) -> (f64, f64)) -> Vec<(f64, f64, f64)> {
    let mut out = vec![(0.0, 0.0, 0.0)];
    let mut balance = 0.0;
    for w in sections.windows(2) {
        let ds = w[1].s - w[0].s;
        let (a0, a1) = (area(&w[0]), area(&w[1]));
        let c = (a0.0 + a1.0) / 2.0 * ds;
        let f = (a0.1 + a1.1) / 2.0 * ds;
        balance += f - c;
        out.push((c, f, balance));
    }
    out
}

/// Material quantities of a corridor, by average end areas.
#[derive(Debug, Default)]
pub struct Quantities {
    /// Layer material → (NLCS object, volume m³).
    pub layers: BTreeMap<String, (NlcsId, f64)>,
    /// Surface → area (m²).
    pub surfaces: BTreeMap<NlcsId, f64>,
    /// Kerb → (NLCS object, length m, volume m³).
    pub kerbs: BTreeMap<String, (NlcsId, f64, f64)>,
}

pub fn quantities(sections: &[Section]) -> Quantities {
    let mut q = Quantities::default();
    type PerMaterial = BTreeMap<String, (NlcsId, f64, f64)>;
    // Per section: material → (id, area, count).
    let sum = |s: &Section, kerb: bool| -> PerMaterial {
        let mut m: PerMaterial = BTreeMap::new();
        for b in s.bodies.iter().filter(|b| b.kerb == kerb) {
            let e = m.entry(b.material.clone()).or_insert((b.nlcs, 0.0, 0.0));
            e.1 += b.area;
            e.2 += 1.0;
        }
        m
    };
    let widths = |s: &Section| -> BTreeMap<NlcsId, f64> {
        let mut m = BTreeMap::new();
        for (id, w) in &s.surfaces {
            *m.entry(*id).or_insert(0.0) += w;
        }
        m
    };
    for w in sections.windows(2) {
        let ds = w[1].s - w[0].s;
        for kerb in [false, true] {
            let (a, b) = (sum(&w[0], kerb), sum(&w[1], kerb));
            let keys: HashSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                let (ia, aa, ca) = a.get(k).copied().unwrap_or((0, 0.0, 0.0));
                let (ib, ab, cb) = b.get(k).copied().unwrap_or((0, 0.0, 0.0));
                let id = if ia != 0 { ia } else { ib };
                let vol = (aa + ab) / 2.0 * ds;
                if kerb {
                    let e = q.kerbs.entry(k.clone()).or_insert((id, 0.0, 0.0));
                    e.1 += (ca + cb) / 2.0 * ds;
                    e.2 += vol;
                } else {
                    q.layers.entry(k.clone()).or_insert((id, 0.0)).1 += vol;
                }
            }
        }
        let (a, b) = (widths(&w[0]), widths(&w[1]));
        let keys: HashSet<&NlcsId> = a.keys().chain(b.keys()).collect();
        for k in keys {
            *q.surfaces.entry(*k).or_insert(0.0) += (a.get(k).unwrap_or(&0.0) + b.get(k).unwrap_or(&0.0)) / 2.0 * ds;
        }
    }
    q
}

/// Triangles between two cross-section lines, each ordered by offset. They
/// may differ in points (where a condition swaps a part, or a width follows
/// a target): both are walked by offset.
pub fn stitch(a: &[(f64, P3)], b: &[(f64, P3)], mesh: &mut MeshBuilder) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    let (mut i, mut j) = (0, 0);
    while i + 1 < a.len() || j + 1 < b.len() {
        let advance_a = if i + 1 >= a.len() {
            false
        } else if j + 1 >= b.len() {
            true
        } else {
            a[i + 1].0 <= b[j + 1].0
        };
        if advance_a {
            mesh.tri(a[i].1, a[i + 1].1, b[j].1);
            i += 1;
        } else {
            mesh.tri(a[i].1, b[j + 1].1, b[j].1);
            j += 1;
        }
    }
}

/// Keys of a section's points for feature lines: side (0 left, 1 right, 2
/// the axis), code and its occurrence counted from the axis outward.
fn point_keys(sct: &Section) -> Vec<(u8, &'static str, u16)> {
    let axis = sct.pts.iter().position(|p| p.code == "AS").unwrap_or(0);
    let mut keys = vec![(2u8, "AS", 0u16); sct.pts.len()];
    let mut count: HashMap<(u8, &'static str), u16> = HashMap::new();
    for i in (0..axis).rev() {
        let c = count.entry((0, sct.pts[i].code)).or_insert(0);
        keys[i] = (0, sct.pts[i].code, *c);
        *c += 1;
    }
    for i in axis + 1..sct.pts.len() {
        let c = count.entry((1, sct.pts[i].code)).or_insert(0);
        keys[i] = (1, sct.pts[i].code, *c);
        *c += 1;
    }
    keys
}

/// A line in the drawing as points in drawing units, with its heights (m)
/// where it has them (3D polylines, lines).
fn target_line(doc: &CadDocument, h: Handle, f: f64) -> Result<Vec<(XY, Option<f64>)>, String> {
    let e = doc.get_entity(h).ok_or_else(|| trf!("doel: geen element met handle {}", "target: no entity with handle {}", super::api::handle_text(h)))?;
    match e {
        EntityType::Polyline3D(p) => Ok(p.vertices.iter().map(|v| ([v.position.x, v.position.y], Some(v.position.z / f))).collect()),
        EntityType::Line(l) => Ok(vec![([l.start.x, l.start.y], Some(l.start.z / f)), ([l.end.x, l.end.y], Some(l.end.z / f))]),
        _ => {
            let al = Alignment::from_entity(e).ok_or_else(|| trf!("doel {}: geen lijn of polylijn", "target {}: not a line or polyline", super::api::handle_text(h)))?;
            let z = match e {
                EntityType::LwPolyline(l) if l.elevation != 0.0 => Some(l.elevation / f),
                _ => None,
            };
            let len = al.length();
            let n = ((len / f).ceil() as usize).clamp(1, 20_000);
            Ok((0..=n).map(|i| (al.at(len * i as f64 / n as f64).0, z)).collect())
        }
    }
}

/// Where the section line p + n·t (t in metres) crosses a polyline.
fn crossings(p: XY, n: XY, f: f64, line: &[(XY, Option<f64>)]) -> Vec<(f64, Option<f64>)> {
    let mut out = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0].0, w[1].0);
        let d = [b[0] - a[0], b[1] - a[1]];
        let den = n[0] * f * d[1] - n[1] * f * d[0];
        if den.abs() < 1e-12 {
            continue;
        }
        let q = [a[0] - p[0], a[1] - p[1]];
        let t = (q[0] * d[1] - q[1] * d[0]) / den;
        let u = (q[0] * n[1] * f - q[1] * n[0] * f) / den;
        if (-1e-9..=1.0 + 1e-9).contains(&u) {
            let z = match (w[0].1, w[1].1) {
                (Some(z0), Some(z1)) => Some(z0 + (z1 - z0) * u),
                (z0, z1) => z0.or(z1),
            };
            out.push((t, z));
        }
    }
    out
}

pub struct Request {
    pub axis: Option<Handle>,
    pub assembly: Assembly,
    pub grade: Option<Grade>,
    /// Section distance (m).
    pub every: f64,
    /// Section views: upper-left corner (drawing), or None for none.
    pub views: Option<XY>,
    /// The stations to build between (with the axis's start station); None:
    /// the whole axis. A junction or roundabout fills the rest.
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// Target lines in the drawing by name (DOELEN=naam:handle;…).
    pub targets: Vec<(String, Handle)>,
}

/// `CIV_CORRIDOR`.
pub fn corridor(opt: &super::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let mut opt = opt.clone();
    // A design: the new status, on both the surface and the lines.
    opt.nlcs.status = Status::N;
    let ctx = Ctx::new(&opt)?;
    let extra: Vec<Handle> = req.axis.into_iter().chain(req.targets.iter().map(|t| t.1)).collect();
    let doc = super::document(&["tin", "ontwerpprofiel"], &extra)?;
    let (al, _, _) = as_profiel::axis(&doc, req.axis)?;
    let (f, k) = (ctx.f, ctx.k);
    let len_m = al.length() / f;
    let asm = req.assembly;
    let lines: Vec<(String, Vec<(XY, Option<f64>)>)> = req.targets.iter().map(|(n, h)| Ok((n.clone(), target_line(&doc, *h, f)?))).collect::<Result<_, String>>()?;
    let grade = match req.grade {
        Some(g) => g,
        None => Grade::Profile(profile_of(&doc, &al.name).ok_or_else(|| {
            trf!(
                "{} heeft geen ontwerpprofiel: maak er een (CIV_ONTWERPPROFIEL) of geef NIVEAU=… of BOVEN=…",
                "{} has no design profile: make one (CIV_ONTWERPPROFIEL) or give NIVEAU=… or BOVEN=…",
                if al.name.is_empty() { crate::lang::tr("de as", "the alignment") } else { &al.name }
            )
        })?),
    };
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let mut stations: Vec<f64> = al.stations(req.every.max(0.5) * f, true).into_iter().map(|s| s / f).collect();
    // A station range: the stations inside it and its ends.
    let (lo, hi) = (req.from.map(|v| (v - al.start).max(0.0)).unwrap_or(0.0), req.to.map(|v| (v - al.start).min(len_m)).unwrap_or(len_m));
    if hi - lo < 0.5 {
        return Err(trf!("VAN/TOT: geen bereik op de as ({} tot {})", "VAN/TOT: no range on the alignment ({} to {})", station(al.start), station(al.start + len_m)));
    }
    if req.from.is_some() || req.to.is_some() {
        stations.retain(|s| *s > lo + 0.05 && *s < hi - 0.05);
        stations.insert(0, lo);
        stations.push(hi);
    }
    if stations.len() > 3000 {
        return Err(trf!("{} dwarsprofielen is te veel: kies een grotere afstand", "{} cross sections is too many: choose a larger distance", stations.len()));
    }
    // Ground at the axis, for "above ground" and the sections' labels.
    let axis_ground: Vec<Option<f64>> = stations.iter().map(|s| ground.height(al.at(s * f).0, f)).collect();
    let design_at = |s: f64| -> Option<f64> {
        match &grade {
            Grade::Profile(p) => Some(as_profiel::design_z(p, s)),
            Grade::Level(z) => Some(*z),
            Grade::AboveGround(dz) => {
                // Mean ground within 50 m either side, so the road does not follow every bump.
                let near: Vec<f64> = stations.iter().zip(&axis_ground).filter(|(t, _)| (*t - s).abs() <= 50.0).filter_map(|(_, g)| *g).collect();
                (!near.is_empty()).then(|| near.iter().sum::<f64>() / near.len() as f64 + dz)
            }
        }
    };
    progress(trf!("Corridor: {} dwarsprofielen ({}) …", "Corridor: {} cross sections ({}) …", stations.len(), ground.label()));
    let reach = asm.half_width() + asm.reach + 10.0;
    let mut sections = Vec::with_capacity(stations.len());
    let mut skipped = 0;
    for (i, s) in stations.iter().enumerate() {
        if i % 25 == 0 {
            progress(trf!("Corridor: dwarsprofiel {}/{}…", "Corridor: cross section {}/{}…", i + 1, stations.len()));
        }
        let Some(z0) = design_at(*s) else {
            skipped += 1;
            continue;
        };
        let (p, dir) = al.at(s * f);
        let rn = [dir.sin(), -dir.cos()];
        let mut targets = Targets::default();
        for (name, line) in &lines {
            for (t, z) in crossings(p, rn, f, line) {
                if t.abs() <= reach {
                    targets.add(name, t, z);
                }
            }
        }
        let mut g = |o: f64| ground.height([p[0] + rn[0] * o * f, p[1] + rn[1] * o * f], f);
        let curve = curve_share(&al, *s, f, asm.runoff);
        sections.push(section_with(&asm, *s, z0, curve, &mut g, &targets));
    }
    if sections.len() < 2 {
        return Err(ground.error().unwrap_or_else(|| crate::lang::tr("te weinig dwarsprofielen met een ontwerphoogte", "too few cross sections with a design height").into()));
    }
    let vols = volumes(&sections);
    let datum_vols = volumes_of(&sections, |s| (s.cut_datum, s.fill_datum));
    let (cut, fill): (f64, f64) = vols.iter().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
    let (cut_d, fill_d): (f64, f64) = datum_vols.iter().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
    let qty = quantities(&sections);
    progress(crate::lang::tr("Corridor tekenen…", "Drawing the corridor…").into());

    // Plan position of an offset (m) in a section.
    let plan = |sct: &Section, o: f64| -> XY {
        let (p, dir) = al.at(sct.s * f);
        [p[0] + dir.sin() * o * f, p[1] - dir.cos() * o * f]
    };
    let p3 = |sct: &Section, o: f64, z: f64| -> P3 {
        let xy = plan(sct, o);
        [xy[0], xy[1], z * f]
    };
    let mut out = Out::default();
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let data = record(&[("corridor", format!("Corridor {name}")), ("as", al.name.clone()), ("sjabloon", asm.name.clone())]);
    // Design surface: strips between consecutive sections.
    let faces_spec = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let mut mesh = MeshBuilder::default();
    let mut datum_mesh = MeshBuilder::default();
    let mut formation_mesh = MeshBuilder::default();
    for w in sections.windows(2) {
        let line = |s: &Section| -> Vec<(f64, P3)> { s.pts.iter().map(|q| (q.o, p3(s, q.o, q.z))).collect() };
        stitch(&line(&w[0]), &line(&w[1]), &mut mesh);
        if asm.has_layers() {
            let line = |s: &Section| -> Vec<(f64, P3)> { s.datum.iter().map(|(o, z)| (*o, p3(s, *o, *z))).collect() };
            stitch(&line(&w[0]), &line(&w[1]), &mut datum_mesh);
            for (si, pi, [a, b]) in &w[0].formation {
                if let Some((_, _, [c, d])) = w[1].formation.iter().find(|x| x.0 == *si && x.1 == *pi) {
                    formation_mesh.quad(p3(&w[0], a.0, a.1), p3(&w[0], b.0, b.1), p3(&w[1], d.0, d.1), p3(&w[1], c.0, c.1));
                }
            }
        }
    }
    let face_count = mesh.faces();
    let mesh_ents = mesh.finish(&faces_spec.name, None, Some(&data));
    let mesh_count = mesh_ents.len();
    out.put(&faces_spec, mesh_ents);
    let mut other_meshes = 0;
    let (datum_faces, formation_faces) = (datum_mesh.faces(), formation_mesh.faces());
    if asm.has_layers() {
        let dspec = ctx.layer("GW", "CUNET_BODEM", Element::G)?;
        let ddata = record(&[("corridor", format!("Corridor {name}")), ("as", al.name.clone()), ("oppervlak", crate::lang::tr("onderkant verhardingsconstructie (subgrade)", "underside of the pavement (subgrade)").into())]);
        let e = datum_mesh.finish(&dspec.name, None, Some(&ddata));
        other_meshes += e.len();
        out.put(&dspec, e);
        if formation_faces > 0 {
            let fspec = ctx.layer("VH", "FUNDERING", Element::G)?;
            let fdata = record(&[("corridor", format!("Corridor {name}")), ("as", al.name.clone()), ("oppervlak", crate::lang::tr("bovenkant fundering (formatie)", "top of the foundation (formation)").into())]);
            let e = formation_mesh.finish(&fspec.name, None, Some(&fdata));
            other_meshes += e.len();
            out.put(&fspec, e);
        }
    }
    // Feature lines: one 3D line per point key, broken where a section lacks
    // it; the daylight split in runs of fill (toe) and cut (crest).
    let poly3 = |pts: Vec<P3>, layer: &str| {
        let mut e = EntityType::Polyline3D(Polyline3D::from_points(pts.iter().map(|p| v3([p[0], p[1]], p[2])).collect()));
        let c = e.common_mut();
        c.layer = layer.to_string();
        c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        c.linetype_scale = k;
        e
    };
    let toe = ctx.layer("GW", "TALUD_TEEN", Element::G)?;
    let top = ctx.layer("GW", "TALUD_KRUIN", Element::G)?;
    let axis3 = ctx.layer("AM", "AS", Element::G)?;
    // key → runs of (section index, point, NLCS object, fill)
    type Key = (u8, &'static str, u16);
    let mut tracks: BTreeMap<Key, Vec<(usize, P3, NlcsId, Option<bool>)>> = BTreeMap::new();
    let mut order: Vec<Key> = Vec::new();
    for (si, sct) in sections.iter().enumerate() {
        for (key, q) in point_keys(sct).into_iter().zip(&sct.pts) {
            let fill = if key.0 < 2 { sct.fill_side[key.0 as usize] } else { None };
            if !tracks.contains_key(&key) {
                order.push(key);
            }
            tracks.entry(key).or_default().push((si, p3(sct, q.o, q.z), q.nlcs, fill));
        }
    }
    let mut lines_n = 0;
    let mut specs: HashMap<NlcsId, LayerSpec> = HashMap::new();
    for key in order {
        let track = &tracks[&key];
        // Runs of consecutive sections (and, for the daylight, of one kind).
        let mut runs: Vec<Vec<(P3, NlcsId, Option<bool>)>> = Vec::new();
        let mut last: Option<(usize, Option<bool>)> = None;
        for (si, p, id, fill) in track {
            let split = match last {
                None => true,
                Some((ls, lf)) => *si != ls + 1 || (key.1 == "TL" && *fill != lf),
            };
            if split {
                // The daylight changes kind between sections: the runs share the point.
                if key.1 == "TL" && last.is_some_and(|(ls, _)| *si == ls + 1) {
                    if let Some(r) = runs.last_mut() {
                        r.push((*p, *id, *fill));
                    }
                }
                runs.push(Vec::new());
            }
            runs.last_mut().unwrap().push((*p, *id, *fill));
            last = Some((*si, *fill));
        }
        for run in runs.into_iter().filter(|r| r.len() > 1) {
            let pts: Vec<P3> = run.iter().map(|r| r.0).collect();
            let spec = match key.1 {
                "TL" => match run.last().and_then(|r| r.2) {
                    Some(true) => toe.clone(),
                    Some(false) => top.clone(),
                    None => continue,
                },
                "AS" => axis3.clone(),
                _ => {
                    let id = run[0].1;
                    match specs.get(&id) {
                        Some(s) => s.clone(),
                        None => {
                            let s = nlcs_layer(&ctx, id, Element::G)?;
                            specs.insert(id, s.clone());
                            s
                        }
                    }
                }
            };
            let mut e = poly3(pts, &spec.name);
            if key.1 == "AS" {
                e.common_mut().extended_data.add_record(data.clone());
            }
            out.put(&spec, [e]);
            lines_n += 1;
        }
    }
    let handles = out.write_handles(&format!("Corridor {name}"))?;
    // Section views with the design and its layers, when asked: one more step.
    let mut view_rows = 0;
    let mut view_count = 0;
    if let Some(at) = req.views {
        let mut vout = Out::default();
        let left = sections.iter().map(|s| -s.pts[0].o).fold(0.0, f64::max).ceil() + 2.0;
        let right = sections.iter().map(|s| s.pts[s.pts.len() - 1].o).fold(0.0, f64::max).ceil() + 2.0;
        let mut layer_lines = Vec::new();
        let views: Vec<SectionView> = sections
            .iter()
            .map(|sct| {
                let (p, dir) = al.at(sct.s * f);
                let rn = [dir.sin(), -dir.cos()];
                let plan_at = |o: f64| [p[0] + rn[0] * o * f, p[1] + rn[1] * o * f];
                let step = ((left + right) / 200.0).max(0.25);
                let mut o = -left;
                let mut g = Vec::new();
                while o <= right + 1e-9 {
                    g.push((o, ground.height(plan_at(o), f)));
                    o += step;
                }
                let mut outlines = Vec::new();
                for b in &sct.bodies {
                    let spec = specs.get(&b.nlcs).cloned().or_else(|| nlcs_layer(&ctx, b.nlcs, Element::G).ok());
                    if let Some(spec) = spec {
                        let mut poly = b.poly.clone();
                        poly.push(b.poly[0]);
                        outlines.push((spec, poly));
                    }
                }
                layer_lines.push(outlines);
                SectionView {
                    s: sct.s * f,
                    ground: g,
                    design: sct.pts.iter().map(|q| (q.o, q.z)).collect(),
                    axis_z: Some(sct.z_axis),
                    cut: Some(sct.cut),
                    fill: Some(sct.fill),
                    plan: (plan_at(-left), plan_at(right)),
                }
            })
            .collect();
        let mut vopt = ctx.opt.clone();
        vopt.section_every = req.every;
        let vctx = Ctx { opt: vopt, ..Ctx::new(&opt)? };
        let place = (!at[0].is_nan()).then_some(at);
        as_profiel::SECTION_OUTLINES.with(|o| *o.borrow_mut() = layer_lines);
        view_rows = as_profiel::draw_section_views(&vctx, &al, &views, left, right, place, &trf!("ontwerp {} over {}", "design {} over {}", asm.name, ground.label()), " — corridor", &mut vout)?.0;
        as_profiel::SECTION_OUTLINES.with(|o| o.borrow_mut().clear());
        view_count = vout.write(&trf!("Dwarsprofielen corridor {}", "Cross sections corridor {}", name))?;
    }
    ground.note_source();
    // The design surface and the corridor's lines, for later commands.
    let first_line = (mesh_count + other_meshes).min(handles.len());
    crate::bridge::index_add("tin", "N", &handles[..mesh_count.min(handles.len())], false);
    crate::bridge::index_add("corridor", &format!("Corridor {name}"), &handles[first_line..(first_line + lines_n).min(handles.len())], true);
    // The earthwork list beside the drawing.
    let mut csv = String::from(crate::lang::tr(
        "Station;Afstand (m);Ontwerphoogte (m);Afgraven (m2);Ophogen (m2);Afgraven (m3);Ophogen (m3);Balans (m3);Afgraven tot onderkant (m3);Ophogen tot onderkant (m3)\n",
        "Station;Distance (m);Design height (m);Cut (m2);Fill (m2);Cut (m3);Fill (m3);Balance (m3);Cut to subgrade (m3);Fill to subgrade (m3)\n",
    ));
    let dec = num;
    for ((sct, v), vd) in sections.iter().zip(&vols).zip(&datum_vols) {
        csv.push_str(&format!(
            "{};{};{};{};{};{};{};{};{};{}\n",
            station(al.start + sct.s),
            dec(sct.s, 2),
            dec(sct.z_axis, 3),
            dec(sct.cut, 2),
            dec(sct.fill, 2),
            dec(v.0, 1),
            dec(v.1, 1),
            dec(v.2, 1),
            dec(vd.0, 1),
            dec(vd.1, 1)
        ));
    }
    let file = name.replace(' ', "_").to_lowercase();
    let csv_path = crate::bridge::beside_drawing(&format!("corridor_{file}.csv"));
    let _ = std::fs::write(&csv_path, csv);
    // The material list.
    let layer_name = |id: NlcsId| nlcs_layer(&ctx, id, Element::G).map(|s| s.name).unwrap_or_default();
    let mut mcsv = String::from(crate::lang::tr("Soort;Materiaal;NLCS-laag;Hoeveelheid;Eenheid\n", "Kind;Material;NLCS layer;Quantity;Unit\n"));
    let (k_layer, k_kerb, k_surface) = (crate::lang::tr("laag", "layer"), crate::lang::tr("band", "kerb"), crate::lang::tr("oppervlak", "surface"));
    let mut materials = Vec::new();
    for (m, (id, v)) in &qty.layers {
        mcsv.push_str(&format!("{k_layer};{m};{};{};m3\n", layer_name(*id), dec(*v, 2)));
        materials.push(serde_json::json!({"materiaal": m, "laag": layer_name(*id), "volume_m3": v}));
    }
    let mut kerbs = Vec::new();
    for (m, (id, l, v)) in &qty.kerbs {
        mcsv.push_str(&format!("{k_kerb};{m};{};{};m\n", layer_name(*id), dec(*l, 1)));
        mcsv.push_str(&format!("{k_kerb};{m};{};{};m3\n", layer_name(*id), dec(*v, 2)));
        kerbs.push(serde_json::json!({"naam": m, "laag": layer_name(*id), "lengte_m": l, "volume_m3": v}));
    }
    let mut surfaces = Vec::new();
    for (id, a) in &qty.surfaces {
        mcsv.push_str(&format!("{k_surface};{};{};{};m2\n", nlcs::object(*id).map(|o| nlcs::humanize(&o.name)).unwrap_or_default(), layer_name(*id), dec(*a, 1)));
        surfaces.push(serde_json::json!({"laag": layer_name(*id), "oppervlak_m2": a}));
    }
    let mcsv_path = crate::bridge::beside_drawing(&format!("corridor_{file}_materialen.csv"));
    if !qty.layers.is_empty() || !qty.kerbs.is_empty() {
        let _ = std::fs::write(&mcsv_path, mcsv);
        super::api::note("materialen_csv", mcsv_path.to_string_lossy().into_owned());
    }
    super::api::note("corridor", format!("Corridor {name}"));
    super::api::note("sjabloon", asm.name.clone());
    super::api::note("profieltype", asm.name.clone());
    super::api::note("lengte_m", len_m);
    super::api::note("van", al.start + sections[0].s);
    super::api::note("tot", al.start + sections[sections.len() - 1].s);
    super::api::note("dwarsprofielen", sections.len());
    super::api::note("driehoeken", face_count);
    super::api::note("driehoeken_onderkant", datum_faces);
    super::api::note("driehoeken_formatie", formation_faces);
    super::api::note("kantlijnen", lines_n);
    super::api::note("afgraven_m3", cut);
    super::api::note("ophogen_m3", fill);
    super::api::note("balans_m3", fill - cut);
    super::api::note("afgraven_onderkant_m3", cut_d);
    super::api::note("ophogen_onderkant_m3", fill_d);
    super::api::note("materialen", materials);
    super::api::note("banden", kerbs);
    super::api::note("oppervlakken", surfaces);
    super::api::note("csv", csv_path.to_string_lossy().into_owned());
    super::api::note("eerste_handle", handles.first().map(|h| super::api::handle_text(*h)).unwrap_or_default());
    super::api::note(
        "profielen",
        sections
            .iter()
            .zip(&vols)
            .map(|(s, v)| {
                serde_json::json!({
                    "station": al.start + s.s, "ontwerp": s.z_axis, "afgraven_m2": s.cut, "ophogen_m2": s.fill, "balans_m3": v.2,
                    "links": s.pts.first().map(|p| p.o), "rechts": s.pts.last().map(|p| p.o),
                    "codes": s.pts.iter().map(|p| p.code).collect::<Vec<_>>().join(" "),
                })
            })
            .collect::<Vec<_>>(),
    );
    let (zlo, zhi) = sections.iter().fold((f64::MAX, f64::MIN), |a, s| (a.0.min(s.z_axis), a.1.max(s.z_axis)));
    let layer_total: f64 = qty.layers.values().map(|v| v.1).sum();
    let skipped_t = if skipped > 0 { trf!(" ({} zonder ontwerphoogte)", " ({} without a design height)", skipped) } else { String::new() };
    let layers_t = if layer_total > 0.0 {
        trf!("; verharding en fundering {} m³ in {} lagen", "; pavement and foundation {} m³ in {} layers", super::terrein::m3(layer_total), qty.layers.len())
    } else {
        String::new()
    };
    let views_t = if view_rows > 0 { trf!(", dwarsprofielen in {} rijen", ", cross sections in {} rows", view_rows) } else { String::new() };
    let balance = format!("{}{}", if fill >= cut { "+" } else { "-" }, super::terrein::m3((fill - cut).abs()));
    Ok(trf!(
        "Corridor {} ({}, {:.0} m): {} dwarsprofielen{}, ontwerp {} tot {} m; afgraven {} m³, ophogen {} m³ (balans {} m³){}; {} driehoeken, {} kantlijnen{} — {} elementen. Lijst: {}",
        "Corridor {} ({}, {:.0} m): {} cross sections{}, design {} to {} m; cut {} m³, fill {} m³ (balance {} m³){}; {} triangles, {} feature lines{} — {} entities. List: {}",
        name,
        asm.name,
        sections[sections.len() - 1].s - sections[0].s,
        sections.len(),
        skipped_t,
        nap(zlo, 2),
        nap(zhi, 2),
        super::terrein::m3(cut),
        super::terrein::m3(fill),
        balance,
        layers_t,
        face_count,
        lines_n,
        views_t,
        handles.len() + view_count,
        csv_path.display()
    ))
}

/// The design surface of a corridor read back (for checks).
pub fn design_surface(doc: &CadDocument) -> Option<Tin> {
    Tin::of_status(doc, 'N')
}

/// Handles of everything a corridor drew (by its object data), to replace it.
pub fn corridor_handles(doc: &CadDocument, name: &str) -> HashSet<Handle> {
    doc.model_space_entities().filter(|e| super::data_value(e, "corridor").as_deref() == Some(name)).map(|e| e.common().handle).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weg() -> Assembly {
        let mut a = Assembly::symmetric("weg", vec![Part::new(PartKind::Lane, 3.25, -0.025), Part::new(PartKind::Shoulder, 1.5, -0.04)]);
        a.superelevation = 0.04;
        a
    }

    #[test]
    fn a_road_in_fill_and_cut() {
        let asm = weg();
        assert!((asm.half_width() - 4.75).abs() < 1e-9);
        // Flat ground at 0, road at +1: fill with 1:3 slopes to the toe.
        let mut flat = |_: f64| Some(0.0);
        let s = section(&asm, 0.0, 1.0, None, &mut flat);
        let edge_z = 1.0 - 0.025 * 3.25 - 0.04 * 1.5;
        let toe = 4.75 + edge_z * 3.0;
        assert!((s.pts[0].o + toe).abs() < 0.01 && (s.pts.last().unwrap().o - toe).abs() < 0.01, "{:?}", s.pts);
        assert_eq!(s.fill_side, [Some(true), Some(true)]);
        // Area: the trapezoid of the road body (axis to edge) plus the slopes.
        let expected = 2.0 * ((1.0 + (1.0 - 0.025 * 3.25)) / 2.0 * 3.25 + ((1.0 - 0.025 * 3.25) + edge_z) / 2.0 * 1.5 + edge_z * edge_z * 3.0 / 2.0);
        assert!((s.fill - expected).abs() < 0.02, "{} vs {expected}", s.fill);
        assert!(s.cut < 1e-9);
        // Without layers the subgrade is the top.
        assert_eq!((s.cut_datum, s.fill_datum), (s.cut, s.fill));
        // Road at −1: cut with 1:2 slopes.
        let s = section(&asm, 0.0, -1.0, None, &mut flat);
        assert_eq!(s.fill_side, [Some(false), Some(false)]);
        assert!(s.fill < 1e-9 && s.cut > 10.0);
    }

    #[test]
    fn superelevation_tilts_towards_the_inside() {
        let asm = weg();
        // In a left curve: left lane down 4 %, right lane up 4 %.
        let sl = lane_slopes(&asm, Some(1.0), [-0.025; 2]);
        assert!((sl[0] + 0.04).abs() < 1e-12 && (sl[1] - 0.04).abs() < 1e-12);
        // Halfway the runoff: halfway between crown and full.
        let sl = lane_slopes(&asm, Some(-0.5), [-0.025; 2]);
        assert!((sl[1] - (-0.025 + (-0.04 + 0.025) * 0.5)).abs() < 1e-12);
        assert!((sl[0] - (-0.025 + (0.04 + 0.025) * 0.5)).abs() < 1e-12);
        // Along an axis: full in the curve, nothing far from it.
        let al = Alignment::from_pis(&[[0.0, 0.0], [200.0, 0.0], [200.0, 200.0]], &[100.0]).unwrap();
        assert_eq!(curve_share(&al, 150.0, 1.0, 30.0), Some(1.0));
        assert!((curve_share(&al, 85.0, 1.0, 30.0).unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(curve_share(&al, 10.0, 1.0, 30.0), None);
    }

    #[test]
    fn average_end_areas() {
        let mk = |s: f64, cut: f64, fill: f64| Section {
            s,
            z_axis: 0.0,
            pts: vec![],
            fill_side: [None, None],
            cut,
            fill,
            cut_datum: cut,
            fill_datum: fill,
            datum: vec![],
            formation: vec![],
            bodies: vec![],
            surfaces: vec![],
        };
        let v = volumes(&[mk(0.0, 0.0, 10.0), mk(20.0, 4.0, 6.0), mk(40.0, 8.0, 0.0)]);
        assert_eq!(v[1], (40.0, 160.0, 120.0));
        assert_eq!(v[2], (120.0, 60.0, 60.0));
    }

    /// A town street side: lane, gutter, kerb, footway, all with layers.
    fn street() -> Assembly {
        let mut lane = Part::new(PartKind::Lane, 3.0, -0.025);
        lane.layers = vec![Layer::new("AC 11 surf (deklaag)", 0.04), Layer::new("AC 16 bin (tussenlaag)", 0.06), Layer::new("menggranulaat", 0.25), Layer::new("zand (zandbed)", 0.30)];
        let mut gutter = Part::new(PartKind::Gutter, 0.30, -0.025);
        gutter.layers = vec![Layer::new("betonstraatsteen", 0.08), Layer::new("straatlaag (brekerzand)", 0.05), Layer::new("menggranulaat", 0.25)];
        let mut kerb = Part::new(PartKind::Kerb, 0.15, 0.0);
        kerb.name = "trottoirband 13/15×25".into();
        (kerb.upstand, kerb.height, kerb.batter) = (0.12, 0.25, 0.02);
        let mut footway = Part::new(PartKind::Footway, 2.0, 0.02);
        footway.layers = vec![Layer::new("betontegels 30×30×5", 0.05), Layer::new("straatlaag (straatzand)", 0.05), Layer::new("zand (zandbed)", 0.20)];
        Assembly::symmetric("straat", vec![lane, gutter, kerb, footway])
    }

    #[test]
    fn kerb_footway_and_layers() {
        let asm = street();
        let mut flat = |_: f64| Some(0.0);
        let s = section(&asm, 0.0, 1.0, None, &mut flat);
        let r: Vec<&Pt> = s.pts.iter().filter(|p| p.o > 0.0).collect();
        let codes: Vec<&str> = r.iter().map(|p| p.code).collect();
        assert_eq!(codes, ["RS", "GO", "BV", "BA", "TR", "TL"]);
        let gutter_z = 1.0 - 0.025 * 3.3;
        // The kerb stands 12 cm above the gutter; its front face runs 2 cm.
        assert!((r[2].z - (gutter_z + 0.12)).abs() < 1e-9 && (r[2].o - 3.32).abs() < 1e-9);
        assert!((r[3].o - 3.45).abs() < 1e-9 && (r[3].z - r[2].z).abs() < 1e-9);
        // The footway rises 2 % away from the road.
        assert!((r[4].z - (r[3].z + 0.04)).abs() < 1e-9 && (r[4].o - 5.45).abs() < 1e-9);
        // Layers: width × thickness; the kerb's body is its outline.
        let area = |m: &str| s.bodies.iter().filter(|b| b.material == m).map(|b| b.area).sum::<f64>();
        assert!((area("AC 11 surf (deklaag)") - 2.0 * 3.0 * 0.04).abs() < 1e-9);
        assert!((area("menggranulaat") - 2.0 * (3.0 + 0.3) * 0.25).abs() < 1e-9);
        let kerb = area("trottoirband 13/15×25") / 2.0;
        assert!((kerb - (0.15 * 0.25 - 0.02 * 0.12 / 2.0)).abs() < 1e-6, "{kerb}");
        // The subgrade lies under the lane's 0,65 m build-up at the axis.
        assert!((interp(&s.datum, 0.0).unwrap() - (1.0 - 0.65)).abs() < 1e-9);
        // The formation: under the asphalt (10 cm) of the lane.
        let (_, _, [a, _]) = s.formation.iter().find(|f| f.0 == 1 && f.1 == 0).unwrap();
        assert!((a.1 - 0.90).abs() < 1e-9);
        // Earthwork to the subgrade is more fill-less than to the top.
        assert!(s.fill_datum < s.fill);
        // The carriageway edge for junctions: lanes and gutter, then the kerb.
        let (edge, kerb) = asm.edge(1);
        assert!((edge - 3.3).abs() < 1e-9 && kerb.is_some());
    }

    #[test]
    fn ditch_and_conditions() {
        // In cut a ditch, otherwise a shoulder.
        let lane = Part::new(PartKind::Lane, 2.5, -0.025);
        let mut ditch = Part::new(PartKind::Ditch, 0.0, 0.0);
        ditch.ditch = Some(Ditch { bottom: 0.5, depth: 0.8, level: None, inner: 1.5, outer: 2.0 });
        ditch.condition = Some(Condition { cut: true, min: 0.0, max: 99.0 });
        let mut berm = Part::new(PartKind::Shoulder, 1.0, -0.04);
        berm.condition = Some(Condition { cut: false, min: 0.0, max: 99.0 });
        let asm = Assembly::symmetric("polder", vec![lane, ditch, berm]);
        // Ground at +0.5, road at 0: cut. Ditch bottom at 0 − 0,0625 − 0,8.
        let mut high = |_: f64| Some(0.5);
        let s = section(&asm, 0.0, 0.0, None, &mut high);
        let r: Vec<&Pt> = s.pts.iter().filter(|p| p.o > 0.0).collect();
        assert_eq!(r.iter().map(|p| p.code).collect::<Vec<_>>(), ["RS", "SL", "SL", "TL"]);
        let edge = -0.025 * 2.5;
        assert!((r[1].z - (edge - 0.8)).abs() < 1e-9 && (r[1].o - (2.5 + 1.2)).abs() < 1e-9);
        assert!((r[2].o - (2.5 + 1.7)).abs() < 1e-9);
        // The back slope 1:2 rises to the ground at +0,5.
        assert!((r[3].z - 0.5).abs() < 0.01 && (r[3].o - (4.2 + (0.5 - r[1].z) * 2.0)).abs() < 0.02, "{:?}", r[3]);
        assert_eq!(s.fill_side[1], Some(false));
        // Ground at −1: fill, so the shoulder and a fill slope.
        let mut low = |_: f64| Some(-1.0);
        let s = section(&asm, 0.0, 0.0, None, &mut low);
        let r: Vec<&str> = s.pts.iter().filter(|p| p.o > 0.0).map(|p| p.code).collect();
        assert_eq!(r, ["RS", "BE", "TL"]);
        assert_eq!(s.fill_side[1], Some(true));
        // A minimum depth: a shallow cut is no cut.
        let mut asm2 = asm.clone();
        asm2.sides[1][1].condition = Some(Condition { cut: true, min: 1.0, max: 99.0 });
        let s = section(&asm2, 0.0, 0.0, None, &mut high);
        assert!(!s.pts.iter().any(|p| p.o > 0.0 && p.code == "SL"));
        // A ditch that is not the last part rises back to its inner edge.
        let mut asm3 = asm.clone();
        asm3.sides[1][1].condition = None;
        asm3.sides[1][2].condition = None;
        let s = section(&asm3, 0.0, 0.0, None, &mut high);
        let r: Vec<&Pt> = s.pts.iter().filter(|p| p.o > 0.0).collect();
        assert_eq!(r.iter().map(|p| p.code).collect::<Vec<_>>(), ["RS", "SL", "SL", "SI", "BE", "TL"]);
        assert!((r[3].z - edge).abs() < 1e-9 && (r[3].o - (4.2 + 0.8 * 2.0)).abs() < 1e-9);
        // Bottom level instead of depth.
        let mut asm4 = asm3.clone();
        asm4.sides[1][1].ditch = Some(Ditch { level: Some(-1.5), ..asm4.sides[1][1].ditch.unwrap() });
        let s = section(&asm4, 0.0, 0.0, None, &mut high);
        assert!(s.pts.iter().any(|p| p.code == "SL" && (p.z + 1.5).abs() < 1e-9));
    }

    #[test]
    fn targets_for_width_and_level() {
        // A footway to a building line at 9 m and to the ground at its edge.
        let mut footway = Part::new(PartKind::Footway, 2.0, 0.02);
        footway.width_target = Some(Target::Line("rooilijn".into()));
        footway.level_target = Some(Target::Ground);
        (footway.slope_min, footway.slope_max) = (-0.01, 0.04);
        let asm = Assembly::symmetric("t", vec![Part::new(PartKind::Lane, 3.0, -0.025), footway]);
        let mut t = Targets::default();
        t.add("Rooilijn", 9.0, None);
        t.add("rooilijn", -7.0, None);
        // Ground at the edge 10 cm above the lane edge: slope 0,1 / 6 = 1,7 %.
        let edge = -0.075;
        let mut g = |_: f64| Some(edge + 0.1);
        let s = section_with(&asm, 0.0, 0.0, None, &mut g, &t);
        let r: Vec<&Pt> = s.pts.iter().filter(|p| p.code == "TR").collect();
        assert!((r[0].o + 7.0).abs() < 1e-9 && (r[1].o - 9.0).abs() < 1e-9);
        assert!((r[1].z - (edge + 0.1)).abs() < 1e-9);
        // Too steep: the slope stops at its limit (4 %).
        let mut g = |_: f64| Some(edge + 1.0);
        let s = section_with(&asm, 0.0, 0.0, None, &mut g, &t);
        let r = s.pts.iter().find(|p| p.code == "TR" && p.o > 0.0).unwrap();
        assert!((r.z - (edge + 0.04 * 6.0)).abs() < 1e-9);
        // An offset target without a line: the fixed distance.
        let mut asm2 = asm.clone();
        asm2.sides[1][1].width_target = Some(Target::Offset(5.0));
        let s = section_with(&asm2, 0.0, 0.0, None, &mut g, &Targets::default());
        assert!((s.pts.iter().find(|p| p.code == "TR" && p.o > 0.0).unwrap().o - 5.0).abs() < 1e-9);
        // Crossings of a section line with a polyline, with heights.
        let line = vec![([5.0, -10.0], Some(1.0)), ([5.0, 10.0], Some(3.0))];
        let c = crossings([0.0, 0.0], [1.0, 0.0], 1.0, &line);
        assert_eq!(c.len(), 1);
        assert!((c[0].0 - 5.0).abs() < 1e-9 && (c[0].1.unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn material_quantities() {
        let asm = street();
        let mut flat = |_: f64| Some(0.0);
        let a = section(&asm, 0.0, 1.0, None, &mut flat);
        let b = section(&asm, 10.0, 1.0, None, &mut flat);
        let q = quantities(&[a, b]);
        assert!((q.layers["AC 11 surf (deklaag)"].1 - 2.0 * 3.0 * 0.04 * 10.0).abs() < 1e-9);
        let (_, len, _) = q.kerbs["trottoirband 13/15×25"];
        assert!((len - 20.0).abs() < 1e-9, "two kerbs over 10 m: {len}");
        let tiles = q.surfaces[&nlcs_id("VH", "OPENVERHARDING_TEGEL")];
        assert!((tiles - 2.0 * 2.0 * 10.0).abs() < 1e-9);
        assert!(material_nlcs("AC 8 surf rood") == nlcs_id("VH", "GESLOTENVERHARDING_ASFALT_ROOD"));
        assert!(material_nlcs("menggranulaat") == nlcs_id("VH", "FUNDERING_GRANULAAT"));
        assert!(bound_default("straatlaag (brekerzand)") && !bound_default("zand (zandbed)"));
    }

    #[test]
    fn stitching_covers_the_strip() {
        // Two straight sections 1 m apart with different points: the plan
        // area of the triangles is the strip's.
        let a: Vec<(f64, P3)> = [-5.0, -1.0, 0.0, 3.0, 5.0].iter().map(|o| (*o, [*o, 0.0, 0.0])).collect();
        let b: Vec<(f64, P3)> = [-5.0, 0.0, 2.0, 2.5, 5.0].iter().map(|o| (*o, [*o, 1.0, 0.0])).collect();
        let mut m = MeshBuilder::default();
        stitch(&a, &b, &mut m);
        assert_eq!(m.faces(), a.len() + b.len() - 2);
        let ents = m.finish("x", None, None);
        let EntityType::Mesh(mesh) = &ents[0] else { panic!() };
        let area: f64 = mesh
            .faces
            .iter()
            .map(|f| {
                let p: Vec<_> = f.vertices.iter().map(|i| mesh.vertices[*i]).collect();
                ((p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y)).abs() / 2.0
            })
            .sum();
        assert!((area - 10.0).abs() < 1e-9, "{area}");
    }
}
