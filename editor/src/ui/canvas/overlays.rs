use super::camera::{MAX_ZOOM, MIN_ZOOM, t_to_zoom, zoom_to_t};
use crate::app_state::AppState;
use eframe::egui;

// â”€â”€ Zoom Slider Overlay â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

const ZOOM_MARGIN: f32 = 10.0;
const ZOOM_WIDTH: f32 = 32.0;
const ZOOM_BTN_H: f32 = 22.0;
const ZOOM_TRACK_H: f32 = 110.0;
const ZOOM_LABEL_H: f32 = 18.0; // space for % label inside the bar
const ZOOM_TOTAL_H: f32 = ZOOM_BTN_H * 2.0 + ZOOM_TRACK_H + ZOOM_LABEL_H + 6.0;

pub fn zoom_bar_rect(canvas_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(
            canvas_rect.max.x - ZOOM_WIDTH - ZOOM_MARGIN,
            canvas_rect.center().y - ZOOM_TOTAL_H / 2.0,
        ),
        egui::vec2(ZOOM_WIDTH, ZOOM_TOTAL_H),
    )
}

pub fn draw_zoom_bar(ui: &mut egui::Ui, canvas_rect: egui::Rect, state: &mut AppState) {
    let bar_rect = zoom_bar_rect(canvas_rect);
    // Clamp bar inside the canvas so it never gets clipped at window edges
    let bar_rect = egui::Rect::from_min_max(
        bar_rect.min.max(canvas_rect.min + egui::vec2(0.0, 4.0)),
        bar_rect.max.min(canvas_rect.max - egui::vec2(4.0, 4.0)),
    );

    let painter = ui.painter_at(canvas_rect);
    let bg = egui::Color32::from_rgba_unmultiplied(20, 20, 20, 220);
    let border = egui::Color32::from_rgba_unmultiplied(60, 60, 60, 180);
    let dim = egui::Color32::from_rgb(140, 140, 140);
    let bright = egui::Color32::WHITE;
    let accent = egui::Color32::from_rgb(80, 200, 200);

    painter.rect_filled(bar_rect, 7.0, bg);
    painter.rect_stroke(
        bar_rect,
        7.0,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Outside,
    );

    let cx = bar_rect.center().x;

    // Layout (top â†’ bottom): [+] [track] [âˆ’] [label]
    let plus_rect = egui::Rect::from_min_size(bar_rect.min, egui::vec2(ZOOM_WIDTH, ZOOM_BTN_H));
    let track_top = bar_rect.min.y + ZOOM_BTN_H + 2.0;
    let track_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.min.x, track_top),
        egui::vec2(ZOOM_WIDTH, ZOOM_TRACK_H),
    );
    let minus_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.min.x, track_top + ZOOM_TRACK_H + 2.0),
        egui::vec2(ZOOM_WIDTH, ZOOM_BTN_H),
    );
    let label_rect = egui::Rect::from_min_size(
        egui::pos2(bar_rect.min.x, minus_rect.max.y),
        egui::vec2(ZOOM_WIDTH, ZOOM_LABEL_H),
    );

    let mouse = ui.input(|i| i.pointer.hover_pos());
    let clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
    let held = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));

    let plus_hov = mouse.is_some_and(|p| plus_rect.contains(p));
    let minus_hov = mouse.is_some_and(|p| minus_rect.contains(p));
    let track_hov = mouse.is_some_and(|p| track_rect.contains(p));

    // + button
    painter.text(
        plus_rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        egui::FontId::proportional(14.0),
        if plus_hov { bright } else { dim },
    );

    // Track line
    let ti = track_rect.min.y + 4.0;
    let tb = track_rect.max.y - 4.0;
    let th = tb - ti;
    painter.line_segment(
        [egui::pos2(cx, ti), egui::pos2(cx, tb)],
        egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 50, 50)),
    );

    // 100% tick
    let t100 = zoom_to_t(1.0);
    let tick_y = tb - t100 * th;
    painter.line_segment(
        [egui::pos2(cx - 5.0, tick_y), egui::pos2(cx + 5.0, tick_y)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 80, 80)),
    );

    // Thumb
    let t = zoom_to_t(state.session.camera.zoom);
    let thumb_y = tb - t * th;
    let th_hov = mouse.is_some_and(|p| (p.x - cx).abs() < 10.0 && (p.y - thumb_y).abs() < 10.0);

    if th_hov || (held && track_hov) {
        painter.circle_filled(
            egui::pos2(cx, thumb_y),
            8.0,
            egui::Color32::from_rgba_unmultiplied(80, 200, 200, 35),
        );
    }
    painter.circle_filled(egui::pos2(cx, thumb_y), 5.0, accent);
    painter.circle_stroke(
        egui::pos2(cx, thumb_y),
        5.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 240, 240)),
    );

    // âˆ’ button
    painter.text(
        minus_rect.center(),
        egui::Align2::CENTER_CENTER,
        "âˆ’",
        egui::FontId::proportional(14.0),
        if minus_hov { bright } else { dim },
    );

    // % label â€” inside the bar now, no clipping risk
    let zoom_pct = (state.session.camera.zoom * 100.0).round() as i32;
    painter.text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{}%", zoom_pct),
        egui::FontId::proportional(10.0),
        dim,
    );

    // â”€â”€ Interactions â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
    if clicked {
        if plus_hov {
            state.session.camera.zoom =
                (state.session.camera.zoom * 1.25).clamp(MIN_ZOOM, MAX_ZOOM);
        } else if minus_hov {
            state.session.camera.zoom =
                (state.session.camera.zoom / 1.25).clamp(MIN_ZOOM, MAX_ZOOM);
        } else if track_hov && let Some(p) = mouse {
            state.session.camera.zoom = t_to_zoom(1.0 - (p.y - ti) / th);
        }
    }
    if held
        && track_hov
        && let Some(p) = mouse
    {
        state.session.camera.zoom = t_to_zoom((1.0 - (p.y - ti) / th).clamp(0.0, 1.0));
    }

    if plus_hov || minus_hov || track_hov || th_hov {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

// â”€â”€ Grid â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub fn draw_grid(
    ui: &egui::Ui,
    rect: egui::Rect,
    state: &AppState,
    theme: &crate::theme::Theme,
    settings: &crate::config::GridSettings,
) {
    ui.painter().rect_filled(
        rect,
        0.0,
        ui.visuals().extreme_bg_color.linear_multiply(0.5),
    );

    let base_size = settings.cell.max(1.0);
    let cx = rect.center().x - state.session.camera.position.x * state.session.camera.zoom;
    let cy = rect.center().y + state.session.camera.position.y * state.session.camera.zoom;

    // Static checker: one fixed world-space cell size, so cells simply grow and
    // shrink with zoom. Deliberately no zoom-adaptive level switching â€” the
    // checker is a transparency backdrop, and a cell size that snaps between
    // levels reads as "the texture changed" rather than "I zoomed".
    let scaled = base_size * state.session.camera.zoom;

    // Below this the checker is visual noise, and the cell count explodes (a 3px
    // cell is ~230k rects on a 1080p viewport, every frame), so stop drawing
    // cells and leave the flat background.
    let min_cell_px = settings.min_cell_px.max(2.0);
    if settings.show && scaled >= min_cell_px {
        // Fade out as cells approach the noise floor so it does not pop off.
        let alpha = ((scaled - min_cell_px) / 8.0).clamp(0.2, 1.0);

        let color_even = theme.grid_color_even().linear_multiply(alpha);
        let color_odd = theme.grid_color_odd().linear_multiply(alpha);

        let first_x = ((rect.min.x - cx) / scaled).floor() as i32;
        let last_x = ((rect.max.x - cx) / scaled).ceil() as i32;
        let first_y = ((rect.min.y - cy) / scaled).floor() as i32;
        let last_y = ((rect.max.y - cy) / scaled).ceil() as i32;

        for i in first_x..=last_x {
            for j in first_y..=last_y {
                let is_even = (i.rem_euclid(2) + j.rem_euclid(2)) % 2 == 0;
                let color = if is_even { color_even } else { color_odd };
                let x1 = cx + (i as f32) * scaled;
                let y1 = cy + (j as f32) * scaled;
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x1, y1),
                        egui::pos2(x1 + scaled, y1 + scaled),
                    ),
                    0.0,
                    color,
                );
            }
        }
    }

    // Origin axes always visible
    let axis = egui::Stroke::new(1.5, theme.origin_color());
    ui.painter().line_segment(
        [egui::pos2(cx, rect.min.y), egui::pos2(cx, rect.max.y)],
        axis,
    );
    ui.painter().line_segment(
        [egui::pos2(rect.min.x, cy), egui::pos2(rect.max.x, cy)],
        axis,
    );
}

// The floating transform overlay lived here until it was removed: it duplicated
// the inspector's fields with its own (wrong) units, and a modal panel parked
// over the artwork is the wrong shape for this anyway. The Properties panel is
// the single place to type a transform until a proper on-canvas widget lands
// (T-708).
