use crate::app_state::AppState;
use crate::commands::key_cmds::{BoneProperty, TimelineAddr};
use crate::session::TransformTool;
use eframe::egui;

// Axis accent colors — same palette as Blender/Unity
const X_COLOR: egui::Color32 = egui::Color32::from_rgb(210, 70, 60);
const Y_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 185, 80);
const SINGLE_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 140, 220);

const FIELD_H: f32 = 22.0;
const LABEL_W: f32 = 72.0;
const ACCENT_W: f32 = 3.0;
const ROW_GAP: f32 = 3.0;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    ui.add_space(2.0);
    breadcrumb(ui, state);

    // A focused constraint gets the panel to itself: it belongs to the rig, not
    // to whichever bone happens to be selected, and showing it under a bone's
    // transform made it look like a property of that bone.
    if let Some(crate::session::Selection::Constraint(id)) = state.session.selection.clone() {
        focused_constraint(ui, state, id);
        return;
    }

    // Transform first, always. It is the control the user reaches for on every
    // second action; the slot's colour/attachment block used to sit above it and
    // pushed it off the top of the panel whenever a slot was selected.
    let Some(bone_id) = state.session.active_bone() else {
        // No bone — show whatever slot is selected, then the hint.
        if let Some(slot_id) = state.session.active_slot() {
            slot_inspector(ui, state, slot_id);
            attachment_inspector(ui, state, slot_id);
            ui.add_space(10.0);
        }
        ui.add_space(32.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(crate::ui::icons::NOTHING_SELECTED)
                    .size(32.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(6.0);
            ui.label(egui::RichText::new("No bone selected").color(ui.visuals().weak_text_color()));
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Click a bone on the canvas or in the Hierarchy")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    };

    let Some(bone) = state.doc.skeleton.bones.get(bone_id) else {
        return;
    };

    // ── Bone info ─────────────────────────────────────────────────────────
    let bone_name = bone.name.clone();
    let bone_len = bone.length;
    let parent_name = bone
        .parent
        .and_then(|p| state.doc.skeleton.bones.get(p))
        .map(|b| b.name.clone());

    section_header(ui, crate::ui::icons::BONE, &bone_name);
    ui.add_space(2.0);
    info_row(ui, "Length", &format!("{:.2}", bone_len));
    if let Some(p) = &parent_name {
        info_row(ui, "Parent", p);
    }

    // Colour: how limbs are told apart at a glance, and it inherits down the
    // hierarchy, so setting it on a shoulder colours the whole arm (T-505).
    {
        let current = state
            .doc
            .skeleton
            .bones
            .get(bone_id)
            .map(|b| b.color)
            .unwrap_or(ankhimate_core::skeleton::Bone::default_color());
        let inherited = crate::ui::canvas::renderer::group_color(&state.doc.skeleton, bone_id);
        let own = current != ankhimate_core::skeleton::Bone::default_color();
        let mut rgba = egui::Color32::from_rgba_unmultiplied(
            (inherited[0] * 255.0) as u8,
            (inherited[1] * 255.0) as u8,
            (inherited[2] * 255.0) as u8,
            255,
        );
        ui.horizontal(|ui| {
            ui.add_sized([LABEL_W, FIELD_H], egui::Label::new("Colour"));
            let editable = state.session.can_edit_structure();
            let changed = ui
                .add_enabled_ui(editable, |ui| ui.color_edit_button_srgba(&mut rgba))
                .inner
                .changed();
            if changed {
                state.dispatch(Box::new(crate::commands::bone_cmds::SetBoneColor::new(
                    bone_id,
                    [
                        rgba.r() as f32 / 255.0,
                        rgba.g() as f32 / 255.0,
                        rgba.b() as f32 / 255.0,
                        current[3],
                    ],
                )));
            }
            if !own {
                ui.label(
                    egui::RichText::new("inherited")
                        .size(10.0)
                        .color(ui.visuals().weak_text_color()),
                );
            } else if ui
                .add_enabled(editable, egui::Button::new("Reset").small())
                .on_hover_text("Fall back to the nearest coloured ancestor")
                .clicked()
            {
                state.dispatch(Box::new(crate::commands::bone_cmds::SetBoneColor::new(
                    bone_id,
                    ankhimate_core::skeleton::Bone::default_color(),
                )));
            }
        });
    }
    ui.add_space(10.0);

    // ── Local Transform ───────────────────────────────────────────────────
    section_header(ui, crate::ui::icons::TRANSLATE, "Local Transform");
    ui.add_space(4.0);

    // Setup mode edits the setup transform; Animate mode edits the *posed*
    // value, which is what the viewport shows and what a key will capture
    // (T-207). Reading the pose also picks up an in-flight preview, so the
    // numbers track a drag.
    let (mut rot, mut tx, mut ty, mut sx, mut sy, mut shx, mut shy) = {
        let setup = state
            .doc
            .skeleton
            .bones
            .get(bone_id)
            .unwrap()
            .local_transform;
        let t = if state.session.is_animating() {
            state.pose.locals.get(bone_id).copied().unwrap_or(setup)
        } else {
            setup
        };
        (
            t.rotation.to_degrees(),
            t.position.x,
            t.position.y,
            t.scale.x,
            t.scale.y,
            // Shear is radians in core like `rotation` (ADR 0002); the panel
            // shows degrees, as every rigging tool does.
            t.shear.x.to_degrees(),
            t.shear.y.to_degrees(),
        )
    };

    let mut changed = false;
    // Key affordances only mean something against a clip (T-210).
    let animating = state.session.is_animating();
    let pending = state.session.pending_pose.contains(&bone_id);
    let mut dot_action: Option<(BoneProperty, DotAction)> = None;

    let key_state = |state: &AppState, property: BoneProperty| {
        if pending {
            // An uncommitted pose applies to every channel the user moved; the
            // dot flags the bone, and keying commits what actually changed.
            return crate::edit_router::KeyState::Modified;
        }
        crate::edit_router::key_state(
            &state.doc,
            &state.session,
            &TimelineAddr::Bone {
                bone: bone_id,
                property,
            },
        )
    };

    // Rotate — single field
    let rot_sel = state.session.active_transform_tool == TransformTool::Rotate;
    let keyed = key_state(state, BoneProperty::Rotate);
    let (row_changed, action) = keyed_row(ui, animating, keyed, |ui| {
        transform_row_single(ui, "Rotate", &mut rot, 0.5, 2, SINGLE_COLOR, rot_sel)
    });
    if row_changed {
        changed = true;
        state.session.active_transform_tool = TransformTool::Rotate;
    }
    if action != DotAction::None {
        dot_action = Some((BoneProperty::Rotate, action));
    }
    ui.add_space(ROW_GAP);

    // Translate
    let tr_sel = state.session.active_transform_tool == TransformTool::Translate;
    let keyed = key_state(state, BoneProperty::Translate);
    let (row_changed, action) = keyed_row(ui, animating, keyed, |ui| {
        transform_row_xy(ui, "Translate", &mut tx, &mut ty, 0.5, 2, tr_sel)
    });
    if row_changed {
        changed = true;
        state.session.active_transform_tool = TransformTool::Translate;
    }
    if action != DotAction::None {
        dot_action = Some((BoneProperty::Translate, action));
    }
    ui.add_space(ROW_GAP);

    // Scale
    let sc_sel = state.session.active_transform_tool == TransformTool::Scale;
    let keyed = key_state(state, BoneProperty::Scale);
    let (row_changed, action) = keyed_row(ui, animating, keyed, |ui| {
        transform_row_xy(ui, "Scale", &mut sx, &mut sy, 0.01, 3, sc_sel)
    });
    if row_changed {
        changed = true;
        state.session.active_transform_tool = TransformTool::Scale;
    }
    if action != DotAction::None {
        dot_action = Some((BoneProperty::Scale, action));
    }
    ui.add_space(ROW_GAP);

    // Shear — degrees, so it drags at the Rotate rate rather than the 0.01 step
    // that suited radians.
    let sh_sel = state.session.active_transform_tool == TransformTool::Shear;
    let keyed = key_state(state, BoneProperty::Shear);
    let (row_changed, action) = keyed_row(ui, animating, keyed, |ui| {
        transform_row_xy(ui, "Shear", &mut shx, &mut shy, 0.5, 2, sh_sel)
    });
    if row_changed {
        changed = true;
        state.session.active_transform_tool = TransformTool::Shear;
    }
    if action != DotAction::None {
        dot_action = Some((BoneProperty::Shear, action));
    }

    if let Some((property, action)) = dot_action {
        apply_dot_action(state, bone_id, property, action);
    }

    if changed {
        // Routed through the mode (T-207): a setup edit in Setup, keys at the
        // playhead in Animate. Undoable, and merged — dragging a spinbox
        // produces one history entry rather than one per frame.
        let local = ankhimate_core::math::Transform {
            position: glam::vec2(tx, ty),
            rotation: rot.to_radians(),
            scale: glam::vec2(sx, sy),
            shear: glam::vec2(shx.to_radians(), shy.to_radians()),
        };
        state.commit_bone_pose(bone_id, local);
    }

    // ── World Transform (read-only) ───────────────────────────────────────
    ui.add_space(10.0);
    section_header(ui, crate::ui::icons::WORLD, "World Transform");
    ui.add_space(2.0);

    let wt = match state.doc.skeleton.bones.contains_key(bone_id) {
        true => state.pose.world_decomposed(bone_id),
        false => return,
    };
    readonly_row(ui, "Pos X", &format!("{:.2}", wt.position.x));
    readonly_row(ui, "Pos Y", &format!("{:.2}", wt.position.y));
    readonly_row(ui, "Rotation", &format!("{:.2}°", wt.rotation.to_degrees()));
    readonly_row(ui, "Scale X", &format!("{:.3}", wt.scale.x));
    readonly_row(ui, "Scale Y", &format!("{:.3}", wt.scale.y));

    // ── Constraints (T-501) ──────────────────────────────────────────────
    constraint_inspector(ui, state, bone_id);
    ik_inspector(ui, state, bone_id);
    physics_inspector(ui, state, bone_id);
    path_constraint_inspector(ui, state, bone_id);

    // ── Slot section (T-205) ─────────────────────────────────────────────
    // Below the transform: it describes what the bone *shows*, which is the
    // less-used half of this panel.
    if let Some(slot_id) = state.session.active_slot() {
        ui.add_space(10.0);
        slot_inspector(ui, state, slot_id);
        attachment_inspector(ui, state, slot_id);
    }
}

/// The path to the focused item, as clickable steps (T-708).
///
/// The inspector shows one thing at a time and used to leave you to infer which
/// — a slot's panel and its attachment's panel look alike, and the difference
/// matters when you are chasing a misplaced piece. The trail also walks back up:
/// clicking a step selects it.
fn breadcrumb(ui: &mut egui::Ui, state: &mut AppState) {
    use crate::session::Selection;

    let Some(selection) = state.session.selection.clone() else {
        return;
    };

    // (label, what clicking it focuses)
    let mut steps: Vec<(String, Selection)> = Vec::new();
    let bone_trail = |state: &AppState, bone: ankhimate_core::ids::BoneId| {
        let mut trail = Vec::new();
        let mut cursor = Some(bone);
        // Bounded by the hierarchy depth; a cycle is impossible by construction.
        for _ in 0..64 {
            let Some(id) = cursor else { break };
            let Some(b) = state.doc.skeleton.bones.get(id) else {
                break;
            };
            trail.push((b.name.clone(), Selection::Bone(id)));
            cursor = b.parent;
        }
        trail.reverse();
        trail
    };

    match &selection {
        Selection::Bone(bone) => steps.extend(bone_trail(state, *bone)),
        Selection::Slot(slot) => {
            if let Some(s) = state.doc.skeleton.slots.get(*slot) {
                steps.extend(bone_trail(state, s.bone));
                steps.push((s.name.clone(), Selection::Slot(*slot)));
            }
        }
        Selection::Attachment { slot, name } => {
            if let Some(s) = state.doc.skeleton.slots.get(*slot) {
                steps.extend(bone_trail(state, s.bone));
                steps.push((s.name.clone(), Selection::Slot(*slot)));
            }
            steps.push((
                name.clone(),
                Selection::Attachment {
                    slot: *slot,
                    name: name.clone(),
                },
            ));
        }
        Selection::Constraint(id) => {
            if let Some(c) = state.doc.skeleton.constraints.get(*id) {
                if let Some(&bone) = c.affected_bones().first() {
                    steps.extend(bone_trail(state, bone));
                }
                steps.push((c.name().to_string(), Selection::Constraint(*id)));
            }
        }
    }

    let mut jump: Option<Selection> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, (label, target)) in steps.iter().enumerate() {
            if i > 0 {
                ui.label(
                    egui::RichText::new("›")
                        .size(11.0)
                        .color(ui.visuals().weak_text_color()),
                );
            }
            let last = i + 1 == steps.len();
            let text = egui::RichText::new(label).size(11.0);
            let text = if last {
                text.strong()
            } else {
                text.color(ui.visuals().weak_text_color())
            };
            if ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .clicked()
            {
                jump = Some(target.clone());
            }
        }
    });
    if let Some(target) = jump {
        match &target {
            Selection::Bone(b) => state.session.select_bone(Some(*b)),
            Selection::Slot(s) => state.session.select_slot(Some(*s)),
            Selection::Attachment { slot, name } => {
                let bone = state.doc.skeleton.slots.get(*slot).map(|s| s.bone);
                if let Some(bone) = bone {
                    state.session.select_attachment(*slot, name.clone(), bone);
                }
            }
            Selection::Constraint(c) => state.session.select_constraint(*c),
        }
    }
    ui.separator();
}

