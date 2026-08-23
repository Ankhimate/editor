---
title: .ankh authoring format, version 3
description: The normative ZIP container and JSON schema for editable Ankhimate projects.
---

# `.ankh` authoring format, version 3

This chapter specifies the current schema. `formats/src/schema.rs` remains the
machine-enforced authority if prose and code differ.

## Container and global conventions

An `.ankh` file is a Deflate ZIP. It must contain UTF-8 `project.json`; bundled
image bytes live below `images/` using each asset's relative `file`. Directories
are optional. Other archive entries are ignored. Writers must not use absolute
paths or traversal components for image names.

Times and durations are seconds. Angles are counter-clockwise degrees on disk.
Colors are normalized RGBA arrays. References are names, not in-memory slotmap
keys. Array order is meaningful for bones, slots, setup draw order, constraint
order, timelines, and keys. Serde defaults shown below apply when fields are
omitted. Unknown JSON fields are captured and written back for types carrying an
`extra` map; do not assume unknown archive entries are rewritten by a save.

## Project objects

| Object | Fields and rules |
|---|---|
| Project | required `version`, `name`, `fps`; default-empty `assets`, `bones`, `slots`, `draw_order`, `skins`, `constraints`, `constraint_order`, `animations`; `default_skin`; optional groups, PSD provenance map, opaque export presets. |
| Asset | `name`, relative `file`, pixel `width`/`height`, optional advisory `source_path`. Pixels are in `images/file`. |
| Group | `name`, RGBA `color`, `members` as `kind:name`, optional parent group name. Organization only. |
| Bone | `name`, parent name, `length`, `tx`, `ty`, degree `rotation`/`shear_x`/`shear_y`, `sx`/`sy` default 1, three inheritance booleans default true, optional color. |
| Slot | `name`, bone name, optional setup attachment name, light/dark colors, blend-mode string. `draw_order` lists slot names. |
| Skin | `name`, entries, optional bone and constraint names. An entry has `slot`, lookup `name`, and tagged attachment. |
| Constraint | `name`, `type`, target and ordered bones plus kind-specific fields. `constraint_order` names application order. |
| Animation | `name`, duration, looping (default true), timelines, runtime events, editor-only markers, and per-bone sampling offsets. |

## Attachments

Attachments are tagged by lowercase `type`: `region`, `mesh`, `clipping`, `path`,
`boundingbox`, or `point`. A region names a texture and stores transform, size,
UV rectangle, normalized bottom-left pivot (default 0.5, 0.5), and optional image
sequence. A sequence has texture-frame names, fps, mode (`hold`, `once`, `loop`,
`ping_pong`, plus reverse variants), and setup index.

A mesh stores flat XY `vertices`, flat UVs, triangle indices, preserved edge index
pairs, and per-vertex arrays of `(bone_name, weight)`. Empty weights mean rigid.
`linked` identifies source skin/slot/attachment and whether deform is inherited.
Bounding boxes share the flat vertex and optional weight shape. Clipping adds an
optional inclusive end-slot name. Paths add `closed` and `constant_speed` (default
true). Points store X, Y, and degree rotation.

## Constraints

IK uses `bend_direction` and `mix` (defaults 1), softness, stretch, stretch limit
(default 1.1), and stiffness. Transform constraints add named per-axis
`transform_mix`, seven offsets `[x,y,rotation,sx,sy,shear_x,shear_y]`, and local/
relative flags. Physics uses `[inertia,strength,damping,mass]`, force vector
`[wind_x,wind_y,gravity_x,gravity_y]`, and `[rotate,translate]` channel flags. Path
constraints name a slot and store `[position,spacing,mix_rotate,mix_translate]`.

## Timelines and keys

`kind` is one of `bone_translate`, `bone_rotate`, `bone_scale`, `bone_shear`,
`slot_color`, `slot_visible`, `slot_attachment`, `draw_order`, `ik_mix`,
`ik_bend_direction`, `ik_softness`, `transform_constraint_mix`, or `deform`.
Translate, scale, and shear identify `axis: x|y`; rotation/shear values are degrees.

Continuous scalar/color/mix/deform keys contain `time`, values, and flattened
interpolation. `{ "curve": "linear" }` is the default; stepped holds; Bézier adds
`handles: [out_x,out_y,in_x,in_y]`, with X in 0..1 and Y unbounded. Visibility and
attachment are discrete. Draw-order keys store `(slot_name, setup_offset)` pairs.
Version 3 weighted deform offsets are flat XY pairs per influence, not per vertex.

Events store time, name, integer/float/string payloads, optional audio asset, volume,
and balance. Markers store time/name/color but never enter runtime export. Bone
offsets store a bone name and seconds; positive trails and negative leads.

## Minimal `project.json`

```json
{
  "version": 3,
  "name": "minimal",
  "fps": 30,
  "bones": [{ "name": "root", "length": 30.0 }],
  "animations": []
}
```

Load resolves names after parsing. Missing parents, slots, textures, attachments,
or constraint targets are accumulated in `LoadReport` where recovery is possible;
they do not make every project unreadable. A load/save round trip must preserve
known semantics and unknown fields supported by `extra`.
