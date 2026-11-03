//! AHN point clouds: the COPC tiles (1 × 1 km, Cloud Optimized Point Cloud)
//! on Het Waterschapshuis' object storage, read over HTTP range requests so
//! only the octree nodes in the import area are downloaded. Written out as
//! an indexed scan (.rcs) that Open CAD Studio attaches with
//! POINTCLOUDATTACH.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use copc_rs::{BoundsSelection, CopcReader, LodSelection, Vector};

use super::http;

const BASE: &str = "https://fsn1.your-objectstorage.com/hwh-ahn";

/// AHN edition of the point cloud.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Edition {
    Ahn4,
    #[default]
    Ahn5,
}

impl Edition {
    pub const ALL: [Edition; 2] = [Edition::Ahn5, Edition::Ahn4];
    pub fn label(self) -> &'static str {
        match self {
            Edition::Ahn4 => "AHN4 (2020–2022)",
            Edition::Ahn5 => "AHN5 (2023–2025)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Edition::Ahn4 => "AHN4",
            Edition::Ahn5 => "AHN5",
        }
    }
    /// The COPC tile whose lower-left corner is (x, y) in RD (km grid).
    pub fn tile_url(self, x: i64, y: i64) -> String {
        let v = self.short();
        format!("{BASE}/{v}_KM/01_LAZ/{v}_C_{x:06}_{y:06}.COPC.LAZ")
    }
}

/// AHN point classes (ASPRS codes as AHN uses them).
pub const CLASSES: &[(u8, &str, [u8; 3])] = &[
    (2, "Maaiveld", [196, 160, 100]),
    (6, "Gebouwen", [214, 86, 64]),
    (9, "Water", [64, 128, 224]),
    (26, "Kunstwerken", [170, 90, 200]),
    (1, "Overig (o.a. begroeiing)", [86, 170, 72]),
    (14, "Hoogspanning", [240, 200, 40]),
];

/// How points are coloured in the scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Colouring {
    #[default]
    Classification,
    Height,
    /// The aerial-photo RGB the edition carries, else by class.
    Rgb,
}

impl Colouring {
    pub const ALL: [Colouring; 3] = [Colouring::Classification, Colouring::Height, Colouring::Rgb];
    pub fn label(self) -> &'static str {
        match self {
            Colouring::Classification => "Klasse",
            Colouring::Height => "Hoogte",
            Colouring::Rgb => "RGB (luchtfoto)",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub edition: Edition,
    /// Point spacing to aim for (m): the COPC level of detail.
    pub spacing: f64,
    /// ASPRS classes to keep (empty: all).
    pub classes: Vec<u8>,
    pub max_points: usize,
    pub colouring: Colouring,
}

impl Default for Options {
    fn default() -> Self {
        Self { edition: Edition::default(), spacing: 0.25, classes: Vec::new(), max_points: 8_000_000, colouring: Colouring::default() }
    }
}

pub struct Cloud {
    /// Tiles taken from the older edition because the chosen one lacks them.
    pub fallback_tiles: usize,
    /// RD + NAP.
    pub xyz: Vec<[f64; 3]>,
    pub class: Vec<u8>,
    pub intensity: Vec<u16>,
    pub rgb: Option<Vec<[u8; 3]>>,
    /// The point limit cut the result short.
    pub truncated: bool,
    pub tiles: usize,
}

/// A remote file as Read + Seek through cached HTTP range requests.
struct RangeReader {
    url: String,
    len: u64,
    pos: u64,
    blocks: HashMap<u64, Arc<Vec<u8>>>,
    order: Vec<u64>,
    fetched: Arc<std::sync::atomic::AtomicU64>,
}

const BLOCK: u64 = 1 << 20;
const KEEP_BLOCKS: usize = 48;

impl RangeReader {
    fn open(url: &str, fetched: Arc<std::sync::atomic::AtomicU64>) -> Result<Self, String> {
        let len = http::content_length(url)?;
        Ok(Self { url: url.to_string(), len, pos: 0, blocks: HashMap::new(), order: Vec::new(), fetched })
    }

