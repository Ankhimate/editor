---
title: Animation and graph editing
description: Create clips, key exact properties, edit timing and curves, author events, and diagnose unexpected motion.
---

# Animation and graph editing

An animation clip records changes from the rig's Setup pose. It does not contain a
second skeleton. A bone translation key is an offset from its Setup position, a
rotation or shear key is an angular offset, and a scale key is a multiplier. Slot
color, attachment, visibility, draw order, deform, and constraint timelines record
their own kinds of values.

That distinction matters when the rig changes: correcting a bone in Setup changes
the foundation beneath every clip, while posing that bone in Animate changes only
the active clip.

## Before you key anything

1. Finish the structural part of the rig in **Setup** mode.
2. Open the **Animations** panel and create or select a clip.
3. Switch to **Animate** with <kbd>Tab</kbd>.
4. Set the clip duration and decide whether the authored clip loops.
5. Move to the frame you want, select a bone or slot, and edit a property.

The timeline is deliberately inactive in Setup mode. It shows an explanation and
an **Animate** button instead of a sheet that appears editable but cannot affect the
pose.

## Manage clips

The Animations panel is the source of clip selection and clip metadata. The list is
sorted alphabetically and each row reports its duration, number of timelines, and
number of events. A loop icon identifies clips whose saved **Loop animation** value
is enabled.

### Create a clip

Choose **New**. The editor creates a one-second clip, selects it, and makes it ready
for animation. Generated names are only starting points; rename the clip to an
action such as `idle`, `walk`, or `attack_heavy` before building a large library.

### Rename, duplicate, or delete

Right-click a clip for **Rename**, **Duplicate**, or **Delete**. Duplicate makes a
deep copy, including keys and clip data, under a generated unique name. Select the
copy from the alphabetized list before editing it. This is the safest starting
point for a variation such as `walk_injured` or before a broad retime.

Names must be non-empty and unique. Deleting the active clip selects another clip
when one remains. If no clips remain, Animate mode has no active animation to edit.

### Duration

Duration is stored in seconds and may be set from `0.05` to `600` seconds. It is
the playback boundary, not a destructive trim operation. If you shorten a clip
below its last authored key, the Animations panel warns **keys reach _n_s**. Those
later keys remain in the document and reappear if the duration is extended.

### Two different loop switches

- **Loop animation** in the Animations panel is saved with the clip for runtime
  handoff.
- **Loop** in the timeline transport controls only the editor's current preview
  session.

Changing one does not change the other. Use the transport loop while judging a
transition; set the clip property according to how the exported animation should
behave.

## Read the timeline

The Dopesheet and Graph are separate panels and may be visible together. They share
the same time zoom, horizontal scroll, row fold state, and row filtering.

The timeline body has four parts:

| Area | Meaning |
|---|---|
| Header | Active clip, transport, keying controls, onion skin, FPS, and zoom |
| Ruler | Frame or second labels, clip-wide key summaries, markers, and playhead |
| Event lane | Runtime events that fire during playback |
| Row tree and sheet | Animated targets, property tracks, keys, and curves |

Rows are grouped under the bone, slot, or global target they affect. Expand a group
to see the real property rows. Translate, scale, and shear use independent X and Y
rows; rotation uses one row. The axes may therefore have different key times and
different easing.

### Summary markers are not keys

The small summary diamonds on the ruler combine key times from every property
track. They answer “does anything change here?” and make navigation easier. The
editable keys live on property rows. Editing a summary as though it were a real key
would be ambiguous, so summaries are read-only.

### Editable and read-only rows

Bone transform rows and slot color or attachment rows can be edited directly in the
dopesheet. Specialized rows—draw order, IK values, transform-constraint mix,
deform, and visibility—are shown for timing context but are read-only there. Edit
those values in their owning panel or inspector. A read-only row is muted and uses
the read-only icon.

### Fold and isolate rows

