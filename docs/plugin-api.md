# The plugin API

What a plugin, a script or an MCP client can do to a rig, and what it can ask
about one. Four surfaces, of which three exist today.

> **This is a public contract.** Operator ids, argument names and the fields
> `describe` returns are all referred to by string, and there is no compiler on
> the caller's side. Additions are free; renames break someone's plugin silently.
> The rule `docs/export-context.md` states applies here for the same reason.

## The shape

```rust
let ops = DocOps::builtin();
let mut edit = Edit::default();

ops.invoke("bone.create", &mut edit,
    &Args::from_json(json!({ "name": "root" })))?;
ops.invoke("bone.create", &mut edit,
    &Args::from_json(json!({ "name": "spine", "parent": "root", "y": 40.0 })))?;

let rig = ankhimate_document::describe(&edit.doc);
edit.undo();
```

No editor, no window, no GPU. `ankhimate-document` is the whole dependency.

## 1. Verbs

A verb is a dotted id, a JSON Schema, and an invocation. `DocOps::builtin()`
carries the built-ins; `ids()` lists them and `get(id).schema()` describes one.

| Id | Does | Mode |
|---|---|---|
| `bone.create` | Add a bone, optionally under a parent | Setup |
| `bone.set_transform` | Move, turn or scale a bone | Setup |
| `bone.rename` | Rename a bone | Setup |
| `bone.delete` | Delete a bone and its subtree | Setup |
| `slot.create` | Add a slot on a bone | Setup |
| `anim.create` | Add an animation clip | either |
| `asset.add_image` | Add an image, bytes base64-encoded | Setup |
| `attachment.create_region` | Put a sprite in a slot | Setup |
| `attachment.create_mesh` | Put a mesh in a slot, weighted or rigid | Setup |
| `anim.key_bone` | Key translate / rotate / scale / shear | either |
| `anim.key_attachment` | Key which attachment a slot shows | either |
| `import.report` | Say what an import could not carry across | either |
| `constraint.create_ik` | IK over a chain, with a target bone at its tip | Setup |
| `constraint.create_transform` | One bone drives others | Setup |
| `constraint.create_physics` | A bone that sways, lags and settles | Setup |
| `constraint.set_ik` | Retune mix, bend, softness, stretch | either |
| `constraint.set_transform` | Retune per-axis mixes and offsets | either |
| `constraint.set_physics` | Retune inertia, damping, mass, gravity, wind | either |
| `constraint.delete` | Remove a constraint by name | Setup |

Verbs that act on a *selection* or a tool live in the editor instead, because a
selection is something only an editor has.

### Creating and configuring are separate verbs

`constraint.create_ik` takes the chain; `constraint.set_ik` does not. Changing
what a constraint acts on is a different decision from changing how strongly it
acts, and a `set` that accepted both would let a typo in `bones` silently
rebuild the constraint rather than fail.

**An argument left out keeps its current value.** "Leave it" and "reset it to a
default you never saw" are different answers, and only the first is what a
partial edit means.

The set is sized by a rule, not by taste. `docs/export-plan.md` requires our own
runtime format to be a template, so a format the engine cannot express is found
before a user finds it; the import side needs the same guarantee, and the way to
get it is that a shipped importer must be writable as a plugin. These verbs are
what that turned out to require.

### Arguments name things, they do not hold ids

Every reference is a **name**. Slotmap keys are not stable across sessions
(ADR 0004) — a plugin that stored one would break on the next load, and undo
hands a restored bone a *new* key, which is why `IdRemap` exists at all. Names
resolve at invoke time, and a name the rig does not have is an error the caller
sees rather than a silent no-op.

### Absent is not the same as wrong

An omitted argument takes its default. An argument of the wrong *type* is an
error, because a misspelled key quietly taking a default is how an afternoon
disappears.

`bone.set_transform` goes further: an omitted field keeps its current value
rather than zeroing it, so one axis can be nudged without restating the other
five.

### Errors say which argument

`OpError` distinguishes three cases, and each names what went wrong:

- `Args(Missing | WrongType | Unresolved)` — the call could not be read.
- `Refused(WrongMode)` — the mode rule said no, and says which mode was wanted.
- `Unknown(id)` — no verb answers to that.

## 2. Reading

`describe(&doc)` returns **the template context** — the same JSON tree
`docs/export-context.md` documents, field for field.

That is deliberate. A separate read vocabulary would be a second public contract
to keep in step with the first, and the two would drift the way the Edit menu
drifted from the keymap before the registry existed. Someone who has written an
exporter already knows this API.

