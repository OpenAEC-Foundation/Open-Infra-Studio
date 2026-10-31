//! Alignments (Road "Alignment", "Profile", "Region", "Section"): an axis
//! from a polyline with stationing, tangent points and radii; the length
//! profile and cross sections along it, over the TIN in the drawing or AHN.

use std::collections::HashSet;
use std::f64::consts::{FRAC_PI_2, PI};

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle, LwPolyline, Vector2};

use super::{nice_step, record, station, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::{Element, LayerSpec};

/// One piece of the axis, in drawing units.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Line { a: XY, b: XY },
    /// Centre, radius, start angle and signed sweep (counter-clockwise > 0).
    Arc { c: XY, r: f64, a0: f64, sweep: f64 },
}

impl Seg {
    pub fn length(&self) -> f64 {
        match self {
            Seg::Line { a, b } => (b[0] - a[0]).hypot(b[1] - a[1]),
            Seg::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }

    /// Point and direction (radians) at distance `s` from the start.
    pub fn at(&self, s: f64) -> (XY, f64) {
        match self {
            Seg::Line { a, b } => {
                let len = self.length().max(1e-12);
                let t = s / len;
                ([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t], (b[1] - a[1]).atan2(b[0] - a[0]))
            }
            Seg::Arc { c, r, a0, sweep } => {
                let sign = sweep.signum();
                let ang = a0 + sign * s / r;
                ([c[0] + r * ang.cos(), c[1] + r * ang.sin()], ang + sign * FRAC_PI_2)
            }
        }
    }

    /// A polyline piece from `a` to `b` with a DXF bulge.
    pub fn from_bulge(a: XY, b: XY, bulge: f64) -> Seg {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let chord = dx.hypot(dy);
        if bulge.abs() < 1e-9 || chord < 1e-12 {
            return Seg::Line { a, b };
        }
        let theta = 4.0 * bulge.atan();
        let r = chord / (2.0 * (theta / 2.0).sin());
        let h = r * (theta / 2.0).cos();
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let c = [mid[0] - dy / chord * h, mid[1] + dx / chord * h];
        Seg::Arc { c, r: r.abs(), a0: (a[1] - c[1]).atan2(a[0] - c[0]), sweep: theta }
    }

    /// The piece moved `d` to the right (left when negative); a curve whose
    /// radius would vanish is left out.
    pub fn offset(&self, d: f64) -> Option<Seg> {
        match self {
            Seg::Line { a, b } => {
                let dir = (b[1] - a[1]).atan2(b[0] - a[0]);
                let n = [dir.sin() * d, -dir.cos() * d];
                Some(Seg::Line { a: [a[0] + n[0], a[1] + n[1]], b: [b[0] + n[0], b[1] + n[1]] })
            }
            // Right of a counter-clockwise curve is outside.
            Seg::Arc { c, r, a0, sweep } => {
                let r2 = r + sweep.signum() * d;
                (r2 > 1e-9).then_some(Seg::Arc { c: *c, r: r2, a0: *a0, sweep: *sweep })
            }
        }
    }

    /// Nearest point on the piece: distance along it, offset (right > 0)
    /// and distance to it.
    pub fn project(&self, p: XY) -> (f64, f64, f64) {
        match self {
            Seg::Line { a, b } => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len2 = (dx * dx + dy * dy).max(1e-24);
                let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0);
                let foot = [a[0] + dx * t, a[1] + dy * t];
                let len = len2.sqrt();
                let cross = (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / len;
                (t * len, -cross, (p[0] - foot[0]).hypot(p[1] - foot[1]))
            }
            Seg::Arc { c, r, a0, sweep } => {
                let phi = (p[1] - c[1]).atan2(p[0] - c[0]);
                let delta = if *sweep > 0.0 { (phi - a0).rem_euclid(2.0 * PI) } else { (a0 - phi).rem_euclid(2.0 * PI) };
                let rho = (p[0] - c[0]).hypot(p[1] - c[1]);
                let off = sweep.signum() * (rho - r);
                if delta <= sweep.abs() {
                    (r * delta, off, off.abs())
                } else {
                    // Past an end: the nearer end point.
                    let (e0, e1) = (self.at(0.0).0, self.at(self.length()).0);
                    let (d0, d1) = ((p[0] - e0[0]).hypot(p[1] - e0[1]), (p[0] - e1[0]).hypot(p[1] - e1[1]));
                    if d0 <= d1 { (0.0, off, d0) } else { (self.length(), off, d1) }
                }
            }
        }
    }

