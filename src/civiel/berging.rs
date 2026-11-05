//! `CIV_BERGING` (issue #64): a storage facility — wadi, pond or crate
//! system — from a closed polyline (the bottom outline; for crates the
//! footprint). It draws the bottom and top outlines at their levels with the
//! side slopes, computes the stage–storage curve (`rekenkern::berging`),
//! routes a design storm (`rekenkern::buien`) through it with an orifice,
//! a weir and infiltration, and draws a level–time graph beside it. With
//! `TIN=ja` the design TIN is cut to the basin the way `CIV_PLATEAU` does.
//!
//! The method, defaults and limitations are in `docs/civiel-water.md`.

use ocs_plugin_api::host::codec::entities::{Polyline3D, TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{EntityType, Handle, Line};
use rekenkern::berging::{self as kern, InfiltrationArea, Outlet, RoutingOptions, RoutingResult, StageStorage};
use rekenkern::buien::{self, CsvValue, Hyetograph, StresstestStorm};
use serde_json::{json, Value};

use super::{api, nice_step, record, v3, Ctx, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::{Element, LayerSpec, Status};

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

/// Side slope 1:n of a wadi or pond when none is given: 1:3, a common choice
/// for slopes that are mown and safe to walk; not from a norm.
pub const TALUD: f64 = 3.0;
/// Routing time step (s): the 5-minute blocks of the Leidraad storms in five.
pub const DT: f64 = 60.0;
/// Graph size in paper millimetres.
const GRAFIEK_B: f64 = 100.0;
const GRAFIEK_H: f64 = 50.0;
/// At most this many points on the graph line.
const GRAFIEK_PUNTEN: usize = 400;

pub const ARGS: &[&str] = &["LIJN", "SOORT", "BODEM", "DIEPTE", "TALUD", "KNIJP", "DREMPEL", "INFILTRATIE", "BUI", "OPPERVLAK", "NAAM", "TIN", "PLAATS"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Soort {
    Wadi,
    Vijver,
    Krat,
}

impl Soort {
    pub fn parse(s: &str) -> Result<Soort, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "wadi" => Ok(Soort::Wadi),
            "vijver" | "pond" => Ok(Soort::Vijver),
            "krat" | "kratten" | "crate" | "crates" => Ok(Soort::Krat),
            _ => Err(tr("SOORT: wadi, vijver of krat", "SOORT: wadi, vijver (pond) or krat (crates)").into()),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Soort::Wadi => "wadi",
            Soort::Vijver => "vijver",
            Soort::Krat => "krat",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Soort::Wadi => "Wadi",
            Soort::Vijver => tr("Vijver", "Pond"),
            Soort::Krat => tr("Kratten", "Crates"),
        }
    }

    /// Open water: side slopes, rain falls on it.
    pub fn open(self) -> bool {
        self != Soort::Krat
    }

    /// The NLCS object (RI, infiltration and storage facilities).
    pub fn object(self) -> &'static str {
        match self {
            Soort::Wadi => "ITR_RESERVOIR_INFILTRATIERESERVOIR_WADI",
            Soort::Vijver => "ITR_RESERVOIR_BERGINGSVIJVER",
            Soort::Krat => "ITR_RESERVOIR_INFILTRATIERESERVOIR",
        }
    }
}

/// The design storm.
#[derive(Clone, Debug, PartialEq)]
pub enum Bui {
    /// Leidraad Riolering standard storm 01–10.
    Leidraad(u8),
    /// A DPRA stress test event.
    Stresstest(StresstestStorm),
    /// A uniform block: mm in minutes.
    Blok { mm: f64, min: f64 },
    /// A "minutes,value" text file.
    Bestand(String),
}

/// BUI=01…10 | T100 | T250 | T1000 | 48H | R100 | R250 | R1000 | <mm>:<min> | <path>.
pub fn parse_bui(s: &str) -> Result<Bui, String> {
    let t = s.trim();
    let up = t.to_ascii_uppercase();
    if !up.is_empty() && up.len() <= 2 && up.chars().all(|c| c.is_ascii_digit()) {
        let n: u8 = up.parse().unwrap_or(0);
        return if (1..=10).contains(&n) { Ok(Bui::Leidraad(n)) } else { Err(tr("BUI: Leidraad-bui 01 t/m 10", "BUI: Leidraad storm 01 to 10").into()) };
    }
    let st = match up.as_str() {
        "T100" => Some(StresstestStorm::Local1hT100),
        "T250" => Some(StresstestStorm::Local1hT250),
        "T1000" => Some(StresstestStorm::Local2hT1000),
        "48H" | "T48H" => Some(StresstestStorm::Local48h),
        "R100" => Some(StresstestStorm::Regional48hT100),
        "R250" => Some(StresstestStorm::Regional48hT250),
        "R1000" => Some(StresstestStorm::Regional48hT1000),
        _ => None,
    };
    if let Some(st) = st {
        return Ok(Bui::Stresstest(st));
    }
    if let Some((a, b)) = t.split_once(':') {
        if let (Ok(mm), Ok(min)) = (a.trim().replace(',', ".").parse::<f64>(), b.trim().replace(',', ".").parse::<f64>()) {
            if mm < 0.0 || min <= 0.0 {
                return Err(tr("BUI: <mm>:<minuten> met mm ≥ 0 en minuten > 0", "BUI: <mm>:<minutes> with mm ≥ 0 and minutes > 0").into());
            }
            return Ok(Bui::Blok { mm, min });
        }
    }
    if t.is_empty() {
        return Err(tr("BUI: geef een bui", "BUI: give a storm").into());
    }
    Ok(Bui::Bestand(t.to_string()))
}

