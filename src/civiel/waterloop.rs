//! `CIV_WATERLOOP` (issue #74): the steady water surface along a watercourse
//! at a design discharge. Cross sections across the axis come from the
//! ground (the TIN, else AHN or the worldwide terrain, as
//! `CIV_DWARSPROFIELEN` samples them); `rekenkern::waterloop` runs the
//! standard step method with culverts and weirs. The result is drawn as the
//! water line in the axis's length profile, the inundation boundary in plan
//! (the outer waterlines left and right) on GW WATERGANG_WATERLIJN, and a
//! Markdown and JSON report beside the drawing.
//!
//! The method, defaults and limitations are in `docs/civiel-water.md`.

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};
use rekenkern::waterloop::{self as kern, Boundary, CrossSection, Culvert, CulvertShape, Profile, ProfileResult, Structure, StructureKind, Weir};
use serde_json::{json, Value};

use super::as_profiel::Alignment;
use super::{api, data_value, record, station, Ctx, Maaiveld, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::Element;

/// A message in the interface language, with positional arguments.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

// ── Design values (sources: docs/civiel-water.md) ───────────────────────────

/// Manning n of the watercourse (s/m^⅓): Chow, *Open-Channel Hydraulics*
/// (1959) table 5-6, excavated earth channel, winding and sluggish, dense
/// weeds: normal value 0.035. A maintained ditch with grass is nearer 0.030.
pub const RUWHEID: f64 = 0.035;
/// Manning n of a culvert barrel: concrete pipe, 0.012–0.013 (FHWA HDS-5,
/// *Hydraulic Design of Highway Culverts*, 3rd ed. 2012, appendix table);
/// the upper value.
pub const RUWHEID_DUIKER: f64 = 0.013;
/// Distance (m) beyond each end of a structure at which its bounding cross
/// sections lie: the sections stand in the open channel, not on the
/// embankment over a culvert.
pub const MARGE: f64 = 1.0;
/// Length along the axis (m) taken by a weir between its two sections.
pub const STUW_LENGTE: f64 = 1.0;
/// Most cross sections in one run, as `CIV_DWARSPROFIELEN`.
pub const MAX_SECTIES: usize = 400;

pub const ARGS: &[&str] = &["AS", "Q", "RUWHEID", "BENEDENSTROOMS", "INTERVAL", "BREEDTE", "DUIKER", "STUW", "RICHTING"];

/// The downstream boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum Beneden {
    /// A known water level (m).
    Peil(f64),
    /// Normal depth on the mean bed slope.
    Normaal,
    /// Critical depth (a free fall).
    Kritiek,
}

/// The flow direction relative to the axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Richting {
    /// From the bed: the water runs to the lower end.
    Auto,
    /// Along the axis (stations increase downstream).
    Mee,
    /// Against the axis.
    Tegen,
}

/// A culvert at a station of the axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Duiker {
    /// Station of the middle (m, the axis's stationing).
    pub station: f64,
    pub vorm: CulvertShape,
    pub lengte: f64,
    /// Invert at the inlet (upstream) and the outlet (downstream), m.
    pub bob_in: f64,
    pub bob_uit: f64,
}

/// A weir at a station of the axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Stuw {
    pub station: f64,
    pub kruin: f64,
    pub breedte: f64,
}

/// What `CIV_WATERLOOP` computes.
#[derive(Clone, Debug)]
pub struct Waterloop {
    pub as_h: Option<Handle>,
    /// Discharge (m³/s).
    pub q: f64,
    /// Manning n.
    pub n: f64,
    pub beneden: Beneden,
    /// Cross sections every (m); the settings' section distance if none.
    pub interval: Option<f64>,
    /// Half width of the cross sections (m); the settings' widths if none.
    pub breedte: Option<f64>,
    pub duikers: Vec<Duiker>,
    pub stuwen: Vec<Stuw>,
    pub richting: Richting,
}

/// A length in metres, or in millimetres from 20 up (a Ø800 culvert).
pub fn metres(v: f64) -> f64 {
    if v >= 20.0 { v / 1000.0 } else { v }
}

fn num(t: &str, what: &str) -> Result<f64, String> {
    t.trim().parse::<f64>().map_err(|_| t!("{}: geen getal '{}'", "{}: not a number '{}'", what, t.trim()))
}

/// RUWHEID=: Manning n (below 1), or a Manning coefficient k_M = 1/n in
/// m^⅓/s (1 and up, as Dutch water boards write it: 25–35).
pub fn parse_ruwheid(s: &str) -> Result<f64, String> {
    let v = num(&s.replace(',', "."), "RUWHEID")?;
    if v <= 0.0 {
        return Err(tr("RUWHEID: Manning n (bijv. 0.035) of k_M (bijv. 30) groter dan 0", "RUWHEID: Manning n (e.g. 0.035) or k_M (e.g. 30) above 0").into());
    }
    Ok(if v >= 1.0 { 1.0 / v } else { v })
}

