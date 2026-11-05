//! Sewer design for Dutch practice (pipe networks): a run of
//! manholes and free-flow pipes from a polyline or picked points, with its
//! invert levels (BOB, binnenonderkant buis) computed downstream so that the
//! minimum cover and the minimum slope both hold over the drawing's TIN or
//! AHN. Everything goes on the NLCS RI layers the GWSW import uses, with
//! labels, and the whole network is object data on its entities
//! (`riool=<name>`), so it is read back, edited and recomputed
//! (`CIV_RIOOL_BIJWERKEN`). Further: its length profile, checks (cover,
//! slope, manhole spacing, Prandtl-Colebrook capacity, KLIC and sewer
//! crossings), export to GWSW-OroX and CSV, and an existing GWSW network
//! taken over as an editable network.
//!
//! The design values and their sources are in `docs/civiel-riolering.md`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, Circle, Color, EntityType, Handle, Vector3};
use ocs_plugin_api::host::ExtendedDataRecord;
use serde_json::{json, Value};

use super::api::{self, handle_text};
use super::terrein::Tin;
use super::{data_value, hoogtebron, nice_step, record, Ctx, Hoogtebron, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::gis::gwsw;
use crate::gis::vector::Feature;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::{self, Element, LayerSpec};

/// A message in the interface language, with positional arguments:
/// `t!("{} putten", "{} manholes", n)`.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

// ── Design values (sources: docs/civiel-riolering.md) ───────────────────────

/// Minimum cover above the top of the pipe, outside of the wall (m):
/// RIONED, Module Riolering voor het HBO §5.2.1 (frost and traffic).
pub const DEKKING: f64 = 0.8;
/// Rain intensity for a rational-method design flow (l/s/ha): RIONED HBO §5.2.2.
pub const REGEN: f64 = 60.0;
/// Peak dry weather flow per inhabitant (l/h): RIONED HBO §5.2.2.
pub const DWA_PER_INWONER: f64 = 12.0;
/// Kinematic viscosity of water at 10 °C (m²/s).
pub const NU: f64 = 1.31e-6;
/// Self-cleansing full-pipe velocity (m/s): RIONED HBO §5.2.1.
pub const V_MIN: f64 = 0.6;
/// Vertical clearance, outside to outside (m): between sewers RIONED HBO
/// ch. 6 (0,10–0,15 m); to other pipes and cables 0,20 m (De Ronde Venen,
/// art. 2.1.2).
pub const VRIJ_RIOOL: f64 = 0.15;
pub const VRIJ_KLIC: f64 = 0.20;
const G: f64 = 9.81;

/// Minimum slope (fraction) for a nominal size: i = 1/D with D in mm
/// (RIONED HBO §5.2.1, about constant bed shear stress).
pub fn min_verhang(dn: f64) -> f64 {
    1.0 / dn.max(100.0)
}

/// Maximum manhole spacing (m): about 70 m up to DN 800, up to 100 m for
/// larger pipes (RIONED HBO ch. 6; Rotterdam and Veenendaal use 75 m).
pub fn max_putafstand(dn: f64) -> f64 {
    if dn <= 800.0 { 70.0 } else { 100.0 }
}

/// Maximum full-pipe velocity (m/s): 4 m/s (RIONED HBO §5.2.1), 3 m/s for
/// concrete carrying sand.
pub fn v_max(mat: Materiaal) -> f64 {
    if matches!(mat, Materiaal::Beton) { 3.0 } else { 4.0 }
}

/// Default nominal size per system (mm): DWA 250, HWA and GWA 300 inside
/// (concrete) or 315 outside (plastic) — RIONED HBO §5.2.1, Veenendaal 556–559.
pub fn standaard_diameter(s: Stelsel) -> f64 {
    match s {
        Stelsel::Dwa => 250.0,
        _ => 315.0,
    }
}

/// Default inner manhole size (mm, round) for the largest pipe: Rotterdam,
/// PvE Vrijverval riolering, tabel 2 (800 up to 400, 1000 up to 600, …).
pub fn standaard_put(dn: f64) -> [f64; 2] {
    let size = [(400.0, 800.0), (600.0, 1000.0), (800.0, 1250.0), (1000.0, 1500.0), (1250.0, 1750.0)].iter().find(|(d, _)| dn <= *d + 1e-9).map(|(_, s)| *s).unwrap_or(2000.0);
    [size, 0.0]
}

// ── Systems and materials ───────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stelsel {
    Dwa,
    Hwa,
    Gwa,
    Itr,
}

impl Stelsel {
    pub const ALL: [Stelsel; 4] = [Stelsel::Dwa, Stelsel::Hwa, Stelsel::Gwa, Stelsel::Itr];

    pub fn parse(s: &str) -> Option<Stelsel> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "dwa" | "vuilwater" | "afvalwater" | "vwa" | "vuilwaterriool" => Stelsel::Dwa,
            "hwa" | "rwa" | "hemelwater" | "regenwater" | "hemelwaterriool" => Stelsel::Hwa,
            "gwa" | "gemengd" | "gemengdriool" => Stelsel::Gwa,
            "itr" | "infiltratie" | "iriool" | "infiltratieriool" => Stelsel::Itr,
            _ => return None,
        })
    }

    /// The NLCS prefix in main group RI.
    pub fn code(self) -> &'static str {
        match self {
            Stelsel::Dwa => "DWA",
            Stelsel::Hwa => "HWA",
            Stelsel::Gwa => "GWA",
            Stelsel::Itr => "ITR",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Stelsel::Dwa => tr("vuilwater", "foul water"),
            Stelsel::Hwa => tr("hemelwater", "storm water"),
            Stelsel::Gwa => tr("gemengd", "combined"),
            Stelsel::Itr => tr("infiltratie", "infiltration"),
        }
    }

    /// The GWSW class of a free-flow pipe of this system.
    pub fn gwsw_leiding(self) -> &'static str {
        match self {
            Stelsel::Dwa => "Vuilwaterriool",
            Stelsel::Hwa => "Hemelwaterriool",
            Stelsel::Gwa => "GemengdRiool",
            Stelsel::Itr => "Infiltratieriool",
        }
    }

    /// The GWSW class of the system (stelsel).
    pub fn gwsw_stelsel(self) -> &'static str {
        match self {
            Stelsel::Dwa => "Vuilwaterstelsel",
            Stelsel::Hwa => "Hemelwaterstelsel",
            Stelsel::Gwa => "GemengdStelsel",
            Stelsel::Itr => "Infiltratiestelsel",
        }
    }

    /// From a GWSW pipe type or system type (as the GWSW import reads them).
    pub fn from_gwsw(kind: &str, stelseltype: &str) -> Stelsel {
        match gwsw::system(kind, stelseltype) {
            "DWA" => Stelsel::Dwa,
            "HWA" => Stelsel::Hwa,
            "ITR" => Stelsel::Itr,
            _ => Stelsel::Gwa,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Materiaal {
    Beton,
    Pvc,
    Pp,
    Pe,
    Gres,
}

impl Materiaal {
    pub fn parse(s: &str) -> Option<Materiaal> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "beton" | "bt" | "gewapendbeton" | "concrete" => Materiaal::Beton,
            "pvc" | "polyvinylchloride" => Materiaal::Pvc,
            "pp" | "polypropyleen" | "polypropeen" => Materiaal::Pp,
            "pe" | "hdpe" | "polyethyleen" | "polyetheen" => Materiaal::Pe,
            "gres" | "gr" | "stoneware" | "keramiek" => Materiaal::Gres,
            _ => return None,
        })
    }

    /// The NLCS material name.
    pub fn code(self) -> &'static str {
        match self {
            Materiaal::Beton => "BETON",
            Materiaal::Pvc => "PVC",
            Materiaal::Pp => "PP",
            Materiaal::Pe => "PE",
            Materiaal::Gres => "GRES",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Materiaal::Beton => tr("beton", "concrete"),
            Materiaal::Pvc => "PVC",
            Materiaal::Pp => "PP",
            Materiaal::Pe => "PE",
            Materiaal::Gres => tr("gres", "vitrified clay"),
        }
    }

    /// The GWSW material individual (`MateriaalLeiding`).
    pub fn gwsw(self) -> &'static str {
        match self {
            Materiaal::Beton => "Beton",
            Materiaal::Pvc => "PVC",
            Materiaal::Pp => "Polypropyleen",
            Materiaal::Pe => "PE",
            Materiaal::Gres => "Gres",
        }
    }

    /// Plastic pipes are named by their outside diameter (PVC 315), concrete
    /// and vitrified clay by the inside (beton 300).
    pub fn nominal_outside(self) -> bool {
        matches!(self, Materiaal::Pvc | Materiaal::Pp | Materiaal::Pe)
    }

    /// Wall thickness (mm) at a nominal size.
    pub fn wand(self, dn: f64) -> f64 {
        match self {
            // NEN-EN 1401 SN 8 (Wavin BENOR sheet); other sizes OD / 34 (SDR 34, estimate).
            Materiaal::Pvc => table(&[(160.0, 4.7), (200.0, 5.9), (250.0, 7.3), (315.0, 9.2), (400.0, 11.7), (500.0, 14.6)], dn).unwrap_or((dn / 34.0 * 10.0).round() / 10.0),
            // Solid-wall PP and PE: SDR 33 (estimate, not verified).
            Materiaal::Pp | Materiaal::Pe => (dn / 33.0 * 10.0).round() / 10.0,
            // Concrete (NEN-EN 1916): Martens "Maru betonbuizen", interpolated.
            Materiaal::Beton => interpolate(&[(250.0, 50.0), (300.0, 55.0), (400.0, 55.0), (500.0, 70.0), (600.0, 85.0), (700.0, 95.0), (800.0, 105.0), (900.0, 110.0), (1000.0, 125.0), (1250.0, 154.0), (1500.0, 187.0)], dn),
            // Vitrified clay (NEN-EN 295): about DN/12 + 10 mm (estimate, not verified).
            Materiaal::Gres => (dn / 12.0 + 10.0).round(),
        }
    }

    pub fn binnen(self, dn: f64) -> f64 {
        if self.nominal_outside() { dn - 2.0 * self.wand(dn) } else { dn }
    }

    pub fn buiten(self, dn: f64) -> f64 {
        if self.nominal_outside() { dn } else { dn + 2.0 * self.wand(dn) }
    }

    /// Wall roughness k (mm) for Prandtl-Colebrook: RIONED HBO tabel 5.5
    /// (first indication; PP as PE, an estimate).
    pub fn ruwheid(self) -> f64 {
        match self {
            Materiaal::Pvc | Materiaal::Pp | Materiaal::Pe => 0.4,
            Materiaal::Gres => 0.5,
            Materiaal::Beton => 3.0,
        }
    }
}

/// A value from a table at an exact size.
fn table(t: &[(f64, f64)], x: f64) -> Option<f64> {
    t.iter().find(|(k, _)| (k - x).abs() < 0.5).map(|(_, v)| *v)
}

/// Linear in a sorted table, constant ratio beyond its ends.
fn interpolate(t: &[(f64, f64)], x: f64) -> f64 {
    let (first, last) = (t[0], t[t.len() - 1]);
    if x <= first.0 {
        return (first.1 * x / first.0).round();
    }
    if x >= last.0 {
        return (last.1 * x / last.0).round();
    }
    for w in t.windows(2) {
        if x <= w[1].0 {
            return (w[0].1 + (w[1].1 - w[0].1) * (x - w[0].0) / (w[1].0 - w[0].0)).round();
        }
    }
    last.1
}

/// Default material: plastic up to 630, concrete above.
pub fn standaard_materiaal(dn: f64) -> Materiaal {
    if dn <= 630.0 { Materiaal::Pvc } else { Materiaal::Beton }
}

// ── The network ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Put {
    pub naam: String,
    /// Project coordinates (m).
    pub xy: XY,
    /// Ground level (m NAP).
    pub maaiveld: Option<f64>,
    /// Set by hand: not taken from the TIN or AHN again.
    pub maaiveld_vast: bool,
    /// Fixed outgoing invert level (m NAP), e.g. the start BOB.
    pub bob_vast: Option<f64>,
    /// Inner size (mm): [diameter, 0] round, [width, length] rectangular.
    pub maat: [f64; 2],
    pub stelsel: Stelsel,
    /// Existing (taken over from GWSW): its pipes are not recomputed.
    pub bestaand: bool,
}

impl Put {
    pub fn new(naam: String, xy: XY, stelsel: Stelsel, maat: [f64; 2]) -> Put {
        Put { naam, xy, maaiveld: None, maaiveld_vast: false, bob_vast: None, maat, stelsel, bestaand: false }
    }

    /// Half the outer extent in plan (m), for drawing and labels.
    pub fn straal(&self) -> f64 {
        self.maat[0].max(self.maat[1]) / 2000.0
    }

