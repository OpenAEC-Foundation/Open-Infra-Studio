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
use open_infra::civiel::{self, as_profiel, corridor, landmeten, landxml, terrein};
use open_infra::{bridge, ui};

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
const I_CITYGML: IconKind = icon!("citygml.svg");
const I_IFC: IconKind = icon!("ifc.svg");
const I_KLIC3D: IconKind = icon!("klic3d.svg");
const I_SEE: IconKind = icon!("doorzicht.svg");
const I_CPT: IconKind = icon!("sonderingen.svg");

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
            use open_infra::lang::{dutch, tr};
            let mut groups = vec![
                // Points (Road "GeoPoints" and "Input-Output").
                RibbonGroup {
                    title: tr("Punten", "Points"),
                    tools: vec![
                        file_tool("CIV_PUNTEN_IMPORT", tr("Punten importeren", "Import points"), I_POINTS, tr("Puntenbestand openen", "Open a point file"), tr("Puntenbestand (PNEZD, PENZD, CSV)", "Point file (PNEZD, PENZD, CSV)"), &["csv", "txt", "xyz", "pnt", "asc"], true),
                        small("CIV_PUNT", tr("Punt plaatsen", "Place point"), I_PLACE),
                        small("CIV_PUNTEN_EXPORT", tr("Punten exporteren", "Export points"), I_EXPORT),
                        file_tool("CIV_LANDXML", "LandXML", I_LANDXML, tr("LandXML openen", "Open LandXML"), "LandXML", &["xml", "landxml"], false),
                    ],
                },
                // Surfaces.
                RibbonGroup {
                    title: tr("Oppervlak", "Surface"),
                    tools: vec![
                        large("CIV_TIN", tr("TIN maken", "Create TIN"), I_TIN),
                        small("CIV_HOOGTELIJNEN", tr("Hoogtelijnen", "Contours"), I_CONTOURS),
                        small("CIV_ANALYSE", tr("Hoogteanalyse", "Elevation analysis"), I_ANALYSIS),
                        small("CIV_VOLUME", tr("Grondverzet", "Earthworks"), I_VOLUME),
                    ],
                },
                // Alignments.
                RibbonGroup {
                    title: tr("As", "Alignment"),
                    tools: vec![
                        large("CIV_AS", tr("As maken", "Create alignment"), I_AXIS),
                        small("CIV_AS_OFFSET", tr("As verschuiven", "Offset alignment"), I_OFFSET),
                        small("CIV_STATION", tr("Station / offset", "Station / offset"), I_STATION),
                    ],
                },
                // Profiles and sections.
                RibbonGroup {
                    title: tr("Profielen", "Profiles"),
                    tools: vec![
                        large("CIV_LENGTEPROFIEL", tr("Lengteprofiel", "Length profile"), I_PROFILE),
                        small("CIV_ONTWERPPROFIEL", tr("Ontwerpprofiel", "Design profile"), I_DESIGN),
                        small("CIV_DWARSPROFIELEN", tr("Dwarsprofielen", "Cross sections"), I_SECTIONS),
                    ],
                },
                // Corridors: assembly along axis and design profile.
                RibbonGroup { title: "Corridor", tools: vec![large("CIV_CORRIDOR", "Corridor", I_CORRIDOR)] },
            ];
            // The 3D city and landscape model: open formats for everyone, the
            // Dutch registers (3D BAG, AHN, BGT, KLIC, BRO) in one menu.
            let model = vec![
                file_tool("CIV_CITYGML", "CityGML/JSON", I_CITYGML, tr("CityGML of CityJSON openen", "Open CityGML or CityJSON"), "CityGML / CityJSON", &["gml", "xml", "citygml", "json", "jsonl"], true),
                small("CIV_DOORZICHT", tr("Maaiveld doorzichtig", "See-through ground"), I_SEE),
                small("CIV_WATERPEIL", tr("Waterpeil", "Water level"), I_WATER),
                small("CIV_IFC", "IFC 4x3", I_IFC),
            ];
            groups.push(RibbonGroup { title: tr("3D-model", "3D model"), tools: model });
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
                        ("CIV_FOTOTERREIN", tr("Terrein + luchtfoto", "Terrain + aerial photo"), I_PHOTO),
                        ("CIV_BGT3D", tr("BGT naar 3D", "BGT to 3D"), I_BGT3D),
                        ("CIV_KLIC3D", tr("KLIC in 3D", "KLIC in 3D"), I_KLIC3D),
                        ("CIV_SONDERINGEN3D", tr("Sonderingen 3D (BRO)", "CPTs in 3D (BRO)"), I_CPT),
                    ],
                    default: "CIV_BAG3D",
                    }],
                });
            }
            groups.push(RibbonGroup {
                title: tr("Instellingen", "Settings"),
                tools: vec![large("CIV_INSTELLINGEN", tr("Instellingen", "Settings"), I_SETTINGS), small("CIV_INFO", tr("Modelinfo", "Model info"), I_INFO)],
            });
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

