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
use ankhimate_core::attachment::{
    Attachment, BoundingBoxAttachment, ClippingAttachment, PointAttachment,
};
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
#[derive(Clone)]
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

/// Apply a [`ClipEdit`] to one bounding box.
///
/// Same gestures, same undo shape, different attachment — a hitbox is a polygon
/// on a slot exactly as a clip is, and giving it its own vertex editor would mean
/// two implementations of "drag a vertex" to keep in step.
pub struct EditBoundingBox {
    skin: SkinId,
    slot: SlotId,
    name: String,
    edit: ClipEdit,
    before: Option<BoundingBoxAttachment>,
    label: &'static str,
}

impl EditBoundingBox {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, edit: ClipEdit) -> Self {
        let label = match &edit {
            ClipEdit::MoveVertices(_) => "Move Box Vertices",
            ClipEdit::InsertVertex(_, _) => "Add Box Vertex",
            ClipEdit::RemoveVertices(_) => "Delete Box Vertices",
            ClipEdit::SetEndSlot(_) => "Set Box Range",
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

    fn box_mut<'a>(&self, doc: &'a mut Document) -> Option<&'a mut BoundingBoxAttachment> {
        match doc
            .skeleton
            .skins
            .get_mut(self.skin)?
            .entries
            .get_mut(&(self.slot, self.name.clone()))?
        {
            Attachment::BoundingBox(b) => Some(b),
            _ => None,
        }
    }
}

