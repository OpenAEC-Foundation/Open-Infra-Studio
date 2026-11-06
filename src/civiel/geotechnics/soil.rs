//! Soil units and their parameters: the rows of NEN 9997-1 table 2.b
//! ("Karakteristieke waarden van grondeigenschappen"), the Dutch table of
//! characteristic soil properties by main soil name, admixture and
//! consistency.
//!
//! The standard itself is sold by NEN and is not public. The values below
//! were taken from a public reproduction of the table (Constructief
//! tabellenboek voor bouwkunde en civiele techniek, preview edition at
//! tabellenboek.eu, pp. 269–270, "Grondeigenschappen") and checked in part
//! against public Dutch geotechnical reports that quote it. Check them
//! against your copy of the standard before you rely on them; see
//! docs/geotechnics.md.
//!
//! Where the table gives a range (the stiff and the strongly mixed rows),
//! the default is the first value of the range, which is the lower strength
//! and the lower stiffness (and the matching higher compression ratio). The
//! consolidation coefficient cv is not in the table: its defaults are
//! estimates, marked as such.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::gis::bro::{Grond, Interval};

/// Unit weight of water (kN/m³).
pub const GAMMA_W: f64 = 9.81;

/// The main soil name of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Main {
    Gravel,
    Sand,
    Loam,
    Clay,
    Peat,
}

impl Main {
    /// Sand and gravel drain freely: no consolidation time, and they are
    /// the drainage boundaries of the clay and peat between them.
    pub fn draining(self) -> bool {
        matches!(self, Main::Gravel | Main::Sand)
    }
}

/// One row of table 2.b. Pairs are (first, last) of a range; single values
/// repeat. Units: kN/m³, MPa (qc, E100), degrees, kPa.
pub struct Row {
    /// The Dutch code, after the table's names: "klei_schoon_slap".
    pub code: &'static str,
    /// The English code: "clay_clean_soft".
    pub code_en: &'static str,
    pub nl: &'static str,
    pub en: &'static str,
    pub main: Main,
    /// The representative cone resistance of the row (MPa); the CPT
    /// classification picks the nearest row by it.
    pub qc: f64,
    pub gamma: (f64, f64),
    pub gamma_sat: (f64, f64),
    pub e100: (f64, f64),
    pub phi: (f64, f64),
    pub c: (f64, f64),
    /// Undrained shear strength (kPa), cohesive soils only.
    pub cu: Option<(f64, f64)>,
    /// Koppejan: primary (C'p) and secondary (C's) compression coefficients
    /// above the preconsolidation stress; C's is not given for sand and gravel.
    pub cp: (f64, f64),
    pub cs: Option<(f64, f64)>,
    /// Compression ratio Cc/(1+e0), secondary compression Cα and the
    /// swelling (reloading) ratio Csw/(1+e0).
    pub cr: (f64, f64),
    pub ca: (f64, f64),
    pub rr: (f64, f64),
}

const fn one(v: f64) -> (f64, f64) {
    (v, v)
}

macro_rules! row {
    ($code:literal, $en_code:literal, $nl:literal, $en:literal, $main:ident, qc $qc:expr, g $g:expr, gs $gs:expr, e $e:expr, phi $phi:expr, c $c:expr, cu $cu:expr, cp $cp:expr, cs $cs:expr, cr $cr:expr, ca $ca:expr, rr $rr:expr) => {
        Row { code: $code, code_en: $en_code, nl: $nl, en: $en, main: Main::$main, qc: $qc, gamma: $g, gamma_sat: $gs, e100: $e, phi: $phi, c: $c, cu: $cu, cp: $cp, cs: $cs, cr: $cr, ca: $ca, rr: $rr }
    };
}

