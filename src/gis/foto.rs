//! Aerial and satellite photos for any project location, as one image over a
//! box in any CRS: to colour a terrain surface or drape over a 3D model.
//!
//! - the Netherlands: PDOK's aerial photo (8 cm, CC BY 4.0);
//! - the United States: the USGS National Map imagery (public domain);
//! - elsewhere: Sentinel-2 cloudless by EOX (10 m). The 2016 mosaic is
//!   CC BY 4.0; the later years are CC BY-NC-SA 4.0 (non-commercial) and only
//!   used when asked for by year.

use image::RgbaImage;

use super::crs::{self, Transformer};
use super::http;
use super::site::envelope;
use crate::lang::tr;

/// The largest photo assembled for one command (pixels).
const MAX_PIXELS: f64 = 64e6;
/// Pixels per GetMap request side.
const TILE: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Photo {
    Pdok,
    Usgs,
    /// EOX Sentinel-2 cloudless; `None` is the 2016 mosaic (CC BY 4.0).
    Sentinel2(Option<u32>),
}

/// The lower 48 by a coarse outline (lon, lat): the 49th parallel, the Great
/// Lakes and Maine against Canada, the Rio Grande and the line to Tijuana
/// against Mexico, out to sea elsewhere. A few km off along the borders.
const LOWER_48: &[[f64; 2]] = &[
    [-124.8, 48.4], [-123.3, 49.0], [-95.2, 49.0], [-89.6, 48.0], [-84.6, 46.5], [-82.4, 45.3], [-82.5, 42.6],
    [-83.1, 42.0], [-78.9, 42.9], [-79.2, 43.4], [-76.4, 44.1], [-74.8, 45.0], [-71.5, 45.0], [-71.1, 45.3],
    [-70.0, 46.7], [-69.2, 47.45], [-68.2, 47.35], [-67.8, 47.07], [-67.8, 45.7], [-67.0, 44.9], [-66.0, 44.5],
    [-75.0, 35.0], [-80.0, 24.0], [-82.0, 24.3], [-97.1, 25.9], [-99.1, 26.4], [-100.3, 28.3], [-101.4, 29.8],
    [-102.7, 29.7], [-103.2, 28.97], [-104.4, 29.6], [-106.2, 31.45], [-106.45, 31.75], [-106.53, 31.78], [-108.2, 31.78], [-108.2, 31.33], [-111.07, 31.33],
    [-114.8, 32.5], [-117.1, 32.5], [-118.0, 32.0], [-125.5, 40.0],
];

/// The United States (lower 48, Alaska, Hawaii, Puerto Rico), coarsely.
pub fn in_united_states(lon: f64, lat: f64) -> bool {
    const BOXES: [[f64; 4]; 4] = [
        // Alaska west of the Yukon border, and the panhandle.
        [-170.0, 51.0, -141.0, 71.5],
        [-141.0, 54.6, -130.0, 60.3],
        [-160.5, 18.8, -154.7, 22.4],
        [-67.3, 17.8, -65.2, 18.6],
    ];
    crs::in_polygon(lon, lat, LOWER_48) || BOXES.iter().any(|b| lon >= b[0] && lon <= b[2] && lat >= b[1] && lat <= b[3])
}

impl Photo {
    /// The best open photo for a location.
    pub fn for_location(lon: f64, lat: f64) -> Photo {
        if crs::in_netherlands(lon, lat) {
            Photo::Pdok
        } else if in_united_states(lon, lat) {
            Photo::Usgs
        } else {
            Photo::Sentinel2(None)
        }
    }

