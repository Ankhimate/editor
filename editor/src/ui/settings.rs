//! The settings window (T-701).
//!
//! Three things, all of which used to be constants somebody had to recompile to
//! change: the viewport checker, text sizes per area, and the colour scheme.
//!
//! Settings apply **live**, not on OK. A colour you cannot see until you close
//! the dialog is a colour you pick by trial and error, and the whole point of a
//! theme editor is to watch the editor change while you drag.
//!
//! Everything here lives in `Config`, which is written to the platform config
//! directory — none of it belongs to a project, and a grid size that travelled
//! in a `.ankh` would fight whoever opened it next.

use crate::app_state::AppState;
use crate::config::{Config, FontSettings};
use crate::theme::Theme;
use eframe::egui;

/// Which section is showing.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Appearance,
    Grid,
    Fonts,
    Keys,
    Saving,
}

/// Draw the window. Returns `true` when it should close.
pub fn ui(
    ctx: &egui::Context,
    state: &mut AppState,
    config: &mut Config,
    theme: &mut Theme,
    available: &mut Vec<Theme>,
    operators: &crate::commands::registry::Registry,
    open: &mut bool,
) -> bool {
    if !*open {
        return false;
    }
    let mut close = false;
    let mut section = ctx
        .memory(|m| {
            m.data
                .get_temp::<Section>(egui::Id::new("settings_section"))
        })
        .unwrap_or_default();

    // `theme` is borrowed mutably by the body, so the dialog reads its chrome
    // colours from a copy taken before the borrow starts.
    let chrome = theme.clone();
    let dialog = crate::ui::dialog::Dialog::new("settings", "Settings")
        .icon(crate::ui::icons::PROPERTIES)
        .width(520.0)
        .show(ctx, &chrome, |ui| {
            ui.horizontal(|ui| {
                for (label, icon, value) in [
                    ("Appearance", crate::ui::icons::PALETTE, Section::Appearance),
                    ("Grid", crate::ui::icons::GRID, Section::Grid),
                    ("Fonts", crate::ui::icons::STRING, Section::Fonts),
                    ("Keys", crate::ui::icons::PROPERTIES, Section::Keys),
                    ("Saving", crate::ui::icons::TIME, Section::Saving),
                ] {
                    if ui
                        .selectable_label(section == value, format!("{icon}  {label}"))
                        .clicked()
                    {
                        section = value;
                    }
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(340.0)
                .show(ui, |ui| match section {
                    Section::Appearance => appearance(ui, config, theme, available),
                    Section::Grid => grid(ui, config),
                    Section::Fonts => fonts(ui, config),
                    Section::Keys => keys(ui, config, operators),
                    Section::Saving => saving(ui, config),
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("Reset to defaults")
                        .on_hover_text("Grid and fonts only — the theme is left alone")
                        .clicked()
                    {
                        config.grid = Default::default();
                        config.fonts = Default::default();
                        // Lives on the Grid page but is not part of `GridSettings`,
                        // so resetting that struct would leave it behind — and a
                        // reset button that skips a control on its own page reads
                        // as broken.
                        config.hover_labels = Config::default().hover_labels;
                        state.session.set_status("Grid and fonts reset");
                    }
                });
            });
        });
    close |= dialog.closed;

    ctx.memory_mut(|m| {
        m.data
            .insert_temp(egui::Id::new("settings_section"), section)
    });
    if close {
        *open = false;
        config.save();
    }
    close
}

/// Autosave interval (T-701).
fn saving(ui: &mut egui::Ui, config: &mut Config) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Autosave").strong());
    ui.add_space(4.0);

    let mut on = config.autosave_secs > 0;
    if ui
        .checkbox(&mut on, "Keep a recovery copy while working")
        .on_hover_text(
            "Writes beside the project as <name>.ankh.autosave. Your own file is never \
             touched, and the copy is removed when you save.",
        )
        .changed()
    {
        config.autosave_secs = if on {
            crate::autosave::DEFAULT_INTERVAL_SECS
        } else {
            0
        };
        config.save();
    }

    ui.add_enabled_ui(on, |ui| {
        ui.horizontal(|ui| {
            ui.label("Every");
            let mut minutes = (config.autosave_secs as f32 / 60.0).max(0.5);
            if ui
                .add(
                    egui::DragValue::new(&mut minutes)
                        .speed(0.25)
                        .range(0.5..=30.0)
                        .suffix(" min"),
                )
                .changed()
            {
                config.autosave_secs = (minutes * 60.0).round() as u64;
                config.save();
            }
        });
    });

    ui.add_space(8.0);
    ui.label(
        egui::RichText::new(
            "A copy is only written when something has changed, so an idle project is \
             not rewritten on a timer.",
        )
        .weak()
        .small(),
    );
}

/// The operator currently waiting for a chord, if any.
///
/// In egui's temp memory rather than on `Config`: it is transient interaction
/// state, and a half-finished rebinding must not reach disk.
const CAPTURING: &str = "keymap_capturing";

