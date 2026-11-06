//! Geotechnics: soil profiles from CPTs and boreholes (`CIV_GRONDPROFIEL`),
//! slope stability of dikes, embankments and ditch banks
//! (`CIV_STABILITEIT`) and the settlement of fills (`CIV_ZETTING`).
//!
//! A calculation aid, not a stamped design: the defaults come from a cited
//! table and are marked as estimates where no source holds; the engineer
//! checks the soil parameters and the assumptions. See docs/geotechnics.md
//! for every formula and source.

pub mod map;
pub mod model;
pub mod profile;
pub mod section;
pub mod settlement;
pub mod soil;
pub mod stability;

use std::collections::BTreeMap;

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{Color, EntityType};
use ocs_plugin_api::host::ExtendedDataRecord;

use super::Ctx;
use crate::gis::bro::{self, Kind};
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::LayerSpec;
use model::{Band, Column, Layer, Model, Subsoil};

pub(crate) use super::grading::tf;

/// The positioning line under every result.
pub fn disclaimer() -> &'static str {
    tr(
        "Rekenhulpmiddel, geen gewaarmerkt ontwerp: controleer de grondparameters en uitgangspunten.",
        "Calculation aid, not a stamped design: check the soil parameters and assumptions.",
    )
}

/// The parameters of these units from table 2.b with the overrides.
pub fn units_of<'a>(codes: impl IntoIterator<Item = &'a str>, overrides: &BTreeMap<String, Vec<(String, f64)>>) -> Result<BTreeMap<String, soil::Params>, String> {
    let mut out = BTreeMap::new();
    for c in codes {
        if !out.contains_key(c) {
            out.insert(c.to_string(), soil::params_with(c, overrides)?);
        }
    }
    Ok(out)
}

/// Columns shallower than this (m) are left out.
pub const MIN_DEPTH: f64 = 2.0;

/// Bands thinner than this (m) join a neighbour in the calculations.
pub const THIN: f64 = 0.2;