/// A storm from "minutes,value" text: mm/h, unless the header before the
/// first data line names mm per block ("mm" without "/h").
pub fn storm_from_text(name: &str, text: &str) -> Result<Hyetograph, String> {
    let header = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take_while(|l| !l.starts_with(|c: char| c.is_ascii_digit() || c == '.' || c == '-'))
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    let per_hour = header.contains("mm/h") || header.contains("mm/u") || header.contains("mm per uur") || header.contains("mm per hour");
    let unit = if header.contains("mm") && !per_hour { CsvValue::DepthMm } else { CsvValue::IntensityMmH };
    Hyetograph::from_csv(name, text, unit)
}

impl Bui {
    pub fn storm(&self) -> Result<Hyetograph, String> {
        match self {
            Bui::Leidraad(n) => buien::leidraad_bui(*n),
            Bui::Stresstest(s) => Ok(s.hyetograph()),
            Bui::Blok { mm, min } => buien::uniform_block(*mm, *min),
            Bui::Bestand(p) => {
                let text = std::fs::read_to_string(p).map_err(|e| t!("BUI: {} niet te lezen: {}", "BUI: cannot read {}: {}", p, e))?;
                let name = std::path::Path::new(p).file_stem().and_then(|s| s.to_str()).unwrap_or("bui").to_string();
                storm_from_text(&name, &text)
            }
        }
    }
}

/// What `CIV_BERGING` builds and computes.
#[derive(Clone, Debug)]
pub struct Berging {
    pub lijn: Option<Handle>,
    pub soort: Soort,
    /// Bottom level (m).
    pub bodem: f64,
    /// Depth from the bottom to the top (m).
    pub diepte: f64,
    /// Side slope 1:n (0 for crates).
    pub talud: f64,
    /// Orifice: diameter (m) and invert (m; the bottom if none).
    pub knijp: Option<(f64, Option<f64>)>,
    /// Weir: crest level (m) and width (m).
    pub drempel: Option<(f64, f64)>,
    /// Infiltration rate (mm/h) over the wetted area.
    pub infiltratie: Option<f64>,
    pub bui: Bui,
    /// Connected paved area (m²).
    pub oppervlak: Option<f64>,
    pub naam: Option<String>,
    /// Cut the design TIN to the basin (as CIV_PLATEAU).
    pub tin: bool,
    /// Lower-left corner of the graph (drawing).
    pub plaats: Option<XY>,
}

/// "a,b": commas separate the values, decimals take a point.
fn pair(s: &str, what: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|p| p.trim().parse::<f64>().map_err(|_| t!("{}: geen getal '{}' (punt als decimaalteken)", "{}: not a number '{}' (decimal point)", what, p.trim()))).collect()
}

impl Berging {
    pub fn from_args(a: &api::Args) -> Result<Berging, String> {
        let soort = Soort::parse(a.str("SOORT").ok_or_else(|| tr("geef SOORT=wadi|vijver|krat", "give SOORT=wadi|vijver (pond)|krat (crates)").to_string())?)?;
        let bodem = a.f64("BODEM")?.ok_or_else(|| tr("geef het bodempeil: BODEM=<m NAP>", "give the bottom level: BODEM=<m>").to_string())?;
        let diepte = a.f64("DIEPTE")?.ok_or_else(|| tr("geef de diepte: DIEPTE=<m>", "give the depth: DIEPTE=<m>").to_string())?;
        if diepte <= 0.0 {
            return Err(tr("DIEPTE moet groter dan 0 zijn", "DIEPTE must be above 0").into());
        }
        let talud = match (a.str("TALUD"), soort) {
            (_, Soort::Krat) => 0.0,
            (Some(t), _) => super::grading::parse_ratio(t)?,
            (None, _) => TALUD,
        };
        let knijp = match a.str("KNIJP") {
            None => None,
            Some(s) => {
                let v = pair(s, "KNIJP")?;
                let d = super::waterloop::metres(*v.first().ok_or("KNIJP")?);
                if d <= 0.0 || v.len() > 2 {
                    return Err(tr("KNIJP=<diameter m of mm>[,<bob m>]", "KNIJP=<diameter m or mm>[,<invert m>]").into());
                }
                Some((d, v.get(1).copied()))
            }
        };
        let drempel = match a.str("DREMPEL") {
            None => None,
            Some(s) => match pair(s, "DREMPEL")?.as_slice() {
                [z, b] if *b > 0.0 => Some((*z, *b)),
                _ => return Err(tr("DREMPEL=<peil m>,<breedte m>", "DREMPEL=<crest level m>,<width m>").into()),
            },
        };
        let infiltratie = a.f64("INFILTRATIE")?;
        if infiltratie.is_some_and(|v| v < 0.0) {
            return Err(tr("INFILTRATIE: mm/h, niet negatief", "INFILTRATIE: mm/h, not negative").into());
        }
        let oppervlak = a.str("OPPERVLAK").map(super::riool::parse_oppervlak).transpose()?;
        if oppervlak.is_some_and(|v| v < 0.0) {
            return Err(tr("OPPERVLAK: niet negatief", "OPPERVLAK: not negative").into());
        }
        Ok(Berging {
            lijn: a.handle("LIJN")?,
            soort,
            bodem,
            diepte,
            talud,
            knijp,
            drempel,
            infiltratie,
            bui: a.str("BUI").map(parse_bui).transpose()?.unwrap_or(Bui::Leidraad(8)),
            oppervlak,
            naam: a.str("NAAM").map(str::to_string),
            tin: matches!(a.str("TIN").map(|v| v.to_ascii_lowercase()).as_deref(), Some("ja" | "yes" | "1" | "true" | "aan")),
            plaats: None,
        })
    }