/// NEN 9997-1 table 2.b, row by row (see the module documentation for the source).
pub static TABLE: &[Row] = &[
    row!("grind_zwak_siltig_los", "gravel_slightly_silty_loose", "Grind, zwak siltig, los", "Gravel, slightly silty, loose", Gravel,
        qc 15.0, g one(17.0), gs one(19.0), e one(45.0), phi one(32.5), c one(0.0), cu None, cp one(500.0), cs None, cr one(0.0046), ca one(0.0), rr one(0.0015)),
    row!("grind_zwak_siltig_matig", "gravel_slightly_silty_medium", "Grind, zwak siltig, matig", "Gravel, slightly silty, medium dense", Gravel,
        qc 25.0, g one(18.0), gs one(20.0), e one(75.0), phi one(35.0), c one(0.0), cu None, cp one(1000.0), cs None, cr one(0.0023), ca one(0.0), rr one(0.0006)),
    row!("grind_zwak_siltig_vast", "gravel_slightly_silty_dense", "Grind, zwak siltig, vast", "Gravel, slightly silty, dense", Gravel,
        qc 30.0, g (19.0, 20.0), gs (21.0, 22.0), e (90.0, 105.0), phi (37.5, 40.0), c one(0.0), cu None, cp (1200.0, 1400.0), cs None, cr (0.0019, 0.0016), ca one(0.0), rr (0.0006, 0.0005)),
    row!("grind_sterk_siltig_los", "gravel_very_silty_loose", "Grind, sterk siltig, los", "Gravel, very silty, loose", Gravel,
        qc 10.0, g one(18.0), gs one(20.0), e one(30.0), phi one(30.0), c one(0.0), cu None, cp one(400.0), cs None, cr one(0.0058), ca one(0.0), rr one(0.0019)),
    row!("grind_sterk_siltig_matig", "gravel_very_silty_medium", "Grind, sterk siltig, matig", "Gravel, very silty, medium dense", Gravel,
        qc 15.0, g one(19.0), gs one(21.0), e one(45.0), phi one(32.5), c one(0.0), cu None, cp one(600.0), cs None, cr one(0.0038), ca one(0.0), rr one(0.0013)),
    row!("grind_sterk_siltig_vast", "gravel_very_silty_dense", "Grind, sterk siltig, vast", "Gravel, very silty, dense", Gravel,
        qc 25.0, g (20.0, 21.0), gs (22.0, 22.5), e (75.0, 110.0), phi (35.0, 40.0), c one(0.0), cu None, cp (1000.0, 1500.0), cs None, cr (0.0023, 0.0015), ca one(0.0), rr (0.0008, 0.0005)),
    row!("zand_schoon_los", "sand_clean_loose", "Zand, schoon, los", "Sand, clean, loose", Sand,
        qc 5.0, g one(17.0), gs one(19.0), e one(15.0), phi one(30.0), c one(0.0), cu None, cp one(200.0), cs None, cr one(0.0115), ca one(0.0), rr one(0.0038)),
    row!("zand_schoon_matig", "sand_clean_medium", "Zand, schoon, matig", "Sand, clean, medium dense", Sand,
        qc 15.0, g one(18.0), gs one(20.0), e one(45.0), phi one(32.5), c one(0.0), cu None, cp one(600.0), cs None, cr one(0.0038), ca one(0.0), rr one(0.0013)),
    row!("zand_schoon_vast", "sand_clean_dense", "Zand, schoon, vast", "Sand, clean, dense", Sand,
        qc 25.0, g (19.0, 20.0), gs (21.0, 22.0), e (75.0, 110.0), phi (35.0, 40.0), c one(0.0), cu None, cp (1000.0, 1500.0), cs None, cr (0.0023, 0.0015), ca one(0.0), rr (0.0006, 0.0005)),
    row!("zand_zwak_siltig", "sand_slightly_silty", "Zand, zwak siltig, kleiig", "Sand, slightly silty or clayey", Sand,
        qc 12.0, g (18.0, 19.0), gs (20.0, 21.0), e (35.0, 50.0), phi (27.0, 32.5), c one(0.0), cu None, cp (450.0, 650.0), cs None, cr (0.0051, 0.0035), ca one(0.0), rr (0.0017, 0.0012)),
    row!("zand_sterk_siltig", "sand_very_silty", "Zand, sterk siltig, kleiig", "Sand, very silty or clayey", Sand,
        qc 8.0, g (18.0, 19.0), gs (20.0, 21.0), e (15.0, 30.0), phi (25.0, 30.0), c one(0.0), cu None, cp (200.0, 400.0), cs None, cr (0.0115, 0.0058), ca one(0.0), rr (0.0038, 0.0019)),
    row!("leem_zwak_zandig_slap", "loam_slightly_sandy_soft", "Leem, zwak zandig, slap", "Loam, slightly sandy, soft", Loam,
        qc 1.0, g one(19.0), gs one(19.0), e one(2.0), phi (27.5, 30.0), c one(0.0), cu Some(one(50.0)), cp one(25.0), cs Some(one(650.0)), cr one(0.0920), ca one(0.0037), rr one(0.0307)),
    row!("leem_zwak_zandig_matig", "loam_slightly_sandy_firm", "Leem, zwak zandig, matig", "Loam, slightly sandy, firm", Loam,
        qc 2.0, g one(20.0), gs one(20.0), e one(3.0), phi (27.5, 32.5), c one(1.0), cu Some(one(100.0)), cp one(45.0), cs Some(one(1300.0)), cr one(0.0511), ca one(0.0020), rr one(0.0170)),
    row!("leem_zwak_zandig_vast", "loam_slightly_sandy_stiff", "Leem, zwak zandig, vast", "Loam, slightly sandy, stiff", Loam,
        qc 3.0, g (21.0, 22.0), gs (21.0, 22.0), e (5.0, 7.0), phi (27.5, 35.0), c (2.5, 3.8), cu Some((200.0, 300.0)), cp (70.0, 100.0), cs Some((1900.0, 2500.0)), cr (0.0329, 0.0230), ca (0.0013, 0.0009), rr (0.0110, 0.0077)),
    row!("leem_sterk_zandig", "loam_very_sandy", "Leem, sterk zandig", "Loam, very sandy", Loam,
        qc 2.0, g (19.0, 20.0), gs (19.0, 20.0), e (3.0, 5.0), phi (27.5, 35.0), c (0.0, 1.0), cu Some((50.0, 100.0)), cp (45.0, 70.0), cs Some((1300.0, 2000.0)), cr (0.0511, 0.0329), ca (0.0020, 0.0013), rr (0.0170, 0.0110)),
    row!("klei_schoon_slap", "clay_clean_soft", "Klei, schoon, slap", "Clay, clean, soft", Clay,
        qc 0.5, g one(14.0), gs one(14.0), e one(1.0), phi one(17.5), c one(0.0), cu Some(one(25.0)), cp one(7.0), cs Some(one(80.0)), cr one(0.3286), ca one(0.0131), rr one(0.1095)),
    row!("klei_schoon_matig", "clay_clean_firm", "Klei, schoon, matig", "Clay, clean, firm", Clay,
        qc 1.0, g one(17.0), gs one(17.0), e one(2.0), phi one(17.5), c one(5.0), cu Some(one(50.0)), cp one(15.0), cs Some(one(160.0)), cr one(0.1533), ca one(0.0061), rr one(0.0511)),
    row!("klei_schoon_vast", "clay_clean_stiff", "Klei, schoon, vast", "Clay, clean, stiff", Clay,
        qc 2.0, g (19.0, 20.0), gs (19.0, 20.0), e (4.0, 10.0), phi (17.5, 25.0), c (13.0, 15.0), cu Some((100.0, 200.0)), cp (25.0, 30.0), cs Some((320.0, 500.0)), cr (0.0920, 0.0767), ca (0.0037, 0.0031), rr (0.0307, 0.0256)),
    row!("klei_zwak_zandig_slap", "clay_slightly_sandy_soft", "Klei, zwak zandig, slap", "Clay, slightly sandy, soft", Clay,
        qc 0.7, g one(15.0), gs one(15.0), e one(1.5), phi one(22.5), c one(0.0), cu Some(one(40.0)), cp one(10.0), cs Some(one(110.0)), cr one(0.2300), ca one(0.0092), rr one(0.0767)),
    row!("klei_zwak_zandig_matig", "clay_slightly_sandy_firm", "Klei, zwak zandig, matig", "Clay, slightly sandy, firm", Clay,
        qc 1.5, g one(18.0), gs one(18.0), e one(3.0), phi one(22.5), c one(5.0), cu Some(one(80.0)), cp one(20.0), cs Some(one(240.0)), cr one(0.1150), ca one(0.0046), rr one(0.0383)),
    row!("klei_zwak_zandig_vast", "clay_slightly_sandy_stiff", "Klei, zwak zandig, vast", "Clay, slightly sandy, stiff", Clay,
        qc 2.5, g (20.0, 21.0), gs (20.0, 21.0), e (5.0, 10.0), phi (22.5, 27.5), c (13.0, 15.0), cu Some((120.0, 170.0)), cp (30.0, 50.0), cs Some((400.0, 600.0)), cr (0.0767, 0.0460), ca (0.0031, 0.0018), rr (0.0256, 0.0153)),
    row!("klei_sterk_zandig", "clay_very_sandy", "Klei, sterk zandig", "Clay, very sandy", Clay,
        qc 1.0, g (18.0, 20.0), gs (18.0, 20.0), e (2.0, 5.0), phi (27.5, 32.5), c (0.0, 1.0), cu Some((0.0, 10.0)), cp (25.0, 140.0), cs Some((320.0, 1680.0)), cr (0.0920, 0.0164), ca (0.0037, 0.0007), rr (0.0307, 0.0055)),
    row!("klei_organisch_slap", "clay_organic_soft", "Klei, organisch, slap", "Clay, organic, soft", Clay,
        qc 0.2, g one(13.0), gs one(13.0), e one(0.5), phi one(15.0), c (0.0, 1.0), cu Some(one(10.0)), cp one(7.5), cs Some(one(30.0)), cr one(0.3067), ca one(0.0153), rr one(0.1022)),
    row!("klei_organisch_matig", "clay_organic_firm", "Klei, organisch, matig", "Clay, organic, firm", Clay,
        qc 0.5, g (15.0, 16.0), gs (15.0, 16.0), e (1.0, 2.0), phi one(15.0), c (0.0, 1.0), cu Some((25.0, 30.0)), cp (10.0, 15.0), cs Some((40.0, 60.0)), cr (0.2300, 0.1533), ca (0.0115, 0.0077), rr (0.0767, 0.0511)),
    row!("veen_slap", "peat_soft", "Veen, niet voorbelast, slap", "Peat, not preloaded, soft", Peat,
        qc 0.1, g (10.0, 12.0), gs (10.0, 12.0), e (0.2, 0.5), phi one(15.0), c (1.0, 2.5), cu Some((10.0, 20.0)), cp (5.0, 7.5), cs Some((20.0, 30.0)), cr (0.4600, 0.3067), ca (0.0230, 0.0153), rr (0.1533, 0.1022)),
    row!("veen_matig", "peat_firm", "Veen, matig voorbelast, matig", "Peat, moderately preloaded, firm", Peat,
        qc 0.2, g (12.0, 13.0), gs (12.0, 13.0), e (0.5, 1.0), phi one(15.0), c (2.5, 5.0), cu Some((20.0, 30.0)), cp (7.5, 10.0), cs Some((30.0, 40.0)), cr (0.3067, 0.2300), ca (0.0153, 0.0115), rr (0.1022, 0.0767)),
];

