//! The startup page (T-304).
//!
//! A whole page, not a window over the editor. It used to be a modal dialog
//! floating on top of a live workspace, which was wrong twice: the editor behind
//! it was fully drawn and half-visible around the edges, so the first thing a new
//! user saw was a busy tool they had not asked for yet; and a dialog implies
//! something to go *back* to, when on launch there is nothing behind it.
//!
//! A page states what is true — you have not opened anything yet, and this is
//! where you choose. The editor appears once there is a project to put in it.
//!
//! The page carries its own ✕, top right, which starts an empty project: "skip
//! this, I will build from scratch". It is deliberately not the window's ✕ in
//! the title bar — that one quits, as it does everywhere else, and a launcher
//! that hijacked it would leave a user who wanted to leave trapped in an editor
//! instead.

use crate::config::Config;
use eframe::egui;
use std::path::PathBuf;

/// What the user picked, resolved by the caller so the file dialog does not run
/// mid-layout.
pub enum StartupChoice {
    None,
    NewProject,
    OpenDialog,
    Open(PathBuf),
}

/// Width the two columns are laid out inside.
///
/// Centred in whatever the window is rather than filling it: a launcher stretched
/// across an ultrawide monitor puts "New project" and the recent list a foot
/// apart, and the eye has to travel the whole way to find out there is nothing
/// in between.
const CONTENT_WIDTH: f32 = 620.0;

/// Draw the page into the central area the caller has already opened.
///
/// Takes a `Ui` rather than a `Context` so the caller owns the panel: the
/// workspace and this page are alternatives for the same central area, and
/// having both construct their own would mean two panels racing for it on the
/// frame the choice changes.
pub fn ui(ui: &mut egui::Ui, config: &mut Config) -> StartupChoice {
    let mut choice = StartupChoice::None;
    let samples = crate::config::sample_projects();
    let mut forget: Option<PathBuf> = None;

    // The page's own dismiss, in its top-right corner — distinct from
    // the window's ✕ in the title bar, which still quits. This one skips
    // the launcher and drops straight into an empty project, which is
    // what "close the thing asking me to choose" has to mean when there
    // is nothing behind it to go back to.
    let full = ui.max_rect();
    let close = egui::Rect::from_min_size(
        egui::pos2(full.max.x - 30.0, full.min.y),
        egui::vec2(30.0, 26.0),
    );
    let response = ui.allocate_rect(close, egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(close, 5.0, ui.visuals().faint_bg_color);
    }
    ui.painter().text(
        close.center(),
        egui::Align2::CENTER_CENTER,
        crate::ui::icons::CLOSE,
        egui::FontId::proportional(14.0),
        ui.visuals().weak_text_color(),
    );
    if response
        .on_hover_text("Skip this and start an empty project")
        .clicked()
    {
        choice = StartupChoice::NewProject;
    }

    // Vertically centred, so the page reads as a considered screen rather
    // than as content that ran out at the top of an empty window.
    let available = ui.available_height();
    ui.add_space(((available - 420.0) * 0.35).max(0.0));

    ui.vertical_centered(|ui| {
        ui.set_max_width(CONTENT_WIDTH);

        // ── Masthead ─────────────────────────────────────────────
        ui.heading(egui::RichText::new("Ankhimate").size(30.0));
        ui.label(
            egui::RichText::new(format!(
                "2D skeletal animation · {}",
                env!("CARGO_PKG_VERSION")
            ))
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(22.0);

        ui.horizontal_top(|ui| {
            // ── Actions ──────────────────────────────────────────
            ui.vertical(|ui| {
                ui.set_min_width(200.0);
                if ui
                    .add_sized([190.0, 34.0], egui::Button::new("New project"))
                    .clicked()
                {
                    choice = StartupChoice::NewProject;
                }
                ui.add_space(6.0);
                if ui
                    .add_sized([190.0, 34.0], egui::Button::new("Open…"))
                    .clicked()
                {
                    choice = StartupChoice::OpenDialog;
                }

                if !samples.is_empty() {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("Samples")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(2.0);
                    for path in &samples {
                        let name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("sample");
                        if ui
                            .add_sized([190.0, 26.0], egui::Button::new(name))
                            .clicked()
                        {
                            choice = StartupChoice::Open(path.clone());
                        }
                    }
                }
            });

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(20.0);

            // ── Recent files ─────────────────────────────────────
            ui.vertical(|ui| {
                ui.set_min_width(330.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Recent").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !config.recent_files.is_empty() && ui.small_button("Clear").clicked() {
                            config.clear_recent();
                        }
                    });
                });
                ui.separator();

                if config.recent_files.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Nothing yet — open a project or try a sample.")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for path in config.recent_files.clone() {
                            // A file that has moved is shown greyed rather
                            // than hidden: knowing it is gone beats
                            // wondering where it went.
                            let exists = path.exists();
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("project");
                            ui.horizontal(|ui| {
                                let button =
                                    egui::Button::new(name).min_size(egui::vec2(210.0, 24.0));
                                let response = ui.add_enabled(exists, button);
                                let response = response.on_hover_text(path.display().to_string());
                                if response.clicked() {
                                    choice = StartupChoice::Open(path.clone());
                                }
                                if !exists {
                                    ui.label(
                                        egui::RichText::new("missing")
                                            .small()
                                            .color(egui::Color32::from_rgb(230, 90, 90)),
                                    );
                                    if ui.small_button("Forget").clicked() {
                                        forget = Some(path.clone());
                                    }
                                }
                            });
                        }
                    });
            });
        });

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(6.0);

        // The skip preference, and nothing else — dismissing is the ✕ in
        // the page's own corner, above.
        ui.horizontal(|ui| {
            let mut skip = config.skip_startup;
            if ui
                .checkbox(&mut skip, "Skip this page next time")
                .on_hover_text(
                    "Launch straight into an empty project. \
                             This page is still under File › New.",
                )
                .changed()
            {
                config.skip_startup = skip;
                config.save();
            }
        });
    });

    if let Some(path) = forget {
        config.forget_recent(&path);
    }
    choice
}
