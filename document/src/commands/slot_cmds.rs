//! Slot mutations as undoable commands (PLAN §3.2).

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::ids::{BoneId, SlotId};
use ankhimate_core::slot::{BlendMode, Slot};

/// Create a slot on a bone and append it to the setup draw order.
pub struct CreateSlot {
    slot: Slot,
    created: Option<SlotId>,
}

impl CreateSlot {
    pub fn new(name: impl Into<String>, bone: BoneId) -> Self {
        Self {
            slot: Slot::new(name.into(), bone),
            created: None,
        }
    }

    pub fn created_id(&self) -> Option<SlotId> {
        self.created
    }
}

impl EditCommand for CreateSlot {
    fn apply(&mut self, doc: &mut Document) {
        let id = doc.skeleton.slots.insert(self.slot.clone());
        doc.skeleton.draw_order.push(id);
        self.created = Some(id);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.skeleton.slots.remove(id);
            doc.skeleton.draw_order.retain(|&s| s != id);
            for (_, skin) in doc.skeleton.skins.iter_mut() {
                skin.remove_slot(id);
            }
        }
    }

    fn label(&self) -> &str {
        "Create Slot"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete a slot, its draw-order entry, and its attachment data in every skin.
pub struct DeleteSlot {
    target: SlotId,
    removed: Option<Slot>,
    /// Index in `draw_order`, so undo puts it back where it was rather than at
    /// the end (which would silently change what draws on top).
    draw_index: Option<usize>,
    /// Attachment entries per skin, so undo restores the artwork too.
    entries: Vec<(
        ankhimate_core::ids::SkinId,
        String,
        ankhimate_core::attachment::Attachment,
    )>,
    restored: Option<SlotId>,
}

impl DeleteSlot {
    pub fn new(target: SlotId) -> Self {
        Self {
            target,
            removed: None,
            draw_index: None,
            entries: Vec::new(),
            restored: None,
        }
    }
}

impl EditCommand for DeleteSlot {
    fn apply(&mut self, doc: &mut Document) {
        let id = self.restored.take().unwrap_or(self.target);
        let Some(slot) = doc.skeleton.slots.get(id).cloned() else {
            return;
        };
        self.removed = Some(slot);
        self.draw_index = doc.skeleton.draw_order.iter().position(|&s| s == id);

        // Capture attachment data before it is dropped.
        self.entries.clear();
        for (skin_id, skin) in doc.skeleton.skins.iter() {
            for name in skin.names_for_slot(id) {
                if let Some(att) = skin.get(id, name) {
                    self.entries.push((skin_id, name.to_string(), att.clone()));
                }
            }
        }

        doc.skeleton.slots.remove(id);
        doc.skeleton.draw_order.retain(|&s| s != id);
        for (_, skin) in doc.skeleton.skins.iter_mut() {
            skin.remove_slot(id);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(slot) = self.removed.take() else {
            return;
        };
        let id = doc.skeleton.slots.insert(slot);
        match self.draw_index {
            Some(i) if i <= doc.skeleton.draw_order.len() => doc.skeleton.draw_order.insert(i, id),
            _ => doc.skeleton.draw_order.push(id),
        }
        for (skin_id, name, att) in self.entries.drain(..) {
            if let Some(skin) = doc.skeleton.skins.get_mut(skin_id) {
                skin.set(id, name, att);
            }
        }
        self.restored = Some(id);
    }

    fn label(&self) -> &str {
        "Delete Slot"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Change which attachment **name** a slot shows.
///
/// Only the name — attachment data belongs to skins and is never touched by this
/// (PLAN §2.4, normative).
pub struct SetSlotAttachment {
    slot: SlotId,
    name: Option<String>,
    before: Option<Option<String>>,
}

impl SetSlotAttachment {
    pub fn new(slot: SlotId, name: Option<String>) -> Self {
        Self {
            slot,
            name,
            before: None,
        }
    }
}

impl EditCommand for SetSlotAttachment {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(s) = doc.skeleton.slots.get_mut(self.slot) {
            if self.before.is_none() {
                self.before = Some(s.attachment.clone());
            }
            s.attachment = self.name.clone();
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(s)) = (self.before.take(), doc.skeleton.slots.get_mut(self.slot))
        {
            s.attachment = before;
        }
    }

    fn label(&self) -> &str {
        "Set Attachment"
    }

    /// Setup value; Animate mode writes a stepped `SlotAttachment` key instead.
    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a slot's setup color (RGBA). Merges so dragging the picker is one step.
pub struct SetSlotColor {
    slot: SlotId,
    color: [f32; 4],
    before: Option<[f32; 4]>,
}

impl SetSlotColor {
    pub fn new(slot: SlotId, color: [f32; 4]) -> Self {
        Self {
            slot,
            color,
            before: None,
        }
    }
}

impl EditCommand for SetSlotColor {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(s) = doc.skeleton.slots.get_mut(self.slot) {
            if self.before.is_none() {
                self.before = Some(s.color);
            }
            s.color = self.color;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(s)) = (self.before, doc.skeleton.slots.get_mut(self.slot)) {
            s.color = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        if let Some(other) = next.as_any().downcast_ref::<SetSlotColor>()
            && other.slot == self.slot
        {
            self.color = other.color;
            return true;
        }
        false
    }

    fn label(&self) -> &str {
        "Set Slot Color"
    }

    /// Setup value; Animate mode writes a `SlotColor` key instead.
    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Reorder the setup draw order.
pub struct SetDrawOrder {
    order: Vec<SlotId>,
    before: Option<Vec<SlotId>>,
}

impl SetDrawOrder {
    pub fn new(order: Vec<SlotId>) -> Self {
        Self {
            order,
            before: None,
        }
    }
}

impl EditCommand for SetDrawOrder {
    fn apply(&mut self, doc: &mut Document) {
        if self.before.is_none() {
            self.before = Some(doc.skeleton.draw_order.clone());
        }
        doc.skeleton.draw_order = self.order.clone();
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(before) = self.before.take() {
            doc.skeleton.draw_order = before;
        }
    }

    fn label(&self) -> &str {
        "Reorder Slots"
    }

    /// Setup draw order; Animate mode writes a `DrawOrder` key instead (T-204).
    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Move a slot to another bone while keeping its setup artwork fixed in world
/// space. The attachment coordinates are rewritten because they live in the
/// owning bone's local frame.
pub struct SetSlotBone {
    slot: SlotId,
    bone: BoneId,
    local_map: ankhimate_core::transforms::Affine2,
    before_bone: Option<BoneId>,
    before_attachments: Option<
        Vec<(
            ankhimate_core::ids::SkinId,
            String,
            ankhimate_core::attachment::Attachment,
        )>,
    >,
    order: Option<Vec<SlotId>>,
    before_order: Option<Vec<SlotId>>,
}

impl SetSlotBone {
    pub fn keeping_world(
        skeleton: &ankhimate_core::skeleton::Skeleton,
        slot: SlotId,
        bone: BoneId,
    ) -> Self {
        let old_world = skeleton
            .slots
            .get(slot)
            .map(|slot| skeleton.setup_world(slot.bone))
            .unwrap_or_default();
        let local_map = skeleton
            .setup_world(bone)
            .invert()
            .unwrap_or_default()
            .mul(&old_world);
        Self {
            slot,
            bone,
            local_map,
            before_bone: None,
            before_attachments: None,
            order: None,
            before_order: None,
        }
    }

    /// Also place the slot at a particular draw-order position as part of the
    /// same drag gesture and undo step.
    pub fn with_order(mut self, order: Vec<SlotId>) -> Self {
        self.order = Some(order);
        self
    }
}

fn map_attachment(
    attachment: &mut ankhimate_core::attachment::Attachment,
    map: &ankhimate_core::transforms::Affine2,
) {
    use ankhimate_core::attachment::Attachment;
    match attachment {
        Attachment::Region(region) => {
            let local = ankhimate_core::math::Transform {
                position: region.local_offset,
                rotation: region.local_rotation,
                scale: region.local_scale,
                shear: glam::Vec2::ZERO,
            };
            let transformed = map.mul(&local.to_affine()).decompose();
            region.local_offset = transformed.position;
            region.local_rotation = transformed.rotation;
            region.local_scale = transformed.scale;
        }
        Attachment::Mesh(mesh) if mesh.weights.is_empty() && mesh.linked.is_none() => {
            for vertex in &mut mesh.setup_vertices {
                *vertex = map.transform_point(*vertex);
            }
        }
        Attachment::Clipping(clip) => {
            for vertex in &mut clip.vertices {
                *vertex = map.transform_point(*vertex);
            }
        }
        Attachment::Path(path) => {
            for vertex in &mut path.vertices {
                *vertex = map.transform_point(*vertex);
            }
        }
        Attachment::BoundingBox(bounds) if bounds.weights.is_empty() => {
            for vertex in &mut bounds.vertices {
                *vertex = map.transform_point(*vertex);
            }
        }
        Attachment::Point(point) => {
            point.position = map.transform_point(point.position);
            point.rotation = map
                .transform_vector(glam::Vec2::from_angle(point.rotation))
                .to_angle();
        }
        // Weighted and linked geometry already resolves through its own bones or
        // source mesh, so changing the slot's fallback bone must not rewrite it.
        Attachment::Mesh(_) | Attachment::BoundingBox(_) => {}
    }
}

impl EditCommand for SetSlotBone {
    fn apply(&mut self, doc: &mut Document) {
        if !doc.skeleton.bones.contains_key(self.bone) {
            return;
        }
        let Some(slot) = doc.skeleton.slots.get_mut(self.slot) else {
            return;
        };
        let changes_bone = slot.bone != self.bone;
        if !changes_bone && self.order.is_none() {
            return;
        }
        if changes_bone && self.before_bone.is_none() {
            let slot_id = self.slot;
            self.before_bone = Some(slot.bone);
            self.before_attachments = Some(
                doc.skeleton
                    .skins
                    .iter()
                    .flat_map(|(skin_id, skin)| {
                        skin.names_for_slot(slot_id).filter_map(move |name| {
                            skin.get(slot_id, name)
                                .cloned()
                                .map(|attachment| (skin_id, name.to_string(), attachment))
                        })
                    })
                    .collect(),
            );
        }
        if changes_bone {
            slot.bone = self.bone;
            for (_, skin) in doc.skeleton.skins.iter_mut() {
                let names: Vec<String> =
                    skin.names_for_slot(self.slot).map(str::to_string).collect();
                for name in names {
                    if let Some(attachment) = skin.entries.get_mut(&(self.slot, name)) {
                        map_attachment(attachment, &self.local_map);
                    }
                }
            }
        }
        if let Some(order) = &self.order {
            self.before_order
                .get_or_insert_with(|| doc.skeleton.draw_order.clone());
            doc.skeleton.draw_order = order.clone();
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(slot)) =
            (self.before_bone, doc.skeleton.slots.get_mut(self.slot))
        {
            slot.bone = before;
        }
        if let Some(attachments) = &self.before_attachments {
            for (skin, name, attachment) in attachments {
                if let Some(skin) = doc.skeleton.skins.get_mut(*skin) {
                    skin.set(self.slot, name, attachment.clone());
                }
            }
        }
        if let Some(before) = &self.before_order {
            doc.skeleton.draw_order = before.clone();
        }
    }

    fn label(&self) -> &str {
        "Move Slot"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Everything about how a slot composites, as one value (T-505).
#[derive(Clone, Copy, PartialEq)]
pub struct SlotPresentation {
    pub blend_mode: BlendMode,
    pub dark_color: Option<[f32; 4]>,
}

/// Set a slot's blend mode and two-color tint.
///
/// Setup-only: which way a slot composites is rig data, not a pose. The
/// *animated* half is `SlotColor` and `SlotVisible`, which are timelines.
pub struct SetSlotPresentation {
    slot: SlotId,
    after: SlotPresentation,
    before: Option<SlotPresentation>,
}

impl SetSlotPresentation {
    pub fn new(slot: SlotId, after: SlotPresentation) -> Self {
        Self {
            slot,
            after,
            before: None,
        }
    }
}

impl EditCommand for SetSlotPresentation {
    fn apply(&mut self, doc: &mut Document) {
        let Some(slot) = doc.skeleton.slots.get_mut(self.slot) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(SlotPresentation {
                blend_mode: slot.blend_mode,
                dark_color: slot.dark_color,
            });
        }
        slot.blend_mode = self.after.blend_mode;
        slot.dark_color = self.after.dark_color;
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(slot)) =
            (self.before.take(), doc.skeleton.slots.get_mut(self.slot))
        {
            slot.blend_mode = before.blend_mode;
            slot.dark_color = before.dark_color;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetSlotPresentation>() else {
            return false;
        };
        if other.slot != self.slot {
            return false;
        }
        // Dragging in the colour picker is one edit.
        self.after = other.after;
        true
    }

    fn label(&self) -> &str {
        "Set Slot Presentation"
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
    use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

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

    fn doc_with_bone() -> (Document, BoneId) {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        (doc, bone)
    }

    #[test]
    fn create_slot_undo_removes_it_from_draw_order_too() {
        let (mut doc, bone) = doc_with_bone();
        let mut history = History::default();

        history.push(Box::new(CreateSlot::new("arm", bone)), &mut doc);
        assert_eq!(doc.skeleton.slots.len(), 1);
        assert_eq!(doc.skeleton.draw_order.len(), 1);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.slots.len(), 0);
        assert!(doc.skeleton.draw_order.is_empty(), "draw order cleaned up");
    }

    #[test]
    fn delete_slot_undo_restores_attachments_and_draw_position() {
        let (mut doc, bone) = doc_with_bone();
        // Three slots; we delete the middle one.
        let a = doc.skeleton.slots.insert(Slot::new("a".into(), bone));
        let b = doc.skeleton.slots.insert(Slot::new("b".into(), bone));
        let c = doc.skeleton.slots.insert(Slot::new("c".into(), bone));
        doc.skeleton.draw_order = vec![a, b, c];

        let default_skin = doc.skeleton.default_skin;
        doc.skeleton.skins[default_skin].set(b, "art", region("b.png"));

        let mut history = History::default();
        history.push(Box::new(DeleteSlot::new(b)), &mut doc);
        assert_eq!(doc.skeleton.draw_order, vec![a, c]);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.draw_order.len(), 3);
        // Restored into the *middle*, not appended.
        let restored = doc.skeleton.draw_order[1];
        assert_eq!(doc.skeleton.slots[restored].name, "b");
        // And its artwork came back.
        match doc.skeleton.skins[default_skin].get(restored, "art") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "b.png"),
            other => panic!("attachment not restored: {other:?}"),
        }
    }

    #[test]
    fn delete_slot_redo_after_undo_works() {
        let (mut doc, bone) = doc_with_bone();
        let s = doc.skeleton.slots.insert(Slot::new("s".into(), bone));
        doc.skeleton.draw_order.push(s);

        let mut history = History::default();
        history.push(Box::new(DeleteSlot::new(s)), &mut doc);
        history.undo(&mut doc);
        assert_eq!(doc.skeleton.slots.len(), 1);

        history.redo(&mut doc);
        assert_eq!(doc.skeleton.slots.len(), 0, "redo used the restored id");
    }

    #[test]
    fn set_attachment_changes_only_the_name() {
        let (mut doc, bone) = doc_with_bone();
        let s = doc.skeleton.slots.insert(Slot::new("s".into(), bone));
        let default_skin = doc.skeleton.default_skin;
        doc.skeleton.skins[default_skin].set(s, "open", region("open.png"));
        doc.skeleton.skins[default_skin].set(s, "shut", region("shut.png"));

        let mut history = History::default();
        history.push(
            Box::new(SetSlotAttachment::new(s, Some("shut".into()))),
            &mut doc,
        );
        assert_eq!(doc.skeleton.slots[s].attachment, Some("shut".to_string()));
        // Both attachments still exist — data untouched.
        assert_eq!(
            doc.skeleton.skins[default_skin].names_for_slot(s).count(),
            2
        );

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.slots[s].attachment, None);
    }

    #[test]
    fn draw_order_reorder_roundtrips() {
        let (mut doc, bone) = doc_with_bone();
        let a = doc.skeleton.slots.insert(Slot::new("a".into(), bone));
        let b = doc.skeleton.slots.insert(Slot::new("b".into(), bone));
        doc.skeleton.draw_order = vec![a, b];

        let mut history = History::default();
        history.push(Box::new(SetDrawOrder::new(vec![b, a])), &mut doc);
        assert_eq!(doc.skeleton.draw_order, vec![b, a]);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.draw_order, vec![a, b]);
    }

    #[test]
    fn moving_a_slot_to_another_bone_keeps_region_world_position() {
        let (mut doc, root) = doc_with_bone();
        let child = doc.skeleton.add_bone(Bone {
            name: "child".into(),
            parent: Some(root),
            length: 10.0,
            local_transform: Transform {
                position: glam::vec2(25.0, 8.0),
                rotation: 0.4,
                ..Transform::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = doc.skeleton.add_slot(Slot {
            attachment: Some("art".into()),
            ..Slot::new("art".into(), root)
        });
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(slot, "art", region("art"));
        let before = match doc.skeleton.skins[skin].get(slot, "art").unwrap() {
            Attachment::Region(region) => doc
                .skeleton
                .setup_world(root)
                .transform_point(region.local_offset),
            _ => unreachable!(),
        };
        let mut history = History::default();

        history.push(
            Box::new(SetSlotBone::keeping_world(&doc.skeleton, slot, child)),
            &mut doc,
        );

        assert_eq!(doc.skeleton.slots[slot].bone, child);
        let after = match doc.skeleton.skins[skin].get(slot, "art").unwrap() {
            Attachment::Region(region) => doc
                .skeleton
                .setup_world(child)
                .transform_point(region.local_offset),
            _ => unreachable!(),
        };
        assert!(before.distance(after) < 1e-4, "{before:?} != {after:?}");

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.slots[slot].bone, root);
    }
}
