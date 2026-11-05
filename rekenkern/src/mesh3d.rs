//! 3D exchange of triangle meshes: glTF 2.0 binary (GLB), Wavefront OBJ
//! (+ MTL) and STL, both ways. Issue #71.
//!
//! The model keeps world coordinates in `f64` (RD New or UTM metres are too
//! large for `f32`) and every writer subtracts a local origin before writing
//! single precision. The origin and the EPSG code travel along: in GLB as
//! `asset.extras` and `scenes[0].extras` (`{"origin":[x,y,z],"epsg":n}`), in
//! OBJ as `# origin`/`# epsg` comments, in STL in the 80-byte binary header or
//! the ASCII `solid` line. Readers add the origin back when they find it.
//!
//! Axes: the model is Z-up (civil convention). glTF is Y-up with +Z towards
//! the viewer (glTF 2.0 §3.4), so a model point (x, y, z) is written as
//! (x, z, −y) and read back as (X, −Z, Y). OBJ stays Z-up unless asked
//! otherwise (`# up Y` is then written so our reader converts back). UVs use
//! the glTF convention (v = 0 at the image's top row); OBJ `vt` stores 1 − v.
//!
//! Sources: Khronos glTF 2.0 specification (GLB container §4.4, accessors
//! §3.6, materials §3.9); Wavefront OBJ/MTL (Appendix B1/B2 of the Advanced
//! Visualizer manual); STL per 3D Systems' "SLC/STL" format description.
//! Volume by the divergence theorem: V = Σ (a · (b × c)) / 6 over the
//! triangles of a closed, outward-oriented mesh.

use crate::P3;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::collections::HashMap;

/// An embedded image (the terrain's aerial photo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    /// `image/png` or `image/jpeg`.
    pub mime: String,
    pub data: Vec<u8>,
}

impl Texture {
    /// Wraps image bytes, recognising PNG/JPEG by their signature.
    pub fn from_bytes(data: Vec<u8>) -> Result<Texture, String> {
        let mime = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "image/jpeg"
        } else {
            return Err("texture must be PNG or JPEG".into());
        };
        Ok(Texture {
            mime: mime.into(),
            data,
        })
    }
    fn ext(&self) -> &str {
        if self.mime == "image/jpeg" {
            "jpg"
        } else {
            "png"
        }
    }
}

/// A named triangle mesh with one colour (one layer's material).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub name: String,
    /// World coordinates, Z-up.
    pub positions: Vec<P3>,
    /// Counter-clockwise seen from outside.
    pub indices: Vec<[u32; 3]>,
    /// Linear RGBA 0–1.
    pub color: [f32; 4],
    /// Texture coordinates per position (glTF convention); the mesh then
    /// shows the model's texture.
    pub uvs: Option<Vec<[f32; 2]>>,
}

/// Meshes with their local origin, CRS and optional texture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshModel {
    pub meshes: Vec<Mesh>,
    /// Subtracted before writing single precision.
    pub origin: P3,
    /// CRS of the world coordinates (28992 = RD New).
    pub epsg: Option<u32>,
    pub texture: Option<Texture>,
}

/// Options for [`write_obj`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjOptions {
    /// Keep Z-up; false writes Y-up (x, z, −y) for viewers that expect it.
    pub z_up: bool,
    /// MTL file name for `mtllib`; `None` writes no material reference.
    pub mtl_file: Option<String>,
    /// Image file name for `map_Kd` of textured meshes.
    pub texture_file: Option<String>,
}

impl Default for ObjOptions {
    fn default() -> Self {
        ObjOptions {
            z_up: true,
            mtl_file: Some("model.mtl".into()),
            texture_file: None,
        }
    }
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: P3, b: P3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: P3) -> f64 {
    dot(a, a).sqrt()
}

impl Mesh {
    /// A mesh without UVs.
    pub fn new(name: &str, positions: Vec<P3>, indices: Vec<[u32; 3]>, color: [f32; 4]) -> Mesh {
        Mesh {
            name: name.into(),
            positions,
            indices,
            color,
            uvs: None,
        }
    }
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len()
    }
    fn tri(&self, t: &[u32; 3]) -> [P3; 3] {
        [
            self.positions[t[0] as usize],
            self.positions[t[1] as usize],
            self.positions[t[2] as usize],
        ]
    }
    /// Signed volume (m³) by the divergence theorem; positive for a closed
    /// outward-oriented mesh. Computed relative to the first vertex.
    pub fn volume(&self) -> f64 {
        let Some(&o) = self.positions.first() else {
            return 0.0;
        };
        self.indices
            .iter()
            .map(|t| {
                let [a, b, c] = self.tri(t);
                dot(sub(a, o), cross(sub(b, o), sub(c, o)))
            })
            .sum::<f64>()
            / 6.0
    }
    /// Surface area (m²).
    pub fn area(&self) -> f64 {
        self.indices
            .iter()
            .map(|t| {
                let [a, b, c] = self.tri(t);
                norm(cross(sub(b, a), sub(c, a))) / 2.0
            })
            .sum()
    }
    /// Axis-aligned bounds (min, max); `None` when empty.
    pub fn bbox(&self) -> Option<(P3, P3)> {
        bbox_of(self.positions.iter())
    }
    /// Checks index bounds and UV count.
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len();
        if let Some(t) = self
            .indices
            .iter()
            .find(|t| t.iter().any(|&i| i as usize >= n))
        {
            return Err(format!(
                "mesh '{}': index {:?} out of range ({n} positions)",
                self.name, t
            ));
        }
        if let Some(uv) = &self.uvs {
            if uv.len() != n {
                return Err(format!(
                    "mesh '{}': {} UVs for {n} positions",
                    self.name,
                    uv.len()
                ));
            }
        }
        Ok(())
    }
}

fn bbox_of<'a>(pts: impl Iterator<Item = &'a P3>) -> Option<(P3, P3)> {
    let mut it = pts.peekable();
    let first = **it.peek()?;
    Some(it.fold((first, first), |(mut lo, mut hi), p| {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
        (lo, hi)
    }))
}

