//! Terrain (Road "Terrain"): a TIN from survey points, 3D lines or AHN,
//! drawn as 3D faces on OG-TRIANGULATIE; contours and the boundary from it.
//! Coordinates and heights of a `Tin` are drawing units.

use std::collections::{HashMap, HashSet};

use ocs_plugin_api::host::codec::entities::{Face3D, Polyline3D};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use super::{record, v3, Ctx, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::Element;

pub type P3 = [f64; 3];

pub struct Tin {
    pub points: Vec<P3>,
    /// Counter-clockwise vertex indices.
    pub triangles: Vec<[usize; 3]>,
    index: Index,
}

/// Triangles per cell of a uniform grid, for height lookups.
struct Index {
    origin: XY,
    cell: f64,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

impl Index {
    fn build(points: &[P3], triangles: &[[usize; 3]]) -> Index {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in points {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let n = (triangles.len() as f64).sqrt().ceil().max(1.0);
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-9);
        let cell = size / n;
        let cols = ((hi[0] - lo[0]) / cell).floor() as usize + 1;
        let rows = ((hi[1] - lo[1]) / cell).floor() as usize + 1;
        let mut cells = vec![Vec::new(); cols * rows];
        for (t, tri) in triangles.iter().enumerate() {
            let xs = tri.map(|i| points[i][0]);
            let ys = tri.map(|i| points[i][1]);
            let c0 = ((xs.iter().cloned().fold(f64::MAX, f64::min) - lo[0]) / cell).floor() as usize;
            let c1 = ((xs.iter().cloned().fold(f64::MIN, f64::max) - lo[0]) / cell).floor() as usize;
            let r0 = ((ys.iter().cloned().fold(f64::MAX, f64::min) - lo[1]) / cell).floor() as usize;
            let r1 = ((ys.iter().cloned().fold(f64::MIN, f64::max) - lo[1]) / cell).floor() as usize;
            for r in r0..=r1.min(rows - 1) {
                for c in c0..=c1.min(cols - 1) {
                    cells[r * cols + c].push(t as u32);
                }
            }
        }
        Index { origin: lo, cell, cols, rows, cells }
    }

    fn candidates(&self, p: XY) -> &[u32] {
        let c = ((p[0] - self.origin[0]) / self.cell).floor();
        let r = ((p[1] - self.origin[1]) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return &[];
        }
        &self.cells[r as usize * self.cols + c as usize]
    }
}

fn area2(a: P3, b: P3, c: P3) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])
}

impl Tin {
    fn assemble(points: Vec<P3>, mut triangles: Vec<[usize; 3]>) -> Tin {
        for t in triangles.iter_mut() {
            if area2(points[t[0]], points[t[1]], points[t[2]]) < 0.0 {
                t.swap(1, 2);
            }
        }
        let index = Index::build(&points, &triangles);
        Tin { points, triangles, index }
    }

    /// Delaunay triangulation of the points; triangles with an edge longer
    /// than `max_edge` (drawing units, 0 = no limit) are left out, like
    /// Road's MaxLength, so a concave site keeps its shape.
    pub fn build(points: &[P3], max_edge: f64) -> Result<Tin, String> {
        // Equal positions: keep the first.
        let mut seen = HashSet::new();
        let pts: Vec<P3> = points.iter().copied().filter(|p| seen.insert(((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64))).collect();
        if pts.len() < 3 {
            return Err(format!("een terrein heeft minstens drie punten met een hoogte nodig ({} gevonden)", pts.len()));
        }
        // Around the centre: Delaunay on large RD numbers loses precision.
        let c = [pts.iter().map(|p| p[0]).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p[1]).sum::<f64>() / pts.len() as f64];
        let d: Vec<delaunator::Point> = pts.iter().map(|p| delaunator::Point { x: p[0] - c[0], y: p[1] - c[1] }).collect();
        let tri = delaunator::triangulate(&d);
        let mut triangles = Vec::with_capacity(tri.triangles.len() / 3);
        for t in tri.triangles.chunks_exact(3) {
            let (a, b, cc) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            if area2(a, b, cc).abs() < 1e-12 {
                continue;
            }
            if max_edge > 0.0 {
                let long = [(a, b), (b, cc), (cc, a)].iter().any(|(p, q)| (q[0] - p[0]).hypot(q[1] - p[1]) > max_edge);
                if long {
                    continue;
                }
            }
            triangles.push([t[0], t[1], t[2]]);
        }
        if triangles.is_empty() {
            return Err("geen driehoeken: liggen de punten op één lijn, of is de maximale ribbe te kort?".into());
        }
        Ok(Tin::assemble(pts, triangles))
    }

