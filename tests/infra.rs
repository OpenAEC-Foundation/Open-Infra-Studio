use open_infra::library::infra::{self, connect, geometry, Placement};

#[test]
fn surface_pattern_keeps_a_stable_entity_count_when_resized() {
    let item = infra::find("straatbaksteen-kf").unwrap();
    for length in [5.0, 12.0] {
        let mut p = Placement::default();
        p.values.insert("length".into(), length);
        let (_, es) = infra::host::entities(item, &p, 1.0).unwrap();
        assert_eq!(
            es.len(),
            2,
            "one boundary and one native hatch, independent of dimensions"
        );
    }
}

#[test]
fn every_status_and_component_has_a_valid_nlcs_layer() {
    for status in ["B", "N", "T", "V"] {
        for item in infra::catalog().unwrap() {
            let p = Placement {
                item: item.id.clone(),
                instance: "check".into(),
                status: status.into(),
                ..Default::default()
            };
            let (layers, es) = infra::host::entities(item, &p, 1.0).unwrap();
            assert_eq!(layers.len(), es.len());
            for layer in layers {
                assert_eq!(
                    open_infra::nlcs::parse_layer(&layer.name)
                        .unwrap()
                        .status
                        .code(),
                    status
                );
            }
        }
    }
}

#[test]
fn tee_angle_and_profile_thickness_change_the_actual_output() {
    let mut p = Placement::default();
    p.values.insert("angle".into(), 45.0);
    let b = geometry::build(infra::find("t-stuk-pe75").unwrap(), &p).unwrap();
    let branch = b.ports.iter().find(|v| v.id == "branch").unwrap();
    assert!((branch.direction[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-8);
    let mut p = Placement::default();
    p.values.insert("layer_1".into(), 0.2);
    let b = geometry::build(infra::find("profiel-fietsstraat").unwrap(), &p).unwrap();
    if let geometry::Primitive::Poly { points, .. } = &b.parts[0].geometry {
        assert!((points[0][1] + 0.2).abs() < 1e-8);
    } else {
        panic!("profile boundary")
    }
}

#[test]
fn catalog_has_every_design_theme_and_valid_geometry() {
    let items = infra::catalog().unwrap();
    assert!(items.len() >= 60);
    for item in items {
        let built = geometry::build(item, &Placement::default())
            .unwrap_or_else(|e| panic!("{}: {e}", item.id));
        assert!(!built.parts.is_empty(), "{}", item.id);
        assert!(built.quantity.is_finite() && built.quantity >= 0.0);
        for part in &built.parts {
            let o = open_infra::nlcs::object(part.object_id).unwrap();
            assert!(open_infra::nlcs::element_allowed(o, &part.element));
        }
    }
    for theme in [
        "verharding",
        "banden",
        "riolering",
        "hemelwater",
        "nuts",
        "verlichting",
        "groen",
        "inrichting",
        "verkeer",
        "profielen",
        "annotatie",
    ] {
        assert!(items.iter().any(|i| i.category == theme), "missing {theme}");
    }
}

#[test]
fn connection_aligns_rotated_ports_and_rejects_diameter_or_system_mismatch() {
    let pipe = infra::find("persleiding-pe75").unwrap();
    let valve = infra::find("afsluiter-pe75").unwrap();
    let mut parent = Placement {
        at: [100.0, 200.0, 8.0],
        rotation: std::f64::consts::FRAC_PI_2,
        ..Default::default()
    };
    parent.values.insert("length".into(), 10.0);
    let target = geometry::build(pipe, &parent).unwrap();
    let child = connect::attach(valve, &Placement::default(), "in", &target, "out").unwrap();
    let placed = geometry::build(valve, &child).unwrap();
    let a = target.ports.iter().find(|p| p.id == "out").unwrap();
    let b = placed.ports.iter().find(|p| p.id == "in").unwrap();
    for k in 0..3 {
        assert!((a.at[k] - b.at[k]).abs() < 1e-8);
    }
    assert!((a.direction[0] + b.direction[0]).abs() < 1e-8);
    assert!((a.direction[1] + b.direction[1]).abs() < 1e-8);
    assert!(connect::attach(
        infra::find("afsluiter-pe63").unwrap(),
        &Placement::default(),
        "in",
        &target,
        "out"
    )
    .is_err());
    assert!(connect::attach(
        infra::find("waterleiding").unwrap(),
        &Placement::default(),
        "in",
        &target,
        "out"
    )
    .is_err());
}

#[test]
fn invalid_parameters_and_bow_tie_polygon_are_rejected() {
    let i = infra::find("opsluitband-150x250").unwrap();
    for (key, value) in [("width", f64::NAN), ("length", -1.0), ("typo", 1.0)] {
        let mut p = Placement::default();
        p.values.insert(key.into(), value);
        assert!(geometry::build(i, &p).is_err(), "{key}");
    }
    let p = Placement {
        path: vec![[0., 0.], [2., 2.], [0., 2.], [2., 0.]],
        ..Default::default()
    };
    assert!(geometry::build(infra::find("grasbeton-120").unwrap(), &p).is_err());
}

#[test]
fn area_and_slope_are_computed_from_actual_geometry() {
    let p = Placement {
        path: vec![[0., 0.], [8., 0.], [8., 3.], [0., 3.]],
        ..Default::default()
    };
    let b = geometry::build(infra::find("grasbeton-120").unwrap(), &p).unwrap();
    assert!((b.quantity - 24.0).abs() < 1e-8);
    let mut p = Placement::default();
    p.values.insert("length".into(), 20.0);
    p.values.insert("slope".into(), 0.005);
    let b = geometry::build(infra::find("persleiding-pe75").unwrap(), &p).unwrap();
    let end = b.ports.iter().find(|x| x.id == "out").unwrap();
    assert!((end.at[2] + 0.1).abs() < 1e-8);
}

#[test]
fn regeneration_moves_children_and_refuses_occupied_ports_and_cycles() {
    use infra::{connect::resolve, Link};
    let mut parent = Placement {
        item: "persleiding-pe75".into(),
        instance: "main".into(),
        ..Default::default()
    };
    let child = Placement {
        item: "afsluiter-pe75".into(),
        instance: "valve".into(),
        link: Some(Link {
            instance: "main".into(),
            port: "out".into(),
            own_port: "in".into(),
        }),
        ..Default::default()
    };
    parent.values.insert("length".into(), 25.);
    let solved = resolve(&[parent.clone(), child.clone()]).unwrap();
    assert!((solved[1].at[0] - 25.).abs() < 1e-8);
    let mut duplicate = child.clone();
    duplicate.instance = "second".into();
    assert!(resolve(&[parent.clone(), child.clone(), duplicate]).is_err());
    parent.link = Some(Link {
        instance: "valve".into(),
        port: "out".into(),
        own_port: "in".into(),
    });
    assert!(resolve(&[parent, child]).is_err());
}

#[test]
fn metadata_roundtrip_and_drawing_units_preserve_quantities() {
    use ocs_plugin_api::host::codec::CadDocument;
    let i = infra::find("persleiding-pe75").unwrap();
    let p = Placement {
        item: i.id.clone(),
        instance: "main".into(),
        at: [187000., 432000., 9.],
        ..Default::default()
    };
    for factor in [1., 1000.] {
        let (_, es) = infra::host::entities(i, &p, factor).unwrap();
        let mut doc = CadDocument::new();
        for e in es {
            doc.add_entity(e).unwrap();
        }
        let read = infra::host::scene(&doc).unwrap();
        assert_eq!(read[0].at, p.at);
        let q = open_infra::hoeveelheden::compute(&doc, factor, None);
        assert_eq!(q.rows.iter().map(|r| r.count).sum::<usize>(), 1);
        assert!((q.rows.iter().map(|r| r.length).sum::<f64>() - 10.).abs() < 1e-8);
    }
}

#[test]
fn curb_fits_surface_edge_and_an_unknown_level_is_not_a_nap_value() {
    let mut surface = Placement::default();
    surface.values.insert("length".into(), 8.);
    surface.values.insert("width".into(), 3.);
    let area = geometry::build(infra::find("grasbeton-120").unwrap(), &surface).unwrap();
    let curb = infra::find("opsluitband-150x250").unwrap();
    let p = connect::attach(curb, &Placement::default(), "in", &area, "edge-0").unwrap();
    let b = geometry::build(curb, &p).unwrap();
    assert!((b.quantity - 8.).abs() < 1e-8);
    assert!((p.at[1] + 0.075).abs() < 1e-8);
    let annotation =
        geometry::build(infra::find("hoogtepunt").unwrap(), &Placement::default()).unwrap();
    assert!(!annotation.elevation_known);
    assert!(annotation.warnings.iter().any(|w| w.contains("Hoogte")));
}
