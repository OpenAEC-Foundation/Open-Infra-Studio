//! Workbench page "Services wereldwijd": search every known WMS, WFS and tile
//! service in the world, see what covers the place on the map, inspect a
//! service's layers and coordinate systems, and load a layer into the drawing
//! through the same import code as the WMS and WFS pages.
//!
//! A child module of `app`, so it shares the window's map and helpers.

use eframe::egui::{self, Color32, RichText, Vec2};

use super::*;
use crate::gis::catalog::{self, CrsPlan, Query, Service, Source};
use crate::gis::crs::Support;
use crate::lang::tr;

/// What a service's GetCapabilities brought.
#[derive(Clone)]
pub(super) enum Caps {
    Wms(WmsCaps),
    Wfs(WfsCaps),
}

/// One layer or feature type, for the list.
struct LayerRow {
    name: String,
    title: String,
    crs: Vec<String>,
    bbox: Option<[f64; 4]>,
}

impl Caps {
    fn rows(&self) -> Vec<LayerRow> {
        match self {
            Caps::Wms(c) => c.layers.iter().map(|l| LayerRow { name: l.name.clone(), title: l.title.clone(), crs: l.crs.clone(), bbox: l.bbox }).collect(),
            Caps::Wfs(c) => c.types.iter().map(|t| LayerRow { name: t.name.clone(), title: t.title.clone(), crs: t.crs.clone(), bbox: t.bbox }).collect(),
        }
    }

    fn version(&self) -> String {
        match self {
            Caps::Wms(c) => format!("WMS {}", c.version),
            Caps::Wfs(c) => format!("WFS {}", c.version),
        }
    }
}

pub(super) struct ServicesPage {
    catalogue: Vec<Service>,
    query: String,
    region: Option<crate::localisation::Region>,
    categories: Vec<String>,
    kinds: Vec<ServerKind>,
    /// Only services whose coverage contains the location.
    here: bool,
    /// The location is the project site instead of the map centre.
    use_site: bool,
    cache: Option<(Query, Vec<usize>)>,
    selected: Option<Service>,
    caps: Option<Caps>,
    caps_job: Option<Job<Caps>>,
    layer: Option<String>,
    layer_filter: String,
    url: String,
    url_kind: ServerKind,
    resolution: f64,
    limit: usize,
    run: Option<Job<String>>,
}

impl Default for ServicesPage {
    fn default() -> Self {
        Self {
            catalogue: Vec::new(),
            query: String::new(),
            region: None,
            categories: Vec::new(),
            kinds: Vec::new(),
            here: true,
            use_site: false,
            cache: None,
            selected: None,
            caps: None,
            caps_job: None,
            layer: None,
            layer_filter: String::new(),
            url: String::new(),
            url_kind: ServerKind::Wms,
            resolution: 0.5,
            limit: 5000,
            run: None,
        }
    }
}

impl ServicesPage {
    pub(super) fn regional(&mut self,region:crate::localisation::Region) {
        self.region=Some(region);self.query.clear();self.categories.clear();self.kinds.clear();self.here=false;self.cache=None;
        self.selected=None;self.caps=None;self.caps_job=None;self.layer=None;self.layer_filter.clear();
    }
    /// Read the catalogue again (the user's servers may have changed).
    pub(super) fn refresh(&mut self) {
        self.catalogue.clear();
        self.cache = None;
    }
}

#[cfg(test)]
mod regional_tests {
    use super::*;
    #[test]
    fn entering_us_catalogue_cannot_import_previously_selected_dutch_service() {
        let mut page=ServicesPage::default();
        page.selected=catalog::all().into_iter().find(|s|s.country=="NL");
        assert!(page.selected.is_some());page.layer=Some("pand".into());
        page.regional(crate::localisation::Region::UnitedStates);
        assert!(page.selected.is_none());assert!(page.layer.is_none());
        assert!(!page.here,"a previous Dutch map centre must not hide the US catalogue");
    }
}

fn kind_label(k: ServerKind) -> &'static str {
    match k {
        ServerKind::Wms => "WMS",
        ServerKind::Wfs => "WFS",
        ServerKind::Xyz => "XYZ",
    }
}