- Click the triangle on a target group to fold or unfold it.
- Click the dot at the right of a group to isolate all of its property rows.
- Click a property dot to isolate only that row.
- More than one row may be isolated at once.
- When isolation is active, click the **solo _n_** indicator in the header to show
  everything again.

Isolation is a view filter. Muted keys stay visible so the sheet never conceals
what the clip contains, but they cannot be edited until their row is shown.

## Navigate time

The orange line and triangular handle are the current playhead.

| Control | Shortcut | Result |
|---|---|---|
| Start | — | Stop and move to time zero |
| Previous key | <kbd>Ctrl</kbd>+<kbd>←</kbd> | Jump to the previous key on any track |
| Previous frame | <kbd>←</kbd> | Stop and move back one frame |
| Play or pause | <kbd>Space</kbd> | Toggle preview playback |
| Next frame | <kbd>→</kbd> | Stop and move forward one frame |
| Next key | <kbd>Ctrl</kbd>+<kbd>→</kbd> | Jump to the next key on any track |
| End | — | Stop at the clip duration |

Starting playback while already at the end rewinds first. Without preview looping,
playback stops at the duration. With it enabled, overshoot is carried into the next
lap so playback speed remains steady.

Click or drag the ruler to scrub. Scrubbing normally snaps to whole frames and is
also attracted to a nearby named marker. Hold <kbd>Alt</kbd> to ignore marker
magnetism while scrubbing; frame snapping still applies.

### FPS is a grid, not a retime command

The FPS field accepts `1` through `240`. It controls frame labels, stepping, and
frame snapping. Keys remain stored in seconds. Changing a project from 30 FPS to
60 FPS does not move keys to preserve their old frame numbers; the same key at
`0.5` seconds is displayed as frame 15 and then frame 30.

### Pan and zoom

- Use the zoom buttons, type or drag the **px/f** value, or use
  <kbd>Ctrl</kbd>+wheel over the timeline.
- A trackpad pinch also zooms around the pointer.
- Click **Fit** to show the complete clip.
- Middle-drag the sheet to pan in time.
- <kbd>Shift</kbd>+wheel, or a horizontal wheel gesture, scrolls in time.
- Plain vertical wheel scrolls the property rows.
- Drag the divider to resize the target-name column.

## Key a bone transform

Select a bone and use the viewport gizmo or Inspector. Translate, rotate, scale,
and shear are separate properties; translate, scale, and shear are also split into
independent X and Y tracks.

The small key dot beside each Inspector field describes that exact track:

| Dot state | Meaning |
|---|---|
| Empty outline | This clip has no timeline for the property |
| Hollow ring | A timeline exists, but there is no key at the playhead |
| Filled | A key exists at the playhead |
| Amber | The displayed value has been changed but not committed |

Click a dot to write or update that field's key at the playhead. Hold
<kbd>Alt</kbd> and click a filled dot to remove the key at that time. Removing the
last key does not change the Setup pose; evaluation falls back to the remaining
timeline or Setup value.

### Auto-key on

Auto-key is enabled by default in Animate mode. Posing a bone writes only the
channels that actually changed. Moving horizontally does not create an unnecessary
Y translation key, for example.

When the first key for a changed channel is added after time zero, Ankhimate also
creates a baseline key at time zero. The baseline represents no offset from Setup
(`0` for translation, rotation, and shear; `1` for scale). This makes the new
motion ease from the Setup pose instead of applying the later value from the start
of the clip.

### Auto-key off and pending poses

Turn off the red record control when you want to try a pose without immediately
writing it. Bone edits then remain as an unsaved preview and the Inspector shows
amber key dots. Press <kbd>K</kbd>, click one of those dots, or use the enabled key
button in the transport to commit the pose.

A pending pose belongs to the frame where it was made. Scrubbing, stepping, or
otherwise moving the playhead discards it and restores evaluated animation. Save
the pose with a key before leaving the frame.

