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
//! # Undo
//!
//! A plugin's commands are applied to a throwaway `Edit`, taken off its history
//! and pushed onto the editor's as **one** `Group`. One click of a panel button
//! can invoke five verbs, and five presses of Ctrl-Z to take back one click is
//! not undo, it is a puzzle.
//!
//! # The document round trip
//!
//! `Host::build_panel` wants an `Edit`, which owns its `Document`; `AppState`
//! owns the live one. Builds move it into an `Edit` and immediately return it.
//! Actions take one explicit snapshot for a background worker so the renderer
//! keeps the live document; this is per gesture, never per frame.

use crate::app_state::AppState;
use ankhimate_plugins::Host;
use ankhimate_plugins::panel::{PanelAction, PickKind, Widget};
use eframe::egui;

/// What each panel is showing, and the revision it was built at.
#[derive(Default)]
pub struct PanelCache {
    built: std::collections::HashMap<String, (u64, Vec<Widget>)>,
    /// Every widget's current value per panel, by action name.
    ///
    /// **The host owns this, not the plugin.** A fresh JS runtime is built per
    /// call so a plugin cannot hold state across an undo — which also means a
    /// handler writing `this.name = value` finds it gone next call. The first
    /// plugin written against this API did exactly that and created nine bones
    /// all named `new_bone`.
    ///
    /// Kept in `Session` with the rest of the cache: a half-typed field is not
    /// part of the rig and has no business in a save file.
    state: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>,
    job: Option<PanelJob>,
}

struct PanelJob {
    panel: String,
    action: String,
    revision: u64,
    receiver: std::sync::mpsc::Receiver<PanelJobResult>,
}

struct PanelJobResult {
    edit: ankhimate_document::Edit,
    effect: Result<ankhimate_plugins::panel::PanelEffect, String>,
}

impl PanelCache {
    /// Forget one panel's widgets, so the next frame rebuilds it.
    ///
    /// Called after an action, because a handler that changed nothing in the
    /// document still changed what the panel should show — a mode the plugin
    /// tracks itself, for instance.
    pub fn invalidate(&mut self, id: &str) {
        // The widget list goes; the values stay. A panel rebuilt after a click
        // that dropped what the user had typed would be a form that clears
        // itself every time it is used.
        self.built.remove(id);
    }
}

/// Draw the panel `id`, and act on whatever the user touched.
pub fn ui(ui: &mut egui::Ui, state: &mut AppState, plugins: &crate::plugins::Plugins, id: &str) {
    poll_job(state);

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

    let mut widgets = state
        .session
        .panels
        .built
        .get(id)
        .map(|(_, widgets)| widgets.clone())
        .unwrap_or_default();
    if let Some(values) = state.session.panels.state.get(id) {
        for widget in &mut widgets {
            if let Widget::Choice { on, value, .. } = widget
                && let Some(serde_json::Value::String(saved)) = values.get(on)
            {
                *value = saved.clone();
            }
        }
    }

    // Thumbnails are resolved before drawing, because loading one needs `&mut
    // AppState` and drawing takes `&AppState`. Done once per frame for the whole
    // panel rather than per widget, so a strip of forty images is forty cache
    // hits and not forty borrows fought over.
    let textures = resolve_thumbnails(ui.ctx(), state, &widgets);

    // Collected rather than acted on in place: a handler dispatches commands,
    // and mutating the document while walking the list that document produced is
    // how a panel draws half of one revision and half of the next.
    let mut touched = None;
    for widget in &widgets {
        if let Some(action) = draw(ui, state, &textures, widget) {
            touched = Some(action);
        }
    }

    if state.session.panels.job.is_some() {
        let phase = ((ui.input(|input| input.time) * 0.7) % 1.0) as f32;
        ui.add_space(8.0);
        ui.label("Importing item…");
        ui.add(egui::ProgressBar::new(phase));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        return;
    }

    if let Some(mut action) = touched {
        let transient = widgets
            .iter()
            .any(|widget| matches!(widget, Widget::File { on, .. } if on == &action.action));
        // Remembered before the handler runs, so a plugin reading `state.name`
        // sees what the user just typed rather than the value from last time.
        let state_for_panel = state
            .session
            .panels
            .state
            .entry(id.to_string())
            .or_default();
        if !transient && !action.value.is_null() {
            state_for_panel.insert(action.action.clone(), action.value.clone());
        }
        action.state = state_for_panel.clone();

        start_job(state, source, id.to_string(), action);
    }
}

