use open_infra::gis::files;

#[test]
fn imro_preserves_repeated_value_type_pairs_texts_and_associations() {
    let data = files::gml(include_str!("fixtures/planning/synthetic-imro.gml")).unwrap();
    let object = data
        .features
        .iter()
        .find(|f| f.props["id"] == "maten")
        .unwrap();
    let attributes = &object.props["imro_attributen"];
    let measures = attributes["maatvoeringInfo"].as_array().unwrap();
    assert_eq!(measures.len(), 2);
    assert_eq!(measures[0]["WaardeEnType"]["waarde"], "10");
    assert_eq!(
        measures[0]["WaardeEnType"]["waardeType"],
        "maximum bouwhoogte (m)"
    );
    assert_eq!(measures[1]["WaardeEnType"]["waarde"], "6");
    assert_eq!(
        measures[1]["WaardeEnType"]["waardeType"],
        "maximum goothoogte (m)"
    );
    let texts = attributes["verwijzingNaarTekstInfo"].as_array().unwrap();
    assert_eq!(texts.len(), 2);
    assert_eq!(
        texts[0]["TekstReferentie_BP"]["verwijzingNaarTekst"],
        "regels.html#artikel-3"
    );
    assert_eq!(texts[1]["TekstReferentie_BP"]["typeTekst"], "toelichting");
    assert_eq!(attributes["bestemmingsvlak"]["@xlink:href"], "#zone");
    assert!(attributes.get("geometrie").is_none());
    assert_eq!(object.lines.len(), 2);
}

#[test]
fn imro_rejects_excessively_nested_metadata() {
    let original = include_str!("fixtures/planning/synthetic-imro.gml");
    let nested = format!(
        "{}value{}",
        "<imro:extra>".repeat(70),
        "</imro:extra>".repeat(70)
    );
    let xml = original.replace("<imro:naam>Wonen</imro:naam>", &nested);
    assert!(files::gml(&xml).unwrap_err().contains("attributen te diep"));
}

#[test]
fn local_file_roundtrip_keeps_skipped_object_warning() {
    let original = include_str!("fixtures/planning/synthetic-imro.gml");
    let xml = original.replace("<imro:geometrie xlink:href=\"#polygon\"/>", "");
    let file = files::gml(&xml).unwrap();
    let data = open_infra::gis::planning::Data::from_file_data(&file).unwrap();
    assert!(!data.warnings.is_empty());
    assert!(file.fields().iter().all(|k| !k.starts_with("__imro_")));
}

#[test]
fn imro_import_preserves_nested_metadata_and_referenced_geometry() {
    let d = files::gml(include_str!("fixtures/planning/synthetic-imro.gml")).unwrap();
    let plan = d
        .features
        .iter()
        .find(|f| f.props.get("naam").and_then(|v| v.as_str()) == Some("Synthetisch testplan"))
        .unwrap();
    assert_eq!(
        plan.props.get("planstatus").and_then(|v| v.as_str()),
        Some("vastgesteld")
    );
    let zone = d
        .features
        .iter()
        .find(|f| f.props.get("naam").and_then(|v| v.as_str()) == Some("Wonen"))
        .unwrap();
    assert_eq!(
        zone.lines.len(),
        2,
        "outer boundary and interior hole are resolved from xlink"
    );
}

#[test]
fn imro_rejects_external_missing_cyclic_references_and_ambiguous_crs() {
    let original = include_str!("fixtures/planning/synthetic-imro.gml");
    for (from, to) in [
        ("#polygon", "https://example.invalid/geometry"),
        ("#polygon", "#missing"),
        ("EPSG:28992", "unknown"),
    ] {
        assert!(files::gml(&original.replace(from, to)).is_err(), "{to}");
    }
    let cycle = original.replace(
        "<gml:Polygon gml:id=\"polygon\"",
        "<gml:Polygon xlink:href=\"#polygon\" gml:id=\"polygon\"",
    );
    assert!(files::gml(&cycle).is_err());
    let arc = original
        .replace("<gml:posList>", "<gml:Arc><gml:posList>")
        .replace("</gml:posList>", "</gml:posList></gml:Arc>");
    assert!(files::gml(&arc).unwrap_err().contains("gebogen"));
}
