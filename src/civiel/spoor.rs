//! Rail (`CIV_SPOOR`, `CIV_WISSEL`): a track alignment with cant and
//! turnouts. Each circular curve gets its equilibrium cant D_eq = c·V²/R
//! (c = 11.8 for standard gauge: V km/h, R m, D mm), an applied cant within
//! the limits, and the cant deficiency (and excess for slow trains). The
//! cant ramps linearly along the clothoids; ramp gradient and the rates of
//! change of cant and cant deficiency are checked. A cant diagram band goes
//! below the project; parallel tracks follow at the track centre distance.
//! Turnouts come from a library of standard types and create the diverging
//! track as an alignment. Values and sources: `assets/criteria/spoor.json`.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{EntityType, Handle};
use serde::Deserialize;

use super::as_profiel::{self, Alignment, AxisInput, Seg};
use super::ontwerpcriteria::Value;
use super::{record, station, Ctx, Out};
use crate::gis::crs::XY;
use crate::nlcs::{Element, LayerSpec};

const TABLE: &str = include_str!("../../assets/criteria/spoor.json");

/// The cant rules (mm, mm/m, mm/s).
#[derive(Clone, Debug, Deserialize)]
pub struct CantRules {
    /// c in D_eq = c·V²/R.
    pub factor: Value,
    pub d_max: Value,
    pub i_max: Value,
    #[serde(default)]
    pub e_max: Option<Value>,
    /// Steepest cant ramp (mm per m).
    pub helling_max: Value,
    #[serde(default)]
    pub dddt_max: Option<Value>,
    #[serde(default)]
    pub didt_max: Option<Value>,
    /// Applied cant is rounded to this (mm).
    #[serde(default = "five")]
    pub afronding: f64,
}

fn five() -> f64 {
    5.0
}

/// A standard turnout: 1:n, diverging radius, lengths (m) and speed.
#[derive(Clone, Debug, Deserialize)]
pub struct Turnout {
    pub id: String,
    pub n: f64,
    pub r: Value,
    /// Start of the turnout to the crossing point of the track axes (MS).
    #[serde(default)]
    pub a: Option<Value>,
    /// The crossing point to the end of the turnout, along the diverging track.
    #[serde(default)]
    pub b: Option<Value>,
    #[serde(default)]
    pub v_afbuigend: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RailTable {
    pub naam: String,
    pub verkanting: CantRules,
    pub spoorafstand: Value,
    pub wissels: Vec<Turnout>,
    pub bronnen: BTreeMap<String, String>,
}

impl RailTable {
    pub fn load() -> RailTable {
        serde_json::from_str(TABLE).expect("assets/criteria/spoor.json")
    }

    pub fn cite(&self, v: &Value) -> String {
        let src = self.bronnen.get(&v.bron).cloned().unwrap_or_else(|| v.bron.clone());
        if v.schatting { format!("{src} ({})", crate::lang::tr("schatting", "estimate")) } else { src }
    }

    pub fn turnout(&self, id: &str) -> Option<&Turnout> {
        let id = id.trim().replace(' ', "");
        self.wissels.iter().find(|t| t.id.eq_ignore_ascii_case(&id) || t.id.trim_start_matches("1:") == id)
    }
}

/// Equilibrium cant (mm) for speed `v` (km/h) on radius `r` (m).
pub fn equilibrium_cant(factor: f64, v: f64, r: f64) -> f64 {
    if r <= 0.0 { 0.0 } else { factor * v * v / r }
}

/// One circular curve of a track with its cant (m from the start, mm).
#[derive(Clone, Debug, PartialEq)]
pub struct CurveCant {
    pub s0: f64,
    pub s1: f64,
    pub r: f64,
    /// Turning left.
    pub left: bool,
    pub d_eq: f64,
    pub d: f64,
    /// Cant deficiency at V and excess at the minimum speed.
    pub i: f64,
    pub e: Option<f64>,
}

/// A cant ramp along a clothoid (m, mm, mm/m, mm/s).
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub s0: f64,
    pub s1: f64,
    pub d0: f64,
    pub d1: f64,
    pub gradient: f64,
    pub dddt: f64,
    pub didt: f64,
}

/// A rule broken: where, what, value, limit and source.
#[derive(Clone, Debug, PartialEq)]
pub struct Breach {
    pub s0: f64,
    pub s1: f64,
    pub check: &'static str,
    pub value: f64,
    pub limit: f64,
    pub source: String,
}

