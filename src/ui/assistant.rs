//! The Assistant page: what an AI assistant proposed (accept, reject,
//! dismiss, zoom to), whether its changes are proposed or applied, and the
//! activity log of the drawing.

use eframe::egui::{self, Color32, RichText, Vec2};

use super::jobs::{poll, Job};
use crate::assistant::activity::{self, Entry, State};
use crate::assistant::proposal::{self, Proposal, Status};
use crate::assistant::{self, capture, Mode};
use crate::lang::tr;

const ACCENT: Color32 = Color32::from_rgb(33, 150, 220);
const OK_GREEN: Color32 = Color32::from_rgb(152, 195, 121);
const ERR_RED: Color32 = Color32::from_rgb(224, 108, 117);
const WARN: Color32 = Color32::from_rgb(229, 192, 123);

type Overview = (Vec<(Proposal, Result<Status, String>)>, Vec<Entry>);

pub struct AssistantPage {
    mode: Mode,
    proposals: Vec<(Proposal, Result<Status, String>)>,
    log: Vec<Entry>,
    load: Option<Job<Overview>>,
    act: Option<Job<String>>,
    /// Reload on the next frame.
    dirty: bool,
    only_assistants: bool,
    message: Option<(bool, String)>,
}

impl Default for AssistantPage {
    fn default() -> Self {
        Self { mode: assistant::mode(), proposals: Vec::new(), log: Vec::new(), load: None, act: None, dirty: true, only_assistants: false, message: None }
    }
}

fn badge(ui: &mut egui::Ui, text: &str, colour: Color32) {
    egui::Frame::new().fill(colour.gamma_multiply(0.25)).corner_radius(3.0).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).small().strong().color(colour));
    });
}

fn state_colour(s: State) -> Color32 {
    match s {
        State::Pending => ACCENT,
        State::Accepted | State::Applied | State::Done => OK_GREEN,
        State::Rejected | State::Failed => ERR_RED,
        State::Dismissed | State::NoChange => WARN,
    }
}

fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(10)).corner_radius(4.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).small().strong().color(ACCENT));
        ui.add_space(2.0);
        add(ui);
    });
    ui.add_space(6.0);
}

impl AssistantPage {
    /// Proposals or the log changed (a message from the plugin).
    pub fn changed(&mut self) {
        self.dirty = true;
    }

    fn start(&mut self, ctx: &egui::Context, label: &str, f: impl FnOnce() -> Result<String, String> + Send + 'static) {
        self.act = Some(Job::spawn(ctx, label, move |_| capture::without(f)));
    }

    pub fn page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(r) = poll(&mut self.load) {
            match r {
                Ok((p, l)) => {
                    self.proposals = p;
                    self.log = l;
                }
                Err(e) => self.message = Some((false, e)),
            }
        }
        if let Some(r) = poll(&mut self.act) {
            self.message = Some(match r {
                Ok(m) => (true, m),
                Err(e) => (false, e),
            });
            self.dirty = true;
        }
        if self.dirty && self.load.is_none() && self.act.is_none() && crate::bridge::target().is_some() {
            self.dirty = false;
            self.load = Some(Job::spawn(&ctx, tr("Voorstellen lezen…", "Reading proposals…"), |_| Ok((proposal::overview(), activity::read()))));
        }
        let busy = self.load.is_some() || self.act.is_some();

        ui.add_space(4.0);
        ui.label(RichText::new(tr("Assistent", "Assistant")).size(18.0).strong());
        ui.label(
            RichText::new(tr(
                "Wat een AI-assistent via de MCP-server in deze tekening doet: voorstellen om te accepteren of af te wijzen, en een logboek in gewone woorden.",
                "What an AI assistant does in this drawing through the MCP server: proposals to accept or reject, and a log in plain words.",
            ))
            .weak(),
        );
        ui.add_space(6.0);
        if let Some((ok, m)) = &self.message {
            ui.colored_label(if *ok { OK_GREEN } else { ERR_RED }, m);
            ui.add_space(4.0);
        }

