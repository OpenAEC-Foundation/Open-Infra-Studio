//! OGC GetCapabilities parsing for WMS (1.1.1 / 1.3.0) and WFS (1.1 / 2.0).

use roxmltree::{Document, Node};

use super::crs::normalize;
use super::http;

#[derive(Clone, Debug)]
pub struct WmsLayer {
    pub name: String,
    pub title: String,
    pub crs: Vec<String>,
    /// Extent in WGS 84 [west, south, east, north], inherited from the parent
    /// layer when the layer has none.
    pub bbox: Option<[f64; 4]>,
}

#[derive(Clone, Debug)]
pub struct WmsCaps {
    pub version: String,
    pub title: String,
    pub get_map_url: String,
    pub formats: Vec<String>,
    pub layers: Vec<WmsLayer>,
}

#[derive(Clone, Debug)]
pub struct WfsType {
    pub name: String,
    pub title: String,
    pub crs: Vec<String>,
    /// Extent in WGS 84 [west, south, east, north].
    pub bbox: Option<[f64; 4]>,
}

#[derive(Clone, Debug)]
pub struct WfsCaps {
    pub version: String,
    pub title: String,
    pub get_feature_url: String,
    pub output_formats: Vec<String>,
    pub types: Vec<WfsType>,
}

fn children<'a, 'i>(n: Node<'a, 'i>, local: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    n.children().filter(move |c| c.is_element() && c.tag_name().name() == local)
}

fn child<'a, 'i>(n: Node<'a, 'i>, local: &'a str) -> Option<Node<'a, 'i>> {
    children(n, local).next()
}

fn text_of(n: Option<Node>, local: &str) -> String {
    n.and_then(|n| child(n, local)).and_then(|c| c.text()).unwrap_or("").trim().to_string()
}

fn parse(xml: &str) -> Result<Document<'_>, String> {
    if let Some(e) = http::service_error(xml) {
        return Err(e);
    }
    // WMS 1.1.1 capabilities often start with a DOCTYPE (WMS_MS_Capabilities.dtd).
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    Document::parse_with_options(xml, opt).map_err(|e| format!("ongeldig antwoord van de server: {e}"))
}

fn href(n: Node) -> Option<String> {
    n.attributes().find(|a| a.name() == "href").map(|a| a.value().to_string())
}

/// URL of an operation (WMS DCPType or OWS OperationsMetadata).
fn operation_url(root: Node, op: &str) -> Option<String> {
    for n in root.descendants().filter(|n| n.is_element()) {
        let is_op = n.tag_name().name() == op
            || (n.tag_name().name() == "Operation" && n.attribute("name") == Some(op));
        if !is_op {
            continue;
        }
        if let Some(r) = n.descendants().find(|d| d.tag_name().name() == "OnlineResource").and_then(href) {
            return Some(r);
        }
        if let Some(r) = n.descendants().find(|d| d.tag_name().name() == "Get").and_then(href) {
            return Some(r);
        }
    }
    None
}

/// Keep https when the capabilities advertise a plain-http endpoint.
fn same_scheme(service: &str, advertised: Option<String>) -> String {
    match advertised {
        Some(a) if service.starts_with("https:") && a.starts_with("http:") => format!("https:{}", &a[5..]),
        Some(a) if !a.is_empty() => a,
        _ => service.to_string(),
    }
}

/// A WGS 84 extent from the capabilities: WMS 1.3 `EX_GeographicBoundingBox`,
/// WMS 1.1 `LatLonBoundingBox`, WFS 1.0 `LatLongBoundingBox` or WFS 1.1/2.0
/// `ows:WGS84BoundingBox`. Degenerate or impossible boxes are dropped.
fn wgs84_bbox(n: Node) -> Option<[f64; 4]> {
    let num = |t: &str| t.trim().parse::<f64>().ok();
    let valid = |b: [f64; 4]| (b.iter().all(|v| v.is_finite()) && b[0] <= b[2] && b[1] <= b[3] && b[0] >= -180.5 && b[2] <= 180.5 && b[1] >= -90.5 && b[3] <= 90.5).then_some(b);
    if let Some(g) = child(n, "EX_GeographicBoundingBox") {
        let v = |k: &str| child(g, k).and_then(|c| c.text()).and_then(num);
        return valid([v("westBoundLongitude")?, v("southBoundLatitude")?, v("eastBoundLongitude")?, v("northBoundLatitude")?]);
    }
    if let Some(g) = child(n, "LatLonBoundingBox").or_else(|| child(n, "LatLongBoundingBox")) {
        let a = |k: &str| g.attribute(k).and_then(num);
        return valid([a("minx")?, a("miny")?, a("maxx")?, a("maxy")?]);
    }
    if let Some(g) = child(n, "WGS84BoundingBox") {
        let corner = |k: &str| -> Option<[f64; 2]> {
            let t = child(g, k)?.text()?;
            let v: Vec<f64> = t.split_whitespace().filter_map(num).collect();
            (v.len() == 2).then(|| [v[0], v[1]])
        };
        let (lo, hi) = (corner("LowerCorner")?, corner("UpperCorner")?);
        return valid([lo[0], lo[1], hi[0], hi[1]]);
    }
    None
}

