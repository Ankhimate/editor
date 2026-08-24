---
title: .ankh authoring format, version 3
description: Field-by-field specification of the editable Ankhimate project container and JSON schema.
---

# `.ankh` authoring format, version 3

This is the normative authoring-file reference for schema version 3. The Rust
schema remains the machine-enforced authority if prose and implementation differ.
An `.ankh` is an editable project, not a game runtime file.

## Conventions

- **Required** means omission is a JSON error. **Default** is the reader's value
  when omitted. `number` is an `f32`; integers are explicitly identified.
- Time is seconds; distances are authoring-space units; angles are
  counter-clockwise degrees. World Y points up.
- Colors are `[r,g,b,a]`, conventionally in `0..1`. UVs are normalized. The
  schema does not clamp numeric ranges unless stated.
- Names are case-sensitive references. Names, never in-memory IDs, go on disk.
- Writers should emit finite numbers and non-negative sizes, lengths, durations,
  FPS, weights, damping, and mass even where deserialization is more permissive.

## ZIP container

The current writer creates a Deflate ZIP. `project.json` is required UTF-8 JSON.
Image bytes are stored at `images/<Asset.file>`; nested relative paths work.
Directories and other entries are ignored and are not preserved by save. Writers
must reject absolute paths, drive prefixes, and `.`/`..` traversal components.
Archive order has no meaning; JSON array order does.

## Project

<!-- schema:Project.version --><!-- schema:Project.name --><!-- schema:Project.fps --><!-- schema:Project.assets --><!-- schema:Project.bones --><!-- schema:Project.slots --><!-- schema:Project.draw_order --><!-- schema:Project.skins --><!-- schema:Project.default_skin --><!-- schema:Project.constraints --><!-- schema:Project.constraint_order --><!-- schema:Project.animations --><!-- schema:Project.groups --><!-- schema:Project.psd_layer_paths --><!-- schema:Project.export_presets --><!-- schema:Project.extra -->

| Field | Type; presence/default | Meaning |
|---|---|---|
| `version` | unsigned integer; required | Must be `3` here. `0` is invalid; versions newer than the reader are refused. |
| `name` | string; required | User-visible project name; independent of filename. |
| `fps` | unsigned integer; required | Display/snapping rate. Keys remain in seconds and are not rescaled when FPS changes. Use greater than zero. |
| `assets` | `Asset[]`; default `[]` | Image library; asset names should be unique. |
| `bones` | `Bone[]`; default `[]` | Setup bones. Parent order is rebuilt topologically. |
| `slots` | `Slot[]`; default `[]` | Drawable anchors, in storage order. |
| `draw_order` | `string[]`; default `[]` | Setup slots back-to-front. Bad names are reported; unlisted slots are appended. |
| `skins` | `Skin[]`; default `[]` | Named attachment maps. |
| `default_skin` | string; default `""` | Skin name. Empty or unresolved selects the first skin, if any. |
| `constraints` | `Constraint[]`; default `[]` | All constraint kinds. |
| `constraint_order` | `string[]`; default `[]` | Evaluation order. Bad names are reported; unlisted constraints are appended. |
| `animations` | `Animation[]`; default `[]` | Named clips. |
| `groups` | `Group[]`; default `[]`, omitted empty | Editor folders only; no evaluation effect. |
| `psd_layer_paths` | object `{asset: layerPath}`; default `{}`, omitted empty | PSD provenance used to match layers during re-import. |
| `export_presets` | arbitrary JSON array; default `[]`, omitted empty | Opaque exporter configuration preserved without validation by `formats`. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. A key must not collide with a known field. |

## Assets and groups

<!-- schema:Asset.name --><!-- schema:Asset.file --><!-- schema:Asset.width --><!-- schema:Asset.height --><!-- schema:Asset.source_path --><!-- schema:Asset.extra -->

| `Asset` field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Texture identity referenced by attachments, sequences, and event audio. |
| `file` | string; required | Relative path below `images/`, not an OS path. |
| `width`, `height` | unsigned integer; required | Source pixel dimensions; should match decoded bytes. |
| `source_path` | string or `null`; default `null` | Advisory machine-local path for reload-from-source. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