impl MeshModel {
    /// An empty model around `origin`.
    pub fn new(origin: P3, epsg: Option<u32>) -> MeshModel {
        MeshModel {
            meshes: Vec::new(),
            origin,
            epsg,
            texture: None,
        }
    }
    /// Total triangles.
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(Mesh::triangle_count).sum()
    }
    /// Sum of mesh volumes.
    pub fn volume(&self) -> f64 {
        self.meshes.iter().map(Mesh::volume).sum()
    }
    /// Sum of mesh areas.
    pub fn area(&self) -> f64 {
        self.meshes.iter().map(Mesh::area).sum()
    }
    /// Bounds over all meshes.
    pub fn bbox(&self) -> Option<(P3, P3)> {
        bbox_of(self.meshes.iter().flat_map(|m| m.positions.iter()))
    }
    /// Validates all meshes.
    pub fn validate(&self) -> Result<(), String> {
        self.meshes.iter().try_for_each(Mesh::validate)
    }
    fn local(&self, p: P3) -> [f32; 3] {
        let d = sub(p, self.origin);
        [d[0] as f32, d[1] as f32, d[2] as f32]
    }
    fn georef_json(&self) -> Json {
        let mut e = json!({ "origin": self.origin, "up": "glTF Y-up; model Z-up: (x, y, z) = (X, -Z, Y) + origin" });
        if let Some(c) = self.epsg {
            e["epsg"] = json!(c);
        }
        e
    }
    fn header_text(&self) -> String {
        let mut s = format!(
            "origin={},{},{}",
            self.origin[0], self.origin[1], self.origin[2]
        );
        if let Some(c) = self.epsg {
            s.push_str(&format!(" epsg={c}"));
        }
        s
    }
    fn apply_header_text(&mut self, text: &str) {
        for w in text.split_whitespace() {
            if let Some(v) = w.strip_prefix("origin=") {
                let c: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                if c.len() == 3 {
                    self.origin = [c[0], c[1], c[2]];
                }
            } else if let Some(v) = w.strip_prefix("epsg=") {
                self.epsg = v.parse().ok();
            }
        }
    }
}

// ---------------------------------------------------------------- GLB write

const GLB_MAGIC: u32 = 0x4654_6C67; // "glTF"
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

fn pad4(v: &mut Vec<u8>, byte: u8) {
    while !v.len().is_multiple_of(4) {
        v.push(byte);
    }
}

/// Writes glTF 2.0 binary. Meshes without triangles are skipped.
pub fn write_glb(model: &MeshModel) -> Result<Vec<u8>, String> {
    model.validate()?;
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut materials = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let textured = model.texture.is_some();
    let mut view = |bin: &mut Vec<u8>, data: &[u8], target: Option<u32>| -> usize {
        pad4(bin, 0);
        let mut v = json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": data.len() });
        if let Some(t) = target {
            v["target"] = json!(t);
        }
        bin.extend_from_slice(data);
        views.push(v);
        views.len() - 1
    };
    for m in model.meshes.iter().filter(|m| !m.indices.is_empty()) {
        // Positions, Y-up.
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut pos = Vec::with_capacity(m.positions.len() * 12);
        for &p in &m.positions {
            let l = model.local(p);
            let y = [l[0], l[2], -l[1]];
            for k in 0..3 {
                lo[k] = lo[k].min(y[k]);
                hi[k] = hi[k].max(y[k]);
                pos.extend_from_slice(&y[k].to_le_bytes());
            }
        }
        let pv = view(&mut bin, &pos, Some(34962));
        accessors.push(
            json!({ "bufferView": pv, "componentType": 5126, "count": m.positions.len(),
            "type": "VEC3", "min": lo, "max": hi }),
        );
        let pa = accessors.len() - 1;
        let mut attrs = json!({ "POSITION": pa });
        if let (Some(uv), true) = (&m.uvs, textured) {
            let bytes: Vec<u8> = uv
                .iter()
                .flat_map(|t| t.iter().flat_map(|c| c.to_le_bytes()))
                .collect();
            let uvv = view(&mut bin, &bytes, Some(34962));
            accessors.push(json!({ "bufferView": uvv, "componentType": 5126, "count": uv.len(), "type": "VEC2" }));
            attrs["TEXCOORD_0"] = json!(accessors.len() - 1);
        }
        let idx: Vec<u8> = m
            .indices
            .iter()
            .flat_map(|t| t.iter().flat_map(|i| i.to_le_bytes()))
            .collect();
        let iv = view(&mut bin, &idx, Some(34963));
        accessors.push(json!({ "bufferView": iv, "componentType": 5125, "count": m.indices.len() * 3, "type": "SCALAR" }));
        let ia = accessors.len() - 1;
        let mut pbr =
            json!({ "baseColorFactor": m.color, "metallicFactor": 0.0, "roughnessFactor": 1.0 });
        if textured && m.uvs.is_some() {
            pbr["baseColorTexture"] = json!({ "index": 0 });
        }
        let mut mat = json!({ "name": m.name, "pbrMetallicRoughness": pbr, "doubleSided": true });
        if m.color[3] < 1.0 {
            mat["alphaMode"] = json!("BLEND");
        }
        materials.push(mat);
        meshes.push(
            json!({ "name": m.name, "primitives": [{ "attributes": attrs, "indices": ia,
            "material": materials.len() - 1, "mode": 4 }] }),
        );
        nodes.push(json!({ "name": m.name, "mesh": meshes.len() - 1 }));
    }
    let georef = model.georef_json();
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "Open Infra rekenkern", "extras": georef },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>(), "extras": georef }],
        "nodes": nodes, "meshes": meshes, "materials": materials, "accessors": accessors,
    });
    if let Some(t) = &model.texture {
        let iv = view(&mut bin, &t.data, None);
        doc["images"] = json!([{ "bufferView": iv, "mimeType": t.mime }]);
        doc["samplers"] =
            json!([{ "magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071 }]);
        doc["textures"] = json!([{ "source": 0, "sampler": 0 }]);
    }
    pad4(&mut bin, 0);
    if !bin.is_empty() {
        doc["buffers"] = json!([{ "byteLength": bin.len() }]);
    }
    doc["bufferViews"] = json!(views);
    let mut js = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    pad4(&mut js, b' ');
    let total = 12 + 8 + js.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&js);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

// ----------------------------------------------------------------- GLB read

fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| "truncated glTF data".to_string())
}

