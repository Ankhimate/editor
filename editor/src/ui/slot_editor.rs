//! The slot-space editor: one attachment, on its own, in its bone's frame.
//!
//! Double-clicking a piece of art opens it here, the way double-clicking a smart
//! object opens it in its own document. The rest of the rig is gone, the bone's
//! origin is the origin, and the only thing on screen is the piece being placed.
//!
//! That matters because placing art in the main viewport means fighting it: the
//! piece is one of sixty, usually overlapped by two others, and the handles you
//! want are a few pixels across at whatever zoom the rig happens to be at. Here
//! the piece fills the pane and the handles are the size of handles.
//!
//! What it edits is the attachment's **local** transform — offset, rotation,
//! scale and pivot within the slot's bone — which is rig data, so Setup mode
//! only, same as the inspector's fields.

use crate::app_state::AppState;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::SlotId;
use ankhimate_document::commands::attachment_cmds::{RegionProps, SetRegionProps, owning_skin};
use eframe::egui;

/// Screen radius of a draggable handle.
const HANDLE_R: f32 = 5.0;
/// How far the rotate ring sits outside the quad's corner.
const ROTATE_GAP: f32 = 22.0;

/// What the pane is editing, and how it is framed.
#[derive(Clone)]
pub struct SlotEdit {
    pub slot: SlotId,
    pub attachment: String,
    /// Screen pixels per local unit. `None` until the first paint fits it.
    pub zoom: Option<f32>,
    /// Pan, in local units.
    pub center: glam::Vec2,
}

impl SlotEdit {
    pub fn new(slot: SlotId, attachment: impl Into<String>) -> Self {
        Self {
            slot,
            attachment: attachment.into(),
            zoom: None,
            center: glam::Vec2::ZERO,
        }
    }
}

