//! An NLCS drawing sheet in model space around the import area: frame,
//! title block, legend of the NLCS layers used, north arrow, scale bar and
//! RD coordinate crosses — all on the NLCS "AL" (algemeen) layers and sized
//! for the plot scale.

use ocs_plugin_api::host::codec::{EntityType, LineWeight, LwPolyline, Vector2};

use crate::basisdata::NlcsOptions;
use crate::bridge;
use crate::emit;
use crate::gis::crs::XY;
use crate::gis::site::Site;
use crate::nlcs::{self, Element, LayerSpec};

#[derive(Clone, Debug)]
pub struct SheetInfo {
    pub project: String,
    pub subject: String,
    pub drawn_by: String,
    pub number: String,
    pub date: String,
    /// Data sources for the title block.
    pub sources: String,
}

impl Default for SheetInfo {
    fn default() -> Self {
        Self {
            project: String::new(),
            subject: "Situatie bestaand".into(),
            drawn_by: "Open Infra".into(),
            number: "OI-001".into(),
            date: today(),
            // Empty: the sources the imports into this drawing recorded.
            sources: String::new(),
        }
    }
}

fn today() -> String {
    chrono::Local::now().format("%d-%m-%Y").to_string()
}

/// Landscape A-formats (mm).
const PAPERS: &[(&str, f64, f64)] = &[("A3", 420.0, 297.0), ("A2", 594.0, 420.0), ("A1", 841.0, 594.0), ("A0", 1189.0, 841.0)];
/// Width of the legend / title block column (mm).
const STRIP: f64 = 185.0;
const TITLE_H: f64 = 88.0;

/// Paper format whose map area holds the site at the plot scale.
pub fn paper_for(site: &Site, scale: f64) -> (&'static str, f64, f64) {
    let (w, h) = (site.width * 1000.0 / scale, site.height * 1000.0 / scale);
    PAPERS.iter().copied().find(|(_, pw, ph)| map_area(*pw, *ph).fits(w, h)).unwrap_or(PAPERS[PAPERS.len() - 1])
}

struct Area {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Area {
    fn fits(&self, w: f64, h: f64) -> bool {
        w <= self.x1 - self.x0 && h <= self.y1 - self.y0
    }
}

/// The map part of the sheet: 20 mm binding margin left, 10 mm elsewhere,
/// the legend column on the right.
fn map_area(pw: f64, ph: f64) -> Area {
    Area { x0: 20.0, y0: 10.0, x1: pw - 10.0 - STRIP, y1: ph - 10.0 }
}

struct Sheet {
    /// Drawing position of the paper's lower-left corner.
    origin: XY,
    /// Drawing units per paper millimetre.
    k: f64,
    out: Vec<(LayerSpec, Vec<EntityType>)>,
    status: nlcs::Status,
    discipline: String,
}

impl Sheet {
    fn at(&self, x: f64, y: f64) -> XY {
        [self.origin[0] + x * self.k, self.origin[1] + y * self.k]
    }

    fn spec(&self, name: &str, el: Element) -> LayerSpec {
        let obj = nlcs::object_by_name("AL", name).or_else(|| nlcs::object_by_name("ZZ", name)).expect("NLCS AL object");
        nlcs::layer(obj, self.status, &self.discipline, el)
    }

    fn put(&mut self, spec: LayerSpec, e: Vec<EntityType>) {
        match self.out.iter_mut().find(|(s, _)| s.name == spec.name) {
            Some((_, v)) => v.extend(e),
            None => self.out.push((spec, e)),
        }
    }

    fn poly(&mut self, layer: &str, pts: &[(f64, f64)], closed: bool) {
        let spec = self.spec(layer, Element::G);
        let line: Vec<XY> = pts.iter().map(|(x, y)| self.at(*x, *y)).collect();
        let e = emit::polylines(&[(line, closed)], &spec.name, self.k, None);
        self.put(spec, e);
    }

    fn rect(&mut self, layer: &str, x0: f64, y0: f64, x1: f64, y1: f64) {
        self.poly(layer, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true);
    }

