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
const MIN_PX_PER_SEC: f32 = 4.0;
/// 100 px per frame at 60 fps. The old ceiling of 1400 was about 47 px/f at 30,
/// which is not enough to separate two keys a frame apart on a long clip.
const MAX_PX_PER_SEC: f32 = 6000.0;

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
#[derive(Clone)]
pub struct ViewState {
    pub px_per_sec: f32,
    pub scroll_sec: f32,
    pub divider_w: f32,
    pub mode: SheetMode,
    /// Vertical scroll offset (px) shared by tree and sheet.
    pub scroll_y: f32,
    /// Rows shown on their own, keyed by [`VisibleRow::solo_id`].
    ///
    /// Empty means "show everything" rather than "show nothing": a solo set that
    /// emptied itself into a blank sheet would be a trap, and the last row
    /// un-soloed is the commonest way to empty it.
    pub soloed: std::collections::BTreeSet<u64>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            px_per_sec: 120.0,
            scroll_sec: 0.0,
            divider_w: 168.0,
            mode: SheetMode::Dopesheet,
            scroll_y: 0.0,
            soloed: Default::default(),
        }
    }
}

/// Everything the timeline paints itself with: colours by channel, and one text
/// size for a panel that draws sixty rows of its own labels.
#[derive(Clone, Copy)]
pub struct Style<'a> {
    pub theme: &'a crate::theme::Theme,
    pub text: f32,
}

/// The dopesheet pane.
pub fn dopesheet(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &crate::theme::Theme,
    fonts: &crate::config::FontSettings,
) {
    let style = Style {
        theme,
        text: fonts.for_area(crate::config::Area::Timeline),
    };
    panel(ui, state, SheetMode::Dopesheet, style)
}

/// The graph pane.
///
/// A separate pane rather than a toggle inside one panel: they answer different
/// questions — *when* does something happen versus *how* does it get there — and
/// an animator wants both on screen at once, which a toggle makes impossible.
pub fn graph_view(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &crate::theme::Theme,
    fonts: &crate::config::FontSettings,
) {
    let style = Style {
        theme,
        text: fonts.for_area(crate::config::Area::Timeline),
    };
    panel(ui, state, SheetMode::Graph, style)
}

fn panel(ui: &mut egui::Ui, state: &mut AppState, mode: SheetMode, style: Style<'_>) {
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

    // One shared view state, keyed on the context rather than on the pane: the
    // dopesheet and the graph scroll and zoom together, which is the whole point
    // of having them side by side.
    let view_id = egui::Id::new("tl_view");
    let mut view: ViewState = ui
        .ctx()
        .memory(|m| m.data.get_temp(view_id))
        .unwrap_or_default();
    view.mode = mode;

    // ── Header: transport and view controls ──────────────────────────────
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
        let (dy, dx, modifiers, pointer_x, zoom) = ui.ctx().input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.smooth_scroll_delta.x,
                i.modifiers,
                i.pointer.hover_pos().map(|p| p.x),
                i.zoom_delta(),
            )
        });
        // A trackpad pinch arrives as a zoom gesture, not as ctrl+wheel, so it
        // needs its own path or the gesture does nothing at all.
        if (zoom - 1.0).abs() > 1e-4 {
            let px = pointer_x.unwrap_or(sheet_x0_now);
            let time_at_cursor = view.scroll_sec + (px - sheet_x0_now) / view.px_per_sec;
            let new_pps = (view.px_per_sec * zoom).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
            view.scroll_sec = (time_at_cursor - (px - sheet_x0_now) / new_pps).max(0.0);
            view.px_per_sec = new_pps;
        } else if modifiers.ctrl && dy != 0.0 {
            // Zoom, keeping the time under the cursor fixed.
            let px = pointer_x.unwrap_or(sheet_x0_now);
            let time_at_cursor = view.scroll_sec + (px - sheet_x0_now) / view.px_per_sec;
            let factor = (dy * 0.0025).exp();
            let new_pps = (view.px_per_sec * factor).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
            view.scroll_sec = (time_at_cursor - (px - sheet_x0_now) / new_pps).max(0.0);
            view.px_per_sec = new_pps;
        } else if modifiers.shift || dx != 0.0 {
            // Shift-wheel, or a real horizontal wheel, scrolls time.
            let scroll = if dx != 0.0 { dx } else { dy };
            if scroll != 0.0 {
                view.scroll_sec = (view.scroll_sec - scroll / view.px_per_sec).max(0.0);
            }
        } else if dy != 0.0 {
            // Plain wheel scrolls the rows. It used to scroll *time*, which left
            // the row list with no way to move at all — on a rig with sixty rows
            // everything below the fold was unreachable.
            view.scroll_y = (view.scroll_y - dy).max(0.0);
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
    ruler::ui(ui, state, &layout, ruler_rect, style);

    // ── Event lane, directly under the ruler (T-506) ─────────────────────
    // Events belong to the clip rather than to any bone, so they get a lane of
    // their own above the dopesheet instead of a row inside its group tree.
    let event_rect = egui::Rect::from_min_size(
        egui::pos2(body.left(), ruler_rect.bottom()),
        egui::vec2(body.width(), events::LANE_HEIGHT),
    );
    events::ui(ui, state, &layout, event_rect, style);

    // ── Body: tree | divider | sheet ─────────────────────────────────────
    let body_rect =
        egui::Rect::from_min_max(egui::pos2(body.left(), event_rect.bottom()), body.max);
    let tree_rect = egui::Rect::from_min_max(
        body_rect.min,
        egui::pos2(sheet_x0 - 1.0, body_rect.bottom()),
    );
    let sheet_rect = egui::Rect::from_min_max(egui::pos2(sheet_x0, body_rect.min.y), body_rect.max);

    tree::ui(ui, state, &model, &mut view, tree_rect, style);

    match view.mode {
        SheetMode::Dopesheet => sheet::ui(
            ui, state, anim_id, &model, &mut view, &layout, sheet_rect, style,
        ),
        SheetMode::Graph => graph::ui(
            ui, state, anim_id, &model, &view, &layout, sheet_rect, style,
        ),
    }

    // Draggable divider between tree and sheet.
    divider(ui, state, &mut view, body_rect, sheet_x0);

    ui.ctx().memory_mut(|m| m.data.insert_temp(view_id, view));
}

