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
- [x] Rust details removed from the product site; the separate `/api/` rustdoc project owns them.
- [x] Format specification: v3 structure, migrations, export/runtime contracts, JSON example.
- [x] Plugin and MCP reference: generated registries plus security and workflow guides.
- [x] Comparison and contributor path: dated evidence policy, architecture, extension guides.
- [ ] Screenshot set: requires a deliberate running-editor capture pass at a fixed size.
- [ ] External comparison evidence expansion: re-verify official sources before publication.
- [ ] Manual accessibility, narrow-screen, print, and Pages artifact inspection.

## Validation

- [x] `xtask docs-sync` generates registry-owned references.
- [x] `xtask docs-check` checks generated pages, schema version, JSON, local links, and metadata.
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
