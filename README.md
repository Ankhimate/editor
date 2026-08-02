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
- **Bones** — hierarchy, FK, 2-bone IK, translate/rotate/scale/shear gizmos.
- **Slots, attachments and skins** — art is addressed through slots, so one
  skeleton drives many outfits.
- **Meshes** — convert a region to a mesh, edit vertices, trace a silhouette
  from the artwork's alpha, paint bone weights.
- **Animation** — dopesheet, bezier interpolation, transform timelines, and
  free-form deform (FFD) timelines.
- **Files** — `.ankh` projects (a zip of `project.json` + images) and image import.

## Not there yet

Constraints beyond IK (transform, path, physics), event timelines, PSD import,
clipping/masking authoring, and the whole export pipeline — spritesheet, video,
and the game-side runtime crate.

## Crates

| Crate | Description |
|---|---|
| `core` (`ankhimate-core`) | Framework-free data model + `evaluate()`. The single runtime contract used by the editor, exporters, and games. `#![forbid(unsafe_code)]`, compiles for native + `wasm32`. |
| `editor` (`ankhimate-editor`) | egui/wgpu desktop application. |
| `formats` (`ankhimate-formats`) | `.ankh` read/write, importers, version migration. |
| `export` (`ankhimate-export`) | Atlas packing, image/spritesheet/video export, runtime-format bake. *(stub)* |
| `runtime` (`ankhimate-runtime`) | Thin playback API for games. *(planned)* |

## Build & run

Requirements: a recent stable Rust toolchain (see `rust-toolchain.toml`) and a
GPU with Vulkan, Metal, DX12, or GL support.

```bash
cargo run -p ankhimate-editor   # launch the editor
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

## Documentation

- [`docs/ARCHITECTURE_PLAN.md`](docs/ARCHITECTURE_PLAN.md) — normative architecture & roadmap
- [`docs/TASKS.md`](docs/TASKS.md) — task breakdown and current status
- [`docs/rigging-walkthrough.md`](docs/rigging-walkthrough.md) — how a rig is put together
- [`docs/adr/`](docs/adr/) — architecture decision records