    fn block(&mut self, index: u64) -> std::io::Result<Arc<Vec<u8>>> {
        if let Some(b) = self.blocks.get(&index) {
            return Ok(b.clone());
        }
        let start = index * BLOCK;
        let end = (start + BLOCK).min(self.len) - 1;
        let bytes = http::get_range(&self.url, start, end).map_err(std::io::Error::other)?;
        self.fetched.fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        let b = Arc::new(bytes);
        self.blocks.insert(index, b.clone());
        self.order.push(index);
        if self.order.len() > KEEP_BLOCKS {
            let old = self.order.remove(0);
            self.blocks.remove(&old);
        }
        Ok(b)
    }
}

impl Read for RangeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let index = self.pos / BLOCK;
        let block = self.block(index)?;
        let offset = (self.pos - index * BLOCK) as usize;
        let n = buf.len().min(block.len().saturating_sub(offset));
        buf[..n].copy_from_slice(&block[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for RangeReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let next = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.len as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if next < 0 {
            return Err(std::io::Error::other("seek voor het begin"));
        }
        self.pos = next as u64;
        Ok(self.pos)
    }
}

/// The 1 km tiles (lower-left corners) an RD box touches.
pub fn tiles_for(bbox: [f64; 4]) -> Vec<(i64, i64)> {
    let k = |v: f64| (v / 1000.0).floor() as i64 * 1000;
    let mut out = Vec::new();
    let mut y = k(bbox[1]);
    while (y as f64) < bbox[3] {
        let mut x = k(bbox[0]);
        while (x as f64) < bbox[2] {
            out.push((x, y));
            x += 1000;
        }
        y += 1000;
    }
    out
}

/// One point of a cloud, in the coordinates of its source (RD + NAP for
/// AHN, the file's own CRS for files).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub xyz: [f64; 3],
    /// ASPRS class; `None` when the source has no classification.
    pub class: Option<u8>,
    pub rgb: Option<[u8; 3]>,
    pub intensity: u16,
}

/// What one pass over a point source saw.
#[derive(Clone, Debug, Default)]
pub struct Pass {
    /// Points read from the source (before the caller's filters).
    pub read: u64,
    /// AHN tiles read, and those taken from AHN4 because AHN5 lacks them.
    pub tiles: usize,
    pub fallback_tiles: usize,
    /// Bytes downloaded (AHN).
    pub bytes: u64,
    /// The visitor stopped the pass.
    pub stopped: bool,
}