    /// `FOTO=` of a command: `auto` (or empty), `pdok`, `usgs`, `sentinel2`
    /// or `sentinel2-<year>` (2017–2025).
    pub fn choose(choice: Option<&str>, lon: f64, lat: f64) -> Result<Photo, String> {
        let c = choice.unwrap_or("").trim().to_ascii_lowercase();
        let photo = match c.as_str() {
            "" | "auto" => return Ok(Photo::for_location(lon, lat)),
            "pdok" | "nl" => Photo::Pdok,
            "usgs" | "us" | "naip" => Photo::Usgs,
            "sentinel2" | "sentinel" | "s2" | "eox" => Photo::Sentinel2(None),
            other => match other.strip_prefix("sentinel2-").or(other.strip_prefix("s2-")).and_then(|y| y.parse::<u32>().ok()) {
                Some(y) if (2017..=2025).contains(&y) => Photo::Sentinel2(Some(y)),
                Some(2016) => Photo::Sentinel2(None),
                _ => {
                    return Err(format!(
                        "{} FOTO={other}: auto, pdok, usgs, sentinel2 {} sentinel2-2017 … sentinel2-2025",
                        tr("onbekende luchtfoto", "unknown photo"),
                        tr("of", "or")
                    ))
                }
            },
        };
        match photo {
            Photo::Pdok if !crs::in_netherlands(lon, lat) => Err(tr("de PDOK-luchtfoto dekt alleen Nederland", "the PDOK aerial photo covers the Netherlands only").into()),
            Photo::Usgs if !in_united_states(lon, lat) => Err(tr("de USGS-luchtfoto dekt alleen de Verenigde Staten", "the USGS photo covers the United States only").into()),
            p => Ok(p),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Photo::Pdok => tr("luchtfoto PDOK", "PDOK aerial photo").into(),
            Photo::Usgs => tr("luchtfoto USGS", "USGS aerial photo").into(),
            Photo::Sentinel2(None) => "Sentinel-2 cloudless 2016".into(),
            Photo::Sentinel2(Some(y)) => format!("Sentinel-2 cloudless {y}"),
        }
    }

    /// For the title block's list of sources.
    pub fn attribution(&self) -> String {
        match self {
            Photo::Pdok => "Luchtfoto (Beeldmateriaal.nl / PDOK, CC BY 4.0)".into(),
            Photo::Usgs => "USGS The National Map: Imagery (public domain)".into(),
            Photo::Sentinel2(None) => "Sentinel-2 cloudless 2016 by EOX IT Services GmbH (contains modified Copernicus Sentinel data 2016), CC BY 4.0".into(),
            Photo::Sentinel2(Some(y)) => format!("Sentinel-2 cloudless {y} by EOX IT Services GmbH (contains modified Copernicus Sentinel data {y}), CC BY-NC-SA 4.0"),
        }
    }

