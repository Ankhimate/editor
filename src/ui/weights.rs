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
use ankhimate_core::attachment::{Attachment, MeshAttachment, VertexWeight};
use ankhimate_core::ids::{BoneId, SkinId, SlotId};
use ankhimate_document::commands::attachment_cmds::owning_skin;
use ankhimate_document::commands::weight_cmds::{
    self, BrushMode, SetWeights, auto_weight, prune, remove_bone, set_weight, swap_bones,
};
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
    // These are one choice, not navigation. The old selectable-label row read
    // as five tabs and fought the panel's actual Properties/Weights tabs.
    let direct = state.session.weight_paint_settings.direct;
    let selected = if direct {
        "Direct"
    } else {
        state.session.weight_paint_settings.mode.label()
    };
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("weight_input_mode")
            .selected_text(selected)
            .width(150.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(direct, "Direct")
                    .on_hover_text("Type a weight for the selected vertices instead of painting")
                    .clicked()
                {
                    state.session.weight_paint_settings.direct = true;
                    ui.close();
                }
                ui.separator();
                for mode in BrushMode::ALL {
                    let active = !state.session.weight_paint_settings.direct
                        && state.session.weight_paint_settings.mode == mode;
                    let hint = match mode {
                        BrushMode::Add => "Raise toward Weight, never past it",
                        BrushMode::Subtract => "Lower toward zero; Weight is the rate",
                        BrushMode::Replace => "Set to exactly Weight, from either side",
                        BrushMode::Smooth => "Average with neighbouring vertices",
                    };
                    if ui
                        .selectable_label(active, mode.label())
                        .on_hover_text(hint)
                        .clicked()
                    {
                        state.session.weight_paint_settings.direct = false;
                        state.session.weight_paint_settings.mode = mode;
                        ui.close();
                    }
                }
            });
        ui.label("Mode");
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

