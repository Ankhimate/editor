//! The dopesheet body: banded rows, keyed spans, key ticks, playhead.
//!
//! Iterates the same [`VisibleRow`] list as the name tree so every key sits on
//! its label's row. Interactions: click/ctrl-click select, drag selected keys in
//! time (one merged [`MoveKeys`]), drag off the bottom to delete, right-click for
//! the context menu.

use super::model::{TimelineModel, VisibleRow};
use super::tree::{band_color, is_folded};
use super::{Layout, ROW_H, ViewState};
use crate::app_state::AppState;
use ankhimate_core::animation::Interp;
use ankhimate_core::ids::AnimationId;
use ankhimate_document::commands::key_cmds::{DeleteKeys, KeyRef, MoveKeys, SetInterp};
use eframe::egui;

const KEY_HIT_R: f32 = 5.0;
const DELETE_DROP_MARGIN: f32 = 36.0;

/// Selected keys, in egui memory (UI state, never undoable).
#[derive(Clone, Default)]
pub struct Selection {
    pub keys: Vec<KeyRef>,
}

impl Selection {
    fn contains(&self, r: &KeyRef) -> bool {
        self.keys.contains(r)
    }
    fn toggle(&mut self, r: KeyRef) {
        if let Some(i) = self.keys.iter().position(|k| k == &r) {
            self.keys.remove(i);
        } else {
            self.keys.push(r);
        }
    }
    fn set_single(&mut self, r: KeyRef) {
        self.keys.clear();
        self.keys.push(r);
    }
}

#[derive(Clone, Default)]
struct KeyDrag {
    active: bool,
    start_x: f32,
    last_delta_frames: i64,
}

/// In-progress rubber-band box select.
#[derive(Clone, Copy, Default)]
struct BoxSelect {
    active: bool,
    start: egui::Pos2,
}

