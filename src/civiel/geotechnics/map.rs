//! `CIV_ZETTING`: the settlement of a fill (the design surface above the
//! existing ground, or a given height over an area) on the soil model, at
//! the points of a grid: a map in colour bands, contours, the values per
//! point and the settlement against time for one point (drawn and in JSON).

use ocs_plugin_api::host::codec::entities::Solid;
use ocs_plugin_api::host::codec::{Color, EntityType};

use super::model::{self, Subsoil};
use super::settlement::{self, Method, Rect, DAY, YEAR};
use super::{coloured, disclaimer, num, side_text, tf, SoilArgs};
use crate::civiel::terrein::Tin;
use crate::civiel::{api, nice_step, record, v3, Ctx, Out};
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::{Element, LayerSpec};

/// The fill.
#[derive(Clone, Debug)]
pub enum Fill {
    /// A polygon (project coordinates) with a height (m).
    Area(Vec<XY>, f64),
    /// The design surface (N) above the existing ground, optionally inside a polygon.
    Design(Option<Vec<XY>>),
}

#[derive(Clone, Copy, Debug)]
pub enum Water {
    Level(f64),
    /// Depth below the existing ground (m).
    Depth(f64),
}

pub struct Request {
    pub fill: Fill,
    /// Unit weight of the fill (kN/m³).
    pub gamma: f64,
    /// End time (years).
    pub years: f64,
    pub cell: Option<f64>,
    /// The point of the time curve (project coordinates); `None`: the largest settlement.
    pub point: Option<XY>,
    pub method: Method,
    pub water: Water,
    pub soil: SoilArgs,
    /// Margin around the fill (m).
    pub margin: f64,
    pub bands: usize,
}

fn inside(poly: &[XY], p: XY) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// Settlement band colours, small to large (colour indices).
const RAMP: [u8; 9] = [130, 110, 90, 70, 50, 40, 30, 20, 10];

fn tool_layer(name: &str, aci: u8) -> LayerSpec {
    LayerSpec { name: name.to_string(), color: aci, rgb: crate::nlcs::color_rgb(aci), true_color: None, lineweight: None, linetype: "CONTINUOUS".into(), object_id: 0 }
}

fn square(c: XY, h: f64, layer: &str) -> EntityType {
    let mut e = EntityType::Solid(Solid::new(v3([c[0] - h, c[1] - h], 0.0), v3([c[0] + h, c[1] - h], 0.0), v3([c[0] - h, c[1] + h], 0.0), v3([c[0] + h, c[1] + h], 0.0)));
    let cm = e.common_mut();
    cm.layer = layer.to_string();
    cm.color = Color::ByLayer;
    e
}

