//! Undo/redo via the command pattern (PLAN §3.2, defect D6).
//!
//! Every mutation of the [`Document`] goes through [`History::push`]. Commands
//! store the *minimum* needed to reverse themselves — not a JSON snapshot of the
//! whole skeleton, which is what the pre-T-107 history did and why undo cost grew
//! with project size.
//!
//! # Drag coalescing
//!
//! A drag would otherwise push a command per frame. Two mechanisms prevent that:
//!
//! * [`EditCommand::merge`] lets a command absorb its successor, so a stream of
//!   same-target edits collapses into one undo step;
//! * live drags write into `Session::preview_locals` and issue a single command on
//!   mouse-up, so the document is never touched mid-drag (defect D7).

pub mod asset_cmds;
pub mod attachment_cmds;
pub mod bone_cmds;
pub mod clip_cmds;
pub mod constraint_cmds;
pub mod event_cmds;
pub mod key_cmds;
pub mod mesh_cmds;
pub mod slot_cmds;
pub mod weight_cmds;

use crate::doc::Document;
use ankhimate_core::ids::BoneId;

/// Bone ids that changed identity because a command re-created an entity.
///
/// `slotmap` deliberately has no key-preserving reinsert, so undoing a delete
/// hands the restored bone a **new** key. Any other command still holding the old
/// key would silently no-op. After each apply/revert, `History` collects these
/// remaps and rewrites the rest of the stack.
#[derive(Debug, Default)]
pub struct IdRemap {
    pub bones: Vec<(BoneId, BoneId)>,
}

impl IdRemap {
    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    pub fn remap_bone(&mut self, from: BoneId, to: BoneId) {
        self.bones.push((from, to));
    }

    /// The id `bone` has become, or `bone` itself.
    pub fn bone(&self, bone: BoneId) -> BoneId {
        self.bones
            .iter()
            .find(|(from, _)| *from == bone)
            .map(|(_, to)| *to)
            .unwrap_or(bone)
    }
}

/// A reversible edit to the document.
pub trait EditCommand {
    /// Apply the edit. Called once when dispatched, and again on redo.
    fn apply(&mut self, doc: &mut Document);
    /// Restore the document to its pre-[`apply`](Self::apply) state.
    fn revert(&mut self, doc: &mut Document);
    /// Absorb `next` into `self` if they are the same logical edit continuing
    /// (e.g. successive frames of one drag). Return `false` to keep them
    /// separate — the default, since merging is opt-in per command.
    fn merge(&mut self, _next: &dyn EditCommand) -> bool {
        false
    }
    /// Human-readable name for the Edit menu ("Undo Move Bone").
    fn label(&self) -> &str;

    /// The work mode this command may run in, or `None` when it is legal in both
    /// (T-207). Structural edits — anything that changes what the rig *is* rather
    /// than what it is *doing* — return `Some(WorkMode::Setup)`, and
    /// [`History::push_in_mode`] refuses them elsewhere. Making the rule a
    /// property of the command rather than a UI convention is what makes it
    /// testable: a panel cannot forget to check.
    fn requires_mode(&self) -> Option<crate::session::WorkMode> {
        None
    }

    /// Report entity ids this command just re-created under a new key, so the
    /// rest of the history can be rewritten. Only commands that re-insert
    /// entities need to implement this.
    fn take_remap(&mut self) -> IdRemap {
        IdRemap::default()
    }

    /// Rewrite any entity ids this command holds. Called on every *other* command
    /// in the history after a sibling re-creates an entity.
    fn apply_remap(&mut self, _remap: &IdRemap) {}

