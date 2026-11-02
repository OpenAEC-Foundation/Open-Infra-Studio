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

const ACI_CHOICES: &[(i16, &str)] = &[(1, "rood"), (2, "geel"), (3, "groen"), (4, "cyaan"), (5, "blauw"), (6, "magenta"), (7, "wit/zwart"), (8, "grijs"), (30, "oranje")];
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
            srv: ServersPage { region: "Eigen servers".into(), ..Default::default() },
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
            group(ui, "Projectlocatie ontbreekt", |ui| {
                ui.colored_label(WARN, "Deze tekening heeft nog geen projectlocatie. Stel die eerst in.");
                if ui.button("Geografische locatie instellen…").clicked() {
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
        group(ui, "Kaart", |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Button::image_and_text(icon("area"), "Gebied tekenen").selected(self.map.area_mode))
                    .on_hover_text("Sleep op de kaart een rechthoek als importgebied (of houd Shift ingedrukt)")
                    .clicked()
                    .then(|| self.map.area_mode = !self.map.area_mode);
                if ui.add_enabled(ring.is_some(), egui::Button::new("Zoom naar gebied")).clicked() {
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
                    self.loc.suggest = Some(Job::spawn(&ctx, "Suggesties…", move |_| geocode::suggest(&q2).map(|h| (q2, h))));
                }
            }
        }
        if let Some(r) = poll(&mut self.loc.crs_job) {
            match r {
                Ok(info) if info.proj4.contains("+proj=longlat") => self.err("Kies een geprojecteerd stelsel (in meters) als projectstelsel."),
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

        page_header(ui, "location", "Geografische locatie instellen", "Kies de projectlocatie op de kaart of zoek een adres; teken of typ de begrenzing van het importgebied.");
        let draft = self.draft.clone();
        match self.map_section(ui, Some(&draft), Some([draft.lon, draft.lat]), None, &[]) {
            Some(MapEvent::Clicked(ll)) => {
                self.draft.lon = ll[0];
                self.draft.lat = ll[1];
                self.draft.label = format!("Kaartpunt {:.5}, {:.5}", ll[1], ll[0]);
                self.draft.reproject_origin();
            }
            Some(MapEvent::RectDrawn(a, b)) => {
                if let (Ok(pa), Ok(pb)) = (crs::transform("EPSG:4326", &self.draft.crs, a), crs::transform("EPSG:4326", &self.draft.crs, b)) {
                    self.draft.set_origin(((pa[0] + pb[0]) / 2.0).round(), ((pa[1] + pb[1]) / 2.0).round());
                    self.draft.width = (pa[0] - pb[0]).abs().round().max(10.0);
                    self.draft.height = (pa[1] - pb[1]).abs().round().max(10.0);
                    self.draft.label = format!("Getekend gebied {} × {} m", self.draft.width, self.draft.height);
                    self.map.area_mode = false;
                }
            }
            None => {}
        }

        let mut field: Option<egui::Rect> = None;
        let mut pick: Option<usize> = None;
        let mut search_all = false;
        group(ui, "Locatie", |ui| {
            egui::Grid::new("loc").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label("Zoek plaats / adres");
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.loc.query)
                            .hint_text("adres, plaats, perceel, straat — of 51.814, 4.660 / RD 104885 425320")
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
                    if ui.add_enabled(self.loc.search.is_none() && typed, egui::Button::new("Zoeken")).on_hover_text("In Nederland (PDOK) en wereldwijd (OpenStreetMap)").on_disabled_hover_text("Typ eerst een adres, plaats of coördinaten").clicked() {
                        search_all = true;
                    }
                });
                ui.end_row();

                ui.label("Coördinatenstelsel (CRS)");
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
                            let star = if i.code == suggested { "  ★ aanbevolen" } else { "" };
                            if ui.selectable_label(i.code == self.draft.crs, format!("{} — {} ({}){star}", i.code, i.name, i.region)).clicked() {
                                chosen = Some(i.code.clone());
                            }
                        }
                    });
                    if let Some(c) = chosen {
                        self.draft.crs = c;
                        self.draft.reproject_origin();
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.loc.custom_epsg).hint_text("EPSG-code").desired_width(80.0));
                    if ui.add_enabled(self.loc.crs_job.is_none() && !self.loc.custom_epsg.trim().is_empty(), egui::Button::new("Laden")).on_hover_text("Definitie ophalen via epsg.io").clicked() {
                        let code = self.loc.custom_epsg.clone();
                        self.loc.crs_job = Some(Job::spawn(&ctx, "Stelsel laden…", move |_| crs::ensure(&code)));
                    }
                });
                ui.end_row();

                ui.label("Breedtegraad / lengtegraad");
                ui.horizontal(|ui| {
                    let a = ui.add(egui::DragValue::new(&mut self.draft.lat).speed(0.0001).fixed_decimals(7).prefix("lat "));
                    let b = ui.add(egui::DragValue::new(&mut self.draft.lon).speed(0.0001).fixed_decimals(7).prefix("lon "));
                    if a.changed() || b.changed() {
                        self.draft.reproject_origin();
                    }
                });
                ui.end_row();

                ui.label("Omschrijving");
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
            self.loc.search = Some(Job::spawn(&ctx, "Zoeken…", move |_| geocode::search(&q)));
        }
        if let Some(i) = pick {
            self.pick_hit(&ctx, i);
        }

        group(ui, "Begrenzing en plaatsing", |ui| {
            egui::Grid::new("bbox").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label("Basispunt X / Y (m)");
                ui.horizontal(|ui| {
                    let (mut e, mut n) = (self.draft.origin_e, self.draft.origin_n);
                    let a = ui.add(egui::DragValue::new(&mut e).speed(1.0).fixed_decimals(3).prefix("X "));
                    let b = ui.add(egui::DragValue::new(&mut n).speed(1.0).fixed_decimals(3).prefix("Y "));
                    if a.changed() || b.changed() {
                        self.draft.set_origin(e, n);
                    }
                });
                ui.end_row();
                ui.label("Begrenzing breedte × hoogte (m)");
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
                ui.label("Coördinaten in de tekening");
                ui.vertical(|ui| {
                    ui.radio_value(&mut self.draft.absolute, true, "Absoluut — CRS-coördinaten (bijv. RD) direct in de tekening");
                    ui.radio_value(&mut self.draft.absolute, false, "Relatief — basispunt = 0,0");
                });
                ui.end_row();
                ui.label("Na opslaan");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.loc.geo_origin, "Basispunt-markering in de tekening plaatsen (GeoOrigin)");
                    ui.checkbox(&mut self.loc.aerial, "Luchtfoto importeren (PDOK in NL, Sentinel-2 elders)");
                });
                ui.end_row();
            });
            let units = bridge::target().map(|t| t.unit_factor).unwrap_or(1.0);
            ui.label(RichText::new(format!(
                "Tekeneenheid: {} (INSUNITS){}",
                if (units - 1.0).abs() < 1e-9 { "meter".to_string() } else if (units - 1000.0).abs() < 1e-9 { "millimeter".into() } else { format!("{units} per meter") },
                if self.has_site { "" } else { " · deze tekening heeft nog geen projectlocatie" }
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
                                    "PDOK" => "NEDERLAND · PDOK",
                                    "OSM" => "WERELDWIJD · OPENSTREETMAP",
                                    _ => "INVOER",
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
                        ui.label(RichText::new(format!("Niets gevonden voor \"{q}\".")).weak());
                    }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if busy {
                            ui.spinner();
                            ui.label(RichText::new(if self.loc.search.is_some() { "Zoeken in Nederland en wereldwijd…" } else { "Suggesties…" }).small().weak());
                        } else if self.loc.hits_full {
                            ui.label(RichText::new("Pijltjes en Enter om te kiezen · Esc sluit").small().weak());
                        } else {
                            ui.label(RichText::new("Enter of Zoeken: ook wereldwijd zoeken (OpenStreetMap) · pijltjes om te kiezen · Esc sluit").small().weak());
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
            self.loc.crs_job = Some(Job::spawn(ctx, "Stelsel laden…", move |_| crs::ensure(&c)));
        }
        self.map.center = [h.lon, h.lat];
        self.fit_pending = true;
        self.loc.query = h.label.clone();
        self.loc.hits_for = h.label.clone();
        self.loc.popup = false;
        self.loc.highlight = None;
        self.loc.typed_at = None;
        self.message = Some((true, format!("Locatie: {} — stelsel {code}. Klik op OK om op te slaan.", h.label)));
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
                    self.message = Some((true, format!("{} lagen (WMS {}).", c.layers.len(), c.version)));
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
                    self.message = Some((true, "Voorbeeld getoond op de kaart.".into()));
                }
                Err(e) => self.err(format!("Test mislukt: {e}")),
            }
        }
        if let Some(r) = poll(&mut self.raster.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(ui, "map", "Rasterdata laden met WMS / TMS", "Luchtfoto's, kaarten en hoogtebeelden als onderlegger in de tekening.");
        let Some(site) = self.saved_site(ui) else { return };
        let preview = self.raster.preview.take();
        self.map_section(ui, Some(&site), None, preview.as_ref(), &[]);
        self.raster.preview = preview;

        group(ui, "Webserver / verzoek", |ui| {
            egui::Grid::new("wms").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label("Voorgedefinieerde server");
                if let Some(s) = server_combo(ui, "wms_srv", &self.servers, &[ServerKind::Wms, ServerKind::Xyz], self.raster.server.as_ref()) {
                    self.raster.caps = None;
                    self.raster.layer = None;
                    self.raster.preview = None;
                    self.raster.filter.clear();
                    if s.kind == ServerKind::Wms {
                        let url = s.url.clone();
                        self.raster.caps_job = Some(Job::spawn(&ctx, "Capabilities ophalen…", move |_| caps::fetch_wms(&url)));
                        self.raster.resolution = 0.25;
                    } else {
                        self.raster.resolution = 0.5;
                    }
                    self.raster.server = Some(s);
                }
                ui.end_row();
                if let Some(s) = &self.raster.server {
                    ui.label("Bron");
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
                    ui.label("Laag");
                    ui.label(RichText::new(self.raster.layer.as_ref().map(|l| format!("{}  ({})", l.title, l.name)).unwrap_or_else(|| "— kies hieronder —".into())).strong());
                    ui.end_row();
                    ui.label("Formaat");
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
                ui.add(egui::TextEdit::singleline(&mut self.raster.filter).hint_text(format!("Filter {} lagen…", c.layers.len())).desired_width(f32::INFINITY));
                let mut chosen = None;
                list_frame(ui, 170.0, "wms_layers", |ui| {
                    let f = self.raster.filter.to_lowercase();
                    for l in c.layers.iter().filter(|l| f.is_empty() || l.name.to_lowercase().contains(&f) || l.title.to_lowercase().contains(&f)) {
                        let sel = self.raster.layer.as_ref().is_some_and(|x| x.name == l.name);
                        let reproj = !l.crs.is_empty() && !l.crs.contains(&site.crs);
                        let txt = format!("{}{}{}", l.title, if l.title != l.name { format!("   ({})", l.name) } else { String::new() }, if reproj { "   · herprojectie" } else { "" });
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
            group(ui, "Import-instellingen", |ui| {
                egui::Grid::new("wms_imp").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                    ui.label("Resolutie");
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
                    ui.label(RichText::new(format!("{w:.0} × {h:.0} px voor {} × {} m{}", site.width, site.height, if too_big { " — te groot: verhoog de resolutie of verklein het gebied" } else { "" }))
                        .color(if too_big { ERR_RED } else { ui.visuals().weak_text_color() }));
                    ui.end_row();
                    ui.label("Vervagen");
                    ui.add(egui::Slider::new(&mut self.raster.fade, 0..=80).suffix(" %"));
                    ui.end_row();
                    ui.label("Laag in tekening");
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
                    self.message = Some((true, format!("{} objecttypen (WFS {}).", c.types.len(), c.version)));
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
                    self.message = Some((true, format!("{} objecten opgehaald{} — zie de kaart.", res.features.len(), if res.truncated { " (limiet bereikt)" } else { "" })));
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

        page_header(ui, "vector", "2D-vectordata laden met WFS", "Percelen, panden, wegvakken, spoor en andere objecten als lijnen met labels.");
        let Some(site) = self.saved_site(ui) else { return };
        let preview = std::mem::take(&mut self.wfs.preview);
        self.map_section(ui, Some(&site), None, None, &preview);
        self.wfs.preview = preview;

        group(ui, "Webserver / verzoek", |ui| {
            egui::Grid::new("wfs").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label("Voorgedefinieerde server");
                if let Some(s) = server_combo(ui, "wfs_srv", &self.servers, &[ServerKind::Wfs], self.wfs.server.as_ref()) {
                    self.wfs.caps = None;
                    self.wfs.ft = None;
                    self.wfs.result = None;
                    self.wfs.preview.clear();
                    let url = s.url.clone();
                    self.wfs.caps_job = Some(Job::spawn(&ctx, "Capabilities ophalen…", move |_| caps::fetch_wfs(&url)));
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
                    ui.label("Objecttype");
                    ui.label(RichText::new(self.wfs.ft.as_ref().map(|t| format!("{}  ({})", t.title, t.name)).unwrap_or_else(|| "— kies hieronder —".into())).strong());
                    ui.end_row();
                    ui.label("Max. aantal objecten");
                    ui.add(egui::DragValue::new(&mut self.wfs.limit).speed(100).range(1..=200_000));
                    ui.end_row();
                }
            });
            if let Some(c) = &self.wfs.caps {
                ui.add_space(4.0);
                ui.add(egui::TextEdit::singleline(&mut self.wfs.filter).hint_text(format!("Filter {} objecttypen…", c.types.len())).desired_width(f32::INFINITY));
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

        group(ui, "Import-instellingen", |ui| {
            egui::Grid::new("wfs_imp").num_columns(2).spacing([14.0, 7.0]).min_col_width(170.0).show(ui, |ui| {
                ui.label("Label (attribuut)");
                let names = self.wfs.result.as_ref().map(|r| vector::property_names(&r.features)).unwrap_or_default();
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("wfs_label").selected_text(if self.wfs.label.is_empty() { "— geen —".to_string() } else { self.wfs.label.clone() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.wfs.label, String::new(), "— geen —");
                        for n in &names {
                            ui.selectable_value(&mut self.wfs.label, n.clone(), n);
                        }
                    });
                    if names.is_empty() {
                        ui.label(RichText::new("eerst Test om attributen te zien").small().weak());
                    }
                });
                ui.end_row();
                ui.label("Teksthoogte");
                ui.add(egui::DragValue::new(&mut self.wfs.text_m).speed(0.1).range(0.05..=100.0).suffix(" m"));
                ui.end_row();
                ui.label("Lijnkleur (laag)");
                if aci_combo(ui, "wfs_aci", &mut self.wfs.aci) {
                    let c = aci_color(self.wfs.aci);
                    self.wfs.preview.iter_mut().for_each(|(_, col)| *col = c);
                }
                ui.end_row();
                ui.label("Begrenzing");
                ui.checkbox(&mut self.wfs.clip, "Bijsnijden op het importgebied");
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
                    self.message = Some((true, format!("{n} OpenStreetMap-objecten opgehaald — zie de kaart.")));
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

        page_header(ui, "globe", "OpenStreetMap importeren", "Wereldwijd: gebouwen, wegen, spoor, water, kunstwerken, leidingen en meer — elk thema op een eigen laag.");
        let Some(site) = self.saved_site(ui) else { return };
        let fetched = self.osm.fetched.take();
        self.map_section(ui, Some(&site), None, None, fetched.as_ref().map(|f| f.2.as_slice()).unwrap_or(&[]));
        self.osm.fetched = fetched;

        group(ui, "Thema's", |ui| {
            egui::Grid::new("osm").num_columns(3).spacing([14.0, 5.0]).show(ui, |ui| {
                for (i, t) in THEMES.iter().enumerate() {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, aci_color(t.aci));
                    ui.checkbox(&mut self.osm.on[i], t.label);
                    ui.label(RichText::new(t.layer).monospace().small().weak());
                    ui.end_row();
                }
            });
        });
        group(ui, "Import-instellingen", |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.osm.labels, "Namen en huisnummers als tekst");
                ui.add(egui::DragValue::new(&mut self.osm.text_m).speed(0.1).range(0.1..=50.0).prefix("hoogte ").suffix(" m"));
            });
            ui.checkbox(&mut self.osm.clip, "Bijsnijden op het importgebied");
            ui.label(RichText::new("Data © OpenStreetMap-bijdragers (ODbL), via de Overpass API. Publieke Overpass-servers zijn soms druk; ophalen kan een minuut duren.").small().weak());
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
        page_header(ui, "pdok", "Nederlandse basisregistraties importeren (PDOK)", "Luchtfoto, kadaster, BAG, NWB en ProRail in één keer — kies wat u nodig hebt.");
        let Some(site) = self.saved_site(ui) else { return };
        self.map_section(ui, Some(&site), None, None, &[]);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, "Let op: de projectlocatie ligt buiten Nederland; PDOK levert hier geen data.");
        }
        let mut groups: Vec<&str> = Vec::new();
        for d in PDOK {
            if !groups.contains(&d.group) {
                groups.push(d.group);
            }
        }
        ui.columns(groups.len().max(1), |cols| {
            for (col, g) in cols.iter_mut().zip(&groups) {
                group(col, g, |ui| {
                    for (i, d) in PDOK.iter().enumerate().filter(|(_, d)| d.group == *g) {
                        ui.checkbox(&mut self.pdok.on[i], d.label);
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
        page_header(ui, "point", "Coördinaten omrekenen", "Klik op de kaart, typ een coördinaat of gebruik \"Punt info\" in het lint om een punt in de tekening aan te wijzen.");
        let site = bridge::site();
        let marker = self.coords.picked.or_else(|| crs::transform(&self.coords.crs, "EPSG:4326", self.coords.xy).ok());
        if let Some(MapEvent::Clicked(ll)) = self.map_section(ui, site.as_ref(), marker, None, &[]) {
            self.coords.picked = Some(ll);
            if let Ok(q) = crs::transform("EPSG:4326", &self.coords.crs, ll) {
                self.coords.xy = q;
            }
        }
        group(ui, "Invoer", |ui| {
            let mut codes: Vec<String> = crs::builtin_list().into_iter().map(|i| i.code).collect();
            if let Some(s) = &site {
                if !codes.contains(&s.crs) {
                    codes.push(s.crs.clone());
                }
            }
            let geo = crs::is_geographic(&self.coords.crs);
            ui.horizontal(|ui| {
                ui.label("Stelsel");
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
        group(ui, "Resultaat", |ui| {
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
            if ui.add_enabled(!busy, primary_button("OK")).on_hover_text("Projectlocatie opslaan in de tekening").clicked() {
                match bridge::save_site(&self.draft) {
                    Ok(()) => {
                        self.has_site = true;
                        let site = self.draft.clone();
                        let (origin, aerial) = (self.loc.geo_origin, self.loc.aerial);
                        let summary = format!("Projectlocatie opgeslagen: {} — {} X {:.2} Y {:.2}, {}×{} m.", site.label, site.crs, site.origin_e, site.origin_n, site.width, site.height);
                        if origin || aerial {
                            self.loc.apply = Some(Job::spawn(&ctx, "Bezig…", move |p| {
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
                    Err(e) => self.err(format!("Opslaan mislukt: {e}")),
                }
            }
            if ui.button("Annuleren").on_hover_text("Wijzigingen terugdraaien en het venster sluiten").clicked() {
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
            if ui.add_enabled(ready && !too_big && self.raster.run.is_none(), primary_button("OK en importeren")).clicked() {
                let server = self.raster.server.clone().unwrap();
                let opt = raster::Options { resolution: self.raster.resolution, format: self.raster.format.clone() };
                let (fade, caps, layer, s) = (self.raster.fade, self.raster.caps.clone(), self.raster.layer.clone(), site.clone());
                self.raster.run = Some(Job::spawn(&ctx, "Importeren…", move |p| match server.kind {
                    ServerKind::Xyz => import::xyz(&s, &server, &opt, fade, p),
                    _ => import::wms(&s, &server, caps.as_ref().ok_or("geen capabilities")?, layer.as_ref().ok_or("geen laag")?, &opt, fade, p),
                }));
            }
            if ui.add_enabled(ready && self.raster.preview_job.is_none(), egui::Button::new("Test — voorbeeld op kaart")).clicked() {
                let (server, caps, layer, fmt, s) = (self.raster.server.clone().unwrap(), self.raster.caps.clone(), self.raster.layer.clone(), self.raster.format.clone(), site.clone());
                self.raster.preview_job = Some(Job::spawn(&ctx, "Voorbeeld ophalen…", move |_| match server.kind {
                    ServerKind::Xyz => raster::preview_xyz(&s, &server),
                    _ => raster::preview_wms(&s, caps.as_ref().ok_or("geen capabilities")?, layer.as_ref().ok_or("geen laag")?, &fmt, 900),
                }));
            }
            if self.raster.preview.is_some() && ui.button("Voorbeeld wissen").clicked() {
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
            if ui.add_enabled(can, primary_button("OK en importeren")).clicked() {
                let style = VectorStyle { aci: self.wfs.aci, label_field: (!self.wfs.label.is_empty()).then(|| self.wfs.label.clone()), text_m: self.wfs.text_m, clip: self.wfs.clip };
                let (s, ft, caps, limit, cached) = (site.clone(), self.wfs.ft.clone().unwrap(), self.wfs.caps.clone(), self.wfs.limit, self.wfs.result.take());
                self.wfs.preview.clear();
                self.wfs.place = Some(Job::spawn(&ctx, "Importeren…", move |p| {
                    let res = match cached {
                        Some(r) => r,
                        None => import::wfs_fetch(&s, caps.as_ref().ok_or("geen capabilities")?, &ft, limit, p)?,
                    };
                    import::wfs_place(&s, &ft, &res, &style)
                }));
            }
            if ui.add_enabled(can, egui::Button::new("Test — ophalen en tonen")).clicked() {
                let (s, c, t, n) = (site.clone(), self.wfs.caps.clone().unwrap(), self.wfs.ft.clone().unwrap(), self.wfs.limit);
                self.wfs.fetch = Some(Job::spawn(&ctx, "Objecten ophalen…", move |p| {
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
            if ui.add_enabled(!ids.is_empty() && !busy, primary_button("OK en importeren")).clicked() {
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
            if ui.add_enabled(!ids.is_empty() && !busy, egui::Button::new("Test — ophalen en tonen")).clicked() {
                let (s, ids2) = (site.clone(), ids.clone());
                self.osm.fetch = Some(Job::spawn(&ctx, "OpenStreetMap ophalen…", move |p| {
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
            if ui.add_enabled(!chosen.is_empty() && self.pdok.run.is_none(), primary_button("OK en importeren")).clicked() {
                let s = site.clone();
                self.pdok.run = Some(Job::spawn(&ctx, "PDOK…", move |p| {
                    let mut done = Vec::new();
                    for (k, i) in chosen.iter().enumerate() {
                        let d = &PDOK[*i];
                        p(format!("{}/{}: {}…", k + 1, chosen.len(), d.label));
                        match import::run_dataset(&s, &d.data, &|m: String| p(format!("{}/{}: {} — {m}", k + 1, chosen.len(), d.label))) {
                            Ok(_) => done.push(d.label.to_string()),
                            Err(e) => done.push(format!("{} (mislukt: {e})", d.label)),
                        }
                    }
                    Ok(format!("PDOK geïmporteerd: {}.", done.join("; ")))
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
        self.nlcs.klic_job = Some(Job::spawn(ctx, "KLIC-levering lezen…", move |_| {
            let lev = klic::read(&path)?;
            let preview = klic_preview(&site, &lev)?;
            Ok((lev, preview))
        }));
    }

    fn nlcs_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.nlcs.klic_job) {
            match r {
                Ok((lev, preview)) => {
                    self.message = Some((true, format!("KLIC {}: {} objecten van {} netbeheerders.", lev.meldnummer, lev.features.len(), lev.operators.len())));
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
                    self.message = Some((true, format!("{n} BGT-objecten opgehaald — zie de kaart.")));
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

        page_header(ui, "nlcs", "BGT, kadaster en KLIC conform NLCS", "Basisdata voor een infra-ontwerp: elk object op zijn NLCS-laag, met kleur, lijndikte, lijntype en objectgegevens.");
        let Some(site) = self.saved_site(ui) else { return };
        let mut lines: Preview2 = self.nlcs.fetched.as_ref().map(|f| f.4.clone()).unwrap_or_default();
        lines.extend(self.nlcs.klic_preview.iter().cloned());
        self.map_section(ui, Some(&site), None, None, &lines);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, "Let op: BGT, BRK en KLIC bestaan alleen voor Nederland.");
        }

        group(ui, "Tekenstandaard NLCS 5.02", |ui| {
            egui::Grid::new("nlcs_std").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Tekenstatus");
                egui::ComboBox::from_id_salt("nlcs_status").selected_text(self.nlcs.status.label()).show_ui(ui, |ui| {
                    for st in Status::ALL {
                        ui.selectable_value(&mut self.nlcs.status, st, st.label());
                    }
                });
                ui.label("Schaal");
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
                ui.label("Discipline BGT/BRK");
                combo(ui, "nlcs_disc", &mut self.nlcs.discipline);
                ui.label("Discipline KLIC");
                combo(ui, "nlcs_disc_klic", &mut self.nlcs.klic_discipline);
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.nlcs.object_data, "Objectgegevens (XDATA)");
                ui.checkbox(&mut self.nlcs.labels, "Teksten: straatnamen, huisnummers, perceelnummers, leidinglabels");
                ui.checkbox(&mut self.nlcs.clip, "Bijsnijden op het importgebied");
            });
            ui.label(RichText::new("Laagnaam: STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT, bijv. B-XX-VH-GESLOTENVERHARDING_ASFALT-G. Lijntypen in papier-mm met de lijntypeschaal van de tekenschaal; teksten krijgen de stijl van hun objectlaag, de teksten van het tekenblad volgen de NLCS-tekstklassen.").small().weak());
        });

        ui.columns(2, |cols| {
            group(&mut cols[0], "BGT (PDOK, actueel)", |ui| {
                for (i, g) in bgt::GROUPS.iter().enumerate() {
                    ui.checkbox(&mut self.nlcs.bgt_on[i], g.label);
                }
            });
            group(&mut cols[1], "Kadastrale kaart (BRK)", |ui| {
                ui.checkbox(&mut self.nlcs.brk, "Perceelgrenzen en -nummers (KG-GRENS_PERCEEL)");
            });
            group(&mut cols[1], "Tekenblad", |ui| {
                ui.checkbox(&mut self.nlcs.sheet, "Kader, titelblok, legenda, noordpijl, schaalbalk en RD-kruisen");
                let (paper, _, _) = tekenblad::paper_for(&site, self.nlcs.scale);
                ui.label(RichText::new(format!("Formaat bij 1:{}: {paper} liggend", self.nlcs.scale)).small().weak());
                egui::Grid::new("sheet").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Project");
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.project).hint_text(site.label.as_str()).desired_width(240.0));
                    ui.end_row();
                    ui.label("Onderwerp");
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.subject).desired_width(240.0));
                    ui.end_row();
                    ui.label("Tekeningnummer");
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.number).desired_width(240.0));
                    ui.end_row();
                    ui.label("Getekend");
                    ui.add(egui::TextEdit::singleline(&mut self.nlcs.info.drawn_by).desired_width(240.0));
                    ui.end_row();
                });
            });
        });

        group(ui, "KLIC-levering (WIBON, IMKL 2.0)", |ui| {
            ui.horizontal(|ui| {
                ui.label("Bestand");
                ui.add(egui::TextEdit::singleline(&mut self.nlcs.klic_path).hint_text("Levering_….zip of GI_gebiedsinformatielevering_….xml — of sleep het bestand op dit venster").desired_width(ui.available_width() - 90.0));
                if ui.add_enabled(!self.nlcs.klic_path.trim().is_empty() && self.nlcs.klic_job.is_none(), egui::Button::new("Lezen")).clicked() {
                    let ctx = ui.ctx().clone();
                    self.load_klic(&ctx);
                }
            });
            job_status(ui, &self.nlcs.klic_job);
            if let Some(lev) = self.nlcs.klic.clone() {
                ui.label(RichText::new(format!("Melding {} · {} objecten · {}", lev.meldnummer, lev.features.len(), lev.operators.join(", "))).small());
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
                    ui.checkbox(&mut self.nlcs.imkl_colors, "IMKL-themakleuren in plaats van NLCS-kleuren");
                    if ui.button("Importgebied op deze levering zetten").clicked() {
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
                                        self.ok("Importgebied aangepast aan de KLIC-levering.");
                                    }
                                    Err(e) => self.err(e),
                                }
                            }
                        }
                    }
                });
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
            if ui.add_enabled(anything && !busy, primary_button("OK en importeren")).clicked() {
                let cached = self.nlcs.fetched.take().filter(|f| f.0 == collections && f.1 == brk);
                let (opt, kopt, lev, sheet, info) = (self.nlcs.options(), self.nlcs.klic_options(), self.nlcs.klic.clone().filter(|_| klic_ready), self.nlcs.sheet, self.nlcs.info.clone());
                let (s, cols) = (site.clone(), collections.clone());
                self.nlcs.run = Some(Job::spawn(&ctx, "Basisdata…", move |p| {
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
                        p("BGT tekenen…".into());
                        done.push(basisdata::bgt_place(&s, &bgt_data, &opt)?);
                    }
                    if let Some(c) = cad {
                        p("Kadaster tekenen…".into());
                        done.push(basisdata::cadastre_place(&s, &c, &opt)?);
                    }
                    if let Some(lev) = lev {
                        p("KLIC tekenen…".into());
                        done.push(basisdata::klic_place(&s, &lev, &kopt)?);
                    }
                    if sheet {
                        p("Tekenblad…".into());
                        done.push(tekenblad::draw(&s, &opt, &info)?);
                    }
                    bridge::zoom_extents();
                    Ok(done.join(" "))
                }));
            }
            if ui.add_enabled((!collections.is_empty() || brk) && !busy, egui::Button::new("Test — ophalen en tonen")).clicked() {
                let (s, cols, labels) = (site.clone(), collections.clone(), self.nlcs.labels);
                self.nlcs.fetch = Some(Job::spawn(&ctx, "BGT ophalen…", move |p| {
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
                    self.message = Some((true, format!("AHN: {} – {} m NAP, {} hoogtelijnen — zie de kaart.", hoogte::nap(p.1 .0, 2), hoogte::nap(p.1 .1, 2), p.0.len())));
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
        page_header(ui, "ahn", "AHN-hoogtes, profielen en puntenwolk", "Actueel Hoogtebestand Nederland: hoogtepunten, hoogtelijnen en lengteprofielen uit AHN4, de puntenwolk uit AHN5.");
        let Some(site) = self.saved_site(ui) else { return };
        let lines = self.ahn.preview.as_ref().map(|p| p.0.clone()).unwrap_or_default();
        self.map_section(ui, Some(&site), None, None, &lines);
        if !(site.lon > 3.2 && site.lon < 7.3 && site.lat > 50.7 && site.lat < 53.7) {
            ui.colored_label(WARN, "Let op: AHN bestaat alleen voor Nederland.");
        }
        let o = &mut self.ahn.opt;
        group(ui, "Tekenen", |ui| {
            ui.horizontal(|ui| {
                ui.label("Schaal");
                egui::ComboBox::from_id_salt("ahn_scale").selected_text(format!("1:{}", o.nlcs.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0] {
                        ui.selectable_value(&mut o.nlcs.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.label("Tekenstatus");
                egui::ComboBox::from_id_salt("ahn_status").selected_text(o.nlcs.status.label()).show_ui(ui, |ui| {
                    for st in nlcs::Status::ALL {
                        ui.selectable_value(&mut o.nlcs.status, st, st.label());
                    }
                });
                ui.checkbox(&mut o.nlcs.labels, "Hoogtes als tekst");
                ui.checkbox(&mut o.nlcs.clip, "Bijsnijden op het importgebied");
            });
            ui.label(RichText::new("Hoogtes in m NAP; lijnen en punten liggen op hun hoogte (Z). Lagen: OG-HOOGTEPUNT, OG-HOOGTELIJN_MINOR/MAJOR, GW-MAAIVELD, AL-PROFIELBALK.").small().weak());
        });
        ui.columns(2, |cols| {
            group(&mut cols[0], "Hoogtepunten", |ui| {
                ui.checkbox(&mut self.ahn.spots, "Hoogtepunten in een raster");
                ui.horizontal(|ui| {
                    ui.label("Raster");
                    egui::ComboBox::from_id_salt("ahn_spacing").selected_text(format!("{} m", o.spot_spacing)).show_ui(ui, |ui| {
                        for v in [2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0] {
                            ui.selectable_value(&mut o.spot_spacing, v, format!("{v} m"));
                        }
                    });
                    ui.label("Model");
                    egui::ComboBox::from_id_salt("ahn_model").selected_text(o.model.label()).show_ui(ui, |ui| {
                        for m in [Model::Dtm, Model::Dsm] {
                            ui.selectable_value(&mut o.model, m, m.label());
                        }
                    });
                });
            });
            group(&mut cols[0], "Hoogtelijnen (maaiveld)", |ui| {
                ui.checkbox(&mut self.ahn.contours, "Hoogtelijnen");
                ui.horizontal(|ui| {
                    ui.label("Interval");
                    egui::ComboBox::from_id_salt("ahn_interval").selected_text(format!("{} m", o.interval)).show_ui(ui, |ui| {
                        for v in [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.interval, v, format!("{v} m"));
                        }
                    });
                    ui.label("hoofdlijn elke");
                    ui.add(egui::DragValue::new(&mut o.major_every).range(1..=20));
                });
                ui.horizontal(|ui| {
                    ui.label("Rastercel");
                    egui::ComboBox::from_id_salt("ahn_cell").selected_text(format!("{} m", o.contour_cell)).show_ui(ui, |ui| {
                        for v in [0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.contour_cell, v, format!("{v} m"));
                        }
                    });
                    ui.checkbox(&mut o.smooth, "Gladstrijken");
                });
            });
            group(&mut cols[1], "Lengteprofiel", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Stations om de");
                    ui.add(egui::DragValue::new(&mut o.step).range(0.25..=50.0).speed(0.25).suffix(" m"));
                    ui.label("overhoging");
                    ui.add(egui::DragValue::new(&mut o.exaggeration).range(1.0..=50.0).speed(0.5).suffix("×"));
                });
                ui.checkbox(&mut o.surface, "Ook het oppervlak (DSM: gebouwen, bomen)");
                ui.label(RichText::new("Kies in Open CAD Studio Profiel in het lint (of GIS_PROFIEL): selecteer eerst een polyline of wijs de punten aan, Enter, en wijs dan de linkeronderhoek van het profiel aan.").small().weak());
            });
            group(&mut cols[1], "Puntenwolk (COPC)", |ui| {
                ui.checkbox(&mut self.ahn.cloud, "AHN-puntenwolk koppelen (.rcs naast de tekening)");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("ahn_ed").selected_text(o.cloud.edition.label()).show_ui(ui, |ui| {
                        for e in Edition::ALL {
                            ui.selectable_value(&mut o.cloud.edition, e, e.label());
                        }
                    });
                    ui.label("puntafstand");
                    egui::ComboBox::from_id_salt("ahn_spacing_pc").selected_text(format!("{} m", o.cloud.spacing)).show_ui(ui, |ui| {
                        for v in [0.1, 0.25, 0.5, 1.0, 2.0] {
                            ui.selectable_value(&mut o.cloud.spacing, v, format!("{v} m"));
                        }
                    });
                    ui.label("kleur");
                    egui::ComboBox::from_id_salt("ahn_col").selected_text(o.cloud.colouring.label()).show_ui(ui, |ui| {
                        for c in Colouring::ALL {
                            ui.selectable_value(&mut o.cloud.colouring, c, c.label());
                        }
                    });
                });
                egui::Grid::new("ahn_classes").num_columns(3).spacing([10.0, 3.0]).show(ui, |ui| {
                    for (i, (_, label, rgb)) in pointcloud::CLASSES.iter().enumerate() {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        ui.checkbox(&mut self.ahn.class_on[i], *label);
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                let area = site.width * site.height;
                let estimate = area * (1.0 / (o.cloud.spacing * o.cloud.spacing)).min(20.0);
                ui.label(RichText::new(format!("Ongeveer {:.1} miljoen punten voor {:.0} × {:.0} m; alleen de nodige delen van de 1 km-tegels worden gedownload.", estimate / 1e6, site.width, site.height)).small().weak());
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
            if ui.add_enabled(any && !busy, primary_button("OK en importeren")).clicked() {
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
            if ui.add_enabled(!busy, egui::Button::new("Test — hoogtelijnen op de kaart")).clicked() {
                let (s, opt) = (site.clone(), self.ahn.opt.clone());
                self.ahn.test = Some(Job::spawn(&ctx, "AHN ophalen…", move |p| {
                    let b = crate::basisdata::rd_bbox(&s)?;
                    let mut grid = ahn::fetch(b, Model::Dtm, opt.contour_cell, p)?;
                    if opt.smooth {
                        grid = grid.smoothed();
                    }
                    let range = grid.range().ok_or("AHN heeft hier geen hoogtes")?;
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
        self.controle.job = Some(Job::spawn(ctx, "Tekening controleren…", move |_| {
            let doc = bridge::snapshot()?;
            let opt = nlcs_check::Options { layer_props: !bridge::partial(), ..opt };
            Ok(nlcs_check::check(&doc, &opt))
        }));
    }

    fn controle_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.controle.job) {
            match r {
                Ok(rep) => {
                    self.message = Some((true, format!("NLCS-controle: {} conform — {}, {}.", rep.score_label(), nlcs_check::counted(rep.count(Severity::Fout), "fout", "fouten"), nlcs_check::counted(rep.count(Severity::Waarschuwing), "waarschuwing", "waarschuwingen"))));
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
        page_header(ui, "check", "NLCS-controle", "Toetst de tekening aan NLCS 5.02: laagnamen, laagkleur, -lijndikte en -lijntype per tekenstatus, elementen op DuLaag, teksthoogtes.");
        group(ui, "Instellingen", |ui| {
            ui.horizontal(|ui| {
                ui.label("Tekenschaal (voor teksthoogtes)");
                egui::ComboBox::from_id_salt("chk_scale").selected_text(format!("1:{}", self.controle.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0] {
                        ui.selectable_value(&mut self.controle.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.checkbox(&mut self.controle.text_heights, "Teksthoogtes");
                ui.checkbox(&mut self.controle.empty_layers, "Lege lagen melden");
            });
        });
        group(ui, "BGT-mutatie", |ui| {
            ui.horizontal(|ui| {
                ui.label("Bronhouder");
                ui.add(egui::TextEdit::singleline(&mut self.controle.bronhouder).desired_width(70.0).hint_text("G0505"))
                    .on_hover_text("Code van de gemeente (G + CBS-code). Leeg: die van de BGT-objecten in de tekening.");
                ui.checkbox(&mut self.controle.as_built, "Gerealiseerd: lagen N als revisie (geen plantopografie)");
            });
            ui.label(RichText::new("Nieuw (N) wordt plantopografie, revisie (R) gerealiseerd, V beëindigt het BGT-object. Type en functie komen uit de BGT-gegevens, het BGT-object eronder of de NLCS-laag; wat niet eenduidig is, staat in het rapport.").small().weak());
        });
        let Some(rep) = self.controle.report.clone() else {
            group(ui, "Resultaat", |ui| {
                job_status(ui, &self.controle.job);
                if self.controle.job.is_none() {
                    ui.label(RichText::new("Klik op Controleren.").weak());
                }
            });
            return;
        };
        group(ui, "Resultaat", |ui| {
            ui.horizontal(|ui| {
                let score = rep.score();
                let colour = if score >= 95.0 { OK_GREEN } else if score >= 70.0 { WARN } else { ERR_RED };
                ui.label(RichText::new(rep.score_label()).size(30.0).strong().color(colour));
                ui.vertical(|ui| {
                    ui.label(RichText::new("conform NLCS").strong());
                    ui.label(RichText::new(format!("{} van {} elementen · {} lagen, waarvan {} NLCS", rep.conform_entities, rep.entities, rep.layers, rep.nlcs_layers)).small().weak());
                });
                ui.add_space(24.0);
                for s in [Severity::Fout, Severity::Waarschuwing, Severity::Info] {
                    badge(ui, &format!("{} {}", rep.count(s), s.label()), severity_colour(s));
                }
            });
            let bar = egui::ProgressBar::new((rep.score() / 100.0) as f32).desired_height(6.0);
            ui.add(bar);
        });
        group(ui, "Bevindingen", |ui| {
            if rep.findings.is_empty() {
                ui.label(RichText::new("Geen bevindingen: de tekening volgt NLCS.").color(OK_GREEN));
                return;
            }
            for (rule, list) in rep.by_rule() {
                let open = self.controle.open == Some(rule);
                ui.horizontal(|ui| {
                    badge(ui, rule.severity().label(), severity_colour(rule.severity()));
                    let title = format!("{}  {}  ({}×)", rule.code(), rule.title(), list.len());
                    if ui.selectable_label(open, RichText::new(title).strong()).clicked() {
                        self.controle.open = if open { None } else { Some(rule) };
                    }
                    let handles: Vec<_> = list.iter().flat_map(|f| f.handles.iter().copied()).collect();
                    if !handles.is_empty() && ui.small_button(format!("Selecteer {}", handles.len())).on_hover_text("Selecteer deze elementen in de tekening").clicked() {
                        match bridge::select(&handles) {
                            Ok(()) => self.message = Some((true, format!("{} element(en) geselecteerd in de tekening.", handles.len()))),
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
                                if !f.handles.is_empty() && ui.small_button("Selecteer").clicked() {
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
            if ui.add_enabled(!busy, primary_button("Controleren")).clicked() {
                self.start_check(&ctx);
            }
            let label = if repairs > 0 { format!("Herstellen ({repairs})") } else { "Herstellen".into() };
            if ui.add_enabled(!busy && repairs > 0, egui::Button::new(label)).on_hover_text("Laagkleur, -lijndikte en -lijntype volgens NLCS; elementen op DuLaag. Laagnamen en teksthoogtes blijven zoals ze zijn.").clicked() {
                if let Some(rep) = self.controle.report.clone() {
                    self.controle.repair = Some(Job::spawn(&ctx, "Herstellen…", move |_| {
                        let doc = bridge::snapshot()?;
                        nlcs_check::repair(&doc, &rep.repairs())
                    }));
                }
            }
            if ui.add_enabled(self.controle.report.is_some(), egui::Button::new("Rapport opslaan")).clicked() {
                if let Some(rep) = &self.controle.report {
                    let opt = nlcs_check::Options { scale: self.controle.scale, ..Default::default() };
                    let path = bridge::beside_drawing("nlcs-controle.md");
                    match std::fs::write(&path, nlcs_check::markdown(rep, &bridge::drawing_name(), &opt)) {
                        Ok(()) => self.ok(format!("Rapport opgeslagen: {}", path.display())),
                        Err(e) => self.err(format!("{}: {e}", path.display())),
                    }
                }
            }
            let (bronhouder, as_built) = (self.controle.bronhouder.clone(), self.controle.as_built);
            if ui
                .add_enabled(!busy && self.controle.mutation.is_none(), egui::Button::new("BGT-mutatie"))
                .on_hover_text("Het ontwerp op NLCS-lagen als BGT-mutatie (IMGeo-GML 2.1.1) met een rapport, naast de tekening. N = plantopografie, R = gerealiseerd, V = beëindigen.")
                .clicked()
            {
                self.controle.mutation = Some(Job::spawn(&ctx, "BGT-mutatie maken…", move |_| crate::bgt_mutatie::export(&bronhouder, as_built, None)));
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
        self.hoev.job = Some(Job::spawn(ctx, "Hoeveelheden berekenen…", move |_| {
            let doc = bridge::snapshot()?;
            let sel: std::collections::HashSet<_> = if only_selection { bridge::selection()?.into_iter().collect() } else { Default::default() };
            if only_selection && sel.is_empty() {
                return Err("niets geselecteerd in de tekening".into());
            }
            Ok(hoeveelheden::compute(&doc, unit, if only_selection { Some(&sel) } else { None }))
        }));
    }

    fn hoeveelheden_page(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = poll(&mut self.hoev.job) {
            match r {
                Ok(q) => {
                    self.message = Some((true, format!("Hoeveelheden van {} elementen op {} lagen.", q.entities, q.rows.len())));
                    self.hoev.result = Some(q);
                }
                Err(e) => self.err(e),
            }
        }
        page_header(ui, "quantities", "Hoeveelheden", "Aantal, lengte en oppervlakte per NLCS-laag of per object, uit de geometrie van de tekening.");
        group(ui, "Instellingen", |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.hoev.selection, "Alleen de selectie in de tekening");
                ui.checkbox(&mut self.hoev.per_object, "Per object (G, S en A samen; zonder teksten)");
                ui.add_enabled(!self.hoev.per_object, egui::Checkbox::new(&mut self.hoev.annotation, "Ook tekenblad en annotatie (AL, ZZ)"));
                ui.add(egui::TextEdit::singleline(&mut self.hoev.filter).hint_text("filter, bijv. VH of asfalt").desired_width(200.0));
            });
        });
        let Some(q) = self.hoev.result.clone() else {
            group(ui, "Resultaat", |ui| {
                job_status(ui, &self.hoev.job);
                if self.hoev.job.is_none() {
                    ui.label(RichText::new("Klik op Berekenen.").weak());
                }
            });
            return;
        };
        let rows = if self.hoev.per_object { hoeveelheden::per_object(&q) } else { q.rows.clone() };
        let filter = self.hoev.filter.trim().to_lowercase();
        let annotation = self.hoev.annotation;
        let rows: Vec<_> = rows.into_iter().filter(|r| annotation || !r.annotation()).filter(|r| filter.is_empty() || r.layer.to_lowercase().contains(&filter) || r.object_label().to_lowercase().contains(&filter)).collect();
        group(ui, &format!("Resultaat — {} regels{}", rows.len(), if q.from_selection { " (selectie)" } else { "" }), |ui| {
            list_frame(ui, 520.0, "quantities", |ui| {
                egui::Grid::new("q").num_columns(7).striped(true).spacing([14.0, 5.0]).show(ui, |ui| {
                    for h in ["Object", "Status", "Laag", "Aantal", "Lengte (m)", "Opp. (m²)", "Hoofdmaat"] {
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
                        let (v, u) = r.main();
                        ui.label(RichText::new(format!("{} {u}", nl(v, if u == "st" { 0 } else { 1 }))).strong());
                        ui.end_row();
                        tl += r.length;
                        ta += r.area;
                    }
                    ui.label(RichText::new("Totaal").strong());
                    ui.label("");
                    ui.label("");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(rows.iter().map(|r| r.count).sum::<usize>().to_string()).strong()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(nl(tl, 2)).strong()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(nl(ta, 2)).strong()));
                    ui.end_row();
                });
            });
            ui.label(RichText::new("Oppervlakte: gesloten polylijnen (met bogen), cirkels, ellipsen en arceringen (met eilanden). Lengte: lijnen, bogen, polylijnen, splines. Een hoeveelhedenstaat in de tekening: lintknop Hoeveelhedenstaat of GIS_HOEVEELHEDEN_TABEL.").small().weak());
        });
    }

    fn hoeveelheden_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        button_bar(ui, |ui| {
            if ui.add_enabled(self.hoev.job.is_none(), primary_button("Berekenen")).clicked() {
                self.start_quantities(&ctx);
            }
            if ui.add_enabled(self.hoev.result.is_some(), egui::Button::new("CSV opslaan")).on_hover_text("Puntkomma's en decimale komma's, voor Excel en LibreOffice").clicked() {
                if let Some(q) = &self.hoev.result {
                    let path = bridge::beside_drawing("hoeveelheden.csv");
                    match std::fs::write(&path, hoeveelheden::csv(q)) {
                        Ok(()) => self.ok(format!("Hoeveelheden opgeslagen: {}", path.display())),
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
        page_header(ui, "axis", "Civiel — instellingen", "Civiele ontwerptools op het tabblad Civiel: meetpunten, terrein (TIN), assen, profielen en grondverzet. De instellingen gelden ook voor de knoppen in het lint.");
        let o = &mut self.civ.opt;
        let num = |ui: &mut egui::Ui, v: &mut f64, lo: f64, hi: f64, speed: f64, suffix: &str| {
            ui.add(egui::DragValue::new(v).range(lo..=hi).speed(speed).suffix(suffix));
        };
        group(ui, "Tekenen", |ui| {
            ui.horizontal(|ui| {
                ui.label("Schaal");
                egui::ComboBox::from_id_salt("civ_scale").selected_text(format!("1:{}", o.nlcs.scale)).show_ui(ui, |ui| {
                    for sc in [100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0] {
                        ui.selectable_value(&mut o.nlcs.scale, sc, format!("1:{sc}"));
                    }
                });
                ui.label("Tekenstatus");
                egui::ComboBox::from_id_salt("civ_status").selected_text(o.nlcs.status.label()).show_ui(ui, |ui| {
                    for st in nlcs::Status::ALL {
                        ui.selectable_value(&mut o.nlcs.status, st, st.label());
                    }
                });
                ui.checkbox(&mut o.nlcs.labels, "Teksten");
            });
            ui.label(RichText::new("Alles op NLCS-lagen: OG-HOOGTEPUNT, OG-TRIANGULATIE, OG-HOOGTELIJN, AM-AS, AM-KILOMETRERING, AM-TANGENTPUNT, AL-PROFIELBALK, GW-MAAIVELD.").small().weak());
        });
        ui.columns(2, |cols| {
            group(&mut cols[0], "Punten", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Bestandsformaat");
                    egui::ComboBox::from_id_salt("civ_format").selected_text(o.format.label()).show_ui(ui, |ui| {
                        for f in Format::ALL {
                            ui.selectable_value(&mut o.format, f, f.label());
                        }
                    });
                });
                ui.checkbox(&mut o.point_labels, "Puntnummer, hoogte en code bij elk punt");
                ui.label(RichText::new("Scheidingsteken (; , tab spatie) en decimale komma worden herkend. Automatisch: noord/oost uit de getallen.").small().weak());
            });
            group(&mut cols[0], "Oppervlak", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Langste driehoekszijde");
                    num(ui, &mut o.max_edge, 1.0, 10_000.0, 1.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label("TIN uit AHN: puntafstand");
                    num(ui, &mut o.ahn_spacing, 0.5, 100.0, 0.5, " m");
                });
                ui.horizontal(|ui| {
                    ui.label("Hoogtelijnen om de");
                    egui::ComboBox::from_id_salt("civ_interval").selected_text(format!("{} m", o.interval)).show_ui(ui, |ui| {
                        for v in [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 5.0] {
                            ui.selectable_value(&mut o.interval, v, format!("{v} m"));
                        }
                    });
                    ui.label("hoofdlijn elke");
                    ui.add(egui::DragValue::new(&mut o.major_every).range(1..=20));
                });
                ui.horizontal(|ui| {
                    ui.label("Hoogteanalyse");
                    ui.add(egui::DragValue::new(&mut o.bands).range(2..=9).suffix(" banden"));
                });
                ui.horizontal(|ui| {
                    ui.label("Grondverzet: raster");
                    num(ui, &mut o.volume_cell, 0.1, 50.0, 0.1, " m");
                    ui.label("peil");
                    num(ui, &mut o.level, -100.0, 1000.0, 0.05, " m");
                });
                ui.label(RichText::new("TIN: selecteer punten, 3D-lijnen of hoogtelijnen; zonder selectie uit AHN in het importgebied. Grondverzet: selecteer de 3D-vlakken van het ontwerp; zonder selectie t.o.v. het peil.").small().weak());
            });
            group(&mut cols[1], "As", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Streepjes om de");
                    num(ui, &mut o.tick, 1.0, 1000.0, 1.0, " m");
                    ui.label("stationering om de");
                    num(ui, &mut o.label_every, 1.0, 5000.0, 10.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label("Verschuivingen (m, links < 0)");
                    if ui.add(egui::TextEdit::singleline(&mut self.civ.offsets).desired_width(140.0)).changed() {
                        o.offsets = parse_offsets(&self.civ.offsets);
                    }
                });
            });
            group(&mut cols[1], "Profielen", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Lengteprofiel: stap");
                    num(ui, &mut o.step, 0.1, 50.0, 0.1, " m");
                    ui.label("overhoging");
                    num(ui, &mut o.exaggeration, 1.0, 50.0, 0.5, "×");
                });
                ui.horizontal(|ui| {
                    ui.label("Ontwerpprofiel: boog");
                    num(ui, &mut o.vertical_curve, 0.0, 2000.0, 5.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label("Dwarsprofielen om de");
                    num(ui, &mut o.section_every, 1.0, 1000.0, 1.0, " m");
                    ui.label("links");
                    num(ui, &mut o.left, 1.0, 500.0, 1.0, " m");
                    ui.label("rechts");
                    num(ui, &mut o.right, 1.0, 500.0, 1.0, " m");
                });
                ui.horizontal(|ui| {
                    ui.label("overhoging");
                    num(ui, &mut o.section_exaggeration, 1.0, 20.0, 0.5, "×");
                    ui.label("per rij");
                    ui.add(egui::DragValue::new(&mut o.per_row).range(1..=20));
                });
                ui.label(RichText::new("Selecteer de as in de tekening en kies Lengteprofiel of Dwarsprofielen in het lint; klik dan waar ze komen. Ontwerpprofiel: teken een polylijn in het lengteprofiel en selecteer die.").small().weak());
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
                ("TIN maken", (|p| terrein::create(&settings(), terrein::TinSource::Auto, p)) as fn(&dyn Fn(String)) -> Result<String, String>),
                ("Hoogtelijnen", |p| terrein::contours(&settings(), terrein::Surface::Auto, p)),
                ("Hoogteanalyse", |_| terrein::analysis(&settings(), terrein::Surface::Auto)),
                ("Grondverzet", |p| terrein::volume(&settings(), terrein::Design::Auto, p)),
                ("As maken", |_| as_profiel::create(&settings(), as_profiel::AxisInput::Line(None), None, None)),
                ("As verschuiven", |_| as_profiel::offsets(&settings(), None)),
                ("Ontwerpprofiel", |_| as_profiel::design_profile(&settings(), None, None, None)),
                ("Punten exporteren", |_| landmeten::export(&settings(), None)),
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
                    let mut user = servers::load_user();
                    let name = if self.srv.name.trim().is_empty() { title } else { self.srv.name.trim().to_string() };
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    user.push(MapServer {
                        id: format!("user-{stamp}"),
                        kind,
                        name: name.clone(),
                        region: if self.srv.region.trim().is_empty() { "Eigen servers".into() } else { self.srv.region.trim().into() },
                        url: if kind == ServerKind::Xyz { self.srv.url.trim().to_string() } else { crate::gis::http::clean_service_url(&self.srv.url) },
                        attribution: String::new(),
                        default_layer: String::new(),
                        max_zoom: (kind == ServerKind::Xyz).then_some(19),
                        builtin: false,
                    });
                    match servers::save_user(&user) {
                        Ok(()) => {
                            self.servers = servers::all();
                            self.svc.refresh();
                            self.srv.url.clear();
                            self.srv.name.clear();
                            self.ok(format!("Server \"{name}\" toegevoegd."));
                        }
                        Err(e) => self.err(e),
                    }
                }
                Err(e) => self.err(format!("Server niet toegevoegd: {e}")),
            }
        }
        page_header(ui, "server", "Serverbeheer", "Voorgedefinieerde servers (Nederland, Vlaanderen, wereld) en uw eigen WMS-, WFS- en tegelservers.");
        group(ui, "Server toevoegen", |ui| {
            ui.horizontal(|ui| {
                let kind = self.srv.kind.get_or_insert(ServerKind::Wms);
                for k in [ServerKind::Wms, ServerKind::Wfs, ServerKind::Xyz] {
                    ui.radio_value(kind, k, k.label());
                }
            });
            let xyz = self.srv.kind == Some(ServerKind::Xyz);
            ui.add(egui::TextEdit::singleline(&mut self.srv.url).hint_text(if xyz { "https://…/{z}/{x}/{y}.png" } else { "https://…/wms of …/wfs (GetCapabilities wordt getest)" }).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.srv.name).hint_text("naam (leeg = titel van de server)"));
                ui.add(egui::TextEdit::singleline(&mut self.srv.region).hint_text("groep"));
                if ui.add_enabled(self.srv.test.is_none() && !self.srv.url.trim().is_empty(), primary_button("Testen en toevoegen")).clicked() {
                    let (kind, url) = (self.srv.kind.unwrap_or(ServerKind::Wms), self.srv.url.trim().to_string());
                    self.srv.test = Some(Job::spawn(&ctx, "Server testen…", move |_| match kind {
                        ServerKind::Wms => caps::fetch_wms(&crate::gis::http::clean_service_url(&url)).map(|c| c.title),
                        ServerKind::Wfs => caps::fetch_wfs(&crate::gis::http::clean_service_url(&url)).map(|c| c.title),
                        ServerKind::Xyz => {
                            if !(url.contains("{z}") && url.contains("{x}") && url.contains("{y}")) {
                                return Err("tegel-URL moet {z}, {x} en {y} bevatten".into());
                            }
                            crate::gis::http::get_image(&url.replace("{z}", "0").replace("{x}", "0").replace("{y}", "0")).map(|_| "Tegelserver".to_string())
                        }
                    }));
                }
                if let Some(j) = &self.srv.test {
                    ui.spinner();
                    ui.label(j.status());
                }
            });
        });
        let mut remove = None;
        group(ui, "Servers", |ui| {
            let mut regions: Vec<String> = Vec::new();
            for s in &self.servers {
                if !regions.contains(&s.region) {
                    regions.push(s.region.clone());
                }
            }
            egui::Grid::new("srv_list").num_columns(4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for r in regions {
                    ui.label(RichText::new(r.to_uppercase()).small().strong().color(ACCENT));
                    ui.end_row();
                    for s in self.servers.iter().filter(|s| s.region == r) {
                        ui.label(RichText::new(s.kind.label()).monospace().small());
                        ui.label(&s.name);
                        ui.label(RichText::new(&s.url).small().weak());
                        if s.builtin {
                            ui.label(RichText::new("standaard").small().weak());
                        } else if ui.small_button("Verwijderen").clicked() {
                            remove = Some(s.id.clone());
                        }
                        ui.end_row();
                    }
                }
            });
        });
        if let Some(id) = remove {
            let user: Vec<MapServer> = servers::load_user().into_iter().filter(|s| s.id != id).collect();
            match servers::save_user(&user) {
                Ok(()) => {
                    self.servers = servers::all();
                    self.svc.refresh();
                }
                Err(e) => self.err(e),
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

/// One search hit: a coloured kind badge, the name and below it the place,
/// the source on the right. Long texts end in "…".
fn hit_row(ui: &mut egui::Ui, h: &Hit, highlighted: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), egui::Sense::click());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    if highlighted || resp.hovered() {
        painter.rect_filled(rect, 4.0, if highlighted { ACCENT.gamma_multiply(0.35) } else { visuals.widgets.hovered.weak_bg_fill });
    }
    let badge_font = egui::FontId::proportional(10.5);
    let badge = painter.layout_no_wrap(h.kind.clone(), badge_font, Color32::WHITE);
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
                let hay = format!("{} {} {} {}", s.name, s.region, s.url, s.id).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            };
            let mut region = "";
            for s in all.iter().filter(|s| kinds.contains(&s.kind) && hit(s)) {
                if s.region != region {
                    region = &s.region;
                    ui.label(RichText::new(region.to_uppercase()).small().color(ACCENT));
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
    let name = ACI_CHOICES.iter().find(|(a, _)| a == aci).map(|(_, n)| *n).unwrap_or("?");
    egui::ComboBox::from_id_salt(id).selected_text(RichText::new(name).color(aci_color(*aci))).show_ui(ui, |ui| {
        for (a, n) in ACI_CHOICES {
            ui.selectable_value(aci, *a, RichText::new(*n).color(aci_color(*a)));
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
                ui.label(RichText::new(if civil { "Open Infra · Civiel" } else { "Open Infra · GIS" }).size(15.0).strong());
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
                ui.label(RichText::new("CIVIEL").small().weak());
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
                    ui.label(RichText::new(format!("{}\nX {:.1}\nY {:.1}\n{} × {} m · {}", s.crs, s.origin_e, s.origin_n, s.width, s.height, if s.absolute { "absoluut" } else { "relatief" })).small().monospace().weak());
                } else {
                    ui.label(RichText::new("Nog niet ingesteld").small().color(WARN));
                }
                if let Some(t) = bridge::target() {
                    let doc = t.doc_path.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "(niet opgeslagen tekening)".into());
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("Tekening: {doc}")).small().weak());
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                match &self.message {
                    Some((true, m)) => ui.label(RichText::new(m).color(OK_GREEN)),
                    Some((false, m)) => ui.label(RichText::new(m).color(ERR_RED)),
                    None => ui.label(RichText::new("Klaar.").weak()),
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
