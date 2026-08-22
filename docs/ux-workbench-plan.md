# Ankhimate workbench plan

Status: proposed product and interaction roadmap. `docs/TASKS.md` remains the
authority for implementation status and acceptance.

## Product direction

Ankhimate should become a **context-first animation workbench** for riggers,
animators, and game teams. Its primary job is to keep three answers visible at
all times:

1. What am I editing?
2. At what time and in which clip am I editing it?
3. What will this action change?

The goal is not to copy Spine's layout. The goal is to match the workflow
compression visible in established tools, retain Ankhimate's cleaner visual
foundation, and then use its open architecture to do work a closed format
editor cannot.

The plan has two tracks:

- **Close the workbench gap:** context, density, hierarchy, timeline, graph,
  viewport feedback, settings, accessibility, and performance.
- **Build durable advantages:** N-bone IK, nested armatures, exact editing,
  automation, safe plugins, user-authored export formats, and open runtimes.

## What the comparison shows

The screenshots expose a workflow problem more than a styling problem.

| Area | Spine's advantage in the comparison | Ankhimate response |
|---|---|---|
| Context | Mode, tool, selection, clip, and frame are simultaneously legible | Add one persistent context ribbon and remove contradictory panel states |
| Density | More useful controls and timeline rows fit without feeling accidental | Add task-specific workspaces and a compact density mode |
| Hierarchy | The tree exposes bones, slots, constraints, draw order, skins, and animations in one navigable model | Make relationships, visibility, locking, filtering, reveal, and drag targets explicit |
| Timeline | Rows, channel types, keys, events, range, and transport read as one instrument | Give the timeline more space, stronger grouping, semantic channel colors, and direct framing/filter actions |
| Inspector | Selection immediately reveals relevant actions and properties | Make the inspector follow the authoritative selection and present primary actions first |
| Viewport | Active tool and transform space are hard to miss | Show tool, mode, snapping, pivot, and selection feedback next to the canvas |
| Discoverability | Text and icons reinforce each other; advanced controls remain findable | Use labels for primary actions, tooltips for every icon-only action, and progressive disclosure for detail |

Ankhimate should preserve its current strengths: calm surfaces, clear typography,
consistent panel borders, dockability, and its brand mark. The missing quality is
**workflow compression**: fewer gestures, less eye travel, and less hidden state.

## The target workbench

The default layout remains dockable, but ships with purpose-built presets:

```text
 Setup workspace
 ┌ menu / document / mode ─────────────────────────────────────┐
 ├ tools ┬──────────── viewport ────────────┬ hierarchy ───────┤
 │       │                                  ├ inspector ───────┤
 │       │                                  ├ assets / skins ──┤
 ├───────┴ context ribbon ──────────────────┴──────────────────┤
 └ compact timeline / status / diagnostics ────────────────────┘

 Animate workspace
 ┌ menu / document / mode ─────────────────────────────────────┐
 ├ tools ┬──────────── viewport ────────────┬ hierarchy ───────┤
 │       │                                  ├ inspector ───────┤
 ├───────┴ context ribbon ──────────────────┴──────────────────┤
 ├ clips ┬──────── full-width dopesheet or graph ──────────────┤
 └───────┴ transport / range / status / diagnostics ───────────┘
```

The signature element is the **context ribbon**. It is one compact row showing:

`Setup/Animate > active clip > selection > active tool > transform space > snap > frame`

It is not another toolbar. Each segment is both status and entry point. For
example, clicking the selection segment reveals its hierarchy path; clicking
the clip changes clips; transform tools expose only their relevant numeric
fields. This creates one reliable place to understand the current operation.

### Visual system

Use the existing theme system rather than hardcoded panel colors. The default
theme already supplies the right compact vocabulary:

- `Brand ink` — `#faf79f`: current mode, primary action, selected editable point.
- `Workspace` — `#18181b`: main editing surface.
- `Raised surface` — `#27272a`: panels and transient controls.
- `Divider` — `#3f3f46`: grouping, never decoration.
- `Translate` — `#6ea0e6`; `Rotate` — `#6ec86e`.
- Scale, shear, events, draw order, warnings, and onion ghosts use semantic
  theme tokens, with color never serving as the only cue.

Keep the bundled UI sans for labels, tabular monospace for frames and numeric
values, and Lucide for semantic actions. Animation software benefits from dense,
square work surfaces; avoid decorative cards, gradients, and excessive rounding.

