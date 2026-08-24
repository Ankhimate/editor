---
title: Meshes and deformation
description: Complete workflow for topology, tracing, UVs, binding, weight painting, cleanup, and deform keys.
---

# Meshes and deformation

A region is one rigid rectangle. Convert it to a mesh when the artwork must bend,
curve, crease, or use a non-rectangular silhouette. A mesh separates four things:

- **Setup vertices** define the authored shape in slot-local space.
- **Triangles and pinned edges** define how that surface is connected.
- **UVs** define where every vertex samples the image.
- **Weights and bind data** define which bones move every vertex.

Animation deform keys add offsets to the setup shape before skinning. Editing
topology, editing weights, and animating deformation are related workflows, but
they write different data.

## Mode and tool boundaries

| Operation | Setup | Animate |
|---|---|---|
| Convert region to mesh | Allowed | Refused |
| Move mesh vertices with **Edit vertices** | Changes setup vertices | Writes/replaces a deform key at the playhead |
| Add/delete vertices, pin edges, retriangulate, trace, reset/edit UVs | Allowed | Structural commands are refused |
| Weight Paint tool (`W`), Bind, Auto, Smooth, Weld, Prune, Update | Allowed | Weight Paint tool is disabled |
| Move the whole attachment | Use attachment **Edit on canvas** | Attachment setup transform cannot be animated this way |

The same vertex drag intentionally has two meanings. Check the mode chip before
dragging: Setup repairs the base mesh shared by every clip; Animate changes only
the current clip's shape.

## Convert a region without changing its appearance

1. Enter Setup.
2. Select a slot whose resolved attachment is a region.
3. In Attachment, click **To mesh**.

Conversion creates four setup vertices at the region's exact corners, matching
UVs, and two triangles. The result should look identical until a vertex moves.
The attachment keeps its name, texture, slot, and skin ownership. Mesh placement
remains active after conversion; vertex editing is a separate explicit toggle.

Conversion is undoable. If the result jumps immediately, treat that as a bug or
a pre-existing invalid region transform rather than compensating the vertices.

## Mesh section and vertex-edit mode

The Inspector's Mesh section reports vertex count, triangle count, and pinned
edge count. Enable **Edit vertices** to draw the wireframe and handles.

| Gesture | Result |
|---|---|
| Click a vertex | Select it and begin a drag. |
| Drag a selected vertex | Move every selected vertex by the same delta. One drag is one undo step. |
| `Shift`-click | Toggle one vertex without replacing the rest. |
| `Ctrl`-drag | Box-select from anywhere, even over a dense vertex field. Replaces selection. |
| `Ctrl+Shift`-drag | Add the box contents to the selection. |
| Click empty space | Clear the vertex selection. |
| Click near an edge | Insert a vertex at the nearest point on that edge and select it. Setup-only. |
| `X` or `Delete` | Delete selected vertices. Setup-only; a mesh cannot go below three vertices. |
| `C` with exactly two selected | Pin or release the edge between them. Setup-only. |
| `Escape` | Leave mesh-edit mode and clear selected vertices. |

Handle hit areas and edge insertion distance are screen-sized, so zooming does
not make them impossible to grab. Vertex handles follow the drawn, skinned, and
deformed positions; in Animate you grab where a point currently appears, not
where its setup position used to be.

### Numeric vertex and UV editing

With vertices selected, the Inspector exposes their coordinates. When all
selected vertices share a value, the field shows it; otherwise it shows a mixed
state. Editing a field assigns that coordinate to the whole selection. For one
selected vertex, U and V fields are clamped to `0..1` and edit its texture sample.

For one selected weighted vertex, **Influences · vertex N** lists bone names and
weights. Each value accepts `0..1`; delete removes that influence. **Total** is
highlighted when it differs from 1 by more than 0.001, and **Normalize** rescales
the remaining influences to total 1. An empty list means this vertex rides the
slot bone rigidly.

## Triangulation and pinned edges

Triangles are the surface that actually renders. Ankhimate uses deterministic
constrained Delaunay triangulation: it generally avoids thin triangles, but a
plain vertex list does not say which points form an outer contour or a hole.

Click **Retriangulate** after manual changes when the current connections are no
longer useful. It rebuilds triangles while retaining pinned edges. Pin an edge
with `C` or **Pin/release edge** when automatic triangulation bridges a notch,
crosses a seam, or chooses the wrong diagonal. Exactly two vertices must be
selected. Pressing the action again releases the pin.