/// Which bones influence this mesh, strongest first.
///
/// A framed list that takes the rest of the panel, with the operations that act
/// on a bone along its bottom edge. Loose rows followed by loose buttons gave no
/// hint that the buttons acted on the rows above; a box with its own footer says
/// so without a word of explanation.
///
/// Without this list there is no way to see that a stray bone picked up 2% of a
/// mesh three sessions ago, and nothing to aim Swap or Remove at.
fn bound_bones(ui: &mut egui::Ui, state: &mut AppState, target: &Target, mesh: &MeshAttachment) {
    // Ranked in core, not here: the rank is what the swatch colour is keyed on,
    // and the canvas pies and paint overlay key on the same one. Ranking twice
    // is how the list and the mesh end up disagreeing about which bone is green.
    let totals: Vec<(BoneId, f32)> = mesh.bound_bones();

    let mut select: Option<BoneId> = None;
    let mut toggle_lock: Option<BoneId> = None;
    let mut action: Option<Action> = None;

    // ── Header ──────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Bones:").size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let pastable = state
                .session
                .weight_clipboard
                .as_ref()
                .is_some_and(|w| w.len() == mesh.setup_vertices.len());
            if ui
                .add_enabled(
                    pastable,
                    egui::Button::new(crate::ui::icons::PASTE).small(),
                )
                .on_hover_text(
                    "Paste weights onto this mesh.\nOnly between meshes with the same vertex count.",
                )
                .clicked()
            {
                action = Some(Action::Paste);
            }
            if ui
                .add_enabled(
                    !totals.is_empty(),
                    egui::Button::new(crate::ui::icons::DUPLICATE).small(),
                )
                .on_hover_text("Copy this mesh's weights")
                .clicked()
            {
                action = Some(Action::Copy);
            }
        });
    });

    // ── The list ────────────────────────────────────────────────────────
    // Reserves the rest of the panel minus the footer, so the box does not
    // resize as bones come and go — a list that grows under the pointer moves
    // the row you were about to click.
    let footer = 30.0;
    let height = (ui.available_height() - footer).clamp(80.0, 400.0);
    egui::Frame::NONE
        .fill(ui.visuals().extreme_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(4)
        .inner_margin(egui::Margin::same(2))
        .show(ui, |ui| {
            ui.set_min_height(height);
            ui.set_width(ui.available_width());
            if totals.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("No bones bound")
                            .size(10.5)
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.label(
                        egui::RichText::new("Select bones and press Bind")
                            .size(10.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("bound_bones")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let vertices = mesh.weights.len().max(1) as f32;
                    let active = state.session.active_bone();
                    for (rank, (bone, total)) in totals.iter().enumerate() {
                        let Some(info) = state.doc.skeleton.bones.get(*bone) else {
                            continue;
                        };
                        let name = info.name.clone();
                        // From the bone's rank on *this mesh*, which is what the
                        // vertex pies and the paint overlay colour by too. Not
                        // `group_color`: that answers "which limb", so a limb
                        // given one colour makes every row here the same swatch,
                        // and the rows exist to be told apart from each other.
                        // Every row is a bound bone, so every row has a rank —
                        // `color_for_rank` only returns `None` for bones this
                        // mesh does not use, which cannot appear in this list.
                        let Some(color) = crate::ui::canvas::renderer::color_for_rank(Some(rank))
                        else {
                            continue;
                        };
                        let locked = state.session.weight_paint_settings.locked.contains(bone);
                        match bone_row(
                            ui,
                            &name,
                            color,
                            total / vertices,
                            active == Some(*bone),
                            locked,
                        ) {
                            RowClick::Select => select = Some(*bone),
                            RowClick::ToggleLock => toggle_lock = Some(*bone),
                            RowClick::None => {}
                        }
                    }
                });
        });

    // ── Footer ──────────────────────────────────────────────────────────
    let selected_bones = state.session.selected_bones.clone();
    let active = state.session.active_bone();
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!selected_bones.is_empty(), egui::Button::new("Bind"))
            .on_hover_text(
                "Add the selected bones to this mesh, computing starting weights from them.",
            )
            .clicked()
        {
            action = Some(Action::Bind);
        }
        if ui
            .add_enabled(selected_bones.len() == 2, egui::Button::new("Swap"))
            .on_hover_text(
                "Exchange two bones' weights across the whole mesh.\nSelect exactly two bones first.",
            )
            .clicked()
        {
            action = Some(Action::Swap);
        }
        if ui
            .add_enabled(active.is_some(), egui::Button::new("Remove"))
            .on_hover_text("Remove the highlighted bone's influence from this mesh")
            .clicked()
        {
            action = Some(Action::Remove);
        }
    });

    // ── Apply, after the layout so nothing mutates mid-borrow ───────────
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
    match action {
        Some(Action::Copy) => {
            state.session.weight_clipboard = Some(mesh.weights.clone());
            state.session.set_status("Weights copied");
        }
        Some(Action::Paste) => {
            if let Some(weights) = state.session.weight_clipboard.clone() {
                dispatch(state, target, weights, "Paste Weights");
            }
        }
        Some(Action::Bind) => auto_weight_mesh(state, target, mesh, &selected_bones, &[]),
        Some(Action::Swap) => {
            let weights = swap_bones(&mesh.weights, selected_bones[0], selected_bones[1]);
            dispatch(state, target, weights, "Swap Bone Weights");
        }
        Some(Action::Remove) => {
            if let Some(bone) = active {
                let weights = remove_bone(&mesh.weights, bone);
                dispatch(state, target, weights, "Remove Bone Weights");
            }
        }
        None => {}
    }
}

/// What the header or footer asked for this frame.
enum Action {
    Copy,
    Paste,
    Bind,
    Swap,
    Remove,
}

enum RowClick {
    None,
    Select,
    ToggleLock,
}

