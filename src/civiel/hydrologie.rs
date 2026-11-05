//! Hydrology: where water runs and where it gathers.
//!
//! - **Water drop** (`CIV_WATERDRUPPEL`): the path of a drop over the TIN by
//!   steepest descent across the triangles. In a triangle the drop runs down
//!   the plane's gradient to an edge; where both triangles at an edge slope
//!   towards it (a channel) it follows the edge down; at a vertex it takes the
//!   steepest way out (a triangle's gradient or a descending edge). It stops
//!   in a pit, at the edge of the TIN or on a flat triangle.
//! - **Catchments** (`CIV_STROOMGEBIED`): D8 flow directions on a grid sampled
//!   from the TIN or fetched from the height source (AHN in the Netherlands).
//!   Depressions are filled with Priority-Flood (Barnes, Lehman & Mulla 2014,
//!   "Priority-Flood: An optimal depression-filling and watershed-labeling
//!   algorithm for digital elevation models", Computers & Geosciences 62);
//!   flats drain along the flooding order. Each cell belongs to the first
//!   outlet downstream of it. Cells without a height (water, buildings in the
//!   AHN ground model) and the grid border are where water leaves the model.
//! - **Ponds** (`CIV_PLASSEN`, also part of `CIV_STROOMGEBIED`): the filled
//!   depressions, with their depth and volume — water on the street.
//!
//! Neither NLCS 5.02 nor the BGT has objects for flow paths or catchments, so
//! these go on tool layers OI-WATERDRUPPEL, OI-STROOMGEBIED and OI-PLAS, like
//! the height analysis (OI-ANALYSE-HOOGTE-nn).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use ocs_plugin_api::host::codec::entities::{Polyline3D, TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{Circle, Color, EntityType, Vector3};

