//! The drawing's own geographic location: the GEODATA object that other CAD
//! programs write (GEOGRAPHICLOCATION, GEOMAP). Open Infra
//! reads it to set the project site of a drawing that has none yet.
//!
//! Open CAD Studio's plugin API cannot create objects other than XRecords
//! (HakanSeven12/OpenCADStudio#2303), so in the open drawing Open Infra keeps
//! its site in the `OPENINFRA/SITE` XRecord. `GIS_GEODATA` writes a copy of
//! the drawing (DWG or DXF) with a GEODATA object for the project location,
//! through the codec, for AutoCAD, Civil 3D and BricsCAD.

use super::crs;
use super::files::epsg_from_wkt;
use super::site::Site;

/// MapGuide coordinate system names in DWG GEODATA, with their EPSG codes.
const MAPGUIDE: &[(&str, &str)] = &[
    ("LL84", "EPSG:4326"),
    ("WGS84.PSEUDOMERCATOR", "EPSG:3857"),
    ("AMERSFOORT.RD-NEW", "EPSG:28992"),
    ("RD-NEW", "EPSG:28992"),
    ("BELGE72.LAMBERT72", "EPSG:31370"),
    ("BELGE-LAMBERT-72", "EPSG:31370"),
    ("BRITISHNATGRID", "EPSG:27700"),
    ("CH1903PLUS.LV95", "EPSG:2056"),
    ("RGF93.LAMBERT-93", "EPSG:2154"),
];

