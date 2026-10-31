//! Workbench page: the vehicle library. Search and filter on the left, a
//! drawing of the chosen vehicle on the right, and buttons that insert it
//! into the drawing.

use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2};

use crate::bridge;
use crate::lang::{self, tr, Lang};
use crate::library::draw::{self, View};
use crate::library::{self, Vehicle};

const ACCENT: Color32 = Color32::from_rgb(33, 150, 220);

pub struct LibraryPage {
    pub show_infra: bool,
    infra: super::infra::InfraPage,
    search: String,
    category: Option<&'static str>,
    region: Option<String>,
    selected: Option<String>,
    view: View,
    message: Option<(bool, String)>,
}

impl Default for LibraryPage {
    fn default() -> Self {
        Self { show_infra: false, infra: Default::default(), search: String::new(), category: None, region: None, selected: None, view: View::Top, message: None }
    }
}

fn matches(v: &Vehicle, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hay = format!("{} {} {} {} {}", v.id, v.name_en, v.name_nl, v.standard, v.region).to_lowercase();
    q.split_whitespace().all(|w| hay.contains(w))
}

/// Run a command line in Open CAD Studio from a worker (it may wait on the host).
fn run(cmd: String) {
    std::thread::spawn(move || {
        if let Err(e) = bridge::execute(&cmd) {
            bridge::error(&e);
        }
    });
}

fn draw_vehicle(ui: &mut egui::Ui, v: &Vehicle, view: View, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let shapes = draw::shapes(v, view);
    let b = draw::bounds(&shapes);
    let (w, h) = ((b[2] - b[0]).max(0.1), (b[3] - b[1]).max(0.1));
    let margin = 28.0;
    let scale = ((rect.width() - 2.0 * margin) as f64 / w).min((rect.height() - 2.0 * margin) as f64 / h);
    let cx = rect.center().x as f64 - (b[0] + b[2]) / 2.0 * scale;
    let cy = rect.center().y as f64 + (b[1] + b[3]) / 2.0 * scale;
    let to = |p: [f64; 2]| egui::pos2((cx + p[0] * scale) as f32, (cy - p[1] * scale) as f32);
    let stroke = Stroke::new(1.3, Color32::from_rgb(220, 228, 235));
    for s in &shapes {
        let pts: Vec<egui::Pos2> = draw::flatten(s).into_iter().map(to).collect();
        painter.add(egui::Shape::line(pts, stroke));
    }
    // Insertion point (the front) and the driving direction.
    let o = to([0.0, 0.0]);
    painter.circle_stroke(o, 4.0, Stroke::new(1.5, ACCENT));
    painter.arrow(o, Vec2::new(22.0, 0.0), Stroke::new(1.5, ACCENT));
    // Overall length below.
    let y = rect.bottom() - 10.0;
    let (x0, x1) = (to([b[0], 0.0]).x, to([b[2], 0.0]).x);
    painter.line_segment([egui::pos2(x0, y), egui::pos2(x1, y)], Stroke::new(1.0, ACCENT));
    painter.text(egui::pos2((x0 + x1) / 2.0, y - 2.0), egui::Align2::CENTER_BOTTOM, format!("{:.2} m", v.length()), egui::FontId::proportional(12.0), ACCENT);
}

