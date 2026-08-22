# MCP server

`ankhimate-mcp` is a stdio MCP server which keeps one rig open per connection.
It consumes the same document verbs, importer registry, export plan, and read
surface as the editor and plugins; it does not define a second editing API.

`.ankh` and PSD are first-party importers. Spine and DragonBones appear only
when their JavaScript packages are installed in the platform plugin directory.
The same discovery code is used by the editor and MCP. `export_rig` accepts a
saved/built-in preset name or an installed plugin exporter id/label.

Run it with `cargo run -p ankhimate-mcp`.

## Tools

- `open_rig`, `new_rig`, `describe_rig`, `list_verbs`, `run_script`,
  `save_rig`, and `export_rig` provide the headless edit/export loop.
- `render_frame` renders one setup pose or named animation time.
- `render_contact_sheet` renders explicit `times`, or `frame_count` evenly
  spaced times, into labeled cells. One union camera is used for every cell.

Both render tools return an MCP `image` content block with base64 PNG data and
MIME type `image/png`; they do not write a temporary image. `width`, `height`,
RGBA `background`, and `camera` are per-call. Omit `camera.center` and
`camera.zoom` for automatic fitting, or provide both for fixed framing.

The optional `focus` object is also per-call:

```json
{
  "bones": ["thigh_l", "shin_l", "foot_l"],
  "include_descendants": false,
  "mode": "dim",
  "other_opacity": 0.12,
  "show_bone_names": true,
  "show_joint_points": true,
  "show_constraint_targets": false,
  "motion_trails": ["foot_l"]
}
```

`dim` retains faint body context, `isolate` hides unrelated art,
`skeleton_only` draws focused bone diagnostics without art, and `art_only`
draws focused slots without bone widgets. A slot is associated with its direct
bone. Descendants are included only when requested. A weighted mesh is kept or
dropped as one attachment; individual influences never cut its geometry.
Motion-trail bones must be in the effective focused set and use evaluated bone
tips at the contact sheet's rendered times.

Focus is renderer input, not session or document state. It does not select,
edit, key, save, or add undo history. Constraints and the complete pose are
evaluated before visual filtering.

## Rendering boundary and current omissions

`ankhimate-render` is a transport-free CPU renderer over `Document` and core
`Pose`. It reuses core region placement, weighted-mesh deformation/skinning,
and polygon clipping. It implements animated draw order, attachment visibility,
sequence frames, light/two-color tint, and all four slot blend modes. MCP only
parses requests and packages returned bytes. This layer is the starting point
for T-601 sequence and spritesheet export rather than an MCP-only path.

This first slice deliberately omits persistent camera/selection state,
non-default composed skin selection, editor authoring gizmos, GPU acceleration,
and deterministic simulation from time zero for physics constraints. Ordinary
IK, transform, and path constraints evaluate through core. T-601 still needs
frame iteration at export FPS (including physics state), trim/global-bounds
options, sequence/sheet writers and sidecar metadata, UI, performance budgets,
and golden comparison against the live viewport.

Saving over the opened source remains refused. Template/plugin export still
uses path confinement, plan-before-write, all-or-nothing writes, and never
deletes orphan files.
