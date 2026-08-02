# ADR 002: Affine2 transform math (no Mat4 decompose in hot path)

- **Status:** Accepted
- **Date:** 2026-07-30
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §2.2, §1.2 (defect D2)

## Context

The original `core` composed 4×4 matrices (`Mat4`) for world transforms and then
decomposed them via `to_scale_rotation_translation()` (defect D2). This throws
away **shear** and produces wrong results under non-uniform parent scale +
child rotation — a well-known failure of the quaternion-decompose approach in
2D skeletal animation. Spine and similar tools use an explicit 2×3 affine model
with inherit-rotation / inherit-scale flags instead.

## Decision

Introduce an `Affine2 { a, b, c, d, tx, ty }` 2×3 affine matrix in
`core/src/transforms.rs`. The world pass composes affines directly:

```
world = parent.affine * compose(local)   // compose applies rot, scale, shear directly
```

- No `Mat4`, no `Quat`, no `to_scale_rotation_translation` in the hot path.
- `decompose() -> Transform` exists **once**, used only by editor gizmos and
  world→local conversions, with tests for non-uniform scale × rotation × shear.
- `Bone.inherit: Inherit { rotation, scale, reflect }` flags are honored during
  world computation (the Spine inheritance model).

### Angle unit (settled in T-102)

`core::math::Transform::rotation` and `shear` are **radians**, everywhere in
`core`. Degrees appear only at two boundaries:

- **Serialization** — `.ankh` animation keys are degrees (PLAN §2.7); the
  `formats` crate converts on read/write (T-108).
- **Editor widgets** — inspector spinboxes show degrees and convert on edit.

Rationale: every trig call in the world pass and the IK solver wants radians, so
storing degrees in `core` would mean a conversion per bone per frame in the hot
path, plus a second unit for `shear` (which has no degree-based precedent in the
format). The PLAN's "degrees at the document level" is satisfied by the
serialized document being in degrees, which is what a human hand-editing an
`.ankh` file sees.

## Alternatives considered

- **Keep Mat4, fix decompose:** fundamentally lossy for the shear + non-uniform
  case. Rejected.
- **2×3 as `[f32; 6]`:** struct of named fields is clearer and the same size.

## Consequences

- World transforms are correct under shear and non-uniform parent scale.
- One decompose implementation to test, instead of decompose-on-every-bone.
- `MeshAttachment` inverse binds move from `Mat4` to `Affine2`.
- Property tests must verify compose→decompose→compose stability (ε<1e-4).
