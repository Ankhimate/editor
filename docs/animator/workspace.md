---
title: Workspace and selection
description: Learn the hierarchy, inspector, viewport, animation panels, selection rules, and transform spaces.
---

# Workspace and selection

The central viewport shows the rig. The hierarchy organizes bones, slots,
attachments, constraints, and groups; the inspector edits the current target;
Assets holds imported artwork; the lower animation area contains the dopesheet
and graph editor. The context ribbon changes with the current tool. Panels use
tabs and most can be detached; **Partial:** the viewport itself cannot yet detach.

Click to replace the selection and use the platform multi-select modifier to add
or remove items. Box selection and shared-pivot transforms support bulk posing.
The hierarchy filter narrows visible rows without deleting or changing them.
Expansion, selection, camera, and active tool are session state: they are neither
saved nor undone.

In Setup mode, drag a bone in the tree to reparent it. Move slots to change setup
draw order. Groups organize the authoring view and must not cause an entity to be
drawn twice. Color chips identify bones in the viewport and weight tools; they do
not tint attached artwork.

The inspector's transform-space selector distinguishes local, parent, and world
editing. World values are computed views of local truth; editing a world value
solves a new local transform rather than storing a second transform.