/// One row of the bone list: swatch, name, share.
///
/// Painted rather than assembled from widgets so the highlight spans the full
/// width. A `selectable_label` highlights only its own text, which on a row with
/// a percentage on the far right leaves the selection looking like it covers
/// half of what it covers.
fn bone_row(
    ui: &mut egui::Ui,
    name: &str,
    color: [f32; 4],
    share: f32,
    selected: bool,
    locked: bool,
) -> RowClick {
    let height = 20.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let visuals = ui.visuals().clone();
    if selected {
        ui.painter().rect_filled(rect, 3, visuals.selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3, visuals.faint_bg_color);
    }

    // The bone's own colour, as in the tree — a swatch ties the row to the thing
    // on the canvas without the name having to be read.
    let swatch = egui::Rect::from_center_size(
        egui::pos2(rect.min.x + 12.0, rect.center().y),
        egui::vec2(11.0, 11.0),
    );
    ui.painter().rect_filled(
        swatch,
        2,
        egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        ),
    );

    // On a selected row the fill is the theme's primary, which is a light
    // accent in the shipped themes — `strong_text_color()` is near-white and
    // vanished into it. `selection.stroke.color` is the theme's own answer to
    // "what reads on top of primary", so it stays legible whatever primary is.
    let text_color = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0),
        text_color,
    );

    // Mean influence across the mesh: enough to spot a bone holding 1% of
    // everything, which is the shape a stray binding takes. Left blank below
    // half a percent rather than shown as "0%", which reads as a measurement.
    if share >= 0.005 {
        ui.painter().text(
            egui::pos2(rect.max.x - 24.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.0}%", share * 100.0),
            egui::FontId::proportional(11.0),
            if selected {
                text_color
            } else {
                visuals.weak_text_color()
            },
        );
    }

    // The lock, on the right edge. Drawn only when locked or hovered: a column
    // of open padlocks down every row is noise for a setting almost nobody
    // touches.
    let mut click = if response.clicked() {
        RowClick::Select
    } else {
        RowClick::None
    };
    if locked || response.hovered() {
        let lock = egui::Rect::from_center_size(
            egui::pos2(rect.max.x - 10.0, rect.center().y),
            egui::vec2(16.0, height),
        );
        let lock_response = ui.interact(lock, response.id.with("lock"), egui::Sense::click());
        ui.painter().text(
            lock.center(),
            egui::Align2::CENTER_CENTER,
            if locked {
                crate::ui::icons::LOCKED
            } else {
                crate::ui::icons::UNLOCKED
            },
            egui::FontId::proportional(11.0),
            // Same reason as the name: on a selected row the dimmed colours are
            // the ones that disappear into the accent fill.
            match (locked, selected) {
                (true, _) => visuals.warn_fg_color,
                (false, true) => text_color,
                (false, false) => visuals.weak_text_color(),
            },
        );
        if lock_response
            .on_hover_text("Hold this bone's weight while painting others")
            .clicked()
        {
            click = RowClick::ToggleLock;
        }
    }
    click
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
        // One button, scoped by whatever is selected — which is what the
        // selection is for. A separate "Bind" that differed only in reading the
        // bone selection was two buttons for one idea.
        let scope = match (selected_bones.is_empty(), selected_vertices.is_empty()) {
            (true, true) => "every bone and vertex".to_string(),
            (false, true) => format!("{} selected bones", selected_bones.len()),
            (true, false) => format!("{} selected vertices", selected_vertices.len()),
            (false, false) => format!(
                "{} bones on {} vertices",
                selected_bones.len(),
                selected_vertices.len()
            ),
        };
        if ui
            .button("Auto")
            .on_hover_text(format!(
                "Compute weights automatically, measuring across the mesh surface.
                 Applies to {scope}.
                 A starting point to refine, not a finished rig."
            ))
            .clicked()
        {
            auto_weight_mesh(
                state,
                target,
                mesh,
                &selected_bones,
                &selected_vertices,
            );
        }
        if ui
            .button("Smooth")
            .on_hover_text(
                "Blend the weights of the selected vertices, or all vertices if                  none are selected.",
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
                "Set this mesh's weights to match another mesh, wherever their                  vertices coincide.
                 The **last** slot selected is the source; the rest are changed.",
            )
            .clicked()
        {
            weld_selected(state);
        }
    });

    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Prune")
            .on_hover_text(
                "Remove weights below the threshold, from the selected vertices                  or all of them if none are selected.",
            )
            .clicked()
        {
            let settings = &state.session.weight_paint_settings;
            let (max_bones, threshold) = (settings.max_bones, settings.prune_threshold);
            let weights = mesh
                .weights
                .iter()
                .enumerate()
                .map(|(i, vertex)| {
                    let mut vertex = vertex.clone();
                    if selected_vertices.is_empty() || selected_vertices.contains(&i) {
                        prune(&mut vertex, max_bones, threshold);
                    }
                    vertex
                })
                .collect();
            dispatch(state, target, weights, "Prune Weights");
        }
        if ui
            .button("Update")
            .on_hover_text(
                "Store the current mesh vertices as the bind positions.
                 Do this after moving a bone that already holds weight, or the                  art drifts away from it.",
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
            .on_hover_text(
                "Flags a vertex in the viewport when more than this many bones                  influence it. Also the ceiling Prune trims to.",
            )
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
        .on_hover_text("Prune: weights below this are removed");
    });

    // The count, not just the rings. A ring is only found by looking at the
    // right part of the mesh, and the whole point of a budget warning is that it
    // reaches you when you were not looking for it.
    let limit = state.session.weight_paint_settings.max_bones;
    let over = mesh
        .weights
        .iter()
        .filter(|w| weight_cmds::over_influenced(w, limit))
        .count();
    if over > 0 {
        ui.label(
            egui::RichText::new(format!(
                "{over} vertices exceed {limit} bones — Prune trims them"
            ))
            .size(10.5)
            .color(ui.visuals().warn_fg_color),
        );
    }
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

