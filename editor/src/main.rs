pub mod app;
pub mod app_state;
pub mod args;
pub mod autosave;
pub mod config;
pub mod edit_router;
pub mod fileops;
pub mod keymap;
pub mod operators;
mod paste_tests;
pub mod registry;
pub mod renderer;
pub mod session;
pub mod theme;
pub mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();
    // Prefer DX12 over Vulkan on Windows.
    //
    // wgpu 29's Vulkan backend panics while tearing the swapchain down —
    // "Trying to destroy a SwapchainAcquireSemaphore that is still in use by a
    // SurfaceTexture" — so closing the editor aborts instead of exiting. It is a
    // shutdown-only fault, but a crash on exit is a crash: autosave and config
    // writes never run, and the user is told the program failed after doing
    // nothing wrong. Overlay layers that inject into Vulkan (Steam's, among
    // others) make it more likely.
    //
    // DX12 is the better default on Windows regardless. Vulkan is still
    // reachable with `WGPU_BACKEND=vulkan` for anyone who needs it.
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or({
            if cfg!(target_os = "windows") {
                // DX12 alone, not DX12-preferred: with both enabled wgpu is free
                // to pick Vulkan anyway, and then the shutdown panic is back.
                wgpu::Backends::DX12
            } else {
                wgpu::Backends::PRIMARY
            }
        });
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 720.0])
        .with_title("Ankhimate")
        .with_decorations(false)
        .with_transparent(true);
    // The taskbar and Alt-Tab entry. Without this the window carries whatever
    // stock icon the platform hands an unbranded binary, which is the one place
    // a user picks the editor out of a row of other windows.
    if let Some(icon) = ui::branding::window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        wgpu_options,
        viewport,
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
