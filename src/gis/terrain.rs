//! Terrain heights for any project location.
//!
//! In the Netherlands the source is AHN (0.5 m, NAP) from the PDOK WCS. Anywhere
//! else it is the open *Terrain Tiles* dataset on AWS (Mapzen terrarium
//! encoding): a worldwide elevation mosaic of SRTM, GMTED, ETOPO and finer
//! national models where they were open (USGS 3DEP, Canada, Austria, ...),
//! readable without an API key. Grids come out in the same [`Grid`] form, in
//! the CRS the source works in, so contours and profiles do not care where
//! the heights came from.

use std::collections::HashMap;

use super::ahn::{self, Grid, Model, Station};
use super::crs::{self, Transformer, XY};
use super::http;
use super::site::{envelope, Site};
use crate::lang::tr;

pub const TERRARIUM_URL: &str = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";
/// Deepest zoom level of the Terrain Tiles.
const MAX_ZOOM: u32 = 15;
const MAX_TILES: usize = 144;
const EARTH: f64 = 156_543.033_92;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// AHN4 from PDOK, the Netherlands only.
    Ahn,
    /// Terrain Tiles (AWS open data), worldwide.
    Terrarium,
}

/// Where heights for a site come from, and the CRS its grids are in.
#[derive(Clone, Debug)]
pub struct Source {
    pub kind: Kind,
    pub crs: String,
    pub model: Model,
}

impl Source {
    /// AHN inside the Netherlands, Terrain Tiles elsewhere. Grids for
    /// Terrain Tiles are made in the site's CRS (UTM when that is
    /// geographic), so cells are square metres.
    pub fn for_site(site: &Site, model: Model) -> Source {
        if crs::in_netherlands(site.lon, site.lat) {
            return Source { kind: Kind::Ahn, crs: "EPSG:28992".into(), model };
        }
        let crs = if crs::is_geographic(&site.crs) { crs::utm_code_for(site.lon, site.lat) } else { site.crs.clone() };
        Source { kind: Kind::Terrarium, crs, model }
    }

    pub fn is_ahn(&self) -> bool {
        self.kind == Kind::Ahn
    }

    /// The import area in this source's CRS.
    pub fn bbox(&self, site: &Site) -> Result<[f64; 4], String> {
        if site.crs == self.crs {
            Ok(site.bbox())
        } else {
            crs::ensure(&self.crs)?;
            envelope(site.bbox(), &Transformer::new(&site.crs, &self.crs)?)
        }
    }

