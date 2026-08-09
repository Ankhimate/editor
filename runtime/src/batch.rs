//! Draw batches (T-604) — geometry a renderer can hand to a GPU.
//!
//! The output is deliberately dumb: positions, UVs, colours, indices, in draw
//! order. No wgpu types, no texture handles, no assumptions about coordinate
//! conventions beyond core's own (Y up, PLAN §2.2). A game maps
//! [`DrawBatch::texture`] to whatever its renderer calls a texture and uploads
//! the vertices.
//!
//! Every position here comes from [`Pose`], and the skinning maths comes from
//! [`Pose::skinned_vertex`] — the same call the editor's viewport makes. That is
//! the point: a mesh that looks right while animating must look right in the
//! game, and two implementations of "where does this vertex go" guarantee it
//! eventually will not.

use crate::Rig;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::SkinId;
use ankhimate_core::pose::Pose;

/// How a batch composites.
///
/// Re-exported from core rather than redefined: a runtime enum that drifted from
/// the document's would render a slot differently in the game than in the
/// editor, silently.
pub use ankhimate_core::slot::BlendMode as Blend;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub u: f32,
    pub v: f32,
    /// Straight RGBA, 0..=1, already multiplied by the slot's tint.
    pub color: [f32; 4],
}

/// One draw call's worth of geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawBatch {
    /// Asset name the geometry samples. Names, not ids (ADR 0004) — the host
    /// maps them to its own textures.
    pub texture: String,
    pub blend: Blend,
    /// Slot name, so a game can attach behaviour to a specific part.
    pub slot: String,
    pub vertices: Vec<Vertex>,
    /// Triangle indices into `vertices`.
    pub indices: Vec<u32>,
    /// Two-colour tint, when the slot has one (T-505). `None` is the common
    /// case and costs a renderer nothing.
    pub dark_color: Option<[f32; 4]>,
}

/// Build the batches for a posed rig, in draw order.
///
/// Batches are **not** merged across slots even when they share a texture:
/// draw order is the whole point of a slot list, and merging two slots that a
/// third sits between would silently reorder the rig.
pub fn build_batches(rig: &Rig, pose: &Pose, active_skins: &[SkinId]) -> Vec<DrawBatch> {
    let mut batches = Vec::new();

    for &slot_id in &pose.draw_order {
        let Some(slot) = rig.skeleton.slots.get(slot_id) else {
            continue;
        };
        // An explicitly hidden slot draws nothing. Absent means visible, which
        // is the common case.
        if pose.slot_visible.get(slot_id).is_some_and(|v| !v) {
            continue;
        }
        let Some(name) = pose.attachment_name(&rig.skeleton, slot_id) else {
            continue;
        };
        let Some(attachment) = rig
            .skeleton
            .resolve_many(active_skins, slot_id, name)
            .or_else(|| rig.skeleton.resolve_slot_many(active_skins, slot_id))
        else {
            continue;
        };

        let color = pose
            .slot_colors
            .get(slot_id)
            .copied()
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let dark_color = pose.slot_dark_colors.get(slot_id).copied();
        let blend = slot.blend_mode;

        let batch = match attachment {
            Attachment::Region(region) => {
                let Some(world) = pose.worlds.get(slot.bone) else {
                    continue;
                };
                let corners = region.local_corners().map(|c| world.transform_point(c));
                let uv = &region.uv_rect;
                // local_corners is TL, BL, BR, TR; UVs follow the same order so
                // the quad is not mirrored.
                let uvs = [
                    (uv.x, uv.y),
                    (uv.x, uv.y + uv.h),
                    (uv.x + uv.w, uv.y + uv.h),
                    (uv.x + uv.w, uv.y),
                ];
                let vertices = corners
                    .iter()
                    .zip(uvs)
                    .map(|(p, (u, v))| Vertex {
                        x: p.x,
                        y: p.y,
                        u,
                        v,
                        color,
                    })
                    .collect();
                DrawBatch {
                    texture: region.texture.clone(),
                    blend,
                    slot: slot.name.clone(),
                    vertices,
                    indices: vec![0, 1, 2, 0, 2, 3],
                    dark_color,
                }
            }
            Attachment::Mesh(mesh) => {
                // A linked mesh borrows another's geometry (T-802), so resolve
                // through the link before reading vertices.
                let source = rig.skeleton.resolve_linked_mesh(active_skins, mesh);
                let deform = pose.deforms.get(&(slot_id, name.to_string()));

                let mut vertices = Vec::with_capacity(source.setup_vertices.len());
                for (i, &local) in source.setup_vertices.iter().enumerate() {
                    let offset = deform
                        .and_then(|d| d.get(i))
                        .copied()
                        .unwrap_or(glam::Vec2::ZERO);
                    let local = local + offset;

                    let world = match source.weights.get(i) {
                        Some(weights) if !weights.is_empty() => pose.skinned_vertex(weights, local),
                        // Unweighted: rigid to the slot's own bone.
                        _ => match pose.worlds.get(slot.bone) {
                            Some(world) => world.transform_point(local),
                            None => local,
                        },
                    };
                    let uv = source.uvs.get(i).copied().unwrap_or(glam::Vec2::ZERO);
                    vertices.push(Vertex {
                        x: world.x,
                        y: world.y,
                        u: uv.x,
                        v: uv.y,
                        color,
                    });
                }

                DrawBatch {
                    texture: mesh.texture.clone(),
                    blend,
                    slot: slot.name.clone(),
                    vertices,
                    indices: source.triangles.iter().flatten().copied().collect(),
                    dark_color,
                }
            }
            // Clipping, bounding boxes, paths and points draw nothing. They are
            // real rig features with runtime meaning — a game reads bounding
            // boxes for hit tests — but they are not geometry, and returning
            // them as empty batches would make every consumer filter them out.
            _ => continue,
        };

        if !batch.vertices.is_empty() && !batch.indices.is_empty() {
            batches.push(batch);
        }
    }

    batches
}
