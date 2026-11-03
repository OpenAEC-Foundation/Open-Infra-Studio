//! Parcels and parcel sizing: a plot, a closed polyline, split into parcels
//! by dividing lines perpendicular to a street line or parallel to a given
//! direction. Three ways to size them: N equal parcels, a target area
//! (`OPP=`) or a frontage width (`FRONT=`); area and frontage together act as
//! minimums, as subdivision tools usually treat them. Each dividing line is
//! found by bisection on the exact (clipped polygon) area, so the areas are
//! exact to well under 0.01 m².
//!
//! The engine works in metres around a local origin; the drawing side reads
//! the plot and street, draws the parcels on KG-GRENS_PERCEEL with their
//! number and area, object data and a CSV table.

use std::collections::HashSet;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{EntityType, Handle};

use super::{record, Ctx, Out};
use crate::gis::crs::XY;
use crate::nlcs::Element;

/// A message in the interface language (Dutch or English).
macro_rules! t {
    ($nl:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

fn tr(nl: &str, en: &str) -> String {
    crate::lang::tr(nl, en).to_string()
}

fn sub(a: XY, b: XY) -> XY {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: XY, b: XY) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: XY) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: XY) -> XY {
    let l = len(a).max(1e-12);
    [a[0] / l, a[1] / l]
}

/// Signed area of a ring (counter-clockwise > 0).
pub fn signed_area(r: &[XY]) -> f64 {
    let n = r.len();
    if n < 3 {
        return 0.0;
    }
    // Around the first vertex: large coordinates would cost precision.
    let o = r[0];
    let mut a = 0.0;
    for i in 1..n - 1 {
        let (p, q) = (sub(r[i], o), sub(r[i + 1], o));
        a += p[0] * q[1] - q[0] * p[1];
    }
    a / 2.0
}

pub fn area(r: &[XY]) -> f64 {
    signed_area(r).abs()
}

pub fn perimeter(r: &[XY]) -> f64 {
    (0..r.len()).map(|i| len(sub(r[(i + 1) % r.len()], r[i]))).sum()
}

