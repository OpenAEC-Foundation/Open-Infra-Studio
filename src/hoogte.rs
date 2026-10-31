//! AHN in the drawing: spot heights, contour lines, profiles and the point
//! cloud.
//! Heights are NAP; entities sit at their true elevation (Z).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use ocs_plugin_api::host::codec::{EntityType, Line, Vector3};
use ocs_plugin_api::host::ExtendedDataRecord;

use crate::basisdata::{rd_bbox, NlcsOptions};
use crate::bridge;
use crate::emit;
use crate::gis::ahn::{self, Model, Station};
use crate::gis::crs::{self, XY};
use crate::gis::pointcloud;
use crate::gis::site::Site;
use crate::gis::terrain::Source;
use crate::gis::vector;
use crate::lang::{self, tr};
use crate::nlcs::{self, Element, LayerSpec};

#[derive(Clone, Debug)]
pub struct AhnOptions {
    pub nlcs: NlcsOptions,
    /// Height model for spot heights.
    pub model: Model,
    /// Spot height grid spacing (m).
    pub spot_spacing: f64,
    /// Contour interval (m); every `major_every`-th level is a major line.
    pub interval: f64,
    pub major_every: u32,
    /// Grid cell the contours are traced on (m).
    pub contour_cell: f64,
    pub smooth: bool,
    /// Profile: station distance (m), vertical exaggeration, DSM line too.
    pub step: f64,
    pub exaggeration: f64,
    pub surface: bool,
    pub cloud: pointcloud::Options,
}

impl Default for AhnOptions {
    fn default() -> Self {
        Self {
            nlcs: NlcsOptions::default(),
            model: Model::Dtm,
            spot_spacing: 10.0,
            interval: 0.25,
            major_every: 4,
            contour_cell: 1.0,
            smooth: true,
            step: 1.0,
            exaggeration: 10.0,
            surface: false,
            cloud: pointcloud::Options::default(),
        }
    }
}

/// The settings the window last used; commands from the ribbon use them too.
pub fn settings() -> AhnOptions {
    store().lock().unwrap().clone()
}

pub fn set_settings(o: AhnOptions) {
    *store().lock().unwrap() = o;
}

fn store() -> &'static Mutex<AhnOptions> {
    static S: OnceLock<Mutex<AhnOptions>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(AhnOptions::default()))
}

/// Height in NAP the Dutch way: "+2,43", "-4,57".
pub fn nap(z: f64, decimals: usize) -> String {
    let s = format!("{:+.*}", decimals, z);
    s.replace('.', ",")
}

fn layer(group: &str, name: &str, opt: &NlcsOptions, el: Element) -> Result<LayerSpec, String> {
    let obj = nlcs::object_by_name(group, name).ok_or_else(|| format!("NLCS-object {group} {name} ontbreekt"))?;
    Ok(nlcs::layer(obj, opt.status, &opt.discipline, el))
}

fn record(pairs: &[(&str, String)]) -> ExtendedDataRecord {
    bridge::object_data(pairs.iter().map(|(k, v)| (*k, v.clone())))
}

fn at_z(mut e: EntityType, z: f64) -> EntityType {
    match &mut e {
        EntityType::Text(t) => {
            t.insertion_point.z = z;
            if let Some(a) = t.alignment_point.as_mut() {
                a.z = z;
            }
        }
        EntityType::Circle(c) => c.center.z = z,
        EntityType::LwPolyline(l) => l.elevation = z,
        _ => {}
    }
    e
}

/// Writes entities per NLCS layer as one undo step.
struct Out {
    layers: Vec<(LayerSpec, Vec<EntityType>)>,
}

