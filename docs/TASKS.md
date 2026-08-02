# Ankhimate — Agent Task List

> Companion to [ARCHITECTURE_PLAN.md](ARCHITECTURE_PLAN.md) (referenced below as **PLAN §n**).
> Each task is designed to be handed to one AI agent as a single PR-sized work unit.
>
> **Rules for every agent:**
> - Read PLAN §0 (license), §2 (target data model), and the §-refs listed on your task before coding.
> - **Never** copy, translate, or closely paraphrase source code from another animation editor
>   (several are GPL-3.0). Feature behavior only, clean-room (PLAN §0).
> - `cargo test && cargo clippy --workspace -- -D warnings && cargo fmt --check` must pass at the
>   end of the task. Do not break the editor build even if your task is core-only.
> - New decisions that deviate from the PLAN require an ADR file in `docs/adr/` and a note in the
>   PR description.
> - Types/signatures in the PLAN are normative; keep names unless the PLAN marks them *(suggestion)*.
> - From T-207 onward: **every new editing surface must declare its behavior in both work modes**
>   (Setup / Animate). A PR that adds an edit path without stating its mode semantics is incomplete.
>
> **Legend:** `Deps:` tasks that must be merged first. `∥` = tasks in the same group that can run
> in parallel with each other. ★ = suitable for a smaller/less capable agent.

## Revision note (this pass)

- Phases 0–2 shipped (T-001 … T-206) and are kept verbatim below for reference; amendments added
  where the mode split changes them.
- **New foundational task T-207 — Setup mode / Animate mode.** It is the spine of the editor UX and
  every later task references it. Nothing in Phase 3+ should be built before it lands.
- Phase 2 gained T-208 … T-211 (animation manager, clipboard, keyed-value affordances, pose tools).
- Former Phase 5 (export + polish) was split and renumbered so a **Phase 5 "Rig power"** could hold
  the constraint/animation-depth work that a production system needs and the old list omitted
  (transform/path/physics constraints, events, animatable visibility, IK completeness).
- New numbering: **3xx** assets & import · **4xx** mesh & deform · **5xx** rig power ·
  **6xx** export & runtime · **7xx** production polish · **8xx** release. Old T-4xx/T-5xx/T-6xx
  numbers are re-mapped where noted on the task (`was T-…`).

---

## Phase 0 — Groundwork

### ✅ T-001 ★ License + repo hygiene
**Deps:** none · **Refs:** PLAN §0, §3.3
- Add `LICENSE-MIT` + `LICENSE-APACHE` at workspace root; `license = "MIT OR Apache-2.0"` in every
  crate's `Cargo.toml`. *(If the maintainer has chosen differently, stop and ask.)*
- Add `README.md` stub (project pitch, build instructions from PLAN), `CONTRIBUTING.md` stub.
- Pin `egui_tiles = "*"` in `editor/Cargo.toml` to the exact version currently in `Cargo.lock`.
- Add `rust-toolchain.toml` (stable, pinned minor).
**Accept:** workspace builds. *(Dependency-licence checking was dropped — see ADR 005.)*

### ✅ T-002 ★ CI skeleton
**Deps:** T-001 · **Refs:** PLAN §3.3
- `.github/workflows/ci.yml`: fmt-check, clippy `-D warnings`, `cargo test --workspace` on
  Linux + Windows; build-only job `cargo check -p ankhimate-core --target wasm32-unknown-unknown`.
- Cache with `Swatinem/rust-cache`.
**Accept:** CI green on a no-op PR.

### ✅ T-003 ★ ADR seed
**Deps:** none · **Refs:** PLAN §2, §6
- Create `docs/adr/template.md` and ADRs 001–005 summarizing decisions already made in the PLAN:
  001 slotmap IDs, 002 Affine2 math (no Mat4 decompose in hot path), 003 skin/attachment
  resolution model, 004 `.ankh` zip container, 005 licensing/clean-room policy.
**Accept:** 5 ADRs exist, each ≤1 page, each linking the PLAN section it records.

---

## Phase 1 — Core remediation (PLAN §4)

> Order within this phase matters: T-101 → T-102 → T-103 → (T-104 ∥ T-105) → T-106 → T-107 → T-108.

### ✅ T-101 Stable entity IDs (R1)
**Deps:** T-001 · **Refs:** PLAN §1.2 D1, §2.1, §2.3
- New `core/src/ids.rs` with `new_key_type!` for `BoneId, SlotId, SkinId, AnimationId, ConstraintId`.
- `Skeleton.bones: SlotMap<BoneId, Bone>`; `Bone.parent: Option<BoneId>`; replace every `usize`
  bone/constraint reference in `core` with typed IDs.
- `Skeleton.update_order: Vec<BoneId>` + `rebuild_update_order()` (topological, deterministic
  tie-break by name). `Skeleton::remove_bone(id)` reparents children and returns a removal report.
**Accept:** unit tests — delete middle bone keeps other bones' identity valid; update order is
topological regardless of insertion order; editor runs.

### ✅ T-102 Affine2 transform math (R2)
**Deps:** T-101 · **Refs:** PLAN §1.2 D2, §2.2
- `core/src/transforms.rs`: `Affine2` with `compose`, `mul`, `transform_point`, `invert`,
  `decompose`. `Bone.inherit: Inherit {rotation, scale, reflect}` honored in the world pass.
  All `Mat4`/`Quat` removed from `skeleton.rs`. Mesh inverse binds are `Affine2`.
**Accept:** property tests — compose∘decompose∘compose stable (ε<1e-4); hand-computed child
position under a (2,1)-scaled rotated parent; IK tests still pass.

### ✅ T-103 Pose extraction + evaluate() skeleton (R3)
**Deps:** T-102 · **Refs:** PLAN §1.2 D7, §2.6
- `core/src/pose.rs` with `Pose` per PLAN §2.6 and `evaluate(skel, anims, out)` (4-stage pipeline).
- `Bone.world_transform` and all `#[serde(skip)]` derived fields deleted. Core is `std::time`-free.
**Accept:** identical inputs → identical poses; viewport renders as before.

### ✅ T-104 ∥ Constraint system + correct IK blending (R4)
**Deps:** T-103 · **Refs:** PLAN §1.2 D3, §2.5
- `core/src/constraints.rs`: `Constraint` enum + ordered application in `evaluate` stage 3.
- Solver outputs **local** rotations; `local_rot += wrap_angle(solved - current) * mix`; FK re-run
  for chain bones + descendants. Chain length 1 (aim) and 2. `softness`/`stretch` fields reserved.
**Accept:** mix=0.5 across ±π produces no flip; reachable/unreachable tests preserved; descendants
follow.

### ✅ T-105 ∥ Skin layer (R5)
**Deps:** T-101 · **Refs:** PLAN §1.2 D4, §2.4
- `core/src/skin.rs`: `Skin { name, entries: HashMap<(SlotId, String), Attachment> }`;
  `Skeleton.skins` + `default_skin`; `Slot.attachment: Option<String>`; free `resolve(...)` with
  default-skin fallback; `Session.active_skin`.
**Accept:** fallback to default skin; swapping active skin swaps rendering without touching slots.

### ✅ T-106 Animation model + sampling (R6)
**Deps:** T-103, T-104, T-105 · **Refs:** PLAN §1.2 D5, §2.7, §2.3
- `core/src/animation.rs` per PLAN §2.7: `Animation`, `Timeline` enum, `Key<T>`,
  `Interp {Linear, Stepped, Bezier}`. Binary search + sequential cache; shortest-arc rotation;
  bezier via documented solve. `evaluate` stage 2 complete (bone/slot/draw-order/IkMix/Deform).
  Multi-animation mixing by `alpha`.
**Accept:** golden curve tests; draw-order offset case; α=0.5 mix across ±π.