    /// A TIN from 3D faces: shared corners become shared vertices again.
    pub fn from_faces(faces: &[[P3; 3]]) -> Option<Tin> {
        let mut points = Vec::new();
        let mut ids: HashMap<(i64, i64, i64), usize> = HashMap::new();
        let mut triangles = Vec::with_capacity(faces.len());
        for f in faces {
            let t = f.map(|p| {
                let key = ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64, (p[2] * 1000.0).round() as i64);
                *ids.entry(key).or_insert_with(|| {
                    points.push(p);
                    points.len() - 1
                })
            });
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                triangles.push(t);
            }
        }
        (!triangles.is_empty()).then(|| Tin::assemble(points, triangles))
    }

    /// The faces on the drawing's triangulation layers (OG-TRIANGULATIE),
    /// or the selected faces when `only` is not empty.
    pub fn from_document_with(doc: &CadDocument, only: &HashSet<Handle>) -> Option<Tin> {
        let faces: Vec<[P3; 3]> = doc
            .model_space_entities()
            .filter(|e| matches!(e, EntityType::Face3D(_) | EntityType::Mesh(_)))
            .filter(|e| if only.is_empty() { e.common().layer.contains("-OG-TRIANGULATIE-") } else { only.contains(&e.common().handle) })
            .flat_map(triangles_of)
            .collect();
        Tin::from_faces(&faces)
    }

    /// The existing ground: the faces on OG-TRIANGULATIE layers of status B
    /// (bestaand); a design surface sits on status N (nieuw).
    pub fn from_document(doc: &CadDocument) -> Option<Tin> {
        Tin::of_status(doc, 'B')
    }

    /// The faces on the OG-TRIANGULATIE layers of one drawing status.
    pub fn of_status(doc: &CadDocument, status: char) -> Option<Tin> {
        let prefix = format!("{status}-");
        let faces: Vec<[P3; 3]> = doc
            .model_space_entities()
            .filter(|e| e.common().layer.starts_with(&prefix) && e.common().layer.contains("-OG-TRIANGULATIE-"))
            .flat_map(triangles_of)
            .collect();
        Tin::from_faces(&faces)
    }

    /// Height (drawing units) at a point, linear in its triangle.
    pub fn height(&self, p: XY) -> Option<f64> {
        for &t in self.index.candidates(p) {
            let [a, b, c] = self.triangles[t as usize].map(|i| self.points[i]);
            let d = area2(a, b, c);
            let q = [p[0], p[1], 0.0];
            let (u, v, w) = (area2(q, b, c) / d, area2(a, q, c) / d, area2(a, b, q) / d);
            let eps = -1e-9;
            if u >= eps && v >= eps && w >= eps {
                return Some(u * a[2] + v * b[2] + w * c[2]);
            }
        }
        None
    }

    pub fn range(&self) -> (f64, f64) {
        self.points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])))
    }

    /// Contours at `level` (drawing units): every triangle that the level
    /// crosses gives a segment between two of its edges; segments sharing an
    /// edge are chained. Closed rings come back closed.
    pub fn contour(&self, level: f64) -> Vec<(Vec<XY>, bool)> {
        type Edge = (usize, usize);
        let key = |a: usize, b: usize| -> Edge { if a < b { (a, b) } else { (b, a) } };
        let above = |i: usize| self.points[i][2] >= level;
        let mut segments: Vec<[Edge; 2]> = Vec::new();
        for t in &self.triangles {
            let cut: Vec<Edge> = [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])].into_iter().filter(|(a, b)| above(*a) != above(*b)).map(|(a, b)| key(a, b)).collect();
            if cut.len() == 2 {
                segments.push([cut[0], cut[1]]);
            }
        }
        let at = |e: Edge| -> XY {
            let (a, b) = (self.points[e.0], self.points[e.1]);
            let t = (level - a[2]) / (b[2] - a[2]);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        };
        let mut by_edge: HashMap<Edge, Vec<usize>> = HashMap::new();
        for (i, s) in segments.iter().enumerate() {
            for e in s {
                by_edge.entry(*e).or_default().push(i);
            }
        }
        let mut used = vec![false; segments.len()];
        let mut out = Vec::new();
        // Walk from `start` over edge `from` through unused segments.
        let walk = |start: usize, from: Edge, used: &mut Vec<bool>| -> (Vec<Edge>, bool) {
            let mut chain = vec![from];
            let (mut seg, mut edge) = (start, from);
            loop {
                used[seg] = true;
                let s = segments[seg];
                let next = if s[0] == edge { s[1] } else { s[0] };
                chain.push(next);
                match by_edge[&next].iter().copied().find(|&j| !used[j]) {
                    Some(j) => {
                        seg = j;
                        edge = next;
                    }
                    None => return (chain.clone(), chain.len() > 3 && chain[0] == next),
                }
            }
        };
        // Open lines start at an edge on the TIN border (one segment).
        for i in 0..segments.len() {
            if used[i] {
                continue;
            }
            if let Some(&start) = segments[i].iter().find(|e| by_edge[*e].len() == 1) {
                let (chain, _) = walk(i, start, &mut used);
                out.push((chain.into_iter().map(at).collect(), false));
            }
        }
        // What is left are rings.
        for i in 0..segments.len() {
            if !used[i] {
                let (mut chain, closed) = walk(i, segments[i][0], &mut used);
                if closed {
                    chain.pop();
                }
                out.push((chain.into_iter().map(at).collect(), closed));
            }
        }
        out
    }

    /// The outline: edges of one triangle only, chained into rings (the
    /// outer border and holes).
    pub fn boundary(&self) -> Vec<Vec<P3>> {
        let mut count: HashMap<(usize, usize), usize> = HashMap::new();
        for t in &self.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *count.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
            }
        }
        let mut next: HashMap<usize, usize> = HashMap::new();
        for t in &self.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                if count[&(a.min(b), a.max(b))] == 1 {
                    next.insert(a, b);
                }
            }
        }
        let mut rings = Vec::new();
        let mut done = HashSet::new();
        let mut starts: Vec<usize> = next.keys().copied().collect();
        starts.sort();
        for s in starts {
            if done.contains(&s) {
                continue;
            }
            let mut ring = Vec::new();
            let mut v = s;
            while done.insert(v) {
                ring.push(self.points[v]);
                match next.get(&v) {
                    Some(n) => v = *n,
                    None => break,
                }
            }
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
        rings
    }
}

