---
title: Constraints
description: Build and tune IK, transform, path, and physics constraints, control solve order, and diagnose unexpected motion.
---

# Constraints

Constraints make bones respond to targets, paths, or simulated motion after their
ordinary parent-child pose has been evaluated. They are rig structure: create,
configure, reorder, and delete them in **Setup** mode. In Animate mode, moving a
target bone may be keyed like any other bone, but the constraint relationship
itself remains part of the reusable rig.

Ankhimate currently provides four constraint types:

| Type | Artistic purpose | Typical examples |
|---|---|---|
| IK | Make a bone chain reach toward a target | Arm, leg, aiming bone, tentacle |
| Transform | Copy selected transform channels from another bone | Look direction, mirrored control, follow bone |
| Path | Place and orient bones along an authored polyline | Tail, tread, belt, vine |
| Physics | Add stateful lag, sway, and settling | Hair, cloth strip, antenna, loose tail |

## Constraint workflow

1. Switch to **Setup**.
2. Select the bone, or a continuous chain of bones, that should be driven.
3. In the Inspector, choose **Add driver…**.
4. Pick IK, Transform constraint, or Physics. Path constraints are created from a
   path attachment as described later in this chapter.
5. Select the new constraint in the **Constraints** panel or through the selected
   bone's Inspector.
6. Tune the controls while moving the target or playing representative motion.
7. Check the global solve order whenever more than one constraint affects the same
   limb or a constraint's target is itself constrained.

All creation, property changes, ordering, and deletion are undoable document edits.
Simulation buttons such as Physics **Pause** and **Reset** are session controls and
are not undoable or saved as part of the rig.

## Find and select constraints

The Inspector lists constraints on the bones they **drive**, not only on their
targets. Select a bone that moves unexpectedly and inspect its constraint sections
to answer “what is writing this pose?”

The Constraints panel shows every constraint in solve order. Each row reports:

- its type and generated name;
- how many bones it drives;
- its target where applicable;
- a useful activity value, such as IK mix or transform rotation mix.

Click a row to focus that constraint. The Inspector then shows its kind, all driven
bones, and the same per-type editor used from a bone selection.

**Current limitation:** the editor generates constraint names but does not provide a
rename control. Imported or plugin-authored names are displayed as supplied.

## Solve order

Constraints run from top to bottom in the Constraints panel. Each constraint sees
the result of all earlier constraints. After one changes a bone, the editor
recomputes that bone's descendants before the next constraint runs.

Use the up and down arrows to reorder constraints in Setup mode. Order is saved and
can materially change the pose.

For example, a leg IK should usually solve before a foot-aim constraint. If the foot
solves first, it aims from the unsolved leg pose; the later leg movement carries the
foot away from that result.

Practical ordering rules:

1. Put broad body or limb placement before small corrective controls.
2. Put a chain's IK before constraints that refine its tip.
3. Put path placement before a transform constraint that adjusts a path-driven bone.
4. Put physics after the deliberate controls whose motion should disturb it.
5. When two constraints intentionally compete, remember that the later result is
   applied to a pose already changed by the earlier one; mix values do not make
   ordering irrelevant.

If changing order fixes a rig, use recognizable bone and target names, because
constraint names themselves cannot currently be renamed in the editor.

## Inverse kinematics

IK answers: “where must this chain point or bend so its tip reaches the target?”
The target is an ordinary bone. Animate that target to animate the reach.

Ankhimate selects a solver according to chain length:

| Chain length | Behavior |
|---:|---|
| 1 bone | Aim the bone's X axis toward the target |
| 2 bones | Solve the two joints exactly with the selected bend side |
| 3 or more bones | Solve the complete chain with FABRIK |

The root of a long chain stays fixed. Every segment retains its length unless
**Stretch** is enabled. A target beyond natural reach makes a non-stretching chain
point straight toward it rather than break bone lengths.

### Create an IK target

For one-bone aim:

1. Select the bone.
2. Choose **Add driver… → IK target**.

For a limb or long chain:

1. Select a continuous parent-to-child chain in the Hierarchy. Shift-click is the
   usual way to extend the selection through the visible hierarchy.
2. Confirm that no unrelated bone is selected.
3. Choose **Add driver… → IK target (_n_-bone chain)**.

The selection must form one unbroken parent-child line. A disjoint selection does
not enable multi-bone IK creation.

Creation is one undo step. It adds:

- an unparented, zero-length target bone at the current world-space tip of the
  selected chain; and
- an IK constraint that drives the chain root-to-tip.

The target is deliberately unparented. Parenting it under the driven chain would
make the solver chase its own output. The initial target position matches the tip
so enabling the new constraint does not immediately jump the limb.

