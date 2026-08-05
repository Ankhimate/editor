//! The UV editing pane (T-401).
//!
//! A mesh has two shapes: where its vertices sit in the world, and where they
//! sample the texture. The canvas edits the first. Nothing edited the second, so
//! a traced or hand-built mesh was stuck with UVs projected from its bounding
//! box — fine for a flat quad, wrong the moment the art is not axis-aligned
//! inside its own image.
//!
//! Shown as a window rather than a panel: it needs the texture at a usable size,
//! and it is opened to fix something specific and then closed.

use crate::app_state::AppState;
use crate::commands::mesh_cmds::{EditMesh, MeshEdit};
use crate::theme::Theme;
use ankhimate_core::attachment::{Attachment, MeshAttachment};
use ankhimate_core::ids::{SkinId, SlotId};
use eframe::egui;

/// Which mesh the pane is editing. Session state — closing it leaves nothing.
#[derive(Clone)]
pub struct UvPane {
    pub skin: SkinId,
    pub slot: SlotId,
    pub name: String,
    /// Index being dragged, if any.
    pub dragging: Option<usize>,
}

impl UvPane {
    pub fn new(skin: SkinId, slot: SlotId, name: String) -> Self {
        Self {
            skin,
            slot,
            name,
            dragging: None,
        }
    }
}

/// Grab radius for a UV handle, in screen pixels.
const HANDLE_HIT: f32 = 8.0;
const TEXTURE_KEY: &str = "uv_pane_texture";

pub fn ui(ctx: &egui::Context, state: &mut AppState, theme: &Theme) {
    let Some(pane) = state.session.uv_pane.clone() else {
        return;
    };
    let Some(mesh) = resolve(state, &pane).cloned() else {
        // The attachment went away underneath the window — a skin switch, an
        // undo. Closing is the only honest response.
        state.session.uv_pane = None;
        return;
    };

    let mut close = false;
    let mut reset = false;
    let mut moved: Option<(usize, glam::Vec2)> = None;
    let mut released = false;

    let dialog = crate::ui::dialog::Dialog::new("uv_editor", "UV editor")
        .icon(crate::ui::icons::MESH)
        .width(660.0)
        .show(ctx, theme, |ui| {
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
                             texture. The selection from the canvas is highlighted.",
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
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
                    ui.add_space(4.0);
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
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
    close |= dialog.closed;
    if close {
        state.session.uv_pane = None;
        state.session.thumbnails.remove(TEXTURE_KEY);
    }
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

/// The texture with the mesh drawn over it in UV space.
fn canvas(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &Theme,
    pane: &UvPane,
    mesh: &MeshAttachment,
    moved: &mut Option<(usize, glam::Vec2)>,
) -> egui::Response {
    const SIZE: f32 = 340.0;

    let (rect, response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);

    if let Some(handle) = texture(ui, state, mesh) {
        egui::Image::new(&handle).paint_at(ui, rect);
    }

    // UV space is (0,0) top-left, (1,1) bottom-right — the same convention the
    // sampler uses, so what is dragged here is literally what is sampled.
    let to_screen = |uv: glam::Vec2| {
        egui::pos2(
            rect.min.x + uv.x * rect.width(),
            rect.min.y + uv.y * rect.height(),
        )
    };
    let to_uv = |p: egui::Pos2| {
        glam::Vec2::new(
            ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0),
            ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0),
        )
    };

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

    let dragging = pane.dragging;
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

    // Pick on press, move while held. Same shape as the canvas tool so the two
    // panes do not need different muscle memory.
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let nearest = mesh
            .uvs
            .iter()
            .enumerate()
            .map(|(i, uv)| (i, (to_screen(*uv) - pos).length()))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, distance)) = nearest
            && distance <= HANDLE_HIT
            && let Some(pane) = state.session.uv_pane.as_mut()
        {
            pane.dragging = Some(index);
        }
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
