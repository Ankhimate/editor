---
title: Install and make the first save
description: Build Ankhimate, open a shipped sample, and save a first project safely.
---

# Install and make the first save

Ankhimate currently has a source-first workflow. Install the Rust toolchain named
by `rust-toolchain.toml` and a Git client, clone the repository, then run:

```console
cargo run -p ankhimate-editor
```

A GPU supporting Vulkan, Metal, DirectX 12, or OpenGL is required by wgpu. From
the startup screen, create a project or open one of `samples/minimal.ankh`,
`samples/walker.ankh`, or `samples/tentacle.ankh`.

Save a new project with **File → Save As** and choose a `.ankh` path. Keep its
matching `<name>.assets/` directory beside it when moving or backing up the
project. External assets are not a version-control strategy. The editor binary is locked on
Windows while running, so close it before `cargo build`; `cargo check` is unaffected.

Build this website with the locked Bun dependencies:

```console
bun install --frozen-lockfile
bun run check
bun run build
```
