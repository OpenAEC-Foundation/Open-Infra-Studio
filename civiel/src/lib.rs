//! Open Infra — Civiel: the second ribbon tab, with civil design tools after
//! Hakan Seven's FreeCAD Road workbench. Survey points, surfaces (TIN,
//! contours, height analysis, earthwork), alignments (stationing, offsets,
//! station/offset), length, design and cross sections. The engine is
//! `open_infra::civiel`; this plugin is its ribbon and commands.

use std::sync::{Mutex, OnceLock};

use ocs_plugin_api::host::{BuiltinPlugin, CommandStep, HostApi, InteractiveCommand, PreviewWire};
use ocs_plugin_api::manifest::{ApiVersion, PluginManifest};
use ocs_plugin_api::ribbon::{CadModule, IconKind, ModuleEvent, RibbonGroup, RibbonItem, ToolDef};

use open_infra::civiel::api::{self, Args};
use open_infra::civiel::{self, as_profiel, corridor, landmeten, landxml, riool, terrein};
use open_infra::lang::tr;
use open_infra::{bridge, ui};
// Grading: feature lines, plateaus, slopes, TIN volumes, mass haul.
use open_infra::civiel::{grading, massa};

static MANIFEST: PluginManifest = PluginManifest {
    id: "openaec.open-infra-civiel",
    name: "Open Infra — Civiel",
    version: env!("CARGO_PKG_VERSION"),
    description: "Civiele ontwerptools: meetpunten, terrein (TIN), assen met stationering, lengte-, ontwerp- en dwarsprofielen, grondverzet. Op NLCS-lagen.",
    api_version: ApiVersion::CURRENT,
    ribbon_order: 61,
    xdata_apps: &[bridge::XDATA_APP],
    command_prefixes: &["CIV_"],
};

macro_rules! icon {
    ($f:literal) => {
        IconKind::Svg(include_bytes!(concat!("../../assets/", $f)))
    };
}

const I_POINTS: IconKind = icon!("meetpunten.svg");
const I_EXPORT: IconKind = icon!("punten-export.svg");
const I_PLACE: IconKind = icon!("punt-plaatsen.svg");
const I_LANDXML: IconKind = icon!("landxml.svg");
const I_TIN: IconKind = icon!("tin.svg");
const I_CONTOURS: IconKind = icon!("hoogtelijnen.svg");
const I_ANALYSIS: IconKind = icon!("hoogteanalyse.svg");
const I_VOLUME: IconKind = icon!("grondverzet.svg");
const I_AXIS: IconKind = icon!("as.svg");
const I_OFFSET: IconKind = icon!("as-verschuiven.svg");
const I_STATION: IconKind = icon!("station.svg");
const I_PROFILE: IconKind = icon!("profiel.svg");
const I_DESIGN: IconKind = icon!("ontwerpprofiel.svg");
const I_SECTIONS: IconKind = icon!("dwarsprofielen.svg");
const I_SETTINGS: IconKind = icon!("instellingen.svg");
const I_CORRIDOR: IconKind = icon!("corridor.svg");
const I_INFO: IconKind = icon!("info.svg");
const I_BAG3D: IconKind = icon!("bag3d.svg");
const I_PHOTO: IconKind = icon!("fototerrein.svg");
const I_BGT3D: IconKind = icon!("bgt3d.svg");
const I_WATER: IconKind = icon!("waterpeil.svg");
// Point clouds: TIN from points, 3D mesh.
const I_CLOUD_TIN: IconKind = icon!("puntenwolk-tin.svg");
const I_MESH: IconKind = icon!("mesh.svg");
const I_CITYGML: IconKind = icon!("citygml.svg");
const I_IFC: IconKind = icon!("ifc.svg");
const I_KLIC3D: IconKind = icon!("klic3d.svg");
const I_SEE: IconKind = icon!("doorzicht.svg");
const I_CPT: IconKind = icon!("sonderingen.svg");
// Grading.
const I_FEATURE: IconKind = icon!("kantlijn.svg");
const I_PLATEAU: IconKind = icon!("plateau.svg");
const I_SLOPE: IconKind = icon!("talud.svg");
const I_VOLUME_TIN: IconKind = icon!("grondverzet-tin.svg");
const I_MASS_HAUL: IconKind = icon!("grondbalans.svg");
// Plan production: sheets and labels.
const I_SHEETS: IconKind = icon!("bladen.svg");
const I_SHEETS_PDF: IconKind = icon!("bladen-pdf.svg");
const I_LABELS: IconKind = icon!("labels.svg");
const I_LABELS_UPDATE: IconKind = icon!("labels-bijwerken.svg");
// Corridor stream: assemblies, junctions, roundabouts.
const I_ASSEMBLY: IconKind = icon!("profieltype.svg");
const I_JUNCTION: IconKind = icon!("kruispunt.svg");
const I_ROUNDABOUT: IconKind = icon!("rotonde.svg");
const I_CODES: IconKind = icon!("codes.svg");
const I_PARCELS: IconKind = icon!("percelen.svg");
const I_PARCEL_INFO: IconKind = icon!("perceel-info.svg");
const I_DROP: IconKind = icon!("waterdruppel.svg");
const I_CATCHMENT: IconKind = icon!("stroomgebied.svg");
const I_PONDS: IconKind = icon!("plassen.svg");
// Exchange (stream uitwisseling).
const I_LANDXML_EXPORT: IconKind = icon!("landxml-export.svg");
// Riolering (src/civiel/riool.rs).
const I_RIOOL: IconKind = icon!("riool.svg");
const I_RIOOL_PROFIEL: IconKind = icon!("riool-profiel.svg");
const I_RIOOL_TOETS: IconKind = icon!("riool-toets.svg");
const I_RIOOL_BIJWERKEN: IconKind = icon!("riool-bijwerken.svg");
const I_RIOOL_EXPORT: IconKind = icon!("riool-export.svg");
const I_RIOOL_GWSW: IconKind = icon!("riool-gwsw.svg");
// Design check, sight distance and rail (as-spoor).
const I_CRITERIA: IconKind = icon!("ontwerpcriteria.svg");
const I_CHECK: IconKind = icon!("as-toets.svg");
const I_SIGHT: IconKind = icon!("zicht.svg");
const I_RAIL: IconKind = icon!("spoor.svg");
const I_TURNOUT: IconKind = icon!("wissel.svg");

fn command(id: &'static str) -> ModuleEvent {
    ModuleEvent::Command(id.to_string())
}

fn large(id: &'static str, label: &'static str, icon: IconKind) -> RibbonItem {
    RibbonItem::LargeTool(ToolDef { id, label, icon, event: command(id) })
}

fn small(id: &'static str, label: &'static str, icon: IconKind) -> RibbonItem {
    RibbonItem::LabeledTool(ToolDef { id, label, icon, event: command(id) })
}

fn file_tool(id: &'static str, label: &'static str, icon: IconKind, title: &str, filter: &str, ext: &[&str], big: bool) -> RibbonItem {
    let t = ToolDef {
        id,
        label,
        icon,
        event: ModuleEvent::PluginFileDialog {
            command: id.into(),
            title: title.into(),
            filter_name: filter.into(),
            extensions: ext.iter().map(|e| e.to_string()).collect(),
        },
    };
    if big { RibbonItem::LargeTool(t) } else { RibbonItem::LabeledTool(t) }
}

/// Point clouds: a TIN from ground points (AHN in the Netherlands, or a
/// file) and a 3D mesh of an object.
/// The terrain group: TIN, point clouds (files; the AHN points sit in the
/// Dutch menu), analysis and water.
fn terrain_tools() -> Vec<RibbonItem> {
    use open_infra::lang::{dutch, tr};
    let cloud = tr("Puntenwolk (LAS, LAZ, E57, PLY, PTS, XYZ)", "Point cloud (LAS, LAZ, E57, PLY, PTS, XYZ)");
    let exts = &["las", "laz", "e57", "ply", "pts", "xyz", "txt"];
    let mut tools = vec![
        large("CIV_TIN", tr("TIN maken", "Create TIN"), I_TIN),
        file_tool("CIV_PUNTENWOLK_BESTAND", tr("TIN uit puntenwolk", "TIN from point cloud"), I_CLOUD_TIN, tr("Puntenwolk openen", "Open a point cloud"), cloud, exts, false),
    ];
    // The mesh of the AHN points, or (without the Dutch modules) of a file.
    if dutch() {
        tools.push(small("CIV_MESH", tr("3D-mesh", "3D mesh"), I_MESH));
    } else {
        tools.push(file_tool("CIV_MESH", tr("3D-mesh", "3D mesh"), I_MESH, tr("Puntenwolk openen", "Open a point cloud"), cloud, exts, false));
    }
    tools.push(RibbonItem::LabeledDropdown {
        id: "CIV_MENU_ANALYSE",
        label: tr("Analyse", "Analysis"),
        icon: I_ANALYSIS,
        items: vec![
            ("CIV_HOOGTELIJNEN", tr("Hoogtelijnen", "Contours"), I_CONTOURS),
            ("CIV_ANALYSE", tr("Hoogteanalyse", "Elevation analysis"), I_ANALYSIS),
            ("CIV_VOLUME", tr("Grondverzet", "Earthworks"), I_VOLUME),
        ],
        default: "CIV_HOOGTELIJNEN",
    });
    tools.push(RibbonItem::LabeledDropdown {
        id: "CIV_MENU_WATER",
        label: tr("Water", "Water"),
        icon: I_CATCHMENT,
        items: vec![
            ("CIV_STROOMGEBIED", tr("Stroomgebied", "Catchment"), I_CATCHMENT),
            ("CIV_WATERDRUPPEL", tr("Waterdruppel", "Water drop"), I_DROP),
            ("CIV_PLASSEN", tr("Plassen", "Ponds"), I_PONDS),
        ],
        default: "CIV_STROOMGEBIED",
    });
    tools
}

struct CivielModule;

impl CadModule for CivielModule {
    fn id(&self) -> &'static str {
        "open_infra_civiel"
    }
    fn title(&self) -> &'static str {
        "Civiel"
    }
    fn ribbon_groups(&self) -> &[RibbonGroup] {
        static GROUPS: OnceLock<Vec<RibbonGroup>> = OnceLock::new();
        GROUPS.get_or_init(|| {
            use open_infra::lang::dutch;
            // Eight groups at most (the ninth is the regional one); related
            // tools share a menu, file dialogs stay buttons (a menu item can
            // only run a command).
            let menu = |id: &'static str, label: &'static str, icon: IconKind, items: Vec<(&'static str, &'static str, IconKind)>, default: &'static str| RibbonItem::LabeledDropdown { id, label, icon, items, default };
            let mut groups = vec![
                // Points: import and export, survey codes, parcels.
                RibbonGroup {
                    title: tr("Punten", "Points"),
                    tools: vec![
                        file_tool("CIV_PUNTEN_IMPORT", tr("Punten importeren", "Import points"), I_POINTS, tr("Puntenbestand openen", "Open a point file"), tr("Puntenbestand (PNEZD, PENZD, CSV)", "Point file (PNEZD, PENZD, CSV)"), &["csv", "txt", "xyz", "pnt", "asc"], true),
                        file_tool("CIV_LANDXML", "LandXML", I_LANDXML, tr("LandXML openen", "Open LandXML"), "LandXML", &["xml", "landxml"], false),
                        file_tool("CIV_IFC_IMPORT", tr("IFC inlezen", "IFC import"), I_IFC, tr("IFC openen", "Open IFC"), "IFC", &["ifc"], false),
                        menu("CIV_MENU_PUNTEN", tr("Punten", "Points"), I_PLACE, vec![
                            ("CIV_PUNT", tr("Punt plaatsen", "Place point"), I_PLACE),
                            ("CIV_PUNTEN_EXPORT", tr("Punten exporteren", "Export points"), I_EXPORT),
                            ("CIV_CODES_TOEPASSEN", tr("Codes toepassen", "Apply codes"), I_CODES),
                        ], "CIV_PUNT"),
                        menu("CIV_MENU_PERCELEN", tr("Percelen", "Parcels"), I_PARCELS, vec![
                            ("CIV_PERCELEN", tr("Percelen verdelen", "Divide parcels"), I_PARCELS),
                            ("CIV_PERCEEL_INFO", tr("Perceelinfo", "Parcel info"), I_PARCEL_INFO),
                        ], "CIV_PERCELEN"),
                    ],
                },
                // Terrain: TIN, point clouds, analysis, water.
                RibbonGroup { title: tr("Terrein", "Terrain"), tools: terrain_tools() },
                // Alignment and profiles, design checks, rail.
                RibbonGroup {
                    title: tr("As en profiel", "Alignment and profile"),
                    tools: vec![
                        large("CIV_AS", tr("As maken", "Create alignment"), I_AXIS),
                        menu("CIV_MENU_AS", tr("As", "Alignment"), I_OFFSET, vec![
                            ("CIV_AS_OFFSET", tr("As verschuiven", "Offset alignment"), I_OFFSET),
                            ("CIV_STATION", tr("Station / offset", "Station / offset"), I_STATION),
                            ("CIV_AS_TOETS", tr("As toetsen", "Check alignment"), I_CHECK),
                            ("CIV_ONTWERPCRITERIA", tr("Ontwerpcriteria", "Design criteria"), I_CRITERIA),
                            ("CIV_ZICHT", tr("Zichtafstand", "Sight distance"), I_SIGHT),
                        ], "CIV_AS_OFFSET"),
                        small("CIV_LENGTEPROFIEL", tr("Lengteprofiel", "Length profile"), I_PROFILE),
                        small("CIV_ONTWERPPROFIEL", tr("Ontwerpprofiel", "Design profile"), I_DESIGN),
                        small("CIV_DWARSPROFIELEN", tr("Dwarsprofielen", "Cross sections"), I_SECTIONS),
                        menu("CIV_MENU_SPOOR", tr("Spoor", "Rail"), I_RAIL, vec![
                            ("CIV_SPOOR", tr("Spooras", "Track"), I_RAIL),
                            ("CIV_WISSEL", tr("Wissel", "Turnout"), I_TURNOUT),
                        ], "CIV_SPOOR"),
                    ],
                },
                // Corridors: assembly along axis and design profile; junctions.
                RibbonGroup {
                    title: "Corridor",
                    tools: vec![
                        large("CIV_CORRIDOR", "Corridor", I_CORRIDOR),
                        small("CIV_PROFIELTYPEN", tr("Profieltypen", "Assemblies"), I_ASSEMBLY),
                        menu("CIV_MENU_KRUISPUNT", tr("Kruispunten", "Junctions"), I_JUNCTION, vec![
                            ("CIV_KRUISPUNT", tr("Kruispunt", "Junction"), I_JUNCTION),
                            ("CIV_ROTONDE", tr("Rotonde", "Roundabout"), I_ROUNDABOUT),
                        ], "CIV_KRUISPUNT"),
                    ],
                },
                // Grading: feature lines, plateaus and slopes, exact earthwork, mass haul.
                RibbonGroup {
                    title: tr("Grondwerk", "Grading"),
                    tools: vec![
                        large("CIV_KANTLIJN", tr("Kantlijn", "Feature line"), I_FEATURE),
                        menu("CIV_MENU_VORM", tr("Vormgeven", "Shape"), I_PLATEAU, vec![
                            ("CIV_PLATEAU", tr("Plateau", "Plateau"), I_PLATEAU),
                            ("CIV_TALUD", tr("Talud", "Slope"), I_SLOPE),
                            ("CIV_KANTLIJN_PUNT", tr("Hoogtepunt", "Height point"), I_PLACE),
                        ], "CIV_PLATEAU"),
                        menu("CIV_MENU_VOLUME", tr("Volumes", "Volumes"), I_VOLUME_TIN, vec![
                            ("CIV_VOLUME_TIN", tr("Grondverzet TIN", "Volume TIN"), I_VOLUME_TIN),
                            ("CIV_GRONDBALANS", tr("Grondbalans", "Mass haul"), I_MASS_HAUL),
                        ], "CIV_VOLUME_TIN"),
                    ],
                },
                // Pipe networks (sewers).
                RibbonGroup { title: tr("Riolering", "Sewers"), tools: riool_tools() },
                // Output and 3D: sheets, labels, the 3D model, exchange, settings.
                RibbonGroup {
                    title: tr("Uitvoer en 3D", "Output and 3D"),
                    tools: vec![
                        large("CIV_BLADEN", tr("Bladen maken", "Create sheets"), I_SHEETS),
                        menu("CIV_MENU_BLADEN", tr("Bladen", "Sheets"), I_SHEETS_PDF, vec![
                            ("CIV_BLADEN_PDF", tr("Bladen naar PDF", "Sheets to PDF"), I_SHEETS_PDF),
                            ("CIV_LABELS", "Labels", I_LABELS),
                            ("CIV_LABELS_BIJWERKEN", tr("Labels bijwerken", "Update labels"), I_LABELS_UPDATE),
                            ("CIV_BIJWERKEN", tr("Profielen bijwerken", "Update profiles"), I_LABELS_UPDATE),
                        ], "CIV_BLADEN_PDF"),
                        file_tool("CIV_CITYGML", "CityGML/JSON", I_CITYGML, tr("CityGML of CityJSON openen", "Open CityGML or CityJSON"), "CityGML / CityJSON", &["gml", "xml", "citygml", "json", "jsonl"], false),
                        file_tool("CIV_3D_IMPORT", tr("3D-model", "3D model"), I_MESH, tr("3D-model openen (OBJ, STL, glTF)", "Open a 3D model (OBJ, STL, glTF)"), "OBJ / STL / glTF", &["obj", "stl", "glb", "gltf"], false),
                        file_tool("CIV_IFC_IDS", tr("IDS-controle", "IDS check"), I_CHECK, tr("IDS van de opdrachtgever openen", "Open the client's IDS"), "IDS", &["ids", "xml"], false),
                        menu("CIV_MENU_WEERGAVE", tr("Weergave", "View"), I_PHOTO, vec![
                            ("CIV_FOTOTERREIN", tr("Terrein + luchtfoto", "Terrain + aerial photo"), I_PHOTO),
                            ("CIV_DOORZICHT", tr("Maaiveld doorzichtig", "See-through ground"), I_SEE),
                            ("CIV_WATERPEIL", tr("Waterpeil", "Water level"), I_WATER),
                        ], "CIV_FOTOTERREIN"),
                        menu("CIV_MENU_EXPORT", tr("Exporteren", "Export"), I_IFC, vec![
                            ("CIV_IFC", "IFC 4x3", I_IFC),
                            ("CIV_LANDXML_EXPORT", "LandXML", I_LANDXML_EXPORT),
                            ("CIV_3D_EXPORT", tr("3D (glTF)", "3D (glTF)"), I_MESH),
                        ], "CIV_IFC"),
                        menu("CIV_MENU_INSTELLINGEN", tr("Instellingen", "Settings"), I_SETTINGS, vec![
                            ("CIV_INSTELLINGEN", tr("Instellingen", "Settings"), I_SETTINGS),
                            ("CIV_INFO", tr("Modelinfo", "Model info"), I_INFO),
                        ], "CIV_INSTELLINGEN"),
                    ],
                },
            ];
            // The Netherlands, always on the far right.
            if dutch() {
                groups.push(RibbonGroup {
                    title: tr("Nederland", "Netherlands"),
                    tools: vec![RibbonItem::LabeledDropdown {
                        id: "CIV_NL_3D",
                        label: tr("3D-basisdata", "3D base data"),
                        icon: I_BAG3D,
                        items: vec![
                            ("CIV_3DMODEL", tr("3D-model (alles)", "3D model (all)"), I_BAG3D),
                            ("CIV_BAG3D", "3D BAG", I_BAG3D),
                            ("CIV_BGT3D", tr("BGT naar 3D", "BGT to 3D"), I_BGT3D),
                            ("CIV_KLIC3D", tr("KLIC in 3D", "KLIC in 3D"), I_KLIC3D),
                            ("CIV_SONDERINGEN3D", tr("Sonderingen 3D (BRO)", "CPTs in 3D (BRO)"), I_CPT),
                            ("CIV_PUNTENWOLK_TIN", tr("TIN uit AHN-punten", "TIN from AHN points"), I_CLOUD_TIN),
                        ],
                        default: "CIV_BAG3D",
                    }],
                });
            }
            groups
        })
    }
}