The pending-pose workflow currently applies to bone transforms. Slot color,
attachment, and draw-order edits use their own animation routing and do not offer
the same held-preview behavior.

### Key the active transform property

With the Dopesheet or Graph focused, <kbd>K</kbd> also keys the active bone's active
transform property. Translate, scale, and shear write both axes in this explicit
property-key path; Inspector dots remain the precise way to key only one axis.

## Select and edit dopesheet keys

- Click a key to replace the current key selection with that key.
- <kbd>Ctrl</kbd>-click a key to toggle it in the selection.
- Drag on empty space to box-select. Without <kbd>Ctrl</kbd>, this clears the old
  selection first; with <kbd>Ctrl</kbd>, it adds to it.
- Click empty sheet space to clear the selection.
- Drag any selected key horizontally to move the whole selection. Movement snaps
  to frame increments and repeated drag updates merge into one undo step.
- Drag selected keys below the bottom of the sheet and release to delete them.
- Right-click a key to delete the selection or assign an interpolation preset.

Available presets are **Linear**, **Stepped**, **Ease In**, **Ease Out**,
**Ease In-Out**, **Sine In-Out**, and **Snap**. Snap is a designer-facing alias
for Stepped.

**Not supported:** copying and pasting an arbitrary dopesheet key selection is not
implemented. <kbd>Ctrl</kbd>+<kbd>C</kbd> and <kbd>Ctrl</kbd>+<kbd>V</kbd> operate on
selected bones or pose clipboard data, not timeline keys.

## Shape motion in the Graph

The Graph plots continuous numeric channels from unfolded, visible rows. It can
edit bone translate, rotate, scale, and shear curves, plus slot color alpha. It
does not plot discrete attachment, visibility, draw-order, event, marker, or
deform data. If no numeric channel is available, it says so rather than drawing an
empty graph with unexplained controls.

### Move graph keys

Drag a key point horizontally to change its time and vertically to change its
value. Horizontal motion snaps to frames by default; hold <kbd>Alt</kbd> for free
time placement. A visible **snap: frames** or **free** badge always reports the
current mode.

The value range fits the sampled curve, not just key values. This keeps a Bézier
overshoot visible even when its control handles carry the curve beyond both keys.

### Interpolation belongs to the arriving key

The interpolation stored on a key describes how the animation approaches that key
from the previous one:

- **Linear** advances at a constant fraction of the segment.
- **Stepped** holds the previous value until the arriving key's time.
- **Bézier** maps normalized segment time through two editable control handles.

Linear segments show straight-line handles; moving either converts the segment to
Bézier without first changing its shape. Stepped segments have no
curve to shape and therefore show no handles.

Handle time is kept within the segment so the curve never doubles back in time.
Handle value is deliberately unbounded. Pulling it above or below the endpoint
range creates anticipation, bounce, or follow-through. If the sampled curve moves
more than 10% beyond the keyed value range, the graph displays an **overshoot**
warning. This is information, not an error, and the value is not clamped.

If a control point lies outside the fitted panel, its marker is pinned to the
nearest edge as a hollow ring. Dragging that ring still edits the real off-panel
handle. This prevents an extreme curve from becoming impossible to recover.

**Not supported:** numeric tangent entry and typed handle coordinates are not
implemented. Handle editing is graphical. For the design rules behind these
behaviors, see [Graph editor interaction rationale](/graph-editor/).

### Rotation and the ±180° boundary

Rotation interpolates by the shortest arc. A change from `170°` to `-170°` moves
through the nearby boundary instead of turning 340 degrees the long way around.
For a deliberate full spin or a chosen long direction, add intermediate keys that
make the intended path unambiguous.

## Animate slots and attachments

### Slot color and opacity

Slot color keys store absolute normalized RGBA. The Inspector color key dot writes
the full color. The Graph currently displays and edits only alpha, so change RGB in
the Inspector. Alpha is continuous and can be eased; use visibility for a hard cut.

