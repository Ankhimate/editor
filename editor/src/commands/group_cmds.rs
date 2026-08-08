//! Hierarchy folders as undoable commands.
//!
//! A group is organisation, not rig structure — it has no transform and nothing
//! inherits from it — but it is *document* state, because a rigger files a
//! sixty-bone rig once and everyone who opens it afterwards should find it
//! filed. So grouping is an edit, and edits go on the undo stack.
//!
//! Setup-only, like everything else that changes how the rig is put together.
//!
//! Each command snapshots the folder table rather than inverting its edit. The
//! table is a handful of names and id lists, and an index-based inverse would be
//! pointing at the wrong folder the moment one is removed above it.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::ids::GroupId;
use ankhimate_core::skeleton::{Group, GroupMember};

/// Everything the folder table holds, for snapshot/restore.
type Snapshot = (
    ankhimate_core::slotmap::SlotMap<GroupId, Group>,
    Vec<GroupId>,
);

fn snapshot(doc: &Document) -> Snapshot {
    (
        doc.skeleton.groups.clone(),
        doc.skeleton.group_order.clone(),
    )
}

fn restore(doc: &mut Document, snap: Snapshot) {
    doc.skeleton.groups = snap.0;
    doc.skeleton.group_order = snap.1;
}

/// Create a folder holding the given members.
pub struct CreateGroup {
    name: String,
    members: Vec<GroupMember>,
    before: Option<Snapshot>,
}

impl CreateGroup {
    pub fn new(name: impl Into<String>, members: Vec<GroupMember>) -> Self {
        Self {
            name: name.into(),
            members,
            before: None,
        }
    }
}

impl EditCommand for CreateGroup {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = Some(snapshot(doc));
        }
        let id = doc.skeleton.add_group(Group::new(self.name.clone()));
        // Through `assign_to_group` rather than by pushing directly, so a member
        // already filed elsewhere is moved rather than duplicated — membership is
        // exclusive and that is enforced in one place.
        for member in &self.members {
            doc.skeleton.assign_to_group(*member, Some(id));
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            restore(doc, before);
        }
    }

    fn label(&self) -> &str {
        "Create Group"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What [`EditGroup`] does to the folder it names.
pub enum GroupEdit {
    Rename(String),
    /// Dissolve the folder, leaving its contents where they are.
    Ungroup,
    /// File more members into it, taking them out of whatever held them.
    Add(Vec<GroupMember>),
    SetColor([f32; 4]),
}

pub struct EditGroup {
    group: GroupId,
    edit: GroupEdit,
    before: Option<Snapshot>,
}

impl EditGroup {
    pub fn new(group: GroupId, edit: GroupEdit) -> Self {
        Self {
            group,
            edit,
            before: None,
        }
    }
}

impl EditCommand for EditGroup {
    fn apply(&mut self, doc: &mut Document) {
        if !doc.skeleton.groups.contains_key(self.group) {
            return;
        }
        if self.before.is_none() {
            self.before = Some(snapshot(doc));
        }
        match &self.edit {
            GroupEdit::Rename(name) => {
                if let Some(group) = doc.skeleton.groups.get_mut(self.group) {
                    group.name = name.clone();
                }
            }
            GroupEdit::Ungroup => {
                doc.skeleton.remove_group(self.group);
            }
            GroupEdit::Add(members) => {
                for member in members {
                    doc.skeleton.assign_to_group(*member, Some(self.group));
                }
            }
            GroupEdit::SetColor(color) => {
                if let Some(group) = doc.skeleton.groups.get_mut(self.group) {
                    group.color = *color;
                }
            }
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            restore(doc, before);
        }
    }

    fn label(&self) -> &str {
        match self.edit {
            GroupEdit::Rename(_) => "Rename Group",
            GroupEdit::Ungroup => "Ungroup",
            GroupEdit::Add(_) => "Add To Group",
            GroupEdit::SetColor(_) => "Recolour Group",
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

    /// Ungrouping dissolves the folder and keeps everything that was in it.
    ///
    /// The distinction the whole feature rests on: a folder is not a thing that
    /// owns its contents, so removing it must not remove them.
    #[test]
    fn ungrouping_keeps_the_bones() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let b = doc.skeleton.add_bone(bone("b"));
        let mut history = History::default();

        history.push(
            Box::new(CreateGroup::new(
                "arm",
                vec![GroupMember::Bone(a), GroupMember::Bone(b)],
            )),
            &mut doc,
        );
        let group = doc.skeleton.groups.keys().next().unwrap();
        assert_eq!(doc.skeleton.groups[group].members.len(), 2);

        history.push(
            Box::new(EditGroup::new(group, GroupEdit::Ungroup)),
            &mut doc,
        );
        assert!(doc.skeleton.groups.is_empty(), "the folder went");
        assert!(doc.skeleton.bones.contains_key(a), "the bones did not");
        assert!(doc.skeleton.bones.contains_key(b));

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.groups.len(), 1, "and it comes back");
        let group = doc.skeleton.groups.keys().next().unwrap();
        assert_eq!(doc.skeleton.groups[group].members.len(), 2);
    }

    /// Filing a bone into a second folder takes it out of the first.
    #[test]
    fn adding_to_a_group_moves_rather_than_copies() {
        let mut doc = Document::new();
        let a = doc.skeleton.add_bone(bone("a"));
        let mut history = History::default();

        history.push(
            Box::new(CreateGroup::new("first", vec![GroupMember::Bone(a)])),
            &mut doc,
        );
        history.push(Box::new(CreateGroup::new("second", vec![])), &mut doc);
        let second = doc
            .skeleton
            .groups
            .iter()
            .find(|(_, g)| g.name == "second")
            .map(|(id, _)| id)
            .unwrap();

        history.push(
            Box::new(EditGroup::new(
                second,
                GroupEdit::Add(vec![GroupMember::Bone(a)]),
            )),
            &mut doc,
        );

        assert_eq!(doc.skeleton.group_of(GroupMember::Bone(a)), Some(second));
        let first = doc
            .skeleton
            .groups
            .iter()
            .find(|(_, g)| g.name == "first")
            .map(|(id, _)| id)
            .unwrap();
        assert!(
            doc.skeleton.groups[first].members.is_empty(),
            "the bone is in one folder, not two"
        );

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.group_of(GroupMember::Bone(a)), Some(first));
    }
}

