//! Mass haul (grondbalans): the cut and fill per section of a corridor,
//! accumulated along its axis into a mass haul diagram. Cut that can be
//! reused counts with a volume factor; the rest goes off site. The diagram
//! shows the balance points (where cut and fill along the way balance), the
//! haul (volume × distance, the area between the mass line and the balance
//! line), and, with a free haul distance, the volume moved further than that
//! and its overhaul. Drawn as a profile-style graph; the numbers also come
//! back as JSON (`ID=`) and as a CSV beside the drawing.
//!
//! The section volumes come from the list the corridor writes beside the
//! drawing (`<drawing>_corridor_<axis>.csv`), or are measured again from the
//! design surface (N) against the existing ground along the axis.

use std::path::{Path, PathBuf};

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{EntityType, Handle};

use super::as_profiel::{self, Alignment};
use super::grading::{fmt_n, tf, whole};
use super::terrein::Tin;
use super::{api, nice_step, record, station, Ctx, Maaiveld, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::{Element, LayerSpec};

/// The volumes of one interval: up to `s` (m from the start of the axis),
/// cut and fill (m³) since the previous station.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub s: f64,
    pub cut: f64,
    pub fill: f64,
}

/// A stretch between two balance points (or the start or end).
#[derive(Clone, Debug, PartialEq)]
pub struct Loop {
    pub from: f64,
    pub to: f64,
    /// Material moves forward (> 0, cut before fill) or back (< 0).
    pub sign: f64,
    /// The largest |mass| in it: the volume moved within the loop.
    pub volume: f64,
    /// The free haul chord: level and its two ends, when the loop is wider.
    pub chord: Option<(f64, f64, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Haul {
    pub s: Vec<f64>,
    /// Mass ordinate: reused cut × factor − fill, summed from the start.
    pub mass: Vec<f64>,
    /// Per interval: cut reused (× factor) and cut that goes off site (in situ).
    pub reused: Vec<f64>,
    pub waste: Vec<f64>,
    pub balance_points: Vec<f64>,
    pub loops: Vec<Loop>,
    /// Volume moved along the axis (m³) and the haul (m³·m).
    pub moved: f64,
    pub haul: f64,
    /// Within the free haul distance, and beyond it with its overhaul (m³·m:
    /// volume × the distance beyond the free haul distance).
    pub free_volume: f64,
    pub over_volume: f64,
    pub overhaul: f64,
    /// Left over at the end (to take away) or short (to bring in), m³.
    pub surplus: f64,
    pub shortage: f64,
    pub cut: f64,
    pub fill: f64,
}

/// Where the piecewise linear mass line is at or beyond `m` (≥ m when
/// `above`): intervals (from, to, reaches the end).
fn intervals(s: &[f64], mass: &[f64], m: f64, above: bool) -> Vec<(f64, f64, bool)> {
    let inside = |v: f64| if above { v >= m } else { v <= m };
    let mut out = Vec::new();
    let mut start: Option<f64> = if inside(mass[0]) { Some(s[0]) } else { None };
    for i in 0..s.len() - 1 {
        let (a, b) = (mass[i], mass[i + 1]);
        let (ia, ib) = (inside(a), inside(b));
        if ia != ib {
            let t = if (b - a).abs() > 0.0 { (m - a) / (b - a) } else { 0.0 };
            let x = s[i] + (s[i + 1] - s[i]) * t.clamp(0.0, 1.0);
            if ib {
                start = Some(x);
            } else if let Some(a0) = start.take() {
                out.push((a0, x, false));
            }
        }
    }
    if let Some(a0) = start {
        out.push((a0, s[s.len() - 1], true));
    }
    out
}

/// The mass haul of these rows: cut × `reuse` (share 0–1) × `factor` goes
/// into fill; `free` is the free haul distance (m, 0 = none).
pub fn mass_haul(rows: &[Row], factor: f64, reuse: f64, free: f64) -> Haul {
    let mut h = Haul::default();
    if rows.len() < 2 {
        return h;
    }
    let mut m = 0.0;
    for r in rows {
        let used = r.cut * reuse * factor;
        m += used - r.fill;
        h.s.push(r.s);
        h.mass.push(m);
        h.reused.push(used);
        h.waste.push(r.cut * (1.0 - reuse));
        h.cut += r.cut;
        h.fill += r.fill;
    }
    // The ordinate starts at zero at the first station.
    let m0 = h.mass[0];
    for v in h.mass.iter_mut() {
        *v -= m0;
    }
    let end = h.mass[h.mass.len() - 1];
    h.surplus = end.max(0.0) + h.waste.iter().sum::<f64>();
    h.shortage = (-end).max(0.0);
    // Balance points: where the mass line crosses zero.
    for i in 1..h.s.len() {
        let (a, b) = (h.mass[i - 1], h.mass[i]);
        if (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0) {
            h.balance_points.push(h.s[i - 1] + (h.s[i] - h.s[i - 1]) * a / (a - b));
        } else if b == 0.0 && i + 1 < h.s.len() && a != 0.0 && h.mass[i + 1] != 0.0 && a.signum() != h.mass[i + 1].signum() {
            h.balance_points.push(h.s[i]);
        }
    }
    // Sweep the levels: at every level the mass line above it (or below,
    // for negative levels) splits into intervals; each closed one moves
    // that slice of volume over its width.
    let (top, bottom) = (h.mass.iter().cloned().fold(0.0, f64::max), h.mass.iter().cloned().fold(0.0, f64::min));
    for (lo, hi, above) in [(0.0, top, true), (bottom, 0.0, false)] {
        if hi - lo <= 0.0 {
            continue;
        }
        let mut levels: Vec<f64> = h.mass.iter().copied().filter(|v| *v > lo && *v < hi).collect();
        levels.extend([lo, hi]);
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        for w in levels.windows(2) {
            let sub = 16;
            let dm = (w[1] - w[0]) / sub as f64;
            for j in 0..sub {
                let level = w[0] + (j as f64 + 0.5) * dm;
                for (a, b, at_end) in intervals(&h.s, &h.mass, level, above) {
                    if at_end {
                        continue;
                    }
                    let width = b - a;
                    h.moved += dm;
                    h.haul += width * dm;
                    if free > 0.0 && width > free {
                        h.over_volume += dm;
                        h.overhaul += (width - free) * dm;
                    } else {
                        h.free_volume += dm;
                    }
                }
            }
        }
    }
    // Loops between balance points, with their free haul chord.
    let mut cuts = vec![h.s[0]];
    cuts.extend(h.balance_points.iter().copied());
    cuts.push(h.s[h.s.len() - 1]);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let inside: Vec<usize> = (0..h.s.len()).filter(|i| h.s[*i] > a && h.s[*i] < b).collect();
        let Some(&peak) = inside.iter().max_by(|x, y| h.mass[**x].abs().total_cmp(&h.mass[**y].abs())) else { continue };
        let (pm, ps) = (h.mass[peak], h.s[peak]);
        if pm.abs() < 1e-9 {
            continue;
        }
        let closed = b < h.s[h.s.len() - 1] - 1e-9 || h.mass[h.s.len() - 1].abs() < 1e-9;
        // The interval round the peak at a level, and its width.
        let around = |level: f64| intervals(&h.s, &h.mass, level, pm > 0.0).into_iter().find(|(x, y, _)| *x <= ps + 1e-9 && *y >= ps - 1e-9);
        let chord = if free > 0.0 && closed {
            let wide = |level: f64| around(level).map(|(x, y, _)| y - x).unwrap_or(0.0);
            if wide(pm * 1e-6) <= free {
                None
            } else {
                // Bisection: the level where the loop is exactly as wide as the free haul.
                let (mut near0, mut far) = (pm * 1e-6, pm);
                for _ in 0..60 {
                    let mid = (near0 + far) / 2.0;
                    if wide(mid) > free { near0 = mid } else { far = mid }
                }
                around(far).map(|(x, y, _)| (far, x, y))
            }
        } else {
            None
        };
        h.loops.push(Loop { from: a, to: b, sign: pm.signum(), volume: pm.abs(), chord });
    }
    h
}

/// Read the corridor's section list: distance, cut and fill volumes per row.
pub fn read_corridor_csv(path: &Path) -> Result<Vec<Row>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let c: Vec<&str> = line.split(';').collect();
        if c.len() < 7 {
            continue;
        }
        let n = |i: usize| super::grading::number(c[i]);
        rows.push(Row { s: n(1)?, cut: n(5)?, fill: n(6)? });
    }
    if rows.len() < 2 {
        return Err(tf!("{}: te weinig dwarsprofielen", "{}: too few sections", path.display()));
    }
    Ok(rows)
}

