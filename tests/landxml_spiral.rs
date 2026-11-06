//! Transition curves (clothoids) from LandXML are drawn as arcs within a
//! 1 mm tolerance. A spiral that turns many times must give an answer
//! quickly instead of hanging the import.

use std::time::Duration;

use open_infra::civiel::as_profiel::Alignment;
use open_infra::civiel::landxml;

const SPIRAL: &str = r#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <CgPoints><CgPoint name="1">0 0</CgPoint></CgPoints>
  <Alignments><Alignment name="as"><CoordGeom>
    <Spiral length="30" radiusStart="INF" radiusEnd="200" rot="cw"><Start>50 150</Start><PI>60 150</PI><End>79.98313 150.74970</End></Spiral>
  </CoordGeom></Alignment></Alignments>
</LandXML>"#;

/// What `GIS_LANDXML` does after reading (drawing factor 1): every axis as
/// a polyline of arcs. The vertex count, or `None` after 10 s.
fn draw_within_10_s(text: String) -> Option<usize> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let x = landxml::parse(&text).unwrap();
        let n = x.alignments.iter().map(|a| {
            let al = Alignment { name: a.name.clone(), start: a.start, segs: a.segs.clone() };
            al.to_polyline_data(0.001).0.vertices.len()
        });
        let _ = send.send(n.sum());
    });
    receive.recv_timeout(Duration::from_secs(10)).ok()
}

#[test]
fn a_normal_spiral_still_draws() {
    let n = draw_within_10_s(SPIRAL.into()).expect("no answer within 10 s");
    assert!((2..100).contains(&n), "{n} vertices");
}

#[test]
fn a_spiral_turning_more_than_a_full_circle_is_skipped_with_a_warning() {
    for (from, to) in [
        ("length=\"30\"", "length=\"1e9\""),
        ("radiusEnd=\"200\"", "radiusEnd=\"0.002\""),
        ("radiusStart=\"INF\"", "radiusStart=\"0.001\""),
    ] {
        let text = SPIRAL.replace(from, to);
        assert_ne!(text, SPIRAL, "{from}");
        let x = landxml::parse(&text).unwrap();
        assert!(x.alignments.is_empty() && x.warnings.iter().any(|w| w.contains("Alignment as")), "{to}: {:?}", x.warnings);
    }
}

#[test]
fn a_long_spiral_gives_an_answer_within_10_s() {
    let text = SPIRAL.replace("length=\"30\"", "length=\"1e9\"").replace("radiusEnd=\"200\"", "radiusEnd=\"1\"");
    assert!(draw_within_10_s(text).is_some(), "no answer within 10 s");
}
