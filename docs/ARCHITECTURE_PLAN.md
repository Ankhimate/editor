# Ankhimate — Architecture & Project Plan (v1)

> **Audience:** human contributors and AI agents implementing tasks. Every section is written to be
> actionable in isolation: type signatures, file paths, and acceptance criteria are normative unless
> marked *(suggestion)*.
>
> **Scope:** the Rust implementation in `ankhimate-rs/` (egui + wgpu editor). This supersedes the
> older Tauri/React plan in the parent repo for the editor itself.

---

## 0. Legal constraint — read first

Several existing 2D skeletal editors are **GPL-3.0**; the rest are proprietary. Ankhimate may
**replicate their feature set** but must **not copy or translate their source code** unless
Ankhimate itself adopts a GPL-compatible license.

**Policy (normative):**
1. All reference-derived work in this plan is **clean-room**: we describe *behavior* to implement,
   never code to port. Contributors must not paste, transliterate, or closely paraphrase another
   editor's source. File formats are reverse-engineered from sample **data files** and written up.
2. Decide the Ankhimate license up front. Recommendation: **MIT OR Apache-2.0** for
   `ankhimate-core` / `ankhimate-runtime` (so games can embed the runtime freely — this is a
   competitive advantage over GPL tools), and the same for the editor for simplicity.
3. FFmpeg integration must shell out to an external `ffmpeg` binary (system or user-downloaded),
   never link libav statically, to avoid (L)GPL linkage questions.

---

## 1. Current-state audit (`ankhimate-rs`)

### 1.1 Inventory

| Crate | Contents | Status |
|---|---|---|
| `core` | `math.rs` (Transform pos/rot/scale/shear, Mat4 compose, 2-bone IK + tests), `skeleton.rs` (Bone, IkConstraint, Skeleton), `slot.rs` (Slot, SlotId via slotmap, BlendMode), `attachment.rs` (Region + Mesh w/ weights, inverse-bind matrices, FFD), `animation.rs` (stub) | Foundation exists; several structural defects (below) |
| `editor` | eframe/egui 0.34 + wgpu 29, `egui_tiles` docking, JSON themes (catppuccin/nord/solarized), toolbar/tree/inspector/timeline panels, canvas w/ camera + tools (select, create_bone, weight_paint), custom wgpu renderer + WGSL shaders | Good shell; state management ad-hoc |
| `export` | `ProjectData` → JSON / bincode | Placeholder |

### 1.2 Defects vs. the required data architecture

These are the concrete gaps agents will fix in Phase 1 (§4):

- **D1 — Fragile bone identity.** `Bone.parent_index: Option<usize>`, `Slot.bone: usize`,
  `VertexWeight.bone_index: usize`, `IkConstraint.*_index: usize`. Deleting or reordering a bone
  silently corrupts every reference. World-transform update also relies on `Vec` order being
  topologically sorted (`update_world_transforms` iterates `0..len`) and `is_descendant` assumes
  `child_idx + 1..` ordering — an invariant nothing enforces.
- **D2 — Lossy world-transform math.** `core/src/skeleton.rs:109` composes 4×4 matrices then
  decomposes via `to_scale_rotation_translation()`. This throws away shear (comment at
  `skeleton.rs:127` admits it) and produces wrong results under non-uniform parent scale +
  child rotation. 2D skeletal math needs a 2×3 affine representation with explicit
  inherit-rotation/inherit-scale flags (Spine model), not quaternion round-trips.
- **D3 — IK blends world angles linearly.** `skeleton.rs:90` lerps raw radians
  (`p_rot * (1-mix) + p_angle * mix`) — breaks across the ±π wrap. Must blend via shortest-arc
  and be re-applied in *local* space so children propagate correctly.
- **D4 — No Skin layer.** `Skeleton.attachments: HashMap<SlotId, Attachment>` binds each slot to
  exactly one attachment permanently. Required model: slot holds a *name* reference; the active
  **Skin** resolves `(slot, name) → Attachment`. Nothing supports attachment swapping or
  alternate skins today.