    pub fn maat_text(&self) -> String {
        if self.maat[1] > 0.0 { format!("{:.0}x{:.0}", self.maat[0], self.maat[1]) } else { format!("{:.0}", self.maat[0]) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Leiding {
    pub van: usize,
    pub naar: usize,
    /// Nominal size (mm).
    pub dn: f64,
    pub materiaal: Materiaal,
    pub stelsel: Stelsel,
    pub bob_van: f64,
    pub bob_naar: f64,
    /// Design rules of this pipe: minimum cover (m) and slope (fraction).
    pub dekking_min: f64,
    pub verhang_min: f64,
    pub bestaand: bool,
    /// Ground along the pipe after a computation: (m from `van`, m NAP).
    pub grond: Vec<(f64, f64)>,
}

impl Leiding {
    pub fn new(van: usize, naar: usize, dn: f64, materiaal: Materiaal, stelsel: Stelsel) -> Leiding {
        Leiding { van, naar, dn, materiaal, stelsel, bob_van: 0.0, bob_naar: 0.0, dekking_min: DEKKING, verhang_min: min_verhang(dn), bestaand: false, grond: Vec::new() }
    }

    /// Inner diameter (m).
    pub fn d(&self) -> f64 {
        self.materiaal.binnen(self.dn) / 1000.0
    }

    /// Wall thickness (m).
    pub fn t(&self) -> f64 {
        self.materiaal.wand(self.dn) / 1000.0
    }

    /// From the invert to the outside top of the pipe (m).
    pub fn hoogte(&self) -> f64 {
        self.d() + self.t()
    }

    /// Invert level at `x` m from the start.
    pub fn bob_at(&self, x: f64, len: f64) -> f64 {
        if len <= 0.0 { self.bob_van } else { self.bob_van + (self.bob_naar - self.bob_van) * (x / len).clamp(0.0, 1.0) }
    }

    pub fn label(&self) -> String {
        format!("{} Ø{:.0}", self.materiaal.label(), self.dn)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    pub naam: String,
    pub putten: Vec<Put>,
    pub leidingen: Vec<Leiding>,
}

/// The mean ground in a ring around a point (0,5 to 3 m), where the point
/// itself has none.
fn near(ground: &mut dyn FnMut(XY) -> Option<f64>, p: XY) -> Option<f64> {
    for r in [0.5, 1.0, 2.0, 3.0] {
        let found: Vec<f64> = (0..8).filter_map(|i| {
            let a = i as f64 * std::f64::consts::FRAC_PI_4;
            ground([p[0] + r * a.cos(), p[1] + r * a.sin()])
        }).collect();
        if !found.is_empty() {
            return Some(found.iter().sum::<f64>() / found.len() as f64);
        }
    }
    None
}

pub fn dist(a: XY, b: XY) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

impl Net {
    pub fn lengte(&self, i: usize) -> f64 {
        let l = &self.leidingen[i];
        dist(self.putten[l.van].xy, self.putten[l.naar].xy)
    }

    pub fn verhang(&self, i: usize) -> f64 {
        let len = self.lengte(i);
        if len <= 0.0 { 0.0 } else { (self.leidingen[i].bob_van - self.leidingen[i].bob_naar) / len }
    }

    pub fn naam_leiding(&self, i: usize) -> String {
        let l = &self.leidingen[i];
        format!("{}-{}", self.putten[l.van].naam, self.putten[l.naar].naam)
    }

    /// The main system: that of most pipes.
    pub fn stelsel(&self) -> Stelsel {
        let mut n: HashMap<Stelsel, usize> = HashMap::new();
        for l in &self.leidingen {
            *n.entry(l.stelsel).or_default() += 1;
        }
        n.into_iter().max_by_key(|(s, c)| (*c, std::cmp::Reverse(*s))).map(|(s, _)| s).or(self.putten.first().map(|p| p.stelsel)).unwrap_or(Stelsel::Dwa)
    }

    pub fn incoming(&self, put: usize) -> impl Iterator<Item = usize> + '_ {
        self.leidingen.iter().enumerate().filter(move |(_, l)| l.naar == put).map(|(i, _)| i)
    }

    pub fn outgoing(&self, put: usize) -> impl Iterator<Item = usize> + '_ {
        self.leidingen.iter().enumerate().filter(move |(_, l)| l.van == put).map(|(i, _)| i)
    }

    /// The lowest invert at a manhole (its bottom), from its pipes.
    pub fn bob_laagst(&self, put: usize) -> Option<f64> {
        self.leidingen.iter().filter(|l| l.van == put || l.naar == put).map(|l| if l.van == put { l.bob_van } else { l.bob_naar }).reduce(f64::min)
    }

    /// The design pipes in flow order: every pipe after the pipes that flow
    /// into its upstream manhole. Existing pipes are fixed and need no order.
    pub fn volgorde(&self) -> Result<Vec<usize>, String> {
        let design: Vec<usize> = (0..self.leidingen.len()).filter(|i| !self.leidingen[*i].bestaand).collect();
        let mut waiting: HashMap<usize, usize> = HashMap::new();
        for &i in &design {
            let n = self.incoming(self.leidingen[i].van).filter(|j| !self.leidingen[*j].bestaand).count();
            waiting.insert(i, n);
        }
        let mut ready: Vec<usize> = design.iter().copied().filter(|i| waiting[i] == 0).collect();
        ready.reverse();
        let mut out = Vec::with_capacity(design.len());
        while let Some(i) = ready.pop() {
            out.push(i);
            let down = self.leidingen[i].naar;
            let mut next: Vec<usize> = self.outgoing(down).filter(|j| !self.leidingen[*j].bestaand).collect();
            next.reverse();
            for j in next {
                let w = waiting.get_mut(&j).unwrap();
                *w -= 1;
                if *w == 0 {
                    ready.push(j);
                }
            }
        }
        if out.len() < design.len() {
            return Err(tr("het riool loopt in een kring: geef de leidingen een stroomrichting zonder lus", "the sewer runs in a loop: give the pipes a flow direction without a cycle").into());
        }
        Ok(out)
    }

    /// Ground levels at the manholes and along the pipes from `ground`
    /// (project coordinates → m NAP), every `step` m. A manhole where the
    /// source has no height (AHN under a tree or a car) takes the ground
    /// close by, else that of its pipes' nearest samples; returns notes on
    /// those estimates.
    pub fn sample_ground(&mut self, ground: &mut dyn FnMut(XY) -> Option<f64>, step: f64) -> Vec<String> {
        let mut notes = Vec::new();
        for p in &mut self.putten {
            if !p.maaiveld_vast {
                if let Some(z) = ground(p.xy).or_else(|| near(ground, p.xy)) {
                    p.maaiveld = Some(z);
                }
            }
        }
        for i in 0..self.leidingen.len() {
            let (a, b) = (self.putten[self.leidingen[i].van].xy, self.putten[self.leidingen[i].naar].xy);
            let len = dist(a, b);
            let n = ((len / step.max(0.1)).ceil() as usize).clamp(1, 400);
            let mut g = Vec::with_capacity(n + 1);
            for j in 0..=n {
                let x = len * j as f64 / n as f64;
                let z = if j == 0 {
                    self.putten[self.leidingen[i].van].maaiveld
                } else if j == n {
                    self.putten[self.leidingen[i].naar].maaiveld
                } else {
                    let t = j as f64 / n as f64;
                    ground([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t])
                };
                if let Some(z) = z {
                    g.push((x, z));
                }
            }
            self.leidingen[i].grond = g;
        }
        // Still none: the nearest ground along its pipes.
        for k in 0..self.putten.len() {
            if self.putten[k].maaiveld.is_some() {
                continue;
            }
            let mut best: Option<(f64, f64)> = None;
            for (i, l) in self.leidingen.iter().enumerate() {
                let len = self.lengte(i);
                let end = if l.van == k { 0.0 } else if l.naar == k { len } else { continue };
                for (x, z) in &l.grond {
                    let d = (x - end).abs();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, *z));
                    }
                }
            }
            if let Some((d, z)) = best {
                self.putten[k].maaiveld = Some(z);
                notes.push(t!("maaiveld van put {} geschat uit het maaiveld op {} m", "ground level of manhole {} estimated from the ground {} m away", self.putten[k].naam, dec(d, 1)));
            }
        }
        notes
    }

    /// The invert levels of the design pipes, downstream: each pipe starts
    /// as high as its upstream manhole allows (the minimum cover over its
    /// whole length, the pipes flowing in with their crowns matched, a fixed
    /// BOB) and falls at the minimum slope, or steeper where the ground falls
    /// faster, so the cover holds everywhere (a cover-and-slope rule).
    /// Returns warnings.
    pub fn bereken(&mut self) -> Result<Vec<String>, String> {
        let mut warn = Vec::new();
        for i in self.volgorde()? {
            let len = self.lengte(i);
            let l = &self.leidingen[i];
            let (van, naar) = (l.van, l.naar);
            let name = self.naam_leiding(i);
            if len < 1e-3 {
                return Err(t!("leiding {} heeft geen lengte", "pipe {} has no length", name));
            }
            let h = l.hoogte();
            let lim: Vec<(f64, f64)> = l.grond.iter().map(|(x, g)| (*x, g - l.dekking_min - h)).collect();
            // From the pipes flowing in: never higher, crowns matched when
            // this pipe is larger.
            let from_in = self
                .incoming(van)
                .map(|j| {
                    let p = &self.leidingen[j];
                    p.bob_naar - (l.d() - p.d()).max(0.0)
                })
                .reduce(f64::min);
            let lim0 = lim.iter().find(|(x, _)| *x < 1e-6).map(|(_, v)| *v).or_else(|| lim.iter().map(|(_, v)| *v).reduce(f64::min));
            let mut b = match (lim0, from_in) {
                (Some(a), Some(c)) => a.min(c),
                (Some(a), None) => a,
                (None, Some(c)) => c,
                (None, None) => match self.putten[van].bob_vast {
                    Some(v) => v,
                    None => return Err(t!("geen maaiveld langs leiding {}: geef de put een maaiveld (MAAIVELD=) of een BOB", "no ground level along pipe {}: give the manhole a ground level (MAAIVELD=) or an invert level (BOB)", name)),
                },
            };
            if let Some(v) = self.putten[van].bob_vast {
                if let Some(c) = from_in {
                    if v > c + 1e-4 {
                        warn.push(t!("BOB {} in put {} ligt hoger dan de inkomende leiding ({})", "invert {} in manhole {} is above the incoming pipe ({})", nap(v, 2), self.putten[van].naam, nap(c, 2)));
                    }
                }
                if let Some(a) = lim0 {
                    if v > a + 1e-4 {
                        warn.push(t!("BOB {} in put {} geeft {} m te weinig dekking", "invert {} in manhole {} leaves {} m too little cover", nap(v, 2), self.putten[van].naam, dec(v - a, 2)));
                    }
                }
                b = v;
            }
            let mut s = l.verhang_min.max(0.0);
            for (x, v) in &lim {
                if *x > 1e-6 {
                    s = s.max((b - v) / x);
                }
            }
            let l = &mut self.leidingen[i];
            l.bob_van = b;
            l.bob_naar = b - s * len;
            let arrive = l.bob_naar;
            // Joining an existing manhole: arrive above its outgoing pipe.
            for k in self.outgoing(naar).collect::<Vec<_>>() {
                let o = &self.leidingen[k];
                if o.bestaand && arrive < o.bob_van - 1e-3 {
                    warn.push(t!(
                        "leiding {} komt in put {} {} m onder de BOB van de bestaande leiding ({})",
                        "pipe {} arrives in manhole {} {} m below the invert of the existing pipe ({})",
                        name,
                        self.putten[naar].naam,
                        dec(o.bob_van - arrive, 2),
                        nap(o.bob_van, 2)
                    ));
                }
            }
        }
        Ok(warn)
    }

    /// Cover (m) along pipe `i`: (x, cover) at its ground samples.
    pub fn dekking(&self, i: usize) -> Vec<(f64, f64)> {
        let len = self.lengte(i);
        let l = &self.leidingen[i];
        l.grond.iter().map(|(x, g)| (*x, g - (l.bob_at(*x, len) + l.hoogte()))).collect()
    }

    /// Upstream pipe length draining through each pipe, itself included (m).
    pub fn bovenstrooms(&self) -> Vec<f64> {
        let n = self.leidingen.len();
        let mut up = vec![f64::NAN; n];
        fn visit(net: &Net, i: usize, up: &mut Vec<f64>, depth: usize) -> f64 {
            if !up[i].is_nan() {
                return up[i];
            }
            if depth > net.leidingen.len() {
                return 0.0;
            }
            up[i] = 0.0;
            let mut v = net.lengte(i);
            let ins: Vec<usize> = net.incoming(net.leidingen[i].van).collect();
            for j in ins {
                v += visit(net, j, up, depth + 1);
            }
            up[i] = v;
            v
        }
        for i in 0..n {
            visit(self, i, &mut up, 0);
        }
        up
    }

    /// The pipes in a path from manhole `from` downstream to `to` (or the
    /// longest path through the network when not given).
    pub fn pad(&self, from: Option<usize>, to: Option<usize>) -> Result<Vec<usize>, String> {
        if self.leidingen.is_empty() {
            return Err(tr("het riool heeft geen leidingen", "the sewer has no pipes").into());
        }
        // Longest path ending at each pipe (by length), following the flow.
        let mut best: Vec<(f64, Option<usize>)> = vec![(f64::NAN, None); self.leidingen.len()];
        fn longest(net: &Net, i: usize, from: Option<usize>, best: &mut Vec<(f64, Option<usize>)>, depth: usize) -> f64 {
            if !best[i].0.is_nan() {
                return best[i].0;
            }
            best[i] = (f64::NEG_INFINITY, None);
            let l = &net.leidingen[i];
            let own = net.lengte(i);
            let start_ok = from.is_none_or(|f| f == l.van);
            let mut v = if start_ok { own } else { f64::NEG_INFINITY };
            let mut prev = None;
            if depth <= net.leidingen.len() && from != Some(l.van) {
                for j in net.incoming(l.van).collect::<Vec<_>>() {
                    let w = longest(net, j, from, best, depth + 1) + own;
                    if w > v {
                        v = w;
                        prev = Some(j);
                    }
                }
            }
            best[i] = (v, prev);
            v
        }
        let ends: Vec<usize> = match to {
            Some(t) => self.incoming(t).collect(),
            None => (0..self.leidingen.len()).collect(),
        };
        let mut end = None;
        let mut top = f64::NEG_INFINITY;
        for i in ends {
            let v = longest(self, i, from, &mut best, 0);
            if v > top {
                top = v;
                end = Some(i);
            }
        }
        let Some(mut i) = end.filter(|_| top.is_finite()) else {
            return Err(tr("geen doorgaande leidingen tussen die putten (let op de stroomrichting)", "no connected pipes between those manholes (mind the flow direction)").into());
        };
        let mut path = vec![i];
        while let Some(j) = best[i].1 {
            path.push(j);
            i = j;
        }
        path.reverse();
        Ok(path)
    }
}

// ── Hydraulics ──────────────────────────────────────────────────────────────

/// Full-pipe velocity (m/s) after Prandtl-Colebrook (Colebrook-White):
/// v = −2 log₁₀(k / 3,71 D + 2,51 ν / (D √(2gDI))) · √(2gDI), with D the
/// inner diameter (m), I the slope, k the wall roughness (m).
pub fn snelheid_vol(d: f64, slope: f64, k: f64) -> f64 {
    if d <= 0.0 || slope <= 0.0 {
        return 0.0;
    }
    let r = (2.0 * G * d * slope).sqrt();
    -2.0 * (k / (3.71 * d) + 2.51 * NU / (d * r)).log10() * r
}

/// Full-pipe capacity (m³/s).
pub fn capaciteit_vol(d: f64, slope: f64, k: f64) -> f64 {
    snelheid_vol(d, slope, k) * std::f64::consts::PI * d * d / 4.0
}

// ── Argument parsing ────────────────────────────────────────────────────────

/// A slope: "1:250", in per mille (4 = 4 ‰) or as a fraction (0,004).
pub fn parse_verhang(s: &str) -> Result<f64, String> {
    let t = s.trim().replace(',', ".").replace('‰', "");
    if let Some((a, b)) = t.split_once(':') {
        let bad = || t!("geen verhang: '{}'", "not a slope: '{}'", s);
        let (a, b): (f64, f64) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
        if a <= 0.0 || b <= 0.0 {
            return Err(bad());
        }
        return Ok(a / b);
    }
    let v: f64 = t.trim().parse().map_err(|_| t!("geen verhang: '{}' (bijv. 1:250, 4 voor 4 ‰ of 0,004)", "not a slope: '{}' (e.g. 1:250, 4 for 4 ‰ or 0.004)", s))?;
    if v <= 0.0 {
        return Err(t!("verhang moet positief zijn: '{}'", "the slope must be positive: '{}'", s));
    }
    Ok(if v < 0.1 { v } else { v / 1000.0 })
}

/// "1:250" for a slope (fraction).
pub fn verhang_text(s: f64) -> String {
    if s <= 1e-9 {
        return if s < -1e-9 { tr("tegenverhang", "adverse slope").into() } else { "0".into() };
    }
    let n = 1.0 / s;
    if n >= 100.0 { format!("1:{:.0}", (n / 5.0).round() * 5.0) } else { format!("1:{:.0}", n) }
}

/// Manhole size: "1000" (round) or "800x1000" (mm).
pub fn parse_maat(s: &str) -> Result<[f64; 2], String> {
    let t = s.trim().to_ascii_lowercase().replace(',', ".");
    let num = |v: &str| v.trim().parse::<f64>().ok().filter(|v| *v >= 300.0 && *v <= 5000.0);
    match t.split_once(['x', '*']) {
        Some((a, b)) => match (num(a), num(b)) {
            (Some(a), Some(b)) => Ok([a, b]),
            _ => Err(t!("PUT: maat in mm, bijv. 1000 of 800x1000; kreeg '{}'", "PUT: size in mm, e.g. 1000 or 800x1000; got '{}'", s)),
        },
        None => num(&t).map(|a| [a, 0.0]).ok_or_else(|| t!("PUT: maat in mm, bijv. 1000 of 800x1000; kreeg '{}'", "PUT: size in mm, e.g. 1000 or 800x1000; got '{}'", s)),
    }
}

/// A connected area: m², or hectares with "ha".
pub fn parse_oppervlak(s: &str) -> Result<f64, String> {
    let t = s.trim().to_ascii_lowercase().replace(',', ".");
    let (v, ha) = match t.strip_suffix("ha") {
        Some(v) => (v, true),
        None => (t.strip_suffix("m2").unwrap_or(&t), false),
    };
    let v: f64 = v.trim().parse().map_err(|_| t!("OPPERVLAK: m² of ha, bijv. 12500 of 1,25ha; kreeg '{}'", "OPPERVLAK: m² or ha, e.g. 12500 or 1.25ha; got '{}'", s))?;
    Ok(if ha { v * 10_000.0 } else { v })
}

/// Manhole names from a prefix with a start number ("P101" → P101, P102 …;
/// "DWA-001" keeps the zeros).
pub fn putnummers(start: &str, n: usize) -> Vec<String> {
    let digits = start.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    let (prefix, num) = start.split_at(start.len() - digits);
    let first: u64 = num.parse().unwrap_or(1);
    (0..n).map(|i| format!("{prefix}{:0width$}", first + i as u64, width = digits)).collect()
}

fn dec(v: f64, n: usize) -> String {
    format!("{v:.n$}").replace('.', ",")
}

fn m(v: f64) -> String {
    format!("{v:.3}")
}

// ── Ground ──────────────────────────────────────────────────────────────────

/// Where ground levels come from: the TINs in the drawing (design N first
/// when asked, then existing B), else AHN (or the worldwide terrain).
pub struct Grond {
    tins: Vec<(char, Tin)>,
    bron: Option<Hoogtebron>,
    bron_err: Option<String>,
    used: BTreeSet<String>,
    f: f64,
    site: Option<crate::gis::site::Site>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Referentie {
    /// The existing TIN (B), else AHN: like the other civil tools.
    Auto,
    /// The design TIN (N) first, then B, then AHN.
    Ontwerp,
    Ahn,
}

impl Referentie {
    pub fn parse(s: &str) -> Option<Referentie> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "tin" | "b" | "bestaand" => Referentie::Auto,
            "ontwerp" | "n" => Referentie::Ontwerp,
            "ahn" => Referentie::Ahn,
            _ => return None,
        })
    }

    pub fn code(self) -> &'static str {
        match self {
            Referentie::Auto => "auto",
            Referentie::Ontwerp => "ontwerp",
            Referentie::Ahn => "ahn",
        }
    }
}

impl Grond {
    pub fn new(ctx: &Ctx, doc: &CadDocument, r: Referentie) -> Grond {
        let order: &[char] = match r {
            Referentie::Auto => &['B'],
            Referentie::Ontwerp => &['N', 'B'],
            Referentie::Ahn => &[],
        };
        let tins = order.iter().filter_map(|c| Tin::of_status(doc, *c).map(|t| (*c, t))).collect();
        let (bron, bron_err) = match hoogtebron(ctx) {
            Ok(b) => (Some(b), None),
            Err(e) => (None, Some(e)),
        };
        Grond { tins, bron, bron_err, used: BTreeSet::new(), f: ctx.f, site: ctx.site.clone() }
    }

    /// Ground level (m) at a point in project coordinates.
    pub fn height(&mut self, p: XY) -> Option<f64> {
        let d = match &self.site {
            Some(s) => s.to_drawing(p, self.f),
            None => [p[0] * self.f, p[1] * self.f],
        };
        for (c, t) in &self.tins {
            if let Some(z) = t.height(d) {
                self.used.insert(format!("TIN {c}"));
                return Some(z / self.f);
            }
        }
        let b = self.bron.as_mut()?;
        let z = b.height(d);
        if z.is_some() {
            self.used.insert(b.label());
        }
        z
    }

    pub fn label(&self) -> String {
        if self.used.is_empty() { tr("geen maaiveld", "no ground level").into() } else { self.used.iter().cloned().collect::<Vec<_>>().join(" + ") }
    }

    pub fn note_source(&self) {
        if let Some(b) = &self.bron {
            if self.used.contains(&b.label()) {
                bridge::note_source(b.attribution());
            }
        }
    }

    pub fn error(&self) -> String {
        self.bron.as_ref().and_then(|b| b.error()).or(self.bron_err.clone()).unwrap_or_else(|| tr("geen TIN in de tekening en geen hoogtebron op deze plek", "no TIN in the drawing and no height source at this place").into())
    }
}

// ── The network in the drawing (object data) ────────────────────────────────

fn num(e: &EntityType, key: &str) -> Option<f64> {
    data_value(e, key).and_then(|s| s.trim().replace(',', ".").parse().ok()).filter(|v: &f64| v.is_finite())
}

fn yes(e: &EntityType, key: &str) -> bool {
    matches!(data_value(e, key).as_deref(), Some("ja" | "1" | "true"))
}

/// The centre of a manhole outline: a circle, or a closed polyline.
fn centre(e: &EntityType) -> Option<XY> {
    match e {
        EntityType::Circle(c) => Some([c.center.x, c.center.y]),
        EntityType::LwPolyline(p) if !p.vertices.is_empty() => {
            let n = p.vertices.len() as f64;
            Some([p.vertices.iter().map(|v| v.location.x).sum::<f64>() / n, p.vertices.iter().map(|v| v.location.y).sum::<f64>() / n])
        }
        _ => None,
    }
}

/// Plan vertices of a line-like entity (drawing coordinates) with their z,
/// when the entity carries one.
fn vertices(e: &EntityType) -> Vec<(XY, Option<f64>)> {
    match e {
        EntityType::Line(l) => vec![([l.start.x, l.start.y], Some(l.start.z)), ([l.end.x, l.end.y], Some(l.end.z))],
        EntityType::LwPolyline(p) => {
            let mut v: Vec<(XY, Option<f64>)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], Some(p.elevation))).collect();
            if p.is_closed && v.len() > 2 {
                v.push(v[0]);
            }
            v
        }
        EntityType::Polyline2D(p) => p.vertices.iter().map(|v| ([v.location.x, v.location.y], Some(v.location.z))).collect(),
        EntityType::Polyline3D(p) => p.vertices.iter().map(|v| ([v.position.x, v.position.y], Some(v.position.z))).collect(),
        _ => Vec::new(),
    }
}

/// A network read back from its entities.
pub struct Gelezen {
    pub net: Net,
    /// Every entity of the network (to replace it).
    pub handles: Vec<Handle>,
    /// Handles of the manholes and pipes, by index.
    pub put_handles: Vec<Option<Handle>>,
    pub leiding_handles: Vec<Option<Handle>>,
    /// Changes in the drawing since the last computation.
    pub changes: Vec<String>,
    pub referentie: Referentie,
    /// The minimum slope follows the nominal size (no VERHANG= given).
    pub verhang_auto: Vec<bool>,
}

/// Every CIV_RIOOL network in the drawing, by its object data.
pub fn lees(doc: &CadDocument, ctx: &Ctx) -> Vec<Gelezen> {
    let mut groups: std::collections::BTreeMap<String, Vec<&EntityType>> = Default::default();
    for e in doc.model_space_entities() {
        if let Some(n) = data_value(e, "riool") {
            groups.entry(n).or_default().push(e);
        }
    }
    groups.into_iter().map(|(naam, es)| lees_net(ctx, naam, &es)).filter(|g| !g.net.putten.is_empty() || !g.handles.is_empty()).collect()
}

