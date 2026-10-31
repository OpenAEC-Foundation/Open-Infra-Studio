//! Junctions and roundabouts between corridors.
//!
//! A junction (`CIV_KRUISPUNT`) joins a main road and a side road, each an
//! axis with a design profile and an assembly. It finds where the axes
//! cross, makes a T or X junction, rounds each corner with a radius between
//! the carriageway edges (kerb returns), and builds the junction surface:
//! the main road keeps its cross section, the side road's profile is tied
//! in to the main road's edge (its grade and cross slope warp between the
//! main road and the mouth of the arm), and the corners follow their kerb
//! returns. The surface is a TIN patch on OG-TRIANGULATIE (status N) whose
//! edges at the arm mouths are the corridors' own sections, so corridors
//! built from the mouth stations on (`CIV_CORRIDOR VAN=` / `TOT=`) fit.
//!
//! A roundabout (`CIV_ROTONDE`) is a single-lane roundabout with the
//! dimensions of CROW publication 126 ("Eenheid in rotondes"): outer and
//! inner radius, an overrunnable strip around the central island, and three
//! or more arms on given axes, each with an entry and an exit lane, entry
//! and exit radii and a splitter island. Its carriageway falls outward; the
//! arms tie in to their road at the mouth.
//!
//! Both draw 2D plan lines on NLCS layers and the 3D surface.

use std::f64::consts::{PI, TAU};

use ocs_plugin_api::host::codec::entities::Polyline3D;
use ocs_plugin_api::host::codec::{CadDocument, Color, EntityType, Handle, LwPolyline, Vector2};
use ocs_plugin_api::host::ExtendedDataRecord;

use super::as_profiel::{self, Alignment, Pvi};
use super::corridor::{self, nlcs_id, nlcs_layer, num, trf, Assembly};
use super::model3d::MeshBuilder;
use super::terrein::{Tin, P3};
use super::{record, station, v3, Ctx, Out};
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::{Element, LayerSpec, Status};

fn sub(a: XY, b: XY) -> XY {
    [a[0] - b[0], a[1] - b[1]]
}

fn dist(a: XY, b: XY) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn ang(a: XY) -> f64 {
    a[1].atan2(a[0])
}

/// An angle difference in (−π, π].
fn wrap(a: f64) -> f64 {
    let a = (a + PI).rem_euclid(TAU) - PI;
    if a <= -PI { a + TAU } else { a }
}

/// The point at distance `s` along the axis and offset `o` (right > 0),
/// both in drawing units.
pub fn offset_point(al: &Alignment, s: f64, o: f64) -> XY {
    let (p, d) = al.at(s);
    [p[0] + d.sin() * o, p[1] - d.cos() * o]
}

/// The axis at offset `o` between distances `s0` and `s1`, every `step`
/// (in the order given): (distance, point).
fn trace(al: &Alignment, o: f64, s0: f64, s1: f64, step: f64) -> Vec<(f64, XY)> {
    let n = ((s1 - s0).abs() / step).ceil().max(1.0) as usize;
    (0..=n).map(|i| s0 + (s1 - s0) * i as f64 / n as f64).map(|s| (s, offset_point(al, s, o))).collect()
}

/// The axis as points every `step`, extended straight by `ext` at both ends.
fn axis_points(al: &Alignment, step: f64, ext: f64) -> Vec<(f64, XY)> {
    let l = al.length();
    let mut v = Vec::new();
    if ext > 0.0 {
        let (p, d) = al.at(0.0);
        v.push((-ext, [p[0] - d.cos() * ext, p[1] - d.sin() * ext]));
    }
    v.extend(trace(al, 0.0, 0.0, l, step));
    if ext > 0.0 {
        let (p, d) = al.at(l);
        v.push((l + ext, [p[0] + d.cos() * ext, p[1] + d.sin() * ext]));
    }
    v
}

/// Where segments a and b cross: the fractions along each.
fn seg_cross(a0: XY, a1: XY, b0: XY, b1: XY) -> Option<(f64, f64)> {
    let (r, s) = (sub(a1, a0), sub(b1, b0));
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let q = sub(b0, a0);
    let t = (q[0] * s[1] - q[1] * s[0]) / den;
    let u = (q[0] * r[1] - q[1] * r[0]) / den;
    ((-1e-9..=1.0 + 1e-9).contains(&t) && (-1e-9..=1.0 + 1e-9).contains(&u)).then_some((t, u))
}

/// Where two traced lines cross: the distances along each and the point.
fn crossings(a: &[(f64, XY)], b: &[(f64, XY)]) -> Vec<(f64, f64, XY)> {
    let bb = |v: &[(f64, XY)]| v.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p.1[0]), b[1].min(p.1[1]), b[2].max(p.1[0]), b[3].max(p.1[1])]);
    let mut out = Vec::new();
    for wa in a.windows(2) {
        let ba = bb(wa);
        for wb in b.windows(2) {
            let bo = bb(wb);
            if ba[0] > bo[2] || bo[0] > ba[2] || ba[1] > bo[3] || bo[1] > ba[3] {
                continue;
            }
            if let Some((t, u)) = seg_cross(wa[0].1, wa[1].1, wb[0].1, wb[1].1) {
                let p = [wa[0].1[0] + (wa[1].1[0] - wa[0].1[0]) * t, wa[0].1[1] + (wa[1].1[1] - wa[0].1[1]) * t];
                out.push((wa[0].0 + (wa[1].0 - wa[0].0) * t, wb[0].0 + (wb[1].0 - wb[0].0) * u, p));
            }
        }
    }
    out
}

/// Whether `p` lies inside the rings (even-odd: holes are rings too).
pub fn inside(rings: &[Vec<XY>], p: XY) -> bool {
    let mut c = false;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) {
                c = !c;
            }
        }
    }
    c
}

/// Points on an arc around `c` from angle `a0` over the signed `sweep`,
/// at most `step` apart, both ends included.
fn arc_points(c: XY, r: f64, a0: f64, sweep: f64, step: f64) -> Vec<XY> {
    let n = ((r * sweep.abs()) / step).ceil().max(1.0) as usize;
    (0..=n).map(|i| a0 + sweep * i as f64 / n as f64).map(|a| [c[0] + r * a.cos(), c[1] + r * a.sin()]).collect()
}

/// A TIN patch: Delaunay over the points, the triangles whose centre lies
/// inside the rings. Points in drawing units, heights in drawing units.
pub fn patch(points: &[P3], rings: &[Vec<XY>]) -> Result<Vec<[P3; 3]>, String> {
    let tin = Tin::build(points, 0.0)?;
    Ok(tin
        .triangles
        .iter()
        .map(|t| t.map(|i| tin.points[i]))
        .filter(|t| {
            let c = [(t[0][0] + t[1][0] + t[2][0]) / 3.0, (t[0][1] + t[1][1] + t[2][1]) / 3.0];
            inside(rings, c)
        })
        .collect())
}

/// The carriageway's cross profile of an assembly: (offset m, height m
/// relative to the axis), from the left edge to the right one.
pub fn carriageway_profile(asm: &Assembly) -> Vec<(f64, f64)> {
    let (l, r) = (asm.edge(0).0, asm.edge(1).0);
    let mut none = |_: f64| None;
    let s = corridor::section(asm, 0.0, 0.0, None, &mut none);
    let mut v: Vec<(f64, f64)> = s.pts.iter().filter(|p| p.o >= -l - 1e-9 && p.o <= r + 1e-9).map(|p| (p.o, p.z)).collect();
    v.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
    v
}

/// Height in a profile at an offset, flat beyond its ends.
fn profile_z(p: &[(f64, f64)], o: f64) -> f64 {
    if p.is_empty() {
        return 0.0;
    }
    if o <= p[0].0 {
        return p[0].1;
    }
    for w in p.windows(2) {
        if o <= w[1].0 {
            let t = if (w[1].0 - w[0].0).abs() < 1e-12 { 0.0 } else { (o - w[0].0) / (w[1].0 - w[0].0) };
            return w[0].1 + (w[1].1 - w[0].1) * t;
        }
    }
    p[p.len() - 1].1
}

/// One road at a junction or roundabout.
pub struct Road {
    pub al: Alignment,
    pub asm: Assembly,
    /// The design profile; None: level with the junction.
    pub pvis: Option<Vec<Pvi>>,
}

impl Road {
    /// Carriageway edge on the alignment's left (sign < 0) or right side (drawing units).
    fn edge(&self, sign: f64, f: f64) -> f64 {
        self.asm.edge(if sign < 0.0 { 0 } else { 1 }).0 * f
    }

    fn kerb(&self, sign: f64) -> Option<&corridor::Part> {
        self.asm.edge(if sign < 0.0 { 0 } else { 1 }).1
    }

    fn design(&self, s: f64, f: f64, level: f64) -> f64 {
        match &self.pvis {
            Some(p) => as_profiel::design_z(p, s / f),
            None => level,
        }
    }
}

