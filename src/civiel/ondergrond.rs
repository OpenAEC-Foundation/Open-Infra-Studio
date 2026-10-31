//! The subsurface from the BRO: cone penetration tests and boreholes of the
//! project area. In plan a symbol with its BRO id (`GIS_Q_SONDERINGEN`); in
//! 3D a column per test from the surface level (m NAP) down, one mesh per
//! soil layer in its conventional colour, with the cone resistance and
//! friction ratio as graphs beside it (`CIV_SONDERINGEN3D`).
//!
//! Layers: NLCS MO-ONDERZOEK_SONDERING and MO-ONDERZOEK_BORING (main group
//! "Milieu en Onderzoek", the NLCS objects for site investigation), G for
//! the columns and graphs, S for the plan symbol, T for its labels.

use ocs_plugin_api::host::codec::entities::Polyline3D;
use ocs_plugin_api::host::codec::{Color, EntityType};

use super::model3d::{MeshBuilder, P3};
use super::{record, v3, CivielOptions, Ctx, Out};
use crate::gis::bro::{self, Grond, Interval, Kind, Point};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::{Element, LayerSpec};

/// What `CIV_SONDERINGEN3D` draws.
#[derive(Clone, Debug)]
pub struct Request {
    pub kinds: Vec<Kind>,
    /// At most this many of each kind, nearest to the project centre first.
    pub max: usize,
    /// Column radius (m).
    pub radius: f64,
    /// Thinnest CPT layer (m); thinner ones join their neighbour.
    pub min_layer: f64,
    /// qc and Rf graphs beside each CPT column.
    pub graphs: bool,
    /// Plan symbols and labels too.
    pub labels: bool,
}

impl Default for Request {
    fn default() -> Self {
        Self { kinds: vec![Kind::Cpt, Kind::Bhr], max: 100, radius: 0.4, min_layer: 0.2, graphs: true, labels: true }
    }
}

/// `BRON=`: sonderingen, boringen or alles.
pub fn kinds(arg: Option<&str>) -> Result<Vec<Kind>, String> {
    match arg.map(|s| s.to_ascii_lowercase()) {
        None => Ok(vec![Kind::Cpt, Kind::Bhr]),
        Some(s) if s.starts_with("son") || s == "cpt" => Ok(vec![Kind::Cpt]),
        Some(s) if s.starts_with("bor") || s.starts_with("bhr") => Ok(vec![Kind::Bhr]),
        Some(s) if s == "alles" || s == "beide" || s == "all" => Ok(vec![Kind::Cpt, Kind::Bhr]),
        Some(_) => Err("BRON: sonderingen, boringen of alles".into()),
    }
}

fn layer(ctx: &Ctx, kind: Kind, el: Element) -> Result<LayerSpec, String> {
    ctx.layer("MO", if kind == Kind::Cpt { "ONDERZOEK_SONDERING" } else { "ONDERZOEK_BORING" }, el)
}

/// The BRO objects of the project area with their drawing position, nearest
/// to the centre first; the second value counts what `max` left out.
fn find(ctx: &Ctx, kind: Kind, max: usize, progress: &dyn Fn(String)) -> Result<(Vec<(Point, XY)>, usize), String> {
    let site = ctx.site.clone().ok_or("stel eerst de projectlocatie in (GIS → Locatie, of GIS_SITE)")?;
    let rd = crate::basisdata::rd_bbox(&site)?;
    let ll = crate::gis::site::envelope(rd, &crate::gis::crs::Transformer::new("EPSG:28992", "EPSG:4326")?)?;
    progress(format!("BRO: {}en zoeken…", kind.label()));
    let found = bro::search(kind, ll)?;
    let to = crate::gis::vector::data_to_site(&site, "EPSG:28992", &[])?;
    let rect = site.drawing_rect(ctx.f);
    let mut pts: Vec<(Point, XY)> = found
        .into_iter()
        .filter_map(|p| {
            let at_rd = p.rd.or_else(|| p.lonlat.and_then(|l| crate::gis::crs::transform("EPSG:4326", "EPSG:28992", l).ok()))?;
            let at = site.to_drawing(to(at_rd)?, ctx.f);
            crate::gis::vector::inside(at, rect).then_some((p, at))
        })
        .collect();
    let c = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    pts.sort_by(|a, b| (a.1[0] - c[0]).hypot(a.1[1] - c[1]).total_cmp(&(b.1[0] - c[0]).hypot(b.1[1] - c[1])));
    let cut = pts.len().saturating_sub(max);
    pts.truncate(max);
    Ok((pts, cut))
}