/// What a drag on the pane is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Grab {
    Offset,
    Pivot,
    /// A corner, by index into `local_corners` (TL, BL, BR, TR).
    Corner(usize),
    Rotate,
}

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(edit) = state.session.slot_edit.clone() else {
        empty(
            ui,
            "Nothing open",
            "Double-click a piece of art in the viewport to place it here",
        );
        return;
    };
    let Some(skin) = owning_skin(
        &state.doc,
        state.session.active_skin,
        edit.slot,
        &edit.attachment,
    ) else {
        empty(ui, "That attachment is gone", "");
        return;
    };
    let Some(Attachment::Region(region)) = state.doc.skeleton.skins[skin]
        .get(edit.slot, &edit.attachment)
        .cloned()
    else {
        empty(
            ui,
            "Not a placeable attachment",
            "Meshes are shaped on the main viewport, not placed here",
        );
        return;
    };

    header(ui, state, &edit, &region);

    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    checkerboard(&painter, rect, ui.visuals());

    // Fit on first open: the piece should fill the pane without anyone reaching
    // for a zoom control to see what they just opened.
    let mut edit = edit;
    let zoom = match edit.zoom {
        Some(zoom) => zoom,
        None => {
            let extent = region
                .local_corners()
                .iter()
                .fold(0.0_f32, |acc, c| acc.max(c.x.abs()).max(c.y.abs()))
                .max(region.width.max(region.height) * 0.5)
                .max(1.0);
            let fit = (rect.width().min(rect.height()) * 0.35) / extent;
            edit.zoom = Some(fit);
            fit
        }
    };

    // Local space is Y-up (PLAN §2.2); screen is Y-down. The pan is copied out
    // so the closures do not borrow `edit`, which the navigation below mutates.
    let center = edit.center;
    let to_screen = move |p: glam::Vec2| {
        egui::pos2(
            rect.center().x + (p.x - center.x) * zoom,
            rect.center().y - (p.y - center.y) * zoom,
        )
    };
    let to_local = move |p: egui::Pos2| {
        glam::vec2(
            (p.x - rect.center().x) / zoom + center.x,
            (rect.center().y - p.y) / zoom + center.y,
        )
    };

    axes(&painter, rect, &to_screen, ui.visuals());
    draw_art(ui, &painter, state, &region, &to_screen);

    let corners = region.local_corners();
    let screen_corners: Vec<egui::Pos2> = corners.iter().map(|c| to_screen(*c)).collect();
    let pivot_screen = to_screen(region.local_offset);

    // Outline and handles.
    let accent = ui.visuals().selection.bg_fill;
    painter.add(egui::Shape::closed_line(
        screen_corners.clone(),
        egui::Stroke::new(1.5, accent),
    ));
    for corner in &screen_corners {
        painter.circle_filled(*corner, HANDLE_R, accent);
        painter.circle_stroke(
            *corner,
            HANDLE_R,
            egui::Stroke::new(1.0, ui.visuals().extreme_bg_color),
        );
    }
    // The pivot: a ring plus a cross, so it never reads as a fifth corner.
    painter.circle_stroke(
        pivot_screen,
        HANDLE_R + 2.0,
        egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 200, 90)),
    );
    for axis in [egui::vec2(7.0, 0.0), egui::vec2(0.0, 7.0)] {
        painter.line_segment(
            [pivot_screen - axis, pivot_screen + axis],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 200, 90)),
        );
    }
    // The rotate ring hangs off the top-right corner, outside the quad so it
    // never competes with the corner handle underneath it.
    let rotate_screen = screen_corners[3] + egui::vec2(ROTATE_GAP, -ROTATE_GAP);
    painter.circle_stroke(
        rotate_screen,
        HANDLE_R,
        egui::Stroke::new(1.5, egui::Color32::from_rgb(140, 200, 255)),
    );
    painter.line_segment(
        [screen_corners[3], rotate_screen],
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgb(140, 200, 255).gamma_multiply(0.5),
        ),
    );

    // ── Navigation ───────────────────────────────────────────────────────
    if ui.rect_contains_pointer(rect) {
        let (scroll, zoom_gesture) = ui
            .ctx()
            .input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = if (zoom_gesture - 1.0).abs() > 1e-4 {
            zoom_gesture
        } else if scroll != 0.0 {
            (scroll * 0.0025).exp()
        } else {
            1.0
        };
        if (factor - 1.0).abs() > 1e-6 {
            edit.zoom = Some((zoom * factor).clamp(0.05, 200.0));
        }
    }
    if response.dragged_by(egui::PointerButton::Middle) {
        let delta = response.drag_delta();
        edit.center -= glam::vec2(delta.x / zoom, -delta.y / zoom);
    }

    // ── Handles ──────────────────────────────────────────────────────────
    let setup = state.session.can_edit_structure();
    let mut props = RegionProps::from_region(&region);
    let mut changed = false;

    if setup {
        if response.drag_started()
            && let Some(p) = response.interact_pointer_pos()
        {
            let near = |q: egui::Pos2| (p - q).length() <= HANDLE_R * 2.4;
            // Pivot first, then rotate, then corners, then the body: each sits
            // on top of the next, so testing the other way round would make the
            // small handles unreachable.
            let grab = if near(pivot_screen) {
                Some(Grab::Pivot)
            } else if near(rotate_screen) {
                Some(Grab::Rotate)
            } else {
                screen_corners
                    .iter()
                    .position(|c| near(*c))
                    .map(Grab::Corner)
                    .or_else(|| inside(&screen_corners, p).then_some(Grab::Offset))
            };
            state.session.slot_edit_grab_kind = grab.map(grab_code);
        }
        if response.drag_stopped() {
            state.session.slot_edit_grab_kind = None;
        }

        if let (Some(code), Some(p)) = (
            state.session.slot_edit_grab_kind,
            response.interact_pointer_pos(),
        ) && response.dragged()
        {
            let local = to_local(p);
            match grab_from_code(code) {
                Grab::Offset => {
                    props.offset = local;
                    changed = true;
                }
                Grab::Pivot => {
                    // Moving the pivot must not move the art: `with_pivot_keeping_position`
                    // compensates the offset, which is the whole reason it exists.
                    let size = glam::vec2(props.width, props.height) * props.scale;
                    if size.x.abs() > 1e-4 && size.y.abs() > 1e-4 {
                        let (sin, cos) = (-props.rotation).sin_cos();
                        let d = local - props.offset;
                        let unrotated = glam::vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
                        let pivot = glam::vec2(
                            (unrotated.x / size.x + 0.5).clamp(-1.0, 2.0),
                            (unrotated.y / size.y + 0.5).clamp(-1.0, 2.0),
                        );
                        props = props.with_pivot_keeping_position(pivot);
                        changed = true;
                    }
                }
                Grab::Corner(_) => {
                    // Scale from the pivot: the distance the cursor is from it,
                    // over the distance the corner started at.
                    let d = local - props.offset;
                    let (sin, cos) = (-props.rotation).sin_cos();
                    let unrotated = glam::vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
                    let half = glam::vec2(props.width, props.height) * 0.5;
                    if half.x.abs() > 1e-4 && half.y.abs() > 1e-4 {
                        let scale = glam::vec2(
                            (unrotated.x / half.x).abs().max(0.01),
                            (unrotated.y / half.y).abs().max(0.01),
                        );
                        // Shift keeps the aspect, which is what you want nine
                        // times in ten and impossible to hit by hand otherwise.
                        props.scale = if ui.input(|i| i.modifiers.shift) {
                            glam::Vec2::splat((scale.x + scale.y) * 0.5)
                        } else {
                            scale
                        };
                        changed = true;
                    }
                }
                Grab::Rotate => {
                    let d = local - props.offset;
                    if d.length() > 1e-3 {
                        // The handle sits at the top-right corner, so subtract
                        // where that corner is to keep the art from jumping when
                        // the drag starts.
                        let corner = corners[3] - region.local_offset;
                        props.rotation =
                            d.y.atan2(d.x) - corner.y.atan2(corner.x) + region.local_rotation;
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        state.dispatch(Box::new(SetRegionProps::new(
            skin,
            edit.slot,
            edit.attachment.clone(),
            props,
        )));
    }

    readout(ui, rect, &region, zoom);
    state.session.slot_edit = Some(edit);
}

/// Grab kinds survive a frame in session state, so they travel as a small code
/// rather than dragging a UI-private enum into `Session`.
fn grab_code(grab: Grab) -> usize {
    match grab {
        Grab::Offset => 0,
        Grab::Pivot => 1,
        Grab::Rotate => 2,
        Grab::Corner(i) => 3 + i,
    }
}

fn grab_from_code(code: usize) -> Grab {
    match code {
        0 => Grab::Offset,
        1 => Grab::Pivot,
        2 => Grab::Rotate,
        other => Grab::Corner(other - 3),
    }
}

fn header(
    ui: &mut egui::Ui,
    state: &mut AppState,
    edit: &SlotEdit,
    region: &ankhimate_core::attachment::RegionAttachment,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(crate::ui::icons::IMAGE)
                .size(13.0)
                .color(ui.visuals().selection.bg_fill),
        );
        let slot_name = state
            .doc
            .skeleton
            .slots
            .get(edit.slot)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        ui.label(
            egui::RichText::new(format!("{slot_name} › {}", edit.attachment))
                .strong()
                .size(12.0),
        );
        ui.label(
            egui::RichText::new(&region.texture)
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(crate::ui::icons::CLOSE)
                .on_hover_text("Close")
                .clicked()
            {
                state.session.slot_edit = None;
            }
            if ui
                .button(crate::ui::icons::FIT)
                .on_hover_text("Fit to view")
                .clicked()
                && let Some(edit) = state.session.slot_edit.as_mut()
            {
                edit.zoom = None;
                edit.center = glam::Vec2::ZERO;
            }
        });
    });
    if !state.session.can_edit_structure() {
        ui.label(
            egui::RichText::new("Animating — switch to Setup (Tab) to place art")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
    }
    ui.separator();
}

