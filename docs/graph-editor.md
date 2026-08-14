# Graph editor interaction rules (T-912)

Four rules and one deferral, each written down because it answers a complaint
filed repeatedly against the established editors' curve editors — unexpected
snapping, curves that cannot be seen, tangents that overshoot without saying so,
and handles that cannot be grabbed back. They are easy to undo by accident in a
later change, so the reasoning lives here rather than only in the diff.

The source of the complaints is a five-year cluster of independent forum threads
against Spine's graph editor. That evidence is **title-level**: the forum is
JS-rendered and resisted extraction, so treat the specific claims as unconfirmed.
What is confirmed is the shape — independent users, several years, one feature,
no resolution. The rules below are written against that shape.

---

## 1. A selected curve is never invisible

**Rule.** The value range is fitted to the *sampled curve*, not to the key
values, at the same 2px step the curve is drawn at.

**Why.** A bezier with steep enough handles travels past the values it
interpolates between. Framing on keys alone drew that overshoot outside the
panel: the curve left the top of the view and nothing said it had. Measuring
what is about to be stroked is the only framing that cannot lie.

**What would break it.** Reverting to `channels.values` for the range because it
is cheaper. It is cheaper, and it is wrong for exactly the curves a graph editor
exists to edit.

## 2. Snapping is opt-in and its state is always visible

**Rule.** Dragging a key snaps to whole frames; **Alt** drags freely. A badge in
the top-left corner reads `snap: frames` or `free`, every frame, whether or not
anything is being dragged.

**Why.** Silent snapping is the specific complaint. A key nudged a third of a
frame springs back to where it was and the editor never says why — which reads
as the drag not working rather than as a mode doing its job.

**What would break it.** Hiding the badge when nothing is selected, or when
snapping is on "because that is the default". A mode is only reasonable-about if
it is legible *before* it surprises you.

## 3. An overshooting tangent is visually distinct from a well-behaved one

**Rule.** When a curve travels more than 10% of its keyed range past its keys,
the panel says `overshoot N%` in the warning colour.

**Why.** Once rule 1 re-frames the view to contain an overshoot, the overshoot
stops being visible *as* an overshoot — it just looks like a curve. But the
difference between a bounce someone authored and a handle dragged too far is the
whole question, and the shape alone no longer answers it.

**Not clamped, deliberately.** An overshoot is a legitimate thing to author; it
is how a bounce is made. Silently flattening one would be worse than the
invisibility this replaced. The editor reports; the animator decides.

**The threshold.** 10% of the keyed range. Below that, an ease that peeks past
its key is ordinary and flagging it would be noise — a warning that fires on
everything is a warning nobody reads.

## 4. A handle is always grabbable

**Rule.** A handle whose true position falls outside the panel is drawn pinned to
the edge, as a hollow ring rather than a filled dot, and stays draggable. Pinning
moves the marker; the stored value is never touched.

**Why.** Rule 1 frames the *sampled curve*, and a cubic does not pass through its
control points — so a handle shaping a legitimate overshoot routinely sits
outside the view. The grab box is gated on the panel rect, so a marker drawn out
there is unreachable: the handle could not be dragged back and the only way out
would be undo. Grabbing a pinned handle and dragging snaps it to the pointer,
which is the recovery gesture.

**What would break it.** Widening the auto-fit to include handle positions
instead. It sounds like the same fix and is worse twice over: the overshoot
percentage in rule 3 is computed from that same range, so the badge would report
a number that is not overshoot, and one handle at four times the span zooms the
view to mostly dead space, flattening every curve in the panel.

## 5. Tangent values can be typed

**Status: not implemented.** Handles are draggable only.

Doing it properly needs the graph's key selection — which currently lives in
egui memory local to the dopesheet — plumbed somewhere the inspector can read,
so the numbers appear beside the other numeric fields rather than in a popup of
their own. That is a bigger change than the three rules above and is better done
with the rest of the inspector's key editing than bolted to the graph.

It pairs with T-902 (numeric entry for every vertex and handle), which covers
mesh, UV, path and bounding-box points but not curve tangents.