/// The cant of a whole track.
#[derive(Clone, Debug, Default)]
pub struct CantPlan {
    pub curves: Vec<CurveCant>,
    pub ramps: Vec<Ramp>,
    /// Applied and equilibrium cant (mm) along the track: (m, D, D_eq).
    pub line: Vec<(f64, f64, f64)>,
    pub breaches: Vec<Breach>,
}

/// The cant of a track for speed `v` (km/h; `vmin` for the excess), with
/// the applied cant per curve given (mm, the last repeats) or chosen: the
/// equilibrium cant, at most the maximum and (with `vmin`) at most the
/// slow train's equilibrium plus the allowed excess, rounded.
pub fn cant_plan(al: &Alignment, f: f64, v: f64, vmin: Option<f64>, given: &[f64], t: &RailTable) -> CantPlan {
    let rules = &t.verkanting;
    let c = rules.factor.v;
    let starts = al.seg_starts();
    let mut plan = CantPlan::default();
    // Applied cant per piece (circular curves only).
    let mut seg_cant: Vec<Option<f64>> = vec![None; al.segs.len()];
    let mut n_curve = 0;
    for (i, seg) in al.segs.iter().enumerate() {
        let Seg::Arc { r, sweep, .. } = seg else { continue };
        let r_m = r / f;
        let d_eq = equilibrium_cant(c, v, r_m);
        let d = match given.get(n_curve).or(given.last()) {
            Some(d) => *d,
            None => {
                let mut d = d_eq.min(rules.d_max.v);
                if let (Some(vm), Some(e)) = (vmin, &rules.e_max) {
                    d = d.min(equilibrium_cant(c, vm, r_m) + e.v);
                }
                let step = rules.afronding.max(1.0);
                ((d / step).round() * step).clamp(0.0, rules.d_max.v)
            }
        };
        n_curve += 1;
        seg_cant[i] = Some(d);
        let (s0, s1) = (starts[i] / f, (starts[i] + seg.length()) / f);
        let e = vmin.map(|vm| d - equilibrium_cant(c, vm, r_m));
        let cc = CurveCant { s0, s1, r: r_m, left: *sweep > 0.0, d_eq, d, i: d_eq - d, e };
        let mut breach = |check, value: f64, lim: &Value| {
            if value > lim.v + 1e-9 {
                plan.breaches.push(Breach { s0, s1, check, value, limit: lim.v, source: t.cite(lim) });
            }
        };
        breach("verkanting", d, &rules.d_max);
        breach("tekort", cc.i, &rules.i_max);
        if let (Some(e), Some(lim)) = (cc.e, &rules.e_max) {
            breach("overschot", e, lim);
        }
        // Without clothoids the cant has nowhere to ramp.
        let straight = |j: Option<usize>| j.and_then(|j| al.segs.get(j)).is_some_and(|s| matches!(s, Seg::Line { .. }));
        if d > 0.0 && (straight(i.checked_sub(1)) || straight(Some(i + 1)) || i == 0 || i + 1 == al.segs.len()) {
            plan.breaches.push(Breach { s0, s1, check: "geen_overgangsboog", value: d, limit: 0.0, source: t.cite(&rules.helling_max) });
        }
        plan.curves.push(cc);
    }
    // Cant at each piece's ends: curves keep theirs, clothoids ramp between
    // their neighbours (0 towards a straight), straights have none.
    let cant_end = |i: usize, at_start: bool| -> f64 {
        match &al.segs[i] {
            Seg::Arc { .. } => seg_cant[i].unwrap_or(0.0),
            Seg::Line { .. } => 0.0,
            Seg::Spiral { k0, k1, .. } => {
                let (k, j) = if at_start { (*k0, i.checked_sub(1)) } else { (*k1, Some(i + 1)) };
                if k.abs() < 1e-12 {
                    return 0.0;
                }
                // The curve on that side, or this clothoid's own curvature.
                match j.and_then(|j| seg_cant.get(j).copied().flatten()) {
                    Some(d) => d,
                    None => equilibrium_cant(c, v, 1.0 / (k.abs() * f)).min(rules.d_max.v),
                }
            }
        }
    };
    for (i, seg) in al.segs.iter().enumerate() {
        let (s0, len) = (starts[i] / f, seg.length() / f);
        let (d0, d1) = (cant_end(i, true), cant_end(i, false));
        let eq = |s_local: f64| equilibrium_cant(c, v, 1.0 / seg.curvature(s_local * f).abs().max(1e-300) / f);
        let n = if matches!(seg, Seg::Spiral { .. }) { 8 } else { 1 };
        for j in 0..=n {
            let u = j as f64 / n as f64;
            plan.line.push((s0 + u * len, d0 + (d1 - d0) * u, eq(u * len)));
        }
        if let Seg::Spiral { .. } = seg {
            if (d1 - d0).abs() < 1e-9 || len <= 0.0 {
                continue;
            }
            let gradient = (d1 - d0).abs() / len;
            let (i0, i1) = (eq(0.0) - d0, eq(len) - d1);
            let ramp = Ramp { s0, s1: s0 + len, d0, d1, gradient, dddt: gradient * v / 3.6, didt: (i1 - i0).abs() / len * v / 3.6 };
            let mut breach = |check, value: f64, lim: &Option<Value>| {
                if let Some(lim) = lim {
                    if value > lim.v + 1e-9 {
                        plan.breaches.push(Breach { s0, s1: s0 + len, check, value, limit: lim.v, source: t.cite(lim) });
                    }
                }
            };
            breach("helling", ramp.gradient, &Some(rules.helling_max.clone()));
            breach("dddt", ramp.dddt, &rules.dddt_max);
            breach("didt", ramp.didt, &rules.didt_max);
            plan.ramps.push(ramp);
        }
    }
    plan.line.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
    plan.breaches.sort_by(|a, b| a.s0.total_cmp(&b.s0));
    plan
}

