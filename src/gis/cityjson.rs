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
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    transform: Option<Transform>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(rename = "CityObjects", borrow)]
    objects: HashMap<String, &'a RawValue>,
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
fn convert(id: &str, o: &Value, parent: Option<&Value>, verts: &[[f64; 3]]) -> Result<Option<CityObject>, String> {
    let class = o["type"].as_str().unwrap_or("").trim_start_matches('+').to_string();
    let Some(geometry) = o["geometry"].as_array() else { return Ok(None) };
    let geoms: Vec<&Value> = geometry.iter().filter(|g| matches!(g["type"].as_str(), Some("MultiSurface" | "CompositeSurface" | "Solid" | "MultiSolid" | "CompositeSolid"))).collect();
    let top = geoms.iter().map(|g| lod_rank(g)).fold(f64::MIN, f64::max);
    let Some(&g) = geoms.iter().find(|g| lod_rank(g) == top) else { return Ok(None) };
    let sem: Vec<Part> = g["semantics"]["surfaces"].as_array().into_iter().flatten().map(|s| part_of(s["type"].as_str().unwrap_or(""))).collect();
    let invalid = || format!("CityJSON: object {id}: ongeldige geometrie of vertexindex");
    fn array(v: &Value) -> Result<&Vec<Value>, String> {
        v.as_array().ok_or_else(|| "CityJSON: geometrie-array verwacht".into())
    }
    let ring = |r: &Value| -> Result<Vec<[f64; 3]>, String> {
        let points = array(r)?.iter().map(|i| {
            i.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| verts.get(i).copied()).ok_or_else(invalid)
        }).collect::<Result<Vec<_>, _>>()?;
        if points.len() < 3 { return Err(invalid()); }
        Ok(points)
    };
    let mut surfaces = Vec::new();
    let mut push = |s: &Value, v: &Value| -> Result<(), String> {
        let rings = array(s)?.iter().map(ring).collect::<Result<Vec<_>, _>>()?;
        if rings.is_empty() { return Err(invalid()); }
        surfaces.push((v.as_u64().and_then(|i| sem.get(i as usize).copied()).unwrap_or(Part::Any), rings));
        Ok(())
    };
    let b = &g["boundaries"];
    let vals = &g["semantics"]["values"];
    match g["type"].as_str().unwrap_or("") {
        // [surface][ring][vertex]; values [surface].
        "MultiSurface" | "CompositeSurface" => {
            for (i, s) in array(b)?.iter().enumerate() {
                push(s, &vals[i])?;
            }
        }
        // [shell][surface][ring][vertex]; values [shell][surface].
        "Solid" => {
            for (h, shell) in array(b)?.iter().enumerate() {
                for (i, s) in array(shell)?.iter().enumerate() {
                    push(s, &vals[h][i])?;
                }
            }
        }
        // [solid][shell][surface][ring][vertex].
        _ => {
            for (k, solid) in array(b)?.iter().enumerate() {
                for (h, shell) in array(solid)?.iter().enumerate() {
                    for (i, s) in array(shell)?.iter().enumerate() {
                        push(s, &vals[k][h][i])?;
                    }
                }
            }
        }
    }
    if surfaces.is_empty() {
        return Ok(None);
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
    Ok(Some(CityObject {
        theme: theme_of(&class),
        year: pick(&year),
        height: height(own).or_else(|| parent.and_then(|p| height(&p["attributes"]))),
        id: ident,
        look,
        class,
        surfaces,
        flat: false,
    }))
}

fn apply(t: &Option<Transform>, v: &mut [[f64; 3]]) -> Result<(), String> {
    for (i, p) in v.iter_mut().enumerate() {
        for k in 0..3 {
            if let Some(t) = t {
                p[k] = p[k] * t.scale[k] + t.translate[k];
            }
            if !p[k].is_finite() {
                return Err(format!("CityJSON: vertex {i}: niet-eindige coördinaat na transformatie"));
            }
        }
    }
    Ok(())
}

