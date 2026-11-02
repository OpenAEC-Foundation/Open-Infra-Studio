use ocs_plugin_api::host::{
    codec::{entities::BoundaryEdge, EntityType},
    XDataValue,
};
use open_infra::{
    gis::{
        planning::{Data, Object},
        site::Site,
    },
    plannen,
};
use serde_json::json;

fn data(kind: &str, group: &str) -> Data {
    Data {
        crs: "EPSG:28992".into(),
        warnings: vec![],
        objects: vec![Object {
            id: "zone-1".into(),
            kind: kind.into(),
            group: group.into(),
            name: "Wonen".into(),
            geometry: json!({"type":"Polygon","coordinates":[
            [[100000.,400000.],[100020.,400000.],[100020.,400020.],[100000.,400020.],[100000.,400000.]],
            [[100008.,400008.],[100012.,400008.],[100012.,400012.],[100008.,400012.],[100008.,400008.]]]}),
            properties: serde_json::from_value(
                json!({"plan_id":"NL.IMRO.test", "regels":{"tekst":"ë".repeat(350)}}),
            )
            .unwrap(),
        }],
    }
}

#[test]
fn single_designation_is_one_native_solid_hatch_with_hole_and_local_millimetres() {
    let site = Site {
        absolute: false,
        origin_e: 100000.,
        origin_n: 400000.,
        ..Default::default()
    };
    let rendered = plannen::render(&site, &data("Enkelbestemming", "wonen"), 1000., 1.).unwrap();
    assert_eq!(rendered.objects, 1);
    let hatches: Vec<_> = rendered
        .entities
        .iter()
        .filter_map(|e| {
            if let EntityType::Hatch(h) = e {
                Some(h)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(hatches.len(), 1);
    let hatch = hatches[0];
    assert!(hatch.is_solid);
    assert_eq!(hatch.paths.len(), 2);
    assert!(hatch.paths[0].flags.is_external());
    assert!(!hatch.paths[1].flags.is_external());
    if let BoundaryEdge::Polyline(p) = &hatch.paths[0].edges[0] {
        assert!((p.vertices[1].x - 20000.).abs() < 0.01);
    } else {
        panic!("native closed polyline boundary required")
    }
    for e in &rendered.entities {
        let r = e
            .common()
            .extended_data
            .get_record(open_infra::bridge::XDATA_APP)
            .unwrap();
        assert!(r
            .values
            .iter()
            .any(|v| matches!(v,XDataValue::String(s) if s=="imro_id=zone-1")));
    }
    let labels: Vec<_> = rendered
        .entities
        .iter()
        .filter_map(|e| {
            if let EntityType::Text(t) = e {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(labels.len(), 1);
    let p = &labels[0].insertion_point;
    assert!(
        !(p.x > 8000. && p.x < 12000. && p.y > 8000. && p.y < 12000.),
        "label must avoid the polygon hole"
    );
}

#[test]
fn double_designation_keeps_pattern_holes_and_full_nested_metadata() {
    let mut source = data("Dubbelbestemming", "waarde - archeologie");
    let text = format!("{}{}{}", "ë".repeat(110), " ".repeat(450), " einde");
    source.objects[0]
        .properties
        .insert("regels".into(), json!({"tekst":text}));
    let rendered = plannen::render(&Site::default(), &source, 1., 1.).unwrap();
    let hatch = rendered
        .entities
        .iter()
        .find_map(|e| {
            if let EntityType::Hatch(h) = e {
                Some(h)
            } else {
                None
            }
        })
        .expect("pattern hatch");
    assert!(!hatch.is_solid);
    assert!(!hatch.pattern.lines.is_empty());
    assert_eq!(hatch.paths.len(), 2);
    let record = hatch
        .common
        .extended_data
        .get_record(open_infra::bridge::XDATA_APP)
        .unwrap();
    let mut chunks: Vec<_> = record
        .values
        .iter()
        .filter_map(|v| {
            if let XDataValue::String(s) = v {
                s.split_once('=')
                    .filter(|(k, _)| k.starts_with("imro_json_"))
            } else {
                None
            }
        })
        .collect();
    chunks.sort_by_key(|(k, _)| *k);
    let encoded: String = chunks.iter().map(|(_, v)| *v).collect();
    let saved: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(saved["properties"]["regels"]["tekst"], text);
}

#[test]
fn plans_have_separate_layers_and_measurement_labels_keep_value_type_pairs() {
    let mut a = data("Maatvoering", "");
    a.objects[0].properties.insert(
        "imro_attributen".into(),
        json!({"maatvoeringInfo":[
            {"WaardeEnType":{"waarde":"10","waardeType":"maximum bouwhoogte (m)"}},
            {"WaardeEnType":{"waarde":"6","waardeType":"maximum goothoogte (m)"}}
        ]}),
    );
    let mut b = a.clone();
    b.objects[0]
        .properties
        .insert("plan_id".into(), "NL.IMRO.second".into());
    let ra = plannen::render(&Site::default(), &a, 1., 1.).unwrap();
    let rb = plannen::render(&Site::default(), &b, 1., 1.).unwrap();
    assert!(
        ra.layers.keys().all(|l| !rb.layers.contains_key(l)),
        "different plans need independently manageable layers"
    );
    let text = ra
        .entities
        .iter()
        .find_map(|e| {
            if let EntityType::Text(t) = e {
                Some(t.value.as_str())
            } else {
                None
            }
        })
        .unwrap();
    assert!(text.contains("10") && text.contains("maximum bouwhoogte (m)"));
    assert!(text.contains("6") && text.contains("maximum goothoogte (m)"));
}

#[test]
fn coincident_features_get_separate_label_positions_outside_holes() {
    let mut d = data("Enkelbestemming", "wonen");
    d.objects.push(d.objects[0].clone());
    let rendered = plannen::render(
        &Site {
            absolute: false,
            origin_e: 100000.,
            origin_n: 400000.,
            ..Site::default()
        },
        &d,
        1.,
        0.35,
    )
    .unwrap();
    let points: Vec<_> = rendered
        .entities
        .iter()
        .filter_map(|e| {
            if let EntityType::Text(t) = e {
                Some(t.insertion_point)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(points.len(), 2);
    assert!((points[0].y - points[1].y).abs() > 0.6);
    assert!(points
        .iter()
        .all(|p| !(p.x > 8. && p.x < 12. && p.y > 8. && p.y < 12.)));
}
