//! Workbench window, laid out like the dialogs of the FreeCAD GIS(2BIM)
//! workbench: a map with the location and bounding box on top, then
//! "Locatie / Begrenzing", "Webserver / Verzoek" and "Import-instellingen"
//! groups, and OK / Test / Annuleren at the bottom.

use std::sync::mpsc::Receiver;
use std::sync::Arc;

#[path = "services.rs"]
mod services;
#[path = "localisation.rs"]
mod localisation;

use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};

use super::jobs::{poll, Job};
use super::map::{LonLat, MapEvent, MapView, Overlay};
use super::{Product, Tab, UiMsg};
use crate::bridge;
use crate::lang::{tr, tr_format};
use crate::gis::caps::{self, WfsCaps, WfsType, WmsCaps, WmsLayer};
use crate::gis::crs::{self, CrsInfo, Transformer};
use crate::gis::geocode::{self, Hit};
use crate::gis::osm::THEMES;
use crate::gis::raster::{self, Preview, MAX_TOTAL_PX};
use crate::gis::servers::{self, MapServer, ServerKind};
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature};
use crate::gis::wfs::WfsResult;
use crate::basisdata::{self, Cadastre, KlicOptions, NlcsOptions};
use crate::gis::{bgt, klic};
use crate::import::{self, VectorStyle, PDOK};
use crate::nlcs::{self, Element, Status};
use crate::tekenblad::{self, SheetInfo};
use crate::gis::ahn::{self, Model};
use crate::gis::pointcloud::{self, Colouring, Edition};
use crate::hoogte::{self, AhnOptions};
use crate::hoeveelheden::{self, Quantities};
use crate::nlcs_check::{self, Report, Rule, Severity};
use crate::civiel::{self, landmeten::Format, CivielOptions};

const ACI_CHOICES: &[(i16, &str, &str)] = &[
    (1, "rood", "red"),
    (2, "geel", "yellow"),
    (3, "groen", "green"),
    (4, "cyaan", "cyan"),
    (5, "blauw", "blue"),
    (6, "magenta", "magenta"),
    (7, "wit/zwart", "white/black"),
    (8, "grijs", "grey"),
    (30, "oranje", "orange"),
];
const ACCENT: Color32 = Color32::from_rgb(33, 150, 220);
const OK_GREEN: Color32 = Color32::from_rgb(152, 195, 121);
const ERR_RED: Color32 = Color32::from_rgb(224, 108, 117);
const WARN: Color32 = Color32::from_rgb(229, 192, 123);
/// Map height on the import pages; the location page gets most of the window,
/// like the map pane of FreeCAD's "Set Geographic Location" dialog.
const MAP_H: f32 = 360.0;

fn aci_color(aci: i16) -> Color32 {
    match aci {
        1 => Color32::from_rgb(230, 60, 60),
        2 => Color32::from_rgb(235, 200, 40),
        3 => Color32::from_rgb(40, 180, 70),
        4 => Color32::from_rgb(40, 190, 210),
        5 => Color32::from_rgb(60, 100, 230),
        6 => Color32::from_rgb(210, 70, 200),
        8 => Color32::from_gray(110),
        30 => Color32::from_rgb(240, 140, 40),
        _ => Color32::from_gray(60),
    }
}

/// Toolbar-style icons from the plugin's SVGs, recoloured for the dark theme.
fn icon(name: &str) -> egui::Image<'static> {
    let svg: &str = match name {
        "location" => include_str!("../../assets/location.svg"),
        "map" => include_str!("../../assets/map.svg"),
        "vector" => include_str!("../../assets/vector.svg"),
        "globe" => include_str!("../../assets/globe.svg"),
        "pdok" => include_str!("../../assets/pdok.svg"),
        "point" => include_str!("../../assets/point.svg"),
        "server" => include_str!("../../assets/server.svg"),
        "area" => include_str!("../../assets/area.svg"),
        "nlcs" => include_str!("../../assets/bgt.svg"),
        "ahn" => include_str!("../../assets/ahn.svg"),
        "vehicle" => include_str!("../../assets/vehicle.svg"),
        "file" => include_str!("../../assets/file.svg"),
        "check" => include_str!("../../assets/nlcs-controle.svg"),
        "quantities" => include_str!("../../assets/hoeveelheden.svg"),
        "axis" => include_str!("../../assets/as.svg"),
        "services" => include_str!("../../assets/services.svg"),
        _ => include_str!("../../assets/list.svg"),
    };
    let bytes = svg.replace("currentColor", "#dfe5ea").into_bytes();
    egui::Image::from_bytes(format!("bytes://open-infra/{name}.svg"), bytes).fit_to_exact_size(Vec2::splat(18.0))
}

/// The import area as a lon/lat ring (edges sampled, so it follows the
/// projection's curvature on the web map).
fn site_ring(site: &Site) -> Option<Vec<LonLat>> {
    let t = Transformer::new(&site.crs, "EPSG:4326").ok()?;
    let b = site.bbox();
    let mut ring = Vec::with_capacity(33);
    const N: usize = 8;
    let edges = [([b[0], b[3]], [b[2], b[3]]), ([b[2], b[3]], [b[2], b[1]]), ([b[2], b[1]], [b[0], b[1]]), ([b[0], b[1]], [b[0], b[3]])];
    for (a, c) in edges {
        for i in 0..N {
            let f = i as f64 / N as f64;
            ring.push(t.apply([a[0] + (c[0] - a[0]) * f, a[1] + (c[1] - a[1]) * f]).ok()?);
        }
    }
    Some(ring)
}

// ── Page states ─────────────────────────────────────────────────────────────

#[derive(Default)]
struct LocationPage {
    query: String,
    /// Suggestions (while typing) or full search results, for `hits_for`.
    hits: Vec<Hit>,
    hits_for: String,
    /// The hits come from the full search (PDOK and OpenStreetMap).
    hits_full: bool,
    search: Option<Job<Vec<Hit>>>,
    suggest: Option<Job<(String, Vec<Hit>)>>,
    /// When the query last changed: suggestions wait for a pause in typing.
    typed_at: Option<std::time::Instant>,
    popup: bool,
    /// Highlighted row, moved with the arrow keys.
    highlight: Option<usize>,
    custom_epsg: String,
    crs_job: Option<Job<CrsInfo>>,
    geo_origin: bool,
    aerial: bool,
    apply: Option<Job<String>>,
}

struct RasterPage {
    server: Option<MapServer>,
    caps: Option<WmsCaps>,
    caps_job: Option<Job<WmsCaps>>,
    layer: Option<WmsLayer>,
    filter: String,
    format: String,
    resolution: f64,
    fade: u8,
    preview: Option<(TextureHandle, [LonLat; 4])>,
    preview_job: Option<Job<Preview>>,
    run: Option<Job<String>>,
}

struct WfsPage {
    server: Option<MapServer>,
    caps: Option<WfsCaps>,
    caps_job: Option<Job<WfsCaps>>,
    ft: Option<WfsType>,
    filter: String,
    limit: usize,
    fetch: Option<Job<(WfsResult, Vec<Vec<LonLat>>)>>,
    result: Option<WfsResult>,
    preview: Vec<(Vec<LonLat>, Color32)>,
    label: String,
    text_m: f64,
    aci: i16,
    clip: bool,
    place: Option<Job<String>>,
}

type OsmFetched = (Vec<&'static str>, Vec<Vec<Feature>>, Vec<(Vec<LonLat>, Color32)>);

struct OsmPage {
    on: Vec<bool>,
    labels: bool,
    text_m: f64,
    clip: bool,
    fetch: Option<Job<OsmFetched>>,
    fetched: Option<OsmFetched>,
    run: Option<Job<String>>,
}

struct PdokPage {
    on: Vec<bool>,
    run: Option<Job<String>>,
}

type Preview2 = Vec<(Vec<LonLat>, Color32)>;
/// Fetched BGT/BRK for the chosen groups, with its map preview.
type NlcsFetched = (Vec<&'static str>, bool, Arc<Vec<(String, Vec<Feature>)>>, Option<Arc<Cadastre>>, Preview2);

/// Contours for the map preview, with the height range.
type AhnPreview = (Preview2, (f64, f64), f64);

struct AhnPage {
    opt: AhnOptions,
    spots: bool,
    contours: bool,
    cloud: bool,
    class_on: Vec<bool>,
    preview: Option<AhnPreview>,
    test: Option<Job<AhnPreview>>,
    run: Option<Job<String>>,
}

impl Default for AhnPage {
    fn default() -> Self {
        let opt = hoogte::settings();
        let class_on = pointcloud::CLASSES.iter().map(|c| opt.cloud.classes.is_empty() || opt.cloud.classes.contains(&c.0)).collect();
        Self { opt, spots: false, contours: true, cloud: false, class_on, preview: None, test: None, run: None }
    }
}

/// Blue (low) to red (high).
fn height_colour(t: f64) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.5 { (40.0 + 60.0 * t * 2.0, 120.0 + 100.0 * t * 2.0, 220.0 - 140.0 * t * 2.0) } else { (100.0 + 130.0 * (t - 0.5) * 2.0, 220.0 - 160.0 * (t - 0.5) * 2.0, 80.0 - 40.0 * (t - 0.5) * 2.0) };
    Color32::from_rgb(r as u8, g as u8, b as u8)
}

struct ControlePage {
    scale: f64,
    text_heights: bool,
    empty_layers: bool,
    job: Option<Job<Report>>,
    report: Option<Report>,
    open: Option<Rule>,
    repair: Option<Job<String>>,
    /// BGT mutation of the design (IMGeo-GML).
    bronhouder: String,
    as_built: bool,
    mutation: Option<Job<String>>,
}

impl Default for ControlePage {
    fn default() -> Self {
        Self { scale: 500.0, text_heights: true, empty_layers: true, job: None, report: None, open: None, repair: None, bronhouder: String::new(), as_built: false, mutation: None }
    }
}

struct CivielPage {
    opt: CivielOptions,
    /// The offsets as typed: "-3,5; 3,5".
    offsets: String,
    run: Option<Job<String>>,
}

impl Default for CivielPage {
    fn default() -> Self {
        let opt = civiel::settings();
        let offsets = offsets_text(&opt.offsets);
        Self { opt, offsets, run: None }
    }
}

fn offsets_text(v: &[f64]) -> String {
    v.iter().map(|d| format!("{d}").replace('.', ",")).collect::<Vec<_>>().join("; ")
}

fn parse_offsets(s: &str) -> Vec<f64> {
    s.split([';', ' ']).filter_map(|t| t.trim().replace(',', ".").parse::<f64>().ok()).filter(|d| d.abs() > 1e-9).collect()
}

#[derive(Default)]
struct HoeveelhedenPage {
    selection: bool,
    annotation: bool,
    per_object: bool,
    filter: String,
    job: Option<Job<Quantities>>,
    result: Option<Quantities>,
}

fn drawing_status(status: Status) -> &'static str {
    match status {
        Status::B => tr(status.label(), "B — existing"),
        Status::N => tr(status.label(), "N — new"),
        Status::T => tr(status.label(), "T — temporary"),
        Status::V => tr(status.label(), "V — to be removed"),
    }
}

fn elevation_model(model: Model) -> &'static str {
    match model {
        Model::Dtm => tr(model.label(), "DTM — ground surface"),
        Model::Dsm => tr(model.label(), "DSM — surface (buildings, trees)"),
    }
}

fn point_colouring(colouring: Colouring) -> &'static str {
    match colouring {
        Colouring::Classification => tr(colouring.label(), "Class"),
        Colouring::Height => tr(colouring.label(), "Elevation"),
        Colouring::Rgb => tr(colouring.label(), "RGB (aerial imagery)"),
    }
}

fn survey_format(format: Format) -> &'static str {
    match format {
        Format::Auto => tr(format.label(), "Automatic"),
        Format::Pnezd => tr(format.label(), "PNEZD (point, northing, easting, Z, code)"),
        Format::Penzd => tr(format.label(), "PENZD (point, X, Y, Z, code)"),
        Format::Nezd => tr(format.label(), "NEZD (northing, easting, Z, code)"),
        Format::Enzd => tr(format.label(), "ENZD (X, Y, Z, code)"),
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Fout => tr(severity.label(), "error"),
        Severity::Waarschuwing => tr(severity.label(), "warning"),
        Severity::Info => severity.label(),
    }
}

fn rule_title(rule: Rule) -> &'static str {
    let english = match rule {
        Rule::LayerName => "Layer name does not follow NLCS",
        Rule::LayerObject => "Object is absent from the main group",
        Rule::LayerElement => "Element type is disallowed for the object",
        Rule::LayerColor => "Layer colour differs from NLCS",
        Rule::LayerLineweight => "Layer lineweight differs from NLCS",
        Rule::LayerLinetype => "Layer linetype differs from NLCS",
        Rule::EntityColor => "Entity colour is not ByLayer",
        Rule::EntityLineweight => "Entity lineweight is not ByLayer",
        Rule::EntityLinetype => "Entity linetype is not ByLayer",
        Rule::LayerZero => "Entities on layer 0",
        Rule::TextHeight => "Text height does not match an NLCS text class",
        Rule::EmptyLayer => "Empty NLCS layer",
    };
    tr(rule.title(), english)
}

fn severity_colour(s: Severity) -> Color32 {
    match s {
        Severity::Fout => ERR_RED,
        Severity::Waarschuwing => WARN,
        Severity::Info => Color32::from_gray(140),
    }
}

/// A small coloured label.
fn badge(ui: &mut egui::Ui, text: &str, colour: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::proportional(10.5), Color32::BLACK);
    let (rect, _) = ui.allocate_exact_size(Vec2::new((galley.size().x + 12.0).max(84.0), 18.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 9.0, colour);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::BLACK);
}

fn nl(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    // Thousands with a dot, decimals with a comma.
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
    let neg = int.starts_with('-');
    let digits: Vec<char> = int.trim_start_matches('-').chars().collect();
    let mut grouped = String::new();
    for (i, c) in digits.iter().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(*c);
    }
    format!("{}{}{}", if neg { "-" } else { "" }, grouped, frac.map(|f| format!(",{f}")).unwrap_or_default())
}

struct NlcsPage {
    status: Status,
    discipline: String,
    scale: f64,
    object_data: bool,
    labels: bool,
    clip: bool,
    bgt_on: Vec<bool>,
    brk: bool,
    klic_path: String,
    klic: Option<Arc<klic::Levering>>,
    klic_preview: Preview2,
    klic_job: Option<Job<(klic::Levering, Preview2)>>,
    klic_on: Vec<(String, usize, bool)>,
    klic_discipline: String,
    imkl_colors: bool,
    sheet: bool,
    info: SheetInfo,
    fetch: Option<Job<NlcsFetched>>,
    fetched: Option<NlcsFetched>,
    run: Option<Job<String>>,
}

impl Default for NlcsPage {
    fn default() -> Self {
        let o = NlcsOptions::default();
        Self {
            status: o.status,
            discipline: o.discipline,
            scale: o.scale,
            object_data: o.object_data,
            labels: o.labels,
            clip: o.clip,
            bgt_on: bgt::GROUPS.iter().map(|g| g.default_on).collect(),
            brk: true,
            klic_path: String::new(),
            klic: None,
            klic_preview: Vec::new(),
            klic_job: None,
            klic_on: Vec::new(),
            klic_discipline: "OI".into(),
            imkl_colors: false,
            sheet: true,
            info: SheetInfo::default(),
            fetch: None,
            fetched: None,
            run: None,
        }
    }
}

impl NlcsPage {
    fn options(&self) -> NlcsOptions {
        NlcsOptions { status: self.status, discipline: self.discipline.clone(), scale: self.scale, object_data: self.object_data, labels: self.labels, clip: self.clip }
    }
    fn klic_options(&self) -> KlicOptions {
        let mut nlcs = self.options();
        nlcs.discipline = self.klic_discipline.clone();
        nlcs.clip = false;
        KlicOptions { nlcs, imkl_colors: self.imkl_colors, themes: self.klic_on.iter().filter(|t| t.2).map(|t| t.0.clone()).collect() }
    }
    fn collections(&self) -> Vec<&'static str> {
        bgt::GROUPS.iter().zip(&self.bgt_on).filter(|(_, on)| **on).flat_map(|(g, _)| g.collections.iter().copied()).collect()
    }
}

fn preview_color(rgb: (u8, u8, u8)) -> Color32 {
    // NLCS colour 7 is white on a dark CAD background: dark on the map.
    if rgb == (255, 255, 255) { Color32::from_gray(40) } else { Color32::from_rgb(rgb.0, rgb.1, rgb.2) }
}

fn rd_lines(to: &dyn Fn([f64; 2]) -> Option<[f64; 2]>, f: &Feature, color: Color32, out: &mut Preview2) {
    for (line, closed) in &f.lines {
        let mut pts: Vec<LonLat> = line.iter().filter_map(|p| to(*p)).collect();
        if *closed {
            if let Some(first) = pts.first().copied() {
                pts.push(first);
            }
        }
        if pts.len() > 1 {
            out.push((pts, color));
        }
    }
}

fn bgt_preview(site: &Site, data: &[(String, Vec<Feature>)], cad: Option<&Cadastre>) -> Result<Preview2, String> {
    let to = vector::data_to(site, "EPSG:28992", "EPSG:4326", &[])?;
    let mut out = Vec::new();
    for (c, feats) in data {
        if bgt::is_label(c) {
            continue;
        }
        for f in feats {
            let rgb = nlcs::bgt_lookup(c, &f.props).map(|x| x.0).or_else(|| nlcs::bgt_default(c)).and_then(nlcs::object).map(|o| nlcs::layer(o, Status::B, "XX", Element::G).rgb).unwrap_or((128, 128, 128));
            rd_lines(&*to, f, preview_color(rgb), &mut out);
        }
    }
    if let Some(cad) = cad {
        let rgb = nlcs::object_by_name("KG", "GRENS_PERCEEL").map(|o| nlcs::layer(o, Status::B, "XX", Element::G).rgb).unwrap_or((255, 0, 0));
        for f in &cad.boundaries {
            rd_lines(&*to, f, preview_color(rgb), &mut out);
        }
    }
    Ok(out)
}

fn klic_preview(site: &Site, lev: &klic::Levering) -> Result<Preview2, String> {
    let to = vector::data_to(site, "EPSG:28992", "EPSG:4326", &[])?;
    let mut out = Vec::new();
    for f in &lev.features {
        rd_lines(&*to, f, preview_color(nlcs::imkl_rgb(klic::thema(f))), &mut out);
    }
    Ok(out)
}

/// The RD extent of a delivery.
fn klic_extent(lev: &klic::Levering) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for f in &lev.features {
        for p in f.lines.iter().flat_map(|(l, _)| l.iter()).chain(f.points.iter()) {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
    }
    (b[0] < b[2]).then_some(b)
}

#[derive(Default)]
struct ServersPage {
    kind: Option<ServerKind>,
    url: String,
    name: String,
    region: String,
    test: Option<Job<String>>,
    /// QGIS connections export: path and the connections read from it, with their checkbox.
    qgis_path: String,
    qgis: Vec<(crate::gis::qgis::Connection, bool)>,
}