- **D5 — Animation model is a stub.** `HashMap<String, AnimationCurve>` of `(time, f32)` pairs:
  no typed targets, no interpolation modes, no slot timelines (color/attachment), no **draw-order
  timeline** (a hard requirement), no events.
- **D6 — Undo = full JSON snapshots** (`editor/src/state.rs History`). O(document) memory per
  action, no action names/coalescing, and it only snapshots the skeleton — animations and slots
  will silently escape undo as the document grows.
- **D7 — Editor mutates the setup pose directly.** No separation between *setup pose* (document
  data), *animated pose* (evaluation result), and *tool previews*. Auto-keying and "edit while
  playing" are impossible without this split.
- **D8 — `bincode::Error` + serde on `SlotMap` in save format.** Serialized slotmap keys are
  unstable across versions; the on-disk format must use plain arrays with string/stable IDs, not
  serialized slotmaps.

---

## 2. Target data architecture (`ankhimate-core`)

This is the normative model. All remediation and features converge on it.

### 2.1 Identity

Use `slotmap` keys for **all** entities (bones too, not just slots) inside the in-memory document;
use stable string names only at the serialization boundary.

```rust
// core/src/ids.rs
slotmap::new_key_type! {
    pub struct BoneId;
    pub struct SlotId;
    pub struct SkinId;
    pub struct AnimationId;
    pub struct ConstraintId;
}
```

Rationale: O(1) lookup, stable across deletes, niche-optimized `Option<BoneId>`, serde-skippable.
On save, IDs are replaced by names (§6). `BoneId = usize` aliases are removed everywhere.

### 2.2 Bones — pure transform math

```rust
// core/src/skeleton.rs
pub struct Bone {
    pub name: String,
    pub parent: Option<BoneId>,
    pub length: f32,                    // editor visualization + IK
    pub local: Transform,               // SETUP pose (document data)
    pub inherit: Inherit,               // rotation/scale inheritance flags
    pub color: [f32; 4],                // editor-only tint of the bone widget
    pub icon: BoneIcon,                 // editor-only (see F-13 group colors)
}

pub struct Inherit { pub rotation: bool, pub scale: bool, pub reflect: bool }
```

**Bones own nothing visual and no draw order.** No `zindex`, no texture, no vertices on `Bone`
(this is precisely where the bone-centric model of other editors is rejected).

World transforms live **outside** the document in a `Pose` buffer (§2.6): a
`SecondaryMap<BoneId, WorldTransform>` recomputed every evaluation. `#[serde(skip)]`-style mixing
of document and derived state (current `Bone.world_transform`) is removed.

**Math:** replace Mat4 round-trips with a 2D affine:

```rust
// core/src/transforms.rs
#[derive(Clone, Copy)]
pub struct Affine2 { pub a: f32, pub b: f32, pub c: f32, pub d: f32, pub tx: f32, pub ty: f32 }
// world = parent.affine * compose(local)   — compose applies rot, scale, shear directly
// No decompose in the hot path. Decompose (a,b,c,d → rotation/scale/shear) exists only for
// editor gizmos and world→local conversions, implemented once, with unit tests for
// non-uniform-scale + rotation + shear cases.
```

Traversal order: maintain `Skeleton.update_order: Vec<BoneId>` (topologically sorted, rebuilt on
hierarchy edits), never rely on insertion order.

### 2.3 Slots & draw order

```rust
pub struct Slot {
    pub name: String,
    pub bone: BoneId,                       // exactly one bone
    pub attachment: Option<String>,         // NAME of attachment; resolved via active skin
    pub color: [f32; 4],
    pub dark_color: Option<[f32; 4]>,
    pub blend_mode: BlendMode,              // Normal | Additive | Multiply | Screen
}

pub struct Skeleton {
    pub bones: SlotMap<BoneId, Bone>,
    pub update_order: Vec<BoneId>,               // derived, rebuilt on edit
    pub slots: SlotMap<SlotId, Slot>,
    pub draw_order: Vec<SlotId>,                 // SETUP draw order — flat, explicit
    pub skins: SlotMap<SkinId, Skin>,
    pub default_skin: SkinId,
    pub constraints: SlotMap<ConstraintId, Constraint>,
    pub constraint_order: Vec<ConstraintId>,
}
```

