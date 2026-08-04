use super::{CanvasTool, ToolContext, update_hover_state};
use eframe::egui;

pub struct SelectTool;

/// Drag the selected slot's artwork instead of its bone (T-307).
///
/// One handle — the pivot crosshair — driven by whichever transform tool is
/// active, plus alt-drag to move the pivot itself. A full second gizmo set would
/// double the on-canvas furniture for an edit that is mostly nudging.
///
/// Unlike bone drags this writes to the document each frame rather than staging
/// a preview: `SetRegionProps` merges, so the whole drag is still one undo step,
/// and it matches what the inspector's spinboxes already do.
fn drag_attachment(ctx: &mut ToolContext, cursor_world: glam::Vec2) -> bool {
    use crate::commands::attachment_cmds::{RegionProps, SetRegionProps, owning_skin};
    use ankhimate_core::attachment::Attachment;

    let Some(slot_id) = ctx.state.session.active_slot() else {
        return false;
    };
    let Some(prev) = ctx.state.session.drag_start_world_pos else {
        return false;
    };
    let Some(name) = ctx
        .state
        .doc
        .skeleton
        .slots
        .get(slot_id)
        .and_then(|s| s.attachment.clone())
    else {
        return false;
    };
    let Some(skin) = owning_skin(
        &ctx.state.doc,
        ctx.state.session.active_skin,
        slot_id,
        &name,
    ) else {
        return false;
    };
    let Some(Attachment::Region(region)) = ctx.state.doc.skeleton.skins[skin].get(slot_id, &name)
    else {
        return false;
    };
    let props = RegionProps::from_region(region);

    // Work in the bone's local space: the attachment's numbers live there, so a
    // sheared or scaled parent does not skew the edit.
    let bone = ctx.state.doc.skeleton.slots[slot_id].bone;
    let Some(bone_world) = ctx.state.pose.worlds.get(bone).copied() else {
        return false;
    };
    let Some(inv) = bone_world.invert() else {
        return false;
    };
    let local_now = inv.transform_point(cursor_world);
    let local_prev = inv.transform_point(prev);
    let delta = local_now - local_prev;

    let alt = ctx.ui.input(|i| i.modifiers.alt);
    let next = if alt {
        // Alt-drag moves the pivot under the cursor, art staying put.
        let size = glam::vec2(props.width, props.height) * props.scale;
        if size.x.abs() < 1e-4 || size.y.abs() < 1e-4 {
            return false;
        }
        let (sin, cos) = (-props.rotation).sin_cos();
        let unrotated = {
            let d = local_now - props.offset;
            glam::vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
        };
        let pivot = props.pivot + unrotated / size;
        props.with_pivot_keeping_position(pivot.clamp(glam::Vec2::ZERO, glam::Vec2::ONE))
    } else {
        match ctx.state.session.active_transform_tool {
            crate::session::TransformTool::Translate => RegionProps {
                offset: props.offset + delta,
                ..props
            },
            crate::session::TransformTool::Rotate => {
                let angle = |v: glam::Vec2| v.y.atan2(v.x);
                let swept = ankhimate_core::transforms::wrap_angle(
                    angle(local_now - props.offset) - angle(local_prev - props.offset),
                );
                RegionProps {
                    rotation: props.rotation + swept,
                    ..props
                }
            }
            crate::session::TransformTool::Scale => {
                // Distance from the pivot drives a uniform scale; dragging past
                // it would flip the art, so the factor is clamped positive.
                let before = (local_prev - props.offset).length().max(1e-3);
                let after = (local_now - props.offset).length().max(1e-3);
                let factor = (after / before).clamp(0.2, 5.0);
                RegionProps {
                    scale: props.scale * factor,
                    ..props
                }
            }
            // Shear has no attachment equivalent — a region is a quad.
            crate::session::TransformTool::Shear => return false,
        }
    };

    ctx.state
        .dispatch(Box::new(SetRegionProps::new(skin, slot_id, name, next)));
    ctx.state.session.drag_start_world_pos = Some(cursor_world);
    true
}