/// Reads GLB, or glTF JSON whose buffers are embedded `data:` URIs.
/// Triangle primitives (modes 4, 5, 6) of the default scene are returned,
/// one mesh per primitive, with node transforms applied and converted back
/// to Z-up world coordinates.
pub fn read_gltf(bytes: &[u8]) -> Result<MeshModel, String> {
    let (doc, bin): (Json, Option<&[u8]>) = if bytes.starts_with(b"glTF") {
        let total = u32_at(bytes, 8)? as usize;
        if total > bytes.len() {
            return Err("GLB length exceeds data".into());
        }
        let jl = u32_at(bytes, 12)? as usize;
        if u32_at(bytes, 16)? != CHUNK_JSON {
            return Err("first GLB chunk is not JSON".into());
        }
        let js = bytes.get(20..20 + jl).ok_or("truncated GLB JSON chunk")?;
        let doc: Json = serde_json::from_slice(js).map_err(|e| format!("GLB JSON: {e}"))?;
        let o = 20 + jl;
        let bin = if o + 8 <= total && u32_at(bytes, o + 4)? == CHUNK_BIN {
            let bl = u32_at(bytes, o)? as usize;
            Some(
                bytes
                    .get(o + 8..o + 8 + bl)
                    .ok_or("truncated GLB BIN chunk")?,
            )
        } else {
            None
        };
        (doc, bin)
    } else {
        (
            serde_json::from_slice(bytes).map_err(|e| format!("glTF JSON: {e}"))?,
            None,
        )
    };
    if let Some(req) = doc["extensionsRequired"].as_array() {
        if !req.is_empty() {
            return Err(format!(
                "unsupported required glTF extensions: {}",
                Json::Array(req.clone())
            ));
        }
    }
    // Buffers.
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for (k, b) in doc["buffers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        match b["uri"].as_str() {
            None if k == 0 => buffers.push(bin.ok_or("GLB without BIN chunk")?.to_vec()),
            None => return Err("buffer without uri".into()),
            Some(u) if u.starts_with("data:") => {
                let data = u
                    .split_once(";base64,")
                    .ok_or("only base64 data URIs are supported")?
                    .1;
                buffers.push(base64_decode(data)?);
            }
            Some(u) => return Err(format!("external buffer '{u}' not supported; use GLB")),
        }
    }
    let r = GltfReader {
        doc: &doc,
        buffers: &buffers,
    };
    let mut model = MeshModel::default();
    for ex in [
        &doc["asset"]["extras"],
        &doc["scenes"][doc["scene"].as_u64().unwrap_or(0) as usize]["extras"],
    ] {
        if let Some(o) = ex["origin"].as_array() {
            if o.len() == 3 {
                model.origin = [
                    o[0].as_f64().unwrap_or(0.0),
                    o[1].as_f64().unwrap_or(0.0),
                    o[2].as_f64().unwrap_or(0.0),
                ];
            }
        }
        if let Some(e) = ex["epsg"].as_u64() {
            model.epsg = Some(e as u32);
        }
    }
    let roots: Vec<usize> = match doc["scenes"].get(doc["scene"].as_u64().unwrap_or(0) as usize) {
        Some(s) => s["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_u64().map(|n| n as usize))
                    .collect()
            })
            .unwrap_or_default(),
        None => {
            // No scene: every node that is nobody's child.
            let nodes = doc["nodes"].as_array().cloned().unwrap_or_default();
            let mut child = vec![false; nodes.len()];
            for n in &nodes {
                for c in n["children"].as_array().into_iter().flatten() {
                    if let Some(c) = c.as_u64() {
                        if let Some(f) = child.get_mut(c as usize) {
                            *f = true;
                        }
                    }
                }
            }
            (0..nodes.len()).filter(|&k| !child[k]).collect()
        }
    };
    for n in roots {
        r.node(n, IDENTITY, &mut model, 0)?;
    }
    Ok(model)
}

type Mat4 = [f64; 16]; // column-major as in glTF

const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

fn trs(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> Mat4 {
    let [x, y, z, w] = q;
    let r = [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + z * w),
        2.0 * (x * z - y * w),
        2.0 * (x * y - z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z + x * w),
        2.0 * (x * z + y * w),
        2.0 * (y * z - x * w),
        1.0 - 2.0 * (x * x + y * y),
    ];
    [
        r[0] * s[0],
        r[1] * s[0],
        r[2] * s[0],
        0.0,
        r[3] * s[1],
        r[4] * s[1],
        r[5] * s[1],
        0.0,
        r[6] * s[2],
        r[7] * s[2],
        r[8] * s[2],
        0.0,
        t[0],
        t[1],
        t[2],
        1.0,
    ]
}

fn nums<const N: usize>(v: &Json, default: [f64; N]) -> [f64; N] {
    let mut out = default;
    if let Some(a) = v.as_array() {
        for (k, x) in a.iter().take(N).enumerate() {
            out[k] = x.as_f64().unwrap_or(default[k]);
        }
    }
    out
}

/// The bytes of one accessor and how to step through them.
struct AccessorView<'a> {
    bytes: &'a [u8],
    stride: Option<usize>,
    count: usize,
    component: u64,
    normalized: bool,
}

struct GltfReader<'a> {
    doc: &'a Json,
    buffers: &'a [Vec<u8>],
}

