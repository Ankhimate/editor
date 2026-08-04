//! Fold a PSD import into the document as one undoable step (T-302).
//!
//! One command, not one per bone. An import that undoes in ninety steps is an
//! import nobody dares run twice, and "I imported the wrong file" is exactly the
//! mistake this has to make cheap.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::BoneId;
use ankhimate_formats::psd::PsdImport;
use std::collections::HashMap;

pub struct ImportPsd {
    imported: Option<PsdImport>,
    /// Replace the document outright, rather than merging under a new bone.
    replace: bool,
    name: String,
    /// The whole prior document, captured on first apply.
    ///
    /// Coarse on purpose. A merge touches bones, slots, skins, assets and the
    /// draw order at once, and reconstructing the inverse of all of that is far
    /// more code — and far more places to be subtly wrong — than keeping a copy
    /// of a structure that is a few hundred kilobytes at worst.
    before: Option<Box<Snapshot>>,
}

struct Snapshot {
    skeleton: ankhimate_core::skeleton::Skeleton,
    assets: ankhimate_core::assets::AssetDb,
    psd_layer_paths: HashMap<String, String>,
}

impl ImportPsd {
    pub fn new(imported: PsdImport, replace: bool, name: impl Into<String>) -> Self {
        Self {
            imported: Some(imported),
            replace,
            name: name.into(),
            before: None,
        }
    }
}

impl EditCommand for ImportPsd {
    fn apply(&mut self, doc: &mut Document) {
        let Some(imported) = self.imported.take() else {
            return; // Re-apply after undo is handled by `revert` restoring state.
        };
        if self.before.is_none() {
            self.before = Some(Box::new(Snapshot {
                skeleton: doc.skeleton.clone(),
                assets: doc.assets.clone(),
                psd_layer_paths: doc.psd_layer_paths.clone(),
            }));
        }

        if self.replace {
            doc.skeleton = imported.skeleton;
            doc.assets = imported.assets;
            doc.psd_layer_paths = imported.layer_paths;
            self.imported = None;
            return;
        }

        // Merge: the imported rig arrives under a bone of its own so it can be
        // moved as a unit, and so two imports cannot collide at the root.
        let anchor = doc.skeleton.add_bone(ankhimate_core::skeleton::Bone {
            name: ankhimate_core::skeleton::unique_name(
                &self.name,
                doc.skeleton.bones.iter().map(|(_, b)| b.name.as_str()),
            ),
            parent: None,
            length: 1.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: ankhimate_core::skeleton::Bone::default_color(),
        });

        let mut bone_map: HashMap<BoneId, BoneId> = HashMap::new();
        // Imported bones come out parent-before-child, so one pass maps them.
        for id in imported.skeleton.update_order.iter() {
            let Some(bone) = imported.skeleton.bones.get(*id) else {
                continue;
            };
            let parent = match bone.parent {
                Some(p) => bone_map.get(&p).copied().unwrap_or(anchor),
                None => anchor,
            };
            let new = doc.skeleton.add_bone(ankhimate_core::skeleton::Bone {
                name: ankhimate_core::skeleton::unique_name(
                    &bone.name,
                    doc.skeleton.bones.iter().map(|(_, b)| b.name.as_str()),
                ),
                parent: Some(parent),
                ..bone.clone()
            });
            bone_map.insert(*id, new);
        }

        for asset in imported.assets.images.values() {
            doc.assets.add(asset.clone());
        }

        let mut slot_map = HashMap::new();
        for slot_id in imported.skeleton.draw_order.iter() {
            let Some(slot) = imported.skeleton.slots.get(*slot_id) else {
                continue;
            };
            let Some(&bone) = bone_map.get(&slot.bone) else {
                continue;
            };
            let new = doc.skeleton.add_slot(ankhimate_core::slot::Slot {
                bone,
                name: ankhimate_core::skeleton::unique_name(
                    &slot.name,
                    doc.skeleton.slots.iter().map(|(_, s)| s.name.as_str()),
                ),
                ..slot.clone()
            });
            slot_map.insert(*slot_id, new);
        }

        let default_skin = doc.skeleton.default_skin;
        for (source_skin_id, source_skin) in imported.skeleton.skins.iter() {
            // An imported non-default skin keeps its name; its entries never
            // merge into the open document's default skin, or a `@skin:` group
            // would silently become base art.
            let target = if source_skin_id == imported.skeleton.default_skin {
                default_skin
            } else {
                match doc
                    .skeleton
                    .skins
                    .iter()
                    .find(|(_, s)| s.name == source_skin.name)
                    .map(|(id, _)| id)
                {
                    Some(id) => id,
                    None => doc
                        .skeleton
                        .add_skin(ankhimate_core::skin::Skin::new(source_skin.name.clone())),
                }
            };
            for ((slot, name), attachment) in source_skin.entries.iter() {
                let Some(&slot) = slot_map.get(slot) else {
                    continue;
                };
                let mut attachment = attachment.clone();
                if let Attachment::Mesh(mesh) = &mut attachment {
                    mesh.weights.iter_mut().flatten().for_each(|w| {
                        if let Some(&mapped) = bone_map.get(&w.bone) {
                            w.bone = mapped;
                        }
                    });
                }
                doc.skeleton.skins[target].set(slot, name.clone(), attachment);
            }
        }

        for (_, constraint) in imported.skeleton.constraints.iter() {
            let mut constraint = constraint.clone();
            remap_constraint(&mut constraint, &bone_map);
            doc.skeleton.add_constraint(constraint);
        }

        for (asset_name, layer_path) in imported.layer_paths {
            doc.psd_layer_paths.insert(asset_name, layer_path);
        }

        doc.skeleton.rebuild_update_order();
        self.imported = None;
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.take() else {
            return;
        };
        doc.skeleton = before.skeleton;
        doc.assets = before.assets;
        doc.psd_layer_paths = before.psd_layer_paths;
    }

    fn label(&self) -> &str {
        "Import PSD"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Point a constraint's bone references at their merged copies.
fn remap_constraint(
    constraint: &mut ankhimate_core::constraints::Constraint,
    map: &HashMap<BoneId, BoneId>,
) {
    use ankhimate_core::constraints::Constraint;
    let remap = |id: &mut BoneId| {
        if let Some(&mapped) = map.get(id) {
            *id = mapped;
        }
    };
    match constraint {
        Constraint::Ik(ik) => {
            remap(&mut ik.target);
            ik.bones.iter_mut().for_each(remap);
        }
        Constraint::Transform(tc) => {
            remap(&mut tc.target);
            tc.bones.iter_mut().for_each(remap);
        }
        Constraint::Physics(p) => remap(&mut p.bone),
        Constraint::Path(p) => p.bones.iter_mut().for_each(remap),
    }
}
