//! Deeply nested XML must give an answer, not a stack overflow. Plugin
//! commands run on threads with the default 2 MiB stack, and a stack
//! overflow aborts the whole plugin: `catch_unwind` does not catch it.

use open_infra::civiel::landxml;
use open_infra::gis::{bro, caps, files, klic_extra, planning, qgis};

const DEPTH: usize = 50_000;

/// `depth` nested elements inside a `root` element.
fn deep(root: &str, depth: usize) -> String {
    format!("<{root}>{}{}</{root}>", "<a>".repeat(depth), "</a>".repeat(depth))
}

/// Runs `read` on a thread with the stack size of a plugin command.
fn on_command_stack<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new().stack_size(2 << 20).spawn(read).unwrap().join().unwrap()
}

#[test]
fn xml_readers_survive_deep_nesting() {
    type Reader = fn(&str) -> Result<(), String>;
    let readers: [(&str, &str, Reader); 11] = [
        ("LandXML", "LandXML", |t| landxml::parse(t).map(drop)),
        ("KML", "kml", |t| files::kml(t).map(drop)),
        ("GPX", "gpx", |t| files::gpx(t).map(drop)),
        ("GML", "FeatureCollection", |t| files::gml(t).map(drop)),
        ("IMRO GML", "FeatureCollection", |t| planning::gml::parse(t).map(drop)),
        ("BRO search", "dispatchCharacteristicsResponse", |t| bro::parse_characteristics(t).map(drop)),
        ("BRO CPT", "dispatchDataResponse", |t| bro::parse_cpt(t).map(drop)),
        ("BRO borehole", "dispatchDataResponse", |t| bro::parse_bhr(t).map(drop)),
        ("WMS capabilities", "WMS_Capabilities", |t| caps::parse_wms("https://example.org/wms", t).map(drop)),
        ("QGIS connections", "qgsWMSConnections", |t| qgis::parse(t).map(drop)),
        ("KLIC", "FeatureCollection", |t| klic_extra::parse(t).map(drop)),
    ];
    for (name, root, read) in readers {
        let text = deep(root, DEPTH);
        // Ok or an error message are both fine; the test fails by aborting.
        let result = on_command_stack(move || read(&text));
        eprintln!("{name}: {result:?}");
    }
    let imro = on_command_stack(|| planning::gml::is_imro(&deep("FeatureCollection", DEPTH)));
    assert!(!imro);
}