    /// A warning that belongs with the result, if any.
    pub fn licence_note(&self) -> Option<&'static str> {
        matches!(self, Photo::Sentinel2(Some(_))).then(|| {
            tr(
                "Let op: deze Sentinel-2-jaargang is alleen voor niet-commercieel gebruik (CC BY-NC-SA 4.0); FOTO=sentinel2 is de vrij bruikbare van 2016.",
                "Note: this Sentinel-2 year is for non-commercial use only (CC BY-NC-SA 4.0); FOTO=sentinel2 is the freely usable 2016 mosaic.",
            )
        })
    }

    /// Finest useful pixel (m): finer requests only cost time.
    pub fn finest(&self) -> f64 {
        match self {
            Photo::Pdok => 0.08,
            Photo::Usgs => 0.5,
            Photo::Sentinel2(_) => 5.0,
        }
    }

    /// The CRS the service is asked in.
    fn native_crs(&self) -> &'static str {
        match self {
            Photo::Pdok => "EPSG:28992",
            _ => "EPSG:3857",
        }
    }

    /// A GetMap URL for `bbox` (in [`Self::native_crs`]).
    pub fn get_map_url(&self, bbox: [f64; 4], w: u32, h: u32) -> String {
        let (base, layer) = match self {
            Photo::Pdok => ("https://service.pdok.nl/hwh/luchtfotorgb/wms/v1_0", "Actueel_orthoHR".to_string()),
            Photo::Usgs => ("https://basemap.nationalmap.gov/arcgis/services/USGSImageryOnly/MapServer/WMSServer", "0".to_string()),
            Photo::Sentinel2(None) => ("https://tiles.maps.eox.at/wms", "s2cloudless_3857".to_string()),
            Photo::Sentinel2(Some(y)) => ("https://tiles.maps.eox.at/wms", format!("s2cloudless-{y}_3857")),
        };
        http::with_params(
            base,
            &[
                ("SERVICE", "WMS".into()),
                ("VERSION", "1.3.0".into()),
                ("REQUEST", "GetMap".into()),
                ("LAYERS", layer),
                ("STYLES", String::new()),
                ("CRS", self.native_crs().into()),
                ("BBOX", format!("{},{},{},{}", bbox[0], bbox[1], bbox[2], bbox[3])),
                ("WIDTH", w.to_string()),
                ("HEIGHT", h.to_string()),
                ("FORMAT", "image/jpeg".into()),
            ],
        )
    }

    /// The photo over `bbox` in `crs` at about `px` m a pixel. Returns the
    /// image (pixel (0, 0) at the top left of `bbox`) and the pixel size it
    /// has, which is coarser than `px` when the area would be too large.
    pub fn image(&self, crs_code: &str, bbox: [f64; 4], px: f64, progress: &dyn Fn(String)) -> Result<(RgbaImage, f64), String> {
        let (bw, bh) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
        let mut px = px.max(self.finest() / 2.0);
        if bw * bh / (px * px) > MAX_PIXELS {
            px = (bw * bh / MAX_PIXELS).sqrt();
        }
        let (pw, ph) = ((bw / px).ceil().max(1.0) as u32, (bh / px).ceil().max(1.0) as u32);
        let native = self.native_crs();
        if crs_code == native {
            return Ok((self.mosaic([bbox[0], bbox[3] - ph as f64 * px, bbox[0] + pw as f64 * px, bbox[3]], pw, ph, progress)?, px));
        }
        crs::ensure(crs_code)?;
        let out_box = [bbox[0], bbox[3] - ph as f64 * px, bbox[0] + pw as f64 * px, bbox[3]];
        let env = envelope(out_box, &Transformer::new(crs_code, native)?)?;
        // Same pixel count over the native box as over ours, a little margin.
        let scale = ((env[2] - env[0]) / (out_box[2] - out_box[0])).max((env[3] - env[1]) / (out_box[3] - out_box[1]));
        let npx = px * scale;
        let (nw, nh) = (((env[2] - env[0]) / npx).ceil().max(1.0) as u32, ((env[3] - env[1]) / npx).ceil().max(1.0) as u32);
        let nbox = [env[0], env[3] - nh as f64 * npx, env[0] + nw as f64 * npx, env[3]];
        let canvas = self.mosaic(nbox, nw, nh, progress)?;
        progress(tr("Luchtfoto herprojecteren…", "Reprojecting the photo…").into());
        Ok((super::raster::warp(&canvas, native, nbox, crs_code, out_box, pw, ph)?, px))
    }

    /// `w`×`h` pixels over `bbox` (native CRS), in requests the service accepts.
    fn mosaic(&self, bbox: [f64; 4], w: u32, h: u32, progress: &dyn Fn(String)) -> Result<RgbaImage, String> {
        let (sx, sy) = ((bbox[2] - bbox[0]) / w as f64, (bbox[3] - bbox[1]) / h as f64);
        let mut photo = RgbaImage::new(w, h);
        let (tx, ty) = (w.div_ceil(TILE), h.div_ceil(TILE));
        for j in 0..ty {
            for i in 0..tx {
                progress(format!("{} {}/{}…", self.label(), j * tx + i + 1, tx * ty));
                let (x0, y0) = (i * TILE, j * TILE);
                let (wp, hp) = ((w - x0).min(TILE), (h - y0).min(TILE));
                let bb = [bbox[0] + x0 as f64 * sx, bbox[3] - (y0 + hp) as f64 * sy, bbox[0] + (x0 + wp) as f64 * sx, bbox[3] - y0 as f64 * sy];
                let bytes = http::get_image(&self.get_map_url(bb, wp, hp))?;
                let img = image::load_from_memory(&bytes)
                    .map(|i| i.to_rgba8())
                    .map_err(|e| format!("{} {}: {e}", self.label(), tr("onleesbaar", "unreadable")))?;
                image::imageops::replace(&mut photo, &img, x0 as i64, y0 as i64);
            }
        }
        Ok(photo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_per_location() {
        assert_eq!(Photo::for_location(4.67, 51.81), Photo::Pdok, "Dordrecht");
        assert_eq!(Photo::for_location(-119.53, 37.75), Photo::Usgs, "Half Dome");
        assert_eq!(Photo::for_location(-149.9, 61.2), Photo::Usgs, "Anchorage");
        assert_eq!(Photo::for_location(7.66, 45.98), Photo::Sentinel2(None), "Matterhorn");
        assert_eq!(Photo::for_location(-73.57, 45.50), Photo::Sentinel2(None), "Montréal");
        assert_eq!(Photo::for_location(-79.38, 43.65), Photo::Sentinel2(None), "Toronto");
        assert_eq!(Photo::for_location(-123.12, 49.28), Photo::Sentinel2(None), "Vancouver");
        assert_eq!(Photo::for_location(-99.13, 19.43), Photo::Sentinel2(None), "Mexico City");
        assert_eq!(Photo::for_location(-106.42, 31.69), Photo::Sentinel2(None), "Ciudad Juárez");
        for (lon, lat, name) in [(-74.0, 40.7, "New York"), (-87.63, 41.88, "Chicago"), (-83.05, 42.33, "Detroit"), (-122.33, 47.61, "Seattle"),
            (-117.16, 32.72, "San Diego"), (-80.19, 25.76, "Miami"), (-106.44, 31.80, "El Paso"), (-68.77, 44.80, "Bangor"),
            (-134.42, 58.30, "Juneau"), (-157.86, 21.31, "Honolulu"), (-66.1, 18.47, "San Juan")] {
            assert_eq!(Photo::for_location(lon, lat), Photo::Usgs, "{name}");
        }
    }

    #[test]
    fn choice_by_name_and_year() {
        assert_eq!(Photo::choose(Some("sentinel2-2025"), 7.66, 45.98).unwrap(), Photo::Sentinel2(Some(2025)));
        assert_eq!(Photo::choose(Some("S2"), -119.5, 37.7).unwrap(), Photo::Sentinel2(None));
        assert_eq!(Photo::choose(Some("auto"), -119.5, 37.7).unwrap(), Photo::Usgs);
        assert!(Photo::choose(Some("pdok"), 7.66, 45.98).is_err());
        assert!(Photo::choose(Some("usgs"), 7.66, 45.98).is_err());
        assert!(Photo::choose(Some("sentinel2-2010"), 7.66, 45.98).is_err());
        assert!(Photo::choose(Some("google"), 7.66, 45.98).is_err());
    }

    #[test]
    fn only_later_sentinel_years_are_non_commercial() {
        assert!(Photo::Sentinel2(None).licence_note().is_none());
        assert!(Photo::Sentinel2(None).attribution().contains("CC BY 4.0"));
        assert!(Photo::Sentinel2(Some(2024)).attribution().contains("NC"));
        assert!(Photo::Sentinel2(Some(2024)).licence_note().is_some());
        assert!(Photo::Usgs.licence_note().is_none());
    }

    #[test]
    fn get_map_urls() {
        let u = Photo::Usgs.get_map_url([1.0, 2.0, 3.0, 4.0], 10, 20);
        assert!(u.starts_with("https://basemap.nationalmap.gov/") && u.contains("3857"), "{u}");
        assert!(u.contains("BBOX=1") && u.contains("WIDTH=10") && u.contains("HEIGHT=20"));
        assert!(Photo::Sentinel2(Some(2023)).get_map_url([0.0; 4], 1, 1).contains("s2cloudless-2023_3857"));
        assert!(Photo::Sentinel2(None).get_map_url([0.0; 4], 1, 1).contains("LAYERS=s2cloudless_3857"));
        assert!(Photo::Pdok.get_map_url([0.0; 4], 1, 1).contains("28992"));
    }
}