fn point_data(p: &Point) -> Vec<(&'static str, String)> {
    vec![
        ("bron", format!("BRO {}", if p.cpt { "CPT" } else { "BHR-GT" })),
        ("bro_id", p.id.clone()),
        ("datum", p.date.clone()),
        ("maaiveld_m_nap", p.surface.map(|z| format!("{z:.3}")).unwrap_or_default()),
        ("referentie", format!("{} {}", p.reference, p.datum).trim().to_string()),
        ("einddiepte_m", p.end_depth.map(|d| format!("{d:.2}")).unwrap_or_default()),
        ("kwaliteitsregime", p.regime.clone()),
    ]
}

/// "25.2" → "25,2".
fn comma(v: f64, decimals: usize) -> String {
    format!("{v:.decimals$}").replace('.', ",")
}

/// The plan symbol: a circle with a cone (sondering) or a cross (boring),
/// the BRO id above, surface level and end depth below.
fn plan_symbol(ctx: &Ctx, p: &Point, at: XY, out: &mut Out) -> Result<(), String> {
    let kind = p.kind();
    let (s, t) = (layer(ctx, kind, Element::S)?, layer(ctx, kind, Element::T)?);
    let d = record(&point_data(p));
    let r = 1.2 * ctx.k;
    let mut ents = crate::emit::markers(&[at], &s.name, r, Some(&d));
    let lines: Vec<(Vec<XY>, bool)> = if kind == Kind::Cpt {
        let a = |deg: f64| [at[0] + r * deg.to_radians().cos(), at[1] + r * deg.to_radians().sin()];
        vec![(vec![a(150.0), a(30.0), a(270.0)], true)]
    } else {
        let q = r * std::f64::consts::FRAC_1_SQRT_2;
        vec![(vec![[at[0] - q, at[1] - q], [at[0] + q, at[1] + q]], false), (vec![[at[0] - q, at[1] + q], [at[0] + q, at[1] - q]], false)]
    };
    ents.extend(crate::emit::polylines(&lines, &s.name, ctx.k, Some(&d)));
    out.put(&s, ents);
    let h = 1.8;
    let below = match (p.surface, p.end_depth) {
        (Some(z), Some(e)) => format!("mv {} · {} m", nap(z, 2), comma(e, 1)),
        (Some(z), None) => format!("mv {}", nap(z, 2)),
        (None, Some(e)) => format!("{} m", comma(e, 1)),
        _ => String::new(),
    };
    let mut texts = vec![ctx.text([at[0], at[1] + r + 1.2 * h * ctx.k], &p.id, h, 0.0, &t, Some(&d))];
    if !below.is_empty() {
        texts.push(ctx.text([at[0], at[1] - r - 1.2 * h * ctx.k], &below, h, 0.0, &t, Some(&d)));
    }
    out.put(&t, texts);
    Ok(())
}

