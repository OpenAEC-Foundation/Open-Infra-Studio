//! The soil model: soil columns (CPTs, boreholes, or layers given by hand)
//! along a line, and the soil at any station between them.
//!
//! **Interpolation.** Each column is a sequence of soil units from its
//! surface level down to the model base (its last unit continues below the
//! end of the test). Between two neighbouring columns A and B:
//!
//! 1. The unit sequences are matched in order (a dynamic-programming
//!    alignment, as in sequence correlation): two units may match when they
//!    have the same main soil (gravel, sand, loam, clay, peat); a match
//!    weighs the smaller thickness, divided by 1 + the difference of their
//!    mid levels / 2 m, and counts 0.6 when the rows differ in admixture or
//!    consistency. The alignment with the largest total weight wins.
//! 2. Matched units are one layer between A and B. A unit without a match
//!    is a wedge: it has its thickness at its own column and none at the
//!    other, at the place the other column's sequence puts it.
//! 3. Every layer boundary is interpolated linearly in elevation between A
//!    and B. Boundaries never cross, and the model is exact at the columns.
//! 4. A matched pair of different rows (e.g. soft and firm clay) changes
//!    row halfway between the columns. Before the first and after the last
//!    column the nearest column holds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::soil::{self, Main, Params};
use crate::gis::crs::XY;

/// One layer of a column: top and bottom (m NAP), the soil unit code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub top: f64,
    pub bottom: f64,
    pub code: String,
    /// Mean cone resistance of a CPT layer (MPa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qc: Option<f64>,
    /// The description it came from (SBT zone or borehole soil name).
    #[serde(default)]
    pub description: String,
}

impl Layer {
    pub fn thickness(&self) -> f64 {
        self.top - self.bottom
    }
}

/// A soil column at a station of the model line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    /// BRO id, or a name.
    pub id: String,
    /// "cpt", "borehole" or "input".
    pub kind: String,
    /// Station along the model line (m from its start) and offset (m, right > 0).
    pub station: f64,
    pub offset: f64,
    /// Position in project coordinates (m).
    pub xy: XY,
    /// Surface level and the end of the investigation (m NAP).
    pub surface: f64,
    pub end: f64,
    /// From the surface down, contiguous.
    pub layers: Vec<Layer>,
}

/// A soil model along a line.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    /// The line in project coordinates (m) and its start station.
    pub line: Vec<XY>,
    pub start_station: f64,
    /// Sorted by station.
    pub columns: Vec<Column>,
    /// Model base (m NAP): the deepest end of investigation.
    pub base: f64,
    /// Parameters of every unit in the model (table values and overrides).
    pub units: BTreeMap<String, Params>,
    /// Where the columns come from, for the report.
    #[serde(default)]
    pub source: String,
}

/// A layer of the soil at one place: top, bottom (m NAP) and unit code.
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub top: f64,
    pub bottom: f64,
    pub code: String,
}

impl Band {
    pub fn thickness(&self) -> f64 {
        self.top - self.bottom
    }
}

/// The units of a column down to `base`: consecutive layers of the same
/// code joined, the last one continued to the base.
fn units(c: &Column, base: f64) -> Vec<Band> {
    let mut out: Vec<Band> = Vec::new();
    for l in &c.layers {
        match out.last_mut() {
            Some(b) if b.code == l.code => b.bottom = l.bottom,
            _ => out.push(Band { top: l.top, bottom: l.bottom, code: l.code.clone() }),
        }
    }
    if let Some(b) = out.last_mut() {
        b.bottom = b.bottom.min(base);
    }
    out
}

fn main_of(code: &str) -> Option<Main> {
    soil::row(code).map(|r| r.main)
}

