use crate::app_state::AppState;
use crate::session::{Tool, TransformTool, WorkMode};
use crate::theme::Theme;
use eframe::egui;

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &mut Theme,
    available_themes: &[Theme],
    trigger_undo: &mut bool,
    trigger_redo: &mut bool,
) {
    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);

    ui.horizontal(|ui| {
        // ── Work mode (T-207) ──────────────────────────────────────────
        // First control in the window on purpose: it decides what every other
        // control in the editor writes to.
        mode_switch(ui, state, theme);

        ui.add_space(6.0);
        ui.add(egui::Separator::default().vertical().shrink(6.0));
        ui.add_space(6.0);

        // ── Tool buttons ───────────────────────────────────────────────
        // Rig-authoring tools are Setup-only; they stay visible while animating
        // so the toolbar does not jump, but disabled with the reason on hover.
        let setup = state.session.can_edit_structure();
        tool_btn(
            ui,
            theme,
            ToolBtn {
                icon: egui_phosphor::regular::CURSOR,
                shortcut: "V",
                tooltip: "Select",
                selected: state.session.tool == Tool::Select,
                enabled: true,
            },
            || state.session.tool = Tool::Select,
        );
        tool_btn(
            ui,
            theme,
            ToolBtn {
                icon: egui_phosphor::regular::BONE,
                shortcut: "B",
                tooltip: "Create Bone",
                selected: state.session.tool == Tool::CreateBone,
                enabled: setup,
            },
            || state.session.tool = Tool::CreateBone,
        );
        tool_btn(
            ui,
            theme,
            ToolBtn {
                icon: egui_phosphor::regular::PAINT_BRUSH,
                shortcut: "W",
                tooltip: "Weight Paint",
                selected: state.session.tool == Tool::WeightPaint,
                enabled: setup,
            },
            || state.session.tool = Tool::WeightPaint,
        );

        ui.add_space(6.0);
        ui.add(egui::Separator::default().vertical().shrink(6.0));
        ui.add_space(6.0);

        // ── Transform gizmo (T/R/S/H) ──────────────────────────────────
        // These pick which gizmo the Select tool shows, so choosing one also
        // switches back to Select: clicking "Rotate" while the bone-creation
        // tool is active otherwise sets a mode the next click cannot use.
        //
        // Enabled in both work modes on purpose — posing in Animate is the main
        // reason to reach for them.
        for spec in [
            (
                egui_phosphor::regular::ARROWS_OUT_CARDINAL,
                "T",
                "Translate",
                TransformTool::Translate,
            ),
            (
                egui_phosphor::regular::ARROW_CLOCKWISE,
                "R",
                "Rotate",
                TransformTool::Rotate,
            ),
            (
                egui_phosphor::regular::RESIZE,
                "S",
                "Scale",
                TransformTool::Scale,
            ),
            (
                egui_phosphor::regular::PARALLELOGRAM,
                "H",
                "Shear",
                TransformTool::Shear,
            ),
        ] {
            let (icon, shortcut, tooltip, tool) = spec;
            tool_btn(
                ui,
                theme,
                ToolBtn {
                    icon,
                    shortcut,
                    tooltip,
                    // Only lit while Select is active: the gizmo is not on
                    // screen under the other tools, so showing it as the current
                    // mode would be a lie.
                    selected: state.session.tool == Tool::Select
                        && state.session.active_transform_tool == tool,
                    enabled: true,
                },
                || {
                    state.session.active_transform_tool = tool;
                    state.session.tool = Tool::Select;
                },
            );
        }

        ui.add_space(6.0);
        ui.add(egui::Separator::default().vertical().shrink(6.0));
        ui.add_space(6.0);

        // ── Visibility filters ─────────────────────────────────────────
        // Toggles rather than a menu: on a dense rig these get flipped every few
        // seconds, and a menu makes that four clicks instead of one.
        for (icon, tooltip, flag) in [
            (
                egui_phosphor::regular::IMAGE_SQUARE,
                "Show artwork (1)",
                &mut state.session.show_artwork,
            ),
            (
                egui_phosphor::regular::BONE,
                "Show bones (2)",
                &mut state.session.show_bones,
            ),
        ] {
            let on = *flag;
            let button = egui::Button::new(egui::RichText::new(icon).size(15.0).color(if on {
                theme.primary()
            } else {
                ui.visuals().weak_text_color()
            }))
            .fill(if on {
                ui.visuals().faint_bg_color
            } else {
                egui::Color32::TRANSPARENT
            });
            if ui.add(button).on_hover_text(tooltip).clicked() {
                *flag = !on;
            }
        }

        ui.add_space(6.0);
        ui.add(egui::Separator::default().vertical().shrink(6.0));
        ui.add_space(6.0);

        // ── Undo / Redo ────────────────────────────────────────────────
        let can_undo = state.history.can_undo();
        let can_redo = state.history.can_redo();

        let undo = ui
            .add_enabled(can_undo, icon_btn(egui_phosphor::regular::ARROW_U_UP_LEFT))
            .on_hover_text(format!(
                "Undo (Ctrl+Z)  [{} steps]",
                state.history.undo_depth()
            ));
        if undo.clicked() {
            *trigger_undo = true;
        }

        let redo = ui
            .add_enabled(can_redo, icon_btn(egui_phosphor::regular::ARROW_U_UP_RIGHT))
            .on_hover_text(format!("Redo (Ctrl+Y)  [{} steps]", 0));
        if redo.clicked() {
            *trigger_redo = true;
        }

        // ── Theme selector (right-aligned) ─────────────────────────────
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let mut current = theme.clone();
            egui::ComboBox::from_id_salt("theme_selector")
                .selected_text(format!(
                    "{} {}",
                    egui_phosphor::regular::PALETTE,
                    current.label()
                ))
                .width(130.0)
                .show_ui(ui, |ui| {
                    for t in available_themes {
                        ui.selectable_value(&mut current, t.clone(), t.label());
                    }
                });
            if current != *theme {
                *theme = current;
                ui.ctx().request_repaint();
            }
        });
    });
}

