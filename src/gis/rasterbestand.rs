//! Raster and point cloud files into the drawing: GeoTIFF (an orthophoto,
//! or a height model in float or 16-bit), ESRI ASCII grids, JPG/PNG with a
//! world file, and point clouds (LAS/LAZ, E57, PLY, PTS, XYZ). Images are
//! placed as raster images (warped to the project CRS when needed); height
//! models become a shaded relief image and contour lines on NLCS layers;
//! point clouds an RCS file attached to the drawing.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};

use super::ahn::{self, Grid};
use super::crs::XY;
use super::raster::PlacedImage;
use super::site::Site;
use crate::bridge;
use crate::emit;
use crate::nlcs::{self, Element};

/// Extensions for the file dialog (lower case, without the dot).
pub const EXTENSIONS: &[&str] = &["tif", "tiff", "asc", "jpg", "jpeg", "png", "las", "laz", "e57", "ply", "pts", "xyz"];

/// What a raster file holds.
pub enum Raster {
    /// Colours: the image and its bounds [x0, y0, x1, y1] in `crs`.
    Image { img: RgbaImage, bbox: [f64; 4], crs: Option<String> },
    /// Heights (m) on a regular grid in `crs`.
    Heights { grid: Grid, crs: Option<String> },
}

/// EPSG code from the GeoKeyDirectory: projected (3072), else geographic (2048).
fn geokey_epsg(keys: &[u64]) -> Option<String> {
    if keys.len() < 4 {
        return None;
    }
    let n = keys[3] as usize;
    let mut found = None;
    for i in 0..n {
        let k = &keys.get(4 + i * 4..8 + i * 4)?;
        // Inline SHORT values (location 0) only.
        if k[1] == 0 && (k[0] == 3072 || (k[0] == 2048 && found.is_none())) && k[3] > 0 && k[3] < 32767 {
            found = Some(format!("EPSG:{}", k[3]));
            if k[0] == 3072 {
                break;
            }
        }
    }
    found
}

/// A GeoTIFF: heights for one float or 16-bit band, an image otherwise.
pub fn read_geotiff(bytes: &[u8]) -> Result<Raster, String> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::Tag;
    use tiff::ColorType;
    let err = |e: tiff::TiffError| format!("GeoTIFF: {e}");
    let mut dec = Decoder::new(Cursor::new(bytes)).map_err(err)?;
    let (w, h) = dec.dimensions().map_err(err)?;
    let scale = dec.get_tag_f64_vec(Tag::ModelPixelScaleTag).ok();
    let tie = dec.get_tag_f64_vec(Tag::ModelTiepointTag).ok();
    let transform = dec.get_tag_f64_vec(Tag::Unknown(34264)).ok();
    let keys = dec.get_tag_u64_vec(Tag::Unknown(34735)).ok();
    let crs = keys.as_deref().and_then(geokey_epsg);
    let nodata: Option<f64> = dec.get_tag_ascii_string(Tag::GdalNodata).ok().and_then(|s| s.trim().trim_end_matches('\0').parse().ok());
    // Top-left corner and pixel size.
    let (x0, y1, sx, sy) = match (scale, tie, transform) {
        (Some(s), Some(t), _) if s.len() >= 2 && t.len() >= 6 => (t[3] - t[0] * s[0], t[4] + t[1] * s[1], s[0], s[1]),
        (_, _, Some(m)) if m.len() >= 8 => (m[3], m[7], m[0], -m[5]),
        _ => return Err("GeoTIFF zonder georeferentie (geen tiepoint of transformatie); een world file (.tfw) helpt".into()),
    };
    let color = dec.colortype().map_err(err)?;
    let bbox = [x0, y1 - h as f64 * sy, x0 + w as f64 * sx, y1];
    let data = dec.read_image().map_err(err)?;
    let heights = |v: Vec<f64>| -> Raster {
        let z = v.into_iter().map(|v| if !v.is_finite() || nodata.is_some_and(|n| v == n) || v.abs() > 1.0e5 { f32::NAN } else { v as f32 }).collect();
        Raster::Heights { grid: Grid { x0, y1, cell: sx, w: w as usize, h: h as usize, z }, crs: crs.clone() }
    };
    Ok(match (color, data) {
        (ColorType::Gray(_), DecodingResult::F32(v)) => heights(v.into_iter().map(f64::from).collect()),
        (ColorType::Gray(_), DecodingResult::F64(v)) => heights(v),
        (ColorType::Gray(_), DecodingResult::I16(v)) => heights(v.into_iter().map(f64::from).collect()),
        (ColorType::Gray(_), DecodingResult::U16(v)) => heights(v.into_iter().map(f64::from).collect()),
        (ColorType::Gray(_), DecodingResult::I32(v)) => heights(v.into_iter().map(f64::from).collect()),
        (c, DecodingResult::U8(v)) => {
            let n = match c {
                ColorType::RGB(_) => 3,
                ColorType::RGBA(_) => 4,
                ColorType::GrayA(_) => 2,
                _ => 1,
            };
            let mut img = RgbaImage::new(w, h);
            for (i, px) in img.pixels_mut().enumerate() {
                let p = &v[i * n..(i * n + n).min(v.len())];
                *px = match n {
                    3 => Rgba([p[0], p[1], p[2], 255]),
                    4 => Rgba([p[0], p[1], p[2], p[3]]),
                    2 => Rgba([p[0], p[0], p[0], p[1]]),
                    _ => Rgba([p[0], p[0], p[0], 255]),
                };
            }
            Raster::Image { img, bbox, crs }
        }
        (c, _) => return Err(format!("GeoTIFF met {c:?}-pixels wordt nog niet ondersteund")),
    })
}