    /// Text at paper position; `left` aligns left instead of centred.
    fn text(&mut self, layer: &str, x: f64, y: f64, value: &str, mm: f64, left: bool) {
        let spec = self.spec(layer, Element::T);
        let mut e = emit::text(self.at(x, y), value, mm * self.k, 0.0, &spec.name, None);
        if left {
            if let EntityType::Text(t) = &mut e {
                t.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
                t.vertical_alignment = ocs_plugin_api::host::codec::entities::TextVerticalAlignment::Middle;
            }
        }
        text_pen(&mut e, mm);
        self.put(spec, vec![e]);
    }
}

/// NLCS text class: a text's colour and lineweight follow its height.
pub fn text_pen(e: &mut EntityType, mm: f64) {
    if let Some((color, lw)) = nlcs::text_class(mm) {
        let c = e.common_mut();
        c.color = ocs_plugin_api::host::codec::Color::Index(color);
        c.line_weight = LineWeight::from_value(lw);
    }
}

/// A round number near `v` (1, 2, 2.5 or 5 × 10ⁿ).
fn nice(v: f64) -> f64 {
    let p = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * p).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs())).unwrap_or(v)
}

pub fn draw(site: &Site, opt: &NlcsOptions, info: &SheetInfo) -> Result<String, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    let f = t.unit_factor;
    let (paper, pw, ph) = paper_for(site, opt.scale);
    let k = opt.scale / 1000.0 * f;
    let ma = map_area(pw, ph);
    let rect = site.drawing_rect(f);
    let centre = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    let origin = [centre[0] - (ma.x0 + ma.x1) / 2.0 * k, centre[1] - (ma.y0 + ma.y1) / 2.0 * k];
    let mut s = Sheet { origin, k, out: Vec::new(), status: nlcs::Status::B, discipline: opt.discipline.clone() };

    // Frame: trim line, drawing frame, map window.
    s.rect("TEKENBLAD_KADER_AFSNIJMARKERING", 0.0, 0.0, pw, ph);
    s.rect("TEKENBLAD_KADER", 20.0, 10.0, pw - 10.0, ph - 10.0);
    s.rect("TEKENBLAD_DEELKADER", ma.x0, ma.y0, ma.x1, ma.y1);
    let sx = ma.x1;
    s.poly("TEKENBLAD_DEELKADER", &[(sx, 10.0 + TITLE_H), (pw - 10.0, 10.0 + TITLE_H)], false);

    coordinate_crosses(&mut s, site, opt.scale, f, &ma);
    north_arrow(&mut s, ma.x1 - 18.0, ma.y1 - 26.0);
    scale_bar(&mut s, ma.x0 + 10.0, ma.y0 + 10.0, opt.scale);
    title_block(&mut s, sx, pw - 10.0, info, opt, paper, &site.crs, &site.label);
    let shown = legend(&mut s, sx + 5.0, pw - 15.0, 10.0 + TITLE_H + 5.0, ph - 15.0);

    let mut ents = Vec::new();
    for (spec, e) in s.out {
        bridge::ensure_nlcs_layer(&spec);
        ents.extend(e);
    }
    let n = bridge::add_entities("NLCS-tekenblad", ents)?;
    Ok(format!("Tekenblad {paper} liggend, schaal 1:{} — {n} elementen, legenda met {shown} lagen.", opt.scale))
}