impl GltfReader<'_> {
    fn node(
        &self,
        n: usize,
        parent: Mat4,
        model: &mut MeshModel,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("glTF node hierarchy too deep or cyclic".into());
        }
        let node = self.doc["nodes"]
            .get(n)
            .ok_or_else(|| format!("node {n} missing"))?;
        let local = if node["matrix"].is_array() {
            nums::<16>(&node["matrix"], IDENTITY)
        } else {
            trs(
                nums(&node["translation"], [0.0; 3]),
                nums(&node["rotation"], [0.0, 0.0, 0.0, 1.0]),
                nums(&node["scale"], [1.0; 3]),
            )
        };
        let world = mat_mul(&parent, &local);
        if let Some(mi) = node["mesh"].as_u64() {
            self.mesh(mi as usize, &world, model)?;
        }
        for c in node["children"].as_array().into_iter().flatten() {
            if let Some(c) = c.as_u64() {
                self.node(c as usize, world, model, depth + 1)?;
            }
        }
        Ok(())
    }

    fn mesh(&self, mi: usize, m: &Mat4, model: &mut MeshModel) -> Result<(), String> {
        let mesh = self.doc["meshes"]
            .get(mi)
            .ok_or_else(|| format!("mesh {mi} missing"))?;
        let name = mesh["name"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("mesh{mi}"));
        let prims = mesh["primitives"].as_array().cloned().unwrap_or_default();
        for (k, p) in prims.iter().enumerate() {
            let mode = p["mode"].as_u64().unwrap_or(4);
            if !(4..=6).contains(&mode) {
                continue; // points and lines
            }
            if p["extensions"].get("KHR_draco_mesh_compression").is_some() {
                return Err("Draco-compressed glTF is not supported".into());
            }
            let pa = p["attributes"]["POSITION"]
                .as_u64()
                .ok_or("primitive without POSITION")? as usize;
            let raw = self.floats(pa, 3)?;
            let positions: Vec<P3> = raw
                .chunks(3)
                .map(|v| {
                    let w = [
                        m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12],
                        m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13],
                        m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14],
                    ];
                    [
                        w[0] + model.origin[0],
                        -w[2] + model.origin[1],
                        w[1] + model.origin[2],
                    ]
                })
                .collect();
            let flat: Vec<u32> = match p["indices"].as_u64() {
                Some(ia) => self.ints(ia as usize)?,
                None => (0..positions.len() as u32).collect(),
            };
            let mut indices = Vec::new();
            match mode {
                4 => indices.extend(flat.as_chunks::<3>().0.iter().copied()),
                5 => {
                    for k in 2..flat.len() {
                        let t = if k % 2 == 0 {
                            [flat[k - 2], flat[k - 1], flat[k]]
                        } else {
                            [flat[k - 1], flat[k - 2], flat[k]]
                        };
                        indices.push(t);
                    }
                }
                _ => {
                    for k in 2..flat.len() {
                        indices.push([flat[0], flat[k - 1], flat[k]]);
                    }
                }
            }
            // A mirroring transform flips the winding.
            let det = m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2])
                + m[8] * (m[1] * m[6] - m[5] * m[2]);
            if det < 0.0 {
                indices.iter_mut().for_each(|t| t.swap(1, 2));
            }
            let uvs = match p["attributes"]["TEXCOORD_0"].as_u64() {
                Some(ta) => Some(
                    self.floats(ta as usize, 2)?
                        .chunks(2)
                        .map(|c| [c[0] as f32, c[1] as f32])
                        .collect(),
                ),
                None => None,
            };
            let mut color = [1.0f32; 4];
            if let Some(mat) = p["material"]
                .as_u64()
                .and_then(|i| self.doc["materials"].get(i as usize))
            {
                let pbr = &mat["pbrMetallicRoughness"];
                let c = nums(&pbr["baseColorFactor"], [1.0; 4]);
                color = [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32];
                if model.texture.is_none() {
                    if let Some(ti) = pbr["baseColorTexture"]["index"].as_u64() {
                        model.texture = self.texture(ti as usize).ok();
                    }
                }
            }
            let name = if prims.len() > 1 {
                format!("{name}#{k}")
            } else {
                name.clone()
            };
            let mesh = Mesh {
                name,
                positions,
                indices,
                color,
                uvs,
            };
            mesh.validate()?;
            model.meshes.push(mesh);
        }
        Ok(())
    }

    fn view_bytes(&self, vi: usize) -> Result<(&[u8], Option<usize>), String> {
        let v = self.doc["bufferViews"]
            .get(vi)
            .ok_or_else(|| format!("bufferView {vi} missing"))?;
        let buf = self
            .buffers
            .get(v["buffer"].as_u64().unwrap_or(0) as usize)
            .ok_or("buffer missing")?;
        let o = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let l = v["byteLength"]
            .as_u64()
            .ok_or("bufferView without byteLength")? as usize;
        let bytes = buf.get(o..o + l).ok_or("bufferView outside buffer")?;
        Ok((bytes, v["byteStride"].as_u64().map(|s| s as usize)))
    }

    /// Accessor elements as f64 in groups of `n` components (float only,
    /// or normalized integers).
    fn floats(&self, ai: usize, n: usize) -> Result<Vec<f64>, String> {
        let AccessorView {
            bytes,
            stride,
            count,
            component: ct,
            normalized,
        } = self.accessor(ai)?;
        let size = comp_size(ct)?;
        let stride = stride.unwrap_or(size * n);
        let mut out = Vec::with_capacity(count * n);
        for e in 0..count {
            for c in 0..n {
                let o = e * stride + c * size;
                let s = bytes
                    .get(o..o + size)
                    .ok_or("accessor outside bufferView")?;
                let v = match ct {
                    5126 => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
                    5121 if normalized => s[0] as f64 / 255.0,
                    5123 if normalized => u16::from_le_bytes([s[0], s[1]]) as f64 / 65535.0,
                    _ => {
                        return Err(format!(
                            "accessor {ai}: component type {ct} not supported for floats"
                        ))
                    }
                };
                out.push(v);
            }
        }
        Ok(out)
    }

    fn ints(&self, ai: usize) -> Result<Vec<u32>, String> {
        let AccessorView {
            bytes,
            stride,
            count,
            component: ct,
            ..
        } = self.accessor(ai)?;
        let size = comp_size(ct)?;
        let stride = stride.unwrap_or(size);
        (0..count)
            .map(|e| {
                let s = bytes
                    .get(e * stride..e * stride + size)
                    .ok_or("index accessor outside bufferView")?;
                Ok(match ct {
                    5121 => s[0] as u32,
                    5123 => u16::from_le_bytes([s[0], s[1]]) as u32,
                    5125 => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    _ => return Err(format!("index component type {ct} not supported")),
                })
            })
            .collect()
    }

    fn accessor(&self, ai: usize) -> Result<AccessorView<'_>, String> {
        let a = self.doc["accessors"]
            .get(ai)
            .ok_or_else(|| format!("accessor {ai} missing"))?;
        if a.get("sparse").is_some() {
            return Err("sparse accessors are not supported".into());
        }
        let vi = a["bufferView"]
            .as_u64()
            .ok_or("accessor without bufferView")? as usize;
        let (bytes, stride) = self.view_bytes(vi)?;
        let o = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        Ok(AccessorView {
            bytes: bytes.get(o..).ok_or("accessor offset outside bufferView")?,
            stride,
            count: a["count"].as_u64().unwrap_or(0) as usize,
            component: a["componentType"].as_u64().unwrap_or(0),
            normalized: a["normalized"].as_bool().unwrap_or(false),
        })
    }

    fn texture(&self, ti: usize) -> Result<Texture, String> {
        let src = self.doc["textures"][ti]["source"]
            .as_u64()
            .ok_or("texture without source")? as usize;
        let img = &self.doc["images"][src];
        if let Some(vi) = img["bufferView"].as_u64() {
            let (b, _) = self.view_bytes(vi as usize)?;
            return Texture::from_bytes(b.to_vec());
        }
        if let Some(u) = img["uri"].as_str() {
            if let Some((_, d)) = u.split_once(";base64,") {
                return Texture::from_bytes(base64_decode(d)?);
            }
        }
        Err("external texture images are not supported".into())
    }
}

