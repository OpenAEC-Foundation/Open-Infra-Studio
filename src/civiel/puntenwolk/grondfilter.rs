//! Finding the ground in a point cloud without classification: the Simple
//! Morphological Filter (SMRF) of Pingel, Clarke & McBride (2013).
//!
//! 1. A minimum surface: the lowest point per cell; low outliers (a cell far
//!    below all its neighbours) are left out; empty cells are filled from
//!    their neighbours.
//! 2. Progressive opening: the surface is opened (erosion, then dilation)
//!    with windows of growing radius up to `window`. A cell that the opening
//!    lowers by more than `slope × radius` stands on an object (a building,
//!    a tree) and drops out.
//! 3. The ground cells, filled again, give a provisional terrain model. A
//!    point is ground when it lies within `threshold + scalar × slope of the
//!    terrain` of that model.
//!
//! Open Infra uses a slope of 0.3 rather than SMRF's 0.15, so dikes and
//! embankments of about 1:3 stay ground (an estimate; `HELLING=` sets it).
//!
//! The windows are squares, as in the progressive morphological filter of
//! Zhang et al. (2003); SMRF uses discs. Openings run in time independent
//! of the window (van Herk 1992, Gil & Werman 1993). Coordinates in metres.

use crate::lang::tr;

pub type P3 = [f64; 3];

/// The SMRF parameters, with the defaults of Pingel et al. (2013) as PDAL's
/// `filters.smrf` uses them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smrf {
    /// Cell of the minimum surface (m).
    pub cell: f64,
    /// Terrain slope that is still ground (rise over run).
    pub slope: f64,
    /// Largest window radius (m): about half the largest building.
    pub window: f64,
    /// Height above the terrain model that is still ground (m).
    pub threshold: f64,
    /// Extra height per unit of terrain slope (m).
    pub scalar: f64,
}

impl Default for Smrf {
    fn default() -> Self {
        Smrf { cell: 1.0, slope: 0.15, window: 18.0, threshold: 0.5, scalar: 1.25 }
    }
}

/// Most cells of the minimum surface (about 400 MB of grids).
const MAX_CELLS: usize = 12_000_000;

/// A low outlier lies this far below every neighbouring cell (m). An
/// estimate: well beyond a ditch edge at 1 m cells, well within multipath
/// errors of a laser scan.
const LOW_OUTLIER: f64 = 1.0;

struct Grid {
    x0: f64,
    y0: f64,
    cell: f64,
    cols: usize,
    rows: usize,
}

impl Grid {
    fn index(&self, x: f64, y: f64) -> usize {
        let c = (((x - self.x0) / self.cell).floor() as usize).min(self.cols - 1);
        let r = (((y - self.y0) / self.cell).floor() as usize).min(self.rows - 1);
        r * self.cols + c
    }
}

