//! Frame ruler + orange playhead. Click/drag anywhere on the sheet portion of
//! the ruler scrubs `Session.playhead`, snapped to whole frames.

use super::{Layout, sheet};
use crate::app_state::AppState;
use eframe::egui;

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

    // Scrub.
    let resp = ui.interact(
        sheet_rect,
        ui.id().with("tl_scrub"),
        egui::Sense::click_and_drag(),
    );
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        let t = layout.snap_time(layout.x_to_time(pos.x)).max(0.0);
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
