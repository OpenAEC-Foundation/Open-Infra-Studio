//! The drawing's own geographic location: the GEODATA object that other CAD
//! programs write (GEOGRAPHICLOCATION, GEOMAP). Open Infra
//! reads it to set the project site of a drawing that has none yet.
//!
//! Open CAD Studio's plugin API cannot create objects other than XRecords,
//! so Open Infra cannot write GEODATA yet; it keeps its site in the
//! `OPENINFRA/SITE` XRecord.

use super::crs;
use super::files::epsg_from_wkt;
use super::site::Site;

/// MapGuide coordinate system names in DWG GEODATA, with their EPSG codes.
const MAPGUIDE: &[(&str, &str)] = &[
    ("LL84", "EPSG:4326"),
    ("WGS84.PSEUDOMERCATOR", "EPSG:3857"),
    ("AMERSFOORT.RD-NEW", "EPSG:28992"),
    ("RD-NEW", "EPSG:28992"),
    ("BELGE72.LAMBERT72", "EPSG:31370"),
    ("BELGE-LAMBERT-72", "EPSG:31370"),
    ("BRITISHNATGRID", "EPSG:27700"),
    ("CH1903PLUS.LV95", "EPSG:2056"),
    ("RGF93.LAMBERT-93", "EPSG:2154"),
];

/// The EPSG code a GEODATA coordinate system definition names: MapGuide XML
/// (R2010+, an EPSG alias or a known name) or WKT (R2009).
pub fn epsg_from_definition(def: &str) -> Option<String> {
    let up = def.to_ascii_uppercase();
    // <Alias id="28992" type="CoordinateSystem"><ObjectId>28992</ObjectId><Namespace>EPSG Code</Namespace>
    if let Some(i) = up.find("<NAMESPACE>EPSG CODE</NAMESPACE>") {
        let before = &up[..i];
        if let Some(j) = before.rfind("<OBJECTID>") {
            let code: String = before[j + 10..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                return Some(format!("EPSG:{code}"));
            }
        }
    }
    if up.contains("PROJCS[") || up.contains("GEOGCS[") || up.contains("PROJCRS[") {
        return epsg_from_wkt(def);
    }
    // <CoordinateSystem id="UTM84-31N"> / ETRS89.UTM-31N
    let id = up.split("COORDINATESYSTEM ID=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or(up.trim());
    if let Some((_, c)) = MAPGUIDE.iter().find(|(n, _)| *n == id) {
        return Some((*c).into());
    }
    let utm = |prefix: &str, base: u32| -> Option<String> {
        let rest = id.strip_prefix(prefix)?;
        let zone: u32 = rest.trim_end_matches(['N', 'S']).parse().ok()?;
        let south = rest.ends_with('S');
        Some(format!("EPSG:{}", if south { base + 100 } else { base } + zone))
    };
    utm("UTM84-", 32600).or_else(|| utm("ETRS89.UTM-", 25800))
}

/// What a GEODATA object says about the drawing's location.
#[derive(Clone, Debug)]
pub struct GeoLocation {
    /// 1 local grid, 2 projected grid, 3 geographic.
    pub coordinate_type: i16,
    pub design_point: [f64; 2],
    pub reference_point: [f64; 2],
    pub definition: String,
    /// Drawing units per CRS unit as the object states it (horizontal unit scale).
    pub unit_scale: f64,
    pub north: [f64; 2],
}

/// A project site from a GEODATA location, for a drawing in `unit_factor`
/// drawing units per metre. Only a north-up drawing maps onto a site.
pub fn site_from(g: &GeoLocation, unit_factor: f64) -> Result<Site, String> {
    let code = epsg_from_definition(&g.definition).ok_or("GEODATA: coordinate system not recognised")?;
    crs::ensure(&code)?;
    let tilt = g.north[0].atan2(g.north[1]).abs();
    if g.north != [0.0, 0.0] && tilt > 1e-6 {
        return Err(format!("GEODATA: north is turned {:.2}°; Open Infra works north-up", tilt.to_degrees()));
    }
    let f = if unit_factor > 0.0 { unit_factor } else { 1.0 };
    let mut site = Site::default();
    let projected = g.coordinate_type == 2 && !crs::is_geographic(&code);
    if projected {
        site.crs = code;
        site.reproject_origin();
        // CRS = reference + (drawing − design) / units per metre.
        let o = [g.reference_point[0] - g.design_point[0] / f, g.reference_point[1] - g.design_point[1] / f];
        site.absolute = o[0].abs() < 1e-3 && o[1].abs() < 1e-3;
        let at = if site.absolute { g.reference_point } else { o };
        site.set_origin(at[0], at[1]);
    } else {
        // Geographic reference (lon, lat): a projected grid for the area,
        // with the design point as the base point.
        let (lon, lat) = (g.reference_point[0], g.reference_point[1]);
        if !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) {
            return Err("GEODATA: geographic reference point out of range".into());
        }
        if g.design_point != [0.0, 0.0] {
            return Err("GEODATA: a geographic location with the design point away from 0,0 is not supported".into());
        }
        site.lon = lon;
        site.lat = lat;
        site.crs = crs::suggest(lon, lat);
        crs::ensure(&site.crs)?;
        site.absolute = false;
        site.reproject_origin();
    }
    site.label = format!("GEODATA ({})", site.crs);
    Ok(site)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions() {
        let xml = r#"<?xml version="1.0"?><Dictionary><Alias id="28992" type="CoordinateSystem"><ObjectId>28992</ObjectId><Namespace>EPSG Code</Namespace></Alias><CoordinateSystem id="Amersfoort.RD-New"/></Dictionary>"#;
        assert_eq!(epsg_from_definition(xml).as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg_from_definition(r#"<Dictionary><CoordinateSystem id="UTM84-32N"></CoordinateSystem></Dictionary>"#).as_deref(), Some("EPSG:32632"));
        assert_eq!(epsg_from_definition(r#"<Dictionary><CoordinateSystem id="BritishNatGrid"/></Dictionary>"#).as_deref(), Some("EPSG:27700"));
        assert_eq!(epsg_from_definition(r#"PROJCS["Amersfoort / RD New",AUTHORITY["EPSG","28992"]]"#).as_deref(), Some("EPSG:28992"));
    }

    #[test]
    fn projected_site() {
        let g = GeoLocation { coordinate_type: 2, design_point: [0.0, 0.0], reference_point: [155000.0, 463000.0], definition: "<CoordinateSystem id=\"Amersfoort.RD-New\"/>".into(), unit_scale: 1.0, north: [0.0, 1.0] };
        let s = site_from(&g, 1.0).unwrap();
        assert_eq!(s.crs, "EPSG:28992");
        assert!(!s.absolute);
        assert!((s.origin_e - 155000.0).abs() < 1e-6);
        let abs = GeoLocation { design_point: [155000.0, 463000.0], ..g };
        assert!(site_from(&abs, 1.0).unwrap().absolute);
    }
}