Missing image bytes do not invalidate JSON; the asset loads without usable bytes.

<!-- schema:Group.name --><!-- schema:Group.color --><!-- schema:Group.members --><!-- schema:Group.parent -->

| `Group` field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Folder identity; should be unique. |
| `color` | RGBA; default `[0.55,0.58,0.65,1]` | Hierarchy display color. |
| `members` | `string[]`; default `[]` | Ordered `bone:<name>` or `slot:<name>` members. Invalid kinds/names are reported and skipped. |
| `parent` | string; default `""`, omitted empty | Enclosing group. Missing parents and cycles are reported and the bad edge is omitted. |

`Group` has no `extra`; its unknown fields are discarded on save.

## Skeleton

<!-- schema:Bone.name --><!-- schema:Bone.parent --><!-- schema:Bone.length --><!-- schema:Bone.tx --><!-- schema:Bone.ty --><!-- schema:Bone.rotation --><!-- schema:Bone.sx --><!-- schema:Bone.sy --><!-- schema:Bone.shear_x --><!-- schema:Bone.shear_y --><!-- schema:Bone.inherit_rotation --><!-- schema:Bone.inherit_scale --><!-- schema:Bone.inherit_reflect --><!-- schema:Bone.color --><!-- schema:Bone.extra -->

| `Bone` field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Unique bone identity. |
| `parent` | string; default `""` | Parent name; empty is root. Missing parent is reported and loads as root. |
| `length` | number; required | Setup length along local +X. |
| `tx`, `ty` | number; default `0` | Local setup translation. Root values are relative to world origin. |
| `rotation` | number; default `0` | Local setup rotation in degrees. |
| `sx`, `sy` | number; default `1` | Local scale. Negative reflects; zero is degenerate. |
| `shear_x`, `shear_y` | number; default `0` | Local shear angles in degrees. |
| `inherit_rotation` | boolean; default `true` | Include parent rotation in world composition. |
| `inherit_scale` | boolean; default `true` | Include parent scale. |
| `inherit_reflect` | boolean; default `true` | Include reflection from the parent's determinant. |
| `color` | RGBA or `null`; default `null` | Editor display color, not rendering tint. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

World transforms are derived and never serialized.

<!-- schema:Slot.name --><!-- schema:Slot.bone --><!-- schema:Slot.attachment --><!-- schema:Slot.color --><!-- schema:Slot.dark_color --><!-- schema:Slot.blend_mode --><!-- schema:Slot.extra -->

| `Slot` field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Unique slot identity. |
| `bone` | string; required | Transform bone. Missing bone is reported and the slot is skipped. |
| `attachment` | string or `null`; default `null` | Setup attachment lookup name in active skin; `null` draws nothing. |
| `color` | RGBA; default `[1,1,1,1]` | Multiplicative light tint and opacity. |
| `dark_color` | RGBA or `null`; default `null` | Optional two-color dark tint. |
| `blend_mode` | string; default `""` | `normal`, `additive`, `multiply`, `screen`; empty/unknown loads as `normal`. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

## Skins and attachment tag

<!-- schema:Skin.name --><!-- schema:Skin.entries --><!-- schema:Skin.bones --><!-- schema:Skin.constraints --><!-- schema:Skin.extra --><!-- schema:SkinEntry.slot --><!-- schema:SkinEntry.name --><!-- schema:SkinEntry.attachment -->

| Field | Type; presence/default | Meaning |
|---|---|---|
| `Skin.name` | string; required | Skin identity. |
| `Skin.entries` | `SkinEntry[]`; default `[]` | Lookup records keyed by `(slot,name)`. |
| `Skin.bones` | `string[]`; default `[]`, omitted empty | Bones active only with this skin; bad names are reported/skipped. |
| `Skin.constraints` | `string[]`; default `[]`, omitted empty | Skin-only constraints; bad names are reported/skipped. |
| `Skin` unknown fields | arbitrary JSON | Preserved. |
| `SkinEntry.slot` | string; required | Owning slot; unresolved entry is reported/skipped. |
| `SkinEntry.name` | string; required | Name used by slot and attachment timelines. |
| `SkinEntry.attachment` | tagged object; required | Attachment payload. `SkinEntry` itself has no unknown-field preservation. |

