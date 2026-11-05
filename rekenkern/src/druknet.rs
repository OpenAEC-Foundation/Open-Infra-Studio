//! Pressure networks (issue #67): steady-state heads, pressures and flows in
//! drinking-water and pressure-main networks, and the fire-flow
//! (*bluswater*) check at a hydrant.
//!
//! What: junctions with elevation and demand, reservoirs (fixed head), pipes
//! with Hazen-Williams or Darcy-Weisbach friction plus a minor-loss
//! coefficient, and valves as pipes that can be closed. [`solve`] returns the
//! head and pressure per node and the flow, velocity and headloss per pipe;
//! [`fire_flow`] finds the largest extra draw at a hydrant that keeps the
//! hydrant pressure at a minimum.
//!
//! Why this method: the global gradient algorithm of Todini & Pilati (1988),
//! "A gradient algorithm for the analysis of pipe networks", as used by
//! EPANET (Rossman et al., EPANET 2.2 user manual, ch. "Analysis
//! algorithms"). It solves heads and flows together with Newton steps, so
//! loops need no loop definitions, and every step is one sparse symmetric
//! positive-definite solve (here: envelope Cholesky after reverse
//! Cuthill-McKee ordering).
//!
//! Formulas (SI; EPANET 2.2 manual, table "pipe headloss formulas"):
//! - Hazen-Williams: h = 10.667·C^-1.852·D^-4.871·L·Q^1.852.
//! - Darcy-Weisbach: h = 8fL/(gπ²D⁵)·Q², f from Swamee & Jain (1976)
//!   f = 0.25/[log₁₀(k/3.7D + 5.74/Re^0.9)]² or from Colebrook-White (1939)
//!   by iteration; laminar f = 64/Re below Re = 2000. Between Re 2000 and
//!   4000 f is interpolated linearly (EPANET uses a cubic; the zone is rare
//!   in distribution networks).
//! - Minor loss: h = K·v²/2g = 8K/(gπ²D⁴)·Q².
//! - Near zero flow the headloss is made linear (secant below 1e-8 m³/s) so
//!   the gradient never vanishes, as EPANET does with its small-gradient cap.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Gravitational acceleration (m/s²).
pub const G: f64 = 9.81;
/// kPa per metre water column: ρ·g/1000 with ρ = 1000 kg/m³ and g = 9.81.
pub const KPA_PER_M: f64 = 9.81;
/// Kinematic viscosity of water at 20 °C (m²/s), from standard property
/// tables; drinking water is often 10–12 °C (≈ 1.3e-6), set it per case.
pub const WATER_VISCOSITY_20C: f64 = 1.004e-6;
/// Default required fire flow: 60 m³/h (1000 l/min) for one primary
/// hydrant, Brandweer Nederland, *Handreiking Bluswatervoorziening en
/// Bereikbaarheid* (2003; the 2019 edition allows 30 m³/h in many cases —
/// regional policy decides, e.g. Veiligheidsregio Groningen 2025 prefers
/// 60 m³/h with 30 m³/h as minimum). Adjustable.
pub const FIRE_FLOW_REQUIRED: f64 = 60.0 / 3600.0;
/// Default minimum hydrant pressure while drawing the fire flow: 150 kPa
/// (1.5 bar). Regional fire-service policy (Veiligheidsregio Groningen,
/// *Beleid bluswatervoorziening en bereikbaarheid*, 2025) asks 1 bar at the
/// pump of the fire engine, which by rule of thumb needs 1.5–2 bar at the
/// pressurised source; the lower end is used. Adjustable.
pub const FIRE_MIN_PRESSURE_KPA: f64 = 150.0;

/// Flow below which the headloss curve is made linear (m³/s).
const Q_SMALL: f64 = 1e-8;
/// Initial velocity guess for the flows (m/s); EPANET starts at 1 ft/s.
const V_INIT: f64 = 0.3;

/// Pipe wall friction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Friction {
    /// Hazen-Williams with coefficient C (-).
    HazenWilliams {
        /// Hazen-Williams C, e.g. 120–150 for new pipes.
        c: f64,
    },
    /// Darcy-Weisbach with wall roughness k (mm).
    DarcyWeisbach {
        /// Equivalent sand roughness k (mm).
        k_mm: f64,
    },
}

/// How the Darcy friction factor is computed in turbulent flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrictionFactor {
    /// Explicit Swamee-Jain approximation (as EPANET).
    SwameeJain,
    /// Implicit Colebrook-White, iterated.
    Colebrook,
}

/// Open or closed link; a closed pipe is a closed valve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStatus {
    /// Carries flow.
    Open,
    /// Carries no flow.
    Closed,
}

/// A node with unknown head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    /// Unique id.
    pub id: String,
    /// Ground or pipe elevation used for the pressure (m).
    pub elevation: f64,
    /// Demand drawn at the node (m³/s); negative for an injection.
    pub demand: f64,
}