/// `GIS_Q_SONDERINGEN`: the CPTs and boreholes of the project area in plan
/// (from the search only, no measurements fetched).
pub fn overview(opt: &CivielOptions, kinds: &[Kind], progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let mut out = Out::default();
    let mut counts = Vec::new();
    let mut ids = Vec::new();
    for kind in kinds {
        let (pts, _) = find(&ctx, *kind, usize::MAX, progress)?;
        for (p, at) in &pts {
            plan_symbol(&ctx, p, *at, &mut out)?;
            ids.push(serde_json::json!({"bro_id": p.id, "soort": kind.label(), "maaiveld_m_nap": p.surface, "einddiepte_m": p.end_depth, "datum": p.date}));
        }
        counts.push(format!("{} {}en", pts.len(), kind.label()));
    }
    if ids.is_empty() {
        return Err("geen sonderingen of boringen in de BRO voor het projectgebied (alleen Nederland)".into());
    }
    let n = out.write("BRO sonderingen en boringen")?;
    crate::bridge::note_source("BRO sonderingen en boringen (TNO / BRO, CC0)");
    super::api::note("objecten", ids);
    Ok(format!("BRO: {} — {n} elementen op MO-ONDERZOEK_SONDERING en _BORING.", counts.join(", ")))
}

/// A vertical prism of `sides` sides from `z_top` down to `z_bottom`
/// (drawing units), smooth sides and flat caps.
pub fn prism(mb: &mut MeshBuilder, c: XY, r: f64, sides: usize, z_top: f64, z_bottom: f64) {
    let at = |i: usize, z: f64| -> P3 {
        let a = std::f64::consts::TAU * (i % sides) as f64 / sides as f64;
        [c[0] + r * a.cos(), c[1] + r * a.sin(), z]
    };
    mb.new_group();
    for i in 0..sides {
        mb.quad(at(i, z_bottom), at(i + 1, z_bottom), at(i + 1, z_top), at(i, z_top));
    }
    mb.new_group();
    for i in 0..sides {
        mb.tri([c[0], c[1], z_top], at(i, z_top), at(i + 1, z_top));
    }
    mb.new_group();
    for i in 0..sides {
        mb.tri([c[0], c[1], z_bottom], at(i + 1, z_bottom), at(i, z_bottom));
    }
}

fn rgb(g: Grond) -> Color {
    let (r, gg, b) = g.rgb();
    Color::Rgb { r, g: gg, b }
}

/// One column: a mesh per layer on the G layer, with object data.
#[allow(clippy::too_many_arguments)]
fn column(ctx: &Ctx, p: &Point, at: XY, z0: f64, layers: &[Interval], radius: f64, spec: &LayerSpec, out: &mut Out) -> usize {
    let f = ctx.f;
    let sides = if p.cpt { 12 } else { 8 };
    let mut n = 0;
    for l in layers {
        let (top, bottom) = (z0 - l.top, z0 - l.bottom);
        let mut mb = MeshBuilder::default();
        prism(&mut mb, at, radius * f, sides, top * f, bottom * f);
        let mut pairs = point_data(p);
        pairs.extend([
            ("grondsoort", l.soil.label().to_string()),
            ("omschrijving", l.name.clone()),
            ("bovenkant_m_nap", format!("{top:.2}")),
            ("onderkant_m_nap", format!("{bottom:.2}")),
            ("dikte_m", format!("{:.2}", l.thickness())),
        ]);
        if let Some(q) = l.qc {
            pairs.push(("qc_gem_mpa", format!("{q:.2}")));
        }
        if let Some(r) = l.rf {
            pairs.push(("rf_gem_pct", format!("{r:.2}")));
        }
        if p.cpt {
            pairs.push(("classificatie", "Robertson 2010 (SBT, qc en Rf)".into()));
        }
        let d = record(&pairs);
        out.put(spec, mb.finish(&spec.name, Some(rgb(l.soil)), Some(&d)));
        n += 1;
    }
    n
}

