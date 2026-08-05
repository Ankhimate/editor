//! The weight painting panel.
//!
//! Everything about weights that is not a drag on the canvas: the brush's
//! settings, the list of bones actually influencing this mesh, and the whole-mesh
//! operations (auto-weight, prune, swap, copy/paste).
//!
//! Split out of the inspector because it outgrew it. The inspector's job is "what
//! is selected"; this is a mode's control surface, and interleaving the two made
//! both harder to read.
//!
//! The single idea holding it together: **the brush's `strength` is the weight it
//! drives toward**, not an amount it adds. Painting is then predictable — a dab
//! at the centre of a full-strength brush lands on 1.0, and the other bones on
//! that vertex give way proportionally.

use crate::app_state::AppState;
use crate::commands::attachment_cmds::owning_skin;
use crate::commands::weight_cmds::{
    self, BrushMode, SetWeights, auto_weight, prune, remove_bone, set_weight, swap_bones,
};
use ankhimate_core::attachment::{Attachment, MeshAttachment, VertexWeight};
use ankhimate_core::ids::{BoneId, SkinId, SlotId};
use eframe::egui;

/// Where the panel's target mesh lives, resolved once per frame.
struct Target {
    skin: SkinId,
    slot: SlotId,
    name: String,
}

fn target(state: &AppState) -> Option<(Target, MeshAttachment)> {
    let slot = state.session.active_slot()?;
    let name = state.doc.skeleton.slots.get(slot)?.attachment.clone()?;
    let skin = owning_skin(&state.doc, state.session.active_skin, slot, &name)?;
    let Attachment::Mesh(mesh) = state.doc.skeleton.skins[skin].get(slot, &name)? else {
        return None;
    };
    Some((
        Target {
            skin,
            slot,
            name: name.clone(),
        },
        mesh.clone(),
    ))
}

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let Some((target, mesh)) = target(state) else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(crate::ui::icons::WEIGHT_PAINT)
                    .size(22.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Select a slot with a mesh")
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    };

    controls(ui, state, &target, &mesh);
    ui.add_space(10.0);
    bound_bones(ui, state, &target, &mesh);
}

/// The brush, in one column of labelled rows.
///
/// Laid out label-right of the control rather than as free-standing widgets:
/// four sliders and a chip row with no alignment reads as a pile of settings,
/// and the panel is looked at while the other hand is on the canvas.
fn controls(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    // ── Mode ────────────────────────────────────────────────────────────
    // Direct sits with the brush modes rather than in a section of its own.
    // They are alternatives — either the pointer paints or the slider sets —
    // and putting them side by side makes that a single choice instead of two
    // controls that have to be reconciled.
    ui.horizontal_wrapped(|ui| {
        let direct = state.session.weight_paint_settings.direct;
        if ui
            .selectable_label(direct, "Direct")
            .on_hover_text("Type a weight for the selected vertices instead of painting")
            .clicked()
        {
            state.session.weight_paint_settings.direct = true;
        }
        for mode in BrushMode::ALL {
            let selected = !direct && state.session.weight_paint_settings.mode == mode;
            let hint = match mode {
                BrushMode::Add => "Raise toward Weight, never past it",
                BrushMode::Subtract => "Lower toward zero; Weight is the rate",
                BrushMode::Replace => "Set to exactly Weight, from either side",
                BrushMode::Smooth => "Average with neighbouring vertices",
            };
            if ui
                .selectable_label(selected, mode.label())
                .on_hover_text(hint)
                .clicked()
            {
                state.session.weight_paint_settings.direct = false;
                state.session.weight_paint_settings.mode = mode;
            }
        }
    });
    ui.add_space(6.0);

    // ── Weight / Size / Feather ─────────────────────────────────────────
    let direct = state.session.weight_paint_settings.direct;
    let selected_vertices = state.session.selected_vertices.clone();
    let bone = state.session.active_bone();

    let weight_changed = ui
        .add(
            egui::Slider::new(&mut state.session.weight_paint_settings.strength, 0.0..=1.0)
                .text("Weight")
                .fixed_decimals(2),
        )
        .on_hover_text(if direct {
            "The weight written to the selected vertices"
        } else {
            "The weight the brush drives toward — not an amount added"
        })
        .changed();

    // Size and feather describe a brush, so they go quiet when there is no
    // brush. Hiding them instead would make the panel jump height every time
    // the mode changes.
    ui.add_enabled_ui(!direct, |ui| {
        ui.add(
            egui::Slider::new(&mut state.session.weight_paint_settings.radius, 4.0..=400.0)
                .text("Size"),
        );
        ui.add(
            egui::Slider::new(&mut state.session.weight_paint_settings.feather, 0.0..=1.0)
                .text("Feather")
                .fixed_decimals(2),
        )
        .on_hover_text(
            "How much of the radius is gradient.\n\
             0 stamps a hard edge, 1 fades from the centre.",
        );
    });

    if direct
        && weight_changed
        && let Some(bone) = bone
    {
        let value = state.session.weight_paint_settings.strength;
        apply_direct(state, target, mesh, &selected_vertices, bone, value);
    }

    // ── Actions ─────────────────────────────────────────────────────────
    ui.add_space(6.0);
    actions(ui, state, target, mesh);

    // ── What the canvas shows ───────────────────────────────────────────
    ui.add_space(6.0);
    let settings = &mut state.session.weight_paint_settings;
    ui.horizontal(|ui| {
        ui.checkbox(&mut settings.show_overlay, "Overlay")
            .on_hover_text("Shade the mesh by the selected bone's influence");
        ui.checkbox(&mut settings.show_pies, "Pies")
            .on_hover_text("Show each vertex's whole influence split, in bone colours");
        ui.checkbox(&mut settings.show_selected_only, "Selected")
            .on_hover_text("Only mark the vertices picked in mesh edit mode");
    });

    // One line, and only the line that applies right now. The panel used to
    // carry a paragraph of instructions and a vertex-count heading that said
    // nothing when the count was zero.
    ui.add_space(4.0);
    let hint = if direct {
        match (bone.is_some(), selected_vertices.len()) {
            (false, _) => "Select a bone to set its weight".to_string(),
            (true, 0) => "Pick vertices in mesh edit mode".to_string(),
            (true, n) => format!("Weight applies to {n} selected vertices"),
        }
    } else {
        "Click a bone to aim the brush, then drag over the mesh".to_string()
    };
    ui.label(
        egui::RichText::new(hint)
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
    );
}