fn comp_size(ct: u64) -> Result<usize, String> {
    match ct {
        5120 | 5121 => Ok(1),
        5122 | 5123 => Ok(2),
        5125 | 5126 => Ok(4),
        _ => Err(format!("unknown component type {ct}")),
    }
}

/// Decodes standard base64 (RFC 4648 §4), padding optional.
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return Err("invalid base64".into()),
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------- OBJ

/// Writes OBJ and MTL text. Indices are global 1-based; each mesh is an `o`
/// with its own `usemtl`. Coordinates relative to the origin.
pub fn write_obj(model: &MeshModel, opt: &ObjOptions) -> Result<(String, String), String> {
    use std::fmt::Write;
    model.validate()?;
    let mut obj = String::from("# Open Infra rekenkern\n");
    let mut mtl = String::from("# Open Infra rekenkern\n");
    let o = model.origin;
    let _ = writeln!(obj, "# origin {} {} {}", o[0], o[1], o[2]);
    if let Some(c) = model.epsg {
        let _ = writeln!(obj, "# epsg {c}");
    }
    let _ = writeln!(obj, "# up {}", if opt.z_up { "Z" } else { "Y" });
    if let Some(f) = &opt.mtl_file {
        let _ = writeln!(obj, "mtllib {f}");
    }
    let (mut vbase, mut tbase) = (1usize, 1usize);
    for (k, m) in model.meshes.iter().enumerate() {
        let mat = format!("mat{k}_{}", sanitize(&m.name));
        let _ = writeln!(obj, "o {}", sanitize(&m.name));
        if opt.mtl_file.is_some() {
            let _ = writeln!(obj, "usemtl {mat}");
        }
        for &p in &m.positions {
            let d = sub(p, o);
            let d = if opt.z_up { d } else { [d[0], d[2], -d[1]] };
            let _ = writeln!(obj, "v {} {} {}", d[0], d[1], d[2]);
        }
        let uv = m.uvs.as_ref();
        for t in uv.into_iter().flatten() {
            let _ = writeln!(obj, "vt {} {}", t[0], 1.0 - t[1]);
        }
        for t in &m.indices {
            let [a, b, c] = t.map(|i| i as usize);
            if uv.is_some() {
                let _ = writeln!(
                    obj,
                    "f {}/{} {}/{} {}/{}",
                    a + vbase,
                    a + tbase,
                    b + vbase,
                    b + tbase,
                    c + vbase,
                    c + tbase
                );
            } else {
                let _ = writeln!(obj, "f {} {} {}", a + vbase, b + vbase, c + vbase);
            }
        }
        vbase += m.positions.len();
        tbase += uv.map_or(0, Vec::len);
        let c = m.color;
        let _ = writeln!(
            mtl,
            "newmtl {mat}\nKa 0 0 0\nKd {} {} {}\nKs 0 0 0\nd {}\nillum 1",
            c[0], c[1], c[2], c[3]
        );
        if let (Some(_), Some(_), Some(tf)) = (uv, &model.texture, &opt.texture_file) {
            let _ = writeln!(mtl, "map_Kd {tf}");
        }
    }
    Ok((obj, mtl))
}

/// A file name for the texture next to an OBJ (`terrain.png`).
pub fn texture_file_name(model: &MeshModel, stem: &str) -> Option<String> {
    model
        .texture
        .as_ref()
        .map(|t| format!("{stem}.{}", t.ext()))
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    if t.is_empty() {
        "mesh".into()
    } else {
        t
    }
}

/// Reads MTL text: material name → `Kd` + `d` (or `Tr`) as RGBA.
pub fn read_mtl(text: &str) -> HashMap<String, [f32; 4]> {
    let mut out = HashMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(kw) = it.next() else { continue };
        let vals: Vec<f32> = it.clone().filter_map(|x| x.parse().ok()).collect();
        match kw {
            "newmtl" => {
                let name = it.collect::<Vec<_>>().join(" ");
                out.insert(name.clone(), [0.8, 0.8, 0.8, 1.0]);
                cur = Some(name);
            }
            "Kd" if vals.len() >= 3 => {
                if let Some(c) = cur.as_ref().and_then(|n| out.get_mut(n)) {
                    c[..3].copy_from_slice(&vals[..3]);
                }
            }
            "d" | "Tr" if !vals.is_empty() => {
                if let Some(c) = cur.as_ref().and_then(|n| out.get_mut(n)) {
                    c[3] = if kw == "d" { vals[0] } else { 1.0 - vals[0] };
                }
            }
            _ => {}
        }
    }
    out
}

