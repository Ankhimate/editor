//! The UV editing pane (T-401).
//!
//! A mesh has two shapes: where its vertices sit in the world, and where they
//! sample the texture. The canvas edits the first. Nothing edited the second, so
//! a traced or hand-built mesh was stuck with UVs projected from its bounding
//! box — fine for a flat quad, wrong the moment the art is not axis-aligned
//! inside its own image.
//!
//! A dockable pane rather than a window. It was a modal dialog, which was wrong
//! twice over: UVs are edited *against* the canvas — you drag a point here and
//! look there to see what moved — and a modal blocks exactly that. As a tab it
//! can sit beside the viewport, be dragged wherever it is useful, and be closed
//! from its own tab like any other.

use crate::app_state::AppState;
use crate::theme::Theme;
use ankhimate_core::attachment::{Attachment, MeshAttachment};
use ankhimate_core::ids::{SkinId, SlotId};
use ankhimate_document::commands::mesh_cmds::{EditMesh, MeshEdit};
use eframe::egui;

/// Which mesh the pane is editing. Session state — closing it leaves nothing.
#[derive(Clone)]
pub struct UvPane {
    pub skin: SkinId,
    pub slot: SlotId,
    pub name: String,
    /// Index being dragged, if any.
    pub dragging: Option<usize>,
    /// Magnification over the fit-to-pane scale. 1.0 shows the whole texture.
    pub zoom: f32,
    /// Offset of the texture's centre from the pane's, in points.
    pub pan: glam::Vec2,
}

impl UvPane {
    pub fn new(skin: SkinId, slot: SlotId, name: String) -> Self {
        Self {
            skin,
            slot,
            name,
            dragging: None,
            zoom: 1.0,
            pan: glam::Vec2::ZERO,
        }
    }

    /// Back to showing the whole texture, centred.
    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = glam::Vec2::ZERO;
    }
}

/// Grab radius for a UV handle, in screen pixels.
const HANDLE_HIT: f32 = 8.0;
const TEXTURE_KEY: &str = "uv_pane_texture";
/// Width of the details column beside the canvas.
const SIDEBAR_W: f32 = 200.0;
/// Zoom limits. The floor keeps the texture findable, the ceiling is a little
/// past where one texel fills a 16px cell — enough to place a vertex exactly.
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 40.0;
/// Side of one checkerboard square, in points.
const CHECKER: f32 = 8.0;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState, theme: &Theme) {
    let Some(pane) = state.session.uv_pane.clone() else {
        // An empty pane says what would fill it. A blank card reads as broken.
        empty(ui);
        return;
    };
    let Some(mesh) = resolve(state, &pane).cloned() else {
        // The attachment went away underneath the pane — a skin switch, an undo.
        // Dropping the target is the only honest response; the tab stays open
        // and goes back to its empty state.
        state.session.uv_pane = None;
        return;
    };

    let mut reset = false;
    let mut moved: Option<(usize, glam::Vec2)> = None;
    let mut released = false;

    ui.horizontal_top(|ui| {
        let response = canvas(ui, state, theme, &pane, &mesh, &mut moved);
        if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
            released = true;
        }

        ui.vertical(|ui| {
            ui.set_min_width(180.0);
            ui.label(egui::RichText::new(&pane.name).strong());
            ui.label(
                egui::RichText::new(&mesh.texture)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(
                    "Drag a point to change where that vertex samples the \
                     texture. Scroll to zoom, middle-drag or drag empty space \
                     to pan.",
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // Read back from the session, not from the clone taken at the
                // top: the canvas has already run this frame and moved it.
                let zoom = state.session.uv_pane.as_ref().map_or(1.0, |p| p.zoom);
                ui.label(
                    egui::RichText::new(format!("{:.0}%", zoom * 100.0))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if ui
                    .button(crate::ui::icons::FIT)
                    .on_hover_text("Fit the whole texture in the pane")
                    .clicked()
                    && let Some(pane) = state.session.uv_pane.as_mut()
                {
                    pane.reset_view();
                }
            });

            ui.add_space(10.0);
            if ui
                .button("Reset UVs")
                .on_hover_text(
                    "Re-project every UV from the mesh's current bounds, \
                     discarding hand edits",
                )
                .clicked()
            {
                reset = true;
            }
        });
    });

    if let Some((index, uv)) = moved {
        // One command per frame of the drag; `EditMesh` merges them, so the
        // whole drag lands as a single undo step.
        state.dispatch(Box::new(EditMesh::new(
            pane.skin,
            pane.slot,
            pane.name.clone(),
            MeshEdit::MoveUvs(vec![(index, uv)]),
        )));
    }
    if released && let Some(pane) = state.session.uv_pane.as_mut() {
        pane.dragging = None;
    }
    if reset {
        state.dispatch(Box::new(EditMesh::new(
            pane.skin,
            pane.slot,
            pane.name.clone(),
            MeshEdit::ResetUvs,
        )));
    }
}