- `draw_order` is the *setup* order. The **animated** draw order lives in the `Pose` (§2.6) as a
  reorderable copy, driven by draw-order timelines. Animating it never mutates `Skeleton`.
- Draw-order keyframes store **offsets** (`Vec<(SlotId, i32)>` — "slot moved +2 / −1 from setup"),
  matching Spine semantics: robust when slots are added later, and cheap to serialize.

### 2.4 Attachments & Skins

```rust
pub enum Attachment {
    Region(RegionAttachment),   // quad: offset/rot/scale/size + texture region
    Mesh(MeshAttachment),       // vertices, uvs, triangles, optional weights, edges (editor)
    Clipping(ClippingAttachment),   // post-v1
    Point(PointAttachment),         // post-v1 (spawn markers etc.)
}

pub struct Skin {
    pub name: String,
    /// (slot, attachment-name) -> attachment data
    pub entries: HashMap<(SlotId, String), Attachment>,
}
```

Resolution rule (the **only** way renderers obtain an attachment):

```rust
fn resolve<'a>(skel: &'a Skeleton, active: SkinId, slot: SlotId, name: &str) -> Option<&'a Attachment> {
    skel.skins[active].entries.get(&(slot, name.into()))
        .or_else(|| skel.skins[skel.default_skin].entries.get(&(slot, name.into())))
}
```

Consequences (normative):
- Animations may only change `slot.attachment` **names** (attachment timeline) — never attachment
  data. Swapping the active skin re-textures the whole character with zero animation changes.
- The existing `MeshAttachment` weight/FFD code in `core/src/attachment.rs` is kept conceptually
  but reworked: `bone_index: usize` → `BoneId`; inverse-bind `Mat4` → `Affine2`; FFD keyframes move
  out of the attachment into the animation model as **deform timelines** (data lives with
  animations, not assets).

### 2.5 Constraints

Generalize the current hardcoded 2-bone IK into an ordered constraint list applied after the FK
pass, in `constraint_order`:

```rust
pub enum Constraint {
    Ik(IkConstraint),           // target BoneId, chain Vec<BoneId> (1 or 2 bones), mix,
                                // bend_direction, softness, stretch: bool
    // Post-v1: Transform(TransformConstraint), Path(PathConstraint), Physics(PhysicsConstraint)
}
```

Fix D3: solve produces *local* rotations for chain bones; mix blends local rotations via
shortest-arc (`wrap_angle(target - current) * mix`); then descendants of the chain are re-run
through the FK pass. `PhysicsConstraint` (sway/bounce/orbit — F-13) is designed into this
enum now, implemented post-v1.

### 2.6 Pose & evaluation pipeline (the runtime contract)

```rust
// core/src/pose.rs — derived state, never serialized
pub struct Pose {
    pub locals: SecondaryMap<BoneId, Transform>,     // setup ⊕ animation
    pub worlds: SecondaryMap<BoneId, Affine2>,
    pub slot_colors: SecondaryMap<SlotId, [f32; 4]>,
    pub slot_attachments: SecondaryMap<SlotId, Option<String>>,
    pub draw_order: Vec<SlotId>,                     // animated order
    pub deforms: HashMap<(SlotId, String), Vec<glam::Vec2>>, // FFD vertex offsets
}

pub fn evaluate(skel: &Skeleton, anims: &[(&Animation, f32 /*time*/, f32 /*alpha*/)], out: &mut Pose);
```

