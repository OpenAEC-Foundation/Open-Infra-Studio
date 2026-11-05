//! Sewer hydraulics for a dendritic (tree) network of manholes and
//! free-flow pipes: design flow per pipe (rational method for storm water,
//! dry weather flow per inhabitant, infiltration), partially filled circular
//! pipes after Prandtl-Colebrook (Colebrook-White with the hydraulic radius),
//! the choice of the smallest standard diameter that carries the design flow,
//! and a steady hydraulic grade line from the outfalls upstream that flags
//! surcharged pipes and water on the street.
//!
//! Prandtl-Colebrook is used rather than Manning because Dutch sewer practice
//! and Stichting RIONED work with the wall roughness k (mm), and the main
//! crate's `CIV_RIOOL_TOETS` checks full-pipe capacity with the same formula;
//! for a part-full pipe D is replaced by 4R (RIONED, *Module Riolering voor
//! het HBO*, v2 2009, §5.4.1, eqs. 5.9–5.10 for the part-full geometry).
//!
//! The model is steady and simple on purpose: the grade line is a backwater
//! estimate, not the hydrodynamic computation of Leidraad Riolering module
//! C2100 (HBO §5.5.3) with standard storms; see `hydraulic_grade_line`.
//!
//! Main source: Stichting RIONED, *Module Riolering voor het HBO*, v2, March
//! 2009 (here "HBO"), <https://www.riool.net/media/cgtfrx5t/module-riolering-voor-het-hbo.pdf>.

use std::collections::HashMap;
use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

/// Gravity (m/s²).
pub const G: f64 = 9.81;
/// Kinematic viscosity of water at 10 °C (m²/s), as the main crate's sewer check.
pub const NU: f64 = 1.31e-6;

// ── Design values ───────────────────────────────────────────────────────────

/// Design rain intensity (l/s/ha) on the connected paved area: HBO §5.2.2
/// (90 l/s/ha is sometimes chosen in sloping areas).
pub const RAIN: f64 = 60.0;
/// Household waste water per inhabitant per day (l): HBO §5.2.2.
pub const DWA_PER_DAY: f64 = 120.0;
/// Hours over which the daily waste water is discharged: HBO §5.2.2, giving
/// the Dutch design peak 120 / 10 = 12 l/h per inhabitant (peak factor 2,4).
pub const DWA_HOURS: f64 = 10.0;
/// Inhabitants per dwelling. Adjustable default: 2.5 is common in Dutch
/// municipal sewer plans; not verified in a RIONED primary source.
pub const PER_DWELLING: f64 = 2.5;
/// Infiltration (lekwater) in m³ per km of sewer per hour: HBO §5.2.2
/// (up to about 1 m³/(km·h) in soft soils).
pub const INFILTRATION: f64 = 0.2;
/// Self-cleansing velocity (m/s): HBO §5.2.1; applied to the full-pipe
/// velocity, as the main crate's `CIV_RIOOL_TOETS` does.
pub const V_MIN: f64 = 0.6;
/// Maximum velocity (m/s) for continuously loaded sewers: HBO §5.2.1 (2–3 m/s
/// for concrete carrying sand).
pub const V_MAX: f64 = 4.0;
/// Design filling ratio h/D of a foul (DWA) sewer in a separate system:
/// HBO §5.2.1 (30–50 %).
pub const FILL_FOUL: f64 = 0.5;

/// Standard inner diameters (m) of PVC SN 8 sewer pipe (NEN-EN 1401): outside
/// 160, 200, 250, 315, 400, 500 mm minus twice the wall 4.7, 5.9, 7.3, 9.2,
/// 11.7, 14.6 mm (Wavin SN 8 sheet, as in the main crate's `docs/civiel-riolering.md`).
pub fn diameters_pvc() -> Vec<f64> {
    [(160.0, 4.7), (200.0, 5.9), (250.0, 7.3), (315.0, 9.2), (400.0, 11.7), (500.0, 14.6)].iter().map(|(od, w)| (od - 2.0 * w) / 1000.0).collect()
}

/// Standard inner diameters (m) of concrete sewer pipe (NEN-EN 1916): the
/// series of Martens, *Maru betonbuizen* (one manufacturer), 300–1500 mm.
pub fn diameters_concrete() -> Vec<f64> {
    [300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1000.0, 1250.0, 1500.0].iter().map(|d| d / 1000.0).collect()
}

// ── The network ─────────────────────────────────────────────────────────────

fn one() -> f64 {
    1.0
}

/// A manhole or connection point with what drains into it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    /// Ground level (m); `None` when unknown (no flooding check).
    #[serde(default)]
    pub ground: Option<f64>,
    /// Connected paved area (m²).
    #[serde(default)]
    pub area: f64,
    /// Runoff coefficient on `area`: 1 on paved area (HBO §5.2.2).
    #[serde(default = "one")]
    pub runoff: f64,
    #[serde(default)]
    pub inhabitants: f64,
    /// Dwellings, counted as `Loads::per_dwelling` inhabitants each.
    #[serde(default)]
    pub dwellings: f64,
    /// Any further constant inflow, e.g. industry or drainage (m³/s).
    #[serde(default)]
    pub extra: f64,
}

