//! Persistent answer to: mode, clip, selection, tool, and time.

use crate::app_state::AppState;
use crate::session::{Selection, Tool, TransformTool, WorkMode};
use crate::theme::{IconRole, Theme};
use eframe::egui;

pub const HEIGHT: f32 = 40.0;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState, theme: &Theme) {
    egui::Frame::NONE
        .fill(crate::theme::hex_to_color(&theme.panel_fill))
        .stroke(egui::Stroke::new(1.0, theme.card_border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_height(26.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
            ui.horizontal_centered(|ui| {
                mode_segment(ui, state, theme);
                clip_segment(ui, state);
                selection_segment(ui, state);
                tool_segment(ui, state);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    frame_segment(ui, state, theme);
                    if state.session.work_mode == WorkMode::Animate {
                        let color = if state.session.auto_key {
                            theme.icon_color(IconRole::Destructive)
                        } else {
                            ui.visuals().weak_text_color()
                        };
                        ui.label(
                            egui::RichText::new(crate::ui::icons::RECORD)
                                .color(color)
                                .size(12.0),
                        )
                        .on_hover_text(if state.session.auto_key {
                            "Auto-key is on"
                        } else {
                            "Auto-key is off"
                        });
                    }
                });
            });
        });
}

fn mode_segment(ui: &mut egui::Ui, state: &mut AppState, theme: &Theme) {
    let current = state.session.work_mode;
    for (mode, label) in [(WorkMode::Setup, "Setup"), (WorkMode::Animate, "Animate")] {
        let selected = mode == current;
        let text = if selected {
            egui::RichText::new(label)
                .strong()
                .color(theme.on_primary())
        } else {
            egui::RichText::new(label).color(ui.visuals().weak_text_color())
        };
        let button = egui::Button::new(text)
            .selected(selected)
            .corner_radius(6)
            .min_size(egui::vec2(
                if mode == WorkMode::Setup { 52.0 } else { 62.0 },
                24.0,
            ));
        if ui.add(button).clicked() && !selected {
            state.set_work_mode(mode);
        }
    }
}

fn clip_segment(ui: &mut egui::Ui, state: &mut AppState) {
    let mut clips: Vec<_> = state
        .doc
        .animations
        .iter()
        .map(|(id, animation)| (id, animation.name.clone(), animation.duration))
        .collect();
    clips.sort_by_key(|(_, name, _)| name.to_lowercase());
    let active_name = state
        .session
        .active_animation
        .and_then(|id| state.doc.animations.get(id))
        .map(|animation| animation.name.as_str())
        .unwrap_or("No clip");

    ui.label(egui::RichText::new(crate::ui::icons::ANIMATIONS).color(
        crate::theme::current_icon_color(ui.ctx(), IconRole::Animation),
    ));
    egui::ComboBox::from_id_salt("context_clip")
        .selected_text(active_name)
        .width(120.0)
        .show_ui(ui, |ui| {
            if clips.is_empty() {
                ui.label(egui::RichText::new("No animations yet").weak());
            }
            for (id, name, duration) in clips {
                if ui
                    .selectable_label(state.session.active_animation == Some(id), name)
                    .clicked()
                {
                    state.session.active_animation = Some(id);
                    if state.session.playhead > duration {
                        state.set_playhead(duration);
                    }
                }
            }
        });
}

fn selection_segment(ui: &mut egui::Ui, state: &mut AppState) {
    let (icon, label, _) = selection_identity(state).unwrap_or((
        crate::ui::icons::NOTHING_SELECTED,
        "Nothing selected".to_string(),
        IconRole::Neutral,
    ));
    let response = ui
        .add(
            egui::Button::new(crate::ui::semantic_icon_label(ui, icon, &label))
                .frame(false)
                .corner_radius(6),
        )
        .on_hover_text("Reveal the selection in the hierarchy");
    if response.clicked() && state.session.selection.is_some() {
        state.session.reveal_selection = true;
    }
}

