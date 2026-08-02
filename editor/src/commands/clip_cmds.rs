//! Clipping attachment authoring as undoable commands (T-405).
//!
//! A clip is rig structure — it decides what the artwork *is*, not how it moves
//! — so everything here is Setup-only (T-207).
//!
//! Like [`crate::commands::mesh_cmds`], each command snapshots the attachment
//! rather than inverting its edit. A clip polygon is a handful of points and the
//! snapshot is a `clone`; hand-written inverses would buy nothing but bugs.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::attachment::{Attachment, ClippingAttachment};
use ankhimate_core::ids::{SkinId, SlotId};

fn clip_mut<'a>(
    doc: &'a mut Document,
    skin: SkinId,
    slot: SlotId,
    name: &str,
) -> Option<&'a mut ClippingAttachment> {
    match doc
        .skeleton
        .skins
        .get_mut(skin)?
        .entries
        .get_mut(&(slot, name.to_string()))?
    {
        Attachment::Clipping(clip) => Some(clip),
        _ => None,
    }
}

/// Add a clipping attachment to a slot, with a starting quad.
///
/// The polygon starts as a real rectangle rather than empty: a clip with no
/// vertices masks nothing, so an empty one would look like the command silently
/// failed. Sized from the slot's own art where there is any, so the first drag
/// is an adjustment rather than a construction.
pub struct AddClipping {
    skin: SkinId,
    slot: SlotId,
    name: String,
    size: f32,
    added: bool,
}

impl AddClipping {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, size: f32) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            size: size.max(1.0),
            added: false,
        }
    }
}