/// CPTs and boreholes from the BRO within `radius` m of a line (project
/// coordinates), as soil columns with their station and offset along it;
/// the second value lists what was skipped.
pub fn collect_near(ctx: &Ctx, line: &[XY], kinds: &[Kind], radius: f64, max: usize, min_layer: f64, progress: &dyn Fn(String)) -> Result<(Vec<Column>, Vec<String>), String> {
    let site = ctx.site.clone().ok_or_else(|| tr("stel eerst de projectlocatie in (GIS → Locatie, of GIS_SITE)", "set the project location first (GIS → Location, or GIS_SITE)").to_string())?;
    if line.is_empty() {
        return Err(tr("geen lijn", "no line").into());
    }
    let b = line.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]);
    let b = [b[0] - radius, b[1] - radius, b[2] + radius, b[3] + radius];
    let rd = if site.crs == "EPSG:28992" { b } else { crate::gis::site::envelope(b, &crate::gis::crs::Transformer::new(&site.crs, "EPSG:28992")?)? };
    let ll = crate::gis::site::envelope(rd, &crate::gis::crs::Transformer::new("EPSG:28992", "EPSG:4326")?)?;
    let to = crate::gis::vector::data_to_site(&site, "EPSG:28992", &[])?;
    let mut skipped = Vec::new();
    let mut columns = Vec::new();
    let mut bron: Option<super::Hoogtebron> = None;
    for kind in kinds {
        progress(format!("BRO: {}…", kind.label()));
        let found = bro::search(*kind, ll)?;
        skipped.extend(found.warnings);
        // (point, project position, station, offset), nearest to the line first.
        let mut near: Vec<(bro::Point, XY, f64, f64)> = found
            .points
            .into_iter()
            .filter(|p| !p.deregistered)
            .filter_map(|p| {
                let at_rd = p.rd.or_else(|| p.lonlat.and_then(|l| crate::gis::crs::transform("EPSG:4326", "EPSG:28992", l).ok()))?;
                let q = to(at_rd)?;
                // Within `radius` of the line (beyond its ends: of its end points).
                let (s, off) = model::station_offset(line, q);
                (off.abs() <= radius).then_some((p, q, s, off))
            })
            .collect();
        near.sort_by(|a, b| a.3.abs().total_cmp(&b.3.abs()));
        near.truncate(max);
        if near.is_empty() {
            continue;
        }
        let ids: Vec<String> = near.iter().map(|n| n.0.id.clone()).collect();
        type Got = Result<(bro::Point, Vec<bro::Interval>, bool), String>;
        let got: Vec<Got> = match kind {
            Kind::Cpt => bro::fetch_many(&ids, bro::fetch_cpt, progress).into_iter().map(|r| r.map(|c| (c.point, bro::cpt_intervals(&c.samples, min_layer.min(0.2)), true))).collect(),
            Kind::Bhr => bro::fetch_many(&ids, bro::fetch_bhr, progress).into_iter().map(|r| r.map(|b| (b.point, b.layers, false))).collect(),
        };
        for ((head, q, s, off), r) in near.into_iter().zip(got) {
            let (full, intervals, cpt) = match r {
                Ok(x) => x,
                Err(e) => {
                    skipped.push(e);
                    continue;
                }
            };
            let z0 = match full.surface.or(head.surface) {
                Some(z) => z,
                None => {
                    if bron.is_none() {
                        bron = Some(super::hoogtebron(ctx)?);
                    }
                    match bron.as_mut().and_then(|b| b.height(ctx.to_drawing(q))) {
                        Some(z) => z,
                        None => {
                            skipped.push(format!("{}: {}", head.id, tr("geen maaiveldhoogte", "no surface level")));
                            continue;
                        }
                    }
                }
            };
            let raw: Vec<model::RawLayer> = intervals
                .iter()
                .filter(|l| l.bottom > l.top)
                .map(|l| (z0 - l.top, z0 - l.bottom, if cpt { soil::of_cpt(l) } else { soil::of_borehole(l) }, l.qc, l.name.clone()))
                .collect();
            let mut layers: Vec<Layer> = model::simplify(raw, min_layer);
            if layers.is_empty() {
                skipped.push(format!("{}: {}", head.id, tr("geen bruikbare lagen", "no usable layers")));
                continue;
            }
            // The first unit from the surface level (a predrilled or unmeasured top takes it).
            layers[0].top = layers[0].top.max(z0);
            let end = layers.last().map(|l| l.bottom).unwrap_or(z0);
            // Too shallow: its last unit would stand for the whole depth.
            if z0 - end < MIN_DEPTH {
                skipped.push(format!("{}: {} {} m", head.id, tr("te ondiep", "too shallow"), num(z0 - end, 2)));
                continue;
            }
            columns.push(Column { id: head.id.clone(), kind: if cpt { "cpt" } else { "borehole" }.into(), station: s, offset: off, xy: q, surface: z0, end, layers });
        }
    }
    if let Some(b) = &bron {
        crate::bridge::note_source(b.attribution());
    }
    columns.sort_by(|a, b| a.station.total_cmp(&b.station));
    // Two columns at (nearly) one station: keep the CPT, then the deeper one.
    let mut kept: Vec<Column> = Vec::new();
    for c in columns {
        match kept.last_mut() {
            Some(prev) if (c.station - prev.station).abs() < 0.5 => {
                let better = (c.kind == "cpt" && prev.kind != "cpt") || (c.kind == prev.kind && c.end < prev.end);
                skipped.push(format!("{} {} {}", if better { &prev.id } else { &c.id }, tr("valt samen met", "coincides with"), if better { &c.id } else { &prev.id }));
                if better {
                    *prev = c;
                }
            }
            _ => kept.push(c),
        }
    }
    if !kept.is_empty() {
        crate::bridge::note_source("BRO sonderingen en boringen (TNO / BRO, CC0)");
    }
    Ok((kept, skipped))
}

