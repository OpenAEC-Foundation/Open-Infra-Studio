//! Application defaults and drawing jurisdiction are deliberately separate.
use super::*;
use crate::{
    lang::tr,
    localisation::{self, Region},
};
impl App {
    pub(super) fn localisation_page(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            "globe",
            tr("Localisatie", "Localisation"),
            tr(
                "Taal, projectregio en regionale databronnen.",
                "Language, project region and regional data sources.",
            ),
        );
        group(ui, tr("Toepassing", "Application"), |ui| {
            super::super::library::language_switch(ui);
            ui.label(tr("De standaardregio bepaalt het lint na herstart. Taal en regio zijn onafhankelijk.","The default region sets the ribbon after restart. Language and region are independent."));
        });
        let site = bridge::site();
        let mut region = site
            .as_ref()
            .map(|s| s.region)
            .unwrap_or(localisation::current());
        let before = region;
        group(ui, tr("Projectregio", "Project region"), |ui| {
            ui.add_enabled_ui(site.is_some(), |ui| {
                egui::ComboBox::from_id_salt("project-region")
                    .selected_text(region.label())
                    .show_ui(ui, |ui| {
                        for choice in localisation::REGIONS {
                            ui.selectable_value(&mut region, choice, choice.label());
                        }
                    });
            });
            if let Some(site) = &site {
                ui.label(format!("{} · {}", site.label, site.crs));
            } else {
                ui.label(tr(
                    "Stel eerst een projectlocatie in om de regio in de tekening te bewaren.",
                    "Set a project location first to store the region in the drawing.",
                ));
            }
            ui.label(tr("Een regiowijziging verplaatst geen geometrie en wijzigt geen CRS, tekeneenheden of hoogtedatum.","Changing region does not move geometry or change the CRS, drawing units or vertical datum."));
        });
        if before != region {
            if let Some(mut site) = site {
                site.region = region;
                match bridge::save_site(&site) {
                    Ok(()) => {
                        self.draft.region = region;
                        self.ok(tr(
                            "Projectregio opgeslagen in de tekening.",
                            "Project region stored in the drawing.",
                        ));
                    }
                    Err(e) => self.err(e),
                }
            }
        }
        group(ui, region.label(), |ui| {
            ui.label(region.guidance());
            ui.label(format!(
                "{}: {}",
                tr("Tekenstandaard", "Drawing standard"),
                region.standard()
            ));
            if let Some(target) = bridge::target() {
                ui.label(format!(
                    "{}: {}",
                    tr("Tekeneenheden per meter", "Drawing units per metre"),
                    target.unit_factor
                ));
            }
            if ui
                .button(tr("Regionale services openen", "Open regional services"))
                .clicked()
            {
                self.svc.regional(region);
                self.tab = Tab::Services;
            }
            if region == Region::UnitedStates {
                ui.label(tr("International foot = 0,3048 m. Historische US survey foot = 1200/3937 m. Bron- en tekeneenheden moeten apart worden vastgesteld.","International foot = 0.3048 m. Historical US survey foot = 1200/3937 m. Source and drawing units must be identified separately."));
                ui.label(tr("SPCS2022/het gemoderniseerde NSRS staat op 31 oktober 2026 nog op de NGS-bètasite. Controleer de actuele officiële status en de projecteisen.","As of 31 October 2026, SPCS2022/modernized NSRS tools remain on the NGS beta site. Check current official status and project requirements."));
            }
            for &(label, url) in region.references() {
                ui.hyperlink_to(label, url);
            }
        });
    }
}
