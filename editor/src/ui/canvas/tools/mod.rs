use super::camera::screen_to_world;
use crate::app_state::AppState;
use eframe::egui;

pub mod clip_edit;
pub mod create_bone;
pub mod mesh_edit;
pub mod select;
pub mod weight_paint;

pub struct ToolContext<'a> {
    pub ui: &'a mut egui::Ui,
    pub response: &'a egui::Response,
    pub rect: egui::Rect,
    pub state: &'a mut AppState,
}

pub trait CanvasTool {
    fn update(&mut self, ctx: &mut ToolContext);
}

// ── Hit Testing ─────────────────────────────────────────────────────────

/// Compute shortest distance from point p to line segment ab.
pub fn distance_point_to_segment(p: glam::Vec2, a: glam::Vec2, b: glam::Vec2) -> f32 {
    let ab = b - a;
    let length_sq = ab.length_squared();
    if length_sq < 0.000001 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / length_sq).clamp(0.0, 1.0);
    let projection = a + t * ab;
    (p - projection).length()
}

/// Hit-test a screen-space cursor point against a bone's screen-space center line.
pub fn hit_test_bone(
    cursor_screen: glam::Vec2,
    camera: &crate::ui::canvas::camera::Camera2D,
    viewport_size: glam::Vec2,
    skeleton: &ankhimate_core::skeleton::Skeleton,
    pose: &ankhimate_core::pose::Pose,
    bone_id: ankhimate_core::ids::BoneId,
) -> f32 {
    let origin_screen = camera.world_to_screen(pose.world_position(bone_id), viewport_size);
    let tip_screen = camera.world_to_screen(pose.world_tip(skeleton, bone_id), viewport_size);
    distance_point_to_segment(cursor_screen, origin_screen, tip_screen)
}

/// Shared logic to update hover state for all tools
pub fn update_hover_state(ctx: &mut ToolContext) {
    ctx.state.session.hovered_bone = None;
    // A hidden bone is not on screen to click. Half-hiding it — invisible but
    // still stealing clicks from the art underneath — is worse than not hiding
    // it at all.
    if !ctx.state.session.show_bones {
        return;
    }
    if ctx.response.hovered()
        && let Some(mouse_pos) = ctx.ui.input(|i| i.pointer.hover_pos())
    {
        let cursor_screen =
            glam::Vec2::new(mouse_pos.x - ctx.rect.min.x, mouse_pos.y - ctx.rect.min.y);
        let viewport_size = glam::Vec2::new(ctx.rect.width(), ctx.rect.height());

        let mut best_bone = None;
        let mut best_dist = f32::MAX;
        let mut best_depth = 0;

        for (bone_id, bone) in ctx.state.doc.skeleton.bones.iter() {
            let dist = hit_test_bone(
                cursor_screen,
                &ctx.state.session.camera,
                viewport_size,
                &ctx.state.doc.skeleton,
                &ctx.state.pose,
                bone_id,
            );

            // 8.0px screen tolerance
            if dist <= 8.0 {
                let mut depth = 0;
                let mut curr = bone.parent;
                while let Some(p) = curr {
                    depth += 1;
                    curr = ctx.state.doc.skeleton.bones.get(p).and_then(|b| b.parent);
                }

                // Tie break: if within 1 pixel, prefer deeper in hierarchy (children over parents)
                let is_better = if (dist - best_dist).abs() < 1.0 {
                    depth > best_depth
                } else {
                    dist < best_dist
                };

                if is_better {
                    best_dist = dist;
                    best_depth = depth;
                    best_bone = Some(bone_id);
                }
            }
        }
        ctx.state.session.hovered_bone = best_bone;
    }
}

/// Convert a screen-space egui position to world-space coordinates (re-export
/// so tools can pull it from one place).
pub fn cursor_world(response_rect: egui::Rect, mouse: egui::Pos2, state: &AppState) -> glam::Vec2 {
    screen_to_world(mouse, response_rect, state)
}
