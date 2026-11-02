//! Local GIS files → features: GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX
//! and CSV with coordinate columns. Each reader returns the features with
//! their attributes and, when the file says so, its coordinate system.

use std::io::Read;
use std::path::Path;

use serde_json::{Map, Value};

use super::crs::XY;
use super::vector::{self, Feature};

/// What a file held.
#[derive(Clone, Debug, Default)]
pub struct FileData {
    pub format: &'static str,
    pub features: Vec<Feature>,
    /// The CRS the file declares (`EPSG:xxxx`), if it does.
    pub crs: Option<String>,
    /// Layers in a GeoPackage (the one read is the first, or the asked one).
    pub layers: Vec<String>,
    /// Recoverable reader errors; these are import diagnostics, never attributes.
    pub warnings: Vec<String>,
}

impl FileData {
    pub fn report(&self, message: impl Into<String>) -> String {
        let message = message.into();
        if self.warnings.is_empty() { message } else { format!("{message} {}", self.warnings.join(" ")) }
    }
    /// Coordinate bounds of all features (file CRS).
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut any = false;
        for f in &self.features {
            for p in f.lines.iter().flat_map(|(l, _)| l.iter()).chain(f.points.iter()) {
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
                any = true;
            }
        }
        any.then_some(b)
    }

    /// Attribute names, in first-seen order.
    pub fn fields(&self) -> Vec<String> {
        vector::property_names(&self.features).into_iter().filter(|k| !k.starts_with("__imro_")).collect()
    }
}

/// Read a file by its extension. `layer` picks a GeoPackage table.
pub fn read(path: &Path, layer: Option<&str>) -> Result<FileData, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "geojson" | "json" => geojson(&std::fs::read(path).map_err(|e| io(path, e))?),
        "shp" | "dbf" | "shx" => shapefile(&path.with_extension("shp")),
        "gpkg" => geopackage(path, layer),
        "kml" => kml(&std::fs::read_to_string(path).map_err(|e| io(path, e))?),
        "kmz" => kmz(path),
        "gpx" => gpx(&std::fs::read_to_string(path).map_err(|e| io(path, e))?),
        "gml" | "xml" => gml(&std::fs::read_to_string(path).map_err(|e| io(path, e))?),
        "csv" | "txt" | "tsv" => csv(&std::fs::read_to_string(path).map_err(|e| io(path, e))?),
        other => Err(format!("unsupported file type .{other} (GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX, GML, CSV)")),
    }
}

fn io(path: &Path, e: std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

/// Vector file extensions this module reads (for file dialogs).
pub const EXTENSIONS: &[&str] = &["geojson", "json", "shp", "gpkg", "kml", "kmz", "gpx", "gml", "csv", "txt"];

/// What kind of geodata a file is, by its extension: the one file dialog
/// routes vector files here and rasters / point clouds to their own readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Vector,
    Raster,
    PointCloud,
    Unknown,
}

pub fn kind(path: &Path) -> Kind {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        e if EXTENSIONS.contains(&e) || e == "dbf" || e == "shx" || e == "xml" => Kind::Vector,
        "tif" | "tiff" | "asc" | "jpg" | "jpeg" | "png" | "jp2" => Kind::Raster,
        "las" | "laz" | "e57" | "ply" => Kind::PointCloud,
        _ => Kind::Unknown,
    }
}

// ── CRS from declarations ───────────────────────────────────────────────────

/// `EPSG:xxxx` from a GeoJSON `crs` name, a URN or a URL.
fn epsg_from_name(name: &str) -> Option<String> {
    let digits: String = name.rsplit([':', '/', '#']).next()?.chars().filter(|c| c.is_ascii_digit()).collect();
    if name.to_ascii_uppercase().contains("CRS84") {
        return Some("EPSG:4326".into());
    }
    (!digits.is_empty() && name.to_ascii_uppercase().contains("EPSG")).then(|| format!("EPSG:{digits}"))
}

/// The EPSG code a WKT names: the root `AUTHORITY` / `ID` (the last one in
/// the text), or a well-known ESRI name when there is none.
pub fn epsg_from_wkt(wkt: &str) -> Option<String> {
    let up = wkt.to_ascii_uppercase();
    for key in ["AUTHORITY[\"EPSG\",", "ID[\"EPSG\","] {
        if let Some(i) = up.rfind(key) {
            let rest = &up[i + key.len()..];
            let code: String = rest.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                return Some(format!("EPSG:{code}"));
            }
        }
    }
    let name = up.split('"').nth(1).unwrap_or("").replace(' ', "_");
    let known: &[(&str, &str)] = &[
        ("RD_NEW", "EPSG:28992"),
        ("AMERSFOORT_RD_NEW", "EPSG:28992"),
        ("BELGE_LAMBERT_1972", "EPSG:31370"),
        ("BELGE_LAMBERT_2008", "EPSG:3812"),
        ("BRITISH_NATIONAL_GRID", "EPSG:27700"),
        ("OSGB_1936_BRITISH_NATIONAL_GRID", "EPSG:27700"),
        ("CH1903+_LV95", "EPSG:2056"),
        ("RGF93_LAMBERT_93", "EPSG:2154"),
        ("RGF93_V1_LAMBERT-93", "EPSG:2154"),
        ("GCS_WGS_1984", "EPSG:4326"),
        ("WGS_84", "EPSG:4326"),
        ("WGS_1984_WEB_MERCATOR_AUXILIARY_SPHERE", "EPSG:3857"),
        ("ETRS89_/_LAEA_EUROPE", "EPSG:3035"),
        ("GDA2020_MGA_ZONE_55", "EPSG:7855"),
        ("GDA2020_MGA_ZONE_56", "EPSG:7856"),
    ];
    if let Some((_, c)) = known.iter().find(|(n, _)| *n == name) {
        return Some((*c).into());
    }
    // (WGS_1984|ETRS_1989|NAD_1983)_UTM_ZONE_31N and friends.
    let utm = name.split("UTM_ZONE_").nth(1)?;
    let zone: u32 = utm.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    let north = !utm.trim_start_matches(|c: char| c.is_ascii_digit()).starts_with('S');
    if name.starts_with("ETRS") && north && (28..=38).contains(&zone) {
        Some(format!("EPSG:{}", 25800 + zone))
    } else if name.starts_with("NAD_1983") && north && (1..=23).contains(&zone) {
        Some(format!("EPSG:{}", 26900 + zone))
    } else if name.starts_with("WGS") || name.starts_with("ETRS") {
        Some(format!("EPSG:{}", if north { 32600 } else { 32700 } + zone))
    } else {
        None
    }
}