/// Where the corridor of an axis wrote its section list.
pub fn corridor_csv(axis_name: &str) -> PathBuf {
    let name = if axis_name.is_empty() { "as" } else { axis_name };
    bridge::beside_drawing(&format!("corridor_{}.csv", name.replace(' ', "_").to_lowercase()))
}

/// Cut and fill areas (m²) across the axis at one station between the
/// design surface and the ground: the run of design next to the axis,
/// sampled every 0.1 m, at most `reach` m to each side.
fn section_areas(al: &Alignment, s: f64, f: f64, reach: f64, design: &Tin, ground: &mut Maaiveld) -> Option<(f64, f64)> {
    let (p, dir) = al.at(s * f);
    let rn = [dir.sin(), -dir.cos()];
    let at = |o: f64| [p[0] + rn[0] * o * f, p[1] + rn[1] * o * f];
    let step = 0.1;
    let n = (reach / step).round() as i64;
    // Start at the axis, or the nearest offset with a design height; then
    // cells of 0.1 m outwards on both sides while the design goes on.
    let start = (0..=n).flat_map(|k| [k, -k]).find(|k| design.height(at(*k as f64 * step)).is_some())?;
    let (mut cut, mut fill) = (0.0, 0.0);
    for dir in [1i64, -1] {
        let mut k = start;
        while k.abs() <= n {
            let mid = (k as f64 + 0.5 * dir as f64) * step;
            let Some(zd) = design.height(at(mid)) else { break };
            if let Some(zg) = ground.height(at(mid), f) {
                let dz = zd / f - zg;
                if dz > 0.0 { fill += dz * step } else { cut -= dz * step }
            }
            k += dir;
        }
    }
    Some((cut, fill))
}

