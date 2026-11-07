//! Vehicle blocks in the drawing: define once per drawing, insert at a
//! picked point and direction, or lay out a catalogue sheet.

use std::collections::HashSet;
use std::sync::Mutex;

use ocs_plugin_api::host::codec::{
    entities::{Insert, TextHorizontalAlignment, TextVerticalAlignment},
    Circle, Color, EntityType, LineWeight, LwPolyline, Text, Vector2, Vector3,
};
use ocs_plugin_api::host::{CommandStep, InteractiveCommand, PreviewWire, TableOperation};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};

use super::draw::{self, Shape, View};
use super::Vehicle;
use crate::bridge;
use crate::lang::tr;

/// The layer vehicle inserts go on.
pub fn layer() -> &'static str {
    tr("OI-VOERTUIGEN", "OI-VEHICLES")
}

fn request(req: PluginRequest) -> Result<PluginResponse, String> {
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    match t.sender.request(req) {
        Ok(PluginResponse::Error(e)) => Err(e),
        Ok(r) => Ok(r),
        Err(e) => Err(e.to_string()),
    }
}

fn table(op: TableOperation) -> Result<(), String> {
    match request(PluginRequest::TableOperation { operation: op })? {
        PluginResponse::TableResult(Ok(_)) => Ok(()),
        PluginResponse::TableResult(Err(e)) => Err(e),
        other => Err(format!("{other:?}")),
    }
}

/// Block geometry: layer 0, colour and lineweight ByBlock, so an insert takes
/// its layer's look.
fn entities(shapes: &[Shape], scale: f64) -> Vec<EntityType> {
    shapes
        .iter()
        .map(|s| {
            let mut e = match s {
                Shape::Poly { pts, bulges, closed } => {
                    let mut lw = LwPolyline::new();
                    for (p, b) in pts.iter().zip(bulges) {
                        lw.add_point_with_bulge(Vector2::new(p[0] * scale, p[1] * scale), *b);
                    }
                    lw.is_closed = *closed;
                    EntityType::LwPolyline(lw)
                }
                Shape::Circle { c, r } => EntityType::Circle(Circle::from_center_radius(Vector3::new(c[0] * scale, c[1] * scale, 0.0), r * scale)),
            };
            let c = e.common_mut();
            c.layer = "0".into();
            c.color = Color::ByBlock;
            c.line_weight = LineWeight::ByBlock;
            e
        })
        .collect()
}

/// Make sure the block exists in the active drawing; returns its name.
pub fn ensure_block(v: &Vehicle, view: View) -> Result<String, String> {
    define(v, v.block_name(view), || draw::shapes(v, view))
}

/// The plan block of one unit (its own front at the base point), for the
/// positions along a swept path where the units turn apart.
pub fn ensure_unit_block(v: &Vehicle, i: usize) -> Result<String, String> {
    if v.units.len() == 1 {
        return ensure_block(v, View::Top);
    }
    let name = format!("{}_U{}", v.block_name(View::Top).trim_end_matches("_TOP"), i + 1) + "_TOP";
    define(v, name, || draw::top_unit(v, i))
}

fn define(v: &Vehicle, name: String, shapes: impl FnOnce() -> Vec<Shape>) -> Result<String, String> {
    static MADE: Mutex<Option<HashSet<(u64, String)>>> = Mutex::new(None);
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let key = (t.tab_id, name.clone());
    if MADE.lock().unwrap().get_or_insert_with(HashSet::new).contains(&key) {
        return Ok(name);
    }
    // Already in the drawing (an earlier session, a template)?
    let description = format!("{} — {} ({})", v.name_en, v.dimensions(), if v.standard.is_empty() { "Open Infra" } else { &v.standard });
    let exists = table(TableOperation::BlockModify { name: name.clone(), description: Some(description.clone()), explodable: Some(true), scale_uniformly: None }).is_ok();
    if !exists {
        let geometry = entities(&shapes(), t.unit_factor);
        let handles = bridge::add_entities_handles(&format!("Open Infra: {name}"), geometry)?;
        table(TableOperation::BlockCreate { name: name.clone(), entities: handles, base_point: [0.0, 0.0, 0.0], erase_originals: true, description: Some(description) })?;
    }
    MADE.lock().unwrap().get_or_insert_with(HashSet::new).insert(key);
    Ok(name)
}

