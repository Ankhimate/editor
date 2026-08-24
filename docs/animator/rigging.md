---
title: Setup and rigging
description: A complete guide to building bones, slots, rigid attachments, hierarchy, transforms, and setup draw order.
---

# Setup and rigging

Rigging defines how a character is built before animation is applied. A basic
Ankhimate rig has four layers:

1. **Assets** are image bytes in the project library.
2. **Bones** form the transform hierarchy.
3. **Slots** hang from bones and define presentation and draw order.
4. **Attachments** are the actual regions, meshes, masks, paths, boxes, or points
   found through a slot and the active skin.

Changing one layer does not automatically change the others. Importing an asset
does not necessarily attach it. Moving a region inside a slot does not move its
bone. Reordering a slot does not change the bone hierarchy.

## Setup mode is the rigging boundary

Use **Setup** for structure and the setup pose. Use **Animate** for keyed pose
changes. Switch with the segmented control in the context ribbon or `Tab`.

| Action | Setup | Animate |
|---|---|---|
| Move, rotate, scale, or shear a bone | Changes its setup transform | Changes the posed value; auto-key writes keys, or auto-key off leaves a pending preview |
| Change slot color | Changes setup tint | Writes/previews a color key |
| Reorder slots | Changes setup draw order | Writes a draw-order key at the playhead |
| Change the slot attachment | Changes setup attachment | Writes/previews a stepped attachment key |
| Create, delete, rename, or reparent bones/slots | Allowed and undoable | Refused with a “switch to Setup” message |
| Import assets or change attachment geometry | Allowed and undoable | Refused |
| Create Bone and Weight Paint tools | Available | Disabled |

Setup always displays the unanimated setup pose, even when the playhead is not at
zero. The viewport says **SETUP POSE**. Animate requires an active clip; when none
exists, entering Animate creates or selects one rather than accepting edits that
have nowhere to go.

If an edit appears to vanish when switching modes, first decide which result you
intended: a permanent change to the rig or a pose in one animation.

## What selection means

The focused item can be a bone, slot, attachment, or constraint. The Inspector's
breadcrumb shows the exact path and each breadcrumb segment is clickable.

### Bone selection

- Plain-click a bone in the Hierarchy to replace the bone selection.
- `Ctrl`-click toggles one bone.
- `Shift`-click extends from the active bone through the visible hierarchy order.
- Canvas multi-selection uses the platform multi-select modifier. Box selection
  and multi-bone transforms operate on the selection as a group.
- Selecting a slot or attachment focuses that item and its owning bone. Bone and
  slot selections are not two independent sets.

The **last active bone** is significant for commands such as attaching an asset
or parenting a right-clicked bone. Check the context ribbon when the intended
target is unclear.

Selection, hierarchy expansion, filter text, locks, temporary visibility,
isolation, active tool, and camera are session state. They are not saved in the
`.ankh` file and are not undoable.

### Filtering, expansion, and isolation

The Hierarchy filter matches bone, slot, attachment, and constraint names. A
matching descendant keeps its ancestors visible and opens the branch, so a match
cannot remain hidden inside a collapsed limb. Clearing the filter restores the
ordinary expansion state.

Right-click a bone and choose **Isolate this limb**, or use `Shift+H`, to show the
bone and its descendants alone. **Exit isolation** restores the full rig. This is
a viewport aid; it does not alter slot visibility or exported content.

### Locks and visibility eyes

The bone-row padlock prevents viewport drags and auto-key changes to that bone.
It does not remove the bone, freeze evaluation, or become project data. Hierarchy
eye controls temporarily hide bones or slots for editing. A hidden slot here is
different from an animated **Visible** key: the eye is session-only, while the
key is part of the animation.

## Creating a skeleton

Select **Create bone** on the tool rail or press `B`. The tool is Setup-only.

1. Drag from the desired joint to the desired tip. Drag direction establishes
   the local +X axis; drag distance establishes length.