/// The alignment of two unit sequences: pairs (index in a, index in b),
/// either side `None` for a wedge; in order from the top.
pub fn align(a: &[Band], b: &[Band]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (a.len(), b.len());
    let score = |i: usize, j: usize| -> Option<f64> {
        let (x, y) = (&a[i], &b[j]);
        let (mx, my) = (main_of(&x.code)?, main_of(&y.code)?);
        if mx != my {
            return None;
        }
        let t = (x.top - x.bottom).min(y.top - y.bottom).max(0.05);
        let dz = ((x.top + x.bottom) / 2.0 - (y.top + y.bottom) / 2.0).abs();
        Some(t / (1.0 + dz / 2.0) * if x.code == y.code { 1.0 } else { 0.6 })
    };
    let mut s = vec![vec![0.0f64; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            let mut best = s[i][j + 1].max(s[i + 1][j]);
            if let Some(w) = score(i, j) {
                best = best.max(s[i][j] + w);
            }
            s[i + 1][j + 1] = best;
        }
    }
    // Back from the end: matches, then the unmatched runs between them.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if let Some(w) = score(i - 1, j - 1) {
            if (s[i][j] - (s[i - 1][j - 1] + w)).abs() < 1e-12 && s[i][j] > s[i - 1][j].max(s[i][j - 1]) - 1e-12 {
                pairs.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
                continue;
            }
        }
        if s[i - 1][j] >= s[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    let mut out = Vec::new();
    let (mut ai, mut bj) = (0, 0);
    let mid = |x: &Band| (x.top + x.bottom) / 2.0;
    let flush = |ai: &mut usize, bj: &mut usize, to_a: usize, to_b: usize, out: &mut Vec<(Option<usize>, Option<usize>)>| {
        // The unmatched units of both sides, higher mid level first.
        let mut run: Vec<(f64, Option<usize>, Option<usize>)> = (*ai..to_a).map(|k| (mid(&a[k]), Some(k), None)).collect();
        run.extend((*bj..to_b).map(|k| (mid(&b[k]), None, Some(k))));
        run.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.2.is_some().cmp(&y.2.is_some())));
        // Keep each side's own order (the sort is by level, which agrees).
        out.extend(run.into_iter().map(|(_, p, q)| (p, q)));
        *ai = to_a;
        *bj = to_b;
    };
    for (pi, pj) in pairs {
        flush(&mut ai, &mut bj, pi, pj, &mut out);
        out.push((Some(pi), Some(pj)));
        ai = pi + 1;
        bj = pj + 1;
    }
    flush(&mut ai, &mut bj, n, m, &mut out);
    out
}

/// The interpolated soil between two columns at weight `w` (0 at a, 1 at b).
pub fn between(a: &[Band], b: &[Band], pairs: &[(Option<usize>, Option<usize>)], w: f64) -> Vec<Band> {
    let w = w.clamp(0.0, 1.0);
    let (mut za, mut zb) = (a.first().map(|x| x.top).unwrap_or(0.0), b.first().map(|x| x.top).unwrap_or(0.0));
    let mut out: Vec<Band> = Vec::new();
    for (p, q) in pairs {
        let ta = p.map(|i| a[i].top - a[i].bottom).unwrap_or(0.0);
        let tb = q.map(|j| b[j].top - b[j].bottom).unwrap_or(0.0);
        let top = (1.0 - w) * za + w * zb;
        za -= ta;
        zb -= tb;
        let bottom = (1.0 - w) * za + w * zb;
        let code = match (p, q) {
            (Some(i), Some(j)) => if w < 0.5 { &a[*i].code } else { &b[*j].code },
            (Some(i), None) => &a[*i].code,
            (None, Some(j)) => &b[*j].code,
            (None, None) => continue,
        };
        if top - bottom > 1e-6 {
            match out.last_mut() {
                Some(l) if l.code == *code => l.bottom = bottom,
                _ => out.push(Band { top, bottom, code: code.clone() }),
            }
        }
    }
    out
}