## Roadmap

### W0 — Baseline and usability harness

Before rearranging panels, record the current interaction cost of six canonical
flows on a 1920×1080 display:

1. Import an image before creating a bone, then attach it by dragging.
2. Reparent and reorder a slot in the hierarchy.
3. Create a bone chain, bind a mesh, paint weights, and correct one vertex.
4. Create an animation, key a transform, edit its easing, and add an event.
5. Diagnose a broken reference and export a runtime package.
6. Import the DragonBones `Ankh` sample and verify animation and draw order.

Capture gesture count, completion time, errors, panel changes, and moments where
the active context is unclear. Add a manual release checklist and representative
fixtures. Screenshot tests may pin stable layout states, but behavioural tests
must remain the acceptance gate.

**Exit:** every later work package has a measured baseline and a named workflow
it improves.

### W1 — Context, selection, and density

This is the highest-value package because every editing mode benefits from it.

- Add the context ribbon and make mode, clip, frame, selection, and tool agree.
- Define one authoritative selection projection into viewport, hierarchy,
  inspector, dopesheet, graph, draw order, and assets.
- Add `Comfortable` and `Compact` density settings; preserve the current control
  size as the accessible default.
- Ship `Setup`, `Animate`, `Mesh/Weights`, and `Export` workspace presets while
  retaining arbitrary docking.
- Replace passive empty panels with one relevant next action. Do not show “no
  bone selected” when an attachment or timeline is the active selection.
- Keep primary properties and actions above the fold; advanced properties may
  collapse under stable named sections.
- Persist workspace, density, dock layout, and the context ribbon's optional
  sections through T-701.

**Exit:** switching between setup and animation never leaves stale or
contradictory context; changing clips takes one gesture; the selected entity is
identifiable in every visible surface.

### W2 — Hierarchy and asset workflow

Consolidate the recent asset and tree fixes into a deliberate interaction model.

- Permit image import with an empty document and attachment by drag to either a
  tree bone or viewport bone.
- Make drop previews distinguish reparent, sibling reorder, slot reorder, and
  invalid targets before release.
- Give every row consistent visibility, lock, selection, and type affordances;
  add solo/isolate where it has a clear document/session meaning.
- Add type filters, fuzzy search, collapse-to-selection, reveal-in-tree, and a
  hierarchy breadcrumb in the inspector.
- Show slots under their owning bones while keeping global draw order available
  as a synchronized projection, not a second truth.
- Preserve setup/animate rules: structural drops are Setup-only and animation
  gestures key properties in Animate.
- Use operators and undoable commands for all mutations; repeated drag updates
  merge into one undo step.

**Exit:** import-and-attach is one drag after file selection; any visible slot
can be moved or reordered in one drag; undo restores the exact previous parent
and order.

### W3 — Timeline as the animation workbench

- Let the timeline occupy 35–45% of the Animate workspace by default and offer a
  single-command maximize/restore action.
- Keep clip name, duration, FPS, current frame, loop range, playback state, and
  snapping visible in a fixed header.
- Group rows by bone/slot/constraint, then property; retain expansion and row
  height across dopesheet/graph switches.
- Add `Fit clip`, `Fit selection`, `Previous/next key`, `Key selected`, and
  `Filter animated` as direct actions with shortcuts.
- Use theme channel colors consistently in row icons, keys, curves, and viewport
  feedback. Shape and icon must supplement color.
- Make draw-order and event timelines first-class rows with readable values,
  not anonymous diamonds.
- Virtualize large row/key sets under T-706 and keep headers/labels pinned while
  scrolling.
- Surface markers and nondestructive offsets as ordinary timeline tools, not
  hidden advanced features.

**Exit:** the user can identify the clip, object, property, key type, and frame
without opening another panel; 300 visible tracks scroll at 60 fps on the
documented reference machine.

### W4 — Graph editor completion

This package completes T-704 and the remaining part of T-912.

- Support single-curve focus and multi-property overlays with normalize mode.
- Add fit selection/visible curves, a readable value axis, crosshair values, and
  persistent curve visibility.
- Make bezier handles large enough to target, with broken/aligned/mirrored modes
  and merged drag commands.
- Add exact numeric value, time, and tangent entry.
- Warn visually about unintended overshoot without clamping authored curves.
- Keep point selection stable when toggling between dopesheet and graph.

