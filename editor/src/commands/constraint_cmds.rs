//! Constraint authoring as undoable commands (T-501).
//!
//! Constraints are rig structure — they decide how bones relate, not where they
//! are at one instant — so everything here is Setup-only (T-207). The animated
//! part is the mix, which is a timeline (`TransformConstraintMix`) and goes
//! through the key commands like any other animated value.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::constraints::{
    Constraint, IkConstraint, PhysicsConstraint, TransformConstraint,
};
use ankhimate_core::ids::{BoneId, ConstraintId};

/// Add a transform constraint driving `bones` from `target`.
pub struct AddTransformConstraint {
    name: String,
    target: BoneId,
    bones: Vec<BoneId>,
    /// Set on apply so revert can remove exactly what was added.
    created: Option<ConstraintId>,
}

impl AddTransformConstraint {
    pub fn new(name: impl Into<String>, target: BoneId, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            target,
            bones,
            created: None,
        }
    }
}

impl EditCommand for AddTransformConstraint {
    fn apply(&mut self, doc: &mut Document) {
        if doc.skeleton.bones.get(self.target).is_none() || self.bones.is_empty() {
            return;
        }
        // A bone driven by itself would read its own output; drop those rather
        // than create a constraint that silently does nothing.
        let bones: Vec<BoneId> = self
            .bones
            .iter()
            .copied()
            .filter(|b| *b != self.target && doc.skeleton.bones.contains_key(*b))
            .collect();
        if bones.is_empty() {
            return;
        }
        let id =
            doc.skeleton
                .add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
                    self.name.clone(),
                    self.target,
                    bones,
                )));
        self.created = Some(id);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.remove_constraint(id);
        }
    }

    fn label(&self) -> &str {
        "Add Transform Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete a constraint, remembering enough to put it back.
/// Rearrange the order constraints solve in.
///
/// Not cosmetic. Spineboy's leg IK runs before its foot IK; swap them and each
/// foot is aimed against a leg that has not moved yet, which reads as boots at
/// the wrong angle with no visible cause. The order is rig data, so it goes
/// through a command like everything else.
pub struct SetConstraintOrder {
    order: Vec<ConstraintId>,
    before: Option<Vec<ConstraintId>>,
}

impl SetConstraintOrder {
    pub fn new(order: Vec<ConstraintId>) -> Self {
        Self {
            order,
            before: None,
        }
    }
}

impl EditCommand for SetConstraintOrder {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = Some(doc.skeleton.constraint_order.clone());
        }
        // Only ids that still exist, and every one exactly once: a stale id from
        // an undone delete would otherwise resurrect a constraint that is gone.
        let mut next = Vec::with_capacity(self.order.len());
        for id in &self.order {
            if doc.skeleton.constraints.contains_key(*id) && !next.contains(id) {
                next.push(*id);
            }
        }
        // Anything the caller did not mention keeps its place at the end, so a
        // constraint added while this command was on the stack is not dropped.
        for (id, _) in doc.skeleton.constraints.iter() {
            if !next.contains(&id) {
                next.push(id);
            }
        }
        doc.skeleton.constraint_order = next;
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            doc.skeleton.constraint_order = before;
        }
    }

    fn label(&self) -> &str {
        "Reorder Constraints"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub struct RemoveConstraint {
    id: ConstraintId,
    removed: Option<Constraint>,
    /// Where it sat in `constraint_order`; order changes the result (each
    /// constraint sees the previous ones' output), so restoring it at the end
    /// would not be an undo.
    order_index: Option<usize>,
}

impl RemoveConstraint {
    pub fn new(id: ConstraintId) -> Self {
        Self {
            id,
            removed: None,
            order_index: None,
        }
    }
}

impl EditCommand for RemoveConstraint {
    fn apply(&mut self, doc: &mut Document) {
        self.order_index = doc
            .skeleton
            .constraint_order
            .iter()
            .position(|c| *c == self.id);
        self.removed = doc.skeleton.constraints.get(self.id).cloned();
        doc.skeleton.remove_constraint(self.id);
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(constraint) = self.removed.take() else {
            return;
        };
        // A fresh id: the slotmap key is gone. Timelines keyed to the old id
        // cannot be revived, which is why removing a constrained-and-keyed
        // constraint is a real edit rather than a toggle.
        let id = doc.skeleton.constraints.insert(constraint);
        match self.order_index {
            Some(at) if at <= doc.skeleton.constraint_order.len() => {
                doc.skeleton.constraint_order.insert(at, id)
            }
            _ => doc.skeleton.constraint_order.push(id),
        }
        self.id = id;
    }

    fn label(&self) -> &str {
        "Remove Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Everything editable about a transform constraint, as one value.
///
/// One command for the whole struct rather than one per field: the fields are
/// edited together in one inspector section, and a drag across four mix sliders
/// should not leave four entries in the undo stack.
#[derive(Clone, PartialEq)]
pub struct TransformProps {
    pub target: BoneId,
    pub bones: Vec<BoneId>,
    pub offsets: ankhimate_core::math::Transform,
    pub mix_rotate: f32,
    pub mix_translate: f32,
    pub mix_scale: f32,
    pub mix_shear: f32,
    pub local: bool,
    pub relative: bool,
}

impl TransformProps {
    pub fn from_constraint(tc: &TransformConstraint) -> Self {
        Self {
            target: tc.target,
            bones: tc.bones.clone(),
            offsets: tc.offsets,
            mix_rotate: tc.mix_rotate,
            mix_translate: tc.mix_translate,
            mix_scale: tc.mix_scale,
            mix_shear: tc.mix_shear,
            local: tc.local,
            relative: tc.relative,
        }
    }

    fn write_to(&self, tc: &mut TransformConstraint) {
        tc.target = self.target;
        tc.bones = self.bones.clone();
        tc.offsets = self.offsets;
        tc.mix_rotate = self.mix_rotate;
        tc.mix_translate = self.mix_translate;
        tc.mix_scale = self.mix_scale;
        tc.mix_shear = self.mix_shear;
        tc.local = self.local;
        tc.relative = self.relative;
    }
}

pub struct SetTransformProps {
    id: ConstraintId,
    after: TransformProps,
    before: Option<TransformProps>,
}

impl SetTransformProps {
    pub fn new(id: ConstraintId, after: TransformProps) -> Self {
        Self {
            id,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetTransformProps {
    fn apply(&mut self, doc: &mut Document) {
        let Some(Constraint::Transform(tc)) = doc.skeleton.constraints.get_mut(self.id) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(TransformProps::from_constraint(tc));
        }
        self.after.write_to(tc);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(Constraint::Transform(tc))) = (
            self.before.take(),
            doc.skeleton.constraints.get_mut(self.id),
        ) {
            before.write_to(tc);
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetTransformProps>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after.clone();
        true
    }

    fn label(&self) -> &str {
        "Edit Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Create an IK target bone and a constraint that reaches for it (T-504).
///
/// One command, not three, because the three are useless apart: a target bone
/// with no constraint is a stray bone, and a constraint with no target does not
/// evaluate. Undo has to remove both.
///
/// The target is created **unparented**, at the chain's tip. Parenting it to the
/// chain would make the constraint chase its own output, and putting it anywhere
/// else means the rig jumps the moment the constraint switches on.
pub struct CreateIkTarget {
    /// Chain root first, tip last.
    chain: Vec<BoneId>,
    name: String,
    /// Where the target bone goes — the chain tip's world position, resolved by
    /// the caller since the command has no pose.
    position: glam::Vec2,
    created: Option<(BoneId, ConstraintId)>,
}

impl CreateIkTarget {
    pub fn new(chain: Vec<BoneId>, name: impl Into<String>, position: glam::Vec2) -> Self {
        Self {
            chain,
            name: name.into(),
            position,
            created: None,
        }
    }
}

impl EditCommand for CreateIkTarget {
    fn apply(&mut self, doc: &mut Document) {
        if self.chain.is_empty()
            || self
                .chain
                .iter()
                .any(|b| !doc.skeleton.bones.contains_key(*b))
        {
            return;
        }
        let target = doc.skeleton.add_bone(ankhimate_core::skeleton::Bone {
            name: format!("{}_target", self.name),
            parent: None,
            length: 0.0,
            local_transform: ankhimate_core::math::Transform {
                position: self.position,
                ..Default::default()
            },
            inherit: Default::default(),
            color: ankhimate_core::skeleton::Bone::default_color(),
        });
        let constraint = doc.skeleton.add_constraint(Constraint::Ik(IkConstraint {
            bones: self.chain.clone(),
            ..IkConstraint::aim(self.name.clone(), target, self.chain[0])
        }));
        self.created = Some((target, constraint));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some((target, constraint)) = self.created.take() {
            doc.skeleton.remove_constraint(constraint);
            // Removing the bone would also drop the constraint via the
            // dependency sweep; doing the constraint first keeps the order
            // explicit rather than relying on that.
            doc.skeleton.remove_bone(target);
        }
    }

    fn label(&self) -> &str {
        "Create IK Target"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Everything editable about an IK constraint.
#[derive(Clone, PartialEq)]
pub struct IkProps {
    pub target: BoneId,
    pub bones: Vec<BoneId>,
    pub bend_direction: f32,
    pub mix: f32,
    pub softness: f32,
    pub stretch: bool,
    pub stretch_limit: f32,
}

impl IkProps {
    pub fn from_constraint(ik: &IkConstraint) -> Self {
        Self {
            target: ik.target,
            bones: ik.bones.clone(),
            bend_direction: ik.bend_direction,
            mix: ik.mix,
            softness: ik.softness,
            stretch: ik.stretch,
            stretch_limit: ik.stretch_limit,
        }
    }

    fn write_to(&self, ik: &mut IkConstraint) {
        ik.target = self.target;
        ik.bones = self.bones.clone();
        ik.bend_direction = self.bend_direction;
        ik.mix = self.mix;
        ik.softness = self.softness;
        ik.stretch = self.stretch;
        ik.stretch_limit = self.stretch_limit;
    }
}

pub struct SetIkProps {
    id: ConstraintId,
    after: IkProps,
    before: Option<IkProps>,
}

impl SetIkProps {
    pub fn new(id: ConstraintId, after: IkProps) -> Self {
        Self {
            id,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetIkProps {
    fn apply(&mut self, doc: &mut Document) {
        let Some(Constraint::Ik(ik)) = doc.skeleton.constraints.get_mut(self.id) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(IkProps::from_constraint(ik));
        }
        self.after.write_to(ik);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(Constraint::Ik(ik))) = (
            self.before.take(),
            doc.skeleton.constraints.get_mut(self.id),
        ) {
            before.write_to(ik);
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetIkProps>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after.clone();
        true
    }

    fn label(&self) -> &str {
        "Edit IK Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Add a physics constraint to a bone (T-503).
pub struct AddPhysics {
    bone: BoneId,
    name: String,
    created: Option<ConstraintId>,
}

impl AddPhysics {
    pub fn new(bone: BoneId, name: impl Into<String>) -> Self {
        Self {
            bone,
            name: name.into(),
            created: None,
        }
    }
}

impl EditCommand for AddPhysics {
    fn apply(&mut self, doc: &mut Document) {
        if !doc.skeleton.bones.contains_key(self.bone) {
            return;
        }
        self.created = Some(doc.skeleton.add_constraint(Constraint::Physics(
            PhysicsConstraint::sway(self.name.clone(), self.bone),
        )));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.remove_constraint(id);
        }
    }

    fn label(&self) -> &str {
        "Add Physics"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Everything editable about a physics constraint.
#[derive(Clone, Copy, PartialEq)]
pub struct PhysicsProps {
    pub inertia: f32,
    pub strength: f32,
    pub damping: f32,
    pub mass: f32,
    pub wind: glam::Vec2,
    pub gravity: glam::Vec2,
    pub mix: f32,
    pub rotate: bool,
    pub translate: bool,
}

impl PhysicsProps {
    pub fn from_constraint(p: &PhysicsConstraint) -> Self {
        Self {
            inertia: p.inertia,
            strength: p.strength,
            damping: p.damping,
            mass: p.mass,
            wind: p.wind,
            gravity: p.gravity,
            mix: p.mix,
            rotate: p.rotate,
            translate: p.translate,
        }
    }

    fn write_to(&self, p: &mut PhysicsConstraint) {
        p.inertia = self.inertia;
        p.strength = self.strength;
        p.damping = self.damping;
        p.mass = self.mass;
        p.wind = self.wind;
        p.gravity = self.gravity;
        p.mix = self.mix;
        p.rotate = self.rotate;
        p.translate = self.translate;
    }
}

pub struct SetPhysicsProps {
    id: ConstraintId,
    after: PhysicsProps,
    before: Option<PhysicsProps>,
}

impl SetPhysicsProps {
    pub fn new(id: ConstraintId, after: PhysicsProps) -> Self {
        Self {
            id,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetPhysicsProps {
    fn apply(&mut self, doc: &mut Document) {
        let Some(Constraint::Physics(p)) = doc.skeleton.constraints.get_mut(self.id) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(PhysicsProps::from_constraint(p));
        }
        self.after.write_to(p);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(Constraint::Physics(p))) = (
            self.before.take(),
            doc.skeleton.constraints.get_mut(self.id),
        ) {
            before.write_to(p);
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetPhysicsProps>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Edit Physics"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Add a path constraint driving `bones` along the path on `slot` (T-502).
pub struct AddPathConstraint {
    name: String,
    slot: ankhimate_core::ids::SlotId,
    bones: Vec<BoneId>,
    created: Option<ConstraintId>,
}

impl AddPathConstraint {
    pub fn new(
        name: impl Into<String>,
        slot: ankhimate_core::ids::SlotId,
        bones: Vec<BoneId>,
    ) -> Self {
        Self {
            name: name.into(),
            slot,
            bones,
            created: None,
        }
    }
}

impl EditCommand for AddPathConstraint {
    fn apply(&mut self, doc: &mut Document) {
        if self.bones.is_empty() || !doc.skeleton.slots.contains_key(self.slot) {
            return;
        }
        self.created = Some(doc.skeleton.add_constraint(Constraint::Path(
            ankhimate_core::constraints::PathConstraint::new(
                self.name.clone(),
                self.slot,
                self.bones.clone(),
            ),
        )));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.remove_constraint(id);
        }
    }

    fn label(&self) -> &str {
        "Add Path Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Everything editable about a path constraint.
#[derive(Clone, Copy, PartialEq)]
pub struct PathProps {
    pub position: f32,
    pub spacing: f32,
    pub mix_rotate: f32,
    pub mix_translate: f32,
}

pub struct SetPathProps {
    id: ConstraintId,
    after: PathProps,
    before: Option<PathProps>,
}

impl SetPathProps {
    pub fn new(id: ConstraintId, after: PathProps) -> Self {
        Self {
            id,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetPathProps {
    fn apply(&mut self, doc: &mut Document) {
        let Some(Constraint::Path(p)) = doc.skeleton.constraints.get_mut(self.id) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(PathProps {
                position: p.position,
                spacing: p.spacing,
                mix_rotate: p.mix_rotate,
                mix_translate: p.mix_translate,
            });
        }
        p.position = self.after.position;
        p.spacing = self.after.spacing;
        p.mix_rotate = self.after.mix_rotate;
        p.mix_translate = self.after.mix_translate;
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(Constraint::Path(p))) = (
            self.before.take(),
            doc.skeleton.constraints.get_mut(self.id),
        ) {
            p.position = before.position;
            p.spacing = before.spacing;
            p.mix_rotate = before.mix_rotate;
            p.mix_translate = before.mix_translate;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetPathProps>() else {
            return false;
        };
        if other.id != self.id {
            return false;
        }
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Edit Path Constraint"
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

    fn doc_with_two_bones() -> (Document, BoneId, BoneId) {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(Bone {
            name: "a".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let b = doc.skeleton.add_bone(Bone {
            name: "b".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        (doc, a, b)
    }

    #[test]
    fn adding_and_undoing_a_constraint_leaves_no_trace() {
        let (mut doc, a, b) = doc_with_two_bones();
        let mut history = History::default();
        history.push(
            Box::new(AddTransformConstraint::new("look", a, vec![b])),
            &mut doc,
        );
        assert_eq!(doc.skeleton.constraints.len(), 1);
        assert_eq!(doc.skeleton.constraint_order.len(), 1);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.constraints.len(), 0);
        assert!(doc.skeleton.constraint_order.is_empty());
    }

    #[test]
    fn a_bone_cannot_be_constrained_to_itself() {
        let (mut doc, a, _) = doc_with_two_bones();
        let mut history = History::default();
        history.push(
            Box::new(AddTransformConstraint::new("self", a, vec![a])),
            &mut doc,
        );
        assert_eq!(
            doc.skeleton.constraints.len(),
            0,
            "a self-driving constraint is refused, not created inert"
        );
    }

    #[test]
    fn removing_a_constraint_restores_its_place_in_the_order() {
        let (mut doc, a, b) = doc_with_two_bones();
        let mut history = History::default();
        for name in ["first", "second", "third"] {
            history.push(
                Box::new(AddTransformConstraint::new(name, a, vec![b])),
                &mut doc,
            );
        }
        let middle = doc.skeleton.constraint_order[1];
        history.push(Box::new(RemoveConstraint::new(middle)), &mut doc);
        assert_eq!(doc.skeleton.constraint_order.len(), 2);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.constraint_order.len(), 3);
        let names: Vec<&str> = doc
            .skeleton
            .constraint_order
            .iter()
            .filter_map(|id| doc.skeleton.constraints.get(*id))
            .map(|c| c.name())
            .collect();
        assert_eq!(
            names,
            vec!["first", "second", "third"],
            "order matters — it decides which constraint wins"
        );
    }

    #[test]
    fn editing_mixes_merges_into_one_undo_step() {
        let (mut doc, a, b) = doc_with_two_bones();
        let mut history = History::default();
        history.push(
            Box::new(AddTransformConstraint::new("look", a, vec![b])),
            &mut doc,
        );
        let id = doc.skeleton.constraint_order[0];
        let Some(Constraint::Transform(tc)) = doc.skeleton.constraints.get(id) else {
            panic!("it is a transform constraint");
        };
        let base = TransformProps::from_constraint(tc);

        // Two frames of one slider drag.
        for mix in [0.5, 0.25] {
            history.push(
                Box::new(SetTransformProps::new(
                    id,
                    TransformProps {
                        mix_rotate: mix,
                        ..base.clone()
                    },
                )),
                &mut doc,
            );
        }
        history.undo(&mut doc);

        let Some(Constraint::Transform(tc)) = doc.skeleton.constraints.get(id) else {
            panic!("still there");
        };
        assert_eq!(tc.mix_rotate, 1.0, "one undo returns to before the drag");
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::constraints::IkConstraint;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

    fn doc_with_constraints() -> (Document, Vec<ConstraintId>) {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let ids = ["a", "b", "c"]
            .into_iter()
            .map(|name| {
                doc.skeleton.add_constraint(Constraint::Ik(IkConstraint {
                    name: name.into(),
                    target: bone,
                    bones: vec![bone],
                    bend_direction: 1.0,
                    mix: 1.0,
                    softness: 0.0,
                    stretch: false,
                    stretch_limit: 1.1,
                }))
            })
            .collect();
        (doc, ids)
    }

    #[test]
    fn reordering_round_trips_through_undo() {
        let (mut doc, ids) = doc_with_constraints();
        let mut history = History::default();
        let swapped = vec![ids[2], ids[0], ids[1]];
        history.push(Box::new(SetConstraintOrder::new(swapped.clone())), &mut doc);
        assert_eq!(doc.skeleton.constraint_order, swapped);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.constraint_order, ids);
    }

    /// A stale id — from an undone delete, say — must not resurrect a constraint
    /// that is gone, and a constraint the caller forgot must not vanish.
    #[test]
    fn unknown_ids_are_dropped_and_missing_ones_kept() {
        let (mut doc, ids) = doc_with_constraints();
        let ghost = doc
            .skeleton
            .constraints
            .insert(Constraint::Ik(IkConstraint {
                name: "ghost".into(),
                target: ids
                    .first()
                    .map(|_| doc.skeleton.bones.keys().next().unwrap())
                    .unwrap(),
                bones: vec![],
                bend_direction: 1.0,
                mix: 1.0,
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
            }));
        doc.skeleton.constraints.remove(ghost);

        let mut history = History::default();
        // Mentions a dead id and omits a live one.
        history.push(
            Box::new(SetConstraintOrder::new(vec![ghost, ids[1]])),
            &mut doc,
        );
        let order = &doc.skeleton.constraint_order;
        assert!(!order.contains(&ghost), "a dead constraint came back");
        assert_eq!(order.len(), 3, "every live constraint is still ordered");
        assert_eq!(order[0], ids[1], "the requested order is honoured first");
    }
}