/// Which points are ground.
pub fn ground(points: &[P3], p: &Smrf) -> Result<Vec<bool>, String> {
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let b = points.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]);
    let cell = p.cell.max(0.05);
    let cols = ((b[2] - b[0]) / cell).floor() as usize + 1;
    let rows = ((b[3] - b[1]) / cell).floor() as usize + 1;
    if cols * rows > MAX_CELLS {
        return Err(trf!(
            "grondfilter: {} cellen van {} m is te veel; kies een kleiner gebied of een grotere CEL",
            "ground filter: {} cells of {} m is too many; choose a smaller area or a larger CEL",
            cols * rows,
            cell
        ));
    }
    let g = Grid { x0: b[0], y0: b[1], cell, cols, rows };
    // 1. Minimum surface.
    let mut zmin = vec![f64::NAN; cols * rows];
    for q in points {
        let i = g.index(q[0], q[1]);
        if zmin[i].is_nan() || q[2] < zmin[i] {
            zmin[i] = q[2];
        }
    }
    // Low outliers: far below every neighbour that has a value.
    let mut low = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if zmin[i].is_nan() {
                continue;
            }
            let mut n = 0;
            let mut below_all = true;
            for (nc, nr) in neighbours(c, r, cols, rows) {
                let z = zmin[nr * cols + nc];
                if z.is_nan() {
                    continue;
                }
                n += 1;
                if zmin[i] > z - LOW_OUTLIER {
                    below_all = false;
                }
            }
            if n >= 3 && below_all {
                low.push(i);
            }
        }
    }
    for i in low {
        zmin[i] = f64::NAN;
    }
    let base = inpaint(&zmin, cols, rows).ok_or(tr("grondfilter: geen punten", "ground filter: no points"))?;
    // 2. Progressive opening.
    let mut object = vec![false; cols * rows];
    let mut prev = base;
    let radii = (p.window / cell).ceil().max(1.0) as usize;
    for radius in 1..=radii {
        let opened = open(&prev, cols, rows, radius);
        let t = p.slope * radius as f64 * cell;
        for i in 0..prev.len() {
            if prev[i] - opened[i] > t {
                object[i] = true;
            }
        }
        prev = opened;
    }
    // 3. Provisional terrain model and its slope.
    let ground_cells: Vec<f64> = zmin.iter().zip(&object).map(|(z, o)| if *o { f64::NAN } else { *z }).collect();
    let dem = inpaint(&ground_cells, cols, rows).ok_or(tr("grondfilter: alles lijkt bebouwing; kies een grotere HELLING of een kleiner VENSTER", "ground filter: everything looks like an object; choose a larger HELLING or a smaller VENSTER"))?;
    let at = |c: i64, r: i64| dem[(r.clamp(0, rows as i64 - 1) as usize) * cols + c.clamp(0, cols as i64 - 1) as usize];
    let mut slope = vec![0.0; cols * rows];
    for r in 0..rows as i64 {
        for c in 0..cols as i64 {
            let gx = (at(c + 1, r) - at(c - 1, r)) / (2.0 * cell);
            let gy = (at(c, r + 1) - at(c, r - 1)) / (2.0 * cell);
            slope[r as usize * cols + c as usize] = gx.hypot(gy);
        }
    }
    // Bilinear between cell centres.
    let sample = |grid: &[f64], x: f64, y: f64| -> f64 {
        let fx = ((x - g.x0) / cell - 0.5).clamp(0.0, (cols - 1) as f64);
        let fy = ((y - g.y0) / cell - 0.5).clamp(0.0, (rows - 1) as f64);
        let (c0, r0) = (fx.floor() as usize, fy.floor() as usize);
        let (c1, r1) = ((c0 + 1).min(cols - 1), (r0 + 1).min(rows - 1));
        let (u, v) = (fx - c0 as f64, fy - r0 as f64);
        let z = |c: usize, r: usize| grid[r * cols + c];
        z(c0, r0) * (1.0 - u) * (1.0 - v) + z(c1, r0) * u * (1.0 - v) + z(c0, r1) * (1.0 - u) * v + z(c1, r1) * u * v
    };
    Ok(points
        .iter()
        .map(|q| {
            let d = sample(&dem, q[0], q[1]);
            let s = slope[g.index(q[0], q[1])];
            (q[2] - d).abs() <= p.threshold + p.scalar * s
        })
        .collect())
}

/// A ground point more than this above the median of its neighbours is a
/// spike (a point low on a wall, a car roof): left out (m, an estimate).
pub const SPIKE: f64 = 0.4;

/// Spikes among ground points one per cell of `cell` m: points more than
/// `spike` above the median of the points within about a metre around them
/// (at least four). Returns which points to keep.
pub fn without_spikes(points: &[P3], cell: f64, spike: f64) -> Vec<bool> {
    let key = |p: &P3| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut at: std::collections::HashMap<(i64, i64), Vec<f64>> = std::collections::HashMap::new();
    for p in points {
        at.entry(key(p)).or_default().push(p[2]);
    }
    points
        .iter()
        .map(|p| {
            let (i, j) = key(p);
            let k = ((1.0 / cell).round() as i64).clamp(1, 4);
            let mut z: Vec<f64> = Vec::with_capacity(24);
            for di in -k..=k {
                for dj in -k..=k {
                    if di == 0 && dj == 0 {
                        continue;
                    }
                    if let Some(v) = at.get(&(i + di, j + dj)) {
                        z.extend(v);
                    }
                }
            }
            if z.len() < 4 {
                return true;
            }
            z.sort_by(f64::total_cmp);
            p[2] - z[z.len() / 2] <= spike
        })
        .collect()
}

