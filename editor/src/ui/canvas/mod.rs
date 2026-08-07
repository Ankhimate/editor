pub mod camera;
pub mod hover_label;
pub mod outline;
pub mod overlays;
pub mod renderer;
pub mod tools;

use crate::app_state::AppState;
use crate::theme::Theme;
use eframe::egui;
use tools::create_bone::CreateBoneTool;
use tools::select::SelectTool;
use tools::weight_paint::WeightPaintTool;
use tools::{CanvasTool, ToolContext};

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &Theme,
    grid: &crate::config::GridSettings,
    hover_labels: bool,
) {
    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

    // 1. Handle Navigation. The camera gets first claim on pointer input; when it
    // takes it (panning, zooming, or the cursor is over the zoom bar) the tool is
    // skipped so a single gesture never drives two things at once.
    let camera_consumed_input = camera::handle_navigation(ui, &response, rect, state);

    // Releasing the mouse must always end an in-flight gizmo drag, even on a
    // frame the camera claims input — otherwise `dragging_gizmo` stays latched
    // and the bone keeps following the cursor after the button is up.
    if camera_consumed_input
        && state.session.dragging_gizmo != crate::session::GizmoInteraction::None
        && ui.input(|i| i.pointer.primary_released())
    {
        state.session.dragging_gizmo = crate::session::GizmoInteraction::None;
        state.session.drag_start_world_pos = None;
        // Commit whatever the drag had staged, as one undo step.
        let committed: Vec<(ankhimate_core::ids::BoneId, ankhimate_core::math::Transform)> = state
            .session
            .preview_locals
            .iter()
            .map(|(bone, local)| (bone, *local))
            .collect();
        state.session.clear_previews();
        for (bone, local) in committed {
            state.commit_bone_pose(bone, local);
        }
        state.refresh_pose();
    }

    // Likewise, a pan started mid-bone-drag must not leave a ghost preview bone
    // hanging around on the canvas.
    if camera_consumed_input && !ui.input(|i| i.pointer.primary_down()) {
        state.session.clear_previews();
    }

    // 2. Dispatch to current Tool
    if !camera_consumed_input {
        let mut tool_ctx = ToolContext {
            ui,
            response: &response,
            rect,
            state,
        };

        // Setup-only tools are inert while animating (T-207). The toolbar
        // disables them and the mode switch resets the active tool, so this is
        // the belt to that suspenders — a tool must never author rig structure
        // from an Animate-mode drag.
        let setup = tool_ctx.state.session.can_edit_structure();
        match tool_ctx.state.session.tool {
            crate::session::Tool::Select => {
                SelectTool.update(&mut tool_ctx);
            }
            crate::session::Tool::CreateBone if setup => {
                CreateBoneTool.update(&mut tool_ctx);
            }
            crate::session::Tool::WeightPaint if setup => {
                WeightPaintTool.update(&mut tool_ctx);
            }
            _ => SelectTool.update(&mut tool_ctx),
        }
    }

    // 2b. Image drop-import (T-301).
    handle_dropped_files(ui, rect, state);

    // 3. Draw Grid
    overlays::draw_grid(ui, rect, state, theme, grid);

    // 4. Render artwork + bones. Textures are decoded first because the upload
    // needs `&mut state` while the render pass reads it immutably.
    let uploads = renderer::prepare_textures(state);
    // Traced before painting, because painting only reads.
    outline::warm_cache(state);
    renderer::render_bones(ui, rect, state, theme, uploads);

    // 5. Draw UI Overlays (Zoom Bar)
    overlays::draw_zoom_bar(ui, rect, state);

    // 7. Mode chrome (T-207). The viewport must never be ambiguous about where
    // an edit will land, so each mode gets its own border and corner chip:
    //   Setup   — neutral chip, no border; the rig as authored.
    //   Animate — accent border (red while auto-key is armed, the universal
    //             "recording" cue), clip name + frame in the corner.
    draw_mode_chrome(ui, rect, state);

    // 8. Name whatever the cursor is over (T-913). Last, so the label is above
    // the artwork, the gizmos and the weight overlay — a tooltip that something
    // else can cover is not doing its job.
    hover_label::draw(ui, rect, state, hover_labels);
}

/// Import images dropped onto the viewport (T-301).
///
/// Each file becomes an asset + slot + region attachment on the selected bone
/// (or the first root), positioned where it was dropped — the gesture means "put
/// this on the rig", so one drop is one undo step that produces something
/// visible.
fn handle_dropped_files(ui: &egui::Ui, rect: egui::Rect, state: &mut AppState) {
    let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return;
    }
    if !state.session.can_edit_structure() {
        state
            .session
            .set_status("Switch to Setup mode to import images (Tab)");
        return;
    }

    // Attach to the selected bone, else the first root — a rig always has
    // somewhere to hang art, and silently doing nothing would be worse.
    let Some(bone) = state.session.active_bone().or_else(|| {
        state.doc.skeleton.update_order.iter().copied().find(|&id| {
            state
                .doc
                .skeleton
                .bones
                .get(id)
                .is_some_and(|b| b.parent.is_none())
        })
    }) else {
        state
            .session
            .set_status("Create a bone first, then drop an image onto it");
        return;
    };

    // Drop point in world space, expressed relative to the target bone so the
    // image lands under the cursor rather than at the bone's origin.
    let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
    let world = pointer
        .map(|p| camera::screen_to_world(p, rect, state))
        .unwrap_or_default();
    let local = state
        .pose
        .worlds
        .get(bone)
        .and_then(|w| w.invert())
        .map(|inv| inv.transform_point(world))
        .unwrap_or(glam::Vec2::ZERO);

    for file in dropped {
        let Some(path) = file.path.clone() else {
            continue;
        };
        import_image_file(state, &path, bone, local);
    }
}

