# What Ankhimate does that other 2D editors don't

Short list, kept honest. Each entry says what the difference is, why the other
tools decided otherwise, and where to try it here. If you arrived from Spine,
Spriter or DragonBones, these are the assumptions worth dropping.

Everything below is shipped and testable today. Things we intend to build live in
[TASKS.md](TASKS.md) under Phase 9, not here.

---

## IK chains of any length

**Elsewhere:** two bones, and the cap is deliberate. Spine documents it plainly —
"constraining three or more bones is not supported because it is nondeterministic
and would be difficult to control". The workaround is chaining several two-bone
constraints together by hand, which does not behave like one chain.

**Here:** any length. A tentacle, a tail, a spine that reaches, a rope — one
constraint over as many bones as you like.

The objection is real, not an excuse: three or more bones have infinitely many
solutions for a given target, so a solver has to be told which one you want.
Spine answers by refusing. We answer with FABRIK for the solve and
**`bend_direction`** for the ambiguity — it picks the side the chain converges
from, and it is the control that makes a long chain predictable instead of
floppy. It is the "Flip bend" checkbox in the IK inspector.

**Try it:** open `samples/tentacle.ankh` and drag `tentacle-target`. Eight bones,
one constraint. Play the `curl` animation — it keys only the target.

**Build one:** select the bones of the chain in the Hierarchy (shift-click for a
run), then **Create IK target** in the inspector. The button tells you the chain
length it is about to build. One selected bone gives an aim constraint instead,
which is the other thing people want this for.

Relevant code: `core/src/constraints.rs` (`solve_fabrik`, `IkConstraint::chain`),
tests in `core/src/pose.rs`.

---

## Weight painting with an actual brush

**Elsewhere:** sliders. In Spine you select vertices and drag a value bar per
bone. A brush has been requested there since 2016 and repeatedly since; it is
still sliders.

**Here:** paint. Pick a bone, drag over the mesh, weights fall off with the
brush's feather. The controls live in the Weights panel:

| Control | What it does |
|---|---|
| **Direct / Add / Subtract / Replace / Smooth** | What a stroke does to what is already there. Smooth averages against neighbours, which is how you kill a crease. |
| **Weight** | The value a stroke paints toward. |
| **Size / Feather** | Brush radius, and how much of it is soft edge. |
| **Lock** (padlock per bone) | Hold a bone's weights steady while painting others, so fixing one influence does not quietly rob another. |

Sliders still work — select vertices in mesh edit mode and set a value — for when
you want an exact number rather than a gesture.

**Colour coding.** Every bone bound to a mesh gets a fixed colour by its rank on
that mesh: first is always sky, second always pink, and so on. The same colour is
used in the bones list, in the vertex pies, in the paint overlay, and on the bone
gizmo itself while the weight tool is up. If something is pink on the mesh, the
pink row in the list is the bone holding it.

Relevant code: `editor/src/commands/weight_cmds.rs`,
`editor/src/ui/canvas/tools/weight_paint.rs`.

---

## Naming what is under the cursor

**Elsewhere:** click it and look at a panel.

**Here:** hover it. The viewport draws a breadcrumb trail — bone ancestry, then
slot, then attachment or vertex — with the hierarchy's own icons, so a bone, a
slot and a mesh that all share the name `front-foot` are told apart at a glance.

Hovering a bone also lists any constraint driving it, which is the answer to the
most confusing state in rigging: a bone that ignores you when you drag it.
Hovering a mesh vertex lists every bone weighting it, with percentages, in the
rank colours above.

Off in Settings → Grid → Viewport if you would rather not have it; **Alt** summons
one on demand either way.

Relevant code: `editor/src/ui/canvas/hover_label.rs`.

---

## Licensing

MIT or Apache-2.0, per seat cost of nothing, and the runtime is embeddable
without a per-title licence. This is not a feature so much as the absence of a
recurring negotiation, but it is the difference people mention first.

---

## What we do *not* claim

Being straight about this is worth more than a longer list:

* **No export or runtime yet.** Phase 6 is unstarted — no image sequence, no
  video, no runtime crate. Spine's twenty official runtimes are its real moat and
  we have none of it. Today this is an editor, not a pipeline.
* **Fewer years of polish.** No onion skinning, no autosave, no crash recovery,
  and the curve editor wants a pass. Spine has had a decade to sand these down.
* **Smaller ecosystem.** No third-party tutorials, no asset marketplace, no
  StackOverflow answers. You are reading the documentation that exists.