impl Out {
    fn new() -> Self {
        Self { layers: Vec::new() }
    }
    fn put(&mut self, spec: &LayerSpec, e: Vec<EntityType>) {
        match self.layers.iter_mut().find(|(s, _)| s.name == spec.name) {
            Some((_, v)) => v.extend(e),
            None => self.layers.push((spec.clone(), e)),
        }
    }
    fn write(self, label: &str) -> Result<usize, String> {
        let mut all = Vec::new();
        let mut first_data = None;
        for (spec, e) in self.layers {
            bridge::ensure_nlcs_layer(&spec);
            if first_data.is_none() {
                first_data = e.iter().find_map(|x| x.common().extended_data.get_record(bridge::XDATA_APP).cloned());
            }
            all.extend(e);
        }
        let handles = bridge::add_entities_handles(label, all)?;
        if let (Some(h), Some(r)) = (handles.first(), first_data) {
            bridge::register_object_data(*h, r);
        }
        Ok(handles.len())
    }
}

/// RD → drawing.
fn rd_to_drawing(site: &Site, f: f64) -> Result<impl Fn(XY) -> Option<XY>, String> {
    grid_to_drawing(site, "EPSG:28992", f)
}

/// A height source's CRS → drawing.
fn grid_to_drawing(site: &Site, grid_crs: &str, f: f64) -> Result<impl Fn(XY) -> Option<XY>, String> {
    let t = vector::data_to_site(site, grid_crs, &[])?;
    let s = site.clone();
    Ok(move |p: XY| t(p).map(|q| s.to_drawing(q, f)))
}

/// Drawing → a height source's CRS.
fn drawing_to_grid(site: &Site, grid_crs: &str, f: f64, p: XY) -> Result<XY, String> {
    let q = site.from_drawing(p, f);
    if site.crs == grid_crs {
        Ok(q)
    } else {
        crs::transform(&site.crs, grid_crs, q)
    }
}

/// A height label: NAP the Dutch way for AHN, otherwise in the interface's
/// number style.
fn height_text(z: f64, decimals: usize, src: &Source) -> String {
    if src.is_ahn() || lang::current() == lang::Lang::Nl {
        nap(z, decimals)
    } else {
        format!("{:+.*}", decimals, z)
    }
}

fn target_factor() -> Result<f64, String> {
    bridge::target().map(|t| t.unit_factor).ok_or_else(|| "geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad".into())
}

// ── Spot heights ────────────────────────────────────────────────────────────

pub fn spot_heights(site: &Site, opt: &AhnOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let f = target_factor()?;
    let src = Source::for_site(site, opt.model);
    let b = src.bbox(site)?;
    let grid = src.fetch(b, src.native_cell(), progress)?;
    let to = grid_to_drawing(site, &src.crs, f)?;
    let o = &opt.nlcs;
    let k = o.scale / 1000.0 * f;
    let (s_spec, t_spec) = (layer("OG", "HOOGTEPUNT", o, Element::S)?, layer("OG", "HOOGTEPUNT", o, Element::T)?);
    let rect = site.drawing_rect(f);
    let mut out = Out::new();
    let step = opt.spot_spacing.max(1.0);
    let mut n = 0;
    let mut y = (b[1] / step).ceil() * step;
    while y <= b[3] {
        let mut x = (b[0] / step).ceil() * step;
        while x <= b[2] {
            if let (Some(z), Some(p)) = (grid.sample(x, y), to([x, y])) {
                if vector::inside(p, rect) {
                    let d = if src.is_ahn() {
                        record(&[("bron", src.label()), ("hoogte_nap", format!("{z:.3}")), ("x_rd", format!("{x:.2}")), ("y_rd", format!("{y:.2}"))])
                    } else {
                        record(&[("source", src.label()), ("height_m", format!("{z:.3}")), ("x", format!("{x:.2}")), ("y", format!("{y:.2}")), ("crs", src.crs.clone())])
                    };
                    let zz = z * f;
                    let arm = 0.8 * k;
                    let mut cross = Vec::new();
                    for (a, c) in [([p[0] - arm, p[1]], [p[0] + arm, p[1]]), ([p[0], p[1] - arm], [p[0], p[1] + arm])] {
                        let mut l = EntityType::Line(Line::from_points(Vector3::new(a[0], a[1], zz), Vector3::new(c[0], c[1], zz)));
                        let cm = l.common_mut();
                        cm.layer = s_spec.name.clone();
                        cm.extended_data.add_record(d.clone());
                        cross.push(l);
                    }
                    out.put(&s_spec, cross);
                    if o.labels {
                        let mut t = emit::text([p[0] + 1.2 * k, p[1] + 1.2 * k], &height_text(z, 2, &src), 1.8 * k, 0.0, &t_spec.name, Some(&d));
                        if let EntityType::Text(tx) = &mut t {
                            tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
                        }
                        out.put(&t_spec, vec![at_z(t, zz)]);
                    }
                    n += 1;
                }
            }
            x += step;
        }
        y += step;
    }
    if n == 0 {
        return Err(format!("{} {}", src.label(), tr("heeft geen hoogtes in het importgebied", "has no heights in the import area")));
    }
    let written = out.write(tr("Hoogtepunten", "Spot heights"))?;
    bridge::note_source(src.attribution());
    Ok(format!("{n} {} ({} m {}, {}) — {written} {} {}.", tr("hoogtepunten", "spot heights"), step, tr("raster", "grid"), src.label(), tr("elementen op", "entities on"), s_spec.name))
}

