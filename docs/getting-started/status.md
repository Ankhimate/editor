---
title: Status and support
description: Understand Ankhimate's pre-alpha maturity, status labels, supported targets, and bug-reporting expectations.
---

# Status and support

Status labels have precise meanings throughout this book:

| Label | Meaning |
|---|---|
| **Current** | Implemented and covered by the current repository. |
| **Partial** | Useful behavior exists, but the stated workflow has known gaps. |
| **Experimental** | Implemented, but its contract or UX may change substantially. |
| **Planned** | Recorded work; not available to users. |
| **Not supported** | Deliberately unavailable or outside this repository. |

**Current:** Windows, macOS, and Linux are intended desktop targets through
egui/wgpu. The framework-free core also compiles for `wasm32`. Exact packaged
release availability can differ; building from source is the reliable path today.

**Partial:** production polish. Autosave writes a sidecar and offers a newer copy
on startup; onion skinning is experimental. Persistent keymap editing, diagnostics,
localization, panic-time emergency recovery, and the final performance pass remain open.
Autosave is recovery assistance, not a backup.

**Planned:** PNG sequence, spritesheet, and video file export. MCP can already
render frames and contact sheets in memory, but the editor export workflow is not built.

**Not supported here:** game-side runtime implementations. They live in the
[separate runtime repository](https://github.com/Ankhimate/runtime).

Report reproducible bugs at the [editor issue tracker](https://github.com/Ankhimate/editor/issues).
Include the OS, GPU/backend, commit or release, steps, expected and observed
behavior, and a minimal `.ankh` file when it is safe to share. Remove personal paths
and private artwork first.
