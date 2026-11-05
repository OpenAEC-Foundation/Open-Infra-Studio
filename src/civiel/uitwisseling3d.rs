//! 3D exchange (issue #71): the drawing's 3D content out as glTF binary
//! (GLB), Wavefront OBJ (+ MTL) or STL (`CIV_3D_EXPORT`), and OBJ, STL and
//! GLB/glTF in as meshes (`CIV_3D_IMPORT`). The file formats themselves are
//! `rekenkern::mesh3d`; this module turns drawing entities into its
//! `MeshModel` and back.
//!
//! Export: meshes (MESH), polyface and polygon meshes and 3D faces, so TINs,
//! the photo terrain, 3D BAG, BGT in 3D, CityGML, KLIC and CPT columns,
//! corridor bodies and point cloud meshes. One glTF mesh/material per layer
//! and colour: the photo terrain is drawn as one mesh per palette colour
//! (true colours, no image), so it leaves as a set of coloured meshes
//! rather than a texture. Coordinates are metres in the project CRS with
//! the project location's base point as the local origin; the EPSG code and
//! the origin travel in the file (glTF `extras`, OBJ comments, STL header).
//!
//! Import: every mesh of the file becomes MESH entities (at most 100 000
//! vertices each) on one layer, with its triangle count, area and volume as
//! object data.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use ocs_plugin_api::host::codec::entities::{Mesh as CadMesh, MeshFace};
use ocs_plugin_api::host::codec::{CadDocument, Color, EntityType, Handle, Transparency, Vector3};
use rekenkern::mesh3d::{self, Mesh, MeshModel, ObjOptions};
use rekenkern::P3;

use super::{record, Ctx, Out};
use crate::gis::site::Site;
use crate::lang::tr;
use crate::nlcs::{self, Element, LayerSpec};

/// A message in the interface language, with positional arguments.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

/// Vertices per MESH entity, as `model3d::MeshBuilder` keeps them.
const MAX_VERTS: usize = 100_000;

// ── Formats ─────────────────────────────────────────────────────────────────

/// The exchange formats (`FORMAAT=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Formaat {
    Glb,
    Obj,
    Stl,
}

impl Formaat {
    pub fn parse(s: &str) -> Result<Formaat, String> {
        match s.trim().trim_start_matches('.').to_ascii_uppercase().as_str() {
            "GLB" | "GLTF" => Ok(Formaat::Glb),
            "OBJ" => Ok(Formaat::Obj),
            "STL" => Ok(Formaat::Stl),
            "USD" | "USDA" | "USDZ" => Err(tr("FORMAAT=USD wordt (nog) niet ondersteund: kies GLB, OBJ of STL", "FORMAAT=USD is not supported (yet): choose GLB, OBJ or STL").into()),
            _ => Err(tr("FORMAAT: GLB, OBJ of STL", "FORMAAT: GLB, OBJ or STL").into()),
        }
    }

    /// The format of a file, by its extension.
    pub fn of_path(p: &Path) -> Option<Formaat> {
        p.extension().and_then(|e| e.to_str()).and_then(|e| Formaat::parse(e).ok())
    }

    pub fn ext(self) -> &'static str {
        match self {
            Formaat::Glb => "glb",
            Formaat::Obj => "obj",
            Formaat::Stl => "stl",
        }
    }
}

// ── Colours ─────────────────────────────────────────────────────────────────

/// An sRGB byte as a linear 0–1 value (IEC 61966-2-1), as glTF wants
/// material colours.
pub fn linear(c: u8) -> f32 {
    let s = c as f64 / 255.0;
    (if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }) as f32
}