fn lees_net(ctx: &Ctx, naam: String, es: &[&EntityType]) -> Gelezen {
    let handles: Vec<Handle> = es.iter().map(|e| e.common().handle).collect();
    let mut net = Net { naam, putten: Vec::new(), leidingen: Vec::new() };
    let mut put_handles = Vec::new();
    let mut recorded: Vec<Option<XY>> = Vec::new();
    let mut moved: Vec<bool> = Vec::new();
    let mut changes = Vec::new();
    let mut referentie = Referentie::Auto;
    let rol = |e: &EntityType| data_value(e, "rol").unwrap_or_default();
    for e in es.iter().filter(|e| rol(e) == "put") {
        let Some(c) = centre(e) else { continue };
        let Some(name) = data_value(e, "put") else { continue };
        if net.putten.iter().any(|p| p.naam == name) {
            continue;
        }
        let cur = ctx.from_drawing(c);
        let rec = num(e, "x").zip(num(e, "y")).map(|(x, y)| [x, y]);
        if let Some(r) = data_value(e, "referentie").and_then(|r| Referentie::parse(&r)) {
            referentie = r;
        }
        let stelsel = data_value(e, "stelsel").and_then(|s| Stelsel::parse(&s)).unwrap_or(Stelsel::Dwa);
        let maat = data_value(e, "afmeting").and_then(|s| parse_maat(&s).ok()).unwrap_or([1000.0, 0.0]);
        let mut p = Put::new(name, cur, stelsel, maat);
        p.maaiveld = num(e, "maaiveld");
        p.maaiveld_vast = yes(e, "maaiveld_vast");
        p.bob_vast = num(e, "bob_vast");
        p.bestaand = yes(e, "bestaand");
        let is_moved = rec.is_some_and(|r| dist(r, cur) > 1e-3);
        if is_moved {
            changes.push(t!("put {} verplaatst ({} m)", "manhole {} moved ({} m)", p.naam, dec(dist(rec.unwrap(), cur), 2)));
        }
        net.putten.push(p);
        put_handles.push(Some(e.common().handle));
        recorded.push(rec);
        moved.push(is_moved);
    }
    let mut leiding_handles = Vec::new();
    let mut verhang_auto = Vec::new();
    let index = |net: &Net, n: &str| net.putten.iter().position(|p| p.naam == n);
    let mut missing: Vec<String> = Vec::new();
    // (from, to, pipe, its drawn ends, handle, minimum slope by size)
    type RawPipe = (String, String, Leiding, Option<(XY, XY)>, Handle, bool);
    let mut raw: Vec<RawPipe> = Vec::new();
    for e in es.iter().filter(|e| rol(e) == "leiding") {
        let (Some(van), Some(naar)) = (data_value(e, "van"), data_value(e, "naar")) else { continue };
        let v = vertices(e);
        let ends = (v.len() >= 2).then(|| (ctx.from_drawing(v[0].0), ctx.from_drawing(v[v.len() - 1].0)));
        let dn = num(e, "diameter").unwrap_or(250.0);
        let mat = data_value(e, "materiaal").and_then(|s| Materiaal::parse(&s)).unwrap_or_else(|| standaard_materiaal(dn));
        let stelsel = data_value(e, "stelsel").and_then(|s| Stelsel::parse(&s)).unwrap_or(Stelsel::Dwa);
        let mut l = Leiding::new(0, 0, dn, mat, stelsel);
        l.bob_van = num(e, "bob_van").unwrap_or(0.0);
        l.bob_naar = num(e, "bob_naar").unwrap_or(0.0);
        l.dekking_min = num(e, "dekking_min").unwrap_or(DEKKING);
        l.verhang_min = num(e, "verhang_min").unwrap_or_else(|| min_verhang(dn));
        l.bestaand = yes(e, "bestaand");
        for n in [&van, &naar] {
            if index(&net, n).is_none() && !missing.contains(n) {
                missing.push(n.clone());
            }
        }
        raw.push((van, naar, l, ends, e.common().handle, data_value(e, "verhang_auto").is_none_or(|v| v == "ja")));
    }
    // A manhole deleted in the drawing: with one pipe in and one out the two
    // pipes become one; otherwise it comes back where its pipes end.
    for name in &missing {
        let ins: Vec<usize> = raw.iter().enumerate().filter(|(_, r)| &r.1 == name).map(|(i, _)| i).collect();
        let outs: Vec<usize> = raw.iter().enumerate().filter(|(_, r)| &r.0 == name).map(|(i, _)| i).collect();
        if ins.len() == 1 && outs.len() == 1 {
            let (a, b) = (ins[0], outs[0]);
            let down = raw[b].clone();
            let up = &mut raw[a];
            up.1 = down.1.clone();
            if down.2.dn > up.2.dn {
                up.2.dn = down.2.dn;
                up.2.materiaal = down.2.materiaal;
            }
            up.3 = match (up.3, down.3) {
                (Some(u), Some(d)) => Some((u.0, d.1)),
                _ => None,
            };
            raw.remove(b);
            changes.push(t!("put {} verwijderd: de leidingen erdoor zijn samengevoegd", "manhole {} deleted: its pipes are joined into one", name));
        } else {
            let at = raw.iter().find_map(|r| if &r.0 == name { r.3.map(|e| e.0) } else if &r.1 == name { r.3.map(|e| e.1) } else { None });
            if let Some(at) = at {
                let st = raw.iter().find(|r| &r.0 == name || &r.1 == name).map(|r| r.2.stelsel).unwrap_or(Stelsel::Dwa);
                net.putten.push(Put::new(name.clone(), at, st, [1000.0, 0.0]));
                put_handles.push(None);
                recorded.push(Some(at));
                moved.push(false);
                changes.push(t!("put {} ontbrak en is teruggezet", "manhole {} was missing and is put back", name));
            }
        }
    }
    for (van, naar, mut l, ends, h, auto) in raw {
        let (Some(a), Some(b)) = (index(&net, &van), index(&net, &naar)) else { continue };
        l.van = a;
        l.naar = b;
        // A pipe end dragged away from a manhole that stayed: the manhole follows.
        if let Some((e0, e1)) = ends {
            for (p, e) in [(a, e0), (b, e1)] {
                if !moved[p] {
                    if let Some(r) = recorded[p] {
                        if dist(r, e) > 1e-3 && dist(net.putten[p].xy, r) < 1e-3 {
                            changes.push(t!("put {} verplaatst met de leiding ({} m)", "manhole {} moved with its pipe ({} m)", net.putten[p].naam, dec(dist(r, e), 2)));
                            net.putten[p].xy = e;
                            moved[p] = true;
                        }
                    }
                }
            }
        }
        net.leidingen.push(l);
        leiding_handles.push(Some(h));
        verhang_auto.push(auto);
    }
    Gelezen { net, handles, put_handles, leiding_handles, changes, referentie, verhang_auto }
}

/// The NLCS object of a pipe: system, material and size, as the GWSW import
/// chooses it.
pub fn leiding_object(s: Stelsel, mat: Materiaal, dn: f64) -> String {
    let mut f = Feature::default();
    f.props.insert("type".into(), Value::String(s.gwsw_leiding().into()));
    f.props.insert("materiaal_leiding".into(), Value::String(mat.code().into()));
    f.props.insert("breedte_leiding".into(), json!(dn.round()));
    gwsw::nlcs_candidates("beheerleiding", &f).into_iter().find(|c| nlcs::object_by_name("RI", c).is_some()).unwrap_or_else(|| format!("{}_RIOOLLEIDING", s.code()))
}

/// The NLCS object of an inspection manhole of a system.
pub fn put_object(s: Stelsel) -> String {
    let mut f = Feature::default();
    f.props.insert("type".into(), Value::String("Inspectieput".into()));
    f.props.insert("stelseltype".into(), Value::String(s.gwsw_stelsel().into()));
    gwsw::nlcs_candidates("beheerput", &f).into_iter().find(|c| nlcs::object_by_name("RI", c).is_some()).unwrap_or_else(|| format!("{}_RIOOLPUT", s.code()))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Tag {
    Put(usize),
    Leiding(usize),
    Overig,
}

/// All new handles, and the handle per tag.
type Written = (Vec<Handle>, Vec<(Tag, Handle)>);

/// [`Out`] that remembers which handle belongs to which manhole and pipe.
#[derive(Default)]
struct Uit {
    out: Out,
    tags: Vec<(String, Tag)>,
}

impl Uit {
    fn put(&mut self, spec: &LayerSpec, e: Vec<EntityType>, tag: Tag) {
        for _ in &e {
            self.tags.push((spec.name.clone(), tag));
        }
        self.out.put(spec, e);
    }

    /// Write as one undo step: all handles, and the handle per tag.
    fn write(self, label: &str) -> Result<Written, String> {
        // Out writes layer by layer, in the order the layers were first used.
        let mut order: Vec<&str> = Vec::new();
        for (l, _) in &self.tags {
            if !order.contains(&l.as_str()) {
                order.push(l);
            }
        }
        let mut tagged: Vec<(usize, Tag)> = self.tags.iter().map(|(l, t)| (order.iter().position(|o| o == l).unwrap_or(0), *t)).collect();
        tagged.sort_by_key(|(i, _)| *i);
        let handles = self.out.write_handles(label)?;
        let pairs = tagged.into_iter().zip(handles.iter().copied()).map(|((_, t), h)| (t, h)).collect();
        Ok((handles, pairs))
    }
}

fn put_record(net: &Net, i: usize, referentie: Referentie) -> ExtendedDataRecord {
    let p = &net.putten[i];
    let low = net.bob_laagst(i);
    record(&[
        ("riool", net.naam.clone()),
        ("rol", "put".into()),
        ("put", p.naam.clone()),
        ("stelsel", p.stelsel.code().into()),
        ("x", m(p.xy[0])),
        ("y", m(p.xy[1])),
        ("maaiveld", p.maaiveld.map(m).unwrap_or_default()),
        ("maaiveld_vast", if p.maaiveld_vast { "ja".into() } else { String::new() }),
        ("bob_vast", p.bob_vast.map(m).unwrap_or_default()),
        ("bob_laagst", low.map(m).unwrap_or_default()),
        ("diepte", p.maaiveld.zip(low).map(|(g, b)| m(g - b)).unwrap_or_default()),
        ("afmeting", p.maat_text()),
        ("vorm", if p.maat[1] > 0.0 { "rechthoekig".into() } else { "rond".into() }),
        ("type", "Inspectieput".into()),
        ("bestaand", if p.bestaand { "ja".into() } else { String::new() }),
        ("referentie", referentie.code().into()),
    ])
}

fn leiding_record(net: &Net, i: usize, auto: bool) -> ExtendedDataRecord {
    let l = &net.leidingen[i];
    let cover = net.dekking(i);
    let at = |x: f64| cover.iter().min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs())).map(|c| m(c.1)).unwrap_or_default();
    let len = net.lengte(i);
    record(&[
        ("riool", net.naam.clone()),
        ("rol", "leiding".into()),
        ("leiding", net.naam_leiding(i)),
        ("van", net.putten[l.van].naam.clone()),
        ("naar", net.putten[l.naar].naam.clone()),
        ("stelsel", l.stelsel.code().into()),
        ("type", l.stelsel.gwsw_leiding().into()),
        ("diameter", format!("{:.0}", l.dn)),
        ("materiaal", l.materiaal.code().into()),
        ("wanddikte", format!("{:.1}", l.materiaal.wand(l.dn))),
        ("binnendiameter", format!("{:.1}", l.materiaal.binnen(l.dn))),
        ("bob_van", format!("{:.4}", l.bob_van)),
        ("bob_naar", format!("{:.4}", l.bob_naar)),
        ("lengte", m(len)),
        ("verhang", format!("{:.6}", net.verhang(i))),
        ("dekking_van", at(0.0)),
        ("dekking_naar", at(len)),
        ("dekking_min", m(l.dekking_min)),
        ("verhang_min", format!("{:.6}", l.verhang_min)),
        ("verhang_auto", if auto { "ja".into() } else { "nee".into() }),
        ("bestaand", if l.bestaand { "ja".into() } else { String::new() }),
    ])
}

fn styled(mut e: EntityType, spec: &LayerSpec, k: f64, data: Option<&ExtendedDataRecord>) -> EntityType {
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = Color::ByLayer;
    c.linetype_scale = k;
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

fn pline(pts: Vec<XY>, closed: bool, spec: &LayerSpec, k: f64, data: Option<&ExtendedDataRecord>) -> EntityType {
    crate::emit::polylines(&[(pts, closed)], &spec.name, k, data).remove(0)
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

/// The network in plan: pipes from manhole centre to centre, manholes at
/// their true size, labels (manhole number, ground level, BOB at both pipe
/// ends, size, material, length and slope per pipe) and flow arrows.
fn draw_plan(ctx: &Ctx, net: &Net, g: &Gelezen, labels: bool, out: &mut Uit) -> Result<(), String> {
    let (f, k) = (ctx.f, ctx.k);
    let tag = record(&[("riool", net.naam.clone()), ("rol", "label".into())]);
    let tag = Some(&tag);
    let label_layer = |s: Stelsel, what: &str| ctx.layer("RI", &format!("{}_{what}", s.code()), Element::T);
    for (i, l) in net.leidingen.iter().enumerate() {
        let spec = ctx.layer("RI", &leiding_object(l.stelsel, l.materiaal, l.dn), Element::G)?;
        let (pa, pb) = (&net.putten[l.van], &net.putten[l.naar]);
        let (a, b) = (ctx.to_drawing(pa.xy), ctx.to_drawing(pb.xy));
        let data = leiding_record(net, i, g.verhang_auto.get(i).copied().unwrap_or(true));
        out.put(&spec, vec![pline(vec![a, b], false, &spec, k, Some(&data))], Tag::Leiding(i));
        if !labels {
            continue;
        }
        let len = dist(a, b);
        if len < 1e-9 {
            continue;
        }
        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let ang = dir[1].atan2(dir[0]);
        let rot = crate::emit::readable(ang);
        let left = [-dir[1], dir[0]];
        let at = |s: f64, side: f64| [a[0] + dir[0] * s + left[0] * side, a[1] + dir[1] * s + left[1] * side];
        // Flow arrow in the middle of the pipe.
        let arrow = ctx.layer("RI", &format!("{}_STROMINGSRICHTING", l.stelsel.code()), Element::G)?;
        let tip = at(len / 2.0 + 1.5 * k, 0.0);
        let back = at(len / 2.0 - 1.5 * k, 0.0);
        let w = 1.0 * k;
        out.put(&arrow, vec![pline(vec![[back[0] + left[0] * w, back[1] + left[1] * w], tip, [back[0] - left[0] * w, back[1] - left[1] * w]], false, &arrow, k, tag)], Tag::Overig);
        // Size, material, length and slope beside the middle, right of the flow.
        let pipe_t = ctx.layer("RI", &leiding_object(l.stelsel, l.materiaal, l.dn), Element::T)?;
        let text = format!("{}  L={} m  i={}", l.label(), dec(net.lengte(i), 2), verhang_text(net.verhang(i)));
        out.put(&pipe_t, vec![ctx.text(at(len / 2.0, -2.2 * k), &text, 1.8, rot, &pipe_t, tag)], Tag::Overig);
        // BOB at both ends, left of the flow.
        let bob_t = label_layer(l.stelsel, "BOB")?;
        let ra = pa.straal() * f + 8.0 * k;
        let rb = pb.straal() * f + 8.0 * k;
        if len > ra + rb {
            out.put(&bob_t, vec![ctx.text(at(ra, 1.6 * k), &format!("{} {}", tr("BOB", "IL"), nap(l.bob_van, 2)), 1.8, rot, &bob_t, tag)], Tag::Overig);
            out.put(&bob_t, vec![ctx.text(at(len - rb, 1.6 * k), &format!("{} {}", tr("BOB", "IL"), nap(l.bob_naar, 2)), 1.8, rot, &bob_t, tag)], Tag::Overig);
        }
    }
    for (i, p) in net.putten.iter().enumerate() {
        let spec = ctx.layer("RI", &put_object(p.stelsel), Element::G)?;
        let c = ctx.to_drawing(p.xy);
        let data = put_record(net, i, g.referentie);
        // The direction of its pipes, for a rectangular manhole and the labels.
        let dir = net
            .outgoing(i)
            .chain(net.incoming(i))
            .next()
            .map(|j| {
                let l = &net.leidingen[j];
                let (a, b) = (net.putten[l.van].xy, net.putten[l.naar].xy);
                (b[1] - a[1]).atan2(b[0] - a[0])
            })
            .unwrap_or(0.0);
        let e = if p.maat[1] > 0.0 {
            let (hw, hl) = (p.maat[0] / 2000.0 * f, p.maat[1] / 2000.0 * f);
            let (u, v) = ([dir.cos(), dir.sin()], [-dir.sin(), dir.cos()]);
            let corner = |a: f64, b: f64| [c[0] + u[0] * a + v[0] * b, c[1] + u[1] * a + v[1] * b];
            pline(vec![corner(-hl, -hw), corner(hl, -hw), corner(hl, hw), corner(-hl, hw)], true, &spec, k, Some(&data))
        } else {
            styled(EntityType::Circle(Circle::from_center_radius(Vector3::new(c[0], c[1], 0.0), p.maat[0] / 2000.0 * f)), &spec, 1.0, Some(&data))
        };
        out.put(&spec, vec![e], Tag::Put(i));
        if !labels {
            continue;
        }
        // Number on the left of the pipes, ground level on the right, both
        // horizontal and clear of the manhole.
        let n = [-dir.sin(), dir.cos()];
        let r = p.straal() * f;
        let place = |side: f64, half_w: f64, half_h: f64| {
            let d = r + 1.0 * k + n[0].abs() * half_w + n[1].abs() * half_h;
            [c[0] + n[0] * d * side, c[1] + n[1] * d * side]
        };
        let nr_t = label_layer(p.stelsel, "PUTNUMMER")?;
        let w_nr = p.naam.chars().count() as f64 * 2.5 * 0.4 * k;
        out.put(&nr_t, vec![ctx.text(place(1.0, w_nr, 1.25 * k), &p.naam, 2.5, 0.0, &nr_t, tag)], Tag::Overig);
        if let Some(z) = p.maaiveld {
            let mv_t = label_layer(p.stelsel, "PUTDEKSELNIVEAU")?;
            let s = format!("{} {}", tr("mv", "GL"), nap(z, 2));
            let w = s.chars().count() as f64 * 1.8 * 0.4 * k;
            out.put(&mv_t, vec![ctx.text(place(-1.0, w, 0.9 * k), &s, 1.8, 0.0, &mv_t, tag)], Tag::Overig);
        }
    }
    Ok(())
}

/// All handles of a network, and those of its manholes and pipes by index.
type NetHandles = (Vec<Handle>, Vec<Option<Handle>>, Vec<Option<Handle>>);

/// Draw a network (replacing `old` entities) and index it. Returns the
/// handles of its manholes and pipes.
fn write_net(ctx: &Ctx, g: &Gelezen, labels: bool, old: &[Handle], label: &str) -> Result<NetHandles, String> {
    let mut out = Uit::default();
    draw_plan(ctx, &g.net, g, labels, &mut out)?;
    if !old.is_empty() {
        bridge::remove_entities(&t!("{} (oud)", "{} (old)", label), old)?;
    }
    let (handles, pairs) = out.write(label)?;
    bridge::index_add("riool", &g.net.naam, &handles, true);
    let mut puts = vec![None; g.net.putten.len()];
    let mut pipes = vec![None; g.net.leidingen.len()];
    for (t, h) in pairs {
        match t {
            Tag::Put(i) => puts[i] = Some(h),
            Tag::Leiding(i) => pipes[i] = Some(h),
            Tag::Overig => {}
        }
    }
    Ok((handles, puts, pipes))
}

/// The outcome of a network for scripts (ID=).
fn note_net(net: &Net, puts: &[Option<Handle>], pipes: &[Option<Handle>], warnings: &[String], source: &str) {
    let h = |x: &Option<Handle>| x.map(handle_text).unwrap_or_default();
    let putten: Vec<Value> = net
        .putten
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let low = net.bob_laagst(i);
            json!({"naam": p.naam, "x": p.xy[0], "y": p.xy[1], "maaiveld": p.maaiveld, "bob_laagst": low, "diepte": p.maaiveld.zip(low).map(|(g, b)| g - b), "afmeting": p.maat_text(), "bestaand": p.bestaand, "handle": h(&puts.get(i).copied().flatten())})
        })
        .collect();
    let leidingen: Vec<Value> = net
        .leidingen
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let cover = net.dekking(i);
            let min_cover = cover.iter().map(|c| c.1).reduce(f64::min);
            let low = l.grond.iter().copied().min_by(|a, b| a.1.total_cmp(&b.1));
            json!({
                "maaiveld_laagst": low.map(|g| g.1), "maaiveld_laagst_op": low.map(|g| g.0),
                "naam": net.naam_leiding(i), "van": net.putten[l.van].naam, "naar": net.putten[l.naar].naam,
                "stelsel": l.stelsel.code(), "diameter": l.dn, "materiaal": l.materiaal.code(), "binnendiameter": l.materiaal.binnen(l.dn),
                "wanddikte": l.materiaal.wand(l.dn), "lengte": net.lengte(i), "verhang": net.verhang(i), "verhang_tekst": verhang_text(net.verhang(i)),
                "bob_van": l.bob_van, "bob_naar": l.bob_naar, "dekking_min": min_cover, "dekking_eis": l.dekking_min, "verhang_eis": l.verhang_min,
                "bestaand": l.bestaand, "handle": h(&pipes.get(i).copied().flatten()),
            })
        })
        .collect();
    api::note("riool", net.naam.clone());
    api::note("stelsel", net.stelsel().code());
    api::note("putten", putten);
    api::note("leidingen", leidingen);
    api::note("waarschuwingen", warnings.to_vec());
    api::note("maaiveld", source);
}

