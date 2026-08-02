//! The dopesheet body: banded rows, group summary dots, key diamonds, playhead.
//!
//! Iterates the same [`VisibleRow`] list as the name tree so every key sits on
//! its label's row. Interactions: click/ctrl-click select, drag selected keys in
//! time (one merged [`MoveKeys`]), drag off the bottom to delete, right-click for
//! the context menu.

use super::model::{TimelineModel, VisibleRow};
use super::tree::{band_color, is_folded};
use super::{Layout, ROW_H, ViewState};
use crate::app_state::AppState;
use crate::commands::key_cmds::{DeleteKeys, KeyRef, MoveKeys, SetInterp};
use ankhimate_core::animation::Interp;
use ankhimate_core::ids::AnimationId;
use eframe::egui;

const DIAMOND_R: f32 = 5.0;
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

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    model: &TimelineModel,
    view: &mut ViewState,
    layout: &Layout,
    rect: egui::Rect,
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

    // Vertical frame gridlines behind the rows.
    draw_frame_grid(&painter, layout, rect, &visuals);

    let folded = |id: u64| is_folded(ui, id);
    let rows = model.visible_rows(&folded);

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
        if i % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, band_color(&visuals));
        }

        match row {
            VisibleRow::Group { data, folded, .. } => {
                // Summary dots: one per distinct child key time. When the group is
                // folded these are the only markers, so the user still sees where
                // its keys are.
                for &t in &data.summary_times {
                    let x = layout.time_to_x(t);
                    if x < rect.left() - 4.0 || x > rect.right() + 4.0 {
                        continue;
                    }
                    let dim = if *folded { 1.0 } else { 0.5 };
                    painter.circle_filled(
                        egui::pos2(x, y + ROW_H / 2.0),
                        2.5,
                        visuals.weak_text_color().gamma_multiply(dim),
                    );
                }
            }
            VisibleRow::Property { data, .. } => {
                for k in &data.keys {
                    let x = layout.time_to_x(k.time);
                    if x < rect.left() - DIAMOND_R || x > rect.right() + DIAMOND_R {
                        continue;
                    }
                    let center = egui::pos2(x, y + ROW_H / 2.0);
                    let kref = KeyRef {
                        addr: data.addr.clone(),
                        index: k.index,
                    };
                    let selected = selection.contains(&kref);
                    draw_key(
                        &painter,
                        center,
                        k.interp,
                        selected,
                        data.read_only,
                        &visuals,
                    );

                    if data.read_only {
                        continue;
                    }
                    key_positions.push((kref.clone(), center));
                    let hit =
                        egui::Rect::from_center_size(center, egui::vec2(DIAMOND_R * 2.4, ROW_H));
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

/// Draw a key marker by interpolation kind (diamond / square / bezier dot).
fn draw_key(
    painter: &egui::Painter,
    center: egui::Pos2,
    interp: Interp,
    selected: bool,
    read_only: bool,
    visuals: &egui::Visuals,
) {
    let fill = if selected {
        visuals.selection.bg_fill
    } else if read_only {
        visuals.weak_text_color()
    } else {
        egui::Color32::from_rgb(230, 200, 120)
    };
    let stroke = egui::Stroke::new(1.0, visuals.extreme_bg_color);
    match interp {
        Interp::Stepped => {
            let rect =
                egui::Rect::from_center_size(center, egui::vec2(DIAMOND_R * 1.7, DIAMOND_R * 1.7));
            painter.rect_filled(rect, 1.0, fill);
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
        }
        Interp::Linear | Interp::Bezier { .. } => {
            let d = DIAMOND_R;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(center.x, center.y - d),
                    egui::pos2(center.x + d, center.y),
                    egui::pos2(center.x, center.y + d),
                    egui::pos2(center.x - d, center.y),
                ],
                fill,
                stroke,
            ));
            if matches!(interp, Interp::Bezier { .. }) {
                painter.circle_filled(center, 1.6, visuals.extreme_bg_color);
            }
        }
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
        for (label, interp) in crate::commands::key_cmds::presets::all() {
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