/// Is a settings row waiting to swallow the next chord?
///
/// `App` asks before running the keymap, so the key you press to rebind does not
/// also fire the thing it was previously bound to.
pub fn capturing(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.data.get_temp::<String>(egui::Id::new(CAPTURING)))
        .is_some()
}

/// Rebindable key list (T-701).
///
/// Rows come from the operator registry, so a plugin's operators appear here
/// with no work — which is the point of the registry over an enum.
fn keys(ui: &mut egui::Ui, config: &mut Config, operators: &crate::commands::registry::Registry) {
    let mut capturing: Option<String> = ui
        .ctx()
        .memory(|m| m.data.get_temp::<String>(egui::Id::new(CAPTURING)));

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Click a shortcut, then press the keys you want. Esc cancels.")
            .weak()
            .small(),
    );
    ui.add_space(6.0);

    // While capturing, the first key event wins. Read before drawing so the row
    // shows the result in the same frame it is chosen.
    if let Some(target) = capturing.clone()
        && let Some(chord) = pressed_chord(ui.ctx())
    {
        if chord.key != egui::Key::Escape {
            config.keymap.rebind(chord, &target);
            // Written now rather than when the dialog closes. The module note
            // says config is saved on change for this reason: losing a
            // rebinding to a crash teaches you not to trust the setting.
            config.save();
        }
        capturing = None;
    }

    let mut reset: Option<String> = None;
    egui::Grid::new("keymap_grid")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            for id in operators.ids() {
                let Some(op) = operators.get(id) else {
                    continue;
                };
                ui.label(op.label());

                let is_target = capturing.as_deref() == Some(id);
                let chord = config.keymap.chord_for(id);
                let text = match (is_target, chord) {
                    (true, _) => "press keys…".to_string(),
                    (false, Some(c)) => c.label(),
                    (false, None) => "—".to_string(),
                };
                let button = egui::Button::new(text).min_size(egui::vec2(120.0, 0.0));
                if ui.add(button).clicked() {
                    capturing = Some(id.to_string());
                }

                // A chord bound to something else is a conflict worth naming
                // before the user discovers it by pressing the key.
                ui.horizontal(|ui| {
                    if ui.small_button("Reset").clicked() {
                        reset = Some(id.to_string());
                    }
                    if let Some(c) = chord
                        && let Some(other) = config.keymap.operator_for(c)
                        && other != id
                    {
                        let name = operators.get(other).map_or(other, |o| o.label());
                        ui.label(egui::RichText::new(format!("also {name}")).weak().small());
                    }
                });
                ui.end_row();
            }
        });

    if let Some(id) = reset {
        config.keymap.reset(&id);
        config.save();
    }

    // Bindings whose operator is gone — an uninstalled plugin, a renamed
    // built-in. Shown rather than deleted, so reinstalling brings the key back.
    let stale: Vec<String> = config
        .keymap
        .unresolved(operators)
        .map(|b| format!("{} → {}", b.chord.label(), b.operator))
        .collect();
    if !stale.is_empty() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Bound to something not currently loaded")
                .weak()
                .small(),
        );
        for line in stale {
            ui.label(egui::RichText::new(line).weak().small());
        }
    }

    ui.ctx().memory_mut(|m| match &capturing {
        Some(id) => {
            m.data.insert_temp(egui::Id::new(CAPTURING), id.clone());
        }
        None => m.data.remove::<String>(egui::Id::new(CAPTURING)),
    });
}

/// The first key pressed this frame, with its modifiers.
///
/// Modifier keys alone are skipped: someone reaching for `Ctrl+K` presses Ctrl
/// first, and capturing that as the binding would make the chord impossible to
/// enter.
fn pressed_chord(ctx: &egui::Context) -> Option<crate::keymap::Chord> {
    ctx.input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => Some(crate::keymap::Chord {
                key: *key,
                ctrl: modifiers.ctrl,
                shift: modifiers.shift,
                alt: modifiers.alt,
            }),
            _ => None,
        })
    })
}