/// The Setup ⇄ Animate segmented switch (T-207).
fn mode_switch(ui: &mut egui::Ui, state: &mut AppState, theme: &Theme) {
    let current = state.session.work_mode;
    let mut clicked: Option<WorkMode> = None;

    for (mode, tooltip) in [
        (
            WorkMode::Setup,
            "Setup mode (Tab) — edit the rig: create, parent, and pose the setup skeleton",
        ),
        (
            WorkMode::Animate,
            "Animate mode (Tab) — edits become keys on the active animation",
        ),
    ] {
        let selected = mode == current;
        let (fill, text) = if selected {
            (theme.primary(), theme.on_primary())
        } else {
            (egui::Color32::TRANSPARENT, ui.visuals().weak_text_color())
        };
        let btn = egui::Button::new(egui::RichText::new(mode.label()).size(10.5).color(text))
            .min_size(egui::vec2(66.0, 24.0))
            .fill(fill);
        if ui.add(btn).on_hover_text(tooltip).clicked() && !selected {
            clicked = Some(mode);
        }
    }

    if let Some(mode) = clicked {
        state.set_work_mode(mode);
    }
}

/// One toolbar tool button.
struct ToolBtn<'a> {
    icon: &'a str,
    shortcut: &'a str,
    tooltip: &'a str,
    selected: bool,
    /// Setup-only tools are drawn but disabled while animating (T-207).
    enabled: bool,
}

fn tool_btn<F: FnOnce()>(ui: &mut egui::Ui, theme: &Theme, spec: ToolBtn<'_>, on_click: F) {
    let ToolBtn {
        icon,
        shortcut,
        tooltip,
        selected,
        enabled,
    } = spec;
    let (bg_fill, mut icon_color, stroke) = if selected {
        (
            theme.primary(),
            theme.on_primary(),
            egui::Stroke::new(1.0, theme.primary().linear_multiply(0.7)),
        )
    } else {
        (
            egui::Color32::TRANSPARENT,
            ui.visuals().text_color(),
            egui::Stroke::NONE,
        )
    };
    if !enabled {
        icon_color = ui.visuals().weak_text_color().gamma_multiply(0.5);
    }

    let btn = egui::Button::new("")
        .min_size(egui::vec2(36.0, 32.0))
        .fill(bg_fill)
        .stroke(stroke);

    let hover = if enabled {
        format!("{tooltip} ({shortcut})")
    } else {
        format!("{tooltip} ({shortcut}) — Setup mode only")
    };
    let response = ui.add_enabled(enabled, btn).on_hover_text(hover);
    let painter = ui.painter_at(response.rect);

    // Icon (center-top biased) + shortcut hint below
    let cx = response.rect.center().x;
    let icon_y = response.rect.center().y - 3.0;
    let hint_y = response.rect.max.y - 6.0;

    painter.text(
        egui::pos2(cx, icon_y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(15.0),
        icon_color,
    );
    painter.text(
        egui::pos2(cx, hint_y),
        egui::Align2::CENTER_CENTER,
        shortcut,
        egui::FontId::proportional(8.0),
        icon_color.gamma_multiply(0.55),
    );

    if response.clicked() {
        on_click();
    }
}

fn icon_btn(icon: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(icon).size(15.0))
        .min_size(egui::vec2(30.0, 32.0))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
}
