//! Project site — the GIS location every import works from, comparable to the
//! Site object of the FreeCAD GIS2BIM workbench. It is stored with the drawing.
//!
//! Drawing coordinates: X east, Y north. `unit_factor` is drawing units per
//! metre (1 for a drawing in metres, 1000 in millimetres).

use serde::{Deserialize, Serialize};

use super::crs::{self, XY};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Site {
    /// Project jurisdiction, independent of interface language and coordinate system.
    pub region: crate::localisation::Region,
    pub label: String,
    pub lon: f64,
    pub lat: f64,
    pub crs: String,
    /// Proj definition of `crs`, kept so the drawing reopens without epsg.io.
    pub crs_proj4: String,
    pub crs_name: String,
    /// Base point in the project CRS (metres).
    pub origin_e: f64,
    pub origin_n: f64,
    /// Import extent around the base point (metres).
    pub width: f64,
    pub height: f64,
    /// true: CRS coordinates in the drawing (e.g. RD); false: base point = 0,0.
    pub absolute: bool,
}

impl Default for Site {
    fn default() -> Self {
        let mut s = Site {
            region: crate::localisation::Region::Netherlands,
            // Grote Kerk (Onze-Lieve-Vrouwekerk), Dordrecht — building centre in
            // OSM; Lange Geldersekade is also the default
            // of the FreeCAD GIS2BIM location dialog.
            label: "Grote Kerk, Dordrecht".into(),
            lon: 4.660_612,
            lat: 51.814_158,
            crs: "EPSG:28992".into(),
            crs_proj4: String::new(),
            crs_name: String::new(),
            origin_e: 0.0,
            origin_n: 0.0,
            width: 500.0,
            height: 500.0,
            absolute: true,
        };
        s.reproject_origin();
        s
    }
}

impl Site {
    /// Defaults for a new drawing only; never changes an existing drawing.
    pub fn for_region(region:crate::localisation::Region)->Self {
        let mut site=Self::default();site.region=region;
        if region!=crate::localisation::Region::Netherlands {
            let (lon,lat)=if region==crate::localisation::Region::UnitedStates {(-77.0365,38.8977)}else{(0.,0.)};
            site.lon=lon;site.lat=lat;site.label="Project location".into();
            site.crs=super::crs::suggest(lon,lat);site.reproject_origin();
        }
        site
    }
    /// Recompute the base point from lon/lat in the current CRS.
    pub fn reproject_origin(&mut self) {
        if let Some(info) = crs::known(&self.crs) {
            self.crs_proj4 = info.proj4;
            self.crs_name = info.name;
        }
        if let Ok([e, n]) = crs::transform("EPSG:4326", &self.crs, [self.lon, self.lat]) {
            self.origin_e = (e * 1000.0).round() / 1000.0;
            self.origin_n = (n * 1000.0).round() / 1000.0;
        }
    }

    /// Set the base point in the project CRS and update lon/lat.
    pub fn set_origin(&mut self, e: f64, n: f64) {
        self.origin_e = e;
        self.origin_n = n;
        if let Ok([lon, lat]) = crs::transform(&self.crs, "EPSG:4326", [e, n]) {
            self.lon = lon;
            self.lat = lat;
        }
    }

    /// Make the stored CRS definition known to the transformer again.
    pub fn register_crs(&self) {
        if !self.crs_proj4.is_empty() {
            crs::register(crs::CrsInfo {
                code: self.crs.clone(),
                name: if self.crs_name.is_empty() { self.crs.clone() } else { self.crs_name.clone() },
                region: "tekening".into(),
                proj4: self.crs_proj4.clone(),
            });
        }
    }

    /// Import extent in the project CRS: [min_e, min_n, max_e, max_n].
    pub fn bbox(&self) -> [f64; 4] {
        [
            self.origin_e - self.width / 2.0,
            self.origin_n - self.height / 2.0,
            self.origin_e + self.width / 2.0,
            self.origin_n + self.height / 2.0,
        ]
    }

    /// Envelope of the import extent as lon/lat [w, s, e, n].
    pub fn bbox_lonlat(&self) -> Result<[f64; 4], String> {
        envelope(self.bbox(), &crs::Transformer::new(&self.crs, "EPSG:4326")?)
    }

    fn offset(&self) -> XY {
        if self.absolute { [0.0, 0.0] } else { [self.origin_e, self.origin_n] }
    }