/// A short summary of a computed network.
fn summary(net: &Net) -> String {
    let len: f64 = (0..net.leidingen.len()).map(|i| net.lengte(i)).sum();
    let bobs: Vec<f64> = net.leidingen.iter().flat_map(|l| [l.bob_van, l.bob_naar]).collect();
    let (lo, hi) = bobs.iter().fold((f64::MAX, f64::MIN), |a, b| (a.0.min(*b), a.1.max(*b)));
    let depth = (0..net.putten.len()).filter_map(|i| net.putten[i].maaiveld.zip(net.bob_laagst(i)).map(|(g, b)| g - b)).reduce(f64::max);
    t!(
        "{} putten, {} leidingen, {} m; BOB {} tot {}{}",
        "{} manholes, {} pipes, {} m; invert {} to {}{}",
        net.putten.len(),
        net.leidingen.len(),
        dec(len, 1),
        nap(hi.min(1e9), 2),
        nap(lo.max(-1e9), 2),
        depth.map(|d| t!(", diepste put {} m", ", deepest manhole {} m", dec(d, 2))).unwrap_or_default()
    )
}

/// The number of a network named "Riool n" (or "Sewer n").
fn net_number(name: &str) -> Option<usize> {
    name.strip_prefix("Riool ").or_else(|| name.strip_prefix("Sewer ")).and_then(|v| v.trim().parse::<usize>().ok())
}

/// The next free "Riool n" ("Sewer n").
fn next_name(nets: &[Gelezen]) -> String {
    let mut n = 0;
    for name in nets.iter().map(|g| g.net.naam.clone()).chain(bridge::index_read().into_iter().filter(|(k, _, _)| k == "riool").map(|(_, n, _)| n)) {
        if let Some(v) = net_number(&name) {
            n = n.max(v);
        }
    }
    format!("{} {}", tr("Riool", "Sewer"), n + 1)
}

// ── CIV_RIOOL: design a run ─────────────────────────────────────────────────

/// Where the run comes from.
pub enum Invoer {
    /// A polyline or line: by handle, or the selected one.
    Lijn(Option<Handle>),
    /// Picked points (drawing coordinates).
    Punten(Vec<XY>),
    /// Points in project coordinates (scripts).
    Project(Vec<XY>),
}

/// The design choices of `CIV_RIOOL` (arguments; defaults where `None`).
#[derive(Clone, Debug)]
pub struct Ontwerp {
    pub stelsel: Stelsel,
    pub dn: Option<f64>,
    pub materiaal: Option<Materiaal>,
    pub dekking: Option<f64>,
    pub verhang: Option<f64>,
    pub bob: Option<f64>,
    pub put: Option<[f64; 2]>,
    pub putnr: Option<String>,
    pub naam: Option<String>,
    pub omkeren: bool,
    pub referentie: Referentie,
}

pub const ONTWERP_ARGS: &[&str] = &["LIJN", "PUNTEN", "STELSEL", "DIAMETER", "MATERIAAL", "DEKKING", "VERHANG", "BOB", "PUT", "PUTNR", "NAAM", "OMKEREN", "MAAIVELD"];

fn ja(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.to_ascii_lowercase()).as_deref(), Some("ja" | "yes" | "1" | "true" | "aan"))
}

impl Ontwerp {
    pub fn from_args(a: &api::Args) -> Result<Ontwerp, String> {
        let stelsel = match a.str("STELSEL") {
            Some(s) => Stelsel::parse(s).ok_or(tr("STELSEL: DWA, HWA, GWA of ITR", "STELSEL: DWA, HWA, GWA or ITR"))?,
            None => Stelsel::Dwa,
        };
        let materiaal = a.str("MATERIAAL").map(|s| Materiaal::parse(s).ok_or(tr("MATERIAAL: beton, PVC, PP, PE of gres", "MATERIAAL: beton (concrete), PVC, PP, PE or gres (vitrified clay)"))).transpose()?;
        let bob = match a.str("BOB") {
            Some(s) if s.eq_ignore_ascii_case("auto") => None,
            Some(_) => a.f64("BOB")?,
            None => None,
        };
        let dn = a.f64("DIAMETER")?;
        if dn.is_some_and(|d| !(100.0..=4000.0).contains(&d)) {
            return Err(tr("DIAMETER in mm, 100 tot 4000", "DIAMETER in mm, 100 to 4000").into());
        }
        let dekking = a.f64("DEKKING")?;
        if dekking.is_some_and(|d| !(0.0..=10.0).contains(&d)) {
            return Err(tr("DEKKING in m, 0 tot 10", "DEKKING in m, 0 to 10").into());
        }
        Ok(Ontwerp {
            stelsel,
            dn,
            materiaal,
            dekking,
            verhang: a.str("VERHANG").map(parse_verhang).transpose()?,
            bob,
            put: a.str("PUT").map(parse_maat).transpose()?,
            putnr: a.str("PUTNR").map(str::to_string),
            naam: a.str("NAAM").map(str::to_string),
            omkeren: ja(a.str("OMKEREN")),
            referentie: match a.str("MAAIVELD") {
                Some(s) => Referentie::parse(s).ok_or(tr("MAAIVELD: AUTO (TIN, anders AHN), ONTWERP (ontwerp-TIN eerst) of AHN", "MAAIVELD: AUTO (TIN, else the height source), ONTWERP (design TIN first) or AHN"))?,
                None => Referentie::Auto,
            },
        })
    }
}

impl Default for Ontwerp {
    fn default() -> Self {
        Ontwerp { stelsel: Stelsel::Dwa, dn: None, materiaal: None, dekking: None, verhang: None, bob: None, put: None, putnr: None, naam: None, omkeren: false, referentie: Referentie::Auto }
    }
}

/// The plan vertices of the input (project coordinates).
fn input_points(ctx: &Ctx, doc: &CadDocument, input: Invoer) -> Result<Vec<XY>, String> {
    let pts: Vec<XY> = match input {
        Invoer::Project(p) => p,
        Invoer::Punten(p) => p.into_iter().map(|q| ctx.from_drawing(q)).collect(),
        Invoer::Lijn(h) => {
            let e = match h {
                Some(h) => doc.get_entity(h).ok_or_else(|| t!("geen element met handle {}", "no entity with handle {}", handle_text(h)))?,
                None => {
                    let sel: HashSet<Handle> = bridge::selection()?.into_iter().collect();
                    let found: Vec<&EntityType> = doc.model_space_entities().filter(|e| sel.contains(&e.common().handle) && vertices(e).len() >= 2).collect();
                    match found.len() {
                        1 => found[0],
                        0 => return Err(tr("selecteer een polylijn met de putten op de hoekpunten, of geef LIJN=<handle> of PUNTEN=x,y;x,y;…", "select a polyline with the manholes at its vertices, or give LIJN=<handle> or PUNTEN=x,y;x,y;…").into()),
                        n => return Err(t!("{} lijnen geselecteerd: selecteer er één", "{} lines selected: select one", n)),
                    }
                }
            };
            if matches!(e, EntityType::LwPolyline(p) if p.vertices.iter().any(|v| v.bulge.abs() > 1e-9)) {
                bridge::info(tr("Riolering: bogen in de polylijn worden recht tussen de putten gelegd.", "Sewers: arcs in the polyline become straight pipes between the manholes."));
            }
            vertices(e).into_iter().map(|(p, _)| ctx.from_drawing(p)).collect()
        }
    };
    let mut out: Vec<XY> = Vec::new();
    for p in pts {
        if out.last().is_none_or(|q| dist(*q, p) > 0.01) {
            out.push(p);
        }
    }
    if out.len() < 2 {
        return Err(tr("een riool heeft minstens twee putten nodig", "a sewer needs at least two manholes").into());
    }
    Ok(out)
}

