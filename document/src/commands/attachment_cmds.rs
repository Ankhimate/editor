//! Attachment mutations as undoable commands (T-307).
//!
//! An attachment's transform is **rig data**, not animation: it is where the art
//! sits inside its slot, which is a Setup-mode decision (T-207). Animating the
//! same geometry is what `Deform` timelines are for (T-404).
//!
//! Every command here names the skin it edits. Attachments live in skins, and
//! resolution falls back from the active skin to the default (ADR 0003), so
//! "which one did I just change?" has to be answered explicitly rather than
//! guessed inside the command.

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::animation::Timeline;
use ankhimate_core::attachment::{Attachment, RegionAttachment, Sequence};
use ankhimate_core::ids::{SkinId, SlotId};

/// The editable transform of a region attachment — everything the inspector
/// shows, and nothing that identifies it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionProps {
    pub offset: glam::Vec2,
    /// Radians, like every other angle in core (ADR 0002).
    pub rotation: f32,
    pub scale: glam::Vec2,
    pub width: f32,
    pub height: f32,
    /// Normalized image coordinates; `(0,0)` bottom-left, `(0.5,0.5)` centre.
    pub pivot: glam::Vec2,
}

impl RegionProps {
    pub fn from_region(region: &RegionAttachment) -> Self {
        Self {
            offset: region.local_offset,
            rotation: region.local_rotation,
            scale: region.local_scale,
            width: region.width,
            height: region.height,
            pivot: region.pivot,
        }
    }

    fn apply_to(&self, region: &mut RegionAttachment) {
        region.local_offset = self.offset;
        region.local_rotation = self.rotation;
        region.local_scale = self.scale;
        region.width = self.width;
        region.height = self.height;
        region.pivot = self.pivot;
    }

    /// Move the pivot without moving the art: the offset shifts by the same
    /// amount the quad would have jumped.
    ///
    /// Re-pivoting is something you do *while* looking at placed artwork —
    /// having the sprite leap across the canvas each time would make finding the
    /// right pivot a game of chase.
    pub fn with_pivot_keeping_position(&self, pivot: glam::Vec2) -> Self {
        let size = glam::vec2(self.width, self.height) * self.scale;
        let delta = (pivot - self.pivot) * size;
        let (sin, cos) = self.rotation.sin_cos();
        let rotated = glam::vec2(delta.x * cos - delta.y * sin, delta.x * sin + delta.y * cos);
        Self {
            pivot,
            offset: self.offset + rotated,
            ..*self
        }
    }
}

/// Find the skin a slot's attachment actually resolves through: the active skin
/// if it defines the name, else the default (ADR 0003).
///
/// Edits go to the skin the value was *read* from, so a change never silently
/// creates an override in a skin the user was not looking at.
pub fn owning_skin(doc: &Document, active: SkinId, slot: SlotId, name: &str) -> Option<SkinId> {
    if doc
        .skeleton
        .skins
        .get(active)
        .is_some_and(|s| s.get(slot, name).is_some())
    {
        return Some(active);
    }
    let default = doc.skeleton.default_skin;
    doc.skeleton
        .skins
        .get(default)
        .and_then(|s| s.get(slot, name))
        .map(|_| default)
}

/// Set a region attachment's transform. Merges, so a spinbox drag is one step.
pub struct SetRegionProps {
    skin: SkinId,
    slot: SlotId,
    name: String,
    after: RegionProps,
    before: Option<RegionProps>,
}

impl SetRegionProps {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, after: RegionProps) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            after,
            before: None,
        }
    }

    fn region<'a>(&self, doc: &'a mut Document) -> Option<&'a mut RegionAttachment> {
        match doc
            .skeleton
            .skins
            .get_mut(self.skin)?
            .entries
            .get_mut(&(self.slot, self.name.clone()))?
        {
            Attachment::Region(r) => Some(r),
            // Meshes are edited in mesh mode; nothing else has a region
            // transform to set.
            _ => None,
        }
    }
}