/// The consolidation coefficient (m²/s) of a cohesive row. Not in table
/// 2.b: an **estimate** of the order of magnitude for Holocene Dutch
/// deposits (soft clay 10⁻⁸–10⁻⁷, sandy clay and loam higher, peat about
/// 10⁻⁷ at first). Replace it with oedometer values (`GROND=<code>:cv=`).
fn cv_estimate(row: &Row) -> Option<f64> {
    Some(match row.main {
        Main::Gravel | Main::Sand => return None,
        Main::Loam => 3e-7,
        Main::Peat => 1e-7,
        Main::Clay if row.code.contains("sterk_zandig") => 1e-6,
        Main::Clay if row.code.contains("zwak_zandig") => 1e-7,
        Main::Clay => 3e-8,
    })
}

/// The parameters of one soil unit as the calculations use them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// Unit weight above and below the phreatic level (kN/m³).
    pub gamma: f64,
    pub gamma_sat: f64,
    /// Effective cohesion (kPa) and friction angle (degrees).
    pub c: f64,
    pub phi: f64,
    /// Koppejan above the preconsolidation stress: C'p and C's.
    pub cp: f64,
    pub cs: Option<f64>,
    /// Koppejan below the preconsolidation stress: Cp = ln 10 / (Csw/(1+e0))
    /// from the table's reloading ratio, Cs = C's · Cp / C'p (an estimate).
    pub cp_pre: f64,
    pub cs_pre: Option<f64>,
    /// Compression ratio Cc/(1+e0), reloading ratio Csw/(1+e0), secondary Cα.
    pub cr: f64,
    pub rr: f64,
    pub ca: f64,
    /// Consolidation coefficient (m²/s); `None` drains freely.
    pub cv: Option<f64>,
    /// Pre-overburden pressure σ'p − σ'0 (kPa); 0 = normally consolidated.
    pub pop: f64,
    /// Where the values come from: "NEN 9997-1 tabel 2.b", plus what was overridden.
    pub source: String,
}

