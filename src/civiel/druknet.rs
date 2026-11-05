//! `CIV_DRUKNET` (issue #67): a drinking-water or pressure network from the
//! lines in the drawing — the selection, the lines on `LAAG=`, or the
//! infra library's water mains — solved with `rekenkern::druknet` (global
//! gradient method). Line ends within `SNAP` m become one node, a line end
//! on another line splits it (a tee); node levels come from the ground.
//! Pressure per node and flow and velocity per pipe are labelled in the
//! drawing, each hydrant gets a fire-flow check, and a Markdown and JSON
//! report is written beside the drawing.
//!
//! The method, defaults and limitations are in `docs/civiel-water.md`.

use std::collections::{BTreeMap, HashSet};

use ocs_plugin_api::host::codec::{EntityType, Handle};
use rekenkern::druknet::{self as kern, FireFlowOptions, FireFlowResult, Friction, Junction, LinkStatus, Network, Pipe, Reservoir, Solution, SolverOptions};
use serde_json::{json, Value};

use super::{api, data_value, Ctx, Maaiveld, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::Element;

/// A message in the interface language, with positional arguments.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

// ── Design values (sources: docs/civiel-water.md) ───────────────────────────

/// Hazen-Williams C when no RUWHEID= is given: EPANET 2 users manual
/// (Rossman 2000) table 3.2 gives 130–140 for new cast iron and 140–150 for
/// plastic; 130 allows for some ageing. Adjustable.
pub const HAZEN_WILLIAMS_C: f64 = 130.0;
/// Line ends closer than this (m) are one node.
pub const SNAP: f64 = 0.5;
/// A hydrant or source at most this far (m) from the network is connected
/// to it (at the nearest node, or by splitting the nearest pipe).
pub const KOPPEL: f64 = 10.0;
/// The layers read without a selection or `LAAG=`.
pub const STANDAARD_LAAG: &str = "*-KL-WATER_TRANSPORTLEIDING*";
/// Infra library items that are pipes of the network.
fn is_pipe_item(item: &str) -> bool {
    item == "waterleiding" || item.starts_with("persleiding")
}

pub const ARGS: &[&str] = &["LAAG", "BRON", "VRAAG", "DIAMETER", "RUWHEID", "BRANDKRAAN", "MINDRUK", "BLUSDEBIET"];

/// Demand on the junctions.
#[derive(Clone, Debug, PartialEq)]
pub enum Vraag {
    /// The same on every junction (m³/s).
    PerKnoop(f64),
    /// A total (m³/s) spread over the junctions by the pipe length around them.
    Totaal(f64),
}

/// What `CIV_DRUKNET` computes.
#[derive(Clone, Debug)]
pub struct Druknet {
    /// Layer patterns (`*` wildcards); empty: the selection, or the water mains.
    pub lagen: Vec<String>,
    /// Sources: project point and head (m).
    pub bronnen: Vec<(XY, f64)>,
    pub vraag: Vraag,
    /// Internal diameter (m) of pipes without one in their object data.
    pub diameter: Option<f64>,
    pub wrijving: Friction,
    /// Hydrants given as project points (next to the library's hydrants).
    pub brandkranen: Vec<XY>,
    pub mindruk_kpa: f64,
    /// Required fire flow (m³/s).
    pub blusdebiet: f64,
}

fn num(t: &str, what: &str) -> Result<f64, String> {
    t.trim().replace(',', ".").parse::<f64>().map_err(|_| t!("{}: geen getal '{}'", "{}: not a number '{}'", what, t.trim()))
}

/// BRON=x,y,stijghoogte;… (project coordinates, head in m).
pub fn parse_bronnen(s: &str) -> Result<Vec<(XY, f64)>, String> {
    s.split(';')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let f: Vec<&str> = p.split(',').collect();
            if f.len() != 3 {
                return Err(t!("BRON: verwacht x,y,stijghoogte, kreeg '{}'", "BRON: expected x,y,head, got '{}'", p));
            }
            let v: Vec<f64> = f.iter().map(|t| t.trim().parse::<f64>().map_err(|_| t!("BRON: geen getal '{}'", "BRON: not a number '{}'", t.trim()))).collect::<Result<_, _>>()?;
            Ok(([v[0], v[1]], v[2]))
        })
        .collect()
}

/// VRAAG=<l/s per knoop> | TOTAAL:<l/s>.
pub fn parse_vraag(s: &str) -> Result<Vraag, String> {
    let t = s.trim().to_ascii_uppercase();
    let (total, v) = match t.split_once(':') {
        Some((a, b)) if a == "TOTAAL" || a == "TOTAL" => (true, b),
        Some((a, b)) if b == "TOTAAL" || b == "TOTAL" => (true, a),
        Some(_) => return Err(tr("VRAAG: l/s per knoop, of TOTAAL:<l/s>", "VRAAG: l/s per node, or TOTAAL:<l/s> (total)").into()),
        None => (false, t.as_str()),
    };
    let q = num(v, "VRAAG")? / 1000.0;
    if q < 0.0 {
        return Err(tr("VRAAG: niet negatief", "VRAAG: not negative").into());
    }
    Ok(if total { Vraag::Totaal(q) } else { Vraag::PerKnoop(q) })
}

/// RUWHEID=: Hazen-Williams C (20 and up) or Darcy-Weisbach k in mm (below 20).
pub fn parse_wrijving(s: &str) -> Result<Friction, String> {
    let v = num(s, "RUWHEID")?;
    match v {
        v if v >= 20.0 => Ok(Friction::HazenWilliams { c: v }),
        v if v >= 0.0 => Ok(Friction::DarcyWeisbach { k_mm: v }),
        _ => Err(tr("RUWHEID: Hazen-Williams C (bijv. 130) of k in mm (bijv. 0.1)", "RUWHEID: Hazen-Williams C (e.g. 130) or k in mm (e.g. 0.1)").into()),
    }
}