// ── Contours ────────────────────────────────────────────────────────────────

pub fn contour_lines(site: &Site, opt: &AhnOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let f = target_factor()?;
    let src = Source::for_site(site, Model::Dtm);
    let b = src.bbox(site)?;
    let mut grid = src.fetch(b, opt.contour_cell, progress)?;
    if opt.smooth {
        grid = grid.smoothed();
    }
    let range = grid.range().ok_or_else(|| format!("{} {}", src.label(), tr("heeft geen hoogtes in het importgebied", "has no heights in the import area")))?;
    progress(tr("Hoogtelijnen berekenen…", "Tracing contour lines…").into());
    // Outside AHN the heights are 30 m-ish models: a finer interval only
    // draws their noise.
    let interval = if src.is_ahn() { opt.interval.max(0.05) } else { opt.interval.max(1.0) };
    let levels = ahn::levels(range, interval);
    let min_length = (8.0 * grid.cell).max(10.0);
    let lines = ahn::contours(&grid, &levels, min_length);
    let to = grid_to_drawing(site, &src.crs, f)?;
    let o = &opt.nlcs;
    let k = o.scale / 1000.0 * f;
    let minor = layer("OG", "HOOGTELIJN_MINOR", o, Element::G)?;
    let major = layer("OG", "HOOGTELIJN_MAJOR", o, Element::G)?;
    let major_t = layer("OG", "HOOGTELIJN_MAJOR", o, Element::T)?;
    let major_step = interval * opt.major_every.max(1) as f64;
    let is_major = |l: f64| ((l / major_step).round() * major_step - l).abs() < interval * 0.01;
    let rect = site.drawing_rect(f);
    let mut out = Out::new();
    let mut count = 0;
    for (level, polys) in &lines {
        let big = is_major(*level);
        let spec = if big { &major } else { &minor };
        let d = if src.is_ahn() {
            record(&[("bron", "AHN4 dtm_05m".into()), ("hoogte_nap", format!("{level:.3}")), ("interval", format!("{interval}"))])
        } else {
            record(&[("source", src.label()), ("height_m", format!("{level:.3}")), ("interval", format!("{interval}"))])
        };
        for (pts, closed) in polys {
            let dpts: Vec<XY> = pts.iter().filter_map(|p| to(*p)).collect();
            // Clip to the import area like the other imports.
            let pieces: Vec<(Vec<XY>, bool)> = if !o.clip {
                vec![(dpts, *closed)]
            } else if *closed && dpts.iter().all(|p| vector::inside(*p, rect)) {
                vec![(dpts, true)]
            } else {
                let mut open = dpts.clone();
                if *closed {
                    if let Some(first) = open.first().copied() {
                        open.push(first);
                    }
                }
                vector::clip_line(&open, rect).into_iter().map(|l| (l, false)).collect()
            };
            for (piece, c) in pieces {
                if piece.len() < 2 {
                    continue;
                }
                let ents: Vec<EntityType> = emit::polylines(&[(piece.clone(), c)], &spec.name, k, Some(&d)).into_iter().map(|e| at_z(e, level * f)).collect();
                out.put(spec, ents);
                count += 1;
                let len: f64 = piece.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
                if big && o.labels && len > 40.0 * k {
                    if let Some((at, angle)) = emit::label_spot(&piece) {
                        let t = emit::text(at, &height_text(*level, 2, &src), 1.8 * k, emit::readable(angle), &major_t.name, Some(&d));
                        out.put(&major_t, vec![at_z(t, level * f)]);
                    }
                }
            }
        }
    }
    if count == 0 {
        return Err(format!("{} {interval} m ({} {:.2} m)", tr("geen hoogtelijnen met interval", "no contour lines at an interval of"), tr("hoogteverschil", "height range"), range.1 - range.0));
    }
    let written = out.write(tr("Hoogtelijnen", "Contour lines"))?;
    bridge::note_source(src.attribution());
    Ok(format!(
        "{count} {}, {} – {} m {} {} {} m ({} m {}, {}{}) — {written} {}.",
        tr("hoogtelijnen", "contour lines"),
        height_text(range.0, 2, &src),
        height_text(range.1, 2, &src),
        src.datum(),
        tr("om de", "every"),
        height_text(interval, 2, &src).trim_start_matches('+'),
        grid.cell,
        tr("raster", "grid"),
        src.label(),
        if opt.smooth { tr(", gladgestreken", ", smoothed") } else { "" },
        tr("elementen", "entities")
    ))
}