/// The panel for one focused constraint (T-708).
fn focused_constraint(
    ui: &mut egui::Ui,
    state: &mut AppState,
    id: ankhimate_core::ids::ConstraintId,
) {
    use ankhimate_core::constraints::Constraint;

    let Some(constraint) = state.doc.skeleton.constraints.get(id) else {
        ui.label(
            egui::RichText::new("This constraint no longer exists")
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };
    let (icon, kind) = match constraint {
        Constraint::Ik(_) => (crate::ui::icons::IK, "IK constraint"),
        Constraint::Transform(_) => (
            crate::ui::icons::TRANSFORM_CONSTRAINT,
            "Transform constraint",
        ),
        Constraint::Physics(_) => (crate::ui::icons::PHYSICS, "Physics constraint"),
        Constraint::Path(_) => (crate::ui::icons::PATH, "Path constraint"),
    };
    let name = constraint.name().to_string();
    // The bones it drives, so "what does this actually move" is answerable
    // without cross-referencing the tree.
    let driven: Vec<String> = constraint
        .affected_bones()
        .iter()
        .filter_map(|b| state.doc.skeleton.bones.get(*b))
        .map(|b| b.name.clone())
        .collect();
    let first = constraint.affected_bones().first().copied();

    section_header(ui, icon, &name);
    ui.add_space(2.0);
    info_row(ui, "Kind", kind);
    info_row(ui, "Drives", &driven.join(", "));
    ui.add_space(6.0);

    // The per-kind editors are written against a driven bone, which is also how
    // they are reached from the bone panel; passing the first one keeps a single
    // implementation rather than a second copy that can drift.
    let Some(bone) = first else { return };
    match state.doc.skeleton.constraints.get(id) {
        Some(Constraint::Ik(_)) => ik_inspector(ui, state, bone),
        Some(Constraint::Transform(_)) => constraint_inspector(ui, state, bone),
        Some(Constraint::Physics(_)) => physics_inspector(ui, state, bone),
        Some(Constraint::Path(_)) => path_constraint_inspector(ui, state, bone),
        None => {}
    }
}

/// Constraints driving the selected bone (T-501).
///
/// Listed on the **driven** bone, not the target: "why is this bone moving on
/// its own" is the question a rigger actually asks, and the answer is whichever
/// constraints write to it. A target bone can drive a dozen others and does not
/// want a dozen sections.
fn constraint_inspector(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bone_id: ankhimate_core::ids::BoneId,
) {
    use crate::commands::constraint_cmds::{
        AddTransformConstraint, RemoveConstraint, SetTransformProps, TransformProps,
    };
    use ankhimate_core::constraints::Constraint;

    let setup = state.session.can_edit_structure();

    // Constraints in application order, filtered to the ones touching this bone.
    let driving: Vec<(ankhimate_core::ids::ConstraintId, String, TransformProps)> = state
        .doc
        .skeleton
        .constraint_order
        .iter()
        .filter_map(|id| {
            let Some(Constraint::Transform(tc)) = state.doc.skeleton.constraints.get(*id) else {
                return None;
            };
            tc.bones
                .contains(&bone_id)
                .then(|| (*id, tc.name.clone(), TransformProps::from_constraint(tc)))
        })
        .collect();

    ui.add_space(10.0);
    section_header(ui, crate::ui::icons::CONSTRAINT, "Constraints");
    ui.add_space(2.0);

    if driving.is_empty() {
        ui.label(
            egui::RichText::new("Nothing drives this bone")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
    }

    // Bone names for the target picker, resolved once.
    let bones: Vec<(ankhimate_core::ids::BoneId, String)> = state
        .doc
        .skeleton
        .bones
        .iter()
        .filter(|(id, _)| *id != bone_id)
        .map(|(id, b)| (id, b.name.clone()))
        .collect();

    let mut edit: Option<(ankhimate_core::ids::ConstraintId, TransformProps)> = None;
    let mut remove: Option<ankhimate_core::ids::ConstraintId> = None;

    for (id, name, props) in &driving {
        let mut next = props.clone();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).size(11.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(setup, egui::Button::new("✕").small())
                    .on_hover_text("Delete this constraint")
                    .clicked()
                {
                    remove = Some(*id);
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Target").size(10.5));
            let current = bones
                .iter()
                .find(|(b, _)| *b == next.target)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "<missing>".to_string());
            egui::ComboBox::from_id_salt(("constraint_target", id))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (candidate, label) in &bones {
                        if ui
                            .selectable_label(*candidate == next.target, label)
                            .clicked()
                        {
                            next.target = *candidate;
                        }
                    }
                });
        });

        // The four channel mixes. Separate because "follow its rotation but not
        // its position" is the common case, not the exception.
        for (label, value, hint) in [
            (
                "Rotate",
                &mut next.mix_rotate,
                "How much of the target's rotation this bone takes",
            ),
            (
                "Translate",
                &mut next.mix_translate,
                "How much of the target's position this bone takes",
            ),
            ("Scale", &mut next.mix_scale, "…its scale"),
            ("Shear", &mut next.mix_shear, "…its shear"),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [64.0, 18.0],
                    egui::Label::new(egui::RichText::new(label).size(10.5)),
                );
                ui.add_enabled(setup, egui::Slider::new(value, 0.0..=1.0).max_decimals(2))
                    .on_hover_text(hint);
            });
        }

        ui.horizontal(|ui| {
            let mut offset_degrees = next.offsets.rotation.to_degrees();
            ui.label(egui::RichText::new("Offset°").size(10.5));
            if ui
                .add_enabled(
                    setup,
                    egui::DragValue::new(&mut offset_degrees)
                        .speed(0.5)
                        .range(-360.0..=360.0),
                )
                .on_hover_text("Added to the target's rotation — 'track it, but stay 10° off'")
                .changed()
            {
                next.offsets.rotation = offset_degrees.to_radians();
            }
            ui.add_enabled(setup, egui::Checkbox::new(&mut next.local, "Local"))
                .on_hover_text(
                    "Copy the target's transform relative to its own parent, rather \
                     than its world transform",
                );
            ui.add_enabled(setup, egui::Checkbox::new(&mut next.relative, "Add"))
                .on_hover_text(
                    "Add the target's transform to this bone's own instead of \
                     replacing it — layers on top of an animation",
                );
        });

        if next != *props {
            edit = Some((*id, next));
        }
    }

    ui.add_space(6.0);
    // Creating one needs a second bone to point at; the picker above changes it
    // afterwards, so any other bone is a fine starting target.
    let default_target = state
        .doc
        .skeleton
        .bones
        .get(bone_id)
        .and_then(|b| b.parent)
        .or_else(|| bones.first().map(|(id, _)| *id));
    if ui
        .add_enabled(
            setup && default_target.is_some(),
            egui::Button::new("Add transform constraint").small(),
        )
        .on_hover_text(
            "Drive this bone from another one — a head that tracks a look-at \
             target, a wheel that mirrors a shaft",
        )
        .clicked()
        && let Some(target) = default_target
    {
        let name = format!("constraint {}", state.doc.skeleton.constraints.len() + 1);
        state.dispatch(Box::new(AddTransformConstraint::new(
            name,
            target,
            vec![bone_id],
        )));
    }

    if let Some((id, props)) = edit {
        state.dispatch(Box::new(SetTransformProps::new(id, props)));
    }
    if let Some(id) = remove {
        state.dispatch(Box::new(RemoveConstraint::new(id)));
    }
}