/// Triangles of a mesh (MESH entity): every face fanned from its first corner.
fn mesh_triangles(m: &ocs_plugin_api::host::codec::entities::Mesh) -> Vec<[P3; 3]> {
    let p = |i: usize| m.vertices.get(i).map(|v| [v.x, v.y, v.z]);
    let mut out = Vec::new();
    for f in &m.faces {
        for k in 1..f.vertices.len().saturating_sub(1) {
            if let (Some(a), Some(b), Some(c)) = (p(f.vertices[0]), p(f.vertices[k]), p(f.vertices[k + 1])) {
                out.push([a, b, c]);
            }
        }
    }
    out
}

/// The triangles of a terrain entity: 3D faces and meshes.
fn triangles_of(e: &EntityType) -> Vec<[P3; 3]> {
    match e {
        EntityType::Face3D(f) => face_triangles(f),
        EntityType::Mesh(m) => mesh_triangles(m),
        _ => Vec::new(),
    }
}

fn face_triangles(f: &Face3D) -> Vec<[P3; 3]> {
    let p = |v: &ocs_plugin_api::host::codec::Vector3| [v.x, v.y, v.z];
    let (a, b, c, d) = (p(&f.first_corner), p(&f.second_corner), p(&f.third_corner), p(&f.fourth_corner));
    let mut out = vec![[a, b, c]];
    if d != c {
        out.push([a, c, d]);
    }
    out
}

/// Points with a height from the selected entities: points, 3D polylines,
/// lines and polylines at an elevation (contour lines), 3D faces.
pub fn points_from(doc: &CadDocument, selection: &HashSet<Handle>) -> Vec<P3> {
    let mut out = Vec::new();
    for e in doc.model_space_entities() {
        if !selection.contains(&e.common().handle) {
            continue;
        }
        match e {
            EntityType::Point(p) => out.push([p.location.x, p.location.y, p.location.z]),
            EntityType::Polyline3D(p) => out.extend(p.vertices.iter().map(|v| [v.position.x, v.position.y, v.position.z])),
            EntityType::LwPolyline(l) if l.elevation != 0.0 => out.extend(l.vertices.iter().map(|v| [v.location.x, v.location.y, l.elevation])),
            EntityType::Line(l) if l.start.z != 0.0 || l.end.z != 0.0 => {
                out.push([l.start.x, l.start.y, l.start.z]);
                out.push([l.end.x, l.end.y, l.end.z]);
            }
            EntityType::Face3D(f) => {
                for v in [&f.first_corner, &f.second_corner, &f.third_corner, &f.fourth_corner] {
                    out.push([v.x, v.y, v.z]);
                }
            }
            _ => {}
        }
    }
    out
}

/// More than this many triangles makes the drawing slow: ask for a coarser grid.
const MAX_TRIANGLES: usize = 120_000;

static TERRAINS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn next_name() -> String {
    format!("Terrein {}", TERRAINS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1)
}

/// A TIN as 3D faces on OG-TRIANGULATIE and its outline on OG-GRENS; the
/// object data sits on the first face and the outline. Returns the face layer.
pub fn draw(ctx: &Ctx, tin: &Tin, name: &str, source: &str, max_edge_m: f64, out: &mut Out) -> Result<(crate::nlcs::LayerSpec, usize), String> {
    let mut pairs = vec![
        ("terrein", name.to_string()),
        ("bron", source.to_string()),
        ("punten", tin.points.len().to_string()),
        ("driehoeken", tin.triangles.len().to_string()),
    ];
    if max_edge_m > 0.0 {
        pairs.push(("max_ribbe_m", format!("{max_edge_m:.1}")));
    }
    let d = record(&pairs);
    let faces = ctx.layer("OG", "TRIANGULATIE", Element::G)?;
    let border = ctx.layer("OG", "GRENS", Element::G)?;
    // One mesh (a few for big terrains), not a face per triangle.
    let mut m = super::model3d::MeshBuilder::default();
    for t in &tin.triangles {
        let [a, b, c] = t.map(|i| tin.points[i]);
        m.tri(a, b, c);
    }
    let ents = m.finish(&faces.name, None, Some(&d));
    let meshes = ents.len();
    out.put(&faces, ents);
    for ring in tin.boundary() {
        let mut p = Polyline3D::from_points(ring.iter().map(|p| v3([p[0], p[1]], p[2])).collect());
        p.flags.closed = true;
        let mut e = EntityType::Polyline3D(p);
        let cm = e.common_mut();
        cm.layer = border.name.clone();
        cm.color = ocs_plugin_api::host::codec::Color::ByLayer;
        cm.extended_data.add_record(d.clone());
        out.put(&border, [e]);
    }
    Ok((faces, meshes))
}

