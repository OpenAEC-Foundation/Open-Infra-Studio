//! Local IMRO GML, including nested attributes and in-document geometry references.
use super::{Data, Object};
use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub fn is_imro(text: &str) -> bool {
    Document::parse(text).is_ok_and(|d| {
        d.descendants().any(|n| {
            n.is_element()
                && n.tag_name()
                    .namespace()
                    .is_some_and(|s| s.contains("/imro/"))
        })
    })
}

/// Keep compound attributes intact: names and values in repeated records belong
/// together, and an xlink association can have no text content at all.
fn attribute_value(node: Node, depth: usize) -> Result<Value, String> {
    if depth > 64 {
        return Err("IMRO-attributen te diep genest".into());
    }
    let mut values = Map::new();
    for attribute in node.attributes() {
        let name = if attribute.namespace() == Some("http://www.w3.org/1999/xlink") {
            format!("@xlink:{}", attribute.name())
        } else {
            format!("@{}", attribute.name())
        };
        values.insert(name, attribute.value().into());
    }
    for child in node.children().filter(|n| n.is_element()) {
        let value = attribute_value(child, depth + 1)?;
        let key = child.tag_name().name().to_string();
        if let Some(previous) = values.get_mut(&key) {
            match previous {
                Value::Array(items) => items.push(value),
                previous => {
                    let first = std::mem::replace(previous, Value::Null);
                    *previous = Value::Array(vec![first, value]);
                }
            }
        } else {
            values.insert(key, value);
        }
    }
    let text = node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect::<String>();
    let text = text.trim();
    if values.is_empty() {
        return Ok(Value::String(text.into()));
    }
    if !text.is_empty() {
        values.insert("#text".into(), text.into());
    }
    Ok(Value::Object(values))
}

