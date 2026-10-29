//! Plan and profile sheets (plan production; the Dutch bestektekening):
//! `CIV_BLADEN` divides an alignment into sheets and makes
//! one paper-space layout per sheet, with the plan strip turned along the
//! alignment in a twisted viewport, the length profile of the sheet's
//! station range in a second viewport, match lines (aansluitlijnen) with
//! their station, a north arrow, scale bars and the NLCS title block of
//! [`crate::tekenblad`] with "blad n van N".
//!
//! Open CAD Studio's plugin API (v2026.43) creates paper-space layouts
//! (`TableOperation::LayoutCreate`, `LayoutSetPage`, `LayoutSetCurrent`) and,
//! while a layout is current, `AddEntities` puts entities — viewports
//! included — on its paper space. So the sheets are real layouts with real
//! viewports. What stays in model space: the sheet outlines along the
//! alignment (view frames, AL-TEKENBLADGRENS, not plotted), the match lines
//! (AL-BLADVERWIJZING_AS) and one profile view per sheet below the project.
//!
//! `CIV_BLADEN_PDF` plots every sheet to PDF through the host's `-PLOT`.

use std::f64::consts::FRAC_PI_2;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment, Viewport};
use ocs_plugin_api::host::codec::{EntityType, Handle, Vector3};
use ocs_plugin_api::host::{ExtendedDataRecord, TableOperation, XRecordEntry, XRecordValue};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};

use super::as_profiel::{self, design_z, Alignment, Pvi, Seg};
use super::{api, nice_step, record, station, CivielOptions, Ctx, Maaiveld, Out};
use crate::bridge;
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::{Element, LayerSpec};
use crate::lang::tr;
use crate::tekenblad::{self, SheetInfo};

/// A message in the interface language, from its Dutch and English text.
fn t2(nl: String, en: String) -> String {
    tr(&nl, &en).to_string()
}

// ── Paper and sheet layout ──────────────────────────────────────────────────

/// A landscape A-format sheet (mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub name: &'static str,
    pub w: f64,
    pub h: f64,
}

pub const PAPERS: [Paper; 4] = [
    Paper { name: "A0", w: 1189.0, h: 841.0 },
    Paper { name: "A1", w: 841.0, h: 594.0 },
    Paper { name: "A2", w: 594.0, h: 420.0 },
    Paper { name: "A3", w: 420.0, h: 297.0 },
];

impl Paper {
    pub fn parse(s: &str) -> Option<Paper> {
        PAPERS.iter().copied().find(|p| p.name.eq_ignore_ascii_case(s.trim()))
    }

    /// Width of the title column on the right: the 185 mm of the NLCS title
    /// block of `GIS_TEKENBLAD`, narrower on A3 so the views keep their width.
    pub fn column(&self) -> f64 {
        if self.w < 500.0 {
            125.0
        } else {
            185.0
        }
    }
}

/// A rectangle on the sheet (paper mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn centre(&self) -> XY {
        [(self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0]
    }
}

/// Where the views go on a sheet: the plan above, the profile below it, both
/// as wide; the title column on the right.
#[derive(Clone, Copy, Debug)]
pub struct SheetLayout {
    pub plan: Rect,
    pub profile: Rect,
    pub column: Rect,
}

/// Room above a view for its title (mm).
const VIEW_TITLE: f64 = 9.0;

pub fn sheet_layout(p: &Paper) -> SheetLayout {
    // The NLCS frame: 20 mm binding margin left, 10 mm elsewhere.
    let (fx0, fy0, fx1, fy1) = (20.0, 10.0, p.w - 10.0, p.h - 10.0);
    let cx0 = fx1 - p.column();
    // A3 gives the profile a bit more: its band takes the same millimetres.
    let split = fy0 + (fy1 - fy0) * if p.w < 500.0 { 0.5 } else { 0.44 };
    SheetLayout {
        plan: Rect { x0: fx0 + 5.0, y0: split + 5.0, x1: cx0 - 5.0, y1: fy1 - 5.0 - VIEW_TITLE },
        profile: Rect { x0: fx0 + 5.0, y0: fy0 + 5.0, x1: cx0 - 5.0, y1: split - 5.0 - VIEW_TITLE },
        column: Rect { x0: cx0, y0: fy0, x1: fx1, y1: fy1 },
    }
}

impl SheetLayout {
    /// The profile as tall as it needs (`mm`, at most its room); the plan
    /// takes the rest.
    pub fn with_profile_height(mut self, mm: f64) -> SheetLayout {
        let h = mm.clamp(10.0, self.profile.h());
        self.profile.y1 = self.profile.y0 + h;
        self.plan.y0 = self.profile.y1 + VIEW_TITLE + 5.0;
        self
    }
}

/// The rows of the profile band, from the bottom up.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Alignement,
    Helling,
    Ontwerp,
    Maaiveld,
    Stationering,
}

impl Row {
    fn title(self) -> &'static str {
        match self {
            Row::Alignement => tr("Alignement", "Alignment"),
            Row::Helling => tr("Helling / boog", "Grade / curve"),
            Row::Ontwerp => tr("Ontwerp (m NAP)", "Design (m)"),
            Row::Maaiveld => tr("Maaiveld (m NAP)", "Ground (m)"),
            Row::Stationering => tr("Stationering", "Station"),
        }
    }

    /// Height on paper (mm): vertical numbers need about 12 mm.
    fn mm(self) -> f64 {
        match self {
            Row::Alignement => 7.0,
            Row::Helling => 8.0,
            Row::Ontwerp | Row::Maaiveld => 12.0,
            Row::Stationering => 14.0,
        }
    }
}

fn rows(design: bool) -> Vec<Row> {
    if design {
        vec![Row::Alignement, Row::Helling, Row::Ontwerp, Row::Maaiveld, Row::Stationering]
    } else {
        vec![Row::Alignement, Row::Maaiveld, Row::Stationering]
    }
}

/// Width of the column with the row titles and height labels, left of the graph (mm).
const HEADER_MM: f64 = 34.0;

// ── Dividing the alignment into sheets ──────────────────────────────────────

/// One sheet's stretch of the alignment.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Distance along the axis (drawing units) where the sheet starts and
    /// ends: the match lines.
    pub s0: f64,
    pub s1: f64,
    /// Direction of the plan strip (radians): it is shown pointing right.
    pub angle: f64,
    /// Model point at the centre of the plan window.
    pub centre: XY,
}

/// What the sheet division has to respect, in drawing units.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The plan window (the viewport at the plot scale).
    pub w: f64,
    pub h: f64,
    /// Shown beyond the match lines, in total (half on each side).
    pub overlap: f64,
    /// Longest stretch per sheet: the width of the profile graph.
    pub max_len: f64,
    /// Match lines on stations that are a multiple of this (0: anywhere).
    pub round: f64,
    /// Station at the start of the axis.
    pub start: f64,
}

/// Free room across the axis on each side, as a share of the window height.
const SIDE: f64 = 0.12;
/// Free room at both ends along the axis, as a share of the window width.
const END: f64 = 0.02;

/// The window of a stretch `a`..`b`: turned along its chord and centred on
/// the axis between the two match lines plus the overlap. Returns the span
/// and the extent of the axis in the window, along and across.
fn window(al: &Alignment, a: f64, b: f64, fr: &Frame) -> (Span, f64, f64) {
    let len = al.length();
    let (lo, hi) = ((a - fr.overlap / 2.0).max(0.0), (b + fr.overlap / 2.0).min(len));
    let (pa, da) = al.at(a);
    let (pb, _) = al.at(b);
    let chord = [pb[0] - pa[0], pb[1] - pa[1]];
    let angle = if chord[0].hypot(chord[1]) > fr.w * 1e-6 { chord[1].atan2(chord[0]) } else { da };
    let (c, s) = (angle.cos(), angle.sin());
    let n = (((hi - lo) / (fr.w / 400.0)).ceil() as usize).clamp(8, 4000);
    let mut ext = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for i in 0..=n {
        let (p, _) = al.at(lo + (hi - lo) * i as f64 / n as f64);
        let (u, v) = (p[0] * c + p[1] * s, -p[0] * s + p[1] * c);
        ext = [ext[0].min(u), ext[1].min(v), ext[2].max(u), ext[3].max(v)];
    }
    let (um, vm) = ((ext[0] + ext[2]) / 2.0, (ext[1] + ext[3]) / 2.0);
    let centre = [um * c - vm * s, um * s + vm * c];
    (Span { s0: a, s1: b, angle, centre }, ext[2] - ext[0], ext[3] - ext[1])
}

