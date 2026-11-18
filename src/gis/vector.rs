//! Vector data: GeoJSON features → drawing geometry in the project CRS,
//! clipped to the import extent.

use serde_json::Value;

use super::crs::{Transformer, XY};
use super::site::Site;

#[derive(Clone, Debug, Default)]
pub struct Feature {
    pub lines: Vec<(Vec<XY>, bool)>,
    pub points: Vec<XY>,
    pub props: serde_json::Map<String, Value>,
}

fn xy(v: &Value) -> Option<XY> {
    let a = v.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

fn ring(v: &Value) -> Vec<XY> {
    v.as_array().map(|a| a.iter().filter_map(xy).collect()).unwrap_or_default()
}

fn add_geometry(g: &Value, f: &mut Feature) {
    let c = &g["coordinates"];
    let list = || c.as_array().cloned().unwrap_or_default();
    match g["type"].as_str().unwrap_or("") {
        "Point" => f.points.extend(xy(c)),
        "MultiPoint" => f.points.extend(list().iter().filter_map(xy)),
        "LineString" => f.lines.push((ring(c), false)),
        "MultiLineString" => f.lines.extend(list().iter().map(|l| (ring(l), false))),
        "Polygon" => f.lines.extend(list().iter().map(|r| (ring(r), true))),
        "MultiPolygon" => {
            for p in list() {
                f.lines.extend(p.as_array().cloned().unwrap_or_default().iter().map(|r| (ring(r), true)));
            }
        }
        "GeometryCollection" => {
            for sub in g["geometries"].as_array().cloned().unwrap_or_default() {
                add_geometry(&sub, f);
            }
        }
        _ => {}
    }
}

pub fn parse_geojson(v: &Value) -> Vec<Feature> {
    v["features"]
        .as_array()
        .map(|fs| {
            fs.iter()
                .map(|f| {
                    let mut out = Feature { props: f["properties"].as_object().cloned().unwrap_or_default(), ..Default::default() };
                    if f["geometry"].is_object() {
                        add_geometry(&f["geometry"], &mut out);
                    }
                    out
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sample(features: &[Feature]) -> Vec<XY> {
    let mut out = Vec::new();
    for f in features {
        out.extend(f.lines.iter().filter_map(|(l, _)| l.first().copied()));
        out.extend(f.points.iter().copied());
        if out.len() > 200 {
            break;
        }
    }
    out
}

/// Data CRS → project CRS. Some servers answer EPSG:4326 in lat/lon order;
/// pick whichever axis order lands near the site.
pub fn data_to_site(site: &Site, data_crs: &str, features: &[Feature]) -> Result<Box<dyn Fn(XY) -> Option<XY>>, String> {
    data_to(site, data_crs, &site.crs, features)
}

/// Data CRS → any CRS, with the same axis-order detection (judged at the site).
pub fn data_to(site: &Site, data_crs: &str, to: &str, features: &[Feature]) -> Result<Box<dyn Fn(XY) -> Option<XY>>, String> {
    let t = Transformer::new(data_crs, &site.crs)?;
    let out = Transformer::new(data_crs, to)?;
    let pts = sample(features);
    let near = |swap: bool| {
        pts.iter()
            .filter(|p| {
                let q = if swap { [p[1], p[0]] } else { **p };
                t.apply(q).is_ok_and(|r| (r[0] - site.origin_e).hypot(r[1] - site.origin_n) < 50_000.0)
            })
            .count()
    };
    let swap = !pts.is_empty() && near(true) > near(false);
    Ok(Box::new(move |p: XY| out.apply(if swap { [p[1], p[0]] } else { p }).ok()))
}

/// Feature outlines in lon/lat, for the workbench map preview.
pub fn preview_lines(site: &Site, data_crs: &str, features: &[Feature]) -> Result<Vec<Vec<XY>>, String> {
    let f = data_to(site, data_crs, "EPSG:4326", features)?;
    let mut out = Vec::new();
    for feat in features {
        for (line, closed) in &feat.lines {
            let mut pts: Vec<XY> = line.iter().filter_map(|p| f(*p)).collect();
            if *closed && pts.len() > 2 && pts.first() != pts.last() {
                pts.push(pts[0]);
            }
            if pts.len() > 1 {
                out.push(pts);
            }
        }
        for p in &feat.points {
            if let Some(q) = f(*p) {
                let d = 0.00002;
                out.push(vec![[q[0] - d, q[1]], [q[0] + d, q[1]]]);
                out.push(vec![[q[0], q[1] - d], [q[0], q[1] + d]]);
            }
        }
    }
    Ok(out)
}

pub type Rect = [f64; 4];

pub fn inside(p: XY, r: Rect) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

/// Liang–Barsky: the visible pieces of a polyline inside `r`.
pub fn clip_line(pts: &[XY], r: Rect) -> Vec<Vec<XY>> {
    let mut out = Vec::new();
    let mut cur: Vec<XY> = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut visible = true;
        for (p, q) in [(-dx, a[0] - r[0]), (dx, r[2] - a[0]), (-dy, a[1] - r[1]), (dy, r[3] - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    visible = false;
                    break;
                }
                continue;
            }
            let t = q / p;
            if p < 0.0 {
                if t > t1 { visible = false; break; }
                t0 = t0.max(t);
            } else {
                if t < t0 { visible = false; break; }
                t1 = t1.min(t);
            }
        }
        if !visible {
            if cur.len() > 1 { out.push(std::mem::take(&mut cur)); } else { cur.clear(); }
            continue;
        }
        let s = [a[0] + t0 * dx, a[1] + t0 * dy];
        let e = [a[0] + t1 * dx, a[1] + t1 * dy];
        if cur.is_empty() {
            cur.push(s);
        }
        cur.push(e);
        if t1 < 1.0 {
            if cur.len() > 1 { out.push(std::mem::take(&mut cur)); } else { cur.clear(); }
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// Sutherland–Hodgman: a closed ring clipped to `r`.
pub fn clip_ring(pts: &[XY], r: Rect) -> Vec<XY> {
    type Edge = (fn(XY, Rect) -> bool, fn(XY, XY, Rect) -> XY);
    let edges: [Edge; 4] = [
        (|p, r| p[0] >= r[0], |a, b, r| [r[0], a[1] + (b[1] - a[1]) * (r[0] - a[0]) / (b[0] - a[0])]),
        (|p, r| p[0] <= r[2], |a, b, r| [r[2], a[1] + (b[1] - a[1]) * (r[2] - a[0]) / (b[0] - a[0])]),
        (|p, r| p[1] >= r[1], |a, b, r| [a[0] + (b[0] - a[0]) * (r[1] - a[1]) / (b[1] - a[1]), r[1]]),
        (|p, r| p[1] <= r[3], |a, b, r| [a[0] + (b[0] - a[0]) * (r[3] - a[1]) / (b[1] - a[1]), r[3]]),
    ];
    let mut ring = pts.to_vec();
    for (is_in, cut) in edges {
        if ring.is_empty() {
            break;
        }
        let mut next = Vec::with_capacity(ring.len() + 4);
        for i in 0..ring.len() {
            let a = ring[(i + ring.len() - 1) % ring.len()];
            let b = ring[i];
            match (is_in(a, r), is_in(b, r)) {
                (_, true) => {
                    if !is_in(a, r) {
                        next.push(cut(a, b, r));
                    }
                    next.push(b);
                }
                (true, false) => next.push(cut(a, b, r)),
                (false, false) => {}
            }
        }
        ring = next;
    }
    ring
}

pub(crate) fn prop_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// One feature in drawing coordinates: its (clipped) lines and rings, and
/// its points inside the site.
pub fn feature_to_drawing(site: &Site, f: &Feature, to_site: &dyn Fn(XY) -> Option<XY>, unit_factor: f64, clip: bool) -> (Vec<(Vec<XY>, bool)>, Vec<XY>) {
    let rect = site.drawing_rect(unit_factor);
    let conv = |p: XY| to_site(p).map(|q| site.to_drawing(q, unit_factor));
    let mut lines = Vec::new();
    for (raw, closed) in &f.lines {
        let mut pts: Vec<XY> = raw.iter().filter_map(|p| conv(*p)).collect();
        if *closed && pts.len() > 2 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() < 2 {
            continue;
        }
        if !clip {
            lines.push((pts, *closed));
        } else if *closed {
            let r = clip_ring(&pts, rect);
            if r.len() > 2 {
                lines.push((r, true));
            }
        } else {
            lines.extend(clip_line(&pts, rect).into_iter().map(|l| (l, false)));
        }
    }
    let points = f.points.iter().filter_map(|p| conv(*p)).filter(|q| !clip || inside(*q, rect)).collect();
    (lines, points)
}

/// A point inside a ring for its label: the centroid when it lies inside,
/// otherwise the middle of the widest inside span through it.
pub fn label_point(ring: &[XY]) -> Option<XY> {
    if ring.len() < 3 {
        return None;
    }
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    // Relative to the first vertex, for precision with RD coordinates.
    let o = ring[0];
    for i in 0..ring.len() {
        let p = [ring[i][0] - o[0], ring[i][1] - o[1]];
        let q = [ring[(i + 1) % ring.len()][0] - o[0], ring[(i + 1) % ring.len()][1] - o[1]];
        let c = p[0] * q[1] - q[0] * p[1];
        a += c;
        cx += (p[0] + q[0]) * c;
        cy += (p[1] + q[1]) * c;
    }
    if a.abs() < 1e-12 {
        return None;
    }
    let c = [cx / (3.0 * a) + o[0], cy / (3.0 * a) + o[1]];
    if point_in_ring(c, ring) {
        return Some(c);
    }
    let mut xs: Vec<f64> = Vec::new();
    for i in 0..ring.len() {
        let (p, q) = (ring[i], ring[(i + 1) % ring.len()]);
        if (p[1] > c[1]) != (q[1] > c[1]) {
            xs.push(p[0] + (c[1] - p[1]) / (q[1] - p[1]) * (q[0] - p[0]));
        }
    }
    xs.sort_by(f64::total_cmp);
    xs.chunks(2).filter(|w| w.len() == 2).max_by(|a, b| (a[1] - a[0]).total_cmp(&(b[1] - b[0]))).map(|w| [(w[0] + w[1]) / 2.0, c[1]])
}

pub fn point_in_ring(p: XY, ring: &[XY]) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Attribute names present on the features (for the label picker).
pub fn property_names(features: &[Feature]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for f in features.iter().take(50) {
        for k in f.props.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = [0.0, 0.0, 10.0, 10.0];

    #[test]
    fn line_crossing_is_cut_into_pieces() {
        let pieces = clip_line(&[[-5.0, 5.0], [5.0, 5.0], [15.0, 5.0], [15.0, 8.0], [5.0, 8.0]], R);
        assert_eq!(pieces, vec![vec![[0.0, 5.0], [5.0, 5.0], [10.0, 5.0]], vec![[10.0, 8.0], [5.0, 8.0]]]);
    }

    #[test]
    fn ring_is_clipped_to_rect() {
        let r = clip_ring(&[[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]], R);
        let mut xs: Vec<_> = r.iter().map(|p| (p[0] as i32, p[1] as i32)).collect();
        xs.sort();
        xs.dedup();
        assert_eq!(xs, vec![(0, 0), (0, 5), (5, 0), (5, 5)]);
    }

    #[test]
    fn geojson_polygon_and_point() {
        let v: Value = serde_json::from_str(r#"{"features":[{"properties":{"n":1},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}},{"properties":{},"geometry":{"type":"Point","coordinates":[2,3]}}]}"#).unwrap();
        let f = parse_geojson(&v);
        assert_eq!(f.len(), 2);
        assert!(f[0].lines[0].1);
        assert_eq!(f[1].points, vec![[2.0, 3.0]]);
    }
}