**Exit:** a complete easing correction—including exact tangent editing—requires
no trip to another panel; undo restores the curve exactly.

### W5 — Viewport tools and feedback

- Complete T-708's translate, rotate, scale, shear, pivot, snapping, and numeric
  entry interactions.
- Add a small contextual tool shelf adjacent to the viewport; the ribbon remains
  the canonical status surface.
- Make local/parent/world space, pivot mode, snapping, axis lock, and auto-key
  impossible to miss while active.
- Add frame selection, zoom to fit, isolate, and overlay controls for bones,
  slots, constraints, mesh edges, names, and paths.
- Finish onion skinning acceptance: configurable past/future count, frame/key
  stepping, selection-only mode, accessible tints, and zero evaluation cost when
  disabled.
- Keep hover naming available as an optional anatomy overlay, especially for
  constraints and dense meshes.

**Exit:** every transform mode has immediate canvas feedback and exact entry;
turning onion skinning off performs no ghost evaluations.

### W6 — Reliability, accessibility, and production polish

Complete T-701–T-709 rather than treating polish as a final cosmetic pass.

- Finish persistent keymap editing, conflict reporting, and per-binding reset.
- Add keyboard traversal, visible focus, tooltips for every icon-only action,
  colorblind-safe defaults, and screen-scale testing.
- Implement diagnostics that select and frame the offender and run before
  export without silently blocking it.
- Finish autosave, recovery, close guards, crash dumps, and opt-in update checks.
- Establish performance budgets for evaluation, drawing, timeline scrolling,
  plugin panels, and imports.
- Add a command palette backed by the operator registry so every action remains
  discoverable even when its panel is closed.

**Exit:** the create-bone → animate → diagnose → export path is keyboard
reachable; a forced crash recovers the document; a fresh install makes no
network request until the user requests one.

## Go beyond Spine without making false claims

Spine is not weak at the basics. Its current documentation includes brush-based
weight editing, attachment reparenting, multiple skeletons, ghosting, graph and
dopesheet views, and headless command-line export. Those are parity targets, not
Ankhimate differentiators.

The defensible opportunities below derive from Ankhimate's architecture or from
explicitly documented Spine limits. Revalidate competitor claims before using
them in marketing.

### A1 — N-bone IK as a complete workflow

Spine documents one- and two-bone IK and recommends composing constraints or FK
for longer chains. Ankhimate already has deterministic arc-seeded FABRIK with
per-bone stiffness.

Finish the product around the solver:

- chain creation by selecting first bone, tip, and target;
- viewport stiffness handles and a compact chain inspector;
- pole/bend guidance, reach visualization, and diagnostics;
- animation/runtime/export parity and sample rigs for tails, ropes, trunks, and
  tentacles;
- one-click conversion between authored FK pose and IK target where exact.

**Promise:** controllable IK chains longer than two bones, not merely a solver
hidden in the model.

### A2 — Nested animated armatures

Implement `docs/nested-armatures.md` now that the document/plugin boundary has
landed. A slot should reference a reusable child rig with `FollowHost` or
deterministic `StartOnShow` playback.

This enables reusable weapons, effects, faces, and props whose animation travels
with the asset. Include recursion limits, cycle diagnostics, clear timeline
ownership, format migration, exporter context evolution, and runtime support.

**Promise:** reusable animated sub-rigs with their own deterministic playheads,
not only static attachment substitution or separately layered skeletons.

### A3 — Exact and safe editing everywhere

Complete the unfinished Phase 9 work:

- T-901 bulk rename with reference preview and collision handling;
- T-902 numeric vertex, handle, UV, and transform entry;
- T-904 named selection sets;
- T-909 rig and animation transfer with explicit mapping/conflicts;
- T-912 numeric tangent editing;
- a second synchronized viewport from T-910 for setup/animate or camera/detail
  comparison.

Add a non-destructive operation stack only where the operation has clear,
serializable semantics. Do not turn ordinary edits into an opaque modifier
system.

**Promise:** large corrective operations are previewable, undoable, and exact.

### A4 — Open format creation, not an exporter list

Keep Ankhimate's own runtime format expressed through the same strict
Handlebars engine available to users. Add:

- schema-aware completion and inline validation for template authors;
- preview of planned files and diffs before writing;
- deterministic export fixtures and package-level compatibility tests;
- shareable exporter packages installed through the same marketplace;
- rendered output as its separate T-601/T-602 pipeline.

