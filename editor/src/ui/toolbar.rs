//! The tool rail: a vertical strip of tools down the left edge.
//!
//! Vertical and on the left because tools are picked with the pointer already
//! over the viewport, and a horizontal strip across the top is the longest trip
//! from where the hand already is. It also leaves the full window width to the
//! panels, which matters most for the timeline.
//!
//! Icon-only. The rail is 44px, so there is no room for labels — the tooltip
//! carries the name and the shortcut, and the shortcut is what anybody uses
//! after the first day anyway.
//!
//! What is *not* here: the theme picker (Settings owns it) and the clip
//! controls (the timeline's own header owns those). A rail that accumulates
//! everything is a rail nobody can find anything in.

use crate::app_state::AppState;
use crate::session::{Tool, TransformTool};
use eframe::egui;

/// Width of the rail, including its margins.
pub const RAIL_WIDTH: f32 = 44.0;

const BTN: f32 = 32.0;

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut AppState,
    theme: &crate::theme::Theme,
    trigger_undo: &mut bool,
    trigger_redo: &mut bool,
) {
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 3.0);
    ui.vertical_centered(|ui| {
        // ── Tools ──────────────────────────────────────────────────────
        // Rig-authoring tools stay visible while animating so the rail does not
        // reflow under the cursor, but disabled with the reason on hover.
        let setup = state.session.can_edit_structure();
        let tool = state.session.tool;
        for spec in [
            (crate::ui::icons::SELECT, "V", "Select", Tool::Select, true),
            (
                crate::ui::icons::CREATE_BONE,
                "B",
                "Create bone",
                Tool::CreateBone,
                setup,
            ),
            (
                crate::ui::icons::WEIGHT_PAINT,
                "W",
                "Weight paint",
                Tool::WeightPaint,
                setup,
            ),
        ] {
            let (icon, key, name, value, enabled) = spec;
            if rail_button(ui, theme, icon, name, key, tool == value, enabled) {
                state.session.tool = value;
            }
        }
        gap(ui);

        // ── Transform gizmo ────────────────────────────────────────────
        let active = state.session.active_transform_tool;
        for spec in [
            (
                crate::ui::icons::TOOL_TRANSLATE,
                "T",
                "Translate",
                TransformTool::Translate,
            ),
            (
                crate::ui::icons::TOOL_ROTATE,
                "R",
                "Rotate",
                TransformTool::Rotate,
            ),
            (
                crate::ui::icons::TOOL_SCALE,
                "S",
                "Scale",
                TransformTool::Scale,
            ),
            (
                crate::ui::icons::TOOL_SHEAR,
                "H",
                "Shear",
                TransformTool::Shear,
            ),
        ] {
            let (icon, key, name, value) = spec;
            // Only lit under Select: the gizmo is not on screen under the other
            // tools, so showing it as the current mode would be a lie.
            let on = tool == Tool::Select && active == value;
            if rail_button(ui, theme, icon, name, key, on, true) {
                state.session.active_transform_tool = value;
                state.session.tool = Tool::Select;
            }
        }
        gap(ui);

        // ── Visibility ─────────────────────────────────────────────────
        if rail_button(
            ui,
            theme,
            crate::ui::icons::IMAGE,
            "Show artwork",
            "1",
            state.session.show_artwork,
            true,
        ) {
            state.session.show_artwork = !state.session.show_artwork;
        }
        if rail_button(
            ui,
            theme,
            crate::ui::icons::BONE,
            "Show bones",
            "2",
            state.session.show_bones,
            true,
        ) {
            state.session.show_bones = !state.session.show_bones;
        }

        // ── Undo / redo, pinned to the bottom ──────────────────────────
        // Out of the way of the tools, which are the things reached for by
        // muscle memory; undo has a keyboard shortcut everybody already knows.
        let remaining = ui.available_height() - (BTN * 2.0 + 6.0);
        if remaining > 0.0 {
            ui.add_space(remaining);
        }
        let can_undo = state.history.can_undo();
        let depth = state.history.undo_depth();
        if rail_button_enabled(
            ui,
            theme,
            crate::ui::icons::UNDO,
            &format!("Undo (Ctrl+Z) — {depth} steps"),
            can_undo,
        ) {
            *trigger_undo = true;
        }
        let can_redo = state.history.can_redo();
        if rail_button_enabled(ui, theme, crate::ui::icons::REDO, "Redo (Ctrl+Y)", can_redo) {
            *trigger_redo = true;
        }
    });
}

/// A separator between groups of tools.
fn gap(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(BTN, 13.0), egui::Sense::hover());
    let rule = egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 1.0));
    ui.painter().rect_filled(
        rule,
        0.0,
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );
}

/// One rail button. Returns whether it was clicked.
fn rail_button(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    icon: &str,
    name: &str,
    shortcut: &str,
    selected: bool,
    enabled: bool,
) -> bool {
    let hover = if enabled {
        format!("{name}  ({shortcut})")
    } else {
        format!("{name}  ({shortcut}) — Setup mode only")
    };
    button(ui, theme, icon, &hover, selected, enabled)
}

fn rail_button_enabled(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    icon: &str,
    hover: &str,
    enabled: bool,
) -> bool {
    button(ui, theme, icon, hover, false, enabled)
}

fn button(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    icon: &str,
    hover: &str,
    selected: bool,
    enabled: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(BTN, BTN),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    let hover_factor = crate::ui::motion::factor(
        ui.ctx(),
        response.id.with("hover"),
        response.hovered(),
        crate::ui::motion::Role::Quick,
    );

    // Selected is a tinted plate rather than a solid one: a solid accent block
    // at this size fights the viewport for attention every frame it is on.
    if selected {
        ui.painter()
            .rect_filled(rect, 6, accent.gamma_multiply(0.22));
    } else if hover_factor > 0.0 {
        ui.painter()
            .rect_filled(rect, 6, visuals.faint_bg_color.gamma_multiply(hover_factor));
    }

    let semantic = theme.icon_color(crate::ui::icons::role(icon));
    let color = if !enabled {
        semantic.gamma_multiply(0.32)
    } else if selected {
        semantic
    } else {
        semantic.gamma_multiply(0.82)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(16.0),
        color,
    );

    response.on_hover_text(hover).clicked()
}
