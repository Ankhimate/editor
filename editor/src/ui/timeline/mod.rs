//! Spine-style timeline: a folded name tree, a dopesheet / graph sheet, a frame
//! ruler, and a transport bar (T-201..T-206 + graph-editor polish).
//!
//! ```text
//! ┌ clip picker · transport · [Dopesheet|Graph] ─────────────────────────────┐
//! ├ frame ruler ─────────────────────────────────────────────────────────────┤
//! │ name tree        ┊  key sheet (diamonds) or graph (bezier curves)         │
//! │  ▼ arm           ┊     ◆      ◆         ◆                                  │
//! │    rotate        ┊     ◆      ◆         ◆                                  │
//! └──────────────────┴──────────────────────────────────────────────────────┘
//! ```
//!
//! The tree and the sheet share horizontal/vertical geometry through [`Layout`]
//! so a key lines up with its ruler tick and its row label. Zoom, scroll, the
//! divider position, the dopesheet/graph mode, and per-group fold state are UI
//! state kept in egui memory — never in `Session`/undo.

mod events;
mod graph;
mod model;
mod ruler;
mod sheet;
mod transport;
mod tree;

use crate::app_state::AppState;
use eframe::egui;

pub use transport::toggle_play;

/// Row height for every property row and group header.
pub const ROW_H: f32 = 20.0;
/// Height of the frame ruler.
pub const RULER_H: f32 = 24.0;

const MIN_DIVIDER: f32 = 90.0;
const MIN_PX_PER_SEC: f32 = 12.0;
const MAX_PX_PER_SEC: f32 = 1400.0;

/// Which view the sheet is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SheetMode {
    Dopesheet,
    Graph,
}

/// Shared horizontal geometry between the ruler, tree, and sheet.
#[derive(Clone, Copy)]
pub struct Layout {
    /// Left edge of the sheet in screen px (= panel left + divider width).
    pub sheet_x0: f32,
    /// Screen px per second.
    pub px_per_sec: f32,
    /// Time (seconds) at the sheet's left edge.
    pub scroll_sec: f32,
    pub fps: u32,
}

impl Layout {
    pub fn time_to_x(&self, time: f32) -> f32 {
        self.sheet_x0 + (time - self.scroll_sec) * self.px_per_sec
    }
    pub fn x_to_time(&self, x: f32) -> f32 {
        self.scroll_sec + (x - self.sheet_x0) / self.px_per_sec
    }
    pub fn snap_time(&self, time: f32) -> f32 {
        let frame = (time * self.fps as f32).round();
        (frame / self.fps as f32).max(0.0)
    }
}