/// BENEDENSTROOMS=<level>|NORMAAL|KRITIEK.
pub fn parse_beneden(s: &str) -> Result<Beneden, String> {
    match s.trim().to_ascii_uppercase().as_str() {
        "NORMAAL" | "NORMAL" | "N" => Ok(Beneden::Normaal),
        "KRITIEK" | "CRITICAL" | "K" | "C" => Ok(Beneden::Kritiek),
        v => num(&v.replace(',', "."), "BENEDENSTROOMS").map(Beneden::Peil).map_err(|_| tr("BENEDENSTROOMS: een peil (m), NORMAAL of KRITIEK", "BENEDENSTROOMS: a level (m), NORMAAL (normal depth) or KRITIEK (critical depth)").into()),
    }
}

/// RICHTING=AUTO|MEE|TEGEN.
pub fn parse_richting(s: &str) -> Result<Richting, String> {
    match s.trim().to_ascii_uppercase().as_str() {
        "AUTO" => Ok(Richting::Auto),
        "MEE" | "AS" | "ALONG" => Ok(Richting::Mee),
        "TEGEN" | "AGAINST" => Ok(Richting::Tegen),
        _ => Err(tr("RICHTING: AUTO, MEE (stroomt met de as mee) of TEGEN", "RICHTING: AUTO, MEE (flows along the axis) or TEGEN (against it)").into()),
    }
}

/// A barrel size: a diameter ("0.8", "800") or width × height ("2x1.5", "2000x1500").
pub fn parse_vorm(s: &str) -> Result<CulvertShape, String> {
    let t = s.trim().to_ascii_lowercase();
    let shape = match t.split_once(['x', '×']) {
        Some((w, h)) => CulvertShape::Rectangular { width: metres(num(w, "DUIKER")?), height: metres(num(h, "DUIKER")?) },
        None => CulvertShape::Circular { diameter: metres(num(&t, "DUIKER")?) },
    };
    let ok = match shape {
        CulvertShape::Circular { diameter } => diameter > 0.0,
        CulvertShape::Rectangular { width, height } => width > 0.0 && height > 0.0,
    };
    if ok { Ok(shape) } else { Err(t!("DUIKER: maat '{}' moet groter dan 0 zijn", "DUIKER: size '{}' must be above 0", s)) }
}

/// DUIKER=station,diameter,lengte,bob_in,bob_uit;… (decimal points).
pub fn parse_duikers(s: &str) -> Result<Vec<Duiker>, String> {
    s.split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let f: Vec<&str> = p.split(',').collect();
            if f.len() != 5 {
                return Err(t!("DUIKER: verwacht station,diameter,lengte,bob_in,bob_uit (punt als decimaalteken), kreeg '{}'", "DUIKER: expected station,diameter,length,invert_in,invert_out (decimal point), got '{}'", p));
            }
            let d = Duiker { station: num(f[0], "DUIKER")?, vorm: parse_vorm(f[1])?, lengte: num(f[2], "DUIKER")?, bob_in: num(f[3], "DUIKER")?, bob_uit: num(f[4], "DUIKER")? };
            if d.lengte <= 0.0 {
                return Err(t!("DUIKER bij {}: lengte moet groter dan 0 zijn", "DUIKER at {}: length must be above 0", d.station));
            }
            Ok(d)
        })
        .collect()
}

/// STUW=station,kruin,breedte;…
pub fn parse_stuwen(s: &str) -> Result<Vec<Stuw>, String> {
    s.split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let f: Vec<&str> = p.split(',').collect();
            if f.len() != 3 {
                return Err(t!("STUW: verwacht station,kruin,breedte (punt als decimaalteken), kreeg '{}'", "STUW: expected station,crest,width (decimal point), got '{}'", p));
            }
            let w = Stuw { station: num(f[0], "STUW")?, kruin: num(f[1], "STUW")?, breedte: num(f[2], "STUW")? };
            if w.breedte <= 0.0 {
                return Err(t!("STUW bij {}: breedte moet groter dan 0 zijn", "STUW at {}: width must be above 0", w.station));
            }
            Ok(w)
        })
        .collect()
}

impl Waterloop {
    pub fn from_args(a: &api::Args) -> Result<Waterloop, String> {
        let q = a.f64("Q")?.ok_or_else(|| tr("geef het afvoerdebiet: Q=<m³/s>", "give the discharge: Q=<m³/s>").to_string())?;
        if q <= 0.0 {
            return Err(tr("Q: het debiet moet groter dan 0 zijn (m³/s)", "Q: the discharge must be above 0 (m³/s)").into());
        }
        Ok(Waterloop {
            as_h: a.handle("AS")?,
            q,
            n: a.str("RUWHEID").map(parse_ruwheid).transpose()?.unwrap_or(RUWHEID),
            beneden: a.str("BENEDENSTROOMS").map(parse_beneden).transpose()?.unwrap_or(Beneden::Normaal),
            interval: a.f64("INTERVAL")?.map(|v| v.max(1.0)),
            breedte: a.f64("BREEDTE")?.map(|v| v.max(1.0)),
            duikers: a.str("DUIKER").map(parse_duikers).transpose()?.unwrap_or_default(),
            stuwen: a.str("STUW").map(parse_stuwen).transpose()?.unwrap_or_default(),
            richting: a.str("RICHTING").map(parse_richting).transpose()?.unwrap_or(Richting::Auto),
        })
    }

