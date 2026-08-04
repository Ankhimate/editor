//! The event pane (T-506).
//!
//! Events are markers on the timeline, and the timeline can show *where* they
//! are but not what they carry — a strip a few pixels tall has no room for a
//! name, three payload fields and a sound. This is the table.
//!
//! Editing here writes through the same commands the timeline strip does, so an
//! event dragged on the strip and one retimed in this table land on the same
//! undo stack in the same shape.

use crate::app_state::AppState;
use crate::commands::event_cmds::{AddEvent, EditEvent, EventEdit};
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(anim_id) = state.session.active_animation else {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No animation selected")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    };

    ui.horizontal(|ui| {
        if ui
            .button(format!("{} Add at playhead", egui_phosphor::fill::PLUS))
            .clicked()
        {
            let time = state.session.playhead;
            let name = format!("event{}", event_count(state, anim_id) + 1);
            state.dispatch(Box::new(AddEvent::new(anim_id, name, time)));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{} events", event_count(state, anim_id)))
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
    ui.separator();

    let events: Vec<(usize, ankhimate_core::animation::EventKey)> = state
        .doc
        .animations
        .get(anim_id)
        .map(|a| a.events.iter().cloned().enumerate().collect())
        .unwrap_or_default();

    if events.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No events in this animation")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new("An event fires a callback: a footstep, a hit, a sound")
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    }

    let mut edit: Option<(usize, EventEdit)> = None;
    let mut remove: Option<usize> = None;

    egui::ScrollArea::vertical()
        .id_salt("event_table")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, event) in &events {
                let id = ui.make_persistent_id(("event_row", index));
                let header = format!("{:.3}s   {}", event.time, event.name);
                egui::CollapsingHeader::new(header)
                    .id_salt(id)
                    .show(ui, |ui| {
                        let mut time = event.time;
                        let mut name = event.name.clone();
                        let mut int_value = event.int_value;
                        let mut float_value = event.float_value;
                        let mut string_value = event.string_value.clone();
                        let mut audio = event.audio.clone();
                        let mut volume = event.volume;
                        let mut balance = event.balance;

                        ui.horizontal(|ui| {
                            ui.label("Time");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut time)
                                        .speed(0.01)
                                        .range(0.0..=600.0)
                                        .suffix(" s"),
                                )
                                .changed()
                            {
                                edit = Some((*index, EventEdit::SetTime(time)));
                            }
                            if ui.button("At playhead").clicked() {
                                edit = Some((*index, EventEdit::SetTime(state.session.playhead)));
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            if ui
                                .add(egui::TextEdit::singleline(&mut name).desired_width(160.0))
                                .changed()
                            {
                                edit = Some((*index, EventEdit::Rename(name.clone())));
                            }
                        });

                        // The payload is one edit, not four: they are almost
                        // always set together, and four commands per keystroke
                        // would bury the undo stack.
                        let mut payload_changed = false;
                        ui.horizontal(|ui| {
                            ui.label("Int");
                            payload_changed |=
                                ui.add(egui::DragValue::new(&mut int_value)).changed();
                            ui.label("Float");
                            payload_changed |= ui
                                .add(egui::DragValue::new(&mut float_value).speed(0.01))
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("String");
                            payload_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut string_value)
                                        .desired_width(160.0),
                                )
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Audio");
                            payload_changed |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut audio)
                                        .hint_text("asset name")
                                        .desired_width(160.0),
                                )
                                .changed();
                        });
                        if !audio.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("Volume");
                                payload_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut volume)
                                            .speed(0.01)
                                            .range(0.0..=4.0),
                                    )
                                    .changed();
                                ui.label("Balance");
                                payload_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut balance)
                                            .speed(0.01)
                                            .range(-1.0..=1.0),
                                    )
                                    .on_hover_text("-1 hard left · 0 centre · 1 hard right")
                                    .changed();
                            });
                        }
                        if payload_changed {
                            edit = Some((
                                *index,
                                EventEdit::SetPayload {
                                    int_value,
                                    float_value,
                                    string_value: string_value.clone(),
                                    audio: audio.clone(),
                                    volume,
                                    balance,
                                },
                            ));
                        }

                        ui.add_space(4.0);
                        if ui
                            .button(
                                egui::RichText::new(format!(
                                    "{} Delete",
                                    egui_phosphor::fill::TRASH
                                ))
                                .color(ui.visuals().error_fg_color),
                            )
                            .clicked()
                        {
                            remove = Some(*index);
                        }
                    });
            }
        });

    // Removal last: deleting inside the loop would shift every index after it
    // while the rows above are still using the old ones.
    if let Some(index) = remove {
        state.dispatch(Box::new(EditEvent::new(anim_id, index, EventEdit::Remove)));
    } else if let Some((index, edit)) = edit {
        state.dispatch(Box::new(EditEvent::new(anim_id, index, edit)));
    }
}

fn event_count(state: &AppState, anim: ankhimate_core::ids::AnimationId) -> usize {
    state
        .doc
        .animations
        .get(anim)
        .map(|a| a.events.len())
        .unwrap_or(0)
}