// Eight parameters, all of them distinct things the panel needs and none of
// which group into a struct that would mean anything on its own.
#[allow(clippy::too_many_arguments)]
pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    model: &TimelineModel,
    view: &mut ViewState,
    layout: &Layout,
    rect: egui::Rect,
    style: super::Style<'_>,
) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    painter.rect_filled(rect, 0.0, sheet_bg(&visuals));

    let sel_id = ui.id().with("tl_sel");
    let mut selection: Selection = ui
        .ctx()
        .memory(|m| m.data.get_temp(sel_id))
        .unwrap_or_default();
    let drag_id = ui.id().with("tl_keydrag");
    let mut drag: KeyDrag = ui
        .ctx()
        .memory(|m| m.data.get_temp(drag_id))
        .unwrap_or_default();
    let box_id = ui.id().with("tl_boxsel");
    let mut boxsel: BoxSelect = ui
        .ctx()
        .memory(|m| m.data.get_temp(box_id))
        .unwrap_or_default();

    // `K` sets a key on the active bone's active property.
    if ui.ctx().input(|i| i.key_pressed(egui::Key::K)) {
        super::graph::set_key_for_active_bone(state, anim);
    }

    // Background interaction is registered FIRST (lowest z), so the per-key
    // widgets drawn afterward sit on top and win clicks. Middle-drag pans the
    // time axis; left-drag on empty space starts a box select.
    let bg = ui.interact(
        rect,
        ui.id().with("tl_sheet_bg"),
        egui::Sense::click_and_drag(),
    );

    let folded = |id: u64| is_folded(ui, id);
    let rows = model.visible_rows(&folded);
    // Where the clip ends, for the row fills below.
    let duration = state
        .doc
        .animations
        .get(anim)
        .map(|a| a.duration)
        .unwrap_or(0.0);

    let mut y = rect.top() - view.scroll_y;
    let mut hit_any_key = false;
    // Screen centres of every editable key this frame, for box-select.
    let mut key_positions: Vec<(KeyRef, egui::Pos2)> = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), ROW_H));
        if row_rect.bottom() < rect.top() || row_rect.top() > rect.bottom() {
            y += ROW_H;
            continue;
        }
        // Two fills per row, not one. The sheet used to paint the tree's own
        // band colour edge to edge, which made a twelve-bone rig a wall of
        // alternating grey with no shape to it — and painted rows out to the
        // right margin forever, so most of what you were reading was empty
        // track that could never hold a key.
        //
        // Inside the clip the band is the tree's, damped: the tree column is
        // where names are read and deserves the contrast, while the sheet is a
        // backdrop for diamonds. Past the clip's end it drops to a flat dark
        // fill, so "where this animation stops" is visible without counting
        // frames on the ruler.
        let is_group = matches!(row, VisibleRow::Group { .. });
        let band = band_color(style.theme, is_group);
        let end_x = layout.time_to_x(duration).clamp(rect.left(), rect.right());
        painter.rect_filled(
            egui::Rect::from_min_max(row_rect.min, egui::pos2(end_x, row_rect.bottom())),
            0.0,
            band,
        );
        if end_x < rect.right() {
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(end_x, row_rect.top()), row_rect.max),
                0.0,
                band.gamma_multiply(0.72),
            );
        }
        // Paint the grid after the lane fill. Drawing it once behind all rows
        // made every band cover it, leaving the dopesheet without a frame
        // rhythm even though the grid code existed.
        draw_frame_grid(&painter, layout, row_rect.intersect(rect), &visuals);
        painter.line_segment(
            [row_rect.left_bottom(), row_rect.right_bottom()],
            egui::Stroke::new(
                1.0,
                visuals
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.7),
            ),
        );
        // A row filtered out by soloing keeps its keys on screen but greyed:
        // hiding them outright would make the dopesheet lie about what the clip
        // contains, which is the one thing it is for.
        let shown = row.is_soloed(&view.soloed);

        match row {
            VisibleRow::Group { .. } => {}
            VisibleRow::Property { data, .. } => {
                let channel = style.theme.channel_color(data.label);
                let values = data
                    .channels
                    .first()
                    .map(|channel| channel.values.as_slice());
                let value_range = values.and_then(value_range);
                let channel_selected = selection.keys.iter().any(|key| key.addr == data.addr);
                let display_color = if channel_selected {
                    style.theme.primary()
                } else {
                    channel
                };
                // A dopesheet describes held motion, not only isolated moments.
                // Join each key to the next, and the final key to clip end, so a
                // glance shows where a channel is active across time.
                for (index, key) in data.keys.iter().enumerate() {
                    let end = data
                        .keys
                        .get(index + 1)
                        .map_or(duration, |next| next.time)
                        .max(key.time);
                    let x0 = layout.time_to_x(key.time).clamp(rect.left(), rect.right());
                    let x1 = layout.time_to_x(end).clamp(rect.left(), rect.right());
                    if x1 > x0 {
                        let color = display_color.gamma_multiply(if shown { 0.9 } else { 0.3 });
                        let center_y = y + ROW_H / 2.0;
                        let y0 = keyed_y(values, value_range, index, center_y);
                        let y1 = if data.keys.get(index + 1).is_some() {
                            keyed_y(values, value_range, index + 1, center_y)
                        } else {
                            y0
                        };
                        draw_key_span(&painter, x0, x1, y0, y1, key.interp, color);
                    }
                }
                for k in &data.keys {
                    let x = layout.time_to_x(k.time);
                    if x < rect.left() - KEY_HIT_R || x > rect.right() + KEY_HIT_R {
                        continue;
                    }
                    let center_y = y + ROW_H / 2.0;
                    let center = egui::pos2(x, center_y);
                    let kref = KeyRef {
                        addr: data.addr.clone(),
                        index: k.index,
                    };
                    let selected = selection.contains(&kref);
                    // Keys wear the same channel colour as their graph curve.
                    draw_key(
                        &painter,
                        center,
                        k.interp,
                        selected,
                        data.read_only || !shown,
                        &visuals,
                        display_color,
                    );

                    // A greyed row is not editable either: dragging a key you
                    // cannot properly see is how keys end up somewhere nobody
                    // meant to put them.
                    if data.read_only || !shown {
                        continue;
                    }
                    key_positions.push((kref.clone(), center));
                    let hit =
                        egui::Rect::from_center_size(center, egui::vec2(KEY_HIT_R * 2.4, ROW_H));
                    let resp = ui.interact(
                        hit,
                        // The row index is part of the id because a document may
                        // legally hold two timelines for one property — an
                        // importer that emits X and Y as separate tracks, say.
                        // Addressing by target alone made those rows share ids,
                        // and egui reports the clash across the whole panel.
                        ui.id().with(("tl_key", i, data.addr.stable_id(), k.index)),
                        egui::Sense::click_and_drag(),
                    );
                    if resp.hovered() || selected {
                        painter.rect_stroke(
                            egui::Rect::from_center_size(
                                egui::pos2(center.x, center_y),
                                egui::vec2(7.0, ROW_H - 1.0),
                            ),
                            2.0,
                            egui::Stroke::new(
                                if selected { 2.0 } else { 1.0 },
                                if selected {
                                    style.theme.primary()
                                } else {
                                    visuals.strong_text_color()
                                },
                            ),
                            egui::StrokeKind::Middle,
                        );
                    }
                    if resp.clicked() {
                        hit_any_key = true;
                        if ui.ctx().input(|i| i.modifiers.ctrl) {
                            selection.toggle(kref.clone());
                        } else {
                            selection.set_single(kref.clone());
                        }
                    }
                    if resp.drag_started() {
                        if !selection.contains(&kref) {
                            selection.set_single(kref.clone());
                        }
                        drag.active = true;
                        drag.start_x = resp.interact_pointer_pos().map(|p| p.x).unwrap_or(x);
                        drag.last_delta_frames = 0;
                    }
                    resp.context_menu(|ui| {
                        if !selection.contains(&kref) {
                            selection.set_single(kref.clone());
                        }
                        key_context_menu(ui, state, anim, &selection);
                    });
                }
            }
        }
        y += ROW_H;
    }

    let sheet_bottom = y.min(rect.bottom());

    // Playhead over the sheet.
    let px = layout.time_to_x(state.session.playhead);
    if px >= rect.left() && px <= rect.right() {
        painter.line_segment(
            [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
            egui::Stroke::new(1.0, playhead_color().gamma_multiply(0.7)),
        );
    }

    // Selection drag.
    if drag.active {
        let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
        let released = ui.ctx().input(|i| i.pointer.any_released());
        if let Some(p) = pointer {
            let dx_time = (p.x - drag.start_x) / layout.px_per_sec;
            let delta_frames = (dx_time * layout.fps as f32).round() as i64;
            if delta_frames != drag.last_delta_frames {
                let step = (delta_frames - drag.last_delta_frames) as f32 / layout.fps as f32;
                drag.last_delta_frames = delta_frames;
                state.dispatch(Box::new(MoveKeys::new(anim, selection.keys.clone(), step)));
            }
            if released {
                if p.y > sheet_bottom + DELETE_DROP_MARGIN && !selection.keys.is_empty() {
                    state.dispatch(Box::new(DeleteKeys::new(anim, selection.keys.clone())));
                    selection.keys.clear();
                }
                drag = KeyDrag::default();
            }
        }
        if released {
            drag = KeyDrag::default();
        }
    }

    // Middle-drag pans the time axis (Spine's hand-pan). Registered on `bg` so it
    // never fights a key drag.
    if bg.dragged_by(egui::PointerButton::Middle) {
        let dx = bg.drag_delta().x;
        view.scroll_sec = (view.scroll_sec - dx / layout.px_per_sec).max(0.0);
    }

    // Left-drag on empty space starts a box select; a plain click clears.
    if !drag.active {
        if bg.drag_started_by(egui::PointerButton::Primary)
            && let Some(p) = bg.interact_pointer_pos()
        {
            boxsel.active = true;
            boxsel.start = p;
            if !ui.ctx().input(|i| i.modifiers.ctrl) {
                selection.keys.clear();
            }
        }
        if bg.clicked() && !hit_any_key {
            selection.keys.clear();
        }
    }

    if boxsel.active {
        let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
        let released = ui.ctx().input(|i| i.pointer.any_released());
        if let Some(p) = pointer {
            let band = egui::Rect::from_two_pos(boxsel.start, p);
            // Draw the marquee.
            painter.rect_filled(band, 0.0, visuals.selection.bg_fill.gamma_multiply(0.15));
            painter.rect_stroke(
                band,
                0.0,
                egui::Stroke::new(1.0, visuals.selection.bg_fill),
                egui::StrokeKind::Inside,
            );
            if released {
                // Additively select every key whose centre is inside the band.
                for (kref, center) in &key_positions {
                    if band.contains(*center) && !selection.contains(kref) {
                        selection.keys.push(kref.clone());
                    }
                }
                boxsel = BoxSelect::default();
            }
        }
        if released {
            boxsel = BoxSelect::default();
        }
        ui.ctx().request_repaint();
    }

    ui.ctx().memory_mut(|m| {
        m.data.insert_temp(sel_id, selection);
        m.data.insert_temp(drag_id, drag);
        m.data.insert_temp(box_id, boxsel);
    });
}

