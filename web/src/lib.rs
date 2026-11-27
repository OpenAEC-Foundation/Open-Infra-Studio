//! Open Infra in de browser: de rekenkern van de GIS-workbench als
//! WebAssembly, voor de webversie van Open CAD Studio.
//!
//! De webversie van Open CAD Studio laadt geen native plugins. Daarom draait
//! hier dezelfde code als in de desktop-plugin — de NLCS-tabellen
//! (`src/nlcs.rs`), de BGT-indeling (`src/gis/bgt.rs`) en het knippen en
//! omzetten van geometrie (`src/gis/vector.rs`), letterlijk meegecompileerd —
//! en levert het resultaat als definities voor `entities_create` van de
//! besturingskoppeling van Open CAD Studio. Wie de webversie host (bv. Baken
//! CAD) haalt de gegevens op (de browser mag naar PDOK) en zet de entities in
//! de geopende tekening.
//!
//! Wat in de browser niet kan (bestanden, SQLite, threads, het netwerk vanuit
//! Rust) zit niet in deze crate. `crs` en `site` zijn hier een RD-versie:
//! de webversie werkt in RD New (EPSG:28992), met RD-coördinaten in de
//! tekening.

use serde::Deserialize;
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[path = "../../src/nlcs.rs"]
#[allow(dead_code)]
pub mod nlcs;

pub mod gis {
    /// RD-only vervanging van `src/gis/crs.rs` (dat EPSG-definities ophaalt).
    pub mod crs {
        pub type XY = [f64; 2];
        pub struct Transformer;
        impl Transformer {
            pub fn new(from: &str, to: &str) -> Result<Self, String> {
                if from == to || (from == "EPSG:28992" && to == "EPSG:28992") {
                    Ok(Transformer)
                } else {
                    Err(format!("de webversie werkt in RD (EPSG:28992), niet {from} → {to}"))
                }
            }
            pub fn apply(&self, p: XY) -> Result<XY, String> {
                Ok(p)
            }
        }
    }

    /// RD-only vervanging van `src/gis/site.rs`: een gebied in RD, met
    /// RD-coördinaten in de tekening (geen basispunt).
    pub mod site {
        use super::crs::{Transformer, XY};
        pub struct Site {
            pub crs: String,
            pub origin_e: f64,
            pub origin_n: f64,
            pub width: f64,
            pub height: f64,
        }
        impl Site {
            pub fn rd(bbox: [f64; 4]) -> Self {
                Site {
                    crs: "EPSG:28992".into(),
                    origin_e: (bbox[0] + bbox[2]) / 2.0,
                    origin_n: (bbox[1] + bbox[3]) / 2.0,
                    width: (bbox[2] - bbox[0]).abs(),
                    height: (bbox[3] - bbox[1]).abs(),
                }
            }
            pub fn bbox(&self) -> [f64; 4] {
                [self.origin_e - self.width / 2.0, self.origin_n - self.height / 2.0, self.origin_e + self.width / 2.0, self.origin_n + self.height / 2.0]
            }
            pub fn to_drawing(&self, p: XY, unit_factor: f64) -> XY {
                [p[0] * unit_factor, p[1] * unit_factor]
            }
            pub fn drawing_rect(&self, unit_factor: f64) -> [f64; 4] {
                let [a, b, c, d] = self.bbox();
                [a * unit_factor, b * unit_factor, c * unit_factor, d * unit_factor]
            }
        }
        pub fn envelope(b: [f64; 4], _t: &Transformer) -> Result<[f64; 4], String> {
            Ok(b)
        }
    }

    /// Ophalen gebeurt in de browser; zie `bgt_verzoeken`.
    pub mod ogcapi {
        use super::vector::Feature;
        pub fn fetch_items(_base: &str, _collection: &str, _bbox: [f64; 4], _epsg: u32, _filter: Option<&str>, _limit: usize, _progress: &dyn Fn(String)) -> Result<(Vec<Feature>, bool), String> {
            Err("in de webversie haalt de browser de gegevens op".into())
        }
    }

    #[path = "../../../src/gis/vector.rs"]
    #[allow(dead_code)]
    pub mod vector;

    #[path = "../../../src/gis/bgt.rs"]
    #[allow(dead_code)]
    pub mod bgt;
}

use gis::{bgt, site::Site, vector};
use nlcs::{Element, Status};

#[derive(Deserialize)]
#[serde(default)]
pub struct Opties {
    /// Gebied in RD: [minx, miny, maxx, maxy].
    pub bbox: [f64; 4],
    /// Tekenstatus: "B" (bestaand, standaard), "N", "T" of "V".
    pub status: String,
    /// NLCS-discipline in de laagnaam ("XX" = algemeen).
    pub discipline: String,
    /// Schaal (500 = 1:500): tekstgrootte en symbolen.
    pub schaal: f64,
    /// Straatnamen en huisnummers.
    pub teksten: bool,
    /// Knippen op het gebied.
    pub knippen: bool,
}