/// Run an engine function on a worker (the callback runs inside the host's
/// input handling; network and big drawings take longer than its timeout).
fn job(host: &mut dyn HostApi, label: &str, id: Option<String>, f: impl FnOnce(&dyn Fn(String)) -> Result<String, String> + Send + 'static) {
    host.push_info(&format!("[Open Infra] {label} …"));
    let label = label.to_string();
    std::thread::spawn(move || {
        let _ = api::take();
        let r = f(&|_| {});
        let data = api::take();
        match &r {
            Ok(m) => bridge::info(m),
            Err(e) => bridge::error(&format!("{label}: {e}")),
        }
        // Scripts and agents read the outcome back (ID=…).
        if let Some(id) = id {
            match r {
                Ok(m) => api::write(&id, true, &m, data),
                Err(e) => api::write(&id, false, &format!("{label}: {e}"), data),
            }
        }
    });
}

/// An argument error, reported like a failed job (also to the ID file).
fn refuse(host: &mut dyn HostApi, id: Option<String>, label: &str, e: String) {
    host.push_error(&format!("[Open Infra] {label}: {e}"));
    if let Some(id) = id {
        api::write(&id, false, &format!("{label}: {e}"), Default::default());
    }
}

/// The selected axis, read by handle (not the whole drawing, which may be huge).
fn selected_axis(host: &mut dyn HostApi) -> Option<as_profiel::Alignment> {
    let sel = host.selection();
    if sel.len() != 1 {
        return None;
    }
    let e = bridge::entities(&sel).ok()?.into_iter().next()??;
    as_profiel::Alignment::from_entity(&e)
}

/// `CIV_PUNT`: click points; each gets the next number and its ground height.
struct PlaceCmd;

fn placer() -> &'static Mutex<Option<landmeten::Placer>> {
    static P: OnceLock<Mutex<Option<landmeten::Placer>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

impl InteractiveCommand for PlaceCmd {
    fn prompt(&self) -> String {
        "Civiel: plaats een punt [Enter = klaar]".into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        std::thread::spawn(move || {
            let mut guard = placer().lock().unwrap();
            if guard.is_none() {
                match landmeten::Placer::new(&civiel::settings()) {
                    Ok(p) => *guard = Some(p),
                    Err(e) => return bridge::error(&format!("Punt plaatsen: {e}")),
                }
            }
            match guard.as_mut().unwrap().place([pt[0], pt[1]]) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("Punt plaatsen: {e}")),
            }
        });
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        CommandStep::Done
    }
}

/// `CIV_STATION`: station and offset of clicked points along the selected axis.
struct StationCmd {
    axis: as_profiel::Alignment,
    f: f64,
}

impl InteractiveCommand for StationCmd {
    fn prompt(&self) -> String {
        "Civiel: punt voor station en offset [Enter = klaar]".into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let text = as_profiel::station_offset_text(&self.axis, [pt[0], pt[1]], self.f);
        std::thread::spawn(move || bridge::info(&text));
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        // A line from the cursor to its foot on the axis.
        let (s, _) = self.axis.station_offset([pt[0], pt[1]]);
        let (foot, _) = self.axis.at(s);
        vec![PreviewWire::with_color(vec![[foot[0], foot[1], 0.0], pt], [0.95, 0.6, 0.15, 1.0])]
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Place {
    LengthProfile,
    CrossSections,
}

/// Pick where a length profile (lower-left corner) or the cross sections
/// (upper-left corner) go; a rectangle of their size follows the cursor.
struct PlaceViewCmd {
    what: Place,
    size: [f64; 2],
}

impl InteractiveCommand for PlaceViewCmd {
    fn prompt(&self) -> String {
        match self.what {
            Place::LengthProfile => "Civiel: linkeronderhoek van het lengteprofiel".into(),
            Place::CrossSections => "Civiel: linkerbovenhoek van de dwarsprofielen".into(),
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let (what, at) = (self.what, [pt[0], pt[1]]);
        std::thread::spawn(move || {
            let r = match what {
                Place::LengthProfile => as_profiel::length_profile(&civiel::settings(), None, Some(at), &|_| {}),
                Place::CrossSections => as_profiel::cross_sections(&civiel::settings(), None, Some(at), &|_| {}),
            };
            match r {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&e),
            }
        });
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let [w, h] = self.size;
        let (x0, y0) = (pt[0], pt[1]);
        let y1 = if self.what == Place::LengthProfile { y0 + h } else { y0 - h };
        vec![PreviewWire::with_color(vec![[x0, y0, 0.0], [x0 + w, y0, 0.0], [x0 + w, y1, 0.0], [x0, y1, 0.0], [x0, y0, 0.0]], [0.4, 0.8, 1.0, 1.0])]
    }
}

/// `CIV_TALUD` from the ribbon: click the side of the selected line the
/// slope goes to.
struct SlopeSideCmd {
    line: Vec<[f64; 3]>,
    handle: ocs_plugin_api::host::codec::Handle,
}

impl InteractiveCommand for SlopeSideCmd {
    fn prompt(&self) -> String {
        tr("Grondwerk: klik aan de kant waar het talud komt", "Grading: click the side the slope goes to").into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        // The side of the nearest segment the point lies on.
        let mut best = (f64::MAX, 1.0);
        for w in self.line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = (dx * dx + dy * dy).max(1e-24);
            let t = (((pt[0] - a[0]) * dx + (pt[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
            let d = (a[0] + dx * t - pt[0]).hypot(a[1] + dy * t - pt[1]);
            if d < best.0 {
                best = (d, if dx * (pt[1] - a[1]) - dy * (pt[0] - a[0]) > 0.0 { -1.0 } else { 1.0 });
            }
        }
        let (side, h) = (best.1, self.handle);
        std::thread::spawn(move || {
            let o = grading::design_options(&civiel::settings(), false);
            let req = grading::SlopeRequest {
                source: grading::Source::Handle(h),
                side,
                to: grading::SlopeTo::Ground { cut: DEFAULT_CUT, fill: DEFAULT_FILL, reach: 50.0 },
                level: None,
                step: 1.0,
                name: None,
                join: true,
            };
            match grading::slope(&o, req, &|_| {}) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("{}: {e}", tr("Talud", "Slope"))),
            }
        });
        CommandStep::Done
    }
}

/// `CIV_KANTLIJN_PUNT` from the ribbon: click points on the selected
/// feature line; each becomes a height point (height interpolated).
struct HeightPointCmd {
    handle: ocs_plugin_api::host::codec::Handle,
}

fn edit_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

impl InteractiveCommand for HeightPointCmd {
    fn prompt(&self) -> String {
        tr("Grondwerk: klik een hoogtepunt op de kantlijn [Enter = klaar]", "Grading: click a height point on the feature line [Enter = done]").into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let h = self.handle;
        std::thread::spawn(move || {
            let _one = edit_lock().lock().unwrap_or_else(|e| e.into_inner());
            let o = civiel::settings();
            let at = match civiel::Ctx::new(&o) {
                Ok(c) => c.from_drawing([pt[0], pt[1]]),
                Err(e) => return bridge::error(&e),
            };
            match grading::edit_feature(&o, Some(h), grading::EditArgs::Insert { at, z: None }) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("{}: {e}", tr("Hoogtepunt", "Height point"))),
            }
        });
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        CommandStep::Done
    }
}

/// `CIV_WATERDRUPPEL`: click points; each gets the path of a drop.
struct DropCmd {
    cell: f64,
}

fn drops() -> &'static Mutex<Option<civiel::hydrologie::DropSession>> {
    static D: OnceLock<Mutex<Option<civiel::hydrologie::DropSession>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(None))
}

impl InteractiveCommand for DropCmd {
    fn prompt(&self) -> String {
        open_infra::lang::tr("Civiel: startpunt van de waterdruppel [Enter = klaar]", "Civil: start of the water drop [Enter = done]").into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let cell = self.cell;
        std::thread::spawn(move || {
            let mut guard = drops().lock().unwrap();
            if guard.is_none() {
                match civiel::hydrologie::DropSession::new(&civiel::settings(), cell, &|_| {}) {
                    Ok(s) => *guard = Some(s),
                    Err(e) => return bridge::error(&format!("{}: {e}", open_infra::lang::tr("Waterdruppel", "Water drop"))),
                }
            }
            match guard.as_mut().unwrap().run(&[[pt[0], pt[1]]]) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("{}: {e}", open_infra::lang::tr("Waterdruppel", "Water drop"))),
            }
        });
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        CommandStep::Done
    }
}

/// Slopes when none are given: the corridor's road template (fill 1:3, cut
/// 1:2); a starting value, not a standard.
const DEFAULT_CUT: f64 = 2.0;
const DEFAULT_FILL: f64 = 3.0;