/// What a TIN is made of.
#[derive(Clone, Debug, PartialEq)]
pub enum TinSource {
    /// The selection, or AHN when nothing is selected.
    Auto,
    Ahn,
    Selection,
    /// The survey points (OG-HOOGTEPUNT) and breaklines (OG-BREUKLIJN) of the drawing.
    Points,
    /// Everything with a height on one layer.
    Layer(String),
}

/// `CIV_TIN`: a terrain on OG-TRIANGULATIE (status from the settings: B for
/// the existing ground, N for a design).
pub fn create(opt: &super::CivielOptions, source: TinSource, progress: &dyn Fn(String)) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = if source == TinSource::Ahn { ocs_plugin_api::host::codec::CadDocument::new() } else { super::document(&["punten"], &[])? };
    let mut sel: HashSet<Handle> = match &source {
        TinSource::Auto | TinSource::Selection => crate::bridge::selection().unwrap_or_default().into_iter().collect(),
        TinSource::Ahn => HashSet::new(),
        TinSource::Points => doc
            .model_space_entities()
            .filter(|e| {
                let l = &e.common().layer;
                (matches!(e, EntityType::Point(_)) && l.contains("-OG-HOOGTEPUNT-")) || l.contains("-OG-BREUKLIJN-")
            })
            .map(|e| e.common().handle)
            .collect(),
        TinSource::Layer(name) => doc.model_space_entities().filter(|e| e.common().layer.eq_ignore_ascii_case(name)).map(|e| e.common().handle).collect(),
    };
    if sel.is_empty() && matches!(source, TinSource::Selection | TinSource::Points | TinSource::Layer(_)) {
        return Err(match source {
            TinSource::Points => "geen meetpunten (OG-HOOGTEPUNT) of breuklijnen in de tekening".into(),
            TinSource::Layer(n) => format!("niets op laag {n}"),
            _ => "niets geselecteerd".into(),
        });
    }
    if source == TinSource::Ahn {
        sel.clear();
    }
    let (points, source) = if sel.is_empty() {
        let bron = super::hoogtebron(&ctx)?;
        progress(format!("{} ophalen…", bron.label()));
        let pts = bron.grid_points(&ctx, opt.ahn_spacing.max(0.5), progress)?;
        (pts, format!("{} om de {} m", bron.label(), opt.ahn_spacing))
    } else {
        (points_from(&doc, &sel), format!("{} geselecteerde elementen", sel.len()))
    };
    progress(format!("Driehoeken maken van {} punten…", points.len()));
    let max_edge = if sel.is_empty() { opt.ahn_spacing * 3.0 } else { opt.max_edge } * ctx.f;
    let tin = Tin::build(&points, max_edge)?;
    if tin.triangles.len() > MAX_TRIANGLES {
        return Err(format!("{} driehoeken is te veel voor de tekening; kies een grotere puntafstand of een kleiner gebied", tin.triangles.len()));
    }
    let name = next_name();
    let mut out = Out::default();
    let (faces, meshes) = draw(&ctx, &tin, &name, &source, max_edge / ctx.f, &mut out)?;
    let (zmin, zmax) = tin.range();
    super::api::note("terrein", name.clone());
    super::api::note("driehoeken", tin.triangles.len());
    super::api::note("punten", tin.points.len());
    super::api::note("laag", faces.name.clone());
    super::api::note("hoogte_min", zmin / ctx.f);
    super::api::note("hoogte_max", zmax / ctx.f);
    progress("Terrein tekenen…".into());
    let handles = out.write_handles(&name)?;
    let written = handles.len();
    let status = faces.name.chars().next().unwrap_or('B').to_string();
    crate::bridge::index_add("tin", &status, &handles[..meshes.min(handles.len())], false);
    if sel.is_empty() {
        crate::bridge::note_source("AHN4 (PDOK, CC0)");
    }
    Ok(format!(
        "{name}: {} punten, {} driehoeken, {} tot {} m ({source}) — {written} elementen op {}.",
        tin.points.len(),
        tin.triangles.len(),
        nap(zmin / ctx.f, 2),
        nap(zmax / ctx.f, 2),
        faces.name
    ))
}

