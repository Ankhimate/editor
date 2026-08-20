//! Drawing a plugin's panel.
//!
//! The plugin returned a list of widgets (`ankhimate_plugins::panel`); this
//! paints them and hands back what the user touched. A script is never called
//! from inside the paint loop — it built the list before drawing started, and
//! its handler runs after drawing ends.
//!
//! # Why the list is cached
//!
//! `build` runs when the document changes or when one of the panel's own
//! widgets is touched, not once a frame. A JS context per panel per frame is the
//! cost the declarative design exists to avoid, and a panel's contents change
//! when the rig does — not when the compositor asks.
//!
//! The cache is keyed on `AppState::revision`, which every edit already bumps. A
//! panel showing stale numbers after an edit is the failure this has to avoid,
//! and the revision is the one counter that already means "something changed".
//!
//! # The document round trip
//!
//! `Host::build_panel` wants an `Edit`, which owns its `Document`; `AppState`
//! owns the live one and `Document` is deliberately not `Clone` — a rig with its
//! images in it is not something to copy per frame. So the document is **moved
//! into** an `Edit` for the call and moved back after, which is the same thing
//! the plugin host does internally and for the same reason.

use crate::app_state::AppState;
use ankhimate_plugins::Host;
use ankhimate_plugins::panel::{PanelAction, PickKind, Widget};
use eframe::egui;

/// What each panel is showing, and the revision it was built at.
#[derive(Default)]
pub struct PanelCache {
    built: std::collections::HashMap<String, (u64, Vec<Widget>)>,
}

impl PanelCache {
    /// Forget one panel's widgets, so the next frame rebuilds it.
    ///
    /// Called after an action, because a handler that changed nothing in the
    /// document still changed what the panel should show — a mode the plugin
    /// tracks itself, for instance.
    pub fn invalidate(&mut self, id: &str) {
        self.built.remove(id);
    }
}