// ── GeoJSON ─────────────────────────────────────────────────────────────────

pub fn geojson(bytes: &[u8]) -> Result<FileData, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("GeoJSON: {e}"))?;
    let v = if v["type"] == "Feature" { serde_json::json!({"type": "FeatureCollection", "features": [v]}) } else { v };
    let crs = v["crs"]["properties"]["name"].as_str().and_then(epsg_from_name).or(Some("EPSG:4326".into()));
    Ok(FileData { format: "GeoJSON", features: vector::parse_geojson(&v), crs, layers: Vec::new(), warnings: Vec::new() })
}

// ── Shapefile ───────────────────────────────────────────────────────────────

fn dbf_value(v: &shapefile::dbase::FieldValue) -> Value {
    use shapefile::dbase::FieldValue as F;
    match v {
        F::Character(Some(s)) | F::Memo(s) => Value::String(s.trim().to_string()),
        F::Numeric(Some(n)) | F::Double(n) | F::Currency(n) => serde_json::Number::from_f64(*n).map(Value::Number).unwrap_or(Value::Null),
        F::Float(Some(n)) => serde_json::Number::from_f64(*n as f64).map(Value::Number).unwrap_or(Value::Null),
        F::Integer(i) => Value::from(*i),
        F::Logical(Some(b)) => Value::Bool(*b),
        F::Date(Some(d)) => Value::String(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())),
        _ => Value::Null,
    }
}

pub fn shapefile(path: &Path) -> Result<FileData, String> {
    use shapefile::Shape;
    let mut reader = shapefile::Reader::from_path(path).map_err(|e| format!("Shapefile {}: {e}", path.display()))?;
    let crs = std::fs::read_to_string(path.with_extension("prj")).ok().and_then(|w| epsg_from_wkt(&w));
    let mut features = Vec::new();
    for item in reader.iter_shapes_and_records() {
        let (shape, record) = item.map_err(|e| format!("Shapefile: {e}"))?;
        let mut f = Feature::default();
        let mut props = Map::new();
        for (k, v) in record {
            props.insert(k, dbf_value(&v));
        }
        f.props = props;
        let p2 = |x: f64, y: f64| -> XY { [x, y] };
        match shape {
            Shape::NullShape => {}
            Shape::Point(p) => f.points.push(p2(p.x, p.y)),
            Shape::PointM(p) => f.points.push(p2(p.x, p.y)),
            Shape::PointZ(p) => f.points.push(p2(p.x, p.y)),
            Shape::Multipoint(m) => f.points.extend(m.points().iter().map(|p| p2(p.x, p.y))),
            Shape::MultipointM(m) => f.points.extend(m.points().iter().map(|p| p2(p.x, p.y))),
            Shape::MultipointZ(m) => f.points.extend(m.points().iter().map(|p| p2(p.x, p.y))),
            Shape::Polyline(l) => f.lines.extend(l.parts().iter().map(|part| (part.iter().map(|p| p2(p.x, p.y)).collect(), false))),
            Shape::PolylineM(l) => f.lines.extend(l.parts().iter().map(|part| (part.iter().map(|p| p2(p.x, p.y)).collect(), false))),
            Shape::PolylineZ(l) => f.lines.extend(l.parts().iter().map(|part| (part.iter().map(|p| p2(p.x, p.y)).collect(), false))),
            Shape::Polygon(g) => f.lines.extend(g.rings().iter().map(|r| (r.points().iter().map(|p| p2(p.x, p.y)).collect(), true))),
            Shape::PolygonM(g) => f.lines.extend(g.rings().iter().map(|r| (r.points().iter().map(|p| p2(p.x, p.y)).collect(), true))),
            Shape::PolygonZ(g) => f.lines.extend(g.rings().iter().map(|r| (r.points().iter().map(|p| p2(p.x, p.y)).collect(), true))),
            Shape::Multipatch(_) => {}
        }
        features.push(f);
    }
    Ok(FileData { format: "Shapefile", features, crs, layers: Vec::new(), warnings: Vec::new() })
}

// ── WKB (GeoPackage) ────────────────────────────────────────────────────────

struct Wkb<'a> {
    b: &'a [u8],
    i: usize,
}

