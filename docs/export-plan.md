# Export — plan

> Status: proposed. Nothing here is built yet. This document is the argument for
> a shape; `docs/TASKS.md` gets the task entries once the shape is agreed.

## The requirement

Export must be **user-authorable**. Ankhimate does not know which engine a user
targets, and the list of engines is not closeable — Godot, Unity, Phaser, bevy,
LÖVE, a hand-rolled C++ renderer, an engine that does not exist yet. An exporter
per engine is a treadmill the project loses: every one is a maintenance burden,
and the format a user actually needs is always the one not shipped.

So the deliverable is not "more exporters". It is **an export format editor**,
with the built-in formats authored in that same editor. If Ankhimate's own
runtime format cannot be expressed as a user template, the template system is too
weak and we would have shipped a second-class extension point.

## The decision: a template engine as the substrate

Three approaches were considered.

**Text templates + an expression language** *(chosen)*. The user writes the
output file as literal text with placeholders and loops over the document. It is
deterministic, has no sandbox to get wrong, is inspectable and diffable, and the
failure mode is a bad string rather than a hung editor. Its ceiling is text: it
cannot emit binary formats, and it cannot compute.

**Embedded scripting (Rhai/Lua)**. A script receives the document and emits
files. It buys arbitrary logic and binary output. It costs a sandbox, execution
limits, a whole language's error surface to document and support, and it puts
non-deterministic code inside a pipeline whose determinism is a stated invariant
(PLAN §2.6). Exporters are a data walk, not a computation; this pays a permanent
cost for a capability the task rarely needs.

**A field-mapping UI with no code**. Pick fields, rename keys, choose JSON or XML.
Approachable, nothing to learn. The ceiling is far too low to be the substrate:
Godot `.tres` and Unity's format are not "JSON with renamed keys". The first
format it cannot express leaves the user with no escape hatch, which is the exact
opposite of the requirement.

**These are not exclusive, and that decides it.** Templates are the substrate;
the other two ride on top.

```
Template engine   ← everything compiles to this; one execution path
   ├─ Field-mapping UI  → generates a template. Beginners never see the syntax.
   └─ Template editor   → live preview, for anything the mapper cannot say.
```

The mapper is a **view of a template**, not a parallel pipeline. "Edit as
template" drops the user into text pre-filled with what the mapper produced —
a ramp from no-code to full control, rather than two dead ends. Scripting stays
available as a later escape hatch if a real format demands it; adding it does not
invalidate anything here.

The mapper ships **after** the engine, once the engine's shape is known from real
use. Building it first would freeze guesses about what templates need.

## Syntax

Handlebars, via the `handlebars` crate.

Writing a bespoke parser is a week of work plus a permanent maintenance surface,
to arrive at something users have to learn from scratch. Handlebars is widely
known, logic-less by design (which matches "a data walk, not a computation"),
and its restrictions are the ones we would have imposed anyway.

### Verified before committing to it

The plan rests on this crate, so its load-bearing behaviour was checked against
`handlebars 6.4.3` rather than assumed.

**Strict mode is mandatory, and it is good.** Default (non-strict) renders a
missing field as an empty string — `[{{nope}}]` on an empty context yields
`[]`. That is the corrupt-export-that-looks-fine failure: an empty string where
a bone name belongs. With `set_strict_mode(true)` the same template errors, and
the error locates itself:

```
Error rendering "t" line 1, col 23: Failed to access variable in strict mode Some("nope")
```

Template name, line, column. That is enough to underline the mistake in the
editor's template pane, which T-603d needs.

**Helpers compose, and the awkward cases work.** One render exercised nested
helper calls, comma placement and parent scope together:

```hbs
{"bones":[{{#each bones}}{"n":"{{name}}","r":{{round (deg rot) 2}},"p":"{{../unit}}"}{{#unless @last}},{{/unless}}{{/each}}]}
```

→ `{"bones":[{"n":"root","r":0.0,...},{"n":"spine","r":90.0,...}]}`

So: `{{round (deg rot) 2}}` composes helpers as arguments; `@last` places commas
without trailing-comma JSON errors; `../unit` reaches the enclosing scope from
inside a loop. Output paths template through the same engine —
`anim/{{animation.name}}.json` → `anim/walk.json` — which is what makes
`per: animation` file naming work with no extra machinery.

**Rendering is byte-deterministic.** Same template, same context, identical
output across renders — the property `evaluate()` is held to (PLAN §2.6), now
confirmed to survive through export.

The remaining unknown is whether `handlebars` pulls a heavier dependency tree
than the workspace wants. Check at the point it is added; it is a
proportionate-cost question, not a blocker.

### Helper set

