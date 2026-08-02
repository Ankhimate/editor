pub mod app;
pub mod app_state;
pub mod clipboard;
pub mod commands;
pub mod config;
pub mod doc;
pub mod edit_router;
pub mod fileops;
pub mod meshgen;
pub mod renderer;
pub mod session;
pub mod theme;
pub mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("Ankhimate")
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "Ankhimate",
        options,
        Box::new(|cc| Ok(Box::new(app::AnkhimateApp::new(cc)))),
    )
}