/// A node with fixed head: reservoir, tank at fixed level or source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reservoir {
    /// Unique id.
    pub id: String,
    /// Fixed hydraulic head (m).
    pub head: f64,
}

/// A pipe or valve between two nodes; positive flow runs from `from` to `to`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pipe {
    /// Unique id.
    pub id: String,
    /// Start node id.
    pub from: String,
    /// End node id.
    pub to: String,
    /// Length (m).
    pub length: f64,
    /// Internal diameter (m).
    pub diameter: f64,
    /// Wall friction.
    pub friction: Friction,
    /// Sum of minor-loss coefficients K (-).
    pub minor_loss: f64,
    /// Open or closed.
    pub status: LinkStatus,
}

/// A pressure network.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Network {
    /// Nodes with unknown head.
    pub junctions: Vec<Junction>,
    /// Nodes with fixed head.
    pub reservoirs: Vec<Reservoir>,
    /// Pipes and valves.
    pub pipes: Vec<Pipe>,
}

/// Solver settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolverOptions {
    /// Converged when Σ|ΔQ| / Σ|Q| falls below this (EPANET's "accuracy"
    /// criterion; default 1e-6, tighter than EPANET's usual 0.001 because
    /// iterations are cheap).
    pub tolerance: f64,
    /// Maximum Newton iterations (default 200, an adjustable choice).
    pub max_iterations: usize,
    /// Kinematic viscosity (m²/s) for the Reynolds number.
    pub viscosity: f64,
    /// Turbulent friction factor formula for Darcy-Weisbach pipes.
    pub friction_factor: FrictionFactor,
}

impl Default for SolverOptions {
    fn default() -> Self {
        SolverOptions {
            tolerance: 1e-6,
            max_iterations: 200,
            viscosity: WATER_VISCOSITY_20C,
            friction_factor: FrictionFactor::SwameeJain,
        }
    }
}

/// Result per node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeResult {
    /// Node id.
    pub id: String,
    /// Hydraulic head (m).
    pub head: f64,
    /// Pressure head above the elevation (m water column); 0 for reservoirs.
    pub pressure_m: f64,
    /// Pressure (kPa).
    pub pressure_kpa: f64,
    /// Demand (m³/s); for a reservoir minus its supply to the network.
    pub demand: f64,
}

/// Result per pipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipeResult {
    /// Pipe id.
    pub id: String,
    /// Flow from `from` to `to` (m³/s).
    pub flow: f64,
    /// Mean velocity (m/s), signed like the flow.
    pub velocity: f64,
    /// Headloss from `from` to `to` by the friction and minor-loss formulas (m).
    pub headloss: f64,
    /// Friction slope (m per km), absolute.
    pub headloss_per_km: f64,
    /// Darcy friction factor for Darcy-Weisbach pipes.
    pub friction_factor: Option<f64>,
}

/// Steady-state solution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Solution {
    /// Junctions in input order, then reservoirs.
    pub nodes: Vec<NodeResult>,
    /// Pipes in input order.
    pub pipes: Vec<PipeResult>,
    /// Newton iterations used.
    pub iterations: usize,
    /// Final Σ|ΔQ| / Σ|Q|.
    pub relative_flow_change: f64,
}

impl Solution {
    /// Result of node `id`.
    pub fn node(&self, id: &str) -> Option<&NodeResult> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Result of pipe `id`.
    pub fn pipe(&self, id: &str) -> Option<&PipeResult> {
        self.pipes.iter().find(|p| p.id == id)
    }
}

/// Darcy friction factor for Reynolds number `re` and relative roughness
/// `rel_rough` = k/D.
pub fn friction_factor(re: f64, rel_rough: f64, method: FrictionFactor) -> f64 {
    let turbulent = |re: f64| {
        let sj = 0.25 / (rel_rough / 3.7 + 5.74 / re.powf(0.9)).log10().powi(2);
        match method {
            FrictionFactor::SwameeJain => sj,
            FrictionFactor::Colebrook => {
                let mut x = 1.0 / sj.sqrt();
                for _ in 0..50 {
                    let next = -2.0 * (rel_rough / 3.7 + 2.51 * x / re).log10();
                    let done = (next - x).abs() < 1e-12 * x;
                    x = next;
                    if done {
                        break;
                    }
                }
                1.0 / (x * x)
            }
        }
    };
    if re <= 0.0 {
        f64::INFINITY
    } else if re < 2000.0 {
        64.0 / re
    } else if re > 4000.0 {
        turbulent(re)
    } else {
        let (f0, f1) = (64.0 / 2000.0, turbulent(4000.0));
        f0 + (f1 - f0) * (re - 2000.0) / 2000.0
    }
}

