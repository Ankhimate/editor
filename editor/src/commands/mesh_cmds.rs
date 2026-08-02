//! Mesh attachment editing as undoable commands (T-401).
//!
//! Mesh topology is rig data, so everything here is Setup-only (T-207).
//! Animating the same geometry is what `Deform` timelines are for (T-404).
//!
//! Each command snapshots the whole attachment rather than inverting its edit.
//! A mesh is a few dozen vertices, the snapshot is a `clone`, and the
//! alternative — hand-written inverses for vertex insertion with the index
//! rewrites it implies — is where the bugs would live.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::attachment::{Attachment, MeshAttachment};
use ankhimate_core::ids::{SkinId, SlotId};

/// Fetch the attachment an edit targets.
fn attachment_mut<'a>(
    doc: &'a mut Document,
    skin: SkinId,
    slot: SlotId,
    name: &str,
) -> Option<&'a mut Attachment> {
    doc.skeleton
        .skins
        .get_mut(skin)?
        .entries
        .get_mut(&(slot, name.to_string()))
}

/// Swap a region attachment for an equivalent mesh, or back again on undo.
pub struct ConvertToMesh {
    skin: SkinId,
    slot: SlotId,
    name: String,
    before: Option<Attachment>,
}

impl ConvertToMesh {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            before: None,
        }
    }
}

impl EditCommand for ConvertToMesh {
    fn apply(&mut self, doc: &mut Document) {
        let Some(attachment) = attachment_mut(doc, self.skin, self.slot, &self.name) else {
            return;
        };
        let Attachment::Region(region) = attachment else {
            return; // Already a mesh.
        };
        let mesh = MeshAttachment::from_region(region);
        self.before = Some(attachment.clone());
        *attachment = Attachment::Mesh(mesh);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(attachment)) = (
            self.before.take(),
            attachment_mut(doc, self.skin, self.slot, &self.name),
        ) {
            *attachment = before;
        }
    }

    fn label(&self) -> &str {
        "Convert To Mesh"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Replace a mesh's geometry with a traced silhouette (T-402).
///
/// Separate from [`EditMesh`] because it throws away topology rather than
/// nudging it: weights and deform keys are keyed to vertex indices, and after a
/// trace those indices mean something else entirely. The caller warns; this
/// command clears them rather than leaving them pointing at the wrong points.
pub struct TraceMesh {
    skin: SkinId,
    slot: SlotId,
    name: String,
    vertices: Vec<glam::Vec2>,
    uvs: Vec<glam::Vec2>,
    triangles: Vec<[u32; 3]>,
    before: Option<MeshAttachment>,
}

impl TraceMesh {
    pub fn new(
        skin: SkinId,
        slot: SlotId,
        name: impl Into<String>,
        vertices: Vec<glam::Vec2>,
        uvs: Vec<glam::Vec2>,
        triangles: Vec<[u32; 3]>,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            vertices,
            uvs,
            triangles,
            before: None,
        }
    }
}

