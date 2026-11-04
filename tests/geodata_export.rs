//! A drawing with a GEODATA object as the codec writes it (DWG and DXF), for
//! tests/test_geodata_ezdxf.py to read with ezdxf: an independent reader of
//! AutoCAD's GEODATA.
use ocs_plugin_api::host::codec::{CadDocument, DxfWriter, DwgWriter};
use ocs_plugin_api::host::codec::entities::Line;
use ocs_plugin_api::host::codec::{EntityType, Vector3};
#[test]
fn drawing_with_geodata_for_ezdxf() {
    let mut site = open_infra::gis::site::Site::default();
    site.crs = "EPSG:28992".into();
    site.set_origin(104885.0, 425330.0);
    site.absolute = false;
    let mut doc = CadDocument::new();
    let _ = doc.add_entity(EntityType::Line(Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 10.0, 0.0))));
    open_infra::gis::geodata::set_geodata(&mut doc, &site, 1.0).unwrap();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/geodata");
    std::fs::create_dir_all(&out).unwrap();
    DxfWriter::new(&doc).write_to_file(out.join("test.dxf")).unwrap();
    DwgWriter::write_to_file(out.join("test.dwg"), &doc).unwrap();
}