/// Headloss (m) of `pipe` at flow `q`, positive in the flow direction sign.
fn headloss(pipe: &Pipe, q: f64, opts: &SolverOptions) -> f64 {
    let d = pipe.diameter;
    let minor = 8.0 * pipe.minor_loss / (G * std::f64::consts::PI.powi(2) * d.powi(4));
    let aq = q.abs();
    if aq < Q_SMALL {
        // linear secant through the curve at Q_SMALL
        return headloss(pipe, Q_SMALL, opts) / Q_SMALL * q;
    }
    let friction = match pipe.friction {
        Friction::HazenWilliams { c } => {
            10.667 * pipe.length / (c.powf(1.852) * d.powf(4.871)) * aq.powf(1.852)
        }
        Friction::DarcyWeisbach { k_mm } => {
            let re = 4.0 * aq / (std::f64::consts::PI * d * opts.viscosity);
            let f = friction_factor(re, k_mm / 1000.0 / d, opts.friction_factor);
            8.0 * f * pipe.length / (G * std::f64::consts::PI.powi(2) * d.powi(5)) * aq * aq
        }
    };
    (friction + minor * aq * aq) * q.signum()
}

/// Headloss and its derivative dh/dQ (> 0).
fn headloss_gradient(pipe: &Pipe, q: f64, opts: &SolverOptions) -> (f64, f64) {
    let h = headloss(pipe, q, opts);
    let aq = q.abs();
    let g = if aq < Q_SMALL {
        headloss(pipe, Q_SMALL, opts) / Q_SMALL
    } else {
        match pipe.friction {
            Friction::HazenWilliams { .. } => {
                let minor = 8.0 * pipe.minor_loss
                    / (G * std::f64::consts::PI.powi(2) * pipe.diameter.powi(4));
                let friction = h.abs() - minor * aq * aq;
                1.852 * friction / aq + 2.0 * minor * aq
            }
            Friction::DarcyWeisbach { .. } => {
                let dq = (1e-6 * aq).max(Q_SMALL);
                (headloss(pipe, aq + dq, opts) - headloss(pipe, (aq - dq).max(0.0), opts))
                    / (aq + dq - (aq - dq).max(0.0))
            }
        }
    };
    (h, g.max(1e-12))
}

/// Envelope (skyline) storage of a symmetric matrix, lower triangle by rows.
struct Skyline {
    first: Vec<usize>,
    start: Vec<usize>,
    val: Vec<f64>,
}

impl Skyline {
    fn new(n: usize, edges: &[(usize, usize)]) -> Self {
        let mut first: Vec<usize> = (0..n).collect();
        for &(a, b) in edges {
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            first[hi] = first[hi].min(lo);
        }
        let mut start = Vec::with_capacity(n + 1);
        let mut len = 0;
        for (i, f) in first.iter().enumerate() {
            start.push(len);
            len += i - f + 1;
        }
        start.push(len);
        Skyline {
            first,
            start,
            val: vec![0.0; len],
        }
    }

    fn at(&mut self, i: usize, j: usize) -> &mut f64 {
        let (r, c) = if i >= j { (i, j) } else { (j, i) };
        &mut self.val[self.start[r] + c - self.first[r]]
    }

    fn get(&self, r: usize, c: usize) -> f64 {
        self.val[self.start[r] + c - self.first[r]]
    }

    /// In-place Cholesky factorisation; fails if not positive definite.
    fn factor(&mut self) -> Result<(), String> {
        let n = self.first.len();
        for i in 0..n {
            for j in self.first[i]..=i {
                let k0 = self.first[i].max(self.first[j]);
                let mut sum = self.get(i, j);
                for k in k0..j {
                    sum -= self.get(i, k) * self.get(j, k);
                }
                let v = if j < i {
                    sum / self.get(j, j)
                } else if sum > 0.0 {
                    sum.sqrt()
                } else {
                    return Err(
                        "network matrix is singular (a node without a path to a fixed head?)"
                            .into(),
                    );
                };
                *self.at(i, j) = v;
            }
        }
        Ok(())
    }

    /// Solves L·Lᵀ·x = b in place.
    fn solve(&self, b: &mut [f64]) {
        let n = b.len();
        for i in 0..n {
            let (f, row) = (self.first[i], &self.val[self.start[i]..self.start[i + 1]]);
            let s: f64 = row[..i - f].iter().zip(&b[f..i]).map(|(l, x)| l * x).sum();
            b[i] = (b[i] - s) / row[i - f];
        }
        for i in (0..n).rev() {
            let (f, row) = (self.first[i], &self.val[self.start[i]..self.start[i + 1]]);
            b[i] /= row[i - f];
            let xi = b[i];
            for (x, l) in b[f..i].iter_mut().zip(&row[..i - f]) {
                *x -= l * xi;
            }
        }
    }
}