    fn bulge(&self) -> f64 {
        match self {
            Seg::Line { .. } => 0.0,
            Seg::Arc { sweep, .. } => (sweep / 4.0).tan(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Alignment {
    pub name: String,
    /// Station at the start (m).
    pub start: f64,
    pub segs: Vec<Seg>,
}

/// What happens at a point of the axis: begin or end of a curve (BB, EB),
/// a corner between straights (HP), the start and the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// Distance from the start (drawing units).
    pub s: f64,
    pub code: &'static str,
}

impl Alignment {
    /// The axis of a line, arc or (2D) polyline.
    pub fn from_entity(e: &EntityType) -> Option<Alignment> {
        let segs = match e {
            EntityType::Line(l) => vec![Seg::Line { a: [l.start.x, l.start.y], b: [l.end.x, l.end.y] }],
            EntityType::Arc(a) => {
                let mut sweep = a.end_angle - a.start_angle;
                if sweep <= 0.0 {
                    sweep += 2.0 * PI;
                }
                vec![Seg::Arc { c: [a.center.x, a.center.y], r: a.radius, a0: a.start_angle, sweep }]
            }
            EntityType::LwPolyline(l) => {
                let v: Vec<(XY, f64)> = l.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
                pieces(&v, l.is_closed)
            }
            EntityType::Polyline2D(p) => {
                let v: Vec<(XY, f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
                pieces(&v, p.flags.is_closed())
            }
            EntityType::Polyline3D(p) => {
                let v: Vec<(XY, f64)> = p.vertices.iter().map(|v| ([v.position.x, v.position.y], 0.0)).collect();
                pieces(&v, p.flags.closed)
            }
            _ => return None,
        };
        let segs: Vec<Seg> = segs.into_iter().filter(|s| s.length() > 1e-9).collect();
        if segs.is_empty() {
            return None;
        }
        let name = super::data_value(e, "as").unwrap_or_default();
        let start = super::data_value(e, "beginstation").and_then(|s| s.replace(',', ".").parse().ok()).unwrap_or(0.0);
        Some(Alignment { name, start, segs })
    }

    pub fn length(&self) -> f64 {
        self.segs.iter().map(Seg::length).sum()
    }

    /// Point and direction at distance `s` along the axis (clamped).
    pub fn at(&self, s: f64) -> (XY, f64) {
        let mut run = 0.0;
        for seg in &self.segs {
            let l = seg.length();
            if s <= run + l {
                return seg.at((s - run).max(0.0));
            }
            run += l;
        }
        let last = self.segs.last().unwrap();
        last.at(last.length())
    }

    /// Begin and end of curves, corners, start and end.
    pub fn marks(&self) -> Vec<Mark> {
        let mut out = vec![Mark { s: 0.0, code: "BA" }];
        let mut run = 0.0;
        for (i, seg) in self.segs.iter().enumerate() {
            let l = seg.length();
            let arc = matches!(seg, Seg::Arc { .. });
            if i > 0 {
                let prev_arc = matches!(self.segs[i - 1], Seg::Arc { .. });
                let (_, d0) = self.segs[i - 1].at(self.segs[i - 1].length());
                let (_, d1) = seg.at(0.0);
                // Under a quarter degree is a chord of a drawn spiral, not a corner.
                let kink = (((d1 - d0 + PI).rem_euclid(2.0 * PI)) - PI).abs() > 0.25_f64.to_radians();
                let code = match (prev_arc, arc) {
                    (false, true) => "BB",
                    (true, false) => "EB",
                    (true, true) => "OB",
                    (false, false) if kink => "HP",
                    _ => "",
                };
                if !code.is_empty() {
                    out.push(Mark { s: run, code });
                }
            }
            run += l;
        }
        out.push(Mark { s: run, code: "EA" });
        out
    }

    /// Distances along the axis every `every` (drawing units) from station
    /// zero of the axis, plus every mark when asked.
    pub fn stations(&self, every: f64, marks: bool) -> Vec<f64> {
        let len = self.length();
        let mut out = Vec::new();
        if every > 0.0 {
            // Round stations: the first multiple of `every` at or after the start station.
            let mut s = 0.0;
            while s <= len + 1e-9 {
                out.push(s);
                s += every;
            }
        }
        if marks {
            out.extend(self.marks().into_iter().map(|m| m.s));
        }
        out.push(len);
        out.sort_by(f64::total_cmp);
        out.dedup_by(|a, b| (*a - *b).abs() < every.max(1e-6) * 1e-3);
        out
    }

    /// Station distance (drawing units from the start) and offset (right > 0)
    /// of a point.
    pub fn station_offset(&self, p: XY) -> (f64, f64) {
        let mut best = (0.0, 0.0, f64::MAX);
        let mut run = 0.0;
        for seg in &self.segs {
            let (s, off, dist) = seg.project(p);
            if dist < best.2 {
                best = (run + s, off, dist);
            }
            run += seg.length();
        }
        (best.0, best.1)
    }

    /// The axis moved `d` to the right (left when negative): straights meet
    /// at their intersection, curves keep their centre.
    pub fn offset_polyline(&self, d: f64) -> Option<LwPolyline> {
        let segs: Vec<Seg> = self.segs.iter().filter_map(|s| s.offset(d)).collect();
        let first = segs.first()?;
        let mut v: Vec<(XY, f64)> = vec![(first.at(0.0).0, first.bulge())];
        for (i, seg) in segs.iter().enumerate() {
            let end = seg.at(seg.length()).0;
            let Some(next) = segs.get(i + 1) else {
                v.push((end, 0.0));
                break;
            };
            let start = next.at(0.0).0;
            if (end[0] - start[0]).hypot(end[1] - start[1]) < 1e-6 {
                v.push((start, next.bulge()));
            } else if let (Seg::Line { a: a1, b: b1 }, Seg::Line { a: a2, b: b2 }) = (seg, next) {
                match intersect(*a1, *b1, *a2, *b2) {
                    Some(x) => v.push((x, 0.0)),
                    None => {
                        v.push((end, 0.0));
                        v.push((start, 0.0));
                    }
                }
            } else {
                v.push((end, 0.0));
                v.push((start, next.bulge()));
            }
        }
        let mut lw = LwPolyline::new();
        for (p, b) in v {
            lw.add_point_with_bulge(Vector2::new(p[0], p[1]), b);
        }
        Some(lw)
    }

    /// Plan extent (the curves by their points every few metres).
    pub fn bbox(&self) -> [f64; 4] {
        let len = self.length();
        let n = ((len / 5.0).ceil() as usize).clamp(8, 4000);
        bounds((0..=n).map(|i| self.at(len * i as f64 / n as f64).0))
    }

    /// An axis through points of intersection ("by layout"): the
    /// straights between them, a circular curve of the given radius at each
    /// inner PI (the last radius repeats; 0 leaves a corner).
    pub fn from_pis(pis: &[XY], radii: &[f64]) -> Result<Alignment, String> {
        if pis.len() < 2 {
            return Err("een as heeft minstens twee punten nodig".into());
        }
        let mut segs = Vec::new();
        let mut from = pis[0];
        // Tangent length used at each PI, to check that curves fit.
        let mut prev_t = 0.0;
        for i in 1..pis.len() - 1 {
            let (a, p, b) = (pis[i - 1], pis[i], pis[i + 1]);
            let d_in = (p[1] - a[1]).atan2(p[0] - a[0]);
            let d_out = (b[1] - p[1]).atan2(b[0] - p[0]);
            let delta = (d_out - d_in + PI).rem_euclid(2.0 * PI) - PI;
            let r = radii.get(i - 1).or(radii.last()).copied().unwrap_or(0.0);
            if r <= 0.0 || delta.abs() < 1e-9 {
                segs.push(Seg::Line { a: from, b: p });
                from = p;
                prev_t = 0.0;
                continue;
            }
            let t = r * (delta.abs() / 2.0).tan();
            let len_in = (p[0] - a[0]).hypot(p[1] - a[1]);
            if prev_t + t > len_in + 1e-6 {
                return Err(format!("boog {i} (R = {r:.0}) past niet: raaklijn {:.1} m, beschikbaar {:.1} m", t, len_in - prev_t));
            }
            let bc = [p[0] - d_in.cos() * t, p[1] - d_in.sin() * t];
            let ec = [p[0] + d_out.cos() * t, p[1] + d_out.sin() * t];
            // Centre on the inside: left of the way for a left turn.
            let side = delta.signum();
            let c = [bc[0] - d_in.sin() * r * side, bc[1] + d_in.cos() * r * side];
            if (bc[0] - from[0]).hypot(bc[1] - from[1]) > 1e-9 {
                segs.push(Seg::Line { a: from, b: bc });
            }
            segs.push(Seg::Arc { c, r, a0: (bc[1] - c[1]).atan2(bc[0] - c[0]), sweep: delta });
            from = ec;
            prev_t = t;
        }
        let last = pis[pis.len() - 1];
        let len_last = (last[0] - pis[pis.len() - 2][0]).hypot(last[1] - pis[pis.len() - 2][1]);
        if prev_t > len_last + 1e-6 {
            return Err(format!("de laatste boog past niet: raaklijn {prev_t:.1} m, rechtstand {len_last:.1} m"));
        }
        if (last[0] - from[0]).hypot(last[1] - from[1]) > 1e-9 {
            segs.push(Seg::Line { a: from, b: last });
        }
        Ok(Alignment { name: String::new(), start: 0.0, segs })
    }

    /// The axis as a polyline with bulges.
    pub fn to_polyline(&self) -> LwPolyline {
        let mut lw = LwPolyline::new();
        for seg in &self.segs {
            let (p, _) = seg.at(0.0);
            lw.add_point_with_bulge(Vector2::new(p[0], p[1]), seg.bulge());
        }
        let last = self.segs.last().unwrap();
        let (p, _) = last.at(last.length());
        lw.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
        lw
    }
}

/// Where the lines through a–b and c–d cross.
fn intersect(a: XY, b: XY, c: XY, d: XY) -> Option<XY> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
    Some([a[0] + r[0] * t, a[1] + r[1] * t])
}

fn bounds(pts: impl Iterator<Item = XY>) -> [f64; 4] {
    pts.fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

fn pieces(v: &[(XY, f64)], closed: bool) -> Vec<Seg> {
    let mut out: Vec<Seg> = v.windows(2).map(|w| Seg::from_bulge(w[0].0, w[1].0, w[0].1)).collect();
    if closed && v.len() > 2 {
        let (a, b) = (v[v.len() - 1], v[0]);
        out.push(Seg::from_bulge(a.0, b.0, a.1));
    }
    out
}

/// The axis: the entity with `handle`, or the one selected entity that can
/// be an axis.
pub fn axis(doc: &CadDocument, handle: Option<Handle>) -> Result<(Alignment, Handle, String), String> {
    if let Some(h) = handle {
        let e = doc.get_entity(h).ok_or_else(|| format!("geen element met handle {}", super::api::handle_text(h)))?;
        let al = Alignment::from_entity(e).ok_or_else(|| format!("{} is geen polylijn, lijn of boog", super::api::handle_text(h)))?;
        return Ok((al, h, e.common().layer.clone()));
    }
    let sel: HashSet<Handle> = crate::bridge::selection()?.into_iter().collect();
    let found: Vec<(&EntityType, Alignment)> = doc
        .model_space_entities()
        .filter(|e| sel.contains(&e.common().handle))
        .filter_map(|e| Alignment::from_entity(e).map(|a| (e, a)))
        .collect();
    match found.len() {
        0 => Err("selecteer eerst één as: een polylijn, lijn of boog".into()),
        1 => Ok((found[0].1.clone(), found[0].0.common().handle, found[0].0.common().layer.clone())),
        n => Err(format!("{n} lijnen geselecteerd: selecteer er één als as")),
    }
}

/// The next free "As n" in the drawing.
fn next_name(doc: &CadDocument) -> String {
    let n = doc
        .model_space_entities()
        .filter_map(|e| super::data_value(e, "as"))
        .filter_map(|s| s.strip_prefix("As ").and_then(|n| n.trim().parse::<usize>().ok()))
        .max()
        .unwrap_or(0);
    format!("As {}", n + 1)
}

fn styled(mut e: EntityType, spec: &LayerSpec, k: f64, data: Option<&ocs_plugin_api::host::ExtendedDataRecord>) -> EntityType {
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    c.linetype_scale = k;
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

fn line(a: XY, b: XY, spec: &LayerSpec, k: f64) -> EntityType {
    crate::emit::polylines(&[(vec![a, b], false)], &spec.name, k, None).remove(0)
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

/// Where an axis comes from: a line in the drawing (by handle, or the
/// selection), or points of intersection with curve radii.
pub enum AxisInput {
    Line(Option<Handle>),
    /// PIs in drawing coordinates, radii in drawing units (0 = a corner).
    Pis(Vec<XY>, Vec<f64>),
}

/// `CIV_AS`: an axis on AM-AS with stationing (AM-KILOMETRERING), tangent
/// points (AM-TANGENTPUNT) and radii.
pub fn create(opt: &super::CivielOptions, input: AxisInput, name: Option<String>, start: Option<f64>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let extra: Vec<Handle> = match &input {
        AxisInput::Line(Some(h)) => vec![*h],
        _ => Vec::new(),
    };
    let input_handle = match &input {
        AxisInput::Line(h) => AxisInput::Line(*h),
        AxisInput::Pis(..) => AxisInput::Line(None),
    };
    let doc = super::document(&["as"], &extra)?;
    let (mut al, layer) = match input {
        AxisInput::Line(h) => {
            let (al, found, layer) = axis(&doc, h)?;
            // The selected line: its handle goes in the index too.
            let _ = found;
            (al, layer)
        }
        AxisInput::Pis(pis, radii) => (Alignment::from_pis(&pis, &radii)?, String::new()),
    };
    if let Some(n) = name {
        al.name = n;
    }
    if al.name.is_empty() {
        al.name = next_name(&doc);
    }
    if let Some(s) = start {
        al.start = s;
    }
    let (f, k) = (ctx.f, ctx.k);
    let len = al.length();
    let d = record(&[("as", al.name.clone()), ("beginstation", format!("{:.3}", al.start)), ("lengte_m", format!("{:.3}", len / f))]);
    let axis = ctx.layer("AM", "AS", Element::G)?;
    let axis_t = ctx.layer("AM", "AS", Element::T)?;
    let km = ctx.layer("AM", "KILOMETRERING", Element::G)?;
    let km_t = ctx.layer("AM", "KILOMETRERING", Element::T)?;
    let tp = ctx.layer("AM", "TANGENTPUNT", Element::G)?;
    let tp_t = ctx.layer("AM", "TANGENTPUNT", Element::T)?;
    let mut out = Out::default();
    // The axis itself, unless the line already is one; first, so its
    // handle is the first one written.
    let drawn = !layer.contains("-AM-AS");
    if drawn {
        out.put(&axis, [styled(EntityType::LwPolyline(al.to_polyline()), &axis, k, Some(&d))]);
    }
    let normal = |dir: f64| [-(dir.sin()), dir.cos()];
    // Ticks every `tick` m, long ones with the station every `label_every` m.
    let tick = opt.tick.max(1.0) * f;
    let label_every = opt.label_every.max(opt.tick) * f;
    let mut s = 0.0;
    while s <= len + 1e-9 {
        let (p, dir) = al.at(s);
        let n = normal(dir);
        let st = al.start + s / f;
        let big = ((st * f / label_every).round() * label_every - st * f).abs() < 1e-6 * f.max(1.0);
        let half = if big { 2.5 * k } else { 1.0 * k };
        out.put(&km, [line([p[0] - n[0] * half, p[1] - n[1] * half], [p[0] + n[0] * half, p[1] + n[1] * half], &km, k)]);
        if big {
            let at = [p[0] + n[0] * 3.5 * k, p[1] + n[1] * 3.5 * k];
            let t = ctx.text(at, &station(st), 1.8, crate::emit::readable(dir + FRAC_PI_2), &km_t, None);
            out.put(&km_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        }
        s += tick;
    }
    // Tangent points and corners, with their station.
    for m in al.marks() {
        let (p, dir) = al.at(m.s);
        let n = normal(dir);
        let half = 4.0 * k;
        out.put(&tp, [line([p[0] - n[0] * half, p[1] - n[1] * half], [p[0] + n[0] * half, p[1] + n[1] * half], &tp, k)]);
        let at = [p[0] - n[0] * 5.0 * k, p[1] - n[1] * 5.0 * k];
        let t = ctx.text(at, &format!("{} {}", m.code, station(al.start + m.s / f)), 1.8, crate::emit::readable(dir + FRAC_PI_2), &tp_t, None);
        out.put(&tp_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
    }
    // Radii in the middle of each curve, on the inside.
    let mut run = 0.0;
    let mut curves = 0;
    for seg in &al.segs {
        if let Seg::Arc { r, sweep, .. } = seg {
            let (p, dir) = seg.at(seg.length() / 2.0);
            let n = normal(dir);
            let side = sweep.signum();
            let at = [p[0] + n[0] * side * 3.0 * k, p[1] + n[1] * side * 3.0 * k];
            let t = ctx.text(at, &format!("R = {}", format!("{:.2}", r / f).replace('.', ",")), 1.8, crate::emit::readable(dir), &axis_t, None);
            out.put(&axis_t, [t]);
            curves += 1;
        }
        run += seg.length();
    }
    let _ = run;
    // The name at the start.
    let (p0, dir0) = al.at(0.0);
    let back = [p0[0] - dir0.cos() * 6.0 * k, p0[1] - dir0.sin() * 6.0 * k];
    let t = ctx.text(back, &al.name, 3.5, crate::emit::readable(dir0), &axis_t, Some(&d));
    out.put(&axis_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
    let handles = out.write_handles(&al.name)?;
    let written = handles.len();
    if drawn {
        if let Some(h) = handles.first() {
            super::api::note("as", super::api::handle_text(*h));
            crate::bridge::index_add("as", &al.name, &[*h], true);
        }
    } else if let AxisInput::Line(Some(h)) = &input_handle {
        crate::bridge::index_add("as", &al.name, &[*h], true);
    }
    super::api::note("naam", al.name.clone());
    super::api::note("lengte_m", len / f);
    super::api::note("beginstation", al.start);
    super::api::note("eindstation", al.start + len / f);
    super::api::note("bogen", curves);
    super::api::note("raakpunten", al.marks().iter().map(|m| serde_json::json!({"code": m.code, "station": al.start + m.s / f})).collect::<Vec<_>>());
    Ok(format!(
        "{}: {} m, {} bocht(en), stationering {} tot {} — {written} elementen.",
        al.name,
        format!("{:.2}", len / f).replace('.', ","),
        curves,
        station(al.start),
        station(al.start + len / f)
    ))
}

/// Heights along a list of plan points; `None` where there is no ground.
fn heights(ground: &mut Maaiveld, pts: &[XY], f: f64) -> Vec<Option<f64>> {
    pts.iter().map(|p| ground.height(*p, f)).collect()
}

/// `GIS_AS_LENGTEPROFIEL`: the length profile of the selected axis with its
/// lower-left corner at `at`.
pub fn length_profile(opt: &super::CivielOptions, handle: Option<Handle>, at: Option<XY>, progress: &dyn Fn(String)) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin"], &handle.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = axis(&doc, handle)?;
    let wanted = at;
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let (f, k) = (ctx.f, ctx.k);
    let len = al.length();
    let step = opt.step.max(0.1) * f;
    let mut ss: Vec<f64> = Vec::new();
    let mut s = 0.0;
    while s < len {
        ss.push(s);
        s += step;
    }
    ss.push(len);
    ss.extend(al.marks().into_iter().map(|m| m.s));
    ss.sort_by(f64::total_cmp);
    ss.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    progress(format!("Hoogtes langs {} ({}) …", if al.name.is_empty() { "de as" } else { &al.name }, ground.label()));
    let pts: Vec<XY> = ss.iter().map(|s| al.at(*s).0).collect();
    let zs = heights(&mut ground, &pts, f);
    let known: Vec<f64> = zs.iter().flatten().copied().collect();
    if known.is_empty() {
        return Err(ground.error().unwrap_or_else(|| format!("geen hoogtes langs de as ({})", ground.label())));
    }
    let (zmin, zmax) = known.iter().fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
    let v = opt.exaggeration.max(1.0);
    let hstep = nice_step(((zmax - zmin) / 6.0).max(0.1));
    let base = (zmin / hstep).floor() * hstep - hstep;
    let top = (zmax / hstep).ceil() * hstep + hstep;
    let row = 14.0 * k;
    let bands = 3.0;
    let band = bands * row;
    let head = 42.0 * k;
    // Without a place: in the row of profiles under the project.
    let at = wanted.unwrap_or_else(|| {
        let size = [head + len + 10.0 * k, band + (top - base) * v * f + 20.0 * k];
        let spot = crate::bridge::layout_spot("onder", size, clear_area(&ctx, &al), 30.0 * k);
        [spot[0] + head, spot[1]]
    });
    let gx = |s: f64| at[0] + s;
    let gy = |z: f64| at[1] + band + (z - base) * v * f;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let tp = ctx.layer("AM", "TANGENTPUNT", Element::G)?;
    let mut out = Out::default();
    let pl = |pts: Vec<XY>, closed: bool, spec: &LayerSpec| crate::emit::polylines(&[(pts, closed)], &spec.name, k, None);
    let (x0, x1, y0, y1) = (at[0] - head, gx(len), at[1], gy(top));
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let title = format!("Lengteprofiel {name}");
    // The frame knows its scale: the design profile reads stations and
    // heights back from points drawn in it.
    let frame_data = record(&[
        ("lengteprofiel", title.clone()),
        ("as", al.name.clone()),
        ("x_nul", format!("{:.6}", gx(0.0))),
        ("y_basis", format!("{:.6}", gy(base))),
        ("basis", format!("{base:.4}")),
        ("overhoging", format!("{v}")),
        ("eenheid", format!("{f}")),
        ("beginstation", format!("{:.4}", al.start)),
        ("lengte_m", format!("{:.4}", len / f)),
    ]);
    out.put(&frame, crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], &frame.name, k, Some(&frame_data)));
    for r in 1..=bands as usize {
        out.put(&frame, pl(vec![[x0, y0 + r as f64 * row], [x1, y0 + r as f64 * row]], false, &frame));
    }
    out.put(&frame, pl(vec![[at[0], y0], [at[0], y1]], false, &frame));
    for (r, title) in ["Stationering", "Maaiveld (m)", "Alignement"].iter().enumerate() {
        let y = y0 + band - (r as f64 + 0.5) * row;
        let t = ctx.text([x0 + 2.0 * k, y], title, 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    // Height grid.
    let mut z = base;
    while z <= top + 1e-9 {
        let y = gy(z);
        if z > base + 1e-9 && z < top - 1e-9 {
            out.put(&frame, pl(vec![[at[0], y], [x1, y]], false, &frame));
        }
        if z < top - 1e-9 {
            let t = ctx.text([at[0] - 2.0 * k, y + 1.4 * k], &nap(z, 2), 1.8, 0.0, &frame_t, None);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        }
        z += hstep;
    }
    // Stations with their height, every label distance and at every mark.
    let label = nice_step(20.0 * opt.nlcs.scale / 1000.0).max(opt.step) * f;
    let mut cols: Vec<f64> = al.stations(label, true);
    cols.dedup_by(|a, b| (*a - *b).abs() < 6.0 * k);
    let height_at = |s: f64| -> Option<f64> {
        ss.iter().zip(&zs).min_by(|a, b| (a.0 - s).abs().total_cmp(&(b.0 - s).abs())).and_then(|(_, z)| *z)
    };
    for s in &cols {
        let x = gx(*s);
        out.put(&frame, pl(vec![[x, y0 + row], [x, y0 + band]], false, &frame));
        if *s > 1e-6 && *s < len - 1e-6 {
            out.put(&frame, pl(vec![[x, gy(base)], [x, y1]], false, &frame));
        }
        let rot = FRAC_PI_2;
        let st = station(al.start + s / f);
        out.put(&frame_t, [ctx.text([x - 1.0 * k, y0 + band - 0.5 * row], &st, 1.8, rot, &frame_t, None)]);
        if let Some(z) = height_at(*s) {
            out.put(&frame_t, [ctx.text([x - 1.0 * k, y0 + band - 1.5 * row], &nap(z, 2), 1.8, rot, &frame_t, None)]);
        }
    }
    // Alignment band: straights on the middle line, curves raised (left)
    // or lowered (right) with their radius.
    let mid = y0 + 0.5 * row;
    let mut run = 0.0;
    for seg in &al.segs {
        let l = seg.length();
        match seg {
            Seg::Line { .. } => out.put(&frame, pl(vec![[gx(run), mid], [gx(run + l), mid]], false, &frame)),
            Seg::Arc { r, sweep, .. } => {
                let y = mid + sweep.signum() * 0.3 * row;
                out.put(&frame, pl(vec![[gx(run), mid], [gx(run), y], [gx(run + l), y], [gx(run + l), mid]], false, &frame));
                if l > 20.0 * k {
                    let t = ctx.text([gx(run + l / 2.0), y + sweep.signum() * -0.15 * row], &format!("R {:.0}", r / f), 1.8, 0.0, &frame_t, None);
                    out.put(&frame_t, [t]);
                }
            }
        }
        run += l;
    }
    // Tangent points through the graph.
    for m in al.marks() {
        if m.s > 1e-6 && m.s < len - 1e-6 {
            out.put(&tp, pl(vec![[gx(m.s), y0 + row], [gx(m.s), y1]], false, &tp));
        }
    }
    // The ground line, broken where there is no height.
    let mut run_pts: Vec<XY> = Vec::new();
    for (s, z) in ss.iter().zip(&zs) {
        match z {
            Some(z) => run_pts.push([gx(*s), gy(*z)]),
            None => {
                if run_pts.len() > 1 {
                    out.put(&ground_l, pl(std::mem::take(&mut run_pts), false, &ground_l));
                }
                run_pts.clear();
            }
        }
    }
    if run_pts.len() > 1 {
        out.put(&ground_l, pl(run_pts, false, &ground_l));
    }
    let t = ctx.text([x0, y1 + 9.0 * k], &title, 3.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let sub = format!("{} · horizontaal 1:{} · verticaal 1:{} (overhoging {}×) · lengte {:.1} m", ground.label(), opt.nlcs.scale, (opt.nlcs.scale / v).round(), v, len / f);
    let t = ctx.text([x0, y1 + 4.0 * k], &sub.replace('.', ","), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let handles = out.write_handles(&title)?;
    let n = handles.len();
    ground.note_source();
    if let Some(h) = handles.first() {
        super::api::note("lengteprofiel", super::api::handle_text(*h));
        crate::bridge::index_add("lengteprofiel", &al.name, &[*h], false);
    }
    super::api::note("maaiveld_min", zmin);
    super::api::note("maaiveld_max", zmax);
    super::api::note("plaats", serde_json::json!([at[0], at[1]]));
    Ok(format!("{title}: {} m, {} – {} m ({}), {n} elementen.", format!("{:.1}", len / f).replace('.', ","), nap(zmin, 2), nap(zmax, 2), ground.label()))
}

/// Cross sections every `section_every` m (and at every tangent point)
/// of the selected axis, in a grid with its upper-left corner at `at`; the
/// section lines in plan.
pub fn cross_sections(opt: &super::CivielOptions, handle: Option<Handle>, at: Option<XY>, progress: &dyn Fn(String)) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin"], &handle.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = axis(&doc, handle)?;
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let f = ctx.f;
    let stations = al.stations(opt.section_every.max(1.0) * f, true);
    if stations.len() > 400 {
        return Err(format!("{} dwarsprofielen is te veel: kies een grotere afstand", stations.len()));
    }
    let (left, right) = (opt.left.max(1.0), opt.right.max(1.0));
    let step = (opt.step.max(0.1)).min((left + right) / 20.0);
    let offsets: Vec<f64> = {
        let mut o = Vec::new();
        let mut d = -left;
        while d < right - 1e-9 {
            o.push(d);
            d += step;
        }
        o.push(right);
        o
    };
    progress(format!("{} dwarsprofielen ({}) …", stations.len(), ground.label()));
    let mut views = Vec::new();
    for (i, s) in stations.iter().enumerate() {
        if i % 10 == 0 {
            progress(format!("Dwarsprofiel {}/{}…", i + 1, stations.len()));
        }
        let (p, dir) = al.at(*s);
        // Offsets: negative left of the axis, positive right (looking ahead).
        let right_n = [dir.sin(), -dir.cos()];
        let plan_at = |d: f64| [p[0] + right_n[0] * d * f, p[1] + right_n[1] * d * f];
        let zs: Vec<(f64, Option<f64>)> = offsets.iter().map(|d| (*d, ground.height(plan_at(*d), f))).collect();
        let axis_z = ground.height(p, f);
        views.push(SectionView { s: *s, ground: zs, design: Vec::new(), axis_z, cut: None, fill: None, plan: (plan_at(-left), plan_at(right)) });
    }
    if views.iter().all(|v| v.ground.iter().all(|p| p.1.is_none())) {
        return Err(ground.error().unwrap_or_else(|| format!("geen hoogtes langs de as ({})", ground.label())));
    }
    let mut out = Out::default();
    let label = ground.label();
    let (rows, at) = draw_section_views(&ctx, &al, &views, left, right, at, &label, "", &mut out)?;
    let title = format!("Dwarsprofielen {}", if al.name.is_empty() { "as" } else { &al.name });
    let n = out.write(&title)?;
    ground.note_source();
    super::api::note("dwarsprofielen", views.len());
    super::api::note("plaats", serde_json::json!([at[0], at[1]]));
    Ok(format!("{title}: {} profielen in {rows} rij(en) van {} ({label}), {n} elementen.", views.len(), opt.per_row.max(1)))
}

/// The area views keep clear of: the project area and the axis.
pub fn clear_area(ctx: &Ctx, al: &Alignment) -> [f64; 4] {
    let b = al.bbox();
    match &ctx.site {
        Some(s) => {
            let r = s.drawing_rect(ctx.f);
            [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])]
        }
        None => b,
    }
}

/// One cross section for drawing: ground (offset m, height m), optionally a
/// design line and its cut and fill areas (m²).
pub struct SectionView {
    /// Distance along the axis (drawing units).
    pub s: f64,
    pub ground: Vec<(f64, Option<f64>)>,
    pub design: Vec<(f64, f64)>,
    /// Height written at the axis: the design's, else the ground's.
    pub axis_z: Option<f64>,
    pub cut: Option<f64>,
    pub fill: Option<f64>,
    /// The section line in plan.
    pub plan: (XY, XY),
}

/// Section views in a grid (upper-left corner `at`), each with its height
/// scale, offsets, axis, ground, design, areas and title; the section lines
/// in plan. Returns the number of rows.
#[allow(clippy::too_many_arguments)]
pub fn draw_section_views(ctx: &Ctx, al: &Alignment, views: &[SectionView], left: f64, right: f64, at: Option<XY>, source: &str, what: &str, out: &mut Out) -> Result<(usize, XY), String> {
    let opt = &ctx.opt;
    let (f, k) = (ctx.f, ctx.k);
    let zs_of = |v: &SectionView| -> Vec<f64> { v.ground.iter().filter_map(|p| p.1).chain(v.design.iter().map(|p| p.1)).collect() };
    // One frame size for all: the largest height range of a section.
    let vex = opt.section_exaggeration.max(1.0);
    let span = views
        .iter()
        .filter_map(|v| {
            let z = zs_of(v);
            (!z.is_empty()).then(|| z.iter().cloned().fold(f64::MIN, f64::max) - z.iter().cloned().fold(f64::MAX, f64::min))
        })
        .fold(0.0, f64::max);
    let hstep = nice_step((span / 3.0).max(0.1));
    let rows_h = ((span / hstep).ceil() + 2.0) * hstep;
    let w = (left + right) * f;
    let gh = rows_h * vex * f;
    let band = 7.0 * k;
    let (cell_w, cell_h) = (w + 30.0 * k, gh + band + 22.0 * k);
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let design_l = ctx.layer("GW", "HOOGTE_AFWERKHOOGTE", Element::G)?;
    let axis_l = ctx.layer("AM", "AS", Element::G)?;
    let sec = ctx.layer("AL", "VERWIJZING_DOORSNEDE", Element::G)?;
    let sec_t = ctx.layer("AL", "VERWIJZING_DOORSNEDE", Element::T)?;
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    let per_row = opt.per_row.max(1);
    // Without a place: a column right of the project, beside earlier ones.
    let at = at.unwrap_or_else(|| {
        let size = [per_row.min(views.len()) as f64 * cell_w + 20.0 * k, views.len().div_ceil(per_row) as f64 * cell_h + 12.0 * k];
        let spot = crate::bridge::layout_spot("rechts", size, clear_area(ctx, al), 40.0 * k);
        [spot[0], spot[1] - 12.0 * k]
    });
    for (i, sct) in views.iter().enumerate() {
        let (col, row) = (i % per_row, i / per_row);
        let ox = at[0] + 20.0 * k + col as f64 * cell_w;
        let oy = at[1] - (row as f64 + 1.0) * cell_h + 10.0 * k;
        let z = zs_of(sct);
        let st = station(al.start + sct.s / f);
        let zlo = if z.is_empty() { 0.0 } else { z.iter().cloned().fold(f64::MAX, f64::min) };
        let base = (zlo / hstep).floor() * hstep - hstep;
        let gx = |d: f64| ox + (d + left) * f;
        let gy = |z: f64| oy + band + (z - base) * vex * f;
        let (x0, x1, y0, y1) = (ox, ox + w, oy, oy + band + gh);
        out.put(&frame, pl(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]], &frame));
        out.put(&frame, pl(vec![[x0, y0 + band], [x1, y0 + band]], &frame));
        // Height lines with labels on the left.
        let mut zz = base;
        while zz <= base + rows_h + 1e-9 {
            let y = gy(zz);
            if zz > base + 1e-9 && zz < base + rows_h - 1e-9 {
                out.put(&frame, pl(vec![[x0, y], [x0 + 1.5 * k, y]], &frame));
                out.put(&frame, pl(vec![[x1 - 1.5 * k, y], [x1, y]], &frame));
                let t = ctx.text([x0 - 1.5 * k, y], &nap(zz, 2), 1.8, 0.0, &frame_t, None);
                out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
            }
            zz += hstep;
        }
        // Offsets in the band.
        let dstep = nice_step((left + right) / 8.0);
        let mut d = -(left / dstep).floor() * dstep;
        while d <= right + 1e-9 {
            out.put(&frame, pl(vec![[gx(d), y0 + band], [gx(d), y0 + band - 1.5 * k]], &frame));
            out.put(&frame_t, [ctx.text([gx(d), y0 + band / 2.0], &format!("{d:.0}"), 1.8, 0.0, &frame_t, None)]);
            d += dstep;
        }
        // Axis, ground and design.
        out.put(&axis_l, pl(vec![[gx(0.0), y0 + band], [gx(0.0), y1]], &axis_l));
        let mut run: Vec<XY> = Vec::new();
        for (d, z) in &sct.ground {
            match z {
                Some(z) => run.push([gx(*d), gy(*z)]),
                None => {
                    if run.len() > 1 {
                        out.put(&ground_l, pl(std::mem::take(&mut run), &ground_l));
                    }
                    run.clear();
                }
            }
        }
        if run.len() > 1 {
            out.put(&ground_l, pl(run, &ground_l));
        }
        if sct.design.len() > 1 {
            out.put(&design_l, pl(sct.design.iter().map(|(d, z)| [gx(*d), gy(*z)]).collect(), &design_l));
        }
        if let Some(zc) = sct.axis_z {
            let t = ctx.text([gx(0.0) + 1.0 * k, gy(zc) + 2.0 * k], &nap(zc, 2), 1.8, 0.0, &frame_t, None);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        }
        let t = ctx.text([(x0 + x1) / 2.0, y1 + 4.0 * k], &format!("Dwarsprofiel {st}"), 2.5, 0.0, &frame_t, None);
        out.put(&frame_t, [t]);
        if let (Some(c), Some(fl)) = (sct.cut, sct.fill) {
            let t = ctx.text([x0 + 2.0 * k, y1 - 3.0 * k], &format!("afgraven {} m² · ophogen {} m²", format!("{c:.2}").replace('.', ","), format!("{fl:.2}").replace('.', ",")), 1.8, 0.0, &frame_t, None);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        }
        // In plan: the section line and its station.
        let d = record(&[("dwarsprofiel", st.clone()), ("as", al.name.clone())]);
        out.put(&sec, crate::emit::polylines(&[(vec![sct.plan.0, sct.plan.1], false)], &sec.name, k, Some(&d)));
        let (a, b) = sct.plan;
        let ang = (b[1] - a[1]).atan2(b[0] - a[0]);
        let t = ctx.text([a[0] - ang.cos() * 2.0 * k, a[1] - ang.sin() * 2.0 * k], &st, 1.8, crate::emit::readable(ang), &sec_t, None);
        out.put(&sec_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
    }
    let rows = views.len().div_ceil(per_row);
    let title = format!("Dwarsprofielen {}{what}", if al.name.is_empty() { "as" } else { &al.name });
    let t = ctx.text([at[0], at[1] + 6.0 * k], &title, 3.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let sub = format!(
        "{source} · {left} m links, {right} m rechts · om de {} m · horizontaal 1:{} · verticaal 1:{} (overhoging {vex}×)",
        opt.section_every,
        opt.nlcs.scale,
        (opt.nlcs.scale / vex).round(),
    );
    let t = ctx.text([at[0], at[1] + 2.0 * k], &sub.replace('.', ","), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    Ok((rows, at))
}

/// `CIV_AS_OFFSET`: lines beside the selected axis at the offsets of the
/// settings (offset alignments), on AM-STRING_HULP.
pub fn offsets(opt: &super::CivielOptions, handle: Option<Handle>) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&[], &handle.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = axis(&doc, handle)?;
    if opt.offsets.is_empty() {
        return Err("geen afstanden ingesteld (Instellingen → As verschuiven)".into());
    }
    let spec = ctx.layer("AM", "STRING_HULP", Element::G)?;
    let spec_t = ctx.layer("AM", "STRING_HULP", Element::T)?;
    let mut out = Out::default();
    let mut made = Vec::new();
    for d in &opt.offsets {
        let Some(lw) = al.offset_polyline(d * ctx.f) else { continue };
        let side = if *d < 0.0 { "links" } else { "rechts" };
        let label = format!("{} m {side}", format!("{:.2}", d.abs()).replace('.', ","));
        let data = record(&[("as", al.name.clone()), ("verschuiving_m", format!("{d:.3}"))]);
        out.put(&spec, [styled(EntityType::LwPolyline(lw), &spec, ctx.k, Some(&data))]);
        // Name it at its start.
        let (p, dir) = al.at(0.0);
        let at = [p[0] + dir.sin() * d * ctx.f - dir.cos() * 2.0 * ctx.k, p[1] - dir.cos() * d * ctx.f - dir.sin() * 2.0 * ctx.k];
        let t = ctx.text(at, &label, 1.8, crate::emit::readable(dir), &spec_t, None);
        out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        made.push(label);
    }
    if out.is_empty() {
        return Err("de verschuivingen vallen binnen de kleinste boogstraal".into());
    }
    let handles = out.write_handles("As verschuiven")?;
    let n = handles.len();
    super::api::note("lijnen", handles.iter().take(made.len()).map(|h| super::api::handle_text(*h)).collect::<Vec<_>>());
    Ok(format!("{}: {} verschoven lijn(en) ({}) — {n} elementen op {}.", if al.name.is_empty() { "As" } else { &al.name }, made.len(), made.join(", "), spec.name))
}

/// Station and offset of a point for the command line: "0+123,45 · 3,20 m rechts".
pub fn station_offset_text(al: &Alignment, p: XY, f: f64) -> String {
    let (s, off) = al.station_offset(p);
    let side = if off.abs() < 0.005 * f { "op de as".to_string() } else { format!("{} m {}", format!("{:.2}", off.abs() / f).replace('.', ","), if off < 0.0 { "links" } else { "rechts" }) };
    format!("{} {} · {side}", if al.name.is_empty() { "As" } else { &al.name }, station(al.start + s / f))
}

/// A point of intersection of the design profile with its vertical curve.
#[derive(Clone, Debug, PartialEq)]
pub struct Pvi {
    /// Distance from the start of the axis (m) and height (m).
    pub s: f64,
    pub z: f64,
    /// Length of the parabolic curve (m), 0 at the ends.
    pub l: f64,
}

/// Curve lengths: the wanted length, but never longer than a neighbouring
/// straight, so curves do not overlap.
pub fn fit_curves(points: &[(f64, f64)], wanted: f64) -> Vec<Pvi> {
    let n = points.len();
    points
        .iter()
        .enumerate()
        .map(|(i, (s, z))| {
            let l = if i == 0 || i + 1 == n { 0.0 } else { wanted.min(s - points[i - 1].0).min(points[i + 1].0 - s).max(0.0) };
            Pvi { s: *s, z: *z, l }
        })
        .collect()
}

fn grade(a: &Pvi, b: &Pvi) -> f64 {
    (b.z - a.z) / (b.s - a.s).max(1e-9)
}

/// Height of the design profile at `s` (m from the start of the axis).
pub fn design_z(pvis: &[Pvi], s: f64) -> f64 {
    for j in 1..pvis.len().saturating_sub(1) {
        let p = &pvis[j];
        if p.l > 0.0 && (s - p.s).abs() <= p.l / 2.0 {
            let (g1, g2) = (grade(&pvis[j - 1], p), grade(p, &pvis[j + 1]));
            let x = s - (p.s - p.l / 2.0);
            return p.z - g1 * p.l / 2.0 + g1 * x + (g2 - g1) / (2.0 * p.l) * x * x;
        }
    }
    let i = pvis.windows(2).position(|w| s <= w[1].s).unwrap_or(pvis.len().saturating_sub(2));
    let (a, b) = (&pvis[i], &pvis[i + 1]);
    a.z + grade(a, b) * (s - a.s)
}

fn data_f64(e: &EntityType, key: &str) -> Option<f64> {
    super::data_value(e, key).and_then(|v| v.parse().ok())
}

/// `CIV_ONTWERPPROFIEL`: the selected polyline, drawn in a length profile,
/// becomes the design profile: its vertices are the PVIs, with parabolic
/// curves between the grades.
pub fn design_profile(opt: &super::CivielOptions, drawn_line: Option<Handle>, axis_h: Option<Handle>, pvi: Option<Vec<(f64, f64)>>) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["lengteprofiel", "ontwerpprofiel"], &axis_h.into_iter().chain(drawn_line).collect::<Vec<_>>())?;
    let frames: Vec<(&EntityType, [f64; 4])> = doc
        .model_space_entities()
        .filter(|e| super::data_value(e, "lengteprofiel").is_some())
        .filter_map(|e| match e {
            EntityType::LwPolyline(l) => Some((e, bounds(l.vertices.iter().map(|v| [v.location.x, v.location.y])))),
            _ => None,
        })
        .collect();
    let (frame, pvi_mode, drawn): (&EntityType, bool, Vec<(f64, f64)>) = match pvi {
        // PVIs as station:height, in the newest length profile of the axis.
        Some(list) => {
            let (al, _, _) = axis(&doc, axis_h)?;
            let (frame, _) = frames
                .iter()
                .rev()
                .find(|(e, _)| super::data_value(e, "as").as_deref() == Some(al.name.as_str()))
                .ok_or_else(|| format!("maak eerst een lengteprofiel van {} (CIV_LENGTEPROFIEL)", if al.name.is_empty() { "deze as" } else { &al.name }))?;
            (*frame, true, list)
        }
        // A line drawn in a length profile.
        None => {
            let sel: HashSet<Handle> = match drawn_line {
                Some(h) => [h].into_iter().collect(),
                None => crate::bridge::selection()?.into_iter().collect(),
            };
            let pts: Vec<XY> = doc
                .model_space_entities()
                .filter(|e| sel.contains(&e.common().handle))
                .find_map(|e| match e {
                    EntityType::LwPolyline(l) if l.vertices.len() >= 2 && super::data_value(e, "lengteprofiel").is_none() => Some(l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect()),
                    EntityType::Line(l) => Some(vec![[l.start.x, l.start.y], [l.end.x, l.end.y]]),
                    _ => None,
                })
                .ok_or("selecteer de ontwerplijn: een polylijn getekend in een lengteprofiel (of geef PVI=station:hoogte;…)")?;
            let (frame, _) = frames
                .iter()
                .find(|(_, b)| pts.iter().all(|p| p[0] >= b[0] - 1e-6 && p[0] <= b[2] + 1e-6 && p[1] >= b[1] - 1e-6 && p[1] <= b[3] + 1e-6))
                .ok_or("de lijn ligt niet in een lengteprofiel van Open Infra (Profielen → Lengteprofiel)")?;
            (*frame, false, pts.iter().map(|p| (p[0], p[1])).collect())
        }
    };
    let get = |k: &str| data_f64(frame, k).ok_or_else(|| format!("het lengteprofiel mist '{k}'"));
    let (x_nul, y_basis, basis, v, f, start, length) = (get("x_nul")?, get("y_basis")?, get("basis")?, get("overhoging")?, get("eenheid")?, get("beginstation")?, get("lengte_m")?);
    let as_name = super::data_value(frame, "as").unwrap_or_default();
    let mut pts: Vec<(f64, f64)> = if pvi_mode {
        drawn.iter().map(|(st, z)| ((st - start).clamp(0.0, length), *z)).collect()
    } else {
        drawn.iter().map(|(x, y)| (((x - x_nul) / f).clamp(0.0, length), basis + (y - y_basis) / (v * f))).collect()
    };
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-3);
    if pts.len() < 2 {
        return Err("de ontwerplijn heeft minstens twee punten op verschillende stations nodig".into());
    }
    let pvis = fit_curves(&pts, opt.vertical_curve.max(0.0));
    let k = ctx.k;
    let spec = ctx.layer("AM", "AS", Element::G)?;
    let spec_t = ctx.layer("AM", "AS", Element::T)?;
    let to = |s: f64, z: f64| -> XY { [x_nul + s * f, y_basis + (z - basis) * v * f] };
    // The line itself, sampled finely in the curves.
    let (s0, s1) = (pvis[0].s, pvis[pvis.len() - 1].s);
    let mut ss: Vec<f64> = Vec::new();
    let step = (opt.step.max(0.1)).min((s1 - s0) / 4.0).max(0.01);
    let mut s = s0;
    while s < s1 {
        ss.push(s);
        s += step;
    }
    ss.push(s1);
    for p in &pvis {
        ss.extend([p.s - p.l / 2.0, p.s + p.l / 2.0]);
    }
    ss.retain(|s| *s >= s0 && *s <= s1);
    ss.sort_by(f64::total_cmp);
    ss.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let curve: Vec<XY> = ss.iter().map(|s| to(*s, design_z(&pvis, *s))).collect();
    let pvi_text = pvis.iter().map(|p| format!("{:.3}:{:.3}", p.s, p.z)).collect::<Vec<_>>().join(";");
    let data = record(&[("ontwerpprofiel", format!("Ontwerpprofiel {as_name}")), ("as", as_name.clone()), ("pvi", pvi_text), ("boog_m", format!("{}", opt.vertical_curve))]);
    let mut out = Out::default();
    out.put(&spec, crate::emit::polylines(&[(curve, false)], &spec.name, k, Some(&data)));
    // Grades along the straights.
    for w in pvis.windows(2) {
        let g = grade(&w[0], &w[1]);
        let (a, b) = (w[0].s + w[0].l / 2.0, w[1].s - w[1].l / 2.0);
        let mid = (a + b) / 2.0;
        let ang = (g * v).atan();
        let p = to(mid, design_z(&pvis, mid));
        let at = [p[0] - ang.sin() * 2.5 * k, p[1] + ang.cos() * 2.5 * k];
        let label = format!("{:+.2} %", g * 100.0).replace('.', ",");
        out.put(&spec_t, [ctx.text(at, &label, 1.8, ang, &spec_t, None)]);
    }
    // PVIs: a mark at the tangent point and its station, height and curve.
    for (j, p) in pvis.iter().enumerate() {
        let at = to(p.s, p.z);
        out.put(&spec, [line(at, [at[0], at[1] + 6.0 * k], &spec, k)]);
        let mut label = format!("{} {}", station(start + p.s), nap(p.z, 2));
        if p.l > 0.0 {
            let (g1, g2) = (grade(&pvis[j - 1], p), grade(p, &pvis[j + 1]));
            let dg = (g2 - g1).abs();
            let kind = if g2 < g1 { "topboog" } else { "voetboog" };
            if dg > 1e-9 {
                label.push_str(&format!(" · {kind} L {:.0} R {:.0}", p.l, p.l / dg));
            }
        }
        let t = ctx.text([at[0] - 1.0 * k, at[1] + 7.0 * k], &label, 1.8, FRAC_PI_2, &spec_t, None);
        out.put(&spec_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    let handles = out.write_handles("Ontwerpprofiel")?;
    let n = handles.len();
    if let Some(h) = handles.first() {
        super::api::note("ontwerpprofiel", super::api::handle_text(*h));
        crate::bridge::index_add("ontwerpprofiel", &as_name, &[*h], true);
    }
    super::api::note("pvi", pvis.iter().map(|p| serde_json::json!({"station": start + p.s, "hoogte": p.z, "boog_m": p.l})).collect::<Vec<_>>());
    super::api::note("hellingen_pct", pvis.windows(2).map(|w| grade(&w[0], &w[1]) * 100.0).collect::<Vec<_>>());
    let grades: Vec<String> = pvis.windows(2).map(|w| format!("{:+.2} %", grade(&w[0], &w[1]) * 100.0).replace('.', ",")).collect();
    Ok(format!("Ontwerpprofiel {as_name}: {} PVI's, hellingen {} — {n} elementen.", pvis.len(), grades.join(" / ")))
}

/// Size of the cross-section grid for the preview: width, height (drawing units).
pub fn sections_extent(al: &Alignment, opt: &super::CivielOptions, f: f64) -> (f64, f64) {
    let k = opt.nlcs.scale / 1000.0 * f;
    let n = al.stations(opt.section_every.max(1.0) * f, true).len();
    let per_row = opt.per_row.max(1);
    let w = (opt.left + opt.right) * f + 30.0 * k;
    (per_row.min(n) as f64 * w, n.div_ceil(per_row) as f64 * (10.0 * opt.section_exaggeration * f + 30.0 * k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: XY, b: XY) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn bulges_become_arcs() {
        // Bulge 1 from (0,0) to (2,0): a half circle through (1,-1).
        let s = Seg::from_bulge([0.0, 0.0], [2.0, 0.0], 1.0);
        assert!((s.length() - PI).abs() < 1e-9);
        assert!(close(s.at(PI / 2.0).0, [1.0, -1.0]));
        assert!(close(s.at(PI).0, [2.0, 0.0]));
        assert!((s.bulge() - 1.0).abs() < 1e-9);
        // Bulge -1 goes the other way, through (1,1).
        let s = Seg::from_bulge([0.0, 0.0], [2.0, 0.0], -1.0);
        assert!(close(s.at(PI / 2.0).0, [1.0, 1.0]));
    }

    #[test]
    fn an_axis_with_a_curve() {
        // Straight 100 m east, a quarter circle R 50 to the left, 100 m north.
        let b = (PI / 8.0).tan();
        let mut lw = LwPolyline::new();
        lw.add_point_with_bulge(Vector2::new(0.0, 0.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(100.0, 0.0), b);
        lw.add_point_with_bulge(Vector2::new(150.0, 50.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(150.0, 150.0), 0.0);
        let al = Alignment::from_entity(&EntityType::LwPolyline(lw)).unwrap();
        let len = 200.0 + 25.0 * PI;
        assert!((al.length() - len).abs() < 1e-9);
        let codes: Vec<&str> = al.marks().iter().map(|m| m.code).collect();
        assert_eq!(codes, ["BA", "BB", "EB", "EA"]);
        // Halfway the curve: on the circle around (100, 50), heading north-east.
        let (p, dir) = al.at(100.0 + 12.5 * PI);
        assert!(close(p, [100.0 + 50.0 * (PI / 4.0).sin(), 50.0 - 50.0 * (PI / 4.0).cos()]));
        assert!((dir - PI / 4.0).abs() < 1e-9);
        // Stations every 50 m plus the tangent points and the end.
        let st = al.stations(50.0, true);
        assert_eq!(st[..4], [0.0, 50.0, 100.0, 150.0]);
        assert!(st.contains(&(100.0 + 25.0 * PI)));
        assert_eq!(*st.last().unwrap(), len);
        // And back to a polyline with the same bulge.
        let back = al.to_polyline();
        assert!((back.vertices[1].bulge - b).abs() < 1e-9);
    }

    #[test]
    fn offsets_and_stations_of_points() {
        let b = (PI / 8.0).tan();
        let mut lw = LwPolyline::new();
        lw.add_point_with_bulge(Vector2::new(0.0, 0.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(100.0, 0.0), b);
        lw.add_point_with_bulge(Vector2::new(150.0, 50.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(150.0, 150.0), 0.0);
        let al = Alignment::from_entity(&EntityType::LwPolyline(lw)).unwrap();
        // 3 m right of the curve (outside of a left turn): radius 53.
        let right = Alignment::from_entity(&EntityType::LwPolyline(al.offset_polyline(3.0).unwrap())).unwrap();
        assert!((right.length() - (200.0 + 26.5 * PI)).abs() < 1e-6);
        let left = Alignment::from_entity(&EntityType::LwPolyline(al.offset_polyline(-3.0).unwrap())).unwrap();
        assert!((left.length() - (200.0 + 23.5 * PI)).abs() < 1e-6);
        // A point 2 m left of station 50, one 4 m right on the curve's middle.
        let (s, off) = al.station_offset([50.0, 2.0]);
        assert!((s - 50.0).abs() < 1e-9 && (off + 2.0).abs() < 1e-9);
        let (p, dir) = al.at(100.0 + 12.5 * PI);
        let q = [p[0] + dir.sin() * 4.0, p[1] - dir.cos() * 4.0];
        let (s, off) = al.station_offset(q);
        assert!((s - (100.0 + 12.5 * PI)).abs() < 1e-6 && (off - 4.0).abs() < 1e-9);
        // Corners of straights meet in their offset.
        let mut lw = LwPolyline::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)] {
            lw.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        let al = Alignment::from_entity(&EntityType::LwPolyline(lw)).unwrap();
        let o = al.offset_polyline(1.0).unwrap();
        let pts: Vec<XY> = o.vertices.iter().map(|v| [v.location.x, v.location.y]).collect();
        assert_eq!(pts, vec![[0.0, -1.0], [11.0, -1.0], [11.0, 10.0]]);
    }

    #[test]
    fn a_design_profile_with_a_crest() {
        // Up 2 % for 100 m, down 1 % for 100 m; a 50 m curve at the top.
        let pvis = fit_curves(&[(0.0, 0.0), (100.0, 2.0), (200.0, 1.0)], 50.0);
        assert_eq!(pvis[1].l, 50.0);
        // On the straights, the grade.
        assert!((design_z(&pvis, 50.0) - 1.0).abs() < 1e-9);
        assert!((design_z(&pvis, 150.0) - 1.5).abs() < 1e-9);
        // The curve starts and ends on them, and stays below the PVI: e = Δg·L/8.
        assert!((design_z(&pvis, 75.0) - 1.5).abs() < 1e-9);
        assert!((design_z(&pvis, 125.0) - 1.75).abs() < 1e-9);
        assert!((design_z(&pvis, 100.0) - (2.0 - 0.03 * 50.0 / 8.0)).abs() < 1e-9);
        // A curve never runs past the next PVI.
        let short = fit_curves(&[(0.0, 0.0), (20.0, 1.0), (200.0, 0.0)], 50.0);
        assert_eq!(short[1].l, 20.0);
    }

    #[test]
    fn an_axis_by_layout() {
        // East 100, then north: R 50 at the corner, a quarter circle.
        let al = Alignment::from_pis(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], &[50.0]).unwrap();
        assert_eq!(al.segs.len(), 3);
        assert!((al.length() - (50.0 + 25.0 * PI + 50.0)).abs() < 1e-9);
        match &al.segs[1] {
            Seg::Arc { c, r, sweep, .. } => {
                assert!(close(*c, [50.0, 50.0]) && (*r - 50.0).abs() < 1e-9 && (sweep - PI / 2.0).abs() < 1e-9);
            }
            s => panic!("{s:?}"),
        }
        // Ends on the last PI, tangent at both ends of the curve.
        assert!(close(al.at(al.length()).0, [100.0, 100.0]));
        let codes: Vec<&str> = al.marks().iter().map(|m| m.code).collect();
        assert_eq!(codes, ["BA", "BB", "EB", "EA"]);
        // A right turn has its centre on the right; too large a radius is refused.
        let right = Alignment::from_pis(&[[0.0, 0.0], [100.0, 0.0], [100.0, -100.0]], &[30.0]).unwrap();
        assert!(matches!(right.segs[1], Seg::Arc { sweep, .. } if sweep < 0.0));
        assert!(Alignment::from_pis(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], &[200.0]).is_err());
        // Radius 0: a corner.
        assert_eq!(Alignment::from_pis(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], &[0.0]).unwrap().marks()[1].code, "HP");
    }

    #[test]
    fn corners_between_straights() {
        let mut lw = LwPolyline::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (20.0, 10.0)] {
            lw.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        let al = Alignment::from_entity(&EntityType::LwPolyline(lw)).unwrap();
        let codes: Vec<&str> = al.marks().iter().map(|m| m.code).collect();
        assert_eq!(codes, ["BA", "HP", "HP", "EA"]);
    }
}
