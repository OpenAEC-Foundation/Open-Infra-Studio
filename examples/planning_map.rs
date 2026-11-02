//! Reproducible local IMRO drawing, including a polygon interior ring.
use ocs_plugin_api::host::codec::{
    tables::{AppId, Layer, LineType},
    CadDocument, Color, DwgReader, DwgWriter, DxfReader, DxfWriter,
};
use open_infra::{
    gis::{planning, site::Site},
    plannen,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/planning-example".into());
    let path = std::path::Path::new(&out);
    std::fs::create_dir_all(path)?;
    let data = planning::gml::parse(include_str!(
        "../tests/fixtures/planning/synthetic-imro.gml"
    ))?;
    let site = Site {
        origin_e: 100000.,
        origin_n: 400000.,
        absolute: false,
        ..Site::default()
    };
    let rendered = plannen::render(&site, &data, 1., 0.35)?;
    let mut doc = CadDocument::new();
    doc.header.insertion_units = 6;
    doc.app_ids.add(AppId::new("OPENINFRA"))?;
    doc.line_types.add(LineType::new("IMRO_PLANCONTOUR"))?;
    for (name, color) in rendered.layers {
        let mut layer = Layer::new(name);
        layer.color = Color::Rgb {
            r: color[0],
            g: color[1],
            b: color[2],
        };
        doc.layers.add(layer)?;
    }
    for entity in rendered.entities {
        doc.add_entity(entity)?;
    }
    let count = doc.entities().count();
    let dxf = path.join("synthetic-plan.dxf");
    DxfWriter::new(&doc).write_to_file(&dxf)?;
    assert_eq!(
        DxfReader::from_file(&dxf)?.read()?.entities().count(),
        count
    );
    let dwg = path.join("synthetic-plan.dwg");
    DwgWriter::write_to_file(&dwg, &doc)?;
    assert_eq!(
        DwgReader::from_file(&dwg)?.read()?.entities().count(),
        count
    );
    println!(
        "{count} entities; DXF/DWG roundtrips verified in {}",
        path.display()
    );
    Ok(())
}