use super::terrein::{Tin, P3};
use super::{record, v3, Ctx, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::LayerSpec;

/// A message in the interface language (Dutch or English).
macro_rules! t {
    ($nl:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

fn tr(nl: &str, en: &str) -> String {
    crate::lang::tr(nl, en).to_string()
}

/// A number with `n` decimals, a decimal comma in Dutch.
fn num(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

fn sub(a: XY, b: XY) -> XY {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: XY, b: XY) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dot(a: XY, b: XY) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: XY) -> f64 {
    a[0].hypot(a[1])
}

// ── Water drop over a TIN ───────────────────────────────────────────────────

/// Why a drop path ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A local minimum: the water stays here (a pond).
    Pit,
    /// It runs off the TIN.
    Edge,
    /// A level triangle or edge.
    Flat,
    /// The step limit (a very long path).
    Limit,
}

impl Stop {
    pub fn label(self) -> &'static str {
        match self {
            Stop::Pit => crate::lang::tr("laagste punt (put)", "low point (pit)"),
            Stop::Edge => crate::lang::tr("rand van het terrein", "edge of the surface"),
            Stop::Flat => crate::lang::tr("vlak terrein", "flat ground"),
            Stop::Limit => crate::lang::tr("stappenlimiet", "step limit"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DropPath {
    /// Positions and heights in the TIN's units.
    pub points: Vec<P3>,
    pub stop: Stop,
}

impl DropPath {
    pub fn length(&self) -> f64 {
        self.points.windows(2).fold(0.0, |s, w| s + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
    }
    pub fn drop(&self) -> f64 {
        match (self.points.first(), self.points.last()) {
            (Some(a), Some(b)) => a[2] - b[2],
            _ => 0.0,
        }
    }
}

/// A TIN prepared for flow: gradients, neighbours and a lookup grid.
pub struct Flow {
    pub tin: Tin,
    grad: Vec<XY>,
    /// Neighbour across edge k (vertices k and k+1) of each triangle.
    nb: Vec<[Option<usize>; 3]>,
    /// Triangles around each vertex.
    around: Vec<Vec<usize>>,
    /// Vertices on the outline of the TIN.
    border: Vec<bool>,
    origin: XY,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl Flow {
    pub fn new(tin: Tin) -> Flow {
        let p = |i: usize| tin.points[i];
        let grad: Vec<XY> = tin
            .triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let d = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
                if d.abs() < 1e-18 {
                    return [0.0, 0.0];
                }
                let gx = ((b[2] - a[2]) * (c[1] - a[1]) - (c[2] - a[2]) * (b[1] - a[1])) / d;
                let gy = ((c[2] - a[2]) * (b[0] - a[0]) - (b[2] - a[2]) * (c[0] - a[0])) / d;
                [gx, gy]
            })
            .collect();
        let mut edges: HashMap<(usize, usize), Vec<(usize, usize)>> = HashMap::new();
        let mut around = vec![Vec::new(); tin.points.len()];
        for (ti, t) in tin.triangles.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edges.entry((a.min(b), a.max(b))).or_default().push((ti, k));
                around[t[k]].push(ti);
            }
        }
        let mut nb = vec![[None; 3]; tin.triangles.len()];
        let mut border = vec![false; tin.points.len()];
        for (&(a, b), list) in &edges {
            match list[..] {
                [(t1, k1), (t2, k2)] => {
                    nb[t1][k1] = Some(t2);
                    nb[t2][k2] = Some(t1);
                }
                _ => {
                    border[a] = true;
                    border[b] = true;
                }
            }
        }
        // Triangles per cell of a uniform grid.
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for q in &tin.points {
            lo = [lo[0].min(q[0]), lo[1].min(q[1])];
            hi = [hi[0].max(q[0]), hi[1].max(q[1])];
        }
        let n = (tin.triangles.len() as f64).sqrt().ceil().max(1.0);
        let cell = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / n).max(1e-9);
        let cols = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); cols * rows];
        for (ti, t) in tin.triangles.iter().enumerate() {
            let xs = t.map(|i| tin.points[i][0]);
            let ys = t.map(|i| tin.points[i][1]);
            let c0 = ((xs.iter().cloned().fold(f64::MAX, f64::min) - lo[0]) / cell).floor() as usize;
            let c1 = ((xs.iter().cloned().fold(f64::MIN, f64::max) - lo[0]) / cell).floor() as usize;
            let r0 = ((ys.iter().cloned().fold(f64::MAX, f64::min) - lo[1]) / cell).floor() as usize;
            let r1 = ((ys.iter().cloned().fold(f64::MIN, f64::max) - lo[1]) / cell).floor() as usize;
            for r in r0..=r1.min(rows - 1) {
                for c in c0..=c1.min(cols - 1) {
                    cells[r * cols + c].push(ti as u32);
                }
            }
        }
        Flow { tin, grad, nb, around, border, origin: lo, cell, cols, rows, cells }
    }

    fn xy(&self, i: usize) -> XY {
        let p = self.tin.points[i];
        [p[0], p[1]]
    }

    /// The triangle that holds a point.
    pub fn locate(&self, p: XY) -> Option<usize> {
        let c = ((p[0] - self.origin[0]) / self.cell).floor();
        let r = ((p[1] - self.origin[1]) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        self.cells[r as usize * self.cols + c as usize].iter().map(|t| *t as usize).find(|&t| {
            let [a, b, c] = self.tin.triangles[t].map(|i| self.xy(i));
            let s = 1e-9 * len(sub(b, a)).max(len(sub(c, a)));
            cross(sub(b, a), sub(p, a)) >= -s && cross(sub(c, b), sub(p, b)) >= -s && cross(sub(a, c), sub(p, c)) >= -s
        })
    }

    fn plane_z(&self, t: usize, p: XY) -> f64 {
        let a = self.tin.points[self.tin.triangles[t][0]];
        let g = self.grad[t];
        a[2] + g[0] * (p[0] - a[0]) + g[1] * (p[1] - a[1])
    }

    /// From a vertex without a way down, across the vertices of the same
    /// height (breadth first, a few thousand at most) to the nearest one with
    /// a lower neighbour or on the outline: the route there, `v` left out.
    fn across_level(&self, v: usize) -> Option<Vec<usize>> {
        let z = self.tin.points[v][2];
        let tol = 1e-6 * (1.0 + z.abs());
        let neighbours = |u: usize| -> Vec<usize> {
            let mut out = Vec::new();
            for &t in &self.around[u] {
                for w in self.tin.triangles[t] {
                    if w != u && !out.contains(&w) {
                        out.push(w);
                    }
                }
            }
            out
        };
        let mut parent: HashMap<usize, usize> = HashMap::new();
        let mut queue = std::collections::VecDeque::from([v]);
        parent.insert(v, v);
        while let Some(u) = queue.pop_front() {
            if parent.len() > 5000 {
                return None;
            }
            let ns = neighbours(u);
            if u != v && (self.border[u] || ns.iter().any(|w| self.tin.points[*w][2] < z - tol)) {
                let mut route = vec![u];
                let mut x = u;
                while parent[&x] != v {
                    x = parent[&x];
                    route.push(x);
                }
                route.reverse();
                return Some(route);
            }
            for w in ns {
                if (self.tin.points[w][2] - z).abs() <= tol && !parent.contains_key(&w) {
                    parent.insert(w, u);
                    queue.push_back(w);
                }
            }
        }
        None
    }

    /// The path of a drop let go at `p`.
    pub fn drop_path(&self, p: XY) -> Option<DropPath> {
        enum At {
            Tri(usize),
            Vert(usize),
        }
        let t0 = self.locate(p)?;
        let mut pts = vec![[p[0], p[1], self.plane_z(t0, p)]];
        let mut state = At::Tri(t0);
        let mut cur = p;
        let mut seen: HashSet<usize> = HashSet::new();
        // A hop to the next triangle without moving (the drop sat on their
        // shared edge): where it came from.
        let mut hop_from: Option<usize> = None;
        for _ in 0..500_000 {
            match state {
                At::Tri(t) => {
                    let g = self.grad[t];
                    let gl = len(g);
                    let tri = self.tin.triangles[t];
                    if gl < 1e-12 {
                        // A level triangle: on to its nearest corner, and
                        // from there across the level ground.
                        let v = *tri.iter().min_by(|a, b| len(sub(self.xy(**a), cur)).total_cmp(&len(sub(self.xy(**b), cur)))).unwrap();
                        if seen.contains(&v) {
                            return Some(DropPath { points: pts, stop: Stop::Flat });
                        }
                        pts.push(self.tin.points[v]);
                        cur = self.xy(v);
                        state = At::Vert(v);
                        continue;
                    }
                    let d = [-g[0] / gl, -g[1] / gl];
                    let size = (0..3).map(|k| len(sub(self.xy(tri[(k + 1) % 3]), self.xy(tri[k])))).fold(0.0, f64::max);
                    let tol = 1e-9 * size.max(1e-6);
                    let mut best: Option<(f64, usize, f64)> = None;
                    for k in 0..3 {
                        let (u, w) = (self.xy(tri[k]), self.xy(tri[(k + 1) % 3]));
                        let e = sub(w, u);
                        let den = cross(d, e);
                        if den.abs() < 1e-15 * len(e) {
                            continue;
                        }
                        let r = sub(u, cur);
                        let (lam, mu) = (cross(r, e) / den, cross(r, d) / den);
                        if lam > tol && (-1e-9..=1.0 + 1e-9).contains(&mu) && best.is_none_or(|b| lam < b.0) {
                            best = Some((lam, k, mu.clamp(0.0, 1.0)));
                        }
                    }
                    let Some((lam, k, mu)) = best else {
                        // No way out forward. On a corner: on from that vertex.
                        if let Some(&v) = tri.iter().find(|&&v| len(sub(self.xy(v), cur)) <= tol) {
                            if seen.contains(&v) {
                                return Some(DropPath { points: pts, stop: Stop::Pit });
                            }
                            cur = self.xy(v);
                            hop_from = None;
                            state = At::Vert(v);
                            continue;
                        }
                        // On an edge: the drop runs into the triangle beyond
                        // it, or both sides slope to the edge (a channel) and
                        // it follows it down.
                        let on = (0..3).find(|&k| {
                            let (u, w) = (self.xy(tri[k]), self.xy(tri[(k + 1) % 3]));
                            let e = sub(w, u);
                            let l = len(e).max(1e-12);
                            let along = dot(sub(cur, u), e) / l;
                            (cross(e, sub(cur, u)) / l).abs() <= 1e-6 * l.max(1.0) && along > -1e-9 && along < l + 1e-9
                        });
                        let Some(k) = on else {
                            return Some(DropPath { points: pts, stop: Stop::Flat });
                        };
                        let (iu, iw) = (tri[k], tri[(k + 1) % 3]);
                        match self.nb[t][k] {
                            Some(t2) if hop_from != Some(t2) => {
                                hop_from = Some(t);
                                state = At::Tri(t2);
                            }
                            _ => {
                                let (zu, zw) = (self.tin.points[iu][2], self.tin.points[iw][2]);
                                let v = if zu < zw {
                                    iu
                                } else if zw < zu {
                                    iw
                                } else {
                                    return Some(DropPath { points: pts, stop: Stop::Flat });
                                };
                                pts.push(self.tin.points[v]);
                                cur = self.xy(v);
                                hop_from = None;
                                state = At::Vert(v);
                            }
                        }
                        continue;
                    };
                    hop_from = None;
                    let (iu, iw) = (tri[k], tri[(k + 1) % 3]);
                    if mu < 1e-7 || mu > 1.0 - 1e-7 {
                        let v = if mu < 0.5 { iu } else { iw };
                        pts.push(self.tin.points[v]);
                        cur = self.xy(v);
                        state = At::Vert(v);
                        continue;
                    }
                    let q = [cur[0] + d[0] * lam, cur[1] + d[1] * lam];
                    let (zu, zw) = (self.tin.points[iu][2], self.tin.points[iw][2]);
                    pts.push([q[0], q[1], zu + mu * (zw - zu)]);
                    let Some(t2) = self.nb[t][k] else {
                        return Some(DropPath { points: pts, stop: Stop::Edge });
                    };
                    // Does the drop go on into the next triangle, or do both
                    // slope towards this edge (a channel)?
                    let e = sub(self.xy(iw), self.xy(iu));
                    let inward = [e[1], -e[0]];
                    let g2 = self.grad[t2];
                    if dot([-g2[0], -g2[1]], inward) > 1e-12 * len(g2) * len(e) {
                        cur = q;
                        state = At::Tri(t2);
                    } else {
                        let v = if zu < zw {
                            iu
                        } else if zw < zu {
                            iw
                        } else {
                            return Some(DropPath { points: pts, stop: Stop::Flat });
                        };
                        pts.push(self.tin.points[v]);
                        cur = self.xy(v);
                        state = At::Vert(v);
                    }
                }
                At::Vert(v) => {
                    if !seen.insert(v) {
                        return Some(DropPath { points: pts, stop: Stop::Pit });
                    }
                    let zv = self.tin.points[v][2];
                    let pv = self.xy(v);
                    let mut best: Option<(f64, bool, usize)> = None;
                    let mut consider = |slope: f64, tri: bool, i: usize| {
                        if slope > 1e-12 && best.is_none_or(|b| slope > b.0) {
                            best = Some((slope, tri, i));
                        }
                    };
                    for &t in &self.around[v] {
                        let tri = self.tin.triangles[t];
                        let j = tri.iter().position(|&i| i == v).unwrap_or(0);
                        let (a, b) = (tri[(j + 1) % 3], tri[(j + 2) % 3]);
                        let g = self.grad[t];
                        let gl = len(g);
                        if gl > 1e-12 {
                            let d = [-g[0] / gl, -g[1] / gl];
                            let (va, vb) = (sub(self.xy(a), pv), sub(self.xy(b), pv));
                            if cross(va, d) >= -1e-12 * len(va) && cross(d, vb) >= -1e-12 * len(vb) {
                                consider(gl, true, t);
                            }
                        }
                        for w in [a, b] {
                            let zw = self.tin.points[w][2];
                            let l = len(sub(self.xy(w), pv));
                            if zw < zv && l > 0.0 {
                                consider((zv - zw) / l, false, w);
                            }
                        }
                    }
                    if best.is_none() && !self.border[v] {
                        // Level ground: across it to the nearest vertex that
                        // has a way down (or lies on the outline).
                        if let Some(route) = self.across_level(v) {
                            for w in &route {
                                seen.insert(*w);
                                pts.push(self.tin.points[*w]);
                            }
                            let w = *route.last().unwrap();
                            cur = self.xy(w);
                            seen.remove(&w);
                            state = At::Vert(w);
                            continue;
                        }
                    }
                    match best {
                        // The lowest point of the outline: the water runs off.
                        None if self.border[v] => return Some(DropPath { points: pts, stop: Stop::Edge }),
                        None => return Some(DropPath { points: pts, stop: Stop::Pit }),
                        Some((_, true, t)) => state = At::Tri(t),
                        Some((_, false, w)) => {
                            pts.push(self.tin.points[w]);
                            cur = self.xy(w);
                            state = At::Vert(w);
                        }
                    }
                }
            }
        }
        Some(DropPath { points: pts, stop: Stop::Limit })
    }
}

// ── Grids: D8, Priority-Flood, catchments, ponds ────────────────────────────

/// A height grid. Row 0 is the bottom row; `x0, y0` is the lower-left corner,
/// in the grid's own coordinates (drawing units for a grid from the TIN, the
/// height source's CRS for AHN).
#[derive(Clone, Debug)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    /// Heights in metres; NaN where unknown.
    pub z: Vec<f64>,
    /// The cell size in metres (areas and volumes).
    pub cell_m: f64,
}

pub const OUT: u32 = u32::MAX;

