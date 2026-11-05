//! Storm on the ground (#63): rain falls on a height grid, runs over the
//! surface, gathers in ponds and spills over their rims — the waterlogging
//! stress test. The static tools (water drop, catchments, filled ponds in
//! `civiel::hydrologie`) say *where* water goes; this says *how much*, *how
//! deep* and *when*.
//!
//! Scheme: the local-inertial formulation of the shallow water equations of
//! Bates, Horritt & Fewtrell (2010), "A simple inertial formulation of the
//! shallow water equations for efficient two-dimensional flood inundation
//! modelling", Journal of Hydrology 387(1–2), 33–45 — the LISFLOOD-FP
//! floodplain solver — with the θ weighting of de Almeida, Bates, Freer &
//! Souvignet (2012), "Improving the stability of a simple formulation of the
//! shallow water equations for 2-D flood modeling", Water Resources Research
//! 48, W05528. Per face between 4-neighbours the unit discharge q (m²/s) is
//! updated explicitly with the water surface slope and a semi-implicit
//! Manning friction term:
//!
//! ```text
//! q⁺ = (θ·q + (1−θ)/2·(q₋ + q₊) − g·h_f·Δt·∂η/∂x) / (1 + g·Δt·n²·|q| / h_f^(7/3))
//! ```
//!
//! with η = z + h, h_f = max(η₁, η₂) − max(z₁, z₂) the depth through which
//! water can flow and q₋, q₊ the faces in line on either side. θ = 1 is the
//! scheme of Bates et al.; θ < 1 adds a little diffusion that damps the
//! checkerboard oscillations of deep, low-friction water (de Almeida et al.
//! 2012). The default θ = 0.8 is chosen by test: the closed bowl below
//! sloshes with θ = 1 and α = 0.7.
//!
//! Why this scheme: it gives depths and arrival times, not only the end
//! state of "fill and spill" routing, and its stable time step scales with
//! Δx, not Δx² as for the diffusive wave. The time step is adaptive,
//! Δt = α·Δx / (√(g·h_f,max) + |u|_max); Bates et al. (2010) use √(g·h_max)
//! with α of about 0.2–0.7; the flow speed is added here for steep terrain.
//!
//! Robustness and mass: after the fluxes are computed, each cell's outgoing
//! fluxes are scaled down together when they would take more water than the
//! cell holds. Every face moves the same volume out of one cell and into the
//! other, so mass is conserved to round-off and depths never go negative,
//! also when the CFL condition is momentarily violated.
//!
//! Per step: fluxes → depths, then sinks (gullies, sewer inlets: an outflow
//! capacity in m³/s, limited by the water present), rain and infiltration (a
//! constant capacity, limited likewise).
//!
//! Boundary: closed (a wall), or free outflow, where beyond the edge lies an
//! always-dry ghost cell at the ground level of the edge cell, so water falls
//! off the edge down its own depth gradient (h/Δx). Cells without a height
//! (NaN) are outside the model: the boundary condition applies at their faces
//! too, as in `civiel::hydrologie` where they are where water leaves.
//!
//! Defaults: Manning n = 0.030 s/m^(1/3) (Chow 1959, *Open-Channel
//! Hydraulics*, table 5-6: floodplain, pasture without brush, short grass,
//! normal value); α = 0.7 (Bates et al. 2010); θ = 0.8 (see above); flow
//! depth threshold 1 mm and pond depth 0.05 m (model choices, not norms).
//!
//! Cost: the faces are a flat list computed in parallel (std threads, the
//! result does not depend on their number); 500×500 cells take some 15 ms a
//! step on a 4-core laptop (Ryzen 7 3700U), 90 s for 1.5 h of storm on 2 m
//! cells (the ignored test `perf_500x500`).
//!
//! Limitations: no momentum advection (local inertial), no Horton decay or
//! saturation of the infiltration capacity, sinks never surcharge (no water
//! back from the sewer), ponds are reported on the final depths.

use serde::{Deserialize, Serialize};

use crate::P2;

const G: f64 = 9.81;

/// A regular height grid. Row 0 is the bottom (southern) row; cell
/// `i = row·nx + col` has its lower-left corner at `origin + (col, row)·cell`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    /// Lower-left corner of cell 0 (m).
    pub origin: P2,
    /// Cell size (m).
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    /// Ground levels (m), `nx·ny` values; NaN = no data / outside the model.
    pub z: Vec<f64>,
}