/// A coverage box as a closed lon/lat ring for the map.
fn ring(b: [f64; 4]) -> Vec<LonLat> {
    let (s, n) = (b[1].max(-85.0), b[3].min(85.0));
    vec![[b[0], s], [b[2], s], [b[2], n], [b[0], n], [b[0], s]]
}

fn support_text(s: Support) -> (&'static str, Color32) {
    match s {
        Support::Builtin => (tr("ingebouwd", "built in"), OK_GREEN),
        Support::Online => (tr("via epsg.io", "via epsg.io"), WARN),
        Support::Unsupported => (tr("niet om te rekenen", "cannot transform"), ERR_RED),
    }
}

/// One line on how the service's data reaches the project CRS.
fn plan_text(plan: &CrsPlan, site_crs: &str) -> (String, Color32) {
    if plan.native {
        return (format!("{} {site_crs}: {}", tr("Levert het projectstelsel", "Serves the project CRS"), tr("geen herprojectie nodig.", "no reprojection needed.")), OK_GREEN);
    }
    match plan.support {
        Support::Builtin => (format!("{} {} {} {site_crs} ({}).", tr("Opgevraagd in", "Requested in"), plan.request, tr("en herprojecteerd naar", "and reprojected to"), tr("ingebouwd", "built in")), OK_GREEN),
        Support::Online => (format!("{} {} {} {site_crs}; {}", tr("Opgevraagd in", "Requested in"), plan.request, tr("en herprojecteerd naar", "and reprojected to"), tr("de definitie komt eenmalig van epsg.io.", "its definition comes once from epsg.io.")), WARN),
        Support::Unsupported => (format!("⚠ {} ({}).", tr("Geen van de coördinatenstelsels van deze dienst kan worden omgerekend", "None of this service's coordinate systems can be transformed"), plan.request), ERR_RED),
    }
}

impl App {
    /// The point "at this location" means: the map centre, or the site.
    fn services_point(&self) -> LonLat {
        match (self.svc.use_site, bridge::site()) {
            (true, Some(s)) => [s.lon, s.lat],
            _ => self.map.center,
        }
    }

    fn select_service(&mut self, ctx: &egui::Context, s: Service) {
        self.svc.caps = None;
        self.svc.layer = (!s.default_layer.is_empty()).then(|| s.default_layer.clone());
        self.svc.layer_filter.clear();
        self.svc.resolution = if s.kind == ServerKind::Xyz { 0.5 } else { 0.25 };
        let (kind, url) = (s.kind, s.url.clone());
        self.svc.caps_job = match kind {
            ServerKind::Wms => Some(Job::spawn(ctx, tr("Capabilities ophalen…", "Fetching capabilities…"), move |_| caps::fetch_wms(&url).map(Caps::Wms))),
            ServerKind::Wfs => Some(Job::spawn(ctx, tr("Capabilities ophalen…", "Fetching capabilities…"), move |_| caps::fetch_wfs(&url).map(Caps::Wfs))),
            ServerKind::Xyz => None,
        };
        self.svc.selected = Some(s);
    }

    /// Keep a copy among the user's own servers (with the chosen layer).
    fn add_service_to_mine(&mut self) {
        let Some(s) = self.svc.selected.clone() else { return };
        let mut user = servers::load_user();
        let layer = self.svc.layer.clone().unwrap_or_else(|| s.default_layer.clone());
        if user.iter().any(|u| u.url == s.url && u.kind == s.kind && u.default_layer == layer) {
            self.message = Some((true, tr("Deze dienst staat al bij uw servers.", "This service is already among your servers.").into()));
            return;
        }
        let id = if s.source == Source::User && !s.id.starts_with("url-") { s.id.clone() } else { format!("user-{}", s.id.trim_start_matches("url-")) };
        user.push(MapServer {
            id,
            kind: s.kind,
            name: s.name.clone(),
            region: tr("Eigen servers", "My servers").into(),
            url: s.url.clone(),
            attribution: if s.attribution.is_empty() { s.provider.clone() } else { s.attribution.clone() },
            default_layer: layer,
            max_zoom: (s.kind == ServerKind::Xyz).then_some(19),
            builtin: false,
        });
        match servers::save_user(&user) {
            Ok(()) => {
                self.servers = servers::all();
                self.svc.catalogue = catalog::all();
                self.svc.cache = None;
                self.ok(format!("{} \"{}\" {}", tr("Dienst", "Service"), s.name, tr("toegevoegd aan uw servers.", "added to your servers.")));
            }
            Err(e) => self.err(e),
        }
    }

