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
    /// Read diagnostics, kept separate from object attributes.
    pub warnings: Vec<String>,
    /// Source objects omitted while keeping the rest of the file readable.
    pub skipped: usize,
}

impl FileData {
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
        // Bare coordinates without a header row: a point cloud (XYZ in .txt).
        "txt" if super::pointcloud::txt_is_points(path) => Kind::PointCloud,
        e if EXTENSIONS.contains(&e) || e == "dbf" || e == "shx" || e == "xml" => Kind::Vector,
        "tif" | "tiff" | "asc" | "jpg" | "jpeg" | "png" | "jp2" => Kind::Raster,
        "las" | "laz" | "e57" | "ply" | "pts" | "xyz" => Kind::PointCloud,
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
    Ok(FileData { format: "GeoJSON", features: vector::parse_geojson(&v), crs, layers: Vec::new(), warnings: Vec::new(), skipped: 0 })
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
    Ok(FileData { format: "Shapefile", features, crs, layers: Vec::new(), warnings: Vec::new(), skipped: 0 })
}

// ── WKB (GeoPackage) ────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GpkgGeometryError {
    Corrupt,
    Unsupported(u32),
    UnsupportedEncoding,
}

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
    fn geometry(&mut self, f: &mut Feature) -> Result<Option<u32>, GpkgGeometryError> {
        use GpkgGeometryError::{Corrupt, Unsupported};
        // WKB stores children immediately after each collection header. A
        // count of pending geometries consumes that preorder stream without
        // recursion or a fixed nesting limit.
        let mut pending = 1usize;
        let mut unsupported = None;
        while pending > 0 {
            pending -= 1;
            let le = match self.u8().ok_or(Corrupt)? { 0 => false, 1 => true, _ => return Err(Corrupt) };
            let raw = self.u32(le).ok_or(Corrupt)?;
            let (base, dims) = match raw {
                r if r & 0x8000_0000 != 0 || r & 0x4000_0000 != 0 => (r & 0xFFFF, 2 + (r & 0x8000_0000 != 0) as usize + (r & 0x4000_0000 != 0) as usize),
                r if r >= 3000 => (r - 3000, 4),
                r if r >= 2000 => (r - 2000, 3),
                r if r >= 1000 => (r - 1000, 3),
                r => (r, 2),
            };
            let point = |w: &mut Self| -> Option<XY> {
                let x = w.f64(le)?;
                let y = w.f64(le)?;
                for _ in 2..dims { w.f64(le)?; }
                Some([x, y])
            };
            let line = |w: &mut Self| -> Option<Vec<XY>> {
                let n = w.u32(le)? as usize;
                if n.checked_mul(dims * 8)? > w.b.len().saturating_sub(w.i) { return None; }
                let points: Vec<XY> = (0..n).map(|_| point(w)).collect::<Option<_>>()?;
                points.iter().flatten().all(|v| v.is_finite()).then_some(points)
            };
            match base {
                1 => {
                    let p = point(self).ok_or(Corrupt)?;
                    if p[0].is_finite() && p[1].is_finite() { f.points.push(p); }
                    else if !p.iter().all(|v| v.is_nan()) { return Err(Corrupt); }
                }
                2 => f.lines.push((line(self).ok_or(Corrupt)?, false)),
                3 => {
                    let rings = self.u32(le).ok_or(Corrupt)? as usize;
                    if rings > self.b.len().saturating_sub(self.i) / 4 { return Err(Corrupt); }
                    for _ in 0..rings { f.lines.push((line(self).ok_or(Corrupt)?, true)); }
                }
                4..=7 => {
                    let count = self.u32(le).ok_or(Corrupt)? as usize;
                    if count > self.b.len().saturating_sub(self.i) / 5 { return Err(Corrupt); }
                    pending = pending.checked_add(count).ok_or(Corrupt)?;
                }
                8 => {
                    // CircularString uses a point array. Validate its bytes
                    // but do not turn a curve into straight segments. Empty
                    // curves preserve their record and optional attributes.
                    if !line(self).ok_or(Corrupt)?.is_empty() { unsupported.get_or_insert(base); }
                }
                _ => return Err(Unsupported(base)),
            }
        }
        Ok(unsupported)
    }
}

/// A GeoPackage geometry blob: the `GP` header (with an optional envelope),
/// then standard WKB.
pub fn gpkg_geometry(blob: &[u8], f: &mut Feature) -> bool {
    gpkg_geometry_result(blob, f).is_ok()
}