/// ESRI ASCII grid (.asc).
pub fn read_asc(text: &str) -> Result<Grid, String> {
    let mut head = std::collections::HashMap::new();
    let mut lines = text.lines().peekable();
    while let Some(l) = lines.peek() {
        let mut parts = l.split_whitespace();
        match (parts.next(), parts.next()) {
            (Some(k), Some(v)) if k.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
                head.insert(k.to_ascii_lowercase(), v.parse::<f64>().map_err(|_| format!("ASC: kopregel '{l}'"))?);
                lines.next();
            }
            _ => break,
        }
    }
    let get = |k: &str| head.get(k).copied().ok_or_else(|| format!("ASC: '{k}' ontbreekt"));
    let (w, h, cell) = (get("ncols")? as usize, get("nrows")? as usize, get("cellsize")?);
    let x0 = head.get("xllcorner").copied().or_else(|| head.get("xllcenter").map(|c| c - cell / 2.0)).ok_or("ASC: xllcorner ontbreekt")?;
    let y0 = head.get("yllcorner").copied().or_else(|| head.get("yllcenter").map(|c| c - cell / 2.0)).ok_or("ASC: yllcorner ontbreekt")?;
    let nodata = head.get("nodata_value").copied();
    let z: Vec<f32> = lines
        .flat_map(|l| l.split_whitespace())
        .map(|t| t.parse::<f64>().map(|v| if nodata == Some(v) { f32::NAN } else { v as f32 }).unwrap_or(f32::NAN))
        .take(w * h)
        .collect();
    if z.len() < w * h {
        return Err(format!("ASC: {} waarden, {} verwacht", z.len(), w * h));
    }
    Ok(Grid { x0, y1: y0 + h as f64 * cell, cell, w, h, z })
}

/// The world file beside an image: .tfw/.jgw/.pgw (first and last letter of
/// the extension plus w) or .wld. Returns the bounds of a w×h image.
pub fn world_file(path: &Path, w: u32, h: u32) -> Option<[f64; 4]> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let short = format!("{}{}w", &ext[..1], &ext[ext.len() - 1..]);
    let text = [short.as_str(), "wld", &format!("{ext}w")].iter().find_map(|e| std::fs::read_to_string(path.with_extension(e)).ok())?;
    let v: Vec<f64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if v.len() < 6 || v[1].abs() > 1e-9 || v[2].abs() > 1e-9 {
        return None;
    }
    // A, D, B, E, C, F: C and F are the centre of the top-left pixel.
    let (a, e, c, f) = (v[0], v[3], v[4], v[5]);
    let (x0, y1) = (c - a / 2.0, f - e / 2.0);
    Some([x0, y1 + e * h as f64, x0 + a * w as f64, y1])
}

