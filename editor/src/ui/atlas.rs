//! Spritesheet import: slice a grid into individual assets (T-305).
//!
//! Cells are **cropped into their own assets** rather than kept as UV windows
//! into one shared texture. That costs some memory and gives up atlas batching,
//! but it makes every downstream feature — pivots, meshes, per-image reload —
//! work without a second code path, and the export atlas packer (T-603) rebuilds
//! a packed sheet anyway.

use crate::app_state::AppState;
use crate::commands::asset_cmds::AddAssets;
use ankhimate_core::assets::ImageAsset;
use eframe::egui;

/// A sheet waiting to be sliced. Session state — cancelling must leave no trace.
#[derive(Clone)]
pub struct PendingAtlas {
    pub name: String,
    pub source_path: Option<String>,
    /// The encoded file, kept so cropping can decode it on demand.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    /// Border skipped on every edge of the sheet.
    pub margin: u32,
    /// Gap between neighbouring cells.
    pub spacing: u32,
    /// Skip cells that are entirely transparent — sheets are usually ragged.
    pub skip_empty: bool,
}

impl PendingAtlas {
    pub fn new(name: String, bytes: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            name,
            source_path: None,
            bytes,
            width,
            height,
            columns: 4,
            rows: 4,
            margin: 0,
            spacing: 0,
            skip_empty: true,
        }
    }

    /// Pixel rect of one cell: `(x, y, w, h)`.
    ///
    /// Integer division deliberately leaves any remainder as unused pixels on
    /// the right/bottom edge rather than stretching cells to fit — a sheet whose
    /// size is not a multiple of its grid is a sheet with a margin, and guessing
    /// otherwise smears every frame by a pixel.
    pub fn cell_rect(&self, col: u32, row: u32) -> (u32, u32, u32, u32) {
        let usable_w = self
            .width
            .saturating_sub(self.margin * 2)
            .saturating_sub(self.spacing * self.columns.saturating_sub(1));
        let usable_h = self
            .height
            .saturating_sub(self.margin * 2)
            .saturating_sub(self.spacing * self.rows.saturating_sub(1));
        let cell_w = usable_w / self.columns.max(1);
        let cell_h = usable_h / self.rows.max(1);
        (
            self.margin + col * (cell_w + self.spacing),
            self.margin + row * (cell_h + self.spacing),
            cell_w,
            cell_h,
        )
    }

    pub fn cell_count(&self) -> u32 {
        self.columns.max(1) * self.rows.max(1)
    }
}

/// Draw the slicer window. Returns `true` when it should close.
pub fn ui(ctx: &egui::Context, state: &mut AppState) -> bool {
    let Some(mut pending) = state.session.pending_atlas.clone() else {
        return false;
    };

    let mut close = false;
    let mut confirm = false;

    egui::Window::new("Import spritesheet")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(560.0)
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                preview(ui, state, &pending);

                ui.vertical(|ui| {
                    ui.set_min_width(190.0);
                    ui.label(
                        egui::RichText::new(format!("{}×{} px", pending.width, pending.height))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        ui.label("Columns");
                        ui.add(egui::DragValue::new(&mut pending.columns).range(1..=64));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rows");
                        ui.add(egui::DragValue::new(&mut pending.rows).range(1..=64));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Margin");
                        ui.add(egui::DragValue::new(&mut pending.margin).range(0..=512))
                            .on_hover_text("Border skipped on every edge of the sheet");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Spacing");
                        ui.add(egui::DragValue::new(&mut pending.spacing).range(0..=512))
                            .on_hover_text("Gap between neighbouring cells");
                    });
                    ui.checkbox(&mut pending.skip_empty, "Skip empty cells")
                        .on_hover_text("Ignore cells that are fully transparent");

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut pending.name).desired_width(120.0));
                    });

                    let (w, h, _) = {
                        let (_, _, w, h) = pending.cell_rect(0, 0);
                        (w, h, ())
                    };
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} cells · {w}×{h} px each",
                            pending.cell_count()
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                    if w == 0 || h == 0 {
                        ui.label(
                            egui::RichText::new("Grid does not fit the sheet")
                                .small()
                                .color(egui::Color32::from_rgb(230, 90, 90)),
                        );
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(w > 0 && h > 0, egui::Button::new("Import"))
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            });
        });

    if confirm {
        slice_and_import(state, &pending);
        close = true;
    }
    if close {
        state.session.pending_atlas = None;
        state.session.thumbnails.remove(ATLAS_PREVIEW_KEY);
    } else {
        state.session.pending_atlas = Some(pending);
    }
    close
}

const ATLAS_PREVIEW_KEY: &str = "atlas_preview";

