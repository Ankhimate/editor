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

/// How the sheet is cut up.
///
/// A grid covers the common case in four numbers; a rect list covers everything
/// else. Both produce the same thing — a list of named crops — so the import
/// path below them is shared.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SliceMode {
    Grid,
    Rects,
}

/// One named crop out of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

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
    pub mode: SliceMode,
    /// Hand-placed crops, used when `mode` is [`SliceMode::Rects`].
    pub rects: Vec<Cell>,
    /// Which rect the numeric fields and the preview handles act on.
    pub selected: Option<usize>,
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
            mode: SliceMode::Grid,
            rects: Vec::new(),
            selected: None,
        }
    }

    /// The crops this sheet will produce, whichever mode it is in.
    ///
    /// One function so the preview, the count and the import can never disagree
    /// about what is about to be cut.
    pub fn cells(&self) -> Vec<Cell> {
        match self.mode {
            SliceMode::Rects => self.rects.clone(),
            SliceMode::Grid => {
                let mut cells = Vec::new();
                for row in 0..self.rows.max(1) {
                    for col in 0..self.columns.max(1) {
                        let (x, y, w, h) = self.cell_rect(col, row);
                        if w == 0 || h == 0 {
                            continue;
                        }
                        let index = row * self.columns.max(1) + col;
                        cells.push(Cell {
                            name: format!("{}_{index:02}", self.name),
                            x,
                            y,
                            w,
                            h,
                        });
                    }
                }
                cells
            }
        }
    }

    /// Turn the current grid into editable rects.
    ///
    /// Switching modes carries the work over rather than starting from an empty
    /// list: "the grid is right except for two frames" is the usual reason
    /// anybody reaches for manual rects at all.
    pub fn adopt_grid(&mut self) {
        self.rects = self.cells();
        self.selected = self.rects.first().map(|_| 0);
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

/// Propose one rect per island of non-transparent pixels.
///
/// Flood fill rather than a grid guess, because the sheets that need this are
/// exactly the ones that are not on a grid — hand-packed exports where frames
/// differ in size. Islands are 4-connected: a diagonal touch is far more often
/// two frames whose corners graze than one frame with a pinched waist.
///
/// `alpha_floor` exists because "transparent" in an exported sheet is usually
/// *nearly* transparent — antialiased edges leave a halo of alpha 1-3 that
/// bridges neighbouring frames into one blob.
pub fn detect_cells(
    pixels: &image::RgbaImage,
    alpha_floor: u8,
    min_size: u32,
    name_prefix: &str,
) -> Vec<Cell> {
    let (w, h) = pixels.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let solid = |x: u32, y: u32| pixels.get_pixel(x, y).0[3] > alpha_floor;

    let mut seen = vec![false; (w * h) as usize];
    let mut cells = Vec::new();
    let mut stack: Vec<(u32, u32)> = Vec::new();

    for start_y in 0..h {
        for start_x in 0..w {
            let index = (start_y * w + start_x) as usize;
            if seen[index] || !solid(start_x, start_y) {
                continue;
            }
            // Explicit stack, not recursion: a full-bleed sheet is one island of
            // a million pixels and would blow the call stack.
            stack.push((start_x, start_y));
            seen[index] = true;
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (start_x, start_y, start_x, start_y);

            while let Some((x, y)) = stack.pop() {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                let mut visit = |nx: u32, ny: u32, stack: &mut Vec<(u32, u32)>| {
                    let i = (ny * w + nx) as usize;
                    if !seen[i] && solid(nx, ny) {
                        seen[i] = true;
                        stack.push((nx, ny));
                    }
                };
                if x > 0 {
                    visit(x - 1, y, &mut stack);
                }
                if x + 1 < w {
                    visit(x + 1, y, &mut stack);
                }
                if y > 0 {
                    visit(x, y - 1, &mut stack);
                }
                if y + 1 < h {
                    visit(x, y + 1, &mut stack);
                }
            }

            let (cw, ch) = (max_x - min_x + 1, max_y - min_y + 1);
            // Stray antialiasing dots are not frames.
            if cw < min_size || ch < min_size {
                continue;
            }
            cells.push(Cell {
                name: String::new(),
                x: min_x,
                y: min_y,
                w: cw,
                h: ch,
            });
        }
    }

    // Reading order, so the numbering matches how an artist laid the sheet out
    // rather than how the scan happened to reach each island.
    cells.sort_by_key(|c| (c.y, c.x));
    for (i, cell) in cells.iter_mut().enumerate() {
        cell.name = format!("{name_prefix}_{i:02}");
    }
    cells
}

/// Draw the slicer window. Returns `true` when it should close.
pub fn ui(ctx: &egui::Context, state: &mut AppState, theme: &crate::theme::Theme) -> bool {
    let Some(mut pending) = state.session.pending_atlas.clone() else {
        return false;
    };

    let mut close = false;
    let mut confirm = false;

    let dialog = crate::ui::dialog::Dialog::new("atlas_import", "Import spritesheet")
        .icon(crate::ui::icons::IMPORT_SHEET)
        .width(560.0)
        .show(ctx, theme, |ui| {
            ui.horizontal_top(|ui| {
                preview(ui, state, &mut pending);

                ui.vertical(|ui| {
                    ui.set_min_width(230.0);
                    ui.label(
                        egui::RichText::new(format!("{}×{} px", pending.width, pending.height))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(pending.mode == SliceMode::Grid, "Grid")
                            .clicked()
                        {
                            pending.mode = SliceMode::Grid;
                        }
                        if ui
                            .selectable_label(pending.mode == SliceMode::Rects, "Rects")
                            .on_hover_text("Hand-placed crops, for a sheet that is not on a grid")
                            .clicked()
                            && pending.mode != SliceMode::Rects
                        {
                            pending.mode = SliceMode::Rects;
                            if pending.rects.is_empty() {
                                pending.adopt_grid();
                            }
                        }
                    });
                    ui.add_space(4.0);

                    if pending.mode == SliceMode::Rects {
                        let (c, x) = rect_controls(ui, state, &mut pending);
                        confirm |= c;
                        close |= x;
                        return;
                    }

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

                    let (_, _, w, h) = pending.cell_rect(0, 0);
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
    close |= dialog.closed;
    if close {
        state.session.pending_atlas = None;
        state.session.thumbnails.remove(ATLAS_PREVIEW_KEY);
    } else {
        state.session.pending_atlas = Some(pending);
    }
    close
}

/// Numeric fields, the rect list, and the detect helper.
fn rect_controls(
    ui: &mut egui::Ui,
    state: &mut AppState,
    pending: &mut PendingAtlas,
) -> (bool, bool) {
    let (mut confirm, mut close) = (false, false);
    ui.horizontal(|ui| {
        if ui
            .button("Detect cells")
            .on_hover_text("Propose one rect per island of opaque pixels")
            .clicked()
        {
            match image::load_from_memory(&pending.bytes) {
                Ok(image) => {
                    let found =
                        detect_cells(&image.to_rgba8(), ALPHA_FLOOR, MIN_CELL, &pending.name);
                    if found.is_empty() {
                        state.session.set_status("No opaque regions found");
                    } else {
                        let count = found.len();
                        pending.rects = found;
                        pending.selected = Some(0);
                        state.session.set_status(format!("Found {count} cells"));
                    }
                }
                Err(e) => state
                    .session
                    .set_status(format!("Could not decode the sheet: {e}")),
            }
        }
        if ui
            .button("From grid")
            .on_hover_text("Replace the list with the current grid")
            .clicked()
        {
            pending.adopt_grid();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            let index = pending.rects.len();
            pending.rects.push(Cell {
                name: format!("{}_{index:02}", pending.name),
                x: 0,
                y: 0,
                w: (pending.width / 4).max(1),
                h: (pending.height / 4).max(1),
            });
            pending.selected = Some(index);
        }
        let has_selection = pending.selected.is_some();
        if ui
            .add_enabled(has_selection, egui::Button::new("Remove"))
            .clicked()
            && let Some(index) = pending.selected
            && index < pending.rects.len()
        {
            pending.rects.remove(index);
            pending.selected = if pending.rects.is_empty() {
                None
            } else {
                Some(index.min(pending.rects.len() - 1))
            };
        }
    });

    ui.add_space(6.0);
    let (sheet_w, sheet_h) = (pending.width.max(1), pending.height.max(1));
    let mut select: Option<usize> = None;
    egui::ScrollArea::vertical()
        .max_height(220.0)
        .show(ui, |ui| {
            for (index, cell) in pending.rects.iter_mut().enumerate() {
                let selected = pending.selected == Some(index);
                ui.horizontal(|ui| {
                    if ui.selectable_label(selected, "●").clicked() {
                        select = Some(index);
                    }
                    // Renaming in place: the whole reason to name a cell is that
                    // `sheet_07` tells you nothing when you go looking for it.
                    if ui
                        .add(egui::TextEdit::singleline(&mut cell.name).desired_width(96.0))
                        .has_focus()
                    {
                        select = Some(index);
                    }
                });
                if selected {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("L").size(10.5));
                        ui.add(egui::DragValue::new(&mut cell.x).range(0..=sheet_w - 1));
                        ui.label(egui::RichText::new("T").size(10.5));
                        ui.add(egui::DragValue::new(&mut cell.y).range(0..=sheet_h - 1));
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("W").size(10.5));
                        ui.add(egui::DragValue::new(&mut cell.w).range(1..=sheet_w));
                        ui.label(egui::RichText::new("H").size(10.5));
                        ui.add(egui::DragValue::new(&mut cell.h).range(1..=sheet_h));
                    });
                }
            }
        });
    if let Some(index) = select {
        pending.selected = Some(index);
    }
    // Clamp after editing rather than while typing: fighting a value mid-keystroke
    // makes the field feel broken.
    for cell in &mut pending.rects {
        cell.x = cell.x.min(sheet_w - 1);
        cell.y = cell.y.min(sheet_h - 1);
        cell.w = cell.w.clamp(1, sheet_w - cell.x);
        cell.h = cell.h.clamp(1, sheet_h - cell.y);
    }

    ui.add_space(6.0);
    ui.checkbox(&mut pending.skip_empty, "Skip empty cells");
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("{} cells", pending.rects.len()))
            .small()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!pending.rects.is_empty(), egui::Button::new("Import"))
            .clicked()
        {
            confirm = true;
        }
        if ui.button("Cancel").clicked() {
            close = true;
        }
    });
    (confirm, close)
}

