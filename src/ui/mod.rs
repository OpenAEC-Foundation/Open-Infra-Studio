//! The GIS workbench window. It runs on its own thread in the plugin process
//! (Open CAD Studio has no plugin dialog API), next to the CAD window, and
//! writes into the drawing through [`crate::bridge`].
//!
//! The thread and workbench state outlive each native window. eframe reuses
//! the thread's winit event loop when a host command opens the next window.

mod app;
mod files;
mod jobs;
mod library;
mod infra;
mod map;
mod planning;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Localisation,
    Location,
    Raster,
    Wfs,
    Osm,
    Pdok,
    Nlcs,
    Ahn,
    Controle,
    Hoeveelheden,
    Servers,
    Coords,
    Library,
    Files,
    /// Service browser: WMS/WFS/tile services worldwide.
    Services,
    /// Legacy IMRO and TAM spatial planning data.
    Planning,
    /// The Civiel plugin's settings and actions.
    Civiel,
}

/// Which plugin this window belongs to: the GIS tab or the Civiel tab (each
/// plugin runs in its own process with its own window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    Gis,
    Civiel,
}

static PRODUCT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_product(p: Product) {
    PRODUCT.store(p as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn product() -> Product {
    if PRODUCT.load(std::sync::atomic::Ordering::Relaxed) == Product::Civiel as u8 { Product::Civiel } else { Product::Gis }
}

pub enum UiMsg {
    RegionalServices(crate::localisation::Region),
    InfraLibrary,
    Show(Tab),
    /// A point picked in the drawing (project CRS), for the coordinate tab.
    Picked { point: [f64; 2], source_crs: String },
    /// OCS closed or replaced the drawing in this tab.
    DrawingClosed(u64),
    /// A KLIC delivery chosen in Open CAD Studio's file dialog.
    Klic(std::path::PathBuf),
    /// A GIS file chosen in Open CAD Studio's file dialog.
    File(std::path::PathBuf),
    /// Run the page's action (validation, quantities) right away.
    Run(Tab),
}

struct Message {
    target: Option<crate::bridge::Target>,
    site: Option<crate::gis::site::Site>,
    action: UiMsg,
}

impl Message {
    fn capture(action: UiMsg) -> Self {
        let (target, site) = crate::bridge::workbench_document();
        Self { target, site, action }
    }
}

struct Handle {
    tx: Option<Sender<Message>>,
    ctx: Option<egui::Context>,
}

static HANDLE: Mutex<Handle> = Mutex::new(Handle { tx: None, ctx: None });

pub fn send(msg: UiMsg) -> bool {
    let msg = Message::capture(msg);
    let handle = HANDLE.lock().unwrap();
    let sent = handle.tx.as_ref().is_some_and(|tx| tx.send(msg).is_ok());
    if sent {
        request_repaint(&handle);
    }
    sent
}

pub(super) fn repaint() {
    request_repaint(&HANDLE.lock().unwrap());
}

fn request_repaint(handle: &Handle) {
    if let Some(ctx) = &handle.ctx {
        ctx.request_repaint();
    }
}

/// Test builds only, with the bridge-trace marker in the config folder: let a
/// debugger attach to this plugin process (Linux restricts ptrace to parents),
/// to see where a window hangs.
pub(crate) fn allow_debugger() {
    #[cfg(all(target_os = "linux", any(feature = "ui-test", debug_assertions)))]
    if crate::gis::servers::config_dir().join("bridge-trace").exists() {
        extern "C" {
            fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
        }
        const PR_SET_PTRACER: i32 = 0x5961_6d61;
        // SAFETY: prctl with PR_SET_PTRACER only changes who may trace this process.
        unsafe {
            prctl(PR_SET_PTRACER, u64::MAX, 0, 0, 0);
        }
    }
}

/// Show the workbench on `tab`, creating the window on first use.
pub fn open(tab: Tab) -> Result<(), String> {
    let message = Message::capture(UiMsg::Show(tab));
    let mut handle = HANDLE.lock().unwrap();
    if let Some(tx) = &handle.tx {
        if tx.send(message).is_ok() {
            request_repaint(&handle);
            return Ok(());
        }
        return Err("de GIS-workbench is gestopt".into());
    }
    // On macOS the GIS plugin starts the UI loop on its main thread.
    if cfg!(target_os = "macos") {
        return Err("het venster is op macOS niet beschikbaar; gebruik de opdrachten".into());
    }
    let (tx, rx) = channel();
    tx.send(message).expect("new workbench channel");
    allow_debugger();
    std::thread::Builder::new()
        .name("open-infra-ui".into())
        .spawn(move || run_workbench(rx))
        .map_err(|e| e.to_string())?;
    handle.tx = Some(tx);
    Ok(())
}

#[derive(Default)]
struct Workbench {
    drawings: HashMap<Option<u64>, app::App>,
    civiel_offsets: Option<String>,
}

impl Workbench {
    fn with_document(
        &mut self,
        target: Option<crate::bridge::Target>,
        site: Option<crate::gis::site::Site>,
        run: impl FnOnce(&mut app::App, &mut Option<String>),
    ) {
        let tab = target.as_ref().map(|target| target.tab_id);
        // Pin reads, writes and new jobs to the drawing whose state is shown
        // even if a host command attaches another drawing mid-frame.
        crate::bridge::with_workbench_document(target, site, || {
            let app = self.drawings.entry(tab).or_insert_with(app::App::new);
            run(app, &mut self.civiel_offsets);
        });
    }

    fn with_document_ui(
        &mut self,
        ui: &mut egui::Ui,
        draw: impl FnOnce(&mut app::App, &mut egui::Ui, &mut Option<String>),
    ) {
        let (target, site) = crate::bridge::workbench_document();
        self.with_document(target, site.clone(), |app, civiel_offsets| {
            app.sync_site(site);
            ui.push_id(app.widget_scope, |ui| draw(app, ui, civiel_offsets));
        });
    }

    // A late reply can update its own drawing's page, but only a message for
    // the current drawing may focus or open the native window.
    fn handle_message(&mut self, msg: Message) -> bool {
        if let UiMsg::DrawingClosed(tab_id) = msg.action {
            self.drawings.remove(&Some(tab_id));
            return false;
        }
        if msg.target.as_ref().is_some_and(|target| !crate::bridge::workbench_target_is_open(target)) {
            return false;
        }

        let (active, saved_site) = crate::bridge::workbench_document();
        let focus = match (&msg.target, &active) {
            (Some(target), Some(active)) => std::sync::Arc::ptr_eq(&target.sender, &active.sender),
            (None, None) => true,
            _ => false,
        };
        let site = if focus { saved_site } else { msg.site };
        self.with_document(msg.target, site.clone(), |app, _| {
            if focus {
                app.sync_site(site);
            }
            app.handle_message(msg.action);
        });
        focus
    }
}

fn run_workbench(rx: Receiver<Message>) {
    let mut workbench = Workbench::default();
    while let Ok(msg) = rx.recv() {
        if !workbench.handle_message(msg) {
            continue;
        }

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(if product() == Product::Civiel { "Open Infra — Civiel" } else { "Open Infra — GIS-workbench" })
                .with_app_id(if product() == Product::Civiel { "open-infra-civiel" } else { "open-infra-gis" })
                .with_inner_size([1280.0, 1000.0])
                .with_min_inner_size([760.0, 560.0])
                .with_icon(std::sync::Arc::new(app_icon())),
            run_and_return: true,
            event_loop_builder: Some(Box::new(|builder| {
                // Linux and Windows run IPC on the main thread. Both Linux
                // extension traits set the same platform-wide any_thread flag.
                #[cfg(all(unix, not(target_os = "macos")))]
                winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(builder, true);
                #[cfg(windows)]
                winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(builder, true);
                #[cfg(target_os = "macos")]
                winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(builder, false);
                let _ = builder;
            })),
            ..Default::default()
        };
        let result = eframe::run_native(
            "Open Infra GIS",
            options,
            Box::new(|cc| {
                app::App::configure_window(&cc.egui_ctx);
                HANDLE.lock().unwrap().ctx = Some(cc.egui_ctx.clone());
                Ok(Box::new(Window { workbench: &mut workbench, rx: &rx }))
            }),
        );
        HANDLE.lock().unwrap().ctx = None;
        for app in workbench.drawings.values_mut() {
            app.reset_window();
        }
        if let Err(e) = result {
            crate::bridge::error(&format!("GIS-venster kon niet starten: {e}"));
        }
    }
}

struct Window<'a> {
    workbench: &'a mut Workbench,
    rx: &'a Receiver<Message>,
}