/// A manhole of another network at this point (within its outline, at least 0.5 m).
fn put_near(nets: &[Gelezen], p: XY) -> Option<(usize, usize)> {
    let mut best: Option<(f64, usize, usize)> = None;
    for (n, g) in nets.iter().enumerate() {
        for (i, q) in g.net.putten.iter().enumerate() {
            let d = dist(q.xy, p);
            if d <= q.straal().max(0.5) && best.is_none_or(|b| d < b.0) {
                best = Some((d, n, i));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// Start number for new manholes in a network: after the highest "P<n>".
fn next_putnr(net: &Net, netnr: usize) -> String {
    let max = net.putten.iter().filter_map(|p| p.naam.strip_prefix('P').and_then(|n| n.parse::<u64>().ok())).max();
    match max {
        Some(m) => format!("P{}", m + 1),
        None => format!("P{}", netnr * 100 + 1),
    }
}

/// `CIV_RIOOL`: a run of manholes and pipes along a polyline or points, its
/// BOB computed downstream, drawn on NLCS RI layers with labels. A run that
/// ends (or starts) in a manhole of another network joins that network.
pub fn ontwerp(opt: &super::CivielOptions, input: Invoer, o: Ontwerp, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let extra: Vec<Handle> = match &input {
        Invoer::Lijn(Some(h)) => vec![*h],
        _ => Vec::new(),
    };
    let doc = super::document(&["riool", "tin"], &extra)?;
    let mut pts = input_points(&ctx, &doc, input)?;
    if o.omkeren {
        pts.reverse();
    }
    let mut nets = lees(&doc, &ctx);
    // Joining an existing network at the last point (or the first).
    let join_end = put_near(&nets, *pts.last().unwrap());
    let join_start = put_near(&nets, pts[0]);
    let joined = join_end.or(join_start).map(|j| j.0);
    let (mut g, old) = match joined {
        Some(n) => {
            let g = nets.remove(n);
            let old = g.handles.clone();
            (g, old)
        }
        None => {
            let naam = o.naam.clone().unwrap_or_else(|| next_name(&nets));
            if nets.iter().any(|g| g.net.naam == naam) {
                return Err(t!("er is al een riool {}: kies een andere NAAM= of werk het bij met CIV_RIOOL_BIJWERKEN", "there is already a sewer {}: choose another NAAM= or update it with CIV_RIOOL_BIJWERKEN", naam));
            }
            let g = Gelezen { net: Net { naam, putten: Vec::new(), leidingen: Vec::new() }, handles: Vec::new(), put_handles: Vec::new(), leiding_handles: Vec::new(), changes: Vec::new(), referentie: o.referentie, verhang_auto: Vec::new() };
            (g, Vec::new())
        }
    };
    if joined.is_some() && o.naam.as_ref().is_some_and(|n| *n != g.net.naam) {
        return Err(t!("het riool sluit aan op {}: laat NAAM= weg of kies een begin- en eindpunt los van andere putten", "the sewer joins {}: leave out NAAM= or start and end away from other manholes", g.net.naam));
    }
    let netnr = net_number(&g.net.naam).unwrap_or(nets.len() + 1);
    let dn = o.dn.unwrap_or_else(|| standaard_diameter(o.stelsel));
    let mat = o.materiaal.unwrap_or_else(|| standaard_materiaal(dn));
    let maat = o.put.unwrap_or_else(|| standaard_put(dn));
    // Manholes: the joined ones are reused, the others are new.
    let first_new = g.net.putten.len();
    let start = o.putnr.clone().unwrap_or_else(|| next_putnr(&g.net, netnr));
    let existing = |p: XY, g: &Gelezen| g.net.putten.iter().position(|q| dist(q.xy, p) <= q.straal().max(0.5));
    let n_new = pts.iter().filter(|p| joined.is_none() || existing(**p, &g).is_none()).count();
    let taken: HashSet<String> = g.net.putten.iter().map(|p| p.naam.clone()).collect();
    let mut names = putnummers(&start, n_new + taken.len()).into_iter().filter(|n| !taken.contains(n)).collect::<Vec<_>>().into_iter();
    let mut idx = Vec::with_capacity(pts.len());
    for p in &pts {
        let found = if joined.is_some() { existing(*p, &g) } else { None };
        match found {
            Some(i) => idx.push(i),
            None => {
                let name = names.next().ok_or(tr("te weinig putnummers", "too few manhole numbers"))?;
                g.net.putten.push(Put::new(name, *p, o.stelsel, maat));
                g.put_handles.push(None);
                idx.push(g.net.putten.len() - 1);
            }
        }
    }
    if let (Some(b), true) = (o.bob, idx[0] >= first_new) {
        g.net.putten[idx[0]].bob_vast = Some(b);
    }
    for w in idx.windows(2) {
        if w[0] == w[1] {
            continue;
        }
        let mut l = Leiding::new(w[0], w[1], dn, mat, o.stelsel);
        if let Some(c) = o.dekking {
            l.dekking_min = c;
        }
        if let Some(s) = o.verhang {
            l.verhang_min = s;
        }
        g.net.leidingen.push(l);
        g.leiding_handles.push(None);
        g.verhang_auto.push(o.verhang.is_none());
    }
    progress(t!("Maaiveld langs {} …", "Ground level along {} …", g.net.naam));
    let mut ground = Grond::new(&ctx, &doc, if joined.is_some() { g.referentie } else { o.referentie });
    g.referentie = if joined.is_some() { g.referentie } else { o.referentie };
    let step = opt.step.max(0.5);
    let notes = g.net.sample_ground(&mut |p| ground.height(p), step);
    if g.net.putten.iter().any(|p| p.maaiveld.is_none()) && g.net.leidingen.iter().all(|l| l.grond.is_empty()) {
        return Err(t!("geen maaiveld langs het riool: {}", "no ground level along the sewer: {}", ground.error()));
    }
    let mut warnings = notes;
    warnings.extend(g.net.bereken()?);
    let label = g.net.naam.clone();
    let (handles, puts, pipes) = write_net(&ctx, &g, opt.point_labels, &old, &label)?;
    ground.note_source();
    note_net(&g.net, &puts, &pipes, &warnings, &ground.label());
    api::note("aangesloten", joined.is_some());
    let what = if joined.is_some() { t!("aangesloten op {}", "joined to {}", g.net.naam) } else { g.net.naam.clone() };
    let mut msg = t!(
        "{} ({}, {}): {} — maaiveld {}; {} elementen.",
        "{} ({}, {}): {} — ground level {}; {} entities.",
        what,
        g.net.stelsel().label(),
        Leiding::new(0, 0, dn, mat, o.stelsel).label(),
        summary(&g.net),
        ground.label(),
        handles.len()
    );
    if !warnings.is_empty() {
        msg.push_str(&t!(" Let op: {}.", " Note: {}.", warnings.join("; ")));
    }
    Ok(msg)
}

// ── CIV_RIOOL_BIJWERKEN: edit and recompute ─────────────────────────────────

/// Edits by argument: one manhole (`PUTNR=`), one pipe (`LEIDING=`) or, for
/// the pipe properties, the whole network.
#[derive(Clone, Debug, Default)]
pub struct Wijziging {
    pub put: Option<String>,
    pub leiding: Option<String>,
    pub dn: Option<f64>,
    pub materiaal: Option<Materiaal>,
    pub stelsel: Option<Stelsel>,
    pub dekking: Option<f64>,
    pub verhang: Option<f64>,
    /// `Some(None)`: back to automatic.
    pub bob: Option<Option<f64>>,
    pub maaiveld: Option<Option<f64>>,
    pub maat: Option<[f64; 2]>,
    pub referentie: Option<Referentie>,
    /// A new manhole on the nearest pipe (project coordinates).
    pub invoegen: Option<XY>,
}

pub const BIJWERKEN_ARGS: &[&str] = &["RIOOL", "PUTNR", "LEIDING", "DIAMETER", "MATERIAAL", "STELSEL", "DEKKING", "VERHANG", "BOB", "MAAIVELD", "PUT", "INVOEGEN", "REFERENTIE"];

impl Wijziging {
    pub fn from_args(a: &api::Args) -> Result<Wijziging, String> {
        let level = |k: &str| -> Result<Option<Option<f64>>, String> {
            match a.str(k) {
                None => Ok(None),
                Some(s) if s.eq_ignore_ascii_case("auto") => Ok(Some(None)),
                Some(_) => Ok(Some(a.f64(k)?)),
            }
        };
        Ok(Wijziging {
            put: a.str("PUTNR").map(str::to_string),
            leiding: a.str("LEIDING").map(str::to_string),
            dn: a.f64("DIAMETER")?,
            materiaal: a.str("MATERIAAL").map(|s| Materiaal::parse(s).ok_or(tr("MATERIAAL: beton, PVC, PP, PE of gres", "MATERIAAL: beton (concrete), PVC, PP, PE or gres (vitrified clay)"))).transpose()?,
            stelsel: a.str("STELSEL").map(|s| Stelsel::parse(s).ok_or(tr("STELSEL: DWA, HWA, GWA of ITR", "STELSEL: DWA, HWA, GWA or ITR"))).transpose()?,
            dekking: a.f64("DEKKING")?,
            verhang: a.str("VERHANG").map(parse_verhang).transpose()?,
            bob: level("BOB")?,
            maaiveld: level("MAAIVELD")?,
            maat: a.str("PUT").map(parse_maat).transpose()?,
            referentie: a.str("REFERENTIE").map(|s| Referentie::parse(s).ok_or(tr("REFERENTIE: AUTO, ONTWERP of AHN", "REFERENTIE: AUTO, ONTWERP or AHN"))).transpose()?,
            invoegen: a.points("INVOEGEN")?.and_then(|p| p.first().copied()),
        })
    }

    fn is_empty(&self) -> bool {
        self.dn.is_none() && self.materiaal.is_none() && self.stelsel.is_none() && self.dekking.is_none() && self.verhang.is_none() && self.bob.is_none() && self.maaiveld.is_none() && self.maat.is_none() && self.referentie.is_none() && self.invoegen.is_none()
    }
}

/// Apply argument edits to a network; returns what changed.
fn apply(g: &mut Gelezen, w: &Wijziging) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    let net = &mut g.net;
    if let Some(at) = w.invoegen {
        // Split the nearest design pipe at the foot of the point.
        let mut best: Option<(f64, usize, f64)> = None;
        for (i, l) in net.leidingen.iter().enumerate() {
            let (a, b) = (net.putten[l.van].xy, net.putten[l.naar].xy);
            let len = dist(a, b);
            if len < 1e-6 {
                continue;
            }
            let t = (((at[0] - a[0]) * (b[0] - a[0]) + (at[1] - a[1]) * (b[1] - a[1])) / (len * len)).clamp(0.0, 1.0);
            let foot = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let d = dist(foot, at);
            if t > 0.01 && t < 0.99 && best.is_none_or(|x| d < x.0) {
                best = Some((d, i, t));
            }
        }
        let (d, i, t) = best.ok_or(tr("INVOEGEN: geen leiding om een put in te zetten", "INVOEGEN: no pipe to put a manhole in"))?;
        if d > 5.0 {
            return Err(t!("INVOEGEN: de dichtstbijzijnde leiding ligt {} m verderop", "INVOEGEN: the nearest pipe is {} m away", dec(d, 1)));
        }
        let l = net.leidingen[i].clone();
        let (a, b) = (net.putten[l.van].xy, net.putten[l.naar].xy);
        let xy = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let name = w.put.clone().filter(|n| !net.putten.iter().any(|p| p.naam == *n)).unwrap_or_else(|| {
            let base = net.putten[l.van].naam.clone();
            (1..).map(|k| format!("{base}.{k}")).find(|n| !net.putten.iter().any(|p| p.naam == *n)).unwrap()
        });
        let up = &net.putten[l.van];
        let mut p = Put::new(name.clone(), xy, l.stelsel, up.maat);
        p.bestaand = false;
        net.putten.push(p);
        g.put_handles.push(None);
        let new = net.putten.len() - 1;
        let mut tail = l.clone();
        tail.van = new;
        net.leidingen[i].naar = new;
        net.leidingen[i].bestaand = false;
        tail.bestaand = false;
        net.leidingen.push(tail);
        g.leiding_handles.push(None);
        let auto = g.verhang_auto.get(i).copied().unwrap_or(true);
        g.verhang_auto.push(auto);
        done.push(t!("put {} ingevoegd", "manhole {} inserted", name));
    }
    let put = match &w.put {
        Some(n) if w.invoegen.is_none() => Some(net.putten.iter().position(|p| p.naam == *n).ok_or_else(|| t!("PUTNR: geen put {} in {}", "PUTNR: no manhole {} in {}", n, net.naam))?),
        _ => None,
    };
    let pipes: Vec<usize> = match &w.leiding {
        Some(s) => {
            let found: Vec<usize> = (0..net.leidingen.len())
                .filter(|i| {
                    let name = net.naam_leiding(*i);
                    name.eq_ignore_ascii_case(s) || s.parse::<usize>().ok() == Some(*i + 1) || net.putten[net.leidingen[*i].van].naam.eq_ignore_ascii_case(s)
                })
                .collect();
            if found.is_empty() {
                return Err(t!("LEIDING: geen leiding {} in {} (gebruik van-naar, bijv. P101-P102, of het volgnummer)", "LEIDING: no pipe {} in {} (use from-to, e.g. P101-P102, or its number)", s, net.naam));
            }
            found
        }
        None if put.is_none() => (0..net.leidingen.len()).filter(|i| !net.leidingen[*i].bestaand).collect(),
        None => Vec::new(),
    };
    for &i in &pipes {
        let l = &mut net.leidingen[i];
        if let Some(d) = w.dn {
            l.dn = d;
            if g.verhang_auto.get(i).copied().unwrap_or(true) && w.verhang.is_none() {
                l.verhang_min = min_verhang(d);
            }
        }
        if let Some(mat) = w.materiaal {
            l.materiaal = mat;
        }
        if let Some(s) = w.stelsel {
            l.stelsel = s;
        }
        if let Some(c) = w.dekking {
            l.dekking_min = c;
        }
        if let Some(s) = w.verhang {
            l.verhang_min = s;
            if let Some(a) = g.verhang_auto.get_mut(i) {
                *a = false;
            }
        }
        if w.dn.is_some() || w.materiaal.is_some() || w.stelsel.is_some() || w.dekking.is_some() || w.verhang.is_some() {
            // An edited existing pipe becomes a design pipe.
            l.bestaand = false;
        }
    }
    if !pipes.is_empty() && (w.dn.is_some() || w.materiaal.is_some() || w.stelsel.is_some() || w.dekking.is_some() || w.verhang.is_some()) {
        done.push(t!("{} leiding(en) aangepast", "{} pipe(s) changed", pipes.len()));
    }
    match put {
        Some(p) => {
            let pt = &mut net.putten[p];
            if let Some(b) = w.bob {
                pt.bob_vast = b;
                done.push(t!("put {}: BOB {}", "manhole {}: invert {}", pt.naam, b.map(|v| nap(v, 2)).unwrap_or_else(|| tr("automatisch", "automatic").into())));
            }
            if let Some(z) = w.maaiveld {
                pt.maaiveld_vast = z.is_some();
                if z.is_some() {
                    pt.maaiveld = z;
                }
                done.push(t!("put {}: maaiveld {}", "manhole {}: ground level {}", pt.naam, z.map(|v| nap(v, 2)).unwrap_or_else(|| tr("uit TIN/AHN", "from the TIN or height source").into())));
            }
            if let Some(mt) = w.maat {
                pt.maat = mt;
                done.push(t!("put {}: afmeting {}", "manhole {}: size {}", pt.naam, pt.maat_text()));
            }
            if let Some(s) = w.stelsel {
                pt.stelsel = s;
            }
        }
        None => {
            if (w.bob.is_some() || w.maaiveld.is_some()) && w.invoegen.is_none() {
                return Err(tr("BOB= en MAAIVELD= horen bij één put: geef PUTNR=<putnummer>", "BOB= and MAAIVELD= belong to one manhole: give PUTNR=<number>").into());
            }
            if let Some(mt) = w.maat {
                for p in net.putten.iter_mut().filter(|p| !p.bestaand) {
                    p.maat = mt;
                }
                done.push(t!("putten: afmeting {}", "manholes: size {}", if mt[1] > 0.0 { format!("{:.0}x{:.0}", mt[0], mt[1]) } else { format!("{:.0}", mt[0]) }));
            }
            if let Some(s) = w.stelsel {
                for p in net.putten.iter_mut().filter(|p| !p.bestaand) {
                    p.stelsel = s;
                }
            }
        }
    }
    if let Some(r) = w.referentie {
        g.referentie = r;
        done.push(t!("maaiveld uit {}", "ground level from {}", r.code()));
    }
    Ok(done)
}

/// Which networks a command works on: by name, else those with a selected
/// entity, else the only one (or all, when `all` is true).
fn choose(nets: Vec<Gelezen>, naam: Option<&str>, all: bool) -> Result<Vec<Gelezen>, String> {
    if nets.is_empty() {
        return Err(tr("geen riool in de tekening: ontwerp er een met CIV_RIOOL", "no sewer in the drawing: design one with CIV_RIOOL").into());
    }
    if let Some(n) = naam {
        let found: Vec<Gelezen> = nets.into_iter().filter(|g| g.net.naam.eq_ignore_ascii_case(n)).collect();
        if found.is_empty() {
            return Err(t!("geen riool {} in de tekening", "no sewer {} in the drawing", n));
        }
        return Ok(found);
    }
    let sel: HashSet<Handle> = bridge::selection().unwrap_or_default().into_iter().collect();
    if !sel.is_empty() {
        let picked: Vec<usize> = nets.iter().enumerate().filter(|(_, g)| g.handles.iter().any(|h| sel.contains(h))).map(|(i, _)| i).collect();
        if !picked.is_empty() {
            return Ok(nets.into_iter().enumerate().filter(|(i, _)| picked.contains(i)).map(|(_, g)| g).collect());
        }
    }
    if nets.len() == 1 || all {
        return Ok(nets);
    }
    let names: Vec<String> = nets.iter().map(|g| g.net.naam.clone()).collect();
    Err(t!("er zijn {} riolen ({}): selecteer er een of geef RIOOL=<naam>", "there are {} sewers ({}): select one or give RIOOL=<name>", names.len(), names.join(", ")))
}

/// `CIV_RIOOL_BIJWERKEN`: read the network back from the drawing (moved
/// manholes, dragged pipe ends, deleted manholes, edited object data),
/// apply argument edits, recompute the BOB and draw it again; its length
/// profile follows.
pub fn bijwerken(opt: &super::CivielOptions, naam: Option<String>, w: Wijziging, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["riool", "rioolprofiel", "riooltoets", "tin"], &[])?;
    let targeted = naam.is_some() || !w.is_empty();
    let chosen = choose(lees(&doc, &ctx), naam.as_deref(), !targeted)?;
    if chosen.len() > 1 && !w.is_empty() && (w.put.is_some() || w.leiding.is_some()) {
        return Err(tr("PUTNR= en LEIDING= gelden voor één riool: geef RIOOL=<naam>", "PUTNR= and LEIDING= apply to one sewer: give RIOOL=<name>").into());
    }
    let mut msgs = Vec::new();
    let mut all_warnings = Vec::new();
    for mut g in chosen {
        progress(t!("{} bijwerken …", "Updating {} …", g.net.naam));
        let mut changes = std::mem::take(&mut g.changes);
        changes.extend(apply(&mut g, &w)?);
        let mut ground = Grond::new(&ctx, &doc, g.referentie);
        let mut warnings = g.net.sample_ground(&mut |p| ground.height(p), opt.step.max(0.5));
        warnings.extend(g.net.bereken()?);
        // Old check markers no longer fit.
        let stale: Vec<Handle> = doc.model_space_entities().filter(|e| data_value(e, "riooltoets").as_deref() == Some(g.net.naam.as_str())).map(|e| e.common().handle).collect();
        if !stale.is_empty() {
            let _ = bridge::remove_entities(tr("Riooltoets (oud)", "Sewer check (old)"), &stale);
        }
        let old = g.handles.clone();
        let (handles, puts, pipes) = write_net(&ctx, &g, opt.point_labels, &old, &t!("{} bijwerken", "Update {}", g.net.naam))?;
        ground.note_source();
        // The length profile follows, in its place.
        let profile = redraw_profile(&ctx, &doc, &g, opt, &mut ground)?;
        note_net(&g.net, &puts, &pipes, &warnings, &ground.label());
        api::note("wijzigingen", changes.clone());
        msgs.push(t!(
            "{} bijgewerkt: {}{}{} — {} elementen.",
            "{} updated: {}{}{} — {} entities.",
            g.net.naam,
            summary(&g.net),
            if changes.is_empty() { String::new() } else { format!("; {}", changes.join(", ")) },
            profile.map(|p| t!("; lengteprofiel opnieuw ({})", "; length profile redrawn ({})", p)).unwrap_or_default(),
            handles.len()
        ));
        all_warnings.extend(warnings);
    }
    let mut msg = msgs.join(" ");
    if !all_warnings.is_empty() {
        msg.push_str(&t!(" Let op: {}.", " Note: {}.", all_warnings.join("; ")));
    }
    Ok(msg)
}

// ── CIV_RIOOL_PROFIEL: the length profile of a run ──────────────────────────

/// The length profile of `path` (pipes in flow order), in the style of the
/// alignment's length profile: ground line, pipes as double lines (invert
/// and inner crown), manholes from their bottom to the ground, a height grid
/// and a band with manhole numbers, ground level, BOB in and out, distance,
/// length and slope, size and material. `at` is the lower-left corner of the
/// graph (station 0 on the band's bottom line).
fn draw_profile(ctx: &Ctx, net: &Net, path: &[usize], ground: &mut Grond, opt: &super::CivielOptions, at: Option<XY>, out: &mut Out) -> Result<(XY, f64, f64), String> {
    let (f, k) = (ctx.f, ctx.k);
    let naam = net.naam.clone();
    // Stations of the manholes along the path.
    let mut puts = vec![net.leidingen[path[0]].van];
    let mut st = vec![0.0];
    for &i in path {
        puts.push(net.leidingen[i].naar);
        st.push(st.last().unwrap() + net.lengte(i));
    }
    let len = *st.last().unwrap();
    // Ground every step along the pipes.
    let step = opt.step.max(0.25);
    let mut samples: Vec<(f64, Option<f64>)> = Vec::new();
    for (j, &i) in path.iter().enumerate() {
        let l = &net.leidingen[i];
        let (a, b) = (net.putten[l.van].xy, net.putten[l.naar].xy);
        let pl = dist(a, b);
        let n = ((pl / step).ceil() as usize).clamp(1, 2000);
        for q in 0..=n {
            if q == 0 && j > 0 {
                continue;
            }
            let t = q as f64 / n as f64;
            let z = ground.height([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            samples.push((st[j] + pl * t, z));
        }
    }
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    for &i in path {
        let l = &net.leidingen[i];
        lo = lo.min(l.bob_van.min(l.bob_naar));
        hi = hi.max(l.bob_van.max(l.bob_naar) + l.d());
    }
    for z in samples.iter().filter_map(|s| s.1).chain(puts.iter().filter_map(|p| net.putten[*p].maaiveld)) {
        hi = hi.max(z);
        lo = lo.min(z);
    }
    if lo > hi {
        return Err(tr("geen hoogtes voor het lengteprofiel", "no heights for the length profile").into());
    }
    let v = opt.exaggeration.max(1.0);
    let hstep = nice_step(((hi - lo) / 6.0).max(0.1));
    let base = (lo / hstep).floor() * hstep - hstep;
    let top = (hi / hstep).ceil() * hstep + hstep;
    let rows: [(&str, f64); 6] = [(tr("Putnummer", "Manhole"), 8.0), (tr("Maaiveld (m)", "Ground level (m)"), 14.0), (tr("BOB (m)", "Invert (m)"), 16.0), (tr("Afstand (m)", "Distance (m)"), 14.0), (tr("Lengte, verhang", "Length, slope"), 8.0), (tr("Leiding", "Pipe"), 8.0)];
    let band: f64 = rows.iter().map(|r| r.1).sum::<f64>() * k;
    let head = 42.0 * k;
    let at = match at {
        Some(a) => a,
        None => {
            let size = [head + len * f + 10.0 * k, band + (top - base) * v * f + 20.0 * k];
            let b = net.putten.iter().map(|p| ctx.to_drawing(p.xy)).fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
            let area = match &ctx.site {
                Some(s) => {
                    let r = s.drawing_rect(f);
                    [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])]
                }
                None => b,
            };
            let spot = bridge::layout_spot("onder", size, area, 30.0 * k);
            [spot[0] + head, spot[1]]
        }
    };
    let gx = |s: f64| at[0] + s * f;
    let gy = |z: f64| at[1] + band + (z - base) * v * f;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let tag = record(&[("rioolprofiel", naam.clone()), ("rol", "profiel".into())]);
    let tag = Some(&tag);
    let pl = |pts: Vec<XY>, closed: bool, spec: &LayerSpec| pline(pts, closed, spec, k, tag);
    let (x0, x1, y0, y1) = (at[0] - head, gx(len), at[1], gy(top));
    let (van, naar) = (net.putten[puts[0]].naam.clone(), net.putten[*puts.last().unwrap()].naam.clone());
    let title = t!("Lengteprofiel {} ({} – {})", "Length profile {} ({} – {})", naam, van, naar);
    let frame_data = record(&[
        ("rioolprofiel", naam.clone()),
        ("rol", "kader".into()),
        ("van", van.clone()),
        ("naar", naar.clone()),
        ("plaats_x", format!("{:.6}", at[0])),
        ("plaats_y", format!("{:.6}", at[1])),
        ("x_nul", format!("{:.6}", gx(0.0))),
        ("y_basis", format!("{:.6}", gy(base))),
        ("basis", format!("{base:.4}")),
        ("overhoging", format!("{v}")),
        ("eenheid", format!("{f}")),
        ("lengte_m", format!("{len:.4}")),
    ]);
    out.put(&frame, [pline(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true, &frame, k, Some(&frame_data))]);
    // Band rows, top row first under the graph.
    let mut row_y = Vec::new();
    let mut y = y0 + band;
    for (title_r, h) in rows {
        let h = h * k;
        row_y.push((y - h, y));
        out.put(&frame, [pl(vec![[x0, y - h], [x1, y - h]], false, &frame)]);
        let t = ctx.text([x0 + 2.0 * k, y - h / 2.0], title_r, 1.8, 0.0, &frame_t, tag);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        y -= h;
    }
    out.put(&frame, [pl(vec![[x0, y0 + band], [x1, y0 + band]], false, &frame), pl(vec![[at[0], y0], [at[0], y1]], false, &frame)]);
    // Height grid.
    let mut z = base;
    while z <= top + 1e-9 {
        let yy = gy(z);
        if z > base + 1e-9 && z < top - 1e-9 {
            out.put(&frame, [pl(vec![[at[0], yy], [x1, yy]], false, &frame)]);
        }
        if z < top - 1e-9 {
            let t = ctx.text([at[0] - 2.0 * k, yy + 1.4 * k], &nap(z, 2), 1.8, 0.0, &frame_t, tag);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        }
        z += hstep;
    }
    let mid = |r: usize| (row_y[r].0 + row_y[r].1) / 2.0;
    let rot = FRAC_PI_2;
    // Manholes: band values and their shaft in the graph.
    for (j, &p) in puts.iter().enumerate() {
        let put = &net.putten[p];
        let x = gx(st[j]);
        // The manhole line through the band, under its number.
        out.put(&frame, [pl(vec![[x, y0], [x, row_y[0].0]], false, &frame)]);
        out.put(&frame_t, [ctx.text([x, mid(0)], &put.naam, 1.8, 0.0, &frame_t, tag)]);
        if let Some(g) = put.maaiveld {
            out.put(&frame_t, [ctx.text([x - 1.0 * k, mid(1)], &nap(g, 2), 1.8, rot, &frame_t, tag)]);
        }
        let bob_in = (j > 0).then(|| net.leidingen[path[j - 1]].bob_naar);
        let bob_out = (j < path.len()).then(|| net.leidingen[path[j]].bob_van);
        match (bob_in, bob_out) {
            (Some(a), Some(b)) if (a - b).abs() > 0.005 => {
                out.put(&frame_t, [ctx.text([x - 1.6 * k, mid(2)], &nap(a, 2), 1.8, rot, &frame_t, tag)]);
                out.put(&frame_t, [ctx.text([x + 2.4 * k, mid(2)], &nap(b, 2), 1.8, rot, &frame_t, tag)]);
            }
            (Some(a), _) | (None, Some(a)) => out.put(&frame_t, [ctx.text([x - 1.0 * k, mid(2)], &nap(a, 2), 1.8, rot, &frame_t, tag)]),
            _ => {}
        }
        out.put(&frame_t, [ctx.text([x - 1.0 * k, mid(3)], &dec(st[j], 2), 1.8, rot, &frame_t, tag)]);
        // The shaft, at true width, from its bottom to the ground.
        let low = [bob_in, bob_out].into_iter().flatten().reduce(f64::min).or(net.bob_laagst(p));
        if let (Some(low), Some(g)) = (low, put.maaiveld) {
            let spec = ctx.layer("RI", &put_object(put.stelsel), Element::G)?;
            let r = put.straal() * f;
            out.put(&spec, [pl(vec![[x - r, gy(low) - 0.15 * v * f], [x + r, gy(low) - 0.15 * v * f], [x + r, gy(g)], [x - r, gy(g)]], true, &spec)]);
        }
    }
    // Pipes: invert and inner crown between the shafts; length, slope, size.
    for (j, &i) in path.iter().enumerate() {
        let l = &net.leidingen[i];
        let pl_len = net.lengte(i);
        let (ra, rb) = (net.putten[l.van].straal(), net.putten[l.naar].straal());
        let (sa, sb) = (ra.min(pl_len / 2.0), (pl_len - rb).max(pl_len / 2.0));
        let spec = ctx.layer("RI", &leiding_object(l.stelsel, l.materiaal, l.dn), Element::G)?;
        let (za, zb) = (l.bob_at(sa, pl_len), l.bob_at(sb, pl_len));
        let d = l.d();
        out.put(&spec, [pl(vec![[gx(st[j] + sa), gy(za)], [gx(st[j] + sb), gy(zb)]], false, &spec), pl(vec![[gx(st[j] + sa), gy(za + d)], [gx(st[j] + sb), gy(zb + d)]], false, &spec)]);
        let xm = gx(st[j] + pl_len / 2.0);
        let s = net.verhang(i);
        out.put(&frame_t, [ctx.text([xm, mid(4)], &format!("L={}  i={}", dec(pl_len, 2), verhang_text(s)), 1.8, 0.0, &frame_t, tag)]);
        out.put(&frame_t, [ctx.text([xm, mid(5)], &l.label(), 1.8, 0.0, &frame_t, tag)]);
    }
    // The ground line, broken where there is no height.
    let mut run: Vec<XY> = Vec::new();
    for (s, z) in &samples {
        match z {
            Some(z) => run.push([gx(*s), gy(*z)]),
            None => {
                if run.len() > 1 {
                    out.put(&ground_l, [pl(std::mem::take(&mut run), false, &ground_l)]);
                }
                run.clear();
            }
        }
    }
    if run.len() > 1 {
        out.put(&ground_l, [pl(run, false, &ground_l)]);
    }
    let t = ctx.text([x0, y1 + 9.0 * k], &title, 3.5, 0.0, &frame_t, tag);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let sub = t!(
        "{} · maaiveld {} · horizontaal 1:{} · verticaal 1:{} (overhoging {}×) · lengte {} m",
        "{} · ground level {} · horizontal 1:{} · vertical 1:{} (exaggeration {}×) · length {} m",
        net.stelsel().label(),
        ground.label(),
        opt.nlcs.scale,
        (opt.nlcs.scale / v).round(),
        v,
        dec(len, 1)
    );
    let t = ctx.text([x0, y1 + 4.0 * k], &sub, 1.8, 0.0, &frame_t, tag);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    Ok((at, len, base))
}

fn put_index(net: &Net, name: Option<&str>) -> Result<Option<usize>, String> {
    match name {
        None => Ok(None),
        Some(n) => net.putten.iter().position(|p| p.naam.eq_ignore_ascii_case(n)).map(Some).ok_or_else(|| t!("geen put {} in {}", "no manhole {} in {}", n, net.naam)),
    }
}

/// The profile entities of a network, and where its graph sits.
fn old_profile(doc: &CadDocument, naam: &str) -> (Vec<Handle>, Option<(XY, String, String)>) {
    let mut hs = Vec::new();
    let mut place = None;
    for e in doc.model_space_entities() {
        if data_value(e, "rioolprofiel").as_deref() == Some(naam) {
            hs.push(e.common().handle);
            if data_value(e, "rol").as_deref() == Some("kader") {
                if let (Some(x), Some(y)) = (num(e, "plaats_x"), num(e, "plaats_y")) {
                    place = Some(([x, y], data_value(e, "van").unwrap_or_default(), data_value(e, "naar").unwrap_or_default()));
                }
            }
        }
    }
    (hs, place)
}

/// Draw the profile again in its place after an update. `None`: there was none.
fn redraw_profile(ctx: &Ctx, doc: &CadDocument, g: &Gelezen, opt: &super::CivielOptions, ground: &mut Grond) -> Result<Option<String>, String> {
    let (old, place) = old_profile(doc, &g.net.naam);
    let Some((at, van, naar)) = place else { return Ok(None) };
    let from = g.net.putten.iter().position(|p| p.naam == van);
    let to = g.net.putten.iter().position(|p| p.naam == naar);
    let path = g.net.pad(from, to).or_else(|_| g.net.pad(None, None))?;
    let mut out = Out::default();
    draw_profile(ctx, &g.net, &path, ground, opt, Some(at), &mut out)?;
    bridge::remove_entities(tr("Rioolprofiel (oud)", "Sewer profile (old)"), &old)?;
    let hs = out.write_handles(&t!("Lengteprofiel {}", "Length profile {}", g.net.naam))?;
    bridge::index_add("rioolprofiel", &g.net.naam, &hs, true);
    Ok(Some(t!("{} elementen", "{} entities", hs.len())))
}

/// `CIV_RIOOL_PROFIEL`: the length profile of a network, from manhole `van`
/// downstream to `naar` (default: its longest run). A profile drawn before
/// is replaced, in its place unless `at` is given.
pub fn profiel(opt: &super::CivielOptions, naam: Option<String>, van: Option<String>, naar: Option<String>, at: Option<XY>, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["riool", "rioolprofiel", "tin"], &[])?;
    let mut chosen = choose(lees(&doc, &ctx), naam.as_deref(), false)?;
    let g = chosen.remove(0);
    if !g.changes.is_empty() {
        return Err(t!("{} is in de tekening gewijzigd ({}): voer eerst CIV_RIOOL_BIJWERKEN uit", "{} was changed in the drawing ({}): run CIV_RIOOL_BIJWERKEN first", g.net.naam, g.changes.join(", ")));
    }
    let from = put_index(&g.net, van.as_deref())?;
    let to = put_index(&g.net, naar.as_deref())?;
    let path = g.net.pad(from, to)?;
    progress(t!("Lengteprofiel {} …", "Length profile {} …", g.net.naam));
    let (old, place) = old_profile(&doc, &g.net.naam);
    let at = at.or(place.map(|p| p.0));
    let mut ground = Grond::new(&ctx, &doc, g.referentie);
    let mut out = Out::default();
    let (at, len, _) = draw_profile(&ctx, &g.net, &path, &mut ground, opt, at, &mut out)?;
    if !old.is_empty() {
        bridge::remove_entities(tr("Rioolprofiel (oud)", "Sewer profile (old)"), &old)?;
    }
    let hs = out.write_handles(&t!("Lengteprofiel {}", "Length profile {}", g.net.naam))?;
    bridge::index_add("rioolprofiel", &g.net.naam, &hs, true);
    ground.note_source();
    let names: Vec<String> = std::iter::once(g.net.putten[g.net.leidingen[path[0]].van].naam.clone()).chain(path.iter().map(|i| g.net.putten[g.net.leidingen[*i].naar].naam.clone())).collect();
    api::note("riool", g.net.naam.clone());
    api::note("putten", names.clone());
    api::note("lengte", len);
    api::note("plaats", json!([at[0], at[1]]));
    api::note("kader", hs.first().map(|h| handle_text(*h)).unwrap_or_default());
    Ok(t!(
        "Lengteprofiel {} ({} – {}, {} putten, {} m, maaiveld {}): {} elementen.",
        "Length profile {} ({} – {}, {} manholes, {} m, ground level {}): {} entities.",
        g.net.naam,
        names.first().unwrap(),
        names.last().unwrap(),
        names.len(),
        dec(len, 1),
        ground.label(),
        hs.len()
    ))
}

// ── CIV_RIOOL_TOETS: checks ─────────────────────────────────────────────────

/// What `CIV_RIOOL_TOETS` checks against (arguments; defaults where `None`).
#[derive(Clone, Debug, Default)]
pub struct Toets {
    /// Design flow at the outlet (l/s).
    pub q: Option<f64>,
    /// Connected paved area (m²) × `regen` (l/s/ha).
    pub oppervlak: Option<f64>,
    pub regen: Option<f64>,
    /// Inhabitants × the peak dry weather flow.
    pub inwoners: Option<f64>,
    pub putafstand: Option<f64>,
    pub vrij: Option<f64>,
    /// Wall roughness (mm) for every pipe.
    pub ruwheid: Option<f64>,
}

pub const TOETS_ARGS: &[&str] = &["RIOOL", "Q", "OPPERVLAK", "REGEN", "INWONERS", "PUTAFSTAND", "VRIJ", "RUWHEID"];

impl Toets {
    pub fn from_args(a: &api::Args) -> Result<Toets, String> {
        Ok(Toets {
            q: a.f64("Q")?,
            oppervlak: a.str("OPPERVLAK").map(parse_oppervlak).transpose()?,
            regen: a.f64("REGEN")?,
            inwoners: a.f64("INWONERS")?,
            putafstand: a.f64("PUTAFSTAND")?,
            vrij: a.f64("VRIJ")?,
            ruwheid: a.f64("RUWHEID")?,
        })
    }

    /// The design flow at the outlet (l/s) and how it was found.
    pub fn debiet(&self) -> Option<(f64, String)> {
        if let Some(q) = self.q {
            return Some((q, format!("Q = {} l/s", dec(q, 1))));
        }
        let mut parts = Vec::new();
        let mut q = 0.0;
        if let Some(a) = self.oppervlak {
            let i = self.regen.unwrap_or(REGEN);
            q += a / 10_000.0 * i;
            parts.push(format!("{} ha × {} l/s/ha", dec(a / 10_000.0, 3), dec(i, 0)));
        }
        if let Some(n) = self.inwoners {
            q += n * DWA_PER_INWONER / 3600.0;
            parts.push(t!("{:.0} inwoners × {} l/h", "{:.0} inhabitants × {} l/h", n, dec(DWA_PER_INWONER, 0)));
        }
        (!parts.is_empty()).then(|| (q, format!("{} = {} l/s", parts.join(" + "), dec(q, 1))))
    }
}

#[derive(Clone, Debug)]
pub struct Bevinding {
    pub soort: &'static str,
    /// "fout" or "waarschuwing".
    pub ernst: &'static str,
    pub object: String,
    /// Project coordinates.
    pub xy: XY,
    pub tekst: String,
    pub kort: String,
}

/// A pipe or cable of a KLIC delivery in the drawing.
struct Kabel {
    label: String,
    /// Project coordinates, with the level (m NAP) where known.
    pts: Vec<(XY, Option<f64>)>,
    /// Depth below ground of the top (m) from the object data.
    diepte: Option<f64>,
    /// Outside diameter (m).
    d: f64,
}

/// A number at the start of a text ("160 mm", "160").
fn leading_number(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    t.replace(',', ".").parse().ok()
}

fn klic_cables(doc: &CadDocument, ctx: &Ctx) -> Vec<Kabel> {
    let f = ctx.f;
    let mut out = Vec::new();
    for e in doc.model_space_entities() {
        if data_value(e, "riool").is_some() || data_value(e, "rioolprofiel").is_some() || data_value(e, "riooltoets").is_some() {
            continue;
        }
        let layer = &e.common().layer;
        let klic = data_value(e, "bron").as_deref() == Some("KLIC") || layer.contains("-KL-");
        if !klic || data_value(e, "thema").as_deref() == Some("graafpolygoon") || layer.contains("-ZZ-") {
            continue;
        }
        let v = vertices(e);
        if v.len() < 2 {
            continue;
        }
        // Heights: only from a real 3D line (some vertex off zero).
        let three_d = matches!(e, EntityType::Polyline3D(_) | EntityType::Line(_) | EntityType::Polyline2D(_)) && v.iter().any(|(_, z)| z.is_some_and(|z| z.abs() > 1e-9));
        let abs = ["diepte_nap", "hoogte_nap", "z_nap", "bovenkant_nap"].iter().find_map(|k| num(e, k));
        let pts = v.iter().map(|(p, z)| (ctx.from_drawing(*p), if three_d { z.map(|z| z / f) } else { abs })).collect();
        let diepte = ["diepte", "diepte_m", "diepteTovMaaiveld", "diepte_tov_maaiveld", "dekking"].iter().find_map(|k| num(e, k));
        let d = data_value(e, "diameter").and_then(|s| leading_number(&s)).map(|v| if v > 5.0 { v / 1000.0 } else { v }).unwrap_or(0.0);
        let label = match data_value(e, "thema") {
            Some(_) => {
                let mut l = format!("KLIC {}", data_value(e, "thema").unwrap_or_default());
                if let Some(n) = data_value(e, "netbeheerder") {
                    l.push_str(&format!(" ({n})"));
                }
                l
            }
            None => t!("KLIC-laag {}", "KLIC layer {}", layer),
        };
        out.push(Kabel { label, pts, diepte, d });
    }
    out
}

/// Segment intersection: the parameters along both segments.
fn cross(a: XY, b: XY, c: XY, d: XY) -> Option<(f64, f64)> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / den;
    let u = (q[0] * r[1] - q[1] * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// Another sewer pipe: a CIV_RIOOL pipe of another network, or an imported
/// GWSW pipe with its BOB.
struct AnderRiool {
    label: String,
    a: XY,
    b: XY,
    bob_a: f64,
    bob_b: f64,
    /// Inner diameter and wall (m).
    d: f64,
    t: f64,
}

fn other_sewers(doc: &CadDocument, ctx: &Ctx, nets: &[Gelezen], skip: &str) -> Vec<AnderRiool> {
    let mut out = Vec::new();
    for g in nets.iter().filter(|g| g.net.naam != skip) {
        for (i, l) in g.net.leidingen.iter().enumerate() {
            out.push(AnderRiool { label: format!("{} {}", g.net.naam, g.net.naam_leiding(i)), a: g.net.putten[l.van].xy, b: g.net.putten[l.naar].xy, bob_a: l.bob_van, bob_b: l.bob_naar, d: l.d(), t: l.t() });
        }
    }
    // GWSW pipes (GIS_Q_GWSW, CIV_RIOOL_IMPORT) not taken over.
    for e in doc.model_space_entities() {
        if data_value(e, "riool").is_some() {
            continue;
        }
        let (Some(b0), Some(b1)) = (num(e, "bob_beginpunt_leiding"), num(e, "bob_eindpunt_leiding")) else { continue };
        let v = vertices(e);
        if v.len() < 2 {
            continue;
        }
        let dn = num(e, "breedte_leiding").unwrap_or(300.0);
        let mat = data_value(e, "materiaal_leiding").and_then(|m| Materiaal::parse(&m)).unwrap_or(Materiaal::Beton);
        let pts: Vec<XY> = v.iter().map(|(p, _)| ctx.from_drawing(*p)).collect();
        let total: f64 = pts.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>().max(1e-9);
        let mut run = 0.0;
        for w in pts.windows(2) {
            let l = dist(w[0], w[1]);
            let at = |x: f64| b0 + (b1 - b0) * (x / total);
            out.push(AnderRiool { label: format!("GWSW {}", data_value(e, "naam").unwrap_or_default()), a: w[0], b: w[1], bob_a: at(run), bob_b: at(run + l), d: mat.binnen(dn) / 1000.0, t: mat.wand(dn) / 1000.0 });
            run += l;
        }
    }
    out
}

/// `CIV_RIOOL_TOETS`: cover, slope, manhole spacing, full-pipe capacity and
/// velocity (Prandtl-Colebrook), crossings with KLIC cables and pipes and
/// with other sewers. Markers in the drawing (OI-RIOOLTOETS), a CSV beside
/// it, the findings as JSON (ID=).
pub fn toets(opt: &super::CivielOptions, naam: Option<String>, t: Toets, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["riool", "riooltoets", "tin"], &[])?;
    let all = lees(&doc, &ctx);
    let names: Vec<String> = choose(lees(&doc, &ctx), naam.as_deref(), true)?.into_iter().map(|g| g.net.naam).collect();
    let cables = klic_cables(&doc, &ctx);
    let flow = t.debiet();
    let mut findings: Vec<(String, Bevinding)> = Vec::new();
    let mut hydraulics = Vec::new();
    let mut crossings = Vec::new();
    let mut checked = Vec::new();
    for g in all.iter().filter(|g| names.contains(&g.net.naam)) {
        progress(t!("{} toetsen …", "Checking {} …", g.net.naam));
        let mut net = g.net.clone();
        let mut ground = Grond::new(&ctx, &doc, g.referentie);
        let _ = net.sample_ground(&mut |p| ground.height(p), opt.step.max(0.5));
        let mut add = |b: Bevinding| findings.push((net.naam.clone(), b));
        if !g.changes.is_empty() {
            let xy = net.putten.first().map(|p| p.xy).unwrap_or([0.0, 0.0]);
            add(Bevinding { soort: "gewijzigd", ernst: "fout", object: net.naam.clone(), xy, tekst: t!("gewijzigd in de tekening ({}): voer CIV_RIOOL_BIJWERKEN uit", "changed in the drawing ({}): run CIV_RIOOL_BIJWERKEN", g.changes.join(", ")), kort: tr("gewijzigd: bijwerken", "changed: update").into() });
        }
        let up = net.bovenstrooms();
        let total: f64 = (0..net.leidingen.len()).filter(|i| net.outgoing(net.leidingen[*i].naar).next().is_none()).map(|i| up[i]).sum::<f64>().max(1e-9);
        for (i, up_i) in up.iter().copied().enumerate() {
            let l = net.leidingen[i].clone();
            let name = net.naam_leiding(i);
            let (a, b) = (net.putten[l.van].xy, net.putten[l.naar].xy);
            let len = net.lengte(i);
            let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let along = |x: f64| [a[0] + (b[0] - a[0]) * x / len.max(1e-9), a[1] + (b[1] - a[1]) * x / len.max(1e-9)];
            // Cover over the whole pipe.
            let cover = net.dekking(i);
            if let Some((x, c)) = cover.iter().copied().min_by(|p, q| p.1.total_cmp(&q.1)) {
                if c < l.dekking_min - 0.005 {
                    add(Bevinding { soort: "dekking", ernst: "fout", object: name.clone(), xy: along(x), tekst: t!("leiding {}: dekking {} m op {} m van {}, eis {} m", "pipe {}: cover {} m at {} m from {}, required {} m", name, dec(c, 2), dec(x, 1), net.putten[l.van].naam, dec(l.dekking_min, 2)), kort: t!("dekking {} < {}", "cover {} < {}", dec(c, 2), dec(l.dekking_min, 2)) });
                }
            }
            let s = net.verhang(i);
            if s <= 0.0 {
                add(Bevinding { soort: "verhang", ernst: "fout", object: name.clone(), xy: middle, tekst: t!("leiding {}: {} (BOB {} → {})", "pipe {}: {} (invert {} → {})", name, if s < 0.0 { tr("tegenverhang", "adverse slope") } else { tr("geen verhang", "no slope") }, nap(l.bob_van, 2), nap(l.bob_naar, 2)), kort: tr("tegenverhang", "adverse slope").into() });
            } else if s < l.verhang_min - 0.0011 / len.max(1.0) {
                add(Bevinding { soort: "verhang", ernst: "fout", object: name.clone(), xy: middle, tekst: t!("leiding {}: verhang {} flauwer dan {}", "pipe {}: slope {} flatter than {}", name, verhang_text(s), verhang_text(l.verhang_min)), kort: format!("i {} < {}", verhang_text(s), verhang_text(l.verhang_min)) });
            }
            let max_len = t.putafstand.unwrap_or_else(|| max_putafstand(l.dn));
            if len > max_len + 1e-6 {
                add(Bevinding { soort: "putafstand", ernst: "fout", object: name.clone(), xy: middle, tekst: t!("leiding {}: putafstand {} m, meer dan {} m", "pipe {}: manhole spacing {} m, more than {} m", name, dec(len, 1), dec(max_len, 0)), kort: t!("putafstand {} > {} m", "spacing {} > {} m", dec(len, 0), dec(max_len, 0)) });
            }
            // Full-pipe capacity and velocity.
            let kr = t.ruwheid.unwrap_or_else(|| l.materiaal.ruwheid()) / 1000.0;
            let v = snelheid_vol(l.d(), s.max(0.0), kr);
            let qv = capaciteit_vol(l.d(), s.max(0.0), kr) * 1000.0;
            let q = flow.as_ref().map(|(q, _)| q * up_i / total);
            if let Some(q) = q {
                if q > qv + 1e-9 {
                    add(Bevinding { soort: "capaciteit", ernst: "fout", object: name.clone(), xy: middle, tekst: t!("leiding {}: ontwerpdebiet {} l/s groter dan de capaciteit bij volle buis {} l/s", "pipe {}: design flow {} l/s exceeds the full-pipe capacity {} l/s", name, dec(q, 1), dec(qv, 1)), kort: format!("Q {} > {} l/s", dec(q, 1), dec(qv, 1)) });
                }
            }
            if s > 0.0 && v < V_MIN {
                add(Bevinding { soort: "snelheid", ernst: "waarschuwing", object: name.clone(), xy: middle, tekst: t!("leiding {}: snelheid bij volle buis {} m/s, minder dan {} m/s (zelfreiniging)", "pipe {}: full-pipe velocity {} m/s, less than {} m/s (self-cleansing)", name, dec(v, 2), dec(V_MIN, 1)), kort: format!("v {} < {} m/s", dec(v, 2), dec(V_MIN, 1)) });
            }
            if v > v_max(l.materiaal) {
                add(Bevinding { soort: "snelheid", ernst: "waarschuwing", object: name.clone(), xy: middle, tekst: t!("leiding {}: snelheid bij volle buis {} m/s, meer dan {} m/s", "pipe {}: full-pipe velocity {} m/s, more than {} m/s", name, dec(v, 2), dec(v_max(l.materiaal), 1)), kort: format!("v {} > {} m/s", dec(v, 2), dec(v_max(l.materiaal), 1)) });
            }
            hydraulics.push(json!({
                "riool": net.naam, "leiding": name, "diameter": l.dn, "binnendiameter_mm": l.d() * 1000.0, "materiaal": l.materiaal.code(),
                "ruwheid_mm": kr * 1000.0, "verhang": s, "lengte": len, "snelheid_vol": v, "capaciteit_vol": qv, "debiet": q,
                "vullingsgraad": q.map(|q| q / qv.max(1e-9)), "dekking_min": cover.iter().map(|c| c.1).reduce(f64::min),
            }));
            // KLIC crossings in plan.
            let pipe_top = |x: f64| l.bob_at(x, len) + l.d() + l.t();
            let pipe_bottom = |x: f64| l.bob_at(x, len) - l.t();
            let vrij_k = t.vrij.unwrap_or(VRIJ_KLIC);
            for c in &cables {
                for w in c.pts.windows(2) {
                    let Some((tt, u)) = cross(a, b, w[0].0, w[1].0) else { continue };
                    let x = tt * len;
                    let at = along(x);
                    let g_here = ground.height(at);
                    let top = match (w[0].1, w[1].1) {
                        (Some(z0), Some(z1)) => Some(z0 + (z1 - z0) * u),
                        _ => c.diepte.zip(g_here).map(|(d, g)| g - d),
                    };
                    let mut row = json!({"riool": net.naam, "leiding": name, "met": c.label, "x": at[0], "y": at[1], "afstand_van": x, "bob": l.bob_at(x, len), "bovenkant_riool": pipe_top(x), "maaiveld": g_here});
                    match top {
                        Some(top) => {
                            let bottom = top - c.d;
                            let clear = (bottom - pipe_top(x)).max(pipe_bottom(x) - top);
                            row["bovenkant_kabel"] = json!(top);
                            row["vrije_ruimte"] = json!(clear);
                            row["ok"] = json!(clear >= vrij_k);
                            if clear < vrij_k {
                                add(Bevinding { soort: "klic", ernst: "fout", object: name.clone(), xy: at, tekst: t!("leiding {} kruist {} op {} m van {}: vrije ruimte {} m, eis {} m", "pipe {} crosses {} at {} m from {}: clearance {} m, required {} m", name, c.label, dec(x, 1), net.putten[l.van].naam, dec(clear, 2), dec(vrij_k, 2)), kort: t!("KLIC: vrij {} < {}", "KLIC: clear {} < {}", dec(clear, 2), dec(vrij_k, 2)) });
                            }
                        }
                        None => {
                            row["ok"] = Value::Null;
                            let depth = g_here.map(|g| t!(", bovenkant riool {} m onder maaiveld", ", top of the sewer {} m below ground", dec(g - pipe_top(x), 2))).unwrap_or_default();
                            add(Bevinding { soort: "klic", ernst: "waarschuwing", object: name.clone(), xy: at, tekst: t!("leiding {} kruist {} op {} m van {}; diepte van de kabel/leiding onbekend — BOB {}{}", "pipe {} crosses {} at {} m from {}; depth of the cable/pipe unknown — invert {}{}", name, c.label, dec(x, 1), net.putten[l.van].naam, nap(l.bob_at(x, len), 2), depth), kort: tr("KLIC: diepte onbekend", "KLIC: depth unknown").into() });
                        }
                    }
                    crossings.push(row);
                }
            }
            // Other sewers: vertical clearance at the crossing.
            let vrij_r = t.vrij.unwrap_or(VRIJ_RIOOL);
            for o in other_sewers(&doc, &ctx, &all, &net.naam) {
                let Some((tt, u)) = cross(a, b, o.a, o.b) else { continue };
                // Pipes that meet in a manhole do not cross.
                if (tt < 1e-6 || tt > 1.0 - 1e-6) && (u < 1e-6 || u > 1.0 - 1e-6) {
                    continue;
                }
                let x = tt * len;
                let at = along(x);
                let ob = o.bob_a + (o.bob_b - o.bob_a) * u;
                let (o_top, o_bottom) = (ob + o.d + o.t, ob - o.t);
                let clear = (o_bottom - pipe_top(x)).max(pipe_bottom(x) - o_top);
                crossings.push(json!({"riool": net.naam, "leiding": name, "met": o.label, "x": at[0], "y": at[1], "afstand_van": x, "bob": l.bob_at(x, len), "bob_ander": ob, "vrije_ruimte": clear, "ok": clear >= vrij_r}));
                if clear < vrij_r {
                    add(Bevinding { soort: "kruising", ernst: "fout", object: name.clone(), xy: at, tekst: t!("leiding {} kruist {}: vrije ruimte {} m, eis {} m (BOB {} en {})", "pipe {} crosses {}: clearance {} m, required {} m (inverts {} and {})", name, o.label, dec(clear, 2), dec(vrij_r, 2), nap(l.bob_at(x, len), 2), nap(ob, 2)), kort: t!("riool: vrij {} < {}", "sewer: clear {} < {}", dec(clear, 2), dec(vrij_r, 2)) });
                }
            }
        }
        ground.note_source();
        checked.push(net.naam.clone());
    }
    // Markers: the old ones of these networks go.
    let old: Vec<Handle> = doc.model_space_entities().filter(|e| data_value(e, "riooltoets").is_some_and(|n| checked.contains(&n))).map(|e| e.common().handle).collect();
    if !old.is_empty() {
        bridge::remove_entities(tr("Riooltoets (oud)", "Sewer check (old)"), &old)?;
    }
    let mut out = Out::default();
    let layer = |name: &str, aci: u8| LayerSpec { name: name.into(), color: aci, rgb: nlcs::color_rgb(aci), true_color: None, lineweight: Some(35), linetype: "CONTINUOUS".into(), object_id: 0 };
    let (red, orange) = (layer("OI-RIOOLTOETS", 1), layer("OI-RIOOLTOETS-LET-OP", 30));
    for (net, b) in &findings {
        let spec = if b.ernst == "fout" { &red } else { &orange };
        let data = record(&[("riooltoets", net.clone()), ("soort", b.soort.into()), ("ernst", b.ernst.into()), ("object", b.object.clone()), ("bevinding", b.tekst.clone())]);
        let c = ctx.to_drawing(b.xy);
        out.put(spec, [styled(EntityType::Circle(Circle::from_center_radius(Vector3::new(c[0], c[1], 0.0), 2.5 * k)), spec, 1.0, Some(&data))]);
        let t = ctx.text([c[0] + 3.5 * k, c[1] + 3.0 * k], &b.kort, 1.8, 0.0, spec, Some(&data));
        out.put(spec, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    let _ = f;
    let handles = if out.is_empty() { Vec::new() } else { out.write_handles(tr("Riooltoets", "Sewer check"))? };
    for n in &checked {
        bridge::index_add("riooltoets", n, &handles, true);
    }
    // The report.
    let mut csv = String::from(tr("Riool;Soort;Ernst;Object;X;Y;Bevinding\n", "Sewer;Kind;Severity;Object;X;Y;Finding\n"));
    for (net, b) in &findings {
        csv.push_str(&format!("{net};{};{};{};{};{};{}\n", b.soort, b.ernst, b.object, dec(b.xy[0], 3), dec(b.xy[1], 3), b.tekst.replace(';', ",")));
    }
    let slug = checked.join("_").replace(' ', "_").to_lowercase();
    let csv_path = bridge::beside_drawing(&format!("riooltoets_{slug}.csv"));
    let _ = std::fs::write(&csv_path, csv);
    let errors = findings.iter().filter(|(_, b)| b.ernst == "fout").count();
    let warnings = findings.len() - errors;
    let mut by_kind: Vec<(&str, usize)> = Vec::new();
    for (_, b) in &findings {
        match by_kind.iter_mut().find(|(k, _)| *k == b.soort) {
            Some(e) => e.1 += 1,
            None => by_kind.push((b.soort, 1)),
        }
    }
    api::note("riolen", checked.clone());
    api::note("fouten", errors);
    api::note("waarschuwingen", warnings);
    api::note("bevindingen", findings.iter().map(|(n, b)| json!({"riool": n, "soort": b.soort, "ernst": b.ernst, "object": b.object, "x": b.xy[0], "y": b.xy[1], "tekst": b.tekst})).collect::<Vec<_>>());
    api::note("leidingen", hydraulics);
    api::note("kruisingen", crossings.clone());
    api::note("klic_objecten", cables.len());
    api::note("debiet", flow.as_ref().map(|f| json!({"q": f.0, "uitleg": f.1})).unwrap_or(Value::Null));
    api::note("csv", csv_path.to_string_lossy().into_owned());
    api::note("markeringen", handles.len());
    let kinds = by_kind.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ");
    Ok(t!(
        "Riooltoets {}: {} fout(en), {} waarschuwing(en){}; {} kruising(en) ({} KLIC-objecten in de tekening){}. Rapport: {}",
        "Sewer check {}: {} error(s), {} warning(s){}; {} crossing(s) ({} KLIC objects in the drawing){}. Report: {}",
        checked.join(", "),
        errors,
        warnings,
        if kinds.is_empty() { String::new() } else { format!(" ({kinds})") },
        crossings.len(),
        cables.len(),
        flow.map(|f| format!("; {}", f.1)).unwrap_or_else(|| tr("; geen ontwerpdebiet (Q=, OPPERVLAK= of INWONERS=)", "; no design flow (Q=, OPPERVLAK= or INWONERS=)").into()),
        csv_path.display()
    ))
}

// ── CIV_RIOOL_EXPORT: GWSW-OroX and CSV ─────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formaat {
    Gwsw,
    Csv,
    /// GWSW-HydX: the CSV set for hydraulic models.
    Hydx,
}

impl Formaat {
    pub fn parse(s: &str) -> Option<Formaat> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "gwsw" | "orox" | "ttl" | "gwsw-orox" | "rdf" => Formaat::Gwsw,
            "csv" => Formaat::Csv,
            "hydx" | "gwsw-hydx" | "hyd" => Formaat::Hydx,
            _ => return None,
        })
    }
}