impl Grid {
    /// Check the dimensions.
    pub fn check(&self) -> Result<(), String> {
        if !(self.cell > 0.0 && self.cell.is_finite()) {
            return Err(format!("grid: cell size {} must be positive", self.cell));
        }
        if self.nx == 0 || self.ny == 0 {
            return Err("grid: no cells".into());
        }
        if self.z.len() != self.nx * self.ny {
            return Err(format!("grid: {} levels for {}×{} cells", self.z.len(), self.nx, self.ny));
        }
        Ok(())
    }

    /// The centre of cell `i`.
    pub fn centre(&self, i: usize) -> P2 {
        let (c, r) = (i % self.nx, i / self.nx);
        [self.origin[0] + (c as f64 + 0.5) * self.cell, self.origin[1] + (r as f64 + 0.5) * self.cell]
    }

    /// The cell containing `p`, if inside the grid.
    pub fn cell_at(&self, p: P2) -> Option<usize> {
        let c = ((p[0] - self.origin[0]) / self.cell).floor();
        let r = ((p[1] - self.origin[1]) / self.cell).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.nx && (r as usize) < self.ny).then(|| r as usize * self.nx + c as usize)
    }

    /// Cell area (m²).
    pub fn area(&self) -> f64 {
        self.cell * self.cell
    }
}

/// A value for every cell, or one for all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Field {
    Uniform(f64),
    PerCell(Vec<f64>),
}

impl Field {
    fn expand(&self, n: usize, what: &str) -> Result<Vec<f64>, String> {
        let v = match self {
            Field::Uniform(v) => vec![*v; n],
            Field::PerCell(v) if v.len() == n => v.clone(),
            Field::PerCell(v) => return Err(format!("{what}: {} values for {n} cells", v.len())),
        };
        if v.iter().any(|x| !(x.is_finite() && *x >= 0.0)) {
            return Err(format!("{what}: values must be finite and ≥ 0"));
        }
        Ok(v)
    }
}

/// Where water leaves the model: a gully or sewer inlet at a point, taking up
/// to `capacity` m³/s from the cell it lies in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sink {
    pub at: P2,
    pub capacity: f64,
}

/// The grid edge (and the faces next to cells without a height).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Boundary {
    /// A wall: nothing leaves.
    Closed,
    /// Free outflow into a dry ghost cell at the edge cell's ground level.
    Free,
}

/// Simulation settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Manning's n (s/m^(1/3)).
    pub manning: Field,
    /// Infiltration capacity (mm/h).
    pub infiltration: Field,
    pub sinks: Vec<Sink>,
    pub boundary: Boundary,
    /// Simulated time (s); 0 = the duration of the rain.
    pub duration: f64,
    /// Time step coefficient α (Bates et al. 2010: 0.2–0.7).
    pub alpha: f64,
    /// Weight θ of a face's own old discharge against its neighbours in line
    /// (de Almeida et al. 2012); 1 = Bates et al. 2010.
    pub theta: f64,
    /// Largest time step (s), for dry spells.
    pub max_dt: f64,
    /// No flow through a face with less water than this (m).
    pub flow_depth: f64,
    /// A pond is wetter than this (m).
    pub pond_depth: f64,
    /// Hydrograph interval (s); 0 = none.
    pub report: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            manning: Field::Uniform(0.030),
            infiltration: Field::Uniform(0.0),
            sinks: Vec::new(),
            boundary: Boundary::Closed,
            duration: 0.0,
            alpha: 0.7,
            theta: 0.8,
            max_dt: 10.0,
            flow_depth: 0.001,
            pond_depth: 0.05,
            report: 60.0,
        }
    }
}

/// Where a pond would overflow: the lowest point on its rim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Overflow {
    pub cell: usize,
    pub at: P2,
    /// Ground level of the spill point (m).
    pub level: f64,
    /// Spill level − water level (m); ≤ 0 means it is overflowing.
    pub freeboard: f64,
}

/// A connected wet area (4-neighbours) deeper than the pond depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pond {
    pub cells: Vec<usize>,
    /// Area (m²) and water volume (m³) of its cells.
    pub area: f64,
    pub volume: f64,
    pub max_depth: f64,
    pub deepest: usize,
    /// Highest water surface (m).
    pub level: f64,
    /// None when the water cannot leave (a closed model filled up).
    pub overflow: Option<Overflow>,
}

/// Where the rain went (m³): `rain = storage + infiltration + boundary +
/// sinks + error`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterBalance {
    pub rain: f64,
    pub storage: f64,
    pub infiltration: f64,
    pub boundary: f64,
    pub sinks: f64,
    pub error: f64,
    /// |error| / rain.
    pub relative_error: f64,
}