/// The CRS beside a file: an EPSG code in a .prj (AUTHORITY or a known name).
pub fn prj_crs(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path.with_extension("prj")).ok()?;
    if let Some(i) = text.rfind("AUTHORITY[\"EPSG\",\"") {
        let code: String = text[i + 18..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if !code.is_empty() {
            return Some(format!("EPSG:{code}"));
        }
    }
    let t = text.to_ascii_lowercase();
    [("amersfoort", "EPSG:28992"), ("rd_new", "EPSG:28992"), ("belge_lambert_1972", "EPSG:31370"), ("british_national_grid", "EPSG:27700"), ("web_mercator", "EPSG:3857"), ("wgs_1984\"", "EPSG:4326")]
        .iter()
        .find(|(k, _)| t.contains(k))
        .map(|(_, c)| c.to_string())
}

fn read(path: &Path) -> Result<Raster, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let side_crs = prj_crs(path);
    match ext.as_str() {
        "tif" | "tiff" => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            match read_geotiff(&bytes) {
                Ok(Raster::Image { img, bbox, crs }) => Ok(Raster::Image { img, bbox, crs: crs.or(side_crs) }),
                Ok(Raster::Heights { grid, crs }) => Ok(Raster::Heights { grid, crs: crs.or(side_crs) }),
                // A plain TIFF with a world file.
                Err(e) => {
                    let img = image_from_tiff(&bytes).map_err(|_| e.clone())?;
                    let bbox = world_file(path, img.width(), img.height()).ok_or(e)?;
                    Ok(Raster::Image { img, bbox, crs: side_crs })
                }
            }
        }
        "asc" => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Raster::Heights { grid: read_asc(&text)?, crs: side_crs })
        }
        "jpg" | "jpeg" | "png" => {
            let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.to_rgba8();
            let bbox = world_file(path, img.width(), img.height()).ok_or("geen world file (.jgw/.pgw/.wld) naast de afbeelding: zonder kan het niet geplaatst worden")?;
            Ok(Raster::Image { img, bbox, crs: side_crs })
        }
        _ => Err(format!("{ext}: geen rasterformaat")),
    }
}

fn image_from_tiff(bytes: &[u8]) -> Result<RgbaImage, String> {
    match read_geotiff(bytes) {
        Ok(Raster::Image { img, .. }) => Ok(img),
        _ => Err("TIFF onleesbaar".into()),
    }
}

/// A shaded relief of a height grid (light from the north-west), with
/// transparent cells where there is no height.
pub fn hillshade(g: &Grid) -> RgbaImage {
    let mut img = RgbaImage::new(g.w as u32, g.h as u32);
    let at = |c: i64, r: i64| -> Option<f64> { g.at(c.clamp(0, g.w as i64 - 1) as usize, r.clamp(0, g.h as i64 - 1) as usize) };
    let (az, alt) = (315f64.to_radians(), 45f64.to_radians());
    for r in 0..g.h as i64 {
        for c in 0..g.w as i64 {
            let Some(z) = at(c, r) else { continue };
            let dzdx = (at(c + 1, r).unwrap_or(z) - at(c - 1, r).unwrap_or(z)) / (2.0 * g.cell);
            let dzdy = (at(c, r - 1).unwrap_or(z) - at(c, r + 1).unwrap_or(z)) / (2.0 * g.cell);
            let slope = (dzdx.hypot(dzdy)).atan();
            let aspect = dzdy.atan2(-dzdx);
            let v = (alt.sin() * slope.cos() + alt.cos() * slope.sin() * (az - aspect).cos()).clamp(0.0, 1.0);
            let s = (40.0 + v * 215.0) as u8;
            img.put_pixel(c as u32, r as u32, Rgba([s, s, s, 255]));
        }
    }
    img
}

