//! The export panel (T-603d).
//!
//! Three columns of one job: pick a preset, edit its templates, see what they
//! will write. The preview is what makes the feature learnable — authoring a
//! format blind and finding the mistake in an engine's import error is a loop
//! long enough that users give up. It renders the **real** context through the
//! real pipeline, never a sample: a preview that can disagree with the export is
//! worse than none.
//!
//! The context browser is not decoration either. Without it the only way to
//! learn what a template can address is the reference document, and users do not
//! read reference documents while typing.

use crate::app_state::AppState;
use ankhimate_document::commands::export_cmds::{
    AddPreset, EditTemplate, RemovePreset, SetPresetOptions,
};
use ankhimate_export::preset::{Cadence, Preset, Template};
use ankhimate_export::presets;
use ankhimate_export::run::{self, Plan};
use eframe::egui;

/// Which preset and template the panel is looking at.
///
/// Session state: which row is selected is not a property of the rig, and it
/// would be wrong to find it in a teammate's file.
#[derive(Debug, Default)]
pub struct ExportUiState {
    pub preset: usize,
    pub template: usize,
    pub show_context: bool,
    /// Last error, kept so it stays on screen after the frame that produced it.
    pub error: Option<String>,
    pub last_export: Option<String>,
    /// The last preview, and what it was rendered from.
    ///
    /// The preview runs the **real** pipeline, which bakes an atlas — decoding
    /// and re-encoding every image — and renders every template for every clip.
    /// That is right for fidelity and ruinous per frame: at 60fps it re-encodes
    /// megabytes of PNG a second, and egui repaints continuously while a text
    /// field has focus, so it fired on every keystroke in the template editor.
    /// Keyed on the document revision and the selected preset, it runs once per
    /// actual change instead.
    cache: Option<CachedPreview>,
}

/// A rendered preview plus the inputs that produced it.
#[derive(Debug)]
struct CachedPreview {
    revision: u64,
    preset: usize,
    /// The plan, or the error rendering it — both are worth not recomputing,
    /// and an error that flickers because it is recomputed mid-keystroke reads
    /// as a different bug than the one it is.
    result: Result<Plan, String>,
    /// Built alongside, for the context browser: it needs the same schema
    /// conversion, and doing it separately doubled the cost whenever the browser
    /// was open.
    context: serde_json::Value,
}

pub fn ui(ui: &mut egui::Ui, state: &mut AppState, plugins: &crate::plugins::Plugins) {
    let presets_list = state.doc.presets();

    ui.label(egui::RichText::new("Export").strong());
    ui.columns(3, |columns| {
        let button_width = columns[0].available_width();
        egui::containers::menu::MenuButton::from_button(
            egui::Button::new(format!("{}  Preset", crate::ui::icons::ADD))
                .min_size(egui::vec2(button_width, crate::ui::CONTROL_HEIGHT)),
        )
        .ui(&mut columns[0], |ui| {
            for builtin in presets::builtin() {
                if ui.button(&builtin.name).clicked() {
                    state.dispatch(Box::new(AddPreset::new(builtin)));
                    ui.close();
                }
            }
            // Plugin formats, beside the built-in presets. A plugin
            // exporter is a format like any other, and a separate menu for
            // "plugin formats" would be the duplication the registry exists
            // to remove.
            //
            // Added as a preset carrying only the plugin's label: the panel
            // resolves that label back to the exporter when the export runs,
            // so a document saved with one still names what produced it even
            // on a machine where the plugin is not installed.
            let plugin_exporters = plugins.exporters();
            if !plugin_exporters.is_empty() {
                ui.separator();
                for (_, label) in plugin_exporters {
                    if ui.button(&label).clicked() {
                        state.dispatch(Box::new(AddPreset::new(Preset::new(label))));
                        ui.close();
                    }
                }
            }

            ui.separator();
            if ui.button("Empty preset").clicked() {
                let mut blank = Preset::new("New format");
                blank.templates.push(Template {
                    name: "output".into(),
                    output_path: "{{project.name}}.json".into(),
                    per: Cadence::Once,
                    body: "{\n  \"name\": \"{{project.name}}\"\n}\n".into(),
                });
                state.dispatch(Box::new(AddPreset::new(blank)));
                ui.close();
            }
        });
        if columns[1]
            .add_enabled(
                !presets_list.is_empty(),
                egui::Button::new(format!("{}  Delete", crate::ui::icons::DELETE))
                    .min_size(egui::vec2(button_width, crate::ui::CONTROL_HEIGHT)),
            )
            .clicked()
        {
            let index = state.session.export.preset;
            state.dispatch(Box::new(RemovePreset::new(index)));
            state.session.export.preset = index.saturating_sub(1);
        }
        if columns[2]
            .add_sized(
                [button_width, crate::ui::CONTROL_HEIGHT],
                egui::Button::new(format!("{}  Export…", crate::ui::icons::DOWNLOAD)),
            )
            .on_hover_text("Render every template and write the file set")
            .clicked()
        {
            run_export(state, plugins, &presets_list);
        }
    });

    if presets_list.is_empty() {
        ui.add_space(8.0);
        ui.label("No export presets yet.");
        ui.label(
            egui::RichText::new(
                "A preset is a format: an atlas setting plus one or more templates that say \
                 what the files look like. Start from a built-in and edit it.",
            )
            .weak(),
        );
        return;
    }

    let index = state.session.export.preset.min(presets_list.len() - 1);
    state.session.export.preset = index;

    ui.separator();
    egui::ComboBox::from_id_salt("export_preset")
        .selected_text(&presets_list[index].name)
        .show_ui(ui, |ui| {
            for (i, preset) in presets_list.iter().enumerate() {
                ui.selectable_value(&mut state.session.export.preset, i, &preset.name);
            }
        });

    let preset = &presets_list[state.session.export.preset.min(presets_list.len() - 1)];
    if !preset.description.is_empty() {
        ui.label(egui::RichText::new(&preset.description).weak().small());
    }

    ui.add_space(4.0);
    options(ui, state, preset);
    ui.separator();
    templates(ui, state, preset);

    if let Some(error) = state.session.export.error.clone() {
        ui.add_space(4.0);
        ui.colored_label(egui::Color32::from_rgb(220, 90, 90), error);
    }
    if let Some(done) = state.session.export.last_export.clone() {
        ui.add_space(4.0);
        ui.colored_label(egui::Color32::from_rgb(120, 190, 120), done);
    }
}

