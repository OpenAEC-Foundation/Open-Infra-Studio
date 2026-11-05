//! The Problems page: every design check in one list (`CIV_PROBLEMEN`), by
//! severity and object, open and accepted. Per problem: zoom to it in the
//! drawing, apply a fix the program computed, or accept it with a reason;
//! and run the checks again.

use eframe::egui::{self, Color32, RichText};

use super::app::{badge, button_bar, group, job_status, list_frame, page_header, primary_button, ACCENT, ERR_RED, OK_GREEN, WARN};
use super::jobs::{poll, Job};
use crate::civiel::problems::{self, Checks, Problem, Scope, Severity, Status};
use crate::lang::tr;

pub struct ProblemsPage {
    problems: Vec<Problem>,
    notes: Vec<String>,
    checks: Checks,
    selection: bool,
    markers: bool,
    show_accepted: bool,
    /// The problem whose acceptance is being typed (index), and the reason.
    accepting: Option<usize>,
    reason: String,
    author: String,
    run: Option<Job<problems::Found>>,
    action: Option<Job<String>>,
    message: Option<(bool, String)>,
    ran: bool,
}

impl Default for ProblemsPage {
    fn default() -> Self {
        Self {
            problems: Vec::new(),
            notes: Vec::new(),
            checks: Checks::all(),
            selection: false,
            markers: true,
            show_accepted: true,
            accepting: None,
            reason: String::new(),
            author: problems::author(),
            run: None,
            action: None,
            message: None,
            ran: false,
        }
    }
}

fn colour(s: Severity) -> Color32 {
    match s {
        Severity::Error => ERR_RED,
        Severity::Warning => WARN,
        Severity::Info => Color32::from_gray(150),
    }
}

impl ProblemsPage {
    fn busy(&self) -> bool {
        self.run.is_some() || self.action.is_some()
    }