impl eframe::App for Window<'_> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Let eframe destroy the native window. Messages arriving during the
        // close remain queued for the next window.
        if ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        for msg in self.rx.try_iter() {
            if self.workbench.handle_message(msg) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.workbench.with_document_ui(ui, |app, ui, offsets| app.ui(ui, offsets));
    }
}

/// The host calls its self-hosted plugin entry point on the process main
/// thread. AppKit also requires that thread, so serve host IPC on a worker.
#[cfg(all(target_os = "macos", feature = "plugin"))]
pub(crate) fn run_macos_plugin(socket: String) -> Result<(), String> {
    let (tx, rx) = channel();
    HANDLE.lock().unwrap().tx = Some(tx);
    std::thread::Builder::new()
        .name("open-infra-ipc".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(|| {
                ocs_plugin_api::runner::run_with_plugin(
                    &socket,
                    ocs_plugin_api::API_VERSION,
                    Box::new(crate::OpenInfra),
                )
            });
            // This is a dedicated plugin runner. The host normally exits
            // the process with this same status when its entry point returns.
            std::process::exit(match result {
                Ok(Ok(())) => 0,
                Ok(Err(e)) => {
                    eprintln!("[plugin] {e}");
                    1
                }
                Err(_) => 2,
            });
        })
        .map_err(|e| e.to_string())?;
    run_workbench(rx);
    Ok(())
}

