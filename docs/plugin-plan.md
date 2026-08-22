# Plugins, and the headless surface underneath them

Status: all eight stages landed. This document retains the sequence and the
reasoning that fixed the order; `docs/plugin-api.md` is the public contract.

## The thesis

Ankhimate cannot know which engine a rig targets, and the list is not closeable.
That is already why **export is a format editor** rather than a set of exporters
(`docs/export-plan.md`). Import has the same shape, and so does every "can it
also do X" a user will ask. The deliverable is the **engine for writing
extensions**, not the extensions.

Blender is the model, not VSCode. Both are extensible; only one is a native
application whose plugins can override built-in behaviour, and that capability
comes from **registries**, not from the UI toolkit. Every extensible thing is
looked up by name, and built-ins register through the same door an addon uses.
An enum closes the set at compile time and no plugin host can reopen it.

## The four surfaces

A plugin API is not "a panel API". It is four things, and a panel is the least
important:

1. **Commands** — the verbs. Register new ones, shadow built-ins.
2. **Document read** — the nouns. Query bones, slots, skins, timelines.
3. **Session read** — selection, playhead, mode. Never saved, never undone.
4. **UI** — panels, menu entries, keybindings. Declarative; the host draws.

Deliberately absent: a document *write* surface. Plugins never mutate `doc`
directly — every mutation is a command, which is what keeps undo honest and
keeps a plugin from corrupting a rig. This is the rule `CLAUDE.md` already
imposes on panels, extended to plugins.

## Why MCP is last, and why it still shapes the design

An MCP server — "make me an animation without opening the editor" — is **not a
separate road**. It needs the same four surfaces, minus the UI one. It is
another consumer of the plugin API, alongside JS and the editor's own menus.

Two consequences:

- A plugin that registers `import.dragonbones` is reachable from the editor's
  File menu, from JS, *and* from MCP, with nobody writing MCP support for it.
- Two separate action vocabularies would drift, exactly as the Edit menu had
  already drifted from the keymap before stage 3 removed the duplication.

So MCP goes last — but the **headless/session split it forces** goes first,
because retrofitting it after plugins depend on the surface is not possible.

## What the split actually is

`AppState::dispatch` does more than apply a command. `after_document_change`
(`app_state.rs:827`) does four things:

| | Needed headless? |
|---|---|
| `revision` bump | No — editor bookkeeping (autosave's dirty check) |
| `prune_selection` | No — session only |
| `rebind_meshes` | **Yes** — binds are derived and not serialized; a weighted mesh without them is broken |
| `refresh_pose` | On demand, not eagerly |

That third row is the finding. The split is otherwise clean, but `rebind_meshes`
is document integrity rather than editor convenience, and it has to travel with
the document operators or every headless mesh edit silently produces a broken
rig.

Operators divide the same way:

- **Document operators** — `bone.create`, `key.insert`. Mutate the document,
  need no session. Headless-capable.
- **Session operators** — `tool.select`, `gizmo.rotate`, `view.toggle_bones`.
  Meaningless without a UI.

An operator declares which it is, the way `requires_mode` already declares
Setup-vs-Animate. Of the 21 built-ins today, roughly 8 are document-level and 13
are session-level.

## Sequence

### 1. Split the operator surface

Move the registry out of `editor` into a crate both a headless binary and the
editor can depend on. Operators declare document-level vs session-level.
`rebind_meshes` travels with the document side.

Unblocks everything below. Nothing else can be built correctly first.

**Check:** a document operator runs against a bare `Document` with no `Session`
and no `History` in scope.

**Measured before starting, and cheaper than this plan first assumed.** The
command layer's only dependency on session state is `WorkMode` — a bare enum
with a `label()` and nothing else. Every other mention of `session` in
`commands/` is test code or the word in prose. `doc.rs` imports `ankhimate_core`
alone, and `commands/mod.rs` imports `Document` and `BoneId`.

So `EditCommand`, its 82 impls, `Document` and `WorkMode` move down together and
compile framework-free without rewriting. What stays behind is the ~16 of 21
built-in operators that touch tools, gizmo modes and selection, plus the three
`AppState` fields (`session`, `physics`, `pose`) that are derived or
interaction state by construction.

The invasive part is not the split; it is that `AppState::dispatch` is the only
sanctioned mutation path and headless callers need an equivalent that keeps
undo, `requires_mode` and `rebind_meshes` without owning a `Session`.

### 2. Arguments and schema

```rust
fn schema(&self) -> serde_json::Value;
fn invoke(&self, ctx: &mut Ctx, args: &Value) -> OpResult;
```

Today every operator reads live selection from `AppState` — fine for a
keybinding, useless for a caller that wants "create a bone named `spine` under
`root`". One piece of work, three consumers: JS plugins, MCP, and macros.

### 3. Wrap the document commands as operators

82 `EditCommand` structs have no verb wrapper. Large but mechanical.

Ordering note: extract the shape from **two** real cases before generalising.
The `ImportPlugin` trait has the same rule and is why the second importer is a
prerequisite rather than a nicety — a trait guessed from one sample encodes that
sample's accidents.

### 4. Document read surface

The public contract, in the same sense `docs/export-context.md` is one: a rename
breaks user plugins silently, with no compiler on that side. Slowest and most
deliberate step; everything else is mechanical by comparison.

### 5. Format registry

Importers and exporters by name, built-ins registered through the same door.
Exporters are already half-way there — `presets/mod.rs:74` `builtin()` is the
shape. Importers are a hardcoded call at `fileops.rs:112`.

**Blocked on a second importer.** See step 3's ordering note.

### 6. JS host

`rquickjs` (QuickJS). Small, embeds cleanly, sandboxing is straightforward. No
JIT, which does not matter: plugins are gesture-bound and IO-bound, never in the
hot loop.

Rejected: `deno_core`/V8 (huge dependency, complicates `wasm32`), Boa (slower,
spec-incomplete in corners).

**The hard boundary: nothing plugin-side runs inside `evaluate()`.** That is what
keeps determinism (PLAN §2.6) and the `wasm32` target intact.

### 7. Panels

A **widget vocabulary**, not a plugin-draws-egui API. The plugin returns a
declarative widget list; the host draws it. That dissolves the immediate-mode
problem, keeps plugin code out of the paint loop, and matches what Blender
actually does — Python addons call `layout.prop()`, they do not draw pixels.

Cost, stated plainly: a thumbnail-strip picker only exists if the host ships a
thumbnail-strip widget. That is why Blender addon UIs all look alike. It is the
right trade.

Frame-by-frame animation is the motivating case, and most of it is *not* a
plugin problem: attachment timelines already exist, and onion skinning,
image-sequence import and the hotkeys are core editor work.

### 8. MCP server

**Landed:** `ankhimate-mcp` is an rmcp-based stdio server. It deliberately
advertises nine coarse tools: open/new, describe, list verbs, run a sandboxed
script, save, export, render a frame, and render a contact sheet. Render results
are MCP image content, backed by the reusable `ankhimate-render` layer rather
than protocol code. The rig stays open across calls. Native `.ankh`, Spine,
DragonBones, and PSD all enter through the shared importer registry. The full
render/focus contract is in [mcp.md](mcp.md).

A new binary over `core` + `formats` + `export` and the document operators. No
winit, no UI thread, no `&mut AppState`.

Small once 1–4 exist — a transport and a tool list.

**Never writes in place.** No undo and no editor to inspect the damage, so an
LLM that mangles a rig has mangled the file. Open, mutate in memory, write where
the caller names — the same rule export already follows.

Honest expectation: an LLM will not author a good walk cycle, because timing is
the whole skill. It will do rig structure from a description, mechanical
animation (blinks, holds, loops), bulk edits across many rigs, retargeting, and
rough first drafts to fix by hand.

## What is deliberately not on this list

- **Electron.** Would cost the whole application and buy only plugin-authoring
  convenience. Canvas rendering would still be WebGPU; the maths would still
  cross a boundary every frame; `core`'s guarantees, the determinism story and
  the `wasm32` target would all be discarded. VSCode's modularity comes from its
  extension host and a stable API, not from the renderer being HTML.
- **Auto-exposing the whole registry over MCP.** A faithful mirror of every
  command is a worse tool surface than a deliberate coarse one. "Move the bone a
  bit left" is not expressible in a tool call; "mirror the limbs" is.
