//! Labels: stationing along an axis, curve data,
//! the PVIs and grades of a design profile, spot heights and slope arrows
//! on a TIN, survey point labels. The style comes from
//! `<config>/labels.json` (text height in paper mm, decimals, prefixes, NLCS
//! layers; written with the defaults on first use), sized for the plot
//! scale of the settings (`CivielOptions`).
//!
//! Open CAD Studio has no reactors, so a label cannot follow its source by
//! itself. Every label carries object data instead: its kind (`label`), its
//! source (`bron`: a handle, or `tin:B` for a terrain) and a fingerprint of
//! the source's geometry and data (`vingerafdruk`); spot heights also keep
//! where they sit (`plek`). The labels are indexed per source in the
//! drawing (OPENINFRA/INDEX, kind "labels"). `CIV_LABELS_BIJWERKEN` compares
//! the fingerprints with the sources as they are now: the labels of a
//! changed or moved source are drawn again, those of a deleted source go.

use std::collections::{HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};
use ocs_plugin_api::host::{ExtendedDataRecord, XDataValue};
use serde::{Deserialize, Serialize};

use super::as_profiel::{design_z, fit_curves, Alignment, Pvi, Seg};
use super::bladen::aligned;
use super::{api, data_value, record, station, CivielOptions, Ctx};
use crate::bridge;
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::{Element, LayerSpec};

/// A message in the interface language, from its Dutch and English text.
fn t2(nl: String, en: String) -> String {
    tr(&nl, &en).to_string()
}

// ── Style ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// Text height on paper (mm) of the labels; NLCS text classes are 1.8,
    /// 2.5, 3.5, 5 and 7 mm.
    pub tekst_mm: f64,
    pub decimalen: Decimals,
    pub stationering: StationStyle,
    pub bogen: CurveStyle,
    pub pvi: PviStyle,
    pub hellingen: LayerOnly,
    pub hoogtes: SpotStyle,
    pub afschot: SlopeStyle,
    pub punten: PointStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decimals {
    pub hoogte: usize,
    pub station: usize,
    pub helling: usize,
    pub straal: usize,
    pub lengte: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StationStyle {
    /// Ticks every `klein_m`, a long tick with its label every `groot_m`.
    pub klein_m: f64,
    pub groot_m: f64,
    /// "station" (0+100) or "hectometer" (0,1).
    pub formaat: String,
    pub voorvoegsel: String,
    /// NLCS layer as "GROUP OBJECT".
    pub laag: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurveStyle {
    pub voorvoegsel_straal: String,
    pub voorvoegsel_lengte: String,
    /// Radius and length texts.
    pub laag: String,
    /// Tangent points BB, EB, HP.
    pub laag_punten: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PviStyle {
    pub voorvoegsel: String,
    pub laag: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerOnly {
    pub laag: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpotStyle {
    pub voorvoegsel: String,
    /// Grid spacing when no points are given (m).
    pub raster_m: f64,
    pub laag: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlopeStyle {
    pub raster_m: f64,
    /// Arrow length on paper (mm).
    pub pijl_mm: f64,
    /// Flatter than this (%) gets no arrow.
    pub min_procent: f64,
    pub laag: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PointStyle {
    pub naam: bool,
    pub hoogte: bool,
    pub code: bool,
    pub laag: String,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            tekst_mm: 1.8,
            decimalen: Decimals::default(),
            stationering: StationStyle::default(),
            bogen: CurveStyle::default(),
            pvi: PviStyle::default(),
            hellingen: LayerOnly { laag: "AM AS".into() },
            hoogtes: SpotStyle::default(),
            afschot: SlopeStyle::default(),
            punten: PointStyle::default(),
        }
    }
}

impl Default for Decimals {
    fn default() -> Self {
        Decimals { hoogte: 2, station: 2, helling: 2, straal: 2, lengte: 2 }
    }
}

impl Default for StationStyle {
    fn default() -> Self {
        StationStyle { klein_m: 20.0, groot_m: 100.0, formaat: "station".into(), voorvoegsel: String::new(), laag: "AM KILOMETRERING".into() }
    }
}

impl Default for CurveStyle {
    fn default() -> Self {
        CurveStyle { voorvoegsel_straal: "R = ".into(), voorvoegsel_lengte: "L = ".into(), laag: "AM AS".into(), laag_punten: "AM TANGENTPUNT".into() }
    }
}

impl Default for PviStyle {
    fn default() -> Self {
        PviStyle { voorvoegsel: "PVI ".into(), laag: "AM AS".into() }
    }
}

impl Default for LayerOnly {
    fn default() -> Self {
        LayerOnly { laag: "AM AS".into() }
    }
}

impl Default for SpotStyle {
    fn default() -> Self {
        SpotStyle { voorvoegsel: String::new(), raster_m: 20.0, laag: "ZZ HOOGTELABEL".into() }
    }
}

impl Default for SlopeStyle {
    fn default() -> Self {
        SlopeStyle { raster_m: 20.0, pijl_mm: 8.0, min_procent: 0.1, laag: "ZZ AANPIJLING".into() }
    }
}

impl Default for PointStyle {
    fn default() -> Self {
        PointStyle { naam: true, hoogte: true, code: true, laag: "OG HOOGTEPUNT".into() }
    }
}

pub fn style_path() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("labels.json")
}

/// The label style: `<config>/labels.json`, written with the defaults when
/// it is missing so it can be edited.
pub fn style() -> Result<Style, String> {
    let path = style_path();
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display())),
        Err(_) => {
            let s = Style::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&s).unwrap_or_default());
            Ok(s)
        }
    }
}

/// "AM KILOMETRERING" → the NLCS layer of that object.
fn layer(ctx: &Ctx, spec: &str, el: Element) -> Result<LayerSpec, String> {
    let (g, o) = spec.trim().split_once([' ', '-', '/']).ok_or_else(|| t2(format!("labels.json: laag '{spec}' moet 'GROEP OBJECT' zijn, bijv. 'AM KILOMETRERING'"), format!("labels.json: layer '{spec}' must be 'GROUP OBJECT', e.g. 'AM KILOMETRERING'")))?;
    ctx.layer(g.trim(), o.trim(), el)
}