impl EditCommand for AddClipping {
    fn apply(&mut self, doc: &mut Document) {
        let Some(skin) = doc.skeleton.skins.get_mut(self.skin) else {
            return;
        };
        if skin.get(self.slot, &self.name).is_some() {
            return; // Name already taken in this skin.
        }
        let half = self.size * 0.5;
        let clip = ClippingAttachment {
            vertices: vec![
                glam::vec2(-half, -half),
                glam::vec2(half, -half),
                glam::vec2(half, half),
                glam::vec2(-half, half),
            ],
            end_slot: None,
        };
        skin.set(self.slot, self.name.clone(), Attachment::Clipping(clip));
        self.added = true;
        // A slot shows whichever attachment it names, so pointing it at the new
        // clip is what makes it take effect.
        if let Some(slot) = doc.skeleton.slots.get_mut(self.slot) {
            slot.attachment = Some(self.name.clone());
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if !self.added {
            return;
        }
        if let Some(skin) = doc.skeleton.skins.get_mut(self.skin) {
            skin.remove(self.slot, &self.name);
        }
        if let Some(slot) = doc.skeleton.slots.get_mut(self.slot)
            && slot.attachment.as_deref() == Some(self.name.as_str())
        {
            slot.attachment = None;
        }
        self.added = false;
    }

    fn label(&self) -> &str {
        "Add Clipping"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What an edit does to a clip polygon.
pub enum ClipEdit {
    /// Move vertices to new local positions. Merges, so a drag is one step.
    MoveVertices(Vec<(usize, glam::Vec2)>),
    /// Insert a vertex at an index, keeping the perimeter order.
    InsertVertex(usize, glam::Vec2),
    /// Remove vertices by index.
    RemoveVertices(Vec<usize>),
    /// Point the clip at the slot it stops after, or `None` to clip to the end.
    SetEndSlot(Option<String>),
}

/// Apply a [`ClipEdit`] to one clipping attachment.
pub struct EditClip {
    skin: SkinId,
    slot: SlotId,
    name: String,
    edit: ClipEdit,
    before: Option<ClippingAttachment>,
    label: &'static str,
}

impl EditClip {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, edit: ClipEdit) -> Self {
        let label = match &edit {
            ClipEdit::MoveVertices(_) => "Move Clip Vertices",
            ClipEdit::InsertVertex(_, _) => "Add Clip Vertex",
            ClipEdit::RemoveVertices(_) => "Delete Clip Vertices",
            ClipEdit::SetEndSlot(_) => "Set Clip Range",
        };
        Self {
            skin,
            slot,
            name: name.into(),
            edit,
            before: None,
            label,
        }
    }
}

impl EditCommand for EditClip {
    fn apply(&mut self, doc: &mut Document) {
        let capture = self.before.is_none();
        let Some(clip) = clip_mut(doc, self.skin, self.slot, &self.name) else {
            return;
        };
        if capture {
            self.before = Some(clip.clone());
        }

        match &self.edit {
            ClipEdit::MoveVertices(moves) => {
                for (index, position) in moves {
                    if let Some(vertex) = clip.vertices.get_mut(*index) {
                        *vertex = *position;
                    }
                }
            }
            ClipEdit::InsertVertex(index, position) => {
                let at = (*index).min(clip.vertices.len());
                clip.vertices.insert(at, *position);
            }
            ClipEdit::RemoveVertices(indices) => {
                // Below three points a polygon has no interior, and a clip with
                // no interior masks everything — which reads as "the rig
                // vanished", not as an edit.
                if clip.vertices.len().saturating_sub(indices.len()) < 3 {
                    self.before = None;
                    return;
                }
                let mut sorted = indices.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                sorted.dedup();
                for index in sorted {
                    if index < clip.vertices.len() {
                        clip.vertices.remove(index);
                    }
                }
            }
            ClipEdit::SetEndSlot(end) => clip.end_slot = end.clone(),
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(clip)) = (
            self.before.take(),
            clip_mut(doc, self.skin, self.slot, &self.name),
        ) {
            *clip = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditClip>() else {
            return false;
        };
        if other.skin != self.skin || other.slot != self.slot || other.name != self.name {
            return false;
        }
        match (&mut self.edit, &other.edit) {
            (ClipEdit::MoveVertices(ours), ClipEdit::MoveVertices(theirs)) => {
                *ours = theirs.clone();
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        self.label
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
    use ankhimate_core::slot::Slot;

    fn doc_with_slot() -> (Document, SkinId, SlotId) {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = doc.skeleton.add_slot(Slot::new("mask".to_string(), bone));
        let skin = doc.skeleton.default_skin;
        (doc, skin, slot)
    }

    #[test]
    fn adding_a_clip_gives_it_a_polygon_and_points_the_slot_at_it() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(
            Box::new(AddClipping::new(skin, slot, "clip", 100.0)),
            &mut doc,
        );

        let Some(Attachment::Clipping(clip)) = doc.skeleton.skins[skin].get(slot, "clip") else {
            panic!("the clip was created");
        };
        assert_eq!(clip.vertices.len(), 4, "a usable starting quad");
        assert_eq!(
            doc.skeleton.slots[slot].attachment.as_deref(),
            Some("clip"),
            "the slot shows it, or it would do nothing"
        );

        history.undo(&mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "clip").is_none());
        assert_eq!(doc.skeleton.slots[slot].attachment, None);
    }

    #[test]
    fn a_clip_polygon_never_drops_below_three_points() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(
            Box::new(AddClipping::new(skin, slot, "clip", 100.0)),
            &mut doc,
        );
        history.push(
            Box::new(EditClip::new(
                skin,
                slot,
                "clip",
                ClipEdit::RemoveVertices(vec![0, 1]),
            )),
            &mut doc,
        );

        let Some(Attachment::Clipping(clip)) = doc.skeleton.skins[skin].get(slot, "clip") else {
            panic!("still there");
        };
        assert_eq!(
            clip.vertices.len(),
            4,
            "the removal was refused, not half-applied"
        );
    }

    #[test]
    fn dragging_the_polygon_is_one_undo_step() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(
            Box::new(AddClipping::new(skin, slot, "clip", 100.0)),
            &mut doc,
        );
        // Two frames of one drag.
        for x in [10.0, 20.0] {
            history.push(
                Box::new(EditClip::new(
                    skin,
                    slot,
                    "clip",
                    ClipEdit::MoveVertices(vec![(0, glam::vec2(x, 0.0))]),
                )),
                &mut doc,
            );
        }
        history.undo(&mut doc);

        let Some(Attachment::Clipping(clip)) = doc.skeleton.skins[skin].get(slot, "clip") else {
            panic!("still there");
        };
        assert_eq!(
            clip.vertices[0],
            glam::vec2(-50.0, -50.0),
            "one undo returns to before the whole drag"
        );
    }
}