/// `GIS_TIN_HOOGTELIJNEN`: contours of the selected faces, or of the TIN on
/// the triangulation layers.
/// Which surface a command works on: the selected faces (else the existing
/// ground), or the faces of one drawing status (B bestaand, N nieuw).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Surface {
    Auto,
    Status(char),
}

fn surface_tin(doc: &CadDocument, which: Surface) -> Option<Tin> {
    match which {
        Surface::Status(c) => Tin::of_status(doc, c),
        Surface::Auto => {
            let sel: HashSet<Handle> = crate::bridge::selection().unwrap_or_default().into_iter().collect();
            let faces: HashSet<Handle> = doc
                .model_space_entities()
                .filter(|e| matches!(e, EntityType::Face3D(_) | EntityType::Mesh(_)) && sel.contains(&e.common().handle))
                .map(|e| e.common().handle)
                .collect();
            if faces.is_empty() { Tin::from_document(doc).or_else(|| Tin::from_document_with(doc, &HashSet::new())) } else { Tin::from_document_with(doc, &faces) }
        }
    }
}

pub fn contours(opt: &super::CivielOptions, which: Surface, progress: &dyn Fn(String)) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin"], &[])?;
    let tin = surface_tin(&doc, which).ok_or("geen terrein: maak eerst een TIN (Oppervlak → TIN maken) of selecteer 3D-vlakken")?;
    let (lo, hi) = tin.range();
    let (lo, hi) = (lo / ctx.f, hi / ctx.f);
    let interval = opt.interval.max(0.01);
    let levels = crate::gis::ahn::levels((lo, hi), interval);
    if levels.len() > 2000 {
        return Err(format!("{} hoogtelijnen is te veel: kies een groter interval", levels.len()));
    }
    progress(format!("{} hoogtelijnen berekenen…", levels.len()));
    let minor = ctx.layer("OG", "HOOGTELIJN_MINOR", Element::G)?;
    let major = ctx.layer("OG", "HOOGTELIJN_MAJOR", Element::G)?;
    let major_t = ctx.layer("OG", "HOOGTELIJN_MAJOR", Element::T)?;
    let major_step = interval * opt.major_every.max(1) as f64;
    let is_major = |l: f64| ((l / major_step).round() * major_step - l).abs() < interval * 0.01;
    let mut out = Out::default();
    let mut count = 0;
    let k = ctx.k;
    for level in levels {
        let big = is_major(level);
        let spec = if big { &major } else { &minor };
        let d = record(&[("bron", "TIN".into()), ("hoogte", format!("{level:.3}")), ("interval", format!("{interval}"))]);
        for (pts, closed) in tin.contour(level * ctx.f) {
            if pts.len() < 2 {
                continue;
            }
            let mut ents = crate::emit::polylines(&[(pts.clone(), closed)], &spec.name, k, Some(&d));
            for e in ents.iter_mut() {
                if let EntityType::LwPolyline(l) = e {
                    l.elevation = level * ctx.f;
                }
            }
            out.put(spec, ents);
            count += 1;
            let len: f64 = pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            if big && ctx.opt.nlcs.labels && len > 40.0 * k {
                if let Some((at, angle)) = crate::emit::label_spot(&pts) {
                    let mut t = ctx.text(at, &nap(level, 2), 1.8, crate::emit::readable(angle), &major_t, Some(&d));
                    if let EntityType::Text(tx) = &mut t {
                        tx.insertion_point.z = level * ctx.f;
                        if let Some(a) = tx.alignment_point.as_mut() {
                            a.z = level * ctx.f;
                        }
                    }
                    out.put(&major_t, [t]);
                }
            }
        }
    }
    if count == 0 {
        return Err(format!("geen hoogtelijnen met interval {interval} m (hoogteverschil {:.2} m)", hi - lo));
    }
    let written = out.write("Hoogtelijnen terrein")?;
    Ok(format!("{count} hoogtelijnen van het terrein ({} driehoeken), {} tot {} m om de {} m — {written} elementen.", tin.triangles.len(), nap(lo, 2), nap(hi, 2), interval))
}

/// The selected 3D faces (the design surface, when any) and the TIN on the
/// triangulation layers without them (the existing ground).
fn surfaces(doc: &CadDocument) -> Result<(Option<Tin>, Option<Tin>), String> {
    let sel: HashSet<Handle> = crate::bridge::selection().unwrap_or_default().into_iter().collect();
    let picked: HashSet<Handle> = doc
        .model_space_entities()
        .filter(|e| matches!(e, EntityType::Face3D(_) | EntityType::Mesh(_)) && sel.contains(&e.common().handle))
        .map(|e| e.common().handle)
        .collect();
    if picked.is_empty() {
        return Ok((None, Tin::from_document(doc)));
    }
    let design = Tin::from_document_with(doc, &picked);
    let rest: Vec<[P3; 3]> = doc
        .model_space_entities()
        .filter(|e| e.common().layer.starts_with("B-") && e.common().layer.contains("-OG-TRIANGULATIE-") && !picked.contains(&e.common().handle))
        .flat_map(triangles_of)
        .collect();
    Ok((design, Tin::from_faces(&rest)))
}