### ✅ T-107 Editor Document/Session split + command undo (R7)
**Deps:** T-106 · **Refs:** PLAN §1.2 D6/D7, §3.2
- `editor/src/doc.rs`, `editor/src/session.rs`; `EditCommand` trait (apply/revert/merge/label) +
  `History { cap: 200 }`; commands in `bone_cmds.rs`, `slot_cmds.rs`, `key_cmds.rs`.
- Drags write `Session.preview_locals`; the Document is mutated by one merged command on mouse-up.
**Accept:** create 3 bones, pose, delete one, undo×4 restores exactly; history holds commands.

> **Amendment (T-207):** `editor/src/state.rs` still exists as a compatibility shim. T-207 finishes
> its removal — `AppState` becomes `{ doc, session, pose, history }` and nothing else.

### ✅ T-108 `formats` crate + .ankh v1 save/load (R8)
**Deps:** T-107 · **Refs:** PLAN §1.2 D8, §6.1
- Crate `formats/`: zip container (`project.json` + `images/`), entities keyed by **name**,
  `version: 1`, `migrate.rs` scaffold, rename-on-collision in core. File▸Open/Save/Save-As via `rfd`.
**Accept:** round-trip golden test on `samples/minimal.ankh`; unknown top-level fields survive.

---

## Phase 2 — Animation MVP

> Shipped: T-201 → (T-202 ∥ T-203 ∥ T-205) → T-204; T-206.
> New in this pass: **T-207 → (T-208 ∥ T-209 ∥ T-210 ∥ T-211)**.

### ✅ T-201 Dopesheet panel (F-5)
**Deps:** T-108 · **Refs:** PLAN §5 F-5, §3.2
- `editor/src/ui/timeline/{mod,sheet,ruler,tree,model}.rs`. Rows grouped `bone ▸ property` /
  `slot ▸ property`, fold/unfold, synced scroll, frame ruler at `Document.meta.fps`, playhead
  scrub, diamond keys, drag/drag-off-delete/ctrl-click/shift-range/box-select/context menu,
  zoom 0.1–3.0. All edits via `key_cmds.rs`.
**Accept:** every interaction undoable with correct labels; scrubbing updates the viewport live.

### ✅ T-202 Playback + auto-key (F-8)
**Deps:** T-201 · **Refs:** PLAN §5 F-8, §7.3
- Transport bar (play/pause, loop, FPS, frame step, jump key, start/end); wall-clock playhead in
  editor only; `Session.auto_key` + red viewport border; locked bones never record.
**Accept:** pose@0 → auto-key → pose@12 → play interpolates and loops.

> **Amendment (T-207):** `auto_key` stops being a free-floating toggle. In Animate mode it defaults
> **on**; in Setup mode it is not applicable and is hidden. The toggle survives as an "auto-key off"
> escape hatch inside Animate mode (manual keying via `K`).

### ✅ T-203 Bezier interpolation UI (F-6)
**Deps:** T-201 · **Refs:** PLAN §5 F-6, §2.7
- Presets in the key context menu (Linear, Stepped, Ease In/Out/In-Out, Sine In-Out, Snap); applies
  to a multi-selection as one command; stepped drawn square, bezier diamond+dot.
**Accept:** Ease In-Out visibly changes motion; undo reverts interp.

### ✅ T-204 Draw-order animation UI
**Deps:** T-201 · **Refs:** PLAN §2.3, §2.6
- Draw-order panel listing `Pose.draw_order` at the playhead; drag to reorder writes a `DrawOrder`
  key with auto-key on, edits `Skeleton.draw_order` with it off.
**Accept:** front/back arm swap mid-animation; setup order untouched; undo works both ways.

> **Amendment (T-207):** the "auto-key on/off" branch becomes the mode branch — Setup edits
> `Skeleton.draw_order`, Animate writes `DrawOrder` keys. Same behavior, discoverable rule.

### ✅ T-205 Slot timelines UI (color + attachment swap)
**Deps:** T-201 · **Refs:** PLAN §2.7
- Slot inspector: keyable color picker, keyable attachment dropdown (stepped) over the active skin.
**Accept:** blink (alpha keys) + mouth swap replay after save/load.

### ✅ T-206 ★ Hierarchy panel UX (F-12)
**Deps:** T-107 · **Refs:** PLAN §5 F-12
- `editor/src/ui/tree.rs`: drag-and-drop reparent (onto = child, between = sibling) with
  `ReparentBoneCommand` preserving world pose; visibility eye, lock padlock, fold, inline rename,
  cascade delete with child count.
**Accept:** reparenting a posed bone leaves the pose identical; locked bone ignores viewport drags.

> **Amendment (T-207):** reparent / create / delete / rename are **structural** — Setup mode only.
> In Animate mode the rows stay visible and selectable but structural affordances are disabled with
> a "switch to Setup to edit the rig" tooltip.

---

### ✅ T-207 Setup mode / Animate mode (the editing-context split)
**Deps:** T-107, T-202, T-204 · **Refs:** PLAN §2.6, §3.2, §7 · **ADR:** [0006-work-modes](adr/0006-work-modes.md)

> **Shipped.** `Session.work_mode`, `editor/src/edit_router.rs`,
> `EditCommand::requires_mode` + `History::push_in_mode`, mode switch in the toolbar
> (`Tab`), mode chrome in the viewport, `state.rs` → `app_state.rs`. Two defects found
> and fixed on the way: posing at `t = 0` with auto-key on used to overwrite the setup
> pose instead of keying, and the hierarchy panel mutated `draw_order`/created slots
> outside the command system (neither undoable nor mode-gated).

The single largest UX gap versus Spine/Spriter, and the rule that makes every later feature
unambiguous: *the same gesture means "define the rig" in Setup and "animate the rig" in Animate.*

**Model**
- `Session.work_mode: WorkMode { Setup, Animate }` (default `Setup`). Rename the existing
  `EditorMode` (Select / CreateBone / WeightPaint) to `Tool` — it is a tool selector, not a mode,
  and the name collision is the reason the split was never obvious in the UI.
- Setup mode always evaluates with **no animation applied** (`evaluate(skel, &[], &mut pose)`),
  regardless of playhead. The playhead is not lost — it is simply not applied.
- Animate mode requires `Session.active_animation`; entering it with none prompts to create one
  (T-208) rather than silently doing nothing.

**Edit routing (the normative part).** Add `editor/src/edit_router.rs`. Every viewport/inspector
edit calls the router, never a command constructor directly:

```rust
pub enum EditIntent {
    BoneLocal { bone: BoneId, value: Transform },   // pose gesture or numeric field
    SlotColor { slot: SlotId, value: [f32; 4] },
    SlotAttachment { slot: SlotId, value: Option<String> },
    DrawOrder { order: Vec<SlotId> },
    Deform { slot: SlotId, attachment: String, verts: Vec<Vec2> },   // T-404
    IkMix { constraint: ConstraintId, value: f32 },
}

/// Setup  → mutates Skeleton setup data via the corresponding *setup* command.
/// Animate → writes/updates keys on `session.active_animation` at `session.playhead`
///           via the corresponding *key* command (offsets from setup, PLAN §2.7).
pub fn route(intent: EditIntent, doc: &mut Document, session: &Session) -> Box<dyn EditCommand>;
```

- Animate-mode routing creates the timeline on demand **and** a `t=0` setup-equal key first when
  missing (existing T-202 rule, moved into the router so every intent inherits it).
- With `auto_key = false` in Animate mode the router returns a *preview-only* result: the pose is
  overridden for display, nothing is committed until the user presses `K` (key selected) or
  `Shift+K` (key all changed properties).

**Structural gate.** `EditCommand` gains `fn requires_mode(&self) -> Option<WorkMode>` (default
`None` = allowed in both). Structural commands — create/delete/reparent/rename bone, create/delete
slot, skin add/remove, constraint add/remove, asset import, mesh topology change — return
`Some(Setup)`. `History::push` rejects a mode-violating command with a status-bar message instead of
mutating; this makes the invariant testable rather than a UI convention.

