//! OpenStreetMap via the Overpass API — worldwide buildings, roads, rail,
//! water, civil structures, land use, utilities, fences and house numbers.

use serde_json::Value;

use super::http;
use super::site::Site;
use super::vector::Feature;

pub struct Theme {
    pub id: &'static str,
    pub label: &'static str,
    pub layer: &'static str,
    /// ACI colour index for the layer.
    pub aci: i16,
    /// Overpass statements; `{bbox}` is replaced by the site extent.
    pub query: &'static str,
    pub label_tag: Option<&'static str>,
    pub default_on: bool,
    matcher: fn(&serde_json::Map<String, Value>, &str) -> bool,
}

impl Theme {
    pub fn matches(&self, tags: &serde_json::Map<String, Value>, kind: &str) -> bool {
        (self.matcher)(tags, kind)
    }
}

fn tag<'a>(t: &'a serde_json::Map<String, Value>, k: &str) -> &'a str {
    t.get(k).and_then(Value::as_str).unwrap_or("")
}

pub const THEMES: &[Theme] = &[
    Theme { id: "gebouwen", label: "Gebouwen", layer: "OSM-GEBOUWEN", aci: 30, default_on: true, label_tag: None,
        query: r#"way["building"]({bbox});relation["building"]({bbox});"#,
        matcher: |t, _| !tag(t, "building").is_empty() },
    Theme { id: "wegen", label: "Wegen en paden", layer: "OSM-WEGEN", aci: 2, default_on: true, label_tag: Some("name"),
        query: r#"way["highway"]({bbox});"#,
        matcher: |t, _| !tag(t, "highway").is_empty() },
    Theme { id: "spoor", label: "Spoor, tram en metro", layer: "OSM-SPOOR", aci: 6, default_on: true, label_tag: None,
        query: r#"way["railway"~"^(rail|tram|light_rail|subway|narrow_gauge|monorail)$"]({bbox});"#,
        matcher: |t, _| matches!(tag(t, "railway"), "rail" | "tram" | "light_rail" | "subway" | "narrow_gauge" | "monorail") },
    Theme { id: "water", label: "Water en waterlopen", layer: "OSM-WATER", aci: 5, default_on: true, label_tag: Some("name"),
        query: r#"way["waterway"]({bbox});way["natural"="water"]({bbox});relation["natural"="water"]({bbox});"#,
        matcher: |t, _| !tag(t, "waterway").is_empty() || tag(t, "natural") == "water" },
    Theme { id: "kunstwerken", label: "Kunstwerken (bruggen, tunnels, viaducten)", layer: "OSM-KUNSTWERKEN", aci: 1, default_on: false, label_tag: Some("name"),
        query: r#"way["man_made"="bridge"]({bbox});way["bridge"]["bridge"!="no"]({bbox});way["tunnel"]["tunnel"!="no"]({bbox});"#,
        matcher: |t, _| tag(t, "man_made") == "bridge" || !matches!(tag(t, "bridge"), "" | "no") || !matches!(tag(t, "tunnel"), "" | "no") },
    Theme { id: "landgebruik", label: "Landgebruik", layer: "OSM-LANDGEBRUIK", aci: 3, default_on: false, label_tag: None,
        query: r#"way["landuse"]({bbox});relation["landuse"]({bbox});"#,
        matcher: |t, _| !tag(t, "landuse").is_empty() },
    Theme { id: "leidingen", label: "Kabels en leidingen (hoogspanning, pijpleidingen)", layer: "OSM-KABELS-LEIDINGEN", aci: 9, default_on: false, label_tag: None,
        query: r#"way["power"~"^(line|minor_line|cable)$"]({bbox});way["man_made"="pipeline"]({bbox});node["power"="tower"]({bbox});"#,
        matcher: |t, _| matches!(tag(t, "power"), "line" | "minor_line" | "cable" | "tower") || tag(t, "man_made") == "pipeline" },
    Theme { id: "grenzen", label: "Terreingrenzen, hekken en keerwanden", layer: "OSM-GRENZEN", aci: 4, default_on: false, label_tag: None,
        query: r#"way["barrier"~"^(fence|wall|hedge|retaining_wall|guard_rail)$"]({bbox});"#,
        matcher: |t, _| matches!(tag(t, "barrier"), "fence" | "wall" | "hedge" | "retaining_wall" | "guard_rail") },
    Theme { id: "huisnummers", label: "Huisnummers", layer: "OSM-ADRESSEN", aci: 8, default_on: false, label_tag: Some("addr:housenumber"),
        query: r#"node["addr:housenumber"]({bbox});"#,
        matcher: |t, kind| kind == "node" && !tag(t, "addr:housenumber").is_empty() },
];

const ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
];

fn coords(geom: &Value) -> Vec<[f64; 2]> {
    geom.as_array()
        .map(|a| a.iter().filter_map(|g| Some([g["lon"].as_f64()?, g["lat"].as_f64()?])).collect())
        .unwrap_or_default()
}

fn to_feature(el: &Value) -> Option<Feature> {
    let mut f = Feature { props: el["tags"].as_object().cloned().unwrap_or_default(), ..Default::default() };
    match el["type"].as_str()? {
        "node" => f.points.push([el["lon"].as_f64()?, el["lat"].as_f64()?]),
        "way" => {
            let c = coords(&el["geometry"]);
            if c.len() < 2 {
                return None;
            }
            let closed = c.len() > 3 && c.first() == c.last();
            f.lines.push((c, closed));
        }
        "relation" => {
            for m in el["members"].as_array()? {
                let c = coords(&m["geometry"]);
                if c.len() > 1 {
                    let closed = c.len() > 3 && c.first() == c.last();
                    f.lines.push((c, closed));
                }
            }
            if f.lines.is_empty() {
                return None;
            }
        }
        _ => return None,
    }
    Some(f)
}

/// One combined Overpass query for all chosen themes (kinder to the public
/// servers); the result is sorted per theme on the client.
pub fn fetch(site: &Site, themes: &[&Theme], progress: &dyn Fn(String)) -> Result<Vec<Vec<Feature>>, String> {
    let [w, s, e, n] = site.bbox_lonlat()?;
    let bbox = format!("{s:.7},{w:.7},{n:.7},{e:.7}");
    let statements: String = themes.iter().map(|t| t.query.replace("{bbox}", &bbox)).collect();
    let query = format!("[out:json][timeout:120];({statements});out geom;");
    // The main instance is the most reliable but rate-limits per IP (429/504
    // when busy): try it, pause, try again, then fall back to the mirrors.
    let attempts: [(&str, u64, u64); 5] = [
        (ENDPOINTS[0], 0, 120),
        (ENDPOINTS[0], 8, 120),
        (ENDPOINTS[1], 0, 70),
        (ENDPOINTS[2], 0, 40),
        (ENDPOINTS[3], 0, 70),
    ];
    let mut last = String::new();
    for (url, pause, timeout) in attempts {
        if pause > 0 {
            progress(format!("Overpass is druk, opnieuw over {pause} s…"));
            std::thread::sleep(std::time::Duration::from_secs(pause));
        }
        let host = url.split('/').nth(2).unwrap_or(url);
        progress(format!("OpenStreetMap ophalen via {host}… (kan even duren)"));
        let body = match http::post_form(url, &[("data", &query)], timeout) {
            Ok(f) => f.body,
            Err(e) => {
                last = e;
                continue;
            }
        };
        let json: Value = match serde_json::from_slice(&body) {
            Ok(j) => j,
            Err(e) => {
                last = format!("{host}: {e}");
                continue;
            }
        };
        if let Some(remark) = json["remark"].as_str().filter(|r| r.contains("error") || r.contains("timed out")) {
            last = format!("{host}: {remark}");
            continue;
        }
        let mut out: Vec<Vec<Feature>> = themes.iter().map(|_| Vec::new()).collect();
        for el in json["elements"].as_array().cloned().unwrap_or_default() {
            let empty = serde_json::Map::new();
            let tags = el["tags"].as_object().unwrap_or(&empty);
            let kind = el["type"].as_str().unwrap_or("");
            let hits: Vec<usize> = themes.iter().enumerate().filter(|(_, t)| t.matches(tags, kind)).map(|(i, _)| i).collect();
            if hits.is_empty() {
                continue;
            }
            if let Some(f) = to_feature(&el) {
                for i in hits {
                    out[i].push(f.clone());
                }
            }
        }
        return Ok(out);
    }
    Err(format!("OpenStreetMap (Overpass) is overbelast of onbereikbaar — probeer het later opnieuw. ({last})"))
}