impl EditCommand for TraceMesh {
    fn apply(&mut self, doc: &mut Document) {
        let Some(Attachment::Mesh(mesh)) = attachment_mut(doc, self.skin, self.slot, &self.name)
        else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(mesh.clone());
        }
        mesh.setup_vertices = self.vertices.clone();
        mesh.uvs = self.uvs.clone();
        mesh.triangles = self.triangles.clone();
        // Index-keyed data cannot survive a topology change.
        mesh.weights.clear();
        mesh.inverse_bind_matrices.clear();
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(Attachment::Mesh(mesh))) = (
            self.before.take(),
            attachment_mut(doc, self.skin, self.slot, &self.name),
        ) {
            *mesh = before;
        }
    }

    fn label(&self) -> &str {
        "Trace Mesh"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What an edit does to a mesh, so one command type covers every operation.
pub enum MeshEdit {
    /// Move vertices to new local positions. Merges, so a drag is one step.
    MoveVertices(Vec<(usize, glam::Vec2)>),
    /// Insert a vertex at a local position, re-triangulating around it.
    AddVertex(glam::Vec2),
    /// Remove vertices by index, highest first so earlier indices stay valid.
    RemoveVertices(Vec<usize>),
    /// Re-run triangulation over the current vertices.
    Retriangulate,
}

/// Apply a [`MeshEdit`] to one mesh attachment.
pub struct EditMesh {
    skin: SkinId,
    slot: SlotId,
    name: String,
    edit: MeshEdit,
    before: Option<MeshAttachment>,
    label: &'static str,
}

impl EditMesh {
    pub fn new(skin: SkinId, slot: SlotId, name: impl Into<String>, edit: MeshEdit) -> Self {
        let label = match &edit {
            MeshEdit::MoveVertices(_) => "Move Vertices",
            MeshEdit::AddVertex(_) => "Add Vertex",
            MeshEdit::RemoveVertices(_) => "Delete Vertices",
            MeshEdit::Retriangulate => "Retriangulate",
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

    fn mesh<'a>(&self, doc: &'a mut Document) -> Option<&'a mut MeshAttachment> {
        match attachment_mut(doc, self.skin, self.slot, &self.name)? {
            Attachment::Mesh(mesh) => Some(mesh),
            Attachment::Region(_) | Attachment::Clipping(_) => None,
        }
    }
}

impl EditCommand for EditMesh {
    fn apply(&mut self, doc: &mut Document) {
        // Take the snapshot before touching anything, and only once — a merged
        // drag must still undo to where it started.
        let capture = self.before.is_none();
        let Some(mesh) = self.mesh(doc) else {
            return;
        };
        if capture {
            self.before = Some(mesh.clone());
        }

        match &self.edit {
            MeshEdit::MoveVertices(moves) => {
                for (index, position) in moves {
                    if let Some(vertex) = mesh.setup_vertices.get_mut(*index) {
                        *vertex = *position;
                    }
                }
            }
            MeshEdit::AddVertex(position) => {
                let uv = mesh.uv_for_local(*position);
                mesh.setup_vertices.push(*position);
                mesh.uvs.push(uv);
                // A new vertex has no influences yet; leaving `weights` short
                // would desynchronise it from `setup_vertices`, which the
                // skinning code indexes in lockstep.
                if !mesh.weights.is_empty() {
                    mesh.weights.push(Vec::new());
                }
                crate::meshgen::retriangulate(mesh);
            }
            MeshEdit::RemoveVertices(indices) => {
                // A mesh needs three points to be a surface; below that there is
                // nothing to draw and nothing to undo to that makes sense.
                if mesh.setup_vertices.len().saturating_sub(indices.len()) < 3 {
                    self.before = None;
                    return;
                }
                let mut sorted = indices.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                sorted.dedup();
                for index in sorted {
                    if index < mesh.setup_vertices.len() {
                        mesh.setup_vertices.remove(index);
                    }
                    if index < mesh.uvs.len() {
                        mesh.uvs.remove(index);
                    }
                    if index < mesh.weights.len() {
                        mesh.weights.remove(index);
                    }
                }
                crate::meshgen::retriangulate(mesh);
            }
            MeshEdit::Retriangulate => crate::meshgen::retriangulate(mesh),
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(mesh)) = (self.before.take(), self.mesh(doc)) {
            *mesh = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<EditMesh>() else {
            return false;
        };
        // Only vertex drags merge: inserting and deleting are discrete edits a
        // user expects to undo one at a time.
        match (&mut self.edit, &other.edit) {
            (MeshEdit::MoveVertices(ours), MeshEdit::MoveVertices(theirs))
                if other.skin == self.skin
                    && other.slot == self.slot
                    && other.name == self.name =>
            {
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
mod tests {
    use super::*;
    use crate::commands::History;
    use ankhimate_core::attachment::{Rect, RegionAttachment};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_core::slot::Slot;

    fn doc_with_region() -> (Document, SkinId, SlotId) {
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
            attachment: Some("art".into()),
            ..Slot::new("art_slot".to_string(), bone)
        });
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(
            slot,
            "art",
            Attachment::Region(RegionAttachment {
                texture: "art".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 100.0,
                height: 100.0,
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

    fn mesh_of(doc: &Document, skin: SkinId, slot: SlotId) -> MeshAttachment {
        match doc.skeleton.skins[skin].get(slot, "art") {
            Some(Attachment::Mesh(mesh)) => mesh.clone(),
            other => panic!("expected a mesh, got {other:?}"),
        }
    }

    /// T-401 acceptance: a quad becomes a mesh, gains a vertex, and round-trips
    /// back to a region on undo.
    #[test]
    fn convert_add_vertex_and_undo_back_to_a_region() {
        let (mut doc, skin, slot) = doc_with_region();
        let mut history = History::default();

        history.push(Box::new(ConvertToMesh::new(skin, slot, "art")), &mut doc);
        let mesh = mesh_of(&doc, skin, slot);
        assert_eq!(mesh.setup_vertices.len(), 4);
        assert_eq!(mesh.triangles.len(), 2);

        history.push(
            Box::new(EditMesh::new(
                skin,
                slot,
                "art",
                MeshEdit::AddVertex(glam::vec2(0.0, 0.0)),
            )),
            &mut doc,
        );
        let mesh = mesh_of(&doc, skin, slot);
        assert_eq!(mesh.setup_vertices.len(), 5, "a centre vertex was added");
        assert_eq!(mesh.uvs.len(), 5, "and it got a UV");
        assert_eq!(
            mesh.triangles.len(),
            4,
            "the quad re-triangulated around it: {:?}",
            mesh.triangles
        );

        history.undo(&mut doc);
        assert_eq!(mesh_of(&doc, skin, slot).setup_vertices.len(), 4);

        history.undo(&mut doc);
        assert!(
            matches!(
                doc.skeleton.skins[skin].get(slot, "art"),
                Some(Attachment::Region(_))
            ),
            "undoing the conversion restores the region"
        );
    }

    #[test]
    fn dragging_vertices_merges_into_one_undo_step() {
        let (mut doc, skin, slot) = doc_with_region();
        let mut history = History::default();
        history.push(Box::new(ConvertToMesh::new(skin, slot, "art")), &mut doc);
        let before = mesh_of(&doc, skin, slot).setup_vertices[0];
        let depth = history.undo_depth();

        for x in 1..=5 {
            history.push(
                Box::new(EditMesh::new(
                    skin,
                    slot,
                    "art",
                    MeshEdit::MoveVertices(vec![(0, glam::vec2(x as f32, 0.0))]),
                )),
                &mut doc,
            );
        }
        assert_eq!(history.undo_depth(), depth + 1, "one step for the drag");
        assert_eq!(mesh_of(&doc, skin, slot).setup_vertices[0].x, 5.0);

        history.undo(&mut doc);
        assert_eq!(
            mesh_of(&doc, skin, slot).setup_vertices[0],
            before,
            "undo returns to the pre-drag position, not the previous frame"
        );
    }

    /// A mesh with fewer than three vertices is not a surface; the delete is
    /// refused rather than leaving something undrawable behind.
    #[test]
    fn deleting_below_three_vertices_is_refused() {
        let (mut doc, skin, slot) = doc_with_region();
        let mut history = History::default();
        history.push(Box::new(ConvertToMesh::new(skin, slot, "art")), &mut doc);

        history.push(
            Box::new(EditMesh::new(
                skin,
                slot,
                "art",
                MeshEdit::RemoveVertices(vec![0, 1]),
            )),
            &mut doc,
        );
        assert_eq!(
            mesh_of(&doc, skin, slot).setup_vertices.len(),
            4,
            "refused: 4 - 2 would leave 2"
        );

        history.push(
            Box::new(EditMesh::new(
                skin,
                slot,
                "art",
                MeshEdit::RemoveVertices(vec![3]),
            )),
            &mut doc,
        );
        assert_eq!(mesh_of(&doc, skin, slot).setup_vertices.len(), 3, "allowed");
    }

    #[test]
    fn removing_a_vertex_keeps_uvs_and_weights_in_step() {
        let (mut doc, skin, slot) = doc_with_region();
        let mut history = History::default();
        history.push(Box::new(ConvertToMesh::new(skin, slot, "art")), &mut doc);

        // Give the mesh weights so the parallel-array invariant is exercised.
        if let Some(Attachment::Mesh(mesh)) = doc.skeleton.skins[skin]
            .entries
            .get_mut(&(slot, "art".to_string()))
        {
            mesh.weights = vec![Vec::new(); mesh.setup_vertices.len()];
        }

        history.push(
            Box::new(EditMesh::new(
                skin,
                slot,
                "art",
                MeshEdit::RemoveVertices(vec![1]),
            )),
            &mut doc,
        );
        let mesh = mesh_of(&doc, skin, slot);
        assert_eq!(mesh.setup_vertices.len(), 3);
        assert_eq!(mesh.uvs.len(), 3);
        assert_eq!(mesh.weights.len(), 3, "weights track the vertex list");
    }
}