impl Node {
    pub fn new(id: &str) -> Node {
        Node { id: id.into(), ground: None, area: 0.0, runoff: 1.0, inhabitants: 0.0, dwellings: 0.0, extra: 0.0 }
    }
}

/// A free-flow circular pipe from `from` (upstream) to `to`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pipe {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Length (m).
    pub length: f64,
    /// Inner diameter (m).
    pub diameter: f64,
    /// Invert level at the upstream end (m).
    pub invert_up: f64,
    /// Invert level at the downstream end (m).
    pub invert_down: f64,
    /// Wall roughness (mm): PVC 0.4, vitrified clay 0.5, concrete 3.0 as a
    /// first indication (HBO tabel 5.5).
    pub k_mm: f64,
}

impl Pipe {
    /// Bed slope (fraction, positive downhill).
    pub fn slope(&self) -> f64 {
        (self.invert_up - self.invert_down) / self.length.max(1e-9)
    }
}

/// A tree of pipes draining to one or more outfalls (nodes without an outgoing pipe).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Network {
    pub nodes: Vec<Node>,
    pub pipes: Vec<Pipe>,
}

/// Node indices by id, and per pipe its (from, to) node index.
struct Topo {
    from: Vec<usize>,
    to: Vec<usize>,
    out: Vec<Option<usize>>,
    order: Vec<usize>,
}

fn topo(net: &Network) -> Result<Topo, String> {
    let mut idx: HashMap<&str, usize> = HashMap::new();
    for (i, n) in net.nodes.iter().enumerate() {
        if idx.insert(n.id.as_str(), i).is_some() {
            return Err(format!("node {} occurs twice", n.id));
        }
    }
    let find = |id: &str, p: &Pipe| idx.get(id).copied().ok_or_else(|| format!("pipe {}: unknown node {}", p.id, id));
    let mut from = Vec::new();
    let mut to = Vec::new();
    let mut out: Vec<Option<usize>> = vec![None; net.nodes.len()];
    let mut indeg = vec![0usize; net.nodes.len()];
    let mut ids = std::collections::HashSet::new();
    for (i, p) in net.pipes.iter().enumerate() {
        if !ids.insert(p.id.as_str()) {
            return Err(format!("pipe {} occurs twice", p.id));
        }
        if p.length <= 0.0 || p.diameter <= 0.0 {
            return Err(format!("pipe {}: length and diameter must be positive", p.id));
        }
        let (a, b) = (find(&p.from, p)?, find(&p.to, p)?);
        if a == b {
            return Err(format!("pipe {} connects node {} to itself", p.id, p.from));
        }
        if let Some(o) = out[a] {
            return Err(format!("node {} has two outgoing pipes ({} and {}): only tree networks are supported", p.from, net.pipes[o].id, p.id));
        }
        out[a] = Some(i);
        indeg[b] += 1;
        from.push(a);
        to.push(b);
    }
    // Kahn: nodes without inflow first; what is left lies on a cycle.
    let mut stack: Vec<usize> = (0..net.nodes.len()).filter(|&n| indeg[n] == 0).collect();
    let mut order = Vec::new();
    while let Some(n) = stack.pop() {
        if let Some(p) = out[n] {
            order.push(p);
            indeg[to[p]] -= 1;
            if indeg[to[p]] == 0 {
                stack.push(to[p]);
            }
        }
    }
    if order.len() < net.pipes.len() {
        let p = (0..net.pipes.len()).find(|p| !order.contains(p)).unwrap_or(0);
        return Err(format!("the network has a cycle (through pipe {})", net.pipes[p].id));
    }
    Ok(Topo { from, to, out, order })
}

/// Pipe indices in flow order: each pipe after every pipe upstream of it.
/// Errors on unknown or duplicate ids, a node with two outgoing pipes, or a cycle.
pub fn flow_order(net: &Network) -> Result<Vec<usize>, String> {
    topo(net).map(|t| t.order)
}

// ── Design flow ─────────────────────────────────────────────────────────────

/// Load parameters for the design flow; `Default` holds the sourced values above.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loads {
    /// Rain intensity (l/s/ha).
    pub rain: f64,
    /// Waste water per inhabitant per day (l).
    pub dwa_per_day: f64,
    /// Hours over which it is discharged (peak = per_day / hours per hour).
    pub dwa_hours: f64,
    pub per_dwelling: f64,
    /// Infiltration (m³ per km pipe per hour).
    pub infiltration: f64,
}

impl Default for Loads {
    fn default() -> Loads {
        Loads { rain: RAIN, dwa_per_day: DWA_PER_DAY, dwa_hours: DWA_HOURS, per_dwelling: PER_DWELLING, infiltration: INFILTRATION }
    }
}