/// Where the selected slot's pivot sits on screen, if there is one.
fn attachment_pivot_screen(ctx: &ToolContext, viewport_size: glam::Vec2) -> Option<glam::Vec2> {
    use ankhimate_core::attachment::Attachment;
    let slot_id = ctx.state.session.active_slot()?;
    let slot = ctx.state.doc.skeleton.slots.get(slot_id)?;
    let Attachment::Region(region) = ctx
        .state
        .doc
        .skeleton
        .resolve_slot(ctx.state.session.active_skin, slot_id)?
    else {
        return None;
    };
    let bone_world = ctx.state.pose.worlds.get(slot.bone)?;
    Some(ctx.state.session.camera.world_to_screen(
        bone_world.transform_point(region.local_offset),
        viewport_size,
    ))
}

/// The attachment-editing interaction: grab the pivot crosshair, drag with the
/// active transform tool, alt-drag to move the pivot itself.
fn update_attachment_mode(ctx: &mut ToolContext, mouse_screen: Option<glam::Vec2>) {
    use crate::session::GizmoInteraction as G;
    let viewport_size = glam::Vec2::new(ctx.rect.width(), ctx.rect.height());

    if ctx.ui.input(|i| i.pointer.primary_released()) {
        ctx.state.session.dragging_gizmo = G::None;
        ctx.state.session.drag_start_world_pos = None;
    }

    if ctx.state.session.dragging_gizmo != G::None {
        if let Some(mouse_p) = mouse_screen {
            let cursor_world = ctx
                .state
                .session
                .camera
                .screen_to_world(mouse_p, viewport_size);
            drag_attachment(ctx, cursor_world);
        }
        return;
    }

    // Hover: the pivot crosshair, and the art itself.
    ctx.state.session.hovered_gizmo = G::None;
    let pivot = attachment_pivot_screen(ctx, viewport_size);
    if let (Some(mouse_p), Some(pivot)) = (mouse_screen, pivot)
        && (mouse_p - pivot).length() <= 12.0
    {
        ctx.state.session.hovered_gizmo = G::TranslateFree;
        ctx.ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // Bones first, and they win. Artwork covers most of the viewport, so letting
    // it answer first meant that once you selected a piece of art every bone
    // underneath it became unclickable — a one-way door out of bone editing.
    update_hover_state(ctx);
    let under_cursor = if ctx.state.session.hovered_bone.is_some() {
        None
    } else {
        mouse_screen
            .filter(|_| ctx.response.hovered())
            .and_then(|mouse_p| {
                let world = ctx
                    .state
                    .session
                    .camera
                    .screen_to_world(mouse_p, viewport_size);
                pick_attachment(ctx.state, world)
            })
    };
    ctx.state.session.hovered_attachment = under_cursor
        .as_ref()
        .map(|(slot, name, _)| (*slot, name.clone()));
    if ctx.state.session.hovered_gizmo == G::None
        && (under_cursor.is_some() || ctx.state.session.hovered_bone.is_some())
    {
        ctx.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ctx.response.hovered() && ctx.ui.input(|i| i.pointer.primary_pressed()) {
        if ctx.state.session.hovered_gizmo != G::None {
            ctx.state.session.dragging_gizmo = G::TranslateFree;
            if let Some(mouse_p) = mouse_screen {
                ctx.state.session.drag_start_world_pos = Some(
                    ctx.state
                        .session
                        .camera
                        .screen_to_world(mouse_p, viewport_size),
                );
            }
        } else if let Some(bone) = ctx.state.session.hovered_bone {
            ctx.state.session.select_bone(Some(bone));
        } else if let Some((slot, name, bone)) = under_cursor {
            let already = matches!(
                &ctx.state.session.selection,
                Some(crate::session::Selection::Attachment { slot: s, name: n })
                    if *s == slot && *n == name
            );
            if already {
                // Clicking the piece you already have starts moving it. Making
                // the pivot crosshair the only grab point meant hunting for a
                // 12-pixel dot before you could nudge anything.
                ctx.state.session.dragging_gizmo = G::TranslateFree;
                if let Some(mouse_p) = mouse_screen {
                    ctx.state.session.drag_start_world_pos = Some(
                        ctx.state
                            .session
                            .camera
                            .screen_to_world(mouse_p, viewport_size),
                    );
                }
            } else {
                ctx.state.session.select_attachment(slot, name, bone);
            }
        } else {
            // Empty space clears the selection, the same as it does for bones.
            ctx.state.session.select_bone(None);
        }
    }
}

impl CanvasTool for SelectTool {
    fn update(&mut self, ctx: &mut ToolContext) {
        // Zoom-bar / navigation exclusivity is handled by the canvas dispatcher
        // (`canvas::ui`), which skips tools entirely when the camera claims input.
        let mouse_pos = ctx.ui.input(|i| i.pointer.hover_pos());
        let mouse_screen =
            mouse_pos.map(|p| glam::vec2(p.x - ctx.rect.min.x, p.y - ctx.rect.min.y));
        let viewport_size = glam::Vec2::new(ctx.rect.width(), ctx.rect.height());

        // Mesh vertex editing owns the pointer while it is on (T-401), and clip
        // polygon editing likewise (T-405). Both hang off the same toggle; the
        // selected attachment's kind decides which one answers.
        if super::mesh_edit::target(ctx.state).is_some() {
            super::mesh_edit::update(ctx, mouse_screen);
            return;
        }
        if super::clip_edit::target(ctx.state).is_some() {
            super::clip_edit::update(ctx, mouse_screen);
            return;
        }

        // Attachment placement is its own interaction (T-307): same tools, a
        // different target, so it gets its own input path rather than branching
        // inside every bone gesture below.
        if ctx.state.session.editing_attachment() {
            update_attachment_mode(ctx, mouse_screen);
            return;
        }

        // Handle Drag Release — commit the accumulated preview as ONE command so
        // the whole drag is a single undo step and the document was never touched
        // mid-drag (PLAN §3.2, defect D7).
        if ctx.ui.input(|i| i.pointer.primary_released())
            && ctx.state.session.dragging_gizmo != crate::session::GizmoInteraction::None
        {
            ctx.state.session.dragging_gizmo = crate::session::GizmoInteraction::None;
            ctx.state.session.drag_start_world_pos = None;

            let committed: Vec<(ankhimate_core::ids::BoneId, ankhimate_core::math::Transform)> =
                ctx.state
                    .session
                    .preview_locals
                    .iter()
                    .map(|(bone, local)| (bone, *local))
                    .collect();
            ctx.state.session.clear_previews();
            for (bone, local) in committed {
                // Routes to a setup edit or auto-key depending on session state
                // (T-202); locked bones are dropped inside.
                ctx.state.commit_bone_pose(bone, local);
            }
            ctx.state.refresh_pose();
        }

        // Handle Dragging — write into the session preview, not the document.
        if ctx.state.session.dragging_gizmo != crate::session::GizmoInteraction::None {
            if let (Some(mouse_p), Some(selected_id)) =
                (mouse_screen, ctx.state.session.active_bone())
                && ctx.state.doc.skeleton.bones.contains_key(selected_id)
            {
                let cursor_world = ctx
                    .state
                    .session
                    .camera
                    .screen_to_world(mouse_p, viewport_size);
                // Start from the in-flight preview if there is one, otherwise from
                // the authored value.
                let mut local = ctx
                    .state
                    .session
                    .preview_locals
                    .get(selected_id)
                    .copied()
                    .unwrap_or(ctx.state.doc.skeleton.bones[selected_id].local_transform);

                match ctx.state.session.dragging_gizmo {
                    crate::session::GizmoInteraction::Rotate => {
                        if let Some(prev_pos) = ctx.state.session.drag_start_world_pos {
                            let origin_world = ctx.state.pose.world_position(selected_id);
                            let start_delta = prev_pos - origin_world;
                            let current_delta = cursor_world - origin_world;
                            let angle_diff = f32::atan2(current_delta.y, current_delta.x)
                                - f32::atan2(start_delta.y, start_delta.x);
                            local.rotation += angle_diff;
                        }
                    }
                    crate::session::GizmoInteraction::TranslateFree => {
                        if let Some(prev_pos) = ctx.state.session.drag_start_world_pos {
                            local.position += cursor_world - prev_pos;
                        }
                    }
                    crate::session::GizmoInteraction::TranslateX => {
                        if let Some(prev_pos) = ctx.state.session.drag_start_world_pos {
                            let angle = ctx.state.pose.world_decomposed(selected_id).rotation;
                            let dir_x = glam::Vec2::new(angle.cos(), angle.sin());
                            local.position += dir_x * (cursor_world - prev_pos).dot(dir_x);
                        }
                    }
                    crate::session::GizmoInteraction::TranslateY => {
                        if let Some(prev_pos) = ctx.state.session.drag_start_world_pos {
                            let angle = ctx.state.pose.world_decomposed(selected_id).rotation;
                            let dir_y = glam::Vec2::new(-angle.sin(), angle.cos());
                            local.position += dir_y * (cursor_world - prev_pos).dot(dir_y);
                        }
                    }
                    // Shear swings one axis and leaves the other alone. The
                    // delta is measured as the angle the cursor swept around the
                    // bone origin, which makes grabbing either end of a bar work
                    // without a 180° jump.
                    kind @ (crate::session::GizmoInteraction::ShearX
                    | crate::session::GizmoInteraction::ShearY) => {
                        if let Some(prev_pos) = ctx.state.session.drag_start_world_pos {
                            let origin_world = ctx.state.pose.world_position(selected_id);
                            let before = prev_pos - origin_world;
                            let after = cursor_world - origin_world;
                            // Ignore a drag that starts on the origin: the angle
                            // is meaningless there and would snap the axis.
                            if before.length() > 1e-3 && after.length() > 1e-3 {
                                let delta = ankhimate_core::transforms::wrap_angle(
                                    after.y.atan2(after.x) - before.y.atan2(before.x),
                                );
                                if kind == crate::session::GizmoInteraction::ShearX {
                                    local.shear.x += delta;
                                } else {
                                    local.shear.y += delta;
                                }
                            }
                        }
                    }
                    _ => {}
                }

                ctx.state.session.set_preview_local(selected_id, local);
                ctx.state.session.drag_start_world_pos = Some(cursor_world);
                ctx.state.refresh_pose();
            }
            return; // Skip hover/selection updates while dragging
        }

        // Handle Hovering Gizmos
        ctx.state.session.hovered_gizmo = crate::session::GizmoInteraction::None;

        if let (Some(mouse_p), Some(selected_id)) = (mouse_screen, ctx.state.session.active_bone())
            && ctx.state.doc.skeleton.bones.contains_key(selected_id)
        {
            let selected_world = ctx.state.pose.world_decomposed(selected_id);
            if ctx.state.session.active_transform_tool == crate::session::TransformTool::Translate {
                let origin_world = ctx.state.pose.world_position(selected_id);
                let angle = selected_world.rotation;
                let dir_x_world = glam::Vec2::new(angle.cos(), angle.sin());
                let dir_y_world = glam::Vec2::new(-angle.sin(), angle.cos());

                let origin_screen = ctx
                    .state
                    .session
                    .camera
                    .world_to_screen(origin_world, viewport_size);
                let x_p1_screen = ctx
                    .state
                    .session
                    .camera
                    .world_to_screen(origin_world + dir_x_world, viewport_size);
                let screen_dir_x = (x_p1_screen - origin_screen).normalize_or_zero();

                let y_p1_screen = ctx
                    .state
                    .session
                    .camera
                    .world_to_screen(origin_world + dir_y_world, viewport_size);
                let screen_dir_y = (y_p1_screen - origin_screen).normalize_or_zero();

                let v = mouse_p - origin_screen;
                let proj_x = v.dot(screen_dir_x);
                let proj_y = v.dot(screen_dir_y);
                let dist_x = (v - screen_dir_x * proj_x).length();
                let dist_y = (v - screen_dir_y * proj_y).length();

                let hover_dist = 8.0;
                let gizmo_len = 50.0;

                if v.length() <= 10.0 {
                    ctx.state.session.hovered_gizmo =
                        crate::session::GizmoInteraction::TranslateFree;
                } else if proj_x > 10.0 && proj_x <= gizmo_len && dist_x <= hover_dist {
                    ctx.state.session.hovered_gizmo = crate::session::GizmoInteraction::TranslateX;
                } else if proj_y > 10.0 && proj_y <= gizmo_len && dist_y <= hover_dist {
                    ctx.state.session.hovered_gizmo = crate::session::GizmoInteraction::TranslateY;
                }
            } else if ctx.state.session.active_transform_tool
                == crate::session::TransformTool::Rotate
            {
                let origin_world = ctx.state.pose.world_position(selected_id);
                let origin_screen = ctx
                    .state
                    .session
                    .camera
                    .world_to_screen(origin_world, viewport_size);
                let v = mouse_p - origin_screen;
                let dist = v.length();

                if dist <= 10.0 {
                    ctx.state.session.hovered_gizmo =
                        crate::session::GizmoInteraction::TranslateFree;
                } else if (20.0..=40.0).contains(&dist) {
                    ctx.state.session.hovered_gizmo = crate::session::GizmoInteraction::Rotate;
                }
            } else if ctx.state.session.active_transform_tool
                == crate::session::TransformTool::Shear
            {
                // Same frame the renderer draws, so the dot you see is the dot
                // you hit (`ShearFrame`).
                if let Some(frame) = crate::ui::canvas::renderer::ShearFrame::for_bone(
                    ctx.state,
                    selected_id,
                    viewport_size,
                ) {
                    ctx.state.session.hovered_gizmo = frame.hit(mouse_p);
                }
            }
        }

        // Update hover state
        if ctx.state.session.hovered_gizmo == crate::session::GizmoInteraction::None {
            update_hover_state(ctx);
            if ctx.state.session.hovered_bone.is_some() {
                ctx.ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        } else {
            ctx.ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        // The artwork under the cursor, so the renderer can outline it. Skipped
        // while a bone is hovered: two things highlighted at once reads as
        // neither, and the bone is what a click would take.
        ctx.state.session.hovered_attachment = match (
            ctx.state.session.hovered_bone,
            ctx.response.hovered().then_some(mouse_screen).flatten(),
        ) {
            (None, Some(mouse_p)) => {
                let world = ctx
                    .state
                    .session
                    .camera
                    .screen_to_world(mouse_p, viewport_size);
                pick_attachment(ctx.state, world).map(|(slot, name, _)| (slot, name))
            }
            _ => None,
        };

        // Handle Click / Start Drag
        if ctx.response.hovered() && ctx.ui.input(|i| i.pointer.primary_pressed()) {
            if ctx.state.session.hovered_gizmo != crate::session::GizmoInteraction::None {
                // Begin a drag. No snapshot and no document write — the drag lives
                // in `Session::preview_locals` until release.
                ctx.state.session.dragging_gizmo = ctx.state.session.hovered_gizmo;
                if let Some(mouse_p) = mouse_screen {
                    ctx.state.session.drag_start_world_pos = Some(
                        ctx.state
                            .session
                            .camera
                            .screen_to_world(mouse_p, viewport_size),
                    );
                }
            } else {
                let ctrl = ctx.ui.input(|i| i.modifiers.ctrl);
                match (ctrl, ctx.state.session.hovered_bone) {
                    // Ctrl-click toggles a bone in the multi-selection.
                    (true, Some(bone)) => ctx.state.session.toggle_bone(bone),
                    (_, Some(bone)) => ctx.state.session.select_bone(Some(bone)),
                    // No bone under the cursor: try the artwork before giving up
                    // and clearing. Clicking a piece is how you ask "what is
                    // this and where does it live", and the tree scrolls to the
                    // answer (T-708).
                    (_, None) => {
                        let picked = mouse_screen.and_then(|m| {
                            let world = ctx.state.session.camera.screen_to_world(m, viewport_size);
                            pick_attachment(ctx.state, world)
                        });
                        match picked {
                            Some((slot, name, bone)) => {
                                ctx.state.session.select_attachment(slot, name, bone)
                            }
                            None => ctx.state.session.select_bone(None),
                        }
                    }
                }
            }
        }
    }
}

/// The front-most attachment under a world-space point, if any (T-708).
///
/// Walks the draw order backwards, so what you click is what you see on top.
/// Regions test against their quad, meshes against their triangles — a mesh's
/// bounding box would happily claim the gap inside a curved piece.
fn pick_attachment(
    state: &crate::app_state::AppState,
    world: glam::Vec2,
) -> Option<(
    ankhimate_core::ids::SlotId,
    String,
    ankhimate_core::ids::BoneId,
)> {
    use ankhimate_core::attachment::Attachment;

    // Same rule as bones: hidden art is not clickable.
    if !state.session.show_artwork {
        return None;
    }

    let inside = |a: glam::Vec2, b: glam::Vec2, c: glam::Vec2| {
        let sign = |p: glam::Vec2, q: glam::Vec2, r: glam::Vec2| (q - p).perp_dot(r - p);
        let (d1, d2, d3) = (sign(a, b, world), sign(b, c, world), sign(c, a, world));
        let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(negative && positive)
    };

    for &slot_id in state.pose.draw_order.iter().rev() {
        let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
            continue;
        };
        // Hidden slots are not clickable — they are not on screen to click.
        if state.pose.slot_visible.get(slot_id) == Some(&false) {
            continue;
        }
        // The name the pose is showing, so clicking a swapped-in attachment
        // selects that one rather than whatever setup happens to name.
        let Some(name) = state
            .pose
            .attachment_name(&state.doc.skeleton, slot_id)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(attachment) =
            state
                .doc
                .skeleton
                .resolve_posed(&state.session.skin_stack(), &state.pose, slot_id)
        else {
            continue;
        };
        let bone_world = state.pose.world(slot.bone);
        let hit = match attachment {
            Attachment::Region(r) => {
                let c = r.local_corners().map(|v| bone_world.transform_point(v));
                inside(c[0], c[1], c[2]) || inside(c[0], c[2], c[3])
            }
            Attachment::Mesh(m) => {
                let skinned = !m.weights.is_empty() && !m.inverse_bind_matrices.is_empty();
                let at = |i: usize| {
                    if skinned {
                        m.skin_vertex_with_ffd(i, glam::Vec2::ZERO, &state.pose)
                    } else {
                        bone_world.transform_point(m.setup_vertices[i])
                    }
                };
                m.triangles.iter().any(|t| {
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    a < m.setup_vertices.len()
                        && b < m.setup_vertices.len()
                        && c < m.setup_vertices.len()
                        && inside(at(a), at(b), at(c))
                })
            }
            // A hitbox is pickable through its own polygon — it is the one piece
            // of non-artwork geometry an animator positions by eye, so it has to
            // be grabbable where it is drawn.
            Attachment::BoundingBox(b) => {
                let local = bone_world
                    .invert()
                    .map(|inv| inv.transform_point(world))
                    .unwrap_or(world);
                b.contains(local)
            }
            // A point has no area; the gizmo layer picks it by proximity.
            // Clips and paths are authoring geometry with their own handles.
            Attachment::Clipping(_) | Attachment::Path(_) | Attachment::Point(_) => false,
        };
        if hit {
            return Some((slot_id, name, slot.bone));
        }
    }
    None
}
