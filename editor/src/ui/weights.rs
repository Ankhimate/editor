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
    BrushMode, SetWeights, auto_weight, prune, remove_bone, set_weight, swap_bones,
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
    super::inspector::section_header(ui, crate::ui::icons::WEIGHT_PAINT, "Weight Paint");
    ui.add_space(4.0);

    let Some((target, mesh)) = target(state) else {
        ui.label(
            egui::RichText::new("Select a slot with a mesh attachment to paint weights.")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(8.0);
        return;
    };

    brush_controls(ui, state);
    ui.add_space(8.0);
    bound_bones(ui, state, &target, &mesh);
    ui.add_space(8.0);
    direct_entry(ui, state, &target, &mesh);
    ui.add_space(8.0);
    actions(ui, state, &target, &mesh);

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(
            "Click a bone on the canvas to aim the brush; drag over the mesh to paint.",
        )
        .size(10.5)
        .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(8.0);
}

fn brush_controls(ui: &mut egui::Ui, state: &mut AppState) {
    let settings = &mut state.session.weight_paint_settings;

    ui.horizontal_wrapped(|ui| {
        ui.add_space(8.0);
        for mode in BrushMode::ALL {
            let selected = settings.mode == mode;
            let hint = match mode {
                BrushMode::Add => "Raise toward Strength, never past it",
                BrushMode::Subtract => "Lower toward zero; Strength is the rate",
                BrushMode::Replace => "Set to exactly Strength, from either side",
                BrushMode::Smooth => "Average with neighbouring vertices",
            };
            if ui
                .selectable_label(selected, mode.label())
                .on_hover_text(hint)
                .clicked()
            {
                settings.mode = mode;
            }
        }
    });
    ui.add_space(4.0);

    ui.add(egui::Slider::new(&mut settings.radius, 4.0..=400.0).text("Size"));
    ui.add(
        egui::Slider::new(&mut settings.strength, 0.0..=1.0)
            .text("Strength")
            .fixed_decimals(2),
    )
    .on_hover_text("The weight the brush drives toward — not an amount added");
    ui.add(
        egui::Slider::new(&mut settings.feather, 0.0..=1.0)
            .text("Feather")
            .fixed_decimals(2),
    )
    .on_hover_text(
        "How much of the radius is gradient.\n\
         0 stamps a hard edge, 1 fades from the centre.",
    );

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut settings.show_overlay, "Overlay")
            .on_hover_text("Shade the mesh by the selected bone's influence");
        ui.checkbox(&mut settings.show_pies, "Pies")
            .on_hover_text("Show each vertex's whole influence split, in bone colours");
        ui.checkbox(&mut settings.show_selected_only, "Selected")
            .on_hover_text("Only mark the vertices picked in mesh edit mode");
    });
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

/// Exact weights for the vertices picked in mesh-edit mode.
///
/// The brush is a gesture and gestures are approximate. Some weights want to be
/// said outright — a vertex at the very end of a limb is 100% that bone, and
/// nudging a brush until it reads 1.00 is a silly way to say so.
fn direct_entry(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    let selected = state.session.selected_vertices.clone();
    ui.label(
        egui::RichText::new(format!("SELECTED VERTICES  {}", selected.len()))
            .size(10.5)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    if selected.is_empty() {
        ui.label(
            egui::RichText::new("Pick vertices in mesh edit mode to set weights exactly")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }
    let Some(bone) = state.session.active_bone() else {
        ui.label(
            egui::RichText::new("Select a bone to set its weight")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };

    // The mean across the selection, so one field can stand for many vertices.
    // Showing the first vertex's value instead would silently misreport the rest.
    let mean = selected
        .iter()
        .filter_map(|i| mesh.weights.get(*i))
        .map(|v| v.iter().find(|w| w.bone == bone).map_or(0.0, |w| w.weight))
        .sum::<f32>()
        / selected.len() as f32;

    let mut value = mean;
    let name = state
        .doc
        .skeleton
        .bones
        .get(bone)
        .map(|b| b.name.clone())
        .unwrap_or_default();
    let changed = ui
        .add(
            egui::Slider::new(&mut value, 0.0..=1.0)
                .text(&name)
                .fixed_decimals(3),
        )
        .changed();

    ui.horizontal(|ui| {
        let mut preset = None;
        for (label, v) in [("0%", 0.0), ("25%", 0.25), ("50%", 0.5), ("100%", 1.0)] {
            if ui.small_button(label).clicked() {
                preset = Some(v);
            }
        }
        if let Some(v) = preset {
            apply_direct(state, target, mesh, &selected, bone, v);
        }
    });

    if changed {
        apply_direct(state, target, mesh, &selected, bone, value);
    }
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

fn actions(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    ui.horizontal(|ui| {
        if ui
            .button("Auto-weight")
            .on_hover_text(
                "Bind every vertex to nearby bones by distance across the mesh surface.\n\
                 A starting point to refine, not a finished rig.",
            )
            .clicked()
        {
            auto_weight_mesh(state, target, mesh);
        }
        if ui
            .button("Prune")
            .on_hover_text("Drop the weakest influences on every vertex")
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
    });

    let settings = &mut state.session.weight_paint_settings;
    let mut bones = settings.max_bones as f32;
    if ui
        .add(egui::Slider::new(&mut bones, 1.0..=8.0).text("Max bones"))
        .on_hover_text("Ceiling on how many bones may influence one vertex")
        .changed()
    {
        settings.max_bones = bones.round() as usize;
    }
    ui.add(
        egui::Slider::new(&mut settings.prune_threshold, 0.0..=0.2)
            .text("Threshold")
            .fixed_decimals(3),
    )
    .on_hover_text("Weights below this are dropped by Prune");
}

/// Auto-weight this mesh against every bone in the rig.
fn auto_weight_mesh(state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
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

    let weights = auto_weight(mesh, &bones, 2.0);
    let vertices = weights.len();
    if dispatch(state, target, weights, "Auto-weight") {
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