    /// Hand the service and layer to the WMS or WFS page for all its options.
    fn open_on_import_page(&mut self, ctx: &egui::Context) {
        let Some(s) = self.svc.selected.clone() else { return };
        let ms = s.to_map_server();
        match (s.kind, self.svc.caps.clone()) {
            (ServerKind::Wfs, fetched) => {
                self.wfs.result = None;
                self.wfs.preview.clear();
                self.wfs.filter.clear();
                match fetched {
                    Some(Caps::Wfs(c)) => {
                        self.wfs.ft = c.types.iter().find(|t| Some(&t.name) == self.svc.layer.as_ref()).or(c.types.first()).cloned();
                        self.wfs.caps = Some(c);
                    }
                    _ => {
                        let url = ms.url.clone();
                        self.wfs.caps = None;
                        self.wfs.ft = None;
                        self.wfs.caps_job = Some(Job::spawn(ctx, "Capabilities ophalen…", move |_| caps::fetch_wfs(&url)));
                    }
                }
                self.wfs.server = Some(ms);
                self.tab = Tab::Wfs;
            }
            (kind, fetched) => {
                self.raster.preview = None;
                self.raster.filter.clear();
                self.raster.resolution = if kind == ServerKind::Xyz { 0.5 } else { 0.25 };
                self.raster.caps = None;
                self.raster.layer = None;
                match fetched {
                    Some(Caps::Wms(c)) => {
                        self.raster.layer = c.layers.iter().find(|l| Some(&l.name) == self.svc.layer.as_ref()).or(c.layers.first()).cloned();
                        if let Some(l) = &self.raster.layer {
                            self.raster.format = raster::default_format(&c, &l.name);
                        }
                        self.raster.caps = Some(c);
                    }
                    _ if kind == ServerKind::Wms => {
                        let url = ms.url.clone();
                        self.raster.caps_job = Some(Job::spawn(ctx, "Capabilities ophalen…", move |_| caps::fetch_wms(&url)));
                    }
                    _ => {}
                }
                self.raster.server = Some(ms);
                self.tab = Tab::Raster;
            }
        }
    }

    pub(super) fn services_page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.svc.catalogue.is_empty() {
            self.svc.catalogue = catalog::all();
        }
        if let Some(r) = poll(&mut self.svc.caps_job) {
            match r {
                Ok(c) => {
                    let rows = c.rows();
                    if self.svc.layer.as_ref().is_none_or(|l| !rows.iter().any(|r| &r.name == l)) {
                        self.svc.layer = rows.first().map(|r| r.name.clone());
                    }
                    self.message = Some((true, format!("{}: {} {}", c.version(), rows.len(), tr("lagen", "layers"))));
                    self.svc.caps = Some(c);
                }
                Err(e) => self.err(format!("GetCapabilities: {e}")),
            }
        }
        if let Some(r) = poll(&mut self.svc.run) {
            match r {
                Ok(m) => self.ok(m),
                Err(e) => self.err(e),
            }
        }

        page_header(
            ui,
            "services",
            tr("Services wereldwijd", "Services worldwide"),
            tr(
                "Zoek WMS-, WFS- en tegeldiensten in de hele wereld. Zoom op de kaart in op een plek om te zien wat daar beschikbaar is.",
                "Search WMS, WFS and tile services worldwide. Zoom the map in on a place to see what is available there.",
            ),
        );
        let site = bridge::site();
        let site_crs = site.as_ref().map(|s| s.crs.clone()).unwrap_or_else(|| "EPSG:28992".into());

        // Map: the selected service's coverage (orange) and layer extent (green).
        let mut lines: Vec<(Vec<LonLat>, Color32)> = Vec::new();
        if let Some(b) = self.svc.selected.as_ref().and_then(|s| s.bbox) {
            lines.push((ring(b), Color32::from_rgb(240, 140, 30)));
        }
        if let (Some(c), Some(l)) = (&self.svc.caps, &self.svc.layer) {
            if let Some(b) = c.rows().into_iter().find(|r| &r.name == l).and_then(|r| r.bbox) {
                lines.push((ring(b), Color32::from_rgb(40, 170, 80)));
            }
        }
        let marker = self.svc.here.then(|| self.services_point());
        if let Some(MapEvent::Clicked(p)) = self.map_section(ui, site.as_ref(), marker, None, &lines) {
            // A click moves the search point there.
            self.map.center = p;
            self.svc.use_site = false;
        }

