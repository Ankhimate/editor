//! Named selection sets as undoable commands (T-904).
//!
//! A set is document state — a rigger builds "left arm" once and whoever opens
//! the file next has it — so creating or deleting one is a document edit and
//! belongs on the undo stack like any other.
//!
//! Setup-only: a set describes the rig's structure, and structural edits are
//! Setup's job (T-207).
//!
//! Each command snapshots the whole list rather than inverting its edit. The
//! list is short, and an index-based inverse would be pointing at the wrong set
//! the moment one is removed above it.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::ids::BoneId;
use ankhimate_core::skeleton::SelectionSet;

/// Save the given bones under a name, replacing any set already using it.
///
/// Replacing rather than uniquifying, because "save selection as 'left arm'"
/// twice means the second one wins — that is what the words say, and a
/// `left arm_2` nobody asked for is the outcome the bulk-rename dialog exists to
/// avoid elsewhere.
pub struct SaveSelectionSet {
    name: String,
    bones: Vec<BoneId>,
    before: Option<Vec<SelectionSet>>,
}

impl SaveSelectionSet {
    pub fn new(name: impl Into<String>, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            bones,
            before: None,
        }
    }
}

impl EditCommand for SaveSelectionSet {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = Some(doc.skeleton.selection_sets.clone());
        }
        // Bones that no longer exist are dropped here rather than saved and
        // pruned later: a set is only worth having if every id in it resolves.
        let bones: Vec<BoneId> = self
            .bones
            .iter()
            .copied()
            .filter(|b| doc.skeleton.bones.contains_key(*b))
            .collect();
        if bones.is_empty() {
            return;
        }
        doc.skeleton.selection_sets.retain(|s| s.name != self.name);
        doc.skeleton.selection_sets.push(SelectionSet {
            name: self.name.clone(),
            bones,
        });
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            doc.skeleton.selection_sets = before;
        }
    }

    fn label(&self) -> &str {
        "Save Selection Set"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &super::IdRemap) {
        for bone in &mut self.bones {
            *bone = remap.bone(*bone);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What [`EditSelectionSet`] does to the set it names.
pub enum SetEdit {
    Rename(String),
    Remove,
}

/// Rename or delete one set, by index.
pub struct EditSelectionSet {
    index: usize,
    edit: SetEdit,
    before: Option<Vec<SelectionSet>>,
}

impl EditSelectionSet {
    pub fn new(index: usize, edit: SetEdit) -> Self {
        Self {
            index,
            edit,
            before: None,
        }
    }
}

impl EditCommand for EditSelectionSet {
    fn apply(&mut self, doc: &mut Document) {
        if self.index >= doc.skeleton.selection_sets.len() {
            return;
        }
        if self.before.is_none() {
            self.before = Some(doc.skeleton.selection_sets.clone());
        }
        match &self.edit {
            SetEdit::Rename(name) => {
                // A rename onto another set's name would leave two rows reading
                // the same, and clicking either would be a coin flip. The other
                // one goes, matching what `SaveSelectionSet` does.
                //
                // Renamed first, *then* the duplicate is dropped. In that order
                // no index has to be adjusted, so `apply` stays a pure function
                // of the snapshot and survives a redo unchanged.
                if let Some(set) = doc.skeleton.selection_sets.get_mut(self.index) {
                    set.name = name.clone();
                }
                let target = self.index;
                let mut position = 0;
                doc.skeleton.selection_sets.retain(|s| {
                    let index = position;
                    position += 1;
                    // The row being renamed always survives; another wearing the
                    // same name does not.
                    index == target || s.name != *name
                });
            }
            SetEdit::Remove => {
                doc.skeleton.selection_sets.remove(self.index);
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            doc.skeleton.selection_sets = before;
        }
    }

    fn label(&self) -> &str {
        match self.edit {
            SetEdit::Rename(_) => "Rename Selection Set",
            SetEdit::Remove => "Delete Selection Set",
        }
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.to_string(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn saving_a_set_round_trips_through_undo() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let b = doc.skeleton.add_bone(bone("b"));
        let mut history = History::default();

        history.push(
            Box::new(SaveSelectionSet::new("left arm", vec![a, b])),
            &mut doc,
        );
        assert_eq!(doc.skeleton.selection_sets.len(), 1);
        assert_eq!(doc.skeleton.selection_sets[0].bones, vec![a, b]);

        history.undo(&mut doc);
        assert!(doc.skeleton.selection_sets.is_empty());
    }

    /// Saving under a name already in use replaces it — "save as X" twice means
    /// the second one wins.
    #[test]
    fn saving_over_a_name_replaces_that_set() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let b = doc.skeleton.add_bone(bone("b"));
        let mut history = History::default();

        history.push(Box::new(SaveSelectionSet::new("arm", vec![a])), &mut doc);
        history.push(Box::new(SaveSelectionSet::new("arm", vec![b])), &mut doc);

        assert_eq!(
            doc.skeleton.selection_sets.len(),
            1,
            "not two rows named arm"
        );
        assert_eq!(doc.skeleton.selection_sets[0].bones, vec![b]);
    }

    /// Deleting a bone takes it out of every set, and a set left empty goes with
    /// it — a set that silently selects fewer bones than it names is worse than
    /// no set.
    #[test]
    fn deleting_a_bone_prunes_the_sets_holding_it() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let b = doc.skeleton.add_bone(bone("b"));
        let mut history = History::default();

        history.push(
            Box::new(SaveSelectionSet::new("pair", vec![a, b])),
            &mut doc,
        );
        history.push(Box::new(SaveSelectionSet::new("just_b", vec![b])), &mut doc);

        doc.skeleton.remove_bone(b);

        assert_eq!(
            doc.skeleton.selection_sets.len(),
            1,
            "the set holding only the deleted bone is gone"
        );
        assert_eq!(doc.skeleton.selection_sets[0].name, "pair");
        assert_eq!(doc.skeleton.selection_sets[0].bones, vec![a]);
    }

    #[test]
    fn renaming_and_removing_a_set_undo() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let mut history = History::default();
        history.push(Box::new(SaveSelectionSet::new("old", vec![a])), &mut doc);

        history.push(
            Box::new(EditSelectionSet::new(0, SetEdit::Rename("new".to_string()))),
            &mut doc,
        );
        assert_eq!(doc.skeleton.selection_sets[0].name, "new");
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.selection_sets[0].name, "old");

        history.push(
            Box::new(EditSelectionSet::new(0, SetEdit::Remove)),
            &mut doc,
        );
        assert!(doc.skeleton.selection_sets.is_empty());
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.selection_sets.len(), 1);
    }
}
