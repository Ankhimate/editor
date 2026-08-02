# ADR 001: Slotmap keys for all entity identity

- **Status:** Accepted
- **Date:** 2026-07-30
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §2.1, §1.2 (defect D1)

## Context

The original `core` identified bones by `Vec` index (`Bone.parent_index: Option<usize>`,
`Slot.bone: usize`, `VertexWeight.bone_index: usize`, `IkConstraint.*_index: usize`).
Deleting or reordering a bone silently corrupted every reference. World-transform
update also assumed the `Vec` was topologically sorted, an invariant nothing
enforced (defect D1).

We need stable identity that survives deletion and reordering, with O(1)
lookup, while keeping the in-memory document cheap to mutate.

## Decision

Use `slotmap` keys for **all** entities inside the in-memory document — bones
included, not just slots:

```rust
slotmap::new_key_type! {
    pub struct BoneId;
    pub struct SlotId;
    pub struct SkinId;
    pub struct AnimationId;
    pub struct ConstraintId;
}
```

- `Bone.parent: Option<BoneId>`; every other `usize` bone reference becomes a
  typed ID.
- Stable string names exist **only** at the serialization boundary; the on-disk
  format never stores slotmap keys (see ADR 004).
- Traversal uses an explicit `update_order: Vec<BoneId>` rebuilt on hierarchy
  edits, never insertion order.

## Alternatives considered

- **`Uuid` per entity:** stable and copyable, but cache-unfriendly (128-bit),
  adds a generation/lookup layer, and human-unreadable when debugging.
- **Generational indices (hand-rolled):** exactly what `slotmap` provides, but
  battle-tested and serde-skippable. No reason to hand-roll.

## Consequences

- Deleting a bone reparents/drops dependents via one `remove_bone(id)` API; IDs
  of all other entities stay valid.
- `Option<BoneId>` is niche-optimized (no extra storage over the key).
- Core gains a hard dependency on `slotmap` (already present).
- Serialization must map id↔name, adding a (small, one-time-per-save) step.