Fixed pipeline order — **1)** copy setup pose into `Pose`; **2)** apply animation timelines
(possibly several, mixed by `alpha` — this gives free crossfades in the runtime); **3)** apply
constraints in `constraint_order`; **4)** compute world affines along `update_order`.

This function is the *entire* runtime contract: the editor viewport, the exporters, and the
shipping game runtime all call the same `evaluate`. Determinism requirement: identical inputs →
bit-identical `Pose` (no `Instant`, no global state in core).

### 2.7 Animation model

```rust
// core/src/animation.rs
pub struct Animation {
    pub name: String,
    pub duration: f32,                      // seconds; editor displays frames at project FPS
    pub timelines: Vec<Timeline>,
    pub events: Vec<EventKey>,              // post-v1: named triggers for runtimes
}

pub enum Timeline {
    // Bone timelines — each is Vec<Key<T>> sorted by time
    BoneTranslate { bone: BoneId, keys: Vec<Key<glam::Vec2>> },
    BoneRotate    { bone: BoneId, keys: Vec<Key<f32>> },      // degrees, shortest-arc interp
    BoneScale     { bone: BoneId, keys: Vec<Key<glam::Vec2>> },
    BoneShear     { bone: BoneId, keys: Vec<Key<glam::Vec2>> },
    // Slot timelines
    SlotColor      { slot: SlotId, keys: Vec<Key<[f32; 4]>> },
    SlotAttachment { slot: SlotId, keys: Vec<Key<Option<String>>> }, // stepped only
    // Skeleton timelines
    DrawOrder { keys: Vec<Key<Vec<(SlotId, i32)>>> },          // offsets from setup; stepped
    // Constraint timelines
    IkMix { constraint: ConstraintId, keys: Vec<Key<f32>> },
    // Deform (FFD)
    Deform { slot: SlotId, attachment: String, keys: Vec<Key<Vec<glam::Vec2>>> },
}

pub struct Key<T> { pub time: f32, pub value: T, pub interp: Interp }
pub enum Interp {
    Linear,
    Stepped,
    Bezier { out_handle: glam::Vec2, in_handle: glam::Vec2 }, // normalized 0..1 time/value space
}
```

- Bezier handles per-key (F-6) with named presets (`ease-in/out/in-out/sine/snap`) as
  editor-side factories that emit `Interp::Bezier` — presets are UI sugar, not a storage variant.
- Sampling: binary search per timeline + per-timeline `last_key` cache for sequential playback.
- Non-interpolatable timelines (`SlotAttachment`, `DrawOrder`) are stepped by construction.

---

## 3. System architecture

### 3.1 Crate graph (workspace)

```
ankhimate-rs/
  core/         ankhimate-core     — data model + evaluate(). Deps: glam, slotmap, serde. NO I/O,
                                     no image decoding, no egui/wgpu. #![forbid(unsafe_code)].
  document/     ankhimate-document — undoable document, named operators, shared read surface.
  formats/      ankhimate-formats  — .ankh read/write, atlas descriptors, importers (PSD, images),
                                     version migration.                                   [NEW]
  export/       ankhimate-export   — atlas packing and strict user-authored format templates.
  plugins/      ankhimate-plugins  — sandboxed QuickJS host over document operators.
  render/       ankhimate-render   — transport-free headless CPU renderer.
  editor/       ankhimate-editor   — egui/wgpu desktop app.
  mcp/          ankhimate-mcp      — stdio MCP consumer of document/plugins/render.
  xtask/                           — cargo xtask: CI checks, packaging, sample generation. [NEW]
```

Game runtimes live in their own multi-language repository. Its Rust reference
implementation consumes the public `core` and `formats` crates; JavaScript,
Phaser, Unity and future implementations consume the same exported format and
shared conformance fixtures.

Dependency rule: `core` is the leaf contract; editor never appears in another
crate's dependencies. `core` must compile for `wasm32-unknown-unknown`
(comparable tools ship web builds — we keep that door open; the editor targets native first).

### 3.2 Editor internal architecture