<!-- schema:Attachment.Region --><!-- schema:Attachment.Mesh --><!-- schema:Attachment.Clipping --><!-- schema:Attachment.Path --><!-- schema:Attachment.BoundingBox --><!-- schema:Attachment.Point -->

`attachment.type` is required: `region`, `mesh`, `clipping`, `path`,
`boundingbox` (no underscore), or `point`. Unknown tags are fatal JSON errors.

### Region and sequence

<!-- schema:Region.texture --><!-- schema:Region.offset_x --><!-- schema:Region.offset_y --><!-- schema:Region.rotation --><!-- schema:Region.scale_x --><!-- schema:Region.scale_y --><!-- schema:Region.width --><!-- schema:Region.height --><!-- schema:Region.uv --><!-- schema:Region.pivot_x --><!-- schema:Region.pivot_y --><!-- schema:Region.sequence --><!-- schema:Region.extra -->

| `Region` field | Type; presence/default | Meaning |
|---|---|---|
| `texture` | string; required | Asset name. Missing bytes yield an unavailable texture. |
| `offset_x`, `offset_y` | number; default `0` | Slot-local rectangle offset. |
| `rotation` | number; default `0` | Local degrees. |
| `scale_x`, `scale_y` | number; default `1` | Local scale; negative reflects. |
| `width`, `height` | number; required | Authored world-unit size; use positive values. |
| `uv` | four numbers; default `[0,0,0,0]` | `[u_min,v_min,u_max,v_max]`; normally each in `0..1`. |
| `pivot_x`, `pivot_y` | number; default `0.5` | Normalized pivot, `(0,0)` bottom-left; values may place it outside the image. |
| `sequence` | `Sequence` or `null`; default `null`, omitted absent | Optional texture animation. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

<!-- schema:Sequence.frames --><!-- schema:Sequence.fps --><!-- schema:Sequence.mode --><!-- schema:Sequence.setup_index -->

| `Sequence` field | Type; presence/default | Meaning |
|---|---|---|
| `frames` | `string[]`; default `[]` | Asset names in play order. |
| `fps` | number; default `0` | Advance rate; non-positive holds setup frame. |
| `mode` | string; default `""` | `hold`, `once`, `loop`, `ping_pong`, or the latter three with `_reverse`; unknown becomes `hold`. |
| `setup_index` | unsigned integer; default `0` | Initial frame, clamped to available range during evaluation. |

### Mesh and linked mesh

<!-- schema:Mesh.texture --><!-- schema:Mesh.vertices --><!-- schema:Mesh.uvs --><!-- schema:Mesh.triangles --><!-- schema:Mesh.edges --><!-- schema:Mesh.weights --><!-- schema:Mesh.linked --><!-- schema:Mesh.sequence --><!-- schema:Mesh.extra -->

| `Mesh` field | Type; presence/default | Meaning |
|---|---|---|
| `texture` | string; required | Sampled asset name. |
| `vertices` | `number[]`; default `[]` | Flat local `[x,y,...]`; odd tail is ignored. Ignored when `linked` resolves. |
| `uvs` | `number[]`; default `[]` | Flat normalized `[u,v,...]`, normally one pair per vertex. |
| `triangles` | unsigned integer array; default `[]` | Vertex indices in triples; indices should be in range. |
| `edges` | unsigned integer array; default `[]`, omitted empty | Preserved vertex-index pairs; use even length and valid indices. |
| `weights` | `[[[bone,weight],...],...]`; default `[]` | One influence list per vertex. Empty means rigid to slot bone. Weights should be non-negative and sum to 1 per vertex; missing bones are skipped/reported. |
| `linked` | `LinkedMesh` or `null`; default `null`, omitted absent | Borrow source mesh geometry. |
| `sequence` | `Sequence` or `null`; default `null`, omitted absent | Optional texture animation. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

