//! Open Infra — GIS workbench for Open CAD Studio.
//!
//! A "GIS" ribbon tab modelled on the FreeCAD GIS(2BIM) workbench: set the
//! project site and coordinate system, then pull WMS/tile map layers, WFS
//! vector data and OpenStreetMap into the drawing — the Netherlands (PDOK)
//! and worldwide. The workbench window runs in this plugin's own process.
//!
//! The same crate is the engine of the Civiel plugin (civiel/), which links
//! it without the `plugin` feature: then the GIS tab's items are unused.

#![cfg_attr(not(feature = "plugin"), allow(dead_code))]

pub mod bridge;
pub mod civiel;
pub mod emit;
pub mod hoeveelheden;
pub mod hoogte;
pub mod ifc;
pub mod gis;
pub mod import;
pub mod basisdata;
pub mod bestand;
pub mod plannen;
pub mod bijwerken;
pub mod lang;
pub mod localisation;
pub mod library;
pub mod riolering;
pub mod bgt_mutatie;
pub mod planning_commands;
pub mod nlcs;
pub mod nlcs_check;
pub mod tekenblad;
pub mod ui;

use std::sync::OnceLock;

use ocs_plugin_api::host::{BuiltinPlugin, CommandStep, HostApi, InteractiveCommand, PreviewWire};
use ocs_plugin_api::manifest::{ApiVersion, PluginManifest};
use ocs_plugin_api::ribbon::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

use gis::site::Site;

static MANIFEST: PluginManifest = PluginManifest {
    id: "openaec.open-infra",
    name: "Open Infra — GIS",
    version: env!("CARGO_PKG_VERSION"),
    description: "GIS-workbench en NLCS-basisdata voor infra en civiele techniek: projectlocatie, WMS/WFS-servers, OpenStreetMap, BGT, kadaster en KLIC conform NLCS. Nederland (PDOK) en wereldwijd.",
    api_version: ApiVersion::CURRENT,
    ribbon_order: 60,
    xdata_apps: &[bridge::XDATA_APP],
    command_prefixes: &["GIS_"],
};

const I_LOCATION: IconKind = IconKind::Svg(include_bytes!("../assets/location.svg"));
const I_AREA: IconKind = IconKind::Svg(include_bytes!("../assets/area.svg"));
const I_MAP: IconKind = IconKind::Svg(include_bytes!("../assets/map.svg"));
const I_LAYERS: IconKind = IconKind::Svg(include_bytes!("../assets/layers.svg"));
const I_VECTOR: IconKind = IconKind::Svg(include_bytes!("../assets/vector.svg"));
const I_GLOBE: IconKind = IconKind::Svg(include_bytes!("../assets/globe.svg"));
const I_POINT: IconKind = IconKind::Svg(include_bytes!("../assets/point.svg"));
const I_SERVER: IconKind = IconKind::Svg(include_bytes!("../assets/server.svg"));
const I_QUICK: IconKind = IconKind::Svg(include_bytes!("../assets/quick.svg"));
const I_PDOK: IconKind = IconKind::Svg(include_bytes!("../assets/pdok.svg"));
const I_BGT: IconKind = IconKind::Svg(include_bytes!("../assets/bgt.svg"));
const I_KADASTER: IconKind = IconKind::Svg(include_bytes!("../assets/kadaster.svg"));
const I_KLIC: IconKind = IconKind::Svg(include_bytes!("../assets/klic.svg"));
const I_SHEET: IconKind = IconKind::Svg(include_bytes!("../assets/sheet.svg"));
const I_IFC: IconKind = IconKind::Svg(include_bytes!("../assets/ifc.svg"));
const I_AHN: IconKind = IconKind::Svg(include_bytes!("../assets/ahn.svg"));
const I_CONTOURS: IconKind = IconKind::Svg(include_bytes!("../assets/hoogtelijnen.svg"));
const I_PROFILE: IconKind = IconKind::Svg(include_bytes!("../assets/profiel.svg"));
const I_CLOUD: IconKind = IconKind::Svg(include_bytes!("../assets/puntenwolk.svg"));
const I_VEHICLE: IconKind = IconKind::Svg(include_bytes!("../assets/vehicle.svg"));
const I_CATALOGUE: IconKind = IconKind::Svg(include_bytes!("../assets/catalogue.svg"));
const I_SWEPT: IconKind = IconKind::Svg(include_bytes!("../assets/swept.svg"));
const I_FILE: IconKind = IconKind::Svg(include_bytes!("../assets/file.svg"));
const I_CHECK: IconKind = IconKind::Svg(include_bytes!("../assets/nlcs-controle.svg"));
const I_QUANTITIES: IconKind = IconKind::Svg(include_bytes!("../assets/hoeveelheden.svg"));
const I_TABLE: IconKind = IconKind::Svg(include_bytes!("../assets/hoeveelhedenstaat.svg"));
const I_MUTATION: IconKind = IconKind::Svg(include_bytes!("../assets/bgt-mutatie.svg"));
const I_CPT: IconKind = IconKind::Svg(include_bytes!("../assets/sonderingen.svg"));
const I_SERVICES: IconKind = IconKind::Svg(include_bytes!("../assets/services.svg"));

/// One-click imports from the ribbon (no window): command → what to fetch.
enum Quick {
    Data(import::Dataset),
    Osm,
}

const QUICK: &[(&str, &str, Quick)] = &[
    ("GIS_US_TOPO", "USGS topografie|USGS topography", Quick::Data(import::Dataset::Raster { server: "w-us-usgs-tnm-topo-3857" })),
    ("GIS_US_ELEVATION", "USGS 3DEP hoogtekaart|USGS 3DEP elevation map", Quick::Data(import::Dataset::Raster { server: "w-us-usgs-3dep-hillshade-3857" })),
    ("GIS_US_BOUNDARIES", "Census bestuurlijke grenzen|Census administrative boundaries", Quick::Data(import::Dataset::Raster { server: "w-us-census-tigerweb-boundaries-3857" })),
    ("GIS_US_SOILS", "NRCS bodemkaart|NRCS soil map", Quick::Data(import::Dataset::Raster { server: "w-us-nrcs-ssurgo-soilmapunits-3857" })),
    ("GIS_Q_LUCHTFOTO", "Luchtfoto (PDOK)|Aerial photo (PDOK)", Quick::Data(import::Dataset::Raster { server: "pdok-luchtfoto" })),
    ("GIS_Q_KADASTER", "Kadastrale kaart (PDOK)|Cadastral map (PDOK)", Quick::Data(import::Dataset::Raster { server: "pdok-kadastralekaart-wms" })),
    ("GIS_Q_AHN", "AHN hoogtekaart (PDOK)|AHN elevation map (PDOK)", Quick::Data(import::Dataset::Raster { server: "pdok-ahn-wms" })),
    ("GIS_Q_OSMKAART", "OpenStreetMap-kaart (wereld)|OpenStreetMap map (world)", Quick::Data(import::Dataset::Raster { server: "osm-tiles" })),
    ("GIS_Q_SATELLIET", "Sentinel-2 satelliet (wereld)|Sentinel-2 satellite (world)", Quick::Data(import::Dataset::Raster { server: "eox-s2-wms" })),
    ("GIS_Q_BAG", "BAG panden (PDOK)|BAG buildings (PDOK)", Quick::Data(import::Dataset::Wfs { server: "pdok-bag-wfs", typename: "bag:pand", label: None, aci: 30 })),
    ("GIS_Q_PERCELEN", "Kadastrale percelen (PDOK)|Cadastral parcels (PDOK)", Quick::Data(import::Dataset::Wfs { server: "pdok-kadastralekaart-wfs", typename: "kadastralekaart:Perceel", label: Some("perceelnummer"), aci: 3 })),
    ("GIS_Q_NWB", "NWB wegvakken (RWS)|NWB road sections (RWS)", Quick::Data(import::Dataset::Wfs { server: "pdok-nwbwegen-wfs", typename: "nwbwegen:wegvakken", label: Some("stt_naam"), aci: 2 })),
    ("GIS_Q_HECTO", "NWB hectometerpunten (RWS)|NWB hectometre posts (RWS)", Quick::Data(import::Dataset::Wfs { server: "pdok-nwbwegen-wfs", typename: "nwbwegen:hectopunten", label: Some("hectomtrng"), aci: 2 })),
    ("GIS_Q_SPOOR", "Spoorassen (ProRail)|Rail centrelines (ProRail)", Quick::Data(import::Dataset::Wfs { server: "pdok-spoorwegen-wfs", typename: "spoorwegen:spooras", label: None, aci: 6 })),
    ("GIS_Q_OSM", "OpenStreetMap-vectoren (wereld)|OpenStreetMap vectors (world)", Quick::Osm),
    ("GIS_Q_NATURA2000", "Natura 2000 (PDOK)|Natura 2000 (PDOK)", Quick::Data(import::Dataset::Raster { server: "pdok-natura2000-wms" })),
    ("GIS_Q_GEOMORFOLOGIE", "Geomorfologische kaart (BRO)|Geomorphological map (BRO)", Quick::Data(import::Dataset::Raster { server: "pdok-bro-geomorfologie-wms" })),
    ("GIS_Q_BODEMKAART", "Bodemkaart (BRO)|Soil map (BRO)", Quick::Data(import::Dataset::Raster { server: "pdok-bro-bodemkaart-wms" })),
    ("GIS_Q_HISGIS", "Minuutplans 1832 (HisGIS)|Cadastral maps 1832 (HisGIS)", Quick::Data(import::Dataset::Raster { server: "hisgis-minuutplans-1832" })),
    ("GIS_Q_TIJDREIS", "Topotijdreis 1900 (Kadaster)|Topotijdreis 1900 (Kadaster)", Quick::Data(import::Dataset::Raster { server: "topotijdreis-1900" })),
];

