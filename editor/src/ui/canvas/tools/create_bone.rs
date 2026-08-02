use super::{CanvasTool, ToolContext, update_hover_state};
use crate::commands::EditCommand;
use crate::commands::bone_cmds::CreateBone;
use crate::ui::canvas::camera::screen_to_world;
use crate::ui::canvas::overlays::zoom_bar_rect;
use eframe::egui;

pub struct CreateBoneTool;

impl CanvasTool for CreateBoneTool {
    fn update(&mut self, ctx: &mut ToolContext) {
        update_hover_state(ctx);
        ctx.ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);

        let primary_pressed = ctx
            .ui
            .input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        let primary_down = ctx
            .ui
            .input(|i| i.pointer.button_down(egui::PointerButton::Primary));
        let primary_released = ctx
            .ui
            .input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        let mouse_pos = ctx.ui.input(|i| i.pointer.latest_pos());

        let zoom_bar_rect = zoom_bar_rect(ctx.rect);

        // Start: place bone base at exact click position
        if primary_pressed && ctx.response.contains_pointer() {
            // Don't start bone creation if clicking on the zoom bar overlay
            let over_overlay = mouse_pos.is_some_and(|p| zoom_bar_rect.contains(p));
            if !over_overlay && let Some(pos) = mouse_pos {
                let click_world = screen_to_world(pos, ctx.rect, ctx.state);

                // If a parent bone is selected, start the new bone from the parent's tip
                let start_pos = match ctx.state.session.active_bone() {
                    Some(parent_id) if ctx.state.doc.skeleton.bones.contains_key(parent_id) => {
                        ctx.state.pose.world_tip(&ctx.state.doc.skeleton, parent_id)
                    }
                    _ => click_world,
                };

                ctx.state.session.preview_bone = Some((start_pos, click_world));
            }
        }

        // Drag: update the preview end point
        if primary_down
            && ctx.state.session.preview_bone.is_some()
            && let Some(pos) = mouse_pos
        {
            let current_world = screen_to_world(pos, ctx.rect, ctx.state);
            if let Some(preview) = &mut ctx.state.session.preview_bone {
                preview.1 = current_world;
            }
        }

        // Release: commit the bone
        if primary_released
            && let Some((start_world, end_world)) = ctx.state.session.preview_bone.take()
        {
            let delta = end_world - start_world;
            let length = delta.length();

            // Only create the bone if dragged a minimum distance (prevents accidental micro-bones)
            if length > 2.0 / ctx.state.session.camera.zoom {
                // Rotation: angle from base to tip in world space
                let world_rot = f32::atan2(delta.y, delta.x);

                // Calculate local transform relative to selected parent
                let mut local_pos = start_world;
                let mut local_rot = world_rot;

                if let Some(parent_id) = ctx.state.session.active_bone()
                    && ctx.state.doc.skeleton.bones.contains_key(parent_id)
                {
                    // World → parent-local via the affine inverse (a
                    // zero-scaled parent has no inverse; keep the world
                    // values in that case).
                    let parent_world = ctx.state.pose.world(parent_id);
                    if let Some(inv) = parent_world.invert() {
                        local_pos = inv.transform_point(start_world);
                    }
                    local_rot = world_rot - parent_world.decompose().rotation;
                }

                let new_bone = ankhimate_core::skeleton::Bone {
                    name: format!("Bone {}", ctx.state.doc.skeleton.bones.len()),
                    parent: ctx.state.session.active_bone(),
                    length,
                    local_transform: ankhimate_core::math::Transform {
                        position: local_pos,
                        rotation: local_rot,
                        scale: glam::Vec2::new(1.0, 1.0),
                        shear: glam::Vec2::ZERO,
                    },
                    inherit: Default::default(),
                    color: ankhimate_core::skeleton::Bone::default_color(),
                };

                // One undoable command; `CreateBone` reports the id it assigned.
                let mut cmd = CreateBone::new(new_bone);
                cmd.apply(&mut ctx.state.doc);
                let new_id = cmd.created_id();
                ctx.state.dispatch_applied(Box::new(cmd));

                // Auto-select new bone for chaining (like Spine)
                ctx.state.session.select_bone(new_id);
            }
        }
    }
}
