//! Design storms (hyetographs) shared by storage routing ([`crate::berging`])
//! and the storm stress test on terrain.
//!
//! A storm is a list of blocks, each with a duration and a constant intensity
//! in mm/h. That is the form in which Dutch standard storms are published, it
//! integrates exactly, and any measured series can be brought into it.
//!
//! Built in:
//! - the ten standard storms (*standaardbuien*) 01–10 of Stichting RIONED,
//!   *Leidraad Riolering* module C2100 "Rioleringsberekeningen, hydraulisch
//!   functioneren" (August 2004), appendix 3, table B3.2, in mm per 5 minutes.
//!   Bui 08 (T = 2 years, peak at the end, 19.8 mm in 60 min, peak
//!   39.6 mm/h = 110 l/s/ha) is the usual design check for sewer capacity.
//! - the standard events of the DPRA stress test *wateroverlast*:
//!   "Notitie standaarden stresstest wateroverlast", 7 April 2020, updated
//!   November 2024 (klimaatadaptatienederland.nl), table 4, climate 2050:
//!   local 70 mm / 1 h (T = 100), 90 mm / 1 h (T = 250), 155 mm / 2 h
//!   (T = 1000), 200 mm / 48 h; regional 130 mm / 48 h (T = 100),
//!   150 mm / 48 h (T = 250), 185 mm / 48 h (T = 1000). The note prescribes
//!   constant intensity over the duration, so these are single blocks.
//! - a uniform block storm (mm over minutes) and a reader for "minutes,mm/h"
//!   or "minutes,mm" text, for anything else (measured storms, other norms).
//!
//! Other storms are added by writing their blocks with [`Hyetograph::new`] or
//! [`Hyetograph::from_depths`]; [`Hyetograph::scaled`] gives climate or
//! "bui 10 + 50 %" style variants.

use serde::{Deserialize, Serialize};

/// Block length of the Leidraad Riolering standard storms: 5 minutes
/// (C2100, table B3.2).
pub const LEIDRAAD_BLOCK_S: f64 = 300.0;

/// Leidraad Riolering C2100 appendix 3, table B3.2: rainfall in mm per
/// 5 minutes for standard storms 01–10, from the start of the storm.
/// 01/02: T = 0.25 y (peak early/late), 03/04: 0.5 y, 05/06: 1 y,
/// 07/08: 2 y, 09: 5 y, 10: 10 y.
const LEIDRAAD_MM_PER_5MIN: [&[f64]; 10] = [
    // 01, T = 0.25 y, peak early, 10.5 mm
    &[
        0.30, 0.60, 0.90, 1.20, 1.50, 1.50, 1.05, 0.90, 0.75, 0.60, 0.45, 0.30, 0.15, 0.15, 0.15,
    ],
    // 02, T = 0.25 y, peak late, 10.5 mm
    &[
        0.15, 0.15, 0.15, 0.30, 0.45, 0.60, 0.75, 0.90, 1.05, 1.50, 1.50, 1.20, 0.90, 0.60, 0.30,
    ],
    // 03, T = 0.5 y, peak early, 14.4 mm
    &[
        0.30, 0.60, 0.90, 1.50, 2.10, 2.10, 1.50, 1.20, 1.05, 0.90, 0.75, 0.60, 0.45, 0.30, 0.15,
    ],
    // 04, T = 0.5 y, peak late, 14.4 mm
    &[
        0.15, 0.30, 0.45, 0.60, 0.75, 0.90, 1.05, 1.20, 1.50, 2.10, 2.10, 1.50, 0.90, 0.60, 0.30,
    ],
    // 05, T = 1 y, peak early, 16.8 mm
    &[
        0.30, 0.60, 1.50, 2.70, 2.70, 2.10, 1.50, 1.20, 1.05, 0.90, 0.75, 0.60, 0.45, 0.30, 0.15,
    ],
    // 06, T = 1 y, peak late, 16.8 mm
    &[
        0.15, 0.30, 0.45, 0.60, 0.75, 0.90, 1.05, 1.20, 1.50, 2.10, 2.70, 2.70, 1.50, 0.60, 0.30,
    ],
    // 07, T = 2 y, peak early, 19.8 mm
    &[
        0.60, 1.20, 2.10, 3.30, 3.30, 2.70, 2.10, 1.50, 1.20, 0.90, 0.60, 0.30,
    ],
    // 08, T = 2 y, peak late, 19.8 mm
    &[
        0.30, 0.60, 0.90, 1.20, 1.50, 2.10, 2.70, 3.30, 3.30, 2.10, 1.20, 0.60,
    ],
    // 09, T = 5 y, 29.4 mm
    &[
        1.50, 2.70, 4.80, 4.80, 4.20, 3.30, 2.70, 2.10, 1.50, 0.90, 0.60, 0.30,
    ],
    // 10, T = 10 y, 35.7 mm
    &[1.80, 3.60, 6.30, 6.30, 5.70, 4.80, 3.60, 2.40, 1.20],
];