Replace ad-hoc `AppState` mutation with a **document / session / derived** split:

```
editor/src/
  doc.rs        Document { skeleton: Skeleton, animations: SlotMap<AnimationId, Animation>,
                           assets: AssetDb, meta } — the ONLY undoable state.
  session.rs    Session  { selection (multi!), tool, active_animation, playhead, camera,
                           active_skin, onion_skin cfg, auto_key: bool } — not undoable, not saved.
  commands/     Command pattern (fix D6):
                  trait EditCommand { fn apply(&mut self, doc: &mut Document) -> Result<()>;
                                      fn revert(&mut self, doc: &mut Document) -> Result<()>;
                                      fn merge(&mut self, next: &dyn EditCommand) -> bool; // coalesce drags
                                      fn label(&self) -> &str; }
                One file per command family: bone_cmds.rs, slot_cmds.rs, key_cmds.rs, skin_cmds.rs.
                Every mutation of Document goes through History::push(cmd). Drag interactions issue
                one command on mouse-up (preview via Pose overrides, not document edits — fixes D7).
  ui/           panels (existing layout kept): canvas/, tree.rs, inspector.rs, timeline/, toolbar.rs
  renderer/     wgpu scene renderer: consumes runtime batches + editor gizmo layer
```

**Frame flow:** input → commands mutate `Document` → `evaluate(doc.skeleton, active anim @
playhead, &mut pose)` → renderer draws pose → gizmos draw from pose + session. The viewport is a
pure function of `(Document, Session)`; this is what makes onion skinning (evaluate at N extra
times), export (evaluate offscreen), and "edit while playing" trivial rather than special-cased.

### 3.3 Contributor onboarding (open-source health)

- `docs/` (in-repo, normative): `architecture.md` (this doc, split), `format-spec.md` (.ankh v1),
  `runtime-guide.md`, `CONTRIBUTING.md` with a 15-minute "add a timeline type" walkthrough.
- ADRs in `docs/adr/NNN-*.md` for every decision in this plan (IDs, affine math, skin model,
  license). Agents implementing tasks must link the ADR they satisfy.
- CI (GitHub Actions): `cargo fmt --check`, `clippy -D warnings`, `cargo test` (all crates),
  `cargo test -p ankhimate-core --target wasm32-unknown-unknown` (build only), golden-file tests
  for `.ankh` round-trip, screenshot tests for the renderer via `wgpu` headless + image diff.
- `samples/` with 2–3 CC0 rigged characters used by tests and docs.
- Issue labels `good-first-issue` seeded from Phase tasks marked ★ in §7.

---

## 4. Codebase remediation plan (Phase 1 detail)

Ordered steps; each is a self-contained PR an agent can execute. **No step may break `cargo test`.**

| # | Task | Files | Notes / acceptance |
|---|---|---|---|
| R1 | Introduce `ids.rs`; migrate `Bone` storage to `SlotMap<BoneId, Bone>`; add `update_order` rebuild fn; replace `parent_index`, `Slot.bone`, `IkConstraint` indices, `VertexWeight.bone_index` | `core/src/{ids,skeleton,slot,attachment}.rs` | Deleting a bone must orphan-reparent children and drop dependent slots/constraints via one `remove_bone(id)` API with tests. Kills D1. |
| R2 | Replace Mat4 pipeline with `Affine2` compose; add decompose util + tests (non-uniform scale × rotation × shear); add `Inherit` flags | `core/src/transforms.rs` (new), `skeleton.rs` | Property test: compose→decompose→compose roundtrip ε<1e-4. Kills D2. |
| R3 | Extract `Pose` + `evaluate()`; remove `Bone.world_transform` and `#[serde(skip)]` derived fields from document types | `core/src/pose.rs` (new) | Editor compiles against pose; renderer reads pose only. Kills D7 (core half). |
| R4 | Constraint rework: `Constraint` enum, ordered application, local-space IK blend with angle wrapping | `core/src/constraints.rs` (new, absorbs ik parts of `math.rs`) | Test: mix=0.5 across ±π boundary has no flip. Kills D3. |
| R5 | Skin layer: `Skin`, `default_skin`, resolution fn; `Skeleton.attachments` map deleted; slot gets `attachment: Option<String>` | `core/src/{skin,slot,skeleton}.rs` | Renderer resolves via skin only. Kills D4. |
| R6 | New animation model per §2.7; delete old `animation.rs` stub; sampling + mixing with tests (golden curves) | `core/src/animation.rs` | Includes draw-order offset application into `Pose.draw_order`. Kills D5. |
| R7 | Editor: `Document`/`Session` split + command-pattern `History`; port existing tools (select/create-bone/weight-paint) to commands; drag = preview + single command on release | `editor/src/{doc,session,commands/*}.rs`, `state.rs` deleted | Undo depth 200 with O(command) memory; labels shown in Edit menu. Kills D6, D7 (editor half). |
| R8 | Save format v1 (§6) in `formats/`; delete bincode path; version field + migration scaffold | `formats/` (new crate), `export/src/lib.rs` shrinks | Round-trip golden tests. Kills D8. |