struct CoordsPage {
    crs: String,
    xy: [f64; 2],
    picked: Option<LonLat>,
    job: Option<Job<CrsInfo>>,
}

pub struct App {
    rx: Receiver<UiMsg>,
    tab: Tab,
    /// Location page edits a draft; the other pages use the saved site.
    draft: Site,
    has_site: bool,
    message: Option<(bool, String)>,
    map: MapView,
    fit_pending: bool,
    loc: LocationPage,
    raster: RasterPage,
    wfs: WfsPage,
    osm: OsmPage,
    pdok: PdokPage,
    nlcs: NlcsPage,
    ahn: AhnPage,
    controle: ControlePage,
    hoev: HoeveelhedenPage,
    civ: CivielPage,
    srv: ServersPage,
    coords: CoordsPage,
    servers: Vec<MapServer>,
    lib: super::library::LibraryPage,
    files: super::files::FilesPage,
    svc: services::ServicesPage,
    planning: super::planning::PlanningPage,
}

#[cfg(test)]
mod qgis_preview_tests {
    use super::*;

    fn render(connections: Vec<crate::gis::qgis::Connection>) -> String {
        let context = egui::Context::default();
        let creation = eframe::CreationContext::_new_kittest(context.clone());
        let (_, receiver) = std::sync::mpsc::channel();
        let mut app = App::new(&creation, receiver);
        app.srv.qgis = connections.into_iter().map(|c| (c, true)).collect();
        let output = context.run_ui(egui::RawInput::default(), |ui| app.qgis_import(ui));
        let rendered = format!("{:?}", output.shapes);
        output.drop_without_applying_deltas();
        rendered
    }

