# ADR 007: Physics state lives outside `evaluate`

- **Status:** Accepted
- **Date:** 2026-08-02
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §2.5, §2.6; T-503

## Context

Physics constraints (sway, bounce, jiggle) are *stateful*: where a hair bone
lands this frame depends on where it was last frame and how much time passed.
Every other constraint is a pure function of the document and the playhead.

`evaluate` is contractually pure and deterministic (PLAN §2.6): identical inputs
must produce a bit-identical `Pose`, because the editor viewport, the exporters,
and the shipping runtime all call it and must agree. Storing velocity inside the
`Skeleton` would make the document mutate during rendering; storing it in a
global or a thread-local would make two viewports of the same rig interfere and
would make an export depend on whatever the editor had been doing beforehand.

## Decision

1. **`PhysicsState` is caller-owned.** It maps `(constraint, bone)` to the
   simulation's velocity and last position. `evaluate` does not own it, does not
   allocate it, and cannot reach one implicitly.
2. **Two entry points.**
   - `evaluate(skel, anims, out)` — unchanged, still pure. Physics constraints
     apply their **rest** result: the pose the bone settles to with no motion.
     This is what a thumbnail, a diff, and a unit test should see.
   - `evaluate_with(skel, anims, &mut physics, dt, out)` — advances the
     simulation by `dt` and applies the result.
3. **Fixed-step integration.** `dt` is consumed in fixed sub-steps
   (`PHYSICS_STEP`), with the remainder carried in the state. A 30fps export and
   a 144Hz editor therefore integrate the same trajectory, and a frame hitch does
   not launch a hair bone across the screen.
4. **The caller owns the cadence.** The editor advances one state per viewport;
   an exporter advances at the export fps; a runtime holds one per animated
   instance. Nothing shares a state implicitly.

## Alternatives considered

- **State inside `Skeleton`:** simplest to write, and it makes the document
  mutate while drawing. Undo would have to snapshot velocity; two views of one
  rig would fight. Rejected.
- **Variable-step integration:** fewer moving parts, but the result depends on
  the frame rate, so an export at 30fps would not match the editor at 60. That
  breaks the one promise the pose contract makes. Rejected.
- **Physics as a post-pass outside constraints:** would not compose with
  constraint order — a physics bone driven by an IK chain has to run after it.
  Rejected.

## Consequences

- `evaluate` keeps its determinism test, and physics gets its own: the same `dt`
  sequence must produce the same trajectory.
- A caller that forgets to advance the state sees the rest pose, which is a
  visible, sensible failure rather than a frozen or exploding rig.
- The editor needs a "reset physics" action, since a state that has drifted
  cannot be recovered from the document alone.