/// One hydrograph sample: means over the interval ending at `t` (m³/s) and
/// the storage at `t` (m³).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: f64,
    pub rain: f64,
    pub boundary: f64,
    pub sinks: f64,
    pub storage: f64,
}

/// The outcome of a storm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flood {
    /// Per cell (m, s); 0 for cells without a height.
    pub max_depth: Vec<f64>,
    pub time_of_max: Vec<f64>,
    pub final_depth: Vec<f64>,
    /// Ponds on the final depths, largest volume first.
    pub ponds: Vec<Pond>,
    /// Volume (m³) that left over the boundary, per cell it left from.
    pub edge_outflow: Vec<(usize, f64)>,
    /// Volume (m³) per sink, in the order given.
    pub sink_outflow: Vec<f64>,
    pub balance: WaterBalance,
    pub hydrograph: Vec<Sample>,
    pub steps: usize,
    pub duration: f64,
}

const NONE: u32 = u32::MAX;

/// The constants of one time step.
struct Face {
    /// g·Δt and g·Δt/Δx.
    gdt: f64,
    gdt_dx: f64,
    thr: f64,
    theta: f64,
}

/// The faces that can carry water: cells `a` (−x / −y side) and `b` (+x /
/// +y side), NONE for a ghost beyond a free boundary; positive q runs a → b.
/// `prev` and `next` are the faces in line on either side (the face itself
/// where there is none), for the θ weighting.
struct Faces {
    a: Vec<u32>,
    b: Vec<u32>,
    n2: Vec<f64>,
    prev: Vec<u32>,
    next: Vec<u32>,
}

impl Face {
    /// The new unit discharge q (m²/s) on a face from cell a to cell b (de
    /// Almeida et al. 2012, eq. with `qw` the θ-weighted old discharge and
    /// `q` the face's own for the friction), its flow depth h_f (0: no flow)
    /// and the flow speed |q|/h_f.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn flux(&self, q: f64, qw: f64, za: f64, ha: f64, zb: f64, hb: f64, n2: f64) -> (f64, f64, f64) {
        if ha <= self.thr && hb <= self.thr {
            return (0.0, 0.0, 0.0); // h_f ≤ max(ha, hb)
        }
        let (ea, eb) = (za + ha, zb + hb);
        let hf = ea.max(eb) - za.max(zb);
        if hf <= self.thr {
            return (0.0, 0.0, 0.0);
        }
        let r = inv_cbrt(hf);
        let r3 = r * r * r;
        let push = qw - self.gdt_dx * hf * (eb - ea);
        let qn = if q == 0.0 { push } else { push / (1.0 + self.gdt * n2 * q.abs() * r3 * r3 * r) };
        (qn, hf, qn.abs() * r3)
    }

    /// New q on the faces from `start` on (as many as `q` holds) from the
    /// old `q0`; the largest flow depth and flow speed.
    fn run(&self, f: &Faces, start: usize, z: &[f64], h: &[f64], q0: &[f64], q: &mut [f64]) -> (f64, f64) {
        let (mut hf_max, mut u_max) = (0.0f64, 0.0f64);
        let w = 0.5 * (1.0 - self.theta);
        for (j, qk) in q.iter_mut().enumerate() {
            let k = start + j;
            let (a, b) = (f.a[k], f.b[k]);
            let qs = q0[k];
            let qw = self.theta * qs + w * (q0[f.prev[k] as usize] + q0[f.next[k] as usize]);
            let (qn, hf, u) = if a != NONE && b != NONE {
                let (a, b) = (a as usize, b as usize);
                self.flux(qs, qw, z[a], h[a], z[b], h[b], f.n2[k])
            } else if b == NONE {
                let a = a as usize;
                let (qn, hf, u) = self.flux(qs, qw, z[a], h[a], z[a], 0.0, f.n2[k]);
                (qn.max(0.0), hf, u)
            } else {
                let b = b as usize;
                let (qn, hf, u) = self.flux(qs, qw, z[b], 0.0, z[b], h[b], f.n2[k]);
                (qn.min(0.0), hf, u)
            };
            *qk = qn;
            hf_max = hf_max.max(hf);
            u_max = u_max.max(u);
        }
        (hf_max, u_max)
    }
}