    /// The structures' footprints along the axis: (from, to) in metres from
    /// the start of the axis, with the margin; checked to lie inside it and
    /// apart.
    pub fn footprints(&self, start: f64, len: f64) -> Result<Vec<(f64, f64)>, String> {
        let mut out: Vec<(f64, f64, f64)> = self
            .duikers
            .iter()
            .map(|d| (d.station, d.lengte / 2.0))
            .chain(self.stuwen.iter().map(|w| (w.station, STUW_LENGTE / 2.0)))
            .map(|(st, half)| {
                let c = st - start;
                (c - half - MARGE, c + half + MARGE, st)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        for f in &out {
            if f.0 <= 1e-6 || f.1 >= len - 1e-6 {
                return Err(t!(
                    "het kunstwerk bij station {} ligt te dicht bij het begin of eind van de as: er moet aan beide kanten een dwarsprofiel passen",
                    "the structure at station {} lies too close to the start or end of the axis: a cross section must fit on both sides",
                    station(f.2)
                ));
            }
        }
        for w in out.windows(2) {
            if w[1].0 < w[0].1 {
                return Err(t!("de kunstwerken bij {} en {} liggen te dicht bij elkaar", "the structures at {} and {} lie too close together", station(w[0].2), station(w[1].2)));
            }
        }
        Ok(out.into_iter().map(|f| (f.0, f.1)).collect())
    }
}

// ── From the drawing to the kernel ──────────────────────────────────────────

/// Where the cross sections lie (metres from the start of the axis): every
/// `every` m, the ends, and on both sides of each structure footprint, with
/// none inside a footprint.
pub fn section_distances(len: f64, every: f64, footprints: &[(f64, f64)]) -> Vec<f64> {
    let mut d: Vec<f64> = Vec::new();
    let mut s = 0.0;
    while s < len - 1e-9 {
        d.push(s);
        s += every;
    }
    d.push(len);
    let near = (0.25 * every).min(1.0);
    d.retain(|s| footprints.iter().all(|&(a, b)| *s <= a - near || *s >= b + near));
    for &(a, b) in footprints {
        d.push(a);
        d.push(b);
    }
    d.sort_by(f64::total_cmp);
    d.dedup_by(|a, b| (*a - *b).abs() < 0.05);
    d
}

/// A kernel cross section from ground samples (axis offset m, right > 0;
/// height): points left to right looking downstream. When the water runs
/// against the axis, looking downstream is looking back, so the offsets
/// change sign.
pub fn section_points(samples: &[(f64, Option<f64>)], along: bool) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = samples.iter().filter_map(|(o, z)| z.map(|z| [if along { *o } else { -*o }, z])).collect();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    pts
}

/// A kernel offset (looking downstream) back to the axis offset.
pub fn axis_offset(kernel: f64, along: bool) -> f64 {
    if along { kernel } else { -kernel }
}

/// Least-squares slope dz/dd of the bed (lowest point) along the axis.
pub fn bed_gradient(bed: &[(f64, f64)]) -> Option<f64> {
    let n = bed.len() as f64;
    if bed.len() < 2 {
        return None;
    }
    let (md, mz) = (bed.iter().map(|b| b.0).sum::<f64>() / n, bed.iter().map(|b| b.1).sum::<f64>() / n);
    let sdd: f64 = bed.iter().map(|b| (b.0 - md).powi(2)).sum();
    (sdd > 1e-12).then(|| bed.iter().map(|b| (b.0 - md) * (b.1 - mz)).sum::<f64>() / sdd)
}

/// Whether the water runs along the axis: the bed falls along it (or is
/// flat: an axis is usually drawn with the flow).
pub fn flows_along(bed: &[(f64, f64)]) -> bool {
    bed_gradient(bed).map_or(true, |g| g <= 1e-6)
}

/// The kernel computation from sections (station m, ground samples) and the request.
pub fn profile(w: &Waterloop, sections: &[(f64, Vec<(f64, Option<f64>)>)], along: bool) -> Result<Profile, String> {
    let mut secs = Vec::with_capacity(sections.len());
    for (st, samples) in sections {
        let pts = section_points(samples, along);
        if pts.len() < 3 {
            return Err(t!("dwarsprofiel bij {}: te weinig hoogtes in het maaiveld", "cross section at {}: too few ground heights", station(*st)));
        }
        secs.push(CrossSection { station: *st, points: pts, n: w.n, contraction: None, expansion: None });
    }
    let boundary = match w.beneden {
        Beneden::Peil(z) => Boundary::Level(z),
        Beneden::Kritiek => Boundary::CriticalDepth,
        Beneden::Normaal => {
            let bed: Vec<(f64, f64)> = secs.iter().map(|s| (s.station, s.z_min())).collect();
            let fall = bed_gradient(&bed).map(|g| if along { -g } else { g }).unwrap_or(0.0);
            if fall <= 1e-5 {
                return Err(tr(
                    "de bodem heeft geen verval in de stroomrichting, dus geen normaaldiepte: geef BENEDENSTROOMS=<peil> of KRITIEK",
                    "the bed has no fall in the flow direction, so no normal depth: give BENEDENSTROOMS=<level> or KRITIEK",
                )
                .into());
            }
            Boundary::NormalDepth(fall)
        }
    };
    let mut structures = Vec::new();
    for d in &w.duikers {
        structures.push(Structure {
            station: d.station,
            kind: StructureKind::Culvert(Culvert { shape: d.vorm.clone(), length: d.lengte, invert_up: d.bob_in, invert_down: d.bob_uit, n: RUWHEID_DUIKER, entrance: kern::ENTRANCE, exit: kern::EXIT }),
        });
    }
    for s in &w.stuwen {
        structures.push(Structure { station: s.station, kind: StructureKind::Weir(Weir { crest: s.kruin, width: s.breedte, c: kern::WEIR_C }) });
    }
    Ok(Profile { q: w.q, sections: secs, structures, boundary, stations_downstream: along, contraction: kern::CONTRACTION, expansion: kern::EXPANSION })
}

/// A length profile frame: where station and height land in the drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Kader {
    pub x_nul: f64,
    pub y_basis: f64,
    pub basis: f64,
    pub overhoging: f64,
    pub eenheid: f64,
    pub beginstation: f64,
}

