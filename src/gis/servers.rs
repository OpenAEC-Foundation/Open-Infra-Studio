//! Map server catalogue: presets for the Netherlands (PDOK), Flanders and
//! worldwide sources, plus servers the user adds (stored in the config dir).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "../../tests/support/server_saves.rs"]
mod storage_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    Wms,
    Wfs,
    Xyz,
}

impl ServerKind {
    pub fn label(self) -> &'static str {
        match self {
            ServerKind::Wms => "WMS",
            ServerKind::Wfs => "WFS",
            ServerKind::Xyz => "XYZ-tegels",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapServer {
    pub id: String,
    pub kind: ServerKind,
    pub name: String,
    pub region: String,
    /// Service endpoint (WMS/WFS) or tile template with {z}/{x}/{y} (XYZ).
    pub url: String,
    #[serde(default)]
    pub attribution: String,
    /// Layer / feature type preselected in the workbench.
    #[serde(default)]
    pub default_layer: String,
    /// XYZ only: highest zoom level the server provides.
    #[serde(default)]
    pub max_zoom: Option<u32>,
    /// XYZ only: lowest zoom level the server provides (QGIS `zmin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_zoom: Option<u32>,
    #[serde(skip)]
    pub builtin: bool,
}

fn preset(id: &str, kind: ServerKind, region: &str, name: &str, url: &str, layer: &str, attribution: &str) -> MapServer {
    MapServer {
        id: id.into(),
        kind,
        name: name.into(),
        region: region.into(),
        url: url.into(),
        attribution: attribution.into(),
        default_layer: layer.into(),
        max_zoom: if kind == ServerKind::Xyz { Some(19) } else { None },
        min_zoom: None,
        builtin: true,
    }
}

/// A checked public service from the worldwide list (`assets/servers/world.json`,
/// made from GIS2BIM's server catalogue by `tools/import_gis2bim_servers.py`).
#[derive(Clone, Debug, Deserialize)]
pub struct WorldPreset {
    pub id: String,
    pub kind: ServerKind,
    pub name: String,
    pub region_nl: String,
    pub region_en: String,
    pub country: String,
    #[serde(default)]
    pub category: String,
    pub url: String,
    #[serde(default)]
    pub default_layer: String,
    #[serde(default)]
    pub attribution: String,
    #[serde(default)]
    pub terms: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub license: String,
    /// Where the service has data, as GIS2BIM describes it.
    #[serde(default)]
    pub coverage: String,
    /// Coordinate systems, the one GIS2BIM requests first.
    #[serde(default)]
    pub crs: Vec<String>,
    /// Coverage in WGS 84 [west, south, east, north].
    #[serde(default)]
    pub bbox: Option<[f64; 4]>,
    #[serde(default)]
    pub verified: String,
}

pub fn world() -> &'static [WorldPreset] {
    static W: std::sync::OnceLock<Vec<WorldPreset>> = std::sync::OnceLock::new();
    W.get_or_init(|| serde_json::from_str(include_str!("../../assets/servers/world.json")).unwrap_or_default())
}

/// Country code of a worldwide preset (`DE`, `GLOBAL`, ...).
pub fn country(id: &str) -> Option<&'static str> {
    world().iter().find(|w| w.id == id).map(|w| w.country.as_str())
}

pub fn presets() -> Vec<MapServer> {
    use ServerKind::*;
    let mut v = builtin();
    v.extend(world().iter().map(|w| MapServer {
        id: w.id.clone(),
        kind: w.kind,
        name: w.name.clone(),
        region: crate::lang::tr(&w.region_nl, &w.region_en).to_string(),
        url: w.url.clone(),
        attribution: w.attribution.clone(),
        default_layer: w.default_layer.clone(),
        max_zoom: if w.kind == Xyz { Some(19) } else { None },
        min_zoom: None,
        builtin: true,
    }));
    v
}