/// A number the Dutch way: decimal comma.
pub fn num(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}").replace('.', ",")
}

/// A height: "+2,43".
pub fn height(v: f64, decimals: usize) -> String {
    format!("{v:+.decimals$}").replace('.', ",")
}

/// A station in the style's format.
pub fn station_text(m: f64, s: &StationStyle, decimals: usize) -> String {
    let body = if s.formaat.eq_ignore_ascii_case("hectometer") {
        num(m / 1000.0, 1)
    } else if decimals == 0 || (m - m.round()).abs() < 0.005 {
        station(m.round())
    } else {
        station(m)
    };
    format!("{}{body}", s.voorvoegsel)
}

// ── Kinds and sources ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Stationering,
    Bogen,
    Pvi,
    Hellingen,
    Hoogtes,
    Afschot,
    Punten,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Stationering, Kind::Bogen, Kind::Pvi, Kind::Hellingen, Kind::Hoogtes, Kind::Afschot, Kind::Punten];

    pub fn code(self) -> &'static str {
        match self {
            Kind::Stationering => "stationering",
            Kind::Bogen => "bogen",
            Kind::Pvi => "pvi",
            Kind::Hellingen => "hellingen",
            Kind::Hoogtes => "hoogtes",
            Kind::Afschot => "afschot",
            Kind::Punten => "punten",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "stationering" | "station" | "hectometrering" | "km" => Kind::Stationering,
            "bogen" | "boog" | "boogdata" | "curves" => Kind::Bogen,
            "pvi" | "pvis" | "verticale_bogen" => Kind::Pvi,
            "hellingen" | "helling" | "grades" => Kind::Hellingen,
            "hoogtes" | "hoogte" | "hoogtepunten" | "spot" => Kind::Hoogtes,
            "afschot" | "hellingpijlen" | "pijlen" => Kind::Afschot,
            "punten" | "punt" | "puntlabels" => Kind::Punten,
            _ => return None,
        })
    }

    /// "stationering;bogen" or "alles".
    pub fn list(s: &str) -> Result<Vec<Kind>, String> {
        if s.trim().eq_ignore_ascii_case("alles") {
            return Ok(Kind::ALL.to_vec());
        }
        s.split([';', ',']).filter(|t| !t.trim().is_empty()).map(|t| Kind::parse(t).ok_or_else(|| {
                let all = Kind::ALL.map(Kind::code).join(", ");
                t2(format!("SOORT: onbekend '{t}'; kies {all}"), format!("SOORT: unknown '{t}'; choose {all}"))
            }))
            .collect()
    }
}

/// An index key: "<kind> <source>[ <variant>]".
fn key(kind: Kind, bron: &str, variant: &str) -> String {
    if variant.is_empty() {
        format!("{} {bron}", kind.code())
    } else {
        format!("{} {bron} {variant}", kind.code())
    }
}

fn parse_key(k: &str) -> Option<(Kind, String, String)> {
    let mut p = k.splitn(3, ' ');
    let kind = Kind::parse(p.next()?)?;
    let bron = p.next()?.to_string();
    Some((kind, bron, p.next().unwrap_or("").to_string()))
}

