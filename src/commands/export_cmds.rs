//! Export preset commands (T-603d).
//!
//! Presets are document state, so editing one is undoable like any other edit.
//! Typing in a template body would otherwise be the one place in the editor
//! where Ctrl+Z does nothing.

use super::EditCommand;
use crate::doc::Document;
use ankhimate_export::preset::{Preset, Template};

fn to_value(preset: &Preset) -> serde_json::Value {
    serde_json::to_value(preset).expect("a preset serializes")
}

/// Add a preset — a built-in clone, or an empty one.
pub struct AddPreset {
    preset: Preset,
    /// Where it landed, so revert removes the right one.
    index: Option<usize>,
}

impl AddPreset {
    pub fn new(preset: Preset) -> Self {
        Self {
            preset,
            index: None,
        }
    }
}

impl EditCommand for AddPreset {
    fn apply(&mut self, doc: &mut Document) {
        // A duplicate name is confusing rather than wrong, so it is made unique
        // instead of refused: cloning "Godot 4" twice should give a second
        // preset, not silently overwrite the first.
        let mut preset = self.preset.clone();
        let taken: Vec<String> = doc.presets().into_iter().map(|p| p.name).collect();
        if taken.iter().any(|n| n == &preset.name) {
            let base = preset.name.clone();
            let mut n = 2;
            while taken.iter().any(|t| t == &format!("{base} {n}")) {
                n += 1;
            }
            preset.name = format!("{base} {n}");
        }
        doc.export_presets.push(to_value(&preset));
        self.index = Some(doc.export_presets.len() - 1);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(index) = self.index
            && index < doc.export_presets.len()
        {
            doc.export_presets.remove(index);
        }
    }

    fn label(&self) -> &str {
        "Add Export Preset"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Remove a preset, remembering it for undo.
pub struct RemovePreset {
    index: usize,
    removed: Option<serde_json::Value>,
}

impl RemovePreset {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            removed: None,
        }
    }
}

impl EditCommand for RemovePreset {
    fn apply(&mut self, doc: &mut Document) {
        if self.index < doc.export_presets.len() {
            self.removed = Some(doc.export_presets.remove(self.index));
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(value) = self.removed.take() {
            let at = self.index.min(doc.export_presets.len());
            doc.export_presets.insert(at, value);
        }
    }

    fn label(&self) -> &str {
        "Remove Export Preset"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Replace a preset's settings — output folder, atlas options, name.
pub struct SetPresetOptions {
    index: usize,
    next: Preset,
    before: Option<serde_json::Value>,
}

impl SetPresetOptions {
    pub fn new(index: usize, next: Preset) -> Self {
        Self {
            index,
            next,
            before: None,
        }
    }
}

impl EditCommand for SetPresetOptions {
    fn apply(&mut self, doc: &mut Document) {
        if self.index >= doc.export_presets.len() {
            return;
        }
        // Capture `before` on the *first* apply only, so a merged drag undoes to
        // where it began rather than to the previous frame.
        if self.before.is_none() {
            self.before = Some(doc.export_presets[self.index].clone());
        }
        doc.export_presets[self.index] = to_value(&self.next);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), true) = (&self.before, self.index < doc.export_presets.len()) {
            doc.export_presets[self.index] = before.clone();
        }
    }

    /// Dragging the padding spinner is one edit, not forty.
    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetPresetOptions>() else {
            return false;
        };
        if other.index != self.index {
            return false;
        }
        self.next = other.next.clone();
        true
    }

    fn label(&self) -> &str {
        "Export Settings"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Replace one template within a preset.
pub struct EditTemplate {
    preset: usize,
    template: usize,
    next: Template,
    before: Option<serde_json::Value>,
}

impl EditTemplate {
    pub fn new(preset: usize, template: usize, next: Template) -> Self {
        Self {
            preset,
            template,
            next,
            before: None,
        }
    }
}

impl EditCommand for EditTemplate {
    fn apply(&mut self, doc: &mut Document) {
        if self.preset >= doc.export_presets.len() {
            return;
        }
        if self.before.is_none() {
            self.before = Some(doc.export_presets[self.preset].clone());
        }
        let Ok(mut preset) =
            serde_json::from_value::<Preset>(doc.export_presets[self.preset].clone())
        else {
            return;
        };
        if self.template < preset.templates.len() {
            preset.templates[self.template] = self.next.clone();
            doc.export_presets[self.preset] = to_value(&preset);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), true) = (&self.before, self.preset < doc.export_presets.len()) {
            doc.export_presets[self.preset] = before.clone();
        }
    }

    /// Typing is one undo step per field, not one per keystroke.
    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditTemplate>() else {
            return false;
        };
        if other.preset != self.preset || other.template != self.template {
            return false;
        }
        self.next = other.next.clone();
        true
    }

    fn label(&self) -> &str {
        "Edit Template"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