fn options(ui: &mut egui::Ui, state: &mut AppState, preset: &Preset) {
    let mut edited = preset.clone();
    let mut changed = false;

    crate::ui::inspector::section_header(ui, crate::ui::icons::EXPORT, "Output settings");
    egui::Grid::new("export_options")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            crate::ui::form_label(ui, "Output folder");
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut edited.output_dir)
                        .desired_width(ui.available_width().max(180.0)),
                )
                .changed();
            ui.end_row();

            crate::ui::form_label(ui, "Atlas");
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut edited.atlas.enabled, "Bake").changed();
                if edited.atlas.enabled {
                    changed |= ui
                        .checkbox(&mut edited.atlas.settings.trim, "Trim")
                        .on_hover_text("Cut transparent borders and record the offset")
                        .changed();
                }
            });
            ui.end_row();

            if edited.atlas.enabled {
                crate::ui::form_label(ui, "Padding");
                changed |= ui
                    .add(egui::DragValue::new(&mut edited.atlas.settings.padding).range(0..=32))
                    .on_hover_text("Empty pixels between regions")
                    .changed();
                ui.end_row();

                crate::ui::form_label(ui, "Extrude");
                changed |= ui
                    .add(egui::DragValue::new(&mut edited.atlas.settings.extrude).range(0..=8))
                    .on_hover_text(
                        "Edge pixels duplicated outward. Without this a renderer sampling at \
                         non-integer zoom reaches past the region and every sprite gets a halo.",
                    )
                    .changed();
                ui.end_row();

                crate::ui::form_label(ui, "Max page");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut edited.atlas.settings.max_page)
                            .range(64..=8192)
                            .speed(16.0),
                    )
                    .changed();
                ui.end_row();

                crate::ui::form_label(ui, "Power of two");
                changed |= ui
                    .checkbox(&mut edited.atlas.settings.power_of_two, "")
                    .changed();
                ui.end_row();
            }
        });

    if changed {
        let index = state.session.export.preset;
        state.dispatch(Box::new(SetPresetOptions::new(index, edited)));
    }
}

