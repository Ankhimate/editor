use crate::app_state::AppState;
use crate::session::Selection;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::constraints::Constraint;
use ankhimate_core::ids::{BoneId, ConstraintId, SlotId};
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    // The hierarchy is rig structure, so it is authored in Setup mode only
    // (T-207). Rows stay browsable while animating — selecting a bone to key it
    // is exactly what this panel is for — but the edits are refused, so say so
    // once at the top instead of failing silently per action.
    if !state.session.can_edit_structure() {
        ui.label(
            egui::RichText::new("Animating — switch to Setup (Tab) to edit the rig")
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);
    }

    // ── Filter ─────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(egui_phosphor::fill::MAGNIFYING_GLASS)
                .size(12.0)
                .color(ui.visuals().weak_text_color()),
        );
        let width = ui.available_width() - 24.0;
        ui.add(
            egui::TextEdit::singleline(&mut state.session.tree_filter)
                .hint_text("Filter")
                .desired_width(width.max(40.0)),
        );
        if !state.session.tree_filter.is_empty()
            && ui
                .add(
                    egui::Button::new(egui_phosphor::fill::X_CIRCLE)
                        .fill(egui::Color32::TRANSPARENT),
                )
                .on_hover_text("Clear")
                .clicked()
        {
            state.session.tree_filter.clear();
        }
    });
    ui.add_space(2.0);

    // ── Bones ──────────────────────────────────────────────────────────
    section_header_counted(
        ui,
        egui_phosphor::fill::BONE,
        "Bones",
        Some(state.doc.skeleton.bones.len()),
    );

    let root_bones: Vec<BoneId> = state
        .doc
        .skeleton
        .bones
        .iter()
        .filter_map(|(id, b)| if b.parent.is_none() { Some(id) } else { None })
        .collect();

    if root_bones.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No bones yet")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                egui::RichText::new("Use the Create Bone tool (B)")
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
    } else {
        for root in root_bones {
            render_bone_node(ui, state, root, 0);
        }
    }

    ui.add_space(8.0);

    // ── Draw Order / Slots ─────────────────────────────────────────────
    ui.horizontal(|ui| {
        section_header_counted(
            ui,
            egui_phosphor::fill::STACK,
            "Draw Order",
            Some(state.doc.skeleton.draw_order.len()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Creating a slot is structural: Setup mode only, and via a command
            // so it is undoable (it used to poke the skeleton directly).
            let setup = state.session.can_edit_structure();
            let can_add = setup && state.session.active_bone().is_some();
            let btn = ui.add_enabled(
                can_add,
                egui::Button::new(
                    egui::RichText::new(format!("{} Add", egui_phosphor::regular::PLUS)).size(11.0),
                )
                .min_size(egui::vec2(0.0, 20.0)),
            );
            let btn = if !setup {
                btn.on_hover_text("Switch to Setup mode to add slots (Tab)")
            } else if !can_add {
                btn.on_hover_text("Select a bone first to add a slot")
            } else {
                btn
            };
            if btn.clicked()
                && let Some(bone_id) = state.session.active_bone()
            {
                let name = format!("Slot {}", state.doc.skeleton.slots.len() + 1);
                if state.dispatch(Box::new(crate::commands::slot_cmds::CreateSlot::new(
                    name, bone_id,
                ))) && let Some(&id) = state.doc.skeleton.draw_order.last()
                {
                    state.session.select_slot(Some(id));
                }
            }
        });
    });

    if state.doc.skeleton.draw_order.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("No slots yet")
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
        });
        return;
    }

    let mut move_up = None;
    let mut move_down = None;

    for (i, &slot_id) in state.doc.skeleton.draw_order.iter().enumerate() {
        if let Some(slot) = state.doc.skeleton.slots.get(slot_id) {
            let is_selected = state.session.active_slot() == Some(slot_id);
            let row_height = 24.0;

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row_height),
                egui::Sense::click(),
            );

            if is_selected {
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
                state.session.select_slot(Some(slot_id));
            }

            let text_color = if is_selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().text_color()
            };

            let arrow_w = 16.0;
            let up_rect = egui::Rect::from_min_size(rect.min, egui::vec2(arrow_w, row_height));
            let dn_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x + arrow_w, rect.min.y),
                egui::vec2(arrow_w, row_height),
            );

            let up_resp = ui.interact(up_rect, ui.id().with(("up", i)), egui::Sense::click());
            let dn_resp = ui.interact(dn_rect, ui.id().with(("dn", i)), egui::Sense::click());
            if up_resp.clicked() {
                move_up = Some(i);
            }
            if dn_resp.clicked() {
                move_down = Some(i);
            }

            let arrow_color = ui.visuals().weak_text_color();
            ui.painter().text(
                up_rect.center(),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::CARET_UP,
                egui::FontId::proportional(10.0),
                arrow_color,
            );
            ui.painter().text(
                dn_rect.center(),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::regular::CARET_DOWN,
                egui::FontId::proportional(10.0),
                arrow_color,
            );

            let icon_x = rect.min.x + arrow_w * 2.0 + 6.0;
            ui.painter().text(
                egui::pos2(icon_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                egui_phosphor::regular::CIRCLE_DASHED,
                egui::FontId::proportional(12.0),
                text_color.gamma_multiply(0.6),
            );
            ui.painter().text(
                egui::pos2(icon_x + 16.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &slot.name,
                egui::FontId::proportional(13.0),
                text_color,
            );
        }
    }

    // Reordering goes through the router like every other edit (T-207): the
    // setup stack in Setup mode, a draw-order key in Animate mode. It used to
    // swap the vector in place, which was neither undoable nor animatable.
    let swap = match (move_up, move_down) {
        (Some(i), _) if i > 0 => Some((i, i - 1)),
        (_, Some(i)) if i + 1 < state.doc.skeleton.draw_order.len() => Some((i, i + 1)),
        _ => None,
    };
    if let Some((a, b)) = swap {
        // Indices address the list this panel drew, which is the setup order.
        let mut order = state.doc.skeleton.draw_order.clone();
        order.swap(a, b);
        state.commit_draw_order(order);
    }
}