/// One block of a hyetograph: constant intensity over a duration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Duration of the block (s).
    pub duration_s: f64,
    /// Rain intensity during the block (mm/h).
    pub intensity_mm_h: f64,
}

/// A design or measured storm as consecutive blocks from t = 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hyetograph {
    /// Name for reports, e.g. "bui 08".
    pub name: String,
    /// Consecutive blocks; the storm starts at t = 0.
    pub blocks: Vec<Block>,
}

/// Meaning of the second column of a storm CSV.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvValue {
    /// Intensity during the block (mm/h).
    IntensityMmH,
    /// Rain depth fallen in the block (mm).
    DepthMm,
}

impl Hyetograph {
    /// A storm from blocks; durations must be positive, intensities finite and ≥ 0.
    pub fn new(name: &str, blocks: Vec<Block>) -> Result<Self, String> {
        if blocks.is_empty() {
            return Err(format!("storm '{name}' has no blocks"));
        }
        for (i, b) in blocks.iter().enumerate() {
            if !(b.duration_s.is_finite() && b.duration_s > 0.0) {
                return Err(format!("storm '{name}' block {i}: duration must be > 0"));
            }
            if !(b.intensity_mm_h.is_finite() && b.intensity_mm_h >= 0.0) {
                return Err(format!("storm '{name}' block {i}: intensity must be ≥ 0"));
            }
        }
        Ok(Hyetograph {
            name: name.to_string(),
            blocks,
        })
    }

    /// A storm from equal blocks given as depth (mm) per block.
    pub fn from_depths(name: &str, block_s: f64, depths_mm: &[f64]) -> Result<Self, String> {
        if !(block_s.is_finite() && block_s > 0.0) {
            return Err(format!("storm '{name}': block length must be > 0"));
        }
        let blocks = depths_mm
            .iter()
            .map(|d| Block {
                duration_s: block_s,
                intensity_mm_h: d * 3600.0 / block_s,
            })
            .collect();
        Hyetograph::new(name, blocks)
    }

    /// Total duration (s).
    pub fn duration_s(&self) -> f64 {
        self.blocks.iter().map(|b| b.duration_s).sum()
    }

    /// Total rain depth (mm).
    pub fn total_depth_mm(&self) -> f64 {
        self.blocks
            .iter()
            .map(|b| b.intensity_mm_h * b.duration_s / 3600.0)
            .sum()
    }

    /// Highest block intensity (mm/h).
    pub fn peak_intensity_mm_h(&self) -> f64 {
        self.blocks
            .iter()
            .map(|b| b.intensity_mm_h)
            .fold(0.0, f64::max)
    }

    /// Intensity (mm/h) at time `t_s`; 0 outside the storm. Block ends belong
    /// to the next block.
    pub fn intensity_at(&self, t_s: f64) -> f64 {
        if t_s < 0.0 {
            return 0.0;
        }
        let mut start = 0.0;
        for b in &self.blocks {
            if t_s < start + b.duration_s {
                return b.intensity_mm_h;
            }
            start += b.duration_s;
        }
        0.0
    }

    /// Exact rain depth (mm) between `t0_s` and `t1_s`.
    pub fn depth_between_mm(&self, t0_s: f64, t1_s: f64) -> f64 {
        if t1_s <= t0_s {
            return 0.0;
        }
        let mut start = 0.0;
        let mut depth = 0.0;
        for b in &self.blocks {
            let end = start + b.duration_s;
            let overlap = end.min(t1_s) - start.max(t0_s);
            if overlap > 0.0 {
                depth += b.intensity_mm_h * overlap / 3600.0;
            }
            if end >= t1_s {
                break;
            }
            start = end;
        }
        depth
    }