<!-- schema:LinkedMesh.skin --><!-- schema:LinkedMesh.slot --><!-- schema:LinkedMesh.attachment --><!-- schema:LinkedMesh.inherit_deform -->

| `LinkedMesh` field | Type; presence/default | Meaning |
|---|---|---|
| `skin` | string or `null`; default `null`, omitted absent | Source skin; `null` means default skin. |
| `slot` | string; required | Source slot lookup part. |
| `attachment` | string; required | Source attachment lookup part; target must be a mesh. |
| `inherit_deform` | boolean; default `true` | Inherit deformation targeting the source. |

Weighted v3 deform uses one XY pair per influence (all influence lists in vertex
order). Rigid deform uses one XY pair per vertex.

### Other attachments

<!-- schema:BoundingBox.vertices --><!-- schema:BoundingBox.weights --><!-- schema:BoundingBox.extra --><!-- schema:Clipping.vertices --><!-- schema:Clipping.end_slot --><!-- schema:Clipping.extra --><!-- schema:Path.vertices --><!-- schema:Path.closed --><!-- schema:Path.constant_speed --><!-- schema:Path.extra --><!-- schema:Point.x --><!-- schema:Point.y --><!-- schema:Point.rotation --><!-- schema:Point.extra -->

| Field | Type; presence/default | Meaning |
|---|---|---|
| `BoundingBox.vertices` | flat XY array; default `[]` | Non-rendered polygon. |
| `BoundingBox.weights` | mesh weight shape; default `[]`, omitted empty | Optional skinning; empty is rigid. |
| `Clipping.vertices` | flat XY array; default `[]` | Mask polygon. |
| `Clipping.end_slot` | string or `null`; default `null` | Inclusive stopping slot; `null` clips remaining draw order. |
| `Path.vertices` | flat XY array; default `[]` | Curve points. |
| `Path.closed` | boolean; default `false` | Join last point to first. |
| `Path.constant_speed` | boolean; default `true` | Arc-length spacing when true; point-index parameterization otherwise. |
| `Point.x`, `Point.y` | number; default `0` | Non-rendered slot-local anchor. |
| `Point.rotation` | number; default `0` | Anchor degrees. |
| each object's unknown fields (`extra`) | arbitrary JSON | Preserved. |

## Constraints

<!-- schema:Constraint.name --><!-- schema:Constraint.kind --><!-- schema:Constraint.target --><!-- schema:Constraint.bones --><!-- schema:Constraint.bend_direction --><!-- schema:Constraint.mix --><!-- schema:Constraint.softness --><!-- schema:Constraint.stretch --><!-- schema:Constraint.stretch_limit --><!-- schema:Constraint.stiffness --><!-- schema:Constraint.transform_mix --><!-- schema:Constraint.offsets --><!-- schema:Constraint.local --><!-- schema:Constraint.relative --><!-- schema:Constraint.physics --><!-- schema:Constraint.forces --><!-- schema:Constraint.channels --><!-- schema:Constraint.slot --><!-- schema:Constraint.path --><!-- schema:Constraint.extra -->