#[cfg(test)]
mod regional_quick_tests {
    use super::*;
    #[test]
    fn us_ribbon_shortcuts_resolve_to_us_raster_sources() {
        let catalogue=gis::catalog::all();
        let mut checked=0;
        for (command,_,quick) in QUICK.iter().filter(|(c,..)|c.starts_with("GIS_US_")) {
            if let Quick::Data(import::Dataset::Raster{server})=quick {
                let source=catalogue.iter().find(|s|s.id==*server).unwrap_or_else(||panic!("{command}: missing source {server}"));
                assert_eq!(source.country,"US");assert_eq!(source.kind,gis::servers::ServerKind::Wms);checked+=1;
            }else{panic!("{command}: expected raster source")}
        }
        assert_eq!(checked,4);
    }
}

/// A quick-import label in the interface language (`"nl|en"`).
fn ql(label: &'static str) -> &'static str {
    let (nl, en) = label.split_once('|').unwrap_or((label, label));
    lang::tr(nl, en)
}

fn is_raster(q: &Quick) -> bool {
    matches!(q, Quick::Data(import::Dataset::Raster { .. }))
}

fn tool(id: &'static str, label: &'static str, icon: IconKind) -> RibbonItem {
    RibbonItem::LargeTool(ToolDef { id, label, icon, event: ModuleEvent::Command(id.to_string()) })
}

struct GisModule;

impl CadModule for GisModule {
    fn id(&self) -> &'static str {
        "open_infra_gis"
    }
    fn title(&self) -> &'static str {
        "GIS"
    }
    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: OnceLock<Vec<RibbonGroup>> = OnceLock::new();
        GROUPS.get_or_init(|| {
            // Worldwide tools first and large; the Dutch registers (PDOK, BGT,
            // BRK, KLIC, AHN, BRO, NLCS) in one compact group of menus, at most
            // a fifth of the ribbon, and off altogether with `netherlands:
            // false` in the settings.
            use lang::tr;
            let world = |c: &str| matches!(c, "GIS_Q_OSMKAART" | "GIS_Q_SATELLIET" | "GIS_Q_OSM");
            let quick_nl = |raster: bool| -> Vec<(&'static str, &'static str, IconKind)> {
                QUICK.iter().filter(|(c, _, q)| !world(c) && !c.starts_with("GIS_US_") && is_raster(q) == raster).map(|(c, l, _)| (*c, ql(l), I_QUICK)).collect()
            };
            let quick_world: Vec<(&'static str, &'static str, IconKind)> = QUICK.iter().filter(|(c, ..)| world(c)).map(|(c, l, _)| (*c, ql(l), I_QUICK)).collect();
            let small = |id: &'static str, label: &'static str, icon: IconKind| RibbonItem::LabeledTool(ToolDef { id, label, icon, event: ModuleEvent::Command(id.to_string()) });
            let menu = |id: &'static str, label: &'static str, icon: IconKind, items: Vec<(&'static str, &'static str, IconKind)>, default: &'static str| RibbonItem::LabeledDropdown { id, label, icon, items, default };
            let mut groups = vec![
                RibbonGroup {
                    title: tr("Locatie", "Location"),
                    tools: vec![
                        tool("GIS_LOCATIE", tr("Geo-locatie", "Geolocation"), I_LOCATION),
                        small("GIS_GEBIED", tr("Gebied", "Area"), I_AREA),
                        small("GIS_PUNT", tr("Punt info", "Point info"), I_POINT),
                        small("GIS_COORD", tr("Coördinaten", "Coordinates"), I_POINT),
                    ],
                },
                // Like the FreeCAD GIS workbench toolbars: any server, worldwide.
                RibbonGroup {
                    title: tr("Kaarten & data", "Maps & data"),
                    tools: vec![
                        tool("GIS_SERVICES", tr("Services wereldwijd", "Services worldwide"), I_SERVICES),
                        tool("GIS_KAART", "WMS / TMS", I_MAP),
                        tool("GIS_WFS", "WFS", I_VECTOR),
                        tool("GIS_OSM", "OpenStreetMap", I_GLOBE),
                        RibbonItem::LargeDropdown { id: "GIS_Q_WERELD", label: tr("Snel (wereld)", "Quick (world)"), icon: I_LAYERS, items: quick_world, default: "GIS_Q_OSMKAART" },
                        // One file dialog for vector files, rasters and point clouds.
                        RibbonItem::LargeTool(ToolDef {
                            id: "GIS_BESTAND",
                            label: tr("Bestand openen", "Open file"),
                            icon: I_FILE,
                            event: ModuleEvent::PluginFileDialog {
                                command: "GIS_BESTAND".into(),
                                title: tr("Geodatabestand openen", "Open a geodata file").into(),
                                filter_name: "GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX, GML, CSV, GeoTIFF, ASC, JPG/PNG + world file, LAS/LAZ".into(),
                                extensions: gis::files::EXTENSIONS.iter().chain(gis::rasterbestand::EXTENSIONS.iter()).map(|e| e.to_string()).collect(),
                            },
                        }),
                    ],
                },
                // Heights: the AHN in the Netherlands, Terrain Tiles elsewhere.
                RibbonGroup {
                    title: tr("Hoogte", "Elevation"),
                    tools: vec![
                        small("GIS_HOOGTEPUNTEN", tr("Hoogtepunten", "Spot heights"), I_POINT),
                        small("GIS_HOOGTELIJNEN", tr("Hoogtelijnen", "Contours"), I_CONTOURS),
                        small("GIS_PROFIEL", tr("Profiel", "Profile"), I_PROFILE),
                    ],
                },
                RibbonGroup {
                    title: tr("Uitvoer", "Output"),
                    tools: vec![tool("GIS_TEKENBLAD", tr("Tekenblad", "Drawing sheet"), I_SHEET), small("GIS_IFC", "IFC 4x3", I_IFC), small("GIS_SERVERS", "Servers", I_SERVER)],
                },
                // Block libraries (vehicles), like the vehicle sets of
                // swept-path software.
                RibbonGroup {
                    title: tr("Bibliotheek", "Library"),
                    tools: vec![
                        small("GIS_INFRA_LIBRARY", tr("Infra-objecten", "Infrastructure"), I_CATALOGUE),
                        tool("GIS_VOERTUIGEN", tr("Voertuigen", "Vehicles"), I_VEHICLE),
                        small("GIS_VOERTUIGOVERZICHT", tr("Overzicht", "Catalogue"), I_CATALOGUE),
                        small("GIS_VEEGPAD", tr("Veegpad", "Swept path"), I_SWEPT),
                    ],
                },
            ];
            if lang::dutch() {
                // The Netherlands: PDOK, BGT/BRK/KLIC/BRO to NLCS, AHN, NLCS checks.
                groups.insert(
                    3,
                    RibbonGroup {
                        title: tr("Nederland", "Netherlands"),
                        tools: vec![
                            menu(
                                "GIS_NL_BASIS",
                                tr("Basisdata NL", "Dutch base data"),
                                I_BGT,
                                vec![
                                    ("GIS_BGT", tr("BGT · BRK · KLIC (venster)", "BGT · BRK · KLIC (window)"), I_BGT),
                                    ("GIS_Q_BGT", tr("BGT importeren", "Import BGT"), I_BGT),
                                    ("GIS_BGT_BIJWERKEN", tr("BGT bijwerken", "Update BGT"), I_BGT),
                                    ("GIS_Q_BRK", tr("Kadaster importeren", "Import cadastre"), I_KADASTER),
                                    ("GIS_KLIC", tr("KLIC-levering", "KLIC delivery"), I_KLIC),
                                    ("GIS_Q_GWSW", tr("GWSW-riolering", "GWSW sewers"), I_BGT),
                                    ("GIS_Q_SONDERINGEN", tr("Sonderingen en boringen (BRO)", "CPTs and boreholes (BRO)"), I_CPT),
                                    ("GIS_AHN", tr("AHN (venster)", "AHN (window)"), I_AHN),
                                    ("GIS_PUNTENWOLK", tr("AHN-puntenwolk", "AHN point cloud"), I_CLOUD),
                                    ("GIS_PLANNEN", tr("Ruimtelijke plannen", "Spatial plans"), I_LAYERS),
                                    ("GIS_PDOK", tr("PDOK-catalogus", "PDOK catalogue"), I_PDOK),
                                ],
                                "GIS_BGT",
                            ),
                            menu("GIS_NL_KAART", tr("Kaartbeeld NL", "Dutch maps"), I_LAYERS, quick_nl(true), "GIS_Q_LUCHTFOTO"),
                            menu("GIS_NL_DATA", tr("Vectordata NL", "Dutch vector data"), I_QUICK, quick_nl(false), "GIS_Q_BAG"),
                            menu(
                                "GIS_NL_NLCS",
                                "NLCS",
                                I_CHECK,
                                vec![
                                    ("GIS_NLCS_CONTROLE", tr("Controle", "Check"), I_CHECK),
                                    ("GIS_HOEVEELHEDEN", tr("Hoeveelheden", "Quantities"), I_QUANTITIES),
                                    ("GIS_HOEVEELHEDEN_TABEL", tr("Hoeveelhedenstaat", "Bill of quantities"), I_TABLE),
                                    ("GIS_BGT_MUTATIE", tr("BGT-mutatie", "BGT mutation"), I_MUTATION),
                                ],
                                "GIS_NLCS_CONTROLE",
                            ),
                        ],
                    },
                );
            }
            if localisation::current()==localisation::Region::UnitedStates {
                let mut tools=vec![small("GIS_US_SERVICES", tr("Services VS", "US services"), I_SERVICES)];
                tools.extend(QUICK.iter().filter(|(c,..)|c.starts_with("GIS_US_")).map(|(c,l,_)|small(c,ql(l),I_MAP)));
                groups.insert(3,RibbonGroup{title:tr("Verenigde Staten","United States"),tools});
            }
            groups
        })
    }
}

