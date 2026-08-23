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
//!
//! # Commands and operators
//!
//! An [`EditCommand`] is an *instance*: "move bone `b` from here to there",
//! holding what it needs to reverse itself. That is the wrong thing for a
//! keymap or a plugin to name — they want the verb, not one occurrence of it.
//!
//! `registry::Operator` is the editor-side verb: a stable string id, an applicability
//! test, and an `invoke` that reads live state and dispatches whatever command
//! the situation calls for. Operators sit *above* commands and never replace
//! them, so undo, drag-merge and the T-207 mode rule keep their single
//! enforcement point.

pub mod asset_cmds;
pub mod attachment_cmds;
pub mod bone_cmds;
pub mod clip_cmds;
pub mod constraint_cmds;
pub mod create_attachment_cmds;
pub mod document_cmds;
pub mod event_cmds;
pub mod export_cmds;
pub mod group_cmds;
pub mod key_cmds;
pub mod marker_cmds;
pub mod mesh_cmds;
pub mod psd_cmds;
pub mod skin_cmds;
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

    /// Fold another remap into this one, for a command that groups others.
    pub fn extend(&mut self, other: IdRemap) {
        self.bones.extend(other.bones);
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
    fn requires_mode(&self) -> Option<crate::WorkMode> {
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

/// Several commands that undo as one step.
///
/// The case this exists for is a plugin: one click of a panel button can invoke
/// five verbs, and five presses of Ctrl-Z to take back one click is not undo, it
/// is a puzzle. The same is true of any caller that produces a batch from a
/// single gesture.
///
/// Built from commands that are **already applied** — the plugin ran against a
/// real document and the edits are in it. `apply` therefore re-applies, which is
/// what redo needs, and the group is pushed with [`History::push_applied`] so it
/// is not applied twice on the way in.
pub struct Group {
    /// In the order they were applied. Undone in reverse.
    commands: Vec<Box<dyn EditCommand>>,
    label: String,
}

impl Group {
    pub fn new(commands: Vec<Box<dyn EditCommand>>, label: impl Into<String>) -> Self {
        Self {
            commands,
            label: label.into(),
        }
    }

    /// Is there anything in it?
    ///
    /// A caller with nothing to group should push nothing: an undo step that
    /// undoes nothing is a keypress the user spends finding out it did nothing.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl EditCommand for Group {
    fn apply(&mut self, doc: &mut Document) {
        for command in &mut self.commands {
            command.apply(doc);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        // Reverse order. A command that created a bone and a command that moved
        // it undo the other way round, or the move is reverted against a bone
        // that is already gone.
        for command in self.commands.iter_mut().rev() {
            command.revert(doc);
        }
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn requires_mode(&self) -> Option<crate::WorkMode> {
        // The mode was checked per command as each was dispatched. Asking again
        // for the group would refuse a batch that legitimately spans both — a
        // plugin that adds a bone and keys it does structural work and animation
        // work in one click.
        None
    }

    fn take_remap(&mut self) -> IdRemap {
        // Collected across the whole group: a caller re-creating an entity has
        // to tell every other command in the history, and which member of the
        // group did it is not their concern.
        let mut remap = IdRemap::default();
        for command in &mut self.commands {
            remap.extend(command.take_remap());
        }
        remap
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        for command in &mut self.commands {
            command.apply_remap(remap);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
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
        mode: crate::WorkMode,
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

    /// Take every applied command out, leaving the history empty.
    ///
    /// For a caller that ran commands against a throwaway `Edit` and now wants
    /// them on its own stack — a plugin panel, an importer. The commands are
    /// already applied to the document that travelled with them, so the caller
    /// pushes them with [`Self::push_applied`] rather than dispatching again.
    ///
    /// Redo is dropped, not returned: a throwaway history has nothing on it that
    /// the caller's own redo stack should inherit.
    pub fn take_applied(&mut self) -> Vec<Box<dyn EditCommand>> {
        self.redo.clear();
        std::mem::take(&mut self.undo)
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

    #[test]
    fn a_group_undoes_as_one_step() {
        // The case it exists for: one click of a plugin panel button can invoke
        // five verbs, and five presses of Ctrl-Z to take back one click is not
        // undo, it is a puzzle.
        let mut doc = Document::new();
        let mut history = History::default();

        // Applied against a throwaway document first, the way a plugin's are.
        let mut side = Document::new();
        let mut made: Vec<Box<dyn EditCommand>> = Vec::new();
        for name in ["a", "b", "c"] {
            let mut cmd = bone_cmds::CreateBone::new(ankhimate_core::skeleton::Bone {
                name: name.into(),
                parent: None,
                length: 10.0,
                local_transform: Default::default(),
                inherit: Default::default(),
                color: ankhimate_core::skeleton::Bone::default_color(),
            });
            cmd.apply(&mut side);
            made.push(Box::new(cmd));
        }
        doc = side;
        assert_eq!(doc.skeleton.bones.len(), 3);

        history.push_applied(Box::new(Group::new(made, "three bones")));
        assert_eq!(history.undo_depth(), 1, "one step, not three");

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 0, "and it took all three back");

        history.redo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 3, "redo put them all back");
    }

    #[test]
    fn a_group_undoes_in_reverse_order() {
        // Order is observable through the *name*: create-then-rename undone
        // forwards puts the bone back as `arm` and then reverts a rename that
        // never happened to it, leaving the wrong name. Undone backwards the
        // rename goes first and the bone comes back out entirely.
        //
        // Checked on a group of two renames rather than a create, because a
        // create's revert removes the bone and every later revert then finds
        // nothing to do — which hides the order rather than testing it.
        let mut doc = Document::new();
        let mut create = bone_cmds::CreateBone::new(ankhimate_core::skeleton::Bone {
            name: "a".into(),
            parent: None,
            length: 10.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: ankhimate_core::skeleton::Bone::default_color(),
        });
        create.apply(&mut doc);
        let bone = doc
            .skeleton
            .bones
            .iter()
            .next()
            .map(|(id, _)| id)
            .expect("the bone");

        let mut first = bone_cmds::RenameBone::new(bone, "b");
        first.apply(&mut doc);
        let mut second = bone_cmds::RenameBone::new(bone, "c");
        second.apply(&mut doc);
        assert_eq!(doc.skeleton.bones[bone].name, "c");

        let mut history = History::default();
        history.push_applied(Box::new(Group::new(
            vec![Box::new(first), Box::new(second)],
            "two renames",
        )));
        history.undo(&mut doc);

        assert_eq!(
            doc.skeleton.bones[bone].name, "a",
            "reverting `c`→`b` first and then `b`→`a` gets back to the start;              the other order lands on `b`"
        );
    }

    #[test]
    fn an_empty_group_is_recognisable_as_one() {
        // A caller with nothing to group should push nothing: an undo step that
        // undoes nothing is a keypress the user spends finding out it did
        // nothing.
        assert!(Group::new(Vec::new(), "nothing").is_empty());
    }
}