impl Druknet {
    pub fn from_args(a: &api::Args) -> Result<Druknet, String> {
        let diameter = a.f64("DIAMETER")?.map(super::waterloop::metres);
        if diameter.is_some_and(|d| d <= 0.0) {
            return Err(tr("DIAMETER: binnendiameter in mm, groter dan 0", "DIAMETER: internal diameter in mm, above 0").into());
        }
        let r = Druknet {
            lagen: a.str("LAAG").map(|l| l.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
            bronnen: a.str("BRON").map(parse_bronnen).transpose()?.unwrap_or_default(),
            vraag: a.str("VRAAG").map(parse_vraag).transpose()?.unwrap_or(Vraag::PerKnoop(0.0)),
            diameter,
            wrijving: a.str("RUWHEID").map(parse_wrijving).transpose()?.unwrap_or(Friction::HazenWilliams { c: HAZEN_WILLIAMS_C }),
            brandkranen: a.points("BRANDKRAAN")?.unwrap_or_default(),
            mindruk_kpa: a.f64("MINDRUK")?.unwrap_or(kern::FIRE_MIN_PRESSURE_KPA),
            blusdebiet: a.f64("BLUSDEBIET")?.map(|v| v / 3600.0).unwrap_or(kern::FIRE_FLOW_REQUIRED),
        };
        if r.bronnen.is_empty() {
            return Err(tr("geef minstens één bron: BRON=x,y,stijghoogte (m NAP)", "give at least one source: BRON=x,y,head (m)").into());
        }
        if r.blusdebiet < 0.0 {
            return Err(tr("BLUSDEBIET: m³/h, niet negatief", "BLUSDEBIET: m³/h, not negative").into());
        }
        Ok(r)
    }
}

// ── The network from lines ──────────────────────────────────────────────────

/// A line read from the drawing (project metres).
#[derive(Clone, Debug, PartialEq)]
pub struct Lijn {
    pub pts: Vec<XY>,
    /// Internal diameter (m) from the object data.
    pub diameter: Option<f64>,
    /// Where it came from (a handle), for the report.
    pub bron: String,
}

/// A pipe between two nodes, along its line.
#[derive(Clone, Debug, PartialEq)]
pub struct Leiding {
    pub van: usize,
    pub naar: usize,
    pub pts: Vec<XY>,
    pub diameter: Option<f64>,
    pub bron: String,
}

impl Leiding {
    pub fn lengte(&self) -> f64 {
        length(&self.pts)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Net {
    pub knopen: Vec<XY>,
    pub leidingen: Vec<Leiding>,
}

fn dist(a: XY, b: XY) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn length(pts: &[XY]) -> f64 {
    pts.windows(2).map(|w| dist(w[0], w[1])).sum()
}

/// The point of a polyline nearest `p`: (distance, arc length, point).
fn project(pts: &[XY], p: XY) -> (f64, f64, XY) {
    let mut best = (f64::MAX, 0.0, pts[0]);
    let mut run = 0.0;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = dist(a, b);
        let t = if l > 0.0 { (((p[0] - a[0]) * (b[0] - a[0]) + (p[1] - a[1]) * (b[1] - a[1])) / (l * l)).clamp(0.0, 1.0) } else { 0.0 };
        let q = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let d = dist(p, q);
        if d < best.0 {
            best = (d, run + t * l, q);
        }
        run += l;
    }
    best
}

/// A polyline cut at arc lengths (sorted, inside), each cut at its point.
fn cut(pts: &[XY], cuts: &[(f64, XY)]) -> Vec<Vec<XY>> {
    let mut out = vec![vec![pts[0]]];
    let mut run = 0.0;
    let mut c = 0;
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        while c < cuts.len() && cuts[c].0 <= run + l {
            out.last_mut().unwrap().push(cuts[c].1);
            out.push(vec![cuts[c].1]);
            c += 1;
        }
        out.last_mut().unwrap().push(w[1]);
        run += l;
    }
    for piece in &mut out {
        piece.dedup_by(|a, b| dist(*a, *b) < 1e-9);
    }
    out.retain(|p| p.len() >= 2 && length(p) > 1e-6);
    out
}

/// The node at `p`: an existing one within `tol`, or a new one.
fn node_at(nodes: &mut Vec<XY>, p: XY, tol: f64) -> usize {
    match nodes.iter().position(|n| dist(*n, p) <= tol) {
        Some(i) => i,
        None => {
            nodes.push(p);
            nodes.len() - 1
        }
    }
}

/// The network of the lines: ends within `tol` are one node, a node on
/// another line splits that line, a closed ring gets a node halfway.
pub fn bouw(lijnen: &[Lijn], tol: f64) -> Result<Net, String> {
    let mut lines: Vec<Lijn> = lijnen.to_vec();
    for l in &mut lines {
        l.pts.dedup_by(|a, b| dist(*a, *b) < 1e-9);
    }
    lines.retain(|l| l.pts.len() >= 2 && length(&l.pts) > tol.max(1e-6));
    if lines.is_empty() {
        return Err(tr("geen leidingen: selecteer lijnen of polylijnen, of geef LAAG=", "no pipes: select lines or polylines, or give LAAG=").into());
    }
    let mut nodes: Vec<XY> = Vec::new();
    for l in &lines {
        node_at(&mut nodes, l.pts[0], tol);
        node_at(&mut nodes, *l.pts.last().unwrap(), tol);
    }
    let mut net = Net { knopen: nodes.clone(), leidingen: Vec::new() };
    for l in &lines {
        let len = length(&l.pts);
        // Nodes on this line away from its ends: tees.
        let mut cuts: Vec<(f64, XY)> = nodes
            .iter()
            .filter_map(|n| {
                let (d, s, q) = project(&l.pts, *n);
                (d <= tol && s > tol && s < len - tol).then_some((s, q))
            })
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        for piece in cut(&l.pts, &cuts) {
            let van = node_at(&mut net.knopen, piece[0], tol);
            let naar = node_at(&mut net.knopen, *piece.last().unwrap(), tol);
            net.leidingen.push(Leiding { van, naar, pts: piece, diameter: l.diameter, bron: l.bron.clone() });
        }
    }
    // A pipe from a node to itself (a ring): a node halfway.
    let rings: Vec<usize> = (0..net.leidingen.len()).filter(|&i| net.leidingen[i].van == net.leidingen[i].naar).collect();
    for i in rings.into_iter().rev() {
        let half = net.leidingen[i].lengte() / 2.0;
        split(&mut net, i, half, tol);
    }
    Ok(net)
}

/// Split pipe `i` at arc length `s`; the new (or existing) node there.
fn split(net: &mut Net, i: usize, s: f64, tol: f64) -> usize {
    let p = net.leidingen.remove(i);
    let at = {
        let mut run = 0.0;
        let mut q = p.pts[0];
        for w in p.pts.windows(2) {
            let l = dist(w[0], w[1]);
            if run + l >= s {
                let t = if l > 0.0 { (s - run) / l } else { 0.0 };
                q = [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
                break;
            }
            run += l;
        }
        q
    };
    let k = node_at(&mut net.knopen, at, tol.min(1e-6));
    let pieces = cut(&p.pts, &[(s, at)]);
    let ends = [p.van, p.naar];
    for (j, piece) in pieces.into_iter().enumerate() {
        let (van, naar) = if j == 0 { (ends[0], k) } else { (k, ends[1]) };
        net.leidingen.push(Leiding { van, naar, pts: piece, diameter: p.diameter, bron: p.bron.clone() });
    }
    k
}

/// The node for a hydrant or source at `p`: the nearest node, or a new node
/// on the nearest pipe (which is split), at most `max` m away.
pub fn koppel(net: &mut Net, p: XY, max: f64) -> Result<usize, String> {
    let node = net.knopen.iter().enumerate().map(|(i, n)| (dist(*n, p), i)).min_by(|a, b| a.0.total_cmp(&b.0));
    let pipe = net.leidingen.iter().enumerate().map(|(i, l)| (project(&l.pts, p), i)).min_by(|a, b| a.0 .0.total_cmp(&b.0 .0));
    match (node, pipe) {
        (Some((dn, n)), Some(((dp, s, _), i))) => {
            if dn <= max && dn <= dp + SNAP {
                Ok(n)
            } else if dp <= max {
                let len = net.leidingen[i].lengte();
                if s <= SNAP {
                    Ok(net.leidingen[i].van)
                } else if s >= len - SNAP {
                    Ok(net.leidingen[i].naar)
                } else {
                    Ok(split(net, i, s, SNAP))
                }
            } else {
                Err(t!("punt {:.1}, {:.1} ligt {:.0} m van het net (meer dan {} m)", "point {:.1}, {:.1} lies {:.0} m from the network (more than {} m)", p[0], p[1], dp.min(dn), max))
            }
        }
        _ => Err(tr("geen net", "no network").into()),
    }
}

/// Demand per node (m³/s); fixed-head nodes take none. A total is spread by
/// half the length of the pipes at each node.
pub fn vraag(net: &Net, v: &Vraag, vast: &[bool]) -> Vec<f64> {
    let n = net.knopen.len();
    match *v {
        Vraag::PerKnoop(q) => (0..n).map(|i| if vast[i] { 0.0 } else { q }).collect(),
        Vraag::Totaal(q) => {
            let mut w = vec![0.0; n];
            for l in &net.leidingen {
                w[l.van] += l.lengte() / 2.0;
                w[l.naar] += l.lengte() / 2.0;
            }
            for (wi, f) in w.iter_mut().zip(vast) {
                if *f {
                    *wi = 0.0;
                }
            }
            let sum: f64 = w.iter().sum();
            if sum > 0.0 { w.iter().map(|x| q * x / sum).collect() } else { vec![0.0; n] }
        }
    }
}

/// Node ids: "BRON1" … for the sources, "K1" … for the rest.
pub fn ids(net: &Net, bronnen: &[(usize, f64)]) -> Vec<String> {
    (0..net.knopen.len())
        .map(|i| match bronnen.iter().position(|b| b.0 == i) {
            Some(j) => format!("BRON{}", j + 1),
            None => format!("K{}", i + 1),
        })
        .collect()
}

/// The kernel network: junctions with level and demand, the sources as
/// reservoirs, a pipe per piece ("L1" …).
pub fn kern_net(net: &Net, hoogte: &[f64], vraag: &[f64], bronnen: &[(usize, f64)], wrijving: &Friction, standaard: Option<f64>) -> Result<Network, String> {
    let id = ids(net, bronnen);
    let missing = net.leidingen.iter().filter(|l| l.diameter.is_none()).count();
    if missing > 0 && standaard.is_none() {
        return Err(t!("{} leiding(en) zonder diameter in de objectdata: geef DIAMETER=<mm>", "{} pipe(s) without a diameter in their object data: give DIAMETER=<mm>", missing));
    }
    let mut out = Network::default();
    for (i, _) in net.knopen.iter().enumerate() {
        match bronnen.iter().find(|b| b.0 == i) {
            Some(b) => out.reservoirs.push(Reservoir { id: id[i].clone(), head: b.1 }),
            None => out.junctions.push(Junction { id: id[i].clone(), elevation: hoogte[i], demand: vraag[i] }),
        }
    }
    for (j, l) in net.leidingen.iter().enumerate() {
        out.pipes.push(Pipe {
            id: format!("L{}", j + 1),
            from: id[l.van].clone(),
            to: id[l.naar].clone(),
            length: l.lengte(),
            diameter: l.diameter.or(standaard).unwrap_or(0.0),
            friction: wrijving.clone(),
            minor_loss: 0.0,
            status: LinkStatus::Open,
        });
    }
    Ok(out)
}

/// An infra library object's item and placement, from its object data (the
/// primary entity only).
pub fn infra(d: &BTreeMap<String, String>) -> Option<(String, Value)> {
    let item = d.get("infra_item")?;
    if d.get("infra_primary").map(String::as_str) != Some("1") {
        return Some((item.clone(), Value::Null));
    }
    let text: String = d.iter().filter(|(k, _)| k.starts_with("infra_p_")).map(|(_, v)| v.as_str()).collect();
    Some((item.clone(), serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// A library pipe's diameter (m): its own value, else the catalogue default.
pub fn library_diameter(item: &str, placement: &Value) -> Option<f64> {
    placement["values"]["diameter"]
        .as_f64()
        .or_else(|| crate::library::infra::find(item).ok().and_then(|i| i.parameters.iter().find(|x| x.key == "diameter").map(|x| x.default)))
        .filter(|v| *v > 0.0)
        .map(|v| v / 1000.0)
}

// ── In the drawing ──────────────────────────────────────────────────────────

fn dec(v: f64, n: usize) -> String {
    let s = format!("{v:.n$}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

/// A diameter in mm from object data ("110", "Ø110", "110 mm").
fn mm(s: &str) -> Option<f64> {
    let t: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
    t.replace(',', ".").parse::<f64>().ok().filter(|v| *v > 0.0).map(super::waterloop::metres)
}

/// The lines and hydrants to read: on the layers, else the selection, else
/// the water mains (library objects and the KL water transport layer).
fn lees(ctx: &Ctx, doc: &ocs_plugin_api::host::codec::CadDocument, lagen: &[String]) -> Result<(Vec<Lijn>, Vec<XY>), String> {
    use super::hydrologie::layer_matches;
    let sel: HashSet<Handle> = if lagen.is_empty() { bridge::selection().unwrap_or_default().into_iter().collect() } else { HashSet::new() };
    let mut lijnen = Vec::new();
    let mut kranen = Vec::new();
    for e in doc.model_space_entities() {
        let layer = &e.common().layer;
        let h = e.common().handle;
        let lib = infra(&crate::library::infra::host::data(e));
        let wanted = if !lagen.is_empty() {
            lagen.iter().any(|p| layer_matches(p, layer))
        } else if !sel.is_empty() {
            sel.contains(&h)
        } else {
            layer_matches(STANDAARD_LAAG, layer) || lib.as_ref().is_some_and(|(i, _)| is_pipe_item(i) || i == "hydrant") || layer_matches("*BRANDHYDRANT*", layer)
        };
        if !wanted {
            continue;
        }
        if let Some((item, p)) = lib {
            if p.is_null() {
                continue; // a secondary part of a library object
            }
            if item == "hydrant" {
                if let (Some(x), Some(y)) = (p["at"][0].as_f64(), p["at"][1].as_f64()) {
                    kranen.push([x, y]);
                }
                continue;
            }
            if !is_pipe_item(&item) {
                continue;
            }
            if let Some(l) = super::grading::line_of(e) {
                let d = library_diameter(&item, &p);
                lijnen.push(Lijn { pts: l.pts.iter().map(|q| ctx.from_drawing([q[0], q[1]])).collect(), diameter: d, bron: api::handle_text(h) });
            }
            continue;
        }
        let at = match e {
            EntityType::Point(p) => Some([p.location.x, p.location.y]),
            EntityType::Insert(i) => Some([i.insert_point.x, i.insert_point.y]),
            EntityType::Circle(c) => Some([c.center.x, c.center.y]),
            _ => None,
        };
        if let Some(at) = at {
            if layer_matches("*BRANDHYDRANT*", layer) {
                kranen.push(ctx.from_drawing(at));
            }
            continue;
        }
        if let Some(l) = super::grading::line_of(e) {
            let d = data_value(e, "binnendiameter_mm").or_else(|| data_value(e, "diameter")).or_else(|| data_value(e, "dn")).and_then(|v| mm(&v));
            lijnen.push(Lijn { pts: l.pts.iter().map(|q| ctx.from_drawing([q[0], q[1]])).collect(), diameter: d, bron: api::handle_text(h) });
        }
    }
    Ok((lijnen, kranen))
}

/// `CIV_DRUKNET`: build, solve, check hydrants, label and report.
pub fn druknet(opt: &super::CivielOptions, r: Druknet, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let k = ctx.k;
    let doc = super::document(&["tin"], &[])?;
    let (lijnen, mut kranen) = lees(&ctx, &doc, &r.lagen)?;
    let mut net = bouw(&lijnen, SNAP)?;
    let mut bronnen = Vec::new();
    for (p, head) in &r.bronnen {
        let n = koppel(&mut net, *p, KOPPEL).map_err(|e| format!("BRON: {e}"))?;
        bronnen.push((n, *head));
    }
    kranen.extend(r.brandkranen.iter().copied());
    let mut hydranten = Vec::new();
    let mut notes = Vec::new();
    for p in &kranen {
        match koppel(&mut net, *p, KOPPEL) {
            Ok(n) if bronnen.iter().any(|b| b.0 == n) => notes.push(t!("brandkraan bij {:.1}, {:.1} valt op een bron", "hydrant at {:.1}, {:.1} coincides with a source", p[0], p[1])),
            Ok(n) => {
                if !hydranten.contains(&n) {
                    hydranten.push(n);
                }
            }
            Err(e) => notes.push(format!("{}: {e}", tr("brandkraan", "hydrant"))),
        }
    }
    let vast: Vec<bool> = (0..net.knopen.len()).map(|i| bronnen.iter().any(|b| b.0 == i)).collect();
    progress(t!("Maaiveld bij {} knopen …", "Ground at {} nodes …", net.knopen.len()));
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let known: Vec<Option<f64>> = net.knopen.iter().map(|p| ground.height(ctx.to_drawing(*p), ctx.f)).collect();
    let have: Vec<f64> = known.iter().flatten().copied().collect();
    if have.is_empty() {
        return Err(ground.error().unwrap_or_else(|| t!("geen maaiveldhoogtes bij de knopen ({})", "no ground levels at the nodes ({})", ground.label())));
    }
    let mean = have.iter().sum::<f64>() / have.len() as f64;
    let missing = known.iter().filter(|z| z.is_none()).count();
    if missing > 0 {
        notes.push(t!("{} knopen zonder maaiveld: gemiddelde {} aangenomen", "{} nodes without ground level: mean {} assumed", missing, nap(mean, 2)));
    }
    let hoogte: Vec<f64> = known.iter().map(|z| z.unwrap_or(mean)).collect();
    let q = vraag(&net, &r.vraag, &vast);
    let kn = kern_net(&net, &hoogte, &q, &bronnen, &r.wrijving, r.diameter)?;
    progress(t!("{} knopen, {} leidingen oplossen …", "Solving {} nodes, {} pipes …", net.knopen.len(), net.leidingen.len()));
    let sopts = SolverOptions::default();
    let sol = kern::solve(&kn, &sopts)?;
    let fopts = FireFlowOptions { min_pressure_kpa: r.mindruk_kpa, required_flow: r.blusdebiet, ..FireFlowOptions::default() };
    let id = ids(&net, &bronnen);
    let mut fire: Vec<(usize, FireFlowResult)> = Vec::new();
    for (j, n) in hydranten.iter().enumerate() {
        progress(t!("Bluswater brandkraan {}/{} …", "Fire flow hydrant {}/{} …", j + 1, hydranten.len()));
        fire.push((*n, kern::fire_flow(&kn, &id[*n], &fopts, &sopts)?));
    }

    // ── Labels ──
    let lt = ctx.layer("KL", "WATER_TRANSPORTLEIDING", Element::T)?;
    let hg = ctx.layer("KL", "WATER_BRANDLEIDING_BRANDHYDRANT", Element::G)?;
    let ht = ctx.layer("KL", "WATER_BRANDLEIDING_BRANDHYDRANT", Element::T)?;
    let mut out = Out::default();
    let node_result = |i: usize| sol.node(&id[i]);
    for (i, p) in net.knopen.iter().enumerate() {
        let d = ctx.to_drawing(*p);
        let text = match node_result(i) {
            Some(n) if vast[i] => format!("{} {} m", id[i], dec(n.head, 2)),
            Some(n) => format!("{} {} kPa", id[i], dec(n.pressure_kpa, 0)),
            None => continue,
        };
        out.put(&lt, [ctx.text([d[0], d[1] + 2.0 * k], &text, 1.8, 0.0, &lt, None)]);
    }
    for (j, l) in net.leidingen.iter().enumerate() {
        let Some(pr) = sol.pipe(&format!("L{}", j + 1)) else { continue };
        let pts: Vec<XY> = l.pts.iter().map(|p| ctx.to_drawing(*p)).collect();
        let Some((mid, ang)) = crate::emit::label_spot(&pts) else { continue };
        let rot = crate::emit::readable(ang);
        let off = [-rot.sin() * 1.5 * k, rot.cos() * 1.5 * k];
        let text = format!("L{} · {} l/s · {} m/s · Ø{}", j + 1, dec(pr.flow.abs() * 1000.0, 2), dec(pr.velocity.abs(), 2), dec(kn.pipes[j].diameter * 1000.0, 0));
        out.put(&lt, [ctx.text([mid[0] + off[0], mid[1] + off[1]], &text, 1.8, rot, &lt, None)]);
    }
    for (j, (n, f)) in fire.iter().enumerate() {
        let d = ctx.to_drawing(net.knopen[*n]);
        out.put(&hg, crate::emit::markers(&[d], &hg.name, 1.5 * k, None));
        let ok = if f.meets_requirement { tr("voldoet", "meets") } else { tr("onvoldoende", "insufficient") };
        let text = format!("BK{} {} m³/h ({} kPa) {}", j + 1, dec(f.available_flow * 3600.0, 0), dec(r.mindruk_kpa, 0), ok);
        out.put(&ht, [ctx.text([d[0], d[1] - 2.5 * k], &text, 1.8, 0.0, &ht, None)]);
    }
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(kind, key, _)| kind == "druknet" && key == "labels").flat_map(|(_, _, h)| h).collect();
    if !old.is_empty() {
        let _ = bridge::remove_entities(&t!("Druknet (oud)", "Pressure network (old)"), &old);
    }
    let handles = out.write_handles(tr("Druknet", "Pressure network"))?;
    bridge::index_add("druknet", "labels", &handles, true);
    ground.note_source();

    // ── Report ──
    let junctions: Vec<&kern::NodeResult> = sol.nodes.iter().filter(|n| kn.junctions.iter().any(|j| j.id == n.id)).collect();
    let low = junctions.iter().min_by(|a, b| a.pressure_kpa.total_cmp(&b.pressure_kpa)).copied();
    let high = junctions.iter().max_by(|a, b| a.pressure_kpa.total_cmp(&b.pressure_kpa)).copied();
    let fast = sol.pipes.iter().max_by(|a, b| a.velocity.abs().total_cmp(&b.velocity.abs()));
    let path = bridge::beside_drawing("druknet.md");
    let _ = std::fs::write(&path, report(&net, &kn, &sol, &fire, &id, &r));
    let nodes: Vec<Value> = net
        .knopen
        .iter()
        .enumerate()
        .filter_map(|(i, p)| node_result(i).map(|n| json!({"id": id[i], "x": p[0], "y": p[1], "maaiveld": hoogte[i], "stijghoogte": n.head, "druk_kpa": n.pressure_kpa, "vraag_ls": n.demand * 1000.0, "bron": vast[i]})))
        .collect();
    let pipes: Vec<Value> = sol
        .pipes
        .iter()
        .enumerate()
        .map(|(j, p)| json!({"id": p.id, "van": kn.pipes[j].from, "naar": kn.pipes[j].to, "lengte": kn.pipes[j].length, "diameter_mm": kn.pipes[j].diameter * 1000.0, "debiet_ls": p.flow * 1000.0, "snelheid": p.velocity, "verlies_m": p.headloss, "verlies_m_per_km": p.headloss_per_km, "element": net.leidingen[j].bron}))
        .collect();
    let kranen_json: Vec<Value> = fire
        .iter()
        .enumerate()
        .map(|(j, (n, f))| json!({"id": format!("BK{}", j + 1), "knoop": id[*n], "x": net.knopen[*n][0], "y": net.knopen[*n][1], "beschikbaar_m3h": f.available_flow * 3600.0, "druk_bij_beschikbaar_kpa": f.pressure_at_available_kpa, "nodig_m3h": f.required_flow * 3600.0, "druk_bij_nodig_kpa": f.pressure_at_required_kpa, "laagste_bij_nodig": {"knoop": f.lowest_at_required.0, "druk_kpa": f.lowest_at_required.1}, "voldoet": f.meets_requirement}))
        .collect();
    let json_path = bridge::beside_drawing("druknet.json");
    let _ = std::fs::write(&json_path, serde_json::to_string_pretty(&json!({"net": kn, "knopen": nodes, "leidingen": pipes, "brandkranen": kranen_json})).unwrap_or_default());
    api::note("knopen", nodes);
    api::note("leidingen", pipes);
    api::note("brandkranen", kranen_json);
    api::note("iteraties", sol.iterations);
    api::note("rapport", path.to_string_lossy().into_owned());
    api::note("json", json_path.to_string_lossy().into_owned());

    let mut msg = t!(
        "Druknet: {} knopen, {} leidingen, {} bron(nen); druk {} – {} kPa ({} – {}); v max {} m/s ({}).",
        "Pressure network: {} nodes, {} pipes, {} source(s); pressure {} – {} kPa ({} – {}); v max {} m/s ({}).",
        net.knopen.len(),
        net.leidingen.len(),
        bronnen.len(),
        low.map(|n| dec(n.pressure_kpa, 0)).unwrap_or_default(),
        high.map(|n| dec(n.pressure_kpa, 0)).unwrap_or_default(),
        low.map(|n| n.id.clone()).unwrap_or_default(),
        high.map(|n| n.id.clone()).unwrap_or_default(),
        fast.map(|p| dec(p.velocity.abs(), 2)).unwrap_or_default(),
        fast.map(|p| p.id.clone()).unwrap_or_default()
    );
    if !fire.is_empty() {
        let ok = fire.iter().filter(|f| f.1.meets_requirement).count();
        msg.push_str(&t!(
            " Brandkranen: {} van {} halen {} m³/h bij {} kPa.",
            " Hydrants: {} of {} reach {} m³/h at {} kPa.",
            ok,
            fire.len(),
            dec(r.blusdebiet * 3600.0, 0),
            dec(r.mindruk_kpa, 0)
        ));
    }
    if !notes.is_empty() {
        msg.push_str(&format!(" {}: {}.", tr("Let op", "Note"), notes.join("; ")));
    }
    msg.push_str(&format!(" {}: {}", tr("Rapport", "Report"), path.display()));
    Ok(msg)
}

fn report(net: &Net, kn: &Network, sol: &Solution, fire: &[(usize, FireFlowResult)], id: &[String], r: &Druknet) -> String {
    let mut s = format!("# {}\n\n", tr("Drukleidingnet", "Pressure network"));
    let friction = match r.wrijving {
        Friction::HazenWilliams { c } => format!("Hazen-Williams C = {c}"),
        Friction::DarcyWeisbach { k_mm } => format!("Darcy-Weisbach k = {k_mm} mm (Swamee-Jain)"),
    };
    s.push_str(&t!(
        "{} knopen, {} leidingen; {}; opgelost in {} iteraties (globale gradiëntmethode, Todini & Pilati 1988).\n\n",
        "{} nodes, {} pipes; {}; solved in {} iterations (global gradient method, Todini & Pilati 1988).\n\n",
        net.knopen.len(),
        net.leidingen.len(),
        friction,
        sol.iterations
    ));
    s.push_str(&format!("| {} | {} | {} | {} | {} |\n|---|---|---|---|---|\n", tr("Knoop", "Node"), tr("Maaiveld (m)", "Ground (m)"), tr("Stijghoogte (m)", "Head (m)"), tr("Druk (kPa)", "Pressure (kPa)"), tr("Vraag (l/s)", "Demand (l/s)")));
    for name in id {
        let Some(n) = sol.node(name) else { continue };
        let z = kn.junctions.iter().find(|j| &j.id == name).map(|j| nap(j.elevation, 2)).unwrap_or_else(|| "–".into());
        s.push_str(&format!("| {} | {} | {} | {} | {} |\n", name, z, nap(n.head, 2), dec(n.pressure_kpa, 0), dec(n.demand * 1000.0, 3)));
    }
    s.push_str(&format!("\n| {} | {} | {} | Ø (mm) | Q (l/s) | v (m/s) | {} |\n|---|---|---|---|---|---|---|\n", tr("Leiding", "Pipe"), tr("Van – naar", "From – to"), tr("Lengte (m)", "Length (m)"), tr("Verlies (m/km)", "Loss (m/km)")));
    for (j, p) in sol.pipes.iter().enumerate() {
        let k = &kn.pipes[j];
        s.push_str(&format!("| {} | {} – {} | {} | {} | {} | {} | {} |\n", p.id, k.from, k.to, dec(k.length, 1), dec(k.diameter * 1000.0, 0), dec(p.flow * 1000.0, 2), dec(p.velocity, 2), dec(p.headloss_per_km, 2)));
    }
    if !fire.is_empty() {
        s.push_str(&t!(
            "\n## Bluswater\n\nNodig {} m³/h bij minstens {} kPa aan de brandkraan (Brandweer Nederland, Handreiking Bluswatervoorziening en Bereikbaarheid; regionaal beleid beslist).\n\n| Brandkraan | Knoop | Beschikbaar (m³/h) | Druk bij nodig debiet (kPa) | Laagste druk elders (kPa) | |\n|---|---|---|---|---|---|\n",
            "\n## Fire flow\n\nRequired {} m³/h at no less than {} kPa at the hydrant (Brandweer Nederland, Handreiking Bluswatervoorziening en Bereikbaarheid; regional policy decides).\n\n| Hydrant | Node | Available (m³/h) | Pressure at required flow (kPa) | Lowest pressure elsewhere (kPa) | |\n|---|---|---|---|---|---|\n",
            dec(r.blusdebiet * 3600.0, 0),
            dec(r.mindruk_kpa, 0)
        ));
        for (j, (n, f)) in fire.iter().enumerate() {
            s.push_str(&format!(
                "| BK{} | {} | {} | {} | {} ({}) | {} |\n",
                j + 1,
                id[*n],
                dec(f.available_flow * 3600.0, 1),
                dec(f.pressure_at_required_kpa, 0),
                dec(f.lowest_at_required.1, 0),
                f.lowest_at_required.0,
                if f.meets_requirement { tr("voldoet", "meets") } else { tr("onvoldoende", "insufficient") }
            ));
        }
    }
    s.push_str(&format!("\n{}\n", tr("Zie docs/civiel-water.md.", "See docs/civiel-water.md.")));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} != {b} (tol {tol})");
    }

    fn lijn(pts: &[XY], d: Option<f64>) -> Lijn {
        Lijn { pts: pts.to_vec(), diameter: d, bron: String::new() }
    }

    #[test]
    fn arguments() {
        assert_eq!(parse_bronnen("100,200,25.5;0,0,20").unwrap(), vec![([100.0, 200.0], 25.5), ([0.0, 0.0], 20.0)]);
        assert!(parse_bronnen("100,200").is_err());
        assert_eq!(parse_vraag("0,5").unwrap(), Vraag::PerKnoop(0.0005));
        assert_eq!(parse_vraag("totaal:2.5").unwrap(), Vraag::Totaal(0.0025));
        assert_eq!(parse_vraag("2.5:TOTAAL").unwrap(), Vraag::Totaal(0.0025));
        assert!(parse_vraag("x:2").is_err());
        assert_eq!(parse_wrijving("140").unwrap(), Friction::HazenWilliams { c: 140.0 });
        assert_eq!(parse_wrijving("0.1").unwrap(), Friction::DarcyWeisbach { k_mm: 0.1 });
        let a = api::Args::parse("BRON=10,0,30 VRAAG=0.2 DIAMETER=110 BRANDKRAAN=50,0;60,5 MINDRUK=200 BLUSDEBIET=30");
        let d = Druknet::from_args(&a).unwrap();
        assert_eq!(d.diameter, Some(0.11));
        assert_eq!(d.brandkranen.len(), 2);
        close(d.blusdebiet, 30.0 / 3600.0, 1e-12);
        assert_eq!(d.wrijving, Friction::HazenWilliams { c: HAZEN_WILLIAMS_C });
        assert!(Druknet::from_args(&api::Args::parse("VRAAG=1")).is_err());
        assert_eq!(mm("Ø110 mm"), Some(0.11));
    }

    #[test]
    fn ends_snap_and_tees_split() {
        // A main 0→100 and a branch from (40.2, 0.3) up: the branch end lands on the main.
        let net = bouw(&[lijn(&[[0.0, 0.0], [100.0, 0.0]], Some(0.15)), lijn(&[[40.2, 0.3], [40.0, 50.0]], None), lijn(&[[100.3, 0.0], [100.0, -80.0]], Some(0.1))], SNAP).unwrap();
        assert_eq!(net.knopen.len(), 5);
        assert_eq!(net.leidingen.len(), 4);
        let total: f64 = net.leidingen.iter().map(|l| l.lengte()).sum();
        close(total, 100.0 + (0.2f64.powi(2) + 49.7f64.powi(2)).sqrt() + (0.3f64.powi(2) + 80.0f64.powi(2)).sqrt(), 1e-6);
        // The main is split at the tee into 40.2 and 59.8 m, both Ø150.
        let mut main: Vec<f64> = net.leidingen.iter().filter(|l| l.diameter == Some(0.15)).map(|l| l.lengte()).collect();
        main.sort_by(f64::total_cmp);
        assert_eq!(main.len(), 2);
        close(main[0], 40.2, 1e-9);
        close(main[1], 59.8, 1e-9);
        // Every node has a pipe.
        for i in 0..net.knopen.len() {
            assert!(net.leidingen.iter().any(|l| l.van == i || l.naar == i));
        }
    }

    #[test]
    fn rings_and_hydrants() {
        let ring = lijn(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0], [0.0, 0.0]], Some(0.1));
        let mut net = bouw(&[ring], SNAP).unwrap();
        assert_eq!((net.knopen.len(), net.leidingen.len()), (2, 2));
        assert!(net.leidingen.iter().all(|l| l.van != l.naar && (l.lengte() - 100.0).abs() < 1e-9));
        // A hydrant 3 m off the ring splits it; one near a node reuses it.
        let n = koppel(&mut net, [25.0, 3.0], KOPPEL).unwrap();
        assert_eq!(net.knopen[n], [25.0, 0.0]);
        assert_eq!(net.leidingen.len(), 3);
        let m = koppel(&mut net, [25.2, -0.2], KOPPEL).unwrap();
        assert_eq!(m, n);
        assert!(koppel(&mut net, [200.0, 200.0], KOPPEL).is_err());
    }

    #[test]
    fn demand_and_kernel_network() {
        // Source — 1000 m — K2 — 500 m — K3.
        let mut net = bouw(&[lijn(&[[0.0, 0.0], [1000.0, 0.0], [1500.0, 0.0]], None)], SNAP).unwrap();
        let mid = koppel(&mut net, [1000.0, 0.0], KOPPEL).unwrap();
        let src = koppel(&mut net, [0.0, 0.0], KOPPEL).unwrap();
        let vast: Vec<bool> = (0..net.knopen.len()).map(|i| i == src).collect();
        let q = vraag(&net, &Vraag::Totaal(0.01), &vast);
        close(q.iter().sum::<f64>(), 0.01, 1e-12);
        close(q[src], 0.0, 0.0);
        // Half lengths: the middle node 750 m, the end 250 m.
        close(q[mid], 0.0075, 1e-12);
        let per = vraag(&net, &Vraag::PerKnoop(0.001), &vast);
        assert_eq!(per.iter().filter(|v| **v > 0.0).count(), 2);
        let bronnen = vec![(src, 30.0)];
        let hz = vec![0.0; net.knopen.len()];
        let f = Friction::HazenWilliams { c: 130.0 };
        assert!(kern_net(&net, &hz, &q, &bronnen, &f, None).is_err());
        let kn = kern_net(&net, &hz, &q, &bronnen, &f, Some(0.2)).unwrap();
        assert_eq!((kn.junctions.len(), kn.reservoirs.len(), kn.pipes.len()), (2, 1, 2));
        assert_eq!(kn.reservoirs[0].id, "BRON1");
        let sol = kern::solve(&kn, &SolverOptions::default()).unwrap();
        // Hazen-Williams over the first 1000 m carrying the whole 10 l/s.
        let h1 = 10.667 * 1000.0 / (130f64.powf(1.852) * 0.2f64.powf(4.871)) * 0.01f64.powf(1.852);
        let id = ids(&net, &bronnen);
        close(sol.node(&id[mid]).unwrap().head, 30.0 - h1, 1e-4);
        let ff = kern::fire_flow(&kn, &id[mid], &FireFlowOptions::default(), &SolverOptions::default()).unwrap();
        assert!(ff.available_flow > 0.0);
    }

    #[test]
    fn library_object_data() {
        let mut d = BTreeMap::new();
        d.insert("infra_item".to_string(), "waterleiding".to_string());
        d.insert("infra_primary".to_string(), "1".to_string());
        d.insert("infra_p_0000".to_string(), r#"{"at":[10.0,20.0,0.0],"values":{"diam"#.to_string());
        d.insert("infra_p_0001".to_string(), r#"eter":110.0}}"#.to_string());
        let (item, p) = infra(&d).unwrap();
        assert_eq!(item, "waterleiding");
        assert_eq!(p["values"]["diameter"].as_f64(), Some(110.0));
        assert_eq!(p["at"][1].as_f64(), Some(20.0));
        d.insert("infra_primary".to_string(), "0".to_string());
        assert!(infra(&d).unwrap().1.is_null());
        assert!(infra(&BTreeMap::new()).is_none());
        assert!(is_pipe_item("persleiding-pe63") && !is_pipe_item("afsluiter-pe63"));
        assert_eq!(library_diameter("waterleiding", &p), Some(0.11));
        assert_eq!(library_diameter("waterleiding", &json!({"values": {}})), Some(0.075));
        assert_eq!(library_diameter("persleiding-pe63", &Value::Null), Some(0.063));
    }
}