/// `import_path`: place a raster or point cloud file in the project.
pub fn import_path(site: &Site, path: &Path, progress: &dyn Fn(String)) -> Result<String, String> {
    if super::pointcloud::is_point_file(path) {
        return point_cloud(site, path, progress);
    }
    let t = bridge::target().ok_or("geen tekening actief")?;
    let f = t.unit_factor;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("raster").to_string();
    progress(format!("{} lezen…", path.display()));
    let raster = read(path)?;
    let dir = bridge::image_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    match raster {
        Raster::Image { img, bbox, crs } => {
            let crs = crs.unwrap_or_else(|| site.crs.clone());
            let (img, bbox) = to_site(site, img, bbox, &crs)?;
            let file = dir.join(format!("{name}.png"));
            img.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let placed = PlacedImage { path: file.clone(), bbox, px: [img.width(), img.height()] };
            let layer = emit::layer_name("GIS", &name);
            bridge::ensure_layer(&layer, ocs_plugin_api::host::codec::Color::ByLayer);
            let n = bridge::add_entities(&format!("Raster {name}"), emit::images(site, &[placed], &layer, f, 0))?;
            Ok(format!("{name}: {}×{} px geplaatst ({crs}) — {n} element op {layer}.", img.width(), img.height()))
        }
        Raster::Heights { grid, crs } => heights(site, &grid, &crs.unwrap_or_else(|| site.crs.clone()), &name, &dir, f, progress),
    }
}

/// An image in another CRS warped onto the project CRS.
fn to_site(site: &Site, img: RgbaImage, bbox: [f64; 4], crs: &str) -> Result<(RgbaImage, [f64; 4]), String> {
    if crs.eq_ignore_ascii_case(&site.crs) {
        return Ok((img, bbox));
    }
    let dst = super::site::envelope(bbox, &super::crs::Transformer::new(crs, &site.crs)?)?;
    let (w, h) = (img.width(), img.height());
    let out = super::raster::warp(&img, crs, bbox, &site.crs, dst, w, h)?;
    Ok((out, dst))
}

/// A height model: a shaded relief under contour lines.
fn heights(site: &Site, grid: &Grid, crs: &str, name: &str, dir: &Path, f: f64, progress: &dyn Fn(String)) -> Result<String, String> {
    let (lo, hi) = grid.range().ok_or("het hoogtemodel heeft geen waarden")?;
    progress("Reliëf en hoogtelijnen…".into());
    let shade = hillshade(grid);
    let bbox = [grid.x0, grid.y1 - grid.h as f64 * grid.cell, grid.x0 + grid.w as f64 * grid.cell, grid.y1];
    let (shade, sbbox) = to_site(site, shade, bbox, crs)?;
    let file = dir.join(format!("{name}_relief.png"));
    shade.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let relief_layer = emit::layer_name("GIS", &format!("{name} relief"));
    bridge::ensure_layer(&relief_layer, ocs_plugin_api::host::codec::Color::ByLayer);
    let mut ents = emit::images(site, &[PlacedImage { path: file, bbox: sbbox, px: [shade.width(), shade.height()] }], &relief_layer, f, 0);
    // Contours: about twenty levels over the range.
    let interval = crate::civiel::nice_step(((hi - lo) / 20.0).max(0.01));
    let levels = ahn::levels((lo, hi), interval);
    let lines = ahn::contours(grid, &levels, 4.0 * grid.cell);
    let to = super::vector::data_to_site(site, crs, &[])?;
    let opt = crate::basisdata::NlcsOptions::default();
    let spec = nlcs::layer(nlcs::object_by_name("OG", "HOOGTELIJN").ok_or("NLCS OG-HOOGTELIJN ontbreekt")?, opt.status, &opt.discipline, Element::G);
    bridge::ensure_nlcs_layer(&spec);
    let k = opt.scale / 1000.0 * f;
    let mut count = 0;
    for (level, polys) in &lines {
        for (pts, closed) in polys {
            let d: Vec<XY> = pts.iter().filter_map(|p| to(*p)).map(|q| site.to_drawing(q, f)).collect();
            if d.len() < 2 {
                continue;
            }
            for mut e in emit::polylines(&[(d, *closed)], &spec.name, k, None) {
                if let ocs_plugin_api::host::codec::EntityType::LwPolyline(l) = &mut e {
                    l.elevation = level * f;
                }
                ents.push(e);
                count += 1;
            }
        }
    }
    let n = bridge::add_entities(&format!("Hoogtemodel {name}"), ents)?;
    Ok(format!(
        "{name}: hoogtemodel {}×{} cellen van {} m, {} tot {} m — reliëf en {count} hoogtelijnen om de {} m ({n} elementen).",
        grid.w,
        grid.h,
        format!("{}", grid.cell).replace('.', ","),
        crate::hoogte::nap(lo, 2),
        crate::hoogte::nap(hi, 2),
        format!("{interval}").replace('.', ",")
    ))
}