/// Where a calculation takes its soil from (`LAGEN=`, `MODEL=`, or the BRO).
#[derive(Clone, Debug)]
pub enum SoilChoice {
    /// Horizontal layers by top level (m NAP) and the base level.
    Layers(Vec<(f64, String)>, Option<f64>),
    /// A stored soil model by name; `None`: the nearest stored one, or the BRO.
    Model(Option<String>),
}

/// The soil arguments shared by the commands.
#[derive(Clone, Debug)]
pub struct SoilArgs {
    pub choice: SoilChoice,
    pub overrides: BTreeMap<String, Vec<(String, f64)>>,
    pub kinds: Vec<Kind>,
    pub radius: f64,
    pub min_layer: f64,
}

/// The soil for a calculation around `near` (project coordinates): given
/// layers, a stored model, or else the BRO columns near it (along the line
/// as a model when `as_line`, otherwise the nearest column per point).
pub fn subsoil(ctx: &Ctx, args: &SoilArgs, near: &[XY], as_line: bool, progress: &dyn Fn(String)) -> Result<Subsoil, String> {
    let fresh = |codes: Vec<String>| units_of(codes.iter().map(String::as_str), &args.overrides);
    match &args.choice {
        SoilChoice::Layers(tops, base) => {
            let base = base.unwrap_or_else(|| tops.last().map(|t| t.0 - 10.0).unwrap_or(-20.0));
            if tops.last().is_some_and(|t| t.0 <= base) {
                return Err(tr("BASIS ligt boven de onderste laag", "BASIS lies above the lowest layer").into());
            }
            let units = fresh(tops.iter().map(|t| t.1.clone()).collect())?;
            Ok(Subsoil::Layers { tops: tops.clone(), base, units })
        }
        SoilChoice::Model(name) => {
            let all = model::load_all();
            let pick = match name {
                Some(n) => Some(all.get(n).cloned().ok_or_else(|| format!("{} '{n}' ({}: {})", tr("geen grondmodel", "no soil model"), tr("in de tekening", "in the drawing"), all.keys().cloned().collect::<Vec<_>>().join(", ")))?),
                None => {
                    // The stored model whose line passes nearest, within reach.
                    let c = centroid(near);
                    all.values()
                        .map(|m| (model::station_offset(&m.line, c).1.abs(), m))
                        .filter(|(d, _)| *d <= args.radius.max(25.0) * 2.0)
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .map(|(_, m)| m.clone())
                }
            };
            if let Some(mut m) = pick {
                if !args.overrides.is_empty() {
                    let codes: Vec<String> = m.units.keys().cloned().collect();
                    for (k, v) in fresh(codes)? {
                        m.units.insert(k, v);
                    }
                }
                return Ok(Subsoil::Model(m));
            }
            // No model: the BRO near the line or area.
            let (columns, skipped) = collect_near(ctx, near, &args.kinds, args.radius, 30, args.min_layer, progress)?;
            if columns.is_empty() {
                let why = skipped.first().map(|s| format!(" ({s})")).unwrap_or_default();
                return Err(format!(
                    "{} {} m{why}; {}",
                    tr("geen sonderingen of boringen in de BRO binnen", "no CPTs or boreholes in the BRO within"),
                    args.radius,
                    tr("geef LAGEN=bovenkant:grondsoort;… of maak eerst een grondprofiel (CIV_GRONDPROFIEL)", "give LAGEN=top:unit;… or make a soil profile first (CIV_GRONDPROFIEL)")
                ));
            }
            let base = columns.iter().map(|c| c.end).fold(f64::MAX, f64::min);
            let codes: Vec<String> = columns.iter().flat_map(|c| c.layers.iter().map(|l| l.code.clone())).collect();
            let units = fresh(codes)?;
            Ok(if as_line {
                Subsoil::Model(Model { name: tr("BRO langs de doorsnede", "BRO along the section").into(), line: near.to_vec(), start_station: 0.0, columns, base, units, source: "BRO".into() })
            } else {
                Subsoil::Nearest { columns, base, units }
            })
        }
    }
}