    /// Short name for object data and messages.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Ahn => format!("AHN4 {}", self.model.coverage()),
            Kind::Terrarium => "Terrain Tiles (SRTM/NASA, USGS 3DEP)".into(),
        }
    }

    /// For the title block's list of sources.
    pub fn attribution(&self) -> &'static str {
        match self.kind {
            Kind::Ahn => "AHN4 (PDOK, CC0)",
            Kind::Terrarium => "Terrain Tiles (AWS Open Data: SRTM, GMTED, ETOPO1, USGS 3DEP and others)",
        }
    }

    /// The height reference: NAP for AHN, (about) mean sea level otherwise.
    pub fn datum(&self) -> &'static str {
        match self.kind {
            Kind::Ahn => "NAP",
            Kind::Terrarium => tr("zeeniveau", "MSL"),
        }
    }

    /// The finest useful cell (m).
    pub fn native_cell(&self) -> f64 {
        match self.kind {
            Kind::Ahn => ahn::NATIVE_CELL,
            // The tiles go to ~3 m pixels, but most of the world is SRTM at 30 m.
            Kind::Terrarium => 5.0,
        }
    }

    /// Heights for `bbox` (in `self.crs`) at `cell` metres.
    pub fn fetch(&self, bbox: [f64; 4], cell: f64, progress: &dyn Fn(String)) -> Result<Grid, String> {
        match self.kind {
            Kind::Ahn => ahn::fetch(bbox, self.model, cell, progress),
            Kind::Terrarium => terrarium_grid(&self.crs, bbox, cell.max(self.native_cell()), progress),
        }
    }

    /// Stations along a route in `self.crs`, as [`ahn::profile`] does.
    pub fn profile(&self, route: &[XY], step: f64, surface: bool, progress: &dyn Fn(String)) -> Result<Vec<Station>, String> {
        match self.kind {
            Kind::Ahn => ahn::profile(route, step, surface, ahn::NATIVE_CELL, progress),
            Kind::Terrarium => {
                if route.len() < 2 {
                    return Err(tr("een profiel heeft minstens twee punten nodig", "a profile needs at least two points").into());
                }
                let to_ll = Transformer::new(&self.crs, "EPSG:4326")?;
                let positions = ahn::stations(route, step.max(1.0));
                let lat = to_ll.apply(route[0]).map(|p| p[1]).unwrap_or(0.0);
                let mut mosaic = Mosaic::new(zoom_for(lat, 10.0));
                let n = positions.len();
                let mut out = Vec::with_capacity(n);
                for (k, (chainage, at)) in positions.into_iter().enumerate() {
                    if k % 200 == 0 {
                        progress(format!("{} {k}/{n}…", tr("Hoogtes langs het profiel:", "Heights along the profile:")));
                    }
                    let ground = to_ll.apply(at).ok().and_then(|ll| mosaic.sample(ll[0], ll[1]));
                    out.push(Station { chainage, at, ground, surface: None });
                }
                if out.iter().all(|s| s.ground.is_none()) {
                    return Err(mosaic.error.unwrap_or_else(|| tr("geen hoogtes langs deze lijn", "no heights along this line").into()));
                }
                Ok(out)
            }
        }
    }
}

/// Heights at single points, tiles fetched as needed and kept: AHN tiles in
/// the Netherlands, Terrain Tiles elsewhere. Positions are in the source's CRS.
pub struct Sampler {
    inner: SamplerKind,
}

enum SamplerKind {
    Ahn(ahn::Sampler),
    World { to_ll: Transformer, mosaic: Mosaic },
}

impl Source {
    /// A point sampler for this source; `lat` picks the tile zoom outside AHN.
    pub fn sampler(&self, lat: f64, cell: f64) -> Result<Sampler, String> {
        Ok(Sampler {
            inner: match self.kind {
                Kind::Ahn => SamplerKind::Ahn(ahn::Sampler::new(self.model, cell)),
                Kind::Terrarium => {
                    crs::ensure(&self.crs)?;
                    SamplerKind::World { to_ll: Transformer::new(&self.crs, "EPSG:4326")?, mosaic: Mosaic::new(zoom_for(lat, cell.max(self.native_cell()))) }
                }
            },
        })
    }
}

impl Sampler {
    pub fn height(&mut self, x: f64, y: f64) -> Option<f64> {
        match &mut self.inner {
            SamplerKind::Ahn(s) => s.height(x, y),
            SamplerKind::World { to_ll, mosaic } => {
                let ll = to_ll.apply([x, y]).ok()?;
                mosaic.sample(ll[0], ll[1])
            }
        }
    }

    pub fn error(&self) -> Option<String> {
        match &self.inner {
            SamplerKind::Ahn(s) => s.error.clone(),
            SamplerKind::World { mosaic, .. } => mosaic.error.clone(),
        }
    }
}

// ── Terrain Tiles (terrarium PNG) ───────────────────────────────────────────

fn ground_resolution(lat: f64, z: u32) -> f64 {
    EARTH * lat.to_radians().cos().abs().max(0.01) / 2f64.powi(z as i32)
}

/// The shallowest zoom whose pixels are at most `cell` metres.
fn zoom_for(lat: f64, cell: f64) -> u32 {
    let mut z = 1;
    while z < MAX_ZOOM && ground_resolution(lat, z) > cell {
        z += 1;
    }
    z
}