type ToRd = Box<dyn Fn(XY) -> Result<XY, String>>;

/// Project coordinates → RD (GWSW works in RD with NAP).
fn to_rd(ctx: &Ctx) -> Result<ToRd, String> {
    match &ctx.site {
        Some(s) if s.crs != "EPSG:28992" => {
            let t = crate::gis::crs::Transformer::new(&s.crs, "EPSG:28992")?;
            Ok(Box::new(move |p| t.apply(p)))
        }
        _ => Ok(Box::new(Ok)),
    }
}

/// The OroX dataset of networks (RD).
pub fn orox_of(nets: &[&Net], naam: &str, rd: &dyn Fn(XY) -> Result<XY, String>) -> Result<gwsw::Orox, String> {
    let mut o = gwsw::Orox { naam: naam.to_string(), app: format!("Open Infra {}", env!("CARGO_PKG_VERSION")), datum: chrono::Local::now().format("%Y-%m-%d").to_string(), stelsels: Vec::new(), putten: Vec::new(), leidingen: Vec::new() };
    let stelsel_of = |net: &Net, s: Stelsel, list: &mut Vec<gwsw::OroxStelsel>| -> usize {
        let label = format!("{} {}", net.naam, s.code());
        match list.iter().position(|x| x.label == label) {
            Some(i) => i,
            None => {
                list.push(gwsw::OroxStelsel { klasse: s.gwsw_stelsel().into(), label });
                list.len() - 1
            }
        }
    };
    for net in nets {
        let base = o.putten.len();
        for (i, p) in net.putten.iter().enumerate() {
            let st = stelsel_of(net, p.stelsel, &mut o.stelsels);
            let depth = p.maaiveld.zip(net.bob_laagst(i)).map(|(g, b)| ((g - b) * 1000.0).round()).filter(|h| (500.0..=4000.0).contains(h));
            o.putten.push(gwsw::OroxPut {
                naam: p.naam.clone(),
                klasse: "Inspectieput".into(),
                stelsel: st,
                xy: rd(p.xy)?,
                maaiveld: p.maaiveld,
                breedte: p.maat[0],
                lengte: if p.maat[1] > 0.0 { p.maat[1] } else { p.maat[0] },
                hoogte: depth,
                rond: p.maat[1] <= 0.0,
                materiaal: None,
            });
        }
        for (i, l) in net.leidingen.iter().enumerate() {
            let st = stelsel_of(net, l.stelsel, &mut o.stelsels);
            let (a, b) = (rd(net.putten[l.van].xy)?, rd(net.putten[l.naar].xy)?);
            o.leidingen.push(gwsw::OroxLeiding {
                naam: net.naam_leiding(i),
                klasse: l.stelsel.gwsw_leiding().into(),
                stelsel: st,
                van: base + l.van,
                naar: base + l.naar,
                punten: vec![a, b],
                breedte: l.dn,
                materiaal: l.materiaal.gwsw().into(),
                lengte: net.lengte(i),
                bob_van: l.bob_van,
                bob_naar: l.bob_naar,
                ontwerp: !l.bestaand,
            });
        }
    }
    Ok(o)
}

