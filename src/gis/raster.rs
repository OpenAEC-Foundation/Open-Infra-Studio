//! Raster import — WMS GetMap and XYZ tiles, written as georeferenced image
//! files for IMAGE entities in the project CRS.
//!
//! When a server cannot deliver the project CRS (e.g. OSM tiles in Web
//! Mercator) the image is reprojected here with a control-point grid, so it
//! lines up with the vector data.

use std::path::{Path, PathBuf};

use image::{ImageFormat, Rgba, RgbaImage};

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

const MAX_TILE_PX: f64 = 2048.0;
pub const MAX_TOTAL_PX: f64 = 64e6;

fn tiles(b: [f64; 4], res: f64) -> Vec<([f64; 4], u32, u32)> {
    let tile_m = MAX_TILE_PX * res;
    let mut out = Vec::new();
    let mut e = b[0];
    while e < b[2] - 1e-6 {
        let mut n = b[1];
        let e2 = (e + tile_m).min(b[2]);
        while n < b[3] - 1e-6 {
            let n2 = (n + tile_m).min(b[3]);
            out.push(([e, n, e2, n2], (((e2 - e) / res).round() as u32).max(1), (((n2 - n) / res).round() as u32).max(1)));
            n = n2;
        }
        e = e2;
    }
    out
}

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

fn save_bytes(bytes: &[u8], path: &Path) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
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

pub fn import_wms(
    site: &Site,
    caps: &WmsCaps,
    layer: &WmsLayer,
    opt: &Options,
    out_dir: &Path,
    progress: &dyn Fn(String),
) -> Result<Vec<PlacedImage>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let crs = request_crs(layer, &site.crs);
    // A system outside the built-in list: its definition from epsg.io, once.
    super::crs::ensure(&crs)?;
    let parts = tiles(site.bbox(), opt.resolution);
    let jpeg = opt.format == "image/jpeg";
    let ext = if jpeg { "jpg" } else { "png" };
    let base = format!("{}_{}", slug(&layer.name), stamp());
    let mut placed = Vec::new();
    for (i, (b, w, h)) in parts.iter().enumerate() {
        progress(format!("Kaartdeel {}/{} ophalen…", i + 1, parts.len()));
        let path = out_dir.join(format!("{base}_{}.{ext}", i + 1));
        if crs == site.crs {
            let bytes = http::get_image(&get_map_url(caps, &layer.name, &crs, *b, *w, *h, &opt.format))?;
            // Keep the server's encoding when it is what we want on disk.
            if (jpeg && bytes.starts_with(&[0xFF, 0xD8])) || (!jpeg && bytes.starts_with(b"\x89PNG")) {
                save_bytes(&bytes, &path)?;
            } else {
                save(&decode(&bytes)?, &path, jpeg)?;
            }
        } else {
            let src_bbox = envelope(*b, &Transformer::new(&site.crs, &crs)?)?;
            let bytes = http::get_image(&get_map_url(caps, &layer.name, &crs, src_bbox, *w, *h, &opt.format))?;
            progress(format!("Kaartdeel {}/{} herprojecteren naar {}…", i + 1, parts.len(), site.crs));
            let img = warp(&decode(&bytes)?, &crs, src_bbox, &site.crs, *b, *w, *h)?;
            save(&img, &path, jpeg)?;
        }
        placed.push(PlacedImage { path, bbox: *b, px: [*w, *h] });
    }
    Ok(placed)
}

const ORIGIN: f64 = std::f64::consts::PI * 6_378_137.0;
const MAX_TILES: u32 = 64;

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
    let mut z = (z_ideal.ceil() as i32).clamp(0, max_zoom);
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
    let (size, x0, x1, y0, y1) = r;
    let (nx, ny) = ((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
    let mut canvas = RgbaImage::new(nx * 256, ny * 256);
    let total = nx * ny;
    let mut done = 0;
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            done += 1;
            progress(format!("Tegels ophalen {done}/{total} (zoom {z})…"));
            let url = server.url.replace("{z}", &z.to_string()).replace("{x}", &tx.to_string()).replace("{y}", &ty.to_string());
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
    fn tiles_cover_extent_without_overlap() {
        let t = tiles([0.0, 0.0, 1000.0, 600.0], 0.25);
        assert_eq!(t.len(), 4);
        assert_eq!(t[0].0, [0.0, 0.0, 512.0, 512.0]);
        assert_eq!((t[0].1, t[0].2), (2048, 2048));
        assert_eq!(t[1].0, [0.0, 512.0, 512.0, 600.0]);
        assert_eq!((t[3].1, t[3].2), (1952, 352));
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
    let (size, x0, x1, y0, y1) = r;
    let mut canvas = RgbaImage::new(((x1 - x0 + 1) * 256) as u32, ((y1 - y0 + 1) * 256) as u32);
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let url = server.url.replace("{z}", &z.to_string()).replace("{x}", &tx.to_string()).replace("{y}", &ty.to_string());
            let tile = decode(&http::get_image(&url)?)?;
            image::imageops::overlay(&mut canvas, &tile, (tx - x0) * 256, (ty - y0) * 256);
        }
    }
    let b = [x0 as f64 * size - ORIGIN, ORIGIN - (y1 + 1) as f64 * size, (x1 + 1) as f64 * size - ORIGIN, ORIGIN - y0 as f64 * size];
    Ok(Preview { size: [canvas.width() as usize, canvas.height() as usize], rgba: canvas.into_raw(), corners: corners_lonlat(b, "EPSG:3857")? })
}