/// The header row above the ruler.
///
/// Grouped rather than a single run of buttons: transport, then what is being
/// keyed, then zoom. The clip picker used to live here and does not any more —
/// choosing which animation to work on is the Animations pane's job, and having
/// it in two places meant two things to keep in sync and one more control
/// between the user and the play button.
fn header(ui: &mut egui::Ui, state: &mut AppState, view: &mut ViewState) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.add_space(2.0);

        // The clip's name, as a label. Read-only on purpose: it says what you are
        // editing without being a third place to change it.
        let name = state
            .session
            .active_animation
            .and_then(|id| state.doc.animations.get(id))
            .map(|a| a.name.clone())
            .unwrap_or_default();
        ui.label(
            egui::RichText::new(format!("{} {name}", crate::ui::icons::ANIMATIONS))
                .strong()
                .size(11.5),
        );

        // Soloing is a mode with no other exit: the dots are per row, and a rig
        // with sixty of them makes "which one did I click" a real question.
        if !view.soloed.is_empty() {
            let count = view.soloed.len();
            if ui
                .button(
                    egui::RichText::new(format!("{} solo {count}", crate::ui::icons::DOT_ON))
                        .color(ui.visuals().selection.bg_fill)
                        .size(11.0),
                )
                .on_hover_text("Showing only some rows — click to show everything")
                .clicked()
            {
                view.soloed.clear();
            }
        }

        group_gap(ui);
        transport::ui(ui, state);

        group_gap(ui);
        // Zoom, at the end where it is out of the way of the controls used every
        // few seconds.
        if ui
            .button(crate::ui::icons::ZOOM_OUT)
            .on_hover_text("Zoom out (Ctrl+scroll)")
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec / 1.4).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
        }
        if ui
            .button(crate::ui::icons::ZOOM_IN)
            .on_hover_text("Zoom in (Ctrl+scroll)")
            .clicked()
        {
            view.px_per_sec = (view.px_per_sec * 1.4).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
        }
        // Typed as well as dragged: "line these two up at 40 px per frame" is a
        // thing animators say, and hunting for it with a wheel is not an answer.
        let fps = state.doc.meta.fps.max(1) as f32;
        let mut frame_px = view.px_per_sec / fps;
        if ui
            .add(
                egui::DragValue::new(&mut frame_px)
                    .speed(0.1)
                    .range(MIN_PX_PER_SEC / fps..=MAX_PX_PER_SEC / fps)
                    .suffix(" px/f"),
            )
            .on_hover_text("Pixels per frame — drag or type")
            .changed()
        {
            view.px_per_sec = (frame_px * fps).clamp(MIN_PX_PER_SEC, MAX_PX_PER_SEC);
        }
        if ui
            .button(crate::ui::icons::FIT)
            .on_hover_text("Fit the clip to the view")
            .clicked()
        {
            fit_to_clip(state, view);
        }
    });
}

/// The space between two groups of controls.
///
/// A separator plus symmetric padding, in one place, so the groups do not drift
/// apart as controls are added and removed.
fn group_gap(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    ui.add(egui::Separator::default().vertical().shrink(4.0));
    ui.add_space(6.0);
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
            egui::RichText::new(crate::ui::icons::DOPESHEET)
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
            egui::RichText::new(crate::ui::icons::DOPESHEET)
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

// The clip menu that used to live here — rename, duplicate, delete, duration —
// moved to the Animations pane. Managing clips and scrubbing one are different
// jobs, and a menu that did both meant the same command reachable from two
// places, each with its own idea of what was selected.