/// The EPSG code a GEODATA coordinate system definition names: MapGuide XML
/// (R2010+, an EPSG alias or a known name) or WKT (R2009).
pub fn epsg_from_definition(def: &str) -> Option<String> {
    let up = def.to_ascii_uppercase();
    // AutoCAD: <Alias id="28992" type="CoordinateSystem"><ObjectId>RD-New</ObjectId><Namespace>EPSG Code</Namespace>
    if let Some(i) = up.find("<NAMESPACE>EPSG CODE</NAMESPACE>") {
        if let Some(j) = up[..i].rfind("<ALIAS ID=\"") {
            let code: String = up[j + 11..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                return Some(format!("EPSG:{code}"));
            }
        }
    }
    // <Alias id="…"><ObjectId>28992</ObjectId><Namespace>EPSG Code</Namespace>
    if let Some(i) = up.find("<NAMESPACE>EPSG CODE</NAMESPACE>") {
        let before = &up[..i];
        if let Some(j) = before.rfind("<OBJECTID>") {
            let code: String = before[j + 10..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if !code.is_empty() {
                return Some(format!("EPSG:{code}"));
            }
        }
    }
    if up.contains("PROJCS[") || up.contains("GEOGCS[") || up.contains("PROJCRS[") {
        return epsg_from_wkt(def);
    }
    // <CoordinateSystem id="UTM84-31N"> / ETRS89.UTM-31N
    let id = up.split("COORDINATESYSTEM ID=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or(up.trim());
    if let Some((_, c)) = MAPGUIDE.iter().find(|(n, _)| *n == id) {
        return Some((*c).into());
    }
    let utm = |prefix: &str, base: u32| -> Option<String> {
        let rest = id.strip_prefix(prefix)?;
        let zone: u32 = rest.trim_end_matches(['N', 'S']).parse().ok()?;
        let south = rest.ends_with('S');
        Some(format!("EPSG:{}", if south { base + 100 } else { base } + zone))
    };
    utm("UTM84-", 32600).or_else(|| utm("ETRS89.UTM-", 25800))
}

/// The CS-Map (MapGuide) name AutoCAD uses for an EPSG code, when we know it.
fn mapguide_name(code: &str) -> String {
    let n: u32 = code.trim_start_matches("EPSG:").parse().unwrap_or(0);
    match n {
        28992 => "Amersfoort.RD-New".into(),
        31370 => "Belge72.Lambert72".into(),
        27700 => "BritishNatGrid".into(),
        2056 => "CH1903Plus.LV95".into(),
        2154 => "RGF93.Lambert-93".into(),
        3857 => "WGS84.PseudoMercator".into(),
        4326 => "LL84".into(),
        32601..=32660 => format!("UTM84-{}N", n - 32600),
        32701..=32760 => format!("UTM84-{}S", n - 32700),
        25801..=25860 => format!("ETRS89.UTM-{}N", n - 25800),
        _ => format!("EPSG-{n}"),
    }
}

/// The coordinate system definition of a GEODATA object (MapGuide XML, as
/// AutoCAD writes it): the EPSG code as an alias of the CS-Map name. On one
/// line: a DXF string turns line breaks into `\P`, which is no XML.
pub fn definition_for(code: &str) -> String {
    let n = code.trim_start_matches("EPSG:");
    let name = mapguide_name(code);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\" standalone=\"no\" ?><Dictionary version=\"1.0\" xmlns=\"http://www.osgeo.org/mapguide/coordinatesystem\"><Alias id=\"{n}\" type=\"CoordinateSystem\"><ObjectId>{name}</ObjectId><Namespace>EPSG Code</Namespace></Alias><CoordinateSystem id=\"{name}\"><Name>{name}</Name><Authority>EPSG:{n}</Authority><Axis uom=\"METER\"><CoordinateSystemAxis><AxisOrder>1</AxisOrder><AxisName>Easting</AxisName><AxisAbbreviation>E</AxisAbbreviation><AxisDirection>east</AxisDirection></CoordinateSystemAxis><CoordinateSystemAxis><AxisOrder>2</AxisOrder><AxisName>Northing</AxisName><AxisAbbreviation>N</AxisAbbreviation><AxisDirection>north</AxisDirection></CoordinateSystemAxis></Axis></CoordinateSystem></Dictionary>"
    )
}

/// The GEODATA object for a project site in a drawing of `unit_factor`
/// drawing units per metre, the inverse of [`site_from`]: a local grid (the
/// reference point in the CRS) with the design point at the site's base
/// point (0,0 for a relative site).
pub fn geodata_for(site: &Site, unit_factor: f64) -> ocs_plugin_api::host::codec::objects::GeoData {
    use ocs_plugin_api::host::codec::{Vector2, Vector3};
    let f = if unit_factor > 0.0 { unit_factor } else { 1.0 };
    let mut g = ocs_plugin_api::host::codec::objects::GeoData::new();
    g.version = 2;
    // Local grid: the reference point in the CRS's own coordinates, as
    // AutoCAD writes it (and the configuration ezdxf has tested).
    g.coordinate_type = 1;
    let design = if site.absolute { [site.origin_e * f, site.origin_n * f] } else { [0.0, 0.0] };
    g.design_point = Vector3::new(design[0], design[1], 0.0);
    g.reference_point = Vector3::new(site.origin_e, site.origin_n, 0.0);
    g.north_direction = Vector2::new(0.0, 1.0);
    g.up_direction = Vector3::new(0.0, 0.0, 1.0);
    // Drawing units per metre; INSUNITS-style unit codes (6 metres, 4 millimetres).
    let units = if (f - 1000.0).abs() < 1e-9 { 4 } else if (f - 100.0).abs() < 1e-9 { 5 } else { 6 };
    g.horizontal_unit_scale = 1.0 / f;
    g.vertical_unit_scale = 1.0 / f;
    g.horizontal_units = units;
    g.vertical_units = units;
    g.scale_estimation_method = 1;
    g.coordinate_system_definition = definition_for(&site.crs);
    g
}

/// Adds (or replaces) the GEODATA object of the model space in `doc`, linked
/// as AutoCAD does: the model space block record's extension dictionary
/// holds it under `ACAD_GEOGRAPHICDATA`.
pub fn set_geodata(doc: &mut ocs_plugin_api::host::codec::CadDocument, site: &Site, unit_factor: f64) -> Result<(), String> {
    use ocs_plugin_api::host::codec::objects::ObjectType;
    use ocs_plugin_api::host::codec::tables::TableEntry;
    let ms = doc.block_records.get("*Model_Space").map(|b| b.handle()).ok_or("geen modelruimte in de tekening")?;
    let old: Vec<_> = doc.objects.iter().filter(|(_, o)| matches!(o, ObjectType::GeoData(_))).map(|(h, _)| *h).collect();
    for h in &old {
        doc.objects.remove(h);
    }
    let dict = doc.ensure_extension_dictionary(ms);
    let handle = doc.allocate_handle();
    let mut g = geodata_for(site, unit_factor);
    g.handle = handle;
    g.owner = dict;
    g.host_block = ms;
    doc.objects.insert(handle, ObjectType::GeoData(g));
    match doc.objects.get_mut(&dict) {
        Some(ObjectType::Dictionary(d)) => {
            d.entries.retain(|(k, h)| k != "ACAD_GEOGRAPHICDATA" && !old.contains(h));
            d.entries.push(("ACAD_GEOGRAPHICDATA".into(), handle));
            if !d.hard_owner_entries.iter().any(|k| k == "ACAD_GEOGRAPHICDATA") {
                d.hard_owner_entries.push("ACAD_GEOGRAPHICDATA".into());
            }
            Ok(())
        }
        _ => Err("de modelruimte heeft geen woordenboek voor GEODATA".into()),
    }
}

/// What a GEODATA object says about the drawing's location.
#[derive(Clone, Debug)]
pub struct GeoLocation {
    /// 1 local grid, 2 projected grid (both: reference point in the CRS), 3 geographic.
    pub coordinate_type: i16,
    pub design_point: [f64; 2],
    pub reference_point: [f64; 2],
    pub definition: String,
    /// Drawing units per CRS unit as the object states it (horizontal unit scale).
    pub unit_scale: f64,
    pub north: [f64; 2],
}

/// A project site from a GEODATA location, for a drawing in `unit_factor`
/// drawing units per metre. Only a north-up drawing maps onto a site.
pub fn site_from(g: &GeoLocation, unit_factor: f64) -> Result<Site, String> {
    let code = epsg_from_definition(&g.definition).ok_or("GEODATA: coordinate system not recognised")?;
    crs::ensure(&code)?;
    let tilt = g.north[0].atan2(g.north[1]).abs();
    if g.north != [0.0, 0.0] && tilt > 1e-6 {
        return Err(format!("GEODATA: north is turned {:.2}°; Open Infra works north-up", tilt.to_degrees()));
    }
    let f = if unit_factor > 0.0 { unit_factor } else { 1.0 };
    let mut site = Site::default();
    // Local (1) and projected (2) grids give the reference point in the CRS;
    // geographic (3) in longitude and latitude.
    let projected = matches!(g.coordinate_type, 1 | 2) && !crs::is_geographic(&code);
    if projected {
        site.crs = code;
        site.reproject_origin();
        // CRS = reference + (drawing − design) / units per metre.
        let o = [g.reference_point[0] - g.design_point[0] / f, g.reference_point[1] - g.design_point[1] / f];
        site.absolute = o[0].abs() < 1e-3 && o[1].abs() < 1e-3;
        let at = if site.absolute { g.reference_point } else { o };
        site.set_origin(at[0], at[1]);
    } else {
        // Geographic reference (lon, lat): a projected grid for the area,
        // with the design point as the base point.
        let (lon, lat) = (g.reference_point[0], g.reference_point[1]);
        if !(-180.0..=180.0).contains(&lon) || !(-90.0..=90.0).contains(&lat) {
            return Err("GEODATA: geographic reference point out of range".into());
        }
        if g.design_point != [0.0, 0.0] {
            return Err("GEODATA: a geographic location with the design point away from 0,0 is not supported".into());
        }
        site.lon = lon;
        site.lat = lat;
        site.crs = crs::suggest(lon, lat);
        crs::ensure(&site.crs)?;
        site.absolute = false;
        site.reproject_origin();
    }
    site.label = format!("GEODATA ({})", site.crs);
    Ok(site)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions() {
        let xml = r#"<?xml version="1.0"?><Dictionary><Alias id="28992" type="CoordinateSystem"><ObjectId>28992</ObjectId><Namespace>EPSG Code</Namespace></Alias><CoordinateSystem id="Amersfoort.RD-New"/></Dictionary>"#;
        assert_eq!(epsg_from_definition(xml).as_deref(), Some("EPSG:28992"));
        assert_eq!(epsg_from_definition(r#"<Dictionary><CoordinateSystem id="UTM84-32N"></CoordinateSystem></Dictionary>"#).as_deref(), Some("EPSG:32632"));
        assert_eq!(epsg_from_definition(r#"<Dictionary><CoordinateSystem id="BritishNatGrid"/></Dictionary>"#).as_deref(), Some("EPSG:27700"));
        assert_eq!(epsg_from_definition(r#"PROJCS["Amersfoort / RD New",AUTHORITY["EPSG","28992"]]"#).as_deref(), Some("EPSG:28992"));
    }

    #[test]
    fn written_definitions_read_back() {
        for code in ["EPSG:28992", "EPSG:32631", "EPSG:25832", "EPSG:2056", "EPSG:3006"] {
            assert_eq!(epsg_from_definition(&definition_for(code)).as_deref(), Some(code), "{code}");
        }
        // AutoCAD's own form: the EPSG code in the alias id, a CS-Map name as object id.
        let acad = r#"<Alias id="27700" type="CoordinateSystem"><ObjectId>OSGB-7P_2</ObjectId><Namespace>EPSG Code</Namespace></Alias>"#;
        assert_eq!(epsg_from_definition(acad).as_deref(), Some("EPSG:27700"));
    }

    #[test]
    fn geodata_round_trip_through_a_dwg() {
        use ocs_plugin_api::host::codec::objects::ObjectType;
        use ocs_plugin_api::host::codec::{CadDocument, DwgReader, DwgWriter};
        for (absolute, f) in [(false, 1.0), (true, 1.0), (false, 1000.0)] {
            let mut site = Site::default();
            site.crs = "EPSG:28992".into();
            site.set_origin(104885.0, 425330.0);
            site.absolute = absolute;
            let mut doc = CadDocument::new();
            set_geodata(&mut doc, &site, f).unwrap();
            set_geodata(&mut doc, &site, f).unwrap(); // replaced, not doubled
            let path = std::env::temp_dir().join(format!("oi-geodata-{}-{absolute}-{f}.dwg", std::process::id()));
            DwgWriter::write_to_file(&path, &doc).unwrap();
            let back = DwgReader::from_file(&path).unwrap().read().unwrap();
            let _ = std::fs::remove_file(&path);
            let gs: Vec<_> = back.objects.values().filter_map(|o| match o { ObjectType::GeoData(g) => Some(g), _ => None }).collect();
            assert_eq!(gs.len(), 1, "one GEODATA object");
            let g = gs[0];
            let loc = GeoLocation { coordinate_type: g.coordinate_type, design_point: [g.design_point.x, g.design_point.y], reference_point: [g.reference_point.x, g.reference_point.y], definition: g.coordinate_system_definition.clone(), unit_scale: g.horizontal_unit_scale, north: [g.north_direction.x, g.north_direction.y] };
            let s = site_from(&loc, f).unwrap();
            assert_eq!((s.crs.as_str(), s.absolute), ("EPSG:28992", absolute));
            assert!((s.origin_e - 104885.0).abs() < 1e-6 && (s.origin_n - 425330.0).abs() < 1e-6);
            let ms = back.block_records.get("*Model_Space").map(|b| ocs_plugin_api::host::codec::tables::TableEntry::handle(b)).unwrap();
            let dict = back.extension_dictionary_handle(ms).expect("model space extension dictionary");
            match back.objects.get(&dict) {
                Some(ObjectType::Dictionary(d)) => assert!(d.entries.iter().any(|(k, h)| k == "ACAD_GEOGRAPHICDATA" && *h == g.handle)),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn projected_site() {
        let g = GeoLocation { coordinate_type: 2, design_point: [0.0, 0.0], reference_point: [155000.0, 463000.0], definition: "<CoordinateSystem id=\"Amersfoort.RD-New\"/>".into(), unit_scale: 1.0, north: [0.0, 1.0] };
        let s = site_from(&g, 1.0).unwrap();
        assert_eq!(s.crs, "EPSG:28992");
        assert!(!s.absolute);
        assert!((s.origin_e - 155000.0).abs() < 1e-6);
        let abs = GeoLocation { design_point: [155000.0, 463000.0], ..g };
        assert!(site_from(&abs, 1.0).unwrap().absolute);
    }
}
