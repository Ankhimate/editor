//! The timeline's left name pane: a folded tree of `target ▸ property` rows.
//!
//! Group headers (bone/slot) carry a fold triangle and a summary dot; property
//! rows sit indented beneath. Row order and height match the sheet exactly (both
//! iterate [`TimelineModel::visible_rows`]), so labels line up with their keys.

use super::model::{TimelineModel, VisibleRow};
use super::{ROW_H, ViewState};
use crate::app_state::AppState;
use eframe::egui;

/// Gap between a row's icon and its label.
///
/// Not zero and not the default word spacing: the glyphs are drawn by hand at a
/// fixed x, so nothing else puts space there.
const ICON_GAP: f32 = 17.0;

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
    style: super::Style<'_>,
) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();

    // Panel background.
    painter.rect_filled(rect, 0.0, visuals.panel_fill);

    let folded = |id: u64| is_folded(ui, id);
    let rows = model.visible_rows(&folded);

    let mut y = rect.top() - view.scroll_y;
    let mut toggle: Option<u64> = None;
    let mut solo_toggle: Option<(u64, bool)> = None;

    for row in rows.iter() {
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(rect.width(), ROW_H));
        // Cull rows scrolled out of view.
        if row_rect.bottom() < rect.top() || row_rect.top() > rect.bottom() {
            y += ROW_H;
            continue;
        }

        painter.rect_filled(
            row_rect,
            0.0,
            band_color(&visuals, matches!(row, VisibleRow::Group { .. })),
        );

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
                    egui::pos2(rect.left() + 22.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.icon,
                    egui::FontId::proportional(style.text + 0.5),
                    tint,
                );
                // Group label. The gap after the icon is deliberate: glyph and
                // word ran together, and at a glance the pair read as one long
                // unfamiliar word rather than as an icon and a name.
                painter.text(
                    egui::pos2(rect.left() + 22.0 + ICON_GAP, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    &data.label,
                    egui::FontId::proportional(style.text + 0.5),
                    visuals.strong_text_color(),
                );
            }
            VisibleRow::Property { data, .. } => {
                let color = if !row.is_soloed(&view.soloed) {
                    visuals.weak_text_color().gamma_multiply(0.5)
                } else if data.read_only {
                    visuals.weak_text_color()
                } else {
                    visuals.text_color()
                };
                // The channel's own colour, the same one the graph plots it in.
                let icon_color = if data.read_only || !row.is_soloed(&view.soloed) {
                    color
                } else {
                    style.theme.channel_color(data.label)
                };
                painter.text(
                    egui::pos2(rect.left() + 42.0, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.icon,
                    egui::FontId::proportional(style.text),
                    icon_color,
                );
                painter.text(
                    egui::pos2(rect.left() + 42.0 + ICON_GAP, y + ROW_H / 2.0),
                    egui::Align2::LEFT_CENTER,
                    data.label,
                    egui::FontId::proportional(style.text),
                    color,
                );
            }
        }

        // Solo dot, right-aligned. Filled when this row is the one being shown.
        let dot_rect =
            egui::Rect::from_min_size(egui::pos2(rect.right() - 16.0, y), egui::vec2(14.0, ROW_H));
        let dot = ui.interact(
            dot_rect,
            ui.id().with(("tl_solo", row.solo_id())),
            egui::Sense::click(),
        );
        let on = view.soloed.contains(&row.solo_id());
        if dot.clicked() {
            solo_toggle = Some((row.solo_id(), !on));
        }
        painter.text(
            dot_rect.center(),
            egui::Align2::CENTER_CENTER,
            if on {
                egui_phosphor::fill::CIRCLE
            } else {
                egui_phosphor::regular::CIRCLE
            },
            egui::FontId::proportional(8.0),
            if on {
                visuals.selection.bg_fill
            } else if dot.hovered() {
                visuals.strong_text_color()
            } else {
                visuals.weak_text_color().gamma_multiply(0.6)
            },
        );

        y += ROW_H;
    }

    if let Some((id, on)) = solo_toggle {
        if on {
            view.soloed.insert(id);
        } else {
            view.soloed.remove(&id);
        }
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

/// Row background, by what the row *is* rather than by whether it is odd.
///
/// Zebra striping made a group header and the property under it look the same
/// whenever they happened to land on the same parity, which is exactly the
/// distinction the panel exists to draw. A header is a heading and its
/// properties are its contents; two tones say so on every row, every time.
pub fn band_color(visuals: &egui::Visuals, group: bool) -> egui::Color32 {
    if group {
        if visuals.dark_mode {
            visuals.faint_bg_color.linear_multiply(2.1)
        } else {
            visuals.faint_bg_color.linear_multiply(0.9)
        }
    } else if visuals.dark_mode {
        visuals.faint_bg_color.linear_multiply(0.85)
    } else {
        visuals.faint_bg_color.linear_multiply(1.4)
    }
}