/// Alpha at or below this counts as empty. Antialiased edges leave a halo of 1-3
/// that would otherwise weld neighbouring frames into one island.
const ALPHA_FLOOR: u8 = 8;
/// Islands smaller than this on either axis are stray pixels, not frames.
const MIN_CELL: u32 = 3;

const ATLAS_PREVIEW_KEY: &str = "atlas_preview";

/// The sheet with the grid drawn over it, so the numbers have something to mean.
fn preview(ui: &mut egui::Ui, state: &mut AppState, pending: &mut PendingAtlas) {
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

    let sense = if pending.mode == SliceMode::Rects {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    match handle {
        Some(handle) => {
            egui::Image::new(&handle).paint_at(ui, rect);
        }
        None => {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        }
    }

    // Drawn from the same `cells()` the import uses, so what you see is exactly
    // what will be cut.
    let painter = ui.painter_at(rect);
    let line = egui::Stroke::new(
        1.0,
        egui::Color32::from_rgba_unmultiplied(120, 230, 255, 180),
    );
    let selected_line = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 90));
    let cells = pending.cells();
    // Runtime pixel maths: the rect comes from image data, not from a layout.
    let to_screen = |x: u32, y: u32, w: u32, h: u32| {
        egui::Rect::from_min_size(
            rect.min + egui::vec2(x as f32 * scale, y as f32 * scale),
            egui::vec2(w as f32 * scale, h as f32 * scale),
        )
    };
    for (index, cell) in cells.iter().enumerate() {
        let is_selected = pending.mode == SliceMode::Rects && pending.selected == Some(index);
        painter.rect_stroke(
            to_screen(cell.x, cell.y, cell.w, cell.h),
            0.0,
            if is_selected { selected_line } else { line },
            egui::StrokeKind::Inside,
        );
    }

    if pending.mode != SliceMode::Rects {
        return;
    }

    // The selected rect gets a corner handle. One handle, not eight: with L/T/W/H
    // fields right there, more handles would be more ways to nudge a value by a
    // pixel you did not mean.
    if let Some(index) = pending.selected
        && let Some(cell) = pending.rects.get(index)
    {
        let screen = to_screen(cell.x, cell.y, cell.w, cell.h);
        painter.circle_filled(screen.max, HANDLE, selected_line.color);
    }

    let Some(pointer) = response.interact_pointer_pos() else {
        // A drag that ends outside the image still has to stop dragging.
        if response.drag_stopped() {
            state.session.atlas_drag = None;
        }
        return;
    };
    let in_image = |p: egui::Pos2| {
        (
            ((p.x - rect.min.x) / scale).round().max(0.0) as u32,
            ((p.y - rect.min.y) / scale).round().max(0.0) as u32,
        )
    };

    if response.drag_started() {
        // Corner first: it sits inside its own rect, so hit-testing the body
        // first would make the handle unreachable.
        let mut grabbed = None;
        if let Some(index) = pending.selected
            && let Some(cell) = pending.rects.get(index)
        {
            let screen = to_screen(cell.x, cell.y, cell.w, cell.h);
            if (pointer - screen.max).length() <= HANDLE * 2.0 {
                grabbed = Some((index, AtlasDrag::Resize));
            }
        }
        if grabbed.is_none() {
            // Topmost first, so a rect drawn over another can still be picked.
            for (index, cell) in pending.rects.iter().enumerate().rev() {
                if to_screen(cell.x, cell.y, cell.w, cell.h).contains(pointer) {
                    grabbed = Some((index, AtlasDrag::Move));
                    break;
                }
            }
        }
        if let Some((index, kind)) = grabbed {
            pending.selected = Some(index);
            state.session.atlas_drag = Some(kind);
        }
    }

    if response.dragged()
        && let Some(kind) = state.session.atlas_drag
        && let Some(index) = pending.selected
    {
        let (sheet_w, sheet_h) = (pending.width.max(1), pending.height.max(1));
        let delta = response.drag_delta() / scale;
        if let Some(cell) = pending.rects.get_mut(index) {
            match kind {
                AtlasDrag::Move => {
                    let nx = (cell.x as f32 + delta.x).round().max(0.0) as u32;
                    let ny = (cell.y as f32 + delta.y).round().max(0.0) as u32;
                    cell.x = nx.min(sheet_w.saturating_sub(cell.w));
                    cell.y = ny.min(sheet_h.saturating_sub(cell.h));
                }
                AtlasDrag::Resize => {
                    let (px, py) = in_image(pointer);
                    cell.w = px.saturating_sub(cell.x).max(1).min(sheet_w - cell.x);
                    cell.h = py.saturating_sub(cell.y).max(1).min(sheet_h - cell.y);
                }
            }
        }
    }
    if response.drag_stopped() {
        state.session.atlas_drag = None;
    }
}

