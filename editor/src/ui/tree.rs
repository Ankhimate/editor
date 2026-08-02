use crate::app_state::AppState;
use ankhimate_core::ids::BoneId;
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

    // ── Bones ──────────────────────────────────────────────────────────
    section_header(ui, egui_phosphor::regular::BONE, "Bones");

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
        section_header(ui, egui_phosphor::regular::STACK, "Draw Order");
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

fn section_header(ui: &mut egui::Ui, icon: &str, label: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(icon)
                .size(12.0)
                .color(ui.visuals().selection.bg_fill),
        );
        ui.add_space(2.0);
        ui.label(egui::RichText::new(label).strong().size(12.0));
    });
    ui.separator();
    ui.add_space(2.0);
}

fn render_bone_node(ui: &mut egui::Ui, state: &mut AppState, bone_id: BoneId, depth: usize) {
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
    let mut is_open = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(true));
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

    ui.painter().text(
        egui::pos2(cx + 8.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::BONE,
        egui::FontId::proportional(12.0),
        text_color.gamma_multiply(if is_selected { 1.0 } else { 0.7 }),
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
        for child in children {
            render_bone_node(ui, state, child, depth + 1);
        }
    }
}
