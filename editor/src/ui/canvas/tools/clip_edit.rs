//! Clip polygon editing on the canvas (T-405).
//!
//! The same gestures as [`super::mesh_edit`] — drag a vertex, Ctrl+drag to
//! box-select, click an edge to insert, `X` to delete, `Esc` to leave — because
//! a user who has learned one should not have to learn the other.
//!
//! It is a separate module rather than a generalisation of the mesh tool because
//! the two differ in everything except the gestures: a clip has no UVs, no
//! weights, no skinning, no deform timeline, and its vertices are a **ring**
//! where a mesh's are a set. Insertion in particular has to respect perimeter
//! order here (a point goes *between* its neighbours) and must not in a mesh.
//! One function serving both would be a pile of branches on which kind it is.

use super::ToolContext;
use crate::commands::EditCommand;
use crate::commands::attachment_cmds::owning_skin;
use crate::commands::clip_cmds::{ClipEdit, EditBoundingBox, EditClip, EditPath};
use ankhimate_core::attachment::{Attachment, ClippingAttachment};
use ankhimate_core::ids::{SkinId, SlotId};
use eframe::egui;

const VERTEX_HIT: f32 = 9.0;
const EDGE_HIT: f32 = 8.0;

pub struct ClipTarget {
    pub skin: SkinId,
    pub slot: SlotId,
    pub name: String,
    pub clip: ClippingAttachment,
    pub world: ankhimate_core::transforms::Affine2,
    /// Which attachment the polygon belongs to. All three edit with the same
    /// gestures; they differ in whether the ring closes and which command the
    /// edit routes to.
    pub kind: PolygonKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolygonKind {
    Clip,
    /// Open rather than closed (T-502).
    Path,
    BoundingBox,
}

impl PolygonKind {
    /// Does the last vertex join back to the first?
    pub fn closed(self) -> bool {
        !matches!(self, PolygonKind::Path)
    }
}

/// Resolve the selected slot's clip, if clip editing applies right now.
///
/// Setup-only: a clip is rig structure, and the command refuses in Animate
/// anyway — resolving no target keeps the handles off screen instead of
/// offering a drag that will be rejected.
pub fn target(state: &crate::app_state::AppState) -> Option<ClipTarget> {
    if !state.session.mesh_edit || !state.session.can_edit_structure() {
        return None;
    }
    let slot_id = state.session.active_slot()?;
    let slot = state.doc.skeleton.slots.get(slot_id)?;
    let name = slot.attachment.clone()?;
    let skin = owning_skin(&state.doc, state.session.active_skin, slot_id, &name)?;
    // Clips and paths are both "a polygon on a slot" as far as the gestures go;
    // a path is simply open rather than closed.
    let (clip, kind) = match state.doc.skeleton.skins[skin].get(slot_id, &name)? {
        Attachment::Clipping(clip) => (clip.clone(), PolygonKind::Clip),
        Attachment::Path(path) => (
            ClippingAttachment {
                vertices: path.vertices.clone(),
                end_slot: None,
            },
            PolygonKind::Path,
        ),
        Attachment::BoundingBox(bb) => (
            ClippingAttachment {
                vertices: bb.vertices.clone(),
                end_slot: None,
            },
            PolygonKind::BoundingBox,
        ),
        _ => return None,
    };
    let world = *state.pose.worlds.get(slot.bone)?;
    Some(ClipTarget {
        skin,
        slot: slot_id,
        name,
        clip,
        world,
        kind,
    })
}

/// Polygon vertices in canvas-local screen space, in ring order.
pub fn vertex_screen_positions(
    target: &ClipTarget,
    state: &crate::app_state::AppState,
    viewport_size: glam::Vec2,
) -> Vec<glam::Vec2> {
    target
        .clip
        .vertices
        .iter()
        .map(|v| {
            let world = target.world.transform_point(*v);
            state.session.camera.world_to_screen(world, viewport_size)
        })
        .collect()
}

/// The right command for whichever kind of polygon this is.
fn edit_command(target: &ClipTarget, edit: ClipEdit) -> Box<dyn EditCommand> {
    let (skin, slot, name) = (target.skin, target.slot, target.name.clone());
    match target.kind {
        PolygonKind::Path => Box::new(EditPath::new(skin, slot, name, edit)),
        PolygonKind::Clip => Box::new(EditClip::new(skin, slot, name, edit)),
        PolygonKind::BoundingBox => Box::new(EditBoundingBox::new(skin, slot, name, edit)),
    }
}

pub fn update(ctx: &mut ToolContext, mouse_screen: Option<glam::Vec2>) {
    let viewport_size = glam::Vec2::new(ctx.rect.width(), ctx.rect.height());
    let Some(target) = target(ctx.state) else {
        return;
    };
    let positions = vertex_screen_positions(&target, ctx.state, viewport_size);

    let (delete, escape) = ctx.ui.input(|i| {
        (
            i.key_pressed(egui::Key::X) || i.key_pressed(egui::Key::Delete),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if escape {
        ctx.state.session.mesh_edit = false;
        ctx.state.session.selected_vertices.clear();
        return;
    }
    if delete && !ctx.state.session.selected_vertices.is_empty() {
        let indices = ctx.state.session.selected_vertices.clone();
        if ctx
            .state
            .dispatch(edit_command(&target, ClipEdit::RemoveVertices(indices)))
        {
            ctx.state.session.selected_vertices.clear();
        } else if target.kind == PolygonKind::Path {
            ctx.state
                .session
                .set_status("A path needs at least two vertices");
        } else if target.kind == PolygonKind::BoundingBox {
            ctx.state
                .session
                .set_status("A bounding box needs at least three vertices");
        } else {
            ctx.state
                .session
                .set_status("A clip needs at least three vertices");
        }
        return;
    }

    // ── Drag ─────────────────────────────────────────────────────────────
    if ctx.ui.input(|i| i.pointer.primary_released()) {
        ctx.state.session.dragging_vertex = None;
        if let Some(start) = ctx.state.session.vertex_box_start.take()
            && let Some(end) = mouse_screen
        {
            let (min, max) = (start.min(end), start.max(end));
            if !ctx.ui.input(|i| i.modifiers.shift) {
                ctx.state.session.selected_vertices.clear();
            }
            for (index, position) in positions.iter().enumerate() {
                if position.x >= min.x
                    && position.x <= max.x
                    && position.y >= min.y
                    && position.y <= max.y
                    && !ctx.state.session.selected_vertices.contains(&index)
                {
                    ctx.state.session.selected_vertices.push(index);
                }
            }
        }
    }
    if ctx.state.session.vertex_box_start.is_some() {
        return;
    }

    if let (Some(index), Some(mouse)) = (ctx.state.session.dragging_vertex, mouse_screen) {
        let world = ctx
            .state
            .session
            .camera
            .screen_to_world(mouse, viewport_size);
        if let Some(inverse) = target.world.invert() {
            let local = inverse.transform_point(world);
            let Some(anchor) = target.clip.vertices.get(index).copied() else {
                return;
            };
            let selected = if ctx.state.session.selected_vertices.contains(&index) {
                ctx.state.session.selected_vertices.clone()
            } else {
                vec![index]
            };
            let delta = local - anchor;
            let moves: Vec<(usize, glam::Vec2)> = selected
                .iter()
                .filter_map(|&i| target.clip.vertices.get(i).map(|v| (i, *v + delta)))
                .collect();
            ctx.state
                .dispatch(edit_command(&target, ClipEdit::MoveVertices(moves)));
        }
        return;
    }

    // ── Press ────────────────────────────────────────────────────────────
    if !(ctx.response.hovered() && ctx.ui.input(|i| i.pointer.primary_pressed())) {
        return;
    }
    let Some(mouse) = mouse_screen else {
        return;
    };
    if ctx.ui.input(|i| i.modifiers.ctrl) {
        ctx.state.session.vertex_box_start = Some(mouse);
        return;
    }

    let nearest = positions
        .iter()
        .enumerate()
        .map(|(i, p)| (i, (*p - mouse).length()))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((index, distance)) = nearest
        && distance <= VERTEX_HIT
    {
        if ctx.ui.input(|i| i.modifiers.shift) {
            if let Some(at) = ctx
                .state
                .session
                .selected_vertices
                .iter()
                .position(|v| *v == index)
            {
                ctx.state.session.selected_vertices.remove(at);
            } else {
                ctx.state.session.selected_vertices.push(index);
            }
        } else if !ctx.state.session.selected_vertices.contains(&index) {
            ctx.state.session.selected_vertices = vec![index];
        }
        ctx.state.session.dragging_vertex = Some(index);
        return;
    }

    // Not on a vertex: a click near an edge splits it. The new point goes
    // *between* that edge's endpoints — a ring has an order, and appending
    // would cut a corner across the polygon instead.
    // A clip is a closed ring, a path is not: an open path has one fewer edge,
    // and offering the phantom last→first one would insert a vertex on a
    // segment that is not drawn.
    let count = positions.len();
    let edges = if !target.kind.closed() {
        count.saturating_sub(1)
    } else {
        count
    };
    let mut best: Option<(usize, f32)> = None;
    for i in 0..edges {
        let (a, b) = (positions[i], positions[(i + 1) % count]);
        let distance = distance_to_segment(mouse, a, b);
        if best.is_none_or(|(_, d)| distance < d) {
            best = Some((i, distance));
        }
    }
    if let Some((edge, distance)) = best
        && distance <= EDGE_HIT
        && let Some(inverse) = target.world.invert()
    {
        let world = ctx
            .state
            .session
            .camera
            .screen_to_world(mouse, viewport_size);
        let local = inverse.transform_point(world);
        if ctx.state.dispatch(edit_command(
            &target,
            ClipEdit::InsertVertex(edge + 1, local),
        )) {
            ctx.state.session.selected_vertices = vec![edge + 1];
        }
        return;
    }

    ctx.state.session.selected_vertices.clear();
}

fn distance_to_segment(p: glam::Vec2, a: glam::Vec2, b: glam::Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-6 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}