    #[test]
    fn qgis_preview_does_not_render_url_credentials() {
        let connections = crate::gis::qgis::parse(r#"<qgsWMSConnections>
            <wms name="Besloten" url="https://example.invalid/wms?password=SYNTHETIC_VALUE"/>
            <wms name="Openbaar" url="https://public.invalid/wms"/>
        </qgsWMSConnections>"#).unwrap();
        let rendered = render(connections);
        assert!(rendered.contains("public.invalid"), "de echte voorvertoning moet gevuld zijn");
        assert!(!rendered.contains("SYNTHETIC_VALUE"));
    }

    #[test]
    fn azure_maps_subscription_keys_do_not_render() {
        let connections = crate::gis::qgis::parse(include_str!("../../tests/fixtures/qgis/azure-maps.xml")).unwrap();
        assert_eq!(connections.len(), 3);
        let public = crate::gis::qgis::parse(r#"<qgsXYZTilesConnections><xyztiles name="Openbaar" url="https://public.invalid/{z}/{x}/{y}.png"/></qgsXYZTilesConnections>"#).unwrap().remove(0);
        let mut failures = Vec::new();
        for c in connections {
            let name = c.server.name.clone();
            let rendered = render(vec![c, public.clone()]);
            assert!(rendered.contains("public.invalid"), "de echte voorvertoning moet gevuld zijn");
            let leaked = rendered.contains("SYNTHETIC_VALUE");
            println!("{name}: gevoelige waarde in voorvertoning={leaked}");
            if leaked {
                failures.push(name);
            }
        }
        assert!(failures.is_empty(), "Zichtbaar in de voorvertoning: {}", failures.join(", "));
    }

    fn check_auth_cases(cases: Vec<crate::gis::qgis::auth_test_support::Case>) {
        let public = crate::gis::qgis::parse(r#"<qgsWMSConnections><wms name="Openbaar" url="https://public.invalid/wms"/></qgsWMSConnections>"#).unwrap().remove(0);
        let mut failures = Vec::new();
        for case in cases {
            let name = case.connection.server.name.clone();
            let url = case.connection.server.url.clone();
            let rendered = render(vec![case.connection, public.clone()]);
            assert!(rendered.contains("public.invalid"), "de echte voorvertoning moet gevuld zijn");
            let leaked = crate::gis::qgis::auth_test_support::contains_test_secret(&rendered);
            let public_visible = !url.is_empty() && rendered.contains(&url);
            println!("{name}: verwacht blokkade={}, gevoelige waarde zichtbaar={leaked}, publieke URL zichtbaar={public_visible}", case.blocked);
            if leaked || (!case.blocked && !public_visible) { failures.push(name); }
        }
        assert!(failures.is_empty(), "Onjuiste voorvertoning: {}", failures.join(", "));
    }

    #[test]
    fn ordnance_survey_keys_do_not_render() {
        check_auth_cases(crate::gis::qgis::auth_test_support::os_cases());
    }

    #[test]
    fn geoserver_and_here_credentials_do_not_render() {
        check_auth_cases(crate::gis::qgis::auth_test_support::review_cases());
    }

    #[test]
    fn audited_credentials_do_not_render_and_public_urls_stay_visible() {
        check_auth_cases(crate::gis::qgis::auth_test_support::audited_cases());
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rx: Receiver<UiMsg>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = ACCENT;
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.all_styles_mut(|s| {
            s.spacing.item_spacing = Vec2::new(8.0, 6.0);
            s.spacing.button_padding = Vec2::new(10.0, 4.0);
        });
        let site = bridge::site();
        let draft = site.clone().unwrap_or_else(||Site::for_region(crate::localisation::current()));
        Self {
            rx,
            tab: Tab::Location,
            has_site: site.is_some(),
            map: MapView::new([draft.lon, draft.lat], 15.0),
            draft,
            fit_pending: true,
            message: None,
            loc: LocationPage::default(),
            raster: RasterPage { server: None, caps: None, caps_job: None, layer: None, filter: String::new(), format: "image/png".into(), resolution: 0.25, fade: 0, preview: None, preview_job: None, run: None },
            wfs: WfsPage { server: None, caps: None, caps_job: None, ft: None, filter: String::new(), limit: 5000, fetch: None, result: None, preview: Vec::new(), label: String::new(), text_m: 1.0, aci: 2, clip: true, place: None },
            osm: OsmPage { on: THEMES.iter().map(|t| t.default_on).collect(), labels: true, text_m: 2.0, clip: true, fetch: None, fetched: None, run: None },
            pdok: PdokPage { on: PDOK.iter().map(|d| d.default_on).collect(), run: None },
            nlcs: NlcsPage::default(),
            ahn: AhnPage::default(),
            controle: ControlePage::default(),
            hoev: HoeveelhedenPage::default(),
            civ: CivielPage::default(),
            srv: ServersPage::default(),
            coords: CoordsPage { crs: "EPSG:4326".into(), xy: [4.660_612, 51.814_158], picked: None, job: None },
            servers: servers::all(),
            lib: Default::default(),
            files: Default::default(),
            svc: Default::default(),
            planning: Default::default(),
        }
    }

    fn ok(&mut self, msg: impl Into<String>) {
        let m = msg.into();
        bridge::info(&m);
        self.message = Some((true, m));
    }

    fn err(&mut self, msg: impl Into<String>) {
        self.message = Some((false, msg.into()));
    }

    fn reload_site(&mut self) {
        match bridge::site() {
            Some(s) => {
                self.draft = s;
                self.has_site = true;
            }
            None => self.has_site = false,
        }
        self.fit_pending = true;
    }

    fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                UiMsg::RegionalServices(region) => {self.svc.regional(region);self.tab=Tab::Services;}
                UiMsg::InfraLibrary => { self.tab = Tab::Library; self.lib.show_infra = true; }
                UiMsg::Show(tab) => {
                    self.tab = tab;
                    self.reload_site();
                    self.servers = servers::all();
                    self.svc.refresh();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiMsg::Picked(p) => {
                    self.tab = Tab::Coords;
                    if let Ok(ll) = crs::transform(&self.draft.crs, "EPSG:4326", p) {
                        self.coords.picked = Some(ll);
                        self.map.center = ll;
                    }
                    if let Ok(q) = crs::transform(&self.draft.crs, &self.coords.crs, p) {
                        self.coords.xy = q;
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiMsg::SiteChanged => self.reload_site(),
                UiMsg::File(path) => {
                    self.tab = Tab::Files;
                    self.reload_site();
                    self.files.open(ctx, path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiMsg::Run(tab) => {
                    self.tab = tab;
                    self.reload_site();
                    match tab {
                        Tab::Controle => self.start_check(ctx),
                        Tab::Hoeveelheden => self.start_quantities(ctx),
                        _ => {}
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                UiMsg::Klic(path) => {
                    self.tab = Tab::Nlcs;
                    self.reload_site();
                    self.nlcs.klic_path = path.to_string_lossy().into_owned();
                    self.load_klic(ctx);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
    }

    /// The site for import pages (saved in the drawing), or a hint to set one.
    fn saved_site(&mut self, ui: &mut egui::Ui) -> Option<Site> {
        let s = bridge::site();
        if s.is_none() {
            group(ui, tr("Projectlocatie ontbreekt", "Project location missing"), |ui| {
                ui.colored_label(WARN, tr("Deze tekening heeft nog geen projectlocatie. Stel die eerst in.", "Set a project location for this drawing first."));
                if ui.button(tr("Geografische locatie instellen…", "Set geographic location…")).clicked() {
                    self.tab = Tab::Location;
                }
            });
        }
        s
    }

    /// Map toolbar + map. `site` gives the bounding box drawn on the map.
    fn map_section(&mut self, ui: &mut egui::Ui, site: Option<&Site>, marker: Option<LonLat>, image: Option<&(TextureHandle, [LonLat; 4])>, lines: &[(Vec<LonLat>, Color32)]) -> Option<MapEvent> {
        let ring = site.and_then(site_ring);
        let map_h = if self.tab == Tab::Location { (ui.ctx().content_rect().height() * 0.62).max(460.0) } else { MAP_H };
        if self.fit_pending {
            if let Some(r) = &ring {
                self.map.fit(r, Vec2::new(ui.available_width(), map_h));
                self.fit_pending = false;
            }
        }
        let mut event = None;
        group(ui, tr("Kaart", "Map"), |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Button::image_and_text(icon("area"), tr("Gebied tekenen", "Draw area")).selected(self.map.area_mode))
                    .on_hover_text(tr("Sleep op de kaart een rechthoek als importgebied (of houd Shift ingedrukt)", "Drag a rectangle on the map to set the import area (or hold Shift)"))
                    .clicked()
                    .then(|| self.map.area_mode = !self.map.area_mode);
                if ui.add_enabled(ring.is_some(), egui::Button::new(tr("Zoom naar gebied", "Zoom to area"))).clicked() {
                    if let Some(r) = &ring {
                        self.map.fit(r, Vec2::new(ui.available_width(), map_h));
                    }
                }
                if let Some(h) = self.map.hover() {
                    let mut txt = format!("{:.6}° N  {:.6}° O", h[1], h[0]);
                    if let Some(s) = site {
                        if let Ok(p) = crs::transform("EPSG:4326", &s.crs, h) {
                            txt.push_str(&format!("   |   {}  X {:.1}  Y {:.1}", s.crs, p[0], p[1]));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(txt).monospace().small().weak());
                    });
                }
            });
            event = self.map.show(ui, map_h, Overlay { marker, bbox: ring.clone(), image: image.map(|(t, c)| (t, *c)), lines });
        });
        event
    }

    // ── Geografische locatie ──────────────────────────────────────────────
    fn location(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.loc.search) {
            match r {
                Ok(h) => {
                    self.loc.hits = h;
                    self.loc.hits_for = self.loc.query.trim().to_string();
                    self.loc.hits_full = true;
                    self.loc.highlight = None;
                    self.loc.popup = true;
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.loc.suggest) {
            if let Ok((q, h)) = r {
                // Only for what is in the field now, and never over a full search.
                if q == self.loc.query.trim() && !(self.loc.hits_full && self.loc.hits_for == q) {
                    self.loc.hits = h;
                    self.loc.hits_for = q;
                    self.loc.hits_full = false;
                    self.loc.highlight = None;
                }
            }
        }
        // Suggestions after a short pause in typing (PDOK only: Nominatim
        // does not allow type-ahead).
        let q = self.loc.query.trim().to_string();
        if let Some(t) = self.loc.typed_at {
            let wait = std::time::Duration::from_millis(300);
            if t.elapsed() < wait {
                ctx.request_repaint_after(wait - t.elapsed());
            } else if self.loc.suggest.is_none() {
                self.loc.typed_at = None;
                if q.chars().count() >= 3 && q != self.loc.hits_for {
                    let q2 = q.clone();
                    self.loc.suggest = Some(Job::spawn(&ctx, tr("Suggesties…", "Suggestions…"), move |_| geocode::suggest(&q2).map(|h| (q2, h))));
                }
            }
        }
        if let Some(r) = poll(&mut self.loc.crs_job) {
            match r {
                Ok(info) if info.proj4.contains("+proj=longlat") => self.err(tr("Kies een geprojecteerd stelsel (in meters) als projectstelsel.", "Choose a projected coordinate system (in metres) for the project.")),
                Ok(info) => {
                    self.draft.crs = info.code;
                    self.draft.reproject_origin();
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.loc.apply) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(ui, "location", tr("Geografische locatie instellen", "Set geographic location"), tr("Kies de projectlocatie op de kaart of zoek een adres; teken of typ de begrenzing van het importgebied.", "Pick the project location on the map or search for an address. Draw or enter the import area bounds."));
        let draft = self.draft.clone();
        match self.map_section(ui, Some(&draft), Some([draft.lon, draft.lat]), None, &[]) {
            Some(MapEvent::Clicked(ll)) => {
                self.draft.lon = ll[0];
                self.draft.lat = ll[1];
                self.draft.label = tr_format!("Kaartpunt {:.5}, {:.5}", "Map point {:.5}, {:.5}", ll[1], ll[0]);
                self.draft.reproject_origin();
            }
            Some(MapEvent::RectDrawn(a, b)) => {
                if let (Ok(pa), Ok(pb)) = (crs::transform("EPSG:4326", &self.draft.crs, a), crs::transform("EPSG:4326", &self.draft.crs, b)) {
                    self.draft.set_origin(((pa[0] + pb[0]) / 2.0).round(), ((pa[1] + pb[1]) / 2.0).round());
                    self.draft.width = (pa[0] - pb[0]).abs().round().max(10.0);
                    self.draft.height = (pa[1] - pb[1]).abs().round().max(10.0);
                    self.draft.label = tr_format!("Getekend gebied {} × {} m", "Drawn area {} × {} m", self.draft.width, self.draft.height);
                    self.map.area_mode = false;
                }
            }
            None => {}
        }

        let mut field: Option<egui::Rect> = None;
        let mut pick: Option<usize> = None;
        let mut search_all = false;
        group(ui, tr("Locatie", "Location"), |ui| {
            egui::Grid::new("loc").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label(tr("Zoek plaats / adres", "Find a place or address"));
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.loc.query)
                            .hint_text(tr("adres, plaats, perceel, straat — of 51.814, 4.660 / RD 104885 425320", "address, place, parcel, street or 51.814, 4.660 / RD 104885 425320"))
                            .desired_width(ui.available_width() - 90.0),
                    );
                    field = Some(edit.rect);
                    if edit.changed() {
                        self.loc.typed_at = Some(std::time::Instant::now());
                        self.loc.popup = !self.loc.query.trim().is_empty();
                        self.loc.highlight = None;
                        if self.loc.query.trim().is_empty() {
                            self.loc.hits.clear();
                            self.loc.hits_for.clear();
                        }
                    }
                    if edit.gained_focus() && !self.loc.hits.is_empty() {
                        self.loc.popup = true;
                    }
                    if edit.has_focus() || edit.lost_focus() {
                        let (down, up, esc, enter) = ui.input(|i| (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
                        let n = self.loc.hits.len();
                        if down && n > 0 {
                            self.loc.popup = true;
                            self.loc.highlight = Some(self.loc.highlight.map_or(0, |h| (h + 1).min(n - 1)));
                        }
                        if up {
                            self.loc.highlight = self.loc.highlight.and_then(|h| h.checked_sub(1));
                        }
                        if esc {
                            self.loc.popup = false;
                        }
                        if enter && edit.lost_focus() {
                            // A row picked with the arrow keys; otherwise search everywhere.
                            match self.loc.highlight.filter(|h| self.loc.popup && *h < n) {
                                Some(h) => pick = Some(h),
                                None => search_all = true,
                            }
                        }
                    }
                    let typed = !self.loc.query.trim().is_empty();
                    if ui.add_enabled(self.loc.search.is_none() && typed, egui::Button::new(tr("Zoeken", "Search"))).on_hover_text(tr("In Nederland (PDOK) en wereldwijd (OpenStreetMap)", "In the Netherlands (PDOK) and worldwide (OpenStreetMap)")).on_disabled_hover_text(tr("Typ eerst een adres, plaats of coördinaten", "Enter an address, place or coordinates first")).clicked() {
                        search_all = true;
                    }
                });
                ui.end_row();

                ui.label(tr("Coördinatenstelsel (CRS)", "Coordinate reference system (CRS)"));
                ui.horizontal(|ui| {
                    let suggested = crs::suggest(self.draft.lon, self.draft.lat);
                    let mut list = crs::builtin_list();
                    for c in [&suggested, &self.draft.crs] {
                        if !list.iter().any(|i| &i.code == c) {
                            if let Some(i) = crs::known(c) {
                                list.push(i);
                            }
                        }
                    }
                    let current = crs::known(&self.draft.crs).map(|i| format!("{} — {}", i.code, i.name)).unwrap_or(self.draft.crs.clone());
                    let mut chosen = None;
                    egui::ComboBox::from_id_salt("crs").width(340.0).height(420.0).selected_text(current).show_ui(ui, |ui| {
                        for i in list.iter().filter(|i| !i.proj4.contains("+proj=longlat")) {
                            let star = if i.code == suggested { tr("  ★ aanbevolen", "  ★ recommended") } else { "" };
                            if ui.selectable_label(i.code == self.draft.crs, format!("{} — {} ({}){star}", i.code, i.name, i.region)).clicked() {
                                chosen = Some(i.code.clone());
                            }
                        }
                    });
                    if let Some(c) = chosen {
                        self.draft.crs = c;
                        self.draft.reproject_origin();
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.loc.custom_epsg).hint_text(tr("EPSG-code", "EPSG code")).desired_width(80.0));
                    if ui.add_enabled(self.loc.crs_job.is_none() && !self.loc.custom_epsg.trim().is_empty(), egui::Button::new(tr("Laden", "Load"))).on_hover_text(tr("Definitie ophalen via epsg.io", "Fetch the definition from epsg.io")).clicked() {
                        let code = self.loc.custom_epsg.clone();
                        self.loc.crs_job = Some(Job::spawn(&ctx, tr("Stelsel laden…", "Loading CRS…"), move |_| crs::ensure(&code)));
                    }
                });
                ui.end_row();

                ui.label(tr("Breedtegraad / lengtegraad", "Latitude / longitude"));
                ui.horizontal(|ui| {
                    let a = ui.add(egui::DragValue::new(&mut self.draft.lat).speed(0.0001).fixed_decimals(7).prefix("lat "));
                    let b = ui.add(egui::DragValue::new(&mut self.draft.lon).speed(0.0001).fixed_decimals(7).prefix("lon "));
                    if a.changed() || b.changed() {
                        self.draft.reproject_origin();
                    }
                });
                ui.end_row();

                ui.label(tr("Omschrijving", "Description"));
                ui.add(egui::TextEdit::singleline(&mut self.draft.label).desired_width(f32::INFINITY));
                ui.end_row();
            });
        });

        if let Some(rect) = field {
            if let Some(i) = self.search_popup(ui, rect) {
                pick = Some(i);
            }
        }
        if search_all && !self.loc.query.trim().is_empty() && self.loc.search.is_none() {
            let q = self.loc.query.trim().to_string();
            self.loc.popup = true;
            self.loc.typed_at = None;
            self.loc.search = Some(Job::spawn(&ctx, tr("Zoeken…", "Searching…"), move |_| geocode::search(&q)));
        }
        if let Some(i) = pick {
            self.pick_hit(&ctx, i);
        }

        group(ui, tr("Begrenzing en plaatsing", "Bounds and placement"), |ui| {
            egui::Grid::new("bbox").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label(tr("Basispunt X / Y (m)", "Origin X / Y (m)"));
                ui.horizontal(|ui| {
                    let (mut e, mut n) = (self.draft.origin_e, self.draft.origin_n);
                    let a = ui.add(egui::DragValue::new(&mut e).speed(1.0).fixed_decimals(3).prefix("X "));
                    let b = ui.add(egui::DragValue::new(&mut n).speed(1.0).fixed_decimals(3).prefix("Y "));
                    if a.changed() || b.changed() {
                        self.draft.set_origin(e, n);
                    }
                });
                ui.end_row();
                ui.label(tr("Begrenzing breedte × hoogte (m)", "Area width × height (m)"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.draft.width).speed(10.0).range(10.0..=20_000.0));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut self.draft.height).speed(10.0).range(10.0..=20_000.0));
                    for s in [250.0, 500.0, 1000.0, 2000.0] {
                        if ui.small_button(format!("{s:.0}")).clicked() {
                            self.draft.width = s;
                            self.draft.height = s;
                        }
                    }
                });
                ui.end_row();
                ui.label(tr("Coördinaten in de tekening", "Coordinates in the drawing"));
                ui.vertical(|ui| {
                    ui.radio_value(&mut self.draft.absolute, true, tr("Absoluut — CRS-coördinaten (bijv. RD) direct in de tekening", "Absolute — use CRS coordinates (e.g. RD) directly in the drawing"));
                    ui.radio_value(&mut self.draft.absolute, false, tr("Relatief — basispunt = 0,0", "Relative — origin = 0,0"));
                });
                ui.end_row();
                ui.label(tr("Na opslaan", "After saving"));
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.loc.geo_origin, tr("Basispunt-markering in de tekening plaatsen (GeoOrigin)", "Place an origin marker in the drawing (GeoOrigin)"));
                    ui.checkbox(&mut self.loc.aerial, tr("Luchtfoto importeren (PDOK in NL, Sentinel-2 elders)", "Import aerial imagery (PDOK in the Netherlands, Sentinel-2 elsewhere)"));
                });
                ui.end_row();
            });
            let units = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
            ui.label(RichText::new(tr_format!(
                "Tekeneenheid: {} (INSUNITS){}", "Drawing unit: {} (INSUNITS){}",
                if (units - 1.0).abs() < 1e-9 { tr("meter", "metre").to_string() } else if (units - 1000.0).abs() < 1e-9 { tr("millimeter", "millimetre").into() } else { tr_format!("{units} per meter", "{units} per metre") },
                if self.has_site { "" } else { tr(" · deze tekening heeft nog geen projectlocatie", " · this drawing has no project location yet") }
            )).small().weak());
        });

    }

    /// The suggestion list under the search field. Returns a clicked row.
    fn search_popup(&mut self, ui: &mut egui::Ui, field: egui::Rect) -> Option<usize> {
        let busy = self.loc.search.is_some() || self.loc.suggest.is_some();
        let q = self.loc.query.trim().to_string();
        let current = !self.loc.hits.is_empty() && self.loc.hits_for == q;
        let nothing = self.loc.hits_full && self.loc.hits_for == q && self.loc.hits.is_empty();
        if !self.loc.popup || q.is_empty() || !(current || busy || nothing || q.chars().count() >= 3) {
            return None;
        }
        let mut picked = None;
        let width = field.width() + 90.0;
        // Below the field when it fits, else above it: never over the field.
        let rows = if current { self.loc.hits.len() } else { 1 };
        let needed = rows as f32 * 40.0 + 70.0;
        let screen = ui.ctx().content_rect();
        let below = screen.bottom() - field.bottom() - 10.0;
        let above = field.top() - screen.top() - 10.0;
        let (pos, pivot, room) = if needed <= below || below >= above {
            (field.left_bottom() + Vec2::new(0.0, 4.0), egui::Align2::LEFT_TOP, below)
        } else {
            (field.left_top() - Vec2::new(0.0, 4.0), egui::Align2::LEFT_BOTTOM, above)
        };
        let area = egui::Area::new(egui::Id::unique("open-infra-geo-suggest"))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(pos)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.set_width(width);
                    egui::ScrollArea::vertical().max_height((room - 60.0).max(120.0)).auto_shrink([false, true]).show(ui, |ui| {
                    if current {
                        // Grouped by source, the group of the best hit first.
                        let first = self.loc.hits[0].source;
                        let mut order: Vec<usize> = (0..self.loc.hits.len()).filter(|i| self.loc.hits[*i].source == first).collect();
                        order.extend((0..self.loc.hits.len()).filter(|i| self.loc.hits[*i].source != first));
                        let groups = order.iter().map(|i| self.loc.hits[*i].source).collect::<std::collections::BTreeSet<_>>().len();
                        let mut last_source = "";
                        for i in order {
                            let h = &self.loc.hits[i];
                            if groups > 1 && h.source != last_source {
                                let title = match h.source {
                                    "PDOK" => tr("NEDERLAND · PDOK", "NETHERLANDS · PDOK"),
                                    "OSM" => tr("WERELDWIJD · OPENSTREETMAP", "WORLDWIDE · OPENSTREETMAP"),
                                    _ => tr("INVOER", "INPUT"),
                                };
                                ui.add_space(if last_source.is_empty() { 0.0 } else { 4.0 });
                                ui.label(RichText::new(title).small().strong().color(ACCENT));
                                last_source = h.source;
                            }
                            if hit_row(ui, h, self.loc.highlight == Some(i)).clicked() {
                                picked = Some(i);
                            }
                        }
                    } else if nothing {
                        ui.label(RichText::new(tr_format!("Niets gevonden voor \"{q}\".", "No results for \"{q}\".")).weak());
                    }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if busy {
                            ui.spinner();
                            ui.label(RichText::new(if self.loc.search.is_some() { tr("Zoeken in Nederland en wereldwijd…", "Searching in the Netherlands and worldwide…") } else { tr("Suggesties…", "Suggestions…") }).small().weak());
                        } else if self.loc.hits_full {
                            ui.label(RichText::new(tr("Pijltjes en Enter om te kiezen · Esc sluit", "Arrow keys and Enter to select · Esc to close")).small().weak());
                        } else {
                            ui.label(RichText::new(tr("Enter of Zoeken: ook wereldwijd zoeken (OpenStreetMap) · pijltjes om te kiezen · Esc sluit", "Enter or Search to include worldwide results (OpenStreetMap) · arrow keys to select · Esc to close")).small().weak());
                        }
                    });
                });
            });
        // A click elsewhere closes the list.
        let clicked_outside = ui.ctx().input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !field.expand2(Vec2::new(90.0, 0.0)).contains(p)));
        if clicked_outside {
            self.loc.popup = false;
        }
        picked
    }

    fn pick_hit(&mut self, ctx: &egui::Context, i: usize) {
        let Some(h) = self.loc.hits.get(i).cloned() else { return };
        self.draft.label = h.label.clone();
        self.draft.lon = h.lon;
        self.draft.lat = h.lat;
        let code = crs::suggest_for(&h.country, h.lon, h.lat);
        if crs::known(&code).is_some() {
            self.draft.crs = code.clone();
            self.draft.reproject_origin();
        } else {
            let c = code.clone();
            self.loc.crs_job = Some(Job::spawn(ctx, tr("Stelsel laden…", "Loading CRS…"), move |_| crs::ensure(&c)));
        }
        self.map.center = [h.lon, h.lat];
        self.fit_pending = true;
        self.loc.query = h.label.clone();
        self.loc.hits_for = h.label.clone();
        self.loc.popup = false;
        self.loc.highlight = None;
        self.loc.typed_at = None;
        self.message = Some((true, tr_format!("Locatie: {} — stelsel {code}. Klik op OK om op te slaan.", "Location: {} · CRS {code}. Click OK to save.", h.label)));
    }

    // ── Rasterdata (WMS / TMS) ──────────────────────────────────────────────
    fn raster(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.raster.caps_job) {
            match r {
                Ok(c) => {
                    let pre = self.raster.server.as_ref().map(|s| s.default_layer.clone()).unwrap_or_default();
                    self.raster.layer = c.layers.iter().find(|l| l.name == pre).or(c.layers.first()).cloned();
                    if let Some(l) = &self.raster.layer {
                        self.raster.format = raster::default_format(&c, &l.name);
                    }
                    self.message = Some((true, tr_format!("{} lagen (WMS {}).", "{} layers (WMS {}).", c.layers.len(), c.version)));
                    self.raster.caps = Some(c);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.raster.preview_job) {
            match r {
                Ok(p) => {
                    let img = egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba);
                    self.raster.preview = Some((ctx.load_texture("wms-preview", img, egui::TextureOptions::LINEAR), p.corners));
                    self.message = Some((true, tr("Voorbeeld getoond op de kaart.", "Preview shown on the map.").into()));
                }
                Err(e) => self.err(tr_format!("Test mislukt: {e}", "Test failed: {e}")),
            }
        }
        if let Some(r) = poll(&mut self.raster.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(ui, "map", tr("Rasterdata laden met WMS / TMS", "Load raster data with WMS / TMS"), tr("Luchtfoto's, kaarten en hoogtebeelden als onderlegger in de tekening.", "Aerial imagery, maps and elevation imagery as drawing underlays."));
        let Some(site) = self.saved_site(ui) else { return };
        let preview = self.raster.preview.take();
        self.map_section(ui, Some(&site), None, preview.as_ref(), &[]);
        self.raster.preview = preview;

        group(ui, tr("Webserver / verzoek", "Web server / request"), |ui| {
            egui::Grid::new("wms").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label(tr("Voorgedefinieerde server", "Predefined server"));
                if let Some(s) = server_combo(ui, "wms_srv", &self.servers, &[ServerKind::Wms, ServerKind::Xyz], self.raster.server.as_ref()) {
                    self.raster.caps = None;
                    self.raster.layer = None;
                    self.raster.preview = None;
                    self.raster.filter.clear();
                    if s.kind == ServerKind::Wms {
                        let url = s.url.clone();
                        self.raster.caps_job = Some(Job::spawn(&ctx, tr("Capabilities ophalen…", "Fetching capabilities…"), move |_| caps::fetch_wms(&url)));
                        self.raster.resolution = 0.25;
                    } else {
                        self.raster.resolution = 0.5;
                    }
                    self.raster.server = Some(s);
                }
                ui.end_row();
                if let Some(s) = &self.raster.server {
                    ui.label(tr("Bron", "Source"));
                    ui.label(RichText::new(format!("{}  ·  {}", s.attribution, s.url)).small().weak());
                    ui.end_row();
                }
                if let Some(j) = &self.raster.caps_job {
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(j.status());
                    });
                    ui.end_row();
                }
                if let (Some(c), Some(ServerKind::Wms)) = (&self.raster.caps, self.raster.server.as_ref().map(|s| s.kind)) {
                    ui.label(tr("Laag", "Layer"));
                    ui.label(RichText::new(self.raster.layer.as_ref().map(|l| format!("{}  ({})", l.title, l.name)).unwrap_or_else(|| tr("— kies hieronder —", "— choose below —").into())).strong());
                    ui.end_row();
                    ui.label(tr("Formaat", "Format"));
                    egui::ComboBox::from_id_salt("fmt").selected_text(self.raster.format.clone()).show_ui(ui, |ui| {
                        for f in c.formats.iter().filter(|f| f.starts_with("image/png") || f.starts_with("image/jpeg")) {
                            ui.selectable_value(&mut self.raster.format, f.clone(), f);
                        }
                    });
                    ui.end_row();
                }
            });
            if let (Some(c), Some(ServerKind::Wms)) = (&self.raster.caps, self.raster.server.as_ref().map(|s| s.kind)) {
                ui.add_space(4.0);
                ui.add(egui::TextEdit::singleline(&mut self.raster.filter).hint_text(tr_format!("Filter {} lagen…", "Filter {} layers…", c.layers.len())).desired_width(f32::INFINITY));
                let mut chosen = None;
                list_frame(ui, 170.0, "wms_layers", |ui| {
                    let f = self.raster.filter.to_lowercase();
                    for l in c.layers.iter().filter(|l| f.is_empty() || l.name.to_lowercase().contains(&f) || l.title.to_lowercase().contains(&f)) {
                        let sel = self.raster.layer.as_ref().is_some_and(|x| x.name == l.name);
                        let reproj = !l.crs.is_empty() && !l.crs.contains(&site.crs);
                        let txt = format!("{}{}{}", l.title, if l.title != l.name { format!("   ({})", l.name) } else { String::new() }, if reproj { tr("   · herprojectie", "   · reprojection") } else { "" });
                        if ui.selectable_label(sel, txt).clicked() {
                            chosen = Some(l.clone());
                        }
                    }
                });
                if let Some(l) = chosen {
                    self.raster.format = raster::default_format(c, &l.name);
                    self.raster.layer = Some(l);
                    self.raster.preview = None;
                }
            }
        });

        let mut too_big = false;
        if self.raster.server.is_some() {
            group(ui, tr("Import-instellingen", "Import settings"), |ui| {
                egui::Grid::new("wms_imp").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                    ui.label(tr("Resolutie", "Resolution"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.raster.resolution).speed(0.01).range(0.02..=100.0).suffix(" m/px"));
                        for r in [0.08, 0.25, 0.5, 1.0] {
                            if ui.small_button(format!("{r}")).clicked() {
                                self.raster.resolution = r;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Pixels");
                    let (w, h) = (site.width / self.raster.resolution, site.height / self.raster.resolution);
                    too_big = w * h > MAX_TOTAL_PX;
                    ui.label(RichText::new(tr_format!("{w:.0} × {h:.0} px voor {} × {} m{}", "{w:.0} × {h:.0} px for {} × {} m{}", site.width, site.height, if too_big { tr(" — te groot: verhoog de resolutie of verklein het gebied", " — too large: increase metres per pixel or reduce the area") } else { "" }))
                        .color(if too_big { ERR_RED } else { ui.visuals().weak_text_color() }));
                    ui.end_row();
                    ui.label(tr("Vervagen", "Fade"));
                    ui.add(egui::Slider::new(&mut self.raster.fade, 0..=80).suffix(" %"));
                    ui.end_row();
                    ui.label(tr("Laag in tekening", "Layer in drawing"));
                    let name = match (&self.raster.server, &self.raster.layer) {
                        (Some(s), _) if s.kind == ServerKind::Xyz => crate::emit::layer_name("GIS", &s.name),
                        (_, Some(l)) => crate::emit::layer_name("GIS-WMS", &l.title),
                        _ => "—".into(),
                    };
                    ui.label(RichText::new(name).monospace());
                    ui.end_row();
                });
            });
        }

    }

    // ── Vectordata (WFS) ────────────────────────────────────────────────────
    fn wfs(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.wfs.caps_job) {
            match r {
                Ok(c) => {
                    let pre = self.wfs.server.as_ref().map(|s| s.default_layer.clone()).unwrap_or_default();
                    self.wfs.ft = c.types.iter().find(|t| t.name == pre).or(c.types.first()).cloned();
                    self.message = Some((true, tr_format!("{} objecttypen (WFS {}).", "{} feature types (WFS {}).", c.types.len(), c.version)));
                    self.wfs.caps = Some(c);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.wfs.fetch) {
            match r {
                Ok((res, lines)) => {
                    let names = vector::property_names(&res.features);
                    const HINTS: &[&str] = &["perceelnummer", "huisnummer", "hectomtrng", "kmGeocodeT", "stt_naam", "naam", "name"];
                    self.wfs.label = HINTS.iter().find(|h| names.iter().any(|n| n == *h)).map(|h| h.to_string()).unwrap_or_default();
                    let color = aci_color(self.wfs.aci);
                    self.wfs.preview = lines.into_iter().map(|l| (l, color)).collect();
                    self.message = Some((true, tr_format!("{} objecten opgehaald{} — zie de kaart.", "{} features fetched{} — see the map.", res.features.len(), if res.truncated { tr(" (limiet bereikt)", " (limit reached)") } else { "" })));
                    self.wfs.result = Some(res);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.wfs.place) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(ui, "vector", tr("2D-vectordata laden met WFS", "Load 2D vector data with WFS"), tr("Percelen, panden, wegvakken, spoor en andere objecten als lijnen met labels.", "Parcels, buildings, road sections, railways and other features as lines with labels."));
        let Some(site) = self.saved_site(ui) else { return };
        let preview = std::mem::take(&mut self.wfs.preview);
        self.map_section(ui, Some(&site), None, None, &preview);
        self.wfs.preview = preview;

        group(ui, tr("Webserver / verzoek", "Web server / request"), |ui| {
            egui::Grid::new("wfs").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label(tr("Voorgedefinieerde server", "Predefined server"));
                if let Some(s) = server_combo(ui, "wfs_srv", &self.servers, &[ServerKind::Wfs], self.wfs.server.as_ref()) {
                    self.wfs.caps = None;
                    self.wfs.ft = None;
                    self.wfs.result = None;
                    self.wfs.preview.clear();
                    let url = s.url.clone();
                    self.wfs.caps_job = Some(Job::spawn(&ctx, tr("Capabilities ophalen…", "Fetching capabilities…"), move |_| caps::fetch_wfs(&url)));
                    self.wfs.server = Some(s);
                }
                ui.end_row();
                if let Some(j) = &self.wfs.caps_job {
                    ui.label("");
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(j.status());
                    });
                    ui.end_row();
                }
                if let Some(_c) = &self.wfs.caps {
                    ui.label(tr("Objecttype", "Feature type"));
                    ui.label(RichText::new(self.wfs.ft.as_ref().map(|t| format!("{}  ({})", t.title, t.name)).unwrap_or_else(|| tr("— kies hieronder —", "— choose below —").into())).strong());
                    ui.end_row();
                    ui.label(tr("Max. aantal objecten", "Maximum number of features"));
                    ui.add(egui::DragValue::new(&mut self.wfs.limit).speed(100).range(1..=200_000));
                    ui.end_row();
                }
            });
            if let Some(c) = &self.wfs.caps {
                ui.add_space(4.0);
                ui.add(egui::TextEdit::singleline(&mut self.wfs.filter).hint_text(tr_format!("Filter {} objecttypen…", "Filter {} feature types…", c.types.len())).desired_width(f32::INFINITY));
                let mut chosen = None;
                list_frame(ui, 150.0, "wfs_types", |ui| {
                    let f = self.wfs.filter.to_lowercase();
                    for t in c.types.iter().filter(|t| f.is_empty() || t.name.to_lowercase().contains(&f) || t.title.to_lowercase().contains(&f)) {
                        let sel = self.wfs.ft.as_ref().is_some_and(|x| x.name == t.name);
                        if ui.selectable_label(sel, format!("{}   ({})", t.title, t.name)).clicked() {
                            chosen = Some(t.clone());
                        }
                    }
                });
                if let Some(t) = chosen {
                    self.wfs.ft = Some(t);
                    self.wfs.result = None;
                    self.wfs.preview.clear();
                }
            }
        });

        group(ui, tr("Import-instellingen", "Import settings"), |ui| {
            egui::Grid::new("wfs_imp").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label(tr("Label (attribuut)", "Label (attribute)"));
                let names = self.wfs.result.as_ref().map(|r| vector::property_names(&r.features)).unwrap_or_default();
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("wfs_label").selected_text(if self.wfs.label.is_empty() { tr("— geen —", "— none —").to_string() } else { self.wfs.label.clone() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.wfs.label, String::new(), tr("— geen —", "— none —"));
                        for n in &names {
                            ui.selectable_value(&mut self.wfs.label, n.clone(), n);
                        }
                    });
                    if names.is_empty() {
                        ui.label(RichText::new(tr("eerst Test om attributen te zien", "Use Test to see attributes first")).small().weak());
                    }
                });
                ui.end_row();
                ui.label(tr("Teksthoogte", "Text height"));
                ui.add(egui::DragValue::new(&mut self.wfs.text_m).speed(0.1).range(0.05..=100.0).suffix(" m"));
                ui.end_row();
                ui.label(tr("Lijnkleur (laag)", "Line colour (layer)"));
                if aci_combo(ui, "wfs_aci", &mut self.wfs.aci) {
                    let c = aci_color(self.wfs.aci);
                    self.wfs.preview.iter_mut().for_each(|(_, col)| *col = c);
                }
                ui.end_row();
                ui.label(tr("Begrenzing", "Bounds"));
                ui.checkbox(&mut self.wfs.clip, tr("Bijsnijden op het importgebied", "Clip to the import area"));
                ui.end_row();
            });
        });

    }

    // ── OpenStreetMap ───────────────────────────────────────────────────────
    fn osm(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.osm.fetch) {
            match r {
                Ok(f) => {
                    let n: usize = f.1.iter().map(Vec::len).sum();
                    self.message = Some((true, tr_format!("{n} OpenStreetMap-objecten opgehaald — zie de kaart.", "{n} OpenStreetMap features fetched — see the map.")));
                    self.osm.fetched = Some(f);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.osm.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(ui, "globe", tr("OpenStreetMap importeren", "Import OpenStreetMap"), tr("Wereldwijd: gebouwen, wegen, spoor, water, kunstwerken, leidingen en meer — elk thema op een eigen laag.", "Worldwide buildings, roads, railways, water, structures, utilities and more, with each theme on its own layer."));
        let Some(site) = self.saved_site(ui) else { return };
        let fetched = self.osm.fetched.take();
        self.map_section(ui, Some(&site), None, None, fetched.as_ref().map(|f| f.2.as_slice()).unwrap_or(&[]));
        self.osm.fetched = fetched;

        group(ui, tr("Thema's", "Themes"), |ui| {
            egui::Grid::new("osm").num_columns(3).spacing([14.0, 5.0]).show(ui, |ui| {
                for (i, t) in THEMES.iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, aci_color(t.aci));
                    ui.checkbox(&mut self.osm.on[i], tr(t.label, t.label_en));
                    ui.label(RichText::new(t.layer).monospace().small().weak());
                    ui.end_row();
                }
            });
        });
        group(ui, tr("Import-instellingen", "Import settings"), |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.osm.labels, tr("Namen en huisnummers als tekst", "Names and house numbers as text"));
                ui.add(egui::DragValue::new(&mut self.osm.text_m).speed(0.1).range(0.1..=50.0).prefix(tr("hoogte ", "height ")).suffix(" m"));
            });
            ui.checkbox(&mut self.osm.clip, tr("Bijsnijden op het importgebied", "Clip to the import area"));
            ui.label(RichText::new(tr("Data © OpenStreetMap-bijdragers (ODbL), via de Overpass API. Publieke Overpass-servers zijn soms druk; ophalen kan een minuut duren.", "Data © OpenStreetMap contributors (ODbL), via the Overpass API. Public Overpass servers can be busy. Fetching data can take a minute.")).small().weak());
        });

    }

    // ── PDOK (NL) ───────────────────────────────────────────────────────────
    fn pdok(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.pdok.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "pdok", tr("Nederlandse basisregistraties importeren (PDOK)", "Import Dutch reference datasets (PDOK)"), tr("Luchtfoto, kadaster, BAG, NWB en ProRail in één keer — kies wat u nodig hebt.", "Choose the aerial imagery, cadastre, BAG, NWB and ProRail data to import together."));
        let Some(site) = self.saved_site(ui) else { return };
        self.map_section(ui, Some(&site), None, None, &[]);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, tr("Let op: de projectlocatie ligt buiten Nederland; PDOK levert hier geen data.", "The project location is outside the Netherlands, where PDOK does not provide data."));
        }
        let mut groups: Vec<&str> = Vec::new();
        for d in PDOK {
            let group = tr(d.group, d.group_en);
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        ui.columns(groups.len().max(1), |cols| {
            for (col, g) in cols.iter_mut().zip(&groups) {
                group(col, g, |ui| {
                    for (i, d) in PDOK.iter().enumerate().filter(|(_, d)| tr(d.group, d.group_en) == *g) {
                        ui.checkbox(&mut self.pdok.on[i], tr(d.label, d.label_en));
                    }
                });
            }
        });
    }

    // ── Coördinaten ─────────────────────────────────────────────────────────
    fn coords(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.coords.job) {
            match r {
                Ok(info) => {
                    if let Ok(q) = crs::transform(&self.coords.crs, &info.code, self.coords.xy) {
                        self.coords.xy = q;
                    }
                    self.coords.crs = info.code;
                }
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "point", tr("Coördinaten omrekenen", "Convert coordinates"), tr("Klik op de kaart, typ een coördinaat of gebruik \"Punt info\" in het lint om een punt in de tekening aan te wijzen.", "Click the map, enter coordinates or use Point info on the ribbon to pick a point in the drawing."));
        let site = bridge::site();
        let marker = self.coords.picked.or_else(|| crs::transform(&self.coords.crs, "EPSG:4326", self.coords.xy).ok());
        if let Some(MapEvent::Clicked(ll)) = self.map_section(ui, site.as_ref(), marker, None, &[]) {
            self.coords.picked = Some(ll);
            if let Ok(q) = crs::transform("EPSG:4326", &self.coords.crs, ll) {
                self.coords.xy = q;
            }
        }
        group(ui, tr("Invoer", "Input"), |ui| {
            let mut codes: Vec<String> = crs::builtin_list().into_iter().map(|i| i.code).collect();
            if let Some(s) = &site {
                if !codes.contains(&s.crs) {
                    codes.push(s.crs.clone());
                }
            }
            let geo = crs::is_geographic(&self.coords.crs);
            ui.horizontal(|ui| {
                ui.label(tr("Stelsel", "CRS"));
                let mut chosen = None;
                egui::ComboBox::from_id_salt("coord_crs").width(320.0).selected_text(crs::known(&self.coords.crs).map(|i| format!("{} — {}", i.code, i.name)).unwrap_or(self.coords.crs.clone())).show_ui(ui, |ui| {
                    for c in &codes {
                        if let Some(i) = crs::known(c) {
                            if ui.selectable_label(*c == self.coords.crs, format!("{} — {}", i.code, i.name)).clicked() {
                                chosen = Some(c.clone());
                            }
                        }
                    }
                });
                if let Some(c) = chosen {
                    self.coords.job = Some(Job::spawn(&ctx, "…", move |_| crs::ensure(&c)));
                }
                let a = ui.add(egui::DragValue::new(&mut self.coords.xy[0]).speed(if geo { 0.0001 } else { 1.0 }).fixed_decimals(if geo { 8 } else { 3 }).prefix(if geo { "lon " } else { "X " }));
                let b = ui.add(egui::DragValue::new(&mut self.coords.xy[1]).speed(if geo { 0.0001 } else { 1.0 }).fixed_decimals(if geo { 8 } else { 3 }).prefix(if geo { "lat " } else { "Y " }));
                if a.changed() || b.changed() {
                    self.coords.picked = None;
                }
            });
        });
        group(ui, tr("Resultaat", "Result"), |ui| {
            let mut targets: Vec<String> = vec!["EPSG:4326".into(), "EPSG:28992".into(), "EPSG:3857".into()];
            if let Some(s) = &site {
                targets.insert(1, s.crs.clone());
            }
            if let Ok(ll) = crs::transform(&self.coords.crs, "EPSG:4326", self.coords.xy) {
                targets.push(crs::utm_code_for(ll[0], ll[1]));
            }
            targets.dedup();
            egui::Grid::new("coord_out").num_columns(3).striped(true).spacing([18.0, 4.0]).show(ui, |ui| {
                for t in targets {
                    let Some(info) = crs::known(&t) else { continue };
                    ui.label(format!("{} — {}", info.code, info.name));
                    match crs::transform(&self.coords.crs, &t, self.coords.xy) {
                        Ok(p) if crs::is_geographic(&t) => {
                            ui.monospace(format!("lon {:.8}", p[0]));
                            ui.monospace(format!("lat {:.8}", p[1]));
                        }
                        Ok(p) => {
                            ui.monospace(format!("X {:.3}", p[0]));
                            ui.monospace(format!("Y {:.3}", p[1]));
                        }
                        Err(_) => {
                            ui.label("—");
                            ui.label("—");
                        }
                    }
                    ui.end_row();
                }
            });
        });
    }

    fn location_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let busy = self.loc.apply.is_some() || self.loc.crs_job.is_some() || self.loc.search.is_some();
        button_bar(ui, |ui| {
            if ui.add_enabled(!busy, primary_button("OK")).on_hover_text(tr("Projectlocatie opslaan in de tekening", "Save the project location in the drawing")).clicked() {
                match bridge::save_site(&self.draft) {
                    Ok(()) => {
                        self.has_site = true;
                        let site = self.draft.clone();
                        let (origin, aerial) = (self.loc.geo_origin, self.loc.aerial);
                        let summary = tr_format!("Projectlocatie opgeslagen: {} — {} X {:.2} Y {:.2}, {}×{} m.", "Project location saved: {} — {} X {:.2} Y {:.2}, {}×{} m.", site.label, site.crs, site.origin_e, site.origin_n, site.width, site.height);
                        if origin || aerial {
                            self.loc.apply = Some(Job::spawn(&ctx, tr("Bezig…", "Working…"), move |p| {
                                let mut out = vec![summary];
                                if origin {
                                    out.push(import::geo_origin(&site)?);
                                }
                                if aerial {
                                    out.push(import::aerial(&site, p)?);
                                }
                                Ok(out.join(" "))
                            }));
                        } else {
                            self.ok(summary);
                        }
                    }
                    Err(e) => self.err(tr_format!("Opslaan mislukt: {e}", "Save failed: {e}")),
                }
            }
            if ui.button(tr("Annuleren", "Cancel")).on_hover_text(tr("Wijzigingen terugdraaien en het venster sluiten", "Discard changes and close the window")).clicked() {
                // As in the FreeCAD dialogs: drop the changes and close.
                self.reload_site();
                self.loc.query.clear();
                self.loc.hits.clear();
                self.loc.popup = false;
                self.loc.highlight = None;
                self.message = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            job_status(ui, &self.loc.apply);
            job_status(ui, &self.loc.search);
        });
    }

    fn raster_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        let too_big = (site.width / self.raster.resolution) * (site.height / self.raster.resolution) > MAX_TOTAL_PX;
        let is_xyz = self.raster.server.as_ref().is_some_and(|s| s.kind == ServerKind::Xyz);
        let ready = self.raster.server.is_some() && (is_xyz || self.raster.layer.is_some());
        button_bar(ui, |ui| {
            if ui.add_enabled(ready && !too_big && self.raster.run.is_none(), primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let server = self.raster.server.clone().unwrap();
                let opt = raster::Options { resolution: self.raster.resolution, format: self.raster.format.clone() };
                let (fade, caps, layer, s) = (self.raster.fade, self.raster.caps.clone(), self.raster.layer.clone(), site.clone());
                self.raster.run = Some(Job::spawn(&ctx, tr("Importeren…", "Importing…"), move |p| match server.kind {
                    ServerKind::Xyz => import::xyz(&s, &server, &opt, fade, p),
                    _ => import::wms(&s, &server, caps.as_ref().ok_or(tr("geen capabilities", "no capabilities"))?, layer.as_ref().ok_or(tr("geen laag", "no layer"))?, &opt, fade, p),
                }));
            }
            if ui.add_enabled(ready && self.raster.preview_job.is_none(), egui::Button::new(tr("Test — voorbeeld op kaart", "Test — preview on map"))).clicked() {
                let (server, caps, layer, fmt, s) = (self.raster.server.clone().unwrap(), self.raster.caps.clone(), self.raster.layer.clone(), self.raster.format.clone(), site.clone());
                self.raster.preview_job = Some(Job::spawn(&ctx, tr("Voorbeeld ophalen…", "Fetching preview…"), move |_| match server.kind {
                    ServerKind::Xyz => raster::preview_xyz(&s, &server),
                    _ => raster::preview_wms(&s, caps.as_ref().ok_or(tr("geen capabilities", "no capabilities"))?, layer.as_ref().ok_or(tr("geen laag", "no layer"))?, &fmt, 900),
                }));
            }
            if self.raster.preview.is_some() && ui.button(tr("Voorbeeld wissen", "Clear preview")).clicked() {
                self.raster.preview = None;
            }
            job_status(ui, &self.raster.run);
            job_status(ui, &self.raster.preview_job);
        });
    }

    fn wfs_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        button_bar(ui, |ui| {
            let can = self.wfs.ft.is_some() && self.wfs.place.is_none() && self.wfs.fetch.is_none();
            if ui.add_enabled(can, primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let style = VectorStyle { aci: self.wfs.aci, label_field: (!self.wfs.label.is_empty()).then(|| self.wfs.label.clone()), text_m: self.wfs.text_m, clip: self.wfs.clip };
                let (s, ft, caps, limit, cached) = (site.clone(), self.wfs.ft.clone().unwrap(), self.wfs.caps.clone(), self.wfs.limit, self.wfs.result.take());
                self.wfs.preview.clear();
                self.wfs.place = Some(Job::spawn(&ctx, tr("Importeren…", "Importing…"), move |p| {
                    let res = match cached {
                        Some(r) => r,
                        None => import::wfs_fetch(&s, caps.as_ref().ok_or(tr("geen capabilities", "no capabilities"))?, &ft, limit, p)?,
                    };
                    import::wfs_place(&s, &ft, &res, &style)
                }));
            }
            if ui.add_enabled(can, egui::Button::new(tr("Test — ophalen en tonen", "Test — fetch and display"))).clicked() {
                let (s, c, t, n) = (site.clone(), self.wfs.caps.clone().unwrap(), self.wfs.ft.clone().unwrap(), self.wfs.limit);
                self.wfs.fetch = Some(Job::spawn(&ctx, tr("Objecten ophalen…", "Fetching features…"), move |p| {
                    let res = import::wfs_fetch(&s, &c, &t, n, p)?;
                    let lines = vector::preview_lines(&s, &res.data_crs, &res.features)?;
                    Ok((res, lines))
                }));
            }
            job_status(ui, &self.wfs.place);
            job_status(ui, &self.wfs.fetch);
        });
    }

    fn osm_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        let ids: Vec<&'static str> = THEMES.iter().zip(&self.osm.on).filter(|(_, on)| **on).map(|(t, _)| t.id).collect();
        let busy = self.osm.fetch.is_some() || self.osm.run.is_some();
        button_bar(ui, |ui| {
            if ui.add_enabled(!ids.is_empty() && !busy, primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let cached = self.osm.fetched.take().filter(|f| f.0 == ids).map(|f| f.1);
                let (s, ids2, labels, h, clip) = (site.clone(), ids.clone(), self.osm.labels, self.osm.text_m, self.osm.clip);
                self.osm.run = Some(Job::spawn(&ctx, "OpenStreetMap…", move |p| {
                    let per_theme = match cached {
                        Some(c) => c,
                        None => import::osm_fetch(&s, &ids2, p)?,
                    };
                    import::osm_place(&s, &ids2, &per_theme, labels, h, clip)
                }));
            }
            if ui.add_enabled(!ids.is_empty() && !busy, egui::Button::new(tr("Test — ophalen en tonen", "Test — fetch and display"))).clicked() {
                let (s, ids2) = (site.clone(), ids.clone());
                self.osm.fetch = Some(Job::spawn(&ctx, tr("OpenStreetMap ophalen…", "Fetching OpenStreetMap data…"), move |p| {
                    let per_theme = import::osm_fetch(&s, &ids2, p)?;
                    let mut lines = Vec::new();
                    for (id, feats) in ids2.iter().zip(&per_theme) {
                        let color = THEMES.iter().find(|t| t.id == *id).map(|t| aci_color(t.aci)).unwrap_or(Color32::GRAY);
                        lines.extend(vector::preview_lines(&s, "EPSG:4326", feats)?.into_iter().map(|l| (l, color)));
                    }
                    Ok((ids2, per_theme, lines))
                }));
            }
            job_status(ui, &self.osm.run);
            job_status(ui, &self.osm.fetch);
        });
    }

    fn pdok_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        button_bar(ui, |ui| {
            let chosen: Vec<usize> = (0..PDOK.len()).filter(|i| self.pdok.on[*i]).collect();
            if ui.add_enabled(!chosen.is_empty() && self.pdok.run.is_none(), primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let s = site.clone();
                self.pdok.run = Some(Job::spawn(&ctx, "PDOK…", move |p| {
                    let mut done = Vec::new();
                    for (k, i) in chosen.iter().enumerate() {
                        let d = &PDOK[*i];
                        let label = tr(d.label, d.label_en);
                        p(format!("{}/{}: {}…", k + 1, chosen.len(), label));
                        match import::run_dataset(&s, &d.data, &|m: String| p(format!("{}/{}: {} — {m}", k + 1, chosen.len(), label))) {
                            Ok(_) => done.push(label.to_string()),
                            Err(e) => done.push(tr_format!("{} (mislukt: {e})", "{} (failed: {e})", label)),
                        }
                    }
                    Ok(tr_format!("PDOK geïmporteerd: {}.", "PDOK data imported: {}.", done.join("; ")))
                }));
            }
            if let Some(j) = &self.pdok.run {
                ui.spinner();
                ui.label(j.status());
            }
        });
    }

    // ── BGT · Kadaster · KLIC (NLCS) ────────────────────────────────────────
    fn load_klic(&mut self, ctx: &egui::Context) {
        let path = std::path::PathBuf::from(self.nlcs.klic_path.trim().trim_matches('"'));
        let Some(site) = bridge::site().or_else(|| Some(self.draft.clone())) else { return };
        self.nlcs.klic = None;
        self.nlcs.klic_preview.clear();
        self.nlcs.klic_job = Some(Job::spawn(ctx, tr("KLIC-levering lezen…", "Reading KLIC delivery…"), move |_| {
            let lev = klic::read(&path)?;
            let preview = klic_preview(&site, &lev)?;
            Ok((lev, preview))
        }));
    }

    fn nlcs_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.nlcs.klic_job) {
            match r {
                Ok((lev, preview)) => {
                    self.message = Some((true, tr_format!("KLIC {}: {} objecten van {} netbeheerders.", "KLIC {}: {} features from {} network operators.", lev.meldnummer, lev.features.len(), lev.operators.len())));
                    self.nlcs.klic_on = lev.themes.iter().map(|(t, n)| (t.clone(), *n, true)).collect();
                    self.nlcs.klic = Some(Arc::new(lev));
                    self.nlcs.klic_preview = preview;
                }
                Err(e) => self.err(format!("KLIC: {e}")),
            }
        }
        if let Some(r) = poll(&mut self.nlcs.fetch) {
            match r {
                Ok(f) => {
                    let n: usize = f.2.iter().map(|(_, v)| v.len()).sum();
                    self.message = Some((true, tr_format!("{n} BGT-objecten opgehaald — zie de kaart.", "{n} BGT features fetched — see the map.")));
                    self.nlcs.fetched = Some(f);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.nlcs.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }
        // A delivery dropped on the window.
        let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.into_iter().next() {
            self.nlcs.klic_path = p.to_string_lossy().into_owned();
            let ctx = ui.ctx().clone();
            self.load_klic(&ctx);
        }

        page_header(ui, "nlcs", tr("BGT, kadaster en KLIC conform NLCS", "BGT, cadastre and KLIC in NLCS format"), tr("Basisdata voor een infra-ontwerp: elk object op zijn NLCS-laag, met kleur, lijndikte, lijntype en objectgegevens.", "Reference data for infrastructure design. Each feature is placed on its NLCS layer with colour, lineweight, linetype and attributes."));
        let Some(site) = self.saved_site(ui) else { return };
        let mut lines: Preview2 = self.nlcs.fetched.as_ref().map(|f| f.4.clone()).unwrap_or_default();
        lines.extend(self.nlcs.klic_preview.iter().cloned());
        self.map_section(ui, Some(&site), None, None, &lines);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, tr("Let op: BGT, BRK en KLIC bestaan alleen voor Nederland.", "BGT, BRK and KLIC cover the Netherlands only."));
        }

        group(ui, tr("Tekenstandaard NLCS 5.02", "Drawing standard NLCS 5.02"), |ui| {
            egui::Grid::new("nlcs_std").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(tr("Tekenstatus", "Drawing status"));
                egui::ComboBox::from_id_salt("nlcs_status").selected_text(drawing_status(self.nlcs.status)).show_ui(ui, |ui| {
                    for st in Status::ALL {
                        ui.selectable_value(&mut self.nlcs.status, st, drawing_status(st));
                    }
                });
                ui.label(tr("Schaal", "Scale"));
                egui::ComboBox::from_id_salt("nlcs_scale").selected_text(format!("1:{}", self.nlcs.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0] {
                        ui.selectable_value(&mut self.nlcs.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.end_row();
                let disciplines = nlcs::disciplines();
                let combo = |ui: &mut egui::Ui, id: &str, value: &mut String| {
                    let current = disciplines.iter().find(|d| d.0 == value.as_str()).map(|d| format!("{} — {}", d.0, d.1)).unwrap_or(value.clone());
                    egui::ComboBox::from_id_salt(id).width(260.0).selected_text(current).show_ui(ui, |ui| {
                        for (c, d) in &disciplines {
                            ui.selectable_value(value, c.to_string(), format!("{c} — {d}"));
                        }
                    });
                };
                ui.label(tr("Discipline BGT/BRK", "BGT/BRK discipline"));
                combo(ui, "nlcs_disc", &mut self.nlcs.discipline);
                ui.label(tr("Discipline KLIC", "KLIC discipline"));
                combo(ui, "nlcs_disc_klic", &mut self.nlcs.klic_discipline);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.nlcs.object_data, tr("Objectgegevens (XDATA)", "Feature attributes (XDATA)"));
                ui.checkbox(&mut self.nlcs.labels, tr("Teksten: straatnamen, huisnummers, perceelnummers, leidinglabels", "Labels: street names, house numbers, parcel numbers, utility lines"));
                ui.checkbox(&mut self.nlcs.clip, tr("Bijsnijden op het importgebied", "Clip to the import area"));
            });
            ui.label(RichText::new(tr("Laagnaam: STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT, bijv. B-XX-VH-GESLOTENVERHARDING_ASFALT-G. Lijntypen in papier-mm met de lijntypeschaal van de tekenschaal; teksten krijgen de stijl van hun objectlaag, de teksten van het tekenblad volgen de NLCS-tekstklassen.", "Layer name: STATUS-DISCIPLINE-MAIN GROUP-OBJECT-ELEMENT, e.g. B-XX-VH-GESLOTENVERHARDING_ASFALT-G. Linetypes use paper millimetres and scale with the drawing. Labels use their object layer's text style. Sheet text follows NLCS text classes.")).small().weak());
        });

        ui.columns(2, |cols| {
            group(&mut cols[0], tr("BGT (PDOK, actueel)", "BGT (PDOK, current)"), |ui| {
                for (i, g) in bgt::GROUPS.iter().enumerate() {
                    ui.checkbox(&mut self.nlcs.bgt_on[i], tr(g.label, g.label_en));
                }
            });
            group(&mut cols[1], tr("Kadastrale kaart (BRK)", "Cadastral map (BRK)"), |ui| {
                ui.checkbox(&mut self.nlcs.brk, tr("Perceelgrenzen en -nummers (KG-GRENS_PERCEEL)", "Parcel boundaries and numbers (KG-GRENS_PERCEEL)"));
            });
            group(&mut cols[1], tr("Tekenblad", "Drawing sheet"), |ui| {
                ui.checkbox(&mut self.nlcs.sheet, tr("Kader, titelblok, legenda, noordpijl, schaalbalk en RD-kruisen", "Border, title block, legend, north arrow, scale bar and RD grid crosses"));
                let (paper, _, _) = tekenblad::paper_for(&site, self.nlcs.scale);
                ui.label(RichText::new(tr_format!("Formaat bij 1:{}: {paper} liggend", "Paper at 1:{}: {paper} landscape", self.nlcs.scale)).small().weak());
                egui::Grid::new("sheet").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Project");
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.project).hint_text(site.label.as_str()).desired_width(240.0));
                    ui.end_row();
                    ui.label(tr("Onderwerp", "Subject"));
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.subject).desired_width(240.0));
                    ui.end_row();
                    ui.label(tr("Tekeningnummer", "Drawing number"));
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.number).desired_width(240.0));
                    ui.end_row();
                    ui.label(tr("Getekend", "Drawn by"));
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.drawn_by).desired_width(240.0));
                    ui.end_row();
                });
            });
        });

        group(ui, tr("KLIC-levering (WIBON, IMKL 2.0)", "KLIC delivery (WIBON, IMKL 2.0)"), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Bestand", "File"));
                ui.add(egui::TextEdit::singleline(&mut self.nlcs.klic_path).hint_text(tr("Levering_….zip of GI_gebiedsinformatielevering_….xml — of sleep het bestand op dit venster", "Levering_….zip or GI_gebiedsinformatielevering_….xml — or drop the file on this window")).desired_width(ui.available_width() - 90.0));
                if ui.add_enabled(!self.nlcs.klic_path.trim().is_empty() && self.nlcs.klic_job.is_none(), egui::Button::new(tr("Lezen", "Read"))).clicked() {
                    let ctx = ui.ctx().clone();
                    self.load_klic(&ctx);
                }
            });
            job_status(ui, &self.nlcs.klic_job);
            if let Some(lev) = self.nlcs.klic.clone() {
                ui.label(RichText::new(tr_format!("Melding {} · {} objecten · {}", "Request {} · {} features · {}", lev.meldnummer, lev.features.len(), lev.operators.join(", "))).small());
                egui::Grid::new("klic_themes").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                    for (i, (t, n, on)) in self.nlcs.klic_on.iter_mut().enumerate() {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, preview_color(nlcs::imkl_rgb(t)));
                        ui.checkbox(on, format!("{} ({n})", klic::theme_label(t)));
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.nlcs.imkl_colors, tr("IMKL-themakleuren in plaats van NLCS-kleuren", "Use IMKL theme colours instead of NLCS colours"));
                    if ui.button(tr("Importgebied op deze levering zetten", "Set import area to this delivery")).clicked() {
                        if let Some(b) = klic_extent(&lev) {
                            let mut s = site.clone();
                            let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                            let c = if s.crs == "EPSG:28992" { Ok(c) } else { crs::transform("EPSG:28992", &s.crs, c) };
                            if let Ok(c) = c {
                                s.set_origin((c[0] * 10.0).round() / 10.0, (c[1] * 10.0).round() / 10.0);
                                s.width = ((b[2] - b[0]) + 40.0).ceil();
                                s.height = ((b[3] - b[1]) + 40.0).ceil();
                                s.label = format!("KLIC {}", lev.meldnummer);
                                match bridge::save_site(&s) {
                                    Ok(()) => {
                                        self.draft = s;
                                        self.fit_pending = true;
                                        self.nlcs.fetched = None;
                                        self.ok(tr("Importgebied aangepast aan de KLIC-levering.", "Import area updated to match the KLIC delivery."));
                                    }
                                    Err(e) => self.err(e),
                                }
                            }
                        }
                    }
                });
                // The letters, conditions and sketches of the operators, folded away.
                if !lev.extras.attachments.is_empty() {
                    egui::CollapsingHeader::new(tr_format!("Bijlagen ({})", "Attachments ({})", lev.extras.attachments.len())).id_salt("klic_bijlagen").show(ui, |ui| {
                        egui::Grid::new("klic_attachments").num_columns(3).striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
                            for a in &lev.extras.attachments {
                                let name = a.path.rsplit('/').next().unwrap_or(&a.path).to_string();
                                ui.label(RichText::new(format!("{} · {}", a.operator, a.kind)).small().weak());
                                ui.label(RichText::new(&name).small()).on_hover_text(&a.path);
                                if ui.small_button(tr("Openen", "Open")).clicked() {
                                    let (src, rel) = (lev.source.clone(), a.path.clone());
                                    let dir = crate::gis::servers::config_dir().join("klic-bijlagen").join(&lev.meldnummer);
                                    std::thread::spawn(move || {
                                        let r = crate::gis::klic_extra::attachment_file(&src, &rel, &dir).and_then(|p| crate::gis::klic_extra::open_file(&p));
                                        if let Err(e) = r {
                                            crate::bridge::error(&e);
                                        }
                                    });
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
            }
        });
    }

    fn nlcs_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        let collections = self.nlcs.collections();
        let brk = self.nlcs.brk;
        let klic_ready = self.nlcs.klic.is_some() && self.nlcs.klic_on.iter().any(|t| t.2);
        let busy = self.nlcs.fetch.is_some() || self.nlcs.run.is_some();
        let anything = !collections.is_empty() || brk || klic_ready;
        button_bar(ui, |ui| {
            if ui.add_enabled(anything && !busy, primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let cached = self.nlcs.fetched.take().filter(|f| f.0 == collections && f.1 == brk);
                let (opt, kopt, lev, sheet, info) = (self.nlcs.options(), self.nlcs.klic_options(), self.nlcs.klic.clone().filter(|_| klic_ready), self.nlcs.sheet, self.nlcs.info.clone());
                let (s, cols) = (site.clone(), collections.clone());
                self.nlcs.run = Some(Job::spawn(&ctx, tr("Basisdata…", "Reference data…"), move |p| {
                    let mut done = Vec::new();
                    let (bgt_data, cad) = match cached {
                        Some(c) => (c.2, c.3),
                        None => {
                            let b = if cols.is_empty() { Arc::new(Vec::new()) } else { Arc::new(bgt::fetch(&s, &cols, p)?) };
                            let c = if brk { Some(Arc::new(basisdata::cadastre_fetch(&s, opt.labels, p)?)) } else { None };
                            (b, c)
                        }
                    };
                    if !bgt_data.is_empty() {
                        p(tr("BGT tekenen…", "Drawing BGT features…").into());
                        done.push(basisdata::bgt_place(&s, &bgt_data, &opt)?);
                    }
                    if let Some(c) = cad {
                        p(tr("Kadaster tekenen…", "Drawing cadastral features…").into());
                        done.push(basisdata::cadastre_place(&s, &c, &opt)?);
                    }
                    if let Some(lev) = lev {
                        p(tr("KLIC tekenen…", "Drawing KLIC features…").into());
                        done.push(basisdata::klic_place(&s, &lev, &kopt)?);
                    }
                    if sheet {
                        p(tr("Tekenblad…", "Drawing sheet…").into());
                        done.push(tekenblad::draw(&s, &opt, &info)?);
                    }
                    bridge::zoom_extents();
                    Ok(done.join(" "))
                }));
            }
            if ui.add_enabled((!collections.is_empty() || brk) && !busy, egui::Button::new(tr("Test — ophalen en tonen", "Test — fetch and display"))).clicked() {
                let (s, cols, labels) = (site.clone(), collections.clone(), self.nlcs.labels);
                self.nlcs.fetch = Some(Job::spawn(&ctx, tr("BGT ophalen…", "Fetching BGT data…"), move |p| {
                    let data = if cols.is_empty() { Vec::new() } else { bgt::fetch(&s, &cols, p)? };
                    let cad = if brk { Some(basisdata::cadastre_fetch(&s, labels, p)?) } else { None };
                    let preview = bgt_preview(&s, &data, cad.as_ref())?;
                    Ok((cols, brk, Arc::new(data), cad.map(Arc::new), preview))
                }));
            }
            job_status(ui, &self.nlcs.run);
            job_status(ui, &self.nlcs.fetch);
        });
    }

    // ── AHN · Profiel · Puntenwolk ──────────────────────────────────────────
    fn ahn_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.ahn.test) {
            match r {
                Ok(p) => {
                    self.message = Some((true, tr_format!("AHN: {} – {} m NAP, {} hoogtelijnen — zie de kaart.", "AHN: {} – {} m NAP, {} contours — see the map.", hoogte::nap(p.1 .0, 2), hoogte::nap(p.1 .1, 2), p.0.len())));
                    self.ahn.preview = Some(p);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.ahn.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "ahn", tr("AHN-hoogtes, profielen en puntenwolk", "AHN elevations, profiles and point cloud"), tr("Actueel Hoogtebestand Nederland: hoogtepunten, hoogtelijnen en lengteprofielen uit AHN4, de puntenwolk uit AHN5.", "Dutch elevation data: spot heights, contours and longitudinal profiles from AHN4, and point clouds from AHN5."));
        let Some(site) = self.saved_site(ui) else { return };
        let lines = self.ahn.preview.as_ref().map(|p| p.0.clone()).unwrap_or_default();
        self.map_section(ui, Some(&site), None, None, &lines);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, tr("Let op: AHN bestaat alleen voor Nederland.", "AHN covers the Netherlands only."));
        }
        let o = &mut self.ahn.opt;
        group(ui, tr("Tekenen", "Drawing"), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Schaal", "Scale"));
                egui::ComboBox::from_id_salt("ahn_scale").selected_text(format!("1:{}", o.nlcs.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0] {
                        ui.selectable_value(&mut o.nlcs.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.label(tr("Tekenstatus", "Drawing status"));
                egui::ComboBox::from_id_salt("ahn_status").selected_text(drawing_status(o.nlcs.status)).show_ui(ui, |ui| {
                    for st in nlcs::Status::ALL {
                        ui.selectable_value(&mut o.nlcs.status, st, drawing_status(st));
                    }
                });
                ui.checkbox(&mut o.nlcs.labels, tr("Hoogtes als tekst", "Elevation labels"));
                ui.checkbox(&mut o.nlcs.clip, tr("Bijsnijden op het importgebied", "Clip to the import area"));
            });
            ui.label(RichText::new(tr("Hoogtes in m NAP; lijnen en punten liggen op hun hoogte (Z). Lagen: OG-HOOGTEPUNT, OG-HOOGTELIJN_MINOR/MAJOR, GW-MAAIVELD, AL-PROFIELBALK.", "Elevations in metres NAP. Lines and points use their true elevation (Z). Layers: OG-HOOGTEPUNT, OG-HOOGTELIJN_MINOR/MAJOR, GW-MAAIVELD, AL-PROFIELBALK.")).small().weak());
        });
        ui.columns(2, |cols| {
            group(&mut cols[0], tr("Hoogtepunten", "Spot heights"), |ui| {
                ui.checkbox(&mut self.ahn.spots, tr("Hoogtepunten in een raster", "Spot heights on a grid"));
                ui.horizontal(|ui| {
                    ui.label(tr("Raster", "Grid"));
                    egui::ComboBox::from_id_salt("ahn_spacing").selected_text(format!("{} m", o.spot_spacing)).show_ui(ui, |ui| {
                        for v in [2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0] {
                            ui.selectable_value(&mut o.spot_spacing, v, format!("{v} m"));
                        }
                    });
                    ui.label("Model");
                    egui::ComboBox::from_id_salt("ahn_model").selected_text(elevation_model(o.model)).show_ui(ui, |ui| {
                        for m in [Model::Dtm, Model::Dsm] {
                            ui.selectable_value(&mut o.model, m, elevation_model(m));
                        }
                    });
                });
            });
            group(&mut cols[0], tr("Hoogtelijnen (maaiveld)", "Contours (ground surface)"), |ui| {
                ui.checkbox(&mut self.ahn.contours, tr("Hoogtelijnen", "Contours"));
                ui.horizontal(|ui| {
                    ui.label("Interval");
                    egui::ComboBox::from_id_salt("ahn_interval").selected_text(format!("{} m", o.interval)).show_ui(ui, |ui| {
                        for v in [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.interval, v, format!("{v} m"));
                        }
                    });
                    ui.label(tr("hoofdlijn elke", "major contour every"));
                    ui.add(egui::DragValue::new(&mut o.major_every).range(1..=20));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Rastercel", "Grid cell"));
                    egui::ComboBox::from_id_salt("ahn_cell").selected_text(format!("{} m", o.contour_cell)).show_ui(ui, |ui| {
                        for v in [0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.contour_cell, v, format!("{v} m"));
                        }
                    });
                    ui.checkbox(&mut o.smooth, tr("Gladstrijken", "Smooth"));
                });
            });
            group(&mut cols[1], tr("Lengteprofiel", "Longitudinal profile"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Stations om de", "Stations every"));
                    ui.add(egui::DragValue::new(&mut o.step).range(0.25..=50.0).speed(0.25).suffix(" m"));
                    ui.label(tr("overhoging", "vertical exaggeration"));
                    ui.add(egui::DragValue::new(&mut o.exaggeration).range(1.0..=50.0).speed(0.5).suffix("×"));
                });
                ui.checkbox(&mut o.surface, tr("Ook het oppervlak (DSM: gebouwen, bomen)", "Also include the surface (DSM: buildings, trees)"));
                ui.label(RichText::new(tr("Kies in Open CAD Studio Profiel in het lint (of GIS_PROFILE): selecteer eerst een polyline of wijs de punten aan, Enter, en wijs dan de linkeronderhoek van het profiel aan.", "Choose Profile on the Open CAD Studio ribbon or run GIS_PROFILE. Select a polyline or pick points, press Enter, then pick the lower-left corner of the profile.")).small().weak());
            });
            group(&mut cols[1], tr("Puntenwolk (COPC)", "Point cloud (COPC)"), |ui| {
                ui.checkbox(&mut self.ahn.cloud, tr("AHN-puntenwolk koppelen (.rcs naast de tekening)", "Attach AHN point cloud (.rcs beside the drawing)"));
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("ahn_ed").selected_text(o.cloud.edition.label()).show_ui(ui, |ui| {
                        for e in Edition::ALL {
                            ui.selectable_value(&mut o.cloud.edition, e, e.label());
                        }
                    });
                    ui.label(tr("puntafstand", "point spacing"));
                    egui::ComboBox::from_id_salt("ahn_spacing_pc").selected_text(format!("{} m", o.cloud.spacing)).show_ui(ui, |ui| {
                        for v in [0.1, 0.25, 0.5, 1.0, 2.0] {
                            ui.selectable_value(&mut o.cloud.spacing, v, format!("{v} m"));
                        }
                    });
                    ui.label(tr("kleur", "colour"));
                    egui::ComboBox::from_id_salt("ahn_col").selected_text(point_colouring(o.cloud.colouring)).show_ui(ui, |ui| {
                        for c in Colouring::ALL {
                            ui.selectable_value(&mut o.cloud.colouring, c, point_colouring(c));
                        }
                    });
                });
                egui::Grid::new("ahn_classes").num_columns(3).spacing([10.0, 3.0]).show(ui, |ui| {
                    for (i, (class, label, rgb)) in pointcloud::CLASSES.iter().enumerate() {
                        let label = match class {
                            2 => tr(label, "Ground"),
                            6 => tr(label, "Buildings"),
                            9 => tr(label, "Water"),
                            26 => tr(label, "Structures"),
                            1 => tr(label, "Other (including vegetation)"),
                            14 => tr(label, "Power lines"),
                            _ => label,
                        };
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        ui.checkbox(&mut self.ahn.class_on[i], label);
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                let area = site.width * site.height;
                let estimate = area * (1.0 / (o.cloud.spacing * o.cloud.spacing)).min(20.0);
                ui.label(RichText::new(tr_format!("Ongeveer {:.1} miljoen punten voor {:.0} × {:.0} m; alleen de nodige delen van de 1 km-tegels worden gedownload.", "About {:.1} million points for {:.0} × {:.0} m. Only the relevant parts of the 1 km tiles are downloaded.", estimate / 1e6, site.width, site.height)).small().weak());
            });
        });
        // Keep the shared settings in step: the ribbon buttons use them too.
        let all = self.ahn.class_on.iter().all(|c| *c);
        self.ahn.opt.cloud.classes = if all { Vec::new() } else { pointcloud::CLASSES.iter().zip(&self.ahn.class_on).filter(|(_, on)| **on).map(|(c, _)| c.0).collect() };
        hoogte::set_settings(self.ahn.opt.clone());
    }

    fn ahn_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(site) = bridge::site() else { return };
        let busy = self.ahn.test.is_some() || self.ahn.run.is_some();
        let any = self.ahn.spots || self.ahn.contours || self.ahn.cloud;
        button_bar(ui, |ui| {
            if ui.add_enabled(any && !busy, primary_button(tr("OK en importeren", "OK and import"))).clicked() {
                let (s, opt, spots, contours, cloud) = (site.clone(), self.ahn.opt.clone(), self.ahn.spots, self.ahn.contours, self.ahn.cloud);
                self.ahn.run = Some(Job::spawn(&ctx, "AHN…", move |p| {
                    let mut done = Vec::new();
                    if contours {
                        done.push(hoogte::contour_lines(&s, &opt, p)?);
                    }
                    if spots {
                        done.push(hoogte::spot_heights(&s, &opt, p)?);
                    }
                    if cloud {
                        done.push(hoogte::point_cloud(&s, &opt, p)?);
                    }
                    Ok(done.join(" "))
                }));
            }
            if ui.add_enabled(!busy, egui::Button::new(tr("Test — hoogtelijnen op de kaart", "Test — contours on map"))).clicked() {
                let (s, opt) = (site.clone(), self.ahn.opt.clone());
                self.ahn.test = Some(Job::spawn(&ctx, tr("AHN ophalen…", "Fetching AHN data…"), move |p| {
                    let b = crate::basisdata::rd_bbox(&s)?;
                    let mut grid = ahn::fetch(b, Model::Dtm, opt.contour_cell, p)?;
                    if opt.smooth {
                        grid = grid.smoothed();
                    }
                    let range = grid.range().ok_or(tr("AHN heeft hier geen hoogtes", "AHN has no elevations here"))?;
                    let lines = ahn::contours(&grid, &ahn::levels(range, opt.interval), (8.0 * opt.contour_cell).max(10.0));
                    let to = vector::data_to(&s, "EPSG:28992", "EPSG:4326", &[])?;
                    let mut out = Vec::new();
                    for (level, polys) in &lines {
                        let c = height_colour((level - range.0) / (range.1 - range.0).max(0.01));
                        for (pts, closed) in polys {
                            let mut ll: Vec<LonLat> = pts.iter().filter_map(|p| to(*p)).collect();
                            if *closed {
                                if let Some(f) = ll.first().copied() {
                                    ll.push(f);
                                }
                            }
                            out.push((ll, c));
                        }
                    }
                    Ok((out, range, grid.coverage()))
                }));
            }
            job_status(ui, &self.ahn.run);
            job_status(ui, &self.ahn.test);
        });
    }

    // ── NLCS-controle ───────────────────────────────────────────────────────
    fn start_check(&mut self, ctx: &egui::Context) {
        if self.controle.job.is_some() {
            return;
        }
        let opt = nlcs_check::Options {
            scale: self.controle.scale,
            unit_factor: bridge::target().map(|t| t.unit_factor).unwrap_or(1.0),
            text_heights: self.controle.text_heights,
            empty_layers: self.controle.empty_layers,
            layer_props: true,
        };
        self.controle.job = Some(Job::spawn(ctx, tr("Tekening controleren…", "Checking drawing…"), move |_| {
            let doc = bridge::snapshot()?;
            let opt = nlcs_check::Options { layer_props: !bridge::partial(), ..opt };
            Ok(nlcs_check::check(&doc, &opt))
        }));
    }

    fn controle_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.controle.job) {
            match r {
                Ok(rep) => {
                    self.message = Some((true, tr_format!("NLCS-controle: {} conform — {}, {}.", "NLCS check: {} compliant — {}, {}.", rep.score_label(), nlcs_check::counted(rep.count(Severity::Fout), tr("fout", "error"), tr("fouten", "errors")), nlcs_check::counted(rep.count(Severity::Waarschuwing), tr("waarschuwing", "warning"), tr("waarschuwingen", "warnings")))));
                    self.controle.report = Some(rep);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.controle.repair) {
            match r {
                Ok(m) => {
                    self.ok(m);
                    let ctx = ui.ctx().clone();
                    self.start_check(&ctx);
                }
                Err(e) => self.err(e),
            }
        }
        if let Some(r) = poll(&mut self.controle.mutation) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "check", tr("NLCS-controle", "NLCS check"), tr("Toetst de tekening aan NLCS 5.02: laagnamen, laagkleur, -lijndikte en -lijntype per tekenstatus, elementen op DuLaag, teksthoogtes.", "Checks the drawing against NLCS 5.02: layer names, colour, lineweight and linetype by status, ByLayer properties and text heights."));
        group(ui, tr("Instellingen", "Settings"), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Tekenschaal (voor teksthoogtes)", "Drawing scale (for text heights)"));
                egui::ComboBox::from_id_salt("chk_scale").selected_text(format!("1:{}", self.controle.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0] {
                        ui.selectable_value(&mut self.controle.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.checkbox(&mut self.controle.text_heights, tr("Teksthoogtes", "Text heights"));
                ui.checkbox(&mut self.controle.empty_layers, tr("Lege lagen melden", "Report empty layers"));
            });
        });
        group(ui, tr("BGT-mutatie", "BGT change"), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Bronhouder", "Data authority"));
                ui.add(egui::TextEdit::singleline(&mut self.controle.bronhouder).desired_width(70.0).hint_text("G0505"))
                    .on_hover_text(tr("Code van de gemeente (G + CBS-code). Leeg: die van de BGT-objecten in de tekening.", "Municipality code (G + CBS code). Leave empty to use the authority from the BGT features in the drawing."));
                ui.checkbox(&mut self.controle.as_built, tr("Gerealiseerd: lagen N als revisie (geen plantopografie)", "As built: treat N layers as revisions (no planned topography)"));
            });
            ui.label(RichText::new(tr("Nieuw (N) wordt plantopografie, revisie (R) gerealiseerd, V beëindigt het BGT-object. Type en functie komen uit de BGT-gegevens, het BGT-object eronder of de NLCS-laag; wat niet eenduidig is, staat in het rapport.", "New (N) is planned topography, revision (R) is as built, and V retires the BGT feature. Type and function come from BGT attributes, the underlying BGT feature or the NLCS layer. See the report for ambiguous cases.")).small().weak());
        });
        let Some(rep) = self.controle.report.clone() else {
            group(ui, tr("Resultaat", "Result"), |ui| {
                job_status(ui, &self.controle.job);
                if self.controle.job.is_none() {
                    ui.label(RichText::new(tr("Klik op Controleren.", "Click Check.")).weak());
                }
            });
            return;
        };
        group(ui, tr("Resultaat", "Result"), |ui| {
            ui.horizontal(|ui| {
                let score = rep.score();
                let colour = if score >= 95.0 { OK_GREEN } else if score >= 70.0 { WARN } else { ERR_RED };
                ui.label(RichText::new(rep.score_label()).size(30.0).strong().color(colour));
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr("conform NLCS", "NLCS compliant")).strong());
                    ui.label(RichText::new(tr_format!("{} van {} elementen · {} lagen, waarvan {} NLCS", "{} of {} entities · {} layers, {} of them NLCS", rep.conform_entities, rep.entities, rep.layers, rep.nlcs_layers)).small().weak());
                });
                ui.add_space(24.0);
                for s in [Severity::Fout, Severity::Waarschuwing, Severity::Info] {
                    badge(ui, &format!("{} {}", rep.count(s), severity_label(s)), severity_colour(s));
                }
            });
            let bar = egui::ProgressBar::new((rep.score() / 100.0) as f32).desired_height(6.0);
            ui.add(bar);
        });
        group(ui, tr("Bevindingen", "Findings"), |ui| {
            if rep.findings.is_empty() {
                ui.label(RichText::new(tr("Geen bevindingen: de tekening volgt NLCS.", "No findings: the drawing follows NLCS.")).color(OK_GREEN));
                return;
            }
            for (rule, list) in rep.by_rule() {
                let open = self.controle.open == Some(rule);
                ui.horizontal(|ui| {
                    badge(ui, severity_label(rule.severity()), severity_colour(rule.severity()));
                    let title = format!("{}  {}  ({}×)", rule.code(), rule_title(rule), list.len());
                    if ui.selectable_label(open, RichText::new(title).strong()).clicked() {
                        self.controle.open = if open { None } else { Some(rule) };
                    }
                    let handles: Vec<_> = list.iter().flat_map(|f| f.handles.iter().copied()).collect();
                    if !handles.is_empty() && ui.small_button(tr_format!("Selecteer {}", "Select {}", handles.len())).on_hover_text(tr("Selecteer deze elementen in de tekening", "Select these entities in the drawing")).clicked() {
                        match bridge::select(&handles) {
                            Ok(()) => self.message = Some((true, tr_format!("{} element(en) geselecteerd in de tekening.", "Selected {} entities in the drawing.", handles.len()))),
                            Err(e) => self.message = Some((false, e)),
                        }
                    }
                });
                if open {
                    list_frame(ui, 220.0, &format!("rule{}", rule.code()), |ui| {
                        egui::Grid::new(format!("findings{}", rule.code())).num_columns(3).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                            for f in &list {
                                ui.label(RichText::new(&f.layer).monospace().small());
                                ui.label(RichText::new(&f.message).small());
                                if !f.handles.is_empty() && ui.small_button(tr("Selecteer", "Select")).clicked() {
                                    let _ = bridge::select(&f.handles);
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
            }
        });
    }

    fn controle_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let busy = self.controle.job.is_some() || self.controle.repair.is_some();
        let repairs = self.controle.report.as_ref().map(|r| r.repairs().len()).unwrap_or(0);
        button_bar(ui, |ui| {
            if ui.add_enabled(!busy, primary_button(tr("Controleren", "Check"))).clicked() {
                self.start_check(&ctx);
            }
            let label = if repairs > 0 { tr_format!("Herstellen ({repairs})", "Repair ({repairs})") } else { tr("Herstellen", "Repair").into() };
            if ui.add_enabled(!busy && repairs > 0, egui::Button::new(label)).on_hover_text(tr("Laagkleur, -lijndikte en -lijntype volgens NLCS; elementen op DuLaag. Laagnamen en teksthoogtes blijven zoals ze zijn.", "Set layer colour, lineweight and linetype according to NLCS and entity properties to ByLayer. Keep layer names and text heights.")).clicked() {
                if let Some(rep) = self.controle.report.clone() {
                    self.controle.repair = Some(Job::spawn(&ctx, tr("Herstellen…", "Repairing…"), move |_| {
                        let doc = bridge::snapshot()?;
                        nlcs_check::repair(&doc, &rep.repairs())
                    }));
                }
            }
            if ui.add_enabled(self.controle.report.is_some(), egui::Button::new(tr("Rapport opslaan", "Save report"))).clicked() {
                if let Some(rep) = &self.controle.report {
                    let opt = nlcs_check::Options { scale: self.controle.scale, ..Default::default() };
                    let path = bridge::beside_drawing("nlcs-controle.md");
                    match std::fs::write(&path, nlcs_check::markdown(rep, &bridge::drawing_name(), &opt)) {
                        Ok(()) => self.ok(tr_format!("Rapport opgeslagen: {}", "Report saved: {}", path.display())),
                        Err(e) => self.err(format!("{}: {e}", path.display())),
                    }
                }
            }
            let (bronhouder, as_built) = (self.controle.bronhouder.clone(), self.controle.as_built);
            if ui
                .add_enabled(!busy && self.controle.mutation.is_none(), egui::Button::new(tr("BGT-mutatie", "BGT change")))
                .on_hover_text(tr("Het ontwerp op NLCS-lagen als BGT-mutatie (IMGeo-GML 2.1.1) met een rapport, naast de tekening. N = plantopografie, R = gerealiseerd, V = beëindigen.", "Export the design on NLCS layers as a BGT change (IMGeo-GML 2.1.1) with a report beside the drawing. N = planned, R = as built, V = retired."))
                .clicked()
            {
                self.controle.mutation = Some(Job::spawn(&ctx, tr("BGT-mutatie maken…", "Creating BGT change…"), move |_| crate::bgt_mutatie::export(&bronhouder, as_built, None)));
            }
            job_status(ui, &self.controle.job);
            job_status(ui, &self.controle.repair);
            job_status(ui, &self.controle.mutation);
        });
    }

    // ── Hoeveelheden ────────────────────────────────────────────────────────
    fn start_quantities(&mut self, ctx: &egui::Context) {
        if self.hoev.job.is_some() {
            return;
        }
        let only_selection = self.hoev.selection;
        let unit = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
        self.hoev.job = Some(Job::spawn(ctx, tr("Hoeveelheden berekenen…", "Calculating quantities…"), move |_| {
            let doc = bridge::snapshot()?;
            let sel: std::collections::HashSet<_> = if only_selection { bridge::selection()?.into_iter().collect() } else { Default::default() };
            if only_selection && sel.is_empty() {
                return Err(tr("niets geselecteerd in de tekening", "nothing selected in the drawing").into());
            }
            Ok(hoeveelheden::compute(&doc, unit, if only_selection { Some(&sel) } else { None }))
        }));
    }

    fn hoeveelheden_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.hoev.job) {
            match r {
                Ok(q) => {
                    let mut m = tr_format!("Hoeveelheden van {} elementen op {} lagen.", "Quantities for {} entities on {} layers.", q.entities, q.rows.len());
                    if let Some(note) = q.duplicates_note() {
                        m.push_str(&format!(" {note}."));
                    }
                    self.message = Some((true, m));
                    self.hoev.result = Some(q);
                }
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "quantities", tr("Hoeveelheden", "Quantities"), tr("Aantal, lengte en oppervlakte per NLCS-laag of per object, uit de geometrie van de tekening.", "Count, length and area by NLCS layer or object, calculated from the drawing geometry."));
        group(ui, tr("Instellingen", "Settings"), |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.hoev.selection, tr("Alleen de selectie in de tekening", "Selected entities only"));
                ui.checkbox(&mut self.hoev.per_object, tr("Per object (G, S en A samen; zonder teksten)", "By object (combine G, S and A, exclude text)"));
                ui.add_enabled(!self.hoev.per_object, egui::Checkbox::new(&mut self.hoev.annotation, tr("Ook tekenblad en annotatie (AL, ZZ)", "Include drawing sheet and annotations (AL, ZZ)")));
                ui.add(egui::TextEdit::singleline(&mut self.hoev.filter).hint_text(tr("filter, bijv. VH of asfalt", "filter, e.g. VH or asfalt")).desired_width(200.0));
            });
        });
        let Some(q) = self.hoev.result.clone() else {
            group(ui, tr("Resultaat", "Result"), |ui| {
                job_status(ui, &self.hoev.job);
                if self.hoev.job.is_none() {
                    ui.label(RichText::new(tr("Klik op Berekenen.", "Click Calculate.")).weak());
                }
            });
            return;
        };
        let rows = if self.hoev.per_object { hoeveelheden::per_object(&q) } else { q.rows.clone() };
        let filter = self.hoev.filter.trim().to_lowercase();
        let annotation = self.hoev.annotation;
        let rows: Vec<_> = rows.into_iter().filter(|r| annotation || !r.annotation()).filter(|r| filter.is_empty() || r.layer.to_lowercase().contains(&filter) || r.object_label().to_lowercase().contains(&filter)).collect();
        group(ui, &tr_format!("Resultaat — {} regels{}", "Result — {} rows{}", rows.len(), if q.from_selection { tr(" (selectie)", " (selection)") } else { "" }), |ui| {
            list_frame(ui, 520.0, "quantities", |ui| {
                egui::Grid::new("q").num_columns(7).striped(true).spacing([14.0, 5.0]).show(ui, |ui| {
                    for h in ["Object", "Status", tr("Laag", "Layer"), tr("Aantal", "Count"), tr("Lengte (m)", "Length (m)"), tr("Opp. (m²)", "Area (m²)"), tr("Hoofdmaat", "Main dimension")] {
                        ui.label(RichText::new(h).small().strong().color(ACCENT));
                    }
                    ui.end_row();
                    let (mut tl, mut ta) = (0.0, 0.0);
                    for r in &rows {
                        ui.label(r.object_label());
                        ui.label(RichText::new(hoeveelheden::status_label(&r.status)).small());
                        ui.label(RichText::new(&r.layer).monospace().small().weak());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(r.count.to_string()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(if r.length > 0.0 { nl(r.length, 2) } else { "–".into() }));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(if r.area > 0.0 { nl(r.area, 2) } else { "–".into() }));
                        let (v, unit) = r.main();
                        let count = unit == "st";
                        let unit = if count { tr("st", "pcs") } else { unit };
                        ui.label(RichText::new(format!("{} {unit}", nl(v, if count { 0 } else { 1 }))).strong());
                        ui.end_row();
                        tl += r.length;
                        ta += r.area;
                    }
                    ui.label(RichText::new(tr("Totaal", "Total")).strong());
                    ui.label("");
                    ui.label("");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(rows.iter().map(|r| r.count).sum::<usize>().to_string()).strong()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(nl(tl, 2)).strong()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(nl(ta, 2)).strong()));
                    ui.end_row();
                });
            });
            if let Some(note) = q.duplicates_note() {
                ui.horizontal(|ui| {
                    ui.colored_label(WARN, format!("⚠ {note}."));
                    if ui.small_button(tr("Kopieën selecteren", "Select duplicates")).on_hover_text(tr("Selecteer de dubbel getekende kopieën in de tekening, om ze te controleren of te verwijderen", "Select duplicate entities in the drawing to review or delete them")).clicked() {
                        let copies: Vec<_> = q.duplicates.iter().map(|d| d.2).collect();
                        std::thread::spawn(move || {
                            if let Err(e) = crate::bridge::select(&copies) {
                                crate::bridge::error(&e);
                            }
                        });
                    }
                });
            }
            ui.label(RichText::new(tr("Oppervlakte: gesloten polylijnen (met bogen), cirkels, ellipsen en arceringen (met eilanden). Lengte: lijnen, bogen, polylijnen, splines. Een element dat op dezelfde laag dubbel getekend is (zelfde vorm op de millimeter, ook omgekeerd), telt één keer. Een hoeveelhedenstaat in de tekening: lintknop Hoeveelhedenstaat of GIS_QUANTITIES_TABLE.", "Area covers closed polylines (including arcs), circles, ellipses and hatches (including islands). Length covers lines, arcs, polylines and splines. Duplicate geometry on the same layer counts once if shapes match to the millimetre, including in reverse. Use Quantities table on the ribbon or GIS_QUANTITIES_TABLE to add a table to the drawing.")).small().weak());
        });
    }

    fn hoeveelheden_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        button_bar(ui, |ui| {
            if ui.add_enabled(self.hoev.job.is_none(), primary_button(tr("Berekenen", "Calculate"))).clicked() {
                self.start_quantities(&ctx);
            }
            if ui.add_enabled(self.hoev.result.is_some(), egui::Button::new(tr("CSV opslaan", "Save CSV"))).on_hover_text(tr("Puntkomma's en decimale komma's, voor Excel en LibreOffice", "Semicolon delimiters and decimal commas for Excel and LibreOffice")).clicked() {
                if let Some(q) = &self.hoev.result {
                    let path = bridge::beside_drawing("hoeveelheden.csv");
                    match std::fs::write(&path, hoeveelheden::csv(q)) {
                        Ok(()) => self.ok(tr_format!("Hoeveelheden opgeslagen: {}", "Quantities saved: {}", path.display())),
                        Err(e) => self.err(format!("{}: {e}", path.display())),
                    }
                }
            }
            job_status(ui, &self.hoev.job);
        });
    }

    // ── Civiel ──────────────────────────────────────────────────────────────
    fn civiel_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.civ.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "axis", tr("Civiel — instellingen", "Civil — settings"), tr("Civiele ontwerptools op het tabblad Civiel: meetpunten, terrein (TIN), assen, profielen en grondverzet. De instellingen gelden ook voor de knoppen in het lint.", "Civil design tools on the Civil tab: survey points, terrain (TIN), alignments, profiles and earthworks. These settings also apply to the ribbon tools."));
        let o = &mut self.civ.opt;
        let num = |ui: &mut egui::Ui, v: &mut f64, lo: f64, hi: f64, speed: f64, suffix: &str| {
            ui.add(egui::DragValue::new(v).range(lo..=hi).speed(speed).suffix(suffix));
        };
        group(ui, tr("Tekenen", "Drawing"), |ui| {
            ui.horizontal(|ui| {
                ui.label(tr("Schaal", "Scale"));
                egui::ComboBox::from_id_salt("civ_scale").selected_text(format!("1:{}", o.nlcs.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0] {
                        ui.selectable_value(&mut o.nlcs.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.label(tr("Tekenstatus", "Drawing status"));
                egui::ComboBox::from_id_salt("civ_status").selected_text(drawing_status(o.nlcs.status)).show_ui(ui, |ui| {
                    for st in nlcs::Status::ALL {
                        ui.selectable_value(&mut o.nlcs.status, st, drawing_status(st));
                    }
                });
                ui.checkbox(&mut o.nlcs.labels, tr("Teksten", "Labels"));
            });
            ui.label(RichText::new(tr("Alles op NLCS-lagen: OG-HOOGTEPUNT, OG-TRIANGULATIE, OG-HOOGTELIJN, AM-AS, AM-KILOMETRERING, AM-TANGENTPUNT, AL-PROFIELBALK, GW-MAAIVELD.", "All features use NLCS layers: OG-HOOGTEPUNT, OG-TRIANGULATIE, OG-HOOGTELIJN, AM-AS, AM-KILOMETRERING, AM-TANGENTPUNT, AL-PROFIELBALK, GW-MAAIVELD.")).small().weak());
        });
        ui.columns(2, |cols| {
            group(&mut cols[0], tr("Punten", "Points"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Bestandsformaat", "File format"));
                    egui::ComboBox::from_id_salt("civ_format").selected_text(survey_format(o.format)).show_ui(ui, |ui| {
                        for f in Format::ALL {
                            ui.selectable_value(&mut o.format, f, survey_format(f));
                        }
                    });
                });
                ui.checkbox(&mut o.point_labels, tr("Puntnummer, hoogte en code bij elk punt", "Point number, elevation and code at each point"));
                ui.label(RichText::new(tr("Scheidingsteken (; , tab spatie) en decimale komma worden herkend. Automatisch: noord/oost uit de getallen.", "Recognises semicolons, commas, tabs and spaces as separators, and decimal commas. Auto detects northing and easting from the numbers.")).small().weak());
            });
            group(&mut cols[0], tr("Oppervlak", "Surface"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Langste driehoekszijde", "Longest triangle edge"));
                    num(ui, &mut o.max_edge, 1.0, 10_000.0, 1.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("TIN uit AHN: puntafstand", "TIN from AHN: point spacing"));
                    num(ui, &mut o.ahn_spacing, 0.5, 100.0, 0.5, " m");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Hoogtelijnen om de", "Contours every"));
                    egui::ComboBox::from_id_salt("civ_interval").selected_text(format!("{} m", o.interval)).show_ui(ui, |ui| {
                        for v in [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.interval, v, format!("{v} m"));
                        }
                    });
                    ui.label(tr("hoofdlijn elke", "major contour every"));
                    ui.add(egui::DragValue::new(&mut o.major_every).range(1..=20));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Hoogteanalyse", "Elevation analysis"));
                    ui.add(egui::DragValue::new(&mut o.bands).range(2..=9).suffix(tr(" banden", " bands")));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Grondverzet: raster", "Earthworks: grid"));
                    num(ui, &mut o.volume_cell, 0.1, 50.0, 0.1, " m");
                    ui.label(tr("peil", "reference level"));
                    num(ui, &mut o.level, -100.0, 1000.0, 0.05, " m");
                });
                ui.label(RichText::new(tr("TIN: selecteer punten, 3D-lijnen of hoogtelijnen; zonder selectie uit AHN in het importgebied. Grondverzet: selecteer de 3D-vlakken van het ontwerp; zonder selectie t.o.v. het peil.", "For a TIN, select points, 3D lines or contours. With no selection, use AHN in the import area. For earthworks, select the design 3D faces. With no selection, compare to the reference level.")).small().weak());
            });
            group(&mut cols[1], tr("As", "Alignment"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Streepjes om de", "Ticks every"));
                    num(ui, &mut o.tick, 1.0, 1000.0, 1.0, " m");
                    ui.label(tr("stationering om de", "station labels every"));
                    num(ui, &mut o.label_every, 1.0, 5000.0, 10.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Verschuivingen (m, links < 0)", "Offsets (m, left < 0)"));
                    if ui.add(egui::TextEdit::singleline(&mut self.civ.offsets).desired_width(140.0)).changed() {
                        o.offsets = parse_offsets(&self.civ.offsets);
                    }
                });
            });
            group(&mut cols[1], tr("Profielen", "Profiles"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Lengteprofiel: stap", "Longitudinal profile: step"));
                    num(ui, &mut o.step, 0.1, 50.0, 0.1, " m");
                    ui.label(tr("overhoging", "vertical exaggeration"));
                    num(ui, &mut o.exaggeration, 1.0, 50.0, 0.5, "×");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Ontwerpprofiel: boog", "Design profile: curve"));
                    num(ui, &mut o.vertical_curve, 0.0, 2000.0, 5.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Dwarsprofielen om de", "Cross sections every"));
                    num(ui, &mut o.section_every, 1.0, 1000.0, 1.0, " m");
                    ui.label(tr("links", "left"));
                    num(ui, &mut o.left, 1.0, 500.0, 1.0, " m");
                    ui.label(tr("rechts", "right"));
                    num(ui, &mut o.right, 1.0, 500.0, 1.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label(tr("overhoging", "vertical exaggeration"));
                    num(ui, &mut o.section_exaggeration, 1.0, 20.0, 0.5, "×");
                    ui.label(tr("per rij", "per row"));
                    ui.add(egui::DragValue::new(&mut o.per_row).range(1..=20));
                });
                ui.label(RichText::new(tr("Selecteer de as in de tekening en kies Lengteprofiel of Dwarsprofielen in het lint; klik dan waar ze komen. Ontwerpprofiel: teken een polylijn in het lengteprofiel en selecteer die.", "Select the alignment in the drawing and choose Longitudinal profile or Cross sections on the ribbon. Click to place the result. For a design profile, draw a polyline in the longitudinal profile and select it.")).small().weak());
            });
        });
        civiel::set_settings(self.civ.opt.clone());
    }

    fn civiel_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let busy = self.civ.run.is_some();
        let mut start: Option<(&'static str, fn(&dyn Fn(String)) -> Result<String, String>)> = None;
        button_bar(ui, |ui| {
            use civiel::{as_profiel, landmeten, settings, terrein};
            for (label, f) in [
                (tr("TIN maken", "Create TIN"), (|p| terrein::create(&settings(), terrein::TinSource::Auto, p)) as fn(&dyn Fn(String)) -> Result<String, String>),
                (tr("Hoogtelijnen", "Contours"), |p| terrein::contours(&settings(), terrein::Surface::Auto, p)),
                (tr("Hoogteanalyse", "Elevation analysis"), |_| terrein::analysis(&settings(), terrein::Surface::Auto)),
                (tr("Grondverzet", "Earthworks"), |p| terrein::volume(&settings(), terrein::Design::Auto, p)),
                (tr("As maken", "Create alignment"), |_| as_profiel::create(&settings(), as_profiel::AxisInput::Line(None), None, None)),
                (tr("As verschuiven", "Offset alignment"), |_| as_profiel::offsets(&settings(), None)),
                (tr("Ontwerpprofiel", "Design profile"), |_| as_profiel::design_profile(&settings(), None, None, None)),
                (tr("Punten exporteren", "Export points"), |_| landmeten::export(&settings(), None)),
            ] {
                if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                    start = Some((label, f));
                }
            }
            job_status(ui, &self.civ.run);
        });
        if let Some((label, f)) = start {
            civiel::set_settings(self.civ.opt.clone());
            self.civ.run = Some(Job::spawn(&ctx, &format!("{label}…"), move |p| f(p)));
        }
    }

    // ── Servers ─────────────────────────────────────────────────────────────
    fn servers_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.srv.test) {
            match r {
                Ok(title) => {
                    let kind = self.srv.kind.unwrap_or(ServerKind::Wms);
                    let name = if self.srv.name.trim().is_empty() { title } else { self.srv.name.trim().to_string() };
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let server = MapServer {
                        id: format!("user-{stamp}"),
                        kind,
                        name: name.clone(),
                        region: if self.srv.region.trim().is_empty() { "Eigen servers".into() } else { self.srv.region.trim().into() },
                        url: if kind == ServerKind::Xyz { self.srv.url.trim().to_string() } else { crate::gis::http::clean_service_url(&self.srv.url) },
                        attribution: String::new(),
                        default_layer: String::new(),
                        max_zoom: (kind == ServerKind::Xyz).then_some(19),
                        min_zoom: None,
                        builtin: false,
                    };
                    match servers::update_user(|user| user.push(server)) {
                        Ok(()) => {
                            self.servers = servers::all();
                            self.svc.refresh();
                            self.srv.url.clear();
                            self.srv.name.clear();
                            self.ok(tr_format!("Server \"{name}\" toegevoegd.", "Server \"{name}\" added."));
                        }
                        Err(e) => self.err(e),
                    }
                }
                Err(e) => self.err(tr_format!("Server niet toegevoegd: {e}", "Server not added: {e}")),
            }
        }
        page_header(ui, "server", tr("Serverbeheer", "Manage servers"), tr("Voorgedefinieerde servers (Nederland, Vlaanderen, wereld) en uw eigen WMS-, WFS- en tegelservers.", "Predefined servers for the Netherlands, Flanders and worldwide, plus your own WMS, WFS and tile servers."));
        group(ui, tr("Server toevoegen", "Add server"), |ui| {
            ui.horizontal(|ui| {
                let kind = self.srv.kind.get_or_insert(ServerKind::Wms);
                for k in [ServerKind::Wms, ServerKind::Wfs, ServerKind::Xyz] {
                    ui.radio_value(kind, k, k.label());
                }
            });
            let xyz = self.srv.kind == Some(ServerKind::Xyz);
            ui.add(egui::TextEdit::singleline(&mut self.srv.url).hint_text(if xyz { "https://…/{z}/{x}/{y}.png" } else { tr("https://…/wms of …/wfs (GetCapabilities wordt getest)", "https://…/wms or …/wfs (tests GetCapabilities)") }).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.srv.name).hint_text(tr("naam (leeg = titel van de server)", "name (leave empty to use the server title)")));
                ui.add(egui::TextEdit::singleline(&mut self.srv.region).hint_text(tr("groep (leeg = eigen servers)", "group (leave empty for custom servers)")));
                if ui.add_enabled(self.srv.test.is_none() && !self.srv.url.trim().is_empty(), primary_button(tr("Testen en toevoegen", "Test and add"))).clicked() {
                    let (kind, url) = (self.srv.kind.unwrap_or(ServerKind::Wms), self.srv.url.trim().to_string());
                    self.srv.test = Some(Job::spawn(&ctx, tr("Server testen…", "Testing server…"), move |_| match kind {
                        ServerKind::Wms => caps::fetch_wms(&crate::gis::http::clean_service_url(&url)).map(|c| c.title),
                        ServerKind::Wfs => caps::fetch_wfs(&crate::gis::http::clean_service_url(&url)).map(|c| c.title),
                        ServerKind::Xyz => {
                            if !(url.contains("{z}") && url.contains("{x}") && url.contains("{y}")) {
                                return Err(tr("tegel-URL moet {z}, {x} en {y} bevatten", "tile URL must contain {z}, {x} and {y}").into());
                            }
                            crate::gis::http::get_image(&url.replace("{z}", "0").replace("{x}", "0").replace("{y}", "0")).map(|_| tr("Tegelserver", "Tile server").to_string())
                        }
                    }));
                }
                if let Some(j) = &self.srv.test {
                    ui.spinner();
                    ui.label(j.status());
                }
            });
        });
        self.qgis_import(ui);
        let mut remove = None;
        group(ui, tr("Servers", "Servers"), |ui| {
            let mut regions: Vec<String> = Vec::new();
            for s in &self.servers {
                if !regions.contains(&s.region) {
                    regions.push(s.region.clone());
                }
            }
            egui::Grid::new("srv_list").num_columns(4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for r in regions {
                    ui.label(RichText::new(server_region_label(&r).to_uppercase()).small().strong().color(ACCENT));
                    ui.end_row();
                    for s in self.servers.iter().filter(|s| s.region == r) {
                        ui.label(RichText::new(s.kind.label()).monospace().small());
                        ui.label(&s.name);
                        ui.label(RichText::new(&s.url).small().weak());
                        if s.builtin {
                            ui.label(RichText::new(tr("standaard", "predefined")).small().weak());
                        } else if ui.small_button(tr("Verwijderen", "Remove")).clicked() {
                            remove = Some(s.id.clone());
                        }
                        ui.end_row();
                    }
                }
            });
        });
        if let Some(id) = remove {
            match servers::update_user(|user| user.retain(|s| s.id != id)) {
                Ok(()) => {
                    self.servers = servers::all();
                    self.svc.refresh();
                }
                Err(e) => self.err(e),
            }
        }
    }
}