| Helper | Use |
|---|---|
| `{{deg r}}` / `{{rad d}}` | Angle unit conversion, both directions |
| `{{round v places}}` | Trim float noise; keeps exports diffable |
| `{{json this}}` | Escape hatch — dump a subtree verbatim |
| `{{eq a b}}`, `{{#if}}`, `{{#unless}}` | Conditionals, e.g. omit an empty timeline |
| `@index`, `@first`, `@last` | Separators and indices inside `{{#each}}` |
| `{{pad n width}}` | Zero-padded frame numbers in filenames |

## What a template sees

A single JSON-shaped context, built once per export and reused for every
template in the set — one construction, one set of semantics to document.

**The context is a public contract.** Once users have templates, renaming a
field breaks their work silently, and unlike our own code there is no compiler
to catch it. It carries `context_version`, and changing a field name is a
breaking change subject to the same discipline as the file format (ADR 0004).
Additive fields are safe; that asymmetry should be exploited — when unsure,
ship the smaller surface and add later.

### Root

| Key | Notes |
|---|---|
| `context_version` | Integer. Bump on any breaking change to this table |
| `project` | `name`, `fps`, `version` |
| `skeleton` | `bones[]`, `slots[]`, `draw_order[]`, `skins[]`, `constraints[]`, `groups[]` |
| `animations[]` | See below |
| `atlas` | `pages[]`, `regions[]` — absent if the preset bakes no atlas |
| `export` | `output_dir`, `preset_name`, `template_name` |
| `animation` | **Only** under `per: animation` — the one being rendered |

`export` carries **no timestamp**. A timestamp in the context makes every export
differ from the last, which destroys diffability and makes "did the rig actually
change?" unanswerable in version control. Users who want one can add it in their
build script, where it belongs.

### Bone

Mirrors `formats::schema::Bone`, which is already the name-keyed, degrees-on-disk
shape this needs (ADR 0004, PLAN §2.7) — so the context is a projection of the
save schema, not a third representation to keep in sync.

`name`, `parent` (name; empty for root), `length`, `tx`, `ty`, `rotation` (deg),
`sx`, `sy`, `shear_x`, `shear_y` (deg), `inherit_rotation`, `inherit_scale`,
`inherit_reflect`, `color`.

Plus two the schema does not store, because a template cannot compute them and
every consumer needs them:

- `index` — position in `update_order`. Engines that store bones as a flat array
  with integer parent references need this; deriving it in a template is not
  possible.
- `children[]` — child names. Handlebars cannot invert a parent pointer.

### Slot / attachment

Slot: `name`, `bone`, `attachment` (setup name, may be empty), `color`,
`dark_color`, `blend`.

Attachment: `name`, `type` (`region` | `mesh` | `clipping` | `bounding_box` |
`point` | `path`), `path` (the atlas region key), transform, and per type:
mesh `vertices[]`, `uvs[]`, `triangles[]`, `weights[]`, `hull`; clipping `end_slot`
+ `vertices[]`; sequence `frames`, `fps`, `mode`.

**Weights are emitted in the standard packed form** — per vertex, a count then
that many `(bone_index, x, y, weight)` tuples — not our internal layout. Every
runtime format expects some variant of this, and a template cannot restructure
nested arrays. Doing it in Rust once beats every user reimplementing it never.

### Animation

`name`, `duration`, `events[]`, `markers[]`, and timelines grouped by kind:
`bones[]` (each `{ name, translate[], rotate[], scale[], shear[] }`), `slots[]`
(`attachment[]`, `color[]`), `deform[]`, `draw_order[]`, `ik[]`, `path[]`,
`physics[]`.

Each key: `time`, `value(s)`, `curve` (`linear` | `stepped` | `bezier`), and for
bezier the four control values. Grouping by kind rather than a flat list matches
how every runtime format is written and how a template needs to walk it.

**Per-bone track offsets are resolved into key times before the context is
built.** They are an editor authoring convenience (Phase 9); no runtime format
has a concept for them, and leaving them for a template to apply guarantees
every preset gets it wrong. Bake, don't export.

### Atlas region

`name` (attachment name — the join key), `page`, `x`, `y`, `width`, `height`,
`offset_x`, `offset_y`, `original_width`, `original_height`, `rotated`.

Trim offsets and original size are what let a runtime place a trimmed sprite
where the untrimmed one sat. Emitting the packed rect alone silently shifts every
trimmed attachment — a bug that looks like a rigging error, so it must not be
the template author's problem.

## Scope of one export

A format is rarely one file. Godot wants a `.tres` plus the atlas; a JSON runtime
wants `skeleton.json` + `atlas.png` + `atlas.json`; some engines want a file per
animation.

So an **export preset** is:

```jsonc
{
  "preset_version": 1,
  "name": "Godot 4",
  "output_dir": "export/godot",          // relative to the .ankh, or absolute
  "atlas": {
    "enabled": true,
    "trim": true,
    "padding": 2,
    "extrude": 1,                        // duplicate edge pixels; kills bleed at non-integer zoom
    "max_page": 2048,
    "power_of_two": false,
    "allow_rotation": true
  },
  "templates": [
    { "output_path": "{{project.name}}.tres", "per": "once",      "body": "..." },
    { "output_path": "anim/{{animation.name}}.tres",
      "per": "animation", "body": "..." }
  ],
  "copy_images": false                   // raw source images alongside; for no-atlas presets
}
```

Fields worth their justification:

- **`extrude`** is separate from `padding` and both are needed. Padding spaces
  regions apart; extrude duplicates each region's edge pixel outward. Without
  extrude, a renderer sampling at non-integer zoom pulls in the neighbouring
  sprite and every attachment gets a faint halo — the classic atlas-bleed bug,
  reported as "my sprites have coloured edges" and near-impossible for a user to
  diagnose.
- **`allow_rotation`** must be a toggle, not always-on. It packs tighter, but
  some runtimes cannot un-rotate a region, and a template author has no way to
  compensate.
- **`preset_version`** for the same reason the context has one — presets are user
  data that outlives the version that wrote them.

Presets are stored **in the project** under a new `export_presets` key (a rig's
export settings belong to the rig, and `Extra` on `Project` means an older
Ankhimate round-trips them rather than dropping them). They also import/export as
standalone `.ankhpreset` JSON, so one preset serves a studio's whole project set.

**Template bodies are stored inline, not as file paths.** A path makes a project
non-portable — send the `.ankh` to a colleague and the export silently breaks.
Inline costs some duplication across projects, which is what the standalone
preset file is for.

`per: animation` renders the template once per animation, with `{{animation}}`
bound to that one — this is what makes per-animation files expressible without
scripting.

## Writing files is the dangerous part

Rendering is pure; writing is not. Export points a user-authored path template at
the filesystem, which is the one place in this project where a bug destroys work
that was never ours to lose.

**Paths are confined to the output directory.** `output_path` is a template, so
it can render to anything — `../../.bashrc`, `C:\Windows\...`, or with a
traversal hidden inside a bone name. Every rendered path is normalised and
verified to stay under `output_dir`; one that escapes aborts the export. This is
not hypothetical: a rig from an untrusted source could carry an attachment named
`../../..%2fpayload`, and presets are meant to be shared between studios, so both
halves can arrive from elsewhere.

**Export is all-or-nothing.** Render every template to memory first; write only
once all of them succeed. A template that fails on animation nine must not leave
eight files from this run beside four from the last, which is a state no user can
reason about and no runtime can load.

**Nothing is deleted, ever.** A stale-file sweep of `output_dir` is the obvious
convenience and it is refused: `output_dir` is user-chosen and can be pointed at
a source tree by accident. Report orphans instead — list what is present but
unwritten and let the user decide. Overwriting a file this export produces is
fine; removing one it does not is not our call.

**Overwrite is surfaced, not silent.** The pre-export summary names how many
files will be created versus replaced. Writing over a hand-edited file with no
warning is the same class of loss.

## Order of work

Atlas first: templates reference atlas regions, so a template engine with no
atlas can only emit half a format.

### T-603a — Atlas bake
`export/src/atlas.rs`. Trim transparent borders, pack (MaxRects/skyline),
padding + extrude, power-of-two toggle, multi-page. Emits `atlas.png` (+`_2`…)
and a region table: rect, trim offset, original size, rotation flag.

Framework-free — no wgpu. This is CPU image work over `AssetDb`, so it is
testable headless and in CI.

**Accept:** every attachment resolves to a region; a trimmed region's offsets
reconstruct the original placement exactly (round-trip test); a rig that
overflows one page produces two and every region still resolves; **packing the
same asset set twice yields identical pages** (a hash-order-dependent packer
makes every export a spurious diff); a fully-transparent image degrades to a
1×1 region rather than a zero-area rect or a panic.

### T-603b — Template engine
`export/src/template.rs` + `context.rs`. Context builder, helper set, strict
missing-field errors, render-to-files. Deterministic: same document, same bytes.

**Accept:** a fixture rig renders byte-identically across runs and platforms;
a missing field is an error with template name, line and field, not an empty
string; a template that emits a file per animation produces exactly one per
animation; **a path template escaping `output_dir` aborts the export and writes
nothing**; **a template failing mid-set leaves the output directory untouched**
(render-all-then-write, verified by a template that throws on the last
animation); a rig with zero animations renders without error.

### T-603c — Native runtime format, authored as a template
Ankhimate's own `skeleton.json`, written as a preset rather than as Rust.