fn compound_attributes(feature: Node) -> Result<Value, String> {
    let mut attributes = Map::new();
    for child in feature.children().filter(|node| {
        node.is_element()
            && node
                .tag_name()
                .namespace()
                .is_some_and(|s| s.contains("/imro/"))
            && !["geometrie", "geometry", "positie", "begrenzing"].contains(&node.tag_name().name())
    }) {
        let value = attribute_value(child, 0)?;
        let key = child.tag_name().name().to_string();
        if let Some(previous) = attributes.get_mut(&key) {
            match previous {
                Value::Array(items) => items.push(value),
                previous => {
                    let first = std::mem::replace(previous, Value::Null);
                    *previous = Value::Array(vec![first, value]);
                }
            }
        } else {
            attributes.insert(key, value);
        }
    }
    Ok(Value::Object(attributes))
}
fn coords(
    node: Node,
    ids: &HashMap<String, Node>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Result<Vec<[f64; 2]>, String> {
    if depth > 64 {
        return Err("GML-coördinaten te diep genest".into());
    }
    if let Some(href) = node
        .attributes()
        .find(|a| a.name() == "href")
        .map(|a| a.value())
    {
        let id = href.strip_prefix('#').ok_or(
            "GML bevat externe geometriereferentie; lever een zelfstandig GML-bestand aan",
        )?;
        if !seen.insert(id.into()) {
            return Err("kringverwijzing in GML".into());
        }
        let out = coords(
            *ids.get(id)
                .ok_or_else(|| format!("ontbrekende GML-geometrie {id}"))?,
            ids,
            seen,
            depth + 1,
        );
        seen.remove(id);
        return out;
    }
    let name = node.tag_name().name();
    if matches!(
        name,
        "Arc"
            | "ArcString"
            | "Circle"
            | "CircleByCenterPoint"
            | "ArcByCenterPoint"
            | "BSpline"
            | "Bezier"
            | "Clothoid"
            | "GeodesicString"
    ) || node.attribute("orientation") == Some("-")
    {
        return Err("IMRO bevat een gebogen of omgekeerd georiënteerd GML-segment; exporteer dit eerst als gesegmenteerde lijnen".into());
    }
    if ["posList", "pos", "coordinates"].contains(&name) {
        let text = node.text().unwrap_or("");
        let dimension = node
            .ancestors()
            .find_map(|n| n.attribute("srsDimension"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(2);
        if !(2..=3).contains(&dimension) {
            return Err("onbekende GML-coördinaatdimensie".into());
        }
        let nums = text
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.parse::<f64>()
                    .map_err(|_| "ongeldig GML-coördinaat".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if nums.len() % dimension != 0 || nums.iter().any(|n| !n.is_finite()) {
            return Err("onvolledige of niet-eindige GML-coördinaten".into());
        }
        return Ok(nums.chunks(dimension).map(|v| [v[0], v[1]]).collect());
    }
    let mut out = vec![];
    for c in node.children().filter(|c| c.is_element()) {
        let mut p = coords(c, ids, seen, depth + 1)?;
        if out.last().is_some() && p.first() == out.last() {
            p.remove(0);
        }
        out.extend(p);
    }
    Ok(out)
}
fn geometry(
    node: Node,
    ids: &HashMap<String, Node>,
    seen: &mut HashSet<String>,
    depth: usize,
) -> Result<Option<Value>, String> {
    if depth > 64 {
        return Err("GML-geometrie te diep genest".into());
    }
    if let Some(href) = node
        .attributes()
        .find(|a| a.name() == "href")
        .map(|a| a.value())
    {
        let id = href
            .strip_prefix('#')
            .ok_or("externe GML-geometriereferentie wordt niet automatisch opgehaald")?;
        if !seen.insert(id.into()) {
            return Err("kringverwijzing in GML".into());
        }
        let g = geometry(
            *ids.get(id)
                .ok_or_else(|| format!("ontbrekende GML-geometrie {id}"))?,
            ids,
            seen,
            depth + 1,
        );
        seen.remove(id);
        return g;
    }
    let name = node.tag_name().name();
    if ["Polygon", "PolygonPatch"].contains(&name) {
        let mut rings = vec![];
        let mut boundaries = node
            .children()
            .filter(|c| {
                c.is_element()
                    && ["exterior", "outerBoundaryIs", "interior", "innerBoundaryIs"]
                        .contains(&c.tag_name().name())
            })
            .collect::<Vec<_>>();
        boundaries.sort_by_key(|b| !matches!(b.tag_name().name(), "exterior" | "outerBoundaryIs"));
        if boundaries
            .iter()
            .filter(|b| matches!(b.tag_name().name(), "exterior" | "outerBoundaryIs"))
            .count()
            != 1
        {
            return Err("IMRO-polygoon moet één buitenring bevatten".into());
        }
        for boundary in boundaries {
            let points = coords(boundary, ids, &mut HashSet::new(), 0)?;
            if points.len() < 4 || points.first() != points.last() {
                return Err("IMRO bevat een ongesloten polygoonring".into());
            }
            rings.push(points);
        }
        if rings.is_empty() {
            return Err("IMRO-polygoon heeft geen grenzen".into());
        }
        return Ok(Some(json!({"type":"Polygon","coordinates":rings})));
    }
    if ["Point", "LineString", "Curve", "LinearRing", "Ring"].contains(&name) {
        let points = coords(node, ids, &mut HashSet::new(), 0)?;
        return if name == "Point" {
            Ok(points
                .first()
                .map(|p| json!({"type":"Point","coordinates":p})))
        } else {
            Ok((points.len() > 1).then(|| json!({"type":"LineString","coordinates":points})))
        };
    }
    let mut gs = vec![];
    for c in node.children().filter(|c| c.is_element()) {
        if let Some(g) = geometry(c, ids, seen, depth + 1)? {
            gs.push(g);
        }
    }
    Ok(match gs.len() {
        0 => None,
        1 => gs.pop(),
        _ => Some(json!({"type":"GeometryCollection","geometries":gs})),
    })
}
pub fn parse(text: &str) -> Result<Data, String> {
    let doc = Document::parse(text).map_err(|e| format!("IMRO GML: {e}"))?;
    let mut ids = HashMap::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if let Some(id) = n
            .attributes()
            .find(|a| a.name() == "id")
            .map(|a| a.value().to_string())
        {
            if ids.insert(id.clone(), n).is_some() {
                return Err(format!("dubbel GML-ID {id}"));
            }
        }
    }
    let mut crs = None;
    for s in doc.descendants().filter_map(|n| n.attribute("srsName")) {
        let code = s.rsplit([':', '/']).next().unwrap_or("");
        if !code.chars().all(|c| c.is_ascii_digit()) || code.is_empty() {
            return Err("GML: onbekend coördinatenstelsel".into());
        }
        let code = format!("EPSG:{code}");
        if crs.as_ref().is_some_and(|c| c != &code) {
            return Err("GML bevat verschillende coördinatenstelsels".into());
        }
        crs = Some(code);
    }
    let mut objects = vec![];
    let mut warnings = vec![];
    for feature in doc.descendants().filter(|n| {
        n.is_element()
            && n.tag_name()
                .namespace()
                .is_some_and(|s| s.contains("/imro/"))
            && n.parent().is_some_and(|p| {
                ["featureMember", "featureMembers", "member"].contains(&p.tag_name().name())
            })
    }) {
        let mut props = Map::new();
        for n in feature.descendants().filter(|n| {
            n.is_element()
                && n.children().all(|c| !c.is_element())
                && n.tag_name()
                    .namespace()
                    .is_some_and(|s| s.contains("/imro/"))
        }) {
            if let Some(t) = n.text().map(str::trim).filter(|s| !s.is_empty()) {
                props.insert(n.tag_name().name().into(), t.into());
            }
        }
        props.insert("imro_attributen".into(), compound_attributes(feature)?);
        let mut gs = vec![];
        for prop in feature.children().filter(|n| {
            n.is_element()
                && ["geometrie", "geometry", "positie", "begrenzing"].contains(&n.tag_name().name())
        }) {
            if let Some(g) = geometry(prop, &ids, &mut HashSet::new(), 0)? {
                gs.push(g)
            }
        }
        let id = feature
            .attributes()
            .find(|a| a.name() == "id")
            .map(|a| a.value().to_string())
            .or_else(|| {
                props
                    .get("lokaalID")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if gs.is_empty() {
            warnings.push(format!("{id}: geen tekenbare geometrie"));
            continue;
        }
        let geometry = if gs.len() == 1 {
            gs.remove(0)
        } else {
            json!({"type":"GeometryCollection","geometries":gs})
        };
        super::polygons(&geometry)?;
        let get = |k: &str| {
            props
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        objects.push(Object {
            id,
            kind: feature.tag_name().name().into(),
            group: get("bestemmingshoofdgroep"),
            name: get("naam"),
            geometry,
            properties: props,
        });
    }
    if objects.is_empty() {
        return Err("Geen IMRO-planobjecten met geometrie gevonden".into());
    }
    let plans: Vec<_> = objects
        .iter()
        .filter(|o| o.kind.to_lowercase().ends_with("plangebied"))
        .collect();
    if plans.len() == 1 {
        let plan = plans[0];
        let id = plan
            .properties
            .get("lokaalID")
            .and_then(Value::as_str)
            .unwrap_or(&plan.id)
            .to_string();
        let status = plan.properties.get("planstatus").cloned();
        let date = plan.properties.get("datum").cloned();
        for object in &mut objects {
            object
                .properties
                .insert("plan_id".into(), id.clone().into());
            if let Some(status) = &status {
                object
                    .properties
                    .insert("planstatus".into(), status.clone());
            }
            if let Some(date) = &date {
                object
                    .properties
                    .insert("planstatusdatum".into(), date.clone());
            }
        }
    }
    Ok(Data {
        objects,
        crs: crs.ok_or("GML mist srsName; bepaal het bronstelsel vóór import")?,
        warnings,
    })
}