fn fits(al: &Alignment, a: f64, b: f64, fr: &Frame) -> bool {
    let (_, along, across) = window(al, a, b, fr);
    let eps = fr.w * 1e-9;
    along <= fr.w * (1.0 - 2.0 * END) + eps && across <= fr.h * (1.0 - 2.0 * SIDE) + eps
}

/// Divide the alignment into sheets: each stretch as long as its window and
/// the profile allow, then spread evenly when that fits too, with the match
/// lines on round stations. A curve tighter than a window gets short sheets.
pub fn divide(al: &Alignment, fr: &Frame) -> Vec<Span> {
    let len = al.length();
    let tol = (fr.w / 4000.0).max(1e-9);
    let longest = fr.max_len.min(fr.w * (1.0 - 2.0 * END) - fr.overlap).max(tol * 20.0);
    let rounded = |b: f64, a: f64| -> f64 {
        if fr.round <= 0.0 {
            return b;
        }
        let r = ((fr.start + b) / fr.round).floor() * fr.round - fr.start;
        if r > a + fr.round.min(longest / 4.0) {
            r
        } else {
            b
        }
    };
    // Greedy: as far as fits.
    let mut cuts = vec![0.0];
    let mut a = 0.0;
    while len - a > tol {
        let hi = (len - a).min(longest);
        let d = if fits(al, a, a + hi, fr) {
            hi
        } else {
            let (mut lo, mut up) = (0.0, hi);
            while up - lo > tol {
                let mid = (lo + up) / 2.0;
                if fits(al, a, a + mid, fr) {
                    lo = mid
                } else {
                    up = mid
                }
            }
            // Nothing fits (a curve tighter than the window): move on anyway.
            if lo < tol * 20.0 {
                (longest / 10.0).min(len - a)
            } else {
                lo
            }
        };
        let mut b = a + d;
        if len - b > tol {
            b = rounded(b, a);
        } else {
            b = len;
        }
        cuts.push(b);
        a = b;
    }
    // Spread evenly over as many sheets, when every one of them fits.
    let n = cuts.len() - 1;
    if n > 1 {
        let mut even = vec![0.0];
        for i in 1..n {
            let b = len * i as f64 / n as f64;
            let b = if fr.round > 0.0 { ((fr.start + b) / fr.round).round() * fr.round - fr.start } else { b };
            even.push(b);
        }
        even.push(len);
        let ok = even.windows(2).all(|w| w[1] - w[0] > tol && w[1] - w[0] <= longest + tol && fits(al, w[0], w[1], fr));
        if ok {
            cuts = even;
        }
    }
    cuts.windows(2).map(|w| window(al, w[0], w[1], fr).0).collect()
}

// ── Profile heights ─────────────────────────────────────────────────────────

/// Ground heights along the axis: distance (drawing units), height (m).
pub struct Samples {
    pub s: Vec<f64>,
    pub z: Vec<Option<f64>>,
}

impl Samples {
    /// Height at `s`, linear between the samples; none in a gap.
    pub fn at(&self, s: f64) -> Option<f64> {
        let i = self.s.partition_point(|x| *x < s);
        if i < self.s.len() && (self.s[i] - s).abs() < 1e-9 {
            return self.z[i];
        }
        if i == 0 || i >= self.s.len() {
            return None;
        }
        let (a, b) = (self.z[i - 1]?, self.z[i]?);
        let t = (s - self.s[i - 1]) / (self.s[i] - self.s[i - 1]);
        Some(a + (b - a) * t)
    }
}

fn sample(al: &Alignment, ground: &mut Maaiveld, step: f64, f: f64) -> Samples {
    let len = al.length();
    let mut s: Vec<f64> = Vec::new();
    let mut x = 0.0;
    while x < len {
        s.push(x);
        x += step;
    }
    s.push(len);
    s.extend(al.marks().into_iter().map(|m| m.s));
    s.sort_by(f64::total_cmp);
    s.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let z = s.iter().map(|d| ground.height(al.at(*d).0, f)).collect();
    Samples { s, z }
}

