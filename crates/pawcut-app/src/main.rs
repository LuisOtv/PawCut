//! PawCut: editor de vídeo simples, com cara de página Edit do DaVinci Resolve.

mod app;
mod config;
mod engine;
mod keymap;
mod panels;
mod state;
mod workspace;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PawCut")
            .with_app_id("pawcut")
            .with_inner_size([1600.0, 950.0])
            .with_min_inner_size([960.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "PawCut",
        options,
        Box::new(move |cc| Ok(Box::new(app::PawCutApp::new(cc, args)))),
    )
}