/// Station (m from the start, clamped to the line) and offset (m, right of
/// the line > 0) of a point on a polyline.
pub fn station_offset(line: &[XY], p: XY) -> (f64, f64) {
    let mut best = (f64::MAX, 0.0, 0.0);
    let mut run = 0.0;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        if l < 1e-12 {
            continue;
        }
        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (l * l)).clamp(0.0, 1.0);
        let q = [a[0] + dx * t, a[1] + dy * t];
        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
        if d < best.0 {
            let side = if dx * (p[1] - a[1]) - dy * (p[0] - a[0]) > 0.0 { -1.0 } else { 1.0 };
            best = (d, run + t * l, side * d);
        }
        run += l;
    }
    (best.1, best.2)
}

impl Model {
    pub fn length(&self) -> f64 {
        self.line.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
    }

    /// Station (m from the line's start, clamped) and offset (m, right > 0)
    /// of a point in project coordinates.
    pub fn station_offset(&self, p: XY) -> (f64, f64) {
        station_offset(&self.line, p)
    }

    /// The soil at a station (m from the start), from the top of the model
    /// (the interpolated surface levels of the columns) to its base.
    pub fn at_station(&self, s: f64) -> Vec<Band> {
        let cols = &self.columns;
        if cols.is_empty() {
            return Vec::new();
        }
        if s <= cols[0].station || cols.len() == 1 {
            return units(&cols[0], self.base);
        }
        if s >= cols[cols.len() - 1].station {
            return units(&cols[cols.len() - 1], self.base);
        }
        let k = cols.windows(2).position(|w| s >= w[0].station && s <= w[1].station).unwrap_or(0);
        let (a, b) = (units(&cols[k], self.base), units(&cols[k + 1], self.base));
        let span = (cols[k + 1].station - cols[k].station).max(1e-9);
        between(&a, &b, &align(&a, &b), (s - cols[k].station) / span)
    }

    /// The soil under a point (project coordinates): the model at the point's
    /// station.
    pub fn at_point(&self, p: XY) -> Vec<Band> {
        self.at_station(self.station_offset(p).0)
    }

    /// The parameters of a unit (table values when the model lacks it).
    pub fn params(&self, code: &str) -> Params {
        self.units.get(code).cloned().or_else(|| soil::row(code).map(|r| r.params())).unwrap_or_else(|| soil::row("zand_schoon_los").unwrap().params())
    }
}

/// Where the soil comes from in a calculation.
#[derive(Clone, Debug)]
pub enum Subsoil {
    /// A soil model along a line.
    Model(Model),
    /// Horizontal layers by their top level (m NAP), down to `base`.
    Layers { tops: Vec<(f64, String)>, base: f64, units: BTreeMap<String, Params> },
    /// Scattered columns: under a point the nearest one holds.
    Nearest { columns: Vec<Column>, base: f64, units: BTreeMap<String, Params> },
}

impl Subsoil {
    /// The bands under a point (project coordinates), top down.
    pub fn at(&self, p: XY) -> Vec<Band> {
        match self {
            Subsoil::Model(m) => m.at_point(p),
            Subsoil::Layers { tops, base, .. } => {
                let mut out = Vec::new();
                for (i, (top, code)) in tops.iter().enumerate() {
                    let bottom = tops.get(i + 1).map(|t| t.0).unwrap_or(*base);
                    if *top - bottom > 1e-9 {
                        out.push(Band { top: *top, bottom, code: code.clone() });
                    }
                }
                out
            }
            Subsoil::Nearest { columns, base, .. } => columns
                .iter()
                .min_by(|a, b| (a.xy[0] - p[0]).hypot(a.xy[1] - p[1]).total_cmp(&(b.xy[0] - p[0]).hypot(b.xy[1] - p[1])))
                .map(|c| units(c, *base))
                .unwrap_or_default(),
        }
    }

    pub fn base(&self) -> f64 {
        match self {
            Subsoil::Model(m) => m.base,
            Subsoil::Layers { base, .. } | Subsoil::Nearest { base, .. } => *base,
        }
    }

