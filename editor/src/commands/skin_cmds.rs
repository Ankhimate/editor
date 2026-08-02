//! Skin management as undoable commands (T-507).
//!
//! Skins are rig structure — which art a slot *can* show — so these are
//! Setup-only (T-207). Which skins are worn together is a viewing state and
//! lives in `Session`, not here: baking a combination into the document would
//! mean re-authoring the rig to preview a different outfit.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::ids::SkinId;
use ankhimate_core::skin::Skin;

/// Create an empty skin, or a copy of an existing one.
pub struct AddSkin {
    name: String,
    /// Copy this skin's entries into the new one.
    copy_from: Option<SkinId>,
    created: Option<SkinId>,
}

impl AddSkin {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            copy_from: None,
            created: None,
        }
    }

    pub fn duplicating(name: impl Into<String>, source: SkinId) -> Self {
        Self {
            name: name.into(),
            copy_from: Some(source),
            created: None,
        }
    }
}

impl EditCommand for AddSkin {
    fn apply(&mut self, doc: &mut Document) {
        // Names address skins on disk (ADR 0004), so a duplicate name would make
        // one of the two unreachable after a save/load round trip.
        let taken = doc.skeleton.skins.values().any(|s| s.name == self.name);
        if taken {
            return;
        }
        let mut skin = Skin::new(self.name.clone());
        if let Some(source) = self.copy_from
            && let Some(from) = doc.skeleton.skins.get(source)
        {
            skin.entries = from.entries.clone();
        }
        self.created = Some(doc.skeleton.add_skin(skin));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.skins.remove(id);
        }
    }

    fn label(&self) -> &str {
        "Add Skin"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Rename a skin.
///
/// References to skins are by id in memory and by name on disk, so a rename is
/// purely local — nothing else has to be rewritten, unlike a bone rename.
pub struct RenameSkin {
    id: SkinId,
    to: String,
    from: Option<String>,
}

impl RenameSkin {
    pub fn new(id: SkinId, to: impl Into<String>) -> Self {
        Self {
            id,
            to: to.into(),
            from: None,
        }
    }
}

impl EditCommand for RenameSkin {
    fn apply(&mut self, doc: &mut Document) {
        let taken = doc
            .skeleton
            .skins
            .iter()
            .any(|(id, s)| id != self.id && s.name == self.to);
        if taken || self.to.is_empty() {
            return;
        }
        if let Some(skin) = doc.skeleton.skins.get_mut(self.id) {
            self.from = Some(std::mem::replace(&mut skin.name, self.to.clone()));
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(from), Some(skin)) = (self.from.take(), doc.skeleton.skins.get_mut(self.id)) {
            skin.name = from;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<RenameSkin>() else {
            return false;
        };
        // Typing in a text field is one rename, not one per keystroke.
        if other.id != self.id {
            return false;
        }
        self.to = other.to.clone();
        true
    }

    fn label(&self) -> &str {
        "Rename Skin"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete a skin and everything it holds.
pub struct RemoveSkin {
    id: SkinId,
    removed: Option<Skin>,
}

impl RemoveSkin {
    pub fn new(id: SkinId) -> Self {
        Self { id, removed: None }
    }
}

impl EditCommand for RemoveSkin {
    fn apply(&mut self, doc: &mut Document) {
        // The default skin is the fallback every resolution ends at; deleting it
        // would make slots with no override draw nothing.
        if self.id == doc.skeleton.default_skin {
            return;
        }
        self.removed = doc.skeleton.skins.remove(self.id);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(skin) = self.removed.take() {
            // A fresh id — the slotmap key is gone. Nothing references a skin by
            // id except the session, which falls back to the default.
            self.id = doc.skeleton.add_skin(skin);
        }
    }

    fn label(&self) -> &str {
        "Delete Skin"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Copy every attachment from one skin into another, overwriting on conflict.
pub struct CopyAttachments {
    from: SkinId,
    to: SkinId,
    before: Option<Skin>,
}

impl CopyAttachments {
    pub fn new(from: SkinId, to: SkinId) -> Self {
        Self {
            from,
            to,
            before: None,
        }
    }
}

impl EditCommand for CopyAttachments {
    fn apply(&mut self, doc: &mut Document) {
        if self.from == self.to {
            return;
        }
        let Some(source) = doc.skeleton.skins.get(self.from).cloned() else {
            return;
        };
        let Some(target) = doc.skeleton.skins.get_mut(self.to) else {
            return;
        };
        self.before = Some(target.clone());
        for (key, attachment) in source.entries {
            target.entries.insert(key, attachment);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(target)) =
            (self.before.take(), doc.skeleton.skins.get_mut(self.to))
        {
            *target = before;
        }
    }

    fn label(&self) -> &str {
        "Copy Attachments"
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

    #[test]
    fn a_duplicate_skin_name_is_refused() {
        let mut doc = Document::new();
        let mut history = History::default();
        history.push(Box::new(AddSkin::new("outfit")), &mut doc);
        let count = doc.skeleton.skins.len();
        history.push(Box::new(AddSkin::new("outfit")), &mut doc);
        assert_eq!(
            doc.skeleton.skins.len(),
            count,
            "names address skins on disk, so two cannot share one"
        );
    }

    #[test]
    fn duplicating_a_skin_copies_its_entries() {
        use ankhimate_core::attachment::{Attachment, ClippingAttachment};
        use ankhimate_core::skeleton::Bone;
        use ankhimate_core::slot::Slot;

        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 1.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = doc.skeleton.add_slot(Slot::new("art".into(), bone));
        let default = doc.skeleton.default_skin;
        doc.skeleton.skins[default].set(
            slot,
            "thing".to_string(),
            Attachment::Clipping(ClippingAttachment::default()),
        );

        let mut history = History::default();
        history.push(Box::new(AddSkin::duplicating("copy", default)), &mut doc);
        let copy = doc
            .skeleton
            .skins
            .iter()
            .find(|(_, s)| s.name == "copy")
            .map(|(id, _)| id)
            .expect("the duplicate exists");
        assert!(doc.skeleton.skins[copy].get(slot, "thing").is_some());

        history.undo(&mut doc);
        assert!(doc.skeleton.skins.values().all(|s| s.name != "copy"));
    }

    #[test]
    fn the_default_skin_cannot_be_deleted() {
        let mut doc = Document::new();
        let default = doc.skeleton.default_skin;
        let mut history = History::default();
        history.push(Box::new(RemoveSkin::new(default)), &mut doc);
        assert!(
            doc.skeleton.skins.contains_key(default),
            "every resolution falls back to it"
        );
    }

    #[test]
    fn renaming_merges_into_one_undo_step() {
        let mut doc = Document::new();
        let default = doc.skeleton.default_skin;
        let original = doc.skeleton.skins[default].name.clone();
        let mut history = History::default();
        for name in ["d", "de", "dev"] {
            history.push(Box::new(RenameSkin::new(default, name)), &mut doc);
        }
        assert_eq!(doc.skeleton.skins[default].name, "dev");
        history.undo(&mut doc);
        assert_eq!(
            doc.skeleton.skins[default].name, original,
            "typing a name is one edit"
        );
    }
}