// ── Junction ────────────────────────────────────────────────────────────────

/// An arm: a road leaving the junction in one direction.
#[derive(Clone, Debug)]
pub struct Arm {
    /// 0 main road, 1 side road.
    pub road: usize,
    /// +1 towards increasing stations, −1 back.
    pub dir: f64,
    pub angle: f64,
    /// Length available from the junction (drawing units).
    pub avail: f64,
    /// Distance from the junction to the arm's mouth (drawing units).
    pub mouth: f64,
}

/// A rounded corner between an arm's left edge and the next arm's right edge.
#[derive(Clone, Debug)]
pub struct Fillet {
    pub r: f64,
    pub c: XY,
    /// Tangent points on the first and the second arm's edge.
    pub t: [XY; 2],
    /// Their distances from the junction along each arm.
    pub d: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct JunctionPlan {
    pub x: XY,
    /// Distances of the junction along the main and the side axis.
    pub s: [f64; 2],
    /// Counter-clockwise, the main road's forward arm first.
    pub arms: Vec<Arm>,
    /// Corner i lies between arm i and arm i+1; None where one road runs on.
    pub corners: Vec<Option<Fillet>>,
    /// The paved area, counter-clockwise.
    pub boundary: Vec<XY>,
    pub kind: &'static str,
}

/// The junction's layout. `radii`: corner radii in drawing units, in the
/// order of the rounded corners (the last repeats).
pub fn plan_junction(roads: &[Road; 2], radii: &[f64], f: f64) -> Result<JunctionPlan, String> {
    let step = 0.5 * f;
    let main = axis_points(&roads[0].al, step, 0.0);
    let side = axis_points(&roads[1].al, step, 60.0 * f);
    let xs = crossings(&main, &side);
    let (sm, ss, x) = *xs
        .iter()
        .min_by(|a, b| {
            // Nearest the side road's end that is closest to the main road.
            let l = roads[1].al.length();
            let key = |c: &(f64, f64, XY)| c.1.min((l - c.1).abs()).abs();
            key(a).total_cmp(&key(b))
        })
        .ok_or_else(|| tr("de assen kruisen elkaar niet (ook niet verlengd)", "the axes do not cross (not even extended)").to_string())?;
    let s = [sm.clamp(0.0, roads[0].al.length()), ss];
    let edge_max = |r: &Road| r.asm.edge(0).0.max(r.asm.edge(1).0) * f;
    let mut arms = Vec::new();
    for (ri, road) in roads.iter().enumerate() {
        let l = road.al.length();
        let sx = s[ri].clamp(0.0, l);
        let need = edge_max(&roads[1 - ri]) + 5.0 * f;
        for dir in [1.0, -1.0] {
            let avail = if dir > 0.0 { l - sx } else { sx };
            if avail > need {
                let p = road.al.at(sx + dir * need).0;
                arms.push(Arm { road: ri, dir, angle: ang(sub(p, x)), avail, mouth: 0.0 });
            }
        }
    }
    if arms.iter().filter(|a| a.road == 0).count() < 2 {
        return Err(tr("de hoofdweg moet door het kruispunt lopen", "the main road must run through the junction").into());
    }
    if !arms.iter().any(|a| a.road == 1) {
        return Err(tr("de zijweg sluit niet aan: hij is te kort naast de hoofdweg", "the side road does not connect: it is too short beside the main road").into());
    }
    arms.sort_by(|a, b| a.angle.total_cmp(&b.angle));
    // The main road's forward arm first.
    let first = arms.iter().position(|a| a.road == 0 && a.dir > 0.0).unwrap_or(0);
    arms.rotate_left(first);
    let n = arms.len();
    let kind = if arms.iter().filter(|a| a.road == 1).count() >= 2 { "X" } else { "T" };
    let mut corners = Vec::with_capacity(n);
    let mut k = 0;
    for i in 0..n {
        let (a, b) = (&arms[i], &arms[(i + 1) % n]);
        if a.road == b.road {
            corners.push(None);
            continue;
        }
        let r = radii.get(k).or(radii.last()).copied().unwrap_or(8.0 * f).max(0.5 * f);
        k += 1;
        let (ra, rb) = (&roads[a.road], &roads[b.road]);
        // Arm a's left side, arm b's right side (looking outward).
        let (sa, sb) = (-a.dir, b.dir);
        let (ea, eb) = (ra.edge(sa, f), rb.edge(sb, f));
        let (xa, xb) = (s[a.road].clamp(0.0, ra.al.length()), s[b.road].clamp(0.0, rb.al.length()));
        let reach = |arm: &Arm| arm.avail.min(4.0 * r + 60.0 * f);
        let ta = trace(&ra.al, sa * (ea + r), xa, xa + a.dir * reach(a), step / 2.0);
        let tb = trace(&rb.al, sb * (eb + r), xb, xb + b.dir * reach(b), step / 2.0);
        let c = crossings(&ta, &tb).into_iter().min_by(|p, q| dist(p.2, x).total_cmp(&dist(q.2, x))).ok_or_else(|| {
            trf!("hoek {}: de boogstraal {} m past niet tussen de wegranden", "corner {}: the radius {} m does not fit between the road edges", k, num(r / f, 1))
        })?;
        let t = [offset_point(&ra.al, c.0, sa * ea), offset_point(&rb.al, c.1, sb * eb)];
        corners.push(Some(Fillet { r, c: c.2, t, d: [(c.0 - xa).abs(), (c.1 - xb).abs()] }));
    }
    // Mouths: past the farthest tangent point of the arm.
    let margin = 2.0 * f;
    for i in 0..n {
        let mut d: f64 = edge_max(&roads[1 - arms[i].road]) + margin;
        if let Some(c) = &corners[i] {
            d = d.max(c.d[0] + margin);
        }
        if let Some(c) = &corners[(i + n - 1) % n] {
            d = d.max(c.d[1] + margin);
        }
        if d > arms[i].avail {
            return Err(trf!("arm {} is te kort voor de hoeken ({} m nodig)", "arm {} is too short for the corners ({} m needed)", i + 1, num(d / f, 1)));
        }
        arms[i].mouth = d;
    }
    // The paved area: out along each arm's right edge, across its mouth,
    // back along its left edge, round the corner to the next arm.
    let mut boundary = Vec::new();
    for i in 0..n {
        let a = &arms[i];
        let road = &roads[a.road];
        let sx = s[a.road].clamp(0.0, road.al.length());
        let (er, el) = (road.edge(a.dir, f), road.edge(-a.dir, f));
        let d0 = corners[(i + n - 1) % n].as_ref().map(|c| c.d[1]).unwrap_or(0.0);
        let d1 = corners[i].as_ref().map(|c| c.d[0]).unwrap_or(0.0);
        boundary.extend(trace(&road.al, a.dir * er, sx + a.dir * d0, sx + a.dir * a.mouth, step).into_iter().map(|p| p.1));
        boundary.extend(trace(&road.al, -a.dir * el, sx + a.dir * a.mouth, sx + a.dir * d1, step).into_iter().map(|p| p.1));
        if let Some(c) = &corners[i] {
            let (a0, a1) = (ang(sub(c.t[0], c.c)), ang(sub(c.t[1], c.c)));
            let pts = arc_points(c.c, c.r, a0, wrap(a1 - a0), step);
            boundary.extend(pts[1..pts.len() - 1].iter().copied());
        }
    }
    boundary.dedup_by(|a, b| dist(*a, *b) < 1e-6);
    Ok(JunctionPlan { x, s, arms, corners, boundary, kind })
}

/// Heights in a junction: the main road's surface on its carriageway, the
/// side road warped from the main road's edge to its own section at the
/// mouth.
struct JunctionSurface<'a> {
    roads: &'a [Road; 2],
    plan: &'a JunctionPlan,
    profiles: [Vec<(f64, f64)>; 2],
    f: f64,
    /// Level for roads without a design profile (m).
    level: f64,
}