/// FNV-1a over rounded numbers and texts: stable between runs and builds.
pub fn fingerprint(values: &[f64], texts: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: &[u8]| {
        for x in b {
            h ^= *x as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for v in values {
        eat(&((v * 1000.0).round() as i64).to_le_bytes());
    }
    for t in texts {
        eat(t.as_bytes());
        eat(&[0]);
    }
    format!("{h:016x}")
}

/// Fingerprint of a source entity: its geometry and its object data.
pub fn entity_fingerprint(e: &EntityType) -> String {
    let mut v: Vec<f64> = Vec::new();
    match e {
        EntityType::LwPolyline(l) => {
            for p in &l.vertices {
                v.extend([p.location.x, p.location.y, p.bulge]);
            }
        }
        EntityType::Point(p) => v.extend([p.location.x, p.location.y, p.location.z]),
        EntityType::Line(l) => v.extend([l.start.x, l.start.y, l.start.z, l.end.x, l.end.y, l.end.z]),
        EntityType::Arc(a) => v.extend([a.center.x, a.center.y, a.radius, a.start_angle, a.end_angle]),
        other => {
            if let Some(al) = Alignment::from_entity(other) {
                let len = al.length();
                for i in 0..=64 {
                    let (p, _) = al.at(len * i as f64 / 64.0);
                    v.extend(p);
                }
            }
        }
    }
    let texts: Vec<String> = e
        .common()
        .extended_data
        .get_record(bridge::XDATA_APP)
        .map(|r| {
            r.values
                .iter()
                .filter_map(|x| match x {
                    XDataValue::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    fingerprint(&v, &texts)
}

pub(crate) fn tin_fingerprint(t: &super::terrein::Tin) -> String {
    let mut v = Vec::with_capacity(t.points.len() * 3 + 1);
    v.push(t.triangles.len() as f64);
    for p in &t.points {
        v.extend(p);
    }
    fingerprint(&v, &[])
}

fn hex(h: Handle) -> String {
    api::handle_text(h)
}

/// The labels of one source, ready to write.
struct Group {
    key: String,
    ents: Vec<(LayerSpec, EntityType)>,
}

// ── Drawing the labels ──────────────────────────────────────────────────────

struct Painter<'a> {
    ctx: &'a Ctx,
    style: &'a Style,
}

impl Painter<'_> {
    fn mm(&self) -> f64 {
        self.style.tekst_mm.max(0.5)
    }

    fn line(&self, a: XY, b: XY, spec: &LayerSpec, d: &ExtendedDataRecord) -> EntityType {
        crate::emit::polylines(&[(vec![a, b], false)], &spec.name, self.ctx.k, Some(d)).remove(0)
    }

    fn text(&self, at: XY, value: &str, rot: f64, spec: &LayerSpec, d: &ExtendedDataRecord, h: TextHorizontalAlignment) -> EntityType {
        aligned(self.ctx.text(at, value, self.mm(), rot, spec, Some(d)), h, TextVerticalAlignment::Middle)
    }

    /// Text going out along `rot` from `at`, never upside down.
    fn outward(&self, at: XY, value: &str, rot: f64, spec: &LayerSpec, d: &ExtendedDataRecord) -> EntityType {
        let r = crate::emit::readable(rot);
        let flipped = (r - rot.rem_euclid(std::f64::consts::TAU)).abs() > 1e-6;
        let h = if flipped { TextHorizontalAlignment::Right } else { TextHorizontalAlignment::Left };
        self.text(at, value, r, spec, d, h)
    }

    /// Ticks every `klein_m`, long ones with the station every `groot_m`.
    fn stationing(&self, al: &Alignment, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let (f, k) = (self.ctx.f, self.ctx.k);
        let st = &self.style.stationering;
        let g = layer(self.ctx, &st.laag, Element::G)?;
        let t = layer(self.ctx, &st.laag, Element::T)?;
        let small = st.klein_m.max(1.0) * f;
        let big = st.groot_m.max(st.klein_m.max(1.0)) * f;
        let start = al.start * f;
        let len = al.length();
        let mut out = Vec::new();
        let mut s = (start / small).ceil() * small - start;
        while s <= len + 1e-9 {
            let abs = start + s;
            let is_big = ((abs / big).round() * big - abs).abs() < 1e-6 * f.max(1.0);
            let (p, dir) = al.at(s);
            let nrm = [-dir.sin(), dir.cos()];
            let half = if is_big { 2.5 } else { 1.2 } * k;
            out.push((g.clone(), self.line([p[0] - nrm[0] * half, p[1] - nrm[1] * half], [p[0] + nrm[0] * half, p[1] + nrm[1] * half], &g, d)));
            if is_big {
                let at = [p[0] + nrm[0] * (half + 1.0 * k), p[1] + nrm[1] * (half + 1.0 * k)];
                out.push((t.clone(), self.outward(at, &station_text(abs / f, st, 0), dir + FRAC_PI_2, &t, d)));
            }
            s += small;
        }
        Ok(out)
    }

    /// Tangent points with their station; radius and length in each curve.
    fn curves(&self, al: &Alignment, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let (f, k) = (self.ctx.f, self.ctx.k);
        let cs = &self.style.bogen;
        let dec = &self.style.decimalen;
        let pg = layer(self.ctx, &cs.laag_punten, Element::G)?;
        let pt = layer(self.ctx, &cs.laag_punten, Element::T)?;
        let tt = layer(self.ctx, &cs.laag, Element::T)?;
        let mut out = Vec::new();
        for m in al.marks() {
            if !matches!(m.code, "BB" | "EB" | "OB" | "HP") {
                continue;
            }
            let (p, dir) = al.at(m.s);
            let nrm = [-dir.sin(), dir.cos()];
            let half = 4.0 * k;
            out.push((pg.clone(), self.line([p[0] - nrm[0] * half, p[1] - nrm[1] * half], [p[0] + nrm[0] * half, p[1] + nrm[1] * half], &pg, d)));
            let at = [p[0] - nrm[0] * (half + 1.0 * k), p[1] - nrm[1] * (half + 1.0 * k)];
            let label = format!("{} {}", m.code, station_text(al.start + m.s / f, &self.style.stationering, dec.station));
            out.push((pt.clone(), self.outward(at, &label, dir - FRAC_PI_2, &pt, d)));
        }
        let mut run = 0.0;
        for seg in &al.segs {
            let l = seg.length();
            match seg {
                Seg::Line { .. } => {}
                // A clothoid: its parameter A and length, on the turning side.
                Seg::Spiral { k0, k1, .. } => {
                    let (p, dir) = al.at(run + l / 2.0);
                    let nrm = [-dir.sin(), dir.cos()];
                    let side = if k1.abs() >= k0.abs() { k1.signum() } else { k0.signum() };
                    let rot = crate::emit::readable(dir);
                    let aa = seg.spiral_a().unwrap_or(0.0);
                    for (i, text) in [format!("A {}", num(aa / f, dec.straal)), format!("{}{}", cs.voorvoegsel_lengte, num(l / f, dec.lengte))].iter().enumerate() {
                        let off = (3.0 + 3.2 * i as f64) * k * side;
                        out.push((tt.clone(), self.text([p[0] + nrm[0] * off, p[1] + nrm[1] * off], text, rot, &tt, d, TextHorizontalAlignment::Center)));
                    }
                }
                Seg::Arc { r, sweep, .. } => {
                    let (p, dir) = al.at(run + l / 2.0);
                    let nrm = [-dir.sin(), dir.cos()];
                    let side = sweep.signum();
                    let rot = crate::emit::readable(dir);
                    for (i, text) in [format!("{}{}", cs.voorvoegsel_straal, num(r / f, dec.straal)), format!("{}{}", cs.voorvoegsel_lengte, num(l / f, dec.lengte))].iter().enumerate() {
                        let off = (3.0 + 3.2 * i as f64) * k * side;
                        out.push((tt.clone(), self.text([p[0] + nrm[0] * off, p[1] + nrm[1] * off], text, rot, &tt, d, TextHorizontalAlignment::Center)));
                    }
                }
            }
            run += l;
        }
        Ok(out)
    }

    /// The PVIs of a design profile: a mark at the point of intersection,
    /// station and height, and the vertical curve with its radius and length.
    fn pvis(&self, map: &ProfileMap, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let k = self.ctx.k;
        let ps = &self.style.pvi;
        let dec = &self.style.decimalen;
        let g = layer(self.ctx, &ps.laag, Element::G)?;
        let t = layer(self.ctx, &ps.laag, Element::T)?;
        let p = &map.pvis;
        let mut out = Vec::new();
        for (j, q) in p.iter().enumerate() {
            let tip = map.to(q.s, q.z);
            let on = map.to(q.s, design_z(p, q.s));
            out.extend(crate::emit::markers(&[tip], &g.name, 0.8 * k, Some(d)).into_iter().map(|e| (g.clone(), e)));
            let top = [tip[0], tip[1].max(on[1]) + 4.0 * k];
            out.push((g.clone(), self.line([on[0], on[1].min(tip[1])], top, &g, d)));
            let label = format!("{}{} {}", ps.voorvoegsel, station_text(map.start + q.s, &self.style.stationering, dec.station), height(q.z, dec.hoogte));
            out.push((t.clone(), self.text([top[0] - 1.2 * k, top[1] + 1.0 * k], &label, FRAC_PI_2, &t, d, TextHorizontalAlignment::Left)));
            if q.l > 0.0 && j > 0 && j + 1 < p.len() {
                let (g1, g2) = (grade(&p[j - 1], q), grade(q, &p[j + 1]));
                let dg = (g2 - g1).abs();
                if dg > 1e-9 {
                    let kind = if g2 < g1 { tr("topboog", "crest curve") } else { tr("voetboog", "sag curve") };
                    let curve = format!("{kind} R {} L {}", num(q.l / dg, 0), num(q.l, dec.lengte.min(1)));
                    out.push((t.clone(), self.text([top[0] + 1.6 * k, top[1] + 1.0 * k], &curve, FRAC_PI_2, &t, d, TextHorizontalAlignment::Left)));
                }
            }
        }
        Ok(out)
    }

    /// Grades along the straights of a design profile, above the line.
    fn grades(&self, map: &ProfileMap, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let k = self.ctx.k;
        let t = layer(self.ctx, &self.style.hellingen.laag, Element::T)?;
        let p = &map.pvis;
        let mut out = Vec::new();
        for w in p.windows(2) {
            let gr = grade(&w[0], &w[1]);
            let (a, b) = (w[0].s + w[0].l / 2.0, w[1].s - w[1].l / 2.0);
            let mid = (a + b) / 2.0;
            let ang = (gr * map.v).atan();
            let q = map.to(mid, design_z(p, mid));
            let at = [q[0] - ang.sin() * 2.5 * k, q[1] + ang.cos() * 2.5 * k];
            out.push((t.clone(), self.text(at, &format!("{} %", height(gr * 100.0, self.style.decimalen.helling)), ang, &t, d, TextHorizontalAlignment::Center)));
        }
        Ok(out)
    }

    /// A spot height: a small cross and the height beside it.
    fn spot(&self, at: XY, z: f64, base: &[(&str, String)]) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let k = self.ctx.k;
        let sp = &self.style.hoogtes;
        let g = layer(self.ctx, &sp.laag, Element::S)?;
        let t = layer(self.ctx, &sp.laag, Element::T)?;
        let mut pairs = base.to_vec();
        pairs.push(("plek", format!("{:.3},{:.3}", at[0], at[1])));
        let d = record(&pairs);
        let arm = 0.8 * k;
        Ok(vec![
            (g.clone(), self.line([at[0] - arm, at[1]], [at[0] + arm, at[1]], &g, &d)),
            (g.clone(), self.line([at[0], at[1] - arm], [at[0], at[1] + arm], &g, &d)),
            (t.clone(), self.text([at[0] + 1.2 * k, at[1] + 1.2 * k], &format!("{}{}", sp.voorvoegsel, height(z, self.style.decimalen.hoogte)), 0.0, &t, &d, TextHorizontalAlignment::Left)),
        ])
    }

    /// A slope arrow pointing down the TIN, with the slope in %.
    fn slope(&self, at: XY, gx: f64, gy: f64, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let k = self.ctx.k;
        let sl = &self.style.afschot;
        let g = layer(self.ctx, &sl.laag, Element::G)?;
        let t = layer(self.ctx, &sl.laag, Element::T)?;
        let s = gx.hypot(gy);
        let u = [-gx / s, -gy / s];
        let half = sl.pijl_mm.max(2.0) * k / 2.0;
        let (a, b) = ([at[0] - u[0] * half, at[1] - u[1] * half], [at[0] + u[0] * half, at[1] + u[1] * half]);
        let head = 1.5 * k;
        let ang = u[1].atan2(u[0]);
        let wing = |da: f64| [b[0] - head * (ang + da).cos(), b[1] - head * (ang + da).sin()];
        let mut out = vec![(g.clone(), self.line(a, b, &g, d))];
        out.push((g.clone(), crate::emit::polylines(&[(vec![wing(0.4), b, wing(-0.4)], false)], &g.name, k, Some(d)).remove(0)));
        let nrm = [-u[1], u[0]];
        let tat = [at[0] + nrm[0] * 1.6 * k, at[1] + nrm[1] * 1.6 * k];
        out.push((t.clone(), self.text(tat, &format!("{} %", num(s * 100.0, 1)), crate::emit::readable(ang), &t, d, TextHorizontalAlignment::Center)));
        Ok(out)
    }

    /// A survey point's number, height and code beside it.
    fn point(&self, e: &EntityType, d: &ExtendedDataRecord) -> Result<Vec<(LayerSpec, EntityType)>, String> {
        let k = self.ctx.k;
        let ps = &self.style.punten;
        let t = layer(self.ctx, &ps.laag, Element::T)?;
        let EntityType::Point(p) = e else { return Ok(Vec::new()) };
        let at = [p.location.x, p.location.y];
        let z = data_value(e, "z").and_then(|v| v.parse::<f64>().ok()).unwrap_or(p.location.z / self.ctx.f);
        let mut rows: Vec<String> = Vec::new();
        if ps.naam {
            rows.extend(data_value(e, "puntnummer"));
        }
        if ps.hoogte {
            rows.push(height(z, self.style.decimalen.hoogte));
        }
        if ps.code {
            rows.extend(data_value(e, "code").filter(|c| !c.is_empty()));
        }
        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, r)| (t.clone(), self.text([at[0] + 1.2 * k, at[1] + 1.4 * k - i as f64 * 2.7 * k], r, 0.0, &t, d, TextHorizontalAlignment::Left)))
            .collect())
    }
}

