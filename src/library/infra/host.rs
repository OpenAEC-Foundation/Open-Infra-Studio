//! CAD serialization and atomic regeneration of connected object trees.
use super::{
    connect,
    geometry::{self, Primitive},
    Item, Placement,
};
use crate::{
    bridge,
    nlcs::{self, Element, LayerSpec},
};
use ocs_plugin_api::host::codec::entities::hatch::PolylineEdge;
use ocs_plugin_api::host::codec::entities::{
    BoundaryEdge, BoundaryPath, Hatch, HatchPattern, HatchPatternLine, HatchPatternType,
};
use ocs_plugin_api::host::{
    codec::{
        entities::Polyline3D, CadDocument, Circle, Color, EntityType, Handle, LineWeight,
        LwPolyline, Vector2, Vector3,
    },
    XDataValue,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use std::collections::BTreeMap;

pub fn data(e: &EntityType) -> BTreeMap<String, String> {
    e.common()
        .extended_data
        .get_record(bridge::XDATA_APP)
        .map(|r| {
            r.values
                .iter()
                .filter_map(|v| {
                    if let XDataValue::String(s) = v {
                        s.split_once('=')
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
fn chunks(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + 220).min(s.len());
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        out.push(s[start..end].to_string());
        start = end;
    }
    out
}
pub fn scene(doc: &CadDocument) -> Result<Vec<Placement>, String> {
    let mut out = vec![];
    for e in doc.model_space_entities() {
        let d = data(e);
        if d.get("infra_primary").map(String::as_str) != Some("1") {
            continue;
        }
        let text = d
            .iter()
            .filter(|(k, _)| k.starts_with("infra_p_"))
            .map(|(_, v)| v.as_str())
            .collect::<String>();
        let p: Placement = serde_json::from_str(&text)
            .map_err(|e| format!("beschadigde infra-parameters: {e}"))?;
        out.push(p);
    }
    Ok(out)
}
pub fn entities(
    item: &Item,
    p: &Placement,
    u: f64,
) -> Result<(Vec<LayerSpec>, Vec<EntityType>), String> {
    if !u.is_finite() || u <= 0. {
        return Err("ongeldige tekeningeenheid".into());
    }
    let built = geometry::build(item, p)?;
    let encoded = chunks(&serde_json::to_string(p).map_err(|e| e.to_string())?);
    let status = nlcs::Status::from_code(&p.status).ok_or("status")?;
    let mut layers = vec![];
    let mut out = vec![];
    for (part_index, part) in built.parts.into_iter().enumerate() {
        let obj = nlcs::object(part.object_id).ok_or("NLCS-object")?;
        let element = match part.element.as_str() {
            "G" => Element::G,
            "S" => Element::S,
            "A" => Element::A,
            "T" => Element::T,
            _ => return Err("elementtype".into()),
        };
        if !nlcs::element_allowed(obj, element.code()) {
            return Err("element niet toegestaan".into());
        }
        let spec = nlcs::layer(obj, status, &p.discipline, element);
        let site = bridge::site();
        let v3 = |v: [f64; 3]| {
            let xy = site
                .as_ref()
                .map(|s| s.to_drawing([v[0], v[1]], u))
                .unwrap_or([v[0] * u, v[1] * u]);
            Vector3::new(xy[0], xy[1], v[2] * u)
        };
        let mut e = match part.geometry {
            Primitive::Hatch {
                boundary,
                patterns,
                origin,
                ..
            } => {
                let mut pattern = HatchPattern::new(format!("A{}-OI_SCHEMATISCH", obj.group));
                pattern.description = "Eigen schematisch materiaalpatroon".into();
                let base = v3(origin);
                for [spacing, angle] in patterns {
                    pattern.add_line(HatchPatternLine {
                        angle,
                        base_point: Vector2::new(base.x, base.y),
                        offset: Vector2::new(-angle.sin() * spacing * u, angle.cos() * spacing * u),
                        dash_lengths: vec![],
                    });
                }
                let mut hatch = Hatch::with_pattern(pattern);
                hatch.pattern_type = HatchPatternType::UserDefined;
                hatch.elevation = boundary[0][2] * u;
                let mut path = BoundaryPath::external();
                path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
                    boundary
                        .into_iter()
                        .map(|v| {
                            let v = v3(v);
                            Vector2::new(v.x, v.y)
                        })
                        .collect(),
                    true,
                )));
                hatch.paths.push(path);
                EntityType::Hatch(Box::new(hatch))
            }
            Primitive::Poly { points, closed } => {
                if points.iter().any(|v| (v[2] - points[0][2]).abs() > 1e-10) {
                    let mut poly = Polyline3D::from_points(points.into_iter().map(v3).collect());
                    poly.flags.closed = closed;
                    EntityType::Polyline3D(poly)
                } else {
                    let mut poly = LwPolyline::new();
                    poly.elevation = points[0][2] * u;
                    for a in points {
                        let v = v3(a);
                        poly.add_point_with_bulge(Vector2::new(v.x, v.y), 0.);
                    }
                    poly.is_closed = closed;
                    EntityType::LwPolyline(poly)
                }
            }
            Primitive::Circle { center, radius } => {
                EntityType::Circle(Circle::from_center_radius(v3(center), radius * u))
            }
            Primitive::Text {
                at,
                value,
                height,
                rotation,
            } => {
                let pos = v3(at);
                let mut e = crate::emit::text(
                    [pos.x, pos.y],
                    &value,
                    height * u,
                    rotation,
                    &spec.name,
                    None,
                );
                if let EntityType::Text(t) = &mut e {
                    t.insertion_point.z = at[2] * u;
                }
                e
            }
        };
        let mut pairs = vec![
            ("infra_part".to_string(), part_index.to_string()),
            ("infra_instance".to_string(), p.instance.clone()),
            ("infra_item".into(), item.id.clone()),
            (
                "infra_primary".into(),
                if part.primary { "1" } else { "0" }.into(),
            ),
            ("infra_version".into(), "1".into()),
        ];
        if part.primary {
            pairs.push(("infra_quantity".into(), built.quantity.to_string()));
            pairs.push(("infra_unit".into(), built.unit.clone()));
            for (k, s) in encoded.iter().enumerate() {
                pairs.push((format!("infra_p_{k:04}"), s.clone()));
            }
        }
        let record = bridge::object_data(pairs.iter().map(|(k, v)| (k.as_str(), v.clone())));
        let c = e.common_mut();
        c.layer = spec.name.clone();
        c.color = Color::ByLayer;
        c.line_weight = LineWeight::ByLayer;
        c.linetype_scale = p.scale / 1000. * u;
        c.extended_data.add_record(record);
        layers.push(spec);
        out.push(e);
    }
    Ok((layers, out))
}
fn request(req: PluginRequest) -> Result<PluginResponse, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    match t.sender.request(req).map_err(|e| e.to_string())? {
        PluginResponse::Error(e) => Err(e),
        r => Ok(r),
    }
}
pub fn place(p: Placement) -> Result<serde_json::Value, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    let doc = bridge::snapshot()?;
    let mut network = scene(&doc)?;
    if network.iter().any(|v| v.instance == p.instance) {
        return Err("instantie-ID bestaat al".into());
    }
    network.push(p.clone());
    let solved = connect::resolve(&network)?;
    let p = solved
        .iter()
        .find(|v| v.instance == p.instance)
        .ok_or("instantie")?;
    let item = super::find(&p.item)?;
    let (layers, es) = entities(item, p, t.unit_factor)?;
    for layer in &layers {
        bridge::ensure_nlcs_layer(layer);
    }
    // One batch avoids partially placed objects across multiple AddEntities requests.
    request(PluginRequest::PushUndo {
        label: "Open Infra: infra-object".into(),
    })?;
    let expected = es.len();
    let handles = match request(PluginRequest::AddEntities(es))? {
        PluginResponse::Handles(h) if h.len() == expected => h,
        _ => return Err("host heeft niet alle objectonderdelen bevestigd; controleer Undo".into()),
    };
    let _ = request(PluginRequest::SetDirty);
    let _ = request(PluginRequest::BumpGeometry);
    result(item, p, &handles)
}
pub fn result(item: &Item, p: &Placement, handles: &[Handle]) -> Result<serde_json::Value, String> {
    let built = geometry::build(item, p)?;
    Ok(
        serde_json::json!({"instance":p.instance,"item":item.id,"placement":p,"ports":built.ports,"boundary":built.boundary,"quantity":built.quantity,"unit":built.unit,"warnings":built.warnings,
        "handles":handles.iter().map(|h|crate::civiel::api::handle_text(*h)).collect::<Vec<_>>()}),
    )
}
/// All connected descendants are recalculated first; the host replaces them atomically.
pub fn update(
    instance: &str,
    values: BTreeMap<String, f64>,
    path: Option<Vec<[f64; 2]>>,
) -> Result<serde_json::Value, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    let doc = bridge::snapshot()?;
    let mut network = scene(&doc)?;
    let target = network
        .iter_mut()
        .find(|v| v.instance == instance)
        .ok_or("instantie niet gevonden")?;
    target.values.extend(values);
    if let Some(path) = path {
        if target
            .link
            .as_ref()
            .is_some_and(|l| l.port.starts_with("edge-"))
        {
            return Err("wijzig het bovenliggende vlak; de band volgt de rand".into());
        }
        let (s, c) = target.rotation.sin_cos();
        target.path = path
            .into_iter()
            .map(|v| {
                let (x, y) = (v[0] - target.at[0], v[1] - target.at[1]);
                [c * x + s * y, -s * x + c * y]
            })
            .collect();
    }
    let solved = connect::resolve(&network)?;
    let mut affected = std::collections::HashSet::from([instance.to_string()]);
    loop {
        let before = affected.len();
        for p in &solved {
            if p.link
                .as_ref()
                .is_some_and(|l| affected.contains(&l.instance))
            {
                affected.insert(p.instance.clone());
            }
        }
        if affected.len() == before {
            break;
        }
    }
    let mut replacements = vec![];
    for p in &solved {
        if !affected.contains(&p.instance) {
            continue;
        }
        let item = super::find(&p.item)?;
        let (_, mut es) = entities(item, p, t.unit_factor)?;
        let mut originals: Vec<_> = doc
            .model_space_entities()
            .filter(|e| {
                data(e).get("infra_instance").map(String::as_str) == Some(p.instance.as_str())
            })
            .collect();
        originals.sort_by_key(|e| {
            data(e)
                .get("infra_part")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(usize::MAX)
        });
        if originals.len() != es.len() {
            return Err(
                "onderdelenaantal gewijzigd of object onvolledig; update afgebroken".into(),
            );
        }
        for (e, old) in es.iter_mut().zip(originals) {
            let c = e.common_mut();
            c.handle = old.common().handle;
            c.owner_handle = old.common().owner_handle;
        }
        replacements.extend(es);
    }
    match request(PluginRequest::UpdateEntitiesTransaction {
        label: "Open Infra: verbonden infra bijwerken".into(),
        entities: replacements,
    })? {
        PluginResponse::EntityTransactionResult(Ok(())) => {}
        r => return Err(format!("update niet bevestigd: {r:?}")),
    }
    let _ = request(PluginRequest::BumpGeometry);
    Ok(
        serde_json::json!({"updated":instance,"instances":affected.len(),"objects":solved.into_iter().filter(|p|affected.contains(&p.instance)).collect::<Vec<_>>()}),
    )
}