/// qc (red, to the east, 0.1 m per MPa) and Rf (blue, to the west, 0.25 m
/// per %) as 3D lines beside a CPT column, with a zero line for each.
#[allow(clippy::too_many_arguments)]
fn graphs(ctx: &Ctx, p: &Point, at: XY, z0: f64, samples: &[bro::Sample], radius: f64, spec: &LayerSpec, out: &mut Out) {
    let f = ctx.f;
    if samples.len() < 2 {
        return;
    }
    let base = (radius + 0.2) * f;
    let z = |s: &bro::Sample| (z0 - s.depth) * f;
    let d = record(&[("bron", "BRO CPT".into()), ("bro_id", p.id.clone()), ("grafiek", "qc 0,1 m/MPa (oost), Rf 0,25 m/% (west)".into())]);
    let line = |pts: Vec<[f64; 3]>, c: Color| {
        let mut e = EntityType::Polyline3D(Polyline3D::from_points(pts.into_iter().map(|q| v3([q[0], q[1]], q[2])).collect()));
        let cm = e.common_mut();
        cm.layer = spec.name.clone();
        cm.color = c;
        cm.extended_data.add_record(d.clone());
        e
    };
    let (top, bottom) = (z(&samples[0]), z(&samples[samples.len() - 1]));
    let red = Color::Rgb { r: 214, g: 40, b: 40 };
    let blue = Color::Rgb { r: 40, g: 92, b: 214 };
    let grey = Color::Rgb { r: 120, g: 120, b: 120 };
    let qc: Vec<[f64; 3]> = samples.iter().map(|s| [at[0] + base + s.qc.min(60.0) * 0.1 * f, at[1], z(s)]).collect();
    let rf: Vec<[f64; 3]> = samples.iter().filter_map(|s| s.rf.map(|r| [at[0] - base - r.min(12.0) * 0.25 * f, at[1], z(s)])).collect();
    out.put(spec, [line(vec![[at[0] + base, at[1], top], [at[0] + base, at[1], bottom]], grey), line(qc, red)]);
    if rf.len() >= 2 {
        out.put(spec, [line(vec![[at[0] - base, at[1], top], [at[0] - base, at[1], bottom]], grey), line(rf, blue)]);
    }
}

/// (head, layers, CPT samples) of one fetched object.
type Fetched = Result<(Point, Vec<Interval>, Vec<bro::Sample>), String>;

