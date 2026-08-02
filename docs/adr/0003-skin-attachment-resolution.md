# ADR 003: Skin-based attachment resolution model

- **Status:** Accepted
- **Date:** 2026-07-30
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §2.3, §2.4, §1.2 (defect D4)

## Context

The original `Skeleton.attachments: HashMap<SlotId, Attachment>` bound each slot
to exactly one attachment permanently (defect D4). This cannot support alternate
skins (e.g. a character's summer/winter outfits), nor attachment swapping
without losing the original. Spine, Spriter, and the open-source editors that
chase them all separate the *slot* (which holds a name) from the *attachment
data* (which
lives in a skin dictionary).

## Decision

A `Slot` holds only an attachment **name** (`Option<String>`); the active
**Skin** resolves `(slot, name) → Attachment`. There is exactly one resolution
rule:

```rust
fn resolve(skel, active, slot, name) -> Option<&Attachment> {
    skel.skins[active].entries.get(&(slot, name))
        .or_else(|| skel.skins[skel.default_skin].entries.get(&(slot, name)))
}
```

- `Skeleton.skins: SlotMap<SkinId, Skin>` + `default_skin: SkinId`.
- Animations may only change `slot.attachment` **names** (attachment timeline),
  never attachment data.
- Swapping the active skin re-textures the whole character with zero animation
  changes — strictly more general than a single global "style" switch.

## Alternatives considered

- **Per-slot attachment list with an index:** loses the skin abstraction;
  re-skinning means editing every slot.
- **Attachment data inline on the slot (original):** no skin swapping at all.

## Consequences

- Renderers obtain attachments through `resolve` only — a single code path.
- The `MeshAttachment` weight/FFD code is reworked: `bone_index: usize` →
  `BoneId`; inverse binds `Mat4` → `Affine2`; FFD keyframes move out of the
  attachment into animations as **deform timelines**.
- A `Session.active_skin` field drives which skin the editor renders.
