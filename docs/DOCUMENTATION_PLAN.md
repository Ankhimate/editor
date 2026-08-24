---
title: Documentation work plan
description: Completion, validation, and source-of-truth checklist for the documentation website.
slug: documentation-plan
---

# Documentation work plan

This checklist is the interruption-safe record for the mdBook project. A checked
item means the page exists and has passed the stated automated checks; it does not
override feature status in `TASKS.md`.

## Milestones

- [x] Site foundation: Bun/Starlight config, navigation, theme, status vocabulary, Pages workflow.
- [x] Animator path: task-oriented rigging, animation, import/export, recipes, troubleshooting.
- [x] Rigging fundamentals deep pass: modes, hierarchy, selection, bones, slots, rigid attachments, assets, and draw order verified against the editor UI.
- [x] Mesh and deformation deep pass: topology, tracing, UVs, binding, every weight control, deform keys, and current linked/per-influence limitations verified against implementation.
- [x] Animation deep pass: clips, transport, explicit and pending keying, dopesheet, graph curves, slot tracks, events, markers, offsets, retiming, onion skin, and current editing limitations verified against implementation.
- [x] Rust details removed from the product site; the separate `/api/` rustdoc project owns them.
- [x] Format specification: field-complete v3 structure, migrations, export/runtime contracts, JSON example.
- [x] Plugin and MCP reference: generated registries plus security and workflow guides.
- [x] Comparison and contributor path: dated evidence policy, architecture, extension guides.
- [ ] Screenshot set: requires a deliberate running-editor capture pass at a fixed size.
- [ ] External comparison evidence expansion: re-verify official sources before publication.
- [ ] Manual accessibility, narrow-screen, print, and Pages artifact inspection.

## Validation

- [x] `xtask docs-sync` generates registry-owned references.
- [x] `xtask docs-check` checks generated pages, schema version, JSON, local links, and metadata.
- [x] `xtask docs-check` parses `schema.rs` and requires a documentation marker for every public schema field and enum variant.
- [x] `bun run check` and the combined Starlight/rustdoc `bun run build` verified locally.
- [x] Workspace tests, format, and clippy exit successfully; clippy retains pre-existing warnings outside documentation tooling.

## Source map

| Subject | Source of truth |
|---|---|
| Feature completion | `docs/TASKS.md`, tests, editor UI |
| Rust APIs and algorithms | Separate `/api/` rustdoc site |
| Authoring schema | `formats/src/schema.rs`, `convert.rs`, `migrate.rs` |
| Document verbs | `DocOps::builtin()`; generated reference |
| MCP tools | `mcp::tools::all()`; generated reference |
| Plugin API | `plugins/src`, plugin tests, `docs/plugin-api.md` |
| Export contract | `export/src`, `docs/export-context.md` |