/// Section volumes measured again: design (N) against the ground every
/// `every` m along the axis, average end areas between stations.
fn rows_from_tin(ctx: &Ctx, al: &Alignment, design: &Tin, ground: &mut Maaiveld, every: f64, reach: f64) -> Result<Vec<Row>, String> {
    let f = ctx.f;
    let stations: Vec<f64> = al.stations(every.max(0.5) * f, true).into_iter().map(|s| s / f).collect();
    let mut areas = Vec::new();
    for s in stations {
        if let Some((c, fl)) = section_areas(al, s, f, reach, design, ground) {
            areas.push((s, c, fl));
        }
    }
    if areas.len() < 2 {
        return Err(tr("het ontwerpoppervlak (N) ligt niet langs de as: maak eerst een corridor", "the design surface (N) does not lie along the axis: make a corridor first").into());
    }
    let mut rows = vec![Row { s: areas[0].0, cut: 0.0, fill: 0.0 }];
    for w in areas.windows(2) {
        let ds = w[1].0 - w[0].0;
        rows.push(Row { s: w[1].0, cut: (w[0].1 + w[1].1) / 2.0 * ds, fill: (w[0].2 + w[1].2) / 2.0 * ds });
    }
    Ok(rows)
}

/// Where the section volumes come from.
#[derive(Clone, Debug, PartialEq)]
pub enum VolumeSource {
    /// The corridor's list when there is one, else measured on the TINs.
    Auto,
    Corridor,
    Tin,
    Csv(PathBuf),
}