fn start_job(state: &mut AppState, source: String, panel: String, action: PanelAction) {
    let revision = state.revision;
    let action_name = action.action.clone();
    let mode = match state.session.work_mode {
        crate::session::WorkMode::Setup => ankhimate_document::WorkMode::Setup,
        crate::session::WorkMode::Animate => ankhimate_document::WorkMode::Animate,
    };
    // The live document stays available to the renderer while the isolated
    // snapshot is edited. The result is accepted only if that source revision
    // is still current, so concurrent authoring can never be overwritten.
    let mut edit = ankhimate_document::Edit::new(state.doc.clone());
    edit.mode = mode;
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_panel = panel.clone();
    std::thread::spawn(move || {
        let effect = Host::new()
            .panel_action(&source, &worker_panel, &action, &mut edit)
            .map_err(|error| error.to_string());
        let _ = sender.send(PanelJobResult { edit, effect });
    });
    state.session.panels.job = Some(PanelJob {
        panel,
        action: action_name,
        revision,
        receiver,
    });
}

fn poll_job(state: &mut AppState) {
    let received = match state
        .session
        .panels
        .job
        .as_ref()
        .map(|job| job.receiver.try_recv())
    {
        Some(Ok(result)) => Some(Ok(result)),
        Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => Some(Err(())),
        Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => None,
    };
    let Some(received) = received else { return };
    let job = state.session.panels.job.take().expect("job was polled");
    let Ok(result) = received else {
        state
            .session
            .set_status("Plugin worker stopped unexpectedly");
        return;
    };
    if state.revision != job.revision {
        state.session.set_status(format!(
            "`{}` import was not applied because the rig changed while it loaded",
            job.panel
        ));
        state.session.panels.invalidate(&job.panel);
        return;
    }
    return_document(
        state,
        result.edit,
        &format!("{}: {}", job.panel, job.action),
    );
    match result.effect {
        Ok(effect) => apply_effect(state, effect),
        Err(error) => state
            .session
            .set_status(format!("`{}`: {error}", job.panel)),
    }
    state.session.panels.invalidate(&job.panel);
}

