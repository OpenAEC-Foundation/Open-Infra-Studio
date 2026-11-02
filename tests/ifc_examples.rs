//! Vaste tekeningen door dezelfde IFC-schrijver als GIS_IFC en CIV_IFC.
use std::path::Path;

use ocs_plugin_api::host::codec::{CadDocument, DwgReader, EntityType, LwPolyline, Vector2};
use open_infra::{
    civiel::as_profiel::Alignment,
    gis::site::{unit_factor_for_insunits, Site},
    ifc, nlcs,
};

#[path = "support/ifc_drawing.rs"]
mod ifc_drawing;

fn add(
    doc: &mut CadDocument,
    mut entity: EntityType,
    group: &str,
    object: &str,
    data: &[(&str, &str)],
) {
    let object = nlcs::object_by_name(group, object).expect("NLCS-object");
    let common = entity.common_mut();
    common.layer = nlcs::layer(object, nlcs::Status::N, "XX", nlcs::Element::G).name;
    if !data.is_empty() {
        common
            .extended_data
            .add_record(open_infra::bridge::object_data(
                data.iter().map(|(k, v)| (*k, v.to_string())),
            ));
    }
    doc.add_entity(entity).unwrap();
}

fn line(points: &[[f64; 2]], closed: bool) -> EntityType {
    let mut line = LwPolyline::new();
    for p in points {
        line.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
    }
    line.is_closed = closed;
    EntityType::LwPolyline(line)
}

fn write(doc: &CadDocument, name: &str, f: f64, site: Option<&Site>, options: &ifc::Options) {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ifc-test");
    std::fs::create_dir_all(&out).unwrap();
    let source = ifc::Source {
        doc,
        f,
        site,
        name: format!("{name}.dwg"),
    };
    let report = ifc::write(&source, options, &out.join(format!("{name}.ifc"))).unwrap();
    println!(
        "{name}: {:?}; overgeslagen {:?}",
        report.classes, report.skipped
    );
}

#[test]
fn export_fixed_models() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let options = ifc::Options::default();
    write(&CadDocument::new(), "leeg", 1., None, &options);
    for (name, path) in [
        ("catalogus", "examples/infra-bibliotheek/catalogus.dwg"),
        (
            "aansluitvoorbeeld",
            "examples/infra-bibliotheek/aansluitvoorbeeld.dwg",
        ),
        (
            "dordrecht-nlcs",
            "examples/dordrecht-nlcs/dordrecht-grote-kerk-nlcs.dwg",
        ),
    ] {
        let doc = DwgReader::from_file(root.join(path))
            .unwrap()
            .read()
            .unwrap();
        assert!(doc.model_space_entities().count() > 0, "{path}");
        let site = ifc_drawing::site_of(&doc);
        let factor = unit_factor_for_insunits(doc.header.insertion_units as i64);
        write(&doc, name, factor, site.as_ref(), &options);
    }
    let mut axes = CadDocument::new();
    for (name, points) in [
        ("Links", [[0., 0.], [100., 0.], [180., 80.]]),
        ("Rechts", [[0., 200.], [100., 200.], [180., 120.]]),
    ] {
        let axis = Alignment::from_pis(&points, &[60.]).unwrap();
        add(
            &mut axes,
            EntityType::LwPolyline(axis.to_polyline()),
            "AM",
            "AS",
            &[("as", name), ("beginstation", "125.000")],
        );
    }
    write(&axes, "assen", 1., None, &options);
    let mut standalone = CadDocument::new();
    add(
        &mut standalone,
        line(&[[0., 0.], [5., 0.], [5., 1.], [0., 1.]], true),
        "VW",
        "MARKERING",
        &[],
    );
    add(
        &mut standalone,
        line(&[[0., 10.], [5., 10.], [5., 15.], [0., 15.]], true),
        "GW",
        "ONTGRAVINGSVAK-WERK",
        &[],
    );
    write(&standalone, "losse-objecten", 1., None, &options);
    let mut mm = CadDocument::new();
    add(
        &mut mm,
        line(&[[0., 0.], [10000., 0.]], false),
        "KL",
        "GAS_LD",
        &[("naam", "Leiding 'één' \\ noord")],
    );
    write(&mm, "millimeters", 1000., None, &options);
    write(
        &mm,
        "lege-selectie",
        1000.,
        None,
        &ifc::Options {
            only: Some(Default::default()),
            no_texts: true,
        },
    );
}