/// Which bones actually influence this mesh, strongest first.
///
/// Spine calls this the bound-bones list and it is the thing that was missing
/// most: without it there is no way to see that a stray bone picked up 2% of a
/// mesh three sessions ago, and no way to lock, swap or remove one.
fn bound_bones(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    let mut totals: Vec<(BoneId, f32)> = Vec::new();
    for vertex in &mesh.weights {
        for w in vertex {
            match totals.iter_mut().find(|(bone, _)| *bone == w.bone) {
                Some((_, total)) => *total += w.weight,
                None => totals.push((w.bone, w.weight)),
            }
        }
    }
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));

    ui.label(
        egui::RichText::new(format!("BOUND BONES  {}", totals.len()))
            .size(10.5)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    if totals.is_empty() {
        ui.label(
            egui::RichText::new("None yet — paint, or use Auto-weight")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    let vertices = mesh.weights.len().max(1) as f32;
    let active = state.session.active_bone();
    let mut select: Option<BoneId> = None;
    let mut toggle_lock: Option<BoneId> = None;
    let mut remove: Option<BoneId> = None;

    for (bone, total) in &totals {
        let Some(name) = state.doc.skeleton.bones.get(*bone).map(|b| b.name.clone()) else {
            continue;
        };
        let locked = state.session.weight_paint_settings.locked.contains(bone);
        ui.horizontal(|ui| {
            let lock_icon = if locked {
                crate::ui::icons::LOCKED
            } else {
                crate::ui::icons::UNLOCKED
            };
            if ui
                .selectable_label(locked, lock_icon)
                .on_hover_text("Hold this bone's weight while painting others")
                .clicked()
            {
                toggle_lock = Some(*bone);
            }
            if ui
                .selectable_label(active == Some(*bone), &name)
                .on_hover_text("Aim the brush at this bone")
                .clicked()
            {
                select = Some(*bone);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(crate::ui::icons::DELETE)
                    .on_hover_text("Remove this bone's influence from the mesh")
                    .clicked()
                {
                    remove = Some(*bone);
                }
                // Mean influence across the mesh: enough to spot a bone holding
                // 1% of everything, which is the shape a stray binding takes.
                ui.label(
                    egui::RichText::new(format!("{:.0}%", total / vertices * 100.0))
                        .size(10.5)
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
    }

    if let Some(bone) = select {
        state.session.select_bone(Some(bone));
    }
    if let Some(bone) = toggle_lock {
        let locked = &mut state.session.weight_paint_settings.locked;
        match locked.iter().position(|b| *b == bone) {
            Some(index) => {
                locked.remove(index);
            }
            None => locked.push(bone),
        }
    }
    if let Some(bone) = remove {
        let weights = remove_bone(&mesh.weights, bone);
        dispatch(state, target, weights, "Remove Bone Weights");
    }

    // Swap needs exactly two bones named, and the selection is the natural way
    // to name them.
    let selected = state.session.selected_bones.clone();
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(selected.len() == 2, egui::Button::new("Swap").small())
            .on_hover_text(
                "Exchange two bones' weights across the whole mesh.\n\
                 Select exactly two bones first.",
            )
            .clicked()
        {
            let weights = swap_bones(&mesh.weights, selected[0], selected[1]);
            dispatch(state, target, weights, "Swap Bone Weights");
        }
        if ui
            .add_enabled(!totals.is_empty(), egui::Button::new("Copy").small())
            .on_hover_text("Copy this mesh's weights")
            .clicked()
        {
            state.session.weight_clipboard = Some(mesh.weights.clone());
            state.session.set_status("Weights copied");
        }
        let pastable = state
            .session
            .weight_clipboard
            .as_ref()
            .is_some_and(|w| w.len() == mesh.weights.len().max(mesh.setup_vertices.len()));
        if ui
            .add_enabled(pastable, egui::Button::new("Paste").small())
            .on_hover_text(
                "Paste weights onto this mesh.\n\
                 Only between meshes with the same vertex count.",
            )
            .clicked()
            && let Some(weights) = state.session.weight_clipboard.clone()
        {
            dispatch(state, target, weights, "Paste Weights");
        }
    });
}

fn apply_direct(
    state: &mut AppState,
    target: &Target,
    mesh: &MeshAttachment,
    selected: &[usize],
    bone: BoneId,
    value: f32,
) {
    let mut weights = mesh.weights.clone();
    weights.resize(mesh.setup_vertices.len(), Vec::new());
    let locked = state.session.weight_paint_settings.locked.clone();
    for index in selected {
        if let Some(vertex) = weights.get_mut(*index) {
            set_weight(vertex, bone, value, &locked);
        }
    }
    dispatch(state, target, weights, "Set Vertex Weights");
}

/// The whole-mesh operations, in two rows.
///
/// Grouped by what they touch rather than by how often they are used: the first
/// row assigns weights, the second cleans up and maintains them. A flat grid of
/// eight buttons reads as eight equally likely things to press, which is how the
/// panel got a reputation for being unreadable.
fn actions(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    let selected_bones = state.session.selected_bones.clone();
    let selected_vertices = state.session.selected_vertices.clone();

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Auto")
            .on_hover_text(
                "Weight every vertex to nearby bones, measured across the mesh surface.
                 A starting point to refine, not a finished rig.",
            )
            .clicked()
        {
            auto_weight_mesh(state, target, mesh, None);
        }
        if ui
            .add_enabled(!selected_bones.is_empty(), egui::Button::new("Bind"))
            .on_hover_text(
                "Same, but only from the bones you have selected.
                 Select bones first.",
            )
            .clicked()
        {
            auto_weight_mesh(state, target, mesh, Some(&selected_bones));
        }
        if ui
            .button("Smooth")
            .on_hover_text(
                "Average each vertex with its neighbours.
                 Applies to the selected vertices, or the whole mesh if none.",
            )
            .clicked()
        {
            smooth_all(state, target, mesh, &selected_vertices);
        }
        if ui
            .add_enabled(
                state.session.selected_slots.len() > 1,
                egui::Button::new("Weld"),
            )
            .on_hover_text(
                "Give vertices that sit on top of each other the same weights.
                 Select two or more slots whose meshes meet at a seam.",
            )
            .clicked()
        {
            weld_selected(state);
        }
    });

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Prune")
            .on_hover_text("Drop the weakest influences, by the limits below")
            .clicked()
        {
            let settings = &state.session.weight_paint_settings;
            let (max_bones, threshold) = (settings.max_bones, settings.prune_threshold);
            let weights = mesh
                .weights
                .iter()
                .map(|vertex| {
                    let mut vertex = vertex.clone();
                    prune(&mut vertex, max_bones, threshold);
                    vertex
                })
                .collect();
            dispatch(state, target, weights, "Prune Weights");
        }
        if ui
            .button("Update")
            .on_hover_text(
                "Recapture the bind pose from the current setup.
                 Do this after moving a bone that already holds weight.",
            )
            .clicked()
        {
            dispatch(state, target, mesh.weights.clone(), "Update Bindings");
            state.session.set_status("Bindings updated");
        }

        let settings = &mut state.session.weight_paint_settings;
        let mut bones = settings.max_bones as f32;
        if ui
            .add(egui::DragValue::new(&mut bones).range(1.0..=8.0).speed(0.1))
            .on_hover_text("Prune: most bones allowed on one vertex")
            .changed()
        {
            settings.max_bones = bones.round() as usize;
        }
        ui.add(
            egui::DragValue::new(&mut settings.prune_threshold)
                .range(0.0..=0.2)
                .speed(0.002)
                .fixed_decimals(3),
        )
        .on_hover_text("Prune: weights below this are dropped");
    });
}

