//! Grondlichamen via dezelfde IFC-export als GIS_IFC en CIV_IFC.
use std::path::Path;

use ocs_plugin_api::host::codec::{
    entities::{Mesh, MeshFace},
    CadDocument, EntityType, LwPolyline, Vector2, Vector3,
};
use open_infra::{gis::site::Site, ifc};

fn add(doc: &mut CadDocument, mut entity: EntityType, layer: &str, data: &[(&str, &str)]) {
    let common = entity.common_mut();
    common.layer = layer.into();
    if !data.is_empty() {
        common
            .extended_data
            .add_record(open_infra::bridge::object_data(
                data.iter().map(|(k, v)| (*k, v.to_string())),
            ));
    }
    doc.add_entity(entity).unwrap();
}

fn mesh(points: &[[f64; 3]], triangles: &[[usize; 3]]) -> EntityType {
    let mut mesh = Mesh::new();
    mesh.vertices = points
        .iter()
        .map(|p| Vector3::new(p[0], p[1], p[2]))
        .collect();
    mesh.faces = triangles
        .iter()
        .map(|t| MeshFace::new(t.to_vec()))
        .collect();
    EntityType::Mesh(mesh)
}

fn check_earthworks(name: &str, centre: f64, level: f64, expected: &[f64]) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ifc-test");
    std::fs::create_dir_all(&directory).unwrap();
    let mut errors = Vec::new();
    for (unit, f) in [("m", 1.0), ("mm", 1000.0)] {
        for (location, origin) in [("lokaal", [0.0, 0.0]), ("rd", [155_000.0, 463_000.0])] {
            let p = |x: f64, y: f64, z: f64| [(origin[0] + x) * f, (origin[1] + y) * f, z * f];
            let ground = [
                p(0.0, 0.0, 0.0),
                p(10.0, 0.0, 0.0),
                p(10.0, 10.0, 0.0),
                p(0.0, 10.0, 0.0),
                p(5.0, 5.0, centre),
            ];
            let design = [
                p(0.0, 0.0, level),
                p(10.0, 0.0, level),
                p(10.0, 10.0, level),
                p(0.0, 10.0, level),
            ];
            let mut doc = CadDocument::new();
            let mut axis = LwPolyline::new();
            for point in &design[..2] {
                axis.add_point_with_bulge(Vector2::new(point[0], point[1]), 0.0);
            }
            add(
                &mut doc,
                EntityType::LwPolyline(axis),
                "N-XX-AM-AS-G",
                &[("as", "A1")],
            );
            add(
                &mut doc,
                mesh(&ground, &[[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]]),
                "B-XX-OG-TRIANGULATIE-G",
                &[],
            );
            add(
                &mut doc,
                mesh(&design, &[[0, 1, 2], [0, 2, 3]]),
                "N-XX-OG-TRIANGULATIE-G",
                &[("as", "A1"), ("corridor", "Proefvak")],
            );
            let file = format!("grondlichaam-{name}-{unit}-{location}");
            let site = Site {
                origin_e: origin[0],
                origin_n: origin[1],
                absolute: true,
                ..Site::default()
            };
            let source = ifc::Source {
                doc: &doc,
                f,
                site: Some(&site),
                name: format!("{file}.dwg"),
            };
            let report = ifc::write(
                &source,
                &ifc::Options::default(),
                &directory.join(format!("{file}.ifc")),
            )
            .unwrap();
            assert_eq!(report.classes.get("IfcEarthworksFill"), Some(&1));
            if report.classes.get("IfcEarthworksCut").copied().unwrap_or(0) != expected.len() - 1 {
                errors.push(format!("{file}: onjuist aantal ontgravingen"));
            }
            let output = std::fs::read_to_string(directory.join(format!("{file}.ifc"))).unwrap();
            let volumes: Vec<f64> = output
                .lines()
                .filter(|line| line.contains("'Volume'"))
                .map(|line| {
                    line.split("IFCREAL(")
                        .nth(1)
                        .unwrap()
                        .split(')')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap()
                })
                .collect();
            println!("{file}: {volumes:?} m³; verwacht {expected:?}");
            // De bestaande eigenschap bewaart drie decimalen; de Python-test
            // controleert de geschreven driehoeken zelf tot op 0,000001 m³.
            if volumes.len() != expected.len()
                || volumes
                    .iter()
                    .zip(expected)
                    .any(|(a, b)| (a - b).abs() > 0.0005)
            {
                errors.push(format!("{file}: {volumes:?}, verwacht {expected:?}"));
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn earthworks_preserve_ground_peak() {
    check_earthworks("top", 1.0, 2.0, &[500.0 / 3.0]);
}

#[test]
fn earthworks_preserve_ground_valley() {
    check_earthworks("dal", -1.0, 2.0, &[700.0 / 3.0]);
}

#[test]
fn earthworks_split_crossing_surfaces() {
    check_earthworks("snijding", 1.0, 0.5, &[125.0 / 6.0, 25.0 / 6.0]);
}
