//! Inverse bind matrices for weighted meshes.
//!
//! In the document crate, not the editor, because binds are `#[serde(skip)]`
//! derived state: a weighted mesh without them is broken, whether it was edited
//! by a panel or by a script. `docs/plugin-plan.md` calls this out as the one
//! part of the editor's `after_document_change` that is document integrity
//! rather than UI bookkeeping.

use crate::doc::Document;

/// Capture inverse bind matrices for any weighted mesh that lacks them.
///
/// Binds are `#[serde(skip)]` derived state — they come from the **setup**
/// pose, so they can always be recomputed and must be, after a load or any
/// change to a mesh's influences. Capturing at setup is also what stops a
/// newly painted bone from yanking the mesh: at the setup pose every bone's
/// `world × inverse_bind` is the identity, so adding an influence moves
/// nothing until the rig is actually posed.
pub fn rebind_meshes(doc: &mut Document) {
    use ankhimate_core::attachment::Attachment;

    let needs_binds = doc.skeleton.skins.iter().any(|(_, skin)| {
        skin.entries.values().any(|a| match a {
            Attachment::Mesh(mesh) => {
                !mesh.weights.is_empty() && mesh.inverse_bind_matrices.is_empty()
            }
            // A bounding box is skinned the same way a mesh is, and it binds
            // on the same pass — a hitbox that lags the art it guards is a
            // bug you only notice in playtest.
            Attachment::BoundingBox(b) => !b.weights.is_empty(),
            // The rest carry no weights, so they never need binds.
            Attachment::Region(_)
            | Attachment::Clipping(_)
            | Attachment::Path(_)
            | Attachment::Point(_) => false,
        })
    });
    if !needs_binds {
        return;
    }

    // The setup pose, not the current one: binds must not depend on where
    // the playhead happens to be.
    let mut setup = ankhimate_core::pose::Pose::new();
    ankhimate_core::pose::evaluate(&doc.skeleton, &[], &mut setup);

    // A mesh's vertices live in its slot's bone frame, so the bind needs
    // that bone's world affine — the entry key carries the slot.
    let slot_bones: std::collections::HashMap<_, _> = doc
        .skeleton
        .slots
        .iter()
        .map(|(id, slot)| (id, setup.world(slot.bone)))
        .collect();
    for (_, skin) in doc.skeleton.skins.iter_mut() {
        for ((slot, _), attachment) in skin.entries.iter_mut() {
            if let Attachment::Mesh(mesh) = attachment
                && !mesh.weights.is_empty()
                && mesh.inverse_bind_matrices.is_empty()
            {
                let space = slot_bones
                    .get(slot)
                    .copied()
                    .unwrap_or(ankhimate_core::transforms::Affine2::IDENTITY);
                mesh.bind_to_pose(&setup, space);
            }
        }
    }
}
