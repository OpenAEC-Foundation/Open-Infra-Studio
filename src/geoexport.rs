//! The drawing as geodata: every element with its layer, its NLCS meaning and
//! its object data (the `OPENINFRA` XDATA) as attributes, to GeoJSON (WGS 84,
//! as RFC 7946 asks) or a GeoPackage (in the project CRS, one table each for
//! points, lines and areas). Host-independent: it takes a document, the
//! project location and the drawing units.
//!
//! - lines, polylines (bulges as arc points), arcs, ellipses and splines
//!   become line strings; closed ones (a BGT outline, a circle) areas;
//! - hatches and solids become areas (outer boundary and islands);
//! - points, block references (symbols) and texts become points; a text
//!   keeps its value as `tekst`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::f64::consts::PI;
use std::path::Path;

use ocs_plugin_api::host::codec::entities::BoundaryEdge;
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use crate::gis::crs::{self, Transformer};
use crate::gis::site::Site;
use crate::lang::tr;

pub type P3 = [f64; 3];

#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Point(P3),
    Line(Vec<P3>),
    /// Outer ring first, then islands; rings closed (first = last).
    Polygon(Vec<Vec<P3>>),
}

#[derive(Clone, Debug)]
pub struct GeoFeature {
    pub geom: Geom,
    pub props: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Only these elements.
    pub selection: Option<HashSet<Handle>>,
    /// Only layers whose name contains this (case-insensitive).
    pub layers: Option<String>,
    /// Also sheet and annotation layers (AL, ZZ); off by default.
    pub annotation: bool,
}

/// Points of a polyline with bulges, the arcs in steps of at most 5°.
fn bulged(pts: &[([f64; 3], f64)], closed: bool) -> Vec<P3> {
    let n = pts.len();
    let mut out = Vec::new();
    if n == 0 {
        return out;
    }
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let (a, bulge) = pts[i];
        let b = pts[(i + 1) % n].0;
        out.push(a);
        if bulge.abs() > 1e-12 {
            let theta = 4.0 * bulge.atan();
            let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
            if chord > 0.0 {
                let r = chord / (2.0 * (theta / 2.0).sin());
                // Centre left of the chord for a positive bulge.
                let (mx, my) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
                let d = (r * r - chord * chord / 4.0).max(0.0).sqrt() * if theta.abs() > PI { -1.0 } else { 1.0 };
                let (ux, uy) = ((b[0] - a[0]) / chord, (b[1] - a[1]) / chord);
                let s = theta.signum();
                let c = [mx - uy * d * s, my + ux * d * s];
                let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
                let steps = ((theta.abs() / (PI / 36.0)).ceil() as usize).max(2);
                for k in 1..steps {
                    let t = a0 + theta * k as f64 / steps as f64;
                    let z = a[2] + (b[2] - a[2]) * k as f64 / steps as f64;
                    out.push([c[0] + r.abs() * t.cos(), c[1] + r.abs() * t.sin(), z]);
                }
            }
        }
    }
    out.push(if closed { pts[0].0 } else { pts[n - 1].0 });
    out
}

fn arc_points(c: P3, r: f64, start: f64, end: f64) -> Vec<P3> {
    let mut sweep = end - start;
    if sweep <= 0.0 {
        sweep += 2.0 * PI;
    }
    let steps = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
    (0..=steps).map(|k| {
        let t = start + sweep * k as f64 / steps as f64;
        [c[0] + r * t.cos(), c[1] + r * t.sin(), c[2]]
    }).collect()
}

