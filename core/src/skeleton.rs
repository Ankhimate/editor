use crate::attachment::Attachment;
use crate::constraints::Constraint;
use crate::ids::{BoneId, ConstraintId, SkinId, SlotId};
use crate::math::Transform;
use crate::skin::Skin;
use crate::slot::Slot;
use crate::transforms::Inherit;
use serde::{Deserialize, Serialize};
use slotmap::SlotMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<BoneId>,
    pub length: f32,
    pub local_transform: Transform,
    /// Rotation/scale inheritance flags (PLAN §2.2, ADR 0002).
    #[serde(default)]
    pub inherit: Inherit,
    /// RGBA color for rendering. Defaults to Spine-like teal.
    #[serde(default = "Bone::default_color")]
    pub color: [f32; 4],
}

impl Bone {
    pub fn default_color() -> [f32; 4] {
        [0.0, 0.80, 0.80, 0.85] // Teal with slight transparency
    }

    // World-space queries live on `Pose` (T-103): a `Bone` holds document data
    // only, so it cannot know where it ended up. See `Pose::world_tip`.
}

/// Make `name` unique against `taken` by appending `_2`, `_3`, … (ADR 0004).
///
/// Name-keyed serialization means duplicates silently lose data on save, so
/// uniqueness is enforced at insert time rather than validated later.
pub fn unique_name<'a>(name: &str, taken: impl Iterator<Item = &'a str>) -> String {
    let existing: std::collections::HashSet<&str> = taken.collect();
    if !existing.contains(name) {
        return name.to_string();
    }
    // Start at _2: the unsuffixed name is conceptually the first.
    (2..)
        .map(|n| format!("{name}_{n}"))
        .find(|candidate| !existing.contains(candidate.as_str()))
        .expect("an unused suffix always exists")
}

/// Report of what [`Skeleton::remove_bone`] tore down alongside the bone.
#[derive(Debug, Clone, Default)]
pub struct RemoveBoneReport {
    pub removed_bone_name: String,
    /// Children that were re-parented to the removed bone's parent.
    pub reparented_children: Vec<BoneId>,
    /// Slots that referenced the removed bone (and thus were removed).
    pub removed_slots: Vec<SlotId>,
    /// Names of IK constraints that referenced the removed bone.
    pub removed_constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Skeleton {
    pub bones: SlotMap<BoneId, Bone>,
    /// Topologically sorted bone ids (parents before children), rebuilt on
    /// every hierarchy edit via [`Skeleton::rebuild_update_order`]. World
    /// transforms are computed along this order — never insertion order.
    #[serde(skip)]
    pub update_order: Vec<BoneId>,
    #[serde(default)]
    pub constraints: SlotMap<ConstraintId, Constraint>,
    /// Order in which constraints are applied after the FK pass (PLAN §2.5).
    /// Explicit, because chained constraints are order-sensitive — never rely on
    /// slotmap iteration order.
    #[serde(default)]
    pub constraint_order: Vec<ConstraintId>,
    #[serde(default)]
    pub slots: SlotMap<SlotId, Slot>,
    #[serde(default)]
    pub draw_order: Vec<SlotId>,
    /// Attachment data, grouped into skins (PLAN §2.4, ADR 0003). Slots hold only
    /// attachment *names*; the data lives here and is looked up via
    /// [`Skeleton::resolve`].
    #[serde(default)]
    pub skins: SlotMap<SkinId, Skin>,
    /// The skin every lookup falls back to. Created by [`Skeleton::new`].
    #[serde(default)]
    pub default_skin: SkinId,
}

impl Skeleton {
    /// A skeleton with an empty default skin.
    ///
    /// Prefer this over `Skeleton::default()`: the derived `Default` leaves
    /// `default_skin` as the null key with no skin behind it, which only exists
    /// so serde can deserialize into a fresh value before filling the fields.
    pub fn new() -> Self {
        let mut skel = Self::default();
        skel.default_skin = skel.skins.insert(Skin::new("default"));
        skel
    }

