//! Bulk rename (T-901).
//!
//! Renaming forty bones one at a time is the kind of tedium that stops people
//! naming things properly at all, and a rig whose bones are `bone`, `bone_2`,
//! `bone_3` is one nobody else can pick up. Spine has had this filed as issue
//! #330 with no implementation.
//!
//! Two modes, because the two things people actually want are different shapes:
//!
//! * **Number** — a pattern with `{n}` in it, applied down the selection.
//!   `tail_{n}` over eight bones gives `tail_1` … `tail_8`.
//! * **Replace** — swap a substring across the selection, for the
//!   `left`→`right` mirror pass and for fixing a typo propagated by a duplicate.
//!
//! # `{n}` counts in *selection* order
//!
//! Not tree order. Clicking down a tail and getting 1..N in the order you
//! clicked is the whole point; getting them in whatever order the hierarchy
//! happens to hold is a different feature nobody asked for.
//!
//! # The preview is the safety
//!
//! A batch rename is hard to eyeball afterwards — forty rows of near-identical
//! names, and the mistake is that two of them collided. So the dialog shows
//! every `old → new` before anything is applied, flags names that would clash,
//! and the apply button says how many rows it is about to write.

use crate::app_state::AppState;
use crate::commands::bone_cmds::RenameBones;
use ankhimate_core::ids::BoneId;
use eframe::egui;

/// Which transformation the dialog is applying.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Number,
    Replace,
}

/// The dialog's own state, kept across frames while it is open.
pub struct RenameState {
    pub mode: Mode,
    /// Pattern for [`Mode::Number`]. `{n}` is the index.
    pub pattern: String,
    pub start: i32,
    pub step: i32,
    /// Zero-pad width; 1 means no padding.
    pub pad: usize,
    /// Needle and replacement for [`Mode::Replace`].
    pub find: String,
    pub replace: String,
}

impl Default for RenameState {
    fn default() -> Self {
        Self {
            mode: Mode::Number,
            pattern: "bone_{n}".to_string(),
            start: 1,
            step: 1,
            pad: 1,
            find: String::new(),
            replace: String::new(),
        }
    }
}

/// What the dialog would do, as `(bone, old, new)`.
///
/// Computed every frame from the current settings rather than cached: the
/// preview and the applied result must be the same function of the same inputs,
/// and two code paths is how they drift apart.
fn plan(state: &AppState, settings: &RenameState) -> Vec<(BoneId, String, String)> {
    let mut out = Vec::new();
    for (index, &bone) in state.session.selected_bones.iter().enumerate() {
        let Some(current) = state.doc.skeleton.bones.get(bone).map(|b| b.name.clone()) else {
            continue;
        };
        let new = match settings.mode {
            Mode::Number => {
                let n = settings.start + settings.step * index as i32;
                let number = if settings.pad > 1 {
                    format!("{:0width$}", n, width = settings.pad)
                } else {
                    n.to_string()
                };
                // No `{n}` in the pattern would give every bone the same name,
                // which core would then suffix into `x`, `x_2`, `x_3` — a
                // numbering nobody chose. Appending is the readable rescue.
                if settings.pattern.contains("{n}") {
                    settings.pattern.replace("{n}", &number)
                } else {
                    format!("{}{}", settings.pattern, number)
                }
            }
            Mode::Replace => {
                if settings.find.is_empty() {
                    current.clone()
                } else {
                    current.replace(&settings.find, &settings.replace)
                }
            }
        };
        out.push((bone, current, new));
    }
    out
}