/// `GIS_GEBIED`: pick two opposite corners of the import area in the drawing.
struct AreaCmd {
    first: Option<[f64; 2]>,
    site: Site,
    unit_factor: f64,
}

impl InteractiveCommand for AreaCmd {
    fn prompt(&self) -> String {
        if self.first.is_none() { "GIS: eerste hoek van het importgebied".into() } else { "GIS: tegenoverliggende hoek".into() }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let Some(a) = self.first else {
            self.first = Some([pt[0], pt[1]]);
            return CommandStep::NeedPoint;
        };
        let b = [pt[0], pt[1]];
        let pa = self.site.from_drawing(a, self.unit_factor);
        let pb = self.site.from_drawing(b, self.unit_factor);
        let (w, h) = ((pa[0] - pb[0]).abs(), (pa[1] - pb[1]).abs());
        if w < 1.0 || h < 1.0 {
            return CommandStep::Cancel;
        }
        let mut site = self.site.clone();
        site.set_origin((pa[0] + pb[0]) / 2.0, (pa[1] + pb[1]) / 2.0);
        site.width = (w * 10.0).round() / 10.0;
        site.height = (h * 10.0).round() / 10.0;
        // Write from a worker: this callback runs inside the host's input
        // handling and must not wait on another host round-trip.
        std::thread::spawn(move || match bridge::save_site(&site) {
            Ok(()) => {
                bridge::info(&format!("Importgebied {} × {} m rond X {:.2} Y {:.2} ({}).", site.width, site.height, site.origin_e, site.origin_n, site.crs));
                ui::send(ui::UiMsg::SiteChanged);
            }
            Err(e) => bridge::error(&format!("Gebied niet opgeslagen: {e}")),
        });
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let Some(a) = self.first else { return Vec::new() };
        let p = |x: f64, y: f64| [x, y, 0.0];
        vec![PreviewWire::with_color(
            vec![p(a[0], a[1]), p(pt[0], a[1]), p(pt[0], pt[1]), p(a[0], pt[1]), p(a[0], a[1])],
            [0.95, 0.6, 0.15, 1.0],
        )]
    }
}

/// `GIS_PUNT`: report a picked point in the project CRS, WGS 84 and RD/UTM.
struct PointCmd {
    site: Site,
    unit_factor: f64,
}

impl InteractiveCommand for PointCmd {
    fn prompt(&self) -> String {
        "GIS: wijs een punt aan".into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let p = self.site.from_drawing([pt[0], pt[1]], self.unit_factor);
        let (site, unit) = (self.site.crs.clone(), self.unit_factor);
        std::thread::spawn(move || {
            let mut lines = vec![format!("{site}: X {:.3}  Y {:.3}", p[0], p[1])];
            if let Ok(ll) = gis::crs::transform(&site, "EPSG:4326", p) {
                lines.push(format!("WGS 84: {:.8} N  {:.8} E", ll[1], ll[0]));
                let utm = gis::crs::utm_code_for(ll[0], ll[1]);
                for other in ["EPSG:28992".to_string(), utm] {
                    if other != site {
                        if let (Ok(q), Some(i)) = (gis::crs::transform(&site, &other, p), gis::crs::known(&other)) {
                            lines.push(format!("{}: X {:.3}  Y {:.3}", i.name, q[0], q[1]));
                        }
                    }
                }
            }
            let _ = unit;
            let rd = if site == "EPSG:28992" { Ok(p) } else { gis::crs::transform(&site, "EPSG:28992", p) };
            if let Ok(rd) = rd {
                if (0.0..300_000.0).contains(&rd[0]) && (290_000.0..630_000.0).contains(&rd[1]) {
                    let (dtm, dsm) = hoogte::heights_at(rd);
                    let show = |z: Option<f64>| z.map(|z| format!("{} m NAP", hoogte::nap(z, 2))).unwrap_or_else(|| "—".into());
                    lines.push(format!("AHN4 maaiveld {} · oppervlak {}", show(dtm), show(dsm)));
                }
            }
            bridge::info(&lines.join("  |  "));
            ui::send(ui::UiMsg::Picked(p));
        });
        CommandStep::Done
    }
}

/// `GIS_PROFIEL`: the route (picked, or a selected polyline), then the
/// lower-left corner of the profile drawing.
struct ProfileCmd {
    route: Vec<[f64; 2]>,
    picking_route: bool,
    site: Site,
}

impl InteractiveCommand for ProfileCmd {
    fn prompt(&self) -> String {
        match (self.picking_route, self.route.len()) {
            (true, 0) => "GIS-profiel: beginpunt van de profiellijn".into(),
            (true, 1) => "GIS-profiel: volgend punt".into(),
            (true, _) => "GIS-profiel: volgend punt [Enter = klaar]".into(),
            (false, _) => "GIS-profiel: linkeronderhoek van het lengteprofiel".into(),
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        if self.picking_route {
            self.route.push([pt[0], pt[1]]);
            return CommandStep::NeedPoint;
        }
        let (site, route, at) = (self.site.clone(), self.route.clone(), [pt[0], pt[1]]);
        // A worker: this callback runs inside the host's input handling.
        std::thread::spawn(move || {
            let opt = hoogte::settings();
            match hoogte::profile(&site, &route, at, &opt, &|_| {}) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("Profiel: {e}")),
            }
        });
        CommandStep::Done
    }
    fn on_enter(&mut self) -> CommandStep {
        if self.picking_route && self.route.len() >= 2 {
            self.picking_route = false;
            CommandStep::NeedPoint
        } else {
            CommandStep::Cancel
        }
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let mut pts: Vec<[f64; 3]> = self.route.iter().map(|p| [p[0], p[1], 0.0]).collect();
        if self.picking_route {
            if pts.is_empty() {
                return Vec::new();
            }
            pts.push(pt);
            return vec![PreviewWire::with_color(pts, [0.95, 0.6, 0.15, 1.0])];
        }
        // The profile's foot: as long as the route, at the cursor.
        let length: f64 = self.route.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
        let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
        let foot = vec![[pt[0], pt[1], 0.0], [pt[0] + length * unit, pt[1], 0.0]];
        vec![PreviewWire::with_color(pts, [0.95, 0.6, 0.15, 1.0]), PreviewWire::with_color(foot, [0.4, 0.8, 1.0, 1.0])]
    }
}