/// Reads OBJ text: `v`, `vt`, `f` (polygons fan-triangulated, negative
/// indices), `o`/`g` start a new mesh, `usemtl` starts one when the current
/// mesh already has faces in another material. Normals, lines, curves and
/// free-form surfaces are ignored. `mtl` gives material colours.
pub fn read_obj(text: &str, mtl: Option<&str>) -> Result<MeshModel, String> {
    let colors = mtl.map(read_mtl).unwrap_or_default();
    let mut model = MeshModel::default();
    let mut y_up = false;
    let mut v: Vec<P3> = Vec::new();
    let mut vt: Vec<[f32; 2]> = Vec::new();
    struct Cur {
        name: String,
        mat: Option<String>,
        map: HashMap<(usize, Option<usize>), u32>,
        mesh: Mesh,
        uv_used: bool,
    }
    let new_cur = |name: String, mat: Option<String>| Cur {
        mesh: Mesh::new(&name, Vec::new(), Vec::new(), [0.8, 0.8, 0.8, 1.0]),
        name,
        mat,
        map: HashMap::new(),
        uv_used: false,
    };
    let mut cur = new_cur("mesh".into(), None);
    let mut done: Vec<Mesh> = Vec::new();
    let finish = |c: Cur, done: &mut Vec<Mesh>, colors: &HashMap<String, [f32; 4]>| {
        if c.mesh.indices.is_empty() {
            return;
        }
        let mut m = c.mesh;
        m.name = c.name;
        if let Some(col) = c.mat.as_ref().and_then(|n| colors.get(n)) {
            m.color = *col;
        }
        if !c.uv_used {
            m.uvs = None;
        }
        done.push(m);
    };
    for (ln, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(c) = line.strip_prefix('#') {
            let mut it = c.split_whitespace();
            match it.next() {
                Some("origin") => {
                    let c: Vec<f64> = it.filter_map(|x| x.parse().ok()).collect();
                    if c.len() == 3 {
                        model.origin = [c[0], c[1], c[2]];
                    }
                }
                Some("epsg") => model.epsg = it.next().and_then(|x| x.parse().ok()),
                Some("up") => y_up = it.next() == Some("Y"),
                _ => {}
            }
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(kw) = it.next() else { continue };
        let bad = |what: &str| format!("OBJ line {}: {what}", ln + 1);
        match kw {
            "v" => {
                let c: Vec<f64> = it
                    .take(3)
                    .map(|x| x.parse().map_err(|_| bad("bad vertex")))
                    .collect::<Result<_, _>>()?;
                if c.len() < 3 {
                    return Err(bad("vertex needs x y z"));
                }
                v.push([c[0], c[1], c[2]]);
            }
            "vt" => {
                let c: Vec<f32> = it.take(2).filter_map(|x| x.parse().ok()).collect();
                vt.push([
                    c.first().copied().unwrap_or(0.0),
                    1.0 - c.get(1).copied().unwrap_or(0.0),
                ]);
            }
            "o" | "g" => {
                let name = it.collect::<Vec<_>>().join(" ");
                let mat = cur.mat.clone();
                let old = std::mem::replace(
                    &mut cur,
                    new_cur(if name.is_empty() { "mesh".into() } else { name }, mat),
                );
                finish(old, &mut done, &colors);
            }
            "usemtl" => {
                let mat = it.collect::<Vec<_>>().join(" ");
                if !cur.mesh.indices.is_empty() && cur.mat.as_deref() != Some(&mat) {
                    let name = cur.name.clone();
                    let old = std::mem::replace(&mut cur, new_cur(name, Some(mat)));
                    finish(old, &mut done, &colors);
                } else {
                    cur.mat = Some(mat);
                }
            }
            "f" => {
                let mut poly = Vec::new();
                for tok in it {
                    let mut parts = tok.split('/');
                    let resolve = |s: Option<&str>, n: usize| -> Result<Option<usize>, String> {
                        match s.filter(|s| !s.is_empty()) {
                            None => Ok(None),
                            Some(s) => {
                                let i: i64 = s.parse().map_err(|_| bad("bad face index"))?;
                                let r = if i < 0 { n as i64 + i } else { i - 1 };
                                if r < 0 || r as usize >= n {
                                    return Err(bad("face index out of range"));
                                }
                                Ok(Some(r as usize))
                            }
                        }
                    };
                    let vi = resolve(parts.next(), v.len())?
                        .ok_or_else(|| bad("face without vertex"))?;
                    let ti = resolve(parts.next(), vt.len())?;
                    let key = (vi, ti);
                    let idx = match cur.map.get(&key) {
                        Some(&i) => i,
                        None => {
                            let i = cur.mesh.positions.len() as u32;
                            cur.mesh.positions.push(v[vi]);
                            let uvs = cur.mesh.uvs.get_or_insert_with(Vec::new);
                            uvs.push(ti.map(|t| vt[t]).unwrap_or([0.0, 0.0]));
                            cur.uv_used |= ti.is_some();
                            cur.map.insert(key, i);
                            i
                        }
                    };
                    poly.push(idx);
                }
                for k in 2..poly.len() {
                    cur.mesh.indices.push([poly[0], poly[k - 1], poly[k]]);
                }
            }
            _ => {}
        }
    }
    finish(cur, &mut done, &colors);
    let o = model.origin;
    for m in &mut done {
        for p in &mut m.positions {
            let q = if y_up { [p[0], -p[2], p[1]] } else { *p };
            *p = [q[0] + o[0], q[1] + o[1], q[2] + o[2]];
        }
    }
    model.meshes = done;
    Ok(model)
}

// ---------------------------------------------------------------------- STL

fn facet_normal(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let a = [t[0][0] as f64, t[0][1] as f64, t[0][2] as f64];
    let b = [t[1][0] as f64, t[1][1] as f64, t[1][2] as f64];
    let c = [t[2][0] as f64, t[2][1] as f64, t[2][2] as f64];
    let n = cross(sub(b, a), sub(c, a));
    let l = norm(n);
    if l == 0.0 {
        [0.0; 3]
    } else {
        [(n[0] / l) as f32, (n[1] / l) as f32, (n[2] / l) as f32]
    }
}

fn local_tris(model: &MeshModel, m: &Mesh) -> Vec<[[f32; 3]; 3]> {
    m.indices
        .iter()
        .map(|t| t.map(|i| model.local(m.positions[i as usize])))
        .collect()
}

/// Writes binary STL of all meshes (one solid; colours are lost). The
/// header holds `origin=x,y,z epsg=n`.
pub fn write_stl_binary(model: &MeshModel) -> Result<Vec<u8>, String> {
    model.validate()?;
    let n = model.triangle_count();
    let mut out = Vec::with_capacity(84 + 50 * n);
    let mut header = format!("OpenInfra {}", model.header_text()).into_bytes();
    header.resize(80, 0);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(n as u32).to_le_bytes());
    for m in &model.meshes {
        for t in local_tris(model, m) {
            for v in std::iter::once(facet_normal(&t)).chain(t) {
                v.iter()
                    .for_each(|c| out.extend_from_slice(&c.to_le_bytes()));
            }
            out.extend_from_slice(&[0, 0]);
        }
    }
    Ok(out)
}

/// Writes ASCII STL, one `solid` per mesh.
pub fn write_stl_ascii(model: &MeshModel) -> Result<String, String> {
    use std::fmt::Write;
    model.validate()?;
    let mut s = String::new();
    for m in &model.meshes {
        let name = sanitize(&m.name);
        let _ = writeln!(s, "solid {name} {}", model.header_text());
        for t in local_tris(model, m) {
            let n = facet_normal(&t);
            let _ = writeln!(
                s,
                "  facet normal {:e} {:e} {:e}\n    outer loop",
                n[0], n[1], n[2]
            );
            for v in t {
                let _ = writeln!(s, "      vertex {:e} {:e} {:e}", v[0], v[1], v[2]);
            }
            let _ = writeln!(s, "    endloop\n  endfacet");
        }
        let _ = writeln!(s, "endsolid {name}");
    }
    Ok(s)
}

/// Welds identical vertices so triangle soup becomes an indexed mesh.
struct Welder {
    map: HashMap<[u32; 3], u32>,
    mesh: Mesh,
}