/// Design flows of one pipe (m³/s), everything upstream included.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipeFlow {
    pub pipe: String,
    /// Rational method: Σ c·i·A.
    pub q_storm: f64,
    /// Peak dry weather flow.
    pub q_dwa: f64,
    /// Infiltration plus the nodes' `extra`.
    pub q_other: f64,
    /// Sum of the three.
    pub q_design: f64,
}

/// Design flow per pipe (same order as `net.pipes`), accumulated downstream:
/// storm Q = c·i·A (rational method, HBO §5.2.2), dry weather flow
/// inhabitants × per_day / hours, infiltration per pipe length. For a separate
/// system give the foul and storm networks their own loads (area 0 or no inhabitants).
pub fn design_flows(net: &Network, loads: &Loads) -> Result<Vec<PipeFlow>, String> {
    let t = topo(net)?;
    let mut inflow = vec![[0.0f64; 3]; net.nodes.len()];
    let mut res = vec![[0.0f64; 3]; net.pipes.len()];
    for &p in &t.order {
        let n = &net.nodes[t.from[p]];
        let storm = n.runoff * loads.rain / 1000.0 * n.area / 10_000.0;
        let inh = n.inhabitants + n.dwellings * loads.per_dwelling;
        let dwa = inh * loads.dwa_per_day / loads.dwa_hours.max(1e-9) / 3600.0 / 1000.0;
        let other = n.extra + loads.infiltration * net.pipes[p].length / 1000.0 / 3600.0;
        let up = inflow[t.from[p]];
        let q = [up[0] + storm, up[1] + dwa, up[2] + other];
        res[p] = q;
        for (a, b) in inflow[t.to[p]].iter_mut().zip(q) {
            *a += b;
        }
    }
    Ok(net
        .pipes
        .iter()
        .zip(res)
        .map(|(p, q)| PipeFlow { pipe: p.id.clone(), q_storm: q[0], q_dwa: q[1], q_other: q[2], q_design: q[0] + q[1] + q[2] })
        .collect())
}

// ── Partially filled circular pipe ──────────────────────────────────────────

/// Wetted geometry of a circle of diameter `d` filled to depth `h`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CircleSection {
    /// Central angle of the wetted arc (rad).
    pub theta: f64,
    pub area: f64,
    pub perimeter: f64,
    pub top_width: f64,
    pub hydraulic_radius: f64,
}

/// Geometry of a part-full circle: θ = 2 acos(1 − 2h/D), A = D²(θ − sin θ)/8,
/// P = Dθ/2, T = D sin(θ/2) (HBO eqs. 5.9–5.10 in another form).
pub fn circle_section(d: f64, h: f64) -> CircleSection {
    let h = h.clamp(0.0, d);
    let theta = 2.0 * (1.0 - 2.0 * h / d).clamp(-1.0, 1.0).acos();
    let area = d * d / 8.0 * (theta - theta.sin());
    let perimeter = d * theta / 2.0;
    let top_width = d * (theta / 2.0).sin();
    let hydraulic_radius = if perimeter > 0.0 { area / perimeter } else { 0.0 };
    CircleSection { theta, area, perimeter, top_width, hydraulic_radius }
}

/// Mean velocity (m/s) after Prandtl-Colebrook with the hydraulic radius:
/// v = −2 log₁₀(k/(3,71·4R) + 2,51ν/(4R·√(2g·4R·I)))·√(2g·4R·I), k in m.
/// With R = D/4 this is the full-pipe formula of the main crate.
pub fn velocity_cw(r: f64, slope: f64, k: f64) -> f64 {
    if r <= 0.0 || slope <= 0.0 {
        return 0.0;
    }
    let dh = 4.0 * r;
    let s = (2.0 * G * dh * slope).sqrt();
    -2.0 * (k / (3.71 * dh) + 2.51 * NU / (dh * s)).log10() * s
}

/// Full-pipe velocity (m/s) and capacity (m³/s); `k` in m.
pub fn full_pipe(d: f64, slope: f64, k: f64) -> (f64, f64) {
    let v = velocity_cw(d / 4.0, slope, k);
    (v, v * PI * d * d / 4.0)
}

/// Uniform part-full flow in a circular pipe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartFull {
    /// Water depth (m).
    pub depth: f64,
    /// h/D.
    pub fill: f64,
    pub area: f64,
    pub velocity: f64,
    pub hydraulic_radius: f64,
    pub v_full: f64,
    pub q_full: f64,
}

fn q_at(d: f64, h: f64, slope: f64, k: f64) -> f64 {
    let c = circle_section(d, h);
    c.area * velocity_cw(c.hydraulic_radius, slope, k)
}

