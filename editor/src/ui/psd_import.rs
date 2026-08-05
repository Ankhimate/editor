//! PSD import modal (T-302).
//!
//! The mapping lives in `ankhimate_formats::psd`; this is the window that shows
//! what is about to happen and lets the user say no to parts of it. The preview
//! tree is the whole point: a PSD is somebody else's document, and importing one
//! blind is how you end up with sixty slots named `Layer 14 copy`.

use crate::app_state::AppState;
use ankhimate_formats::psd::{ImportOptions, LayerNode};
use eframe::egui;
use std::collections::HashSet;

/// A PSD staged for import. Session state — cancelling must leave no trace.
#[derive(Clone)]
pub struct PendingPsd {
    pub name: String,
    pub source_path: Option<String>,
    pub bytes: Vec<u8>,
    pub nodes: Vec<LayerNode>,
    /// Paths the user has ticked. Seeded from what the PSD shows.
    pub include: HashSet<String>,
    /// Groups to collapse into one attachment.
    pub flatten: HashSet<String>,
    pub scale: f32,
    pub skip_hidden: bool,
    /// Replace the open document rather than merging into it.
    pub replace: bool,
}

impl PendingPsd {
    pub fn new(name: String, bytes: Vec<u8>, nodes: Vec<LayerNode>) -> Self {
        // Everything visible starts ticked: the common case is "import this
        // file", and starting from nothing makes the user click forty times to
        // reach it.
        let include = nodes
            .iter()
            .filter(|n| n.visible)
            .map(|n| n.path.clone())
            .collect();
        Self {
            name,
            source_path: None,
            bytes,
            nodes,
            include,
            flatten: HashSet::new(),
            scale: 1.0,
            skip_hidden: true,
            replace: true,
        }
    }

    fn options(&self) -> ImportOptions {
        ImportOptions {
            scale: self.scale,
            include: self.include.clone(),
            flatten: self.flatten.clone(),
            skip_hidden: self.skip_hidden,
        }
    }

    /// Ticking a group ticks everything inside it, which is what "include this
    /// arm" means. Untick works the same way.
    fn set_subtree(&mut self, path: &str, on: bool) {
        let prefix = format!("{path}/");
        let affected: Vec<String> = self
            .nodes
            .iter()
            .filter(|n| n.path == path || n.path.starts_with(&prefix))
            .map(|n| n.path.clone())
            .collect();
        for p in affected {
            if on {
                self.include.insert(p);
            } else {
                self.include.remove(&p);
            }
        }
    }

    fn counts(&self) -> (usize, usize) {
        let groups = self
            .nodes
            .iter()
            .filter(|n| n.is_group && self.include.contains(&n.path))
            .count();
        let layers = self
            .nodes
            .iter()
            .filter(|n| !n.is_group && self.include.contains(&n.path))
            .count();
        (groups, layers)
    }
}