### IK controls

| Control | Range | Meaning |
|---|---:|---|
| Target | Any selectable target bone | Bone position the chain reaches toward |
| Mix | 0.0–1.0 | 0 is the original FK pose; 1 is the complete IK result |
| Softness | 0–1000 world units | Distance over which extension eases near full reach |
| Stretch | Off/on | Permit the chain to lengthen for an out-of-range target |
| Stretch limit | 1.0–3.0 | Maximum length factor when Stretch is enabled |
| Flip bend | Off/on | Choose the side on which the chain folds |
| Stiffness | 0.0–1.0 | For 3+ bones, preserve the authored bend instead of redistributing it |

The target picker excludes the currently inspected bone, but you should also avoid
choosing any other bone inside the driven chain. A target inside its own chain is a
feedback loop and the evaluator skips that IK constraint.

Deleting an IK constraint leaves its target bone in the rig. Delete or repurpose the
target separately if it is no longer needed.

### Mix

Mix blends rotation and any permitted stretch from the ordinary FK pose toward the
solved result. Use partial mix when keyed joint rotations should remain visible or
when IK is only a correction.

- 0.0: the constraint is inert.
- 0.5: each affected rotation moves halfway toward the solved direction.
- 1.0: the chain follows the target as completely as reach permits.

Rotation blending uses the short direction across the ±180° boundary, avoiding a
full spin when the target crosses that seam.

### Softness

Softness affects only the approach to full extension. A target comfortably inside
the chain's reach is still reached exactly. Near the reach boundary, the effective
target approaches maximum extension smoothly instead of snapping the joints into a
straight line.

Use a small positive softness for arms and legs that visibly pop as a target moves
through their maximum reach. The value is a world-space distance, so its useful
size depends on the dimensions of the rig.

Softness is applied before stretch. A soft, stretching chain therefore eases toward
extension before calculating how much extra length it needs.

### Stretch and stretch limit

Stretch scales the chain along each bone's length axis only when the target is
beyond natural reach. The factor is capped by **Stretch limit**:

- 1.0 permits no growth even if Stretch is checked;
- 1.1 permits up to 10% growth;
- 2.0 permits up to double length.

Partial IK mix also partially applies stretch. Because scale inheritance already
carries a parent's growth into connected children, the evaluator avoids multiplying
the same stretch repeatedly down a continuous chain.

Keep the limit close to 1.0 for limbs. Larger factors are more suitable for
deliberately elastic rigs.

### Flip bend

Flip bend chooses the side of the root-to-target line on which joints should land.
For a two-bone leg, it is the knee direction. For a long chain, it selects the side
used to seed the distributed curve and is enforced again after solving.

If a chain flips unpredictably:

1. verify the selected bones are ordered root-to-tip;
2. toggle **Flip bend**;
3. move the target away from the exact root position;
4. check for zero-length bones;
5. inspect later constraints that may rotate the result again.

### Stiffness for long chains

Stiffness appears only for chains of three or more bones because one- and two-bone
IK have exact solutions with no starting-shape choice.

- 0.0 starts from an even arc, spreading bend across the chain. Use it for a
  tentacle, tail, rope, or flexible vine.
- 1.0 starts from the current posed shape and moves only what it must. Use it for
  a hand-posed spine or a chain whose authored keys should remain recognizable.
- Intermediate values blend those preferences.

A long chain can reach the same target with many valid shapes. Stiffness chooses
which family of solution should be preferred; it is not a conventional spring
stiffness and does not add simulated motion.

### Degenerate IK inputs

IK safely does nothing rather than producing a corrupt pose when:

- the target is missing;
- a driven bone is missing;
- the target is part of its own chain;
- a long-chain segment has effectively zero length;
- a one-bone target occupies exactly the same position as the aiming bone.

If the constraint row exists but nothing moves, first check **Mix**, then target and
chain validity.

## Transform constraints

A transform constraint makes one or more bones follow selected channels of another
bone. Unlike IK, it does not try to reach a point with a chain. It compares the
target's transform with each driven bone and blends rotation, translation, scale,
and shear independently.

### Create a transform constraint

1. Select the driven bone.
2. Choose **Add driver… → Transform constraint**.
3. Pick the intended target in the new Inspector section.
4. Enable only the channels that should follow.

The new constraint begins as rotation-only at full rotation mix. Its initial target
is the selected bone's parent when available, otherwise the first other bone in the
rig. A transform constraint cannot drive its target from itself; creation refuses a
selection that leaves no valid driven bone.

### Transform controls

