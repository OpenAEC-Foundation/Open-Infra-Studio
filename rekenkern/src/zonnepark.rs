//! Solar park layout and a rough yield estimate, issue #73.
//!
//! Layout. Rectangular tables are placed in rows perpendicular to the panel
//! azimuth inside a site polygon, kept at least `setback` from its boundary
//! and `exclusion_buffer` from exclusion polygons (ditches, cables, trees),
//! optionally skipping positions where the ground slope exceeds a limit.
//! Rows start at the site's extent plus the setback and step by the pitch;
//! every `path_every_rows` rows a maintenance path is inserted. Within a row
//! tables are laid greedily from one end: when a position does not fit the
//! row is scanned forward and the earliest fitting position found by
//! bisection, so irregular sites are filled reasonably but not optimally.
//!
//! Angles in **degrees**; azimuth clockwise from north (180 = south).
//! Lengths in metres. The site is a simple polygon without holes (use
//! exclusions for holes).
//!
//! Row spacing without shading at solar noon on the winter solstice: the sun
//! altitude is `α = 90° − |φ| − 23.44°`; a table of slant depth `d` and tilt
//! `β` casts a shadow `d·sinβ / tanα` behind its footprint `d·cosβ`, so
//! `pitch = d·cosβ + d·sinβ·|cos(Δaz)| / tanα`, where `Δaz` is the angle
//! between the panel azimuth and the noon sun (south in the northern
//! hemisphere). Ground coverage ratio `GCR = d / pitch`.
//!
//! Yield. Monthly global horizontal irradiation defaults to the KNMI De Bilt
//! normals 1991–2020 (klimtab_9120_260, "Globale straling som in kJ/cm²":
//! 7.3 12.9 26.7 42.4 54.2 55.1 55.3 46.9 31.4 18.8 8.4 5.4, year 364.8
//! kJ/cm² ≈ 1013 kWh/m²). Its monthly clearness index is applied at the
//! site's latitude, split into beam and diffuse with the Erbs et al. monthly
//! correlation, distributed over the mean day with Collares-Pereira & Rabl
//! (total) and Liu & Jordan (diffuse) and transposed with the isotropic sky
//! model (all in Duffie & Beckman, *Solar Engineering of Thermal Processes*,
//! ch. 1–2). Specific yield = plane-of-array irradiation × performance ratio
//! (kWh/kWp, since STC is 1 kW/m²). Default PR 0.80 is an adjustable
//! assumption. Expect ±10 % at best: no row shading, soiling, snow or
//! temperature model beyond the PR. Sanity check: the Dutch national
//! benchmark is 875 kWh/kWp (Utrecht University / CBS, as reported by Solar
//! Magazine, "Nieuw kengetal vastgesteld voor opbrengst zonnepanelen in
//! Nederland").

use crate::P2;
use serde::{Deserialize, Serialize};

/// KNMI De Bilt monthly global radiation 1991–2020, kWh/m² (kJ/cm² ÷ 0.36).
pub const DE_BILT_GHI_KWH_M2: [f64; 12] = [
    7.3 / 0.36,
    12.9 / 0.36,
    26.7 / 0.36,
    42.4 / 0.36,
    54.2 / 0.36,
    55.1 / 0.36,
    55.3 / 0.36,
    46.9 / 0.36,
    31.4 / 0.36,
    18.8 / 0.36,
    8.4 / 0.36,
    5.4 / 0.36,
];
/// Latitude of De Bilt (KNMI station 260), degrees.
pub const DE_BILT_LATITUDE: f64 = 52.10;
/// Earth's axial tilt used for the solstice, degrees.
pub const OBLIQUITY_DEG: f64 = 23.44;

/// A table (rack) of panels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableSpec {
    /// Length along the row, m.
    pub length: f64,
    /// Slant depth of the panel plane, m.
    pub depth: f64,
    pub tilt_deg: f64,
    /// Direction the panels face, degrees clockwise from north.
    pub azimuth_deg: f64,
    /// Gap between tables in a row, m.
    pub gap: f64,
}