    /// Mean intensity (mm/h) per step of `dt_s`, from t = 0 until the step
    /// that contains the end of the storm. Depth is conserved exactly.
    pub fn resample(&self, dt_s: f64) -> Result<Vec<f64>, String> {
        if !(dt_s.is_finite() && dt_s > 0.0) {
            return Err("time step must be > 0".into());
        }
        let n = (self.duration_s() / dt_s - 1e-9).ceil().max(1.0) as usize;
        Ok((0..n)
            .map(|k| {
                let t0 = k as f64 * dt_s;
                self.depth_between_mm(t0, t0 + dt_s) * 3600.0 / dt_s
            })
            .collect())
    }

    /// The same storm with all intensities multiplied by `factor` (e.g. a
    /// climate factor).
    pub fn scaled(&self, factor: f64) -> Hyetograph {
        Hyetograph {
            name: format!("{} × {factor}", self.name),
            blocks: self
                .blocks
                .iter()
                .map(|b| Block {
                    duration_s: b.duration_s,
                    intensity_mm_h: b.intensity_mm_h * factor,
                })
                .collect(),
        }
    }

    /// A storm from text with one block per line: "minutes,value".
    ///
    /// `minutes` is the time at the END of the block, counted from the start
    /// of the storm (as rain-gauge series are stamped), so it must increase;
    /// the first block starts at 0. `value` is mm/h or mm per block, as set by
    /// `unit`. Separators: comma, semicolon, tab or spaces; with semicolon or
    /// tab a decimal comma is accepted. Empty lines, lines starting with '#'
    /// and a header before the first data line are skipped.
    pub fn from_csv(name: &str, text: &str, unit: CsvValue) -> Result<Self, String> {
        let mut blocks = Vec::new();
        let mut prev_min = 0.0;
        for (nr, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<String> = if line.contains(';') || line.contains('\t') {
                line.split([';', '\t'])
                    .map(|f| f.trim().replace(',', "."))
                    .collect()
            } else if line.contains(',') {
                line.split(',').map(|f| f.trim().to_string()).collect()
            } else {
                line.split_whitespace().map(str::to_string).collect()
            };
            let parsed =
                (fields.len() >= 2).then(|| (fields[0].parse::<f64>(), fields[1].parse::<f64>()));
            let (t, v) = match parsed {
                Some((Ok(t), Ok(v))) => (t, v),
                _ if blocks.is_empty() => continue, // header
                _ => {
                    return Err(format!(
                        "storm '{name}' line {}: expected 'minutes,value'",
                        nr + 1
                    ))
                }
            };
            if t.is_nan() || t <= prev_min {
                return Err(format!(
                    "storm '{name}' line {}: time {t} min must be after {prev_min} min",
                    nr + 1
                ));
            }
            let dur_s = (t - prev_min) * 60.0;
            let intensity = match unit {
                CsvValue::IntensityMmH => v,
                CsvValue::DepthMm => v * 3600.0 / dur_s,
            };
            blocks.push(Block {
                duration_s: dur_s,
                intensity_mm_h: intensity,
            });
            prev_min = t;
        }
        Hyetograph::new(name, blocks)
    }
}

/// A uniform block storm: `depth_mm` spread evenly over `minutes`.
pub fn uniform_block(depth_mm: f64, minutes: f64) -> Result<Hyetograph, String> {
    if !(minutes.is_finite() && minutes > 0.0) {
        return Err("storm duration must be > 0".into());
    }
    Hyetograph::new(
        &format!("{depth_mm} mm in {minutes} min"),
        vec![Block {
            duration_s: minutes * 60.0,
            intensity_mm_h: depth_mm * 60.0 / minutes,
        }],
    )
}

/// Standard storm `nr` (1–10) of Leidraad Riolering C2100, table B3.2, in
/// 5-minute blocks.
pub fn leidraad_bui(nr: u8) -> Result<Hyetograph, String> {
    let depths = LEIDRAAD_MM_PER_5MIN
        .get((nr as usize).wrapping_sub(1))
        .ok_or_else(|| format!("Leidraad Riolering has standard storms 01–10, not {nr:02}"))?;
    Hyetograph::from_depths(&format!("bui {nr:02}"), LEIDRAAD_BLOCK_S, depths)
}