/// The sheet with the grid drawn over it, so the numbers have something to mean.
fn preview(ui: &mut egui::Ui, state: &mut AppState, pending: &PendingAtlas) {
    const MAX: f32 = 320.0;

    let handle = match state.session.thumbnails.get(ATLAS_PREVIEW_KEY) {
        Some(handle) => Some(handle.clone()),
        None => image::load_from_memory(&pending.bytes).ok().map(|img| {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let color =
                egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
            let handle =
                ui.ctx()
                    .load_texture(ATLAS_PREVIEW_KEY, color, egui::TextureOptions::LINEAR);
            state
                .session
                .thumbnails
                .insert(ATLAS_PREVIEW_KEY.to_string(), handle.clone());
            handle
        }),
    };

    let scale = (MAX / pending.width.max(1) as f32).min(MAX / pending.height.max(1) as f32);
    let size = egui::vec2(pending.width as f32 * scale, pending.height as f32 * scale);

    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    match handle {
        Some(handle) => {
            egui::Image::new(&handle).paint_at(ui, rect);
        }
        None => {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        }
    }

    // Grid lines from the same `cell_rect` the import uses, so what is drawn is
    // exactly what will be cut.
    let painter = ui.painter_at(rect);
    let line = egui::Stroke::new(
        1.0,
        egui::Color32::from_rgba_unmultiplied(120, 230, 255, 180),
    );
    for row in 0..pending.rows.max(1) {
        for col in 0..pending.columns.max(1) {
            let (x, y, w, h) = pending.cell_rect(col, row);
            let cell = egui::Rect::from_min_size(
                rect.min + egui::vec2(x as f32 * scale, y as f32 * scale),
                egui::vec2(w as f32 * scale, h as f32 * scale),
            );
            painter.rect_stroke(cell, 0.0, line, egui::StrokeKind::Inside);
        }
    }
}

/// Crop every cell into its own asset and import them as one undo step.
fn slice_and_import(state: &mut AppState, pending: &PendingAtlas) {
    let Ok(image) = image::load_from_memory(&pending.bytes) else {
        state.session.set_status("Could not decode the sheet");
        return;
    };
    let sheet = image.to_rgba8();

    let mut assets = Vec::new();
    let mut skipped = 0;
    for row in 0..pending.rows.max(1) {
        for col in 0..pending.columns.max(1) {
            let (x, y, w, h) = pending.cell_rect(col, row);
            if w == 0 || h == 0 || x + w > pending.width || y + h > pending.height {
                continue;
            }
            let cell = image::imageops::crop_imm(&sheet, x, y, w, h).to_image();
            if pending.skip_empty && cell.pixels().all(|p| p.0[3] == 0) {
                skipped += 1;
                continue;
            }

            // Re-encoded as PNG: these bytes are new images, not the user's
            // original file, so there is nothing to preserve byte-for-byte.
            let mut bytes = Vec::new();
            if image::DynamicImage::ImageRgba8(cell)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .is_err()
            {
                continue;
            }

            let index = row * pending.columns.max(1) + col;
            assets.push(ImageAsset::new(
                format!("{}_{index:02}", pending.name),
                bytes,
                w,
                h,
            ));
        }
    }

    if assets.is_empty() {
        state.session.set_status("No cells to import");
        return;
    }
    let count = assets.len();
    if state.dispatch(Box::new(AddAssets::new(assets))) {
        state.session.set_status(match skipped {
            0 => format!("Imported {count} cells"),
            n => format!("Imported {count} cells ({n} empty skipped)"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(width: u32, height: u32) -> PendingAtlas {
        PendingAtlas::new("sheet".into(), Vec::new(), width, height)
    }

    #[test]
    fn plain_grid_splits_evenly() {
        let mut atlas = sheet(64, 64);
        atlas.columns = 4;
        atlas.rows = 4;
        assert_eq!(atlas.cell_rect(0, 0), (0, 0, 16, 16));
        assert_eq!(atlas.cell_rect(3, 3), (48, 48, 16, 16));
        assert_eq!(atlas.cell_count(), 16);
    }

    #[test]
    fn margin_and_spacing_are_taken_off_the_usable_area() {
        // 64px wide, 2px margin each side, 4 columns with 2px gaps:
        // usable = 64 - 4 - 6 = 54 → 13px cells (remainder left unused).
        let mut atlas = sheet(64, 64);
        atlas.columns = 4;
        atlas.rows = 1;
        atlas.margin = 2;
        atlas.spacing = 2;
        let (x, _, w, _) = atlas.cell_rect(0, 0);
        assert_eq!((x, w), (2, 13));
        let (x, _, _, _) = atlas.cell_rect(1, 0);
        assert_eq!(x, 2 + 13 + 2, "next cell clears the gap");
    }

    #[test]
    fn a_grid_that_cannot_fit_yields_zero_sized_cells() {
        // The UI refuses to import on this rather than emitting garbage.
        let mut atlas = sheet(8, 8);
        atlas.columns = 16;
        atlas.rows = 1;
        let (_, _, w, _) = atlas.cell_rect(0, 0);
        assert_eq!(w, 0);
    }
}
