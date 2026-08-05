//! Weight painting on the canvas (T-403).
//!
//! Paint the **selected bone's** influence onto the **selected slot's** mesh.
//! Click a bone to switch which influence you are painting; drag to paint.
//!
//! The pre-T-403 version mutated the document directly, so a stroke could not be
//! undone. Every dab now goes through `PaintWeights`, which merges the stroke
//! into one history entry and clears the binds so they are recaptured from the
//! setup pose — that recapture is what keeps the mesh from lurching when a new
//! bone starts influencing a vertex.

use super::{CanvasTool, ToolContext};
use crate::commands::attachment_cmds::owning_skin;
use crate::commands::weight_cmds::{PaintWeights, brush};
use ankhimate_core::attachment::Attachment;

pub struct WeightPaintTool;

impl CanvasTool for WeightPaintTool {
    fn update(&mut self, ctx: &mut ToolContext) {
        super::update_hover_state(ctx);

        // Clicking a bone re-aims the brush rather than moving anything.
        if ctx.response.clicked()
            && let Some(hovered) = ctx.state.session.hovered_bone
        {
            ctx.state.session.select_bone(Some(hovered));
            return;
        }

        if !ctx.response.dragged() {
            return;
        }
        let (Some(bone), Some(slot_id)) = (
            ctx.state.session.active_bone(),
            ctx.state.session.active_slot(),
        ) else {
            ctx.state
                .session
                .set_status("Select a bone and a slot to paint weights");
            return;
        };
        let Some(mouse) = ctx.ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };

        let cursor_screen = glam::vec2(mouse.x - ctx.rect.min.x, mouse.y - ctx.rect.min.y);
        let cursor_world = ctx.state.session.camera.screen_to_world(
            cursor_screen,
            glam::vec2(ctx.rect.width(), ctx.rect.height()),
        );

        // Resolve the mesh the same way every other attachment edit does, so a
        // stroke never forks an override into a skin the user is not looking at.
        let Some(name) = ctx
            .state
            .doc
            .skeleton
            .slots
            .get(slot_id)
            .and_then(|s| s.attachment.clone())
        else {
            return;
        };
        let Some(skin) = owning_skin(
            &ctx.state.doc,
            ctx.state.session.active_skin,
            slot_id,
            &name,
        ) else {
            return;
        };
        let Some(Attachment::Mesh(mesh)) = ctx.state.doc.skeleton.skins[skin].get(slot_id, &name)
        else {
            ctx.state
                .session
                .set_status("This slot has no mesh — convert it first");
            return;
        };

        let slot_bone = ctx.state.doc.skeleton.slots[slot_id].bone;
        let Some(bone_world) = ctx.state.pose.worlds.get(slot_bone).copied() else {
            return;
        };

        // Distances are measured where the vertices are *drawn*, so the brush
        // covers what is under the cursor even on a posed rig.
        let distances: Vec<f32> = (0..mesh.setup_vertices.len())
            .map(|i| {
                let world =
                    mesh.skin_vertex_with_ffd(i, glam::Vec2::ZERO, &ctx.state.pose, &bone_world);
                (cursor_world - world).length()
            })
            .collect();

        let settings = &ctx.state.session.weight_paint_settings;
        let weights = brush(
            mesh,
            bone,
            settings.mode,
            &distances,
            settings.radius,
            settings.strength,
            settings.feather,
            &settings.locked,
        );
        ctx.state
            .dispatch(Box::new(PaintWeights::new(skin, slot_id, name, weights)));
    }
}
