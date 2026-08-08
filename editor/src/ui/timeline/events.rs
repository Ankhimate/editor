//! The event marker lane (T-506).
//!
//! A thin strip under the ruler holding one marker per event. It is its own lane
//! rather than a dopesheet row because events belong to the *clip*, not to a
//! bone or slot: filing them under a bone would mean picking a bone they have
//! nothing to do with, and the dopesheet's group tree has no place for "the
//! animation itself".
//!
//! Markers drag to retime, right-click to rename or delete, and the lane's empty
//! space adds one at the click.

use super::Layout;
use crate::app_state::AppState;
use crate::commands::event_cmds::{AddEvent, EditEvent, EventEdit};
use eframe::egui;

/// Height of the lane, in points.
pub const LANE_HEIGHT: f32 = 18.0;
/// How close to a marker a click has to land to grab it.
const MARKER_HIT: f32 = 7.0;

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    layout: &Layout,
    rect: egui::Rect,
    style: super::Style<'_>,
) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 0.0, visuals.faint_bg_color.gamma_multiply(0.5));

    let sheet_rect = egui::Rect::from_min_max(egui::pos2(layout.sheet_x0, rect.top()), rect.max);

    // The label sits in the tree column, so the lane reads as a row rather than
    // as an unexplained strip.
    painter.text(
        egui::pos2(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "events",
        egui::FontId::proportional(style.text - 0.5),
        visuals.weak_text_color(),
    );

    let Some(anim_id) = state.session.active_animation else {
        return;
    };
    let Some(anim) = state.doc.animations.get(anim_id) else {
        return;
    };
    let events: Vec<(usize, f32, String)> = anim
        .events
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.time, e.name.clone()))
        .collect();

    let response = ui.interact(
        sheet_rect,
        ui.id().with("event_lane"),
        egui::Sense::click_and_drag(),
    );
    let color = style.theme.event_marker();

    // ── Draw ─────────────────────────────────────────────────────────────
    for (index, time, name) in &events {
        let x = layout.time_to_x(*time);
        if x < sheet_rect.left() - 20.0 || x > sheet_rect.right() + 20.0 {
            continue;
        }
        let dragging = state.session.dragging_event == Some(*index);
        let y = rect.center().y;
        // A pennant rather than a diamond: keys are diamonds everywhere else in
        // this panel, and an event is not a key.
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, y - 6.0),
                egui::pos2(x + 7.0, y - 6.0),
                egui::pos2(x + 4.0, y - 2.0),
                egui::pos2(x + 7.0, y + 2.0),
                egui::pos2(x, y + 2.0),
            ],
            if dragging {
                egui::Color32::WHITE
            } else {
                color
            },
            egui::Stroke::NONE,
        ));
        painter.line_segment(
            [egui::pos2(x, y - 6.0), egui::pos2(x, y + 6.0)],
            egui::Stroke::new(1.0, color),
        );
        // The name only when there is room for it, so a dense clip does not
        // turn into overlapping text.
        let next_x = events
            .iter()
            .filter(|(_, t, _)| *t > *time)
            .map(|(_, t, _)| layout.time_to_x(*t))
            .fold(f32::INFINITY, f32::min);
        if next_x - x > 40.0 {
            painter.text(
                egui::pos2(x + 9.0, y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(9.5),
                visuals.weak_text_color(),
            );
        }
    }

    // ── Interact ─────────────────────────────────────────────────────────
    // Animate-only, like the commands themselves: there is no clip to mark in
    // Setup mode.
    if !state.session.is_animating() {
        return;
    }

    if ui.input(|i| i.pointer.primary_released()) {
        state.session.dragging_event = None;
    }

    if let Some(index) = state.session.dragging_event {
        if let Some(pos) = response.interact_pointer_pos() {
            let time = layout.snap_time(layout.x_to_time(pos.x)).max(0.0);
            state.dispatch(Box::new(EditEvent::new(
                anim_id,
                index,
                EventEdit::SetTime(time),
            )));
        }
        return;
    }

    let nearest = |pos: egui::Pos2| {
        events
            .iter()
            .map(|(i, t, _)| (*i, (layout.time_to_x(*t) - pos.x).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .filter(|(_, d)| *d <= MARKER_HIT)
            .map(|(i, _)| i)
    };

    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(index) = nearest(pos)
    {
        state.session.dragging_event = Some(index);
        return;
    }

    // Double-click on empty lane adds an event there. Single click is left for
    // scrubbing-adjacent gestures, so a stray click does not litter the clip.
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && nearest(pos).is_none()
    {
        let time = layout.snap_time(layout.x_to_time(pos.x)).max(0.0);
        let name = format!("event {}", events.len() + 1);
        state.dispatch(Box::new(AddEvent::new(anim_id, name, time)));
    }

    // Right-click an event: rename or delete.
    //
    // Two things had to change for this to work at all. Which event the popup is
    // about is captured when it opens rather than hit-tested each frame — moving
    // the cursor off the pennant used to make the test fail, and the menu swapped
    // to its "nothing here" branch, which is why reaching for the field replaced
    // it with "Double-click the lane to add an event".
    //
    // And it is a window we open and close ourselves, **not**
    // `Response::context_menu`.
    // egui's context menu closes on any plain click of its host response
    // (`Popup::context_menu`), and the host here is the whole lane with the
    // popup drawn over it — so clicking the name field counted as clicking the
    // lane and dismissed the menu before a character could be typed.
    let popup_id = ui.id().with("event_popup");
    if response.secondary_clicked() {
        let target = response.interact_pointer_pos().and_then(nearest);
        let anchor = response
            .interact_pointer_pos()
            .unwrap_or_else(|| rect.left_bottom());
        ui.ctx()
            .data_mut(|d| d.insert_temp(popup_id, (target, anchor)));
    }

    if let Some((menu_target, anchor)) = ui
        .ctx()
        .data(|d| d.get_temp::<(Option<usize>, egui::Pos2)>(popup_id))
    {
        let mut close = false;
        egui::Window::new("event_popup")
            .title_bar(false)
            .resizable(false)
            .fixed_pos(anchor)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let Some(index) = menu_target else {
                    ui.label(
                        egui::RichText::new("Double-click the lane to add an event")
                            .size(10.5)
                            .color(ui.visuals().weak_text_color()),
                    );
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    return;
                };
                let Some((_, _, name)) = events.iter().find(|(i, _, _)| *i == index) else {
                    close = true;
                    return;
                };
                // The buffer lives in egui's temp store: a local rebuilt from
                // the document each frame would be overwritten by the value it
                // had just produced, so every keystroke would vanish.
                let buffer_id = ui.id().with(("event_name", index));
                let mut renamed = ui
                    .ctx()
                    .data(|d| d.get_temp::<String>(buffer_id))
                    .unwrap_or_else(|| name.clone());

                ui.label(egui::RichText::new("Event").strong());
                let field = ui.add(
                    egui::TextEdit::singleline(&mut renamed)
                        .desired_width(150.0)
                        .hint_text("name"),
                );
                if field.changed() {
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(buffer_id, renamed.clone()));
                }
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let renamable = !renamed.trim().is_empty() && renamed.trim() != name.trim();
                let clicked = ui
                    .add_enabled(renamable, egui::Button::new("Rename"))
                    .clicked();
                if (entered || clicked) && renamable {
                    state.dispatch(Box::new(EditEvent::new(
                        anim_id,
                        index,
                        EventEdit::Rename(renamed.trim().to_string()),
                    )));
                    ui.ctx().data_mut(|d| d.remove_temp::<String>(buffer_id));
                    close = true;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        state.dispatch(Box::new(EditEvent::new(anim_id, index, EventEdit::Remove)));
                        close = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close {
            ui.ctx()
                .data_mut(|d| d.remove_temp::<(Option<usize>, egui::Pos2)>(popup_id));
        }
    }
}
