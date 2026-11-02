//! The exchange model: what the LandXML export writes, gathered from the
//! drawing's Open Infra objects (the object index `OPENINFRA/INDEX`
//! and the object data `OPENINFRA` on the entities). Everything is in
//! project coordinates: easting and northing in metres in the site's CRS,
//! heights in metres (NAP in the Netherlands).

use std::collections::{HashMap, HashSet};

use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use super::as_profiel::{Alignment, Pvi, Seg};
use super::landmeten::SurveyPoint;
use super::terrein::{Tin, P3};
use super::{data_value, Ctx, Maaiveld};
use crate::lang::tr;
use crate::gis::crs::XY;

/// The site's coordinate reference system, as LandXML writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Crs {
    /// "EPSG:28992".
    pub code: String,
    pub epsg: Option<u32>,
    pub name: String,
    /// The height reference: "NAP" in the Netherlands, else "MSL".
    pub vertical: String,
    /// The site's base point (m).
    pub base: XY,
}

impl Crs {
    pub fn of_site(site: &crate::gis::site::Site) -> Crs {
        let epsg = site.crs.strip_prefix("EPSG:").or(site.crs.strip_prefix("epsg:")).and_then(|c| c.trim().parse().ok());
        let nl = crate::gis::crs::in_netherlands(site.lon, site.lat);
        Crs {
            code: site.crs.clone(),
            epsg,
            name: if site.crs_name.is_empty() { site.crs.clone() } else { site.crs_name.clone() },
            vertical: if nl { "NAP".into() } else { "MSL".into() },
            base: [site.origin_e, site.origin_n],
        }
    }
}

/// A TIN: existing ground (status B) or a design (N).
#[derive(Clone, Debug, Default)]
pub struct Surface {
    pub name: String,
    /// NLCS drawing status: 'B' bestaand (existing), 'N' nieuw (design).
    pub status: char,
    pub source: String,
    /// Easting, northing, height (m).
    pub points: Vec<P3>,
    /// Counter-clockwise seen from above.
    pub triangles: Vec<[usize; 3]>,
    /// The corridor whose design surface this is.
    pub corridor: Option<String>,
}

impl Surface {
    pub fn range(&self) -> (f64, f64) {
        self.points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])))
    }

    /// Plan area (m²).
    pub fn area(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.points[i]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs() / 2.0
            })
            .sum()
    }
}

/// An alignment with its profiles.
#[derive(Clone, Debug)]
pub struct Axis {
    pub name: String,
    /// Station at the start (m).
    pub start: f64,
    /// The pieces in project metres.
    pub segs: Vec<Seg>,
    /// The ground along it: the surface's name and (distance from the
    /// start, height) in metres.
    pub ground: Option<(String, Vec<(f64, f64)>)>,
    /// The design profile: PVIs at distances from the start (m).
    pub design: Option<Vec<Pvi>>,
}

impl Axis {
    pub fn length(&self) -> f64 {
        self.segs.iter().map(Seg::length).sum()
    }
}

/// What a corridor feature line marks, from its NLCS layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Axis,
    /// Edge of the paved lanes (asphalt).
    Pavement,
    /// Outer edge of the shoulder.
    Shoulder,
    /// Edge of a dike crest.
    Crest,
    /// Toe of a fill slope (daylight in fill).
    Toe,
    /// Top of a cut slope (daylight in cut).
    Top,
    Other,
}

