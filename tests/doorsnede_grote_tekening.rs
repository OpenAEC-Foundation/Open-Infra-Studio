//! Cross sections on a drawing too large for the shared view (#61, point 6):
//! the BGT surfaces, KLIC cables and survey points along the axis must still
//! be cut, or the command must refuse.
use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{
    codec::{
        entities::{Face3D, LwPolyline, Point},
        EntityType, Handle, Vector2, Vector3,
    },
    EntityPage, PluginRequestError, PluginRequestSender, XRecordEntry,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use open_infra::{bridge, civiel};

const AXIS: u64 = 0x10;

/// A drawing the host cannot share as a whole: the plugin reads it by handle
/// and, when the host supports it, by query.
struct Drawing {
    entities: Vec<EntityType>,
    queries: bool,
    added: Mutex<Vec<EntityType>>,
}

fn bbox(e: &EntityType) -> Option<[f64; 4]> {
    let pts: Vec<[f64; 2]> = match e {
        EntityType::LwPolyline(p) => p.vertices.iter().map(|v| [v.location.x, v.location.y]).collect(),
        EntityType::Point(p) => vec![[p.location.x, p.location.y]],
        EntityType::Face3D(f) => [f.first_corner, f.second_corner, f.third_corner, f.fourth_corner].iter().map(|v| [v.x, v.y]).collect(),
        _ => return None,
    };
    Some(pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]))
}

/// What the host's `query` operation answers (see Open CAD Studio
/// `entity_query`): type by DXF name, XDATA application, bounds overlap.
fn matches(e: &EntityType, q: &serde_json::Value) -> bool {
    let kind = match e {
        EntityType::LwPolyline(_) => "LWPOLYLINE",
        EntityType::Point(_) => "POINT",
        EntityType::Face3D(_) => "3DFACE",
        _ => "",
    };
    if q["type"].as_str().is_some_and(|t| !t.eq_ignore_ascii_case(kind)) {
        return false;
    }
    if q["xdata_app"].as_str().is_some_and(|a| e.common().extended_data.get_record(a).is_none()) {
        return false;
    }
    if let (Some(b), Some(q)) = (bbox(e), q["bounds"].as_array()) {
        let q: Vec<f64> = q.iter().filter_map(|v| v.as_f64()).collect();
        if b[2] < q[0] || b[3] < q[1] || b[0] > q[2] || b[1] > q[3] {
            return false;
        }
    }
    true
}

impl PluginRequestSender for Drawing {
    fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        Ok(match request {
            // Too large to share: the plugin falls back to reading by handle.
            PluginRequest::OpenDocumentViewV4 { .. } => PluginResponse::Error("de tekening is te groot voor de gedeelde weergave".into()),
            PluginRequest::CloseDocumentViewV4 { .. } => PluginResponse::Ok,
            PluginRequest::ReadXRecord { name, .. } if name == "INDEX" => PluginResponse::XRecordEntries(Ok(vec![
                XRecordEntry::string(1, "tin\tB\t20 21"),
                XRecordEntry::string(1, "punten\t\t40"),
            ])),
            PluginRequest::ReadXRecord { .. } => PluginResponse::XRecordEntries(Ok(Vec::new())),
            PluginRequest::GetSelection => PluginResponse::Selection(Vec::new()),
            PluginRequest::GetEntitiesBatch { handles } => PluginResponse::GetEntitiesBatchResult(Ok(handles
                .iter()
                .map(|h| self.entities.iter().find(|e| e.common().handle == *h).cloned())
                .collect())),
            PluginRequest::QueryEntities { query } if self.queries => {
                let q: serde_json::Value = serde_json::from_str(&query).unwrap();
                let all: Vec<EntityType> = self.entities.iter().filter(|e| matches(e, &q)).cloned().collect();
                let offset = q["offset"].as_u64().unwrap_or(0) as usize;
                let limit = q["limit"].as_u64().unwrap_or(1000) as usize;
                let page: Vec<EntityType> = all.iter().skip(offset).take(limit).cloned().collect();
                let next = (offset + page.len() < all.len()).then_some((offset + page.len()) as u64);
                PluginResponse::EntityPage(Ok(EntityPage { entities: page, count: all.len() as u64, next_offset: next }))
            }
            PluginRequest::QueryEntities { .. } => PluginResponse::EntityPage(Err("query_entities is not supported by this host".into())),
            PluginRequest::AddLayer(_) => PluginResponse::OptHandle(Some(Handle::from(1000u64))),
            PluginRequest::TableOperation { .. } => PluginResponse::TableResult(Ok(Handle::from(1001u64))),
            PluginRequest::AddEntities(entities) => {
                let mut added = self.added.lock().unwrap();
                let handles = (added.len()..added.len() + entities.len()).map(|i| Handle::from(0x1000 + i as u64)).collect();
                added.extend(entities);
                PluginResponse::Handles(handles)
            }
            PluginRequest::UpdateXRecord { .. } => PluginResponse::XRecordUpdateResult(Ok(Handle::from(1002u64))),
            PluginRequest::CreateXRecord { .. } => PluginResponse::XRecordCreateResult(Ok(Handle::from(1002u64))),
            PluginRequest::PushUndo { .. }
            | PluginRequest::WriteRecord { .. }
            | PluginRequest::SetDirty
            | PluginRequest::BumpGeometry
            | PluginRequest::PushInfo(_)
            | PluginRequest::PushError(_) => PluginResponse::Ok,
            other => return Err(PluginRequestError(format!("onverwacht hostverzoek: {other:?}"))),
        })
    }
}