fn ensure_layer() {
    bridge::ensure_layer(layer(), Color::from_index(5));
}

fn object_data(v: &Vehicle) -> ocs_plugin_api::host::ExtendedDataRecord {
    bridge::object_data([
        ("vehicle", v.id.clone()),
        ("name", v.name_en.clone()),
        ("dimensions", v.dimensions()),
        ("standard", v.standard.clone()),
        ("source", v.source_url.clone()),
    ])
}

/// One insert of a vehicle block. `at` in drawing units, `rotation` in radians;
/// `mirror` flips a side view to drive to the left.
pub fn insert_entity(v: &Vehicle, view: View, block: &str, at: [f64; 2], rotation: f64, mirror: bool) -> EntityType {
    let mut ins = Insert::new(block, Vector3::new(at[0], at[1], 0.0));
    ins.rotation = rotation;
    if mirror {
        ins.set_x_scale(-1.0);
    }
    let mut e = EntityType::Insert(ins);
    let c = e.common_mut();
    c.layer = layer().into();
    c.color = Color::ByLayer;
    c.extended_data.add_record(object_data(v));
    let _ = view;
    e
}

/// Define the block when needed and insert it.
pub fn place(v: &Vehicle, view: View, at: [f64; 2], rotation: f64, mirror: bool) -> Result<String, String> {
    let block = ensure_block(v, view)?;
    ensure_layer();
    let handles = bridge::add_entities_handles(&format!("Open Infra: {}", v.name_en), vec![insert_entity(v, view, &block, at, rotation, mirror)])?;
    if let Some(h) = handles.first() {
        bridge::register_object_data(*h, object_data(v));
    }
    Ok(format!("{} ({}) {} {} {}.", v.name(), v.dimensions(), tr("ingevoegd als blok", "inserted as block"), block, tr("op laag", "on layer").to_string() + " " + layer()))
}