impl Row {
    pub fn label(&self) -> &'static str {
        crate::lang::tr(self.nl, self.en)
    }

    /// The table values as parameters (the first value of every range).
    pub fn params(&self) -> Params {
        let rr = self.rr.0;
        let cp_pre = if rr > 0.0 { std::f64::consts::LN_10 / rr } else { self.cp.0 * 3.0 };
        let cs = self.cs.map(|c| c.0);
        Params {
            gamma: self.gamma.0,
            gamma_sat: self.gamma_sat.0,
            c: self.c.0,
            phi: self.phi.0,
            cp: self.cp.0,
            cs,
            cp_pre,
            cs_pre: cs.map(|c| c * cp_pre / self.cp.0),
            cr: self.cr.0,
            rr,
            ca: self.ca.0,
            cv: cv_estimate(self),
            pop: 0.0,
            source: "NEN 9997-1 tabel 2.b; cv estimate".into(),
        }
    }

    pub fn grond(&self) -> Grond {
        match self.main {
            Main::Gravel => Grond::Grind,
            Main::Sand if self.code.contains("siltig") && !self.code.starts_with("zand_schoon") => Grond::SiltigZand,
            Main::Sand => Grond::Zand,
            Main::Loam => Grond::Silt,
            Main::Clay => Grond::Klei,
            Main::Peat => Grond::Veen,
        }
    }

    /// The fill colour: the conventional colour of the main soil, lighter
    /// for loose and soft, darker for dense and stiff.
    pub fn rgb(&self) -> (u8, u8, u8) {
        let (r, g, b) = if self.code.contains("organisch") { (104, 128, 64) } else { self.grond().rgb() };
        let f = if self.code.ends_with("_los") || self.code.ends_with("_slap") {
            1.12
        } else if self.code.ends_with("_vast") {
            0.78
        } else if self.code.ends_with("_matig") {
            0.95
        } else {
            1.0
        };
        let s = |v: u8| ((v as f64) * f).round().clamp(0.0, 255.0) as u8;
        (s(r), s(g), s(b))
    }
}

