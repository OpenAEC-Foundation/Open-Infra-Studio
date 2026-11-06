//! What a cross section cuts in plan: the BGT surfaces it crosses, drawn as a
//! band along the ground in the colour of their layer, and the KLIC cables
//! and pipes, drawn as a small circle at their (assumed) depth below the
//! ground with a short label. The roadmap's "dwarsprofielen langs een lijn
//! (AHN + gesneden BGT/KLIC-objecten)" and the trial trench (proefsleuf)
//! picture: what to expect where you dig.
//!
//! Elements are recognised by their object data (`bron=BGT` / `bron=KLIC`).

use ocs_plugin_api::host::codec::{CadDocument, EntityType};

use super::as_profiel::{Outlines, SectionView};
use crate::gis::crs::XY;
use crate::nlcs::{self, LayerSpec};

/// A BGT surface or a KLIC line in plan (drawing units), with its layer.
pub struct Cuttable {
    pub layer: LayerSpec,
    pub kind: Kind,
    pub pts: Vec<XY>,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// A closed BGT outline.
    Surface,
    /// A KLIC cable or pipe at this depth (m below ground).
    Utility(f64),
    /// A survey point at this height (m): what the trial trench found.
    Survey(f64),
}

/// A short theme code for the label under a crossing.
fn short_theme(thema: &str) -> &'static str {
    match thema {
        "laagspanning" => "LS",
        "middenspanning" => "MS",
        "hoogspanning" | "landelijkHoogspanningsnet" => "HS",
        "datatransport" => "D",
        "gasLageDruk" => "G-LD",
        "gasHogeDruk" => "G-HD",
        "water" => "W",
        "warmte" => "WA",
        "rioolVrijverval" | "rioolOnderOverOfOnderdruk" => "R",
        "petrochemie" | "buisleidingGevaarlijkeInhoud" => "CH",
        _ => "K",
    }
}

/// The layer's spec from its name: the NLCS colours when it is an NLCS layer.
fn spec_of(name: &str) -> LayerSpec {
    if let Ok(p) = nlcs::parse_layer(name) {
        if let Some(obj) = nlcs::object_by_name(&p.group, &p.object) {
            let element = match p.element.as_str() {
                "A" => nlcs::Element::A,
                "S" => nlcs::Element::S,
                "T" => nlcs::Element::T,
                _ => nlcs::Element::G,
            };
            let spec = nlcs::layer(obj, p.status, &p.discipline, element);
            if spec.name == name {
                return spec;
            }
        }
    }
    LayerSpec { name: name.to_string(), color: 7, rgb: (255, 255, 255), true_color: None, lineweight: None, linetype: "Continuous".into(), object_id: 0 }
}

/// The BGT surfaces and KLIC lines in a document.
pub fn cuttables(doc: &CadDocument) -> Vec<Cuttable> {
    let mut out = Vec::new();
    for e in doc.model_space_entities() {
        let data = crate::library::infra::host::data(e);
        let source = data.get("bron").map(String::as_str).unwrap_or("");
        if let EntityType::Point(p) = e {
            // A measured point (CIV_PUNTEN_IMPORT): number, code and height.
            if let (Some(n), Some(z)) = (data.get("puntnummer"), data.get("z").and_then(|z| z.parse::<f64>().ok())) {
                let label = match data.get("code") {
                    Some(c) if !c.is_empty() => format!("{n} {c}"),
                    _ => n.clone(),
                };
                out.push(Cuttable { layer: spec_of(&e.common().layer), kind: Kind::Survey(z), pts: vec![[p.location.x, p.location.y]], label });
            }
            continue;
        }
        let (pts, closed): (Vec<XY>, bool) = match e {
            EntityType::LwPolyline(p) => (p.vertices.iter().map(|v| [v.location.x, v.location.y]).collect(), p.is_closed),
            EntityType::Polyline2D(p) => (p.vertices.iter().map(|v| [v.location.x, v.location.y]).collect(), p.flags.is_closed()),
            EntityType::Polyline3D(p) => (p.vertices.iter().map(|v| [v.position.x, v.position.y]).collect(), p.flags.closed),
            EntityType::Line(l) => (vec![[l.start.x, l.start.y], [l.end.x, l.end.y]], false),
            _ => continue,
        };
        if pts.len() < 2 {
            continue;
        }
        let layer = e.common().layer.clone();
        let object = nlcs::parse_layer(&layer).map(|p| nlcs::humanize(&p.object)).unwrap_or_default();
        if source.eq_ignore_ascii_case("BGT") && closed && pts.len() >= 3 {
            out.push(Cuttable { layer: spec_of(&layer), kind: Kind::Surface, pts, label: object });
        } else if source.eq_ignore_ascii_case("KLIC") && !closed {
            // Casings and cable beds wrap the cables: not a crossing of their own.
            let kind = data.get("type").map(String::as_str).unwrap_or("");
            if matches!(kind, "Mantelbuis" | "Kabelbed") || layer.contains("MANTELBUIS") {
                continue;
            }
            let thema = data.get("thema").cloned().unwrap_or_default();
            let depth = data.get("diepte").and_then(|d| d.replace(',', ".").parse::<f64>().ok()).unwrap_or_else(|| super::model3d::klic_depth(&thema));
            let label = short_theme(&thema).to_string();
            out.push(Cuttable { layer: spec_of(&layer), kind: Kind::Utility(depth), pts, label });
        }
    }
    out
}