fn centroid(pts: &[XY]) -> XY {
    let n = pts.len().max(1) as f64;
    let s = pts.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

/// Polygons of the soil along x: for samples (x, bands top down), one ring
/// in (x, z) per run of samples with the same sequence of units; where the
/// sequence changes the rings meet halfway between the samples.
pub fn band_polygons(samples: &[(f64, Vec<Band>)]) -> Vec<(String, Vec<XY>)> {
    let mut out = Vec::new();
    let codes = |b: &[Band]| b.iter().map(|x| x.code.clone()).collect::<Vec<_>>();
    let mut start = 0;
    while start < samples.len() {
        let seq = codes(&samples[start].1);
        let mut end = start + 1;
        while end < samples.len() && codes(&samples[end].1) == seq {
            end += 1;
        }
        // Edges halfway to the neighbouring runs.
        let x_left = if start == 0 { samples[0].0 } else { 0.5 * (samples[start - 1].0 + samples[start].0) };
        let x_right = if end == samples.len() { samples[end - 1].0 } else { 0.5 * (samples[end - 1].0 + samples[end].0) };
        for (k, code) in seq.iter().enumerate() {
            let mut top: Vec<XY> = vec![[x_left, samples[start].1[k].top]];
            let mut bottom: Vec<XY> = vec![[x_left, samples[start].1[k].bottom]];
            for s in &samples[start..end] {
                top.push([s.0, s.1[k].top]);
                bottom.push([s.0, s.1[k].bottom]);
            }
            top.push([x_right, samples[end - 1].1[k].top]);
            bottom.push([x_right, samples[end - 1].1[k].bottom]);
            top.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
            bottom.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
            let mut ring = top;
            ring.extend(bottom.into_iter().rev());
            if ring.len() >= 3 {
                out.push((code.clone(), ring));
            }
        }
        start = end;
    }
    out
}

/// The boundaries between the units along x: per run of samples with the
/// same sequence, the bottom of every unit but the last, as polylines.
pub fn band_boundaries(samples: &[(f64, Vec<Band>)]) -> Vec<Vec<XY>> {
    let mut out = Vec::new();
    let codes = |b: &[Band]| b.iter().map(|x| x.code.clone()).collect::<Vec<_>>();
    let mut start = 0;
    while start < samples.len() {
        let seq = codes(&samples[start].1);
        let mut end = start + 1;
        while end < samples.len() && codes(&samples[end].1) == seq {
            end += 1;
        }
        let x_left = if start == 0 { samples[0].0 } else { 0.5 * (samples[start - 1].0 + samples[start].0) };
        let x_right = if end == samples.len() { samples[end - 1].0 } else { 0.5 * (samples[end - 1].0 + samples[end].0) };
        for k in 0..seq.len().saturating_sub(1) {
            let mut line: Vec<XY> = vec![[x_left, samples[start].1[k].bottom]];
            line.extend(samples[start..end].iter().map(|s| [s.0, s.1[k].bottom]));
            line.push([x_right, samples[end - 1].1[k].bottom]);
            line.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-9);
            if line.len() >= 2 {
                out.push(line);
            }
        }
        start = end;
    }
    out
}

/// A solid hatch in a true colour.
pub fn hatch(ring: &[XY], spec: &LayerSpec, rgb: (u8, u8, u8), data: Option<&ExtendedDataRecord>) -> EntityType {
    let mut e = EntityType::Hatch(Box::new(crate::plannen::hatch(&[ring.to_vec()], false, 1.0)));
    let c = e.common_mut();
    c.layer = spec.name.clone();
    c.color = Color::Rgb { r: rgb.0, g: rgb.1, b: rgb.2 };
    if let Some(d) = data {
        c.extended_data.add_record(d.clone());
    }
    e
}