This is the dogfooding gate and it is deliberately load-bearing: if our own
format needs a feature the engine lacks, the engine is short and we find out
before users do. It also gives users a known-good, complete template to clone.

**Accept:** the shipped preset renders a runtime file for the sample rig that a
schema test validates; **loading it and evaluating gives the same pose as
evaluating the source document** at ten sampled times, ε documented — the only
check that proves the format is complete rather than merely well-formed; any
feature the template cannot express is written down in this file as an engine
gap, not worked around in Rust.

### T-603d — Editor UI
Preset list, template editor pane, live preview against the current document,
inline errors, "export now". Output-path preview so a user sees the file set
before writing anything.

The preview is what makes the feature learnable. Authoring a format blind and
discovering the mistake in an engine's import error is a loop long enough that
users give up; rendering against the open rig as they type turns it into an
edit-and-look. Preview renders the **real** context, never a sample — a preview
that disagrees with the export is worse than none.

A **context browser** beside the editor lists what is available at the cursor's
scope. Without it the only way to learn the context is this document, and users
will not read it.

Note the egui trap already paid for in this repo (`CLAUDE.md`): a text field
rebuilt from the document each frame swallows keystrokes. The template buffer
lives in `ui.data` while editing.

**Accept:** editing a template updates the preview without writing files; a
broken template shows the error at its line and never writes a partial file set;
the pre-export summary names created-vs-replaced counts before anything is
written; a template surviving a project save/load round-trip renders identically.

### T-603e — Preset library
Godot, Unity, Phaser, generic JSON. Each authored clean-room from published,
publicly documented format specs — never by reading another editor's exporter.

**Clean-room applies with full force here** (PLAN §0). Several established
editors are GPL-3.0, and an exporter is exactly the artefact where copying is
tempting and detectable. A target format is implemented from its **own engine's**
public documentation. Where a format is another tool's proprietary schema, the
answer is that we do not ship that preset — a user may author it themselves,
which is precisely why the feature is user-authorable.

**Accept:** each preset produces a file that its target engine's documented
schema accepts; each carries a comment naming the public spec it was written
from; loading a preset's output in the target engine is verified once by hand
and the result recorded.

### T-604 — `ankhimate-runtime`
Unchanged from `docs/TASKS.md`: loads the T-603c output, `AnimationState`,
crossfade, events, physics ownership, `Vec<DrawBatch>`. No wgpu; builds for
`wasm32`.

## Explicitly not in this plan

**T-601/T-602 — PNG sequence, spritesheet, video.** A different pipeline
entirely: headless wgpu render of evaluated poses to frames, then ffmpeg over
stdin. No template involvement. Real and wanted, but orthogonal — sequencing it
here would only delay the format work.

**Binary output.** Text templates cannot express it. If a target demands it, that
is the argument for the scripting escape hatch, and it should be made with a
concrete engine as the evidence.

## Where this leaves the crate boundary

`export` gains `image` and `handlebars` and stays free of egui and wgpu. It
depends on `core` and on `formats` (for the schema types the context mirrors).
The editor drives it; the crate itself runs headless, which is what makes CI
tests and a future CLI exporter possible without a display.

`core` is untouched. Nothing here belongs in it: the context is a projection of
the *disk* schema, in degrees, by name — `formats`' concern, not `core`'s.

## Open questions

Decisions this plan does not make, flagged so they are made deliberately at the
point they bite rather than by accident.

**Does the context own baked variants?** Baking IK to bone keys, baking physics,
and flattening composed skins are all in the original T-603. They are
transformations of the *document*, not of the template, so they belong in the
preset's options and must run before the context is built. Unresolved: whether
that is preset config (simple, but every preset repeats it) or a separate
pre-export pipeline stage the preset selects. Decide at T-603b, when the context
builder's shape is real.

**How large a rig does the preview tolerate?** Re-rendering every template on
every keystroke is fine for the sample rig and possibly not for a 200-bone one.
Measure before optimising; the fix (debounce, or preview one template) is easy
and picking it now would be guessing.

**Does a preset need per-animation selection?** Exporting a subset of animations
is a real need, and it could live in the preset or in the export dialog. Dialog
is the better guess — it is a per-run choice, not a property of the format — but
it is not decided.

## Consequences for the rest of the project

Two things elsewhere become load-bearing once this ships.

**`docs/TASKS.md` T-603 is superseded.** Its single "runtime export + atlas bake"
entry becomes T-603a–e. T-604 and T-605 are unchanged; T-605's format-spec work
gains a second target, since the template context now needs the same normative
field table `.ankh` gets.

**The README's "not there yet" changes meaning.** Today it says the export
pipeline is unwritten. After T-603c it says image/video export is unwritten,
which is a materially smaller claim — and the sentence "a rig cannot leave the
editor for an engine" stops being true, which is the point of the whole phase.