/// A linear 0–1 value back to an sRGB byte.
pub fn srgb(c: f32) -> u8 {
    let l = (c as f64).clamp(0.0, 1.0);
    let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// Layer colours from the layer table, else the NLCS table.
fn layer_colours(doc: &CadDocument) -> HashMap<String, (u8, u8, u8)> {
    let mut out: HashMap<String, (u8, u8, u8)> = doc.layers.iter().filter_map(|l| l.color.rgb().map(|c| (l.name.clone(), c))).collect();
    for e in doc.model_space_entities() {
        let name = &e.common().layer;
        if !out.contains_key(name) {
            if let Some(c) = nlcs_colour(name) {
                out.insert(name.clone(), c);
            }
        }
    }
    out
}

fn nlcs_colour(layer: &str) -> Option<(u8, u8, u8)> {
    let ln = nlcs::parse_layer(layer).ok()?;
    let obj = nlcs::object_by_name(&ln.group, &ln.object)?;
    let spec = nlcs::layer(obj, ln.status, &ln.discipline, Element::G);
    Some(spec.true_color.unwrap_or(spec.rgb))
}

// ── Drawing → model ─────────────────────────────────────────────────────────

/// Triangles of a drawing entity in drawing coordinates: its vertices and
/// counter-clockwise index triples (faces fanned from their first corner).
/// `Err` names why an entity has none.
pub fn triangles(e: &EntityType) -> Result<(Vec<P3>, Vec<[u32; 3]>), &'static str> {
    let v3 = |v: &Vector3| [v.x, v.y, v.z];
    let fan = |faces: &mut Vec<[u32; 3]>, f: &[usize], n: usize| {
        if f.len() < 3 || f.iter().any(|&i| i >= n) {
            return;
        }
        for k in 1..f.len() - 1 {
            let t = [f[0] as u32, f[k] as u32, f[k + 1] as u32];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                faces.push(t);
            }
        }
    };
    let (verts, tris) = match e {
        EntityType::Mesh(m) => {
            let verts: Vec<P3> = m.vertices.iter().map(v3).collect();
            let mut tris = Vec::new();
            for f in &m.faces {
                fan(&mut tris, &f.vertices, verts.len());
            }
            (verts, tris)
        }
        EntityType::PolyfaceMesh(m) => {
            let verts: Vec<P3> = m.vertices.iter().map(|v| v3(&v.location)).collect();
            let mut tris = Vec::new();
            for f in &m.faces {
                // 1-based; a negative index hides the edge after it.
                let idx: Vec<usize> = [f.index1, f.index2, f.index3, f.index4].iter().filter(|i| **i != 0).map(|i| i.unsigned_abs() as usize - 1).collect();
                fan(&mut tris, &idx, verts.len());
            }
            (verts, tris)
        }
        EntityType::PolygonMesh(m) => {
            let (rows, cols) = (m.m_vertex_count.max(0) as usize, m.n_vertex_count.max(0) as usize);
            let verts: Vec<P3> = m.vertices.iter().map(|v| v3(&v.location)).collect();
            if rows < 2 || cols < 2 || verts.len() < rows * cols {
                return Err("lege mesh");
            }
            let (closed_m, closed_n) = (m.is_closed_m(), m.is_closed_n());
            let mut tris = Vec::new();
            let at = |i: usize, j: usize| (i % rows) * cols + j % cols;
            for i in 0..(if closed_m { rows } else { rows - 1 }) {
                for j in 0..(if closed_n { cols } else { cols - 1 }) {
                    fan(&mut tris, &[at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)], verts.len());
                }
            }
            (verts, tris)
        }
        EntityType::Face3D(f) => {
            let mut c = vec![v3(&f.first_corner), v3(&f.second_corner), v3(&f.third_corner)];
            let d = v3(&f.fourth_corner);
            if (0..3).any(|k| (d[k] - c[2][k]).abs() > 1e-9) {
                c.push(d);
            }
            let mut tris = Vec::new();
            fan(&mut tris, &(0..c.len()).collect::<Vec<_>>(), c.len());
            (c, tris)
        }
        EntityType::Solid3D(_) | EntityType::Body(_) | EntityType::Region(_) | EntityType::Surface(_) => return Err("ACIS-solid of -vlak"),
        EntityType::Text(_) | EntityType::MText(_) | EntityType::Dimension(_) | EntityType::Leader(_) | EntityType::MultiLeader(_) => return Err("tekst of maat"),
        EntityType::Hatch(_) | EntityType::RasterImage(_) | EntityType::Wipeout(_) | EntityType::Viewport(_) => return Err("arcering, kaartbeeld of viewport"),
        EntityType::Insert(_) => return Err("blokverwijzing"),
        _ => return Err("lijn of punt (geen vlakken)"),
    };
    if tris.is_empty() {
        return Err("lege mesh");
    }
    Ok((verts, tris))
}

/// What [`collect`] gathered.
#[derive(Debug, Default)]
pub struct Verzameling {
    pub model: MeshModel,
    /// Entities that gave triangles.
    pub entities: usize,
    /// Entities left out, per reason.
    pub skipped: BTreeMap<String, usize>,
}

/// The EPSG number of a CRS name ("EPSG:28992").
pub fn epsg_of(crs: &str) -> Option<u32> {
    crs.trim().to_ascii_uppercase().strip_prefix("EPSG:").and_then(|n| n.trim().parse().ok())
}

