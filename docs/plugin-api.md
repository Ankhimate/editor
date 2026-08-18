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

Deliberately few. These are what a script needs to *build* a rig; verbs that act
on a selection or a tool live in the editor, because a selection is something
only an editor has.

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

## Still to come

- **A UI surface** — panels and menu entries a plugin can add
  (`docs/plugin-plan.md` step 7).
- **A host** — QuickJS, so plugins are written in JavaScript rather than Rust
  (step 6).
- **An MCP server** — the same verbs and the same read surface over a transport
  (step 8).

None of those add a fifth vocabulary. They are consumers of this one.
