//! Mesh vertex editing on the canvas (T-401).
//!
//! Active for the selected slot's mesh attachment while `Session::mesh_edit` is
//! on. Interactions, deliberately few:
//!
//! * click a vertex to select it, drag to move it (one undo step per drag);
//!   a drag moves the whole selection, not just the grabbed vertex;
//! * **Ctrl+drag** anywhere box-selects, shift-drag adds to the selection;
//! * shift-click toggles one vertex;
//! * click an edge to insert a vertex there;
//! * `C` with two vertices selected pins the edge between them, or releases it
//!   if it is already pinned (T-401);
//! * `X` or `Delete` removes the selection;
//! * `Escape` leaves mesh mode.
//!
//! Box-select is on Ctrl rather than on a bare drag from empty space: in a dense
//! mesh — precisely when a box is wanted — there is no empty space to start from,
//! so every attempt grabbed a vertex instead.
//!
//! The **mode decides what a drag writes** (T-207): in Setup it moves the mesh's
//! own vertices via `EditMesh`; in Animate it keys their offsets as a deform
//! (T-404), leaving the authored shape alone. Structural edits — inserting and
//! deleting vertices — are Setup-only, enforced by the command rather than
//! re-checked here.

use super::ToolContext;
use ankhimate_core::attachment::{Attachment, MeshAttachment};
use ankhimate_core::ids::{SkinId, SlotId};
use ankhimate_document::commands::attachment_cmds::owning_skin;
use ankhimate_document::commands::mesh_cmds::{EditMesh, MeshEdit};
use eframe::egui;

/// Screen-space grab radius for a vertex.
pub const VERTEX_HIT: f32 = 9.0;
/// How close to an edge a click must land to insert a vertex there.
const EDGE_HIT: f32 = 8.0;

/// The mesh the tool is editing, with everything needed to address it.
pub struct MeshTarget {
    pub skin: SkinId,
    pub slot: SlotId,
    pub name: String,
    pub mesh: MeshAttachment,
    /// Bone→world affine, for moving between local and screen space.
    pub world: ankhimate_core::transforms::Affine2,
}

/// Resolve the selected slot's mesh, if mesh editing applies right now.
///
/// Available in **both** modes: Setup edits the mesh's own vertices, Animate
/// keys their offsets as a deform (T-404). The gate is only that mesh editing
/// is switched on and a slot with a mesh is selected.
pub fn target(state: &crate::app_state::AppState) -> Option<MeshTarget> {
    if !state.session.mesh_edit {
        return None;
    }
    let slot_id = state.session.active_slot()?;
    let slot = state.doc.skeleton.slots.get(slot_id)?;
    let name = slot.attachment.clone()?;
    let skin = owning_skin(&state.doc, state.session.active_skin, slot_id, &name)?;
    let Attachment::Mesh(mesh) = state.doc.skeleton.skins[skin].get(slot_id, &name)? else {
        return None;
    };
    let world = *state.pose.worlds.get(slot.bone)?;
    Some(MeshTarget {
        skin,
        slot: slot_id,
        name,
        mesh: mesh.clone(),
        world,
    })
}

/// Vertex positions in canvas-local screen space, parallel to `setup_vertices`.
pub fn vertex_screen_positions(
    target: &MeshTarget,
    state: &crate::app_state::AppState,
    viewport_size: glam::Vec2,
) -> Vec<glam::Vec2> {
    // Handles must sit on the *drawn* vertices, deform and skinning included, or
    // grabbing one in Animate mode would mean aiming at where it used to be.
    let deform = state.pose.deforms.get(&(target.slot, target.name.clone()));
    (0..target.mesh.setup_vertices.len())
        .map(|i| {
            let offset = deform.and_then(|d| d.get(i).copied()).unwrap_or_default();
            let world = target
                .mesh
                .skin_vertex_with_ffd(i, offset, &state.pose, &target.world);
            state.session.camera.world_to_screen(world, viewport_size)
        })
        .collect()
}