    /// Resolve the attachment a slot should show, honoring the active skin with a
    /// fallback to the default skin (PLAN §2.4 — the **only** way renderers should
    /// obtain an attachment).
    pub fn resolve(&self, active: SkinId, slot: SlotId, name: &str) -> Option<&Attachment> {
        self.resolve_many(&[active], slot, name)
    }

    /// Resolve against **several** active skins, first match winning (T-507).
    ///
    /// Composition rather than one global "style": a hat skin and an armor skin
    /// should be wearable together, and a tool that can only have one active
    /// forces every combination to exist as its own skin. Order is priority, so
    /// the caller decides which layer wins a conflict, and the default skin is
    /// always the last fallback — a slot the outfits say nothing about still
    /// shows its base art rather than vanishing.
    pub fn resolve_many(&self, active: &[SkinId], slot: SlotId, name: &str) -> Option<&Attachment> {
        active
            .iter()
            .find_map(|id| self.skins.get(*id).and_then(|skin| skin.get(slot, name)))
            .or_else(|| {
                self.skins
                    .get(self.default_skin)
                    .and_then(|skin| skin.get(slot, name))
            })
    }

    /// Resolve whatever the slot currently points at, if anything.
    pub fn resolve_slot(&self, active: SkinId, slot: SlotId) -> Option<&Attachment> {
        self.resolve_slot_many(&[active], slot)
    }

    /// [`Self::resolve_slot`] against several active skins (T-507).
    pub fn resolve_slot_many(&self, active: &[SkinId], slot: SlotId) -> Option<&Attachment> {
        let name = self.slots.get(slot)?.attachment.as_deref()?;
        self.resolve_many(active, slot, name)
    }

    /// Insert a skin and return its id.
    pub fn add_skin(&mut self, skin: Skin) -> SkinId {
        self.skins.insert(skin)
    }

    /// Insert a bone, return its id, and rebuild the update order.
    ///
    /// The name is made unique first: `.ankh` serializes entities **by name**
    /// (ADR 0004), so two bones sharing a name would collide on save.
    pub fn add_bone(&mut self, mut bone: Bone) -> BoneId {
        bone.name = unique_name(&bone.name, self.bones.values().map(|b| b.name.as_str()));
        let id = self.bones.insert(bone);
        self.rebuild_update_order();
        id
    }

    /// Insert a slot with a unique name, appending it to the setup draw order.
    pub fn add_slot(&mut self, mut slot: Slot) -> SlotId {
        slot.name = unique_name(&slot.name, self.slots.values().map(|s| s.name.as_str()));
        let id = self.slots.insert(slot);
        self.draw_order.push(id);
        id
    }

    /// Rename a bone, making the new name unique among the *other* bones.
    /// Returns the name actually assigned.
    pub fn rename_bone(&mut self, id: BoneId, name: &str) -> Option<String> {
        let taken: Vec<String> = self
            .bones
            .iter()
            .filter(|(other, _)| *other != id)
            .map(|(_, b)| b.name.clone())
            .collect();
        let unique = unique_name(name, taken.iter().map(|s| s.as_str()));
        let bone = self.bones.get_mut(id)?;
        bone.name = unique.clone();
        self.rebuild_update_order();
        Some(unique)
    }

    /// The **setup** world affine of a bone: its local transform composed up the
    /// parent chain, ignoring animation. Used by editor reparenting to keep a bone
    /// visually fixed while its parent changes (T-206, ADR 0002).
    pub fn setup_world(&self, id: BoneId) -> crate::transforms::Affine2 {
        let Some(bone) = self.bones.get(id) else {
            return crate::transforms::Affine2::IDENTITY;
        };
        match bone.parent {
            Some(parent) => crate::transforms::Affine2::compose_child(
                &self.setup_world(parent),
                &bone.local_transform,
                &bone.inherit,
            ),
            None => bone.local_transform.to_affine(),
        }
    }

    /// Insert a constraint, appending it to the end of [`Self::constraint_order`].
    pub fn add_constraint(&mut self, constraint: Constraint) -> ConstraintId {
        let id = self.constraints.insert(constraint);
        self.constraint_order.push(id);
        id
    }

