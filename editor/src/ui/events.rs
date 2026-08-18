//! The event pane (T-506).
//!
//! Events are markers on the timeline, and the timeline can show *where* they
//! are but not what they carry — a strip a few pixels tall has no room for a
//! name, three payload fields and a sound. This is the table.
//!
//! **List above, one form below.** Every event as an expandable card put the
//! fields at a different height for every event and made comparing two of them
//! impossible; with one form the field you want is always in the same place. It
//! also means the payload rows can share a label column, so the values line up
//! instead of stepping in and out with the width of each label.
//!
//! Editing here writes through the same commands the timeline strip does, so an
//! event dragged on the strip and one retimed in this table land on the same
//! undo stack in the same shape.

use crate::app_state::AppState;
use ankhimate_core::animation::EventKey;
use ankhimate_core::ids::AnimationId;
use ankhimate_document::commands::event_cmds::{AddEvent, DuplicateEvent, EditEvent, EventEdit};
use eframe::egui;

/// Width of the icon-and-label column, so every value field starts at the same x.
const LABEL_W: f32 = 92.0;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState, theme: &crate::theme::Theme) {
    let Some(anim_id) = state.session.active_animation else {
        empty_note(ui, "No animation selected", "");
        return;
    };

    let events: Vec<EventKey> = state
        .doc
        .animations
        .get(anim_id)
        .map(|a| a.events.clone())
        .unwrap_or_default();

    // Events re-sort by time whenever one is retimed, so a stored index can end
    // up past the end or pointing at a different event. Clamp rather than
    // remember: an index into a list that reorders itself is not an identity.
    if state
        .session
        .selected_event
        .is_some_and(|i| i >= events.len())
    {
        state.session.selected_event = events.len().checked_sub(1);
    }

    toolbar(ui, state, anim_id, &events);
    ui.separator();

    if events.is_empty() {
        empty_note(
            ui,
            "No events in this animation",
            "An event fires a callback: a footstep, a hit, a sound",
        );
        return;
    }

    list(ui, state, &events, theme);
    ui.add_space(6.0);
    ui.separator();
    form(ui, state, anim_id, &events, theme);
}

