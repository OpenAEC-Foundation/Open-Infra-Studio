//! IFC 4.3 export of a real drawing, read straight from the DWG (no Open
//! CAD Studio needed): the project location from the drawing's OPENINFRA
//! dictionary, the units from its header.
//!
//!     cargo test --test ifc_dwg -- --ignored --nocapture
//!     OI_IFC_DWG=docs/video/bron/data/dordrecht-showcase.dwg cargo test --test ifc_dwg -- --ignored --nocapture
//!
//! Writes target/ifc-test/<drawing>.ifc; check it with
//! `python tests/ifc_validate.py target/ifc-test/<drawing>.ifc`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ocs_plugin_api::host::codec::objects::{ObjectType, XRecordValue};
use ocs_plugin_api::host::codec::{CadDocument, DwgReader};

use open_infra::gis::site::{unit_factor_for_insunits, Site};
use open_infra::ifc;

/// The site JSON Open Infra keeps in the drawing (NOD OPENINFRA/SITE).
fn site_of(doc: &CadDocument) -> Option<Site> {
    doc.objects.values().find_map(|o| match o {
        ObjectType::XRecord(x) if x.name.eq_ignore_ascii_case("SITE") || x.entries.iter().any(|e| matches!(&e.value, XRecordValue::String(s) if s.contains("crs_proj4"))) => {
            x.entries.iter().find_map(|e| match &e.value {
                XRecordValue::String(s) if s.trim_start().starts_with('{') => serde_json::from_str(s).ok(),
                _ => None,
            })
        }
        _ => None,
    })
}

#[test]
#[ignore = "reads a DWG from disk; run with --ignored"]
fn export_a_real_drawing() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dwg: PathBuf = std::env::var_os("OI_IFC_DWG").map(PathBuf::from).unwrap_or_else(|| root.join("examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.dwg"));
    let t = Instant::now();
    let doc = DwgReader::from_file(&dwg).expect("open").read().expect("read");
    let site = site_of(&doc);
    let f = unit_factor_for_insunits(doc.header.insertion_units as i64);
    println!("{}: {} entities, read in {:.1} s, {} drawing units per metre, site {:?}", dwg.display(), doc.model_space_entities().count(), t.elapsed().as_secs_f64(), f, site.as_ref().map(|s| (&s.label, &s.crs)));
    let dir = root.join("target/ifc-test");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(format!("{}.ifc", dwg.file_stem().unwrap().to_string_lossy()));
    let name = dwg.file_name().unwrap().to_string_lossy().into_owned();
    let t = Instant::now();
    let src = ifc::Source { doc: &doc, f, site: site.as_ref(), name };
    let r = ifc::write(&src, &ifc::Options::default(), &out).expect("export");
    println!("{} in {:.1} s, {:.1} MB", r.summary(), t.elapsed().as_secs_f64(), std::fs::metadata(&out).unwrap().len() as f64 / 1e6);
    assert!(r.objects() > 0);
}