const NEIGHBOURS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

impl Grid {
    pub fn centre(&self, i: usize) -> XY {
        let (c, r) = (i % self.w, i / self.w);
        [self.x0 + (c as f64 + 0.5) * self.cell, self.y0 + (r as f64 + 0.5) * self.cell]
    }

    pub fn cell_at(&self, p: XY) -> Option<usize> {
        let c = ((p[0] - self.x0) / self.cell).floor();
        let r = ((p[1] - self.y0) / self.cell).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.w && (r as usize) < self.h).then(|| r as usize * self.w + c as usize)
    }

    fn valid(&self, i: usize) -> bool {
        self.z[i].is_finite()
    }

    fn neighbours(&self, i: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        let (c, r) = ((i % self.w) as i64, (i / self.w) as i64);
        NEIGHBOURS.iter().filter_map(move |(dc, dr)| {
            let (cc, rr) = (c + dc, r + dr);
            (cc >= 0 && rr >= 0 && (cc as usize) < self.w && (rr as usize) < self.h).then(|| (rr as usize * self.w + cc as usize, if dc * dr != 0 { std::f64::consts::SQRT_2 } else { 1.0 }))
        })
    }
}

/// Order-preserving key of a float for the heap.
fn key(v: f64) -> u64 {
    let b = v.to_bits();
    if v >= 0.0 {
        b ^ (1 << 63)
    } else {
        !b
    }
}

/// Flow on a grid after filling the depressions.
pub struct Drainage {
    /// The filled surface (m): depressions up to their spill level.
    pub filled: Vec<f64>,
    /// The cell each cell drains to (`OUT`: off the model).
    pub down: Vec<u32>,
    /// Cells from downstream to upstream (the Priority-Flood order).
    pub order: Vec<u32>,
    /// Upstream cells per cell, itself included.
    pub acc: Vec<u32>,
}

/// Priority-Flood from the border and the cells next to gaps, then D8
/// (steepest descent on the filled surface; flats and filled pits drain
/// along the flooding order).
pub fn drainage(g: &Grid) -> Drainage {
    let n = g.w * g.h;
    let mut filled = g.z.clone();
    let mut parent = vec![OUT; n];
    let mut done = vec![false; n];
    let mut heap: BinaryHeap<Reverse<(u64, u64, u32)>> = BinaryHeap::new();
    let mut seq = 0u64;
    for i in 0..n {
        if !g.valid(i) {
            continue;
        }
        let (c, r) = (i % g.w, i / g.w);
        let edge = c == 0 || r == 0 || c + 1 == g.w || r + 1 == g.h || g.neighbours(i).any(|(j, _)| !g.valid(j));
        if edge {
            done[i] = true;
            heap.push(Reverse((key(filled[i]), seq, i as u32)));
            seq += 1;
        }
    }
    let mut order = Vec::with_capacity(n);
    while let Some(Reverse((_, _, c))) = heap.pop() {
        let c = c as usize;
        order.push(c as u32);
        let fc = filled[c];
        for (j, _) in g.neighbours(c) {
            if done[j] || !g.valid(j) {
                continue;
            }
            done[j] = true;
            filled[j] = filled[j].max(fc);
            parent[j] = c as u32;
            heap.push(Reverse((key(filled[j]), seq, j as u32)));
            seq += 1;
        }
    }
    let mut down = vec![OUT; n];
    for &i in &order {
        let i = i as usize;
        let mut best: Option<(f64, usize)> = None;
        for (j, dist) in g.neighbours(i) {
            if g.valid(j) && filled[j] < filled[i] {
                let s = (filled[i] - filled[j]) / dist;
                if best.is_none_or(|b| s > b.0) {
                    best = Some((s, j));
                }
            }
        }
        down[i] = match best {
            Some((_, j)) => j as u32,
            None => parent[i],
        };
    }
    let mut acc = vec![0u32; n];
    for &i in &order {
        acc[i as usize] = 1;
    }
    for &i in order.iter().rev() {
        let d = down[i as usize];
        if d != OUT {
            acc[d as usize] += acc[i as usize];
        }
    }
    Drainage { filled, down, order, acc }
}

/// The cell with the most flow within `radius` cells of `i` (pour point
/// snapping), `i` itself when nothing within carries more.
pub fn snap(g: &Grid, d: &Drainage, i: usize, radius: usize) -> usize {
    if radius == 0 {
        return i;
    }
    let (c, r) = ((i % g.w) as i64, (i / g.w) as i64);
    let rad = radius as i64;
    let mut best = i;
    for dr in -rad..=rad {
        for dc in -rad..=rad {
            if dc * dc + dr * dr > rad * rad {
                continue;
            }
            let (cc, rr) = (c + dc, r + dr);
            if cc < 0 || rr < 0 || cc as usize >= g.w || rr as usize >= g.h {
                continue;
            }
            let j = rr as usize * g.w + cc as usize;
            if g.valid(j) && d.acc[j] > d.acc[best] {
                best = j;
            }
        }
    }
    best
}

/// The nearest cell with a height within `radius` cells of `i`.
pub fn nearest_valid(g: &Grid, i: usize, radius: usize) -> Option<usize> {
    if g.valid(i) {
        return Some(i);
    }
    let (c, r) = ((i % g.w) as i64, (i / g.w) as i64);
    let rad = radius as i64;
    let mut best: Option<(i64, usize)> = None;
    for dr in -rad..=rad {
        for dc in -rad..=rad {
            let d2 = dc * dc + dr * dr;
            let (cc, rr) = (c + dc, r + dr);
            if d2 > rad * rad || cc < 0 || rr < 0 || cc as usize >= g.w || rr as usize >= g.h {
                continue;
            }
            let j = rr as usize * g.w + cc as usize;
            if g.valid(j) && best.is_none_or(|b| d2 < b.0) {
                best = Some((d2, j));
            }
        }
    }
    best.map(|b| b.1)
}

/// Per cell the index of the outlet it drains to first (-1: none).
pub fn catchments(g: &Grid, d: &Drainage, outlets: &[usize]) -> Vec<i32> {
    let mut label = vec![-1i32; g.w * g.h];
    for (k, &o) in outlets.iter().enumerate() {
        label[o] = k as i32;
    }
    let is_outlet: HashSet<usize> = outlets.iter().copied().collect();
    for &i in &d.order {
        let i = i as usize;
        if is_outlet.contains(&i) {
            continue;
        }
        let dn = d.down[i];
        if dn != OUT {
            label[i] = label[dn as usize];
        }
    }
    label
}

/// A filled depression: water that stays on the surface.
#[derive(Clone, Debug)]
pub struct Pond {
    /// Its cells.
    pub cells: Vec<usize>,
    /// Deepest point (m) and where.
    pub depth: f64,
    pub deepest: usize,
    /// Water level (m) and volume (m³), area (m²).
    pub level: f64,
    pub volume: f64,
    pub area: f64,
}