impl JunctionSurface<'_> {
    fn main_z(&self, p: XY) -> f64 {
        let (s, o) = self.roads[0].al.station_offset(p);
        self.roads[0].design(s, self.f, self.level) + profile_z(&self.profiles[0], o / self.f)
    }

    /// On the main road's carriageway between its mouths.
    fn on_main(&self, p: XY) -> bool {
        let f = self.f;
        let (s, o) = self.roads[0].al.station_offset(p);
        let e = self.roads[0].edge(o, f);
        let reach: Vec<f64> = self.plan.arms.iter().filter(|a| a.road == 0).map(|a| self.plan.s[0] + a.dir * a.mouth).collect();
        let (lo, hi) = reach.iter().fold((f64::MAX, f64::MIN), |r, v| (r.0.min(*v), r.1.max(*v)));
        o.abs() <= e + 1e-6 * f && s >= lo - 1e-6 && s <= hi + 1e-6
    }

    /// The side arm a point belongs to: the nearest in angle.
    fn side_arm(&self, p: XY) -> Option<&Arm> {
        let a = ang(sub(p, self.plan.x));
        self.plan.arms.iter().filter(|r| r.road == 1).min_by(|x, y| wrap(x.angle - a).abs().total_cmp(&wrap(y.angle - a).abs()))
    }

    /// Where the side road at offset `o` (drawing units) meets the main
    /// road's edge on the arm's side: the distance along the arm.
    fn tie_distance(&self, arm: &Arm, o: f64) -> f64 {
        let (main, side) = (&self.roads[0], &self.roads[1]);
        let f = self.f;
        let sx = self.plan.s[1];
        let probe = offset_point(&side.al, sx + arm.dir * arm.mouth, 0.0);
        let sign = main.al.station_offset(probe).1.signum();
        let e = main.edge(sign, f);
        let sm = self.plan.s[0];
        let span = arm.mouth + 30.0 * f;
        let edge = trace(&main.al, sign * e, (sm - span).max(0.0), (sm + span).min(main.al.length()), 0.25 * f);
        let line = trace(&side.al, o, sx, sx + arm.dir * arm.mouth, 0.25 * f);
        crossings(&line, &edge).into_iter().map(|c| (c.0 - sx).abs()).min_by(f64::total_cmp).unwrap_or(e)
    }

    fn side_z(&self, arm: &Arm, p: XY) -> f64 {
        let side = &self.roads[1];
        let f = self.f;
        let (s, o) = side.al.station_offset(p);
        let d = (s - self.plan.s[1]) * arm.dir;
        let de = self.tie_distance(arm, o);
        let tie = self.main_z(offset_point(&side.al, self.plan.s[1] + arm.dir * de, o));
        let sm = self.plan.s[1] + arm.dir * arm.mouth;
        let mouth = side.design(sm, f, self.level) + profile_z(&self.profiles[1], o / f);
        let t = ((d - de) / (arm.mouth - de).max(1e-9)).clamp(0.0, 1.0);
        tie + (mouth - tie) * t
    }

    fn z(&self, p: XY) -> f64 {
        if self.on_main(p) {
            return self.main_z(p);
        }
        match self.side_arm(p) {
            Some(arm) => self.side_z(arm, p),
            None => self.main_z(p),
        }
    }
}

/// The natural colour of a VH surface, or None (by layer).
fn natural(on: bool, vh: &str) -> Option<Color> {
    on.then(|| {
        let (r, g, b) = corridor::surface_colour(nlcs_id("VH", vh));
        Color::Rgb { r, g, b }
    })
}