/// IK constraints reaching for, or driving, the selected bone (T-504).
///
/// Sits under the transform-constraint list and reads the same way: the
/// constraints that explain why this bone moves on its own.
fn ik_inspector(ui: &mut egui::Ui, state: &mut AppState, bone_id: ankhimate_core::ids::BoneId) {
    use crate::commands::constraint_cmds::{CreateIkTarget, IkProps, RemoveConstraint, SetIkProps};
    use ankhimate_core::constraints::Constraint;

    let setup = state.session.can_edit_structure();

    let driving: Vec<(ankhimate_core::ids::ConstraintId, String, IkProps)> = state
        .doc
        .skeleton
        .constraint_order
        .iter()
        .filter_map(|id| {
            let Some(Constraint::Ik(ik)) = state.doc.skeleton.constraints.get(*id) else {
                return None;
            };
            ik.bones
                .contains(&bone_id)
                .then(|| (*id, ik.name.clone(), IkProps::from_constraint(ik)))
        })
        .collect();

    let bones: Vec<(ankhimate_core::ids::BoneId, String)> = state
        .doc
        .skeleton
        .bones
        .iter()
        .filter(|(id, _)| *id != bone_id)
        .map(|(id, b)| (id, b.name.clone()))
        .collect();

    let mut edit: Option<(ankhimate_core::ids::ConstraintId, IkProps)> = None;
    let mut remove: Option<ankhimate_core::ids::ConstraintId> = None;

    for (id, name, props) in &driving {
        let mut next = props.clone();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} (IK, {} bones)", name, props.bones.len()))
                    .size(11.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(setup, egui::Button::new("✕").small())
                    .on_hover_text("Delete this constraint (the target bone stays)")
                    .clicked()
                {
                    remove = Some(*id);
                }
            });
        });

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Target").size(10.5));
            let current = bones
                .iter()
                .find(|(b, _)| *b == next.target)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "<missing>".to_string());
            egui::ComboBox::from_id_salt(("ik_target", id))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (candidate, label) in &bones {
                        if ui
                            .selectable_label(*candidate == next.target, label)
                            .clicked()
                        {
                            next.target = *candidate;
                        }
                    }
                });
        });

        ui.horizontal(|ui| {
            ui.add_sized(
                [64.0, 18.0],
                egui::Label::new(egui::RichText::new("Mix").size(10.5)),
            );
            ui.add_enabled(
                setup,
                egui::Slider::new(&mut next.mix, 0.0..=1.0).max_decimals(2),
            )
            .on_hover_text("0 is pure FK, 1 is pure IK");
        });
        ui.horizontal(|ui| {
            ui.add_sized(
                [64.0, 18.0],
                egui::Label::new(egui::RichText::new("Softness").size(10.5)),
            );
            ui.add_enabled(
                setup,
                egui::DragValue::new(&mut next.softness)
                    .speed(0.2)
                    .range(0.0..=1000.0),
            )
            .on_hover_text(
                "Ease the last stretch of reach, in world units, so the chain does \
                 not snap straight as the target leaves its range",
            );
        });
        ui.horizontal(|ui| {
            ui.add_enabled(setup, egui::Checkbox::new(&mut next.stretch, "Stretch"))
                .on_hover_text("Let the chain lengthen to reach a target beyond its natural reach");
            if next.stretch {
                ui.add_enabled(
                    setup,
                    egui::DragValue::new(&mut next.stretch_limit)
                        .speed(0.01)
                        .range(1.0..=3.0),
                )
                .on_hover_text("Most it may grow, as a factor of its natural length");
            }
            let mut flipped = next.bend_direction < 0.0;
            if ui
                .add_enabled(setup, egui::Checkbox::new(&mut flipped, "Flip bend"))
                .on_hover_text("Which way the chain's elbow points")
                .changed()
            {
                next.bend_direction = if flipped { -1.0 } else { 1.0 };
            }
        });

        if next != *props {
            edit = Some((*id, next));
        }
    }

    // ── Create from a selection ──────────────────────────────────────────
    // The chain is the selected bones in hierarchy order, so "select shoulder,
    // elbow, wrist → create IK" is the whole flow. One selected bone makes an
    // aim constraint, which is the other thing people want this for.
    let chain = selected_chain(state);
    ui.add_space(6.0);
    let can_create = setup && !chain.is_empty();
    let label = match chain.len() {
        0 | 1 => "Create IK target".to_string(),
        n => format!("Create IK target ({n}-bone chain)"),
    };
    if ui
        .add_enabled(can_create, egui::Button::new(label).small())
        .on_hover_text(
            "Make a target bone at the chain's tip and an IK constraint that \
             reaches for it.\nSelect several bones (shift-click in the Hierarchy) \
             to build a longer chain.",
        )
        .clicked()
        && let Some(&tip) = chain.last()
    {
        // The target starts exactly at the tip so switching the constraint on
        // does not move the rig.
        let position = state.pose.world_tip(&state.doc.skeleton, tip);
        let name = format!("ik {}", state.doc.skeleton.constraints.len() + 1);
        state.dispatch(Box::new(CreateIkTarget::new(chain, name, position)));
    }

    if let Some((id, props)) = edit {
        state.dispatch(Box::new(SetIkProps::new(id, props)));
    }
    if let Some(id) = remove {
        state.dispatch(Box::new(RemoveConstraint::new(id)));
    }
}

/// The selected bones as an IK chain: root first, and only while they form an
/// unbroken parent→child line.
///
/// A "chain" of unrelated bones is not solvable — FABRIK walks parent to child —
/// so a disjoint selection yields nothing rather than a constraint that would
/// evaluate to nonsense.
fn selected_chain(state: &AppState) -> Vec<ankhimate_core::ids::BoneId> {
    let selection = state.session.selected_bones.clone();
    if selection.is_empty() {
        return Vec::new();
    }
    if selection.len() == 1 {
        return selection;
    }
    // Order by depth, then check each is the previous one's child.
    let depth = |mut b: ankhimate_core::ids::BoneId| {
        let mut n = 0;
        while let Some(parent) = state.doc.skeleton.bones.get(b).and_then(|x| x.parent) {
            n += 1;
            b = parent;
        }
        n
    };
    let mut ordered = selection;
    ordered.sort_by_key(|b| depth(*b));
    for pair in ordered.windows(2) {
        let child_parent = state.doc.skeleton.bones.get(pair[1]).and_then(|b| b.parent);
        if child_parent != Some(pair[0]) {
            return Vec::new();
        }
    }
    ordered
}

/// Physics constraints on the selected bone (T-503).
fn physics_inspector(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bone_id: ankhimate_core::ids::BoneId,
) {
    use crate::commands::constraint_cmds::{
        AddPhysics, PhysicsProps, RemoveConstraint, SetPhysicsProps,
    };
    use ankhimate_core::constraints::Constraint;

    let setup = state.session.can_edit_structure();
    let existing: Vec<(ankhimate_core::ids::ConstraintId, String, PhysicsProps)> = state
        .doc
        .skeleton
        .constraint_order
        .iter()
        .filter_map(|id| {
            let Some(Constraint::Physics(p)) = state.doc.skeleton.constraints.get(*id) else {
                return None;
            };
            (p.bone == bone_id).then(|| (*id, p.name.clone(), PhysicsProps::from_constraint(p)))
        })
        .collect();

    let mut edit: Option<(ankhimate_core::ids::ConstraintId, PhysicsProps)> = None;
    let mut remove: Option<ankhimate_core::ids::ConstraintId> = None;

    for (id, name, props) in &existing {
        let mut next = *props;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{name} (physics)"))
                    .size(11.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(setup, egui::Button::new("✕").small())
                    .on_hover_text("Delete this constraint")
                    .clicked()
                {
                    remove = Some(*id);
                }
            });
        });

        for (label, value, range, hint) in [
            (
                "Inertia",
                &mut next.inertia,
                0.0..=1.0,
                "How much the bone resists following its parent — this is what reads as weight",
            ),
            (
                "Strength",
                &mut next.strength,
                0.0..=200.0,
                "How hard it is pulled back to rest. Higher is stiffer and faster.",
            ),
            (
                "Damping",
                &mut next.damping,
                0.0..=1.0,
                "How quickly motion bleeds off. At 1 it barely overshoots; at 0 it never settles.",
            ),
            (
                "Mass",
                &mut next.mass,
                0.05..=10.0,
                "Heavier bones move less for the same push",
            ),
            (
                "Mix",
                &mut next.mix,
                0.0..=1.0,
                "0 is off, 1 is fully simulated",
            ),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [64.0, 18.0],
                    egui::Label::new(egui::RichText::new(label).size(10.5)),
                );
                ui.add_enabled(setup, egui::Slider::new(value, range).max_decimals(2))
                    .on_hover_text(hint);
            });
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Wind").size(10.5));
            ui.add_enabled(setup, egui::DragValue::new(&mut next.wind.x).speed(0.5))
                .on_hover_text("Constant sideways push, in world units");
            ui.add_enabled(setup, egui::DragValue::new(&mut next.wind.y).speed(0.5));
            ui.label(egui::RichText::new("Gravity").size(10.5));
            ui.add_enabled(setup, egui::DragValue::new(&mut next.gravity.y).speed(0.5))
                .on_hover_text("Negative pulls down — world Y is up");
        });
        ui.horizontal(|ui| {
            ui.add_enabled(setup, egui::Checkbox::new(&mut next.rotate, "Rotate"))
                .on_hover_text("Let the simulation swing the bone");
            ui.add_enabled(setup, egui::Checkbox::new(&mut next.translate, "Translate"))
                .on_hover_text("Let it slide as well — for a bone with no length to swing on");
        });

        if next != *props {
            edit = Some((*id, next));
        }
    }

    // ── Simulation controls ──────────────────────────────────────────────
    // The simulation is session state (ADR 0007), so these are not undoable and
    // deliberately live next to the values they help tune.
    if !existing.is_empty() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .button("Reset")
                .on_hover_text("Return every simulated bone to rest")
                .clicked()
            {
                state.physics.reset();
                state.refresh_pose();
            }
            let mut paused = state.physics.paused;
            if ui
                .checkbox(&mut paused, "Pause")
                .on_hover_text("Freeze the simulation without losing where it is")
                .changed()
            {
                state.physics.paused = paused;
            }
            let mut simulate = state.session.simulate_in_setup;
            if ui
                .checkbox(&mut simulate, "Run in Setup")
                .on_hover_text(
                    "Simulate while in Setup mode, so these values can be tuned \
                     without an animation to play",
                )
                .changed()
            {
                state.session.simulate_in_setup = simulate;
                if !simulate {
                    state.physics.reset();
                    state.refresh_pose();
                }
            }
        });
    }

    ui.add_space(4.0);
    if ui
        .add_enabled(setup, egui::Button::new("Add physics").small())
        .on_hover_text("Make this bone sway — hair, a tail, a chain, cloth")
        .clicked()
    {
        let name = format!("physics {}", state.doc.skeleton.constraints.len() + 1);
        state.dispatch(Box::new(AddPhysics::new(bone_id, name)));
    }

    if let Some((id, props)) = edit {
        state.dispatch(Box::new(SetPhysicsProps::new(id, props)));
    }
    if let Some(id) = remove {
        state.dispatch(Box::new(RemoveConstraint::new(id)));
    }
}