/// The part of a ring where `(p − o)·n ≤ 0` (Sutherland–Hodgman against one
/// line). A concave ring cut in two comes back as one ring with a
/// zero-width bridge; its area is still exact.
pub fn clip_half(r: &[XY], o: XY, n: XY) -> Vec<XY> {
    let side = |p: XY| dot(sub(p, o), n);
    let mut out = Vec::with_capacity(r.len() + 2);
    for i in 0..r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        let (sa, sb) = (side(a), side(b));
        if sa <= 0.0 {
            out.push(a);
        }
        if (sa < 0.0 && sb > 0.0) || (sa > 0.0 && sb < 0.0) {
            let t = sa / (sa - sb);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    // Drop repeated points.
    out.dedup_by(|a, b| len(sub(*a, *b)) < 1e-12);
    if out.len() > 1 && len(sub(out[0], *out.last().unwrap())) < 1e-12 {
        out.pop();
    }
    if out.len() < 3 {
        Vec::new()
    } else {
        out
    }
}

/// Where the dividing lines go: a family of lines with a parameter `s`.
#[derive(Clone, Debug)]
pub enum Sweep {
    /// Lines perpendicular to `u` (unit): line `s` is {p : (p − o)·u = s}.
    Direction { o: XY, u: XY },
    /// Lines perpendicular to a street polyline; `s` is the station along it
    /// (extended straight beyond its ends).
    Street { line: Vec<XY> },
}

impl Sweep {
    /// Dividing lines parallel to `angle` (radians from the x axis).
    pub fn parallel_to(angle: f64) -> Sweep {
        Sweep::Direction { o: [0.0, 0.0], u: [-angle.sin(), angle.cos()] }
    }

    /// The point and forward direction of line `s`: the part "before" it is
    /// {p : (p − point)·dir ≤ 0}.
    pub fn line(&self, s: f64) -> (XY, XY) {
        match self {
            Sweep::Direction { o, u } => ([o[0] + u[0] * s, o[1] + u[1] * s], *u),
            Sweep::Street { line } => {
                // The direction turns smoothly from vertex to vertex (the
                // tangent at a vertex halves the bend), so the dividing line
                // moves without jumps and the area before it is continuous.
                let n = line.len();
                let dirs: Vec<XY> = (0..n - 1).map(|i| unit(sub(line[i + 1], line[i]))).collect();
                let tangent = |v: usize| -> XY {
                    if v == 0 {
                        dirs[0]
                    } else if v >= n - 1 {
                        dirs[n - 2]
                    } else {
                        unit([dirs[v - 1][0] + dirs[v][0], dirs[v - 1][1] + dirs[v][1]])
                    }
                };
                let mut at = 0.0;
                for i in 0..n - 1 {
                    let l = len(sub(line[i + 1], line[i]));
                    if s <= at + l || i == n - 2 {
                        let t = s - at;
                        let u = (t / l.max(1e-12)).clamp(0.0, 1.0);
                        let (ta, tb) = (tangent(i), tangent(i + 1));
                        let d = unit([ta[0] * (1.0 - u) + tb[0] * u, ta[1] * (1.0 - u) + tb[1] * u]);
                        return ([line[i][0] + dirs[i][0] * t, line[i][1] + dirs[i][1] * t], d);
                    }
                    at += l;
                }
                (line[0], dirs[0])
            }
        }
    }

    /// The parameter of a point (its foot on the street, or its projection).
    pub fn station(&self, p: XY) -> f64 {
        match self {
            Sweep::Direction { o, u } => dot(sub(p, *o), *u),
            Sweep::Street { line } => {
                let mut best = (f64::MAX, 0.0);
                let mut at = 0.0;
                let n = line.len();
                for i in 0..n - 1 {
                    let (a, b) = (line[i], line[i + 1]);
                    let l = len(sub(b, a));
                    if l < 1e-12 {
                        continue;
                    }
                    let d = unit(sub(b, a));
                    let mut t = dot(sub(p, a), d);
                    // Beyond the ends the street runs on straight.
                    if i > 0 {
                        t = t.max(0.0);
                    }
                    if i < n - 2 {
                        t = t.min(l);
                    }
                    let foot = [a[0] + d[0] * t, a[1] + d[1] * t];
                    let dist = len(sub(p, foot));
                    if dist < best.0 {
                        best = (dist, at + t);
                    }
                    at += l;
                }
                best.1
            }
        }
    }

    /// The plot between lines `s0` and `s1`.
    pub fn piece(&self, plot: &[XY], s0: f64, s1: f64) -> Vec<XY> {
        let (p1, d1) = self.line(s1);
        let before = clip_half(plot, p1, d1);
        let (p0, d0) = self.line(s0);
        clip_half(&before, p0, [-d0[0], -d0[1]])
    }

    /// Area of the plot before line `s`.
    fn area_before(&self, plot: &[XY], s: f64) -> f64 {
        let (p, d) = self.line(s);
        area(&clip_half(plot, p, d))
    }
}

/// The edges of the plot along the street: within 2 m of the nearest one
/// and at most 30° from the street's direction. Without a street, none.
pub fn front_edges(plot: &[XY], sweep: &Sweep) -> Vec<(XY, XY)> {
    let Sweep::Street { .. } = sweep else { return Vec::new() };
    let n = plot.len();
    let edges: Vec<(XY, XY, f64, f64)> = (0..n)
        .filter_map(|i| {
            let (a, b) = (plot[i], plot[(i + 1) % n]);
            let l = len(sub(b, a));
            (l > 1e-9).then(|| {
                let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                let s = sweep.station(m);
                let (foot, dir) = sweep.line(s);
                let cos = dot(unit(sub(b, a)), dir).abs();
                (a, b, len(sub(m, foot)), cos)
            })
        })
        .collect();
    let near = edges.iter().filter(|e| e.3 >= 30f64.to_radians().cos()).map(|e| e.2).fold(f64::MAX, f64::min);
    edges.into_iter().filter(|e| e.3 >= 30f64.to_radians().cos() && e.2 <= near + 2.0).map(|e| (e.0, e.1)).collect()
}

/// Length of the front edges before line `s`.
fn front_before(front: &[(XY, XY)], sweep: &Sweep, s: f64) -> f64 {
    let (p, d) = sweep.line(s);
    front
        .iter()
        .map(|(a, b)| {
            let (sa, sb) = (dot(sub(*a, p), d), dot(sub(*b, p), d));
            let l = len(sub(*b, *a));
            if sa <= 0.0 && sb <= 0.0 {
                l
            } else if sa > 0.0 && sb > 0.0 {
                0.0
            } else {
                l * if sa <= 0.0 { sa / (sa - sb) } else { sb / (sb - sa) }
            }
        })
        .sum()
}

/// The frontage of a parcel: its edges that lie on the plot's front edges
/// (without a street: its width across the dividing lines).
pub fn frontage(parcel: &[XY], front: &[(XY, XY)], s0: f64, s1: f64) -> f64 {
    if front.is_empty() {
        return s1 - s0;
    }
    let n = parcel.len();
    let mut total = 0.0;
    for i in 0..n {
        let (a, b) = (parcel[i], parcel[(i + 1) % n]);
        let on = front.iter().any(|(p, q)| {
            let d = unit(sub(*q, *p));
            let off = |x: XY| (sub(x, *p)[0] * d[1] - sub(x, *p)[1] * d[0]).abs();
            let t = |x: XY| dot(sub(x, *p), d);
            let l = len(sub(*q, *p));
            off(a) < 1e-6 && off(b) < 1e-6 && t(a) > -1e-6 && t(a) < l + 1e-6 && t(b) > -1e-6 && t(b) < l + 1e-6
        });
        if on {
            total += len(sub(b, a));
        }
    }
    total
}

/// What happens with what is left after the last full parcel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remainder {
    /// Added to the last parcel (the usual default).
    Last,
    /// A parcel of its own.
    Own,
    /// Spread: all parcels equal, as many as fit.
    Spread,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing {
    Equal(usize),
    /// Minimum area (m²) and/or minimum frontage (m).
    Min { area: Option<f64>, front: Option<f64>, rest: Remainder },
}

#[derive(Clone, Debug)]
pub struct Parcel {
    pub ring: Vec<XY>,
    pub area: f64,
    pub perimeter: f64,
    pub frontage: f64,
    pub s0: f64,
    pub s1: f64,
}

/// Bisection for the line `s` in [lo, hi] where the monotone `f(s)` reaches
/// `target`.
fn solve(f: &dyn Fn(f64) -> f64, target: f64, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if f(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-11 {
            break;
        }
    }
    (lo + hi) / 2.0
}

/// Split a plot (metres, any orientation) into parcels.
pub fn split(plot: &[XY], sweep: &Sweep, sizing: Sizing) -> Result<Vec<Parcel>, String> {
    let mut plot: Vec<XY> = plot.to_vec();
    plot.dedup_by(|a, b| len(sub(*a, *b)) < 1e-9);
    if plot.len() > 1 && len(sub(plot[0], *plot.last().unwrap())) < 1e-9 {
        plot.pop();
    }
    if plot.len() < 3 {
        return Err(tr("het perceel heeft minstens drie hoekpunten nodig", "the plot needs at least three corners"));
    }
    if signed_area(&plot) < 0.0 {
        plot.reverse();
    }
    let total = area(&plot);
    if total < 1e-6 {
        return Err(tr("het perceel heeft geen oppervlak", "the plot has no area"));
    }
    let stations: Vec<f64> = plot.iter().map(|p| sweep.station(*p)).collect();
    let span = stations.iter().fold(0.0f64, |m, s| m.max(s.abs())) + plot.iter().map(|p| len(sub(*p, plot[0]))).fold(0.0, f64::max);
    // A bracket around the whole plot, then tightened to where its area starts and ends.
    let (mut lo, mut hi) = (stations.iter().cloned().fold(f64::MAX, f64::min) - span, stations.iter().cloned().fold(f64::MIN, f64::max) + span);
    let a = |s: f64| sweep.area_before(&plot, s);
    lo = solve(&a, 1e-9, lo, hi);
    hi = solve(&a, total - 1e-9, lo, hi);
    let front = front_edges(&plot, sweep);
    let fr = |s: f64| if front.is_empty() { s - lo } else { front_before(&front, sweep, s) };
    let total_front = fr(hi);
    let mut cuts = vec![lo];
    match sizing {
        Sizing::Equal(n) => {
            let n = n.max(1);
            for k in 1..n {
                cuts.push(solve(&a, total * k as f64 / n as f64, lo, hi));
            }
        }
        Sizing::Min { area: min_area, front: min_front, rest } => {
            if min_area.is_none() && min_front.is_none() {
                return Err(tr("geef een oppervlakte (OPP=) of een frontbreedte (FRONT=)", "give an area (OPP=) or a frontage (FRONT=)"));
            }
            if min_area.is_some_and(|v| v <= 0.0) || min_front.is_some_and(|v| v <= 0.0) {
                return Err(tr("oppervlakte en frontbreedte moeten groter dan 0 zijn", "area and frontage must be larger than 0"));
            }
            if min_front.is_some() && total_front <= 1e-9 {
                return Err(tr("het perceel heeft geen voorkant langs de straat", "the plot has no front along the street"));
            }
            let mut s = lo;
            loop {
                let mut next = s;
                if let Some(t) = min_area {
                    let want = a(s) + t;
                    if want > total - 1e-9 {
                        break;
                    }
                    next = next.max(solve(&a, want, s, hi));
                }
                if let Some(w) = min_front {
                    let want = fr(s) + w;
                    if want > total_front - 1e-9 {
                        break;
                    }
                    next = next.max(solve(&fr, want, s, hi));
                }
                if next >= hi - 1e-9 {
                    break;
                }
                cuts.push(next);
                s = next;
                if cuts.len() > 10_000 {
                    return Err(tr("meer dan 10.000 percelen: kies een grotere oppervlakte of frontbreedte", "more than 10,000 parcels: choose a larger area or frontage"));
                }
            }
            let full = cuts.len() - 1;
            if full == 0 {
                return Err(t!("het perceel ({total:.2} m²) is kleiner dan één kavel van die maat", "the plot ({total:.2} m²) is smaller than one parcel of that size"));
            }
            let remainder = total - a(cuts[full]);
            if remainder > 1e-6 {
                match rest {
                    Remainder::Own => {}
                    Remainder::Last => {
                        cuts.pop();
                    }
                    Remainder::Spread => {
                        // As many equal parcels as there were full ones.
                        cuts.truncate(1);
                        for k in 1..full {
                            if min_area.is_some() {
                                cuts.push(solve(&a, total * k as f64 / full as f64, lo, hi));
                            } else {
                                cuts.push(solve(&fr, total_front * k as f64 / full as f64, lo, hi));
                            }
                        }
                    }
                }
            }
        }
    }
    cuts.push(hi);
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        // The outer pieces reach past the plot: no clipping line there.
        let s0 = if w[0] <= lo { lo - span } else { w[0] };
        let s1 = if w[1] >= hi { hi + span } else { w[1] };
        let ring = sweep.piece(&plot, s0, s1);
        if ring.len() < 3 {
            continue;
        }
        let ar = area(&ring);
        out.push(Parcel { perimeter: perimeter(&ring), frontage: frontage(&ring, &front, w[0], w[1]), area: ar, ring, s0: w[0], s1: w[1] });
    }
    Ok(out)
}