impl App {
    /// Connections exported from QGIS: read the XML, show what can be added,
    /// add the chosen ones in one save.
    fn qgis_import(&mut self, ui: &mut egui::Ui) {
        use crate::gis::qgis;
        let mut read = ui.ctx().input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")))
        });
        let mut add = false;
        group(ui, tr("QGIS-verbindingen importeren", "Import QGIS connections"), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.srv.qgis_path)
                        .hint_text(tr("XML uit QGIS (Verbindingen opslaan) — of sleep het bestand op dit venster", "XML from QGIS (Save connections) — or drop the file on this window"))
                        .desired_width(ui.available_width() - 90.0),
                );
                if ui.add_enabled(!self.srv.qgis_path.trim().is_empty(), egui::Button::new(tr("Lezen", "Read"))).clicked() {
                    read = Some(std::path::PathBuf::from(self.srv.qgis_path.trim().trim_matches('"')));
                }
            });
            if self.srv.qgis.is_empty() {
                ui.label(RichText::new(tr("WMS-, WFS- en XYZ-verbindingen. Verbindingen met een wachtwoord, eigen headers of een afwijkende asvolgorde worden getoond maar niet toegevoegd.", "WMS, WFS and XYZ connections. Connections with a password, own headers or a different axis order are shown but not added.")).small().weak());
                return;
            }
            egui::Grid::new("qgis_import").num_columns(4).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                for (c, on) in self.srv.qgis.iter_mut() {
                    ui.add_enabled(c.importable(), egui::Checkbox::without_text(on));
                    ui.label(RichText::new(c.server.kind.label()).monospace().small());
                    let name = ui.label(&c.server.name);
                    if c.blocking.is_empty() {
                        name.on_hover_text(&c.server.url);
                    }
                    if c.duplicate {
                        ui.label(RichText::new(tr("staat er al", "already there")).small().weak());
                    } else if c.blocking.is_empty() {
                        ui.label(RichText::new(c.server.url.as_str()).small().weak());
                    } else {
                        ui.colored_label(WARN, RichText::new(c.blocking.join(" · ")).small());
                    }
                    ui.end_row();
                }
            });
            let n = self.srv.qgis.iter().filter(|(c, on)| *on && c.importable()).count();
            ui.horizontal(|ui| {
                if ui.add_enabled(n > 0, primary_button(&format!("{} ({n})", tr("Toevoegen", "Add")))).clicked() {
                    add = true;
                }
                if ui.small_button(tr("Sluiten", "Close")).clicked() {
                    self.srv.qgis.clear();
                }
            });
        });
        if let Some(path) = read {
            self.srv.qgis_path = path.to_string_lossy().into_owned();
            match qgis::read(&path) {
                Ok(mut conns) => {
                    qgis::mark_duplicates(&mut conns, &servers::all());
                    self.srv.qgis = conns.into_iter().map(|c| { let on = c.importable(); (c, on) }).collect();
                }
                Err(e) => {
                    self.srv.qgis.clear();
                    self.err(e);
                }
            }
        }
        if add {
            let chosen: Vec<&qgis::Connection> = self.srv.qgis.iter().filter(|(c, on)| *on && c.importable()).map(|(c, _)| c).collect();
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let saved = servers::update_user(|user| {
                let (list, added) = qgis::merge(user, &chosen, stamp);
                *user = list;
                added
            });
            match saved {
                Ok(added) => {
                    self.servers = servers::all();
                    self.svc.refresh();
                    self.srv.qgis.clear();
                    self.ok(format!("{added} {}", tr("QGIS-verbindingen toegevoegd. Kies ze in Services wereldwijd om ze te testen.", "QGIS connections added. Choose them in Services worldwide to test them.")));
                }
                Err(e) => self.err(format!("{}: {e}", tr("Niets toegevoegd", "Nothing added"))),
            }
        }
    }
}