A pin constrains connectivity; it does not weld vertices or stop them moving.
Crossing/invalid constraints or degenerate points may prevent a useful result.
Keep at least three non-collinear vertices and inspect the wireframe after every
large topology edit.

## Trace a silhouette from artwork

**Trace from image…** opens a live preview. It replaces the current mesh rather
than adding points to it.

| Control | Range | Effect |
|---|---:|---|
| **Detail** | `0..100` | Number of outline vertices. Higher follows the silhouette more closely. |
| **Concavity** | `0..100` | Preserves notches and gaps that ordinary simplification tends to flatten. |
| **Refinement** | `0..100` | Effort spent sliding points to better outline positions; does not itself increase count. |
| **Uniform** | `0..1` | Subdivides long edges for more even spacing and predictable bending. |
| **Interior** | `0..100` | Density of interior points added by Refine so the middle can bend. |
| **Alpha threshold** | `1..255` | Pixels at or above this alpha count as solid. |
| **Padding** | `0..20` | Pushes the outline outward to avoid clipping antialiased edge pixels. |

**Trace** recuts the outline and removes refined interior points. **Refine** adds
interior points using current settings. The preview reports total, outline,
interior, and contour counts. **Cancel** changes nothing. **OK** replaces setup
vertices, UVs, and triangles as one undoable operation.

Applying a trace clears all weights because old weights referenced old vertex
indices; the status message says so. Existing deform tracks likewise describe
the old topology and should be removed or rebuilt. If no triangles result, lower
the alpha threshold, reduce detail on very large art, or verify image bytes.

Practical guidance:

- Put more vertices where curvature changes, not uniformly everywhere.
- Add interior points across elbows, knees, cloth folds, and soft torsos.
- Avoid hundreds of nearly redundant outline points; they increase weighting
  work without producing visible flexibility.
- Use padding large enough to keep filtered edge pixels, but not enough to pull
  transparent background into the mesh.

## UV editor

Click **UV editor…** in the Mesh section. The pane shows the texture with mesh
points and edges in UV space.

- Drag a UV point to change where that vertex samples the texture.
- Scroll to zoom from 10% to 4000%.
- Middle-drag, or drag empty space, to pan.
- The Fit button returns to the whole texture centered at 100% fit.
- **Reset UVs** reprojects every UV from the mesh's current axis-aligned bounds
  and discards hand edits.

UV dragging merges into one undo step. UV movement changes texture mapping only;
it does not move the mesh in the viewport. Conversely, moving setup vertices does
not automatically rewrite carefully authored UVs.

Use Reset UVs when you want the entire texture stretched over the current mesh.
Do not use it after intentionally mapping a seam, atlas subregion, or unusual
texture layout unless you are prepared to rebuild that mapping.

## Rigid and weighted vertices

A mesh with no weights is rigid: every vertex follows the slot's bone. Once
weights exist, each vertex is blended from its own influence list. A vertex with
no usable influences still falls back to the slot bone rather than collapsing to
the origin.

For a weighted vertex, each bone transforms the setup point from its recorded
bind relationship and the weighted results are combined. Weights should normally
sum to 1. Too many tiny influences produce soft, expensive, hard-to-debug motion;
Prune exists to control that.

## Enter Weight Paint

Select a slot containing a mesh, enter Setup, and press `W` or choose **Weight
paint**. Click a bone in the canvas or bound-bone list to choose the influence.
Then drag over the mesh.

The brush measures distance to the vertices where they are currently drawn, so
it remains aligned on a posed rig. A stroke is undoable as one step. When weights
change, stale inverse binds are cleared and recaptured from the setup pose so
adding an influence does not make the mesh jump.

### Input modes

| Mode | Behavior |
|---|---|
| **Add** | Raises the active bone toward **Weight**, never beyond it. |
| **Subtract** | Lowers the active bone toward zero; **Weight** acts as the removal rate. |
| **Replace** | Drives the active bone toward exactly **Weight** from either side. |
| **Smooth** | Averages the active bone with neighboring vertices. |
| **Direct** | No brush. Changing **Weight** writes that exact active-bone value to vertices selected in mesh-edit mode. |

In brush modes, **Weight** is a destination, not an amount added per dab. At the
center of a full-strength brush, Add/Replace can reach 1 exactly. Other unlocked
influences give up the remaining share proportionally.

| Control | Range | Meaning |
|---|---:|---|
| **Weight** | `0..1` | Target for Add/Replace/Direct; removal rate for Subtract. |
| **Size** | `4..400` | Brush radius in world-space distance. Disabled in Direct. |
| **Feather** | `0..1` | Fraction of radius used for falloff. `0` is hard; `1` fades from center to edge. Disabled in Direct. |