fn templates(ui: &mut egui::Ui, state: &mut AppState, preset: &Preset) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Templates").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(&mut state.session.export.show_context, "Context")
                .on_hover_text("What a template can address");
        });
    });

    if preset.templates.is_empty() {
        ui.label(egui::RichText::new("This preset has no templates.").weak());
        return;
    }

    let template_index = state
        .session
        .export
        .template
        .min(preset.templates.len() - 1);
    state.session.export.template = template_index;

    ui.horizontal_wrapped(|ui| {
        for (i, template) in preset.templates.iter().enumerate() {
            ui.selectable_value(&mut state.session.export.template, i, &template.name);
        }
    });

    let template = &preset.templates[state
        .session
        .export
        .template
        .min(preset.templates.len() - 1)];
    let mut edited = template.clone();
    let mut changed = false;

    ui.horizontal(|ui| {
        crate::ui::form_label(ui, "Writes");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut edited.output_path).desired_width(220.0))
            .on_hover_text("The output path is a template too")
            .changed();
        let mut per_animation = edited.per == Cadence::Animation;
        if ui
            .checkbox(&mut per_animation, "per animation")
            .on_hover_text("Render once per clip, with {{animation}} bound to it")
            .changed()
        {
            edited.per = if per_animation {
                Cadence::Animation
            } else {
                Cadence::Once
            };
            changed = true;
        }
    });

    // The buffer lives in `ui.data` while editing. A text field rebuilt from the
    // document every frame swallows keystrokes — a trap this repo has already
    // paid for once (see CLAUDE.md).
    let buffer_id = egui::Id::new((
        "export_template_body",
        state.session.export.preset,
        state.session.export.template,
    ));
    let mut body = ui
        .data(|d| d.get_temp::<String>(buffer_id))
        .unwrap_or_else(|| edited.body.clone());

    let response = ui.add(
        egui::TextEdit::multiline(&mut body)
            .code_editor()
            .desired_rows(12)
            .desired_width(f32::INFINITY),
    );
    if response.changed() {
        ui.data_mut(|d| d.insert_temp(buffer_id, body.clone()));
        edited.body = body.clone();
        changed = true;
    } else if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(buffer_id));
    }

    if changed {
        let preset_index = state.session.export.preset;
        let template_index = state.session.export.template;
        state.dispatch(Box::new(EditTemplate::new(
            preset_index,
            template_index,
            edited,
        )));
    }

    ui.add_space(6.0);
    preview(ui, state, preset);

    if state.session.export.show_context {
        ui.separator();
        context_browser(ui, state, preset);
    }
}

/// Rebuild the cached preview if the rig or the selected preset has moved on.
///
/// Everything expensive in this panel happens here, exactly once per change.
fn refresh_cache(state: &mut AppState, preset: &Preset) {
    let revision = state.revision;
    let index = state.session.export.preset;
    let fresh = state
        .session
        .export
        .cache
        .as_ref()
        .is_some_and(|c| c.revision == revision && c.preset == index);
    if fresh {
        return;
    }

    // One schema conversion feeds both the plan and the context browser.
    let schema = ankhimate_formats::convert::to_schema(&state.doc.as_project_ref());
    // `Named`: the preview shows a file count and the rendered text, never the
    // pixels, so the atlas is baked but its pages are not PNG-compressed.
    let result = run::plan_with(&schema, &state.doc.assets, preset, run::Images::Named)
        .map_err(|e| e.to_string());
    let info = ankhimate_export::context::ExportInfo {
        output_dir: preset.output_dir.clone(),
        preset_name: preset.name.clone(),
        template_name: String::new(),
        atlas_stem: preset.atlas.stem.clone(),
    };
    let context = ankhimate_export::context::Context::build(&schema, None, info).to_value();

    state.session.export.cache = Some(CachedPreview {
        revision,
        preset: index,
        result,
        context,
    });
}

/// Show what the preset would write, without writing anything.
fn preview(ui: &mut egui::Ui, state: &mut AppState, preset: &Preset) {
    refresh_cache(state, preset);
    let Some(cache) = &state.session.export.cache else {
        return;
    };

    let plan = match &cache.result {
        Ok(plan) => plan,
        Err(e) => {
            ui.colored_label(egui::Color32::from_rgb(220, 90, 90), e);
            return;
        }
    };

    summary(ui, plan);
    // Show the first file this template produced. For a per-animation template
    // that is one of many, which is the point — the user sees the shape, not
    // every copy.
    if let Some(file) = plan
        .files
        .iter()
        .find(|f| f.path.ends_with(".json") || f.path.ends_with(".txt") || !f.path.contains('.'))
    {
        ui.collapsing(format!("Preview — {}", file.path), |ui| {
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .id_salt("export_preview")
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut file.contents.as_str())
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });
        });
    }
}

fn summary(ui: &mut egui::Ui, plan: &Plan) {
    let files = plan.files.len();
    let pages = plan.binaries.len();
    ui.label(
        egui::RichText::new(format!(
            "{files} file{} + {pages} image{}",
            if files == 1 { "" } else { "s" },
            if pages == 1 { "" } else { "s" }
        ))
        .weak()
        .small(),
    );
}

