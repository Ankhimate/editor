//! Timeline marker authoring as undoable commands (T-906).
//!
//! Markers are notes an animator leaves on the ruler — "contact", "down",
//! "passing", "up" on a walk cycle — so a clip's structure is written down
//! rather than counted out every time it is opened.
//!
//! Deliberately a separate module from `event_cmds`, mirroring the separation in
//! the model: an event fires into the running game, a marker never leaves the
//! editor. Sharing commands between them would make it one edit away for a note
//! to become something gameplay reacts to.
//!
//! Animate-only, for the same reason events are: a marker marks a moment in a
//! clip, and there is no clip in Setup mode to mark.
//!
//! Each command snapshots the clip's marker list rather than inverting its edit.
//! The list is a handful of small structs, and the alternative — index-based
//! inverses that survive a re-sort — is where the bugs would be.

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::animation::Marker;
use ankhimate_core::ids::AnimationId;

fn markers_mut(doc: &mut Document, anim: AnimationId) -> Option<&mut Vec<Marker>> {
    doc.animations.get_mut(anim).map(|a| &mut a.markers)
}

/// Add a marker at a time.
pub struct AddMarker {
    anim: AnimationId,
    marker: Marker,
    before: Option<Vec<Marker>>,
}

impl AddMarker {
    pub fn new(anim: AnimationId, name: impl Into<String>, time: f32) -> Self {
        Self {
            anim,
            marker: Marker::new(time.max(0.0), name),
            before: None,
        }
    }
}