/// Drop whatever the pane was editing, and its cached texture.
///
/// Called when the tab is closed, so reopening it on a different mesh does not
/// come up showing the previous one's art.
pub fn clear(state: &mut AppState) {
    state.session.uv_pane = None;
    state.session.thumbnails.remove(TEXTURE_KEY);
}

fn empty(ui: &mut egui::Ui) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(crate::ui::icons::MESH)
                .size(24.0)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(6.0);
        ui.label(egui::RichText::new("No mesh open").color(ui.visuals().weak_text_color()));
        ui.label(
            egui::RichText::new("Select a mesh attachment and use \"Edit UVs\"")
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    });
}

fn resolve<'a>(state: &'a AppState, pane: &UvPane) -> Option<&'a MeshAttachment> {
    match state
        .doc
        .skeleton
        .skins
        .get(pane.skin)?
        .get(pane.slot, &pane.name)?
    {
        Attachment::Mesh(mesh) => Some(mesh),
        _ => None,
    }
}

/// Where the texture lands inside the viewport.
///
/// Fit first, then magnify. Keeping the fit as the unit means zoom 1.0 always
/// means "the whole thing", whatever the art's proportions — a zoom expressed in
/// texels-per-point would read as a different amount of magnification for every
/// piece of art in the rig.
///
/// The margin keeps the texture's edge off the pane's, so the outermost UVs are
/// grabbable rather than pinned against the border.
fn image_rect(viewport: egui::Rect, texel: egui::Vec2, zoom: f32, pan: glam::Vec2) -> egui::Rect {
    const MARGIN: f32 = 0.92;
    if texel.x <= 0.0 || texel.y <= 0.0 {
        return viewport;
    }
    let fit = (viewport.width() / texel.x).min(viewport.height() / texel.y) * MARGIN;
    egui::Rect::from_center_size(
        viewport.center() + egui::vec2(pan.x, pan.y),
        texel * fit * zoom,
    )
}