/// Most points an attached scan keeps.
const MAX_SCAN_POINTS: usize = 30_000_000;

/// The CRS of a point cloud file: what the file states, else a `.prj`
/// beside it, else the project's.
pub fn point_file_crs(site: &Site, path: &Path) -> (String, &'static str) {
    match super::pointcloud::file_crs(path) {
        Some(c) => (c, "bestand"),
        None => match prj_crs(path) {
            Some(c) => (c, ".prj"),
            None => (site.crs.clone(), "project"),
        },
    }
}

/// A point cloud file (LAS/LAZ, E57, PLY, PTS, XYZ): points in the project
/// (thinned to at most 30 million), written as RCS beside the drawing and
/// attached. Colours: the file's RGB, else its classes, else intensity,
/// else height.
fn point_cloud(site: &Site, path: &Path, progress: &dyn Fn(String)) -> Result<String, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    let f = t.unit_factor;
    let (crs, crs_from) = point_file_crs(site, path);
    let to = super::vector::data_to_site(site, &crs, &[])?;
    // Thin as the file streams: every `stride`-th point; when the scan is
    // full, every other one is dropped and the stride doubles.
    let mut stride = 1u64;
    let mut seen = 0u64;
    let (mut xyz, mut rgb, mut class, mut inten) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut any_rgb, mut any_class, mut any_intensity) = (false, false, false);
    progress(format!("{} …", path.display()));
    let pass = super::pointcloud::stream_file(path, None, 0.0, progress, &mut |s| {
        seen += 1;
        if !seen.is_multiple_of(stride) {
            return true;
        }
        let Some(q) = to([s.xyz[0], s.xyz[1]]) else { return true };
        let d = site.to_drawing(q, f);
        xyz.push([d[0], d[1], s.xyz[2] * f]);
        any_rgb |= s.rgb.is_some();
        rgb.push(s.rgb.unwrap_or([180, 180, 180]));
        let c = s.class.unwrap_or(0);
        any_class |= c > 1;
        class.push(c);
        any_intensity |= s.intensity > 0;
        inten.push(s.intensity);
        if xyz.len() >= MAX_SCAN_POINTS {
            halve(&mut xyz);
            halve(&mut rgb);
            halve(&mut class);
            halve(&mut inten);
            stride *= 2;
        }
        true
    })?;
    if xyz.is_empty() {
        return Err(crate::lang::tr("geen punten in het bestand", "no points in the file").into());
    }
    let cols: Vec<[u8; 3]> = if any_rgb {
        rgb
    } else if any_class {
        let by_class = |c: u8| super::pointcloud::CLASSES.iter().find(|k| k.0 == c).map(|k| k.2).unwrap_or([160, 160, 160]);
        class.iter().map(|c| by_class(*c)).collect()
    } else if any_intensity {
        inten.iter().map(|i| [(i >> 8) as u8; 3]).collect()
    } else {
        height_colours(&xyz)
    };
    let dir = bridge::image_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("puntenwolk");
    let file: PathBuf = dir.join(format!("{stem}.rcs"));
    progress(format!("{} ({})…", crate::lang::tr("Puntenwolk schrijven", "Writing the point cloud"), xyz.len()));
    super::pointcloud::write_rcs(&file, &xyz, &cols, &inten)?;
    crate::civiel::api::note("punten", xyz.len());
    crate::civiel::api::note("punten_bestand", pass.read);
    crate::civiel::api::note("crs", crs.clone());
    crate::civiel::api::note("rcs", file.to_string_lossy().into_owned());
    let p = file.to_string_lossy().into_owned();
    if p.contains(char::is_whitespace) {
        return Ok(crate::civiel::puntenwolk::trf!(
            "{} punten opgeslagen in {}. Het pad bevat spaties: koppel het bestand met POINTCLOUDATTACH.",
            "{} points saved in {}. The path holds spaces: attach the file with POINTCLOUDATTACH.",
            xyz.len(),
            p
        ));
    }
    bridge::execute(&format!("-POINTCLOUDATTACH {p} 0,0,0 1 0\n"))?;
    let thinned = if stride > 1 { crate::civiel::puntenwolk::trf!(", elke {}e", ", every {}th", stride) } else { String::new() };
    Ok(crate::civiel::puntenwolk::trf!(
        "{}: {} van {} punten ({} uit {}{}) gekoppeld als {}.",
        "{}: {} of {} points ({} from {}{}) attached as {}.",
        stem,
        xyz.len(),
        pass.read,
        crs,
        crate::civiel::puntenwolk::crs_source(crs_from),
        thinned,
        file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ))
}

