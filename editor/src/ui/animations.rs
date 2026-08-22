//! The animation manager (T-208).
//!
//! The clip list has lived inside the timeline's header, which works while a rig
//! has three clips and stops working at eleven: a dropdown is a poor list, and
//! nothing else in the editor shows what a clip *contains* without switching to
//! it first.
//!
//! This is the list, with the per-clip properties beside it, so picking a clip
//! and editing its length are the same gesture.

use crate::app_state::AppState;
use ankhimate_core::ids::AnimationId;
use ankhimate_document::commands::key_cmds::{
    CreateAnimation, DeleteAnimation, DuplicateAnimation, RenameAnimation, SetAnimationMeta,
};
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState, theme: &crate::theme::Theme) {
    ui.columns(2, |columns| {
        let button_width = columns[0].available_width();
        if columns[0]
            .add_sized(
                [button_width, crate::ui::CONTROL_HEIGHT],
                egui::Button::new(format!("{}  New", crate::ui::icons::ADD)),
            )
            .clicked()
        {
            let name = format!("animation{}", state.doc.animations.len() + 1);
            if state.dispatch(Box::new(CreateAnimation::new(name, 1.0))) {
                // Select what was just made: creating a clip and then having to
                // find it in the list is two steps for one intention.
                if let Some(id) = state.doc.animations.keys().last() {
                    state.session.active_animation = Some(id);
                }
            }
        }
        let active = state.session.active_animation;
        if columns[1]
            .add_enabled(
                active.is_some(),
                egui::Button::new(format!("{}  Duplicate", crate::ui::icons::DUPLICATE))
                    .min_size(egui::vec2(button_width, crate::ui::CONTROL_HEIGHT)),
            )
            .clicked()
            && let Some(id) = active
        {
            state.dispatch(Box::new(DuplicateAnimation::new(id)));
        }
    });
    ui.separator();

    let mut clips: Vec<(AnimationId, String, f32, bool, usize, usize)> = state
        .doc
        .animations
        .iter()
        .map(|(id, a)| {
            (
                id,
                a.name.clone(),
                a.duration,
                a.looping,
                a.timelines.len(),
                a.events.len(),
            )
        })
        .collect();
    // Alphabetical: slotmap order is insertion order, which is meaningless to
    // anyone looking for `walk`.
    clips.sort_by_key(|c| c.1.to_lowercase());

    if clips.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No animations yet")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    }

    let mut rename: Option<(AnimationId, String)> = None;
    let mut delete: Option<AnimationId> = None;

    egui::ScrollArea::vertical()
        .id_salt("animation_list")
        .max_height(220.0)
        .show(ui, |ui| {
            for (id, name, duration, looping, tracks, events) in &clips {
                let selected = state.session.active_animation == Some(*id);
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 22.0),
                    egui::Sense::click(),
                );
                if selected {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        ui.visuals().selection.bg_fill.linear_multiply(0.3),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                }
                if response.clicked() {
                    state.session.active_animation = Some(*id);
                }
                let text_color = if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().text_color()
                };
                ui.painter().text(
                    egui::pos2(rect.min.x + 8.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    if *looping {
                        crate::ui::icons::LOOP
                    } else {
                        crate::ui::icons::DOPESHEET
                    },
                    egui::FontId::proportional(12.0),
                    text_color.gamma_multiply(0.7),
                );
                ui.painter().text(
                    egui::pos2(rect.min.x + 26.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    name,
                    egui::FontId::proportional(12.5),
                    text_color,
                );
                // Length and contents on the right: which clip is the long one,
                // and which is the empty one somebody made by accident.
                ui.painter().text(
                    egui::pos2(rect.max.x - 8.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{duration:.2}s · {tracks} tracks · {events} events"),
                    egui::FontId::proportional(10.0),
                    ui.visuals().weak_text_color(),
                );

                response.context_menu(|ui| {
                    if crate::ui::action_button(ui, crate::ui::icons::EDIT, "Rename…").clicked() {
                        rename = Some((*id, name.clone()));
                        ui.close();
                    }
                    if crate::ui::action_button(ui, crate::ui::icons::DUPLICATE, "Duplicate")
                        .clicked()
                    {
                        state.dispatch(Box::new(DuplicateAnimation::new(*id)));
                        ui.close();
                    }
                    ui.separator();
                    if crate::ui::action_button(ui, crate::ui::icons::DELETE, "Delete").clicked() {
                        delete = Some(*id);
                        ui.close();
                    }
                });
            }
        });

    if let Some(id) = delete {
        state.dispatch(Box::new(DeleteAnimation::new(id)));
        if state.session.active_animation == Some(id) {
            state.session.active_animation = state.doc.animations.keys().next();
        }
    }
    if let Some((id, name)) = rename {
        ui.data_mut(|d| d.insert_temp(egui::Id::new("anim_rename"), (id, name)));
    }
    rename_popup(ui, state, theme);

    // ── Properties of the selected clip ────────────────────────────────
    let Some(anim_id) = state.session.active_animation else {
        return;
    };
    let Some(anim) = state.doc.animations.get(anim_id) else {
        return;
    };
    let (mut duration, mut looping) = (anim.duration, anim.looping);
    let content = anim.content_duration();

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Selected clip").strong().size(11.0));
    ui.add_space(4.0);

    let mut changed = false;
    ui.horizontal(|ui| {
        crate::ui::form_label(ui, "Duration");
        changed |= ui
            .add(
                egui::DragValue::new(&mut duration)
                    .speed(0.05)
                    .range(0.05..=600.0)
                    .suffix(" s"),
            )
            .changed();
        // Trimming below the last key hides work rather than deleting it, so the
        // number that says where the keys actually end has to be visible.
        if content > duration + 1e-4 {
            ui.label(
                egui::RichText::new(format!("keys reach {content:.2}s"))
                    .size(10.0)
                    .color(ui.visuals().warn_fg_color),
            )
            .on_hover_text("Shortening the clip does not delete keys past the end");
        }
    });
    ui.horizontal(|ui| {
        crate::ui::form_label(ui, "Playback");
        changed |= ui.checkbox(&mut looping, "Loop animation").changed();
    });
    if changed {
        state.dispatch(Box::new(SetAnimationMeta::new(anim_id, duration, looping)));
    }
}

/// A modal rename, because renaming in place inside a scroll area fights the
/// row's click handling.
fn rename_popup(ui: &mut egui::Ui, state: &mut AppState, theme: &crate::theme::Theme) {
    let id = egui::Id::new("anim_rename");
    let Some((anim, mut name)) = ui.data(|d| d.get_temp::<(AnimationId, String)>(id)) else {
        return;
    };
    let mut close = false;
    let mut commit = false;
    let dialog = crate::ui::dialog::Dialog::new("rename_animation", "Rename animation")
        .icon(crate::ui::icons::ANIMATIONS)
        .width(320.0)
        .show(ui.ctx(), theme, |ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut name).desired_width(220.0));
            response.request_focus();
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::ui::action_button(ui, crate::ui::icons::CLOSE, "Cancel").clicked() {
                    close = true;
                }
                if crate::ui::action_button(ui, crate::ui::icons::EDIT, "Rename").clicked() {
                    commit = true;
                }
            });
        });
    if commit && !name.trim().is_empty() {
        state.dispatch(Box::new(RenameAnimation::new(
            anim,
            name.trim().to_string(),
        )));
        close = true;
    }
    close |= dialog.closed;
    if close {
        ui.data_mut(|d| d.remove::<(AnimationId, String)>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, (anim, name)));
    }
}