/// Colours from low (blue) to high (red), as ACI colour indices.
const RAMP: [u8; 9] = [170, 150, 130, 110, 90, 70, 50, 30, 10];

/// A plain tool layer (no NLCS object) in a colour index.
fn tool_layer(name: &str, aci: u8) -> crate::nlcs::LayerSpec {
    crate::nlcs::LayerSpec {
        name: name.to_string(),
        color: aci,
        rgb: crate::nlcs::color_rgb(aci),
        true_color: None,
        lineweight: None,
        linetype: "CONTINUOUS".into(),
        object_id: 0,
    }
}

fn solid(pts: [XY; 4], layer: &str) -> EntityType {
    use ocs_plugin_api::host::codec::entities::Solid;
    // SOLID corners run 1-2-4-3: the third is opposite the second.
    let mut e = EntityType::Solid(Solid::new(v3(pts[0], 0.0), v3(pts[1], 0.0), v3(pts[3], 0.0), v3(pts[2], 0.0)));
    let c = e.common_mut();
    c.layer = layer.to_string();
    c.color = ocs_plugin_api::host::codec::Color::ByLayer;
    e
}

/// `CIV_ANALYSE`: the terrain coloured in height bands (elevation
/// analysis), with a legend beside it. Tool layers OI-ANALYSE-HOOGTE-nn.
pub fn analysis(opt: &super::CivielOptions, which: Surface) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin"], &[])?;
    let tin = surface_tin(&doc, which).ok_or("geen terrein: maak eerst een TIN of selecteer 3D-vlakken")?;
    let f = ctx.f;
    let (lo, hi) = tin.range();
    let (lo, hi) = (lo / f, hi / f);
    let wanted = opt.bands.clamp(2, RAMP.len());
    let step = super::nice_step(((hi - lo) / wanted as f64).max(0.01));
    let base = (lo / step).floor() * step;
    let n = (((hi - base) / step).ceil() as usize).clamp(1, 40);
    let colour = |i: usize| RAMP[if n == 1 { 0 } else { i * (RAMP.len() - 1) / (n - 1) }];
    let layers: Vec<crate::nlcs::LayerSpec> = (0..n).map(|i| tool_layer(&format!("OI-ANALYSE-HOOGTE-{:02}", i + 1), colour(i))).collect();
    let mut out = Out::default();
    let mut counts = vec![0usize; n];
    for t in &tin.triangles {
        let [a, b, c] = t.map(|i| tin.points[i]);
        let z = (a[2] + b[2] + c[2]) / 3.0 / f;
        let i = (((z - base) / step).floor() as usize).min(n - 1);
        counts[i] += 1;
        out.put(&layers[i], [solid([[a[0], a[1]], [b[0], b[1]], [c[0], c[1]], [c[0], c[1]]], &layers[i].name)]);
    }
    // Legend to the right of the terrain.
    let (mut x1, mut y1) = (f64::MIN, f64::MIN);
    for p in &tin.points {
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let k = ctx.k;
    let (lx, box_h) = (x1 + 10.0 * k, 5.0 * k);
    for i in 0..n {
        let y = y1 - (n - i) as f64 * (box_h + 1.5 * k);
        let l = &layers[i];
        out.put(l, [solid([[lx, y], [lx + 8.0 * k, y], [lx + 8.0 * k, y + box_h], [lx, y + box_h]], &l.name)]);
        let label = format!("{} tot {} m · {} %", nap(base + i as f64 * step, 2), nap(base + (i + 1) as f64 * step, 2), counts[i] * 100 / tin.triangles.len().max(1));
        let mut t = crate::emit::text([lx + 10.0 * k, y + box_h / 2.0], &label, 1.8 * k, 0.0, &l.name, None);
        if let EntityType::Text(tx) = &mut t {
            tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
        }
        out.put(l, [t]);
    }
    let title = crate::emit::text([lx, y1 + 4.0 * k], "Hoogteanalyse", 2.5 * k, 0.0, &layers[n - 1].name, None);
    let title = if let EntityType::Text(mut tx) = title {
        tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
        EntityType::Text(tx)
    } else {
        title
    };
    out.put(&layers[n - 1], [title]);
    let written = out.write("Hoogteanalyse")?;
    Ok(format!("Hoogteanalyse: {n} banden van {} m, {} tot {} m — {written} elementen (lagen OI-ANALYSE-HOOGTE-..).", format!("{step}").replace('.', ","), nap(base, 2), nap(base + n as f64 * step, 2)))
}

/// Cut and fill on a grid: volumes (m³) and areas (m²).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Earthwork {
    pub cut: f64,
    pub fill: f64,
    pub cut_area: f64,
    pub fill_area: f64,
}

