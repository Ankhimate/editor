//! The graph editor: property channels as editable bezier curves (Spine's Graph
//! tab, folding in T-507 early).
//!
//! Each unfolded property row contributes one or two scalar **channels** (rotate
//! → angle; translate/scale/shear → x, y; color → alpha). Channels are plotted as
//! curves over time, auto-fit to the visible value range. Interactions:
//!
//! * drag a key point vertically → change its value (one overwrite `AddKey`);
//! * drag a key horizontally → change its time (snapped, via `MoveKeys`);
//! * drag a bezier handle → set that key's `Interp::Bezier` control points.
//!
//! A segment's easing lives on the key that **ends** it, so its two handles hang
//! off the key before it (`out_handle`) and the key itself (`in_handle`), in
//! normalized 0..1 time/value space against that segment.
//!
//! Value editing rebuilds the whole key value (both axes of a vec2) so the
//! sibling channel is preserved.

use super::model::{TimelineModel, VisibleRow};
use super::{Layout, ViewState};
use crate::app_state::AppState;
use crate::commands::key_cmds::{AddKey, BoneProperty, KeyRef, KeyValue, MoveKeys, TimelineAddr};
use ankhimate_core::animation::{Animation, Interp, Timeline};
use ankhimate_core::ids::AnimationId;
use eframe::egui;

const KEY_R: f32 = 4.0;

