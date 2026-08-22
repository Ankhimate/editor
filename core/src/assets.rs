//! Image assets — the pixels a rig is made of (T-301).
//!
//! Assets live in the document, not the skin: a skin says *which attachment* a
//! slot shows, an attachment says *which asset* it samples, and the asset owns
//! the bytes. Swapping a skin therefore never touches pixels, and two
//! attachments can share one asset without copying it.
//!
//! Bytes are stored **verbatim** as the encoded file (PNG/JPG/WebP), never
//! re-encoded: a save must return the user's own pixels, and core has no image
//! decoder (PLAN §3.1 forbids one here). `width`/`height` are supplied by the
//! importer, which does the decoding.

use crate::ids::AssetId;
use crate::slotmap::SlotMap;
use serde::{Deserialize, Serialize};

/// One imported image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Unique within the document — attachments reference this on disk (ADR 0004).
    pub name: String,
    /// The encoded file, byte-for-byte as imported.
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Where it came from, when it came from a file. Used by "reload from
    /// source" and relinking (T-306); absent for assets that arrived inside an
    /// `.ankh` written on another machine.
    #[serde(default)]
    pub source_path: Option<String>,
}

impl ImageAsset {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            name: name.into(),
            bytes,
            width,
            height,
            source_path: None,
        }
    }

    pub fn size(&self) -> glam::Vec2 {
        glam::vec2(self.width as f32, self.height as f32)
    }
}

/// The document's image library.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetDb {
    pub images: SlotMap<AssetId, ImageAsset>,
}

impl AssetDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn get(&self, id: AssetId) -> Option<&ImageAsset> {
        self.images.get(id)
    }

    /// Insert, uniquifying the name (`arm`, `arm_2`, `arm_3`, …).
    ///
    /// Names are the on-disk reference (ADR 0004), so a collision would make one
    /// of the two unreachable after a round trip. Uniquifying on the way in is
    /// the same rule `Skeleton::add_bone` follows.
    pub fn add(&mut self, mut asset: ImageAsset) -> AssetId {
        asset.name = self.unique_name(&asset.name);
        self.images.insert(asset)
    }

    pub fn remove(&mut self, id: AssetId) -> Option<ImageAsset> {
        self.images.remove(id)
    }

    pub fn by_name(&self, name: &str) -> Option<AssetId> {
        self.images
            .iter()
            .find(|(_, a)| a.name == name)
            .map(|(id, _)| id)
    }

    /// Rename, uniquifying against the other assets. Returns the name applied.
    pub fn rename(&mut self, id: AssetId, name: &str) -> Option<String> {
        if !self.images.contains_key(id) {
            return None;
        }
        let unique = self.unique_name_excluding(name, Some(id));
        let asset = self.images.get_mut(id)?;
        asset.name = unique.clone();
        Some(unique)
    }

    fn unique_name(&self, wanted: &str) -> String {
        self.unique_name_excluding(wanted, None)
    }

    fn unique_name_excluding(&self, wanted: &str, ignore: Option<AssetId>) -> String {
        let taken = |candidate: &str| {
            self.images
                .iter()
                .any(|(id, a)| Some(id) != ignore && a.name == candidate)
        };
        if !taken(wanted) {
            return wanted.to_string();
        }
        let mut n = 2;
        loop {
            let candidate = format!("{wanted}_{n}");
            if !taken(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ImageAsset {
        ImageAsset::new(name, vec![1, 2, 3], 4, 4)
    }

    #[test]
    fn names_are_uniquified_on_insert() {
        let mut db = AssetDb::new();
        let a = db.add(asset("arm"));
        let b = db.add(asset("arm"));
        let c = db.add(asset("arm"));
        assert_eq!(db.get(a).unwrap().name, "arm");
        assert_eq!(db.get(b).unwrap().name, "arm_2");
        assert_eq!(db.get(c).unwrap().name, "arm_3");
    }

    #[test]
    fn lookup_by_name_and_rename() {
        let mut db = AssetDb::new();
        let a = db.add(asset("arm"));
        db.add(asset("leg"));
        assert_eq!(db.by_name("arm"), Some(a));

        // Renaming onto a taken name uniquifies rather than colliding.
        assert_eq!(db.rename(a, "leg").as_deref(), Some("leg_2"));
        // Renaming to its own name is a no-op, not `arm_2`.
        let leg2 = db.by_name("leg_2").unwrap();
        assert_eq!(db.rename(leg2, "leg_2").as_deref(), Some("leg_2"));
    }

    #[test]
    fn removing_frees_the_name() {
        let mut db = AssetDb::new();
        let a = db.add(asset("arm"));
        db.remove(a);
        let b = db.add(asset("arm"));
        assert_eq!(db.get(b).unwrap().name, "arm", "name was released");
    }
}