pub fn update(ctx: &mut ToolContext, mouse_screen: Option<glam::Vec2>) {
    let viewport_size = glam::Vec2::new(ctx.rect.width(), ctx.rect.height());
    let Some(target) = target(ctx.state) else {
        // Nothing to edit means nothing under the cursor to name (T-913); a
        // stale index here would have the label describing a mesh that is no
        // longer open.
        ctx.state.session.hovered_vertex = None;
        return;
    };
    let positions = vertex_screen_positions(&target, ctx.state, viewport_size);

    // Which vertex a click would take. Recorded rather than recomputed at paint
    // time (T-913): the renderer's highlight and the hover label are two readers
    // of one answer, and two searches could disagree by a frame.
    ctx.state.session.hovered_vertex = mouse_screen.and_then(|mouse| {
        positions
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (*p - mouse).length()))
            .filter(|(_, d)| *d <= VERTEX_HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });

    // ── Keyboard ─────────────────────────────────────────────────────────
    let (delete, escape, edge) = ctx.ui.input(|i| {
        (
            i.key_pressed(egui::Key::X) || i.key_pressed(egui::Key::Delete),
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::C),
        )
    });
    // `C` with exactly two vertices selected toggles the edge between them:
    // pin it if the triangulation keeps wandering, release it if the pin was
    // the mistake. One key both ways — there is nothing to remember.
    if edge {
        let selected = ctx.state.session.selected_vertices.clone();
        if let [a, b] = selected[..] {
            let pinned = {
                let edge = [(a.min(b)) as u32, (a.max(b)) as u32];
                target.mesh.edges.contains(&edge)
            };
            let edit = if pinned {
                MeshEdit::RemoveEdge(a, b)
            } else {
                MeshEdit::AddEdge(a, b)
            };
            if ctx.state.dispatch(Box::new(EditMesh::new(
                target.skin,
                target.slot,
                target.name.clone(),
                edit,
            ))) {
                ctx.state.session.set_status(if pinned {
                    "Edge released"
                } else {
                    "Edge pinned"
                });
            }
        } else {
            ctx.state
                .session
                .set_status("Select exactly two vertices to pin an edge between them");
        }
        return;
    }
    if escape {
        ctx.state.session.mesh_edit = false;
        ctx.state.session.selected_vertices.clear();
        return;
    }
    if delete && !ctx.state.session.selected_vertices.is_empty() {
        let indices = ctx.state.session.selected_vertices.clone();
        let removed = ctx.state.dispatch(Box::new(EditMesh::new(
            target.skin,
            target.slot,
            target.name.clone(),
            MeshEdit::RemoveVertices(indices),
        )));
        if removed {
            ctx.state.session.selected_vertices.clear();
        }
        // The command refuses to go below three vertices; say so rather than
        // leaving the user wondering why nothing happened.
        if target.mesh.setup_vertices.len() < 4 {
            ctx.state
                .session
                .set_status("A mesh needs at least three vertices");
        }
        return;
    }

    // ── Drag ─────────────────────────────────────────────────────────────
    if ctx.ui.input(|i| i.pointer.primary_released()) {
        ctx.state.session.dragging_vertex = None;
        // Finish a box: everything inside joins the selection.
        if let Some(start) = ctx.state.session.vertex_box_start.take()
            && let Some(end) = mouse_screen
        {
            let (min, max) = (start.min(end), start.max(end));
            // Shift extends; a plain Ctrl-drag replaces, so the common case —
            // "select this cluster" — does not accumulate the last one.
            let additive = ctx.ui.input(|i| i.modifiers.shift);
            if !additive {
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

    // A box in progress just draws (the renderer reads `vertex_box_start`); no
    // vertex moves until it closes.
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
            let Some(anchor) = target.mesh.setup_vertices.get(index).copied() else {
                return;
            };
            // Everything selected moves together, rigidly, by the delta the
            // grabbed vertex travelled — dragging one of five picked vertices
            // and moving only that one would make multi-select pointless.
            let selected = if ctx.state.session.selected_vertices.contains(&index) {
                ctx.state.session.selected_vertices.clone()
            } else {
                vec![index]
            };
            let delta = local - anchor;

            if ctx.state.session.is_animating() {
                key_deform(ctx, &target, &selected, delta);
            } else {
                let moves: Vec<(usize, glam::Vec2)> = selected
                    .iter()
                    .filter_map(|&i| target.mesh.setup_vertices.get(i).map(|v| (i, *v + delta)))
                    .collect();
                ctx.state.dispatch(Box::new(EditMesh::new(
                    target.skin,
                    target.slot,
                    target.name.clone(),
                    MeshEdit::MoveVertices(moves),
                )));
            }
        }
        return;
    }

    // ── Press: pick a vertex, or split an edge ───────────────────────────
    if !(ctx.response.hovered() && ctx.ui.input(|i| i.pointer.primary_pressed())) {
        return;
    }
    let Some(mouse) = mouse_screen else {
        return;
    };

    // Ctrl starts a box wherever the cursor is, vertex or not. Deciding by
    // what is under the pointer meant that inside a dense mesh — which is
    // exactly when a box is wanted — the press always landed on a vertex and
    // dragged it instead.
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
        let additive = ctx.ui.input(|i| i.modifiers.shift);
        if additive {
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

    // Not on a vertex: an edge click inserts. Distances are measured in screen
    // space so the target is the same size at any zoom.
    let world = ctx
        .state
        .session
        .camera
        .screen_to_world(mouse, viewport_size);
    let Some(inverse) = target.world.invert() else {
        return;
    };
    let local = inverse.transform_point(world);
    if let Some((i, j, closest_local, _)) =
        ankhimate_document::meshgen::nearest_edge(&target.mesh, local)
    {
        let a = positions.get(i).copied().unwrap_or_default();
        let b = positions.get(j).copied().unwrap_or_default();
        let screen_distance = distance_to_segment(mouse, a, b);
        if screen_distance <= EDGE_HIT {
            ctx.state.dispatch(Box::new(EditMesh::new(
                target.skin,
                target.slot,
                target.name.clone(),
                MeshEdit::AddVertex(closest_local),
            )));
            // Select what was just made — it is almost always the next thing to
            // be moved.
            let new_index = target.mesh.setup_vertices.len();
            ctx.state.session.selected_vertices = vec![new_index];
            return;
        }
    }

    // Empty space with no modifier: drop the selection, the same as clicking off
    // a bone.
    ctx.state.session.selected_vertices.clear();
}

/// Key the whole mesh's offsets at the playhead, with `index` moved to `local`.
///
/// A deform key holds every vertex, not just the moved one: the sampler
/// interpolates whole shapes, so a key that listed one vertex would snap the
/// rest back to setup the moment it took effect.
fn key_deform(ctx: &mut ToolContext, target: &MeshTarget, indices: &[usize], delta: glam::Vec2) {
    use ankhimate_document::commands::key_cmds::AddDeformKey;

    let Some(anim) = ctx.state.session.active_animation else {
        return;
    };
    // Start from what is on screen — any existing deform at this time — so
    // dragging a second vertex does not discard the first.
    let mut offsets: Vec<glam::Vec2> = ctx
        .state
        .pose
        .deforms
        .get(&(target.slot, target.name.clone()))
        .cloned()
        .unwrap_or_else(|| vec![glam::Vec2::ZERO; target.mesh.setup_vertices.len()]);
    offsets.resize(target.mesh.setup_vertices.len(), glam::Vec2::ZERO);

    for &index in indices {
        if let Some(offset) = offsets.get_mut(index) {
            *offset += delta;
        }
    }

    ctx.state.dispatch(Box::new(AddDeformKey::new(
        anim,
        target.slot,
        target.name.clone(),
        ctx.state.session.playhead,
        offsets,
    )));
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