/// Contour lines of a grid of values at a level (marching squares), joined
/// into polylines; grid cell (i, j) centre = origin + (i, j)·cell. The
/// crossings are keyed by the grid edge they lie on, so the joining never
/// depends on rounding.
fn contours(vals: &[Vec<Option<f64>>], level: f64, origin: XY, cell: f64) -> Vec<Vec<XY>> {
    use std::collections::HashMap;
    // A hair above the level, so no grid value lies exactly on it.
    let level = level + 1e-9 * (1.0 + level.abs());
    let ny = vals.len();
    let nx = vals.first().map(|r| r.len()).unwrap_or(0);
    let p = |i: usize, j: usize| [origin[0] + i as f64 * cell, origin[1] + j as f64 * cell];
    // An edge: its lower-left vertex and whether it runs along x.
    type Edge = (usize, usize, bool);
    let mut segs: Vec<((Edge, XY), (Edge, XY))> = Vec::new();
    for j in 0..ny.saturating_sub(1) {
        for i in 0..nx.saturating_sub(1) {
            let (Some(a), Some(b), Some(c), Some(d)) = (vals[j][i], vals[j][i + 1], vals[j + 1][i + 1], vals[j + 1][i]) else { continue };
            let corners = [((i, j), a), ((i + 1, j), b), ((i + 1, j + 1), c), ((i, j + 1), d)];
            let mut cross: Vec<(Edge, XY)> = Vec::new();
            for e in 0..4 {
                let ((qa, va), (qb, vb)) = (corners[e], corners[(e + 1) % 4]);
                if (va < level) != (vb < level) {
                    let t = (level - va) / (vb - va);
                    let (pa, pb) = (p(qa.0, qa.1), p(qb.0, qb.1));
                    let lo = if (qa.1, qa.0) <= (qb.1, qb.0) { qa } else { qb };
                    cross.push(((lo.0, lo.1, qa.1 == qb.1), [pa[0] + t * (pb[0] - pa[0]), pa[1] + t * (pb[1] - pa[1])]));
                }
            }
            if cross.len() == 2 {
                segs.push((cross[0], cross[1]));
            } else if cross.len() == 4 {
                segs.push((cross[0], cross[1]));
                segs.push((cross[2], cross[3]));
            }
        }
    }
    let mut at: HashMap<Edge, Vec<usize>> = HashMap::new();
    for (k, (a, b)) in segs.iter().enumerate() {
        at.entry(a.0).or_default().push(k);
        at.entry(b.0).or_default().push(k);
    }
    let mut used = vec![false; segs.len()];
    let mut lines = Vec::new();
    for first in 0..segs.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let mut line: std::collections::VecDeque<(Edge, XY)> = [segs[first].0, segs[first].1].into_iter().collect();
        for forward in [true, false] {
            loop {
                let end = if forward { line.back() } else { line.front() }.map(|e| e.0).unwrap();
                let Some(&k) = at.get(&end).and_then(|v| v.iter().find(|k| !used[**k])) else { break };
                used[k] = true;
                let (a, b) = segs[k];
                let next = if a.0 == end { b } else { a };
                if forward { line.push_back(next) } else { line.push_front(next) }
            }
        }
        lines.push(line.into_iter().map(|e| e.1).collect());
    }
    lines
}