fn coordinate_crosses(s: &mut Sheet, site: &Site, scale: f64, f: f64, ma: &Area) {
    // Crosses every scale/10 m (50 m at 1:500): about every 100 mm on paper.
    let step = nice(scale / 10.0);
    let arm = 2.5;
    let (origin, k) = (s.origin, s.k);
    let paper = |e: f64, n: f64| -> (f64, f64) {
        let d = site.to_drawing([e, n], f);
        ((d[0] - origin[0]) / k, (d[1] - origin[1]) / k)
    };
    let (lo, hi) = (site.from_drawing(s.at(ma.x0, ma.y0), f), site.from_drawing(s.at(ma.x1, ma.y1), f));
    let mut e = (lo[0] / step).ceil() * step;
    let mut labels_x = Vec::new();
    while e <= hi[0] {
        let mut n = (lo[1] / step).ceil() * step;
        while n <= hi[1] {
            let (x, y) = paper(e, n);
            if x - arm > ma.x0 && x + arm < ma.x1 && y - arm > ma.y0 && y + arm < ma.y1 {
                s.poly("COORDINAATPUNT", &[(x - arm, y), (x + arm, y)], false);
                s.poly("COORDINAATPUNT", &[(x, y - arm), (x, y + arm)], false);
            }
            n += step;
        }
        labels_x.push(e);
        e += step;
    }
    for e in labels_x {
        let (x, _) = paper(e, lo[1]);
        if x > ma.x0 + 8.0 && x < ma.x1 - 8.0 {
            s.text("COORDINAATPUNT", x, ma.y0 + 3.0, &format!("X {e:.0}"), 1.8, false);
        }
    }
    let mut n = (lo[1] / step).ceil() * step;
    while n <= hi[1] {
        let (_, y) = paper(lo[0], n);
        if y > ma.y0 + 6.0 && y < ma.y1 - 6.0 {
            s.text("COORDINAATPUNT", ma.x0 + 2.0, y + 2.0, &format!("Y {n:.0}"), 1.8, true);
        }
        n += step;
    }
}

fn north_arrow(s: &mut Sheet, x: f64, y: f64) {
    // Grid north (RD): straight up.
    // NLCS knows ORIENTATIE only as a symbol (S), outline included.
    let spec = s.spec("ORIENTATIE", Element::S);
    let outline: Vec<XY> = [(x, y + 10.0), (x + 4.0, y - 6.0), (x, y - 3.0), (x - 4.0, y - 6.0)].iter().map(|(a, b)| s.at(*a, *b)).collect();
    let e = emit::polylines(&[(outline, true)], &spec.name, s.k, None);
    s.put(spec.clone(), e);
    let solid: Vec<XY> = [(x, y + 10.0), (x, y - 3.0), (x + 4.0, y - 6.0)].iter().map(|(a, b)| s.at(*a, *b)).collect();
    let e = emit::polylines(&[(solid, true)], &spec.name, s.k, None);
    s.put(spec, e);
    s.text("ORIENTATIE", x, y + 14.0, "N", 3.5, false);
}

fn scale_bar(s: &mut Sheet, x: f64, y: f64, scale: f64) {
    let total_m = nice(100.0 * scale / 1000.0);
    let cells = 5;
    let cell_mm = total_m / cells as f64 * 1000.0 / scale;
    let h = 2.0;
    s.rect("TEKENBLAD_SCHAALBALK", x, y, x + cell_mm * cells as f64, y + h);
    for i in 0..cells {
        let x0 = x + cell_mm * i as f64;
        if i % 2 == 0 {
            // A filled cell: a polyline as wide as the bar.
            let spec = s.spec("TEKENBLAD_SCHAALBALK", Element::G);
            let mut lw = LwPolyline::new();
            let (a, b) = (s.at(x0, y + h / 2.0), s.at(x0 + cell_mm, y + h / 2.0));
            lw.add_point_with_bulge(Vector2::new(a[0], a[1]), 0.0);
            lw.add_point_with_bulge(Vector2::new(b[0], b[1]), 0.0);
            lw.constant_width = h * s.k;
            let mut e = EntityType::LwPolyline(lw);
            e.common_mut().layer = spec.name.clone();
            s.put(spec, vec![e]);
        }
        let v = total_m / cells as f64 * i as f64;
        s.text("TEKENBLAD_SCHAALBALK", x0, y + h + 2.5, &fmt_m(v), 1.8, false);
    }
    s.text("TEKENBLAD_SCHAALBALK", x + cell_mm * cells as f64, y + h + 2.5, &format!("{} m", fmt_m(total_m)), 1.8, false);
    s.text("TEKENBLAD_SCHAALBALK", x, y - 3.0, &format!("schaal 1:{scale}"), 1.8, true);
}

fn fmt_m(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 { format!("{v:.0}") } else { format!("{v}") }
}

