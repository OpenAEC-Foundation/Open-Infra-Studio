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
fn an_s_curve_within_a_full_turn_still_draws() {
    // Curvature 1/80 left to 1/40 right: the heading goes 0 → +0.625 →
    // −1.875 rad, 3.125 rad of turning in all. Only the larger curvature
    // over the full length (7.5 rad) is more than a full circle.
    let text = SPIRAL.replace(
        r#"<Spiral length="30" radiusStart="INF" radiusEnd="200" rot="cw"><Start>50 150</Start><PI>60 150</PI><End>79.98313 150.74970</End></Spiral>"#,
        r#"<Spiral length="300" radiusStart="80" radiusEnd="-40" rot="ccw"><Start>0 0</Start><PI>0 232.687194806618</PI><End>16.477943323296 237.860415321541</End></Spiral>"#,
    );
    assert_ne!(text, SPIRAL);
    let x = landxml::parse(&text).unwrap();
    assert!(x.alignments.len() == 1 && x.warnings.is_empty(), "{:?}", x.warnings);
    let a = &x.alignments[0];
    let line = Alignment { name: a.name.clone(), start: a.start, segs: a.segs.clone() }.to_polyline_data(0.001).0;
    assert!((2..1000).contains(&line.vertices.len()), "{} vertices", line.vertices.len());
    // Ends where the End point says (east, north).
    let end = line.vertices[line.vertices.len() - 1].location;
    assert!((end.x - 237.860415321541).abs() < 1e-3 && (end.y - 16.477943323296).abs() < 1e-3, "{} {}", end.x, end.y);
}

#[test]
fn a_long_spiral_gives_an_answer_within_10_s() {
    let text = SPIRAL.replace("length=\"30\"", "length=\"1e9\"").replace("radiusEnd=\"200\"", "radiusEnd=\"1\"");
    assert!(draw_within_10_s(text).is_some(), "no answer within 10 s");
}

#[test]
fn a_long_spiral_far_from_the_origin_gives_an_answer_within_10_s() {
    // Within a full turn, but so far out that rounding keeps every arc
    // over the 1 mm tolerance until each piece is halved 24 times.
    let text = SPIRAL
        .replace("length=\"30\"", "length=\"1e7\"")
        .replace("radiusEnd=\"200\"", "radiusEnd=\"1e7\"")
        .replace("<Start>50 150</Start>", "<Start>1e15 1e15</Start>")
        .replace("<PI>60 150</PI>", "<PI>1e15 1.00000000000001e15</PI>");
    let n = draw_within_10_s(text).expect("no answer within 10 s");
    assert!(n <= 20_000, "{n} vertices");
}