/// Path constraints driving the selected bone (T-502).
fn path_constraint_inspector(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bone_id: ankhimate_core::ids::BoneId,
) {
    use crate::commands::constraint_cmds::{
        AddPathConstraint, PathProps, RemoveConstraint, SetPathProps,
    };
    use ankhimate_core::attachment::Attachment;
    use ankhimate_core::constraints::Constraint;

    let setup = state.session.can_edit_structure();
    let existing: Vec<(ankhimate_core::ids::ConstraintId, String, usize, PathProps)> = state
        .doc
        .skeleton
        .constraint_order
        .iter()
        .filter_map(|id| {
            let Some(Constraint::Path(p)) = state.doc.skeleton.constraints.get(*id) else {
                return None;
            };
            p.bones.contains(&bone_id).then(|| {
                (
                    *id,
                    p.name.clone(),
                    p.bones.len(),
                    PathProps {
                        position: p.position,
                        spacing: p.spacing,
                        mix_rotate: p.mix_rotate,
                        mix_translate: p.mix_translate,
                    },
                )
            })
        })
        .collect();

    let mut edit: Option<(ankhimate_core::ids::ConstraintId, PathProps)> = None;
    let mut remove: Option<ankhimate_core::ids::ConstraintId> = None;

    for (id, name, count, props) in &existing {
        let mut next = *props;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{name} (path, {count} bones)"))
                    .size(11.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(setup, egui::Button::new("✕").small())
                    .on_hover_text("Delete this constraint")
                    .clicked()
                {
                    remove = Some(*id);
                }
            });
        });
        for (label, value, range, hint) in [
            (
                "Position",
                &mut next.position,
                0.0..=1.0,
                "Where the chain starts along the path — animate it to slide the chain",
            ),
            (
                "Spacing",
                &mut next.spacing,
                0.0..=2.0,
                "Gap between bones. 1 spreads them over the whole path.",
            ),
            (
                "Rotate",
                &mut next.mix_rotate,
                0.0..=1.0,
                "How much of the path's direction each bone takes",
            ),
            (
                "Translate",
                &mut next.mix_translate,
                0.0..=1.0,
                "How much of the path's position each bone takes",
            ),
        ] {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [64.0, 18.0],
                    egui::Label::new(egui::RichText::new(label).size(10.5)),
                );
                ui.add_enabled(setup, egui::Slider::new(value, range).max_decimals(2))
                    .on_hover_text(hint);
            });
        }
        if next != *props {
            edit = Some((*id, next));
        }
    }

    // Creating one needs a slot that actually holds a path, and the chain is the
    // current bone selection — the same gesture that creates an IK target.
    let paths: Vec<(ankhimate_core::ids::SlotId, String)> = state
        .doc
        .skeleton
        .slots
        .iter()
        .filter(|(id, slot)| {
            slot.attachment.as_deref().is_some_and(|name| {
                matches!(
                    state.doc.skeleton.skins[state.doc.skeleton.default_skin].get(*id, name),
                    Some(Attachment::Path(_))
                )
            })
        })
        .map(|(id, slot)| (id, slot.name.clone()))
        .collect();

    if !paths.is_empty() {
        ui.add_space(4.0);
        let chain = state.session.selected_bones.clone();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Drive along").size(10.5));
            egui::ComboBox::from_id_salt(("path_pick", bone_id))
                .selected_text("choose a path…")
                .show_ui(ui, |ui| {
                    for (slot, name) in &paths {
                        if ui.selectable_label(false, name).clicked() && setup {
                            let bones = if chain.is_empty() {
                                vec![bone_id]
                            } else {
                                chain.clone()
                            };
                            let label =
                                format!("path {}", state.doc.skeleton.constraints.len() + 1);
                            state.dispatch(Box::new(AddPathConstraint::new(label, *slot, bones)));
                        }
                    }
                });
        })
        .response
        .on_hover_text("Selected bones follow the chosen path, in selection order");
    }

    if let Some((id, props)) = edit {
        state.dispatch(Box::new(SetPathProps::new(id, props)));
    }
    if let Some(id) = remove {
        state.dispatch(Box::new(RemoveConstraint::new(id)));
    }
}

/// The selected vertex's bone influences, as numbers (T-403).
///
/// A heat map answers "roughly how much", which is what painting needs. It
/// cannot answer "is this exactly 1.0, or 0.98 with a stray influence left over"
/// — and a stray 0.02 on the wrong bone is precisely the bug that makes a mesh
/// twitch in one frame of an animation. So: the numbers, editable, with the
/// stray removable.
///
/// Shown for a single selected vertex. Multi-select would have to average or
/// list every combination, and neither reads as an answer.
fn influence_list(
    ui: &mut egui::Ui,
    state: &mut AppState,
    skin: ankhimate_core::ids::SkinId,
    slot_id: ankhimate_core::ids::SlotId,
    name: &str,
) {
    use ankhimate_core::attachment::{Attachment, VertexWeight};
    use ankhimate_core::ids::BoneId;

    let selected: Vec<usize> = state.session.selected_vertices.clone();
    if selected.len() != 1 {
        return;
    }
    let index = selected[0];

    let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot_id, name) else {
        return;
    };
    let Some(influences) = mesh.weights.get(index) else {
        return;
    };
    // Names resolved up front: the edit below needs `&mut state`, and a bone
    // lookup mid-edit would borrow it twice.
    let rows: Vec<(BoneId, String, f32)> = influences
        .iter()
        .map(|w| {
            let label = state
                .doc
                .skeleton
                .bones
                .get(w.bone)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "<deleted bone>".to_string());
            (w.bone, label, w.weight)
        })
        .collect();
    let total: f32 = rows.iter().map(|(_, _, w)| *w).sum();
    let all_weights = mesh.weights.clone();
    let setup = state.session.can_edit_structure();

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(format!("Influences · vertex {index}"))
            .size(11.0)
            .strong(),
    );
    if rows.is_empty() {
        ui.label(
            egui::RichText::new("Unweighted — rides its slot's bone rigidly")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    let mut edit: Option<Vec<Vec<VertexWeight>>> = None;
    for (bone, label, weight) in &rows {
        ui.horizontal(|ui| {
            let mut value = *weight;
            if ui
                .add_enabled(
                    setup,
                    egui::DragValue::new(&mut value)
                        .speed(0.01)
                        .range(0.0..=1.0)
                        .max_decimals(3),
                )
                .changed()
            {
                let mut next = all_weights.clone();
                if let Some(w) = next
                    .get_mut(index)
                    .and_then(|v| v.iter_mut().find(|w| w.bone == *bone))
                {
                    w.weight = value;
                }
                edit = Some(next);
            }
            ui.label(egui::RichText::new(label).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(setup, egui::Button::new("✕").small())
                    .on_hover_text("Remove this influence")
                    .clicked()
                {
                    let mut next = all_weights.clone();
                    if let Some(v) = next.get_mut(index) {
                        v.retain(|w| w.bone != *bone);
                    }
                    edit = Some(next);
                }
            });
        });
    }

    ui.horizontal(|ui| {
        // Weights that do not sum to 1 scale the vertex toward the origin, so
        // the total is worth saying out loud rather than leaving to be inferred
        // from a mesh that looks slightly wrong.
        let off = (total - 1.0).abs() > 1e-3;
        ui.label(
            egui::RichText::new(format!("Total {total:.3}"))
                .size(10.5)
                .color(if off {
                    egui::Color32::from_rgb(230, 150, 60)
                } else {
                    ui.visuals().weak_text_color()
                }),
        );
        if off
            && ui
                .add_enabled(setup, egui::Button::new("Normalize").small())
                .on_hover_text("Scale these influences so they sum to 1")
                .clicked()
        {
            let mut next = all_weights.clone();
            if let Some(v) = next.get_mut(index) {
                crate::commands::weight_cmds::normalize(v);
            }
            edit = Some(next);
        }
    });

    if let Some(weights) = edit {
        state.dispatch(Box::new(crate::commands::weight_cmds::PaintWeights::new(
            skin,
            slot_id,
            name.to_string(),
            weights,
        )));
    }
}

// ── Key affordances (T-210) ────────────────────────────────────────────────

/// What the user did to a key dot.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum DotAction {
    #[default]
    None,
    /// Plain click — key this property at the playhead.
    Key,
    /// Alt-click — remove the key that is here.
    Unkey,
}