/// The diverging track of a turnout: from its start `s` (drawing units)
/// on `main`, to the left or right, along or against the stationing, with
/// `extra` straight beyond its end. Returns the track and its main points
/// (start, crossing point of the axes, end).
pub fn turnout_track(main: &Alignment, s: f64, t: &Turnout, left: bool, forward: bool, extra: f64, f: f64) -> Result<(Alignment, [XY; 3]), String> {
    if t.n <= 0.0 || t.r.v <= 0.0 {
        return Err(format!("wissel {}: geen geldige geometrie", t.id));
    }
    let (p0, mut dir) = main.at(s);
    if !forward {
        dir += PI;
    }
    let side = if left { 1.0 } else { -1.0 };
    let alpha = (1.0 / t.n).atan();
    let r = t.r.v * f;
    let tangent = r * (alpha / 2.0).tan();
    // Lead straight before the curve when the table's a is longer than its tangent.
    let a = t.a.as_ref().map(|a| a.v * f).unwrap_or(tangent).max(tangent);
    let b = t.b.as_ref().map(|b| b.v * f).unwrap_or(tangent).max(tangent);
    let u = [dir.cos(), dir.sin()];
    let lead = a - tangent;
    let mut segs = Vec::new();
    let mut at = p0;
    if lead > 1e-9 {
        at = [p0[0] + u[0] * lead, p0[1] + u[1] * lead];
        segs.push(Seg::Line { a: p0, b: at });
    }
    let c = [at[0] - dir.sin() * r * side, at[1] + dir.cos() * r * side];
    let arc = Seg::Arc { c, r, a0: (at[1] - c[1]).atan2(at[0] - c[0]), sweep: side * alpha };
    let (arc_end, out_dir) = arc.at(arc.length());
    segs.push(arc);
    let ms = [p0[0] + u[0] * a, p0[1] + u[1] * a];
    let we = [ms[0] + out_dir.cos() * b, ms[1] + out_dir.sin() * b];
    let tail = b - tangent + extra.max(0.0);
    if tail > 1e-9 {
        segs.push(Seg::Line { a: arc_end, b: [arc_end[0] + out_dir.cos() * tail, arc_end[1] + out_dir.sin() * tail] });
    }
    Ok((Alignment { name: String::new(), start: 0.0, segs }, [p0, ms, we]))
}

/// Whether the interface is Dutch.
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

fn breach_text(b: &Breach) -> String {
    let (v, l) = (comma(b.value, 0), comma(b.limit, 0));
    let (vd, ld) = (comma(b.value, 2), comma(b.limit, 2));
    match b.check {
        "verkanting" => tx(format!("verkanting {v} mm > {l} mm"), format!("cant {v} mm > {l} mm")),
        "tekort" => tx(format!("tekort aan verkanting {v} mm > {l} mm"), format!("cant deficiency {v} mm > {l} mm")),
        "overschot" => tx(format!("overschot aan verkanting {v} mm > {l} mm"), format!("cant excess {v} mm > {l} mm")),
        "helling" => tx(format!("verkantingshelling {vd} mm/m > {ld} mm/m"), format!("cant gradient {vd} mm/m > {ld} mm/m")),
        "dddt" => tx(format!("verkantingsverandering {v} mm/s > {l} mm/s"), format!("rate of cant change {v} mm/s > {l} mm/s")),
        "didt" => tx(format!("verandering tekort {v} mm/s > {l} mm/s"), format!("rate of cant deficiency change {v} mm/s > {l} mm/s")),
        _ => tx(format!("verkanting {v} mm zonder overgangsboog"), format!("cant {v} mm without a transition curve")),
    }
}