    pub fn units(&self) -> &BTreeMap<String, Params> {
        match self {
            Subsoil::Model(m) => &m.units,
            Subsoil::Layers { units, .. } | Subsoil::Nearest { units, .. } => units,
        }
    }

    pub fn units_mut(&mut self) -> &mut BTreeMap<String, Params> {
        match self {
            Subsoil::Model(m) => &mut m.units,
            Subsoil::Layers { units, .. } | Subsoil::Nearest { units, .. } => units,
        }
    }

    pub fn params(&self, code: &str) -> Params {
        self.units().get(code).cloned().or_else(|| soil::row(code).map(|r| r.params())).unwrap_or_else(|| soil::row("zand_schoon_los").unwrap().params())
    }

    /// Text for the report.
    pub fn describe(&self) -> String {
        match self {
            Subsoil::Model(m) => format!("{} '{}' ({} {})", crate::lang::tr("grondmodel", "soil model"), m.name, m.columns.len(), crate::lang::tr("kolommen", "columns")),
            Subsoil::Layers { tops, .. } => format!("{} ({})", crate::lang::tr("opgegeven lagen", "given layers"), tops.len()),
            Subsoil::Nearest { columns, .. } => format!("{} ({})", crate::lang::tr("dichtstbijzijnde sondering of boring", "nearest CPT or borehole"), columns.len()),
        }
    }
}

/// The bands under a point clipped to the ground surface `z_ground`: cut
/// where the ground is lower than the model top, the top unit continued up
/// where it is higher.
pub fn clip_to_surface(bands: &[Band], z_ground: f64) -> Vec<Band> {
    let mut out: Vec<Band> = bands.iter().filter(|b| b.bottom < z_ground).cloned().collect();
    if let Some(first) = out.first_mut() {
        first.top = z_ground;
    }
    out
}

/// Bands thinner than `min` (m) join their thicker neighbour: the thin
/// wedges near a pinch-out, which would only add noise to a calculation.
pub fn merge_thin(mut bands: Vec<Band>, min: f64) -> Vec<Band> {
    while bands.len() > 1 {
        let Some((i, _)) = bands.iter().enumerate().filter(|(_, b)| b.thickness() < min).min_by(|a, b| a.1.thickness().total_cmp(&b.1.thickness())) else { break };
        let into = if i == 0 {
            1
        } else if i + 1 == bands.len() || bands[i - 1].thickness() >= bands[i + 1].thickness() {
            i - 1
        } else {
            i + 1
        };
        let b = bands.remove(i);
        let j = if into > i { into - 1 } else { into };
        bands[j].top = bands[j].top.max(b.top);
        bands[j].bottom = bands[j].bottom.min(b.bottom);
        let mut k = 1;
        while k < bands.len() {
            if bands[k].code == bands[k - 1].code {
                let n = bands.remove(k);
                bands[k - 1].bottom = n.bottom;
            } else {
                k += 1;
            }
        }
    }
    bands
}

/// `LAGEN=top:code;top:code;…` (tops in m NAP, from high to low).
pub fn parse_layers(text: &str) -> Result<Vec<(f64, String)>, String> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (z, code) = part.split_once(':').ok_or_else(|| format!("LAGEN: {} '{part}'", crate::lang::tr("verwacht bovenkant:grondsoort, kreeg", "expected top:soil unit, got")))?;
        let z: f64 = z.trim().replace(',', ".").parse().map_err(|_| format!("LAGEN: '{z}'"))?;
        let r = soil::row(code.trim()).ok_or_else(|| soil::unknown_code(code))?;
        if out.last().is_some_and(|l| l.0 <= z) {
            return Err(crate::lang::tr("LAGEN: de bovenkanten moeten van hoog naar laag lopen", "LAGEN: the tops must run from high to low").into());
        }
        out.push((z, r.code.to_string()));
    }
    if out.is_empty() {
        return Err("LAGEN: top:code;top:code".into());
    }
    Ok(out)
}