impl Welder {
    fn new(name: &str) -> Welder {
        Welder {
            map: HashMap::new(),
            mesh: Mesh::new(name, Vec::new(), Vec::new(), [0.8, 0.8, 0.8, 1.0]),
        }
    }
    fn vertex(&mut self, v: [f32; 3], origin: P3) -> u32 {
        let key = v.map(|c| if c == 0.0 { 0 } else { c.to_bits() });
        let n = self.mesh.positions.len() as u32;
        *self.map.entry(key).or_insert_with(|| {
            self.mesh.positions.push([
                v[0] as f64 + origin[0],
                v[1] as f64 + origin[1],
                v[2] as f64 + origin[2],
            ]);
            n
        })
    }
}

/// Reads STL, binary or ASCII (detected by the binary size rule and the
/// `solid` keyword). Vertices are welded; normals are ignored.
pub fn read_stl(bytes: &[u8]) -> Result<MeshModel, String> {
    let is_binary = bytes.len() >= 84 && {
        let n = u32_at(bytes, 80)? as usize;
        84 + 50 * n == bytes.len()
    };
    let mut model = MeshModel::default();
    if is_binary {
        let header = String::from_utf8_lossy(&bytes[..80])
            .trim_end_matches('\0')
            .to_string();
        model.apply_header_text(&header);
        let n = u32_at(bytes, 80)? as usize;
        let mut w = Welder::new("stl");
        for k in 0..n {
            let o = 84 + 50 * k + 12;
            let mut tri = [0u32; 3];
            for (j, t) in tri.iter_mut().enumerate() {
                let f = |c: usize| {
                    f32::from_le_bytes(
                        bytes[o + 12 * j + 4 * c..o + 12 * j + 4 * c + 4]
                            .try_into()
                            .unwrap_or([0; 4]),
                    )
                };
                *t = w.vertex([f(0), f(1), f(2)], model.origin);
            }
            w.mesh.indices.push(tri);
        }
        model.meshes.push(w.mesh);
        return Ok(model);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "STL is neither valid binary nor ASCII")?;
    if !text.trim_start().starts_with("solid") {
        return Err("STL is neither valid binary nor ASCII".into());
    }
    let mut cur: Option<Welder> = None;
    let mut tri: Vec<u32> = Vec::new();
    for (ln, line) in text.lines().enumerate() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("solid") => {
                let rest: Vec<&str> = it.collect();
                model.apply_header_text(&rest.join(" "));
                let name = rest
                    .iter()
                    .find(|w| !w.contains('='))
                    .copied()
                    .unwrap_or("stl");
                cur = Some(Welder::new(name));
            }
            Some("vertex") => {
                let c: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                if c.len() != 3 {
                    return Err(format!("STL line {}: bad vertex", ln + 1));
                }
                let w = cur.as_mut().ok_or("vertex outside solid")?;
                tri.push(w.vertex([c[0], c[1], c[2]], model.origin));
            }
            Some("endloop") => {
                let w = cur.as_mut().ok_or("endloop outside solid")?;
                for k in 2..tri.len() {
                    w.mesh.indices.push([tri[0], tri[k - 1], tri[k]]);
                }
                tri.clear();
            }
            Some("endsolid") => {
                if let Some(w) = cur.take() {
                    model.meshes.push(w.mesh);
                }
            }
            _ => {}
        }
    }
    if let Some(w) = cur.take() {
        model.meshes.push(w.mesh);
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit cube [0,1]³ shifted to `at`, outward CCW faces.
    fn cube(at: P3) -> Mesh {
        let p: Vec<P3> = (0..8)
            .map(|i| {
                [
                    at[0] + (i & 1) as f64,
                    at[1] + ((i >> 1) & 1) as f64,
                    at[2] + ((i >> 2) & 1) as f64,
                ]
            })
            .collect();
        let q = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let idx = q
            .iter()
            .flat_map(|f| [[f[0], f[1], f[2]], [f[0], f[2], f[3]]])
            .collect();
        Mesh::new("kubus", p, idx, [0.5, 0.4, 0.3, 1.0])
    }

    fn rd_model() -> MeshModel {
        let mut m = MeshModel::new([155000.0, 463000.0, 0.0], Some(28992));
        m.meshes.push(cube([155010.25, 463020.5, 1.125]));
        let mut terrain = Mesh::new(
            "maaiveld",
            vec![
                [154990.0, 462990.0, 0.5],
                [155090.0, 462990.0, 0.7],
                [155090.0, 463090.0, 1.1],
                [154990.0, 463090.0, 0.9],
            ],
            vec![[0, 1, 2], [0, 2, 3]],
            [1.0, 1.0, 1.0, 1.0],
        );
        terrain.uvs = Some(vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
        m.meshes.push(terrain);
        m
    }

    fn close(a: &MeshModel, b: &MeshModel, tol: f64) {
        assert_eq!(a.meshes.len(), b.meshes.len());
        for (x, y) in a.meshes.iter().zip(&b.meshes) {
            assert_eq!(x.triangle_count(), y.triangle_count());
            for (t, u) in x.indices.iter().zip(&y.indices) {
                for k in 0..3 {
                    let (p, q) = (x.positions[t[k] as usize], y.positions[u[k] as usize]);
                    assert!(norm(sub(p, q)) < tol, "{p:?} vs {q:?}");
                }
            }
        }
    }

    #[test]
    fn unit_cube_volume_area_bbox() {
        let c = cube([155000.0, 463000.0, 3.0]);
        assert!((c.volume() - 1.0).abs() < 1e-9);
        assert!((c.area() - 6.0).abs() < 1e-9);
        assert_eq!(
            c.bbox(),
            Some(([155000.0, 463000.0, 3.0], [155001.0, 463001.0, 4.0]))
        );
        assert_eq!(c.triangle_count(), 12);
    }

    #[test]
    fn glb_round_trip() {
        let mut m = rd_model();
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR-fake".to_vec();
        m.texture = Some(Texture::from_bytes(png.clone()).unwrap());
        let glb = write_glb(&m).unwrap();
        let r = read_gltf(&glb).unwrap();
        assert_eq!(r.origin, m.origin);
        assert_eq!(r.epsg, Some(28992));
        assert_eq!(r.triangle_count(), 14);
        close(&m, &r, 1e-4);
        assert_eq!(r.meshes[0].color, [0.5, 0.4, 0.3, 1.0]);
        assert!(
            (r.meshes[0].volume() - 1.0).abs() < 1e-5,
            "Y-up conversion keeps orientation"
        );
        assert_eq!(r.meshes[1].uvs.as_ref().unwrap()[2], [1.0, 0.0]);
        assert_eq!(r.texture.unwrap().data, png);
    }

    #[test]
    fn glb_container_is_valid() {
        let mut m = rd_model();
        m.texture = Some(Texture::from_bytes(vec![0xFF, 0xD8, 0xFF, 0xE0, 1]).unwrap()); // 5 bytes: BIN needs padding
        let g = write_glb(&m).unwrap();
        let rd = |o: usize| u32::from_le_bytes(g[o..o + 4].try_into().unwrap()) as usize;
        assert_eq!(&g[0..4], b"glTF");
        assert_eq!(rd(4), 2);
        assert_eq!(rd(8), g.len());
        let jl = rd(12);
        assert_eq!(rd(16), CHUNK_JSON as usize);
        assert_eq!(jl % 4, 0);
        let js = &g[20..20 + jl];
        let trimmed = js.iter().rposition(|&c| c != b' ').unwrap() + 1;
        assert_eq!(js[trimmed - 1], b'}', "JSON padded with spaces only");
        let doc: Json = serde_json::from_slice(js).unwrap();
        let bo = 20 + jl;
        let bl = rd(bo);
        assert_eq!(rd(bo + 4), CHUNK_BIN as usize);
        assert_eq!(bl % 4, 0);
        assert_eq!(bo + 8 + bl, g.len());
        assert_eq!(
            doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
            bl
        );
        let img_view =
            &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().unwrap() as usize];
        let end = (img_view["byteOffset"].as_u64().unwrap()
            + img_view["byteLength"].as_u64().unwrap()) as usize;
        assert!(
            g[bo + 8 + end..].iter().all(|&b| b == 0),
            "BIN padded with zeros"
        );
        for v in doc["bufferViews"].as_array().unwrap() {
            assert_eq!(v["byteOffset"].as_u64().unwrap() % 4, 0);
        }
        let pos = &doc["accessors"][0];
        assert_eq!(pos["min"].as_array().unwrap().len(), 3);
        // Y-up: model z 1.125..2.125 becomes glTF y.
        assert_eq!(pos["min"][1].as_f64().unwrap(), 1.125);
        assert_eq!(doc["asset"]["extras"]["epsg"], 28992);
    }

    #[test]
    fn gltf_node_transforms() {
        // Hand-written glTF: one triangle, parent translation + child matrix (scale 2).
        let tri: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let b64 = {
            const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut s = String::new();
            for c in tri.chunks(3) {
                let n = (c[0] as u32) << 16
                    | (*c.get(1).unwrap_or(&0) as u32) << 8
                    | *c.get(2).unwrap_or(&0) as u32;
                for k in 0..=c.len() {
                    s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
                }
            }
            s
        };
        let doc = json!({
            "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
            "nodes": [{"translation": [10, 0, 0], "children": [1]},
                      {"matrix": [2,0,0,0, 0,2,0,0, 0,0,2,0, 0,0,0,1], "mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}],
            "bufferViews": [{"buffer": 0, "byteLength": 36}],
            "buffers": [{"byteLength": 36, "uri": format!("data:application/octet-stream;base64,{b64}")}]
        });
        let r = read_gltf(doc.to_string().as_bytes()).unwrap();
        let p = &r.meshes[0].positions;
        // glTF (12,0,0) → model (12,0,0); glTF (10,2,0) → model (10,0,2).
        assert_eq!(p[1], [12.0, 0.0, 0.0]);
        assert_eq!(p[2], [10.0, -0.0, 2.0]);
    }

    #[test]
    fn obj_round_trip() {
        let mut m = rd_model();
        m.texture = Some(Texture::from_bytes(b"\x89PNG\r\n\x1a\nx".to_vec()).unwrap());
        for z_up in [true, false] {
            let opt = ObjOptions {
                z_up,
                mtl_file: Some("m.mtl".into()),
                texture_file: texture_file_name(&m, "luchtfoto"),
            };
            let (obj, mtl) = write_obj(&m, &opt).unwrap();
            assert!(mtl.contains("map_Kd luchtfoto.png"));
            let r = read_obj(&obj, Some(&mtl)).unwrap();
            assert_eq!(r.origin, m.origin);
            assert_eq!(r.epsg, Some(28992));
            close(&m, &r, 1e-9);
            assert_eq!(r.meshes[0].color, [0.5, 0.4, 0.3, 1.0]);
            assert!(r.meshes[0].uvs.is_none());
            let uv = r.meshes[1].uvs.as_ref().unwrap();
            assert!((uv[0][1] - 1.0).abs() < 1e-6);
            assert!((r.volume() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn obj_polygons_negative_indices_groups() {
        let t = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvn 0 0 1\ng vloer\nf -4//1 -3//1 -2//1 -1//1\no wand\nv 0 0 1\nf 1 2 5\n";
        let r = read_obj(t, None).unwrap();
        assert_eq!(r.meshes.len(), 2);
        assert_eq!(r.meshes[0].name, "vloer");
        assert_eq!(r.meshes[0].indices, vec![[0, 1, 2], [0, 2, 3]]);
        assert!((r.meshes[0].area() - 1.0).abs() < 1e-12);
        assert_eq!(r.meshes[1].positions.len(), 3);
        assert!(read_obj("f 1 2 3\n", None).is_err());
    }

    #[test]
    fn stl_round_trips() {
        let m = rd_model();
        let b = write_stl_binary(&m).unwrap();
        assert_eq!(b.len(), 84 + 50 * 14);
        let r = read_stl(&b).unwrap();
        assert_eq!(r.origin, m.origin);
        assert_eq!(r.epsg, Some(28992));
        assert_eq!(r.triangle_count(), 14);
        assert_eq!(
            r.meshes[0].positions.len(),
            12,
            "welded: 8 cube + 4 terrain"
        );
        assert!((r.area() - m.area()).abs() < 1e-2);
        let (lo, hi) = r.bbox().unwrap();
        assert!((lo[0] - 154990.0).abs() < 1e-4 && (hi[2] - 2.125).abs() < 1e-4);

        let a = write_stl_ascii(&m).unwrap();
        let r = read_stl(a.as_bytes()).unwrap();
        assert_eq!(r.origin, m.origin);
        assert_eq!(r.meshes.len(), 2);
        assert_eq!(r.meshes[0].name, "kubus");
        close(&m, &r, 1e-4);
        assert!((r.meshes[0].volume() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert_eq!(base64_decode("TQ==").unwrap(), b"M");
    }
}