/// Read an image file and import it onto `bone` — the one path both the drop
/// handler and the Assets panel's Import button go through.
pub fn import_image_file(
    state: &mut AppState,
    path: &std::path::Path,
    bone: ankhimate_core::ids::BoneId,
    offset: glam::Vec2,
) {
    match std::fs::read(path) {
        Ok(bytes) => import_image_bytes(state, path, bytes, bone, offset),
        Err(e) => state
            .session
            .set_status(format!("Could not read {}: {e}", path.display())),
    }
}

/// Decode enough of an image to know its size, then import it as one command.
fn import_image_bytes(
    state: &mut AppState,
    path: &std::path::Path,
    bytes: Vec<u8>,
    bone: ankhimate_core::ids::BoneId,
    offset: glam::Vec2,
) {
    let (width, height) = match image::load_from_memory(&bytes) {
        Ok(img) => (img.width(), img.height()),
        Err(e) => {
            state
                .session
                .set_status(format!("{} is not a supported image: {e}", path.display()));
            return;
        }
    };

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .to_string();
    let mut asset = ankhimate_core::assets::ImageAsset::new(name, bytes, width, height);
    asset.source_path = Some(path.to_string_lossy().into_owned());

    let cmd = crate::commands::asset_cmds::ImportImage::new(asset, bone, offset);
    // The import appends its slot, so the new artwork is the last draw-order
    // entry. Select it: the user just placed it, so it is what the inspector
    // should be talking about.
    if state.dispatch(Box::new(cmd))
        && let Some(&slot) = state.doc.skeleton.draw_order.last()
    {
        state.session.select_slot(Some(slot));
    }
}

fn draw_mode_chrome(ui: &egui::Ui, rect: egui::Rect, state: &AppState) {
    let painter = ui.painter();
    let animating = state.session.is_animating();

    if animating {
        let color = if state.session.auto_key {
            egui::Color32::from_rgb(230, 60, 60)
        } else {
            egui::Color32::from_rgb(230, 170, 60)
        };
        painter.rect_stroke(
            rect.shrink(1.0),
            0.0,
            egui::Stroke::new(2.0, color),
            egui::StrokeKind::Inside,
        );
    }

    // Corner chip: what mode, and — while animating — which clip and frame.
    let (label, chip) = if animating {
        let clip = state
            .session
            .active_animation
            .and_then(|id| state.doc.animations.get(id))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "—".into());
        let frame = (state.session.playhead * state.doc.meta.fps.max(1) as f32).round() as i64;
        (
            format!("ANIMATE · {clip} · f{frame}"),
            egui::Color32::from_rgb(230, 60, 60),
        )
    } else {
        (
            "SETUP POSE".to_string(),
            ui.visuals().weak_text_color().gamma_multiply(0.8),
        )
    };

    let pos = rect.left_top() + egui::vec2(10.0, 10.0);
    let galley = painter.layout_no_wrap(label, egui::FontId::proportional(11.0), chip);
    let bg = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(12.0, 6.0));
    painter.rect_filled(
        bg,
        egui::epaint::CornerRadius::same(3),
        ui.visuals().extreme_bg_color.gamma_multiply(0.85),
    );
    painter.galley(pos + egui::vec2(6.0, 3.0), galley, chip);

    // Unkeyed edits are easy to lose; say so where the user is looking.
    let mut below = bg.left_bottom() + egui::vec2(0.0, 6.0);
    if animating && state.session.has_pending_pose() {
        painter.text(
            below,
            egui::Align2::LEFT_TOP,
            "unkeyed pose — press K",
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgb(230, 170, 60),
        );
        below.y += 17.0;
    }

    // Isolation is a state the user can forget they are in, and forgetting it
    // means concluding the rig is broken (T-903). So the badge is loud, always
    // on screen while it applies, and says how to leave — a quiet indicator
    // would be worse than none, because it would look like the rig.
    if state.session.is_isolating() {
        let n = state.session.isolated_bones.len();
        let color = egui::Color32::from_rgb(120, 190, 255);
        let galley = painter.layout_no_wrap(
            format!("ISOLATED · {n} bone(s) · Shift+H to exit"),
            egui::FontId::proportional(11.0),
            color,
        );
        let chip = egui::Rect::from_min_size(below, galley.size() + egui::vec2(12.0, 6.0));
        painter.rect_filled(
            chip,
            egui::epaint::CornerRadius::same(3),
            ui.visuals().extreme_bg_color.gamma_multiply(0.9),
        );
        painter.rect_stroke(
            chip,
            egui::epaint::CornerRadius::same(3),
            egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        painter.galley(below + egui::vec2(6.0, 3.0), galley, color);
    }
}
