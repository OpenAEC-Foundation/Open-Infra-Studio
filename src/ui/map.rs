//! Slippy map for the workbench, in the role of the FreeCAD GIS workbench's
//! map pane: background tiles (OSM, PDOK, satellite), the project location
//! and bounding box, and previews of what an import will bring in.
//!
//! Drag pans, the wheel zooms around the cursor, a click picks a location and
//! a drag in "area" mode draws a new bounding box.

use crate::lang::tr;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};

use crate::gis::http;

pub type LonLat = [f64; 2];

pub struct Basemap {
    pub name: (&'static str, &'static str),
    /// Tile template with {z}, {x}, {y}.
    pub url: &'static str,
    pub max_zoom: u8,
    pub attribution: (&'static str, &'static str),
}

pub const BASEMAPS: &[Basemap] = &[
    Basemap { name: ("OpenStreetMap", "OpenStreetMap"), url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png", max_zoom: 19, attribution: ("© OpenStreetMap-bijdragers", "© OpenStreetMap contributors") },
    Basemap { name: ("PDOK BRT-achtergrondkaart (NL)", "PDOK BRT reference map (NL)"), url: "https://service.pdok.nl/brt/achtergrondkaart/wmts/v2_0/standaard/EPSG:3857/{z}/{x}/{y}.png", max_zoom: 19, attribution: ("Kadaster / PDOK", "Kadaster / PDOK") },
    Basemap { name: ("PDOK BRT grijs (NL)", "PDOK BRT greyscale (NL)"), url: "https://service.pdok.nl/brt/achtergrondkaart/wmts/v2_0/grijs/EPSG:3857/{z}/{x}/{y}.png", max_zoom: 19, attribution: ("Kadaster / PDOK", "Kadaster / PDOK") },
    Basemap { name: ("PDOK luchtfoto (NL)", "PDOK aerial imagery (NL)"), url: "https://service.pdok.nl/hwh/luchtfotorgb/wmts/v1_0/Actueel_orthoHR/EPSG:3857/{z}/{x}/{y}.jpeg", max_zoom: 21, attribution: ("Beeldmateriaal.nl / PDOK", "Beeldmateriaal.nl / PDOK") },
    Basemap { name: ("Sentinel-2 satelliet (wereld)", "Sentinel-2 satellite (worldwide)"), url: "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2023_3857/default/g/{z}/{y}/{x}.jpg", max_zoom: 16, attribution: ("Sentinel-2 cloudless by EOX", "Sentinel-2 cloudless by EOX") },
];

const TILE: f64 = 256.0;
const MAX_INFLIGHT: usize = 6;
const MAX_CACHE: usize = 600;

/// lon/lat → world pixels at zoom `z` (Web Mercator).
pub fn project(p: LonLat, z: f64) -> [f64; 2] {
    let n = TILE * 2f64.powf(z);
    let lat = p[1].clamp(-85.05112878, 85.05112878).to_radians();
    [(p[0] + 180.0) / 360.0 * n, (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n]
}

pub fn unproject(w: [f64; 2], z: f64) -> LonLat {
    let n = TILE * 2f64.powf(z);
    let lon = w[0] / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * w[1] / n)).sinh().atan().to_degrees();
    [lon, lat]
}

type Key = (usize, u8, u32, u32);

enum Slot {
    Loading,
    Ready(ColorImage),
    Failed,
}

/// Things drawn on top of the background.
#[derive(Default)]
pub struct Overlay<'a> {
    pub marker: Option<LonLat>,
    /// Closed ring (lon/lat), e.g. the import area.
    pub bbox: Option<Vec<LonLat>>,
    /// Preview raster with its corners (lon/lat): upper-left, upper-right, lower-right, lower-left.
    pub image: Option<(&'a TextureHandle, [LonLat; 4])>,
    pub lines: &'a [(Vec<LonLat>, Color32)],
}

pub enum MapEvent {
    Clicked(LonLat),
    RectDrawn(LonLat, LonLat),
}

