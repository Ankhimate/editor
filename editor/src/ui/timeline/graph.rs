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
//! off the key before it (`out_handle`) and the key itself (`in_handle`), as
//! fractions of that segment. Time is 0..1; value is not, and a handle past
//! either key is an overshoot (see `docs/graph-editor.md` rules 3 and 4).
//!
//! Value editing rebuilds the whole key value (both axes of a vec2) so the
//! sibling channel is preserved.

use super::model::{TimelineModel, VisibleRow};
use super::{Layout, ViewState};
use crate::app_state::AppState;
use crate::commands::key_cmds::{AddKey, BoneProperty, KeyRef, KeyValue, MoveKeys, TimelineAddr};
use ankhimate_core::animation::{Animation, Axis, Interp, Timeline};
use ankhimate_core::ids::AnimationId;
use eframe::egui;

const KEY_R: f32 = 4.0;

/// The colour for one channel: the property's colour, shaded by axis.
///
/// Colouring by axis alone — x red, y green — meant a green curve was "the y
/// one" in one row and "the rotate one" in the next, so the colour carried no
/// meaning across the panel. Property first; the second axis of a two-channel
/// property is the same hue, darker, which tells x from y without inventing a
/// second vocabulary.
fn channel_color(theme: &crate::theme::Theme, property: &str, axis: usize) -> egui::Color32 {
    let base = theme.channel_color(property);
    if axis == 1 {
        base.gamma_multiply(0.62)
    } else {
        base
    }
}

