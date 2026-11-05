//! Golden files: the official NLCS++ example deliveries (tests/fixtures/nlcspp).

use std::collections::HashMap;

use ocs_plugin_api::host::codec::DwgReader;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/nlcspp/");

fn pair(name: &str) -> (nlcspp::Delivery, HashMap<String, String>) {
    let xml = std::fs::read_to_string(format!("{DIR}{name}.xml")).unwrap();
    let d = nlcspp::xml::read(&xml).unwrap();
    let doc = DwgReader::from_file(format!("{DIR}{name}.dwg")).unwrap().read().unwrap();
    let layers = doc.model_space_entities().map(|e| (format!("{:X}", e.common().handle.value()), e.common().layer.clone())).collect();
    (d, layers)
}

fn tag_count(xml: &str, kind: &str) -> usize {
    xml.matches(&format!("<{kind}>")).count()
}

#[test]
fn every_object_of_the_examples_is_read() {
    for (name, op) in [("alliander-demo", "Alliander"), ("enexis-voorbeeld", "Enexis")] {
        let xml = std::fs::read_to_string(format!("{DIR}{name}.xml")).unwrap();
        let d = nlcspp::xml::read(&xml).unwrap();
        assert_eq!(d.operator.as_deref(), Some(op));
        assert_eq!(d.version.as_deref(), Some("11"));
        let mut kinds: Vec<&str> = d.objects.iter().map(|o| o.kind.as_str()).collect();
        kinds.sort();
        kinds.dedup();
        for k in kinds {
            assert_eq!(d.objects.iter().filter(|o| o.kind == k).count(), tag_count(&xml, k), "{name}: {k}");
        }
        assert!(d.objects.iter().all(|o| o.geom.is_some()), "{name}: every object has geometry");
        assert_eq!(d.objects.len(), xml.matches("<Geometry>").count());
    }
}

#[test]
fn the_grammar_gives_the_layers_of_the_examples() {
    for (name, op) in [("alliander-demo", "Alliander"), ("enexis-voorbeeld", "Enexis")] {
        let (d, layers) = pair(name);
        let (mut same, mut differ, mut none) = (0, Vec::new(), 0);
        for o in &d.objects {
            let Some(actual) = o.get("Handle").and_then(|h| layers.get(&h.to_uppercase())) else { continue };
            match nlcspp::layer::for_object(o, Some(op)) {
                Some(l) if l.eq_ignore_ascii_case(actual) => same += 1,
                Some(l) => differ.push(format!("{} {}: {l} ≠ {actual}", o.kind, o.get("Handle").unwrap_or(""))),
                None => none += 1,
            }
        }
        let total = same + differ.len();
        println!("{name}: {same}/{total} layers as in the drawing, {none} objects without a layer rule");
        let mut kinds = std::collections::BTreeMap::new();
        for d in &differ {
            let (ours, theirs) = d.split_once(": ").unwrap().1.split_once(" ≠ ").unwrap();
            let (a, b) = (nlcspp::layer::parse(ours).unwrap(), nlcspp::layer::parse(theirs).unwrap());
            let diff: Vec<String> = (0..a.parts.len().max(b.parts.len())).filter(|i| a.parts.get(*i) != b.parts.get(*i)).map(|i| format!("{i}:{}→{}", a.parts.get(i).map(String::as_str).unwrap_or("-"), b.parts.get(i).map(String::as_str).unwrap_or("-"))).collect();
            *kinds.entry(format!("{} {}", d.split(' ').next().unwrap(), diff.join(" "))).or_insert(0) += 1;
        }
        for (k, n) in &kinds {
            println!("  {n:4} × {k}");
        }
        // The rest are objects whose layer in the example drawing disagrees
        // with their own attributes in the example XML (153 Enexis LS cables
        // say AANSLUITNET and 230 V but lie on a DISTRIBUTIE_400 V layer);
        // the grammar follows the attributes.
        let floor = if name == "enexis-voorbeeld" { 0.75 } else { 0.70 };
        assert!(none == 0, "{name}: {none} objects without a layer");
        assert!(same as f64 >= floor * total as f64, "{name}: {same}/{total}");
    }
}

/// The examples as Open Infra writes them (V12.1 with a project reference),
/// for tests/test_nlcspp_xsd.py to validate against the operators' XSDs.
#[test]
fn the_examples_written_as_v12_1() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/nlcspp");
    std::fs::create_dir_all(&out).unwrap();
    for (name, op) in [("alliander-demo", "Alliander"), ("enexis-voorbeeld", "Enexis")] {
        let mut d = nlcspp::xml::read(&std::fs::read_to_string(format!("{DIR}{name}.xml")).unwrap()).unwrap();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for o in &d.objects {
            let pts: Vec<[f64; 3]> = match &o.geom {
                Some(nlcspp::Geom::Point(p)) => vec![*p],
                Some(nlcspp::Geom::Line(v)) => v.clone(),
                Some(nlcspp::Geom::Polygon(r)) => r.concat(),
                None => vec![],
            };
            for p in pts {
                (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
            }
        }
        let mut p = nlcspp::Object { kind: "AprojectReferentie".into(), ..Default::default() };
        for (k, v) in [("Projectnummer", "OI-TEST-1"), ("Netbeheerder", nlcspp::model::netbeheerder_value(op)), ("NaamAannemer", "Open Infra Studio"), ("Tekeningtype", "EINDREVISIE")] {
            p.set(k, v);
        }
        p.geom = Some(nlcspp::Geom::Polygon(vec![vec![[x0, y0, 0.0], [x1, y0, 0.0], [x1, y1, 0.0], [x0, y1, 0.0]]]));
        d.project = Some(p);
        let x = nlcspp::xml::write(&d, op, "EPSG:28992");
        let back = nlcspp::xml::read(&x).unwrap();
        assert_eq!(back.objects.len(), d.objects.iter().filter(|o| nlcspp::model::model().object(&o.kind).is_some()).count());
        std::fs::write(out.join(format!("{name}-v12.1.xml")), x).unwrap();
        // Our check's code-list errors, for the XSD test to compare with.
        let findings = nlcspp::check::check(&d, Some(op));
        let list_errors: Vec<String> = findings.iter().filter(|f| f.level == nlcspp::Level::Fout && f.message.contains("keuzelijst")).map(|f| format!("{}/{}", f.object, f.field)).collect();
        std::fs::write(out.join(format!("{name}-keuzelijstfouten.json")), serde_json::to_string(&list_errors).unwrap()).unwrap();
    }
}