/// Rail options of `CIV_SPOOR`.
pub struct RailRequest {
    pub input: AxisInput,
    pub name: Option<String>,
    pub start: Option<f64>,
    /// Line speed and the slowest train (km/h).
    pub v: f64,
    pub vmin: Option<f64>,
    /// Applied cant per curve (mm); empty: chosen.
    pub cant: Vec<f64>,
    /// Number of tracks and their centre distance (m; < 0 to the left).
    pub tracks: usize,
    pub distance: Option<f64>,
    /// Lower-left corner of the cant diagram (drawing), or found.
    pub diagram: Option<XY>,
}

/// `CIV_SPOOR`: a track on AM-AS_SPOOR_TREIN with stationing, main points,
/// radii, clothoids and the cant per curve; parallel tracks at the track
/// centre distance; the cant diagram below the project.
pub fn track(opt: &super::CivielOptions, req: RailRequest) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let table = RailTable::load();
    let extra: Vec<Handle> = match &req.input {
        AxisInput::Line(Some(h)) => vec![*h],
        _ => Vec::new(),
    };
    let doc = super::document(&["as"], &extra)?;
    let (mut al, src_layer) = match req.input {
        AxisInput::Line(h) => {
            let (al, _, layer) = as_profiel::axis(&doc, h)?;
            (al, layer)
        }
        AxisInput::Pis(pis, radii) => (Alignment::from_pis(&pis, &radii)?, String::new()),
        AxisInput::Layout(pis, radii, tr) => (Alignment::from_layout(&pis, &radii, &tr)?, String::new()),
    };
    if let Some(n) = req.name {
        al.name = n;
    }
    if al.name.is_empty() {
        al.name = crate::lang::tr("Spoor 1", "Track 1").to_string();
    }
    if let Some(s) = req.start {
        al.start = s;
    }
    let (f, k) = (ctx.f, ctx.k);
    let plan = cant_plan(&al, f, req.v, req.vmin, &req.cant, &table);
    let distance = req.distance.unwrap_or(table.spoorafstand.v);
    let axis = ctx.layer("AM", "AS_SPOOR_TREIN", Element::G)?;
    let axis_t = ctx.layer("AM", "AS_SPOOR_TREIN", Element::T)?;
    let mut out = Out::default();
    let cant_text = plan.curves.iter().map(|c| format!("{:.0}", c.d)).collect::<Vec<_>>().join(";");
    let data = vec![
        ("spoor".to_string(), al.name.clone()),
        ("snelheid_kmh".to_string(), format!("{}", req.v)),
        ("verkanting_mm".to_string(), cant_text),
        ("spoorafstand_m".to_string(), format!("{distance:.3}")),
    ];
    let drawn = !src_layer.contains("-AM-AS_SPOOR");
    as_profiel::draw_axis(&ctx, &al, &axis, &axis_t, drawn, &data, &mut out)?;
    // The cant of each curve beside it, on the outside.
    for c in &plan.curves {
        let (p, dir) = al.at((c.s0 + c.s1) / 2.0 * f);
        let side = if c.left { -1.0 } else { 1.0 };
        let at = [p[0] - dir.sin() * side * 6.0 * k, p[1] + dir.cos() * side * 6.0 * k];
        let mut label = format!("D = {} mm · Deq = {} · I = {}", comma(c.d, 0), comma(c.d_eq, 0), comma(c.i, 0));
        if let Some(e) = c.e {
            label.push_str(&format!(" · E = {}", comma(e, 0)));
        }
        out.put(&axis_t, [ctx.text(at, &label, 1.8, crate::emit::readable(dir), &axis_t, None)]);
    }
    // Parallel tracks.
    let mut parallel = 0;
    for n in 1..req.tracks.max(1) {
        let d = distance * n as f64;
        let Some(lw) = al.offset_polyline(d * f) else { continue };
        let pd = record(&[("spoor", format!("{} {}", al.name, n + 1)), ("as", al.name.clone()), ("verschuiving_m", format!("{d:.3}"))]);
        let mut e = EntityType::LwPolyline(lw);
        let cm = e.common_mut();
        cm.layer = axis.name.clone();
        cm.color = ocs_plugin_api::host::codec::Color::ByLayer;
        cm.linetype_scale = k;
        cm.extended_data.add_record(pd);
        out.put(&axis, [e]);
        let (p, dir) = al.at(0.0);
        let at = [p[0] + dir.sin() * d * f - dir.cos() * 2.0 * k, p[1] - dir.cos() * d * f - dir.sin() * 2.0 * k];
        let label = tx(format!("spoor {} ({} m)", n + 1, comma(d.abs(), 2)), format!("track {} ({} m)", n + 1, comma(d.abs(), 2)));
        let t = ctx.text(at, &label, 1.8, crate::emit::readable(dir), &axis_t, None);
        out.put(&axis_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        parallel += 1;
    }
    let handles = out.write_handles(&al.name)?;
    if drawn {
        if let Some(h) = handles.first() {
            super::api::note("as", super::api::handle_text(*h));
            crate::bridge::index_add("as", &al.name, &[*h], true);
        }
    }
    // The cant diagram, in its own undo step.
    let mut band = Out::default();
    let at = cant_diagram(&ctx, &al, &plan, req.v, &table, req.diagram, &mut band)?;
    let band_handles = band.write_handles(&format!("{} {}", crate::lang::tr("Verkantingsdiagram", "Cant diagram"), al.name))?;
    if let Some(h) = band_handles.first() {
        super::api::note("diagram", super::api::handle_text(*h));
    }
    let len = al.length() / f;
    super::api::note("naam", al.name.clone());
    super::api::note("lengte_m", len);
    super::api::note("snelheid_kmh", req.v);
    super::api::note("spoorafstand_m", distance);
    super::api::note("sporen", 1 + parallel);
    super::api::note("diagram_plaats", serde_json::json!([at[0], at[1]]));
    super::api::note(
        "bogen",
        plan.curves
            .iter()
            .map(|c| serde_json::json!({"station": al.start + c.s0, "eindstation": al.start + c.s1, "r_m": c.r, "deq_mm": c.d_eq, "d_mm": c.d, "tekort_mm": c.i, "overschot_mm": c.e}))
            .collect::<Vec<_>>(),
    );
    super::api::note(
        "overgangen",
        plan.ramps
            .iter()
            .map(|r| serde_json::json!({"station": al.start + r.s0, "eindstation": al.start + r.s1, "van_mm": r.d0, "tot_mm": r.d1, "helling_mm_per_m": r.gradient, "dddt_mm_per_s": r.dddt, "didt_mm_per_s": r.didt}))
            .collect::<Vec<_>>(),
    );
    super::api::note(
        "overtredingen",
        plan.breaches
            .iter()
            .map(|b| serde_json::json!({"station": al.start + b.s0, "eindstation": al.start + b.s1, "toets": b.check, "waarde": b.value, "grens": b.limit, "tekst": breach_text(b), "bron": b.source}))
            .collect::<Vec<_>>(),
    );
    let problems = if plan.breaches.is_empty() {
        crate::lang::tr("binnen de grenzen", "within the limits").to_string()
    } else {
        plan.breaches.iter().take(6).map(|b| format!("{} {}", station(al.start + b.s0), breach_text(b))).collect::<Vec<_>>().join("; ")
    };
    let n = handles.len() + band_handles.len();
    Ok(tx(
        format!("{}: {} m, V = {} km/h, {} bogen ({problems}), {} spo(o)r(en) — {n} elementen.", al.name, comma(len, 2), req.v, plan.curves.len(), 1 + parallel),
        format!("{}: {} m, V = {} km/h, {} curves ({problems}), {} track(s) — {n} entities.", al.name, comma(len, 2), req.v, plan.curves.len(), 1 + parallel),
    ))
}