// ── Profiles ────────────────────────────────────────────────────────────────

static PROFILES: AtomicUsize = AtomicUsize::new(0);

/// Section letter: A, B, … Z, AA, AB, …
fn letter(n: usize) -> String {
    let mut s = String::new();
    let mut n = n + 1;
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

/// A length profile along `route` (drawing coordinates) placed with its
/// lower-left corner at `at`: ground (DTM) and optionally surface (DSM)
/// lines, a height grid, a profile band with distances and heights, and
/// the section line A–A' in plan.
pub fn profile(site: &Site, route: &[XY], at: XY, opt: &AhnOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let f = target_factor()?;
    let src = Source::for_site(site, Model::Dtm);
    let pts: Vec<XY> = route.iter().map(|p| drawing_to_grid(site, &src.crs, f, *p)).collect::<Result<_, _>>()?;
    // Only AHN has a separate surface model.
    let with_surface = opt.surface && src.is_ahn();
    let stations = src.profile(&pts, opt.step, with_surface, progress)?;
    progress(tr("Profiel tekenen…", "Drawing the profile…").into());
    let o = &opt.nlcs;
    let k = o.scale / 1000.0 * f;
    let v = opt.exaggeration.max(1.0);
    let name = letter(PROFILES.fetch_add(1, Ordering::Relaxed));
    let zs: Vec<f64> = stations.iter().flat_map(|s| [s.ground, s.surface]).flatten().collect();
    let (zmin, zmax) = zs.iter().fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
    let hstep = nice_step((zmax - zmin) / 6.0);
    let base = (zmin / hstep).floor() * hstep - hstep;
    let top = (zmax / hstep).ceil() * hstep + hstep;
    let length = stations.last().map(|s| s.chainage).unwrap_or(0.0);
    let band_rows = if with_surface { 3 } else { 2 };
    // Rows tall enough for values written upright (rotated a quarter turn).
    let row = 14.0 * k;
    let band = band_rows as f64 * row;
    let head = 42.0 * k;
    let gx = |ch: f64| at[0] + ch * f;
    let gy = |z: f64| at[1] + band + (z - base) * v * f;

    let frame = layer("AL", "PROFIELBALK", o, Element::G)?;
    let frame_t = layer("AL", "PROFIELBALK", o, Element::T)?;
    let ground = layer("GW", "MAAIVELD", o, Element::G)?;
    let surface = layer("ZZ", "HULPLIJN", o, Element::G)?;
    let nap_l = layer("ZZ", "NAP", o, Element::G)?;
    let section = layer("AL", "VERWIJZING_DOORSNEDE", o, Element::G)?;
    let section_t = layer("AL", "VERWIJZING_DOORSNEDE", o, Element::T)?;
    let mut out = Out::new();
    let pl = |pts: Vec<XY>, closed: bool, spec: &LayerSpec| emit::polylines(&[(pts, closed)], &spec.name, k, None);
    let txt = |p: XY, s: &str, mm: f64, rot: f64, spec: &LayerSpec, left: bool| {
        let mut t = emit::text(p, s, mm * k, rot, &spec.name, None);
        if left {
            if let EntityType::Text(tx) = &mut t {
                tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
            }
        }
        crate::tekenblad::text_pen(&mut t, mm);
        t
    };

    // Frame: graph and band, header column on the left.
    let (x0, x1, y0, y1) = (at[0] - head, gx(length), at[1], gy(top));
    out.put(&frame, pl(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true, &frame));
    for r in 1..=band_rows {
        let y = y0 + r as f64 * row;
        out.put(&frame, pl(vec![[x0, y], [x1, y]], false, &frame));
    }
    out.put(&frame, pl(vec![[at[0], y0], [at[0], y1]], false, &frame));
    let ground_row = if src.is_ahn() { "Maaiveld AHN4 (m NAP)".to_string() } else { format!("{} (m {})", tr("Maaiveld", "Ground level"), src.datum()) };
    let mut rows = vec![tr("Afstand (m)", "Distance (m)").to_string(), ground_row];
    if with_surface {
        rows.push("Oppervlak AHN4 (m NAP)".into());
    }
    for (r, title) in rows.iter().enumerate() {
        let y = y0 + band - (r as f64 + 0.5) * row;
        let t = txt([x0 + 2.0 * k, y], title, 1.8, 0.0, &frame_t, true);
        out.put(&frame_t, vec![t]);
    }
    // Height grid with labels.
    let mut z = base;
    while z <= top + 1e-9 {
        let y = gy(z);
        if z > base + 1e-9 && z < top - 1e-9 {
            out.put(&frame, pl(vec![[at[0], y], [x1, y]], false, &frame));
        }
        // Just above its grid line, clear of the frame; none at the top.
        if z < top - 1e-9 {
            let t = txt([at[0] - 2.0 * k, y + 1.4 * k], &height_text(z, 2, &src), 1.8, 0.0, &frame_t, false);
            let t = if let EntityType::Text(mut tx) = t {
                tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Right;
                EntityType::Text(tx)
            } else {
                t
            };
            out.put(&frame_t, vec![t]);
        }
        z += hstep;
    }
    if base < 0.0 && top > 0.0 {
        out.put(&nap_l, pl(vec![[at[0], gy(0.0)], [x1, gy(0.0)]], false, &nap_l));
        let t = txt([x1 - 2.0 * k, gy(0.0) + 1.5 * k], src.datum(), 1.8, 0.0, &frame_t, false);
        out.put(&frame_t, vec![t]);
    }
    // Distances and heights in the band at every label station.
    let label_step = nice_step(20.0 * o.scale / 1000.0).max(opt.step);
    let height_at = |ch: f64, pick: fn(&Station) -> Option<f64>| -> Option<f64> {
        stations.iter().min_by(|a, b| (a.chainage - ch).abs().total_cmp(&(b.chainage - ch).abs())).and_then(pick)
    };
    let mut ch = 0.0;
    let mut chainages = Vec::new();
    while ch < length - 1e-6 {
        chainages.push(ch);
        ch += label_step;
    }
    chainages.push(length);
    for ch in &chainages {
        let x = gx(*ch);
        out.put(&frame, pl(vec![[x, y0], [x, y0 + band]], false, &frame));
        if *ch > 1e-6 && *ch < length - 1e-6 {
            out.put(&frame, pl(vec![[x, gy(base)], [x, y1]], false, &frame));
        }
        let rot = std::f64::consts::FRAC_PI_2;
        let whole = (ch - ch.round()).abs() < 1e-6;
        let comma = src.is_ahn() || lang::current() == lang::Lang::Nl;
        let distance = if whole { format!("{ch:.0}") } else if comma { format!("{ch:.1}").replace('.', ",") } else { format!("{ch:.1}") };
        let mut values = vec![(distance, 0)];
        if let Some(z) = height_at(*ch, |s| s.ground) {
            values.push((height_text(z, 2, &src), 1));
        }
        if with_surface {
            if let Some(z) = height_at(*ch, |s| s.surface) {
                values.push((height_text(z, 2, &src), 2));
            }
        }
        for (s, r) in values {
            let y = y0 + band - (r as f64 + 0.5) * row;
            out.put(&frame_t, vec![txt([x - 1.0 * k, y], &s, 1.8, rot, &frame_t, false)]);
        }
    }
    // The profile lines, broken where AHN has no value.
    let mut add_line = |pick: fn(&Station) -> Option<f64>, spec: &LayerSpec| {
        let mut run: Vec<XY> = Vec::new();
        let flush = |run: &mut Vec<XY>, out: &mut Out| {
            if run.len() > 1 {
                out.put(spec, emit::polylines(&[(std::mem::take(run), false)], &spec.name, k, None));
            }
            run.clear();
        };
        for s in &stations {
            match pick(s) {
                Some(z) => run.push([gx(s.chainage), gy(z)]),
                None => flush(&mut run, &mut out),
            }
        }
        flush(&mut run, &mut out);
    };
    add_line(|s| s.ground, &ground);
    if with_surface {
        add_line(|s| s.surface, &surface);
    }
    // Title.
    let title = format!("{} {name} – {name}'", tr("Lengteprofiel", "Longitudinal profile"));
    out.put(&frame_t, vec![txt([x0, y1 + 9.0 * k], &title, 3.5, 0.0, &frame_t, true)]);
    let sub = if src.is_ahn() {
        format!(
            "AHN4 maaiveld (DTM 0,5 m){} · horizontaal 1:{} · verticaal 1:{} (overhoging {}×) · lengte {:.1} m",
            if with_surface { " en oppervlak (DSM)" } else { "" },
            o.scale,
            (o.scale / v).round(),
            v,
            length
        )
        .replace('.', ",")
    } else {
        format!(
            "{} · {} 1:{} · {} 1:{} ({} {}×) · {} {:.1} m",
            src.label(),
            tr("horizontaal", "horizontal"),
            o.scale,
            tr("verticaal", "vertical"),
            (o.scale / v).round(),
            tr("overhoging", "exaggeration"),
            v,
            tr("lengte", "length"),
            length
        )
    };
    out.put(&frame_t, vec![txt([x0, y1 + 4.0 * k], &sub, 1.8, 0.0, &frame_t, true)]);

    // Section line in plan, with its letters.
    let d = record(&[("bron", src.label()), ("profiel", format!("{name}-{name}'")), ("lengte_m", format!("{length:.2}"))]);
    out.put(&section, emit::polylines(&[(route.to_vec(), false)], &section.name, k, Some(&d)));
    let ends = [(route[0], route[1], name.clone()), (route[route.len() - 1], route[route.len() - 2], format!("{name}'"))];
    for (p, q, label) in ends {
        let dir = [p[0] - q[0], p[1] - q[1]];
        let len = dir[0].hypot(dir[1]).max(1e-9);
        let at = [p[0] + dir[0] / len * 4.0 * k, p[1] + dir[1] / len * 4.0 * k];
        let t = txt(at, &label, 3.5, 0.0, &section_t, false);
        out.put(&section_t, vec![t]);
    }
    let n = out.write(&format!("{} {name}", tr("Lengteprofiel", "Longitudinal profile")))?;
    bridge::note_source(src.attribution());
    let missing = stations.iter().filter(|s| s.ground.is_none()).count();
    let mut msg = format!(
        "{} {name}–{name}': {:.1} m, {} stations, {} – {} m {} ({n} {}).",
        tr("Lengteprofiel", "Longitudinal profile"),
        length,
        stations.len(),
        height_text(zmin, 2, &src),
        height_text(zmax, 2, &src),
        src.datum(),
        tr("elementen", "entities")
    );
    if missing > 0 {
        msg.push_str(&format!(" {missing} {}", tr("stations zonder maaiveldhoogte (gebouwen, water).", "stations without a ground height (buildings, water).")));
    }
    Ok(msg)
}

/// 1, 2 or 5 × 10ⁿ at or above `v`.
fn nice_step(v: f64) -> f64 {
    let v = v.max(1e-3);
    let p = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|s| *s >= v * 0.999).unwrap_or(10.0 * p)
}

