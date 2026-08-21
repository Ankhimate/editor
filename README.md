# Ankhimate

A 2D skeletal animation editor written in Rust — bones, skins, meshes, weights
and keyframes — with a portable, deterministic core you can embed in a game.

Rig image parts into a skeleton, pose it on a timeline, and export a runtime
format your engine can play back. Free and open source under MIT OR Apache-2.0:
no runtime licence to buy, no copyleft to inherit.

> **Status: pre-alpha, under active development.** The editor runs and the data
> model is stable enough to build on, but the file format is not frozen and
> export is unfinished. Not yet suitable for production rigs.
> [`docs/TASKS.md`](docs/TASKS.md) tracks exactly what exists and what does not.

## What works today

- **Setup and Animate modes** — structural edits belong to Setup, poses to
  Animate. The editor refuses the wrong one rather than quietly corrupting a rig.
- **Bones** — hierarchy, FK, translate/rotate/scale/shear gizmos, and IK chains
  of **any length** (FABRIK), not the two most editors cap at.
- **Constraints** — IK, transform, path and physics, with a stability harness
  behind the last of those (fixed-step, framerate-independent, soak-tested).
- **Slots, attachments and skins** — art is addressed through slots, so one
  skeleton drives many outfits. Clipping polygons, bounding boxes, points,
  sequences and linked meshes are all authorable.
- **Meshes** — convert a region to a mesh, edit vertices, trace a silhouette
  from the artwork's alpha, and paint bone weights with a real brush — radius,
  feather, five blend modes, per-bone locking. Every vertex and UV is typeable,
  and a multi-selection aligns on an axis.
- **Animation** — dopesheet, graph editor, bezier interpolation, transform and
  free-form deform (FFD) timelines, events with audio, ruler markers, and
  non-destructive per-bone track offsets for secondary motion.
- **Working on a big rig** — folders in the hierarchy, isolation mode, bulk
  rename with numbering, box-select, multi-selection transforms about a shared
  pivot, hover labels naming what is under the cursor, and panels that tear off
  into their own window.
- **Files** — `.ankh` projects (a zip of `project.json` + images), image import,
  PSD import, and atlas/spritesheet import.

- **Export, in any format you can write** — export is a *format editor*, not a
  fixed list of exporters. A preset is an atlas setting plus templates that say
  what the files look like, with a live preview against the open rig. Ankhimate's
  own runtime format ships as one of those templates, so nothing is reachable to
  us that is not reachable to you. See
  [`docs/export-context.md`](docs/export-context.md).
- **A runtime crate** — `ankhimate-runtime` loads an export and plays it:
  crossfade, events, physics, draw batches. No wgpu, no window.

## Not there yet

**Rendered output**: image sequence, spritesheet and video export are unwritten.
The data pipeline works — a rig and its atlas can leave for an engine — but
Ankhimate cannot yet hand you a PNG sequence or an MP4 of a clip. A reusable
headless renderer now powers MCP frame/contact-sheet previews, but the T-601
file-export iteration, UI, physics stepping and metadata are still open.

The runtime crate has no worked example yet — `macroquad_player` and
`docs/runtime-guide.md` are the next task.

Also missing: onion skinning, autosave and crash recovery, localization, and a
second viewport in its own window (other panels tear off; the canvas does not).

## Crates

| Crate | Description |
|---|---|
| `core` (`ankhimate-core`) | Framework-free data model + `evaluate()`. The single runtime contract used by the editor, exporters, and games. `#![forbid(unsafe_code)]`, compiles for native + `wasm32`. |
| `document` (`ankhimate-document`) | Headless document, undo, named verbs, and the shared plugin/MCP read surface. |
| `editor` (`ankhimate-editor`) | egui/wgpu desktop application. |
| `formats` (`ankhimate-formats`) | `.ankh` read/write, importers, version migration. |
| `plugins` (`ankhimate-plugins`) | Sandboxed QuickJS plugins: operators, importers, exporters, and declarative panels. |
| `render` (`ankhimate-render`) | Transport-free headless PNG renderer shared by MCP previews and future rendered exports. |
| `mcp` (`ankhimate-mcp`) | Stdio MCP server over the same headless verbs and format/export registries. |
| `export` (`ankhimate-export`) | Atlas bake + the template engine presets are written against. Headless. |
| `runtime` (`ankhimate-runtime`) | Playback for games: load, crossfade, events, draw batches. No wgpu. |

## Build & run

Requirements: a recent stable Rust toolchain (see `rust-toolchain.toml`) and a
GPU with Vulkan, Metal, DX12, or GL support.

```bash
cargo run -p ankhimate-editor   # launch the editor
cargo run -p ankhimate-mcp      # start the MCP stdio server
cargo test --workspace          # run all tests
cargo fmt --check               # check formatting
cargo clippy --workspace -- -D warnings   # lint
```

## Try it

A generated sample rig — 12 bones, a walk cycle, an IK constraint and event
markers — plus a walkthrough of how it was built:

```bash
cargo run -p ankhimate-formats --example make_sample   # writes samples/walker.ankh
cargo run -p ankhimate-editor                          # then open it
```

See [`docs/rigging-walkthrough.md`](docs/rigging-walkthrough.md).

An eight-bone IK chain, which a two-bone solver cannot express at all — drag
`tentacle-target` and the whole thing curls:

```bash
cargo run -p ankhimate-formats --example gen_tentacle   # writes samples/tentacle.ankh
```

[`docs/what-others-cannot.md`](docs/what-others-cannot.md) is the short, honest
list of where this goes further than the established editors — and where it does
not.

## Contributing

Contributions are welcome — see [`CONTRIBUTING.md`](CONTRIBUTING.md). Work is
broken into PR-sized tasks in [`docs/TASKS.md`](docs/TASKS.md), each with its own
acceptance criteria; picking one up is the easiest way in.

**Read the clean-room policy first.** Ankhimate implements features observed in
other animation editors — never their source code.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. This explicitly allows embedding the runtime in games under
either license — a deliberate advantage over GPL-only alternatives.

Unless you state otherwise, any contribution you intentionally submit for
inclusion shall be dual-licensed as above, with no additional terms.

### Bundled assets

The editor ships the [Lucide](https://lucide.dev) icon font
(`editor/assets/lucide.ttf`) under the ISC license, reproduced verbatim at
`editor/assets/LUCIDE-LICENSE`. ISC is permissive and imposes no obligation on
anything you build with Ankhimate — it applies to the icon font alone, which the
runtime does not use.

## Documentation

- [`docs/ARCHITECTURE_PLAN.md`](docs/ARCHITECTURE_PLAN.md) — normative architecture & roadmap
- [`docs/TASKS.md`](docs/TASKS.md) — task breakdown and current status
- [`docs/rigging-walkthrough.md`](docs/rigging-walkthrough.md) — how a rig is put together
- [`docs/what-others-cannot.md`](docs/what-others-cannot.md) — where this goes past the established editors, and where it does not
- [`docs/graph-editor.md`](docs/graph-editor.md) — the curve editor's interaction rules, and why each one is what it is
- [`docs/export-context.md`](docs/export-context.md) — writing an export format: syntax, helpers, and every field a template can address
- [`docs/export-plan.md`](docs/export-plan.md) — why export is a format editor rather than a list of exporters
- [`docs/adr/`](docs/adr/) — architecture decision records