// Eight parameters, all of them distinct things the panel needs and none of
// which group into a struct that would mean anything on its own.
#[allow(clippy::too_many_arguments)]
pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    anim: AnimationId,
    model: &TimelineModel,
    view: &ViewState,
    layout: &Layout,
    rect: egui::Rect,
    style: super::Style<'_>,
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

    // Fit to the *curve*, not only to the keys (T-912).
    //
    // A bezier whose handles are steep enough overshoots the values it
    // interpolates between, and framing on key values alone drew that overshoot
    // outside the panel — the curve simply left the top of the view with nothing
    // to say it had. "A selected curve is never invisible" is the acceptance bar
    // for this task, and it cannot be met by looking at keys.
    //
    // Sampled at the same 2px step the curves are drawn at, so what is measured
    // is exactly what is about to be stroked.
    let key_lo = lo;
    let key_hi = hi;
    if let Some(animation) = state.doc.animations.get(anim) {
        for row in &rows {
            if !row.is_soloed(&view.soloed) {
                continue;
            }
            let VisibleRow::Property { data, .. } = row else {
                continue;
            };
            if data.read_only {
                continue;
            }
            let Some(timeline) = animation
                .timelines
                .iter()
                .find(|t| data.addr.matches_timeline(t))
            else {
                continue;
            };
            {
                let mut x = rect.left();
                while x <= rect.right() {
                    if let Some(v) = sample_channel(timeline, layout.x_to_time(x).max(0.0)) {
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                    x += 2.0;
                }
            }
        }
    }
    // How far past its keys the curve travels, as a fraction of the keyed range.
    // Reported below rather than clamped: an overshoot is a legitimate thing to
    // author — it is how a bounce is made — and silently flattening one would be
    // worse than the invisibility this replaces.
    let keyed_span = (key_hi - key_lo).abs().max(1e-6);
    let overshoot = ((key_lo - lo).max(0.0) + (hi - key_hi).max(0.0)) / keyed_span;

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

    // Say when a curve travels well past the values it interpolates (T-912).
    //
    // An overshooting tangent has to be visually distinct from a well-behaved
    // one: it is the difference between a bounce someone authored and a handle
    // dragged too far by accident, and the curve alone does not distinguish
    // them once the view has been re-framed to contain it. A tenth of the keyed
    // range is the threshold — below that, an ease that peeks past its key is
    // ordinary and saying so would be noise.
    if overshoot > 0.1 {
        painter.text(
            egui::pos2(rect.right() - 6.0, rect.top() + 4.0),
            egui::Align2::RIGHT_TOP,
            format!("overshoot {:.0}%", overshoot * 100.0),
            egui::FontId::proportional(10.0),
            visuals.warn_fg_color,
        );
    }

    // Snapping state, always on screen (T-912). Silent snapping is the specific
    // complaint; a mode the user cannot see is a mode they cannot reason about
    // when a key refuses to sit where they put it.
    let free = ui.input(|i| i.modifiers.alt);
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        if free { "free" } else { "snap: frames" },
        egui::FontId::proportional(10.0),
        if free {
            egui::Color32::from_rgb(120, 190, 255)
        } else {
            visuals.weak_text_color()
        },
    );

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
            let color = channel_color(style.theme, data.label, ch.axis);
            // Sample the curve across the visible range for a smooth plot.
            draw_channel_curve(
                &painter, &animation, &data.addr, layout, rect, &val_to_y, color,
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
                    // Frame snapping, escapable with Alt (T-912). It used to be
                    // unconditional and unannounced, which is the "unexpected
                    // snapping" the reference tool's graph editor is repeatedly
                    // reported for: a key nudged a third of a frame springs back
                    // and the editor never says why. The badge below states the
                    // mode; Alt is the way out of it.
                    let raw = layout.x_to_time(p.x);
                    let new_time = if ui.input(|i| i.modifiers.alt) {
                        raw.max(0.0)
                    } else {
                        layout.snap_time(raw).max(0.0)
                    };
                    pending = Some(Edit {
                        addr: data.addr.clone(),
                        index: k.index,
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

/// Where to draw a handle so it stays inside the panel, and whether it moved.
///
/// The view frames the *sampled curve* (rule 1), and a cubic does not pass
/// through its control points — so a handle shaping a legitimate overshoot can
/// land outside the panel. Drawn there it would be unreachable: the hit box is
/// gated on the panel rect, so the handle could not be dragged back and the only
/// way out would be undo.
///
/// Pinning moves the *marker*, never the value. The flag lets the caller draw a
/// pinned handle differently, which is the same contract as rule 3 — report,
/// do not clamp.
fn pinned(true_pos: egui::Pos2, rect: egui::Rect, radius: f32) -> (egui::Pos2, bool) {
    let inset = rect.shrink(radius + 1.0);
    // A panel narrower than the inset would invert the range; `max` keeps the
    // clamp well-formed rather than producing a NaN.
    let x = true_pos
        .x
        .clamp(inset.left(), inset.right().max(inset.left()));
    let y = true_pos
        .y
        .clamp(inset.top(), inset.bottom().max(inset.top()));
    let at = egui::pos2(x, y);
    ((at), (at - true_pos).length() > 0.5)
}

/// Draw and drag both handles of every segment in one channel.
///
/// Handles live as fractions of the segment they belong to, so they are
/// converted to and from graph coordinates here. Time is clamped to 0..1 on
/// drag; value is deliberately free, and a handle past either key overshoots. A
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
            // Drawn inside the panel even when the handle itself is not: an
            // overshoot puts a control point past the curve it shapes, and a
            // marker drawn off-panel cannot be grabbed to undo it. The stored
            // value is untouched — only the marker moves.
            let (pos, is_pinned) = pinned(to_screen(handle), rect, HANDLE_R);
            let stem = anchors[which];
            painter.line_segment(
                [stem, pos],
                egui::Stroke::new(1.0, color.gamma_multiply(0.45)),
            );
            if is_pinned {
                // Hollow, so a handle resting at the edge is not mistaken for
                // one that genuinely sits there. The channel's own colour keeps
                // it readable against whichever curve it belongs to.
                painter.circle_stroke(
                    pos,
                    HANDLE_R + 1.0,
                    egui::Stroke::new(1.5, color.gamma_multiply(0.9)),
                );
            } else {
                painter.circle_filled(pos, HANDLE_R, color.gamma_multiply(0.8));
            }

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
    new_val: f32,
    new_time: f32,
    old_time: f32,
}

/// Apply a graph key drag: value change via overwrite `AddKey`, time change via
/// `MoveKeys`.
fn apply_edit(state: &mut AppState, anim: AnimationId, animation: &Animation, edit: Edit) {
    // Reconstruct the full key value, preserving the sibling axis of a vec2.
    let value = current_key_value(animation, &edit.addr, edit.index, edit.new_val);
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

/// Build the [`KeyValue`] for a key from the value a drag produced.
///
/// A bone track drives one number, so the dragged value *is* the key's value.
/// This used to fetch the existing pair and overwrite one axis of it, which
/// existed only because one keyframe held both — dragging y had to re-state x
/// to avoid erasing it.
///
/// Colour is still packed, so its alpha channel keeps the read-modify-write.
fn current_key_value(
    animation: &Animation,
    addr: &TimelineAddr,
    index: usize,
    new_val: f32,
) -> Option<KeyValue> {
    let timeline = animation
        .timelines
        .iter()
        .find(|t| addr.matches_timeline(t))?;
    match timeline {
        Timeline::BoneRotate { keys, .. }
        | Timeline::BoneTranslate { keys, .. }
        | Timeline::BoneScale { keys, .. }
        | Timeline::BoneShear { keys, .. } => keys.get(index).map(|_| KeyValue::Scalar(new_val)),
        Timeline::SlotColor { keys, .. } => keys.get(index).map(|k| {
            let mut c = k.value;
            c[3] = new_val.clamp(0.0, 1.0);
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
        if let Some(v) = sample_channel(timeline, t) {
            pts.push(egui::pos2(x, val_to_y(v)));
        }
        x += step;
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, egui::Stroke::new(1.5, color)));
    }
}

/// Sample a timeline at `time` using core's sampler.
///
/// Every bone track is one scalar channel now, so there is no axis to pick.
fn sample_channel(timeline: &Timeline, time: f32) -> Option<f32> {
    use ankhimate_core::animation::{sample, sample_angle_degrees};
    let mut hint = 0usize;
    match timeline {
        Timeline::BoneRotate { keys, .. } => sample_angle_degrees(keys, time, &mut hint),
        Timeline::BoneTranslate { keys, .. }
        | Timeline::BoneScale { keys, .. }
        | Timeline::BoneShear { keys, .. } => sample(keys, time, &mut hint),
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
    let Some(_setup) = state
        .doc
        .skeleton
        .bones
        .get(bone)
        .map(|b| b.local_transform)
    else {
        return;
    };
    let Some(_local) = state.pose.locals.get(bone).copied() else {
        return;
    };
    let time = state.session.playhead;

    let property = match state.session.active_transform_tool {
        crate::session::TransformTool::Translate => BoneProperty::Translate,
        crate::session::TransformTool::Rotate => BoneProperty::Rotate,
        crate::session::TransformTool::Scale => BoneProperty::Scale,
        crate::session::TransformTool::Shear => BoneProperty::Shear,
    };
    // Both tracks of a two-axis property, so "key this bone" still keys the
    // whole property. `bone_key_value` is the one definition of what a posed
    // value means as a key — this used to hold a second copy of it, and the two
    // could disagree.
    let axes: &[Option<Axis>] = match property {
        BoneProperty::Rotate => &[None],
        _ => &[Some(Axis::X), Some(Axis::Y)],
    };
    for &axis in axes {
        let addr = TimelineAddr::Bone {
            bone,
            property,
            axis,
        };
        let Some(value) =
            crate::edit_router::bone_key_value(&state.doc, &state.pose, bone, property, axis)
        else {
            continue;
        };
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr,
            time,
            value,
            Interp::Linear,
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(400.0, 250.0))
    }

    /// A handle inside the panel is drawn where it is.
    #[test]
    fn a_handle_in_view_is_not_moved() {
        let (at, moved) = pinned(egui::pos2(200.0, 150.0), panel(), HANDLE_R);
        assert_eq!(at, egui::pos2(200.0, 150.0));
        assert!(!moved);
    }

    /// A handle above or below the panel is pulled back to the edge.
    ///
    /// The whole point: the grab box is gated on the panel rect, so a marker
    /// drawn outside it cannot be dragged. An overshoot puts a control point
    /// past the curve it shapes, which is exactly when this happens.
    #[test]
    fn a_handle_out_of_view_is_pulled_to_the_edge_and_flagged() {
        let rect = panel();
        for y in [-500.0_f32, 1000.0] {
            let (at, moved) = pinned(egui::pos2(200.0, y), rect, HANDLE_R);
            assert!(moved, "a handle at y={y} is outside {rect:?}");
            assert!(
                rect.contains(at),
                "pinned to {at:?}, which is still outside {rect:?}"
            );
            // And its grab box overlaps the panel, which is what makes it
            // reachable at all.
            let hit = egui::Rect::from_center_size(at, egui::vec2(12.0, 12.0));
            assert!(rect.intersects(hit));
        }
    }

    /// Pinning is inset far enough that the whole marker stays visible.
    #[test]
    fn a_pinned_handle_is_not_half_off_the_edge() {
        let rect = panel();
        let (at, _) = pinned(egui::pos2(200.0, -500.0), rect, HANDLE_R);
        assert!(
            at.y - HANDLE_R >= rect.top(),
            "the marker's top edge clips: {at:?}"
        );
    }

    /// A degenerate panel produces a position rather than a NaN.
    #[test]
    fn a_panel_thinner_than_the_marker_still_yields_a_point() {
        let sliver = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(2.0, 2.0));
        let (at, _) = pinned(egui::pos2(50.0, 50.0), sliver, HANDLE_R);
        assert!(at.x.is_finite() && at.y.is_finite(), "{at:?}");
    }
}