    pub fn top(&self) -> f64 {
        self.bodem + self.diepte
    }

    /// The stage–storage curve of the outline (metres).
    pub fn curve(&self, outline: &[XY]) -> Result<StageStorage, String> {
        match self.soort {
            Soort::Krat => StageStorage::crates(self.bodem, polygon_area(outline), self.diepte, kern::CRATE_POROSITY),
            _ => StageStorage::basin(outline, self.bodem, self.diepte, self.talud),
        }
    }

    /// The outlets: orifice (at the bottom unless its invert is given),
    /// weir, infiltration over the wetted area.
    pub fn outlets(&self) -> Vec<Outlet> {
        let mut o = Vec::new();
        if let Some((d, inv)) = self.knijp {
            o.push(Outlet::orifice(d, inv.unwrap_or(self.bodem)));
        }
        if let Some((z, b)) = self.drempel {
            o.push(Outlet::weir(z, b));
        }
        if let Some(r) = self.infiltratie.filter(|r| *r > 0.0) {
            o.push(Outlet::infiltration(r, InfiltrationArea::Wetted));
        }
        o
    }

    /// The area that drains into the facility (m²): the connected area,
    /// plus the open water itself.
    pub fn inflow_area(&self, curve: &StageStorage) -> f64 {
        self.oppervlak.unwrap_or(0.0) + if self.soort.open() { *curve.areas.last().unwrap_or(&0.0) } else { 0.0 }
    }

    /// The storm routed through the facility.
    pub fn route(&self, curve: &StageStorage, storm: &Hyetograph) -> Result<RoutingResult, String> {
        let inflow = kern::inflow_from_storm(storm, self.inflow_area(curve), 1.0, DT)?;
        kern::route(curve, &self.outlets(), &inflow, &RoutingOptions::default())
    }
}

// ── Geometry ────────────────────────────────────────────────────────────────

/// Signed area (counter-clockwise > 0).
fn signed_area(p: &[XY]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f64>() / 2.0
}

pub fn polygon_area(p: &[XY]) -> f64 {
    signed_area(p).abs()
}

