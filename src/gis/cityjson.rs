//! CityJSON reader (1.0, 1.1, 2.0) and CityJSONSeq (`.city.jsonl`): the
//! same city objects as the CityGML reader. A whole CityJSON file is read
//! with its objects left as raw text and parsed one at a time, so files of
//! a whole city (hundreds of MB) do not become one huge JSON tree.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::Value;

use super::citygml::{epsg, theme_of, CityModel, CityObject, Part};

#[derive(Deserialize, Default)]
struct Transform {
    scale: [f64; 3],
    translate: [f64; 3],
}

#[derive(Deserialize)]
struct Doc<'a> {
    #[serde(default)]
    transform: Option<Transform>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(rename = "CityObjects", borrow, default)]
    objects: HashMap<String, &'a RawValue>,
    #[serde(default)]
    vertices: Vec<[f64; 3]>,
}

/// The CRS of a CityJSON text, without parsing it: the first
/// `"referenceSystem": "…"` (CityJSON 1.0 `"crs": {"epsg": n}` too).
pub fn crs_of(text: &str) -> Option<String> {
    if let Some(i) = text.find("\"referenceSystem\"") {
        let rest = &text[i + 17..];
        let a = rest.find('"')? + 1;
        let b = rest[a..].find('"')?;
        return epsg(&rest[a..a + b]);
    }
    let i = text.find("\"epsg\"")?;
    let digits: String = text[i + 6..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    epsg(&format!("EPSG:{digits}"))
}

fn part_of(t: &str) -> Part {
    match t {
        "RoofSurface" => Part::Roof,
        "WallSurface" | "ClosureSurface" | "OuterCeilingSurface" => Part::Wall,
        "GroundSurface" | "OuterFloorSurface" => Part::Ground,
        _ => Part::Any,
    }
}

fn lod_rank(g: &Value) -> f64 {
    match &g["lod"] {
        Value::String(s) => s.parse().unwrap_or(0.0),
        v => v.as_f64().unwrap_or(0.0),
    }
}

fn year(a: &Value) -> Option<i64> {
    ["yearOfConstruction", "oorspronkelijkbouwjaar", "bouwjaar", "construction_year"].iter().find_map(|k| match &a[*k] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.get(..4).and_then(|y| y.parse().ok()),
        _ => None,
    })
}

fn height(a: &Value) -> Option<f64> {
    a["measuredHeight"].as_f64().or_else(|| Some(a["b3_h_dak_70p"].as_f64()? - a["b3_h_maaiveld"].as_f64()?))
}