| Control | Range | Meaning |
|---|---:|---|
| Target | Another bone | Source transform |
| Rotate | 0.0–1.0 | Amount of target rotation |
| Translate X/Y | -1.0–1.0 | Amount of target position per local bone axis |
| Scale X/Y | -1.0–1.0 | Amount of target scale per axis |
| Shear X/Y | -1.0–1.0 | Amount of target shear per axis |
| Offset° | -360–360 degrees | Rotation added to the target before mixing |
| Local | Off/on | Compare each bone's transform relative to its parent |
| Add | Off/on | Add the target contribution instead of moving toward it |

The signed two-axis mixes are intentional. A translation mix of X 1 and Y 0 follows
only horizontally. A negative X value mirrors the target's X contribution. Rotation
is a single angle, so it has one unsigned mix.

**Current limitation:** the data model can store position, scale, and shear offsets,
but the current Inspector exposes only the rotation offset.

### World versus Local

With **Local** off, the constraint compares world transforms. Two bones under
different parents can face the same world direction or approach the same world
position.

With **Local** on, it compares each bone's transform relative to its own parent.
This copies a relationship rather than an absolute placement. For example, a right
hand can copy how far a left hand is rotated relative to its forearm even when the
two arms occupy different parts of the rig.

Translation X and Y mixes are applied in the driven bone's parent frame, matching
the axes used by its transform controls. They do not silently become fixed world X
and Y under a rotated parent.

### Absolute versus Add

With **Add** off, the driven bone moves toward the target plus offsets. At full mix,
the enabled channel becomes the target's value in the selected space.

With **Add** on, the target's transform is added to the driven bone's existing pose.
This layers the relationship over animation instead of replacing it. Additive scale
is multiplicative, while translation, rotation, and shear are added.

Use absolute mode for “be like that target.” Use Add for “keep this pose and inherit
that target's motion too.”

### Common transform setups

For a head direction control:

1. Create a separate target/control bone.
2. Add a Transform constraint to the head.
3. Leave only Rotate at 1.0.
4. Set **Offset°** if the artwork's resting direction differs from the target.

For a one-axis follower:

1. Set Rotate, Scale, and Shear to zero.
2. Set Translate X to the desired positive or negative mix.
3. Leave Translate Y at zero.
4. Test under the actual rotated parent hierarchy before deciding the sign.

## Path constraints

A path constraint places bones at sampled positions along a path attachment and can
turn them to follow each segment. Paths are stored as polylines: the straight
segments visible in Setup are the same geometry evaluation follows.

### Create and shape a path

1. In Setup, create or select an empty slot on the bone that should own the path.
2. In the slot Inspector choose **Add path**.
3. The editor creates an open, three-point path, makes it the slot's active
   attachment, and enters polygon editing.
4. Drag a point to move it.
5. <kbd>Shift</kbd>-click points to toggle a multi-selection.
6. <kbd>Ctrl</kbd>-drag empty space to box-select points; hold <kbd>Shift</kbd>
   while releasing to add to the existing selection.
7. Click near a segment to insert a point between its endpoints.
8. Press <kbd>X</kbd> or <kbd>Delete</kbd> to remove selected points. A path keeps
   at least two.
9. Press <kbd>Esc</kbd> to leave polygon editing.

Path vertices are local to the slot's bone. Moving or animating that bone moves the
path in world space.

Newly authored paths are open and use constant-distance sampling. **Partial:** the
format and evaluator also support closed paths and vertex-index sampling, but the
editor currently has no controls for changing the closed or constant-speed
properties. Imported or plugin-authored values are honored.

### Connect bones to the path

1. Select the bones to drive. The current selection order becomes the path order.
2. Select one of those bones so its Inspector is visible.
3. Under the path-constraint section, choose the path slot from **Drive along**.

The picker lists slots whose current attachment is a path in the default skin. A
path override that exists only in another skin is not offered. At evaluation time,
path constraints also deliberately resolve from the default skin so changing an
outfit cannot unexpectedly reroute the skeleton.

### Path controls

| Control | Range | Meaning |
|---|---:|---|
| Position | 0.0–1.0 | Starting fraction of path length |
| Spacing | 0.0–2.0 | Multiplier for the normal gap between bones |
| Rotate | 0.0–1.0 | Amount each bone follows path direction |
| Translate | 0.0–1.0 | Amount each bone moves to its sampled path point |

At Position 0, the first bone starts at the path beginning. Increasing Position
slides all placements forward. On an open path, placements beyond either end clamp
to the endpoint and keep its final direction. A closed imported path wraps.

At Spacing 1, multiple bones are spread across one complete path length. Lower
values pack them closer; higher values push later bones farther along or beyond an
open path's end.