Everything currently working (docking UI, themes, camera, wgpu mesh renderer, weight painting,
two-bone solver math + its tests) is **kept** — remediation rewires ownership and math, it does not
restart the project.

## 5. Feature extraction from the reference tools

Clean-room feature matrix. Every row is *observed behavior* of an existing editor, never its code.
"Adaptation" states how the feature maps onto §2's architecture — in most cases *better* than the
bone-centric model these features come from.

| ID | Reference feature (observed behavior) | Adaptation into Ankhimate | Phase |
|---|---|---|---|
| F-1 | Textures attach straight to bones with per-bone `zindex` | **Rejected as-is.** Becomes slot + attachment + skin (§2.3–2.4). Any importer for a bone-centric container maps each textured bone → auto-created slot + region attachment in default skin; `zindex` sort → initial `draw_order`. | 1–2 |
| F-2 | PSD import: layer groups → bones, naming conventions (`$pivot`, `$ik_*`, style names), auto-flattening | High-value differentiator. Implement in `formats/src/psd.rs` using the `psd` crate. Mapping: layer group → bone; layer → slot+region attachment; `$pivot` layer → bone origin; `$ik_` prefix → IK constraint scaffold; PSD style-named groups → **skins** (cleaner than a global "styles" switch). Document our conventions in `docs/psd-import.md`. | 3 |
| F-3 | Auto **mesh tracing**: alpha silhouette → Delaunay triangulation, triangles validated against opaque texels | `editor/src/meshgen.rs`: marching-squares outline on alpha channel → simplify (Douglas-Peucker) → constrained Delaunay (`spade` crate) → drop fully-transparent triangles. Output = `MeshAttachment` replacing a Region in the active skin. | 4 |
| F-4 | Vertex weight binding with position-compensation when (un)binding | Already partially present (weight paint tool). Keep; add the UX nicety: rebinding recomputes inverse binds so the mesh doesn't jump. | 4 |
| F-5 | Dopesheet: per-bone rows, diamond keys, drag to move, drag-off to delete, multi-select (ctrl/shift), colored alternation, zoom 0.1–3.0, right-click menus | `editor/src/ui/timeline/dopesheet.rs`. Rows are grouped **bone → property** and **slot → property** (our model has slot tracks; bone-centric models cannot). Drag-off-to-delete kept — good UX. | 2 |
| F-6 | Bezier keyframe handles + presets (linear, sine in/out/in-out, snap, custom) | `Interp::Bezier` (§2.7) + preset palette in key context menu; add a **curve editor** lane (post-dopesheet) the alternatives lack polish on. | 2/5 |
| F-7 | Onion skinning toggle | Evaluate pose at `t ± k` frames, render tinted ghost passes. Cheap given §2.6. | 5 |
| F-8 | Playback: fps spinbox, elapsed-time based playhead, keyboard frame stepping, edit-while-playing (records edits as keys when enabled) | `Session.playhead` driven by wall clock at editor level (core stays deterministic). `auto_key` flag + "recording" red border. Locked bones (F-13) never record. | 2 |
| F-9 | Export: spritesheet (sprites/row), image sequence PNG/JPG, MP4/GIF via ffmpeg (bundled or system), resolution/background/loop options | `export/`: offscreen wgpu render of evaluated poses → `image` crate encode; ffmpeg driven via `std::process::Command` (§0). Export modal lists animations w/ frame counts, validation warnings. | 5 |
| F-10 | Runtime export: JSON + packed texture atlas (padding options), optional **IK baking**, IK exclusion | `.ankh` runtime bake (§6): atlas pack via `crunch` or skyline packer in `export/src/atlas.rs`; "bake constraints" = sample constraints into bone timelines at export fps then strip constraints. | 5 |
| F-11 | Undo with continued-action chaining; skip no-op undos | Superseded by command pattern (R7): `merge()` gives coalescing, explicit no-op detection in commands. | 1 |
| F-12 | Hierarchy panel: drag bone above target = sibling, below = child; position compensation so bones stay put on reparent; cascade delete; fold; lock; hide | `editor/src/ui/tree.rs` extension. Reparent = `ReparentBoneCommand` that rewrites `local` so world pose is unchanged (needs R2 decompose). Lock/hide/fold are editor flags on a `SecondaryMap` in `Session`… except `hidden` which is honest document state on `Slot`. | 2 |
| F-13 | Bone physics (sway, bounce, orbit, dampening) | `Constraint::Physics` post-v1 (§2.5). Not in v1 scope; reserve serialization space now. | post-v1 |
| F-14 | Styles (texture sets, one active) | Superseded by full **Skins** — strictly more general (per-slot dictionaries, not global texture lists). | 1 (R5) |
| F-15 | Settings: UI scale, keybinding remap (30+ actions), color/theme config, autosave frequency, startup window with recents/samples | Editor: keymap struct serialized to `config.ron` in platform config dir (`directories` crate); themes already exist; autosave timer writes `<project>.ankh.autosave`; startup window with recent projects + sample rigs. | 5 |
| F-16 | Warnings system (export validation etc.) | `editor/src/diagnostics.rs`: rules run on Document change (unweighted mesh verts, slot w/o attachment, keys past duration…), surfaced in a status-bar popover and pre-export check. | 5 |
| F-17 | Web/WASM build | Keep `core`/`runtime` wasm-clean from day one (CI); editor wasm build is post-v1. | post-v1 |
| F-18 | Atlas import modal (crop textures out of an existing sheet) | `formats`: atlas-grid slicer + manual rect editor in AssetPanel. | 4 |