/// x^(−1/3) for x > 0 without a division or libm call (the friction term
/// needs h_f^(−7/3) on every face, every step): a bit-level first guess,
/// then three Newton steps y ← y·(4 − x·y³)/3 (relative error < 1e-9).
#[inline(always)]
fn inv_cbrt(x: f64) -> f64 {
    let mut y = f64::from_bits(0x553E_F0FF_289D_D796 - x.to_bits() / 3);
    for _ in 0..3 {
        y *= (4.0 - x * y * y * y) * (1.0 / 3.0);
    }
    y
}

/// Total duration (s) and the rain intensity (m/s) at time `t`.
fn rain_at(rain: &[(f64, f64)], t: f64) -> (f64, f64) {
    let mut start = 0.0;
    for &(d, i) in rain {
        if t < start + d {
            return (start + d, i / 3.6e6);
        }
        start += d;
    }
    (f64::INFINITY, 0.0)
}

/// Simulate `rain` — blocks of (duration s, intensity mm/h) — on `grid`.
pub fn simulate(grid: &Grid, rain: &[(f64, f64)], s: &Settings) -> Result<Flood, String> {
    grid.check()?;
    if rain.iter().any(|(d, i)| !(d.is_finite() && *d >= 0.0 && i.is_finite() && *i >= 0.0)) {
        return Err("rain: durations and intensities must be finite and ≥ 0".into());
    }
    if !(s.alpha > 0.0 && s.alpha <= 1.0) {
        return Err(format!("alpha {} must be in (0, 1]", s.alpha));
    }
    if !(s.theta >= 0.5 && s.theta <= 1.0) {
        return Err(format!("theta {} must be in [0.5, 1]", s.theta));
    }
    let ok = s.max_dt > 0.0 && s.flow_depth > 0.0 && s.pond_depth >= 0.0 && s.duration >= 0.0 && s.report >= 0.0;
    if !ok {
        return Err("settings: max_dt and flow_depth must be > 0, duration, report and pond_depth ≥ 0".into());
    }
    let (nx, ny, n) = (grid.nx, grid.ny, grid.nx * grid.ny);
    let (dx, area) = (grid.cell, grid.area());
    let z = &grid.z;
    let valid: Vec<bool> = z.iter().map(|v| v.is_finite()).collect();
    let n_valid = valid.iter().filter(|v| **v).count();
    if n_valid == 0 {
        return Err("grid: no cell has a height".into());
    }
    let manning = s.manning.expand(n, "manning")?;
    let n2: Vec<f64> = manning.iter().map(|v| v * v).collect();
    let infil: Vec<f64> = s.infiltration.expand(n, "infiltration")?.iter().map(|v| v / 3.6e6).collect();
    let mut sinks = Vec::with_capacity(s.sinks.len());
    for (k, sk) in s.sinks.iter().enumerate() {
        let c = grid.cell_at(sk.at).filter(|c| valid[*c]).ok_or_else(|| format!("sink {}: not on a cell with a height", k + 1))?;
        if !(sk.capacity.is_finite() && sk.capacity >= 0.0) {
            return Err(format!("sink {}: capacity must be ≥ 0", k + 1));
        }
        sinks.push((c, sk.capacity));
    }
    let rain_total: f64 = rain.iter().map(|r| r.0).sum();
    let t_end = if s.duration > 0.0 { s.duration } else { rain_total };
    let free = s.boundary == Boundary::Free;

    let mut f = Faces { a: Vec::new(), b: Vec::new(), n2: Vec::new(), prev: Vec::new(), next: Vec::new() };
    // Compact index of every structured face: x faces r·(nx+1) + c between
    // cells (c−1, r) and (c, r), then y faces r·nx + c between (c, r−1) and (c, r).
    let nfx = (nx + 1) * ny;
    let mut slot = vec![NONE; nfx + nx * (ny + 1)];
    {
        let mut add = |id: usize, a: Option<usize>, b: Option<usize>| {
            let a = a.filter(|i| valid[*i]);
            let b = b.filter(|i| valid[*i]);
            let (ka, kb, n2f) = match (a, b) {
                (Some(a), Some(b)) => (a as u32, b as u32, 0.5 * (n2[a] + n2[b])),
                (Some(a), None) if free => (a as u32, NONE, n2[a]),
                (None, Some(b)) if free => (NONE, b as u32, n2[b]),
                _ => return,
            };
            slot[id] = f.a.len() as u32;
            f.a.push(ka);
            f.b.push(kb);
            f.n2.push(n2f);
        };
        for row in 0..ny {
            for col in 0..=nx {
                add(row * (nx + 1) + col, (col > 0).then(|| row * nx + col - 1), (col < nx).then(|| row * nx + col));
            }
        }
        for row in 0..=ny {
            for col in 0..nx {
                add(nfx + row * nx + col, (row > 0).then(|| (row - 1) * nx + col), (row < ny).then(|| row * nx + col));
            }
        }
    }
    let nf = f.a.len();
    f.prev = vec![0; nf];
    f.next = vec![0; nf];
    for (id, &k) in slot.iter().enumerate() {
        if k == NONE {
            continue;
        }
        let (p, x) = if id < nfx {
            let col = id % (nx + 1);
            ((col > 0).then(|| id - 1), (col < nx).then(|| id + 1))
        } else {
            let row = (id - nfx) / nx;
            ((row > 0).then(|| id - nx), (row < ny).then(|| id + nx))
        };
        let pick = |o: Option<usize>| o.map(|o| slot[o]).filter(|v| *v != NONE).unwrap_or(k);
        f.prev[k as usize] = pick(p);
        f.next[k as usize] = pick(x);
    }
    drop(slot);
    let threads = if nf < 40_000 { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).min(8) };
    let mut q = vec![0.0f64; nf];
    let mut q0 = vec![0.0f64; nf];
    let mut h = vec![0.0f64; n];
    let mut out = vec![0.0f64; n];
    let mut max_depth = vec![0.0f64; n];
    let mut time_of_max = vec![0.0f64; n];
    let mut edge = vec![0.0f64; n];
    let mut sink_out = vec![0.0f64; sinks.len()];
    let (mut v_rain, mut v_inf, mut v_edge, mut v_sink) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut hydrograph = Vec::new();
    let (mut rep_rain, mut rep_edge, mut rep_sink, mut rep_t0) = (0.0, 0.0, 0.0, 0.0);
    let mut next_report = if s.report > 0.0 { s.report.min(t_end) } else { f64::INFINITY };
    let thr = s.flow_depth;
    // Largest flow depth and flow speed of the last step, for the time step.
    let (mut hf_max, mut u_max) = (0.0f64, 0.0f64);
    let mut t = 0.0f64;
    let mut steps = 0usize;
    let mut storage = 0.0f64;

    while t < t_end - 1e-9 {
        let (rain_change, r) = rain_at(rain, t);
        let speed = (G * hf_max).sqrt() + u_max;
        let mut dt = if speed > 0.0 { s.alpha * dx / speed } else { s.max_dt };
        dt = dt.min(s.max_dt).min(t_end - t).min(rain_change - t).min(next_report - t).max(1e-9);
        let step = Face { gdt: G * dt, gdt_dx: G * dt / dx, thr, theta: s.theta };
        std::mem::swap(&mut q, &mut q0);

        // 1. Fluxes (in parallel: every face on its own, so the result does
        // not depend on the number of threads), and what each cell would give.
        (hf_max, u_max) = if threads > 1 {
            let chunk = nf.div_ceil(threads);
            std::thread::scope(|sc| {
                let jobs: Vec<_> = q
                    .chunks_mut(chunk)
                    .enumerate()
                    .map(|(j, qc)| {
                        let (f, z, h, q0, step) = (&f, &z[..], &h[..], &q0[..], &step);
                        sc.spawn(move || step.run(f, j * chunk, z, h, q0, qc))
                    })
                    .collect();
                jobs.into_iter().map(|j| j.join().expect("flux thread")).fold((0.0f64, 0.0f64), |m, v| (m.0.max(v.0), m.1.max(v.1)))
            })
        } else {
            step.run(&f, 0, z, &h, &q0, &mut q)
        };
        out.fill(0.0);
        for k in 0..nf {
            let qk = q[k];
            if qk > 0.0 {
                out[f.a[k] as usize] += qk;
            } else if qk < 0.0 {
                out[f.b[k] as usize] -= qk;
            }
        }

        // 2. Limiter: a cell gives at most what it holds; `out` becomes the
        // factor on its outgoing faces.
        let give = dt * dx / area;
        for (o, hi) in out.iter_mut().zip(&h) {
            let d = *o * give;
            *o = if d > *hi { hi / d } else { 1.0 };
        }

        // 3. Move the water.
        let mut step_edge = 0.0;
        for (k, qr) in q.iter_mut().enumerate() {
            if *qr == 0.0 {
                continue;
            }
            let (a, b) = (f.a[k], f.b[k]);
            let qk = *qr * out[if *qr > 0.0 { a } else { b } as usize];
            *qr = qk;
            let d = qk * give;
            if a != NONE {
                h[a as usize] -= d;
            } else {
                edge[b as usize] -= d * area;
                step_edge -= d * area;
            }
            if b != NONE {
                h[b as usize] += d;
            } else {
                edge[a as usize] += d * area;
                step_edge += d * area;
            }
        }

        // 4. Sinks.
        let mut step_sink = 0.0;
        for (k, &(c, cap)) in sinks.iter().enumerate() {
            let v = (cap * dt).min(h[c].max(0.0) * area);
            h[c] -= v / area;
            sink_out[k] += v;
            step_sink += v;
        }

        // 5. Rain, infiltration, maxima and storage.
        let t1 = t + dt;
        let rd = r * dt;
        let (mut step_inf, mut sum) = (0.0, 0.0);
        for i in 0..n {
            if !valid[i] {
                continue;
            }
            let mut hi = h[i].max(0.0) + rd;
            let loss = (infil[i] * dt).min(hi);
            hi -= loss;
            step_inf += loss;
            h[i] = hi;
            sum += hi;
            if hi > max_depth[i] {
                max_depth[i] = hi;
                time_of_max[i] = t1;
            }
        }
        storage = sum * area;
        let step_rain = rd * area * n_valid as f64;
        step_inf *= area;

        v_rain += step_rain;
        v_inf += step_inf;
        v_edge += step_edge;
        v_sink += step_sink;
        rep_rain += step_rain;
        rep_edge += step_edge;
        rep_sink += step_sink;
        t = t1;
        steps += 1;
        if t >= next_report - 1e-9 {
            let span = (t - rep_t0).max(1e-12);
            hydrograph.push(Sample { t, rain: rep_rain / span, boundary: rep_edge / span, sinks: rep_sink / span, storage });
            (rep_rain, rep_edge, rep_sink, rep_t0) = (0.0, 0.0, 0.0, t);
            next_report = if t < t_end - 1e-9 { (t + s.report).min(t_end) } else { f64::INFINITY };
        }
    }
    let error = v_rain - (storage + v_inf + v_edge + v_sink);
    let balance = WaterBalance {
        rain: v_rain,
        storage,
        infiltration: v_inf,
        boundary: v_edge,
        sinks: v_sink,
        error,
        relative_error: if v_rain > 0.0 { error.abs() / v_rain } else { 0.0 },
    };
    let ponds = ponds(grid, &h, s.pond_depth, s.boundary);
    let edge_outflow = edge.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| (i, *v)).collect();
    Ok(Flood { max_depth, time_of_max, final_depth: h, ponds, edge_outflow, sink_outflow: sink_out, balance, hydrograph, steps, duration: t })
}

