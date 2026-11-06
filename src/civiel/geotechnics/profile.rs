//! `CIV_GRONDPROFIEL`: the soil along a line from the BRO CPTs and
//! boreholes near it, as a profile view (soil units in colour, the columns
//! themselves, the ground line from the TIN or AHN, a legend), kept in the
//! drawing as a soil model along the line's stations for `CIV_STABILITEIT`
//! and `CIV_ZETTING`.

use ocs_plugin_api::host::codec::Handle;

use super::model::{self, Model};
use super::{band_boundaries, band_polygons, coloured, disclaimer, hatch, num, side_text, soil, tf, unit_line, SoilArgs};
use crate::civiel::as_profiel::{self, Alignment};
use crate::civiel::{api, nice_step, record, station, Ctx, Maaiveld, Out};
use crate::gis::bro::Kind;
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::lang::tr;
use crate::nlcs::Element;

pub struct Request {
    /// The line or axis; `None`: the selection.
    pub line: Option<Handle>,
    pub kinds: Vec<Kind>,
    /// Search distance from the line (m).
    pub radius: f64,
    pub max: usize,
    pub min_layer: f64,
    pub name: Option<String>,
    pub soil: SoilArgs,
    /// Lower-left corner of the view (drawing coordinates).
    pub at: Option<XY>,
}

/// The line of an alignment in project coordinates, every metre on curves.
pub fn line_of(ctx: &Ctx, al: &Alignment) -> Vec<XY> {
    let f = ctx.f;
    let mut ss: Vec<f64> = al.seg_starts();
    let len = al.length();
    ss.push(len);
    for (i, seg) in al.segs.iter().enumerate() {
        if !matches!(seg, as_profiel::Seg::Line { .. }) {
            let s0 = ss[i];
            let l = seg.length();
            let n = (l / f).ceil().clamp(2.0, 2000.0) as usize;
            ss.extend((1..n).map(|k| s0 + l * k as f64 / n as f64));
        }
    }
    ss.sort_by(f64::total_cmp);
    ss.dedup_by(|a, b| (*a - *b).abs() < 1e-6 * f);
    ss.into_iter().map(|s| ctx.from_drawing(al.at(s).0)).collect()
}