fn apply_effect(state: &mut AppState, effect: ankhimate_plugins::panel::PanelEffect) {
    let slots: std::collections::HashMap<String, _> = state
        .doc
        .skeleton
        .slots
        .iter()
        .map(|(slot, data)| (data.name.clone(), slot))
        .collect();
    for (name, visible) in effect.slot_visibility {
        if let Some(slot) = slots.get(&name).copied() {
            if visible {
                state.session.hidden_slots.remove(&slot);
            } else {
                state.session.hidden_slots.insert(slot);
            }
        }
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
    // A build should not edit, but a plugin that does anyway must not have its
    // change vanish — the label says where it came from.
    return_document(state, edit, &format!("{id}: build"));

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
fn return_document(state: &mut AppState, mut edit: ankhimate_document::Edit, label: &str) {
    // The plugin's commands are already applied to the document that is coming
    // back, so they are pushed as applied rather than dispatched again.
    //
    // Grouped, because one click of a panel button can invoke five verbs and
    // five presses of Ctrl-Z to take back one click is not undo, it is a
    // puzzle.
    let applied = edit.history.take_applied();
    state.doc = edit.doc;

    if !applied.is_empty() {
        let group = ankhimate_document::commands::Group::new(applied, label.to_string());
        state.dispatch_applied(Box::new(group));
    }
}

/// Draw one widget, returning what it produced if the user touched it.
fn draw(
    ui: &mut egui::Ui,
    state: &AppState,
    textures: &Textures,
    widget: &Widget,
) -> Option<PanelAction> {
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
                ..Default::default()
            })
        }
        Widget::File {
            file,
            on,
            extensions,
            multiple,
            tooltip,
        } => {
            let response = ui.button(file);
            let response = match tooltip {
                Some(text) => response.on_hover_text(text),
                None => response,
            };
            if !response.clicked() {
                return None;
            }
            let mut dialog = rfd::FileDialog::new().set_title(file);
            if !extensions.is_empty() {
                let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
                dialog = dialog.add_filter(file, &extensions);
            }
            let paths = if *multiple {
                dialog.pick_files().unwrap_or_default()
            } else {
                dialog.pick_file().into_iter().collect()
            };
            if paths.is_empty() {
                return None;
            }
            let files: Vec<serde_json::Value> = paths
                .into_iter()
                .filter_map(|path| {
                    let bytes = std::fs::read(&path).ok()?;
                    Some(serde_json::json!({
                        "name": path.file_name()?.to_str()?,
                        "bytes_base64": ankhimate_plugins::importer::encode_base64(&bytes),
                    }))
                })
                .collect();
            Some(PanelAction {
                action: on.clone(),
                value: serde_json::Value::Array(files),
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
            })
        }
        Widget::List {
            list,
            on,
            selected,
            rows,
        } => {
            // A row is the text plus the button padding either side of it —
            // `text_style_height` alone is the glyphs, and a height computed
            // from it shows seven rows in the space eight were asked for.
            let row_height = ui.text_style_height(&egui::TextStyle::Body)
                + ui.spacing().button_padding.y * 2.0
                + ui.spacing().item_spacing.y;
            let wanted = wanted_rows(*rows, list.len());
            let mut picked = None;

            egui::ScrollArea::vertical()
                .id_salt(("plugin_list", on))
                // `auto_shrink` off across the width, or the area is only as
                // wide as its longest row and a list of short names sits in a
                // narrow column against the left edge of a wide panel.
                .auto_shrink([false, true])
                .max_height(wanted as f32 * row_height)
                .show(ui, |ui| {
                    // And the rows fill it. A `selectable_label` sizes to its
                    // text, so without this the highlight is a ragged edge
                    // rather than a row.
                    ui.set_min_width(ui.available_width());
                    for (index, row) in list.iter().enumerate() {
                        let response = ui.add_sized(
                            [ui.available_width(), row_height],
                            egui::SelectableLabel::new(*selected == Some(index), row),
                        );
                        if response.clicked() {
                            picked = Some(index);
                        }
                    }
                });
            picked.map(|index| PanelAction {
                action: on.clone(),
                value: serde_json::json!(index),
                ..Default::default()
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
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for name in thumbnails {
                            let chosen = selected.as_deref() == Some(name.as_str());
                            if thumbnail(ui, textures, name, side, chosen) {
                                picked = Some(name.clone());
                            }
                        }
                    });
                });
            picked.map(|name| PanelAction {
                action: on.clone(),
                value: serde_json::Value::String(name),
                ..Default::default()
            })
        }
    }
}

/// Loaded thumbnails for this frame, by asset name.
type Textures = std::collections::HashMap<String, egui::TextureHandle>;

/// Load every thumbnail the panel's widgets ask for.
///
/// Up front rather than during the draw: `scaled_thumbnail` needs `&mut
/// AppState` to fill its cache and the draw has only a shared borrow. Doing it
/// once for the whole panel also means a strip of forty images is forty cache
/// hits rather than forty separate decisions about whether to decode.
fn resolve_thumbnails(ctx: &egui::Context, state: &mut AppState, widgets: &[Widget]) -> Textures {
    let wanted: Vec<String> = widgets
        .iter()
        .filter_map(|w| match w {
            Widget::Thumbnails { thumbnails, .. } => Some(thumbnails.clone()),
            _ => None,
        })
        .flatten()
        .collect();

    let mut textures = Textures::new();
    for name in wanted {
        if textures.contains_key(&name) {
            continue;
        }
        let Some(id) = state.doc.assets.by_name(&name) else {
            // A name with no image behind it. Left absent rather than defaulted,
            // so the widget can say the art is missing instead of drawing a
            // placeholder that looks like art that failed to load.
            continue;
        };
        if let Some(handle) = crate::ui::assets::scaled_thumbnail(ctx, state, id, 128) {
            textures.insert(name, handle);
        }
    }
    textures
}