/// Smooth the selection, or the whole mesh when nothing is selected.
fn smooth_all(state: &mut AppState, target: &Target, mesh: &MeshAttachment, selected: &[usize]) {
    let Some(bone) = state.session.active_bone() else {
        state
            .session
            .set_status("Select a bone to smooth its weights");
        return;
    };
    // Zero distance means full effect; anything outside the selection is put
    // past the radius so the same brush code can express "these and no others".
    let distances: Vec<f32> = (0..mesh.setup_vertices.len())
        .map(|i| {
            if selected.is_empty() || selected.contains(&i) {
                0.0
            } else {
                f32::INFINITY
            }
        })
        .collect();
    let locked = state.session.weight_paint_settings.locked.clone();
    let weights = weight_cmds::brush(
        mesh,
        bone,
        BrushMode::Smooth,
        &distances,
        1.0,
        1.0,
        1.0,
        &locked,
    );
    dispatch(state, target, weights, "Smooth Weights");
}

/// Weld every selected slot's mesh together along the seams they share.
fn weld_selected(state: &mut AppState) {
    let slots = state.session.selected_slots.clone();
    let mut targets: Vec<Target> = Vec::new();
    let mut inputs: Vec<(Vec<glam::Vec2>, Vec<Vec<VertexWeight>>)> = Vec::new();

    for slot in slots {
        let Some(name) = state
            .doc
            .skeleton
            .slots
            .get(slot)
            .and_then(|s| s.attachment.clone())
        else {
            continue;
        };
        let Some(skin) = owning_skin(&state.doc, state.session.active_skin, slot, &name) else {
            continue;
        };
        let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot, &name) else {
            continue;
        };
        // Compared in world space: each mesh's vertices live in its own slot
        // bone's frame, and two seams that touch on screen have quite different
        // local coordinates.
        let bone = state.doc.skeleton.slots[slot].bone;
        let Some(world) = state.pose.worlds.get(bone).copied() else {
            continue;
        };
        let positions: Vec<glam::Vec2> = mesh
            .setup_vertices
            .iter()
            .map(|v| world.transform_point(*v))
            .collect();
        let mut weights = mesh.weights.clone();
        weights.resize(positions.len(), Vec::new());
        inputs.push((positions, weights));
        targets.push(Target { skin, slot, name });
    }

    if inputs.len() < 2 {
        state
            .session
            .set_status("Select two or more slots with meshes to weld");
        return;
    }

    // A pixel of slack. Seams are authored by eye or by tracing, so exact
    // equality would match almost nothing.
    let welded = weight_cmds::weld(&inputs, 1.0);
    for (target, weights) in targets.iter().zip(welded) {
        dispatch(state, target, weights, "Weld Weights");
    }
    state
        .session
        .set_status(format!("Welded {} meshes", targets.len()));
}

