//! Thinning a point cloud for a TIN, in two steps:
//!
//! 1. While the cloud streams, one point per cell of a square grid: the
//!    measured point nearest the cell centre (of the wanted classes), and
//!    the lowest point of any class (for the ground filter). The grid
//!    doubles its cell when it would hold more cells than allowed, so a
//!    city-size area fits in memory.
//! 2. Greedy insertion after Garland & Heckbert (1995): start from the
//!    convex hull and a coarse sample, triangulate, and insert in every
//!    triangle the point that lies farthest from it vertically, until every
//!    point lies within the tolerance or the triangle budget is spent. Flat
//!    ground keeps few points, curbs, ditches and slopes many.
//!
//! Units are those of the points (drawing units in Open Infra).

use std::collections::{HashMap, HashSet};

use crate::lang::tr;

pub type P3 = [f64; 3];

#[derive(Clone, Copy, Debug)]
struct Cell {
    /// The wanted point nearest the cell centre; `keep_d` is its squared
    /// distance (infinite: none yet).
    keep: P3,
    keep_d: f64,
    /// The lowest point of any class (z infinite: none).
    low: P3,
}

/// One point per grid cell, kept while a cloud streams.
pub struct CellGrid {
    cell: f64,
    cells: HashMap<(i64, i64), Cell>,
    max_cells: usize,
    track_low: bool,
}

impl CellGrid {
    pub fn new(cell: f64, max_cells: usize, track_low: bool) -> CellGrid {
        CellGrid { cell: cell.max(1e-9), cells: HashMap::new(), max_cells: max_cells.max(16), track_low }
    }

    /// The cell size now (it doubles when the grid grows too large).
    pub fn cell(&self) -> f64 {
        self.cell
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    fn key(&self, p: P3) -> (i64, i64) {
        ((p[0] / self.cell).floor() as i64, (p[1] / self.cell).floor() as i64)
    }

    fn dist(&self, k: (i64, i64), p: P3) -> f64 {
        let c = [(k.0 as f64 + 0.5) * self.cell, (k.1 as f64 + 0.5) * self.cell];
        (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)
    }

    /// Add a point: `wanted` points compete for the cell's point; every
    /// point counts for the lowest.
    pub fn add(&mut self, p: P3, wanted: bool) {
        if !wanted && !self.track_low {
            return;
        }
        let k = self.key(p);
        let d = self.dist(k, p);
        let track_low = self.track_low;
        let e = self.cells.entry(k).or_insert(Cell { keep: p, keep_d: f64::INFINITY, low: [p[0], p[1], f64::INFINITY] });
        if wanted && d < e.keep_d {
            e.keep = p;
            e.keep_d = d;
        }
        if track_low && p[2] < e.low[2] {
            e.low = p;
        }
        if self.cells.len() > self.max_cells {
            self.coarsen();
        }
    }

    /// Double the cell: four cells become one, keeping the point nearest
    /// the new centre and the lowest.
    fn coarsen(&mut self) {
        self.cell *= 2.0;
        let old = std::mem::take(&mut self.cells);
        for ((i, j), c) in old {
            let k = (i.div_euclid(2), j.div_euclid(2));
            let d = if c.keep_d.is_finite() { self.dist(k, c.keep) } else { f64::INFINITY };
            match self.cells.get_mut(&k) {
                Some(e) => {
                    if d < e.keep_d {
                        e.keep = c.keep;
                        e.keep_d = d;
                    }
                    if c.low[2] < e.low[2] {
                        e.low = c.low;
                    }
                }
                None => {
                    self.cells.insert(k, Cell { keep: c.keep, keep_d: d, low: c.low });
                }
            }
        }
    }

    /// The cells' wanted points.
    pub fn kept(&self) -> Vec<P3> {
        let mut v: Vec<P3> = self.cells.values().filter(|c| c.keep_d.is_finite()).map(|c| c.keep).collect();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        v
    }

    /// The cells' lowest points.
    pub fn lowest(&self) -> Vec<P3> {
        let mut v: Vec<P3> = self.cells.values().filter(|c| c.low[2].is_finite()).map(|c| c.low).collect();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        v
    }
}

/// A TIN chosen from the points: the kept points and their triangles
/// (indices into `points`).
#[derive(Debug, Clone)]
pub struct Greedy {
    pub points: Vec<P3>,
    pub triangles: Vec<[usize; 3]>,
    /// Largest vertical distance of a covered input point to the TIN.
    pub max_error: f64,
    /// The triangle budget stopped the insertion before the tolerance was met.
    pub capped: bool,
    pub passes: usize,
}

fn area2(a: P3, b: P3, c: P3) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])
}

