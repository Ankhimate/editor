//! Stable entity identity for all in-memory document entities.
//!
//! See ADR 0001 and PLAN §2.1. We use [`slotmap`] keys for **all** entities
//! (bones included, not just slots) so that deletes/reorders never corrupt
//! references. Stable string names exist only at the serialization boundary
//! (ADR 0004).

use slotmap::new_key_type;

new_key_type! {
    /// A bone in a [`crate::skeleton::Skeleton`].
    pub struct BoneId;
    /// A slot (draw-order entry) in a [`crate::skeleton::Skeleton`].
    pub struct SlotId;
    /// A skin (per-slot attachment dictionary) — see `skin` (T-105).
    pub struct SkinId;
    /// An animation in a document.
    pub struct AnimationId;
    /// A constraint (IK, transform, path, …) in a [`crate::skeleton::Skeleton`].
    pub struct ConstraintId;
    /// An image asset in a document's [`crate::assets::AssetDb`] (T-301).
    pub struct AssetId;
}

// `slotmap` keys serialize as integers by default; we never persist them (names
// are used on disk per ADR 0004), but serde is derived so they can round-trip
// through editor-only snapshots without a custom impl.

// Re-export the slotmap container types used across the crate so callers don't
// need to depend on slotmap directly for the common case.
pub use slotmap::{DenseSlotMap, KeyData, SecondaryMap, SlotMap};
