//! Native CAD representation of IMRO map data; the rendering is not a legal determination.
use crate::{
    bridge, emit,
    gis::{
        crs::{Transformer, XY},
        files,
        planning::{self, Data, Object},
        site::Site,
        vector,
    },
};
use ocs_plugin_api::{
    host::{
        codec::{
            entities::{
                hatch::PolylineEdge, BoundaryEdge, BoundaryPath, Hatch, HatchPattern,
                HatchPatternLine, HatchPatternType,
            },
            Color, EntityType, LineWeight, Vector2,
        },
        ExtendedDataRecord, TableOperation, XDataValue,
    },
    ipc::protocol::{PluginRequest, PluginResponse},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

const DASH: &str = "IMRO_PLANCONTOUR";

pub struct Rendered {
    /// Solids first, then patterned overlays, outlines and labels.
    pub entities: Vec<EntityType>,
    pub layers: BTreeMap<String, [u8; 3]>,
    pub objects: usize,
}

fn layer(object: &Object, suffix: &str) -> String {
    let plan = object
        .properties
        .get("plan_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let key = format!("{plan}-{}-{}", object.kind, object.group);
    // Keep layer names safe even when a supplied name contains multibyte text.
    let clean: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .take(42)
        .collect();
    let hash = key.bytes().fold(2166136261_u32, |h, b| {
        h.wrapping_mul(16777619) ^ u32::from(b)
    });
    format!("IMRO_{clean}_{hash:08X}_{suffix}")
}

fn metadata(object: &Object, source: Option<&str>) -> Result<ExtendedDataRecord, String> {
    let mut pairs = vec![
        ("imro_id".to_string(), object.id.clone()),
        ("imro_type".into(), object.kind.clone()),
        ("imro_groep".into(), object.group.clone()),
        ("imro_naam".into(), object.name.clone()),
    ];
    for key in [
        "plan_id",
        "planstatus",
        "planstatusdatum",
        "eindeRechtsgeldigheid",
        "verwijzingNaarTekst",
    ] {
        if let Some(value) = object.properties.get(key) {
            pairs.push((
                key.into(),
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string()),
            ));
        }
    }
    if let Some(source) = source {
        pairs.push(("imro_bron".into(), source.into()));
    }
    let text=serde_json::to_string(&json!({"id":object.id,"kind":object.kind,"group":object.group,"name":object.name,"properties":object.properties,"source":source})).map_err(|e|e.to_string())?;
    let mut record = bridge::object_data(pairs.iter().map(|(k, v)| (k.as_str(), v.clone())));
    let mut start = 0;
    let mut index = 0;
    while start < text.len() {
        let mut end = (start + 220).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        // Preserve whitespace at chunk boundaries; searchable fields use the
        // ordinary trimmed object-data helper, the lossless payload does not.
        record.values.push(XDataValue::String(format!(
            "imro_json_{index:05}={}",
            &text[start..end]
        )));
        start = end;
        index += 1;
    }
    Ok(record)
}

fn styled(
    mut entity: EntityType,
    name: &str,
    weight: i16,
    data: &ExtendedDataRecord,
) -> EntityType {
    let c = entity.common_mut();
    c.layer = name.into();
    c.color = Color::ByLayer;
    c.line_weight = LineWeight::from_value(weight);
    c.extended_data.add_record(data.clone());
    entity
}

fn label_text(object: &Object) -> String {
    fn pairs(value: &Value, out: &mut Vec<String>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    pairs(value, out)
                }
            }
            Value::Object(fields) => {
                if let (Some(value), Some(kind)) = (
                    fields.get("waarde"),
                    fields.get("waardeType").and_then(Value::as_str),
                ) {
                    let value = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    out.push(format!("{kind}: {value}"));
                } else {
                    for value in fields.values() {
                        pairs(value, out)
                    }
                }
            }
            _ => {}
        }
    }
    if object.kind.to_lowercase().contains("maatvoering") {
        let mut labels = vec![];
        if let Some(values) = object.properties.get("maatvoeringInfo").or_else(|| {
            object
                .properties
                .get("imro_attributen")
                .and_then(|a| a.get("maatvoeringInfo"))
        }) {
            pairs(values, &mut labels)
        }
        if !labels.is_empty() {
            return labels.join("; ");
        }
    }
    if object.name.trim().is_empty() {
        object
            .properties
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .into()
    } else {
        object.name.clone()
    }
}

