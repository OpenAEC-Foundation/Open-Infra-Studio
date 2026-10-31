//! The GIS workbench window. It runs on its own thread in the plugin process
//! (Open CAD Studio has no plugin dialog API), next to the CAD window, and
//! writes into the drawing through [`crate::bridge`].
//!
//! winit allows one event loop per process, so the window is created once and
//! afterwards only hidden/shown; closing it hides it.

mod app;
mod files;
mod jobs;
mod library;
mod infra;
mod map;
mod planning;

use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

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
    tx: Sender<UiMsg>,
}

fn handle() -> &'static Mutex<Option<Handle>> {
    static H: OnceLock<Mutex<Option<Handle>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(None))
}

static CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn send(msg: UiMsg) -> bool {
    let guard = handle().lock().unwrap();
    let Some(h) = guard.as_ref() else { return false };
    let ok = h.tx.send(msg).is_ok();
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
    ok
}

/// Show the workbench on `tab`, creating the window on first use.
pub fn open(tab: Tab) -> Result<(), String> {
    if send(UiMsg::Show(tab)) {
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        return Err("het GIS-venster is op macOS nog niet beschikbaar; gebruik de GIS_-commando's".into());
    }
    let (tx, rx) = channel();
    tx.send(UiMsg::Show(tab)).ok();
    *handle().lock().unwrap() = Some(Handle { tx });
    std::thread::Builder::new()
        .name("open-infra-ui".into())
        .spawn(move || {
            let mut options = eframe::NativeOptions {
                viewport: egui::ViewportBuilder::default()
                    .with_title(if product() == Product::Civiel { "Open Infra — Civiel" } else { "Open Infra — GIS-workbench" })
                    .with_app_id(if product() == Product::Civiel { "open-infra-civiel" } else { "open-infra-gis" })
                    .with_inner_size([1280.0, 1000.0])
                    .with_min_inner_size([760.0, 560.0])
                    .with_icon(std::sync::Arc::new(app_icon())),
                ..Default::default()
            };
            options.event_loop_builder = Some(Box::new(|builder| {
                // The plugin's main thread serves the host IPC; the window
                // lives on this one.
                #[cfg(all(unix, not(target_os = "macos")))]
                winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(builder, true);
                #[cfg(windows)]
                winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(builder, true);
                let _ = builder;
            }));
            let result = eframe::run_native(
                "Open Infra GIS",
                options,
                Box::new(move |cc| {
                    let _ = CTX.set(cc.egui_ctx.clone());
                    Ok(Box::new(app::App::new(cc, rx)))
                }),
            );
            *handle().lock().unwrap() = None;
            if let Err(e) = result {
                crate::bridge::error(&format!("GIS-venster kon niet starten: {e}"));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The window and taskbar icon: a map pin for GIS, a profile for Civiel.
fn app_icon() -> egui::IconData {
    let png: &[u8] = if product() == Product::Civiel { include_bytes!("../../assets/app-icon-civiel.png") } else { include_bytes!("../../assets/app-icon-gis.png") };
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}