/// A lightweight polyline with bulges (arcs) on a layer.
fn lw(pts: &[(XY, f64)], closed: bool, spec: &LayerSpec, k: f64, data: Option<&ExtendedDataRecord>) -> EntityType {
    let mut l = LwPolyline::new();
    for (p, b) in pts {
        l.add_point_with_bulge(Vector2::new(p[0], p[1]), *b);
    }
    l.is_closed = closed;
    let mut e = EntityType::LwPolyline(l);
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = Color::ByLayer;
    c.linetype_scale = k;
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

fn poly3(pts: &[P3], spec: &LayerSpec, k: f64) -> EntityType {
    let mut e = EntityType::Polyline3D(Polyline3D::from_points(pts.iter().map(|p| v3([p[0], p[1]], p[2])).collect()));
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = Color::ByLayer;
    c.linetype_scale = k;
    e
}

/// Give-way triangles ("haaientanden") along a line from `a` to `b`,
/// pointing towards `toward` (a unit vector), sizes in drawing units.
fn shark_teeth(a: XY, b: XY, toward: XY, base: f64, height: f64, gap: f64) -> Vec<Vec<XY>> {
    let l = dist(a, b);
    let u = [(b[0] - a[0]) / l.max(1e-9), (b[1] - a[1]) / l.max(1e-9)];
    let n = ((l + gap) / (base + gap)).floor() as usize;
    let start = (l - (n as f64 * (base + gap) - gap)) / 2.0;
    (0..n)
        .map(|i| {
            let t0 = start + i as f64 * (base + gap);
            let p0 = [a[0] + u[0] * t0, a[1] + u[1] * t0];
            let p1 = [p0[0] + u[0] * base, p0[1] + u[1] * base];
            let tip = [(p0[0] + p1[0]) / 2.0 + toward[0] * height, (p0[1] + p1[1]) / 2.0 + toward[1] * height];
            vec![p0, p1, tip]
        })
        .collect()
}

/// The axes a junction or roundabout uses, read from the drawing.
fn road_of(doc: &CadDocument, h: Handle, asm: Option<&str>) -> Result<Road, String> {
    let (al, _, _) = as_profiel::axis(doc, Some(h))?;
    let name = asm.map(str::to_string).or_else(|| corridor::assembly_of(doc, &al.name)).unwrap_or_else(|| "weg".into());
    let asm = super::profieltype::find(&name)?;
    let pvis = corridor::profile_of(doc, &al.name);
    Ok(Road { al, asm, pvis })
}

fn axis_label(al: &Alignment) -> String {
    if al.name.is_empty() { tr("as", "axis").to_string() } else { al.name.clone() }
}

pub struct JunctionRequest {
    pub main: Handle,
    pub side: Handle,
    pub main_asm: Option<String>,
    pub side_asm: Option<String>,
    /// Corner radii (m); empty: the default.
    pub radii: Vec<f64>,
    /// Level for roads without a design profile (m).
    pub level: Option<f64>,
    pub name: Option<String>,
    /// From a selection: the side road is the axis that ends nearest the
    /// crossing (the main road runs through).
    pub auto_order: bool,
    /// The surface in natural colours (asphalt), else by layer.
    pub natural: bool,
}

/// The default corner radius (m): an estimate for a junction of two
/// distributor roads in a built-up area; see docs/civiel-corridor.md.
pub const CORNER_RADIUS: f64 = 8.0;

/// `CIV_KRUISPUNT`.
pub fn junction(opt: &super::CivielOptions, req: JunctionRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let mut opt = opt.clone();
    opt.nlcs.status = Status::N;
    let ctx = Ctx::new(&opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["ontwerpprofiel", "corridor"], &[req.main, req.side])?;
    let mut roads = [road_of(&doc, req.main, req.main_asm.as_deref())?, road_of(&doc, req.side, req.side_asm.as_deref())?];
    if req.auto_order {
        let xs = crossings(&axis_points(&roads[0].al, 0.5 * f, 60.0 * f), &axis_points(&roads[1].al, 0.5 * f, 60.0 * f));
        if let Some(x) = xs.first() {
            let end = |s: f64, l: f64| s.abs().min((l - s).abs());
            let (e0, e1) = (end(x.0, roads[0].al.length()), end(x.1, roads[1].al.length()));
            let swap = if (e0 - e1).abs() < 5.0 * f { roads[1].al.length() > roads[0].al.length() } else { e0 < e1 };
            if swap {
                roads.swap(0, 1);
            }
        }
    }
    if roads[0].pvis.is_none() && req.level.is_none() {
        return Err(trf!(
            "{} heeft geen ontwerpprofiel: maak er een (CIV_ONTWERPPROFIEL) of geef NIVEAU=…",
            "{} has no design profile: make one (CIV_ONTWERPPROFIEL) or give NIVEAU=…",
            axis_label(&roads[0].al)
        ));
    }
    progress(tr("Kruispunt: geometrie…", "Junction: geometry…").into());
    let radii: Vec<f64> = if req.radii.is_empty() { vec![CORNER_RADIUS * f] } else { req.radii.iter().map(|r| r * f).collect() };
    let plan = plan_junction(&roads, &radii, f)?;
    let level0 = req.level.unwrap_or_else(|| roads[0].design(plan.s[0], f, 0.0));
    let surf = JunctionSurface { roads: &roads, plan: &plan, profiles: [carriageway_profile(&roads[0].asm), carriageway_profile(&roads[1].asm)], f, level: level0 };
    // Points: the boundary (corners interpolated along their arcs), and a
    // grid over the main road and each side arm.
    progress(tr("Kruispunt: oppervlak…", "Junction: surface…").into());
    let n = plan.arms.len();
    let mut pts: Vec<P3> = Vec::new();
    let mut returns: Vec<(Vec<P3>, f64)> = Vec::new();
    for (i, c) in plan.corners.iter().enumerate() {
        let Some(c) = c else { continue };
        let (z0, z1) = (surf.z(c.t[0]), surf.z(c.t[1]));
        let (a0, a1) = (ang(sub(c.t[0], c.c)), ang(sub(c.t[1], c.c)));
        let arc = arc_points(c.c, c.r, a0, wrap(a1 - a0), 0.5 * f);
        let m = arc.len() - 1;
        let line: Vec<P3> = arc.iter().enumerate().map(|(j, p)| [p[0], p[1], (z0 + (z1 - z0) * j as f64 / m as f64) * f]).collect();
        pts.extend(line.iter().copied());
        let arms = (&plan.arms[i], &plan.arms[(i + 1) % n]);
        let upstand = roads[arms.0.road].kerb(-arms.0.dir).or(roads[arms.1.road].kerb(arms.1.dir)).map(|p| p.upstand).unwrap_or(0.0);
        returns.push((line, upstand));
    }
    let on_arc = |p: XY| plan.corners.iter().flatten().any(|c| (dist(p, c.c) - c.r).abs() < 1e-6 * f);
    for p in &plan.boundary {
        if !on_arc(*p) {
            pts.push([p[0], p[1], surf.z(*p) * f]);
        }
    }
    let grid = 1.0 * f;
    for arm in plan.arms.iter() {
        let road = &roads[arm.road];
        let sx = plan.s[arm.road].clamp(0.0, road.al.length());
        let prof = &surf.profiles[arm.road];
        let (lo, hi) = (prof.first().map(|p| p.0).unwrap_or(0.0) * f, prof.last().map(|p| p.0).unwrap_or(0.0) * f);
        let mut offs: Vec<f64> = prof.iter().map(|p| p.0 * f).collect();
        let mut o = lo;
        while o < hi {
            offs.push(o);
            o += grid;
        }
        let start = if arm.road == 1 { surf.tie_distance(arm, 0.0) - grid } else { 0.0 };
        let mut d = start.max(0.0);
        while d <= arm.mouth + 1e-9 {
            for o in &offs {
                let p = offset_point(&road.al, sx + arm.dir * d, *o);
                if arm.road == 1 && surf.on_main(p) {
                    continue;
                }
                pts.push([p[0], p[1], surf.z(p) * f]);
            }
            d += grid;
        }
    }
    let tris = patch(&pts, std::slice::from_ref(&plan.boundary))?;
    if tris.is_empty() {
        return Err(tr("het kruispuntvlak is leeg", "the junction surface is empty").into());
    }
    // Drawing.
    let name = req.name.unwrap_or_else(|| format!("{} {} / {}", tr("Kruispunt", "Junction"), axis_label(&roads[0].al), axis_label(&roads[1].al)));
    let data = record(&[("kruispunt", name.clone()), ("hoofdweg", roads[0].al.name.clone()), ("zijweg", roads[1].al.name.clone()), ("type", plan.kind.into())]);
    let mut out = Out::default();
    let faces = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let mut mesh = MeshBuilder::default();
    for t in &tris {
        mesh.tri(t[0], t[1], t[2]);
    }
    let face_count = mesh.faces();
    let meshes = mesh.finish(&faces.name, natural(req.natural, "GESLOTENVERHARDING_ASFALT"), Some(&data));
    let mesh_count = meshes.len();
    out.put(&faces, meshes);
    // Kerb returns and the arms' edges, in plan.
    let edge_layer = |road: &Road, sign: f64| -> Result<LayerSpec, String> {
        match road.kerb(sign) {
            Some(kb) => nlcs_layer(&ctx, kb.surface, Element::G),
            None => {
                let id = road.asm.sides[if sign < 0.0 { 0 } else { 1 }].iter().rev().find(|p| p.kind.carriageway()).map(|p| p.surface).unwrap_or(nlcs_id("VH", "GESLOTENVERHARDING_ASFALT"));
                nlcs_layer(&ctx, id, Element::G)
            }
        }
    };
    let mut corner_data = Vec::new();
    let mut rc = 0;
    for (i, c) in plan.corners.iter().enumerate() {
        let Some(c) = c else { continue };
        let (a, b) = (&plan.arms[i], &plan.arms[(i + 1) % n]);
        let (ra, rb) = (&roads[a.road], &roads[b.road]);
        let spec = edge_layer(ra, -a.dir)?;
        let (a0, a1) = (ang(sub(c.t[0], c.c)), ang(sub(c.t[1], c.c)));
        let sweep = wrap(a1 - a0);
        out.put(&spec, [lw(&[(c.t[0], (sweep / 4.0).tan()), (c.t[1], 0.0)], false, &spec, k, Some(&data))]);
        let kerb_w = ra.kerb(-a.dir).or(rb.kerb(b.dir)).map(|p| p.width * f);
        if let Some(w) = kerb_w {
            let r2 = c.r + w;
            let p0 = [c.c[0] + r2 * a0.cos(), c.c[1] + r2 * a0.sin()];
            let p1 = [c.c[0] + r2 * a1.cos(), c.c[1] + r2 * a1.sin()];
            out.put(&spec, [lw(&[(p0, (sweep / 4.0).tan()), (p1, 0.0)], false, &spec, k, Some(&data))]);
        }
        // In 3D: the kerb return's foot and top.
        if let Some((line, up)) = returns.get(rc) {
            out.put(&spec, [poly3(line, &spec, k)]);
            if *up > 0.0 {
                let top: Vec<P3> = line.iter().map(|p| [p[0], p[1], p[2] + up * f]).collect();
                out.put(&spec, [poly3(&top, &spec, k)]);
            }
        }
        rc += 1;
        let cp = ctx.from_drawing(c.c);
        corner_data.push(serde_json::json!({"hoek": rc, "straal_m": c.r / f, "middelpunt": [cp[0], cp[1]], "armen": [i + 1, (i + 1) % n + 1]}));
    }
    // The arms' edges from the corner to the mouth; the straight edge of a T.
    for (i, a) in plan.arms.iter().enumerate() {
        let road = &roads[a.road];
        let sx = plan.s[a.road].clamp(0.0, road.al.length());
        for sign in [a.dir, -a.dir] {
            let corner = if sign == a.dir { plan.corners[(i + n - 1) % n].as_ref().map(|c| c.d[1]) } else { plan.corners[i].as_ref().map(|c| c.d[0]) };
            let d0 = corner.unwrap_or(0.0);
            let pts: Vec<(XY, f64)> = trace(&road.al, sign * road.edge(sign, f), sx + a.dir * d0, sx + a.dir * a.mouth, 0.5 * f).into_iter().map(|p| (p.1, 0.0)).collect();
            if pts.len() > 1 {
                let spec = edge_layer(road, sign)?;
                out.put(&spec, [lw(&pts, false, &spec, k, Some(&data))]);
            }
        }
    }
    // Give way on the side road: shark teeth across its approach lane.
    let teeth_l = ctx.layer("VW", "MARKERING_DWARS_DRIEHOEK", Element::G)?;
    let mut teeth = 0;
    for a in plan.arms.iter().filter(|a| a.road == 1) {
        let side = &roads[1];
        let de = surf.tie_distance(a, 0.0) + 0.3 * f;
        let s = plan.s[1] + a.dir * de;
        // The approach lane: the arm's left side looking outward.
        let el = side.edge(-a.dir, f);
        let (p0, p1) = (offset_point(&side.al, s, -a.dir * 0.1 * f), offset_point(&side.al, s, -a.dir * (el - 0.3 * f).max(0.5 * f)));
        let (_, dir) = side.al.at(s);
        let out_dir = [dir.cos() * a.dir, dir.sin() * a.dir];
        for t in shark_teeth(p0, p1, out_dir, 0.5 * f, 0.6 * f, 0.25 * f) {
            out.put(&teeth_l, [lw(&t.iter().map(|p| (*p, 0.0)).collect::<Vec<_>>(), true, &teeth_l, k, Some(&data))]);
            teeth += 1;
        }
    }
    let handles = out.write_handles(&name)?;
    crate::bridge::index_add("tin", "N", &handles[..mesh_count.min(handles.len())], false);
    crate::bridge::index_add("kruispunt", &name, &handles[..1.min(handles.len())], true);
    let xp = ctx.from_drawing(plan.x);
    let arms: Vec<serde_json::Value> = plan
        .arms
        .iter()
        .map(|a| {
            let road = &roads[a.road];
            let sm = road.al.start + (plan.s[a.road] + a.dir * a.mouth) / f;
            serde_json::json!({
                "as": road.al.name, "weg": if a.road == 0 { "hoofdweg" } else { "zijweg" }, "richting": if a.dir > 0.0 { "+" } else { "-" },
                "station_mond": sm, "corridor": format!("{}={:.2}", if a.dir > 0.0 { "VAN" } else { "TOT" }, sm),
            })
        })
        .collect();
    let zx = surf.z(plan.x);
    super::api::note("kruispunt", name.clone());
    super::api::note("type", plan.kind);
    super::api::note("punt", serde_json::json!([xp[0], xp[1]]));
    super::api::note("station_hoofdweg", roads[0].al.start + plan.s[0] / f);
    super::api::note("station_zijweg", roads[1].al.start + plan.s[1] / f);
    super::api::note("hoogte", zx);
    super::api::note("armen", arms);
    super::api::note("hoeken", corner_data);
    super::api::note("driehoeken", face_count);
    super::api::note("haaientanden", teeth);
    super::api::note("profieltypen", serde_json::json!([roads[0].asm.name, roads[1].asm.name]));
    let mouths: Vec<String> = plan.arms.iter().map(|a| station(roads[a.road].al.start + (plan.s[a.road] + a.dir * a.mouth) / f)).collect();
    Ok(trf!(
        "{} ({}-kruispunt, {} armen, {} hoeken R {} m): {} driehoeken op {} m NAP; armen eindigen op {} — bouw de corridors vanaf daar (VAN=/TOT=). {} elementen.",
        "{} ({} junction, {} arms, {} corners R {} m): {} triangles at {} m; the arms end at {} — build the corridors from there (VAN=/TOT=). {} entities.",
        name,
        plan.kind,
        n,
        rc,
        radii.iter().map(|r| num(r / f, 1)).collect::<Vec<_>>().join("/"),
        face_count,
        num(zx, 2),
        mouths.join(", "),
        handles.len()
    ))
}

// ── Roundabout ──────────────────────────────────────────────────────────────

/// Dimensions of a single-lane roundabout (m, the slope a fraction).
#[derive(Clone, Debug, PartialEq)]
pub struct RoundDims {
    /// Outer radius of the carriageway.
    pub r_out: f64,
    /// Inner radius: the central island with its overrunnable strip.
    pub r_in: f64,
    /// Width of the overrunnable strip.
    pub apron: f64,
    pub entry_w: f64,
    pub exit_w: f64,
    pub r_entry: f64,
    pub r_exit: f64,
    pub split_w: f64,
    pub split_l: f64,
    /// From the splitter's end to the arm's mouth, where the lanes narrow
    /// to the road.
    pub taper: f64,
    /// Outward cross slope of the carriageway.
    pub slope: f64,
    /// Upstand of the overrunnable strip and of the island's kerb.
    pub apron_up: f64,
    pub island_up: f64,
}

impl RoundDims {
    /// CROW publication 126 ("Eenheid in rotondes"), with splitter islands:
    /// within (true) or outside built-up areas. The overrunnable strip is
    /// CROW's 3,00 m for a 22 m design vehicle; cross slope, upstands and
    /// the taper are estimates (docs/civiel-corridor.md).
    pub fn crow(built_up: bool) -> RoundDims {
        let (r_out, r_in, split_w) = if built_up { (16.0, 10.5, 2.5) } else { (18.0, 12.75, 3.0) };
        RoundDims {
            r_out,
            r_in,
            apron: 3.0,
            entry_w: 4.0,
            exit_w: 4.5,
            r_entry: 12.0,
            r_exit: 15.0,
            split_w,
            split_l: 14.0,
            taper: 15.0,
            slope: 0.025,
            apron_up: 0.05,
            island_up: 0.15,
        }
    }
}

/// A rounding between the circle and a straight edge.
#[derive(Clone, Debug)]
pub struct Rounding {
    pub c: XY,
    pub r: f64,
    /// Tangent point on the lane edge, its distance along the arm, and the
    /// tangent point on the circle.
    pub t_edge: XY,
    pub d_edge: f64,
    pub t_circle: XY,
}

#[derive(Clone, Debug)]
pub struct RArm {
    pub road: usize,
    pub dir: f64,
    /// Distance of the centre along the road's axis (drawing units).
    pub s_c: f64,
    pub angle: f64,
    /// Distance from the centre to the mouth.
    pub d_m: f64,
    /// The road's edges at the mouth, right and left looking outward.
    pub e_r: f64,
    pub e_l: f64,
    pub exit: Rounding,
    pub entry: Rounding,
}

#[derive(Clone, Debug)]
pub struct RoundPlan {
    pub c: XY,
    pub dims: RoundDims,
    pub arms: Vec<RArm>,
    /// The paved area's outline (counter-clockwise), and the holes: the
    /// central island with its strip, the splitter islands.
    pub outline: Vec<XY>,
    pub splitters: Vec<Vec<XY>>,
}

impl RArm {
    /// A point at distance `d` along the arm and offset `o` (right > 0
    /// looking outward), drawing units.
    pub fn point(&self, roads: &[Road], d: f64, o: f64) -> XY {
        offset_point(&roads[self.road].al, self.s_c + self.dir * d, self.dir * o)
    }

    /// The outer edges (right > 0, left < 0) at distance `d`.
    fn edges(&self, dims: &RoundDims, f: f64, d: f64) -> (f64, f64) {
        let (o_r, o_l) = ((dims.split_w / 2.0 + dims.exit_w) * f, -(dims.split_w / 2.0 + dims.entry_w) * f);
        let d1 = (dims.r_out + dims.split_l) * f;
        if d <= d1 {
            return (o_r, o_l);
        }
        let t = ((d - d1) / (self.d_m - d1).max(1e-9)).clamp(0.0, 1.0);
        (o_r + (self.e_r - o_r) * t, o_l + (-self.e_l - o_l) * t)
    }
}

/// The rounding between the circle and an edge at offset `o` (drawing units,
/// signed looking outward) with radius `r`: its centre lies at `o ± r`, at
/// `r_out + r` from the roundabout's centre.
fn rounding(arm: &RArm, roads: &[Road], c: XY, r_out: f64, o: f64, r: f64, d_max: f64) -> Option<Rounding> {
    let oc = o + o.signum() * r;
    let g = |d: f64| dist(arm.point(roads, d, oc), c) - (r_out + r);
    let (mut lo, mut hi) = (0.0, d_max);
    if g(lo) > 0.0 || g(hi) < 0.0 {
        return None;
    }
    for _ in 0..60 {
        let m = (lo + hi) / 2.0;
        if g(m) < 0.0 { lo = m } else { hi = m }
    }
    let d = (lo + hi) / 2.0;
    let p = arm.point(roads, d, oc);
    let k = r_out / (r_out + r);
    Some(Rounding { c: p, r, t_edge: arm.point(roads, d, o), d_edge: d, t_circle: [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k] })
}

/// The roundabout's layout.
pub fn plan_roundabout(c: XY, roads: &[Road], dims: &RoundDims, f: f64) -> Result<RoundPlan, String> {
    let need = (dims.r_out + dims.split_l + 2.0) * f;
    let mut arms = Vec::new();
    for (ri, road) in roads.iter().enumerate() {
        let (s_c, off) = road.al.station_offset(c);
        if off.abs() > dims.r_in * f {
            return Err(trf!("{} loopt niet door het midden van de rotonde", "{} does not run through the centre of the roundabout", axis_label(&road.al)));
        }
        let l = road.al.length();
        for dir in [1.0, -1.0] {
            let avail = if dir > 0.0 { l - s_c } else { s_c };
            if avail < (dims.r_out + 5.0) * f {
                continue;
            }
            if avail < need {
                return Err(trf!("een arm van {} is te kort: {} m, nodig {} m", "an arm of {} is too short: {} m, needed {} m", axis_label(&road.al), num(avail / f, 1), num(need / f, 1)));
            }
            let p = road.al.at(s_c + dir * (dims.r_out + 5.0) * f).0;
            let d_m = avail.min((dims.r_out + dims.split_l + dims.taper) * f);
            let (e_r, e_l) = (road.edge(dir, f), road.edge(-dir, f));
            let dummy = Rounding { c, r: 0.0, t_edge: c, d_edge: 0.0, t_circle: c };
            arms.push(RArm { road: ri, dir, s_c, angle: ang(sub(p, c)), d_m, e_r, e_l, exit: dummy.clone(), entry: dummy });
        }
    }
    if arms.len() < 3 || arms.len() > 6 {
        return Err(trf!("een rotonde heeft 3 tot 6 armen; deze assen geven er {}", "a roundabout has 3 to 6 arms; these axes give {}", arms.len()));
    }
    arms.sort_by(|a, b| a.angle.total_cmp(&b.angle));
    let d1 = (dims.r_out + dims.split_l) * f;
    let (o_r, o_l) = ((dims.split_w / 2.0 + dims.exit_w) * f, -(dims.split_w / 2.0 + dims.entry_w) * f);
    for (i, arm) in arms.iter_mut().enumerate() {
        let a = arm.clone();
        let fail = |what: &str| trf!("arm {}: de {}boog past niet voor het einde van de middengeleider", "arm {}: the {} radius does not fit before the end of the splitter island", i + 1, what);
        arm.exit = rounding(&a, roads, c, dims.r_out * f, o_r, dims.r_exit * f, d1).ok_or_else(|| fail(tr("uitgangs", "exit ")))?;
        arm.entry = rounding(&a, roads, c, dims.r_out * f, o_l, dims.r_entry * f, d1).ok_or_else(|| fail(tr("ingangs", "entry ")))?;
    }
    // Neighbouring arms must leave a piece of circle between them.
    let n = arms.len();
    for i in 0..n {
        let (a, b) = (&arms[i], &arms[(i + 1) % n]);
        let gap = (ang(sub(b.exit.t_circle, c)) - ang(sub(a.entry.t_circle, c))).rem_euclid(TAU);
        let own = (ang(sub(a.entry.t_circle, c)) - ang(sub(a.exit.t_circle, c))).rem_euclid(TAU);
        if gap > PI * 1.9 || own > PI {
            return Err(trf!("armen {} en {} liggen te dicht bij elkaar", "arms {} and {} are too close together", i + 1, (i + 1) % n + 1));
        }
    }
    let step = 0.5 * f;
    let mut outline = Vec::new();
    for i in 0..n {
        let a = &arms[i];
        let (ex, en) = (&a.exit, &a.entry);
        let s0 = ang(sub(ex.t_circle, ex.c));
        outline.extend(arc_points(ex.c, ex.r, s0, wrap(ang(sub(ex.t_edge, ex.c)) - s0), step));
        let mut d = ex.d_edge + step;
        while d < a.d_m {
            outline.push(a.point(roads, d, a.edges(dims, f, d).0));
            d += step;
        }
        outline.push(a.point(roads, a.d_m, a.edges(dims, f, a.d_m).0));
        outline.push(a.point(roads, a.d_m, a.edges(dims, f, a.d_m).1));
        let mut d = a.d_m - step;
        while d > en.d_edge {
            outline.push(a.point(roads, d, a.edges(dims, f, d).1));
            d -= step;
        }
        let s0 = ang(sub(en.t_edge, en.c));
        outline.extend(arc_points(en.c, en.r, s0, wrap(ang(sub(en.t_circle, en.c)) - s0), step));
        let b = &arms[(i + 1) % n];
        let a0 = ang(sub(en.t_circle, c));
        let sweep = (ang(sub(b.exit.t_circle, c)) - a0).rem_euclid(TAU);
        let pts = arc_points(c, dims.r_out * f, a0, sweep, step);
        outline.extend(pts[1..pts.len() - 1].iter().copied());
    }
    outline.dedup_by(|a, b| dist(*a, *b) < 1e-6);
    // Splitter islands: a round nose at the circle, straight sides, a point
    // at the far end.
    let w = dims.split_w * f / 2.0;
    let nose = dims.r_out * f + 1.0 * f + w;
    let mut splitters = Vec::new();
    for a in &arms {
        let mut s = Vec::new();
        let end = d1 - 4.0 * f;
        let mut d = nose;
        while d < end {
            s.push(a.point(roads, d, w));
            d += step;
        }
        s.push(a.point(roads, end, w));
        s.push(a.point(roads, d1, 0.0));
        s.push(a.point(roads, end, -w));
        let mut d = end - step;
        while d > nose {
            s.push(a.point(roads, d, -w));
            d -= step;
        }
        // The nose: a half circle round (nose, 0).
        for j in 0..=12 {
            let t = -PI / 2.0 - PI * j as f64 / 12.0;
            s.push(a.point(roads, nose + w * t.cos(), w * t.sin()));
        }
        splitters.push(s);
    }
    Ok(RoundPlan { c, dims: dims.clone(), arms, outline, splitters })
}

/// Heights on a roundabout: a cone falling outward over the carriageway,
/// the arms warped from its edge to their road at the mouth.
struct RoundSurface<'a> {
    roads: &'a [Road],
    plan: &'a RoundPlan,
    profiles: Vec<Vec<(f64, f64)>>,
    f: f64,
    /// Height of the carriageway's inner edge (m).
    z0: f64,
}