/// Panel module size on the table and its rating.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSpec {
    /// Module dimension along the row, m.
    pub along: f64,
    /// Module dimension up the slope, m.
    pub up: f64,
    /// Peak power, Wp.
    pub wp: f64,
}

impl PanelSpec {
    pub fn per_table(&self, t: &TableSpec) -> usize {
        if self.along <= 0.0 || self.up <= 0.0 {
            return 0;
        }
        ((t.length / self.along + 1e-9).floor() * (t.depth / self.up + 1e-9).floor()) as usize
    }
}

/// How the row pitch (front edge to front edge, horizontal) is chosen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RowSpacing {
    Pitch(f64),
    /// Ground coverage ratio depth / pitch.
    GroundCoverRatio(f64),
    /// No shading at solar noon on the winter solstice.
    WinterNoon {
        latitude_deg: f64,
    },
}

/// Layout input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SiteInput {
    pub site: Vec<P2>,
    pub exclusions: Vec<Vec<P2>>,
    pub setback: f64,
    pub exclusion_buffer: f64,
    pub table: TableSpec,
    pub spacing: RowSpacing,
    /// Insert a maintenance path after every N rows (0 = none).
    pub path_every_rows: usize,
    pub path_width: f64,
}

/// Layout result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolarLayout {
    /// Table footprints as closed rings (4 corners).
    pub tables: Vec<Vec<P2>>,
    /// Rows holding at least one table.
    pub rows: usize,
    pub pitch: f64,
    pub gcr: f64,
    pub panel_count: usize,
    pub kwp: f64,
}

/// Solar altitude at noon on the winter solstice, degrees.
pub fn winter_noon_altitude(latitude_deg: f64) -> f64 {
    90.0 - latitude_deg.abs() - OBLIQUITY_DEG
}

/// Pitch for no shading at winter-solstice noon (see module docs).
pub fn winter_noon_pitch(
    depth: f64,
    tilt_deg: f64,
    latitude_deg: f64,
    azimuth_deg: f64,
) -> Result<f64, String> {
    let alt = winter_noon_altitude(latitude_deg);
    if alt <= 0.5 {
        return Err(format!(
            "sun below {alt:.1}° at winter noon: no finite pitch"
        ));
    }
    let b = tilt_deg.to_radians();
    let sun_az = if latitude_deg >= 0.0 { 180.0 } else { 0.0 };
    let daz = (azimuth_deg - sun_az).to_radians().cos().abs();
    Ok(depth * b.cos() + depth * b.sin() * daz / alt.to_radians().tan())
}

// ------------------------------------------------------------- geometry

fn cross(o: P2, a: P2, b: P2) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn point_in_polygon(p: P2, poly: &[P2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn point_seg_dist(p: P2, a: P2, b: P2) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let l2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * ab[0]).hypot(p[1] - a[1] - t * ab[1])
}

/// Proper crossing (touching does not count), with tolerance `eps`.
fn segs_cross(a: P2, b: P2, c: P2, d: P2, eps: f64) -> bool {
    let d1 = cross(a, b, c);
    let d2 = cross(a, b, d);
    let d3 = cross(c, d, a);
    let d4 = cross(c, d, b);
    let s = |x: f64, l: f64| {
        if x > eps * l {
            1
        } else if x < -eps * l {
            -1
        } else {
            0
        }
    };
    let lab = (b[0] - a[0]).hypot(b[1] - a[1]);
    let lcd = (d[0] - c[0]).hypot(d[1] - c[1]);
    s(d1, lab) * s(d2, lab) < 0 && s(d3, lcd) * s(d4, lcd) < 0
}

fn seg_dist(a: P2, b: P2, c: P2, d: P2) -> f64 {
    if segs_cross(a, b, c, d, 0.0) {
        return 0.0;
    }
    point_seg_dist(a, c, d)
        .min(point_seg_dist(b, c, d))
        .min(point_seg_dist(c, a, b))
        .min(point_seg_dist(d, a, b))
}

fn edges(poly: &[P2]) -> impl Iterator<Item = (P2, P2)> + '_ {
    (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))
}

