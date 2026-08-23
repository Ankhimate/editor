---
title: Comparison and migration
description: A conservative, dated comparison of Ankhimate, Spine, and DragonBones with migration cautions.
---

# Comparison and migration

> Verified 2026-08-23. This is a conservative orientation, not a scorecard.
> External products change; re-check linked official material before relying on it.

Ankhimate is still evolving. Spine has substantially greater production history,
documentation depth, tooling polish, and runtime coverage. DragonBones maintenance
and current distribution details are not inferred where official evidence is unclear.

| Area | Ankhimate 0.1 | Spine | DragonBones |
|---|---|---|---|
| Editor maturity | **Partial** — pre-alpha, production polish incomplete | **Current** — established commercial editor | **Unknown** — verify current official distribution/support |
| Bones, meshes, weights | **Current** | **Current** | **Current** in published format/editor material |
| IK/constraints | **Current** — includes arbitrary-length IK | **Current** | **Unknown** for current parity |
| Graph animation | **Current** | **Current** | **Unknown** for current parity |
| Runtime ecosystem | **Partial** — external repository, limited coverage | **Current** — broad official runtime set | **Unknown** — ecosystem status varies |
| Sandboxed format plugins | **Current** | **Unknown** as an equivalent architecture | **Unknown** |
| Named verbs and MCP | **Current** | **Unknown** as an equivalent built-in contract | **Unknown** |
| User-authored export templates | **Current** | **Unknown** as an equivalent core workflow | **Unknown** |
| Rendered video/sequence export | **Planned** | **Current** | **Unknown** |
| Open-source editor | **Current**, MIT OR Apache-2.0 | **Not supported** (commercial proprietary editor) | **Unknown** for currently maintained official editor |

Official starting points: [Spine documentation](https://esotericsoftware.com/spine-user-guide),
[Spine runtimes](https://esotericsoftware.com/spine-runtimes), and the
[DragonBones GitHub organization](https://github.com/DragonBones). Claims marked
Unknown require a new evidence pass rather than assumptions from old community posts.

Choose Spine when production-proven tooling, extensive official runtimes, and deep
documentation outweigh license cost and proprietary authoring. Consider Ankhimate
when open-source authoring, inspectable formats, sandboxed format extensions,
headless named automation, MCP, or custom template output are central—and pre-alpha
risk is acceptable. Treat DragonBones choice as a project-specific investigation.

Spine and DragonBones importers are sandboxed community packages. Bones, slots,
regions, meshes, weights, skins, and common timelines can carry across when mapped;
unsupported constraints, blend behavior, curves, events, linked data, or runtime-
specific metadata may be lossy. Import into a new file, read the report, compare
setup and representative frames, then keep the source project. Nested armatures,
additional runtimes, rendered video, and remaining production polish are roadmap
items, not current advantages.