Spine already automates export through a CLI, so automation alone is not the
claim. The advantage is that users author and distribute new safe formats
without rebuilding the editor.

### A5 — Sandboxed marketplace and automation surface

Turn the completed plugin/MCP architecture into a coherent product:

- marketplace index fetched explicitly by the user, with version, hash,
  compatibility, source, and permission information;
- install, update, rollback, disable, and remove flows with no stale package
  cache;
- importer/exporter/operator/panel capability labels before installation;
- reproducible lock data per project or workspace;
- the same named verbs and read surface in the editor, plugins, and MCP;
- recipes for batch validation, procedural rigging, migration, preview, and CI.

Scripts remain sandboxed without filesystem, network, or clock access. Host
operations retain path confinement, transactions, and undo.

**Promise:** one safe automation vocabulary for humans, plugins, and agents.

### A6 — Open runtime ecosystem

Keep runtimes in their own repository and treat them as a family rather than a
single Rust playback crate:

- define a language-neutral conformance corpus from `core::evaluate()`;
- ship runtime-format fixtures, expected poses, events, draw batches, and
  crossfade results;
- use the corpus for Phaser/TypeScript first, then vanilla web, Unity, and other
  community runtimes;
- publish a compatibility matrix by format version and feature;
- require runtime implementations to pass conformance before being called
  supported.

**Promise:** an open, testable runtime contract that is not tied to the desktop
editor's implementation language.

## Priority and dependency order

| Priority | Work | Why now |
|---|---|---|
| P0 | W0, W1, W2, W3 | Removes the daily friction visible in the comparison and stabilizes selection/context semantics |
| P1 | W4, W5, W6 | Completes professional animation and production-readiness workflows |
| P1 | A1, A4, A5 | Mostly builds on capabilities already present and makes them discoverable |
| P2 | A3 | High-value precision features that can land independently |
| P2 | A6 | Requires the runtime-format contract and conformance fixtures |
| P3 | A2 | Highest differentiation, but also the most invasive model/schema migration |

Nested armatures must follow a dedicated migration plan. Visual work must not
pull egui concepts into `core`, and workspace changes must not invent a second
mutation path around document commands.

## Release gates

Ankhimate should not call this plan complete because screenshots look denser.
The following gates define the outcome:

- The six baseline workflows complete with no unexplained context switch and at
  least 30% fewer gestures where W1–W3 target them.
- Active mode, clip, selection, tool, and frame never disagree across visible
  surfaces.
- Import-and-attach and hierarchy reordering each take one direct drag after the
  source is available.
- Timeline scrolling holds 60 fps at 300 rows; the documented 500-bone
  evaluation benchmark stays within T-706's budget.
- All document mutations are undoable commands and drag sequences undo once.
- Setup/Animate enforcement is identical for menu, keymap, plugin, and MCP
  callers.
- Keyboard-only create → key → export succeeds; every icon-only control has a
  tooltip and visible focus.
- Marketplace packages are versioned, integrity-checked, reversible, and never
  gain ambient filesystem/network/clock access.
- Each supported runtime passes the same conformance fixtures.
- `cargo test --workspace`, `cargo fmt --all --check`, and
  `cargo clippy --workspace --all-targets` remain clean at each milestone.

## Non-goals

- Do not clone Spine's visual arrangement, terminology, or interactions when a
  clearer Ankhimate-native design exists.
- Do not make every surface permanently dense; density follows the active job.
- Do not hide structural mode rules behind convenient direct mutation.
- Do not move renderer, editor, filesystem, or clock concerns into `core`.
- Do not market an advantage solely because an older competitor version lacked
  it; maintain an evidence note and recheck current official documentation.

## Evidence used for competitor boundaries

- [Spine user guide](https://esotericsoftware.com/spine-user-guide/)
- [Spine IK constraints](https://en.esotericsoftware.com/spine-ik-constraints)
- [Spine weights](https://us.esotericsoftware.com/spine-weights)
- [Spine attachments](https://us.esotericsoftware.com/spine-attachments)
- [Spine skeletons](https://en.esotericsoftware.com/spine-skeletons)
- [Spine export](https://us.esotericsoftware.com/spine-export/)
- [Spine command-line interface](https://en.esotericsoftware.com/spine-command-line-interface)

