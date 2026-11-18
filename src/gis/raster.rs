//! Raster import — WMS GetMap and XYZ tiles, written as georeferenced image
//! files for IMAGE entities in the project CRS.
//!
//! When a server cannot deliver the project CRS (e.g. OSM tiles in Web
//! Mercator) the image is reprojected here with a control-point grid, so it
//! lines up with the vector data.

use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgba, RgbaImage};

use crate::lang::tr;

use super::caps::{WmsCaps, WmsLayer};
use super::crs::{is_geographic, Transformer};
use super::http;
use super::servers::MapServer;
use super::site::{envelope, Site};

/// An image file covering `bbox` ([min_e, min_n, max_e, max_n], project CRS).
#[derive(Clone, Debug)]
pub struct PlacedImage {
    pub path: PathBuf,
    pub bbox: [f64; 4],
    pub px: [u32; 2],
}

pub struct Options {
    /// Ground resolution, metres per pixel.
    pub resolution: f64,
    pub format: String,
}

const MAX_TILE_PX: u32 = 2048;
pub const MAX_TOTAL_PX: f64 = 64e6;
// 50 Mpx costs at most 191 MiB of RGBA pixels in the host's decoded cache.
const MAX_AERIAL_PX: u64 = 50_000_000;

fn is_photo(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["ortho", "luchtfoto", "lufo", "s2cloud", "sentinel", "satell", "imagery", "bluemarble"].iter().any(|k| n.contains(k))
}

pub fn default_format(caps: &WmsCaps, layer: &str) -> String {
    let has = |f: &str| caps.formats.iter().any(|x| x == f);
    if is_photo(layer) && has("image/jpeg") {
        "image/jpeg".into()
    } else if has("image/png") || caps.formats.is_empty() {
        "image/png".into()
    } else {
        caps.formats[0].clone()
    }
}

/// The project CRS when the layer offers it, otherwise a common fallback,
/// otherwise a system that transforms offline, otherwise the layer's first.
pub fn request_crs(layer: &WmsLayer, site_crs: &str) -> String {
    if layer.crs.is_empty() || layer.crs.iter().any(|c| c == site_crs) {
        return site_crs.to_string();
    }
    for c in ["EPSG:3857", "EPSG:900913", "EPSG:4326"] {
        if layer.crs.iter().any(|x| x == c) {
            return if c == "EPSG:900913" { "EPSG:3857".into() } else { c.into() };
        }
    }
    if let Some(c) = layer.crs.iter().find(|c| super::crs::support(c) == super::crs::Support::Builtin) {
        return c.clone();
    }
    layer.crs[0].clone()
}

fn get_map_url(caps: &WmsCaps, layer: &str, crs: &str, b: [f64; 4], w: u32, h: u32, format: &str) -> String {
    let v13 = caps.version.starts_with("1.3");
    // WMS 1.3.0 uses the EPSG axis order: lat/lon for geographic systems.
    let bb = if v13 && is_geographic(crs) { [b[1], b[0], b[3], b[2]] } else { b };
    http::with_params(
        &caps.get_map_url,
        &[
            ("SERVICE", "WMS".into()),
            ("VERSION", if v13 { "1.3.0" } else { "1.1.1" }.into()),
            ("REQUEST", "GetMap".into()),
            ("LAYERS", layer.into()),
            ("STYLES", String::new()),
            (if v13 { "CRS" } else { "SRS" }, crs.into()),
            ("BBOX", format!("{},{},{},{}", bb[0], bb[1], bb[2], bb[3])),
            ("WIDTH", w.to_string()),
            ("HEIGHT", h.to_string()),
            ("FORMAT", format.into()),
            ("EXCEPTIONS", if v13 { "XML" } else { "application/vnd.ogc.se_xml" }.into()),
            ("TRANSPARENT", if format == "image/jpeg" { "FALSE" } else { "TRUE" }.into()),
        ],
    )
}