fn toolbar(ui: &mut egui::Ui, state: &mut AppState, anim: AnimationId, events: &[EventKey]) {
    ui.horizontal(|ui| {
        if ui
            .button(format!("{} New", crate::ui::icons::ADD))
            .on_hover_text("Add an event at the playhead")
            .clicked()
        {
            let time = state.session.playhead;
            let name = format!("event{}", events.len() + 1);
            if state.dispatch(Box::new(AddEvent::new(anim, name, time))) {
                // Select what was just made, and find it by time: the list keeps
                // itself sorted, so the new event is not necessarily last.
                state.session.selected_event = state
                    .doc
                    .animations
                    .get(anim)
                    .and_then(|a| a.events.iter().position(|e| e.time >= time));
            }
        }
        let selected = state.session.selected_event;
        if ui
            .add_enabled(
                selected.is_some(),
                egui::Button::new(crate::ui::icons::DUPLICATE),
            )
            .on_hover_text("Duplicate")
            .clicked()
            && let Some(index) = selected
        {
            state.dispatch(Box::new(DuplicateEvent::new(anim, index)));
        }
        if ui
            .add_enabled(
                selected.is_some(),
                egui::Button::new(
                    egui::RichText::new(crate::ui::icons::DELETE)
                        .color(ui.visuals().error_fg_color),
                ),
            )
            .on_hover_text("Delete")
            .clicked()
            && let Some(index) = selected
        {
            state.dispatch(Box::new(EditEvent::new(anim, index, EventEdit::Remove)));
            state.session.selected_event = None;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{} events", events.len()))
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
}

/// The event list: time, name, and a hint of the payload.
fn list(ui: &mut egui::Ui, state: &mut AppState, events: &[EventKey], theme: &crate::theme::Theme) {
    egui::ScrollArea::vertical()
        .id_salt("event_list")
        .max_height(160.0)
        .show(ui, |ui| {
            for (index, event) in events.iter().enumerate() {
                let selected = state.session.selected_event == Some(index);
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 21.0),
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
                    state.session.selected_event = Some(index);
                    // Jumping the playhead is what makes the list a way of
                    // *finding* an event rather than only of editing one.
                    state.session.playhead = event.time;
                }
                let text_color = if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().text_color()
                };
                ui.painter().text(
                    egui::pos2(rect.min.x + 8.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    if event.audio.is_empty() {
                        crate::ui::icons::EVENTS
                    } else {
                        crate::ui::icons::AUDIO
                    },
                    egui::FontId::proportional(11.0),
                    // The same colour the timeline lane marks events in.
                    theme.event_marker(),
                );
                // Time in a fixed column so the names line up under each other.
                ui.painter().text(
                    egui::pos2(rect.min.x + 26.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{:.3}s", event.time),
                    egui::FontId::monospace(10.5),
                    ui.visuals().weak_text_color(),
                );
                ui.painter().text(
                    egui::pos2(rect.min.x + 84.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &event.name,
                    egui::FontId::proportional(12.5),
                    text_color,
                );
                let hint = payload_hint(event);
                if !hint.is_empty() {
                    ui.painter().text(
                        egui::pos2(rect.max.x - 6.0, rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        hint,
                        egui::FontId::proportional(10.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            }
        });
}

/// One line summarising what an event carries, for the list.
///
/// Only the fields that are actually set: a row reading `0 · 0.00 · ""` for
/// every event is noise, and noise on every row hides the one event that does
/// carry something.
fn payload_hint(event: &EventKey) -> String {
    let mut parts = Vec::new();
    if event.int_value != 0 {
        parts.push(event.int_value.to_string());
    }
    if event.float_value != 0.0 {
        parts.push(format!("{:.2}", event.float_value));
    }
    if !event.string_value.is_empty() {
        parts.push(event.string_value.clone());
    }
    if !event.audio.is_empty() {
        parts.push(event.audio.clone());
    }
    parts.join(" · ")
}

/// The properties of the selected event.
fn form(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    events: &[EventKey],
    theme: &crate::theme::Theme,
) {
    let Some(index) = state.session.selected_event else {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Select an event to edit it")
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    };
    let Some(event) = events.get(index).cloned() else {
        return;
    };

    let mut edit: Option<EventEdit> = None;

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(crate::ui::icons::EVENTS)
                .size(13.0)
                .color(theme.event_marker()),
        );
        ui.label(egui::RichText::new("Event").strong().size(12.0));
        let mut name = event.name.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY))
            .changed()
        {
            edit = Some(EventEdit::Rename(name));
        }
    });
    ui.add_space(4.0);

    // Time, with a shortcut for the commonest retime there is.
    let retime = row(ui, crate::ui::icons::TIME, "Time", |ui| {
        let mut time = event.time;
        let mut changed = ui
            .add(
                egui::DragValue::new(&mut time)
                    .speed(0.01)
                    .range(0.0..=600.0)
                    .suffix(" s"),
            )
            .changed();
        if ui
            .button("At playhead")
            .on_hover_text("Move this event to where the playhead is")
            .clicked()
        {
            time = state.session.playhead;
            changed = true;
        }
        changed.then_some(EventEdit::SetTime(time))
    });
    if let Some(retime) = retime {
        edit = Some(retime);
    }

    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("PAYLOAD")
            .size(9.5)
            .color(ui.visuals().weak_text_color()),
    );

    // The payload is one command, not four: the fields are set together, and a
    // command per keystroke would bury the undo stack.
    let mut payload = event.clone();
    let mut payload_changed = false;

    payload_changed |= row(ui, crate::ui::icons::INTEGER, "Integer", |ui| {
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::DragValue::new(&mut payload.int_value),
        )
        .changed()
        .then_some(())
    })
    .is_some();

    payload_changed |= row(ui, crate::ui::icons::FLOAT, "Float", |ui| {
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::DragValue::new(&mut payload.float_value).speed(0.01),
        )
        .changed()
        .then_some(())
    })
    .is_some();

    payload_changed |= row(ui, crate::ui::icons::STRING, "String", |ui| {
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::TextEdit::singleline(&mut payload.string_value),
        )
        .changed()
        .then_some(())
    })
    .is_some();

    payload_changed |= row(ui, crate::ui::icons::AUDIO, "Audio", |ui| {
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::TextEdit::singleline(&mut payload.audio).hint_text("asset name"),
        )
        .changed()
        .then_some(())
    })
    .is_some();

    // Volume and balance only mean anything with a sound attached, so they only
    // appear with one. Two disabled sliders on every event is furniture.
    if !payload.audio.is_empty() {
        payload_changed |= row(ui, crate::ui::icons::AUDIO, "Volume", |ui| {
            ui.add_sized(
                [ui.available_width(), 20.0],
                egui::Slider::new(&mut payload.volume, 0.0..=2.0).fixed_decimals(2),
            )
            .changed()
            .then_some(())
        })
        .is_some();

        payload_changed |= row(
            ui,
            crate::ui::icons::TRANSFORM_CONSTRAINT,
            "Balance",
            |ui| {
                ui.add_sized(
                    [ui.available_width(), 20.0],
                    egui::Slider::new(&mut payload.balance, -1.0..=1.0)
                        .fixed_decimals(2)
                        .text(""),
                )
                .on_hover_text("-1 hard left · 0 centre · 1 hard right")
                .changed()
                .then_some(())
            },
        )
        .is_some();
    }

    if payload_changed {
        edit = Some(EventEdit::SetPayload {
            int_value: payload.int_value,
            float_value: payload.float_value,
            string_value: payload.string_value,
            audio: payload.audio,
            volume: payload.volume,
            balance: payload.balance,
        });
    }

    if let Some(edit) = edit {
        state.dispatch(Box::new(EditEvent::new(anim, index, edit)));
    }
}