## 6. File formats

### 6.1 `.ankh` project (editor save)

Zip container (like ora/docx — good for forward-compat):

```
project.ankh (zip)
├─ project.json      # version, name, fps, document (schema below)
├─ images/…          # original source PNGs (referenced by asset id)
└─ thumbs/cover.png  # optional, for startup window
```

`project.json` schema notes: entities keyed by **name strings** (`"bones": [{"name": "arm",
"parent": "shoulder", …}]`), never slotmap keys; `"version": 1` mandatory; unknown fields must be
preserved on round-trip where feasible (serde `flatten` capture). Migration lives in
`formats/src/migrate.rs` (every editor in this space grows a backwards-compatibility layer — plan
it from day 1).

### 6.2 `.ankh.runtime` (game runtime export)

`skeleton.json` (+ optional postcard binary twin) + `atlas.png` + `atlas.json`. Produced only by
export bake (F-10): trimmed, atlas-packed, optionally constraint-baked. `ankhimate-runtime` loads
this, not editor projects.

## 7. UI/UX strategy

**Stack decision: keep `eframe`/`egui` + `wgpu` + `egui_tiles`.** Rationale: already working,
immediate-mode fits a pose-is-a-pure-function editor (§3.2), egui demonstrably suffices for this
exact product, and Rust-native beats reintroducing a web stack for contributor coherence. Risks
accepted: egui text/i18n polish, breaking releases (pin versions; upgrade in dedicated PRs;
`egui_tiles = "*"` in `editor/Cargo.toml` must be pinned — do this in R7).