/// The depressions at least `min_depth` deep and `min_area` large, deepest first.
pub fn ponds(g: &Grid, d: &Drainage, min_depth: f64, min_area: f64) -> Vec<Pond> {
    let n = g.w * g.h;
    let depth = |i: usize| if g.valid(i) { d.filled[i] - g.z[i] } else { 0.0 };
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    let a = g.cell_m * g.cell_m;
    for start in 0..n {
        if seen[start] || depth(start) <= 1e-6 {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let mut cells = Vec::new();
        while let Some(i) = stack.pop() {
            cells.push(i);
            for (j, _) in g.neighbours(i) {
                if !seen[j] && depth(j) > 1e-6 {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let deepest = *cells.iter().max_by(|x, y| depth(**x).total_cmp(&depth(**y))).unwrap();
        let p = Pond {
            depth: depth(deepest),
            deepest,
            level: cells.iter().map(|&i| d.filled[i]).fold(f64::MIN, f64::max),
            volume: cells.iter().map(|&i| depth(i) * a).sum(),
            area: cells.len() as f64 * a,
            cells,
        };
        if p.depth >= min_depth && p.area >= min_area {
            out.push(p);
        }
    }
    out.sort_by(|x, y| y.depth.total_cmp(&x.depth));
    out
}

/// The outlines of labelled regions (label ≥ 0) as rings of grid
/// coordinates, the region on the left (outer rings counter-clockwise).
pub fn outlines(g: &Grid, label: &[i32]) -> HashMap<i32, Vec<Vec<XY>>> {
    type V = (i64, i64);
    let mut edges: HashMap<i32, HashMap<V, Vec<V>>> = HashMap::new();
    let at = |c: i64, r: i64| -> i32 {
        if c < 0 || r < 0 || c as usize >= g.w || r as usize >= g.h {
            -1
        } else {
            label[r as usize * g.w + c as usize]
        }
    };
    for r in 0..g.h as i64 {
        for c in 0..g.w as i64 {
            let l = at(c, r);
            if l < 0 {
                continue;
            }
            let e = edges.entry(l).or_default();
            let mut add = |a: V, b: V| e.entry(a).or_default().push(b);
            if at(c, r - 1) != l {
                add((c, r), (c + 1, r));
            }
            if at(c + 1, r) != l {
                add((c + 1, r), (c + 1, r + 1));
            }
            if at(c, r + 1) != l {
                add((c + 1, r + 1), (c, r + 1));
            }
            if at(c - 1, r) != l {
                add((c, r + 1), (c, r));
            }
        }
    }
    let mut out = HashMap::new();
    for (l, mut next) in edges {
        let mut rings = Vec::new();
        let mut starts: Vec<V> = next.keys().copied().collect();
        starts.sort();
        for s in starts {
            while next.get(&s).is_some_and(|v| !v.is_empty()) {
                let mut ring = vec![s];
                let mut prev = s;
                let mut cur = next.get_mut(&s).unwrap().pop().unwrap();
                while cur != s {
                    ring.push(cur);
                    let list = match next.get_mut(&cur) {
                        Some(v) if !v.is_empty() => v,
                        _ => break,
                    };
                    // At a pinch (two cells touching at a corner) turn
                    // towards the region: the pieces stay separate rings.
                    let din = (cur.0 - prev.0, cur.1 - prev.1);
                    let pick = (0..list.len())
                        .max_by_key(|&k| {
                            let dout = (list[k].0 - cur.0, list[k].1 - cur.1);
                            din.0 * dout.1 - din.1 * dout.0
                        })
                        .unwrap();
                    prev = cur;
                    cur = list.swap_remove(pick);
                }
                // Keep the corners only.
                let n = ring.len();
                let corners: Vec<XY> = (0..n)
                    .filter(|&k| {
                        let (a, b, c) = (ring[(k + n - 1) % n], ring[k], ring[(k + 1) % n]);
                        (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0) != 0
                    })
                    .map(|k| [g.x0 + ring[k].0 as f64 * g.cell, g.y0 + ring[k].1 as f64 * g.cell])
                    .collect();
                if corners.len() >= 3 {
                    rings.push(corners);
                }
            }
        }
        out.insert(l, rings);
    }
    out
}

/// A grid of the TIN's heights (drawing units in, metres out).
pub fn grid_from_tin(tin: &Tin, cell_m: f64, f: f64) -> Result<Grid, String> {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &tin.points {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let cell = cell_m * f;
    let (w, h) = (((hi[0] - lo[0]) / cell).ceil().max(1.0) as usize, ((hi[1] - lo[1]) / cell).ceil().max(1.0) as usize);
    if w * h > 4_000_000 {
        return Err(t!("{w} × {h} cellen is te veel: kies een grotere cel (CEL=) dan {cell_m} m", "{w} × {h} cells is too many: choose a cell (CEL=) larger than {cell_m} m"));
    }
    let mut g = Grid { x0: lo[0], y0: lo[1], cell, w, h, z: vec![f64::NAN; w * h], cell_m };
    for i in 0..w * h {
        if let Some(z) = tin.height(g.centre(i)) {
            g.z[i] = z / f;
        }
    }
    Ok(g)
}

// ── In the drawing ──────────────────────────────────────────────────────────

fn tool_layer(name: &str, aci: u8) -> LayerSpec {
    LayerSpec { name: name.into(), color: aci, rgb: crate::nlcs::color_rgb(aci), true_color: None, lineweight: None, linetype: "CONTINUOUS".into(), object_id: 0 }
}

/// The ground for a water drop: the TIN in the drawing (existing ground
/// first), else an AHN TIN in memory every `spacing` m.
fn surface(ctx: &Ctx, spacing: f64, progress: &dyn Fn(String)) -> Result<(Tin, String), String> {
    let doc = super::document(&["tin"], &[])?;
    if let Some(t) = Tin::from_document(&doc).or_else(|| Tin::from_document_with(&doc, &HashSet::new())) {
        let label = t!("TIN in de tekening ({} driehoeken)", "TIN in the drawing ({} triangles)", t.triangles.len());
        return Ok((t, label));
    }
    let bron = super::hoogtebron(ctx)?;
    progress(t!("{} ophalen…", "Fetching {}…", bron.label()));
    let pts = bron.grid_points(ctx, spacing, progress)?;
    crate::bridge::note_source(bron.attribution());
    let tin = Tin::build(&pts, spacing * 3.0 * ctx.f)?;
    Ok((tin, t!("{} om de {spacing} m (geen TIN in de tekening)", "{} every {spacing} m (no TIN in the drawing)", bron.label())))
}

/// Water drops from picked points, kept between clicks.
pub struct DropSession {
    ctx: Ctx,
    flow: Flow,
    source: String,
    count: usize,
}

impl DropSession {
    pub fn new(opt: &super::CivielOptions, spacing: f64, progress: &dyn Fn(String)) -> Result<DropSession, String> {
        let ctx = Ctx::new(opt)?;
        let (tin, source) = surface(&ctx, spacing, progress)?;
        Ok(DropSession { ctx, flow: Flow::new(tin), source, count: 0 })
    }

    /// Paths from drawing points, drawn as 3D polylines on OI-WATERDRUPPEL.
    pub fn run(&mut self, starts: &[XY]) -> Result<String, String> {
        let f = self.ctx.f;
        let k = self.ctx.k;
        let line_l = tool_layer("OI-WATERDRUPPEL", 5);
        let mut out = Out::default();
        let mut rows = Vec::new();
        let mut texts = Vec::new();
        for p in starts {
            let Some(path) = self.flow.drop_path(*p) else {
                texts.push(t!("{:.1}, {:.1}: buiten het terrein", "{:.1}, {:.1}: outside the surface", p[0], p[1]));
                continue;
            };
            self.count += 1;
            let n = self.count;
            let end = *path.points.last().unwrap();
            let (length, drop) = (path.length() / f, path.drop() / f);
            let d = record(&[
                ("waterdruppel", n.to_string()),
                ("lengte_m", format!("{length:.2}")),
                ("verval_m", format!("{drop:.3}")),
                ("einde", path.stop.label().to_string()),
                ("bron", self.source.clone()),
            ]);
            let mut pl = Polyline3D::from_points(path.points.iter().map(|q| v3([q[0], q[1]], q[2])).collect());
            pl.flags.closed = false;
            let mut e = EntityType::Polyline3D(pl);
            let c = e.common_mut();
            c.layer = line_l.name.clone();
            c.color = Color::ByLayer;
            c.extended_data.add_record(d.clone());
            out.put(&line_l, [e]);
            // Start and end marks.
            let mut ring = EntityType::Circle(Circle::from_center_radius(Vector3::new(end[0], end[1], end[2]), 0.8 * k));
            ring.common_mut().layer = line_l.name.clone();
            let mut start = EntityType::Circle(Circle::from_center_radius(Vector3::new(p[0], p[1], path.points[0][2]), 0.4 * k));
            start.common_mut().layer = line_l.name.clone();
            out.put(&line_l, [start, ring]);
            let mut t = self.ctx.text([end[0] + 1.2 * k, end[1]], format!("{} {}", nap(end[2] / f, 2), if path.stop == Stop::Pit { crate::lang::tr("put", "pit") } else { "" }).trim(), 1.8, 0.0, &line_l, None);
            if let EntityType::Text(tx) = &mut t {
                tx.horizontal_alignment = TextHorizontalAlignment::Left;
            }
            out.put(&line_l, [t]);
            let q = self.ctx.from_drawing([end[0], end[1]]);
            rows.push(serde_json::json!({
                "nummer": n,
                "lengte_m": (length * 100.0).round() / 100.0,
                "verval_m": (drop * 1000.0).round() / 1000.0,
                "einde": path.stop.label(),
                "eindpunt": [ (q[0] * 1000.0).round() / 1000.0, (q[1] * 1000.0).round() / 1000.0, (end[2] / f * 1000.0).round() / 1000.0 ],
                "punten": path.points.len(),
            }));
            texts.push(t!(
                "druppel {n}: {} m lang, {} m verval, einde: {} op {} m",
                "drop {n}: {} m long, {} m fall, end: {} at {} m",
                num(length, 1),
                num(drop, 2),
                path.stop.label(),
                nap(end[2] / f, 2)
            ));
        }
        if out.is_empty() {
            return Err(t!("geen waterdruppel: {}", "no water drop: {}", texts.join("; ")));
        }
        out.write(crate::lang::tr("Waterdruppel", "Water drop"))?;
        super::api::note("druppels", rows);
        super::api::note("bron", self.source.clone());
        Ok(t!("Waterdruppel over {} — {}.", "Water drop over {} — {}.", self.source, texts.join("; ")))
    }
}

/// Where the outlets of `CIV_STROOMGEBIED` come from.
#[derive(Clone, Debug, Default)]
pub struct Outlets {
    /// Drawing points.
    pub points: Vec<XY>,
    /// Layer patterns (`*` wildcards): points, blocks and circles on them.
    pub layers: Vec<String>,
}

pub(crate) fn layer_matches(pattern: &str, layer: &str) -> bool {
    let (p, l) = (pattern.to_ascii_uppercase(), layer.to_ascii_uppercase());
    let parts: Vec<&str> = p.split('*').collect();
    if parts.len() == 1 {
        return p == l;
    }
    let mut rest = l.as_str();
    for (k, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if k == 0 {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if k == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(i) => rest = &rest[i + part.len()..],
                None => return false,
            }
        }
    }
    true
}

pub struct CatchmentRequest {
    /// "TIN", "AHN" or automatic (the TIN when the drawing has one).
    pub source: Option<String>,
    pub cell: f64,
    pub outlets: Outlets,
    /// Snap outlets to the strongest flow within this many metres.
    pub snap: f64,
    /// Ponds: draw them, and their minimum depth (m) and area (m²).
    pub ponds: bool,
    pub min_depth: f64,
    pub min_area: f64,
}

/// The grid and its transform to the drawing.
struct Model {
    grid: Grid,
    to_drawing: Box<dyn Fn(XY) -> Option<XY>>,
    to_grid: Box<dyn Fn(XY) -> Option<XY>>,
    source: String,
}

fn model(ctx: &Ctx, req: &CatchmentRequest, progress: &dyn Fn(String)) -> Result<Model, String> {
    let want = req.source.as_deref().map(|s| s.to_ascii_uppercase());
    if want.as_deref() != Some("AHN") {
        let doc = super::document(&["tin"], &[])?;
        if let Some(t) = Tin::from_document(&doc).or_else(|| Tin::from_document_with(&doc, &HashSet::new())) {
            progress(tr("Raster van de TIN…", "Grid from the TIN…"));
            let grid = grid_from_tin(&t, req.cell, ctx.f)?;
            return Ok(Model { grid, to_drawing: Box::new(Some), to_grid: Box::new(Some), source: t!("TIN in de tekening, cel {} m", "TIN in the drawing, cell {} m", req.cell) });
        }
        if want.as_deref() == Some("TIN") {
            return Err(tr("geen TIN in de tekening: maak er een (CIV_TIN) of kies BRON=AHN", "no TIN in the drawing: make one (CIV_TIN) or choose BRON=AHN"));
        }
    }
    let site = ctx.site.clone().ok_or_else(|| tr("stel eerst de projectlocatie in (GIS_SITE): daar hangen de hoogtes aan", "set the project location first (GIS_SITE): the heights depend on it"))?;
    let src = crate::gis::terrain::Source::for_site(&site, crate::gis::ahn::Model::Dtm);
    let b = src.bbox(&site)?;
    let cells = ((b[2] - b[0]) / req.cell) * ((b[3] - b[1]) / req.cell);
    if cells > 4.0e6 {
        return Err(t!("{cells:.0} cellen is te veel: kies een grotere cel (CEL=) dan {} m", "{cells:.0} cells is too many: choose a cell (CEL=) larger than {} m", req.cell));
    }
    progress(t!("{} ophalen…", "Fetching {}…", src.label()));
    let a = src.fetch(b, req.cell, progress)?;
    // Row 0 at the bottom.
    let mut z = vec![f64::NAN; a.w * a.h];
    for r in 0..a.h {
        for c in 0..a.w {
            if let Some(v) = a.at(c, a.h - 1 - r) {
                z[r * a.w + c] = v;
            }
        }
    }
    let grid = Grid { x0: a.x0, y0: a.y1 - a.h as f64 * a.cell, cell: a.cell, w: a.w, h: a.h, z, cell_m: a.cell };
    let f = ctx.f;
    let to_site = crate::gis::vector::data_to_site(&site, &src.crs, &[])?;
    let s2 = site.clone();
    let to_drawing: Box<dyn Fn(XY) -> Option<XY>> = Box::new(move |p| to_site(p).map(|q| s2.to_drawing(q, f)));
    let (scrs, dcrs, s3) = (site.crs.clone(), src.crs.clone(), site.clone());
    let to_grid: Box<dyn Fn(XY) -> Option<XY>> = Box::new(move |p| {
        let q = s3.from_drawing(p, f);
        if scrs == dcrs { Some(q) } else { crate::gis::crs::transform(&scrs, &dcrs, q).ok() }
    });
    crate::bridge::note_source(src.attribution());
    Ok(Model { grid, to_drawing, to_grid, source: t!("{}, cel {} m", "{}, cell {} m", src.label(), a.cell) })
}

/// The outlets as drawing points with names: the given points and the
/// points, blocks and circles on the outlet layers.
fn outlet_points(o: &Outlets) -> Result<Vec<(String, XY)>, String> {
    let mut out: Vec<(String, XY)> = o.points.iter().enumerate().map(|(i, p)| (t!("punt {}", "point {}", i + 1), *p)).collect();
    if !o.layers.is_empty() {
        let doc = super::document(&[], &[])?;
        for e in doc.model_space_entities() {
            let layer = &e.common().layer;
            if !o.layers.iter().any(|p| layer_matches(p, layer)) {
                continue;
            }
            let at = match e {
                EntityType::Point(p) => [p.location.x, p.location.y],
                EntityType::Insert(i) => [i.insert_point.x, i.insert_point.y],
                EntityType::Circle(c) => [c.center.x, c.center.y],
                _ => continue,
            };
            let name = super::data_value(e, "puntnummer").or_else(|| super::data_value(e, "naam")).unwrap_or_else(|| super::api::handle_text(e.common().handle));
            out.push((name, at));
        }
    }
    Ok(out)
}

/// `CIV_STROOMGEBIED` (and `CIV_PLASSEN` without outlets): catchments per
/// outlet and the ponds of the filled depressions.
pub fn catchment(opt: &super::CivielOptions, req: CatchmentRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let wanted = outlet_points(&req.outlets)?;
    if wanted.is_empty() && !req.ponds {
        return Err(tr("geen afvoerpunten: geef PUNTEN=x,y;… of LAAG=<laag> (bijv. LAAG=*KOLK*), of klik ze aan", "no outlets: give PUNTEN=x,y;… or LAAG=<layer> (e.g. LAAG=*KOLK*), or click them"));
    }
    if !req.outlets.layers.is_empty() && wanted.len() == req.outlets.points.len() {
        return Err(t!("niets gevonden op laag {} (punten, blokken of cirkels)", "nothing found on layer {} (points, blocks or circles)", req.outlets.layers.join(";")));
    }
    let m = model(&ctx, &req, progress)?;
    let g = &m.grid;
    progress(tr("Afvoerrichtingen (Priority-Flood, D8)…", "Flow directions (Priority-Flood, D8)…"));
    let d = drainage(g);
    let k = ctx.k;
    let cell_a = g.cell_m * g.cell_m;
    let mut out = Out::default();
    // Outlets on the grid.
    let radius = (req.snap / g.cell_m).round().max(0.0) as usize;
    let mut cells: Vec<usize> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    for (name, p) in &wanted {
        let Some(c) = (m.to_grid)(*p).and_then(|q| g.cell_at(q)) else {
            skipped.push(t!("{name} (buiten het raster)", "{name} (outside the grid)"));
            continue;
        };
        // On a cell without a height (a building, water, a hole in the
        // TIN): the nearest cell with one, within the snap distance.
        let Some(c) = nearest_valid(g, c, radius.max(3)) else {
            skipped.push(t!("{name} (geen hoogte)", "{name} (no height)"));
            continue;
        };
        let c = snap(g, &d, c, radius);
        if cells.contains(&c) {
            skipped.push(t!("{name} (zelfde cel als een ander afvoerpunt)", "{name} (same cell as another outlet)"));
            continue;
        }
        cells.push(c);
        names.push(name.clone());
    }
    let mut rows = Vec::new();
    let mut summary = Vec::new();
    if !cells.is_empty() {
        let labels = catchments(g, &d, &cells);
        let mut count = vec![0usize; cells.len()];
        for l in &labels {
            if *l >= 0 {
                count[*l as usize] += 1;
            }
        }
        let rings = outlines(g, &labels);
        let line_l = tool_layer("OI-STROOMGEBIED", 4);
        let point_l = tool_layer("OI-STROOMGEBIED-AFVOERPUNT", 1);
        for (i, name) in names.iter().enumerate() {
            let area = count[i] as f64 * cell_a;
            let sg = format!("SG{}", i + 1);
            let dat = record(&[("stroomgebied", sg.clone()), ("afvoerpunt", name.clone()), ("oppervlakte_m2", format!("{area:.1}")), ("cellen", count[i].to_string()), ("bron", m.source.clone())]);
            let mut biggest: Option<(f64, Vec<XY>)> = None;
            for ring in rings.get(&(i as i32)).into_iter().flatten() {
                let simple = crate::gis::ahn::simplify(ring, g.cell * 0.75, true);
                let pts: Vec<XY> = simple.iter().filter_map(|p| (m.to_drawing)(*p)).collect();
                if pts.len() < 3 {
                    continue;
                }
                let a = super::percelen::area(&pts);
                if biggest.as_ref().is_none_or(|b| a > b.0) {
                    biggest = Some((a, pts.clone()));
                }
                out.put(&line_l, crate::emit::polylines(&[(pts, true)], &line_l.name, k, Some(&dat)));
            }
            let at = (m.to_drawing)(g.centre(cells[i])).unwrap_or([0.0, 0.0]);
            let mut mark = EntityType::Circle(Circle::from_center_radius(Vector3::new(at[0], at[1], 0.0), 1.0 * k));
            mark.common_mut().layer = point_l.name.clone();
            mark.common_mut().extended_data.add_record(dat.clone());
            out.put(&point_l, [mark]);
            if let Some((_, ring)) = biggest {
                let lp = super::percelen::label_point(&ring);
                let mut t = ctx.text(lp, &format!("{sg} · {} m²", super::percelen::m2(area)), 2.0, 0.0, &line_l, None);
                if let EntityType::Text(tx) = &mut t {
                    tx.horizontal_alignment = TextHorizontalAlignment::Center;
                    tx.vertical_alignment = TextVerticalAlignment::Middle;
                    tx.alignment_point = Some(tx.insertion_point);
                }
                out.put(&line_l, [t]);
            }
            let q = ctx.from_drawing(at);
            rows.push(serde_json::json!({"stroomgebied": sg, "afvoerpunt": name, "oppervlakte_m2": (area * 10.0).round() / 10.0, "cellen": count[i], "x": (q[0] * 100.0).round() / 100.0, "y": (q[1] * 100.0).round() / 100.0}));
            summary.push(format!("{sg} ({name}) {} m²", super::percelen::m2(area)));
        }
    }
    let mut pond_rows = Vec::new();
    let mut pond_total = (0usize, 0.0f64);
    if req.ponds {
        let found = ponds(g, &d, req.min_depth, req.min_area);
        let mut label = vec![-1i32; g.w * g.h];
        for (i, p) in found.iter().enumerate() {
            for &c in &p.cells {
                label[c] = i as i32;
            }
        }
        let rings = outlines(g, &label);
        let pond_l = tool_layer("OI-PLAS", 150);
        for (i, p) in found.iter().enumerate() {
            let name = t!("Plas {}", "Pond {}", i + 1);
            let dat = record(&[
                ("plas", (i + 1).to_string()),
                ("diepte_m", format!("{:.3}", p.depth)),
                ("volume_m3", format!("{:.2}", p.volume)),
                ("oppervlakte_m2", format!("{:.1}", p.area)),
                ("waterpeil_m", format!("{:.3}", p.level)),
                ("bron", m.source.clone()),
            ]);
            for ring in rings.get(&(i as i32)).into_iter().flatten() {
                let simple = crate::gis::ahn::simplify(ring, g.cell * 0.5, true);
                let pts: Vec<XY> = simple.iter().filter_map(|q| (m.to_drawing)(*q)).collect();
                if pts.len() >= 3 {
                    out.put(&pond_l, crate::emit::polylines(&[(pts, true)], &pond_l.name, k, Some(&dat)));
                }
            }
            let at = (m.to_drawing)(g.centre(p.deepest)).unwrap_or([0.0, 0.0]);
            let mut t = ctx.text(
                [at[0] + 1.2 * k, at[1]],
                &format!("{name} · {} m · {} m³", num(p.depth, 2), num(p.volume, 1)),
                1.8,
                0.0,
                &pond_l,
                None,
            );
            if let EntityType::Text(tx) = &mut t {
                tx.horizontal_alignment = TextHorizontalAlignment::Left;
            }
            out.put(&pond_l, [t]);
            let q = ctx.from_drawing(at);
            pond_rows.push(serde_json::json!({
                "plas": i + 1,
                "diepte_m": (p.depth * 1000.0).round() / 1000.0,
                "volume_m3": (p.volume * 100.0).round() / 100.0,
                "oppervlakte_m2": (p.area * 10.0).round() / 10.0,
                "waterpeil_m": (p.level * 1000.0).round() / 1000.0,
                "x": (q[0] * 100.0).round() / 100.0,
                "y": (q[1] * 100.0).round() / 100.0,
            }));
            pond_total.0 += 1;
            pond_total.1 += p.volume;
        }
    }
    if out.is_empty() {
        let why = if skipped.is_empty() { String::new() } else { format!(": {}", skipped.join(", ")) };
        return Err(t!("niets te tekenen ({}){why}", "nothing to draw ({}){why}", m.source));
    }
    let what = if cells.is_empty() { crate::lang::tr("Plassen", "Ponds") } else { crate::lang::tr("Stroomgebieden", "Catchments") };
    out.write(what)?;
    super::api::note("stroomgebieden", rows);
    super::api::note("plassen", pond_rows);
    super::api::note("bron", m.source.clone());
    super::api::note("raster", serde_json::json!({"breedte": g.w, "hoogte": g.h, "cel_m": g.cell_m}));
    if !skipped.is_empty() {
        super::api::note("overgeslagen", skipped.clone());
    }
    let mut msg = format!("{what} ({}): ", m.source);
    if !summary.is_empty() {
        msg.push_str(&summary.join(", "));
    }
    if req.ponds {
        if !summary.is_empty() {
            msg.push_str("; ");
        }
        msg.push_str(&t!(
            "{} plas(sen) dieper dan {} m, samen {} m³",
            "{} pond(s) deeper than {} m, {} m³ in all",
            pond_total.0,
            num(req.min_depth, 2),
            num(pond_total.1, 1)
        ));
    }
    if !skipped.is_empty() {
        msg.push_str(&t!(" — overgeslagen: {}", " — skipped: {}", skipped.join(", ")));
    }
    Ok(msg + ".")
}

// ── CIV_BUI: a storm on the ground ──────────────────────────────────────────

/// `CIV_BUI`: the storm, the ground and what takes water away.
pub struct StormRequest {
    /// The ground grid, as `CIV_STROOMGEBIED` makes it (`BRON=`, `CEL=`).
    pub ground: CatchmentRequest,
    /// `BUI=`: `08`, `T100`, `T250`, `T1000`, `48h`, `R100`…, `<mm>/<min>` or a CSV file.
    pub storm: String,
    /// Infiltration capacity (mm/h).
    pub infiltration: f64,
    /// Manning's n.
    pub manning: Option<f64>,
    /// Water leaves over the edge of the model (`RAND=open`).
    pub free_edge: bool,
    /// Gullies (`KOLKEN=` points or `LAAG=`) and their capacity (l/s each).
    pub sinks: Outlets,
    pub sink_capacity: f64,
    /// Minutes simulated after the rain.
    pub after: f64,
}

/// Depth classes (m) of the maximum water depth, as the stress test maps
/// show them (Klimaateffectatlas: 10, 15, 20 and 30 cm), with 5 cm below.
pub const DEPTH_CLASSES: [f64; 5] = [0.05, 0.10, 0.15, 0.20, 0.30];
const DEPTH_ACI: [u8; 5] = [151, 141, 150, 160, 170];

fn storm_of(spec: &str) -> Result<rekenkern::buien::Hyetograph, String> {
    let path = std::path::Path::new(spec);
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv") || e.eq_ignore_ascii_case("txt")) {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{spec}: {e}"))?;
        let unit = if text.to_ascii_lowercase().contains("mm/h") { rekenkern::buien::CsvValue::IntensityMmH } else { rekenkern::buien::CsvValue::DepthMm };
        return rekenkern::buien::Hyetograph::from_csv(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), &text, unit);
    }
    rekenkern::buien::parse_storm(spec)
}