        group(ui, tr("Wijzigingen van een assistent", "Changes by an assistant"), |ui| {
            let before = self.mode;
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.mode, Mode::Propose, tr("Voorstellen (beoordelen)", "Propose (review first)"));
                ui.radio_value(&mut self.mode, Mode::Apply, tr("Direct toepassen", "Apply at once"));
            });
            ui.label(
                RichText::new(tr(
                    "Voorstellen: een bewerking van de assistent komt in de tekening, gemarkeerd met een kader op laag OI-ASSISTANT-PROPOSAL, en wacht tot u accepteert of afwijst. Direct toepassen: zoals vroeger, alleen ongedaan maken helpt dan. Beide worden gelogd.",
                    "Propose: an assistant's edit enters the drawing, marked by a frame on layer OI-ASSISTANT-PROPOSAL, and waits until you accept or reject it. Apply at once: as before, only undo helps then. Both are logged.",
                ))
                .small()
                .weak(),
            );
            if self.mode != before {
                self.message = Some(match assistant::set_mode(self.mode) {
                    Ok(()) => (true, format!("{} {}", tr("Assistent:", "Assistant:"), if self.mode == Mode::Propose { tr("voorstellen", "propose") } else { tr("direct toepassen", "apply at once") })),
                    Err(e) => (false, e),
                });
            }
        });

        let mut action: Option<(String, u8)> = None;
        group(ui, tr("Openstaande voorstellen", "Pending proposals"), |ui| {
            if self.proposals.is_empty() {
                ui.label(RichText::new(tr("Geen openstaande voorstellen.", "No pending proposals.")).weak());
            }
            for (p, st) in &self.proposals {
                let (stale, why) = match st {
                    Ok(s) if s.stale && !s.changed_by.is_empty() => (true, format!("{} {}", tr("gewijzigd door het latere voorstel", "changed by the later proposal"), s.changed_by.join(", "))),
                    Ok(s) if s.stale && s.missing > 0 => (true, format!("{} {}", s.missing, tr("elementen verwijderd sinds het voorstel", "entities deleted since the proposal"))),
                    Ok(s) if s.stale => (true, tr("bewerkt sinds het voorstel", "edited since the proposal").to_string()),
                    Ok(_) => (false, String::new()),
                    Err(e) => (true, e.clone()),
                };
                egui::Frame::new().fill(ui.visuals().extreme_bg_color).corner_radius(3.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&p.id).strong().color(ACCENT));
                        ui.label(RichText::new(p.title()).strong());
                        if stale {
                            badge(ui, tr("verouderd", "stale"), WARN);
                        } else {
                            badge(ui, State::Pending.label(), ACCENT);
                        }
                    });
                    ui.label(RichText::new(format!("{} · {} · {} · +{} ~{} −{}", assistant::short_time(&p.created), p.client, p.tool, p.added.len(), p.modified.len(), p.removed.len())).small().weak());
                    if !p.message.is_empty() {
                        ui.label(RichText::new(&p.message).small());
                    }
                    if stale {
                        ui.label(RichText::new(format!("{why} — {}", tr("alleen accepteren of laten vervallen", "accept or dismiss only"))).small().color(WARN));
                    } else if !p.restorable {
                        ui.label(RichText::new(tr("De vorige toestand is niet bewaard: alleen accepteren of laten vervallen.", "The previous state was not kept: accept or dismiss only.")).small().color(WARN));
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new(tr("Accepteren", "Accept"))).clicked() {
                            action = Some((p.id.clone(), 0));
                        }
                        if ui.add_enabled(!busy && !stale && p.restorable, egui::Button::new(tr("Afwijzen", "Reject"))).clicked() {
                            action = Some((p.id.clone(), 1));
                        }
                        if (stale || !p.restorable) && ui.add_enabled(!busy, egui::Button::new(tr("Laten vervallen", "Dismiss"))).clicked() {
                            action = Some((p.id.clone(), 2));
                        }
                        if ui.add_enabled(!busy && p.bbox.is_some(), egui::Button::new(tr("Inzoomen", "Zoom to"))).clicked() {
                            action = Some((p.id.clone(), 3));
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
        if let Some((id, what)) = action {
            match what {
                0 => self.start(&ctx, tr("Accepteren…", "Accepting…"), move || proposal::accept(&id, "user")),
                1 => self.start(&ctx, tr("Afwijzen…", "Rejecting…"), move || proposal::reject(&id, "user")),
                2 => self.start(&ctx, tr("Laten vervallen…", "Dismissing…"), move || proposal::dismiss(&id, "user")),
                _ => self.start(&ctx, tr("Inzoomen…", "Zooming…"), move || proposal::zoom(&id)),
            }
        }

        group(ui, tr("Activiteitenlog", "Activity log"), |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.only_assistants, tr("Alleen assistenten", "Assistants only"));
                ui.label(RichText::new(format!("{} {}", self.log.len(), tr("regels, in de tekening bewaard", "lines, kept in the drawing"))).small().weak());
            });
            if self.log.is_empty() {
                ui.label(RichText::new(tr("Nog niets gelogd.", "Nothing logged yet.")).weak());
            }
            egui::Grid::new("assistant_log").striped(true).num_columns(5).spacing(Vec2::new(10.0, 3.0)).show(ui, |ui| {
                for e in self.log.iter().rev().filter(|e| !self.only_assistants || e.who != "user").take(200) {
                    ui.label(RichText::new(assistant::short_time(&e.at)).small().monospace());
                    ui.label(RichText::new(e.who_label()).small());
                    ui.label(RichText::new(e.title()).small()).on_hover_text(format!("{}\n{}", e.command, e.message));
                    ui.label(RichText::new(e.counts()).small().monospace());
                    ui.horizontal(|ui| {
                        badge(ui, e.state.label(), state_colour(e.state));
                        if let Some(p) = &e.proposal {
                            ui.label(RichText::new(p).small().weak());
                        }
                    });
                    ui.end_row();
                }
            });
        });

        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(tr("Vernieuwen", "Refresh"))).clicked() {
                self.dirty = true;
            }
            if let Some(j) = self.load.as_ref() {
                ui.spinner();
                ui.label(j.status());
            } else if let Some(j) = self.act.as_ref() {
                ui.spinner();
                ui.label(j.status());
            }
        });
    }
}