/// Convex hull (Andrew's monotone chain), indices into `pts`.
fn hull(pts: &[P3]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..pts.len()).collect();
    idx.sort_by(|&a, &b| pts[a][0].total_cmp(&pts[b][0]).then(pts[a][1].total_cmp(&pts[b][1])));
    let cross = |o: usize, a: usize, b: usize| area2(pts[o], pts[a], pts[b]);
    let mut lower: Vec<usize> = Vec::new();
    for &i in &idx {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], i) <= 0.0 {
            lower.pop();
        }
        lower.push(i);
    }
    let mut upper: Vec<usize> = Vec::new();
    for &i in idx.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], i) <= 0.0 {
            upper.pop();
        }
        upper.push(i);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Where the data is: occupied cells of `cell`, to tell a triangle over a
/// gap in the cloud (a building, a pond without points) from one over data.
struct Occupancy {
    cell: f64,
    reach: f64,
    set: HashSet<(i64, i64)>,
}

impl Occupancy {
    fn new(pts: &[P3], reach: f64) -> Occupancy {
        let cell = (reach / 2.0).max(1e-9);
        let set = pts.iter().map(|p| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64)).collect();
        Occupancy { cell, reach, set }
    }

    /// Whether a data point lies within about `reach` of `p`.
    fn near(&self, p: [f64; 2]) -> bool {
        let (i, j) = ((p[0] / self.cell).floor() as i64, (p[1] / self.cell).floor() as i64);
        let k = (self.reach / self.cell).ceil() as i64;
        for di in -k..=k {
            for dj in -k..=k {
                if !self.set.contains(&(i + di, j + dj)) {
                    continue;
                }
                // Distance from p to that cell's square.
                let x0 = (i + di) as f64 * self.cell;
                let y0 = (j + dj) as f64 * self.cell;
                let dx = (x0 - p[0]).max(0.0).max(p[0] - (x0 + self.cell));
                let dy = (y0 - p[1]).max(0.0).max(p[1] - (y0 + self.cell));
                if dx.hypot(dy) <= self.reach {
                    return true;
                }
            }
        }
        false
    }
}

/// Triangles per grid cell, to find the triangle under a point.
struct Locator {
    origin: [f64; 2],
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl Locator {
    fn new(pts: &[P3], tris: &[[usize; 3]], bbox: [f64; 4]) -> Locator {
        let n = (tris.len() as f64).sqrt().ceil().max(1.0);
        let size = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]).max(1e-9);
        let cell = size / n;
        let cols = ((bbox[2] - bbox[0]) / cell).floor() as usize + 1;
        let rows = ((bbox[3] - bbox[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); cols * rows];
        for (t, tri) in tris.iter().enumerate() {
            let [a, b, c] = tri.map(|i| pts[i]);
            let c0 = (((a[0].min(b[0]).min(c[0])) - bbox[0]) / cell).floor().max(0.0) as usize;
            let c1 = (((a[0].max(b[0]).max(c[0])) - bbox[0]) / cell).floor().max(0.0) as usize;
            let r0 = (((a[1].min(b[1]).min(c[1])) - bbox[1]) / cell).floor().max(0.0) as usize;
            let r1 = (((a[1].max(b[1]).max(c[1])) - bbox[1]) / cell).floor().max(0.0) as usize;
            for r in r0..=r1.min(rows - 1) {
                for cc in c0..=c1.min(cols - 1) {
                    cells[r * cols + cc].push(t as u32);
                }
            }
        }
        Locator { origin: [bbox[0], bbox[1]], cell, cols, rows, cells }
    }