With constant-distance sampling, equal gaps mean equal world distance even when
path segments have very different lengths. With imported index sampling, each path
segment receives an equal share regardless of physical length, so bones crowd in
densely authored sections.

**Not supported:** Bézier handles for path geometry are not implemented. Add more
polyline points where a path needs a smoother turn.

**Not supported:** the current editor does not key Path Position, Spacing, Rotate,
or Translate. Despite the Position tooltip mentioning animation, no path timeline
variant or key control is currently available.

## Physics constraints

Physics adds secondary motion to one bone. Parent motion disturbs the constrained
bone; a spring returns it toward the ordinary evaluated pose. Children follow the
simulated local offset.

Unlike IK, Transform, and Path, physics depends on previous frames. Play from a
known state when comparing settings. A still scrub evaluates the settled/rest pose
rather than inventing an arbitrary frozen simulation frame.

### Create physics

1. Select the bone that should sway.
2. Choose **Add driver… → Physics**.
3. Enable **Run in Setup** while tuning without a clip, or play an animation that
   moves the bone's parent.

The default is rotation-only sway with Inertia 0.5, Strength 40, Damping 0.5, Mass
1, Mix 1, and no wind or gravity.

### Physics controls

| Control | Editor range | Meaning |
|---|---:|---|
| Inertia | 0.0–1.0 | Resistance to following parent motion; higher produces more lag |
| Strength | 0.0–200.0 | Spring pull toward the ordinary pose |
| Damping | 0.0–1.0 | Rate at which motion loses energy |
| Mass | 0.05–10.0 | Heavier values react less to the same push |
| Wind X/Y | Unbounded drag values | Constant world-space push |
| Gravity Y | Unbounded drag value | Constant vertical world-space pull; negative is down |
| Mix | 0.0–1.0 | Blend from unsimulated pose to simulated offset |
| Rotate | Off/on | Apply simulated angular sway |
| Translate | Off/on | Apply simulated positional drift |

The document stores both gravity axes, but the current Inspector exposes Wind X and
Y and only Gravity Y.

### Tune by effect

For heavy delayed motion, raise Inertia or Mass. They are not identical: Inertia
increases the disturbance from parent movement, while Mass reduces response to the
spring and constant forces.

For a tight return, raise Strength. Very high strength with low damping produces a
fast oscillation. Raise Damping to remove energy sooner. At Damping 0, motion does
not settle on its own; at 1, the default spring returns with little overshoot.

Use Wind for a persistent directional push. Use negative Gravity Y for downward
pull in Ankhimate's Y-up world. If Translate is off, those forces can still create
rotation through their sideways component.

Mix 0 makes the constraint visibly inert without deleting it. Turning both Rotate
and Translate off also makes it inert.

### Simulation controls and readout

- **Reset** forgets accumulated offsets and velocities and returns all simulated
  bones to rest.
- **Pause** freezes simulation state without losing it.
- **Run in Setup** advances physics while the editor is in Setup mode, allowing
  live tuning without an animation.
- Turning **Run in Setup** off resets physics.
- The Inspector reports **moving** with angular and positional speed, or
  **settled** when both fall below the stability threshold.

Physics normally advances while timeline playback is running. It uses a fixed
internal step so 30, 60, and 144 Hz playback follow substantially the same
trajectory. Extremely long frame hitches use a capped catch-up budget; the
simulation may under-advance rather than freeze the editor trying to replay every
missed step.

Physics state belongs to the editor session, not the saved rig. Each runtime rig
instance needs its own simulation state. A thumbnail or static render with no
simulation state shows the settled pose.

**Not supported:** physics properties do not have animation key controls or physics
timeline variants in the current editor.

## Constraint animation status

Constraint setup fields are disabled in Animate mode. The current implementation
has this narrower timeline support:

| Timeline data | Evaluated | Editable in timeline | Authoring control in editor |
|---|---:|---:|---:|
| IK mix | Yes | Read-only row | No |
| IK bend direction | Yes, stepped | Read-only row | No |
| IK softness | Yes | Read-only row | No |
| Transform per-channel mixes | Yes | Read-only row | No |
| IK stretch or stiffness | No timeline type | — | No |
| Path properties | No timeline type | — | No |
| Physics properties | No timeline type | — | No |

Imported, migrated, or plugin-authored supported timelines affect evaluation and
appear in the Dopesheet. **Partial:** there are no Inspector key dots or other
current editor controls for creating and changing those constraint keys.

The ordinary workaround is often to animate a target bone:

- animate an IK target to change reach;
- animate a Transform target to change what followers receive;
- animate the bone that owns a path to move the entire path;
- play authored parent motion to drive physics.