fn gpkg_geometry_result(blob: &[u8], f: &mut Feature) -> Result<(), GpkgGeometryError> {
    use GpkgGeometryError::{Corrupt, Unsupported, UnsupportedEncoding};
    let mut extended = false;
    let bytes = if blob.starts_with(b"GP") {
        if blob.len() < 8 || blob[2] != 0 || blob[3] & 0b1100_0000 != 0 { return Err(Corrupt); }
        extended = blob[3] & 0x20 != 0;
        let envelope = match (blob[3] >> 1) & 0b111 {
            0 => 0,
            1 => 32,
            2 | 3 => 48,
            4 => 64,
            _ => return Err(Corrupt),
        };
        blob.get(8 + envelope..).ok_or(Corrupt)?
    } else { blob };
    let mut parsed = Feature::default();
    let mut wkb = Wkb { b: bytes, i: 0 };
    let kind = match wkb.geometry(&mut parsed) {
        Err(Corrupt) if extended => return Err(UnsupportedEncoding),
        result => result?,
    };
    // Legacy extended headers with fully readable WKB keep working. An
    // unknown extended payload is unsupported; standard WKB must be complete.
    if wkb.i != bytes.len() { return Err(if extended { UnsupportedEncoding } else { Corrupt }); }
    if let Some(kind) = kind { return Err(Unsupported(kind)); }
    f.points.extend(parsed.points);
    f.lines.extend(parsed.lines);
    Ok(())
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
    let mut warnings = Vec::new();
    let mut row_index = 0;
    'rows: while let Some(row) = rows.next().map_err(err)? {
        row_index += 1;
        let mut f = Feature::default();
        for (i, n) in names.iter().enumerate() {
            let v = row.get_ref(i).map_err(err)?;
            if i == gi {
                if let ValueRef::Blob(b) = v {
                    if let Err(error) = gpkg_geometry_result(b, &mut f) {
                        let reason = match error {
                            GpkgGeometryError::Corrupt => "onleesbare geometrie".to_string(),
                            GpkgGeometryError::Unsupported(kind) => format!("niet ondersteunde geometriesoort {kind}"),
                            GpkgGeometryError::UnsupportedEncoding => "niet ondersteunde uitgebreide geometrie".to_string(),
                        };
                        warnings.push(format!("GeoPackage: {reason} in laag {table}, rij {row_index}; object overgeslagen"));
                        continue 'rows;
                    }
                } else if !matches!(v, ValueRef::Null) {
                    warnings.push(format!("GeoPackage: geometrie is geen blob in laag {table}, rij {row_index}; object overgeslagen"));
                    continue 'rows;
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
    let skipped = warnings.len();
    Ok(FileData { format: "GeoPackage", features, crs, layers, warnings, skipped })
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
    Ok(FileData { format: "KML", features, crs: Some("EPSG:4326".into()), layers: Vec::new(), warnings: Vec::new(), skipped: 0 })
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
    Ok(FileData { format: "GPX", features, crs: Some("EPSG:4326".into()), layers: Vec::new(), warnings: Vec::new(), skipped: 0 })
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

// XML comments can split a simple value into several text nodes. Joining
// without added whitespace preserves the XML value, including split numbers.
fn gml_text(node: roxmltree::Node) -> String {
    node.children().filter(|n| n.is_text()).filter_map(|n| n.text()).collect()
}

fn gml_coords(node: roxmltree::Node, dim: usize) -> Result<Vec<XY>, String> {
    // GML 3: inherit the nearest dimension from the coordinate or geometry.
    if let Some(list) = node.descendants().find(|d| d.has_tag_name("posList")) {
        let d = gml_dimension(list, dim)?;
        let v = gml_numbers(gml_text(list).trim(), None)?;
        if v.len() % d != 0 { return Err("GML: onvolledige coördinaat in posList".into()); }
        return Ok(v.chunks_exact(d).map(|c| [c[0], c[1]]).collect());
    }
    let mut pos = Vec::new();
    for p in node.descendants().filter(|d| d.has_tag_name("pos")) {
        let v = gml_numbers(gml_text(p).trim(), None)?;
        let d = gml_dimension(p, v.len())?;
        if d < 2 || v.len() != d { return Err("GML: onvolledige coördinaat in pos".into()); }
        pos.push([v[0], v[1]]);
    }
    if !pos.is_empty() { return Ok(pos); }
    // GML 2: x,y[,z] tuples, separated by whitespace.
    if let Some(c) = node.descendants().find(|d| d.has_tag_name("coordinates")) {
        let mut dimension = None;
        return gml_text(c).split_whitespace().map(|tuple| {
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
fn gml_curve(root: roxmltree::Node) -> Result<Vec<XY>, String> {
    let mut pending = vec![root];
    let mut points = Vec::new();
    while let Some(node) = pending.pop() {
        let name = node.tag_name().name();
        if matches!(name, "LineString" | "LineStringSegment" | "LinearRing") {
            let next = gml_coords(node, 2)?;
            if next.len() < if name == "LinearRing" { 3 } else { 2 } { return Err(format!("GML: te weinig coördinaten in {name}")); }
            let shared = points.last().is_some_and(|p| *p == next[0]);
            if !points.is_empty() && !shared { return Err("GML: curveonderdelen sluiten niet aan".into()); }
            points.extend(next.into_iter().skip(usize::from(shared)));
            continue;
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
        // XML nodes form an acyclic tree. An explicit stack keeps arbitrary
        // nesting off the Rust call stack without imposing a schema limit.
        pending.extend(parts.into_iter().rev());
    }
    Ok(points)
}

/// The geometry elements directly under a property, read into `f`.
fn gml_geometry(node: roxmltree::Node, f: &mut Feature) -> Result<bool, String> {
    let mut pending = vec![node];
    let mut recognized = false;
    while let Some(node) = pending.pop() {
        let name = node.tag_name().name();
        match name {
            "Point" => {
                let points = gml_coords(node, 2)?;
                if points.len() != 1 { return Err("GML: Point verwacht één coördinaat".into()); }
                f.points.extend(points);
            }
            "LineString" | "Curve" | "LineStringSegment" | "LinearRing" | "Ring" | "CompositeCurve" => {
                let ring = matches!(name, "LinearRing" | "Ring");
                let points = gml_curve(node)?;
                if ring && points.len() < 3 { return Err(format!("GML: te weinig coördinaten in {name}")); }
                if !points.is_empty() { f.lines.push((points, ring)); }
            }
            "Polygon" | "Surface" | "PolygonPatch" => {
                let rings: Vec<_> = node.descendants().filter(|d| d.has_tag_name("LinearRing") || d.has_tag_name("Ring")).collect();
                pending.extend(rings.into_iter().rev());
            }
            "MultiPoint" | "MultiCurve" | "MultiLineString" | "MultiSurface" | "MultiPolygon" | "MultiGeometry" | "CompositeSurface" => {
                // pointMember / surfaceMembers: the geometries one level down.
                let parts: Vec<_> = node.children().filter(|c| c.is_element()).flat_map(|m| m.children().filter(|c| c.is_element())).collect();
                pending.extend(parts.into_iter().rev());
            }
            _ => continue,
        }
        recognized = true;
    }
    Ok(recognized)
}

/// Features from a GML file: every member of a feature collection (or of a
/// WFS response) with its geometry and its simple properties. The CRS comes
/// from the first `srsName`.
pub fn gml(text: &str) -> Result<FileData, String> {
    // roxmltree's tokenizer is recursive. Measure nesting with a streaming
    // parser first, then provide enough stack for deep valid XML instead of
    // rejecting it at a fixed depth or overflowing the caller's stack.
    let invalid = || crate::lang::tr("geen tekenbare objecten in het bestand", "no drawable features in the file").to_string();
    let mut reader = quick_xml::Reader::from_str(text);
    let (mut depth, mut deepest) = (0usize, 0usize);
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(_)) => { depth += 1; deepest = deepest.max(depth); }
            Ok(quick_xml::events::Event::End(_)) => depth = depth.saturating_sub(1),
            // Let the original XML parser report syntax errors, on the stack
            // needed for the prefix already measured, preserving its details.
            Ok(quick_xml::events::Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if deepest <= 64 { return gml_inner(text); }
    let stack = deepest.checked_mul(32 * 1024).and_then(|size| size.checked_add(2 * 1024 * 1024)).ok_or_else(invalid)?;
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new().stack_size(stack).spawn_scoped(scope, || gml_inner(text)).map_err(|_| invalid())?;
        handle.join().map_err(|_| invalid())?
    })
}

fn gml_inner(text: &str) -> Result<FileData, String> {
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
    let skipped = warnings.len();
    Ok(FileData { format: "GML", features, crs, layers: Vec::new(), warnings, skipped })
}

// ── CSV ─────────────────────────────────────────────────────────────────────

struct CsvRecord {
    end: usize,
    lines: usize,
    fields: Vec<String>,
    invalid: bool,
}

/// One decoder for complete records and physical-line recovery. Recovery
/// keeps main's permissive fields, but never joins two physical lines.
fn csv_record(text: &str, sep: char, physical: bool) -> CsvRecord {
    let mut out = CsvRecord { end: text.len(), lines: 0, fields: Vec::new(), invalid: false };
    let mut cur = String::new();
    let mut quoted = false;
    let mut quoted_field = false;
    let field = |fields: &mut Vec<String>, cur: &mut String, quoted: bool| {
        let value = std::mem::take(cur);
        fields.push(if quoted { value } else { value.trim().to_string() });
    };
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '\r' | '\n') {
            let end = if c == '\r' && chars.peek().is_some_and(|(_, c)| *c == '\n') {
                chars.next(); i + 2
            } else { i + 1 };
            out.lines += 1;
            if !quoted || physical {
                field(&mut out.fields, &mut cur, quoted_field);
                out.end = end;
                return out;
            }
            cur.push_str(&text[i..end]);
            continue;
        }
        match c {
            '"' if quoted && chars.peek().is_some_and(|(_, c)| *c == '"') => {
                cur.push('"');
                chars.next();
            }
            '"' => {
                if !physical {
                    let next = chars.clone().map(|(_, c)| c)
                        .find(|c| !c.is_whitespace() || matches!(*c, '\r' | '\n') || *c == sep);
                    let invalid_close = quoted && next.is_some_and(|c| c != sep && !matches!(c, '\r' | '\n'));
                    let invalid_open = !quoted && !cur.trim().is_empty();
                    if invalid_close || invalid_open {
                        out.end = i;
                        out.invalid = true;
                        return out;
                    }
                }
                if !quoted && cur.trim().is_empty() { quoted_field = true; cur.clear(); }
                quoted = !quoted;
            }
            c if c == sep && !quoted => {
                field(&mut out.fields, &mut cur, quoted_field);
                quoted_field = false;
            }
            c if quoted_field && !quoted && c.is_whitespace() => {}
            c => cur.push(c),
        }
    }
    field(&mut out.fields, &mut cur, quoted_field);
    out.invalid = quoted && !physical;
    out
}

/// Complete records with their physical start line. Keep newlines inside
/// quoted fields verbatim. Malformed records are replayed by the same
/// decoder one physical line at a time, preserving later readable rows.
fn csv_records(text: &str, sep: char, warnings: &mut Vec<String>) -> Vec<(usize, Vec<String>)> {
    let mut records = Vec::new();
    let (mut start, mut line) = (0usize, 1usize);
    while start < text.len() {
        let tail = &text[start..];
        let record = csv_record(tail, sep, false);
        if !record.invalid {
            if !tail[..record.end].trim().is_empty() { records.push((line, record.fields)); }
            line += record.lines;
            start += record.end;
            continue;
        }
        warnings.push(format!("CSV: {} {line}; {}",
            crate::lang::tr("niet afgesloten aangehaald veld vanaf regel", "unterminated quoted field starting on line"),
            crate::lang::tr("resterende regels afzonderlijk gelezen", "remaining lines read separately")));
        // Recover only the first physical line. The following record may
        // contain valid quoted newlines and must use normal decoding again.
        let physical = csv_record(tail, sep, true);
        if !tail[..physical.end].trim().is_empty() { records.push((line, physical.fields)); }
        line += physical.lines;
        start += physical.end;
    }
    records
}

fn number(s: &str) -> Option<f64> {
    let t = s.trim();
    t.parse().ok().or_else(|| if t.matches(',').count() == 1 && !t.contains('.') { t.replace(',', ".").parse().ok() } else { None })
}

/// Points from a table with coordinate columns: x/y, easting/northing,
/// lon/lat (also Dutch names). `;`, tab or `,` separated.
pub fn csv(text: &str) -> Result<FileData, String> {
    let head = text.lines().find(|line| !line.trim().is_empty()).ok_or("CSV: empty file")?;
    let sep = [';', '\t', ','].into_iter().max_by_key(|s| head.matches(*s).count()).unwrap_or(',');
    let mut warnings = Vec::new();
    let mut records = csv_records(text, sep, &mut warnings).into_iter();
    let (_, head) = records.next().ok_or("CSV: empty file")?;
    let cols: Vec<String> = head.into_iter().map(|c| c.trim().trim_start_matches('\u{feff}').to_string()).collect();
    let find = |names: &[&str]| cols.iter().position(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)));
    let (ix, iy, geographic) = if let (Some(x), Some(y)) = (find(&["lon", "lng", "long", "longitude", "lengtegraad"]), find(&["lat", "latitude", "breedtegraad"])) {
        (x, y, true)
    } else if let (Some(x), Some(y)) = (find(&["x", "easting", "east", "e", "x_rd", "rd_x", "xcoord", "x_coord"]), find(&["y", "northing", "north", "n", "y_rd", "rd_y", "ycoord", "y_coord"])) {
        (x, y, false)
    } else {
        return Err(format!("CSV: no coordinate columns (x/y, easting/northing or lon/lat) in {}", cols.join(", ")));
    };
    let mut features = Vec::new();
    let mut skipped = 0;
    for (line_number, vals) in records {
        let coordinate = |index: usize| {
            vals.get(index).and_then(|v| number(v)).filter(|v| v.is_finite()).ok_or_else(|| format!("CSV: {} {line_number}: {} {}; {}", crate::lang::tr("regel", "line"), crate::lang::tr("ontbrekende of ongeldige coördinaat in kolom", "missing or invalid coordinate in column"), cols[index], crate::lang::tr("rij overgeslagen", "row skipped")))
        };
        let (x, y) = match coordinate(ix).and_then(|x| coordinate(iy).map(|y| (x, y))) {
            Ok(xy) => xy,
            Err(error) => { skipped += 1; warnings.push(error); continue; }
        };
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
    Ok(FileData { format: "CSV", features, crs: geographic.then(|| "EPSG:4326".to_string()), layers: Vec::new(), warnings, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_csv_recovery_keeps_a_multiline_successor(source: &str) {
        let expected_points = vec![[100.0, 200.0], [110.0, 220.0], [120.0, 240.0]];
        let mut failures = Vec::new();
        for separator in [',', ';', '\t'] {
            for newline in ["\n", "\r\n"] {
                let text = source.replace(',', &separator.to_string()).replace('\n', newline);
                let data = match csv(&text) {
                    Ok(data) => data,
                    Err(error) => { failures.push(format!("{separator:?}/{newline:?}: {error}")); continue; }
                };
                let points = data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>();
                let note = data.features.get(1).and_then(|f| f.props.get("note")).and_then(Value::as_str);
                let expected_note = format!("put{newline}west");
                if points != expected_points || data.skipped != 1 || note != Some(expected_note.as_str()) {
                    failures.push(format!("{separator:?}/{newline:?}: points={points:?}, skipped={}, note={note:?}", data.skipped));
                }
            }
        }
        // Collect every variant before failing, so the negative control
        // proves both line endings and all three separators independently.
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn sol_csv_note_first_recovery_preserves_the_next_multiline_record() {
        assert_csv_recovery_keeps_a_multiline_successor(
            "note,x,y\ngoed,100,200\n\"onaf,bad,210\n\"put\nwest\",110,220\neind,120,240");
    }

    #[test]
    fn sol_csv_coordinate_first_recovery_preserves_the_next_multiline_note() {
        assert_csv_recovery_keeps_a_multiline_successor(
            "x,y,note\n100,200,goed\nbad,210,\"onaf\n110,220,\"put\nwest\"\n120,240,eind");
    }

    #[test]
    fn sol_csv_broken_note_with_bad_coordinates_keeps_both_later_rows() {
        let text = "x,y,note\n100,200,goed\nbad,210,\"onaf\n110,220,\"normaal\"\n120,240,goed";
        let data = csv(text).unwrap();
        assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(),
            vec![[100.0, 200.0], [110.0, 220.0], [120.0, 240.0]]);
        assert_eq!(data.skipped, 1);
        assert_eq!(data.features[1].props["note"], "normaal");
        assert!(!data.warnings.is_empty());
    }

    #[test]
    fn sol_csv_broken_note_with_valid_coordinates_keeps_every_point() {
        let text = "x,y,note\n100,200,goed\n105,210,\"onaf\n110,220,\"normaal\"\n120,240,goed";
        let data = csv(text).unwrap();
        assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(),
            vec![[100.0, 200.0], [105.0, 210.0], [110.0, 220.0], [120.0, 240.0]]);
        assert_eq!(data.skipped, 0);
        assert_eq!(data.features[1].props["note"], "onaf");
        assert_eq!(data.features[2].props["note"], "normaal");
        assert!(!data.warnings.is_empty());
    }

    #[test]
    fn sol_csv_multiline_note_preserves_both_points_full_text_and_no_warnings() {
        let text = "x,y,note\n100,200,\"put\nwest\"\n110,220,normaal";
        let data = csv(text).unwrap();
        assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(), vec![[100.0, 200.0], [110.0, 220.0]]);
        assert_eq!(data.features[0].props["note"], "put\nwest");
        assert_eq!(data.features[1].props["note"], "normaal");
        assert!(data.warnings.is_empty(), "geen onterechte waarschuwing voor west: {:?}", data.warnings);
    }

    #[test]
    fn csv_unterminated_note_diagnostic_does_not_count_a_skipped_point() {
        let data = csv("x,y,note\n100,200,\"unfinished\n110,220,normaal\n").unwrap();
        assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(), vec![[100.0, 200.0], [110.0, 220.0]]);
        assert_eq!(data.warnings.len(), 1);
        assert_eq!(data.skipped, 0);
    }

    #[test]
    fn csv_and_gml_keep_read_diagnostics_in_the_result() {
        let csv_data = csv("x,y\n100,200\nbad,220\n").unwrap();
        assert_eq!(csv_data.skipped, 1);
        assert_eq!(csv_data.warnings.len(), 1);

        let generic = gml(include_str!("../../tests/fixtures/gml/mixed.gml")).unwrap();
        assert_eq!((generic.features.len(), generic.skipped, generic.warnings.len()), (1, 1, 1));
    }

    #[test]
    fn imro_unreadable_geometry_diagnostics_stay_in_the_read_result() {
        let text = include_str!("../../tests/fixtures/planning/synthetic-imro.gml").replace("<imro:geometrie xlink:href=\"#polygon\"/>", "");
        let data = gml(&text).unwrap();
        assert_eq!(data.format, "IMRO");
        assert!(!data.features.is_empty());
        assert!(data.warnings.iter().any(|w| w == "zone: geen tekenbare geometrie"));
    }

    const GML: &str = include_str!("../../tests/fixtures/gml/features.gml");

    fn with_first_gml_line(geometry: &str) -> String {
        let doc = roxmltree::Document::parse(GML).unwrap();
        let line = doc.descendants().find(|n| n.has_tag_name("LineString")).unwrap();
        let mut text = GML.to_string();
        text.replace_range(line.range(), geometry);
        text
    }

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
    fn gml_skips_invalid_or_incomplete_coordinate_tuples_with_internal_details() {
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
        let data = gml(&with_first_gml_line(ring)).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].lines[0], (vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 0.0]], true));
        assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn gml_empty_curve_container_does_not_hide_the_adjacent_point() {
        let text = with_first_gml_line("<gml:Curve><gml:segments/></gml:Curve>");
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
        assert_eq!(data.skipped, 1);
    }

    #[test]
    fn gml_bad_line_between_two_points_is_skipped_and_counted() {
        let text = r#"<gml:FeatureCollection xmlns:gml="http://www.opengis.net/gml" xmlns:t="urn:test">
          <gml:featureMember><t:point gml:id="p1"><t:geometry><gml:Point><gml:pos>0 0</gml:pos></gml:Point></t:geometry></t:point></gml:featureMember>
          <gml:featureMember><t:line gml:id="broken"><t:geometry><gml:LineString><gml:posList>1 2 bad 3 4</gml:posList></gml:LineString></t:geometry></t:line></gml:featureMember>
          <gml:featureMember><t:point gml:id="p2"><t:geometry><gml:Point><gml:pos>10 10</gml:pos></gml:Point></t:geometry></t:point></gml:featureMember>
        </gml:FeatureCollection>"#;
        let data = gml(text).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].points, vec![[0.0, 0.0]]);
        assert_eq!(data.features[1].points, vec![[10.0, 10.0]]);
        assert_eq!(data.skipped, 1);
    }

    #[test]
    fn gml_projects_pos_in_any_complete_dimension_and_trims_the_attribute() {
        for (dimension, values) in [("4", "1 2 3 4"), (" 2 ", "1 2"), ("5", "1 2 3 4 5")] {
            let text = GML.replace("<gml:Point>", &format!("<gml:Point srsDimension=\"{dimension}\">"))
                .replace("2600005 1200005", values);
            assert_ne!(text, GML);
            let data = gml(&text).unwrap();
            assert_eq!(data.features.len(), 2);
            assert_eq!(data.features[1].points, vec![[1.0, 2.0]]);
            assert_eq!(data.skipped, 0);
        }
    }

    #[test]
    fn gml_deep_curves_and_collections_keep_the_geometry_read_by_main() {
        for depth in [65, 256] {
            for (container, member) in [("CompositeCurve", "curveMember"), ("MultiGeometry", "geometryMember")] {
                let mut geometry = "<gml:LineString><gml:posList>0 0 10 0</gml:posList></gml:LineString>".to_string();
                for _ in 0..depth {
                    geometry = format!("<gml:{container}><gml:{member}>{geometry}</gml:{member}></gml:{container}>");
                }
                let data = gml(&with_first_gml_line(&geometry)).unwrap();
                assert_eq!(data.features.len(), 2, "{container}, depth {depth}");
                assert_eq!(data.features[0].lines[0], (vec![[0.0, 0.0], [10.0, 0.0]], false));
                assert_eq!(data.features[1].points, vec![[2600005.0, 1200005.0]]);
                assert_eq!(data.skipped, 0);
            }
        }
    }

    #[test]
    fn gml_coordinate_comments_preserve_every_text_part() {
        let text = GML.replace("2600000 1200000 5 2600010 1200020 6", "2600000 1200000 5<!--comment--> 2600010 1200020 6");
        let data = gml(&text).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].lines[0].0, vec![[2600000.0, 1200000.0], [2600010.0, 1200020.0]]);
        assert_eq!(data.skipped, 0);
    }

    #[test]
    fn gml_bad_number_after_a_comment_cannot_hide_behind_a_valid_prefix() {
        let text = GML.replace("2600000 1200000 5 2600010 1200020 6", "2600000 1200000 5 2600010 1200020 6<!--comment--> BAD");
        let data = gml(&text).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
        assert_eq!(data.skipped, 1);
    }

    #[test]
    fn gml_linear_segments_are_joined_and_unsupported_parts_are_reported() {
        let curve = "<gml:Curve><gml:segments><gml:LineStringSegment><gml:posList>0 0 10 0</gml:posList></gml:LineStringSegment><gml:LineStringSegment><gml:posList>10 0 10 10</gml:posList></gml:LineStringSegment></gml:segments></gml:Curve>";
        let data = gml(&with_first_gml_line(curve)).unwrap();
        assert_eq!(data.features[0].lines[0], (vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false));
        assert!(data.warnings.is_empty());
        for curve in [curve.replace("LineStringSegment", "Arc"), curve.replace("10 0 10 10", "20 0 10 10")] {
            let data = gml(&with_first_gml_line(&curve)).unwrap();
            assert_eq!(data.features.len(), 1);
            assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
            assert_eq!(data.warnings.len(), 1);
        }
    }

    #[test]
    fn gml_file_import_counts_a_skipped_feature_and_keeps_the_good_one() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gml/mixed.gml");
        let data = read(&path, None).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
        assert_eq!(data.warnings.len(), 1);
        assert_eq!(data.skipped, 1);
        assert!(data.warnings[0].contains("broken-line"));
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

    const WKB_POINT: &[u8] = include_bytes!("../../tests/fixtures/geopackage/point.wkb");
    const WKB_LINE: &[u8] = include_bytes!("../../tests/fixtures/geopackage/line.wkb");

    fn gpkg_blob(wkb: &[u8]) -> Vec<u8> {
        let mut blob = vec![b'G', b'P', 0, 1];
        blob.extend(2056i32.to_le_bytes());
        blob.extend(wkb);
        blob
    }

    fn temporary_gpkg(name: &str, blob: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("oi-gpkg-{name}-{}.gpkg", std::process::id()));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("DROP TABLE IF EXISTS gpkg_contents; DROP TABLE IF EXISTS gpkg_geometry_columns; DROP TABLE IF EXISTS gpkg_spatial_ref_sys; DROP TABLE IF EXISTS sample;
            CREATE TABLE gpkg_contents (table_name TEXT, data_type TEXT);
            CREATE TABLE gpkg_geometry_columns (table_name TEXT, column_name TEXT, srs_id INTEGER);
            CREATE TABLE gpkg_spatial_ref_sys (srs_id INTEGER, organization TEXT, organization_coordsys_id INTEGER);
            CREATE TABLE sample (id INTEGER, geom BLOB, name TEXT);
            INSERT INTO gpkg_contents VALUES ('sample','features');
            INSERT INTO gpkg_geometry_columns VALUES ('sample','geom',2056);
            INSERT INTO gpkg_spatial_ref_sys VALUES (2056,'EPSG',2056);").unwrap();
        db.execute("INSERT INTO sample VALUES (1,?1,'synthetic')", [blob]).unwrap();
        path
    }

    fn circular_string() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/geopackage/circular.wkb").to_vec()
    }

    fn broken_collection() -> Vec<u8> {
        let mut wkb = vec![1];
        wkb.extend(7u32.to_le_bytes());
        wkb.extend(2u32.to_le_bytes());
        wkb.extend(WKB_POINT);
        wkb.extend(&WKB_LINE[..10]);
        wkb
    }

    fn mixed_layer_gpkg(name: &str, middle: &[u8]) -> std::path::PathBuf {
        let path = temporary_gpkg(name, &gpkg_blob(WKB_LINE));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("DROP TABLE IF EXISTS a_mix; DROP TABLE IF EXISTS b_good;
            ALTER TABLE sample RENAME TO a_mix;
            UPDATE gpkg_contents SET table_name = 'a_mix';
            UPDATE gpkg_geometry_columns SET table_name = 'a_mix';
            CREATE TABLE b_good (id INTEGER, geom BLOB, name TEXT);
            INSERT INTO gpkg_contents VALUES ('b_good','features');
            INSERT INTO gpkg_geometry_columns VALUES ('b_good','geom',2056);").unwrap();
        db.execute("INSERT INTO a_mix VALUES (2,?1,'middle')", [&gpkg_blob(middle)]).unwrap();
        db.execute("INSERT INTO a_mix VALUES (3,?1,'point')", [&gpkg_blob(WKB_POINT)]).unwrap();
        db.execute("INSERT INTO b_good VALUES (1,?1,'healthy')", [&gpkg_blob(WKB_POINT)]).unwrap();
        path
    }

    fn assert_readable_neighbours_and_layers(path: &Path) {
        let mixed = read(path, None).unwrap();
        assert_eq!(mixed.layers, vec!["a_mix", "b_good"]);
        let line = mixed.features.iter().find(|f| f.props["id"] == 1).unwrap();
        let point = mixed.features.iter().find(|f| f.props["id"] == 3).unwrap();
        let mut expected = Feature::default();
        assert!(gpkg_geometry(WKB_LINE, &mut expected));
        assert_eq!(line.lines, expected.lines);
        assert_eq!(point.points, vec![[2600000.25, 1200000.5]]);
        let healthy = read(path, Some("b_good")).unwrap();
        assert_eq!(healthy.layers, vec!["a_mix", "b_good"]);
        assert_eq!(healthy.features.len(), 1);
        assert_eq!(healthy.features[0].points, point.points);
        assert_eq!(healthy.crs.as_deref(), Some("EPSG:2056"));
    }

    #[test]
    fn sol_gpkg_circular_string_keeps_readable_rows_and_other_layer() {
        let path = mixed_layer_gpkg("sol-curve", &circular_string());
        assert_readable_neighbours_and_layers(&path);
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("geometriesoort 8"));
        assert!(!data.warnings[0].contains("onleesbaar"));
    }

    #[test]
    fn sol_gpkg_truncated_wkb_keeps_readable_rows_and_other_layer() {
        let path = mixed_layer_gpkg("sol-truncated", &WKB_LINE[..10]);
        assert_readable_neighbours_and_layers(&path);
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("onleesbare"));
    }

    #[test]
    fn sol_gpkg_broken_collection_keeps_readable_rows_and_other_layer() {
        let path = mixed_layer_gpkg("sol-collection", &broken_collection());
        assert_readable_neighbours_and_layers(&path);
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[1].points.len(), 1);
        assert_eq!(data.warnings.len(), 1);
    }

    #[test]
    fn gpkg_unreadable_default_layer_keeps_healthy_layer_selectable() {
        let path = mixed_layer_gpkg("empty-default", &broken_collection());
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("DELETE FROM a_mix WHERE id != 2", []).unwrap();
        drop(db);
        let data = read(&path, None).unwrap();
        assert!(data.features.is_empty());
        assert_eq!(data.layers, vec!["a_mix", "b_good"]);
        assert_eq!(read(&path, Some("b_good")).unwrap().features[0].points, vec![[2600000.25, 1200000.5]]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn sol_gpkg_65_nested_collections_preserve_the_point_read_by_main() {
        let mut wkb = Vec::new();
        for _ in 0..65 {
            wkb.push(1);
            wkb.extend(7u32.to_le_bytes());
            wkb.extend(1u32.to_le_bytes());
        }
        wkb.extend(WKB_POINT);
        let path = temporary_gpkg("sol-depth-65", &gpkg_blob(&wkb));
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600000.25, 1200000.5]]);
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn gpkg_many_nested_collections_do_not_use_the_call_stack() {
        let mut wkb = Vec::new();
        for _ in 0..5000 {
            wkb.push(1);
            wkb.extend(7u32.to_le_bytes());
            wkb.extend(1u32.to_le_bytes());
        }
        wkb.extend(WKB_POINT);
        let mut feature = Feature::default();
        assert!(gpkg_geometry(&wkb, &mut feature));
        assert_eq!(feature.points, vec![[2600000.25, 1200000.5]]);
    }

    #[test]
    fn gpkg_truncated_circular_string_is_corrupt_instead_of_unsupported() {
        let curve = circular_string();
        let path = mixed_layer_gpkg("truncated-curve", &curve[..curve.len() - 1]);
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("onleesbare geometrie"));
        assert!(!data.warnings[0].contains("niet ondersteunde"));
    }

    #[test]
    fn gpkg_unsupported_collection_does_not_commit_a_readable_prefix() {
        let mut wkb = vec![1];
        wkb.extend(7u32.to_le_bytes());
        wkb.extend(2u32.to_le_bytes());
        wkb.extend(WKB_POINT);
        wkb.extend(circular_string());
        let mut feature = Feature { points: vec![[1.0, 2.0]], ..Default::default() };
        assert!(!gpkg_geometry(&wkb, &mut feature));
        assert_eq!(feature.points, vec![[1.0, 2.0]]);
        assert!(feature.lines.is_empty());
    }

    #[test]
    fn gpkg_main_compatible_neighbours_and_layers_stay_available() {
        for (name, middle) in [("compat-curve", circular_string()), ("compat-truncated", WKB_LINE[..10].to_vec()), ("compat-collection", broken_collection())] {
            let path = mixed_layer_gpkg(name, &middle);
            assert_readable_neighbours_and_layers(&path);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn gpkg_fixed_wkb_and_file_route_preserve_geometry_and_crs() {
        for (wkb, points, lines) in [(WKB_POINT, 1, 0), (WKB_LINE, 0, 1)] {
            let mut f = Feature::default();
            assert!(gpkg_geometry(&gpkg_blob(wkb), &mut f));
            assert_eq!((f.points.len(), f.lines.len()), (points, lines));
        }
        let path = temporary_gpkg("valid", &gpkg_blob(WKB_POINT));
        let data = read(&path, None).unwrap();
        assert_eq!(data.crs.as_deref(), Some("EPSG:2056"));
        assert_eq!(data.features[0].points[0], [2600000.25, 1200000.5]);
        assert_eq!(data.features[0].props["name"], "synthetic");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn gpkg_preserves_big_endian_iso_dimensions_and_empty_points() {
        let mut big_endian = vec![0];
        big_endian.extend(1u32.to_be_bytes());
        big_endian.extend(12.0f64.to_be_bytes());
        big_endian.extend(34.0f64.to_be_bytes());
        let mut f = Feature::default();
        assert!(gpkg_geometry(&big_endian, &mut f));
        assert_eq!(f.points, vec![[12.0, 34.0]]);
        for kind in [1001u32, 2001, 3001, 0x8000_0001, 0x4000_0001] {
            let mut point = vec![1];
            point.extend(kind.to_le_bytes());
            point.extend(12.0f64.to_le_bytes());
            point.extend(34.0f64.to_le_bytes());
            point.extend(5.0f64.to_le_bytes());
            if kind == 3001 { point.extend(6.0f64.to_le_bytes()); }
            let mut f = Feature::default();
            assert!(gpkg_geometry(&point, &mut f), "{kind}");
            assert_eq!(f.points, vec![[12.0, 34.0]]);
        }
        let mut empty = WKB_POINT.to_vec();
        empty[5..13].copy_from_slice(&f64::NAN.to_le_bytes());
        empty[13..21].copy_from_slice(&f64::NAN.to_le_bytes());
        let mut f = Feature::default();
        assert!(gpkg_geometry(&empty, &mut f));
        assert!(f.points.is_empty());
    }

    #[test]
    fn gpkg_preserves_envelopes_and_valid_geometry_collections() {
        let mut envelope = vec![b'G', b'P', 0, 3];
        envelope.extend(2056i32.to_le_bytes());
        for bound in [2600000.0f64, 2600010.0, 1200000.0, 1200020.0] {
            envelope.extend(bound.to_le_bytes());
        }
        envelope.extend(WKB_POINT);
        let mut f = Feature::default();
        assert!(gpkg_geometry(&envelope, &mut f));
        assert_eq!(f.points.len(), 1);
        let mut collection = vec![1];
        collection.extend(7u32.to_le_bytes());
        collection.extend(2u32.to_le_bytes());
        collection.extend(WKB_POINT);
        collection.extend(WKB_LINE);
        let mut f = Feature::default();
        assert!(gpkg_geometry(&collection, &mut f));
        assert_eq!((f.points.len(), f.lines.len()), (1, 1));
    }

    #[test]
    fn gpkg_legacy_extended_flag_keeps_the_readable_point_from_main() {
        let mut blob = gpkg_blob(WKB_POINT);
        blob[3] |= 0x20;
        let mut feature = Feature::default();
        assert!(gpkg_geometry(&blob, &mut feature));
        assert_eq!(feature.points, vec![[2600000.25, 1200000.5]]);
    }

    #[test]
    fn gpkg_empty_circular_string_preserves_the_record_attributes() {
        let mut empty = vec![1];
        empty.extend(8u32.to_le_bytes());
        empty.extend(0u32.to_le_bytes());
        let mut blob = gpkg_blob(&empty);
        blob[3] |= 0x10;
        let path = temporary_gpkg("empty-circular", &blob);
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].props["name"], "synthetic");
        assert!(data.features[0].points.is_empty() && data.features[0].lines.is_empty());
        assert!(data.warnings.is_empty());
    }

    #[test]
    fn gpkg_sql_null_geometry_preserves_the_feature_and_attributes() {
        let path = temporary_gpkg("null", &gpkg_blob(WKB_POINT));
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute("UPDATE sample SET geom = NULL WHERE id = 1", []).unwrap();
        }
        let data = read(&path, None).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].props["name"], "synthetic");
        assert!(data.features[0].points.is_empty() && data.features[0].lines.is_empty());
        assert_eq!(data.crs.as_deref(), Some("EPSG:2056"));
    }

    #[test]
    fn gpkg_empty_geometries_remain_supported_with_the_empty_flag() {
        let mut point = WKB_POINT.to_vec();
        point[5..13].copy_from_slice(&f64::NAN.to_le_bytes());
        point[13..21].copy_from_slice(&f64::NAN.to_le_bytes());
        let mut empty = vec![point];
        for kind in 2..=7u32 {
            let mut wkb = vec![1];
            wkb.extend(kind.to_le_bytes());
            wkb.extend(0u32.to_le_bytes());
            empty.push(wkb);
        }
        for wkb in empty {
            let mut blob = gpkg_blob(&wkb);
            blob[3] |= 0x10;
            let mut f = Feature::default();
            assert!(gpkg_geometry(&blob, &mut f));
            assert!(f.points.is_empty() && f.lines.iter().all(|(line, _)| line.is_empty()));
        }
    }

    #[test]
    fn gpkg_rejects_bad_headers_byte_order_and_trailing_data() {
        let mut version = gpkg_blob(WKB_POINT);
        version[2] = 1;
        let mut envelope = gpkg_blob(WKB_POINT);
        envelope[3] = 1 | (5 << 1);
        let mut byte_order = vec![9];
        byte_order.extend(1u32.to_be_bytes());
        byte_order.extend(1.0f64.to_be_bytes());
        byte_order.extend(2.0f64.to_be_bytes());
        let mut trailing = WKB_POINT.to_vec();
        trailing.push(0);
        for blob in [version, envelope, byte_order, trailing] {
            assert!(!gpkg_geometry(&blob, &mut Feature::default()), "{blob:?}");
        }
    }

    #[test]
    fn gpkg_failure_does_not_leave_partial_geometry() {
        let mut collection = vec![1];
        collection.extend(7u32.to_le_bytes());
        collection.extend(2u32.to_le_bytes());
        collection.extend(WKB_POINT);
        collection.extend(&WKB_LINE[..10]);
        let mut f = Feature { points: vec![[1.0, 2.0]], ..Default::default() };
        assert!(!gpkg_geometry(&collection, &mut f));
        assert_eq!(f.points, vec![[1.0, 2.0]], "failed parse must be atomic");
        assert!(f.lines.is_empty());
    }

    #[test]
    fn gpkg_reports_corrupt_geometry_through_the_real_file_reader() {
        let path = temporary_gpkg("broken", &gpkg_blob(&WKB_LINE[..10]));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("INSERT INTO sample (id, geom) VALUES (2, ?1)", [&gpkg_blob(WKB_POINT)]).unwrap();
        drop(db);
        let result = read(&path, None);
        std::fs::remove_file(path).unwrap();
        let data = result.unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points.len(), 1);
        assert_eq!(data.features[0].props["id"], 2);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains("rij 1"));
        assert_eq!(data.skipped, 1);
    }

    #[test]
    fn gpkg_reads_deep_collections_and_rejects_nonfinite_lines() {
        let mut nested = WKB_POINT.to_vec();
        for _ in 0..100 {
            let mut outer = vec![1];
            outer.extend(7u32.to_le_bytes());
            outer.extend(1u32.to_le_bytes());
            outer.extend(nested);
            nested = outer;
        }
        assert!(gpkg_geometry(&nested, &mut Feature::default()));
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut line = WKB_LINE.to_vec();
            line[9..17].copy_from_slice(&value.to_le_bytes());
            assert!(!gpkg_geometry(&line, &mut Feature::default()));
        }
        let mut large = WKB_POINT.to_vec();
        large[5..13].copy_from_slice(&1e100f64.to_le_bytes());
        assert!(gpkg_geometry(&large, &mut Feature::default()));
    }

    #[test]
    fn gpkg_rejects_every_truncation_and_impossible_point_count() {
        let blob = gpkg_blob(WKB_LINE);
        for end in 0..blob.len() {
            assert!(!gpkg_geometry(&blob[..end], &mut Feature::default()), "{end}");
        }
        let mut line = WKB_LINE.to_vec();
        line[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(!gpkg_geometry(&line, &mut Feature::default()));
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

    #[test]
    fn csv_multiline_records_keep_the_points_read_by_main() {
        let text = include_str!("../../tests/fixtures/csv/multiline.csv");
        let data = csv(text).unwrap();
        assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(), vec![[100.0, 200.0], [110.0, 220.0]]);
    }

    #[test]
    fn csv_multiline_records_preserve_the_whole_quoted_attribute() {
        for newline in ["\n", "\r\n"] {
            let text = include_str!("../../tests/fixtures/csv/multiline.csv").replace('\n', newline);
            let data = csv(&text).unwrap();
            assert_eq!(data.features.len(), 2);
            assert_eq!(data.features[0].props["note"], format!("put{newline}west"));
            assert!(data.warnings.is_empty());
            let text = text.replace(&format!("put{newline}west"), &format!("put{newline}{newline}\"\"west\"\", vijf"));
            let data = csv(&text).unwrap();
            assert_eq!(data.features[0].props["note"], format!("put{newline}{newline}\"west\", vijf"));
            assert_eq!(data.features[1].points, vec![[110.0, 220.0]]);
            assert!(data.warnings.is_empty());
        }
    }

    #[test]
    fn csv_multiline_records_report_the_physical_start_line_of_a_bad_record() {
        let text = "\nx,y,note\n100,200,\"put\nwest\"\nbad,220,kapot\n110,220,normaal\n";
        let data = csv(text).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.warnings.len(), 1);
        assert!(data.warnings[0].contains(crate::lang::tr("regel 5", "line 5")));
    }

    #[test]
    fn csv_unterminated_quotes_keep_the_points_read_by_main() {
        for note in ["text\"", "\"unfinished"] {
            let data = csv(&format!("x,y,note\n100,200,{note}\n110,220,normaal\n")).unwrap();
            assert_eq!(data.features.iter().flat_map(|f| &f.points).copied().collect::<Vec<_>>(), vec![[100.0, 200.0], [110.0, 220.0]]);
        }
    }

    #[test]
    fn csv_fixture_preserves_decimal_commas_quotes_and_optional_fields() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/csv/points.csv");
        let data = read(&path, None).unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.features[0].points[0], [100.25, 200.5]);
        assert_eq!(data.features[1].props["type"], "boom; groot");
        assert_eq!(data.crs, None);
        assert_eq!(csv("lon,lat\n4.6,51.8\n").unwrap().crs.as_deref(), Some("EPSG:4326"));
        assert_eq!(csv("x;y;name\n1;2\n").unwrap().features[0].points[0], [1.0, 2.0]);
    }

    #[test]
    fn csv_keeps_good_rows_and_reports_bad_or_nonfinite_coordinates() {
        for row in ["p2;bad;220;tree", "p2;NaN;220;tree", "p2;110;inf;tree", "p2;1e999;220;tree", "p2;110", "p2;;220;tree"] {
            let text = format!("id;x;y;type\np1;100;200;put\n{row}\n");
            let data = csv(&text).unwrap();
            assert_eq!(data.features.len(), 1, "{row}");
            assert_eq!(data.features[0].points, vec![[100.0, 200.0]]);
            assert_eq!(data.warnings.len(), 1);
            assert!(data.warnings[0].starts_with("CSV:") && data.warnings[0].contains(crate::lang::tr("regel 3", "line 3")));
        }
    }

    #[test]
    fn csv_handles_empty_missing_columns_and_large_finite_coordinates() {
        for text in ["", "\n", "id;name\np1;tree\n"] {
            assert!(csv(text).is_err());
        }
        assert!(csv("x;y\n").unwrap().features.is_empty());
        let large = csv("x;y\n1e100;-1e100\n").unwrap();
        assert_eq!(large.features[0].points[0], [1e100, -1e100]);
    }
}