/// The route of a selected polyline or line, in drawing coordinates.
fn selected_route(host: &mut dyn HostApi) -> Option<Vec<[f64; 2]>> {
    use ocs_plugin_api::host::codec::EntityType;
    let sel = host.selection();
    if sel.len() != 1 {
        return None;
    }
    match host.document().get_entity(sel[0])? {
        EntityType::LwPolyline(l) if l.vertices.len() >= 2 => {
            let mut pts: Vec<[f64; 2]> = l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect();
            if l.is_closed {
                pts.push(pts[0]);
            }
            Some(pts)
        }
        EntityType::Line(l) => Some(vec![[l.start.x, l.start.y], [l.end.x, l.end.y]]),
        _ => None,
    }
}

/// AHN outputs from a ribbon button or the command line.
fn ahn_command(host: &mut dyn HostApi, verb: &str) {
    let Some(site) = require_site(host) else { return };
    let verb = verb.to_string();
    host.push_info(&format!("[Open Infra] {verb} …"));
    std::thread::spawn(move || {
        let opt = hoogte::settings();
        let progress = |_: String| {};
        let result = match verb.as_str() {
            "GIS_HOOGTEPUNTEN" => hoogte::spot_heights(&site, &opt, &progress),
            "GIS_HOOGTELIJNEN" => hoogte::contour_lines(&site, &opt, &progress),
            _ => hoogte::point_cloud(&site, &opt, &progress),
        };
        match result {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{verb}: {e}")),
        }
    });
}

/// `GIS_HOEVEELHEDEN_TABEL`: pick the top-left corner of the bill of
/// quantities.
struct TableCmd;

impl InteractiveCommand for TableCmd {
    fn prompt(&self) -> String {
        "Hoeveelhedenstaat: linkerbovenhoek van de tabel".into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let at = [pt[0], pt[1]];
        std::thread::spawn(move || match quantities_table(at) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("Hoeveelhedenstaat: {e}")),
        });
        CommandStep::Done
    }
}

/// The bill of quantities of the selection (or the whole drawing) at `at`.
fn quantities_table(at: [f64; 2]) -> Result<String, String> {
    let t = bridge::target().ok_or("geen tekening actief")?;
    let doc = bridge::snapshot()?;
    let sel: std::collections::HashSet<_> = bridge::selection().unwrap_or_default().into_iter().collect();
    let q = hoeveelheden::compute(&doc, t.unit_factor, if sel.is_empty() { None } else { Some(&sel) });
    let rows = hoeveelheden::per_object(&q);
    if rows.is_empty() {
        return Err("geen elementen op NLCS-lagen".into());
    }
    let scale = basisdata::NlcsOptions::default().scale;
    let k = scale / 1000.0 * t.unit_factor;
    let mut ents = Vec::new();
    for (spec, e) in hoeveelheden::table(&rows, at, k, "XX") {
        bridge::ensure_nlcs_layer(&spec);
        ents.extend(e);
    }
    let n = bridge::add_entities("Hoeveelhedenstaat", ents)?;
    Ok(format!("Hoeveelhedenstaat: {} regels ({} elementen{}).", rows.len(), n, if sel.is_empty() { "" } else { ", van de selectie" }))
}

/// Validation, repair and quantities without the window: summary on the
/// command line, report or CSV beside the drawing.
fn nlcs_command(host: &mut dyn HostApi, verb: &str) {
    let verb = verb.to_string();
    host.push_info(&format!("[Open Infra] {verb} …"));
    std::thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            let t = bridge::target().ok_or("geen tekening actief")?;
            let doc = bridge::snapshot()?;
            let opt = nlcs_check::Options { unit_factor: t.unit_factor, layer_props: !bridge::partial(), ..Default::default() };
            match verb.as_str() {
                "GIS_NLCS_HERSTEL" => {
                    let r = nlcs_check::check(&doc, &opt);
                    nlcs_check::repair(&doc, &r.repairs())
                }
                "GIS_HOEVEELHEDEN" => {
                    let q = hoeveelheden::compute(&doc, t.unit_factor, None);
                    let path = bridge::beside_drawing("hoeveelheden.csv");
                    std::fs::write(&path, hoeveelheden::csv(&q)).map_err(|e| format!("{}: {e}", path.display()))?;
                    let rows = hoeveelheden::per_object(&q);
                    let top: Vec<String> = rows.iter().take(6).map(|r| {
                        let (v, u) = r.main();
                        format!("{} ({}) {v:.1} {u}", r.object_label(), hoeveelheden::status_label(&r.status))
                    }).collect();
                    Ok(format!("Hoeveelheden van {} elementen in {}: {}{}", q.entities, path.display(), top.join("; "), if rows.len() > 6 { " …" } else { "" }))
                }
                _ => {
                    let r = nlcs_check::check(&doc, &opt);
                    let path = bridge::beside_drawing("nlcs-controle.md");
                    std::fs::write(&path, nlcs_check::markdown(&r, &bridge::drawing_name(), &opt)).map_err(|e| format!("{}: {e}", path.display()))?;
                    Ok(format!(
                        "NLCS-controle: {} conform — {}, {}, {}. Rapport: {}",
                        r.score_label(),
                        nlcs_check::counted(r.count(nlcs_check::Severity::Fout), "fout", "fouten"),
                        nlcs_check::counted(r.count(nlcs_check::Severity::Waarschuwing), "waarschuwing", "waarschuwingen"),
                        nlcs_check::counted(r.count(nlcs_check::Severity::Info), "melding", "meldingen"),
                        path.display()
                    ))
                }
            }
        })();
        match result {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{verb}: {e}")),
        }
    });
}

struct OpenInfra;

/// `GIS_BGT_MUTATIE [BRONHOUDER=G0505] [GEREALISEERD] [SELECTIE]`: the NLCS
/// design as a BGT mutation (IMGeo-GML 2.1.1) with a report, beside the drawing.
fn mutation_command(host: &mut dyn HostApi, cmd: &str) {
    if require_site(host).is_none() {
        return;
    }
    let args = civiel::api::Args::parse(cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or(""));
    let flag = |w: &str| args.words.iter().any(|x| x.eq_ignore_ascii_case(w));
    let only = if flag("SELECTIE") {
        match bridge::selection() {
            Ok(h) if !h.is_empty() => Some(h.into_iter().collect()),
            _ => {
                host.push_error("[Open Infra] GIS_BGT_MUTATIE SELECTIE: selecteer eerst de objecten.");
                bridge::reply(false, "niets geselecteerd");
                return;
            }
        }
    } else {
        None
    };
    let bronhouder = args.str("BRONHOUDER").unwrap_or("").to_string();
    let as_built = flag("GEREALISEERD");
    host.push_info("[Open Infra] BGT-mutatie maken …");
    std::thread::spawn(move || match bgt_mutatie::export(&bronhouder, as_built, only) {
        Ok(m) => bridge::info(&m),
        Err(e) => bridge::error(&format!("GIS_BGT_MUTATIE: {e}")),
    });
}

/// `GIS_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee]`: the drawing as IFC 4.3
/// (IFC4X3_ADD2), beside it as `<tekening>_IFC4X3.ifc`.
fn ifc_command(host: &mut dyn HostApi, cmd: &str) {
    let args = civiel::api::Args::parse(cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or(""));
    let (path, selection, texts) = ifc::args(&args);
    host.push_info("[Open Infra] IFC 4x3 exporteren …");
    std::thread::spawn(move || {
        let _ = civiel::api::take();
        match ifc::command(path, selection, texts, &|_| {}) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("GIS_IFC: {e}")),
        }
    });
}

/// `GIS_Q_SERVER <id> [LAAG=] [RES=m/px]` (or `JAAR=` for Topotijdreis):
/// import any server of the catalogue by id, for scripts and agents.
fn server_command(host: &mut dyn HostApi, cmd: &str) {
    let words: Vec<&str> = cmd.split_whitespace().skip(1).collect();
    let kv = |k: &str| words.iter().find_map(|w| w.split_once('=').filter(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.to_string()));
    let id = match kv("JAAR") {
        Some(y) => format!("topotijdreis-{y}"),
        None => match words.iter().find(|w| !w.contains('=')) {
            Some(id) => id.to_string(),
            None => {
                host.push_error("[Open Infra] GIS_Q_SERVER <id> [LAAG=] [RES=] of GIS_Q_SERVER JAAR=<1815–2014>");
                bridge::reply(false, "GIS_Q_SERVER <id> [LAAG=] [RES=]");
                return;
            }
        },
    };
    let Some(site) = require_site(host) else { return };
    let layer = kv("LAAG");
    let res = kv("RES").and_then(|r| r.replace(',', ".").parse::<f64>().ok());
    host.push_info(&format!("[Open Infra] {id} ophalen…"));
    std::thread::spawn(move || match import::run_server(&site, &id, layer.as_deref(), res, &|_| {}) {
        Ok(m) => bridge::info(&m),
        Err(e) => bridge::error(&format!("GIS_Q_SERVER {id}: {e}")),
    });
}