/// Draw the panel `id`, and act on whatever the user touched.
pub fn ui(ui: &mut egui::Ui, state: &mut AppState, plugins: &crate::plugins::Plugins, id: &str) {
    let Some(source) = plugins.source_for_panel(id).map(str::to_string) else {
        // A panel whose plugin is no longer loaded. Saying so beats an empty
        // card, which reads as a panel that is broken rather than absent.
        ui.label(
            egui::RichText::new(format!("No loaded plugin provides `{id}`."))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };

    if let Err(e) = rebuild_if_stale(state, &source, id) {
        // The author's own message, not "the plugin failed". A panel that
        // cannot say why is one nobody can fix.
        ui.label(
            egui::RichText::new(format!("`{id}` could not be built:\n{e}"))
                .size(11.0)
                .color(ui.visuals().error_fg_color),
        );
        return;
    }

    let widgets = state
        .session
        .panels
        .built
        .get(id)
        .map(|(_, widgets)| widgets.clone())
        .unwrap_or_default();

    // Collected rather than acted on in place: a handler dispatches commands,
    // and mutating the document while walking the list that document produced is
    // how a panel draws half of one revision and half of the next.
    let mut touched = None;
    for widget in &widgets {
        if let Some(action) = draw(ui, state, widget) {
            touched = Some(action);
        }
    }

    if let Some(action) = touched {
        let host = Host::new();
        let mut edit = borrow_document(state);
        let outcome = host.panel_action(&source, id, &action, &mut edit);
        return_document(state, edit);

        if let Err(e) = outcome {
            state.session.set_status(format!("`{id}`: {e}"));
        }
        state.session.panels.invalidate(id);
    }
}

/// Rebuild the widget list if the document has moved since it was made.
fn rebuild_if_stale(
    state: &mut AppState,
    source: &str,
    id: &str,
) -> Result<(), ankhimate_plugins::PluginError> {
    let revision = state.revision;
    let fresh = state
        .session
        .panels
        .built
        .get(id)
        .is_some_and(|(built_at, _)| *built_at == revision);
    if fresh {
        return Ok(());
    }

    let host = Host::new();
    let mut edit = borrow_document(state);
    let built = host.build_panel(source, id, &mut edit);
    return_document(state, edit);

    let widgets = built?;
    state
        .session
        .panels
        .built
        .insert(id.to_string(), (revision, widgets));
    Ok(())
}

/// Move the live document into an `Edit` for a plugin call.
///
/// `Document` is not `Clone` on purpose — a rig with its images in it is not
/// something to copy per frame — so the only honest way to hand it to a host
/// that wants ownership is to move it and move it back.
fn borrow_document(state: &mut AppState) -> ankhimate_document::Edit {
    let mut edit = ankhimate_document::Edit::new(std::mem::take(&mut state.doc));
    // The mode travels with it, or a panel's handler is refused for standing in
    // Setup while the editor is in Animate.
    edit.mode = match state.session.work_mode {
        crate::session::WorkMode::Setup => ankhimate_document::WorkMode::Setup,
        crate::session::WorkMode::Animate => ankhimate_document::WorkMode::Animate,
    };
    edit
}

/// Put it back, and fold anything the plugin did into the editor's own history.
fn return_document(state: &mut AppState, edit: ankhimate_document::Edit) {
    let changed = edit.history.can_undo();
    state.doc = edit.doc;
    if changed {
        // The plugin's commands went onto the `Edit`'s own history, which is
        // discarded here. Bumping the revision is what tells the rest of the
        // editor — and every other panel — that the rig moved.
        //
        // **Undo does not reach these yet.** A plugin's edits are visible and
        // saveable and Ctrl-Z will not take them back, which is a real gap and
        // named here rather than left to be discovered: merging two histories
        // is its own piece of work.
        state.revision = state.revision.wrapping_add(1);
        state.refresh_pose();
    }
}

/// Draw one widget, returning what it produced if the user touched it.
fn draw(ui: &mut egui::Ui, state: &AppState, widget: &Widget) -> Option<PanelAction> {
    match widget {
        Widget::Heading { heading } => {
            ui.add_space(6.0);
            crate::ui::inspector::section_header(ui, crate::ui::icons::PLUGIN, heading);
            ui.add_space(2.0);
            None
        }
        Widget::Text { text, weak } => {
            let rich = egui::RichText::new(text).size(11.0);
            ui.label(if *weak {
                rich.color(ui.visuals().weak_text_color())
            } else {
                rich
            });
            None
        }
        Widget::Separator { .. } => {
            ui.separator();
            None
        }
        Widget::Button {
            button,
            on,
            disabled,
            tooltip,
        } => {
            let response = ui.add_enabled(!*disabled, egui::Button::new(button));
            let response = match tooltip {
                Some(text) => response.on_hover_text(text),
                None => response,
            };
            response.clicked().then(|| PanelAction {
                action: on.clone(),
                value: serde_json::Value::Null,
            })
        }
        Widget::Checkbox {
            checkbox,
            on,
            value,
            tooltip,
        } => {
            // The plugin owns the value; this is a copy for the frame and the
            // answer goes back through the action. Two owners of one boolean is
            // how a tick box starts disagreeing with the thing it controls.
            let mut current = *value;
            let response = ui.checkbox(&mut current, checkbox);
            let response = match tooltip {
                Some(text) => response.on_hover_text(text),
                None => response,
            };
            response.changed().then(|| PanelAction {
                action: on.clone(),
                value: serde_json::Value::Bool(current),
            })
        }
        Widget::Number {
            number,
            on,
            value,
            min,
            max,
            speed,
            tooltip,
        } => {
            let mut current = *value;
            let mut drag = egui::DragValue::new(&mut current);
            if let (Some(low), Some(high)) = (min, max) {
                drag = drag.range(*low..=*high);
            }
            if let Some(speed) = speed {
                drag = drag.speed(*speed);
            }
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(number);
                let response = ui.add(drag);
                let response = match tooltip {
                    Some(text) => response.on_hover_text(text),
                    None => response,
                };
                changed = response.changed();
            });
            changed.then(|| PanelAction {
                action: on.clone(),
                value: serde_json::json!(current),
            })
        }
        Widget::Text_ {
            field,
            on,
            value,
            tooltip,
        } => {
            let mut current = value.clone();
            let mut done = false;
            ui.horizontal(|ui| {
                ui.label(field);
                let response = ui.text_edit_singleline(&mut current);
                let response = match tooltip {
                    Some(text) => response.on_hover_text(text),
                    None => response,
                };
                // On focus loss rather than per keystroke: a handler that runs a
                // verb per character typed is a rig edited letter by letter.
                done = response.lost_focus() && current != *value;
            });
            done.then(|| PanelAction {
                action: on.clone(),
                value: serde_json::Value::String(current),
            })
        }
        Widget::Choice {
            choice,
            on,
            options,
            value,
            tooltip,
        } => dropdown(ui, choice, value, options, tooltip.as_deref()).map(|picked| PanelAction {
            action: on.clone(),
            value: serde_json::Value::String(picked),
        }),
        Widget::Pick {
            pick,
            on,
            of,
            value,
            tooltip,
        } => {
            // The host fills the list, from the open document. A plugin that
            // built it would show names that go stale the moment a bone is
            // renamed, and every plugin would keep it fresh differently.
            let options = names_of(state, *of);
            dropdown(ui, pick, value, &options, tooltip.as_deref()).map(|picked| PanelAction {
                action: on.clone(),
                value: serde_json::Value::String(picked),
            })
        }
        Widget::List {
            list,
            on,
            selected,
            rows,
        } => {
            let row_height = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            let mut picked = None;
            egui::ScrollArea::vertical()
                .id_salt(("plugin_list", on))
                .max_height(rows.unwrap_or(8) as f32 * row_height)
                .show(ui, |ui| {
                    for (index, row) in list.iter().enumerate() {
                        if ui.selectable_label(*selected == Some(index), row).clicked() {
                            picked = Some(index);
                        }
                    }
                });
            picked.map(|index| PanelAction {
                action: on.clone(),
                value: serde_json::json!(index),
            })
        }
        Widget::Thumbnails {
            thumbnails,
            on,
            selected,
            size,
        } => {
            let side = size.unwrap_or(48.0).clamp(24.0, 128.0);
            let mut picked = None;
            egui::ScrollArea::horizontal()
                .id_salt(("plugin_thumbs", on))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for name in thumbnails {
                            let chosen = selected.as_deref() == Some(name.as_str());
                            if thumbnail(ui, state, name, side, chosen) {
                                picked = Some(name.clone());
                            }
                        }
                    });
                });
            picked.map(|name| PanelAction {
                action: on.clone(),
                value: serde_json::Value::String(name),
            })
        }
    }
}