impl Kader {
    /// From the object data `CIV_LENGTEPROFIEL` writes on its frame.
    pub fn of(e: &EntityType) -> Option<Kader> {
        let g = |k: &str| data_value(e, k).and_then(|v| v.parse::<f64>().ok());
        Some(Kader { x_nul: g("x_nul")?, y_basis: g("y_basis")?, basis: g("basis")?, overhoging: g("overhoging")?, eenheid: g("eenheid")?, beginstation: g("beginstation")? })
    }

    pub fn xy(&self, st: f64, z: f64) -> XY {
        [self.x_nul + (st - self.beginstation) * self.eenheid, self.y_basis + (z - self.basis) * self.overhoging * self.eenheid]
    }
}

/// The newest length profile frame of the axis named `name`.
fn frame(doc: &CadDocument, name: &str) -> Option<Kader> {
    doc.model_space_entities().filter(|e| data_value(e, "lengteprofiel").is_some() && data_value(e, "as").as_deref().unwrap_or("") == name).filter_map(Kader::of).last()
}

fn dec(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

fn slug(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "as".into() } else { s }
}

fn shape_text(s: &CulvertShape) -> String {
    match s {
        CulvertShape::Circular { diameter } => format!("Ø{}", dec(*diameter, 2)),
        CulvertShape::Rectangular { width, height } => format!("{}×{}", dec(*width, 2), dec(*height, 2)),
    }
}

/// The report as Markdown.
fn report(name: &str, w: &Waterloop, r: &ProfileResult, along: bool, ground: &str) -> String {
    let mut s = format!("# {} {name}\n\n", tr("Waterlijnberekening", "Water surface profile"));
    s.push_str(&t!(
        "Q = {} m³/s, Manning n = {}, benedenstrooms: {}, stroomrichting: {} de as. Maaiveld: {}.\n\n",
        "Q = {} m³/s, Manning n = {}, downstream: {}, flow direction: {} the axis. Ground: {}.\n\n",
        dec(w.q, 3),
        dec(w.n, 4),
        match w.beneden {
            Beneden::Peil(z) => nap(z, 2),
            Beneden::Normaal => tr("normaaldiepte", "normal depth").into(),
            Beneden::Kritiek => tr("kritieke diepte", "critical depth").into(),
        },
        if along { tr("met", "along") } else { tr("tegen", "against") },
        ground
    ));
    s.push_str(&format!(
        "| Station | {} | {} | v (m/s) | Fr | {} | {} |\n|---|---|---|---|---|---|---|\n",
        tr("Waterstand (m)", "Water level (m)"),
        tr("Diepte (m)", "Depth (m)"),
        tr("Breedte (m)", "Width (m)"),
        tr("Let op", "Note")
    ));
    for x in &r.sections {
        let mut note = Vec::new();
        if x.critical {
            note.push(tr("kritiek", "critical"));
        }
        if x.walls {
            note.push(tr("boven de rand", "above the edge"));
        }
        s.push_str(&format!("| {} | {} | {} | {} | {} | {} | {} |\n", station(x.station), nap(x.level, 3), dec(x.depth, 2), dec(x.velocity, 2), dec(x.froude, 2), dec(x.top_width, 1), note.join(", ")));
    }
    if !r.structures.is_empty() {
        s.push_str(&format!("\n## {}\n\n| Station | {} | {} | {} | {} |\n|---|---|---|---|---|\n", tr("Kunstwerken", "Structures"), tr("Soort", "Kind"), tr("Benedenstrooms (m)", "Downstream (m)"), tr("Bovenstrooms (m)", "Upstream (m)"), tr("Regime", "Regime")));
        for x in &r.structures {
            let kind = if w.duikers.iter().any(|d| (d.station - x.station).abs() < 1e-9) { tr("duiker", "culvert") } else { tr("stuw", "weir") };
            s.push_str(&format!("| {} | {kind} | {} | {} | {} |\n", station(x.station), nap(x.level_down, 3), nap(x.level_up, 3), x.regime));
        }
    }
    s.push_str(&t!(
        "\nStandaardstapmethode (HEC-RAS Hydraulic Reference Manual 4.1, 2010), Manning per nat deelgebied, alleen stromend (subkritisch) water. Contractie {} / expansie {}; duiker intree {} / uittree {}, n = {}; stuw C = {} m^½/s met Villemonte. Zie docs/civiel-water.md.\n",
        "\nStandard step method (HEC-RAS Hydraulic Reference Manual 4.1, 2010), Manning per wet sub-area, subcritical flow only. Contraction {} / expansion {}; culvert entrance {} / exit {}, n = {}; weir C = {} m^½/s with Villemonte. See docs/civiel-water.md.\n",
        kern::CONTRACTION,
        kern::EXPANSION,
        kern::ENTRANCE,
        kern::EXIT,
        RUWHEID_DUIKER,
        kern::WEIR_C
    ));
    s
}