/// Every other element.
fn halve<T>(v: &mut Vec<T>) {
    let mut i = 0;
    v.retain(|_| {
        i += 1;
        i % 2 == 1
    });
}

/// Colours by height (2nd to 98th percentile), for clouds without colour,
/// class or intensity.
fn height_colours(xyz: &[[f64; 3]]) -> Vec<[u8; 3]> {
    let mut z: Vec<f64> = xyz.iter().map(|p| p[2]).collect();
    z.sort_by(f64::total_cmp);
    let lo = z[z.len() * 2 / 100];
    let hi = z[(z.len() * 98 / 100).min(z.len() - 1)].max(lo + 1e-3);
    let stops = [[40.0, 70.0, 200.0], [40.0, 190.0, 220.0], [70.0, 190.0, 80.0], [240.0, 220.0, 60.0], [220.0, 60.0, 50.0]];
    xyz.iter()
        .map(|p| {
            let t = ((p[2] - lo) / (hi - lo)).clamp(0.0, 1.0) * (stops.len() - 1) as f64;
            let i = (t.floor() as usize).min(stops.len() - 2);
            let u = t - i as f64;
            let mix = |k: usize| (stops[i][k] + (stops[i + 1][k] - stops[i][k]) * u).round() as u8;
            [mix(0), mix(1), mix(2)]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_grids_and_world_files() {
        let g = read_asc("ncols 3\nnrows 2\nxllcorner 100\nyllcorner 200\ncellsize 5\nNODATA_value -9999\n1 2 3\n4 -9999 6\n").unwrap();
        assert_eq!((g.w, g.h, g.cell, g.x0, g.y1), (3, 2, 5.0, 100.0, 210.0));
        assert_eq!(g.at(0, 0), Some(1.0));
        assert_eq!(g.at(1, 1), None);
        let dir = std::env::temp_dir().join(format!("oi-wld-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("foto.png");
        std::fs::write(dir.join("foto.pgw"), "0.5\n0\n0\n-0.5\n100000.25\n400099.75\n").unwrap();
        assert_eq!(world_file(&img, 200, 100), Some([100000.0, 400050.0, 100100.0, 400100.0]));
        std::fs::write(dir.join("foto.prj"), "PROJCS[\"Amersfoort / RD New\",AUTHORITY[\"EPSG\",\"28992\"]]").unwrap();
        assert_eq!(prj_crs(&img).as_deref(), Some("EPSG:28992"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(geokey_epsg(&[1, 1, 0, 2, 1024, 0, 1, 1, 3072, 0, 1, 28992]).as_deref(), Some("EPSG:28992"));
        // A slope lit from the north-west is lighter on its north-west face.
        let mut z = Vec::new();
        for r in 0..5 {
            for c in 0..5 {
                z.push((c + r) as f32);
            }
        }
        let shade = hillshade(&Grid { x0: 0.0, y1: 5.0, cell: 1.0, w: 5, h: 5, z });
        assert_eq!(shade.get_pixel(2, 2)[3], 255);
    }
}