fn require_site(host: &mut dyn HostApi) -> Option<Site> {
    let s = bridge::site();
    if s.is_none() {
        host.push_error("[Open Infra] Stel eerst de projectlocatie in: GIS → Locatie.");
        bridge::reply(false, "geen projectlocatie: GIS_SITE eerst");
        let _ = ui::open(ui::Tab::Location);
    }
    s
}

fn open(host: &mut dyn HostApi, tab: ui::Tab) {
    if let Err(e) = ui::open(tab) {
        host.push_error(&format!("[Open Infra] {e}"));
    }
}

fn quick(host: &mut dyn HostApi, q: &'static Quick, label: &'static str) {
    let Some(site) = require_site(host) else { return };
    host.push_info(&format!("[Open Infra] {label} ophalen…"));
    std::thread::spawn(move || {
        let progress = |_: String| {};
        let result = match q {
            Quick::Data(d) => import::run_dataset(&site, d, &progress),
            Quick::Osm => {
                let ids: Vec<&str> = gis::osm::THEMES.iter().filter(|t| t.default_on).map(|t| t.id).collect();
                import::osm(&site, &ids, true, 2.0, true, &progress)
            }
        };
        match result {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{label}: {e}")),
        }
    });
}

/// `GIS_SITE <lon> <lat> [breedte] [hoogte] [EPSG:code] [RELATIEF] [naam]` or
/// `GIS_SITE RD <x> <y> [breedte] [hoogte] [RELATIEF] [naam]`: set the project
/// location from a script, without the window.
fn site_command(host: &mut dyn HostApi, cmd: &str) {
    // The units may have changed since the drawing was first used.
    let tab = host.tab_id();
    bridge::units(host, tab, true);
    bridge::attach(host);
    let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
    let num = |i: usize| args.get(i).and_then(|a| a.replace(',', ".").parse::<f64>().ok());
    let mut site = bridge::site().unwrap_or_else(||Site::for_region(localisation::current()));
    let rd = args.first().is_some_and(|a| a.eq_ignore_ascii_case("RD"));
    let at = if rd { 1 } else { 0 };
    let (Some(a), Some(b)) = (num(at), num(at + 1)) else {
        host.push_error("[Open Infra] GIS_SITE <lon> <lat> [breedte] [hoogte] [EPSG:code]  of  GIS_SITE RD <x> <y> [breedte] [hoogte]");
        bridge::reply(false, "GIS_SITE <lon> <lat> [breedte] [hoogte] [EPSG:code] of GIS_SITE RD <x> <y> [breedte] [hoogte]");
        return;
    };
    if !rd && (a.abs() > 180.0 || b.abs() > 90.0) {
        let msg = format!("GIS_SITE {a} {b}: dat zijn geen lengte- en breedtegraad. Voor RD-coördinaten: GIS_SITE RD {a} {b} …");
        host.push_error(&format!("[Open Infra] {msg}"));
        bridge::reply(false, &msg);
        return;
    }
    if let Some(code) = args.iter().find(|a| a.to_ascii_uppercase().starts_with("EPSG:")) {
        site.crs = code.to_ascii_uppercase();
    } else if rd {
        site.crs = "EPSG:28992".into();
    }
    site.width = num(at + 2).unwrap_or(site.width);
    site.height = num(at + 3).unwrap_or(site.width);
    // RELATIEF: the base point becomes 0,0 in the drawing; ABSOLUUT: CRS
    // coordinates (the default).
    let keyword = |k: &str| args.iter().any(|a| a.eq_ignore_ascii_case(k));
    if keyword("RELATIEF") {
        site.absolute = false;
    } else if keyword("ABSOLUUT") {
        site.absolute = true;
    }
    // Words after the numbers name the location (title block, window).
    let name: Vec<&str> = args
        .iter()
        .skip(at + 2)
        .filter(|a| a.replace(',', ".").parse::<f64>().is_err() && !a.to_ascii_uppercase().starts_with("EPSG:") && !a.eq_ignore_ascii_case("RELATIEF") && !a.eq_ignore_ascii_case("ABSOLUUT"))
        .copied()
        .collect();
    site.label = if !name.is_empty() {
        name.join(" ")
    } else if rd {
        format!("RD {a:.0} {b:.0}")
    } else {
        format!("{b:.5} N {a:.5} E")
    };
    // A CRS that is not built in comes from epsg.io first (off the host's
    // thread); without it the origin would keep the previous location.
    std::thread::spawn(move || {
        if let Err(e) = gis::crs::ensure(&site.crs) {
            return bridge::error(&format!("GIS_SITE: {}: {e}", site.crs));
        }
        if rd {
            site.reproject_origin();
            site.set_origin(a, b);
        } else {
            site.lon = a;
            site.lat = b;
            site.reproject_origin();
        }
        save_site_reply(site)
    });
}

fn save_site_reply(site: Site) {
    match bridge::save_site(&site) {
        Ok(()) => {
            civiel::api::note("projectlocatie", serde_json::json!({"naam": site.label, "crs": site.crs, "x": site.origin_e, "y": site.origin_n, "breedte": site.width, "hoogte": site.height, "absoluut": site.absolute}));
            bridge::info(&format!("Projectlocatie {}: X {:.2} Y {:.2} ({}), {} × {} m.", site.label, site.origin_e, site.origin_n, site.crs, site.width, site.height));
            ui::send(ui::UiMsg::SiteChanged);
        }
        Err(e) => bridge::error(&format!("GIS_SITE: {e}")),
    }
}

/// Base data from the command line or a ribbon button, without the window.
fn base_data(host: &mut dyn HostApi, verb: &str, arg: &str) {
    let Some(site) = require_site(host) else { return };
    let verb = verb.to_string();
    let arg = arg.trim().trim_matches('"').to_string();
    host.push_info(&format!("[Open Infra] {verb} …"));
    std::thread::spawn(move || {
        let progress = |_: String| {};
        let opt = basisdata::NlcsOptions::default();
        let result = match verb.as_str() {
            "GIS_Q_BGT" => {
                let cols: Vec<&str> = gis::bgt::GROUPS.iter().filter(|g| g.default_on).flat_map(|g| g.collections.iter().copied()).collect();
                basisdata::bgt(&site, &cols, &opt, &progress)
            }
            "GIS_Q_BRK" => basisdata::cadastre(&site, &opt, &progress),
            "GIS_KLIC_IMPORT" => basisdata::klic(&site, std::path::Path::new(&arg), &basisdata::KlicOptions::default()),
            _ => {
                let mut info = tekenblad::SheetInfo::default();
                if !arg.is_empty() {
                    info.subject = arg.clone();
                }
                tekenblad::draw(&site, &opt, &info)
            }
        };
        match result {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{verb}: {e}")),
        }
    });
}

/// Command words without the point answers a script may append (`10,20`).
fn words(cmd: &str) -> Vec<&str> {
    cmd.split_whitespace()
        .skip(1)
        .filter(|a| !(a.contains(',') && a.split(',').all(|n| n.trim().parse::<f64>().is_ok())))
        .collect()
}

/// A drawing without an Open Infra site but with a GEODATA object (set in
/// other CAD programs) gets its site from that object, once per
/// tab. The drawing is read on a worker through the bridge, never with
/// `host.document()` in dispatch (that stalls dispatch on large drawings); a
/// drawing too large for a full snapshot is skipped.
fn adopt_geodata(host: &mut dyn HostApi) {
    use ocs_plugin_api::host::codec::objects::ObjectType;
    if bridge::site().is_some() {
        return;
    }
    let tab = host.tab_id();
    static TRIED: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
    if TRIED.lock().unwrap().contains(&tab) {
        return;
    }
    TRIED.lock().unwrap().push(tab);
    std::thread::spawn(move || {
        let Ok(doc) = bridge::snapshot() else { return };
        if bridge::partial() {
            return;
        }
        let found = doc.objects.values().find_map(|o| match o {
            ObjectType::GeoData(g) => Some(gis::geodata::GeoLocation {
                coordinate_type: g.coordinate_type,
                design_point: [g.design_point.x, g.design_point.y],
                reference_point: [g.reference_point.x, g.reference_point.y],
                definition: if g.coordinate_system_definition.is_empty() { g.coordinate_system_wkt.clone() } else { g.coordinate_system_definition.clone() },
                unit_scale: g.horizontal_unit_scale,
                north: [g.north_direction.x, g.north_direction.y],
            }),
            _ => None,
        });
        let Some(g) = found else { return };
        let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
        match gis::geodata::site_from(&g, unit).and_then(|mut site| {site.region=localisation::current();bridge::save_site(&site).map(|_| site)}) {
            Ok(site) => {
                bridge::info(&format!("{} {} ({}).", lang::tr("Projectlocatie overgenomen uit het GEODATA-object van de tekening:", "Project location taken from the drawing's GEODATA object:"), site.crs, if site.absolute { lang::tr("absoluut", "absolute") } else { lang::tr("relatief", "relative") }));
                ui::send(ui::UiMsg::SiteChanged);
            }
            Err(e) => bridge::info(&e),
        }
    });
}