/// Reverse Cuthill-McKee ordering: `perm[old] = new`.
fn rcm(n: usize, adj: &[Vec<usize>]) -> Vec<usize> {
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    let mut by_degree: Vec<usize> = (0..n).collect();
    by_degree.sort_by_key(|&i| adj[i].len());
    for &s in &by_degree {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        let mut queue = VecDeque::from([s]);
        while let Some(v) = queue.pop_front() {
            order.push(v);
            let mut nb: Vec<usize> = adj[v].iter().copied().filter(|&w| !seen[w]).collect();
            nb.sort_by_key(|&w| adj[w].len());
            nb.dedup();
            for w in nb {
                if !seen[w] {
                    seen[w] = true;
                    queue.push_back(w);
                }
            }
        }
    }
    let mut perm = vec![0; n];
    for (new, old) in order.into_iter().rev().enumerate() {
        perm[old] = new;
    }
    perm
}

#[derive(Clone, Copy)]
enum NodeRef {
    Free(usize),
    Fixed(usize),
}

/// Steady-state solution of `net` with the global gradient algorithm.
pub fn solve(net: &Network, opts: &SolverOptions) -> Result<Solution, String> {
    if !(opts.tolerance.is_finite()
        && opts.tolerance > 0.0
        && opts.viscosity.is_finite()
        && opts.viscosity > 0.0)
        || opts.max_iterations == 0
    {
        return Err("tolerance, max_iterations and viscosity must be > 0".into());
    }
    let nj = net.junctions.len();
    let mut index: HashMap<&str, NodeRef> = HashMap::new();
    for (i, j) in net.junctions.iter().enumerate() {
        if !(j.elevation.is_finite() && j.demand.is_finite()) {
            return Err(format!(
                "junction '{}': elevation and demand must be finite",
                j.id
            ));
        }
        if index.insert(&j.id, NodeRef::Free(i)).is_some() {
            return Err(format!("duplicate node id '{}'", j.id));
        }
    }
    for (i, r) in net.reservoirs.iter().enumerate() {
        if !r.head.is_finite() {
            return Err(format!("reservoir '{}': head must be finite", r.id));
        }
        if index.insert(&r.id, NodeRef::Fixed(i)).is_some() {
            return Err(format!("duplicate node id '{}'", r.id));
        }
    }
    let mut ends = Vec::with_capacity(net.pipes.len());
    for p in &net.pipes {
        let a = *index
            .get(p.from.as_str())
            .ok_or_else(|| format!("pipe '{}': unknown node '{}'", p.id, p.from))?;
        let b = *index
            .get(p.to.as_str())
            .ok_or_else(|| format!("pipe '{}': unknown node '{}'", p.id, p.to))?;
        if p.from == p.to {
            return Err(format!("pipe '{}' starts and ends at '{}'", p.id, p.from));
        }
        let rough_ok = match p.friction {
            Friction::HazenWilliams { c } => c > 0.0,
            Friction::DarcyWeisbach { k_mm } => k_mm >= 0.0,
        };
        if !(p.length > 0.0 && p.diameter > 0.0 && p.minor_loss >= 0.0 && rough_ok) {
            return Err(format!(
                "pipe '{}': length, diameter and roughness must be > 0, minor loss ≥ 0",
                p.id
            ));
        }
        ends.push((a, b));
    }

    // Every junction needs a path over open pipes to a fixed head.
    let mut adj = vec![Vec::new(); nj];
    let mut reached = vec![false; nj];
    let mut queue = VecDeque::new();
    let mut edges = Vec::new();
    for (p, &(a, b)) in net.pipes.iter().zip(&ends) {
        if p.status == LinkStatus::Closed {
            continue;
        }
        match (a, b) {
            (NodeRef::Free(i), NodeRef::Free(j)) => {
                adj[i].push(j);
                adj[j].push(i);
                edges.push((i, j));
            }
            (NodeRef::Free(i), NodeRef::Fixed(_)) | (NodeRef::Fixed(_), NodeRef::Free(i))
                if !reached[i] =>
            {
                reached[i] = true;
                queue.push_back(i);
            }
            _ => {}
        }
    }
    while let Some(v) = queue.pop_front() {
        for &w in &adj[v] {
            if !reached[w] {
                reached[w] = true;
                queue.push_back(w);
            }
        }
    }
    let cut: Vec<&str> = net
        .junctions
        .iter()
        .zip(&reached)
        .filter(|(_, r)| !**r)
        .map(|(j, _)| j.id.as_str())
        .collect();
    if !cut.is_empty() {
        return Err(format!(
            "no path to a reservoir over open pipes for: {}",
            cut.join(", ")
        ));
    }

    let perm = rcm(nj, &adj);
    let pedges: Vec<(usize, usize)> = edges.iter().map(|&(a, b)| (perm[a], perm[b])).collect();
    let mut mat = Skyline::new(nj, &pedges);
    let mut q: Vec<f64> = net
        .pipes
        .iter()
        .map(|p| match p.status {
            LinkStatus::Open => V_INIT * std::f64::consts::PI * p.diameter * p.diameter / 4.0,
            LinkStatus::Closed => 0.0,
        })
        .collect();
    let fixed_head = |r: NodeRef, h: &[f64]| match r {
        NodeRef::Free(i) => h[perm[i]],
        NodeRef::Fixed(k) => net.reservoirs[k].head,
    };
    let mut heads = vec![0.0; nj];
    let mut iterations = 0;
    let mut change = f64::INFINITY;
    while iterations < opts.max_iterations {
        iterations += 1;
        mat.val.iter_mut().for_each(|v| *v = 0.0);
        let mut rhs = vec![0.0; nj];
        for (j, junction) in net.junctions.iter().enumerate() {
            rhs[perm[j]] -= junction.demand;
        }
        let mut lin = Vec::with_capacity(net.pipes.len());
        for (k, p) in net.pipes.iter().enumerate() {
            if p.status == LinkStatus::Closed {
                lin.push((0.0, 0.0));
                continue;
            }
            let (h, g) = headloss_gradient(p, q[k], opts);
            let pk = 1.0 / g;
            let yq = q[k] - h / g;
            lin.push((pk, yq));
            match ends[k] {
                (NodeRef::Free(a), NodeRef::Free(b)) => {
                    let (a, b) = (perm[a], perm[b]);
                    *mat.at(a, a) += pk;
                    *mat.at(b, b) += pk;
                    *mat.at(a, b) -= pk;
                    rhs[a] -= yq;
                    rhs[b] += yq;
                }
                (NodeRef::Free(a), NodeRef::Fixed(r)) => {
                    let a = perm[a];
                    *mat.at(a, a) += pk;
                    rhs[a] += -yq + pk * net.reservoirs[r].head;
                }
                (NodeRef::Fixed(r), NodeRef::Free(b)) => {
                    let b = perm[b];
                    *mat.at(b, b) += pk;
                    rhs[b] += yq + pk * net.reservoirs[r].head;
                }
                (NodeRef::Fixed(_), NodeRef::Fixed(_)) => {}
            }
        }
        if nj > 0 {
            mat.factor()?;
            mat.solve(&mut rhs);
        }
        heads = rhs;
        let (mut dsum, mut qsum) = (0.0, 0.0);
        for (k, &(pk, yq)) in lin.iter().enumerate() {
            if net.pipes[k].status == LinkStatus::Closed {
                continue;
            }
            let (a, b) = ends[k];
            let new = yq + pk * (fixed_head(a, &heads) - fixed_head(b, &heads));
            dsum += (new - q[k]).abs();
            qsum += new.abs();
            q[k] = new;
        }
        change = if qsum > 0.0 { dsum / qsum } else { dsum };
        if change <= opts.tolerance {
            break;
        }
    }
    if change > opts.tolerance {
        return Err(format!(
            "no convergence after {iterations} iterations (relative flow change {change:.3e})"
        ));
    }

    let mut supply = vec![0.0; net.reservoirs.len()];
    let mut pipes = Vec::with_capacity(net.pipes.len());
    for (k, p) in net.pipes.iter().enumerate() {
        let flow = q[k];
        if let NodeRef::Fixed(r) = ends[k].0 {
            supply[r] += flow;
        }
        if let NodeRef::Fixed(r) = ends[k].1 {
            supply[r] -= flow;
        }
        let area = std::f64::consts::PI * p.diameter * p.diameter / 4.0;
        let h = if p.status == LinkStatus::Open {
            headloss(p, flow, opts)
        } else {
            0.0
        };
        let ff = match p.friction {
            Friction::DarcyWeisbach { k_mm } if flow != 0.0 => Some(friction_factor(
                4.0 * flow.abs() / (std::f64::consts::PI * p.diameter * opts.viscosity),
                k_mm / 1000.0 / p.diameter,
                opts.friction_factor,
            )),
            _ => None,
        };
        pipes.push(PipeResult {
            id: p.id.clone(),
            flow,
            velocity: flow / area,
            headloss: h,
            headloss_per_km: h.abs() / p.length * 1000.0,
            friction_factor: ff,
        });
    }
    let mut nodes: Vec<NodeResult> = net
        .junctions
        .iter()
        .enumerate()
        .map(|(j, n)| {
            let head = heads[perm[j]];
            let pressure_m = head - n.elevation;
            NodeResult {
                id: n.id.clone(),
                head,
                pressure_m,
                pressure_kpa: pressure_m * KPA_PER_M,
                demand: n.demand,
            }
        })
        .collect();
    nodes.extend(net.reservoirs.iter().zip(&supply).map(|(r, s)| NodeResult {
        id: r.id.clone(),
        head: r.head,
        pressure_m: 0.0,
        pressure_kpa: 0.0,
        demand: -s,
    }));
    Ok(Solution {
        nodes,
        pipes,
        iterations,
        relative_flow_change: change,
    })
}