/// How many rows of height a list should reserve.
///
/// Capped at what is actually there — eight rows of blank under three names is
/// a panel that looks broken — and floored at one, because a zero-height scroll
/// area is invisible and an empty list nobody can see reads as a widget that
/// failed rather than one with nothing in it.
fn wanted_rows(rows: Option<usize>, count: usize) -> usize {
    rows.unwrap_or(8).min(count.max(1))
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
fn thumbnail(
    ui: &mut egui::Ui,
    textures: &Textures,
    name: &str,
    side: f32,
    selected: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());

    match textures.get(name) {
        Some(handle) => {
            // Fitted rather than stretched: a tall sprite squashed into a square
            // is a picker showing art that is not what will be drawn.
            let size = handle.size_vec2();
            let scale = (side / size.x).min(side / size.y);
            let fitted = egui::Rect::from_center_size(rect.center(), size * scale);
            ui.painter().image(
                handle.id(),
                fitted,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            // A name the asset library does not have. The frame and a mark, so
            // it reads as "this art is missing" rather than as a blank square
            // that failed to paint.
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(side * 0.4),
                ui.visuals().weak_text_color(),
            );
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_visibility_effect_is_transient_session_state() {
        use ankhimate_core::{math::Transform, skeleton::Bone, slot::Slot};

        let mut state = AppState::default();
        let bone = state.doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 1.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = state
            .doc
            .skeleton
            .add_slot(Slot::new("rear_item".into(), bone));

        apply_effect(
            &mut state,
            ankhimate_plugins::panel::PanelEffect {
                slot_visibility: [("rear_item".into(), false)].into(),
            },
        );
        assert!(state.session.hidden_slots.contains(&slot));
        assert_eq!(state.doc.skeleton.slots[slot].color, [1.0; 4]);

        apply_effect(
            &mut state,
            ankhimate_plugins::panel::PanelEffect {
                slot_visibility: [("rear_item".into(), true)].into(),
            },
        );
        assert!(!state.session.hidden_slots.contains(&slot));
    }

    #[test]
    fn a_panel_action_leaves_the_live_document_available_until_completion() {
        let source = r#"
          ankhimate.registerPanel({
            id: "test.background", title: "Background",
            build: () => [],
            on: () => ops.invoke("bone.create", { name: "arrived" })
          });
        "#;
        let mut state = AppState::default();
        start_job(
            &mut state,
            source.into(),
            "test.background".into(),
            PanelAction {
                action: "import".into(),
                ..Default::default()
            },
        );
        assert!(state.session.panels.job.is_some());
        assert!(
            state.doc.skeleton.bones.is_empty(),
            "live rig was not moved"
        );

        for _ in 0..200 {
            poll_job(&mut state);
            if state.session.panels.job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(state.session.panels.job.is_none());
        assert!(
            state
                .doc
                .skeleton
                .bones
                .values()
                .any(|bone| bone.name == "arrived")
        );
        assert!(
            state.history.can_undo(),
            "the async action remains undoable"
        );
    }

    #[test]
    fn a_short_list_does_not_reserve_space_for_rows_it_does_not_have() {
        // Eight rows of blank under three names is a panel that looks broken.
        assert_eq!(wanted_rows(Some(8), 3), 3);
    }

    #[test]
    fn a_long_list_stops_at_the_row_count_it_was_given() {
        // The point of `rows`: a rig with sixty bones must not push everything
        // below it off the panel.
        assert_eq!(wanted_rows(Some(8), 60), 8);
    }

    #[test]
    fn an_empty_list_still_has_a_row_of_height() {
        // A zero-height scroll area is invisible, and an empty list that cannot
        // be seen reads as a widget that failed rather than one with nothing in
        // it.
        assert_eq!(wanted_rows(Some(8), 0), 1);
    }
}
