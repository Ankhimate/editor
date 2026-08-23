---
title: Setup and rigging
description: Build bones, slots, and attachments while keeping setup structure separate from animation.
---

# Setup and rigging

Use **Setup** for bones, parent relationships, slots, attachments, skins, and
constraint structure. Use **Animate** for pose changes and keys. Commands declare
their required mode; a caller cannot bypass this rule by using a menu, keymap,
plugin, or MCP script.

## Artwork and bones

Import images through Assets. Before bones exist, artwork can remain unattached.
After creating a bone, attach artwork with the asset action or by dragging it to
a compatible bone/tree/viewport target. A slot controls draw order and color; an
attachment supplies geometry and image data.

Bone transforms are local to the parent. Position places the joint, rotation turns
the local axes, scale stretches them, shear skews them, and length controls the
displayed/solver segment. Rotation, scale, and reflection inheritance flags decide
which parent components flow to a child. Reparent carefully: verify both the setup
pose and existing animations.

Regions are rigid quads with a pivot and transform. Slots can switch the active
attachment, tint it, select a blend mode, or hide it. Setup draw order is the slot
order; animated draw-order keys apply offsets at a time.