/// `CIV_WATERLOOP`: the water surface along the axis at discharge Q.
pub fn waterloop(opt: &super::CivielOptions, w: Waterloop, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["tin", "lengteprofiel"], &w.as_h.into_iter().collect::<Vec<_>>())?;
    let (al, _axis_h, _) = super::as_profiel::axis(&doc, w.as_h)?;
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let len = al.length() / f;
    let every = w.interval.unwrap_or(opt.section_every).max(1.0);
    let half = w.breedte.unwrap_or(opt.left.max(opt.right)).max(1.0);
    let footprints = w.footprints(al.start, len)?;
    let ds = section_distances(len, every, &footprints);
    if ds.len() > MAX_SECTIES {
        return Err(t!("{} dwarsprofielen is te veel: kies een groter INTERVAL=", "{} cross sections are too many: choose a larger INTERVAL=", ds.len()));
    }
    // Ground every half metre (finer for narrow sections) across the axis.
    let step = (half / 40.0).clamp(0.1, 0.5).min(opt.step.max(0.1));
    let offsets: Vec<f64> = {
        let n = (2.0 * half / step).ceil() as usize;
        (0..=n).map(|i| -half + 2.0 * half * i as f64 / n as f64).collect()
    };
    progress(t!("{} dwarsprofielen uit {} …", "{} cross sections from {} …", ds.len(), ground.label()));
    let plan = |al: &Alignment, d: f64, o: f64| -> XY {
        let (p, dir) = al.at(d * f);
        [p[0] + dir.sin() * o * f, p[1] - dir.cos() * o * f]
    };
    let mut sections = Vec::with_capacity(ds.len());
    for (i, d) in ds.iter().enumerate() {
        if i % 20 == 0 {
            progress(t!("Dwarsprofiel {}/{} …", "Cross section {}/{} …", i + 1, ds.len()));
        }
        let samples: Vec<(f64, Option<f64>)> = offsets.iter().map(|o| (*o, ground.height(plan(&al, *d, *o), f))).collect();
        sections.push((al.start + d, samples));
    }
    if sections.iter().all(|s| s.1.iter().all(|p| p.1.is_none())) {
        return Err(ground.error().unwrap_or_else(|| t!("geen hoogtes langs de as ({})", "no heights along the axis ({})", ground.label())));
    }
    let bed: Vec<(f64, f64)> = sections.iter().filter_map(|(st, s)| s.iter().filter_map(|p| p.1).reduce(f64::min).map(|z| (*st, z))).collect();
    let along = match w.richting {
        Richting::Mee => true,
        Richting::Tegen => false,
        Richting::Auto => flows_along(&bed),
    };
    let p = profile(&w, &sections, along)?;
    progress(tr("Waterlijn (standaardstapmethode) …", "Water surface (standard step) …").into());
    let r = kern::standard_step(&p)?;

    // ── Drawing ──
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let line = ctx.layer("GW", "WATERGANG_WATERLIJN", Element::G)?;
    let line_t = ctx.layer("GW", "WATERGANG_WATERLIJN", Element::T)?;
    let mut out = Out::default();
    let data = |zijde: &str| record(&[("waterloop", name.clone()), ("as", al.name.clone()), ("q", format!("{}", w.q)), ("zijde", zijde.to_string())]);
    // Inundation boundary: the outer waterlines per section, in plan.
    let side = |which: bool| -> Vec<XY> {
        r.sections
            .iter()
            .map(|x| {
                let o = axis_offset(if which { x.left } else { x.right }, along);
                plan(&al, x.station - al.start, o)
            })
            .collect()
    };
    let (a, b) = (side(true), side(false));
    let (left, right) = if along { (a, b) } else { (b, a) };
    out.put(&line, crate::emit::polylines(&[(left, false)], &line.name, k, Some(&data("links"))));
    out.put(&line, crate::emit::polylines(&[(right, false)], &line.name, k, Some(&data("rechts"))));
    // Levels at the ends and at the structures, in plan.
    let label_at = |st: f64, text: String, out: &mut Out| {
        let d = st - al.start;
        let (p, dir) = al.at(d * f);
        let t = ctx.text([p[0] - dir.sin() * 3.0 * k, p[1] + dir.cos() * 3.0 * k], &text, 1.8, crate::emit::readable(dir), &line_t, None);
        out.put(&line_t, [t]);
    };
    if let (Some(first), Some(last)) = (r.sections.first(), r.sections.last()) {
        label_at(first.station, format!("{} {}", tr("WS", "WL"), nap(first.level, 2)), &mut out);
        label_at(last.station, format!("{} {}", tr("WS", "WL"), nap(last.level, 2)), &mut out);
    }
    for s in &r.structures {
        let kind = match w.duikers.iter().find(|d| (d.station - s.station).abs() < 1e-9) {
            Some(d) => format!("{} {}", tr("Duiker", "Culvert"), shape_text(&d.vorm)),
            None => tr("Stuw", "Weir").to_string(),
        };
        label_at(s.station, format!("{kind} {} / {}", nap(s.level_up, 2), nap(s.level_down, 2)), &mut out);
    }
    // The water line in the length profile of the axis.
    let in_profile = frame(&doc, &al.name);
    if let Some(fr) = &in_profile {
        let pts: Vec<XY> = r.sections.iter().map(|x| fr.xy(x.station, x.level)).collect();
        out.put(&line, crate::emit::polylines(&[(pts, false)], &line.name, k, Some(&data("lengteprofiel"))));
        for s in &r.structures {
            let p = fr.xy(s.station, s.level_up.max(s.level_down));
            out.put(&line, crate::emit::polylines(&[(vec![p, [p[0], p[1] + 6.0 * k]], false)], &line.name, k, None));
            let t = ctx.text([p[0] - 1.0 * k, p[1] + 7.0 * k], &format!("{} / {}", nap(s.level_up, 2), nap(s.level_down, 2)), 1.8, std::f64::consts::FRAC_PI_2, &line_t, None);
            out.put(&line_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        }
    }
    // A run on the same axis replaces the previous one.
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(kind, key, _)| kind == "waterloop" && *key == al.name).flat_map(|(_, _, h)| h).collect();
    if !old.is_empty() {
        let _ = bridge::remove_entities(&t!("Waterloop {} (oud)", "Water profile {} (old)", name), &old);
    }
    let handles = out.write_handles(&t!("Waterloop {}", "Water profile {}", name))?;
    bridge::index_add("waterloop", &al.name, &handles, true);
    ground.note_source();

    // ── Report ──
    let label = ground.label();
    let md = report(&name, &w, &r, along, &label);
    let base = format!("waterloop-{}", slug(&name));
    let path = bridge::beside_drawing(&format!("{base}.md"));
    let _ = std::fs::write(&path, md);
    let rows: Vec<Value> = r
        .sections
        .iter()
        .map(|x| json!({"station": x.station, "peil": x.level, "diepte": x.depth, "snelheid": x.velocity, "froude": x.froude, "breedte": x.top_width, "links": axis_offset(x.left, along), "rechts": axis_offset(x.right, along), "energie": x.energy, "kritiek": x.critical, "boven_rand": x.walls}))
        .collect();
    let structs: Vec<Value> = r.structures.iter().map(|s| json!({"station": s.station, "peil_beneden": s.level_down, "peil_boven": s.level_up, "energie_boven": s.energy_up, "regime": s.regime})).collect();
    let json_path = bridge::beside_drawing(&format!("{base}.json"));
    let _ = std::fs::write(&json_path, serde_json::to_string_pretty(&json!({"as": al.name, "q": w.q, "manning_n": w.n, "stroomt_met_as": along, "invoer": p, "resultaat": r})).unwrap_or_default());
    api::note("secties", rows);
    api::note("kunstwerken", structs);
    api::note("stroomt_met_as", along);
    api::note("lengteprofiel", in_profile.is_some());
    api::note("rapport", path.to_string_lossy().into_owned());
    api::note("json", json_path.to_string_lossy().into_owned());

    let (up, down) = if along { (r.sections.first(), r.sections.last()) } else { (r.sections.last(), r.sections.first()) };
    let vmax = r.sections.iter().map(|x| x.velocity).fold(0.0, f64::max);
    let walls = r.sections.iter().filter(|x| x.walls).count();
    let crit = r.sections.iter().filter(|x| x.critical).count();
    let mut msg = t!(
        "Waterloop {}: Q {} m³/s over {} dwarsprofielen; waterstand {} bovenstrooms, {} benedenstrooms; v max {} m/s.",
        "Water profile {}: Q {} m³/s over {} cross sections; level {} upstream, {} downstream; v max {} m/s.",
        name,
        dec(w.q, 3),
        r.sections.len(),
        up.map(|x| nap(x.level, 2)).unwrap_or_default(),
        down.map(|x| nap(x.level, 2)).unwrap_or_default(),
        dec(vmax, 2)
    );
    if in_profile.is_none() {
        msg.push_str(&t!(" Geen lengteprofiel van {}: maak er een (CIV_LENGTEPROFIEL) voor de waterlijn daarin.", " No length profile of {}: make one (CIV_LENGTEPROFIEL) for the water line in it.", name));
    }
    if walls > 0 {
        msg.push_str(&t!(" Let op: bij {} dwarsprofiel(en) staat het water boven de rand: vergroot BREEDTE=.", " Note: at {} cross section(s) the water is above the edge: increase BREEDTE=.", walls));
    }
    if crit > 0 {
        msg.push_str(&t!(" {} dwarsprofiel(en) op kritieke diepte (geen stromende oplossing).", " {} cross section(s) at critical depth (no subcritical solution).", crit));
    }
    msg.push_str(&format!(" {}: {}", tr("Rapport", "Report"), path.display()));
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} != {b} (tol {tol})");
    }

    fn request() -> Waterloop {
        Waterloop { as_h: None, q: 2.0, n: 0.03, beneden: Beneden::Normaal, interval: None, breedte: None, duikers: vec![], stuwen: vec![], richting: Richting::Auto }
    }

    #[test]
    fn arguments() {
        close(parse_ruwheid("0.035").unwrap(), 0.035, 1e-12);
        close(parse_ruwheid("25").unwrap(), 0.04, 1e-12);
        assert!(parse_ruwheid("0").is_err());
        assert_eq!(parse_beneden("normaal").unwrap(), Beneden::Normaal);
        assert_eq!(parse_beneden("KRITIEK").unwrap(), Beneden::Kritiek);
        assert_eq!(parse_beneden("-0,45").unwrap(), Beneden::Peil(-0.45));
        assert!(parse_beneden("hoog").is_err());
        assert_eq!(parse_vorm("800").unwrap(), CulvertShape::Circular { diameter: 0.8 });
        assert_eq!(parse_vorm("0.6").unwrap(), CulvertShape::Circular { diameter: 0.6 });
        assert_eq!(parse_vorm("2000x1500").unwrap(), CulvertShape::Rectangular { width: 2.0, height: 1.5 });
        let d = parse_duikers("120,800,12,0.10,0.05;300,1x1,8,0,0").unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0], Duiker { station: 120.0, vorm: CulvertShape::Circular { diameter: 0.8 }, lengte: 12.0, bob_in: 0.10, bob_uit: 0.05 });
        assert!(parse_duikers("120,800,12").is_err());
        let s = parse_stuwen("250,0.8,2.5").unwrap();
        assert_eq!(s, vec![Stuw { station: 250.0, kruin: 0.8, breedte: 2.5 }]);
        let a = api::Args::parse("AS=1F Q=1,5 RUWHEID=30 BENEDENSTROOMS=0.2 DUIKER=50,600,10,0.1,0.05 STUW=80,0.5,2");
        let w = Waterloop::from_args(&a).unwrap();
        assert_eq!(w.q, 1.5);
        close(w.n, 1.0 / 30.0, 1e-12);
        assert_eq!(w.beneden, Beneden::Peil(0.2));
        assert_eq!((w.duikers.len(), w.stuwen.len()), (1, 1));
        assert!(Waterloop::from_args(&api::Args::parse("AS=1F")).is_err());
    }

    #[test]
    fn sections_around_structures() {
        let mut w = request();
        w.duikers = parse_duikers("1050,800,10,0,0").unwrap();
        // Axis from station 1000, 100 m long: the culvert covers 44–56 m with the margin.
        let fp = w.footprints(1000.0, 100.0).unwrap();
        assert_eq!(fp, vec![(44.0, 56.0)]);
        let d = section_distances(100.0, 20.0, &fp);
        assert_eq!(d, vec![0.0, 20.0, 40.0, 44.0, 56.0, 60.0, 80.0, 100.0]);
        // A section close to a footprint end goes.
        let d = section_distances(100.0, 10.0, &fp);
        assert_eq!(d, vec![0.0, 10.0, 20.0, 30.0, 40.0, 44.0, 56.0, 60.0, 70.0, 80.0, 90.0, 100.0]);
        assert_eq!(section_distances(100.0, 10.0, &[(39.5, 51.0)]), vec![0.0, 10.0, 20.0, 30.0, 39.5, 51.0, 60.0, 70.0, 80.0, 90.0, 100.0]);
        // Too close to the end, or two structures overlapping.
        w.duikers = parse_duikers("1095,800,10,0,0").unwrap();
        assert!(w.footprints(1000.0, 100.0).is_err());
        w.duikers = parse_duikers("1050,800,10,0,0").unwrap();
        w.stuwen = parse_stuwen("1055,0.5,2").unwrap();
        assert!(w.footprints(1000.0, 100.0).is_err());
    }

    #[test]
    fn offsets_follow_the_flow() {
        let samples = vec![(-2.0, Some(1.0)), (-1.0, Some(0.0)), (0.0, None), (1.0, Some(0.2)), (2.0, Some(1.5))];
        let along = section_points(&samples, true);
        assert_eq!(along, vec![[-2.0, 1.0], [-1.0, 0.0], [1.0, 0.2], [2.0, 1.5]]);
        // Against the axis: mirrored, still left to right.
        let against = section_points(&samples, false);
        assert_eq!(against, vec![[-2.0, 1.5], [-1.0, 0.2], [1.0, 0.0], [2.0, 1.0]]);
        assert_eq!(axis_offset(-1.0, false), 1.0);
        // The bed falls along the axis: the water runs along it.
        assert!(flows_along(&[(0.0, 1.0), (100.0, 0.8), (200.0, 0.55)]));
        assert!(!flows_along(&[(0.0, 0.5), (100.0, 0.8)]));
        close(bed_gradient(&[(0.0, 1.0), (100.0, 0.9), (200.0, 0.8)]).unwrap(), -0.001, 1e-12);
    }

    /// A trapezoidal ditch sampled like the ground: bottom 1 m, slopes 1:2.
    fn ditch(z: f64) -> Vec<(f64, Option<f64>)> {
        (0..=20).map(|i| {
            let o = -5.0 + i as f64 * 0.5;
            let d = (o.abs() - 0.5).max(0.0);
            (o, Some(z + (d / 2.0).min(2.0)))
        }).collect()
    }

    #[test]
    fn prismatic_ditch_runs_at_normal_depth() {
        // 500 m at 0.5 ‰, drawn downstream: the kernel's normal depth everywhere.
        let s = 0.0005;
        let secs: Vec<(f64, Vec<(f64, Option<f64>)>)> = (0..=25).map(|i| (i as f64 * 20.0, ditch(-(i as f64) * 20.0 * s))).collect();
        let w = request();
        let p = profile(&w, &secs, true).unwrap();
        assert!(matches!(p.boundary, Boundary::NormalDepth(g) if (g - s).abs() < 1e-12));
        assert!(p.stations_downstream);
        let r = kern::standard_step(&p).unwrap();
        let yn = kern::normal_level(&p.sections[0], w.q, s).unwrap() - p.sections[0].z_min();
        for x in &r.sections {
            close(x.depth, yn, 2e-3);
        }
        // Left of the flow is left of the axis.
        assert!(r.sections[0].left < 0.0 && r.sections[0].right > 0.0);
    }

    #[test]
    fn culvert_backs_the_water_up() {
        let s = 0.0005;
        let secs: Vec<(f64, Vec<(f64, Option<f64>)>)> = [0.0, 20.0, 44.0, 56.0, 80.0, 100.0].iter().map(|d| (1000.0 + d, ditch(-d * s))).collect();
        let mut w = request();
        let free = kern::standard_step(&profile(&w, &secs, true).unwrap()).unwrap();
        w.duikers = parse_duikers("1050,1000,10,-0.02,-0.03").unwrap();
        let p = profile(&w, &secs, true).unwrap();
        let r = kern::standard_step(&p).unwrap();
        assert_eq!(r.structures.len(), 1);
        // Upstream of the culvert the water stands higher than without it.
        assert!(r.sections[0].level > free.sections[0].level + 0.01);
        close(r.sections[5].level, free.sections[5].level, 1e-9);
    }

    #[test]
    fn flat_bed_needs_a_level() {
        let secs: Vec<(f64, Vec<(f64, Option<f64>)>)> = (0..5).map(|i| (i as f64 * 10.0, ditch(0.0))).collect();
        let mut w = request();
        assert!(profile(&w, &secs, true).is_err());
        w.beneden = Beneden::Peil(1.0);
        assert_eq!(profile(&w, &secs, true).unwrap().boundary, Boundary::Level(1.0));
        let few = vec![(0.0, vec![(0.0, Some(0.0)), (1.0, None)])];
        assert!(profile(&w, &few, true).is_err());
    }

    #[test]
    fn profile_frame_mapping() {
        let k = Kader { x_nul: 100.0, y_basis: 50.0, basis: -2.0, overhoging: 10.0, eenheid: 1.0, beginstation: 1000.0 };
        assert_eq!(k.xy(1000.0, -2.0), [100.0, 50.0]);
        assert_eq!(k.xy(1020.0, -1.0), [120.0, 60.0]);
    }
}