/// The texture with the mesh drawn over it in UV space.
///
/// A viewport, not a thumbnail. It used to be a fixed 340px square with the
/// texture stretched to fill it, which distorted every piece of art that was not
/// square — and almost none of them are. A vertex sitting on the tip of a shin in
/// a squashed preview is not obviously the same vertex in the rig, which defeats
/// the point of showing the texture at all.
fn canvas(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &Theme,
    pane: &UvPane,
    mesh: &MeshAttachment,
    moved: &mut Option<(usize, glam::Vec2)>,
) -> egui::Response {
    let size = egui::vec2(
        (ui.available_width() - SIDEBAR_W).max(160.0),
        ui.available_height().max(160.0),
    );
    // Middle-drag pans, so it has to be sensed alongside the left-drag that moves
    // vertices.
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);

    let handle = texture(ui, state, mesh);
    // Falls back to a square when the texture is missing, so the pane still draws
    // the mesh rather than collapsing to nothing.
    let texel = handle
        .as_ref()
        .map(|h| h.size_vec2())
        .unwrap_or(egui::vec2(1.0, 1.0));

    let image_rect = image_rect(rect, texel, pane.zoom, pane.pan);

    // Checkerboard under the art, clipped to it: the pieces have transparent
    // margins, and against a flat dark fill there is no telling where the art
    // ends and the canvas begins.
    let checker = painter.with_clip_rect(image_rect.intersect(rect));
    let (light, dark) = (egui::Color32::from_gray(58), egui::Color32::from_gray(44));
    checker.rect_filled(image_rect, 0.0, dark);
    let cols = (image_rect.width() / CHECKER).ceil() as i32;
    let rows = (image_rect.height() / CHECKER).ceil() as i32;
    // Capped: at deep zoom the board would be tens of thousands of quads, and
    // past a point the squares are smaller than the art's own detail anyway.
    if cols * rows <= 8_000 {
        for row in 0..rows {
            for col in 0..cols {
                if (row + col) % 2 != 0 {
                    continue;
                }
                let min = image_rect.min + egui::vec2(col as f32, row as f32) * CHECKER;
                checker.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(CHECKER, CHECKER)),
                    0.0,
                    light,
                );
            }
        }
    }

    if let Some(handle) = &handle {
        egui::Image::new(handle).paint_at(
            &ui.new_child(egui::UiBuilder::new().max_rect(rect)),
            image_rect,
        );
    }
    // The texture's own edge, so "outside the image" is visible when zoomed in.
    painter.rect_stroke(
        image_rect,
        0.0,
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
        egui::StrokeKind::Outside,
    );

    // UV space is (0,0) top-left, (1,1) bottom-right — the same convention the
    // sampler uses, so what is dragged here is literally what is sampled.
    let to_screen = |uv: glam::Vec2| {
        egui::pos2(
            image_rect.min.x + uv.x * image_rect.width(),
            image_rect.min.y + uv.y * image_rect.height(),
        )
    };
    let to_uv = |p: egui::Pos2| {
        glam::Vec2::new(
            ((p.x - image_rect.min.x) / image_rect.width()).clamp(0.0, 1.0),
            ((p.y - image_rect.min.y) / image_rect.height()).clamp(0.0, 1.0),
        )
    };

    // ── Navigation ──────────────────────────────────────────────────────
    // Zoom toward the cursor, so the texel under the pointer stays under it —
    // the thing being inspected is the thing to keep still.
    if response.hovered()
        && let Some(cursor) = ui.input(|i| i.pointer.hover_pos())
    {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0
            && let Some(pane) = state.session.uv_pane.as_mut()
        {
            let old = pane.zoom;
            pane.zoom = (pane.zoom * (scroll * 0.004).exp()).clamp(ZOOM_MIN, ZOOM_MAX);
            let ratio = pane.zoom / old;
            let from_centre = cursor - image_rect.center();
            let shift = from_centre * (1.0 - ratio);
            pane.pan += glam::vec2(shift.x, shift.y);
        }
    }
    // Pick on press, move while held. Same shape as the canvas tool so the two
    // panes do not need different muscle memory.
    //
    // Resolved before the pan, not after: the press and the first drag frame are
    // the same frame, so a pan that consulted last frame's `dragging` would
    // shove the image sideways at the instant a vertex is grabbed.
    let mut dragging = pane.dragging;
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let nearest = mesh
            .uvs
            .iter()
            .enumerate()
            .map(|(i, uv)| (i, (to_screen(*uv) - pos).length()))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        dragging = nearest
            .filter(|(_, distance)| *distance <= HANDLE_HIT)
            .map(|(index, _)| index);
        if let Some(pane) = state.session.uv_pane.as_mut() {
            pane.dragging = dragging;
        }
    }

    // Middle-drag pans, and so does a left-drag that did not grab a vertex.
    let panning =
        ui.input(|i| i.pointer.middle_down()) || (response.dragged() && dragging.is_none());
    if panning && let Some(pane) = state.session.uv_pane.as_mut() {
        let delta = response.drag_delta();
        pane.pan += glam::vec2(delta.x, delta.y);
    }

    for tri in &mesh.triangles {
        for k in 0..3 {
            let (Some(&a), Some(&b)) = (
                mesh.uvs.get(tri[k] as usize),
                mesh.uvs.get(tri[(k + 1) % 3] as usize),
            ) else {
                continue;
            };
            painter.line_segment(
                [to_screen(a), to_screen(b)],
                egui::Stroke::new(1.0, theme.mesh_edge()),
            );
        }
    }

    for (index, uv) in mesh.uvs.iter().enumerate() {
        let selected = state.session.selected_vertices.contains(&index);
        let radius = if selected || dragging == Some(index) {
            5.0
        } else {
            3.0
        };
        let color = if selected || dragging == Some(index) {
            theme.mesh_vertex_selected()
        } else {
            theme.mesh_vertex()
        };
        painter.circle_filled(to_screen(*uv), radius, color);
    }

    if response.dragged()
        && let Some(index) = dragging
        && let Some(pos) = response.interact_pointer_pos()
    {
        *moved = Some((index, to_uv(pos)));
    }

    response
}