/// Lay out vehicles in rows per category, plan view with name and
/// dimensions under each, from `origin` (lower left, drawing units).
pub fn catalogue(list: &[&'static Vehicle], view: View, origin: [f64; 2], progress: &dyn Fn(String)) -> Result<String, String> {
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let u = t.unit_factor;
    ensure_layer();
    let text_layer = tr("OI-VOERTUIGEN-TEKST", "OI-VEHICLES-TEXT");
    bridge::ensure_layer(text_layer, Color::from_index(7));
    // Rows bottom-up in category order: the last category ends on top.
    let mut rows: Vec<(String, Vec<&'static Vehicle>)> = Vec::new();
    for v in list {
        match rows.last_mut() {
            Some((c, r)) if *c == v.category => r.push(v),
            _ => rows.push((v.category.clone(), vec![v])),
        }
    }
    let gap = 2.0;
    let text_h = 0.3;
    let mut y = 0.0_f64;
    let mut out: Vec<EntityType> = Vec::new();
    let mut done = 0;
    let total = list.len();
    let text = |s: &str, p: [f64; 2], h: f64, left: bool| {
        let at = Vector3::new(origin[0] + p[0] * u, origin[1] + p[1] * u, 0.0);
        let mut tx = Text::with_value(s.to_string(), at).with_height(h * u);
        tx.horizontal_alignment = if left { TextHorizontalAlignment::Left } else { TextHorizontalAlignment::Center };
        tx.vertical_alignment = TextVerticalAlignment::Middle;
        tx.alignment_point = Some(at);
        let mut e = EntityType::Text(tx);
        e.common_mut().layer = text_layer.into();
        e
    };
    for (cat, vs) in rows.iter().rev() {
        let row_h = vs.iter().map(|v| if view == View::Top { v.width() } else { draw::height(v) }).fold(0.0, f64::max);
        let mut x = 0.0_f64;
        let base = y + 3.0 * text_h + 0.4;
        for v in vs {
            progress(format!("{} {}/{}", tr("Blokken maken", "Making blocks"), done + 1, total));
            let block = ensure_block(v, view)?;
            let len = v.length();
            let at = if view == View::Top { [x + len, base + row_h / 2.0] } else { [x + len, base] };
            out.push(insert_entity(v, view, &block, [origin[0] + at[0] * u, origin[1] + at[1] * u], 0.0, false));
            out.push(text(v.name(), [x + len / 2.0, y + 2.0 * text_h + 0.1], text_h, false));
            out.push(text(&v.dimensions(), [x + len / 2.0, y + text_h], text_h * 0.8, false));
            x += len.max(3.0) + gap;
            done += 1;
        }
        out.push(text(&super::category_name(cat).to_uppercase(), [0.0, base + row_h + 0.9], text_h * 1.6, true));
        y = base + row_h + 2.5;
    }
    let n = out.len();
    let handles = bridge::add_entities_handles(tr("Open Infra: voertuigoverzicht", "Open Infra: vehicle catalogue"), out)?;
    if let Some(h) = handles.first() {
        if let Some(v) = list.first() {
            bridge::register_object_data(*h, object_data(v));
        }
    }
    bridge::zoom_extents();
    Ok(format!("{} {} ({} {}).", total, tr("voertuigen in het overzicht", "vehicles in the catalogue"), n, tr("objecten", "objects")))
}

/// Preview wires for an insert at `at` with `rotation` (drawing units).
fn preview(shapes: &[Vec<[f64; 2]>], at: [f64; 2], rotation: f64, mirror: bool, unit: f64) -> Vec<PreviewWire> {
    let (s, c) = rotation.sin_cos();
    let m = if mirror { -1.0 } else { 1.0 };
    shapes
        .iter()
        .map(|pts| {
            let wire = pts
                .iter()
                .map(|p| {
                    let (x, y) = (p[0] * unit * m, p[1] * unit);
                    [at[0] + x * c - y * s, at[1] + x * s + y * c, 0.0]
                })
                .collect();
            PreviewWire::with_color(wire, [0.4, 0.8, 1.0, 1.0])
        })
        .collect()
}

/// `GIS_VOERTUIG <id> [ZIJ]`: pick the front of the vehicle, then the
/// direction it drives to (Enter: to the east / to the right).
pub struct InsertCmd {
    pub vehicle: &'static Vehicle,
    pub view: View,
    pub unit: f64,
    first: Option<[f64; 2]>,
    wires: Vec<Vec<[f64; 2]>>,
}

impl InsertCmd {
    pub fn new(vehicle: &'static Vehicle, view: View, unit: f64) -> Self {
        let wires = draw::shapes(vehicle, view).iter().map(draw::flatten).collect();
        Self { vehicle, view, unit, first: None, wires }
    }

    fn orientation(&self, from: [f64; 2], to: [f64; 2]) -> (f64, bool) {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        if self.view == View::Side {
            (0.0, dx < 0.0)
        } else if dx.hypot(dy) < 1e-9 {
            (0.0, false)
        } else {
            (dy.atan2(dx), false)
        }
    }

    fn finish(&self, at: [f64; 2], rotation: f64, mirror: bool) -> CommandStep {
        let (v, view) = (self.vehicle, self.view);
        // A worker: this callback runs inside the host's input handling.
        bridge::spawn_command(move || match place(v, view, at, rotation, mirror) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{}: {e}", v.name())),
        });
        CommandStep::Done
    }
}

impl InteractiveCommand for InsertCmd {
    fn prompt(&self) -> String {
        match self.first {
            None => format!("{} {}", tr("Voertuig: invoegpunt (voorkant) van", "Vehicle: insertion point (front) of"), self.vehicle.name()),
            Some(_) if self.view == View::Side => tr("Voertuig: rijrichting (links of rechts) [Enter = rechts]", "Vehicle: driving direction (left or right) [Enter = right]").into(),
            Some(_) => tr("Voertuig: rijrichting [Enter = oost]", "Vehicle: driving direction [Enter = east]").into(),
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let p = [pt[0], pt[1]];
        match self.first {
            None => {
                self.first = Some(p);
                CommandStep::NeedPoint
            }
            Some(a) => {
                let (rot, mirror) = self.orientation(a, p);
                self.finish(a, rot, mirror)
            }
        }
    }
    fn on_enter(&mut self) -> CommandStep {
        match self.first {
            Some(a) => self.finish(a, 0.0, false),
            None => CommandStep::Cancel,
        }
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let p = [pt[0], pt[1]];
        match self.first {
            None => preview(&self.wires, p, 0.0, false, self.unit),
            Some(a) => {
                let (rot, mirror) = self.orientation(a, p);
                preview(&self.wires, a, rot, mirror, self.unit)
            }
        }
    }
}

/// `GIS_VOERTUIGOVERZICHT [categorie] [ZIJ]`: pick the lower-left corner.
pub struct CatalogueCmd {
    pub list: Vec<&'static Vehicle>,
    pub view: View,
}

impl InteractiveCommand for CatalogueCmd {
    fn prompt(&self) -> String {
        tr("Voertuigoverzicht: linkeronderhoek", "Vehicle catalogue: lower-left corner").into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let (list, view, at) = (self.list.clone(), self.view, [pt[0], pt[1]]);
        bridge::spawn_command(move || match catalogue(&list, view, at, &|_| {}) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&e),
        });
        CommandStep::Done
    }
}