/// Lowest and highest point of ground and design in `s0`..`s1` (m).
fn height_range(samples: &Samples, design: Option<&[Pvi]>, s0: f64, s1: f64, f: f64) -> Option<(f64, f64)> {
    let mut lo = f64::MAX;
    let mut hi = f64::MIN;
    let mut eat = |z: f64| {
        lo = lo.min(z);
        hi = hi.max(z);
    };
    for (s, z) in samples.s.iter().zip(&samples.z) {
        if *s >= s0 - 1e-9 && *s <= s1 + 1e-9 {
            if let Some(z) = z {
                eat(*z);
            }
        }
    }
    if let Some(p) = design {
        let (d0, d1) = (p[0].s * f, p[p.len() - 1].s * f);
        let n = 64;
        for i in 0..=n {
            let s = s0 + (s1 - s0) * i as f64 / n as f64;
            if s >= d0 && s <= d1 {
                eat(design_z(p, s / f));
            }
        }
        for v in p.iter().filter(|v| v.s * f >= s0 && v.s * f <= s1) {
            eat(v.z);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// The vertical scale of the profiles: the wanted exaggeration, or the next
/// smaller usual one (vertical 1:n stays a round scale) at which every
/// sheet's profile fits `avail_mm` of graph height.
#[derive(Clone, Debug, PartialEq)]
pub struct Vertical {
    pub v: f64,
    /// Height grid (m).
    pub step: f64,
    /// Bottom and top of each sheet's graph (m).
    pub ranges: Vec<(f64, f64)>,
    /// False when even 1× is too tall: the profile is cut off at the top.
    pub fits: bool,
}

pub fn fit_vertical(ranges: &[Option<(f64, f64)>], avail_mm: f64, scale: f64, wanted: f64) -> Vertical {
    let wanted = wanted.max(1.0);
    let mut candidates = vec![wanted];
    candidates.extend([100.0, 50.0, 25.0, 20.0, 10.0, 5.0, 4.0, 2.0, 1.0].into_iter().filter(|v| *v < wanted - 1e-9));
    let try_v = |v: f64| -> (f64, Vec<(f64, f64)>, bool) {
        // Paper millimetres per metre of height.
        let mm = v * 1000.0 / scale;
        let step = nice_step(8.0 / mm);
        let (head, foot) = (6.0 / mm, 2.0 / mm);
        let mut ok = true;
        let out = ranges
            .iter()
            .map(|r| {
                let (lo, hi) = r.unwrap_or((0.0, 1.0));
                let base = ((lo - foot) / step).floor() * step;
                let top = (((hi + head) / step).ceil() * step).max(base + 2.0 * step);
                if (top - base) * mm > avail_mm + 1e-9 {
                    ok = false;
                }
                (base, top)
            })
            .collect();
        (step, out, ok)
    };
    for v in &candidates {
        let (step, out, ok) = try_v(*v);
        if ok {
            return Vertical { v: *v, step, ranges: out, fits: true };
        }
    }
    let (step, out, _) = try_v(1.0);
    Vertical { v: 1.0, step, ranges: out, fits: false }
}

// ── The command ─────────────────────────────────────────────────────────────

/// `CIV_BLADEN`: what to make.
#[derive(Clone, Debug)]
pub struct Request {
    pub axis: Option<Handle>,
    pub paper: Paper,
    /// Plan scale: 1000 for 1:1000.
    pub scale: f64,
    /// Wanted vertical exaggeration of the profiles.
    pub exaggeration: f64,
    /// Overlap beyond the match lines (m); 20 mm on paper when not given.
    pub overlap: Option<f64>,
    pub project: Option<String>,
    pub subject: Option<String>,
    /// Drawing number; the sheets get -01, -02, …
    pub number: Option<String>,
}

/// "1:500", "500" or "1/500" → 500.
pub fn parse_scale(s: &str) -> Option<f64> {
    let t = s.trim().replace(',', ".");
    let n = match t.split_once([':', '/']) {
        Some((a, b)) => b.trim().parse::<f64>().ok()? / a.trim().parse::<f64>().ok()?,
        None => t.parse::<f64>().ok()?,
    };
    (n.is_finite() && n >= 1.0).then_some(n)
}

/// A layout name for the host: no characters it refuses, no spaces (the
/// `-PLOT` prompts split on them). A sheet set is an axis and a paper size,
/// so an A1 and an A3 set of one axis live side by side.
pub fn layout_name(axis: &str, paper: &str, n: usize) -> String {
    let clean: String = axis.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    let clean = clean.trim_matches('_');
    format!("{}_{paper}_blad_{n:02}", if clean.is_empty() { "As" } else { clean })
}

fn table(op: TableOperation) -> Result<Handle, String> {
    match bridge::request(PluginRequest::TableOperation { operation: op })? {
        PluginResponse::TableResult(r) => r,
        other => Err(t2(format!("onverwacht antwoord: {other:?}"), format!("unexpected answer: {other:?}"))),
    }
}

/// Back to model space when the sheets are done, also after an error.
struct BackToModel;

impl Drop for BackToModel {
    fn drop(&mut self) {
        let _ = table(TableOperation::LayoutSetCurrent { name: "Model".into() });
    }
}

// The sheets made per axis, in the drawing (NOD OPENINFRA/BLADEN):
// "axis\tlayout\tpaper".
const SHEETS_RECORD: &str = "BLADEN";

fn sheets_read() -> Vec<(String, String, String)> {
    let path = vec!["OPENINFRA".to_string()];
    let Ok(PluginResponse::XRecordEntries(Ok(entries))) = bridge::request(PluginRequest::ReadXRecord { dictionary_path: path, name: SHEETS_RECORD.into() }) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|e| match e.value {
            XRecordValue::String(s) => {
                let mut p = s.split('\t');
                Some((p.next()?.to_string(), p.next()?.to_string(), p.next().unwrap_or("A1").to_string()))
            }
            _ => None,
        })
        .collect()
}

fn sheets_write(all: &[(String, String, String)]) {
    let path = vec!["OPENINFRA".to_string()];
    let mut entries: Vec<XRecordEntry> = all.iter().map(|(a, l, p)| XRecordEntry::string(1, format!("{a}\t{l}\t{p}"))).collect();
    if entries.is_empty() {
        entries.push(XRecordEntry::string(1, "-\t-\t-".to_string()));
    }
    let update = PluginRequest::UpdateXRecord { dictionary_path: path.clone(), name: SHEETS_RECORD.into(), entries: entries.clone() };
    if !matches!(bridge::request(update), Ok(PluginResponse::XRecordUpdateResult(Ok(_)))) {
        let _ = bridge::request(PluginRequest::CreateXRecord { dictionary_path: path, name: SHEETS_RECORD.into(), entries });
    }
}

/// A text with its alignment.
pub(crate) fn aligned(mut t: EntityType, h: TextHorizontalAlignment, v: TextVerticalAlignment) -> EntityType {
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = h;
        tx.vertical_alignment = v;
    }
    t
}

fn rotate(p: XY, a: f64) -> XY {
    let (c, s) = (a.cos(), a.sin());
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

fn add(a: XY, b: XY) -> XY {
    [a[0] + b[0], a[1] + b[1]]
}

fn scaled(a: XY, k: f64) -> XY {
    [a[0] * k, a[1] * k]
}

fn st(al: &Alignment, s: f64, f: f64) -> String {
    station(al.start + s / f)
}

/// One sheet's profile view in model space.
struct View {
    s0: f64,
    s1: f64,
    base: f64,
    top: f64,
    /// Lower-left corner of the band block.
    org: XY,
    /// Width and height of band and graph (drawing units), title left out.
    size: [f64; 2],
}

/// `CIV_BLADEN`: the sheet set of an alignment.
pub fn create(opt: &CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let mut opt = opt.clone();
    opt.nlcs.scale = req.scale;
    let ctx = Ctx::new(&opt)?;
    let (f, k) = (ctx.f, ctx.k);
    let doc = super::document(&["tin", "as", "ontwerpprofiel"], &req.axis.into_iter().collect::<Vec<_>>())?;
    let (al, _, _) = as_profiel::axis(&doc, req.axis)?;
    let name = if al.name.is_empty() { "As".to_string() } else { al.name.clone() };
    let design: Option<Vec<Pvi>> = super::corridor::profile_of(&doc, &al.name);
    let lay = sheet_layout(&req.paper);
    let rows = rows(design.is_some());
    let bands_mm: f64 = rows.iter().map(|r| r.mm()).sum();
    let graph_mm = lay.profile.w() - HEADER_MM - 6.0;
    let overlap = req.overlap.map(|m| m.max(0.0) * f).unwrap_or(20.0 * k);
    let frame = Frame {
        w: lay.plan.w() * k,
        h: lay.plan.h() * k,
        overlap,
        max_len: graph_mm * k,
        round: nice_step(10.0 * req.scale / 1000.0) * f,
        start: al.start * f,
    };
    let spans = divide(&al, &frame);
    let n = spans.len();
    if n > 99 {
        return Err(t2(format!("{n} bladen is te veel: kies een kleinere schaal of een groter formaat"), format!("{n} sheets is too many: choose a smaller scale or a larger paper size")));
    }
    progress(t2(format!("{name}: {n} bladen {} 1:{}; hoogtes langs de as …", req.paper.name, req.scale), format!("{name}: {n} sheets {} 1:{}; heights along the alignment …", req.paper.name, req.scale)));
    let mut ground = Maaiveld::find(&ctx, &doc)?;
    let samples = sample(&al, &mut ground, opt.step.max(0.25) * f, f);
    if samples.z.iter().all(Option::is_none) {
        return Err(ground.error().unwrap_or_else(|| t2(format!("geen hoogtes langs de as ({})", ground.label()), format!("no heights along the alignment ({})", ground.label()))));
    }
    ground.note_source();
    let heights: Vec<Option<(f64, f64)>> = spans.iter().map(|sp| height_range(&samples, design.as_deref(), sp.s0, sp.s1, f)).collect();
    let avail = lay.profile.h() - bands_mm - 6.0;
    let mut vert = fit_vertical(&heights, avail, req.scale, req.exaggeration);
    // One graph height on every sheet: each keeps its own base.
    let span = vert.ranges.iter().map(|(b, t)| t - b).fold(0.0, f64::max);
    for r in vert.ranges.iter_mut() {
        r.1 = r.0 + span;
    }
    let lay = lay.with_profile_height(bands_mm + span * vert.v * 1000.0 / req.scale + 4.0);

    // Model space: view frames, match lines, profile views.
    let set = format!("{name} {}", req.paper.name);
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(kind, key, _)| kind == "bladen" && *key == set).flat_map(|(_, _, h)| h).collect();
    if !old.is_empty() {
        let _ = bridge::remove_entities(&t2(format!("Bladen {name} vervangen"), format!("Replace sheets {name}")), &old);
    }
    let mut out = Out::default();
    // The plan grew into the room the profile leaves: the same centres.
    let frame = Frame { h: lay.plan.h() * k, ..frame };
    let grens = ctx.layer("AL", "TEKENBLADGRENS", Element::G)?;
    let grens_t = ctx.layer("AL", "TEKENBLADGRENS", Element::T)?;
    let match_g = ctx.layer("AL", "BLADVERWIJZING_AS", Element::G)?;
    let match_t = ctx.layer("AL", "BLADVERWIJZING_AS", Element::T)?;
    for (i, sp) in spans.iter().enumerate() {
        let d = record(&[("bladen", set.clone()), ("as", al.name.clone()), ("blad", format!("{} van {n}", i + 1)), ("van", st(&al, sp.s0, f)), ("tot", st(&al, sp.s1, f))]);
        let (hw, hh) = (frame.w / 2.0, frame.h / 2.0);
        let corners: Vec<XY> = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].iter().map(|c| add(sp.centre, rotate(*c, sp.angle))).collect();
        out.put(&grens, crate::emit::polylines(&[(corners, true)], &grens.name, k, Some(&d)));
        let at = add(sp.centre, rotate([-hw + 4.0 * k, hh - 5.0 * k], sp.angle));
        let t = ctx.text(at, &t2(format!("BLAD {} VAN {n}", i + 1), format!("SHEET {} OF {n}", i + 1)), 3.5, sp.angle, &grens_t, None);
        out.put(&grens_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    }
    let mut matches = Vec::new();
    for sp in spans.iter().take(n.saturating_sub(1)) {
        let (p, dir) = al.at(sp.s1);
        let nrm = [-dir.sin(), dir.cos()];
        let half = frame.h * (0.5 - SIDE / 2.0);
        let label = st(&al, sp.s1, f);
        let d = record(&[("aansluitlijn", label.clone()), ("as", al.name.clone()), ("bladen", set.clone())]);
        out.put(&match_g, crate::emit::polylines(&[(vec![add(p, scaled(nrm, -half)), add(p, scaled(nrm, half))], false)], &match_g.name, k, Some(&d)));
        // Along the line, near its upper end, just before it.
        let at = add(add(p, scaled(nrm, half - 30.0 * k)), scaled([dir.cos(), dir.sin()], -2.0 * k));
        let t = ctx.text(at, &format!("{} {label}", tr("AANSLUITLIJN", "MATCH LINE")), 2.5, dir + FRAC_PI_2, &match_t, None);
        out.put(&match_t, [t]);
        matches.push(al.start + sp.s1 / f);
    }
    // Profile views in a column below the project, the first on top.
    let hw = HEADER_MM * k;
    let sizes: Vec<[f64; 2]> =
        spans.iter().zip(&vert.ranges).map(|(sp, (base, top))| [hw + (sp.s1 - sp.s0) + 4.0 * k, bands_mm * k + (top - base) * vert.v * f + 16.0 * k]).collect();
    let gap = 15.0 * k;
    let total = [sizes.iter().map(|s| s[0]).fold(0.0, f64::max), sizes.iter().map(|s| s[1] + gap).sum::<f64>()];
    // Below the project and below every plan window, so no plan shows them.
    let windows = spans.iter().flat_map(|sp| {
        let (hw, hh) = (frame.w / 2.0, frame.h / 2.0);
        [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|c| add(sp.centre, rotate(c, sp.angle)))
    });
    let wb = windows.fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let ca = as_profiel::clear_area(&ctx, &al);
    let area = [ca[0].min(wb[0]), ca[1].min(wb[1]), ca[2].max(wb[2]), ca[3].max(wb[3])];
    let mut spot = bridge::layout_spot("onder", total, area, 30.0 * k);
    let lowest = wb[1] - 30.0 * k;
    if spot[1] + total[1] > lowest {
        // The cursor of earlier views sits inside a plan window: go lower,
        // and move the cursor along.
        let shift = spot[1] + total[1] - lowest;
        spot[1] -= shift;
        bridge::layout_spot("onder", [total[0], shift], area, 0.0);
    }
    let mut y = spot[1] + total[1];
    let mut views = Vec::new();
    for (i, (sp, (base, top))) in spans.iter().zip(&vert.ranges).enumerate() {
        y -= sizes[i][1] + gap;
        let view = View { s0: sp.s0, s1: sp.s1, base: *base, top: *top, org: [spot[0], y], size: [sizes[i][0], sizes[i][1] - 16.0 * k] };
        let (a, b) = (st(&al, sp.s0, f), st(&al, sp.s1, f));
        let title = t2(format!("Lengteprofiel {name} — blad {} van {n} ({a} – {b})", i + 1), format!("Profile {name} — sheet {} of {n} ({a} – {b})", i + 1));
        draw_profile(&ctx, &al, &samples, design.as_deref(), &view, &rows, &vert, &title, &mut out)?;
        views.push(view);
    }
    let handles = out.write_handles(&t2(format!("Bladen {name}"), format!("Sheets {name}")))?;
    bridge::index_add("bladen", &set, &handles, true);
    // The view frames are for model space only: they do not plot.
    for spec in [&grens, &grens_t] {
        let _ = bridge::request(PluginRequest::ModifyLayer(ocs_plugin_api::host::LayerConfig { name: spec.name.clone(), plottable: Some(false), ..Default::default() }));
    }

    // Paper space: one layout per sheet.
    progress(t2(format!("{name}: {n} layouts …"), format!("{name}: {n} layouts …")));
    let mut record_all = sheets_read();
    let same = |a: &str, p: &str| a == name && p == req.paper.name;
    for (_, l, _) in record_all.iter().filter(|(a, _, p)| same(a, p)) {
        let _ = table(TableOperation::LayoutDelete { name: l.clone() });
    }
    record_all.retain(|(a, _, p)| !same(a, p) && a != "-");
    let crs = ctx.site.as_ref().map(|s| s.crs.clone()).unwrap_or_else(|| "—".into());
    let place = ctx.site.as_ref().map(|s| s.label.clone()).unwrap_or_default();
    let mut layouts = Vec::new();
    let made = (|| -> Result<(), String> {
        let _back = BackToModel;
        for (i, view) in views.iter().enumerate() {
            let lname = layout_name(&name, req.paper.name, i + 1);
            let _ = table(TableOperation::LayoutDelete { name: lname.clone() });
            table(TableOperation::LayoutCreate { name: lname.clone() }).map_err(|e| format!("layout {lname}: {e}"))?;
            record_all.push((name.clone(), lname.clone(), req.paper.name.to_string()));
            layouts.push(lname.clone());
            table(TableOperation::LayoutSetPage { name: lname.clone(), paper_size: Some([req.paper.w, req.paper.h]), rotation: Some(0), scale: Some([1.0, 1.0]) })
                .map_err(|e| format!("layout {lname}: {e}"))?;
            table(TableOperation::LayoutSetCurrent { name: lname.clone() }).map_err(|e| format!("layout {lname}: {e}"))?;
            let info = SheetInfo {
                project: req.project.clone().unwrap_or_default(),
                subject: req.subject.clone().unwrap_or_else(|| t2(format!("{name} — situatie en lengteprofiel"), format!("{name} — plan and profile"))),
                number: format!("{}-{:02}", req.number.clone().unwrap_or_else(|| "OI-BL".into()), i + 1),
                ..SheetInfo::default()
            };
            let mut paper = Out::default();
            sheet(&ctx, &req, &lay, &al, &spans, i, view, &vert, &info, &crs, &place, &mut paper)?;
            paper.write(&t2(format!("Blad {} {name}", i + 1), format!("Sheet {} {name}", i + 1)))?;
        }
        Ok(())
    })();
    // The layouts made so far are recorded even after an error, so a next
    // run replaces them.
    sheets_write(&record_all);
    made?;

    api::note("bladen", n);
    api::note("set", set.clone());
    api::note("layouts", layouts.clone());
    api::note("formaat", req.paper.name);
    api::note("schaal", req.scale);
    api::note("vschaal", vert.v);
    api::note("vschaal_gevraagd", req.exaggeration);
    api::note("profiel_past", vert.fits);
    api::note("aansluitlijnen", matches.clone());
    api::note("modelruimte", handles.len());
    api::note(
        "bladindeling",
        spans
            .iter()
            .enumerate()
            .map(|(i, sp)| {
                serde_json::json!({
                    "blad": i + 1,
                    "van": al.start + sp.s0 / f,
                    "tot": al.start + sp.s1 / f,
                    "hoek_graden": sp.angle.to_degrees(),
                    "midden": [sp.centre[0], sp.centre[1]],
                    "layout": layouts.get(i),
                })
            })
            .collect::<Vec<_>>(),
    );
    let (paper_name, vs) = (req.paper.name, (req.scale / vert.v).round());
    let at = if matches.is_empty() { "—".to_string() } else { matches.iter().map(|m| station(*m)).collect::<Vec<_>>().join(", ") };
    let list = layouts.join(", ");
    let mut msg = t2(
        format!("{name}: {n} blad(en) {paper_name} 1:{} (lengteprofiel verticaal 1:{vs}), aansluitlijnen op {at} — layouts {list}.", req.scale),
        format!("{name}: {n} sheet(s) {paper_name} 1:{} (profile vertical 1:{vs}), match lines at {at} — layouts {list}.", req.scale),
    );
    if vert.v < req.exaggeration - 1e-9 {
        let (a, b) = (req.exaggeration, vert.v);
        msg.push_str(&t2(format!(" Overhoging {a}× verlaagd naar {b}× zodat het profiel op het blad past."), format!(" Vertical exaggeration {a}× lowered to {b}× so the profile fits the sheet.")));
    }
    if !vert.fits {
        msg.push_str(tr(" Let op: het profiel is hoger dan het kader en wordt afgesneden.", " Note: the profile is taller than its frame and is cut off."));
    }
    Ok(msg)
}

/// A profile view for one sheet's stretch in model space (at the plan
/// scale): the band with stations, ground and design heights, grades and
/// vertical curves, the alignment; the graph with height grid, ground line,
/// design line, PVIs and tangent points. Same layers as the length profile
/// of `CIV_LENGTEPROFIEL`.
#[allow(clippy::too_many_arguments)]
fn draw_profile(ctx: &Ctx, al: &Alignment, samples: &Samples, design: Option<&[Pvi]>, view: &View, rows: &[Row], vert: &Vertical, title: &str, out: &mut Out) -> Result<(), String> {
    let (f, k) = (ctx.f, ctx.k);
    let v = vert.v;
    let frame = ctx.layer("AL", "PROFIELBALK", Element::G)?;
    let frame_t = ctx.layer("AL", "PROFIELBALK", Element::T)?;
    let ground_l = ctx.layer("GW", "MAAIVELD", Element::G)?;
    let design_l = ctx.layer("GW", "HOOGTE_AFWERKHOOGTE", Element::G)?;
    let design_t = ctx.layer("GW", "HOOGTE_AFWERKHOOGTE", Element::T)?;
    let help = ctx.layer("ZZ", "CONSTRUCTIELIJN", Element::G)?;
    let tp = ctx.layer("AM", "TANGENTPUNT", Element::G)?;
    let pl = |pts: Vec<XY>, spec: &LayerSpec| crate::emit::polylines(&[(pts, false)], &spec.name, k, None);
    let hw = HEADER_MM * k;
    let bh: f64 = rows.iter().map(|r| r.mm()).sum::<f64>() * k;
    let gx = |s: f64| view.org[0] + hw + (s - view.s0);
    let gy = |z: f64| view.org[1] + bh + (z - view.base) * v * f;
    let (x0, x1, y0, y1) = (view.org[0], gx(view.s1), view.org[1], gy(view.top));
    let row_y = |row: Row| -> (f64, f64) {
        let mut y = y0;
        for r in rows {
            if *r == row {
                return (y, y + r.mm() * k);
            }
            y += r.mm() * k;
        }
        (y0, y0)
    };
    let data = record(&[("bladprofiel", title.to_string()), ("as", al.name.clone()), ("van", st(al, view.s0, f)), ("tot", st(al, view.s1, f)), ("overhoging", format!("{v}"))]);
    out.put(&frame, crate::emit::polylines(&[(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], true)], &frame.name, k, Some(&data)));
    out.put(&frame, pl(vec![[gx(view.s0), y0], [gx(view.s0), y1]], &frame));
    let mut yy = y0;
    for r in rows {
        let (a, b) = (yy, yy + r.mm() * k);
        out.put(&frame, pl(vec![[x0, b], [x1, b]], &frame));
        let t = ctx.text([x0 + 2.0 * k, (a + b) / 2.0], r.title(), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
        yy = b;
    }
    // Height grid with NAP labels left of the graph.
    let mut z = view.base;
    while z <= view.top + 1e-9 {
        let y = gy(z);
        if z > view.base + 1e-9 && z < view.top - 1e-9 {
            out.put(&frame, pl(vec![[gx(view.s0), y], [x1, y]], &frame));
        }
        let t = ctx.text([gx(view.s0) - 1.5 * k, y], &nap(z, 2), 1.8, 0.0, &frame_t, None);
        out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Right, TextVerticalAlignment::Middle)]);
        z += vert.step;
    }
    // Columns: round stations, tangent points, PVIs, both ends.
    let label = nice_step(20.0 * ctx.opt.nlcs.scale / 1000.0) * f;
    let mut cols: Vec<f64> = al.stations(label, true).into_iter().filter(|s| *s >= view.s0 - 1e-9 && *s <= view.s1 + 1e-9).collect();
    if let Some(p) = design {
        cols.extend(p.iter().map(|q| q.s * f).filter(|s| *s > view.s0 && *s < view.s1));
    }
    cols.extend([view.s0, view.s1]);
    cols.sort_by(f64::total_cmp);
    // Keep columns at least 4.5 mm apart (ends and tangent points first).
    let mut kept: Vec<f64> = Vec::new();
    for s in cols {
        if kept.last().is_none_or(|l| s - l >= 4.5 * k) {
            kept.push(s);
        } else if (s - view.s1).abs() < 1e-9 {
            kept.pop();
            kept.push(s);
        }
    }
    let (st_a, st_b) = row_y(Row::Stationering);
    let (mv_a, mv_b) = row_y(Row::Maaiveld);
    let (on_a, on_b) = row_y(Row::Ontwerp);
    let in_design = |s: f64| design.and_then(|p| (s >= p[0].s * f - 1e-9 && s <= p[p.len() - 1].s * f + 1e-9).then(|| design_z(p, s / f)));
    for s in &kept {
        let x = gx(*s);
        let low = if design.is_some() { on_a } else { mv_a };
        out.put(&frame, pl(vec![[x, low], [x, st_b]], &frame));
        if *s > view.s0 + 1e-6 && *s < view.s1 - 1e-6 {
            out.put(&frame, pl(vec![[x, gy(view.base)], [x, y1]], &frame));
        }
        let rot = FRAC_PI_2;
        out.put(&frame_t, [ctx.text([x - 1.0 * k, (st_a + st_b) / 2.0], &st(al, *s, f), 1.8, rot, &frame_t, None)]);
        if let Some(z) = samples.at(*s) {
            out.put(&frame_t, [ctx.text([x - 1.0 * k, (mv_a + mv_b) / 2.0], &nap(z, 2), 1.8, rot, &frame_t, None)]);
        }
        if design.is_some() {
            if let Some(z) = in_design(*s) {
                out.put(&frame_t, [ctx.text([x - 1.0 * k, (on_a + on_b) / 2.0], &nap(z, 2), 1.8, rot, &frame_t, None)]);
            }
        }
    }
    // Alignment band: straights on the middle line, curves raised (left) or
    // lowered (right) with their radius.
    let (al_a, al_b) = row_y(Row::Alignement);
    let mid = (al_a + al_b) / 2.0;
    let mut run: f64 = 0.0;
    for seg in &al.segs {
        let l = seg.length();
        let (a, b) = (run.max(view.s0), (run + l).min(view.s1));
        if b > a {
            match seg {
                Seg::Line { .. } => out.put(&frame, pl(vec![[gx(a), mid], [gx(b), mid]], &frame)),
                Seg::Arc { r, sweep, .. } => {
                    let y = mid + sweep.signum() * 0.28 * (al_b - al_a);
                    out.put(&frame, pl(vec![[gx(a), mid], [gx(a), y], [gx(b), y], [gx(b), mid]], &frame));
                    if b - a > 16.0 * k {
                        let ty = y - sweep.signum() * 0.22 * (al_b - al_a);
                        out.put(&frame_t, [ctx.text([gx((a + b) / 2.0), ty], &format!("R {:.0}", r / f), 1.8, 0.0, &frame_t, None)]);
                    }
                }
            }
        }
        run += l;
    }
    // Tangent points through the graph.
    for m in al.marks() {
        if m.s > view.s0 + 1e-6 && m.s < view.s1 - 1e-6 {
            out.put(&tp, pl(vec![[gx(m.s), al_b], [gx(m.s), y1]], &tp));
        }
    }
    // Ground line, broken where there is no height.
    let mut pts: Vec<(f64, Option<f64>)> = vec![(view.s0, samples.at(view.s0))];
    pts.extend(samples.s.iter().zip(&samples.z).filter(|(s, _)| **s > view.s0 && **s < view.s1).map(|(s, z)| (*s, *z)));
    pts.push((view.s1, samples.at(view.s1)));
    let mut line: Vec<XY> = Vec::new();
    for (s, z) in pts {
        match z {
            Some(z) => line.push([gx(s), gy(z)]),
            None => {
                if line.len() > 1 {
                    out.put(&ground_l, pl(std::mem::take(&mut line), &ground_l));
                }
                line.clear();
            }
        }
    }
    if line.len() > 1 {
        out.put(&ground_l, pl(line, &ground_l));
    }
    // Design: the line, its PVIs with tangents, grades and vertical curves.
    if let Some(p) = design {
        let (d0, d1) = ((p[0].s * f).max(view.s0), (p[p.len() - 1].s * f).min(view.s1));
        if d1 > d0 {
            let step = (ctx.opt.step.max(0.25) * f).min((d1 - d0) / 4.0);
            let mut ss: Vec<f64> = Vec::new();
            let mut s = d0;
            while s < d1 {
                ss.push(s);
                s += step;
            }
            ss.push(d1);
            for q in p {
                ss.extend([(q.s - q.l / 2.0) * f, (q.s + q.l / 2.0) * f]);
            }
            ss.retain(|s| *s >= d0 - 1e-9 && *s <= d1 + 1e-9);
            ss.sort_by(f64::total_cmp);
            ss.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            out.put(&design_l, pl(ss.iter().map(|s| [gx(*s), gy(design_z(p, s / f))]).collect(), &design_l));
        }
        let (hl_a, hl_b) = row_y(Row::Helling);
        for (j, q) in p.iter().enumerate() {
            let qs = q.s * f;
            if q.l > 0.0 && j > 0 && j + 1 < p.len() {
                let (g1, g2) = (grade(&p[j - 1], q), grade(q, &p[j + 1]));
                let (c0, c1) = ((q.s - q.l / 2.0) * f, (q.s + q.l / 2.0) * f);
                // Tangents to the PVI, its height above it.
                if qs >= view.s0 && qs <= view.s1 {
                    let tip = [gx(qs), gy(q.z)];
                    let (za, zb) = (q.z - g1 * q.l / 2.0, q.z + g2 * q.l / 2.0);
                    if c0 >= view.s0 && c1 <= view.s1 {
                        out.put(&help, pl(vec![[gx(c0), gy(za)], tip, [gx(c1), gy(zb)]], &help));
                    }
                    out.put(&design_l, crate::emit::markers(&[tip], &design_l.name, 0.8 * k, None));
                    let t = ctx.text([tip[0], tip[1] + 2.8 * k], &format!("PVI {}", nap(q.z, 2)), 1.8, 0.0, &design_t, None);
                    out.put(&design_t, [t]);
                }
                // The curve in the band: a box with its radius and length.
                let (a, b) = (c0.max(view.s0), c1.min(view.s1));
                if b > a {
                    out.put(&frame, crate::emit::polylines(&[(vec![[gx(a), hl_a + 1.5 * k], [gx(b), hl_a + 1.5 * k], [gx(b), hl_b - 1.5 * k], [gx(a), hl_b - 1.5 * k]], true)], &frame.name, k, None));
                    let dg = (g2 - g1).abs();
                    let kind = if g2 < g1 { tr("top", "crest") } else { tr("voet", "sag") };
                    let w = (b - a) / k;
                    let text = if dg < 1e-9 {
                        String::new()
                    } else if w > 34.0 {
                        format!("{kind} R {:.0} L {:.0}", q.l / dg, q.l)
                    } else if w > 16.0 {
                        format!("R {:.0}", q.l / dg)
                    } else {
                        String::new()
                    };
                    if !text.is_empty() {
                        out.put(&frame_t, [ctx.text([gx((a + b) / 2.0), (hl_a + hl_b) / 2.0], &text, 1.8, 0.0, &frame_t, None)]);
                    }
                }
            }
        }
        // Grades between the curves.
        for w in p.windows(2) {
            let g = grade(&w[0], &w[1]);
            let (a, b) = (((w[0].s + w[0].l / 2.0) * f).max(view.s0), ((w[1].s - w[1].l / 2.0) * f).min(view.s1));
            if b - a > 14.0 * k {
                let text = format!("{:+.2} %", g * 100.0).replace('.', ",");
                out.put(&frame_t, [ctx.text([gx((a + b) / 2.0), (hl_a + hl_b) / 2.0], &text, 1.8, 0.0, &frame_t, None)]);
                // A short arrow along the grade's direction of fall.
                let cx = gx((a + b) / 2.0) + 11.0 * k;
                let dy = if g > 0.0 { 1.5 * k } else if g < 0.0 { -1.5 * k } else { 0.0 };
                let yc = (hl_a + hl_b) / 2.0;
                out.put(&frame, pl(vec![[cx - 3.0 * k, yc - dy], [cx + 3.0 * k, yc + dy]], &frame));
            }
        }
    }
    // Title above the graph (in model space only; the sheet has its own).
    let t = ctx.text([x0, y1 + 9.0 * k], title, 2.5, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    let (hs, vs) = (ctx.opt.nlcs.scale, (ctx.opt.nlcs.scale / v).round());
    let sub = t2(format!("horizontaal 1:{hs} · verticaal 1:{vs} (overhoging {v}×)"), format!("horizontal 1:{hs} · vertical 1:{vs} (exaggeration {v}×)"));
    let t = ctx.text([x0, y1 + 4.0 * k], &sub.replace('.', ","), 1.8, 0.0, &frame_t, None);
    out.put(&frame_t, [aligned(t, TextHorizontalAlignment::Left, TextVerticalAlignment::Middle)]);
    Ok(())
}