/// The fields a template can address, at a glance.
fn context_browser(ui: &mut egui::Ui, state: &mut AppState, preset: &Preset) {
    refresh_cache(state, preset);
    let Some(cache) = &state.session.export.cache else {
        return;
    };
    let value = &cache.context;

    ui.label(egui::RichText::new("Context").strong());
    ui.label(
        egui::RichText::new("Values from the open rig. Click a path to copy it.")
            .weak()
            .small(),
    );
    egui::ScrollArea::vertical()
        .max_height(260.0)
        .id_salt("export_context")
        .show(ui, |ui| {
            node(ui, "", value);
        });
}

fn node(ui: &mut egui::Ui, path: &str, value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match child {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        egui::CollapsingHeader::new(key)
                            .id_salt(&child_path)
                            .show(ui, |ui| node(ui, &child_path, child));
                    }
                    _ => leaf(ui, key, &child_path, child),
                }
            }
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                ui.label(egui::RichText::new("(empty)").weak().small());
            }
            // One element is enough to show the shape; listing 200 bones here
            // would bury the field names the browser exists to surface.
            if let Some(first) = items.first() {
                ui.label(
                    egui::RichText::new(format!("{} item(s), each:", items.len()))
                        .weak()
                        .small(),
                );
                node(ui, &format!("{path}.[]"), first);
            }
        }
        _ => leaf(ui, path, path, value),
    }
}

fn leaf(ui: &mut egui::Ui, label: &str, path: &str, value: &serde_json::Value) {
    let text = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let shown = if text.chars().count() > 40 {
        format!("{}…", text.chars().take(40).collect::<String>())
    } else {
        text
    };
    let response = ui.selectable_label(false, format!("{label} = {shown}"));
    if response.clicked() {
        ui.ctx().copy_text(format!("{{{{{path}}}}}"));
    }
    response.on_hover_text(format!("{{{{{path}}}}}"));
}

fn run_export(state: &mut AppState, plugins: &crate::plugins::Plugins, presets_list: &[Preset]) {
    state.session.export.error = None;
    state.session.export.last_export = None;

    let Some(preset) = presets_list.get(state.session.export.preset) else {
        return;
    };
    let Some(dir) = rfd::FileDialog::new().set_title("Export to").pick_folder() else {
        return;
    };

    // A plugin exporter is chosen by name, the same way a preset is. It renders
    // rather than writes — `run::write` owns path confinement, all-or-nothing
    // and never-delete, and those are `docs/export-plan.md`'s rules rather than
    // a plugin author's to re-implement.
    if let Some(exporter) = plugins.exporter_named(&preset.name) {
        run_plugin_export(state, exporter, &dir);
        return;
    }

    let schema = ankhimate_formats::convert::to_schema(&state.doc.as_project_ref());
    match run::export(&schema, &state.doc.assets, preset, &dir) {
        Ok(plan) => {
            state.session.export.last_export = Some(format!(
                "Wrote {} file(s) and {} image(s) to {}",
                plan.files.len(),
                plan.binaries.len(),
                dir.display()
            ));
        }
        Err(e) => state.session.export.error = Some(e.to_string()),
    }
}

/// Run a plugin exporter and write what it produced.
///
/// The document is moved into the exporter and moved back, because
/// `JsExporter::plan` takes ownership: `Document` is not `Clone` — a rig with
/// its images in it is not something to copy per export — and an exporter has no
/// business editing, so it gets the document and hands it straight back.
fn run_plugin_export(
    state: &mut AppState,
    exporter: &ankhimate_plugins::exporter::JsExporter,
    dir: &std::path::Path,
) {
    let doc = std::mem::take(&mut state.doc);
    let outcome = exporter.plan(doc);

    match outcome {
        Ok((plan, doc)) => {
            state.doc = doc;
            match run::write(&plan, dir) {
                Ok(()) => {
                    state.session.export.last_export = Some(format!(
                        "Wrote {} file(s) and {} image(s) to {}",
                        plan.files.len(),
                        plan.binaries.len(),
                        dir.display()
                    ));
                }
                Err(e) => state.session.export.error = Some(e.to_string()),
            }
        }
        Err((e, doc)) => {
            // The document comes back on this path too, which is why `plan`
            // returns it there: losing a user's whole rig to a plugin's typo
            // would be the worst bug in the program.
            state.doc = doc;
            state.session.export.error = Some(e.to_string());
        }
    }
}