pub fn settlement_command(opt: &crate::civiel::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = crate::civiel::document(&["tin"], &[])?;
    let existing = Tin::of_status(&doc, 'B');
    let mut bron = if existing.is_none() { Some(crate::civiel::hoogtebron(&ctx)?) } else { None };
    let design = match &req.fill {
        Fill::Design(_) => Some(Tin::of_status(&doc, 'N').ok_or_else(|| tr("geen ontwerpoppervlak (N-…-OG-TRIANGULATIE): maak eerst een ontwerp of geef GEBIED= HOOGTE=", "no design surface (N-…-OG-TRIANGULATIE): make a design first or give GEBIED= HOOGTE=").to_string())?),
        Fill::Area(..) => None,
    };
    // The ground (m) at a point in project coordinates.
    let mut ground_at = |p: XY| -> Option<f64> {
        let d = ctx.to_drawing(p);
        match &existing {
            Some(t) => t.height(d).map(|z| z / f),
            None => bron.as_mut().and_then(|b| b.height(d)),
        }
    };
    // The extent of the load (project coordinates).
    let poly = match &req.fill {
        Fill::Area(p, _) => Some(p.clone()),
        Fill::Design(p) => p.clone(),
    };
    let ext = match (&poly, &design) {
        (Some(p), _) => p.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]),
        (None, Some(t)) => {
            let e = t.extent();
            let (a, b) = (ctx.from_drawing([e[0], e[1]]), ctx.from_drawing([e[2], e[3]]));
            [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
        }
        (None, None) => return Err("GEBIED=".into()),
    };
    let size = (ext[2] - ext[0]).max(ext[3] - ext[1]);
    let mut cell = req.cell.unwrap_or_else(|| nice_step((size / 30.0).max(1.0))).max(0.5);
    let margin = req.margin.max(0.0);
    let grid_n = |c: f64| (((ext[2] - ext[0] + 2.0 * margin) / c).ceil() as usize + 1, ((ext[3] - ext[1] + 2.0 * margin) / c).ceil() as usize + 1);
    while grid_n(cell).0 * grid_n(cell).1 > 3600 {
        cell = nice_step(cell * 1.5);
    }
    let (nx, ny) = grid_n(cell);
    let origin = [ext[0] - margin, ext[1] - margin];
    let at = |i: usize, j: usize| [origin[0] + i as f64 * cell, origin[1] + j as f64 * cell];
    // The fill thickness per cell: the mean of 3 × 3 samples.
    progress(tr("Ophoging per cel…", "Fill per cell…").into());
    let mut rects = Vec::new();
    let mut thick = vec![vec![0.0f64; nx]; ny];
    let mut grounds = vec![vec![None; nx]; ny];
    let (mut volume, mut loaded) = (0.0, 0.0);
    for j in 0..ny {
        for i in 0..nx {
            let c = at(i, j);
            grounds[j][i] = ground_at(c);
            let mut sum = 0.0;
            for a in 0..3 {
                for b in 0..3 {
                    let p = [c[0] + (a as f64 - 1.0) * cell / 3.0, c[1] + (b as f64 - 1.0) * cell / 3.0];
                    if poly.as_ref().is_some_and(|q| !inside(q, p)) {
                        continue;
                    }
                    sum += match (&req.fill, &design) {
                        (Fill::Area(_, h), _) => *h,
                        (Fill::Design(_), Some(t)) => match (t.height(ctx.to_drawing(p)), ground_at(p)) {
                            (Some(zd), Some(zg)) => (zd / f - zg).max(0.0),
                            _ => 0.0,
                        },
                        _ => 0.0,
                    };
                }
            }
            let h = sum / 9.0;
            thick[j][i] = h;
            if h > 1e-3 {
                rects.push(Rect { x0: c[0] - cell / 2.0, y0: c[1] - cell / 2.0, x1: c[0] + cell / 2.0, y1: c[1] + cell / 2.0, q: req.gamma * h });
                volume += h * cell * cell;
                loaded += cell * cell;
            }
        }
    }
    if rects.is_empty() {
        return Err(tr("geen ophoging: het ontwerp ligt nergens boven het bestaande maaiveld", "no fill: the design lies nowhere above the existing ground").into());
    }
    progress(tr("Grondopbouw…", "Soil…").into());
    let outline: Vec<XY> = match &poly {
        Some(p) => p.clone(),
        None => vec![[ext[0], ext[1]], [ext[2], ext[1]], [ext[2], ext[3]], [ext[0], ext[3]], [ext[0], ext[1]]],
    };
    let sub = super::subsoil(&ctx, &req.soil, &outline, false, progress)?;
    let t_end = req.years.max(0.01) * YEAR;
    // Every point, on a few threads.
    progress(tf!("Zetting in {} punten…", "Settlement at {} points…", nx * ny));
    let pts: Vec<(usize, usize)> = (0..ny).flat_map(|j| (0..nx).map(move |i| (i, j))).collect();
    let results: Vec<Option<(f64, f64)>> = {
        let slots: Vec<std::sync::Mutex<Option<(f64, f64)>>> = pts.iter().map(|_| std::sync::Mutex::new(None)).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| loop {
                    let n = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(&(i, j)) = pts.get(n) else { break };
                    let Some(zg) = grounds[j][i] else { continue };
                    let p = at(i, j);
                    *slots[n].lock().unwrap() = point(&sub, &rects, p, zg, req.water, req.method, t_end).map(|r| (r.primary, r.total));
                });
            }
        });
        slots.into_iter().map(|m| m.into_inner().unwrap()).collect()
    };
    let total: Vec<Vec<Option<f64>>> = (0..ny).map(|j| (0..nx).map(|i| results[j * nx + i].map(|r| r.1)).collect()).collect();
    let max = total.iter().flatten().flatten().fold(0.0f64, |a, b| a.max(*b));
    if max <= 0.0 {
        return Err(tr("geen zetting berekend (geen grond onder de ophoging?)", "no settlement computed (no soil under the fill?)").into());
    }
    // The point of the time curve.
    let (ci, cj) = match req.point {
        Some(p) => (((p[0] - origin[0]) / cell).round().clamp(0.0, (nx - 1) as f64) as usize, ((p[1] - origin[1]) / cell).round().clamp(0.0, (ny - 1) as f64) as usize),
        None => {
            let mut best = (0, 0, f64::MIN);
            for (j, row) in total.iter().enumerate() {
                for (i, v) in row.iter().enumerate() {
                    if let Some(v) = v {
                        if *v > best.2 {
                            best = (i, j, *v);
                        }
                    }
                }
            }
            (best.0, best.1)
        }
    };
    let cp = at(ci, cj);
    let zg = grounds[cj][ci].ok_or_else(|| tr("geen maaiveldhoogte in het gekozen punt", "no ground level at the chosen point").to_string())?;
    let detail = point(&sub, &rects, cp, zg, req.water, req.method, t_end).ok_or("point")?;
    let bands = model::clip_to_surface(&sub.at(cp), zg);
    let water = match req.water {
        Water::Level(z) => z,
        Water::Depth(d) => zg - d,
    };
    let lookup = |c: &str| sub.params(c);
    let col = settlement::Column { bands: &bands, water, params: &lookup };
    let ds = |depth: f64| settlement::stress(&rects, cp[0], cp[1], depth);
    let curve = settlement::curve(&col, &ds, req.method, t_end, 25);
    // ── Drawing ────────────────────────────────────────────────────────────
    progress(tr("Zettingskaart tekenen…", "Drawing the settlement map…").into());
    let n_bands = req.bands.clamp(2, RAMP.len());
    let step = nice_step((max / n_bands as f64).max(0.001));
    let n_bands = ((max / step).ceil() as usize).clamp(1, RAMP.len());
    let band_layers: Vec<LayerSpec> = (0..n_bands).map(|i| tool_layer(&format!("OI-ZETTING-{:02}", i + 1), RAMP[if n_bands == 1 { 0 } else { i * (RAMP.len() - 1) / (n_bands - 1) }])).collect();
    let g_l = ctx.layer("GW", "HOOGTE_ZETTING", Element::G)?;
    let t_l = ctx.layer("GW", "HOOGTE_ZETTING", Element::T)?;
    let fill_l = ctx.layer("GW", "OPHOGING-WERK", Element::G)?;
    let mut out = Out::default();
    let half = cell * f / 2.0;
    let mut counts = vec![0usize; n_bands];
    for (j, row) in total.iter().enumerate() {
        for (i, v) in row.iter().enumerate() {
            let Some(v) = v else { continue };
            if *v < 0.0005 {
                continue;
            }
            let b = ((v / step).floor() as usize).min(n_bands - 1);
            counts[b] += 1;
            out.put(&band_layers[b], [square(ctx.to_drawing(at(i, j)), half, &band_layers[b].name)]);
        }
    }
    let data = record(&[("zetting", tr("zettingskaart", "settlement map").into()), ("methode", format!("{:?}", req.method)), ("tijd_jaar", format!("{}", req.years)), ("gamma_kn_m3", format!("{}", req.gamma))]);
    out.put(&fill_l, [coloured(outline.iter().map(|p| ctx.to_drawing(*p)).collect(), true, &fill_l, k, None, Some(&data))]);
    // Contours at the band edges, labelled in mm.
    let mut contour_count = 0;
    for b in 1..n_bands {
        let level = b as f64 * step;
        for line in contours(&total, level, origin, cell) {
            if line.len() < 2 {
                continue;
            }
            let pts: Vec<XY> = line.iter().map(|p| ctx.to_drawing(*p)).collect();
            let mid = pts[pts.len() / 2];
            out.put(&g_l, [coloured(pts, false, &g_l, k, None, None)]);
            out.put(&t_l, [ctx.text(mid, &format!("{} mm", num(level * 1000.0, 0)), 1.8, 0.0, &t_l, None)]);
            contour_count += 1;
        }
    }
    // Values per point, thinned so texts stay 12 mm apart on paper.
    let every = ((12.0 * k / (cell * f)).ceil() as usize).max(1);
    let mut values = Vec::new();
    for (j, row) in total.iter().enumerate() {
        for (i, v) in row.iter().enumerate() {
            let p = at(i, j);
            values.push(serde_json::json!({"x": p[0], "y": p[1], "ground": grounds[j][i], "fill_m": thick[j][i], "primary_m": results[j * nx + i].map(|r| r.0), "settlement_m": v}));
            if let Some(v) = v {
                if i % every == 0 && j % every == 0 && *v >= 0.0005 {
                    out.put(&t_l, [ctx.text(ctx.to_drawing(p), &num(v * 1000.0, 0), 1.5, 0.0, &t_l, None)]);
                }
            }
        }
    }
    // The chosen point.
    let pd = ctx.to_drawing(cp);
    out.put(&g_l, crate::emit::markers(&[pd], &g_l.name, 1.5 * k, None));
    // Legend and the time graph to the right of the map.
    let right = ctx.to_drawing([ext[2] + margin, ext[3] + margin]);
    let (lx, mut ly) = (right[0] + 12.0 * k, right[1]);
    out.put(&t_l, [side_text(&ctx, [lx, ly], &tf!("Zetting na {} jaar ({:?})", "Settlement after {} years ({:?})", num(req.years, 0), req.method), 2.5, &t_l, false)]);
    for b in (0..n_bands).rev() {
        ly -= 6.0 * k;
        let l = &band_layers[b];
        out.put(l, [square([lx + 3.0 * k, ly], 2.0 * k, &l.name)]);
        let label = if b + 1 == n_bands { format!("> {} mm", num(b as f64 * step * 1000.0, 0)) } else { format!("{} – {} mm", num(b as f64 * step * 1000.0, 0), num((b + 1) as f64 * step * 1000.0, 0)) };
        out.put(l, [side_text(&ctx, [lx + 7.0 * k, ly], &format!("{label} · {} {}", counts[b], tr("punten", "points")), 1.8, l, false)]);
    }
    ly -= 8.0 * k;
    for line in [
        tf!("Ophoging γ = {} kN/m³, {} m³ over {} m²", "Fill γ = {} kN/m³, {} m³ over {} m²", num(req.gamma, 1), num(volume, 0), num(loaded, 0)),
        tf!("Spanning: Boussinesq (Newmark), raster {} m", "Stress: Boussinesq (Newmark), grid {} m", num(cell, 1)),
        sub.describe(),
        disclaimer().to_string(),
    ] {
        out.put(&t_l, [side_text(&ctx, [lx, ly], &line, 1.8, &t_l, false)]);
        ly -= 4.5 * k;
    }
    // Settlement against log time.
    let (gw, gh) = (90.0 * k, 50.0 * k);
    let (g0x, g0y) = (lx + 8.0 * k, ly - 8.0 * k - gh);
    let days_end = (t_end / DAY).max(2.0);
    let smax = nice_step(curve.iter().map(|c| c.1).fold(0.0f64, f64::max).max(0.001) / 4.0) * 4.0;
    let tx = |d: f64| g0x + d.max(1.0).log10() / days_end.log10() * gw;
    let ty = |s: f64| g0y + gh - s / smax * gh;
    out.put(&g_l, [coloured(vec![[g0x, g0y + gh], [g0x + gw, g0y + gh], [g0x + gw, g0y], [g0x, g0y], [g0x, g0y + gh]], false, &g_l, k, None, None)]);
    for (d, lab) in [(1.0, tr("1 d", "1 d")), (10.0, "10 d"), (100.0, "100 d"), (365.25, tr("1 jr", "1 yr")), (3652.5, tr("10 jr", "10 yr")), (36525.0, tr("100 jr", "100 yr"))] {
        if d <= days_end * 1.0001 {
            out.put(&g_l, [coloured(vec![[tx(d), g0y], [tx(d), g0y + gh]], false, &g_l, k, Some((200, 200, 200)), None)]);
            out.put(&t_l, [ctx.text([tx(d), g0y - 3.0 * k], lab, 1.5, 0.0, &t_l, None)]);
        }
    }
    for i in 0..=4 {
        let s = smax * i as f64 / 4.0;
        out.put(&t_l, [side_text(&ctx, [g0x - 1.5 * k, ty(s)], &format!("{} mm", num(s * 1000.0, 0)), 1.5, &t_l, true)]);
    }
    out.put(&g_l, [coloured(curve.iter().map(|(d, s)| [tx(*d), ty(*s)]).collect(), false, &g_l, k, Some((200, 30, 30)), None)]);
    out.put(&t_l, [side_text(&ctx, [g0x, g0y + gh + 4.0 * k], &tf!("Zetting – tijd in {}, {}", "Settlement – time at {}, {}", format!("{:.0}, {:.0}", cp[0], cp[1]), format!("{} mm", num(detail.total * 1000.0, 0))), 2.0, &t_l, false)]);
    let handles = out.write_handles(tr("Zettingskaart", "Settlement map"))?;
    // ── Report ─────────────────────────────────────────────────────────────
    api::note("method", format!("{:?}", req.method));
    api::note("years", req.years);
    api::note("gamma_fill", req.gamma);
    api::note("cell_m", cell);
    api::note("points", nx * ny);
    api::note("max_settlement_m", max);
    api::note("fill_volume_m3", volume);
    api::note("fill_area_m2", loaded);
    api::note("band_step_m", step);
    api::note("contours", contour_count);
    api::note("values", values);
    api::note("point", serde_json::json!({"x": cp[0], "y": cp[1], "ground": zg, "water": water, "primary_m": detail.primary, "total_m": detail.total, "layers": detail.layers, "packages": detail.packages}));
    api::note("curve", curve.iter().map(|(d, s)| serde_json::json!({"days": d, "settlement_m": s})).collect::<Vec<_>>());
    api::note("soil", sub.describe());
    api::note("units", serde_json::to_value(sub.units()).unwrap_or_default());
    let lo = ctx.to_drawing([ext[0] - margin, ext[1] - margin]);
    api::note("extent", serde_json::json!([lo[0], g0y.min(lo[1]), (g0x + gw).max(lx + 110.0 * k), right[1] + 6.0 * k]));
    api::note("map_extent", serde_json::json!([lo[0], lo[1], right[0], right[1]]));
    api::note("graph_extent", serde_json::json!([g0x - 15.0 * k, g0y - 6.0 * k, g0x + gw + 4.0 * k, g0y + gh + 8.0 * k]));
    api::note("handles", handles.len());
    api::note("disclaimer", disclaimer());
    let t90 = detail.packages.iter().map(|p| p.t90).fold(0.0f64, f64::max);
    Ok(format!(
        "{}: {} {} mm, {} {} mm ({:.0}, {:.0}), t90 {} {} — {} {}, {} {}. {}",
        tf!("Zetting na {} jaar", "Settlement after {} years", num(req.years, 0)),
        tr("maximaal", "maximum"),
        num(max * 1000.0, 0),
        tr("in het punt", "at the point"),
        num(detail.total * 1000.0, 0),
        cp[0],
        cp[1],
        num(t90 / 365.25, 1),
        tr("jaar", "years"),
        nx * ny,
        tr("punten", "points"),
        handles.len(),
        tr("elementen", "entities"),
        disclaimer()
    ))
}