Direct needs one active bone and at least one vertex selected. Brush modes need
an active bone, active mesh slot, and a drag on the canvas.

### Display aids

- **Overlay** shades the mesh by the active bone's influence.
- **Pies** show every vertex's complete influence split in per-mesh bone colors.
- **Selected** restricts vertex markers to the mesh-edit selection.
- A ring and warning count identify vertices above the configured influence
  limit; the limit is advisory until Prune is used.

Bone colors in the weight list are ranked for this mesh and need not match the
hierarchy limb color. The percentage at the right of a row is that bone's mean
influence across the mesh; a small unexpected share is a useful clue to stray
weights.

## Binding and bound-bone controls

The **Bones** list contains every bone used by the current mesh, strongest first.
Click a row to make it active. Hover a row to expose its lock.

### Bind

Select one or more bones and click **Bind**. It adds those bones and computes
starting weights by distance from vertices to bone segments. The same surface-
aware automatic weighting used by **Auto** supplies the initial distribution.
Binding is a starting point; test joint extremes and refine it.

### Lock

Locking a bone reserves its current share while painting other bones. If the
painted bone itself is locked, the stroke is a no-op. Locked totals are taken off
the available 1.0 before other influences are redistributed. Locks are session
state, not saved mesh data.

### Swap and Remove

**Swap** requires exactly two selected bones and exchanges their weights across
the whole mesh. It is useful after weighting the wrong side of a symmetric rig.

**Remove** deletes the active bone from every vertex and normalizes what remains.
If nothing remains, those vertices fall back rigidly to the slot bone.

### Copy and paste

Copy stores the current mesh's full weight table in a session clipboard. Paste is
enabled only for a target mesh with the same vertex count. It copies by vertex
index, not by position or topology, so equal count alone does not guarantee that
the correspondence is artistically correct.

## Automatic weighting and cleanup

### Auto

**Auto** computes inverse-distance weights across the connected mesh surface,
not straight through empty space. That prevents nearby but disconnected fingers
or legs from stealing each other's influence.

Selection scopes the operation:

- no selected bones or vertices: every bone and vertex;
- selected bones only: those bones over every vertex;
- selected vertices only: every bone over those vertices;
- both: selected bones over selected vertices.

Auto weights bones as line segments, not only their origins, and applies a fixed
distance falloff. It is deterministic but not semantic: it does not know which
side of a knee should crease or whether nearby cloth should follow a hand.

### Smooth

**Smooth** averages the active bone's weight with neighboring vertices. It acts
on selected vertices, or the whole mesh when none are selected. Select a bone
first. Locks are respected. Repeated smoothing broadens transitions and can erase
a deliberate hard fold.

### Weld

**Weld** copies weights between coincident seams on multiple selected mesh slots.
The **last selected slot is the source** and remains untouched; all other selected
meshes are targets. Positions are compared in world space with one world-unit of
slack, so meshes may live under different bones. Only nearby corresponding
vertices change. Use this for clothing/body seams or separately traced pieces.

### Prune and influence budget

The first numeric cleanup control is **maximum bones**, `1..8`. It both marks
over-budget vertices and limits what Prune keeps. The second is **threshold**,
`0..0.2`; Prune drops weights below it, retains the strongest up to the maximum,
then normalizes. It affects selected vertices, or all when none are selected.

Prune can remove an influence you meant to preserve. Locking affects painting,
not Prune, so inspect the result and undo if the threshold was too aggressive.

### Update bindings

**Update** records current setup vertices against the currently weighted bones as
the new bind relationship. Use it after moving setup bones or setup vertices on
an already weighted mesh when the artwork drifts away from the intended rest
shape. Do not press it merely to “refresh” a correctly behaving mesh: redefining
the bind pose changes the relationship every animation relies on.

## Deform animation

Enable **Edit vertices** on a mesh, switch to Animate, choose a playhead time, and
drag vertices. Ankhimate writes or replaces one Deform key for the `(slot,
attachment)` at that time. The key stores the entire shape's offsets so moving a
second vertex does not reset the first. Repeated drag updates at the same time
merge into one undo step.

Offsets are local changes from setup vertices. Evaluation order is:

1. start with setup geometry;
2. sample/interpolate the deform offsets;
3. add offsets in mesh-local space;
4. apply weighted or rigid skinning;
5. transform and render triangles.