/// The attachment's texture as an egui handle, decoded once and cached.
fn texture(
    ui: &egui::Ui,
    state: &mut AppState,
    mesh: &MeshAttachment,
) -> Option<egui::TextureHandle> {
    if let Some(handle) = state.session.thumbnails.get(TEXTURE_KEY) {
        return Some(handle.clone());
    }
    let id = state.doc.assets.by_name(&mesh.texture)?;
    let bytes = &state.doc.assets.get(id)?.bytes;
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let handle = ui
        .ctx()
        .load_texture(TEXTURE_KEY, color, egui::TextureOptions::LINEAR);
    state
        .session
        .thumbnails
        .insert(TEXTURE_KEY.to_string(), handle.clone());
    Some(handle)
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// The bug this pane was rebuilt to fix: a tall piece drawn into a square
    /// box comes out squashed, and a vertex on the tip of a shin stops looking
    /// like the same vertex it is in the rig.
    #[test]
    fn art_keeps_its_aspect_ratio() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 400.0));
        for texel in [
            egui::vec2(46.0, 97.0),   // tall, like a shin
            egui::vec2(512.0, 64.0),  // wide, like a strip
            egui::vec2(128.0, 128.0), // square
        ] {
            let rect = image_rect(viewport, texel, 1.0, glam::Vec2::ZERO);
            let drawn = rect.width() / rect.height();
            let source = texel.x / texel.y;
            assert!(
                (drawn - source).abs() < 1e-4,
                "{texel:?} drawn at {drawn} but the art is {source}"
            );
        }
    }

    /// Zoom 1.0 must show the whole texture whatever its shape, or "fit" is a
    /// different amount of magnification for every piece in the rig.
    #[test]
    fn zoom_one_fits_inside_the_viewport() {
        let viewport = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(600.0, 400.0));
        for texel in [egui::vec2(46.0, 97.0), egui::vec2(512.0, 64.0)] {
            let rect = image_rect(viewport, texel, 1.0, glam::Vec2::ZERO);
            assert!(
                viewport.contains_rect(rect),
                "{texel:?} overflowed the pane"
            );
        }
    }

    #[test]
    fn pan_offsets_from_the_centre_and_zoom_scales_about_it() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 400.0));
        let texel = egui::vec2(100.0, 50.0);

        let centred = image_rect(viewport, texel, 1.0, glam::Vec2::ZERO);
        assert_eq!(centred.center(), viewport.center());

        let panned = image_rect(viewport, texel, 1.0, glam::vec2(30.0, -12.0));
        assert_eq!(panned.center(), viewport.center() + egui::vec2(30.0, -12.0));
        assert_eq!(panned.size(), centred.size());

        let zoomed = image_rect(viewport, texel, 2.0, glam::Vec2::ZERO);
        assert!((zoomed.width() / centred.width() - 2.0).abs() < 1e-4);
        assert_eq!(zoomed.center(), centred.center());
    }

    /// A missing texture must not divide by zero and hand back a NaN rect.
    #[test]
    fn a_degenerate_texture_falls_back_to_the_viewport() {
        let viewport = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 400.0));
        assert_eq!(
            image_rect(viewport, egui::vec2(0.0, 0.0), 1.0, glam::Vec2::ZERO),
            viewport
        );
    }
}