fn neighbours(c: usize, r: usize, cols: usize, rows: usize) -> impl Iterator<Item = (usize, usize)> {
    let (c, r) = (c as i64, r as i64);
    [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
        .into_iter()
        .map(move |(dc, dr)| (c + dc, r + dr))
        .filter(move |(nc, nr)| *nc >= 0 && *nr >= 0 && (*nc as usize) < cols && (*nr as usize) < rows)
        .map(|(nc, nr)| (nc as usize, nr as usize))
}

/// Fill empty (NaN) cells: ring by ring from the cells with a value, each
/// the mean of its filled neighbours, then smoothed a little (a few Jacobi
/// steps of Laplace's equation on the filled cells only).
fn inpaint(z: &[f64], cols: usize, rows: usize) -> Option<Vec<f64>> {
    if z.iter().all(|v| v.is_nan()) {
        return None;
    }
    let mut out = z.to_vec();
    let empty: Vec<usize> = (0..z.len()).filter(|i| z[*i].is_nan()).collect();
    let mut todo = empty.clone();
    while !todo.is_empty() {
        let mut next = Vec::new();
        let mut set = Vec::new();
        for &i in &todo {
            let (c, r) = (i % cols, i / cols);
            let (mut sum, mut n) = (0.0, 0);
            for (nc, nr) in neighbours(c, r, cols, rows) {
                let v = out[nr * cols + nc];
                if !v.is_nan() {
                    sum += v;
                    n += 1;
                }
            }
            if n > 0 {
                set.push((i, sum / n as f64));
            } else {
                next.push(i);
            }
        }
        if set.is_empty() {
            break;
        }
        for (i, v) in set {
            out[i] = v;
        }
        todo = next;
    }
    for _ in 0..8 {
        let snapshot = out.clone();
        for &i in &empty {
            let (c, r) = (i % cols, i / cols);
            let (mut sum, mut n) = (0.0, 0);
            for (nc, nr) in neighbours(c, r, cols, rows) {
                sum += snapshot[nr * cols + nc];
                n += 1;
            }
            if n > 0 {
                out[i] = sum / n as f64;
            }
        }
    }
    Some(out)
}

/// Running minimum (or maximum) over windows of `2 r + 1` along a line, in
/// constant time per element (van Herk / Gil–Werman).
fn running(line: &[f64], r: usize, max: bool) -> Vec<f64> {
    let n = line.len();
    let w = 2 * r + 1;
    let pad = if max { f64::NEG_INFINITY } else { f64::INFINITY };
    let pick = |a: f64, b: f64| if max { a.max(b) } else { a.min(b) };
    let len = n + 2 * r;
    let padded_len = len.div_ceil(w) * w;
    let mut f = vec![pad; padded_len];
    f[r..r + n].copy_from_slice(line);
    let mut pre = f.clone();
    let mut suf = f.clone();
    for i in 1..padded_len {
        if i % w != 0 {
            pre[i] = pick(pre[i - 1], f[i]);
        }
    }
    for i in (0..padded_len - 1).rev() {
        if (i + 1) % w != 0 {
            suf[i] = pick(suf[i + 1], f[i]);
        }
    }
    (0..n).map(|i| pick(suf[i], pre[i + w - 1])).collect()
}

/// Erosion or dilation with a square of half-width `r` cells.
fn filter(z: &[f64], cols: usize, rows: usize, r: usize, max: bool) -> Vec<f64> {
    let mut rowwise = vec![0.0; z.len()];
    for row in 0..rows {
        let line = &z[row * cols..(row + 1) * cols];
        rowwise[row * cols..(row + 1) * cols].copy_from_slice(&running(line, r, max));
    }
    let mut out = vec![0.0; z.len()];
    let mut col = vec![0.0; rows];
    for c in 0..cols {
        for row in 0..rows {
            col[row] = rowwise[row * cols + c];
        }
        for (row, v) in running(&col, r, max).into_iter().enumerate() {
            out[row * cols + c] = v;
        }
    }
    out
}

/// Morphological opening: erosion, then dilation.
fn open(z: &[f64], cols: usize, rows: usize, r: usize) -> Vec<f64> {
    let eroded = filter(z, cols, rows, r, false);
    filter(&eroded, cols, rows, r, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_minimum_matches_the_naive_one() {
        let line: Vec<f64> = (0..37).map(|i| ((i * 7919) % 23) as f64).collect();
        for r in 0..6 {
            let fast = running(&line, r, false);
            for (i, v) in fast.iter().enumerate() {
                let lo = i.saturating_sub(r);
                let hi = (i + r).min(line.len() - 1);
                let naive = line[lo..=hi].iter().cloned().fold(f64::INFINITY, f64::min);
                assert_eq!(*v, naive, "r {r} i {i}");
            }
            let fast = running(&line, r, true);
            for (i, v) in fast.iter().enumerate() {
                let lo = i.saturating_sub(r);
                let hi = (i + r).min(line.len() - 1);
                assert_eq!(*v, line[lo..=hi].iter().cloned().fold(f64::NEG_INFINITY, f64::max));
            }
        }
    }

    /// A small deterministic random generator (the tests need no crate).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Synthetic airborne scan of 120 × 100 m: rolling ground with a dike
    /// (slopes 1:4),
    /// three buildings (roofs and a few wall points) and twenty trees
    /// (crowns over ground that still gets some pulses). Labels: true for
    /// ground.
    pub(crate) fn scene() -> (Vec<P3>, Vec<bool>) {
        let terrain = |x: f64, y: f64| 1.0 + 0.01 * x + 0.4 * (x / 15.0).sin() * (y / 20.0).cos() + (2.0 - (x - 95.0).abs() * 0.25).max(0.0);
        let buildings = [[10.0, 10.0, 30.0, 25.0, 9.0], [45.0, 50.0, 70.0, 80.0, 14.0], [15.0, 60.0, 25.0, 68.0, 6.0]];
        let mut rnd = Lcg(42);
        let trees: Vec<[f64; 3]> = (0..20).map(|_| [35.0 + rnd.next() * 55.0, 5.0 + rnd.next() * 40.0, 2.0 + rnd.next() * 2.5]).collect();
        let mut pts = Vec::new();
        let mut truth = Vec::new();
        let step = 0.5;
        let mut y = 0.0;
        while y < 100.0 {
            let mut x = 0.0;
            while x < 120.0 {
                let (px, py) = (x + rnd.next() * 0.3, y + rnd.next() * 0.3);
                let g = terrain(px, py) + (rnd.next() - 0.5) * 0.06;
                if let Some(b) = buildings.iter().find(|b| px > b[0] && px < b[2] && py > b[1] && py < b[3]) {
                    pts.push([px, py, terrain(px, py) + b[4] + (rnd.next() - 0.5) * 0.05]);
                    truth.push(false);
                } else if trees.iter().any(|t| (px - t[0]).hypot(py - t[1]) < t[2]) {
                    // Most pulses end in the crown; some reach the ground.
                    if rnd.next() < 0.7 {
                        pts.push([px, py, terrain(px, py) + 4.0 + rnd.next() * 6.0]);
                        truth.push(false);
                    } else {
                        pts.push([px, py, g]);
                        truth.push(true);
                    }
                } else {
                    pts.push([px, py, g]);
                    truth.push(true);
                }
                x += step;
            }
            y += step;
        }
        // Wall points along the building edges.
        for b in &buildings {
            for k in 0..40 {
                let t = k as f64 / 40.0;
                let (px, py) = (b[0] + (b[2] - b[0]) * t, b[1]);
                pts.push([px, py - 0.05, terrain(px, py) + b[4] * rnd.next()]);
                truth.push(false);
            }
        }
        // Two low outliers (multipath).
        pts.push([60.0, 20.0, -8.0]);
        truth.push(false);
        pts.push([100.0, 90.0, -6.0]);
        truth.push(false);
        (pts, truth)
    }

    #[test]
    fn smrf_finds_the_ground_under_buildings_and_trees() {
        let (pts, truth) = scene();
        let found = ground(&pts, &Smrf { slope: 0.3, ..Smrf::default() }).unwrap();
        let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
        for (f, t) in found.iter().zip(&truth) {
            match (*f, *t) {
                (true, true) => tp += 1,
                (true, false) => fp += 1,
                (false, true) => fn_ += 1,
                _ => {}
            }
        }
        let recall = tp as f64 / (tp + fn_) as f64;
        let precision = tp as f64 / (tp + fp) as f64;
        assert!(recall > 0.95, "recall {recall:.3} ({fn_} ground points missed)");
        assert!(precision > 0.99, "precision {precision:.3} ({fp} object points taken for ground)");
        // No roof point and no outlier is ground.
        for (p, f) in pts.iter().zip(&found) {
            if *f {
                assert!(p[2] > -1.0 && p[2] < 6.5, "{p:?} taken for ground");
            }
        }
    }

    #[test]
    fn spikes_go_and_slopes_stay() {
        let mut pts = Vec::new();
        for i in 0..20 {
            for j in 0..20 {
                // A 1:2 embankment along y.
                pts.push([i as f64 * 0.5 + 0.25, j as f64 * 0.5 + 0.25, (j as f64 * 0.25).min(2.0)]);
            }
        }
        pts[105][2] += 0.8;
        let keep = without_spikes(&pts, 0.5, SPIKE);
        assert!(!keep[105]);
        assert_eq!(keep.iter().filter(|k| !**k).count(), 1);
    }

    #[test]
    fn flat_ground_is_all_ground() {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                pts.push([i as f64 * 0.5, j as f64 * 0.5, 2.0 + 0.002 * i as f64]);
            }
        }
        assert!(ground(&pts, &Smrf::default()).unwrap().iter().all(|g| *g));
    }
}