// ── Layout helpers ──────────────────────────────────────────────────────────

/// Badge colour per kind of search hit.
fn kind_colour(kind: &str) -> Color32 {
    match kind {
        "adres" => Color32::from_rgb(38, 150, 140),
        "straat" => Color32::from_rgb(60, 120, 215),
        "postcode" => Color32::from_rgb(110, 130, 145),
        "plaats" | "gemeente" | "provincie" | "land" | "waterschap" => Color32::from_rgb(70, 160, 75),
        "perceel" | "appartement" => Color32::from_rgb(220, 110, 30),
        "coördinaat" => Color32::from_rgb(140, 105, 200),
        "hectometer" => Color32::from_rgb(120, 120, 120),
        _ => Color32::from_rgb(185, 140, 20),
    }
}

fn hit_kind_label(kind: &str) -> &str {
    match kind {
        "adres" => tr("adres", "address"),
        "straat" => tr("straat", "street"),
        "postcode" => tr("postcode", "postcode"),
        "plaats" => tr("plaats", "place"),
        "gemeente" => tr("gemeente", "municipality"),
        "provincie" => tr("provincie", "province"),
        "land" => tr("land", "country"),
        "waterschap" => tr("waterschap", "water authority"),
        "perceel" => tr("perceel", "parcel"),
        "appartement" => tr("appartement", "apartment"),
        "coördinaat" => tr("coördinaat", "coordinate"),
        "hectometer" => tr("hectometer", "hectometre"),
        _ => kind,
    }
}