/// The bone's own axes. The origin is the whole point of this pane: everything
/// here is measured from where the bone is, not from where the art happens to be.
fn axes(
    painter: &egui::Painter,
    rect: egui::Rect,
    to_screen: &impl Fn(glam::Vec2) -> egui::Pos2,
    visuals: &egui::Visuals,
) {
    let origin = to_screen(glam::Vec2::ZERO);
    let faint = visuals.weak_text_color().gamma_multiply(0.5);
    painter.line_segment(
        [
            egui::pos2(rect.left(), origin.y),
            egui::pos2(rect.right(), origin.y),
        ],
        egui::Stroke::new(1.0, faint),
    );
    painter.line_segment(
        [
            egui::pos2(origin.x, rect.top()),
            egui::pos2(origin.x, rect.bottom()),
        ],
        egui::Stroke::new(1.0, faint),
    );
    painter.circle_filled(origin, 3.0, visuals.weak_text_color());
}

/// The artwork itself, drawn into the quad's four corners.
fn draw_art(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    state: &mut AppState,
    region: &ankhimate_core::attachment::RegionAttachment,
    to_screen: &impl Fn(glam::Vec2) -> egui::Pos2,
) {
    let Some(handle) = texture(ui.ctx(), state, &region.texture) else {
        return;
    };
    let corners = region.local_corners().map(to_screen);
    // Corner order is TL, BL, BR, TR; UVs follow, with v running down.
    let uv = [
        egui::pos2(0.0, 0.0),
        egui::pos2(0.0, 1.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(1.0, 0.0),
    ];
    let mut mesh = egui::Mesh::with_texture(handle.id());
    for i in 0..4 {
        mesh.colored_vertex(corners[i], egui::Color32::WHITE);
        mesh.vertices[i].uv = uv[i];
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// The full-resolution texture for an asset, cached.
///
/// Separate from the assets panel's thumbnails: this pane zooms in on a piece to
/// place it by eye, and a 96px thumbnail blown up is exactly the wrong thing to
/// judge a pivot against.
fn texture(ctx: &egui::Context, state: &mut AppState, name: &str) -> Option<egui::TextureHandle> {
    let id = state.doc.assets.by_name(name)?;
    let asset = state.doc.assets.get(id)?;
    let key = format!("slotedit:{}:{}x{}", asset.name, asset.width, asset.height);
    if let Some(handle) = state.session.thumbnails.get(&key) {
        return Some(handle.clone());
    }
    let rgba = image::load_from_memory(&asset.bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    let handle = ctx.load_texture(&key, image, egui::TextureOptions::LINEAR);
    state.session.thumbnails.insert(key, handle.clone());
    Some(handle)
}

/// The numbers, bottom-left, so a drag can be checked without leaving the pane.
fn readout(
    ui: &egui::Ui,
    rect: egui::Rect,
    region: &ankhimate_core::attachment::RegionAttachment,
    zoom: f32,
) {
    let text = format!(
        "offset {:.1}, {:.1}   ·   rot {:.1}°   ·   scale {:.3}, {:.3}   ·   pivot {:.2}, {:.2}   ·   {:.0}%",
        region.local_offset.x,
        region.local_offset.y,
        region.local_rotation.to_degrees(),
        region.local_scale.x,
        region.local_scale.y,
        region.pivot.x,
        region.pivot.y,
        zoom * 100.0,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 8.0, rect.bottom() - 8.0),
        egui::Align2::LEFT_BOTTOM,
        text,
        egui::FontId::monospace(10.0),
        ui.visuals().weak_text_color(),
    );
}

/// A transparency checkerboard, so art with soft edges is judged against
/// something other than a flat panel colour.
fn checkerboard(painter: &egui::Painter, rect: egui::Rect, visuals: &egui::Visuals) {
    const CELL: f32 = 10.0;
    painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
    let light = visuals.extreme_bg_color.gamma_multiply(1.6);
    let cols = (rect.width() / CELL).ceil() as i32;
    let rows = (rect.height() / CELL).ceil() as i32;
    for row in 0..rows {
        for col in 0..cols {
            if (row + col) % 2 == 0 {
                continue;
            }
            let min = rect.min + egui::vec2(col as f32 * CELL, row as f32 * CELL);
            let cell = egui::Rect::from_min_size(min, egui::vec2(CELL, CELL)).intersect(rect);
            painter.rect_filled(cell, 0.0, light);
        }
    }
}

/// Is `p` inside the quad? Two triangles, same test the canvas picker uses.
fn inside(corners: &[egui::Pos2], p: egui::Pos2) -> bool {
    if corners.len() < 4 {
        return false;
    }
    let tri = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
        let sign = |p: egui::Pos2, q: egui::Pos2, r: egui::Pos2| (q - p).rot90().dot(r - p);
        let (d1, d2, d3) = (sign(a, b, p), sign(b, c, p), sign(c, a, p));
        !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
    };
    tri(corners[0], corners[1], corners[2]) || tri(corners[0], corners[2], corners[3])
}

fn empty(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(16.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(12.0)
                .color(ui.visuals().weak_text_color()),
        );
        if !hint.is_empty() {
            ui.label(
                egui::RichText::new(hint)
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grab kind crosses a frame boundary as a plain number, so the round
    /// trip has to be exact — a corner that comes back as the body would move
    /// the art instead of resizing it.
    #[test]
    fn every_grab_survives_the_round_trip() {
        for grab in [
            Grab::Offset,
            Grab::Pivot,
            Grab::Rotate,
            Grab::Corner(0),
            Grab::Corner(3),
        ] {
            assert_eq!(grab_from_code(grab_code(grab)), grab, "{grab:?}");
        }
    }

    #[test]
    fn grab_codes_are_distinct() {
        let codes: Vec<usize> = [
            Grab::Offset,
            Grab::Pivot,
            Grab::Rotate,
            Grab::Corner(0),
            Grab::Corner(1),
            Grab::Corner(2),
            Grab::Corner(3),
        ]
        .into_iter()
        .map(grab_code)
        .collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "two handles share a code");
    }

    #[test]
    fn a_point_inside_the_quad_is_inside() {
        let quad = [
            egui::pos2(0.0, 0.0),
            egui::pos2(0.0, 10.0),
            egui::pos2(10.0, 10.0),
            egui::pos2(10.0, 0.0),
        ];
        assert!(inside(&quad, egui::pos2(5.0, 5.0)));
        assert!(!inside(&quad, egui::pos2(15.0, 5.0)));
    }
}
