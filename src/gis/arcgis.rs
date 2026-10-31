//! ArcGIS tile caches (`…/MapServer` with a `tileInfo`): tiles in the
//! cache's own CRS and levels, e.g. Topotijdreis (Kadaster, every
//! topographic map since 1815, in RD). Used by the XYZ import and preview
//! when a tile server URL has no `{z}/{x}/{y}` template.

use image::RgbaImage;

use super::http;
use super::site::{envelope, Site};
use crate::gis::crs::Transformer;

/// True for an ArcGIS tile cache URL (no XYZ template).
pub fn is_tile_cache(url: &str) -> bool {
    (url.contains("/MapServer") || url.contains("/ImageServer")) && !url.contains("{z}")
}

struct Cache {
    base: String,
    crs: String,
    origin: [f64; 2],
    tile: [u32; 2],
    /// (level, metres per pixel), coarse to fine.
    lods: Vec<(u32, f64)>,
}

fn cache(url: &str) -> Result<Cache, String> {
    let base = url.split('?').next().unwrap_or(url).trim_end_matches('/').to_string();
    let j = http::get_json(&format!("{base}?f=json"))?;
    let t = &j["tileInfo"];
    let wkid = t["spatialReference"]["latestWkid"].as_u64().or_else(|| t["spatialReference"]["wkid"].as_u64()).ok_or("ArcGIS: geen tegelcache (tileInfo ontbreekt)")?;
    let crs = match wkid {
        102100 | 102113 | 900913 => "EPSG:3857".to_string(),
        w => format!("EPSG:{w}"),
    };
    let lods: Vec<(u32, f64)> = t["lods"].as_array().into_iter().flatten().filter_map(|l| Some((l["level"].as_u64()? as u32, l["resolution"].as_f64()?))).collect();
    if lods.is_empty() {
        return Err("ArcGIS: tegelcache zonder niveaus".into());
    }
    Ok(Cache {
        base,
        crs,
        origin: [t["origin"]["x"].as_f64().unwrap_or(0.0), t["origin"]["y"].as_f64().unwrap_or(0.0)],
        tile: [t["cols"].as_u64().unwrap_or(256) as u32, t["rows"].as_u64().unwrap_or(256) as u32],
        lods,
    })
}

/// The tiles covering the site at about `resolution` m/px (at most
/// `max_tiles`), stitched: the image, its CRS and its bounding box there.
pub fn fetch(site: &Site, url: &str, resolution: f64, max_tiles: u32, progress: &dyn Fn(String)) -> Result<(RgbaImage, String, [f64; 4]), String> {
    let c = cache(url)?;
    let b = envelope(site.bbox(), &Transformer::new(&site.crs, &c.crs)?)?;
    // The coarsest level at least as fine as asked, else the finest.
    let mut i = c.lods.iter().position(|(_, r)| *r <= resolution).unwrap_or(c.lods.len() - 1);
    let range = |res: f64| {
        let (tw, th) = (c.tile[0] as f64 * res, c.tile[1] as f64 * res);
        let c0 = ((b[0] - c.origin[0]) / tw).floor() as i64;
        let c1 = ((b[2] - c.origin[0]) / tw).floor() as i64;
        let r0 = ((c.origin[1] - b[3]) / th).floor() as i64;
        let r1 = ((c.origin[1] - b[1]) / th).floor() as i64;
        (tw, th, c0, c1, r0, r1)
    };
    let mut r = range(c.lods[i].1);
    while i > 0 && ((r.3 - r.2 + 1) * (r.5 - r.4 + 1)) as u32 > max_tiles {
        i -= 1;
        r = range(c.lods[i].1);
    }
    let (level, _) = c.lods[i];
    let (tw, th, c0, c1, r0, r1) = r;
    let (nx, ny) = ((c1 - c0 + 1) as u32, (r1 - r0 + 1) as u32);
    let mut canvas = RgbaImage::new(nx * c.tile[0], ny * c.tile[1]);
    let total = nx * ny;
    let mut done = 0;
    for row in r0..=r1 {
        for col in c0..=c1 {
            done += 1;
            progress(format!("Tegels ophalen {done}/{total} (niveau {level})…"));
            // Tiles outside the map are missing: leave them transparent.
            if let Ok(bytes) = http::get_image(&format!("{}/tile/{level}/{row}/{col}", c.base)) {
                if let Ok(t) = image::load_from_memory(&bytes) {
                    image::imageops::overlay(&mut canvas, &t.to_rgba8(), (col - c0) * c.tile[0] as i64, (row - r0) * c.tile[1] as i64);
                }
            }
        }
    }
    let bbox = [c.origin[0] + c0 as f64 * tw, c.origin[1] - (r1 + 1) as f64 * th, c.origin[0] + (c1 + 1) as f64 * tw, c.origin[1] - r0 as f64 * th];
    Ok((canvas, c.crs, bbox))
}

#[cfg(test)]
mod tests {
    #[test]
    fn tile_cache_urls() {
        assert!(super::is_tile_cache("https://tiles.arcgis.com/tiles/nSZVuSZjHpEZZbRo/arcgis/rest/services/Historische_tijdreis_1900/MapServer"));
        assert!(!super::is_tile_cache("https://tile.openstreetmap.org/{z}/{x}/{y}.png"));
    }
}