pub struct MapView {
    pub center: LonLat,
    pub zoom: f64,
    pub basemap: usize,
    /// Drag draws a rectangle instead of panning.
    pub area_mode: bool,
    /// Overlays toggled in the layer switcher.
    pub show_bbox: bool,
    pub show_preview: bool,
    /// Layer switcher pinned open (it also opens while hovered).
    layers_open: bool,
    layer_panel: Option<Rect>,
    slots: Arc<Mutex<HashMap<Key, Slot>>>,
    textures: HashMap<Key, TextureHandle>,
    inflight: Arc<AtomicUsize>,
    drag_start: Option<Pos2>,
    hover: Option<LonLat>,
}

impl MapView {
    pub fn new(center: LonLat, zoom: f64) -> Self {
        Self {
            center,
            zoom,
            basemap: 0,
            area_mode: false,
            slots: Arc::default(),
            textures: HashMap::new(),
            inflight: Arc::default(),
            show_bbox: true,
            show_preview: true,
            layers_open: false,
            layer_panel: None,
            drag_start: None,
            hover: None,
        }
    }

    pub(super) fn reset_window(&mut self) {
        self.textures.clear();
        self.drag_start = None;
        self.hover = None;
    }

    pub fn hover(&self) -> Option<LonLat> {
        self.hover
    }

    /// Centre and zoom so that the lon/lat ring fills about 60 % of the view.
    pub fn fit(&mut self, ring: &[LonLat], view: Vec2) {
        if ring.is_empty() {
            return;
        }
        let (mut a, mut b) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in ring {
            let w = project(*p, 0.0);
            a = [a[0].min(w[0]), a[1].min(w[1])];
            b = [b[0].max(w[0]), b[1].max(w[1])];
        }
        let span = (b[0] - a[0]).max(b[1] - a[1]).max(1e-9);
        let target = (view.x.min(view.y) as f64 * 0.6).max(64.0);
        self.zoom = (target / span).log2().clamp(1.0, BASEMAPS[self.basemap].max_zoom as f64);
        self.center = unproject([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], 0.0);
    }

    fn request(&self, key: Key) {
        let mut slots = self.slots.lock().unwrap();
        if slots.contains_key(&key) || self.inflight.load(Ordering::Relaxed) >= MAX_INFLIGHT {
            return;
        }
        slots.insert(key, Slot::Loading);
        drop(slots);
        self.inflight.fetch_add(1, Ordering::Relaxed);
        let (slots, inflight) = (self.slots.clone(), self.inflight.clone());
        let url = BASEMAPS[key.0].url.replace("{z}", &key.1.to_string()).replace("{x}", &key.2.to_string()).replace("{y}", &key.3.to_string());
        std::thread::spawn(move || {
            let slot = http::get_image(&url)
                .ok()
                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                .map(|img| {
                    let rgba = img.to_rgba8();
                    Slot::Ready(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
                })
                .unwrap_or(Slot::Failed);
            slots.lock().unwrap().insert(key, slot);
            inflight.fetch_sub(1, Ordering::Relaxed);
            super::repaint();
        });
    }

    fn texture(&mut self, ctx: &egui::Context, key: Key) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(&key) {
            return Some(t.clone());
        }
        let mut slots = self.slots.lock().unwrap();
        match slots.get(&key) {
            Some(Slot::Ready(_)) => {
                // The texture takes over; an evicted tile is simply fetched again.
                let Some(Slot::Ready(img)) = slots.remove(&key) else { unreachable!() };
                drop(slots);
                let t = ctx.load_texture(format!("tile-{key:?}"), img, egui::TextureOptions::LINEAR);
                self.textures.insert(key, t.clone());
                Some(t)
            }
            Some(_) => None,
            None => {
                drop(slots);
                self.request(key);
                None
            }
        }
    }