/// The row with this code (Dutch or English, case and separators ignored).
pub fn row(code: &str) -> Option<&'static Row> {
    let norm = |s: &str| s.to_ascii_lowercase().replace(['-', ' ', ','], "_");
    let c = norm(code);
    TABLE.iter().find(|r| r.code == c || r.code_en == c)
}

/// Overrides of the parameters, per unit: `code:key=v,key=v;code:…`. Keys:
/// g, gsat, c, phi, cp, cs, cppre, cspre, cr, rr, ca, cc and e0 (together:
/// cr = cc/(1+e0)), cv (m²/s, 0 = free draining), pop (kPa).
pub fn parse_overrides(text: &str) -> Result<BTreeMap<String, Vec<(String, f64)>>, String> {
    let mut out: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (code, rest) = part.split_once(':').ok_or_else(|| format!("GROND: {} '{part}'", crate::lang::tr("verwacht code:sleutel=waarde, kreeg", "expected code:key=value, got")))?;
        let r = row(code.trim()).ok_or_else(|| unknown_code(code))?;
        for kv in rest.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("GROND: '{kv}'"))?;
            let k = k.trim().to_ascii_lowercase();
            if !["g", "gsat", "c", "phi", "cp", "cs", "cppre", "cspre", "cr", "rr", "ca", "cc", "e0", "cv", "pop"].contains(&k.as_str()) {
                return Err(format!("GROND: {} {k} (g, gsat, c, phi, cp, cs, cppre, cspre, cr, rr, ca, cc, e0, cv, pop)", crate::lang::tr("onbekende sleutel", "unknown key")));
            }
            let v: f64 = v.trim().replace(',', ".").parse().map_err(|_| format!("GROND: {k}={v}"))?;
            if !v.is_finite() || v < 0.0 || (v == 0.0 && !matches!(k.as_str(), "c" | "pop" | "cv" | "ca")) {
                return Err(format!("GROND: {k}={v} {}", crate::lang::tr("moet positief zijn", "must be positive")));
            }
            out.entry(r.code.to_string()).or_default().push((k, v));
        }
    }
    Ok(out)
}