/// A section title, optionally with how many rows are under it.
///
/// The count is what tells you a section is empty because the rig has none of
/// that thing, rather than because a filter hid them all.
fn section_header_counted(ui: &mut egui::Ui, icon: &str, label: &str, count: Option<usize>) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(icon)
                .size(12.0)
                .color(ui.visuals().selection.bg_fill),
        );
        ui.add_space(3.0);
        ui.label(
            egui::RichText::new(label.to_uppercase())
                .strong()
                .size(10.5)
                .color(ui.visuals().strong_text_color()),
        );
        if let Some(count) = count {
            ui.label(
                egui::RichText::new(count.to_string())
                    .size(10.0)
                    .color(ui.visuals().weak_text_color()),
            );
        }
    });
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

/// Does this bone, or anything under it, match the filter?
///
/// Subtree-aware on purpose: matching only the row itself would hide every match
/// that happens to live under a parent whose name does not contain the query,
/// which is most of them.
fn subtree_matches(state: &AppState, bone_id: BoneId, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let Some(bone) = state.doc.skeleton.bones.get(bone_id) else {
        return false;
    };
    if bone.name.to_lowercase().contains(needle) {
        return true;
    }
    // A slot or attachment hanging off this bone counts as a match too — art is
    // what people search for at least as often as bones.
    let slot_match = state
        .doc
        .skeleton
        .slots
        .iter()
        .filter(|(_, s)| s.bone == bone_id)
        .any(|(id, s)| {
            s.name.to_lowercase().contains(needle)
                || state.doc.skeleton.skins.iter().any(|(_, skin)| {
                    skin.names_for_slot(id)
                        .any(|n| n.to_lowercase().contains(needle))
                })
        });
    if slot_match {
        return true;
    }
    let children: Vec<BoneId> = state
        .doc
        .skeleton
        .bones
        .iter()
        .filter_map(|(id, b)| (b.parent == Some(bone_id)).then_some(id))
        .collect();
    children
        .into_iter()
        .any(|child| subtree_matches(state, child, needle))
}