    /// Project CRS (m) → drawing units.
    pub fn to_drawing(&self, p: XY, unit_factor: f64) -> XY {
        let o = self.offset();
        [(p[0] - o[0]) * unit_factor, (p[1] - o[1]) * unit_factor]
    }

    /// Drawing units → project CRS (m).
    pub fn from_drawing(&self, p: XY, unit_factor: f64) -> XY {
        let o = self.offset();
        [p[0] / unit_factor + o[0], p[1] / unit_factor + o[1]]
    }

    /// Import extent in drawing units [min_x, min_y, max_x, max_y].
    pub fn drawing_rect(&self, unit_factor: f64) -> [f64; 4] {
        let [a, b, c, d] = self.bbox();
        let p = self.to_drawing([a, b], unit_factor);
        let q = self.to_drawing([c, d], unit_factor);
        [p[0], p[1], q[0], q[1]]
    }
}

/// Envelope of a bbox after transformation (edges sampled, so curvature is covered).
pub fn envelope(b: [f64; 4], t: &crs::Transformer) -> Result<[f64; 4], String> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    const N: usize = 8;
    for i in 0..=N {
        let f = i as f64 / N as f64;
        for p in [
            [b[0] + (b[2] - b[0]) * f, b[1]],
            [b[0] + (b[2] - b[0]) * f, b[3]],
            [b[0], b[1] + (b[3] - b[1]) * f],
            [b[2], b[1] + (b[3] - b[1]) * f],
        ] {
            let q = t.apply(p)?;
            minx = minx.min(q[0]);
            miny = miny.min(q[1]);
            maxx = maxx.max(q[0]);
            maxy = maxy.max(q[1]);
        }
    }
    Ok([minx, miny, maxx, maxy])
}

/// Drawing units per metre for a DWG `INSUNITS` value (unitless → metres).
pub fn unit_factor_for_insunits(insunits: i64) -> f64 {
    match insunits {
        4 => 1000.0, // mm
        5 => 100.0,  // cm
        14 => 10.0,  // dm
        7 => 0.001,  // km
        1 => 1.0 / 0.0254,
        2 => 1.0 / 0.3048,
        21 => 3937.0 / 1200.0, // historical U.S. survey foot
        _ => 1.0,    // 6 = m, 0 = unitless (civil drawings are metres)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_survey_foot_is_not_the_international_foot() {
        assert!((unit_factor_for_insunits(21)-3937./1200.).abs()<1e-12);
        assert!((unit_factor_for_insunits(21)-unit_factor_for_insunits(2)).abs()>0.000001);
    }
    #[test]
    fn region_persists_without_changing_geometry_and_old_sites_remain_readable() {
        let mut site=Site::default();let bbox=site.bbox();let origin=[site.origin_e,site.origin_n];
        site.region=crate::localisation::Region::UnitedStates;
        let encoded=serde_json::to_value(&site).unwrap();
        let read:Site=serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(read.region,crate::localisation::Region::UnitedStates);assert_eq!(read.bbox(),bbox);assert_eq!([read.origin_e,read.origin_n],origin);
        let mut old=encoded;old.as_object_mut().unwrap().remove("region");
        assert_eq!(serde_json::from_value::<Site>(old).unwrap().region,crate::localisation::Region::Netherlands);
        let mut future=serde_json::to_value(&site).unwrap();future["region"]="future-region".into();
        let read:Site=serde_json::from_value(future).unwrap();assert_eq!(read.region,crate::localisation::Region::International);assert_eq!(read.bbox(),bbox);
        let us=Site::for_region(crate::localisation::Region::UnitedStates);assert_eq!(us.crs,"EPSG:32618");assert!(us.lon< -70.);
    }

    #[test]
    fn default_site_is_dordrecht_grote_kerk() {
        let s = Site::default();
        assert_eq!(s.crs, "EPSG:28992");
        assert!((s.origin_e - 104_897.5).abs() < 1.0 && (s.origin_n - 425_309.8).abs() < 1.0, "{} {}", s.origin_e, s.origin_n);
    }

    #[test]
    fn relative_and_absolute_mapping() {
        let mut s = Site::default();
        s.set_origin(136_100.0, 455_900.0);
        s.absolute = true;
        assert_eq!(s.to_drawing([136_110.0, 455_905.0], 1.0), [136_110.0, 455_905.0]);
        s.absolute = false;
        assert_eq!(s.to_drawing([136_110.0, 455_905.0], 1000.0), [10_000.0, 5_000.0]);
        assert_eq!(s.from_drawing([10_000.0, 5_000.0], 1000.0), [136_110.0, 455_905.0]);
    }
}
