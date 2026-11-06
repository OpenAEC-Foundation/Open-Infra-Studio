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

/// A drawing in memory for the host requests of an import: the entities it
/// adds and the XRecords it keeps.
#[derive(Default)]
struct Drawing {
    entities: std::sync::Mutex<Vec<ocs_plugin_api::host::codec::EntityType>>,
    records: std::sync::Mutex<HashMap<String, Vec<ocs_plugin_api::host::XRecordEntry>>>,
}

impl ocs_plugin_api::host::PluginRequestSender for Drawing {
    fn request(&self, request: ocs_plugin_api::ipc::protocol::PluginRequest) -> Result<ocs_plugin_api::ipc::protocol::PluginResponse, ocs_plugin_api::host::PluginRequestError> {
        use ocs_plugin_api::host::codec::Handle;
        use ocs_plugin_api::ipc::protocol::{PluginRequest as Q, PluginResponse as A};
        Ok(match request {
            Q::AddLayer(_) => A::OptHandle(Some(Handle::from(1000u64))),
            Q::TableOperation { .. } => A::TableResult(Ok(Handle::from(1001u64))),
            Q::AddEntities(entities) => {
                let mut stored = self.entities.lock().unwrap();
                let handles = (stored.len()..stored.len() + entities.len()).map(|i| Handle::from(i as u64 + 1)).collect();
                stored.extend(entities);
                A::Handles(handles)
            }
            Q::ReadXRecord { name, .. } => A::XRecordEntries(Ok(self.records.lock().unwrap().get(&name).cloned().unwrap_or_default())),
            Q::UpdateXRecord { name, entries, .. } => {
                let mut r = self.records.lock().unwrap();
                match r.get_mut(&name) {
                    Some(slot) => {
                        *slot = entries;
                        A::XRecordUpdateResult(Ok(Handle::from(1002u64)))
                    }
                    None => A::XRecordUpdateResult(Err("no such record".into())),
                }
            }
            Q::CreateXRecord { name, entries, .. } => {
                self.records.lock().unwrap().insert(name, entries);
                A::XRecordCreateResult(Ok(Handle::from(1002u64)))
            }
            Q::PushUndo { .. } | Q::WriteRecord { .. } | Q::SetDirty | Q::BumpGeometry => A::Ok,
            other => return Err(ocs_plugin_api::host::PluginRequestError(format!("unexpected host request: {other:?}"))),
        })
    }
}

/// A V12.1 delivery with a cable and two attachments linked to it, one with
/// a location and one without (the schema allows a relation object without
/// geometry), and a sleeve with its content and a note on that content
/// (both without geometry). Every delivery uses the same IDs; `tag` and `dx`
/// tell them apart.
fn linked(tag: &str, dx: f64) -> String {
    let x = |v: f64| v + dx;
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<NLCSnetbeheer xmlns="NS_NLCSnetbeheer" xmlns:gml="http://www.opengis.net/gml/3.2" gml:id="NLCSnetbeheer">
  <AprojectReferentie gml:id="f_PROJECT"><ID>PROJECT</ID><Projectnummer>P-61</Projectnummer><Netbeheerder>Enexis</Netbeheerder><NaamAannemer>Test</NaamAannemer><Tekeningtype>EINDREVISIE</Tekeningtype>
    <Geometry><gml:Polygon gml:id="g_PROJECT" srsName="EPSG:28992"><gml:exterior><gml:LinearRing><gml:posList>105000 425000 105300 425000 105300 425100 105000 425000</gml:posList></gml:LinearRing></gml:exterior></gml:Polygon></Geometry></AprojectReferentie>
  <LSkabel gml:id="f_OLD_ASSET"><Handle>1A</Handle><ID>OLD_ASSET</ID><Toelichting>kabel {tag}</Toelichting><Status>NIEUW</Status><Bedrijfstoestand>IN BEDRIJF</Bedrijfstoestand>
    <Geometry><gml:LineString gml:id="g_OLD_ASSET" srsName="EPSG:28992" srsDimension="3"><gml:posList>{} 425010 -0.6 {} 425040 -0.7</gml:posList></gml:LineString></Geometry></LSkabel>
  <AbestandBijlage gml:id="f_OLD_PHOTO"><Handle>1B</Handle><ID>OLD_PHOTO</ID><Bestandsnaam>foto-{tag}.jpg</Bestandsnaam><Status>NIEUW</Status><AssetObjectID>OLD_ASSET</AssetObjectID>
    <Geometry><gml:Point gml:id="g_OLD_PHOTO" srsName="EPSG:28992"><gml:pos>{} 425020</gml:pos></gml:Point></Geometry></AbestandBijlage>
  <AbestandBijlage gml:id="f_OLD_DOC"><Handle>1C</Handle><ID>OLD_DOC</ID><Bestandsnaam>meetrapport-{tag}.pdf</Bestandsnaam><Status>NIEUW</Status><AssetObjectID>OLD_ASSET</AssetObjectID></AbestandBijlage>
  <Amantelbuis gml:id="f_OLD_SLEEVE"><Handle>1D</Handle><ID>OLD_SLEEVE</ID><Status>NIEUW</Status><Bedrijfstoestand>IN BEDRIJF</Bedrijfstoestand>
    <Geometry><gml:LineString gml:id="g_OLD_SLEEVE" srsName="EPSG:28992" srsDimension="3"><gml:posList>{} 425060 -1.0 {} 425070 -1.0</gml:posList></gml:LineString></Geometry></Amantelbuis>
  <AmantelbuisInhoud gml:id="f_OLD_CONTENT"><ID>OLD_CONTENT</ID><Status>NIEUW</Status><MantelbuisID>OLD_SLEEVE</MantelbuisID></AmantelbuisInhoud>
  <AbestandBijlage gml:id="f_OLD_NOTE"><ID>OLD_NOTE</ID><Bestandsnaam>notitie-{tag}.txt</Bestandsnaam><Status>NIEUW</Status><AssetObjectID>OLD_CONTENT</AssetObjectID></AbestandBijlage>
</NLCSnetbeheer>"#,
        x(105010.0),
        x(105050.0),
        x(105020.0),
        x(105010.0),
        x(105040.0)
    )
}

