# Nested armatures

A slot draws another rig, animating on its own timeline.

Status: **not implemented.** The DragonBones importer flattens the cases its
sample files use and reports the rest (`formats/src/dragonbones.rs`). This is the
plan for the real feature.

## Why

Spine has no equivalent. Its nearest thing is skin-based attachment swapping,
which is static: a skin picks *which image*, never *what is playing*.

Three things nesting buys that a flat model cannot:

- **Reuse.** One explosion rig referenced by twenty characters, fixed once.
  Today the same effect is copied into every rig that wants it, and diverges.
- **Independent timing.** The child plays at its own rate on its own playhead,
  so a two-second flame does not have to be re-keyed into every host clip that
  triggers it.
- **Modularity.** Swapping a weapon brings its animations along, rather than
  bringing art and leaving the animator to re-author the motion.

The importer already shows why the flat version is fragile. `we_bl_4` folds into
a `Sequence`, and it only reads correctly because the host shows it for exactly
as many frames as the sequence is long — see the `Loop`-not-`Once` note in
`dragonbones.rs`. A rig where those two differ imports wrong, and nothing in the
model can express the fix.

**Observed, not hypothetical.** `mecha_1004d`'s `skill_05` plays its flash close
to the original but not identically: the sequence is driven from the host clip's
playhead, so it is somewhere mid-cycle when the slot track switches it on rather
than at its first frame. `StartOnShow` is what makes that exact, and it cannot be
expressed without the child owning a playhead.

## The shape

A nested armature is not a new field on an attachment. It is **another
`(Skeleton, animations, playhead)`**, which makes this a recursion in
`evaluate` rather than a widening of `Attachment`.

```rust
pub struct ArmatureAttachment {
    /// Names a rig in the document's library, not an inline copy — that is what
    /// makes one explosion shared by twenty hosts rather than twenty copies.
    pub armature: String,
    /// Clip to run on the child, and how.
    pub clip: Option<String>,
    pub mode: ChildPlayback,
    /// Placement in the slot bone's frame, as a region has.
    pub local_offset: Vec2,
    pub local_rotation: f32,
    pub local_scale: Vec2,
}
```

`ChildPlayback` is where the design earns its keep:

- `FollowHost` — the child's playhead is the host's, scaled. Deterministic,
  needs no extra state, and is what a looping ambient effect wants.
- `StartOnShow` — the child's playhead is measured from the frame the
  attachment became visible. What a muzzle flash wants, and what the current
  `Sequence` flattening cannot express.
- `Free` — the child runs on wall time, decoupled from the host.

`FollowHost` and `StartOnShow` are both functions of pose inputs, so they keep
`evaluate` deterministic (PLAN §2.6). **`Free` does not** and is therefore
out of scope for `core` — it belongs to a runtime that owns a clock, the same
way physics owns its accumulator.

## Where it lands

- `core/src/attachment.rs` — the variant. Seven kinds instead of six; every
  exhaustive match is a compiler error, which is the cheap part.
- `core/src/skeleton.rs` — a document holds a **library of armatures**, not one
  skeleton. This is the invasive change: `Document.skeleton` becomes
  `Document.armatures: SlotMap<ArmatureId, Skeleton>` plus a root id.
- `core/src/pose.rs` — `evaluate` recurses. `Pose` gains child poses, keyed by
  the slot that hosts them, since a child's bones are not the host's.
- `formats/` — `.ankh` grows an armature list; the DragonBones reader stops
  flattening and the report loses its nested-armature entries.
- `editor/` — the hierarchy shows children, and the timeline needs a way to say
  which armature a clip belongs to.
- `export/` — `docs/export-context.md` is a public contract, so a nested rig
  needs a context shape that does not break templates written before it.

## Cost, honestly

The attachment variant is a day. **The library-of-armatures change is not** — it
touches every place that says `doc.skeleton`, which is most of the editor, and
it changes the `.ankh` schema, which needs a migration.

## Ordering

After the plugin work (`docs/plugin-plan.md`), not before. Two reasons:

- Step 1 of that plan splits document-level from session-level operators, and
  doing it against one skeleton and then again against a library is the same
  work twice.
- `export`'s context is a public contract. Adding nesting to it after the
  template engine has users is a breaking change; adding it while the operator
  surface is already being reshaped is one disruption instead of two.

## Until then

The importer flattens what it can and reports what it cannot. A one-bone nested
armature holding a flipbook becomes a `Sequence` — exact for these files. A
multi-bone nested armature reports `folded == 0` rather than silently mangling
it, which is the property that matters: nothing is lost quietly.