/// One city object: its highest LoD geometry with semantics.
fn convert(id: &str, o: &Value, parent: Option<&Value>, verts: &[[f64; 3]]) -> Option<CityObject> {
    let class = o["type"].as_str().unwrap_or("").trim_start_matches('+').to_string();
    let geoms: Vec<&Value> = o["geometry"].as_array()?.iter().filter(|g| matches!(g["type"].as_str(), Some("MultiSurface" | "CompositeSurface" | "Solid" | "MultiSolid" | "CompositeSolid"))).collect();
    let top = geoms.iter().map(|g| lod_rank(g)).fold(f64::MIN, f64::max);
    let g = *geoms.iter().find(|g| lod_rank(g) == top)?;
    let sem: Vec<Part> = g["semantics"]["surfaces"].as_array().into_iter().flatten().map(|s| part_of(s["type"].as_str().unwrap_or(""))).collect();
    let ring = |r: &Value| -> Vec<[f64; 3]> { r.as_array().into_iter().flatten().filter_map(|i| verts.get(i.as_u64()? as usize).copied()).collect() };
    let mut surfaces = Vec::new();
    let mut push = |s: &Value, v: &Value| {
        let rings: Vec<Vec<[f64; 3]>> = s.as_array().into_iter().flatten().map(ring).filter(|r| r.len() >= 3).collect();
        if !rings.is_empty() {
            surfaces.push((v.as_u64().and_then(|i| sem.get(i as usize).copied()).unwrap_or(Part::Any), rings));
        }
    };
    let b = &g["boundaries"];
    let vals = &g["semantics"]["values"];
    match g["type"].as_str()? {
        // [surface][ring][vertex]; values [surface].
        "MultiSurface" | "CompositeSurface" => {
            for (i, s) in b.as_array()?.iter().enumerate() {
                push(s, &vals[i]);
            }
        }
        // [shell][surface][ring][vertex]; values [shell][surface].
        "Solid" => {
            for (h, shell) in b.as_array()?.iter().enumerate() {
                for (i, s) in shell.as_array().into_iter().flatten().enumerate() {
                    push(s, &vals[h][i]);
                }
            }
        }
        // [solid][shell][surface][ring][vertex].
        _ => {
            for (k, solid) in b.as_array()?.iter().enumerate() {
                for (h, shell) in solid.as_array().into_iter().flatten().enumerate() {
                    for (i, s) in shell.as_array().into_iter().flatten().enumerate() {
                        push(s, &vals[k][h][i]);
                    }
                }
            }
        }
    }
    if surfaces.is_empty() {
        return None;
    }
    let own = &o["attributes"];
    let pick = |f: &dyn Fn(&Value) -> Option<i64>| f(own).or_else(|| parent.and_then(|p| f(&p["attributes"])));
    let ident = own["identificatie"].as_str().or_else(|| parent.and_then(|p| p["attributes"]["identificatie"].as_str())).unwrap_or(id).to_string();
    let look = ["function", "class", "usage", "bgt_fysiekvoorkomen", "plus_fysiekvoorkomen"]
        .iter()
        .filter_map(|k| own[*k].as_str())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    Some(CityObject {
        theme: theme_of(&class),
        year: pick(&year),
        height: height(own).or_else(|| parent.and_then(|p| height(&p["attributes"]))),
        id: ident,
        look,
        class,
        surfaces,
        flat: false,
    })
}

fn apply(t: &Option<Transform>, v: &mut [[f64; 3]]) {
    if let Some(t) = t {
        for p in v {
            for k in 0..3 {
                p[k] = p[k] * t.scale[k] + t.translate[k];
            }
        }
    }
}

