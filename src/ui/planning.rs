//! Spatial planning page: explicit plan choice, followed by download and placement.
//! The API key is held only in this page's memory and never written to settings.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};

use super::jobs::{poll, Job};
use crate::bridge;
use crate::gis::{
    crs,
    planning::{self, Client, Data, Plan},
};
use crate::lang::tr;

pub struct PlanningPage {
    key: String,
    path: String,
    plans: Vec<Plan>,
    selected: Option<String>,
    data: Option<Data>,
    source: String,
    search: Option<Job<Vec<Plan>>>,
    download: Option<Job<Data>>,
    run: Option<Job<String>>,
    message: Option<(bool, String)>,
}

impl Default for PlanningPage {
    fn default() -> Self {
        Self {
            key: std::env::var("OPENINFRA_RP_API_KEY").unwrap_or_default(),
            path: String::new(),
            plans: Vec::new(),
            selected: None,
            data: None,
            source: String::new(),
            search: None,
            download: None,
            run: None,
            message: None,
        }
    }
}

impl PlanningPage {
    fn busy(&self) -> bool {
        self.search.is_some() || self.download.is_some() || self.run.is_some()
    }

    fn finish(&mut self) {
        if let Some(result) = poll(&mut self.search) {
            match result {
                Ok(plans) => {
                    self.message = Some((
                        true,
                        format!(
                            "{} {}",
                            plans.len(),
                            tr(
                                "plannen op de projectlocatie gevonden. Kies een plan.",
                                "plans found at the project location. Choose a plan."
                            )
                        ),
                    ));
                    self.plans = plans;
                }
                Err(error) => self.message = Some((false, error)),
            }
        }
        if let Some(result) = poll(&mut self.download) {
            match result {
                Ok(data) => {
                    self.message = Some((
                        true,
                        format!(
                            "{} {}",
                            data.objects.len(),
                            tr(
                                "planobjecten opgehaald; klaar om te plaatsen.",
                                "plan features downloaded; ready to place."
                            )
                        ),
                    ));
                    self.data = Some(data);
                }
                Err(error) => self.message = Some((false, error)),
            }
        }
        if let Some(result) = poll(&mut self.run) {
            self.message = Some(match result {
                Ok(message) => {
                    bridge::info(&message);
                    (true, message)
                }
                Err(error) => (false, error),
            });
        }
    }

