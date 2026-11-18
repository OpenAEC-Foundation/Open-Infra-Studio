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
    Picked([f64; 2]),
    /// The import area was set in the drawing; reload the site.
    SiteChanged,
    /// A KLIC delivery chosen in Open CAD Studio's file dialog.
    Klic(std::path::PathBuf),
    /// A GIS file chosen in Open CAD Studio's file dialog.
    File(std::path::PathBuf),
    /// Run the page's action (validation, quantities) right away.
    Run(Tab),
}

struct Handle {
    tx: Option<Sender<UiMsg>>,
    ctx: Option<egui::Context>,
}

static HANDLE: Mutex<Handle> = Mutex::new(Handle { tx: None, ctx: None });

pub fn send(msg: UiMsg) -> bool {
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
    let mut handle = HANDLE.lock().unwrap();
    if let Some(tx) = &handle.tx {
        if tx.send(UiMsg::Show(tab)).is_ok() {
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
    tx.send(UiMsg::Show(tab)).expect("new workbench channel");
    allow_debugger();
    std::thread::Builder::new()
        .name("open-infra-ui".into())
        .spawn(move || run_workbench(rx))
        .map_err(|e| e.to_string())?;
    handle.tx = Some(tx);
    Ok(())
}

fn run_workbench(rx: Receiver<UiMsg>) {
    let mut app = app::App::new();
    while let Ok(msg) = rx.recv() {
        if matches!(msg, UiMsg::SiteChanged) {
            // A change in the CAD drawing should not open a closed window.
            app.reload_site();
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
                let _ = builder;
            })),
            ..Default::default()
        };
        let result = eframe::run_native(
            "Open Infra GIS",
            options,
            Box::new(|cc| {
                app.configure_window(&cc.egui_ctx);
                HANDLE.lock().unwrap().ctx = Some(cc.egui_ctx.clone());
                Ok(Box::new(Window { app: &mut app, rx: &rx, first: Some(msg) }))
            }),
        );
        HANDLE.lock().unwrap().ctx = None;
        if let Err(e) = result {
            crate::bridge::error(&format!("GIS-venster kon niet starten: {e}"));
        }
    }
}

struct Window<'a> {
    app: &'a mut app::App,
    rx: &'a Receiver<UiMsg>,
    first: Option<UiMsg>,
}

impl eframe::App for Window<'_> {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Let eframe destroy the native window. Messages arriving during the
        // close remain queued for the next window.
        if ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        for msg in self.first.take().into_iter().chain(self.rx.try_iter()) {
            self.app.handle_message(ctx, msg);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
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