/// One command: from the ribbon (no arguments: selection and clicks) or
/// from a script (arguments; see CIV_HELP).
fn run(host: &mut dyn HostApi, verb: &str, args: &Args, id: Option<String>) -> Result<(), String> {
    let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
    let scripted = !args.is_empty();
    let base = civiel::settings();
    let opt = |own: &[&str]| args.apply(&base, own);
    let path = || args.words.iter().find(|w| !w.contains(',') || std::path::Path::new(w).exists()).cloned();
    match verb {
        "CIV_PUNTEN_IMPORT" | "CIV_LANDXML" => {
            let Some(file) = path() else {
                return Err("geef een bestand: CIV_PUNTEN_IMPORT <pad> (of kies via de knop in het lint)".into());
            };
            let o = opt(&[])?;
            if verb == "CIV_LANDXML" {
                job(host, "LandXML", id, move |p| landxml::import(std::path::Path::new(&file), &o, p));
            } else {
                job(host, "Punten importeren", id, move |_| landmeten::import(std::path::Path::new(&file), &o));
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
            let o = opt(&["BRON", "LAAG"])?;
            let source = match args.str("BRON").map(|s| s.to_ascii_uppercase()) {
                None | Some(_) if args.str("LAAG").is_some() => terrein::TinSource::Layer(args.str("LAAG").unwrap().to_string()),
                None => terrein::TinSource::Auto,
                Some(b) => match b.as_str() {
                    "AHN" => terrein::TinSource::Ahn,
                    "SELECTIE" => terrein::TinSource::Selection,
                    "PUNTEN" => terrein::TinSource::Points,
                    "AUTO" => terrein::TinSource::Auto,
                    _ => return Err("BRON: AHN, SELECTIE, PUNTEN of AUTO (of LAAG=<laagnaam>)".into()),
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
            let o = opt(&["AS", "PI", "R", "NAAM", "BEGIN"])?;
            let input = match args.points("PI")? {
                Some(pis) => {
                    let pis: Vec<[f64; 2]> = pis.into_iter().map(to_drawing).collect::<Result<_, _>>()?;
                    let radii: Vec<f64> = args.list("R")?.unwrap_or_default().into_iter().map(|r| r * unit).collect();
                    as_profiel::AxisInput::Pis(pis, radii)
                }
                None => as_profiel::AxisInput::Line(args.handle("AS")?),
            };
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
            let o = opt(&["AS", "PLAATS"])?;
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
            let mut own = vec!["AS", "NIVEAU", "BOVEN", "DWARSPROFIELEN"];
            own.extend(corridor::Assembly::ARGS);
            let o = opt(&own)?;
            let asm = corridor::Assembly::template(&o.assembly)
                .ok_or_else(|| format!("onbekend sjabloon {}; kies {}", o.assembly, corridor::Assembly::NAMES.join(", ")))?
                .with_args(args)?;
            let grade = match (args.f64("NIVEAU")?, args.f64("BOVEN")?) {
                (Some(z), _) => Some(corridor::Grade::Level(z)),
                (None, Some(dz)) => Some(corridor::Grade::AboveGround(dz)),
                _ => None,
            };
            let views = match args.str("DWARSPROFIELEN") {
                Some(v) if v.contains(',') => place_arg(&Args::parse(&format!("PLAATS={v}")))?,
                Some(v) if matches!(v.to_ascii_lowercase().as_str(), "ja" | "1" | "true") => Some([f64::NAN, f64::NAN]),
                None if o.corridor_views => Some([f64::NAN, f64::NAN]),
                _ => None,
            };
            let axis_h = args.handle("AS")?;
            if axis_h.is_none() && !scripted && selected_axis(host).is_none() {
                return Err("selecteer eerst de as (met ontwerpprofiel) of geef AS=<handle>".into());
            }
            let every = o.corridor_every;
            job(host, "Corridor", id, move |p| {
                // Views without a place (NaN): the corridor finds one.
                corridor::corridor(&o, corridor::Request { axis: axis_h, assembly: asm, grade, every, views }, p)
            });
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
            let o = opt(&[])?;
            let cell = args.f64("CEL")?.unwrap_or(2.0);
            job(host, "Terrein met luchtfoto", id, move |p| civiel::model3d::photo_terrain(&o, cell, p));
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
            let cell = args.f64("CEL")?.unwrap_or(2.0);
            job(host, "3D-model", id, move |p| {
                let mut parts = vec![civiel::model3d::photo_terrain(&o, cell, p)?];
                let groups: Vec<String> = civiel::model3d::default_groups().into_iter().map(str::to_string).collect();
                parts.push(civiel::model3d::bgt3d(&o, &groups, false, real != civiel::model3d::BagStyle::Nlcs, p)?);
                parts.push(civiel::model3d::bag3d(&o, lod, max, real, windows, p)?);
                Ok(parts.join(" | "))
            });
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
            opt(&["BESTAND", "PAD", "TEKST"])?;
            let (file, selection, texts) = open_infra::ifc::args(args);
            job(host, "IFC 4x3", id, move |p| open_infra::ifc::command(file, selection, texts, p));
        }
        "CIV_INFO" => {
            opt(&[])?;
            job(host, "Modelinfo", id, |_| civiel::info());
        }
        "CIV_INSTELLINGEN" => open_settings(host),
        "CIV_HELP" => host.push_info(
            "[Open Infra — Civiel] Lint: selectie en klikken; scripts: argumenten SLEUTEL=waarde, coördinaten x,y in het projectstelsel, ID=<naam> schrijft <config>/api/<naam>.json. \
             CIV_PUNTEN_IMPORT <pad> [FORMAAT=], CIV_PUNT x,y[,z] [NR=] [CODE=], CIV_PUNTEN_EXPORT [PAD=], CIV_LANDXML <pad>; \
             CIV_TIN [BRON=AHN|PUNTEN|SELECTIE] [LAAG=] [STATUS=B|N] [AFSTAND=] [MAXRIBBE=]; CIV_HOOGTELIJNEN / CIV_ANALYSE [OPPERVLAK=B|N]; CIV_VOLUME [ONTWERP=PEIL] [PEIL=] [CEL=]; \
             CIV_AS AS=<handle> | PI=x,y;x,y;… R=r;… [NAAM=] [BEGIN=]; CIV_AS_OFFSET AS= VERSCHUIVING=-3.5;3.5; CIV_STATION AS= x,y; \
             CIV_LENGTEPROFIEL AS= [PLAATS=x,y]; CIV_ONTWERPPROFIEL AS= PVI=station:hoogte;… [BOOG=]; CIV_DWARSPROFIELEN AS= [PLAATS=] [OM=] [LINKS=] [RECHTS=]; \
             3D: CIV_BAG3D [LOD=1.2|1.3|2.2] [MAX=] [STIJL=mooi|maquette|realistisch|nlcs] [RAMEN=nee], CIV_FOTOTERREIN [CEL=2], CIV_BGT3D [GROEPEN=wegen;terrein;water;gebouwen;kunstwerken;scheidingen;spoor;inrichting;vegetatie] [PANDEN=nee], CIV_3DMODEL (alles), CIV_WATERPEIL PEIL=<m NAP>, CIV_CITYGML <pad> [STIJL=], CIV_KLIC3D [<levering>] [VERGROOT=], CIV_DOORZICHT [PCT=70] [ALLES] [UIT], CIV_SONDERINGEN3D [BRON=sonderingen|boringen|alles] [MAX=100] [STRAAL=0.4] [MINLAAG=0.2] [QC=nee] [LABELS=nee] (BRO: kolommen met grondsoorten); \
             CIV_CORRIDOR AS= [SJABLOON=weg|fietspad|snelweg|dijk] [NIVEAU=|BOVEN=] [RIJSTROOK=] [BERM=] [HELLING=] [TALUD=] [INGRAVING=] [KRUIN=] [VERKANTING=] [FREQUENTIE=] [DWARSPROFIELEN=ja|x,y]; CIV_IFC [BESTAND=<pad>] [SELECTIE] [TEKST=nee] (IFC 4x3); CIV_INFO.",
        ),
        _ => return Err(format!("onbekend commando {verb}; zie CIV_HELP")),
    }
    Ok(())
}

ocs_plugin_api::export_plugin!(OpenInfraCiviel);