    /// Remove a constraint and drop it from the application order.
    pub fn remove_constraint(&mut self, id: ConstraintId) -> Option<Constraint> {
        self.constraint_order.retain(|&c| c != id);
        self.constraints.remove(id)
    }

    /// Constraints in application order, skipping any dangling ids.
    pub fn ordered_constraints(&self) -> impl Iterator<Item = (ConstraintId, &Constraint)> {
        self.constraint_order
            .iter()
            .filter_map(|&id| self.constraints.get(id).map(|c| (id, c)))
    }

    /// Rebuild [`update_order`] via pre-order DFS, roots sorted by name and
    /// each parent's children sorted by name. Deterministic regardless of
    /// insertion order. See ADR 0001.
    pub fn rebuild_update_order(&mut self) {
        use std::collections::{HashMap, HashSet};

        let mut children: HashMap<BoneId, Vec<BoneId>> = HashMap::new();
        let mut roots: Vec<BoneId> = Vec::new();
        for (id, bone) in self.bones.iter() {
            match bone.parent {
                Some(parent) => children.entry(parent).or_default().push(id),
                None => roots.push(id),
            }
        }
        for group in children.values_mut() {
            group.sort_by(|&a, &b| self.bones[a].name.cmp(&self.bones[b].name));
        }
        roots.sort_by(|&a, &b| self.bones[a].name.cmp(&self.bones[b].name));

        let mut order = Vec::with_capacity(self.bones.len());
        let mut visited = HashSet::new();
        // Iterative pre-order DFS. Push children reversed so the first child is
        // popped (visited) first — matches the recursive name-sorted order.
        let mut stack: Vec<BoneId> = roots;
        stack.reverse();
        while let Some(id) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            order.push(id);
            if let Some(kids) = children.get(&id) {
                for &kid in kids.iter().rev() {
                    stack.push(kid);
                }
            }
        }

        // Defensive: if a malformed parent pointer left some bones unreached,
        // append them so update_order still contains every bone.
        if order.len() != self.bones.len() {
            for (id, _) in self.bones.iter() {
                if !visited.contains(&id) {
                    order.push(id);
                }
            }
        }

        self.update_order = order;
    }

    /// Remove a bone, reparenting its children to the bone's parent and
    /// tearing down any slots / constraints that depended on it. All other
    /// entity ids stay valid. See ADR 0001 / defect D1.
    pub fn remove_bone(&mut self, id: BoneId) -> Option<RemoveBoneReport> {
        let bone = self.bones.get(id)?;
        let parent = bone.parent;
        let name = bone.name.clone();
        let mut report = RemoveBoneReport {
            removed_bone_name: name,
            ..Default::default()
        };

        // Re-parent direct children to the removed bone's parent.
        let children: Vec<BoneId> = self
            .bones
            .iter()
            .filter(|(_, b)| b.parent == Some(id))
            .map(|(cid, _)| cid)
            .collect();
        for &child in &children {
            if let Some(cb) = self.bones.get_mut(child) {
                cb.parent = parent;
            }
        }
        report.reparented_children = children;

        // Drop slots that reference this bone.
        let removed_slots: Vec<SlotId> = self
            .slots
            .iter()
            .filter(|(_, s)| s.bone == id)
            .map(|(sid, _)| sid)
            .collect();
        for &sid in &removed_slots {
            self.slots.remove(sid);
            self.draw_order.retain(|&s| s != sid);
            // Attachment data lives in the skins now, so every skin has to drop
            // its entries for this slot or they leak (ADR 0003).
            for (_, skin) in self.skins.iter_mut() {
                skin.remove_slot(sid);
            }
        }
        report.removed_slots = removed_slots;

        // Drop constraints that reference this bone, as target or chain member.
        let doomed: Vec<(ConstraintId, String)> = self
            .constraints
            .iter()
            .filter(|(_, c)| match c {
                Constraint::Ik(ik) => ik.target == id || ik.bones.contains(&id),
                Constraint::Transform(tc) => tc.target == id || tc.bones.contains(&id),
                Constraint::Physics(p) => p.bone == id,
                Constraint::Path(p) => p.bones.contains(&id),
            })
            .map(|(cid, c)| (cid, c.name().to_string()))
            .collect();
        for (cid, name) in doomed {
            self.remove_constraint(cid);
            report.removed_constraints.push(name);
        }

        self.bones.remove(id);
        self.rebuild_update_order();
        Some(report)
    }