/// Where segment a–b crosses segment c–d: the parameter along a–b.
fn cross(a: XY, b: XY, c: XY, d: XY) -> Option<f64> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-12 {
        return None;
    }
    let (qx, qy) = (c[0] - a[0], c[1] - a[1]);
    let t = (qx * s[1] - qy * s[0]) / den;
    let u = (qx * r[1] - qy * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

fn inside(p: XY, ring: &[XY]) -> bool {
    let mut c = false;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// The parts of segment a–b inside a ring, as parameter intervals.
pub fn inside_intervals(a: XY, b: XY, ring: &[XY]) -> Vec<(f64, f64)> {
    let n = ring.len();
    let mut ts = vec![0.0, 1.0];
    for i in 0..n {
        if let Some(t) = cross(a, b, ring[i], ring[(i + 1) % n]) {
            ts.push(t);
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    ts.windows(2)
        .filter(|w| {
            let m = (w[0] + w[1]) / 2.0;
            inside([a[0] + (b[0] - a[0]) * m, a[1] + (b[1] - a[1]) * m], ring)
        })
        .map(|w| (w[0], w[1]))
        .collect()
}

/// Where a section line crosses a polyline: parameters along the section.
pub fn crossings(a: XY, b: XY, line: &[XY]) -> Vec<f64> {
    line.windows(2).filter_map(|w| cross(a, b, w[0], w[1])).collect()
}

/// Where point p lies along section line a–b, if it is within `near` of it
/// (drawing units): the parameter along a–b.
fn near_point(a: XY, b: XY, p: XY, near: f64) -> Option<f64> {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let len2 = ab[0] * ab[0] + ab[1] * ab[1];
    if len2 <= 0.0 {
        return None;
    }
    let t = (ap[0] * ab[0] + ap[1] * ab[1]) / len2;
    let off = (ap[0] * ab[1] - ap[1] * ab[0]).abs() / len2.sqrt();
    ((0.0..=1.0).contains(&t) && off <= near).then_some(t)
}

/// Whether one of the section lines cuts this object: runs through the
/// surface, crosses the line, or passes within `near` of the survey point.
/// Only the plan counts, not the ground, as in [`section_cuts`].
pub fn cut_by(c: &Cuttable, lines: &[(XY, XY)], near: f64) -> bool {
    lines.iter().any(|&(a, b)| match c.kind {
        Kind::Surface => !inside_intervals(a, b, &c.pts).is_empty(),
        Kind::Utility(_) => !crossings(a, b, &c.pts).is_empty(),
        Kind::Survey(_) => near_point(a, b, c.pts[0], near).is_some(),
    })
}

/// Everything of an object that [`section_cuts`] draws from: its kind and
/// depth or height, its plan points, its layer and its label.
fn object_fingerprint(c: &Cuttable) -> String {
    let (kind, value) = match c.kind {
        Kind::Surface => ("vlak", 0.0),
        Kind::Utility(depth) => ("leiding", depth),
        Kind::Survey(z) => ("meetpunt", z),
    };
    let values: Vec<f64> = std::iter::once(value).chain(c.pts.iter().flatten().copied()).collect();
    super::labels::fingerprint(&values, &[kind.to_string(), c.layer.name.clone(), c.label.clone()])
}

/// The fingerprints of what the section lines cut, per source: `bgt` (the
/// surfaces), `klic` (the cables and pipes) and `meetpunten` (the survey
/// points within `near`). The order of the objects in the drawing does not
/// count.
pub fn cut_fingerprints<'a>(things: impl IntoIterator<Item = &'a Cuttable>, lines: &[(XY, XY)], near: f64) -> Vec<(String, String)> {
    let mut per: [Vec<String>; 3] = Default::default();
    for c in things.into_iter().filter(|c| cut_by(c, lines, near)) {
        let i = match c.kind {
            Kind::Surface => 0,
            Kind::Utility(_) => 1,
            Kind::Survey(_) => 2,
        };
        per[i].push(object_fingerprint(c));
    }
    ["bgt", "klic", "meetpunten"]
        .into_iter()
        .zip(per)
        .map(|(key, mut fps)| {
            fps.sort();
            (key.to_string(), super::labels::fingerprint(&[], &fps))
        })
        .collect()
}

/// The ground height at an offset, between the sampled points.
fn ground_at(v: &SectionView, d: f64) -> Option<f64> {
    let g: Vec<(f64, f64)> = v.ground.iter().filter_map(|(o, z)| z.map(|z| (*o, z))).collect();
    let i = g.iter().position(|p| p.0 >= d)?;
    if i == 0 {
        return Some(g[0].1);
    }
    let (p, q) = (g[i - 1], g[i]);
    Some(if (q.0 - p.0).abs() < 1e-12 { q.1 } else { p.1 + (q.1 - p.1) * (d - p.0) / (q.0 - p.0) })
}

/// Outlines per view for `draw_section_views`: bands for the BGT surfaces
/// just above the ground, circles of `radius` m for the KLIC lines at their
/// depth, crosses for survey points within `near` (drawing units) of the
/// section line. Also returns per view the labels of the lines (one per
/// `label_gap` m) and points. `left`/`right` are the offsets of the section ends (m).
/// What the sections cut, counted: (BGT surface parts, KLIC crossings, survey points).
pub type Counts = (usize, usize, usize);

/// Labels per view: (offset m, height m, text, above the spot).
pub type Labels = Vec<Vec<(f64, f64, String, bool)>>;

pub fn section_cuts(things: &[Cuttable], views: &[SectionView], left: f64, right: f64, radius: f64, near: f64, label_gap: f64) -> (Outlines, Labels, Counts) {
    let mut outlines = Vec::with_capacity(views.len());
    let mut labels = Vec::with_capacity(views.len());
    let mut counts: Counts = (0, 0, 0);
    for v in views {
        let (a, b) = v.plan;
        let d_of = |t: f64| -left + t * (left + right);
        let mut lines = Vec::new();
        let mut texts = Vec::new();
        let mut found = Vec::new();
        for c in things {
            match c.kind {
                Kind::Surface => {
                    for (t0, t1) in inside_intervals(a, b, &c.pts) {
                        let (d0, d1) = (d_of(t0), d_of(t1));
                        let mut pts: Vec<(f64, f64)> = std::iter::once(d0)
                            .chain(v.ground.iter().map(|g| g.0).filter(|d| *d > d0 && *d < d1))
                            .chain(std::iter::once(d1))
                            .filter_map(|d| ground_at(v, d).map(|z| (d, z + 0.05)))
                            .collect();
                        pts.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-9);
                        if pts.len() >= 2 {
                            lines.push((c.layer.clone(), pts));
                            counts.0 += 1;
                        }
                    }
                }
                Kind::Utility(depth) => {
                    for t in crossings(a, b, &c.pts) {
                        let d = d_of(t);
                        let Some(z) = ground_at(v, d).map(|z| z - depth) else { continue };
                        let ring: Vec<(f64, f64)> = (0..=16).map(|k| {
                            let w = k as f64 / 16.0 * std::f64::consts::TAU;
                            (d + radius * w.cos(), z + radius * w.sin())
                        }).collect();
                        lines.push((c.layer.clone(), ring));
                        texts.push((d, z + radius * 1.5, c.label.clone(), true));
                        counts.1 += 1;
                    }
                }
                Kind::Survey(z) => {
                    let Some(t) = near_point(a, b, c.pts[0], near) else { continue };
                    let d = d_of(t);
                    let r = radius * 1.4;
                    lines.push((c.layer.clone(), vec![(d - r, z - r), (d + r, z + r)]));
                    lines.push((c.layer.clone(), vec![(d - r, z + r), (d + r, z - r)]));
                    found.push((d, z - r * 1.5, c.label.clone(), false));
                    counts.2 += 1;
                }
            }
        }
        // Cable labels side by side would overlap: one per `label_gap` m. The
        // measured points keep theirs (they sit lower, at the found height).
        texts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kept: Vec<(f64, f64, String, bool)> = Vec::new();
        for t in texts {
            if kept.last().is_none_or(|k| t.0 - k.0 >= label_gap) {
                kept.push(t);
            }
        }
        kept.extend(found);
        outlines.push(lines);
        labels.push(kept);
    }
    (outlines, labels, counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(plan: (XY, XY)) -> SectionView {
        SectionView { s: 0.0, ground: (0..=20).map(|i| (-10.0 + i as f64, Some(2.0))).collect(), design: Vec::new(), axis_z: Some(2.0), cut: None, fill: None, plan }
    }

    #[test]
    fn a_section_through_a_surface_and_across_a_cable() {
        let spec = spec_of("B-XX-VH-GESLOTENVERHARDING_ASFALT-G");
        let road = Cuttable { layer: spec.clone(), kind: Kind::Surface, pts: vec![[-3.0, -50.0], [3.0, -50.0], [3.0, 50.0], [-3.0, 50.0]], label: "Asfalt".into() };
        let cable = Cuttable { layer: spec_of("B-OI-KL-ET_LS-G"), kind: Kind::Utility(0.6), pts: vec![[5.0, -50.0], [5.0, 50.0]], label: "Laagspanning".into() };
        // Section from x = -10 to +10 at y = 0 (offsets −10 … +10).
        let v = view(([-10.0, 0.0], [10.0, 0.0]));
        let found = Cuttable { layer: spec_of("N-XX-MEETPUNT-S"), kind: Kind::Survey(1.25), pts: vec![[5.1, 0.4]], label: "12 KABEL".into() };
        let far = Cuttable { layer: spec_of("N-XX-MEETPUNT-S"), kind: Kind::Survey(1.0), pts: vec![[5.0, 3.0]], label: "13".into() };
        let (o, l, n) = section_cuts(&[road, cable, found, far], &[v], 10.0, 10.0, 0.15, 1.0, 0.7);
        assert_eq!(n, (1, 1, 1));
        assert_eq!(o[0].len(), 4, "band, ring and the cross of the point within 1 m");
        assert_eq!(l[0].len(), 2);
        assert_eq!(l[0][1].2, "12 KABEL");
        assert!((l[0][1].0 - 5.1).abs() < 1e-9);
        let band = &o[0][0].1;
        assert!((band[0].0 + 3.0).abs() < 1e-9 && (band.last().unwrap().0 - 3.0).abs() < 1e-9, "{band:?}");
        assert!(band.iter().all(|p| (p.1 - 2.05).abs() < 1e-9));
        let ring = &o[0][1].1;
        assert!((ring[0].0 - 5.15).abs() < 1e-9 && (ring[4].1 - (2.0 - 0.6 + 0.15)).abs() < 1e-9, "{ring:?}");
        assert_eq!(l[0][0].2, "Laagspanning");
        assert_eq!(o[0][0].0.name, "B-XX-VH-GESLOTENVERHARDING_ASFALT-G");
    }

    #[test]
    fn intervals_of_a_concave_surface() {
        // A U shape: the section crosses both legs.
        let u = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [7.0, 10.0], [7.0, 3.0], [3.0, 3.0], [3.0, 10.0], [0.0, 10.0]];
        let iv = inside_intervals([-1.0, 5.0], [11.0, 5.0], &u);
        assert_eq!(iv.len(), 2, "{iv:?}");
        assert!((iv[0].0 * 12.0 - 1.0).abs() < 1e-9 && (iv[0].1 * 12.0 - 4.0).abs() < 1e-9);
    }
}