fn render_bone_node(ui: &mut egui::Ui, state: &mut AppState, bone_id: BoneId, depth: usize) {
    let needle = state.session.tree_filter.to_lowercase();
    if !subtree_matches(state, bone_id, &needle) {
        return;
    }
    let bone_name = match state.doc.skeleton.bones.get(bone_id) {
        Some(b) => b.name.clone(),
        None => return,
    };
    let children: Vec<BoneId> = state
        .doc
        .skeleton
        .bones
        .iter()
        .filter_map(|(id, b)| {
            if b.parent == Some(bone_id) {
                Some(id)
            } else {
                None
            }
        })
        .collect();

    let id = ui.make_persistent_id(bone_id);
    // A filter forces every surviving branch open: a match buried in a collapsed
    // parent is a match nobody can see, which defeats the filter.
    let mut is_open = needle.is_empty() && ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(true))
        || !needle.is_empty();
    let row_height = 22.0;

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height),
        egui::Sense::click(),
    );
    let is_selected = state.session.is_bone_selected(bone_id);

    if is_selected {
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
        state.session.select_bone(Some(bone_id));
    }

    // Right-click: reparent / rename / delete (T-206).
    response.context_menu(|ui| {
        let active = state.session.active_bone();
        if let Some(target) = active
            && target != bone_id
            && ui.button("Parent to selected").clicked()
        {
            // Reparent the *right-clicked* bone under the active selection,
            // keeping it fixed on screen.
            state.dispatch(Box::new(
                crate::commands::bone_cmds::SetBoneParent::keeping_world(
                    &state.doc.skeleton,
                    bone_id,
                    Some(target),
                ),
            ));
            ui.close();
        }
        if ui.button("Unparent (to root)").clicked() {
            state.dispatch(Box::new(
                crate::commands::bone_cmds::SetBoneParent::keeping_world(
                    &state.doc.skeleton,
                    bone_id,
                    None,
                ),
            ));
            ui.close();
        }
        ui.separator();
        if ui.button("Rename").clicked() {
            ui.data_mut(|d| d.insert_temp(id.with("renaming"), true));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            state.dispatch(Box::new(crate::commands::bone_cmds::DeleteBone::new(
                bone_id,
            )));
            ui.close();
        }
    });

    let text_color = if is_selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().text_color()
    };

    // Lock padlock in the left gutter — click toggles (T-206). A locked bone
    // ignores viewport drags and auto-key (enforced in `commit_bone_pose`).
    let locked = state.session.is_bone_locked(bone_id);
    let vis_rect = egui::Rect::from_min_size(rect.min, egui::vec2(24.0, row_height));
    let lock_resp = ui.interact(vis_rect, id.with("lock"), egui::Sense::click());
    if lock_resp.clicked() {
        let new = !locked;
        state.session.locked_bones.insert(bone_id, new);
    }
    let lock_icon = if locked {
        egui_phosphor::regular::LOCK
    } else {
        egui_phosphor::regular::LOCK_OPEN
    };
    ui.painter().text(
        vis_rect.center(),
        egui::Align2::CENTER_CENTER,
        lock_icon,
        egui::FontId::proportional(11.0),
        if locked {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().weak_text_color().gamma_multiply(0.5)
        },
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + 24.0, rect.min.y),
            egui::pos2(rect.min.x + 24.0, rect.max.y),
        ],
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );

    let indent_w = 14.0;
    let start_x = rect.min.x + 24.0 + 4.0;
    let guide_color = ui.visuals().widgets.noninteractive.bg_stroke.color;

    for d in 0..depth {
        let x = start_x + (d as f32) * indent_w + indent_w / 2.0;
        ui.painter().line_segment(
            [egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)],
            egui::Stroke::new(1.0, guide_color),
        );
    }
    if depth > 0 {
        let px = start_x + (depth as f32 - 1.0) * indent_w + indent_w / 2.0;
        let mx = start_x + (depth as f32) * indent_w + indent_w / 2.0;
        ui.painter().line_segment(
            [
                egui::pos2(px, rect.center().y),
                egui::pos2(mx, rect.center().y),
            ],
            egui::Stroke::new(1.0, guide_color),
        );
    }

    let indent = indent_w * (depth as f32);
    let mut cx = start_x + indent;

    if !children.is_empty() {
        let toggle_rect =
            egui::Rect::from_min_size(egui::pos2(cx, rect.min.y), egui::vec2(14.0, row_height));
        let toggle_resp = ui.interact(toggle_rect, id.with("toggle"), egui::Sense::click());
        if toggle_resp.clicked() {
            is_open = !is_open;
            ui.data_mut(|d| d.insert_temp(id, is_open));
        }
        let icon = if is_open {
            egui_phosphor::regular::CARET_DOWN
        } else {
            egui_phosphor::regular::CARET_RIGHT
        };
        let c = if toggle_resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon,
            egui::FontId::proportional(11.0),
            c,
        );
    }
    cx += 14.0;

    // The bone glyph carries the bone's colour — inherited from the nearest
    // coloured ancestor (T-505), so a limb reads as one group at a glance. In a
    // 67-bone rig that is the difference between finding a bone and hunting it.
    ui.painter().text(
        egui::pos2(cx + 8.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::BONE,
        egui::FontId::proportional(12.0),
        bone_tint(&state.doc.skeleton, bone_id).gamma_multiply(if is_selected {
            1.0
        } else {
            0.85
        }),
    );
    cx += 18.0;

    let renaming = ui.data(|d| d.get_temp::<bool>(id.with("renaming")).unwrap_or(false));
    if renaming {
        // Inline rename field over the label area, committed on Enter/blur.
        let field_rect = egui::Rect::from_min_max(
            egui::pos2(cx, rect.min.y + 1.0),
            egui::pos2(rect.max.x - 4.0, rect.max.y - 1.0),
        );
        let mut buf = ui
            .data(|d| d.get_temp::<String>(id.with("rename_buf")))
            .unwrap_or_else(|| bone_name.clone());
        let resp = ui.put(field_rect, egui::TextEdit::singleline(&mut buf));
        resp.request_focus();
        ui.data_mut(|d| d.insert_temp(id.with("rename_buf"), buf.clone()));
        let commit = resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter));
        if commit {
            if !buf.trim().is_empty() && buf != bone_name {
                state.dispatch(Box::new(crate::commands::bone_cmds::RenameBone::new(
                    bone_id, buf,
                )));
            }
            ui.data_mut(|d| {
                d.remove::<bool>(id.with("renaming"));
                d.remove::<String>(id.with("rename_buf"));
            });
        }
    } else {
        // Double-click the label to start renaming.
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(cx, rect.min.y),
            egui::pos2(rect.max.x, rect.max.y),
        );
        let name_resp = ui.interact(name_rect, id.with("name"), egui::Sense::click());
        if name_resp.double_clicked() {
            ui.data_mut(|d| d.insert_temp(id.with("renaming"), true));
        }
        ui.painter().text(
            egui::pos2(cx, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &bone_name,
            egui::FontId::proportional(13.0),
            text_color,
        );
    }

    if is_open {
        // What this bone carries, before its child bones: the slots it draws,
        // each slot's attachments, and the constraints that drive it. All
        // selectable — an item you can see but not click cannot be inspected,
        // which is exactly what made a misplaced attachment hard to chase.
        let slots: Vec<(SlotId, String, Option<String>)> = state
            .doc
            .skeleton
            .slots
            .iter()
            .filter(|(_, s)| s.bone == bone_id)
            .map(|(id, s)| (id, s.name.clone(), s.attachment.clone()))
            .collect();
        for (slot_id, slot_name, active) in slots {
            let clicked = selectable_row(
                ui,
                state,
                Row {
                    icon: egui_phosphor::regular::CIRCLE_DASHED,
                    label: slot_name,
                    tint: None,
                    depth: depth + 2,
                    selection: Selection::Slot(slot_id),
                    detail: String::new(),
                },
            )
            .clicked();
            if clicked {
                state.session.select_slot(Some(slot_id));
            }

            // Attachments in the active skin, so the tree shows what is
            // resolvable right now rather than every skin at once.
            let skin = state.session.active_skin;
            let names: Vec<String> = state
                .doc
                .skeleton
                .skins
                .get(skin)
                .map(|s| s.names_for_slot(slot_id).map(str::to_string).collect())
                .unwrap_or_default();
            for name in names {
                let Some(attachment) = state.doc.skeleton.skins[skin].get(slot_id, &name) else {
                    continue;
                };
                let (icon, kind, hue) = attachment_glyph(attachment);
                let shown = active.as_deref() == Some(name.as_str());
                let clicked = selectable_row(
                    ui,
                    state,
                    Row {
                        icon,
                        label: name.clone(),
                        // A hidden attachment keeps its kind's colour but dimmed
                        // rather than being omitted: "the art exists, this slot
                        // is just not showing it" is a common cause of a piece
                        // missing from the canvas.
                        tint: Some(if shown { hue } else { hue.gamma_multiply(0.4) }),
                        depth: depth + 3,
                        selection: Selection::Attachment {
                            slot: slot_id,
                            name: name.clone(),
                        },
                        detail: kind.to_string(),
                    },
                )
                .clicked();
                if clicked {
                    state.session.select_attachment(slot_id, name, bone_id);
                }
            }
        }

        type ConstraintRow = (
            ConstraintId,
            String,
            &'static str,
            &'static str,
            egui::Color32,
        );
        let constraints: Vec<ConstraintRow> = state
            .doc
            .skeleton
            .constraint_order
            .iter()
            .filter_map(|id| {
                let c = state.doc.skeleton.constraints.get(*id)?;
                c.affected_bones().contains(&bone_id).then(|| {
                    let (icon, kind, hue) = constraint_glyph(c);
                    (*id, c.name().to_string(), icon, kind, hue)
                })
            })
            .collect();
        for (id, name, icon, kind, hue) in constraints {
            let clicked = selectable_row(
                ui,
                state,
                Row {
                    icon,
                    label: name,
                    tint: Some(hue),
                    depth: depth + 2,
                    selection: Selection::Constraint(id),
                    detail: kind.to_string(),
                },
            )
            .clicked();
            if clicked {
                state.session.select_constraint(id);
            }
        }

        for child in children {
            render_bone_node(ui, state, child, depth + 1);
        }
    }
}