/// Stream the AHN points inside an RD box, tile by tile, at a COPC level of
/// detail of `spacing` m (0 = full resolution). Only the octree nodes that
/// touch the box are downloaded. `visit` returns false to stop.
pub fn stream_ahn(bbox: [f64; 4], edition: Edition, spacing: f64, max_tiles: usize, progress: &dyn Fn(String), visit: &mut dyn FnMut(&Sample) -> bool) -> Result<Pass, String> {
    let tiles = tiles_for(bbox);
    if tiles.is_empty() {
        return Err(crate::lang::tr("leeg gebied", "empty area").into());
    }
    if tiles.len() > max_tiles {
        return Err(format!("{} {} {}", crate::lang::tr("het gebied raakt", "the area touches"), tiles.len(), crate::lang::tr("AHN-tegels van 1 km²; kies een kleiner gebied", "AHN tiles of 1 km²; choose a smaller area")));
    }
    let fetched = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut pass = Pass { tiles: tiles.len(), ..Pass::default() };
    let lod = || if spacing > 0.0 { LodSelection::Resolution(spacing.max(0.05)) } else { LodSelection::All };
    for (k, (tx, ty)) in tiles.iter().enumerate() {
        progress(format!("{} tegel {}/{} ({}_{})…", edition.short(), k + 1, tiles.len(), tx, ty));
        // AHN5 is not complete yet: a tile it lacks comes from AHN4.
        let mut editions = vec![edition];
        if edition == Edition::Ahn5 {
            editions.push(Edition::Ahn4);
        }
        let mut reader = None;
        for (i, ed) in editions.iter().enumerate() {
            match RangeReader::open(&ed.tile_url(*tx, *ty), fetched.clone()) {
                Ok(r) => {
                    if i > 0 {
                        pass.fallback_tiles += 1;
                    }
                    reader = Some(r);
                    break;
                }
                // A tile at sea or outside the edition simply isn't there.
                Err(e) if e.contains("404") || e.contains("403") => continue,
                Err(e) => return Err(e),
            }
        }
        let Some(reader) = reader else { continue };
        let mut copc = CopcReader::new(reader).map_err(|e| format!("{}: {e}", edition.short()))?;
        let bounds = copc_rs::Bounds { min: Vector { x: bbox[0], y: bbox[1], z: -200.0 }, max: Vector { x: bbox[2], y: bbox[3], z: 2000.0 } };
        let points = copc.points(lod(), BoundsSelection::Within(bounds)).map_err(|e| format!("{}: {e}", edition.short()))?;
        // The COPC reader panics on a broken read; keep that inside this tile.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for p in points {
                let p = p.map_err(|e| e.to_string())?;
                pass.read += 1;
                let s = Sample { xyz: [p.x, p.y, p.z], class: Some(u8::from(p.classification)), rgb: p.color.map(|c| [(c.red >> 8) as u8, (c.green >> 8) as u8, (c.blue >> 8) as u8]), intensity: p.intensity };
                if !visit(&s) {
                    pass.stopped = true;
                    break;
                }
                if pass.read.is_multiple_of(250_000) {
                    progress(format!(
                        "{} tegel {}/{}: {} punten, {:.1} MB opgehaald…",
                        edition.short(),
                        k + 1,
                        tiles.len(),
                        pass.read,
                        fetched.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1e6
                    ));
                }
            }
            Ok::<(), String>(())
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("{}: {}: {e}", edition.short(), crate::lang::tr("de puntenwolk kon niet worden gelezen", "the point cloud could not be read"))),
            Err(_) => return Err(format!("{}: {}", edition.short(), crate::lang::tr("de puntenwolk kon niet worden gelezen (netwerk?)", "the point cloud could not be read (network?)"))),
        }
        if pass.stopped {
            break;
        }
    }
    pass.bytes = fetched.load(std::sync::atomic::Ordering::Relaxed);
    Ok(pass)
}

/// Points of the AHN point cloud inside an RD box.
pub fn fetch(bbox: [f64; 4], opt: &Options, progress: &dyn Fn(String)) -> Result<Cloud, String> {
    let mut cloud = Cloud { fallback_tiles: 0, xyz: Vec::new(), class: Vec::new(), intensity: Vec::new(), rgb: None, truncated: false, tiles: 0 };
    let mut rgb: Vec<[u8; 3]> = Vec::new();
    let mut any_rgb = false;
    let keep = |c: u8| opt.classes.is_empty() || opt.classes.contains(&c);
    let pass = stream_ahn(bbox, opt.edition, opt.spacing.max(0.05), 16, progress, &mut |s| {
        let class = s.class.unwrap_or(0);
        if !keep(class) {
            return true;
        }
        if cloud.xyz.len() >= opt.max_points {
            cloud.truncated = true;
            return false;
        }
        cloud.xyz.push(s.xyz);
        cloud.class.push(class);
        cloud.intensity.push(s.intensity);
        match s.rgb {
            Some(c) => {
                any_rgb = true;
                rgb.push(c);
            }
            None => rgb.push([0, 0, 0]),
        }
        true
    })?;
    cloud.tiles = pass.tiles;
    cloud.fallback_tiles = pass.fallback_tiles;
    if any_rgb {
        cloud.rgb = Some(rgb);
    }
    if cloud.xyz.is_empty() {
        return Err(format!("{} heeft hier geen punten (buiten Nederland, of andere klassen kiezen)", opt.edition.short()));
    }
    Ok(cloud)
}

// ── Point cloud files ───────────────────────────────────────────────────────

/// Point cloud files Open Infra reads (lower case, without the dot); a
/// `.txt` counts when it holds bare coordinates ([`txt_is_points`]).
pub const FILE_EXTENSIONS: &[&str] = &["las", "laz", "e57", "ply", "pts", "xyz"];

