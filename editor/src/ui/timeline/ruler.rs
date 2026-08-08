//! Frame ruler + orange playhead. Click/drag anywhere on the sheet portion of
//! the ruler scrubs `Session.playhead`, snapped to whole frames.
//!
//! Markers (T-906) live here rather than in a lane of their own: a marker labels
//! a *time*, and the ruler is where times are already written. A second strip
//! would have cost vertical space to say the same thing further from it.

use super::{Layout, sheet};
use crate::app_state::AppState;
use crate::commands::marker_cmds::{AddMarker, EditMarker, MarkerEdit};
use eframe::egui;

/// How close to a marker's stem a click must land to grab it.
const MARKER_HIT: f32 = 6.0;

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    layout: &Layout,
    rect: egui::Rect,
    style: super::Style<'_>,
) {
    ui.allocate_rect(rect, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

    let sheet_rect = egui::Rect::from_min_max(egui::pos2(layout.sheet_x0, rect.top()), rect.max);
    let fps = layout.fps as f32;
    let left = layout.scroll_sec;
    let right = layout.x_to_time(sheet_rect.right());
    let frame_px = layout.px_per_sec / fps;
    let first = (left * fps).floor().max(0.0) as i64;
    let last = (right * fps).ceil() as i64;
    let tick = visuals.weak_text_color();

    for frame in first..=last {
        let x = layout.time_to_x(frame as f32 / fps);
        if x < sheet_rect.left() - 1.0 || x > sheet_rect.right() + 1.0 {
            continue;
        }
        let on_second = frame % layout.fps as i64 == 0;
        if !on_second && frame_px < 7.0 {
            continue;
        }
        let h = if on_second {
            rect.height()
        } else {
            rect.height() * 0.4
        };
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - h),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, tick.gamma_multiply(if on_second { 1.0 } else { 0.5 })),
        );
        // Label density scales with zoom: every frame when wide, every 5th when
        // medium, whole seconds when zoomed out.
        let fps_i = layout.fps as i64;
        let label = if frame_px >= 26.0 || (frame_px >= 6.0 && frame % 5 == 0) {
            Some(format!("{frame}"))
        } else if on_second {
            Some(format!("{}s", frame / fps_i))
        } else {
            None
        };
        if let Some(text) = label {
            painter.text(
                egui::pos2(x + 3.0, rect.top() + 2.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(style.text - 1.0),
                tick,
            );
        }
    }

    // ── Markers (T-906) ──────────────────────────────────────────────────
    // Claimed before the scrub below, so grabbing a marker does not also drag
    // the playhead out from under it.
    let markers: Vec<(usize, f32, String, [f32; 4])> = state
        .session
        .active_animation
        .and_then(|id| state.doc.animations.get(id))
        .map(|a| {
            a.markers
                .iter()
                .enumerate()
                .map(|(i, m)| (i, m.time, m.name.clone(), m.color))
                .collect()
        })
        .unwrap_or_default();

    // **One** interaction for the whole strip, branching inside. Markers and the
    // scrub used to register two `interact` calls over the same rect; egui gives
    // the pointer to whichever was registered last, so the scrub silently ate
    // every click the markers needed — including the right-click that opens the
    // rename menu, which therefore never opened at all.
    let marker_response = ui.interact(
        sheet_rect,
        ui.id().with("tl_ruler"),
        egui::Sense::click_and_drag(),
    );
    // `hover_pos` as well as `interact_pointer_pos`: the former is `None` unless
    // a button is down, and the menu has to know which flag was under the cursor
    // on the frame the right-click landed.
    let pointer = marker_response
        .interact_pointer_pos()
        .or_else(|| ui.ctx().pointer_latest_pos());
    let grabbed = pointer.and_then(|p| {
        markers
            .iter()
            .map(|(i, time, _, _)| (*i, (layout.time_to_x(*time) - p.x).abs()))
            .filter(|(_, distance)| *distance <= MARKER_HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });

    let mut marker_edit: Option<(usize, MarkerEdit)> = None;
    // A drag that *starts* on a flag moves the flag; one that starts anywhere
    // else scrubs, even if it passes over a flag on the way. Deciding that once
    // at drag start is what keeps a scrub from snagging on every marker it
    // crosses.
    if marker_response.drag_started() {
        state.session.dragging_marker = grabbed;
    }
    if let (Some(index), Some(pos)) = (state.session.dragging_marker, pointer)
        && marker_response.dragged()
    {
        marker_edit = Some((
            index,
            MarkerEdit::SetTime(layout.snap_time(layout.x_to_time(pos.x)).max(0.0)),
        ));
    }
    if marker_response.drag_stopped() {
        state.session.dragging_marker = None;
    }

    for (index, time, name, color) in &markers {
        let x = layout.time_to_x(*time);
        if x < sheet_rect.left() - 30.0 || x > sheet_rect.right() + 30.0 {
            continue;
        }
        let rgba = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );
        let dragging = state.session.dragging_marker == Some(*index);
        let paint = if dragging { egui::Color32::WHITE } else { rgba };
        // A flag on a stem, hanging from the ruler's underside so it cannot be
        // confused with the playhead's triangle at the top.
        painter.line_segment(
            [
                egui::pos2(x, rect.top() + 2.0),
                egui::pos2(x, rect.bottom()),
            ],
            egui::Stroke::new(1.0, paint.gamma_multiply(0.8)),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, rect.bottom() - 8.0),
                egui::pos2(x + 6.0, rect.bottom() - 6.0),
                egui::pos2(x, rect.bottom() - 4.0),
            ],
            paint,
            egui::Stroke::NONE,
        ));
        // The name only where the next marker leaves room, so a dense clip does
        // not become overlapping text.
        let room = markers
            .get(index + 1)
            .map(|(_, next, _, _)| layout.time_to_x(*next) - x)
            .unwrap_or(f32::MAX);
        if room > 44.0 {
            painter.text(
                egui::pos2(x + 8.0, rect.bottom() - 7.0),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(style.text - 1.5),
                paint,
            );
        }
    }

    // ── Rename / delete ──────────────────────────────────────────────────
    // Which marker the menu is *about* is decided when the menu opens and then
    // remembered. Reading it from the pointer each frame was the first attempt
    // and cannot work: the moment the cursor moves off the flag and into the
    // menu, the hit test finds nothing and the menu swaps to its "empty ruler"
    // branch — so the name field vanished the instant you reached for it.
    let menu_id = ui.id().with("marker_menu_target");
    let clicked_time = pointer
        .map(|p| layout.snap_time(layout.x_to_time(p.x)).max(0.0))
        .unwrap_or(0.0);
    if marker_response.secondary_clicked() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(menu_id, (grabbed, clicked_time)));
    }
    let (menu_target, menu_time) = ui
        .ctx()
        .data(|d| d.get_temp::<(Option<usize>, f32)>(menu_id))
        .unwrap_or((None, 0.0));

    marker_response.context_menu(|ui| {
        if let Some(index) = menu_target {
            let Some((_, _, current, color)) = markers.get(index) else {
                return;
            };
            // The buffer lives in egui's temp store, keyed by marker: a local
            // `String` would be rebuilt from the document every frame, so each
            // keystroke would be overwritten by the value it just produced.
            let buffer_id = ui.id().with(("marker_name", index));
            let mut name = ui
                .ctx()
                .data(|d| d.get_temp::<String>(buffer_id))
                .unwrap_or_else(|| current.clone());

            ui.label(egui::RichText::new("Marker").strong());
            let field = ui.add(
                egui::TextEdit::singleline(&mut name)
                    .desired_width(140.0)
                    .hint_text("name"),
            );
            if field.changed() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(buffer_id, name.clone()));
            }
            // Committed only on a deliberate Enter *in this field*, so one
            // rename is one undo rather than one per letter.
            //
            // Not on `lost_focus`, which was the first attempt and closed the
            // menu on every click inside it: reaching for the colour swatch or
            // Delete drops focus, which dispatched a rename, which re-rendered
            // and dismissed the popup before the click landed.
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            // A button as well as Enter, because a typed name that is only
            // applied by a key nobody mentioned is a name silently thrown away.
            let clicked = ui
                .add_enabled(name.trim() != current.trim(), egui::Button::new("Rename"))
                .clicked();
            if entered || clicked {
                if name != *current && !name.trim().is_empty() {
                    marker_edit = Some((index, MarkerEdit::Rename(name.trim().to_string())));
                }
                ui.ctx().data_mut(|d| d.remove_temp::<String>(buffer_id));
            }

            let mut rgba = egui::Color32::from_rgb(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            );
            ui.horizontal(|ui| {
                ui.label("Colour");
                if ui.color_edit_button_srgba(&mut rgba).changed() {
                    marker_edit = Some((
                        index,
                        MarkerEdit::SetColor([
                            rgba.r() as f32 / 255.0,
                            rgba.g() as f32 / 255.0,
                            rgba.b() as f32 / 255.0,
                            1.0,
                        ]),
                    ));
                }
            });

            ui.separator();
            if ui.button("Delete").clicked() {
                marker_edit = Some((index, MarkerEdit::Remove));
                ui.close();
            }
        } else if ui.button("Add marker here").clicked() {
            if let Some(anim) = state.session.active_animation {
                state.dispatch(Box::new(AddMarker::new(anim, "marker", menu_time)));
            }
            ui.close();
        }
    });

    if let Some((index, edit)) = marker_edit
        && let Some(anim) = state.session.active_animation
    {
        state.dispatch(Box::new(EditMarker::new(anim, index, edit)));
    }

    // Scrub, on the same response — but not while a marker is being dragged, and
    // not on the click that grabbed one.
    let resp = &marker_response;
    // Only an actual marker drag blocks the scrub. Merely hovering near a flag
    // must not, or the ruler would go dead in a band around every marker.
    let marker_has_the_pointer = state.session.dragging_marker.is_some();
    if !marker_has_the_pointer
        && let Some(pos) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        let raw = layout.x_to_time(pos.x);
        // Markers pull the playhead in, within a few pixels' worth of time. The
        // point of naming a pose is being able to get back to it exactly, and
        // frame-snapping alone does not do that on a clip whose key poses sit
        // between frames. Held Alt scrubs freely past them.
        let magnet = MARKER_HIT / layout.px_per_sec.max(1e-3);
        let free = ui.input(|i| i.modifiers.alt);
        let snapped = (!free)
            .then(|| {
                state
                    .session
                    .active_animation
                    .and_then(|id| state.doc.animations.get(id))
                    .and_then(|a| a.marker_near(raw, magnet))
                    .map(|m| m.time)
            })
            .flatten();
        let t = snapped.unwrap_or_else(|| layout.snap_time(raw)).max(0.0);
        let dur = state
            .session
            .active_animation
            .and_then(|id| state.doc.animations.get(id))
            .map(|a| a.duration)
            .unwrap_or(0.0);
        state.session.playing = false;
        // Via `set_playhead` so an unkeyed pose from the frame we are leaving is
        // dropped rather than silently following the playhead (T-207).
        state.set_playhead(t.min(dur.max(0.0)));
    }

    // Orange playhead + triangle handle.
    let px = layout.time_to_x(state.session.playhead);
    if px >= sheet_rect.left() && px <= sheet_rect.right() {
        let orange = sheet::playhead_color();
        painter.line_segment(
            [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
            egui::Stroke::new(1.5, orange),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(px - 5.0, rect.top()),
                egui::pos2(px + 5.0, rect.top()),
                egui::pos2(px, rect.top() + 7.0),
            ],
            orange,
            egui::Stroke::NONE,
        ));
    }
}