/// A key-state dot for one property row.
///
/// Reading the dopesheet to find out whether the value in front of you is keyed
/// is a context switch on every edit; the dot puts that where the number is:
///
/// * empty outline — nothing animates this property
/// * hollow ring — a timeline exists but this frame is not keyed
/// * filled — keyed right here
/// * amber — posed but uncommitted (auto-key off), waiting for `K`
///
/// Only shown in Animate mode: in Setup there is nothing to key against.
fn key_dot(ui: &mut egui::Ui, keyed: crate::edit_router::KeyState) -> DotAction {
    use crate::edit_router::KeyState;

    let size = egui::vec2(14.0, 14.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let center = rect.center();
    let radius = 4.5;

    let accent = egui::Color32::from_rgb(235, 170, 60);
    let live = ui.visuals().selection.bg_fill;
    let dim = ui.visuals().weak_text_color();

    match keyed {
        KeyState::NoTimeline => {
            ui.painter().circle_stroke(
                center,
                radius,
                egui::Stroke::new(1.0, dim.gamma_multiply(0.6)),
            );
        }
        KeyState::Unkeyed => {
            ui.painter()
                .circle_stroke(center, radius, egui::Stroke::new(1.5, live));
        }
        KeyState::Keyed(_) => {
            ui.painter().circle_filled(center, radius, live);
        }
        KeyState::Modified => {
            ui.painter().circle_filled(center, radius, accent);
        }
    }
    if response.hovered() {
        ui.painter()
            .circle_stroke(center, radius + 2.0, egui::Stroke::new(1.0, live));
    }

    let response = response.on_hover_text(match keyed {
        KeyState::Keyed(_) => "Keyed here — click to update, Alt-click to remove",
        KeyState::Modified => "Unkeyed edit — click to key it at the playhead",
        _ => "Click to key this property at the playhead",
    });

    if response.clicked() {
        if ui.input(|i| i.modifiers.alt) {
            return DotAction::Unkey;
        }
        return DotAction::Key;
    }
    DotAction::None
}

/// A 3×3 pivot picker: nine dots inside a frame standing in for the image.
///
/// Returns the pivot that was clicked, in normalized image coordinates with
/// `(0,0)` at the bottom-left — screen rows run top-down, so the Y axis is
/// flipped when mapping a dot back to a value.
fn pivot_grid(ui: &mut egui::Ui, current: glam::Vec2) -> Option<glam::Vec2> {
    const SIZE: f32 = 46.0;
    const DOT: f32 = 3.0;
    const HIT: f32 = 8.0;

    let (rect, response) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click());
    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    let frame = visuals.widgets.noninteractive.bg_stroke.color;
    let enabled = ui.is_enabled();

    let painter = ui.painter();
    // The frame is the image; the dots are where the pivot can land on it.
    painter.rect_filled(
        rect,
        egui::epaint::CornerRadius::same(3),
        visuals.extreme_bg_color,
    );
    painter.rect_stroke(
        rect,
        egui::epaint::CornerRadius::same(3),
        egui::Stroke::new(1.0, frame),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(9.0);
    let pos_of = |col: usize, row: usize| {
        egui::pos2(
            inner.min.x + inner.width() * col as f32 * 0.5,
            inner.min.y + inner.height() * row as f32 * 0.5,
        )
    };
    let hover = response.hover_pos();
    let mut clicked = None;

    for row in 0..3 {
        for col in 0..3 {
            let p = pos_of(col, row);
            // Row 0 is the top of the image, which is v = 1.
            let value = glam::vec2(col as f32 * 0.5, 1.0 - row as f32 * 0.5);
            let selected = (current - value).length() < 1e-3;
            let hovered = enabled && hover.is_some_and(|h| (h - p).length() <= HIT);

            let color = if !enabled {
                visuals.weak_text_color().gamma_multiply(0.5)
            } else if selected {
                accent
            } else if hovered {
                visuals.text_color()
            } else {
                visuals.weak_text_color()
            };

            if selected {
                painter.circle_filled(p, DOT + 3.0, accent.gamma_multiply(0.25));
            }
            painter.circle_filled(p, if selected || hovered { DOT + 1.0 } else { DOT }, color);

            if hovered && response.clicked() {
                clicked = Some(value);
            }
        }
    }

    let label = |v: glam::Vec2| match (v.x, v.y) {
        (0.0, 1.0) => "top-left",
        (0.5, 1.0) => "top",
        (1.0, 1.0) => "top-right",
        (0.0, 0.5) => "left",
        (0.5, 0.5) => "centre",
        (1.0, 0.5) => "right",
        (0.0, 0.0) => "bottom-left",
        (0.5, 0.0) => "bottom",
        _ => "bottom-right",
    };
    let response = match hover.and_then(|h| {
        (0..3)
            .flat_map(|row| (0..3).map(move |col| (col, row)))
            .map(|(col, row)| {
                (
                    glam::vec2(col as f32 * 0.5, 1.0 - row as f32 * 0.5),
                    (h - pos_of(col, row)).length(),
                )
            })
            .filter(|(_, d)| *d <= HIT)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }) {
        Some((value, _)) => response.on_hover_text(format!("Pivot: {}", label(value))),
        None => response.on_hover_text("Pivot presets"),
    };
    let _ = response;

    clicked
}

/// Lay out a property row with its key dot on the right.
///
/// The dot is allocated first so the row shrinks to fit; the row widgets size
/// themselves from `available_width`, and they would otherwise eat the space.
/// In Setup mode there is no dot and the row keeps the full width.
fn keyed_row(
    ui: &mut egui::Ui,
    animating: bool,
    keyed: crate::edit_router::KeyState,
    body: impl FnOnce(&mut egui::Ui) -> bool,
) -> (bool, DotAction) {
    if !animating {
        return (body(ui), DotAction::None);
    }

    const DOT_W: f32 = 18.0;
    let mut changed = false;
    let mut action = DotAction::None;
    ui.horizontal(|ui| {
        let width = (ui.available_width() - DOT_W).max(60.0);
        ui.allocate_ui(egui::vec2(width, FIELD_H), |ui| {
            changed = body(ui);
        });
        action = key_dot(ui, keyed);
    });
    (changed, action)
}

/// Key or unkey one bone property at the playhead.
fn apply_dot_action(
    ui_state: &mut AppState,
    bone: ankhimate_core::ids::BoneId,
    property: BoneProperty,
    action: DotAction,
) {
    use crate::commands::key_cmds::{AddKey, DeleteKeys, KeyRef};
    use ankhimate_core::animation::Interp;

    let Some(anim) = ui_state.session.active_animation else {
        return;
    };
    let addr = TimelineAddr::Bone { bone, property };

    match action {
        DotAction::None => {}
        DotAction::Key => {
            // The value the viewport is showing, including an uncommitted pose —
            // clicking the dot is how you commit exactly that.
            let Some(value) =
                crate::edit_router::bone_key_value(&ui_state.doc, &ui_state.pose, bone, property)
            else {
                return;
            };
            ui_state.dispatch(Box::new(AddKey::new(
                anim,
                addr,
                ui_state.session.playhead,
                value,
                Interp::Linear,
            )));
            ui_state.session.clear_previews();
        }
        DotAction::Unkey => {
            if let crate::edit_router::KeyState::Keyed(index) =
                crate::edit_router::key_state(&ui_state.doc, &ui_state.session, &addr)
            {
                ui_state.dispatch(Box::new(DeleteKeys::new(
                    anim,
                    vec![KeyRef { addr, index }],
                )));
            }
        }
    }
}

// ── Attachment inspector (T-307): where the art sits inside its slot ────────

/// Edit the resolved attachment's own transform.
///
/// This is the difference between "the image renders" and "the image can be
/// placed": without it, art can only be moved by moving its bone, which drags
/// the whole rig around to fix a placement problem.
///
/// Edits land in the skin the attachment was *resolved from* (active, else
/// default — ADR 0003), so changing a value never silently forks an override
/// into a skin the user is not looking at.
fn attachment_inspector(
    ui: &mut egui::Ui,
    state: &mut AppState,
    slot_id: ankhimate_core::ids::SlotId,
) {
    use crate::commands::attachment_cmds::{
        DuplicateAttachment, RegionProps, RemoveAttachment, RenameAttachment, SetRegionProps,
        owning_skin,
    };
    use ankhimate_core::attachment::Attachment;

    let Some(name) = state
        .doc
        .skeleton
        .slots
        .get(slot_id)
        .and_then(|s| s.attachment.clone())
    else {
        // An empty slot is where a clip is made: a clipping attachment carries
        // no art, so a slot holding one holds nothing else. Offering it here
        // makes "an empty slot" read as a choice rather than as a mistake.
        ui.add_space(8.0);
        if ui
            .add_enabled(
                state.session.can_edit_structure(),
                egui::Button::new("Add clipping mask"),
            )
            .on_hover_text(
                "Mask every slot drawn after this one with a polygon — a window, \
                 a porthole, a spotlight",
            )
            .clicked()
        {
            let skin = state.session.active_skin;
            // Named after the slot: a clip is one-per-slot in practice, and a
            // name nobody chose still has to be recognisable in the skin.
            let slot_name = state
                .doc
                .skeleton
                .slots
                .get(slot_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "slot".to_string());
            if state.dispatch(Box::new(crate::commands::clip_cmds::AddClipping::new(
                skin,
                slot_id,
                format!("{slot_name}_clip"),
                200.0,
            ))) {
                state.session.mesh_edit = true;
                state.session.selected_vertices.clear();
            }
        }
        if ui
            .add_enabled(
                state.session.can_edit_structure(),
                egui::Button::new("Add path"),
            )
            .on_hover_text(
                "A curve to drive bones along — a tail, a tread, a belt, vines                  following a spline",
            )
            .clicked()
        {
            let skin = state.session.active_skin;
            let slot_name = state
                .doc
                .skeleton
                .slots
                .get(slot_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "slot".to_string());
            if state.dispatch(Box::new(crate::commands::clip_cmds::AddPath::new(
                skin,
                slot_id,
                format!("{slot_name}_path"),
                200.0,
            ))) {
                state.session.mesh_edit = true;
                state.session.selected_vertices.clear();
            }
        }
        if ui
            .add_enabled(
                state.session.can_edit_structure(),
                egui::Button::new("Add bounding box"),
            )
            .on_hover_text(
                "A polygon a game can hit-test against — a hurtbox, a trigger \
                 region, a pickup zone. Follows the pose; draws nothing.",
            )
            .clicked()
        {
            let skin = state.session.active_skin;
            let slot_name = state
                .doc
                .skeleton
                .slots
                .get(slot_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "slot".to_string());
            if state.dispatch(Box::new(crate::commands::clip_cmds::AddBoundingBox::new(
                skin,
                slot_id,
                format!("{slot_name}_box"),
                120.0,
            ))) {
                state.session.mesh_edit = true;
                state.session.selected_vertices.clear();
            }
        }
        if ui
            .add_enabled(
                state.session.can_edit_structure(),
                egui::Button::new("Add point"),
            )
            .on_hover_text(
                "An anchor with a heading — a muzzle, a footstep spark, \
                 \"hold the sword here\"",
            )
            .clicked()
        {
            let skin = state.session.active_skin;
            let slot_name = state
                .doc
                .skeleton
                .slots
                .get(slot_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "slot".to_string());
            state.dispatch(Box::new(crate::commands::clip_cmds::AddPoint::new(
                skin,
                slot_id,
                format!("{slot_name}_point"),
            )));
        }
        return;
    };
    let Some(skin) = owning_skin(&state.doc, state.session.active_skin, slot_id, &name) else {
        // The slot names an attachment no skin defines — a dangling reference
        // the diagnostics pass will flag (T-702). Say so rather than showing an
        // empty section.
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(format!("Attachment '{name}' is missing from every skin"))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };

    let setup = state.session.can_edit_structure();

    // Mesh attachments get their own small section: their geometry is edited on
    // the canvas (T-401), not through transform fields.
    if let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot_id, &name) {
        let (vertices, triangles) = (mesh.setup_vertices.len(), mesh.triangles.len());
        let pinned_edges = mesh.edges.len();
        ui.add_space(10.0);
        section_header(ui, crate::ui::icons::MESH, "Mesh");
        ui.add_space(2.0);
        info_row(ui, "Vertices", &vertices.to_string());
        info_row(ui, "Triangles", &triangles.to_string());
        if pinned_edges > 0 {
            info_row(ui, "Pinned edges", &pinned_edges.to_string());
        }
        ui.add_space(4.0);

        let mut editing = state.session.mesh_edit;
        if ui
            .add_enabled(setup, egui::Checkbox::new(&mut editing, "Edit vertices"))
            .on_hover_text(
                "Drag vertices · Ctrl+drag to box-select · shift-click to toggle\n\
                 Click an edge to add a vertex · X deletes · Esc leaves\n\
                 C pins the edge between two selected vertices, or releases it",
            )
            .changed()
        {
            state.session.mesh_edit = editing;
            state.session.selected_vertices.clear();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(setup, egui::Button::new("Retriangulate").small())
                .on_hover_text("Rebuild triangles from the current vertices")
                .clicked()
            {
                state.dispatch(Box::new(crate::commands::mesh_cmds::EditMesh::new(
                    skin,
                    slot_id,
                    name.clone(),
                    crate::commands::mesh_cmds::MeshEdit::Retriangulate,
                )));
            }
            if ui
                .add_enabled(setup, egui::Button::new("Trace from image…").small())
                .on_hover_text(
                    "Rebuild the mesh from the artwork's silhouette, with a preview \
                     of the settings",
                )
                .clicked()
            {
                // The settings need a preview to mean anything, so they get a
                // window rather than two spinboxes in a side panel (T-402).
                state.session.pending_trace = Some(crate::ui::trace::PendingTrace::new(
                    skin,
                    slot_id,
                    name.clone(),
                ));
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(setup, egui::Button::new("UV editor…").small())
                .on_hover_text(
                    "Drag where each vertex samples the texture — the mesh's other                      shape, which the canvas cannot show",
                )
                .clicked()
            {
                state.session.uv_pane =
                    Some(crate::ui::uv::UvPane::new(skin, slot_id, name.clone()));
                // Bring the pane forward, adding it back if the tab was closed.
                // Setting the target without surfacing it looks like the button
                // did nothing.
                state.session.focus_tab = Some(crate::ui::Tab::UvEditor);
            }
            let two_selected = state.session.selected_vertices.len() == 2;
            if ui
                .add_enabled(
                    setup && two_selected,
                    egui::Button::new("Pin/release edge").small(),
                )
                .on_hover_text(
                    "Force the triangulation to keep the edge between the two                      selected vertices (C on the canvas)",
                )
                .clicked()
                && let [a, b] = state.session.selected_vertices[..]
            {
                let pinned = {
                    let edge = [(a.min(b)) as u32, (a.max(b)) as u32];
                    matches!(
                        state.doc.skeleton.skins[skin].get(slot_id, &name),
                        Some(Attachment::Mesh(m)) if m.edges.contains(&edge)
                    )
                };
                let edit = if pinned {
                    crate::commands::mesh_cmds::MeshEdit::RemoveEdge(a, b)
                } else {
                    crate::commands::mesh_cmds::MeshEdit::AddEdge(a, b)
                };
                state.dispatch(Box::new(crate::commands::mesh_cmds::EditMesh::new(
                    skin, slot_id, name.clone(), edit,
                )));
            }
        });
        influence_list(ui, state, skin, slot_id, &name);
        return;
    }

    // Bounding boxes: the same polygon editor a clip uses, plus what makes them
    // different — whether they follow one bone or several.
    if let Some(Attachment::BoundingBox(bb)) = state.doc.skeleton.skins[skin].get(slot_id, &name) {
        let vertices = bb.vertices.len();
        let skinned = !bb.weights.is_empty();
        ui.add_space(10.0);
        section_header(ui, crate::ui::icons::HITBOX, "Bounding Box");
        ui.add_space(2.0);
        info_row(ui, "Vertices", &vertices.to_string());
        info_row(
            ui,
            "Follows",
            if skinned {
                "weighted bones"
            } else {
                "its slot's bone"
            },
        );
        ui.add_space(4.0);

        let mut editing = state.session.mesh_edit;
        if ui
            .add_enabled(setup, egui::Checkbox::new(&mut editing, "Edit polygon"))
            .on_hover_text(
                "Drag vertices · Ctrl+drag to box-select · shift-click to toggle\n\
                 Click an edge to add a vertex · X deletes · Esc leaves",
            )
            .changed()
        {
            state.session.mesh_edit = editing;
            state.session.selected_vertices.clear();
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Weight it like a mesh to have it deform with a limb rather than \
                 ride one bone rigidly.",
            )
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    // Points: two numbers and an angle, so they get plain fields rather than a
    // canvas mode.
    if let Some(Attachment::Point(point)) = state.doc.skeleton.skins[skin].get(slot_id, &name) {
        let mut position = point.position;
        let mut degrees = point.rotation.to_degrees();
        ui.add_space(10.0);
        section_header(ui, crate::ui::icons::POINT, "Point");
        ui.add_space(4.0);

        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("X").size(11.0));
            changed |= ui
                .add_enabled(setup, egui::DragValue::new(&mut position.x).speed(0.5))
                .changed();
            ui.label(egui::RichText::new("Y").size(11.0));
            changed |= ui
                .add_enabled(setup, egui::DragValue::new(&mut position.y).speed(0.5))
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Angle").size(11.0));
            changed |= ui
                .add_enabled(
                    setup,
                    egui::DragValue::new(&mut degrees).speed(0.5).suffix("°"),
                )
                .changed();
        });
        if changed {
            state.dispatch(Box::new(crate::commands::clip_cmds::SetPoint::new(
                skin,
                slot_id,
                name.clone(),
                ankhimate_core::attachment::PointAttachment {
                    position,
                    rotation: degrees.to_radians(),
                },
            )));
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Exported for a game to spawn from; draws nothing.")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    // Clipping attachments (T-405): a polygon and the range of slots it masks.
    if let Some(Attachment::Clipping(clip)) = state.doc.skeleton.skins[skin].get(slot_id, &name) {
        let vertices = clip.vertices.len();
        let end_slot = clip.end_slot.clone();
        ui.add_space(10.0);
        section_header(ui, crate::ui::icons::CLIP, "Clipping");
        ui.add_space(2.0);
        info_row(ui, "Vertices", &vertices.to_string());
        ui.add_space(4.0);

        let mut editing = state.session.mesh_edit;
        if ui
            .add_enabled(setup, egui::Checkbox::new(&mut editing, "Edit polygon"))
            .on_hover_text(
                "Drag vertices · Ctrl+drag to box-select · shift-click to toggle\n\
                 Click an edge to add a vertex · X deletes · Esc leaves",
            )
            .changed()
        {
            state.session.mesh_edit = editing;
            state.session.selected_vertices.clear();
        }

        // Which slots this masks. The clip runs from its own place in the draw
        // order until this slot, inclusive — naming the *end* rather than a list
        // is what makes "everything behind the window" one choice instead of a
        // checkbox per slot.
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Masks until").size(11.0));
        let current = end_slot
            .clone()
            .unwrap_or_else(|| "(end of draw order)".to_string());
        let mut chosen: Option<Option<String>> = None;
        egui::ComboBox::from_id_salt("clip_end_slot")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(end_slot.is_none(), "(end of draw order)")
                    .clicked()
                {
                    chosen = Some(None);
                }
                let names: Vec<String> = state
                    .doc
                    .skeleton
                    .draw_order
                    .iter()
                    .filter_map(|id| state.doc.skeleton.slots.get(*id).map(|s| s.name.clone()))
                    .collect();
                for slot_name in names {
                    if ui
                        .selectable_label(end_slot.as_deref() == Some(&slot_name), &slot_name)
                        .clicked()
                    {
                        chosen = Some(Some(slot_name));
                    }
                }
            });
        if let Some(end) = chosen {
            state.dispatch(Box::new(crate::commands::clip_cmds::EditClip::new(
                skin,
                slot_id,
                name.clone(),
                crate::commands::clip_cmds::ClipEdit::SetEndSlot(end),
            )));
        }
        return;
    }

    let Some(Attachment::Region(region)) = state.doc.skeleton.skins[skin].get(slot_id, &name)
    else {
        return;
    };
    let props = RegionProps::from_region(region);
    let texture = region.texture.clone();
    let asset_size = state
        .doc
        .assets
        .by_name(&texture)
        .and_then(|id| state.doc.assets.get(id))
        .map(|a| a.size());
    let skin_name = state.doc.skeleton.skins[skin].name.clone();
    let setup = state.session.can_edit_structure();

    ui.add_space(10.0);
    section_header(ui, crate::ui::icons::ATTACHMENT, "Attachment");
    ui.add_space(2.0);
    info_row(ui, "Image", &texture);
    info_row(ui, "In skin", &skin_name);
    ui.add_space(4.0);

    if !setup {
        ui.label(
            egui::RichText::new("Animating — switch to Setup (Tab) to place art")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);
    }

    // Canvas target toggle (T-307). Explicit, because "drag the bone" and "drag
    // the art" look identical as a gesture — inferring it from the selection
    // would make every stray slot click change what a drag does.
    let mut on_canvas = state.session.edit_target == crate::session::EditTarget::Attachment;
    if ui
        .add_enabled(setup, egui::Checkbox::new(&mut on_canvas, "Edit on canvas"))
        .on_hover_text(
            "Transform tools drag this artwork instead of the bone.\n\
             Grab the pivot crosshair; Alt-drag moves the pivot itself.",
        )
        .changed()
    {
        state.session.edit_target = if on_canvas {
            crate::session::EditTarget::Attachment
        } else {
            crate::session::EditTarget::Bone
        };
    }
    ui.add_space(4.0);

    // Name is the reference other things hold, so it gets its own row and is
    // committed on focus loss rather than per keystroke.
    let mut edited_name = name.clone();
    let mut rename: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label("Name");
        let field = ui.add_enabled(
            setup,
            egui::TextEdit::singleline(&mut edited_name).desired_width(140.0),
        );
        if field.lost_focus() && edited_name != name && !edited_name.trim().is_empty() {
            rename = Some(edited_name.trim().to_string());
        }
    });
    ui.add_space(ROW_GAP);

    let (mut ox, mut oy) = (props.offset.x, props.offset.y);
    let mut rot = props.rotation.to_degrees();
    let (mut sx, mut sy) = (props.scale.x, props.scale.y);
    let (mut w, mut h) = (props.width, props.height);
    let (mut px, mut py) = (props.pivot.x, props.pivot.y);

    let mut changed = false;
    let mut pivot_changed = false;
    ui.add_enabled_ui(setup, |ui| {
        changed |= transform_row_single(ui, "Rotate", &mut rot, 0.5, 2, SINGLE_COLOR, false);
        ui.add_space(ROW_GAP);
        changed |= transform_row_xy(ui, "Offset", &mut ox, &mut oy, 0.5, 2, false);
        ui.add_space(ROW_GAP);
        changed |= transform_row_xy(ui, "Scale", &mut sx, &mut sy, 0.01, 3, false);
        ui.add_space(ROW_GAP);
        changed |= transform_row_xy(ui, "Size", &mut w, &mut h, 0.5, 2, false);
        ui.add_space(ROW_GAP);
        // Pivot: what the image turns and scales around, normalized so it
        // survives a resize — 0,0 is the bottom-left corner, 1,1 the top-right.
        pivot_changed = transform_row_xy(ui, "Pivot", &mut px, &mut py, 0.005, 3, false);
    });

    // Nine-point presets as a 3×3 grid: it is a picture of the image with the
    // pivot on it, which reads instantly — a row of arrow glyphs does not.
    ui.add_space(4.0);
    let mut preset: Option<glam::Vec2> = None;
    ui.add_enabled_ui(setup, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            preset = pivot_grid(ui, glam::vec2(px, py));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Snap the pivot to a\ncorner, edge or centre")
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });

    let mut reset_size = false;
    let mut duplicate = false;
    let mut remove = false;
    let mut to_mesh = false;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let btn = ui.add_enabled(
            setup && asset_size.is_some(),
            egui::Button::new("Reset size").small(),
        );
        if btn
            .on_hover_text("Restore the image's own pixel dimensions")
            .clicked()
        {
            reset_size = true;
        }
        if ui
            .add_enabled(setup, egui::Button::new("Duplicate").small())
            .on_hover_text("Copy under a new name in this slot — the way to build a swap set")
            .clicked()
        {
            duplicate = true;
        }
        if ui
            .add_enabled(setup, egui::Button::new("Remove").small())
            .on_hover_text("Drop this attachment from this skin")
            .clicked()
        {
            remove = true;
        }
        if ui
            .add_enabled(setup, egui::Button::new("To mesh").small())
            .on_hover_text("Convert to a deformable mesh — the quad stays exactly where it is")
            .clicked()
        {
            to_mesh = true;
        }
    });

    // Apply after the layout so no command runs while the widgets are borrowed.
    if let Some(new_name) = rename {
        state.dispatch(Box::new(RenameAttachment::new(
            skin,
            slot_id,
            name.clone(),
            new_name,
        )));
        return;
    }
    if reset_size && let Some(size) = asset_size {
        w = size.x;
        h = size.y;
        changed = true;
    }

    // A pivot change compensates the offset so the art stays where it is; the
    // other fields are plain edits.
    let next = RegionProps {
        offset: glam::vec2(ox, oy),
        rotation: rot.to_radians(),
        scale: glam::vec2(sx, sy),
        width: w,
        height: h,
        pivot: props.pivot,
    };
    let next = match (preset, pivot_changed) {
        (Some(value), _) => Some(next.with_pivot_keeping_position(value)),
        (None, true) => Some(next.with_pivot_keeping_position(glam::vec2(px, py))),
        (None, false) if changed => Some(next),
        _ => None,
    };
    if let Some(next) = next {
        state.dispatch(Box::new(SetRegionProps::new(
            skin,
            slot_id,
            name.clone(),
            next,
        )));
    }
    if duplicate {
        state.dispatch(Box::new(DuplicateAttachment::new(
            skin,
            slot_id,
            name.clone(),
        )));
    }
    if to_mesh
        && state.dispatch(Box::new(crate::commands::mesh_cmds::ConvertToMesh::new(
            skin,
            slot_id,
            name.clone(),
        )))
    {
        // Straight into vertex editing: converting is only ever a prelude to
        // moving a vertex.
        state.session.mesh_edit = true;
        state.session.selected_vertices.clear();
    }
    if remove {
        state.dispatch(Box::new(RemoveAttachment::new(skin, slot_id, name)));
    }
}

