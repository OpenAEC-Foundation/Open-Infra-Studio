//! Sight distance (`CIV_ZICHT`): per station the available sight distance
//! against the required stopping sight distance of the design criteria.
//! The available distance is the shortest of three: over the design
//! profile (eye and object height of the guideline above the road), over
//! the terrain (the TIN, away from the road platform, cutting the inside of
//! curves) and in plan (the sight line against obstacles on chosen layers,
//! such as BGT buildings). Drawn as a graph below the project, with the
//! deficits marked there and along the axis.

use std::collections::HashSet;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use super::as_profiel::{self, Alignment, Seg};
use super::ontwerpcriteria::{self, Choice};
use super::terrein::Tin;
use super::{record, station, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::nlcs::{Element, LayerSpec};

/// Obstacle edges in a uniform grid, for sight lines in plan.
pub struct EdgeGrid {
    edges: Vec<(XY, XY)>,
    origin: XY,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

/// Whether segments a–b and c–d cross or touch.
fn crosses(a: XY, b: XY, c: XY, d: XY) -> bool {
    let orient = |p: XY, q: XY, r: XY| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (d1, d2, d3, d4) = (orient(c, d, a), orient(c, d, b), orient(a, b, c), orient(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    let on = |p: XY, q: XY, r: XY, o: f64| o.abs() < 1e-12 && r[0] >= p[0].min(q[0]) && r[0] <= p[0].max(q[0]) && r[1] >= p[1].min(q[1]) && r[1] <= p[1].max(q[1]);
    on(c, d, a, d1) || on(c, d, b, d2) || on(a, b, c, d3) || on(a, b, d, d4)
}

impl EdgeGrid {
    pub fn new(edges: Vec<(XY, XY)>, cell: f64) -> EdgeGrid {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for (p, q) in &edges {
            for r in [p, q] {
                b = [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[0]), b[3].max(r[1])];
            }
        }
        if edges.is_empty() {
            return EdgeGrid { edges, origin: [0.0, 0.0], cell, cols: 0, rows: 0, cells: Vec::new() };
        }
        let cell = cell.max(1e-6);
        let cols = (((b[2] - b[0]) / cell).floor() as usize + 1).min(4000);
        let rows = (((b[3] - b[1]) / cell).floor() as usize + 1).min(4000);
        let cell = cell.max((b[2] - b[0]) / cols as f64).max((b[3] - b[1]) / rows as f64);
        let mut cells = vec![Vec::new(); cols * rows];
        for (i, (p, q)) in edges.iter().enumerate() {
            let c0 = (((p[0].min(q[0]) - b[0]) / cell) as usize).min(cols - 1);
            let c1 = (((p[0].max(q[0]) - b[0]) / cell) as usize).min(cols - 1);
            let r0 = (((p[1].min(q[1]) - b[1]) / cell) as usize).min(rows - 1);
            let r1 = (((p[1].max(q[1]) - b[1]) / cell) as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    cells[r * cols + c].push(i as u32);
                }
            }
        }
        EdgeGrid { edges, origin: [b[0], b[1]], cell, cols, rows, cells }
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Whether the segment a–b crosses an edge: walks the cells it passes.
    pub fn blocked(&self, a: XY, b: XY) -> bool {
        if self.edges.is_empty() {
            return false;
        }
        let w = [self.cols as f64 * self.cell, self.rows as f64 * self.cell];
        let local = |p: XY| [(p[0] - self.origin[0]) / self.cell, (p[1] - self.origin[1]) / self.cell];
        let (la, lb) = (local(a), local(b));
        // Clip to the grid.
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        let d = [lb[0] - la[0], lb[1] - la[1]];
        for (k, size) in [(0, w[0] / self.cell), (1, w[1] / self.cell)] {
            if d[k].abs() < 1e-15 {
                if la[k] < 0.0 || la[k] > size {
                    return false;
                }
            } else {
                let (u0, u1) = ((0.0 - la[k]) / d[k], (size - la[k]) / d[k]);
                let (lo, hi) = (u0.min(u1), u0.max(u1));
                t0 = t0.max(lo);
                t1 = t1.min(hi);
            }
        }
        if t0 > t1 {
            return false;
        }
        let p = [la[0] + d[0] * t0, la[1] + d[1] * t0];
        let mut cx = (p[0].floor() as isize).clamp(0, self.cols as isize - 1);
        let mut cy = (p[1].floor() as isize).clamp(0, self.rows as isize - 1);
        let step = [d[0].signum() as isize, d[1].signum() as isize];
        let next = |c: isize, k: usize| if d[k] > 0.0 { (c + 1) as f64 } else { c as f64 };
        let mut tmax = [if d[0].abs() > 1e-15 { (next(cx, 0) - la[0]) / d[0] } else { f64::MAX }, if d[1].abs() > 1e-15 { (next(cy, 1) - la[1]) / d[1] } else { f64::MAX }];
        let tdelta = [if d[0].abs() > 1e-15 { 1.0 / d[0].abs() } else { f64::MAX }, if d[1].abs() > 1e-15 { 1.0 / d[1].abs() } else { f64::MAX }];
        let mut seen: HashSet<u32> = HashSet::new();
        loop {
            for &e in &self.cells[cy as usize * self.cols + cx as usize] {
                if seen.insert(e) {
                    let (p, q) = self.edges[e as usize];
                    if crosses(a, b, p, q) {
                        return true;
                    }
                }
            }
            let k = if tmax[0] < tmax[1] { 0 } else { 1 };
            if tmax[k] > t1 {
                return false;
            }
            if k == 0 {
                cx += step[0];
                if cx < 0 || cx >= self.cols as isize {
                    return false;
                }
            } else {
                cy += step[1];
                if cy < 0 || cy >= self.rows as isize {
                    return false;
                }
            }
            tmax[k] += tdelta[k];
        }
    }
}

/// What a sight line runs over.
pub struct Sight<'a> {
    pub al: &'a Alignment,
    /// Drawing units per metre.
    pub f: f64,
    /// Road surface height (m) at a distance (m) along the axis.
    pub road: &'a dyn Fn(f64) -> f64,
    /// Eye and object height above the road (m).
    pub eye: f64,
    pub object: f64,
    /// The driver's line: offset (m) to the right of the driving direction.
    pub lane: f64,
    pub tin: Option<&'a Tin>,
    /// Half the road platform (m): the terrain does not count there.
    pub clear: f64,
    pub obstacles: &'a EdgeGrid,
    /// Longest distance looked for and the step (m).
    pub max: f64,
    pub step: f64,
}

/// The available sight distance at one station (m), each kind on its own;
/// `None` where that kind did not apply (no TIN, no obstacles).
#[derive(Clone, Debug, PartialEq)]
pub struct Available {
    pub s: f64,
    pub forward: bool,
    pub profile: f64,
    pub terrain: Option<f64>,
    pub plan: Option<f64>,
    /// The axis ended before anything blocked the view.
    pub end: bool,
}

impl Available {
    pub fn min(&self) -> f64 {
        self.profile.min(self.terrain.unwrap_or(f64::MAX)).min(self.plan.unwrap_or(f64::MAX))
    }

    /// What limits the view: profiel, terrein or plattegrond.
    pub fn cause(&self) -> &'static str {
        let m = self.min();
        if self.plan.is_some_and(|p| p <= m) {
            "plattegrond"
        } else if self.terrain.is_some_and(|t| t <= m) {
            "terrein"
        } else {
            "profiel"
        }
    }
}

/// The sight distances from station `s` (m from the start of the axis),
/// looking along (`forward`) or against the stationing.
pub fn available(sg: &Sight, s: f64, forward: bool) -> Available {
    let len = sg.al.length() / sg.f;
    let dirn = if forward { 1.0 } else { -1.0 };
    // Offsets in plan: the driver's right is the axis's left when driving back.
    let lane = sg.lane * dirn;
    let plan_at = |t: f64| -> XY {
        let (i, local) = sg.al.seg_at(t * sg.f);
        sg.al.segs[i].offset_point(local, lane * sg.f)
    };
    let eye_xy = plan_at(s);
    let z_eye = (sg.road)(s) + sg.eye;
    let room = if forward { len - s } else { s };
    let (mut profile, mut terrain, mut plan) = (None::<f64>, None::<f64>, None::<f64>);
    let step = sg.step.max(0.05);
    // Steepest rise seen from the eye over the road so far (profile check):
    // a road point blocks the object when it rises more steeply.
    let mut steepest = f64::MIN;
    let sub = step.min(1.0);
    let mut sampled = 0.0;
    let mut d = step;
    let mut reached = 0.0;
    while d <= sg.max.min(room) + 1e-9 {
        while sampled + sub <= d + 1e-9 {
            sampled += sub;
            steepest = steepest.max(((sg.road)(s + dirn * sampled) - z_eye) / sampled);
        }
        let t = s + dirn * d;
        let z_obj = (sg.road)(t) + sg.object;
        if profile.is_none() && steepest > (z_obj - z_eye) / d + 1e-9 {
            profile = Some(d - step);
        }
        let obj_xy = plan_at(t);
        if terrain.is_none() {
            if let Some(tin) = sg.tin {
                let n = ((d / step).ceil() as usize).clamp(2, 40);
                for j in 1..n {
                    let w = j as f64 / n as f64;
                    let p = [eye_xy[0] + (obj_xy[0] - eye_xy[0]) * w, eye_xy[1] + (obj_xy[1] - eye_xy[1]) * w];
                    // On the road platform the profile counts, not the terrain.
                    let road_xy = sg.al.at((s + dirn * d * w) * sg.f).0;
                    if (p[0] - road_xy[0]).hypot(p[1] - road_xy[1]) <= sg.clear * sg.f {
                        continue;
                    }
                    let line = z_eye + (z_obj - z_eye) * w;
                    if tin.height(p).is_some_and(|z| z / sg.f > line + 1e-6) {
                        terrain = Some(d - step);
                        break;
                    }
                }
            }
        }
        if plan.is_none() && sg.obstacles.blocked(eye_xy, obj_xy) {
            plan = Some(d - step);
        }
        reached = d;
        if profile.is_some() && (terrain.is_some() || sg.tin.is_none()) && (plan.is_some() || sg.obstacles.is_empty()) {
            break;
        }
        d += step;
    }
    let free = reached.max(0.0);
    let end = room <= sg.max + 1e-9 && profile.is_none() && terrain.is_none() && plan.is_none();
    Available {
        s,
        forward,
        profile: profile.unwrap_or(free).max(0.0),
        terrain: sg.tin.map(|_| terrain.unwrap_or(free).max(0.0)),
        plan: (!sg.obstacles.is_empty()).then(|| plan.unwrap_or(free).max(0.0)),
        end,
    }
}

/// Stretches (m) where the available distance falls short of `need`.
pub fn deficits(rows: &[Available], need: f64) -> Vec<(f64, f64, f64, &'static str)> {
    let mut out: Vec<(f64, f64, f64, &'static str)> = Vec::new();
    let mut open: Option<(f64, f64, f64, &'static str)> = None;
    for r in rows {
        // The end of the axis is no obstruction.
        let short = r.min() < need - 1e-9 && !r.end;
        match (&mut open, short) {
            (Some(o), true) => {
                o.1 = r.s;
                if r.min() < o.2 {
                    o.2 = r.min();
                    o.3 = r.cause();
                }
            }
            (None, true) => open = Some((r.s, r.s, r.min(), r.cause())),
            (Some(_), false) => out.push(open.take().unwrap()),
            (None, false) => {}
        }
    }
    out.extend(open);
    out
}

/// Obstacle edges on layers whose name contains one of `patterns`.
fn obstacle_edges(doc: &CadDocument, patterns: &[String]) -> Vec<(XY, XY)> {
    let wanted = |layer: &str| {
        let l = layer.to_ascii_uppercase();
        patterns.iter().any(|p| l.contains(&p.to_ascii_uppercase()))
    };
    let mut out = Vec::new();
    let mut chain = |pts: &[(XY, f64)], closed: bool| {
        let mut v: Vec<(XY, f64)> = pts.to_vec();
        if closed && v.len() > 2 {
            v.push((v[0].0, 0.0));
        }
        for w in v.windows(2) {
            let seg = Seg::from_bulge(w[0].0, w[1].0, w[0].1);
            match seg {
                Seg::Line { a, b } => out.push((a, b)),
                Seg::Arc { .. } => {
                    let n = 8;
                    let l = seg.length();
                    for i in 0..n {
                        out.push((seg.at(l * i as f64 / n as f64).0, seg.at(l * (i + 1) as f64 / n as f64).0));
                    }
                }
                Seg::Spiral { .. } => {}
            }
        }
    };
    for e in doc.model_space_entities().filter(|e| wanted(&e.common().layer)) {
        match e {
            EntityType::LwPolyline(l) => chain(&l.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect::<Vec<_>>(), l.is_closed),
            EntityType::Polyline2D(p) => chain(&p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect::<Vec<_>>(), p.flags.is_closed()),
            EntityType::Polyline3D(p) => chain(&p.vertices.iter().map(|v| ([v.position.x, v.position.y], 0.0)).collect::<Vec<_>>(), p.flags.closed),
            EntityType::Line(l) => chain(&[([l.start.x, l.start.y], 0.0), ([l.end.x, l.end.y], 0.0)], false),
            _ => {}
        }
    }
    out
}

/// Options of `CIV_ZICHT`.
pub struct SightRequest {
    pub axis: Option<Handle>,
    pub choice: Option<Choice>,
    /// Overrides: required distance, eye and object height (m).
    pub need: Option<f64>,
    pub eye: Option<f64>,
    pub object: Option<f64>,
    /// Station spacing, driver's line, platform half width, longest distance (m).
    pub every: f64,
    pub lane: f64,
    pub clear: f64,
    pub max: Option<f64>,
    /// Along, against or both ways.
    pub forward: bool,
    pub backward: bool,
    /// Obstacle layer name parts.
    pub layers: Vec<String>,
    /// Lower-left corner of the graph (drawing), or found.
    pub at: Option<XY>,
}

fn nl() -> bool {
    crate::lang::current() == crate::lang::Lang::Nl
}

fn tx(nl_text: String, en_text: String) -> String {
    if nl() { nl_text } else { en_text }
}

fn comma(v: f64, d: usize) -> String {
    format!("{v:.d$}").replace('.', ",")
}

fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

fn cause_text(c: &str) -> &'static str {
    match c {
        "plattegrond" => crate::lang::tr("obstakel in plattegrond", "obstacle in plan"),
        "terrein" => crate::lang::tr("terrein", "terrain"),
        _ => crate::lang::tr("lengteprofiel", "length profile"),
    }
}

/// `CIV_ZICHT`.
pub fn sight(opt: &super::CivielOptions, req: SightRequest, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["as", "ontwerpprofiel", "tin"], &req.axis.into_iter().collect::<Vec<_>>())?;
    let (al, h, _) = as_profiel::axis(&doc, req.axis)?;
    let (f, k) = (ctx.f, ctx.k);
    let len = al.length() / f;
    // Required distance, eye and object height: the criteria, or given.
    let crit = ontwerpcriteria::choice_for(req.choice.clone(), doc.get_entity(h)).map(|c| c.resolve(&ontwerpcriteria::guidelines())).transpose()?;
    let need = req
        .need
        .or_else(|| crit.as_ref().and_then(|(_, _, l)| l.stopzicht.as_ref().map(|v| v.v)))
        .ok_or_else(|| crate::lang::tr("geen stopzicht: kies ontwerpcriteria (CIV_ONTWERPCRITERIA of RICHTLIJN= CATEGORIE= V=) of geef NODIG=<m>", "no stopping sight distance: choose design criteria (CIV_ONTWERPCRITERIA or RICHTLIJN= CATEGORIE= V=) or give NODIG=<m>").to_string())?;
    let pick = |given: Option<f64>, from: fn(&ontwerpcriteria::Guideline) -> &Option<ontwerpcriteria::Value>, default: f64| -> (f64, String) {
        match (given, crit.as_ref().and_then(|(g, _, _)| from(g).as_ref().map(|v| (v.v, g.cite(v))))) {
            (Some(v), _) => (v, crate::lang::tr("opgegeven", "given").to_string()),
            (None, Some((v, src))) => (v, src),
            (None, None) => (default, crate::lang::tr("standaard", "default").to_string()),
        }
    };
    let (eye, eye_src) = pick(req.eye, |g| &g.ooghoogte, 1.10);
    let (object, object_src) = pick(req.object, |g| &g.objecthoogte, 0.50);
    let need_src = match (&req.need, &crit) {
        (Some(_), _) => crate::lang::tr("opgegeven", "given").to_string(),
        (None, Some((g, _, l))) => l.stopzicht.as_ref().map(|v| g.cite(v)).unwrap_or_default(),
        _ => String::new(),
    };
    // The road: the design profile, else the ground along the axis.
    let pvis = super::corridor::profile_of(&doc, &al.name);
    let ground_heights: Vec<f64>;
    let surface_label: String;
    let road: Box<dyn Fn(f64) -> f64> = match &pvis {
        Some(p) => {
            surface_label = crate::lang::tr("ontwerpprofiel", "design profile").to_string();
            let p = p.clone();
            Box::new(move |s| as_profiel::design_z(&p, s))
        }
        None => {
            let mut ground = Maaiveld::find(&ctx, &doc)?;
            progress(format!("{} ({}) …", crate::lang::tr("Hoogtes langs de as", "Heights along the axis"), ground.label()));
            let n = len.ceil() as usize;
            let raw: Vec<Option<f64>> = (0..=n).map(|i| ground.height(al.at((i as f64).min(len) * f).0, f)).collect();
            let known: Vec<(usize, f64)> = raw.iter().enumerate().filter_map(|(i, z)| z.map(|z| (i, z))).collect();
            if known.is_empty() {
                return Err(ground.error().unwrap_or_else(|| crate::lang::tr("geen hoogtes langs de as", "no heights along the axis").to_string()));
            }
            ground_heights = raw.iter().enumerate().map(|(i, z)| z.unwrap_or_else(|| known.iter().min_by_key(|(j, _)| j.abs_diff(i)).unwrap().1)).collect();
            surface_label = ground.label();
            ground.note_source();
            let zs = ground_heights.clone();
            Box::new(move |s| {
                let x = s.clamp(0.0, (zs.len() - 1) as f64);
                let i = (x.floor() as usize).min(zs.len().saturating_sub(2));
                if zs.len() < 2 {
                    return zs[0];
                }
                zs[i] + (zs[i + 1] - zs[i]) * (x - i as f64)
            })
        }
    };
    let tin = Tin::of_status(&doc, 'B');
    let edges = obstacle_edges(&doc, &req.layers);
    let n_edges = edges.len();
    let grid = EdgeGrid::new(edges, 10.0 * f);
    let max = req.max.unwrap_or((need * 1.5).max(100.0)).clamp(10.0, 1500.0);
    let step = (max / 250.0).clamp(0.5, 2.0);
    let sg = Sight { al: &al, f, road: &*road, eye, object, lane: req.lane, tin: tin.as_ref(), clear: req.clear, obstacles: &grid, max, step };
    let every = req.every.max(1.0);
    let mut stations: Vec<f64> = Vec::new();
    let mut s = 0.0;
    while s <= len + 1e-9 {
        stations.push(s);
        s += every;
    }
    if stations.len() > 2000 {
        return Err(tx(format!("{} stations is te veel: kies een grotere OM=", stations.len()), format!("{} stations is too many: choose a larger OM=", stations.len())));
    }
    let mut ways = Vec::new();
    if req.forward {
        ways.push(true);
    }
    if req.backward {
        ways.push(false);
    }
    let mut rows: Vec<Vec<Available>> = Vec::new();
    for (wi, fw) in ways.iter().enumerate() {
        progress(format!("{} {}/{} …", crate::lang::tr("Zichtafstanden", "Sight distances"), wi + 1, ways.len()));
        rows.push(stations.iter().map(|s| available(&sg, *s, *fw)).collect());
    }
    // Drawing: one graph per direction, below the project.
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let zicht = ctx.layer("KW", "ZICHTRUIMTE", Element::G)?;
    let mark = ctx.layer("AL", "AANKLEDING", Element::G)?;
    let mark_t = ctx.layer("AL", "AANKLEDING", Element::T)?;
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    let name = if al.name.is_empty() { "as".to_string() } else { al.name.clone() };
    let mut out = Out::default();
    let mut all_deficits = Vec::new();
    let graph_h = 50.0 * k;
    let row = 8.0 * k;
    let head = 34.0 * k;
    let top = (max / super::nice_step(max / 5.0)).ceil() * super::nice_step(max / 5.0);
    let scale = graph_h / top;
    let mut first_at = None;
    for (rs, fw) in rows.iter().zip(&ways) {
        let at = match (req.at, first_at) {
            (Some(a), None) => a,
            (Some(a), Some(_)) => [a[0], a[1] - graph_h - row - 30.0 * k],
            _ => {
                let spot = crate::bridge::layout_spot("onder", [head + len * f + 10.0 * k, graph_h + row + 18.0 * k], as_profiel::clear_area(&ctx, &al), 30.0 * k);
                [spot[0] + head, spot[1]]
            }
        };
        first_at.get_or_insert(at);
        let gx = |s: f64| at[0] + s * f;
        let gy = |d: f64| at[1] + row + d.min(top) * scale;
        let (x0, x1, y0, y1) = (at[0] - head, gx(len), at[1], gy(top));
        let data = record(&[("zichtafstand", name.clone()), ("as", al.name.clone()), ("richting", if *fw { "heen" } else { "terug" }.to_string()), ("nodig_m", format!("{need:.1}"))]);
        out.put(&frame, crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], &frame.name, k, Some(&data)));
        out.put(&frame, pl(vec![[x0, y0 + row], [x1, y0 + row]], &frame));
        out.put(&frame, pl(vec![[at[0], y0], [at[0], y1]], &frame));
        // Distance grid with labels.
        let dstep = super::nice_step(top / 5.0);
        let mut d = 0.0;
        while d <= top + 1e-9 {
            if d > 1e-9 {
                out.put(&frame, pl(vec![[at[0], gy(d)], [x1, gy(d)]], &frame));
            }
            let t = ctx.text([at[0] - 2.0 * k, gy(d)], &format!("{d:.0}"), 1.8, 0.0, &frame_t, None);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
            d += dstep;
        }
        // Stations along the bottom.
        let lstep = super::nice_step((len / 10.0).max(every));
        let mut s = 0.0;
        while s <= len + 1e-9 {
            out.put(&frame, pl(vec![[gx(s), y0], [gx(s), y0 + row]], &frame));
            let t = ctx.text([gx(s) + 1.0 * k, y0 + row / 2.0], &station(al.start + s), 1.8, 0.0, &frame_t, None);
            out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
            s += lstep;
        }
        // Required (frame layer), each kind (thin) and the governing distance.
        out.put(&frame, pl(vec![[at[0], gy(need)], [x1, gy(need)]], &frame));
        let t = ctx.text([x1 + 2.0 * k, gy(need)], &format!("{} {} m", crate::lang::tr("nodig", "required"), comma(need, 0)), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        let kinds: [(&str, Box<dyn Fn(&Available) -> Option<f64>>); 3] = [
            (crate::lang::tr("profiel", "profile"), Box::new(|r: &Available| Some(r.profile))),
            (crate::lang::tr("terrein", "terrain"), Box::new(|r: &Available| r.terrain)),
            (crate::lang::tr("plattegrond", "plan"), Box::new(|r: &Available| r.plan)),
        ];
        for (label, get) in &kinds {
            let pts: Vec<XY> = rs.iter().filter_map(|r| get(r).map(|v| [gx(r.s), gy(v)])).collect();
            if pts.len() > 1 {
                let last = *pts.last().unwrap();
                out.put(&frame, pl(pts, &frame));
                let t = ctx.text([last[0] + 2.0 * k, last[1] + 2.5 * k], label, 1.8, 0.0, &frame_t, None);
                out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
            }
        }
        out.put(&zicht, pl(rs.iter().map(|r| [gx(r.s), gy(r.min())]).collect(), &zicht));
        // Deficits: shaded between available and required, and along the axis.
        let defs = deficits(rs, need);
        for (a, b, worst, cause) in &defs {
            let inside: Vec<&Available> = rs.iter().filter(|r| r.s >= *a - 1e-9 && r.s <= *b + 1e-9).collect();
            let mut poly: Vec<XY> = inside.iter().map(|r| [gx(r.s), gy(r.min())]).collect();
            poly.extend(inside.iter().rev().map(|r| [gx(r.s), gy(need)]));
            let dd = record(&[("zichttekort", name.clone()), ("van", format!("{:.2}", al.start + a)), ("tot", format!("{:.2}", al.start + b)), ("oorzaak", cause.to_string())]);
            out.put(&mark, crate::emit::polylines(&[(poly, true)], &mark.name, k, Some(&dd)));
            let label = format!("✗ {}–{}: {} m ({})", station(al.start + a), station(al.start + b), comma(*worst, 0), cause_text(cause));
            let t = ctx.text([gx((a + b) / 2.0), gy(need) + 3.0 * k], &label, 1.8, 0.0, &mark_t, None);
            out.put(&mark_t, [aligned(t, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
            // In plan: the stretch beside the axis, and the shortest sight line.
            let off = if *fw { 1.0 } else { -1.0 } * (req.lane + 3.0);
            let n = (((b - a) / 2.0).ceil() as usize).clamp(1, 500);
            let line: Vec<XY> = (0..=n)
                .map(|j| {
                    let s = a + (b - a) * j as f64 / n as f64;
                    let (i, local) = al.seg_at(s * f);
                    al.segs[i].offset_point(local, off * f)
                })
                .collect();
            out.put(&mark, crate::emit::polylines(&[(line, false)], &mark.name, k, Some(&dd)));
            if let Some(r) = inside.iter().min_by(|x, y| x.min().total_cmp(&y.min())) {
                let dirn = if *fw { 1.0 } else { -1.0 };
                let lane = req.lane * dirn;
                let p_at = |t: f64| {
                    let (i, local) = al.seg_at(t * f);
                    al.segs[i].offset_point(local, lane * f)
                };
                let (e, o) = (p_at(r.s), p_at((r.s + dirn * r.min()).clamp(0.0, len)));
                out.put(&zicht, pl(vec![e, o], &zicht));
                let (p, dir) = al.at((a + b) / 2.0 * f);
                let side = off.signum();
                let tat = [p[0] + dir.sin() * (off + side * 3.0) * f, p[1] - dir.cos() * (off + side * 3.0) * f];
                let t = ctx.text(tat, &label, 1.8, crate::emit::readable(dir), &mark_t, None);
                out.put(&mark_t, [aligned(t, if side > 0.0 { TextHorizontalAlignment::Left } else { TextHorizontalAlignment::Right }, TextVerticalAlignment::Middle)]);
            }
        }
        let way = if *fw { crate::lang::tr("met de stationering mee", "along the stationing") } else { crate::lang::tr("tegen de stationering in", "against the stationing") };
        let title = tx(format!("Zichtafstand {name} — {way}"), format!("Sight distance {name} — {way}"));
        let t = ctx.text([x0, y1 + 9.0 * k], &title, 3.5, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        let sub = tx(
            format!("stopzicht {} m ({need_src}) · ooghoogte {} m · objecthoogte {} m · weg: {surface_label} · rijlijn {} m", comma(need, 0), comma(eye, 2), comma(object, 2), comma(req.lane, 2)),
            format!("stopping sight {} m ({need_src}) · eye {} m · object {} m · road: {surface_label} · driver's line {} m", comma(need, 0), comma(eye, 2), comma(object, 2), comma(req.lane, 2)),
        );
        let t = ctx.text([x0, y1 + 4.0 * k], &sub, 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        all_deficits.extend(defs.into_iter().map(|d| (*fw, d)));
    }
    let handles = out.write_handles(&format!("{} {name}", crate::lang::tr("Zichtafstand", "Sight distance")))?;
    if let Some(h) = handles.first() {
        super::api::note("diagram", super::api::handle_text(*h));
    }
    if let Some(at) = first_at {
        super::api::note("plaats", serde_json::json!([at[0], at[1]]));
    }
    super::api::note("nodig_m", need);
    super::api::note("nodig_bron", need_src);
    super::api::note("ooghoogte_m", eye);
    super::api::note("ooghoogte_bron", eye_src);
    super::api::note("objecthoogte_m", object);
    super::api::note("objecthoogte_bron", object_src);
    super::api::note("weg", surface_label.clone());
    super::api::note("terrein", tin.is_some());
    super::api::note("obstakelranden", n_edges);
    super::api::note(
        "stations",
        rows.iter()
            .flatten()
            .map(|r| serde_json::json!({"station": al.start + r.s, "richting": if r.forward { "heen" } else { "terug" }, "profiel_m": r.profile, "terrein_m": r.terrain, "plattegrond_m": r.plan, "beschikbaar_m": r.min(), "einde_as": r.end}))
            .collect::<Vec<_>>(),
    );
    super::api::note(
        "tekorten",
        all_deficits
            .iter()
            .map(|(fw, (a, b, w, c))| serde_json::json!({"richting": if *fw { "heen" } else { "terug" }, "van": al.start + a, "tot": al.start + b, "minimum_m": w, "oorzaak": c}))
            .collect::<Vec<_>>(),
    );
    let n = handles.len();
    if all_deficits.is_empty() {
        return Ok(tx(
            format!("{name}: overal minstens {} m zicht ({} stations; {}) — {n} elementen.", comma(need, 0), stations.len() * ways.len(), surface_label),
            format!("{name}: at least {} m sight everywhere ({} stations; {}) — {n} entities.", comma(need, 0), stations.len() * ways.len(), surface_label),
        ));
    }
    let list: Vec<String> = all_deficits.iter().take(6).map(|(_, (a, b, w, c))| format!("{}–{} {} m ({})", station(al.start + a), station(al.start + b), comma(*w, 0), cause_text(c))).collect();
    Ok(tx(
        format!("{name}: {} tekort(en) aan zicht (nodig {} m): {} — {n} elementen.", all_deficits.len(), comma(need, 0), list.join("; ")),
        format!("{name}: {} sight deficit(s) (required {} m): {} — {n} entities.", all_deficits.len(), comma(need, 0), list.join("; ")),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_: f64) -> f64 {
        0.0
    }

    #[test]
    fn a_crest_limits_the_view() {
        // 300 m straight; a crest of 2 m over 100 m around station 150.
        let al = Alignment::from_pis(&[[0.0, 0.0], [300.0, 0.0]], &[]).unwrap();
        let pvis = as_profiel::fit_curves(&[(0.0, 0.0), (150.0, 3.0), (300.0, 0.0)], 100.0);
        let road = move |s: f64| as_profiel::design_z(&pvis, s);
        let grid = EdgeGrid::new(Vec::new(), 10.0);
        let sg = Sight { al: &al, f: 1.0, road: &road, eye: 1.1, object: 0.5, lane: 0.0, tin: None, clear: 5.0, obstacles: &grid, max: 250.0, step: 0.5 };
        let r = available(&sg, 50.0, true);
        // Over a crest of R = 100 / 0,04 = 2500 m: S = √(2R)(√h1 + √h2) ≈ 133 m when
        // the line of sight lies within the curve.
        let expect = (2.0f64 * 2500.0).sqrt() * (1.1f64.sqrt() + 0.5f64.sqrt());
        assert!(r.profile < 200.0 && r.profile > 90.0, "{r:?} vs {expect}");
        // On the flat everything is visible up to the end of the axis.
        let sg = Sight { road: &flat, ..sg };
        let r = available(&sg, 50.0, true);
        assert!(r.end && (r.profile - 250.0).abs() <= 0.5 + 1e-9, "{r:?}");
        let r = available(&sg, 280.0, true);
        assert!(r.end && r.profile <= 20.0 + 1e-9);
    }

    #[test]
    fn a_crest_curve_against_the_textbook_formula() {
        // A long crest (sight line within the curve): S = √(2R)·(√h1 + √h2).
        let al = Alignment::from_pis(&[[0.0, 0.0], [2000.0, 0.0]], &[]).unwrap();
        let r = 5000.0;
        let pvis = as_profiel::fit_curves(&[(0.0, 0.0), (1000.0, 40.0), (2000.0, 0.0)], 0.08 * r);
        let road = move |s: f64| as_profiel::design_z(&pvis, s);
        let grid = EdgeGrid::new(Vec::new(), 10.0);
        let sg = Sight { al: &al, f: 1.0, road: &road, eye: 1.1, object: 0.5, lane: 0.0, tin: None, clear: 5.0, obstacles: &grid, max: 600.0, step: 0.5 };
        let got = available(&sg, 860.0, true).profile;
        let want = (2.0f64 * r).sqrt() * (1.1f64.sqrt() + 0.5f64.sqrt());
        assert!((got - want).abs() < 2.0, "{got} vs {want}");
    }

    #[test]
    fn a_building_inside_the_curve_blocks_the_view() {
        // East 200 m, then left with R 100: a block inside the curve.
        let al = Alignment::from_pis(&[[0.0, 0.0], [200.0, 0.0], [200.0, 200.0]], &[100.0]).unwrap();
        let block = vec![([130.0, 20.0], [170.0, 20.0]), ([170.0, 20.0], [170.0, 60.0]), ([170.0, 60.0], [130.0, 60.0]), ([130.0, 60.0], [130.0, 20.0])];
        let grid = EdgeGrid::new(block, 10.0);
        let sg = Sight { al: &al, f: 1.0, road: &flat, eye: 1.1, object: 0.5, lane: 0.0, tin: None, clear: 5.0, obstacles: &grid, max: 200.0, step: 0.5 };
        let r = available(&sg, 60.0, true);
        let p = r.plan.unwrap();
        assert!(p > 100.0 && p < 140.0, "{r:?}");
        assert_eq!(r.cause(), "plattegrond");
        // Without the block: free.
        let none = EdgeGrid::new(Vec::new(), 10.0);
        let sg = Sight { obstacles: &none, ..sg };
        assert!(available(&sg, 60.0, true).plan.is_none());
        // Deficits group neighbouring stations.
        let rows: Vec<Available> = [(0.0, 150.0), (10.0, 80.0), (20.0, 60.0), (30.0, 150.0), (40.0, 90.0)]
            .iter()
            .map(|(s, d)| Available { s: *s, forward: true, profile: *d, terrain: None, plan: None, end: false })
            .collect();
        let d = deficits(&rows, 100.0);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].0, d[0].1, d[0].2), (10.0, 20.0, 60.0));
        assert_eq!((d[1].0, d[1].1), (40.0, 40.0));
    }

    #[test]
    fn grid_walk_finds_crossings() {
        let g = EdgeGrid::new(vec![([5.0, -5.0], [5.0, 5.0]), ([50.0, 40.0], [60.0, 40.0])], 3.0);
        assert!(g.blocked([0.0, 0.0], [10.0, 0.0]));
        assert!(!g.blocked([0.0, 0.0], [4.0, 0.0]));
        assert!(g.blocked([55.0, 0.0], [55.0, 100.0]));
        assert!(g.blocked([0.0, 30.0], [100.0, 50.0]));
        assert!(!g.blocked([0.0, 30.0], [100.0, 30.0]));
        assert!(!g.blocked([-10.0, -10.0], [-5.0, 100.0]));
    }
}