/// A labelled property row: icon, label, then the field, all on one grid.
fn row<R>(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    contents: impl FnOnce(&mut egui::Ui) -> Option<R>,
) -> Option<R> {
    let mut out = None;
    ui.horizontal(|ui| {
        // Fixed-width label cell — without it every value field starts at a
        // different x and the column reads as ragged.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(LABEL_W, 20.0), egui::Sense::hover());
        ui.painter().text(
            egui::pos2(rect.min.x + 2.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
        ui.painter().text(
            egui::pos2(rect.min.x + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(11.5),
            ui.visuals().text_color(),
        );
        out = contents(ui);
    });
    out
}

fn empty_note(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        if !hint.is_empty() {
            ui.label(
                egui::RichText::new(hint)
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, int_value: i32, audio: &str) -> EventKey {
        EventKey {
            time: 0.5,
            name: name.into(),
            int_value,
            float_value: 0.0,
            string_value: String::new(),
            audio: audio.into(),
            volume: 1.0,
            balance: 0.0,
        }
    }

    /// An empty payload contributes nothing. A hint reading `0 · 0.00 · ""` on
    /// every row hides the one event that actually carries something.
    #[test]
    fn an_empty_payload_has_no_hint() {
        assert_eq!(payload_hint(&event("step", 0, "")), "");
    }

    #[test]
    fn the_hint_lists_only_what_is_set() {
        assert_eq!(payload_hint(&event("step", 3, "")), "3");
        assert_eq!(payload_hint(&event("step", 0, "gravel")), "gravel");
        assert_eq!(payload_hint(&event("step", 3, "gravel")), "3 · gravel");
    }
}