**UI**
- Mode switch in the toolbar: two segmented buttons (`SETUP` / `ANIMATE`), shortcut `Tab`.
- Animate mode: 2px accent viewport border (replaces T-202's red auto-key border), the animation
  name + frame shown in the viewport corner, timeline panel expanded; Setup mode collapses the
  timeline to its header and greys the transport.
- Setup mode: the rig draws in setup pose with a "SETUP POSE" watermark chip so a user can never
  mistake it for frame 0 of an animation.
- Tool availability per mode: Create Bone = Setup only; Weight Paint = Setup only; Select/Transform
  = both; Mesh edit (T-401) = Setup for topology, Animate for deform (T-404).
- Inspector fields show a **key affordance** in Animate mode only (T-210).

**Accept:**
- Unit: routing a `BoneLocal` intent in Setup produces `SetBoneTransformCommand`; in Animate at
  t=0.4 produces a key command on the active animation and leaves `Skeleton` byte-identical
  (assert setup skeleton equality before/after).
- Unit: a structural command pushed in Animate mode is rejected and the document is unchanged.
- Unit: Setup-mode evaluation with a non-zero playhead equals `evaluate(skel, &[], …)`.
- Manual: pose a bone in Setup, switch to Animate, pose again at frame 12, switch back — the Setup
  pose is exactly what was authored, not the animated value.
- `editor/src/state.rs` deleted; `AppState` holds only `{ doc, session, pose, history }`.

### ✅ T-208 ∥ Animation manager
**Deps:** T-207 · **Refs:** PLAN §2.7, §5 F-8

> **Shipped.** `Animation.looping` in core + schema (defaults true, so older files keep the
> behavior they had); `RenameAnimation` (refuses a taken name rather than making one clip
> unreachable on disk), `DuplicateAnimation` (deep copy, `_2` suffix), `SetAnimationMeta`
> (duration + loop, merged per drag), reusing the existing `DeleteAnimation`. UI is a `⋯` menu
> beside the clip picker: name, duration **in frames**, Loops, Duplicate, Delete. Duplicating
> selects the copy; deleting the last clip drops back to Setup mode.
>
> Duration is authored in frames because that is the unit the ruler, the dopesheet and the user
> share — seconds are the storage detail.
>
> Note: the transport's loop button stays a *preview* toggle, independent of the clip's `looping`
> flag, which is authoring intent for the runtime. Worth revisiting if that split confuses anyone.
- Animation list panel (dockable, or a combo + manager modal): create, rename, duplicate, delete,
  reorder; per-animation `duration` (seconds, displayed as frames at project fps) and a loop hint
  flag used by the runtime (T-604).
- Duration edit is non-destructive: shortening keeps out-of-range keys but T-702 flags them.
- Selecting an animation sets `Session.active_animation` and switches to Animate mode.
- `AnimationCommands` in `commands/anim_cmds.rs`; duplicate performs a deep clone with a fresh name.
**Accept:** create → key → duplicate → edit the copy leaves the original untouched (asserted in a
test); delete + undo restores every timeline; save/load round-trips all animations and durations.

### 🟡 T-209 ∥ Clipboard: copy / paste / duplicate
**Deps:** T-207 · **Refs:** PLAN §3.2

> **Bones and poses shipped.** `editor/src/clipboard.rs` (`Clipboard`, `BoneClip`, `PoseClip`),
> `PasteBones` command, `AppState::{copy_selection, copy_pose, paste, duplicate_selection}`,
> Edit-menu entries and Ctrl+C / Ctrl+Shift+C / Ctrl+V / Ctrl+Shift+V / Ctrl+D.
> * Copying a bone takes its **whole subtree** plus its slots and their skin entries — pasting half
>   a limb would be worse than refusing. Skins are matched by name on paste.
> * Poses are captured by **bone name**, so undo/redo id churn cannot land a pose on the wrong bone.
> * Pasting a pose routes through the mode: a setup edit in Setup, keys at the playhead in Animate —
>   which is exactly the copy-frame-A-to-frame-B workflow.
> * Paste-mirrored negates X translation, rotation and shear; scale is untouched, because mirroring
>   means "the same pose facing the other way", not a reflection of the art.
>
> **Still to do:** copying a *key selection* from the dopesheet (that selection lives in egui memory
> inside `sheet.rs`, so it wants a context-menu entry there rather than a global shortcut).
- `editor/src/clipboard.rs` with a typed payload enum: `Bones(subtree)`, `Keys(Vec<KeyRef>)`,
  `Pose(SecondaryMap<BoneId, Transform>)`.
- Setup mode: copy/paste a bone subtree (deep clone of bones, slots, skin entries for those slots;
  names deduped `_2`), paste-as-child-of-selection; `Ctrl+D` duplicates in place.
- Animate mode: copy/paste selected keys (paste at playhead, preserving relative times; cross-bone
  paste maps by row order); **paste mirrored** (negate X translate, negate rotation) as an explicit
  menu entry — the single most-requested walk-cycle tool.
- Copy/paste **pose** between animations or frames (`Ctrl+Shift+C` / `Ctrl+Shift+V`).
**Accept:** paste-mirrored on a 4-bone arm produces the mirrored pose (numeric test); every paste is
one undo step; clipboard payloads survive an animation delete without dangling ids (resolve by name).

### 🟡 T-210 ∥ ★ Keyed-value affordances in the inspector
**Deps:** T-207 · **Refs:** PLAN §7

> **Dots shipped.** `edit_router::{KeyState, key_state, bone_key_value}` plus a dot on every
> keyable inspector row (four bone transform channels + slot colour), shown in Animate mode only:
> empty outline = no timeline, hollow ring = timeline but unkeyed here, filled = keyed at the
> playhead, amber = posed but uncommitted. Click keys the value the viewport is showing;
> alt-click removes the key that is here. `bone_key_value` is shared with the timeline's set-key so
> the two can never disagree about what "key this" means.
>
> **Still to do:** the right-click menu (revert to setup value / revert to animated value) and the
> Setup-mode "reset to default" affordance.
- In Animate mode every keyable inspector field gets a state dot: **empty** (no timeline),
  **hollow** (timeline exists, not keyed at playhead), **filled** (keyed here), **amber** (value
  differs from the sampled animation value = unkeyed edit pending).
- Click = key this property at playhead; alt-click = delete the key; right-click menu = "revert to
  setup value", "revert to animated value".
- Setup mode shows a plain "reset to default" affordance instead.
**Accept:** dot states are covered by a unit test over a fabricated timeline; keying via the dot and
via the dopesheet produce the identical command.

### ✅ T-211 ∥ Pose tools
**Deps:** T-207 · **Refs:** PLAN §2.6

> **Shipped** as a top-level **Pose** menu, acting on the selection or the whole rig when nothing
> is selected. `SetPoseAsSetup` and `ResetBones` (bone_cmds), `RetimeAnimation` (scale/offset) and
> `ClearBoneAnimation` (key_cmds).
> * Baking writes what the *viewport* shows, in-flight drag preview included — pose until it looks
>   right, then make that the rest pose.
> * Reset clears rotation/scale/shear but **keeps position**: a bone's offset from its parent is
>   rig structure, and collapsing the skeleton onto the origin is never what "reset" means.
> * Retime snapshots the timelines instead of inverting the mapping — a key clamped at zero cannot
>   be un-clamped, so an arithmetic inverse would silently lose keys a negative offset pushed off
>   the start.
> * Clearing drops only *bone* timelines for the selection; a slot's colour animation is not the
>   bone's to take away.
>
> Menu offers fixed retimes (half/double speed, ±1 frame) rather than a numeric field; an arbitrary
> factor wants a small dialog, which is worth doing once the settings modal exists (T-701).
- Setup mode: "set current pose as setup pose" (bakes the visible pose into `Skeleton`, with a
  confirm), "reset bone/selection/skeleton to setup".
- Animate mode: key-all-changed (`Shift+K`), "clear animation from selected bones", "offset all keys
  of the selection by N frames", "scale animation timing by factor" (retiming, one command).
**Accept:** retiming a 30-frame animation by 2.0 doubles every key time and the duration; undo
restores exactly; setup bake is a single undoable command.

---

## Phase 3 — Assets & import pipeline

> T-301 → (T-302 ∥ T-303 ∥ T-305 ∥ T-306); T-304 anytime after T-108.
> This phase unblocks everything visual: today the editor renders bones and gizmos only.

### ✅ T-301 Asset database + image drop-import
**Deps:** T-108, T-207 · **Refs:** PLAN §3.2, §6.1

> **Shipped.** `core/src/assets.rs` (`AssetDb`, `ImageAsset`), `assets` in the `.ankh` schema +
> container binding, `editor/src/commands/asset_cmds.rs`, a textured sprite pipeline in the wgpu
> renderer, viewport drop-import, and an Assets panel.
>
> Three decisions differ from the sketch below, each deliberate:
> * **Attachments reference assets by name, not `AssetId`.** Names are already the on-disk
>   reference (ADR 0004); an id in memory and a name on disk would need a resolution pass on every
>   load and give two sources of truth. `RenameAsset` rewrites referencing attachments.
> * **The GPU texture cache is keyed by a content hash, not `AssetId`.** Slotmap keys are recycled
>   when a document closes, so an id-keyed cache draws the previous project's pixels into the new
>   one. Hashing also de-duplicates identical images.
> * **Sharing one asset between attachments** is not possible yet — "Attach" re-imports the pixels
>   under a uniquified name. Real sharing wants an id-carrying attachment, which lands with T-401.
>
> Not done here, deliberately deferred: texture eviction on document change (memory only —
> T-706), thumbnail grid layout and search (the panel is a list), asset reload/relink (T-306).
- `Document.assets: AssetDb` — `SlotMap<AssetId, ImageAsset { name, bytes, size, source_path:
  Option<PathBuf> }>`; images stored verbatim in the `.ankh` zip under `images/` (no re-encode);
  `formats` schema gains an `assets` array keyed by name + a `Region.texture` → asset-name link.
- Editor renderer: GPU texture cache keyed by `AssetId`, uploaded lazily, evicted on document
  change; extend `renderer/custom_renderer.rs` to draw region attachments (textured quads) sorted by
  `Pose.draw_order`, with per-slot tint from `Pose.slot_colors` and premultiplied alpha.
- Drag-and-drop PNG/JPG/WebP onto the canvas (Setup mode only): creates asset + slot on the selected
  bone (or root) + region attachment in the default skin, positioned at the drop point.
- Assets panel: thumbnail grid, search, rename, delete-with-usage-count warning, drag onto a bone
  row or the canvas to attach.
**Accept:** drop 3 PNGs → 3 textured slots render in draw order; save/reopen preserves bytes
(hash-compared in a test) and pixels; deleting an in-use asset warns and is undoable; dropping in
Animate mode is refused with the mode hint.

### T-302 ∥ PSD import (F-2)
**Deps:** T-301 · **Refs:** PLAN §5 F-2, §0
- `formats/src/psd.rs` using the `psd` crate. Our documented mapping (`docs/psd-import.md`):
  layer group → bone (nested → hierarchy); image layer → slot + region attachment placed from layer
  bounds; layer named `$pivot` in a group → that bone's origin; group prefix `$ik ` → IK constraint
  scaffold over the chain; top-level group `@skin:<name>` → entries land in skin `<name>`.
- Import modal: preview tree with per-group include checkboxes, "flatten group to one attachment"
  toggle, target-scale field, and a summary of what will be created.
- **Re-import over an existing document**: match by layer path → replace asset
  bytes and attachment size, keep bones/animations. Report added/removed/changed layers.
**Accept:** checked-in CC0 test PSD imports to the documented structure (bone/slot/skin counts
asserted); the whole import is one undoable command; re-import of a modified PSD keeps animations
intact (pose-diff test).

### ❌ T-303 — foreign armature importer *(removed)*
Built and then deleted. Ankhimate's format is `.ankh`; carrying an importer for another editor's
container meant maintaining a second format, its sample fixtures, and its licence questions for a
migration path nobody had asked for. The document model and the import-summary plumbing it exercised
are kept and reused by T-302.

If a foreign importer is ever wanted again, it belongs behind a feature flag with its own fixtures,
and the clean-room rule from ADR 005 applies: formats are read from sample **data files**, never from
another tool's source.

### ✅ T-304 ∥ ★ Startup window (part of F-15)
**Deps:** T-108 · **Refs:** PLAN §5 F-15
- On launch with no project: centered panel — New Project, Open, recent files (persisted to the
  platform config dir via `directories`), sample projects from `samples/`, and links to docs.
- Recents show name, path, and last-opened; missing files are greyed with a "locate" action.
**Accept:** recents survive restart; opening a sample works from a clean checkout; a deleted recent
does not crash the launcher.

### 🟡 T-305 ∥ ★ Atlas / spritesheet import modal (F-18) *(was T-404)*
**Deps:** T-301 · **Refs:** PLAN §5 F-18
- Import a sheet: grid slicer (rows/cols/margin/spacing) **or** manual rect list with numeric
  L/T/W/H fields and drag handles on a zoomable preview; auto-name cells (`sheet_00`…) with inline
  rename; each cell is cropped into its own `ImageAsset` for v1.
- Detect-cells helper: flood-fill on alpha to propose rects, user accepts/edits.
**Accept:** a 4×4 sheet imports into 16 assets with pixel-exact crops (golden test); manual rects
round-trip through the modal; cancel leaves the document untouched.

### ✅ T-306 ∥ Asset relink + external-source workflow
**Deps:** T-301 · **Refs:** PLAN §6.1
- Track `source_path` per asset. "Reload from source" (single/all) and "Relink…" when the path is
  gone; a status-bar badge counts stale assets (source mtime newer than the embedded bytes).
- Optional per-project setting "watch sources" (poll on window focus, not a filesystem watcher —
  keeps the dependency surface small).
**Accept:** editing a PNG on disk and pressing Reload updates the viewport without touching the rig;
relinking a moved file clears the stale badge; unit test on the staleness rule.

---

### ✅ T-307 Attachment inspector — per-attachment properties
**Deps:** T-301 · **Refs:** PLAN §2.4

> **On-canvas mode shipped too.** `Session::edit_target` (`EditTarget::{Bone, Attachment}`) with an
> "Edit on canvas" toggle in the Attachment section: the transform tools then drive the artwork via
> the pivot crosshair — translate/rotate/scale about the pivot, Alt-drag to move the pivot itself
> (art staying put), quad outlined while active. Explicit rather than inferred from the selection,
> because "drag the bone" and "drag the art" are the same gesture and a stray slot click must not
> change what it means. Setup-only; `Deform` keys (T-404) are how the same geometry is animated.
>
> Unlike bone drags this writes each frame instead of staging a preview — `SetRegionProps` merges,
> so the drag is still one undo step, and it matches what the inspector spinboxes already do.
>
> `commands/attachment_cmds.rs`
> (`SetRegionProps` merged per drag, `RenameAttachment`, `DuplicateAttachment`,
> `RemoveAttachment`, `owning_skin`) plus the Properties▸Attachment section: name, rotate, offset,
> scale, size, **pivot**, reset-to-image-size, duplicate, remove. Edits land in the skin the
> attachment *resolved from* — active, else default — so changing a value never silently forks an
> override into a skin the user is not looking at.
>
> **Pivot** (`RegionAttachment.pivot`, normalized, `(0,0)` bottom-left, default centre): the point
> the image rotates and scales around. Quad placement moved into
> `RegionAttachment::local_corners()` so the viewport, exporter and runtime cannot disagree about
> it. Changing the pivot compensates the offset, so the art does not jump while you hunt for the
> right point, and a crosshair marks the pivot on canvas for the selected slot. Nine-point presets
> (corners/edges/centre) beside the numeric fields.
>
> **Still to do:** the viewport mode where the transform tools drive the attachment instead of the
> bone. That is a second interaction surface (gizmo retargeting + a way to say which of the two you
> mean), not a variation on the panel, so it is left as its own piece of work.

Region attachments carry their own transform (`local_offset`, `local_rotation`, `local_scale`,
`width`/`height`, `uv_rect`) and nothing can edit it: art can only be moved by moving its bone,
which drags the rig around to fix a placement problem. This is the gap between "images render" and
"images can be authored".

- Inspector section for the **selected slot's resolved attachment**, in the active skin: name
  (renameable), offset X/Y, rotation, scale X/Y, size, and a "reset to image size" action that
  restores `width`/`height` from the asset's pixels.
- Editing writes to the attachment **in the active skin** (never the slot, never another skin) via
  `commands/attachment_cmds.rs`; drag-merged like the bone transform so one gesture is one undo step.
- Slot-level properties that are not the attachment stay in the slot section: name, bone, color.
- Attachment actions: rename (rewrites the slot's `attachment` name and any `SlotAttachment` keys
  that referenced it), duplicate under a new name in the same slot, remove from this skin.
- **On-canvas**: with a slot selected and no bone gizmo active, the transform tools operate on the
  attachment instead of the bone — the Spine "you are moving the image, not the rig" mode. Viewport
  shows the attachment's own axes so the target of a drag is never ambiguous.
- Setup-only for the transform (it is rig data, not animation); `Deform` timelines (T-404) are how
  the same geometry is animated.

**Accept:** nudging an attachment moves only that art, leaving bone and pose byte-identical
(asserted); rename updates the slot and its attachment keys and survives save/load; "reset to image
size" restores the imported dimensions after a manual resize; every edit is one undo step.

---

## Phase 4 — Mesh & deform

> (T-401 → T-402 ∥ T-403) → T-404 → T-405.

### ✅ T-401 Mesh attachment editing UI
**Deps:** T-301, T-207 · **Refs:** PLAN §2.4

> **Core editing shipped.** `MeshAttachment::from_region` (quad with the region's exact corners and
> UVs, so converting is invisible until a vertex moves), `editor/src/meshgen.rs` (Delaunay via
> `spade`), `commands/mesh_cmds.rs` (`ConvertToMesh`, `EditMesh` with move/add/remove/retriangulate),
> canvas vertex editing with wireframe + handles, and a Mesh section in the inspector.
> Mesh attachments now **render textured** — the sprite pipeline takes an index list instead of
> assuming a quad, so regions and meshes share one path.
>
> **A design note worth keeping:** the first triangulation pass tried to read the vertex list as an
> outline and trim triangles outside it. That is unsound — nothing distinguishes a perimeter vertex
> from an interior one, and adding a vertex inside a quad makes the list a *valid* pentagon with a
> notch, so the filter correctly carved away a triangle the user wanted. Concavity needs a real
> contour, which the tracer (T-402) supplies — or a user does, by pinning an edge. The hull remains
> the default; a pinned edge is the override.
>
> **Now complete.** Box-select landed on Ctrl+drag (a bare drag grabs a vertex in a dense mesh,
> which is exactly when a box is wanted). Manual edges are `MeshAttachment.edges`, honoured by a
> constrained Delaunay pass and toggled with `C` — Delaunay maximises the smallest angle, which
> happily bridges a notch, and a pinned edge is how a user says "not there". The UV pane
> (`ui/uv.rs`) drags where each vertex samples the texture, with a reset that re-projects from the
> mesh's current bounds.
>
> **Deliberately not done:** sharing one asset between attachments (the T-301 deviation). It wants
> an id-carrying attachment, which is a schema change, and belongs with the skin work in T-507
> rather than bolted onto mesh editing.
- "Convert to mesh" on a region attachment (command): 4-vertex quad with the same UVs.
- Mesh edit mode (Setup): wireframe overlay, drag vertices (snap modifier), add vertex on edge
  (click), delete vertex (`X`), rectangle-select vertices, edges recomputed via constrained
  triangulation (`spade`), manual edge add/remove for hulls the auto-triangulation gets wrong.
- Minimum-validity guards with clear messages (a mesh needs ≥3 vertices / ≥1 triangle).
- Edits write to the attachment **in the active skin** via commands; UV editing pane beside the
  texture (drag verts in UV space, "reset UVs to bounds").
**Accept:** quad → pentagon round-trips through save/load; every vertex operation is one undo step;
topology edits are refused in Animate mode with the mode hint.

### ✅ T-402 ∥ Auto mesh tracing (F-3)
**Deps:** T-401 · **Refs:** PLAN §5 F-3, §0
- `editor/src/meshgen.rs`: alpha threshold → marching-squares outline (outer **and** inner contours)
  → Douglas-Peucker simplify (tolerance slider) → constrained Delaunay → drop triangles whose
  sampled texels are fully transparent. Optional interior point grid (density slider) and an
  outward padding value so edge texels are not clipped.
- "Trace from image" in mesh mode with live preview and a vertex/triangle-count readout; applying is
  one command that replaces the topology (and remaps existing weights by nearest-vertex, warning
  that deform keys for this attachment will be invalidated — offer to delete or keep them).
**Accept:** unit test on a checked-in donut PNG (inner contour preserved); a 512² sprite traces in
<100 ms release; tracing a weighted mesh does not silently drop weights.

### ✅ T-403 ∥ Weight painting + bind compensation (F-4)
**Deps:** T-401, T-102 · **Refs:** PLAN §5 F-4
- Port `ui/canvas/tools/weight_paint.rs` to the post-T-101 model (BoneId, skin-resolved mesh,
  commands with stroke merging). Brush modes add / subtract / smooth / replace, radius + strength +
  falloff curve, per-bone color overlay, "show weights as heat map" toggle, normalize on stroke end.
- Auto-bind helpers: bind selected vertices to selected bone with rigid weight; **heat/distance
  auto-weight** for the whole mesh against the bone chain (documented formula, deterministic).
- Bind compensation: when a vertex's bone set changes, recompute inverse binds so the deformed
  position at the current pose is unchanged.
**Accept:** painting never visibly jumps the mesh (pose-diff test across a rebind); one stroke = one
undo step; auto-weight on the sample arm produces smooth falloff (numeric test on a few vertices).

### ✅ T-404 Deform (FFD) timelines *(was T-405)*
**Deps:** T-401, T-106, T-207 · **Refs:** PLAN §2.7 `Timeline::Deform`, §2.6 `Pose.deforms`
- In Animate mode, vertex edits in mesh mode route (T-207) to `Deform` keys — offsets from the setup
  vertices; renderer applies `Pose.deforms` before weight skinning (setup + deform → skin).
- Dopesheet row per deformed attachment; deform keys are interpolatable (linear/bezier) and mix
  across animations by alpha.
- Guard: a deform timeline is invalidated if the attachment's vertex count changes (T-402) —
  detected on load and reported by T-702.
**Accept:** waving-flag animation from 3 deform keys; mixes correctly with bone motion; save/load
round-trip; vertex-count mismatch surfaces as a diagnostic rather than a panic.

### ✅ T-405 Clipping attachments (masking)
**Deps:** T-401 · **Refs:** PLAN §2.4 (`ClippingAttachment`)
- `Attachment::Clipping { vertices, end_slot }` in core + schema; the renderer masks the slot range;
  authoring is `ui/canvas/tools/clip_edit.rs` with the same gestures as mesh editing; runtime
  (T-604) will reuse `core::clipping` directly.
- **Not a stencil pass.** The editor renders inside egui's own render pass, which has no
  depth-stencil attachment — taking one would mean rendering the viewport to a private texture
  first. `core::clipping` cuts the triangles instead (ear-clip the polygon, Sutherland-Hodgman each
  piece, interpolate UVs at the cut). That is exact rather than sampled, costs nothing at draw
  time, and is what the runtime has to do anyway: it emits triangle batches to whatever renderer
  the game brought, and cannot assume a stencil buffer exists there either. One implementation,
  two consumers, no way for them to disagree about where a mask's edge falls.
**Accept:** a character behind a clipped window renders masked in the editor and in the runtime
example; disabling the clipping attachment restores the full draw.

---

## Phase 5 — Rig power (constraints & animation depth)

> Everything here is what separates "keyframes work" from "professional rig". All ∥ except T-504
> which touches the IK solver T-104 wrote.

### ✅ T-501 ∥ Transform constraint
**Deps:** T-104 · **Refs:** PLAN §2.5
- `Constraint::Transform { target: BoneId, bones: Vec<BoneId>, offsets: Transform, mix_rotate,
  mix_translate, mix_scale, mix_shear, local: bool, relative: bool }`, applied in
  `constraint_order`. Per-mix timelines (`TransformConstraintMix`).
- Inspector: pick target, mix sliders, offset, local/relative toggles, delete. Listed on the
  **driven** bone rather than the target — "why is this bone moving on its own" is the question a
  rigger asks, and the answer is whichever constraints write to it.
- This is also the first constraint authoring UI of any kind; IK constraints had none.
**Accept:** a "look at" setup — head follows a target bone with mix 0.5 — matches hand-computed
angles; constraint order changes the result deterministically; save/load round-trip.

### ✅ T-502 ∥ Path attachment + path constraint
**Deps:** T-401 · **Refs:** PLAN §2.5
- `Attachment::Path { vertices, closed, constant_speed }` authored with the clip polygon tools,
  which already had the right gestures — a path is simply an *open* ring.
- **Deferred:** bezier control points. The stored shape is the flattened polyline, because a curve
  nobody can measure is no use to a constraint and flattening at author time means the editor and
  the runtime walk identical geometry. Bezier handles would be an authoring convenience over the
  same data, and belong with the curve editor (T-704).
- `Constraint::Path { slot, bones, position, spacing, mix_rotate, mix_translate }`. The mode enums
  collapsed into `PathAttachment::constant_speed` (distance spacing vs vertex-index spacing) — two
  behaviours, and naming them as modes implied more than exist.
- **Deferred:** `PathPosition`/`PathSpacing` timelines. Position is the one worth animating (it is
  what slides a tread); it lands with the next timeline pass.
- Covers mesh "pathing" binds as a strictly more general feature (tails, treads, belts,
  vines following a spline).
**Accept:** a 5-bone tail follows a curved path with even spacing; animating path position slides
the chain along the curve; unit test on arc-length sampling at constant speed.

### ✅ T-503 ∥ Physics constraint (F-13)
**Deps:** T-104 · **Refs:** PLAN §2.5, §2.6 determinism rule
- `Constraint::Physics { bone, inertia, strength, damping, mass, wind, gravity, mix, x/y/rotate/
  scale channels }` — sway/bounce for hair, tails, cloth, chains.
- **Determinism:** `evaluate` stays pure. Physics state lives in a caller-owned
  `PhysicsState` passed in (`evaluate_with(skel, anims, &mut physics, dt, out)`); `evaluate` with no
  physics state applies the constraint's rest result. Editor advances it per frame; exporters
  advance it at export fps; the runtime owns one per instance. Document in ADR `0007-physics.md`.
- Editor: "reset physics", a global pause, and simulate-in-Setup toggle so a rigger can tune values
  without an animation.
**Accept:** a 4-bone tail settles to rest in <2 s with damping 0.5 (numeric test); two runs with the
same dt sequence produce identical output; export at 30 fps and playback at 30 fps match.

### ✅ T-504 IK completeness
**Deps:** T-104, T-207 · **Refs:** PLAN §2.5, §5 F-1
- Implement the reserved `softness` and `stretch` fields; chains longer than 2 via FABRIK
  (documented iteration count/tolerance, deterministic).
- Keyable IK properties beyond mix: `bend_direction` (stepped), `softness`, `stretch` — timelines
  `IkBendDirection`, `IkSoftness` (other editors key constraint direction and IK mode; we cover
  both with typed timelines).
- Editor: select a chain (shift-click in the Hierarchy) → "Create IK target" makes the target bone
  at the chain's tip and the constraint, as one undoable step. Inspector section per constraint:
  target picker, mix, softness, stretch + limit, flip bend.
- **Deferred:** viewport target/chain overlays and the T-210 key dots on these fields. The
  constraint is authorable and animatable without them; they are polish, and belong with T-708.
**Accept:** a 3-bone chain reaches its target within tolerance in ≤N iterations (test); stretch
extends bone length only up to the configured limit; flipping bend direction mid-animation produces
a stepped, flip-free result.

### ✅ T-505 ∥ Slot & bone presentation depth
**Deps:** T-301, T-207 · **Refs:** PLAN §2.3, §2.7
- Slot `blend_mode` (Normal / Additive / Multiply / Screen) honored by the renderer and exporters;
  `dark_color` (two-color tinting) plumbed through schema → pose → renderer → runtime.
- **Animatable visibility:** `Timeline::SlotVisible { slot, keys: Vec<Key<bool>> }` (stepped) plus
  `Pose.slot_visible`. Covers a `Hidden` keyframe element without overloading alpha.
- Bone group color already in the schema — surface it in the tree/viewport (colored bone widgets,
  inherited by children unless overridden).
**Accept:** additive-blended flash effect renders identically in editor and runtime; a visibility
key hides a slot at frame 10 and back at 20 with no interpolation artifacts; round-trips.

### ✅ T-506 ∥ Event timeline
**Deps:** T-106 · **Refs:** PLAN §2.7 (`events`, post-v1 → promoted)
- `Animation.events: Vec<EventKey { time, name, int_value, float_value, string_value }>` +
  `Timeline::Event`; dopesheet row with named markers; runtime (T-604) reports events fired in a
  frame window, including during crossfades and looping wraps.
- Editor: a marker lane under the ruler — double-click to add, drag to retime, right-click to rename
  or delete. Events belong to the *clip*, not to a bone, so they get their own lane rather than a row
  in the dopesheet's group tree.
- `core::animation::events_in_window(anim, from, to, looping)` is the runtime contract: half-open
  `(from, to]`, wraps on loop, and fires every event once per lap when `dt` overshoots the clip.
- **Deferred:** a document-level event *definition* list (name + default payload, reused across
  clips). Events carry their payload inline today, which is enough to author and to fire; shared
  definitions are a convenience that wants the diagnostics pass (T-702) to flag typo'd names.
**Accept:** a footstep event fires exactly once per loop at the right time in a runtime unit test
(including when dt overshoots the loop boundary); events survive save/load.

### ✅ T-507 ∥ Skin manager + multi-skin composition
**Deps:** T-105, T-301 · **Refs:** PLAN §2.4, §5 F-14
- Skin panel: create/rename/delete/duplicate skins, per-slot attachment grid, drag an asset into a
  (slot, skin) cell, "copy attachments from skin".
- **Composition:** `Session.active_skins: Vec<SkinId>` (ordered, first match wins, default-skin
  fallback last) so outfits combine, as tools with a global "style" switch cannot. `resolve()`
  takes a slice; the single-skin call becomes a one-element slice.
- **Deferred:** the per-slot attachment grid and drag-an-asset-into-a-cell. The panel manages skins
  (create/rename/duplicate/delete/copy) and composition; per-cell assignment is the attachment
  inspector'''s job today and a grid is a second way to do it. Runtime/export bake of a composed skin
  lands with T-603.
**Accept:** hat-skin + armor-skin active together render both, with the first winning on conflict
(unit test on `resolve`); a skin rename updates every reference; export bakes the composed result.

---

## Phase 6 — Export & runtime

> T-601 → T-602; T-603 → T-604; T-605 last.

### T-601 Offscreen render + image / spritesheet / sequence export *(was T-501)*
**Deps:** T-106, T-301 · **Refs:** PLAN §5 F-9
- `export/`: headless wgpu render of evaluated poses at a chosen resolution/fps; PNG/JPG/WebP frames
  via the `image` crate. Spritesheet packer (sprites-per-row) + per-frame sequences + a JSON sidecar
  describing frame rects and timing.
- Global-bounds toggle (union AABB over all exported frames so every sprite is the same size),
  per-animation checkboxes with frame counts, background/clear color, padding, trim toggle.
- Physics-bearing rigs (T-503) advance their state at export fps.
**Accept:** exported sheet of the sample walk matches the viewport (golden image diff, small
tolerance); runs headless in CI; a 60-frame export of the sample completes in a documented budget.

### T-602 Video export via ffmpeg *(was T-502)*
**Deps:** T-601 · **Refs:** PLAN §0, §5 F-9
- Locate `ffmpeg` (config setting → PATH auto-detect → friendly error with a download link; never
  bundle or link — PLAN §0 allows process spawn only). Pipe frames over stdin (rawvideo) → MP4
  (encoder choice, bitrate/CRF, background color, loop cycles) and GIF (palettegen/paletteuse pass).
- Progress dialog with cancel; "open after export".
**Accept:** 2 s MP4 + GIF of the sample; cancel leaves no partial file; missing-ffmpeg path shows
guidance and never panics; stderr from a failed encode is surfaced verbatim in the dialog.

### T-603 Runtime export + atlas bake *(was T-503)*
**Deps:** T-106, T-301, T-507 · **Refs:** PLAN §5 F-10, §6.2
- `export/src/atlas.rs`: trim transparent borders, MaxRects/skyline pack (`crunch`), X/Y padding and
  extrude options, power-of-two toggle, multi-page output → `atlas.png` (+`_2`…) and `atlas.json`
  (regions by attachment name with trim offsets and rotation flag).
- `skeleton.json` = runtime schema (subset of `project.json`, editor-only fields stripped).
- Options: **bake IK** (sample constraints at export fps into bone timelines, then strip
  constraints), exclude-IK, bake physics to keys, flatten composed skins, strip unused assets.
**Accept:** the exported folder is self-contained; baked-IK export replays identically to
constraint-live evaluation (pose diff at 10 sampled times, ε documented); multi-page atlas loads in
the runtime example.

### T-604 `ankhimate-runtime` crate + reference integration *(was T-601)*
**Deps:** T-603 · **Refs:** PLAN §3.1, §6.2
- Crate `runtime/`: load the runtime export, `AnimationState` (play / queue / loop / crossfade via
  `evaluate`'s alpha mixing, per-track alpha, playback speed), event dispatch (T-506), physics state
  ownership (T-503). Output = `Vec<DrawBatch { texture_id, blend, clip, vertices, indices }>` sorted
  by draw order. No wgpu dependency; compiles for `wasm32`.
- `examples/macroquad_player/`: loads the sample export, crossfades walk⇄idle on keypress, prints
  events. `docs/runtime-guide.md` walkthrough.
**Accept:** example runs from a clean checkout; crossfade visually smooth; runtime builds for wasm32
in CI; a headless test asserts runtime pose == editor pose for the sample at 10 times.

### T-605 Format spec finalization + migration policy
**Deps:** T-603 · **Refs:** PLAN §6, ADR 0004
- `docs/format-spec.md` generated/verified against the shipped serialization (normative field
  tables for `.ankh` and `.ankh.runtime`); document the unknown-field preservation rule and the
  version-bump/migration procedure; add a migration test harness (`formats/tests/migrations.rs`)
  with one checked-in file per historical version.
**Accept:** every documented field exists in code (a test walks the schema types); a v0-style file
fixture migrates and round-trips; CI fails if `CURRENT_VERSION` changes without a new fixture.

---

## Phase 7 — Production polish

> All ∥. None of these are optional for a 1.0 that competes with Spine/Spriter.

### T-701 Settings, keymap, autosave *(was T-505)*
**Deps:** T-108 · **Refs:** PLAN §5 F-15
- `editor/src/config.rs`: `Config` serialized as RON to the platform config dir (`directories`) —
  UI scale, theme + custom color overrides, grid gap/foreground toggle, pixel-art filtering mode,
  bone widget size, non-selected bone translucency, gizmo ring radii, autosave interval, onion-skin
  defaults, ffmpeg path, recent files, "skip startup window".
- Keymap: an `Action` enum covering every command/tool/transport/mode action; rebindable in a
  Settings modal (click row → press chord), conflict highlighting, reset-to-default per row.
- Autosave: timer writes `<name>.ankh.autosave` beside the project (temp dir if unsaved); recovery
  offer on startup when newer than the save.
**Accept:** rebinding undo to `Ctrl+U` works and persists; kill the process mid-edit → relaunch
offers recovery with intact data; corrupt config falls back to defaults with a status message.

### T-702 Diagnostics / warnings *(was T-506)*
**Deps:** T-108 · **Refs:** PLAN §5 F-16
- `editor/src/diagnostics.rs`, rules run debounced on document change: mesh vertex with zero total
  weight, unbound vertex group, slot with a dangling attachment name, keys beyond duration, empty
  animation, unused asset, duplicate draw-order entries, IK constraint with a missing target or a
  chain spanning disjoint roots, deform timeline with a stale vertex count, empty skin.
- Status-bar badge + popover list; clicking a row selects and frames the offender. Pre-export runs
  the same rules as blocking warnings with continue-anyway.
**Accept:** each rule has a unit test; the export modal surfaces an induced warning; the rule pass
over a 500-bone rig stays under the documented debounce budget.

### T-703 ∥ ★ Onion skinning *(was T-504)*
**Deps:** T-106, T-207 · **Refs:** PLAN §5 F-7
- Transport toggle: evaluate at playhead ±1..k frames (k configurable 1–5), render ghost passes
  tinted (past reddish, future greenish, from theme tokens) under the main pose; opacity falloff;
  "only selected bones" option; key-based stepping option (previous/next key instead of ±frames).
- Animate mode only.
**Accept:** ghosts visible while scrubbing; off = zero extra evaluations (counter assertion in a
debug build); ghost count matches k.

### T-704 ∥ Curve editor refinement *(was T-507)*
**Deps:** T-203 · **Refs:** PLAN §5 F-6
- `ui/timeline/graph.rs` exists — finish it: dopesheet ⇄ curve toggle, per-property value/time
  curves with draggable bezier handles (merged commands), per-property color coding, fit-to-view,
  value-axis numeric readout, multi-property overlay with a normalize toggle, handle-tie modes
  (broken / aligned / mirrored), box-select on curve points.
**Accept:** dragging a handle changes easing live during playback; undo restores handles exactly;
switching modes preserves the selection.

### T-705 ∥ Localization (i18n)
**Deps:** T-701 · **Refs:** PLAN §7
- `editor/src/i18n.rs`: string catalog keyed by dotted ids, loaded from embedded `en` plus optional
  JSON files in the config dir; `t!("bone_panel.name")` macro; a fallback-to-English switch and a
  "show keys" debug mode; language picker in Settings; paste-a-catalog import for translators.
- Sweep the UI for literals (a CI lint or `xtask i18n-check` listing untranslated string literals in
  `ui/`).
**Accept:** switching language re-renders every panel; a catalog missing keys falls back cleanly;
`xtask i18n-check` is green.

### T-706 ∥ Performance pass *(was T-508)*
**Deps:** Phase 2, T-301 · **Refs:** PLAN §7.7
- `xtask bench`: synthetic 500-bone / 200-slot rig; criterion benches for `evaluate` (<1 ms target),
  sampling, and atlas packing. Renderer: instanced bone widgets, batch attachments by
  (texture, blend mode), draw-order-stable sorting, texture cache reuse across frames.
- Timeline UI virtualization (only visible rows/keys drawn) — required once rigs have hundreds of
  timelines.
**Accept:** bench trend report in CI (non-gating); 60 fps with the synthetic rig on an integrated
GPU (recorded in the PR); scrolling a 300-row dopesheet stays above 60 fps.

### T-707 ∥ ★ Crash recovery, update check, feedback
**Deps:** T-701 · **Refs:** PLAN §5 F-15
- Panic hook writes a log next to the binary plus an emergency `.ankh` dump of the in-memory
  document; on next launch, offer to open the log and recover the dump.
- Optional update check against the releases API (off by default, explicit user action or an opt-in
  setting; never auto-download).
- "Send feedback" opens the issue tracker prefilled with version/OS — no telemetry, no silent
  network calls (state this in the privacy note in `README.md`).
**Accept:** an induced panic produces a recoverable dump; update check failure degrades to a
message; a fresh install makes zero network requests until the user asks.

### T-708 ∥ Viewport interaction polish
**Deps:** T-207 · **Refs:** PLAN §7
- Transform gizmo per tool (translate / rotate / scale / shear) with axis handles and rings, plus a
  contextual edit bar: snap X/Y, rotation snap step, aspect-ratio lock, and **pivot editing mode**
  (move/rotate/scale the attachment pivot rather than the bone — established editors have this and
  we currently do not).
- Grid rendering with configurable gap and front/back ordering, pixel-magnification mode for
  pixel-art rigs, ruler-free world-coordinate readout, frame-selection (`F`), zoom-to-fit,
  hide-others/isolate selection.
- Numeric entry on drag (type a value while dragging), and shift/ctrl modifiers documented in one
  place.
**Accept:** pivot edits change rotation center visibly and are undoable; snapping produces exact
multiples (numeric test on the transform helpers); every modifier is listed in the keymap UI.

### T-709 ∥ ★ Accessibility & window UX
**Deps:** T-701 · **Refs:** PLAN §7
- UI scale honored everywhere (no hardcoded pixel fonts), keyboard focus traversal through panels,
  tooltips on every icon-only button, colorblind-safe defaults for the weight heat map and
  onion-skin tints, unsaved-changes guard on close, window geometry/dock layout persistence,
  per-project title-bar path with a dirty marker.
**Accept:** the whole create-bone → key → export flow is reachable by keyboard; closing with unsaved
changes always prompts; layout survives restart.

---

## Phase 8 — Release

### T-801 ★ Samples + documentation *(was T-602)*
**Deps:** Phase 3, Phase 5 · **Refs:** PLAN §3.3
- 3 CC0 rigged samples in `samples/`: a simple 2-bone-limb character, a mesh/deform character, and
  one PSD source demonstrating the import conventions. README with gifs; a "first rig in 10 minutes"
  guide covering Setup → Animate explicitly; `docs/runtime-guide.md` cross-linked.
**Accept:** every doc snippet compiles (doctest or `xtask docs-check`); each sample opens
warning-free (T-702 clean) and plays.

### T-802 Release packaging *(was T-603)*
**Deps:** T-701, T-801 · **Refs:** PLAN §3.3
- `xtask package`: Windows (zip + optional `cargo-wix` installer), macOS (`.app` + dmg, notarization
  documented), Linux (AppImage). GitHub Actions release workflow on tag; version from workspace
  metadata shown in the title bar / About; changelog generated from the milestone.
**Accept:** a tag push produces three platform artifacts; a fresh Windows VM passes the smoke script
(open sample → play → export png).

### T-803 ∥ Web build (F-17, post-v1 gate)
**Deps:** T-604 · **Refs:** PLAN §3.1, §5 F-17
- Keep `core` + `runtime` wasm-clean in CI (already required). Editor wasm target: eframe web
  backend, file access via the browser file picker + OPFS for recents/autosave, ffmpeg export
  disabled with an explanatory message.
- Ship the runtime example as a web demo page for the project site.
**Accept:** the editor loads in a browser and can open a sample, rig, and animate; export paths that
cannot work are disabled rather than broken.

---

## Dependency overview

```
Phase 0: T-001 → T-002        T-003 (free)
Phase 1: T-101 → T-102 → T-103 → { T-104 ∥ T-105* } → T-106 → T-107 → T-108   (*T-105 only needs T-101)
Phase 2: T-108 → T-201 → { T-202 ∥ T-203 ∥ T-205 } → T-204 ; T-206 after T-107
         T-207 after T-107/T-202/T-204 → { T-208 ∥ T-209 ∥ T-210 ∥ T-211 }
Phase 3: T-207 → T-301 → { T-302 ∥ T-303 ∥ T-305 ∥ T-306 ∥ T-307 } ; T-304 after T-108
Phase 4: T-301 → T-401 → { T-402 ∥ T-403 } → T-404 → T-405
Phase 5: T-501 ∥ T-502 ∥ T-503 ∥ T-505 ∥ T-506 ∥ T-507 ; T-504 after T-104+T-207
Phase 6: T-601 → T-602 ; T-603 (after T-507) → T-604 → T-605
Phase 7: all ∥ (T-704 after T-203 ; T-706 after T-301)
Phase 8: T-801 → T-802 ; T-803 after T-604
```

Critical path to a usable 1.0: **T-207 → T-301 → T-401/T-403 → T-603 → T-604**. Everything else is
breadth. Max useful parallel agents: 2–3 in Phase 3, 4–5 from Phase 5 onward.

## Feature parity checklist

> The reference column is the feature set of established 2D skeletal editors, observed as behavior.

| Reference feature | Covered by |
|---|---|
| Bone hierarchy, drag reparent, lock/hide, fold, group color | T-206, T-505 |
| Textures on bones + z-index | T-301 (slots/attachments), T-204 (draw order) |
| Styles (multiple active, priority) | T-105, T-507 |
| PSD import + re-import | T-302 |
| Atlas import modal | T-305 |
| Per-attachment transform (offset/rotation/scale/size within the slot) | T-307 |
| Mesh deformation, vertex/triangle editing | T-401 |
| Mesh tracing (gap/padding) | T-402 |
| Vertex binds + weights | T-403 |
| Bind "pathing" | T-502 (path constraint, generalized) |
| Inverse kinematics (family, constraint dir, distance, mimic target) | T-104, T-504 |
| Bone physics (sway, damping, bounce, ratio) | T-503 |
| Dopesheet, presets, custom curves | T-201, T-203, T-704 |
| Keyframe elements: pos/rot/scale/zindex/texture/tint/hidden/IK | T-201, T-204, T-205, T-504, T-505 |
| Onion skinning | T-703 |
| Playback, fps, edit-while-playing | T-202, T-207 |
| Copy / paste / duplicate | T-209 |
| Export image / spritesheet / sequence | T-601 |
| Export video (ffmpeg, system or bundled) | T-602 |
| Export armature + atlas, bake/exclude IK | T-603 |
| Warnings system | T-702 |
| Settings: UI scale, colors, keymap, autosave, gridlines | T-701, T-708 |
| Startup window, recents, samples, update check | T-304, T-707 |
| Localization | T-705 |
| Crash log recovery, feedback | T-707 |
| Web build | T-803 |
| Camera x/y/zoom fields | T-708 (viewport readout); camera stays Session state by design |

Beyond parity (features the open-source alternatives lack): slot/skin model (T-105/T-507), deform
timelines
(T-404), clipping (T-405), transform + path constraints (T-501/T-502), events (T-506), two-color
tint and blend modes (T-505), curve editor (T-704), embeddable MIT/Apache runtime (T-604).