// ── Slot inspector (T-205): keyable color + attachment ─────────────────────

fn slot_inspector(ui: &mut egui::Ui, state: &mut AppState, slot_id: ankhimate_core::ids::SlotId) {
    let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
        return;
    };
    let slot_name = slot.name.clone();
    // The pose color reflects any active SlotColor key; edits are relative to the
    // setup color, but showing the live value keeps the picker honest.
    let current = state
        .pose
        .slot_colors
        .get(slot_id)
        .copied()
        .unwrap_or(slot.color);
    let current_attachment = slot.attachment.clone();

    section_header(ui, crate::ui::icons::IMAGE, &slot_name);
    ui.add_space(4.0);

    // Color picker (keyable). egui works in sRGB Color32.
    let animating = state.session.is_animating();
    let color_addr = TimelineAddr::SlotColor { slot: slot_id };
    let color_keyed = crate::edit_router::key_state(&state.doc, &state.session, &color_addr);
    let mut new_color: Option<[f32; 4]> = None;
    let mut color_dot = DotAction::None;
    ui.horizontal(|ui| {
        ui.label("Color");
        let mut rgba = egui::Color32::from_rgba_unmultiplied(
            (current[0] * 255.0) as u8,
            (current[1] * 255.0) as u8,
            (current[2] * 255.0) as u8,
            (current[3] * 255.0) as u8,
        );
        if ui.color_edit_button_srgba(&mut rgba).changed() {
            new_color = Some([
                rgba.r() as f32 / 255.0,
                rgba.g() as f32 / 255.0,
                rgba.b() as f32 / 255.0,
                rgba.a() as f32 / 255.0,
            ]);
        }
        if animating {
            color_dot = key_dot(ui, color_keyed);
        }
    });
    if let Some(color) = new_color {
        state.commit_slot_color(slot_id, color);
    }
    match color_dot {
        DotAction::Key => {
            if let Some(anim) = state.session.active_animation {
                // Keys the colour the viewport is showing, so the dot means the
                // same thing here as it does on a transform row.
                state.dispatch(Box::new(crate::commands::key_cmds::AddKey::new(
                    anim,
                    color_addr.clone(),
                    state.session.playhead,
                    crate::commands::key_cmds::KeyValue::Color(current),
                    ankhimate_core::animation::Interp::Linear,
                )));
            }
        }
        DotAction::Unkey => {
            if let (Some(anim), crate::edit_router::KeyState::Keyed(index)) =
                (state.session.active_animation, color_keyed)
            {
                state.dispatch(Box::new(crate::commands::key_cmds::DeleteKeys::new(
                    anim,
                    vec![crate::commands::key_cmds::KeyRef {
                        addr: color_addr,
                        index,
                    }],
                )));
            }
        }
        DotAction::None => {}
    }

    // ── Presentation (T-505) ──────────────────────────────────────────────
    // How the slot composites, and whether it draws at all.
    {
        use crate::commands::slot_cmds::{SetSlotPresentation, SlotPresentation};
        use ankhimate_core::slot::BlendMode;

        let setup = state.session.can_edit_structure();
        let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
            return;
        };
        let mut presentation = SlotPresentation {
            blend_mode: slot.blend_mode,
            dark_color: slot.dark_color,
        };
        let original = presentation;

        ui.horizontal(|ui| {
            ui.label("Blend");
            egui::ComboBox::from_id_salt(("slot_blend", slot_id))
                .selected_text(match presentation.blend_mode {
                    BlendMode::Normal => "Normal",
                    BlendMode::Additive => "Additive",
                    BlendMode::Multiply => "Multiply",
                    BlendMode::Screen => "Screen",
                })
                .show_ui(ui, |ui| {
                    for (mode, label, hint) in [
                        (BlendMode::Normal, "Normal", "Ordinary alpha compositing"),
                        (
                            BlendMode::Additive,
                            "Additive",
                            "Light emitted rather than surface shown — flashes, sparks, glows",
                        ),
                        (
                            BlendMode::Multiply,
                            "Multiply",
                            "Darkens what is underneath — shadows, stains",
                        ),
                        (
                            BlendMode::Screen,
                            "Screen",
                            "Lightens without blowing out — the inverse of multiply",
                        ),
                    ] {
                        if ui
                            .selectable_label(presentation.blend_mode == mode, label)
                            .on_hover_text(hint)
                            .clicked()
                        {
                            presentation.blend_mode = mode;
                        }
                    }
                });
        });

        // Two-color tint. Off by default and shown as a checkbox first, because
        // an always-visible second colour picker reads as "this slot has one"
        // when most do not.
        ui.horizontal(|ui| {
            let mut enabled = presentation.dark_color.is_some();
            if ui
                .add_enabled(setup, egui::Checkbox::new(&mut enabled, "Dark tint"))
                .on_hover_text(
                    "Fill what the texture leaves dark with a second colour — one \
                     sprite reads as both lit and shadowed",
                )
                .changed()
            {
                presentation.dark_color = enabled.then_some([0.0, 0.0, 0.0, 1.0]);
            }
            if let Some(dark) = presentation.dark_color {
                let mut rgba = egui::Color32::from_rgba_unmultiplied(
                    (dark[0] * 255.0) as u8,
                    (dark[1] * 255.0) as u8,
                    (dark[2] * 255.0) as u8,
                    (dark[3] * 255.0) as u8,
                );
                if ui.color_edit_button_srgba(&mut rgba).changed() {
                    presentation.dark_color = Some([
                        rgba.r() as f32 / 255.0,
                        rgba.g() as f32 / 255.0,
                        rgba.b() as f32 / 255.0,
                        rgba.a() as f32 / 255.0,
                    ]);
                }
            }
        });

        if presentation != original && setup {
            state.dispatch(Box::new(SetSlotPresentation::new(slot_id, presentation)));
        }
    }

    // ── Visibility (T-505) ────────────────────────────────────────────────
    // A keyable boolean, distinct from alpha 0: a hidden slot is not drawn at
    // all, and "hidden at frame 10, back at 20" must cut rather than fade.
    {
        let visible = state
            .pose
            .slot_visible
            .get(slot_id)
            .copied()
            .unwrap_or(true);
        let addr = TimelineAddr::SlotVisible { slot: slot_id };
        let keyed = crate::edit_router::key_state(&state.doc, &state.session, &addr);
        let animating = state.session.is_animating();
        let mut toggled: Option<bool> = None;
        let mut dot = DotAction::None;

        ui.horizontal(|ui| {
            let mut shown = visible;
            if ui
                .add_enabled(animating, egui::Checkbox::new(&mut shown, "Visible"))
                .on_hover_text(if animating {
                    "Keyable. A hidden slot is not drawn — unlike alpha 0, which is."
                } else {
                    "Switch to Animate mode (Tab) to key visibility"
                })
                .changed()
            {
                toggled = Some(shown);
            }
            if animating {
                dot = key_dot(ui, keyed);
            }
        });

        // Toggling in Animate mode *is* the key: there is nowhere else for the
        // value to live, since a slot's setup state is "visible".
        if let (Some(shown), Some(anim)) = (
            toggled.or(match dot {
                DotAction::Key => Some(visible),
                _ => None,
            }),
            state.session.active_animation,
        ) {
            state.dispatch(Box::new(crate::commands::key_cmds::AddKey::new(
                anim,
                addr.clone(),
                state.session.playhead,
                crate::commands::key_cmds::KeyValue::Visible(shown),
                ankhimate_core::animation::Interp::Stepped,
            )));
        }
        if let (DotAction::Unkey, Some(anim), crate::edit_router::KeyState::Keyed(index)) =
            (dot, state.session.active_animation, keyed)
        {
            state.dispatch(Box::new(crate::commands::key_cmds::DeleteKeys::new(
                anim,
                vec![crate::commands::key_cmds::KeyRef { addr, index }],
            )));
        }
    }

    // Attachment dropdown: the active skin's entries for this slot, plus "none".
    let names: Vec<String> = {
        let skin = state.session.active_skin;
        state
            .doc
            .skeleton
            .skins
            .get(skin)
            .map(|s| s.names_for_slot(slot_id).map(|s| s.to_string()).collect())
            .unwrap_or_default()
    };
    ui.horizontal(|ui| {
        ui.label("Attachment");
        let selected = current_attachment
            .clone()
            .unwrap_or_else(|| "—".to_string());
        egui::ComboBox::from_id_salt("slot_attachment")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(current_attachment.is_none(), "— none —")
                    .clicked()
                {
                    state.commit_slot_attachment(slot_id, None);
                }
                for name in &names {
                    let is_sel = current_attachment.as_deref() == Some(name.as_str());
                    if ui.selectable_label(is_sel, name).clicked() {
                        state.commit_slot_attachment(slot_id, Some(name.clone()));
                    }
                }
            });
    });
}