impl Default for Opties {
    fn default() -> Self {
        Opties { bbox: [0.0; 4], status: "B".into(), discipline: "XX".into(), schaal: 500.0, teksten: true, knippen: true }
    }
}

fn status(code: &str) -> Status {
    match code {
        "N" => Status::N,
        "T" => Status::T,
        "V" => Status::V,
        _ => Status::B,
    }
}

/// De BGT-groepen zoals de workbench ze toont (id, naam, collecties, standaard aan).
pub fn groepen() -> Value {
    Value::Array(
        bgt::GROUPS
            .iter()
            .map(|g| json!({ "id": g.id, "naam": g.label, "collecties": g.collections, "standaard": g.default_on }))
            .collect(),
    )
}

/// De eerste URL per collectie voor PDOK (OGC API Features, RD, alleen
/// actuele objecten). Volgende pagina's: de `next`-link in het antwoord.
pub fn bgt_verzoeken(collecties: &[String], bbox: [f64; 4]) -> Value {
    let crs = "http%3A%2F%2Fwww.opengis.net%2Fdef%2Fcrs%2FEPSG%2F0%2F28992";
    let filter = "eind_registratie%20IS%20NULL";
    Value::Array(
        collecties
            .iter()
            .map(|c| {
                json!({
                    "collectie": c,
                    "url": format!("{}/collections/{c}/items?f=json&limit=1000&bbox={},{},{},{}&bbox-crs={crs}&crs={crs}&filter={filter}&filter-lang=cql2-text", bgt::API, bbox[0], bbox[1], bbox[2], bbox[3]),
                })
            })
            .collect(),
    )
}

fn polylijn(pts: &[[f64; 2]], gesloten: bool, laag: &str, kleur: u8) -> Value {
    json!({ "type": "LwPolyline", "vertices": pts, "closed": gesloten, "layer": laag, "color": kleur })
}

/// BGT-objecten → entities voor `entities_create`, op de NLCS-lagen.
/// `data`: { "<collectie>": [GeoJSON-feature, …] } (in RD).
/// Geeft { entities, lagen: [{ naam, kleur, lijntype, lijndikte }], objecten, zonderNlcs }.
pub fn bgt_tekenen(data: &Value, opt: &Opties) -> Result<Value, String> {
    if opt.bbox[2] <= opt.bbox[0] || opt.bbox[3] <= opt.bbox[1] {
        return Err("geef een gebied in RD: [minx, miny, maxx, maxy]".into());
    }
    let site = Site::rd(opt.bbox);
    let to_site = vector::data_to_site(&site, "EPSG:28992", &[])?;
    let st = status(&opt.status);
    let papier = |mm: f64| mm * opt.schaal / 1000.0;
    let mut entities = Vec::new();
    let mut lagen = std::collections::BTreeMap::new();
    let (mut objecten, mut zonder) = (0usize, 0usize);
    let obj = data.as_object().ok_or("data: een object per collectie")?;
    for (collectie, features) in obj {
        let fc = json!({ "features": features });
        for feat in vector::parse_geojson(&fc) {
            if !bgt::is_current(&feat) {
                continue;
            }
            objecten += 1;
            if bgt::is_label(collectie) {
                if !opt.teksten {
                    continue;
                }
                let (Some(o), Some((tekst, hoek))) = (nlcs::bgt_label_object(collectie), bgt::label(collectie, &feat)) else { continue };
                let spec = nlcs::layer(o, st, &opt.discipline, Element::T);
                let mm = if collectie == "openbareruimtelabel" { 2.5 } else { 1.8 };
                let (_, punten) = vector::feature_to_drawing(&site, &feat, &*to_site, 1.0, opt.knippen);
                for p in punten {
                    entities.push(json!({ "type": "Text", "value": tekst, "position": p, "height": papier(mm), "rotation_deg": leesbaar(hoek).to_degrees(), "layer": spec.name, "color": spec.color }));
                }
                lagen.insert(spec.name.clone(), spec);
                continue;
            }
            let id = nlcs::bgt_lookup(collectie, &feat.props).map(|(id, _)| id).or_else(|| nlcs::bgt_default(collectie));
            let Some(o) = id.and_then(nlcs::object) else {
                zonder += 1;
                continue;
            };
            let (lijnen, punten) = vector::feature_to_drawing(&site, &feat, &*to_site, 1.0, opt.knippen);
            if !lijnen.is_empty() {
                let spec = nlcs::layer(o, st, &opt.discipline, Element::G);
                entities.extend(lijnen.iter().map(|(pts, gesloten)| polylijn(pts, *gesloten, &spec.name, spec.color)));
                lagen.insert(spec.name.clone(), spec);
            }
            if !punten.is_empty() {
                let spec = nlcs::layer(o, st, &opt.discipline, Element::S);
                entities.extend(punten.iter().map(|p| json!({ "type": "Circle", "center": p, "radius": papier(0.5), "layer": spec.name, "color": spec.color })));
                lagen.insert(spec.name.clone(), spec);
            }
        }
    }
    Ok(json!({
        "entities": entities,
        "lagen": lagen.values().map(|s| json!({ "naam": s.name, "kleur": s.color, "lijntype": s.linetype, "lijndikte": s.lineweight })).collect::<Vec<_>>(),
        "objecten": objecten,
        "zonderNlcs": zonder,
    }))
}

