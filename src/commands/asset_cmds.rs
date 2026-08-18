//! Asset library mutations as undoable commands (T-301).
//!
//! Importing an image is structural — it is part of what the rig *is* — so every
//! command here is Setup-mode only (T-207).

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::assets::ImageAsset;
use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
use ankhimate_core::ids::{AssetId, BoneId, SlotId};
use ankhimate_core::slot::Slot;

/// Import an image and hang it off a bone: asset + slot + region attachment in
/// the default skin, in one undo step.
///
/// The three land together because separately they are useless — an asset with
/// no attachment draws nothing, an attachment with no asset is a dangling name —
/// and a user who drags a PNG onto the canvas means "put this on the rig".
pub struct ImportImage {
    asset: ImageAsset,
    bone: BoneId,
    /// Where the attachment sits relative to the bone, in world units.
    offset: glam::Vec2,
    /// Filled by `apply` so `revert` knows what to remove, and so callers can
    /// select what they just created.
    created: Option<Created>,
}

#[derive(Clone, Copy)]
pub struct Created {
    pub asset: AssetId,
    pub slot: SlotId,
}

impl ImportImage {
    pub fn new(asset: ImageAsset, bone: BoneId, offset: glam::Vec2) -> Self {
        Self {
            asset,
            bone,
            offset,
            created: None,
        }
    }

    pub fn created(&self) -> Option<Created> {
        self.created
    }
}

impl EditCommand for ImportImage {
    fn apply(&mut self, doc: &mut Document) {
        if !doc.skeleton.bones.contains_key(self.bone) {
            return;
        }
        let asset_id = doc.assets.add(self.asset.clone());
        // The db may have uniquified the name; the attachment must reference the
        // name that actually landed, not the one we asked for.
        let (name, size) = match doc.assets.get(asset_id) {
            Some(a) => (a.name.clone(), a.size()),
            None => return,
        };

        let slot_id = doc.skeleton.add_slot(Slot {
            attachment: Some(name.clone()),
            ..Slot::new(name.clone(), self.bone)
        });

        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(
            slot_id,
            &name,
            Attachment::Region(RegionAttachment {
                texture: name.clone(),
                local_offset: self.offset,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: size.x,
                height: size.y,
                uv_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                pivot: glam::Vec2::splat(0.5),
                sequence: None,
            }),
        );

        self.created = Some(Created {
            asset: asset_id,
            slot: slot_id,
        });
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(created) = self.created.take() else {
            return;
        };
        // Keep the asset's own bytes for the redo: `self.asset` still holds them,
        // but the *name* may have been uniquified, so re-read it before dropping.
        if let Some(asset) = doc.assets.get(created.asset) {
            let name = asset.name.clone();
            let skin = doc.skeleton.default_skin;
            doc.skeleton.skins[skin].remove(created.slot, &name);
        }
        doc.assets.remove(created.asset);
        doc.skeleton.slots.remove(created.slot);
        doc.skeleton.draw_order.retain(|&s| s != created.slot);
        for (_, skin) in doc.skeleton.skins.iter_mut() {
            skin.remove_slot(created.slot);
        }
    }