/// Persisted view state (zoom, scroll, divider, mode). UI-only.
#[derive(Clone, Copy)]
pub struct ViewState {
    pub px_per_sec: f32,
    pub scroll_sec: f32,
    pub divider_w: f32,
    pub mode: SheetMode,
    /// Vertical scroll offset (px) shared by tree and sheet.
    pub scroll_y: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            px_per_sec: 120.0,
            scroll_sec: 0.0,
            divider_w: 168.0,
            mode: SheetMode::Dopesheet,
            scroll_y: 0.0,
        }
    }
}

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    // Setup mode has no playhead semantics (the viewport shows the setup pose
    // whatever the timeline says), so the panel collapses to an invitation
    // rather than showing a dopesheet that cannot drive anything (T-207).
    if !state.session.is_animating() {
        setup_mode_placeholder(ui, state);
        return;
    }
    if state.session.active_animation.is_none() {
        clip_chooser(ui, state);
        return;
    }
    let anim_id = state.session.active_animation.unwrap();

    let view_id = ui.id().with("tl_view");
    let mut view: ViewState = ui
        .ctx()
        .memory(|m| m.data.get_temp(view_id))
        .unwrap_or_default();

    // ── Header: clip + transport + mode toggle ───────────────────────────
    header(ui, state, &mut view);
    ui.separator();

    // Build the row model once for tree + sheet.
    let model = model::TimelineModel::build(state, anim_id);

    // Wheel handling over the whole body.
    //   ctrl+wheel → zoom the time axis, anchored under the cursor
    //   plain wheel → scroll time left/right
    // (middle-drag on the sheet also pans; a bottom scrollbar mirrors position.)
    let body = ui.available_rect_before_wrap();
    let sheet_x0_now = body.left() + view.divider_w;
    if ui.rect_contains_pointer(body) {
        let (dy, dx, modifiers, pointer_x) = ui.ctx().input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.smooth_scroll_delta.x,
                i.modifiers,
                i.pointer.hover_pos().map(|p| p.x),
            )
        });
        if modifiers.ctrl && dy != 0.0 {
            // Zoom, keeping the time under the cursor fixed.
            let px = pointer_x.unwrap_or(sheet_x0_now);
            let time_at_cursor = view.scroll_sec + (px - sheet_x0_now) / view.px_per_sec;
            let factor = (dy * 0.0025).exp();
            let new_pps = (view.px_per_sec * factor).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
            view.scroll_sec = (time_at_cursor - (px - sheet_x0_now) / new_pps).max(0.0);
            view.px_per_sec = new_pps;
        } else {
            // Plain wheel scrolls time. Vertical wheel maps to horizontal because
            // most mice have no horizontal wheel; a real hwheel (dx) also works.
            let scroll = if dx != 0.0 { dx } else { dy };
            if scroll != 0.0 {
                view.scroll_sec = (view.scroll_sec - scroll / view.px_per_sec).max(0.0);
            }
        }
    }

    let sheet_x0 = body.left() + view.divider_w;
    let layout = Layout {
        sheet_x0,
        px_per_sec: view.px_per_sec,
        scroll_sec: view.scroll_sec,
        fps: state.doc.meta.fps.max(1),
    };

    // ── Ruler across the sheet ───────────────────────────────────────────
    let ruler_rect = egui::Rect::from_min_size(body.min, egui::vec2(body.width(), RULER_H));
    ruler::ui(ui, state, &layout, ruler_rect);

    // ── Event lane, directly under the ruler (T-506) ─────────────────────
    // Events belong to the clip rather than to any bone, so they get a lane of
    // their own above the dopesheet instead of a row inside its group tree.
    let event_rect = egui::Rect::from_min_size(
        egui::pos2(body.left(), ruler_rect.bottom()),
        egui::vec2(body.width(), events::LANE_HEIGHT),
    );
    events::ui(ui, state, &layout, event_rect);

    // ── Body: tree | divider | sheet ─────────────────────────────────────
    let body_rect =
        egui::Rect::from_min_max(egui::pos2(body.left(), event_rect.bottom()), body.max);
    let tree_rect = egui::Rect::from_min_max(
        body_rect.min,
        egui::pos2(sheet_x0 - 1.0, body_rect.bottom()),
    );
    let sheet_rect = egui::Rect::from_min_max(egui::pos2(sheet_x0, body_rect.min.y), body_rect.max);

    tree::ui(ui, state, &model, &mut view, tree_rect);

    match view.mode {
        SheetMode::Dopesheet => {
            sheet::ui(ui, state, anim_id, &model, &mut view, &layout, sheet_rect)
        }
        SheetMode::Graph => graph::ui(ui, state, anim_id, &model, &view, &layout, sheet_rect),
    }

    // Draggable divider between tree and sheet.
    divider(ui, state, &mut view, body_rect, sheet_x0);

    ui.ctx().memory_mut(|m| m.data.insert_temp(view_id, view));
}