/// Draw the import window. Returns `true` when it should close.
pub fn ui(ctx: &egui::Context, state: &mut AppState, theme: &crate::theme::Theme) -> bool {
    let Some(mut pending) = state.session.pending_psd.clone() else {
        return false;
    };

    let mut close = false;
    let mut confirm = false;

    let dialog = crate::ui::dialog::Dialog::new("psd_import", "Import PSD")
        .icon(crate::ui::icons::IMPORT_PSD)
        .width(620.0)
        .show(ctx, theme, |ui| {
            ui.horizontal_top(|ui| {
                layer_tree(ui, &mut pending);

                ui.vertical(|ui| {
                    ui.set_min_width(230.0);
                    let (groups, layers) = pending.counts();
                    ui.label(
                        egui::RichText::new(format!("{groups} groups · {layers} layers"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Scale");
                        ui.add(
                            egui::DragValue::new(&mut pending.scale)
                                .speed(0.01)
                                .range(0.01..=10.0),
                        )
                        .on_hover_text("World units per PSD pixel");
                    });
                    if ui
                        .checkbox(&mut pending.skip_hidden, "Skip hidden layers")
                        .on_hover_text("A hidden layer is usually a sketch or an alternate")
                        .changed()
                    {
                        // The tick list was seeded from visibility, so honour the
                        // new answer rather than leaving a stale selection.
                        if pending.skip_hidden {
                            let hidden: Vec<String> = pending
                                .nodes
                                .iter()
                                .filter(|n| !n.visible)
                                .map(|n| n.path.clone())
                                .collect();
                            for path in hidden {
                                pending.include.remove(&path);
                            }
                        } else {
                            let all: Vec<String> =
                                pending.nodes.iter().map(|n| n.path.clone()).collect();
                            pending.include.extend(all);
                        }
                    }
                    ui.checkbox(&mut pending.replace, "Replace the open project")
                        .on_hover_text(
                            "Off: add the imported rig to what is already open, keeping \
                             existing bones and animations",
                        );

                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "Group → bone · layer → slot\n\
                             $pivot → that group's origin\n\
                             $ik <name> → IK over the bones inside\n\
                             @skin:<name> → contents go to that skin",
                        )
                        .size(10.5)
                        .color(ui.visuals().weak_text_color()),
                    );

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(layers > 0, egui::Button::new("Import"))
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            });
        });

    if confirm {
        run_import(state, &pending);
        close = true;
    }
    close |= dialog.closed;
    if close {
        state.session.pending_psd = None;
    } else {
        state.session.pending_psd = Some(pending);
    }
    close
}

/// The layer tree with a tick box per row, and a flatten toggle on groups.
fn layer_tree(ui: &mut egui::Ui, pending: &mut PendingPsd) {
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .min_scrolled_height(360.0)
        .show(ui, |ui| {
            ui.set_min_width(330.0);
            // Collected first: the rows mutate `include`, and the borrow would
            // otherwise overlap the iteration.
            let rows: Vec<LayerNode> = pending.nodes.clone();
            for node in rows {
                ui.horizontal(|ui| {
                    // Runtime indent from tree depth — no static class can hold a
                    // value computed per row.
                    ui.add_space(node.depth as f32 * 14.0);
                    let mut on = pending.include.contains(&node.path);
                    if ui.checkbox(&mut on, "").changed() {
                        pending.set_subtree(&node.path, on);
                    }
                    let icon = if node.is_group {
                        crate::ui::icons::FOLDER
                    } else {
                        crate::ui::icons::IMAGE
                    };
                    let label = egui::RichText::new(format!("{icon} {}", node.name)).size(12.0);
                    let label = if node.visible {
                        label
                    } else {
                        label.color(ui.visuals().weak_text_color())
                    };
                    ui.label(label);

                    if node.is_group {
                        let mut flat = pending.flatten.contains(&node.path);
                        if ui
                            .checkbox(&mut flat, "flatten")
                            .on_hover_text(
                                "One attachment for the whole group instead of a bone \
                                 with children",
                            )
                            .changed()
                        {
                            if flat {
                                pending.flatten.insert(node.path.clone());
                            } else {
                                pending.flatten.remove(&node.path);
                            }
                        }
                    } else if node.bounds.2 > 0 {
                        ui.label(
                            egui::RichText::new(format!("{}×{}", node.bounds.2, node.bounds.3))
                                .size(10.0)
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            }
        });
}

/// Run the import and fold the result into the document as one undo step.
fn run_import(state: &mut AppState, pending: &PendingPsd) {
    let imported = match ankhimate_formats::psd::import(&pending.bytes, &pending.options()) {
        Ok(imported) => imported,
        Err(e) => {
            state.session.set_status(format!("PSD import failed: {e}"));
            return;
        }
    };

    let summary = imported.summary.clone();
    let command =
        crate::commands::psd_cmds::ImportPsd::new(imported, pending.replace, pending.name.clone());
    if state.dispatch(Box::new(command)) {
        let skipped = summary.skipped.len();
        state.session.set_status(match skipped {
            0 => format!(
                "Imported {} bones, {} slots, {} images",
                summary.bones, summary.slots, summary.images
            ),
            n => format!(
                "Imported {} bones, {} slots, {} images ({n} layers skipped)",
                summary.bones, summary.slots, summary.images
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, is_group: bool, visible: bool) -> LayerNode {
        LayerNode {
            depth: path.matches('/').count(),
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_group,
            visible,
            bounds: (0, 0, 4, 4),
        }
    }

    fn pending() -> PendingPsd {
        PendingPsd::new(
            "hero".into(),
            Vec::new(),
            vec![
                node("torso", true, true),
                node("torso/chest", false, true),
                node("torso/arm", true, true),
                node("torso/arm/hand", false, true),
                node("sketch", false, false),
            ],
        )
    }

    #[test]
    fn visible_layers_start_ticked_and_hidden_ones_do_not() {
        let p = pending();
        assert!(p.include.contains("torso/chest"));
        assert!(
            !p.include.contains("sketch"),
            "a hidden layer is usually a reference, not art"
        );
    }

    #[test]
    fn unticking_a_group_untickets_everything_inside_it() {
        let mut p = pending();
        p.set_subtree("torso/arm", false);
        assert!(!p.include.contains("torso/arm"));
        assert!(!p.include.contains("torso/arm/hand"));
        // A sibling outside the subtree is untouched.
        assert!(p.include.contains("torso/chest"));
    }

    #[test]
    fn ticking_a_group_reaches_its_descendants() {
        let mut p = pending();
        p.set_subtree("torso", false);
        assert!(p.counts().1 == 0 || !p.include.contains("torso/arm/hand"));
        p.set_subtree("torso", true);
        assert!(p.include.contains("torso/arm/hand"));
    }

    #[test]
    fn counts_split_groups_from_layers() {
        let p = pending();
        // torso + torso/arm are groups; chest + hand are layers; sketch is off.
        assert_eq!(p.counts(), (2, 2));
    }
}