impl LineKind {
    pub fn from_layer(layer: &str) -> LineKind {
        let l = layer.to_ascii_uppercase();
        if l.contains("-AM-AS-") {
            LineKind::Axis
        } else if l.contains("VERHARDING") {
            LineKind::Pavement
        } else if l.contains("GRAS_BERM") {
            LineKind::Shoulder
        } else if l.contains("DIJK_KRUIN") {
            LineKind::Crest
        } else if l.contains("TALUD_TEEN") {
            LineKind::Toe
        } else if l.contains("TALUD_KRUIN") {
            LineKind::Top
        } else {
            LineKind::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LineKind::Axis => tr("As", "Axis"),
            LineKind::Pavement => tr("Kant verharding", "Pavement edge"),
            LineKind::Shoulder => tr("Berm", "Shoulder"),
            LineKind::Crest => tr("Kruin", "Crest"),
            LineKind::Toe => tr("Teen talud", "Toe of slope"),
            LineKind::Top => tr("Insteek talud", "Top of slope"),
            LineKind::Other => tr("Kantlijn", "Feature line"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FeatureLine {
    pub name: String,
    pub kind: LineKind,
    /// Offset from the axis at its first point (m, right > 0), when the
    /// corridor's axis is in the drawing.
    pub offset: Option<f64>,
    pub pts: Vec<P3>,
}

#[derive(Clone, Debug, Default)]
pub struct Corridor {
    pub name: String,
    pub axis: String,
    /// Assembly name (weg, fietspad, snelweg, dijk, …).
    pub template: String,
    pub lines: Vec<FeatureLine>,
    /// Its design surface: an index into `Model::surfaces`.
    pub surface: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    /// Project name: the site label or the drawing name.
    pub name: String,
    pub crs: Option<Crs>,
    pub points: Vec<SurveyPoint>,
    pub surfaces: Vec<Surface>,
    pub axes: Vec<Axis>,
    pub corridors: Vec<Corridor>,
}

impl Model {
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.surfaces.is_empty() && self.axes.is_empty() && self.corridors.is_empty()
    }
}

/// An axis piece from the drawing to project metres: a translation and a
/// scale (the site has no rotation), so angles stay as they are.
///
/// `Seg::Spiral` (stream as-spoor, a clothoid): move its start point like a
/// line's points and divide its length and radii by `ctx.f`; directions and
/// the turning side stay.
pub fn seg_to_project(ctx: &Ctx, s: &Seg) -> Seg {
    match s {
        Seg::Line { a, b } => Seg::Line { a: ctx.from_drawing(*a), b: ctx.from_drawing(*b) },
        Seg::Arc { c, r, a0, sweep } => Seg::Arc { c: ctx.from_drawing(*c), r: r / ctx.f, a0: *a0, sweep: *sweep },
    }
}

/// Which objects to export: everything, or those with an entity in the
/// selection (when there is one).
pub struct Scope {
    pub all: bool,
    pub selection: HashSet<Handle>,
}

impl Scope {
    pub fn everything() -> Scope {
        Scope { all: true, selection: HashSet::new() }
    }

    pub fn wants<'a>(&self, handles: impl IntoIterator<Item = &'a Handle>) -> bool {
        self.all || self.selection.is_empty() || handles.into_iter().any(|h| self.selection.contains(h))
    }
}

/// All values of an entity's object data, as one key.
fn record_key(e: &EntityType) -> String {
    e.common()
        .extended_data
        .get_record(crate::bridge::XDATA_APP)
        .map(|r| {
            r.values
                .iter()
                .filter_map(|v| match v {
                    ocs_plugin_api::host::XDataValue::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
        .unwrap_or_default()
}

/// `name`, or `name (2)`, … when it is taken.
fn unique(name: &str, taken: &mut HashSet<String>) -> String {
    let mut n = name.to_string();
    let mut i = 2;
    while !taken.insert(n.clone()) {
        n = format!("{name} ({i})");
        i += 1;
    }
    n
}

/// Read the drawing: the Open Infra objects of the index, or of the whole
/// drawing when Open CAD Studio shares it.
pub fn gather(ctx: &Ctx, all: bool, progress: &dyn Fn(String)) -> Result<Model, String> {
    progress(tr("Tekening lezen…", "Reading the drawing…").into());
    let doc = super::document(&["tin", "as", "lengteprofiel", "ontwerpprofiel", "corridor", "punten"], &[])?;
    let selection: HashSet<Handle> = if all { HashSet::new() } else { crate::bridge::selection().unwrap_or_default().into_iter().collect() };
    let index = crate::bridge::index_read();
    let scope = Scope { all, selection };
    let mut ground = Ground::new(Box::new(|| Maaiveld::find(ctx, &doc).ok()));
    let mut model = gather_from(ctx, &doc, &scope, &index, &mut ground, progress)?;
    model.name = ctx.site.as_ref().map(|s| s.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(crate::bridge::drawing_name);
    Ok(model)
}

/// The existing ground, made when first needed (AHN means downloads).
pub struct Ground<'a> {
    make: Box<dyn FnMut() -> Option<Maaiveld> + 'a>,
    made: Option<Option<Maaiveld>>,
}

impl<'a> Ground<'a> {
    pub fn new(make: Box<dyn FnMut() -> Option<Maaiveld> + 'a>) -> Ground<'a> {
        Ground { make, made: None }
    }

    pub fn get(&mut self) -> Option<&mut Maaiveld> {
        if self.made.is_none() {
            self.made = Some((self.make)());
        }
        self.made.as_mut().and_then(Option::as_mut)
    }
}

/// [`gather`] on a document in hand: `index` is the object index (kind,
/// key, handles); `ground` gives the existing ground (TIN or AHN) when a
/// ground profile needs it.
pub fn gather_from(
    ctx: &Ctx,
    doc: &CadDocument,
    scope: &Scope,
    index: &[(String, String, Vec<Handle>)],
    ground: &mut Ground,
    progress: &dyn Fn(String),
) -> Result<Model, String> {
    let f = ctx.f;
    let mut model = Model { crs: ctx.site.as_ref().map(Crs::of_site), ..Default::default() };
    let to_project = |p: P3| -> P3 {
        let q = ctx.from_drawing([p[0], p[1]]);
        [q[0], q[1], p[2] / f]
    };

    // Survey points: on OG-HOOGTEPUNT, or with a point number.
    let survey: HashSet<Handle> = doc
        .model_space_entities()
        .filter(|e| matches!(e, EntityType::Point(_)) && (e.common().layer.contains("-OG-HOOGTEPUNT-") || data_value(e, "puntnummer").is_some()))
        .map(|e| e.common().handle)
        .filter(|h| scope.wants([h]))
        .collect();
    if !survey.is_empty() {
        model.points = super::landmeten::collect(doc, ctx, &survey);
    }

    // Surfaces: the meshes and faces on OG-TRIANGULATIE, one surface per
    // object data record (a TIN's meshes all carry the same record).
    progress(tr("Oppervlakken lezen…", "Reading surfaces…").into());
    let mut groups: Vec<(char, String, Vec<Handle>, &EntityType)> = Vec::new();
    for e in doc.model_space_entities() {
        let layer = &e.common().layer;
        if !matches!(e, EntityType::Mesh(_) | EntityType::Face3D(_)) || !layer.contains("-OG-TRIANGULATIE-") {
            continue;
        }
        let status = layer.chars().next().unwrap_or('B').to_ascii_uppercase();
        let key = record_key(e);
        match groups.iter_mut().find(|g| g.0 == status && g.1 == key) {
            Some(g) => g.2.push(e.common().handle),
            None => groups.push((status, key, vec![e.common().handle], e)),
        }
    }
    let mut taken = HashSet::new();
    let mut surface_handles: Vec<Vec<Handle>> = Vec::new();
    let mut unnamed = 0;
    for (status, _, handles, first) in &groups {
        let only: HashSet<Handle> = handles.iter().copied().collect();
        let Some(tin) = Tin::from_document_with(doc, &only) else { continue };
        let corridor = data_value(first, "corridor");
        let name = data_value(first, "terrein").or_else(|| corridor.clone()).unwrap_or_else(|| {
            unnamed += 1;
            format!("{} {unnamed}", if *status == 'N' { tr("Ontwerp", "Design") } else { tr("Maaiveld", "Ground") })
        });
        let source = data_value(first, "bron").or_else(|| data_value(first, "sjabloon").map(|s| format!("corridor, {} {s}", tr("sjabloon", "assembly")))).unwrap_or_default();
        model.surfaces.push(Surface {
            name: unique(&name, &mut taken),
            status: *status,
            source,
            points: tin.points.iter().map(|p| to_project(*p)).collect(),
            triangles: tin.triangles.clone(),
            corridor,
        });
        surface_handles.push(handles.clone());
    }

    // Axes: polylines on AM-AS with an axis name; not offset lines,
    // design profiles, profile frames or corridor lines.
    let mut taken = HashSet::new();
    let mut axes_drawing: HashMap<String, Alignment> = HashMap::new();
    let has_b_tin = model.surfaces.iter().any(|s| s.status == 'B');
    for e in doc.model_space_entities() {
        if !matches!(e, EntityType::LwPolyline(_)) || !e.common().layer.contains("-AM-AS-") {
            continue;
        }
        if data_value(e, "as").is_none() || ["ontwerpprofiel", "verschuiving_m", "lengteprofiel", "corridor"].iter().any(|k| data_value(e, k).is_some()) {
            continue;
        }
        let Some(al) = Alignment::from_entity(e) else { continue };
        axes_drawing.entry(al.name.clone()).or_insert_with(|| al.clone());
        if !scope.wants([&e.common().handle]) {
            continue;
        }
        let raw = if al.name.is_empty() { "As".to_string() } else { al.name.clone() };
        let name = unique(&raw, &mut taken);
        // The ground line: wherever the drawing has a ground TIN, or the
        // axis has a length profile (then from AHN when there is no TIN).
        let framed = doc.model_space_entities().any(|x| data_value(x, "lengteprofiel").is_some() && data_value(x, "as").as_deref() == Some(al.name.as_str()));
        let mut profile = None;
        if has_b_tin || framed {
            if let Some(g) = ground.get() {
                progress(format!("{} {name}…", tr("Maaiveld langs", "Ground along")));
                let step = ctx.opt.step.max(0.5) * f;
                let mut pts = Vec::new();
                for s in al.stations(step, true) {
                    if let Some(z) = g.height(al.at(s).0, f) {
                        pts.push((s / f, z));
                    }
                }
                let label = match g {
                    Maaiveld::Tin(_) => model.surfaces.iter().find(|s| s.status == 'B').map(|s| s.name.clone()).unwrap_or_else(|| tr("Maaiveld", "Ground").into()),
                    Maaiveld::Bron(b) => b.label(),
                    Maaiveld::Puntenwolk(p) => p.label(),
                };
                if pts.len() >= 2 {
                    profile = Some((label, pts));
                }
            }
        }
        model.axes.push(Axis {
            name,
            start: al.start,
            segs: al.segs.iter().map(|s| seg_to_project(ctx, s)).collect(),
            ground: profile,
            design: super::corridor::profile_of(doc, &al.name),
        });
    }

    // Corridors: the feature lines of the index, the design surface by its
    // object data.
    let mut names: Vec<(String, Vec<Handle>)> = index.iter().filter(|(k, _, _)| k == "corridor").map(|(_, n, hs)| (n.clone(), hs.clone())).collect();
    for s in &model.surfaces {
        if let Some(c) = &s.corridor {
            if !names.iter().any(|(n, _)| n == c) {
                names.push((c.clone(), Vec::new()));
            }
        }
    }
    for (name, handles) in names {
        let surface = model.surfaces.iter().position(|s| s.corridor.as_deref() == Some(name.as_str()));
        let mut mine: Vec<&Handle> = handles.iter().collect();
        if let Some(i) = surface {
            mine.extend(surface_handles[i].iter());
        }
        if !scope.wants(mine) {
            continue;
        }
        let lines: Vec<&EntityType> = handles.iter().filter_map(|h| doc.get_entity(*h)).filter(|e| matches!(e, EntityType::Polyline3D(_))).collect();
        let axis_line = lines.iter().find(|e| data_value(e, "corridor").is_some());
        let mesh = surface.and_then(|i| surface_handles[i].first()).and_then(|h| doc.get_entity(*h));
        let info = |k: &str| axis_line.and_then(|e| data_value(e, k)).or_else(|| mesh.and_then(|e| data_value(e, k))).unwrap_or_default();
        let axis_name = info("as");
        let al = axes_drawing.get(&axis_name);
        let mut c = Corridor { name: name.clone(), axis: axis_name.clone(), template: info("sjabloon"), surface, ..Default::default() };
        let mut count: HashMap<(LineKind, i8), usize> = HashMap::new();
        for e in &lines {
            let EntityType::Polyline3D(p) = e else { continue };
            let pts: Vec<P3> = p.vertices.iter().map(|v| [v.position.x, v.position.y, v.position.z]).collect();
            if pts.len() < 2 {
                continue;
            }
            let kind = LineKind::from_layer(&e.common().layer);
            let offset = al.map(|a| a.station_offset([pts[0][0], pts[0][1]]).1 / f);
            let side: i8 = match offset {
                Some(o) if o < -1e-3 => -1,
                Some(o) if o > 1e-3 => 1,
                _ => 0,
            };
            let n = count.entry((kind, side)).or_insert(0);
            *n += 1;
            let mut label = format!("{} {}", name, kind.label());
            if side != 0 {
                label.push(' ');
                label.push_str(if side < 0 { tr("links", "left") } else { tr("rechts", "right") });
            }
            if *n > 1 {
                label.push_str(&format!(" {n}"));
            }
            c.lines.push(FeatureLine { name: label, kind, offset, pts: pts.into_iter().map(to_project).collect() });
        }
        model.corridors.push(c);
    }
    // A corridor brings its design surface, even when only its lines were selected.
    let wanted: Vec<bool> = (0..model.surfaces.len())
        .map(|i| scope.wants(&surface_handles[i]) || model.corridors.iter().any(|c| c.surface == Some(i)))
        .collect();
    let mut keep = Vec::new();
    let mut new_index = vec![None; model.surfaces.len()];
    for (i, s) in std::mem::take(&mut model.surfaces).into_iter().enumerate() {
        if wanted[i] {
            new_index[i] = Some(keep.len());
            keep.push(s);
        }
    }
    model.surfaces = keep;
    for c in model.corridors.iter_mut() {
        c.surface = c.surface.and_then(|i| new_index[i]);
    }

    Ok(model)
}

/// A small model in RD for the writers' tests: two points, a TIN, an axis
/// by PIs with a curve and both profiles, a corridor line.
#[cfg(test)]
pub(crate) fn sample_model() -> Model {
    let o = [104_885.0, 425_320.0];
    let at = |x: f64, y: f64, z: f64| [o[0] + x, o[1] + y, z];
    let al = Alignment::from_pis(&[[o[0] - 80.0, o[1] - 20.0], [o[0], o[1] - 20.0], [o[0] + 60.0, o[1] + 40.0]], &[50.0]).unwrap();
    let len = al.length();
    Model {
        name: "Dordrecht & co".into(),
        crs: Some(Crs { code: "EPSG:28992".into(), epsg: Some(28992), name: "Amersfoort / RD New".into(), vertical: "NAP".into(), base: o }),
        points: vec![
            SurveyPoint { name: "101".into(), e: o[0] + 1.25, n: o[1] - 3.5, z: Some(1.2345), code: "PUT".into() },
            SurveyPoint { name: "101".into(), e: o[0] + 2.0, n: o[1] + 4.0, z: None, code: String::new() },
        ],
        surfaces: vec![Surface {
            name: "Terrein 1".into(),
            status: 'B',
            source: "AHN4 <5 m>".into(),
            points: vec![at(-100.0, -50.0, 0.5), at(100.0, -50.0, 1.0), at(100.0, 50.0, 1.5), at(-100.0, 50.0, 0.75)],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            corridor: None,
        }],
        axes: vec![Axis {
            name: "As 1".into(),
            start: 100.0,
            segs: al.segs.clone(),
            ground: Some(("Terrein 1".into(), vec![(0.0, 0.6), (len / 2.0, 0.9), (len, 1.2)])),
            design: Some(super::as_profiel::fit_curves(&[(0.0, 1.5), (60.0, 2.5), (len, 1.0)], 30.0)),
        }],
        corridors: vec![Corridor {
            name: "Corridor As 1".into(),
            axis: "As 1".into(),
            template: "weg".into(),
            lines: vec![FeatureLine { name: "Corridor As 1 Kant verharding links".into(), kind: LineKind::Pavement, offset: Some(-3.25), pts: vec![at(-80.0, -16.75, 1.4), at(-40.0, -16.75, 1.6)] }],
            ..Default::default()
        }],
    }
}
#[cfg(test)]
mod tests {

    use super::*;
    use ocs_plugin_api::host::codec::entities::Polyline3D;
    use ocs_plugin_api::host::codec::{LwPolyline, Vector2, Vector3};

    fn ctx() -> Ctx {
        let mut site = crate::gis::site::Site::default();
        site.set_origin(104_885.0, 425_320.0);
        site.absolute = false;
        Ctx { f: 1000.0, k: 1.0, site: Some(site), opt: super::super::CivielOptions::default() }
    }

    fn put(doc: &mut CadDocument, mut e: EntityType, layer: &str, data: &[(&str, &str)]) -> Handle {
        let c = e.common_mut();
        c.layer = layer.into();
        if !data.is_empty() {
            c.extended_data.add_record(super::super::record(&data.iter().map(|(k, v)| (*k, v.to_string())).collect::<Vec<_>>()));
        }
        if !doc.layers.iter().any(|l| l.name == layer) {
            let _ = doc.layers.add(ocs_plugin_api::host::codec::tables::Layer::new(layer));
        }
        doc.add_entity(e).unwrap()
    }

    /// A drawing in millimetres, relative to the base point: a TIN, an axis
    /// with a design profile and a corridor line, read back in metres.
    #[test]
    fn reads_a_drawing_into_project_metres() {
        let ctx = ctx();
        let mut doc = CadDocument::new();
        // Ground TIN: z = 1 m over 100 × 100 m.
        let mut m = super::super::model3d::MeshBuilder::default();
        let p = |x: f64, y: f64| [x * 1000.0, y * 1000.0, 1000.0];
        m.tri(p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0));
        m.tri(p(0.0, 0.0), p(100.0, 100.0), p(0.0, 100.0));
        let mesh = m.finish("B-GR-OG-TRIANGULATIE-G", None, None).remove(0);
        let tin = put(&mut doc, mesh, "B-GR-OG-TRIANGULATIE-G", &[("terrein", "Terrein 1"), ("bron", "test")]);
        // The axis: 80 m east along y = 50.
        let mut lw = LwPolyline::new();
        lw.add_point_with_bulge(Vector2::new(10_000.0, 50_000.0), 0.0);
        lw.add_point_with_bulge(Vector2::new(90_000.0, 50_000.0), 0.0);
        let axis = put(&mut doc, EntityType::LwPolyline(lw), "B-GR-AM-AS-G", &[("as", "As 1"), ("beginstation", "100")]);
        put(&mut doc, EntityType::LwPolyline(LwPolyline::new()), "B-GR-AM-AS-G", &[("ontwerpprofiel", "Ontwerpprofiel As 1"), ("as", "As 1"), ("pvi", "0:1.5;40:2.5;80:1.5"), ("boog_m", "20")]);
        // A corridor lane edge 3 m left of the axis.
        let line = Polyline3D::from_points(vec![Vector3::new(10_000.0, 53_000.0, 1500.0), Vector3::new(90_000.0, 53_000.0, 1500.0)]);
        let edge = put(&mut doc, EntityType::Polyline3D(line), "N-GR-VH-GESLOTENVERHARDING_ASFALT-G", &[]);
        let line = Polyline3D::from_points(vec![Vector3::new(10_000.0, 50_000.0, 1600.0), Vector3::new(90_000.0, 50_000.0, 1600.0)]);
        let axis3 = put(&mut doc, EntityType::Polyline3D(line), "N-GR-AM-AS-G", &[("corridor", "Corridor As 1"), ("as", "As 1"), ("sjabloon", "weg")]);
        let index = vec![("tin".to_string(), "B".to_string(), vec![tin]), ("corridor".to_string(), "Corridor As 1".to_string(), vec![edge, axis3])];
        let tin_b = Tin::from_document(&doc).unwrap();
        let mut ground = Ground::new(Box::new(move || Some(Maaiveld::Tin(Tin::from_faces(&tin_b.triangles.iter().map(|t| t.map(|i| tin_b.points[i])).collect::<Vec<_>>()).unwrap()))));
        let m = gather_from(&ctx, &doc, &Scope::everything(), &index, &mut ground, &|_| {}).unwrap();
        assert_eq!(m.surfaces.len(), 1);
        let s = &m.surfaces[0];
        assert_eq!((s.name.as_str(), s.status, s.triangles.len()), ("Terrein 1", 'B', 2));
        assert!(s.points.iter().any(|p| (p[0] - 104_985.0).abs() < 1e-9 && (p[1] - 425_420.0).abs() < 1e-9 && (p[2] - 1.0).abs() < 1e-9));
        assert!((s.area() - 10_000.0).abs() < 1e-6);
        assert_eq!(m.axes.len(), 1);
        let a = &m.axes[0];
        assert_eq!((a.name.as_str(), a.start), ("As 1", 100.0));
        assert!((a.length() - 80.0).abs() < 1e-9);
        assert_eq!(a.segs[0], Seg::Line { a: [104_895.0, 425_370.0], b: [104_975.0, 425_370.0] });
        let (gname, gpts) = a.ground.as_ref().unwrap();
        assert_eq!(gname, "Terrein 1");
        assert!(gpts.len() >= 80 && gpts.iter().all(|(_, z)| (z - 1.0).abs() < 1e-9));
        assert_eq!(a.design.as_ref().unwrap().len(), 3);
        assert_eq!(m.corridors.len(), 1);
        let c = &m.corridors[0];
        assert_eq!((c.axis.as_str(), c.template.as_str(), c.lines.len()), ("As 1", "weg", 2));
        assert_eq!(c.lines[1].kind, LineKind::Axis);
        assert_eq!(c.lines[0].kind, LineKind::Pavement);
        assert!((c.lines[0].offset.unwrap() + 3.0).abs() < 1e-9, "left is negative");
        assert_eq!(c.lines[0].name, format!("Corridor As 1 {} {}", tr("Kant verharding", "Pavement edge"), tr("links", "left")));
        // A selection of the axis only: no TIN, no corridor.
        let only = Scope { all: false, selection: [axis].into_iter().collect() };
        let m = gather_from(&ctx, &doc, &only, &index, &mut Ground::new(Box::new(|| None)), &|_| {}).unwrap();
        assert_eq!((m.axes.len(), m.surfaces.len(), m.corridors.len()), (1, 0, 0));
    }

    #[test]
    fn names_stay_unique() {
        let mut t = HashSet::new();
        assert_eq!(unique("As 1", &mut t), "As 1");
        assert_eq!(unique("As 1", &mut t), "As 1 (2)");
        assert_eq!(unique("As 1", &mut t), "As 1 (3)");
        assert_eq!(LineKind::from_layer("N-GR-GW-TALUD_TEEN-G"), LineKind::Toe);
        assert_eq!(LineKind::from_layer("N-GR-AM-AS-G"), LineKind::Axis);
    }
}