/// `GIS_BESTAND_IMPORT <pad> [EPSG:code]`: a GIS file straight into the drawing.
fn file_import(host: &mut dyn HostApi, path: &str, crs: Option<String>) {
    let Some(site) = require_site(host) else { return };
    if path.is_empty() {
        host.push_error("[Open Infra] GIS_FILE_IMPORT <path> [EPSG:code]");
        return;
    }
    let path = std::path::PathBuf::from(path);
    host.push_info(&format!("[Open Infra] {} {} …", lang::tr("Importeren", "Importing"), path.display()));
    std::thread::spawn(move || match bestand::import_path(&site, &path, crs.as_deref(), &|_| {}) {
        Ok(m) => bridge::info(&m),
        Err(e) => bridge::error(&format!("{}: {e}", path.display())),
    });
}

/// The point `GIS_SERVICES` filters on: `LON=`/`LAT=`, or the site with `HIER`.
fn services_location(args: &civiel::api::Args) -> Result<Option<[f64; 2]>, String> {
    let (lon, lat) = (args.f64("LON")?, args.f64("LAT")?);
    if let (Some(lon), Some(lat)) = (lon, lat) {
        return Ok(Some([lon, lat]));
    }
    if args.words.iter().any(|w| w.eq_ignore_ascii_case("HIER") || w.eq_ignore_ascii_case("HERE")) {
        let s = bridge::site().ok_or("HIER: geen projectlocatie; geef LON= en LAT= of stel eerst GIS_SITE in")?;
        return Ok(Some([s.lon, s.lat]));
    }
    Ok(None)
}