`names(&doc)` is the cheap version: bones, slots, skins and animations by name.
It answers the question that precedes every edit — a verb names its target, so
"what is there to name?" comes first.

**Angles are degrees.** `core` works in radians; the contract does not
(PLAN §2.7).

## 2b. Reading a layered document

Three functions, and only one of them is about Photoshop:

```js
const psd     = ankhimate.readPsd(base64);       // layers + tags + inference
const layer   = ankhimate.parseTags("cape [bone][physics:cloth]");
const result  = ankhimate.infer(layers);         // over layers you built
```

The **tag grammar** — `[bone]`, `[frames]`, `[physics:cloth]` — and
**inference** — is this group a chain or a scatter, is this run of numbered
layers a flipbook, which layers mirror which — are not PSD features. They are a
vocabulary for saying what a layer means and a set of questions about a layer
tree. A plugin importing a layered TIFF, an Aseprite file or a directory of
numbered PNGs wants both.

Exposing them is what stops `[bones]` meaning one thing in the built-in importer
and another in an addon.

### What a layer looks like

```json
{
  "path": "arm [bone]/upper",
  "name": "upper",
  "raw_name": "upper",
  "depth": 1,
  "is_group": false,
  "visible": true,
  "bounds": [0, 28, 73, 73],
  "tags": { "bone": null, "slot": "upper" },
  "unknown_tags": ["wobble"],
  "bone": true,
  "sequence": null,
  "mirrors": null
}
```

`name` is the name with tags stripped — what the bone or slot is called. A
plugin should not have to know the grammar to find that out.

`tags` gives `null` for a bare tag, so "present with no value" and "not present"
stay distinguishable.

`unknown_tags` are handed over rather than dropped: a plugin is the one consumer
that can define new ones.

**`path` carries the raw names**, tags included — `arm [bone]/upper`, not
`arm/upper`. That is a wart, not a decision: a re-import matches on the path, so
adding a tag to a group renames every path beneath it. It is unchanged because
`psd_layer_paths` is already saved in that shape.

### Guesses

`readPsd` and `infer` both return `guesses` beside `layers`:

```json
{
  "path": "face",
  "decided": "`face` is one bone, not 3",
  "because": "its 3 layers scatter in two directions rather than lying along one…",
  "override_with": "[bones] on `face`"
}
```

Every guess carries its evidence and the tag that would say otherwise. A plugin
that shows these lets the artist disagree; one that does not is deciding
silently, which is worse than not guessing at all.

### Failures throw

A bad PSD raises a JS `Error` rather than returning null. A plugin that ignored
a returned error would go on to build a rig out of nothing, and a stack trace
naming the line is the difference between finding that in a second and finding
it in an hour.

## 3. What is deliberately absent

**A write surface.** Nothing hands out a `&mut Document`. Every mutation is a
command, which is what keeps undo honest and what stops a plugin corrupting a
rig — the rule `CLAUDE.md` already imposes on panels, extended outward.

**The live pose.** `describe` reports what a rig *is*, not where it is posed.
Sampling a pose is `core::pose::evaluate`, a different question with a different
cost.

**Session state.** Selection, tools, the playhead. A headless caller has none,
and a verb that needed one could not be reached from here — the compiler
enforces that, since `DocOperator` is handed an `Edit` and never an `AppState`.

## 4. Undo works

Every verb goes through `Edit::dispatch`, which is `AppState::dispatch` without
the UI. So a plugin's edit is undoable and mode-checked exactly as a menu's is,
and `edit.undo()` walks back through them.

The Setup/Animate rule (T-207) reaches a script too. It is a property of the
command, not of the editor, so `Edit::mode` decides and a structural edit in
Animate is refused with the mode it wanted.

## 5. In JavaScript

`ankhimate-plugins` binds the same API to QuickJS. A plugin is a `.js` file:

```js
ops.invoke("bone.create", { name: "root" });
ops.invoke("bone.create", { name: "spine", parent: "root", y: 40 });

for (const bone of rig().skeleton.bones) {
  console.log(bone.name + " at " + bone.rotation + " degrees");
}
```

| Global | Is |
|---|---|
| `ops.list()` | Every verb id, so a plugin discovers rather than hardcodes |
| `ops.schema(id)` | What that verb takes |
| `ops.invoke(id, args)` | Run it. Throws on a bad argument or a refused mode |
| `rig()` | The read surface, as the template context |
| `names()` | Bones, slots, skins and animations by name |
| `console.log(msg)` | Comes back to the host as a line |
| `ankhimate.registerImporter(spec)` | Declare a rig format this plugin reads |
| `ankhimate.sidecar(name)` | A text file beside the imported one |
| `ankhimate.sidecarBytes(name)` | The same, base64, for images |
| `ankhimate.sidecars()` | What is beside the imported file |