/// Two deliveries with the same IDs imported into one drawing, the sleeves
/// deleted, and the drawing exported: every relation object comes back
/// linked to the object of its own delivery, every reference (IDREF) names
/// an ID of the new delivery, and the relations of the deleted sleeves are
/// left out.
#[test]
fn import_then_export_keeps_relations_and_their_references() {
    use open_infra::{bridge, gis::site::Site, nlcspp_host};
    let dir = std::env::temp_dir().join(format!("oi-nlcspp-refs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let drawing = std::sync::Arc::new(Drawing::default());
    let target = || bridge::Target { sender: drawing.clone(), tab_id: 61002, unit_factor: 1.0, doc_path: None };
    let site = Site::default();
    for (tag, dx) in [("a", 0.0), ("b", 100.0)] {
        let file = dir.join(format!("gekoppeld-{tag}.xml"));
        std::fs::write(&file, linked(tag, dx)).unwrap();
        bridge::with_command_context(target(), None, None, || nlcspp_host::import(&site, &file, 1.0)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut doc = ocs_plugin_api::host::codec::CadDocument::new();
    for e in drawing.entities.lock().unwrap().iter() {
        // The user deleted the sleeves.
        if open_infra::library::infra::host::data(e).get("nlcspp_object").map(String::as_str) != Some("Amantelbuis") {
            doc.add_entity(e.clone()).unwrap();
        }
    }
    let project = nlcspp_host::Project { nummer: "P-61".into(), aannemer: "Test".into(), tekeningtype: "EINDREVISIE".into(), ..Default::default() };
    let (x, findings, _) = bridge::with_command_context(target(), None, None, || nlcspp_host::delivery(&doc, &site, 1.0, "Enexis", &project)).unwrap();
    // For tests/test_nlcspp_xsd.py, which validates it against the Enexis XSD.
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/nlcspp");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("relaties-v12.1.xml"), &x).unwrap();

    let back = nlcspp::xml::read(&x).unwrap();
    let ids: Vec<&str> = back.objects.iter().chain(back.project.iter()).filter_map(|o| o.get("ID")).collect();
    let m = nlcspp::model::model();
    for o in &back.objects {
        for f in m.object(&o.kind).unwrap().velden.iter().filter(|f| f.kind == "IDREF") {
            if let Some(v) = o.get(&f.name) {
                assert!(ids.contains(&v), "{} {}: {} = {v} names no ID of the delivery ({ids:?})\n{x}", o.kind, o.get("ID").unwrap_or(""), f.name);
            }
        }
    }
    let named = |name: &str| back.objects.iter().find(|o| o.get("Bestandsnaam") == Some(name));
    for tag in ["a", "b"] {
        let cable = back.objects.iter().find(|o| o.kind == "LSkabel" && o.get("Toelichting") == Some(format!("kabel {tag}").as_str())).unwrap();
        for name in [format!("foto-{tag}.jpg"), format!("meetrapport-{tag}.pdf")] {
            let a = named(name.as_str()).unwrap_or_else(|| panic!("{name} delivered\n{x}"));
            assert_eq!(a.get("AssetObjectID"), cable.get("ID"), "{name} on the cable of its own delivery\n{x}");
        }
        let without = named(format!("meetrapport-{tag}.pdf").as_str()).unwrap();
        assert_eq!(without.geom, None, "the attachment without a location stays without one");
        assert_eq!(without.get("Handle"), None, "no CAD handle: it is not drawn");
        assert!(named(format!("notitie-{tag}.txt").as_str()).is_none(), "its content's sleeve is gone\n{x}");
    }
    assert_eq!(back.objects.iter().filter(|o| o.kind == "AbestandBijlage").count(), 4, "{x}");
    assert!(!back.objects.iter().any(|o| o.kind == "AmantelbuisInhoud" || o.kind == "Amantelbuis"), "{x}");
    let left_out = findings.iter().filter(|f| f.level == nlcspp::Level::Waarschuwing && f.field.is_empty() && f.message.contains("niet geleverd")).count();
    assert_eq!(left_out, 4, "a warning per relation left out: {findings:#?}");
}