/// The 4-neighbours of cell `i` that exist in the grid.
fn neighbours(nx: usize, ny: usize, i: usize) -> impl Iterator<Item = usize> {
    let (c, r) = (i % nx, i / nx);
    [(c > 0).then(|| i - 1), (c + 1 < nx).then(|| i + 1), (r > 0).then(|| i - nx), (r + 1 < ny).then(|| i + nx)].into_iter().flatten()
}

/// Whether water can leave the model from cell `i` (free boundary only).
fn is_exit(grid: &Grid, i: usize) -> bool {
    let (c, r) = (i % grid.nx, i / grid.nx);
    c == 0 || r == 0 || c + 1 == grid.nx || r + 1 == grid.ny || neighbours(grid.nx, grid.ny, i).any(|j| !grid.z[j].is_finite())
}

/// Ponds in a depth field: connected areas (4-neighbours) of cells at least
/// `min_depth` deep, largest volume first, each with its overflow point —
/// found by a Priority-Flood from the pond outward (Barnes, Lehman & Mulla
/// 2014) that stops where the ground falls below the highest level met so
/// far (past the saddle) or at an exit of a free boundary.
pub fn ponds(grid: &Grid, depth: &[f64], min_depth: f64, boundary: Boundary) -> Vec<Pond> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let (nx, ny, n) = (grid.nx, grid.ny, grid.nx * grid.ny);
    let area = grid.area();
    let z = &grid.z;
    let wet = |i: usize| z[i].is_finite() && depth[i] >= min_depth && depth[i] > 0.0;
    let mut label = vec![usize::MAX; n];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    // Visit marks for the overflow search, by pond number.
    let mut seen = vec![usize::MAX; n];
    for start in 0..n {
        if label[start] != usize::MAX || !wet(start) {
            continue;
        }
        let id = out.len();
        let mut cells = Vec::new();
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            cells.push(i);
            for j in neighbours(nx, ny, i) {
                if label[j] == usize::MAX && wet(j) {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        let volume = cells.iter().map(|&i| depth[i]).sum::<f64>() * area;
        let deepest = *cells.iter().max_by(|a, b| depth[**a].total_cmp(&depth[**b])).unwrap();
        let level = cells.iter().map(|&i| z[i] + depth[i]).fold(f64::NEG_INFINITY, f64::max);

        // Overflow: Priority-Flood on the ground outward from the pond.
        let key = |v: f64| Reverse(ordered(v));
        let mut heap = BinaryHeap::new();
        for &i in &cells {
            seen[i] = id;
        }
        let mut spill: Option<(usize, f64)> = None;
        if boundary == Boundary::Free {
            if let Some(&i) = cells.iter().filter(|&&i| is_exit(grid, i)).min_by(|a, b| z[**a].total_cmp(&z[**b])) {
                spill = Some((i, z[i]));
            }
        }
        if spill.is_none() {
            for &i in &cells {
                for j in neighbours(nx, ny, i) {
                    if seen[j] != id && z[j].is_finite() {
                        seen[j] = id;
                        heap.push((key(z[j]), j));
                    }
                }
            }
            let mut high: Option<(usize, f64)> = None;
            while let Some((_, c)) = heap.pop() {
                let zc = z[c];
                match high {
                    Some((_, hz)) if zc < hz - 1e-9 => {
                        spill = high;
                        break;
                    }
                    // On a tie the later cell wins: it lies further along
                    // the way out.
                    _ => high = Some((c, zc)),
                }
                if boundary == Boundary::Free && is_exit(grid, c) {
                    spill = high;
                    break;
                }
                for j in neighbours(nx, ny, c) {
                    if seen[j] != id && z[j].is_finite() {
                        seen[j] = id;
                        heap.push((key(z[j]), j));
                    }
                }
            }
        }
        let overflow = spill.map(|(c, l)| Overflow { cell: c, at: grid.centre(c), level: l, freeboard: l - level });
        out.push(Pond { area: cells.len() as f64 * area, volume, max_depth: depth[deepest], deepest, level, overflow, cells });
    }
    out.sort_by(|a, b| b.volume.total_cmp(&a.volume));
    out
}

/// Order-preserving key of a float.
fn ordered(v: f64) -> u64 {
    let b = v.to_bits();
    if v >= 0.0 {
        b ^ (1 << 63)
    } else {
        !b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of `m×m` cells of `cell` m centred on the origin with z = f(x, y).
    fn grid(m: usize, cell: f64, f: impl Fn(f64, f64) -> f64) -> Grid {
        let half = m as f64 * cell / 2.0;
        let mut g = Grid { origin: [-half, -half], cell, nx: m, ny: m, z: Vec::new() };
        g.z = (0..m * m).map(|i| { let c = g.centre(i); f(c[0], c[1]) }).collect();
        g
    }

    #[test]
    fn inverse_cube_root() {
        for x in [1e-4, 1e-3, 0.0123, 0.5, 1.0, 7.3, 1e3, 1e6] {
            let y = inv_cbrt(x);
            assert!((y * y * y * x - 1.0).abs() < 3e-9, "{x}: {y}");
        }
    }

    #[test]
    fn closed_bowl_keeps_all_rain() {
        let g = grid(20, 2.0, |x, y| 0.002 * (x * x + y * y));
        let s = Settings { duration: 3600.0, ..Settings::default() };
        let f = simulate(&g, &[(1800.0, 60.0)], &s).unwrap();
        let rain = 0.030 * 1600.0;
        assert!((f.balance.rain - rain).abs() < 1e-9 * rain);
        assert!((f.balance.storage - rain).abs() < 1e-3 * rain, "{:?}", f.balance);
        assert!(f.balance.relative_error < 1e-3);
        assert_eq!(f.balance.boundary, 0.0);
        // The water gathers in the middle and stands level.
        let p = &f.ponds[0];
        assert!(p.volume > 0.8 * rain, "{p:?}");
        assert!(p.cells.contains(&(10 * 20 + 10)));
        let levels: Vec<f64> = p.cells.iter().map(|&i| g.z[i] + f.final_depth[i]).collect();
        let (lo, hi) = levels.iter().fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo < 0.01, "surface {lo}..{hi}");
        assert!(p.overflow.is_none());
        assert!(f.time_of_max[10 * 20 + 10] > 1700.0);
    }

    #[test]
    fn tilted_bowl_spills_over_its_low_rim() {
        // Minimum at (−5, 0); the edge is lowest at the middle of the west side.
        let g = grid(20, 2.0, |x, y| 0.002 * (x * x + y * y) + 0.02 * x);
        let s = Settings { duration: 5400.0, boundary: Boundary::Free, ..Settings::default() };
        let f = simulate(&g, &[(3600.0, 200.0)], &s).unwrap();
        assert!(f.balance.relative_error < 1e-3, "{:?}", f.balance);
        assert!(f.balance.boundary > 50.0, "{:?}", f.balance);
        let west: f64 = f.edge_outflow.iter().filter(|(i, _)| { let c = g.centre(*i); c[0] < -18.0 && c[1].abs() < 10.0 }).map(|e| e.1).sum();
        assert!(west > 0.8 * f.balance.boundary, "west {west} of {}", f.balance.boundary);
        let top = f.edge_outflow.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let c = g.centre(top.0);
        assert!(c[0] < -18.0 && c[1].abs() < 3.0, "most outflow at {c:?}");
        // The remaining pond reaches its rim there.
        let p = &f.ponds[0];
        let o = p.overflow.as_ref().unwrap();
        assert!(o.at[0] < -18.0 && o.at[1].abs() < 3.0, "{o:?}");
        assert!(o.freeboard < 0.02, "{o:?}");
    }

    #[test]
    fn flat_plane_drains_over_a_free_edge() {
        let g = grid(20, 1.0, |_, _| 1.0);
        let s = Settings { duration: 4.0 * 3600.0, boundary: Boundary::Free, ..Settings::default() };
        let f = simulate(&g, &[(600.0, 120.0)], &s).unwrap();
        assert!(f.balance.relative_error < 1e-3, "{:?}", f.balance);
        assert!(f.balance.boundary > 0.85 * f.balance.rain, "{:?}", f.balance);
        assert!(f.final_depth.iter().all(|h| *h < 0.003));
        assert!(f.ponds.is_empty());
        let last = f.hydrograph.last().unwrap();
        assert!((last.t - 4.0 * 3600.0).abs() < 1e-6 && (last.storage - f.balance.storage).abs() < 1e-9);
    }

    #[test]
    fn infiltration_and_sinks_take_their_share() {
        let g = grid(30, 1.0, |x, y| 0.002 * (x * x + y * y));
        let s = Settings {
            duration: 3600.0,
            infiltration: Field::Uniform(10.0),
            sinks: vec![Sink { at: [0.2, 0.2], capacity: 0.002 }],
            ..Settings::default()
        };
        let f = simulate(&g, &[(1800.0, 50.0)], &s).unwrap();
        let b = &f.balance;
        assert!(b.relative_error < 1e-3, "{b:?}");
        assert!(b.infiltration > 0.0 && b.sinks > 0.0);
        assert!(b.infiltration <= 0.010 * 900.0 + 1e-6);
        assert!(b.sinks <= 0.002 * 3600.0 + 1e-9);
        assert!((f.sink_outflow[0] - b.sinks).abs() < 1e-9);
    }

    #[test]
    fn no_data_cells_are_walls_when_closed() {
        let mut g = grid(20, 1.0, |x, _| 0.01 * x);
        for r in 0..20 {
            g.z[r * 20 + 10] = f64::NAN;
        }
        let f = simulate(&g, &[(600.0, 100.0)], &Settings { duration: 1800.0, ..Settings::default() }).unwrap();
        assert!(f.balance.relative_error < 1e-3);
        assert!(f.final_depth.iter().enumerate().all(|(i, h)| g.z[i].is_finite() || *h == 0.0));
        assert!(simulate(&g, &[(600.0, -1.0)], &Settings::default()).is_err());
    }

    /// 500×500 cells of 2 m, undulating terrain with depressions, an hour of
    /// 40 mm/h and half an hour after:
    /// `cargo test --release -p rekenkern perf_500 -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn perf_500x500() {
        let m = 500;
        let mut g = Grid { origin: [0.0, 0.0], cell: 2.0, nx: m, ny: m, z: Vec::new() };
        g.z = (0..m * m).map(|i| { let c = g.centre(i); 0.002 * c[0] + 0.3 * (c[0] / 40.0).sin() * (c[1] / 55.0).cos() }).collect();
        let s = Settings { duration: 5400.0, boundary: Boundary::Free, ..Settings::default() };
        let t0 = std::time::Instant::now();
        let f = simulate(&g, &[(3600.0, 40.0)], &s).unwrap();
        eprintln!("500×500: {} steps in {:.1?}, {:?}, {} ponds", f.steps, t0.elapsed(), f.balance, f.ponds.len());
        assert!(f.balance.relative_error < 1e-3);
    }
}