fn sheet_bg(visuals: &egui::Visuals) -> egui::Color32 {
    visuals.extreme_bg_color
}

/// Spine's playhead is orange.
pub fn playhead_color() -> egui::Color32 {
    egui::Color32::from_rgb(240, 150, 40)
}

fn draw_frame_grid(
    painter: &egui::Painter,
    layout: &Layout,
    rect: egui::Rect,
    visuals: &egui::Visuals,
) {
    let fps = layout.fps as f32;
    let left = layout.scroll_sec;
    let right = layout.x_to_time(rect.right());
    let frame_px = layout.px_per_sec / fps;
    let first = (left * fps).floor().max(0.0) as i64;
    let last = (right * fps).ceil() as i64;
    let line = visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.4);
    let second_line = visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.8);
    for frame in first..=last {
        let x = layout.time_to_x(frame as f32 / fps);
        if x < rect.left() || x > rect.right() {
            continue;
        }
        let on_second = frame % layout.fps as i64 == 0;
        if !on_second && frame_px < 8.0 {
            continue;
        }
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, if on_second { second_line } else { line }),
        );
    }
}

fn value_range(values: &[f32]) -> Option<(f32, f32)> {
    let min = values.iter().copied().reduce(f32::min)?;
    let max = values.iter().copied().reduce(f32::max)?;
    (max - min > f32::EPSILON).then_some((min, max))
}