fn grade(a: &Pvi, b: &Pvi) -> f64 {
    (b.z - a.z) / (b.s - a.s).max(1e-9)
}

/// Where a design profile is drawn: the length profile frame of its axis,
/// shifted to where the design line itself lies now (labels follow the line
/// when it is moved).
struct ProfileMap {
    pvis: Vec<Pvi>,
    x_nul: f64,
    y_basis: f64,
    basis: f64,
    v: f64,
    f: f64,
    /// Station at the start of the axis (m).
    start: f64,
    shift: XY,
}

impl ProfileMap {
    fn to(&self, s: f64, z: f64) -> XY {
        [self.x_nul + s * self.f + self.shift[0], self.y_basis + (z - self.basis) * self.v * self.f + self.shift[1]]
    }
}

/// The PVIs of a design line (its object data).
fn pvis_of(e: &EntityType) -> Option<Vec<Pvi>> {
    let pts: Vec<(f64, f64)> = data_value(e, "pvi")?
        .split(';')
        .filter_map(|t| {
            let (a, b) = t.split_once(':')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect();
    let l: f64 = data_value(e, "boog_m").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    (pts.len() >= 2).then(|| fit_curves(&pts, l))
}

fn profile_map(doc: &CadDocument, design: &EntityType) -> Result<ProfileMap, String> {
    let pvis = pvis_of(design).ok_or(tr("het ontwerpprofiel mist zijn PVI's (objectgegevens 'pvi')", "the design profile lacks its PVIs (object data 'pvi')"))?;
    let axis = data_value(design, "as").unwrap_or_default();
    let frame = doc
        .model_space_entities()
        .filter(|e| data_value(e, "lengteprofiel").is_some() && data_value(e, "as").unwrap_or_default() == axis)
        .last()
        .ok_or_else(|| t2(format!("geen lengteprofiel van {axis} gevonden"), format!("no length profile of {axis} found")))?;
    let get = |k: &str| data_value(frame, k).and_then(|v| v.parse::<f64>().ok()).ok_or_else(|| t2(format!("het lengteprofiel mist '{k}'"), format!("the length profile lacks '{k}'")));
    let mut map = ProfileMap { x_nul: get("x_nul")?, y_basis: get("y_basis")?, basis: get("basis")?, v: get("overhoging")?, f: get("eenheid")?, start: get("beginstation")?, pvis, shift: [0.0, 0.0] };
    if let EntityType::LwPolyline(l) = design {
        if let Some(first) = l.vertices.first() {
            let s0 = map.pvis[0].s;
            let at = map.to(s0, design_z(&map.pvis, s0));
            map.shift = [first.location.x - at[0], first.location.y - at[1]];
        }
    }
    Ok(map)
}

/// The design line of an axis: the newest entity with `ontwerpprofiel` data.
fn design_line<'a>(doc: &'a CadDocument, axis: &str) -> Option<&'a EntityType> {
    doc.model_space_entities().filter(|e| data_value(e, "ontwerpprofiel").is_some() && data_value(e, "as").unwrap_or_default() == axis).last()
}