/// Theme picker plus a live colour editor.
fn appearance(
    ui: &mut egui::Ui,
    config: &mut Config,
    theme: &mut Theme,
    available: &mut Vec<Theme>,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Colour scheme").strong());
    ui.add_space(4.0);

    let current = theme.label().to_string();
    egui::ComboBox::from_id_salt("settings_theme")
        .selected_text(&current)
        .width(220.0)
        .show_ui(ui, |ui| {
            for candidate in available.iter() {
                if ui
                    .selectable_label(candidate.label() == current, candidate.label())
                    .clicked()
                {
                    *theme = candidate.clone();
                    config.theme_name = Some(candidate.label().to_string());
                }
            }
        });

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Colours").strong());
    ui.label(
        egui::RichText::new("Edits apply immediately. Save as a new scheme to keep them.")
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(4.0);

    egui::Grid::new("theme_colors")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (label, slot) in theme.editable_colors() {
                ui.label(egui::RichText::new(label).size(11.5));
                color_field(ui, slot);
                ui.end_row();
            }
        });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let name_id = egui::Id::new("settings_theme_name");
        let mut name = ui
            .data(|d| d.get_temp::<String>(name_id))
            .unwrap_or_else(|| format!("{current} copy"));
        ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(180.0)
                .hint_text("New scheme name"),
        );
        if ui
            .add_enabled(!name.trim().is_empty(), egui::Button::new("Save as new"))
            .on_hover_text("Write this scheme to the config directory")
            .clicked()
        {
            let mut saved = theme.clone();
            saved.name = name.trim().to_string();
            match saved.save_to_disk() {
                Ok(path) => {
                    // Replace rather than append when the name is taken: two
                    // identically named schemes in the picker is a worse outcome
                    // than overwriting the one being edited.
                    available.retain(|t| t.label() != saved.label());
                    available.push(saved.clone());
                    config.theme_name = Some(saved.label().to_string());
                    *theme = saved;
                    ui.label(
                        egui::RichText::new(format!("Saved to {}", path.display())).size(10.0),
                    );
                }
                Err(e) => {
                    ui.label(
                        egui::RichText::new(format!("Could not save: {e}"))
                            .size(10.0)
                            .color(ui.visuals().error_fg_color),
                    );
                }
            }
        }
        ui.data_mut(|d| d.insert_temp(name_id, name));
    });
}

/// A hex string edited as a colour swatch, keeping the alpha the text carries.
fn color_field(ui: &mut egui::Ui, hex: &mut String) {
    let mut rgba = crate::theme::hex_to_color(hex).to_array();
    let mut color = egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
    ui.horizontal(|ui| {
        if ui.color_edit_button_srgba(&mut color).changed() {
            rgba = color.to_array();
            *hex = format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                rgba[0], rgba[1], rgba[2], rgba[3]
            );
        }
        // The hex stays editable as text: a value copied from a palette or a
        // brand guide arrives as `#1e232d`, not as a position in a colour wheel.
        ui.add(egui::TextEdit::singleline(hex).desired_width(90.0));
    });
}

fn grid(ui: &mut egui::Ui, config: &mut Config) {
    ui.add_space(4.0);
    ui.checkbox(&mut config.grid.show, "Show the checker");
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label("Cell size");
        ui.add(
            egui::DragValue::new(&mut config.grid.cell)
                .speed(0.5)
                .range(2.0..=1000.0)
                .suffix(" units"),
        )
        .on_hover_text("World units, so the checker sits still against the artwork while zooming");
    });
    ui.horizontal(|ui| {
        ui.label("Hide below");
        ui.add(
            egui::DragValue::new(&mut config.grid.min_cell_px)
                .speed(0.5)
                .range(2.0..=64.0)
                .suffix(" px"),
        )
        .on_hover_text(
            "Zoomed out far enough, the checker is noise — and a 3px cell is a \
             quarter of a million rects a frame",
        );
    });
    ui.add_space(8.0);
    ui.label(
        egui::RichText::new("Checker colours live under Appearance, with the rest of the scheme.")
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
    );

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Viewport").strong());
    ui.add_space(4.0);
    ui.checkbox(&mut config.hover_labels, "Name what the cursor is over")
        .on_hover_text(
            "Show the name and kind of whatever the pointer rests on.\n\
             Hold Alt to summon one on demand while this is off.",
        );
}

fn fonts(ui: &mut egui::Ui, config: &mut Config) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Scale").strong());
    ui.label(
        egui::RichText::new(
            "Renders text and icons at a higher resolution rather than blowing up              the same bitmap — this is the one that fixes blocky glyphs, not the              sizes below.",
        )
        .size(10.5)
        .color(ui.visuals().weak_text_color()),
    );
    ui.horizontal(|ui| {
        let before = config.ui_scale;
        ui.add(
            egui::Slider::new(&mut config.ui_scale, 0.75..=2.5)
                .fixed_decimals(2)
                .suffix("×"),
        );
        if ui.button("Reset").clicked() {
            config.ui_scale = 1.0;
        }
        if (config.ui_scale - before).abs() > 1e-4 {
            // Every glyph is re-rasterised, so this is applied as it changes
            // rather than every frame.
            ui.ctx().set_zoom_factor(config.ui_scale.clamp(0.5, 3.0));
        }
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Sizes").strong());
    ui.label(
        egui::RichText::new(
            "Per area, not one scale: the timeline packs sixty rows into a panel and \
             wants small text, while the inspector is read a field at a time.",
        )
        .size(10.5)
        .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(6.0);

    egui::Grid::new("font_sizes")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for (label, value) in [
                ("Interface", &mut config.fonts.ui),
                ("Hierarchy", &mut config.fonts.tree),
                ("Properties", &mut config.fonts.inspector),
                ("Timeline", &mut config.fonts.timeline),
            ] {
                ui.label(egui::RichText::new(label).size(11.5));
                ui.add(
                    egui::Slider::new(value, FontSettings::MIN..=FontSettings::MAX)
                        .fixed_decimals(1)
                        .suffix(" pt"),
                );
                ui.end_row();
            }
        });
}