// ── Swept paths ─────────────────────────────────────────────────────────

/// A message in the interface language (Dutch or English).
macro_rules! t {
    ($nl:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

pub fn swept_layers() -> (&'static str, &'static str, &'static str) {
    (tr("OI-VEEGPAD", "OI-SWEPT-PATH"), tr("OI-VEEGPAD-WIELEN", "OI-SWEPT-PATH-WHEELS"), tr("OI-VEEGPAD-VOERTUIG", "OI-SWEPT-PATH-VEHICLE"))
}

/// How a swept path is driven.
#[derive(Clone, Copy, Debug)]
pub struct SweptOptions {
    /// Distance between the drawn vehicle positions (m).
    pub every: f64,
    /// Reverse: the rear axle of the last unit follows the path backwards.
    pub reverse: bool,
    /// Articulation limit (degrees); reversing without one, the vehicle's
    /// jackknife angle, else 70° (an estimate, see docs/civiel-overig.md).
    pub limit: Option<f64>,
}

impl Default for SweptOptions {
    fn default() -> Self {
        SweptOptions { every: 10.0, reverse: false, limit: None }
    }
}

/// The articulation limit a swept path is checked against, and where it comes from.
pub fn articulation_limit(v: &Vehicle, o: &SweptOptions) -> (f64, String) {
    match (o.limit, o.reverse.then(|| super::swept::jackknife_angle(v)).flatten()) {
        (Some(l), _) => (l, tr("opgegeven", "given").to_string()),
        (None, Some(j)) => (j, tr("scharenhoek bij volle stuuruitslag", "jackknife angle at full lock").to_string()),
        (None, None) => (70.0, tr("standaard, schatting", "default, an estimate").to_string()),
    }
}

/// Drive `v` along `path` (metres) forward or in reverse.
pub fn simulate(v: &Vehicle, metres: &[[f64; 2]], o: &SweptOptions) -> Result<super::swept::Swept, String> {
    if o.reverse {
        super::swept::drive_reverse(v, metres, 0.05, o.every)
    } else {
        super::swept::drive(v, metres, 0.05, o.every)
    }
}

/// The traces and vehicle positions of a computed swept path as entities
/// (drawing units `u` per metre), the first one with the object data.
pub fn swept_entities(v: &Vehicle, s: &super::swept::Swept, u: f64, data: &ocs_plugin_api::host::ExtendedDataRecord) -> Result<Vec<EntityType>, String> {
    let (body_l, wheel_l, ghost_l) = swept_layers();
    bridge::ensure_layer(body_l, Color::from_index(1));
    bridge::ensure_layer(wheel_l, Color::from_index(3));
    bridge::ensure_layer(ghost_l, Color::from_index(8));
    let poly = |pts: &[[f64; 2]], layer: &str| {
        let mut lw = LwPolyline::new();
        // Every 0.05 m is more than a drawing needs: keep about every 0.25 m.
        for (k, p) in pts.iter().enumerate() {
            if k % 5 == 0 || k + 1 == pts.len() {
                lw.add_point_with_bulge(Vector2::new(p[0] * u, p[1] * u), 0.0);
            }
        }
        let mut e = EntityType::LwPolyline(lw);
        e.common_mut().layer = layer.into();
        e
    };
    let mut out: Vec<EntityType> = Vec::new();
    out.extend(s.body.iter().map(|t| poly(t, body_l)));
    out.extend(s.wheels.iter().map(|t| poly(t, wheel_l)));
    for poses in &s.ghosts {
        for (i, p) in poses.iter().enumerate() {
            let block = ensure_unit_block(v, i)?;
            let mut e = insert_entity(v, View::Top, &block, [p.front[0] * u, p.front[1] * u], p.angle(), false);
            e.common_mut().layer = ghost_l.into();
            out.push(e);
        }
    }
    if let Some(first) = out.first_mut() {
        first.common_mut().extended_data.add_record(data.clone());
    }
    Ok(out)
}

/// The object data of a swept path.
fn swept_data(v: &Vehicle, s: &super::swept::Swept) -> ocs_plugin_api::host::ExtendedDataRecord {
    bridge::object_data([
        ("vehicle", v.id.clone()),
        ("length_m", format!("{:.2}", s.length)),
        ("max_steer_deg", format!("{:.1}", s.max_steer_deg)),
        ("min_outer_radius_m", s.min_outer_radius.map(|r| format!("{r:.2}")).unwrap_or_default()),
        ("max_articulation_deg", if v.units.len() > 1 { format!("{:.1}", s.max_articulation_deg) } else { String::new() }),
        ("reverse", if s.reverse { "yes".into() } else { String::new() }),
    ])
}

/// The message of a swept path: length, steering, radius, articulation and warnings.
pub fn swept_message(v: &Vehicle, s: &super::swept::Swept, o: &SweptOptions) -> String {
    let mut msg = if s.reverse {
        t!("Veegpad (achteruit) van {} — {:.1} m, grootste stuurhoek {:.1}°", "Swept path (reversing) of {} — {:.1} m, largest steering angle {:.1}°", v.name(), s.length, s.max_steer_deg)
    } else {
        t!("Veegpad van {} — {:.1} m, grootste stuurhoek {:.1}°", "Swept path of {} — {:.1} m, largest steering angle {:.1}°", v.name(), s.length, s.max_steer_deg)
    };
    if let Some(r) = s.min_outer_radius {
        msg.push_str(&t!(", kleinste buitenstraal {:.1} m", ", smallest outer radius {:.1} m", r));
    }
    if v.units.len() > 1 {
        msg.push_str(&t!(", grootste knikhoek {:.1}° (na {:.0} m)", ", largest articulation {:.1}° (after {:.0} m)", s.max_articulation_deg, s.articulation_at));
        let (limit, why) = articulation_limit(v, o);
        if (s.reverse || o.limit.is_some()) && s.max_articulation_deg > limit {
            msg.push_str(&if s.reverse {
                t!(" — SCHAREN: knikhoek boven {:.0}° ({why}), achteruit niet te houden", " — JACKKNIFE: articulation above {:.0}° ({why}), not controllable in reverse", limit)
            } else {
                t!(" — knikhoek boven {:.0}° ({why})", " — articulation above {:.0}° ({why})", limit)
            });
        }
    }
    if let Some(design) = v.turning_radius_outer {
        if !s.reverse && s.min_outer_radius.is_some_and(|r| r < design - 0.05) {
            msg.push_str(&format!(" — {} {:.1} m: {}", tr("krapper dan de ontwerpdraaicirkel van", "tighter than the design turning radius of"), design, tr("niet haalbaar", "not drivable")));
        }
    }
    if s.max_steer_deg > 45.0 {
        msg.push_str(&format!(" — {}", tr("stuurhoek boven 45°: controleer de rijlijn", "steering angle above 45°: check the path")));
    }
    msg + "."
}

/// Drive `v` along `path` (drawing units) and draw the body and wheel traces
/// and the vehicle every `o.every` metres.
pub fn swept_path(v: &Vehicle, path: &[[f64; 2]], o: &SweptOptions) -> Result<String, String> {
    let t = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let u = t.unit_factor;
    let metres: Vec<[f64; 2]> = path.iter().map(|p| [p[0] / u, p[1] / u]).collect();
    let s = simulate(v, &metres, o)?;
    let data = swept_data(v, &s);
    let out = swept_entities(v, &s, u, &data)?;
    let handles = bridge::add_entities_handles(&format!("{} {}", tr("Veegpad", "Swept path"), v.name_en), out)?;
    if let Some(h) = handles.first() {
        bridge::register_object_data(*h, data);
    }
    let (limit, _) = articulation_limit(v, o);
    crate::civiel::api::note("lengte_m", (s.length * 100.0).round() / 100.0);
    crate::civiel::api::note("stuurhoek_max_graden", (s.max_steer_deg * 10.0).round() / 10.0);
    crate::civiel::api::note("knikhoek_max_graden", (s.max_articulation_deg * 10.0).round() / 10.0);
    crate::civiel::api::note("knikhoek_grens_graden", (limit * 10.0).round() / 10.0);
    crate::civiel::api::note("scharen", v.units.len() > 1 && (s.reverse || o.limit.is_some()) && s.max_articulation_deg > limit);
    crate::civiel::api::note("achteruit", s.reverse);
    crate::civiel::api::note("elementen", handles.len());
    Ok(swept_message(v, &s, o))
}

/// `GIS_VEEGPAD <id> [interval]`: the path is a selected polyline, or picked
/// points ended with Enter.
pub struct SweptCmd {
    pub vehicle: &'static Vehicle,
    pub every: f64,
    pub path: Vec<[f64; 2]>,
    pub unit: f64,
    pub reverse: bool,
    pub limit: Option<f64>,
}

impl SweptCmd {
    fn options(&self) -> SweptOptions {
        SweptOptions { every: self.every, reverse: self.reverse, limit: self.limit }
    }

    fn run(&self) -> CommandStep {
        let (v, path, o) = (self.vehicle, self.path.clone(), self.options());
        bridge::spawn_command(move || match swept_path(v, &path, &o) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{}: {e}", tr("Veegpad", "Swept path"))),
        });
        CommandStep::Done
    }
}

impl InteractiveCommand for SweptCmd {
    fn prompt(&self) -> String {
        match (self.path.len(), self.reverse) {
            (0, false) => tr("Veegpad: beginpunt van de rijlijn (vooras)", "Swept path: start of the path (front axle)").into(),
            (0, true) => tr("Veegpad achteruit: beginpunt van de rijlijn (achteras van de laatste eenheid)", "Swept path in reverse: start of the path (rear axle of the last unit)").into(),
            (1, _) => tr("Veegpad: volgend punt", "Swept path: next point").into(),
            _ => tr("Veegpad: volgend punt [Enter = klaar]", "Swept path: next point [Enter = done]").into(),
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        self.path.push([pt[0], pt[1]]);
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        if self.path.len() >= 2 {
            self.run()
        } else {
            CommandStep::Cancel
        }
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        if self.path.is_empty() {
            return Vec::new();
        }
        let mut pts: Vec<[f64; 3]> = self.path.iter().map(|p| [p[0], p[1], 0.0]).collect();
        pts.push(pt);
        vec![PreviewWire::with_color(pts, [0.95, 0.35, 0.3, 1.0])]
    }
}

/// Start a swept path straight away on a known path (a selected polyline).
pub fn swept_now(cmd: &SweptCmd) {
    let _ = cmd.run();
}

/// The polyline or line of the selection, read by handle on a worker (the
/// whole drawing may be too large to fetch), in drawing units.
pub fn selected_line(handles: &[ocs_plugin_api::host::codec::Handle]) -> Result<Vec<[f64; 2]>, String> {
    let e = bridge::entities(handles)?.into_iter().flatten().next().ok_or_else(|| tr("de selectie is leeg", "the selection is empty").to_string())?;
    match e {
        EntityType::LwPolyline(l) if l.vertices.len() >= 2 => {
            let mut pts: Vec<[f64; 2]> = l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect();
            if l.is_closed {
                pts.push(pts[0]);
            }
            Ok(pts)
        }
        EntityType::Polyline3D(p) if p.vertices.len() >= 2 => Ok(p.vertices.iter().map(|v| [v.position.x, v.position.y]).collect()),
        EntityType::Line(l) => Ok(vec![[l.start.x, l.start.y], [l.end.x, l.end.y]]),
        _ => Err(tr("selecteer een lijn of polylijn", "select a line or polyline").into()),
    }
}

/// Points "x,y;x,y" in project coordinates → drawing units.
pub fn project_points(text: &str) -> Result<Vec<[f64; 2]>, String> {
    let u = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let site = bridge::site();
    text.split(';')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let v: Vec<f64> = t.split(',').map(|n| n.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| t!("geen punt x,y: '{t}'", "not a point x,y: '{t}'"))?;
            if v.len() < 2 {
                return Err(t!("geen punt x,y: '{t}'", "not a point x,y: '{t}'"));
            }
            Ok(match &site {
                Some(s) => s.to_drawing([v[0], v[1]], u),
                None => [v[0] * u, v[1] * u],
            })
        })
        .collect()
}

