//! The constraint pane.
//!
//! The hierarchy lists the constraints acting on each bone, which answers "why
//! is this bone moving on its own". This answers the other two questions: what
//! constraints exist at all, and — the one nothing else could answer — **in what
//! order do they solve**.
//!
//! Order is not cosmetic. Spineboy's leg IK runs before its foot IK, and swapping
//! them aims each foot against a leg that has not moved yet. That is a rig bug
//! with no visible cause unless the order is something you can see and change.

use crate::app_state::AppState;
use crate::commands::constraint_cmds::RemoveConstraint;
use crate::session::Selection;
use ankhimate_core::constraints::Constraint;
use ankhimate_core::ids::ConstraintId;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let order: Vec<ConstraintId> = state.doc.skeleton.constraint_order.clone();
    let rows: Vec<(ConstraintId, String, &'static str, &'static str, String)> = order
        .iter()
        .filter_map(|id| {
            let c = state.doc.skeleton.constraints.get(*id)?;
            let (icon, kind) = glyph(c);
            Some((*id, c.name().to_string(), icon, kind, summary(state, c)))
        })
        .collect();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Solve order").strong().size(11.0));
        ui.label(
            egui::RichText::new("top first")
                .size(10.0)
                .color(ui.visuals().weak_text_color()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("{}", rows.len()))
                    .size(10.5)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
    ui.separator();

    if rows.is_empty() {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No constraints")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new("Add IK, transform, physics or path from a bone's properties")
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    }

    let setup = state.session.can_edit_structure();
    let mut swap: Option<(usize, usize)> = None;
    let mut remove: Option<ConstraintId> = None;

    egui::ScrollArea::vertical()
        .id_salt("constraint_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, (id, name, icon, kind, detail)) in rows.iter().enumerate() {
                let selected = state.session.selection == Some(Selection::Constraint(*id));
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 24.0),
                    egui::Sense::click(),
                );
                if selected {
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        ui.visuals().selection.bg_fill.linear_multiply(0.3),
                    );
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
                }
                if response.clicked() {
                    state.session.select_constraint(*id);
                }

                // Reorder arrows in the gutter, mirroring the draw-order panel so
                // the two lists behave the same way.
                let arrow_w = 14.0;
                for (slot, up) in [(0.0, true), (1.0, false)] {
                    let arrow_rect = egui::Rect::from_min_size(
                        rect.min + egui::vec2(slot * arrow_w, 0.0),
                        egui::vec2(arrow_w, rect.height()),
                    );
                    let enabled = setup && if up { i > 0 } else { i + 1 < rows.len() };
                    let arrow = ui.interact(
                        arrow_rect,
                        ui.id().with(("constraint_move", i, up)),
                        egui::Sense::click(),
                    );
                    if enabled && arrow.clicked() {
                        swap = Some(if up { (i, i - 1) } else { (i, i + 1) });
                    }
                    ui.painter().text(
                        arrow_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        if up {
                            crate::ui::icons::CARET_UP
                        } else {
                            crate::ui::icons::CARET_DOWN
                        },
                        egui::FontId::proportional(10.0),
                        if enabled {
                            ui.visuals().weak_text_color()
                        } else {
                            ui.visuals().weak_text_color().gamma_multiply(0.3)
                        },
                    );
                }

                let text_color = if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().text_color()
                };
                let x = rect.min.x + arrow_w * 2.0 + 6.0;
                ui.painter().text(
                    egui::pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    icon,
                    egui::FontId::proportional(12.0),
                    hue(kind),
                );
                ui.painter().text(
                    egui::pos2(x + 18.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    name,
                    egui::FontId::proportional(12.5),
                    text_color,
                );
                ui.painter().text(
                    egui::pos2(rect.max.x - 6.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    detail,
                    egui::FontId::proportional(10.0),
                    ui.visuals().weak_text_color(),
                );

                response.context_menu(|ui| {
                    if ui.add_enabled(setup, egui::Button::new("Delete")).clicked() {
                        remove = Some(*id);
                        ui.close();
                    }
                });
            }
        });

    if let Some((a, b)) = swap {
        let mut next = order.clone();
        next.swap(a, b);
        state.dispatch(Box::new(
            crate::commands::constraint_cmds::SetConstraintOrder::new(next),
        ));
    }
    if let Some(id) = remove {
        state.dispatch(Box::new(RemoveConstraint::new(id)));
    }
}

fn glyph(constraint: &Constraint) -> (&'static str, &'static str) {
    match constraint {
        Constraint::Ik(_) => (crate::ui::icons::IK, "IK"),
        Constraint::Transform(_) => (crate::ui::icons::TRANSFORM_CONSTRAINT, "transform"),
        Constraint::Physics(_) => (crate::ui::icons::PHYSICS, "physics"),
        Constraint::Path(_) => (crate::ui::icons::PATH, "path"),
    }
}

fn hue(kind: &str) -> egui::Color32 {
    match kind {
        "IK" => egui::Color32::from_rgb(240, 170, 90),
        "transform" => egui::Color32::from_rgb(150, 190, 240),
        "physics" => egui::Color32::from_rgb(160, 220, 230),
        _ => egui::Color32::from_rgb(220, 190, 120),
    }
}

/// What the row says on the right: the kind, and how much of it is switched on.
///
/// A constraint at mix 0 is the single most confusing state in a rig — it is
/// listed, it is wired up, and it does nothing — so the mix is on the row rather
/// than one click away.
fn summary(state: &AppState, constraint: &Constraint) -> String {
    let bone_name = |id| {
        state
            .doc
            .skeleton
            .bones
            .get(id)
            .map(|b| b.name.as_str())
            .unwrap_or("?")
    };
    match constraint {
        Constraint::Ik(ik) => format!(
            "IK · {} bone(s) → {} · mix {:.0}%",
            ik.bones.len(),
            bone_name(ik.target),
            ik.mix * 100.0
        ),
        Constraint::Transform(tc) => format!(
            "transform · {} bone(s) → {} · rot {:.0}%",
            tc.bones.len(),
            bone_name(tc.target),
            tc.mix.rotate * 100.0
        ),
        Constraint::Physics(p) => format!("physics · {}", bone_name(p.bone)),
        Constraint::Path(p) => format!("path · {} bone(s)", p.bones.len()),
    }
}