    /// The triangle holding `p` and the TIN height there.
    fn find(&self, pts: &[P3], tris: &[[usize; 3]], p: P3) -> Option<(usize, f64)> {
        let c = ((p[0] - self.origin[0]) / self.cell).floor();
        let r = ((p[1] - self.origin[1]) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        for &t in &self.cells[r as usize * self.cols + c as usize] {
            let [a, b, cc] = tris[t as usize].map(|i| pts[i]);
            let d = area2(a, b, cc);
            if d.abs() < 1e-18 {
                continue;
            }
            let (u, v, w) = (area2(p, b, cc) / d, area2(a, p, cc) / d, area2(a, b, p) / d);
            let eps = -1e-9;
            if u >= eps && v >= eps && w >= eps {
                return Some((t as usize, u * a[2] + v * b[2] + w * cc[2]));
            }
        }
        None
    }
}

/// Whether part of a triangle lies over a gap in the data: its centre, or a
/// point of a lattice over it at about the gap size, is far from any point.
fn over_gap(o: &Occupancy, a: P3, b: P3, c: P3) -> bool {
    let longest = [(a, b), (b, c), (c, a)].iter().map(|(p, q)| (q[0] - p[0]).hypot(q[1] - p[1])).fold(0.0, f64::max);
    let n = ((longest / o.reach).ceil() as usize).clamp(1, 64);
    let at = |u: f64, v: f64| [a[0] + (b[0] - a[0]) * u + (c[0] - a[0]) * v, a[1] + (b[1] - a[1]) * u + (c[1] - a[1]) * v];
    if !o.near(at(1.0 / 3.0, 1.0 / 3.0)) {
        return true;
    }
    for i in 0..=n {
        for j in 0..=(n - i) {
            let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
            if !o.near(at(u, v)) {
                return true;
            }
        }
    }
    false
}

/// Delaunay triangles of the chosen points (indices into `pts`), with a
/// flag per triangle: true where its centre lies over a gap in the data.
fn triangulate(pts: &[P3], chosen: &[usize], centre: [f64; 2], occ: Option<&Occupancy>) -> (Vec<[usize; 3]>, Vec<bool>) {
    let d: Vec<delaunator::Point> = chosen.iter().map(|&i| delaunator::Point { x: pts[i][0] - centre[0], y: pts[i][1] - centre[1] }).collect();
    let tri = delaunator::triangulate(&d);
    let mut tris = Vec::with_capacity(tri.triangles.len() / 3);
    let mut holes = Vec::with_capacity(tri.triangles.len() / 3);
    for t in tri.triangles.as_chunks::<3>().0 {
        let ids = [chosen[t[0]], chosen[t[1]], chosen[t[2]]];
        let [a, b, c] = ids.map(|i| pts[i]);
        if area2(a, b, c).abs() < 1e-12 {
            continue;
        }
        holes.push(occ.is_some_and(|o| over_gap(o, a, b, c)));
        tris.push(ids);
    }
    (tris, holes)
}

/// Greedy insertion: a TIN of a subset of `pts` within `tol` (vertical) of
/// every point, with at most about `max_triangles` triangles. Triangles
/// whose centre lies farther than `gap` from any point are left out (gaps
/// in the cloud stay holes); `gap` 0 keeps every triangle.
pub fn greedy_tin(pts: &[P3], tol: f64, max_triangles: usize, gap: f64) -> Result<Greedy, String> {
    if pts.len() < 3 {
        return Err(trf!("een terrein heeft minstens drie punten nodig ({} gevonden)", "a terrain needs at least three points ({} found)", pts.len()));
    }
    let bbox = pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let centre = [(bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0];
    let occ = (gap > 0.0).then(|| Occupancy::new(pts, gap));
    let max_points = (max_triangles / 2).max(4);
    // Start: the convex hull and a coarse sample (about 8 × 8 blocks).
    let mut chosen: Vec<usize> = hull(pts);
    if chosen.len() < 3 {
        return Err(tr("geen driehoeken: de punten liggen op één lijn", "no triangles: the points lie on one line").into());
    }
    let mut in_set = vec![false; pts.len()];
    for &i in &chosen {
        in_set[i] = true;
    }
    let block = ((bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) / 8.0).max(1e-9);
    let mut sample: HashMap<(i64, i64), (f64, usize)> = HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        let k = ((p[0] / block).floor() as i64, (p[1] / block).floor() as i64);
        let c = [(k.0 as f64 + 0.5) * block, (k.1 as f64 + 0.5) * block];
        let d = (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2);
        let e = sample.entry(k).or_insert((f64::INFINITY, i));
        if d < e.0 {
            *e = (d, i);
        }
    }
    let mut extra: Vec<usize> = sample.values().map(|v| v.1).filter(|i| !in_set[*i]).collect();
    extra.sort_unstable();
    if chosen.len() + extra.len() > max_points {
        extra.truncate(max_points.saturating_sub(chosen.len()));
    }
    for i in extra {
        in_set[i] = true;
        chosen.push(i);
    }
    let mut passes = 0;
    let mut capped = false;
    loop {
        passes += 1;
        let (tris, holes) = triangulate(pts, &chosen, centre, occ.as_ref());
        if tris.is_empty() {
            return Err(tr("geen driehoeken: liggen de punten op één lijn?", "no triangles: do the points lie on one line?").into());
        }
        let loc = Locator::new(pts, &tris, bbox);
        // Per triangle the point farthest from it; a point under a hole
        // triangle is data where the TIN has none, so it goes first.
        let mut worst: Vec<(f64, usize)> = vec![(0.0, usize::MAX); tris.len()];
        let mut max_error: f64 = 0.0;
        for (i, p) in pts.iter().enumerate() {
            if in_set[i] {
                continue;
            }
            let Some((t, z)) = loc.find(pts, &tris, *p) else { continue };
            let err = if holes[t] { f64::INFINITY } else { (p[2] - z).abs() };
            if !holes[t] {
                max_error = max_error.max(err);
            }
            if err > worst[t].0 {
                worst[t] = (err, i);
            }
        }
        let mut add: Vec<(f64, usize)> = worst.into_iter().filter(|(e, i)| *i != usize::MAX && *e > tol).collect();
        let done = add.is_empty() || capped;
        if done {
            let triangles: Vec<[usize; 3]> = tris.iter().zip(&holes).filter(|(_, h)| !**h).map(|(t, _)| *t).collect();
            if triangles.is_empty() {
                return Err(tr("geen driehoeken over: de punten liggen te ver uit elkaar voor de gatgrootte (GAT)", "no triangles left: the points lie too far apart for the gap size (GAT)").into());
            }
            return Ok(Greedy { points: pts.to_vec(), triangles, max_error, capped, passes });
        }
        if chosen.len() + add.len() > max_points {
            add.sort_by(|a, b| b.0.total_cmp(&a.0));
            add.truncate(max_points.saturating_sub(chosen.len()));
            capped = true;
        }
        if add.is_empty() {
            capped = true;
        }
        for (_, i) in add {
            in_set[i] = true;
            chosen.push(i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(n: usize, step: f64, z: impl Fn(f64, f64) -> f64) -> Vec<P3> {
        let mut v = Vec::new();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f64 * step, j as f64 * step);
                v.push([x, y, z(x, y)]);
            }
        }
        v
    }

    /// The vertical distance of every point to the TIN (None: not covered).
    fn errors(g: &Greedy, pts: &[P3]) -> Vec<Option<f64>> {
        let tin_pts = &g.points;
        let bbox = pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        let loc = Locator::new(tin_pts, &g.triangles, bbox);
        pts.iter().map(|p| loc.find(tin_pts, &g.triangles, *p).map(|(_, z)| (p[2] - z).abs())).collect()
    }

    #[test]
    fn cells_keep_the_centre_point_and_the_lowest() {
        let mut g = CellGrid::new(1.0, 1000, true);
        g.add([0.1, 0.1, 5.0], true);
        g.add([0.45, 0.55, 6.0], true);
        g.add([0.9, 0.9, 1.0], false);
        assert_eq!(g.len(), 1);
        assert_eq!(g.kept(), vec![[0.45, 0.55, 6.0]]);
        assert_eq!(g.lowest(), vec![[0.9, 0.9, 1.0]]);
        // A cell with only unwanted points has a lowest but no kept point.
        g.add([3.5, 3.5, 2.0], false);
        assert_eq!(g.kept().len(), 1);
        assert_eq!(g.lowest().len(), 2);
    }

    #[test]
    fn a_full_grid_coarsens() {
        let mut g = CellGrid::new(0.5, 100, false);
        for p in field(40, 0.25, |_, _| 0.0) {
            g.add(p, true);
        }
        // 10 × 10 m in cells of 0.5 m is 400 cells: coarsened to 1 m, 100 cells.
        assert!(g.len() <= 100, "{}", g.len());
        assert_eq!(g.cell(), 1.0);
        assert_eq!(g.kept().len(), g.len());
    }

    #[test]
    fn a_plane_needs_few_points() {
        let pts = field(41, 0.5, |x, y| 1.0 + 0.02 * x - 0.01 * y);
        let g = greedy_tin(&pts, 0.01, 100_000, 0.0).unwrap();
        let used: HashSet<usize> = g.triangles.iter().flatten().copied().collect();
        assert!(used.len() < 100, "{} points for a plane", used.len());
        assert!(g.max_error < 1e-9);
        assert!(errors(&g, &pts).iter().all(|e| e.is_some_and(|e| e < 1e-9)));
    }

    #[test]
    fn every_point_lies_within_the_tolerance() {
        // A ditch and a dike across gently undulating ground.
        let z = |x: f64, y: f64| 0.3 * (x / 7.0).sin() + 0.2 * (y / 5.0).cos() + if (x - 10.0).abs() < 1.5 { -1.0 } else { 0.0 } + (1.5 - (y - 14.0).abs()).max(0.0);
        let pts = field(61, 0.4, z);
        let g = greedy_tin(&pts, 0.05, 1_000_000, 0.0).unwrap();
        assert!(!g.capped);
        let errs = errors(&g, &pts);
        assert!(errs.iter().all(|e| e.is_some_and(|e| e <= 0.05 + 1e-9)), "max {:?}", errs.iter().flatten().cloned().fold(0.0, f64::max));
        let used: HashSet<usize> = g.triangles.iter().flatten().copied().collect();
        assert!(used.len() < pts.len() / 2, "{} of {}", used.len(), pts.len());
    }

    #[test]
    fn the_budget_caps_the_triangles() {
        let pts = field(61, 0.4, |x, y| (x * 1.3).sin() + (y * 0.7).cos());
        let g = greedy_tin(&pts, 0.001, 400, 0.0).unwrap();
        assert!(g.capped);
        assert!(g.triangles.len() <= 450, "{}", g.triangles.len());
    }

    #[test]
    fn a_gap_in_the_cloud_stays_a_hole() {
        // A building of 8 × 8 m in a field of 30 × 30 m without points.
        let pts: Vec<P3> = field(61, 0.5, |_, _| 1.0).into_iter().filter(|p| !(p[0] > 11.0 && p[0] < 19.0 && p[1] > 11.0 && p[1] < 19.0)).collect();
        let g = greedy_tin(&pts, 0.05, 100_000, 2.0).unwrap();
        let hit = |x: f64, y: f64| {
            g.triangles.iter().any(|t| {
                let [a, b, c] = t.map(|i| g.points[i]);
                let p = [x, y, 0.0];
                let d = area2(a, b, c);
                let (u, v, w) = (area2(p, b, c) / d, area2(a, p, c) / d, area2(a, b, p) / d);
                u >= 0.0 && v >= 0.0 && w >= 0.0
            })
        };
        assert!(!hit(15.0, 15.0), "the TIN spans the building");
        assert!(hit(5.0, 5.0) && hit(25.0, 25.0));
        // Every point is still covered.
        assert!(errors(&g, &pts).iter().all(|e| e.is_some()));
    }
}