fn hatch_rings(h: &ocs_plugin_api::host::codec::entities::Hatch) -> Vec<Vec<P3>> {
    let z = h.elevation;
    let mut rings: Vec<(Vec<P3>, bool)> = Vec::new();
    for path in &h.paths {
        let mut pts: Vec<P3> = Vec::new();
        for edge in &path.edges {
            match edge {
                BoundaryEdge::Polyline(p) => {
                    let v: Vec<([f64; 3], f64)> = p.vertices.iter().map(|v| ([v.x, v.y, z], v.z)).collect();
                    pts.extend(bulged(&v, p.is_closed));
                }
                BoundaryEdge::Line(l) => {
                    pts.push([l.start.x, l.start.y, z]);
                    pts.push([l.end.x, l.end.y, z]);
                }
                BoundaryEdge::CircularArc(a) => {
                    let (s, e) = if a.counter_clockwise { (a.start_angle, a.end_angle) } else { (-a.start_angle, -a.end_angle) };
                    pts.extend(arc_points([a.center.x, a.center.y, z], a.radius, s.min(e), s.max(e)));
                }
                BoundaryEdge::EllipticArc(e) => pts.push([e.center.x, e.center.y, z]),
                BoundaryEdge::Spline(s) => {
                    if s.fit_points.len() >= 2 {
                        pts.extend(s.fit_points.iter().map(|p| [p.x, p.y, z]));
                    } else {
                        pts.extend(s.control_points.iter().map(|p| [p.x, p.y, z]));
                    }
                }
            }
        }
        pts.dedup();
        if pts.len() >= 3 {
            if pts.first() != pts.last() {
                pts.push(pts[0]);
            }
            rings.push((pts, path.flags.is_external() || path.flags.is_outermost()));
        }
    }
    // The outer ring first (flagged, else the largest).
    let area = |r: &[P3]| r.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f64>().abs() / 2.0;
    let outer = rings.iter().position(|r| r.1).or_else(|| (0..rings.len()).max_by(|&a, &b| area(&rings[a].0).total_cmp(&area(&rings[b].0))));
    match outer {
        Some(i) => {
            let first = rings.remove(i).0;
            std::iter::once(first).chain(rings.into_iter().map(|r| r.0)).collect()
        }
        None => Vec::new(),
    }
}

/// The geometry of one element in drawing units, or `None` for what has no
/// place in geodata (dimensions, viewports, meshes, …).
fn geometry(e: &EntityType) -> Option<Geom> {
    let line_or_area = |pts: Vec<P3>, closed: bool| -> Option<Geom> {
        if pts.len() < 2 {
            return None;
        }
        if closed && pts.len() >= 4 {
            let mut ring = pts;
            if ring.first() != ring.last() {
                ring.push(ring[0]);
            }
            Some(Geom::Polygon(vec![ring]))
        } else {
            Some(Geom::Line(pts))
        }
    };
    match e {
        EntityType::Point(p) => Some(Geom::Point([p.location.x, p.location.y, p.location.z])),
        EntityType::Line(l) => Some(Geom::Line(vec![[l.start.x, l.start.y, l.start.z], [l.end.x, l.end.y, l.end.z]])),
        EntityType::LwPolyline(p) => {
            let v: Vec<([f64; 3], f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y, p.elevation], v.bulge)).collect();
            line_or_area(bulged(&v, p.is_closed), p.is_closed)
        }
        EntityType::Polyline2D(p) => {
            let v: Vec<([f64; 3], f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y, v.location.z], v.bulge)).collect();
            line_or_area(bulged(&v, p.flags.is_closed()), p.flags.is_closed())
        }
        EntityType::Polyline3D(p) => {
            let mut v: Vec<P3> = p.vertices.iter().map(|v| [v.position.x, v.position.y, v.position.z]).collect();
            if p.flags.closed && !v.is_empty() {
                v.push(v[0]);
            }
            line_or_area(v, p.flags.closed)
        }
        EntityType::Arc(a) => Some(Geom::Line(arc_points([a.center.x, a.center.y, a.center.z], a.radius, a.start_angle, a.end_angle))),
        EntityType::Circle(c) => line_or_area(arc_points([c.center.x, c.center.y, c.center.z], c.radius, 0.0, 2.0 * PI), true),
        EntityType::Ellipse(el) => {
            let (a, ratio) = (el.major_axis.x.hypot(el.major_axis.y), el.minor_axis_ratio);
            let rot = el.major_axis.y.atan2(el.major_axis.x);
            let mut sweep = el.end_parameter - el.start_parameter;
            if sweep <= 0.0 {
                sweep += 2.0 * PI;
            }
            let steps = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
            let pts: Vec<P3> = (0..=steps).map(|k| {
                let t = el.start_parameter + sweep * k as f64 / steps as f64;
                let (x, y) = (a * t.cos(), a * ratio * t.sin());
                [el.center.x + x * rot.cos() - y * rot.sin(), el.center.y + x * rot.sin() + y * rot.cos(), el.center.z]
            }).collect();
            line_or_area(pts, (sweep - 2.0 * PI).abs() < 1e-6)
        }
        EntityType::Spline(s) => {
            let src = if s.fit_points.len() >= 2 { &s.fit_points } else { &s.control_points };
            line_or_area(src.iter().map(|p| [p.x, p.y, p.z]).collect(), false)
        }
        EntityType::Hatch(h) => {
            let rings = hatch_rings(h);
            (!rings.is_empty()).then_some(Geom::Polygon(rings))
        }
        EntityType::Solid(s) => {
            let c = [s.first_corner, s.second_corner, s.fourth_corner, s.third_corner, s.first_corner];
            Some(Geom::Polygon(vec![c.iter().map(|p| [p.x, p.y, p.z]).collect()]))
        }
        EntityType::Insert(i) => Some(Geom::Point([i.insert_point.x, i.insert_point.y, i.insert_point.z])),
        EntityType::Text(t) => Some(Geom::Point([t.insertion_point.x, t.insertion_point.y, t.insertion_point.z])),
        EntityType::MText(t) => Some(Geom::Point([t.insertion_point.x, t.insertion_point.y, t.insertion_point.z])),
        _ => None,
    }
}