fn keyed_y(values: Option<&[f32]>, range: Option<(f32, f32)>, index: usize, center_y: f32) -> f32 {
    let Some(((min, max), value)) = range.zip(values.and_then(|values| values.get(index))) else {
        return center_y;
    };
    let normalized = (*value - min) / (max - min);
    center_y + (0.5 - normalized) * 8.0
}

fn draw_key_span(
    painter: &egui::Painter,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    interp: Interp,
    color: egui::Color32,
) {
    let points = match interp {
        Interp::Stepped => vec![egui::pos2(x0, y0), egui::pos2(x1, y0)],
        Interp::Linear => vec![egui::pos2(x0, y0), egui::pos2(x1, y1)],
        Interp::Bezier {
            out_handle,
            in_handle,
        } => (0..=16)
            .map(|step| {
                let t = step as f32 / 16.0;
                let mt = 1.0 - t;
                let bx =
                    3.0 * mt * mt * t * out_handle.x + 3.0 * mt * t * t * in_handle.x + t * t * t;
                let by =
                    3.0 * mt * mt * t * out_handle.y + 3.0 * mt * t * t * in_handle.y + t * t * t;
                egui::pos2(x0 + (x1 - x0) * bx, y0 + (y1 - y0) * by)
            })
            .collect(),
    };
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, color)));
}

/// Draw a compact vertical property-key tick.
fn draw_key(
    painter: &egui::Painter,
    center: egui::Pos2,
    interp: Interp,
    selected: bool,
    read_only: bool,
    visuals: &egui::Visuals,
    channel: egui::Color32,
) {
    let fill = if selected {
        visuals.selection.bg_fill
    } else if read_only {
        visuals.weak_text_color()
    } else {
        channel
    };
    let stroke = egui::Stroke::new(if selected { 3.0 } else { 2.0 }, fill);
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - ROW_H * 0.42),
            egui::pos2(center.x, center.y + ROW_H * 0.42),
        ],
        stroke,
    );
    let cap = if matches!(interp, Interp::Stepped) {
        4.0
    } else {
        2.5
    };
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - cap),
            egui::pos2(center.x + cap, center.y - cap),
        ],
        stroke,
    );
    if matches!(interp, Interp::Bezier { .. }) {
        painter.circle_filled(center, 1.7, visuals.extreme_bg_color);
    }
}

fn key_context_menu(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    selection: &Selection,
) {
    if ui.button("Delete").clicked() {
        state.dispatch(Box::new(DeleteKeys::new(anim, selection.keys.clone())));
        ui.close();
    }
    ui.menu_button("Interpolation", |ui| {
        for (label, interp) in ankhimate_document::commands::key_cmds::presets::all() {
            if ui.button(label).clicked() {
                state.dispatch(Box::new(SetInterp::new(
                    anim,
                    selection.keys.clone(),
                    interp,
                )));
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_values_use_the_rows_compact_vertical_range() {
        let values = [10.0, 20.0, 30.0];
        let range = value_range(&values);
        assert_eq!(keyed_y(Some(&values), range, 0, 50.0), 54.0);
        assert_eq!(keyed_y(Some(&values), range, 1, 50.0), 50.0);
        assert_eq!(keyed_y(Some(&values), range, 2, 50.0), 46.0);
    }
}