| Field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Identity used by order, skins, timelines. |
| `type` | string; required | `ik`, `transform`, `path`, `physics`; unknown is reported/skipped. Rust calls this `kind`. |
| `target` | string; required | IK/transform target bone. For other kinds use `""`; schema still requires it. |
| `bones` | `string[]`; default `[]` | Constrained chain root-first; bad names are reported/removed. |
| `bend_direction` | number; default `1` | IK bend side; emit exactly `1` or `-1`. |
| `mix` | number; default `1` | IK blend, intended `0..1`. |
| `softness` | number; default `0` | Non-negative soft-reach distance. |
| `stretch` | boolean; default `false` | Permit unreachable IK chain to lengthen. |
| `stretch_limit` | number; default `1.1` | Maximum/natural chain-length ratio. |
| `stiffness` | number; default `0` | Long-chain pose retention, intended `0..1`. |
| `transform_mix` | `TransformMix` or `null`; default `null`, omitted absent | Per-channel transform blend; null becomes all zero. |
| `offsets` | seven numbers or `null`; default `null`, omitted absent | `[x,y,rotation,sx,sy,shear_x,shear_y]`; angles degrees; absent is zeros. |
| `local` | boolean; default `false`, omitted false | Transform constraint operates in local space. |
| `relative` | boolean; default `false`, omitted false | Apply target result relative rather than absolute. |
| `physics` | four numbers or `null`; default `null`, omitted absent | `[inertia,strength,damping,mass]`; use positive mass/non-negative damping. |
| `forces` | four numbers or `null`; default `null`, omitted absent | `[wind_x,wind_y,gravity_x,gravity_y]`. |
| `channels` | two booleans or `null`; default `null`, omitted absent | `[rotate,translate]` physics outputs. |
| `slot` | string or `null`; default `null`, omitted absent | Path attachment slot; missing/unresolved is reported. |
| `path` | four numbers or `null`; default `null`, omitted absent | `[position,spacing,rotate_mix,translate_mix]`; conversion default `[0,1,1,1]`. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. Known fields irrelevant to selected type are ignored by conversion. |

<!-- schema:TransformMix.rotate --><!-- schema:TransformMix.translate_x --><!-- schema:TransformMix.translate_y --><!-- schema:TransformMix.scale_x --><!-- schema:TransformMix.scale_y --><!-- schema:TransformMix.shear_x --><!-- schema:TransformMix.shear_y -->

`TransformMix` has `rotate`, `translate_x`, `translate_y`, `scale_x`, `scale_y`,
`shear_x`, and `shear_y`. Each number defaults to zero, is omitted when zero,
and is intended as a `0..1` blend (the schema does not clamp it).

## Animations and timelines

<!-- schema:Animation.name --><!-- schema:Animation.duration --><!-- schema:Animation.looping --><!-- schema:Animation.timelines --><!-- schema:Animation.events --><!-- schema:Animation.markers --><!-- schema:Animation.bone_offsets --><!-- schema:Animation.extra -->

| `Animation` field | Type; presence/default | Meaning |
|---|---|---|
| `name` | string; required | Clip identity. |
| `duration` | number; required | Seconds; use non-negative. Keys beyond it remain stored. |
| `looping` | boolean; default `true` | Runtime playback intent. |
| `timelines` | timeline array; default `[]` | Property tracks; sort each key list by time. |
| `events` | `Event[]`; default `[]`, omitted empty | Runtime triggers. |
| `markers` | `Marker[]`; default `[]`, omitted empty | Editor-only ruler notes, never runtime export. |
| `bone_offsets` | `BoneOffset[]`; default `[]`, omitted empty | Per-bone sample-time shifts. |
| unknown fields (`extra`) | arbitrary JSON | Preserved. |

