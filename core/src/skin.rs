//! Skins — per-slot attachment dictionaries (PLAN §2.4, ADR 0003).
//!
//! A slot stores only the **name** of the attachment it shows. The actual
//! attachment data lives in a [`Skin`], keyed by `(slot, name)`. Swapping the
//! active skin re-textures the whole character with zero changes to animations,
//! because attachment timelines only ever write names.
//!
//! Every skeleton has a default skin, created in `Skeleton::new`. Resolution
//! looks in the active skin first and falls back to the default, so a partial
//! skin ("just a different head") only needs to list what it overrides.

use crate::attachment::Attachment;
use crate::ids::SlotId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A named set of attachments, keyed by the slot they belong to and the
/// attachment name a slot (or an attachment timeline) refers to.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Skin {
    pub name: String,
    /// `(slot, attachment-name)` → attachment data.
    pub entries: HashMap<(SlotId, String), Attachment>,
}

impl Skin {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: HashMap::new(),
        }
    }

    /// Add or replace an attachment for `(slot, name)`, returning the previous
    /// entry if there was one.
    pub fn set(
        &mut self,
        slot: SlotId,
        name: impl Into<String>,
        attachment: Attachment,
    ) -> Option<Attachment> {
        self.entries.insert((slot, name.into()), attachment)
    }

    pub fn get(&self, slot: SlotId, name: &str) -> Option<&Attachment> {
        // `HashMap<(SlotId, String), _>` cannot be probed with `(SlotId, &str)`
        // without allocating; the key is small and lookups are per-slot per-frame
        // at worst, so the allocation is acceptable for now. If it shows up in a
        // profile, switch `entries` to a nested `SecondaryMap<SlotId, HashMap<..>>`.
        self.entries.get(&(slot, name.to_string()))
    }

    pub fn remove(&mut self, slot: SlotId, name: &str) -> Option<Attachment> {
        self.entries.remove(&(slot, name.to_string()))
    }

    /// Every attachment name this skin defines for `slot`.
    pub fn names_for_slot(&self, slot: SlotId) -> impl Iterator<Item = &str> {
        self.entries
            .keys()
            .filter(move |(s, _)| *s == slot)
            .map(|(_, name)| name.as_str())
    }

    /// Drop every entry belonging to `slot` — used when a slot is deleted.
    pub fn remove_slot(&mut self, slot: SlotId) {
        self.entries.retain(|(s, _), _| *s != slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment::{Rect, RegionAttachment};
    use crate::ids::SlotId;
    use slotmap::{Key, KeyData};

    fn slot_id(n: u64) -> SlotId {
        SlotId::from(KeyData::from_ffi(n))
    }

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

    #[test]
    fn set_and_get_roundtrip() {
        let mut skin = Skin::new("default");
        let slot = slot_id(1);
        assert!(skin.get(slot, "arm").is_none());

        skin.set(slot, "arm", region("arm.png"));
        assert!(matches!(skin.get(slot, "arm"), Some(Attachment::Region(_))));
        // Names are per-slot: another slot with the same name is a distinct entry.
        assert!(skin.get(slot_id(2), "arm").is_none());
    }

    #[test]
    fn set_replaces_and_returns_previous() {
        let mut skin = Skin::new("default");
        let slot = slot_id(1);
        skin.set(slot, "arm", region("old.png"));
        let previous = skin.set(slot, "arm", region("new.png"));

        match previous {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "old.png"),
            other => panic!("expected the replaced region, got {other:?}"),
        }
        match skin.get(slot, "arm") {
            Some(Attachment::Region(r)) => assert_eq!(r.texture, "new.png"),
            other => panic!("expected the new region, got {other:?}"),
        }
    }

    #[test]
    fn names_for_slot_lists_only_that_slot() {
        let mut skin = Skin::new("default");
        let a = slot_id(1);
        let b = slot_id(2);
        skin.set(a, "open", region("open.png"));
        skin.set(a, "closed", region("closed.png"));
        skin.set(b, "hat", region("hat.png"));

        let mut names: Vec<&str> = skin.names_for_slot(a).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["closed", "open"]);

        let names: Vec<&str> = skin.names_for_slot(b).collect();
        assert_eq!(names, vec!["hat"]);
    }

    #[test]
    fn remove_slot_drops_every_entry_for_it() {
        let mut skin = Skin::new("default");
        let a = slot_id(1);
        let b = slot_id(2);
        skin.set(a, "open", region("open.png"));
        skin.set(a, "closed", region("closed.png"));
        skin.set(b, "hat", region("hat.png"));

        skin.remove_slot(a);
        assert_eq!(skin.names_for_slot(a).count(), 0);
        assert_eq!(skin.names_for_slot(b).count(), 1);
    }

    #[test]
    fn key_is_not_a_dangling_default() {
        // Guards the test helper itself: `SlotId::from(KeyData::from_ffi(1))` must
        // not collide with the null key, or every lookup above would be vacuous.
        assert!(!slot_id(1).is_null());
    }
}