/// Choose an interior label point, considering all holes as well as concave exteriors.
fn label_point(rings: &[Vec<XY>]) -> Option<XY> {
    let exterior = rings.first()?;
    let inside = |p: XY| {
        vector::point_in_ring(p, exterior)
            && !rings.iter().skip(1).any(|r| vector::point_in_ring(p, r))
    };
    if let Some(p) = vector::label_point(exterior).filter(|p| inside(*p)) {
        return Some(p);
    }
    let mut ys: Vec<_> = rings.iter().flatten().map(|p| p[1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut best: Option<(f64, XY)> = None;
    for w in ys.windows(2) {
        let y = (w[0] + w[1]) / 2.;
        let mut xs = vec![];
        for ring in rings {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for span in xs.chunks_exact(2) {
            let p = [(span[0] + span[1]) / 2., y];
            let width = span[1] - span[0];
            if inside(p) && best.as_ref().is_none_or(|(d, _)| width > *d) {
                best = Some((width, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn hatch(rings: &[Vec<XY>], patterned: bool, unit_factor: f64) -> Hatch {
    let mut hatch = if patterned {
        let mut pattern = HatchPattern::new("IMRO_DUBBELBESTEMMING");
        let angle = std::f64::consts::FRAC_PI_4;
        let spacing = 2. * unit_factor;
        pattern.add_line(HatchPatternLine {
            angle,
            base_point: Vector2::new(rings[0][0][0], rings[0][0][1]),
            offset: Vector2::new(-angle.sin() * spacing, angle.cos() * spacing),
            dash_lengths: vec![],
        });
        let mut h = Hatch::with_pattern(pattern);
        h.pattern_type = HatchPatternType::UserDefined;
        h
    } else {
        Hatch::solid()
    };
    for (i, ring) in rings.iter().enumerate() {
        let mut path = if i == 0 {
            BoundaryPath::external()
        } else {
            BoundaryPath::new()
        };
        path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
            ring.iter().map(|p| Vector2::new(p[0], p[1])).collect(),
            true,
        )));
        hatch.paths.push(path);
    }
    hatch
}

/// Render a complete dataset without contacting the CAD host.
pub fn render(site: &Site, data: &Data, unit_factor: f64, text_m: f64) -> Result<Rendered, String> {
    render_with_source(site, data, unit_factor, text_m, None)
}

fn render_with_source(
    site: &Site,
    data: &Data,
    unit_factor: f64,
    text_m: f64,
    source: Option<&str>,
) -> Result<Rendered, String> {
    if !unit_factor.is_finite() || unit_factor <= 0. || !text_m.is_finite() || text_m <= 0. {
        return Err("Ongeldige tekeningeenheid of teksthoogte".into());
    }
    site.register_crs();
    let transform = Transformer::new(&data.crs, &site.crs)?;
    let convert = |p: XY| -> Result<XY, String> {
        let q = if data.crs.eq_ignore_ascii_case(&site.crs) {
            p
        } else {
            transform.apply(p)?
        };
        let q = site.to_drawing(q, unit_factor);
        if q.iter().all(|v| v.is_finite()) {
            Ok(q)
        } else {
            Err("Ongeldig planpunt na coördinatenomzetting".into())
        }
    };
    let mut out = Rendered {
        entities: vec![],
        layers: BTreeMap::new(),
        objects: 0,
    };
    let mut overlays = vec![];
    let mut outlines = vec![];
    let mut labels = vec![];
    let mut label_boxes: Vec<[f64; 4]> = vec![];
    for object in &data.objects {
        let polygons =
            planning::polygons(&object.geometry).map_err(|e| format!("{}: {e}", object.id))?;
        let style = planning::style(&object.kind, &object.group);
        let record = metadata(object, source)?;
        let border_layer = layer(object, "G");
        let fill_layer = layer(object, "A");
        let text_layer = layer(object, "T");
        let before = out.entities.len() + overlays.len() + outlines.len();
        let mut anchor = None;
        let mut label_rings = None;
        for polygon in polygons {
            let rings: Vec<Vec<XY>> = polygon
                .into_iter()
                .map(|r| r.into_iter().map(convert).collect::<Result<Vec<_>, _>>())
                .collect::<Result<_, _>>()?;
            if style.fill || style.hatch {
                let entity = styled(
                    EntityType::Hatch(Box::new(hatch(&rings, style.hatch, unit_factor))),
                    &fill_layer,
                    0,
                    &record,
                );
                out.layers.insert(fill_layer.clone(), style.color);
                if style.hatch {
                    overlays.push(entity)
                } else {
                    out.entities.push(entity)
                }
            }
            if anchor.is_none() {
                anchor = label_point(&rings);
                if anchor.is_some() {
                    label_rings = Some(rings.clone());
                }
            }
            let lines: Vec<_> = rings
                .into_iter()
                .map(|mut r| {
                    if r.first() == r.last() {
                        r.pop();
                    }
                    (r, true)
                })
                .collect();
            outlines.extend(
                emit::polylines(&lines, &border_layer, unit_factor, Some(&record))
                    .into_iter()
                    .map(|e| {
                        let mut e = styled_without_record(e, style.weight);
                        if style.dash {
                            e.common_mut().linetype = DASH.into();
                        }
                        e
                    }),
            );
        }
        // Nonpolygon parts retain their point/line representation, including mixed collections.
        let features = vector::parse_geojson(
            &json!({"features":[{"geometry":object.geometry,"properties":{}}]}),
        );
        for feature in features {
            let lines = feature
                .lines
                .into_iter()
                .filter(|(_, closed)| !*closed)
                .map(|(l, c)| {
                    Ok((
                        l.into_iter()
                            .map(convert)
                            .collect::<Result<Vec<_>, String>>()?,
                        c,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let points = feature
                .points
                .into_iter()
                .map(convert)
                .collect::<Result<Vec<_>, String>>()?;
            if anchor.is_none() {
                anchor = points.first().copied().or_else(|| {
                    lines
                        .first()
                        .and_then(|(l, _)| emit::label_spot(l).map(|(p, _)| p))
                });
            }
            outlines.extend(
                emit::polylines(&lines, &border_layer, unit_factor, Some(&record))
                    .into_iter()
                    .map(|e| {
                        let mut e = styled_without_record(e, style.weight);
                        if style.dash {
                            e.common_mut().linetype = DASH.into();
                        }
                        e
                    }),
            );
            outlines.extend(emit::markers(
                &points,
                &border_layer,
                text_m * unit_factor * 0.2,
                Some(&record),
            ));
        }
        if out.entities.len() + overlays.len() + outlines.len() > before {
            out.objects += 1;
            out.layers.insert(border_layer.clone(), [40, 40, 40]);
            let label = label_text(object);
            if let Some(mut at) = anchor.filter(|_| !label.trim().is_empty()) {
                let h = text_m * unit_factor;
                let w = label.chars().count() as f64 * h * 0.65;
                let box_at = |p: XY| {
                    [
                        p[0] - w / 2.,
                        p[1] - h * 0.65,
                        p[0] + w / 2.,
                        p[1] + h * 0.65,
                    ]
                };
                for offset in [0., 2., -2., 4., -4., 6., -6., 8., -8.] {
                    let p = [at[0], at[1] + offset * h];
                    let interior = label_rings.as_ref().is_none_or(|rings| {
                        vector::point_in_ring(p, &rings[0])
                            && !rings.iter().skip(1).any(|r| vector::point_in_ring(p, r))
                    });
                    let b = box_at(p);
                    if interior
                        && !label_boxes
                            .iter()
                            .any(|a| a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1])
                    {
                        at = p;
                        break;
                    }
                }
                label_boxes.push(box_at(at));
                out.layers.insert(text_layer.clone(), [40, 40, 40]);
                labels.push(emit::text(
                    at,
                    &label,
                    text_m * unit_factor,
                    0.,
                    &text_layer,
                    Some(&record),
                ));
            }
        }
    }
    out.entities.extend(overlays);
    out.entities.extend(outlines);
    out.entities.extend(labels);
    Ok(out)
}

fn styled_without_record(mut entity: EntityType, weight: i16) -> EntityType {
    entity.common_mut().line_weight = LineWeight::from_value(weight);
    entity
}

pub fn import_path(site: &Site, path: &Path, progress: &dyn Fn(String)) -> Result<String, String> {
    progress(format!("IMRO lezen: {}", path.display()));
    let file = files::read(path, None)?;
    if file.format != "IMRO" {
        return Err("Het bestand bevat geen IMRO-plangegevens".into());
    }
    place(
        site,
        &Data::from_file_data(&file)?,
        &path.display().to_string(),
        progress,
    )
}

pub fn place(
    site: &Site,
    data: &Data,
    source: &str,
    progress: &dyn Fn(String),
) -> Result<String, String> {
    place_with_text(site, data, source, 1., progress)
}

pub(crate) fn place_with_text(
    site: &Site,
    data: &Data,
    source: &str,
    text_m: f64,
    progress: &dyn Fn(String),
) -> Result<String, String> {
    let target = bridge::target().ok_or("Geen tekening actief")?;
    progress(format!("{} IMRO-objecten omzetten", data.objects.len()));
    let rendered = render_with_source(site, data, target.unit_factor, text_m, Some(source))?;
    if rendered.entities.is_empty() {
        return Err("Geen tekenbare IMRO-objecten gevonden".into());
    }
    if rendered
        .entities
        .iter()
        .any(|e| e.common().linetype == DASH)
    {
        let request = PluginRequest::TableOperation {
            operation: TableOperation::LinetypeCreate {
                name: DASH.into(),
                description: "Schematische IMRO-plancontour".into(),
                pattern: vec![2.5, -1.25],
            },
        };
        match target.sender.request(request).map_err(|e| e.to_string())? {
            PluginResponse::TableResult(Ok(_)) => {}
            PluginResponse::TableResult(Err(e)) if e.to_lowercase().contains("already exists") => {}
            other => {
                return Err(format!(
                    "Plancontourlijntype kon niet worden voorbereid: {other:?}"
                ))
            }
        }
    }
    for (name, [r, g, b]) in &rendered.layers {
        bridge::ensure_layer(
            name,
            Color::Rgb {
                r: *r,
                g: *g,
                b: *b,
            },
        );
    }
    let record = rendered
        .entities
        .first()
        .and_then(|e| e.common().extended_data.get_record(bridge::XDATA_APP))
        .cloned();
    let count = rendered.entities.len();
    let handles = bridge::add_entities_handles("IMRO-plangegevens", rendered.entities)?;
    if handles.len() != count {
        return Err("De host heeft niet alle plangegevens bevestigd; controleer Undo".into());
    }
    if let (Some(handle), Some(record)) = (handles.first(), record) {
        bridge::register_object_data(*handle, record);
    }
    bridge::note_source(source);
    bridge::zoom_extents();
    for warning in &data.warnings {
        progress(warning.clone());
    }
    Ok(format!("IMRO: {} objecten, {count} elementen op {} lagen. Kaartgegevens; controleer actuele regels en rechtsgeldigheid.{}",rendered.objects,rendered.layers.len(),if data.warnings.is_empty(){String::new()}else{format!(" {}",data.warnings.join(" "))}))
}