<!-- schema:Timeline.BoneTranslate --><!-- schema:Timeline.BoneTranslate.bone --><!-- schema:Timeline.BoneTranslate.axis --><!-- schema:Timeline.BoneTranslate.keys --><!-- schema:Timeline.BoneTranslate.extra --><!-- schema:Timeline.BoneRotate --><!-- schema:Timeline.BoneRotate.bone --><!-- schema:Timeline.BoneRotate.keys --><!-- schema:Timeline.BoneScale --><!-- schema:Timeline.BoneScale.bone --><!-- schema:Timeline.BoneScale.axis --><!-- schema:Timeline.BoneScale.keys --><!-- schema:Timeline.BoneScale.extra --><!-- schema:Timeline.BoneShear --><!-- schema:Timeline.BoneShear.bone --><!-- schema:Timeline.BoneShear.axis --><!-- schema:Timeline.BoneShear.keys --><!-- schema:Timeline.BoneShear.extra --><!-- schema:Timeline.SlotColor --><!-- schema:Timeline.SlotColor.slot --><!-- schema:Timeline.SlotColor.keys --><!-- schema:Timeline.SlotVisible --><!-- schema:Timeline.SlotVisible.slot --><!-- schema:Timeline.SlotVisible.keys --><!-- schema:Timeline.SlotAttachment --><!-- schema:Timeline.SlotAttachment.slot --><!-- schema:Timeline.SlotAttachment.keys --><!-- schema:Timeline.DrawOrder --><!-- schema:Timeline.DrawOrder.keys --><!-- schema:Timeline.IkMix --><!-- schema:Timeline.IkMix.constraint --><!-- schema:Timeline.IkMix.keys --><!-- schema:Timeline.IkBendDirection --><!-- schema:Timeline.IkBendDirection.constraint --><!-- schema:Timeline.IkBendDirection.keys --><!-- schema:Timeline.IkSoftness --><!-- schema:Timeline.IkSoftness.constraint --><!-- schema:Timeline.IkSoftness.keys --><!-- schema:Timeline.TransformConstraintMix --><!-- schema:Timeline.TransformConstraintMix.constraint --><!-- schema:Timeline.TransformConstraintMix.keys --><!-- schema:Timeline.Deform --><!-- schema:Timeline.Deform.slot --><!-- schema:Timeline.Deform.attachment --><!-- schema:Timeline.Deform.keys -->

| required `kind` | Target and keys | Value |
|---|---|---|
| `bone_translate` | `bone:string`, `axis:x|y` default x, `keys:ScalarKey[]` | Setup-relative translation on one axis. Preserves unknown fields. |
| `bone_rotate` | `bone`, `ScalarKey[]` | Setup-relative degrees; shortest-arc sampling. |
| `bone_scale` | `bone`, axis default x, `ScalarKey[]` | Independently keyed scale axis. Preserves unknown fields. |
| `bone_shear` | `bone`, axis default x, `ScalarKey[]` | Setup-relative shear degrees. Preserves unknown fields. |
| `slot_color` | `slot`, `ColorKey[]` | Continuous RGBA tint. |
| `slot_visible` | `slot`, `VisibleKey[]` | Stepped visibility. |
| `slot_attachment` | `slot`, `AttachmentKey[]` | Stepped attachment lookup or none. |
| `draw_order` | `DrawOrderKey[]` | Setup-relative slot movement. |
| `ik_mix` | `constraint`, `ScalarKey[]` | IK mix. |
| `ik_bend_direction` | `constraint`, `ScalarKey[]` | Discrete `1`/`-1`. |
| `ik_softness` | `constraint`, `ScalarKey[]` | World-unit softness. |
| `transform_constraint_mix` | `constraint`, `MixKey[]` | Seven mix channels. |
| `deform` | `slot`, `attachment`, `DeformKey[]` | Mesh local XY offsets. |

Unresolved timeline targets are reported and the timeline is skipped. Unknown
`kind` is fatal. `axis` accepts only `x`/`y`; omission is `x`.

### Interpolation and keys

<!-- schema:Axis.X --><!-- schema:Axis.Y --><!-- schema:Interp.Linear --><!-- schema:Interp.Stepped --><!-- schema:Interp.Bezier --><!-- schema:Interp.Bezier.handles -->

Continuous keys flatten interpolation: absent or `"curve":"linear"` is linear;
`"curve":"stepped"` holds; Bézier requires `"curve":"bezier"` plus
`"handles":[out_x,out_y,in_x,in_y]`. X handles are time-span fractions intended
in `0..1`; Y handles are value-span fractions and deliberately unbounded for
overshoot. Unknown curves are fatal.