/// The window and taskbar icon: a map pin for GIS, a profile for Civiel.
fn app_icon() -> egui::IconData {
    let png: &[u8] = if product() == Product::Civiel { include_bytes!("../../assets/app-icon-civiel.png") } else { include_bytes!("../../assets/app-icon-gis.png") };
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

#[cfg(test)]
mod widget_scope_tests {
    use super::*;
    use crate::bridge::{self, Target};
    use ocs_plugin_api::host::{PluginRequestError, PluginRequestSender};
    use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
    use std::sync::Arc;

    struct NoHost;

    impl PluginRequestSender for NoHost {
        fn request(&self, _: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
            panic!("a widget-state test must not contact CAD");
        }
    }

    fn frame(
        ctx: &egui::Context,
        workbench: &mut Workbench,
        tab_id: u64,
        text: &mut String,
        input: egui::RawInput,
    ) -> (egui::Id, egui::Response) {
        let target = Target {
            sender: Arc::new(NoHost),
            tab_id,
            unit_factor: 1.0,
            doc_path: None,
        };
        let mut widgets = None;
        ctx.run_ui(input, |ui| {
            bridge::with_workbench_document(Some(target.clone()), None, || {
                workbench.with_document_ui(ui, |_, ui, _| {
                    let id = ui.text_edit_singleline(text).id;
                    widgets = Some((id, ui.button("→")));
                });
            });
        }).drop_without_applying_deltas();
        widgets.unwrap()
    }

    #[test]
    fn undo_after_switching_drawings_does_not_use_the_previous_draft() {
        let ctx = egui::Context::default();
        let mut workbench = Workbench::default();
        let mut a = "Edited A".to_owned();
        let (id_a, _) = frame(&ctx, &mut workbench, 1, &mut a, egui::RawInput::default());
        let mut state = egui::TextEdit::load_state(&ctx, id_a).unwrap();
        let mut undoer = state.undoer();
        undoer.add_undo(&(egui::text::CCursorRange::default(), "Concept A".into()));
        state.set_undoer(undoer);
        egui::TextEdit::store_state(&ctx, id_a, state);

        let mut b = "Concept B".to_owned();
        let (id_b, _) = frame(&ctx, &mut workbench, 2, &mut b, egui::RawInput::default());
        ctx.memory_mut(|memory| memory.request_focus(id_b));
        let modifiers = egui::Modifiers::COMMAND;
        let key = egui::Event::Key {
            key: egui::Key::Z, physical_key: Some(egui::Key::Z),
            pressed: true, repeat: false, modifiers,
        };
        let input = egui::RawInput { events: vec![key], ..Default::default() };
        frame(&ctx, &mut workbench, 2, &mut b, input);
        assert_eq!(b, "Concept B");
    }

    #[test]
    fn releasing_a_pressed_button_after_a_drawing_switch_does_not_click() {
        let ctx = egui::Context::default();
        let mut workbench = Workbench::default();
        let mut text = String::new();
        let (_, action) = frame(&ctx, &mut workbench, 1, &mut text, egui::RawInput::default());
        let pos = action.rect.center();
        let pointer = |pressed| egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos, button: egui::PointerButton::Primary, pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        assert!(!frame(&ctx, &mut workbench, 1, &mut text, pointer(true)).1.clicked());
        assert!(!frame(&ctx, &mut workbench, 2, &mut text, pointer(false)).1.clicked());
        assert!(frame(&ctx, &mut workbench, 2, &mut text, pointer(true)).1.is_pointer_button_down_on());
        assert!(frame(&ctx, &mut workbench, 2, &mut text, pointer(false)).1.clicked());
    }
}
