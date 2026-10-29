//! Open Infra — Civiel: the second ribbon tab, with Civil 3D-like tools after
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
// Grading: feature lines, plateaus, slopes, TIN volumes, mass haul.
use open_infra::civiel::{grading, massa};
use open_infra::lang::tr;

static MANIFEST: PluginManifest = PluginManifest {
    id: "openaec.open-infra-civiel",
    name: "Open Infra — Civiel",
    version: env!("CARGO_PKG_VERSION"),
    description: "Civil 3D-achtige tools: meetpunten, terrein (TIN), assen met stationering, lengte-, ontwerp- en dwarsprofielen, grondverzet. Op NLCS-lagen.",
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
// Grading.
const I_FEATURE: IconKind = icon!("kantlijn.svg");
const I_PLATEAU: IconKind = icon!("plateau.svg");
const I_SLOPE: IconKind = icon!("talud.svg");
const I_VOLUME_TIN: IconKind = icon!("grondverzet-tin.svg");
const I_MASS_HAUL: IconKind = icon!("grondbalans.svg");

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
            vec![
                // Civil 3D "Points" / Road "GeoPoints" and "Input-Output".
                RibbonGroup {
                    title: "Punten",
                    tools: vec![
                        file_tool("CIV_PUNTEN_IMPORT", "Punten importeren", I_POINTS, "Puntenbestand openen", "Puntenbestand (PNEZD, PENZD, CSV)", &["csv", "txt", "xyz", "pnt", "asc"], true),
                        small("CIV_PUNT", "Punt plaatsen", I_PLACE),
                        small("CIV_PUNTEN_EXPORT", "Punten exporteren", I_EXPORT),
                        file_tool("CIV_LANDXML", "LandXML", I_LANDXML, "LandXML openen", "LandXML", &["xml", "landxml"], false),
                    ],
                },
                // Civil 3D "Surfaces".
                RibbonGroup {
                    title: "Oppervlak",
                    tools: vec![
                        large("CIV_TIN", "TIN maken", I_TIN),
                        small("CIV_HOOGTELIJNEN", "Hoogtelijnen", I_CONTOURS),
                        small("CIV_ANALYSE", "Hoogteanalyse", I_ANALYSIS),
                        small("CIV_VOLUME", "Grondverzet", I_VOLUME),
                    ],
                },
                // Civil 3D "Alignments".
                RibbonGroup {
                    title: "As",
                    tools: vec![
                        large("CIV_AS", "As maken", I_AXIS),
                        small("CIV_AS_OFFSET", "As verschuiven", I_OFFSET),
                        small("CIV_STATION", "Station / offset", I_STATION),
                    ],
                },
                // Civil 3D "Profiles" and "Sections".
                RibbonGroup {
                    title: "Profielen",
                    tools: vec![
                        large("CIV_LENGTEPROFIEL", "Lengteprofiel", I_PROFILE),
                        small("CIV_ONTWERPPROFIEL", "Ontwerpprofiel", I_DESIGN),
                        small("CIV_DWARSPROFIELEN", "Dwarsprofielen", I_SECTIONS),
                    ],
                },
                // The 3D city and landscape model.
                RibbonGroup {
                    title: "3D-model",
                    tools: vec![
                        large("CIV_BAG3D", "3D BAG", I_BAG3D),
                        small("CIV_FOTOTERREIN", "Terrein + luchtfoto", I_PHOTO),
                        small("CIV_BGT3D", "BGT naar 3D", I_BGT3D),
                        small("CIV_WATERPEIL", "Waterpeil", I_WATER),
                    ],
                },
                // Civil 3D "Corridors": assembly along axis and design profile.
                RibbonGroup { title: "Corridor", tools: vec![large("CIV_CORRIDOR", "Corridor", I_CORRIDOR)] },
                // Grading: feature lines, plateaus and slopes, exact earthwork, mass haul.
                RibbonGroup {
                    title: tr("Grondwerk", "Grading"),
                    tools: vec![
                        large("CIV_KANTLIJN", tr("Kantlijn", "Feature line"), I_FEATURE),
                        small("CIV_PLATEAU", tr("Plateau", "Plateau"), I_PLATEAU),
                        small("CIV_TALUD", tr("Talud", "Slope"), I_SLOPE),
                        small("CIV_KANTLIJN_PUNT", tr("Hoogtepunt", "Height point"), I_PLACE),
                        small("CIV_VOLUME_TIN", tr("Grondverzet TIN", "Volume TIN"), I_VOLUME_TIN),
                        small("CIV_GRONDBALANS", tr("Grondbalans", "Mass haul"), I_MASS_HAUL),
                    ],
                },
                RibbonGroup {
                    title: "Instellingen",
                    tools: vec![large("CIV_INSTELLINGEN", "Instellingen", I_SETTINGS), small("CIV_INFO", "Modelinfo", I_INFO)],
                },
            ]
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
    // Grading commands.
    if run_grading(host, verb, args, id.clone())? {
        return Ok(());
    }
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
            let o = opt(&["LOD", "MAX", "KLEUR", "STIJL"])?;
            let real = bag_style(args)?;
            let lod = match args.str("LOD") {
                Some(l) => civiel_lod(l)?,
                None => open_infra::gis::bag3d::Lod::L22,
            };
            let max = args.f64("MAX")?.map(|m| m as usize).unwrap_or(25_000);
            job(host, "3D BAG", id, move |p| civiel::model3d::bag3d(&o, lod, max, real, p));
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
            let o = opt(&["LOD", "MAX", "KLEUR", "STIJL"])?;
            let real = bag_style(args)?;
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
                parts.push(civiel::model3d::bag3d(&o, lod, max, real, p)?);
                Ok(parts.join(" | "))
            });
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
             3D: CIV_BAG3D [LOD=1.2|1.3|2.2] [MAX=] [STIJL=mooi|maquette|realistisch|nlcs], CIV_FOTOTERREIN [CEL=2], CIV_BGT3D [GROEPEN=wegen;terrein;water;gebouwen;kunstwerken;scheidingen;spoor;inrichting;vegetatie] [PANDEN=nee], CIV_3DMODEL (alles), CIV_WATERPEIL PEIL=<m NAP>; \
             CIV_CORRIDOR AS= [SJABLOON=weg|fietspad|snelweg|dijk] [NIVEAU=|BOVEN=] [RIJSTROOK=] [BERM=] [HELLING=] [TALUD=] [INGRAVING=] [KRUIN=] [VERKANTING=] [FREQUENTIE=] [DWARSPROFIELEN=ja|x,y]; CIV_INFO.",
        ),
        _ => return Err(format!("onbekend commando {verb}; zie CIV_HELP")),
    }
    Ok(())
}

ocs_plugin_api::export_plugin!(OpenInfraCiviel);