/// `CIV_SONDERINGEN3D`: CPTs and boreholes of the project area as 3D
/// columns of soil layers, from the surface level (m NAP) down.
pub fn columns(opt: &CivielOptions, req: &Request, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let mut out = Out::default();
    let mut bron: Option<super::Hoogtebron> = None;
    let (mut made, mut segs, mut failed, mut cut_total) = (0usize, 0usize, Vec::new(), 0usize);
    let mut counts = Vec::new();
    let mut soils: std::collections::BTreeMap<&'static str, f64> = Default::default();
    let mut list = Vec::new();
    for kind in &req.kinds {
        let (pts, cut) = find(&ctx, *kind, req.max, progress)?;
        cut_total += cut;
        if pts.is_empty() {
            counts.push(format!("0 {}en", kind.label()));
            continue;
        }
        let spec = layer(&ctx, *kind, Element::G)?;
        let ids: Vec<String> = pts.iter().map(|(p, _)| p.id.clone()).collect();
        let fetched: Vec<Fetched> = match kind {
            Kind::Cpt => bro::fetch_many(&ids, bro::fetch_cpt, progress)
                .into_iter()
                .map(|r| r.map(|c| (c.point, bro::cpt_intervals(&c.samples, req.min_layer), c.samples)))
                .collect(),
            Kind::Bhr => bro::fetch_many(&ids, bro::fetch_bhr, progress).into_iter().map(|r| r.map(|b| (b.point, b.layers, Vec::new()))).collect(),
        };
        let mut k = 0;
        for ((head, at), r) in pts.iter().zip(fetched) {
            let (full, layers, samples) = match r {
                Ok(x) => x,
                Err(e) => {
                    failed.push(e);
                    continue;
                }
            };
            if layers.is_empty() {
                failed.push(format!("{}: geen lagen", head.id));
                continue;
            }
            // The surface level of the test; without one, the AHN ground.
            let z0 = match full.surface.or(head.surface) {
                Some(z) => z,
                None => {
                    if bron.is_none() {
                        bron = Some(super::hoogtebron(&ctx)?);
                    }
                    bron.as_mut().and_then(|b| b.height(*at)).unwrap_or(0.0)
                }
            };
            let p = Point { surface: Some(z0), ..full };
            segs += column(&ctx, &p, *at, z0, &layers, req.radius, &spec, &mut out);
            if req.graphs && p.cpt {
                graphs(&ctx, &p, *at, z0, &samples, req.radius, &spec, &mut out);
            }
            if req.labels {
                plan_symbol(&ctx, &p, *at, &mut out)?;
            }
            for l in &layers {
                *soils.entry(l.soil.label()).or_default() += l.thickness();
            }
            list.push(serde_json::json!({
                "bro_id": p.id, "soort": kind.label(), "maaiveld_m_nap": z0, "einddiepte_m": p.end_depth, "datum": p.date,
                "lagen": layers.iter().map(|l| serde_json::json!({"grondsoort": l.soil.label(), "omschrijving": l.name, "bovenkant_m_nap": z0 - l.top, "onderkant_m_nap": z0 - l.bottom})).collect::<Vec<_>>(),
            }));
            k += 1;
            made += 1;
        }
        counts.push(format!("{k} {}en", kind.label()));
    }
    if made == 0 {
        return Err(match failed.first() {
            Some(e) => format!("geen kolommen getekend ({e})"),
            None => "geen sonderingen of boringen in de BRO voor het projectgebied (alleen Nederland)".into(),
        });
    }
    progress("Sonderingen in 3D tekenen…".into());
    let n = out.write("BRO sonderingen 3D")?;
    crate::bridge::note_source("BRO sonderingen en boringen (TNO / BRO, CC0)");
    if let Some(b) = &bron {
        crate::bridge::note_source(b.attribution());
    }
    let total: f64 = soils.values().sum();
    let mix: Vec<String> = soils.iter().filter(|(_, h)| **h > 0.0).map(|(s, h)| format!("{s} {:.0} %", h / total.max(1e-9) * 100.0)).collect();
    super::api::note("kolommen", list);
    super::api::note("lagen", segs);
    super::api::note("overgeslagen", failed.clone());
    super::api::note("afgekapt", cut_total);
    Ok(format!(
        "Sonderingen 3D: {} met {segs} lagen ({}){}{} — {n} elementen op MO-ONDERZOEK_SONDERING en _BORING.",
        counts.join(", "),
        mix.join(", "),
        if failed.is_empty() { String::new() } else { format!(", {} overgeslagen", failed.len()) },
        if cut_total > 0 { format!(", {cut_total} verder weg niet getekend (verhoog MAX=)") } else { String::new() },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prism_is_closed() {
        let mut mb = MeshBuilder::default();
        prism(&mut mb, [10.0, 20.0], 0.4, 12, 2.0, -3.0);
        assert_eq!(mb.faces(), 36);
        let e = mb.finish("L", Some(rgb(Grond::Zand)), None);
        let EntityType::Mesh(m) = &e[0] else { panic!() };
        let (lo, hi) = m.vertices.iter().fold((f64::MAX, f64::MIN), |b, v| (b.0.min(v.z), b.1.max(v.z)));
        assert_eq!((lo, hi), (-3.0, 2.0));
        assert!(m.vertices.iter().all(|v| ((v.x - 10.0).hypot(v.y - 20.0) - 0.4).abs() < 1e-9 || (v.x == 10.0 && v.y == 20.0)));
        assert_eq!(kinds(Some("boringen")).unwrap(), vec![Kind::Bhr]);
        assert_eq!(kinds(None).unwrap().len(), 2);
        assert!(kinds(Some("x")).is_err());
        assert_eq!(comma(25.225, 1), "25,2");
    }
}