pub fn fetch_wms(url: &str) -> Result<WmsCaps, String> {
    let xml = http::get_text(&http::with_params(url, &[("SERVICE", "WMS".into()), ("REQUEST", "GetCapabilities".into())]))?;
    parse_wms(url, &xml)
}

/// WMS capabilities from the XML `url` answered.
pub fn parse_wms(url: &str, xml: &str) -> Result<WmsCaps, String> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    let version = root.attribute("version").unwrap_or("1.3.0").to_string();
    let capability = child(root, "Capability").ok_or("geen WMS-capabilities gevonden")?;
    let formats = child(capability, "Request")
        .and_then(|r| child(r, "GetMap"))
        .map(|g| children(g, "Format").filter_map(|f| f.text()).map(|t| t.trim().to_string()).collect())
        .unwrap_or_default();

    let mut layers = Vec::new();
    fn walk(n: Node, inherited: &[String], parent_bbox: Option<[f64; 4]>, out: &mut Vec<WmsLayer>) {
        let mut crs: Vec<String> = inherited.to_vec();
        let bbox = wgs84_bbox(n).or(parent_bbox);
        for c in children(n, "CRS").chain(children(n, "SRS")) {
            for t in c.text().unwrap_or("").split_whitespace() {
                let code = normalize(t);
                if !crs.contains(&code) {
                    crs.push(code);
                }
            }
        }
        let name = text_of(Some(n), "Name");
        if !name.is_empty() {
            let title = text_of(Some(n), "Title");
            out.push(WmsLayer { title: if title.is_empty() { name.clone() } else { title }, name, crs: crs.clone(), bbox });
        }
        for sub in children(n, "Layer") {
            walk(sub, &crs, bbox, out);
        }
    }
    for top in children(capability, "Layer") {
        walk(top, &[], None, &mut layers);
    }
    Ok(WmsCaps {
        version,
        title: text_of(child(root, "Service"), "Title"),
        get_map_url: same_scheme(url, operation_url(capability, "GetMap")),
        formats,
        layers,
    })
}

pub fn fetch_wfs(url: &str) -> Result<WfsCaps, String> {
    let xml = http::get_text(&http::with_params(url, &[("SERVICE", "WFS".into()), ("REQUEST", "GetCapabilities".into())]))?;
    parse_wfs(url, &xml)
}

/// WFS capabilities from the XML `url` answered.
pub fn parse_wfs(url: &str, xml: &str) -> Result<WfsCaps, String> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    let version = root.attribute("version").unwrap_or("2.0.0").to_string();
    let ident = child(root, "ServiceIdentification").or_else(|| child(root, "Service"));
    let types = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "FeatureType")
        .filter_map(|ft| {
            let name = text_of(Some(ft), "Name");
            if name.is_empty() {
                return None;
            }
            let mut crs: Vec<String> = Vec::new();
            for code in ft
                .children()
                .filter(|c| c.is_element() && ["DefaultCRS", "OtherCRS", "DefaultSRS", "OtherSRS", "SRS"].contains(&c.tag_name().name()))
                .filter_map(|c| c.text())
                .map(normalize)
            {
                if !crs.contains(&code) {
                    crs.push(code);
                }
            }
            let title = text_of(Some(ft), "Title");
            Some(WfsType { title: if title.is_empty() { name.clone() } else { title }, name, crs, bbox: wgs84_bbox(ft) })
        })
        .collect();
    let mut output_formats: Vec<String> = Vec::new();
    for p in root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Parameter") {
        if p.attribute("name").is_some_and(|a| a.eq_ignore_ascii_case("outputFormat")) {
            for v in p.descendants().filter(|v| v.tag_name().name() == "Value") {
                if let Some(t) = v.text() {
                    output_formats.push(t.trim().to_string());
                }
            }
        }
    }
    Ok(WfsCaps {
        version,
        title: text_of(ident, "Title"),
        get_feature_url: same_scheme(url, operation_url(root, "GetFeature")),
        output_formats,
        types,
    })
}

