//! The startup window (T-304).
//!
//! Shown over an empty editor on launch: new, open, recent, samples. An empty
//! canvas with no obvious first move is the worst first impression a tool can
//! make, and "which file was I working on" is the most common thing a user wants
//! from a launcher.

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
    /// Dismiss without choosing — the editor is usable behind this window.
    Dismiss,
}

pub fn ui(ctx: &egui::Context, config: &mut Config, theme: &crate::theme::Theme) -> StartupChoice {
    let mut choice = StartupChoice::None;
    let samples = crate::config::sample_projects();
    let mut forget: Option<PathBuf> = None;

    // Not dismissable: there is nothing behind it yet, so closing it would
    // leave the user staring at an empty editor with no way back.
    let _ = crate::ui::dialog::Dialog::new("startup", "Ankhimate")
        .icon(crate::ui::icons::VIEWPORT)
        .width(560.0)
        .dismissable(false)
        .show(ctx, theme, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Ankhimate");
                ui.label(
                    egui::RichText::new(env!("CARGO_PKG_VERSION"))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
            ui.label(
                egui::RichText::new("2D skeletal animation")
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(10.0);

            ui.horizontal_top(|ui| {
                // ── Actions ──────────────────────────────────────────────
                ui.vertical(|ui| {
                    ui.set_min_width(170.0);
                    if ui
                        .add_sized([160.0, 30.0], egui::Button::new("New project"))
                        .clicked()
                    {
                        choice = StartupChoice::NewProject;
                    }
                    ui.add_space(4.0);
                    if ui
                        .add_sized([160.0, 30.0], egui::Button::new("Open…"))
                        .clicked()
                    {
                        choice = StartupChoice::OpenDialog;
                    }

                    if !samples.is_empty() {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("Samples")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        for path in &samples {
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("sample");
                            if ui
                                .add_sized([160.0, 24.0], egui::Button::new(name))
                                .clicked()
                            {
                                choice = StartupChoice::Open(path.clone());
                            }
                        }
                    }
                });

                ui.separator();

                // ── Recent files ─────────────────────────────────────────
                ui.vertical(|ui| {
                    ui.set_min_width(330.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Recent").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !config.recent_files.is_empty() && ui.small_button("Clear").clicked()
                            {
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
                        .max_height(240.0)
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
                                        egui::Button::new(name).min_size(egui::vec2(200.0, 22.0));
                                    let response = ui.add_enabled(exists, button);
                                    let response =
                                        response.on_hover_text(path.display().to_string());
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

            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                let mut skip = config.skip_startup;
                if ui
                    .checkbox(&mut skip, "Skip this window next time")
                    .changed()
                {
                    config.skip_startup = skip;
                    config.save();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        choice = StartupChoice::Dismiss;
                    }
                });
            });
        });

    if let Some(path) = forget {
        config.forget_recent(&path);
    }
    choice
}