/// A polyline in a colour.
pub fn coloured(pts: Vec<XY>, closed: bool, spec: &LayerSpec, k: f64, rgb: Option<(u8, u8, u8)>, data: Option<&ExtendedDataRecord>) -> EntityType {
    let mut e = crate::emit::polylines(&[(pts, closed)], &spec.name, k, data).remove(0);
    if let Some((r, g, b)) = rgb {
        e.common_mut().color = Color::Rgb { r, g, b };
    }
    e
}

/// A text aligned left (or right) and in the middle.
pub fn side_text(ctx: &Ctx, at: XY, value: &str, mm: f64, spec: &LayerSpec, right: bool) -> EntityType {
    let mut t = ctx.text(at, value, mm, 0.0, spec, None);
    if let EntityType::Text(tx) = &mut t {
        tx.horizontal_alignment = if right { TextHorizontalAlignment::Right } else { TextHorizontalAlignment::Left };
        tx.vertical_alignment = TextVerticalAlignment::Middle;
    }
    t
}

/// "1.234" → "1,23" in Dutch, "1.23" in English.
pub fn num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s }
}

/// The parameters of a unit as one line for a legend.
pub fn unit_line(code: &str, p: &soil::Params) -> String {
    let name = soil::row(code).map(|r| r.label()).unwrap_or(code);
    let cs = p.cs.map(|c| format!(", C's {}", num(c, 0))).unwrap_or_default();
    format!("{name} — γ {}/{} kN/m³, φ' {}°, c' {} kPa, C'p {}{cs}", num(p.gamma, 1), num(p.gamma_sat, 1), num(p.phi, 1), num(p.c, 1), num(p.cp, 0))
}

/// The table of soil units for `CIV_GRONDSOORTEN`.
pub fn list_units() -> Result<String, String> {
    let rows: Vec<serde_json::Value> = soil::TABLE
        .iter()
        .map(|r| {
            let p = r.params();
            serde_json::json!({
                "code": r.code, "code_en": r.code_en, "name": r.label(), "qc_mpa": r.qc,
                "gamma": p.gamma, "gamma_sat": p.gamma_sat, "c": p.c, "phi": p.phi,
                "cp": p.cp, "cs": p.cs, "cr": p.cr, "rr": p.rr, "ca": p.ca, "cv_estimate": p.cv,
                "ranges": {"gamma": r.gamma, "gamma_sat": r.gamma_sat, "phi": r.phi, "c": r.c, "cp": r.cp, "cs": r.cs, "e100_mpa": r.e100, "cu": r.cu},
            })
        })
        .collect();
    super::api::note("units", rows);
    super::api::note("source", "NEN 9997-1 tabel 2.b (via a public reproduction; see docs/geotechnics.md); cv: estimate");
    let names: Vec<String> = soil::TABLE.iter().map(|r| r.code.to_string()).collect();
    Ok(format!("{} {}: {}. {}", soil::TABLE.len(), tr("grondsoorten (NEN 9997-1 tabel 2.b)", "soil units (NEN 9997-1 table 2.b)"), names.join(", "), disclaimer()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygons_follow_the_unit_sequence() {
        let b = |t: f64, m: f64, c: &str| Band { top: t, bottom: m, code: c.into() };
        let samples = vec![
            (0.0, vec![b(0.0, -2.0, "klei_schoon_slap"), b(-2.0, -10.0, "zand_schoon_vast")]),
            (1.0, vec![b(0.0, -2.5, "klei_schoon_slap"), b(-2.5, -10.0, "zand_schoon_vast")]),
            (2.0, vec![b(0.0, -1.0, "klei_schoon_slap"), b(-1.0, -1.5, "veen_slap"), b(-1.5, -10.0, "zand_schoon_vast")]),
        ];
        let p = band_polygons(&samples);
        assert_eq!(p.len(), 5);
        // The first run ends halfway to the third sample.
        assert!(p[0].1.iter().any(|q| (q[0] - 1.5).abs() < 1e-9));
        assert_eq!(p[3].0, "veen_slap");
    }
}