/// A layer as read from a test: top, bottom (m NAP), unit code (if any),
/// mean qc, description.
pub type RawLayer = (f64, f64, Option<&'static str>, Option<f64>, String);

/// Join consecutive layers of the same unit and absorb layers thinner than
/// `min` (m) into the thicker neighbour; layers without a unit (`None`)
/// take the unit below them (or above, at the bottom).
pub fn simplify(layers: Vec<RawLayer>, min: f64) -> Vec<Layer> {
    // Unknown units take a neighbour's.
    let n = layers.len();
    let codes: Vec<Option<&'static str>> = (0..n).map(|i| layers[i].2.or_else(|| layers[i + 1..].iter().find_map(|l| l.2)).or_else(|| layers[..i].iter().rev().find_map(|l| l.2))).collect();
    let mut out: Vec<Layer> = Vec::new();
    for (l, code) in layers.into_iter().zip(codes) {
        let Some(code) = code else { continue };
        match out.last_mut() {
            Some(prev) if prev.code == code => {
                let (t0, t1) = (prev.thickness(), l.0 - l.1);
                prev.qc = match (prev.qc, l.3) {
                    (Some(a), Some(b)) => Some((a * t0 + b * t1) / (t0 + t1).max(1e-9)),
                    (a, b) => a.or(b),
                };
                prev.bottom = l.1;
            }
            _ => out.push(Layer { top: l.0, bottom: l.1, code: code.to_string(), qc: l.3, description: l.4 }),
        }
    }
    // Thin layers into the thicker neighbour, thinnest first.
    while out.len() > 1 {
        let Some((i, _)) = out.iter().enumerate().filter(|(_, l)| l.thickness() < min).min_by(|a, b| a.1.thickness().total_cmp(&b.1.thickness())) else { break };
        let into = if i == 0 {
            1
        } else if i + 1 == out.len() || out[i - 1].thickness() >= out[i + 1].thickness() {
            i - 1
        } else {
            i + 1
        };
        let l = out.remove(i);
        let j = if into > i { into - 1 } else { into };
        let t = &mut out[j];
        t.top = t.top.max(l.top);
        t.bottom = t.bottom.min(l.bottom);
        let mut k = 1;
        while k < out.len() {
            if out[k].code == out[k - 1].code {
                let b = out.remove(k);
                out[k - 1].bottom = b.bottom;
            } else {
                k += 1;
            }
        }
    }
    out
}

/// The stored soil models of the drawing (XRecord `OPENINFRA/GRONDMODEL`).
pub const RECORD: &str = "GRONDMODEL";

pub fn load_all() -> BTreeMap<String, Model> {
    crate::bridge::json_record_read(RECORD).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(model: &Model) -> Result<(), String> {
    let mut all = load_all();
    all.insert(model.name.clone(), model.clone());
    crate::bridge::json_record_write(RECORD, &serde_json::to_string(&all).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(station: f64, layers: &[(f64, f64, &str)]) -> Column {
        Column {
            id: format!("C{station}"),
            kind: "input".into(),
            station,
            offset: 0.0,
            xy: [station, 0.0],
            surface: layers[0].0,
            end: layers[layers.len() - 1].1,
            layers: layers.iter().map(|(t, b, c)| Layer { top: *t, bottom: *b, code: c.to_string(), qc: None, description: String::new() }).collect(),
        }
    }

    fn model(cols: Vec<Column>) -> Model {
        let base = cols.iter().map(|c| c.end).fold(f64::MAX, f64::min);
        Model { name: "t".into(), line: vec![[0.0, 0.0], [100.0, 0.0]], start_station: 0.0, columns: cols, base, units: BTreeMap::new(), source: String::new() }
    }

    #[test]
    fn matched_layers_interpolate_linearly() {
        let m = model(vec![
            col(0.0, &[(0.0, -2.0, "klei_schoon_slap"), (-2.0, -6.0, "veen_slap"), (-6.0, -12.0, "zand_schoon_vast")]),
            col(40.0, &[(-1.0, -4.0, "klei_schoon_slap"), (-4.0, -6.0, "veen_slap"), (-6.0, -14.0, "zand_schoon_vast")]),
        ]);
        let mid = m.at_station(20.0);
        let codes: Vec<&str> = mid.iter().map(|b| b.code.as_str()).collect();
        assert_eq!(codes, ["klei_schoon_slap", "veen_slap", "zand_schoon_vast"]);
        assert!((mid[0].top + 0.5).abs() < 1e-9 && (mid[0].bottom + 3.0).abs() < 1e-9 && (mid[1].bottom + 6.0).abs() < 1e-9);
        // Exact at the columns, the first column's sand continued to the base.
        let at_a = m.at_station(0.0);
        assert_eq!(at_a.last().unwrap().bottom, -14.0);
        assert_eq!(m.at_station(-5.0), at_a);
    }

    #[test]
    fn an_unmatched_layer_wedges_out() {
        let a = col(0.0, &[(0.0, -1.0, "klei_schoon_slap"), (-1.0, -3.0, "veen_slap"), (-3.0, -10.0, "zand_schoon_vast")]);
        let b = col(10.0, &[(0.0, -1.0, "klei_schoon_slap"), (-1.0, -10.0, "zand_schoon_vast")]);
        let m = model(vec![a, b]);
        let quarter = m.at_station(2.5);
        let peat = quarter.iter().find(|b| b.code == "veen_slap").unwrap();
        assert!(((peat.top - peat.bottom) - 1.5).abs() < 1e-9, "{quarter:?}");
        assert!(m.at_station(10.0).iter().all(|b| b.code != "veen_slap"));
        // Totals stay from the top to the base.
        for s in [0.0, 3.0, 7.0, 10.0] {
            let bands = m.at_station(s);
            assert!((bands[0].top - 0.0).abs() < 1e-9 && (bands.last().unwrap().bottom + 10.0).abs() < 1e-9);
            assert!(bands.windows(2).all(|w| (w[0].bottom - w[1].top).abs() < 1e-9));
        }
    }

    #[test]
    fn layers_join_and_thin_ones_merge() {
        let l = simplify(
            vec![
                (0.0, -0.5, None, None, "geen meting".into()),
                (-0.5, -2.0, Some("klei_schoon_slap"), Some(0.4), String::new()),
                (-2.0, -2.1, Some("zand_schoon_los"), Some(4.0), String::new()),
                (-2.1, -3.0, Some("klei_schoon_slap"), Some(0.6), String::new()),
                (-3.0, -8.0, Some("veen_slap"), Some(0.1), String::new()),
            ],
            0.3,
        );
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].top, l[0].bottom, l[0].code.as_str()), (0.0, -3.0, "klei_schoon_slap"));
        let lt = parse_layers("0:klei_schoon_slap;-2.5:veen_slap;-6:zand_schoon_vast").unwrap();
        assert_eq!(lt[1], (-2.5, "veen_slap".to_string()));
        assert!(parse_layers("0:klei_schoon_slap;1:veen_slap").is_err());
        let s = Subsoil::Layers { tops: lt, base: -20.0, units: BTreeMap::new() };
        let b = clip_to_surface(&s.at([0.0, 0.0]), 1.2);
        assert_eq!((b[0].top, b.len(), b[2].bottom), (1.2, 3, -20.0));
        assert_eq!(clip_to_surface(&s.at([0.0, 0.0]), -3.0)[0].code, "veen_slap");
        let b = |t: f64, m: f64, c: &str| Band { top: t, bottom: m, code: c.into() };
        let m = merge_thin(vec![b(6.0, 5.94, "veen_slap"), b(5.94, -3.0, "zand_schoon_matig"), b(-3.0, -3.1, "klei_schoon_slap"), b(-3.1, -9.0, "zand_schoon_matig")], 0.2);
        assert_eq!(m, vec![b(6.0, -9.0, "zand_schoon_matig")]);
    }
}