/// Copy the source mesh's weights onto every other selected slot's mesh.
///
/// The **last** slot selected is the source, matching every other multi-select
/// action in the editor, where the last click is the active one. It is left
/// untouched: one mesh is authority and the rest are brought to it, rather than
/// everything being averaged into a compromise that damages the mesh you had
/// already got right.
fn weld_selected(state: &mut AppState) {
    let slots = state.session.selected_slots.clone();
    let Some((&source_slot, target_slots)) = slots.split_last() else {
        return;
    };

    // World space: each mesh's vertices live in its own slot bone's frame, and
    // two seams that touch on screen have quite different local coordinates.
    let read = |state: &AppState,
                slot: SlotId|
     -> Option<(Target, Vec<glam::Vec2>, Vec<Vec<VertexWeight>>)> {
        let name = state.doc.skeleton.slots.get(slot)?.attachment.clone()?;
        let skin = owning_skin(&state.doc, state.session.active_skin, slot, &name)?;
        let Attachment::Mesh(mesh) = state.doc.skeleton.skins[skin].get(slot, &name)? else {
            return None;
        };
        let world = *state.pose.worlds.get(state.doc.skeleton.slots[slot].bone)?;
        let positions: Vec<glam::Vec2> = mesh
            .setup_vertices
            .iter()
            .map(|v| world.transform_point(*v))
            .collect();
        let mut weights = mesh.weights.clone();
        weights.resize(positions.len(), Vec::new());
        Some((Target { skin, slot, name }, positions, weights))
    };

    let Some((_, source_positions, source_weights)) = read(state, source_slot) else {
        state.session.set_status("The source slot has no mesh");
        return;
    };

    let mut welded = 0;
    for slot in target_slots.iter().copied() {
        let Some((target, positions, weights)) = read(state, slot) else {
            continue;
        };
        // A pixel of slack. Seams are authored by eye or by tracing, so exact
        // equality would match almost nothing.
        let new_weights = weight_cmds::weld_to_source(
            (&source_positions, &source_weights),
            (&positions, &weights),
            1.0,
        );
        dispatch(state, &target, new_weights, "Weld Weights");
        welded += 1;
    }
    state
        .session
        .set_status(format!("Welded {welded} meshes to the source"));
}

/// Auto-weight this mesh, scoped by whatever bones and vertices are selected.
/// Auto-weight this mesh, against the whole rig or just the bones given.
fn auto_weight_mesh(
    state: &mut AppState,
    target: &Target,
    mesh: &MeshAttachment,
    only_bones: &[BoneId],
    only_vertices: &[usize],
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
        .filter(|id| only_bones.is_empty() || only_bones.contains(id))
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

    let weights = auto_weight(mesh, &bones, 2.0, only_vertices);
    let touched = if only_vertices.is_empty() {
        weights.len()
    } else {
        only_vertices.len()
    };
    if dispatch(state, target, weights, "Auto-weight") {
        state
            .session
            .set_status(format!("Auto-weighted {touched} vertices"));
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