// ── Point cloud ─────────────────────────────────────────────────────────────

pub fn point_cloud(site: &Site, opt: &AhnOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let f = target_factor()?;
    if !crs::in_netherlands(site.lon, site.lat) {
        return Err(tr("de AHN-puntenwolk is alleen beschikbaar in Nederland", "the AHN point cloud is only available in the Netherlands").into());
    }
    let b = rd_bbox(site)?;
    let c = &opt.cloud;
    let cloud = pointcloud::fetch(b, c, progress)?;
    progress(format!("{} punten omzetten…", cloud.xyz.len()));
    let to = rd_to_drawing(site, f)?;
    let colours = pointcloud::colours(&cloud, c.colouring);
    let mut xyz = Vec::with_capacity(cloud.xyz.len());
    let mut cols = Vec::with_capacity(cloud.xyz.len());
    let mut inten = Vec::with_capacity(cloud.xyz.len());
    for (i, p) in cloud.xyz.iter().enumerate() {
        if let Some(q) = to([p[0], p[1]]) {
            xyz.push([q[0], q[1], p[2] * f]);
            cols.push(colours[i]);
            inten.push(cloud.intensity[i]);
        }
    }
    let dir = bridge::image_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}_{:.0}_{:.0}_{:.0}x{:.0}.rcs", c.edition.short().to_lowercase(), b[0], b[1], b[2] - b[0], b[3] - b[1]));
    progress(format!("Puntenwolk schrijven ({:.0} MB)…", xyz.len() as f64 * 20.0 / 1e6));
    pointcloud::write_rcs(&file, &xyz, &cols, &inten)?;
    let path = file.to_string_lossy().into_owned();
    if path.contains(char::is_whitespace) {
        return Ok(format!("{} punten opgeslagen in {path}. Het pad bevat spaties: koppel het bestand met POINTCLOUDATTACH.", xyz.len()));
    }
    bridge::execute(&format!("-POINTCLOUDATTACH {path} 0,0,0 1 0\n"))?;
    bridge::note_source(&format!("{} puntenwolk (Het Waterschapshuis, CC0)", c.edition.short()));
    Ok(format!(
        "{} puntenwolk: {} punten{} uit {} tegel(s){}, gekoppeld als {}.",
        c.edition.short(),
        xyz.len(),
        if cloud.truncated { " (limiet bereikt)" } else { "" },
        cloud.tiles,
        if cloud.fallback_tiles > 0 { format!(" waarvan {} uit AHN4 (AHN5 nog niet beschikbaar)", cloud.fallback_tiles) } else { String::new() },
        file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ))
}

/// AHN heights at one RD point, for Punt info: (maaiveld, oppervlak).
pub fn heights_at(p: XY) -> (Option<f64>, Option<f64>) {
    let b = [p[0] - 1.0, p[1] - 1.0, p[0] + 1.0, p[1] + 1.0];
    let get = |m: Model| ahn::fetch(b, m, ahn::NATIVE_CELL, &|_| {}).ok().and_then(|g| g.sample(p[0], p[1]));
    (get(Model::Dtm), get(Model::Dsm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_steps() {
        assert_eq!(nap(2.434, 2), "+2,43");
        assert_eq!(nap(-4.5, 2), "-4,50");
        assert_eq!(letter(0), "A");
        assert_eq!(letter(25), "Z");
        assert_eq!(letter(26), "AA");
        assert_eq!(nice_step(0.3), 0.5);
        assert_eq!(nice_step(1.0), 1.0);
        assert_eq!(nice_step(7.0), 10.0);
    }
}