fn axis_color(axis: usize) -> egui::Color32 {
    match axis {
        0 => egui::Color32::from_rgb(220, 90, 80),   // x — red
        1 => egui::Color32::from_rgb(110, 200, 110), // y — green
        _ => egui::Color32::from_rgb(110, 160, 230), // single — blue
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    model: &TimelineModel,
    view: &ViewState,
    layout: &Layout,
    rect: egui::Rect,
) {
    // Clipped to the sheet: the curve sampler walks every pixel of the plot and
    // the handle interactions allocate their own rects, so without this both
    // spill left over the row names.
    let painter = ui.painter_at(rect).with_clip_rect(rect);
    let visuals = ui.visuals().clone();
    painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);

    if ui.ctx().input(|i| i.key_pressed(egui::Key::K)) {
        set_key_for_active_bone(state, anim);
    }

    // Gather channels from the unfolded, editable property rows.
    //
    // Fold state is read into an owned set first: `visible_rows` borrows `ui`
    // through the predicate, and the row loop below needs `ui` mutably to run
    // handle interactions.
    // The predicate closes over the context, not over `ui`: the row loop below
    // needs `ui` mutably for the handle interactions, and a closure holding `ui`
    // would still be alive across it.
    let ctx = ui.ctx().clone();
    let folded = move |id: u64| -> bool {
        ctx.memory(|m| m.data.get_temp(egui::Id::new(("tl_fold", id))))
            .unwrap_or(false)
    };
    let rows = model.visible_rows(&folded);

    // Auto-fit the value range across all shown channels.
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for row in &rows {
        if !row.is_soloed(&view.soloed) {
            continue;
        }
        if let VisibleRow::Property { data, .. } = row
            && !data.read_only
        {
            for ch in &data.channels {
                for &v in &ch.values {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        // No numeric channels; nothing to plot.
        placeholder(&painter, rect, &visuals);
        return;
    }
    // Pad the range so curves are not glued to the edges.
    if (hi - lo).abs() < 1e-3 {
        lo -= 1.0;
        hi += 1.0;
    }
    let pad = (hi - lo) * 0.1;
    lo -= pad;
    hi += pad;

    let val_to_y = |v: f32| {
        let t = (v - lo) / (hi - lo);
        rect.bottom() - t * rect.height()
    };

    draw_frame_grid(&painter, layout, rect, &visuals);
    draw_value_axis(&painter, rect, lo, hi, &visuals);

    let animation = state.doc.animations.get(anim).cloned();
    let Some(animation) = animation else { return };

    let mut pending: Option<Edit> = None;
    let mut interp_edit: Option<InterpEdit> = None;

    for row in &rows {
        // Soloing is a display filter, so it applies to the curves and not to
        // the row list — the row stays visible so you can un-solo it.
        if !row.is_soloed(&view.soloed) {
            continue;
        }
        let VisibleRow::Property { data, .. } = row else {
            continue;
        };
        if data.read_only {
            continue;
        }
        for ch in &data.channels {
            let color = axis_color(ch.axis);
            // Sample the curve across the visible range for a smooth plot.
            draw_channel_curve(
                &painter, &animation, &data.addr, ch.axis, layout, rect, &val_to_y, color,
            );

            // Bezier handles, one pair per segment. Drawn before the key dots
            // so a handle sitting on top of a key does not hide it.
            if let Some(edit) = bezier_handles(
                ui, &painter, &data.addr, ch.axis, &data.keys, &ch.values, layout, rect, &val_to_y,
                lo, hi, color,
            ) {
                interp_edit = Some(edit);
            }

            // Key points, draggable in value (y) and time (x).
            for k in &data.keys {
                let Some(&value) = ch.values.get(k.index) else {
                    continue;
                };
                let x = layout.time_to_x(k.time);
                let y = val_to_y(value);
                if x < rect.left() - KEY_R || x > rect.right() + KEY_R {
                    continue;
                }
                let center = egui::pos2(x, y);
                painter.circle_filled(center, KEY_R, color);
                painter.circle_stroke(
                    center,
                    KEY_R,
                    egui::Stroke::new(1.0, visuals.extreme_bg_color),
                );

                let hit =
                    egui::Rect::from_center_size(center, egui::vec2(KEY_R * 3.0, KEY_R * 3.0));
                if center.x < rect.left() {
                    continue;
                }
                let resp = ui.interact(
                    hit,
                    ui.id()
                        .with(("graph_key", data.addr.stable_id(), ch.axis, k.index)),
                    egui::Sense::drag(),
                );
                if resp.dragged()
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    // New value from the vertical position; new time from x.
                    let new_val = lo + (rect.bottom() - p.y) / rect.height() * (hi - lo);
                    let new_time = layout.snap_time(layout.x_to_time(p.x)).max(0.0);
                    pending = Some(Edit {
                        addr: data.addr.clone(),
                        index: k.index,
                        axis: ch.axis,
                        new_val,
                        new_time,
                        old_time: k.time,
                    });
                }
            }
        }
    }

    // Playhead.
    let px = layout.time_to_x(state.session.playhead);
    if px >= rect.left() && px <= rect.right() {
        painter.line_segment(
            [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
            egui::Stroke::new(1.0, super::sheet::playhead_color().gamma_multiply(0.7)),
        );
    }

    if let Some(edit) = pending {
        apply_edit(state, anim, &animation, edit);
    }
    if let Some(edit) = interp_edit {
        state.dispatch(Box::new(crate::commands::key_cmds::SetInterp::new(
            anim,
            vec![KeyRef {
                addr: edit.addr,
                index: edit.index,
            }],
            edit.interp,
        )));
    }
}

/// A pending easing change from dragging a handle.
struct InterpEdit {
    addr: TimelineAddr,
    /// The key the easing belongs to — the one that *ends* the segment.
    index: usize,
    interp: Interp,
}

/// Handle radius, and half its grab box.
const HANDLE_R: f32 = 3.5;

/// A linear segment has no stored handles, so it borrows the ones that describe
/// a straight line. Grabbing either converts the segment to a bezier without the
/// curve jumping, which is how every curve editor behaves.
const LINEAR_OUT: glam::Vec2 = glam::Vec2::new(1.0 / 3.0, 1.0 / 3.0);
const LINEAR_IN: glam::Vec2 = glam::Vec2::new(2.0 / 3.0, 2.0 / 3.0);

/// Draw and drag both handles of every segment in one channel.
///
/// Handles live in normalized 0..1 time/value space against the segment they
/// belong to, so they are converted to and from graph coordinates here. A
/// segment whose two keys share a value has no vertical extent to normalize
/// against; its handle then moves in time only, which is the honest behaviour —
/// there is no shape to give a flat run.
#[allow(clippy::too_many_arguments)]
fn bezier_handles(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    addr: &TimelineAddr,
    axis: usize,
    keys: &[super::model::KeyInfo],
    values: &[f32],
    layout: &Layout,
    rect: egui::Rect,
    val_to_y: &impl Fn(f32) -> f32,
    lo: f32,
    hi: f32,
    color: egui::Color32,
) -> Option<InterpEdit> {
    let mut edit = None;
    for window in 0..keys.len().saturating_sub(1) {
        let (a, b) = (&keys[window], &keys[window + 1]);
        // Stepped holds a value: there is no curve between the keys to shape.
        if matches!(b.interp, Interp::Stepped) {
            continue;
        }
        let (Some(&va), Some(&vb)) = (values.get(a.index), values.get(b.index)) else {
            continue;
        };
        let (out, inn) = match b.interp {
            Interp::Bezier {
                out_handle,
                in_handle,
            } => (out_handle, in_handle),
            _ => (LINEAR_OUT, LINEAR_IN),
        };

        let (dt, dv) = (b.time - a.time, vb - va);
        let to_screen = |h: glam::Vec2| {
            egui::pos2(layout.time_to_x(a.time + h.x * dt), val_to_y(va + h.y * dv))
        };
        let anchors = [
            egui::pos2(layout.time_to_x(a.time), val_to_y(va)),
            egui::pos2(layout.time_to_x(b.time), val_to_y(vb)),
        ];
        if anchors[1].x < rect.left() - 40.0 || anchors[0].x > rect.right() + 40.0 {
            continue;
        }

        for (which, handle) in [(0usize, out), (1usize, inn)] {
            let pos = to_screen(handle);
            let stem = anchors[which];
            painter.line_segment(
                [stem, pos],
                egui::Stroke::new(1.0, color.gamma_multiply(0.45)),
            );
            painter.circle_filled(pos, HANDLE_R, color.gamma_multiply(0.8));

            let hit = egui::Rect::from_center_size(pos, egui::vec2(12.0, 12.0));
            // A handle whose grab box hangs over the row names would steal
            // clicks meant for the tree.
            if !rect.intersects(hit) || hit.center().x < rect.left() {
                continue;
            }
            let response = ui.interact(
                hit,
                ui.id()
                    .with(("graph_handle", addr.stable_id(), axis, b.index, which)),
                egui::Sense::drag(),
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            let Some(p) = response
                .dragged()
                .then(|| response.interact_pointer_pos())
                .flatten()
            else {
                continue;
            };
            // Back to normalized space. Time is clamped to the segment: a handle
            // outside it produces a curve that doubles back in time, which is not
            // a function of t and cannot be sampled.
            let nx = if dt.abs() < 1e-6 {
                0.0
            } else {
                ((layout.x_to_time(p.x) - a.time) / dt).clamp(0.0, 1.0)
            };
            let value = lo + (rect.bottom() - p.y) / rect.height().max(1.0) * (hi - lo);
            let ny = if dv.abs() < 1e-6 {
                handle.y
            } else {
                (value - va) / dv
            };
            let moved = glam::vec2(nx, ny);
            let (out, inn) = if which == 0 {
                (moved, inn)
            } else {
                (out, moved)
            };
            edit = Some(InterpEdit {
                addr: addr.clone(),
                index: b.index,
                interp: Interp::Bezier {
                    out_handle: out,
                    in_handle: inn,
                },
            });
        }
    }
    edit
}

/// A pending key edit from a graph drag.
struct Edit {
    addr: TimelineAddr,
    index: usize,
    axis: usize,
    new_val: f32,
    new_time: f32,
    old_time: f32,
}

/// Apply a graph key drag: value change via overwrite `AddKey`, time change via
/// `MoveKeys`.
fn apply_edit(state: &mut AppState, anim: AnimationId, animation: &Animation, edit: Edit) {
    // Reconstruct the full key value, preserving the sibling axis of a vec2.
    let value = current_key_value(animation, &edit.addr, edit.index, edit.axis, edit.new_val);
    if let Some(value) = value {
        // Overwrite at the (possibly unchanged) time.
        state.dispatch(Box::new(AddKey::new(
            anim,
            edit.addr.clone(),
            edit.old_time,
            value,
            interp_of(animation, &edit.addr, edit.index),
        )));
    }
    // Time change as a separate merged move.
    if (edit.new_time - edit.old_time).abs() > 1e-6 {
        let kref = KeyRef {
            addr: edit.addr,
            index: edit.index,
        };
        state.dispatch(Box::new(MoveKeys::new(
            anim,
            vec![kref],
            edit.new_time - edit.old_time,
        )));
    }
}

/// Build the full [`KeyValue`] for a key, replacing `axis` with `new_axis_val`
/// and keeping the other axis of a vec2 as authored.
fn current_key_value(
    animation: &Animation,
    addr: &TimelineAddr,
    index: usize,
    axis: usize,
    new_axis_val: f32,
) -> Option<KeyValue> {
    let timeline = animation
        .timelines
        .iter()
        .find(|t| addr.matches_timeline(t))?;
    match timeline {
        Timeline::BoneRotate { keys, .. } => {
            keys.get(index).map(|_| KeyValue::Scalar(new_axis_val))
        }
        Timeline::BoneTranslate { keys, .. }
        | Timeline::BoneScale { keys, .. }
        | Timeline::BoneShear { keys, .. } => keys.get(index).map(|k| {
            let mut v = k.value;
            if axis == 0 {
                v.x = new_axis_val;
            } else {
                v.y = new_axis_val;
            }
            KeyValue::Vec2(v)
        }),
        Timeline::SlotColor { keys, .. } => keys.get(index).map(|k| {
            let mut c = k.value;
            c[3] = new_axis_val.clamp(0.0, 1.0);
            KeyValue::Color(c)
        }),
        _ => None,
    }
}

fn interp_of(animation: &Animation, addr: &TimelineAddr, index: usize) -> Interp {
    let Some(timeline) = animation
        .timelines
        .iter()
        .find(|t| addr.matches_timeline(t))
    else {
        return Interp::Linear;
    };
    macro_rules! at {
        ($keys:expr) => {
            $keys.get(index).map(|k| k.interp).unwrap_or(Interp::Linear)
        };
    }
    match timeline {
        Timeline::BoneRotate { keys, .. } => at!(keys),
        Timeline::BoneTranslate { keys, .. } => at!(keys),
        Timeline::BoneScale { keys, .. } => at!(keys),
        Timeline::BoneShear { keys, .. } => at!(keys),
        Timeline::SlotColor { keys, .. } => at!(keys),
        _ => Interp::Linear,
    }
}

/// Sample a channel's curve across the visible time range and stroke it.
#[allow(clippy::too_many_arguments)]
fn draw_channel_curve(
    painter: &egui::Painter,
    animation: &Animation,
    addr: &TimelineAddr,
    axis: usize,
    layout: &Layout,
    rect: egui::Rect,
    val_to_y: &impl Fn(f32) -> f32,
    color: egui::Color32,
) {
    let Some(timeline) = animation
        .timelines
        .iter()
        .find(|t| addr.matches_timeline(t))
    else {
        return;
    };
    // Walk pixels across the sheet, sampling the channel value at each.
    let mut pts: Vec<egui::Pos2> = Vec::new();
    let step = 2.0_f32; // px between samples
    let mut x = rect.left();
    while x <= rect.right() {
        let t = layout.x_to_time(x).max(0.0);
        if let Some(v) = sample_channel(timeline, axis, t) {
            pts.push(egui::pos2(x, val_to_y(v)));
        }
        x += step;
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
    }
}

/// Sample one scalar channel of a timeline at `time` using core's sampler.
fn sample_channel(timeline: &Timeline, axis: usize, time: f32) -> Option<f32> {
    use ankhimate_core::animation::{sample, sample_angle_degrees};
    let mut hint = 0usize;
    match timeline {
        Timeline::BoneRotate { keys, .. } => sample_angle_degrees(keys, time, &mut hint),
        Timeline::BoneTranslate { keys, .. }
        | Timeline::BoneScale { keys, .. }
        | Timeline::BoneShear { keys, .. } => {
            sample(keys, time, &mut hint).map(|v| if axis == 0 { v.x } else { v.y })
        }
        Timeline::SlotColor { keys, .. } => sample(keys, time, &mut hint).map(|c| c[3]),
        _ => None,
    }
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
    let line = visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.4);
    let first = (left * fps).floor().max(0.0) as i64;
    let last = (right * fps).ceil() as i64;
    for frame in first..=last {
        if frame % layout.fps as i64 != 0 {
            continue;
        }
        let x = layout.time_to_x(frame as f32 / fps);
        if x < rect.left() || x > rect.right() {
            continue;
        }
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, line),
        );
    }
}

fn draw_value_axis(
    painter: &egui::Painter,
    rect: egui::Rect,
    lo: f32,
    hi: f32,
    visuals: &egui::Visuals,
) {
    // A few horizontal gridlines with value labels.
    let steps = 4;
    for i in 0..=steps {
        let f = i as f32 / steps as f32;
        let y = rect.bottom() - f * rect.height();
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(
                1.0,
                visuals
                    .widgets
                    .noninteractive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.3),
            ),
        );
        let v = lo + f * (hi - lo);
        painter.text(
            egui::pos2(rect.left() + 3.0, y - 1.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{v:.1}"),
            egui::FontId::proportional(9.0),
            visuals.weak_text_color(),
        );
    }
}