/// The class (index in `DEPTH_CLASSES`) of a depth, −1 below the first.
pub fn depth_class(d: f64) -> i32 {
    DEPTH_CLASSES.iter().rposition(|c| d >= *c).map_or(-1, |i| i as i32)
}

/// `CIV_BUI`: a storm on the ground grid, simulated in time (local-inertial
/// overland flow, Bates et al. 2010): the maximum water depth in classes,
/// the ponds with their volume and the point where each one overflows, the
/// outflow over the edge and into gullies, and the water balance.
pub fn storm(opt: &super::CivielOptions, req: StormRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    use rekenkern::bui;
    let ctx = Ctx::new(opt)?;
    let rain = storm_of(&req.storm)?;
    let m = model(&ctx, &req.ground, progress)?;
    let g = &m.grid;
    if g.w * g.h > 400_000 {
        return Err(t!("{} cellen is te veel voor een bui in de tijd: kies een grotere cel (CEL=) of een kleiner gebied", "{} cells is too many for a storm in time: choose a larger cell (CEL=) or a smaller area", g.w * g.h));
    }
    let grid = bui::Grid { origin: [0.0, 0.0], cell: g.cell_m, nx: g.w, ny: g.h, z: g.z.clone() };
    let mut settings = bui::Settings { infiltration: bui::Field::Uniform(req.infiltration.max(0.0)), boundary: if req.free_edge { bui::Boundary::Free } else { bui::Boundary::Closed }, ..bui::Settings::default() };
    if let Some(n) = req.manning {
        settings.manning = bui::Field::Uniform(n);
    }
    let gullies = outlet_points(&req.sinks)?;
    for (_, p) in &gullies {
        if let Some(c) = (m.to_grid)(*p).and_then(|q| g.cell_at(q)) {
            settings.sinks.push(bui::Sink { at: grid.centre(c), capacity: req.sink_capacity / 1000.0 });
        }
    }
    let blocks: Vec<(f64, f64)> = rain.blocks.iter().map(|b| (b.duration_s, b.intensity_mm_h)).collect();
    settings.duration = rain.duration_s() + req.after.max(0.0) * 60.0;
    progress(t!("{} op {} × {} cellen van {} m…", "{} on {} × {} cells of {} m…", rain.name, g.w, g.h, num(g.cell_m, 2)));
    let flood = bui::simulate(&grid, &blocks, &settings)?;
    let k = ctx.k;
    let mut out = Out::default();
    // Maximum depth in classes.
    let label: Vec<i32> = flood.max_depth.iter().map(|d| depth_class(*d)).collect();
    let rings = outlines(g, &label);
    let mut class_area = [0.0f64; 5];
    for l in label.iter().filter(|l| **l >= 0) {
        class_area[*l as usize] += g.cell_m * g.cell_m;
    }
    for (i, c) in DEPTH_CLASSES.iter().enumerate() {
        let layer = tool_layer(&format!("OI-BUI-DIEPTE-{:02}CM", (c * 100.0).round() as i32), DEPTH_ACI[i]);
        let dat = record(&[("bui", rain.name.clone()), ("diepte_vanaf_m", format!("{c:.2}"))]);
        for ring in rings.get(&(i as i32)).into_iter().flatten() {
            let simple = crate::gis::ahn::simplify(ring, g.cell * 0.5, true);
            let pts: Vec<XY> = simple.iter().filter_map(|q| (m.to_drawing)(*q)).collect();
            if pts.len() >= 3 {
                out.put(&layer, crate::emit::polylines(&[(pts, true)], &layer.name, k, Some(&dat)));
            }
        }
    }
    // Ponds at the highest water, with where they overflow.
    let found = bui::ponds(&grid, &flood.max_depth, settings.pond_depth, settings.boundary);
    let pond_l = tool_layer("OI-BUI-PLAS", 5);
    let over_l = tool_layer("OI-BUI-OVERLOOP", 1);
    let mut pond_rows = Vec::new();
    for (i, p) in found.iter().enumerate().filter(|(_, p)| p.volume >= 0.5) {
        let at = (m.to_drawing)(g.centre(p.deepest)).unwrap_or([0.0, 0.0]);
        let text = format!("{} {} · {} m · {} m³", crate::lang::tr("Plas", "Pond"), i + 1, num(p.max_depth, 2), num(p.volume, 1));
        let mut t = ctx.text([at[0] + 1.2 * k, at[1]], &text, 1.8, 0.0, &pond_l, None);
        if let EntityType::Text(tx) = &mut t {
            tx.horizontal_alignment = TextHorizontalAlignment::Left;
        }
        out.put(&pond_l, [t]);
        let mut row = serde_json::json!({"plas": i + 1, "volume_m3": p.volume, "diepte_m": p.max_depth, "oppervlakte_m2": p.area, "waterpeil_m": p.level});
        if let Some(o) = &p.overflow {
            let q = (m.to_drawing)(g.centre(o.cell)).unwrap_or(at);
            let mut c = EntityType::Circle(Circle::from_center_radius(Vector3::new(q[0], q[1], 0.0), 1.0 * k));
            c.common_mut().layer = over_l.name.clone();
            out.put(&over_l, [c]);
            out.put(&over_l, [ctx.text([q[0] + 1.4 * k, q[1]], &t!("overloop {} ({} m)", "overflow {} ({} m)", i + 1, nap(o.level, 2)), 1.8, 0.0, &over_l, None)]);
            row["overloop_peil_m"] = serde_json::json!(o.level);
            row["overloop_marge_m"] = serde_json::json!(o.freeboard);
        }
        pond_rows.push(row);
    }
    if out.is_empty() {
        return Err(t!("geen water dieper dan {} m ({}, {})", "no water deeper than {} m ({}, {})", num(DEPTH_CLASSES[0], 2), rain.name, m.source));
    }
    out.write(crate::lang::tr("Bui", "Storm"))?;
    let b = &flood.balance;
    let wet: f64 = class_area.iter().sum();
    let max = flood.max_depth.iter().copied().fold(0.0, f64::max);
    let mut md = format!("# {}: {}\n\n{}: {} · {} mm · {} min\n\n", crate::lang::tr("Bui op het maaiveld", "Storm on the ground"), rain.name, crate::lang::tr("Bui", "Storm"), rain.name, num(rain.total_depth_mm(), 1), num(rain.duration_s() / 60.0, 0));
    md.push_str(&format!("{}: {}\n\n| {} | m² |\n|---|---|\n", crate::lang::tr("Ondergrond", "Ground"), m.source, crate::lang::tr("Maximale waterdiepte", "Maximum water depth")));
    for (i, c) in DEPTH_CLASSES.iter().enumerate() {
        md.push_str(&format!("| ≥ {} m | {} |\n", num(*c, 2), class_area[i].round()));
    }
    md.push_str(&format!(
        "\n| {} | m³ |\n|---|---|\n| {} | {} |\n| {} | {} |\n| {} | {} |\n| {} | {} |\n| {} | {} |\n| {} | {} % |\n",
        crate::lang::tr("Waterbalans", "Water balance"),
        crate::lang::tr("neerslag", "rain"), b.rain.round(),
        crate::lang::tr("berging op maaiveld", "stored on the ground"), b.storage.round(),
        crate::lang::tr("infiltratie", "infiltration"), b.infiltration.round(),
        crate::lang::tr("over de rand", "over the edge"), b.boundary.round(),
        crate::lang::tr("kolken", "gullies"), b.sinks.round(),
        crate::lang::tr("fout", "error"), num(b.relative_error * 100.0, 4)
    ));
    md.push_str(&t!(
        "\nLokaal-inertiaal model (Bates et al. 2010, LISFLOOD-FP) met Manning n {}, {} tijdstappen. Een stresstest-indicatie, geen vervanging van een gekalibreerd 2D-model met riolering.\n",
        "\nLocal-inertial model (Bates et al. 2010, LISFLOOD-FP) with Manning n {}, {} time steps. A stress test indication, not a replacement for a calibrated 2D model with the sewer.\n",
        req.manning.map(|n| num(n, 3)).unwrap_or_else(|| "0,030".into()),
        flood.steps
    ));
    let path = crate::bridge::beside_drawing("bui.md");
    let _ = std::fs::write(&path, md);
    super::api::note("bui", rain.name.clone());
    super::api::note("neerslag_mm", rain.total_depth_mm());
    super::api::note("max_diepte_m", max);
    super::api::note("nat_oppervlak_m2", wet);
    super::api::note("plassen", pond_rows.clone());
    super::api::note("waterbalans", serde_json::to_value(b).unwrap_or_default());
    super::api::note("rapport", path.to_string_lossy().into_owned());
    Ok(t!(
        "{} ({} mm) op {}: max {} m water, {} m² dieper dan {} m, {} plas(sen); over de rand {} m³, kolken {} m³; waterbalans {} %. Rapport: {}",
        "{} ({} mm) on {}: max {} m of water, {} m² deeper than {} m, {} pond(s); over the edge {} m³, gullies {} m³; water balance {} %. Report: {}",
        rain.name, num(rain.total_depth_mm(), 0), m.source, num(max, 2), wet.round(), num(DEPTH_CLASSES[0], 2), pond_rows.len(), b.boundary.round(), b.sinks.round(), num(b.relative_error * 100.0, 4), path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tin_of(f: impl Fn(f64, f64) -> f64, n: i32, step: f64) -> Tin {
        let mut pts = Vec::new();
        for i in -n..=n {
            for j in -n..=n {
                let (x, y) = (i as f64 * step, j as f64 * step);
                pts.push([x, y, f(x, y)]);
            }
        }
        Tin::build(&pts, 0.0).unwrap()
    }

    #[test]
    fn depth_classes() {
        assert_eq!(depth_class(0.01), -1);
        assert_eq!(depth_class(0.05), 0);
        assert_eq!(depth_class(0.149), 1);
        assert_eq!(depth_class(1.0), 4);
    }

    #[test]
    fn a_drop_runs_down_a_plane_to_the_edge() {
        // z = 0.1 x + 0.05 y: steepest descent is along (−2, −1).
        let flow = Flow::new(tin_of(|x, y| 0.1 * x + 0.05 * y, 10, 5.0));
        // A start whose gradient line meets no vertex of the 5 m grid.
        let p = flow.drop_path([10.3, 10.0]).unwrap();
        assert_eq!(p.stop, Stop::Edge);
        let end = p.points.last().unwrap();
        // Along the gradient line: (x − 10.3) = 2 (y − 10).
        for q in &p.points {
            assert!(((q[0] - 10.3) - 2.0 * (q[1] - 10.0)).abs() < 1e-6, "{q:?}");
            assert!((q[2] - (0.1 * q[0] + 0.05 * q[1])).abs() < 1e-9);
        }
        assert!((end[0] + 50.0).abs() < 1e-6, "{end:?}");
        assert!(p.points.windows(2).all(|w| w[1][2] <= w[0][2] + 1e-12), "always downhill");
    }

    #[test]
    fn a_drop_finds_the_bottom_of_a_bowl_and_a_valley() {
        let flow = Flow::new(tin_of(|x, y| (x * x + y * y) / 100.0, 8, 2.5));
        let p = flow.drop_path([13.3, -7.1]).unwrap();
        assert_eq!(p.stop, Stop::Pit);
        let end = p.points.last().unwrap();
        assert!(end[0].abs() < 2.6 && end[1].abs() < 2.6, "{end:?}");
        // A V-shaped valley along the y axis falling to −y: the drop reaches
        // the valley line and follows it down.
        let flow = Flow::new(tin_of(|x, y| x.abs() * 0.2 + y * 0.01, 10, 2.0));
        let p = flow.drop_path([12.3, 15.0]).unwrap();
        assert_eq!(p.stop, Stop::Edge);
        let end = p.points.last().unwrap();
        assert!(end[0].abs() < 1e-6 && (end[1] + 20.0).abs() < 1e-6, "{end:?}");
    }

    #[test]
    fn a_drop_let_go_on_an_edge_runs_on() {
        // The start lies on a triangle edge (the grid's points and their
        // diagonals): whichever triangle holds it, the drop runs down.
        let flow = Flow::new(tin_of(|x, y| 0.1 * x + 0.03 * y, 10, 2.0));
        for start in [[4.0, 4.0], [3.0, 3.0], [4.0, 3.0], [2.0, 7.0]] {
            let p = flow.drop_path(start).unwrap();
            assert_eq!(p.stop, Stop::Edge, "{start:?}");
            assert!(p.length() > 20.0, "{start:?}: {}", p.length());
        }
        // A V-shaped valley along the y axis, let go on the valley line.
        let flow = Flow::new(tin_of(|x, y| x.abs() * 0.2 + y * 0.01, 10, 2.0));
        let p = flow.drop_path([0.0, 5.0]).unwrap();
        let end = p.points.last().unwrap();
        assert!(end[0].abs() < 1e-6 && (end[1] + 20.0).abs() < 1e-6, "{end:?}");
    }

    #[test]
    fn a_drop_crosses_level_ground() {
        // A level terrace (z = 1) for x < 10, falling to the east beyond it:
        // a drop let go on the terrace walks across it to the nearest way
        // down (the slope, not the far western edge) and runs down.
        let flow = Flow::new(tin_of(|x, _| if x < 10.0 { 1.0 } else { 1.0 - (x - 10.0) * 0.1 }, 10, 2.5));
        let p = flow.drop_path([5.3, 3.1]).unwrap();
        assert_eq!(p.stop, Stop::Edge, "{:?}", p.points.last());
        let end = p.points.last().unwrap();
        assert!((end[0] - 25.0).abs() < 1e-6 && end[2] < 0.0, "{end:?}");
        assert!(p.length() > 19.0, "{}", p.length());
        // A level bowl bottom without a way out is a pit.
        let flow = Flow::new(tin_of(|x, y| (x.abs().max(y.abs()) - 5.0).max(0.0), 6, 2.5));
        let p = flow.drop_path([1.1, 0.7]).unwrap();
        assert_eq!(p.stop, Stop::Pit);
    }

    fn grid(w: usize, h: usize, f: impl Fn(f64, f64) -> f64) -> Grid {
        let mut z = vec![0.0; w * h];
        for r in 0..h {
            for c in 0..w {
                z[r * w + c] = f(c as f64 + 0.5, r as f64 + 0.5);
            }
        }
        Grid { x0: 0.0, y0: 0.0, cell: 1.0, w, h, z, cell_m: 1.0 }
    }

    #[test]
    fn a_pit_is_filled_and_drains_out() {
        // A tilted plane (draining to x = 0) with a pit 0.3 m deep at (10, 10).
        let g = grid(20, 20, |x, y| {
            let d = ((x - 10.5).powi(2) + (y - 10.5).powi(2)).sqrt();
            0.05 * x + if d < 2.0 { -0.3 * (1.0 - d / 2.0) } else { 0.0 }
        });
        let d = drainage(&g);
        // Every cell reaches the outside.
        for i in 0..g.w * g.h {
            let mut j = i;
            let mut steps = 0;
            while d.down[j] != OUT {
                j = d.down[j] as usize;
                steps += 1;
                assert!(steps < 1000, "cycle at {i}");
            }
        }
        let p = ponds(&g, &d, 0.05, 0.0);
        assert_eq!(p.len(), 1);
        assert!(p[0].depth > 0.15 && p[0].depth < 0.31, "{}", p[0].depth);
        // The volume is the sum of the depths: positive and below the cone's.
        let cone = std::f64::consts::PI * 4.0 * 0.3 / 3.0;
        assert!(p[0].volume > 0.0 && p[0].volume < cone + 0.5, "{}", p[0].volume);
        // The whole grid drains to x = 0: the total accumulation at the left
        // column cells is the grid.
        let left: u32 = (0..g.h).map(|r| d.acc[r * g.w]).sum();
        assert_eq!(left as usize, g.w * g.h);
    }

    #[test]
    fn two_outlets_split_a_ridge() {
        // A ridge along x = 10: the left half falls to the west, the right
        // half to the east; outlets halfway down each side.
        let g = grid(20, 10, |x, y| 5.0 - (x - 10.0).abs() * 0.2 + y * 0.001);
        let d = drainage(&g);
        let west = g.cell_at([0.5, 5.5]).unwrap();
        let east = g.cell_at([19.5, 5.5]).unwrap();
        let labels = catchments(&g, &d, &[west, east]);
        let count = |k: i32| labels.iter().filter(|l| **l == k).count();
        // The west outlet gets (part of) the west slope, never the east one.
        assert!(count(0) > 0 && count(1) > 0);
        for r in 0..g.h {
            for c in 0..g.w {
                let l = labels[r * g.w + c];
                if c < 10 {
                    assert_ne!(l, 1);
                } else {
                    assert_ne!(l, 0);
                }
            }
        }
        // Outlines: a rectangle-ish ring per region, area = cells.
        let rings = outlines(&g, &labels);
        for (l, rs) in &rings {
            let a: f64 = rs.iter().map(|r| super::super::percelen::signed_area(r)).sum();
            assert!((a - count(*l) as f64).abs() < 1e-9, "label {l}: {a} vs {}", count(*l));
        }
    }

    #[test]
    fn outlines_of_a_ring_and_a_pinch() {
        // A 4 × 4 block with a hole in the middle, and two cells touching at a corner.
        let mut label = vec![-1i32; 10 * 10];
        for r in 1..5 {
            for c in 1..5 {
                label[r * 10 + c] = 0;
            }
        }
        label[2 * 10 + 2] = -1;
        label[7 * 10 + 7] = 1;
        label[8 * 10 + 8] = 1;
        let g = Grid { x0: 0.0, y0: 0.0, cell: 1.0, w: 10, h: 10, z: vec![0.0; 100], cell_m: 1.0 };
        let rings = outlines(&g, &label);
        let a0: f64 = rings[&0].iter().map(|r| super::super::percelen::signed_area(r)).sum();
        assert_eq!(rings[&0].len(), 2);
        assert!((a0 - 15.0).abs() < 1e-9);
        assert_eq!(rings[&1].len(), 2, "corner-touching cells stay two rings");
        assert!(layer_matches("*KOLK*", "B-XX-RI-GWA_KOLK-S"));
        assert!(layer_matches("b-xx-ri-*", "B-XX-RI-GWA_KOLK-S"));
        assert!(!layer_matches("*PUT", "B-XX-RI-GWA_KOLK-S"));
    }
}
