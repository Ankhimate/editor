# The template context

What a template can address, field by field. This is the reference the export
panel's **Context** button shows live against your own rig — use that while
writing, and this for the details.

> **This is a public contract.** Renaming a field here breaks templates people
> have already written, silently, because there is no compiler on that side. It
> carries `context_version`; additions are free, renames are breaking.

## Syntax in one screen

Templates are [Handlebars](https://handlebarsjs.com/guide/). Everything outside
`{{…}}` is written out verbatim, so a template *looks like* the file it produces.

```hbs
{
  "bones": [
{{#each skeleton.bones}}    {"name":"{{name}}","rotation":{{round rotation 3}}}{{#unless @last}},{{/unless}}
{{/each}}  ]
}
```

- `{{field}}` — insert a value.
- `{{#each list}}…{{/each}}` — loop. Inside, fields are relative to the item;
  `{{../thing}}` reaches back out.
- `{{#if x}}…{{else}}…{{/if}}`, `{{#unless x}}` — conditionals. An empty list is
  false, which is how you skip an empty channel.
- `@first`, `@last`, `@index` — position in a loop. `{{#unless @last}},{{/unless}}`
  is how you avoid a trailing comma.

**A misspelled field is an error, not an empty string.** `{{nmae}}` stops the
export and tells you the line. This is deliberate: the alternative is a file that
looks fine and has a bone with no name in it.

## Helpers

| Helper | Does |
|---|---|
| `{{deg r}}` / `{{rad d}}` | Convert angles. The context is already in degrees; `rad` is for engines that want radians |
| `{{round v places}}` | Trim float noise. Keeps exports diffable |
| `{{json x}}` | Dump a whole subtree as JSON. The escape hatch when a loop is not worth writing |
| `{{numbers x places}}` | Like `json` for a number array, but rounded. `json` widens an `f32` to `f64` and prints `0.4000000059604645`; `round` takes one number and cannot be mapped over a list |
| `{{hex color}}` | `[1,0,0.5,1]` → `ff0080ff` |
| `{{pad n width}}` | `{{pad 7 4}}` → `0007`, for frame numbers |
| `{{len x}}` | Length of a list, object or string |
| `{{eq a b}}`, `{{ne a b}}`, `{{or a b …}}` | Comparisons, for use inside `{{#if}}` |
| `{{add a b}}`, `{{sub}}`, `{{mul}}`, `{{div}}` | Arithmetic |

Helpers compose as arguments: `{{round (deg rotation) 2}}`.

## Root

| Key | What |
|---|---|
| `context_version` | This document's version. `1` today |
| `project` | `name`, `fps`, `version` |
| `skeleton` | The rig — see below |
| `animations` | Every clip, as a list |
| `atlas` | The baked atlas. **Absent** when the preset bakes none — guard with `{{#if atlas}}` |
| `export` | `output_dir`, `preset_name`, `template_name` |
| `animation` | **Only** in a `per: animation` template — the one clip being written |

There is deliberately **no timestamp**. One would make every export differ from
the last, which destroys diffs and makes "did the rig actually change?"
unanswerable in version control.

## `skeleton`

`bones[]`, `slots[]`, `draw_order[]` (slot names), `skins[]`, `default_skin`,
`constraints[]`, `constraint_order[]`.

### `skeleton.bones[]`

Ordered **parents before children**, so a runtime that applies transforms in
array order can just walk it.

| Field | Notes |
|---|---|
| `name`, `parent` | Names. `parent` is `""` for a root |
| `index`, `parent_index` | Position in this array. `parent_index` is `-1` for a root — a number, not null, so it drops straight into a numeric field |
| `children[]` | Child names. Handlebars cannot invert a parent pointer, so this is provided |
| `length` | |
| `x`, `y` | Local position |
| `rotation` | **Degrees** |
| `scale_x`, `scale_y` | |
| `shear_x`, `shear_y` | Degrees |
| `inherit_rotation`, `inherit_scale`, `inherit_reflect` | |
| `color` | RGBA, or null |

### `skeleton.slots[]`

`name`, `bone`, `attachment` (setup attachment name, may be null), `color`,
`dark_color`, `blend` (`normal` / `additive` / `multiply` / `screen`).

### `skeleton.skins[]`

`name`, `bones[]`, `constraints[]`, and `entries[]` of
`{ slot, name, attachment }`.

`slots[]` holds **the same entries grouped by slot** — `{ slot, attachments[] }`,
each attachment `{ name, attachment }`. Use it for a format that nests
attachments as `slot: { name: {…} }`: walking the flat `entries[]` emits a slot's
key once per attachment, which is valid JSON that a parser silently collapses to
whichever came last. Handlebars cannot group, so the grouping ships.

Every attachment has a `type`; the rest depends on it.

| `type` | Fields |
|---|---|
| `region` | `texture`, `x`, `y`, `rotation`, `scale_x`, `scale_y`, `width`, `height`, `source_width`, `source_height`, `uv`, `pivot_x`, `pivot_y`, `sequence` |
| `mesh` | `texture`, `vertices[]`, `uvs[]`, `triangles[]`, `edges[]`, `edges_x2[]`, `weights[]`, `flat_vertices[]`, `weighted`, `vertex_count`, `hull`, `source_width`, `source_height`, `scaled_width`, `scaled_height`, `linked`, `sequence` |
| `clipping` | `vertices[]`, `vertex_count`, `end_slot` |
| `bounding_box` | `vertices[]`, `vertex_count`, `weights[]`, `weighted` |
| `path` | `vertices[]`, `vertex_count`, `closed`, `constant_speed` |
| `point` | `x`, `y`, `rotation` |

`vertices` and `uvs` are flat: `[x, y, x, y, …]`.

### Sizes, and which one a format wants

A region's `width`/`height` are its size in **rig space**, and `scale_x`/`scale_y`
the scaling the artist applied. Those two are the rig's own truth and most
formats — Spine among them — want them passed straight through.

`source_width`/`source_height` are the image file's own pixel size, offered for a
format that addresses the file instead. On a rig authored against half-resolution
art the two disagree by the art scale. They fall back to the declared size when
the asset is unknown, so they are always safe to address.

Do **not** reconstruct a scale by dividing rig size by file size. Odd pixel
dimensions make it fractional — a genuinely half-scale rig reports 1.9778 on one
attachment and 2.0 on the next — and it overwrites any scale the artist actually
authored. This exporter shipped that bug; every region landed slightly off.

A mesh is the other way round. Its vertices are already rig-space, so a format
declaring a mesh's dimensions alongside them wants `scaled_width`/`scaled_height`
— the file size multiplied by the rig's art scale — not the file's own numbers.
`source_width`/`source_height` remain available for the UV-space case.

**Weights come pre-packed**, because restructuring nested arrays is the one thing
a logic-less template genuinely cannot do:

```
weights[i] = { count: 2, bones: [ {bone, x, y, weight}, {bone, x, y, weight} ] }
```

`x`/`y` are the vertex expressed in **that influence's bone space** — not the
mesh's, and not repeated across influences.

The rig stores one position per vertex, in the space of the bone its slot hangs
from, because `core` skins by transforming that single position through each
bound bone. A runtime that stores weights per influence expects the inverse
already applied, so skinning is a weighted sum. The context does that conversion:
`vertex × host_bone.world × influence_bone.world⁻¹`, over the setup pose.

Two bones therefore give a vertex two different coordinates, and they agree only
when the bones coincide. This shipped writing the shared mesh-space position for
every influence: a vertex bound to one bone survived, one bound to two bones far
apart did not, and every weighted mesh on a real rig came out scattered.

`flat_vertices` is the same data as **one flat number array**, bones addressed by
index: per vertex either a plain `x, y` pair when unweighted, or a count followed
by `bone_index, x, y, weight` per influence. Several formats specify exactly this
encoding, and a template can produce neither the flattening nor the name-to-index
resolution.

`edges` is flat vertex-index pairs; `edges_x2` is the same list with each index
doubled, for formats that address the flat vertex array by component offset.

When a mesh has **no authored edges** — every mesh imported from a format that
did not carry them — both fields fall back to the mesh's **boundary**, computed
from the triangulation: an edge belonging to exactly one triangle is on the
perimeter, one belonging to two is interior. Emitting nothing instead is read by
consumers as "the edge structure was dropped in transit"; Spine warns "mesh
internal edges lost" and rebuilds its own triangulation.

`hull` is how many leading vertices form the outline, which formats storing an
outline require to come **first** in the array. It is the boundary vertex count
when the boundary really is the leading run `0..n`, and `vertex_count` otherwise.
Vertex *order* cannot reveal a perimeter — `editor/src/meshgen.rs` explains why
guessing from it is unsound — but the triangulation can, so the boundary is
computed and then *checked* to be a prefix. Ankhimate's tracer builds meshes
contour-first, so it normally is. When it is not, reporting every vertex as hull
costs the consumer its interior edges; claiming a hull that slices the mesh in
the wrong place would be far worse.

### `skeleton.constraints[]`

Always `name`, `type` (`ik` / `transform` / `physics` / `path`), `target`,
`bones[]`, `mix`. Then, by type:

- **ik** — `bend_direction`, `softness`, `stretch`, `stretch_limit`, `stiffness`
- **transform** — `mixes`, `drives` (see below),
  `offsets {x, y, rotation, scale_x, scale_y, shear_x, shear_y}`, `local`, `relative`
- **physics** — `physics {inertia, strength, damping, mass}`,
  `forces {wind_x, wind_y, gravity_x, gravity_y}`, `channels {rotate, translate}`
- **path** — `slot`, `path {position, spacing, mix_rotate, mix_translate}`

Branch with `{{#if (eq type "ik")}}`.

### A transform constraint's mixes are per axis

**Every channel mixes per axis wherever it has axes.** Rotation is one angle, so
one number; translate, scale and shear have two each:

```
mixes { rotate,
        translate_x, translate_y,
        scale_x, scale_y,
        shear_x, shear_y,
        translate, scale, shear }
```

The uniform rule is the point. Spine mixes rotate with one number, translate and
scale with two, and shear with one — an asymmetry that has to be memorised, and
one that leaves shear's second axis with no mix at all. A constraint here can
follow its target horizontally and ignore it vertically, mix `scaleX` without
`scaleY`, or shear on x — none of which Spine can express.

The three unsuffixed names — `translate`, `scale`, `shear` — are for a format
with one mix per channel. Each reports the **driven** axis: the non-zero one when
the axes differ, so a template printing `mixes.translate` into a single slot
still gets the number that makes the constraint work. They are also what a
template written before the axes were split already addresses, which is why
splitting them needed no `CONTEXT_VERSION` bump.

**`drives`** mirrors the same set as booleans — `rotate`, `translate_x`,
`translate_y`, `scale_x`, … plus the unsuffixed `translate` / `scale` / `shear`
meaning "either axis", and `any` for the whole constraint. Guard each field you
emit with its own flag:

```
{{#if drives.translate_x}}, "mixX": {{round mixes.translate_x 4}}{{/if}}
```

A mix of 0 contributes nothing, so a channel the artist left alone must not be
declared. Formats that name the driven channels separately from the amounts —
Spine among them — treat a named channel as switched *on*: declaring one at 0
made a constraint copy its target's scale and shear and stretched every bone it
governed.

Each constraint also carries **`not_last`** — whether another constraint of a
kind this context knows about follows it. Use it as the separator when emitting
them as one array:

```
{{#each skeleton.constraints}}{{#if (eq type "ik")}}{ … }{{#if not_last}},{{/if}}
{{/if}}…{{/each}}
```

`@last` is wrong here. It marks the last *constraint*, not the last one your
template renders, so a kind you have no branch for emits nothing while still
consuming a comma — and the array ends `…}, ]`, which is a parse error rather
than a wrong number. Grouping by kind avoids that but breaks something worse:
constraints solve in authored order, and interleaving kinds is meaningful. A
shoulder transform that runs before an aim IK gives a different pose than one
that runs after. Keep the order, use `not_last`.

## `animations[]`

`name`, `duration` (seconds), `looping`, then timelines grouped by target:

- `bones[]` — `{ name, offset, translate_x[], translate_y[], rotate[],
  scale_x[], scale_y[], shear_x[], shear_y[] }`, plus the merged `translate[]`,
  `scale[]` and `shear[]` described below. A channel is **absent** when the clip
  does not key it, so guard with `{{#if}}`.
- `slots[]` — `{ name, channel, keys[] }` where channel is `color`, `visible` or
  `attachment`.

  ### A two-axis property is two tracks

  Translate, scale and shear are each **two independent tracks**: `translate_x`
  and `translate_y` have their own key times *and* their own easing. An animator
  can key x at frame 3 and y at frame 7, or ease one and leave the other linear.

  Spine cannot express either — its two curves per property share the keyframe's
  times — so a format targeting it reads the merged `translate[]`, `scale[]` and
  `shear[]` instead. Those carry one key per pair in the old shape
  (`{time, x, y, curve, has_next}`), built by unioning the two tracks' times.

  **The merge is lossy and the split channels are not.** Where one axis keys and
  the other does not, the other is sampled linearly, so a bezier on the axis
  without a key contributes a straight line; and one key holds one `curve`, so
  the merged view carries x's. Read `translate_x` / `translate_y` for a format
  with two curves, and `translate` only for one that stores pairs.
- `deform[]` — `{ slot, attachment, keys[] }`, each key with flat `offsets[]`.
- `draw_order[]` — `{ time, offsets[] }`.
- `ik[]`, `transform[]` — constraint channels over time. `ik[]` is one entry per
  *channel* (`mix`, `softness`, `bend_direction`).
- `ik_by_constraint[]` — the same IK timelines merged into one key list per
  constraint, each key carrying whichever channels are keyed at that time. Use
  this for the common format that writes `"constraint": [ { time, mix, softness } ]`;
  a channel with no key at that time is **absent**, not defaulted.

  **`bend_direction` is the exception**: it is always present, seeded from the
  constraint's own setup value and overwritten where the animation keys it.
  Ankhimate has no bend-direction timeline in most rigs — which way a chain bends
  is a property of the constraint — but formats that read it per key default a
  missing one to "positive". Leaving it absent straightened every backward-
  bending knee for the length of the animation while the setup pose stayed
  correct, so the export looked right until it moved.

  A merged key's **`points`** is every channel's control points concatenated —
  `mix` first, then `softness` — and each channel also has its own
  `mix_points` / `softness_points`. `points` is present whenever *any* channel is
  a bezier, with linear channels contributing their straight line: a format that
  reads one curve array per key reads it positionally, so a short array
  misassigns every number after the gap. That is exactly the defect that reached
  Spine as `[error] Invalid curve` — a four-number softness curve written where
  eight were expected.
- `events[]` — `{ time, name, int, float, string, audio, volume, balance }`.

Every key has `time` and a `curve`:

- `"linear"` or `"stepped"` — a plain string.
- Bezier — an object: `{ type: "bezier", handles: [4], out_x, out_y, in_x, in_y }`.

So `{{curve}}` always prints something, and `{{#if curve.handles}}` detects a
bezier.

### Which key a curve belongs to

**A key's `curve`, `points` and `is_bezier` describe the segment *leaving* it.**

The schema stores easing the other way round: `ScalarKey::interp` is how that key
is *arrived at*, so the handles authored on key `i` describe the span
`i-1 → i`. Most published formats — Spine among them — hang the curve on the key
that starts the segment, so the context shifts frames by one for you. A template
reads the key it is on and needs no lookahead.

This is worth stating because getting it wrong is close to invisible: keyframe
poses stay exactly right, since a curve only affects values *between* keys. Only
the frames in between drift. The exporter shipped reading the wrong key, which
left the first key of every track linear and moved every other curve one key
late — 126 wrong curves in one animation, and a rig that posed correctly on every
key and moved wrongly between them.

Bone and IK keys carry **`has_next`** — whether another key follows on the same
channel. **Guard every curve with it.** A curve describes the interpolation
*towards the next key*, so one written on the last key sends the reader looking
for a frame that does not exist; Spine answers `[error] Invalid curve` and then
a null-frame NPE. `points` is already absent there, but `curve` is per-key in the
schema regardless of position, so a template branching on `"stepped"` needs
`has_next` to know when to stay silent:

```
{{#if has_next}}{{#if points}}, "curve": {{numbers points 4}}{{else}}{{#if (eq curve "stepped")}}, "curve": "stepped"{{/if}}{{/if}}{{/if}}
```

Bone and IK keys additionally carry **`points`** — the same bezier as control
points in *absolute* time/value space, four numbers for a scalar channel and
eight for a two-axis one (x's pair first, then y's). It is `null` on a linear or
stepped key, and on the last key of a channel, which has nothing to interpolate
towards.

Reach for `points` whenever the target format stores curves as absolute
coordinates — most do. `handles` are normalized 0..1 across the span to the next
key, so printing them into an absolute slot yields a file that parses, imports,
and animates *wrongly*. The conversion needs the next key's time and value, and a
template cannot look ahead inside `{{#each}}`; that is why it is computed here.

**Not exported:** ruler markers (editor-only notes, deliberately withheld) and
per-bone track offsets — those are *baked into key times* before you see them, so
the secondary motion an animator authored reaches the game without every preset
needing to reimplement it.

## `atlas`

Absent unless the preset bakes one.

- `pages[]` — `{ index, width, height, file }`
- `regions[]` — `{ name, page, x, y, width, height, offset_x, offset_y, original_width, original_height, rotated }`

`name` is the asset name, which is what an attachment's `texture` field holds —
that is the join.

`offset_*` and `original_*` are how a trimmed sprite gets placed where the
untrimmed one sat. A format that writes only the packed rect renders every
trimmed part shifted, and it looks like a rigging mistake rather than an export
one.

## Writing a per-animation file

Set the template's cadence to **per animation** and use `{{animation}}`:

```hbs
Path:  anim/{{animation.name}}.json
Body:  {"name":"{{animation.name}}","duration":{{round animation.duration 3}}}
```

The output path is a template too, which is the whole mechanism — no extra
setting, no naming convention to learn.

## Rules the exporter enforces

- **Paths cannot escape the output folder.** A path rendering to `../…`, an
  absolute path, or a drive letter aborts the export. This is not paranoia:
  `anim/{{animation.name}}.json` renders a name from the rig, and rigs arrive
  from other people.
- **All or nothing.** Every file renders to memory first. A template that fails
  on the ninth animation leaves the folder untouched rather than half-written.
- **Nothing is ever deleted.** Files already there that this export does not
  write are reported as orphans. Renaming an animation leaves the old file
  behind, and only you know whether that matters.
- **Two templates cannot claim one path** — the second would silently overwrite
  the first, so the export stops and names both.