/// The header row above the ruler.
fn header(ui: &mut egui::Ui, state: &mut AppState, view: &mut ViewState) {
    ui.horizontal(|ui| {
        pick_clip_combo(ui, state);
        if ui
            .button(egui_phosphor::regular::PLUS)
            .on_hover_text("New animation")
            .clicked()
        {
            create_clip(state);
        }
        clip_menu(ui, state);
        ui.separator();
        transport::ui(ui, state);
        ui.separator();
        // Dopesheet / Graph toggle.
        if ui
            .selectable_label(view.mode == SheetMode::Dopesheet, "Dopesheet")
            .clicked()
        {
            view.mode = SheetMode::Dopesheet;
        }
        if ui
            .selectable_label(view.mode == SheetMode::Graph, "Graph")
            .clicked()
        {
            view.mode = SheetMode::Graph;
        }

        ui.separator();
        // Zoom controls — reliable regardless of wheel focus.
        if ui
            .button(egui_phosphor::regular::MAGNIFYING_GLASS_MINUS)
            .on_hover_text("Zoom out (Ctrl+scroll)")
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec / 1.4).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
        }
        if ui
            .button(egui_phosphor::regular::MAGNIFYING_GLASS_PLUS)
            .on_hover_text("Zoom in (Ctrl+scroll)")
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec * 1.4).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
        }
        // Percentage readout doubles as a fit-to-clip button.
        let frame_px = view.px_per_sec / state.doc.meta.fps.max(1) as f32;
        if ui
            .button(format!("{frame_px:.0}px/f"))
            .on_hover_text("Fit clip to view")
            .clicked()
        {
            fit_to_clip(state, view);
        }
    });
}

/// Set zoom + scroll so the whole active clip fills the sheet width.
fn fit_to_clip(state: &AppState, view: &mut ViewState) {
    let Some(dur) = state
        .session
        .active_animation
        .and_then(|id| state.doc.animations.get(id))
        .map(|a| a.duration.max(0.1))
    else {
        return;
    };
    // Approximate sheet width: total minus the name column. Good enough — the
    // next frame's real width refines nothing visible.
    let approx_sheet_w = 900.0 - view.divider_w;
    view.px_per_sec = (approx_sheet_w / dur).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
    view.scroll_sec = 0.0;
}

/// The draggable vertical divider that resizes the name tree.
fn divider(
    ui: &mut egui::Ui,
    _state: &mut AppState,
    view: &mut ViewState,
    body_rect: egui::Rect,
    sheet_x0: f32,
) {
    let handle = egui::Rect::from_min_max(
        egui::pos2(sheet_x0 - 3.0, body_rect.top()),
        egui::pos2(sheet_x0 + 3.0, body_rect.bottom()),
    );
    let resp = ui.interact(handle, ui.id().with("tl_divider"), egui::Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if resp.dragged() {
        view.divider_w =
            (view.divider_w + resp.drag_delta().x).clamp(MIN_DIVIDER, body_rect.width() - 120.0);
    }
    let color = if resp.hovered() || resp.dragged() {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    ui.painter().line_segment(
        [
            egui::pos2(sheet_x0 - 1.0, body_rect.top()),
            egui::pos2(sheet_x0 - 1.0, body_rect.bottom()),
        ],
        egui::Stroke::new(1.0, color),
    );
}

// ── Clip selection / creation (shared with the empty state) ──────────────────

/// What the timeline shows in Setup mode: the way into Animate mode.
fn setup_mode_placeholder(ui: &mut egui::Ui, state: &mut AppState) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 2.0 - 40.0);
        ui.label(
            egui::RichText::new(egui_phosphor::regular::FILM_STRIP)
                .size(28.0)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Setup mode — building the rig")
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("Edits change the setup pose. Switch to Animate to key them.")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(8.0);
        if ui.button("Animate  (Tab)").clicked() {
            state.set_work_mode(crate::session::WorkMode::Animate);
        }
    });
}

fn clip_chooser(ui: &mut egui::Ui, state: &mut AppState) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() / 2.0 - 40.0);
        ui.label(
            egui::RichText::new(egui_phosphor::regular::FILM_STRIP)
                .size(28.0)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);
        if state.doc.animations.is_empty() {
            ui.label(
                egui::RichText::new("No animations yet").color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
            if ui.button("New animation").clicked() {
                create_clip(state);
            }
        } else {
            ui.label(
                egui::RichText::new("Select an animation").color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
            pick_clip_combo(ui, state);
        }
    });
}

