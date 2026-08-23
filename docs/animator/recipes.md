---
title: Practical recipes
description: Focused recipes for limbs, tentacles, secondary motion, skins, swaps, events, and export presets.
---

# Practical recipes

## Two-bone limb

Create upper and lower bones, parent the lower to the upper, attach artwork through
slots, and add an IK target. Constrain both bones, choose the bend side, and start
with full mix. Animate the target; add a pole-side correction only when the desired
bend changes.

## Long tentacle

Create a chain of three or more bones and use the arbitrary-length IK solver. Set
small stiffness differences from root to tip, then test reachable and unreachable
targets. `samples/tentacle.ankh` is the shipped reference.

## Secondary-motion hair

Build a short chain, add a physics constraint after primary posing constraints,
and begin with modest inertia plus damping. Test at different playback rates and
after a timeline jump; physics owns state, so settling matters.

## Reusable skin and attachment swap

Keep semantic slot and attachment names stable across skins. Add entries for each
outfit, then key the attachment track when a slot must switch images inside a clip.

## Hitbox event

Create a bounding-box attachment for geometry and an animation event at the active
time. Runtime code decides what the event means; the authoring file does not execute gameplay.

## Export preset

Copy a built-in preset, keep strict mode enabled, render filenames from stable
context fields, preview against a representative rig, then export into an empty
directory and inspect created, replaced, and orphan paths.