/// The line of a grading command: LIJN=<handle>, PUNTEN=x,y[,z];…, or the selection.
fn grading_source(args: &Args, key: &str) -> Result<grading::Source, String> {
    if let Some(h) = args.handle(key)?.or(args.handle("LIJN")?) {
        return Ok(grading::Source::Handle(h));
    }
    if let Some(p) = args.str("PUNTEN") {
        let pts = p
            .split(';')
            .filter(|t| !t.trim().is_empty())
            .map(|t| {
                let c: Result<Vec<f64>, String> = t.split(',').map(grading::number).collect();
                match c {
                    Ok(c) if c.len() == 2 || c.len() == 3 => Ok(c),
                    _ => Err(format!("PUNTEN: {} '{t}'", tr("verwacht x,y of x,y,z, kreeg", "expected x,y or x,y,z, got"))),
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        return Ok(grading::Source::Points(pts));
    }
    Ok(grading::Source::Selection)
}

fn yes(v: Option<&str>) -> Option<bool> {
    v.map(|v| matches!(v.to_ascii_lowercase().as_str(), "ja" | "yes" | "1" | "true" | "aan" | "on"))
}

/// The grading commands (`CIV_KANTLIJN…`, `CIV_PLATEAU`, `CIV_TALUD`,
/// `CIV_VOLUME BASIS=TIN`, `CIV_VOLUME_TIN`, `CIV_GRONDBALANS`); false when
/// the verb is not one of them.
fn run_grading(host: &mut dyn HostApi, verb: &str, args: &Args, id: Option<String>) -> Result<bool, String> {
    let scripted = !args.is_empty();
    let base = civiel::settings();
    let keep_status = args.str("STATUS").is_some();
    let ratio = |k: &str| args.str(k).map(grading::parse_ratio).transpose();
    match verb {
        "CIV_KANTLIJN" => {
            let o = grading::design_options(&args.apply(&base, &["LIJN", "PUNTEN", "HOOGTE", "OPPERVLAK", "START", "HELLING", "PEILEN", "TUSSENPUNTEN", "NAAM", "GESLOTEN"])?, keep_status);
            let source = grading_source(args, "LIJN")?;
            let surface = match args.str("OPPERVLAK").or(args.str("HOOGTE")).map(|s| s.to_ascii_uppercase()) {
                Some(s) if s == "N" || s == "ONTWERP" => 'N',
                _ => 'B',
            };
            let heights = if let Some(start) = args.f64("START")? {
                grading::Heights::Grades { start, grades: args.list("HELLING")?.unwrap_or_default() }
            } else if let Some(p) = args.str("PEILEN") {
                grading::Heights::Levels(p.to_string())
            } else if matches!(args.str("HOOGTE").map(|s| s.to_ascii_uppercase()).as_deref(), Some("EIGEN" | "LIJN" | "OWN")) {
                grading::Heights::Own
            } else {
                grading::Heights::Surface(surface)
            };
            let step = match args.str("TUSSENPUNTEN") {
                Some(v) if grading::number(v).is_ok() => grading::number(v)?,
                v => if yes(v).unwrap_or(!scripted) { o.step } else { 0.0 },
            };
            let req = grading::FeatureRequest { source, heights, step, name: args.str("NAAM").map(str::to_string), closed: yes(args.str("GESLOTEN")) };
            job(host, tr("Kantlijn", "Feature line"), id, move |p| grading::feature_line(&o, req, p));
        }
        "CIV_KANTLIJN_PEIL" | "CIV_KANTLIJN_VERHOOG" | "CIV_KANTLIJN_PUNT" | "CIV_KANTLIJN_HELLING" => {
            let o = args.apply(&base, &["KANTLIJN", "LIJN", "PUNT", "BIJ", "Z", "DZ", "VAN", "TOT", "HELLING"])?;
            let h = args.handle("KANTLIJN")?.or(args.handle("LIJN")?);
            let at = args.points("BIJ")?.and_then(|p| p.first().copied());
            let z = args.f64("Z")?.or(args.f64("PEIL")?);
            let edit = match verb {
                "CIV_KANTLIJN_PEIL" => grading::EditArgs::Level { corner: args.f64("PUNT")?.map(|v| v as usize), near: at, z: z.ok_or("CIV_KANTLIJN_PEIL KANTLIJN=<handle> PUNT=<n> | BIJ=x,y Z=<m>")? },
                "CIV_KANTLIJN_VERHOOG" => grading::EditArgs::Raise(args.f64("DZ")?.ok_or(tr("CIV_KANTLIJN_VERHOOG KANTLIJN=<handle> DZ=<m> (< 0 omlaag)", "CIV_KANTLIJN_VERHOOG KANTLIJN=<handle> DZ=<m> (< 0 down)"))?),
                "CIV_KANTLIJN_PUNT" => match at {
                    Some(at) => grading::EditArgs::Insert { at, z },
                    None => {
                        // From the ribbon: click points on the selected feature line.
                        let sel = host.selection();
                        let Some(handle) = h.or(sel.first().copied().filter(|_| sel.len() == 1)) else {
                            return Err(tr("selecteer eerst één kantlijn, of geef KANTLIJN=<handle> BIJ=x,y [Z=]", "select one feature line first, or give KANTLIJN=<handle> BIJ=x,y [Z=]").into());
                        };
                        host.start_interactive(Box::new(HeightPointCmd { handle }));
                        return Ok(true);
                    }
                },
                _ => grading::EditArgs::Grade {
                    from: args.f64("VAN")?.ok_or("CIV_KANTLIJN_HELLING KANTLIJN=<handle> VAN=<n> TOT=<n> [HELLING=%]")? as usize,
                    to: args.f64("TOT")?.ok_or("CIV_KANTLIJN_HELLING KANTLIJN=<handle> VAN=<n> TOT=<n> [HELLING=%]")? as usize,
                    grade: args.f64("HELLING")?,
                },
            };
            job(host, tr("Kantlijn bewerken", "Edit feature line"), id, move |_| grading::edit_feature(&o, h, edit));
        }
        "CIV_PLATEAU" => {
            let o = grading::design_options(&args.apply(&base, &["LIJN", "PUNTEN", "AFSCHOT", "RICHTING", "PEILPUNT", "TALUD", "TALUD_AF", "TALUD_OP", "REIKWIJDTE", "NAAM", "SAMENVOEGEN"])?, keep_status);
            let source = grading_source(args, "LIJN")?;
            let fall = match args.str("AFSCHOT") {
                None => None,
                Some(v) => {
                    let (pct, dir) = match v.split_once(';') {
                        Some((a, b)) => (a, Some(b)),
                        None => (v, args.str("RICHTING")),
                    };
                    let dir = dir.ok_or(tr("AFSCHOT: geef RICHTING=N|NO|O|ZO|Z|ZW|W|NW|<graden>", "AFSCHOT: give RICHTING=N|NE|E|SE|S|SW|W|NW|<degrees>"))?;
                    Some((grading::number(pct)?, grading::parse_bearing(dir)?))
                }
            };
            let both = ratio("TALUD")?;
            let req = grading::PlateauRequest {
                source,
                level: args.f64("PEIL")?,
                default_level: (!scripted).then_some(base.level),
                fall,
                reference: args.points("PEILPUNT")?.and_then(|p| p.first().copied()),
                cut: ratio("TALUD_AF")?.or(both).unwrap_or(DEFAULT_CUT),
                fill: ratio("TALUD_OP")?.or(both).unwrap_or(DEFAULT_FILL),
                reach: args.f64("REIKWIJDTE")?.unwrap_or(50.0).max(1.0),
                step: if args.str("STAP").is_some() { o.step } else { 1.0 },
                name: args.str("NAAM").map(str::to_string),
                join: yes(args.str("SAMENVOEGEN")).unwrap_or(true),
            };
            job(host, tr("Plateau", "Plateau"), id, move |p| grading::plateau(&o, req, p));
        }
        "CIV_TALUD" => {
            let o = grading::design_options(&args.apply(&base, &["LIJN", "PUNTEN", "ZIJDE", "NAAR", "TALUD", "TALUD_AF", "TALUD_OP", "BREEDTE", "RICHTING", "RANDPEIL", "REIKWIJDTE", "NAAM", "SAMENVOEGEN"])?, keep_status);
            let source = grading_source(args, "LIJN")?;
            if !scripted {
                // From the ribbon: the selected line, then click the side.
                let sel = host.selection();
                let e = (sel.len() == 1).then(|| bridge::entities(&sel).ok()).flatten().and_then(|v| v.into_iter().next().flatten());
                let Some(line) = e.as_ref().and_then(grading::line_of) else {
                    return Err(tr("selecteer eerst één lijn of kantlijn voor het talud", "select one line or feature line for the slope first").into());
                };
                host.start_interactive(Box::new(SlopeSideCmd { line: line.pts, handle: sel[0] }));
                return Ok(true);
            }
            let side = match args.str("ZIJDE").map(|s| s.to_ascii_uppercase()) {
                None => 1.0,
                Some(s) if s.starts_with('R') => 1.0,
                Some(s) if s.starts_with('L') => -1.0,
                Some(_) => return Err("ZIJDE: LINKS | RECHTS".into()),
            };
            let n = ratio("TALUD")?;
            let width = args.f64("BREEDTE")?.or(args.f64("AFSTAND")?);
            let to = match args.str("NAAR").map(|s| s.to_ascii_uppercase()) {
                Some(s) if s == "AFSTAND" || s == "BREEDTE" || (s.is_empty() && width.is_some()) => grading::SlopeTo::Width {
                    width: width.ok_or("NAAR=AFSTAND: BREEDTE=<m>")?,
                    n: n.unwrap_or(DEFAULT_FILL),
                    up: matches!(args.str("RICHTING").map(|s| s.to_ascii_uppercase()).as_deref(), Some("OP" | "UP")),
                },
                Some(s) if s == "PEIL" => grading::SlopeTo::Level { z: args.f64("PEIL")?.ok_or("NAAR=PEIL: PEIL=<m>")?, n: n.unwrap_or(DEFAULT_FILL) },
                None if width.is_some() => grading::SlopeTo::Width { width: width.unwrap(), n: n.unwrap_or(DEFAULT_FILL), up: matches!(args.str("RICHTING").map(|s| s.to_ascii_uppercase()).as_deref(), Some("OP" | "UP")) },
                None | Some(_) => grading::SlopeTo::Ground {
                    cut: ratio("TALUD_AF")?.or(n).unwrap_or(DEFAULT_CUT),
                    fill: ratio("TALUD_OP")?.or(n).unwrap_or(DEFAULT_FILL),
                    reach: args.f64("REIKWIJDTE")?.unwrap_or(50.0).max(1.0),
                },
            };
            let req = grading::SlopeRequest {
                source,
                side,
                to,
                level: args.f64("RANDPEIL")?,
                step: if args.str("STAP").is_some() { o.step } else { 1.0 },
                name: args.str("NAAM").map(str::to_string),
                join: yes(args.str("SAMENVOEGEN")).unwrap_or(true),
            };
            job(host, tr("Talud", "Slope"), id, move |p| grading::slope(&o, req, p));
        }
        "CIV_VOLUME_TIN" | "CIV_VOLUME" if verb == "CIV_VOLUME_TIN" || args.str("BASIS").is_some_and(|b| b.eq_ignore_ascii_case("TIN")) => {
            let o = args.apply(&base, &["BASIS", "ONTWERP", "LOSFACTOR", "VERDICHTING", "VERGELIJK", "KAART"])?;
            let design = match args.str("ONTWERP").map(|s| s.to_ascii_uppercase()) {
                Some(s) if s == "PEIL" => grading::DesignPick::Level,
                Some(s) if s == "BOUWRIJP" || s == "GRADING" => grading::DesignPick::Bouwrijp,
                _ if args.str("PEIL").is_some() => grading::DesignPick::Level,
                _ => grading::DesignPick::Auto,
            };
            let req = grading::VolumeRequest {
                design,
                loose: args.f64("LOSFACTOR")?.unwrap_or(1.0).max(0.01),
                compaction: args.f64("VERDICHTING")?.unwrap_or(1.0).max(0.01),
                bands: if yes(args.str("KAART")) == Some(false) { 0 } else { o.bands.max(2) },
                compare: yes(args.str("VERGELIJK")).unwrap_or(false),
            };
            job(host, tr("Grondverzet TIN", "Volume TIN"), id, move |p| grading::volume_tin(&o, req, p));
        }
        "CIV_GRONDBALANS" => {
            let o = args.apply(&base, &["AS", "BRON", "CSV", "FACTOR", "HERGEBRUIK", "VRIJ", "BREEDTE", "PLAATS"])?;
            let from = match (args.str("CSV"), args.str("BRON").map(|s| s.to_ascii_uppercase())) {
                (Some(p), _) => massa::VolumeSource::Csv(p.into()),
                (None, Some(b)) if b == "CORRIDOR" => massa::VolumeSource::Corridor,
                (None, Some(b)) if b == "TIN" => massa::VolumeSource::Tin,
                (None, None) => massa::VolumeSource::Auto,
                (None, Some(b)) if b == "AUTO" => massa::VolumeSource::Auto,
                (None, Some(_)) => return Err(tr("BRON: AUTO, CORRIDOR of TIN (of CSV=<pad>)", "BRON: AUTO, CORRIDOR or TIN (or CSV=<path>)").into()),
            };
            let at = match args.points("PLAATS")? {
                Some(p) if !p.is_empty() => Some(civiel::Ctx::new(&base)?.to_drawing(p[0])),
                _ => None,
            };
            let req = massa::Request {
                axis: args.handle("AS")?,
                from,
                factor: args.f64("FACTOR")?.unwrap_or(1.0).max(0.0),
                reuse: (args.f64("HERGEBRUIK")?.unwrap_or(100.0) / 100.0).clamp(0.0, 1.0),
                free: args.f64("VRIJ")?.unwrap_or(0.0).max(0.0),
                every: o.corridor_every,
                reach: args.f64("BREEDTE")?.unwrap_or(100.0).max(1.0),
                at,
            };
            job(host, tr("Grondbalans", "Mass haul"), id, move |p| massa::mass_haul_diagram(&o, req, p));
        }
        "CIV_HELP" if args.words.first().is_some_and(|w| w.eq_ignore_ascii_case("GRADING") || w.eq_ignore_ascii_case("GRONDWERK")) => host.push_info(tr(
            "[Open Infra — Grondwerk] CIV_KANTLIJN [LIJN=|PUNTEN=x,y[,z];…] [HOOGTE=MAAIVELD|ONTWERP|EIGEN] [START= HELLING=%;%] [PEILEN=z;;z | n:z;…] [TUSSENPUNTEN=ja|<m>] [NAAM=];              CIV_KANTLIJN_PEIL KANTLIJN= PUNT=|BIJ=x,y Z=; CIV_KANTLIJN_VERHOOG KANTLIJN= DZ=; CIV_KANTLIJN_PUNT KANTLIJN= BIJ=x,y [Z=]; CIV_KANTLIJN_HELLING KANTLIJN= VAN= TOT= [HELLING=%];              CIV_PLATEAU [LIJN=|PUNTEN=] PEIL= [AFSCHOT=% RICHTING=N|O|Z|W|…] [PEILPUNT=x,y] [TALUD_AF=1:n] [TALUD_OP=1:n] [REIKWIJDTE=] [SAMENVOEGEN=nee];              CIV_TALUD [LIJN=|PUNTEN=] [ZIJDE=LINKS|RECHTS] [NAAR=MAAIVELD|AFSTAND|PEIL] [TALUD=1:n] [BREEDTE=] [PEIL=] [RICHTING=OP|AF] [RANDPEIL=];              CIV_VOLUME BASIS=TIN (of CIV_VOLUME_TIN) [ONTWERP=BOUWRIJP|PEIL] [LOSFACTOR=] [VERDICHTING=] [BANDEN=] [KAART=nee] [VERGELIJK=ja];              CIV_GRONDBALANS [AS=] [BRON=AUTO|CORRIDOR|TIN] [CSV=] [FACTOR=] [HERGEBRUIK=%] [VRIJ=m] [FREQUENTIE=] [PLAATS=x,y].",
            "[Open Infra — Grading] CIV_KANTLIJN [LIJN=|PUNTEN=x,y[,z];…] [HOOGTE=MAAIVELD|ONTWERP|EIGEN] [START= HELLING=%;%] [PEILEN=z;;z | n:z;…] [TUSSENPUNTEN=ja|<m>] [NAAM=];              CIV_KANTLIJN_PEIL KANTLIJN= PUNT=|BIJ=x,y Z=; CIV_KANTLIJN_VERHOOG KANTLIJN= DZ=; CIV_KANTLIJN_PUNT KANTLIJN= BIJ=x,y [Z=]; CIV_KANTLIJN_HELLING KANTLIJN= VAN= TOT= [HELLING=%];              CIV_PLATEAU [LIJN=|PUNTEN=] PEIL= [AFSCHOT=% RICHTING=N|E|S|W|…] [PEILPUNT=x,y] [TALUD_AF=1:n] [TALUD_OP=1:n] [REIKWIJDTE=] [SAMENVOEGEN=nee];              CIV_TALUD [LIJN=|PUNTEN=] [ZIJDE=LINKS|RECHTS] [NAAR=MAAIVELD|AFSTAND|PEIL] [TALUD=1:n] [BREEDTE=] [PEIL=] [RICHTING=OP|AF] [RANDPEIL=];              CIV_VOLUME BASIS=TIN (or CIV_VOLUME_TIN) [ONTWERP=BOUWRIJP|PEIL] [LOSFACTOR=] [VERDICHTING=] [BANDEN=] [KAART=nee] [VERGELIJK=ja];              CIV_GRONDBALANS [AS=] [BRON=AUTO|CORRIDOR|TIN] [CSV=] [FACTOR=] [HERGEBRUIK=%] [VRIJ=m] [FREQUENTIE=] [PLAATS=x,y].",
        )),
        _ => return Ok(false),
    }
    Ok(true)
}

/// `CIV_STROOMGEBIED`: click the outlets (gullies, manholes, a culvert),
/// Enter computes their catchments.
struct OutletCmd {
    points: Vec<[f64; 2]>,
    req: Option<civiel::hydrologie::CatchmentRequest>,
}

impl InteractiveCommand for OutletCmd {
    fn prompt(&self) -> String {
        let (first, next) = (
            open_infra::lang::tr("Civiel: afvoerpunt (kolk, put, duiker)", "Civil: outlet (gully, manhole, culvert)"),
            open_infra::lang::tr("Civiel: volgend afvoerpunt [Enter = berekenen]", "Civil: next outlet [Enter = compute]"),
        );
        if self.points.is_empty() { first.into() } else { next.into() }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        self.points.push([pt[0], pt[1]]);
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        let Some(mut req) = self.req.take() else { return CommandStep::Cancel };
        if self.points.is_empty() {
            return CommandStep::Cancel;
        }
        req.outlets.points = std::mem::take(&mut self.points);
        std::thread::spawn(move || match civiel::hydrologie::catchment(&civiel::settings(), req, &|_| {}) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{}: {e}", open_infra::lang::tr("Stroomgebied", "Catchment"))),
        });
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        self.points
            .iter()
            .chain(std::iter::once(&[pt[0], pt[1]]))
            .map(|p| {
                let r = 1.0;
                let ring: Vec<[f64; 3]> = (0..=16).map(|k| {
                    let a = k as f64 / 16.0 * std::f64::consts::TAU;
                    [p[0] + r * a.cos(), p[1] + r * a.sin(), 0.0]
                }).collect();
                PreviewWire::with_color(ring, [0.4, 0.8, 1.0, 1.0])
            })
            .collect()
    }
}

/// The catchment arguments of `CIV_STROOMGEBIED` / `CIV_PLASSEN`.
fn catchment_request(args: &Args, ponds_only: bool) -> Result<civiel::hydrologie::CatchmentRequest, String> {
    let yes = |k: &str, default: bool| match args.str(k).map(|v| v.to_ascii_lowercase()) {
        None => Ok(default),
        Some(v) if matches!(v.as_str(), "ja" | "1" | "true" | "aan") => Ok(true),
        Some(v) if matches!(v.as_str(), "nee" | "0" | "false" | "uit") => Ok(false),
        Some(_) => Err(format!("{k}: {}", open_infra::lang::tr("ja of nee", "ja (yes) or nee (no)"))),
    };
    let points = match args.points("PUNTEN")? {
        Some(p) => p.into_iter().map(to_drawing).collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let layers = args.str("LAAG").map(|l| l.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    Ok(civiel::hydrologie::CatchmentRequest {
        source: args.str("BRON").map(str::to_string),
        cell: args.f64("CEL")?.unwrap_or(1.0).max(0.25),
        outlets: if ponds_only { Default::default() } else { civiel::hydrologie::Outlets { points, layers } },
        snap: args.f64("SNAP")?.unwrap_or(2.0).max(0.0),
        ponds: ponds_only || yes("PLASSEN", true)?,
        min_depth: args.f64("MINDIEPTE")?.unwrap_or(0.05).max(0.0),
        min_area: args.f64("MINOPP")?.unwrap_or(2.0).max(0.0),
    })
}

/// The sewer tools; the GWSW data tools (Dutch municipal sewer data) only
/// with the Dutch modules switched on.
fn riool_tools() -> Vec<RibbonItem> {
    use open_infra::lang::tr;
    let mut items = vec![
        ("CIV_RIOOL_PROFIEL", tr("Lengteprofiel", "Length profile"), I_RIOOL_PROFIEL),
        ("CIV_RIOOL_TOETS", tr("Toetsen", "Check"), I_RIOOL_TOETS),
        ("CIV_RIOOL_HYDRAULICA", tr("Hydraulica", "Hydraulics"), I_RIOOL_TOETS),
        ("CIV_RIOOL_BIJWERKEN", tr("Bijwerken", "Update"), I_RIOOL_BIJWERKEN),
        ("CIV_RIOOL_EXPORT", tr("Exporteren", "Export"), I_RIOOL_EXPORT),
    ];
    if open_infra::lang::dutch() {
        items.push(("CIV_RIOOL_GWSW", tr("GWSW overnemen", "Take over GWSW"), I_RIOOL_GWSW));
    }
    let mut tools = vec![
        large("CIV_RIOOL", tr("Riool ontwerpen", "Design sewer"), I_RIOOL),
        RibbonItem::LabeledDropdown { id: "CIV_MENU_RIOOL", label: tr("Riool", "Sewer"), icon: I_RIOOL_TOETS, items, default: "CIV_RIOOL_TOETS" },
    ];
    if open_infra::lang::dutch() {
        tools.push(file_tool("CIV_RIOOL_IMPORT", tr("GWSW-bestand", "GWSW file"), I_RIOOL_GWSW, tr("GWSW-OroX openen", "Open GWSW-OroX"), "GWSW-OroX (Turtle)", &["ttl"], false));
    }
    tools
}

/// `CIV_RIOOL` from the ribbon: pick the manholes, upstream first; Enter
/// designs the run.
struct RioolCmd {
    pts: Vec<[f64; 2]>,
    design: riool::Ontwerp,
    opt: civiel::CivielOptions,
}

impl InteractiveCommand for RioolCmd {
    fn prompt(&self) -> String {
        use open_infra::lang::tr;
        if self.pts.is_empty() {
            tr("Riolering: eerste put (bovenstrooms)", "Sewers: first manhole (upstream)").into()
        } else {
            format!("{} {} [{}]", tr("Riolering: put", "Sewers: manhole"), self.pts.len() + 1, tr("Enter = klaar", "Enter = done"))
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        self.pts.push([pt[0], pt[1]]);
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        if self.pts.len() >= 2 {
            let (pts, design, opt) = (std::mem::take(&mut self.pts), self.design.clone(), self.opt.clone());
            std::thread::spawn(move || match riool::ontwerp(&opt, riool::Invoer::Punten(pts), design, &|_| {}) {
                Ok(m) => bridge::info(&m),
                Err(e) => bridge::error(&format!("{}: {e}", open_infra::lang::tr("Riool ontwerpen", "Design sewer"))),
            });
        }
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        if self.pts.is_empty() {
            return Vec::new();
        }
        let mut line: Vec<[f64; 3]> = self.pts.iter().map(|p| [p[0], p[1], 0.0]).collect();
        line.push(pt);
        vec![PreviewWire::with_color(line, [0.75, 0.35, 0.85, 1.0])]
    }
}

/// Design criteria from RICHTLIJN=, CATEGORIE= and V= (all three, or none).
fn criteria_choice(args: &Args) -> Result<Option<civiel::ontwerpcriteria::Choice>, String> {
    match (args.str("RICHTLIJN"), args.str("CATEGORIE"), args.f64("V")?) {
        (None, None, None) => Ok(None),
        (Some(g), Some(c), Some(v)) => Ok(Some(civiel::ontwerpcriteria::Choice { guideline: g.to_ascii_lowercase(), category: c.to_ascii_lowercase(), v })),
        _ => Err(tr("geef RICHTLIJN=, CATEGORIE= en V= samen (CIV_ONTWERPCRITERIA toont ze)", "give RICHTLIJN=, CATEGORIE= and V= together (CIV_ONTWERPCRITERIA lists them)").into()),
    }
}

/// An axis from AS= (or the selection) or from PI= R= with LS= or A= per curve.
fn layout_input(args: &Args, unit: f64) -> Result<as_profiel::AxisInput, String> {
    let Some(pis) = args.points("PI")? else {
        return Ok(as_profiel::AxisInput::Line(args.handle("AS")?));
    };
    let pis: Vec<[f64; 2]> = pis.into_iter().map(to_drawing).collect::<Result<_, _>>()?;
    let radii: Vec<f64> = args.list("R")?.unwrap_or_default().into_iter().map(|r| r * unit).collect();
    Ok(match (args.list("LS")?, args.list("A")?) {
        (None, None) => as_profiel::AxisInput::Pis(pis, radii),
        (Some(_), Some(_)) => return Err(tr("geef per boog LS= (lengte overgangsboog) óf A= (clothoïdeparameter), niet beide", "give LS= (transition length) or A= (clothoid parameter) per curve, not both").into()),
        (Some(ls), None) => as_profiel::AxisInput::Layout(pis, radii, ls.into_iter().map(|l| as_profiel::Transition::Length(l * unit)).collect()),
        (None, Some(a)) => as_profiel::AxisInput::Layout(pis, radii, a.into_iter().map(|a| as_profiel::Transition::Parameter(a * unit)).collect()),
    })
}

/// `CIV_WISSEL` from the ribbon: click the turnout start on the selected
/// track, on the side the diverging track goes.
struct TurnoutCmd {
    axis: as_profiel::Alignment,
    handle: ocs_plugin_api::host::codec::Handle,
    f: f64,
}

impl InteractiveCommand for TurnoutCmd {
    fn prompt(&self) -> String {
        tr("Civiel: klik het wisselbegin, aan de kant van het afbuigende spoor (wissel 1:9)", "Civil: click the turnout start, on the side of the diverging track (turnout 1:9)").into()
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let (s, off) = self.axis.station_offset([pt[0], pt[1]]);
        let (h, st) = (self.handle, self.axis.start + s / self.f);
        std::thread::spawn(move || match civiel::spoor::turnout(&civiel::settings(), Some(h), st, "1:9", off < 0.0, true, 50.0, None) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&e),
        });
        CommandStep::Done
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let (s, _) = self.axis.station_offset([pt[0], pt[1]]);
        let (foot, _) = self.axis.at(s);
        vec![PreviewWire::with_color(vec![[foot[0], foot[1], 0.0], pt], [0.95, 0.6, 0.15, 1.0])]
    }
}

fn open_settings(host: &mut dyn HostApi) {
    if let Err(e) = ui::open(ui::Tab::Civiel) {
        host.push_error(&format!("[Open Infra] {e}"));
    }
}

struct OpenInfraCiviel;

impl BuiltinPlugin for OpenInfraCiviel {
    fn manifest(&self) -> &'static PluginManifest {
        &MANIFEST
    }

    fn ribbon(&self) -> Box<dyn CadModule> {
        ui::set_product(ui::Product::Civiel);
        Box::new(CivielModule)
    }

    fn dispatch(&self, host: &mut dyn HostApi, cmd: &str) -> bool {
        let verb = cmd.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        if !verb.starts_with("CIV_") {
            return false;
        }
        ui::set_product(ui::Product::Civiel);
        bridge::attach(host);
        let rest = cmd.split_once(char::is_whitespace).map(|(_, a)| a.trim()).unwrap_or("");
        let args = Args::parse(rest);
        let id = args.id();
        if let Err(e) = run(host, &verb, &args, id.clone()) {
            refuse(host, id, &verb, e);
        }
        true
    }
}

/// KLEUR=realistisch (natural colours, the default) or KLEUR=nlcs (layer colours).
/// `STIJL=mooi|maquette|realistisch|nlcs` (or the older `KLEUR=`).
fn bag_style(args: &Args) -> Result<civiel::model3d::BagStyle, String> {
    use civiel::model3d::BagStyle;
    match args.str("STIJL") {
        Some(s) => BagStyle::parse(s).ok_or_else(|| "STIJL: mooi, maquette, realistisch of nlcs".to_string()),
        // Without a choice: the detailed look; KLEUR=nlcs keeps the layer colours.
        None => Ok(match args.str("KLEUR") {
            None => BagStyle::Mooi,
            Some(_) => if realistic(args)? { BagStyle::Realistisch } else { BagStyle::Nlcs },
        }),
    }
}

fn realistic(args: &Args) -> Result<bool, String> {
    match args.str("KLEUR").map(|s| s.to_ascii_lowercase()) {
        None => Ok(true),
        Some(k) if k.starts_with("real") || k == "foto" || k == "natuurlijk" => Ok(true),
        Some(k) if k == "nlcs" || k == "laag" => Ok(false),
        Some(_) => Err("KLEUR: realistisch of nlcs".into()),
    }
}

fn civiel_lod(l: &str) -> Result<open_infra::gis::bag3d::Lod, String> {
    open_infra::gis::bag3d::Lod::parse(l).ok_or_else(|| "LOD: 1.2, 1.3 of 2.2".to_string())
}

/// Project coordinates of an argument → drawing.
fn to_drawing(p: [f64; 2]) -> Result<[f64; 2], String> {
    Ok(civiel::Ctx::new(&civiel::settings())?.to_drawing(p))
}

fn place_arg(args: &Args) -> Result<Option<[f64; 2]>, String> {
    match args.points("PLAATS")? {
        Some(p) if !p.is_empty() => Ok(Some(to_drawing(p[0])?)),
        _ => Ok(None),
    }
}

/// The assembly arguments (RIJSTROOK=, TALUD=, …) as text, to apply on a worker.
fn assembly_overrides(args: &Args) -> String {
    corridor::Assembly::ARGS.iter().filter_map(|k| args.str(k).map(|v| format!("{k}={v}"))).collect::<Vec<_>>().join(" ")
}

/// "naam:handle;naam:handle" (DOELEN=).
fn named_handles(args: &Args, key: &str) -> Result<Vec<(String, ocs_plugin_api::host::codec::Handle)>, String> {
    let Some(v) = args.str(key) else { return Ok(Vec::new()) };
    v.split(';')
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            let (n, h) = t.split_once(':').ok_or_else(|| format!("{key}: {} '{t}'", open_infra::lang::tr("verwacht naam:handle, kreeg", "expected name:handle, got")))?;
            Ok((n.trim().to_lowercase(), api::parse_handle(h)?))
        })
        .collect()
}

