//! PSD import modal (T-302).
//!
//! The mapping lives in `ankhimate_formats::psd`; this is the window that shows
//! what is about to happen and lets the user say no to parts of it. The preview
//! tree is the whole point: a PSD is somebody else's document, and importing one
//! blind is how you end up with sixty slots named `Layer 14 copy`.

use crate::app_state::AppState;
use ankhimate_formats::psd::{ImportOptions, LayerNode};
use ankhimate_formats::psd_infer::{self, Candidate, Guess};
use ankhimate_formats::psd_tags::{self, Tags};
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
    /// Tags read off each node's name, parallel to `nodes`.
    ///
    /// Parsed once when the file is staged rather than per frame: the tree
    /// redraws sixty times a second and the names do not change.
    pub tags: Vec<Tags>,
    /// What inference decided, and the tag that would say otherwise.
    ///
    /// Shown *before* the import runs. A report afterwards is a receipt for a
    /// rig the artist already has to undo; shown here it is a decision they can
    /// still argue with, which is the difference between inference that is safe
    /// and inference that is merely convenient.
    pub guesses: Vec<Guess>,
    /// Tags this build does not understand, as `(layer path, tag)`.
    pub unknown_tags: Vec<(String, String)>,
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

        let tags: Vec<Tags> = nodes.iter().map(|n| Tags::parse(&n.name)).collect();
        let candidates: Vec<Candidate> = nodes
            .iter()
            .map(|n| Candidate {
                path: n.path.clone(),
                name: n.name.clone(),
                depth: n.depth,
                is_group: n.is_group,
                bounds: n.bounds,
            })
            .collect();
        let mut guesses = Vec::new();
        psd_infer::infer(&candidates, &tags, &mut guesses);

        let unknown_tags = nodes
            .iter()
            .zip(&tags)
            .flat_map(|(node, tags)| {
                tags.names()
                    .filter(|t| !psd_tags::KNOWN.contains(t))
                    .map(|t| (node.path.clone(), t.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();

        Self {
            tags,
            guesses,
            unknown_tags,
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
                    what_was_decided(ui, &pending);

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

/// What the reader worked out on its own, before anything is imported.
///
/// The half of inference that makes it safe rather than merely convenient. A
/// guess the artist cannot see is a rig subtly wrong for a reason nobody can
/// find; shown here, with the tag that would say otherwise, a wrong one is a
/// rename away instead of an undo and a search through documentation.
///
/// Deliberately *before* the Import button rather than after it: a report
/// afterwards is a receipt for a rig they already have to undo.
fn what_was_decided(ui: &mut egui::Ui, pending: &PendingPsd) {
    let weak = ui.visuals().weak_text_color();

    if pending.guesses.is_empty() && pending.unknown_tags.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nothing was guessed — the layer names said everything the reader needed.",
            )
            .size(10.5)
            .color(weak),
        );
        return;
    }

    if !pending.guesses.is_empty() {
        ui.label(
            egui::RichText::new(format!(
                "{} {} decided for you",
                crate::ui::icons::INFERRED,
                pending.guesses.len()
            ))
            .size(11.0)
            .strong(),
        );
        egui::ScrollArea::vertical()
            .id_salt("psd_guesses")
            .max_height(150.0)
            .show(ui, |ui| {
                for guess in &pending.guesses {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(&guess.decided).size(11.0));
                    // The evidence and the override are the two things that let
                    // an artist disagree, so neither hides behind a hover: a
                    // tooltip is unreadable on a list you are scanning.
                    ui.label(
                        egui::RichText::new(format!("because {}", guess.because))
                            .size(10.0)
                            .color(weak),
                    );
                    ui.label(
                        egui::RichText::new(format!("say otherwise with {}", guess.override_with))
                            .size(10.0)
                            .italics()
                            .color(weak),
                    );
                }
            });
    }

    if !pending.unknown_tags.is_empty() {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!(
                "{} {} tag(s) this build does not understand",
                crate::ui::icons::LOSSY,
                pending.unknown_tags.len()
            ))
            .size(11.0)
            .color(ui.visuals().warn_fg_color),
        );
        for (path, tag) in pending.unknown_tags.iter().take(6) {
            ui.label(
                egui::RichText::new(format!("[{tag}] on {path}"))
                    .size(10.0)
                    .color(weak),
            );
        }
    }
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
            for (index, node) in rows.into_iter().enumerate() {
                ui.horizontal(|ui| {
                    // Runtime indent from tree depth — no static class can hold a
                    // value computed per row.
                    ui.add_space(node.depth as f32 * 14.0);
                    let mut on = pending.include.contains(&node.path);
                    if ui.checkbox(&mut on, "").changed() {
                        pending.set_subtree(&node.path, on);
                    }
                    // The tags on this row, and the name with them stripped —
                    // which is what the bone or slot will actually be called.
                    // An artist writing `arm [bone][slot:upper]` should be able
                    // to see that they get a thing called `arm`, here, rather
                    // than after the import.
                    let tags = pending.tags.get(index);
                    let shown = tags.map(|t| t.name.as_str()).unwrap_or(&node.name);
                    let icon = if node.is_group {
                        crate::ui::icons::FOLDER
                    } else {
                        crate::ui::icons::IMAGE
                    };
                    let label = egui::RichText::new(format!("{icon} {shown}")).size(12.0);
                    let label = if node.visible {
                        label
                    } else {
                        label.color(ui.visuals().weak_text_color())
                    };
                    ui.label(label);

                    if let Some(tags) = tags {
                        for tag in tags.names() {
                            let known = psd_tags::KNOWN.contains(&tag);
                            let text = match tags.value(tag) {
                                Some(value) => format!("{tag}:{value}"),
                                None => tag.to_string(),
                            };
                            // An unrecognised tag is coloured rather than
                            // hidden: a misspelled `[bonee]` that looks like
                            // every other chip is an artist wondering why their
                            // tag did nothing.
                            let colour = if known {
                                ui.visuals().weak_text_color()
                            } else {
                                ui.visuals().warn_fg_color
                            };
                            ui.label(
                                egui::RichText::new(format!("{} {text}", crate::ui::icons::TAG))
                                    .size(9.5)
                                    .color(colour),
                            )
                            .on_hover_text(if known {
                                "Read off the layer name"
                            } else {
                                "This build does not understand this tag"
                            });
                        }
                    }

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
    let command = ankhimate_document::commands::psd_cmds::ImportPsd::new(
        imported,
        pending.replace,
        pending.name.clone(),
    );
    if state.dispatch(Box::new(command)) {
        state.session.set_status(import_status(&summary));
    }
}

/// One line saying what arrived, and what did not.
///
/// The counts alone are not the whole answer: a folded sequence means the artist
/// handed over five layers and got one slot back, and a blend mode this model
/// cannot express means a layer will draw differently here than it did in
/// Photoshop. Both are things they should hear about without opening a panel.
fn import_status(summary: &ankhimate_formats::psd::ImportSummary) -> String {
    let mut line = format!(
        "Imported {} bones, {} slots, {} images",
        summary.bones, summary.slots, summary.images
    );
    let mut notes: Vec<String> = Vec::new();
    if !summary.sequences.is_empty() {
        let frames: usize = summary.sequences.iter().map(|(_, n)| n).sum();
        notes.push(format!(
            "{} sequence(s) over {frames} frames",
            summary.sequences.len()
        ));
    }
    if !summary.lost_blend.is_empty() {
        notes.push(format!(
            "{} blend mode(s) with no equivalent",
            summary.lost_blend.len()
        ));
    }
    if !summary.skipped.is_empty() {
        notes.push(format!("{} layers skipped", summary.skipped.len()));
    }
    if !notes.is_empty() {
        line.push_str(&format!(" ({})", notes.join(", ")));
    }
    line
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

    #[test]
    fn staging_a_file_reads_its_tags_and_runs_inference() {
        // The preview has to know what the import will decide *before* it runs.
        // Doing it at stage time rather than per frame also matters: the tree
        // redraws sixty times a second and the layer names do not change.
        let p = PendingPsd::new(
            "hero".into(),
            Vec::new(),
            vec![
                node("fx", true, true),
                node("fx/fire_01", false, true),
                node("fx/fire_02", false, true),
            ],
        );
        assert_eq!(p.tags.len(), p.nodes.len(), "one parse per row");
        assert!(
            p.guesses.iter().any(|g| g.decided.contains("sequence")),
            "the numbered run was noticed before import: {:?}",
            p.guesses
        );
    }

    #[test]
    fn a_tag_is_stripped_from_the_name_the_preview_shows() {
        // An artist writing `arm [bone]` gets a bone called `arm`, and should be
        // able to see that here rather than after importing.
        let p = PendingPsd::new(
            "hero".into(),
            Vec::new(),
            vec![node("arm [bone]", true, true)],
        );
        assert_eq!(p.tags[0].name, "arm");
    }

    #[test]
    fn an_unrecognised_tag_reaches_the_preview() {
        // A misspelled tag must be visible before the import, not discovered
        // afterwards by wondering why it did nothing.
        let p = PendingPsd::new(
            "hero".into(),
            Vec::new(),
            vec![node("arm [bonee]", true, true)],
        );
        assert_eq!(
            p.unknown_tags,
            [("arm [bonee]".to_string(), "bonee".into())]
        );
    }

    #[test]
    fn the_status_line_mentions_what_the_counts_do_not() {
        // Counts alone hide the two things worth hearing about: a run that
        // folded, and a blend mode this model cannot express.
        use ankhimate_formats::psd::ImportSummary;
        let summary = ImportSummary {
            bones: 3,
            slots: 4,
            images: 6,
            sequences: vec![("fire".into(), 3)],
            lost_blend: vec![("cape".into(), "Overlay".into())],
            ..Default::default()
        };
        let line = import_status(&summary);
        assert!(line.contains("1 sequence(s) over 3 frames"), "{line}");
        assert!(line.contains("1 blend mode(s)"), "{line}");
    }

    #[test]
    fn a_clean_import_says_nothing_extra() {
        // The parenthetical is for things that happened. A rig that came across
        // whole should not read as though something went wrong.
        use ankhimate_formats::psd::ImportSummary;
        let line = import_status(&ImportSummary {
            bones: 2,
            slots: 2,
            images: 2,
            ..Default::default()
        });
        assert_eq!(line, "Imported 2 bones, 2 slots, 2 images");
    }
}