// ── Section header with left accent bar ───────────────────────────────────

pub fn section_header(ui: &mut egui::Ui, icon: &str, label: &str) {
    ui.add_space(2.0);
    let accent = ui.visuals().selection.bg_fill;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 20.0), egui::Sense::hover());

    // Left accent bar
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(ACCENT_W, rect.height())),
        egui::epaint::CornerRadius::same(1),
        accent,
    );

    let mut x = rect.min.x + ACCENT_W + 6.0;
    // Icon
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        egui::FontId::proportional(12.0),
        accent,
    );
    x += 16.0;
    // Label
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        ui.visuals().strong_text_color(),
    );

    // Subtle underline
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x, rect.max.y),
            egui::pos2(rect.max.x, rect.max.y),
        ],
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
    ui.add_space(4.0);
}

// ── Info row (read-write, label + value) ─────────────────────────────────

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());
    let lc = ui.visuals().weak_text_color();
    let vc = ui.visuals().text_color();
    ui.painter().text(
        egui::pos2(rect.min.x + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(11.5),
        lc,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + LABEL_W, rect.center().y),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::proportional(11.5),
        vc,
    );
}

// ── Transform row: single field (e.g. Rotate) ─────────────────────────────
// Returns true if value changed.

fn transform_row_single(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f32,
    speed: f64,
    decimals: usize,
    color: egui::Color32,
    active: bool,
) -> bool {
    let w = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(w, FIELD_H), egui::Sense::hover());

    draw_row_label(ui, row_rect, label, active);

    let field_w = w - LABEL_W - 4.0;
    let field_rect = egui::Rect::from_min_size(
        egui::pos2(row_rect.min.x + LABEL_W + 4.0, row_rect.min.y),
        egui::vec2(field_w, FIELD_H),
    );
    colored_drag(ui, field_rect, val, speed, decimals, color, "")
}

