//! Workbench page: a local GIS file (GeoJSON, Shapefile, GeoPackage,
//! KML/KMZ, GPX, CSV) into the drawing.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText, Vec2};

use super::jobs::{poll, Job};
use crate::bestand::{self, FileOptions};
use crate::bridge;
use crate::gis::files::{self, FileData};
use crate::lang::tr;

const ACCENT: Color32 = Color32::from_rgb(33, 150, 220);

#[derive(Default)]
pub struct FilesPage {
    pub path: String,
    gpkg_layer: Option<String>,
    data: Option<FileData>,
    opt: Option<FileOptions>,
    read: Option<Job<FileData>>,
    run: Option<Job<String>>,
    message: Option<(bool, String)>,
}

impl FilesPage {
    /// Load `path` (from the ribbon's file dialog).
    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.path = path.to_string_lossy().into_owned();
        self.gpkg_layer = None;
        self.load(ctx);
    }

    fn load(&mut self, ctx: &egui::Context) {
        let (p, layer) = (PathBuf::from(self.path.trim().trim_matches('"')), self.gpkg_layer.clone());
        self.data = None;
        self.opt = None;
        self.message = None;
        self.read = Some(Job::spawn(ctx, tr("Bestand lezen…", "Reading the file…"), move |_| files::read(&p, layer.as_deref())));
    }

    pub fn page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.read) {
            match r {
                Ok(d) => {
                    let site = bridge::site().unwrap_or_default();
                    self.opt = Some(FileOptions::for_file(&PathBuf::from(self.path.trim()), &d, &site));
                    self.message = Some((true, format!("{}: {} {}", d.format, d.features.len(), tr("objecten gelezen", "features read"))));
                    self.data = Some(d);
                }
                Err(e) => self.message = Some((false, e)),
            }
        }
        if let Some(r) = poll(&mut self.run) {
            self.message = Some(match r {
                Ok(m) => {
                    bridge::info(&m);
                    (true, m)
                }
                Err(e) => (false, e),
            });
        }

        ui.add_space(4.0);
        ui.label(RichText::new(tr("GIS-bestand importeren", "Import a GIS file")).size(18.0).strong());
        ui.label(RichText::new(tr("GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX of CSV met coördinaatkolommen. Attributen komen als objectgegevens (XDATA) op de elementen.", "GeoJSON, Shapefile, GeoPackage, KML/KMZ, GPX or CSV with coordinate columns. Attributes become object data (XDATA) on the entities.")).weak());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(tr("Bestand", "File"));
            let r = ui.add(egui::TextEdit::singleline(&mut self.path).hint_text(tr("pad naar het bestand, of gebruik de knop GIS-bestand in het lint", "path to the file, or use the GIS file button in the ribbon")).desired_width(ui.available_width() - 90.0));
            if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button(tr("Lezen", "Read")).clicked() {
                self.load(&ctx);
            }
        });
        if let Some(j) = &self.read {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(j.status());
            });
        }
        let Some(data) = self.data.clone() else {
            self.status(ui);
            return;
        };
        let Some(mut opt) = self.opt.take() else { return };
        let mut reload = None;
        ui.add_space(6.0);
        egui::Grid::new("files-opt").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label(RichText::new(tr("Formaat", "Format")).weak());
            ui.label(format!("{} · {} {}", data.format, data.features.len(), tr("objecten", "features")));
            ui.end_row();
            if let Some(b) = data.bounds() {
                ui.label(RichText::new(tr("Bereik", "Extent")).weak());
                ui.label(RichText::new(format!("{:.2}, {:.2} — {:.2}, {:.2}", b[0], b[1], b[2], b[3])).monospace());
                ui.end_row();
            }
            if data.layers.len() > 1 {
                ui.label(RichText::new(tr("GeoPackage-laag", "GeoPackage layer")).weak());
                let mut pick = None;
                egui::ComboBox::from_id_salt("gpkg-layer").selected_text(self.gpkg_layer.clone().unwrap_or_else(|| data.layers[0].clone())).show_ui(ui, |ui| {
                    for l in &data.layers {
                        if ui.selectable_label(self.gpkg_layer.as_deref() == Some(l.as_str()), l).clicked() {
                            pick = Some(l.clone());
                        }
                    }
                });
                if let Some(l) = pick {
                    reload = Some(l);
                }
                ui.end_row();
            }
            ui.label(RichText::new(tr("Coördinatenstelsel", "Coordinate system")).weak());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut opt.crs).desired_width(140.0));
                ui.label(RichText::new(if data.crs.is_some() { tr("uit het bestand", "from the file") } else { tr("niet in het bestand: controleer dit", "not in the file: check this") }).small().color(if data.crs.is_some() { Color32::GRAY } else { Color32::from_rgb(229, 192, 123) }));
            });
            ui.end_row();
            ui.label(RichText::new(tr("Laag", "Layer")).weak());
            ui.add(egui::TextEdit::singleline(&mut opt.layer).desired_width(260.0));
            ui.end_row();
            let fields = data.fields();
            let field_combo = |ui: &mut egui::Ui, id: &str, value: &mut Option<String>| {
                egui::ComboBox::from_id_salt(id).selected_text(value.clone().unwrap_or_else(|| "—".into())).show_ui(ui, |ui| {
                    ui.selectable_value(value, None, "—");
                    for f in &fields {
                        ui.selectable_value(value, Some(f.clone()), f);
                    }
                });
            };
            ui.label(RichText::new(tr("Laag per waarde van", "One layer per value of")).weak());
            field_combo(ui, "split-by", &mut opt.split_by);
            ui.end_row();
            ui.label(RichText::new(tr("Label", "Label")).weak());
            field_combo(ui, "label-by", &mut opt.label_field);
            ui.end_row();
            ui.label(RichText::new(tr("Teksthoogte (m)", "Text height (m)")).weak());
            ui.add(egui::DragValue::new(&mut opt.text_m).range(0.1..=50.0).speed(0.1));
            ui.end_row();
            ui.label(RichText::new(tr("Kleur (ACI)", "Colour (ACI)")).weak());
            ui.add(egui::DragValue::new(&mut opt.aci).range(1..=255));
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut opt.clip, tr("Bijsnijden tot het importgebied", "Clip to the import area"));
            ui.end_row();
        });
        if let Some(l) = reload {
            self.gpkg_layer = Some(l);
            self.load(&ctx);
            return;
        }
        ui.add_space(8.0);
        let can = self.run.is_none() && bridge::site().is_some();
        let b = egui::Button::new(RichText::new(tr("OK en importeren", "OK and import")).strong().color(Color32::WHITE)).fill(ACCENT).min_size(Vec2::new(150.0, 28.0));
        if ui.add_enabled(can, b).clicked() {
            let (site, d, o, src) = (bridge::site().unwrap_or_default(), data.clone(), opt.clone(), self.path.trim().to_string());
            let name = PathBuf::from(&src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(src);
            self.run = Some(Job::spawn(&ctx, tr("Importeren…", "Importing…"), move |p| bestand::place(&site, &d, &o, &name, p)));
        }
        self.opt = Some(opt);
        if bridge::site().is_none() {
            ui.label(RichText::new(tr("Stel eerst de projectlocatie in.", "Set the project location first.")).color(Color32::from_rgb(229, 192, 123)));
        }
        if let Some(j) = &self.run {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(j.status());
            });
        }
        self.status(ui);
    }

    fn status(&self, ui: &mut egui::Ui) {
        if let Some((ok, m)) = &self.message {
            ui.label(RichText::new(m).color(if *ok { Color32::from_rgb(152, 195, 121) } else { Color32::from_rgb(224, 108, 117) }));
        }
        ui.add_space(4.0);
        ui.label(RichText::new(format!("{} GIS_FILE_IMPORT <path> [EPSG:code]", tr("Commando:", "Command:"))).small().monospace().weak());
    }
}
