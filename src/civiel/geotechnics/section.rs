//! `CIV_STABILITEIT`: the stability of a slope in a cross section (along a
//! drawn line, or square to an axis at a station) on the ground or design
//! surface and the soil model, with the critical circle, its slices and a
//! JSON report.

use std::collections::BTreeMap;

use ocs_plugin_api::host::codec::{CadDocument, Handle};

use super::model::{self, Subsoil};
use super::stability::{self, Load, Method, SBand, Search, Section};
use super::{band_polygons, coloured, disclaimer, hatch, num, side_text, soil, tf, SoilArgs};
use crate::civiel::as_profiel;
use crate::civiel::terrein::Tin;
use crate::civiel::{api, nice_step, record, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::Element;

/// Where the section lies.
#[derive(Clone, Debug)]
pub enum Place {
    /// Along a line in plan (by handle; `None`: the selection).
    Line(Option<Handle>),
    /// Square to an axis at a station (m), from `left` to `right` m.
    Axis { axis: Option<Handle>, station: f64, left: f64, right: f64 },
}

#[derive(Clone, Debug)]
pub enum Phreatic {
    /// The lowest ground level of the section (the polder ditch level).
    Auto,
    Level(f64),
    /// (x along the section, level) pairs.
    Points(Vec<(f64, f64)>),
}

/// Which surface the section follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    /// The design (N) where there is one, the existing ground elsewhere.
    Auto,
    Existing,
    Design,
}

pub struct Request {
    pub place: Place,
    pub phreatic: Phreatic,
    pub load: Option<Load>,
    pub method: Method,
    pub surface: Surface,
    pub soil: SoilArgs,
    pub min_depth: f64,
    pub at: Option<XY>,
    pub name: Option<String>,
}

/// The section's plan points: (x along the section, drawing point), every
/// `step` m.
fn plan_points(ctx: &Ctx, doc: &CadDocument, place: &Place, step: f64) -> Result<(Vec<(f64, XY)>, String), String> {
    let f = ctx.f;
    match place {
        Place::Line(h) => {
            let (al, _, _) = as_profiel::axis(doc, *h)?;
            let len = al.length();
            let n = ((len / f / step).ceil() as usize).clamp(8, 2000);
            Ok(((0..=n).map(|i| (len / f * i as f64 / n as f64, al.at(len * i as f64 / n as f64).0)).collect(), if al.name.is_empty() { tr("lijn", "line").into() } else { al.name.clone() }))
        }
        Place::Axis { axis, station, left, right } => {
            let (al, _, _) = as_profiel::axis(doc, *axis)?;
            let s = ((station - al.start) * f).clamp(0.0, al.length());
            let (p, dir) = al.at(s);
            let rn = [dir.sin(), -dir.cos()];
            let n = (((left + right) / step).ceil() as usize).clamp(8, 2000);
            let pts = (0..=n)
                .map(|i| {
                    let d = -left + (left + right) * i as f64 / n as f64;
                    (d, [p[0] + rn[0] * d * f, p[1] + rn[1] * d * f])
                })
                .collect();
            Ok((pts, format!("{} {}", if al.name.is_empty() { tr("as", "axis") } else { al.name.as_str() }, crate::civiel::station(*station))))
        }
    }
}