pub(crate) fn builtin() -> Vec<MapServer> {
    use ServerKind::*;
    let nl = crate::lang::tr("Nederland", "Netherlands");
    let vl = crate::lang::tr("België (Vlaanderen)", "Belgium (Flanders)");
    let world = crate::lang::tr("Wereld", "World");
    vec![
        // Nederland — PDOK WMS
        preset("pdok-luchtfoto", Wms, nl, "PDOK Luchtfoto RGB (actueel, 8 cm)", "https://service.pdok.nl/hwh/luchtfotorgb/wms/v1_0", "Actueel_orthoHR", "Beeldmateriaal.nl / PDOK"),
        preset("pdok-kadastralekaart-wms", Wms, nl, "PDOK Kadastrale kaart", "https://service.pdok.nl/kadaster/kadastralekaart/wms/v5_0", "Kadastralekaart", "Kadaster / PDOK"),
        preset("pdok-bag-wms", Wms, nl, "PDOK BAG (panden)", "https://service.pdok.nl/lv/bag/wms/v2_0", "pand", "Kadaster / PDOK"),
        preset("pdok-ruimtelijke-plannen-wms", Wms, nl, "PDOK Ruimtelijke plannen (enkelbestemming)", "https://service.pdok.nl/kadaster/plu/wms/v1_0", "enkelbestemming", "Kadaster / PDOK"),
        preset("pdok-top10nl-wms", Wms, nl, "PDOK TOP10NL topografie", "https://service.pdok.nl/brt/top10nl/wms/v1_0", "", "Kadaster / PDOK"),
        preset("pdok-ahn-wms", Wms, nl, "PDOK AHN hoogtebestand (DTM 0,5 m)", "https://service.pdok.nl/rws/ahn/wms/v1_0", "dtm_05m", "AHN / Rijkswaterstaat / PDOK"),
        // Nederland — PDOK WFS
        preset("pdok-bag-wfs", Wfs, nl, "PDOK BAG — panden en verblijfsobjecten", "https://service.pdok.nl/lv/bag/wfs/v2_0", "bag:pand", "Kadaster / PDOK"),
        preset("pdok-kadastralekaart-wfs", Wfs, nl, "PDOK Kadastrale kaart — percelen", "https://service.pdok.nl/kadaster/kadastralekaart/wfs/v5_0", "kadastralekaart:Perceel", "Kadaster / PDOK"),
        preset("pdok-nwbwegen-wfs", Wfs, nl, "PDOK NWB Wegen (wegvakken, hectopunten)", "https://service.pdok.nl/rws/nwbwegen/wfs/v1_0", "nwbwegen:wegvakken", "Rijkswaterstaat / PDOK"),
        preset("pdok-nwbvaarwegen-wfs", Wfs, nl, "PDOK NWB Vaarwegen", "https://service.pdok.nl/rws/nwbvaarwegen/wfs/v1_0", "", "Rijkswaterstaat / PDOK"),
        preset("pdok-spoorwegen-wfs", Wfs, nl, "PDOK Spoorwegen (ProRail)", "https://service.pdok.nl/prorail/spoorwegen/wfs/v1_0", "spoorwegen:spooras", "ProRail / PDOK"),
        preset("pdok-bestuurlijk-wfs", Wfs, nl, "PDOK Bestuurlijke gebieden", "https://service.pdok.nl/kadaster/bestuurlijkegebieden/wfs/v1_0", "", "Kadaster / PDOK"),
        preset("pdok-natura2000-wfs", Wfs, nl, "PDOK Natura 2000", "https://service.pdok.nl/rvo/natura2000/wfs/v1_0", "", "RVO / PDOK"),
        preset("pdok-natura2000-wms", Wms, nl, "PDOK Natura 2000 (kaart)", "https://service.pdok.nl/rvo/natura2000/wms/v1_0", "natura2000", "RVO / PDOK"),
        preset("pdok-bro-geomorfologie-wms", Wms, nl, "BRO Geomorfologische kaart", "https://service.pdok.nl/bzk/bro-geomorfologischekaart/wms/v2_0", "geomorphological_area", "BRO / TNO / PDOK"),
        preset("pdok-bro-bodemkaart-wms", Wms, nl, "BRO Bodemkaart", "https://service.pdok.nl/bzk/bro-bodemkaart/wms/v1_0", "soilarea", "BRO / WUR / PDOK"),
        // Nederland — historische kaarten
        preset("hisgis-minuutplans-1832", Xyz, nl, "HisGIS — kadastrale minuutplans 1832", "https://geoservices.hisgis.nl/tiles/minuutplans/{z}/{x}/{y}", "", "HisGIS (Fryske Akademy / KNAW)"),
        preset("hisgis-kaart-1832", Xyz, nl, "HisGIS — kaart van 1832", "https://geoservices.hisgis.nl/wmts/osm1832/{z}/{x}/{y}.png", "", "HisGIS (Fryske Akademy / KNAW)"),
        tijdreis(1850),
        tijdreis(1900),
        tijdreis(1950),
        tijdreis(2000),
        // België — Vlaanderen
        preset("vl-ortho-wms", Wms, vl, "Digitaal Vlaanderen — Orthofoto", "https://geo.api.vlaanderen.be/OMWRGBMRVL/wms", "", "Digitaal Vlaanderen"),
        preset("vl-grb-wfs", Wfs, vl, "Digitaal Vlaanderen — GRB", "https://geo.api.vlaanderen.be/GRB/wfs", "", "Digitaal Vlaanderen"),
        // Wereldwijd
        preset("osm-tiles", Xyz, world, "OpenStreetMap (tegels)", "https://tile.openstreetmap.org/{z}/{x}/{y}.png", "", "© OpenStreetMap-bijdragers (ODbL)"),
        preset("terrestris-osm-wms", Wms, world, "terrestris OSM WMS", "https://ows.terrestris.de/osm/service", "OSM-WMS", "© terrestris, © OpenStreetMap-bijdragers"),
        preset("eox-s2-wms", Wms, world, "EOX Sentinel-2 cloudless (satelliet)", "https://tiles.maps.eox.at/wms", "s2cloudless-2023", "Sentinel-2 cloudless by EOX (CC BY-NC-SA 4.0)"),
        preset("nasa-gibs-wms", Wms, world, "NASA GIBS (aardobservatie)", "https://gibs.earthdata.nasa.gov/wms/epsg4326/best/wms.cgi", "BlueMarble_ShadedRelief_Bathymetry", "NASA EOSDIS GIBS"),
        preset("gebco-wms", Wms, world, "GEBCO bathymetrie", "https://wms.gebco.net/mapserv", "GEBCO_LATEST_SUB_ICE_TOPO", "GEBCO Compilation Group"),
    ]
}

