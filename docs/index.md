---
title: Ankhimate
description: Rig image parts into expressive 2D skeletal animation and export data for the runtime you choose.
template: splash
hero:
  tagline: An open-source animation workbench for building rigs, posing movement, and authoring engine-ready exports.
  actions:
    - text: Start animating
      link: /editor/getting-started/install/
      icon: right-arrow
    - text: Integration guides
      link: /editor/formats/
      variant: minimal
---

# Make movement from parts

Ankhimate is an open-source desktop editor for 2D skeletal animation. Instead of
drawing every frame, you attach image parts to bones, pose the bones, and let the
editor interpolate between keyed poses. Meshes bend artwork where rigid parts are
not enough.

> **Maturity:** Ankhimate 0.1 is pre-alpha and actively evolving. It has less UI
> polish, production history, documentation, and runtime coverage than established
> tools. Save copies of important work and read the [status guide](/editor/getting-started/status/).

## Choose a path

- **Animators:** [install the editor](/editor/getting-started/install/), learn the
  [workspace](/editor/animator/workspace/), then build a first rig with the
  [recipes](/editor/animator/recipes/).
- **Integrators:** read the [Ankh v1 format](format-spec.md),
  [plugin](/editor/automation/plugins/), [MCP](/editor/automation/mcp/), or
  [runtime export](/editor/formats/export-runtime/) contract.
- **Rust contributors:** use the [Rust API reference](/editor/api/) for functions,
  modules, and implementation algorithms.

The book documents the current `main` branch. Planning documents are linked as
history; code, schemas, registries, tests, and accepted ADRs are the sources of truth.
