---
title: Meshes, skins, and constraints
description: Bend artwork with meshes and weights, organize variants with skins, and control motion artistically.
---

# Meshes, skins, and constraints

Convert a region to a mesh when rigid movement cannot follow the artwork. Edit
vertices and UVs, triangulate the surface, then bind it to bones. Weight painting
changes how much each influence contributes; radius and feather shape the brush,
blend modes control the edit, and influence locks protect finished weights.

Common failures are diagnostic: a hard crease usually means abrupt neighboring
weights; swimming texture points to incorrect UVs; an inside-out triangle points
to topology or winding; motion around the wrong place points to the bind pose.
Deform keys are offsets, not replacement vertices.

Skins map a slot/attachment name to a concrete attachment. Skin bones and skin
constraints let a skin activate supporting structure. Missing entries fall back
through the documented attachment-resolution rules; inspect the load report when
a named reference cannot resolve.

## Constraints by artistic purpose

- **IK** keeps a chain aimed at or reaching a target. Use bend direction,
  softness, stretch limit, mix, and per-bone stiffness to shape the solution.
- **Transform** copies selected rotation, translation, scale, or shear components;
  local/world and absolute/relative change the reference frame and interpretation.
- **Path** arranges bones along an attachment path with position, spacing, and
  rotation controls. Constant-distance spacing behaves differently from index spacing.
- **Physics** adds delayed secondary motion. Inertia preserves motion; strength
  pulls home; damping removes energy; mass changes response; wind and gravity add forces.

Constraint order is observable. When two constraints affect the same subtree,
moving one earlier can change the result.