    fn zoom_control(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect, max_zoom: f64) {
        let bar = Rect::from_min_size(rect.min + Vec2::new(10.0, 10.0), Vec2::new(30.0, 60.0));
        painter.rect(bar, 4.0, Color32::WHITE, Stroke::new(2.0, Color32::from_black_alpha(50)), egui::StrokeKind::Outside);
        painter.line_segment([bar.left_center(), bar.right_center()], Stroke::new(1.0, Color32::from_gray(204)));
        let plus = Rect::from_min_size(bar.min, Vec2::splat(30.0));
        for (r, label, dz, id) in [(plus, "+", 1.0, "zoom_in"), (plus.translate(Vec2::new(0.0, 30.0)), "−", -1.0, "zoom_out")] {
            let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
            if resp.hovered() {
                painter.rect_filled(r.shrink(1.5), 3.0, Color32::from_gray(244));
            }
            painter.text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(20.0), Color32::from_gray(51));
            if resp.clicked() {
                self.zoom = (self.zoom + dz).clamp(1.0, max_zoom);
            }
        }
    }

    /// The Leaflet layers control: a layers button that unfolds (hover or
    /// click) into a white panel with the background maps and overlays.
    fn layer_control(&mut self, ui: &mut egui::Ui, painter: &egui::Painter, rect: Rect) {
        let ink = Color32::from_gray(51);
        let shadow = Stroke::new(2.0, Color32::from_black_alpha(50));
        let button = Rect::from_min_size(egui::pos2(rect.right() - 46.0, rect.top() + 10.0), Vec2::splat(36.0));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over = |r: Rect| pointer.is_some_and(|p| r.contains(p));
        let expanded = self.layers_open || over(button) || self.layer_panel.is_some_and(over);
        if !expanded {
            self.layer_panel = None;
            painter.rect(button, 4.0, Color32::WHITE, shadow, egui::StrokeKind::Outside);
            // Layers glyph: three stacked diamonds.
            let c = button.center();
            for (dy, fill) in [(5.0, Color32::from_gray(150)), (0.0, Color32::from_gray(110)), (-5.0, Color32::from_gray(70))] {
                let pts = vec![c + Vec2::new(0.0, dy - 6.0), c + Vec2::new(11.0, dy), c + Vec2::new(0.0, dy + 6.0), c + Vec2::new(-11.0, dy)];
                painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, Color32::WHITE)));
            }
            if ui.interact(button, ui.scope_id().with("layers_button"), Sense::click()).clicked() {
                self.layers_open = true;
            }
            return;
        }

        const ROW: f32 = 23.0;
        let overlays: [(&str, bool); 2] = [(tr("Importgebied (begrenzing)", "Import area (bounds)"), self.show_bbox), (tr("Voorbeeld van de import", "Import preview"), self.show_preview)];
        let width = 262.0;
        let height = 14.0 + BASEMAPS.len() as f32 * ROW + 13.0 + overlays.len() as f32 * ROW + 6.0;
        let panel = Rect::from_min_size(egui::pos2(rect.right() - 10.0 - width, rect.top() + 10.0), Vec2::new(width, height));
        self.layer_panel = Some(panel);
        painter.rect(panel, 5.0, Color32::WHITE, shadow, egui::StrokeKind::Outside);
        // Swallow drags and clicks so the map underneath does not react.
        ui.interact(panel, ui.scope_id().with("layers_panel"), Sense::click_and_drag());

        let font = egui::FontId::proportional(13.0);
        let mut y = panel.top() + 7.0;
        for (i, b) in BASEMAPS.iter().enumerate() {
            let row = Rect::from_min_size(egui::pos2(panel.left() + 4.0, y), Vec2::new(width - 8.0, ROW));
            let resp = ui.interact(row, ui.scope_id().with(("basemap", i)), Sense::click());
            if resp.hovered() {
                painter.rect_filled(row, 3.0, Color32::from_gray(240));
            }
            let dot = egui::pos2(row.left() + 12.0, row.center().y);
            painter.circle(dot, 6.5, Color32::WHITE, Stroke::new(1.3, Color32::from_gray(120)));
            if self.basemap == i {
                painter.circle_filled(dot, 3.8, Color32::from_rgb(0, 120, 168));
            }
            painter.text(egui::pos2(row.left() + 26.0, row.center().y), egui::Align2::LEFT_CENTER, tr(b.name.0, b.name.1), font.clone(), ink);
            if resp.clicked() {
                self.basemap = i;
            }
            y += ROW;
        }
        y += 6.0;
        painter.line_segment([egui::pos2(panel.left() + 8.0, y), egui::pos2(panel.right() - 8.0, y)], Stroke::new(1.0, Color32::from_gray(221)));
        y += 7.0;
        for (i, (label, on)) in overlays.iter().enumerate() {
            let row = Rect::from_min_size(egui::pos2(panel.left() + 4.0, y), Vec2::new(width - 8.0, ROW));
            let resp = ui.interact(row, ui.scope_id().with(("overlay", i)), Sense::click());
            if resp.hovered() {
                painter.rect_filled(row, 3.0, Color32::from_gray(240));
            }
            let b = Rect::from_center_size(egui::pos2(row.left() + 12.0, row.center().y), Vec2::splat(12.0));
            painter.rect(b, 2.0, if *on { Color32::from_rgb(0, 120, 168) } else { Color32::WHITE }, Stroke::new(1.3, Color32::from_gray(120)), egui::StrokeKind::Inside);
            if *on {
                painter.add(egui::Shape::line(vec![b.left_center() + Vec2::new(2.5, 0.0), b.center_bottom() + Vec2::new(-1.0, -3.0), b.right_top() + Vec2::new(-2.5, 3.0)], Stroke::new(1.8, Color32::WHITE)));
            }
            painter.text(egui::pos2(row.left() + 26.0, row.center().y), egui::Align2::LEFT_CENTER, *label, font.clone(), ink);
            if resp.clicked() {
                if i == 0 { self.show_bbox = !self.show_bbox } else { self.show_preview = !self.show_preview }
            }
            y += ROW;
        }
        // A pinned panel closes with a click elsewhere.
        if self.layers_open && ui.input(|i| i.pointer.any_click()) && !over(panel) && !over(button) {
            self.layers_open = false;
        }
    }

    /// Draw the map in a `height`-tall strip and handle input.
    pub fn show(&mut self, ui: &mut egui::Ui, height: f32, overlay: Overlay<'_>) -> Option<MapEvent> {
        let ctx = ui.ctx().clone();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(225));

        let bm = &BASEMAPS[self.basemap];
        self.zoom = self.zoom.clamp(1.0, bm.max_zoom as f64);
        let tz = self.zoom.round().clamp(0.0, bm.max_zoom as f64) as u8;
        let scale = 2f64.powf(self.zoom - tz as f64);
        let c = project(self.center, tz as f64);
        let mid = rect.center();
        let to_screen = |w: [f64; 2]| Pos2::new(mid.x + ((w[0] - c[0]) * scale) as f32, mid.y + ((w[1] - c[1]) * scale) as f32);
        let ll_to_screen = |p: LonLat| to_screen(project(p, tz as f64));
        let screen_to_ll = |s: Pos2| unproject([c[0] + (s.x - mid.x) as f64 / scale, c[1] + (s.y - mid.y) as f64 / scale], tz as f64);

        // Background tiles.
        let half = [rect.width() as f64 / 2.0 / scale, rect.height() as f64 / 2.0 / scale];
        let n = 1i64 << tz;
        let (x0, x1) = (((c[0] - half[0]) / TILE).floor() as i64, ((c[0] + half[0]) / TILE).floor() as i64);
        let (y0, y1) = (((c[1] - half[1]) / TILE).floor().max(0.0) as i64, ((c[1] + half[1]) / TILE).floor().min((n - 1) as f64) as i64);
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let wx = tx.rem_euclid(n) as u32;
                let key = (self.basemap, tz, wx, ty as u32);
                let tl = to_screen([tx as f64 * TILE, ty as f64 * TILE]);
                let br = to_screen([(tx + 1) as f64 * TILE, (ty + 1) as f64 * TILE]);
                if let Some(t) = self.texture(&ctx, key) {
                    painter.image(t.id(), Rect::from_min_max(tl, br), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
            }
        }

        // Preview raster.
        if let (Some((tex, corners)), true) = (&overlay.image, self.show_preview) {
            let mut mesh = egui::Mesh::with_texture(tex.id());
            let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
            for (p, uv) in corners.iter().zip(uvs) {
                mesh.vertices.push(egui::epaint::Vertex { pos: ll_to_screen(*p), uv, color: Color32::from_white_alpha(235) });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
        // Vector preview.
        for (line, color) in overlay.lines.iter().filter(|_| self.show_preview) {
            if line.len() > 1 {
                painter.add(egui::Shape::line(line.iter().map(|p| ll_to_screen(*p)).collect(), Stroke::new(1.4, *color)));
            }
        }
        // Import area.
        if let (Some(ring), true) = (&overlay.bbox, self.show_bbox) {
            let pts: Vec<Pos2> = ring.iter().map(|p| ll_to_screen(*p)).collect();
            painter.add(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(40, 120, 255, 28), Stroke::NONE));
            painter.add(egui::Shape::closed_line(pts, Stroke::new(2.0, Color32::from_rgb(40, 110, 240))));
        }
        // Location marker (pin).
        if let Some(m) = overlay.marker {
            let p = ll_to_screen(m);
            let red = Color32::from_rgb(220, 50, 50);
            painter.add(egui::Shape::convex_polygon(vec![p, p + Vec2::new(-7.0, -14.0), p + Vec2::new(7.0, -14.0)], red, Stroke::NONE));
            painter.circle(p + Vec2::new(0.0, -17.0), 8.0, red, Stroke::new(1.5, Color32::WHITE));
            painter.circle_filled(p + Vec2::new(0.0, -17.0), 3.0, Color32::WHITE);
        }

        // Input.
        let mut event = None;
        self.hover = resp.hover_pos().map(screen_to_ll);
        let area = self.area_mode || ui.input(|i| i.modifiers.shift);
        if resp.drag_started() {
            self.drag_start = resp.interact_pointer_pos();
        }
        if area {
            if let (Some(a), Some(b)) = (self.drag_start, resp.interact_pointer_pos()) {
                if resp.dragged() {
                    let r = Rect::from_two_pos(a, b);
                    painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(240, 140, 30, 40), Stroke::new(2.0, Color32::from_rgb(240, 140, 30)), egui::StrokeKind::Middle);
                }
                if resp.drag_stopped() && (a - b).length() > 6.0 {
                    event = Some(MapEvent::RectDrawn(screen_to_ll(a), screen_to_ll(b)));
                }
            }
        } else if resp.dragged() {
            let d = resp.drag_delta();
            self.center = unproject([c[0] - d.x as f64 / scale, c[1] - d.y as f64 / scale], tz as f64);
        }
        if resp.drag_stopped() {
            self.drag_start = None;
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                event = Some(MapEvent::Clicked(screen_to_ll(p)));
            }
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            // The wheel zooms the map; it must not also scroll the page.
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            if scroll != 0.0 {
                if let Some(cur) = resp.hover_pos() {
                    let before = screen_to_ll(cur);
                    self.zoom = (self.zoom + scroll as f64 / 200.0).clamp(1.0, bm.max_zoom as f64);
                    // Keep the point under the cursor in place.
                    let z = self.zoom;
                    let wb = project(before, z);
                    let off = [(cur.x - mid.x) as f64, (cur.y - mid.y) as f64];
                    self.center = unproject([wb[0] - off[0], wb[1] - off[1]], z);
                }
            }
        }

        // Leaflet-style controls: zoom bar top-left, layer switcher top-right.
        self.zoom_control(ui, &painter, rect, bm.max_zoom as f64);
        self.layer_control(ui, &painter, rect);
        let attr = format!("{} · {}", tr(bm.attribution.0, bm.attribution.1), if area { tr("sleep om het gebied te tekenen", "drag to draw the area") } else { tr("klik = locatie · shift+sleep = gebied", "click for location · Shift+drag for area") });
        let galley = painter.layout_no_wrap(attr, egui::FontId::proportional(11.0), Color32::from_gray(30));
        let at = rect.right_bottom() - galley.size() - Vec2::new(6.0, 4.0);
        painter.rect_filled(Rect::from_min_size(at - Vec2::new(4.0, 2.0), galley.size() + Vec2::new(8.0, 4.0)), 3.0, Color32::from_white_alpha(200));
        painter.galley(at, galley, Color32::from_gray(30));

        // Keep the cache bounded.
        if self.textures.len() > MAX_CACHE {
            self.textures.retain(|k, _| k.1 == tz && k.0 == self.basemap);
            self.slots.lock().unwrap().retain(|k, s| matches!(s, Slot::Loading) || (k.1 == tz && k.0 == self.basemap));
        }
        if self.inflight.load(Ordering::Relaxed) > 0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        event
    }
}
