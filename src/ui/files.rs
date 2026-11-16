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
                    self.message = Some((true, d.report(format!("{}: {} {}", d.format, d.features.len(), tr("objecten gelezen", "features read")))));
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

#[cfg(test)]
mod tests {
    use super::*;

    mod import_drawing {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/import_drawing.rs"));
    }

    fn load_page(text: &str, extension: &str) -> FilesPage {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("oi-file-message-{}-{serial}.{extension}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        load_page_path(path)
    }

    fn load_page_path(path: PathBuf) -> FilesPage {
        let ctx = egui::Context::default();
        let mut page = FilesPage::default();
        page.open(&ctx, path.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while page.read.is_some() && std::time::Instant::now() < deadline {
            let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| page.page(ui));
            });
            // This headless test has no renderer. Explicitly consume its
            // texture updates before dropping the completed frame.
            frame.textures_delta.clear();
            if page.read.is_some() { std::thread::sleep(std::time::Duration::from_millis(2)); }
        }
        std::fs::remove_file(path).unwrap();
        assert!(page.read.is_none(), "bestandslezing niet binnen vijf seconden afgerond");
        page
    }

    #[test]
    fn csv_file_page_reports_the_skipped_row_and_reason() {
        let page = load_page("x,y\n100,200\nbad,220\n", "csv");
        let data = page.data.as_ref().unwrap();
        assert_eq!((data.features.len(), data.skipped, data.warnings.len()), (1, 1, 1));
        assert_eq!(page.message, Some((true, format!("CSV: 1 {}\n{}", tr("objecten gelezen", "features read"), data.warnings.join("\n")))));
    }

    #[test]
    fn imro_file_page_reports_each_unreadable_object() {
        let text = include_str!("../../tests/fixtures/planning/synthetic-imro.gml")
            .replace("<imro:geometrie xlink:href=\"#polygon\"/>", "");
        let page = load_page(&text, "gml");
        let data = page.data.as_ref().unwrap();
        assert_eq!(data.features.len(), 2);
        assert!(data.warnings.iter().any(|w| w == &format!("zone: {}", tr("geen tekenbare geometrie", "no drawable geometry"))));
        assert!(data.warnings.iter().any(|w| w == &format!("maten: {}", tr("geen tekenbare geometrie", "no drawable geometry"))));
        assert_eq!(page.message, Some((true, format!("IMRO: 2 {}\n{}", tr("objecten gelezen", "features read"), data.warnings.join("\n")))));
    }

    #[test]
    fn clean_file_page_keeps_the_existing_message() {
        let page = load_page("x,y\n100,200\n", "csv");
        assert_eq!(page.message, Some((true, format!("CSV: 1 {}", tr("objecten gelezen", "features read")))));
    }

    #[test]
    fn gml_file_page_reports_the_broken_object_and_keeps_its_neighbour() {
        let page = load_page(include_str!("../../tests/fixtures/gml/mixed.gml"), "gml");
        let data = page.data.as_ref().unwrap();
        assert_eq!(data.features.len(), 1);
        assert_eq!(data.features[0].points, vec![[2600005.0, 1200005.0]]);
        assert_eq!((data.skipped, data.warnings.len()), (1, 1));
        assert!(data.warnings[0].contains("broken-line"));
        assert_eq!(page.message, Some((true, format!("GML: 1 {}\n{}", tr("objecten gelezen", "features read"), data.warnings[0]))));
    }

    #[test]
    fn gpkg_file_page_reports_the_broken_row_and_keeps_layer_choices() {
        let path = std::env::temp_dir().join(format!("oi-gpkg-message-{}.gpkg", std::process::id()));
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../../tests/fixtures/geopackage/partial.sql")).unwrap();
        drop(db);
        let page = load_page_path(path);
        let data = page.data.as_ref().unwrap();
        assert_eq!(data.features.len(), 2);
        assert_eq!(data.layers, vec!["a_mix", "b_good"]);
        assert_eq!((data.skipped, data.warnings.len()), (1, 1));
        assert!(data.warnings[0].contains("a_mix") && data.warnings[0].contains('2'));
        assert_eq!(page.message, Some((true, format!("GeoPackage: 2 {}\n{}", tr("objecten gelezen", "features read"), data.warnings[0]))));
    }

    // The language is cached once per app session. Each probe therefore runs
    // in its own process instead of changing the parallel test process.
    fn imro_language_probes(cases: &[&str]) {
        let mut failures = Vec::new();
        for case in cases {
            for language in ["nl", "en"] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "ui::files::tests::imro_message_probe_in_isolated_language", "--ignored", "--nocapture"])
                    .env("OPENAEC_TAAL", language)
                    .env("OIS_IMRO_MESSAGE_CASE", case)
                    .output().unwrap();
                if !output.status.success() {
                    failures.push(format!("{case}/{language}: {}\n{}\n{}", output.status, String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn sol_unusable_imro_page_and_command_keep_the_object_reason_in_both_languages() {
        imro_language_probes(&["page-unusable", "command-unusable"]);
    }

    #[test]
    fn imro_partial_import_uses_the_selected_language_and_one_line_per_warning() {
        imro_language_probes(&["page-partial", "command-partial"]);
    }

    #[test]
    #[ignore = "alleen via de procesproeven met een vaste taal"]
    fn imro_message_probe_in_isolated_language() {
        let language = std::env::var("OPENAEC_TAAL").unwrap();
        let dutch = language == "nl";
        assert_eq!(crate::lang::current(), if dutch { crate::lang::Lang::Nl } else { crate::lang::Lang::En });
        let case = std::env::var("OIS_IMRO_MESSAGE_CASE").unwrap();
        let partial = case.ends_with("partial");
        let text = if partial {
            include_str!("../../tests/fixtures/planning/partial-imro.gml")
        } else {
            include_str!("../../tests/fixtures/planning/unusable-imro.gml")
        };
        let reason = if dutch { "geen tekenbare geometrie" } else { "no drawable geometry" };
        let warnings = if partial { format!("zone: {reason}\nmaten: {reason}") } else { format!("zone: {reason}") };
        let drawing = std::sync::Arc::new(import_drawing::Drawing::default());
        let target = bridge::Target { sender: drawing.clone(), tab_id: 74118, unit_factor: 1.0, doc_path: None };
        let site = crate::gis::site::Site::default();
        if case.starts_with("page-") {
            let page = bridge::with_command_context(target, Some(site), None, || load_page(text, "gml"));
            assert!(drawing.entities.lock().unwrap().is_empty());
            assert!(drawing.undo.lock().unwrap().is_empty());
            assert!(drawing.unexpected.lock().unwrap().is_empty());
            if partial {
                let data = page.data.as_ref().unwrap();
                assert_eq!(data.features.len(), 1);
                assert_eq!(data.warnings.len(), 2);
                let read = if dutch { "objecten gelezen" } else { "features read" };
                assert_eq!(page.message, Some((true, format!("IMRO: 1 {read}\n{warnings}"))));
            } else {
                assert!(page.data.is_none());
                assert!(page.opt.is_none());
                assert_eq!(page.message, Some((false, format!("Geen IMRO-planobjecten met geometrie gevonden\n{warnings}"))));
            }
        } else {
            assert!(case.starts_with("command-"));
            let path = std::env::temp_dir().join(format!("oi-imro-message-command-{}.gml", std::process::id()));
            std::fs::write(&path, text).unwrap();
            let result = bridge::with_command_context(target, Some(site.clone()), None, || {
                bestand::import_path(&site, &path, None, &|_| {})
            });
            std::fs::remove_file(path).unwrap();
            assert!(drawing.unexpected.lock().unwrap().is_empty());
            if partial {
                let n = drawing.entities.lock().unwrap().len();
                assert!(n > 0);
                assert_eq!(drawing.undo.lock().unwrap().len(), 1);
                let layers = drawing.layers.lock().unwrap().len();
                let summary = if dutch {
                    format!("IMRO: 1 objecten, {n} elementen op {layers} lagen. Kaartgegevens; controleer actuele regels en rechtsgeldigheid.")
                } else {
                    format!("IMRO: 1 features, {n} entities on {layers} layers. Map data; check current rules and legal validity.")
                };
                let message = result.unwrap();
                assert_eq!(message, format!("{summary}\n{warnings}"));
                assert_eq!(message.matches("zone:").count(), 1);
                assert_eq!(message.matches("maten:").count(), 1);
            } else {
                assert!(drawing.entities.lock().unwrap().is_empty());
                assert!(drawing.undo.lock().unwrap().is_empty());
                assert_eq!(result.unwrap_err(), format!("Geen IMRO-planobjecten met geometrie gevonden\n{warnings}"));
            }
        }
    }

}
