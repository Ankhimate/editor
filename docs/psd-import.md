# PSD import

A layered PSD is already a rig nobody has told the computer about. The group
structure is the hierarchy, the layer bounds are the placement, and the stacking
order is the draw order. Ankhimate reads that structure out rather than asking
you to rebuild it by hand.

Everything below is driven by **layer names**. A name is the one thing every art
tool round-trips, and the one thing an artist can fix without leaving Photoshop.

## The mapping

| In the PSD | Becomes |
|---|---|
| layer group | a bone; nested groups nest |
| image layer | a slot plus a region attachment, sized and placed from the layer bounds |
| layer named `$pivot` | its group's bone origin, instead of the group's bounding-box centre |
| group named `$ik <name>` | a bone, plus an IK constraint named `<name>` over the chain inside it |
| top-level group `@skin:<name>` | its layers land in skin `<name>` rather than the default skin |

Draw order follows the PSD's own layer order, so what you see in Photoshop is
what you see in the viewport.

### `$pivot`

Without it, a group's bone sits at the centre of the group's bounds — fine for a
head, wrong for a shoulder. Add a small layer named `$pivot` inside the group and
put it where the joint is; its centre becomes the bone origin. The layer itself
is not imported.

### `$ik `

A group named `$ik leg` becomes a bone called `leg` and an IK constraint called
`leg` over the bones beneath it, with a target bone created at the chain's tip —
the handle you actually drag.

The chain is the **single-file** run of descendants. IK solves a line of bones,
so if a group branches, the first branch is taken and the rest is left to you.
Guessing which fork you meant would be worse than stopping.

A group with fewer than two bones under it is reported as skipped rather than
producing a constraint that cannot solve.

### `@skin:`

A top-level group named `@skin:winter` puts everything inside it into a skin
called `winter`. Its layers hang from the *parent* group's bone, so alternate art
sits where the base art sits instead of under a bone that only exists in one
outfit.

## Options

| Option | What it does |
|---|---|
| Scale | World units per PSD pixel. A 2048px character at `0.5` becomes 1024 units tall. |
| Skip hidden layers | On by default. A hidden layer is usually a reference sketch or an alternate the artist did not delete. |
| Flatten group | Composites a group into one attachment instead of a bone with children. A face that never articulates does not need eleven bones. |
| Replace the open project | Off merges the imported rig under a bone of its own, keeping existing bones and animations. |

Per-layer tick boxes control what is imported. Ticking a group ticks everything
inside it.

## Coordinates

PSD measures from the top-left with Y down. Ankhimate measures from the canvas
centre with Y up (PLAN §2.2). The conversion happens in one place, `layer_center`
in `formats/src/psd.rs`, and is covered by unit tests — a second copy of it would
be a second chance to disagree about which way is up.

Layer bounds in the file are **inclusive** on the right and bottom edges, so the
width is `right - left + 1`. Off by one there makes every attachment a pixel
short and every pivot half a pixel out.

## Re-import

Each imported asset remembers the layer path it came from (`torso/arm/hand`).
Re-importing the same PSD diffs against those paths and reports what was added,
removed, and unchanged.

Path is the identity, not name alone: a layer that moves inside its group keeps
its path and so keeps its bone. A layer that is **renamed** reads as a delete plus
an add — we cannot tell a rename from a swap, and pretending otherwise would
silently retarget somebody's animation.

The diff reports rather than acts. Deleting a slot would take its animation keys
with it, so what to do about a removed layer is your call.

## Known limits

- **Negative layer bounds.** Art dragged off the canvas edge makes `psd 0.3.5`
  index out of bounds and panic. The call is caught and the layer is reported as
  skipped, so the import survives, but that layer does not come in. Crop or move
  the art inside the canvas to import it.
- **Layer effects, adjustment layers, masks and blend modes** are not read. What
  you get is the layer's own pixels.
- **Rotation.** A PSD has none to read, so every bone comes in unrotated.
- There is **no checked-in fixture PSD** yet, so the acceptance test for T-302 is
  still outstanding. The mapping functions are unit-tested; the end-to-end path
  has been validated by hand against grouped, nested, offset and negative-bounds
  PSDs.
