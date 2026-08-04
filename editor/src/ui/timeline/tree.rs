//! The timeline's left name pane: a folded tree of `target ▸ property` rows.
//!
//! Group headers (bone/slot) carry a fold triangle and a summary dot; property
//! rows sit indented beneath. Row order and height match the sheet exactly (both
//! iterate [`TimelineModel::visible_rows`]), so labels line up with their keys.

use super::model::{TimelineModel, VisibleRow};
use super::{ROW_H, ViewState};
use crate::app_state::AppState;
use eframe::egui;

/// Read a group's fold state from egui memory.
pub fn is_folded(ui: &egui::Ui, fold_id: u64) -> bool {
    ui.ctx()
        .memory(|m| m.data.get_temp(egui::Id::new(("tl_fold", fold_id))))
        .unwrap_or(false)
}

fn set_folded(ui: &egui::Ui, fold_id: u64, folded: bool) {
    ui.ctx().memory_mut(|m| {
        m.data
            .insert_temp(egui::Id::new(("tl_fold", fold_id)), folded)
    });
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    model: &TimelineModel,
    view: &mut ViewState,
    rect: egui::Rect,
) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();

    // Panel background.
    painter.rect_filled(rect, 0.0, visuals.panel_fill);

    let folded = |id: u64| is_folded(ui, id);
    let rows = model.visible_rows(&folded);

    let mut y = rect.top() - view.scroll_y;
    let mut toggle: Option<u64> = None;

    for (i, row) in rows.iter().enumerate() {
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), ROW_H));
        // Cull rows scrolled out of view.
        if row_rect.bottom() < rect.top() || row_rect.top() > rect.bottom() {
            y += ROW_H;
            continue;
        }

        // Alternating band.
        if i % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, band_color(&visuals));
        }

        match row {
            VisibleRow::Group { data, folded, .. } => {
                // Fold triangle.
                let tri_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 2.0, y),
                    egui::vec2(16.0, ROW_H),
                );
                let tri_resp = ui.interact(
                    tri_rect,
                    ui.id().with(("tl_fold_btn", data.fold_id)),
                    egui::Sense::click(),
                );
                if tri_resp.clicked() {
                    toggle = Some(data.fold_id);
                }
                let tri = if *folded {
                    egui_phosphor::regular::CARET_RIGHT
                } else {
                    egui_phosphor::regular::CARET_DOWN
                };
                painter.text(
                    tri_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    tri,
                    egui::FontId::proportional(11.0),
                    visuals.weak_text_color(),
                );
                // Group icon, tinted with the bone's group colour (T-505) so a
                // limb reads as one thing down the whole panel.
                let tint = data
                    .tint
                    .map(|[r, g, b, _]| {
                        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
                        egui::Color32::from_rgb(to_u8(r), to_u8(g), to_u8(b))
                    })
                    .unwrap_or_else(|| visuals.weak_text_color());
                painter.text(
                    egui::pos2(rect.left() + 20.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.icon,
                    egui::FontId::proportional(12.0),
                    tint,
                );
                // Group label.
                painter.text(
                    egui::pos2(rect.left() + 36.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    &data.label,
                    egui::FontId::proportional(12.0),
                    visuals.strong_text_color(),
                );
            }
            VisibleRow::Property { data, .. } => {
                let color = if data.read_only {
                    visuals.weak_text_color()
                } else {
                    visuals.text_color()
                };
                painter.text(
                    egui::pos2(rect.left() + 40.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.icon,
                    egui::FontId::proportional(11.0),
                    property_tint(data.label, color),
                );
                painter.text(
                    egui::pos2(rect.left() + 56.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.label,
                    egui::FontId::proportional(11.0),
                    color,
                );
            }
        }

        y += ROW_H;
    }

    if let Some(id) = toggle {
        let now = is_folded(ui, id);
        set_folded(ui, id, !now);
    }

    // Clamp vertical scroll to content.
    let content_h = rows.len() as f32 * ROW_H;
    let max_scroll = (content_h - rect.height()).max(0.0);
    view.scroll_y = view.scroll_y.clamp(0.0, max_scroll);

    // Suppress unused warning in builds where state is not read yet.
    let _ = state;
}

/// The hue for a property glyph.
///
/// The same hues the graph plots those channels in, so a green `rotate` row and
/// a green curve are recognisably the same thing. Read-only rows keep the text
/// colour: they have no curve to match.
fn property_tint(label: &str, fallback: egui::Color32) -> egui::Color32 {
    match label {
        "translate" => egui::Color32::from_rgb(110, 160, 230),
        "rotate" => egui::Color32::from_rgb(110, 200, 110),
        "scale" => egui::Color32::from_rgb(220, 160, 90),
        "shear" => egui::Color32::from_rgb(190, 140, 220),
        "color" => egui::Color32::from_rgb(220, 120, 150),
        "attachment" => egui::Color32::from_rgb(126, 176, 224),
        _ => fallback,
    }
}

/// Subtle alternating band, a touch lighter than the panel.
pub fn band_color(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        visuals.faint_bg_color.linear_multiply(1.6)
    } else {
        visuals.faint_bg_color
    }
}