/// Whether a path is a point cloud file Open Infra reads.
pub fn is_point_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    FILE_EXTENSIONS.contains(&ext.as_str()) || (ext == "txt" && txt_is_points(path))
}

/// A text file of bare coordinates ("x y z [i] [r g b]", no header line):
/// a point cloud, where a CSV with a header row is vector data.
pub fn txt_is_points(path: &Path) -> bool {
    let Ok(f) = std::fs::File::open(path) else { return false };
    let mut head = String::new();
    let _ = f.take(4096).read_to_string(&mut head);
    let mut lines = head.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("//"));
    let Some(first) = lines.next() else { return false };
    // PTS: a point count on its own line, then the points.
    let first = if first.parse::<u64>().is_ok() { lines.next().unwrap_or("") } else { first };
    let fields: Vec<&str> = first.split(|c: char| c.is_ascii_whitespace() || c == ',' || c == ';').filter(|f| !f.is_empty()).collect();
    fields.len() >= 3 && fields.iter().all(|f| f.parse::<f64>().is_ok())
}

/// Whether a LAS/LAZ file is a COPC file (an octree: reading a box costs
/// only the nodes in it).
pub fn is_copc(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    (ext == "las" || ext == "laz") && CopcReader::from_path(path).is_ok()
}

/// The CRS a point cloud file states: the LAS GeoKeys or WKT, or the E57
/// coordinate metadata. `None` for formats without one (PLY, PTS, XYZ):
/// the caller falls back to a `.prj` beside the file, then the project.
pub fn file_crs(path: &Path) -> Option<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "las" | "laz" => {
            let reader = las::Reader::from_path(path).ok()?;
            las_crs::parse_las_crs(reader.header()).ok().map(|c| format!("EPSG:{}", c.horizontal))
        }
        "e57" => {
            let reader = e57::E57Reader::from_file(path).ok()?;
            let meta = reader.coordinate_metadata()?.trim().to_string();
            if meta.is_empty() {
                return None;
            }
            super::files::epsg_from_wkt(&meta).or_else(|| {
                let up = meta.to_ascii_uppercase();
                let digits: String = up.split("EPSG").nth(1)?.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
                (!digits.is_empty()).then(|| format!("EPSG:{digits}"))
            })
        }
        _ => None,
    }
}