This makes a fold rotate naturally with its bones. Deform keys support linear,
stepped, and Bézier interpolation and mix by animation alpha. Topology edits can
invalidate every stored offset index, so finish topology before detailed deform
animation. Tracing explicitly clears weights but does not automatically repair
old deform timelines.

**Partial — weighted per-influence authoring:** schema v3 and rendering can store
and evaluate a separate deform offset for each influence of a weighted vertex,
which is needed for an independently controlled crease. The current canvas drag
builds one offset per visible vertex and cannot author those influence copies
independently. On vertices with multiple influences, do not rely on the canvas
workflow for exact per-influence deformation; use bone weights for the primary
crease and treat imported/integration-authored per-influence deform as advanced
data until the editor exposes it correctly.

## Linked meshes

A linked mesh can borrow geometry from a source skin/slot/attachment while using
its own texture and optionally inheriting source deformation. Runtime resolution
and `.ankh` round-tripping are implemented.

**Partial:** the current editor has no complete UI for creating or changing the
link target or `inherit_deform`. Do not plan an animator-only linked-mesh workflow
yet. Ordinary region duplication creates independent attachments, not linked
geometry.

## Weighted bounding boxes

A bounding box uses the same polygon editor and can carry weights. With no
weights it follows its slot bone rigidly; with weights it deforms with the rig.
Select **Edit polygon** to move, add, box-select, or delete points using the mesh
gestures. It has no UVs or rendered texture. Use it for hurtboxes and trigger
areas that must follow a bending limb.

## Failure diagnosis

| Visible symptom | Likely cause | Correction |
|---|---|---|
| Texture swims while geometry looks correct | UVs do not correspond to moved vertices | Open UV editor; move individual UVs or deliberately Reset UVs. |
| Transparent wedges or a notch is filled | Automatic triangulation chose an unwanted bridge | Select boundary vertices and pin the intended edge; retriangulate. |
| Triangle is extremely thin or flickers | Duplicate/nearly collinear vertices or poor topology | Move/delete redundant vertices, add useful interior spacing, retriangulate. |
| Mesh jumps when first weighted | Stale/invalid bind relationship or degenerate bone transform | Undo, repair zero scale, bind again; use Update only for an intentional new rest pose. |
| Unpainted part collapses toward origin | Missing usable influences or old invalid bind data | Inspect per-vertex influences; normalize/rebind. Current fallback should otherwise keep it rigid. |
| Joint bends like rubber | Weight transition is too broad or too many bones contribute | Inspect Pies, reduce stray weights, sharpen with Replace/Subtract, then Prune. |
| Joint tears or forms a hard crack | Adjacent vertices have abrupt unrelated weights or seam meshes disagree | Smooth locally; Weld coincident seams; check pinned connectivity. |
| Painting one bone changes another unexpectedly | Normalization redistributes unlocked shares | Lock the influence that must stay fixed, then paint. |
| Brush does nothing | No active bone/mesh, Direct mode is selected, active bone is locked, or editor is not in Setup | Check the hint line, mode, tool, slot, bone, and lock. |
| Direct slider does nothing | No selected vertices or no active bone | Select vertices in Edit vertices, select a bone, then change Weight. |
| Auto weights the wrong limb | Selection scope included unrelated bones or topology connects nearby parts | Undo; select only intended bones/vertices and rerun Auto, then refine manually. |
| Prune removed an important influence | Threshold/maximum was too strict | Undo, lower threshold or raise maximum, and prune a smaller selection. |
| Deform shifts unexpected weighted copies | Canvas-authored vertex offsets do not express the v3 per-influence layout | Prefer weight/bone correction; avoid precision per-influence deform until UI support is complete. |
| Old deform breaks after tracing | Vertex count/order changed | Remove and recreate deform tracks after topology is final. |

## Recommended workflow

1. Place and pivot the region correctly before conversion.
2. Convert to mesh and establish topology in Setup.
3. Trace or edit the silhouette; add interior bending points.
4. Inspect/pin edges and retriangulate.
5. Finish UVs before weighting.
6. Select intended bones and Auto/Bind.
7. Pose the rig at joint extremes while painting and inspecting Pies.
8. Lock finished influences, smooth transitions, then prune to the runtime budget.
9. Update bindings only if you intentionally changed the rest relationship.
10. Finish topology before authoring Animate-mode deform keys.

Next: [Animation](/animator/animation/) for deform tracks, interpolation, and
timeline editing, or return to [Setup and rigging](/animator/rigging/).