impl RoundSurface<'_> {
    fn ring(&self, r: f64) -> f64 {
        let d = &self.plan.dims;
        self.z0 - d.slope * (r - d.r_in)
    }

    fn z(&self, p: XY) -> f64 {
        let (d, f) = (&self.plan.dims, self.f);
        let r = dist(p, self.plan.c) / f;
        if r <= d.r_out + 1e-6 {
            return self.ring(r);
        }
        let a = ang(sub(p, self.plan.c));
        let Some(arm) = self.plan.arms.iter().min_by(|x, y| wrap(x.angle - a).abs().total_cmp(&wrap(y.angle - a).abs())) else { return self.ring(r) };
        let road = &self.roads[arm.road];
        let (s, off) = road.al.station_offset(p);
        let dd = (s - arm.s_c) * arm.dir;
        let o = off * arm.dir;
        let ro = d.r_out * f;
        let dc = if o.abs() < ro { (ro * ro - o * o).sqrt() } else { 0.0 };
        let edge = self.ring(d.r_out);
        let sm = arm.s_c + arm.dir * arm.d_m;
        let mouth = road.design(sm, f, edge) + profile_z(&self.profiles[arm.road], off / f);
        let t = ((dd - dc) / (arm.d_m - dc).max(1e-9)).clamp(0.0, 1.0);
        edge + (mouth - edge) * t
    }
}