/// One command: from the ribbon (no arguments: selection and clicks) or
/// from a script (arguments; see CIV_HELP).
fn run(host: &mut dyn HostApi, verb: &str, args: &Args, id: Option<String>) -> Result<(), String> {
    let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let scripted = !args.is_empty();
    let base = civiel::settings();
    let opt = |own: &[&str]| args.apply(&base, own);
    let path = || args.words.iter().find(|w| !w.contains(',') || std::path::Path::new(w).exists()).cloned();
    // Grading commands.
    if run_grading(host, verb, args, id.clone())? {
        return Ok(());
    }
    match verb {
        "CIV_PUNTEN_IMPORT" | "CIV_LANDXML" => {
            let Some(file) = path() else {
                return Err("geef een bestand: CIV_PUNTEN_IMPORT <pad> (of kies via de knop in het lint)".into());
            };
            let o = opt(&["CODES", "CODETABEL"])?;
            if verb == "CIV_LANDXML" {
                job(host, "LandXML", id, move |p| landxml::import(std::path::Path::new(&file), &o, p));
            } else {
                // CODES=JA: the point codes become linework and symbols.
                let table = args.str("CODETABEL").map(std::path::PathBuf::from);
                let codes = match args.str("CODES").map(|v| v.to_ascii_lowercase()) {
                    Some(v) if matches!(v.as_str(), "ja" | "1" | "true" | "aan") => Some(landmeten::CodeOptions { table }),
                    None if table.is_some() => Some(landmeten::CodeOptions { table }),
                    _ => None,
                };
                job(host, "Punten importeren", id, move |_| landmeten::import_coded(std::path::Path::new(&file), &o, codes));
            }
        }
        "CIV_PUNTEN_EXPORT" => {
            let o = opt(&["PAD"])?;
            let file = args.str("PAD").map(std::path::PathBuf::from);
            job(host, "Punten exporteren", id, move |_| landmeten::export(&o, file));
        }
        "CIV_PUNT" => {
            let coords = args.coords()?;
            if coords.is_empty() {
                *placer().lock().unwrap() = None;
                host.start_interactive(Box::new(PlaceCmd));
                return Ok(());
            }
            let o = opt(&["NR", "CODE"])?;
            let (nr, code) = (args.str("NR").map(str::to_string), args.str("CODE").unwrap_or("").to_string());
            let pts: Vec<([f64; 2], Option<f64>)> = coords.iter().map(|c| Ok((to_drawing([c[0], c[1]])?, c.get(2).copied()))).collect::<Result<_, String>>()?;
            job(host, "Punten plaatsen", id, move |_| {
                let mut placer = landmeten::Placer::new(&o)?;
                let mut lines = Vec::new();
                for (i, (at, z)) in pts.into_iter().enumerate() {
                    let name = nr.as_ref().map(|n| if i == 0 { n.clone() } else { format!("{n}.{i}") });
                    lines.push(placer.place_with(at, z, name, code.clone())?);
                }
                api::note("punten", lines.len());
                Ok(lines.join(" "))
            });
        }
        "CIV_TIN" => {
            let mut own = vec!["BRON", "LAAG"];
            own.extend(civiel::puntenwolk::TinVraag::ARGS);
            let o = opt(&own)?;
            let source = match args.str("BRON").map(|s| s.to_ascii_uppercase()) {
                None | Some(_) if args.str("LAAG").is_some() => terrein::TinSource::Layer(args.str("LAAG").unwrap().to_string()),
                None if args.str("BESTAND").is_some() => terrein::TinSource::Puntenwolk(civiel::puntenwolk::TinVraag::from_args(args)?),
                None => terrein::TinSource::Auto,
                Some(b) => match b.as_str() {
                    "AHN" => terrein::TinSource::Ahn,
                    "SELECTIE" => terrein::TinSource::Selection,
                    "PUNTEN" => terrein::TinSource::Points,
                    "AUTO" => terrein::TinSource::Auto,
                    "PUNTENWOLK" | "WOLK" => terrein::TinSource::Puntenwolk(civiel::puntenwolk::TinVraag::from_args(args)?),
                    _ => return Err("BRON: AHN, SELECTIE, PUNTEN, PUNTENWOLK of AUTO (of LAAG=<laagnaam>)".into()),
                },
            };
            job(host, "TIN maken", id, move |p| terrein::create(&o, source, p));
        }
        "CIV_HOOGTELIJNEN" | "CIV_ANALYSE" => {
            let o = opt(&["OPPERVLAK"])?;
            let which = match args.str("OPPERVLAK").map(|s| s.to_ascii_uppercase()) {
                None => terrein::Surface::Auto,
                Some(s) if s == "B" || s == "BESTAAND" => terrein::Surface::Status('B'),
                Some(s) if s == "N" || s == "ONTWERP" => terrein::Surface::Status('N'),
                Some(_) => return Err("OPPERVLAK: B (bestaand) of N (ontwerp)".into()),
            };
            if verb == "CIV_HOOGTELIJNEN" {
                job(host, "Hoogtelijnen", id, move |p| terrein::contours(&o, which, p));
            } else {
                job(host, "Hoogteanalyse", id, move |_| terrein::analysis(&o, which));
            }
        }
        "CIV_VOLUME" => {
            let o = opt(&["ONTWERP"])?;
            let design = match args.str("ONTWERP").map(|s| s.to_ascii_uppercase()) {
                Some(s) if s == "PEIL" => terrein::Design::Level,
                _ if args.str("PEIL").is_some() => terrein::Design::Level,
                _ => terrein::Design::Auto,
            };
            job(host, "Grondverzet", id, move |p| terrein::volume(&o, design, p));
        }
        "CIV_AS" => {
            let o = opt(&["AS", "PI", "R", "NAAM", "BEGIN", "LS", "A"])?;
            let input = layout_input(args, unit)?;
            let (name, start) = (args.str("NAAM").map(str::to_string), args.f64("BEGIN")?);
            job(host, "As maken", id, move |_| as_profiel::create(&o, input, name, start));
        }
        "CIV_AS_OFFSET" => {
            let o = opt(&["AS"])?;
            let h = args.handle("AS")?;
            job(host, "As verschuiven", id, move |_| as_profiel::offsets(&o, h));
        }
        "CIV_ONTWERPPROFIEL" => {
            let o = opt(&["AS", "PVI", "LIJN"])?;
            let (axis, line, pvi) = (args.handle("AS")?, args.handle("LIJN")?, args.pairs("PVI")?);
            job(host, "Ontwerpprofiel", id, move |_| as_profiel::design_profile(&o, line, axis, pvi));
        }
        "CIV_STATION" if scripted => {
            let o = opt(&["AS"])?;
            let doc = civiel::Ctx::new(&o)?;
            let h = args.handle("AS")?;
            let coords = args.coords()?;
            let pts: Vec<[f64; 2]> = coords.iter().map(|c| doc.to_drawing([c[0], c[1]])).collect();
            job(host, "Station / offset", id, move |_| {
                let snap = civiel::document(&[], &h.into_iter().collect::<Vec<_>>())?;
                let (al, _, _) = as_profiel::axis(&snap, h)?;
                let f = doc.f;
                let mut rows = Vec::new();
                let mut texts = Vec::new();
                for p in pts {
                    let (s, off) = al.station_offset(p);
                    rows.push(serde_json::json!({"station": al.start + s / f, "offset_m": off / f}));
                    texts.push(as_profiel::station_offset_text(&al, p, f));
                }
                api::note("punten", rows);
                Ok(texts.join(" | "))
            });
        }
        "CIV_LENGTEPROFIEL" | "CIV_DWARSPROFIELEN" if scripted => {
            let mut own = vec!["AS", "PLAATS"];
            own.extend(civiel::puntenwolk::Grondbron::ARGS);
            let mut o = opt(&own)?;
            // BRON=PUNTENWOLK: the ground line from a strip of points along the axis.
            o.puntenwolk = civiel::puntenwolk::Grondbron::from_args(args)?;
            let (h, at) = (args.handle("AS")?, place_arg(args)?);
            if verb == "CIV_LENGTEPROFIEL" {
                job(host, "Lengteprofiel", id, move |p| as_profiel::length_profile(&o, h, at, p));
            } else {
                job(host, "Dwarsprofielen", id, move |p| as_profiel::cross_sections(&o, h, at, p));
            }
        }
        "CIV_STATION" | "CIV_LENGTEPROFIEL" | "CIV_DWARSPROFIELEN" => {
            let Some(axis) = selected_axis(host) else {
                return Err("selecteer eerst één as (polylijn, lijn of boog) en kies dan de opdracht, of geef AS=<handle>".into());
            };
            match verb {
                "CIV_STATION" => host.start_interactive(Box::new(StationCmd { axis, f: unit })),
                "CIV_LENGTEPROFIEL" => {
                    let k = base.nlcs.scale / 1000.0 * unit;
                    host.start_interactive(Box::new(PlaceViewCmd { what: Place::LengthProfile, size: [axis.length(), 80.0 * k] }));
                }
                _ => {
                    let (w, h) = as_profiel::sections_extent(&axis, &base, unit);
                    host.start_interactive(Box::new(PlaceViewCmd { what: Place::CrossSections, size: [w, h] }));
                }
            }
        }
        "CIV_CORRIDOR" => {
            let mut own = vec!["AS", "NIVEAU", "BOVEN", "DWARSPROFIELEN", "PROFIELTYPE", "VAN", "TOT", "DOELEN", "KLEUR"];
            own.extend(corridor::Assembly::ARGS);
            let o = opt(&own)?;
            let asm_name = args.str("PROFIELTYPE").map(str::to_string).unwrap_or_else(|| o.assembly.clone());
            let overrides = assembly_overrides(args);
            let grade = match (args.f64("NIVEAU")?, args.f64("BOVEN")?) {
                (Some(z), _) => Some(corridor::Grade::Level(z)),
                (None, Some(dz)) => Some(corridor::Grade::AboveGround(dz)),
                _ => None,
            };
            let views = match args.str("DWARSPROFIELEN") {
                Some(v) if v.contains(',') => place_arg(&Args::parse(&format!("PLAATS={v}")))?,
                Some(v) if matches!(v.to_ascii_lowercase().as_str(), "ja" | "yes" | "1" | "true") => Some([f64::NAN, f64::NAN]),
                None if o.corridor_views => Some([f64::NAN, f64::NAN]),
                _ => None,
            };
            let axis_h = args.handle("AS")?;
            if axis_h.is_none() && !scripted && selected_axis(host).is_none() {
                return Err(open_infra::lang::tr("selecteer eerst de as (met ontwerpprofiel) of geef AS=<handle>", "select the alignment (with a design profile) first, or give AS=<handle>").into());
            }
            let (from, to) = (args.f64("VAN")?, args.f64("TOT")?);
            let targets = named_handles(args, "DOELEN")?;
            let natural = realistic(args)?;
            let every = o.corridor_every;
            job(host, "Corridor", id, move |p| {
                let asm = civiel::profieltype::find(&asm_name)?.with_args(&Args::parse(&overrides))?;
                // Views without a place (NaN): the corridor finds one.
                corridor::corridor(&o, corridor::Request { axis: axis_h, assembly: asm, grade, every, views, from, to, targets, natural }, p)
            });
        }
        // Corridor stream: assemblies, junctions and roundabouts.
        "CIV_PROFIELTYPEN" => {
            opt(&[])?;
            job(host, open_infra::lang::tr("Profieltypen", "Assemblies"), id, |_| civiel::profieltype::list_command());
        }
        "CIV_PROFIELTYPE" => {
            let o = opt(&["NAAM", "KOPIE", "PLAATS"])?;
            let name = args.str("NAAM").map(str::to_string).or_else(|| args.words.first().cloned()).ok_or_else(|| {
                open_infra::lang::tr("geef NAAM=<profieltype> (CIV_PROFIELTYPEN geeft de lijst)", "give NAAM=<assembly> (CIV_PROFIELTYPEN lists them)").to_string()
            })?;
            let copy = args.str("KOPIE").map(str::to_string);
            let at = place_arg(args)?;
            job(host, open_infra::lang::tr("Profieltype", "Assembly"), id, move |_| civiel::profieltype::show(&o, &name, copy.as_deref(), at));
        }
        "CIV_KRUISPUNT" => {
            let o = opt(&["HOOFD", "ZIJ", "R", "PROFIELTYPE", "PROFIELTYPE_HOOFD", "PROFIELTYPE_ZIJ", "NIVEAU", "NAAM", "KLEUR"])?;
            let (main, side) = match (args.handle("HOOFD")?, args.handle("ZIJ")?) {
                (Some(m), Some(s)) => (m, s),
                _ if !scripted => {
                    let sel = host.selection();
                    if sel.len() != 2 {
                        return Err(open_infra::lang::tr("selecteer de hoofdweg en de zijweg (twee assen), of geef HOOFD=<handle> ZIJ=<handle>", "select the main road and the side road (two alignments), or give HOOFD=<handle> ZIJ=<handle>").into());
                    }
                    (sel[0], sel[1])
                }
                _ => return Err(open_infra::lang::tr("geef HOOFD=<handle> en ZIJ=<handle> (de assen)", "give HOOFD=<handle> and ZIJ=<handle> (the alignments)").into()),
            };
            let both = args.str("PROFIELTYPE").map(str::to_string);
            let req = civiel::kruispunt::JunctionRequest {
                main,
                side,
                main_asm: args.str("PROFIELTYPE_HOOFD").map(str::to_string).or(both.clone()),
                side_asm: args.str("PROFIELTYPE_ZIJ").map(str::to_string).or(both),
                radii: args.list("R")?.unwrap_or_default(),
                level: args.f64("NIVEAU")?,
                name: args.str("NAAM").map(str::to_string),
                auto_order: !scripted,
                natural: realistic(args)?,
            };
            job(host, open_infra::lang::tr("Kruispunt", "Junction"), id, move |p| civiel::kruispunt::junction(&o, req, p));
        }
        "CIV_ROTONDE" => {
            let o = opt(&["ASSEN", "MIDDEN", "TYPE", "D", "RIJBAAN", "OVERRIJDBAAR", "IN", "UIT", "INBREEDTE", "UITBREEDTE", "MIDDENGELEIDER", "LENGTE", "AANSLUITING", "HELLING", "PROFIELTYPE", "NIVEAU", "NAAM", "KLEUR"])?;
            let axes = match args.str("ASSEN") {
                Some(list) => list.split(';').filter(|t| !t.trim().is_empty()).map(api::parse_handle).collect::<Result<Vec<_>, _>>()?,
                None if !scripted => host.selection(),
                None => Vec::new(),
            };
            let built_up = match args.str("TYPE").map(|t| t.to_ascii_lowercase()) {
                None => true,
                Some(t) if t.starts_with("binnen") || t == "bibeko" || t == "urban" => true,
                Some(t) if t.starts_with("buiten") || t == "bubeko" || t == "rural" => false,
                Some(_) => return Err(open_infra::lang::tr("TYPE: binnen (bebouwde kom) of buiten", "TYPE: binnen (built-up area) or buiten (rural)").into()),
            };
            let mut dims = civiel::kruispunt::RoundDims::crow(built_up);
            if let Some(d) = args.f64("D")? {
                let w = dims.r_out - dims.r_in;
                dims.r_out = d / 2.0;
                dims.r_in = dims.r_out - w;
            }
            if let Some(w) = args.f64("RIJBAAN")? {
                dims.r_in = dims.r_out - w;
            }
            for (key, field) in [
                ("OVERRIJDBAAR", &mut dims.apron),
                ("IN", &mut dims.r_entry),
                ("UIT", &mut dims.r_exit),
                ("INBREEDTE", &mut dims.entry_w),
                ("UITBREEDTE", &mut dims.exit_w),
                ("MIDDENGELEIDER", &mut dims.split_w),
                ("LENGTE", &mut dims.split_l),
                ("AANSLUITING", &mut dims.taper),
            ] {
                if let Some(v) = args.f64(key)? {
                    *field = v;
                }
            }
            if let Some(h) = args.f64("HELLING")? {
                dims.slope = h / 100.0;
            }
            if dims.r_in <= dims.apron + 1.0 || dims.r_out <= dims.r_in {
                return Err(open_infra::lang::tr("de maten passen niet: D, RIJBAAN en OVERRIJDBAAR", "the dimensions do not fit: D, RIJBAAN and OVERRIJDBAAR").into());
            }
            let centre = match args.points("MIDDEN")? {
                Some(p) if !p.is_empty() => Some(to_drawing(p[0])?),
                _ => None,
            };
            let req = civiel::kruispunt::RoundaboutRequest {
                axes,
                centre,
                dims,
                asm: args.str("PROFIELTYPE").map(str::to_string),
                level: args.f64("NIVEAU")?,
                name: args.str("NAAM").map(str::to_string),
                natural: realistic(args)?,
            };
            job(host, open_infra::lang::tr("Rotonde", "Roundabout"), id, move |p| civiel::kruispunt::roundabout(&o, req, p));
        }
        "CIV_BAG3D" => {
            let o = opt(&["LOD", "MAX", "KLEUR", "STIJL", "RAMEN"])?;
            let real = bag_style(args)?;
            let windows = !matches!(args.str("RAMEN").map(|s| s.to_ascii_lowercase()).as_deref(), Some("nee" | "no" | "0" | "false" | "uit"));
            let lod = match args.str("LOD") {
                Some(l) => civiel_lod(l)?,
                None => open_infra::gis::bag3d::Lod::L22,
            };
            let max = args.f64("MAX")?.map(|m| m as usize).unwrap_or(25_000);
            job(host, "3D BAG", id, move |p| civiel::model3d::bag3d(&o, lod, max, real, windows, p));
        }
        "CIV_DOORZICHT" => {
            let off = args.words.iter().any(|w| w.eq_ignore_ascii_case("UIT") || w.eq_ignore_ascii_case("OFF"));
            let all = args.words.iter().any(|w| w.eq_ignore_ascii_case("ALLES"));
            let pct = if off { 0.0 } else { args.f64("PCT")?.unwrap_or(70.0) };
            job(host, "Maaiveld doorzichtig", id, move |_| civiel::model3d::see_through(pct, all));
        }
        "CIV_KLIC3D" => {
            let file = path().map(std::path::PathBuf::from).or_else(open_infra::basisdata::last_klic);
            let Some(file) = file else {
                return Err("geef een KLIC-levering: CIV_KLIC3D <zip of GI_…xml> [VERGROOT=1] (of importeer eerst de KLIC op het GIS-tabblad)".into());
            };
            let o = opt(&["VERGROOT"])?;
            let scale = args.f64("VERGROOT")?.unwrap_or(1.0).clamp(0.2, 20.0);
            job(host, "KLIC in 3D", id, move |p| civiel::model3d::klic3d(&o, &file, scale, p));
        }
        "CIV_CITYGML" => {
            let Some(file) = path() else {
                return Err("geef een bestand: CIV_CITYGML <pad> [STIJL=mooi|maquette|nlcs] (of kies via de knop in het lint)".into());
            };
            let o = opt(&["STIJL", "KLEUR"])?;
            let style = bag_style(args)?;
            job(host, "CityGML", id, move |p| civiel::model3d::citygml(&o, std::path::Path::new(&file), style, p));
        }
        "CIV_FOTOTERREIN" => {
            let o = opt(&["FOTO"])?;
            let cell = args.f64("CEL")?;
            let foto = args.str("FOTO").map(str::to_string);
            job(host, "Terrein met luchtfoto", id, move |p| civiel::model3d::photo_terrain(&o, cell, foto.as_deref(), p));
        }
        "CIV_WATERPEIL" => {
            let o = opt(&["PEIL"])?;
            let level = args.f64("PEIL")?.or(args.words.first().and_then(|w| w.replace(',', ".").parse().ok())).ok_or("CIV_WATERPEIL PEIL=<m NAP>, bijv. PEIL=2,5")?;
            job(host, "Waterpeil", id, move |p| civiel::model3d::water_level(&o, level, p));
        }
        "CIV_BGT3D" => {
            let o = opt(&["GROEPEN", "PANDEN", "KLEUR"])?;
            let real = realistic(args)?;
            let groups: Vec<String> = match args.str("GROEPEN") {
                Some(g) => g.split([';', ',']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                None => civiel::model3d::default_groups().into_iter().map(str::to_string).collect(),
            };
            let panden = !matches!(args.str("PANDEN").map(|s| s.to_ascii_lowercase()).as_deref(), Some("nee" | "no" | "0" | "false"));
            job(host, "BGT naar 3D", id, move |p| civiel::model3d::bgt3d(&o, &groups, panden, real, p));
        }
        "CIV_3DMODEL" => {
            // All at once: photo terrain, 3D BAG, the BGT without its buildings.
            let o = opt(&["LOD", "MAX", "KLEUR", "STIJL", "RAMEN"])?;
            let real = bag_style(args)?;
            let windows = !matches!(args.str("RAMEN").map(|s| s.to_ascii_lowercase()).as_deref(), Some("nee" | "no" | "0" | "false" | "uit"));
            let lod = match args.str("LOD") {
                Some(l) => civiel_lod(l)?,
                None => open_infra::gis::bag3d::Lod::L22,
            };
            let max = args.f64("MAX")?.map(|m| m as usize).unwrap_or(25_000);
            let cell = args.f64("CEL")?;
            job(host, "3D-model", id, move |p| {
                let mut parts = vec![civiel::model3d::photo_terrain(&o, cell, None, p)?];
                let groups: Vec<String> = civiel::model3d::default_groups().into_iter().map(str::to_string).collect();
                parts.push(civiel::model3d::bgt3d(&o, &groups, false, real != civiel::model3d::BagStyle::Nlcs, p)?);
                parts.push(civiel::model3d::bag3d(&o, lod, max, real, windows, p)?);
                Ok(parts.join(" | "))
            });
        }
        "CIV_CODES_TOEPASSEN" => {
            let o = opt(&["CODETABEL"])?;
            let table = args.str("CODETABEL").map(std::path::PathBuf::from);
            job(host, open_infra::lang::tr("Codes toepassen", "Apply codes"), id, move |_| landmeten::apply_in_drawing(&o, landmeten::CodeOptions { table }));
        }
        "CIV_PERCELEN" => {
            use civiel::percelen::{Direction, Remainder, Request, Sizing};
            let mut o = opt(&["KAVEL", "STRAAT", "N", "OPP", "FRONT", "RICHTING", "REST", "PREFIX", "START"])?;
            // New parcels are design (N) unless STATUS= says otherwise.
            if args.str("STATUS").is_none() {
                o.nlcs.status = open_infra::nlcs::Status::N;
            }
            let rest = match args.str("REST").map(|v| v.to_ascii_lowercase()) {
                None => Remainder::Last,
                Some(v) if v == "laatste" || v == "last" => Remainder::Last,
                Some(v) if v == "apart" || v == "eigen" || v == "own" => Remainder::Own,
                Some(v) if v == "verdelen" || v == "spread" => Remainder::Spread,
                Some(_) => return Err(open_infra::lang::tr("REST: laatste, apart of verdelen", "REST: laatste (last), apart (own) or verdelen (spread)").into()),
            };
            let (n, area, front) = (args.f64("N")?, args.f64("OPP")?, args.f64("FRONT")?);
            let sizing = match (n, area, front) {
                (Some(n), _, _) => Sizing::Equal((n.round() as usize).clamp(1, 10_000)),
                (None, None, None) => Sizing::Equal(2),
                (None, a, w) => Sizing::Min { area: a, front: w, rest },
            };
            let direction = match (args.f64("RICHTING")?, args.handle("STRAAT")?) {
                (Some(deg), _) => Direction::Angle(deg),
                (None, Some(h)) => Direction::Street(Some(h)),
                (None, None) => Direction::Street(None),
            };
            let plot = args.handle("KAVEL")?;
            if plot.is_none() && host.selection().is_empty() {
                return Err(open_infra::lang::tr(
                    "selecteer het perceel (gesloten polylijn) en eventueel de straatlijn, of geef KAVEL=<handle> [STRAAT=<handle>] N=|OPP=|FRONT=",
                    "select the plot (closed polyline) and optionally the street line, or give KAVEL=<handle> [STRAAT=<handle>] N=|OPP=|FRONT=",
                )
                .into());
            }
            let req = Request {
                plot,
                direction,
                sizing,
                prefix: args.str("PREFIX").unwrap_or("P").to_string(),
                first: args.f64("START")?.map(|v| v.max(0.0) as usize).unwrap_or(1),
                id: id.clone(),
            };
            job(host, open_infra::lang::tr("Percelen", "Parcels"), id, move |_| civiel::percelen::parcels(&o, req));
        }
        "CIV_PERCEEL_INFO" => {
            let o = opt(&["PERCEEL", "STRAAT"])?;
            let hs = args.str("PERCEEL").map(|v| v.split(';').filter(|t| !t.trim().is_empty()).map(api::parse_handle).collect::<Result<Vec<_>, _>>()).transpose()?.unwrap_or_default();
            let street = args.handle("STRAAT")?;
            job(host, open_infra::lang::tr("Perceelinfo", "Parcel info"), id, move |_| civiel::percelen::info(&o, hs, street));
        }
        "CIV_WATERDRUPPEL" => {
            let o = opt(&["PUNTEN"])?;
            let cell = args.f64("CEL")?.unwrap_or(1.0).max(0.25);
            let mut pts: Vec<[f64; 2]> = args.points("PUNTEN")?.unwrap_or_default();
            pts.extend(args.coords()?.into_iter().map(|c| [c[0], c[1]]));
            if pts.is_empty() {
                *drops().lock().unwrap() = None;
                host.start_interactive(Box::new(DropCmd { cell }));
                return Ok(());
            }
            let pts: Vec<[f64; 2]> = pts.into_iter().map(to_drawing).collect::<Result<_, _>>()?;
            job(host, open_infra::lang::tr("Waterdruppel", "Water drop"), id, move |p| civiel::hydrologie::DropSession::new(&o, cell, p)?.run(&pts));
        }
        "CIV_STROOMGEBIED" | "CIV_PLASSEN" => {
            let o = opt(&["PUNTEN", "LAAG", "BRON", "SNAP", "PLASSEN", "MINDIEPTE", "MINOPP"])?;
            let ponds_only = verb == "CIV_PLASSEN";
            let req = catchment_request(args, ponds_only)?;
            if !ponds_only && req.outlets.points.is_empty() && req.outlets.layers.is_empty() {
                if scripted && id.is_some() {
                    return Err(open_infra::lang::tr("geef afvoerpunten: PUNTEN=x,y;… of LAAG=<laag>, bijv. LAAG=*KOLK*", "give outlets: PUNTEN=x,y;… or LAAG=<layer>, e.g. LAAG=*KOLK*").into());
                }
                host.start_interactive(Box::new(OutletCmd { points: Vec::new(), req: Some(req) }));
                return Ok(());
            }
            let label = if ponds_only { open_infra::lang::tr("Plassen", "Ponds") } else { open_infra::lang::tr("Stroomgebied", "Catchment") };
            job(host, label, id, move |p| civiel::hydrologie::catchment(&o, req, p));
        }
        // Point clouds: a TIN from the AHN points (of the site or the selected
        // closed polyline) or from a file, and a 3D mesh of an object.
        "CIV_PUNTENWOLK_TIN" | "CIV_PUNTENWOLK_BESTAND" => {
            let o = opt(civiel::puntenwolk::TinVraag::ARGS)?;
            let mut v = civiel::puntenwolk::TinVraag::from_args(args)?;
            // From the file dialog: the path as the rest of the line (it may hold spaces).
            let loose = args.words.join(" ");
            if args.str("BESTAND").is_none() && !loose.trim().is_empty() {
                v.invoer = civiel::puntenwolk::Invoer::file(&loose)?;
            } else if verb == "CIV_PUNTENWOLK_BESTAND" && args.str("BESTAND").is_none() {
                return Err(open_infra::lang::tr("geef een puntenwolkbestand: CIV_PUNTENWOLK_BESTAND <pad> (of kies via de knop in het lint)", "give a point cloud file: CIV_PUNTENWOLK_BESTAND <path> (or pick one with the ribbon button)").into());
            }
            job(host, open_infra::lang::tr("TIN uit puntenwolk", "TIN from point cloud"), id, move |p| terrein::create(&o, terrein::TinSource::Puntenwolk(v), p));
        }
        "CIV_MESH" => {
            let o = opt(civiel::mesh::MeshVraag::ARGS)?;
            let mut v = civiel::mesh::MeshVraag::from_args(args)?;
            // From the file dialog: the path as the rest of the line.
            let loose = args.words.join(" ");
            if args.str("BESTAND").is_none() && !loose.trim().is_empty() {
                v.invoer = civiel::puntenwolk::Invoer::file(&loose)?;
            }
            job(host, open_infra::lang::tr("3D-mesh", "3D mesh"), id, move |p| civiel::mesh::create(&o, &v, p));
        }
        // Exchange: LandXML 1.2 out (stream uitwisseling).
        "CIV_LANDXML_EXPORT" => {
            let o = opt(&["BESTAND", "PAD", "ALLES"])?;
            let file = args.str("BESTAND").or(args.str("PAD")).map(str::to_string).or_else(|| args.words.iter().find(|w| !w.eq_ignore_ascii_case("ALLES") && !w.eq_ignore_ascii_case("ALL")).cloned());
            let all = args.words.iter().any(|w| w.eq_ignore_ascii_case("ALLES") || w.eq_ignore_ascii_case("ALL"))
                || args.str("ALLES").is_some_and(|v| !matches!(v.to_ascii_lowercase().as_str(), "nee" | "no" | "0" | "false"));
            let path = file.map(std::path::PathBuf::from);
            job(host, "LandXML export", id, move |p| landxml::export(&o, path, all, p));
        }
        "CIV_SONDERINGEN3D" | "CIV_ONDERGROND3D" => {
            // LABELS= is the shared point-label setting: plan symbols too.
            let o = opt(&["BRON", "MAX", "STRAAL", "QC", "MINLAAG"])?;
            let off = |k: &str| matches!(args.str(k).map(|s| s.to_ascii_lowercase()).as_deref(), Some("nee" | "no" | "0" | "false" | "uit"));
            let req = civiel::ondergrond::Request {
                kinds: civiel::ondergrond::kinds(args.str("BRON"))?,
                max: args.f64("MAX")?.map(|m| m.max(1.0) as usize).unwrap_or(100),
                radius: args.f64("STRAAL")?.unwrap_or(0.4).clamp(0.05, 5.0),
                min_layer: args.f64("MINLAAG")?.unwrap_or(0.2).clamp(0.0, 5.0),
                graphs: !off("QC"),
                labels: o.point_labels,
            };
            job(host, "Sonderingen 3D", id, move |p| civiel::ondergrond::columns(&o, &req, p));
        }
        "CIV_IFC" => {
            opt(&["BESTAND", "PAD", "TEKST", "IDS"])?;
            let (file, selection, texts) = open_infra::ifc::args(args);
            // IDS=: export, then check the export against the client's IDS.
            match args.str("IDS").map(std::path::PathBuf::from) {
                Some(ids) => job(host, "IFC 4x3 + IDS", id, move |p| civiel::ids::export_and_check(file, selection, texts, ids, p)),
                None => job(host, "IFC 4x3", id, move |p| open_infra::ifc::command(file, selection, texts, p)),
            }
        }
        // Exchange: IFC in, IDS checks, 3D meshes out and in (#66, #65, #71).
        "CIV_IFC_IMPORT" => {
            let Some(file) = path() else {
                return Err(tr("geef een bestand: CIV_IFC_IMPORT <pad.ifc> (of kies via de knop in het lint)", "give a file: CIV_IFC_IMPORT <path.ifc> (or pick one with the ribbon button)").into());
            };
            let o = opt(&[])?;
            // STATUS= sets the terrain's status too (B by default).
            let status = args.str("STATUS").and_then(|s| open_infra::nlcs::Status::from_code(&s.to_ascii_uppercase()));
            job(host, "IFC", id, move |p| civiel::ifc_import::import(&o, std::path::Path::new(&file), status, p));
        }
        "CIV_IFC_IDS" => {
            opt(&["IDS", "IFC"])?;
            let ends = |w: &String, ext: &[&str]| ext.iter().any(|e| w.to_ascii_lowercase().ends_with(e));
            let ids = args.str("IDS").map(str::to_string).or_else(|| args.words.iter().find(|w| ends(*w, &[".ids", ".xml"])).cloned());
            let Some(ids) = ids.map(std::path::PathBuf::from) else {
                return Err(tr("geef de IDS: CIV_IFC_IDS IDS=<pad.ids> [IFC=<pad.ifc>]", "give the IDS: CIV_IFC_IDS IDS=<path.ids> [IFC=<path.ifc>]").into());
            };
            let ifc = args.str("IFC").map(str::to_string).or_else(|| args.words.iter().find(|w| ends(*w, &[".ifc"])).cloned()).map(std::path::PathBuf::from);
            job(host, "IDS", id, move |p| civiel::ids::command(ids, ifc, p));
        }
        "CIV_3D_EXPORT" => {
            // FORMAAT= is this command's own (GLB, OBJ, STL), not the point file format.
            args.without(&["FORMAAT"]).apply(&base, &["PAD", "BESTAND"])?;
            use civiel::uitwisseling3d::Formaat;
            let file = args.str("PAD").or(args.str("BESTAND")).map(str::to_string).or_else(|| args.words.iter().find(|w| !w.eq_ignore_ascii_case("SELECTIE") && !w.eq_ignore_ascii_case("SELECTION")).cloned()).map(std::path::PathBuf::from);
            let formaat = match args.str("FORMAAT") {
                Some(f) => Formaat::parse(f)?,
                None => file.as_deref().and_then(Formaat::of_path).unwrap_or(Formaat::Glb),
            };
            let selection = args.words.iter().any(|w| w.eq_ignore_ascii_case("SELECTIE") || w.eq_ignore_ascii_case("SELECTION"));
            job(host, "3D export", id, move |p| civiel::uitwisseling3d::export(formaat, selection, file, p));
        }
        "CIV_3D_IMPORT" => {
            let Some(file) = path() else {
                return Err(tr("geef een bestand: CIV_3D_IMPORT <pad.obj|stl|glb> (of kies via de knop in het lint)", "give a file: CIV_3D_IMPORT <path.obj|stl|glb> (or pick one with the ribbon button)").into());
            };
            // SCHAAL= is the model's scale here, not the drawing scale.
            let o = args.without(&["SCHAAL"]).apply(&base, &["PLAATS", "ROTATIE", "LAAG"])?;
            let plaats = match args.str("PLAATS") {
                Some(v) => {
                    let c: Vec<f64> = v.split(',').map(|t| t.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| tr("PLAATS: x,y of x,y,z in projectcoördinaten", "PLAATS: x,y or x,y,z in project coordinates").to_string())?;
                    if c.len() < 2 || c.len() > 3 {
                        return Err(tr("PLAATS: x,y of x,y,z in projectcoördinaten", "PLAATS: x,y or x,y,z in project coordinates").into());
                    }
                    Some([c[0], c[1], c.get(2).copied().unwrap_or(0.0)])
                }
                None => None,
            };
            let schaal = args.f64("SCHAAL")?.unwrap_or(1.0);
            if !(schaal > 0.0 && schaal.is_finite()) {
                return Err(tr("SCHAAL moet groter dan 0 zijn", "SCHAAL must be greater than 0").into());
            }
            let pl = civiel::uitwisseling3d::Plaatsing { plaats, schaal, rotatie: args.f64("ROTATIE")?.unwrap_or(0.0) };
            let laag = args.str("LAAG").map(str::to_string);
            job(host, tr("3D-model", "3D model"), id, move |p| civiel::uitwisseling3d::import(&o, std::path::Path::new(&file), pl, laag, p));
        }
        // Plan production: sheets and labels.
        "CIV_BLADEN" => {
            use open_infra::lang::tr;
            let o = args.without(&["FORMAAT", "SCHAAL"]).apply(&base, &["AS", "VSCHAAL", "OVERLAP", "PROJECT", "ONDERWERP", "NUMMER"])?;
            let paper = match args.str("FORMAAT") {
                Some(p) => civiel::bladen::Paper::parse(p).ok_or(tr("FORMAAT: A0, A1, A2 of A3", "FORMAAT: A0, A1, A2 or A3"))?,
                None => civiel::bladen::Paper::parse("A1").expect("A1"),
            };
            let scale = match args.str("SCHAAL") {
                Some(s) => civiel::bladen::parse_scale(s).ok_or(tr("SCHAAL: bijv. 1:1000 of 500", "SCHAAL: e.g. 1:1000 or 500"))?,
                None => base.nlcs.scale,
            };
            // VSCHAAL=10 is an exaggeration, VSCHAAL=1:100 a vertical scale.
            let exaggeration = match args.str("VSCHAAL") {
                Some(v) if v.contains([':', '/']) => scale / civiel::bladen::parse_scale(v).ok_or(tr("VSCHAAL: bijv. 10 of 1:100", "VSCHAAL: e.g. 10 or 1:100"))?,
                Some(_) => args.f64("VSCHAAL")?.unwrap_or(base.exaggeration),
                None => base.exaggeration,
            };
            let axis_h = args.handle("AS")?;
            if axis_h.is_none() && !scripted && selected_axis(host).is_none() {
                return Err(tr("selecteer eerst één as, of geef AS=<handle>", "select one alignment first, or give AS=<handle>").into());
            }
            let req = civiel::bladen::Request {
                axis: axis_h,
                paper,
                scale,
                exaggeration,
                overlap: args.f64("OVERLAP")?,
                project: args.str("PROJECT").map(str::to_string),
                subject: args.str("ONDERWERP").map(str::to_string),
                number: args.str("NUMMER").map(str::to_string),
            };
            job(host, tr("Bladen", "Sheets"), id, move |p| civiel::bladen::create(&o, req, p));
        }
        "CIV_BLADEN_PDF" => {
            opt(&["AS", "NAAM", "MAP"])?;
            let (h, name, dir) = (args.handle("AS")?, args.str("NAAM").map(str::to_string), args.str("MAP").map(std::path::PathBuf::from));
            job(host, "PDF", id, move |p| {
                let name = match (name, h) {
                    (Some(n), _) => Some(n),
                    (None, Some(h)) => bridge::entities(&[h])?.into_iter().flatten().next().and_then(|e| civiel::data_value(&e, "as")),
                    (None, None) => None,
                };
                civiel::bladen::pdf(name, dir, p)
            });
        }
        "CIV_LABELS" => {
            use open_infra::lang::tr;
            let o = opt(&["AS", "SOORT", "PUNTEN", "RASTER", "OPPERVLAK"])?;
            let kinds = match args.str("SOORT") {
                Some(k) => civiel::labels::Kind::list(k)?,
                None => Vec::new(),
            };
            let points: Vec<[f64; 2]> = args.points("PUNTEN")?.unwrap_or_default().into_iter().map(to_drawing).collect::<Result<_, _>>()?;
            let surface = match args.str("OPPERVLAK").map(|s| s.to_ascii_uppercase()) {
                None => 'B',
                Some(s) if s == "B" || s == "BESTAAND" => 'B',
                Some(s) if s == "N" || s == "ONTWERP" => 'N',
                Some(_) => return Err(tr("OPPERVLAK: B (bestaand) of N (ontwerp)", "OPPERVLAK: B (existing) or N (design)").into()),
            };
            let req = civiel::labels::Request { kinds, axis: args.handle("AS")?, points, grid: args.f64("RASTER")?, surface };
            job(host, "Labels", id, move |p| civiel::labels::create(&o, req, p));
        }
        "CIV_BIJWERKEN" => {
            let o = opt(&["CONTROLE"])?;
            let only = args.words.iter().any(|w| w.eq_ignore_ascii_case("CONTROLE") || w.eq_ignore_ascii_case("CHECK"));
            job(host, "Bijwerken", id, move |p| civiel::graaf::update(&o, only, p));
        }
        "CIV_LABELS_BIJWERKEN" => {
            let o = opt(&[])?;
            job(host, open_infra::lang::tr("Labels bijwerken", "Update labels"), id, move |p| civiel::labels::update(&o, p));
        }
        // ── Riolering (src/civiel/riool.rs) ──
        "CIV_RIOOL" => {
            use open_infra::lang::tr;
            let mut o = opt(riool::ONTWERP_ARGS)?;
            // A design: new work, unless STATUS= says otherwise.
            if args.str("STATUS").is_none() {
                o.nlcs.status = open_infra::nlcs::Status::N;
            }
            let design = riool::Ontwerp::from_args(args)?;
            let input = match (args.points("PUNTEN")?, args.handle("LIJN")?) {
                (Some(p), _) => riool::Invoer::Project(p),
                (None, Some(h)) => riool::Invoer::Lijn(Some(h)),
                (None, None) if host.selection().len() == 1 => riool::Invoer::Lijn(None),
                (None, None) => {
                    host.start_interactive(Box::new(RioolCmd { pts: Vec::new(), design, opt: o }));
                    return Ok(());
                }
            };
            job(host, tr("Riool ontwerpen", "Design sewer"), id, move |p| riool::ontwerp(&o, input, design, p));
        }
        "CIV_RIOOL_BIJWERKEN" => {
            let mut o = opt(riool::BIJWERKEN_ARGS)?;
            if args.str("STATUS").is_none() {
                o.nlcs.status = open_infra::nlcs::Status::N;
            }
            let w = riool::Wijziging::from_args(args)?;
            let naam = args.str("RIOOL").map(str::to_string);
            job(host, open_infra::lang::tr("Riool bijwerken", "Update sewer"), id, move |p| riool::bijwerken(&o, naam, w, p));
        }
        "CIV_RIOOL_PROFIEL" => {
            let o = opt(&["RIOOL", "VAN", "NAAR", "PLAATS"])?;
            let (naam, van, naar) = (args.str("RIOOL").map(str::to_string), args.str("VAN").map(str::to_string), args.str("NAAR").map(str::to_string));
            let at = place_arg(args)?;
            job(host, open_infra::lang::tr("Lengteprofiel riool", "Sewer length profile"), id, move |p| riool::profiel(&o, naam, van, naar, at, p));
        }
        "CIV_RIOOL_TOETS" => {
            let o = opt(riool::TOETS_ARGS)?;
            let t = riool::Toets::from_args(args)?;
            let naam = args.str("RIOOL").map(str::to_string);
            job(host, open_infra::lang::tr("Riooltoets", "Sewer check"), id, move |p| riool::toets(&o, naam, t, p));
        }
        "CIV_RIOOL_HYDRAULICA" => {
            let o = opt(riool::HYDRAULICA_ARGS)?;
            let h = riool::Hydraulica::from_args(args)?;
            let naam = args.str("RIOOL").map(str::to_string);
            job(host, open_infra::lang::tr("Rioolhydraulica", "Sewer hydraulics"), id, move |p| riool::hydraulica(&o, naam, h, p));
        }
        "CIV_RIOOL_EXPORT" => {
            // FORMAAT= is this command's own (GWSW or CSV), not the point file format.
            let o = args.without(&["FORMAAT"]).apply(&base, &["RIOOL", "PAD"])?;
            let formaat = match args.str("FORMAAT") {
                Some(f) => riool::Formaat::parse(f).ok_or(open_infra::lang::tr("FORMAAT: GWSW of CSV", "FORMAAT: GWSW or CSV"))?,
                None => riool::Formaat::Gwsw,
            };
            let (naam, pad) = (args.str("RIOOL").map(str::to_string), args.str("PAD").map(std::path::PathBuf::from));
            job(host, open_infra::lang::tr("Riool exporteren", "Export sewer"), id, move |_| riool::export(&o, naam, formaat, pad));
        }
        "CIV_RIOOL_IMPORT" => {
            let Some(file) = path() else {
                return Err(open_infra::lang::tr("geef een bestand: CIV_RIOOL_IMPORT <pad.orox.ttl> (of kies via de knop in het lint)", "give a file: CIV_RIOOL_IMPORT <path.orox.ttl> (or pick one with the ribbon button)").into());
            };
            let o = opt(&[])?;
            job(host, open_infra::lang::tr("GWSW-bestand", "GWSW file"), id, move |_| riool::import(&o, std::path::Path::new(&file)));
        }
        "CIV_RIOOL_GWSW" => {
            let o = opt(&["NAAM"])?;
            let naam = args.str("NAAM").map(str::to_string);
            job(host, open_infra::lang::tr("GWSW overnemen", "Take over GWSW"), id, move |p| riool::van_gwsw(&o, naam, p));
        }
        "CIV_RIOOL_HELP" => host.push_info(
            "[Open Infra — Riolering / Sewers] CIV_RIOOL [LIJN=<handle> | PUNTEN=x,y;x,y;… | selectie | klikken] [STELSEL=DWA|HWA|GWA|ITR] [DIAMETER=mm] [MATERIAAL=beton|PVC|PP|PE|gres] \
             [DEKKING=m] [VERHANG=1:250|4 (‰)] [BOB=m NAP|AUTO] [PUT=800|800x1000] [PUTNR=P101] [NAAM=] [OMKEREN=ja] [MAAIVELD=AUTO|ONTWERP|AHN]; \
             CIV_RIOOL_BIJWERKEN [RIOOL=] [PUTNR= BOB=|MAAIVELD=|PUT=] [LEIDING=P101-P102 DIAMETER=|MATERIAAL=|VERHANG=|DEKKING=] [INVOEGEN=x,y]; \
             CIV_RIOOL_PROFIEL [RIOOL=] [VAN=] [NAAR=] [PLAATS=x,y]; CIV_RIOOL_TOETS [RIOOL=] [Q=l/s | OPPERVLAK=m² of ha [REGEN=l/s/ha] | INWONERS=] [PUTAFSTAND=] [VRIJ=] [RUWHEID=mm]; CIV_RIOOL_HYDRAULICA [RIOOL=] [OPPERVLAK=m² of ha] [REGEN=l/s/ha] [INWONERS= | WONINGEN=] [UITLAATPEIL=m] [DIAMETER=AUTO] [VULLING=h/D] [PUTVERLIES=ξ]; \
             CIV_RIOOL_EXPORT [RIOOL=] [FORMAAT=GWSW|CSV|HYDX] [PAD=]; CIV_RIOOL_IMPORT <pad.orox.ttl>; CIV_RIOOL_GWSW [NAAM=].",
        ),
        // Design criteria, sight distance and rail (as-spoor).
        "CIV_ONTWERPCRITERIA" => {
            let o = opt(&["RICHTLIJN", "CATEGORIE", "V", "AS"])?;
            let _ = o;
            let (choice, h) = (criteria_choice(args)?, args.handle("AS")?);
            job(host, tr("Ontwerpcriteria", "Design criteria"), id, move |_| civiel::ontwerpcriteria::criteria(choice, h));
        }
        "CIV_AS_TOETS" => {
            let o = opt(&["AS", "RICHTLIJN", "CATEGORIE", "V", "VERKANTING"])?;
            let (choice, h, e) = (criteria_choice(args)?, args.handle("AS")?, args.f64("VERKANTING")?);
            job(host, tr("As toetsen", "Check alignment"), id, move |_| civiel::ontwerpcriteria::check(&o, h, choice, e));
        }
        "CIV_ZICHT" => {
            let o = opt(&["AS", "RICHTLIJN", "CATEGORIE", "V", "NODIG", "OOG", "OBJECT", "RIJLIJN", "VRIJ", "MAX", "RICHTING", "LAGEN", "PLAATS"])?;
            let (fw, bw) = match args.str("RICHTING").map(|s| s.to_ascii_lowercase()) {
                None => (true, false),
                Some(r) if r == "heen" || r == "op" || r == "forward" => (true, false),
                Some(r) if r == "terug" || r == "af" || r == "back" => (false, true),
                Some(r) if r == "beide" || r == "both" => (true, true),
                Some(_) => return Err(tr("RICHTING: heen, terug of beide", "RICHTING: heen (along), terug (against) or beide (both)").into()),
            };
            let layers: Vec<String> = match args.str("LAGEN") {
                Some(l) => l.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                None => vec!["-OG-BEBOUWING".to_string()],
            };
            let req = civiel::zicht::SightRequest {
                axis: args.handle("AS")?,
                choice: criteria_choice(args)?,
                need: args.f64("NODIG")?,
                eye: args.f64("OOG")?,
                object: args.f64("OBJECT")?,
                every: args.f64("OM")?.unwrap_or(10.0),
                lane: args.f64("RIJLIJN")?.unwrap_or(1.75),
                clear: args.f64("VRIJ")?.unwrap_or(5.0),
                max: args.f64("MAX")?,
                forward: fw,
                backward: bw,
                layers,
                at: place_arg(args)?,
            };
            job(host, tr("Zichtafstand", "Sight distance"), id, move |p| civiel::zicht::sight(&o, req, p));
        }
        "CIV_SPOOR" => {
            let o = opt(&["AS", "PI", "R", "LS", "A", "NAAM", "BEGIN", "V", "VMIN", "VERKANTING", "SPOREN", "SPOORAFSTAND", "PLAATS"])?;
            let input = layout_input(args, unit)?;
            let v = args.f64("V")?.unwrap_or(100.0);
            let req = civiel::spoor::RailRequest {
                input,
                name: args.str("NAAM").map(str::to_string),
                start: args.f64("BEGIN")?,
                v,
                vmin: args.f64("VMIN")?,
                cant: args.list("VERKANTING")?.unwrap_or_default(),
                tracks: args.f64("SPOREN")?.map(|n| n.clamp(1.0, 8.0) as usize).unwrap_or(1),
                distance: args.f64("SPOORAFSTAND")?,
                diagram: place_arg(args)?,
            };
            job(host, tr("Spooras", "Track"), id, move |_| civiel::spoor::track(&o, req));
        }
        "CIV_WISSEL" if scripted => {
            let o = opt(&["AS", "STATION", "TYPE", "ZIJDE", "RICHTING", "LENGTE", "NAAM"])?;
            let st = args.f64("STATION")?.ok_or_else(|| tr("geef STATION=<m> van het wisselbegin", "give STATION=<m> of the turnout start").to_string())?;
            let kind = args.str("TYPE").unwrap_or("1:9").to_string();
            let left = !matches!(args.str("ZIJDE").map(|s| s.to_ascii_lowercase()).as_deref(), Some("rechts" | "right" | "r"));
            let forward = !matches!(args.str("RICHTING").map(|s| s.to_ascii_lowercase()).as_deref(), Some("af" | "terug" | "back"));
            let (h, extra, name) = (args.handle("AS")?, args.f64("LENGTE")?.unwrap_or(50.0), args.str("NAAM").map(str::to_string));
            job(host, tr("Wissel", "Turnout"), id, move |_| civiel::spoor::turnout(&o, h, st, &kind, left, forward, extra, name));
        }
        "CIV_WISSEL" => {
            let sel = host.selection();
            let Some(axis) = selected_axis(host) else {
                return Err(tr("selecteer eerst één spooras en kies dan Wissel, of geef AS= STATION= TYPE= ZIJDE=", "select one track first, then Turnout, or give AS= STATION= TYPE= ZIJDE=").into());
            };
            host.start_interactive(Box::new(TurnoutCmd { axis, handle: sel[0], f: unit }));
        }
        "CIV_INFO" => {
            opt(&[])?;
            job(host, "Modelinfo", id, |_| civiel::info());
        }
        "CIV_INSTELLINGEN" => open_settings(host),
        "CIV_HELP" => {
            host.push_info(open_infra::lang::tr(
                "[Open Infra — Civiel] Puntenwolk: CIV_TIN BRON=PUNTENWOLK [BESTAND=las/laz/e57/ply/pts/xyz] [EDITIE=AHN5|AHN4] [GEBIED=x,y;x,y] [KLASSEN=2;9|AUTO] [CEL=0.5] [TOL=0.10] [MAX=200000] [GAT=2] [HELLING=0.3] [VENSTER=18]; \
                 CIV_LENGTEPROFIEL / CIV_DWARSPROFIELEN AS= BRON=PUNTENWOLK [STROOK=1] [BESTAND=] [KLASSEN=]; CIV_BIJWERKEN [CONTROLE] (profielen van een gewijzigde as of maaiveld opnieuw); \
                 CIV_MESH [GEBIED=x,y;x,y] [BESTAND=] [ZMIN=] [ZMAX=] [KLASSEN=] [VOXEL=] [GAT=1] [MAX=400000] [SOORT=kunstwerk|brug|viaduct|tunnel|sluis|steiger|pijler|landhoofd|kademuur] [ZIJDE=BOVEN|MIDDEN|x,y,z] [NAAM=].",
                "[Open Infra — Civil] Point clouds: CIV_TIN BRON=PUNTENWOLK [BESTAND=las/laz/e57/ply/pts/xyz] [EDITIE=AHN5|AHN4] [GEBIED=x,y;x,y] [KLASSEN=2;9|AUTO] [CEL=0.5] [TOL=0.10] [MAX=200000] [GAT=2] [HELLING=0.3] [VENSTER=18]; \
                 CIV_LENGTEPROFIEL / CIV_DWARSPROFIELEN AS= BRON=PUNTENWOLK [STROOK=1] [BESTAND=] [KLASSEN=]; \
                 CIV_MESH [GEBIED=x,y;x,y] [BESTAND=] [ZMIN=] [ZMAX=] [KLASSEN=] [VOXEL=] [GAT=1] [MAX=400000] [SOORT=kunstwerk|brug|viaduct|tunnel|sluis|steiger|pijler|landhoofd|kademuur] [ZIJDE=BOVEN|MIDDEN|x,y,z] [NAAM=].",
            ));
            host.push_info(
            "[Open Infra — Civiel] Lint: selectie en klikken; scripts: argumenten SLEUTEL=waarde, coördinaten x,y in het projectstelsel, ID=<naam> schrijft <config>/api/<naam>.json. \
             CIV_PUNTEN_IMPORT <pad> [FORMAAT=], CIV_PUNT x,y[,z] [NR=] [CODE=], CIV_PUNTEN_EXPORT [PAD=], CIV_LANDXML <pad>; \
             CIV_TIN [BRON=AHN|PUNTEN|SELECTIE] [LAAG=] [STATUS=B|N] [AFSTAND=] [MAXRIBBE=]; CIV_HOOGTELIJNEN / CIV_ANALYSE [OPPERVLAK=B|N]; CIV_VOLUME [ONTWERP=PEIL] [PEIL=] [CEL=]; \
             CIV_AS AS=<handle> | PI=x,y;x,y;… R=r;… [LS=l;…|A=a;…] [NAAM=] [BEGIN=]; CIV_AS_OFFSET AS= VERSCHUIVING=-3.5;3.5; CIV_STATION AS= x,y; \
             CIV_LENGTEPROFIEL AS= [PLAATS=x,y]; CIV_ONTWERPPROFIEL AS= PVI=station:hoogte;… [BOOG=]; CIV_DWARSPROFIELEN AS= [PLAATS=] [OM=] [LINKS=] [RECHTS=]; \
             3D: CIV_BAG3D [LOD=1.2|1.3|2.2] [MAX=] [STIJL=mooi|maquette|realistisch|nlcs] [RAMEN=nee], CIV_FOTOTERREIN [CEL=2|10] [FOTO=auto|pdok|usgs|sentinel2|sentinel2-<jaar>] (wereldwijd: AHN of Terrain Tiles met luchtfoto), CIV_BGT3D [GROEPEN=wegen;terrein;water;gebouwen;kunstwerken;scheidingen;spoor;inrichting;vegetatie] [PANDEN=nee], CIV_3DMODEL (alles), CIV_WATERPEIL PEIL=<m NAP>, CIV_CITYGML <pad> [STIJL=], CIV_KLIC3D [<levering>] [VERGROOT=], CIV_DOORZICHT [PCT=70] [ALLES] [UIT], CIV_SONDERINGEN3D [BRON=sonderingen|boringen|alles] [MAX=100] [STRAAL=0.4] [MINLAAG=0.2] [QC=nee] [LABELS=nee] (BRO: kolommen met grondsoorten); \
             CIV_CORRIDOR AS= [PROFIELTYPE=<naam>|<pad.json>] [NIVEAU=|BOVEN=] [VAN=] [TOT=] [DOELEN=naam:handle;…] [RIJSTROOK=] [BERM=] [HELLING=] [TALUD=] [INGRAVING=] [KRUIN=] [VERKANTING=] [FREQUENTIE=] [DWARSPROFIELEN=ja|x,y] [KLEUR=nlcs]; \
             CIV_PROFIELTYPEN, CIV_PROFIELTYPE NAAM= [KOPIE=] [PLAATS=x,y]; CIV_KRUISPUNT HOOFD= ZIJ= [R=8;…] [PROFIELTYPE=] [NIVEAU=]; \
             CIV_ROTONDE ASSEN=h;h [MIDDEN=x,y] [TYPE=binnen|buiten] [D=] [RIJBAAN=] [OVERRIJDBAAR=] [IN=] [UIT=] [INBREEDTE=] [UITBREEDTE=] [MIDDENGELEIDER=] [LENGTE=] [HELLING=] [PROFIELTYPE=] [NIVEAU=]; CIV_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee] [IDS=<pad.ids>] (IFC 4x3); CIV_LANDXML_EXPORT [BESTAND=<pad>] [ALLES]; CIV_INFO; \
             uitwisseling: CIV_IFC_IMPORT <pad.ifc> [STATUS=B|N] (assen, terrein, elementen), CIV_IFC_IDS IDS=<pad.ids> [IFC=<pad.ifc>], CIV_3D_EXPORT [FORMAAT=GLB|OBJ|STL] [SELECTIE] [PAD=], CIV_3D_IMPORT <pad.obj|stl|glb> [PLAATS=x,y,z] [SCHAAL=] [ROTATIE=graden] [LAAG=]; \
             codes: CIV_PUNTEN_IMPORT <pad> CODES=JA [CODETABEL=], CIV_CODES_TOEPASSEN [CODETABEL=]; percelen: CIV_PERCELEN KAVEL= [STRAAT=|RICHTING=graden] N=|OPP=|FRONT= [REST=laatste|apart|verdelen] [PREFIX=P] [START=1], CIV_PERCEEL_INFO [PERCEEL=h;h] [STRAAT=]; \
             water: CIV_WATERDRUPPEL x,y | PUNTEN=x,y;… [CEL=1], CIV_STROOMGEBIED PUNTEN=x,y;… | LAAG=*KOLK* [BRON=TIN|AHN] [CEL=1] [SNAP=2] [PLASSEN=ja] [MINDIEPTE=0.05] [MINOPP=2], CIV_PLASSEN [BRON=] [CEL=] [MINDIEPTE=] [MINOPP=]; \
             CIV_ONTWERPCRITERIA [RICHTLIJN=roa|noa|crow|aashto CATEGORIE= V=] [AS=]; CIV_AS_TOETS AS= [RICHTLIJN= CATEGORIE= V=] [VERKANTING=%]; \
             CIV_ZICHT AS= [RICHTLIJN= CATEGORIE= V=|NODIG=] [OOG=] [OBJECT=] [OM=10] [RIJLIJN=1.75] [VRIJ=5] [MAX=] [RICHTING=heen|terug|beide] [LAGEN=-OG-BEBOUWING;…] [PLAATS=]; \
             CIV_SPOOR AS=|PI= R= [LS=|A=] V= [VMIN=] [VERKANTING=mm;…] [SPOREN=] [SPOORAFSTAND=] [NAAM=] [PLAATS=]; CIV_WISSEL AS= STATION= [TYPE=1:9|1:12|1:15|…] [ZIJDE=links|rechts] [RICHTING=op|af] [LENGTE=].",
            );
        }
        _ => return Err(format!("onbekend commando {verb}; zie CIV_HELP")),
    }
    Ok(())
}

ocs_plugin_api::export_plugin!(OpenInfraCiviel);