/// A labelled dropdown. Returns the newly picked option, if it changed.
fn dropdown(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    options: &[String],
    tooltip: Option<&str>,
) -> Option<String> {
    let mut picked = None;
    ui.horizontal(|ui| {
        let response = ui.label(label);
        if let Some(text) = tooltip {
            response.on_hover_text(text);
        }
        egui::ComboBox::from_id_salt(("plugin_choice", label))
            .selected_text(if value.is_empty() { "—" } else { value })
            .show_ui(ui, |ui| {
                for option in options {
                    if ui.selectable_label(option == value, option).clicked() && option != value {
                        picked = Some(option.clone());
                    }
                }
            });
    });
    picked
}

/// What the rig has of one kind, for a [`Widget::Pick`].
fn names_of(state: &AppState, kind: PickKind) -> Vec<String> {
    match kind {
        PickKind::Bone => state
            .doc
            .skeleton
            .bones
            .iter()
            .map(|(_, b)| b.name.clone())
            .collect(),
        PickKind::Slot => state
            .doc
            .skeleton
            .slots
            .iter()
            .map(|(_, s)| s.name.clone())
            .collect(),
        PickKind::Animation => state
            .doc
            .animations
            .iter()
            .map(|(_, a)| a.name.clone())
            .collect(),
        PickKind::Skin => state
            .doc
            .skeleton
            .skins
            .iter()
            .map(|(_, s)| s.name.clone())
            .collect(),
        PickKind::Image => state
            .doc
            .assets
            .images
            .values()
            .map(|a| a.name.clone())
            .collect(),
    }
}

/// One asset thumbnail. Returns true when clicked.
///
/// The image is not drawn yet: resolving an asset name to a live egui texture
/// needs the cache the assets panel keeps, and reaching into it from here would
/// be a second owner of that cache. The frame, the name and the click all work,
/// so a plugin using this widget gets a picker that picks — it just picks by
/// name rather than by sight, which is stated rather than left to be noticed.
fn thumbnail(ui: &mut egui::Ui, _state: &AppState, name: &str, side: f32, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());

    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let short: String = name.chars().take(6).collect();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        short,
        egui::FontId::proportional(side * 0.22),
        ui.visuals().weak_text_color(),
    );

    if selected {
        ui.painter().rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(name).clicked()
}