/// Draw the dialog. Returns `true` when it should close.
pub fn ui(
    ctx: &egui::Context,
    state: &mut AppState,
    settings: &mut RenameState,
    theme: &crate::theme::Theme,
) -> bool {
    let rows = plan(state, settings);
    let mut apply = false;

    // Names that more than one row wants, plus names already held by a bone
    // outside the selection. Both end in core's uniquifier adding a suffix, and
    // a suffix nobody asked for is exactly what this dialog exists to avoid — so
    // they are shown before the fact rather than discovered after.
    let mut clashes: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (_, _, new) in &rows {
            if !seen.insert(new.as_str()) {
                clashes.insert(new.clone());
            }
        }
        let selected: std::collections::HashSet<BoneId> =
            state.session.selected_bones.iter().copied().collect();
        for (id, bone) in state.doc.skeleton.bones.iter() {
            if !selected.contains(&id) && rows.iter().any(|(_, _, new)| *new == bone.name) {
                clashes.insert(bone.name.clone());
            }
        }
    }

    let response = crate::ui::dialog::Dialog::new("bulk_rename", "Rename bones")
        .icon(crate::ui::icons::BONE)
        .width(440.0)
        .show(ctx, theme, |ui| {
            if rows.is_empty() {
                ui.label(
                    egui::RichText::new("Select the bones to rename first.")
                        .color(ui.visuals().weak_text_color()),
                );
                return;
            }

            ui.horizontal(|ui| {
                ui.selectable_value(&mut settings.mode, Mode::Number, "Number");
                ui.selectable_value(&mut settings.mode, Mode::Replace, "Replace");
            });
            ui.add_space(8.0);

            match settings.mode {
                Mode::Number => {
                    ui.horizontal(|ui| {
                        ui.label("Pattern");
                        ui.add(
                            egui::TextEdit::singleline(&mut settings.pattern)
                                .desired_width(200.0)
                                .hint_text("tail_{n}"),
                        )
                        .on_hover_text("{n} is replaced by the number");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Start");
                        ui.add(egui::DragValue::new(&mut settings.start).speed(1.0));
                        ui.add_space(8.0);
                        ui.label("Step");
                        ui.add(egui::DragValue::new(&mut settings.step).speed(1.0));
                        ui.add_space(8.0);
                        ui.label("Pad");
                        ui.add(egui::DragValue::new(&mut settings.pad).range(1..=6))
                            .on_hover_text("Zero-pad to this width: 3 gives 001, 002");
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Numbered in the order you selected them.")
                            .size(10.5)
                            .color(ui.visuals().weak_text_color()),
                    );
                }
                Mode::Replace => {
                    ui.horizontal(|ui| {
                        ui.label("Find");
                        ui.add(egui::TextEdit::singleline(&mut settings.find).desired_width(150.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Replace");
                        ui.add(
                            egui::TextEdit::singleline(&mut settings.replace).desired_width(150.0),
                        );
                    });
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Preview").strong());
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (_, old, new) in &rows {
                        let clashing = clashes.contains(new);
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [150.0, 16.0],
                                egui::Label::new(
                                    egui::RichText::new(old)
                                        .size(11.0)
                                        .color(ui.visuals().weak_text_color()),
                                ),
                            );
                            ui.label(
                                egui::RichText::new("→")
                                    .size(11.0)
                                    .color(ui.visuals().weak_text_color()),
                            );
                            let text = egui::RichText::new(new).size(11.0);
                            ui.label(if clashing {
                                text.color(ui.visuals().warn_fg_color)
                            } else {
                                text
                            });
                            if clashing {
                                ui.label(
                                    egui::RichText::new("already taken — will be suffixed")
                                        .size(10.0)
                                        .color(ui.visuals().warn_fg_color),
                                );
                            }
                        });
                    }
                });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let unchanged = rows.iter().all(|(_, old, new)| old == new);
                if ui
                    .add_enabled(
                        !unchanged,
                        egui::Button::new(format!("Rename {} bone(s)", rows.len())),
                    )
                    .clicked()
                {
                    apply = true;
                }
                if !clashes.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("{} name(s) already taken", clashes.len()))
                            .size(10.5)
                            .color(ui.visuals().warn_fg_color),
                    );
                }
            });
        });

    if apply {
        let renames: Vec<(BoneId, String)> = rows
            .into_iter()
            .filter(|(_, old, new)| old != new)
            .map(|(id, _, new)| (id, new))
            .collect();
        if !renames.is_empty() {
            state.dispatch(Box::new(RenameBones::new(renames)));
        }
        return true;
    }
    response.closed
}
