---
title: Build plugins
description: Package and test sandboxed importers, exporters, automations, and declarative panels.
---

# Plugin API

Plugins are discovered packages containing `plugin.js` and optional packaged
resources. Marketplace metadata supplies identity, label, version, compatibility,
and download information. Installed packages can restore importer, exporter, and
panel registrations on startup; discovery and load failures are reported per package.
See the checked-in community packages for executable integration examples.

QuickJS isolates plugin code. Scripts receive no filesystem, network, wall clock,
or random host capability. The globals are:

- `ops.list()`, `ops.schema(id)`, and `ops.invoke(id, args)` for discoverable,
  undoable edits with mode enforcement;
- `rig()` for the shared export-context-shaped read tree, `names()` for compact
  inventories, and `console.log()` for captured diagnostics;
- importer registration, layered-document helpers, complete-project import,
  image crop/info, sidecars, and read-only package resources;
- exporter registration plus `emit`, `emitBytes`, `emitPreset`, and `bakeAtlas`;
- declarative panel registration. Panels return host-drawn widgets and named event
  handlers; scripts never draw egui or run during deterministic evaluation.

The exact signatures and widget vocabulary are maintained in the existing
[plugin API contract](/editor/plugin-api/) and tested by `plugins/tests`. The
[generated verb inventory](/editor/reference/document-verbs/) gives every current
ID, mode, label, and argument schema.

Read all arguments before invoking edits when an operation must be atomic. Each
successful verb enters normal undo history. Importers report unresolved or lossy
data. Export output remains path-confined and never deletes. API evolution should
be additive; changing a dotted verb ID, context field, or established global is a
breaking change requiring migration and explicit release notes.

Before publishing, test installation from the package shape, resource and sidecar
access, invalid input, repeated load, disabled/removed state, every registration,
and execution under the sandbox—not a standalone JavaScript runtime.