/// A GeoJSON output format from what the server advertises.
pub fn pick_geojson_format(formats: &[String]) -> String {
    let lower: Vec<String> = formats.iter().map(|f| f.to_ascii_lowercase()).collect();
    for want in ["application/json", "geojson", "json"] {
        if let Some(i) = lower.iter().position(|f| f == want || (want != "application/json" && f.contains(want))) {
            return formats[i].clone();
        }
    }
    "application/json".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geojson_format_preference() {
        let f = vec!["text/xml; subtype=gml/3.2".to_string(), "application/json; subtype=geojson".to_string(), "application/json".to_string()];
        assert_eq!(pick_geojson_format(&f), "application/json");
        assert_eq!(pick_geojson_format(&["GML2".into(), "geojson".into()]), "geojson");
    }

    #[test]
    fn wms_layer_extents_and_inheritance() {
        let xml = r#"<WMS_Capabilities version="1.3.0" xmlns="http://www.opengis.net/wms" xmlns:xlink="http://www.w3.org/1999/xlink">
          <Service><Title>Test</Title></Service>
          <Capability>
            <Request><GetMap><Format>image/png</Format><DCPType><HTTP><Get><OnlineResource xlink:href="http://example.org/wms?"/></Get></HTTP></DCPType></GetMap></Request>
            <Layer><Title>Root</Title><CRS>EPSG:25832</CRS><CRS>CRS:84</CRS>
              <EX_GeographicBoundingBox><westBoundLongitude>5.8</westBoundLongitude><eastBoundLongitude>15.1</eastBoundLongitude><southBoundLatitude>47.2</southBoundLatitude><northBoundLatitude>55.1</northBoundLatitude></EX_GeographicBoundingBox>
              <Layer><Name>flurstuecke</Name><Title>Flurstücke</Title></Layer>
              <Layer><Name>gebaeude</Name><Title>Gebäude</Title><CRS>EPSG:25833</CRS>
                <EX_GeographicBoundingBox><westBoundLongitude>11.2</westBoundLongitude><eastBoundLongitude>14.8</eastBoundLongitude><southBoundLatitude>51.3</southBoundLatitude><northBoundLatitude>53.6</northBoundLatitude></EX_GeographicBoundingBox>
              </Layer>
            </Layer>
          </Capability></WMS_Capabilities>"#;
        let c = parse_wms("https://example.org/wms", xml).unwrap();
        assert_eq!(c.layers.len(), 2);
        assert_eq!(c.layers[0].bbox, Some([5.8, 47.2, 15.1, 55.1]));
        assert_eq!(c.layers[0].crs, vec!["EPSG:25832", "EPSG:4326"]);
        assert_eq!(c.layers[1].bbox, Some([11.2, 51.3, 14.8, 53.6]));
        assert_eq!(c.layers[1].crs, vec!["EPSG:25832", "EPSG:4326", "EPSG:25833"]);
        // https kept although the server advertises http.
        assert_eq!(c.get_map_url, "https://example.org/wms?");
    }

    #[test]
    fn wms_111_with_doctype() {
        let xml = r#"<?xml version="1.0"?>
          <!DOCTYPE WMT_MS_Capabilities SYSTEM "http://schemas.opengis.net/wms/1.1.1/WMS_MS_Capabilities.dtd" [ <!ELEMENT VendorSpecificCapabilities EMPTY> ]>
          <WMT_MS_Capabilities version="1.1.1"><Service><Title>Ortho</Title></Service><Capability>
            <Layer><SRS>EPSG:3879</SRS><Layer><Name>ortho</Name><LatLonBoundingBox minx="24.5" miny="60.05" maxx="24.86" maxy="60.36"/></Layer></Layer>
          </Capability></WMT_MS_Capabilities>"#;
        let c = parse_wms("https://example.org/wms", xml).unwrap();
        assert_eq!(c.version, "1.1.1");
        assert_eq!(c.layers[0].bbox, Some([24.5, 60.05, 24.86, 60.36]));
        assert_eq!(c.layers[0].crs, vec!["EPSG:3879"]);
    }

    #[test]
    fn wfs_type_extents() {
        let xml = r#"<wfs:WFS_Capabilities version="2.0.0" xmlns:wfs="http://www.opengis.net/wfs/2.0" xmlns:ows="http://www.opengis.net/ows/1.1">
          <wfs:FeatureTypeList><wfs:FeatureType><wfs:Name>cp:CadastralParcel</wfs:Name><wfs:Title>Parcels</wfs:Title>
            <wfs:DefaultCRS>urn:ogc:def:crs:EPSG::2180</wfs:DefaultCRS><wfs:OtherCRS>urn:ogc:def:crs:EPSG::4326</wfs:OtherCRS><wfs:OtherCRS>EPSG:2180</wfs:OtherCRS>
            <ows:WGS84BoundingBox><ows:LowerCorner>14.1 49.0</ows:LowerCorner><ows:UpperCorner>24.2 54.8</ows:UpperCorner></ows:WGS84BoundingBox>
          </wfs:FeatureType></wfs:FeatureTypeList></wfs:WFS_Capabilities>"#;
        let c = parse_wfs("https://example.org/wfs", xml).unwrap();
        assert_eq!(c.types[0].crs, vec!["EPSG:2180", "EPSG:4326"]);
        assert_eq!(c.types[0].bbox, Some([14.1, 49.0, 24.2, 54.8]));
        let v1 = r#"<WFS_Capabilities version="1.0.0"><FeatureTypeList><FeatureType><Name>roads</Name><SRS>EPSG:3006</SRS>
          <LatLongBoundingBox minx="10.9" miny="55.3" maxx="24.2" maxy="69.1"/></FeatureType></FeatureTypeList></WFS_Capabilities>"#;
        let c = parse_wfs("https://example.org/wfs", v1).unwrap();
        assert_eq!(c.types[0].bbox, Some([10.9, 55.3, 24.2, 69.1]));
        assert_eq!(c.types[0].title, "roads");
    }
}