/// Radius of the resize handle, and half its grab distance.
const HANDLE: f32 = 4.0;

/// What a drag on the preview is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AtlasDrag {
    Move,
    Resize,
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
    for cell in pending.cells() {
        let (x, y, w, h) = (cell.x, cell.y, cell.w, cell.h);
        if w == 0 || h == 0 || x + w > pending.width || y + h > pending.height {
            continue;
        }
        let cropped = image::imageops::crop_imm(&sheet, x, y, w, h).to_image();
        if pending.skip_empty && cropped.pixels().all(|p| p.0[3] == 0) {
            skipped += 1;
            continue;
        }

        // Re-encoded as PNG: these bytes are new images, not the user's original
        // file, so there is nothing to preserve byte-for-byte.
        let mut bytes = Vec::new();
        if image::DynamicImage::ImageRgba8(cropped)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .is_err()
        {
            continue;
        }
        assets.push(ImageAsset::new(cell.name.clone(), bytes, w, h));
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

    /// Two opaque squares with a transparent gutter: two cells, tight to the
    /// pixels, numbered in reading order.
    #[test]
    fn detect_finds_one_rect_per_island() {
        let mut img = image::RgbaImage::new(20, 10);
        for y in 1..5 {
            for x in 1..5 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        for y in 2..8 {
            for x in 10..16 {
                img.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
            }
        }
        let cells = detect_cells(&img, ALPHA_FLOOR, MIN_CELL, "sheet");
        assert_eq!(cells.len(), 2);
        assert_eq!(
            (cells[0].x, cells[0].y, cells[0].w, cells[0].h),
            (1, 1, 4, 4)
        );
        assert_eq!(
            (cells[1].x, cells[1].y, cells[1].w, cells[1].h),
            (10, 2, 6, 6)
        );
        assert_eq!(cells[0].name, "sheet_00");
        assert_eq!(cells[1].name, "sheet_01");
    }

    /// Antialiased edges leave a halo of alpha 1-3. Counting it as opaque welds
    /// neighbouring frames into one island, which is the whole reason there is a
    /// floor rather than a `> 0` test.
    #[test]
    fn a_faint_halo_does_not_bridge_two_frames() {
        let mut img = image::RgbaImage::new(12, 6);
        for y in 1..5 {
            for x in 1..4 {
                img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
            for x in 8..11 {
                img.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
            }
            // The bridge.
            for x in 4..8 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 3]));
            }
        }
        assert_eq!(detect_cells(&img, ALPHA_FLOOR, MIN_CELL, "s").len(), 2);
        assert_eq!(
            detect_cells(&img, 0, MIN_CELL, "s").len(),
            1,
            "without a floor the halo welds them"
        );
    }

    #[test]
    fn stray_pixels_are_not_frames() {
        let mut img = image::RgbaImage::new(10, 10);
        img.put_pixel(5, 5, image::Rgba([255, 255, 255, 255]));
        assert!(detect_cells(&img, ALPHA_FLOOR, MIN_CELL, "s").is_empty());
    }

    /// Switching to manual rects carries the grid over. "The grid is right except
    /// for two frames" is the usual reason anyone reaches for rects at all.
    #[test]
    fn adopting_the_grid_produces_the_same_cells() {
        let mut atlas = sheet(64, 64);
        atlas.columns = 4;
        atlas.rows = 4;
        let from_grid = atlas.cells();
        atlas.adopt_grid();
        atlas.mode = SliceMode::Rects;
        assert_eq!(atlas.cells(), from_grid);
        assert_eq!(atlas.cells().len(), 16);
    }

    #[test]
    fn rect_mode_uses_its_own_list_not_the_grid() {
        let mut atlas = sheet(64, 64);
        atlas.mode = SliceMode::Rects;
        atlas.rects = vec![Cell {
            name: "hand".into(),
            x: 3,
            y: 4,
            w: 5,
            h: 6,
        }];
        let cells = atlas.cells();
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].name, "hand");
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