impl Wkb<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.i)?;
        self.i += 1;
        Some(v)
    }
    fn u32(&mut self, le: bool) -> Option<u32> {
        let s: [u8; 4] = self.b.get(self.i..self.i + 4)?.try_into().ok()?;
        self.i += 4;
        Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
    fn f64(&mut self, le: bool) -> Option<f64> {
        let s: [u8; 8] = self.b.get(self.i..self.i + 8)?.try_into().ok()?;
        self.i += 8;
        Some(if le { f64::from_le_bytes(s) } else { f64::from_be_bytes(s) })
    }

    /// One geometry into `f`. ISO (1001, 2001, 3001) and EWKB-style flag
    /// dimensions are both read; Z and M are skipped.
    fn geometry(&mut self, f: &mut Feature) -> Option<()> {
        let le = self.u8()? == 1;
        let raw = self.u32(le)?;
        let (base, mut dims) = match raw {
            r if r & 0x8000_0000 != 0 || r & 0x4000_0000 != 0 => (r & 0xFFFF, 2 + (r & 0x8000_0000 != 0) as usize + (r & 0x4000_0000 != 0) as usize),
            r if r >= 3000 => (r - 3000, 4),
            r if r >= 2000 => (r - 2000, 3),
            r if r >= 1000 => (r - 1000, 3),
            r => (r, 2),
        };
        if dims < 2 {
            dims = 2;
        }
        let point = |w: &mut Self| -> Option<XY> {
            let x = w.f64(le)?;
            let y = w.f64(le)?;
            for _ in 2..dims {
                w.f64(le)?;
            }
            Some([x, y])
        };
        let line = |w: &mut Self| -> Option<Vec<XY>> {
            let n = w.u32(le)? as usize;
            (0..n).map(|_| point(w)).collect()
        };
        match base {
            1 => {
                let p = point(self)?;
                if p[0].is_finite() && p[1].is_finite() {
                    f.points.push(p);
                }
            }
            2 => {
                let l = line(self)?;
                f.lines.push((l, false));
            }
            3 => {
                let rings = self.u32(le)?;
                for _ in 0..rings {
                    let r = line(self)?;
                    f.lines.push((r, true));
                }
            }
            4..=7 => {
                let n = self.u32(le)?;
                for _ in 0..n {
                    self.geometry(f)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

/// A GeoPackage geometry blob: the `GP` header (with an optional envelope),
/// then standard WKB.
pub fn gpkg_geometry(blob: &[u8], f: &mut Feature) -> bool {
    if blob.len() < 8 || &blob[0..2] != b"GP" {
        return Wkb { b: blob, i: 0 }.geometry(f).is_some();
    }
    let flags = blob[3];
    let envelope = match (flags >> 1) & 0b111 {
        1 => 32,
        2 | 3 => 48,
        4 => 64,
        _ => 0,
    };
    let start = 8 + envelope;
    blob.get(start..).is_some_and(|b| Wkb { b, i: 0 }.geometry(f).is_some())
}

pub fn geopackage(path: &Path, layer: Option<&str>) -> Result<FileData, String> {
    use rusqlite::{types::ValueRef, Connection, OpenFlags};
    let err = |e: rusqlite::Error| format!("GeoPackage: {e}");
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    let mut st = db.prepare("SELECT c.table_name, g.column_name, g.srs_id FROM gpkg_contents c JOIN gpkg_geometry_columns g ON g.table_name = c.table_name WHERE c.data_type = 'features' ORDER BY c.table_name").map_err(err)?;
    let tables: Vec<(String, String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(err)?.filter_map(Result::ok).collect();
    let layers: Vec<String> = tables.iter().map(|t| t.0.clone()).collect();
    let (table, geom_col, srs) = match layer {
        Some(l) => tables.iter().find(|t| t.0 == l).cloned().ok_or_else(|| format!("GeoPackage: no layer {l}"))?,
        None => tables.first().cloned().ok_or("GeoPackage: no feature layers")?,
    };
    let crs = db
        .query_row("SELECT organization, organization_coordsys_id FROM gpkg_spatial_ref_sys WHERE srs_id = ?1", [srs], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .ok()
        .and_then(|(org, id)| org.eq_ignore_ascii_case("EPSG").then(|| format!("EPSG:{id}")))
        .or_else(|| (srs > 0).then(|| format!("EPSG:{srs}")));
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut st = db.prepare(&format!("SELECT * FROM {}", q(&table))).map_err(err)?;
    let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
    let gi = names.iter().position(|n| n.eq_ignore_ascii_case(&geom_col)).ok_or("GeoPackage: geometry column missing")?;
    let mut rows = st.query([]).map_err(err)?;
    let mut features = Vec::new();
    while let Some(row) = rows.next().map_err(err)? {
        let mut f = Feature::default();
        for (i, n) in names.iter().enumerate() {
            let v = row.get_ref(i).map_err(err)?;
            if i == gi {
                if let ValueRef::Blob(b) = v {
                    gpkg_geometry(b, &mut f);
                }
                continue;
            }
            let val = match v {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(x) => Value::from(x),
                ValueRef::Real(x) => serde_json::Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null),
                ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Blob(_) => continue,
            };
            f.props.insert(n.clone(), val);
        }
        features.push(f);
    }
    Ok(FileData { format: "GeoPackage", features, crs, layers, warnings: Vec::new() })
}

// ── KML / KMZ / GPX ─────────────────────────────────────────────────────────

fn kml_coords(text: &str) -> Vec<XY> {
    text.split_whitespace()
        .filter_map(|t| {
            let mut it = t.split(',').map(|v| v.trim().parse::<f64>());
            Some([it.next()?.ok()?, it.next()?.ok()?])
        })
        .collect()
}

fn kml_geometry(node: roxmltree::Node, f: &mut Feature) {
    let coords = |n: roxmltree::Node| n.descendants().find(|d| d.has_tag_name("coordinates")).and_then(|c| c.text()).map(kml_coords).unwrap_or_default();
    match node.tag_name().name() {
        "Point" => f.points.extend(coords(node)),
        "LineString" => f.lines.push((coords(node), false)),
        "LinearRing" => f.lines.push((coords(node), true)),
        "Polygon" => {
            for ring in node.descendants().filter(|d| d.has_tag_name("LinearRing")) {
                f.lines.push((coords(ring), true));
            }
        }
        "MultiGeometry" => {
            for child in node.children().filter(|c| c.is_element()) {
                kml_geometry(child, f);
            }
        }
        _ => {}
    }
}

pub fn kml(text: &str) -> Result<FileData, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("KML: {e}"))?;
    let mut features = Vec::new();
    for pm in doc.descendants().filter(|n| n.has_tag_name("Placemark")) {
        let mut f = Feature::default();
        for child in pm.children().filter(|c| c.is_element()) {
            match child.tag_name().name() {
                "name" | "description" => {
                    if let Some(t) = child.text() {
                        f.props.insert(child.tag_name().name().into(), Value::String(t.trim().into()));
                    }
                }
                "ExtendedData" => {
                    for d in child.descendants().filter(|d| d.has_tag_name("Data") || d.has_tag_name("SimpleData")) {
                        let key = d.attribute("name").unwrap_or("").to_string();
                        let val = if d.has_tag_name("Data") { d.children().find(|c| c.has_tag_name("value")).and_then(|v| v.text()) } else { d.text() };
                        if !key.is_empty() {
                            f.props.insert(key, Value::String(val.unwrap_or("").trim().into()));
                        }
                    }
                }
                _ => kml_geometry(child, &mut f),
            }
        }
        features.push(f);
    }
    Ok(FileData { format: "KML", features, crs: Some("EPSG:4326".into()), layers: Vec::new(), warnings: Vec::new() })
}

pub fn kmz(path: &Path) -> Result<FileData, String> {
    let file = std::fs::File::open(path).map_err(|e| io(path, e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("KMZ: {e}"))?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.to_ascii_lowercase().ends_with(".kml"))
        .ok_or("KMZ: no .kml inside")?;
    let mut text = String::new();
    zip.by_name(&name).map_err(|e| format!("KMZ: {e}"))?.read_to_string(&mut text).map_err(|e| format!("KMZ: {e}"))?;
    let mut d = kml(&text)?;
    d.format = "KMZ";
    Ok(d)
}

pub fn gpx(text: &str) -> Result<FileData, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("GPX: {e}"))?;
    let pt = |n: roxmltree::Node| -> Option<XY> { Some([n.attribute("lon")?.parse().ok()?, n.attribute("lat")?.parse().ok()?]) };
    let name = |n: roxmltree::Node| n.children().find(|c| c.has_tag_name("name")).and_then(|c| c.text()).map(|t| t.trim().to_string());
    let mut features = Vec::new();
    for n in doc.root_element().children().filter(|c| c.is_element()) {
        let mut f = Feature::default();
        match n.tag_name().name() {
            "wpt" => {
                f.points.extend(pt(n));
                for c in n.children().filter(|c| c.is_element() && c.text().is_some()) {
                    f.props.insert(c.tag_name().name().into(), Value::String(c.text().unwrap_or("").trim().into()));
                }
            }
            "rte" => f.lines.push((n.children().filter(|c| c.has_tag_name("rtept")).filter_map(pt).collect(), false)),
            "trk" => {
                for seg in n.children().filter(|c| c.has_tag_name("trkseg")) {
                    f.lines.push((seg.children().filter(|c| c.has_tag_name("trkpt")).filter_map(pt).collect(), false));
                }
            }
            _ => continue,
        }
        if let Some(nm) = name(n) {
            f.props.entry("name").or_insert(Value::String(nm));
        }
        f.props.insert("gpx_type".into(), Value::String(n.tag_name().name().into()));
        features.push(f);
    }
    Ok(FileData { format: "GPX", features, crs: Some("EPSG:4326".into()), layers: Vec::new(), warnings: Vec::new() })
}

// ── GML (simple features, GML 2 and 3) ─────────────────────────────────────

fn gml_numbers(text: &str, separator: Option<char>) -> Result<Vec<f64>, String> {
    text.split(|c: char| separator.map_or_else(|| c.is_whitespace(), |s| c == s)).filter(|t| separator.is_some() || !t.is_empty()).map(|t| {
        let v = t.trim().parse::<f64>().map_err(|_| format!("GML: ongeldig coördinaatgetal '{t}'"))?;
        if !v.is_finite() { return Err(format!("GML: niet-eindig coördinaatgetal '{t}'")); }
        Ok(v)
    }).collect()
}

fn gml_dimension(node: roxmltree::Node, fallback: usize) -> Result<usize, String> {
    match node.ancestors().find_map(|n| n.attribute("srsDimension")) {
        Some(text) => match text.trim().parse::<usize>() {
            Ok(d) if d >= 2 => Ok(d),
            _ => Err(format!("GML: srsDimension '{text}' kan niet naar XY worden gelezen")),
        },
        None => Ok(fallback),
    }
}

fn gml_coords(node: roxmltree::Node, dim: usize) -> Result<Vec<XY>, String> {
    // GML 3: inherit the nearest dimension from the coordinate or geometry.
    if let Some(list) = node.descendants().find(|d| d.has_tag_name("posList")) {
        let d = gml_dimension(list, dim)?;
        let v = gml_numbers(list.text().unwrap_or("").trim(), None)?;
        if v.len() % d != 0 { return Err("GML: onvolledige coördinaat in posList".into()); }
        return Ok(v.chunks_exact(d).map(|c| [c[0], c[1]]).collect());
    }
    let mut pos = Vec::new();
    for p in node.descendants().filter(|d| d.has_tag_name("pos")) {
        let v = gml_numbers(p.text().unwrap_or("").trim(), None)?;
        let d = gml_dimension(p, v.len())?;
        if d < 2 || v.len() != d { return Err("GML: onvolledige coördinaat in pos".into()); }
        pos.push([v[0], v[1]]);
    }
    if !pos.is_empty() { return Ok(pos); }
    // GML 2: x,y[,z] tuples, separated by whitespace.
    if let Some(c) = node.descendants().find(|d| d.has_tag_name("coordinates")) {
        let mut dimension = None;
        return c.text().unwrap_or("").split_whitespace().map(|tuple| {
            let v = gml_numbers(tuple, Some(','))?;
            let d = *dimension.get_or_insert(gml_dimension(c, v.len())?);
            if d < 2 || v.len() != d { return Err("GML: onvolledige coordinates-tuple".into()); }
            Ok([v[0], v[1]])
        }).collect();
    }
    Ok(Vec::new())
}

// A Ring/Curve contains ordered curve parts. Read every supported part before
// applying any length check; an unhandled part must not become a partial ring.
fn gml_curve(node: roxmltree::Node, depth: usize) -> Result<Vec<XY>, String> {
    if depth > 64 { return Err("GML: te diep geneste curve (leeslimiet 64)".into()); }
    let name = node.tag_name().name();
    if matches!(name, "LineString" | "LineStringSegment" | "LinearRing") {
        let points = gml_coords(node, 2)?;
        if points.len() < if name == "LinearRing" { 3 } else { 2 } { return Err(format!("GML: te weinig coördinaten in {name}")); }
        return Ok(points);
    }
    let parts: Vec<_> = match name {
        "Curve" => node.children().find(|n| n.has_tag_name("segments"))
            .ok_or("GML: Curve zonder segments")?.children().filter(|n| n.is_element()).collect(),
        "Ring" | "CompositeCurve" => {
            let mut parts = Vec::new();
            for member in node.children().filter(|n| n.has_tag_name("curveMember")) {
                let part = member.children().find(|n| n.is_element())
                    .ok_or("GML: curveMember zonder inline curve wordt niet ondersteund")?;
                parts.push(part);
            }
            if parts.is_empty() { return Err(format!("GML: {name} zonder curveMember")); }
            parts
        }
        _ => return Err(format!("GML: curveonderdeel {name} wordt niet ondersteund")),
    };
    let mut points = Vec::new();
    for part in parts {
        let next = gml_curve(part, depth + 1)?;
        if next.is_empty() { continue; }
        let shared = points.last().is_some_and(|p| *p == next[0]);
        if !points.is_empty() && !shared { return Err("GML: curveonderdelen sluiten niet aan".into()); }
        points.extend(next.into_iter().skip(usize::from(shared)));
    }
    Ok(points)
}

/// The geometry elements directly under a property, read into `f`.
fn gml_geometry(node: roxmltree::Node, f: &mut Feature) -> Result<bool, String> {
    gml_geometry_at(node, f, 0)
}

fn gml_geometry_at(node: roxmltree::Node, f: &mut Feature, depth: usize) -> Result<bool, String> {
    if depth > 64 { return Err("GML: te diep geneste geometrie (leeslimiet 64)".into()); }
    let name = node.tag_name().name();
    match name {
        "Point" => {
            let points = gml_coords(node, 2)?;
            if points.len() != 1 { return Err("GML: Point verwacht één coördinaat".into()); }
            f.points.extend(points);
        }
        "LineString" | "Curve" | "LineStringSegment" | "LinearRing" | "Ring" | "CompositeCurve" => {
            let ring = matches!(name, "LinearRing" | "Ring");
            let points = gml_curve(node, 0)?;
            if ring && points.len() < 3 { return Err(format!("GML: te weinig coördinaten in {name}")); }
            if !points.is_empty() { f.lines.push((points, ring)); }
        }
        "Polygon" | "Surface" | "PolygonPatch" => {
            for r in node.descendants().filter(|d| d.has_tag_name("LinearRing") || d.has_tag_name("Ring")) {
                gml_geometry_at(r, f, depth + 1)?;
            }
        }
        "MultiPoint" | "MultiCurve" | "MultiLineString" | "MultiSurface" | "MultiPolygon" | "MultiGeometry" | "CompositeSurface" => {
            for m in node.children().filter(|c| c.is_element()) {
                // pointMember / surfaceMembers: the geometries one level down.
                for g in m.children().filter(|c| c.is_element()) {
                    gml_geometry_at(g, f, depth + 1)?;
                }
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Features from a GML file: every member of a feature collection (or of a
/// WFS response) with its geometry and its simple properties. The CRS comes
/// from the first `srsName`.
pub fn gml(text: &str) -> Result<FileData, String> {
    if super::planning::gml::is_imro(text) { return Ok(super::planning::gml::parse(text)?.file_data()); }
    let doc = roxmltree::Document::parse(text).map_err(|e| format!("GML: {e}"))?;
    let root = doc.root_element();
    let crs = doc.descendants().find_map(|n| n.attribute("srsName")).and_then(epsg_from_name);
    let members: Vec<roxmltree::Node> = root
        .children()
        .filter(|c| c.is_element())
        .flat_map(|m| {
            let n = m.tag_name().name();
            if n == "featureMember" || n == "member" {
                m.children().filter(|c| c.is_element()).collect::<Vec<_>>()
            } else if n == "featureMembers" {
                m.children().filter(|c| c.is_element()).collect()
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut features = Vec::new();
    let mut warnings = Vec::new();
    for (index, fnode) in members.into_iter().enumerate() {
        let id = fnode.attributes().find(|a| a.name() == "id").map(|a| a.value().to_string());
        let parsed = (|| -> Result<Feature, String> {
        let mut f = Feature::default();
        for prop in fnode.children().filter(|c| c.is_element()) {
            let mut geom = false;
            for g in prop.children().filter(|c| c.is_element()) {
                if gml_geometry(g, &mut f)? { geom = true; break; }
            }
            if !geom {
                if let Some(t) = prop.text().map(str::trim).filter(|t| !t.is_empty()) {
                    if prop.children().all(|c| !c.is_element()) {
                        f.props.insert(prop.tag_name().name().to_string(), number(t).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(t.to_string())));
                    }
                }
            }
        }
        if let Some(id) = &id {
            f.props.entry("gml_id").or_insert(Value::String(id.clone()));
        }
        Ok(f)
        })();
        match parsed {
            Ok(f) => features.push(f),
            Err(error) => warnings.push(format!("GML: object {} overgeslagen: {error}", id.unwrap_or_else(|| (index + 1).to_string()))),
        }
    }
    Ok(FileData { format: "GML", features, crs, layers: Vec::new(), warnings })
}

// ── CSV ─────────────────────────────────────────────────────────────────────

fn split_line(line: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

fn number(s: &str) -> Option<f64> {
    let t = s.trim();
    t.parse().ok().or_else(|| if t.matches(',').count() == 1 && !t.contains('.') { t.replace(',', ".").parse().ok() } else { None })
}

/// Points from a table with coordinate columns: x/y, easting/northing,
/// lon/lat (also Dutch names). `;`, tab or `,` separated.
pub fn csv(text: &str) -> Result<FileData, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = lines.next().ok_or("CSV: empty file")?;
    let sep = [';', '\t', ','].into_iter().max_by_key(|s| head.matches(*s).count()).unwrap_or(',');
    let cols: Vec<String> = split_line(head, sep).into_iter().map(|c| c.trim_start_matches('\u{feff}').to_string()).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)));
    let (ix, iy, geographic) = if let (Some(x), Some(y)) = (find(&["lon", "lng", "long", "longitude", "lengtegraad"]), find(&["lat", "latitude", "breedtegraad"])) {
        (x, y, true)
    } else if let (Some(x), Some(y)) = (find(&["x", "easting", "east", "e", "x_rd", "rd_x", "xcoord", "x_coord"]), find(&["y", "northing", "north", "n", "y_rd", "rd_y", "ycoord", "y_coord"])) {
        (x, y, false)
    } else {
        return Err(format!("CSV: no coordinate columns (x/y, easting/northing or lon/lat) in {}", cols.join(", ")));
    };
    let mut features = Vec::new();
    for line in lines {
        let vals = split_line(line, sep);
        let (Some(x), Some(y)) = (vals.get(ix).and_then(|v| number(v)), vals.get(iy).and_then(|v| number(v))) else { continue };
        let mut f = Feature { points: vec![[x, y]], ..Default::default() };
        for (i, c) in cols.iter().enumerate() {
            if i != ix && i != iy {
                if let Some(v) = vals.get(i) {
                    f.props.insert(c.clone(), number(v).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(v.clone())));
                }
            }
        }
        features.push(f);
    }
    Ok(FileData { format: "CSV", features, crs: geographic.then(|| "EPSG:4326".to_string()), layers: Vec::new(), warnings: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GML: &str = include_str!("../../tests/fixtures/gml/features.gml");

    #[test]
    fn gml_fixture_inherits_dimensions_and_preserves_crs_and_ids() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gml/features.gml");
        let data = read(&path, None).unwrap();
        assert_eq!(data.crs.as_deref(), Some("EPSG:2056"));
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].props["gml_id"], "line-1");
        assert_eq!(data.features[0].lines[0].0, vec![[2600000.0, 1200000.0], [2600010.0, 1200020.0]]);
        assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
    }

    #[test]
    fn gml_skips_invalid_or_incomplete_coordinate_tuples_with_a_warning() {
        for (from, to) in [
            ("2600000 1200000 5 2600010 1200020 6", "2600000 bad 1200000 5 2600010 1200020 6"),
            ("2600000 1200000 5 2600010 1200020 6", "2600000 1200000 5 2600010 1200020"),
            ("2600000 1200000 5 2600010 1200020 6", ""),
            ("2600005 1200005", "2600005 bad 1200005"),
            ("2600005 1200005", "2600005"),
            ("2600005 1200005", "NaN 1200005"),
            ("2600005 1200005", "inf 1200005"),
            ("srsDimension=\"3\"", "srsDimension=\"0\""),
            ("srsDimension=\"3\"", "srsDimension=\"bad\""),
        ] {
            let bad = GML.replace(from, to);
            assert_ne!(bad, GML);
            let data = gml(&bad).unwrap();
            assert_eq!(data.features.len(), 1, "{from} -> {to}");
            assert_eq!(data.warnings.len(), 1, "{from} -> {to}");
            assert!(data.warnings[0].contains("overgeslagen"));
        }
    }

    #[test]
    fn gml2_coordinates_reject_invalid_numbers_without_shifting_tuples() {
        let text = GML.replace("<gml:pos>2600005 1200005</gml:pos>", "<gml:coordinates>2600005,1200005,7</gml:coordinates>");
        assert_eq!(gml(&text).unwrap().features[1].points[0], [2600005.0, 1200005.0]);
        for tuple in ["2600005,bad,1200005", "2600005,NaN,7", "2600005,1200005,inf", "2600005"] {
            let data = gml(&text.replace("2600005,1200005,7", tuple)).unwrap();
            assert_eq!(data.features.len(), 1, "{tuple}");
            assert_eq!(data.features[0].props["gml_id"], "line-1");
            assert_eq!(data.warnings.len(), 1);
        }
    }

    #[test]
    fn gml_handles_empty_missing_optional_fields_large_numbers_and_broken_xml() {
        let empty = r#"<gml:FeatureCollection xmlns:gml="http://www.opengis.net/gml"/>"#;
        assert!(gml(empty).unwrap().features.is_empty());
        let huge = gml(&GML.replace("2600005 1200005", "1e100 -1e100")).unwrap();
        assert_eq!(huge.features[1].points[0], [1e100, -1e100]);
        for text in ["", "<", &GML[..GML.len() / 2]] {
            assert!(gml(text).is_err());
        }
    }

    #[test]
    fn gml_nil_properties_preserve_other_features_without_fake_coordinates() {
        for geometry in [
            "<t:geometry nilReason=\"unknown\"/>",
            "<t:geometry xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/>",
            "<t:geometry><gml:MultiSurface/></t:geometry>",
        ] {
            let missing = format!("<wfs:member><t:missing gml:id=\"missing\">{geometry}<t:height xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/></t:missing></wfs:member>");
            let text = GML.replace("</wfs:FeatureCollection>", &format!("{missing}</wfs:FeatureCollection>"));
            let data = gml(&text).unwrap();
            assert_eq!(data.features.len(), 3);
            assert_eq!(data.features[0].lines[0].0, vec![[2600000.0, 1200000.0], [2600010.0, 1200020.0]]);
            assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
            assert!(data.features[2].points.is_empty() && data.features[2].lines.is_empty());
            assert_eq!(data.features[2].props["gml_id"], "missing");
        }
    }

    #[test]
    fn gml_composite_ring_keeps_every_part_and_the_adjacent_point() {
        let ring = "<gml:Polygon><gml:exterior><gml:Ring><gml:curveMember><gml:LineString><gml:posList>0 0 10 0</gml:posList></gml:LineString></gml:curveMember><gml:curveMember><gml:LineString><gml:posList>10 0 10 10</gml:posList></gml:LineString></gml:curveMember><gml:curveMember><gml:LineString><gml:posList>10 10 0 0</gml:posList></gml:LineString></gml:curveMember></gml:Ring></gml:exterior></gml:Polygon>";
        let from = "<gml:LineString srsName=\"urn:ogc:def:crs:EPSG::2056\" srsDimension=\"3\">\n      <gml:posList>2600000 1200000 5 2600010 1200020 6</gml:posList>\n    </gml:LineString>";
        assert!(GML.contains(from));
        let data = gml(&GML.replace(from, ring)).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].lines[0], (vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]], true));
        assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn gml_empty_curve_container_does_not_hide_the_adjacent_point() {
        let text = GML.replace("<gml:LineString srsName=\"urn:ogc:def:crs:EPSG::2056\" srsDimension=\"3\">\n      <gml:posList>2600000 1200000 5 2600010 1200020 6</gml:posList>\n    </gml:LineString>", "<gml:Curve><gml:segments/></gml:Curve>");
        assert_ne!(text, GML);
        let data = gml(&text).unwrap();
        assert_eq!(data.features.len(), 2);
        assert!(data.features[0].lines.is_empty());
        assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
    }

    #[test]
    fn gml_projects_complete_four_and_five_dimensional_tuples_to_xy() {
        for (dim, values) in [(4, "1 2 3 4 5 6 7 8"), (5, "1 2 3 4 5 6 7 8 9 10")] {
            let text = GML.replace("srsDimension=\"3\"", &format!("srsDimension=\"{dim}\""))
                .replace("2600000 1200000 5 2600010 1200020 6", values);
            let data = gml(&text).unwrap();
            assert_eq!(data.features[0].lines[0].0, vec![[1.0, 2.0], [if dim == 4 { 5.0 } else { 6.0 }, if dim == 4 { 6.0 } else { 7.0 }]]);
            assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
        }
    }

    #[test]
    fn gml_bad_object_does_not_discard_the_valid_adjacent_point() {
        let data = gml(&GML.replace("2600000 1200000 5 2600010 1200020 6", "2600000 broken 5 2600010 1200020 6")).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("line-1 overgeslagen"));
        assert!(data.report("1 object gelezen").contains(&data.warnings[0]));
    }

    #[test]
    fn gml_linear_segments_are_joined_and_unsupported_parts_are_reported() {
        let line = "<gml:LineString srsName=\"urn:ogc:def:crs:EPSG::2056\" srsDimension=\"3\">\n      <gml:posList>2600000 1200000 5 2600010 1200020 6</gml:posList>\n    </gml:LineString>";
        let curve = "<gml:Curve><gml:segments><gml:LineStringSegment><gml:posList>0 0 10 0</gml:posList></gml:LineStringSegment><gml:LineStringSegment><gml:posList>10 0 10 10</gml:posList></gml:LineStringSegment></gml:segments></gml:Curve>";
        let data = gml(&GML.replace(line, curve)).unwrap();
        assert_eq!(data.features[0].lines[0], (vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false));
        assert!(data.warnings.is_empty());
        for curve in [curve.replace("LineStringSegment", "Arc"), curve.replace("10 0 10 10", "20 0 10 10")] {
            let data = gml(&GML.replace(line, &curve)).unwrap();
            assert_eq!(data.features.len(), 1);
            assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
            assert_eq!(data.warnings.len(), 1);
        }
    }

    #[test]
    fn gml_file_import_reports_a_skipped_feature_and_keeps_the_good_one() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gml/mixed.gml");
        let data = read(&path, None).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
        assert_eq!(data.warnings.len(), 1);
        let message = data.report("GML: 1 object gelezen");
        assert!(message.contains("broken-line overgeslagen"));
        assert!(message.contains("ongeldig coördinaatgetal"));
    }

    #[test]
    fn wkt_codes() {
        assert_eq!(epsg_from_wkt(r#"PROJCS["Amersfoort / RD New",GEOGCS["Amersfoort",AUTHORITY["EPSG","4289"]],AUTHORITY["EPSG","28992"]]"#).as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg_from_wkt(r#"PROJCS["ETRS_1989_UTM_Zone_32N",GEOGCS["GCS_ETRS_1989"]]"#).as_deref(), Some("EPSG:25832"));
        assert_eq!(epsg_from_wkt(r#"PROJCS["WGS_1984_UTM_Zone_33S",GEOGCS["GCS_WGS_1984"]]"#).as_deref(), Some("EPSG:32733"));
        assert_eq!(epsg_from_wkt(r#"PROJCS["RD_New",GEOGCS["GCS_Amersfoort"]]"#).as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg_from_name("urn:ogc:def:crs:EPSG::28992").as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg_from_name("urn:ogc:def:crs:OGC:1.3:CRS84").as_deref(), Some("EPSG:4326"));
    }

    #[test]
    fn csv_points() {
        let d = csv("naam;x;y\nput 1;104885,5;425320\nput 2;104890;425330\n").unwrap();
        assert_eq!(d.features.len(), 2);
        assert_eq!(d.features[0].points[0], [104885.5, 425320.0]);
        assert_eq!(d.features[0].props["naam"], "put 1");
        assert!(d.crs.is_none());
        let g = csv("name,lon,lat\nA,4.66,51.81\n").unwrap();
        assert_eq!(g.crs.as_deref(), Some("EPSG:4326"));
    }

    #[test]
    fn kml_and_gpx() {
        let k = kml(r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document><Placemark><name>A</name><ExtendedData><Data name="type"><value>tree</value></Data></ExtendedData><Point><coordinates>4.66,51.81,0</coordinates></Point></Placemark><Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>0,0 1,0 1,1 0,0</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark></Document></kml>"#).unwrap();
        assert_eq!(k.features.len(), 2);
        assert_eq!(k.features[0].props["type"], "tree");
        assert_eq!(k.features[1].lines.len(), 1);
        let g = gpx(r#"<gpx><wpt lat="51.8" lon="4.6"><name>W</name></wpt><trk><trkseg><trkpt lat="51.8" lon="4.6"/><trkpt lat="51.9" lon="4.7"/></trkseg></trk></gpx>"#).unwrap();
        assert_eq!(g.features.len(), 2);
        assert_eq!(g.features[1].lines[0].0.len(), 2);
    }

    #[test]
    fn wkb_polygon_in_gpkg_blob() {
        // GP header, no envelope, then little-endian WKB polygon with one ring.
        let mut b = vec![b'G', b'P', 0, 0b0000_0001, 0, 0, 0, 0];
        b.push(1);
        b.extend(3u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(4u32.to_le_bytes());
        for (x, y) in [(0.0f64, 0.0f64), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)] {
            b.extend(x.to_le_bytes());
            b.extend(y.to_le_bytes());
        }
        let mut f = Feature::default();
        assert!(gpkg_geometry(&b, &mut f));
        assert_eq!(f.lines.len(), 1);
        assert_eq!(f.lines[0].0.len(), 4);
    }

    #[test]
    fn test_files_read() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/gisbestanden");
        let shp = read(&dir.join("zurich_lines.shp"), None).unwrap();
        assert_eq!(shp.features.len(), 2);
        assert_eq!(shp.crs.as_deref(), Some("EPSG:2056"));
        assert_eq!(shp.features[0].props["NAME"], "Track west");
        assert_eq!(shp.features[1].lines[0].0.len(), 3);
        let gpkg = read(&dir.join("zurich.gpkg"), None).unwrap();
        assert_eq!(gpkg.features.len(), 2);
        assert_eq!(gpkg.crs.as_deref(), Some("EPSG:2056"));
        assert_eq!(gpkg.features[0].props["number"], "P-1");
        assert_eq!(gpkg.features[0].lines[0].0.len(), 5);
        for name in ["zurich.geojson", "zurich.gpx", "zurich.kml", "zurich_lv95.csv"] {
            let d = read(&dir.join(name), None).unwrap();
            assert!(!d.features.is_empty(), "{name}");
        }
    }

    #[test]
    fn gml_features() {
        let g = gml(r#"<wfs:FeatureCollection xmlns:wfs="http://www.opengis.net/wfs/2.0" xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:x="urn:x"><wfs:member><x:pipe gml:id="p1"><x:name>A</x:name><x:diameter>300</x:diameter><x:geom><gml:LineString srsName="urn:ogc:def:crs:EPSG::28992"><gml:posList>1 2 3 4</gml:posList></gml:LineString></x:geom></x:pipe></wfs:member><wfs:member><x:pit gml:id="q1"><x:geom><gml:Point><gml:pos>5 6</gml:pos></gml:Point></x:geom></x:pit></wfs:member></wfs:FeatureCollection>"#).unwrap();
        assert_eq!(g.crs.as_deref(), Some("EPSG:28992"));
        assert_eq!(g.features.len(), 2);
        assert_eq!(g.features[0].lines[0].0, vec![[1.0, 2.0], [3.0, 4.0]]);
        assert_eq!(g.features[0].props["name"], "A");
        assert_eq!(g.features[1].points[0], [5.0, 6.0]);
        assert_eq!(kind(Path::new("a.tif")), Kind::Raster);
        assert_eq!(kind(Path::new("a.GML")), Kind::Vector);
    }

    #[test]
    fn geojson_crs_member() {
        let d = geojson(br#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::28992"}},"features":[{"type":"Feature","properties":{"a":1},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#).unwrap();
        assert_eq!(d.crs.as_deref(), Some("EPSG:28992"));
        assert_eq!(d.features[0].points[0], [1.0, 2.0]);
    }
}