/// Global pixel position of a lon/lat at zoom `z` (Web Mercator).
fn pixel(lon: f64, lat: f64, z: u32) -> (f64, f64) {
    let n = 2f64.powi(z as i32) * 256.0;
    let lat = lat.clamp(-85.05, 85.05).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Heights of one terrarium tile: `(R·256 + G + B/256) − 32768` metres.
pub fn decode_terrarium(png: &[u8]) -> Result<Vec<f32>, String> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).map_err(|e| format!("Terrain Tiles: {e}"))?.to_rgb8();
    if img.width() != 256 || img.height() != 256 {
        return Err(format!("Terrain Tiles: tile of {}×{} px", img.width(), img.height()));
    }
    Ok(img.pixels().map(|p| (p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0) - 32768.0).collect())
}

fn tile_url(z: u32, x: u32, y: u32) -> String {
    TERRARIUM_URL.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
}

/// A downloaded tile, or why it could not be had.
type Fetched = ((u32, u32), Result<Vec<f32>, String>);

/// Terrain tiles at one zoom level, fetched when first needed.
struct Mosaic {
    z: u32,
    tiles: HashMap<(u32, u32), Option<Vec<f32>>>,
    error: Option<String>,
}

impl Mosaic {
    fn new(z: u32) -> Self {
        Self { z, tiles: HashMap::new(), error: None }
    }

    fn load(&mut self, keys: &[(u32, u32)], progress: &dyn Fn(String)) {
        let todo: Vec<(u32, u32)> = keys.iter().copied().filter(|k| !self.tiles.contains_key(k)).collect();
        let z = self.z;
        let total = todo.len();
        // A few at a time: S3 is quick, the latency is what counts.
        for (i, chunk) in todo.chunks(8).enumerate() {
            progress(format!("{} {}/{}…", tr("Hoogtetegels ophalen", "Fetching elevation tiles"), (i * 8 + chunk.len()).min(total), total));
            let got: Vec<Fetched> = std::thread::scope(|s| {
                let handles: Vec<_> = chunk.iter().map(|&(x, y)| s.spawn(move || ((x, y), http::get(&tile_url(z, x, y)).and_then(|f| decode_terrarium(&f.body))))).collect();
                handles.into_iter().filter_map(|h| h.join().ok()).collect()
            });
            for (k, r) in got {
                match r {
                    Ok(v) => {
                        self.tiles.insert(k, Some(v));
                    }
                    Err(e) => {
                        self.error.get_or_insert(e);
                        self.tiles.insert(k, None);
                    }
                }
            }
        }
    }

    fn value(&mut self, gx: i64, gy: i64) -> Option<f64> {
        let n = 1i64 << self.z;
        let tx = gx.div_euclid(256).rem_euclid(n) as u32;
        let ty = gy.div_euclid(256);
        if ty < 0 || ty >= n {
            return None;
        }
        let key = (tx, ty as u32);
        if !self.tiles.contains_key(&key) {
            self.load(&[key], &|_| {});
        }
        let t = self.tiles.get(&key)?.as_ref()?;
        let v = t[(gy.rem_euclid(256) * 256 + gx.rem_euclid(256)) as usize];
        // Missing data is coded far below any real height.
        (v > -12_000.0).then_some(v as f64)
    }

    /// Bilinear height at a lon/lat (pixel centres at +0.5).
    fn sample(&mut self, lon: f64, lat: f64) -> Option<f64> {
        let (px, py) = pixel(lon, lat, self.z);
        let (fx, fy) = (px - 0.5, py - 0.5);
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let a = self.value(x0, y0)?;
        let b = self.value(x0 + 1, y0).unwrap_or(a);
        let c = self.value(x0, y0 + 1).unwrap_or(a);
        let d = self.value(x0 + 1, y0 + 1).unwrap_or(a);
        let top = a + (b - a) * tx;
        let bottom = c + (d - c) * tx;
        Some(top + (bottom - top) * ty)
    }
}