/// Settings of the fire-flow check.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FireFlowOptions {
    /// Minimum hydrant pressure while drawing (kPa); default `FIRE_MIN_PRESSURE_KPA`.
    pub min_pressure_kpa: f64,
    /// Required fire flow (m³/s); default `FIRE_FLOW_REQUIRED`.
    pub required_flow: f64,
    /// Search no further than this extra flow (m³/s); default 0.5.
    pub max_flow: f64,
    /// Bisection stops when the bracket is this narrow (m³/s); default 1e-6.
    pub flow_tolerance: f64,
}

impl Default for FireFlowOptions {
    fn default() -> Self {
        FireFlowOptions {
            min_pressure_kpa: FIRE_MIN_PRESSURE_KPA,
            required_flow: FIRE_FLOW_REQUIRED,
            max_flow: 0.5,
            flow_tolerance: 1e-6,
        }
    }
}

/// Outcome of the fire-flow check at one hydrant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FireFlowResult {
    /// Hydrant junction id.
    pub hydrant: String,
    /// Largest extra draw that keeps the hydrant at the minimum pressure
    /// (m³/s), at most `max_flow`; 0 if even without draw it is below.
    pub available_flow: f64,
    /// Hydrant pressure at the available flow (kPa).
    pub pressure_at_available_kpa: f64,
    /// Required flow (m³/s).
    pub required_flow: f64,
    /// Hydrant pressure while drawing the required flow (kPa).
    pub pressure_at_required_kpa: f64,
    /// Lowest junction pressure anywhere while drawing the required flow:
    /// (junction id, kPa).
    pub lowest_at_required: (String, f64),
    /// Whether the available flow reaches the required flow.
    pub meets_requirement: bool,
}