/// The manhole and pipe tables (`;`, decimal comma).
pub fn csv_of(nets: &[&Net], rd: &dyn Fn(XY) -> Result<XY, String>) -> Result<(String, String), String> {
    let mut puts = String::from(tr(
        "Riool;Putnummer;Stelsel;X (RD);Y (RD);Maaiveld (m NAP);Afmeting (mm);BOB laagst (m NAP);Diepte (m);Bestaand\n",
        "Sewer;Manhole;System;X (RD);Y (RD);Ground level (m NAP);Size (mm);Lowest invert (m NAP);Depth (m);Existing\n",
    ));
    let mut pipes = String::from(tr(
        "Riool;Leiding;Van;Naar;Stelsel;Materiaal;Diameter (mm);Binnendiameter (mm);Wanddikte (mm);Lengte (m);BOB van (m NAP);BOB naar (m NAP);Verhang;Verhang (1:n);Dekking min (m);Eis dekking (m);Eis verhang;Bestaand\n",
        "Sewer;Pipe;From;To;System;Material;Diameter (mm);Inner diameter (mm);Wall (mm);Length (m);Invert from (m NAP);Invert to (m NAP);Slope;Slope (1:n);Smallest cover (m);Required cover (m);Required slope;Existing\n",
    ));
    let opt_dec = |v: Option<f64>, n: usize| v.map(|v| dec(v, n)).unwrap_or_default();
    for net in nets {
        for (i, p) in net.putten.iter().enumerate() {
            let q = rd(p.xy)?;
            let low = net.bob_laagst(i);
            puts.push_str(&format!(
                "{};{};{};{};{};{};{};{};{};{}\n",
                net.naam,
                p.naam,
                p.stelsel.code(),
                dec(q[0], 3),
                dec(q[1], 3),
                opt_dec(p.maaiveld, 3),
                p.maat_text(),
                opt_dec(low, 3),
                opt_dec(p.maaiveld.zip(low).map(|(g, b)| g - b), 3),
                if p.bestaand { tr("ja", "yes") } else { tr("nee", "no") }
            ));
        }
        for (i, l) in net.leidingen.iter().enumerate() {
            let cover = net.dekking(i).iter().map(|c| c.1).reduce(f64::min);
            pipes.push_str(&format!(
                "{};{};{};{};{};{};{:.0};{};{};{};{};{};{};{};{};{};{};{}\n",
                net.naam,
                net.naam_leiding(i),
                net.putten[l.van].naam,
                net.putten[l.naar].naam,
                l.stelsel.code(),
                l.materiaal.label(),
                l.dn,
                dec(l.materiaal.binnen(l.dn), 1),
                dec(l.materiaal.wand(l.dn), 1),
                dec(net.lengte(i), 3),
                dec(l.bob_van, 3),
                dec(l.bob_naar, 3),
                dec(net.verhang(i), 5),
                verhang_text(net.verhang(i)),
                opt_dec(cover, 3),
                dec(l.dekking_min, 2),
                verhang_text(l.verhang_min),
                if l.bestaand { tr("ja", "yes") } else { tr("nee", "no") }
            ));
        }
    }
    Ok((puts, pipes))
}

/// `CIV_RIOOL_EXPORT`: networks to GWSW-OroX (`FORMAAT=GWSW`, the default),
/// to two CSV tables (`FORMAAT=CSV`) or to a GWSW-HydX folder for hydraulic
/// models (`FORMAAT=HYDX`), beside the drawing or at `PAD=`.
pub fn export(opt: &super::CivielOptions, naam: Option<String>, formaat: Formaat, pad: Option<std::path::PathBuf>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["riool"], &[])?;
    let chosen = choose(lees(&doc, &ctx), naam.as_deref(), true)?;
    if let Some(g) = chosen.iter().find(|g| !g.changes.is_empty()) {
        return Err(t!("{} is in de tekening gewijzigd ({}): voer eerst CIV_RIOOL_BIJWERKEN uit", "{} was changed in the drawing ({}): run CIV_RIOOL_BIJWERKEN first", g.net.naam, g.changes.join(", ")));
    }
    let nets: Vec<&Net> = chosen.iter().map(|g| &g.net).collect();
    let rd = to_rd(&ctx)?;
    let slug = if nets.len() == 1 { orox_slug(&nets[0].naam) } else { "riolering".into() };
    let n_put: usize = nets.iter().map(|n| n.putten.len()).sum();
    let n_pipe: usize = nets.iter().map(|n| n.leidingen.len()).sum();
    api::note("riolen", nets.iter().map(|n| n.naam.clone()).collect::<Vec<_>>());
    api::note("putten", n_put);
    api::note("leidingen", n_pipe);
    match formaat {
        Formaat::Gwsw => {
            let path = pad.unwrap_or_else(|| bridge::beside_drawing(&format!("{slug}.orox.ttl")));
            let name = path.file_name().and_then(|n| n.to_str()).map(|n| n.trim_end_matches(".ttl").trim_end_matches(".orox").to_string()).unwrap_or(slug);
            let o = orox_of(&nets, &name, &*rd)?;
            let text = gwsw::write_orox(&o);
            std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
            api::note("bestand", path.to_string_lossy().into_owned());
            api::note("gwsw_versie", gwsw::OROX_VERSIE);
            Ok(t!("GWSW-OroX (GWSW {}): {} putten, {} leidingen → {}", "GWSW-OroX (GWSW {}): {} manholes, {} pipes → {}", gwsw::OROX_VERSIE, n_put, n_pipe, path.display()))
        }
        Formaat::Hydx => {
            let dir = pad.unwrap_or_else(|| bridge::beside_drawing(&format!("{slug}_hydx")));
            let desc = nets.iter().map(|n| n.naam.clone()).collect::<Vec<_>>().join(", ");
            let t = super::hydx::tables(&nets, &*rd, &super::hydx::today(), &desc)?;
            let files = super::hydx::write(&dir, &t)?;
            api::note("bestanden", files.iter().map(|f| f.to_string_lossy().into_owned()).collect::<Vec<_>>());
            Ok(t!("GWSW-HydX: {} putten, {} leidingen, {} bestanden → {}", "GWSW-HydX: {} manholes, {} pipes, {} files → {}", n_put, n_pipe, files.len(), dir.display()))
        }
        Formaat::Csv => {
            let base = pad.unwrap_or_else(|| bridge::beside_drawing(&format!("{slug}.csv")));
            let stem = base.with_extension("");
            let (p_put, p_pipe) = (std::path::PathBuf::from(format!("{}_putten.csv", stem.display())), std::path::PathBuf::from(format!("{}_leidingen.csv", stem.display())));
            let (a, b) = csv_of(&nets, &*rd)?;
            std::fs::write(&p_put, a).map_err(|e| format!("{}: {e}", p_put.display()))?;
            std::fs::write(&p_pipe, b).map_err(|e| format!("{}: {e}", p_pipe.display()))?;
            api::note("bestanden", vec![p_put.to_string_lossy().into_owned(), p_pipe.to_string_lossy().into_owned()]);
            Ok(t!("CSV: {} putten → {}, {} leidingen → {}", "CSV: {} manholes → {}, {} pipes → {}", n_put, p_put.display(), n_pipe, p_pipe.display()))
        }
    }
}

fn orox_slug(name: &str) -> String {
    gwsw::orox_local(&name.to_lowercase())
}

/// `CIV_RIOOL_IMPORT <file.orox.ttl>`: a GWSW-OroX file drawn by the GWSW
/// import's route (NLCS RI layers, GWSW attributes as object data), like
/// `GIS_Q_GWSW` draws the PDOK service.
pub fn import(opt: &super::CivielOptions, path: &std::path::Path) -> Result<String, String> {
    let site = bridge::site().ok_or(tr("stel eerst de projectlocatie in (GIS → Locatie): GWSW ligt in RD", "set the project location first (GIS → Location): GWSW data is in RD"))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let data = gwsw::read_orox(&text)?;
    for (c, f) in &data {
        api::note(c, f.len());
    }
    let msg = crate::riolering::place(&site, &data, &opt.nlcs)?;
    Ok(format!("{} ({})", msg, path.file_name().and_then(|n| n.to_str()).unwrap_or("")))
}

// ── CIV_RIOOL_GWSW: an existing GWSW network made editable ──────────────────