/// One search hit: a coloured kind badge, the name and below it the place,
/// the source on the right. Long texts end in "…".
fn hit_row(ui: &mut egui::Ui, h: &Hit, highlighted: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
    // Painted by hand: give screen readers (and the UI test) its text.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} — {} ({})", h.name, h.place, hit_kind_label(&h.kind))));
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    if highlighted || resp.hovered() {
        painter.rect_filled(rect, 4.0, if highlighted { ACCENT.gamma_multiply(0.35) } else { visuals.widgets.hovered.weak_bg_fill });
    }
    let badge_font = egui::FontId::proportional(10.5);
    let badge = painter.layout_no_wrap(hit_kind_label(&h.kind).to_owned(), badge_font, Color32::WHITE);
    let badge_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + 8.0, rect.center().y - 9.0), Vec2::new((badge.size().x + 12.0).max(58.0), 18.0));
    painter.rect_filled(badge_rect, 9.0, kind_colour(&h.kind));
    painter.galley(badge_rect.center() - badge.size() / 2.0, badge, Color32::WHITE);
    let text_x = rect.left() + 82.0;
    let source = painter.layout_no_wrap(h.source.to_string(), egui::FontId::proportional(10.0), visuals.weak_text_color());
    let room = (rect.right() - 12.0 - source.size().x - 10.0 - text_x).max(40.0);
    let line = |text: &str, size: f32, colour: Color32| {
        let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(egui::FontId::proportional(size), colour));
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let name = line(&h.name, 14.0, visuals.strong_text_color());
    let place = line(&h.place, 11.5, visuals.weak_text_color());
    painter.galley(egui::pos2(text_x, rect.top() + 4.0), name, Color32::WHITE);
    painter.galley(egui::pos2(text_x, rect.top() + 22.0), place, Color32::WHITE);
    painter.galley(egui::pos2(rect.right() - 10.0 - source.size().x, rect.center().y - source.size().y / 2.0), source, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn page_header(ui: &mut egui::Ui, icon_name: &str, title: &str, subtitle: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(icon(icon_name).fit_to_exact_size(Vec2::splat(26.0)));
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(18.0).strong());
            ui.label(RichText::new(subtitle).weak());
        });
    });
    ui.add_space(6.0);
}