/// Minimum distance between two rings' boundaries (bbox-pruned).
fn ring_dist(r: &[P2], poly: &[P2], limit: f64) -> f64 {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in r {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mut best = f64::INFINITY;
    for (c, d) in edges(poly) {
        if c[0].max(d[0]) < lo[0] - limit
            || c[0].min(d[0]) > hi[0] + limit
            || c[1].max(d[1]) < lo[1] - limit
            || c[1].min(d[1]) > hi[1] + limit
        {
            continue;
        }
        for (a, b) in edges(r) {
            best = best.min(seg_dist(a, b, c, d));
        }
    }
    best
}

const TOL: f64 = 1e-6;

fn fits(rect: &[P2], inp: &SiteInput) -> bool {
    // Inside the site, clear of its boundary by the setback.
    let near = ring_dist(rect, &inp.site, inp.setback.max(TOL));
    if near < inp.setback - TOL {
        return false;
    }
    if near > TOL && !rect.iter().all(|p| point_in_polygon(*p, &inp.site)) {
        return false;
    }
    if near <= TOL {
        // Touching the boundary: require the centre inside and no crossing.
        let c = centre(rect);
        if !point_in_polygon(c, &inp.site) {
            return false;
        }
        for (a, b) in edges(rect) {
            for (p, q) in edges(&inp.site) {
                if segs_cross(a, b, p, q, 1e-9) {
                    return false;
                }
            }
        }
    }
    // Clear of exclusions.
    for ex in &inp.exclusions {
        if ex.len() < 3 {
            continue;
        }
        let d = ring_dist(rect, ex, inp.exclusion_buffer.max(TOL));
        if d < inp.exclusion_buffer.max(0.0)
            + if inp.exclusion_buffer > 0.0 {
                -TOL
            } else {
                TOL
            }
        {
            return false;
        }
        if point_in_polygon(rect[0], ex) || ex.iter().any(|p| point_in_polygon(*p, rect)) {
            return false;
        }
    }
    true
}

fn centre(r: &[P2]) -> P2 {
    let n = r.len() as f64;
    [
        r.iter().map(|p| p[0]).sum::<f64>() / n,
        r.iter().map(|p| p[1]).sum::<f64>() / n,
    ]
}

/// Lay out tables. `slope` optionally gives the ground slope in degrees at a
/// point together with the maximum allowed; it is checked at the corners and
/// centre of each table.
pub fn layout(
    inp: &SiteInput,
    panel: &PanelSpec,
    slope: Option<(&dyn Fn(P2) -> f64, f64)>,
) -> Result<SolarLayout, String> {
    let t = &inp.table;
    if inp.site.len() < 3 {
        return Err("site polygon needs three points".into());
    }
    if !(t.length > 0.0 && t.depth > 0.0 && (0.0..90.0).contains(&t.tilt_deg)) {
        return Err("table length, depth > 0 and tilt in [0, 90) required".into());
    }
    let pitch = match inp.spacing {
        RowSpacing::Pitch(p) => p,
        RowSpacing::GroundCoverRatio(g) if g > 0.0 => t.depth / g,
        RowSpacing::GroundCoverRatio(_) => return Err("GCR must be positive".into()),
        RowSpacing::WinterNoon { latitude_deg } => {
            winter_noon_pitch(t.depth, t.tilt_deg, latitude_deg, t.azimuth_deg)?
        }
    };
    let foot = t.depth * t.tilt_deg.to_radians().cos();
    if pitch < foot - TOL {
        return Err(format!(
            "pitch {pitch:.3} m is less than the table footprint {foot:.3} m"
        ));
    }
    let az = t.azimuth_deg.to_radians();
    let r = [az.cos(), -az.sin()]; // along the row
    let b = [-az.sin(), -az.cos()]; // away from the facing direction
    let world = |u: f64, v: f64| [u * r[0] + v * b[0], u * r[1] + v * b[1]];
    let (mut umin, mut umax, mut vmin, mut vmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for p in &inp.site {
        let (u, v) = (p[0] * r[0] + p[1] * r[1], p[0] * b[0] + p[1] * b[1]);
        umin = umin.min(u);
        umax = umax.max(u);
        vmin = vmin.min(v);
        vmax = vmax.max(v);
    }
    let rect = |u: f64, v: f64| {
        vec![
            world(u, v),
            world(u + t.length, v),
            world(u + t.length, v + foot),
            world(u, v + foot),
        ]
    };
    let ok = |u: f64, v: f64| {
        let rc = rect(u, v);
        if !fits(&rc, inp) {
            return false;
        }
        match &slope {
            Some((f, max)) => rc
                .iter()
                .chain(std::iter::once(&centre(&rc)))
                .all(|p| f(*p) <= *max),
            None => true,
        }
    };
    let step = (t.length / 20.0).clamp(0.05, 0.5);
    let mut tables = Vec::new();
    let mut rows = 0;
    let mut v = vmin + inp.setback;
    let mut row_index = 0;
    while v + foot <= vmax + TOL {
        let mut placed = false;
        let mut u = umin + inp.setback;
        let mut prev_fail: Option<f64> = None;
        while u + t.length <= umax + TOL {
            if ok(u, v) {
                let mut u_place = u;
                if let Some(mut lo) = prev_fail {
                    let mut hi = u;
                    for _ in 0..60 {
                        let mid = 0.5 * (lo + hi);
                        if ok(mid, v) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                        if hi - lo < 1e-7 {
                            break;
                        }
                    }
                    u_place = hi;
                }
                tables.push(rect(u_place, v));
                placed = true;
                u = u_place + t.length + t.gap;
                prev_fail = None;
            } else {
                prev_fail = Some(u);
                u += step;
            }
        }
        if placed {
            rows += 1;
        }
        row_index += 1;
        v += pitch;
        if inp.path_every_rows > 0 && row_index % inp.path_every_rows == 0 {
            v += inp.path_width;
        }
    }
    let panel_count = tables.len() * panel.per_table(t);
    Ok(SolarLayout {
        rows,
        pitch,
        gcr: t.depth / pitch,
        panel_count,
        kwp: panel_count as f64 * panel.wp / 1000.0,
        tables,
    })
}

// ---------------------------------------------------------------- yield

/// Inputs of the monthly-mean-day yield model (see module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct YieldModel {
    pub latitude_deg: f64,
    pub tilt_deg: f64,
    /// Panel azimuth, degrees clockwise from north.
    pub azimuth_deg: f64,
    /// Ground reflectance (0.2 is the usual grass default).
    pub albedo: f64,
    /// Performance ratio (adjustable assumption, default 0.80).
    pub performance_ratio: f64,
    /// Monthly global horizontal irradiation at `reference_latitude_deg`, kWh/m².
    pub monthly_ghi: [f64; 12],
    pub reference_latitude_deg: f64,
}

impl YieldModel {
    /// De Bilt climate, albedo 0.2, PR 0.80.
    pub fn netherlands(latitude_deg: f64, tilt_deg: f64, azimuth_deg: f64) -> Self {
        Self {
            latitude_deg,
            tilt_deg,
            azimuth_deg,
            albedo: 0.2,
            performance_ratio: 0.80,
            monthly_ghi: DE_BILT_GHI_KWH_M2,
            reference_latitude_deg: DE_BILT_LATITUDE,
        }
    }
}

const MEAN_DAY: [f64; 12] = [
    17., 47., 75., 105., 135., 162., 198., 228., 258., 288., 318., 344.,
];
const DAYS: [f64; 12] = [31., 28., 31., 30., 31., 30., 31., 31., 30., 31., 30., 31.];
const GSC: f64 = 1367.0;

fn declination(n: f64) -> f64 {
    (23.45 * ((284.0 + n) * 360.0 / 365.0).to_radians().sin()).to_radians()
}

fn sunset_hour_angle(phi: f64, dec: f64) -> f64 {
    (-phi.tan() * dec.tan()).clamp(-1.0, 1.0).acos()
}

/// Daily extraterrestrial irradiation on a horizontal plane, kWh/m².
fn h0(phi: f64, n: f64) -> f64 {
    let dec = declination(n);
    let ws = sunset_hour_angle(phi, dec);
    let e = 1.0 + 0.033 * (360.0 * n / 365.0).to_radians().cos();
    24.0 / std::f64::consts::PI
        * GSC
        * e
        * (phi.cos() * dec.cos() * ws.sin() + ws * phi.sin() * dec.sin())
        / 1000.0
}

/// Monthly plane-of-array irradiation, kWh/m².
pub fn plane_of_array(m: &YieldModel) -> [f64; 12] {
    let phi = m.latitude_deg.to_radians();
    let phi_ref = m.reference_latitude_deg.to_radians();
    let beta = m.tilt_deg.to_radians();
    let gam = m.azimuth_deg.to_radians();
    let nrm = [beta.sin() * gam.sin(), beta.sin() * gam.cos(), beta.cos()];
    let mut out = [0.0; 12];
    for (k, o) in out.iter_mut().enumerate() {
        let n = MEAN_DAY[k];
        let h0_ref = h0(phi_ref, n);
        let kt = if h0_ref > 0.0 {
            m.monthly_ghi[k] / DAYS[k] / h0_ref
        } else {
            0.0
        };
        let h0_here = h0(phi, n);
        let h = kt * h0_here; // daily mean, kWh/m²
        if h <= 0.0 {
            continue;
        }
        let dec = declination(n);
        let ws = sunset_hour_angle(phi, dec);
        let ktc = kt.clamp(0.3, 0.8);
        let fd = if ws.to_degrees() <= 81.4 {
            1.391 - 3.560 * ktc + 4.189 * ktc.powi(2) - 2.137 * ktc.powi(3)
        } else {
            1.311 - 3.022 * ktc + 3.427 * ktc.powi(2) - 1.821 * ktc.powi(3)
        };
        let hd = (fd * h).min(h);
        let a = 0.409 + 0.5016 * (ws - 60f64.to_radians()).sin();
        let b = 0.6609 - 0.4767 * (ws - 60f64.to_radians()).sin();
        let denom = ws.sin() - ws * ws.cos();
        let steps = 480;
        let dw = 2.0 * ws / steps as f64;
        let (mut st, mut sd) = (0.0, 0.0);
        let mut samples = Vec::with_capacity(steps);
        for i in 0..steps {
            let w = -ws + (i as f64 + 0.5) * dw;
            let base = (w.cos() - ws.cos()).max(0.0) / denom;
            let rt = (a + b * w.cos()) * base;
            let rd = base;
            st += rt;
            sd += rd;
            samples.push((w, rt, rd));
        }
        let mut day = 0.0;
        for (w, rt, rd) in samples {
            let i = h * rt / st;
            let id = hd * rd / sd;
            let ib = (i - id).max(0.0);
            let sun = [
                -dec.cos() * w.sin(),
                dec.sin() * phi.cos() - dec.cos() * phi.sin() * w.cos(),
                dec.sin() * phi.sin() + dec.cos() * phi.cos() * w.cos(),
            ];
            let cz = sun[2];
            let ct = (nrm[0] * sun[0] + nrm[1] * sun[1] + nrm[2] * sun[2]).max(0.0);
            let rb = if cz > 1e-3 { (ct / cz).min(20.0) } else { 0.0 };
            day +=
                ib * rb + id * (1.0 + beta.cos()) / 2.0 + i * m.albedo * (1.0 - beta.cos()) / 2.0;
        }
        *o = day * DAYS[k];
    }
    out
}

/// Specific yield in kWh/kWp per year.
pub fn specific_yield(m: &YieldModel) -> f64 {
    plane_of_array(m).iter().sum::<f64>() * m.performance_ratio
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(setback: f64) -> SiteInput {
        SiteInput {
            site: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]],
            exclusions: vec![],
            setback,
            exclusion_buffer: 0.0,
            table: TableSpec {
                length: 10.0,
                depth: 4.0,
                tilt_deg: 20.0,
                azimuth_deg: 180.0,
                gap: 0.0,
            },
            spacing: RowSpacing::Pitch(8.0),
            path_every_rows: 0,
            path_width: 0.0,
        }
    }

    fn panel() -> PanelSpec {
        PanelSpec {
            along: 1.0,
            up: 2.0,
            wp: 400.0,
        }
    }

    #[test]
    fn rectangle_holds_expected_tables() {
        // Rows at 0, 8, …, 40 (40 + 3.76 ≤ 50): 6 rows × 10 tables.
        let l = layout(&input(0.0), &panel(), None).unwrap();
        assert_eq!(l.tables.len(), 60);
        assert_eq!(l.rows, 6);
        assert_eq!(l.panel_count, 60 * 20);
        assert!((l.kwp - 480.0).abs() < 1e-9);
    }

    #[test]
    fn setback_reduces_count() {
        // 90 m × 40 m usable: 9 per row, rows at 5 … 37: 5 rows.
        let l = layout(&input(5.0), &panel(), None).unwrap();
        assert_eq!(l.tables.len(), 45);
        for t in &l.tables {
            for p in t {
                assert!(p[0] >= 5.0 - 1e-6 && p[0] <= 95.0 + 1e-6);
                assert!(p[1] >= 5.0 - 1e-6 && p[1] <= 45.0 + 1e-6);
            }
        }
    }

    #[test]
    fn exclusion_slope_and_paths() {
        let mut inp = input(0.0);
        // A pond covering x 0–25 in the first row band.
        inp.exclusions
            .push(vec![[1.0, 1.0], [24.0, 1.0], [24.0, 3.0], [1.0, 3.0]]);
        let l = layout(&inp, &panel(), None).unwrap();
        assert_eq!(l.tables.len(), 57);
        // Steep eastern half (x > 50) excluded by slope.
        let steep = |p: P2| if p[0] > 50.0 + 1e-9 { 12.0 } else { 2.0 };
        let l = layout(&input(0.0), &panel(), Some((&steep, 10.0))).unwrap();
        assert_eq!(l.tables.len(), 30);
        // A 4 m path after every 2 rows: rows at 0, 8, 20, 28, 40 → 5 rows.
        let mut inp = input(0.0);
        inp.path_every_rows = 2;
        inp.path_width = 4.0;
        let l = layout(&inp, &panel(), None).unwrap();
        assert_eq!(l.rows, 5);
    }

    #[test]
    fn irregular_site_tables_stay_inside() {
        let mut inp = input(2.0);
        inp.site = vec![[0.0, 0.0], [120.0, 10.0], [90.0, 70.0], [10.0, 50.0]];
        inp.table.azimuth_deg = 165.0;
        let l = layout(&inp, &panel(), None).unwrap();
        assert!(!l.tables.is_empty());
        for t in &l.tables {
            assert!(ring_dist(t, &inp.site, 2.0) >= 2.0 - 1e-5);
            assert!(t.iter().all(|p| point_in_polygon(*p, &inp.site)));
        }
    }

    #[test]
    fn winter_noon_pitch_by_hand() {
        // φ 52°, β 20°, d 4 m: α = 14.56°,
        // pitch = 4·cos20 + 4·sin20 / tan14.56 = 3.7588 + 1.3681 / 0.25973 = 9.026.
        let p = winter_noon_pitch(4.0, 20.0, 52.0, 180.0).unwrap();
        assert!((p - 9.026).abs() < 2e-3, "{p}");
        assert!((winter_noon_altitude(52.0) - 14.56).abs() < 1e-9);
        assert!(winter_noon_pitch(4.0, 20.0, 70.0, 180.0).is_err());
    }

    #[test]
    fn yield_model_sanity() {
        let flat = YieldModel::netherlands(DE_BILT_LATITUDE, 0.0, 180.0);
        let poa: f64 = plane_of_array(&flat).iter().sum();
        let ghi: f64 = DE_BILT_GHI_KWH_M2.iter().sum();
        assert!((poa - ghi).abs() / ghi < 0.01, "{poa} vs {ghi}");
        let tilted = YieldModel::netherlands(DE_BILT_LATITUDE, 35.0, 180.0);
        let gain = plane_of_array(&tilted).iter().sum::<f64>() / ghi;
        assert!((1.05..1.25).contains(&gain), "{gain}");
        let y = specific_yield(&tilted);
        assert!((800.0..1050.0).contains(&y), "{y}");
        let north = YieldModel::netherlands(DE_BILT_LATITUDE, 35.0, 0.0);
        assert!(specific_yield(&north) < specific_yield(&flat));
    }
}