    /// Run the checks (and draw the markers) on a worker.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let (checks, markers) = (self.checks.clone(), self.markers);
        let scope = if self.selection { Scope::Selection } else { Scope::All };
        self.run = Some(Job::spawn(ctx, tr("Toetsen…", "Checking…"), move |p| {
            let opt = crate::civiel::settings();
            let mut found = problems::collect(&opt, &scope, &checks, p)?;
            if markers {
                let c = crate::civiel::Ctx::new(&opt)?;
                problems::draw_markers(&c, &mut found.problems, &scope)?;
            }
            Ok(found)
        }));
    }

    fn finish(&mut self, ctx: &egui::Context) {
        if let Some(r) = poll(&mut self.run) {
            match r {
                Ok(found) => {
                    let open = found.problems.iter().filter(|p| p.status == Status::Open).count();
                    let accepted = found.problems.len() - open;
                    self.message = Some((true, format!("{open} {}, {accepted} {}.", tr("open", "open"), tr("geaccepteerd", "accepted"))));
                    self.problems = found.problems;
                    self.notes = found.notes;
                    self.ran = true;
                }
                Err(e) => self.message = Some((false, e)),
            }
        }
        if let Some(r) = poll(&mut self.action) {
            match r {
                Ok(m) => {
                    crate::bridge::info(&m);
                    self.message = Some((true, m));
                }
                Err(e) => self.message = Some((false, e)),
            }
            // Fixes and acceptances change the list: check again.
            self.start(ctx);
        }
    }

    pub fn page(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.finish(&ctx);
        page_header(
            ui,
            "problems",
            tr("Problemen", "Problems"),
            tr(
                "Alle toetsen in één lijst: as en bogen, ontwerpcriteria, ontwerpprofiel, riolering, grondwerk, corridors, verouderde profielen en NLCS. Met herstel dat het programma uitrekent, of accepteren met een reden.",
                "Every check in one list: alignment and curves, design criteria, design profile, sewers, grading, corridors, out-of-date profiles and NLCS. With fixes the program computes, or accept with a reason.",
            ),
        );
        group(ui, tr("Toetsen", "Checks"), |ui| {
            ui.horizontal_wrapped(|ui| {
                let c = &mut self.checks;
                ui.checkbox(&mut c.alignment, tr("As en bogen", "Alignment and curves"));
                ui.checkbox(&mut c.criteria, tr("Ontwerpcriteria", "Design criteria"));
                ui.checkbox(&mut c.profile, tr("Ontwerpprofiel", "Design profile"));
                ui.checkbox(&mut c.sewer, tr("Riolering", "Sewers"));
                ui.checkbox(&mut c.grading, tr("Grondwerk", "Grading"));
                ui.checkbox(&mut c.corridor, "Corridor");
                ui.checkbox(&mut c.products, tr("Verouderde profielen", "Out-of-date profiles"));
                ui.checkbox(&mut c.nlcs, "NLCS");
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut self.selection, false, tr("Hele tekening", "Whole drawing"));
                ui.radio_value(&mut self.selection, true, tr("Alleen de selectie", "Selection only"));
                ui.add_space(12.0);
                ui.checkbox(&mut self.markers, tr("Markeringen tekenen (OI-PROBLEMEN)", "Draw markers (OI-PROBLEMEN)"));
                ui.checkbox(&mut self.show_accepted, tr("Geaccepteerde tonen", "Show accepted"));
            });
            ui.horizontal(|ui| {
                ui.label(tr("Naam bij accepteren", "Name with acceptances"));
                let r = ui.add(egui::TextEdit::singleline(&mut self.author).desired_width(200.0).hint_text(tr("optioneel", "optional")));
                if r.lost_focus() {
                    let _ = problems::save_author(&self.author);
                }
            });
        });
        if let Some((ok, m)) = &self.message {
            ui.colored_label(if *ok { OK_GREEN } else { ERR_RED }, m);
        }
        for n in &self.notes {
            ui.colored_label(WARN, n);
        }
        if !self.ran {
            group(ui, tr("Resultaat", "Result"), |ui| {
                job_status(ui, &self.run);
                if self.run.is_none() {
                    ui.label(RichText::new(tr("Klik op Toetsen.", "Click Check.")).weak());
                }
            });
            return;
        }
        let count = |s: Severity| self.problems.iter().filter(|p| p.status == Status::Open && p.severity == s).count();
        let accepted = self.problems.iter().filter(|p| p.status == Status::Accepted).count();
        ui.horizontal(|ui| {
            badge(ui, &format!("{} {}", count(Severity::Error), tr("fout(en)", "error(s)")), ERR_RED);
            badge(ui, &format!("{} {}", count(Severity::Warning), tr("waarschuwing(en)", "warning(s)")), WARN);
            badge(ui, &format!("{} info", count(Severity::Info)), Color32::from_gray(150));
            badge(ui, &format!("{accepted} {}", tr("geaccepteerd", "accepted")), OK_GREEN);
        });
        ui.add_space(4.0);
        if self.problems.is_empty() {
            ui.colored_label(OK_GREEN, tr("Geen problemen gevonden.", "No problems found."));
            return;
        }
        let mut todo: Option<Todo> = None;
        let busy = self.busy();
        for (title, status) in [(tr("Open", "Open"), Status::Open), (tr("Geaccepteerd", "Accepted"), Status::Accepted)] {
            if status == Status::Accepted && !self.show_accepted {
                continue;
            }
            let list: Vec<usize> = (0..self.problems.len()).filter(|i| self.problems[*i].status == status).collect();
            if list.is_empty() {
                continue;
            }
            group(ui, &format!("{title} ({})", list.len()), |ui| {
                list_frame(ui, 520.0, &format!("problems-{title}"), |ui| {
                    let mut last_object = String::new();
                    for i in list {
                        let p = &self.problems[i];
                        if p.about.name != last_object {
                            last_object = p.about.name.clone();
                            ui.add_space(4.0);
                            ui.label(RichText::new(format!("{} · {}", p.about.name, p.about.kind)).strong().color(ACCENT));
                        }
                        if let Some(t) = row(ui, i, p, busy, &mut self.accepting, &mut self.reason) {
                            todo = Some(t);
                        }
                    }
                });
            });
        }
        if let Some(t) = todo {
            self.act(&ctx, t);
        }
    }

    fn act(&mut self, ctx: &egui::Context, t: Todo) {
        match t {
            Todo::Zoom(i) => {
                let p = self.problems[i].clone();
                self.action = Some(Job::spawn(ctx, tr("Zoomen…", "Zooming…"), move |_| problems::zoom(&p).map(|_| format!("{} {}", tr("Ingezoomd op", "Zoomed to"), p.object))));
            }
            Todo::Fix(i, f) => {
                let p = self.problems[i].clone();
                self.action = Some(Job::spawn(ctx, tr("Herstellen…", "Fixing…"), move |pr| {
                    let opt = crate::civiel::settings();
                    problems::fix_command(&opt, &p.code, &p.object, Some(&p.fixes[f].id), pr)
                }));
            }
            Todo::Accept(i) => {
                let (p, reason) = (self.problems[i].clone(), self.reason.trim().to_string());
                let _ = problems::save_author(&self.author);
                self.accepting = None;
                self.reason.clear();
                self.action = Some(Job::spawn(ctx, tr("Accepteren…", "Accepting…"), move |pr| {
                    let opt = crate::civiel::settings();
                    problems::accept_command(&opt, &p.code, &p.object, &reason, pr)
                }));
            }
            Todo::Reopen(i) => {
                let p = self.problems[i].clone();
                self.action = Some(Job::spawn(ctx, tr("Heropenen…", "Reopening…"), move |_| problems::reopen(&p.code, &p.object)));
            }
        }
    }

    pub fn actions(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let busy = self.busy();
        button_bar(ui, |ui| {
            if ui.add_enabled(!busy, primary_button(tr("Toetsen", "Check"))).on_hover_text(tr("Alle gekozen toetsen opnieuw uitvoeren", "Run the chosen checks again")).clicked() {
                self.start(&ctx);
            }
            job_status(ui, &self.run);
            job_status(ui, &self.action);
        });
    }
}