/// Fire-flow check: the largest extra demand at junction `hydrant` with the
/// hydrant pressure ≥ the minimum, by bisection on the extra demand (the
/// hydrant pressure falls monotonically with the draw).
pub fn fire_flow(
    net: &Network,
    hydrant: &str,
    fopts: &FireFlowOptions,
    sopts: &SolverOptions,
) -> Result<FireFlowResult, String> {
    let idx = net
        .junctions
        .iter()
        .position(|j| j.id == hydrant)
        .ok_or_else(|| format!("hydrant '{hydrant}' is not a junction"))?;
    if !(fopts.required_flow >= 0.0 && fopts.max_flow > 0.0 && fopts.flow_tolerance > 0.0) {
        return Err("fire flows must be ≥ 0 and the tolerance > 0".into());
    }
    let mut work = net.clone();
    let base = net.junctions[idx].demand;
    let mut run = |extra: f64| -> Result<Solution, String> {
        work.junctions[idx].demand = base + extra;
        solve(&work, sopts)
    };
    let p_at = |s: &Solution| s.nodes[idx].pressure_kpa;

    let at_required = run(fopts.required_flow)?;
    let pressure_at_required_kpa = p_at(&at_required);
    let lowest_at_required = at_required.nodes[..net.junctions.len()]
        .iter()
        .min_by(|a, b| a.pressure_kpa.total_cmp(&b.pressure_kpa))
        .map(|n| (n.id.clone(), n.pressure_kpa))
        .unwrap_or_default();

    let p0 = p_at(&run(0.0)?);
    let (available_flow, pressure_at_available_kpa) = if p0 < fopts.min_pressure_kpa {
        (0.0, p0)
    } else {
        let mut lo = 0.0;
        let mut hi = fopts
            .required_flow
            .max(fopts.flow_tolerance)
            .min(fopts.max_flow);
        loop {
            if p_at(&run(hi)?) < fopts.min_pressure_kpa {
                break;
            }
            lo = hi;
            if hi >= fopts.max_flow {
                break;
            }
            hi = (2.0 * hi).min(fopts.max_flow);
        }
        if lo < hi {
            while hi - lo > fopts.flow_tolerance {
                let mid = 0.5 * (lo + hi);
                if p_at(&run(mid)?) >= fopts.min_pressure_kpa {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
        }
        (lo, p_at(&run(lo)?))
    };
    Ok(FireFlowResult {
        hydrant: hydrant.to_string(),
        available_flow,
        pressure_at_available_kpa,
        required_flow: fopts.required_flow,
        pressure_at_required_kpa,
        lowest_at_required,
        meets_requirement: available_flow + 1e-12 >= fopts.required_flow,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn junction(id: &str, elevation: f64, demand: f64) -> Junction {
        Junction {
            id: id.into(),
            elevation,
            demand,
        }
    }

    fn pipe(
        id: &str,
        from: &str,
        to: &str,
        length: f64,
        diameter: f64,
        friction: Friction,
    ) -> Pipe {
        Pipe {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            length,
            diameter,
            friction,
            minor_loss: 0.0,
            status: LinkStatus::Open,
        }
    }

    const HW: Friction = Friction::HazenWilliams { c: 120.0 };

    fn hw_r(l: f64, d: f64, c: f64) -> f64 {
        10.667 * l / (c.powf(1.852) * d.powf(4.871))
    }

    fn single(friction: Friction, minor: f64) -> Network {
        let mut p = pipe("P1", "R", "J", 1000.0, 0.1, friction);
        p.minor_loss = minor;
        Network {
            junctions: vec![junction("J", 5.0, 0.01)],
            reservoirs: vec![Reservoir {
                id: "R".into(),
                head: 50.0,
            }],
            pipes: vec![p],
        }
    }

    #[test]
    fn single_pipe_hazen_williams() {
        let s = solve(&single(HW, 2.0), &SolverOptions::default()).unwrap();
        let q: f64 = 0.01;
        let minor = 2.0 * 8.0 / (G * std::f64::consts::PI.powi(2) * 0.1f64.powi(4)) * q * q;
        let expected = 50.0 - hw_r(1000.0, 0.1, 120.0) * q.powf(1.852) - minor;
        let j = s.node("J").unwrap();
        assert!((j.head - expected).abs() < 1e-6, "{} vs {expected}", j.head);
        assert!((j.pressure_m - (expected - 5.0)).abs() < 1e-6);
        assert!((j.pressure_kpa - j.pressure_m * 9.81).abs() < 1e-9);
        let p = s.pipe("P1").unwrap();
        assert!((p.flow - q).abs() < 1e-9);
        assert!((p.velocity - q / (std::f64::consts::PI * 0.0025)).abs() < 1e-6);
        assert!((s.node("R").unwrap().demand + q).abs() < 1e-9);
    }

    #[test]
    fn single_pipe_darcy_weisbach() {
        let opts = SolverOptions::default();
        let s = solve(&single(Friction::DarcyWeisbach { k_mm: 0.1 }, 0.0), &opts).unwrap();
        let q: f64 = 0.01;
        let re = 4.0 * q / (std::f64::consts::PI * 0.1 * opts.viscosity);
        let f = 0.25 / (0.001 / 3.7 + 5.74 / re.powf(0.9)).log10().powi(2);
        let v = q / (std::f64::consts::PI * 0.0025);
        let expected = 50.0 - f * 1000.0 / 0.1 * v * v / (2.0 * G);
        assert!((s.node("J").unwrap().head - expected).abs() < 1e-6);
        assert!((s.pipe("P1").unwrap().friction_factor.unwrap() - f).abs() < 1e-12);
        // Colebrook is within 2 % of Swamee-Jain
        let fc = friction_factor(re, 0.001, FrictionFactor::Colebrook);
        assert!((fc - f).abs() / fc < 0.02);
        assert!((friction_factor(1000.0, 0.001, FrictionFactor::SwameeJain) - 0.064).abs() < 1e-12);
    }

    #[test]
    fn parallel_pipes_split_consistently() {
        let net = Network {
            junctions: vec![junction("J", 0.0, 0.03)],
            reservoirs: vec![Reservoir {
                id: "R".into(),
                head: 40.0,
            }],
            pipes: vec![
                pipe("A", "R", "J", 800.0, 0.1, HW),
                pipe("B", "J", "R", 800.0, 0.15, HW),
            ],
        };
        let s = solve(&net, &SolverOptions::default()).unwrap();
        let (qa, qb) = (s.pipe("A").unwrap().flow, -s.pipe("B").unwrap().flow);
        assert!((qa + qb - 0.03).abs() < 1e-9);
        let ratio = (hw_r(800.0, 0.15, 120.0) / hw_r(800.0, 0.1, 120.0)).powf(1.0 / 1.852);
        assert!((qa / qb - ratio).abs() < 1e-6);
        assert!((s.pipe("A").unwrap().headloss + s.pipe("B").unwrap().headloss).abs() < 1e-6);
    }

    #[test]
    fn loop_mass_balance_and_head_sum() {
        let dw = Friction::DarcyWeisbach { k_mm: 0.05 };
        let net = Network {
            junctions: vec![
                junction("A", 2.0, 0.002),
                junction("B", 3.0, 0.008),
                junction("C", 1.0, 0.005),
                junction("D", 0.0, 0.004),
            ],
            reservoirs: vec![Reservoir {
                id: "R".into(),
                head: 45.0,
            }],
            pipes: vec![
                pipe("1", "R", "A", 500.0, 0.2, HW),
                pipe("2", "A", "B", 300.0, 0.15, HW),
                pipe("3", "B", "C", 400.0, 0.1, dw.clone()),
                pipe("4", "C", "A", 350.0, 0.125, HW),
                pipe("5", "C", "D", 200.0, 0.1, dw),
                pipe("6", "D", "B", 250.0, 0.1, HW),
            ],
        };
        let s = solve(
            &net,
            &SolverOptions {
                tolerance: 1e-10,
                ..Default::default()
            },
        )
        .unwrap();
        for j in &net.junctions {
            let net_in: f64 = net
                .pipes
                .iter()
                .zip(&s.pipes)
                .map(|(p, r)| {
                    if p.to == j.id {
                        r.flow
                    } else if p.from == j.id {
                        -r.flow
                    } else {
                        0.0
                    }
                })
                .sum();
            assert!((net_in - j.demand).abs() < 1e-9, "node {}", j.id);
        }
        let h = |id: &str| s.pipe(id).unwrap().headloss;
        assert!((h("2") + h("3") + h("4")).abs() < 1e-6, "loop A-B-C");
        assert!((h("3") + h("5") + h("6")).abs() < 1e-6, "loop B-C-D");
        assert!((s.node("R").unwrap().demand + 0.019).abs() < 1e-9);
    }

    #[test]
    fn closed_valve_and_disconnected_node() {
        let mut net = single(HW, 0.0);
        net.junctions.push(junction("K", 0.0, 0.0));
        net.pipes.push(Pipe {
            status: LinkStatus::Closed,
            ..pipe("V", "J", "K", 10.0, 0.1, HW)
        });
        let err = solve(&net, &SolverOptions::default()).unwrap_err();
        assert!(err.contains('K'), "{err}");
        net.pipes[1].status = LinkStatus::Open;
        net.pipes.push(Pipe {
            status: LinkStatus::Closed,
            ..pipe("V2", "R", "K", 10.0, 0.1, HW)
        });
        let s = solve(&net, &SolverOptions::default()).unwrap();
        assert_eq!(s.pipe("V2").unwrap().flow, 0.0);
        assert!(s.pipe("V").unwrap().flow.abs() < 1e-9);
        assert!((s.node("K").unwrap().head - s.node("J").unwrap().head).abs() < 1e-6);
    }

    #[test]
    fn grid_network_converges() {
        // 20 × 20 grid, 100 m pipes, fed from two corners: 760 pipes, 361 loops.
        let n = 20;
        let id = |i: usize, j: usize| format!("n{i}_{j}");
        let mut net = Network::default();
        for i in 0..n {
            for j in 0..n {
                net.junctions.push(junction(&id(i, j), 0.0, 0.0005));
                if i + 1 < n {
                    net.pipes.push(pipe(
                        &format!("v{i}_{j}"),
                        &id(i, j),
                        &id(i + 1, j),
                        100.0,
                        0.1,
                        HW,
                    ));
                }
                if j + 1 < n {
                    net.pipes.push(pipe(
                        &format!("h{i}_{j}"),
                        &id(i, j),
                        &id(i, j + 1),
                        100.0,
                        0.1,
                        HW,
                    ));
                }
            }
        }
        net.reservoirs.push(Reservoir {
            id: "R1".into(),
            head: 40.0,
        });
        net.reservoirs.push(Reservoir {
            id: "R2".into(),
            head: 38.0,
        });
        net.pipes.push(pipe("s1", "R1", &id(0, 0), 50.0, 0.3, HW));
        net.pipes
            .push(pipe("s2", "R2", &id(n - 1, n - 1), 50.0, 0.3, HW));
        let s = solve(&net, &SolverOptions::default()).unwrap();
        let supply = -(s.node("R1").unwrap().demand + s.node("R2").unwrap().demand);
        assert!((supply - 0.0005 * (n * n) as f64).abs() < 1e-9);
        assert!(s.iterations < 30, "{} iterations", s.iterations);
        // heads fall away from the sources
        assert!(s.node(&id(0, 0)).unwrap().head > s.node(&id(9, 9)).unwrap().head);
    }

    #[test]
    fn fire_flow_reaches_minimum_pressure() {
        let net = Network {
            junctions: vec![junction("H", 0.0, 0.002)],
            reservoirs: vec![Reservoir {
                id: "R".into(),
                head: 40.0,
            }],
            pipes: vec![pipe("P", "R", "H", 500.0, 0.15, HW)],
        };
        let fo = FireFlowOptions::default();
        let r = fire_flow(&net, "H", &fo, &SolverOptions::default()).unwrap();
        let p_min_m = fo.min_pressure_kpa / KPA_PER_M;
        let exact = ((40.0 - p_min_m) / hw_r(500.0, 0.15, 120.0)).powf(1.0 / 1.852) - 0.002;
        assert!(
            (r.available_flow - exact).abs() < 2e-5,
            "{} vs {exact}",
            r.available_flow
        );
        assert!(r.pressure_at_available_kpa >= fo.min_pressure_kpa - 1e-6);
        assert!(r.pressure_at_available_kpa - fo.min_pressure_kpa < 1.0);
        assert!(r.meets_requirement);
        assert_eq!(r.lowest_at_required.0, "H");
        // A long thin main cannot deliver 60 m³/h at 1.5 bar.
        let weak = Network {
            pipes: vec![pipe("P", "R", "H", 3000.0, 0.08, HW)],
            ..net
        };
        let w = fire_flow(&weak, "H", &fo, &SolverOptions::default()).unwrap();
        assert!(!w.meets_requirement && w.available_flow < FIRE_FLOW_REQUIRED);
        assert!((w.pressure_at_available_kpa - fo.min_pressure_kpa).abs() < 1.0);
    }
}