/// The settlement at one point (project coordinates, ground level zg).
fn point(sub: &Subsoil, rects: &[Rect], p: XY, zg: f64, water: Water, method: Method, t_end: f64) -> Option<settlement::PointResult> {
    let bands = model::clip_to_surface(&sub.at(p), zg);
    if bands.is_empty() {
        return None;
    }
    let w = match water {
        Water::Level(z) => z,
        Water::Depth(d) => zg - d,
    };
    let lookup = |c: &str| sub.params(c);
    let col = settlement::Column { bands: &bands, water: w, params: &lookup };
    let ds = |depth: f64| settlement::stress(rects, p[0], p[1], depth);
    Some(settlement::settlement(&col, &ds, method, t_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contours_of_a_cone() {
        let n = 21;
        let vals: Vec<Vec<Option<f64>>> = (0..n).map(|j| (0..n).map(|i| Some(10.0 - ((i as f64 - 10.0).hypot(j as f64 - 10.0)))).collect()).collect();
        let lines = contours(&vals, 5.0, [0.0, 0.0], 1.0);
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert!(l.iter().all(|p| ((p[0] - 10.0).hypot(p[1] - 10.0) - 5.0).abs() < 0.2));
        assert!(inside(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], [5.0, 5.0]));
        assert!(!inside(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], [15.0, 5.0]));
    }
}