/// Reproject `src` (covering `src_bbox` in `src_crs`) onto a w×h raster that
/// covers `dst_bbox` in `dst_crs`: 33×33 control grid, bilinear interpolation
/// of source positions, nearest-neighbour sampling.
pub(crate) fn warp(src: &RgbaImage, src_crs: &str, src_bbox: [f64; 4], dst_crs: &str, dst_bbox: [f64; 4], w: u32, h: u32) -> Result<RgbaImage, String> {
    let inv = Transformer::new(dst_crs, src_crs)?;
    const G: usize = 32;
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    let mut gu = vec![f64::NAN; (G + 1) * (G + 1)];
    let mut gv = vec![f64::NAN; (G + 1) * (G + 1)];
    for j in 0..=G {
        for i in 0..=G {
            let x = dst_bbox[0] + (dst_bbox[2] - dst_bbox[0]) * (i as f64 / G as f64);
            let y = dst_bbox[3] - (dst_bbox[3] - dst_bbox[1]) * (j as f64 / G as f64);
            if let Ok(p) = inv.apply([x, y]) {
                gu[j * (G + 1) + i] = (p[0] - src_bbox[0]) / (src_bbox[2] - src_bbox[0]) * sw;
                gv[j * (G + 1) + i] = (src_bbox[3] - p[1]) / (src_bbox[3] - src_bbox[1]) * sh;
            }
        }
    }
    let mut out = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    for y in 0..h {
        let fy = (y as f64 + 0.5) / h as f64 * G as f64;
        let j = (fy.floor() as usize).min(G - 1);
        let ty = fy - j as f64;
        for x in 0..w {
            let fx = (x as f64 + 0.5) / w as f64 * G as f64;
            let i = (fx.floor() as usize).min(G - 1);
            let tx = fx - i as f64;
            let k = j * (G + 1) + i;
            let lerp = |g: &[f64]| (g[k] * (1.0 - tx) + g[k + 1] * tx) * (1.0 - ty) + (g[k + G + 1] * (1.0 - tx) + g[k + G + 2] * tx) * ty;
            let (u, v) = (lerp(&gu), lerp(&gv));
            if !(u.is_finite() && v.is_finite()) || u < 0.0 || v < 0.0 || u >= sw || v >= sh {
                continue;
            }
            out.put_pixel(x, y, *src.get_pixel(u as u32, v as u32));
        }
    }
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory(bytes).map(|i| i.to_rgba8()).map_err(|e| format!("afbeelding onleesbaar: {e}"))
}

fn save(img: &RgbaImage, path: &Path, jpeg: bool) -> Result<(), String> {
    if jpeg {
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().save_with_format(path, ImageFormat::Jpeg)
    } else {
        img.save_with_format(path, ImageFormat::Png)
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}

fn slug(s: &str) -> String {
    let mut out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    out.truncate(40);
    out
}

fn stamp() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:x}", t.as_millis())
}

/// The file covers the full extent. Missing parts are transparent in it.
#[derive(Debug)]
pub struct WmsImport {
    pub image: PlacedImage,
    pub parts: u32,
    pub missing: Vec<u32>,
}

fn raster_size(span: [f64; 2], resolution: f64) -> [u32; 2] {
    span.map(|length| ((length / resolution) as u32).max(1))
}

fn pixel_count([width, height]: [u32; 2]) -> u64 {
    u64::from(width) * u64::from(height)
}