fn grade(a: &Pvi, b: &Pvi) -> f64 {
    (b.z - a.z) / (b.s - a.s).max(1e-9)
}

/// A viewport on paper (mm) onto model space: looking at `target`, showing
/// `view_height` drawing units across its height, turned by `twist`.
fn viewport(r: &Rect, target: XY, view_height: f64, twist: f64, layer: &str, data: &ExtendedDataRecord) -> EntityType {
    let mut vp = Viewport::new();
    let c = r.centre();
    vp.center = Vector3::new(c[0], c[1], 0.0);
    vp.width = r.w();
    vp.height = r.h();
    vp.view_target = Vector3::new(target[0], target[1], 0.0);
    vp.view_center = Vector3::new(0.0, 0.0, 0.0);
    vp.view_direction = Vector3::new(0.0, 0.0, 1.0);
    vp.view_height = view_height;
    vp.twist_angle = twist;
    vp.custom_scale = vp.height / vp.view_height;
    vp.status.locked = true;
    let mut e = EntityType::Viewport(Box::new(vp));
    let cm = e.common_mut();
    cm.layer = layer.to_string();
    cm.extended_data.add_record(data.clone());
    e
}

/// A text on paper (mm).
fn ptext(at: XY, value: &str, mm: f64, rot: f64, spec: &LayerSpec, h: TextHorizontalAlignment) -> EntityType {
    let mut t = crate::emit::text(at, value, mm, rot, &spec.name, None);
    tekenblad::text_pen(&mut t, mm);
    aligned(t, h, TextVerticalAlignment::Middle)
}