impl EditCommand for AddMarker {
    fn apply(&mut self, doc: &mut Document) {
        let Some(markers) = markers_mut(doc, self.anim) else {
            return;
        };
        self.before = Some(markers.clone());
        markers.push(self.marker.clone());
        markers.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(markers)) = (self.before.take(), markers_mut(doc, self.anim)) {
            *markers = before;
        }
    }

    fn label(&self) -> &str {
        "Add Marker"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What [`EditMarker`] does to the marker it names.
pub enum MarkerEdit {
    /// Move it in time. Merges, so dragging one is a single undo step.
    SetTime(f32),
    Rename(String),
    SetColor([f32; 4]),
    Remove,
}

/// Retime, rename, recolour or delete one marker, by index.
pub struct EditMarker {
    anim: AnimationId,
    index: usize,
    edit: MarkerEdit,
    before: Option<Vec<Marker>>,
}

impl EditMarker {
    pub fn new(anim: AnimationId, index: usize, edit: MarkerEdit) -> Self {
        Self {
            anim,
            index,
            edit,
            before: None,
        }
    }
}

impl EditCommand for EditMarker {
    fn apply(&mut self, doc: &mut Document) {
        let Some(markers) = markers_mut(doc, self.anim) else {
            return;
        };
        if self.index >= markers.len() {
            return;
        }
        if self.before.is_none() {
            self.before = Some(markers.clone());
        }
        // Re-applied from the *original* list on a merged drag, so successive
        // retimes do not compound: `SetTime(3)` after `SetTime(2)` must mean
        // "3", not "2 then 3 again from wherever 2 left it".
        if let Some(before) = &self.before {
            *markers = before.clone();
        }
        match &self.edit {
            MarkerEdit::SetTime(time) => {
                markers[self.index].time = time.max(0.0);
                markers.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
            MarkerEdit::Rename(name) => markers[self.index].name = name.clone(),
            MarkerEdit::SetColor(color) => markers[self.index].color = *color,
            MarkerEdit::Remove => {
                markers.remove(self.index);
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(markers)) = (self.before.take(), markers_mut(doc, self.anim)) {
            *markers = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditMarker>() else {
            return false;
        };
        // Only a drag merges, and only on the same marker. A rename followed by
        // a retime are two things the user did, and one undo for both would be
        // a surprise.
        match (&self.edit, &other.edit) {
            (MarkerEdit::SetTime(_), MarkerEdit::SetTime(to))
                if self.anim == other.anim && self.index == other.index =>
            {
                self.edit = MarkerEdit::SetTime(*to);
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        match self.edit {
            MarkerEdit::SetTime(_) => "Move Marker",
            MarkerEdit::Rename(_) => "Rename Marker",
            MarkerEdit::SetColor(_) => "Recolour Marker",
            MarkerEdit::Remove => "Delete Marker",
        }
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a bone's sampling offset within a clip (T-905).
///
/// Lives beside the marker commands because both are clip-level furniture that
/// moves *time* rather than values — and neither touches a key.
pub struct SetBoneOffset {
    anim: AnimationId,
    bone: ankhimate_core::ids::BoneId,
    after: f32,
    before: Option<f32>,
}

impl SetBoneOffset {
    pub fn new(anim: AnimationId, bone: ankhimate_core::ids::BoneId, after: f32) -> Self {
        Self {
            anim,
            bone,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetBoneOffset {
    fn apply(&mut self, doc: &mut Document) {
        let Some(clip) = doc.animations.get_mut(self.anim) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(clip.bone_offset(self.bone));
        }
        clip.set_bone_offset(self.bone, self.after);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(clip)) = (self.before.take(), doc.animations.get_mut(self.anim))
        {
            clip.set_bone_offset(self.bone, before);
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetBoneOffset>() else {
            return false;
        };
        if other.anim != self.anim || other.bone != self.bone {
            return false;
        }
        // Dragging the field is one edit.
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Set Track Offset"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Animate)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::animation::Animation;

    fn doc_with_clip() -> (Document, AnimationId) {
        let mut doc = Document::new();
        let id = doc.animations.insert(Animation::new("walk", 1.0));
        (doc, id)
    }

    #[test]
    fn markers_are_added_in_time_order_whatever_order_they_arrive() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        for (time, name) in [(0.75, "up"), (0.0, "contact"), (0.5, "passing")] {
            history.push(Box::new(AddMarker::new(anim, name, time)), &mut doc);
        }
        let names: Vec<&str> = doc.animations[anim]
            .markers
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["contact", "passing", "up"]);

        history.undo(&mut doc);
        assert_eq!(
            doc.animations[anim].markers.len(),
            2,
            "one undo, one marker"
        );
    }

    /// Dragging a marker is one undo, and it lands where the drag ended rather
    /// than compounding each frame's move onto the last.
    #[test]
    fn dragging_a_marker_merges_and_does_not_compound() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddMarker::new(anim, "contact", 0.1)), &mut doc);

        for time in [0.2, 0.35, 0.5] {
            history.push(
                Box::new(EditMarker::new(anim, 0, MarkerEdit::SetTime(time))),
                &mut doc,
            );
        }
        assert!((doc.animations[anim].markers[0].time - 0.5).abs() < 1e-6);

        history.undo(&mut doc);
        assert!(
            (doc.animations[anim].markers[0].time - 0.1).abs() < 1e-6,
            "one undo returns to before the drag"
        );
    }

    /// A retime that reorders the list still undoes cleanly — the snapshot is
    /// what makes that true, where an index-based inverse would be pointing at
    /// the wrong marker by then.
    #[test]
    fn retiming_past_a_neighbour_reorders_and_still_undoes() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddMarker::new(anim, "a", 0.1)), &mut doc);
        history.push(Box::new(AddMarker::new(anim, "b", 0.2)), &mut doc);

        // Drag "a" past "b".
        history.push(
            Box::new(EditMarker::new(anim, 0, MarkerEdit::SetTime(0.9))),
            &mut doc,
        );
        let names: Vec<&str> = doc.animations[anim]
            .markers
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["b", "a"], "list re-sorted");

        history.undo(&mut doc);
        let names: Vec<&str> = doc.animations[anim]
            .markers
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"], "and came back");
    }

    #[test]
    fn removing_a_marker_undoes() {
        let (mut doc, anim) = doc_with_clip();
        let mut history = History::default();
        history.push(Box::new(AddMarker::new(anim, "contact", 0.0)), &mut doc);
        history.push(
            Box::new(EditMarker::new(anim, 0, MarkerEdit::Remove)),
            &mut doc,
        );
        assert!(doc.animations[anim].markers.is_empty());

        history.undo(&mut doc);
        assert_eq!(doc.animations[anim].markers.len(), 1);
        assert_eq!(doc.animations[anim].markers[0].name, "contact");
    }
}
