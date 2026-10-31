//! Corridor (Road "Road"): an assembly — the template
//! cross section — swept along an axis and its design profile. Lanes slope
//! away from the axis and tilt in curves (superelevation, with a runoff
//! before and after); at the outer edge a slope runs down (fill) or up (cut)
//! until it meets the existing ground (daylight). The result is a design
//! surface (OG-TRIANGULATIE, status N), 3D feature lines, cut and fill per
//! section with average-end-area volumes, and optionally section views.

use std::collections::HashSet;

use ocs_plugin_api::host::codec::entities::Polyline3D;
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use super::as_profiel::{self, Alignment, Pvi, Seg, SectionView};
use super::terrein::{Tin, P3};
use super::{record, station, v3, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::{Element, Status};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartKind {
    Lane,
    Shoulder,
    Crest,
}

impl PartKind {
    fn code(self) -> &'static str {
        match self {
            PartKind::Lane => "RS",
            PartKind::Shoulder => "BE",
            PartKind::Crest => "KR",
        }
    }
}

/// One subassembly, outward from the axis: width (m) and cross slope
/// (fraction; negative runs down away from the axis).
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub width: f64,
    pub slope: f64,
    pub kind: PartKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub name: String,
    /// The same on both sides.
    pub parts: Vec<Part>,
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
    pub const NAMES: [&'static str; 4] = ["weg", "fietspad", "snelweg", "dijk"];

    /// Ready-made assemblies: a 2×1 rural road, a cycle path, a 2×2
    /// motorway half profile and a dike.
    pub fn template(name: &str) -> Option<Assembly> {
        let lane = |w: f64, s: f64| Part { width: w, slope: s, kind: PartKind::Lane };
        let berm = |w: f64, s: f64| Part { width: w, slope: s, kind: PartKind::Shoulder };
        let (parts, fill, cut, sup) = match name.to_ascii_lowercase().as_str() {
            "weg" => (vec![lane(3.25, -0.025), berm(1.5, -0.04)], 3.0, 2.0, 0.04),
            "fietspad" => (vec![lane(1.5, -0.02), berm(0.5, -0.04)], 2.0, 1.5, 0.0),
            "snelweg" => (vec![lane(7.0, -0.025), lane(3.5, -0.025), berm(2.0, -0.04)], 3.0, 2.0, 0.05),
            "dijk" => (vec![Part { width: 2.5, slope: -0.02, kind: PartKind::Crest }], 3.0, 3.0, 0.0),
            _ => return None,
        };
        Some(Assembly { name: name.to_ascii_lowercase(), parts, fill, cut, reach: 60.0, superelevation: sup, runoff: 30.0 })
    }

    /// A template with the arguments' changes: RIJSTROOK, BERM (widths, m),
    /// HELLING, BERMHELLING (%, down > 0), TALUD, INGRAVING (1:n), KRUIN
    /// (half crest, m), VERKANTING (%), REIKWIJDTE (m).
    pub fn with_args(mut self, a: &super::api::Args) -> Result<Assembly, String> {
        for p in self.parts.iter_mut() {
            match p.kind {
                PartKind::Lane => {
                    if let Some(w) = a.f64("RIJSTROOK")? {
                        p.width = w;
                    }
                    if let Some(h) = a.f64("HELLING")? {
                        p.slope = -h / 100.0;
                    }
                }
                PartKind::Shoulder => {
                    if let Some(w) = a.f64("BERM")? {
                        p.width = w;
                    }
                    if let Some(h) = a.f64("BERMHELLING")? {
                        p.slope = -h / 100.0;
                    }
                }
                PartKind::Crest => {
                    if let Some(w) = a.f64("KRUIN")? {
                        p.width = w;
                    }
                    if let Some(h) = a.f64("HELLING")? {
                        p.slope = -h / 100.0;
                    }
                }
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

    pub fn half_width(&self) -> f64 {
        self.parts.iter().map(|p| p.width).sum()
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

/// One computed cross section.
#[derive(Clone, Debug)]
pub struct Section {
    /// Distance along the axis (m).
    pub s: f64,
    pub z_axis: f64,
    /// Design points from the left daylight to the right one: offset (m,
    /// right > 0), height (m) and a code (RS lane, BE shoulder, KR crest,
    /// AS axis, TL daylight).
    pub pts: Vec<(f64, f64, &'static str)>,
    /// Whether each side's daylight is in fill (true) or cut; None without ground.
    pub fill_side: [Option<bool>; 2],
    pub cut: f64,
    pub fill: f64,
}

/// Cross slopes of the lanes at `s` (m): [left, right], crowned on
/// straights, tilted towards the inside in curves.
fn lane_slopes(asm: &Assembly, curve: Option<f64>, crown: f64) -> [f64; 2] {
    match curve {
        // `turn` > 0: a left curve, the inside is left; 0..1 runoff share.
        Some(t) if asm.superelevation > 0.0 => {
            let e = asm.superelevation.max(-crown);
            let share = t.abs().min(1.0);
            let inside = crown + (-e - crown) * share;
            let outside = crown + (e - crown) * share;
            if t > 0.0 { [inside, outside] } else { [outside, inside] }
        }
        _ => [crown, crown],
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

/// One side, outward (dir −1 left, +1 right): the parts, then the slope to
/// the ground. Returns the points and whether the daylight is in fill.
fn side(asm: &Assembly, z0: f64, dir: f64, lane_slope: f64, ground: &mut dyn FnMut(f64) -> Option<f64>) -> (Vec<(f64, f64, &'static str)>, Option<bool>) {
    let (mut o, mut z) = (0.0, z0);
    let mut pts = Vec::new();
    for p in &asm.parts {
        let slope = if p.kind == PartKind::Lane { lane_slope } else { p.slope };
        o += dir * p.width;
        z += slope * p.width;
        pts.push((o, z, p.kind.code()));
    }
    let Some(g0) = ground(o) else {
        pts.push((o, z, "TL"));
        return (pts, None);
    };
    let fill = z >= g0;
    let grade = if fill { -1.0 / asm.fill.max(0.1) } else { 1.0 / asm.cut.max(0.1) };
    let step = 0.25;
    let mut prev = (0.0, z - g0);
    let mut t = step;
    while t <= asm.reach + 1e-9 {
        let (ot, zt) = (o + dir * t, z + grade * t);
        let Some(gt) = ground(ot) else { break };
        let d = zt - gt;
        if d == 0.0 || d.signum() != prev.1.signum() {
            // Between the previous step and this one.
            let tc = prev.0 + (t - prev.0) * prev.1 / (prev.1 - d);
            pts.push((o + dir * tc, z + grade * tc, "TL"));
            return (pts, Some(fill));
        }
        prev = (t, d);
        t += step;
    }
    let tc = prev.0;
    pts.push((o + dir * tc, z + grade * tc, "TL"));
    (pts, Some(fill))
}

/// Cut and fill areas (m²) between the design line and the ground.
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
    let crown = asm.parts.iter().find(|p| p.kind == PartKind::Lane).map(|p| p.slope).unwrap_or(-0.025);
    let slopes = lane_slopes(asm, curve, crown);
    let (left, lf) = side(asm, z0, -1.0, slopes[0], ground);
    let (right, rf) = side(asm, z0, 1.0, slopes[1], ground);
    let mut pts: Vec<(f64, f64, &'static str)> = left.into_iter().rev().collect();
    pts.push((0.0, z0, "AS"));
    pts.extend(right);
    let line: Vec<(f64, f64)> = pts.iter().map(|p| (p.0, p.1)).collect();
    let (cut, fill) = areas(&line, ground);
    Section { s, z_axis: z0, pts, fill_side: [lf, rf], cut, fill }
}

/// Average end areas: (cut, fill) volume between consecutive sections and
/// the running balance (fill − cut), m³.
pub fn volumes(sections: &[Section]) -> Vec<(f64, f64, f64)> {
    let mut out = vec![(0.0, 0.0, 0.0)];
    let mut balance = 0.0;
    for w in sections.windows(2) {
        let ds = w[1].s - w[0].s;
        let c = (w[0].cut + w[1].cut) / 2.0 * ds;
        let f = (w[0].fill + w[1].fill) / 2.0 * ds;
        balance += f - c;
        out.push((c, f, balance));
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
}

/// `CIV_CORRIDOR`.
pub fn corridor(opt: &super::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let mut opt = opt.clone();
    // A design: the new status, on both the surface and the lines.
    opt.nlcs.status = Status::N;
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin", "ontwerpprofiel"], &req.axis.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = as_profiel::axis(&doc, req.axis)?;
    let (f, k) = (ctx.f, ctx.k);
    let len_m = al.length() / f;
    let asm = req.assembly;
    let grade = match req.grade {
        Some(g) => g,
        None => Grade::Profile(profile_of(&doc, &al.name).ok_or_else(|| {
            format!("{} heeft geen ontwerpprofiel: maak er een (CIV_ONTWERPPROFIEL) of geef NIVEAU=… of BOVEN=…", if al.name.is_empty() { "de as" } else { &al.name })
        })?),
    };
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let stations: Vec<f64> = al.stations(req.every.max(0.5) * f, true).into_iter().map(|s| s / f).collect();
    if stations.len() > 3000 {
        return Err(format!("{} dwarsprofielen is te veel: kies een grotere afstand", stations.len()));
    }
    // Ground at the axis, for "above ground" and the sections' labels.
    let axis_ground: Vec<Option<f64>> = stations.iter().map(|s| ground.height(al.at(s * f).0, f)).collect();
    let design_at = |i: usize, s: f64| -> Option<f64> {
        match &grade {
            Grade::Profile(p) => Some(as_profiel::design_z(p, s)),
            Grade::Level(z) => Some(*z),
            Grade::AboveGround(dz) => {
                // Mean ground within 50 m either side, so the road does not follow every bump.
                let near: Vec<f64> = stations.iter().zip(&axis_ground).filter(|(t, _)| (*t - s).abs() <= 50.0).filter_map(|(_, g)| *g).collect();
                let _ = i;
                (!near.is_empty()).then(|| near.iter().sum::<f64>() / near.len() as f64 + dz)
            }
        }
    };
    progress(format!("Corridor: {} dwarsprofielen ({}) …", stations.len(), ground.label()));
    let mut sections = Vec::with_capacity(stations.len());
    let mut skipped = 0;
    for (i, s) in stations.iter().enumerate() {
        if i % 25 == 0 {
            progress(format!("Corridor: dwarsprofiel {}/{}…", i + 1, stations.len()));
        }
        let Some(z0) = design_at(i, *s) else {
            skipped += 1;
            continue;
        };
        let (p, dir) = al.at(s * f);
        let rn = [dir.sin(), -dir.cos()];
        let mut g = |o: f64| ground.height([p[0] + rn[0] * o * f, p[1] + rn[1] * o * f], f);
        let curve = curve_share(&al, *s, f, asm.runoff);
        sections.push(section(&asm, *s, z0, curve, &mut g));
    }
    if sections.len() < 2 {
        return Err(ground.error().unwrap_or_else(|| "te weinig dwarsprofielen met een ontwerphoogte".into()));
    }
    let vols = volumes(&sections);
    let (cut, fill): (f64, f64) = vols.iter().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
    progress("Corridor tekenen…".into());

    // Plan positions of every design point.
    let plan = |sct: &Section, o: f64| -> XY {
        let (p, dir) = al.at(sct.s * f);
        [p[0] + dir.sin() * o * f, p[1] - dir.cos() * o * f]
    };
    let mut out = Out::default();
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let data = record(&[("corridor", format!("Corridor {name}")), ("as", al.name.clone()), ("sjabloon", asm.name.clone())]);
    // Design surface: strips between consecutive sections.
    let n = sections[0].pts.len();
    let faces_spec = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let mut mesh = super::model3d::MeshBuilder::default();
    for w in sections.windows(2) {
        if w[0].pts.len() != n || w[1].pts.len() != n {
            continue;
        }
        let a: Vec<P3> = w[0].pts.iter().map(|q| {
            let xy = plan(&w[0], q.0);
            [xy[0], xy[1], q.1 * f]
        }).collect();
        let b: Vec<P3> = w[1].pts.iter().map(|q| {
            let xy = plan(&w[1], q.0);
            [xy[0], xy[1], q.1 * f]
        }).collect();
        for j in 0..n - 1 {
            mesh.tri(a[j], a[j + 1], b[j + 1]);
            mesh.tri(a[j], b[j + 1], b[j]);
        }
    }
    let face_count = mesh.faces();
    let mesh_ents = mesh.finish(&faces_spec.name, None, Some(&data));
    let mesh_count = mesh_ents.len();
    out.put(&faces_spec, mesh_ents);
    // Feature lines: one 3D line per template point, the daylight split in
    // runs of fill (toe) and cut (crest).
    let poly3 = |pts: Vec<P3>, layer: &str| {
        let mut e = EntityType::Polyline3D(Polyline3D::from_points(pts.iter().map(|p| v3([p[0], p[1]], p[2])).collect()));
        let c = e.common_mut();
        c.layer = layer.to_string();
        c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        c.linetype_scale = k;
        e
    };
    let lane = ctx.layer("VH", "GESLOTENVERHARDING_ASFALT", Element::G)?;
    let berm = ctx.layer("GR", "GRAS_BERM", Element::G)?;
    let crest = ctx.layer("GW", "DIJK_KRUIN", Element::G)?;
    let toe = ctx.layer("GW", "TALUD_TEEN", Element::G)?;
    let top = ctx.layer("GW", "TALUD_KRUIN", Element::G)?;
    let axis3 = ctx.layer("AM", "AS", Element::G)?;
    let mut lines = 0;
    for j in 0..n {
        let code = sections[0].pts[j].2;
        let pts: Vec<P3> = sections.iter().filter(|s| s.pts.len() == n).map(|s| {
            let xy = plan(s, s.pts[j].0);
            [xy[0], xy[1], s.pts[j].1 * f]
        }).collect();
        if pts.len() < 2 {
            continue;
        }
        match code {
            "TL" => {
                let side = if sections[0].pts[j].0 < 0.0 { 0 } else { 1 };
                let mut run: Vec<P3> = Vec::new();
                let mut kind: Option<bool> = None;
                for (s, p) in sections.iter().filter(|s| s.pts.len() == n).zip(&pts) {
                    let fill = s.fill_side[side];
                    if fill != kind && !run.is_empty() {
                        run.push(*p);
                        if run.len() > 1 {
                            if let Some(fl) = kind {
                                let l = if fl { &toe } else { &top };
                                out.put(l, [poly3(std::mem::take(&mut run), &l.name)]);
                                lines += 1;
                            }
                        }
                        run.clear();
                    }
                    kind = fill;
                    run.push(*p);
                }
                if run.len() > 1 {
                    if let Some(fl) = kind {
                        let l = if fl { &toe } else { &top };
                        out.put(l, [poly3(run, &l.name)]);
                        lines += 1;
                    }
                }
            }
            c => {
                let l = match c {
                    "RS" => &lane,
                    "BE" => &berm,
                    "KR" => &crest,
                    _ => &axis3,
                };
                let mut e = poly3(pts, &l.name);
                if c == "AS" {
                    e.common_mut().extended_data.add_record(data.clone());
                }
                out.put(l, [e]);
                lines += 1;
            }
        }
    }
    // Section views with the design, when asked.
    let mut view_rows = 0;
    if let Some(at) = req.views {
        let left = sections.iter().map(|s| -s.pts[0].0).fold(0.0, f64::max).ceil() + 2.0;
        let right = sections.iter().map(|s| s.pts[s.pts.len() - 1].0).fold(0.0, f64::max).ceil() + 2.0;
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
                SectionView {
                    s: sct.s * f,
                    ground: g,
                    design: sct.pts.iter().map(|q| (q.0, q.1)).collect(),
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
        view_rows = as_profiel::draw_section_views(&vctx, &al, &views, left, right, place, &format!("ontwerp {} over {}", asm.name, ground.label()), " — corridor", &mut out)?.0;
    }
    let handles = out.write_handles(&format!("Corridor {name}"))?;
    ground.note_source();
    // The design surface and the corridor's lines, for later commands.
    crate::bridge::index_add("tin", "N", &handles[..mesh_count.min(handles.len())], false);
    crate::bridge::index_add("corridor", &format!("Corridor {name}"), &handles[mesh_count.min(handles.len())..(mesh_count + lines).min(handles.len())], true);
    // The earthwork list beside the drawing.
    let mut csv = String::from("Station;Afstand (m);Ontwerphoogte (m);Afgraven (m2);Ophogen (m2);Afgraven (m3);Ophogen (m3);Balans (m3)\n");
    let dec = |v: f64, d: usize| format!("{v:.d$}").replace('.', ",");
    for (sct, v) in sections.iter().zip(&vols) {
        csv.push_str(&format!("{};{};{};{};{};{};{};{}\n", station(al.start + sct.s), dec(sct.s, 2), dec(sct.z_axis, 3), dec(sct.cut, 2), dec(sct.fill, 2), dec(v.0, 1), dec(v.1, 1), dec(v.2, 1)));
    }
    let csv_path = crate::bridge::beside_drawing(&format!("corridor_{}.csv", name.replace(' ', "_").to_lowercase()));
    let _ = std::fs::write(&csv_path, csv);
    super::api::note("corridor", format!("Corridor {name}"));
    super::api::note("sjabloon", asm.name.clone());
    super::api::note("lengte_m", len_m);
    super::api::note("dwarsprofielen", sections.len());
    super::api::note("driehoeken", face_count);
    super::api::note("kantlijnen", lines);
    super::api::note("afgraven_m3", cut);
    super::api::note("ophogen_m3", fill);
    super::api::note("balans_m3", fill - cut);
    super::api::note("csv", csv_path.to_string_lossy().into_owned());
    super::api::note("eerste_handle", handles.first().map(|h| super::api::handle_text(*h)).unwrap_or_default());
    super::api::note(
        "profielen",
        sections.iter().zip(&vols).map(|(s, v)| serde_json::json!({"station": al.start + s.s, "ontwerp": s.z_axis, "afgraven_m2": s.cut, "ophogen_m2": s.fill, "balans_m3": v.2})).collect::<Vec<_>>(),
    );
    let (zlo, zhi) = sections.iter().fold((f64::MAX, f64::MIN), |a, s| (a.0.min(s.z_axis), a.1.max(s.z_axis)));
    Ok(format!(
        "Corridor {name} ({}, {:.0} m): {} dwarsprofielen{}, ontwerp {} tot {} m; afgraven {} m³, ophogen {} m³ (balans {}{} m³); {face_count} driehoeken, {lines} kantlijnen{} — {} elementen. Lijst: {}",
        asm.name,
        len_m,
        sections.len(),
        if skipped > 0 { format!(" ({skipped} zonder ontwerphoogte)") } else { String::new() },
        nap(zlo, 2),
        nap(zhi, 2),
        super::terrein::m3(cut),
        super::terrein::m3(fill),
        if fill >= cut { "+" } else { "-" },
        super::terrein::m3((fill - cut).abs()),
        if view_rows > 0 { format!(", dwarsprofielen in {view_rows} rijen") } else { String::new() },
        handles.len(),
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

    #[test]
    fn a_road_in_fill_and_cut() {
        let asm = Assembly::template("weg").unwrap();
        assert!((asm.half_width() - 4.75).abs() < 1e-9);
        // Flat ground at 0, road at +1: fill with 1:3 slopes to the toe.
        let mut flat = |_: f64| Some(0.0);
        let s = section(&asm, 0.0, 1.0, None, &mut flat);
        let edge_z = 1.0 - 0.025 * 3.25 - 0.04 * 1.5;
        let toe = 4.75 + edge_z * 3.0;
        assert!((s.pts[0].0 + toe).abs() < 0.01 && (s.pts.last().unwrap().0 - toe).abs() < 0.01, "{:?}", s.pts);
        assert_eq!(s.fill_side, [Some(true), Some(true)]);
        // Area: the trapezoid of the road body (axis to edge) plus the slopes.
        let expected = 2.0 * ((1.0 + (1.0 - 0.025 * 3.25)) / 2.0 * 3.25 + ((1.0 - 0.025 * 3.25) + edge_z) / 2.0 * 1.5 + edge_z * edge_z * 3.0 / 2.0);
        assert!((s.fill - expected).abs() < 0.02, "{} vs {expected}", s.fill);
        assert!(s.cut < 1e-9);
        // Road at −1: cut with 1:2 slopes.
        let s = section(&asm, 0.0, -1.0, None, &mut flat);
        assert_eq!(s.fill_side, [Some(false), Some(false)]);
        assert!(s.fill < 1e-9 && s.cut > 10.0);
    }

    #[test]
    fn superelevation_tilts_towards_the_inside() {
        let asm = Assembly::template("weg").unwrap();
        // In a left curve: left lane down 4 %, right lane up 4 %.
        let sl = lane_slopes(&asm, Some(1.0), -0.025);
        assert!((sl[0] + 0.04).abs() < 1e-12 && (sl[1] - 0.04).abs() < 1e-12);
        // Halfway the runoff: halfway between crown and full.
        let sl = lane_slopes(&asm, Some(-0.5), -0.025);
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
        let mk = |s: f64, cut: f64, fill: f64| Section { s, z_axis: 0.0, pts: vec![], fill_side: [None, None], cut, fill };
        let v = volumes(&[mk(0.0, 0.0, 10.0), mk(20.0, 4.0, 6.0), mk(40.0, 8.0, 0.0)]);
        assert_eq!(v[1], (40.0, 160.0, 120.0));
        assert_eq!(v[2], (120.0, 60.0, 60.0));
    }
}