/// Read CityJSON or CityJSONSeq text; `keep` decides per object.
pub fn parse_with(text: &str, keep: &dyn Fn(&CityObject) -> bool, progress: &dyn Fn(String)) -> Result<CityModel, String> {
    let mut model = CityModel { crs: crs_of(text), ..Default::default() };
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next().ok_or("CityJSON: leeg bestand")?;
    let seq = serde_json::from_str::<Value>(first).is_ok_and(|v| v["type"] == "CityJSON") && text.contains("\"CityJSONFeature\"");
    if seq {
        // CityJSONSeq: a header line, then one feature (with its own
        // vertices) per line.
        let header: Value = serde_json::from_str(first).map_err(|e| format!("CityJSON: {e}"))?;
        let t: Option<Transform> = serde_json::from_value(header["transform"].clone()).ok();
        for (n, line) in lines.enumerate() {
            if n % 5000 == 0 && n > 0 {
                progress(format!("CityJSON: {n} objecten gelezen…"));
            }
            let Ok(f) = serde_json::from_str::<Value>(line) else { continue };
            let mut verts: Vec<[f64; 3]> = serde_json::from_value(f["vertices"].clone()).unwrap_or_default();
            apply(&t, &mut verts);
            let Some(objects) = f["CityObjects"].as_object() else { continue };
            for (id, o) in objects {
                if o["children"].as_array().is_some_and(|c| !c.is_empty()) {
                    continue;
                }
                let parent = o["parents"].get(0).and_then(Value::as_str).and_then(|p| objects.get(p));
                if let Some(obj) = convert(id, o, parent, &verts) {
                    model.seen += 1;
                    if keep(&obj) {
                        model.objects.push(obj);
                    }
                }
            }
        }
    } else {
        progress("CityJSON ontleden…".into());
        let t0 = std::time::Instant::now();
        let mut doc: Doc = serde_json::from_str(text).map_err(|e| format!("CityJSON: {e}"))?;
        if std::env::var_os("CITY_TIMING").is_some() {
            eprintln!("doc {:.2} s", t0.elapsed().as_secs_f64());
        }
        // 3D BAG API pages keep the transform in the metadata.
        let t = doc.transform.take().or_else(|| doc.metadata.as_ref().and_then(|m| serde_json::from_value(m["transform"].clone()).ok()));
        apply(&t, &mut doc.vertices);
        let total = doc.objects.len();
        for (n, (id, raw)) in doc.objects.iter().enumerate() {
            if n % 5000 == 0 && n > 0 {
                progress(format!("CityJSON: {n} van {total} objecten…"));
            }
            let Ok(o) = serde_json::from_str::<Value>(raw.get()) else { continue };
            if o["geometry"].as_array().is_none_or(|g| g.is_empty()) || o["children"].as_array().is_some_and(|c| !c.is_empty()) {
                continue;
            }
            let parent: Option<Value> = o["parents"].get(0).and_then(Value::as_str).and_then(|p| doc.objects.get(p)).and_then(|r| serde_json::from_str(r.get()).ok());
            if let Some(obj) = convert(id, &o, parent.as_ref(), &doc.vertices) {
                model.seen += 1;
                if keep(&obj) {
                    model.objects.push(obj);
                }
            }
        }
    }
    if model.seen == 0 {
        return Err("CityJSON: geen stadsobjecten met geometrie gevonden".into());
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gis::citygml::Theme;

    #[test]
    fn building_with_part_and_semantics() {
        let doc = r#"{"type":"CityJSON","version":"2.0","transform":{"scale":[0.001,0.001,0.001],"translate":[100000,400000,0]},
          "metadata":{"referenceSystem":"https://www.opengis.net/def/crs/EPSG/0/7415"},
          "CityObjects":{
            "b":{"type":"Building","attributes":{"oorspronkelijkbouwjaar":1925,"identificatie":"NL.IMBAG.Pand.1"},"children":["b-0"]},
            "b-0":{"type":"BuildingPart","parents":["b"],"geometry":[
               {"type":"Solid","lod":"1.2","boundaries":[[[[0,1,2,3]]]]},
               {"type":"Solid","lod":"2.2","boundaries":[[[[0,1,2,3]],[[4,5,6,7]]]],
                "semantics":{"surfaces":[{"type":"GroundSurface"},{"type":"RoofSurface"}],"values":[[0,1]]}}]}},
          "vertices":[[0,0,0],[10000,0,0],[10000,10000,0],[0,10000,0],[0,0,6000],[10000,0,6000],[10000,10000,9000],[0,10000,9000]]}"#;
        let m = parse_with(doc, &|_| true, &|_| {}).unwrap();
        assert_eq!(m.crs.as_deref(), Some("EPSG:28992"));
        assert_eq!(m.objects.len(), 1);
        let b = &m.objects[0];
        assert_eq!((b.theme, b.year, b.id.as_str()), (Theme::Building, Some(1925), "NL.IMBAG.Pand.1"));
        assert_eq!(b.surfaces.len(), 2, "LoD 2.2 only");
        assert_eq!(b.surfaces[1].0, Part::Roof);
        assert_eq!(b.surfaces[1].1[0][2], [100010.0, 400010.0, 9.0]);
    }

    #[test]
    fn cityjson_seq() {
        let text = "{\"type\":\"CityJSON\",\"version\":\"2.0\",\"transform\":{\"scale\":[1,1,1],\"translate\":[0,0,0]},\"metadata\":{\"referenceSystem\":\"https://www.opengis.net/def/crs/EPSG/0/2056\"},\"CityObjects\":{},\"vertices\":[]}\n\
{\"type\":\"CityJSONFeature\",\"id\":\"r\",\"CityObjects\":{\"r\":{\"type\":\"Road\",\"geometry\":[{\"type\":\"MultiSurface\",\"lod\":1,\"boundaries\":[[[0,1,2]]]}]}},\"vertices\":[[0,0,0],[5,0,0],[5,5,0]]}\n";
        let m = parse_with(text, &|_| true, &|_| {}).unwrap();
        assert_eq!(m.crs.as_deref(), Some("EPSG:2056"));
        assert_eq!((m.objects[0].theme, m.seen), (Theme::Transport, 1));
    }
}