/// A titled group box, like the QGroupBox sections of the FreeCAD dialogs.
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(10)).corner_radius(4.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).small().strong().color(ACCENT));
        ui.add_space(2.0);
        add(ui);
    });
    ui.add_space(6.0);
}

fn list_frame(ui: &mut egui::Ui, height: f32, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(egui::Margin::same(4)).corner_radius(3.0).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(id).max_height(height).auto_shrink([false, true]).show(ui, add);
    });
}

fn button_bar(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(add);
}

fn job_status<T: Send + 'static>(ui: &mut egui::Ui, job: &Option<Job<T>>) {
    if let Some(j) = job {
        ui.spinner();
        ui.label(j.status());
    }
}

fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).strong().color(Color32::WHITE)).fill(ACCENT).min_size(Vec2::new(130.0, 28.0))
}

fn server_region_label(region: &str) -> &str {
    match region {
        "Eigen servers" => tr("Eigen servers", "Custom servers"),
        _ => region,
    }
}

fn server_combo(ui: &mut egui::Ui, id: &str, all: &[MapServer], kinds: &[ServerKind], current: Option<&MapServer>) -> Option<MapServer> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().min(520.0))
        .height(480.0)
        .selected_text(current.map(|s| s.name.clone()).unwrap_or_else(|| crate::lang::tr("— kies een server —", "— choose a server —").into()))
        .show_ui(ui, |ui| {
            // Hundreds of presets worldwide: a filter on name, country and URL.
            let fid = egui::Id::NULL.with((id, "filter"));
            let mut filter: String = ui.data_mut(|d| d.get_temp(fid).unwrap_or_default());
            ui.horizontal(|ui| {
                ui.label(crate::lang::tr("Filter", "Filter"));
                ui.add(egui::TextEdit::singleline(&mut filter).hint_text(crate::lang::tr("land, regio of naam", "country, region or name")).desired_width(260.0));
            });
            ui.data_mut(|d| d.insert_temp(fid, filter.clone()));
            let words: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
            let hit = |s: &MapServer| {
                let hay = format!("{} {} {} {} {}", s.name, s.region, server_region_label(&s.region), s.url, s.id).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            };
            let mut region = "";
            for s in all.iter().filter(|s| kinds.contains(&s.kind) && hit(s)) {
                if s.region != region {
                    region = &s.region;
                    ui.label(RichText::new(server_region_label(region).to_uppercase()).small().color(ACCENT));
                }
                let label = if s.kind == ServerKind::Xyz { format!("{}   [{}]", s.name, crate::lang::tr("tegels", "tiles")) } else { s.name.clone() };
                if ui.selectable_label(current.is_some_and(|c| c.id == s.id), label).clicked() {
                    picked = Some(s.clone());
                }
            }
        });
    picked
}