/// Stream a local point cloud file. LAS/LAZ through `las` (a COPC file
/// through `copc-rs`, reading only the nodes in `bbox`, file CRS, at a level
/// of detail of `spacing` m); E57, PLY, PTS and XYZ through the readers of
/// OpenAEC's Open Pointcloud Studio. Other formats are filtered by the
/// caller. `visit` returns false to stop.
pub fn stream_file(path: &Path, bbox: Option<[f64; 4]>, spacing: f64, progress: &dyn Fn(String), visit: &mut dyn FnMut(&Sample) -> bool) -> Result<Pass, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut pass = Pass::default();
    if ext == "las" || ext == "laz" {
        // A COPC file: only the octree nodes in the box.
        if let Ok(mut copc) = CopcReader::from_path(path) {
            let lod = if spacing > 0.0 { LodSelection::Resolution(spacing) } else { LodSelection::All };
            let sel = match bbox {
                Some(b) => BoundsSelection::Within(copc_rs::Bounds { min: Vector { x: b[0], y: b[1], z: -1.0e6 }, max: Vector { x: b[2], y: b[3], z: 1.0e6 } }),
                None => BoundsSelection::All,
            };
            let points = copc.points(lod, sel).map_err(|e| format!("{name}: {e}"))?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for p in points {
                    let p = p.map_err(|e| e.to_string())?;
                    pass.read += 1;
                    let s = Sample { xyz: [p.x, p.y, p.z], class: Some(u8::from(p.classification)), rgb: p.color.map(|c| [(c.red >> 8) as u8, (c.green >> 8) as u8, (c.blue >> 8) as u8]), intensity: p.intensity };
                    if !visit(&s) {
                        pass.stopped = true;
                        break;
                    }
                    if pass.read.is_multiple_of(1_000_000) {
                        progress(format!("{name}: {}…", pass.read));
                    }
                }
                Ok::<(), String>(())
            }));
            return match result {
                Ok(Ok(())) => Ok(pass),
                Ok(Err(e)) => Err(format!("{name}: {e}")),
                Err(_) => Err(format!("{name}: {}", crate::lang::tr("de COPC-puntenwolk is onleesbaar", "the COPC point cloud is unreadable"))),
            };
        }
        let mut reader = las::Reader::from_path(path).map_err(|e| format!("{name}: {e}"))?;
        let total = reader.header().number_of_points();
        for p in reader.points() {
            let Ok(p) = p else { continue };
            pass.read += 1;
            let s = Sample { xyz: [p.x, p.y, p.z], class: Some(u8::from(p.classification)), rgb: p.color.map(|c| [(c.red >> 8) as u8, (c.green >> 8) as u8, (c.blue >> 8) as u8]), intensity: p.intensity };
            if !visit(&s) {
                pass.stopped = true;
                break;
            }
            if pass.read.is_multiple_of(1_000_000) {
                progress(format!("{name}: {} / {total}…", pass.read));
            }
        }
        return Ok(pass);
    }
    let known = ["e57", "ply", "pts", "xyz", "txt", "asc", "csv", "ptx", "pcd"];
    if !known.contains(&ext.as_str()) {
        return Err(format!("{name}: {}", crate::lang::tr("geen puntenwolkformaat (LAS, LAZ, E57, PLY, PTS, XYZ)", "not a point cloud format (LAS, LAZ, E57, PLY, PTS, XYZ)")));
    }
    let mut stop = false;
    let r = pointcloud_core::visit_points(path, &mut |p: pointcloud_core::Point| {
        pass.read += 1;
        let s = Sample { xyz: p.xyz, class: p.classification, rgb: p.rgb, intensity: p.intensity.unwrap_or(0) };
        if !visit(&s) {
            stop = true;
            return Err(pointcloud_core::LoadError::Cancelled);
        }
        if pass.read.is_multiple_of(1_000_000) {
            progress(format!("{name}: {}…", pass.read));
        }
        Ok(())
    });
    match r {
        Ok(()) => {}
        Err(_) if stop => pass.stopped = true,
        Err(e) => return Err(format!("{name}: {e}")),
    }
    Ok(pass)
}

/// Point colours for the chosen scheme.
pub fn colours(cloud: &Cloud, scheme: Colouring) -> Vec<[u8; 3]> {
    let by_class = |c: u8| CLASSES.iter().find(|k| k.0 == c).map(|k| k.2).unwrap_or([160, 160, 160]);
    match scheme {
        Colouring::Rgb if cloud.rgb.is_some() => cloud.rgb.clone().unwrap_or_default(),
        Colouring::Height => {
            // Ground-level range: 2nd to 98th percentile, so a church tower
            // does not wash out the street.
            let mut z: Vec<f64> = cloud.xyz.iter().map(|p| p[2]).collect();
            z.sort_by(f64::total_cmp);
            let lo = z[z.len() * 2 / 100];
            let hi = z[(z.len() * 98 / 100).min(z.len() - 1)].max(lo + 0.1);
            cloud.xyz.iter().map(|p| ramp(((p[2] - lo) / (hi - lo)).clamp(0.0, 1.0))).collect()
        }
        _ => cloud.class.iter().map(|c| by_class(*c)).collect(),
    }
}

/// Blue – cyan – green – yellow – red.
fn ramp(t: f64) -> [u8; 3] {
    let stops = [[40.0, 70.0, 200.0], [40.0, 190.0, 220.0], [70.0, 190.0, 80.0], [240.0, 220.0, 60.0], [220.0, 60.0, 50.0]];
    let f = t * (stops.len() - 1) as f64;
    let i = (f.floor() as usize).min(stops.len() - 2);
    let u = f - i as f64;
    let mix = |k: usize| (stops[i][k] + (stops[i + 1][k] - stops[i][k]) * u).round() as u8;
    [mix(0), mix(1), mix(2)]
}