/// `<config>/OpenCADStudio/open-infra/` — beside the host's own settings.
pub fn config_dir() -> PathBuf {
    let base = if let Some(a) = std::env::var_os("APPDATA") {
        PathBuf::from(a)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support")
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(x)
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
    };
    base.join("OpenCADStudio").join("open-infra")
}

fn user_file() -> PathBuf {
    config_dir().join("servers.json")
}

pub fn load_user() -> Vec<MapServer> {
    std::fs::read_to_string(user_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The user's servers, or an error when the file exists but cannot be read:
/// callers that write the list back must not replace a file they could not parse.
pub fn try_load_user() -> Result<Vec<MapServer>, String> {
    read_user(&user_file())
}

fn read_user(path: &Path) -> Result<Vec<MapServer>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Change the current list while holding a lock shared by both plugin
/// processes. Read, merge and save must all happen within this operation.
pub fn update_user<T>(change: impl FnOnce(&mut Vec<MapServer>) -> T) -> Result<T, String> {
    let path = user_file();
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(dir.join("servers.json.lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let mut servers = read_user(&path)?;
    let result = change(&mut servers);
    write_user(&path, &servers)?;
    // Closing the lock file releases the lock, including on error paths.
    Ok(result)
}

/// Replace the list; callers adding or removing servers use `update_user`
/// so they do not write a snapshot read before another writer's change.
pub fn save_user(servers: &[MapServer]) -> Result<(), String> {
    update_user(|user| *user = servers.to_vec())
}

fn write_user(path: &Path, servers: &[MapServer]) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap();
    let json = serde_json::to_string_pretty(servers).map_err(|e| e.to_string())?;
    let (tmp, mut file) = loop {
        let tmp = dir.join(format!(".servers-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = (|| -> std::io::Result<()> {
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

pub fn all() -> Vec<MapServer> {
    let mut v = presets();
    v.extend(load_user());
    v
}

pub fn find(id: &str) -> Option<MapServer> {
    if let Some(year) = id.strip_prefix("topotijdreis-").and_then(|y| y.parse().ok()) {
        return Some(tijdreis(year));
    }
    all().into_iter().find(|s| s.id.eq_ignore_ascii_case(id))
}

/// Topotijdreis: the Kadaster topographic map of a year (1815–2014), an
/// ArcGIS tile cache in RD (`topotijdreis-<jaar>` works for every year).
pub fn tijdreis(year: u32) -> MapServer {
    let nl = crate::lang::tr("Nederland", "Netherlands");
    let mut s = preset(
        &format!("topotijdreis-{year}"),
        ServerKind::Xyz,
        nl,
        &format!("Topotijdreis {year} (Kadaster)"),
        &format!("https://tiles.arcgis.com/tiles/nSZVuSZjHpEZZbRo/arcgis/rest/services/Historische_tijdreis_{year}/MapServer"),
        "",
        "Kadaster / Esri Nederland (Topotijdreis)",
    );
    s.max_zoom = None;
    s
}