/// `CIV_RIOOL_GWSW`: the GWSW pipes and manholes in the drawing (from
/// `GIS_Q_GWSW` or `CIV_RIOOL_IMPORT`; the selected ones, else all) become
/// CIV_RIOOL networks, one per system, with their measured BOB and ground
/// levels kept as existing ("bestaand"). New runs that end in one of their
/// manholes join them; only the new pipes are computed.
pub fn van_gwsw(opt: &super::CivielOptions, naam: Option<String>, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["riool"], &[])?;
    let sel: HashSet<Handle> = bridge::selection().unwrap_or_default().into_iter().collect();
    let gwsw_entity = |e: &EntityType| data_value(e, "riool").is_none() && data_value(e, "naam").is_some() && e.common().layer.contains("-RI-");
    let pipe_ents: Vec<&EntityType> = doc
        .model_space_entities()
        .filter(|e| gwsw_entity(e) && vertices(e).len() >= 2 && (data_value(e, "beginpunt").is_some() || data_value(e, "bob_beginpunt_leiding").is_some() || e.common().layer.contains("LEIDING")))
        .filter(|e| sel.is_empty() || sel.contains(&e.common().handle))
        .collect();
    if pipe_ents.is_empty() {
        return Err(tr("geen GWSW-leidingen in de tekening (of de selectie): haal ze op met GIS_Q_GWSW of CIV_RIOOL_IMPORT", "no GWSW pipes in the drawing (or the selection): fetch them with GIS_Q_GWSW or CIV_RIOOL_IMPORT").into());
    }
    progress(t!("{} GWSW-leidingen overnemen …", "Taking over {} GWSW pipes …", pipe_ents.len()));
    let put_ents: Vec<&EntityType> = doc.model_space_entities().filter(|e| gwsw_entity(e) && matches!(e, EntityType::Circle(_)) && e.common().layer.contains("RIOOLPUT")).collect();
    // Manholes by name.
    struct Raw {
        naam: String,
        xy: XY,
        maaiveld: Option<f64>,
        maat: [f64; 2],
        handle: Handle,
    }
    let mut raw_puts: Vec<Raw> = Vec::new();
    for e in &put_ents {
        let Some(c) = centre(e) else { continue };
        let mut naam = data_value(e, "naam").unwrap_or_default();
        if raw_puts.iter().any(|p| p.naam == naam) {
            naam = format!("{naam}_{}", raw_puts.len() + 1);
        }
        let b = num(e, "breedte_put").filter(|v| (300.0..=5000.0).contains(v));
        let l = num(e, "lengte_put").filter(|v| (300.0..=5000.0).contains(v));
        let round = data_value(e, "vorm_put").is_none_or(|v| v.to_ascii_lowercase().contains("rond"));
        let maat = match (b, l) {
            (Some(b), Some(l)) if !round && (b - l).abs() > 1.0 => [b, l],
            (Some(b), _) => [b, 0.0],
            _ => [1000.0, 0.0],
        };
        raw_puts.push(Raw { naam, xy: ctx.from_drawing(c), maaiveld: num(e, "maaiveldhoogte"), maat, handle: e.common().handle });
    }
    // Pipes, per system.
    let mut nets: Vec<(Stelsel, Net)> = Vec::new();
    let mut used: HashSet<Handle> = HashSet::new();
    let mut no_bob = 0;
    for e in &pipe_ents {
        let v = vertices(e);
        let (a, b) = (ctx.from_drawing(v[0].0), ctx.from_drawing(v[v.len() - 1].0));
        let stelsel = Stelsel::from_gwsw(&data_value(e, "type").unwrap_or_default(), &data_value(e, "stelseltype").unwrap_or_default());
        if !nets.iter().any(|(s, _)| *s == stelsel) {
            nets.push((stelsel, Net { naam: String::new(), putten: Vec::new(), leidingen: Vec::new() }));
        }
        let net = &mut nets.iter_mut().find(|(s, _)| *s == stelsel).unwrap().1;
        // A manhole by name, else the nearest within a metre, else a node.
        let mut find = |name: Option<String>, at: XY, net: &mut Net| -> usize {
            let raw = name.as_ref().and_then(|n| raw_puts.iter().find(|p| p.naam == *n)).or_else(|| raw_puts.iter().filter(|p| dist(p.xy, at) < 1.0).min_by(|p, q| dist(p.xy, at).total_cmp(&dist(q.xy, at))));
            let (naam, xy, mv, maat) = match raw {
                Some(r) => {
                    used.insert(r.handle);
                    (r.naam.clone(), r.xy, r.maaiveld, r.maat)
                }
                None => (name.unwrap_or_else(|| format!("K{}", net.putten.len() + 1)), at, None, [1000.0, 0.0]),
            };
            if let Some(i) = net.putten.iter().position(|p| p.naam == naam) {
                return i;
            }
            let mut p = Put::new(naam, xy, stelsel, maat);
            p.maaiveld = mv;
            p.maaiveld_vast = mv.is_some();
            p.bestaand = true;
            net.putten.push(p);
            net.putten.len() - 1
        };
        let mut van = find(data_value(e, "beginpunt"), a, net);
        let mut naar = find(data_value(e, "eindpunt"), b, net);
        if van == naar {
            continue;
        }
        let dn = num(e, "breedte_leiding").filter(|d| *d >= 50.0).unwrap_or(300.0);
        let mat = data_value(e, "materiaal_leiding").and_then(|m| Materiaal::parse(&m)).unwrap_or(Materiaal::Beton);
        let mut l = Leiding::new(van, naar, dn, mat, stelsel);
        match (num(e, "bob_beginpunt_leiding"), num(e, "bob_eindpunt_leiding")) {
            (Some(b0), Some(b1)) => {
                // Our pipes point downstream.
                let (mut b0, mut b1) = (b0, b1);
                if b1 > b0 + 1e-3 {
                    std::mem::swap(&mut van, &mut naar);
                    std::mem::swap(&mut b0, &mut b1);
                }
                l.van = van;
                l.naar = naar;
                l.bob_van = b0;
                l.bob_naar = b1;
                l.bestaand = true;
            }
            _ => no_bob += 1,
        }
        net.leidingen.push(l);
        used.insert(e.common().handle);
    }
    // Their manhole labels go with them.
    let old_labels: Vec<Handle> = doc
        .model_space_entities()
        .filter(|e| matches!(e, EntityType::Text(_)) && gwsw_entity(e) && data_value(e, "naam").is_some_and(|n| raw_puts.iter().any(|p| p.naam == n && used.contains(&p.handle))))
        .map(|e| e.common().handle)
        .collect();
    let existing = lees(&doc, &ctx);
    let mut msgs = Vec::new();
    let mut opt_b = opt.clone();
    opt_b.nlcs.status = nlcs::Status::B;
    let ctx_b = Ctx::new(&opt_b)?;
    let several = nets.len() > 1;
    let mut ground = Grond::new(&ctx, &doc, Referentie::Auto);
    let mut handles_all = Vec::new();
    for (s, mut net) in nets {
        let base = match &naam {
            Some(n) if several => format!("{n} {}", s.code()),
            Some(n) => n.clone(),
            None => format!("GWSW {}", s.code()),
        };
        let mut name = base.clone();
        let mut k = 2;
        while existing.iter().any(|g| g.net.naam == name) {
            name = format!("{base} {k}");
            k += 1;
        }
        net.naam = name;
        let mut warnings = net.sample_ground(&mut |p| ground.height(p), opt.step.max(0.5));
        // Pipes without a BOB get one by the design rules.
        warnings.extend(net.bereken()?);
        let n = net.leidingen.len();
        let g = Gelezen { put_handles: vec![None; net.putten.len()], leiding_handles: vec![None; n], net, handles: Vec::new(), changes: Vec::new(), referentie: Referentie::Auto, verhang_auto: vec![true; n] };
        let (hs, _, _) = write_net(&ctx_b, &g, opt.point_labels, &[], &g.net.naam.clone())?;
        handles_all.extend(hs);
        msgs.push(format!("{} ({}): {}{}", g.net.naam, s.label(), summary(&g.net), if warnings.is_empty() { String::new() } else { t!(" — let op: {}", " — note: {}", warnings.join("; ")) }));
    }
    let mut remove: Vec<Handle> = used.into_iter().collect();
    remove.extend(old_labels);
    let removed = bridge::remove_entities(tr("GWSW overnemen", "Take over GWSW"), &remove)?;
    ground.note_source();
    api::note("elementen", handles_all.len());
    api::note("vervangen", removed);
    api::note("zonder_bob", no_bob);
    Ok(t!(
        "GWSW overgenomen als bewerkbaar riool: {}. {} GWSW-elementen vervangen{}. Sluit een nieuw riool aan door CIV_RIOOL in een van deze putten te laten eindigen.",
        "GWSW taken over as an editable sewer: {}. {} GWSW entities replaced{}. Join a new sewer by ending CIV_RIOOL in one of these manholes.",
        msgs.join(" | "),
        removed,
        if no_bob > 0 { t!("; {} leiding(en) zonder BOB kregen er een volgens de ontwerpregels", "; {} pipe(s) without an invert got one by the design rules", no_bob) } else { String::new() }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight run along x with manholes every `step` m, ground `g(x)`.
    fn run(n: usize, step: f64, dn: f64, mat: Materiaal, g: impl Fn(f64) -> f64) -> Net {
        let putten: Vec<Put> = (0..n).map(|i| Put::new(format!("P{}", 101 + i), [i as f64 * step, 0.0], Stelsel::Dwa, standaard_put(dn))).collect();
        let leidingen = (1..n).map(|i| Leiding::new(i - 1, i, dn, mat, Stelsel::Dwa)).collect();
        let mut net = Net { naam: "Riool 1".into(), putten, leidingen };
        let _ = net.sample_ground(&mut |p| Some(g(p[0])), 1.0);
        net
    }

    #[test]
    fn flat_ground_gives_minimum_cover_and_slope() {
        let mut net = run(4, 50.0, 250.0, Materiaal::Pvc, |_| 0.0);
        assert!(net.bereken().unwrap().is_empty());
        // PVC 250 SN8: wall 7.3, inner 235.4 mm; 0.80 m cover to the outside top.
        let first = -0.8 - (0.2354 + 0.0073);
        assert!((net.leidingen[0].bob_van - first).abs() < 1e-9, "{}", net.leidingen[0].bob_van);
        for i in 0..3 {
            assert!((net.verhang(i) - 1.0 / 250.0).abs() < 1e-12);
            assert!((net.leidingen[i].bob_van - (first - 0.2 * i as f64)).abs() < 1e-9);
        }
        // The cover is 0.80 at the start and grows downstream.
        let c = net.dekking(0);
        assert!((c[0].1 - 0.8).abs() < 1e-9 && c.last().unwrap().1 > 0.99);
        assert_eq!(net.bob_laagst(3), Some(net.leidingen[2].bob_naar));
    }

    #[test]
    fn the_pipe_follows_ground_that_falls_faster_than_the_minimum_slope() {
        // Ground falls 1:50; the pipe keeps the minimum cover.
        let mut net = run(3, 40.0, 315.0, Materiaal::Pvc, |x| 2.0 - x / 50.0);
        net.bereken().unwrap();
        assert!((net.verhang(0) - 0.02).abs() < 1e-9 && (net.verhang(1) - 0.02).abs() < 1e-9);
        for i in 0..2 {
            for (_, c) in net.dekking(i) {
                assert!((c - 0.8).abs() < 1e-6, "{c}");
            }
        }
        // A bump in the middle of a pipe lowers its start.
        let mut net = run(2, 40.0, 315.0, Materiaal::Pvc, |x| if (x - 20.0).abs() < 1.0 { -1.0 } else { 0.0 });
        net.bereken().unwrap();
        let min = net.dekking(0).iter().map(|c| c.1).fold(f64::MAX, f64::min);
        assert!((min - 0.8).abs() < 1e-6, "{min}");
        assert!(net.verhang(0) >= 1.0 / 315.0 - 1e-12);
    }

    #[test]
    fn a_larger_pipe_matches_crowns_and_a_fixed_bob_is_kept() {
        let mut net = run(3, 50.0, 250.0, Materiaal::Pvc, |_| 0.0);
        net.leidingen[1].dn = 400.0;
        net.leidingen[1].verhang_min = min_verhang(400.0);
        net.bereken().unwrap();
        let drop = (Materiaal::Pvc.binnen(400.0) - Materiaal::Pvc.binnen(250.0)) / 1000.0;
        // The cover rule asks the larger pipe to start lower still.
        let by_cover = -0.8 - (Materiaal::Pvc.binnen(400.0) + Materiaal::Pvc.wand(400.0)) / 1000.0;
        let expect = (net.leidingen[0].bob_naar - drop).min(by_cover);
        assert!((net.leidingen[1].bob_van - expect).abs() < 1e-9);
        // BOB= at the start, deeper than needed: kept, no warning.
        let mut net = run(2, 50.0, 250.0, Materiaal::Pvc, |_| 0.0);
        net.putten[0].bob_vast = Some(-1.5);
        assert!(net.bereken().unwrap().is_empty());
        assert_eq!(net.leidingen[0].bob_van, -1.5);
        // Too shallow: kept, with a warning about the cover.
        net.putten[0].bob_vast = Some(-0.5);
        let w = net.bereken().unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(net.leidingen[0].bob_van, -0.5);
    }

    #[test]
    fn a_branch_joins_and_an_existing_pipe_stays() {
        // P1 → P2 → P3 existing; a new branch P4 → P2.
        let mut net = run(3, 50.0, 315.0, Materiaal::Pvc, |_| 0.0);
        for l in &mut net.leidingen {
            l.bestaand = true;
        }
        net.leidingen[0].bob_van = -1.2;
        net.leidingen[0].bob_naar = -1.3;
        net.leidingen[1].bob_van = -1.3;
        net.leidingen[1].bob_naar = -1.4;
        net.putten.push(Put::new("P4".into(), [50.0, 60.0], Stelsel::Dwa, [800.0, 0.0]));
        net.leidingen.push(Leiding::new(3, 1, 250.0, Materiaal::Pvc, Stelsel::Dwa));
        let _ = net.sample_ground(&mut |_| Some(0.0), 1.0);
        let w = net.bereken().unwrap();
        assert_eq!(net.volgorde().unwrap(), vec![2]);
        assert_eq!((net.leidingen[0].bob_van, net.leidingen[1].bob_naar), (-1.2, -1.4));
        // −1.0427 − 60/250 = −1.2827: above the outgoing −1.30, no warning.
        assert!(w.is_empty(), "{w:?}");
        // A deep branch arrives below the existing pipe: a warning.
        net.leidingen[2].dekking_min = 1.2;
        let w = net.bereken().unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        // The longest path comes from the 60 m branch; from P101 it runs over the existing pipes.
        assert_eq!(net.pad(None, None).unwrap(), vec![2, 1]);
        assert_eq!(net.pad(Some(0), None).unwrap(), vec![0, 1]);
        assert!(net.pad(Some(2), Some(0)).is_err());
        // Upstream lengths: the outlet pipe drains everything.
        let up = net.bovenstrooms();
        assert!((up[1] - (50.0 + 50.0 + 60.0)).abs() < 1e-9);
    }

    #[test]
    fn a_loop_is_refused() {
        let mut net = run(3, 50.0, 250.0, Materiaal::Pvc, |_| 0.0);
        net.leidingen.push(Leiding::new(2, 0, 250.0, Materiaal::Pvc, Stelsel::Dwa));
        assert!(net.volgorde().is_err());
    }

    #[test]
    fn prandtl_colebrook_full_pipe() {
        // D 300 mm, 1:250, k 1 mm: v = -2 log(k/3.71D + 2.51ν/(D√(2gDI)))·√(2gDI) ≈ 0.925 m/s.
        let v = snelheid_vol(0.3, 0.004, 0.001);
        assert!((v - 0.9247).abs() < 0.001, "{v}");
        let q = capaciteit_vol(0.3, 0.004, 0.001);
        assert!((q - v * std::f64::consts::PI * 0.09 / 4.0).abs() < 1e-12);
        // Smoother and steeper pipes carry more.
        assert!(snelheid_vol(0.3, 0.004, 0.0004) > v && snelheid_vol(0.3, 0.008, 0.001) > v);
        assert_eq!(snelheid_vol(0.3, 0.0, 0.001), 0.0);
    }

    #[test]
    fn design_values_and_arguments() {
        assert_eq!(parse_verhang("1:250").unwrap(), 0.004);
        assert!((parse_verhang("4").unwrap() - 0.004).abs() < 1e-15);
        assert_eq!(parse_verhang("0,004").unwrap(), 0.004);
        assert!(parse_verhang("x").is_err() && parse_verhang("-1").is_err());
        assert_eq!(verhang_text(0.004), "1:250");
        assert_eq!(verhang_text(1.0 / 317.0), "1:315");
        assert_eq!(parse_maat("800x1000").unwrap(), [800.0, 1000.0]);
        assert_eq!(parse_maat("1000").unwrap(), [1000.0, 0.0]);
        assert!(parse_maat("50").is_err());
        assert_eq!(parse_oppervlak("1,25ha").unwrap(), 12_500.0);
        assert_eq!(parse_oppervlak("800").unwrap(), 800.0);
        assert_eq!(putnummers("P101", 3), vec!["P101", "P102", "P103"]);
        assert_eq!(putnummers("DWA-008", 3), vec!["DWA-008", "DWA-009", "DWA-010"]);
        assert_eq!(Materiaal::Pvc.wand(315.0), 9.2);
        assert!((Materiaal::Pvc.binnen(250.0) - 235.4).abs() < 1e-9);
        assert_eq!(Materiaal::Beton.wand(300.0), 55.0);
        assert_eq!(Materiaal::Beton.wand(550.0), 78.0);
        assert_eq!(Materiaal::Beton.binnen(300.0), 300.0);
        assert_eq!(Materiaal::Beton.buiten(300.0), 410.0);
        assert_eq!(min_verhang(250.0), 1.0 / 250.0);
        assert_eq!(standaard_put(250.0), [800.0, 0.0]);
        assert_eq!(standaard_put(500.0), [1000.0, 0.0]);
        assert_eq!(standaard_put(3000.0), [2000.0, 0.0]);
        assert_eq!(max_putafstand(315.0), 70.0);
        assert_eq!(Stelsel::parse("hemelwater"), Some(Stelsel::Hwa));
        assert_eq!(Stelsel::from_gwsw("Vuilwaterriool", ""), Stelsel::Dwa);
        assert_eq!(Materiaal::parse("Polypropyleen"), Some(Materiaal::Pp));
        let t = Toets { oppervlak: Some(10_000.0), inwoners: Some(300.0), ..Default::default() };
        assert!((t.debiet().unwrap().0 - (60.0 + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn layers_follow_the_gwsw_import() {
        assert_eq!(leiding_object(Stelsel::Dwa, Materiaal::Pvc, 250.0), "DWA_RIOOLLEIDING_PVC_250");
        assert_eq!(leiding_object(Stelsel::Hwa, Materiaal::Beton, 600.0), "HWA_RIOOLLEIDING_BETON_600");
        assert_eq!(put_object(Stelsel::Dwa), "DWA_RIOOLPUT_INSPECTIEPUT");
        for s in Stelsel::ALL {
            for what in ["PUTNUMMER", "PUTDEKSELNIVEAU", "BOB", "STROMINGSRICHTING"] {
                assert!(nlcs::object_by_name("RI", &format!("{}_{what}", s.code())).is_some(), "{} {what}", s.code());
            }
            assert!(nlcs::object_by_name("RI", &put_object(s)).is_some());
        }
    }

    #[test]
    fn a_designed_run_reads_back_from_gwsw_orox() {
        let mut net = run(3, 45.0, 250.0, Materiaal::Pvc, |_| 1.0);
        net.bereken().unwrap();
        let o = orox_of(&[&net], "riool_1", &|p| Ok([p[0] + 104_000.0, p[1] + 425_000.0])).unwrap();
        let ttl = gwsw::write_orox(&o);
        let data = gwsw::read_orox(&ttl).unwrap();
        let pipes = &data[0].1;
        assert_eq!(pipes.len(), 2);
        let p = pipes.iter().find(|f| f.props["naam"] == "P101-P102").unwrap();
        assert!((p.props["bob_beginpunt_leiding"].as_f64().unwrap() - net.leidingen[0].bob_van).abs() < 1e-3);
        assert!((p.props["bob_eindpunt_leiding"].as_f64().unwrap() - net.leidingen[0].bob_naar).abs() < 1e-3);
        assert_eq!(gwsw::nlcs_candidates("beheerleiding", p)[0], "DWA_RIOOLLEIDING_PVC_250");
        assert_eq!(data[1].1.len(), 3);
        let (puts, pipes) = csv_of(&[&net], &|p| Ok(p)).unwrap();
        assert_eq!(puts.lines().count(), 4);
        assert_eq!(pipes.lines().count(), 3);
        assert!(pipes.contains("P101-P102;P101;P102;DWA;PVC;250;235,4;7,3;45,000"), "{pipes}");
    }
}