pub fn unknown_code(code: &str) -> String {
    format!("{} '{}' ({})", crate::lang::tr("onbekende grondsoort", "unknown soil unit"), code.trim(), crate::lang::tr("CIV_GRONDSOORTEN geeft de lijst", "CIV_GRONDSOORTEN lists them"))
}

/// The table parameters of `code` with its overrides applied.
pub fn params_with(code: &str, overrides: &BTreeMap<String, Vec<(String, f64)>>) -> Result<Params, String> {
    let r = row(code).ok_or_else(|| unknown_code(code))?;
    let mut p = r.params();
    let Some(list) = overrides.get(r.code) else { return Ok(p) };
    let (mut cc, mut e0) = (None, None);
    let mut keys = Vec::new();
    for (k, v) in list {
        match k.as_str() {
            "g" => p.gamma = *v,
            "gsat" => p.gamma_sat = *v,
            "c" => p.c = *v,
            "phi" => p.phi = *v,
            "cp" => p.cp = *v,
            "cs" => p.cs = Some(*v),
            "cppre" => p.cp_pre = *v,
            "cspre" => p.cs_pre = Some(*v),
            "cr" => p.cr = *v,
            "rr" => p.rr = *v,
            "ca" => p.ca = *v,
            "cc" => cc = Some(*v),
            "e0" => e0 = Some(*v),
            "cv" => p.cv = (*v > 0.0).then_some(*v),
            "pop" => p.pop = *v,
            _ => {}
        }
        keys.push(k.clone());
    }
    match (cc, e0) {
        (Some(cc), Some(e0)) => p.cr = cc / (1.0 + e0),
        (None, None) => {}
        _ => return Err(format!("GROND {}: {}", r.code, crate::lang::tr("geef cc en e0 samen", "give cc and e0 together"))),
    }
    keys.sort();
    keys.dedup();
    p.source = format!("NEN 9997-1 tabel 2.b; {}: {}", crate::lang::tr("eigen waarden", "own values"), keys.join(", "));
    Ok(p)
}

