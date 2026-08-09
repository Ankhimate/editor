# Ankhimate — 2D skeletal animation editor (Rust)

A desktop editor for 2D skeletal animation: rig image parts into a skeleton, pose
it on a timeline, export a runtime format. MIT OR Apache-2.0.

> The `CLAUDE.md` one directory up describes an older Tauri/React/PixiJS
> prototype. **This workspace is the project**; ignore that file.

## Workspace

| Crate | What it is |
|---|---|
| `core` | Framework-free model + `evaluate()`. No egui, no wgpu, no I/O. `#![forbid(unsafe_code)]`, compiles for `wasm32`. |
| `editor` | egui + wgpu desktop app. |
| `formats` | `.ankh` read/write, PSD/atlas import, version migration. |
| `export` | Atlas packing, image/video export, runtime bake. **Stub — 6 lines.** |

## Rules that are not negotiable

- **`core` stays framework-free.** No egui, wgpu, or filesystem access in it. It
  is the contract the editor, exporters and future runtimes all share; anything
  that leaks in has to be re-implemented by every consumer.
- **`evaluate()` is deterministic** (PLAN §2.6). Same inputs, same pose, every
  time — no wall-clock, no RNG, no iteration-order dependence. Physics carries
  its own accumulator for exactly this reason.
- **Every document edit is an undoable command.** `EditCommand` in
  `editor/src/commands/`, dispatched through `AppState::dispatch`. Never mutate
  `doc` from a panel directly. Commands that a drag repeats implement `merge` so
  the drag is one undo step, and capture `before` on the *first* apply so undo
  lands where the drag began.
- **World transforms are computed, never stored.** Locals are the truth; `Pose`
  derives worlds along `update_order`, which is topologically sorted.
- **Names on disk, ids in memory** (ADR 0004). Slotmap keys are not stable across
  sessions; every cross-reference in `formats/src/schema.rs` is a name. A name
  that fails to resolve on load goes to `LoadReport`, it does not fail the load.
- **Setup vs Animate** (T-207, ADR 0006). Structural edits are Setup-only; the
  same gesture in Animate becomes a key. Commands declare `requires_mode`.
- **Clean-room** (PLAN §0). Features are implemented from *observed behaviour*.
  Never copy, translate or closely paraphrase another editor's source — several
  are GPL-3.0.

## Conventions

- Angles are **radians** inside `core`, **degrees** on disk and in the UI. The
  conversion happens in `formats/src/convert.rs` and nowhere else.
- Y is **up** in world space; screen Y is down. The camera converts.
- `Affine2`, not `Mat4` (ADR 0002).
- Session state (selection, camera, tool) lives in `editor/src/session.rs` and is
  never saved or undone. If it would be wrong to find it in a teammate's file, it
  belongs there.

## Build

```bash
cargo run -p ankhimate-editor
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets
```

The editor holds its own binary while running — `cargo build` fails with
"Access is denied" until it is closed. `cargo check` works regardless.

## Testing

Tests are expected to pin *behaviour*, not restate the implementation. A test
that would pass with the bug still present is worse than none — several in this
repo say so in their doc comment when they are weaker than they look (see
`a_grouped_bone_is_drawn_once`).

`cargo test --workspace` is the gate. Formatting and clippy must be clean before
a commit.

## egui gotchas that have cost real time here

- `Response::context_menu` **closes on any plain click of its host response**
  (`Popup::context_menu`, popup.rs:252). For a menu over a wide strip — a ruler,
  a lane — clicking a field inside it counts as clicking the host and dismisses
  the popup. Use a self-managed `egui::Window` instead. See
  `editor/src/ui/timeline/ruler.rs`.
- Two `ui.interact` calls over the same rect: the **later** one wins the pointer.
  A second registration silently eats the first one's clicks.
- `make_persistent_id` hashes against the parent `Ui`. Drawing the same entity in
  two places under one parent produces one id and a red "First use of widget ID"
  wall. `ui.push_id` scopes it — but check first whether the real bug is that the
  thing is drawn twice.
- Text fields rebuilt from the document each frame swallow keystrokes; keep the
  buffer in `ui.data` while editing.

## Documentation

- `docs/TASKS.md` — task breakdown and current status, including what is
  deliberately *not* done and why.
- `docs/ARCHITECTURE_PLAN.md` — normative architecture.
- `docs/what-others-cannot.md` — where this goes past the established editors,
  and where it does not.
- `docs/graph-editor.md` — the curve editor's interaction rules and their reasons.
- `docs/adr/` — architecture decision records.

## Current state

Phases 0–5 and 9 are done; the editor is usable for rigging and animating.

Two gaps, in order of size:

- **Phase 6 — export and runtime — is unstarted.** Nothing can leave the editor
  for a game engine. `export` is a stub and `runtime` does not exist.
- **Phase 7 — production polish — is mostly unstarted**: settings/keymap/autosave,
  diagnostics, onion skinning, localization, a performance pass, crash recovery.
  Some of it arrived incidentally through Phase 9 (the curve editor got its
  T-912 pass), but the boxes are unticked.

`docs/TASKS.md` is authoritative; a ✅ on a task heading means it landed. Three
Phase 9 tasks carry a written note about the part of them that did *not* land —
prefer reading those over assuming a heading means everything under it is done.