2. Start from an existing bone to create a child. Otherwise a root is created.
3. Continue outward in anatomical order: torso to upper arm to forearm to hand,
   for example.
4. Return to Select with `V` when the chain is complete.

A bone's length is both its displayed segment and the distance to its tip. In the
Inspector, changing **Length** carries children sitting at that tip so a resized
limb remains connected. Hold `Alt` while changing length to leave children where
they are—for example when a child deliberately starts halfway along a bone.

Give bones descriptive, unique names. Names are the stable references saved to
disk and used by animation tracks, constraints, plugins, and exporters.

## Understanding bone transforms

Each bone stores one local setup transform: translation, rotation, scale, and
two shear axes. Its world transform is computed through its parents. Animation
stores offsets from setup rather than replacing the setup transform.

| Inspector row | Meaning |
|---|---|
| **Rotate** | Counter-clockwise degrees around the joint. `R` activates the rotation gizmo. |
| **Translate X/Y** | Position relative to the chosen edit view. `T` activates translation. |
| **Scale X/Y** | Axis scale; `1` is unchanged, negative reflects, and zero is degenerate. `S` activates scale. |
| **Shear X/Y** | Turns the local axes independently, in degrees. `H` activates shear. |
| **Length** | Bone segment length, separate from scale. Structural and Setup-only. |

Numeric drags and canvas drags merge into one undo step. Angles shown in the UI
are degrees. The renderer and saved file use the same authored orientation; the
screen's downward Y direction is handled by the camera.

### Local, Parent, and World selector

The transform selector changes what the fields display and edit:

- **Local** edits the selected bone's transform relative to its parent. This is
  the stored source of truth.
- **Parent** edits the selected bone's actual parent. It is a convenience for
  adjusting the upstream transform without changing selection. It is unavailable
  for a root bone. It does *not* mean “show this bone expressed in parent space”—
  Local already does that.
- **World** displays the selected bone in world space. Editing a world value
  solves a new local transform that produces it. No world transform is stored.

World solving can become unstable when a parent transform is degenerate, notably
when a parent scale is zero. Repair the parent scale in Local or Parent view.

### Color and inheritance

Bone **Colour** identifies a limb in the hierarchy, viewport, and weight tools;
it never tints artwork. A bone without its own color inherits the nearest colored
ancestor. **Reset** returns to inherited color.

Rotation, scale, and reflection inheritance flags exist in the rig model and
`.ankh` format and affect parent-to-child world composition. **Partial:** the
current Inspector does not expose these flags. Files and integrations can carry
them, but ordinary editor rigging should currently use the default—inherit all
three—or introduce an intermediate bone when isolation is needed.

## Reparenting without a visual jump

Reparenting changes structure and is Setup-only. The command recomputes the
bone's local transform so its current world placement stays fixed.

There are three routes:

- Drag a bone onto the lower half of another bone row to make it a child.
- Drop on the upper half to place it beside that bone under the same parent.
- Right-click and choose **Parent to selected** or **Unparent (to root)**.

You cannot create a cycle. Reparenting preserves the visible setup placement at
the moment of the operation, but changes how future parent movement and existing
animation combine. After reparenting, inspect at least the setup pose and the
extreme frames of every affected clip. Undo restores the old hierarchy and local
transform together.

Dragging a slot onto a bone changes the slot's owner while preserving its world
placement. This moves artwork between transforms; it does not reparent a bone.

## Deleting, copying, and organizing bones

Right-click provides rename, bulk rename, and delete. Deleting a bone is a
cascade operation; review the descendant count because children and related
structure can be affected. It remains undoable.

In Setup, copy/paste and duplicate operate on complete bone subtrees, including
their slots and skin entries. Pasted names are uniquified with suffixes such as
`_2`; skins are matched by name. This avoids copying half a limb whose slots
still reference missing bones.