    pub fn page(&mut self, ui: &mut egui::Ui) {
        self.finish();
        let ctx = ui.ctx().clone();
        let site = bridge::site();
        let target = bridge::target();
        let busy = self.busy();
        ui.add_space(4.0);
        ui.heading(tr("Ruimtelijke plannen", "Spatial plans"));
        ui.label(tr("IMRO-plannen en TAM-plannen: plancontouren, bestemmingen en aanduidingen als bewerkbare geometrie.", "IMRO and TAM plans: plan boundaries, zoning and designations as editable geometry."));
        ui.label(RichText::new(tr("Dit overzicht bevat geen volledig actueel DSO-omgevingsplan. Controleer status en geldigheid bij de bron.", "This overview does not include the complete current DSO environmental plan. Check status and validity at the source.")).weak());
        ui.hyperlink_to(
            tr("Regels op de kaart openen", "Open Regels op de kaart"),
            "https://omgevingswet.overheid.nl/regels-op-de-kaart/",
        );
        ui.add_space(10.0);

        ui.group(|ui| {
            ui.strong(tr("Plannen op de projectlocatie zoeken", "Find plans at the project location"));
            if let Some(site) = &site {
                ui.label(format!("{} · {} · {:.2}, {:.2}", site.label, site.crs, site.origin_e, site.origin_n));
            } else {
                ui.label(tr("Stel eerst de projectlocatie in.", "Set the project location first."));
            }
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("Eigen API-sleutel", "Your API key"));
                    ui.add(egui::TextEdit::singleline(&mut self.key).password(true).desired_width(280.0));
                    if ui.button(tr("Wissen", "Clear")).clicked() { self.key.clear(); }
                });
                ui.label(RichText::new(tr("Alleen voor deze sessie; kan ook via OPENINFRA_RP_API_KEY worden ingesteld.", "For this session only; can also be set through OPENINFRA_RP_API_KEY.")).small().weak());
                if ui.add_enabled(site.is_some() && !self.key.trim().is_empty(), egui::Button::new(tr("Plannen zoeken", "Find plans"))).clicked() {
                    let site = site.clone().unwrap();
                    let key = self.key.trim().to_string();
                    self.plans.clear();
                    self.selected = None;
                    self.data = None;
                    self.source.clear();
                    self.message = None;
                    self.search = Some(Job::spawn(&ctx, tr("Plannen zoeken…", "Finding plans…"), move |progress| {
                        site.register_crs();
                        let point = crs::transform(&site.crs, "EPSG:28992", [site.origin_e, site.origin_n])?;
                        progress(format!("RD {:.2}, {:.2}", point[0], point[1]));
                        Client::new(key)?.search(point)
                    }));
                }
            });
        });

        if !self.plans.is_empty() {
            ui.add_space(8.0);
            ui.strong(tr("Kies een plan", "Choose a plan"));
            let mut pick = None;
            ui.add_enabled_ui(!busy, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("planning-results")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for plan in &self.plans {
                            let selected = self.selected.as_deref() == Some(plan.id.as_str());
                            if ui
                                .selectable_label(selected, format!("{} — {}", plan.name, plan.id))
                                .clicked()
                            {
                                pick = Some(plan.id.clone());
                            }
                            ui.label(
                                RichText::new(format!(
                                    "{} · {} · {}{}",
                                    plan.authority,
                                    plan.status,
                                    plan.date,
                                    if plan.tam { " · TAM" } else { "" }
                                ))
                                .small()
                                .weak(),
                            );
                            if plan.historical || plan.legal_end.is_some() {
                                ui.colored_label(
                                    Color32::from_rgb(229, 192, 123),
                                    format!(
                                        "{}{}",
                                        tr(
                                            "Historisch of einde rechtsgeldigheid: ",
                                            "Historical or legal validity end: "
                                        ),
                                        plan.legal_end.as_deref().unwrap_or("—")
                                    ),
                                );
                            }
                            ui.separator();
                        }
                    });
            });
            if let Some(id) = pick {
                if self.selected.as_deref() != Some(id.as_str()) {
                    self.selected = Some(id);
                    self.data = None;
                    self.source.clear();
                    self.message = None;
                }
            }
            if ui
                .add_enabled(
                    !busy && self.selected.is_some() && !self.key.trim().is_empty(),
                    egui::Button::new(tr("Gekozen plan ophalen", "Download selected plan")),
                )
                .clicked()
            {
                let id = self.selected.clone().unwrap();
                let key = self.key.trim().to_string();
                self.source = format!("{}/plannen/{id}", planning::BASE);
                self.data = None;
                self.download = Some(Job::spawn(
                    &ctx,
                    tr("Planobjecten ophalen…", "Downloading plan features…"),
                    move |_| Client::new(key)?.download(&id),
                ));
            }
        }

        if let Some(data) = &self.data {
            ui.add_space(8.0);
            ui.label(format!(
                "{} {} · {}",
                data.objects.len(),
                tr("objecten", "features"),
                data.crs
            ));
            for warning in &data.warnings {
                ui.colored_label(Color32::from_rgb(229, 192, 123), warning);
            }
            if ui
                .add_enabled(
                    !busy && site.is_some() && target.is_some(),
                    egui::Button::new(tr("Plan in tekening plaatsen", "Place plan in drawing")),
                )
                .clicked()
            {
                let (site, data, source) =
                    (site.clone().unwrap(), data.clone(), self.source.clone());
                let target = target.clone().unwrap();
                self.run = Some(Job::spawn(
                    &ctx,
                    tr("Plan plaatsen…", "Placing plan…"),
                    move |progress| {
                        bridge::with_command_context(target, Some(site.clone()), None, || {
                            crate::plannen::place(&site, &data, &source, progress)
                        })
                    },
                ));
            }
        }

        ui.add_space(12.0);
        ui.group(|ui| {
            ui.strong(tr("Lokaal IMRO-GML-bestand", "Local IMRO GML file"));
            ui.label(tr(
                "Importeer een gedownload planbestand zonder API-sleutel.",
                "Import a downloaded plan file without an API key.",
            ));
            ui.add_enabled_ui(!busy, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.path)
                        .hint_text(tr("Volledig pad naar .gml", "Full path to .gml"))
                        .desired_width(ui.available_width()),
                );
                if ui
                    .add_enabled(
                        site.is_some() && target.is_some() && !self.path.trim().is_empty(),
                        egui::Button::new(tr("GML in tekening plaatsen", "Place GML in drawing")),
                    )
                    .clicked()
                {
                    let site = site.clone().unwrap();
                    let target = target.clone().unwrap();
                    let path = PathBuf::from(self.path.trim().trim_matches('"'));
                    self.message = None;
                    self.run = Some(Job::spawn(
                        &ctx,
                        tr("GML importeren…", "Importing GML…"),
                        move |progress| {
                            bridge::with_command_context(target, Some(site.clone()), None, || {
                                crate::plannen::import_path(&site, &path, progress)
                            })
                        },
                    ));
                }
            });
        });
        for status in [
            self.search.as_ref().map(Job::status),
            self.download.as_ref().map(Job::status),
            self.run.as_ref().map(Job::status),
        ]
        .into_iter()
        .flatten()
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
            });
        }
        if let Some((ok, message)) = &self.message {
            ui.add_space(6.0);
            ui.colored_label(
                if *ok {
                    Color32::from_rgb(152, 195, 121)
                } else {
                    Color32::from_rgb(224, 108, 117)
                },
                message,
            );
        }
    }
}