impl EditCommand for EditBoundingBox {
    fn apply(&mut self, doc: &mut Document) {
        let capture = self.before.is_none();
        let edit = self.edit.clone();
        let Some(bb) = self.box_mut(doc) else {
            return;
        };
        if capture {
            self.before = Some(bb.clone());
        }
        match edit {
            ClipEdit::MoveVertices(moves) => {
                for (index, position) in moves {
                    if let Some(vertex) = bb.vertices.get_mut(index) {
                        *vertex = position;
                    }
                }
            }
            ClipEdit::InsertVertex(index, position) => {
                let at = index.min(bb.vertices.len());
                bb.vertices.insert(at, position);
                // A skinned box carries one weight list per vertex; inserting
                // without one would shift every later vertex onto the wrong
                // bones. Copy the neighbour it was split from.
                if !bb.weights.is_empty() {
                    let inherited = bb
                        .weights
                        .get(at.saturating_sub(1))
                        .cloned()
                        .unwrap_or_default();
                    bb.weights.insert(at.min(bb.weights.len()), inherited);
                }
            }
            ClipEdit::RemoveVertices(indices) => {
                // Below three points there is no interior, and a hitbox with no
                // interior can never be hit — a silently dead hitbox is the worst
                // outcome here, so refuse instead.
                if bb.vertices.len().saturating_sub(indices.len()) < 3 {
                    self.before = None;
                    return;
                }
                let mut sorted = indices;
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                sorted.dedup();
                for index in sorted {
                    if index < bb.vertices.len() {
                        bb.vertices.remove(index);
                    }
                    if index < bb.weights.len() {
                        bb.weights.remove(index);
                    }
                }
            }
            // A box masks nothing, so it has no range to set.
            ClipEdit::SetEndSlot(_) => self.before = None,
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.take() else {
            return;
        };
        if let Some(bb) = self.box_mut(doc) {
            *bb = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditBoundingBox>() else {
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

/// Add a path attachment to a slot, with a starting line (T-502).
///
/// Add a bounding-box attachment to a slot.
///
/// Starts as a square around the bone origin for the same reason a clip does: an
/// attachment with no vertices has nothing to grab, so "add" would leave the user
/// staring at an unchanged viewport wondering whether it worked.
pub struct AddBoundingBox {
    skin: SkinId,
    slot: SlotId,
    name: String,
    size: f32,
    added: bool,
}

impl AddBoundingBox {
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

impl EditCommand for AddBoundingBox {
    fn apply(&mut self, doc: &mut Document) {
        let Some(skin) = doc.skeleton.skins.get_mut(self.skin) else {
            return;
        };
        if skin.get(self.slot, &self.name).is_some() {
            return; // Name already taken in this skin.
        }
        let half = self.size * 0.5;
        skin.set(
            self.slot,
            self.name.clone(),
            Attachment::BoundingBox(BoundingBoxAttachment {
                vertices: vec![
                    glam::vec2(-half, -half),
                    glam::vec2(half, -half),
                    glam::vec2(half, half),
                    glam::vec2(-half, half),
                ],
                weights: Vec::new(),
            }),
        );
        self.added = true;
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
        "Add Bounding Box"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Add a point attachment to a slot.
pub struct AddPoint {
    skin: SkinId,
    slot: SlotId,
    name: String,
    added: bool,
}

impl AddPoint {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            added: false,
        }
    }
}

impl EditCommand for AddPoint {
    fn apply(&mut self, doc: &mut Document) {
        let Some(skin) = doc.skeleton.skins.get_mut(self.skin) else {
            return;
        };
        if skin.get(self.slot, &self.name).is_some() {
            return;
        }
        skin.set(
            self.slot,
            self.name.clone(),
            Attachment::Point(PointAttachment::default()),
        );
        self.added = true;
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
        "Add Point"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Move or turn a point attachment. Merges, so a drag is one undo step.
pub struct SetPoint {
    skin: SkinId,
    slot: SlotId,
    name: String,
    after: PointAttachment,
    before: Option<PointAttachment>,
}

impl SetPoint {
    pub fn new(
        skin: SkinId,
        slot: SlotId,
        name: impl Into<String>,
        after: PointAttachment,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            after,
            before: None,
        }
    }

    fn point<'a>(&self, doc: &'a mut Document) -> Option<&'a mut PointAttachment> {
        match doc
            .skeleton
            .skins
            .get_mut(self.skin)?
            .entries
            .get_mut(&(self.slot, self.name.clone()))?
        {
            Attachment::Point(p) => Some(p),
            _ => None,
        }
    }
}

impl EditCommand for SetPoint {
    fn apply(&mut self, doc: &mut Document) {
        let after = self.after.clone();
        let capture = self.before.is_none();
        if let Some(point) = self.point(doc) {
            if capture {
                self.before = Some(point.clone());
            }
            *point = after;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.clone() else {
            return;
        };
        if let Some(point) = self.point(doc) {
            *point = before;
        }
    }

    fn label(&self) -> &str {
        "Move Point"
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(next) = next.as_any().downcast_ref::<SetPoint>() else {
            return false;
        };
        if next.skin != self.skin || next.slot != self.slot || next.name != self.name {
            return false;
        }
        self.after = next.after.clone();
        true
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Like [`AddClipping`], it starts as real geometry rather than empty: a path
/// with no vertices drives nothing, which would read as the command failing.
pub struct AddPath {
    skin: SkinId,
    slot: SlotId,
    name: String,
    size: f32,
    added: bool,
}

impl AddPath {
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

impl EditCommand for AddPath {
    fn apply(&mut self, doc: &mut Document) {
        let Some(skin) = doc.skeleton.skins.get_mut(self.skin) else {
            return;
        };
        if skin.get(self.slot, &self.name).is_some() {
            return;
        }
        let half = self.size * 0.5;
        skin.set(
            self.slot,
            self.name.clone(),
            Attachment::Path(ankhimate_core::attachment::PathAttachment {
                vertices: vec![
                    glam::vec2(-half, 0.0),
                    glam::vec2(0.0, half * 0.5),
                    glam::vec2(half, 0.0),
                ],
                closed: false,
                constant_speed: true,
            }),
        );
        self.added = true;
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
        "Add Path"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Move a path attachment's vertices, sharing the clip tools' shape.
pub struct EditPath {
    skin: SkinId,
    slot: SlotId,
    name: String,
    edit: ClipEdit,
    before: Option<ankhimate_core::attachment::PathAttachment>,
    label: &'static str,
}

impl EditPath {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, edit: ClipEdit) -> Self {
        let label = match &edit {
            ClipEdit::MoveVertices(_) => "Move Path Vertices",
            ClipEdit::InsertVertex(_, _) => "Add Path Vertex",
            ClipEdit::RemoveVertices(_) => "Delete Path Vertices",
            ClipEdit::SetEndSlot(_) => "Set Path",
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

    fn path<'a>(
        &self,
        doc: &'a mut Document,
    ) -> Option<&'a mut ankhimate_core::attachment::PathAttachment> {
        match doc
            .skeleton
            .skins
            .get_mut(self.skin)?
            .entries
            .get_mut(&(self.slot, self.name.clone()))?
        {
            Attachment::Path(path) => Some(path),
            _ => None,
        }
    }
}

impl EditCommand for EditPath {
    fn apply(&mut self, doc: &mut Document) {
        let capture = self.before.is_none();
        let Some(path) = self.path(doc) else {
            return;
        };
        if capture {
            self.before = Some(path.clone());
        }
        match &self.edit {
            ClipEdit::MoveVertices(moves) => {
                for (index, position) in moves {
                    if let Some(v) = path.vertices.get_mut(*index) {
                        *v = *position;
                    }
                }
            }
            ClipEdit::InsertVertex(index, position) => {
                let at = (*index).min(path.vertices.len());
                path.vertices.insert(at, *position);
            }
            ClipEdit::RemoveVertices(indices) => {
                // Two points is still a path — a straight line drives a chain
                // perfectly well — so the floor is lower than a clip's three.
                if path.vertices.len().saturating_sub(indices.len()) < 2 {
                    self.before = None;
                    return;
                }
                let mut sorted = indices.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                sorted.dedup();
                for index in sorted {
                    if index < path.vertices.len() {
                        path.vertices.remove(index);
                    }
                }
            }
            ClipEdit::SetEndSlot(_) => {}
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(path)) = (self.before.take(), self.path(doc)) {
            *path = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditPath>() else {
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
mod tests_support {
    use super::*;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    pub fn doc_with_slot() -> (Document, SkinId, SlotId) {
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
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;
    use crate::commands::History;

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

#[cfg(test)]
mod marker_tests {
    use super::tests_support::*;
    use super::*;
    use crate::commands::History;
    use ankhimate_core::attachment::VertexWeight;

    #[test]
    fn adding_a_box_gives_it_a_polygon_and_points_the_slot_at_it() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(
            Box::new(AddBoundingBox::new(skin, slot, "hurtbox", 100.0)),
            &mut doc,
        );

        assert!(matches!(
            doc.skeleton.skins[skin].get(slot, "hurtbox"),
            Some(Attachment::BoundingBox(b)) if b.vertices.len() == 4
        ));
        assert_eq!(
            doc.skeleton.slots[slot].attachment.as_deref(),
            Some("hurtbox")
        );

        history.undo(&mut doc);
        assert!(doc.skeleton.skins[skin].get(slot, "hurtbox").is_none());
        assert!(doc.skeleton.slots[slot].attachment.is_none());
    }

    /// A skinned box keeps one weight list per vertex. Insert and delete have to
    /// move both arrays or every later vertex silently changes which bones it
    /// follows — a hitbox that drifts one limb over is worse than one that never
    /// worked.
    #[test]
    fn box_vertex_edits_keep_the_weight_table_aligned() {
        let (mut doc, skin, slot) = doc_with_slot();
        let bone = doc.skeleton.bones.keys().next().unwrap();
        doc.skeleton.skins[skin].set(
            slot,
            "hurtbox".to_string(),
            Attachment::BoundingBox(BoundingBoxAttachment {
                vertices: vec![
                    glam::vec2(0.0, 0.0),
                    glam::vec2(10.0, 0.0),
                    glam::vec2(10.0, 10.0),
                    glam::vec2(0.0, 10.0),
                ],
                weights: (0..4)
                    .map(|_| vec![VertexWeight { bone, weight: 1.0 }])
                    .collect(),
            }),
        );
        let mut history = History::default();

        history.push(
            Box::new(EditBoundingBox::new(
                skin,
                slot,
                "hurtbox",
                ClipEdit::InsertVertex(2, glam::vec2(12.0, 5.0)),
            )),
            &mut doc,
        );
        let Some(Attachment::BoundingBox(b)) = doc.skeleton.skins[skin].get(slot, "hurtbox") else {
            panic!("box vanished");
        };
        assert_eq!(b.vertices.len(), 5);
        assert_eq!(b.weights.len(), 5, "a vertex without a weight list");

        history.push(
            Box::new(EditBoundingBox::new(
                skin,
                slot,
                "hurtbox",
                ClipEdit::RemoveVertices(vec![0, 2]),
            )),
            &mut doc,
        );
        let Some(Attachment::BoundingBox(b)) = doc.skeleton.skins[skin].get(slot, "hurtbox") else {
            panic!("box vanished");
        };
        assert_eq!((b.vertices.len(), b.weights.len()), (3, 3));
    }

    /// Three points is the floor: below it there is no interior, so the box could
    /// never be hit and the failure would be invisible.
    #[test]
    fn a_box_refuses_to_drop_below_a_triangle() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(
            Box::new(AddBoundingBox::new(skin, slot, "hurtbox", 100.0)),
            &mut doc,
        );
        history.push(
            Box::new(EditBoundingBox::new(
                skin,
                slot,
                "hurtbox",
                ClipEdit::RemoveVertices(vec![0, 1]),
            )),
            &mut doc,
        );
        let Some(Attachment::BoundingBox(b)) = doc.skeleton.skins[skin].get(slot, "hurtbox") else {
            panic!("box vanished");
        };
        assert_eq!(b.vertices.len(), 4, "the edit should have been refused");
    }

    #[test]
    fn moving_a_point_merges_into_one_undo_step() {
        let (mut doc, skin, slot) = doc_with_slot();
        let mut history = History::default();
        history.push(Box::new(AddPoint::new(skin, slot, "muzzle")), &mut doc);
        for x in 1..=3 {
            history.push(
                Box::new(SetPoint::new(
                    skin,
                    slot,
                    "muzzle",
                    PointAttachment {
                        position: glam::vec2(x as f32, 0.0),
                        rotation: 0.0,
                    },
                )),
                &mut doc,
            );
        }
        history.undo(&mut doc);
        let Some(Attachment::Point(p)) = doc.skeleton.skins[skin].get(slot, "muzzle") else {
            panic!("point vanished");
        };
        assert_eq!(
            p.position,
            glam::Vec2::ZERO,
            "one undo should return to where the drag started"
        );
    }
}