fn kind_name(e: &EntityType) -> &'static str {
    match e {
        EntityType::Point(_) => "punt",
        EntityType::Line(_) => "lijn",
        EntityType::LwPolyline(_) | EntityType::Polyline2D(_) | EntityType::Polyline3D(_) => "polylijn",
        EntityType::Arc(_) => "boog",
        EntityType::Circle(_) => "cirkel",
        EntityType::Ellipse(_) => "ellips",
        EntityType::Spline(_) => "spline",
        EntityType::Hatch(_) => "arcering",
        EntityType::Solid(_) => "vlak",
        EntityType::Insert(_) => "symbool",
        EntityType::Text(_) | EntityType::MText(_) => "tekst",
        _ => "overig",
    }
}

/// The elements of the model space as features in the project CRS (x, y)
/// with heights in metres.
pub fn features(doc: &CadDocument, site: &Site, unit_factor: f64, opt: &Options) -> Vec<GeoFeature> {
    let f = unit_factor.max(1e-9);
    let filter = opt.layers.as_ref().map(|s| s.to_lowercase());
    let mut out = Vec::new();
    for e in doc.model_space_entities() {
        let c = e.common();
        if opt.selection.as_ref().is_some_and(|s| !s.contains(&c.handle)) {
            continue;
        }
        if filter.as_ref().is_some_and(|q| !c.layer.to_lowercase().contains(q.as_str())) {
            continue;
        }
        let nlcs = crate::nlcs::parse_layer(&c.layer).ok();
        // Sheet and annotation: the AL discipline or the ZZ main group, also on layers NLCS does not know.
        let parts: Vec<&str> = c.layer.split('-').collect();
        let sheet = nlcs.as_ref().map(|p| p.discipline == "AL" || p.group == "ZZ").unwrap_or(parts.len() >= 4 && (parts[1] == "AL" || parts[2] == "ZZ"));
        if !opt.annotation && sheet {
            continue;
        }
        let Some(g) = geometry(e) else { continue };
        let to = |p: P3| -> P3 {
            let q = site.from_drawing([p[0], p[1]], f);
            [q[0], q[1], p[2] / f]
        };
        let geom = match g {
            Geom::Point(p) => Geom::Point(to(p)),
            Geom::Line(v) => Geom::Line(v.into_iter().map(to).collect()),
            Geom::Polygon(r) => Geom::Polygon(r.into_iter().map(|ring| ring.into_iter().map(to).collect()).collect()),
        };
        let mut props = BTreeMap::new();
        props.insert("laag".into(), c.layer.clone());
        // Our own keys apart from the object data's (a KLIC "type" is the cable kind).
        props.insert("cad_type".into(), kind_name(e).into());
        props.insert("cad_handle".into(), format!("{:X}", c.handle.value()));
        if let Some(p) = &nlcs {
            props.insert("nlcs_status".into(), p.status.code().into());
            props.insert("nlcs_discipline".into(), p.discipline.clone());
            props.insert("nlcs_hoofdgroep".into(), p.group.clone());
            props.insert("nlcs_object".into(), p.object.clone());
            props.insert("nlcs_element".into(), p.element.clone());
        }
        match e {
            EntityType::Text(t) => {
                props.insert("tekst".into(), t.value.clone());
            }
            EntityType::MText(t) => {
                props.insert("tekst".into(), t.value.clone());
            }
            EntityType::Insert(i) => {
                props.insert("blok".into(), i.block_name.clone());
            }
            _ => {}
        }
        for (k, v) in crate::library::infra::host::data(e) {
            props.entry(k).or_insert(v);
        }
        out.push(GeoFeature { geom, props });
    }
    out
}

