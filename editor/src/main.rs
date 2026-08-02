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
    // A path on the command line opens that project straight away:
    //
    //     cargo run -p ankhimate-editor -- samples/walker.ankh
    //
    // Worth the six lines — "did you reopen the file after regenerating it?" is
    // otherwise an invisible failure that looks exactly like a bug in the rig.
    let open = std::env::args().nth(1).map(std::path::PathBuf::from);
    eframe::run_native(
        "Ankhimate",
        options,
        Box::new(move |cc| Ok(Box::new(app::AnkhimateApp::with_file(cc, open)))),
    )
}