pub struct RoundaboutRequest {
    pub axes: Vec<Handle>,
    /// The centre (drawing); None: where the first two axes cross.
    pub centre: Option<XY>,
    pub dims: RoundDims,
    pub asm: Option<String>,
    /// Height of the carriageway's inner edge (m); None: from the arms.
    pub level: Option<f64>,
    pub name: Option<String>,
    /// The surfaces in natural colours, else by layer.
    pub natural: bool,
}

/// `CIV_ROTONDE`.
pub fn roundabout(opt: &super::CivielOptions, req: RoundaboutRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let mut opt = opt.clone();
    opt.nlcs.status = Status::N;
    let ctx = Ctx::new(&opt)?;
    let (f, k) = (ctx.f, ctx.k);
    if req.axes.is_empty() {
        return Err(tr("geef de assen van de armen: ASSEN=<handle>;<handle>…", "give the axes of the arms: ASSEN=<handle>;<handle>…").into());
    }
    let doc = super::document(&["ontwerpprofiel", "corridor"], &req.axes)?;
    let roads: Vec<Road> = req.axes.iter().map(|h| road_of(&doc, *h, req.asm.as_deref())).collect::<Result<_, _>>()?;
    let c = match req.centre {
        Some(c) => c,
        None => {
            if roads.len() < 2 {
                return Err(tr("geef MIDDEN=x,y of twee assen die elkaar kruisen", "give MIDDEN=x,y or two axes that cross").into());
            }
            let xs = crossings(&axis_points(&roads[0].al, 0.5 * f, 0.0), &axis_points(&roads[1].al, 0.5 * f, 0.0));
            xs.first().map(|x| x.2).ok_or_else(|| tr("de eerste twee assen kruisen elkaar niet: geef MIDDEN=x,y", "the first two axes do not cross: give MIDDEN=x,y").to_string())?
        }
    };
    progress(tr("Rotonde: geometrie…", "Roundabout: geometry…").into());
    let dims = req.dims.clone();
    let plan = plan_roundabout(c, &roads, &dims, f)?;
    let z0 = match req.level {
        Some(z) => z,
        None => {
            let zs: Vec<f64> = plan.arms.iter().filter_map(|a| roads[a.road].pvis.as_ref().map(|p| as_profiel::design_z(p, (a.s_c + a.dir * dims.r_out * f) / f))).collect();
            if zs.is_empty() {
                return Err(tr("geen van de assen heeft een ontwerpprofiel: geef NIVEAU=… (hoogte van de binnenrand)", "none of the axes has a design profile: give NIVEAU=… (height of the inner edge)").into());
            }
            zs.iter().sum::<f64>() / zs.len() as f64 + dims.slope * (dims.r_out - dims.r_in)
        }
    };
    let surf = RoundSurface { roads: &roads, plan: &plan, profiles: roads.iter().map(|r| carriageway_profile(&r.asm)).collect(), f, z0 };
    progress(tr("Rotonde: oppervlak…", "Roundabout: surface…").into());
    // Points: the outlines, a polar grid on the carriageway, a grid on each arm.
    let mut pts: Vec<P3> = Vec::new();
    let add = |p: XY, pts: &mut Vec<P3>| pts.push([p[0], p[1], surf.z(p) * f]);
    for p in plan.outline.iter().chain(plan.splitters.iter().flatten()) {
        add(*p, &mut pts);
    }
    let rings = ((dims.r_out - dims.r_in).ceil() as usize).max(2);
    for j in 0..=rings {
        let r = (dims.r_in + (dims.r_out - dims.r_in) * j as f64 / rings as f64) * f;
        let m = ((TAU * r) / f).ceil() as usize;
        for i in 0..m {
            let a = TAU * i as f64 / m as f64;
            add([c[0] + r * a.cos(), c[1] + r * a.sin()], &mut pts);
        }
    }
    for a in &plan.arms {
        let mut d = dims.r_out * f;
        while d <= a.d_m {
            let (o_r, o_l) = a.edges(&dims, f, d);
            let mut o = o_l;
            while o <= o_r {
                add(a.point(&roads, d, o), &mut pts);
                o += f;
            }
            d += f;
        }
    }
    let inner: Vec<XY> = arc_points(c, dims.r_in * f, 0.0, TAU, 0.5 * f);
    let mut rings_poly = vec![plan.outline.clone(), inner[..inner.len() - 1].to_vec()];
    rings_poly.extend(plan.splitters.iter().cloned());
    let tris = patch(&pts, &rings_poly)?;
    // The overrunnable strip and the island, as rings.
    let name = req.name.unwrap_or_else(|| format!("{} {}", tr("Rotonde", "Roundabout"), roads.iter().map(|r| axis_label(&r.al)).collect::<Vec<_>>().join(" / ")));
    let data = record(&[("rotonde", name.clone()), ("diameter_m", format!("{:.2}", dims.r_out * 2.0)), ("assen", roads.iter().map(|r| r.al.name.clone()).collect::<Vec<_>>().join(";"))]);
    let faces = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let mut mesh = MeshBuilder::default();
    for t in &tris {
        mesh.tri(t[0], t[1], t[2]);
    }
    let r_c = dims.r_in - dims.apron;
    let apron_z = |r: f64| z0 + dims.apron_up + dims.slope * (dims.r_in - r);
    let ring = |r: f64, z: f64, m: usize| -> Vec<P3> { (0..m).map(|i| TAU * i as f64 / m as f64).map(|a| [c[0] + r * f * a.cos(), c[1] + r * f * a.sin(), z * f]).collect() };
    let m = ((TAU * dims.r_in).ceil() as usize).max(24);
    let band = |mesh: &mut MeshBuilder, a: &[P3], b: &[P3]| {
        for i in 0..a.len() {
            let j = (i + 1) % a.len();
            mesh.quad(a[i], a[j], b[j], b[i]);
        }
    };
    let apron_rings = [ring(dims.r_in, z0, m), ring(dims.r_in - 0.02, apron_z(dims.r_in), m), ring(r_c, apron_z(r_c), m)];
    let mut apron = MeshBuilder::default();
    band(&mut apron, &apron_rings[0], &apron_rings[1]);
    band(&mut apron, &apron_rings[1], &apron_rings[2]);
    let face_count = mesh.faces() + apron.faces();
    let mut meshes = mesh.finish(&faces.name, natural(req.natural, "GESLOTENVERHARDING_ASFALT"), Some(&data));
    meshes.extend(apron.finish(&faces.name, natural(req.natural, "OPENVERHARDING_STRAATBAKSTEEN"), Some(&data)));
    let mesh_count = meshes.len();
    let mut out = Out::default();
    out.put(&faces, meshes);
    let grass = ctx.layer("GR", "GRAS", Element::G)?;
    let mut island = MeshBuilder::default();
    let top = apron_z(r_c) + dims.island_up;
    let island_rings = [ring(r_c, apron_z(r_c), m), ring(r_c - 0.02, top, m), ring(r_c * 0.6, top + 0.2, m), ring(r_c * 0.25, top + 0.3, m)];
    for w in island_rings.windows(2) {
        band(&mut island, &w[0], &w[1]);
    }
    let centre = [c[0], c[1], (top + 0.35) * f];
    let last = &island_rings[3];
    for i in 0..last.len() {
        island.tri(last[i], last[(i + 1) % last.len()], centre);
    }
    out.put(&grass, island.finish(&grass.name, None, Some(&data)));
    // Plan: the outer kerb line between the mouths, the circles, splitters.
    let kerb_l = ctx.layer("VH", "KANTOPSLUITING_TROTTOIRBAND", Element::G)?;
    let apron_l = ctx.layer("VH", "KANTOPSLUITING_ROTONDEBLOK", Element::G)?;
    let teeth_l = ctx.layer("VW", "MARKERING_DWARS_DRIEHOEK", Element::G)?;
    let n = plan.arms.len();
    let bulge = |sweep: f64| (sweep / 4.0).tan();
    for i in 0..n {
        let (a, b) = (&plan.arms[i], &plan.arms[(i + 1) % n]);
        let mut v: Vec<(XY, f64)> = Vec::new();
        let mut d = a.d_m;
        while d > a.entry.d_edge + 1e-9 {
            v.push((a.point(&roads, d, a.edges(&dims, f, d).1), 0.0));
            d -= 1.0 * f;
        }
        let en = &a.entry;
        let e0 = ang(sub(en.t_edge, en.c));
        v.push((en.t_edge, bulge(wrap(ang(sub(en.t_circle, en.c)) - e0))));
        let c0 = ang(sub(en.t_circle, c));
        v.push((en.t_circle, bulge((ang(sub(b.exit.t_circle, c)) - c0).rem_euclid(TAU))));
        let ex = &b.exit;
        let x0 = ang(sub(ex.t_circle, ex.c));
        v.push((ex.t_circle, bulge(wrap(ang(sub(ex.t_edge, ex.c)) - x0))));
        v.push((ex.t_edge, 0.0));
        let mut d = ex.d_edge + 1.0 * f;
        while d < b.d_m {
            v.push((b.point(&roads, d, b.edges(&dims, f, d).0), 0.0));
            d += 1.0 * f;
        }
        v.push((b.point(&roads, b.d_m, b.edges(&dims, f, b.d_m).0), 0.0));
        out.put(&kerb_l, [lw(&v, false, &kerb_l, k, Some(&data))]);
    }
    let circle = |r: f64| vec![([c[0] + r * f, c[1]], 1.0), ([c[0] - r * f, c[1]], 1.0)];
    out.put(&apron_l, [lw(&circle(dims.r_in), true, &apron_l, k, Some(&data))]);
    out.put(&apron_l, [lw(&circle(r_c), true, &apron_l, k, Some(&data))]);
    for s in &plan.splitters {
        out.put(&kerb_l, [lw(&s.iter().map(|p| (*p, 0.0)).collect::<Vec<_>>(), true, &kerb_l, k, Some(&data))]);
    }
    // Give way at every entry, along the circle across the entry lane.
    let mut teeth = 0;
    for a in &plan.arms {
        let w = dims.split_w / 2.0 * f;
        let o_far = -(dims.split_w / 2.0 + dims.entry_w - 0.3) * f;
        let dd = |o: f64| ((dims.r_out * f).powi(2) - o * o).max(0.0).sqrt() + 0.6 * f;
        let (p0, p1) = (a.point(&roads, dd(-w - 0.1 * f), -w - 0.1 * f), a.point(&roads, dd(o_far), o_far));
        let (_, dir) = roads[a.road].al.at(a.s_c + a.dir * dd(o_far));
        let toward = [dir.cos() * a.dir, dir.sin() * a.dir];
        for t in shark_teeth(p0, p1, toward, 0.5 * f, 0.6 * f, 0.25 * f) {
            out.put(&teeth_l, [lw(&t.iter().map(|p| (*p, 0.0)).collect::<Vec<_>>(), true, &teeth_l, k, Some(&data))]);
            teeth += 1;
        }
    }
    let handles = out.write_handles(&name)?;
    crate::bridge::index_add("tin", "N", &handles[..mesh_count.min(handles.len())], false);
    crate::bridge::index_add("rotonde", &name, &handles[..1.min(handles.len())], true);
    let cp = ctx.from_drawing(c);
    let arms: Vec<serde_json::Value> = plan
        .arms
        .iter()
        .map(|a| {
            let road = &roads[a.road];
            let sm = road.al.start + (a.s_c + a.dir * a.d_m) / f;
            serde_json::json!({
                "as": road.al.name, "richting": if a.dir > 0.0 { "+" } else { "-" }, "hoek_graden": a.angle.to_degrees(),
                "station_midden": road.al.start + a.s_c / f, "station_mond": sm,
                "corridor": format!("{}={:.2}", if a.dir > 0.0 { "VAN" } else { "TOT" }, sm),
                "uitgangsboog_m": a.exit.r / f, "ingangsboog_m": a.entry.r / f,
            })
        })
        .collect();
    super::api::note("rotonde", name.clone());
    super::api::note("midden", serde_json::json!([cp[0], cp[1]]));
    super::api::note("buitendiameter_m", dims.r_out * 2.0);
    super::api::note("binnendiameter_m", dims.r_in * 2.0);
    super::api::note("rijbaan_m", dims.r_out - dims.r_in);
    super::api::note("overrijdbaar_m", dims.apron);
    super::api::note("hoogte_binnenrand", z0);
    super::api::note("hoogte_buitenrand", surf.ring(dims.r_out));
    super::api::note("armen", arms);
    super::api::note("driehoeken", face_count);
    super::api::note("haaientanden", teeth);
    let mouths: Vec<String> = plan.arms.iter().map(|a| station(roads[a.road].al.start + (a.s_c + a.dir * a.d_m) / f)).collect();
    Ok(trf!(
        "{}: D = {} m, rijbaan {} m, overrijdbare strook {} m, {} armen; {} driehoeken, binnenrand {} m, buitenrand {} m ({} % naar buiten); armen eindigen op {} — bouw de corridors vanaf daar. {} elementen.",
        "{}: D = {} m, carriageway {} m, overrunnable strip {} m, {} arms; {} triangles, inner edge {} m, outer edge {} m ({} % outward); the arms end at {} — build the corridors from there. {} entities.",
        name,
        num(dims.r_out * 2.0, 2),
        num(dims.r_out - dims.r_in, 2),
        num(dims.apron, 2),
        n,
        face_count,
        num(z0, 2),
        num(surf.ring(dims.r_out), 2),
        num(dims.slope * 100.0, 1),
        mouths.join(", "),
        handles.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::civiel::corridor::{Part, PartKind};

    fn street() -> Assembly {
        let mut kerb = Part::new(PartKind::Kerb, 0.15, 0.0);
        (kerb.upstand, kerb.height, kerb.batter) = (0.12, 0.25, 0.02);
        Assembly::symmetric("s", vec![Part::new(PartKind::Lane, 3.0, -0.025), Part::new(PartKind::Gutter, 0.3, -0.025), kerb, Part::new(PartKind::Footway, 1.8, 0.02)])
    }

    fn road(pis: &[XY], z: f64) -> Road {
        let al = Alignment::from_pis(pis, &[]).unwrap();
        let l = al.length();
        Road { al, asm: street(), pvis: Some(vec![Pvi { s: 0.0, z, l: 0.0 }, Pvi { s: l, z, l: 0.0 }]) }
    }

    #[test]
    fn a_t_junction_with_kerb_returns() {
        // Main road west–east, side road from the south up to the main axis.
        let roads = [road(&[[-100.0, 0.0], [100.0, 0.0]], 1.0), road(&[[0.0, -80.0], [0.0, 0.0]], 0.6)];
        let plan = plan_junction(&roads, &[8.0], 1.0).unwrap();
        assert_eq!(plan.kind, "T");
        assert_eq!(plan.arms.len(), 3);
        assert!(dist(plan.x, [0.0, 0.0]) < 1e-6);
        // Two rounded corners, centres 3,3 + 8 from both axes.
        let fillets: Vec<&Fillet> = plan.corners.iter().flatten().collect();
        assert_eq!(fillets.len(), 2);
        for c in &fillets {
            assert!((c.c[0].abs() - 11.3).abs() < 0.02 && (c.c[1] + 11.3).abs() < 0.02, "{:?}", c.c);
            // The tangent points lie on the edges, R from the centre.
            for t in c.t {
                assert!((dist(t, c.c) - 8.0).abs() < 0.02);
            }
        }
        // The paved area holds the junction and the side arm, not the corner block.
        let rings = [plan.boundary.clone()];
        assert!(inside(&rings, [0.0, 0.0]) && inside(&rings, [0.0, -10.0]) && inside(&rings, [12.0, 2.0]));
        assert!(!inside(&rings, [20.0, 2.0]), "past the mouth");
        assert!(!inside(&rings, [8.0, -8.0]) && !inside(&rings, [0.0, 5.0]));
        // Arm mouths past the tangent points (3,3 + 8 = 11,3 from the junction).
        for a in &plan.arms {
            assert!(a.mouth >= 11.3 + 2.0 - 1e-6, "{a:?}");
        }
    }

    #[test]
    fn the_side_road_ties_in_to_the_main_road() {
        let roads = [road(&[[-100.0, 0.0], [100.0, 0.0]], 1.0), road(&[[0.0, -80.0], [0.0, 0.0]], 0.6)];
        let plan = plan_junction(&roads, &[8.0], 1.0).unwrap();
        let surf = JunctionSurface { roads: &roads, plan: &plan, profiles: [carriageway_profile(&roads[0].asm), carriageway_profile(&roads[1].asm)], f: 1.0, level: 1.0 };
        // On the main road: its own crowned section.
        assert!((surf.z([5.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!((surf.z([5.0, -3.0]) - (1.0 - 0.075)).abs() < 1e-9);
        // The side road at the main road's edge takes its height, at the
        // mouth its own section; between them a straight grade.
        let arm = plan.arms.iter().find(|a| a.road == 1).unwrap();
        let edge_z = 1.0 - 0.025 * 3.3;
        assert!((surf.z([0.0, -3.31]) - edge_z).abs() < 0.01, "{}", surf.z([0.0, -3.31]));
        assert!((surf.z([0.0, -arm.mouth]) - 0.6).abs() < 1e-6);
        let mid = surf.z([0.0, -(3.3 + arm.mouth) / 2.0]);
        assert!((mid - (edge_z + 0.6) / 2.0).abs() < 0.01, "{mid}");
        // A crossing side road makes an X with four rounded corners.
        let roads = [road(&[[-100.0, 0.0], [100.0, 0.0]], 1.0), road(&[[0.0, -80.0], [0.0, 80.0]], 1.0)];
        let plan = plan_junction(&roads, &[6.0, 10.0], 1.0).unwrap();
        assert_eq!(plan.kind, "X");
        let r: Vec<f64> = plan.corners.iter().flatten().map(|c| c.r).collect();
        assert_eq!(r, [6.0, 10.0, 10.0, 10.0]);
        // A patch over a T: every triangle in the paved area.
        let pts: Vec<P3> = plan.boundary.iter().map(|p| [p[0], p[1], 0.0]).collect();
        let tris = patch(&pts, std::slice::from_ref(&plan.boundary)).unwrap();
        assert!(!tris.is_empty());
    }

    #[test]
    fn a_crow_roundabout_with_four_arms() {
        let roads = vec![road(&[[-120.0, 0.0], [120.0, 0.0]], 1.0), road(&[[0.0, -120.0], [0.0, 120.0]], 1.0)];
        let dims = RoundDims::crow(true);
        let plan = plan_roundabout([0.0, 0.0], &roads, &dims, 1.0).unwrap();
        assert_eq!(plan.arms.len(), 4);
        for a in &plan.arms {
            // The roundings touch the circle and their edge.
            assert!((dist(a.exit.t_circle, [0.0, 0.0]) - 16.0).abs() < 1e-6);
            assert!((dist(a.exit.c, [0.0, 0.0]) - 31.0).abs() < 1e-6);
            assert!((dist(a.exit.t_edge, a.exit.c) - 15.0).abs() < 1e-6);
            assert!((dist(a.entry.c, [0.0, 0.0]) - 28.0).abs() < 1e-6);
            // Exit 5,75 m right of the axis, entry 5,25 m left; tangents before the splitter's end.
            assert!((a.exit.d_edge - (31.0f64.powi(2) - 20.75f64.powi(2)).sqrt()).abs() < 1e-6);
            assert!(a.entry.d_edge < 30.0 && a.exit.d_edge < 30.0);
        }
        let mut rings = vec![plan.outline.clone(), arc_points([0.0, 0.0], 10.5, 0.0, TAU, 0.5)];
        rings.extend(plan.splitters.iter().cloned());
        // The carriageway is paved; the island and the splitters are not.
        assert!(inside(&rings, [0.0, -13.0]) && inside(&rings, [13.0, 0.0]));
        assert!(!inside(&rings, [0.0, 0.0]) && !inside(&rings, [0.0, 22.0]));
        // Entry lane of the east arm (left looking outward = north side) is paved.
        assert!(inside(&rings, [25.0, 3.0]) && inside(&rings, [25.0, -3.0]));
        // The surface falls outward and ties in at the mouths.
        let surf = RoundSurface { roads: &roads, plan: &plan, profiles: roads.iter().map(|r| carriageway_profile(&r.asm)).collect(), f: 1.0, z0: 1.2 };
        assert!((surf.z([0.0, 10.5]) - 1.2).abs() < 1e-9);
        assert!((surf.z([0.0, 16.0]) - (1.2 - 0.025 * 5.5)).abs() < 1e-9);
        let a = &plan.arms[0];
        assert!((surf.z(a.point(&roads, a.d_m, 0.0)) - 1.0).abs() < 1e-6);
        // Three arms from a road that ends at the centre.
        let roads = vec![road(&[[-120.0, 0.0], [120.0, 0.0]], 1.0), road(&[[0.0, -120.0], [0.0, 0.0]], 1.0)];
        assert_eq!(plan_roundabout([0.0, 0.0], &roads, &dims, 1.0).unwrap().arms.len(), 3);
        // Two arms is no roundabout.
        let roads = vec![road(&[[-120.0, 0.0], [120.0, 0.0]], 1.0)];
        assert!(plan_roundabout([0.0, 0.0], &roads, &dims, 1.0).is_err());
    }
}