/// Depth of the largest uniform flow (about 0,94 D, about 1,07 × full).
fn h_qmax(d: f64, slope: f64, k: f64) -> f64 {
    let (mut a, mut b) = (0.75 * d, d);
    let r = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..80 {
        let (x1, x2) = (b - r * (b - a), a + r * (b - a));
        if q_at(d, x1, slope, k) < q_at(d, x2, slope, k) {
            a = x1;
        } else {
            b = x2;
        }
    }
    (a + b) / 2.0
}

/// Uniform (normal) flow depth for discharge `q` in a pipe of diameter `d`,
/// slope and roughness `k` (m). Errors when the slope is not positive or `q`
/// exceeds the largest free-surface flow (the pipe must then surcharge).
pub fn part_full(d: f64, slope: f64, k: f64, q: f64) -> Result<PartFull, String> {
    if slope <= 0.0 {
        return Err("no free-surface flow without a positive slope".into());
    }
    let (v_full, q_full) = full_pipe(d, slope, k);
    let hm = h_qmax(d, slope, k);
    if q > q_at(d, hm, slope, k) {
        return Err(format!("{:.4} m³/s exceeds the free-surface capacity", q));
    }
    let (mut a, mut b) = (0.0, hm);
    for _ in 0..100 {
        let m = (a + b) / 2.0;
        if q_at(d, m, slope, k) < q {
            a = m;
        } else {
            b = m;
        }
    }
    let h = (a + b) / 2.0;
    let c = circle_section(d, h);
    let velocity = if c.area > 0.0 { q / c.area } else { 0.0 };
    Ok(PartFull { depth: h, fill: h / d, area: c.area, velocity, hydraulic_radius: c.hydraulic_radius, v_full, q_full })
}

// ── Diameter choice ─────────────────────────────────────────────────────────

/// What a chosen diameter must satisfy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Criteria {
    /// Largest h/D at the design flow; 1 or more means Q ≤ full-pipe capacity,
    /// the usual storm and combined sewer design at 60 l/s/ha (HBO §5.2.1–5.2.2).
    pub max_fill: f64,
    /// Smallest full-pipe velocity (m/s).
    pub v_min: f64,
    /// Largest velocity, full or at the design flow (m/s).
    pub v_max: f64,
    /// Never smaller than a pipe upstream (common practice; can be switched off).
    pub non_decreasing: bool,
}

impl Default for Criteria {
    fn default() -> Criteria {
        Criteria { max_fill: 1.0, v_min: V_MIN, v_max: V_MAX, non_decreasing: true }
    }
}

impl Criteria {
    /// A foul sewer in a separate system: h/D ≤ 0,5 (HBO §5.2.1).
    pub fn foul() -> Criteria {
        Criteria { max_fill: FILL_FOUL, ..Criteria::default() }
    }
}

/// The diameter chosen for one pipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiameterChoice {
    pub pipe: String,
    /// Inner diameter (m); `None` when no candidate carries the flow.
    pub diameter: Option<f64>,
    pub q_design: f64,
    pub q_full: f64,
    pub v_full: f64,
    /// h/D and velocity at the design flow (0 when not computed).
    pub fill: f64,
    pub velocity: f64,
    /// Velocity limits not met, or why nothing fits.
    pub warnings: Vec<String>,
}

/// Capacity and velocity check of one diameter: (carries, velocities ok, part-full or None).
fn check(d: f64, slope: f64, k: f64, q: f64, c: &Criteria) -> (bool, bool, Option<PartFull>, f64, f64) {
    let (v_full, q_full) = full_pipe(d, slope, k);
    let pf = part_full(d, slope, k, q).ok();
    let carries = if c.max_fill >= 1.0 { q <= q_full } else { pf.map(|p| p.fill <= c.max_fill).unwrap_or(false) };
    let v = pf.map(|p| p.velocity).unwrap_or(v_full);
    let v_ok = v_full >= c.v_min && v_full.max(v) <= c.v_max;
    (carries, v_ok, pf, v_full, q_full)
}

/// The smallest of `candidates` (inner diameters, m, any order) at least
/// `at_least` that carries `q` within `c`; when the velocities fail for every
/// carrying size, the smallest carrying one with a warning.
pub fn choose_diameter(q: f64, slope: f64, k: f64, candidates: &[f64], at_least: f64, c: &Criteria) -> (Option<f64>, Vec<String>) {
    if slope <= 0.0 {
        return (None, vec!["no fall: the pipe has no positive slope".into()]);
    }
    let mut ds: Vec<f64> = candidates.iter().copied().filter(|&d| d > 0.0 && d >= at_least - 1e-9).collect();
    ds.sort_by(|a, b| a.total_cmp(b));
    let mut fallback = None;
    for &d in &ds {
        let (carries, v_ok, ..) = check(d, slope, k, q, c);
        if carries && v_ok {
            return (Some(d), vec![]);
        }
        if carries && fallback.is_none() {
            fallback = Some(d);
        }
    }
    match fallback {
        Some(d) => {
            let (_, _, pf, v_full, _) = check(d, slope, k, q, c);
            let v = pf.map(|p| p.velocity).unwrap_or(v_full);
            (Some(d), vec![format!("velocity outside {:.2}–{:.2} m/s (full {:.2}, at design flow {:.2})", c.v_min, c.v_max, v_full, v)])
        }
        None => (None, vec![format!("no candidate diameter carries {:.4} m³/s", q)]),
    }
}