/// The row of `candidates` whose representative qc is nearest to `qc` on a
/// log scale.
fn nearest_qc(candidates: &[&'static str], qc: f64) -> &'static str {
    let q = qc.max(0.01).ln();
    candidates
        .iter()
        .copied()
        .min_by(|a, b| {
            let d = |c: &str| (row(c).map(|r| r.qc).unwrap_or(1.0).ln() - q).abs();
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(candidates[0])
}

/// The unit of a CPT layer: its Robertson (2010) soil behaviour type gives
/// the main soil and admixture, the layer's mean cone resistance the
/// consistency (the row with the nearest representative qc).
///
/// SBT 2 → peat; 3 → clean clay; 4 (silty clay to clayey silt) →
/// slightly sandy clay; 5 (silty sand to sandy silt) → very sandy clay or
/// very silty sand by qc; 6 → clean sand; 7 (gravelly to dense sand) →
/// clean sand, medium or dense.
pub fn of_cpt(layer: &Interval) -> Option<&'static str> {
    let qc = layer.qc?;
    Some(match layer.soil {
        Grond::Veen => nearest_qc(&["veen_slap", "veen_matig"], qc),
        Grond::Klei => nearest_qc(&["klei_schoon_slap", "klei_schoon_matig", "klei_schoon_vast"], qc),
        Grond::Silt => nearest_qc(&["klei_zwak_zandig_slap", "klei_zwak_zandig_matig", "klei_zwak_zandig_vast"], qc),
        Grond::SiltigZand => nearest_qc(&["klei_sterk_zandig", "zand_sterk_siltig"], qc),
        Grond::Zand => nearest_qc(&["zand_schoon_los", "zand_schoon_matig", "zand_schoon_vast"], qc),
        Grond::Grind => nearest_qc(&["zand_schoon_matig", "zand_schoon_vast"], qc),
        Grond::Antropogeen | Grond::Onbekend => return None,
    })
}

/// The unit of a described borehole layer (NEN-EN-ISO 14688 or NEN 5104
/// name, as [`crate::gis::bro::soil_of_name`] reads it): main soil and
/// admixture from the name; a borehole gives no consistency, so the soft or
/// loose row is taken (the less favourable one).
pub fn of_borehole(layer: &Interval) -> Option<&'static str> {
    let n = layer.name.to_ascii_lowercase();
    let strong = |w: &str| ["sterk", "matig", "uiterst"].iter().any(|g| n.contains(&format!("{g} {w}")));
    let any = |w: &str| n.contains(w);
    Some(match layer.soil {
        Grond::Veen => "veen_slap",
        Grond::Klei if any("humeu") || any("venig") || any("organ") => "klei_organisch_slap",
        Grond::Klei if strong("zandig") => "klei_sterk_zandig",
        Grond::Klei if any("zandig") => "klei_zwak_zandig_slap",
        Grond::Klei => "klei_schoon_slap",
        Grond::Silt if strong("zandig") => "leem_sterk_zandig",
        Grond::Silt => "leem_zwak_zandig_slap",
        Grond::SiltigZand => "zand_sterk_siltig",
        Grond::Zand if any("siltig") || any("kleiig") => "zand_zwak_siltig",
        Grond::Zand => "zand_schoon_los",
        Grond::Grind if strong("siltig") => "grind_sterk_siltig_los",
        Grond::Grind => "grind_zwak_siltig_los",
        // Made ground (rubble, pavement): taken as loose sand, an estimate.
        Grond::Antropogeen => "zand_schoon_los",
        Grond::Onbekend => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_consistent() {
        // The compression ratio is ln 10 / C'p (the table's own relation
        // between Koppejan's C'p and Cc/(1+e0)): 2.3026 / 7 = 0.329.
        for r in TABLE {
            let rel = std::f64::consts::LN_10 / r.cp.0;
            assert!((rel - r.cr.0).abs() / r.cr.0 < 0.03, "{}: ln10/C'p {rel:.4} vs Cc/(1+e0) {}", r.code, r.cr.0);
            assert!(r.gamma.0 <= r.gamma_sat.0, "{}", r.code);
            assert!(row(r.code_en).is_some_and(|x| x.code == r.code));
        }
        assert_eq!(TABLE.len(), 26);
        let p = row("klei_schoon_slap").unwrap().params();
        assert_eq!((p.gamma, p.gamma_sat, p.c, p.phi, p.cp, p.cs), (14.0, 14.0, 0.0, 17.5, 7.0, Some(80.0)));
        // Cp below the preconsolidation stress: ln 10 / 0.1095 = 21 ≈ 3 C'p.
        assert!((p.cp_pre - 21.03).abs() < 0.05);
        assert_eq!(row("Clay clean soft").unwrap().code, "klei_schoon_slap");
    }

    #[test]
    fn cpt_layers_take_the_row_nearest_in_qc() {
        let l = |soil, qc| Interval { top: 0.0, bottom: 1.0, soil, name: String::new(), qc: Some(qc), rf: Some(3.0) };
        assert_eq!(of_cpt(&l(Grond::Klei, 0.4)), Some("klei_schoon_slap"));
        assert_eq!(of_cpt(&l(Grond::Klei, 1.2)), Some("klei_schoon_matig"));
        assert_eq!(of_cpt(&l(Grond::Veen, 0.25)), Some("veen_matig"));
        assert_eq!(of_cpt(&l(Grond::Zand, 18.0)), Some("zand_schoon_matig"));
        assert_eq!(of_cpt(&l(Grond::Zand, 30.0)), Some("zand_schoon_vast"));
        assert_eq!(of_cpt(&l(Grond::SiltigZand, 1.5)), Some("klei_sterk_zandig"));
        assert_eq!(of_cpt(&l(Grond::SiltigZand, 6.0)), Some("zand_sterk_siltig"));
        let b = |soil, name: &str| Interval { top: 0.0, bottom: 1.0, soil, name: name.into(), qc: None, rf: None };
        assert_eq!(of_borehole(&b(Grond::Klei, "sterk zandige klei")), Some("klei_sterk_zandig"));
        assert_eq!(of_borehole(&b(Grond::Klei, "zwak zandige klei")), Some("klei_zwak_zandig_slap"));
        assert_eq!(of_borehole(&b(Grond::Klei, "matig humeuze klei")), Some("klei_organisch_slap"));
        assert_eq!(of_borehole(&b(Grond::Zand, "kleiig zand")), Some("zand_zwak_siltig"));
        assert_eq!(of_borehole(&b(Grond::Onbekend, "niet beschreven")), None);
    }

    #[test]
    fn overrides_apply_per_unit() {
        let o = parse_overrides("veen_slap:c=2,phi=17;klei_schoon_slap:cc=0.5,e0=1.5,cv=1e-8").unwrap();
        let p = params_with("veen_slap", &o).unwrap();
        assert_eq!((p.c, p.phi), (2.0, 17.0));
        assert!(p.source.contains("c, phi"));
        let k = params_with("klei_schoon_slap", &o).unwrap();
        assert!((k.cr - 0.2).abs() < 1e-12 && k.cv == Some(1e-8));
        assert!(parse_overrides("veen_slap:x=1").is_err());
        assert!(parse_overrides("rots:c=1").is_err());
        assert!(params_with("klei_schoon_slap", &parse_overrides("klei_schoon_slap:cc=0.5").unwrap()).is_err());
    }
}