#[allow(clippy::too_many_arguments)]
fn title_block(s: &mut Sheet, x0: f64, x1: f64, info: &SheetInfo, opt: &NlcsOptions, paper: &str, crs: &str, place: &str) {
    let y0 = 10.0;
    let rows = [0.0, 14.0, 26.0, 38.0, 54.0, 70.0, TITLE_H];
    for r in &rows[1..rows.len() - 1] {
        s.poly("TEKENBLAD_TITELBLOK", &[(x0, y0 + r), (x1, y0 + r)], false);
    }
    let w = x1 - x0;
    // Cells in the two lower data rows.
    for frac in [0.25, 0.5, 0.75] {
        s.poly("TEKENBLAD_TITELBLOK", &[(x0 + w * frac, y0 + rows[1]), (x0 + w * frac, y0 + rows[3])], false);
    }
    let label = |s: &mut Sheet, x: f64, y: f64, l: &str| s.text("TEKENBLAD_TITELBLOK", x + 2.0, y - 2.6, l, 1.8, true);
    let value = |s: &mut Sheet, x: f64, y: f64, v: &str, mm: f64| s.text("TEKENBLAD_TITELBLOK", x + 2.0, y, v, mm, true);

    // Bottom row: sources.
    let sources = if info.sources.is_empty() { bridge::sources().join(" · ") } else { info.sources.clone() };
    label(s, x0, y0 + rows[1], "bronnen");
    value(s, x0, y0 + 4.5, if sources.is_empty() { "—" } else { &sources }, 1.8);
    // Number / status / CRS / format.
    let cells = [("tekeningnummer", info.number.clone()), ("tekenstatus", opt.status.label().to_string()), ("coördinatenstelsel", crs.to_string()), ("formaat", format!("{paper} liggend"))];
    for (i, (l, v)) in cells.iter().enumerate() {
        let x = x0 + w * 0.25 * i as f64;
        label(s, x, y0 + rows[2], l);
        value(s, x, y0 + rows[1] + 4.0, v, 2.5);
    }
    let cells = [("schaal", format!("1:{}", opt.scale)), ("datum", info.date.clone()), ("getekend", info.drawn_by.clone()), ("discipline", opt.discipline.clone())];
    for (i, (l, v)) in cells.iter().enumerate() {
        let x = x0 + w * 0.25 * i as f64;
        label(s, x, y0 + rows[3], l);
        value(s, x, y0 + rows[2] + 4.0, v, 2.5);
    }
    label(s, x0, y0 + rows[4], "onderwerp");
    value(s, x0, y0 + rows[3] + 6.0, &info.subject, 3.5);
    label(s, x0, y0 + rows[5], "project");
    value(s, x0, y0 + rows[4] + 6.0, if info.project.is_empty() { place } else { &info.project }, 3.5);
    value(s, x0, y0 + rows[5] + 9.5, "OPEN INFRA", 5.0);
    s.text("TEKENBLAD_TITELBLOK", x1 - 2.0 - 40.0, y0 + rows[5] + 9.5, "NLCS 5.02", 2.5, true);
    s.text("TEKENBLAD_TITELBLOK", x0 + 2.0, y0 + rows[5] + 3.5, "OpenAEC Foundation — Open CAD Studio", 1.8, true);
}