/// A diameter per pipe (same order as `net.pipes`) from the design flows,
/// in flow order so that with `non_decreasing` no pipe is smaller than one upstream.
pub fn choose_diameters(net: &Network, flows: &[PipeFlow], candidates: &[f64], c: &Criteria) -> Result<Vec<DiameterChoice>, String> {
    let t = topo(net)?;
    if flows.len() != net.pipes.len() {
        return Err("one flow per pipe expected".into());
    }
    let mut out: Vec<Option<DiameterChoice>> = vec![None; net.pipes.len()];
    let mut largest_in = vec![0.0f64; net.nodes.len()];
    for &p in &t.order {
        let pipe = &net.pipes[p];
        let q = flows[p].q_design;
        let k = pipe.k_mm / 1000.0;
        let at_least = if c.non_decreasing { largest_in[t.from[p]] } else { 0.0 };
        let (d, warnings) = choose_diameter(q, pipe.slope(), k, candidates, at_least, c);
        let (mut q_full, mut v_full, mut fill, mut velocity) = (0.0, 0.0, 0.0, 0.0);
        if let Some(d) = d {
            let (_, _, pf, vf, qf) = check(d, pipe.slope(), k, q, c);
            q_full = qf;
            v_full = vf;
            if let Some(pf) = pf {
                fill = pf.fill;
                velocity = pf.velocity;
            }
            let to = &mut largest_in[t.to[p]];
            *to = to.max(d);
        }
        out[p] = Some(DiameterChoice { pipe: pipe.id.clone(), diameter: d, q_design: q, q_full, v_full, fill, velocity, warnings });
    }
    Ok(out.into_iter().flatten().collect())
}

// ── Hydraulic grade line ────────────────────────────────────────────────────

/// Boundary conditions and losses for the grade line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HglParams {
    /// Water level at outfalls (node id, m), e.g. the surface water level.
    /// An outfall without one is free: its level is the normal depth in the
    /// pipe flowing in (conservative, no drawdown to critical depth).
    pub outfall_levels: Vec<(String, f64)>,
    /// Manhole loss coefficient ξ on the pipe's velocity head, added at the
    /// upstream manhole of a pressurised pipe. Default 0; HBO §5.4.1 gives
    /// at most 0,5–0,6 inflow plus 1,0 outflow, so up to about 1,5.
    pub manhole_loss: f64,
}

/// Water level at a node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeLevel {
    pub node: String,
    /// Water level (m); `None` for a node without pipes.
    pub level: Option<f64>,
    pub ground: Option<f64>,
    /// The level is above ground (water on the street).
    pub flooded: bool,
}

/// Levels along one pipe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipeLevel {
    pub pipe: String,
    pub level_up: f64,
    pub level_down: f64,
    /// Flowing full over (part of) its length.
    pub surcharged: bool,
    /// Full-pipe friction loss applied (m), 0 for free-surface flow.
    pub friction_loss: f64,
    /// Normal depth h/D, `None` when the flow exceeds free-surface capacity or there is no fall.
    pub normal_fill: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hgl {
    pub nodes: Vec<NodeLevel>,
    pub pipes: Vec<PipeLevel>,
}

/// Darcy friction factor λ for full-pipe flow by Colebrook-White, iterated:
/// 1/√λ = −2 log₁₀(k/(3,71 D) + 2,51/(Re √λ)); `k` in m.
pub fn darcy_lambda(d: f64, v: f64, k: f64) -> f64 {
    let re = (v.abs() * d / NU).max(1.0);
    let mut x: f64 = 8.0; // 1/√λ
    for _ in 0..60 {
        x = -2.0 * (k / (3.71 * d) + 2.51 * x / re).log10();
    }
    1.0 / (x * x)
}

/// Full-pipe friction loss (m) for discharge `q` over `length` (Darcy-Weisbach with `darcy_lambda`).
pub fn friction_loss_full(d: f64, length: f64, k: f64, q: f64) -> f64 {
    let v = q / (PI * d * d / 4.0);
    darcy_lambda(d, v, k) * length / d * v * v / (2.0 * G)
}