impl EditCommand for SetRegionProps {
    fn apply(&mut self, doc: &mut Document) {
        let after = self.after;
        let capture = self.before.is_none();
        if let Some(region) = self.region(doc) {
            if capture {
                self.before = Some(RegionProps::from_region(region));
            }
            after.apply_to(region);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before else {
            return;
        };
        if let Some(region) = self.region(doc) {
            before.apply_to(region);
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<SetRegionProps>() {
            Some(other)
                if other.skin == self.skin
                    && other.slot == self.slot
                    && other.name == self.name =>
            {
                self.after = other.after;
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Edit Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set an attachment's sequence — its frames, rate and mode — or clear it.
///
/// A sequence is **rig data**, like the region transform beside it: it says what
/// this art *is*, not what it does in one animation. `evaluate()` derives the
/// showing frame from playback time (see `Pose::slot_sequence_frames`), so there
/// are no keys to write and nothing here belongs to a timeline.
///
/// `None` clears it, which is how a run that should never have been folded gets
/// taken apart — the frames stay in the asset database either way.
pub struct SetSequence {
    skin: SkinId,
    slot: SlotId,
    name: String,
    after: Option<Sequence>,
    before: Option<Option<Sequence>>,
}

impl SetSequence {
    pub fn new(
        skin: SkinId,
        slot: SlotId,
        name: impl Into<String>,
        after: Option<Sequence>,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            after,
            before: None,
        }
    }

    /// A sequence lives on either kind of textured attachment, so this returns
    /// the field rather than the attachment — the two cases differ in nothing
    /// else.
    fn sequence<'a>(&self, doc: &'a mut Document) -> Option<&'a mut Option<Sequence>> {
        match doc
            .skeleton
            .skins
            .get_mut(self.skin)?
            .entries
            .get_mut(&(self.slot, self.name.clone()))?
        {
            Attachment::Region(r) => Some(&mut r.sequence),
            Attachment::Mesh(m) => Some(&mut m.sequence),
            _ => None,
        }
    }
}

impl EditCommand for SetSequence {
    fn apply(&mut self, doc: &mut Document) {
        let after = self.after.clone();
        let capture = self.before.is_none();
        if let Some(sequence) = self.sequence(doc) {
            if capture {
                self.before = Some(sequence.clone());
            }
            *sequence = after;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.clone() else {
            return;
        };
        if let Some(sequence) = self.sequence(doc) {
            *sequence = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<SetSequence>() {
            // Dragging the fps spinner is one undo step, the same way dragging
            // any other number in the inspector is.
            //
            // Only between two *edits*, though. This command also creates and
            // clears, and merging a drag into the creation makes one undo throw
            // the sequence away — which is what happened the first time, and
            // reads to the user as undo doing something they did not do.
            // What separates them is the state this command *found*: an edit
            // started from a sequence, a creation started from nothing. So the
            // test is on `before`, not on either `after`.
            Some(other)
                if other.skin == self.skin
                    && other.slot == self.slot
                    && other.name == self.name
                    && matches!(self.before, Some(Some(_)))
                    && other.after.is_some() =>
            {
                self.after = other.after.clone();
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Edit Sequence"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Rename an attachment within one skin.
///
/// The name is the reference: a slot points at it and `SlotAttachment` keys
/// spell it out, so both are rewritten or the rename would blank the slot and
/// break every swap animation that mentioned it.
pub struct RenameAttachment {
    skin: SkinId,
    slot: SlotId,
    from: String,
    to: String,
    applied: bool,
}

impl RenameAttachment {
    pub fn new(skin: SkinId, slot: SlotId, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            skin,
            slot,
            from: from.into(),
            to: to.into(),
            applied: false,
        }
    }

    /// `old` → `new` across the skin entry, the slot's setup name, and every
    /// attachment key in every animation.
    fn rename(doc: &mut Document, skin: SkinId, slot: SlotId, old: &str, new: &str) -> bool {
        if old == new {
            return false;
        }
        let Some(skin_ref) = doc.skeleton.skins.get_mut(skin) else {
            return false;
        };
        // Refuse a collision rather than clobbering the other attachment.
        if skin_ref.get(slot, new).is_some() {
            return false;
        }
        let Some(attachment) = skin_ref.remove(slot, old) else {
            return false;
        };
        skin_ref.set(slot, new, attachment);

        if let Some(s) = doc.skeleton.slots.get_mut(slot)
            && s.attachment.as_deref() == Some(old)
        {
            s.attachment = Some(new.to_string());
        }

        for (_, anim) in doc.animations.iter_mut() {
            for timeline in &mut anim.timelines {
                if let Timeline::SlotAttachment { slot: s, keys } = timeline
                    && *s == slot
                {
                    for key in keys.iter_mut() {
                        if key.value.as_deref() == Some(old) {
                            key.value = Some(new.to_string());
                        }
                    }
                }
            }
        }
        true
    }
}

impl EditCommand for RenameAttachment {
    fn apply(&mut self, doc: &mut Document) {
        self.applied = Self::rename(doc, self.skin, self.slot, &self.from, &self.to);
    }

    fn revert(&mut self, doc: &mut Document) {
        if self.applied {
            Self::rename(doc, self.skin, self.slot, &self.to, &self.from);
            self.applied = false;
        }
    }

    fn label(&self) -> &str {
        "Rename Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Copy an attachment under a new name in the same slot and skin — the way to
/// build a swap set (open/closed eyes) from art that is already placed.
pub struct DuplicateAttachment {
    skin: SkinId,
    slot: SlotId,
    source: String,
    created: Option<String>,
}

impl DuplicateAttachment {
    pub fn new(skin: SkinId, slot: SlotId, source: impl Into<String>) -> Self {
        Self {
            skin,
            slot,
            source: source.into(),
            created: None,
        }
    }

    pub fn created_name(&self) -> Option<&str> {
        self.created.as_deref()
    }
}

impl EditCommand for DuplicateAttachment {
    fn apply(&mut self, doc: &mut Document) {
        let Some(skin) = doc.skeleton.skins.get_mut(self.skin) else {
            return;
        };
        let Some(source) = skin.get(self.slot, &self.source).cloned() else {
            return;
        };
        // `name_2`, `name_3`, … — the same uniquifying rule bones and assets use.
        let mut n = 2;
        let name = loop {
            let candidate = format!("{}_{n}", self.source);
            if skin.get(self.slot, &candidate).is_none() {
                break candidate;
            }
            n += 1;
        };
        skin.set(self.slot, name.clone(), source);
        self.created = Some(name);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(name) = self.created.take()
            && let Some(skin) = doc.skeleton.skins.get_mut(self.skin)
        {
            skin.remove(self.slot, &name);
        }
    }

    fn label(&self) -> &str {
        "Duplicate Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Remove an attachment from one skin. The slot keeps pointing at the name, so
/// resolution falls back to the default skin — which is the point of removing an
/// override.
pub struct RemoveAttachment {
    skin: SkinId,
    slot: SlotId,
    name: String,
    removed: Option<Attachment>,
}

impl RemoveAttachment {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            removed: None,
        }
    }
}

impl EditCommand for RemoveAttachment {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(skin) = doc.skeleton.skins.get_mut(self.skin) {
            self.removed = skin.remove(self.slot, &self.name);
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(attachment) = self.removed.take()
            && let Some(skin) = doc.skeleton.skins.get_mut(self.skin)
        {
            skin.set(self.slot, self.name.clone(), attachment);
        }
    }

    fn label(&self) -> &str {
        "Remove Attachment"
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
    use ankhimate_core::animation::{Animation, Key};
    use ankhimate_core::attachment::Rect;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    /// A document with one bone, one slot, and one region attachment "arm".
    fn doc_with_attachment() -> (Document, SkinId, SlotId) {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = doc.skeleton.add_slot(Slot {
            attachment: Some("arm".into()),
            ..Slot::new("arm_slot".to_string(), bone)
        });
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(
            slot,
            "arm",
            Attachment::Region(RegionAttachment {
                texture: "arm".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 64.0,
                height: 32.0,
                sequence: None,
                uv_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                pivot: glam::Vec2::splat(0.5),
            }),
        );
        (doc, skin, slot)
    }

    fn region_of(doc: &Document, skin: SkinId, slot: SlotId, name: &str) -> RegionAttachment {
        match doc.skeleton.skins[skin].get(slot, name) {
            Some(Attachment::Region(r)) => r.clone(),
            other => panic!("expected a region, got {other:?}"),
        }
    }

    /// T-307 acceptance: moving an attachment moves only the art — the bone and
    /// its pose are untouched.
    #[test]
    fn editing_props_leaves_the_bone_alone() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let bone = doc.skeleton.slots[slot].bone;
        let bone_before = doc.skeleton.bones[bone].local_transform;
        let mut history = History::default();

        let mut props = RegionProps::from_region(&region_of(&doc, skin, slot, "arm"));
        props.offset = glam::vec2(12.0, -5.0);
        props.rotation = 0.25;
        history.push(
            Box::new(SetRegionProps::new(skin, slot, "arm", props)),
            &mut doc,
        );

        let region = region_of(&doc, skin, slot, "arm");
        assert_eq!(region.local_offset, glam::vec2(12.0, -5.0));
        assert_eq!(
            doc.skeleton.bones[bone].local_transform, bone_before,
            "the rig must not move when the art does"
        );

        history.undo(&mut doc);
        assert_eq!(
            region_of(&doc, skin, slot, "arm").local_offset,
            glam::Vec2::ZERO
        );
    }

    /// Re-pivoting must not move the artwork: the offset compensates, so the
    /// four corners land exactly where they were.
    #[test]
    fn changing_the_pivot_keeps_the_art_in_place() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let before = region_of(&doc, skin, slot, "arm").local_corners();

        let props = RegionProps::from_region(&region_of(&doc, skin, slot, "arm"));
        let moved = props.with_pivot_keeping_position(glam::vec2(0.0, 0.0));
        let mut history = History::default();
        history.push(
            Box::new(SetRegionProps::new(skin, slot, "arm", moved)),
            &mut doc,
        );

        let region = region_of(&doc, skin, slot, "arm");
        assert_eq!(region.pivot, glam::Vec2::ZERO, "pivot moved");
        for (a, b) in before.iter().zip(region.local_corners().iter()) {
            assert!((*a - *b).length() < 1e-3, "corner moved: {a:?} vs {b:?}");
        }
    }

    /// With the pivot moved, rotation turns about the new point — the reason to
    /// have pivots at all.
    #[test]
    fn rotation_follows_the_pivot() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let props = RegionProps::from_region(&region_of(&doc, skin, slot, "arm"));
        // Pivot at the left edge, then a quarter turn.
        let repivoted = props.with_pivot_keeping_position(glam::vec2(0.0, 0.5));
        let pivot_world = repivoted.offset;
        let turned = RegionProps {
            rotation: std::f32::consts::FRAC_PI_2,
            ..repivoted
        };
        let mut history = History::default();
        history.push(
            Box::new(SetRegionProps::new(skin, slot, "arm", turned)),
            &mut doc,
        );

        let corners = region_of(&doc, skin, slot, "arm").local_corners();
        // The left edge's midpoint is the pivot, and it has not moved.
        let left_mid = (corners[0] + corners[1]) * 0.5;
        assert!(
            (left_mid - pivot_world).length() < 1e-3,
            "pivot stayed put: {left_mid:?} vs {pivot_world:?}"
        );
    }

    #[test]
    fn prop_edits_merge_into_one_undo_step() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();
        let base = RegionProps::from_region(&region_of(&doc, skin, slot, "arm"));

        for x in 1..=5 {
            let props = RegionProps {
                offset: glam::vec2(x as f32, 0.0),
                ..base
            };
            history.push(
                Box::new(SetRegionProps::new(skin, slot, "arm", props)),
                &mut doc,
            );
        }
        assert_eq!(history.undo_depth(), 1, "a drag is one step");
        assert_eq!(region_of(&doc, skin, slot, "arm").local_offset.x, 5.0);

        history.undo(&mut doc);
        assert_eq!(
            region_of(&doc, skin, slot, "arm").local_offset.x,
            0.0,
            "undo returns to the pre-drag value, not the previous frame"
        );
    }

    /// Renaming an attachment must not touch the image it points at (T-901).
    ///
    /// The reference tool has three separate bugs filed here — a find-and-replace
    /// rename that rewrote an attachment's *path* as well as its name, leaving
    /// the art missing. Our model keeps the two in different fields, so this
    /// pins that they stay independent rather than trusting that they do.
    #[test]
    fn renaming_an_attachment_leaves_its_texture_alone() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let texture_before = match doc.skeleton.skins[skin].get(slot, "arm") {
            Some(Attachment::Region(r)) => r.texture.clone(),
            other => panic!("expected a region, got {other:?}"),
        };

        let mut history = History::default();
        history.push(
            Box::new(RenameAttachment::new(skin, slot, "arm", "forearm")),
            &mut doc,
        );

        match doc.skeleton.skins[skin].get(slot, "forearm") {
            Some(Attachment::Region(r)) => assert_eq!(
                r.texture, texture_before,
                "the name moved, the image reference must not"
            ),
            other => panic!("expected the renamed region, got {other:?}"),
        }
    }

    /// Renaming rewrites the slot's setup name and every attachment key that
    /// referenced it — otherwise the slot blanks and swap animations break.
    #[test]
    fn rename_rewrites_slot_and_animation_keys() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut anim = Animation::new("blink", 1.0);
        anim.timelines.push(Timeline::SlotAttachment {
            slot,
            keys: vec![
                Key::stepped(0.0, Some("arm".to_string())),
                Key::stepped(0.5, None),
            ],
        });
        doc.animations.insert(anim);

        let mut history = History::default();
        history.push(
            Box::new(RenameAttachment::new(skin, slot, "arm", "forearm")),
            &mut doc,
        );

        assert!(doc.skeleton.skins[skin].get(slot, "forearm").is_some());
        assert!(doc.skeleton.skins[skin].get(slot, "arm").is_none());
        assert_eq!(
            doc.skeleton.slots[slot].attachment.as_deref(),
            Some("forearm")
        );
        let (_, anim) = doc.animations.iter().next().unwrap();
        match &anim.timelines[0] {
            Timeline::SlotAttachment { keys, .. } => {
                assert_eq!(keys[0].value.as_deref(), Some("forearm"));
                assert_eq!(keys[1].value, None, "unrelated keys untouched");
            }
            other => panic!("expected an attachment timeline, got {other:?}"),
        }

        history.undo(&mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "arm").is_some());
        assert_eq!(doc.skeleton.slots[slot].attachment.as_deref(), Some("arm"));
        let (_, anim) = doc.animations.iter().next().unwrap();
        match &anim.timelines[0] {
            Timeline::SlotAttachment { keys, .. } => {
                assert_eq!(keys[0].value.as_deref(), Some("arm"))
            }
            other => panic!("expected an attachment timeline, got {other:?}"),
        }
    }

    #[test]
    fn rename_onto_an_existing_name_is_refused() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let existing = region_of(&doc, skin, slot, "arm");
        doc.skeleton.skins[skin].set(slot, "other", Attachment::Region(existing));

        let mut history = History::default();
        history.push(
            Box::new(RenameAttachment::new(skin, slot, "arm", "other")),
            &mut doc,
        );
        assert!(
            doc.skeleton.skins[skin].get(slot, "arm").is_some(),
            "collision refused, nothing clobbered"
        );
    }

    #[test]
    fn duplicate_then_undo() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();
        history.push(
            Box::new(DuplicateAttachment::new(skin, slot, "arm")),
            &mut doc,
        );
        assert!(doc.skeleton.skins[skin].get(slot, "arm_2").is_some());
        assert_eq!(doc.skeleton.skins[skin].names_for_slot(slot).count(), 2);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.skins[skin].names_for_slot(slot).count(), 1);
    }

    #[test]
    fn remove_and_restore() {
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();
        history.push(Box::new(RemoveAttachment::new(skin, slot, "arm")), &mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "arm").is_none());
        // The slot still names it — resolution falls through to the default skin.
        assert_eq!(doc.skeleton.slots[slot].attachment.as_deref(), Some("arm"));

        history.undo(&mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "arm").is_some());
    }

    #[test]
    fn owning_skin_prefers_the_active_then_falls_back() {
        let (mut doc, default, slot) = doc_with_attachment();
        // A second skin that does not define "arm" resolves to the default.
        let other = doc
            .skeleton
            .skins
            .insert(ankhimate_core::skin::Skin::new("other"));
        assert_eq!(owning_skin(&doc, other, slot, "arm"), Some(default));

        // Once it defines its own, the active skin wins.
        let region = region_of(&doc, default, slot, "arm");
        doc.skeleton.skins[other].set(slot, "arm", Attachment::Region(region));
        assert_eq!(owning_skin(&doc, other, slot, "arm"), Some(other));
    }

    fn a_sequence() -> Sequence {
        Sequence {
            frames: vec!["fire_01".into(), "fire_02".into(), "fire_03".into()],
            fps: 12.0,
            mode: ankhimate_core::attachment::SequenceMode::Loop,
            setup_index: 0,
        }
    }

    fn sequence_of(doc: &Document, skin: SkinId, slot: SlotId) -> Option<Sequence> {
        match doc.skeleton.skins[skin].get(slot, "arm")? {
            Attachment::Region(r) => r.sequence.clone(),
            _ => None,
        }
    }

    #[test]
    fn setting_a_sequence_is_undoable() {
        // The property every document edit has to have, checked here because a
        // sequence is the first thing the PSD importer produces that had no way
        // to be edited at all.
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();

        history.push(
            Box::new(SetSequence::new(skin, slot, "arm", Some(a_sequence()))),
            &mut doc,
        );
        assert_eq!(
            sequence_of(&doc, skin, slot).map(|s| s.frames.len()),
            Some(3)
        );

        history.undo(&mut doc);
        assert!(
            sequence_of(&doc, skin, slot).is_none(),
            "undo returns the attachment to having no sequence at all"
        );

        history.redo(&mut doc);
        assert_eq!(sequence_of(&doc, skin, slot).map(|s| s.fps), Some(12.0));
    }

    #[test]
    fn clearing_a_sequence_keeps_the_attachment() {
        // "Split into separate slots" clears the fold. The attachment and its
        // texture must survive it — the images are the art, the sequence is only
        // a statement about how they play.
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();
        history.push(
            Box::new(SetSequence::new(skin, slot, "arm", Some(a_sequence()))),
            &mut doc,
        );
        history.push(
            Box::new(SetSequence::new(skin, slot, "arm", None)),
            &mut doc,
        );

        assert!(sequence_of(&doc, skin, slot).is_none());
        assert!(
            doc.skeleton.skins[skin].get(slot, "arm").is_some(),
            "the attachment itself is still there"
        );
    }

    #[test]
    fn dragging_the_rate_is_one_undo_step() {
        // The same rule every other inspector number follows: a drag is one
        // step, not sixty. Without the merge, undo after adjusting fps walks
        // back through every intermediate value.
        let (mut doc, skin, slot) = doc_with_attachment();
        let mut history = History::default();
        history.push(
            Box::new(SetSequence::new(skin, slot, "arm", Some(a_sequence()))),
            &mut doc,
        );

        for fps in [13.0, 14.0, 15.0] {
            let mut next = a_sequence();
            next.fps = fps;
            history.push(
                Box::new(SetSequence::new(skin, slot, "arm", Some(next))),
                &mut doc,
            );
        }
        assert_eq!(sequence_of(&doc, skin, slot).map(|s| s.fps), Some(15.0));

        history.undo(&mut doc);
        assert_eq!(
            sequence_of(&doc, skin, slot).map(|s| s.fps),
            Some(12.0),
            "one undo walks back the whole drag, not one frame of it"
        );
    }
}
