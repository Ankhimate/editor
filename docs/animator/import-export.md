---
title: Import and export
description: Bring artwork and external rigs into Ankhimate, then create safe template-driven engine exports.
---

# Import and export

Native `.ankh`, images, PSD, and atlas input are built-in registry paths. PSD
layer tags can infer rig structure; read the [PSD guide](/editor/psd-import/). Installed
sandboxed plugins add formats such as Spine JSON and DragonBones. These packages
are integration bridges, not claims of ownership over those formats.

Always read the import report. Unresolved names and missing images are reported
without necessarily aborting the whole load; unsupported source features can be
lossy. Save the imported rig as a new `.ankh` file before editing.

Engine export combines an atlas bake with strict Handlebars templates. Preview the
rendered plan, then write it to a dedicated output directory. Paths are confined,
all outputs are planned before writes begin, existing files may be replaced, nothing
is deleted, and orphaned old outputs are reported. Plugin exporters use the same
safety boundary.

**Planned:** PNG sequence, spritesheet animation, and video file export. MCP frame
rendering is current, but it is not the editor's rendered-output workflow.