enum Todo {
    Zoom(usize),
    Fix(usize, usize),
    Accept(usize),
    Reopen(usize),
}

/// One problem: severity, title, message, required/found and its actions.
fn row(ui: &mut egui::Ui, i: usize, p: &Problem, busy: bool, accepting: &mut Option<usize>, reason: &mut String) -> Option<Todo> {
    let mut todo = None;
    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            badge(ui, p.severity.label().get(), colour(p.severity));
            ui.label(RichText::new(p.title.get()).strong());
            ui.label(RichText::new(&p.code).monospace().small().weak());
        });
        ui.label(RichText::new(p.message.get()).small());
        if let (Some(r), Some(f)) = (&p.required, &p.found) {
            ui.label(RichText::new(format!("{}: {} {} · {}: {} {}", tr("eis", "required"), fmt(r.value), r.unit, tr("gevonden", "found"), fmt(f.value), f.unit)).small().monospace());
        }
        if let Some(s) = &p.source {
            ui.label(RichText::new(format!("{}: {s}", tr("bron", "source"))).small().weak());
        }
        if let Some(a) = &p.accepted {
            ui.label(RichText::new(format!("✓ {}: {} — {}{}", tr("geaccepteerd", "accepted"), a.reason, a.date, if a.author.is_empty() { String::new() } else { format!(", {}", a.author) })).small().color(OK_GREEN));
        } else if p.acceptance_outdated {
            ui.label(RichText::new(tr("Eerder geaccepteerd; het object is sindsdien gewijzigd.", "Accepted before; the object has changed since.")).small().color(WARN));
        }
        if p.fixes.is_empty() {
            if let Some(w) = &p.no_fix {
                ui.label(RichText::new(format!("{}: {}", tr("geen herstel", "no fix"), w.get())).small().weak());
            }
        }
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!busy, egui::Button::new(tr("Zoom", "Zoom")).small()).on_hover_text(tr("Inzoomen op het probleem en het object selecteren", "Zoom to the problem and select the object")).clicked() {
                todo = Some(Todo::Zoom(i));
            }
            for (k, f) in p.fixes.iter().enumerate() {
                if ui.add_enabled(!busy && p.status == Status::Open, egui::Button::new(format!("⚒ {}", f.label.get())).small()).on_hover_text(tr("Toepassen: één stap ongedaan te maken", "Apply: one undo step")).clicked() {
                    todo = Some(Todo::Fix(i, k));
                }
            }
            if p.status == Status::Open {
                if *accepting == Some(i) {
                    ui.add(egui::TextEdit::singleline(reason).desired_width(260.0).hint_text(tr("reden", "reason")));
                    if ui.add_enabled(!busy && !reason.trim().is_empty(), egui::Button::new(tr("Accepteren", "Accept")).small()).clicked() {
                        todo = Some(Todo::Accept(i));
                    }
                    if ui.small_button(tr("Annuleren", "Cancel")).clicked() {
                        *accepting = None;
                    }
                } else if ui.add_enabled(!busy, egui::Button::new(tr("Accepteren…", "Accept…")).small()).clicked() {
                    *accepting = Some(i);
                    reason.clear();
                }
            } else if ui.add_enabled(!busy, egui::Button::new(tr("Weer openen", "Open again")).small()).clicked() {
                todo = Some(Todo::Reopen(i));
            }
        });
    });
    ui.separator();
    todo
}

fn fmt(v: f64) -> String {
    let t = if v.abs() >= 100.0 { format!("{v:.1}") } else { format!("{v:.2}") };
    if crate::lang::current() == crate::lang::Lang::Nl { t.replace('.', ",") } else { t }
}