/// One sheet's paper space: frame, viewports, view titles, match line
/// references, and the title column with sheet number, scales, north arrow,
/// legend and title block.
#[allow(clippy::too_many_arguments)]
fn sheet(ctx: &Ctx, req: &Request, lay: &SheetLayout, al: &Alignment, spans: &[Span], i: usize, view: &View, vert: &Vertical, info: &SheetInfo, crs: &str, place: &str, out: &mut Out) -> Result<(), String> {
    let (f, k) = (ctx.f, ctx.k);
    let n = spans.len();
    let sp = &spans[i];
    let p = &req.paper;
    let mut parts = tekenblad::Parts::new([0.0, 0.0], 1.0, &ctx.opt.nlcs);
    parts.frame(p.w, p.h);
    let col = lay.column;
    parts.line("TEKENBLAD_DEELKADER", &[(col.x0, col.y0), (col.x0, col.y1)]);
    // Title block, with the sources the drawing's commands recorded.
    let mut info = info.clone();
    let sources = bridge::sources().join(" · ");
    let max_chars = ((col.w() - 4.0) / 1.25) as usize;
    info.sources = if sources.chars().count() > max_chars { format!("{}…", sources.chars().take(max_chars.saturating_sub(1)).collect::<String>()) } else { sources };
    parts.title_block(col.x0, col.x1, &info, &ctx.opt.nlcs, p.name, crs, place);
    let tb = col.y0 + tekenblad::Parts::TITLE_H;
    parts.line("TEKENBLAD_DEELKADER", &[(col.x0, tb), (col.x1, tb)]);
    // Sheet number and stretch.
    let cxm = (col.x0 + col.x1) / 2.0;
    parts.text("TEKENBLAD_TITELBLOK", cxm, tb + 15.0, &t2(format!("BLAD {} VAN {n}", i + 1), format!("SHEET {} OF {n}", i + 1)), 5.0, false);
    parts.text("TEKENBLAD_TITELBLOK", cxm, tb + 6.5, &format!("{} — {}", st(al, sp.s0, f), st(al, sp.s1, f)), 2.5, false);
    parts.line("TEKENBLAD_DEELKADER", &[(col.x0, tb + 24.0), (col.x1, tb + 24.0)]);
    // Scales.
    let ysc = tb + 34.0;
    parts.scale_bar(col.x0 + 8.0, ysc, req.scale);
    let vscale = (req.scale / vert.v).round();
    let hs = req.scale;
    parts.text("TEKENBLAD_SCHAALBALK", col.x0 + 8.0, ysc + 12.0, &t2(format!("lengteprofiel: horizontaal 1:{hs} · verticaal 1:{vscale}"), format!("profile: horizontal 1:{hs} · vertical 1:{vscale}")), 1.8, true);
    parts.line("TEKENBLAD_DEELKADER", &[(col.x0, ysc + 18.0), (col.x1, ysc + 18.0)]);
    // Legend above the north arrow.
    let ylg = ysc + 58.0;
    parts.line("TEKENBLAD_DEELKADER", &[(col.x0, ylg), (col.x1, ylg)]);
    parts.legend(col.x0 + 5.0, col.x1 - 5.0, ylg + 4.0, col.y1 - 4.0);
    for (spec, e) in parts.finish() {
        out.put(&spec, e);
    }
    // North arrow, turned as the plan is: world north shows at 90° + twist.
    let twist = -sp.angle;
    let orient = ctx.layer("AL", "ORIENTATIE", Element::S)?;
    let c = [cxm, ysc + 38.0];
    let shape = |pts: &[XY]| -> Vec<XY> { pts.iter().map(|q| add(c, rotate(*q, twist))).collect() };
    out.put(&orient, crate::emit::polylines(&[(shape(&[[0.0, 11.0], [4.0, -6.0], [0.0, -3.0], [-4.0, -6.0]]), true)], &orient.name, 1.0, None));
    out.put(&orient, crate::emit::polylines(&[(shape(&[[0.0, 11.0], [0.0, -3.0], [4.0, -6.0]]), true)], &orient.name, 1.0, None));
    let tip = add(c, rotate([0.0, 15.0], twist));
    out.put(&orient, [ptext(tip, "N", 3.5, 0.0, &orient, TextHorizontalAlignment::Center)]);
    // View titles.
    let explain = ctx.layer("AL", "TEKENBLAD_VERKLARING", Element::T)?;
    let (pl, pr) = (lay.plan, lay.profile);
    // This sheet's profile only: the viewport as large as its view.
    let pr = Rect { x0: pr.x0, y0: pr.y0, x1: (pr.x0 + view.size[0] / k + 4.0).min(pr.x1), y1: (pr.y0 + view.size[1] / k + 4.0).min(pr.y1) };
    let (a, b) = (st(al, sp.s0, f), st(al, sp.s1, f));
    // A title and its subtitle after it (the font runs about 0.95 × its height per character).
    let mut title = |x: f64, y: f64, big: &str, small: String| {
        out.put(&explain, [ptext([x, y], big, 3.5, 0.0, &explain, TextHorizontalAlignment::Left)]);
        let after = x + big.chars().count() as f64 * 3.5 * 0.95 + 5.0;
        out.put(&explain, [ptext([after, y], &small, 2.5, 0.0, &explain, TextHorizontalAlignment::Left)]);
    };
    title(pl.x0, pl.y1 + 4.5, tr("SITUATIE", "PLAN"), t2(format!("schaal 1:{hs}"), format!("scale 1:{hs}")));
    title(pr.x0, pr.y1 + 4.5, tr("LENGTEPROFIEL", "PROFILE"), t2(format!("horizontaal 1:{hs} · verticaal 1:{vscale} · {a} — {b}"), format!("horizontal 1:{hs} · vertical 1:{vscale} · {a} — {b}")));
    // The views framed (the plot leaves viewport borders out), then the viewports.
    let part = ctx.layer("AL", "TEKENBLAD_DEELKADER", Element::G)?;
    for r in [&pl, &pr] {
        out.put(&part, crate::emit::polylines(&[(vec![[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]], true)], &part.name, 1.0, None));
    }
    let vpl = ctx.layer("AL", "TEKENBLAD_VIEWPORT", Element::G)?;
    let d = record(&[("bladen", al.name.clone()), ("blad", format!("{} van {n}", i + 1)), ("aanzicht", "situatie".into())]);
    out.put(&vpl, [viewport(&pl, sp.centre, pl.h() * k, twist, &vpl.name, &d)]);
    let w_model = [pr.w() * k, pr.h() * k];
    let target = [view.org[0] - 2.0 * k + w_model[0] / 2.0, view.org[1] - 2.0 * k + w_model[1] / 2.0];
    let d = record(&[("bladen", al.name.clone()), ("blad", format!("{} van {n}", i + 1)), ("aanzicht", "lengteprofiel".into())]);
    out.put(&vpl, [viewport(&pr, target, pr.h() * k, 0.0, &vpl.name, &d)]);
    // Match line references inside the plan view, near its top edge.
    let refs = ctx.layer("AL", "BLADVERWIJZING_AS", Element::T)?;
    let to_paper = |q: XY| -> XY { add(pl.centre(), scaled(rotate([q[0] - sp.centre[0], q[1] - sp.centre[1]], twist), 1.0 / k)) };
    let mut refer = |s: f64, other: usize, after: bool| {
        // Beside the match line, 9 mm off the axis on its left.
        let (q, dir) = al.at(s);
        let nrm = [-dir.sin(), dir.cos()];
        let at = to_paper(add(q, scaled(nrm, 9.0 * k)));
        let (x, yt) = (at[0], at[1].clamp(pl.y0 + 4.0, pl.y1 - 4.0));
        if x < pl.x0 + 2.0 || x > pl.x1 - 2.0 {
            return;
        }
        let (xt, h) = if after { (x + 2.5, TextHorizontalAlignment::Left) } else { (x - 2.5, TextHorizontalAlignment::Right) };
        let text = t2(format!("ZIE BLAD {other}"), format!("SEE SHEET {other}"));
        out.put(&refs, [ptext([xt, yt], &text, 2.5, 0.0, &refs, h)]);
    };
    if i > 0 {
        refer(sp.s0, i, false);
    }
    if i + 1 < n {
        refer(sp.s1, i + 2, true);
    }
    Ok(())
}

