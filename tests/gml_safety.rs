//! Bestaande XML-foutdetails en een begrensde procesproef voor diepe kapotte GML.

use open_infra::gis::files;

#[test]
fn gml_original_xml_diagnostic_matches_sol_reproduction() {
    let malformed = include_str!("fixtures/gml/malformed.gml");
    let expected = "GML: expected 'gml:member' tag, not 'gml:FeatureCollection' at 1:75";
    assert_eq!(files::gml(malformed).unwrap_err(), expected);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gml/malformed.gml");
    assert_eq!(files::read(&path, None).unwrap_err(), expected);

    // Preserve the original parser's diagnostics for other broken XML too.
    for text in ["<root><member></root>", "<root name=\"unfinished", "<root></root></extra>"] {
        let expected = format!("GML: {}", roxmltree::Document::parse(text).unwrap_err());
        assert_eq!(files::gml(text).unwrap_err(), expected, "{text}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn gml_deep_malformed_document_stops_in_a_limited_process() {
    // Python is already part of the offline CI. Only the isolated child gets
    // these limits; the Rust test runner and the build are unaffected.
    let script = r#"
import os
import resource
import subprocess
import sys

def limits():
    settings = [(resource.RLIMIT_AS, 1024 * 1024 * 1024),
                (resource.RLIMIT_CPU, 5), (resource.RLIMIT_CORE, 0)]
    for key, value in settings:
        resource.setrlimit(key, (value, value))
        assert resource.getrlimit(key) == (value, value)

result = subprocess.run(
    [sys.argv[1], '--exact', 'gml_deep_malformed_child', '--ignored', '--nocapture'],
    env=dict(os.environ, OPEN_INFRA_GML_LIMITED_CHILD='1'),
    preexec_fn=limits, timeout=10, check=False)
print('Begrensd GML-proces: exitcode', result.returncode)
sys.exit(0 if result.returncode == 0 else 1)
"#;
    let output = std::process::Command::new("python3")
        .arg("-c").arg(script).arg(std::env::current_exe().unwrap())
        .output().expect("Python voor de bestaande offline CI moet beschikbaar zijn");
    assert!(output.status.success(), "procesproef: {}\n{}\n{}", output.status,
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "alleen via de procesproef met tijds- en geheugenlimiet"]
fn gml_deep_malformed_child() {
    assert_eq!(std::env::var("OPEN_INFRA_GML_LIMITED_CHILD").as_deref(), Ok("1"));
    let prefix = "<gml:FeatureCollection xmlns:gml=\"http://www.opengis.net/gml\"><gml:member><Feature><geometry>";
    let open = "<gml:MultiGeometry><gml:geometryMember>".repeat(256);
    let close = "</gml:geometryMember></gml:MultiGeometry>".repeat(256);
    let suffix = "</geometry></Feature></gml:member></gml:FeatureCollection>";
    let mismatched = format!("{prefix}{open}<gml:Point></gml:LineString>{close}{suffix}");
    let unknown_entity = format!("{prefix}{open}<gml:Point><gml:pos>&unknown;</gml:pos></gml:Point>{close}{suffix}");
    for text in [mismatched, unknown_entity] {
        // Exercise the real reader inside the child. A panic, signal, hang or
        // memory-limit failure makes the parent fail, independently of Err.
        let error = files::gml(&text).unwrap_err();
        assert!(error.starts_with("GML: "), "oorspronkelijke XML-fout ontbreekt: {error}");
    }
}
