//! WFS GetFeature as GeoJSON — in the project CRS when the server offers it,
//! otherwise WGS 84 or another system it lists — with startIndex paging for
//! servers with a page limit. Placing reprojects the features to the project
//! CRS (`vector::data_to_site`).

use super::caps::{pick_geojson_format, WfsCaps, WfsType};
use super::http;
use super::site::Site;
use super::vector::{parse_geojson, Feature};

pub struct WfsResult {
    pub features: Vec<Feature>,
    pub data_crs: String,
    pub truncated: bool,
}

/// The CRS to ask a feature type for: the project CRS when offered (or
/// nothing is listed); WGS 84 when offered; else a listed system that can be
/// transformed (built in first, then any EPSG code via epsg.io); else WGS 84.
pub fn request_crs(ft: &WfsType, site_crs: &str) -> String {
    use super::crs::{is_web_mercator, support, Support};
    if ft.crs.is_empty() || ft.crs.iter().any(|c| c == site_crs) {
        return site_crs.to_string();
    }
    if ft.crs.iter().any(|c| c == "EPSG:4326") {
        return "EPSG:4326".into();
    }
    for level in [Support::Builtin, Support::Online] {
        if let Some(c) = ft.crs.iter().find(|c| support(c) == level && !is_web_mercator(c)) {
            return c.clone();
        }
    }
    "EPSG:4326".into()
}

pub fn fetch(site: &Site, caps: &WfsCaps, ft: &WfsType, limit: usize, progress: &dyn Fn(String)) -> Result<WfsResult, String> {
    let v2 = caps.version.starts_with('2');
    let mut data_crs = request_crs(ft, &site.crs);
    if data_crs != site.crs && data_crs != "EPSG:4326" && super::crs::ensure(&data_crs).is_err() {
        // Its definition could not be loaded: WGS 84, which most servers serve anyway.
        data_crs = "EPSG:4326".into();
    }
    let bbox = if data_crs == site.crs {
        let b = site.bbox();
        format!("{},{},{},{},{}", b[0], b[1], b[2], b[3], site.crs)
    } else if data_crs == "EPSG:4326" {
        let [w, s, e, n] = site.bbox_lonlat()?;
        // urn form = lat/lon axis order (WFS 1.1 / 2.0).
        format!("{s},{w},{n},{e},urn:ogc:def:crs:EPSG::4326")
    } else {
        // The server's own system, easting first (EPSG:xxxx form).
        let b = super::site::envelope(site.bbox(), &super::crs::Transformer::new(&site.crs, &data_crs)?)?;
        format!("{},{},{},{},{}", b[0], b[1], b[2], b[3], data_crs)
    };
    let format = pick_geojson_format(&caps.output_formats);
    let page = limit.min(1000);
    let mut features = Vec::new();
    let mut truncated = false;
    let mut start = 0;
    while start < limit {
        let want = page.min(limit - start);
        let mut params: Vec<(&str, String)> = vec![
            ("SERVICE", "WFS".into()),
            ("VERSION", if v2 { "2.0.0" } else { "1.1.0" }.into()),
            ("REQUEST", "GetFeature".into()),
            (if v2 { "TYPENAMES" } else { "TYPENAME" }, ft.name.clone()),
            ("SRSNAME", data_crs.clone()),
            ("BBOX", bbox.clone()),
            ("OUTPUTFORMAT", format.clone()),
            (if v2 { "COUNT" } else { "MAXFEATURES" }, want.to_string()),
        ];
        if start > 0 {
            params.push(("STARTINDEX", start.to_string()));
        }
        progress(format!("Objecten ophalen… ({})", features.len()));
        let body = http::get_text(&http::with_params(&caps.get_feature_url, &params))?;
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|_| http::service_error(&body).unwrap_or_else(|| "server gaf geen GeoJSON terug".into()))?;
        let got = parse_geojson(&json);
        let n = got.len();
        features.extend(got);
        if n < want {
            break;
        }
        if !v2 {
            // WFS 1.x has no reliable paging.
            truncated = n >= page;
            break;
        }
        start += page;
        if start >= limit {
            truncated = true;
        }
    }
    Ok(WfsResult { features, data_crs, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ft(crs: &[&str]) -> WfsType {
        WfsType { name: "t".into(), title: "t".into(), crs: crs.iter().map(|c| c.to_string()).collect(), bbox: None }
    }

    #[test]
    fn wfs_request_crs_choice() {
        assert_eq!(request_crs(&ft(&[]), "EPSG:28992"), "EPSG:28992");
        assert_eq!(request_crs(&ft(&["EPSG:25832", "EPSG:28992"]), "EPSG:28992"), "EPSG:28992");
        assert_eq!(request_crs(&ft(&["EPSG:25832", "EPSG:4326"]), "EPSG:28992"), "EPSG:4326");
        // Built in before epsg.io; never Web Mercator for vector data.
        assert_eq!(request_crs(&ft(&["EPSG:3857", "EPSG:2180", "EPSG:25833"]), "EPSG:28992"), "EPSG:25833");
        assert_eq!(request_crs(&ft(&["EPSG:2180"]), "EPSG:28992"), "EPSG:2180");
        assert_eq!(request_crs(&ft(&["IGNF:LAMB93"]), "EPSG:2154"), "EPSG:4326");
    }
}