pub struct Request {
    pub axis: Option<Handle>,
    pub from: VolumeSource,
    /// m³ of fill a m³ of reused cut makes (shrinkage < 1, swell > 1).
    pub factor: f64,
    /// Share of the cut that is suitable for fill (0–1).
    pub reuse: f64,
    /// Free haul distance (m, 0 = none).
    pub free: f64,
    /// Station distance when measuring on the TINs (m), and how far across.
    pub every: f64,
    pub reach: f64,
    /// Lower-left corner of the diagram (drawing), or below the project.
    pub at: Option<XY>,
}

fn tool_layer(name: &str, aci: u8) -> LayerSpec {
    LayerSpec { name: name.to_string(), color: aci, rgb: crate::nlcs::color_rgb(aci), true_color: None, lineweight: None, linetype: "CONTINUOUS".into(), object_id: 0 }
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

/// "m³·km", with the right decimal sign.
fn m3km(v: f64) -> String {
    whole(v / 1000.0)
}

/// `CIV_GRONDBALANS`: the mass haul diagram of an axis.
pub fn mass_haul_diagram(opt: &super::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["tin", "as"], &req.axis.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = as_profiel::axis(&doc, req.axis)?;
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let csv = corridor_csv(&al.name);
    let use_csv = match &req.from {
        VolumeSource::Auto => csv.exists().then(|| csv.clone()),
        VolumeSource::Corridor => Some(csv.clone()),
        VolumeSource::Csv(p) => Some(p.clone()),
        VolumeSource::Tin => None,
    };
    let (rows, source) = match use_csv {
        Some(p) => (read_corridor_csv(&p)?, tf!("dwarsprofielen van de corridor ({})", "corridor sections ({})", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        None => {
            progress(tr("Dwarsprofielen meten op het ontwerpoppervlak…", "Measuring sections on the design surface…").into());
            let design = Tin::of_status(&doc, 'N').ok_or(tr("geen corridorlijst en geen ontwerpoppervlak (N): maak eerst een corridor", "no corridor list and no design surface (N): make a corridor first"))?;
            let mut ground = Maaiveld::find(&ctx, &doc)?;
            let rows = rows_from_tin(&ctx, &al, &design, &mut ground, req.every, req.reach)?;
            ground.note_source();
            (rows, tf!("ontwerp (N) t.o.v. {} om de {} m", "design (N) against {} every {} m", ground.label(), fmt_n(req.every)))
        }
    };
    let h = mass_haul(&rows, req.factor, req.reuse, req.free);
    let len = al.length() / f;
    // The diagram: stations across, volume up.
    let (vmin, vmax) = (h.mass.iter().cloned().fold(0.0, f64::min), h.mass.iter().cloned().fold(0.0, f64::max));
    let vstep = nice_step(((vmax - vmin) / 6.0).max(1.0));
    let (base, top) = ((vmin / vstep).floor() * vstep - vstep, (vmax / vstep).ceil() * vstep + vstep);
    let graph_h = 70.0 * k;
    let vs = graph_h / (top - base);
    let row = 10.0 * k;
    let band = 2.0 * row;
    let head = 42.0 * k;
    let at = req.at.unwrap_or_else(|| {
        let size = [head + al.length() + 10.0 * k, band + graph_h + 30.0 * k];
        let spot = bridge::layout_spot("onder", size, as_profiel::clear_area(&ctx, &al), 30.0 * k);
        [spot[0] + head, spot[1]]
    });
    let gx = |s: f64| at[0] + (s - 0.0) * f;
    let gy = |v: f64| at[1] + band + (v - base) * vs;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let line_l = tool_layer("OI-GRONDBALANS-MASSALIJN", 30);
    let bal_l = tool_layer("OI-GRONDBALANS-BALANS", 3);
    let free_l = tool_layer("OI-GRONDBALANS-VRIJ", 5);
    let fwd_l = tool_layer("OI-GRONDBALANS-VOORUIT", 41);
    let back_l = tool_layer("OI-GRONDBALANS-TERUG", 151);
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    let mut out = Out::default();
    let (x0, x1, y0, y1) = (at[0] - head, gx(len), at[1], gy(top));
    let title = tf!("Grondbalans {name}", "Mass haul {name}");
    let data = record(&[
        ("grondbalans", title.clone()),
        ("as", al.name.clone()),
        ("factor", format!("{}", req.factor)),
        ("hergebruik", format!("{}", req.reuse)),
        ("vrij_m", format!("{}", req.free)),
        ("transport_m3km", format!("{:.1}", h.haul / 1000.0)),
    ]);
    // The mass line first: its handle is the first one written.
    let pts: Vec<XY> = h.s.iter().zip(&h.mass).map(|(s, m)| [gx(*s), gy(*m)]).collect();
    out.put(&line_l, crate::emit::polylines(&[(pts, false)], &line_l.name, k, Some(&data)));
    out.put(&frame, crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], &frame.name, k, None));
    out.put(&frame, pl(vec![[x0, y0 + row], [x1, y0 + row]], &frame));
    out.put(&frame, pl(vec![[x0, y0 + band], [x1, y0 + band]], &frame));
    out.put(&frame, pl(vec![[at[0], y0], [at[0], y1]], &frame));
    for (r, t) in [tr("Massa (m³)", "Mass (m³)"), tr("Stationering", "Stations")].iter().enumerate() {
        let tx = ctx.text([x0 + 2.0 * k, y0 + (r as f64 + 0.5) * row], t, 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(tx, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    // Volume grid with labels.
    let mut v = base;
    while v <= top + 1e-9 {
        let y = gy(v);
        if v > base + 1e-9 && v < top - 1e-9 {
            out.put(&frame, pl(vec![[at[0], y], [x1, y]], &frame));
        }
        let t = ctx.text([at[0] - 2.0 * k, y], &whole(v), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        v += vstep;
    }
    // Stations and the mass at them.
    let label = nice_step(20.0 * opt.nlcs.scale / 1000.0).max(1.0);
    let mass_at = |s: f64| -> f64 {
        for i in 1..h.s.len() {
            if s <= h.s[i] + 1e-9 {
                let t = if h.s[i] > h.s[i - 1] { (s - h.s[i - 1]) / (h.s[i] - h.s[i - 1]) } else { 0.0 };
                return h.mass[i - 1] + (h.mass[i] - h.mass[i - 1]) * t.clamp(0.0, 1.0);
            }
        }
        h.mass[h.mass.len() - 1]
    };
    let mut s = 0.0;
    while s <= len + 1e-9 {
        let x = gx(s);
        out.put(&frame, pl(vec![[x, y0], [x, y0 + band]], &frame));
        if s > 1e-9 && s < len - 1e-9 {
            out.put(&frame, pl(vec![[x, gy(base)], [x, y1]], &frame));
        }
        out.put(&frame_t, [ctx.text([x - 1.0 * k, y0 + 1.5 * row], &station(al.start + s), 1.8, std::f64::consts::FRAC_PI_2, &frame_t, None)]);
        out.put(&frame_t, [ctx.text([x - 1.0 * k, y0 + 0.5 * row], &whole(mass_at(s)), 1.8, std::f64::consts::FRAC_PI_2, &frame_t, None)]);
        s += label;
    }
    // The haul: the area between the mass line and the balance line,
    // forward (cut moves ahead) and back.
    let (mut fwd, mut back) = (super::model3d::MeshBuilder::default(), super::model3d::MeshBuilder::default());
    for i in 1..h.s.len() {
        let (sa, sb, ma, mb) = (h.s[i - 1], h.s[i], h.mass[i - 1], h.mass[i]);
        let mut parts = vec![(sa, ma, sb, mb)];
        if ma * mb < 0.0 {
            let sc = sa + (sb - sa) * ma / (ma - mb);
            parts = vec![(sa, ma, sc, 0.0), (sc, 0.0, sb, mb)];
        }
        for (p, pm, q, qm) in parts {
            let mesh = if pm + qm >= 0.0 { &mut fwd } else { &mut back };
            mesh.quad([gx(p), gy(0.0), 0.0], [gx(q), gy(0.0), 0.0], [gx(q), gy(qm), 0.0], [gx(p), gy(pm), 0.0]);
        }
    }
    for (mesh, spec) in [(fwd, &fwd_l), (back, &back_l)] {
        if !mesh.is_empty() {
            out.put(spec, mesh.finish(&spec.name, None, None));
        }
    }
    // Balance line and points.
    out.put(&bal_l, pl(vec![[at[0], gy(0.0)], [x1, gy(0.0)]], &bal_l));
    for b in &h.balance_points {
        out.put(&bal_l, pl(vec![[gx(*b), gy(base)], [gx(*b), y1]], &bal_l));
        let t = ctx.text([gx(*b) + 1.0 * k, gy(0.0) + 1.0 * k], &station(al.start + b), 1.8, std::f64::consts::FRAC_PI_2, &bal_l, None);
        out.put(&bal_l, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Bottom)]);
    }
    // Free haul chords.
    for lp in &h.loops {
        if let Some((level, a, b)) = lp.chord {
            out.put(&free_l, pl(vec![[gx(a), gy(level)], [gx(b), gy(level)]], &free_l));
            for x in [a, b] {
                out.put(&free_l, pl(vec![[gx(x), gy(level) - 1.0 * k], [gx(x), gy(level) + 1.0 * k]], &free_l));
            }
            let t = ctx.text([gx((a + b) / 2.0), gy(level) + lp.sign * 1.5 * k], &tf!("vrij {} m", "free {} m", fmt_n(req.free)), 1.8, 0.0, &free_l, None);
            out.put(&free_l, [aligned(t, TextHorizontalAlignment::Center, if lp.sign > 0.0 { TextVerticalAlignment::Bottom } else { TextVerticalAlignment::Top })]);
        }
    }
    // Title and totals.
    let end = h.mass[h.mass.len() - 1];
    let mut lines = vec![
        tf!(
            "Afgraven {} m³ (hergebruik {} %, factor {}) · ophogen {} m³ · {}",
            "Cut {} m³ (reuse {} %, factor {}) · fill {} m³ · {}",
            whole(h.cut),
            fmt_n(req.reuse * 100.0),
            fmt_n(req.factor),
            whole(h.fill),
            source
        ),
        tf!(
            "Verplaatst {} m³, transport {} m³·km (gemiddeld {} m) · {} balanspunt(en) · {}",
            "Moved {} m³, haul {} m³·km (mean {} m) · {} balance point(s) · {}",
            whole(h.moved),
            m3km(h.haul),
            whole(if h.moved > 0.0 { h.haul / h.moved } else { 0.0 }),
            h.balance_points.len(),
            if end >= 0.0 { tf!("overschot {} m³", "surplus {} m³", whole(h.surplus)) } else { tf!("tekort {} m³", "shortage {} m³", whole(h.shortage)) }
        ),
    ];
    if req.free > 0.0 {
        lines.push(tf!(
            "Vrije transportafstand {} m: {} m³ binnen, {} m³ verder, bovenmatig transport {} m³·km",
            "Free haul distance {} m: {} m³ within, {} m³ beyond, overhaul {} m³·km",
            fmt_n(req.free),
            whole(h.free_volume),
            whole(h.over_volume),
            m3km(h.overhaul)
        ));
    }
    let t = ctx.text([x0, y1 + (5.0 + lines.len() as f64 * 4.0) * k], &title, 3.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    for (i, l) in lines.iter().enumerate() {
        let t = ctx.text([x0, y1 + (4.0 + (lines.len() - 1 - i) as f64 * 4.0) * k], l, 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    let handles = out.write_handles(&title)?;
    if let Some(first) = handles.first() {
        bridge::index_add("grondbalans", &al.name, &[*first], false);
    }
    // The list beside the drawing.
    let dcm = |v: f64, d: usize| format!("{v:.d$}").replace('.', ",");
    let mut text = String::from("Station;Afstand (m);Afgraven (m3);Ophogen (m3);Hergebruik (m3);Afvoer (m3);Massa (m3)\n");
    for (i, r) in rows.iter().enumerate() {
        text.push_str(&format!("{};{};{};{};{};{};{}\n", station(al.start + r.s), dcm(r.s, 2), dcm(r.cut, 1), dcm(r.fill, 1), dcm(h.reused[i], 1), dcm(h.waste[i], 1), dcm(h.mass[i], 1)));
    }
    let csv_out = bridge::beside_drawing(&format!("grondbalans_{}.csv", name.replace(' ', "_").to_lowercase()));
    let _ = std::fs::write(&csv_out, text);
    api::note("grondbalans", handles.first().map(|h| api::handle_text(*h)).unwrap_or_default());
    api::note("as", al.name.clone());
    api::note("bron", source.clone());
    api::note("factor", req.factor);
    api::note("hergebruik", req.reuse);
    api::note("vrij_m", req.free);
    api::note("afgraven_m3", h.cut);
    api::note("ophogen_m3", h.fill);
    api::note("verplaatst_m3", h.moved);
    api::note("transport_m3km", h.haul / 1000.0);
    api::note("gemiddelde_afstand_m", if h.moved > 0.0 { h.haul / h.moved } else { 0.0 });
    api::note("binnen_vrij_m3", h.free_volume);
    api::note("bovenmatig_m3", h.over_volume);
    api::note("bovenmatig_m3km", h.overhaul / 1000.0);
    api::note("overschot_m3", h.surplus);
    api::note("tekort_m3", h.shortage);
    api::note("eindmassa_m3", end);
    api::note("balanspunten", h.balance_points.iter().map(|b| serde_json::json!({"station": al.start + b, "tekst": station(al.start + b)})).collect::<Vec<_>>());
    api::note(
        "lussen",
        h.loops.iter().map(|l| serde_json::json!({"van": al.start + l.from, "tot": al.start + l.to, "richting": if l.sign > 0.0 { "vooruit" } else { "terug" }, "volume_m3": l.volume, "vrij_koorde": l.chord.map(|c| serde_json::json!({"massa_m3": c.0, "van": al.start + c.1, "tot": al.start + c.2}))})).collect::<Vec<_>>(),
    );
    api::note(
        "stations",
        rows.iter().enumerate().map(|(i, r)| serde_json::json!({"station": al.start + r.s, "afgraven_m3": r.cut, "ophogen_m3": r.fill, "hergebruik_m3": h.reused[i], "afvoer_m3": h.waste[i], "massa_m3": h.mass[i]})).collect::<Vec<_>>(),
    );
    api::note("csv", csv_out.to_string_lossy().into_owned());
    api::note("plaats", serde_json::json!([at[0], at[1]]));
    Ok(tf!(
        "{title}: {} stations, transport {} m³·km, {} balanspunt(en), {} — {} elementen. Lijst: {}",
        "{title}: {} stations, haul {} m³·km, {} balance point(s), {} — {} entities. List: {}",
        rows.len(),
        m3km(h.haul),
        h.balance_points.len(),
        if end >= 0.0 { tf!("overschot {} m³", "surplus {} m³", whole(h.surplus)) } else { tf!("tekort {} m³", "shortage {} m³", whole(h.shortage)) },
        handles.len(),
        csv_out.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: &[(f64, f64, f64)]) -> Vec<Row> {
        v.iter().map(|&(s, cut, fill)| Row { s, cut, fill }).collect()
    }

    #[test]
    fn one_cut_one_fill() {
        // 1000 m³ cut spread evenly over 0–100, the same fill over 100–200:
        // a triangle-shaped mass line up to 1000 m³ at 100, back to zero at
        // 200. Every slice moves its width: haul = area = ½ · 200 · 1000.
        let mut v = vec![(0.0, 0.0, 0.0)];
        for i in 1..=20 {
            let s = i as f64 * 10.0;
            v.push(if s <= 100.0 { (s, 100.0, 0.0) } else { (s, 0.0, 100.0) });
        }
        let h = mass_haul(&rows(&v), 1.0, 1.0, 0.0);
        assert!((h.mass[10] - 1000.0).abs() < 1e-9 && h.mass[20].abs() < 1e-9);
        assert!((h.moved - 1000.0).abs() < 1e-6, "{}", h.moved);
        assert!((h.haul - 100_000.0).abs() < 1.0, "{}", h.haul);
        assert!(h.surplus.abs() < 1e-9 && h.shortage.abs() < 1e-9);
        // Free haul 50 m: the chord sits where the loop is 50 m wide, at
        // 750 m³; 250 m³ stays within, 750 m³ goes further. Overhaul:
        // ∫ (w(m) − 50) dm over 0–750 with w = 200 (1 − m / 1000).
        let h = mass_haul(&rows(&v), 1.0, 1.0, 50.0);
        assert!((h.free_volume - 250.0).abs() < 1.0 && (h.over_volume - 750.0).abs() < 1.0, "{h:?}");
        let expect = 200.0 * 750.0 - 100.0 * 750.0 * 750.0 / 1000.0 - 50.0 * 750.0;
        assert!((h.overhaul - expect).abs() / expect < 1e-3, "{} vs {expect}", h.overhaul);
        let (level, a, b) = h.loops[0].chord.unwrap();
        assert!((level - 750.0).abs() < 1e-3 && (b - a - 50.0).abs() < 1e-3, "{:?}", h.loops[0]);
    }

    #[test]
    fn balance_points_surplus_and_reuse() {
        // Fill first (−500), then cut (+800): one balance point, 300 m³ left.
        let h = mass_haul(&rows(&[(0.0, 0.0, 0.0), (50.0, 0.0, 500.0), (100.0, 800.0, 0.0)]), 1.0, 1.0, 0.0);
        assert_eq!(h.balance_points.len(), 1);
        assert!((h.balance_points[0] - (50.0 + 50.0 * 500.0 / 800.0)).abs() < 1e-9);
        assert!((h.surplus - 300.0).abs() < 1e-9 && h.shortage == 0.0);
        assert!(h.loops[0].sign < 0.0);
        // Only half the cut is usable, and it shrinks by 10 %: a shortage.
        let h = mass_haul(&rows(&[(0.0, 0.0, 0.0), (50.0, 0.0, 500.0), (100.0, 800.0, 0.0)]), 0.9, 0.5, 0.0);
        assert!((h.mass[2] - (360.0 - 500.0)).abs() < 1e-9);
        assert!((h.shortage - 140.0).abs() < 1e-9 && (h.surplus - 400.0).abs() < 1e-9, "{h:?}");
        // Moved: the 360 m³ that fills part of the 500.
        assert!((h.moved - 360.0).abs() < 1e-6, "{}", h.moved);
    }

    #[test]
    fn the_corridor_list() {
        let dir = std::env::temp_dir().join(format!("oi-massa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("corridor_as.csv");
        std::fs::write(&p, "Station;Afstand (m);Ontwerphoogte (m);Afgraven (m2);Ophogen (m2);Afgraven (m3);Ophogen (m3);Balans (m3)\n0+000;0,00;1,500;0,00;2,00;0,0;0,0;0,0\n0+010;10,00;1,600;1,00;1,00;5,0;15,0;10,0\n").unwrap();
        let r = read_corridor_csv(&p).unwrap();
        assert_eq!(r, rows(&[(0.0, 0.0, 0.0), (10.0, 5.0, 15.0)]));
        let _ = std::fs::remove_dir_all(dir);
    }
}