/// Leesrichting: nooit ondersteboven (zoals `emit::readable` in de plugin).
fn leesbaar(a: f64) -> f64 {
    let a = a.rem_euclid(std::f64::consts::TAU);
    if a > std::f64::consts::FRAC_PI_2 && a <= 3.0 * std::f64::consts::FRAC_PI_2 {
        a - std::f64::consts::PI
    } else {
        a
    }
}

// ── de JavaScript-kant ─────────────────────────────────────────────────────

#[wasm_bindgen(js_name = versie)]
pub fn js_versie() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(js_name = bgtGroepen)]
pub fn js_bgt_groepen() -> String {
    groepen().to_string()
}

#[wasm_bindgen(js_name = bgtVerzoeken)]
pub fn js_bgt_verzoeken(collecties_json: &str, bbox_json: &str) -> Result<String, JsError> {
    let c: Vec<String> = serde_json::from_str(collecties_json).map_err(|e| JsError::new(&e.to_string()))?;
    let b: [f64; 4] = serde_json::from_str(bbox_json).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(bgt_verzoeken(&c, b).to_string())
}

#[wasm_bindgen(js_name = bgtTekenen)]
pub fn js_bgt_tekenen(data_json: &str, opties_json: &str) -> Result<String, JsError> {
    let data: Value = serde_json::from_str(data_json).map_err(|e| JsError::new(&e.to_string()))?;
    let opt: Opties = serde_json::from_str(opties_json).map_err(|e| JsError::new(&e.to_string()))?;
    bgt_tekenen(&data, &opt).map(|v| v.to_string()).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn een_wegdeel_komt_op_een_nlcs_laag_en_wordt_geknipt() {
        let data = json!({ "wegdeel": [{
            "type": "Feature",
            "properties": { "function": "rijbaan lokale weg", "fysiek_voorkomen": "gesloten verharding" },
            "geometry": { "type": "Polygon", "coordinates": [[[155000.0, 463000.0], [155200.0, 463000.0], [155200.0, 463010.0], [155000.0, 463010.0], [155000.0, 463000.0]]] }
        }]});
        let opt = Opties { bbox: [155050.0, 462990.0, 155100.0, 463020.0], ..Default::default() };
        let uit = bgt_tekenen(&data, &opt).unwrap();
        let e = &uit["entities"][0];
        assert_eq!(e["type"], "LwPolyline");
        assert_eq!(e["closed"], true);
        assert!(e["layer"].as_str().unwrap().starts_with("B-XX-"), "laag {}", e["layer"]);
        for v in e["vertices"].as_array().unwrap() {
            let x = v[0].as_f64().unwrap();
            assert!((155050.0..=155100.0).contains(&x), "geknipt op het gebied: {x}");
        }
        assert_eq!(uit["objecten"], 1);
    }

    #[test]
    fn straatnaam_wordt_tekst_en_historie_valt_weg() {
        let data = json!({ "openbareruimtelabel": [
            { "type": "Feature", "properties": { "openbareruimtenaam": "Dorpsstraat", "hoek": 180.0 }, "geometry": { "type": "Point", "coordinates": [155060.0, 463005.0] } },
            { "type": "Feature", "properties": { "openbareruimtenaam": "Oud", "eind_registratie": "2020-01-01" }, "geometry": { "type": "Point", "coordinates": [155060.0, 463005.0] } }
        ]});
        let opt = Opties { bbox: [155050.0, 462990.0, 155100.0, 463020.0], ..Default::default() };
        let uit = bgt_tekenen(&data, &opt).unwrap();
        let ents = uit["entities"].as_array().unwrap();
        assert_eq!(ents.len(), 1);
        assert_eq!(ents[0]["type"], "Text");
        assert_eq!(ents[0]["value"], "Dorpsstraat");
        assert!(ents[0]["rotation_deg"].as_f64().unwrap().abs() < 1e-9, "nooit ondersteboven");
    }

    #[test]
    fn groepen_en_verzoeken() {
        assert!(groepen().as_array().unwrap().iter().any(|g| g["id"] == "wegen"));
        let v = bgt_verzoeken(&["wegdeel".into()], [1.0, 2.0, 3.0, 4.0]);
        assert!(v[0]["url"].as_str().unwrap().contains("/collections/wegdeel/items?f=json&limit=1000&bbox=1,2,3,4"));
    }
}