fn placeholder(painter: &egui::Painter, rect: egui::Rect, visuals: &egui::Visuals) {
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "No numeric channels to graph",
        egui::FontId::proportional(12.0),
        visuals.weak_text_color(),
    );
}

/// Set a key at the playhead for the active bone's active transform property,
/// capturing the current pose as an offset from setup (shared by dopesheet and
/// graph; lives here to keep one copy).
pub fn set_key_for_active_bone(state: &mut AppState, anim: AnimationId) {
    let Some(bone) = state.session.active_bone() else {
        return;
    };
    let Some(setup) = state
        .doc
        .skeleton
        .bones
        .get(bone)
        .map(|b| b.local_transform)
    else {
        return;
    };
    let Some(local) = state.pose.locals.get(bone).copied() else {
        return;
    };
    let time = state.session.playhead;

    let (addr, value) = match state.session.active_transform_tool {
        crate::session::TransformTool::Translate => (
            TimelineAddr::Bone {
                bone,
                property: BoneProperty::Translate,
            },
            KeyValue::Vec2(local.position - setup.position),
        ),
        crate::session::TransformTool::Rotate => (
            TimelineAddr::Bone {
                bone,
                property: BoneProperty::Rotate,
            },
            KeyValue::Scalar(
                ankhimate_core::transforms::wrap_angle(local.rotation - setup.rotation)
                    .to_degrees(),
            ),
        ),
        crate::session::TransformTool::Scale => (
            TimelineAddr::Bone {
                bone,
                property: BoneProperty::Scale,
            },
            KeyValue::Vec2(glam::vec2(
                safe_div(local.scale.x, setup.scale.x),
                safe_div(local.scale.y, setup.scale.y),
            )),
        ),
        crate::session::TransformTool::Shear => (
            TimelineAddr::Bone {
                bone,
                property: BoneProperty::Shear,
            },
            // Degrees on the wire, like rotate (ADR 0002).
            KeyValue::Vec2(glam::vec2(
                (local.shear.x - setup.shear.x).to_degrees(),
                (local.shear.y - setup.shear.y).to_degrees(),
            )),
        ),
    };
    state.dispatch(Box::new(AddKey::new(
        anim,
        addr,
        time,
        value,
        Interp::Linear,
    )));
}

fn safe_div(a: f32, b: f32) -> f32 {
    if b.abs() < 1e-6 { 1.0 } else { a / b }
}