/// Grid points every `step` (drawing units) over a TIN.
fn grid(t: &super::terrein::Tin, step: f64) -> Vec<XY> {
    let b = t.points.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let mut out = Vec::new();
    let mut x = (b[0] / step).ceil() * step;
    while x <= b[2] {
        let mut y = (b[1] / step).ceil() * step;
        while y <= b[3] {
            out.push([x, y]);
            y += step;
        }
        x += step;
    }
    out
}

/// Slope of the TIN at a point (dz/dx, dz/dy), by central differences.
fn gradient(t: &super::terrein::Tin, p: XY, h: f64) -> Option<(f64, f64)> {
    let (e, w) = (t.height([p[0] + h, p[1]])?, t.height([p[0] - h, p[1]])?);
    let (n, s) = (t.height([p[0], p[1] + h])?, t.height([p[0], p[1] - h])?);
    Some(((e - w) / (2.0 * h), (n - s) / (2.0 * h)))
}

/// What the labels of a terrain need again: the spots, or the grid.
#[derive(Clone, Debug, Default)]
struct TinParams {
    spots: Vec<XY>,
    raster: Option<f64>,
}

fn tin_groups(p: &Painter, kind: Kind, status: char, tin: &super::terrein::Tin, params: &TinParams, variant: &str) -> Result<Group, String> {
    let f = p.ctx.f;
    let bron = format!("tin:{status}");
    let fp = tin_fingerprint(tin);
    let mut base: Vec<(&str, String)> = vec![("label", kind.code().to_string()), ("bron", bron.clone()), ("vingerafdruk", fp)];
    if let Some(r) = params.raster {
        base.push(("raster", format!("{r}")));
    }
    let mut ents = Vec::new();
    match kind {
        Kind::Hoogtes => {
            let spots = match params.raster {
                Some(r) => grid(tin, r * f),
                None => params.spots.clone(),
            };
            if spots.len() > 20_000 {
                let n = spots.len();
                return Err(t2(format!("{n} hoogtepunten is te veel: kies een groter raster"), format!("{n} spot heights is too many: choose a larger grid")));
            }
            for at in spots {
                if let Some(z) = tin.height(at) {
                    ents.extend(p.spot(at, z / f, &base)?);
                }
            }
        }
        Kind::Afschot => {
            let r = params.raster.unwrap_or(p.style.afschot.raster_m) * f;
            let pts = grid(tin, r);
            if pts.len() > 20_000 {
                let n = pts.len();
                return Err(t2(format!("{n} hellingpijlen is te veel: kies een groter raster"), format!("{n} slope arrows is too many: choose a larger grid")));
            }
            let d = record(&base);
            let h = (r / 4.0).min(1.0 * f).max(0.01 * f);
            for at in pts {
                if let Some((gx, gy)) = gradient(tin, at, h) {
                    if gx.hypot(gy) * 100.0 >= p.style.afschot.min_procent {
                        ents.extend(p.slope(at, gx, gy, &d)?);
                    }
                }
            }
        }
        _ => {}
    }
    Ok(Group { key: key(kind, &bron, variant), ents })
}