/// Legend of the NLCS layers drawn in this session, grouped by main group.
/// Returns the number of layers shown.
fn legend(s: &mut Sheet, x0: f64, x1: f64, y0: f64, y1: f64) -> usize {
    let mut layers: Vec<LayerSpec> = bridge::nlcs_layers().into_iter().filter(|l| !l.name.contains("-AL-") && !l.name.ends_with("-T")).collect();
    let key = |l: &LayerSpec| {
        let parts: Vec<&str> = l.name.splitn(5, '-').collect();
        (parts.get(2).copied().unwrap_or("").to_string(), parts.get(3).copied().unwrap_or("").to_string(), l.name.clone())
    };
    layers.sort_by_key(key);
    layers.dedup_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0 == kb.0 && ka.1 == kb.1
    });
    s.text("LEGENDA", x0, y1 - 3.0, "LEGENDA", 3.5, true);
    let row = 5.0;
    let col_w = (x1 - x0) / 2.0;
    let (mut col, mut y) = (0, y1 - 12.0);
    let mut group = String::new();
    let mut shown = 0;
    for l in &layers {
        let (g, obj, _) = key(l);
        let need = if g != group { 2.0 * row } else { row };
        if y - need < y0 {
            if col == 1 {
                break;
            }
            col = 1;
            y = y1 - 12.0;
        }
        let x = x0 + col as f64 * col_w;
        if g != group {
            s.text("LEGENDA", x, y, &format!("{g} — {}", nlcs::group_name(&g)), 2.5, true);
            y -= row;
            group = g.clone();
        }
        // The sample sits on the layer itself: its colour, lineweight and
        // linetype are the layer's. Marked as sheet, so a BGT mutation or a
        // quantity take-off can leave it out.
        let (a, b) = (s.at(x + 1.0, y), s.at(x + 13.0, y));
        let mark = bridge::object_data([("bron", "tekenblad".to_string())]);
        if l.name.ends_with("-S") {
            let c = s.at(x + 7.0, y);
            s.out.push((l.clone(), emit::markers(&[c], &l.name, 0.8 * s.k, Some(&mark))));
        } else {
            s.out.push((l.clone(), emit::polylines(&[(vec![a, b], false)], &l.name, s.k, Some(&mark))));
        }
        s.text("LEGENDA", x + 16.0, y, &nlcs::humanize(&obj), 1.8, true);
        y -= row;
        shown += 1;
    }
    shown
}

/// The parts of an NLCS sheet for a sheet laid out elsewhere (the plan and
/// profile sheets of `civiel::bladen`, in a paper-space layout): the same
/// frame, title block, legend and scale bar as [`draw`], in paper
/// millimetres from `origin` at `k` drawing units per millimetre.
pub struct Parts(Sheet);

impl Parts {
    pub fn new(origin: XY, k: f64, opt: &NlcsOptions) -> Parts {
        Parts(Sheet { origin, k, out: Vec::new(), status: nlcs::Status::B, discipline: opt.discipline.clone() })
    }

    /// Trim line and drawing frame of a `w` × `h` mm sheet: a 20 mm binding
    /// margin on the left, 10 mm elsewhere.
    pub fn frame(&mut self, w: f64, h: f64) {
        self.0.rect("TEKENBLAD_KADER_AFSNIJMARKERING", 0.0, 0.0, w, h);
        self.0.rect("TEKENBLAD_KADER", 20.0, 10.0, w - 10.0, h - 10.0);
    }

    pub fn rect(&mut self, layer: &str, x0: f64, y0: f64, x1: f64, y1: f64) {
        self.0.rect(layer, x0, y0, x1, y1);
    }

    pub fn line(&mut self, layer: &str, pts: &[(f64, f64)]) {
        self.0.poly(layer, pts, false);
    }

    /// Text on an AL layer; `left` aligns left instead of centred.
    pub fn text(&mut self, layer: &str, x: f64, y: f64, value: &str, mm: f64, left: bool) {
        self.0.text(layer, x, y, value, mm, left);
    }

    /// The title block between `x0` and `x1`, 88 mm high from the frame's foot.
    #[allow(clippy::too_many_arguments)]
    pub fn title_block(&mut self, x0: f64, x1: f64, info: &SheetInfo, opt: &NlcsOptions, paper: &str, crs: &str, place: &str) {
        title_block(&mut self.0, x0, x1, info, opt, paper, crs, place);
    }

    /// Height of the title block (mm).
    pub const TITLE_H: f64 = TITLE_H;

    /// Legend of the NLCS layers drawn in this session; the number shown.
    pub fn legend(&mut self, x0: f64, x1: f64, y0: f64, y1: f64) -> usize {
        legend(&mut self.0, x0, x1, y0, y1)
    }

    /// A scale bar of about 100 mm from (`x`, `y`), labelled "schaal 1:n".
    pub fn scale_bar(&mut self, x: f64, y: f64, scale: f64) {
        scale_bar(&mut self.0, x, y, scale);
    }

    /// The entities per NLCS layer.
    pub fn finish(self) -> Vec<(LayerSpec, Vec<EntityType>)> {
        self.0.out
    }
}