/// Returns true when the colour changed.
fn aci_combo(ui: &mut egui::Ui, id: &str, aci: &mut i16) -> bool {
    let before = *aci;
    let name = ACI_CHOICES.iter().find(|(a, _, _)| a == aci).map(|(_, nl, en)| tr(nl, en)).unwrap_or("?");
    egui::ComboBox::from_id_salt(id).selected_text(RichText::new(name).color(aci_color(*aci))).show_ui(ui, |ui| {
        for (a, nl, en) in ACI_CHOICES {
            ui.selectable_value(aci, *a, RichText::new(tr(nl, en)).color(aci_color(*a)));
        }
    });
    before != *aci
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);
        // Closing only hides: the event loop cannot be created twice.
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("nav").exact_size(196.0).resizable(false).show(ui, |ui| {
            ui.add_space(8.0);
            let civil = super::product() == Product::Civiel;
            ui.horizontal(|ui| {
                ui.add(icon(if civil { "axis" } else { "location" }).fit_to_exact_size(Vec2::splat(22.0)));
                ui.label(RichText::new(if civil { tr("Open Infra · Civiel", "Open Infra · Civil") } else { "Open Infra · GIS" }).size(15.0).strong());
            });
            ui.add_space(10.0);
            let mut nav = |ui: &mut egui::Ui, tab: Tab, ic: &str, label: &str| {
                let sel = self.tab == tab;
                let b = egui::Button::image_and_text(icon(ic), RichText::new(label).size(13.5)).selected(sel).frame(sel).min_size(Vec2::new(ui.available_width(), 30.0));
                if ui.add(b).clicked() {
                    self.tab = tab;
                }
            };
            if civil {
                ui.label(RichText::new(tr("CIVIEL", "CIVIL")).small().weak());
                nav(ui, Tab::Civiel, "axis", crate::lang::tr("Instellingen en acties", "Settings and actions"));
                ui.add_space(8.0);
                ui.label(RichText::new(crate::lang::tr("ALGEMEEN","GENERAL")).small().weak());
                nav(ui, Tab::Location, "location", crate::lang::tr("Projectlocatie", "Project location"));
                nav(ui, Tab::Coords, "point", crate::lang::tr("Coördinaten", "Coordinates"));
                if bridge::site().map(|s|s.region).unwrap_or(crate::localisation::current())==crate::localisation::Region::Netherlands {
                    ui.add_space(8.0);
                    ui.label(RichText::new(crate::lang::tr("NEDERLAND", "NETHERLANDS")).small().weak());
                    nav(ui, Tab::Controle, "check", crate::lang::tr("NLCS-controle", "NLCS check"));
                    nav(ui, Tab::Hoeveelheden, "quantities", crate::lang::tr("NLCS-hoeveelheden", "NLCS quantities"));
                }
            } else {
            ui.label(RichText::new(crate::lang::tr("WEBSERVICES","WEB SERVICES")).small().weak());
            nav(ui, Tab::Location, "location", crate::lang::tr("Geografische locatie", "Geographic location"));
            nav(ui, Tab::Raster, "map", crate::lang::tr("WMS / TMS (raster)", "WMS / TMS (raster)"));
            nav(ui, Tab::Wfs, "vector", crate::lang::tr("WFS (vector)", "WFS (vector)"));
            nav(ui, Tab::Services, "services", crate::lang::tr("Services wereldwijd", "Services worldwide"));
            nav(ui, Tab::Osm, "globe", "OpenStreetMap");
            if bridge::site().map(|s|s.region).unwrap_or(crate::localisation::current())==crate::localisation::Region::Netherlands {
                ui.add_space(8.0);
                ui.label(RichText::new(crate::lang::tr("NEDERLAND", "NETHERLANDS")).small().weak());
                nav(ui, Tab::Pdok, "pdok", "PDOK · BAG · NWB");
                nav(ui, Tab::Nlcs, "nlcs", crate::lang::tr("BGT · Kadaster · KLIC", "BGT · Cadastre · KLIC"));
                nav(ui, Tab::Ahn, "ahn", crate::lang::tr("AHN-hoogtes", "AHN elevation"));
                nav(ui, Tab::Planning, "vector", crate::lang::tr("Ruimtelijke plannen", "Spatial plans"));
                nav(ui, Tab::Controle, "check", crate::lang::tr("NLCS-controle", "NLCS check"));
                nav(ui, Tab::Hoeveelheden, "quantities", crate::lang::tr("NLCS-hoeveelheden", "NLCS quantities"));
            }
            ui.add_space(8.0);
            ui.label(RichText::new(crate::lang::tr("HULPMIDDELEN","TOOLS")).small().weak());
            nav(ui, Tab::Coords, "point", crate::lang::tr("Coördinaten", "Coordinates"));
            nav(ui, Tab::Servers, "server", crate::lang::tr("Servers", "Servers"));
            ui.add_space(8.0);
            ui.label(RichText::new(crate::lang::tr("BESTANDEN EN BIBLIOTHEEK", "FILES AND LIBRARY")).small().weak());
            nav(ui, Tab::Files, "file", crate::lang::tr("GIS-bestand", "GIS file"));
            nav(ui, Tab::Library, "vehicle", crate::lang::tr("Bibliotheek", "Library"));
            }
            ui.add_space(8.0);
            nav(ui, Tab::Localisation, "globe", crate::lang::tr("Localisatie", "Localisation"));
            if !civil && bridge::site().map(|s|s.region).unwrap_or(crate::localisation::current())==crate::localisation::Region::UnitedStates {
                ui.add_space(8.0);
                ui.label(RichText::new(crate::lang::tr("VERENIGDE STATEN","UNITED STATES")).small().weak());
                if ui.button(crate::lang::tr("USGS · Census · NRCS","USGS · Census · NRCS")).clicked(){self.svc.regional(crate::localisation::Region::UnitedStates);self.tab=Tab::Services;}
            }
            super::library::language_switch(ui);
            ui.add_space(14.0);

            egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(crate::lang::tr("PROJECTLOCATIE","PROJECT LOCATION")).small().strong().color(ACCENT));
                if let Some(s) = bridge::site() {
                    ui.label(RichText::new(&s.label).small());
                    ui.label(RichText::new(s.region.label()).small().strong());
                    ui.label(RichText::new(format!("{}\nX {:.1}\nY {:.1}\n{} × {} m · {}", s.crs, s.origin_e, s.origin_n, s.width, s.height, if s.absolute { tr("absoluut", "absolute") } else { tr("relatief", "relative") })).small().monospace().weak());
                } else {
                    ui.label(RichText::new(tr("Nog niet ingesteld", "Not set")).small().color(WARN));
                }
                if let Some(t) = bridge::target() {
                    let doc = t.doc_path.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| tr("(niet opgeslagen tekening)", "(unsaved drawing)").into());
                    ui.add_space(4.0);
                    ui.label(RichText::new(tr_format!("Tekening: {doc}", "Drawing: {doc}")).small().weak());
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                match &self.message {
                    Some((true, m)) => ui.label(RichText::new(m).color(OK_GREEN)),
                    Some((false, m)) => ui.label(RichText::new(m).color(ERR_RED)),
                    None => ui.label(RichText::new(tr("Klaar.", "Ready.")).weak()),
                };
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            // OK / Test / Annuleren stay visible at the bottom, as in the FreeCAD dialogs.
            let has_actions = matches!(self.tab, Tab::Location | Tab::Raster | Tab::Wfs | Tab::Osm | Tab::Pdok | Tab::Nlcs | Tab::Ahn | Tab::Controle | Tab::Hoeveelheden | Tab::Civiel | Tab::Services);
            if has_actions {
                egui::Panel::bottom("actions").frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 8))).show(ui, |ui| match self.tab {
                    Tab::Location => self.location_actions(ui),
                    Tab::Raster => self.raster_actions(ui),
                    Tab::Wfs => self.wfs_actions(ui),
                    Tab::Osm => self.osm_actions(ui),
                    Tab::Pdok => self.pdok_actions(ui),
                    Tab::Nlcs => self.nlcs_actions(ui),
                    Tab::Ahn => self.ahn_actions(ui),
                    Tab::Controle => self.controle_actions(ui),
                    Tab::Hoeveelheden => self.hoeveelheden_actions(ui),
                    Tab::Civiel => self.civiel_actions(ui),
                    Tab::Services => self.services_actions(ui),
                    _ => {}
                });
            }
            egui::ScrollArea::vertical().scroll_source(egui::scroll_area::ScrollSource::MOUSE_WHEEL).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(ui.available_width() - 6.0);
                let project_region=bridge::site().map(|s|s.region).unwrap_or(crate::localisation::current());
                if project_region!=crate::localisation::Region::Netherlands && matches!(self.tab,Tab::Civiel|Tab::Library|Tab::Controle|Tab::Hoeveelheden) {
                    ui.colored_label(WARN,crate::lang::tr("De civiele ontwerptools en infra-bibliotheek gebruiken momenteel NLCS-lagen. Voor dit project is een afzonderlijke mapping naar de opleverstandaard nodig.","Civil design tools and the infrastructure library currently use NLCS layers. This project needs a separate mapping to its delivery standard."));
                }
                match self.tab {
                    Tab::Location => self.location(ui),
                    Tab::Raster => self.raster(ui),
                    Tab::Wfs => self.wfs(ui),
                    Tab::Osm => self.osm(ui),
                    Tab::Pdok => self.pdok(ui),
                    Tab::Nlcs => self.nlcs_page(ui),
                    Tab::Ahn => self.ahn_page(ui),
                    Tab::Controle => self.controle_page(ui),
                    Tab::Hoeveelheden => self.hoeveelheden_page(ui),
                    Tab::Servers => self.servers_page(ui),
                    Tab::Coords => self.coords(ui),
                    Tab::Library => self.lib.page(ui),
                    Tab::Files => self.files.page(ui),
                    Tab::Civiel => self.civiel_page(ui),
                    Tab::Services => self.services_page(ui),
                    Tab::Localisation => self.localisation_page(ui),
                    Tab::Planning => self.planning.page(ui),
                }
            });
        });
    }
}