// ── Indexed scan (.rcs) writer ──────────────────────────────────────────────

/// Node cube edge (m): the point records hold 18-bit millimetre offsets,
/// so a node spans at most 262 m.
const NODE_EDGE: f64 = 200.0;
const NODE_RECORD: usize = 368;
const HEADER: usize = 0x200;

/// Write points (drawing coordinates, already scaled) as an indexed scan
/// ("ADOCT" version 3), the layout Open CAD Studio's point cloud reader
/// takes: one node per 200 m cube, every record carrying its own position.
pub fn write_rcs(path: &Path, xyz: &[[f64; 3]], colours: &[[u8; 3]], intensity: &[u16]) -> Result<(), String> {
    if xyz.is_empty() {
        return Err("geen punten".into());
    }
    let mut lo = [f64::MAX; 3];
    let mut hi = [f64::MIN; 3];
    for p in xyz {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // Points by node cube, relative to the low corner.
    let key = |p: &[f64; 3]| -> (i64, i64, i64) {
        (((p[0] - lo[0]) / NODE_EDGE).floor() as i64, ((p[1] - lo[1]) / NODE_EDGE).floor() as i64, ((p[2] - lo[2]) / NODE_EDGE).floor() as i64)
    };
    let mut nodes: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in xyz.iter().enumerate() {
        nodes.entry(key(p)).or_default().push(i);
    }
    let mut keys: Vec<_> = nodes.keys().copied().collect();
    keys.sort();

    // Section 1: node blocks (index entries, then 16-byte point records).
    let mut data: Vec<u8> = Vec::with_capacity(xyz.len() * 20);
    let mut directory: Vec<u8> = Vec::with_capacity(8 + keys.len() * NODE_RECORD);
    directory.extend_from_slice(&(keys.len() as u64).to_le_bytes());
    for k in &keys {
        let members = &nodes[k];
        let cube = [lo[0] + k.0 as f64 * NODE_EDGE, lo[1] + k.1 as f64 * NODE_EDGE, lo[2] + k.2 as f64 * NODE_EDGE];
        let start = data.len();
        for i in 0..members.len() as u32 {
            data.extend_from_slice(&i.to_le_bytes());
        }
        for &i in members {
            let p = xyz[i];
            let q = |v: f64, o: f64| (((v - o) * 1000.0).round().clamp(0.0, 262_143.0)) as u64;
            let packed = q(p[0], cube[0]) | q(p[1], cube[1]) << 18 | q(p[2], cube[2]) << 36;
            data.extend_from_slice(&packed.to_le_bytes());
            data.push((intensity.get(i).copied().unwrap_or(0) >> 8) as u8);
            let c = colours.get(i).copied().unwrap_or([200, 200, 200]);
            data.extend_from_slice(&[c[2], c[1], c[0]]);
            // Normal code past the cube map: "up".
            data.extend_from_slice(&u16::MAX.to_le_bytes());
            data.extend_from_slice(&[0, 0]);
        }
        let size = data.len() - start;
        let mut record = vec![0u8; NODE_RECORD];
        for (j, v) in cube.iter().enumerate() {
            record[j * 8..j * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        // One level, no octree cells: the index is one entry per point.
        record[96..100].copy_from_slice(&1u32.to_le_bytes());
        record[100..104].copy_from_slice(&(members.len() as u32).to_le_bytes());
        record[104..108].copy_from_slice(&(size as u32).to_le_bytes());
        record[236..240].copy_from_slice(&(members.len() as u32).to_le_bytes());
        directory.extend_from_slice(&record);
    }

    let mut header = vec![0u8; HEADER];
    header[0..5].copy_from_slice(b"ADOCT");
    header[5] = 3;
    let put3 = |h: &mut Vec<u8>, at: usize, v: [f64; 3]| {
        for (j, x) in v.iter().enumerate() {
            h[at + j * 8..at + j * 8 + 8].copy_from_slice(&x.to_le_bytes());
        }
    };
    put3(&mut header, 0x10, [0.0; 3]);
    put3(&mut header, 0x28, [0.0; 3]);
    put3(&mut header, 0x40, [1.0; 3]);
    put3(&mut header, 0xA0, lo);
    put3(&mut header, 0xB8, hi);
    header[0xE0] = 1; // RGB
    header[0xE1] = 0; // normals
    header[0xE2] = 1; // intensity
    let id = scan_id(xyz.len(), lo);
    header[0xE3..0xE3 + id.len()].copy_from_slice(id.as_bytes());
    let data_at = HEADER as u64;
    let dir_at = data_at + data.len() as u64;
    header[0x129..0x12D].copy_from_slice(&2u32.to_le_bytes());
    let entry = |h: &mut Vec<u8>, at: usize, kind: u64, offset: u64, size: u64| {
        h[at..at + 8].copy_from_slice(&kind.to_le_bytes());
        h[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
        h[at + 16..at + 24].copy_from_slice(&size.to_le_bytes());
    };
    entry(&mut header, 0x12D, 1, data_at, data.len() as u64);
    entry(&mut header, 0x12D + 24, 2, dir_at, directory.len() as u64);

    let mut f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    for part in [&header, &data, &directory] {
        f.write_all(part).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// A stable "{…}" identifier for the scan from its size and position.
fn scan_id(n: usize, lo: [f64; 3]) -> String {
    let a = (lo[0] * 100.0) as u64 ^ (n as u64).rotate_left(17);
    let b = (lo[1] * 100.0) as u64 ^ (lo[2] * 100.0) as u64;
    format!("{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}", a as u32, (a >> 32) as u16, (b >> 48) as u16, b as u16, (b >> 16) & 0xFFFF_FFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_box() {
        assert_eq!(tiles_for([104735.0, 425195.0, 105035.0, 425445.0]), vec![(104000, 425000), (105000, 425000)]);
        assert_eq!(Edition::Ahn5.tile_url(104000, 425000), "https://fsn1.your-objectstorage.com/hwh-ahn/AHN5_KM/01_LAZ/AHN5_C_104000_425000.COPC.LAZ");
    }

    #[test]
    fn rcs_layout_matches_the_reader() {
        // Re-read with the layout Open CAD Studio's decoder uses.
        let pts: Vec<[f64; 3]> = (0..1000).map(|i| [104000.0 + i as f64 * 0.37, 425000.0 + (i % 50) as f64, 2.0 + (i % 7) as f64]).collect();
        let cols: Vec<[u8; 3]> = (0..1000).map(|i| [i as u8, 0, 255]).collect();
        let path = std::env::temp_dir().join("open-infra-test.rcs");
        write_rcs(&path, &pts, &cols, &vec![65535; 1000]).unwrap();
        let b = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&b[0..5], b"ADOCT");
        let u64_at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let f64_at = |at: usize| f64::from_bits(u64_at(at));
        assert_eq!(u32_at(0x129), 2);
        let (data, dir) = (u64_at(0x12D + 8) as usize, u64_at(0x12D + 24 + 8) as usize);
        let nodes = u64_at(dir) as usize;
        let mut block = data;
        let mut read = Vec::new();
        for n in 0..nodes {
            let rec = dir + 8 + n * NODE_RECORD;
            let cube = [f64_at(rec), f64_at(rec + 8), f64_at(rec + 16)];
            let points = u32_at(rec + 100) as usize;
            let size = u32_at(rec + 104) as usize;
            let mut entries = points;
            for level in 0..32 {
                entries += u32_at(rec + 108 + level * 4) as usize;
            }
            assert_eq!(entries * 4 + points * 16, size);
            for r in b[block + entries * 4..block + size].as_chunks::<16>().0 {
                let v = u64::from_le_bytes(r[0..8].try_into().unwrap());
                let q = |s: u32| ((v >> s) & 0x3FFFF) as f64 * 1e-3;
                read.push(([cube[0] + q(0), cube[1] + q(18), cube[2] + q(36)], [r[11], r[10], r[9]]));
            }
            block += size;
        }
        assert_eq!(read.len(), 1000);
        for (p, c) in &read {
            let i = pts.iter().position(|q| (q[0] - p[0]).abs() < 0.001 && (q[1] - p[1]).abs() < 0.001 && (q[2] - p[2]).abs() < 0.001).expect("point back");
            assert_eq!(*c, cols[i]);
        }
    }

    fn collect(path: &Path) -> Vec<Sample> {
        let mut v = Vec::new();
        stream_file(path, None, 0.0, &|_| {}, &mut |s| {
            v.push(*s);
            true
        })
        .unwrap();
        v
    }

    #[test]
    fn point_files_stream() {
        let dir = std::env::temp_dir().join(format!("oi-wolk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // XYZ with intensity and colour, a PTS with its count line, an ASCII PLY.
        let xyz = dir.join("a.xyz");
        std::fs::write(&xyz, "105000.5 425000.25 1.5 200 255 0 0\n105001.5 425001.25 2.5 100 0 255 0\n").unwrap();
        let pts = dir.join("b.pts");
        std::fs::write(&pts, "2\n1 2 3 -500 10 20 30\n4 5 6 0 40 50 60\n").unwrap();
        let ply = dir.join("c.ply");
        std::fs::write(&ply, "ply\nformat ascii 1.0\nelement vertex 3\nproperty double x\nproperty double y\nproperty double z\nend_header\n1 2 3\n4 5 6\n7 8 9\n").unwrap();
        let txt = dir.join("d.txt");
        std::fs::write(&txt, "10 20 30\n11 21 31\n").unwrap();
        let csv = dir.join("e.txt");
        std::fs::write(&csv, "x;y;naam\n1;2;a\n").unwrap();
        let a = collect(&xyz);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].xyz, [105000.5, 425000.25, 1.5]);
        assert_eq!(a[0].rgb, Some([255, 0, 0]));
        assert_eq!(collect(&pts).len(), 2);
        let c = collect(&ply);
        assert_eq!(c.iter().map(|s| s.xyz[2]).collect::<Vec<_>>(), vec![3.0, 6.0, 9.0]);
        assert!(c.iter().all(|s| s.class.is_none()));
        assert!(is_point_file(&xyz) && is_point_file(&ply) && is_point_file(&txt) && !is_point_file(&csv));
        assert_eq!(collect(&txt).len(), 2);
        // Stopping the pass.
        let mut n = 0;
        let pass = stream_file(&ply, None, 0.0, &|_| {}, &mut |_| {
            n += 1;
            n < 2
        })
        .unwrap();
        assert!(pass.stopped && n == 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_test_clouds_read() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/puntenwolk");
        let las = collect(&data.join("onbewerkt.las"));
        assert_eq!(las.len(), 30_950);
        assert!(las.iter().all(|s| s.class == Some(1)));
        assert!(las.iter().all(|s| s.xyz[0] >= 105_400.0 && s.xyz[0] <= 105_480.5));
        assert!(!is_copc(&data.join("onbewerkt.las")));
        assert_eq!(file_crs(&data.join("onbewerkt.las")), None);
        let ply = collect(&data.join("brug.ply"));
        assert_eq!(ply.len(), 20_648);
        assert!(ply.iter().all(|s| s.xyz[2] >= 0.0 && s.xyz[2] <= 4.6 + 1e-9));
    }

    /// Against the object storage: a small box at the Grote Kerk, Dordrecht.
    #[test]
    #[ignore]
    fn reads_ahn5_copc() {
        let opt = Options { spacing: 1.0, ..Options::default() };
        let c = fetch([104880.0, 425300.0, 104920.0, 425340.0], &opt, &|m| println!("{m}")).unwrap();
        let mut classes: HashMap<u8, usize> = HashMap::new();
        for k in &c.class {
            *classes.entry(*k).or_default() += 1;
        }
        let zmax = c.xyz.iter().map(|p| p[2]).fold(f64::MIN, f64::max);
        println!("{} points, classes {classes:?}, rgb {}, z max {zmax:.1}", c.xyz.len(), c.rgb.is_some());
        assert!(c.xyz.len() > 100);
    }
}
