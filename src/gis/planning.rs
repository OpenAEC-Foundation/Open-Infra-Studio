//! IMRO planning data and the public Ruimtelijke Plannen API v4 contract.
//! This is a map-data client, not a determination of legal applicability.
pub mod gml;
use super::{crs::XY, files::FileData, vector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const BASE: &str =
    "https://ruimte.omgevingswet.overheid.nl/ruimtelijke-plannen/api/opvragen/v4";
pub const COLLECTIONS: &[&str] = &[
    "bestemmingsvlakken",
    "bouwvlakken",
    "functieaanduidingen",
    "bouwaanduidingen",
    "lettertekenaanduidingen",
    "maatvoeringen",
    "figuren",
    "gebiedsaanduidingen",
    "structuurvisiegebieden",
    "structuurvisiecomplexen",
    "structuurvisieverklaringen",
    "besluitvlakken",
    "besluitsubvlakken",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub status: String,
    pub date: String,
    pub authority: String,
    pub legal_end: Option<String>,
    pub historical: bool,
    pub tam: bool,
}
impl Plan {
    pub fn parse(v: &Value) -> Result<Self, String> {
        Ok(Self {
            id: required(v, "id")?,
            name: required(v, "naam")?,
            status: v["planstatusInfo"]["planstatus"]
                .as_str()
                .unwrap_or("")
                .into(),
            date: v["planstatusInfo"]["datum"].as_str().unwrap_or("").into(),
            authority: v["publicerendBevoegdGezag"]["naam"]
                .as_str()
                .unwrap_or("")
                .into(),
            legal_end: v["eindeRechtsgeldigheid"].as_str().map(str::to_string),
            historical: v["isHistorisch"].as_bool().unwrap_or(false),
            tam: v["isTamPlan"].as_bool().unwrap_or(false),
        })
    }
}
fn required(v: &Value, k: &str) -> Result<String, String> {
    v[k].as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Planrespons mist {k}"))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    pub id: String,
    pub kind: String,
    pub group: String,
    pub name: String,
    pub geometry: Value,
    pub properties: Map<String, Value>,
}
#[derive(Clone, Debug)]
pub struct Data {
    pub objects: Vec<Object>,
    pub crs: String,
    pub warnings: Vec<String>,
}
impl Data {
    pub fn file_data(&self) -> FileData {
        let features = self
            .objects
            .iter()
            .enumerate()
            .flat_map(|(index, o)| {
                let mut props = o.properties.clone();
                props.insert("id".into(), o.id.clone().into());
                props.insert("featuretype".into(), o.kind.clone().into());
                props.insert("bestemmingshoofdgroep".into(), o.group.clone().into());
                props.insert("naam".into(), o.name.clone().into());
                props.insert("__imro_geometry".into(), o.geometry.clone());
                if index == 0 {
                    props.insert("__imro_warnings".into(), json!(self.warnings));
                }
                vector::parse_geojson(
                    &json!({"features":[{"geometry":o.geometry,"properties":props}]}),
                )
            })
            .collect();
        FileData {
            format: "IMRO",
            features,
            crs: Some(self.crs.clone()),
            layers: vec![],
        }
    }
    pub fn from_file_data(data: &FileData) -> Result<Self, String> {
        let mut objects = vec![];
        let mut warnings = vec![];
        for f in &data.features {
            let get = |k: &str| {
                f.props
                    .get(k)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            let mut properties = f.props.clone();
            if let Some(Value::Array(items)) = properties.remove("__imro_warnings") {
                warnings.extend(
                    items
                        .into_iter()
                        .filter_map(|v| v.as_str().map(str::to_string)),
                );
            }
            let geometry = properties
                .remove("__imro_geometry")
                .ok_or("IMRO-geometrie ontbreekt")?;
            objects.push(Object {
                id: get("id"),
                kind: get("featuretype"),
                group: get("bestemmingshoofdgroep"),
                name: get("naam"),
                geometry,
                properties,
            });
        }
        Ok(Self {
            objects,
            crs: data.crs.clone().ok_or("GML mist coördinatenstelsel")?,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    fn mock(responses: Vec<(u16, Value)>) -> (Client, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = Client::new("synthetic-secret".into()).unwrap();
        client.base = format!(
            "http://{}/ruimtelijke-plannen/api/opvragen/v4",
            listener.local_addr().unwrap()
        );
        let worker = thread::spawn(move || {
            responses.into_iter().map(|(status,body)|{
            let(mut stream,_)=listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut request=Vec::new();let mut buf=[0;4096];
            loop{let n=stream.read(&mut buf).unwrap();if n==0{break}request.extend_from_slice(&buf[..n]);
                if let Some(end)=request.windows(4).position(|w|w==b"\r\n\r\n"){
                    let headers=String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let len=headers.lines().find_map(|s|s.strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if request.len()>=end+4+len{break}
                }
            }
            let body=body.to_string();write!(stream,"HTTP/1.1 {status} Test\r\nContent-Type: application/hal+json\r\nContent-Crs: epsg:28992\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            String::from_utf8(request).unwrap()
        }).collect()
        });
        (client, worker)
    }
    #[test]
    fn search_uses_rd_post_and_follows_pages_without_putting_key_in_url() {
        let plan = json!({"id":"NL.IMRO.0000.TEST","naam":"Synthetisch","planstatusInfo":{"planstatus":"vastgesteld","datum":"2026-01-01"}});
        let (c, t) = mock(vec![
            (
                200,
                json!({"_embedded":{"plannen":[plan]},"_links":{"next":{"href":"?page=1&pageSize=100&expand=geometrie"}}}),
            ),
            (200, json!({"_embedded":{"plannen":[]}})),
        ]);
        assert_eq!(
            c.search([100000., 400000.]).unwrap()[0].status,
            "vastgesteld"
        );
        let requests = t.join().unwrap();
        assert_eq!(requests.len(), 2);
        for (index, r) in requests.iter().enumerate() {
            let lower = r.to_lowercase();
            assert!(r.starts_with("POST "));
            assert!(r.lines().next().unwrap().contains(&format!("page={index}")));
            assert!(lower.contains("content-crs: epsg:28992"));
            assert!(lower.contains("accept-crs: epsg:28992"));
            assert!(lower.contains("x-api-key: synthetic-secret"));
            assert!(!r.lines().next().unwrap().contains("secret"));
            assert!(r.contains("[100000.0,400000.0]"));
        }
    }
    #[test]
    fn foreign_pagination_is_rejected_and_auth_errors_are_redacted() {
        let (c, t) = mock(vec![(
            200,
            json!({"_embedded":{"plannen":[]},"_links":{"next":{"href":"https://example.invalid/steal"}}}),
        )]);
        assert!(c.search([1., 2.]).unwrap_err().contains("geweigerd"));
        t.join().unwrap();
        let (c, t) = mock(vec![(403, json!({"message":"synthetic-secret"}))]);
        let error = c.search([1., 2.]).unwrap_err();
        assert!(!error.contains("synthetic-secret"));
        assert!(error.contains("API-sleutel"));
        t.join().unwrap();
    }
    #[test]
    fn api_object_type_keeps_double_designation_style() {
        let plan = Plan::parse(&json!({"id":"test","naam":"Test"})).unwrap();
        let object=Object::from_api(&json!({"id":"zone","type":"dubbelbestemming","naam":"Waarde","geometrie":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}),"bestemmingsvlakken",&plan).unwrap();
        assert!(style(&object.kind, &object.group).hatch);
        assert!(!style(&object.kind, &object.group).fill);
        assert_eq!(style("Enkelbestemming", "wonen").color, [255, 255, 0]);
        assert!(!style("Bestemmingsplangebied", "").fill);
        assert!(style("Bestemmingsplangebied", "").dash);
        assert_eq!(object.properties["plan_id"], "test");
        let object=Object::from_api(&json!({"id":"area","geometrie":[{"type":"Point","coordinates":[100000,400000]},{"type":"Point","coordinates":[100001,400001]}]}),"besluitvlakken",&plan).unwrap();
        assert_eq!(object.geometry["type"], "GeometryCollection");
        assert_eq!(object.geometry["geometries"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn download_fetches_contour_and_all_object_collections_with_status() {
        let mut responses = vec![(
            200,
            json!({"id":"NL.IMRO.TEST","naam":"Testplan","isHistorisch":true,"planstatusInfo":{"planstatus":"vastgesteld","datum":"2026-01-01"},"geometrie":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,0]]]}}),
        )];
        for kind in COLLECTIONS {
            let mut embedded = Map::new();
            embedded.insert((*kind).into(),if *kind=="bestemmingsvlakken" {json!([{"id":"zone","type":"enkelbestemming","bestemmingshoofdgroep":"wonen","geometrie":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,0]]]}}])}else{json!([])});
            responses.push((200, json!({"_embedded":embedded})));
        }
        let (c, t) = mock(responses);
        let data = c.download("NL.IMRO.TEST").unwrap();
        assert_eq!(data.objects.len(), 2);
        assert!(!data.warnings.is_empty());
        assert_eq!(data.objects[1].properties["planstatus"], "vastgesteld");
        assert_eq!(data.objects[1].properties["isHistorisch"], true);
        let requests = t.join().unwrap();
        assert_eq!(requests.len(), 14);
        assert!(requests
            .iter()
            .all(|r| r.starts_with("GET ") && r.to_lowercase().contains("accept-crs: epsg:28992")));
    }
}
impl Object {
    pub fn from_api(v: &Value, kind: &str, plan: &Plan) -> Result<Self, String> {
        let raw = v
            .get("geometrie")
            .ok_or_else(|| format!("{}: geometrie ontbreekt", v["id"]))?;
        let geometry = if raw.is_array() {
            json!({"type":"GeometryCollection","geometries":raw})
        } else if raw.is_object() {
            raw.clone()
        } else {
            return Err(format!("{}: geometrie ontbreekt", v["id"]));
        };
        polygons(&geometry)?;
        let mut properties = v.as_object().cloned().ok_or("ongeldig planobject")?;
        properties.remove("geometrie");
        properties.remove("_embedded");
        properties.remove("_links");
        properties.insert("plan_id".into(), plan.id.clone().into());
        properties.insert("planstatus".into(), plan.status.clone().into());
        properties.insert("planstatusdatum".into(), plan.date.clone().into());
        properties.insert("isHistorisch".into(), plan.historical.into());
        properties.insert("isTamPlan".into(), plan.tam.into());
        properties.insert("bevoegdGezag".into(), plan.authority.clone().into());
        if let Some(end) = &plan.legal_end {
            properties.insert("eindeRechtsgeldigheid".into(), end.clone().into());
        }
        Ok(Self {
            id: required(v, "id")?,
            kind: if kind == "bestemmingsvlakken" {
                v["type"].as_str().unwrap_or(kind).into()
            } else {
                kind.into()
            },
            group: v["bestemmingshoofdgroep"].as_str().unwrap_or("").into(),
            name: v["naam"].as_str().unwrap_or("").into(),
            geometry,
            properties,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: [u8; 3],
    pub fill: bool,
    pub hatch: bool,
    pub dash: bool,
    pub weight: i16,
}
pub fn style(kind: &str, group: &str) -> Style {
    let k = kind.to_lowercase();
    let g = group.to_lowercase();
    let color = match g.as_str() {
        "agrarisch" => [235, 240, 210],
        "agrarisch met waarden" => [210, 225, 165],
        "bedrijf" => [180, 95, 210],
        "bedrijventerrein" => [200, 160, 215],
        "bos" => [100, 170, 45],
        "centrum" => [255, 200, 190],
        "cultuur en ontspanning" => [255, 60, 130],
        "detailhandel" => [255, 160, 150],
        "dienstverlening" => [240, 145, 190],
        "gemengd" => [255, 190, 135],
        "groen" => [40, 200, 70],
        "horeca" => [255, 105, 35],
        "kantoor" => [235, 195, 215],
        "maatschappelijk" => [220, 155, 120],
        "natuur" => [130, 165, 145],
        "recreatie" => [185, 215, 70],
        "sport" => [130, 200, 70],
        "tuin" => [200, 215, 110],
        "water" => [175, 205, 225],
        "wonen" => [255, 255, 0],
        "woongebied" => [255, 255, 180],
        "verkeer" | "verkeer - verblijf" => [205, 205, 205],
        _ => [235, 225, 235],
    };
    let double = k.contains("dubbel")
        || g.starts_with("waarde")
        || g.starts_with("leiding")
        || g.starts_with("waterstaat");
    let designation = matches!(
        k.as_str(),
        "enkelbestemming" | "bestemmingsvlak" | "bestemmingsvlakken"
    ) && !double;
    Style {
        color: if designation || double {
            color
        } else {
            [40, 40, 40]
        },
        fill: designation,
        hatch: double,
        dash: k.contains("plangebied") || k.contains("plancontour"),
        weight: if k.contains("bouwvlak") { 35 } else { 18 },
    }
}
pub fn polygons(geometry: &Value) -> Result<Vec<Vec<Vec<XY>>>, String> {
    fn ring(v: &Value) -> Result<Vec<XY>, String> {
        let pts = v
            .as_array()
            .ok_or("ongeldige ring")?
            .iter()
            .map(|p| {
                let a = p.as_array().ok_or("ongeldig coördinaat")?;
                let x = a.first().and_then(Value::as_f64).ok_or("x ontbreekt")?;
                let y = a.get(1).and_then(Value::as_f64).ok_or("y ontbreekt")?;
                if !x.is_finite() || !y.is_finite() {
                    return Err("niet-eindig coördinaat".into());
                }
                Ok([x, y])
            })
            .collect::<Result<Vec<_>, String>>()?;
        if pts.len() < 4 || pts.first() != pts.last() {
            return Err("polygoonring is niet gesloten".into());
        }
        Ok(pts)
    }
    fn poly(v: &Value) -> Result<Vec<Vec<XY>>, String> {
        let rings = v
            .as_array()
            .ok_or("ongeldige polygoon")?
            .iter()
            .map(ring)
            .collect::<Result<Vec<_>, _>>()?;
        if rings.is_empty() {
            Err("lege polygoon".into())
        } else {
            Ok(rings)
        }
    }
    match geometry["type"].as_str().unwrap_or("") {
        "Polygon" => Ok(vec![poly(&geometry["coordinates"])?]),
        "MultiPolygon" => geometry["coordinates"]
            .as_array()
            .ok_or("ongeldige multipolygoon")?
            .iter()
            .map(poly)
            .collect(),
        "GeometryCollection" => {
            let mut out = vec![];
            for g in geometry["geometries"]
                .as_array()
                .ok_or("ongeldige geometriecollectie")?
            {
                out.extend(polygons(g)?);
            }
            Ok(out)
        }
        "Point" | "MultiPoint" | "LineString" | "MultiLineString" => {
            fn check(v: &Value, depth: usize) -> Result<(), String> {
                let a = v.as_array().ok_or("ongeldige coördinaten")?;
                if depth == 0 {
                    if a.len() < 2 || a.iter().any(|n| !n.as_f64().is_some_and(f64::is_finite)) {
                        return Err("ongeldig coördinaat".into());
                    }
                } else {
                    if a.is_empty() {
                        return Err("lege geometrie".into());
                    }
                    for c in a {
                        check(c, depth - 1)?;
                    }
                }
                Ok(())
            }
            let depth = match geometry["type"].as_str().unwrap() {
                "Point" => 0,
                "MultiLineString" => 2,
                _ => 1,
            };
            check(&geometry["coordinates"], depth)?;
            Ok(vec![])
        }
        _ => Err("niet-ondersteunde plangeometrie".into()),
    }
}

pub struct Client {
    key: String,
    base: String,
    agent: ureq::Agent,
}
impl Client {
    pub fn new(key: String) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("Een eigen Ruimtelijke Plannen API-sleutel is nodig; een lokaal GML-bestand kan zonder sleutel worden ingelezen.".into());
        }
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            key,
            base: BASE.into(),
            agent,
        })
    }
    fn request(&self, url: &str, body: Option<&Value>) -> Result<Value, String> {
        if !url.starts_with(&format!("{}/", self.base)) {
            return Err("API-vervolglink buiten de officiële dienst geweigerd".into());
        }
        let resp = if let Some(body) = body {
            self.agent
                .post(url)
                .header("X-Api-Key", &self.key)
                .header("Content-Crs", "epsg:28992")
                .header("Accept-Crs", "epsg:28992")
                .header("Accept", "application/hal+json")
                .header("Content-Type", "application/json")
                .send(body.to_string().as_bytes())
        } else {
            self.agent
                .get(url)
                .header("X-Api-Key", &self.key)
                .header("Accept-Crs", "epsg:28992")
                .header("Accept", "application/hal+json")
                .call()
        };
        let mut resp = resp.map_err(|_| {
            "Plan-API niet bereikbaar; controleer verbinding en dienststatus".to_string()
        })?;
        let code = resp.status().as_u16();
        if !(200..300).contains(&code) {
            return Err(match code {
                401 | 403 => {
                    "Plan-API: API-sleutel ontbreekt, is ongeldig of geeft geen toegang".into()
                }
                429 => "Plan-API: te veel verzoeken; probeer later opnieuw".into(),
                _ => format!("Plan-API antwoordt met HTTP {code}"),
            });
        }
        if let Some(crs) = resp
            .headers()
            .get("Content-Crs")
            .and_then(|v| v.to_str().ok())
        {
            if !crs.to_lowercase().ends_with("28992") {
                return Err("API antwoordt niet in het gevraagde RD-stelsel".into());
            }
        }
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|_| "Plan-API: antwoord te groot of onvolledig")?;
        serde_json::from_slice(&bytes).map_err(|_| "Plan-API: ongeldig JSON-antwoord".into())
    }
    fn collection(
        &self,
        path: &str,
        key: &str,
        body: Option<&Value>,
    ) -> Result<Vec<Value>, String> {
        let mut url = format!("{}{path}", self.base);
        let mut seen = HashSet::new();
        let mut out = vec![];
        loop {
            if !seen.insert(url.clone()) || seen.len() > 120 {
                return Err("Plan-API: ongeldige of te omvangrijke paginering".into());
            }
            let v = self.request(&url, body)?;
            out.extend(
                v["_embedded"][key]
                    .as_array()
                    .ok_or_else(|| format!("Plan-API: collectie {key} ontbreekt"))?
                    .iter()
                    .cloned(),
            );
            match v["_links"]["next"]["href"].as_str() {
                Some(next) => {
                    url = if next.starts_with('?') {
                        format!(
                            "{}{}{}",
                            self.base,
                            path.split('?').next().unwrap_or(path),
                            next
                        )
                    } else if next.starts_with('/') {
                        let prefix = self
                            .base
                            .split("/ruimtelijke-plannen")
                            .next()
                            .ok_or("API-basis")?;
                        format!("{prefix}{next}")
                    } else {
                        next.into()
                    };
                }
                None => break,
            }
        }
        Ok(out)
    }
    pub fn search(&self, point: XY) -> Result<Vec<Plan>, String> {
        if !point.iter().all(|v| v.is_finite()) {
            return Err("ongeldig RD-punt".into());
        }
        let body = json!({"_geo":{"intersects":{"type":"Point","coordinates":point}}});
        self.collection(
            "/plannen/_zoek?page=0&pageSize=100&expand=geometrie",
            "plannen",
            Some(&body),
        )?
        .iter()
        .map(Plan::parse)
        .collect()
    }
    pub fn download(&self, id: &str) -> Result<Data, String> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            return Err("ongeldig plan-ID".into());
        }
        let raw = self.request(
            &format!("{}/plannen/{id}?expand=geometrie", self.base),
            None,
        )?;
        let plan = Plan::parse(&raw)?;
        let mut objects = vec![Object::from_api(&raw, "Plangebied", &plan)?];
        for kind in COLLECTIONS {
            for v in self.collection(
                &format!("/plannen/{id}/{kind}?page=0&pageSize=100&expand=geometrie"),
                kind,
                None,
            )? {
                objects.push(Object::from_api(&v, kind, &plan)?);
            }
        }
        let mut warnings = vec![];
        if plan.historical || plan.legal_end.is_some() {
            warnings.push("Het plan is historisch of heeft een einddatum rechtsgeldigheid; controleer de actuele situatie in Regels op de kaart.".into());
        }
        Ok(Data {
            objects,
            crs: "EPSG:28992".into(),
            warnings,
        })
    }
}