### An importer is a plugin

```js
ankhimate.registerImporter({
  id: "import.mine", label: "My Format", extensions: ["mine"],
  read(text, fileName) {
    const rig = JSON.parse(text);
    const png = ankhimate.sidecarBytes(rig.atlas);
    if (png) ops.invoke("asset.add_image", { name: "atlas", bytes_base64: png });

    for (const b of rig.bones)
      ops.invoke("bone.create", { name: b.name, parent: b.parent });

    ops.invoke("import.report", {
      what: "constraint", where: "spine_ik", detail: "IK is not read yet",
    });
  },
});
```

It builds the rig by calling verbs rather than constructing a document — which
gives it a property the built-in Rust readers do not have: **the import is a run
of commands, so it undoes.** Those replace the document wholesale and cannot.

`import.report` is what keeps a plugin honest. An import that drops half a file
quietly is worse than one that refuses, and the report survives an undo because
it is not part of the rig.

### An exporter is a plugin too

A Handlebars preset stays the right tool when a format is a projection of the
context. A plugin is for when it is not — a checksum over what was written, a
binary header, an index built by counting, a layout that depends on the rig
rather than on the template.

```js
ankhimate.registerExporter({
  id: "export.mine", label: "My Engine",
  write() {
    const r = rig();
    emit("rig.json", JSON.stringify(r.skeleton));
    emit("manifest.txt", "bones=" + r.skeleton.bones.length);
  },
});
```

| Global | Is |
|---|---|
| `ankhimate.registerExporter(spec)` | Declare a format this plugin writes |
| `emit(path, contents)` | A text file, relative to the output directory |
| `emitBytes(path, base64)` | A binary one |
| `bakeAtlas(settings)` | Pack the rig's images; pages as base64 PNG, regions as metadata |

`bakeAtlas` is what lets a plugin produce a real engine format. Most runtime
formats want a packed atlas, and a script has neither pixels nor a rectangle
packer — writing one in JS would be slower and worse than the baker that already
ships. Settings are `trim`, `padding`, `extrude`, `max_page`, `power_of_two` and
`allow_rotation`; omitted ones take the same defaults a preset gets.

```js
const atlas = bakeAtlas({ padding: 2 });
for (const page of atlas.pages) emitBytes(`atlas_${page.index}.png`, page.png_base64);
emit("atlas.json", JSON.stringify(atlas.regions));
```

A bake that fails returns `{ error }` rather than throwing, so a rig with one
undecodable image lets a plugin write the rest and say what was missing — the
same choice the importers make with their report.

**A plugin never touches the disk.** It emits, and the host builds the same
`Plan` the template path produces — so path confinement, the all-or-nothing
write and never-delete all still hold. Those are `docs/export-plan.md`'s rules
and handing a script a file handle would put every one of them in the plugin
author's hands.

A script that throws halfway emits nothing: the files it managed to produce are
discarded with the plan, because a half-written export is one the user has to
notice is half-written.

### Importers can take options

An importer declares `options_schema()` the way an operator declares `schema()`.
Most take none — Spine and DragonBones read what the file says. PSD takes four:
`scale`, `skip_hidden`, `include` and `flatten`.

The distinction that matters is **parameters, not a conversation**. Every option
has a default that produces a usable rig, so an unattended caller — a script, an
MCP client — imports without a UI and the editor's panel refines rather than
supplies. An importer that genuinely needed a dialogue could not be registered
at all, and would stay panel-only.

### Sidecars are not a filesystem

`ankhimate.sidecar` and `sidecarBytes` reach files in the imported rig's own
directory and nowhere else. The host fixes the directory; only a bare file name
comes from the script, and separators, `..`, absolute paths and drive prefixes
are refused rather than resolved.

A failed verb **throws** rather than returning nothing, so a plugin can `try`
around it and a mistake does not continue silently over an edit that never
happened.

### The sandbox is what is absent

There is no `require`, no `process`, no filesystem, no network and no clock. A
QuickJS context starts with none of them and this crate binds none — so a plugin
can reach the rig and nothing else.

### Never in the hot loop

A plugin runs on a gesture or an import, never inside `evaluate()`. That is the
boundary the whole design protects: script in the pose loop would take
determinism (PLAN §2.6) with it.

## Still to come

- **A UI surface** — panels and menu entries a plugin can add
  (`docs/plugin-plan.md` step 7).
- **An MCP server** — the same verbs and the same read surface over a transport
  (step 8).

Neither adds a vocabulary. They are consumers of this one.