/// The cant diagram: applied cant (VH-VERKANTING) and equilibrium cant
/// over the stationing, with the main points; lower-left corner `at` or a
/// free place below the project. Returns the corner.
fn cant_diagram(ctx: &Ctx, al: &Alignment, plan: &CantPlan, v: f64, t: &RailTable, at: Option<XY>, out: &mut Out) -> Result<XY, String> {
    let (f, k) = (ctx.f, ctx.k);
    let len = al.length();
    let top_mm = plan.line.iter().map(|p| p.1.max(p.2)).fold(t.verkanting.d_max.v, f64::max);
    let step_mm = super::nice_step(top_mm / 4.0);
    let top_mm = (top_mm / step_mm).ceil() * step_mm;
    // 1 mm cant is 0.25 paper mm, plus a row for the stationing.
    let scale = 0.25 * k;
    let row = 10.0 * k;
    let head = 30.0 * k;
    let height = row + top_mm * scale;
    let at = at.unwrap_or_else(|| {
        let spot = crate::bridge::layout_spot("onder", [head + len + 10.0 * k, height + 16.0 * k], as_profiel::clear_area(ctx, al), 30.0 * k);
        [spot[0] + head, spot[1]]
    });
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let cant = ctx.layer("VH", "VERKANTING", Element::G)?;
    let tp = ctx.layer("AM", "TANGENTPUNT", Element::G)?;
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    let gx = |s_m: f64| at[0] + s_m * f;
    let gy = |d: f64| at[1] + row + d * scale;
    let (x0, x1, y0, y1) = (at[0] - head, gx(len / f), at[1], gy(top_mm));
    let data = record(&[("verkantingsdiagram", al.name.clone()), ("as", al.name.clone()), ("schaal_mm", format!("{scale}"))]);
    out.put(&frame, crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], &frame.name, k, Some(&data)));
    out.put(&frame, pl(vec![[x0, y0 + row], [x1, y0 + row]], &frame));
    out.put(&frame, pl(vec![[at[0], y0], [at[0], y1]], &frame));
    // Cant grid with labels.
    let mut d = 0.0;
    while d <= top_mm + 1e-9 {
        if d > 1e-9 {
            out.put(&frame, pl(vec![[at[0], gy(d)], [x1, gy(d)]], &frame));
        }
        let tt = ctx.text([at[0] - 2.0 * k, gy(d)], &format!("{d:.0}"), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        d += step_mm;
    }
    let tt = ctx.text([x0 + 2.0 * k, y0 + row / 2.0], crate::lang::tr("Stationering", "Stationing"), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let tt = ctx.text([x0 + 2.0 * k, (gy(0.0) + y1) / 2.0], crate::lang::tr("Verkanting (mm)", "Cant (mm)"), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    // Equilibrium cant (thin, frame layer) and applied cant.
    out.put(&frame, pl(plan.line.iter().map(|p| [gx(p.0), gy(p.2.min(top_mm))]).collect(), &frame));
    out.put(&cant, pl(plan.line.iter().map(|p| [gx(p.0), gy(p.1)]).collect(), &cant));
    // Main points with their station.
    for m in al.marks() {
        let x = gx(m.s / f);
        out.put(&tp, pl(vec![[x, y0], [x, y1]], &tp));
        let tt = ctx.text([x - 1.0 * k, y0 + row / 2.0], &station(al.start + m.s / f), 1.8, PI / 2.0, &frame_t, None);
        out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
    }
    // Each curve's cant.
    for c in &plan.curves {
        let tt = ctx.text([gx((c.s0 + c.s1) / 2.0), gy(c.d) + 2.0 * k], &format!("D {} · Deq {} · I {}", comma(c.d, 0), comma(c.d_eq, 0), comma(c.i, 0)), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Center, TextVerticalAlignment::Bottom)]);
    }
    let title = tx(format!("Verkantingsdiagram {} — V = {} km/h", al.name, v), format!("Cant diagram {} — V = {} km/h", al.name, v));
    let tt = ctx.text([x0, y1 + 8.0 * k], &title, 3.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let sub = tx(
        format!("Deq = {} · V² / R (dunne lijn); toegepaste verkanting (dikke lijn), lineair over de overgangsbogen", comma(t.verkanting.factor.v, 1)),
        format!("Deq = {} · V² / R (thin line); applied cant (thick line), linear along the transition curves", comma(t.verkanting.factor.v, 1)),
    );
    let tt = ctx.text([x0, y1 + 3.5 * k], &sub, 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(tt, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    Ok(at)
}

/// `CIV_WISSEL`: a turnout of a standard type at a station of a track,
/// to the left or right: the diverging track as an alignment, the main
/// points WB (start), MS (crossing point of the axes) and WE (end).
#[allow(clippy::too_many_arguments)]
pub fn turnout(opt: &super::CivielOptions, main_h: Option<Handle>, at_station: f64, kind: &str, left: bool, forward: bool, extra: f64, name: Option<String>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let table = RailTable::load();
    let t = table.turnout(kind).ok_or_else(|| {
        let ids = table.wissels.iter().map(|w| w.id.as_str()).collect::<Vec<_>>().join(", ");
        tx(format!("onbekend wisseltype {kind}; kies {ids}"), format!("unknown turnout type {kind}; choose {ids}"))
    })?;
    let doc = super::document(&["as"], &main_h.into_iter().collect::<Vec<_>>())?;
    let (main, _, _) = as_profiel::axis(&doc, main_h)?;
    let f = ctx.f;
    let s = (at_station - main.start) * f;
    if s < -1e-6 || s > main.length() + 1e-6 {
        return Err(tx(
            format!("station {} ligt niet op {} ({} – {})", station(at_station), main.name, station(main.start), station(main.start + main.length() / f)),
            format!("station {} is not on {} ({} – {})", station(at_station), main.name, station(main.start), station(main.start + main.length() / f)),
        ));
    }
    let (mut track, [wb, ms, we]) = turnout_track(&main, s, t, left, forward, extra * f, f)?;
    // Numbered after the turnouts already in the drawing.
    let n = crate::bridge::index_read().iter().filter(|(kind, _, _)| kind == "wissel").count() + 1;
    let wname = name.unwrap_or_else(|| format!("W{n}"));
    track.name = format!("{wname} {}", crate::lang::tr("afbuigend", "diverging"));
    track.start = at_station;
    let k = ctx.k;
    let axis = ctx.layer("AM", "AS_SPOOR_TREIN", Element::G)?;
    let axis_t = ctx.layer("AM", "AS_SPOOR_TREIN", Element::T)?;
    let tp = ctx.layer("AM", "TANGENTPUNT", Element::G)?;
    let tp_t = ctx.layer("AM", "TANGENTPUNT", Element::T)?;
    let side = if left { crate::lang::tr("links", "left") } else { crate::lang::tr("rechts", "right") };
    let data = vec![
        ("wissel".to_string(), wname.clone()),
        ("type".to_string(), t.id.clone()),
        ("hoofdspoor".to_string(), main.name.clone()),
        ("wisselbegin_station".to_string(), format!("{at_station:.3}")),
        ("zijde".to_string(), if left { "links" } else { "rechts" }.to_string()),
    ];
    let mut out = Out::default();
    as_profiel::draw_axis(&ctx, &track, &axis, &axis_t, true, &data, &mut out)?;
    // The turnout's main points, square to the main track.
    let (_, dir) = main.at(s);
    for (p, code) in [(wb, "WB"), (ms, "MS"), (we, "WE")] {
        let nrm = [-dir.sin(), dir.cos()];
        let half = 3.0 * k;
        out.put(&tp, crate::emit::polylines(&[(vec![[p[0] - nrm[0] * half, p[1] - nrm[1] * half], [p[0] + nrm[0] * half, p[1] + nrm[1] * half]], false)], &tp.name, k, None));
        let label = match (code, nl()) {
            (c, true) => c,
            ("WB", false) => "ST",
            ("MS", false) => "IP",
            _ => "END",
        };
        let tt = ctx.text([p[0] + nrm[0] * 4.0 * k, p[1] + nrm[1] * 4.0 * k], label, 1.8, crate::emit::readable(dir), &tp_t, None);
        out.put(&tp_t, [tt]);
    }
    let speed = t.v_afbuigend.as_ref().map(|v| format!(" · V {} km/h", comma(v.v, 0))).unwrap_or_default();
    let label = format!("{wname} {} R{} {side}{speed}", t.id, comma(t.r.v, 0));
    let mid = [(wb[0] + we[0]) / 2.0, (wb[1] + we[1]) / 2.0];
    let lside = if left { 1.0 } else { -1.0 };
    let tt = ctx.text([mid[0] - dir.sin() * lside * 9.0 * k, mid[1] + dir.cos() * lside * 9.0 * k], &label, 2.5, crate::emit::readable(dir), &axis_t, None);
    out.put(&axis_t, [tt]);
    let handles = out.write_handles(&format!("{} {wname}", crate::lang::tr("Wissel", "Turnout")))?;
    if let Some(h) = handles.first() {
        super::api::note("as", super::api::handle_text(*h));
        crate::bridge::index_add("as", &track.name, &[*h], true);
        crate::bridge::index_add("wissel", &wname, &[*h], true);
    }
    let to_site = |p: XY| ctx.from_drawing(p);
    super::api::note("wissel", wname.clone());
    super::api::note("type", t.id.clone());
    super::api::note("r_m", t.r.v);
    super::api::note("hoek_graden", (1.0 / t.n).atan().to_degrees());
    super::api::note("wisselbegin", serde_json::json!(to_site(wb)));
    super::api::note("mathematisch_snijpunt", serde_json::json!(to_site(ms)));
    super::api::note("wisseleinde", serde_json::json!(to_site(we)));
    super::api::note("afbuigend_lengte_m", track.length() / f);
    super::api::note("bron", table.cite(&t.r));
    let n_ent = handles.len();
    Ok(tx(
        format!("{label}: wisselbegin {} op {}, afbuigend spoor {} m — {n_ent} elementen.", station(at_station), main.name, comma(track.length() / f, 2)),
        format!("{label}: turnout start {} on {}, diverging track {} m — {n_ent} entities.", station(at_station), main.name, comma(track.length() / f, 2)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equilibrium_cant_for_standard_gauge() {
        let t = RailTable::load();
        // 11.8 · V² / R: 120 km/h on R 1000 m is 169,9 mm; 80 on 500 is 151,0.
        assert!((t.verkanting.factor.v - 11.8).abs() < 1e-12);
        assert!((equilibrium_cant(11.8, 120.0, 1000.0) - 169.92).abs() < 1e-9);
        assert!((equilibrium_cant(11.8, 80.0, 500.0) - 151.04).abs() < 1e-9);
        // From g and the rail centre distance of 1500 mm: D = s·v²/(g·R).
        let v = 120.0 / 3.6;
        assert!((1500.0 * v * v / (9.81 * 1000.0) - equilibrium_cant(11.8, 120.0, 1000.0)).abs() < 0.2);
        for w in &t.wissels {
            assert!(t.bronnen.contains_key(&w.r.bron), "{}", w.id);
        }
    }

    #[test]
    fn cant_ramps_along_the_clothoids() {
        let t = RailTable::load();
        // East, then 40° left: R 1000 with clothoids of 120 m, V 120.
        let pi2 = [600.0 + 600.0 * 40f64.to_radians().cos(), 600.0 * 40f64.to_radians().sin()];
        let al = Alignment::from_layout(&[[0.0, 0.0], [600.0, 0.0], pi2], &[1000.0], &[as_profiel::Transition::Length(120.0)]).unwrap();
        let plan = cant_plan(&al, 1.0, 120.0, None, &[], &t);
        assert_eq!(plan.curves.len(), 1);
        let c = &plan.curves[0];
        let dmax = t.verkanting.d_max.v;
        assert!((c.d_eq - 169.92).abs() < 1e-9);
        assert!((c.d - (169.92f64.min(dmax) / 5.0).round() * 5.0).abs() < 1e-9 && (c.i - (c.d_eq - c.d)).abs() < 1e-12);
        // Two ramps of 120 m from 0 to D and back.
        assert_eq!(plan.ramps.len(), 2);
        let r = &plan.ramps[0];
        assert!((r.s1 - r.s0 - 120.0).abs() < 1e-9 && r.d0 == 0.0 && (r.d1 - c.d).abs() < 1e-12);
        assert!((r.gradient - c.d / 120.0).abs() < 1e-12 && (r.dddt - c.d / 120.0 * 120.0 / 3.6).abs() < 1e-9);
        // Halfway the first clothoid the cant is half.
        let mid = r.s0 + 60.0;
        let near = plan.line.iter().min_by(|a, b| (a.0 - mid).abs().total_cmp(&(b.0 - mid).abs())).unwrap();
        assert!((near.0 - mid).abs() < 1e-9 && (near.1 - c.d / 2.0).abs() < 1e-9);
        // A given cant of 200 mm is too much; a curve without clothoids too.
        let plan = cant_plan(&al, 1.0, 120.0, Some(60.0), &[200.0], &t);
        let checks: Vec<&str> = plan.breaches.iter().map(|b| b.check).collect();
        assert!(checks.contains(&"verkanting"), "{checks:?}");
        let bare = Alignment::from_pis(&[[0.0, 0.0], [600.0, 0.0], pi2], &[1000.0]).unwrap();
        let plan = cant_plan(&bare, 1.0, 120.0, None, &[], &t);
        assert!(plan.breaches.iter().any(|b| b.check == "geen_overgangsboog"));
    }

    #[test]
    fn a_turnout_diverges_at_its_angle() {
        let t = RailTable::load();
        let w = t.turnout("1:9").unwrap();
        let main = Alignment::from_pis(&[[0.0, 0.0], [500.0, 0.0]], &[]).unwrap();
        let (track, [wb, ms, we]) = turnout_track(&main, 100.0, w, true, true, 50.0, 1.0).unwrap();
        assert_eq!(wb, [100.0, 0.0]);
        // The diverging track leaves at 1:9 to the left, through MS on the main axis.
        let (end, dir) = track.at(track.length());
        assert!((dir - (1.0f64 / 9.0).atan()).abs() < 1e-12 && end[1] > 0.0);
        assert!(ms[1].abs() < 1e-12 && ms[0] > 100.0);
        let along = [(we[0] - ms[0]), (we[1] - ms[1])];
        assert!((along[1] / along[0] - 1.0 / 9.0).abs() < 1e-9);
        // WE lies on the diverging track.
        let (_, off) = track.station_offset(we);
        assert!(off.abs() < 1e-9);
        // To the right, against the stationing.
        let (track, _) = turnout_track(&main, 300.0, w, false, false, 0.0, 1.0).unwrap();
        let (end, _) = track.at(track.length());
        assert!(end[0] < 300.0 && end[1] > 0.0, "{end:?}");
    }
}