// ── Transform row: XY fields ──────────────────────────────────────────────
// Returns true if either value changed.

fn transform_row_xy(
    ui: &mut egui::Ui,
    label: &str,
    x: &mut f32,
    y: &mut f32,
    speed: f64,
    decimals: usize,
    active: bool,
) -> bool {
    let w = ui.available_width();
    let (row_rect, _) = ui.allocate_exact_size(egui::vec2(w, FIELD_H), egui::Sense::hover());

    draw_row_label(ui, row_rect, label, active);

    let fields_w = w - LABEL_W - 4.0;
    let field_w = (fields_w - 4.0) / 2.0;
    let x_rect = egui::Rect::from_min_size(
        egui::pos2(row_rect.min.x + LABEL_W + 4.0, row_rect.min.y),
        egui::vec2(field_w, FIELD_H),
    );
    let y_rect = egui::Rect::from_min_size(
        egui::pos2(x_rect.max.x + 4.0, row_rect.min.y),
        egui::vec2(field_w, FIELD_H),
    );

    let cx = colored_drag(ui, x_rect, x, speed, decimals, X_COLOR, "");
    let cy = colored_drag(ui, y_rect, y, speed, decimals, Y_COLOR, "");
    cx || cy
}

// ── Readonly world-transform row ─────────────────────────────────────────

fn readonly_row(ui: &mut egui::Ui, label: &str, value: &str) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 18.0), egui::Sense::hover());

    let dim = ui.visuals().weak_text_color();
    let light = ui.visuals().text_color().gamma_multiply(0.7);

    // Subtle left indent bar (dimmer than section headers)
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.min + egui::vec2(8.0, 3.0),
            egui::vec2(1.0, rect.height() - 6.0),
        ),
        0.0,
        dim.gamma_multiply(0.4),
    );

    ui.painter().text(
        egui::pos2(rect.min.x + 16.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(11.0),
        dim,
    );

    // Right-aligned value
    let value_rect =
        egui::Rect::from_min_max(egui::pos2(rect.min.x + LABEL_W, rect.min.y), rect.max);
    ui.painter().text(
        egui::pos2(value_rect.min.x, value_rect.center().y),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::monospace(11.0),
        light,
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn draw_row_label(ui: &egui::Ui, row_rect: egui::Rect, label: &str, active: bool) {
    let accent = ui.visuals().selection.bg_fill;
    let color = if active {
        accent
    } else {
        ui.visuals().weak_text_color()
    };

    // Active row gets a subtle highlight strip on far left
    if active {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(row_rect.min, egui::vec2(2.0, row_rect.height())),
            1.0,
            accent.linear_multiply(0.6),
        );
    }

    ui.painter().text(
        egui::pos2(row_rect.min.x + 8.0, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(11.5),
        color,
    );
}

/// Draws a DragValue with a colored left-strip accent and a themed background.
/// Returns true if the value changed.
fn colored_drag(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    val: &mut f32,
    speed: f64,
    decimals: usize,
    accent: egui::Color32,
    suffix: &'static str,
) -> bool {
    let bg = ui.visuals().extreme_bg_color;
    let border_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    let no_rounding = egui::epaint::CornerRadius {
        nw: 0,
        sw: 0,
        ne: 3,
        se: 3,
    };
    let inner = egui::Rect::from_min_max(rect.min + egui::vec2(ACCENT_W, 0.0), rect.max);

    // 1. Paint our own background covering the full field
    ui.painter()
        .rect_filled(rect, egui::epaint::CornerRadius::same(3), bg);

    // 2. DragValue rendered transparently so our background shows
    let mut changed = false;
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        // Make every state transparent — widget draws nothing behind text
        v.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        v.widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
        v.widgets.active.bg_fill = egui::Color32::TRANSPARENT;
        v.widgets.open.bg_fill = egui::Color32::TRANSPARENT;
        // DragValue text area uses extreme_bg_color — kill it too
        v.extreme_bg_color = egui::Color32::TRANSPARENT;
        // No widget strokes
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        v.widgets.active.bg_stroke = egui::Stroke::NONE;
        // Rounding: flat on left (strip covers), rounded on right
        v.widgets.inactive.corner_radius = no_rounding;
        v.widgets.hovered.corner_radius = no_rounding;
        v.widgets.active.corner_radius = no_rounding;

        changed = ui
            .put(
                inner,
                egui::DragValue::new(val)
                    .speed(speed)
                    .max_decimals(decimals)
                    .suffix(suffix),
            )
            .changed();
    });

    // 3. Colored left strip painted on top
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(ACCENT_W, rect.height())),
        egui::epaint::CornerRadius {
            nw: 3,
            sw: 3,
            ne: 0,
            se: 0,
        },
        accent,
    );

    // 4. Outer border
    ui.painter().rect_stroke(
        rect,
        egui::epaint::CornerRadius::same(3),
        egui::Stroke::new(1.0, border_color),
        egui::StrokeKind::Outside,
    );

    changed
}