fn selection_identity(state: &AppState) -> Option<(&'static str, String, IconRole)> {
    match state.session.selection.as_ref()? {
        Selection::Bone(id) => state
            .doc
            .skeleton
            .bones
            .get(*id)
            .map(|bone| (crate::ui::icons::BONE, bone.name.clone(), IconRole::Rig)),
        Selection::Slot(id) => state.doc.skeleton.slots.get(*id).map(|slot| {
            (
                crate::ui::icons::SLOT,
                slot.name.clone(),
                IconRole::Attachment,
            )
        }),
        Selection::Attachment { name, .. } => Some((
            crate::ui::icons::ATTACHMENT,
            name.clone(),
            IconRole::Attachment,
        )),
        Selection::Constraint(id) => state.doc.skeleton.constraints.get(*id).map(|constraint| {
            (
                crate::ui::icons::CONSTRAINT,
                constraint.name().to_string(),
                IconRole::Constraint,
            )
        }),
    }
}

fn tool_segment(ui: &mut egui::Ui, state: &mut AppState) {
    let (icon, label, _) = tool_identity(state);
    ui.menu_button(crate::ui::semantic_icon_label(ui, icon, label), |ui| {
        if ui.button("Select").clicked() {
            state.session.tool = Tool::Select;
            ui.close();
        }
        ui.separator();
        for (tool, label) in [
            (TransformTool::Translate, "Translate"),
            (TransformTool::Rotate, "Rotate"),
            (TransformTool::Scale, "Scale"),
            (TransformTool::Shear, "Shear"),
        ] {
            if ui
                .selectable_label(
                    state.session.tool == Tool::Select
                        && state.session.active_transform_tool == tool,
                    label,
                )
                .clicked()
            {
                state.session.tool = Tool::Select;
                state.session.active_transform_tool = tool;
                ui.close();
            }
        }
        ui.separator();
        let setup = state.session.can_edit_structure();
        if ui
            .add_enabled(setup, egui::Button::new("Create bone"))
            .on_disabled_hover_text("Setup mode only")
            .clicked()
        {
            state.session.tool = Tool::CreateBone;
            ui.close();
        }
        if ui
            .add_enabled(setup, egui::Button::new("Weight paint"))
            .on_disabled_hover_text("Setup mode only")
            .clicked()
        {
            state.session.tool = Tool::WeightPaint;
            ui.close();
        }
    });
}

fn tool_identity(state: &AppState) -> (&'static str, &'static str, IconRole) {
    match state.session.tool {
        Tool::CreateBone => (crate::ui::icons::CREATE_BONE, "Create bone", IconRole::Rig),
        Tool::WeightPaint => (
            crate::ui::icons::WEIGHT_PAINT,
            "Weight paint",
            IconRole::Mesh,
        ),
        Tool::Select => match state.session.active_transform_tool {
            TransformTool::Translate => (
                crate::ui::icons::TOOL_TRANSLATE,
                "Translate",
                IconRole::Translate,
            ),
            TransformTool::Rotate => (crate::ui::icons::TOOL_ROTATE, "Rotate", IconRole::Rotate),
            TransformTool::Scale => (crate::ui::icons::TOOL_SCALE, "Scale", IconRole::Scale),
            TransformTool::Shear => (crate::ui::icons::TOOL_SHEAR, "Shear", IconRole::Shear),
        },
    }
}

fn frame_segment(ui: &mut egui::Ui, state: &mut AppState, theme: &Theme) {
    let fps = state.doc.meta.fps.max(1);
    let mut frame = (state.session.playhead * fps as f32).round() as i64;
    if ui
        .add(
            egui::DragValue::new(&mut frame)
                .prefix("f ")
                .speed(1.0)
                .range(0..=i64::MAX),
        )
        .on_hover_text(format!("Current frame at {fps} fps"))
        .changed()
    {
        state.set_playhead(frame as f32 / fps as f32);
    }
    ui.label(
        egui::RichText::new(crate::ui::icons::TIME).color(theme.icon_color(IconRole::Animation)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ribbon_names_the_authoritative_selection() {
        let mut state = AppState::default();
        let bone = state.doc.skeleton.add_bone(ankhimate_core::skeleton::Bone {
            name: "hand.L".into(),
            parent: None,
            length: 20.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: ankhimate_core::skeleton::Bone::default_color(),
        });
        state.session.selection = Some(Selection::Bone(bone));
        let (_, label, role) = selection_identity(&state).unwrap();
        assert_eq!(label, "hand.L");
        assert_eq!(role, IconRole::Rig);
    }
}