/// Bui 08 of Leidraad Riolering C2100: T = 2 years, peak late, 19.8 mm in
/// 60 min, peak 39.6 mm/h.
pub fn bui08() -> Hyetograph {
    leidraad_bui(8).expect("bui 08 is in the table")
}

/// The standard events of the DPRA stress test *wateroverlast* (notitie
/// standaarden stresstest wateroverlast, Nov. 2024, table 4, climate 2050).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StresstestStorm {
    /// Local, 70 mm in 1 h, T = 100 years (58 mm in today's climate).
    Local1hT100,
    /// Local, 90 mm in 1 h, T = 250 years (75 mm today).
    Local1hT250,
    /// Local, 155 mm in 2 h, T = 1000 years (128 mm today).
    Local2hT1000,
    /// Local, 200 mm in 48 h (the supra-regional stress test conditions).
    Local48h,
    /// Regional, 130 mm in 48 h, T = 100 years (111 mm today).
    Regional48hT100,
    /// Regional, 150 mm in 48 h, T = 250 years (128 mm today).
    Regional48hT250,
    /// Regional, 185 mm in 48 h, T = 1000 years (157 mm today).
    Regional48hT1000,
}

impl StresstestStorm {
    /// All standard events, short to long.
    pub const ALL: [StresstestStorm; 7] = [
        StresstestStorm::Local1hT100,
        StresstestStorm::Local1hT250,
        StresstestStorm::Local2hT1000,
        StresstestStorm::Local48h,
        StresstestStorm::Regional48hT100,
        StresstestStorm::Regional48hT250,
        StresstestStorm::Regional48hT1000,
    ];

    /// Depth (mm) and duration (h) for climate 2050.
    pub fn depth_and_hours(self) -> (f64, f64) {
        match self {
            StresstestStorm::Local1hT100 => (70.0, 1.0),
            StresstestStorm::Local1hT250 => (90.0, 1.0),
            StresstestStorm::Local2hT1000 => (155.0, 2.0),
            StresstestStorm::Local48h => (200.0, 48.0),
            StresstestStorm::Regional48hT100 => (130.0, 48.0),
            StresstestStorm::Regional48hT250 => (150.0, 48.0),
            StresstestStorm::Regional48hT1000 => (185.0, 48.0),
        }
    }

    /// The event as a hyetograph: constant intensity, as the note prescribes.
    pub fn hyetograph(self) -> Hyetograph {
        let (mm, h) = self.depth_and_hours();
        let mut storm = uniform_block(mm, h * 60.0).expect("positive duration");
        storm.name = format!("stresstest {mm} mm / {h} h");
        storm
    }
}