Groups are folders, not transform parents. Moving a bone or slot into a group
changes only hierarchy organization. A group's color stripe makes related rows
easy to scan; it does not alter bone color or rendering.

## Assets: import first or attach immediately

The Assets panel is the project's embedded image library. It supports PNG, JPEG,
and WebP. Imported bytes are saved inside `.ankh`; `source_path` is only an
advisory link for reload/relink.

### Import without attaching

Use the Assets **Import** action to add one or more images to the library. This is
useful when artwork arrives before the skeleton. No bone, slot, or attachment is
created until you attach the asset later.

The panel lists name, pixel dimensions, source status, and a thumbnail. Double-
click the name to rename. Renaming updates attachment texture references. Delete
reports how many attachments still use the image; deletion is allowed and
undoable, but those attachments stop drawing until repaired.

**Check sources** compares bytes with their original files. **Reload** replaces
pixels from the recorded source; **Relink** chooses a new file and source path.
These update pixels, not rig transforms or animation.

### Drop directly into the viewport

In Setup, drop image files onto the viewport. For every image Ankhimate creates:

- an embedded asset;
- a slot on the selected bone, or on a root when no suitable bone is selected;
- a region attachment in the default skin;
- placement corresponding to the drop position.

If the rig has no bones yet, the files are added to Assets only. If bones exist
but none is selected, the first root is used.

The operation is refused in Animate. Save once after a large import so both the
project JSON and embedded bytes are safely in the container.

### Attach an existing asset

There are three supported workflows:

1. Select a bone, select an Assets row, then click its **Attach** action.
2. Drag an Assets thumbnail onto a bone row in the Hierarchy.
3. Drag an Assets thumbnail onto the viewport directly over the target bone to
   place it there.

Each creates a new slot and region attachment. The asset remains in the library
independently of that attachment. If **Attach** is disabled, either switch to
Setup or select a bone first.

## Slots: transform owner and presentation

A slot hangs from exactly one bone. It does not have its own transform. Its
resolved attachment supplies geometry and local placement; its bone supplies the
world transform.

The Slot section exposes:

| Control | Setup behavior | Animate behavior |
|---|---|---|
| **Color** | Stores setup RGBA tint | Keyable continuous tint; alpha fades |
| **Blend** | Stores Normal, Additive, Multiply, or Screen | Structural presentation; disabled outside Setup |
| **Dark tint** | Enables optional two-color tint | Structural presentation; disabled outside Setup |
| **Visible** | Setup slots are intrinsically visible | Writes a stepped visibility key; hidden means not drawn, unlike alpha zero |
| **Attachment** | Chooses setup attachment lookup | Writes a stepped swap key or clears the slot |

Blend guidance:

- **Normal** is ordinary alpha compositing.
- **Additive** suits emitted light, sparks, and glows.
- **Multiply** darkens what is behind it, useful for shadows or stains.
- **Screen** lightens while retaining more highlight structure than additive.

The Hierarchy eye is a temporary editor aid and is not the same as the Visible
property. Dark tint is optional and is not a substitute for the ordinary Color.

## Region attachments: place the art without moving the rig

A region is a textured rectangle. Select the attachment row—or its owning
slot—and use the Attachment section to edit:

| Control | Meaning |
|---|---|
| **Offset X/Y** | Position inside the slot/bone coordinate system. |
| **Rotate** | Region-local degrees around its pivot. |
| **Scale X/Y** | Region-local scale. |
| **Size X/Y** | Authored rectangle width and height. |
| **Pivot X/Y** | Normalized image position: `(0,0)` bottom-left, `(1,1)` top-right. |

The 3×3 pivot grid snaps to corners, edge centers, or center. Changing the pivot
compensates offset so the image stays visually fixed; subsequent rotation and
scale use the new pivot. **Reset size** restores imported pixel dimensions.