/// A height grid in `crs` over `bbox`, resampled from Terrain Tiles.
fn terrarium_grid(crs_code: &str, bbox: [f64; 4], cell: f64, progress: &dyn Fn(String)) -> Result<Grid, String> {
    crs::ensure(crs_code)?;
    let to_ll = Transformer::new(crs_code, "EPSG:4326")?;
    let ll = envelope(bbox, &to_ll)?;
    let lat = (ll[1] + ll[3]) / 2.0;
    // Pick the zoom for the cell, then back off until the tile count is sane.
    let mut z = zoom_for(lat, cell);
    let tiles_at = |z: u32| {
        let (x0, y0) = pixel(ll[0], ll[3], z);
        let (x1, y1) = pixel(ll[2], ll[1], z);
        let (a, b) = ((x0 / 256.0).floor() as i64 - 1, (y0 / 256.0).floor() as i64 - 1);
        let (c, d) = ((x1 / 256.0).floor() as i64 + 1, (y1 / 256.0).floor() as i64 + 1);
        let n = 1i64 << z;
        let mut keys = Vec::new();
        for ty in b.max(0)..=d.min(n - 1) {
            for tx in a..=c {
                keys.push((tx.rem_euclid(n) as u32, ty as u32));
            }
        }
        keys.sort_unstable();
        keys.dedup();
        keys
    };
    let mut keys = tiles_at(z);
    while keys.len() > MAX_TILES && z > 1 {
        z -= 1;
        keys = tiles_at(z);
    }
    let cell = cell.max(ground_resolution(lat, z) * 0.75);
    let snap = |v: f64, up: bool| if up { (v / cell).ceil() * cell } else { (v / cell).floor() * cell };
    let (x0, y0, x1, y1) = (snap(bbox[0], false), snap(bbox[1], false), snap(bbox[2], true), snap(bbox[3], true));
    let (w, h) = (((x1 - x0) / cell).round() as usize, ((y1 - y0) / cell).round() as usize);
    if w == 0 || h == 0 {
        return Err(tr("leeg gebied", "empty area").into());
    }
    if w * h > 9_000_000 {
        return Err(format!("{w} × {h} {}", tr("cellen is te veel; kies een grovere celgrootte of een kleiner gebied", "cells is too many; choose a coarser cell or a smaller area")));
    }
    let mut mosaic = Mosaic::new(z);
    mosaic.load(&keys, progress);
    if mosaic.tiles.values().all(|t| t.is_none()) {
        return Err(mosaic.error.unwrap_or_else(|| tr("geen hoogtetegels ontvangen", "no elevation tiles received").into()));
    }
    progress(tr("Hoogtes herbemonsteren…", "Resampling heights…").into());
    let mut z_out = vec![f32::NAN; w * h];
    for r in 0..h {
        let y = y1 - (r as f64 + 0.5) * cell;
        for c in 0..w {
            let x = x0 + (c as f64 + 0.5) * cell;
            if let Ok(p) = to_ll.apply([x, y]) {
                if let Some(v) = mosaic.sample(p[0], p[1]) {
                    z_out[r * w + c] = v as f32;
                }
            }
        }
    }
    Ok(Grid { x0, y1, cell, w, h, z: z_out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrarium_decoding() {
        // R=128, G=0, B=0 is 0 m; one step of G is a metre.
        let mut img = image::RgbImage::new(256, 256);
        for p in img.pixels_mut() {
            *p = image::Rgb([128, 10, 128]);
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let v = decode_terrarium(&png).unwrap();
        assert!((v[0] - 10.5).abs() < 1e-3, "{}", v[0]);
    }

    #[test]
    fn zoom_and_pixels() {
        assert!(ground_resolution(0.0, 15) < 5.0);
        assert_eq!(zoom_for(52.0, 30.0), 12);
        let (x, y) = pixel(0.0, 0.0, 1);
        assert!((x - 256.0).abs() < 1e-9 && (y - 256.0).abs() < 1e-9);
    }

    #[test]
    fn source_choice() {
        let mut s = Site::default();
        s.lon = 4.66;
        s.lat = 51.81;
        assert!(Source::for_site(&s, Model::Dtm).is_ahn());
        s.lon = 8.54;
        s.lat = 47.37;
        s.crs = "EPSG:4326".into();
        let src = Source::for_site(&s, Model::Dtm);
        assert_eq!(src.kind, Kind::Terrarium);
        assert_eq!(src.crs, "EPSG:32632");
    }
}