/// Auto-weight this mesh, against the whole rig or just the bones given.
fn auto_weight_mesh(
    state: &mut AppState,
    target: &Target,
    mesh: &MeshAttachment,
    only: Option<&[BoneId]>,
) {
    // Bones as segments in the mesh's local space, which is where its vertices
    // live: distance from a vertex to a *bone* means distance to its segment,
    // not to its origin, or long bones would only pull at one end.
    let slot_bone = state.doc.skeleton.slots[target.slot].bone;
    let Some(bone_world) = state.pose.worlds.get(slot_bone).copied() else {
        return;
    };
    let Some(inverse) = bone_world.invert() else {
        return;
    };
    let bones: Vec<_> = state
        .doc
        .skeleton
        .update_order
        .iter()
        .filter(|id| only.is_none_or(|only| only.contains(id)))
        .map(|&id| {
            let start = state.pose.world_position(id);
            let end = state.pose.world_tip(&state.doc.skeleton, id);
            (
                id,
                inverse.transform_point(start),
                inverse.transform_point(end),
            )
        })
        .collect();
    if bones.is_empty() {
        return;
    }

    let label = if only.is_some() {
        "Bind Bones"
    } else {
        "Auto-weight"
    };
    let weights = auto_weight(mesh, &bones, 2.0);
    let vertices = weights.len();
    if dispatch(state, target, weights, label) {
        state
            .session
            .set_status(format!("Auto-weighted {vertices} vertices"));
    }
}

/// Push a new weight table through the history.
///
/// `PaintWeights` merges consecutive strokes into one undo step, which is right
/// for a drag and wrong for a button — two Prunes in a row are two decisions.
fn dispatch(
    state: &mut AppState,
    target: &Target,
    weights: Vec<Vec<VertexWeight>>,
    label: &'static str,
) -> bool {
    state.dispatch(Box::new(SetWeights::new(
        target.skin,
        target.slot,
        target.name.clone(),
        weights,
        label,
    )))
}
