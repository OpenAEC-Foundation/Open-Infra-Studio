//! GIS engine output → Open CAD Studio (DWG) entities.

use ocs_plugin_api::host::codec::{
    entities::{RasterImage, TextHorizontalAlignment, TextVerticalAlignment},
    Circle, Color, EntityType, LwPolyline, Text, Vector2, Vector3,
};
use ocs_plugin_api::host::ExtendedDataRecord;

use crate::gis::raster::PlacedImage;
use crate::gis::site::Site;
use crate::gis::crs::XY;
use crate::gis::vector::Geometry;

fn on_layer(mut e: EntityType, layer: &str) -> EntityType {
    let c = e.common_mut();
    c.layer = layer.to_string();
    c.color = Color::ByLayer;
    e
}

/// Polylines, point markers and centred labels. `text_h` is in drawing units.
pub fn geometry(g: &Geometry, layer: &str, text_h: f64) -> Vec<EntityType> {
    let mut out = Vec::with_capacity(g.polylines.len() + g.points.len() + g.labels.len());
    for (pts, closed) in &g.polylines {
        let mut lw = LwPolyline::new();
        for p in pts {
            lw.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
        }
        lw.is_closed = *closed;
        out.push(on_layer(EntityType::LwPolyline(lw), layer));
    }
    for p in &g.points {
        out.push(on_layer(EntityType::Circle(Circle::from_center_radius(Vector3::new(p[0], p[1], 0.0), text_h * 0.25)), layer));
    }
    for (p, s) in &g.labels {
        let at = Vector3::new(p[0], p[1], 0.0);
        let mut t = Text::with_value(s.clone(), at).with_height(text_h);
        t.horizontal_alignment = TextHorizontalAlignment::Center;
        t.vertical_alignment = TextVerticalAlignment::Middle;
        t.alignment_point = Some(at);
        out.push(on_layer(EntityType::Text(t), layer));
    }
    out
}

/// IMAGE entities for downloaded map tiles (lower-left insertion, Y north).
pub fn images(site: &Site, placed: &[PlacedImage], layer: &str, unit_factor: f64, fade: u8) -> Vec<EntityType> {
    placed
        .iter()
        .map(|p| {
            let ll = site.to_drawing([p.bbox[0], p.bbox[1]], unit_factor);
            let ur = site.to_drawing([p.bbox[2], p.bbox[3]], unit_factor);
            let mut img = RasterImage::with_size(
                &p.path.to_string_lossy(),
                Vector3::new(ll[0], ll[1], 0.0),
                p.px[0] as f64,
                p.px[1] as f64,
                ur[0] - ll[0],
                ur[1] - ll[1],
            );
            img.fade = fade.min(90);
            // PNG maps (cadastre, Natura 2000, …) lie over the photo: let
            // their transparent background show what is beneath.
            img.set_transparent(p.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")));
            on_layer(EntityType::RasterImage(img), layer)
        })
        .collect()
}

/// DWG layer names: no `<>/\":;?*|,=` characters, upper case like the rest.
pub fn layer_name(prefix: &str, name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if "<>/\\\":;?*|,=`".contains(c) { '-' } else { c })
        .collect();
    let mut s = format!("{prefix}-{clean}").to_uppercase();
    s.truncate(60);
    s
}

// ── NLCS output: one entity per object part, ByLayer, with object data ──────

fn styled(mut e: EntityType, layer: &str, ltscale: f64, data: Option<&ExtendedDataRecord>) -> EntityType {
    let c = e.common_mut();
    c.layer = layer.to_string();
    c.color = Color::ByLayer;
    c.linetype_scale = ltscale;
    if let Some(r) = data {
        c.extended_data.add_record(r.clone());
    }
    e
}

/// Polylines and rings. `ltscale` turns the NLCS paper-millimetre linetypes
/// into drawing units (scale / 1000 × units per metre).
pub fn polylines(lines: &[(Vec<XY>, bool)], layer: &str, ltscale: f64, data: Option<&ExtendedDataRecord>) -> Vec<EntityType> {
    lines
        .iter()
        .map(|(pts, closed)| {
            let mut lw = LwPolyline::new();
            for p in pts {
                lw.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
            }
            lw.is_closed = *closed;
            lw.plinegen = !*closed;
            styled(EntityType::LwPolyline(lw), layer, ltscale, data)
        })
        .collect()
}

/// Point objects as small circles (NLCS element S without its block).
pub fn markers(points: &[XY], layer: &str, radius: f64, data: Option<&ExtendedDataRecord>) -> Vec<EntityType> {
    points
        .iter()
        .map(|p| styled(EntityType::Circle(Circle::from_center_radius(Vector3::new(p[0], p[1], 0.0), radius)), layer, 1.0, data))
        .collect()
}

/// A centred, rotated text (`rotation` in radians, counter-clockwise).
pub fn text(at: XY, value: &str, height: f64, rotation: f64, layer: &str, data: Option<&ExtendedDataRecord>) -> EntityType {
    let p = Vector3::new(at[0], at[1], 0.0);
    let mut t = Text::with_value(value.to_string(), p).with_height(height);
    t.rotation = rotation;
    t.horizontal_alignment = TextHorizontalAlignment::Center;
    t.vertical_alignment = TextVerticalAlignment::Middle;
    t.alignment_point = Some(p);
    styled(EntityType::Text(t), layer, 1.0, data)
}

/// Reading direction for a text along a segment: never upside down.
pub fn readable(angle: f64) -> f64 {
    let a = angle.rem_euclid(std::f64::consts::TAU);
    if a > std::f64::consts::FRAC_PI_2 && a <= 3.0 * std::f64::consts::FRAC_PI_2 {
        a - std::f64::consts::PI
    } else {
        a
    }
}

/// The middle of a line's longest segment and that segment's direction.
pub fn label_spot(pts: &[XY]) -> Option<(XY, f64)> {
    pts.windows(2)
        .map(|w| (w[0], w[1], (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .filter(|s| s.2 > 0.0)
        .map(|(a, b, _)| ([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], (b[1] - a[1]).atan2(b[0] - a[0])))
}
