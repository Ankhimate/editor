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

Every attachment has a `type`; the rest depends on it.

| `type` | Fields |
|---|---|
| `region` | `texture`, `x`, `y`, `rotation`, `scale_x`, `scale_y`, `width`, `height`, `uv`, `pivot_x`, `pivot_y`, `sequence` |
| `mesh` | `texture`, `vertices[]`, `uvs[]`, `triangles[]`, `edges[]`, `weights[]`, `weighted`, `vertex_count`, `linked`, `sequence` |
| `clipping` | `vertices[]`, `vertex_count`, `end_slot` |
| `bounding_box` | `vertices[]`, `vertex_count`, `weights[]`, `weighted` |
| `path` | `vertices[]`, `vertex_count`, `closed`, `constant_speed` |
| `point` | `x`, `y`, `rotation` |

`vertices` and `uvs` are flat: `[x, y, x, y, …]`.

**Weights come pre-packed**, because restructuring nested arrays is the one thing
a logic-less template genuinely cannot do:

```
weights[i] = { count: 2, bones: [ {bone, x, y, weight}, {bone, x, y, weight} ] }
```

`x`/`y` repeat the vertex position per influence, which is the convention every
runtime format uses.

### `skeleton.constraints[]`

Always `name`, `type` (`ik` / `transform` / `physics` / `path`), `target`,
`bones[]`, `mix`. Then, by type:

- **ik** — `bend_direction`, `softness`, `stretch`, `stretch_limit`, `stiffness`
- **transform** — `mixes {rotate, translate, scale, shear}`,
  `offsets {x, y, rotation, scale_x, scale_y, shear_x, shear_y}`, `local`, `relative`
- **physics** — `physics {inertia, strength, damping, mass}`,
  `forces {wind_x, wind_y, gravity_x, gravity_y}`, `channels {rotate, translate}`
- **path** — `slot`, `path {position, spacing, mix_rotate, mix_translate}`

Branch with `{{#if (eq type "ik")}}`.

## `animations[]`

`name`, `duration` (seconds), `looping`, then timelines grouped by target:

- `bones[]` — `{ name, offset, translate[], rotate[], scale[], shear[] }`. A
  channel is **absent** when the clip does not key it, so guard with `{{#if}}`.
- `slots[]` — `{ name, channel, keys[] }` where channel is `color`, `visible` or
  `attachment`.
- `deform[]` — `{ slot, attachment, keys[] }`, each key with flat `offsets[]`.
- `draw_order[]` — `{ time, offsets[] }`.
- `ik[]`, `transform[]` — constraint channels over time.
- `events[]` — `{ time, name, int, float, string, audio, volume, balance }`.

Every key has `time` and a `curve`:

- `"linear"` or `"stepped"` — a plain string.
- Bezier — an object: `{ type: "bezier", handles: [4], out_x, out_y, in_x, in_y }`.

So `{{curve}}` always prints something, and `{{#if curve.handles}}` detects a
bezier.

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