/// One selectable row: an icon, a label, and what it focuses (T-708).
///
/// Every item in the rig gets one — bones, slots, attachments, constraints —
/// because "I can see it but I cannot click it" is how a rig becomes hard to
/// diagnose. Which is exactly the position we were in trying to work out why an
/// imported character looked wrong.
struct Row<'a> {
    icon: &'a str,
    label: String,
    /// Tint for the icon; bones use their own colour, everything else follows
    /// the theme.
    tint: Option<egui::Color32>,
    depth: usize,
    selection: Selection,
    /// Shown to the right in a dimmer colour: the attachment's kind, a
    /// constraint's target, and so on.
    detail: String,
}

/// Draw one row and report whether it was clicked.
fn selectable_row(ui: &mut egui::Ui, state: &mut AppState, row: Row<'_>) -> egui::Response {
    let height = 21.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let selected = state.session.selection.as_ref() == Some(&row.selection);

    if selected {
        ui.painter().rect_filled(
            rect,
            0.0,
            ui.visuals().selection.bg_fill.linear_multiply(0.3),
        );
        // Reveal once, when something outside the tree made the selection.
        // Scrolling every frame pinned the panel to the selected row.
        if state.session.reveal_selection && response.rect.height() > 0.0 {
            ui.scroll_to_rect(rect, None);
            state.session.reveal_selection = false;
        }
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().faint_bg_color);
    }

    let text_color = if selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().text_color()
    };
    let x = rect.min.x + 6.0 + row.depth as f32 * 12.0;
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        row.icon,
        egui::FontId::proportional(12.0),
        row.tint.unwrap_or(text_color.gamma_multiply(0.75)),
    );
    ui.painter().text(
        egui::pos2(x + 16.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &row.label,
        egui::FontId::proportional(12.5),
        text_color,
    );
    if !row.detail.is_empty() {
        ui.painter().text(
            egui::pos2(rect.max.x - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            &row.detail,
            egui::FontId::proportional(10.0),
            ui.visuals().weak_text_color(),
        );
    }
    response
}

/// The icon, one-word kind, and hue for an attachment.
///
/// Filled glyphs: at 12px an outline icon is mostly gaps, and a column of sixty
/// of them reads as texture rather than as a list of distinct things.
///
/// Each kind also gets a hue, because shape alone is not enough at this size —
/// a mesh and a hitbox are both "an angular outline" until one of them is green.
fn attachment_glyph(attachment: &Attachment) -> (&'static str, &'static str, egui::Color32) {
    match attachment {
        Attachment::Region(_) => (
            egui_phosphor::fill::IMAGE_SQUARE,
            "image",
            egui::Color32::from_rgb(126, 176, 224),
        ),
        Attachment::Mesh(_) => (
            egui_phosphor::fill::POLYGON,
            "mesh",
            egui::Color32::from_rgb(140, 200, 150),
        ),
        Attachment::Clipping(_) => (
            egui_phosphor::fill::SCISSORS,
            "clip",
            egui::Color32::from_rgb(200, 160, 220),
        ),
        Attachment::Path(_) => (
            egui_phosphor::fill::PATH,
            "path",
            egui::Color32::from_rgb(220, 190, 120),
        ),
        Attachment::BoundingBox(_) => (
            egui_phosphor::fill::BOUNDING_BOX,
            "hitbox",
            egui::Color32::from_rgb(230, 140, 105),
        ),
        Attachment::Point(_) => (
            egui_phosphor::fill::CROSSHAIR_SIMPLE,
            "point",
            egui::Color32::from_rgb(124, 227, 139),
        ),
    }
}

/// The icon and kind for a constraint. IK and FK-driven constraints get
/// *different* glyphs on purpose: "why is this bone moving on its own" is
/// answered by which kind is attached to it, so the two must not look alike.
fn constraint_glyph(constraint: &Constraint) -> (&'static str, &'static str, egui::Color32) {
    match constraint {
        Constraint::Ik(_) => (
            egui_phosphor::fill::TREE_STRUCTURE,
            "IK",
            egui::Color32::from_rgb(240, 170, 90),
        ),
        Constraint::Transform(_) => (
            egui_phosphor::fill::ARROWS_LEFT_RIGHT,
            "transform",
            egui::Color32::from_rgb(150, 190, 240),
        ),
        Constraint::Physics(_) => (
            egui_phosphor::fill::WIND,
            "physics",
            egui::Color32::from_rgb(160, 220, 230),
        ),
        Constraint::Path(_) => (
            egui_phosphor::fill::PATH,
            "path",
            egui::Color32::from_rgb(220, 190, 120),
        ),
    }
}

/// A bone's colour as the tree should show it: its own, or inherited from the
/// nearest ancestor that set one.
fn bone_tint(skeleton: &ankhimate_core::skeleton::Skeleton, bone: BoneId) -> egui::Color32 {
    let [r, g, b, _] = crate::ui::canvas::renderer::group_color(skeleton, bone);
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
    egui::Color32::from_rgb(to_u8(r), to_u8(g), to_u8(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use ankhimate_core::attachment::{Rect, RegionAttachment};
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    fn rig() -> AppState {
        let mut state = AppState::default();
        let root = state.doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let arm = state.doc.skeleton.add_bone(Bone {
            name: "front-upper-arm".into(),
            parent: Some(root),
            length: 10.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = state.doc.skeleton.add_slot(Slot {
            attachment: Some("gun".into()),
            ..Slot::new("weapon".to_string(), arm)
        });
        let skin = state.doc.skeleton.default_skin;
        state.doc.skeleton.skins[skin].set(
            slot,
            "gun",
            ankhimate_core::attachment::Attachment::Region(RegionAttachment {
                texture: "gun".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 8.0,
                height: 8.0,
                uv_rect: Rect::default(),
                pivot: glam::Vec2::splat(0.5),
                sequence: None,
            }),
        );
        state
    }

    fn bone_by_name(state: &AppState, name: &str) -> BoneId {
        state
            .doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == name)
            .map(|(id, _)| id)
            .unwrap()
    }

    #[test]
    fn an_empty_filter_keeps_everything() {
        let state = rig();
        let root = bone_by_name(&state, "root");
        assert!(subtree_matches(&state, root, ""));
    }

    /// The point of matching a subtree: `arm` lives under `root`, and hiding
    /// `root` would hide the match with it.
    #[test]
    fn a_parent_survives_when_a_descendant_matches() {
        let state = rig();
        let root = bone_by_name(&state, "root");
        assert!(subtree_matches(&state, root, "arm"));
    }

    #[test]
    fn a_branch_with_no_match_anywhere_is_dropped() {
        let state = rig();
        let arm = bone_by_name(&state, "front-upper-arm");
        assert!(!subtree_matches(&state, arm, "leg"));
    }

    /// Art is searched for at least as often as bones, so a slot or attachment
    /// name keeps its bone visible too.
    #[test]
    fn slot_and_attachment_names_match() {
        let state = rig();
        let arm = bone_by_name(&state, "front-upper-arm");
        assert!(subtree_matches(&state, arm, "weapon"), "slot name");
        assert!(subtree_matches(&state, arm, "gun"), "attachment name");
    }

    #[test]
    fn matching_ignores_case() {
        let state = rig();
        let root = bone_by_name(&state, "root");
        assert!(subtree_matches(&state, root, "arm"));
        // The caller lowercases the needle; the haystack is lowercased here.
        assert!(subtree_matches(&state, root, "front-upper"));
    }
}