        group(ui, tr("Zoeken", "Search"), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.svc.query)
                        .hint_text(tr("bijv. kadaster, percelen Bayern, luchtfoto, hoogte, cadastre New South Wales…", "e.g. cadastral parcels, Flurstücke, aerial, elevation, zoning Toronto…"))
                        .desired_width(ui.available_width() - 120.0),
                );
                if ui.add_enabled(!self.svc.query.is_empty(), egui::Button::new(tr("Wissen", "Clear"))).clicked() {
                    self.svc.query.clear();
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tr("Categorie", "Category")).weak());
                for &(id, nl, en, _) in catalog::CATEGORIES {
                    let on = self.svc.categories.iter().any(|c| c == id);
                    if ui.selectable_label(on, tr(nl, en)).clicked() {
                        if on {
                            self.svc.categories.retain(|c| c != id);
                        } else {
                            self.svc.categories.push(id.to_string());
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Type", "Type")).weak());
                for k in [ServerKind::Wms, ServerKind::Wfs, ServerKind::Xyz] {
                    let on = self.svc.kinds.contains(&k);
                    if ui.selectable_label(on, if k == ServerKind::Xyz { tr("XYZ-tegels", "XYZ tiles") } else { kind_label(k) }).clicked() {
                        if on {
                            self.svc.kinds.retain(|x| *x != k);
                        } else {
                            self.svc.kinds.push(k);
                        }
                    }
                }
                ui.separator();
                ui.checkbox(&mut self.svc.here, tr("Op deze locatie", "At this location"));
                ui.add_enabled_ui(self.svc.here, |ui| {
                    ui.radio_value(&mut self.svc.use_site, false, tr("kaartmidden", "map centre"));
                    ui.add_enabled_ui(site.is_some(), |ui| ui.radio_value(&mut self.svc.use_site, true, tr("projectlocatie", "project location")));
                });
                if self.svc.here {
                    let p = self.services_point();
                    ui.label(RichText::new(format!("{:.4}° N  {:.4}° {}", p[1], p[0], tr("O", "E"))).monospace().small().weak());
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(tr("Regio", "Region"));
            egui::ComboBox::from_id_salt("service-region").selected_text(self.svc.region.map(|r|r.label()).unwrap_or(tr("Alle regio’s", "All regions"))).show_ui(ui,|ui| {
                ui.selectable_value(&mut self.svc.region,None,tr("Alle regio’s","All regions"));
                for region in crate::localisation::REGIONS {ui.selectable_value(&mut self.svc.region,Some(region),region.label());}
            });
        });

        // The search runs again only when something changed.
        let q = Query { countries: self.svc.region.map(|r|r.countries()).unwrap_or_default(), text: self.svc.query.clone(), categories: self.svc.categories.clone(), kinds: self.svc.kinds.clone(), at: self.svc.here.then(|| self.services_point()) };
        let hits = match &self.svc.cache {
            Some((cq, h)) if *cq == q => h.clone(),
            _ => {
                let h = catalog::search(&self.svc.catalogue, &q);
                self.svc.cache = Some((q.clone(), h.clone()));
                h
            }
        };
        let mut chosen = None;
        group(ui, &format!("{} ({})", tr("Resultaten", "Results"), hits.len()), |ui| {
            if hits.is_empty() {
                ui.label(RichText::new(tr("Niets gevonden. Probeer andere woorden, minder filters of zet 'Op deze locatie' uit.", "Nothing found. Try other words, fewer filters, or turn off 'At this location'.")).color(WARN));
            }
            const MAX_ROWS: usize = 300;
            let shown = &hits[..hits.len().min(MAX_ROWS)];
            list_frame(ui, 300.0, "svc_results", |ui| {
                for (region, idx) in catalog::group(&self.svc.catalogue, shown) {
                    ui.label(RichText::new(format!("{}  ·  {}", region.to_uppercase(), idx.len())).small().strong().color(ACCENT));
                    for i in idx {
                        let s = &self.svc.catalogue[i];
                        let sel = self.svc.selected.as_ref().is_some_and(|x| x.id == s.id && x.url == s.url);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(kind_label(s.kind)).monospace().small().color(ACCENT));
                            if ui.selectable_label(sel, RichText::new(&s.name).strong()).clicked() {
                                chosen = Some(s.clone());
                            }
                        });
                        let crs = if s.crs.len() > 3 { format!("{} …(+{})", s.crs[..3].join(", "), s.crs.len() - 3) } else { s.crs.join(", ") };
                        let mut parts = vec![s.provider.clone(), catalog::category_label(&s.category), crs];
                        if !s.license.is_empty() {
                            parts.push(s.license.clone());
                        }
                        if !s.coverage.is_empty() && s.coverage != s.region_en {
                            parts.push(s.coverage.clone());
                        }
                        parts.retain(|p| !p.is_empty() && p != "—");
                        ui.add(egui::Label::new(RichText::new(parts.join("  ·  ")).small().weak()).truncate());
                        ui.add_space(2.0);
                    }
                }
                if hits.len() > MAX_ROWS {
                    ui.label(RichText::new(format!("… {} {}", hits.len() - MAX_ROWS, tr("meer: verfijn de zoekopdracht", "more: refine the search"))).weak());
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr("Andere dienst (URL)", "Other service (URL)")).weak());
                for k in [ServerKind::Wms, ServerKind::Wfs, ServerKind::Xyz] {
                    // Not just "WMS": the type filter above has those names.
                    ui.radio_value(&mut self.svc.url_kind, k, format!("{} {}", tr("als", "as"), kind_label(k)));
                }
                ui.add(egui::TextEdit::singleline(&mut self.svc.url).hint_text("https://…/wms  ·  https://…/wfs  ·  https://…/{z}/{x}/{y}.png").desired_width(ui.available_width() - 90.0));
                if ui.add_enabled(!self.svc.url.trim().is_empty(), egui::Button::new(tr("Openen", "Open"))).clicked() {
                    let url = if self.svc.url_kind == ServerKind::Xyz { self.svc.url.trim().to_string() } else { crate::gis::http::clean_service_url(&self.svc.url) };
                    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    let host = url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("").to_string();
                    let ms = MapServer { id: format!("url-{stamp}"), kind: self.svc.url_kind, name: host, region: String::new(), url, attribution: String::new(), default_layer: String::new(), max_zoom: None, builtin: false };
                    chosen = Some(catalog::user_service(&ms));
                }
            });
        });
        if let Some(s) = chosen {
            self.select_service(&ctx, s);
        }

        let Some(s) = self.svc.selected.clone() else { return };
        let point = self.services_point();
        let mut add = false;
        let mut open_page = false;
        let mut refetch = false;
        group(ui, tr("Dienst", "Service"), |ui| {
            egui::Grid::new("svc_detail").num_columns(2).spacing([14.0, 5.0]).min_col_width(150.0).show(ui, |ui| {
                ui.label(RichText::new(tr("Naam", "Name")).weak());
                ui.label(RichText::new(&s.name).strong());
                ui.end_row();
                if !s.provider.is_empty() {
                    ui.label(RichText::new(tr("Aanbieder", "Provider")).weak());
                    ui.label(&s.provider);
                    ui.end_row();
                }
                ui.label(RichText::new(tr("Type · categorie", "Type · category")).weak());
                ui.label(format!("{} · {}", s.kind.label(), catalog::category_label(&s.category)));
                ui.end_row();
                ui.label(RichText::new(tr("Dekking", "Coverage")).weak());
                ui.horizontal_wrapped(|ui| {
                    ui.label(if s.coverage.is_empty() { s.region().to_string() } else { s.coverage.clone() });
                    match s.covers(point) {
                        Some(true) => ui.colored_label(OK_GREEN, tr("· deze locatie valt erbinnen", "· this location is inside")),
                        Some(false) => ui.colored_label(WARN, tr("· deze locatie valt erbuiten", "· this location is outside")),
                        None => ui.label(RichText::new(tr("· dekking onbekend", "· coverage unknown")).weak()),
                    };
                    if let Some(b) = s.bbox {
                        ui.label(RichText::new(format!("[{:.2}, {:.2}, {:.2}, {:.2}]", b[0], b[1], b[2], b[3])).monospace().small().weak());
                    }
                });
                ui.end_row();
                if !s.license.is_empty() || !s.terms.is_empty() {
                    ui.label(RichText::new(tr("Licentie", "Licence")).weak());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&s.license);
                        if s.terms.starts_with("http") {
                            ui.hyperlink_to(tr("voorwaarden", "terms"), &s.terms);
                        }
                    });
                    ui.end_row();
                }
                ui.label(RichText::new("URL").weak());
                ui.add(egui::Label::new(RichText::new(&s.url).monospace().small().weak()).truncate());
                ui.end_row();
            });

            // Project CRS and the service's CRS side by side.
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                cols[0].label(RichText::new(tr("PROJECTSTELSEL", "PROJECT CRS")).small().strong().color(ACCENT));
                match &site {
                    Some(st) => {
                        let name = crs::known(&st.crs).map(|i| i.name).unwrap_or_default();
                        cols[0].label(RichText::new(format!("{}  {}", st.crs, name)).monospace());
                    }
                    None => {
                        cols[0].colored_label(WARN, tr("Nog geen projectlocatie: stel die eerst in.", "No project location yet: set it first."));
                    }
                }
                cols[1].label(RichText::new(tr("STELSELS VAN DE DIENST", "SERVICE CRS")).small().strong().color(ACCENT));
                let layer_crs = match (&self.svc.caps, &self.svc.layer) {
                    (Some(c), Some(l)) => c.rows().into_iter().find(|r| &r.name == l).map(|r| r.crs).filter(|v| !v.is_empty()),
                    _ => None,
                };
                let list = layer_crs.clone().unwrap_or_else(|| s.crs.clone());
                if list.is_empty() {
                    cols[1].label(RichText::new(tr("nog onbekend (wacht op GetCapabilities)", "not known yet (waiting for GetCapabilities)")).weak());
                }
                egui::ScrollArea::vertical().id_salt("svc_crs").max_height(110.0).show(&mut cols[1], |ui| {
                    for c in list.iter().take(40) {
                        let code = crs::normalize(c);
                        let (txt, col) = support_text(crs::support(&code));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&code).monospace().color(if code == site_crs { OK_GREEN } else { ui.visuals().text_color() }));
                            ui.label(RichText::new(txt).small().color(col));
                        });
                    }
                    if list.len() > 40 {
                        ui.label(RichText::new(format!("… +{}", list.len() - 40)).weak());
                    }
                });
            });
            if site.is_some() && (!s.crs.is_empty() || s.kind == ServerKind::Xyz || self.svc.caps.is_some()) {
                let offered = match (&self.svc.caps, &self.svc.layer) {
                    (Some(c), Some(l)) => c.rows().into_iter().find(|r| &r.name == l).map(|r| r.crs).unwrap_or_default(),
                    _ => s.crs.clone(),
                };
                let (txt, col) = plan_text(&catalog::crs_plan(s.kind, &offered, &site_crs), &site_crs);
                ui.label(RichText::new(txt).color(col));
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if s.kind != ServerKind::Xyz && ui.add_enabled(self.svc.caps_job.is_none(), egui::Button::new(tr("Capabilities opnieuw", "Fetch capabilities again"))).clicked() {
                    refetch = true;
                }
                if ui.button(tr("Toevoegen aan mijn servers", "Add to my servers")).clicked() {
                    add = true;
                }
                if ui.button(if s.kind == ServerKind::Wfs { tr("Openen op de WFS-pagina", "Open on the WFS page") } else { tr("Openen op de WMS-pagina", "Open on the WMS page") }).clicked() {
                    open_page = true;
                }
                if let Some(b) = s.bbox {
                    if ui.button(tr("Zoom naar dekking", "Zoom to coverage")).clicked() {
                        self.map.fit(&ring(b), Vec2::new(ui.available_width().max(600.0), MAP_H));
                    }
                }
                job_status(ui, &self.svc.caps_job);
            });
        });

        // Layers / feature types with their extent and CRS.
        if let Some(c) = &self.svc.caps {
            let rows = c.rows();
            let mut pick = None;
            let title = match c {
                Caps::Wms(_) => tr("Lagen", "Layers"),
                Caps::Wfs(_) => tr("Objecttypen", "Feature types"),
            };
            group(ui, &format!("{title} ({})", rows.len()), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.svc.layer_filter).hint_text(tr("Filter op naam of titel…", "Filter by name or title…")).desired_width(f32::INFINITY));
                let f = catalog::fold(&self.svc.layer_filter);
                list_frame(ui, 220.0, "svc_layers", |ui| {
                    for r in rows.iter().filter(|r| f.is_empty() || catalog::fold(&r.name).contains(&f) || catalog::fold(&r.title).contains(&f)) {
                        let sel = self.svc.layer.as_ref() == Some(&r.name);
                        ui.horizontal(|ui| {
                            let txt = if r.title != r.name { format!("{}   ({})", r.title, r.name) } else { r.name.clone() };
                            if ui.selectable_label(sel, txt).clicked() {
                                pick = Some(r.name.clone());
                            }
                            match r.bbox.map(|b| catalog::covers(b, point)) {
                                Some(true) => ui.colored_label(OK_GREEN, RichText::new(tr("locatie binnen bereik", "location inside")).small()),
                                Some(false) => ui.colored_label(WARN, RichText::new(tr("locatie buiten bereik", "location outside")).small()),
                                None => ui.label(RichText::new(tr("bereik onbekend", "extent unknown")).small().weak()),
                            };
                            ui.label(RichText::new(format!("{} CRS", r.crs.len())).small().weak());
                        });
                        if sel {
                            if let Some(b) = r.bbox {
                                ui.label(RichText::new(format!("     WGS 84 [{:.3}, {:.3}, {:.3}, {:.3}]", b[0], b[1], b[2], b[3])).monospace().small().weak());
                            }
                        }
                    }
                });
            });
            if let Some(l) = pick {
                self.svc.layer = Some(l);
            }
        }

        if refetch {
            self.select_service(&ctx, s.clone());
        }
        if add {
            self.add_service_to_mine();
        }
        if open_page {
            self.open_on_import_page(&ctx);
        }
    }

    pub(super) fn services_actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(s) = self.svc.selected.clone() else {
            ui.label(RichText::new(tr("Kies een dienst in de resultaten.", "Choose a service in the results.")).weak());
            return;
        };
        let site = bridge::site();
        let ready = site.is_some()
            && self.svc.run.is_none()
            && match s.kind {
                ServerKind::Xyz => true,
                _ => self.svc.caps.is_some() && self.svc.layer.is_some(),
            };
        button_bar(ui, |ui| {
            if ui.add_enabled(ready, primary_button(tr("Laden in tekening", "Load into the drawing"))).clicked() {
                let (site, caps, layer, res, limit) = (site.clone().unwrap(), self.svc.caps.clone(), self.svc.layer.clone(), self.svc.resolution, self.svc.limit);
                let server = s.to_map_server();
                self.svc.run = Some(Job::spawn(&ctx, tr("Importeren…", "Importing…"), move |p| load(&site, &server, caps.as_ref(), layer.as_deref(), res, limit, p)));
            }
            match s.kind {
                ServerKind::Wfs => {
                    ui.label(tr("Max. objecten", "Max. features"));
                    ui.add(egui::DragValue::new(&mut self.svc.limit).speed(100).range(1..=200_000));
                }
                _ => {
                    ui.label(tr("Resolutie", "Resolution"));
                    ui.add(egui::DragValue::new(&mut self.svc.resolution).speed(0.01).range(0.02..=100.0).suffix(" m/px"));
                }
            }
            if site.is_none() {
                ui.colored_label(WARN, tr("Stel eerst de projectlocatie in.", "Set the project location first."));
            }
            job_status(ui, &self.svc.run);
        });
    }
}

/// Load the chosen layer with the capabilities already fetched.
fn load(site: &Site, server: &MapServer, caps: Option<&Caps>, layer: Option<&str>, resolution: f64, limit: usize, progress: &dyn Fn(String)) -> Result<String, String> {
    let (wms, wfs) = match caps {
        Some(Caps::Wms(c)) => (Some(c), None),
        Some(Caps::Wfs(c)) => (None, Some(c)),
        None => (None, None),
    };
    import::service_layer(site, server, wms, wfs, layer, resolution, limit, progress)
}
