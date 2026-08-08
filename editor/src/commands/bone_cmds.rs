//! Bone mutations as undoable commands (PLAN §3.2).

use super::{EditCommand, IdRemap};
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::ids::BoneId;
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::Bone;

/// Insert a new bone.
///
/// On undo the bone is removed; on redo it is re-inserted. The slotmap hands out
/// a fresh key each time, so the id is re-read from the document rather than
/// cached by callers across an undo.
pub struct CreateBone {
    bone: Bone,
    /// Set once applied, so `revert` knows what to remove.
    created: Option<BoneId>,
}

impl CreateBone {
    pub fn new(bone: Bone) -> Self {
        Self {
            bone,
            created: None,
        }
    }

    /// The id assigned by the most recent `apply`.
    pub fn created_id(&self) -> Option<BoneId> {
        self.created
    }
}

impl EditCommand for CreateBone {
    fn apply(&mut self, doc: &mut Document) {
        self.created = Some(doc.skeleton.add_bone(self.bone.clone()));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.remove_bone(id);
        }
    }

    fn label(&self) -> &str {
        "Create Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        if let Some(id) = self.created {
            self.created = Some(remap.bone(id));
        }
        if let Some(parent) = self.bone.parent {
            self.bone.parent = Some(remap.bone(parent));
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete a bone, along with the slots and constraints that depended on it.
///
/// Undo restores the bone itself and re-parents its children back. Dependent
/// slots/constraints are **not** restored — capturing and rebuilding them is
/// T-108 work once serialization exists; the label says "Delete Bone" and the
/// user can see what came back.
pub struct DeleteBone {
    target: BoneId,
    /// Captured on apply so revert can rebuild.
    removed: Option<Bone>,
    /// Children that were re-parented away, and where they pointed before.
    orphans: Vec<BoneId>,
    /// The id the restored bone gets — differs from `target` after a round-trip.
    restored: Option<BoneId>,
    /// Id changes this command caused, drained by `History`.
    remap: IdRemap,
}

impl DeleteBone {
    pub fn new(target: BoneId) -> Self {
        Self {
            target,
            removed: None,
            orphans: Vec::new(),
            restored: None,
            remap: IdRemap::default(),
        }
    }
}

impl EditCommand for DeleteBone {
    fn apply(&mut self, doc: &mut Document) {
        // On redo, `target` is stale — the bone we deleted came back under a new
        // key. Use the restored id when we have one.
        let id = self.restored.take().unwrap_or(self.target);
        let Some(bone) = doc.skeleton.bones.get(id).cloned() else {
            return;
        };
        self.removed = Some(bone);
        if let Some(report) = doc.skeleton.remove_bone(id) {
            self.orphans = report.reparented_children;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(bone) = self.removed.take() else {
            return;
        };
        let id = doc.skeleton.add_bone(bone);
        // Re-adopt the children that were pushed up to the grandparent.
        for &child in &self.orphans {
            if let Some(c) = doc.skeleton.bones.get_mut(child) {
                c.parent = Some(id);
            }
        }
        doc.skeleton.rebuild_update_order();
        // The restored bone has a new key; tell the rest of the history.
        self.remap.remap_bone(self.target, id);
        self.target = id;
        self.restored = Some(id);
    }

    fn label(&self) -> &str {
        "Delete Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn take_remap(&mut self) -> IdRemap {
        std::mem::take(&mut self.remap)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        self.target = remap.bone(self.target);
        if let Some(id) = self.restored {
            self.restored = Some(remap.bone(id));
        }
        for orphan in &mut self.orphans {
            *orphan = remap.bone(*orphan);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a bone's local transform (pose it).
///
/// Merges with successive edits to the same bone so a drag is one undo step. The
/// `before` value is captured on first apply and preserved through merges, so
/// undo returns to where the drag started rather than to the previous frame.
pub struct SetBoneTransform {
    bone: BoneId,
    after: Transform,
    before: Option<Transform>,
}

impl SetBoneTransform {
    pub fn new(bone: BoneId, after: Transform) -> Self {
        Self {
            bone,
            after,
            before: None,
        }
    }

    pub fn bone(&self) -> BoneId {
        self.bone
    }
}

impl EditCommand for SetBoneTransform {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(b) = doc.skeleton.bones.get_mut(self.bone) {
            if self.before.is_none() {
                self.before = Some(b.local_transform);
            }
            b.local_transform = self.after;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(b)) = (self.before, doc.skeleton.bones.get_mut(self.bone)) {
            b.local_transform = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<SetBoneTransform>() {
            // Same bone: absorb the newer value, keep the original `before`.
            Some(other) if other.bone == self.bone => {
                self.after = other.after;
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Transform Bone"
    }

    /// A setup-pose edit. In Animate mode the same gesture is routed to key
    /// commands instead (`edit_router`), so seeing this command outside Setup
    /// means something bypassed the router.
    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        self.bone = remap.bone(self.bone);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Re-parent a bone, optionally rewriting its local transform so it does not
/// visually move (the caller computes that from the world affine — T-102's
/// `decompose`).
pub struct SetBoneParent {
    bone: BoneId,
    new_parent: Option<BoneId>,
    new_local: Option<Transform>,
    before: Option<(Option<BoneId>, Transform)>,
}

impl SetBoneParent {
    pub fn new(bone: BoneId, new_parent: Option<BoneId>, new_local: Option<Transform>) -> Self {
        Self {
            bone,
            new_parent,
            new_local,
            before: None,
        }
    }

    /// Reparent `bone` under `new_parent` (or to a root when `None`) **without
    /// moving it on screen** (T-206). The new local transform is the bone's setup
    /// world affine expressed in the new parent's space, decomposed back to a
    /// `Transform` (ADR 0002 — decompose is editor-only, never for FK).
    pub fn keeping_world(
        skeleton: &ankhimate_core::skeleton::Skeleton,
        bone: BoneId,
        new_parent: Option<BoneId>,
    ) -> Self {
        let world = skeleton.setup_world(bone);
        let parent_world = match new_parent {
            Some(p) => skeleton.setup_world(p),
            None => ankhimate_core::transforms::Affine2::IDENTITY,
        };
        // local = inv(parent_world) ∘ world. A degenerate (zero-scale) parent has
        // no inverse; fall back to identity so the reparent still succeeds rather
        // than panicking — the bone may jump, but only in a already-broken rig.
        let new_local = parent_world
            .invert()
            .unwrap_or(ankhimate_core::transforms::Affine2::IDENTITY)
            .mul(&world)
            .decompose();
        Self::new(bone, new_parent, Some(new_local))
    }
}

impl EditCommand for SetBoneParent {
    fn apply(&mut self, doc: &mut Document) {
        // Refuse cycles: re-parenting a bone under its own descendant would
        // detach that subtree from every root and hang the update order.
        if let Some(parent) = self.new_parent
            && (parent == self.bone || doc.skeleton.is_descendant(parent, self.bone))
        {
            return;
        }
        let Some(b) = doc.skeleton.bones.get_mut(self.bone) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some((b.parent, b.local_transform));
        }
        b.parent = self.new_parent;
        if let Some(local) = self.new_local {
            b.local_transform = local;
        }
        doc.skeleton.rebuild_update_order();
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some((parent, local)), Some(b)) =
            (self.before, doc.skeleton.bones.get_mut(self.bone))
        {
            b.parent = parent;
            b.local_transform = local;
            doc.skeleton.rebuild_update_order();
        }
    }

    fn label(&self) -> &str {
        "Reparent Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        self.bone = remap.bone(self.bone);
        if let Some(p) = self.new_parent {
            self.new_parent = Some(remap.bone(p));
        }
        if let Some((Some(p), local)) = self.before {
            self.before = Some((Some(remap.bone(p)), local));
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Paste a copied bone subtree under `parent` (T-209).
///
/// Names are uniquified by `Skeleton::add_bone`, so pasting next to the original
/// gives `arm_2` rather than two bones answering to `arm` — which the
/// name-keyed save format could not represent (ADR 0004).
pub struct PasteBones {
    clip: crate::clipboard::BoneClip,
    parent: Option<BoneId>,
    created_bones: Vec<BoneId>,
    created_slots: Vec<ankhimate_core::ids::SlotId>,
}

impl PasteBones {
    pub fn new(clip: crate::clipboard::BoneClip, parent: Option<BoneId>) -> Self {
        Self {
            clip,
            parent,
            created_bones: Vec::new(),
            created_slots: Vec::new(),
        }
    }

    /// The bone the subtree's root landed on, for selecting it afterwards.
    pub fn root_id(&self) -> Option<BoneId> {
        self.created_bones.first().copied()
    }
}

impl EditCommand for PasteBones {
    fn apply(&mut self, doc: &mut Document) {
        self.created_bones.clear();
        self.created_slots.clear();

        // Bones first, in clip order: a child's parent index always refers to an
        // earlier entry (the copy walked the tree top-down), so the mapping is
        // complete by the time it is needed.
        for entry in &self.clip.bones {
            let mut bone = entry.bone.clone();
            bone.parent = match entry.parent {
                Some(i) => self.created_bones.get(i).copied(),
                None => self.parent,
            };
            self.created_bones.push(doc.skeleton.add_bone(bone));
        }

        for clip_slot in &self.clip.slots {
            let Some(&bone) = self.created_bones.get(clip_slot.bone) else {
                continue;
            };
            let mut slot = clip_slot.slot.clone();
            slot.bone = bone;
            let slot_id = doc.skeleton.add_slot(slot);
            self.created_slots.push(slot_id);

            // Skins are matched by name: a subtree copied wearing a costume
            // keeps it, and a skin the target document lacks is skipped rather
            // than invented.
            for (skin_name, att_name, attachment) in &clip_slot.entries {
                let skin_id = doc
                    .skeleton
                    .skins
                    .iter()
                    .find(|(_, s)| &s.name == skin_name)
                    .map(|(id, _)| id);
                if let Some(id) = skin_id {
                    doc.skeleton.skins[id].set(slot_id, att_name.clone(), attachment.clone());
                }
            }
        }

        doc.skeleton.rebuild_update_order();
    }

    fn revert(&mut self, doc: &mut Document) {
        for slot in self.created_slots.drain(..) {
            doc.skeleton.slots.remove(slot);
            doc.skeleton.draw_order.retain(|&s| s != slot);
            for (_, skin) in doc.skeleton.skins.iter_mut() {
                skin.remove_slot(slot);
            }
        }
        // Leaves before roots, so no child is ever orphaned mid-removal.
        for bone in self.created_bones.drain(..).rev() {
            doc.skeleton.bones.remove(bone);
        }
        doc.skeleton.rebuild_update_order();
    }

    fn label(&self) -> &str {
        "Paste Bones"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        if let Some(p) = self.parent {
            self.parent = Some(remap.bone(p));
        }
        for bone in &mut self.created_bones {
            *bone = remap.bone(*bone);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Write the currently displayed pose into the setup skeleton (T-211).
///
/// The rig-fixing move: pose the character until it looks right, then make that
/// the pose everything else is measured from. Captured values are passed in
/// rather than read from a `Pose` so the command owns everything it needs to
/// revert — a `Pose` is derived state and will have moved on by then.
pub struct SetPoseAsSetup {
    /// `(bone, new local transform)`.
    targets: Vec<(BoneId, Transform)>,
    before: Vec<(BoneId, Transform)>,
}

impl SetPoseAsSetup {
    pub fn new(targets: Vec<(BoneId, Transform)>) -> Self {
        Self {
            targets,
            before: Vec::new(),
        }
    }
}

impl EditCommand for SetPoseAsSetup {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_empty() {
            self.before = self
                .targets
                .iter()
                .filter_map(|(id, _)| {
                    doc.skeleton
                        .bones
                        .get(*id)
                        .map(|b| (*id, b.local_transform))
                })
                .collect();
        }
        for (id, local) in &self.targets {
            if let Some(bone) = doc.skeleton.bones.get_mut(*id) {
                bone.local_transform = *local;
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        for (id, local) in self.before.drain(..) {
            if let Some(bone) = doc.skeleton.bones.get_mut(id) {
                bone.local_transform = local;
            }
        }
    }

    fn label(&self) -> &str {
        "Set Pose As Setup"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        for (id, _) in self.targets.iter_mut().chain(self.before.iter_mut()) {
            *id = remap.bone(*id);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Return bones to an untransformed local pose (T-211).
pub struct ResetBones {
    targets: Vec<BoneId>,
    before: Vec<(BoneId, Transform)>,
}

impl ResetBones {
    pub fn new(targets: Vec<BoneId>) -> Self {
        Self {
            targets,
            before: Vec::new(),
        }
    }
}

impl EditCommand for ResetBones {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_empty() {
            self.before = self
                .targets
                .iter()
                .filter_map(|id| {
                    doc.skeleton
                        .bones
                        .get(*id)
                        .map(|b| (*id, b.local_transform))
                })
                .collect();
        }
        for id in &self.targets {
            if let Some(bone) = doc.skeleton.bones.get_mut(*id) {
                // Position is kept: a bone's offset from its parent is where it
                // *is* in the rig, not a pose. Clearing it would collapse the
                // skeleton onto the origin, which is never what "reset" means.
                bone.local_transform = Transform {
                    position: bone.local_transform.position,
                    ..Transform::default()
                };
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        for (id, local) in self.before.drain(..) {
            if let Some(bone) = doc.skeleton.bones.get_mut(id) {
                bone.local_transform = local;
            }
        }
    }

    fn label(&self) -> &str {
        "Reset Bones"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        for id in &mut self.targets {
            *id = remap.bone(*id);
        }
        for (id, _) in &mut self.before {
            *id = remap.bone(*id);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Rename a bone.
pub struct RenameBone {
    bone: BoneId,
    new_name: String,
    before: Option<String>,
}

impl RenameBone {
    pub fn new(bone: BoneId, new_name: impl Into<String>) -> Self {
        Self {
            bone,
            new_name: new_name.into(),
            before: None,
        }
    }
}

impl EditCommand for RenameBone {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = doc.skeleton.bones.get(self.bone).map(|b| b.name.clone());
        }
        // Goes through core so the name is uniquified (ADR 0004) and the
        // name-tie-broken update order is rebuilt.
        doc.skeleton.rename_bone(self.bone, &self.new_name);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            doc.skeleton.rename_bone(self.bone, &before);
        }
    }

    fn label(&self) -> &str {
        "Rename Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        self.bone = remap.bone(self.bone);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a bone's colour (T-505/T-708).
///
/// Colour is how a rigger tells limbs apart at a glance in a 67-bone rig, and it
/// inherits: a bone with the default colour draws in its nearest coloured
/// ancestor's, so colouring a shoulder colours the whole arm.
pub struct SetBoneColor {
    bone: BoneId,
    after: [f32; 4],
    before: Option<[f32; 4]>,
}

impl SetBoneColor {
    pub fn new(bone: BoneId, after: [f32; 4]) -> Self {
        Self {
            bone,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetBoneColor {
    fn apply(&mut self, doc: &mut Document) {
        let Some(bone) = doc.skeleton.bones.get_mut(self.bone) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(bone.color);
        }
        bone.color = self.after;
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(bone)) =
            (self.before.take(), doc.skeleton.bones.get_mut(self.bone))
        {
            bone.color = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetBoneColor>() else {
            return false;
        };
        if other.bone != self.bone {
            return false;
        }
        // Dragging in the colour picker is one edit.
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Set Bone Colour"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Rename many bones at once, as a single undo step (T-901).
///
/// # Why this is not a loop over [`RenameBone`]
///
/// Names are uniquified against the rest of the rig, so renaming one at a time
/// collides with the very bones still waiting their turn. Renaming `a→b` while
/// the old `b` is still called `b` yields `b_2`, and by the end of a shifted
/// batch — `tail1→tail2`, `tail2→tail3` — every name has picked up a suffix it
/// was never meant to have.
///
/// So the whole set is applied together: each bone is first moved to a name
/// nothing can collide with, then to its final one. The intermediate pass is the
/// price of not caring what order the batch arrives in.
///
/// A rename that would land two bones on the same name is *still* resolved by
/// core's uniquifier — that is a genuine conflict the user asked for, not an
/// artefact of the batching.
pub struct RenameBones {
    /// `(bone, desired name)`.
    renames: Vec<(BoneId, String)>,
    before: Option<Vec<(BoneId, String)>>,
}

impl RenameBones {
    pub fn new(renames: Vec<(BoneId, String)>) -> Self {
        Self {
            renames,
            before: None,
        }
    }
}

impl EditCommand for RenameBones {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = Some(
                self.renames
                    .iter()
                    .filter_map(|(id, _)| {
                        doc.skeleton.bones.get(*id).map(|b| (*id, b.name.clone()))
                    })
                    .collect(),
            );
        }
        // Park every bone on a name nothing else can hold, so the second pass
        // never sees a name still occupied by a bone that is about to move off
        // it. The prefix is deliberately unusable as a real name.
        for (index, (id, _)) in self.renames.iter().enumerate() {
            if doc.skeleton.bones.contains_key(*id) {
                doc.skeleton
                    .rename_bone(*id, &format!("\u{0}pending{index}"));
            }
        }
        for (id, name) in &self.renames {
            if doc.skeleton.bones.contains_key(*id) {
                doc.skeleton.rename_bone(*id, name);
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.take() else {
            return;
        };
        // Two passes again, for the same reason: restoring `a`'s old name while
        // another bone still holds it would suffix it.
        for (index, (id, _)) in before.iter().enumerate() {
            if doc.skeleton.bones.contains_key(*id) {
                doc.skeleton
                    .rename_bone(*id, &format!("\u{0}restoring{index}"));
            }
        }
        for (id, name) in &before {
            if doc.skeleton.bones.contains_key(*id) {
                doc.skeleton.rename_bone(*id, name);
            }
        }
    }

    fn label(&self) -> &str {
        "Rename Bones"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn apply_remap(&mut self, remap: &IdRemap) {
        for (id, _) in &mut self.renames {
            *id = remap.bone(*id);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a bone's display length, optionally bringing its children with it
/// (T-907).
///
/// # Why children move
///
/// A bone's length is where its *tip* is, and a child is conventionally placed
/// at its parent's tip. Lengthening an upper arm without moving the elbow leaves
/// the elbow floating mid-bone — the rig is still valid, it just no longer means
/// what it looks like, and every child has to be dragged back by hand. Doing that
/// on a finger chain is the kind of tedium that makes people stop adjusting
/// lengths at all.
///
/// So the default carries them. Only children sitting **at the old tip** move,
/// within a small tolerance: a child deliberately placed halfway down a bone (a
/// muscle bulge, a strap) was put there on purpose and must stay, and one at the
/// bone's origin is a branch point rather than a continuation.
///
/// `carry_children: false` is the escape hatch for exactly that case, and for
/// resizing a bone purely to change how big its gizmo draws.
pub struct SetBoneLength {
    bone: BoneId,
    after: f32,
    carry_children: bool,
    /// The length before the first `apply`, plus every child moved and where it
    /// was. Captured on apply rather than in `new` so a merged drag reverts to
    /// where the drag began, not to where its last frame began.
    before: Option<(f32, Vec<(BoneId, glam::Vec2)>)>,
}

impl SetBoneLength {
    pub fn new(bone: BoneId, after: f32, carry_children: bool) -> Self {
        Self {
            bone,
            after,
            carry_children,
            before: None,
        }
    }
}

/// How close to the tip a child must sit to count as attached to it.
///
/// Proportional to the bone rather than absolute: a tenth of a unit is a lot on
/// a 3-unit finger and nothing on a 300-unit spine.
const TIP_TOLERANCE: f32 = 0.02;

impl EditCommand for SetBoneLength {
    fn apply(&mut self, doc: &mut Document) {
        let Some(bone) = doc.skeleton.bones.get(self.bone) else {
            return;
        };
        let old = bone.length;
        if self.before.is_none() {
            let mut moved = Vec::new();
            if self.carry_children {
                let tolerance = (old.abs() * TIP_TOLERANCE).max(1e-4);
                for (id, child) in doc.skeleton.bones.iter() {
                    if child.parent != Some(self.bone) {
                        continue;
                    }
                    // Children live in the parent's local space, where the tip
                    // is `(length, 0)`, so "at the tip" is a comparison against
                    // the old length on the x axis alone.
                    let position = child.local_transform.position;
                    if (position.x - old).abs() <= tolerance {
                        moved.push((id, position));
                    }
                }
            }
            self.before = Some((old, moved));
        }

        if let Some(bone) = doc.skeleton.bones.get_mut(self.bone) {
            bone.length = self.after;
        }
        // Re-read rather than reuse the list from the first apply: a merged drag
        // calls `apply` repeatedly, and each pass must place the children at the
        // *current* target length.
        if self.carry_children {
            let carried: Vec<BoneId> = self
                .before
                .as_ref()
                .map(|(_, moved)| moved.iter().map(|(id, _)| *id).collect())
                .unwrap_or_default();
            for id in carried {
                if let Some(child) = doc.skeleton.bones.get_mut(id) {
                    child.local_transform.position.x = self.after;
                }
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some((length, moved)) = self.before.take() else {
            return;
        };
        if let Some(bone) = doc.skeleton.bones.get_mut(self.bone) {
            bone.length = length;
        }
        for (id, position) in moved {
            if let Some(child) = doc.skeleton.bones.get_mut(id) {
                child.local_transform.position = position;
            }
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetBoneLength>() else {
            return false;
        };
        // Same bone and same carry decision, or the two are different edits: a
        // drag that started carrying children and ended not carrying them cannot
        // be collapsed into one undo without losing which it was.
        if other.bone != self.bone || other.carry_children != self.carry_children {
            return false;
        }
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Set Bone Length"
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

    fn bone(name: &str, parent: Option<BoneId>) -> Bone {
        Bone {
            name: name.to_string(),
            parent,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    /// Build a root → mid → leaf chain, returning the ids.
    fn chain(doc: &mut Document) -> (BoneId, BoneId, BoneId) {
        let root = doc.skeleton.add_bone(bone("a_root", None));
        let mid = doc.skeleton.add_bone(bone("b_mid", Some(root)));
        let leaf = doc.skeleton.add_bone(bone("c_leaf", Some(mid)));
        (root, mid, leaf)
    }

    /// The case a loop of single renames gets wrong (T-901).
    ///
    /// Shifting a run of names down by one means every target is a name still
    /// held by the next bone in the batch. Renamed one at a time, each collides
    /// with the bone waiting its turn and picks up a `_2` suffix.
    #[test]
    fn a_shifted_batch_rename_does_not_collide_with_itself() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("tail1", None));
        let b = doc.skeleton.add_bone(bone("tail2", Some(a)));
        let c = doc.skeleton.add_bone(bone("tail3", Some(b)));
        let mut history = History::default();

        history.push(
            Box::new(RenameBones::new(vec![
                (a, "tail2".to_string()),
                (b, "tail3".to_string()),
                (c, "tail4".to_string()),
            ])),
            &mut doc,
        );

        assert_eq!(doc.skeleton.bones[a].name, "tail2");
        assert_eq!(doc.skeleton.bones[b].name, "tail3");
        assert_eq!(doc.skeleton.bones[c].name, "tail4");

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones[a].name, "tail1", "and back again");
        assert_eq!(doc.skeleton.bones[b].name, "tail2");
        assert_eq!(doc.skeleton.bones[c].name, "tail3");
    }

    /// Two bones asked for the same name is a real conflict, not a batching
    /// artefact, so core's uniquifier still resolves it.
    #[test]
    fn a_batch_that_asks_for_one_name_twice_still_uniquifies() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a", None));
        let b = doc.skeleton.add_bone(bone("b", None));
        let mut history = History::default();

        history.push(
            Box::new(RenameBones::new(vec![
                (a, "same".to_string()),
                (b, "same".to_string()),
            ])),
            &mut doc,
        );

        let names = [
            doc.skeleton.bones[a].name.clone(),
            doc.skeleton.bones[b].name.clone(),
        ];
        assert!(names.contains(&"same".to_string()));
        assert_ne!(names[0], names[1], "the rig cannot hold two of one name");
    }

    /// A parent, a child at its tip, and a child parked halfway down it.
    fn parent_with_children(doc: &mut Document) -> (BoneId, BoneId, BoneId) {
        let parent = doc.skeleton.add_bone(bone("upper_arm", None));
        let at_tip = doc.skeleton.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("elbow", Some(parent))
        });
        let midway = doc.skeleton.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(5.0, 0.0),
                ..Transform::default()
            },
            ..bone("strap", Some(parent))
        });
        (parent, at_tip, midway)
    }

    /// The T-907 case: lengthening a bone takes the child at its tip along, and
    /// leaves one deliberately placed elsewhere alone.
    #[test]
    fn setting_a_length_carries_the_child_at_the_tip() {
        let mut doc = Document::new();
        let (parent, at_tip, midway) = parent_with_children(&mut doc);
        let mut history = History::default();

        history.push(Box::new(SetBoneLength::new(parent, 25.0, true)), &mut doc);

        assert_eq!(doc.skeleton.bones[parent].length, 25.0);
        assert_eq!(
            doc.skeleton.bones[at_tip].local_transform.position.x, 25.0,
            "the elbow follows the tip"
        );
        assert_eq!(
            doc.skeleton.bones[midway].local_transform.position.x, 5.0,
            "a bone parked mid-parent was put there on purpose"
        );

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones[parent].length, 10.0);
        assert_eq!(
            doc.skeleton.bones[at_tip].local_transform.position.x, 10.0,
            "undo restores the child too, in one step"
        );
    }

    /// Opting out leaves every child exactly where it was.
    #[test]
    fn a_length_change_can_decline_to_carry_children() {
        let mut doc = Document::new();
        let (parent, at_tip, _) = parent_with_children(&mut doc);
        let mut history = History::default();

        history.push(Box::new(SetBoneLength::new(parent, 25.0, false)), &mut doc);

        assert_eq!(doc.skeleton.bones[parent].length, 25.0);
        assert_eq!(
            doc.skeleton.bones[at_tip].local_transform.position.x, 10.0,
            "the child stayed put"
        );
    }

    /// A drag is one undo step, and it reverts to where the drag *began*.
    ///
    /// The merge is what makes that true: without it a drag across twenty frames
    /// is twenty undos, and with a naive one the child's saved position would be
    /// overwritten each frame and undo would land mid-drag.
    #[test]
    fn dragging_a_length_merges_into_one_undo() {
        let mut doc = Document::new();
        let (parent, at_tip, _) = parent_with_children(&mut doc);
        let mut history = History::default();

        for length in [12.0, 18.0, 30.0] {
            history.push(Box::new(SetBoneLength::new(parent, length, true)), &mut doc);
        }
        assert_eq!(doc.skeleton.bones[parent].length, 30.0);
        assert_eq!(doc.skeleton.bones[at_tip].local_transform.position.x, 30.0);

        history.undo(&mut doc);
        assert_eq!(
            doc.skeleton.bones[parent].length, 10.0,
            "one undo returns to before the drag"
        );
        assert_eq!(
            doc.skeleton.bones[at_tip].local_transform.position.x, 10.0,
            "and the child with it"
        );
    }

    #[test]
    fn delete_bone_undo_restores_it_and_readopts_children() {
        let mut doc = Document::new();
        let (root, mid, leaf) = chain(&mut doc);
        let mut history = History::default();

        history.push(Box::new(DeleteBone::new(mid)), &mut doc);
        assert!(doc.skeleton.bones.get(mid).is_none());
        // `leaf` was pushed up to `root`.
        assert_eq!(doc.skeleton.bones[leaf].parent, Some(root));

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 3, "bone came back");
        // `leaf` is a child of the restored bone again, not of `root`.
        let leaf_parent = doc.skeleton.bones[leaf].parent.unwrap();
        assert_ne!(leaf_parent, root, "child must be re-adopted");
        assert_eq!(doc.skeleton.bones[leaf_parent].name, "b_mid");
    }

    #[test]
    fn delete_bone_redo_after_undo_works() {
        let mut doc = Document::new();
        let (_root, mid, _leaf) = chain(&mut doc);
        let mut history = History::default();

        history.push(Box::new(DeleteBone::new(mid)), &mut doc);
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 3);

        // Redo must find the *restored* bone, not the stale original id.
        history.redo(&mut doc);
        assert_eq!(doc.skeleton.bones.len(), 2, "redo deleted it again");
    }

    #[test]
    fn reparent_refuses_to_create_a_cycle() {
        let mut doc = Document::new();
        let (root, mid, _leaf) = chain(&mut doc);
        let mut history = History::default();

        // Parenting `root` under its own descendant would orphan the whole tree.
        history.push(
            Box::new(SetBoneParent::new(root, Some(mid), None)),
            &mut doc,
        );
        assert_eq!(doc.skeleton.bones[root].parent, None, "cycle refused");
        // Every bone is still reachable from a root.
        assert_eq!(doc.skeleton.update_order.len(), 3);
    }

    #[test]
    fn reparent_to_self_is_refused() {
        let mut doc = Document::new();
        let (root, _mid, _leaf) = chain(&mut doc);
        let mut history = History::default();
        history.push(
            Box::new(SetBoneParent::new(root, Some(root), None)),
            &mut doc,
        );
        assert_eq!(doc.skeleton.bones[root].parent, None);
    }

    #[test]
    fn reparent_undo_restores_parent_and_local() {
        let mut doc = Document::new();
        let (root, mid, leaf) = chain(&mut doc);
        let mut history = History::default();

        let moved_local = Transform {
            position: glam::vec2(99.0, 99.0),
            ..Transform::default()
        };
        history.push(
            Box::new(SetBoneParent::new(leaf, Some(root), Some(moved_local))),
            &mut doc,
        );
        assert_eq!(doc.skeleton.bones[leaf].parent, Some(root));

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones[leaf].parent, Some(mid));
        assert_eq!(
            doc.skeleton.bones[leaf].local_transform.position,
            glam::Vec2::ZERO,
            "local transform restored too"
        );
    }

    #[test]
    fn rename_roundtrips_and_keeps_order_topological() {
        let mut doc = Document::new();
        let (root, mid, leaf) = chain(&mut doc);
        let mut history = History::default();

        // Rename so the name-based tie-break would reorder siblings.
        history.push(Box::new(RenameBone::new(mid, "z_last")), &mut doc);
        assert_eq!(doc.skeleton.bones[mid].name, "z_last");

        let order = &doc.skeleton.update_order;
        let pos = |id: BoneId| order.iter().position(|&x| x == id).unwrap();
        assert!(
            pos(root) < pos(mid) && pos(mid) < pos(leaf),
            "still topological"
        );

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones[mid].name, "b_mid");
    }

    #[test]
    fn create_bone_exposes_its_new_id() {
        let mut doc = Document::new();
        let mut cmd = CreateBone::new(bone("a", None));
        assert!(cmd.created_id().is_none());
        cmd.apply(&mut doc);
        let id = cmd.created_id().expect("id after apply");
        assert_eq!(doc.skeleton.bones[id].name, "a");
    }

    #[test]
    fn set_transform_undo_returns_to_pre_drag_value() {
        let mut doc = Document::new();
        let (root, _mid, _leaf) = chain(&mut doc);
        let mut history = History::default();

        let start = doc.skeleton.bones[root].local_transform;
        let end = Transform {
            position: glam::vec2(42.0, 0.0),
            ..start
        };
        history.push(Box::new(SetBoneTransform::new(root, end)), &mut doc);
        assert_eq!(doc.skeleton.bones[root].local_transform.position.x, 42.0);

        history.undo(&mut doc);
        assert_eq!(
            doc.skeleton.bones[root].local_transform.position,
            start.position
        );
    }
}