/// GeoJSON (RFC 7946: WGS 84 longitude, latitude, height).
pub fn geojson(feats: &[GeoFeature], site: &Site) -> Result<String, String> {
    crs::ensure(&site.crs)?;
    let t = Transformer::new(&site.crs, "EPSG:4326")?;
    let ll = |p: &P3| -> Result<serde_json::Value, String> {
        let q = t.apply([p[0], p[1]])?;
        let r = |v: f64, d: i32| (v * 10f64.powi(d)).round() / 10f64.powi(d);
        Ok(serde_json::json!([r(q[0], 8), r(q[1], 8), r(p[2], 3)]))
    };
    let mut list = Vec::with_capacity(feats.len());
    for f in feats {
        let geometry = match &f.geom {
            Geom::Point(p) => serde_json::json!({"type": "Point", "coordinates": ll(p)?}),
            Geom::Line(v) => serde_json::json!({"type": "LineString", "coordinates": v.iter().map(&ll).collect::<Result<Vec<_>, _>>()?}),
            Geom::Polygon(r) => serde_json::json!({"type": "Polygon", "coordinates": r.iter().map(|ring| ring.iter().map(&ll).collect::<Result<Vec<_>, _>>()).collect::<Result<Vec<_>, _>>()?}),
        };
        list.push(serde_json::json!({"type": "Feature", "geometry": geometry, "properties": f.props}));
    }
    serde_json::to_string_pretty(&serde_json::json!({"type": "FeatureCollection", "name": "Open Infra", "features": list})).map_err(|e| e.to_string())
}

/// WKB (little endian, ISO Z) of a geometry.
fn wkb(g: &Geom) -> Vec<u8> {
    let mut b = vec![1u8];
    let pt = |b: &mut Vec<u8>, p: &P3| {
        for v in p {
            b.extend_from_slice(&v.to_le_bytes());
        }
    };
    match g {
        Geom::Point(p) => {
            b.extend_from_slice(&1001u32.to_le_bytes());
            pt(&mut b, p);
        }
        Geom::Line(v) => {
            b.extend_from_slice(&1002u32.to_le_bytes());
            b.extend_from_slice(&(v.len() as u32).to_le_bytes());
            v.iter().for_each(|p| pt(&mut b, p));
        }
        Geom::Polygon(r) => {
            b.extend_from_slice(&1003u32.to_le_bytes());
            b.extend_from_slice(&(r.len() as u32).to_le_bytes());
            for ring in r {
                b.extend_from_slice(&(ring.len() as u32).to_le_bytes());
                ring.iter().for_each(|p| pt(&mut b, p));
            }
        }
    }
    b
}

/// A GeoPackage geometry blob: `GP`, version 0, little endian without an envelope, srs id, WKB.
fn gpkg_blob(g: &Geom, srs: i32) -> Vec<u8> {
    let mut b = vec![b'G', b'P', 0, 0b0000_0001];
    b.extend_from_slice(&srs.to_le_bytes());
    b.extend(wkb(g));
    b
}

/// A column name SQLite and QGIS accept: lowercase letters, digits, underscores.
fn column(k: &str) -> String {
    let c: String = k.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if c.chars().next().is_some_and(|c| c.is_ascii_digit()) || c.is_empty() { format!("a_{c}") } else { c }
}