    /// `true` if `descendant` is anywhere below `ancestor` in the hierarchy.
    pub fn is_descendant(&self, descendant: BoneId, ancestor: BoneId) -> bool {
        let mut id = descendant;
        while let Some(p) = self.bones.get(id).and_then(|b| b.parent) {
            if p == ancestor {
                return true;
            }
            id = p;
        }
        false
    }

    // The world pass lives in `crate::pose::evaluate` (T-103). A `Skeleton` is
    // document data and holds no derived state, so there is deliberately no
    // `update_world_transforms` here any more: call `evaluate` into a `Pose`.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{Rect, RegionAttachment};
    use crate::constraints::IkConstraint;

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
        })
    }

    fn bone(name: &str, parent: Option<BoneId>) -> Bone {
        Bone {
            name: name.to_string(),
            parent,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn update_order_is_topological_regardless_of_insertion() {
        let mut skel = Skeleton::new();
        // Insert in reverse hierarchy order: child first, then parent, then root.
        let root = skel.add_bone(bone("root", None));
        let mid = skel.add_bone(bone("mid", Some(root)));
        let leaf = skel.add_bone(bone("leaf", Some(mid)));

        skel.rebuild_update_order();
        let order = skel.update_order.clone();
        let pos = |id: BoneId| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(root) < pos(mid));
        assert!(pos(mid) < pos(leaf));
    }

    #[test]
    fn update_order_tiebreaks_by_name() {
        let mut skel = Skeleton::new();
        let z = skel.add_bone(bone("zeta", None));
        let a = skel.add_bone(bone("alpha", None));
        let m = skel.add_bone(bone("middle", None));

        // Roots inserted out of name order; update_order should be alpha, middle, zeta.
        skel.rebuild_update_order();
        let order = skel.update_order.clone();
        assert_eq!(order, vec![a, m, z]);
    }

    #[test]
    fn delete_middle_bone_keeps_identity_and_reparents() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let mid = skel.add_bone(bone("mid", Some(root)));
        let leaf = skel.add_bone(bone("leaf", Some(mid)));

        // Children ids before deletion.
        let report = skel.remove_bone(mid).expect("bone exists");

        // `mid` is gone; `root` and `leaf` keep their ids.
        assert!(skel.bones.get(mid).is_none());
        assert!(skel.bones.get(root).is_some());
        assert!(skel.bones.get(leaf).is_some());

        // `leaf` was reparented to `root`.
        assert_eq!(skel.bones.get(leaf).unwrap().parent, Some(root));
        assert_eq!(report.reparented_children, vec![leaf]);

        // update_order still topological and excludes the removed bone.
        assert!(!skel.update_order.contains(&mid));
        let order = skel.update_order.clone();
        let pos = |id: BoneId| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(root) < pos(leaf));
    }

    #[test]
    fn delete_bone_removes_dependent_slots() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let child = skel.add_bone(bone("child", Some(root)));

        let slot_id = skel.slots.insert(crate::slot::Slot::new("s".into(), child));
        skel.draw_order.push(slot_id);

        let report = skel.remove_bone(child).expect("bone exists");
        assert_eq!(report.removed_slots, vec![slot_id]);
        assert!(skel.slots.get(slot_id).is_none());
        assert!(!skel.draw_order.contains(&slot_id));
    }

    #[test]
    fn delete_bone_removes_dependent_ik_constraints() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a", None));
        let b = skel.add_bone(bone("b", Some(a)));
        let target = skel.add_bone(bone("target", None));

        skel.add_constraint(Constraint::Ik(IkConstraint::two_bone("ik", target, [a, b])));

        skel.remove_bone(b).expect("bone exists");
        // The constraint referencing `b` must be gone, and its name reported.
        assert!(skel.constraints.is_empty());
        assert!(skel.constraint_order.is_empty());
    }

    #[test]
    fn delete_bone_reports_removed_constraint_names() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a", None));
        let b = skel.add_bone(bone("b", Some(a)));
        let target = skel.add_bone(bone("target", None));

        skel.add_constraint(Constraint::Ik(IkConstraint::two_bone(
            "left_leg",
            target,
            [a, b],
        )));
        skel.add_constraint(Constraint::Ik(IkConstraint::aim("look_at", target, a)));

        // Removing the *target* tears down both constraints that reference it.
        let report = skel.remove_bone(target).expect("bone exists");
        assert_eq!(report.removed_constraints.len(), 2);
        assert!(report.removed_constraints.contains(&"left_leg".to_string()));
        assert!(report.removed_constraints.contains(&"look_at".to_string()));
        assert!(skel.constraints.is_empty());
    }

    #[test]
    fn new_skeleton_has_a_default_skin() {
        let skel = Skeleton::new();
        assert!(skel.skins.get(skel.default_skin).is_some());
        assert_eq!(skel.skins.len(), 1);
    }

    #[test]
    fn resolve_falls_back_to_default_skin() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(Slot::new("arm".into(), root));

        // Only the default skin defines "arm".
        let default_skin = skel.default_skin;
        skel.skins[default_skin].set(slot, "arm", region("default_arm.png"));

        // An alternate skin that overrides nothing for this slot.
        let alt = skel.add_skin(crate::skin::Skin::new("alt"));

        match skel.resolve(alt, slot, "arm") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "default_arm.png"),
            other => panic!("expected fallback to the default skin, got {other:?}"),
        }
    }

    #[test]
    fn active_skin_overrides_default() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(Slot::new("arm".into(), root));

        let default_skin = skel.default_skin;
        skel.skins[default_skin].set(slot, "arm", region("default_arm.png"));
        let alt = skel.add_skin(crate::skin::Skin::new("alt"));
        skel.skins[alt].set(slot, "arm", region("alt_arm.png"));

        match skel.resolve(alt, slot, "arm") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "alt_arm.png"),
            other => panic!("expected the active skin to win, got {other:?}"),
        }
        // The default skin is untouched by the override.
        match skel.resolve(default_skin, slot, "arm") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "default_arm.png"),
            other => panic!("expected the default entry intact, got {other:?}"),
        }
    }

    #[test]
    fn swapping_skin_does_not_touch_slots() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(Slot::new("arm".into(), root));
        skel.slots[slot].attachment = Some("arm".into());

        let default_skin = skel.default_skin;
        skel.skins[default_skin].set(slot, "arm", region("default_arm.png"));
        let alt = skel.add_skin(crate::skin::Skin::new("alt"));
        skel.skins[alt].set(slot, "arm", region("alt_arm.png"));

        let slot_before = skel.slots[slot].attachment.clone();

        // Resolving through a different skin swaps the rendered attachment...
        match skel.resolve_slot(alt, slot) {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "alt_arm.png"),
            other => panic!("expected alt attachment, got {other:?}"),
        }
        // ...without the slot's stored name changing (PLAN §2.4, normative).
        assert_eq!(skel.slots[slot].attachment, slot_before);
    }

    #[test]
    fn resolve_returns_none_for_unknown_name() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(Slot::new("arm".into(), root));
        let default_skin = skel.default_skin;

        assert!(skel.resolve(default_skin, slot, "nope").is_none());
        // A slot with no attachment name resolves to nothing.
        assert!(skel.resolve_slot(default_skin, slot).is_none());
    }

    #[test]
    fn deleting_a_bone_clears_its_slots_skin_entries() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let child = skel.add_bone(bone("child", Some(root)));
        let slot = skel.slots.insert(Slot::new("arm".into(), child));

        let default_skin = skel.default_skin;
        skel.skins[default_skin].set(slot, "arm", region("arm.png"));
        let alt = skel.add_skin(crate::skin::Skin::new("alt"));
        skel.skins[alt].set(slot, "arm", region("alt_arm.png"));

        skel.remove_bone(child).expect("bone exists");

        // The slot is gone, and no skin still holds data for it.
        assert!(skel.slots.get(slot).is_none());
        for (_, skin) in skel.skins.iter() {
            assert_eq!(skin.names_for_slot(slot).count(), 0);
        }
    }

    #[test]
    fn constraint_order_tracks_add_and_remove() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a", None));
        let t = skel.add_bone(bone("t", None));

        let first = skel.add_constraint(Constraint::Ik(IkConstraint::aim("first", t, a)));
        let second = skel.add_constraint(Constraint::Ik(IkConstraint::aim("second", t, a)));
        assert_eq!(skel.constraint_order, vec![first, second]);

        // Application order follows `constraint_order`, not slotmap order.
        let names: Vec<&str> = skel.ordered_constraints().map(|(_, c)| c.name()).collect();
        assert_eq!(names, vec!["first", "second"]);

        skel.remove_constraint(first);
        assert_eq!(skel.constraint_order, vec![second]);
        assert_eq!(skel.ordered_constraints().count(), 1);
    }
    /// The acceptance case for T-507: a hat skin and an armor skin worn
    /// together resolve both, and the first listed wins a conflict.
    #[test]
    fn composed_skins_layer_with_the_first_winning() {
        use crate::attachment::{Attachment, ClippingAttachment};

        let mut skel = Skeleton::new();
        let bone = skel.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 1.0,
            local_transform: Transform::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        let head = skel.add_slot(Slot::new("head".into(), bone));
        let torso = skel.add_slot(Slot::new("torso".into(), bone));
        let feet = skel.add_slot(Slot::new("feet".into(), bone));

        let clip = |n: f32| {
            Attachment::Clipping(ClippingAttachment {
                vertices: vec![glam::vec2(n, 0.0)],
                end_slot: None,
            })
        };
        let marker = |a: &Attachment| match a {
            Attachment::Clipping(c) => c.vertices[0].x,
            _ => -1.0,
        };

        // Base art for every slot, plus two outfits that each dress one slot —
        // and both dress the torso, which is the conflict.
        let default = skel.default_skin;
        for (slot, name) in [(head, "head"), (torso, "torso"), (feet, "feet")] {
            skel.skins[default].set(slot, name.to_string(), clip(0.0));
        }
        let hat = skel.add_skin(crate::skin::Skin::new("hat"));
        skel.skins[hat].set(head, "head".to_string(), clip(1.0));
        skel.skins[hat].set(torso, "torso".to_string(), clip(1.0));
        let armor = skel.add_skin(crate::skin::Skin::new("armor"));
        skel.skins[armor].set(torso, "torso".to_string(), clip(2.0));

        // Hat first: it wins the torso.
        let stack = [hat, armor];
        assert_eq!(
            marker(skel.resolve_many(&stack, head, "head").unwrap()),
            1.0,
            "the hat skin dressed the head"
        );
        assert_eq!(
            marker(skel.resolve_many(&stack, torso, "torso").unwrap()),
            1.0,
            "first match wins the conflict"
        );
        assert_eq!(
            marker(skel.resolve_many(&stack, feet, "feet").unwrap()),
            0.0,
            "a slot no outfit mentions falls back to the default skin"
        );

        // Reverse the priority and the armor takes the torso instead.
        let stack = [armor, hat];
        assert_eq!(
            marker(skel.resolve_many(&stack, torso, "torso").unwrap()),
            2.0
        );
        assert_eq!(
            marker(skel.resolve_many(&stack, head, "head").unwrap()),
            1.0,
            "the hat still dresses the head; only the conflict changed"
        );
    }

    #[test]
    fn resolving_against_no_skins_still_finds_the_default() {
        use crate::attachment::{Attachment, ClippingAttachment};

        let mut skel = Skeleton::new();
        let bone = skel.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 1.0,
            local_transform: Transform::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        let slot = skel.add_slot(Slot::new("art".into(), bone));
        let default = skel.default_skin;
        skel.skins[default].set(
            slot,
            "art".to_string(),
            Attachment::Clipping(ClippingAttachment::default()),
        );
        assert!(skel.resolve_many(&[], slot, "art").is_some());
    }
}