pub fn stability_command(opt: &crate::civiel::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let extra: Vec<Handle> = match &req.place {
        Place::Line(h) => h.iter().copied().collect(),
        Place::Axis { axis, .. } => axis.iter().copied().collect(),
    };
    let doc = crate::civiel::document(&["tin", "as"], &extra)?;
    let (plan, where_) = plan_points(&ctx, &doc, &req.place, 0.25)?;
    // The surface: design over existing (or one of them), else AHN.
    let design = if req.surface == Surface::Existing { None } else { Tin::of_status(&doc, 'N') };
    if req.surface == Surface::Design && design.is_none() {
        return Err(tr("geen ontwerpoppervlak (N-…-OG-TRIANGULATIE) in de tekening", "no design surface (N-…-OG-TRIANGULATIE) in the drawing").into());
    }
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let mut surface = Vec::with_capacity(plan.len());
    let mut on_design = 0usize;
    for (x, p) in &plan {
        let d = design.as_ref().and_then(|t| t.height(*p)).map(|z| z / f);
        if d.is_some() {
            on_design += 1;
        }
        if let Some(z) = d.or_else(|| ground.height(*p, f)) {
            surface.push([*x, z]);
        }
    }
    if surface.len() < plan.len() / 2 || surface.len() < 4 {
        return Err(ground.error().unwrap_or_else(|| tr("geen hoogtes langs de doorsnede", "no heights along the section").into()));
    }
    let project: Vec<XY> = plan.iter().map(|(_, p)| ctx.from_drawing(*p)).collect();
    progress(tr("Grondopbouw van de doorsnede…", "Soil of the section…").into());
    let sub = super::subsoil(&ctx, &req.soil, &project, true, progress)?;
    // Soil columns every 0.5 m, clipped to the surface.
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut units: Vec<soil::Params> = Vec::new();
    let mut columns: Vec<(f64, Vec<SBand>)> = Vec::new();
    let mut samples: Vec<(f64, Vec<model::Band>)> = Vec::new();
    let surf_at = |x: f64| -> f64 { interp(&surface, x) };
    for (i, (x, _)) in plan.iter().enumerate() {
        if i % 2 != 0 && i + 1 != plan.len() {
            continue;
        }
        let bands = model::clip_to_surface(&sub.at(project[i]), surf_at(*x));
        let mut col = Vec::new();
        for b in &bands {
            let u = *index.entry(b.code.clone()).or_insert_with(|| {
                units.push(sub.params(&b.code));
                units.len() - 1
            });
            col.push(SBand { top: b.top, bottom: b.bottom, unit: u });
        }
        columns.push((*x, col));
        samples.push((*x, bands));
    }
    if units.is_empty() {
        return Err(tr("geen grond onder de doorsnede", "no soil under the section").into());
    }
    let (zmin, zmax) = surface.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    let phreatic: Vec<[f64; 2]> = match &req.phreatic {
        Phreatic::Auto => vec![[surface[0][0], zmin], [surface[surface.len() - 1][0], zmin]],
        Phreatic::Level(z) => vec![[surface[0][0], *z], [surface[surface.len() - 1][0], *z]],
        Phreatic::Points(p) => {
            let mut v: Vec<[f64; 2]> = p.iter().map(|(x, z)| [*x, *z]).collect();
            v.sort_by(|a, b| a[0].total_cmp(&b[0]));
            v
        }
    };
    let sec = Section { surface: surface.clone(), columns, units: units.clone(), phreatic: phreatic.clone(), load: req.load, base: sub.base() };
    progress(tf!("Kritieke cirkel zoeken ({})…", "Searching the critical circle ({})…", req.method.label()));
    let search = Search { method: req.method, min_depth: req.min_depth, ..Search::default() };
    let main = stability::search(&sec, &search).ok_or_else(|| tr("geen geldige glijcirkel gevonden (doorsnede te kort of te vlak?)", "no valid slip circle found (section too short or too flat?)").to_string())?;
    let other_method = if req.method == Method::Bishop { Method::Fellenius } else { Method::Bishop };
    let other = stability::search(&sec, &Search { method: other_method, ..search });
    let crit = main.critical.clone();
    let cut = stability::slices(&sec, &crit.circle, search.slices).ok_or("slices")?;
    let other_same = stability::factor(&cut.slices, other_method).map(|r| r.0);
    // ── Drawing, 1:1 ───────────────────────────────────────────────────────
    let x0 = surface[0][0];
    let x1 = surface[surface.len() - 1][0];
    let c = crit.circle;
    let ztop = (c.zc + 1.0).max(zmax + 2.0).max(main.grid_box[3]);
    let zbot = sub.base().max(zmin - 2.0 * (zmax - zmin) - 8.0).min(c.zc - c.r - 1.0);
    let hstep = nice_step(((ztop - zbot) / 8.0).max(0.5));
    let (zlo, zhi) = ((zbot / hstep).floor() * hstep, (ztop / hstep).ceil() * hstep);
    let head = 24.0 * k;
    let size = [head + (x1 - x0) * f + 10.0 * k, (zhi - zlo) * f + 30.0 * k];
    let area = {
        let b = plan.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, (_, p)| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
        match &ctx.site {
            Some(s) => {
                let r = s.drawing_rect(f);
                [b[0].min(r[0]), b[1].min(r[1]), b[2].max(r[2]), b[3].max(r[3])]
            }
            None => b,
        }
    };
    let at = req.at.unwrap_or_else(|| {
        let spot = crate::bridge::layout_spot("onder", size, area, 30.0 * k);
        [spot[0] + head, spot[1]]
    });
    let gx = |x: f64| at[0] + (x - x0) * f;
    let gy = |z: f64| at[1] + (z - zlo) * f;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let fill = ctx.layer("MO", "BODEM_BOORPROFIEL", Element::A)?;
    let bounds = ctx.layer("MO", "BODEM_BOORPROFIEL", Element::G)?;
    let circle_l = ctx.layer("MO", "BODEM", Element::G)?;
    let circle_t = ctx.layer("MO", "BODEM", Element::T)?;
    let water_l = ctx.layer("MO", "GRONDWATER", Element::G)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let name = req.name.clone().unwrap_or_else(|| format!("{} {where_}", tr("Stabiliteit", "Stability")));
    let mut out = Out::default();
    let data = record(&[
        ("stabiliteit", name.clone()),
        ("methode", req.method.label().into()),
        ("veiligheidsfactor", format!("{:.3}", crit.f)),
        ("middelpunt_x", format!("{:.2}", c.xc)),
        ("middelpunt_z", format!("{:.2}", c.zc)),
        ("straal_m", format!("{:.2}", c.r)),
    ]);
    let (fx0, fx1, fy0, fy1) = (at[0] - head, gx(x1), at[1], gy(zhi));
    out.put(&frame, [coloured(vec![[fx0, fy0], [fx1, fy0], [fx1, fy1], [fx0, fy1]], true, &frame, k, None, Some(&data))]);
    let mut z = zlo;
    while z <= zhi + 1e-9 {
        out.put(&frame, [coloured(vec![[at[0] - 1.5 * k, gy(z)], [at[0], gy(z)]], false, &frame, k, None, None)]);
        out.put(&frame_t, [side_text(&ctx, [at[0] - 2.5 * k, gy(z)], &nap(z, 1), 1.8, &frame_t, true)]);
        z += hstep;
    }
    let xstep = nice_step(((x1 - x0) / 10.0).max(0.5));
    let mut x = (x0 / xstep).ceil() * xstep;
    while x <= x1 + 1e-9 {
        out.put(&frame, [coloured(vec![[gx(x), fy0], [gx(x), fy0 + 1.5 * k]], false, &frame, k, None, None)]);
        out.put(&frame_t, [ctx.text([gx(x), fy0 + 3.5 * k], &num(x, 0), 1.8, 0.0, &frame_t, None)]);
        x += xstep;
    }
    for (code, ring) in band_polygons(&samples) {
        let rgb = soil::row(&code).map(|r| r.rgb()).unwrap_or((210, 210, 210));
        let pts: Vec<XY> = ring.iter().map(|p| [gx(p[0]), gy(p[1].max(zlo))]).collect();
        out.put(&fill, [hatch(&pts, &fill, rgb, None)]);
        out.put(&bounds, [coloured(pts, true, &bounds, k, Some((70, 70, 70)), None)]);
    }
    out.put(&ground_l, [coloured(surface.iter().map(|p| [gx(p[0]), gy(p[1])]).collect(), false, &ground_l, k, None, None)]);
    out.put(&water_l, [coloured(phreatic.iter().map(|p| [gx(p[0].clamp(x0, x1)), gy(p[1])]).collect(), false, &water_l, k, Some((30, 110, 220)), None)]);
    if let Some(l) = req.load {
        let (a, b) = (l.from.min(l.to).max(x0), l.from.max(l.to).min(x1));
        let hq = 4.0 * k;
        let top: Vec<XY> = [a, b].iter().map(|x| [gx(*x), gy(surf_at(*x)) + hq]).collect();
        out.put(&circle_l, [coloured(vec![[gx(a), gy(surf_at(a))], top[0], top[1], [gx(b), gy(surf_at(b))]], false, &circle_l, k, Some((200, 60, 160)), None)]);
        let n = ((b - a) / 1.0).ceil().max(1.0) as usize;
        for i in 0..=n {
            let xx = a + (b - a) * i as f64 / n as f64;
            let (yg, xg) = (gy(surf_at(xx)), gx(xx));
            out.put(&circle_l, [coloured(vec![[xg, yg + hq], [xg, yg], [xg - 0.6 * k, yg + 1.2 * k], [xg + 0.6 * k, yg + 1.2 * k], [xg, yg]], false, &circle_l, k, Some((200, 60, 160)), None)]);
        }
        out.put(&circle_t, [ctx.text([gx((a + b) / 2.0), gy(surf_at((a + b) / 2.0)) + hq + 2.5 * k], &format!("q = {} kPa", num(l.q, 1)), 1.8, 0.0, &circle_t, None)]);
    }
    // The critical circle, its slices, the centre and the search grid.
    let red = Some((220, 30, 30));
    let arc: Vec<XY> = (0..=120)
        .map(|i| {
            let xx = cut.entry + (cut.exit - cut.entry) * i as f64 / 120.0;
            [gx(xx), gy(c.zc - (c.r * c.r - (xx - c.xc).powi(2)).max(0.0).sqrt())]
        })
        .collect();
    out.put(&circle_l, [coloured(arc, false, &circle_l, k, red, Some(&data))]);
    for s in &cut.slices {
        for xx in [s.x - s.b / 2.0, s.x + s.b / 2.0] {
            let zb = c.zc - (c.r * c.r - (xx - c.xc).powi(2)).max(0.0).sqrt();
            out.put(&circle_l, [coloured(vec![[gx(xx), gy(zb)], [gx(xx), gy(surf_at(xx))]], false, &circle_l, k, Some((150, 40, 40)), None)]);
        }
    }
    let ce = [gx(c.xc), gy(c.zc)];
    out.put(&circle_l, [coloured(vec![[ce[0] - k, ce[1]], [ce[0] + k, ce[1]]], false, &circle_l, k, red, None), coloured(vec![[ce[0], ce[1] - k], [ce[0], ce[1] + k]], false, &circle_l, k, red, None)]);
    for xx in [cut.entry, cut.exit] {
        out.put(&circle_l, [coloured(vec![ce, [gx(xx), gy(surf_at(xx))]], false, &circle_l, k, Some((230, 150, 150)), None)]);
    }
    let gb = main.grid_box;
    out.put(&circle_l, [coloured(vec![[gx(gb[0]), gy(gb[1])], [gx(gb[2]), gy(gb[1])], [gx(gb[2]), gy(gb[3])], [gx(gb[0]), gy(gb[3])]], true, &circle_l, k, Some((150, 150, 150)), None)]);
    for g in &main.grid {
        out.put(&circle_l, [coloured(vec![[gx(g[0]) - 0.2 * k, gy(g[1])], [gx(g[0]) + 0.2 * k, gy(g[1])]], false, &circle_l, k, Some((150, 150, 150)), None)]);
    }
    let f_text = tf!("F = {} ({})", "F = {} ({})", num(crit.f, 2), req.method.label());
    out.put(&circle_t, [side_text(&ctx, [ce[0] + 2.0 * k, ce[1] + 2.0 * k], &f_text, 2.5, &circle_t, false)]);
    let title = format!("{name}: F = {} ({})", num(crit.f, 2), req.method.label());
    out.put(&frame_t, [side_text(&ctx, [fx0, fy1 + 20.0 * k], &title, 3.5, &frame_t, false)]);
    let water_text = match &req.phreatic {
        Phreatic::Auto => tf!("freatisch vlak op het laagste maaiveld {} (aanname)", "phreatic level at the lowest ground {} (assumed)", nap(zmin, 2)),
        Phreatic::Level(z) => tf!("freatisch vlak {}", "phreatic level {}", nap(*z, 2)),
        Phreatic::Points(_) => tr("freatische lijn opgegeven", "phreatic line given").to_string(),
    };
    let other_text = match (other_same, other.as_ref()) {
        (Some(o), Some(m)) => tf!("{} op deze cirkel {}, minimum {}", "{} on this circle {}, minimum {}", other_method.label(), num(o, 2), num(m.critical.f, 2)),
        _ => String::new(),
    };
    let notes = [
        format!("{} · {} · {}", sub.describe(), water_text, if on_design > 0 { tr("ontwerp- en bestaand maaiveld", "design and existing ground") } else { tr("bestaand maaiveld", "existing ground") }),
        format!("{} · {} {} · {} {}", other_text, cut.slices.len(), tr("lamellen", "slices"), main.tried, tr("cirkels", "circles")),
        disclaimer().to_string(),
    ];
    for (i, n) in notes.iter().enumerate() {
        out.put(&frame_t, [side_text(&ctx, [fx0, fy1 + (14.0 - 4.5 * i as f64) * k], n, 1.8, &frame_t, false)]);
    }
    let handles = out.write_handles(&name)?;
    ground.note_source();
    // ── Report ─────────────────────────────────────────────────────────────
    // The unit code of every slice's unit index.
    let mut codes = vec![String::new(); units.len()];
    for (c, i) in &index {
        codes[*i] = c.clone();
    }
    let unit_json: serde_json::Map<String, serde_json::Value> = index.iter().map(|(c, i)| (c.clone(), serde_json::to_value(&units[*i]).unwrap_or_default())).collect();
    api::note("method", req.method.label());
    api::note("factor_of_safety", crit.f);
    api::note("critical_circle", serde_json::json!({"xc": c.xc, "zc": c.zc, "r": c.r, "entry_x": cut.entry, "exit_x": cut.exit, "direction": crit.direction}));
    api::note("other_method", serde_json::json!({"method": other_method.label(), "on_critical_circle": other_same, "minimum": other.as_ref().map(|o| o.critical.f), "circle": other.as_ref().map(|o| o.critical.circle)}));
    api::note("slices", serde_json::to_value(&cut.slices).unwrap_or_default());
    api::note("slice_units", codes);
    api::note("units", serde_json::Value::Object(unit_json));
    api::note("surface", serde_json::to_value(&surface).unwrap_or_default());
    api::note("phreatic", serde_json::to_value(&phreatic).unwrap_or_default());
    api::note("load", serde_json::to_value(req.load).unwrap_or_default());
    api::note("soil", sub.describe());
    api::note("grid", serde_json::to_value(&main.grid).unwrap_or_default());
    api::note("grid_box", serde_json::to_value(main.grid_box).unwrap_or_default());
    api::note("circles_tried", main.tried);
    api::note("on_design_surface", on_design);
    api::note("place", serde_json::json!([at[0], at[1]]));
    api::note("extent", serde_json::json!([fx0, fy0, fx1, fy1 + 24.0 * k]));
    api::note("handles", handles.iter().map(|h| api::handle_text(*h)).collect::<Vec<_>>());
    api::note("disclaimer", disclaimer());
    if let Subsoil::Model(m) = &sub {
        api::note("soil_model", m.name.clone());
    }
    Ok(format!(
        "{title} — {} {} {}, {} {} {}; {} · {} {}. {}",
        tr("cirkel middelpunt", "circle centre"),
        num(c.xc, 1),
        nap(c.zc, 1),
        tr("straal", "radius"),
        num(c.r, 1),
        "m",
        other_text,
        handles.len(),
        tr("elementen", "entities"),
        disclaimer()
    ))
}

fn interp(line: &[[f64; 2]], x: f64) -> f64 {
    if x <= line[0][0] {
        return line[0][1];
    }
    if x >= line[line.len() - 1][0] {
        return line[line.len() - 1][1];
    }
    line.windows(2)
        .find(|w| x >= w[0][0] && x <= w[1][0])
        .map(|w| {
            let d = w[1][0] - w[0][0];
            if d.abs() < 1e-12 { w[1][1] } else { w[0][1] + (w[1][1] - w[0][1]) * (x - w[0][0]) / d }
        })
        .unwrap_or(line[0][1])
}