impl LibraryPage {
    pub fn page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.show_infra, false, tr("Voertuigen", "Vehicles"));
            ui.selectable_value(&mut self.show_infra, true, tr("Infra-objecten", "Infrastructure"));
        });
        if self.show_infra { self.infra.page(ui); return; }
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Voertuigbibliotheek", "Vehicle library")).size(18.0).strong());
        ui.label(
            RichText::new(tr(
                "Ontwerpvoertuigen uit AASHTO, FGSV, Austroads en de EU-maximummaten, plus gangbare voertuigen. Elk voertuig wordt een DWG-blok in boven- of zijaanzicht.",
                "Design vehicles from AASHTO, FGSV, Austroads and the EU maximum dimensions, plus common vehicles. Each vehicle becomes a DWG block in plan or side view.",
            ))
            .weak(),
        );
        ui.add_space(6.0);

        let all = library::vehicles();
        let mut regions: Vec<String> = all.iter().map(|v| v.region.clone()).filter(|r| !r.is_empty()).collect();
        regions.sort();
        regions.dedup();

        ui.horizontal(|ui| {
            ui.label(tr("Zoeken", "Search"));
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr("bijv. bus, WB-62, Sattelzug", "e.g. bus, WB-62, Sattelzug")).desired_width(220.0));
            egui::ComboBox::from_id_salt("lib-cat")
                .selected_text(self.category.map(library::category_name).unwrap_or_else(|| tr("Alle categorieën", "All categories").into()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, tr("Alle categorieën", "All categories"));
                    for (code, ..) in library::CATEGORIES {
                        if all.iter().any(|v| v.category == *code) {
                            ui.selectable_value(&mut self.category, Some(*code), library::category_name(code));
                        }
                    }
                });
            egui::ComboBox::from_id_salt("lib-region")
                .selected_text(self.region.clone().unwrap_or_else(|| tr("Alle regio's", "All regions").into()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.region, None, tr("Alle regio's", "All regions"));
                    for r in &regions {
                        ui.selectable_value(&mut self.region, Some(r.clone()), r);
                    }
                });
        });
        ui.add_space(4.0);

        let q = self.search.to_lowercase();
        let list: Vec<&'static Vehicle> = all
            .iter()
            .filter(|v| self.category.is_none_or(|c| v.category == c))
            .filter(|v| self.region.as_ref().is_none_or(|r| &v.region == r))
            .filter(|v| matches(v, &q))
            .collect();
        if self.selected.as_ref().is_none_or(|id| !list.iter().any(|v| &v.id == id)) {
            self.selected = list.first().map(|v| v.id.clone());
        }

        let avail = ui.available_width();
        ui.horizontal_top(|ui| {
            // The list.
            ui.vertical(|ui| {
                ui.set_width((avail * 0.38).max(280.0));
                ui.label(RichText::new(format!("{} {}", list.len(), tr("voertuigen", "vehicles"))).small().weak());
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(egui::Margin::same(4)).corner_radius(3.0).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("lib-list").max_height(560.0).auto_shrink([false, true]).show(ui, |ui| {
                        let mut cat = "";
                        for v in &list {
                            if v.category != cat {
                                cat = &v.category;
                                ui.add_space(4.0);
                                ui.label(RichText::new(library::category_name(cat).to_uppercase()).small().color(ACCENT));
                            }
                            let sel = self.selected.as_deref() == Some(v.id.as_str());
                            let text = RichText::new(format!("{}\n{}", v.name(), v.dimensions())).size(12.5);
                            if ui.selectable_label(sel, text).clicked() {
                                self.selected = Some(v.id.clone());
                            }
                        }
                    });
                });
            });
            ui.add_space(8.0);
            // The chosen vehicle.
            ui.vertical(|ui| {
                let Some(v) = self.selected.as_deref().and_then(library::find) else {
                    ui.label(tr("Geen voertuig gevonden.", "No vehicle found."));
                    return;
                };
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.view, View::Top, tr("Bovenaanzicht", "Plan view"));
                    ui.selectable_value(&mut self.view, View::Side, tr("Zijaanzicht", "Side view"));
                });
                let w = ui.available_width();
                draw_vehicle(ui, v, self.view, Vec2::new(w, if self.view == View::Top { 220.0 } else { 200.0 }));
                ui.add_space(6.0);
                ui.label(RichText::new(v.name()).size(15.0).strong());
                egui::Grid::new("lib-info").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, k: &str, val: String| {
                        ui.label(RichText::new(k).weak());
                        ui.label(val);
                        ui.end_row();
                    };
                    row(ui, tr("Afmetingen", "Dimensions"), v.dimensions());
                    row(ui, tr("Norm / bron", "Standard / source"), if v.standard.is_empty() { "—".into() } else { v.standard.clone() });
                    row(ui, tr("Regio", "Region"), if v.region.is_empty() { "—".into() } else { v.region.clone() });
                    let axles: usize = v.units.iter().map(|u| u.axles.len()).sum();
                    row(ui, tr("Eenheden · assen", "Units · axles"), format!("{} · {axles}", v.units.len()));
                    row(ui, tr("Blok", "Block"), v.block_name(self.view));
                    if !v.estimated.is_empty() {
                        row(ui, tr("Geschat", "Estimated"), v.estimated.join(", "));
                    }
                });
                if !v.source_url.is_empty() {
                    ui.hyperlink_to(tr("Bron openen", "Open source"), &v.source_url);
                }
                if !v.notes.is_empty() {
                    ui.label(RichText::new(&v.notes).small().weak());
                }
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    let primary = |t: &str| egui::Button::new(RichText::new(t.to_string()).strong().color(Color32::WHITE)).fill(ACCENT).min_size(Vec2::new(150.0, 28.0));
                    if ui.add(primary(tr("Invoegen in tekening", "Insert into drawing"))).on_hover_text(tr("Wijs in Open CAD Studio de voorkant en de rijrichting aan.", "Pick the front and the driving direction in Open CAD Studio.")).clicked() {
                        let side = if self.view == View::Side { " SIDE" } else { "" };
                        run(format!("GIS_VEHICLE {}{side}", v.id));
                        self.message = Some((true, tr("Wijs in de tekening de voorkant van het voertuig aan.", "Pick the front of the vehicle in the drawing.").into()));
                    }
                    let side = if self.view == View::Side { " SIDE" } else { "" };
                    if ui.button(tr("Overzicht van deze categorie", "Catalogue of this category")).clicked() {
                        run(format!("GIS_VEHICLE_CATALOG {}{side}", v.category));
                        self.message = Some((true, tr("Wijs in de tekening de linkeronderhoek van het overzicht aan.", "Pick the lower-left corner of the catalogue in the drawing.").into()));
                    }
                    if ui.button(tr("Overzicht van alle voertuigen", "Catalogue of all vehicles")).clicked() {
                        run(format!("GIS_VEHICLE_CATALOG{side}"));
                        self.message = Some((true, tr("Wijs in de tekening de linkeronderhoek van het overzicht aan.", "Pick the lower-left corner of the catalogue in the drawing.").into()));
                    }
                });
                if let Some((ok, m)) = &self.message {
                    ui.label(RichText::new(m).color(if *ok { Color32::from_rgb(152, 195, 121) } else { Color32::from_rgb(224, 108, 117) }));
                }
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{} GIS_VEHICLE {}{}", tr("Commando:", "Command:"), v.id, if self.view == View::Side { " SIDE" } else { "" })).small().monospace().weak());
            });
        });
    }
}