/// The labels of an entity source (axis, design line, survey point).
fn entity_group(p: &Painter, doc: &CadDocument, kind: Kind, e: &EntityType) -> Result<Group, String> {
    let bron = hex(e.common().handle);
    let d = record(&[("label", kind.code().to_string()), ("bron", bron.clone()), ("vingerafdruk", entity_fingerprint(e))]);
    let ents = match kind {
        Kind::Stationering | Kind::Bogen => {
            let al = Alignment::from_entity(e).ok_or(tr("de bron is geen as", "the source is no alignment"))?;
            if kind == Kind::Stationering {
                p.stationing(&al, &d)?
            } else {
                p.curves(&al, &d)?
            }
        }
        Kind::Pvi | Kind::Hellingen => {
            let map = profile_map(doc, e)?;
            if kind == Kind::Pvi {
                p.pvis(&map, &d)?
            } else {
                p.grades(&map, &d)?
            }
        }
        Kind::Punten => p.point(e, &d)?,
        Kind::Hoogtes | Kind::Afschot => Vec::new(),
    };
    Ok(Group { key: key(kind, &bron, ""), ents })
}

/// Write the groups as one undo step; their new handles per key.
fn write(label: &str, groups: Vec<Group>) -> Result<Vec<(String, Vec<Handle>)>, String> {
    let mut made: HashSet<String> = HashSet::new();
    let mut flat = Vec::new();
    let mut owner = Vec::new();
    let mut first: Option<ExtendedDataRecord> = None;
    let keys: Vec<String> = groups.iter().map(|g| g.key.clone()).collect();
    for (gi, g) in groups.into_iter().enumerate() {
        for (spec, e) in g.ents {
            if made.insert(spec.name.clone()) {
                bridge::ensure_nlcs_layer(&spec);
            }
            if first.is_none() {
                first = e.common().extended_data.get_record(bridge::XDATA_APP).cloned();
            }
            flat.push(e);
            owner.push(gi);
        }
    }
    let handles = bridge::add_entities_handles(label, flat)?;
    if let (Some(h), Some(r)) = (handles.first(), first) {
        bridge::register_object_data(*h, r);
    }
    let mut per: Vec<Vec<Handle>> = vec![Vec::new(); keys.len()];
    for (h, gi) in handles.iter().zip(owner) {
        if !h.is_null() {
            per[gi].push(*h);
        }
    }
    Ok(keys.into_iter().zip(per).collect())
}

// ── Commands ────────────────────────────────────────────────────────────────

/// `CIV_LABELS`: what to label.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// Empty: from the selection (an axis: stationing and curves, with PVIs
    /// and grades when it has a design profile; survey points: their labels).
    pub kinds: Vec<Kind>,
    pub axis: Option<Handle>,
    /// Spot heights at these drawing points.
    pub points: Vec<XY>,
    /// Spot heights or slope arrows on a grid (m).
    pub grid: Option<f64>,
    /// The terrain: 'B' (existing) or 'N' (design).
    pub surface: char,
}