fn polyline(handle: u64, layer: &str, pts: &[[f64; 2]], closed: bool, data: &[(&str, &str)]) -> EntityType {
    let mut p = LwPolyline::new();
    for q in pts {
        p.add_point_with_bulge(Vector2::new(q[0], q[1]), 0.0);
    }
    p.is_closed = closed;
    p.common.handle = Handle::from(handle);
    p.common.layer = layer.into();
    if !data.is_empty() {
        p.common.extended_data.add_record(bridge::object_data(data.iter().map(|(k, v)| (*k, v.to_string()))));
    }
    EntityType::LwPolyline(p)
}

/// A 100 m axis due east over flat ground at NAP +1 m. Sections every 20 m,
/// 20 m to both sides: the one at 20 m cuts a BGT road, the one at 60 m a
/// KLIC cable, the one at 80 m passes a survey point. Only the ground and
/// the survey point are in the Open Infra index; BGT and KLIC are not.
fn drawing(queries: bool) -> Drawing {
    let face = |h: u64, c: [[f64; 2]; 3]| {
        let v = |p: [f64; 2]| Vector3::new(p[0], p[1], 1.0);
        let mut f = Face3D::new(v(c[0]), v(c[1]), v(c[2]), v(c[2]));
        f.common.handle = Handle::from(h);
        f.common.layer = "B-OG-TRIANGULATIE-G".into();
        EntityType::Face3D(f)
    };
    let mut point = Point::new();
    point.location = Vector3::new(80.0, 0.5, 1.2);
    point.common.handle = Handle::from(0x40);
    point.common.layer = "B-ME-MEETPUNT-G".into();
    point.common.extended_data.add_record(bridge::object_data([("puntnummer", "101".to_string()), ("z", "1.2".to_string())]));
    Drawing {
        entities: vec![
            polyline(AXIS, "N-WE-AM-AS-G", &[[0.0, 0.0], [100.0, 0.0]], false, &[("as", "As 1")]),
            face(0x20, [[-50.0, -50.0], [150.0, -50.0], [150.0, 50.0]]),
            face(0x21, [[-50.0, -50.0], [150.0, 50.0], [-50.0, 50.0]]),
            polyline(0x30, "B-WE-RIJBAAN-V", &[[10.0, -5.0], [30.0, -5.0], [30.0, 5.0], [10.0, 5.0]], true, &[("bron", "BGT")]),
            polyline(0x31, "B-KL-LAAGSPANNING-G", &[[50.0, -30.0], [70.0, 30.0]], false, &[("bron", "KLIC"), ("thema", "laagspanning")]),
            EntityType::Point(point),
        ],
        queries,
        added: Mutex::new(Vec::new()),
    }
}

fn run(d: Arc<Drawing>, tab: u64) -> Result<String, String> {
    let target = bridge::Target { sender: d, tab_id: tab, unit_factor: 1.0, doc_path: None };
    bridge::with_command_context(target, None, None, || {
        civiel::as_profiel::cross_sections(&civiel::CivielOptions::default(), Some(Handle::from(AXIS)), Some([0.0, -200.0]), &|_| {})
    })
}

#[test]
fn large_drawing_cross_sections_cut_bgt_klic_and_survey_points() {
    let d = Arc::new(drawing(true));
    let msg = run(d.clone(), 61_601).unwrap();
    assert!(msg.contains("Gesneden: 1 BGT-vlakdelen, 1 kabel- en leidingkruisingen"), "{msg}");
    assert!(msg.contains("1 meetpunten binnen 1 m"), "{msg}");
}

#[test]
fn large_drawing_without_query_support_refuses_cross_sections() {
    let d = Arc::new(drawing(false));
    let err = run(d.clone(), 61_602).unwrap_err();
    assert!(err.contains(&open_infra::lang::tr("BGT, KLIC en meetpunten", "BGT, KLIC and survey points")), "{err}");
    assert!(d.added.lock().unwrap().is_empty(), "er is toch een onvolledige doorsnede getekend");
}
