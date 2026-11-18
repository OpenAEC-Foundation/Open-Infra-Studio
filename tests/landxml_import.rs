//! Volledige importuitvoer bij deels leesbare LandXML, zonder echte host.
use std::path::Path;
use std::sync::Arc;
use ocs_plugin_api::host::codec::EntityType;
use open_infra::{bridge, civiel::{landxml, CivielOptions}};

#[path = "support/import_drawing.rs"]
mod import_drawing;
use import_drawing::Drawing;

#[test]
fn partially_readable_landxml_keeps_the_complete_original_success_message() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/landxml/partial.xml");
    let parsed = landxml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!((parsed.points.len(), parsed.warnings.len()), (1, 1));
    assert_eq!((parsed.points[0].e, parsed.points[0].n, parsed.points[0].z), (100.0, 200.0, Some(5.0)));

    let drawing = Arc::new(Drawing::default());
    let target = bridge::Target { sender: drawing.clone(), tab_id: 73003, unit_factor: 1.0, doc_path: None };
    let options = CivielOptions { point_labels: false, ..Default::default() };
    let progress = std::cell::RefCell::new(Vec::new());
    let result = bridge::with_command_context(target, None, None, || {
        landxml::import(&path, &options, &|message| progress.borrow_mut().push(message))
    });

    assert!(drawing.unexpected.lock().unwrap().is_empty(), "{:?}", drawing.unexpected.lock().unwrap());
    assert_eq!(drawing.entities.lock().unwrap().len(), 3);
    assert_eq!(drawing.entities.lock().unwrap().iter().filter(|entity| matches!(entity, EntityType::Point(_))).count(), 1);
    assert_eq!(*drawing.undo.lock().unwrap(), vec!["LandXML partial.xml"]);
    assert_eq!(*progress.borrow(), vec!["LandXML lezen…"]);
    assert_eq!(result.unwrap(), "partial.xml: 1 punten — 3 elementen.");
}