/// Earthwork between a design and the existing ground on a grid of `cell`
/// (drawing units); heights in drawing units, results in metres.
pub fn earthwork(bbox: [f64; 4], cell: f64, f: f64, mut ground: impl FnMut(XY) -> Option<f64>, mut design: impl FnMut(XY) -> Option<f64>) -> Earthwork {
    let mut w = Earthwork::default();
    let area = (cell / f) * (cell / f);
    let mut y = bbox[1] + cell / 2.0;
    while y < bbox[3] {
        let mut x = bbox[0] + cell / 2.0;
        while x < bbox[2] {
            if let (Some(g), Some(d)) = (ground([x, y]), design([x, y])) {
                let dz = (d - g) / f;
                if dz > 0.0 {
                    w.fill += dz * area;
                    w.fill_area += area;
                } else if dz < 0.0 {
                    w.cut -= dz * area;
                    w.cut_area += area;
                }
            }
            x += cell;
        }
        y += cell;
    }
    w
}

fn tin_bbox(t: &Tin) -> [f64; 4] {
    t.points.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

pub fn m3(v: f64) -> String {
    let s = format!("{:.0}", v);
    let digits: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(*c);
    }
    out
}

/// `CIV_VOLUME`: cut and fill between the selected 3D faces (design) and the
/// TIN on the triangulation layers, or between that TIN and the level of the
/// settings when nothing is selected. Drawn as cut (GW-ONTGRAVINGSVAK-WERK)
/// and fill (GW-OPHOGING-WERK) areas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Design {
    /// The selected faces, else the design surface (status N), else the level.
    Auto,
    Level,
}

