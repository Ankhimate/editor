//! Putting an attachment into a skin, undoably.
//!
//! The editor creates attachments as a side effect of other gestures — dropping
//! an image on a slot, converting a region to a mesh — so there was never a
//! command that only did this. An importer needs one: it has a slot, a texture
//! name and some geometry, and no gesture to hang them off.
//!
//! Written as one command over [`Attachment`] rather than one per kind. The
//! variants differ in their payload and not in what adding them *means*, and a
//! `CreateRegion`/`CreateMesh`/`CreateClipping` split would be three copies of
//! the same undo.

use super::{EditCommand, IdRemap};
use crate::doc::Document;
use crate::work_mode::WorkMode;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::{SkinId, SlotId};

/// Add an attachment to a skin under a name.
pub struct CreateAttachment {
    skin: Option<SkinId>,
    slot: SlotId,
    name: String,
    attachment: Attachment,
    /// What was under that name before, so a replace undoes to it rather than
    /// to nothing. An importer reading a file twice would otherwise lose the
    /// first read's attachment with no way back.
    before: Option<Option<Attachment>>,
}

impl CreateAttachment {
    /// `skin` of `None` means the default skin, which is what an importer
    /// building a rig from scratch wants.
    pub fn new(
        skin: Option<SkinId>,
        slot: SlotId,
        name: impl Into<String>,
        attachment: Attachment,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            attachment,
            before: None,
        }
    }

    fn skin_id(&self, doc: &Document) -> SkinId {
        self.skin.unwrap_or(doc.skeleton.default_skin)
    }
}

impl EditCommand for CreateAttachment {
    fn apply(&mut self, doc: &mut Document) {
        let skin = self.skin_id(doc);
        let Some(skin) = doc.skeleton.skins.get_mut(skin) else {
            return;
        };
        // Captured on the *first* apply only, so a redo does not overwrite the
        // original with what the redo itself put there.
        if self.before.is_none() {
            self.before = Some(skin.get(self.slot, &self.name).cloned());
        }
        skin.set(self.slot, self.name.clone(), self.attachment.clone());
    }

    fn revert(&mut self, doc: &mut Document) {
        let skin = self.skin_id(doc);
        let Some(skin) = doc.skeleton.skins.get_mut(skin) else {
            return;
        };
        match self.before.clone().flatten() {
            Some(previous) => {
                skin.set(self.slot, self.name.clone(), previous);
            }
            None => {
                skin.remove(self.slot, &self.name);
            }
        }
    }

    fn label(&self) -> &str {
        "Add Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        // Structural: an attachment is what the rig *is*, not what it is doing.
        Some(WorkMode::Setup)
    }

    fn take_remap(&mut self) -> IdRemap {
        IdRemap::default()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::attachment::{Rect, RegionAttachment};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    fn region(texture: &str) -> Attachment {
        Attachment::Region(RegionAttachment {
            texture: texture.to_string(),
            local_offset: glam::Vec2::ZERO,
            local_rotation: 0.0,
            local_scale: glam::Vec2::ONE,
            width: 10.0,
            height: 10.0,
            uv_rect: Rect::default(),
            pivot: glam::Vec2::splat(0.5),
            sequence: None,
        })
    }

    fn rig() -> (Document, SlotId) {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = doc.skeleton.add_slot(Slot::new("body".to_string(), bone));
        (doc, slot)
    }

    #[test]
    fn an_attachment_is_added_and_undone() {
        let (mut doc, slot) = rig();
        let skin = doc.skeleton.default_skin;
        let mut history = History::default();

        history.push(
            Box::new(CreateAttachment::new(None, slot, "torso", region("img"))),
            &mut doc,
        );
        assert!(doc.skeleton.skins[skin].get(slot, "torso").is_some());

        history.undo(&mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "torso").is_none());
    }

    #[test]
    fn replacing_an_attachment_undoes_to_the_one_it_replaced() {
        // An importer run twice, or a plugin correcting itself, must not leave
        // the slot empty on undo — it was not empty before.
        let (mut doc, slot) = rig();
        let skin = doc.skeleton.default_skin;
        let mut history = History::default();

        history.push(
            Box::new(CreateAttachment::new(None, slot, "torso", region("first"))),
            &mut doc,
        );
        history.push(
            Box::new(CreateAttachment::new(None, slot, "torso", region("second"))),
            &mut doc,
        );

        let texture = |doc: &Document| match doc.skeleton.skins[skin].get(slot, "torso") {
            Some(Attachment::Region(r)) => r.texture.clone(),
            _ => panic!("expected a region"),
        };
        assert_eq!(texture(&doc), "second");

        history.undo(&mut doc);
        assert_eq!(texture(&doc), "first", "undone to what was there");
    }

    #[test]
    fn a_redo_does_not_lose_the_original() {
        // `before` is captured on the first apply only. Capturing it every time
        // would make a redo record its own output as the thing to undo to.
        let (mut doc, slot) = rig();
        let skin = doc.skeleton.default_skin;
        let mut history = History::default();

        history.push(
            Box::new(CreateAttachment::new(None, slot, "torso", region("first"))),
            &mut doc,
        );
        history.push(
            Box::new(CreateAttachment::new(None, slot, "torso", region("second"))),
            &mut doc,
        );
        history.undo(&mut doc);
        history.redo(&mut doc);
        history.undo(&mut doc);

        match doc.skeleton.skins[skin].get(slot, "torso") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "first"),
            _ => panic!("expected a region"),
        }
    }
}