/// Move, rotate, scale or shear every top-level member of a folder at once.
///
/// **Top-level only**, via `group_transform_targets`: a folder holding a
/// shoulder *and* the elbow under it moves the limb once. The elbow already
/// follows the shoulder through ordinary parenting, so writing to both would
/// displace it twice. Attachments are untouched for the same reason — they ride
/// their bone.
///
/// Each member's own local transform is what changes. The group gains no pivot
/// and no transform of its own, so it stays organisation rather than becoming a
/// second parenting system: rotating a group turns each limb about its own
/// origin, which is what "apply this to everything in here" means.
pub struct TransformGroup {
    group: GroupId,
    delta: GroupDelta,
    /// Captured on the first apply so a merged drag reverts to where it began.
    before: Option<Vec<(ankhimate_core::ids::BoneId, ankhimate_core::math::Transform)>>,
}

/// What [`TransformGroup`] adds to each member.
#[derive(Clone, Copy, PartialEq)]
pub struct GroupDelta {
    pub translate: glam::Vec2,
    /// Radians.
    pub rotate: f32,
    /// Multiplied into the member's scale; `1.0` leaves it alone.
    pub scale: glam::Vec2,
    /// Radians, added to shear.
    pub shear: glam::Vec2,
}

impl Default for GroupDelta {
    fn default() -> Self {
        Self {
            translate: glam::Vec2::ZERO,
            rotate: 0.0,
            scale: glam::Vec2::ONE,
            shear: glam::Vec2::ZERO,
        }
    }
}

impl TransformGroup {
    pub fn new(group: GroupId, delta: GroupDelta) -> Self {
        Self {
            group,
            delta,
            before: None,
        }
    }
}

impl EditCommand for TransformGroup {
    fn apply(&mut self, doc: &mut Document) {
        let targets = doc.skeleton.group_transform_targets(self.group);
        if targets.is_empty() {
            return;
        }
        if self.before.is_none() {
            self.before = Some(
                targets
                    .iter()
                    .filter_map(|b| doc.skeleton.bones.get(*b).map(|x| (*b, x.local_transform)))
                    .collect(),
            );
        }
        // Re-applied from the snapshot rather than compounding, so a merged drag
        // means "this much from where it started", not "this much again".
        let Some(before) = &self.before else { return };
        for (bone, original) in before {
            let Some(b) = doc.skeleton.bones.get_mut(*bone) else {
                continue;
            };
            b.local_transform = *original;
            b.local_transform.position += self.delta.translate;
            b.local_transform.rotation += self.delta.rotate;
            b.local_transform.scale *= self.delta.scale;
            b.local_transform.shear += self.delta.shear;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.take() else {
            return;
        };
        for (bone, transform) in before {
            if let Some(b) = doc.skeleton.bones.get_mut(bone) {
                b.local_transform = transform;
            }
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<TransformGroup>() else {
            return false;
        };
        if other.group != self.group {
            return false;
        }
        // A drag, or a held arrow key, is one edit.
        self.delta = other.delta;
        true
    }

    fn label(&self) -> &str {
        "Transform Group"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