pub fn volume(opt: &super::CivielOptions, design: Design, progress: &dyn Fn(String)) -> Result<String, String> {
    let opt = opt.clone();
    let ctx = Ctx::new(&opt)?;
    let doc = super::document(&["tin"], &[])?;
    let (selected, _) = surfaces(&doc)?;
    let f = ctx.f;
    let ground = Tin::from_document(&doc).ok_or("geen bestaand terrein (B-…-OG-TRIANGULATIE): maak eerst een TIN")?;
    let level = opt.level * f;
    let (picked, what) = match design {
        Design::Level => (None, format!("terrein t.o.v. peil {} m", nap(opt.level, 2))),
        Design::Auto => match selected.map(|t| (t, "ontwerp (selectie) t.o.v. terrein")).or_else(|| Tin::of_status(&doc, 'N').map(|t| (t, "ontwerpoppervlak (N) t.o.v. bestaand terrein (B)"))) {
            Some((t, w)) => (Some(t), w.to_string()),
            None => (None, format!("terrein t.o.v. peil {} m", nap(opt.level, 2))),
        },
    };
    let bbox = match &picked {
        Some(d) => tin_bbox(d),
        None => tin_bbox(&ground),
    };
    let cell = opt.volume_cell.max(0.1) * f;
    let cells = ((bbox[2] - bbox[0]) / cell) * ((bbox[3] - bbox[1]) / cell);
    if cells > 4.0e6 {
        return Err(format!("{cells:.0} rastercellen is te veel: kies een grotere cel dan {} m", opt.volume_cell));
    }
    progress("Grondverzet berekenen…".into());
    let design_at = |p: XY| match &picked {
        Some(d) => d.height(p),
        None => Some(level),
    };
    let w = earthwork(bbox, cell, f, |p| ground.height(p), design_at);
    // The map: runs of cut or fill per row on a coarser grid.
    let extent = (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]);
    let dcell = cell.max(extent / 150.0);
    let cut_l = ctx.layer("GW", "ONTGRAVINGSVAK-WERK", Element::A)?;
    let fill_l = ctx.layer("GW", "OPHOGING-WERK", Element::A)?;
    let text_l = ctx.layer("GW", "ONTGRAVINGSVAK-WERK", Element::T)?;
    let mut out = Out::default();
    let mut y = bbox[1];
    while y < bbox[3] {
        let mut x = bbox[0];
        let mut run: Option<(i8, f64)> = None;
        let flush = |run: &mut Option<(i8, f64)>, x_end: f64, out: &mut Out| {
            if let Some((sign, x0)) = run.take() {
                let l = if sign < 0 { &cut_l } else { &fill_l };
                out.put(l, [solid([[x0, y], [x_end, y], [x_end, y + dcell], [x0, y + dcell]], &l.name)]);
            }
        };
        while x < bbox[2] {
            let c = [x + dcell / 2.0, y + dcell / 2.0];
            let sign = match (ground.height(c), design_at(c)) {
                (Some(g), Some(d)) if (d - g).abs() > 0.005 * f => if d > g { 1 } else { -1 },
                _ => 0,
            };
            if run.map(|r| r.0) != Some(sign) {
                flush(&mut run, x, &mut out);
                if sign != 0 {
                    run = Some((sign, x));
                }
            }
            x += dcell;
        }
        flush(&mut run, x, &mut out);
        y += dcell;
    }
    let k = ctx.k;
    let lines = [
        format!("Grondverzet — {what}"),
        format!("Afgraven {} m³ over {} m²", m3(w.cut), m3(w.cut_area)),
        format!("Ophogen {} m³ over {} m²", m3(w.fill), m3(w.fill_area)),
        format!("Netto {} {} m³ · raster {} m", if w.fill >= w.cut { "tekort" } else { "overschot" }, m3((w.fill - w.cut).abs()), opt.volume_cell),
    ];
    for (i, l) in lines.iter().enumerate() {
        let mut t = ctx.text([bbox[0], bbox[3] + (6.0 + (lines.len() - 1 - i) as f64 * 4.0) * k], l, if i == 0 { 2.5 } else { 1.8 }, 0.0, &text_l, None);
        if let EntityType::Text(tx) = &mut t {
            tx.horizontal_alignment = ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left;
        }
        out.put(&text_l, [t]);
    }
    let n = out.write("Grondverzet")?;
    super::api::note("afgraven_m3", w.cut);
    super::api::note("ophogen_m3", w.fill);
    super::api::note("afgraven_m2", w.cut_area);
    super::api::note("ophogen_m2", w.fill_area);
    super::api::note("vergelijking", what.clone());
    Ok(format!("{}: afgraven {} m³, ophogen {} m³ — {n} elementen.", lines[0], m3(w.cut), m3(w.fill)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cone-free test field: z = x / 10 on a 5 × 5 grid of 10 m.
    fn ramp() -> Tin {
        let mut pts = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                let (x, y) = (i as f64 * 10.0, j as f64 * 10.0);
                pts.push([x, y, x / 10.0]);
            }
        }
        Tin::build(&pts, 0.0).unwrap()
    }

    #[test]
    fn tin_heights_and_contours() {
        let t = ramp();
        assert_eq!(t.triangles.len(), 32);
        assert!((t.height([12.5, 31.0]).unwrap() - 1.25).abs() < 1e-9);
        assert_eq!(t.height([-1.0, 5.0]), None);
        // One straight contour across the field at x = 25.
        let c = t.contour(2.5);
        assert_eq!(c.len(), 1);
        assert!(!c[0].1);
        assert!(c[0].0.iter().all(|p| (p[0] - 25.0).abs() < 1e-9));
        let ys: Vec<f64> = c[0].0.iter().map(|p| p[1]).collect();
        assert!(ys.iter().cloned().fold(f64::MAX, f64::min) < 1e-9 && ys.iter().cloned().fold(f64::MIN, f64::max) > 40.0 - 1e-9);
        // The outline is one ring around the square.
        let b = t.boundary();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].len(), 16);
    }

    #[test]
    fn a_hill_gives_closed_rings_and_faces_round_trip() {
        let mut pts = Vec::new();
        for i in -4..=4 {
            for j in -4..=4 {
                let (x, y) = (i as f64 * 5.0, j as f64 * 5.0);
                pts.push([x, y, 10.0 - (x * x + y * y).sqrt() / 5.0]);
            }
        }
        let t = Tin::build(&pts, 0.0).unwrap();
        let rings = t.contour(8.0);
        assert_eq!(rings.len(), 1);
        assert!(rings[0].1, "a contour around a hill top is closed");
        // Back from faces: the same triangles and heights.
        let faces: Vec<[P3; 3]> = t.triangles.iter().map(|tr| tr.map(|i| t.points[i])).collect();
        let again = Tin::from_faces(&faces).unwrap();
        assert_eq!(again.triangles.len(), t.triangles.len());
        assert_eq!(again.points.len(), t.points.len());
        assert!((again.height([1.0, 2.0]).unwrap() - t.height([1.0, 2.0]).unwrap()).abs() < 1e-9);
    }

    #[test]
    fn earthwork_on_a_ramp() {
        // Ground z = x / 10 over 40 × 40 m; level 2: cut where x > 20, fill
        // where x < 20, each a wedge of 20 m × 2 m / 2 × 40 m = 800 m³.
        let t = ramp();
        let w = earthwork([0.0, 0.0, 40.0, 40.0], 0.5, 1.0, |p| t.height(p), |_| Some(2.0));
        assert!((w.cut - 800.0).abs() < 1.0 && (w.fill - 800.0).abs() < 1.0, "{w:?}");
        assert!((w.cut_area - 800.0).abs() < 1.0);
        assert_eq!(m3(1234567.4), "1.234.567");
    }

    #[test]
    fn long_edges_are_dropped() {
        // An L-shape: the triangles across the inner corner are long.
        let mut pts = Vec::new();
        for i in 0..6 {
            pts.push([i as f64 * 10.0, 0.0, 0.0]);
            pts.push([i as f64 * 10.0, 10.0, 0.0]);
            pts.push([0.0, i as f64 * 10.0, 0.0]);
            pts.push([10.0, i as f64 * 10.0, 0.0]);
        }
        let all = Tin::build(&pts, 0.0).unwrap();
        let cut = Tin::build(&pts, 15.0).unwrap();
        assert!(cut.triangles.len() < all.triangles.len());
        assert_eq!(cut.height([40.0, 40.0]), None);
    }
}
