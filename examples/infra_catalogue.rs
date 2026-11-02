//! Reproducible CAD catalogue and connection demonstration, using production geometry.
use ocs_plugin_api::host::codec::{
    tables::{AppId, Layer, LineType, LineTypeElement},
    CadDocument, Color, DwgReader, DwgWriter, DxfReader, DxfWriter, LineWeight,
};
use open_infra::{
    library::infra::{self, connect, geometry, host, Link, Placement},
    nlcs,
};
use std::{collections::BTreeSet, fs, path::Path};

fn new_doc() -> CadDocument {
    let mut doc = CadDocument::new();
    doc.header.insertion_units = 6;
    let _ = doc.app_ids.add(AppId::new("OPENINFRA"));
    doc
}
fn add(doc: &mut CadDocument, p: &Placement) -> Result<(), Box<dyn std::error::Error>> {
    let item = infra::find(&p.item)?;
    let (layers, entities) = host::entities(item, p, 1.)?;
    for spec in layers {
        if !doc
            .line_types
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(&spec.linetype))
        {
            let mut lt = LineType::new(&spec.linetype);
            lt.description = "NLCS; complex shape/text components simplified".into();
            if let Some(pattern) = nlcs::linetype_pattern(&spec.linetype) {
                lt.pattern_length = pattern.iter().map(|x| x.abs()).sum();
                lt.elements = pattern
                    .into_iter()
                    .map(|length| LineTypeElement {
                        length,
                        complex: None,
                    })
                    .collect();
            }
            let _ = doc.line_types.add(lt);
        }
        if !doc.layers.iter().any(|l| l.name == spec.name) {
            let mut l = Layer::new(&spec.name);
            l.color = Color::Index(spec.color);
            l.line_weight = spec
                .lineweight
                .map(LineWeight::from_value)
                .unwrap_or(LineWeight::ByLayer);
            l.line_type = spec.linetype;
            doc.layers.add(l)?;
        }
    }
    for e in entities {
        doc.add_entity(e)?;
    }
    Ok(())
}
fn write_checked(
    doc: &CadDocument,
    out: &Path,
    stem: &str,
    count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let dxf = out.join(format!("{stem}.dxf"));
    DxfWriter::new(doc).write_to_file(&dxf)?;
    let read = DxfReader::from_file(&dxf)?.read()?;
    assert_eq!(host::scene(&read)?.len(), count, "DXF parameter roundtrip");
    let dwg = out.join(format!("{stem}.dwg"));
    DwgWriter::write_to_file(&dwg, doc)?;
    let read = DwgReader::from_file(&dwg)?.read()?;
    assert_eq!(host::scene(&read)?.len(), count, "DWG parameter roundtrip");
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/infra-bibliotheek".into());
    let out = Path::new(&dir);
    fs::create_dir_all(out)?;
    let items = infra::catalog()?;
    let mut doc = new_doc();
    let mut previews = vec![];
    let text_obj = nlcs::object_by_name("AL", "TEKENBLAD_VERKLARING").ok_or("legend object")?;
    let text_layer = nlcs::layer(text_obj, nlcs::Status::N, "XX", nlcs::Element::T);
    let mut layer = Layer::new(&text_layer.name);
    layer.color = Color::Index(text_layer.color);
    doc.layers.add(layer)?;
    for (k, item) in items.iter().enumerate() {
        let p = Placement {
            item: item.id.clone(),
            instance: format!("catalogue-{k}"),
            at: [(k % 4) as f64 * 18., (k / 4) as f64 * 14., 0.],
            scale: 50.,
            ..Default::default()
        };
        add(&mut doc, &p)?;
        let label = open_infra::emit::text(
            [p.at[0] + 4., p.at[1] - 2.],
            &item.name_nl,
            0.23,
            0.,
            &text_layer.name,
            None,
        );
        doc.add_entity(label)?;
        let built = geometry::build(
            item,
            &Placement {
                scale: 50.,
                ..Default::default()
            },
        )?;
        previews.push(serde_json::json!({"id":item.id,"name":item.name_nl,"category":item.category,"built":built}));
    }
    write_checked(&doc, out, "catalogus", items.len())?;
    fs::write(
        out.join("catalogus.json"),
        serde_json::to_vec_pretty(&previews)?,
    )?;
    let mut demo = vec![Placement {
        item: "persleiding-pe75".into(),
        instance: "hoofd".into(),
        at: [0., 0., 9.],
        elevation_known: true,
        ..Default::default()
    }];
    demo[0].values.insert("slope".into(), 0.003);
    for (item, id, parent) in [
        ("afsluiter-pe75", "klep", "hoofd"),
        ("reductie-75-63", "verloop", "klep"),
        ("persleiding-pe63", "tak", "verloop"),
    ] {
        let mut p = Placement {
            item: item.into(),
            instance: id.into(),
            link: Some(Link {
                instance: parent.into(),
                port: "out".into(),
                own_port: "in".into(),
            }),
            ..Default::default()
        };
        if item.starts_with("persleiding") {
            p.values.insert("length".into(), 5.);
            p.values.insert("slope".into(), 0.003);
        }
        demo.push(p);
    }
    let mut p = Placement {
        item: "straatbaksteen-kf".into(),
        instance: "vak".into(),
        at: [0., 8., 9.],
        elevation_known: true,
        ..Default::default()
    };
    p.values.insert("length".into(), 8.);
    p.values.insert("width".into(), 3.);
    demo.push(p);
    demo.push(Placement {
        item: "opsluitband-150x250".into(),
        instance: "band".into(),
        link: Some(Link {
            instance: "vak".into(),
            port: "edge-0".into(),
            own_port: "in".into(),
        }),
        ..Default::default()
    });
    let solved = connect::resolve(&demo)?;
    let mut doc = new_doc();
    for p in &solved {
        add(&mut doc, p)?;
    }
    write_checked(&doc, out, "aansluitvoorbeeld", solved.len())?;
    let mut changed = demo;
    changed[0].values.insert("length".into(), 12.);
    let after = connect::resolve(&changed)?;
    assert!((after.iter().find(|p| p.instance == "klep").unwrap().at[0] - 12.).abs() < 1e-8);
    fs::write(
        out.join("aansluitvoorbeeld.json"),
        serde_json::to_vec_pretty(&solved)?,
    )?;
    let themes: BTreeSet<_> = items.iter().map(|i| i.category.as_str()).collect();
    let mut index=String::from("# Infra-objecten\n\nEigen parametrische geometrie, NLCS-classificatie 5.0.2. Alle maten zijn configureerbare referentiewaarden.\n\n");
    for theme in themes {
        index.push_str(&format!("## {theme}\n\n| ID | Onderdeel | NLCS-object | Soort | Poorten |\n|---|---|---|---|---|\n"));
        for i in items.iter().filter(|i| i.category == theme) {
            let obj = nlcs::object(i.object_id).unwrap();
            index.push_str(&format!(
                "| `{}` | {} | {} / {} | {} | {} |\n",
                i.id, i.name_nl, obj.group, obj.name, i.kind, i.port_count
            ));
        }
        index.push('\n');
    }
    fs::write("docs/infra-objecten.md", format!("{}\n", index.trim_end()))?;
    println!("{} catalogue objects, {} connected demo objects; DXF and DWG reopened with parameters intact",items.len(),solved.len());
    Ok(())
}