/// Read CityJSON or CityJSONSeq text; `keep` decides per object.
pub fn parse_with(text: &str, keep: &dyn Fn(&CityObject) -> bool, progress: &dyn Fn(String)) -> Result<CityModel, String> {
    let mut model = CityModel { crs: crs_of(text), ..Default::default() };
    let mut lines = text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty());
    let (header_line, first) = lines.next().ok_or("CityJSON: leeg bestand")?;
    let seq = serde_json::from_str::<Value>(first).is_ok_and(|v| v["type"] == "CityJSON") && lines.clone().next().is_some();
    if seq {
        // CityJSONSeq: a header line, then one feature (with its own
        // vertices) per line.
        let header: Value = serde_json::from_str(first).map_err(|e| format!("CityJSON: {e}"))?;
        let t: Option<Transform> = serde_json::from_value(header["transform"].clone()).map_err(|e| format!("CityJSON: regel {}: transformatie: {e}", header_line + 1))?;
        for (n, (line_index, line)) in lines.enumerate() {
            let line_number = line_index + 1;
            if n % 5000 == 0 && n > 0 {
                progress(format!("CityJSON: {n} objecten gelezen…"));
            }
            let mut f: Doc = serde_json::from_str(line).map_err(|e| format!("CityJSON: regel {line_number}: {e}"))?;
            if f.kind.as_deref() != Some("CityJSONFeature") {
                return Err(format!("CityJSON: regel {line_number}: CityJSONFeature verwacht"));
            }
            apply(&t, &mut f.vertices)?;
            for (id, raw) in &f.objects {
                let o: Value = serde_json::from_str(raw.get()).map_err(|e| format!("CityJSON: regel {line_number}: object {id}: {e}"))?;
                if o["children"].as_array().is_some_and(|c| !c.is_empty()) {
                    continue;
                }
                let parent: Option<Value> = o["parents"].get(0).and_then(Value::as_str).and_then(|p| f.objects.get(p)).and_then(|r| serde_json::from_str(r.get()).ok());
                if let Some(obj) = convert(id, &o, parent.as_ref(), &f.vertices)? {
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
        let t = match doc.transform.take() {
            Some(t) => Some(t),
            None => serde_json::from_value(doc.metadata.as_ref().map(|m| m["transform"].clone()).unwrap_or(Value::Null)).map_err(|e| format!("CityJSON: transformatie: {e}"))?,
        };
        apply(&t, &mut doc.vertices)?;
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
            if let Some(obj) = convert(id, &o, parent.as_ref(), &doc.vertices)? {
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

    const BUILDING: &str = include_str!("../../tests/fixtures/cityjson/building.city.json");
    const SEQUENCE: &str = include_str!("../../tests/fixtures/cityjson/building.city.jsonl");

    #[test]
    fn fixture_and_file_route_preserve_crs_transform_and_filter() {
        for text in [BUILDING, SEQUENCE] {
            let model = parse_with(text, &|_| true, &|_| {}).unwrap();
            assert_eq!(model.crs.as_deref(), Some("EPSG:2056"));
            assert_eq!((model.seen, model.objects.len()), (1, 1));
            assert_eq!(model.objects[0].surfaces[0].1[0][2], [2600010.0, 1200010.0, 5.0]);
            let filtered = parse_with(text, &|_| false, &|_| {}).unwrap();
            assert_eq!((filtered.seen, filtered.objects.len()), (1, 0));
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cityjson/building.city.jsonl");
        let model = super::super::citygml::read_file(&path, &|crs| {
            assert_eq!(crs, Some("EPSG:2056"));
            Ok(Box::new(|_| true))
        }, &|_| {}).unwrap();
        assert_eq!(model.objects.len(), 1);
    }

    #[test]
    fn sequence_rejects_broken_lines_instead_of_partial_success() {
        for line in ["{", "not-json", "null", r#"{"type":"Other"}"#] {
            let error = parse_with(&format!("{SEQUENCE}{line}\n"), &|_| true, &|_| {}).unwrap_err();
            assert!(error.starts_with("CityJSON:"), "{error}");
            assert!(error.contains('3'), "line number missing: {error}");
        }
    }

    #[test]
    fn sequence_rejects_missing_or_invalid_vertex_arrays() {
        for vertices in ["null", "[]", "[[0,0]]", "[[0,0,0],[1,0,0],[1,1,0],[0,1,\"bad\"]]"] {
            let mut rows: Vec<Value> = SEQUENCE.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
            rows[1]["vertices"] = serde_json::from_str(vertices).unwrap();
            let text = rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
            assert!(parse_with(&text, &|_| true, &|_| {}).is_err(), "{vertices}");
        }
        let mut rows: Vec<Value> = SEQUENCE.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        rows[1].as_object_mut().unwrap().remove("vertices");
        let text = rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        assert!(parse_with(&text, &|_| true, &|_| {}).is_err());
    }

    #[test]
    fn rejects_invalid_transforms_and_overflow_in_documents_and_sequences() {
        for text in [BUILDING, SEQUENCE] {
            for scale in ["null", "[1,1]", "[1,1,\"bad\"]", "[1e308,1,1]"] {
                let bad = if text == BUILDING {
                    let mut doc: Value = serde_json::from_str(text).unwrap();
                    doc["transform"]["scale"] = serde_json::from_str(scale).unwrap();
                    doc.to_string()
                } else {
                    let mut rows: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
                    rows[0]["transform"]["scale"] = serde_json::from_str(scale).unwrap();
                    rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
                };
                assert!(parse_with(&bad, &|_| true, &|_| {}).is_err(), "{scale}");
            }
        }
    }

    #[test]
    fn empty_missing_and_truncated_input_return_errors() {
        for text in ["", "  \n", "{}", r#"{"type":"CityJSON","CityObjects":{},"vertices":[]}"#, "{\"vertices\":"] {
            assert!(parse_with(text, &|_| true, &|_| {}).unwrap_err().starts_with("CityJSON:"));
        }
        let large = BUILDING.replace("1000", "1e100");
        let model = parse_with(&large, &|_| true, &|_| {}).unwrap();
        assert!(model.objects[0].surfaces[0].1[0].iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn rejects_invalid_indices_without_shortening_or_replacing_rings() {
        for boundaries in ["[[[0,1,2,999]]]", "[[[0,1,2,-1]]]", "[[[0,1,2,18446744073709551615]]]", "[[[0,1,2,\"bad\"]]]", "[[[0,1]]]", "[[[999,998,997],[0,1,2]]]"] {
            for text in [BUILDING, SEQUENCE] {
                let bad = if text == BUILDING {
                    let mut doc: Value = serde_json::from_str(text).unwrap();
                    doc["CityObjects"]["building"]["geometry"][0]["boundaries"] = serde_json::from_str(boundaries).unwrap();
                    doc.to_string()
                } else {
                    let mut rows: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
                    rows[1]["CityObjects"]["building"]["geometry"][0]["boundaries"] = serde_json::from_str(boundaries).unwrap();
                    rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
                };
                assert!(parse_with(&bad, &|_| true, &|_| {}).is_err(), "{boundaries}");
            }
        }
    }

    #[test]
    fn sequence_error_reports_physical_line_including_blank_lines() {
        let bad = format!("\n{SEQUENCE}\n{{");
        let error = parse_with(&bad, &|_| true, &|_| {}).unwrap_err();
        assert!(error.contains("regel 5"), "{error}");
    }

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