Enable **Edit on canvas** to make transform gizmos drive the attachment rather
than the bone. The viewport outlines the quad and shows its pivot crosshair.
Translate, rotate, and scale affect only the art. `Alt`-drag the pivot to move it
while keeping the art fixed. Attachment canvas editing is Setup-only and explicit
so an accidental slot click cannot silently retarget a bone drag.

**Duplicate** creates another attachment name in the same slot, suitable for a
swap set. **Remove** deletes it from the skin it resolved from. **To mesh** keeps
the quad's placement while converting it to deformable geometry; mesh editing is
covered separately in [Meshes and deformation](/animator/deformation/).

Edits apply to the skin that supplied the visible attachment—active skin first,
then default—not to a newly created override. Rename also updates the slot setup
name and attachment keys that referenced the old name.

## Empty slots and non-region attachments

An empty slot draws nothing and can be intentional. Its Attachment section can
create these Setup-only types:

- **Clipping mask:** polygon masking following slots through an inclusive end
  slot, or the remainder of draw order.
- **Path:** non-rendered curve for a path constraint.
- **Bounding box:** non-rendered polygon for hit tests, hurtboxes, and triggers;
  it can be rigid or weighted.
- **Point:** non-rendered position and heading for a muzzle, effect origin, grip,
  or other runtime anchor.

Meshes and polygon editing have their own workflow. This chapter only establishes
where these attachments live: under a `(skin, slot, attachment name)` lookup.

## Setup draw order

Slots render back-to-front. The dedicated Draw Order panel deliberately displays
the inverse—**front-most at the top**—because it behaves like a visual layer
stack. Use its up/down controls to move a slot toward the front or back.

The Hierarchy also supports dragging one slot relative to another. Dropping in
the upper or lower half chooses before or after in setup order. Dropping the slot
on a bone changes its owner instead, so watch the highlighted target.

In Setup, reorder changes the baseline stack. In Animate, the same action writes
a draw-order key at the current playhead and labels the panel “animating.” Switch
back to Setup to verify the baseline was not changed.

## A reliable first-rig sequence

1. Enter Setup and import artwork into Assets.
2. Draw roots and chains from body center outward.
3. Name bones before adding many references.
4. Attach each image to the bone that should carry it.
5. Use **Edit on canvas** to place regions without moving bones.
6. Set pivots at the joint that should stay fixed.
7. Arrange slots in setup draw order, far pieces behind near pieces.
8. Color bone groups for readable hierarchy and weight overlays.
9. Save, switch to Animate, and test a few extreme poses.
10. Return to Setup for any structural correction.

## Symptom checklist

| Symptom | Likely cause and correction |
|---|---|
| Art rotates around its middle | Move the region pivot to the joint; placement is compensated automatically. |
| Moving art bends or displaces the whole limb | The edit target is Bone. Select the slot/attachment and enable **Edit on canvas**. |
| Attach is disabled | Enter Setup and select a bone. |
| Imported image exists but nothing draws | It may only be an Asset. Attach it, then check slot attachment name, active skin, asset bytes, and temporary eye state. |
| A slot is behind the wrong piece | Correct setup Draw Order; if it changes only at some frames, inspect draw-order keys. |
| Reparented limb looks right in Setup but wrong in motion | World placement was preserved, but animation now composes through a different parent. Review affected clips. |
| Bone cannot be dragged | Unlock its hierarchy padlock; also ensure Select is active and the intended transform gizmo is chosen. |
| Setup edit appears animated | You are in Animate, or viewing a pending auto-key-off preview. Switch to Setup before changing rig defaults. |
| Slot is invisible although alpha is nonzero | Check animated Visible keys and the session-only Hierarchy eye separately. |
| Parent/world numeric edit behaves unexpectedly | Parent edits the parent bone itself; World solves back to local. Return to Local to inspect stored values. |

Next: [Meshes and deformation](/animator/deformation/) for topology, UVs,
binding, and weights, or [Animation](/animator/animation/) to key the completed
setup rig.