/// `GIS_SERVICES [ZOEK=<tekst>] [CATEGORIE=<c>[;<c>]] [TYPE=WMS|WFS|XYZ] [HIER | LON= LAT=] [LIMIET=50]`:
/// search the service catalogue; the matches come back as JSON (`ID=`).
/// Without arguments it opens the service browser.
fn services_command(host: &mut dyn HostApi, cmd: &str) {
    let rest = cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or("");
    let args = civiel::api::Args::parse(rest);
    if args.is_empty() {
        open(host, ui::Tab::Services);
        bridge::reply(true, "servicebrowser geopend");
        return;
    }
    let result = (|| -> Result<String, String> {
        use gis::catalog;
        let mut text: Vec<String> = ["ZOEK", "SEARCH", "Q"].iter().filter_map(|k| args.str(k)).map(str::to_string).collect();
        text.extend(args.words.iter().filter(|w| !w.eq_ignore_ascii_case("HIER") && !w.eq_ignore_ascii_case("HERE")).cloned());
        let mut categories = Vec::new();
        for c in args.str("CATEGORIE").or(args.str("CATEGORY")).unwrap_or("").split([';', ',']).map(|c| catalog::fold(c.trim())).filter(|c| !c.is_empty()) {
            let found = catalog::categories_for(&c);
            if found.is_empty() {
                return Err(format!("onbekende categorie '{c}'; bekend: {}", catalog::CATEGORIES.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")));
            }
            categories.extend(found.into_iter().map(str::to_string));
        }
        let mut kinds = Vec::new();
        for t in args.str("TYPE").unwrap_or("").split([';', ',']).map(str::trim).filter(|t| !t.is_empty()) {
            kinds.push(match t.to_ascii_uppercase().as_str() {
                "WMS" => gis::servers::ServerKind::Wms,
                "WFS" => gis::servers::ServerKind::Wfs,
                "XYZ" | "TMS" | "TILES" => gis::servers::ServerKind::Xyz,
                other => return Err(format!("TYPE={other}: kies WMS, WFS of XYZ")),
            });
        }
        let at = services_location(&args)?;
        let limit = args.f64("LIMIET")?.or(args.f64("LIMIT")?).unwrap_or(50.0).max(1.0) as usize;
        let all = catalog::all();
        let countries=args.str("LAND").or(args.str("COUNTRY")).unwrap_or("").split([';',',']).map(str::trim).filter(|s|!s.is_empty()).map(str::to_uppercase).collect();
        let hits = catalog::search(&all, &catalog::Query { text: text.join(" "), categories, kinds, at, countries });
        let list: Vec<serde_json::Value> = hits.iter().take(limit).map(|&i| all[i].json(at)).collect();
        civiel::api::note("count", hits.len());
        civiel::api::note("services", list);
        if let Some(p) = at {
            civiel::api::note("location", serde_json::json!({"lon": p[0], "lat": p[1]}));
        }
        let names: Vec<String> = hits.iter().take(5).map(|&i| format!("{} ({}, {})", all[i].name, all[i].kind.label(), all[i].country)).collect();
        Ok(format!("{} {}{}{}", hits.len(), lang::tr("diensten gevonden", "services found"), if names.is_empty() { "" } else { ": " }, names.join("; ")))
    })();
    match result {
        Ok(m) => {
            host.push_info(&format!("[Open Infra] {m}"));
            bridge::reply(true, &m);
        }
        Err(e) => {
            host.push_error(&format!("[Open Infra] GIS_SERVICES: {e}"));
            bridge::reply(false, &e);
        }
    }
}

/// `GIS_SERVICE_LOAD DIENST=<id> [LAAG=<laag>] [RESOLUTIE=0.25] [LIMIET=5000]`: one
/// layer of a catalogue service (id from `GIS_SERVICES`) into the project area.
fn service_load_command(host: &mut dyn HostApi, cmd: &str) {
    let Some(site) = require_site(host) else { return };
    let rest = cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or("");
    let args = civiel::api::Args::parse(rest);
    let id = args.str("DIENST").or(args.str("SERVICE")).or(args.words.first().map(String::as_str)).unwrap_or("").to_string();
    let Some(service) = gis::catalog::all().into_iter().find(|s| s.id.eq_ignore_ascii_case(&id)) else {
        let msg = format!("GIS_SERVICE_LOAD DIENST=<id> [LAAG=<laag>]: {} '{id}' ({})", lang::tr("onbekende dienst", "unknown service"), lang::tr("zie GIS_SERVICES", "see GIS_SERVICES"));
        host.push_error(&format!("[Open Infra] {msg}"));
        bridge::reply(false, &msg);
        return;
    };
    let layer = args.str("LAAG").or(args.str("LAYER")).map(str::to_string);
    let num = |nl: &str, en: &str| -> Result<Option<f64>, String> { Ok(args.f64(nl)?.or(args.f64(en)?)) };
    let (res, limit) = match (num("RESOLUTIE", "RESOLUTION"), num("LIMIET", "LIMIT")) {
        (Ok(r), Ok(l)) => (r.unwrap_or(if service.kind == gis::servers::ServerKind::Xyz { 0.5 } else { 0.25 }).max(0.02), l.unwrap_or(5000.0).max(1.0) as usize),
        (Err(e), _) | (_, Err(e)) => {
            host.push_error(&format!("[Open Infra] GIS_SERVICE_LOAD: {e}"));
            bridge::reply(false, &e);
            return;
        }
    };
    host.push_info(&format!("[Open Infra] {} {} …", service.name, lang::tr("laden", "loading")));
    let server = service.to_map_server();
    std::thread::spawn(move || match import::service_layer(&site, &server, None, None, layer.as_deref(), res, limit, &|_| {}) {
        Ok(m) => bridge::info(&m),
        Err(e) => bridge::error(&format!("GIS_SERVICE_LOAD {}: {e}", server.id)),
    });
}

/// `GIS_VOERTUIG <id> [ZIJ|SIDE]`: insert a vehicle block at a picked point.
fn vehicle_command(host: &mut dyn HostApi, cmd: &str) {
    let args = words(cmd);
    let side = args.iter().any(|a| a.eq_ignore_ascii_case("ZIJ") || a.eq_ignore_ascii_case("SIDE"));
    let id = args.iter().find(|a| !a.eq_ignore_ascii_case("ZIJ") && !a.eq_ignore_ascii_case("SIDE")).copied().unwrap_or("");
    if id.is_empty() {
        open(host, ui::Tab::Library);
        return;
    }
    let Some(v) = library::find(id) else {
        host.push_error(&format!("[Open Infra] {} {id} — {}", lang::tr("onbekend voertuig", "unknown vehicle"), lang::tr("zie GIS_VOERTUIGEN", "see GIS_VEHICLES")));
        return;
    };
    let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let view = if side { library::draw::View::Side } else { library::draw::View::Top };
    host.start_interactive(Box::new(library::place::InsertCmd::new(v, view, unit)));
}

/// `GIS_VEEGPAD <id> [interval]`: drive a vehicle along a selected polyline
/// (or picked points) and draw its swept path.
fn swept_command(host: &mut dyn HostApi, cmd: &str) {
    let args = words(cmd);
    let Some(v) = args.first().and_then(|id| library::find(id)) else {
        host.push_error(&format!("[Open Infra] GIS_SWEPT_PATH <{}> [{}]", lang::tr("voertuig", "vehicle"), lang::tr("interval in m", "interval in m")));
        open(host, ui::Tab::Library);
        return;
    };
    let every = args.get(1).and_then(|a| a.replace(',', ".").parse::<f64>().ok()).unwrap_or(10.0);
    let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let mut sc = library::place::SweptCmd { vehicle: v, every, path: Vec::new(), unit };
    match selected_route(host) {
        Some(route) => {
            sc.path = route;
            host.push_info(&format!("[Open Infra] {}", lang::tr("Veegpad langs de geselecteerde lijn.", "Swept path along the selected line.")));
            library::place::swept_now(&sc);
        }
        None => host.start_interactive(Box::new(sc)),
    }
}

/// `GIS_VOERTUIGOVERZICHT [categorie] [ZIJ|SIDE]`: lay out the library.
fn catalogue_command(host: &mut dyn HostApi, cmd: &str) {
    let args = words(cmd);
    let side = args.iter().any(|a| a.eq_ignore_ascii_case("ZIJ") || a.eq_ignore_ascii_case("SIDE"));
    let cat = args.iter().find(|a| !a.eq_ignore_ascii_case("ZIJ") && !a.eq_ignore_ascii_case("SIDE")).map(|a| a.to_ascii_lowercase());
    let list: Vec<&'static library::Vehicle> = library::vehicles().iter().filter(|v| cat.as_deref().is_none_or(|c| v.category == c)).collect();
    if list.is_empty() {
        host.push_error(&format!("[Open Infra] {}", lang::tr("geen voertuigen in die categorie", "no vehicles in that category")));
        return;
    }
    let view = if side { library::draw::View::Side } else { library::draw::View::Top };
    host.start_interactive(Box::new(library::place::CatalogueCmd { list, view }));
}

impl BuiltinPlugin for OpenInfra {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn ribbon(&self) -> Box<dyn CadModule> {
        Box::new(GisModule)
    }

    fn on_load(&mut self, host: &mut dyn HostApi) {
        bridge::attach(host);
    }

    fn dispatch(&self, host: &mut dyn HostApi, cmd: &str) -> bool {
        let verb = cmd.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        if !verb.starts_with("GIS_") {
            return false;
        }
        bridge::attach(host);
        // Scripts: ID=<name> answers in <config>/api/<name>.json.
        let (cmd_owned, id) = bridge::split_id(cmd);
        let cmd = cmd_owned.as_str();
        bridge::expect_reply(id);
        adopt_geodata(host);
        match verb.as_str() {
            "GIS_INFRA_LIBRARY" | "GIS_INFRA_BIBLIOTHEEK" => { open(host, ui::Tab::Library); ui::send(ui::UiMsg::InfraLibrary); bridge::reply(true, "Infra-bibliotheek geopend"); }
            "GIS_INFRA_CATALOG" | "GIS_INFRA_CATALOGUS" | "GIS_INFRA_PLACE" | "GIS_INFRA_PLAATS" | "GIS_INFRA_INSPECT" | "GIS_INFRA_UPDATE" => library::infra::commands::dispatch(host, cmd),
            "GIS_LOCATIE" | "GIS_WERKBANK" => open(host, ui::Tab::Location),
            "GIS_KAART" | "GIS_WMS" => open(host, ui::Tab::Raster),
            "GIS_WFS" => open(host, ui::Tab::Wfs),
            "GIS_OSM" => open(host, ui::Tab::Osm),
            "GIS_PDOK" => open(host, ui::Tab::Pdok),
            "GIS_LOCALISATION" | "GIS_LOCALISATIE" => open(host,ui::Tab::Localisation),
            "GIS_US_SERVICES" => {open(host,ui::Tab::Services);ui::send(ui::UiMsg::RegionalServices(localisation::Region::UnitedStates));bridge::reply(true,"US services");}
            "GIS_PLANNEN" | "GIS_PLANS" => open(host, ui::Tab::Planning),
            "GIS_PLAN_SEARCH" | "GIS_PLAN_IMPORT" | "GIS_PLAN_GML" => planning_commands::dispatch(cmd),
            "GIS_SERVERS" => open(host, ui::Tab::Servers),
            "GIS_SERVICES" | "GIS_DIENSTEN" => services_command(host, cmd),
            "GIS_SERVICE_LOAD" | "GIS_DIENST_LADEN" => service_load_command(host, cmd),
            "GIS_COORD" => open(host, ui::Tab::Coords),
            "GIS_BGT" | "GIS_NLCS" => open(host, ui::Tab::Nlcs),
            "GIS_AHN" => open(host, ui::Tab::Ahn),
            "GIS_NLCS_CONTROLE" | "GIS_HOEVEELHEDEN" => {
                // The window shows the result; without it, the command line.
                let tab = if verb == "GIS_HOEVEELHEDEN" { ui::Tab::Hoeveelheden } else { ui::Tab::Controle };
                if ui::open(tab).is_ok() {
                    ui::send(ui::UiMsg::Run(tab));
                } else {
                    nlcs_command(host, &verb);
                }
            }
            "GIS_NLCS_RAPPORT" | "GIS_NLCS_HERSTEL" | "GIS_HOEVEELHEDEN_CSV" => {
                let v = match verb.as_str() {
                    "GIS_NLCS_RAPPORT" => "GIS_NLCS_CONTROLE",
                    "GIS_HOEVEELHEDEN_CSV" => "GIS_HOEVEELHEDEN",
                    v => v,
                };
                nlcs_command(host, v);
            }
            "GIS_HOEVEELHEDEN_TABEL" => host.start_interactive(Box::new(TableCmd)),
            "GIS_BGT_MUTATIE" => mutation_command(host, cmd),
            "GIS_IFC" => ifc_command(host, cmd),
            "GIS_HOOGTEPUNTEN" | "GIS_HOOGTELIJNEN" | "GIS_PUNTENWOLK" => ahn_command(host, &verb),
            "GIS_PROFIEL" => {
                let Some(site) = require_site(host) else { return true };
                site.register_crs();
                match selected_route(host) {
                    Some(route) => {
                        host.push_info("[Open Infra] Profiel langs de geselecteerde lijn.");
                        host.start_interactive(Box::new(ProfileCmd { route, picking_route: false, site }));
                    }
                    None => host.start_interactive(Box::new(ProfileCmd { route: Vec::new(), picking_route: true, site })),
                }
            }
            "GIS_SITE" => site_command(host, cmd),
            "GIS_Q_SERVER" => server_command(host, cmd),
            "GIS_Q_BGT" | "GIS_Q_BRK" | "GIS_TEKENBLAD" | "GIS_KLIC_IMPORT" => {
                let arg = cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or("");
                base_data(host, &verb, arg);
            }
            "GIS_KLIC" => {
                // From the ribbon's file dialog: "GIS_KLIC <path>" (original case).
                let arg = cmd.split_once(char::is_whitespace).map(|(_, a)| a.trim().trim_matches('"')).unwrap_or("");
                if arg.is_empty() {
                    open(host, ui::Tab::Nlcs);
                } else if ui::open(ui::Tab::Nlcs).is_ok() {
                    ui::send(ui::UiMsg::Klic(arg.into()));
                } else {
                    base_data(host, "GIS_KLIC_IMPORT", arg);
                }
            }
            "GIS_RASTER_IMPORT" => {
                // GeoTIFF, ASC, JPG/PNG with a world file, LAS/LAZ.
                let arg = cmd.split_once(char::is_whitespace).map(|(_, a)| a.trim().trim_matches('"').to_string()).unwrap_or_default();
                let Some(site) = require_site(host) else { return true };
                if arg.is_empty() {
                    host.push_error(&format!("[Open Infra] GIS_RASTER_IMPORT <bestand> ({})", gis::rasterbestand::EXTENSIONS.join(", ")));
                    bridge::reply(false, "geef een bestand");
                } else {
                    host.push_info("[Open Infra] GIS_RASTER_IMPORT …");
                    std::thread::spawn(move || match gis::rasterbestand::import_path(&site, std::path::Path::new(&arg), &|_| {}) {
                        Ok(m) => bridge::info(&m),
                        Err(e) => bridge::error(&format!("GIS_RASTER_IMPORT: {e}")),
                    });
                }
            }
            "GIS_GEBIED" | "GIS_PUNT" => {
                let site = bridge::site().unwrap_or_default();
                site.register_crs();
                let unit_factor = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
                if verb == "GIS_GEBIED" {
                    host.start_interactive(Box::new(AreaCmd { first: None, site, unit_factor }));
                } else {
                    host.start_interactive(Box::new(PointCmd { site, unit_factor }));
                }
            }
            "GIS_VOERTUIGEN" | "GIS_VEHICLES" | "GIS_BIBLIOTHEEK" | "GIS_LIBRARY" => open(host, ui::Tab::Library),
            "GIS_BESTAND" | "GIS_FILE" => {
                // From the ribbon's file dialog: "GIS_BESTAND <path>" (original case).
                let arg = cmd.split_once(char::is_whitespace).map(|(_, a)| a.trim().trim_matches('"')).unwrap_or("");
                // One file dialog for all geodata: vector files go to the GIS
                // file page, rasters and point clouds to their own readers.
                let kind = gis::files::kind(std::path::Path::new(arg));
                if !arg.is_empty() && matches!(kind, gis::files::Kind::Raster | gis::files::Kind::PointCloud) {
                    let Some(site) = require_site(host) else { return true };
                    let path = std::path::PathBuf::from(arg);
                    host.push_info(&format!("[Open Infra] {} {} …", lang::tr("Importeren", "Importing"), path.display()));
                    std::thread::spawn(move || match gis::rasterbestand::import_path(&site, &path, &|_| {}) {
                        Ok(m) => bridge::info(&m),
                        Err(e) => bridge::error(&format!("{}: {e}", path.display())),
                    });
                } else if !arg.is_empty() && kind == gis::files::Kind::Unknown {
                    host.push_error(&format!("[Open Infra] {} {arg}", lang::tr("Dit bestandstype kan Open Infra niet openen:", "Open Infra cannot open this file type:")));
                } else if arg.is_empty() {
                    open(host, ui::Tab::Files);
                } else if ui::open(ui::Tab::Files).is_ok() {
                    ui::send(ui::UiMsg::File(arg.into()));
                } else {
                    file_import(host, arg, None);
                }
            }
            "GIS_BGT_BIJWERKEN" | "GIS_BGT_UPDATE" => {
                let Some(site) = require_site(host) else { return true };
                host.push_info(&format!("[Open Infra] {} …", lang::tr("BGT bijwerken", "Updating the BGT")));
                std::thread::spawn(move || match bijwerken::bgt_update(&site, &basisdata::NlcsOptions::default(), &|_| {}) {
                    Ok(m) => bridge::info(&m),
                    Err(e) => bridge::error(&format!("BGT: {e}")),
                });
            }
            "GIS_Q_GWSW" | "GIS_GWSW" => {
                let Some(site) = require_site(host) else { return true };
                host.push_info(&format!("[Open Infra] GWSW {} …", lang::tr("riolering ophalen", "sewers")));
                std::thread::spawn(move || {
                    let cols: Vec<&str> = gis::gwsw::COLLECTIONS.iter().map(|(c, _)| *c).collect();
                    match riolering::import(&site, &cols, &basisdata::NlcsOptions::default(), &|_| {}) {
                        Ok(m) => bridge::info(&m),
                        Err(e) => bridge::error(&format!("GWSW: {e}")),
                    }
                });
            }
            "GIS_Q_SONDERINGEN" | "GIS_SONDERINGEN" => {
                // The CPTs and boreholes of the BRO in plan; BRON=sonderingen|boringen.
                if require_site(host).is_none() {
                    return true;
                }
                let args = civiel::api::Args::parse(cmd.split_once(char::is_whitespace).map(|(_, a)| a).unwrap_or(""));
                let kinds = match civiel::ondergrond::kinds(args.str("BRON")) {
                    Ok(k) => k,
                    Err(e) => {
                        host.push_error(&format!("[Open Infra] GIS_Q_SONDERINGEN: {e}"));
                        bridge::reply(false, &e);
                        return true;
                    }
                };
                host.push_info(&format!("[Open Infra] {} …", lang::tr("Sonderingen en boringen (BRO) ophalen", "Fetching CPTs and boreholes (BRO)")));
                std::thread::spawn(move || match civiel::ondergrond::overview(&civiel::settings(), &kinds, &|_| {}) {
                    Ok(m) => bridge::info(&m),
                    Err(e) => bridge::error(&format!("BRO: {e}")),
                });
            }
            "GIS_BESTAND_IMPORT" | "GIS_FILE_IMPORT" => {
                let rest = cmd.split_once(char::is_whitespace).map(|(_, a)| a.trim()).unwrap_or("");
                let (path, crs) = match rest.rsplit_once(char::is_whitespace) {
                    Some((p, c)) if c.to_ascii_uppercase().starts_with("EPSG:") => (p.trim(), Some(c.to_string())),
                    _ => (rest, None),
                };
                file_import(host, path.trim_matches('"'), crs);
            }
            "GIS_VOERTUIG" | "GIS_VEHICLE" => vehicle_command(host, cmd),
            "GIS_VEEGPAD" | "GIS_SWEPT_PATH" | "GIS_SWEPTPATH" => swept_command(host, cmd),
            "GIS_VOERTUIGOVERZICHT" | "GIS_VEHICLE_CATALOG" | "GIS_VEHICLE_CATALOGUE" => catalogue_command(host, cmd),
            "GIS_HELP" => host.push_info(
                "[Open Infra] GIS_LOCATIE, GIS_GEBIED, GIS_PUNT, GIS_KAART, GIS_WFS, GIS_OSM, GIS_PDOK, GIS_COORD, GIS_SERVERS; \
                 snel: GIS_Q_LUCHTFOTO, GIS_Q_KADASTER, GIS_Q_AHN, GIS_Q_OSMKAART, GIS_Q_SATELLIET, GIS_Q_BAG, \
                 GIS_Q_PERCELEN, GIS_Q_NWB, GIS_Q_HECTO, GIS_Q_SPOOR, GIS_Q_OSM; \
                 GIS_SITE <lon> <lat> [b] [h] [EPSG:code] [RELATIEF] [naam] | GIS_SITE RD <x> <y> [b] [h] [RELATIEF] [naam]; \
                 diensten / services: GIS_SERVICES | GIS_DIENSTEN (venster) of [ZOEK=<tekst>] [CATEGORIE=kadaster] [TYPE=WMS|WFS|XYZ] [HIER | LON= LAT=] [LIMIET=50], \
                 GIS_SERVICE_LOAD | GIS_DIENST_LADEN DIENST=<id> [LAAG=<laag>] [RESOLUTIE=0.25] [LIMIET=5000]; \
                 NLCS: GIS_NLCS_CONTROLE, GIS_NLCS_RAPPORT, GIS_NLCS_HERSTEL, GIS_HOEVEELHEDEN, GIS_HOEVEELHEDEN_CSV, GIS_HOEVEELHEDEN_TABEL, GIS_BGT_MUTATIE [BRONHOUDER=G0505] [GEREALISEERD] [SELECTIE]; \
                 IFC 4x3: GIS_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee]; \
                 AHN: GIS_AHN (venster), GIS_HOOGTEPUNTEN, GIS_HOOGTELIJNEN, GIS_PROFIEL, GIS_PUNTENWOLK; \
                 NLCS: GIS_BGT (venster), GIS_Q_BGT, GIS_BGT_BIJWERKEN | GIS_BGT_UPDATE, GIS_Q_BRK, GIS_Q_GWSW, GIS_Q_SONDERINGEN [BRON=sonderingen|boringen] (BRO), GIS_KLIC [bestand], GIS_KLIC_IMPORT <bestand>, GIS_TEKENBLAD [onderwerp]; \
                 Bibliotheek / library: GIS_VOERTUIGEN | GIS_VEHICLES (venster), GIS_VOERTUIG | GIS_VEHICLE <id> [ZIJ|SIDE], \
                 GIS_VOERTUIGOVERZICHT | GIS_VEHICLE_CATALOG [categorie] [ZIJ|SIDE], \
                 GIS_VEEGPAD | GIS_SWEPT_PATH <id> [interval m] (geselecteerde lijn of punten); \
                 bestanden / files: GIS_BESTAND | GIS_FILE [pad] (venster; vector, raster, puntenwolk), GIS_BESTAND_IMPORT | GIS_FILE_IMPORT <pad> [EPSG:code], \
                 GIS_RASTER_IMPORT <GeoTIFF/ASC/JPG/PNG+world file/LAS/LAZ>; scripts: ID=<naam> → <config>/OpenCADStudio/open-infra/api/<naam>.json.",
            ),
            v => match QUICK.iter().find(|(c, ..)| *c == v) {
                Some((_, label, q)) => quick(host, q, ql(label)),
                None => {
                    host.push_error(&format!("[Open Infra] onbekend commando {v} — zie GIS_HELP"));
                    bridge::reply(false, &format!("onbekend commando {v}"));
                }
            },
        }
        true
    }
}

#[cfg(feature = "plugin")]
ocs_plugin_api::export_plugin!(OpenInfra);
