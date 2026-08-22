use crate::app_state::AppState;
use eframe::egui;

pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 20.0;

#[derive(Clone, Copy)]
pub struct Camera2D {
    pub position: glam::Vec2, // world-space point at the center of the viewport
    pub zoom: f32,            // screen pixels per world unit; bigger = more zoomed in
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            position: glam::Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera2D {
    pub fn screen_to_world(&self, screen_pos: glam::Vec2, viewport_size: glam::Vec2) -> glam::Vec2 {
        let centered = screen_pos - viewport_size * 0.5;
        // flip Y if your world space is Y-up and screen space is Y-down
        let world_offset = glam::Vec2::new(centered.x, -centered.y) / self.zoom;
        self.position + world_offset
    }

    pub fn world_to_screen(&self, world_pos: glam::Vec2, viewport_size: glam::Vec2) -> glam::Vec2 {
        let offset = (world_pos - self.position) * self.zoom;
        let screen_offset = glam::Vec2::new(offset.x, -offset.y);
        viewport_size * 0.5 + screen_offset
    }

    pub fn view_proj_matrix(&self, viewport_size: glam::Vec2) -> glam::Mat4 {
        let half_w = viewport_size.x * 0.5 / self.zoom;
        let half_h = viewport_size.y * 0.5 / self.zoom;
        let proj = glam::Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, -1000.0, 1000.0);
        let view = glam::Mat4::from_translation(-self.position.extend(0.0));
        proj * view
    }
}

/// Convert a screen-space egui position to world-space coordinates.
pub fn screen_to_world(pos: egui::Pos2, rect: egui::Rect, state: &AppState) -> glam::Vec2 {
    let screen_pos = glam::Vec2::new(pos.x - rect.min.x, pos.y - rect.min.y);
    let viewport_size = glam::Vec2::new(rect.width(), rect.height());
    state
        .session
        .camera
        .screen_to_world(screen_pos, viewport_size)
}

/// Convert a world-space position to screen-space egui position.
pub fn world_to_screen(world: glam::Vec2, rect: egui::Rect, state: &AppState) -> egui::Pos2 {
    let viewport_size = glam::Vec2::new(rect.width(), rect.height());
    let screen_pos = state.session.camera.world_to_screen(world, viewport_size);
    egui::pos2(rect.min.x + screen_pos.x, rect.min.y + screen_pos.y)
}

/// Logarithmic mapping: zoom MIN_ZOOM..MAX_ZOOM → 0.0..1.0
pub fn zoom_to_t(zoom: f32) -> f32 {
    let min_log = MIN_ZOOM.ln();
    let max_log = MAX_ZOOM.ln();
    (zoom.clamp(MIN_ZOOM, MAX_ZOOM).ln() - min_log) / (max_log - min_log)
}

pub fn t_to_zoom(t: f32) -> f32 {
    let min_log = MIN_ZOOM.ln();
    let max_log = MAX_ZOOM.ln();
    (min_log + t.clamp(0.0, 1.0) * (max_log - min_log)).exp()
}

/// Returns `true` when the camera consumed this frame's pointer input, in which
/// case the active tool must not also act on it. Without this, a space+drag pan
/// would simultaneously drag a gizmo, and a scroll over the zoom bar would zoom
/// twice (once here, once via the bar's own slider).
pub fn handle_navigation(
    ui: &mut egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    state: &mut AppState,
) -> bool {
    let viewport_size = glam::Vec2::new(rect.width(), rect.height());
    let over_zoom_bar = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| super::overlays::zoom_bar_rect(rect).contains(p));

    // Camera Navigation: Pan with Middle or Right Click, or Space + Left Click Drag
    let is_panning = response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Secondary)
        || (response.dragged_by(egui::PointerButton::Primary)
            && ui.input(|i| i.key_down(egui::Key::Space)));

    if is_panning {
        let screen_delta = response.drag_delta();
        // Camera moves opposite to drag delta so the world follows the mouse
        let world_delta =
            glam::Vec2::new(screen_delta.x, -screen_delta.y) / state.session.camera.zoom;
        state.session.camera.position -= world_delta;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    // Camera Navigation: Zoom with Scroll Wheel. Skipped over the zoom bar so
    // the bar's own scroll handling is the only one that runs there.
    let mut zoomed = false;
    if response.hovered() && !over_zoom_bar {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 {
            let cursor_pos = ui
                .input(|i| i.pointer.hover_pos())
                .unwrap_or_else(|| rect.center());
            let local_cursor_pos =
                glam::Vec2::new(cursor_pos.x - rect.min.x, cursor_pos.y - rect.min.y);

            let world_before = state
                .session
                .camera
                .screen_to_world(local_cursor_pos, viewport_size);

            // Quantize to whole wheel notches. `smooth_scroll_delta` accumulates
            // across frames and runs far past one notch on a fast wheel or a
            // precision touchpad, so scaling continuously by the raw delta makes
            // a single flick cross most of the zoom range. Clamping the notch
            // count keeps one gesture to a predictable step.
            const PIXELS_PER_NOTCH: f32 = 50.0;
            const STEP_PER_NOTCH: f32 = 1.1;
            let notches = (scroll_delta / PIXELS_PER_NOTCH).clamp(-3.0, 3.0);

            // Multiplicative so a step feels the same at every zoom level.
            let zoom_factor = STEP_PER_NOTCH.powf(notches);
            state.session.camera.zoom =
                (state.session.camera.zoom * zoom_factor).clamp(MIN_ZOOM, MAX_ZOOM);

            let world_after = state
                .session
                .camera
                .screen_to_world(local_cursor_pos, viewport_size);
            state.session.camera.position += world_before - world_after;
            zoomed = true;
        }
    }

    is_panning || zoomed || over_zoom_bar
}