/// A point inside a ring for its label: the centroid, or else the middle of
/// the widest horizontal chord through it.
pub fn label_point(r: &[XY]) -> XY {
    let a = signed_area(r);
    let o = r[0];
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..r.len() {
        let (p, q) = (sub(r[i], o), sub(r[(i + 1) % r.len()], o));
        let c = p[0] * q[1] - q[0] * p[1];
        cx += (p[0] + q[0]) * c;
        cy += (p[1] + q[1]) * c;
    }
    let c = if a.abs() > 1e-12 { [o[0] + cx / (6.0 * a), o[1] + cy / (6.0 * a)] } else { o };
    if inside(c, r) {
        return c;
    }
    let mut xs: Vec<f64> = Vec::new();
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        if (p[1] > c[1]) != (q[1] > c[1]) {
            xs.push(p[0] + (c[1] - p[1]) / (q[1] - p[1]) * (q[0] - p[0]));
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.chunks(2).filter(|w| w.len() == 2).max_by(|a, b| (a[1] - a[0]).total_cmp(&(b[1] - b[0]))).map(|w| [(w[0] + w[1]) / 2.0, c[1]]).unwrap_or(c)
}

fn inside(p: XY, r: &[XY]) -> bool {
    let mut odd = false;
    let mut j = r.len() - 1;
    for i in 0..r.len() {
        let (a, b) = (r[i], r[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
        j = i;
    }
    odd
}

/// Square metres in the interface language: "1.234,56" or "1,234.56".
pub fn m2(v: f64) -> String {
    m2_in(v, crate::lang::current() == crate::lang::Lang::Nl)
}

/// A number with two decimals and thousands separators, Dutch or English.
pub fn m2_in(v: f64, dutch: bool) -> String {
    let (thousands, comma) = if dutch { ('.', ',') } else { (',', '.') };
    let s = format!("{v:.2}");
    let (int, dec) = s.split_once('.').unwrap_or((&s, "00"));
    let neg = int.starts_with('-');
    let digits: Vec<char> = int.trim_start_matches('-').chars().collect();
    let mut out = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(thousands);
        }
        out.push(*c);
    }
    format!("{}{out}{comma}{dec}", if neg { "-" } else { "" })
}

// ── In the drawing ──────────────────────────────────────────────────────────

/// Metres with two decimals, a decimal comma in Dutch.
fn metres(v: f64) -> String {
    let s = format!("{v:.2}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

/// A closed outline or an open line, in drawing coordinates.
fn outline(e: &EntityType) -> Option<(Vec<XY>, bool)> {
    match e {
        EntityType::LwPolyline(l) if l.vertices.len() >= 2 => {
            let mut pts: Vec<XY> = l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect();
            let closed = l.is_closed || (pts.len() > 3 && len(sub(pts[0], *pts.last().unwrap())) < 1e-9);
            if closed && len(sub(pts[0], *pts.last().unwrap())) < 1e-9 {
                pts.pop();
            }
            Some((pts, closed))
        }
        EntityType::Polyline3D(p) if p.vertices.len() >= 2 => {
            let mut pts: Vec<XY> = p.vertices.iter().map(|v| [v.position.x, v.position.y]).collect();
            let closed = p.flags.closed || (pts.len() > 3 && len(sub(pts[0], *pts.last().unwrap())) < 1e-9);
            if closed && len(sub(pts[0], *pts.last().unwrap())) < 1e-9 {
                pts.pop();
            }
            Some((pts, closed))
        }
        EntityType::Line(l) => Some((vec![[l.start.x, l.start.y], [l.end.x, l.end.y]], false)),
        _ => None,
    }
}

/// How the dividing lines run.
#[derive(Clone, Debug)]
pub enum Direction {
    /// Perpendicular to the street line (a handle), or to the selected open line.
    Street(Option<Handle>),
    /// Parallel to this angle (degrees from east, counter-clockwise).
    Angle(f64),
    /// Perpendicular to the longest side of the plot.
    Auto,
}

pub struct Request {
    pub plot: Option<Handle>,
    pub direction: Direction,
    pub sizing: Sizing,
    /// Prefix of the parcel numbers ("P" gives P1, P2, …) and the first number.
    pub prefix: String,
    pub first: usize,
    pub id: Option<String>,
}

/// `CIV_PERCELEN`: split the plot into parcels and draw them.
pub fn parcels(opt: &super::CivielOptions, req: Request) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let mut extra: Vec<Handle> = req.plot.into_iter().collect();
    if let Direction::Street(Some(h)) = req.direction {
        extra.push(h);
    }
    let sel: Vec<Handle> = crate::bridge::selection().unwrap_or_default();
    let mut wanted: Vec<Handle> = extra.clone();
    wanted.extend(sel.iter().copied());
    let ents: Vec<EntityType> = crate::bridge::entities(&wanted)?.into_iter().flatten().collect();
    let by = |h: Handle| ents.iter().find(|e| e.common().handle == h);
    // The plot: the given handle, else the selected closed outline.
    let (plot_h, plot_d) = match req.plot {
        Some(h) => {
            let e = by(h).ok_or_else(|| t!("geen element {}", "no element {}", super::api::handle_text(h)))?;
            match outline(e) {
                Some((p, true)) => (h, p),
                _ => return Err(tr("het perceel (KAVEL=) moet een gesloten polylijn zijn", "the plot (KAVEL=) must be a closed polyline")),
            }
        }
        None => ents
            .iter()
            .filter(|e| sel.contains(&e.common().handle))
            .find_map(|e| match outline(e) {
                Some((p, true)) => Some((e.common().handle, p)),
                _ => None,
            })
            .ok_or_else(|| tr("selecteer een gesloten polylijn (het perceel of plangebied), of geef KAVEL=<handle>", "select a closed polyline (the plot or plan area), or give KAVEL=<handle>"))?,
    };
    let street_d: Option<Vec<XY>> = match &req.direction {
        Direction::Street(Some(h)) => match by(*h).and_then(outline) {
            Some((p, _)) => Some(p),
            None => return Err(tr("de straat (STRAAT=) moet een lijn of polylijn zijn", "the street (STRAAT=) must be a line or polyline")),
        },
        Direction::Street(None) => ents.iter().filter(|e| sel.contains(&e.common().handle) && e.common().handle != plot_h).find_map(|e| outline(e).filter(|(_, c)| !c).map(|(p, _)| p)),
        _ => None,
    };
    // Metres around the plot's first corner.
    let origin = plot_d[0];
    let to_m = |p: XY| [(p[0] - origin[0]) / f, (p[1] - origin[1]) / f];
    let to_d = |p: XY| [origin[0] + p[0] * f, origin[1] + p[1] * f];
    let plot: Vec<XY> = plot_d.iter().map(|p| to_m(*p)).collect();
    let (sweep, how) = match (&req.direction, street_d) {
        (Direction::Angle(deg), _) => (Sweep::parallel_to(deg.to_radians()), t!("evenwijdig aan {deg}°", "parallel to {deg}°")),
        (_, Some(s)) => {
            let mut line: Vec<XY> = s.iter().map(|p| to_m(*p)).collect();
            line.dedup_by(|a, b| len(sub(*a, *b)) < 1e-9);
            if line.len() < 2 {
                return Err(tr("de straatlijn heeft geen lengte", "the street line has no length"));
            }
            (Sweep::Street { line }, tr("haaks op de straat", "perpendicular to the street"))
        }
        _ => {
            // Perpendicular to the longest side: that side acts as the street.
            let n = plot.len();
            let (i, _) = (0..n).map(|i| (i, len(sub(plot[(i + 1) % n], plot[i])))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            let (a, b) = (plot[i], plot[(i + 1) % n]);
            (Sweep::Street { line: vec![a, b] }, tr("haaks op de langste zijde", "perpendicular to the longest side"))
        }
    };
    let parcels = split(&plot, &sweep, req.sizing)?;
    let method = match req.sizing {
        Sizing::Equal(n) => t!("{n} gelijke delen", "{n} equal parts"),
        Sizing::Min { area, front, rest } => {
            let mut v = Vec::new();
            if let Some(a) = area {
                v.push(t!("oppervlakte {} m²", "area {} m²", m2(a)));
            }
            if let Some(w) = front {
                v.push(t!("front {} m", "frontage {} m", metres(w)));
            }
            let rest = match rest {
                Remainder::Last => tr("rest bij het laatste perceel", "remainder in the last parcel"),
                Remainder::Own => tr("rest als eigen perceel", "remainder as a parcel of its own"),
                Remainder::Spread => tr("rest verdeeld", "remainder spread"),
            };
            format!("{} ({rest})", v.join(crate::lang::tr(" en ", " and ")))
        }
    };
    let line_l = ctx.layer("KG", "GRENS_PERCEEL", Element::G)?;
    let text_l = ctx.layer("KG", "GRENS_PERCEEL", Element::T)?;
    let k = ctx.k;
    let plot_name = super::api::handle_text(plot_h);
    let mut out = Out::default();
    let mut rows = Vec::new();
    // Dutch spreadsheets read semicolons and decimal commas, English ones commas and points.
    let dutch = crate::lang::current() == crate::lang::Lang::Nl;
    let sep = if dutch { ";" } else { "," };
    let mut csv = ["perceel", "oppervlakte_m2", "omtrek_m", "front_m", "x", "y"].join(sep) + "\n";
    for (i, p) in parcels.iter().enumerate() {
        let name = format!("{}{}", req.prefix, req.first + i);
        let ring: Vec<XY> = p.ring.iter().map(|q| to_d(*q)).collect();
        let d = record(&[
            ("perceel", name.clone()),
            ("oppervlakte_m2", format!("{:.2}", p.area)),
            ("omtrek_m", format!("{:.2}", p.perimeter)),
            ("front_m", format!("{:.2}", p.frontage)),
            ("kavel", plot_name.clone()),
            ("methode", method.clone()),
            ("richting", how.clone()),
        ]);
        out.put(&line_l, crate::emit::polylines(&[(ring.clone(), true)], &line_l.name, k, Some(&d)));
        let at = to_d(label_point(&p.ring));
        let centred = |mut t: EntityType| {
            if let EntityType::Text(tx) = &mut t {
                tx.horizontal_alignment = TextHorizontalAlignment::Center;
                tx.vertical_alignment = TextVerticalAlignment::Middle;
                tx.alignment_point = Some(tx.insertion_point);
            }
            t
        };
        out.put(&text_l, [centred(ctx.text([at[0], at[1] + 1.8 * k], &name, 2.5, 0.0, &text_l, None)), centred(ctx.text([at[0], at[1] - 1.8 * k], &format!("{} m²", m2(p.area)), 1.8, 0.0, &text_l, None))]);
        let pp = ctx.from_drawing(at);
        let dec = |v: f64, n: usize| if dutch { format!("{v:.n$}").replace('.', ",") } else { format!("{v:.n$}") };
        csv.push_str(&[name.clone(), dec(p.area, 2), dec(p.perimeter, 2), dec(p.frontage, 2), dec(pp[0], 3), dec(pp[1], 3)].join(sep));
        csv.push('\n');
        rows.push((name, p.area, p.perimeter, p.frontage));
    }
    let handles = out.write_handles(&t!("Percelen {plot_name}", "Parcels {plot_name}"))?;
    // The boundaries come first (their layer was used first).
    let bounds: Vec<Handle> = handles.iter().take(parcels.len()).copied().collect();
    crate::bridge::index_add("percelen", &plot_name, &bounds, true);
    let csv_path = match &req.id {
        Some(id) => super::api::result_dir().join(format!("{id}.csv")),
        None => crate::bridge::beside_drawing("percelen.csv"),
    };
    if let Some(dir) = csv_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&csv_path, &csv).map_err(|e| format!("{}: {e}", csv_path.display()))?;
    let total: f64 = parcels.iter().map(|p| p.area).sum();
    super::api::note(
        "percelen",
        rows.iter()
            .zip(&bounds)
            .map(|((n, a, o, fr), h)| serde_json::json!({"perceel": n, "oppervlakte_m2": (a * 100.0).round() / 100.0, "omtrek_m": (o * 100.0).round() / 100.0, "front_m": (fr * 100.0).round() / 100.0, "handle": super::api::handle_text(*h)}))
            .collect::<Vec<_>>(),
    );
    super::api::note("totaal_m2", (total * 100.0).round() / 100.0);
    super::api::note("kavel_m2", (area(&plot) * 100.0).round() / 100.0);
    super::api::note("csv", csv_path.to_string_lossy().into_owned());
    super::api::note("laag", line_l.name.clone());
    let sizes: Vec<f64> = parcels.iter().map(|p| p.area).collect();
    let (lo, hi) = sizes.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    Ok(t!(
        "{} percelen ({method}, {how}): {} tot {} m², samen {} m² — tabel in {}.",
        "{} parcels ({method}, {how}): {} to {} m², {} m² in all — table in {}.",
        parcels.len(),
        m2(lo),
        m2(hi),
        m2(total),
        csv_path.display()
    ))
}

/// `CIV_PERCEEL_INFO`: area, perimeter and frontage of the selected parcels
/// (closed polylines), or of `PERCEEL=` handles.
pub fn info(opt: &super::CivielOptions, handles: Vec<Handle>, street: Option<Handle>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let mut want: Vec<Handle> = if handles.is_empty() { crate::bridge::selection().unwrap_or_default() } else { handles };
    if let Some(s) = street {
        want.push(s);
    }
    let ents: Vec<EntityType> = crate::bridge::entities(&want)?.into_iter().flatten().collect();
    let street_line: Option<Vec<XY>> = street.and_then(|h| ents.iter().find(|e| e.common().handle == h)).and_then(outline).map(|(p, _)| p);
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for e in &ents {
        let h = e.common().handle;
        if Some(h) == street || !seen.insert(h) {
            continue;
        }
        let Some((ring_d, true)) = outline(e) else { continue };
        let o = ring_d[0];
        let ring: Vec<XY> = ring_d.iter().map(|p| [(p[0] - o[0]) / f, (p[1] - o[1]) / f]).collect();
        let a = area(&ring);
        let per = perimeter(&ring);
        let front = match &street_line {
            Some(s) => {
                let sweep = Sweep::Street { line: s.iter().map(|p| [(p[0] - o[0]) / f, (p[1] - o[1]) / f]).collect() };
                let fe = front_edges(&ring, &sweep);
                Some(fe.iter().map(|(p, q)| len(sub(*q, *p))).sum::<f64>())
            }
            None => super::data_value(e, "front_m").and_then(|v| v.parse::<f64>().ok()),
        };
        let name = super::data_value(e, "perceel").unwrap_or_else(|| super::api::handle_text(h));
        rows.push((name, h, a, per, front));
    }
    if rows.is_empty() {
        return Err(tr("selecteer een of meer gesloten polylijnen (percelen), of geef PERCEEL=<handle>;…", "select one or more closed polylines (parcels), or give PERCEEL=<handle>;…"));
    }
    super::api::note(
        "percelen",
        rows.iter()
            .map(|(n, h, a, p, fr)| serde_json::json!({"perceel": n, "handle": super::api::handle_text(*h), "oppervlakte_m2": (a * 100.0).round() / 100.0, "omtrek_m": (p * 100.0).round() / 100.0, "front_m": fr.map(|v| (v * 100.0).round() / 100.0)}))
            .collect::<Vec<_>>(),
    );
    let total: f64 = rows.iter().map(|r| r.2).sum();
    super::api::note("totaal_m2", (total * 100.0).round() / 100.0);
    let text: Vec<String> = rows
        .iter()
        .map(|(n, _, a, p, fr)| {
            let mut s = t!("{n}: {} m², omtrek {} m", "{n}: {} m², perimeter {} m", m2(*a), metres(*p));
            if let Some(v) = fr {
                s.push_str(&t!(", front {} m", ", frontage {} m", metres(*v)));
            }
            s
        })
        .collect();
    Ok(t!("{} — samen {} m².", "{} — {} m² in all.", text.join("; "), m2(total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 60 × 40 m plot with a slanted back (a trapezoid): 60 m front on the
    /// x axis, depth 40 m at x = 0 and 30 m at x = 60.
    fn trapezoid() -> Vec<XY> {
        vec![[0.0, 0.0], [60.0, 0.0], [60.0, 30.0], [0.0, 40.0]]
    }

    #[test]
    fn areas_and_clipping_are_exact() {
        let t = trapezoid();
        assert!((area(&t) - 2100.0).abs() < 1e-9);
        assert!((perimeter(&t) - (60.0 + 30.0 + 60.0f64.hypot(10.0) + 40.0)).abs() < 1e-9);
        // Large coordinates (RD) keep their precision.
        let rd: Vec<XY> = t.iter().map(|p| [p[0] + 104_885.0, p[1] + 425_320.0]).collect();
        assert!((area(&rd) - 2100.0).abs() < 1e-6);
        let left = clip_half(&t, [30.0, 0.0], [1.0, 0.0]);
        assert!((area(&left) + area(&clip_half(&t, [30.0, 0.0], [-1.0, 0.0])) - 2100.0).abs() < 1e-9);
        assert_eq!(m2_in(1234.5, true), "1.234,50");
        assert_eq!(m2_in(1234.5, false), "1,234.50");
        assert_eq!(m2_in(512.346, true), "512,35");
    }

    #[test]
    fn equal_parcels_are_equal_to_the_hundredth() {
        let t = trapezoid();
        let street = Sweep::Street { line: vec![[-10.0, -5.0], [70.0, -5.0]] };
        let p = split(&t, &street, Sizing::Equal(7)).unwrap();
        assert_eq!(p.len(), 7);
        for q in &p {
            assert!((q.area - 300.0).abs() < 0.005, "{}", q.area);
        }
        let sum: f64 = p.iter().map(|q| q.area).sum();
        assert!((sum - 2100.0).abs() < 1e-6);
        // Perpendicular to the street: vertical dividing lines; the fronts add up.
        let front: f64 = p.iter().map(|q| q.frontage).sum();
        assert!((front - 60.0).abs() < 1e-6, "{front}");
        // The same in RD coordinates and tilted 30°: still exact.
        let (s, c) = 30f64.to_radians().sin_cos();
        let rot = |p: XY| [104_885.0 + p[0] * c - p[1] * s, 425_320.0 + p[0] * s + p[1] * c];
        let tt: Vec<XY> = t.iter().map(|p| rot(*p)).collect();
        let st = Sweep::Street { line: vec![rot([-10.0, -5.0]), rot([70.0, -5.0])] };
        let p = split(&tt, &st, Sizing::Equal(3)).unwrap();
        assert!(p.iter().all(|q| (q.area - 700.0).abs() < 0.005), "{:?}", p.iter().map(|q| q.area).collect::<Vec<_>>());
    }

    #[test]
    fn target_area_with_remainder_options() {
        let t = trapezoid();
        let street = Sweep::Street { line: vec![[0.0, -3.0], [60.0, -3.0]] };
        // 2100 m² in parcels of 400 m²: five full ones and 100 m² left.
        let last = split(&t, &street, Sizing::Min { area: Some(400.0), front: None, rest: Remainder::Last }).unwrap();
        assert_eq!(last.len(), 5);
        assert!(last[..4].iter().all(|q| (q.area - 400.0).abs() < 0.005));
        assert!((last[4].area - 500.0).abs() < 0.005);
        let own = split(&t, &street, Sizing::Min { area: Some(400.0), front: None, rest: Remainder::Own }).unwrap();
        assert_eq!(own.len(), 6);
        assert!((own[5].area - 100.0).abs() < 0.005);
        let spread = split(&t, &street, Sizing::Min { area: Some(400.0), front: None, rest: Remainder::Spread }).unwrap();
        assert_eq!(spread.len(), 5);
        assert!(spread.iter().all(|q| (q.area - 420.0).abs() < 0.005));
        assert!(split(&t, &street, Sizing::Min { area: Some(5000.0), front: None, rest: Remainder::Last }).is_err());
    }

    #[test]
    fn frontage_and_both_minimums() {
        let t = trapezoid();
        let street = Sweep::Street { line: vec![[0.0, -3.0], [60.0, -3.0]] };
        // 12 m fronts: five parcels.
        let p = split(&t, &street, Sizing::Min { area: None, front: Some(12.0), rest: Remainder::Own }).unwrap();
        assert_eq!(p.len(), 5);
        assert!(p.iter().all(|q| (q.frontage - 12.0).abs() < 1e-6), "{:?}", p.iter().map(|q| q.frontage).collect::<Vec<_>>());
        // At least 10 m front and 420 m²: the deep left parcels are set by
        // the front, the shallow right ones by the area.
        let p = split(&t, &street, Sizing::Min { area: Some(380.0), front: Some(10.0), rest: Remainder::Own }).unwrap();
        for q in &p[..p.len() - 1] {
            assert!(q.area >= 380.0 - 0.005 && q.frontage >= 10.0 - 1e-6, "{} {}", q.area, q.frontage);
            assert!((q.area - 380.0).abs() < 0.005 || (q.frontage - 10.0).abs() < 1e-6);
        }
    }

    #[test]
    fn parallel_to_a_direction_and_a_concave_plot() {
        // An L-shaped plot cut by lines parallel to the x axis (0°).
        let l = vec![[0.0, 0.0], [40.0, 0.0], [40.0, 10.0], [10.0, 10.0], [10.0, 40.0], [0.0, 40.0]];
        let total = area(&l);
        assert!((total - 700.0).abs() < 1e-9);
        let p = split(&l, &Sweep::parallel_to(0.0), Sizing::Equal(4)).unwrap();
        assert_eq!(p.len(), 4);
        assert!(p.iter().all(|q| (q.area - 175.0).abs() < 0.005), "{:?}", p.iter().map(|q| q.area).collect::<Vec<_>>());
        // The first cut lies at y where 40·y = 175 (inside the foot of the L).
        assert!((p[0].s1 - 175.0 / 40.0).abs() < 1e-6, "{}", p[0].s1);
        // Labels land inside, also for the L itself.
        assert!(inside(label_point(&l), &l));
    }

    #[test]
    fn a_curved_street_gives_radial_lines() {
        // A ring sector plot along a quarter-circle street of radius 50.
        let mut street = Vec::new();
        for k in 0..=30 {
            let a = k as f64 / 30.0 * std::f64::consts::FRAC_PI_2;
            street.push([50.0 * a.cos(), 50.0 * a.sin()]);
        }
        let mut plot: Vec<XY> = Vec::new();
        for k in 0..=30 {
            let a = k as f64 / 30.0 * std::f64::consts::FRAC_PI_2;
            plot.push([52.0 * a.cos(), 52.0 * a.sin()]);
        }
        for k in (0..=30).rev() {
            let a = k as f64 / 30.0 * std::f64::consts::FRAC_PI_2;
            plot.push([80.0 * a.cos(), 80.0 * a.sin()]);
        }
        let p = split(&plot, &Sweep::Street { line: street }, Sizing::Equal(5)).unwrap();
        let total = area(&plot);
        assert_eq!(p.len(), 5);
        assert!(p.iter().all(|q| (q.area - total / 5.0).abs() < 0.005));
    }
}