/// The polygon moved `d` outward, corners mitred (the top of a basin with
/// side slopes, as `StageStorage::basin` assumes).
pub fn offset_polygon(p: &[XY], d: f64) -> Vec<XY> {
    let n = p.len();
    let s = if signed_area(p) >= 0.0 { 1.0 } else { -1.0 };
    // Outward normal of edge i → i+1: right of a counter-clockwise ring.
    let normal = |i: usize| {
        let (a, b) = (p[i], p[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-12);
        [s * dy / l, -s * dx / l]
    };
    (0..n)
        .map(|i| {
            let (n1, n2) = (normal((i + n - 1) % n), normal(i));
            let c = 1.0 + n1[0] * n2[0] + n1[1] * n2[1];
            let m = if c.abs() < 1e-9 { n2 } else { [(n1[0] + n2[0]) / c, (n1[1] + n2[1]) / c] };
            [p[i][0] + d * m[0], p[i][1] + d * m[1]]
        })
        .collect()
}

/// Slope hatching (NLCS *taludarcering*) between a top outline and the
/// bottom `r` inside it: ticks every `spacing` from the top edge inward,
/// alternately the full width (long) and half (short).
pub fn talud_ticks(top: &[XY], r: f64, spacing: f64) -> Vec<(XY, XY, bool)> {
    let n = top.len();
    let s = if signed_area(top) >= 0.0 { 1.0 } else { -1.0 };
    let mut out = Vec::new();
    if r <= 0.0 || spacing <= 0.0 {
        return out;
    }
    for i in 0..n {
        let (a, b) = (top[i], top[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        if l < 2.0 * spacing {
            continue;
        }
        let u = [dx / l, dy / l];
        let inward = [-s * u[1], s * u[0]];
        let count = (l / spacing).floor() as usize;
        let first = (l - (count - 1) as f64 * spacing) / 2.0;
        for j in 0..count {
            let at = first + j as f64 * spacing;
            // Keep clear of the corner facets (r wide at a right angle).
            if at < r || at > l - r {
                continue;
            }
            let long = j % 2 == 0;
            let p = [a[0] + u[0] * at, a[1] + u[1] * at];
            let len = if long { r } else { r / 2.0 };
            out.push((p, [p[0] + inward[0] * len, p[1] + inward[1] * len], long));
        }
    }
    out
}

/// A level–time graph in paper millimetres: the level line, the axes'
/// ranges and steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Grafiek {
    /// (x, y) in paper mm from the lower-left corner.
    pub lijn: Vec<XY>,
    pub t_max_h: f64,
    pub t_stap_h: f64,
    pub z_min: f64,
    pub z_max: f64,
    pub z_stap: f64,
}

impl Grafiek {
    pub fn of(res: &RoutingResult, bodem: f64, top: f64, drempel: Option<f64>, storm_s: f64) -> Grafiek {
        let s = &res.summary;
        let last = res.time_s.last().copied().unwrap_or(0.0);
        let end = match s.drain_time_s {
            Some(d) => (s.time_of_max_s + 1.2 * d).min(last),
            None => last,
        }
        .max(storm_s)
        .max(DT);
        let t_stap_h = nice_step(end / 3600.0 / 6.0);
        let t_max_h = (end / 3600.0 / t_stap_h).ceil() * t_stap_h;
        let hi = s.max_level.max(top).max(drempel.unwrap_or(f64::MIN));
        let z_stap = nice_step(((hi - bodem) / 5.0).max(0.01));
        let z_min = (bodem / z_stap).floor() * z_stap;
        let z_max = (hi / z_stap).ceil() * z_stap + if (hi / z_stap).fract().abs() < 1e-9 { z_stap } else { 0.0 };
        let g = Grafiek { lijn: Vec::new(), t_max_h, t_stap_h, z_min, z_max, z_stap };
        let inside: Vec<usize> = (0..res.time_s.len()).filter(|&i| res.time_s[i] / 3600.0 <= t_max_h + 1e-9).collect();
        let stride = inside.len().div_ceil(GRAFIEK_PUNTEN).max(1);
        let mut lijn: Vec<XY> = inside.iter().step_by(stride).map(|&i| g.xy(res.time_s[i] / 3600.0, res.level[i])).collect();
        if let Some(&i) = inside.last() {
            let p = g.xy(res.time_s[i] / 3600.0, res.level[i]);
            if lijn.last() != Some(&p) {
                lijn.push(p);
            }
        }
        Grafiek { lijn, ..g }
    }

    pub fn xy(&self, t_h: f64, z: f64) -> XY {
        [t_h / self.t_max_h * GRAFIEK_B, (z - self.z_min) / (self.z_max - self.z_min) * GRAFIEK_H]
    }
}

// ── In the drawing ──────────────────────────────────────────────────────────

fn dec(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

fn poly3(pts: &[XY], z: f64, closed: bool, spec: &LayerSpec, k: f64, data: Option<&ocs_plugin_api::host::ExtendedDataRecord>) -> EntityType {
    let mut p = Polyline3D::from_points(pts.iter().map(|p| v3(*p, z)).collect());
    p.flags.closed = closed;
    let mut e = EntityType::Polyline3D(p);
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    c.linetype_scale = k;
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

fn line3(a: XY, za: f64, b: XY, zb: f64, spec: &LayerSpec) -> EntityType {
    let mut e = EntityType::Line(Line::from_points(v3(a, za), v3(b, zb)));
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    e
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
    s.trim_matches('-').to_string()
}

/// "Wadi 3": one more than the highest number of this kind in the index.
fn next_name(prefix: &str) -> String {
    let n = bridge::index_read()
        .into_iter()
        .filter(|(k, _, _)| k == "berging")
        .filter_map(|(_, key, _)| key.strip_prefix(prefix).and_then(|n| n.trim().parse::<usize>().ok()))
        .max()
        .unwrap_or(0);
    format!("{prefix} {}", n + 1)
}

fn hours(s: f64) -> String {
    dec(s / 3600.0, 1)
}

/// The report as Markdown.
fn report(name: &str, b: &Berging, curve: &StageStorage, storm: &Hyetograph, res: &RoutingResult) -> String {
    let s = &res.summary;
    let mut r = format!("# {name}\n\n");
    r.push_str(&t!(
        "{}: bodem {}, diepte {} m, talud 1:{}; berging tot de rand {} m³ ({} m² aan de rand{}).\n\n",
        "{}: bottom {}, depth {} m, side slope 1:{}; storage to the top {} m³ ({} m² at the top{}).\n\n",
        b.soort.label(),
        nap(b.bodem, 2),
        dec(b.diepte, 2),
        dec(b.talud, 1),
        dec(curve.max_storage(), 1),
        dec(*curve.areas.last().unwrap_or(&0.0), 1),
        if b.soort == Soort::Krat { t!(", poriëngehalte {}", ", void ratio {}", curve.porosity) } else { String::new() }
    ));
    r.push_str(&t!(
        "Bui: {} ({} mm in {} min, piek {} mm/h) op {} m² (aangesloten {} m²{}), afvoercoëfficiënt 1.\n\n",
        "Storm: {} ({} mm in {} min, peak {} mm/h) on {} m² (connected {} m²{}), runoff coefficient 1.\n\n",
        storm.name,
        dec(storm.total_depth_mm(), 1),
        dec(storm.duration_s() / 60.0, 0),
        dec(storm.peak_intensity_mm_h(), 1),
        dec(b.inflow_area(curve), 0),
        dec(b.oppervlak.unwrap_or(0.0), 0),
        if b.soort.open() { tr(" plus het open water", " plus the open water") } else { "" }
    ));
    r.push_str(&format!("| {} | |\n|---|---|\n", tr("Uitkomst", "Result")));
    let rows = [
        (tr("Hoogste peil", "Highest level"), format!("{} ({} m {})", nap(s.max_level, 3), dec(s.max_level - b.bodem, 3), tr("boven de bodem", "above the bottom"))),
        (tr("Hoogste berging", "Highest storage"), format!("{} m³ ({} %)", dec(s.max_storage, 1), dec(100.0 * s.max_storage / curve.max_storage().max(1e-9), 0))),
        (tr("Tijdstip", "Time"), format!("{} h", hours(s.time_of_max_s))),
        (tr("Instroom", "Inflow"), format!("{} m³", dec(s.inflow_volume, 1))),
        (tr("Over de drempel", "Over the weir"), format!("{} m³", dec(s.overflow_volume, 1))),
        (tr("Boven de rand", "Above the top"), if s.above_top { tr("ja: de berging loopt over", "yes: the facility overtops").to_string() } else { tr("nee", "no").to_string() }),
        (tr("Ledigingstijd (tot 1 %)", "Drain time (to 1 %)"), s.drain_time_s.map(|d| format!("{} h", hours(d))).unwrap_or_else(|| tr("niet leeg binnen 7 dagen", "not empty within 7 days").to_string())),
        (tr("Massabalans", "Mass balance"), format!("{:.1e}", s.mass_balance_error)),
    ];
    for (k, v) in rows {
        r.push_str(&format!("| {k} | {v} |\n"));
    }
    let names: Vec<String> = b
        .outlets()
        .iter()
        .map(|o| match o {
            Outlet::Orifice { diameter, invert, .. } => t!("knijp Ø{} mm op {}", "orifice Ø{} mm at {}", dec(diameter * 1000.0, 0), nap(*invert, 2)),
            Outlet::Weir { crest, width, .. } => t!("drempel {} m op {}", "weir {} m at {}", dec(*width, 2), nap(*crest, 2)),
            Outlet::Infiltration { rate_mm_h, .. } => t!("infiltratie {} mm/h", "infiltration {} mm/h", dec(*rate_mm_h, 1)),
        })
        .collect();
    if !names.is_empty() {
        r.push_str(&format!("\n| {} | m³ |\n|---|---|\n", tr("Uitlaat", "Outlet")));
        for (n, v) in names.iter().zip(&s.outlet_volumes) {
            r.push_str(&format!("| {n} | {} |\n", dec(*v, 1)));
        }
    }
    r.push_str(&format!("\n## {}\n\n| {} | {} | {} |\n|---|---|---|\n", tr("Berging-peilkromme", "Stage–storage curve"), tr("Peil (m)", "Level (m)"), tr("Oppervlak (m²)", "Area (m²)"), tr("Berging (m³)", "Storage (m³)")));
    let n = curve.levels.len();
    for i in (0..n).step_by((n / 10).max(1)).chain(std::iter::once(n - 1)).collect::<std::collections::BTreeSet<_>>() {
        r.push_str(&format!("| {} | {} | {} |\n", nap(curve.levels[i], 3), dec(curve.areas[i], 1), dec(curve.porosity * curve.volumes[i], 1)));
    }
    r.push_str(&t!(
        "\nStorage-indicationmethode (gemodificeerde Puls; Chow, Maidment & Mays, Applied Hydrology, 1988, §8.2), tijdstap {} s. Knijp μ = {}, drempel m = {} (Leidraad Riolering C2100, bijlage 2). Zie docs/civiel-water.md.\n",
        "\nStorage-indication method (modified Puls; Chow, Maidment & Mays, Applied Hydrology, 1988, §8.2), time step {} s. Orifice μ = {}, weir m = {} (Leidraad Riolering C2100, appendix 2). See docs/civiel-water.md.\n",
        DT,
        kern::ORIFICE_CD,
        kern::WEIR_M
    ));
    r
}

/// `CIV_BERGING`: the facility from a closed polyline, its curve, the storm
/// routed through it, the drawing and the graph.
pub fn berging(opt: &super::CivielOptions, b: Berging, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let h = match b.lijn {
        Some(h) => h,
        None => {
            let sel = bridge::selection()?;
            match sel.len() {
                1 => sel[0],
                0 => return Err(tr("selecteer eerst de gesloten bodemomtrek, of geef LIJN=<handle>", "select the closed bottom outline first, or give LIJN=<handle>").into()),
                n => return Err(t!("{} elementen geselecteerd: selecteer er één", "{} entities selected: select one", n)),
            }
        }
    };
    let e = bridge::entities(&[h])?.into_iter().next().flatten().ok_or_else(|| t!("geen element met handle {}", "no entity with handle {}", api::handle_text(h)))?;
    let line = super::grading::line_of(&e).ok_or_else(|| t!("{} is geen polylijn", "{} is not a polyline", api::handle_text(h)))?;
    if !line.closed || line.pts.len() < 3 {
        return Err(tr("een berging heeft een gesloten omtrek nodig: sluit de polylijn", "a storage facility needs a closed outline: close the polyline").into());
    }
    let xy: Vec<XY> = line.pts.iter().map(|p| [p[0], p[1]]).collect();
    let outline_m: Vec<XY> = xy.iter().map(|p| ctx.from_drawing(*p)).collect();
    let curve = b.curve(&outline_m)?;
    let storm = b.bui.storm()?;
    progress(t!("{} door de berging …", "Routing {} …", storm.name));
    let res = b.route(&curve, &storm)?;
    let s = res.summary.clone();
    let name = b.naam.clone().unwrap_or_else(|| next_name(b.soort.label()));

    // ── Drawing ──
    let spec = ctx.layer("RI", b.soort.object(), Element::G)?;
    let spec_t = ctx.layer("RI", b.soort.object(), Element::T)?;
    let water = ctx.layer("GW", "WATERGANG_WATERLIJN", Element::G)?;
    let lang = ctx.layer("GW", "TALUDARCERING_LANG", Element::G)?;
    let kort = ctx.layer("GW", "TALUDARCERING_KORT", Element::G)?;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let data = record(&[
        ("berging", name.clone()),
        ("soort", b.soort.code().into()),
        ("bodem", format!("{:.3}", b.bodem)),
        ("diepte", format!("{:.3}", b.diepte)),
        ("talud", format!("1:{}", b.talud)),
        ("berging_m3", format!("{:.1}", curve.max_storage())),
        ("max_peil", format!("{:.3}", s.max_level)),
    ]);
    let mut out = Out::default();
    let (zb, zt) = (b.bodem * f, b.top() * f);
    out.put(&spec, [poly3(&xy, zb, true, &spec, k, Some(&data))]);
    let r = b.talud * b.diepte * f;
    let top = if b.soort.open() && r > 0.0 { offset_polygon(&xy, r) } else { xy.clone() };
    out.put(&spec, [poly3(&top, zt, true, &spec, k, None)]);
    if b.soort.open() && r > 0.0 {
        for (p, q) in xy.iter().zip(&top) {
            out.put(&spec, [line3(*p, zb, *q, zt, &spec)]);
        }
        for (p, q, long) in talud_ticks(&top, r, 4.0 * k) {
            let sp = if long { &lang } else { &kort };
            out.put(sp, [line3(p, zt, q, zt - (if long { 1.0 } else { 0.5 }) * b.diepte * f, sp)]);
        }
        // The waterline at the highest level.
        let dz = (s.max_level - b.bodem).clamp(0.0, b.diepte);
        if dz > 1e-3 {
            out.put(&water, [poly3(&offset_polygon(&xy, b.talud * dz * f), s.max_level * f, true, &water, k, None)]);
        }
    }
    let (_, centre) = super::terrein::area_centroid(&xy);
    let label = format!("{name} · {} {} · {} m³", tr("bodem", "bottom"), nap(b.bodem, 2), dec(curve.max_storage(), 0));
    let t = ctx.text([centre[0], centre[1] + 1.5 * k], &label, 2.5, 0.0, &spec_t, Some(&data));
    out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
    let t = ctx.text([centre[0], centre[1] - 1.5 * k], &format!("{} {} ({})", tr("max. peil", "max. level"), nap(s.max_level, 2), storm.name), 1.8, 0.0, &spec_t, None);
    out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Top)]);

    // The graph, right of the facility.
    let crest = b.drempel.map(|d| d.0);
    let g = Grafiek::of(&res, b.bodem, b.top(), crest, storm.duration_s());
    let bb = top.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].max(p[0]), a[3].max(p[1])]);
    let o = b.plaats.unwrap_or([bb[2] + 25.0 * k, bb[1]]);
    let at = |p: XY| [o[0] + p[0] * k, o[1] + p[1] * k];
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    out.put(&frame, crate::emit::polylines(&[(vec![at([0.0, 0.0]), at([GRAFIEK_B, 0.0]), at([GRAFIEK_B, GRAFIEK_H]), at([0.0, GRAFIEK_H])], true)], &frame.name, k, None));
    let mut th = 0.0;
    while th <= g.t_max_h + 1e-9 {
        let x = g.xy(th, g.z_min)[0];
        if th > 1e-9 && th < g.t_max_h - 1e-9 {
            out.put(&frame, pl(vec![at([x, 0.0]), at([x, GRAFIEK_H])], &frame));
        }
        let t = ctx.text(at([x, -2.5]), &dec(th, if g.t_stap_h < 1.0 { 1 } else { 0 }), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Top)]);
        th += g.t_stap_h;
    }
    let mut z = g.z_min;
    while z <= g.z_max + 1e-9 {
        let y = g.xy(0.0, z)[1];
        if z > g.z_min + 1e-9 && z < g.z_max - 1e-9 {
            out.put(&frame, pl(vec![at([0.0, y]), at([GRAFIEK_B, y])], &frame));
        }
        let t = ctx.text(at([-2.0, y]), &nap(z, 2), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        z += g.z_stap;
    }
    for (zl, what) in [(Some(b.top()), tr("rand", "top")), (crest, tr("drempel", "weir"))] {
        if let Some(zl) = zl {
            let y = g.xy(0.0, zl)[1];
            out.put(&spec, pl(vec![at([0.0, y]), at([GRAFIEK_B, y])], &spec));
            let t = ctx.text(at([GRAFIEK_B - 1.0, y + 1.0]), &format!("{what} {}", nap(zl, 2)), 1.8, 0.0, &spec_t, None);
            out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Bottom)]);
        }
    }
    out.put(&water, pl(g.lijn.iter().map(|p| at(*p)).collect(), &water));
    let t = ctx.text(at([GRAFIEK_B / 2.0, -7.0]), tr("tijd (h)", "time (h)"), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Top)]);
    let t = ctx.text(at([0.0, GRAFIEK_H + 4.0]), &t!("{}: waterstand bij {}", "{}: water level for {}", name, storm.name), 2.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Bottom)]);

    // Again under the same name: the old drawing goes.
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(kind, key, _)| kind == "berging" && *key == name).flat_map(|(_, _, h)| h).collect();
    if !old.is_empty() {
        let _ = bridge::remove_entities(&t!("{} (oud)", "{} (old)", name), &old);
    }
    let handles = out.write_handles(&name)?;
    bridge::index_add("berging", &name, &handles, true);

    // The design TIN, as CIV_PLATEAU: the bottom at its level, the slopes to the ground.
    let mut notes = Vec::new();
    if b.tin {
        if !b.soort.open() || b.talud <= 0.0 {
            notes.push(tr("TIN=ja geldt voor een wadi of vijver met een talud", "TIN=ja applies to a wadi or pond with a side slope").to_string());
        } else {
            let mut o = opt.clone();
            o.nlcs.status = Status::N;
            let req = super::grading::PlateauRequest {
                source: super::grading::Source::Handle(h),
                level: Some(b.bodem),
                default_level: None,
                fall: None,
                reference: None,
                cut: b.talud,
                fill: b.talud,
                reach: 50.0,
                step: 1.0,
                name: Some(format!("{name} TIN")),
                join: true,
            };
            match super::grading::plateau(&o, req, progress) {
                Ok(m) => notes.push(m),
                Err(e) => notes.push(t!("ontwerp-TIN niet aangepast: {}", "design TIN not changed: {}", e)),
            }
        }
    }

    // ── Report ──
    let base = format!("berging-{}", slug(&name));
    let path = bridge::beside_drawing(&format!("{base}.md"));
    let _ = std::fs::write(&path, report(&name, &b, &curve, &storm, &res));
    let every = ((300.0 / DT).round() as usize).max(1);
    let reeks: Vec<Value> = (0..res.time_s.len())
        .step_by(every)
        .map(|i| json!({"tijd_s": res.time_s[i], "peil": res.level[i], "berging": res.storage[i], "instroom": res.inflow[i], "uitstroom": res.outflow.iter().map(|o| o[i]).sum::<f64>()}))
        .collect();
    let kromme = json!({"peil": curve.levels, "oppervlak": curve.areas, "berging": curve.volumes.iter().map(|v| v * curve.porosity).collect::<Vec<_>>(), "porositeit": curve.porosity});
    let summary = json!({
        "naam": name, "soort": b.soort.code(), "bodem": b.bodem, "diepte": b.diepte, "talud": b.talud,
        "berging_m3": curve.max_storage(), "bui": storm.name, "bui_mm": storm.total_depth_mm(), "afvoerend_m2": b.inflow_area(&curve),
        "max_peil": s.max_level, "max_berging_m3": s.max_storage, "tijd_max_s": s.time_of_max_s, "instroom_m3": s.inflow_volume,
        "uitlaat_m3": s.outlet_volumes, "overloop": s.overflows, "overloop_m3": s.overflow_volume, "boven_rand": s.above_top,
        "ledigingstijd_s": s.drain_time_s, "massabalans": s.mass_balance_error,
    });
    let json_path = bridge::beside_drawing(&format!("{base}.json"));
    let _ = std::fs::write(&json_path, serde_json::to_string_pretty(&json!({"berging": summary, "kromme": kromme, "reeks": reeks})).unwrap_or_default());
    api::note("berging", summary);
    api::note("kromme", kromme);
    api::note("reeks", reeks);
    api::note("rapport", path.to_string_lossy().into_owned());
    api::note("json", json_path.to_string_lossy().into_owned());

    let mut msg = t!(
        "{}: {} m³ tot de rand ({} {}, diepte {} m); {} ({} mm) op {} m²: max. peil {} ({} % vol), {}, {}.",
        "{}: {} m³ to the top ({} {}, depth {} m); {} ({} mm) on {} m²: max. level {} ({} % full), {}, {}.",
        name,
        dec(curve.max_storage(), 1),
        tr("bodem", "bottom"),
        nap(b.bodem, 2),
        dec(b.diepte, 2),
        storm.name,
        dec(storm.total_depth_mm(), 1),
        dec(b.inflow_area(&curve), 0),
        nap(s.max_level, 2),
        dec(100.0 * s.max_storage / curve.max_storage().max(1e-9), 0),
        if s.above_top {
            tr("loopt over de rand", "overtops").to_string()
        } else if s.overflows {
            t!("{} m³ over de drempel", "{} m³ over the weir", dec(s.overflow_volume, 1))
        } else {
            tr("geen overloop", "no overflow").to_string()
        },
        match s.drain_time_s {
            Some(d) => t!("leeg na {} h", "empty after {} h", hours(d)),
            None => tr("niet leeg binnen 7 dagen", "not empty within 7 days").to_string(),
        }
    );
    if b.oppervlak.is_none() {
        msg.push_str(&format!(" {}", tr("Geen OPPERVLAK=: alleen de regen op de berging zelf.", "No OPPERVLAK=: only the rain on the facility itself.")));
    }
    if b.outlets().is_empty() {
        msg.push_str(&format!(" {}", tr("Geen uitlaat (KNIJP=, DREMPEL=, INFILTRATIE=): de berging loopt niet leeg.", "No outlet (KNIJP=, DREMPEL=, INFILTRATIE=): the facility does not drain.")));
    }
    for n in notes {
        msg.push(' ');
        msg.push_str(&n);
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

    fn wadi() -> Berging {
        Berging { lijn: None, soort: Soort::Wadi, bodem: 1.0, diepte: 0.5, talud: 3.0, knijp: None, drempel: None, infiltratie: None, bui: Bui::Leidraad(8), oppervlak: Some(1000.0), naam: None, tin: false, plaats: None }
    }

    #[test]
    fn arguments() {
        assert_eq!(parse_bui("08").unwrap(), Bui::Leidraad(8));
        assert_eq!(parse_bui("8").unwrap(), Bui::Leidraad(8));
        assert!(parse_bui("11").is_err());
        assert_eq!(parse_bui("t100").unwrap(), Bui::Stresstest(StresstestStorm::Local1hT100));
        assert_eq!(parse_bui("R1000").unwrap(), Bui::Stresstest(StresstestStorm::Regional48hT1000));
        assert_eq!(parse_bui("40:60").unwrap(), Bui::Blok { mm: 40.0, min: 60.0 });
        assert_eq!(parse_bui("C:/buien/meting.csv").unwrap(), Bui::Bestand("C:/buien/meting.csv".into()));
        let a = api::Args::parse("SOORT=wadi BODEM=1.2 DIEPTE=0,4 TALUD=1:4 KNIJP=110 DREMPEL=1.5,2 INFILTRATIE=10 BUI=T100 OPPERVLAK=0.5ha TIN=ja");
        let b = Berging::from_args(&a).unwrap();
        assert_eq!((b.soort, b.bodem, b.diepte, b.talud), (Soort::Wadi, 1.2, 0.4, 4.0));
        assert_eq!(b.knijp, Some((0.11, None)));
        assert_eq!(b.drempel, Some((1.5, 2.0)));
        assert_eq!(b.oppervlak, Some(5000.0));
        assert!(b.tin);
        let b = Berging::from_args(&api::Args::parse("SOORT=krat BODEM=-1 DIEPTE=0.6 KNIJP=0.05,-0.9 TALUD=1:2")).unwrap();
        assert_eq!((b.talud, b.knijp, b.bui.clone()), (0.0, Some((0.05, Some(-0.9))), Bui::Leidraad(8)));
        assert!(Berging::from_args(&api::Args::parse("SOORT=sloot BODEM=0 DIEPTE=1")).is_err());
        assert!(Berging::from_args(&api::Args::parse("SOORT=wadi DIEPTE=1")).is_err());
        assert!(Berging::from_args(&api::Args::parse("SOORT=wadi BODEM=0 DIEPTE=1 DREMPEL=0.5")).is_err());
    }

    #[test]
    fn storm_files() {
        let i = storm_from_text("i", "minuten,mm/h\n5,12\n10,24\n").unwrap();
        close(i.total_depth_mm(), 3.0, 1e-12);
        let d = storm_from_text("d", "# meting\nminuten;mm\n5;1,0\n10;2,0\n").unwrap();
        close(d.total_depth_mm(), 3.0, 1e-12);
        close(d.peak_intensity_mm_h(), 24.0, 1e-12);
        let n = storm_from_text("n", "5,12\n10,24\n").unwrap();
        close(n.total_depth_mm(), 3.0, 1e-12);
    }

    #[test]
    fn outline_offset_matches_the_kernel_curve() {
        // A 20 × 10 m bottom, clockwise as drawn: the top at 1:3 over 0.5 m is 1.5 m out.
        let bottom = vec![[0.0, 0.0], [0.0, 10.0], [20.0, 10.0], [20.0, 0.0]];
        let top = offset_polygon(&bottom, 1.5);
        assert_eq!(top[0], [-1.5, -1.5]);
        assert_eq!(top[2], [21.5, 11.5]);
        let b = wadi();
        let c = b.curve(&bottom).unwrap();
        close(*c.areas.last().unwrap(), polygon_area(&top), 1e-9);
        close(c.areas[0], 200.0, 1e-9);
        // Average end area over a linear-in-r² area: the exact frustum volume.
        let r: f64 = 1.5;
        let exact = 0.5 * (200.0 + 60.0 * r / 2.0 + 4.0 * r * r / 3.0);
        close(c.max_storage(), exact, 1e-2);
        // Crates: footprint × depth × void ratio.
        let k = Berging { soort: Soort::Krat, talud: 0.0, ..wadi() };
        close(k.curve(&bottom).unwrap().max_storage(), 200.0 * 0.5 * kern::CRATE_POROSITY, 1e-9);
        assert_eq!(k.inflow_area(&k.curve(&bottom).unwrap()), 1000.0);
        close(b.inflow_area(&c), 1000.0 + polygon_area(&top), 1e-9);
    }

    #[test]
    fn slope_ticks_reach_the_bottom() {
        let bottom = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let top = offset_polygon(&bottom, 2.0);
        let ticks = talud_ticks(&top, 2.0, 1.0);
        assert!(!ticks.is_empty());
        for (p, q, long) in &ticks {
            let len = (q[0] - p[0]).hypot(q[1] - p[1]);
            close(len, if *long { 2.0 } else { 1.0 }, 1e-9);
            // Long ticks end on the bottom outline: x or y at its edges.
            if *long {
                let on = (q[1].abs() < 1e-9 || (q[1] - 10.0).abs() < 1e-9) && (0.0..=20.0).contains(&q[0]) || (q[0].abs() < 1e-9 || (q[0] - 20.0).abs() < 1e-9) && (0.0..=10.0).contains(&q[1]);
                assert!(on, "{q:?}");
            }
        }
    }

    #[test]
    fn outlets_and_routing() {
        let bottom = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let mut b = wadi();
        assert!(b.outlets().is_empty());
        b.knijp = Some((0.1, None));
        b.drempel = Some((1.45, 2.0));
        b.infiltratie = Some(20.0);
        let o = b.outlets();
        assert_eq!(o[0], Outlet::orifice(0.1, 1.0));
        assert_eq!(o[1], Outlet::weir(1.45, 2.0));
        assert!(matches!(o[2], Outlet::Infiltration { area: InfiltrationArea::Wetted, .. }));
        let c = b.curve(&bottom).unwrap();
        let storm = b.bui.storm().unwrap();
        let r = b.route(&c, &storm).unwrap();
        let s = &r.summary;
        // 19.8 mm on the connected area plus the open water.
        close(s.inflow_volume, 0.0198 * b.inflow_area(&c), 1e-6);
        assert!(s.max_level > b.bodem && s.max_level < b.top());
        assert!(s.drain_time_s.is_some() && s.mass_balance_error.abs() < 1e-9);
        assert!(!s.overflows);
        // A T = 100 storm on a large area overtops.
        b.oppervlak = Some(20_000.0);
        b.bui = Bui::Stresstest(StresstestStorm::Local1hT100);
        let r = b.route(&c, &b.bui.storm().unwrap()).unwrap();
        assert!(r.summary.overflows);
    }

    #[test]
    fn graph_fits_its_box() {
        let bottom = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [0.0, 10.0]];
        let mut b = wadi();
        b.knijp = Some((0.1, None));
        let c = b.curve(&bottom).unwrap();
        let storm = b.bui.storm().unwrap();
        let r = b.route(&c, &storm).unwrap();
        let g = Grafiek::of(&r, b.bodem, b.top(), None, storm.duration_s());
        assert!(g.lijn.len() >= 2 && g.lijn.len() <= GRAFIEK_PUNTEN + 1);
        assert!(g.z_min <= b.bodem && g.z_max >= b.top());
        assert!(g.t_max_h * 3600.0 >= storm.duration_s());
        for p in &g.lijn {
            assert!(p[0] >= -1e-9 && p[0] <= GRAFIEK_B + 1e-9 && p[1] >= -1e-9 && p[1] <= GRAFIEK_H + 1e-9, "{p:?}");
        }
        assert_eq!(g.lijn[0], g.xy(0.0, b.bodem));
    }
}