Productivity pillars (each is a roadmap acceptance item, ★ = good-first-issue candidates):

1. **Everything undoable, everything labeled** — command labels in Edit menu ("Undo Rotate 'arm_L'").
2. **Direct manipulation first**: rotate = drag bone body, translate = drag joint, no mandatory
   gizmo mode-switching; `R`/`T`/`S` temporary overrides while held (the temp edit-mode
   pattern, F-8); ★ numeric nudge with arrow keys.
3. **Auto-key mode** with unmistakable recording indicator; keying a non-animated property creates
   the timeline + setup-value key at t=0 automatically.
4. **Zero-modal flows** where possible; modals only for import/export/settings (F-15/F-9).
5. **Keyboard-complete**: every command reachable via remappable shortcut (F-15).
6. ★ **Empty-state guidance**: blank canvas shows "drop images here / open sample" affordances.
7. **60 fps floor** with 500 bones + 200 slots on integrated GPU: pose evaluate is O(entities),
   single instanced draw for bone widgets, one draw per blend-mode change for attachments.

## 8. Execution roadmap

Phases are strictly ordered; within a phase, tasks marked ∥ can run in parallel (agent-friendly).
"Exit" = demoable acceptance criteria.

| Phase | Content | Exit criteria |
|---|---|---|
| **0. Groundwork** (small) | License decision (§0) · pin deps · CI skeleton (§3.3) · ADRs 001–005 · `xtask` | CI green on empty PR; LICENSE committed |
| **1. Core remediation** | R1–R8 (§4) ∥ where files disjoint | `cargo test` covers ids/affine/IK-blend/skin-resolution/anim-sampling; editor still runs with ported tools |
| **2. Animation MVP** | Dopesheet (F-5) · playback + auto-key (F-8) · Bezier keys (F-6 storage + presets) · draw-order panel with animatable offsets · attachment/color slot keys · hierarchy panel UX (F-12) | Rig a 10-bone character from imported PNGs, animate idle+walk with eased keys and a draw-order swap, save/load `.ankh`, undo any step |
| **3. Import pipeline** | image drop-import ∥ PSD import (F-2) ∥ asset panel | Import a layered PSD → rigged skeleton with skins |
| **4. Mesh & deform** | mesh attachment editing UI · auto-trace (F-3) ∥ weight paint port + bind compensation (F-4) ∥ atlas import (F-18) · deform timelines | Deform a traced mesh with weights in an animation |
| **5. Polish & export** | exports (F-9, F-10) ∥ onion skin (F-7) ∥ settings/keymap/autosave/startup (F-15) ∥ diagnostics (F-16) · curve editor lane (F-6) · perf pass (§7.7) | Ship a game-loadable `.ankh.runtime` + atlas; render MP4; all v1 issues closed |
| **6. V1 release** | `ankhimate-runtime` docs + one reference integration (macroquad or bevy example) · samples · website/README · binary releases via `xtask` package (Win/macOS/Linux) | Tagged v1.0; a third party can animate + integrate using docs alone |

**Definition of done for v1** — a user can: import PSD or PNGs → rig bones → skin/mesh/weight →
animate with eased keys incl. draw order & attachment swaps → preview with onion skin → export
runtime format + atlas or video → load it in a sample game via `ankhimate-runtime`. All with
undo everywhere, remappable keys, and no data loss across save/load.

---

*Companion docs to be split out as implementation starts: `docs/adr/*`, `docs/format-spec.md`
(normative JSON schema), `docs/psd-import.md`, `docs/runtime-guide.md`.*