<!-- schema:ScalarKey.time --><!-- schema:ScalarKey.value --><!-- schema:ScalarKey.interp --><!-- schema:Vec2Key.time --><!-- schema:Vec2Key.x --><!-- schema:Vec2Key.y --><!-- schema:Vec2Key.interp --><!-- schema:ColorKey.time --><!-- schema:ColorKey.value --><!-- schema:ColorKey.interp --><!-- schema:MixKey.time --><!-- schema:MixKey.value --><!-- schema:MixKey.interp --><!-- schema:MixKey.extra --><!-- schema:VisibleKey.time --><!-- schema:VisibleKey.value --><!-- schema:AttachmentKey.time --><!-- schema:AttachmentKey.value --><!-- schema:DrawOrderKey.time --><!-- schema:DrawOrderKey.offsets --><!-- schema:DeformKey.time --><!-- schema:DeformKey.offsets --><!-- schema:DeformKey.interp -->

| Key | Required fields and meaning |
|---|---|
| `ScalarKey` | `time:number`, `value:number`; flattened interpolation defaults linear. |
| `Vec2Key` | `time`, `x`, `y`; interpolation defaults linear. No current v3 timeline uses it, but it remains a schema type. |
| `ColorKey` | `time`, `value:RGBA`; interpolation defaults linear. |
| `MixKey` | `time`, flattened `TransformMix`; interpolation defaults linear; unknown fields preserved. |
| `VisibleKey` | `time`, `value:boolean`; discrete, no curve. |
| `AttachmentKey` | `time`, `value:string|null`; discrete; null clears. |
| `DrawOrderKey` | `time`, `offsets:[[slot,signedInteger],...]`; offsets are from setup order. |
| `DeformKey` | `time`, `offsets:number[]`; interpolation defaults linear; length follows rigid/weighted mesh rule above. |

Times are seconds from clip start. Duplicate or unsorted times deserialize but
are ambiguous for authoring; writers should emit strictly ascending keys.

### Events, markers, bone offsets

<!-- schema:Event.time --><!-- schema:Event.name --><!-- schema:Event.int_value --><!-- schema:Event.float_value --><!-- schema:Event.string_value --><!-- schema:Event.audio --><!-- schema:Event.volume --><!-- schema:Event.balance --><!-- schema:Marker.time --><!-- schema:Marker.name --><!-- schema:Marker.color --><!-- schema:BoneOffset.bone --><!-- schema:BoneOffset.offset -->

| Field | Type; presence/default | Meaning |
|---|---|---|
| `Event.time`, `Event.name` | number, string; required | Runtime trigger time and identity. |
| `Event.int_value` | signed integer; default `0`, omitted zero | Integer payload. |
| `Event.float_value` | number; default `0`, omitted zero | Float payload. |
| `Event.string_value` | string; default `""`, omitted empty | Text payload. |
| `Event.audio` | string; default `""`, omitted empty | Asset name; runtime owns playback. |
| `Event.volume` | number; default `1`, omitted one | Gain, conventionally `0..1`. |
| `Event.balance` | number; default `0`, omitted zero | Stereo balance, conventionally `-1..1`. |
| `Marker.time`, `Marker.name` | number, string; required | Editor-only position and label. |
| `Marker.color` | RGBA; default `[0.95,0.72,0.30,1]` | Editor display. |
| `BoneOffset.bone` | string; required | Shifted bone; missing name reported/skipped. |
| `BoneOffset.offset` | number; required | Seconds; positive trails, negative leads. |

These objects do not preserve unknown fields.

## Preservation, errors, and recovery

Unknown fields survive only on types explicitly described with `extra` above.
Other structs discard them. Unknown enum tags fail. Object member order and
whitespace do not matter; every array and tuple position described above does.

`LoadReport.dangling` records unresolved parents, bones, slots, draw/constraint
order, skin members, group members/parents/cycles, constraint targets/types,
path slots, timeline targets, and bone offsets. The loader skips the smallest
invalid relationship or object and continues. Missing required fields, wrong
types, invalid tags/version, invalid JSON/UTF-8, and missing `project.json` are
fatal. Native `.ankh` load does not populate the foreign-import `lossy` list.

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

See the checked-in
[parseable example](https://github.com/Ankhimate/editor/blob/main/docs/examples/minimal-v3.json)
and [migrations and compatibility](/formats/migrations/).
