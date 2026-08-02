//! The skin panel (T-507).
//!
//! Two jobs that look similar and are not:
//!
//! * **which skin edits go to** — one, always, so "where did that attachment
//!   land" has a single answer;
//! * **which skins are shown** — any number, layered, because a hat and armor
//!   should be wearable together and a tool with one global switch forces every
//!   combination to exist as its own skin.
//!
//! The radio column is the first, the checkboxes the second. Composition is
//! session state: it is a question the *game* answers at runtime, and baking a
//! combination into the document would mean re-authoring to preview another.

use crate::app_state::AppState;
use crate::commands::skin_cmds::{AddSkin, CopyAttachments, RemoveSkin, RenameSkin};
use ankhimate_core::ids::SkinId;
use ankhimate_core::slotmap::Key as _;
use eframe::egui;

pub fn ui(ui: &mut egui::Ui, state: &mut AppState) {
    let setup = state.session.can_edit_structure();
    let default_skin = state.doc.skeleton.default_skin;
    let active = state.session.active_skin;

    // Ordered so the panel does not reshuffle as skins are added: default first,
    // then insertion order.
    let mut skins: Vec<(SkinId, String, usize)> = state
        .doc
        .skeleton
        .skins
        .iter()
        .map(|(id, s)| (id, s.name.clone(), s.entries.len()))
        .collect();
    // Default first, then by slotmap key so the list does not reshuffle when a
    // skin is added or renamed.
    skins.sort_by_key(|(id, _, _)| (*id != default_skin, id.data().as_ffi()));

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Skins").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(setup, egui::Button::new("+").small())
                .on_hover_text("New empty skin")
                .clicked()
            {
                let name = unique_name(state, "skin");
                state.dispatch(Box::new(AddSkin::new(name)));
            }
            if ui
                .add_enabled(setup, egui::Button::new("⧉").small())
                .on_hover_text("Duplicate the active skin, attachments and all")
                .clicked()
            {
                let base = state
                    .doc
                    .skeleton
                    .skins
                    .get(active)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "skin".to_string());
                let name = unique_name(state, &format!("{base} copy"));
                state.dispatch(Box::new(AddSkin::duplicating(name, active)));
            }
        });
    });
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("● edits go here   ☑ also shown")
            .size(10.0)
            .color(ui.visuals().weak_text_color()),
    );
    ui.separator();

    let mut set_active: Option<SkinId> = None;
    let mut rename: Option<(SkinId, String)> = None;
    let mut remove: Option<SkinId> = None;
    let mut copy_into: Option<(SkinId, SkinId)> = None;
    let mut toggle_layer: Option<SkinId> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (id, name, count) in &skins {
            ui.horizontal(|ui| {
                if ui
                    .radio(*id == active, "")
                    .on_hover_text("Edits go to this skin")
                    .clicked()
                {
                    set_active = Some(*id);
                }

                // The active skin is always shown, so its checkbox would be a
                // lie you could click.
                let layered = state.session.layered_skins.contains(id);
                let mut shown = layered || *id == active;
                if ui
                    .add_enabled(*id != active, egui::Checkbox::new(&mut shown, ""))
                    .on_hover_text("Show this skin underneath the active one")
                    .changed()
                {
                    toggle_layer = Some(*id);
                }

                let mut text = name.clone();
                let editable = ui.add_enabled(
                    setup,
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(110.0)
                        .font(egui::TextStyle::Body),
                );
                if editable.changed() && !text.is_empty() {
                    rename = Some((*id, text));
                }

                ui.label(
                    egui::RichText::new(format!("{count}"))
                        .size(10.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .on_hover_text("Attachments in this skin");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The default skin is every resolution's last fallback;
                    // deleting it would make un-skinned slots draw nothing.
                    let deletable = setup && *id != default_skin;
                    if ui
                        .add_enabled(deletable, egui::Button::new("✕").small())
                        .on_hover_text(if *id == default_skin {
                            "The default skin is the fallback for every slot"
                        } else {
                            "Delete this skin and its attachments"
                        })
                        .clicked()
                    {
                        remove = Some(*id);
                    }
                    if *id != active
                        && ui
                            .add_enabled(setup, egui::Button::new("→").small())
                            .on_hover_text("Copy this skin's attachments into the active one")
                            .clicked()
                    {
                        copy_into = Some((*id, active));
                    }
                });
            });
        }
    });

    if let Some(id) = set_active {
        state.session.active_skin = id;
        // A skin cannot be both the edit target and a layer under itself.
        state.session.layered_skins.retain(|s| *s != id);
        state.refresh_pose();
    }
    if let Some(id) = toggle_layer {
        if let Some(at) = state.session.layered_skins.iter().position(|s| *s == id) {
            state.session.layered_skins.remove(at);
        } else {
            state.session.layered_skins.push(id);
        }
        state.refresh_pose();
    }
    if let Some((id, name)) = rename {
        state.dispatch(Box::new(RenameSkin::new(id, name)));
    }
    if let Some((from, to)) = copy_into {
        state.dispatch(Box::new(CopyAttachments::new(from, to)));
    }
    if let Some(id) = remove {
        // Point the session somewhere real before the skin goes away.
        if state.session.active_skin == id {
            state.session.active_skin = default_skin;
        }
        state.session.layered_skins.retain(|s| *s != id);
        state.dispatch(Box::new(RemoveSkin::new(id)));
    }
}

/// A name no existing skin has, since names address skins on disk.
fn unique_name(state: &AppState, base: &str) -> String {
    let taken = |n: &str| state.doc.skeleton.skins.values().any(|s| s.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}