pub fn soil_profile(opt: &crate::civiel::CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = crate::civiel::document(&["tin", "as"], &req.line.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = as_profiel::axis(&doc, req.line)?;
    let line = line_of(&ctx, &al);
    let len = al.length() / f;
    let (columns, skipped) = super::collect_near(&ctx, &line, &req.kinds, req.radius, req.max, req.min_layer, progress)?;
    if columns.is_empty() {
        let why = skipped.first().map(|s| format!(" ({s})")).unwrap_or_default();
        return Err(tf!("geen sonderingen of boringen in de BRO binnen {} m van de lijn{}", "no CPTs or boreholes in the BRO within {} m of the line{}", num(req.radius, 0), why));
    }
    let base = columns.iter().map(|c| c.end).fold(f64::MAX, f64::min);
    let codes: Vec<String> = columns.iter().flat_map(|c| c.layers.iter().map(|l| l.code.clone())).collect();
    let units = super::units_of(codes.iter().map(String::as_str), &req.soil.overrides)?;
    let name = req.name.clone().unwrap_or_else(|| if al.name.is_empty() { tr("Grondprofiel", "Soil profile").to_string() } else { format!("{} {}", tr("Grondprofiel", "Soil profile"), al.name) });
    let kinds: Vec<&str> = req.kinds.iter().map(|k| if *k == Kind::Cpt { "CPT" } else { "BHR-GT" }).collect();
    let m = Model { name: name.clone(), line: line.clone(), start_station: al.start, columns, base, units, source: format!("BRO {} ({} {} m)", kinds.join(" + "), tr("binnen", "within"), num(req.radius, 0)) };
    // The ground along the line, every step.
    progress(tr("Maaiveld langs de lijn…", "Ground along the line…").into());
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let step = (opt.step.max(0.25)).min(len / 50.0).max(0.25);
    let n = ((len / step).ceil() as usize).clamp(2, 4000);
    let stations: Vec<f64> = (0..=n).map(|i| len * i as f64 / n as f64).collect();
    let gz: Vec<Option<f64>> = stations.iter().map(|s| ground.height(al.at(s * f).0, f)).collect();
    // The soil at every station, clipped to the ground.
    let samples: Vec<(f64, Vec<model::Band>)> = stations
        .iter()
        .zip(&gz)
        .map(|(s, g)| {
            let bands = m.at_station(*s);
            let top = g.or(bands.first().map(|b| b.top)).unwrap_or(0.0);
            (*s, model::clip_to_surface(&bands, top))
        })
        .collect();
    let zmax = samples.iter().filter_map(|s| s.1.first().map(|b| b.top)).chain(m.columns.iter().map(|c| c.surface)).fold(f64::MIN, f64::max);
    let v = opt.exaggeration.max(1.0);
    let hstep = nice_step(((zmax - base) / 8.0).max(0.25));
    let zlo = (base / hstep).floor() * hstep;
    let zhi = (zmax / hstep).ceil() * hstep + hstep;
    let row = 10.0 * k;
    let head = 30.0 * k;
    let legend_w = 120.0 * k;
    let height = row + (zhi - zlo) * v * f + 26.0 * k;
    let at = req.at.unwrap_or_else(|| {
        let spot = crate::bridge::layout_spot("onder", [head + len * f + legend_w, height], as_profiel::clear_area(&ctx, &al), 30.0 * k);
        [spot[0] + head, spot[1]]
    });
    let gx = |s: f64| at[0] + s * f;
    let gy = |z: f64| at[1] + row + (z - zlo) * v * f;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let fill = ctx.layer("MO", "BODEM_BOORPROFIEL", Element::A)?;
    let bounds = ctx.layer("MO", "BODEM_BOORPROFIEL", Element::G)?;
    let text = ctx.layer("MO", "BODEM_BOORPROFIEL", Element::T)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let mut out = Out::default();
    let data = record(&[("grondprofiel", name.clone()), ("as", al.name.clone()), ("methode", "BRO; Robertson 2010 / NEN-EN-ISO 14688 / NEN 5104; NEN 9997-1 tabel 2.b".into())]);
    // Frame and height grid.
    let (x0, x1, y0, y1) = (at[0] - head, gx(len), at[1], gy(zhi));
    out.put(&frame, [coloured(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true, &frame, k, None, Some(&data))]);
    out.put(&frame, [coloured(vec![[at[0], y0], [at[0], y1]], false, &frame, k, None, None), coloured(vec![[x0, y0 + row], [x1, y0 + row]], false, &frame, k, None, None)]);
    let mut z = zlo;
    while z <= zhi + 1e-9 {
        let y = gy(z);
        if z > zlo + 1e-9 && z < zhi - 1e-9 {
            out.put(&frame, [coloured(vec![[at[0], y], [x1, y]], false, &frame, k, Some((200, 200, 200)), None)]);
        }
        out.put(&frame_t, [side_text(&ctx, [at[0] - 2.0 * k, y], &nap(z, 2), 1.8, &frame_t, true)]);
        z += hstep;
    }
    out.put(&frame_t, [side_text(&ctx, [x0 + 2.0 * k, y0 + row / 2.0], tr("Station", "Station"), 1.8, &frame_t, false)]);
    let label = nice_step((len / 10.0).max(1.0));
    let mut s = 0.0;
    while s <= len + 1e-9 {
        out.put(&frame, [coloured(vec![[gx(s), y0 + row], [gx(s), y0 + row + 1.5 * k]], false, &frame, k, None, None)]);
        out.put(&frame_t, [ctx.text([gx(s), y0 + row / 2.0], &station(al.start + s), 1.8, 0.0, &frame_t, None)]);
        s += label;
    }
    // Soil: colour per unit, boundaries as lines.
    let polys = band_polygons(&samples);
    for (code, ring) in &polys {
        let rgb = soil::row(code).map(|r| r.rgb()).unwrap_or((210, 210, 210));
        let pts: Vec<XY> = ring.iter().map(|p| [gx(p[0]), gy(p[1])]).collect();
        let d = record(&[("grondprofiel", name.clone()), ("grondsoort", code.clone()), ("bron", "NEN 9997-1 tabel 2.b".into())]);
        out.put(&fill, [hatch(&pts, &fill, rgb, Some(&d))]);
    }
    for line in band_boundaries(&samples) {
        out.put(&bounds, [coloured(line.iter().map(|p| [gx(p[0]), gy(p[1])]).collect(), false, &bounds, k, Some((60, 60, 60)), None)]);
    }
    // The ground line.
    let mut run: Vec<XY> = Vec::new();
    for (s, g) in stations.iter().zip(&gz) {
        match g {
            Some(z) => run.push([gx(*s), gy(*z)]),
            None => {
                if run.len() > 1 {
                    out.put(&ground_l, [coloured(std::mem::take(&mut run), false, &ground_l, k, None, None)]);
                }
                run.clear();
            }
        }
    }
    if run.len() > 1 {
        out.put(&ground_l, [coloured(run, false, &ground_l, k, None, None)]);
    }
    // The columns as measured, with their id.
    let bar = 1.6 * k;
    for c in &m.columns {
        let kind_obj = if c.kind == "cpt" { "ONDERZOEK_SONDERING" } else { "ONDERZOEK_BORING" };
        let (cg, ct) = (ctx.layer("MO", kind_obj, Element::G)?, ctx.layer("MO", kind_obj, Element::T)?);
        let x = gx(c.station);
        let d = record(&[("bro_id", c.id.clone()), ("soort", c.kind.clone()), ("station_m", format!("{:.2}", c.station)), ("offset_m", format!("{:.2}", c.offset)), ("maaiveld_m_nap", format!("{:.2}", c.surface)), ("einde_m_nap", format!("{:.2}", c.end))]);
        for l in &c.layers {
            let rgb = soil::row(&l.code).map(|r| r.rgb()).unwrap_or((210, 210, 210));
            let ring = vec![[x - bar, gy(l.top)], [x + bar, gy(l.top)], [x + bar, gy(l.bottom)], [x - bar, gy(l.bottom)]];
            out.put(&cg, [hatch(&ring, &cg, rgb, Some(&d))]);
            out.put(&cg, [coloured(ring, true, &cg, k, Some((0, 0, 0)), None)]);
        }
        let mut t = ctx.text([x, gy(c.surface) + 2.0 * k], &c.id, 1.8, std::f64::consts::FRAC_PI_2, &ct, Some(&d));
        if let ocs_plugin_api::host::codec::EntityType::Text(tx) = &mut t {
            tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
        }
        out.put(&ct, [t]);
        out.put(&ct, [ctx.text([x, gy(c.end) - 2.5 * k], &nap(c.end, 1), 1.5, 0.0, &ct, None)]);
    }
    // Legend to the right.
    let used: Vec<&String> = {
        let mut u: Vec<&String> = m.units.keys().collect();
        u.sort_by_key(|c| soil::TABLE.iter().position(|r| r.code == c.as_str()).unwrap_or(99));
        u
    };
    let lx = x1 + 8.0 * k;
    let mut ly = y1 - 4.0 * k;
    out.put(&text, [side_text(&ctx, [lx, ly], tr("Grondsoorten (NEN 9997-1 tabel 2.b)", "Soil units (NEN 9997-1 table 2.b)"), 2.5, &text, false)]);
    for c in &used {
        ly -= 6.0 * k;
        let rgb = soil::row(c).map(|r| r.rgb()).unwrap_or((210, 210, 210));
        let ring = vec![[lx, ly - 2.0 * k], [lx + 6.0 * k, ly - 2.0 * k], [lx + 6.0 * k, ly + 2.0 * k], [lx, ly + 2.0 * k]];
        out.put(&fill, [hatch(&ring, &fill, rgb, None)]);
        out.put(&bounds, [coloured(ring, true, &bounds, k, Some((60, 60, 60)), None)]);
        out.put(&text, [side_text(&ctx, [lx + 8.0 * k, ly], &unit_line(c, &m.units[*c]), 1.8, &text, false)]);
    }
    // Title and notes.
    let title = format!("{name} — {} {}, {} {}", m.columns.iter().filter(|c| c.kind == "cpt").count(), tr("sonderingen", "CPTs"), m.columns.iter().filter(|c| c.kind != "cpt").count(), tr("boringen", "boreholes"));
    out.put(&frame_t, [side_text(&ctx, [x0, y1 + 16.0 * k], &title, 3.5, &frame_t, false)]);
    let sub = tf!(
        "{} · horizontaal 1:{} · verticaal 1:{} (overhoging {}×) · lagen tussen kolommen lineair geïnterpoleerd, wiggen tot de buurkolom",
        "{} · horizontal 1:{} · vertical 1:{} ({}× exaggerated) · layers interpolated linearly between columns, wedges to the neighbour",
        ground.label(),
        opt.nlcs.scale,
        (opt.nlcs.scale / v).round(),
        num(v, 0)
    );
    out.put(&frame_t, [side_text(&ctx, [x0, y1 + 10.0 * k], &sub, 1.8, &frame_t, false)]);
    out.put(&frame_t, [side_text(&ctx, [x0, y1 + 5.0 * k], disclaimer(), 1.8, &frame_t, false)]);
    progress(tr("Grondprofiel tekenen…", "Drawing the soil profile…").into());
    let handles = out.write_handles(&name)?;
    ground.note_source();
    model::save(&m)?;
    if let Some(h) = handles.first() {
        crate::bridge::index_add("grondprofiel", &name, &[*h], true);
        api::note("grondprofiel", api::handle_text(*h));
    }
    api::note("model", serde_json::to_value(&m).unwrap_or_default());
    api::note("columns", m.columns.len());
    api::note("units", m.units.keys().cloned().collect::<Vec<_>>());
    api::note("polygons", polys.len());
    api::note("base_m_nap", base);
    api::note("skipped", skipped.clone());
    api::note("place", serde_json::json!([at[0], at[1]]));
    api::note("extent", serde_json::json!([x0, y0, lx + legend_w, y1 + 20.0 * k]));
    api::note("disclaimer", disclaimer());
    Ok(format!(
        "{title}: {} m, {} {} ({}), {} {} — {} {}. {}",
        num(len, 1),
        m.units.len(),
        tr("grondsoorten", "soil units"),
        used.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", "),
        handles.len(),
        tr("elementen", "entities"),
        tr("grondmodel opgeslagen als", "soil model kept as"),
        name,
        disclaimer()
    ))
}