    /// Type-erased downcast support, so `merge` can inspect a concrete successor.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Bounded undo/redo stacks.
pub struct History {
    undo: Vec<Box<dyn EditCommand>>,
    redo: Vec<Box<dyn EditCommand>>,
    cap: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            cap,
        }
    }

    /// Apply `cmd` and record it for undo.
    ///
    /// If the top of the undo stack accepts `cmd` via
    /// [`merge`](EditCommand::merge), the two collapse into one step instead of
    /// stacking — but `cmd` is still applied first so the document ends up in the
    /// same place either way.
    pub fn push(&mut self, mut cmd: Box<dyn EditCommand>, doc: &mut Document) {
        cmd.apply(doc);
        self.redo.clear();

        if let Some(top) = self.undo.last_mut()
            && top.merge(cmd.as_ref())
        {
            return;
        }

        self.undo.push(cmd);
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
    }

    /// Apply `cmd` only if `mode` allows it (T-207).
    ///
    /// Returns `false` — leaving the document and both stacks untouched — when
    /// the command declares a `requires_mode` that is not the current one. The
    /// caller turns that into a status message; nothing half-applies.
    pub fn push_in_mode(
        &mut self,
        cmd: Box<dyn EditCommand>,
        doc: &mut Document,
        mode: crate::session::WorkMode,
    ) -> bool {
        if let Some(required) = cmd.requires_mode()
            && required != mode
        {
            return false;
        }
        self.push(cmd, doc);
        true
    }

    /// Record a command that has **already** been applied to the document.
    ///
    /// For interactions that mutated the document as they went (or that computed
    /// their result from live state) and only need the undo entry.
    pub fn push_applied(&mut self, cmd: Box<dyn EditCommand>) {
        self.redo.clear();
        self.undo.push(cmd);
        if self.undo.len() > self.cap {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        match self.undo.pop() {
            Some(mut cmd) => {
                cmd.revert(doc);
                let remap = cmd.take_remap();
                self.redo.push(cmd);
                self.broadcast_remap(&remap);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        match self.redo.pop() {
            Some(mut cmd) => {
                cmd.apply(doc);
                let remap = cmd.take_remap();
                self.undo.push(cmd);
                self.broadcast_remap(&remap);
                true
            }
            None => false,
        }
    }

    /// Rewrite ids across the whole history after a command re-created an entity
    /// under a new key. Without this, an older command holding the dead key would
    /// silently no-op (see `IdRemap`).
    fn broadcast_remap(&mut self, remap: &IdRemap) {
        if remap.is_empty() {
            return;
        }
        for cmd in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            cmd.apply_remap(remap);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the next undo step, for the Edit menu.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|c| c.label())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|c| c.label())
    }

    /// Number of recorded undo steps — for tests and diagnostics.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bone_cmds::{CreateBone, SetBoneTransform};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

    fn new_bone(name: &str) -> Bone {
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
    fn undo_redo_roundtrips_a_create() {
        let mut doc = Document::new();
        let mut history = History::default();

        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        assert_eq!(doc.skeleton.bones.len(), 1);

        assert!(history.undo(&mut doc));
        assert_eq!(doc.skeleton.bones.len(), 0);

        assert!(history.redo(&mut doc));
        assert_eq!(doc.skeleton.bones.len(), 1);
    }

    #[test]
    fn undo_on_empty_history_is_a_no_op() {
        let mut doc = Document::new();
        let mut history = History::default();
        assert!(!history.undo(&mut doc));
        assert!(!history.redo(&mut doc));
    }

    #[test]
    fn new_edit_clears_the_redo_stack() {
        let mut doc = Document::new();
        let mut history = History::default();

        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        history.undo(&mut doc);
        assert!(history.can_redo());

        history.push(Box::new(CreateBone::new(new_bone("b"))), &mut doc);
        assert!(
            !history.can_redo(),
            "diverging edit must drop the redo branch"
        );
    }

    #[test]
    fn history_is_capped() {
        let mut doc = Document::new();
        let mut history = History::new(3);
        for i in 0..10 {
            history.push(
                Box::new(CreateBone::new(new_bone(&format!("b{i}")))),
                &mut doc,
            );
        }
        assert_eq!(history.undo_depth(), 3);
        assert_eq!(doc.skeleton.bones.len(), 10, "all edits still applied");
    }

    #[test]
    fn same_bone_transform_edits_merge_into_one_step() {
        let mut doc = Document::new();
        let mut history = History::default();
        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        let bone = *doc.skeleton.update_order.first().unwrap();
        let original = doc.skeleton.bones[bone].local_transform;

        // A drag's worth of successive transform edits.
        for x in 1..=5 {
            let mut t = original;
            t.position.x = x as f32 * 10.0;
            history.push(Box::new(SetBoneTransform::new(bone, t)), &mut doc);
        }

        // One create + one merged move.
        assert_eq!(history.undo_depth(), 2);
        assert_eq!(doc.skeleton.bones[bone].local_transform.position.x, 50.0);

        // Undoing the merged step returns to the pre-drag value in one go.
        history.undo(&mut doc);
        assert_eq!(
            doc.skeleton.bones[bone].local_transform.position,
            original.position
        );
    }

    #[test]
    fn transform_edits_on_different_bones_do_not_merge() {
        let mut doc = Document::new();
        let mut history = History::default();
        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        history.push(Box::new(CreateBone::new(new_bone("b"))), &mut doc);
        let order = doc.skeleton.update_order.clone();
        let (a, b) = (order[0], order[1]);

        let mut t = Transform::default();
        t.position.x = 5.0;
        history.push(Box::new(SetBoneTransform::new(a, t)), &mut doc);
        history.push(Box::new(SetBoneTransform::new(b, t)), &mut doc);

        // 2 creates + 2 distinct moves.
        assert_eq!(history.undo_depth(), 4);
    }

    #[test]
    fn labels_track_the_stacks() {
        let mut doc = Document::new();
        let mut history = History::default();
        assert!(history.undo_label().is_none());

        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        assert_eq!(history.undo_label(), Some("Create Bone"));

        history.undo(&mut doc);
        assert!(history.undo_label().is_none());
        assert_eq!(history.redo_label(), Some("Create Bone"));
    }

    #[test]
    fn undoing_past_a_restored_bone_still_works() {
        // Regression: `slotmap` has no key-preserving reinsert, so undoing a
        // delete gives the bone a NEW key. An older `CreateBone` holding the old
        // key used to silently no-op, leaving the bone behind forever.
        use crate::commands::bone_cmds::DeleteBone;

        let mut doc = Document::new();
        let mut history = History::default();

        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        history.push(Box::new(CreateBone::new(new_bone("b"))), &mut doc);
        let b = doc.skeleton.update_order[1];

        history.push(Box::new(DeleteBone::new(b)), &mut doc);
        assert_eq!(doc.skeleton.bones.len(), 1);

        // Undo the delete: `b` returns under a different key.
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 2);

        // Undo the create of `b` — must actually remove it.
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 1, "create-undo found the new key");

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 0);
    }

    #[test]
    fn transform_command_follows_a_remapped_bone() {
        use crate::commands::bone_cmds::DeleteBone;

        let mut doc = Document::new();
        let mut history = History::default();
        history.push(Box::new(CreateBone::new(new_bone("a"))), &mut doc);
        let a = doc.skeleton.update_order[0];

        let posed = Transform {
            position: glam::vec2(30.0, 0.0),
            ..Default::default()
        };
        history.push(Box::new(SetBoneTransform::new(a, posed)), &mut doc);
        history.push(Box::new(DeleteBone::new(a)), &mut doc);

        // Undo the delete (new key), then the transform.
        history.undo(&mut doc);
        let restored = doc.skeleton.update_order[0];
        assert_eq!(
            doc.skeleton.bones[restored].local_transform.position.x,
            30.0
        );

        history.undo(&mut doc);
        assert_eq!(
            doc.skeleton.bones[restored].local_transform.position.x, 0.0,
            "transform-undo followed the remapped id"
        );
    }
}