/// A storm from a short spec, as the commands take it (`BUI=`):
/// `08` / `bui08` (Leidraad Riolering standard storm 1–10), `T100`, `T250`,
/// `T1000`, `48h`, `R100`, `R250`, `R1000` (the stress test events of
/// [`StresstestStorm`], local and regional), or `<mm>/<minutes>` (a
/// uniform block, e.g. `70/60`). A CSV is read by the caller with [`Hyetograph::from_csv`].
pub fn parse_storm(spec: &str) -> Result<Hyetograph, String> {
    let s = spec.trim().to_ascii_lowercase().replace(' ', "");
    let event = match s.as_str() {
        "t100" => Some(StresstestStorm::Local1hT100),
        "t250" => Some(StresstestStorm::Local1hT250),
        "t1000" => Some(StresstestStorm::Local2hT1000),
        "48h" | "t48" => Some(StresstestStorm::Local48h),
        "r100" => Some(StresstestStorm::Regional48hT100),
        "r250" => Some(StresstestStorm::Regional48hT250),
        "r1000" => Some(StresstestStorm::Regional48hT1000),
        _ => None,
    };
    if let Some(e) = event {
        return Ok(e.hyetograph());
    }
    if let Some((mm, min)) = s.split_once('/') {
        let num = |t: &str| t.trim_end_matches("mm").trim_end_matches("min").replace(',', ".").parse::<f64>().map_err(|_| format!("storm {spec}: expected <mm>/<minutes>"));
        return uniform_block(num(mm)?, num(min)?);
    }
    let nr = s.trim_start_matches("bui").trim_start_matches("leidraad");
    match nr.parse::<u8>() {
        Ok(n) => leidraad_bui(n),
        Err(_) => Err(format!("storm {spec}: use 01–10 (Leidraad), T100, T250, T1000, 48h, R100, R250, R1000 or <mm>/<minutes>")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn storm_specs() {
        assert!(close(parse_storm("08").unwrap().total_depth_mm(), bui08().total_depth_mm(), 1e-9));
        assert!(close(parse_storm("bui08").unwrap().total_depth_mm(), 19.8, 1e-6));
        assert!(close(parse_storm("T100").unwrap().total_depth_mm(), 70.0, 1e-9));
        assert!(close(parse_storm("R1000").unwrap().duration_s(), 48.0 * 3600.0, 1e-9));
        let u = parse_storm("70/60").unwrap();
        assert!(close(u.total_depth_mm(), 70.0, 1e-9) && close(u.duration_s(), 3600.0, 1e-9));
        assert!(parse_storm("regen").is_err() && parse_storm("11").is_err());
    }

    #[test]
    fn leidraad_totals_match_table() {
        let totals = [10.5, 10.5, 14.4, 14.4, 16.8, 16.8, 19.8, 19.8, 29.4, 35.7];
        for (i, total) in totals.iter().enumerate() {
            let b = leidraad_bui(i as u8 + 1).unwrap();
            assert!(
                close(b.total_depth_mm(), *total, 1e-9),
                "bui {} = {}",
                i + 1,
                b.total_depth_mm()
            );
        }
        assert!(leidraad_bui(0).is_err());
        assert!(leidraad_bui(11).is_err());
    }

    #[test]
    fn bui08_shape() {
        let b = bui08();
        assert!(close(b.total_depth_mm(), 19.8, 1e-9));
        assert!(close(b.duration_s(), 3600.0, 1e-9));
        // 3.3 mm per 5 min = 39.6 mm/h = 110 l/s/ha
        assert!(close(b.peak_intensity_mm_h(), 39.6, 1e-9));
        assert!(close(b.peak_intensity_mm_h() / 0.36, 110.0, 1e-9));
        // peak late: the peak block starts after 35 min
        assert!(close(b.intensity_at(37.5 * 60.0), 39.6, 1e-9));
        assert!(close(b.depth_between_mm(0.0, 1800.0), 6.6, 1e-9));
    }

    #[test]
    fn stresstest_depths() {
        for s in StresstestStorm::ALL {
            let (mm, h) = s.depth_and_hours();
            let hy = s.hyetograph();
            assert!(close(hy.total_depth_mm(), mm, 1e-9));
            assert!(close(hy.duration_s(), h * 3600.0, 1e-9));
        }
        assert!(close(
            StresstestStorm::Local1hT100
                .hyetograph()
                .peak_intensity_mm_h(),
            70.0,
            1e-12
        ));
    }

    #[test]
    fn uniform_and_resample_conserve_depth() {
        let u = uniform_block(70.0, 120.0).unwrap();
        assert!(close(u.total_depth_mm(), 70.0, 1e-12));
        assert!(close(u.peak_intensity_mm_h(), 35.0, 1e-12));
        // a step that does not divide the block length
        let r = bui08().resample(420.0).unwrap();
        let depth: f64 = r.iter().map(|i| i * 420.0 / 3600.0).sum();
        assert!(close(depth, 19.8, 1e-9));
        assert_eq!(r.len(), 9);
    }

    #[test]
    fn csv_intensity_and_depth() {
        let a = Hyetograph::from_csv(
            "a",
            "minuten,mm/h\n10,12\n20,36\n30,6\n",
            CsvValue::IntensityMmH,
        )
        .unwrap();
        assert!(close(a.total_depth_mm(), 9.0, 1e-12));
        let b = Hyetograph::from_csv("b", "# meting\n5;0,3\n10;0,6\n15;0,9\n", CsvValue::DepthMm)
            .unwrap();
        assert!(close(b.total_depth_mm(), 1.8, 1e-12));
        assert!(close(b.blocks[2].intensity_mm_h, 10.8, 1e-12));
        assert!(Hyetograph::from_csv("c", "10,1\n5,1\n", CsvValue::DepthMm).is_err());
        assert!(Hyetograph::from_csv("d", "10,1\nx,y\n", CsvValue::DepthMm).is_err());
    }
}