/// Steady grade line from the outfalls upstream, pipe by pipe, with `q` the
/// design flow per pipe (same order as `net.pipes`). Per pipe, with H the
/// level at its downstream node:
/// - pressurised when H is above the downstream crown or the flow exceeds
///   free-surface capacity: from max(H, crown) up by the full-pipe friction
///   loss plus the manhole loss; where that line falls below the upstream
///   crown the upstream end runs free at no less than normal depth;
/// - otherwise free: the upstream level is the higher of normal depth and H
///   (a horizontal backwater estimate instead of a gradually varied profile).
///
/// Limitations: no storage, no time; one outgoing pipe per node; no
/// drawdown at free outfalls; manhole losses only on pressurised pipes.
pub fn hydraulic_grade_line(net: &Network, q: &[f64], p: &HglParams) -> Result<Hgl, String> {
    let t = topo(net)?;
    if q.len() != net.pipes.len() {
        return Err("one flow per pipe expected".into());
    }
    let mut level: Vec<Option<f64>> = vec![None; net.nodes.len()];
    for (id, h) in &p.outfall_levels {
        let i = net.nodes.iter().position(|n| &n.id == id).ok_or_else(|| format!("unknown outfall {}", id))?;
        if t.out[i].is_some() {
            return Err(format!("node {} has an outgoing pipe and is no outfall", id));
        }
        level[i] = Some(*h);
    }
    let normal = |i: usize| -> Option<PartFull> {
        let pipe = &net.pipes[i];
        part_full(pipe.diameter, pipe.slope(), pipe.k_mm / 1000.0, q[i]).ok()
    };
    // Free outfalls: normal depth (or the crown) of the pipes flowing in.
    for i in 0..net.pipes.len() {
        let o = t.to[i];
        if t.out[o].is_none() && !p.outfall_levels.iter().any(|(id, _)| id == &net.nodes[o].id) {
            let pipe = &net.pipes[i];
            let h = pipe.invert_down + normal(i).map(|n| n.depth).unwrap_or(pipe.diameter);
            level[o] = Some(level[o].map_or(h, |l: f64| l.max(h)));
        }
    }
    let mut pipes: Vec<Option<PipeLevel>> = vec![None; net.pipes.len()];
    for &i in t.order.iter().rev() {
        let pipe = &net.pipes[i];
        let h_down = level[t.to[i]].ok_or_else(|| format!("pipe {}: no level downstream", pipe.id))?;
        let d = pipe.diameter;
        let (crown_dn, crown_up) = (pipe.invert_down + d, pipe.invert_up + d);
        let nf = normal(i);
        let free_up = nf.map(|n| pipe.invert_up + n.depth);
        let (h_up, surcharged, hf) = if q[i] > 0.0 && (h_down > crown_dn || nf.is_none()) {
            let hf = friction_loss_full(d, pipe.length, pipe.k_mm / 1000.0, q[i]);
            let v = q[i] / (PI * d * d / 4.0);
            let hp = h_down.max(crown_dn) + hf + p.manhole_loss * v * v / (2.0 * G);
            (free_up.map_or(hp, |f| f.max(hp)), true, hf)
        } else {
            (free_up.unwrap_or(pipe.invert_up).max(h_down), h_down > crown_dn, 0.0)
        };
        let h_up = h_up.max(pipe.invert_up);
        level[t.from[i]] = Some(h_up);
        pipes[i] = Some(PipeLevel { pipe: pipe.id.clone(), level_up: h_up, level_down: h_down, surcharged: surcharged || h_up > crown_up + 1e-9, friction_loss: hf, normal_fill: nf.map(|n| n.fill) });
    }
    let nodes = net
        .nodes
        .iter()
        .zip(&level)
        .map(|(n, l)| NodeLevel { node: n.id.clone(), level: *l, ground: n.ground, flooded: matches!((l, n.ground), (Some(l), Some(g)) if *l > g) })
        .collect();
    Ok(Hgl { nodes, pipes: pipes.into_iter().flatten().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe(id: &str, from: &str, to: &str, len: f64, d: f64, up: f64, dn: f64) -> Pipe {
        Pipe { id: id.into(), from: from.into(), to: to.into(), length: len, diameter: d, invert_up: up, invert_down: dn, k_mm: 0.4 }
    }

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{} != {} (tol {})", a, b, tol);
    }

    #[test]
    fn half_full_pipe() {
        let d = 0.5;
        let c = circle_section(d, d / 2.0);
        close(c.area, PI * d * d / 8.0, 1e-12);
        close(c.hydraulic_radius, d / 4.0, 1e-12);
        close(c.top_width, d, 1e-12);
        let (s, k) = (1.0 / 500.0, 0.4e-3);
        let (v_full, q_full) = full_pipe(d, s, k);
        close(velocity_cw(c.hydraulic_radius, s, k), v_full, 1e-12);
        // Half the full capacity gives exactly half filling.
        let pf = part_full(d, s, k, q_full / 2.0).unwrap();
        close(pf.fill, 0.5, 1e-6);
        close(pf.velocity, v_full, 1e-6);
    }

    #[test]
    fn full_circle_and_capacity_limit() {
        let c = circle_section(1.0, 1.0);
        close(c.area, PI / 4.0, 1e-12);
        close(c.perimeter, PI, 1e-12);
        let (_, q_full) = full_pipe(0.3, 0.004, 0.4e-3);
        // Largest free flow about 1.07 × full, above that no free surface.
        assert!(part_full(0.3, 0.004, 0.4e-3, 1.05 * q_full).is_ok());
        assert!(part_full(0.3, 0.004, 0.4e-3, 1.10 * q_full).is_err());
        let pf = part_full(0.3, 0.004, 0.4e-3, q_full).unwrap();
        assert!(pf.fill > 0.8 && pf.fill < 0.94, "{}", pf.fill);
    }

    #[test]
    fn rational_method_along_a_chain() {
        let mut nodes: Vec<Node> = ["A", "B", "C", "D"].iter().map(|id| Node::new(id)).collect();
        nodes[0].area = 2000.0;
        nodes[1].area = 3000.0;
        nodes[1].inhabitants = 100.0;
        nodes[2].area = 5000.0;
        nodes[2].dwellings = 40.0;
        let pipes = vec![pipe("1", "A", "B", 50.0, 0.3, 1.0, 0.8), pipe("2", "B", "C", 50.0, 0.3, 0.8, 0.6), pipe("3", "C", "D", 50.0, 0.3, 0.6, 0.4)];
        let net = Network { nodes, pipes };
        let loads = Loads { infiltration: 0.0, ..Loads::default() };
        let f = design_flows(&net, &loads).unwrap();
        // 60 l/s/ha: 0.2 ha → 12 l/s, 0.5 ha → 30 l/s, 1.0 ha → 60 l/s.
        close(f[0].q_storm, 0.012, 1e-12);
        close(f[1].q_storm, 0.030, 1e-12);
        close(f[2].q_storm, 0.060, 1e-12);
        // 12 l/h per inhabitant; 40 dwellings × 2.5 = 100 inhabitants.
        close(f[0].q_dwa, 0.0, 1e-15);
        close(f[1].q_dwa, 100.0 * 12.0 / 3.6e6, 1e-12);
        close(f[2].q_dwa, 200.0 * 12.0 / 3.6e6, 1e-12);
        close(f[2].q_design, f[2].q_storm + f[2].q_dwa, 1e-15);
        // Infiltration: 0.2 m³/(km·h) over 150 m.
        let f = design_flows(&net, &Loads::default()).unwrap();
        close(f[2].q_other, 0.2 * 0.15 / 3600.0, 1e-12);
    }

    #[test]
    fn branches_add_up() {
        let nodes: Vec<Node> = ["A", "B", "C"].iter().map(|id| Node { area: 10_000.0, ..Node::new(id) }).collect();
        let pipes = vec![pipe("2", "B", "C", 40.0, 0.3, 0.8, 0.6), pipe("1", "A", "B", 40.0, 0.3, 1.0, 0.8)];
        let mut net = Network { nodes, pipes };
        net.nodes.push(Node { area: 10_000.0, ..Node::new("E") });
        net.pipes.push(pipe("4", "E", "B", 40.0, 0.3, 1.0, 0.8));
        let f = design_flows(&net, &Loads { infiltration: 0.0, ..Loads::default() }).unwrap();
        close(f[0].q_storm, 0.18, 1e-12);
        assert_eq!(flow_order(&net).unwrap().last(), Some(&0));
    }

    #[test]
    fn diameter_choice_at_the_boundary() {
        let (s, k) = (1.0 / 300.0, 0.4e-3);
        let ds = [0.2, 0.25, 0.3, 0.4, 0.5];
        let c = Criteria { v_min: 0.0, v_max: 10.0, ..Criteria::default() };
        let (_, q30) = full_pipe(0.3, s, k);
        assert_eq!(choose_diameter(0.999 * q30, s, k, &ds, 0.0, &c).0, Some(0.3));
        assert_eq!(choose_diameter(1.001 * q30, s, k, &ds, 0.0, &c).0, Some(0.4));
        // Foul sewer at h/D ≤ 0.5: half of the full capacity is the limit.
        let f = Criteria { max_fill: 0.5, ..c.clone() };
        assert_eq!(choose_diameter(0.499 * q30, s, k, &ds, 0.0, &f).0, Some(0.3));
        assert_eq!(choose_diameter(0.501 * q30, s, k, &ds, 0.0, &f).0, Some(0.4));
        // Too much flow for any candidate.
        assert_eq!(choose_diameter(10.0, s, k, &ds, 0.0, &c).0, None);
        // Self-cleansing velocity not reached: the smallest carrying size with a warning.
        let strict = Criteria { v_min: 5.0, ..Criteria::default() };
        let (d, w) = choose_diameter(0.001, s, k, &ds, 0.0, &strict);
        assert_eq!(d, Some(0.2));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn diameters_never_shrink_downstream() {
        let nodes: Vec<Node> = ["A", "B", "C"].iter().map(|id| Node::new(id)).collect();
        // A steep first pipe needs 0.3, the second is so steep 0.2 would do.
        let pipes = vec![pipe("1", "A", "B", 100.0, 0.3, 10.0, 9.8), pipe("2", "B", "C", 100.0, 0.3, 9.8, 5.0)];
        let net = Network { nodes, pipes };
        let flows: Vec<PipeFlow> = net.pipes.iter().map(|p| PipeFlow { pipe: p.id.clone(), q_storm: 0.06, q_dwa: 0.0, q_other: 0.0, q_design: 0.06 }).collect();
        let c = Criteria { v_min: 0.0, v_max: 10.0, ..Criteria::default() };
        let ch = choose_diameters(&net, &flows, &[0.2, 0.25, 0.3, 0.4], &c).unwrap();
        let free = choose_diameters(&net, &flows, &[0.2, 0.25, 0.3, 0.4], &Criteria { non_decreasing: false, ..c }).unwrap();
        assert!(ch[1].diameter >= ch[0].diameter);
        assert!(free[1].diameter < free[0].diameter);
    }

    #[test]
    fn diameter_series() {
        let pvc = diameters_pvc();
        close(pvc[3], 0.2966, 1e-9);
        assert_eq!(diameters_concrete()[0], 0.3);
    }

    #[test]
    fn surcharged_single_pipe() {
        let mut nodes = vec![Node::new("A"), Node::new("B")];
        nodes[0].ground = Some(2.0);
        let net = Network { nodes, pipes: vec![pipe("1", "A", "B", 80.0, 0.4, 0.2, 0.0)] };
        let q = 0.15;
        let hgl = hydraulic_grade_line(&net, &[q], &HglParams { outfall_levels: vec![("B".into(), 1.0)], manhole_loss: 0.0 }).unwrap();
        // Independent: the slope at which the full-pipe capacity is q, times the length.
        let (mut a, mut b) = (1e-7f64, 1.0f64);
        for _ in 0..200 {
            let m = (a * b).sqrt();
            if full_pipe(0.4, m, 0.4e-3).1 < q {
                a = m;
            } else {
                b = m;
            }
        }
        let expected = 1.0 + a * 80.0;
        close(hgl.nodes[0].level.unwrap(), expected, 1e-6);
        assert!(hgl.pipes[0].surcharged);
        assert!(!hgl.nodes[0].flooded);
        // With a manhole loss ξ = 1.5 on top, and the ground lowered: water on the street.
        let mut net2 = net.clone();
        net2.nodes[0].ground = Some(expected);
        let h2 = hydraulic_grade_line(&net2, &[q], &HglParams { outfall_levels: vec![("B".into(), 1.0)], manhole_loss: 1.5 }).unwrap();
        let v = q / (PI * 0.04);
        close(h2.nodes[0].level.unwrap(), expected + 1.5 * v * v / (2.0 * G), 1e-6);
        assert!(h2.nodes[0].flooded);
    }

    #[test]
    fn free_flow_follows_normal_depth() {
        let net = Network { nodes: vec![Node::new("A"), Node::new("B"), Node::new("C")], pipes: vec![pipe("1", "A", "B", 50.0, 0.3, 1.0, 0.8), pipe("2", "B", "C", 50.0, 0.3, 0.8, 0.6)] };
        let q = [0.02, 0.02];
        let hgl = hydraulic_grade_line(&net, &q, &HglParams::default()).unwrap();
        let n = part_full(0.3, 0.004, 0.4e-3, 0.02).unwrap();
        close(hgl.nodes[0].level.unwrap(), 1.0 + n.depth, 1e-9);
        close(hgl.nodes[2].level.unwrap(), 0.6 + n.depth, 1e-9);
        assert!(hgl.pipes.iter().all(|p| !p.surcharged));
    }

    #[test]
    fn cycles_and_bad_networks() {
        let nodes: Vec<Node> = ["A", "B", "C"].iter().map(|id| Node::new(id)).collect();
        let pipes = vec![pipe("1", "A", "B", 10.0, 0.3, 1.0, 0.9), pipe("2", "B", "C", 10.0, 0.3, 0.9, 0.8), pipe("3", "C", "A", 10.0, 0.3, 0.8, 0.7)];
        let net = Network { nodes: nodes.clone(), pipes };
        let e = design_flows(&net, &Loads::default()).unwrap_err();
        assert!(e.contains("cycle"), "{}", e);
        let split = Network { nodes: nodes.clone(), pipes: vec![pipe("1", "A", "B", 10.0, 0.3, 1.0, 0.9), pipe("2", "A", "C", 10.0, 0.3, 1.0, 0.9)] };
        assert!(flow_order(&split).unwrap_err().contains("two outgoing"));
        let unknown = Network { nodes, pipes: vec![pipe("1", "A", "X", 10.0, 0.3, 1.0, 0.9)] };
        assert!(flow_order(&unknown).unwrap_err().contains("unknown node"));
    }
}