pub fn create(opt: &CivielOptions, req: Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let style = style()?;
    let ctx = Ctx::new(opt)?;
    let painter = Painter { ctx: &ctx, style: &style };
    let doc = super::document(&["as", "lengteprofiel", "ontwerpprofiel", "tin", "punten"], &req.axis.into_iter().collect::<Vec<_>>())?;
    let sel: HashSet<Handle> = bridge::selection().unwrap_or_default().into_iter().collect();
    let selected: Vec<&EntityType> = doc.model_space_entities().filter(|e| sel.contains(&e.common().handle)).collect();
    // The axis: by handle, or the one selected line that is an axis.
    let axis: Option<&EntityType> = match req.axis {
        Some(h) => Some(doc.get_entity(h).ok_or_else(|| t2(format!("geen element met handle {}", hex(h)), format!("no entity with handle {}", hex(h))))?),
        None => selected.iter().copied().find(|e| Alignment::from_entity(e).is_some() && data_value(e, "ontwerpprofiel").is_none() && data_value(e, "lengteprofiel").is_none() && !matches!(e, EntityType::Point(_))),
    };
    let axis_name = axis.and_then(|e| data_value(e, "as")).unwrap_or_default();
    // The design line: the selected one, or the newest of the axis.
    let design: Option<&EntityType> = selected.iter().copied().find(|e| data_value(e, "ontwerpprofiel").is_some()).or_else(|| axis.and_then(|_| design_line(&doc, &axis_name)));
    let points: Vec<&EntityType> = {
        let all: Vec<&EntityType> = doc.model_space_entities().filter(|e| matches!(e, EntityType::Point(_)) && crate::nlcs::as_nlcs(&e.common().layer).contains("-OG-HOOGTEPUNT-")).collect();
        let picked: Vec<&EntityType> = all.iter().copied().filter(|e| sel.contains(&e.common().handle)).collect();
        if picked.is_empty() { all } else { picked }
    };
    let mut kinds = req.kinds.clone();
    if kinds.is_empty() {
        if axis.is_some() {
            kinds.extend([Kind::Stationering, Kind::Bogen]);
        }
        if design.is_some() {
            kinds.extend([Kind::Pvi, Kind::Hellingen]);
        }
        if kinds.is_empty() && selected.iter().any(|e| matches!(e, EntityType::Point(_))) {
            kinds.push(Kind::Punten);
        }
        if kinds.is_empty() {
            let all = Kind::ALL.map(Kind::code).join(";");
            return Err(t2(format!("selecteer een as, ontwerpprofiel of meetpunten, of geef SOORT={all}"), format!("select an alignment, design profile or survey points, or give SOORT={all}")));
        }
    }
    let status = if req.surface == 'N' { 'N' } else { 'B' };
    let tin = if kinds.iter().any(|k| matches!(k, Kind::Hoogtes | Kind::Afschot)) {
        Some(super::terrein::Tin::of_status(&doc, status).ok_or_else(|| t2(format!("geen terrein (TIN) met status {status} in de tekening: maak er eerst een (CIV_TIN)"), format!("no terrain (TIN) with status {status} in the drawing: make one first (CIV_TIN)")))?)
    } else {
        None
    };
    let mut groups = Vec::new();
    for kind in &kinds {
        progress(format!("Labels: {} …", kind.code()));
        match kind {
            Kind::Stationering | Kind::Bogen => {
                let e = axis.ok_or(tr("selecteer een as of geef AS=<handle>", "select an alignment or give AS=<handle>"))?;
                groups.push(entity_group(&painter, &doc, *kind, e)?);
            }
            Kind::Pvi | Kind::Hellingen => {
                let e = design.ok_or(tr("geen ontwerpprofiel: maak het eerst (CIV_ONTWERPPROFIEL) en geef AS= van zijn as", "no design profile: make it first (CIV_ONTWERPPROFIEL) and give AS= of its alignment"))?;
                groups.push(entity_group(&painter, &doc, *kind, e)?);
            }
            Kind::Punten => {
                if points.is_empty() {
                    return Err(tr("geen meetpunten in de tekening (Punten → Punten importeren of Punt plaatsen)", "no survey points in the drawing (Points → Import points or Place point)").into());
                }
                for e in &points {
                    groups.push(entity_group(&painter, &doc, *kind, e)?);
                }
            }
            Kind::Hoogtes | Kind::Afschot => {
                let tin = tin.as_ref().expect("tin");
                let (params, variant) = if *kind == Kind::Hoogtes && !req.points.is_empty() {
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                    (TinParams { spots: req.points.clone(), raster: None }, format!("p{stamp:x}"))
                } else {
                    let r = req.grid.unwrap_or(if *kind == Kind::Hoogtes { style.hoogtes.raster_m } else { style.afschot.raster_m }).max(0.1);
                    (TinParams { spots: Vec::new(), raster: Some(r) }, "raster".to_string())
                };
                groups.push(tin_groups(&painter, *kind, status, tin, &params, &variant)?);
            }
        }
    }
    // Labels of these sources made before are replaced.
    let wanted: HashSet<&str> = groups.iter().map(|g| g.key.as_str()).collect();
    let old: Vec<Handle> = bridge::index_read().into_iter().filter(|(k, key, _)| k == "labels" && wanted.contains(key.as_str())).flat_map(|(_, _, h)| h).collect();
    if !old.is_empty() {
        let _ = bridge::remove_entities(tr("Labels vervangen", "Replace labels"), &old);
    }
    let n_groups = groups.len();
    let written = write(tr("Labels", "Labels"), groups)?;
    let total: usize = written.iter().map(|(_, h)| h.len()).sum();
    bridge::index_set_many("labels", &written);
    let mut per_kind: HashMap<&str, usize> = HashMap::new();
    for (key, hs) in &written {
        if let Some((kind, _, _)) = parse_key(key) {
            *per_kind.entry(kind.code()).or_default() += hs.len();
        }
    }
    let mut summary: Vec<String> = kinds.iter().map(|k| format!("{} {}", k.code(), per_kind.get(k.code()).copied().unwrap_or(0))).collect();
    summary.dedup();
    api::note("labels", total);
    api::note("groepen", n_groups);
    api::note("soorten", serde_json::json!(per_kind));
    api::note("stijl", style_path().display().to_string());
    let (list, path) = (summary.join(", "), style_path().display().to_string());
    Ok(t2(format!("Labels: {total} elementen ({list}), stijl {path}."), format!("Labels: {total} entities ({list}), style {path}.")))
}