    fn label(&self) -> &str {
        "Import Image"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Import an image into the library without attaching it anywhere.
pub struct AddAsset {
    asset: ImageAsset,
    created: Option<AssetId>,
}

impl AddAsset {
    pub fn new(asset: ImageAsset) -> Self {
        Self {
            asset,
            created: None,
        }
    }

    pub fn created_id(&self) -> Option<AssetId> {
        self.created
    }
}

impl EditCommand for AddAsset {
    fn apply(&mut self, doc: &mut Document) {
        self.created = Some(doc.assets.add(self.asset.clone()));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.assets.remove(id);
        }
    }

    fn label(&self) -> &str {
        "Add Asset"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Add several images at once — one undo step for one import (T-305).
///
/// Slicing a sheet into sixteen cells is a single user action; sixteen undo
/// steps to take it back would be a punishment for using the tool.
pub struct AddAssets {
    assets: Vec<ImageAsset>,
    created: Vec<AssetId>,
}

impl AddAssets {
    pub fn new(assets: Vec<ImageAsset>) -> Self {
        Self {
            assets,
            created: Vec::new(),
        }
    }

    pub fn created_ids(&self) -> &[AssetId] {
        &self.created
    }
}

impl EditCommand for AddAssets {
    fn apply(&mut self, doc: &mut Document) {
        self.created = self
            .assets
            .iter()
            .map(|asset| doc.assets.add(asset.clone()))
            .collect();
    }

    fn revert(&mut self, doc: &mut Document) {
        for id in self.created.drain(..) {
            doc.assets.remove(id);
        }
    }

    fn label(&self) -> &str {
        "Import Sheet"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete an asset from the library.
///
/// Attachments referencing it are left alone and become dangling — the same
/// shape a missing file has, which the diagnostics pass (T-702) reports and the
/// renderer skips. Silently rewriting the user's attachments would be worse.
pub struct DeleteAsset {
    target: AssetId,
    removed: Option<ImageAsset>,
    restored: Option<AssetId>,
}

impl DeleteAsset {
    pub fn new(target: AssetId) -> Self {
        Self {
            target,
            removed: None,
            restored: None,
        }
    }
}

impl EditCommand for DeleteAsset {
    fn apply(&mut self, doc: &mut Document) {
        let id = self.restored.take().unwrap_or(self.target);
        self.removed = doc.assets.remove(id);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(asset) = self.removed.take() {
            // `add` re-uniquifies, but the name is free again after the removal,
            // so the original name comes back.
            let id = doc.assets.add(asset);
            self.target = id;
            self.restored = Some(id);
        }
    }

    fn label(&self) -> &str {
        "Delete Asset"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Replace an asset's pixels, keeping its name and every reference to it (T-306).
///
/// Both "reload from source" and "relink to a new file" are this command: the
/// only difference is whether the path changes, and attachments reference the
/// *name*, so neither has to touch the rig.
pub struct ReplaceAssetPixels {
    target: AssetId,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    source_path: Option<String>,
    before: Option<(Vec<u8>, u32, u32, Option<String>)>,
    label: &'static str,
}

impl ReplaceAssetPixels {
    /// Re-read the file the asset already points at.
    pub fn reload(target: AssetId, bytes: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            target,
            bytes,
            width,
            height,
            source_path: None,
            before: None,
            label: "Reload Image",
        }
    }

    /// Point the asset at a different file and take its pixels.
    pub fn relink(target: AssetId, bytes: Vec<u8>, width: u32, height: u32, path: String) -> Self {
        Self {
            target,
            bytes,
            width,
            height,
            source_path: Some(path),
            before: None,
            label: "Relink Image",
        }
    }
}

impl EditCommand for ReplaceAssetPixels {
    fn apply(&mut self, doc: &mut Document) {
        let Some(asset) = doc.assets.images.get_mut(self.target) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some((
                std::mem::take(&mut asset.bytes),
                asset.width,
                asset.height,
                asset.source_path.clone(),
            ));
        }
        asset.bytes = self.bytes.clone();
        asset.width = self.width;
        asset.height = self.height;
        if let Some(path) = &self.source_path {
            asset.source_path = Some(path.clone());
        }
        // Attachment sizes are deliberately left alone: art is often re-exported
        // at a different resolution while meaning the same thing on the rig, and
        // silently resizing every attachment would undo the user's placement.
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some((bytes, width, height, path)), Some(asset)) =
            (self.before.take(), doc.assets.images.get_mut(self.target))
        {
            asset.bytes = bytes;
            asset.width = width;
            asset.height = height;
            asset.source_path = path;
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

/// Rename an asset, rewriting every attachment that references it.
pub struct RenameAsset {
    target: AssetId,
    new_name: String,
    before: Option<String>,
}

impl RenameAsset {
    pub fn new(target: AssetId, new_name: impl Into<String>) -> Self {
        Self {
            target,
            new_name: new_name.into(),
            before: None,
        }
    }
}

impl RenameAsset {
    /// Point every region/mesh attachment at `to` wherever it said `from`.
    fn retarget(doc: &mut Document, from: &str, to: &str) {
        for (_, skin) in doc.skeleton.skins.iter_mut() {
            for attachment in skin.entries.values_mut() {
                let texture = match attachment {
                    Attachment::Region(r) => &mut r.texture,
                    Attachment::Mesh(m) => &mut m.texture,
                    // The rest reference no asset, so a rename cannot touch them.
                    Attachment::Clipping(_)
                    | Attachment::Path(_)
                    | Attachment::BoundingBox(_)
                    | Attachment::Point(_) => continue,
                };
                if texture == from {
                    *texture = to.to_string();
                }
            }
        }
    }
}

impl EditCommand for RenameAsset {
    fn apply(&mut self, doc: &mut Document) {
        let Some(old) = doc.assets.get(self.target).map(|a| a.name.clone()) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(old.clone());
        }
        if let Some(applied) = doc.assets.rename(self.target, &self.new_name) {
            Self::retarget(doc, &old, &applied);
            self.new_name = applied;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        let Some(before) = self.before.take() else {
            return;
        };
        let current = doc.assets.get(self.target).map(|a| a.name.clone());
        if let Some(current) = current
            && let Some(applied) = doc.assets.rename(self.target, &before)
        {
            Self::retarget(doc, &current, &applied);
        }
    }

    fn label(&self) -> &str {
        "Rename Asset"
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

    fn png(name: &str) -> ImageAsset {
        ImageAsset::new(
            name,
            vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            64,
            32,
        )
    }

    #[test]
    fn import_creates_asset_slot_and_attachment_in_one_step() {
        let (mut doc, bone) = doc_with_bone();
        let mut history = History::default();

        history.push(
            Box::new(ImportImage::new(png("arm"), bone, glam::Vec2::ZERO)),
            &mut doc,
        );

        assert_eq!(doc.assets.len(), 1);
        assert_eq!(doc.skeleton.slots.len(), 1);
        assert_eq!(doc.skeleton.draw_order.len(), 1);
        let skin = doc.skeleton.default_skin;
        assert_eq!(doc.skeleton.skins[skin].entries.len(), 1);

        // The attachment carries the image's pixel size, so it draws at 1:1.
        let att = doc.skeleton.skins[skin].entries.values().next().unwrap();
        match att {
            Attachment::Region(r) => {
                assert_eq!((r.width, r.height), (64.0, 32.0));
                assert_eq!(r.texture, "arm");
            }
            _ => panic!("expected a region attachment"),
        }

        // One undo removes all three.
        history.undo(&mut doc);
        assert_eq!(doc.assets.len(), 0);
        assert_eq!(doc.skeleton.slots.len(), 0);
        assert_eq!(doc.skeleton.skins[skin].entries.len(), 0);
        assert!(doc.skeleton.draw_order.is_empty());
    }

    #[test]
    fn importing_a_duplicate_name_uniquifies_and_the_attachment_follows() {
        let (mut doc, bone) = doc_with_bone();
        let mut history = History::default();
        history.push(
            Box::new(ImportImage::new(png("arm"), bone, glam::Vec2::ZERO)),
            &mut doc,
        );
        history.push(
            Box::new(ImportImage::new(png("arm"), bone, glam::Vec2::ZERO)),
            &mut doc,
        );

        assert_eq!(doc.assets.len(), 2);
        assert!(doc.assets.by_name("arm_2").is_some());
        let skin = doc.skeleton.default_skin;
        let textures: Vec<String> = doc.skeleton.skins[skin]
            .entries
            .values()
            .map(|a| match a {
                Attachment::Region(r) => r.texture.clone(),
                Attachment::Mesh(m) => m.texture.clone(),
                _ => String::new(),
            })
            .collect();
        assert!(textures.contains(&"arm".to_string()));
        assert!(
            textures.contains(&"arm_2".to_string()),
            "second attachment points at the uniquified asset, not the first image"
        );
    }

    #[test]
    fn rename_rewrites_attachment_references() {
        let (mut doc, bone) = doc_with_bone();
        let mut history = History::default();
        history.push(
            Box::new(ImportImage::new(png("arm"), bone, glam::Vec2::ZERO)),
            &mut doc,
        );
        let id = doc.assets.by_name("arm").unwrap();

        history.push(Box::new(RenameAsset::new(id, "forearm")), &mut doc);
        let skin = doc.skeleton.default_skin;
        let att = doc.skeleton.skins[skin].entries.values().next().unwrap();
        match att {
            Attachment::Region(r) => assert_eq!(r.texture, "forearm"),
            _ => panic!(),
        }

        history.undo(&mut doc);
        let att = doc.skeleton.skins[skin].entries.values().next().unwrap();
        match att {
            Attachment::Region(r) => assert_eq!(r.texture, "arm", "undo restored the reference"),
            _ => panic!(),
        }
    }

    #[test]
    fn delete_asset_round_trips() {
        let mut doc = Document::new();
        let mut history = History::default();
        let id = doc.assets.add(png("arm"));

        history.push(Box::new(DeleteAsset::new(id)), &mut doc);
        assert_eq!(doc.assets.len(), 0);

        history.undo(&mut doc);
        assert_eq!(doc.assets.len(), 1);
        assert!(doc.assets.by_name("arm").is_some(), "name came back too");
    }
}