/// `GIS_VEEGPAD | GIS_SWEPT_PATH <id> [interval] [ACHTERUIT|REVERSE]
/// [KNIK=<graden>] [PAD=x,y;x,y;…]`: along `PAD=` (project coordinates), the
/// selected line, or picked points.
pub fn swept_command(host: &mut dyn ocs_plugin_api::host::HostApi, cmd: &str) {
    let words: Vec<&str> = cmd.split_whitespace().skip(1).collect();
    let mut id = None;
    let mut every = 10.0;
    let mut reverse = false;
    let mut limit = None;
    let mut path_arg = None;
    let mut bad = None;
    for w in &words {
        let up = w.to_ascii_uppercase();
        if let Some((k, v)) = w.split_once('=') {
            match k.to_ascii_uppercase().as_str() {
                "KNIK" | "LIMIT" => match v.replace(',', ".").parse::<f64>() {
                    Ok(x) => limit = Some(x),
                    Err(_) => bad = Some(t!("KNIK: geen getal '{v}'", "LIMIT: not a number '{v}'")),
                },
                "PAD" | "PATH" => path_arg = Some(v.to_string()),
                "INTERVAL" => every = v.replace(',', ".").parse::<f64>().unwrap_or(every),
                other => bad = Some(t!("onbekend argument {other}=", "unknown argument {other}=")),
            }
        } else if up == "ACHTERUIT" || up == "REVERSE" {
            reverse = true;
        } else if w.contains(',') && w.split(',').all(|n| n.trim().parse::<f64>().is_ok()) {
            // Coordinates answer the point prompts.
        } else if let Ok(x) = w.replace(',', ".").parse::<f64>() {
            every = x;
        } else if id.is_none() {
            id = Some(*w);
        }
    }
    let usage = || format!("GIS_SWEPT_PATH <{}> [{}] [ACHTERUIT|REVERSE] [KNIK=<{}>] [PAD=x,y;x,y]", tr("voertuig", "vehicle"), tr("interval in m", "interval in m"), tr("graden", "degrees"));
    if let Some(e) = bad {
        host.push_error(&format!("[Open Infra] {e} — {}", usage()));
        bridge::reply(false, &e);
        return;
    }
    let Some(v) = id.and_then(super::find) else {
        host.push_error(&format!("[Open Infra] {}", usage()));
        bridge::reply(false, &usage());
        let _ = crate::ui::open(crate::ui::Tab::Library);
        return;
    };
    let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let mut sc = SweptCmd { vehicle: v, every, path: Vec::new(), unit, reverse, limit };
    if let Some(text) = path_arg {
        match project_points(&text) {
            Ok(p) if p.len() >= 2 => {
                sc.path = p;
                swept_now(&sc);
            }
            Ok(_) => {
                host.push_error(&format!("[Open Infra] {}", tr("PAD= heeft minstens twee punten nodig", "PATH= needs at least two points")));
                bridge::reply(false, "PAD");
            }
            Err(e) => {
                host.push_error(&format!("[Open Infra] {e}"));
                bridge::reply(false, &e);
            }
        }
        return;
    }
    let sel = host.selection();
    if sel.len() == 1 {
        host.push_info(&format!("[Open Infra] {}", tr("Veegpad langs de geselecteerde lijn.", "Swept path along the selected line.")));
        bridge::spawn_command(move || match selected_line(&sel).and_then(|p| swept_path(sc.vehicle, &p, &sc.options())) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{}: {e}", tr("Veegpad", "Swept path"))),
        });
        return;
    }
    host.start_interactive(Box::new(sc));
}