### Visibility

Visibility is stepped: a slot is either drawn or hidden. In Animate mode, use the
visibility key control in the slot Inspector. The dopesheet shows the timing as a
read-only row.

### Attachment switching

Changing the active attachment in Animate mode writes a stepped attachment key.
Names cannot blend, so the previous attachment remains active until the switch
time. Attachment keys appear in an editable dopesheet row but have no graph curve.

To avoid an unintended attachment appearing before the first switch, establish the
desired starting attachment at frame zero, then add later switches.

### Draw order

Open the Draw Order panel in Animate mode and reorder slots. The editor stores a
stepped draw-order key as offsets from Setup order. When the first later key is
created, a frame-zero baseline preserves the Setup stack before it. The dopesheet
shows draw-order timing but edits remain in the Draw Order panel.

See [Rigging fundamentals: draw order](/animator/rigging/#draw-order) for the
relationship between slot hierarchy and the rendered stack.

### Mesh deformation

Deform keys record vertex offsets for a particular slot and attachment. They are
authored from mesh-edit tools and shown read-only in the dopesheet. Binding and
weighting determine how the base mesh reaches the posed location before deform
offsets are added.

See [Meshes, binding, weights, and deformation](/animator/deformation/) for the
complete workflow and the current per-influence editing limitation.

## Animate constraints

**Partial:** the animation model supports IK mix, IK bend direction, IK softness,
and combined transform-constraint mixes. Imported or plugin-authored keys appear as
read-only global rows, but the current editor has no control for creating or
changing those constraint keys. The Constraints panel edits constraint setup, not
these animation timelines.

- Mix and softness are continuous values and may use easing.
- IK bend direction is stepped (`+1` or `-1`) so a chain never interpolates
  through an undefined middle direction.
- Constraint results are evaluated in constraint order after ordinary bone
  animation, so a fully mixed constraint can override motion that is visibly
  present on bone tracks.

This section records the current animation boundary only. Constraint setup and
artistic recipes need their own focused manual pass.

## Events and markers

Events and markers can share a time, but they serve different consumers.

| Feature | Saved | Runtime meaning | Typical use |
|---|---:|---|---|
| Event | Yes | Fires application data during playback | Footstep, hit frame, sound cue |
| Marker | Yes | Editor navigation label; not a runtime event | Contact pose, anticipation, loop seam |

### Author runtime events

The thin **events** lane sits below the ruler. Double-click empty lane space to add
an event at a frame-snapped time. Drag a pennant to retime it. Right-click a
pennant to rename or delete it.

Use the Events panel for complete data. **New** creates an event at the playhead;
the toolbar can also duplicate or delete the selected event. Clicking an event row
selects it and moves the playhead to its time. The form edits:

| Field | Meaning |
|---|---|
| Name | Callback or event identifier |
| Time | Seconds in the clip; **At playhead** copies the current time |
| Integer | Optional signed integer payload |
| Float | Optional floating-point payload |
| String | Optional text payload |
| Audio | Optional asset name |
| Volume | `0.0`–`2.0`; shown when Audio is non-empty |
| Balance | `-1.0` hard left, `0.0` center, `1.0` hard right; shown with Audio |

Events are kept in time order. Runtime playback fires events when moving forward;
scrubbing backward is navigation and does not replay them.

### Add navigation markers

Press <kbd>M</kbd> to add a marker at the playhead, or right-click empty ruler
space and choose **Add marker here**. Drag the flag to a frame-snapped time.
Right-click it to rename it, set its color, or delete it. A marker attracts ruler
scrubbing when the pointer is within a few pixels, unless <kbd>Alt</kbd> is held.

Use markers to name poses and review points. Use events only when an exported
runtime must react.

## Offset and retime motion

### Non-destructive bone track offsets

Right-click a bone group in the timeline tree to edit **Track offset** in frames.
A positive offset makes that bone trail; a negative offset makes it lead. The
stored keys do not move. Instead, the evaluator reads that bone's tracks earlier or
later. The group label displays the signed offset, such as `+4f`, because the key
positions alone cannot explain the delayed result.

Track offsets are useful for hair, cloth strips, tails, or repeated appendages.
They affect the selected bone's animation sampling; inspect the hierarchy and
constraint order if descendants do not appear to trail as expected.

### Clip-wide retiming

The **Pose** menu offers these Animate-mode commands:

- **Half Speed** multiplies every key time and the clip duration by `2.0`.
- **Double Speed** multiplies every key time and the duration by `0.5`.
- **Shift Keys +1** or **Shift Keys −1** shifts all keys in the active clip by one
  frame using the current FPS.
- **Clear Animation** removes bone timelines for the selected bones, or all bones
  when nothing is selected. It does not clear slot, event, draw-order, deform, or
  constraint data.

These are undoable clip operations. Duplicate a clip before a broad creative
retime when you want to compare versions rather than rely on undo history.

### Pose clipboard is not key clipboard

<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> copies the displayed pose of selected
bones. <kbd>Ctrl</kbd>+<kbd>V</kbd> pastes the current clipboard; the result follows
Setup/Animate routing. <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> pastes a pose
mirrored by flipping X translation and rotation. This transfers a pose at the
current time, not a block of dopesheet keys with their spacing.

## Onion skinning

**Experimental:** onion skinning is available in Animate mode with an active
clip. Toggle the onion icon in the transport, then right-click it to configure:

- `1`–`5` neighboring steps;
- stepping between authored keys instead of adjacent frames;
- limiting ghosts to selected bones.

Onion settings are session view state, not animation data. The feature is useful
for spacing checks, but the complete T-703 interaction acceptance pass is still
open; do not treat its current presentation as a stable production workflow.

## Troubleshooting animation

### I moved a bone, but no key appeared

Confirm that Animate mode and a clip are active. If auto-key is off, look for an
amber Inspector dot and press <kbd>K</kbd> before leaving the frame. A locked bone
refuses both Setup edits and animation keys.

### My unkeyed pose vanished

Moving the playhead intentionally discards a pending pose. Recreate it and click
the exact Inspector dot or press <kbd>K</kbd> before scrubbing.

### A property changes before its first visible key

Continuous timelines hold a value outside their keyed span. Normally the editor
creates a frame-zero Setup baseline when auto-key first creates a later bone track.
Imported data or manually removed baseline keys may not have one. Add the intended
starting key at frame zero.

### Shortening the clip hid keys

Duration is not a trim. Increase it until the **keys reach** warning disappears,
then move or delete the later keys deliberately.

### A key will not move between frames

Dopesheet drags always move by frame increments. In the Graph, hold
<kbd>Alt</kbd> while dragging for free-time placement. The ruler continues to use
frame snapping even when <kbd>Alt</kbd> disables marker magnetism.

### The curve leaves the keyed value range

That is Bézier overshoot. Check the graph's percentage warning and handle rings.
It may be intentional anticipation; otherwise pull the handles toward the segment
or assign a Linear/Ease preset from the dopesheet key menu.

### Rotation turns the wrong way

Rotation takes the shortest arc between adjacent keys. Add an intermediate key on
the intended side of the ±180° boundary to specify a longer turn.

### I can see a key but cannot select it

It may be a group/ruler summary, a muted isolated row, or a specialized read-only
track. Expand the group, clear solo filtering, and edit the actual property row or
the feature's owning panel.

### Motion timing and key positions disagree

Look for a signed frame badge on the bone group. A track offset changes sampling
without moving keys. Clear the offset from the group's right-click editor if that
delay or lead is no longer wanted.

### Preview looping and exported looping disagree

Set **Loop animation** in the Animations panel for saved behavior. The transport
loop is only the current editor preview.