/// The drawing's 3D content as a mesh model in project metres: one mesh per
/// layer and colour, named after the layer (with `#rrggbb` when a layer has
/// several colours). `f` drawing units per metre; the origin is the site's
/// base point (without a site: the rounded lower corner of the content).
pub fn collect(doc: &CadDocument, f: f64, site: Option<&Site>, only: Option<&HashSet<Handle>>) -> Verzameling {
    let f = if f > 0.0 { f } else { 1.0 };
    let world = |p: P3| -> P3 {
        match site {
            Some(s) => {
                let q = s.from_drawing([p[0], p[1]], f);
                [q[0], q[1], p[2] / f]
            }
            None => [p[0] / f, p[1] / f, p[2] / f],
        }
    };
    let colours = layer_colours(doc);
    let mut out = Verzameling::default();
    // (layer, rgb, alpha) → mesh; BTreeMap keeps the file order stable.
    let mut groups: BTreeMap<(String, (u8, u8, u8), u8), Mesh> = BTreeMap::new();
    for e in doc.model_space_entities() {
        let c = e.common();
        if only.is_some_and(|s| !s.contains(&c.handle)) {
            continue;
        }
        if c.invisible {
            *out.skipped.entry("onzichtbaar".into()).or_insert(0) += 1;
            continue;
        }
        let (verts, tris) = match triangles(e) {
            Ok(t) => t,
            Err(r) => {
                *out.skipped.entry(r.into()).or_insert(0) += 1;
                continue;
            }
        };
        let rgb = match c.color {
            Color::Rgb { r, g, b } => (r, g, b),
            Color::Index(_) => c.color.rgb().unwrap_or((160, 160, 160)),
            _ => colours.get(&c.layer).copied().unwrap_or((160, 160, 160)),
        };
        let alpha = match c.transparency {
            Transparency::Explicit(a) => 255 - a,
            _ => 255,
        };
        let m = groups.entry((c.layer.clone(), rgb, alpha)).or_insert_with(|| Mesh::new(&c.layer, Vec::new(), Vec::new(), [linear(rgb.0), linear(rgb.1), linear(rgb.2), alpha as f32 / 255.0]));
        let base = m.positions.len() as u32;
        m.positions.extend(verts.into_iter().map(world));
        m.indices.extend(tris.into_iter().map(|t| t.map(|i| i + base)));
        out.entities += 1;
    }
    let mut per_layer: HashMap<String, usize> = HashMap::new();
    for (layer, _, _) in groups.keys() {
        *per_layer.entry(layer.clone()).or_insert(0) += 1;
    }
    let meshes: Vec<Mesh> = groups
        .into_iter()
        .map(|((layer, rgb, _), mut m)| {
            if per_layer[&layer] > 1 {
                m.name = format!("{layer} #{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2);
            }
            m
        })
        .collect();
    let origin = match site {
        Some(s) => [s.origin_e, s.origin_n, 0.0],
        None => {
            let lo = meshes.iter().flat_map(|m| m.positions.iter()).fold([f64::MAX; 3], |a, p| [a[0].min(p[0]), a[1].min(p[1]), a[2].min(p[2])]);
            if lo[0] == f64::MAX { [0.0; 3] } else { [lo[0].floor(), lo[1].floor(), 0.0] }
        }
    };
    out.model = MeshModel { meshes, origin, epsg: site.and_then(|s| epsg_of(&s.crs)), texture: None };
    out
}

/// The model in one format: the main file's bytes, and for OBJ the MTL
/// text (named `<stem>.mtl`).
pub fn encode(model: &MeshModel, formaat: Formaat, stem: &str) -> Result<(Vec<u8>, Option<String>), String> {
    match formaat {
        Formaat::Glb => Ok((mesh3d::write_glb(model)?, None)),
        // Y-up, as most viewers and modellers expect OBJ; our reader turns it back.
        Formaat::Obj => {
            let opt = ObjOptions { z_up: false, mtl_file: Some(format!("{stem}.mtl")), texture_file: None };
            let (obj, mtl) = mesh3d::write_obj(model, &opt)?;
            Ok((obj.into_bytes(), Some(mtl)))
        }
        Formaat::Stl => Ok((mesh3d::write_stl_binary(model)?, None)),
    }
}

/// Reads a mesh file by its format; for OBJ, `mtl` is the material file's text.
pub fn decode(bytes: &[u8], formaat: Formaat, mtl: Option<&str>) -> Result<MeshModel, String> {
    match formaat {
        Formaat::Glb => mesh3d::read_gltf(bytes),
        Formaat::Obj => mesh3d::read_obj(std::str::from_utf8(bytes).map_err(|_| tr("OBJ is geen UTF-8-tekst", "OBJ is not UTF-8 text").to_string())?, mtl),
        Formaat::Stl => mesh3d::read_stl(bytes),
    }
}

/// The material file an OBJ names (`mtllib`).
pub fn mtllib(obj: &str) -> Option<String> {
    obj.lines().find_map(|l| l.trim().strip_prefix("mtllib ").map(|n| n.trim().to_string())).filter(|n| !n.is_empty())
}

/// `CIV_3D_EXPORT FORMAAT=GLB|OBJ|STL [SELECTIE] [PAD=]`.
pub fn export(formaat: Formaat, selection: bool, path: Option<PathBuf>, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = crate::bridge::target().ok_or("geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad")?;
    progress(tr("Tekening lezen…", "Reading the drawing…").into());
    let doc = crate::bridge::snapshot()?;
    let only = if selection {
        let s: HashSet<Handle> = crate::bridge::selection()?.into_iter().collect();
        if s.is_empty() {
            return Err(tr("selecteer eerst de objecten (of laat SELECTIE weg)", "select the objects first (or leave out SELECTIE)").into());
        }
        Some(s)
    } else {
        None
    };
    let site = crate::bridge::site();
    let v = collect(&doc, t.unit_factor, site.as_ref(), only.as_ref());
    if v.model.triangle_count() == 0 {
        return Err(tr("geen 3D-vlakken gevonden (meshes, polyface meshes of 3D-vlakken)", "no 3D faces found (meshes, polyface meshes or 3D faces)").into());
    }
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing(&format!("3D.{}", formaat.ext())));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model").to_string();
    progress(format!("{} → {}", formaat.ext().to_ascii_uppercase(), path.display()));
    let (bytes, mtl) = encode(&v.model, formaat, &stem)?;
    std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(mtl) = mtl {
        let p = path.with_extension("mtl");
        std::fs::write(&p, mtl).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let tris = v.model.triangle_count();
    super::api::note("bestand", path.to_string_lossy().into_owned());
    super::api::note("formaat", formaat.ext());
    super::api::note("driehoeken", tris);
    super::api::note("meshes", v.model.meshes.len());
    super::api::note("elementen", v.entities);
    super::api::note("oorsprong", v.model.origin.to_vec());
    super::api::note("epsg", v.model.epsg);
    super::api::note("overgeslagen", serde_json::to_value(&v.skipped).unwrap_or_default());
    super::api::note("lagen", v.model.meshes.iter().map(|m| serde_json::json!({"naam": m.name, "driehoeken": m.triangle_count()})).collect::<Vec<_>>());
    let epsg = v.model.epsg.map(|c| format!("EPSG:{c}")).unwrap_or_else(|| tr("zonder CRS (geen projectlocatie)", "without CRS (no project location)").into());
    let mut msg = t!(
        "3D {}: {} driehoeken in {} meshes uit {} elementen, oorsprong {:.0}, {:.0} ({}) → {}",
        "3D {}: {} triangles in {} meshes from {} entities, origin {:.0}, {:.0} ({}) → {}",
        formaat.ext().to_ascii_uppercase(),
        tris,
        v.model.meshes.len(),
        v.entities,
        v.model.origin[0],
        v.model.origin[1],
        epsg,
        path.display()
    );
    let ignored: usize = v.skipped.iter().filter(|(k, _)| k.as_str() != "lijn of punt (geen vlakken)" && k.as_str() != "tekst of maat").map(|(_, n)| n).sum();
    if ignored > 0 {
        let sk: Vec<String> = v.skipped.iter().map(|(k, n)| format!("{k} {n}")).collect();
        msg.push_str(&t!("; niet meegenomen: {}", "; left out: {}", sk.join(", ")));
    }
    Ok(msg)
}

// ── Model → drawing ─────────────────────────────────────────────────────────

/// Where an imported model goes (`PLAATS=`, `SCHAAL=`, `ROTATIE=`).
#[derive(Clone, Debug, PartialEq)]
pub struct Plaatsing {
    /// Insertion point (project metres) of the model's local origin.
    pub plaats: Option<P3>,
    pub schaal: f64,
    /// Degrees, counter-clockwise about the vertical through the origin.
    pub rotatie: f64,
}

impl Default for Plaatsing {
    fn default() -> Self {
        Plaatsing { plaats: None, schaal: 1.0, rotatie: 0.0 }
    }
}

impl Plaatsing {
    /// World (project) coordinates of a model point. The local point is
    /// relative to the model's origin when the file carries one. Without
    /// `plaats`: a georeferenced file stays where it is, a plain one is put
    /// with its origin on `base` (the project's base point).
    pub fn world(&self, model: &MeshModel, p: P3, base: P3) -> P3 {
        let l = [p[0] - model.origin[0], p[1] - model.origin[1], p[2] - model.origin[2]];
        let (s, c) = self.rotatie.to_radians().sin_cos();
        let k = self.schaal;
        let r = [k * (c * l[0] - s * l[1]), k * (s * l[0] + c * l[1]), k * l[2]];
        let at = match self.plaats {
            Some(q) => q,
            None if georeferenced(model) => model.origin,
            None => base,
        };
        [at[0] + r[0], at[1] + r[1], at[2] + r[2]]
    }
}

/// Whether a model carries its own placement (origin or EPSG in the file).
pub fn georeferenced(model: &MeshModel) -> bool {
    model.epsg.is_some() || model.origin != [0.0; 3]
}

/// Whether every edge of a mesh is shared by exactly two triangles (a
/// closed surface: only then is its volume meaningful).
pub fn is_closed(m: &Mesh) -> bool {
    if m.indices.is_empty() {
        return false;
    }
    let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
    for t in &m.indices {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    edges.values().all(|n| *n == 2)
}

/// A mesh as MESH entities in drawing coordinates (`to` maps a model point),
/// split so none has more than 100 000 vertices; degenerate triangles drop.
pub fn cad_meshes(m: &Mesh, to: impl Fn(P3) -> P3) -> Vec<CadMesh> {
    let mut out = Vec::new();
    let mut cur = CadMesh::new();
    let mut map: HashMap<u32, usize> = HashMap::new();
    for t in &m.indices {
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] || t.iter().any(|&i| i as usize >= m.positions.len()) {
            continue;
        }
        if cur.vertices.len() + 3 > MAX_VERTS {
            out.push(std::mem::replace(&mut cur, CadMesh::new()));
            map.clear();
        }
        let f: Vec<usize> = t
            .iter()
            .map(|&i| {
                *map.entry(i).or_insert_with(|| {
                    let p = to(m.positions[i as usize]);
                    cur.vertices.push(Vector3::new(p[0], p[1], p[2]));
                    cur.vertices.len() - 1
                })
            })
            .collect();
        cur.faces.push(MeshFace::new(f));
    }
    if !cur.faces.is_empty() {
        out.push(cur);
    }
    out
}

/// One imported mesh: its name, triangles, area (m²), volume (m³, only
/// for a closed mesh), colour and the entities to draw.
pub struct Geplaatst {
    pub naam: String,
    pub driehoeken: usize,
    pub oppervlak: f64,
    pub volume: Option<f64>,
    pub kleur: (u8, u8, u8),
    pub meshes: Vec<CadMesh>,
}

/// The model placed in the drawing: per mesh its entities and measures.
/// `world` gives project metres of a model point (see [`Plaatsing::world`]),
/// `drawing` turns project metres into drawing coordinates.
pub fn place(model: &MeshModel, world: impl Fn(P3) -> P3, drawing: impl Fn(P3) -> P3) -> Vec<Geplaatst> {
    model
        .meshes
        .iter()
        .filter(|m| !m.indices.is_empty())
        .map(|m| {
            let placed = Mesh { positions: m.positions.iter().map(|p| world(*p)).collect(), ..m.clone() };
            let closed = is_closed(&placed);
            Geplaatst {
                naam: m.name.clone(),
                driehoeken: m.triangle_count(),
                oppervlak: placed.area(),
                volume: closed.then(|| placed.volume().abs()),
                kleur: (srgb(m.color[0]), srgb(m.color[1]), srgb(m.color[2])),
                meshes: cad_meshes(&placed, &drawing),
            }
        })
        .collect()
}

/// The layer for `LAAG=`: a full layer name (NLCS or not), or an NLCS
/// "HOOFDGROEP OBJECT" ("KW KUNSTWERK"); default the structure object.
pub fn layer_spec(ctx: &Ctx, laag: Option<&str>) -> Result<LayerSpec, String> {
    let Some(name) = laag.map(str::trim).filter(|n| !n.is_empty()) else {
        return ctx.layer("KW", "KUNSTWERK", Element::G);
    };
    if let Ok(ln) = nlcs::parse_layer(name) {
        if let Some(obj) = nlcs::object_by_name(&ln.group, &ln.object) {
            let el = match ln.element.as_str() {
                "S" => Element::S,
                "T" => Element::T,
                "A" => Element::A,
                _ => Element::G,
            };
            return Ok(nlcs::layer(obj, ln.status, &ln.discipline, el));
        }
    }
    if let Some((g, o)) = name.split_once(char::is_whitespace) {
        if nlcs::object_by_name(&g.trim().to_ascii_uppercase(), &o.trim().to_ascii_uppercase()).is_some() {
            return ctx.layer(&g.trim().to_ascii_uppercase(), &o.trim().to_ascii_uppercase(), Element::G);
        }
    }
    Ok(LayerSpec { name: name.to_string(), color: 7, rgb: nlcs::color_rgb(7), true_color: None, lineweight: None, linetype: "CONTINUOUS".into(), object_id: 0 })
}

/// `CIV_3D_IMPORT <pad.obj|stl|glb> [PLAATS=x,y,z] [SCHAAL=] [ROTATIE=] [LAAG=]`.
pub fn import(opt: &super::CivielOptions, path: &Path, plaatsing: Plaatsing, laag: Option<String>, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let formaat = Formaat::of_path(path).ok_or_else(|| tr("bestand moet .obj, .stl, .glb of .gltf zijn", "the file must be .obj, .stl, .glb or .gltf").to_string())?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    progress(t!("{} lezen…", "Reading {}…", formaat.ext().to_ascii_uppercase()));
    let mtl = if formaat == Formaat::Obj {
        std::str::from_utf8(&bytes).ok().and_then(mtllib).and_then(|n| std::fs::read_to_string(path.with_file_name(n)).ok())
    } else {
        None
    };
    let model = decode(&bytes, formaat, mtl.as_deref())?;
    if model.triangle_count() == 0 {
        return Err(tr("het bestand bevat geen driehoeken", "the file holds no triangles").into());
    }
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = ctx.site.as_ref().map(|s| [s.origin_e, s.origin_n, 0.0]).unwrap_or([0.0; 3]);
    // A file in another CRS than the project: its points are transformed.
    let site_crs = ctx.site.as_ref().map(|s| s.crs.clone());
    let reproject = match (model.epsg, site_crs.as_deref()) {
        (Some(e), Some(c)) if plaatsing.plaats.is_none() && epsg_of(c) != Some(e) => Some(crate::gis::crs::Transformer::new(&format!("EPSG:{e}"), c)?),
        _ => None,
    };
    let f = ctx.f;
    let world = |p: P3| {
        let w = plaatsing.world(&model, p, base);
        match &reproject {
            Some(t) => t.apply([w[0], w[1]]).map(|q| [q[0], q[1], w[2]]).unwrap_or(w),
            None => w,
        }
    };
    let drawing = |p: P3| {
        let q = ctx.to_drawing([p[0], p[1]]);
        [q[0], q[1], p[2] * f]
    };
    let placed = place(&model, world, drawing);
    let spec = layer_spec(&ctx, laag.as_deref())?;
    let mut out = Out::default();
    let mut total = 0;
    let mut volume = 0.0;
    let mut area = 0.0;
    let mut open = 0;
    for g in &placed {
        let mut pairs = vec![("mesh3d", g.naam.clone()), ("bron", file.clone()), ("driehoeken", g.driehoeken.to_string()), ("oppervlak_m2", format!("{:.3}", g.oppervlak))];
        match g.volume {
            Some(v) => pairs.push(("volume_m3", format!("{v:.3}"))),
            None => pairs.push(("gesloten", "nee".into())),
        }
        let d = record(&pairs);
        let colour = (g.kleur != (204, 204, 204) && g.kleur != (231, 231, 231)).then_some(Color::Rgb { r: g.kleur.0, g: g.kleur.1, b: g.kleur.2 });
        let ents: Vec<EntityType> = g
            .meshes
            .iter()
            .cloned()
            .map(|m| {
                let mut e = EntityType::Mesh(m);
                let c = e.common_mut();
                c.layer = spec.name.clone();
                c.color = colour.unwrap_or(Color::ByLayer);
                c.extended_data.add_record(d.clone());
                e
            })
            .collect();
        out.put(&spec, ents);
        total += g.driehoeken;
        area += g.oppervlak;
        match g.volume {
            Some(v) => volume += v,
            None => open += 1,
        }
    }
    let handles = out.write_handles(&format!("3D {file}"))?;
    super::api::note("bestand", path.to_string_lossy().into_owned());
    super::api::note("driehoeken", total);
    super::api::note("meshes", placed.len());
    super::api::note("oppervlak_m2", area);
    super::api::note("volume_m3", volume);
    super::api::note("open_meshes", open);
    super::api::note("laag", spec.name.clone());
    super::api::note("handles", handles.iter().map(|h| super::api::handle_text(*h)).collect::<Vec<_>>());
    super::api::note("onderdelen", placed.iter().map(|g| serde_json::json!({"naam": g.naam, "driehoeken": g.driehoeken, "oppervlak_m2": g.oppervlak, "volume_m3": g.volume})).collect::<Vec<_>>());
    let vol = if open == placed.len() {
        tr("geen gesloten volume", "no closed volume").to_string()
    } else {
        let mut s = t!("volume {:.2} m³", "volume {:.2} m³", volume);
        if open > 0 {
            s.push_str(&t!(" ({} open meshes zonder volume)", " ({} open meshes without volume)", open));
        }
        s
    };
    Ok(t!(
        "{}: {} meshes, {} driehoeken, oppervlak {:.1} m², {} — {} elementen op {}.",
        "{}: {} meshes, {} triangles, area {:.1} m², {} — {} entities on {}.",
        file,
        placed.len(),
        total,
        area,
        vol,
        handles.len(),
        spec.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::entities::Face3D;

    fn on(mut e: EntityType, layer: &str, rgb: Option<(u8, u8, u8)>) -> EntityType {
        let c = e.common_mut();
        c.layer = layer.into();
        if let Some((r, g, b)) = rgb {
            c.color = Color::Rgb { r, g, b };
        }
        e
    }

    /// A closed unit cube (12 triangles, outward) and a square of two faces.
    fn drawing() -> CadDocument {
        let mut doc = CadDocument::new();
        let mut cube = CadMesh::new();
        let p = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]];
        cube.vertices = p.iter().map(|q| Vector3::new(q[0] + 10.0, q[1] + 20.0, q[2])).collect();
        for f in [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]] {
            cube.faces.push(MeshFace::new(f.to_vec()));
        }
        doc.add_entity(on(EntityType::Mesh(cube), "N-XX-KW-KUNSTWERK-G", Some((200, 60, 40)))).unwrap();
        for (a, b, c) in [([0., 0., 1.], [5., 0., 1.], [5., 5., 1.5]), ([0., 0., 1.], [5., 5., 1.5], [0., 5., 1.2])] {
            let mut f = Face3D::new(Vector3::new(a[0], a[1], a[2]), Vector3::new(b[0], b[1], b[2]), Vector3::new(c[0], c[1], c[2]), Vector3::new(c[0], c[1], c[2]));
            f.common.layer = "B-XX-OG-TRIANGULATIE-G".into();
            doc.add_entity(EntityType::Face3D(f)).unwrap();
        }
        doc
    }

    fn site() -> Site {
        crate::ifc::tests::sample_site()
    }

    #[test]
    fn colours_round_trip_through_linear() {
        for c in [0u8, 1, 10, 60, 128, 200, 255] {
            assert_eq!(srgb(linear(c)), c);
        }
        assert!((linear(255) - 1.0).abs() < 1e-6 && linear(0) == 0.0);
    }

    #[test]
    fn the_drawing_becomes_meshes_per_layer_and_colour() {
        let doc = drawing();
        let v = collect(&doc, 1.0, Some(&site()), None);
        assert_eq!(v.entities, 3);
        assert_eq!(v.model.triangle_count(), 14);
        assert_eq!(v.model.meshes.len(), 2);
        assert_eq!(v.model.origin, [105000.0, 425000.0, 0.0]);
        assert_eq!(v.model.epsg, Some(28992));
        let cube = v.model.meshes.iter().find(|m| m.name.contains("KUNSTWERK")).unwrap();
        // Drawing (10, 20) relative to the site's base point.
        assert_eq!(cube.positions[0], [105010.0, 425020.0, 0.0]);
        assert!((cube.volume() - 1.0).abs() < 1e-9 && is_closed(cube));
        assert_eq!(srgb(cube.color[0]), 200);
        // A millimetre drawing gives the same model.
        let mut mm = CadDocument::new();
        for e in doc.model_space_entities() {
            let mut e = e.clone();
            if let EntityType::Mesh(m) = &mut e {
                m.vertices.iter_mut().for_each(|v| *v = Vector3::new(v.x * 1000.0, v.y * 1000.0, v.z * 1000.0));
                mm.add_entity(e).unwrap();
            }
        }
        let w = collect(&mm, 1000.0, Some(&site()), None);
        assert!((w.model.meshes[0].volume() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn export_and_import_keep_the_triangles() {
        let doc = drawing();
        let s = site();
        let v = collect(&doc, 1.0, Some(&s), None);
        for formaat in [Formaat::Glb, Formaat::Obj, Formaat::Stl] {
            let (bytes, mtl) = encode(&v.model, formaat, "model").unwrap();
            let back = decode(&bytes, formaat, mtl.as_deref()).unwrap();
            assert_eq!(back.triangle_count(), 14, "{formaat:?}");
            assert!(georeferenced(&back), "{formaat:?}");
            let base = [s.origin_e, s.origin_n, 0.0];
            let placed = place(&back, |p| Plaatsing::default().world(&back, p, base), |p| {
                let q = s.to_drawing([p[0], p[1]], 1.0);
                [q[0], q[1], p[2]]
            });
            let faces: usize = placed.iter().flat_map(|g| g.meshes.iter()).map(|m| m.faces.len()).sum();
            assert_eq!(faces, 14, "{formaat:?}");
            // Back where it was drawn, within single precision of the file.
            let all: Vec<&Vector3> = placed.iter().flat_map(|g| g.meshes.iter()).flat_map(|m| m.vertices.iter()).collect();
            assert!(all.iter().any(|q| (q.x - 11.0).abs() < 1e-4 && (q.y - 21.0).abs() < 1e-4 && (q.z - 1.0).abs() < 1e-4), "{formaat:?}");
            if formaat != Formaat::Stl {
                let cube = placed.iter().find(|g| g.naam.contains("KUNSTWERK")).unwrap();
                assert!((cube.volume.unwrap() - 1.0).abs() < 1e-4, "{formaat:?}");
                assert_eq!(cube.kleur, (200, 60, 40), "{formaat:?}");
            }
        }
    }

    #[test]
    fn placement_scale_and_rotation() {
        let mut m = MeshModel::default();
        m.meshes.push(Mesh::new("a", vec![[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]], vec![[0, 1, 2]], [1.0; 4]));
        assert!(!georeferenced(&m));
        let pl = Plaatsing { plaats: Some([100.0, 200.0, 5.0]), schaal: 2.0, rotatie: 90.0 };
        let w = pl.world(&m, [1.0, 0.0, 1.0], [0.0; 3]);
        assert!((w[0] - 100.0).abs() < 1e-9 && (w[1] - 202.0).abs() < 1e-9 && (w[2] - 7.0).abs() < 1e-9);
        // Without PLAATS a plain file sits on the project's base point.
        let w = Plaatsing::default().world(&m, [1.0, 0.0, 0.0], [105000.0, 425000.0, 0.0]);
        assert_eq!(w, [105001.0, 425000.0, 0.0]);
        // Scale: an open triangle has an area, no volume.
        let placed = place(&m, |p| Plaatsing { schaal: 3.0, ..Plaatsing::default() }.world(&m, p, [0.0; 3]), |p| p);
        assert!((placed[0].oppervlak - 4.5).abs() < 1e-9 && placed[0].volume.is_none());
    }

    #[test]
    fn big_meshes_split_into_entities() {
        let n = 60_000u32;
        let positions: Vec<P3> = (0..n * 3).map(|i| [i as f64, (i % 7) as f64, 0.0]).collect();
        let indices: Vec<[u32; 3]> = (0..n).map(|k| [3 * k, 3 * k + 1, 3 * k + 2]).collect();
        let m = Mesh::new("big", positions, indices, [1.0; 4]);
        let parts = cad_meshes(&m, |p| p);
        assert!(parts.len() >= 2 && parts.iter().all(|p| p.vertices.len() <= MAX_VERTS));
        assert_eq!(parts.iter().map(|p| p.faces.len()).sum::<usize>(), n as usize);
    }

    #[test]
    fn formats_and_material_files() {
        assert_eq!(Formaat::parse("glb").unwrap(), Formaat::Glb);
        assert_eq!(Formaat::of_path(Path::new("/x/brug.STL")), Some(Formaat::Stl));
        assert!(Formaat::parse("USD").unwrap_err().contains("USD"));
        assert_eq!(mtllib("# x\nmtllib model.mtl\nv 0 0 0"), Some("model.mtl".into()));
        assert_eq!(epsg_of("EPSG:28992"), Some(28992));
    }
}