fn pick_clip_combo(ui: &mut egui::Ui, state: &mut AppState) {
    let current = state
        .session
        .active_animation
        .and_then(|id| state.doc.animations.get(id))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "—".to_string());

    egui::ComboBox::from_id_salt("clip_combo")
        .selected_text(current)
        .show_ui(ui, |ui| {
            let clips: Vec<_> = state
                .doc
                .animations
                .iter()
                .map(|(id, a)| (id, a.name.clone()))
                .collect();
            for (id, name) in clips {
                let selected = state.session.active_animation == Some(id);
                if ui.selectable_label(selected, name).clicked() {
                    state.session.active_animation = Some(id);
                    state.set_playhead(0.0);
                    // Picking a clip is a statement of intent — go where editing
                    // it actually does something.
                    state.set_work_mode(crate::session::WorkMode::Animate);
                }
            }
        });
}

fn create_clip(state: &mut AppState) {
    state.create_animation();
    state.set_work_mode(crate::session::WorkMode::Animate);
}

/// Manage the active clip: rename, duration, loop, duplicate, delete (T-208).
///
/// A menu rather than a modal — every action here is one click deep and
/// undoable, so a dialog would only add ceremony.
fn clip_menu(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::commands::key_cmds::{
        DeleteAnimation, DuplicateAnimation, RenameAnimation, SetAnimationMeta,
    };

    let Some(anim_id) = state.session.active_animation else {
        return;
    };
    let Some(anim) = state.doc.animations.get(anim_id) else {
        return;
    };
    let (name, duration, looping) = (anim.name.clone(), anim.duration, anim.looping);
    let fps = state.doc.meta.fps.max(1) as f32;

    // Collected inside the menu, applied after it closes: dispatching mid-menu
    // would borrow `state` while the closure still holds it.
    let mut rename: Option<String> = None;
    let mut meta: Option<(f32, bool)> = None;
    let mut duplicate = false;
    let mut delete = false;

    ui.menu_button(egui_phosphor::regular::DOTS_THREE, |ui| {
        ui.set_min_width(220.0);

        ui.horizontal(|ui| {
            ui.label("Name");
            let mut edited = name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut edited).desired_width(140.0))
                .lost_focus()
                && edited != name
                && !edited.trim().is_empty()
            {
                rename = Some(edited.trim().to_string());
            }
        });

        // Duration is authored in frames — that is the unit the dopesheet, the
        // ruler and the user all think in; seconds are the storage detail.
        ui.horizontal(|ui| {
            ui.label("Frames");
            let mut frames = (duration * fps).round().max(1.0);
            if ui
                .add(
                    egui::DragValue::new(&mut frames)
                        .range(1.0..=100_000.0)
                        .speed(1.0),
                )
                .on_hover_text("Shortening keeps keys past the end — they are flagged, not deleted")
                .changed()
            {
                meta = Some((frames / fps, looping));
            }
        });

        let mut loop_flag = looping;
        if ui
            .checkbox(&mut loop_flag, "Loops")
            .on_hover_text("Authoring intent, carried to the runtime")
            .changed()
        {
            meta = Some((duration, loop_flag));
        }

        ui.separator();
        if ui.button("Duplicate").clicked() {
            duplicate = true;
            ui.close();
        }
        if ui.button("Delete").clicked() {
            delete = true;
            ui.close();
        }
    });

    if let Some(new_name) = rename {
        state.dispatch(Box::new(RenameAnimation::new(anim_id, new_name)));
    }
    if let Some((duration, looping)) = meta {
        state.dispatch(Box::new(SetAnimationMeta::new(anim_id, duration, looping)));
    }
    if duplicate {
        state.dispatch(Box::new(DuplicateAnimation::new(anim_id)));
        // Select the copy: duplicating is how you start a variation, so that is
        // what you want to be editing.
        if let Some((id, _)) = state.doc.animations.iter().last() {
            state.session.active_animation = Some(id);
            state.set_playhead(0.0);
        }
    }
    if delete {
        state.dispatch(Box::new(DeleteAnimation::new(anim_id)));
        state.session.active_animation = state.doc.animations.iter().next().map(|(id, _)| id);
        state.set_playhead(0.0);
        if state.session.active_animation.is_none() {
            state.set_work_mode(crate::session::WorkMode::Setup);
        }
    }
}
