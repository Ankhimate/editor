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
| `export` | Atlas bake + the Handlebars template engine every export format is written in. Headless — no egui, no wgpu. |
| `runtime` | Game-side playback: load, crossfade, events, draw batches. Deliberately thin; the maths lives in `core`. |

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
- **Verbs are named, and the names are a contract.** A keybinding, a menu entry
  and (soon) a plugin all reach an action through its dotted id in
  `commands/registry.rs` — `edit.undo`, `tool.create_bone` — never by calling a
  method. Renaming one silently breaks a user's keymap and any plugin that
  shadowed it, with no compiler on that side; treat it exactly as a rename in
  `docs/export-context.md`. An operator's `enabled` is where a precondition
  lives, so every caller gets the same answer rather than each remembering it.
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
- `docs/export-plan.md` — why export is a format editor, and the decisions behind it.
- `docs/plugin-plan.md` — the plugin API's four surfaces, the headless split
  underneath them, and why an MCP server is a consumer of that API rather than a
  second road.
- `docs/plugin-api.md` — the verbs, their arguments and the read surface, as
  they stand. A public contract.
- `docs/nested-armatures.md` — a slot that draws another rig on its own
  playhead: why it beats Spine's static skin swap, and why it waits for the
  plugin work.
- `docs/export-context.md` — the template context, field by field. A public contract.
- `docs/adr/` — architecture decision records.

## Export is user-authored, and that is load-bearing

Export is a **format editor**, not a set of exporters (T-603, `docs/export-plan.md`).
Ankhimate cannot know which engine a rig targets and the list is not closeable,
so the deliverable is the engine for writing exporters: a Handlebars template
over a documented context, plus a baked atlas.

Three rules that are not negotiable here:

- **Our own runtime format is a template**, not Rust. If it cannot be expressed,
  the engine is too weak and we find out before users do.
- **Strict mode always.** Default Handlebars renders a missing field as an empty
  string — a corrupt export that looks fine. It must error with a location.
- **Writing is the dangerous part.** Paths are confined to the output directory
  (they render from rig data, and rigs arrive from other people); an export is
  all-or-nothing; **nothing is ever deleted**, orphans are reported.

The template context (`docs/export-context.md`) is a **public contract** — a
rename breaks user templates silently, with no compiler on that side.

## Current state

Phases 0–5, 6 and 9 are done; a rig can be built, animated, and exported to an
engine in a format the user writes.

Remaining, in order of size:

- **Phase 7 — production polish — is mostly unstarted**: settings/keymap/autosave,
  diagnostics, onion skinning, localization, a performance pass, crash recovery.
  Some of it arrived incidentally through Phase 9 (the curve editor got its
  T-912 pass), but the boxes are unticked.
- **T-601/T-602 — rendered output**: PNG sequence, spritesheet, video. A
  different pipeline entirely (headless wgpu render, then ffmpeg), unrelated to
  the template path.
- **T-604's example**: `macroquad_player` and `docs/runtime-guide.md` are
  unwritten. `wasm32` builds clean but is not wired into CI.

The export panel is **implemented and tested but has never been driven in the
running editor** — T-603d's acceptance says so explicitly.

`docs/TASKS.md` is authoritative; a ✅ on a task heading means it landed. Three
Phase 9 tasks carry a written note about the part of them that did *not* land —
prefer reading those over assuming a heading means everything under it is done.