/// A GeoPackage in the project CRS: tables `punten`, `lijnen` and `vlakken`
/// (only those with features), each with all attribute keys as text columns.
/// An existing file is replaced.
pub fn geopackage(feats: &[GeoFeature], site: &Site, path: &Path) -> Result<(), String> {
    let err = |e: rusqlite::Error| format!("GeoPackage: {e}");
    let epsg: i32 = site.crs.trim_start_matches("EPSG:").parse().map_err(|_| format!("{} {}", tr("geen EPSG-code:", "not an EPSG code:"), site.crs))?;
    let tmp = path.with_extension("gpkg.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        let db = rusqlite::Connection::open(&tmp).map_err(err)?;
        db.execute_batch(
            "PRAGMA application_id = 1196444487; PRAGMA user_version = 10300;
             CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT);
             CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER);
             CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name));
             INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined cartesian SRS', -1, 'NONE', -1, 'undefined', NULL);
             INSERT INTO gpkg_spatial_ref_sys VALUES ('Undefined geographic SRS', 0, 'NONE', 0, 'undefined', NULL);
             INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84 geodetic', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]', NULL);",
        )
        .map_err(err)?;
        if epsg != 4326 {
            let name = crs::builtin_list().into_iter().find(|i| i.code == site.crs).map(|i| i.name).unwrap_or_else(|| site.crs.clone());
            db.execute("INSERT INTO gpkg_spatial_ref_sys VALUES (?1, ?2, 'EPSG', ?2, 'undefined', ?3)", rusqlite::params![name, epsg, "definition by EPSG code"]).map_err(err)?;
        }
        for (table, kind) in [("punten", "POINT"), ("lijnen", "LINESTRING"), ("vlakken", "POLYGON")] {
            let rows: Vec<&GeoFeature> = feats
                .iter()
                .filter(|f| matches!((&f.geom, kind), (Geom::Point(_), "POINT") | (Geom::Line(_), "LINESTRING") | (Geom::Polygon(_), "POLYGON")))
                .collect();
            if rows.is_empty() {
                continue;
            }
            let keys: BTreeSet<String> = rows.iter().flat_map(|f| f.props.keys().cloned()).collect();
            let cols: Vec<(String, String)> = {
                let mut seen = BTreeSet::new();
                keys.iter().filter_map(|k| {
                    let c = column(k);
                    (c != "fid" && c != "geom" && seen.insert(c.clone())).then(|| (k.clone(), c))
                }).collect()
            };
            let defs: String = cols.iter().map(|(_, c)| format!(", \"{c}\" TEXT")).collect();
            db.execute_batch(&format!("CREATE TABLE \"{table}\" (fid INTEGER PRIMARY KEY AUTOINCREMENT, geom {kind}{defs});")).map_err(err)?;
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            {
                let names: String = cols.iter().map(|(_, c)| format!(", \"{c}\"")).collect();
                let marks: String = (0..cols.len()).map(|i| format!(", ?{}", i + 2)).collect();
                let mut st = db.prepare(&format!("INSERT INTO \"{table}\" (geom{names}) VALUES (?1{marks})")).map_err(err)?;
                for f in &rows {
                    let pts: Vec<&P3> = match &f.geom {
                        Geom::Point(p) => vec![p],
                        Geom::Line(v) => v.iter().collect(),
                        Geom::Polygon(r) => r.iter().flatten().collect(),
                    };
                    for p in pts {
                        x0 = x0.min(p[0]);
                        y0 = y0.min(p[1]);
                        x1 = x1.max(p[0]);
                        y1 = y1.max(p[1]);
                    }
                    let mut values: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Blob(gpkg_blob(&f.geom, epsg))];
                    for (k, _) in &cols {
                        values.push(f.props.get(k).map(|v| rusqlite::types::Value::Text(v.clone())).unwrap_or(rusqlite::types::Value::Null));
                    }
                    st.execute(rusqlite::params_from_iter(values)).map_err(err)?;
                }
            }
            db.execute(
                "INSERT INTO gpkg_contents (table_name, data_type, identifier, min_x, min_y, max_x, max_y, srs_id) VALUES (?1, 'features', ?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![table, x0, y0, x1, y1, epsg],
            )
            .map_err(err)?;
            db.execute("INSERT INTO gpkg_geometry_columns VALUES (?1, 'geom', ?2, ?3, 1, 0)", rusqlite::params![table, kind, epsg]).map_err(err)?;
        }
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Counts per kind, for messages: (points, lines, areas).
pub fn counts(feats: &[GeoFeature]) -> (usize, usize, usize) {
    feats.iter().fold((0, 0, 0), |(p, l, a), f| match f.geom {
        Geom::Point(_) => (p + 1, l, a),
        Geom::Line(_) => (p, l + 1, a),
        Geom::Polygon(_) => (p, l, a + 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{Line, LwPolyline, Vector2, Vector3};

    fn site() -> Site {
        let mut s = Site::default();
        s.crs = "EPSG:28992".into();
        s.set_origin(104885.0, 425330.0);
        s.absolute = false; // the drawing around 0,0
        s.width = 300.0;
        s.height = 200.0;
        s
    }

    fn doc() -> CadDocument {
        let mut doc = CadDocument::new();
        let mut l = Line::from_points(Vector3::new(0.0, 0.0, 1.5), Vector3::new(10.0, 0.0, 1.5));
        l.common.layer = "B-OI-KL-ET_LS-G".into();
        l.common.extended_data.add_record(crate::bridge::object_data([("netbeheerder", "Stedin".to_string()), ("spanning", "0,4 kV".to_string())]));
        let _ = doc.add_entity(EntityType::Line(l));
        let mut p = LwPolyline::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            p.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        p.is_closed = true;
        p.common.layer = "N-WE-VH-GESLOTENVERHARDING_ASFALT-A".into();
        let _ = doc.add_entity(EntityType::LwPolyline(p));
        let mut half = LwPolyline::new();
        half.add_point_with_bulge(Vector2::new(0.0, 20.0), 1.0);
        half.add_point_with_bulge(Vector2::new(10.0, 20.0), 0.0);
        half.common.layer = "RANDOM".into();
        let _ = doc.add_entity(EntityType::LwPolyline(half));
        let mut frame = Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0));
        frame.common.layer = "B-AL-ZZ-KADER-G".into();
        let _ = doc.add_entity(EntityType::Line(frame));
        doc
    }

    #[test]
    fn features_carry_layer_nlcs_and_object_data() {
        let s = site();
        let f = features(&doc(), &s, 1.0, &Options::default());
        assert_eq!(f.len(), 3, "the sheet layer stays out");
        let cable = &f[0];
        assert_eq!(cable.props["netbeheerder"], "Stedin");
        assert_eq!(cable.props["nlcs_object"], "ET_LS");
        let Geom::Line(v) = &cable.geom else { panic!() };
        let o = s.from_drawing([0.0, 0.0], 1.0);
        assert_eq!(v[0], [o[0], o[1], 1.5]);
        assert!(matches!(f[1].geom, Geom::Polygon(ref r) if r[0].len() == 5), "a closed polyline is an area");
        let Geom::Line(arc) = &f[2].geom else { panic!() };
        assert!(arc.len() > 10, "the bulge becomes arc points");
        // A half circle of radius 5 below the chord (positive bulge, left of travel? to the right for a CCW arc from (0,20) to (10,20)).
        let mid = arc[arc.len() / 2];
        assert!(((mid[0] - o[0]) - 5.0).abs() < 0.2 && ((mid[1] - o[1]) - 20.0).abs() > 4.8, "{mid:?}");
        let with = features(&doc(), &s, 1.0, &Options { annotation: true, ..Default::default() });
        assert_eq!(with.len(), 4);
        let only = features(&doc(), &s, 1.0, &Options { layers: Some("kl-et".into()), ..Default::default() });
        assert_eq!(only.len(), 1);
    }

    #[test]
    fn geojson_is_wgs84_with_properties() {
        let s = site();
        let j: serde_json::Value = serde_json::from_str(&geojson(&features(&doc(), &s, 1.0, &Options::default()), &s).unwrap()).unwrap();
        let c = &j["features"][0]["geometry"]["coordinates"][0];
        let (lon, lat) = (c[0].as_f64().unwrap(), c[1].as_f64().unwrap());
        assert!((lon - 4.66).abs() < 0.01 && (lat - 51.81).abs() < 0.01, "{lon} {lat}");
        assert_eq!(c[2].as_f64().unwrap(), 1.5);
        assert_eq!(j["features"][1]["geometry"]["type"], "Polygon");
        assert_eq!(j["features"][0]["properties"]["spanning"], "0,4 kV");
    }

    #[test]
    fn geopackage_round_trip_through_our_reader() {
        let s = site();
        let dir = std::env::temp_dir().join(format!("oi-gpkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("export.gpkg");
        geopackage(&features(&doc(), &s, 1.0, &Options::default()), &s, &path).unwrap();
        let lines = crate::gis::files::geopackage(&path, Some("lijnen")).unwrap();
        assert_eq!(lines.crs.as_deref(), Some("EPSG:28992"));
        assert_eq!(lines.features.len(), 2);
        assert!(lines.features.iter().any(|f| f.props.get("netbeheerder").and_then(|v| v.as_str()) == Some("Stedin")));
        let areas = crate::gis::files::geopackage(&path, Some("vlakken")).unwrap();
        assert_eq!(areas.features.len(), 1);
        assert_eq!(areas.features[0].props.get("nlcs_element").and_then(|v| v.as_str()), Some("A"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