// ── PDF ─────────────────────────────────────────────────────────────────────

/// `CIV_BLADEN_PDF`: every sheet of the axis (or of all axes) to its own PDF
/// through the host's `-PLOT`. The prompts split on spaces, so the folder
/// may not hold any: then the plugin's own folder is used.
pub fn pdf(axis: Option<String>, dir: Option<std::path::PathBuf>, progress: &dyn Fn(String)) -> Result<String, String> {
    let sheets: Vec<(String, String, String)> = sheets_read().into_iter().filter(|(a, l, _)| a != "-" && l != "-" && axis.as_ref().is_none_or(|x| x == a)).collect();
    if sheets.is_empty() {
        return Err(match axis {
            Some(a) => t2(format!("geen bladen van {a}: maak ze eerst met CIV_BLADEN"), format!("no sheets of {a}: make them first with CIV_BLADEN")),
            None => tr("geen bladen in deze tekening: maak ze eerst met CIV_BLADEN", "no sheets in this drawing: make them first with CIV_BLADEN").into(),
        });
    }
    let spaced = |p: &std::path::Path| p.to_string_lossy().chars().any(char::is_whitespace);
    let mut dir = dir.unwrap_or_else(|| bridge::beside_drawing("bladen"));
    if spaced(&dir) {
        dir = crate::gis::servers::config_dir().join("bladen");
    }
    if spaced(&dir) {
        let d = dir.display();
        return Err(t2(format!("de map {d} bevat een spatie; geef MAP=<map zonder spaties>"), format!("the folder {d} holds a space; give MAP=<folder without spaces>")));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut done = Vec::new();
    for (i, (_, layout, paper)) in sheets.iter().enumerate() {
        progress(format!("PDF {}/{}: {layout} …", i + 1, sheets.len()));
        let path = dir.join(format!("{layout}.pdf"));
        let _ = std::fs::remove_file(&path);
        // Detailed: layout, PDF, paper, mm, landscape, not upside down, the
        // layout at 1:1, centred, no plot styles, lineweights, three paper
        // space questions, file, do not save the setup, plot.
        let line = format!("-PLOT Y {layout} PDF {paper} Millimeters Landscape N Layout 1:1 Center N Y N N N {} N Y", path.display());
        bridge::execute(&line)?;
        let t0 = std::time::Instant::now();
        let mut last = 0;
        loop {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size > 0 && size == last {
                break;
            }
            last = size;
            if t0.elapsed() > std::time::Duration::from_secs(90) {
                let p = path.display();
                return Err(t2(format!("{layout}: Open CAD Studio schreef geen PDF ({p})"), format!("{layout}: Open CAD Studio wrote no PDF ({p})")));
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        done.push(path.display().to_string());
    }
    api::note("pdf", done.clone());
    api::note("map", dir.display().to_string());
    let (n, d) = (done.len(), dir.display());
    Ok(t2(format!("{n} blad(en) als PDF in {d}."), format!("{n} sheet(s) as PDF in {d}.")))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;

    fn frame(w: f64, h: f64, overlap: f64, max_len: f64, round: f64) -> Frame {
        Frame { w, h, overlap, max_len, round, start: 0.0 }
    }

    /// Every point of the stretch plus overlap lies in the turned window.
    fn inside(al: &Alignment, sp: &Span, fr: &Frame) -> bool {
        let (lo, hi) = ((sp.s0 - fr.overlap / 2.0).max(0.0), (sp.s1 + fr.overlap / 2.0).min(al.length()));
        (0..=200).all(|i| {
            let (p, _) = al.at(lo + (hi - lo) * i as f64 / 200.0);
            let q = rotate([p[0] - sp.centre[0], p[1] - sp.centre[1]], -sp.angle);
            q[0].abs() <= fr.w / 2.0 + 1e-6 && q[1].abs() <= fr.h / 2.0 + 1e-6
        })
    }

    #[test]
    fn a_straight_axis_in_equal_sheets() {
        let al = Alignment::from_pis(&[[0.0, 0.0], [1000.0, 0.0]], &[]).unwrap();
        let fr = frame(600.0, 300.0, 20.0, 580.0, 10.0);
        let spans = divide(&al, &fr);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].s0, spans[0].s1, spans[1].s1), (0.0, 500.0, 1000.0));
        assert!(spans.iter().all(|s| s.angle.abs() < 1e-9));
        // Centred on its stretch plus the overlap (none before 0).
        assert!((spans[0].centre[0] - 255.0).abs() < 1e-6 && spans[0].centre[1].abs() < 1e-9);
        assert!(spans.iter().all(|s| inside(&al, s, &fr)));
        // Short: one sheet.
        let al = Alignment::from_pis(&[[0.0, 0.0], [0.0, 120.0]], &[]).unwrap();
        let spans = divide(&al, &fr);
        assert_eq!(spans.len(), 1);
        assert!((spans[0].angle - FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn the_profile_width_limits_a_sheet() {
        let al = Alignment::from_pis(&[[0.0, 0.0], [1000.0, 0.0]], &[]).unwrap();
        let spans = divide(&al, &frame(600.0, 300.0, 20.0, 230.0, 10.0));
        assert_eq!(spans.len(), 5);
        assert!(spans.iter().all(|s| s.s1 - s.s0 <= 230.0 + 1e-9));
        assert!(spans.windows(2).all(|w| (w[0].s1 - w[1].s0).abs() < 1e-12));
    }

    #[test]
    fn curves_turn_and_shorten_the_sheets() {
        // A hairpin: east 400, a half circle R 60 to the left, back west.
        let al = Alignment::from_pis(&[[0.0, 0.0], [400.0, 0.0], [400.0, 120.0], [0.0, 120.0]], &[60.0]).unwrap();
        let fr = frame(300.0, 150.0, 10.0, 280.0, 0.0);
        let spans = divide(&al, &fr);
        let len = al.length();
        assert!((spans[0].s0).abs() < 1e-12 && (spans.last().unwrap().s1 - len).abs() < 1e-9);
        assert!(spans.windows(2).all(|w| (w[0].s1 - w[1].s0).abs() < 1e-12));
        for s in &spans {
            assert!(inside(&al, s, &fr), "{s:?} does not fit");
        }
        // The last sheet runs west: turned by about 180°.
        assert!((spans.last().unwrap().angle.abs() - PI).abs() < 0.2);
        // The bend shortens a sheet below what the profile allows.
        assert!(spans.iter().any(|s| s.s1 - s.s0 < 250.0), "{spans:?}");
        // A window too low for the bend gets more, shorter sheets.
        let low = frame(300.0, 40.0, 10.0, 280.0, 0.0);
        let tight = divide(&al, &low);
        assert!(tight.len() > spans.len(), "{spans:?}\n{tight:?}");
        assert!(tight.iter().all(|s| inside(&al, s, &low)));
    }

    #[test]
    fn match_lines_on_round_stations() {
        let mut al = Alignment::from_pis(&[[0.0, 0.0], [700.0, 300.0], [1500.0, 300.0]], &[400.0]).unwrap();
        al.start = 1234.5;
        let fr = Frame { w: 600.0, h: 300.0, overlap: 20.0, max_len: 560.0, round: 10.0, start: 1234.5 };
        let spans = divide(&al, &fr);
        assert!(spans.len() >= 3);
        for s in &spans[..spans.len() - 1] {
            let station = 1234.5 + s.s1;
            assert!((station / 10.0 - (station / 10.0).round()).abs() < 1e-9, "{station}");
            assert!(inside(&al, s, &fr));
        }
    }

    #[test]
    fn the_vertical_scale_drops_until_the_profile_fits() {
        // 1:1000, 100 mm of graph: 6 m of height fits at 10× (60 mm + room).
        let v = fit_vertical(&[Some((0.0, 6.0)), Some((1.0, 3.0))], 100.0, 1000.0, 10.0);
        assert_eq!(v.v, 10.0);
        assert!(v.fits && v.step == 1.0);
        assert_eq!(v.ranges[0], (-1.0, 7.0));
        // 15 m does not: 5× (75 mm + room).
        let v = fit_vertical(&[Some((0.0, 15.0))], 100.0, 1000.0, 10.0);
        assert_eq!(v.v, 5.0);
        // Nothing fits: 1× and a warning.
        let v = fit_vertical(&[Some((0.0, 500.0))], 100.0, 1000.0, 10.0);
        assert!(!v.fits && v.v == 1.0);
    }

    #[test]
    fn scales_papers_and_layout_names() {
        assert_eq!(parse_scale("1:500"), Some(500.0));
        assert_eq!(parse_scale("1000"), Some(1000.0));
        assert_eq!(parse_scale("1/250"), Some(250.0));
        assert_eq!(parse_scale("abc"), None);
        assert_eq!(Paper::parse("a3").unwrap().w, 420.0);
        assert_eq!(layout_name("Rondweg Zuid", "A1", 3), "Rondweg_Zuid_A1_blad_03");
        assert_eq!(layout_name("", "A3", 1), "As_A3_blad_01");
        // The plan and the profile are as wide; A3 keeps a narrower title column.
        let l = sheet_layout(&Paper::parse("A1").unwrap());
        assert_eq!((l.plan.x0, l.plan.x1), (l.profile.x0, l.profile.x1));
        assert!(l.plan.y0 > l.profile.y1 + VIEW_TITLE);
        assert_eq!(sheet_layout(&Paper::parse("A3").unwrap()).column.w(), 125.0);
        let s = Samples { s: vec![0.0, 10.0, 20.0], z: vec![Some(1.0), Some(2.0), None] };
        assert_eq!(s.at(5.0), Some(1.5));
        assert_eq!(s.at(15.0), None);
        assert_eq!(s.at(10.0), Some(2.0));
    }
}