fn aerial_size(span: [f64; 2], finest: f64) -> [u32; 2] {
    // Finding the limit on the integer grid also handles long, thin areas.
    let native = raster_size(span, finest);
    if pixel_count(native) <= MAX_AERIAL_PX {
        return native;
    }
    let mut lo = finest;
    let mut hi = span[0].max(span[1]); // a 1 × 1 grid always fits
    for _ in 0..64 {
        let mid = lo + (hi - lo) / 2.0;
        if pixel_count(raster_size(span, mid)) <= MAX_AERIAL_PX {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    raster_size(span, hi)
}

fn wms_extent(site: &Site) -> Result<([f64; 4], [f64; 2]), String> {
    let bbox = site.bbox();
    let span = [bbox[2] - bbox[0], bbox[3] - bbox[1]];
    if span.iter().any(|&n| !n.is_finite() || n <= 0.0) {
        return Err(tr("Ongeldig gebied voor WMS.", "Invalid WMS extent.").into());
    }
    Ok((bbox, span))
}

pub fn import_wms(
    site: &Site,
    caps: &WmsCaps,
    layer: &WmsLayer,
    opt: &Options,
    out_dir: &Path,
    progress: &dyn Fn(String),
) -> Result<WmsImport, String> {
    let (bbox, span) = wms_extent(site)?;
    let px = raster_size(span, opt.resolution);
    if !opt.resolution.is_finite() || opt.resolution <= 0.0 || pixel_count(px) > MAX_TOTAL_PX as u64 {
        return Err(tr("Ongeldige resolutie of te groot WMS-beeld.", "Invalid resolution or WMS image too large.").into());
    }
    import_wms_mosaic(site, caps, layer, &opt.format, bbox, px, out_dir, progress)
}

pub(crate) fn import_aerial_wms(
    site: &Site,
    caps: &WmsCaps,
    layer: &WmsLayer,
    finest: f64,
    out_dir: &Path,
    progress: &dyn Fn(String),
) -> Result<WmsImport, String> {
    let (bbox, span) = wms_extent(site)?;
    let px = aerial_size(span, finest);
    import_wms_mosaic(site, caps, layer, &default_format(caps, &layer.name), bbox, px, out_dir, progress)
}

#[derive(Debug)]
struct WmsTile {
    bbox: [f64; 4],
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Both bounding boxes and copy offsets come from the same pixel edges.
/// Balance each axis across requests to avoid a one-pixel tile at the edge.
fn wms_tiles(bbox: [f64; 4], [w, h]: [u32; 2], [max_w, max_h]: [u32; 2]) -> impl ExactSizeIterator<Item = WmsTile> {
    let (cols, rows) = (w.div_ceil(max_w), h.div_ceil(max_h));
    let east = move |x| bbox[0] + (bbox[2] - bbox[0]) * (f64::from(x) / f64::from(w));
    let north = move |y| bbox[3] - (bbox[3] - bbox[1]) * (f64::from(y) / f64::from(h));
    let edge = |i, size, n| ((u64::from(i) * u64::from(size)) / u64::from(n)) as u32;
    (0..cols * rows).map(move |i| {
        // Number the parts west to east, south to north, as before.
        let (col, row) = (i / rows, rows - 1 - i % rows);
        let (x0, x1) = (edge(col, w, cols), edge(col + 1, w, cols));
        let (y0, y1) = (edge(row, h, rows), edge(row + 1, h, rows));
        WmsTile {
            bbox: [east(x0), north(y1), east(x1), north(y0)],
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        }
    })
}

fn fetch_wms_tile(url: &str, width: u32, height: u32) -> Result<RgbaImage, String> {
    let bytes = http::get_image(url)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    if img.dimensions() != (width, height) {
        return Err(tr("WMS-deel heeft afwijkende afmetingen.", "WMS part has unexpected dimensions.").into());
    }
    let first = img.get_pixel(0, 0);
    if img.pixels().all(|p| p == first || (p[3] == 0 && first[3] == 0)) {
        return Err(tr("WMS-deel bevat overal dezelfde kleur.", "WMS part is entirely one colour.").into());
    }
    Ok(img)
}

#[allow(clippy::too_many_arguments)]
fn import_wms_mosaic(
    site: &Site,
    caps: &WmsCaps,
    layer: &WmsLayer,
    format: &str,
    bbox: [f64; 4],
    px: [u32; 2],
    out_dir: &Path,
    progress: &dyn Fn(String),
) -> Result<WmsImport, String> {
    let crs = request_crs(layer, &site.crs);
    super::crs::ensure(&crs)?;
    let direct = crs == site.crs;
    let source_bbox = if direct { bbox } else { envelope(bbox, &Transformer::new(&site.crs, &crs)?)? };
    let tile_size = [caps.max_width, caps.max_height].map(|n| n.unwrap_or(MAX_TILE_PX).min(MAX_TILE_PX));
    let tiles = wms_tiles(source_bbox, px, tile_size);
    let parts = tiles.len() as u32;
    let mut missing = Vec::new();
    let mut last_error = String::new();
    let mut mosaic = RgbaImage::new(px[0], px[1]);
    for (i, tile) in tiles.enumerate() {
        let part = i as u32 + 1;
        let url = get_map_url(caps, &layer.name, &crs, tile.bbox, tile.width, tile.height, format);
        progress(format!("{} {part}/{parts}…", tr("Kaartdeel ophalen", "Fetching map part")));
        let raster = fetch_wms_tile(&url, tile.width, tile.height).or_else(|_| {
            progress(format!("{} {part}/{parts}…", tr("Kaartdeel opnieuw ophalen", "Retrying map part")));
            fetch_wms_tile(&url, tile.width, tile.height)
        });
        match raster {
            Ok(img) => image::imageops::replace(&mut mosaic, &img, i64::from(tile.x), i64::from(tile.y)),
            Err(err) => {
                missing.push(part);
                last_error = err;
            }
        }
    }
    if missing.len() == parts as usize {
        return Err(format!("{} ({parts}). {last_error}", tr("Alle WMS-delen ontbreken na opnieuw proberen", "All WMS parts are missing after retrying")));
    }
    if !direct {
        progress(format!("{} {}…", tr("Kaartbeeld herprojecteren naar", "Reprojecting map image to"), site.crs));
        mosaic = warp(&mosaic, &crs, source_bbox, &site.crs, bbox, px[0], px[1])?;
    }
    // Preserve transparent gaps, and use PNG for dimensions JPEG cannot encode.
    let jpeg = format == "image/jpeg" && missing.is_empty() && px.iter().all(|&n| n <= u16::MAX.into());
    let path = save_wms_mosaic(&mosaic, &layer.name, out_dir, jpeg)?;
    Ok(WmsImport { image: PlacedImage { path, bbox, px }, parts, missing })
}

fn save_wms_mosaic(img: &RgbaImage, layer: &str, out_dir: &Path, jpeg: bool) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let ext = if jpeg { "jpg" } else { "png" };
    let path = out_dir.join(format!("{}_{stamp:x}.{ext}", slug(layer)));
    let file = std::fs::File::create_new(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut file = std::io::BufWriter::new(file);
    let result = if jpeg {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 90).encode_image(img)
    } else {
        img.write_to(&mut file, ImageFormat::Png)
    };
    let result = result.and_then(|()| std::io::Write::flush(&mut file).map_err(image::ImageError::IoError));
    if let Err(e) = result {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(path)
}

const ORIGIN: f64 = std::f64::consts::PI * 6_378_137.0;
const MAX_TILES: u32 = 64;

/// A tile URL from a template: `{z}`, `{x}`, `{y}`, `{-y}` (TMS row order)
/// and `{q}` (Bing quadkey), the placeholders QGIS writes for XYZ connections.
pub fn tile_url(template: &str, z: i32, x: i64, y: i64) -> String {
    let quadkey = || {
        (1..=z)
            .rev()
            .map(|i| {
                let m = 1i64 << (i - 1);
                char::from(b'0' + ((x & m != 0) as u8) + 2 * ((y & m != 0) as u8))
            })
            .collect::<String>()
    };
    let mut url = template.replace("{z}", &z.to_string()).replace("{x}", &x.to_string());
    if url.contains("{-y}") {
        url = url.replace("{-y}", &((1i64 << z) - 1 - y).to_string());
    }
    if url.contains("{q}") {
        url = url.replace("{q}", &quadkey());
    }
    url.replace("{y}", &y.to_string())
}

/// The zoom level is below what the server provides (QGIS `zmin`).
fn below_min_zoom(server: &MapServer, z: i32) -> Result<(), String> {
    match server.min_zoom {
        Some(min) if z < min as i32 => Err(format!(
            "{} {min}; {} {z}. {}",
            crate::lang::tr("Deze tegelserver levert pas tegels vanaf zoomniveau", "This tile server only provides tiles from zoom level"),
            crate::lang::tr("dit gebied vraagt zoomniveau", "this area needs zoom level"),
            crate::lang::tr("Kies een kleiner gebied of een grotere resolutie.", "Choose a smaller area or a finer resolution."),
        )),
        _ => Ok(()),
    }
}

pub fn import_xyz(site: &Site, server: &MapServer, opt: &Options, out_dir: &Path, progress: &dyn Fn(String)) -> Result<Vec<PlacedImage>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let bbox = site.bbox();
    if super::arcgis::is_tile_cache(&server.url) {
        let (canvas, crs, src_bbox) = super::arcgis::fetch(site, &server.url, opt.resolution, MAX_TILES, progress)?;
        progress(format!("Herprojecteren naar {}…", site.crs));
        let res = opt.resolution.max(site.width.max(site.height) / 4096.0);
        let (w, h) = ((site.width / res).round() as u32, (site.height / res).round() as u32);
        let img = warp(&canvas, &crs, src_bbox, &site.crs, bbox, w.max(1), h.max(1))?;
        let path = out_dir.join(format!("{}_{}.png", slug(&server.id), stamp()));
        save(&img, &path, false)?;
        return Ok(vec![PlacedImage { path, bbox, px: [w, h] }]);
    }
    let merc = envelope(bbox, &Transformer::new(&site.crs, "EPSG:3857")?)?;
    let max_zoom = server.max_zoom.unwrap_or(19) as i32;
    let z_ideal = (2.0 * ORIGIN * site.lat.to_radians().cos() / (256.0 * opt.resolution)).log2();
    let min_zoom = (server.min_zoom.unwrap_or(0) as i32).min(max_zoom);
    let mut z = (z_ideal.ceil() as i32).clamp(min_zoom, max_zoom);
    let range = |z: i32| {
        let size = 2.0 * ORIGIN / 2f64.powi(z);
        let x0 = ((merc[0] + ORIGIN) / size).floor() as i64;
        let x1 = ((merc[2] + ORIGIN) / size).floor() as i64;
        let y0 = ((ORIGIN - merc[3]) / size).floor() as i64;
        let y1 = ((ORIGIN - merc[1]) / size).floor() as i64;
        (size, x0, x1, y0, y1)
    };
    let mut r = range(z);
    while z > 0 && ((r.2 - r.1 + 1) * (r.4 - r.3 + 1)) as u32 > MAX_TILES {
        z -= 1;
        r = range(z);
    }
    below_min_zoom(server, z)?;
    let (size, x0, x1, y0, y1) = r;
    let (nx, ny) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let mut canvas = RgbaImage::new(nx * 256, ny * 256);
    let total = nx * ny;
    let mut done = 0;
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            done += 1;
            progress(format!("Tegels ophalen {done}/{total} (zoom {z})…"));
            let url = tile_url(&server.url, z, tx, ty);
            let tile = decode(&http::get_image(&url)?)?;
            image::imageops::overlay(&mut canvas, &tile, (tx - x0) * 256, (ty - y0) * 256);
            // Tile servers ask for modest request rates.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    let src_bbox = [
        x0 as f64 * size - ORIGIN,
        ORIGIN - (y1 + 1) as f64 * size,
        (x1 + 1) as f64 * size - ORIGIN,
        ORIGIN - y0 as f64 * size,
    ];
    progress(format!("Herprojecteren naar {}…", site.crs));
    let res = opt.resolution.max(site.width.max(site.height) / 4096.0);
    let (w, h) = ((site.width / res).round() as u32, (site.height / res).round() as u32);
    let img = warp(&canvas, "EPSG:3857", src_bbox, &site.crs, bbox, w.max(1), h.max(1))?;
    let path = out_dir.join(format!("{}_z{z}_{}.png", slug(&server.id), stamp()));
    save(&img, &path, false)?;
    Ok(vec![PlacedImage { path, bbox, px: [w, h] }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aerial_tiles_share_one_grid_at_the_source_resolution() {
        let bbox = [120000.13, 425000.17, 120401.16, 425210.20];
        let span = [bbox[2] - bbox[0], bbox[3] - bbox[1]];
        let px = aerial_size(span, 0.08);
        assert_eq!(px, [5012, 2625]);
        let tiles: Vec<_> = wms_tiles(bbox, px, [2000, 1500]).collect();
        assert_eq!(tiles.len(), 6);
        assert_eq!(tiles[0].bbox[0..2], bbox[0..2]);
        assert_eq!(tiles[5].bbox[2..4], bbox[2..4]);
        assert_eq!(tiles[0].bbox[3], tiles[1].bbox[1]);
        assert_eq!(tiles[0].bbox[2], tiles[2].bbox[0]);
        assert_eq!((tiles[0].x, tiles[0].y), (0, 1312));
        assert_eq!((tiles[1].x, tiles[1].y), (0, 0));
        assert_eq!((tiles[2].x, tiles[2].y), (1670, 1312));
        for tile in tiles {
            assert!(tile.width <= 2000 && tile.height <= 1500);
            assert!(((tile.bbox[2] - tile.bbox[0]) / f64::from(tile.width) - span[0] / f64::from(px[0])).abs() < 1e-10);
            assert!(((tile.bbox[3] - tile.bbox[1]) / f64::from(tile.height) - span[1] / f64::from(px[1])).abs() < 1e-10);
        }
    }

    #[test]
    fn aerial_size_uses_the_finest_grid_that_fits() {
        assert_eq!(aerial_size([500.0, 500.0], 0.08), [6250, 6250]);
        for span in [[801.0, 610.0], [20_000.0, 20_000.0], [20_000.0, 10.0]] {
            let px = aerial_size(span, 0.08);
            assert!(pixel_count(px) <= MAX_AERIAL_PX);
            let pitch = (span[0] / f64::from(px[0])).min(span[1] / f64::from(px[1]));
            assert!(pitch >= 0.08);
            if px != raster_size(span, 0.08) {
                let next = (span[0] / f64::from(px[0] + 1)).max(span[1] / f64::from(px[1] + 1));
                let finer = raster_size(span, next.next_down());
                assert!(pixel_count(finer) > MAX_AERIAL_PX, "{span:?}: {px:?} -> {finer:?}");
            }
        }
    }

    #[test]
    fn wms_request_crs_prefers_transformable() {
        let layer = |crs: &[&str]| WmsLayer { name: "l".into(), title: "l".into(), crs: crs.iter().map(|c| c.to_string()).collect(), bbox: None };
        assert_eq!(request_crs(&layer(&["EPSG:28992", "EPSG:3857"]), "EPSG:28992"), "EPSG:28992");
        assert_eq!(request_crs(&layer(&["EPSG:25832", "EPSG:900913"]), "EPSG:28992"), "EPSG:3857");
        // No common fallback: a built-in system before an unknown first one.
        assert_eq!(request_crs(&layer(&["EPSG:2180", "EPSG:25833"]), "EPSG:28992"), "EPSG:25833");
        assert_eq!(request_crs(&layer(&["EPSG:2180"]), "EPSG:28992"), "EPSG:2180");
    }

    #[test]
    fn wms_retries_white_tiles_and_names_missing_parts() {
        use std::io::{BufRead, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/wms", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let worker = std::thread::spawn(move || {
            let white = RgbaImage::from_pixel(2, 2, Rgba([255; 4]));
            let mut good = white.clone();
            good.put_pixel(0, 0, Rgba([32, 64, 96, 255]));
            for (mime, body) in [
                ("image/png", png_bytes(white)),
                ("image/png", png_bytes(good)),
                ("text/xml", b"<ServiceException>laadfout</ServiceException>".to_vec()),
                ("image/png", b"<ServiceException>laadfout</ServiceException>".to_vec()),
            ] {
                let (mut conn, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(&mut conn);
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                let request = line.clone();
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                assert!(request.contains("WIDTH=2") && request.contains("HEIGHT=2"));
                assert!(request.contains("EXCEPTIONS=XML"));
                count.fetch_add(1, Ordering::Relaxed);
                write!(conn, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                conn.write_all(&body).unwrap();
            }
        });
        let caps = super::super::caps::parse_wms(&url, r#"<WMS_Capabilities version="1.3.0">
            <Service><MaxWidth>2</MaxWidth><MaxHeight>2</MaxHeight></Service>
            <Capability><Layer><Name>ortho</Name><CRS>EPSG:28992</CRS></Layer></Capability>
            </WMS_Capabilities>"#).unwrap();
        let site = Site { width: 4.0, height: 2.0, ..Site::default() };
        let opt = Options { resolution: 1.0, format: "image/png".into() };
        let dir = std::env::temp_dir().join(format!("ois-wms-test-{}-{}", std::process::id(), stamp()));
        let result = import_wms(&site, &caps, &caps.layers[0], &opt, &dir, &|_| {});
        assert_eq!(calls.load(Ordering::Relaxed), 4, "blank and invalid replies each need a retry: {result:?}");
        worker.join().unwrap();
        let result = result.unwrap();
        assert_eq!(result.parts, 2);
        assert_eq!(result.missing, [2]);
        assert_eq!(result.image.bbox, site.bbox());
        assert_eq!(result.image.px, [4, 2]);
        let image = image::open(&result.image.path).unwrap().into_rgba8();
        assert_eq!(image.get_pixel(0, 0).0, [32, 64, 96, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [255; 4]);
        assert!(image.enumerate_pixels().filter(|(x, _, _)| *x >= 2).all(|(_, _, p)| p[3] == 0));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn png_bytes(img: RgbaImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
}

/// A small preview raster for the workbench map, with its corners in lon/lat
/// (upper-left, upper-right, lower-right, lower-left).
pub struct Preview {
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
    pub corners: [[f64; 2]; 4],
}

fn corners_lonlat(b: [f64; 4], from: &str) -> Result<[[f64; 2]; 4], String> {
    let t = Transformer::new(from, "EPSG:4326")?;
    Ok([t.apply([b[0], b[3]])?, t.apply([b[2], b[3]])?, t.apply([b[2], b[1]])?, t.apply([b[0], b[1]])?])
}

fn preview_size(site: &Site, max_px: u32) -> (u32, u32) {
    let r = site.width / site.height;
    if r >= 1.0 { (max_px, ((max_px as f64 / r).round() as u32).max(1)) } else { (((max_px as f64 * r).round() as u32).max(1), max_px) }
}

/// "Test" for a WMS layer: one GetMap of the import area, as the server
/// delivers it (no reprojection; corners place it on the map).
pub fn preview_wms(site: &Site, caps: &WmsCaps, layer: &WmsLayer, format: &str, max_px: u32) -> Result<Preview, String> {
    let crs = request_crs(layer, &site.crs);
    super::crs::ensure(&crs)?;
    let data_crs =if crs == "CRS:84" { "EPSG:4326".to_string() } else { crs.clone() };
    let b = if crs == site.crs { site.bbox() } else { envelope(site.bbox(), &Transformer::new(&site.crs, &data_crs)?)? };
    let (w, h) = preview_size(site, max_px);
    let img = decode(&http::get_image(&get_map_url(caps, &layer.name, &crs, b, w, h, format))?)?;
    Ok(Preview { size: [img.width() as usize, img.height() as usize], rgba: img.into_raw(), corners: corners_lonlat(b, &data_crs)? })
}

/// "Test" for a tile server: the tiles covering the import area, stitched.
pub fn preview_xyz(site: &Site, server: &MapServer) -> Result<Preview, String> {
    if super::arcgis::is_tile_cache(&server.url) {
        let res = site.width.max(site.height) / 768.0;
        let (canvas, crs, b) = super::arcgis::fetch(site, &server.url, res, 9, &|_| {})?;
        return Ok(Preview { size: [canvas.width() as usize, canvas.height() as usize], rgba: canvas.into_raw(), corners: corners_lonlat(b, &crs)? });
    }
    let merc = envelope(site.bbox(), &Transformer::new(&site.crs, "EPSG:3857")?)?;
    let mut z = (server.max_zoom.unwrap_or(19) as i32).min(19);
    let range = |z: i32| {
        let size = 2.0 * ORIGIN / 2f64.powi(z);
        (size, ((merc[0] + ORIGIN) / size).floor() as i64, ((merc[2] + ORIGIN) / size).floor() as i64,
         ((ORIGIN - merc[3]) / size).floor() as i64, ((ORIGIN - merc[1]) / size).floor() as i64)
    };
    let mut r = range(z);
    while z > 0 && (r.2 - r.1 + 1) * (r.4 - r.3 + 1) > 9 {
        z -= 1;
        r = range(z);
    }
    below_min_zoom(server, z)?;
    let (size, x0, x1, y0, y1) = r;
    let mut canvas = RgbaImage::new(((x1 - x0 + 1) * 256) as u32, ((y1 - y0 + 1) * 256) as u32);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let url = tile_url(&server.url, z, tx, ty);
            let tile = decode(&http::get_image(&url)?)?;
            image::imageops::overlay(&mut canvas, &tile, (tx - x0) * 256, (ty - y0) * 256);
        }
    }
    let b = [x0 as f64 * size - ORIGIN, ORIGIN - (y1 + 1) as f64 * size, (x1 + 1) as f64 * size - ORIGIN, ORIGIN - y0 as f64 * size];
    Ok(Preview { size: [canvas.width() as usize, canvas.height() as usize], rgba: canvas.into_raw(), corners: corners_lonlat(b, "EPSG:3857")? })
}