/// Language switch for the nav panel; takes effect at the next start.
pub fn language_switch(ui: &mut egui::Ui) {
    let current = lang::current();
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Taal", "Language")).small().weak());
        for (l, label) in [(Lang::Nl, "NL"), (Lang::En, "EN")] {
            if ui.selectable_label(current == l, RichText::new(label).small()).on_hover_text(tr("Geldt na een herstart van Open CAD Studio.", "Takes effect after restarting Open CAD Studio.")).clicked() && current != l {
                if let Err(e) = lang::save(l) {
                    bridge::error(&e);
                } else {
                    bridge::info(tr("Taal opgeslagen; geldt na een herstart van Open CAD Studio.", "Language saved; it takes effect after restarting Open CAD Studio."));
                }
            }
        }
    });
    let mut region = crate::localisation::configured();
    let before = region;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("Standaardregio", "Default region")).small().weak());
        egui::ComboBox::from_id_salt("default-region").selected_text(region.label()).show_ui(ui, |ui| {
            for choice in crate::localisation::REGIONS {ui.selectable_value(&mut region, choice, choice.label());}
        });
    });
    if before != region {
        match crate::localisation::save(region) {
            Err(e) => bridge::error(&e),
            Ok(()) => bridge::info(tr("Standaardregio opgeslagen; herstart voor het lint. De projectregio staat bij Localisatie.", "Default region saved; restart for the ribbon. Set the drawing's region under Localisation.")),
        }
    }
}
