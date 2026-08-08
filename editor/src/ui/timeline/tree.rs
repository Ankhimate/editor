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
    // Offsets are authored in seconds but read in frames — an animator says
    // "four frames behind", never "0.133 seconds behind".
    let fps = state.doc.meta.fps.max(1) as f32;

    // Panel background.
    painter.rect_filled(rect, 0.0, visuals.panel_fill);

    // A right-clicked bone group asks for the offset editor (T-905). Collected
    // rather than opened in place, because the row loop borrows the model.
    let mut offset_popup: Option<(ankhimate_core::ids::BoneId, f32, egui::Pos2)> = None;

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
                    crate::ui::icons::CARET_RIGHT
                } else {
                    crate::ui::icons::CARET_DOWN
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
                // A shifted track says so on its header (T-905). An offset is
                // invisible in the keys — they do not move — so without this a
                // track that plays late looks like a track someone keyed wrong,
                // and the reason is in a panel you would have no cause to open.
                if data.offset != 0.0 {
                    let frames = data.offset * fps;
                    painter.text(
                        egui::pos2(rect.right() - 6.0, y + ROW_H / 2.0),
                        egui::Align2::RIGHT_CENTER,
                        format!("{frames:+.0}f"),
                        egui::FontId::proportional(style.text - 1.0),
                        egui::Color32::from_rgb(120, 190, 255),
                    );
                }
                // Right-click a bone group to offset it.
                if let Some(bone) = data.bone {
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.left(), y),
                        egui::vec2(rect.width(), ROW_H),
                    );
                    let row_resp = ui.interact(
                        row_rect,
                        ui.id().with(("tl_group_row", data.fold_id)),
                        egui::Sense::click(),
                    );
                    if row_resp.secondary_clicked() {
                        offset_popup = Some((bone, data.offset, row_resp.rect.left_bottom()));
                    }
                }
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
                crate::ui::icons::DOT_ON
            } else {
                crate::ui::icons::DOT_OFF
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

    // ── Track offset editor (T-905) ──────────────────────────────────────
    // A window we own, for the reason given in `timeline::ruler`: egui's context
    // menu closes on any plain click of its host, and the host here is a row the
    // popup is drawn over.
    let popup_id = ui.id().with("track_offset_popup");
    if let Some((bone, offset, anchor)) = offset_popup {
        ui.ctx()
            .data_mut(|d| d.insert_temp(popup_id, (bone, offset, anchor)));
    }
    if let Some((bone, current, anchor)) = ui
        .ctx()
        .data(|d| d.get_temp::<(ankhimate_core::ids::BoneId, f32, egui::Pos2)>(popup_id))
    {
        let mut close = false;
        let mut set: Option<f32> = None;
        egui::Window::new("track_offset_popup")
            .title_bar(false)
            .resizable(false)
            .fixed_pos(anchor)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.label(egui::RichText::new("Track offset").strong());
                // Authored in frames, stored in seconds: an animator says "four
                // frames behind", never "0.133 seconds behind".
                let mut frames = current * fps;
                if ui
                    .add(
                        egui::DragValue::new(&mut frames)
                            .speed(0.25)
                            .suffix(" frames"),
                    )
                    .on_hover_text(
                        "Read this bone's curve early or late without moving a \
                         key.\nPositive trails, negative leads.\nWhat a scarf, a \
                         tail or ten strands of hair want.",
                    )
                    .changed()
                {
                    set = Some(frames / fps);
                }
                ui.horizontal(|ui| {
                    if ui.button("Clear").clicked() {
                        set = Some(0.0);
                        close = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        if let Some(seconds) = set
            && let Some(anim) = state.session.active_animation
        {
            state.dispatch(Box::new(crate::commands::marker_cmds::SetBoneOffset::new(
                anim, bone, seconds,
            )));
        }
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.ctx().data_mut(|d| {
                d.remove_temp::<(ankhimate_core::ids::BoneId, f32, egui::Pos2)>(popup_id)
            });
        }
    }
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