/// `CIV_LABELS_BIJWERKEN`: labels follow their source.
pub fn update(opt: &CivielOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let style = style()?;
    let ctx = Ctx::new(opt)?;
    let painter = Painter { ctx: &ctx, style: &style };
    let groups: Vec<(String, Vec<Handle>)> = bridge::index_read().into_iter().filter(|(k, _, _)| k == "labels").map(|(_, key, h)| (key, h)).collect();
    if groups.is_empty() {
        api::note("bijgewerkt", 0);
        api::note("verwijderd", 0);
        api::note("ongewijzigd", 0);
        return Ok(tr("Geen labels om bij te werken (CIV_LABELS).", "No labels to update (CIV_LABELS).").into());
    }
    let sources: Vec<Handle> = groups.iter().filter_map(|(k, _)| parse_key(k)).filter(|(_, b, _)| !b.starts_with("tin:")).filter_map(|(_, b, _)| api::parse_handle(&b).ok()).collect();
    let tins = groups.iter().any(|(k, _)| k.contains(" tin:"));
    let needs: &[&str] = if tins { &["lengteprofiel", "tin"] } else { &["lengteprofiel"] };
    progress(tr("Labels: bronnen lezen …", "Labels: reading the sources …").into());
    let doc = super::document(needs, &sources)?;
    let mut tin_cache: HashMap<char, Option<super::terrein::Tin>> = HashMap::new();
    let (mut redraw, mut changes, mut gone, mut same, mut removed) = (Vec::new(), Vec::new(), 0usize, 0usize, Vec::new());
    for (key, handles) in groups {
        let Some((kind, bron, variant)) = parse_key(&key) else { continue };
        let mut labels = Vec::new();
        for chunk in handles.chunks(500) {
            labels.extend(bridge::entities(chunk)?.into_iter().flatten());
        }
        if labels.is_empty() {
            // The labels were deleted by hand: forget them.
            changes.push((key, Vec::new()));
            continue;
        }
        let live: Vec<Handle> = labels.iter().map(|e| e.common().handle).collect();
        let stored = labels.iter().find_map(|e| data_value(e, "vingerafdruk")).unwrap_or_default();
        if let Some(status) = bron.strip_prefix("tin:").and_then(|s| s.chars().next()) {
            let tin = tin_cache.entry(status).or_insert_with(|| super::terrein::Tin::of_status(&doc, status));
            let Some(tin) = tin.as_ref() else {
                removed.extend(live);
                changes.push((key, Vec::new()));
                gone += 1;
                continue;
            };
            if tin_fingerprint(tin) == stored {
                same += 1;
                changes.push((key, live));
                continue;
            }
            let raster = labels.iter().find_map(|e| data_value(e, "raster")).and_then(|v| v.parse::<f64>().ok());
            let mut spots: Vec<XY> = labels
                .iter()
                .filter_map(|e| data_value(e, "plek"))
                .filter_map(|v| {
                    let (x, y) = v.split_once(',')?;
                    Some([x.parse().ok()?, y.parse().ok()?])
                })
                .collect();
            spots.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            spots.dedup();
            removed.extend(live);
            redraw.push(tin_groups(&painter, kind, status, tin, &TinParams { spots, raster }, &variant)?);
            continue;
        }
        let src = api::parse_handle(&bron).ok().and_then(|h| doc.get_entity(h));
        let Some(src) = src else {
            // The source is gone: its labels go too.
            removed.extend(live);
            changes.push((key, Vec::new()));
            gone += 1;
            continue;
        };
        if entity_fingerprint(src) == stored {
            same += 1;
            changes.push((key, live));
            continue;
        }
        removed.extend(live);
        match entity_group(&painter, &doc, kind, src) {
            Ok(g) => redraw.push(g),
            // A source that cannot be labelled any more (no profile frame): leave it unlabelled.
            Err(e) => {
                bridge::info(&format!("Labels {key}: {e}"));
                changes.push((key, Vec::new()));
            }
        }
    }
    let updated = redraw.len();
    if !removed.is_empty() {
        bridge::remove_entities(tr("Labels bijwerken", "Update labels"), &removed)?;
    }
    if !redraw.is_empty() {
        changes.extend(write(tr("Labels bijwerken", "Update labels"), redraw)?);
    }
    bridge::index_set_many("labels", &changes);
    api::note("bijgewerkt", updated);
    api::note("verwijderd", gone);
    api::note("ongewijzigd", same);
    Ok(t2(
        format!("Labels bijgewerkt: {updated} bron(nen) opnieuw gelabeld, {gone} verwijderd (bron weg), {same} ongewijzigd."),
        format!("Labels updated: {updated} source(s) labelled again, {gone} removed (source gone), {same} unchanged."),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_round_trip_with_defaults() {
        let s: Style = serde_json::from_str(r#"{"tekst_mm": 2.5, "stationering": {"groot_m": 50}}"#).unwrap();
        assert_eq!(s.tekst_mm, 2.5);
        assert_eq!(s.stationering.groot_m, 50.0);
        assert_eq!(s.stationering.klein_m, 20.0);
        assert_eq!(s.hoogtes.laag, "ZZ HOOGTELABEL");
        let back: Style = serde_json::from_str(&serde_json::to_string(&Style::default()).unwrap()).unwrap();
        assert_eq!(back, Style::default());
    }

    #[test]
    fn texts_the_dutch_way() {
        let st = StationStyle::default();
        assert_eq!(station_text(1200.0, &st, 2), "1+200");
        assert_eq!(station_text(123.456, &st, 2), "0+123,46");
        let hm = StationStyle { formaat: "hectometer".into(), voorvoegsel: "km ".into(), ..st };
        assert_eq!(station_text(12_300.0, &hm, 0), "km 12,3");
        assert_eq!(height(2.4321, 2), "+2,43");
        assert_eq!(height(-0.5, 1), "-0,5");
        assert_eq!(num(8000.4, 0), "8000");
    }

    #[test]
    fn kinds_and_keys() {
        assert_eq!(Kind::list("stationering;bogen").unwrap(), vec![Kind::Stationering, Kind::Bogen]);
        assert_eq!(Kind::list("alles").unwrap().len(), 7);
        assert!(Kind::list("foo").is_err());
        let k = key(Kind::Hoogtes, "tin:B", "raster");
        assert_eq!(parse_key(&k), Some((Kind::Hoogtes, "tin:B".into(), "raster".into())));
        assert_eq!(parse_key("pvi 1A3"), Some((Kind::Pvi, "1A3".into(), String::new())));
    }

    #[test]
    fn fingerprints_see_moves_and_edits() {
        use ocs_plugin_api::host::codec::{LwPolyline, Vector2};
        let mut lw = LwPolyline::new();
        lw.add_point_with_bulge(Vector2::new(0.0, 0.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(100.0, 0.0), 0.0);
        let a = EntityType::LwPolyline(lw.clone());
        let mut moved = lw.clone();
        moved.vertices[1].location.x = 100.5;
        let b = EntityType::LwPolyline(moved);
        assert_eq!(entity_fingerprint(&a), entity_fingerprint(&EntityType::LwPolyline(lw)));
        assert_ne!(entity_fingerprint(&a), entity_fingerprint(&b));
        // Object data counts too (a new start station).
        let mut c = a.clone();
        c.common_mut().extended_data.add_record(record(&[("beginstation", "100".into())]));
        assert_ne!(entity_fingerprint(&a), entity_fingerprint(&c));
        assert_eq!(fingerprint(&[1.0, 2.0], &[]), fingerprint(&[1.0000001, 2.0], &[]));
    }

    #[test]
    fn a_grid_and_the_slope_of_a_plane() {
        // A plane falling 2 % to the east over 100 × 100.
        let pts = vec![[0.0, 0.0, 2.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0], [0.0, 100.0, 2.0]];
        let tin = super::super::terrein::Tin::build(&pts, 1000.0).unwrap();
        let g = grid(&tin, 25.0);
        assert_eq!(g.len(), 25);
        let (gx, gy) = gradient(&tin, [50.0, 50.0], 1.0).unwrap();
        assert!((gx + 0.02).abs() < 1e-9 && gy.abs() < 1e-9);
        assert!(gradient(&tin, [150.0, 50.0], 1.0).is_none());
    }
}