These techniques animate inputs to a constant relationship. They do not replace an
animated Mix control when the artistic requirement is to switch a constraint on or
off during a clip.

## Practical recipes

### Two-bone arm or leg

1. Build a direct parent-child pair with meaningful nonzero lengths.
2. Select root then child as one continuous chain.
3. Create the IK target.
4. Move the target around the reachable area.
5. Toggle Flip bend until the elbow or knee folds correctly.
6. Add a small Softness value if the limb pops near full extension.
7. Keep Stretch off for a rigid limb, or use a limit near 1.05–1.1 for subtle
   forgiveness.
8. Animate the target bone in Animate mode.

### One-bone aim

1. Select the aiming bone and create an IK target.
2. Move the target away from the bone origin.
3. Reduce Mix if authored rotation should still contribute.
4. If the artwork points along an axis other than the bone's X axis, correct the
   Setup bone or use a rotation-only Transform constraint with an offset instead.

### Long tentacle

1. Build and select a continuous chain of three or more nonzero-length bones.
2. Create one IK target for the whole chain.
3. Start with Stiffness 0 for distributed curvature.
4. Set the bend side before animating the target across the chain.
5. Increase Stiffness only when the chain should retain keyed curls.
6. Use Softness near full reach; enable limited Stretch only for deliberate
   elasticity.

### Secondary-motion hair

1. Put primary motion on the parent or root hair bone.
2. Add Physics to the child that should lag.
3. Enable Run in Setup and move the parent to test.
4. Start from the defaults, then raise Inertia for lag, adjust Strength for return
   speed, and Damping for settling.
5. Add physics to later children individually if the strand needs multiple
   simulated joints.
6. Order physics after deliberate placement constraints.

### Bones following a route

1. Create an empty slot and add a path.
2. Insert and drag points until the polyline describes the route.
3. Select driven bones in the intended order.
4. Choose the path under Drive along.
5. Set Translate to 1; blend Rotate according to how strongly bones should face
   the route.
6. Adjust Spacing before Position, because spacing determines where later bones
   can fit.

## Troubleshooting constraints

### The constraint exists but does nothing

Check the relevant Mix first. For Transform, every channel may be zero. For Path,
both Translate and Rotate may be zero. For Physics, Mix may be zero or both channel
checkboxes may be off. Then verify targets, driven bones, and path attachments still
exist.

### I cannot add or edit a constraint

Switch to Setup mode. Constraint relationships are structure and their controls are
disabled in Animate. For IK, ensure the selected bones form one continuous
parent-child chain. For Path, ensure an active default-skin path attachment exists.

### IK does not reach the target

The target may be outside natural reach, Mix may be below 1, or Softness may be
deliberately easing the final extension. Enable Stretch and inspect its limit only
if changing bone length is acceptable. A zero-length segment causes long-chain IK
to leave the chain unchanged.

### IK bends backward or changes side

Toggle Flip bend. Make sure the target is not exactly at the root and no later
constraint rewrites the chain. For long chains, lower Stiffness if an authored
starting pose is overpowering the distributed bend you expect.

### Moving the target creates unstable feedback

Ensure an IK or Transform target is not one of its own driven bones or parented
under the driven chain. The target created by **IK target** is safely unparented;
manual reparenting can reintroduce the loop.

### A child is correct until another constraint runs

Inspect solve order. Later constraints read and modify the earlier result. Move the
broad placement constraint upward and the correction downward, then retest all
affected descendants.

### Transform follows in the wrong space

Toggle Local and compare. World mode matches absolute orientation or placement
across different parents; Local copies the relationship to each parent. If only one
direction is wrong, inspect the signed per-axis mix rather than changing space.

### Path bones bunch at a corner

New paths use constant-distance sampling, so check for duplicate or nearly
coincident points and confirm the file was not imported with index-based spacing.
The editor currently cannot toggle an imported path back to constant-distance
sampling.

### Path bones pile up at the end

On an open path, Position plus Spacing may place later bones beyond the available
length; those samples clamp to the endpoint. Lower Position or Spacing, lengthen
the path, or author a closed path through a format/plugin workflow.

### Physics does not move while scrubbing

This is intentional. Physics advances during playback or while Run in Setup is
enabled. Static scrubbing shows a reproducible settled pose.

### Physics never settles

Raise Damping, verify Strength is above zero, and remove or reduce constant Wind and
Gravity. Use the moving/settled readout rather than judging one frame where the bone
happens to pass through its rest position.

### Physics jumps after editing

Press Reset after large structural changes or major parameter changes. Physics
retains prior session velocity until reset; the saved document does not contain
that history.
