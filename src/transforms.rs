//! 2D affine transform math — the only transform pipeline in `core`.
//!
//! See ADR 0002 and PLAN §2.2. No `Mat4`, no `Quat`, no
//! `to_scale_rotation_translation` anywhere: those lose shear and produce wrong
//! results for a rotated child under a non-uniformly scaled parent (defect D2).
//!
//! # Angle unit
//!
//! [`Transform::rotation`] and every angle in this module are **radians**.
//! Degrees exist only at the serialization boundary (`.ankh` keys are degrees,
//! PLAN §2.7) and in editor widgets; both convert when they cross into core.
//!
//! # Column convention
//!
//! `Affine2` is the 2×3 matrix
//!
//! ```text
//! | a  c  tx |
//! | b  d  ty |
//! ```
//!
//! i.e. `(a, b)` is the image of the local X axis and `(c, d)` the image of the
//! local Y axis — column-major, same convention as `glam::Mat2`. Points
//! transform as `p' = M * p`, and `parent.mul(child)` means "apply child first,
//! then parent", so a world pass is `world = parent_world * compose(local)`.

use crate::math::Transform;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Inheritance flags for a bone's world transform (the Spine model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inherit {
    /// Inherit the parent's rotation. When `false` the bone keeps its own
    /// world rotation regardless of how the parent is rotated.
    pub rotation: bool,
    /// Inherit the parent's scale (and shear).
    pub scale: bool,
    /// Inherit the parent's reflection (negative determinant). Only meaningful
    /// when [`Self::rotation`] is `false`; it decides whether a bone that
    /// ignores parent rotation still flips when the parent is mirrored.
    pub reflect: bool,
}

impl Default for Inherit {
    fn default() -> Self {
        Self {
            rotation: true,
            scale: true,
            reflect: true,
        }
    }
}

/// A 2×3 affine matrix. See the module docs for the column convention.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affine2 {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Affine2 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine2 {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// Build the affine of a local [`Transform`]: translate ∘ rotate ∘ shear ∘ scale.
    ///
    /// Shear follows the Spine convention, which is what riggers and every
    /// Spine-shaped import expect: **`shear.x` is an extra rotation of the
    /// X axis, `shear.y` an extra rotation of the Y axis** (both radians, on top
    /// of `rotation`). A bone with `shear.y = 20°` therefore has its Y axis 110°
    /// from its X axis — the classic italic skew — and `shear.x` alone is
    /// indistinguishable from rotation, exactly as in Spine.
    ///
    /// Scale is applied along each axis after shear, so `scale` stays the length
    /// of each axis image.
    pub fn compose(t: &Transform) -> Self {
        // Axis directions after rotation + shear. X axis: `rotation + shear.x`.
        // Y axis: `rotation + FRAC_PI_2 + shear.y`.
        let (sin_x, cos_x) = (t.rotation + t.shear.x).sin_cos();
        let (sin_y, cos_y) = (t.rotation + std::f32::consts::FRAC_PI_2 + t.shear.y).sin_cos();

        Self {
            a: cos_x * t.scale.x,
            b: sin_x * t.scale.x,
            c: cos_y * t.scale.y,
            d: sin_y * t.scale.y,
            tx: t.position.x,
            ty: t.position.y,
        }
    }

    /// `self * rhs` — apply `rhs` first, then `self`.
    pub fn mul(&self, rhs: &Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    /// Transform a point (translation applied).
    pub fn transform_point(&self, p: Vec2) -> Vec2 {
        Vec2::new(
            self.a * p.x + self.c * p.y + self.tx,
            self.b * p.x + self.d * p.y + self.ty,
        )
    }

    /// Transform a direction (translation ignored).
    pub fn transform_vector(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// Matrix inverse, or `None` when the linear part is singular (zero scale
    /// on an axis).
    pub fn invert(&self) -> Option<Self> {
        let det = self.determinant();
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let (a, b, c, d) = (self.d * inv, -self.b * inv, -self.c * inv, self.a * inv);
        Some(Self {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }

    /// Recover a [`Transform`] from this affine.
    ///
    /// **Not for the hot path** (ADR 0002) — this exists for editor gizmos and
    /// world→local conversions (e.g. reparenting without moving a bone). It is
    /// the exact inverse of [`Self::compose`] for any non-degenerate affine,
    /// including non-uniform scale combined with rotation and shear.
    ///
    /// A negative determinant (mirrored transform) is reported as a negative
    /// `scale.y`, matching the convention `compose` reproduces.
    pub fn decompose(&self) -> Transform {
        let x_axis = Vec2::new(self.a, self.b);
        let y_axis = Vec2::new(self.c, self.d);

        let rotation = x_axis.y.atan2(x_axis.x);
        let scale_x = x_axis.length();

        // Signed Y scale: positive when the Y axis is counter-clockwise from X
        // (determinant > 0), negative for a mirrored transform.
        let det = self.determinant();
        let scale_y = y_axis.length() * if det < 0.0 { -1.0 } else { 1.0 };

        // shear.y is how far the Y axis deviates from perpendicular-to-X.
        let y_angle = if scale_y < 0.0 {
            (-y_axis.y).atan2(-y_axis.x)
        } else {
            y_axis.y.atan2(y_axis.x)
        };
        let shear_y = wrap_angle(y_angle - rotation - std::f32::consts::FRAC_PI_2);

        Transform {
            position: Vec2::new(self.tx, self.ty),
            rotation,
            scale: Vec2::new(scale_x, scale_y),
            // `compose` folds shear.x into the X-axis angle, which decompose
            // reports as `rotation`; the two are indistinguishable in an affine,
            // so the canonical form puts all X-axis rotation in `rotation` and
            // leaves `shear.x` at zero (same canonicalization Spine uses).
            shear: Vec2::new(0.0, shear_y),
        }
    }

    /// World affine of a child given its parent's world affine, honoring
    /// `inherit`.
    ///
    /// * `rotation && scale` — plain `parent * compose(local)`.
    /// * `!scale` — the parent's scale/shear is stripped: only its rotation
    ///   (and translation) propagate.
    /// * `!rotation` — the parent's rotation is stripped: the child keeps its
    ///   own world rotation but stays attached to the parent's origin, and
    ///   inherits scale when `scale` is set.
    /// * `!reflect` — a mirrored parent does not flip the child.
    pub fn compose_child(parent: &Self, local: &Transform, inherit: &Inherit) -> Self {
        let local_affine = Self::compose(local);

        if inherit.rotation && inherit.scale && inherit.reflect {
            return parent.mul(&local_affine);
        }

        let origin = parent.transform_point(local.position);
        let parent_decomposed = parent.decompose();

        let mut effective = Transform {
            position: Vec2::ZERO,
            rotation: if inherit.rotation {
                parent_decomposed.rotation
            } else {
                0.0
            },
            scale: if inherit.scale {
                parent_decomposed.scale
            } else {
                Vec2::ONE
            },
            shear: if inherit.scale {
                parent_decomposed.shear
            } else {
                Vec2::ZERO
            },
        };

        if !inherit.reflect && effective.scale.y < 0.0 {
            effective.scale.y = -effective.scale.y;
        }

        let mut world = Self::compose(&effective).mul(&local_affine);
        world.tx = origin.x;
        world.ty = origin.y;
        world
    }
}

/// Wrap an angle (radians) into `(-π, π]`.
pub fn wrap_angle(mut angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    angle = (angle + PI).rem_euclid(TAU) - PI;
    if angle <= -PI { angle + TAU } else { angle }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    const EPS: f32 = 1e-4;

    fn assert_transform_eq(got: &Transform, want: &Transform) {
        assert!(
            (got.position - want.position).length() < EPS,
            "position {:?} != {:?}",
            got.position,
            want.position
        );
        assert!(
            wrap_angle(got.rotation - want.rotation).abs() < EPS,
            "rotation {} != {}",
            got.rotation,
            want.rotation
        );
        assert!(
            (got.scale - want.scale).length() < EPS,
            "scale {:?} != {:?}",
            got.scale,
            want.scale
        );
        assert!(
            (got.shear - want.shear).length() < EPS,
            "shear {:?} != {:?}",
            got.shear,
            want.shear
        );
    }

    fn assert_affine_eq(got: &Affine2, want: &Affine2) {
        for (g, w) in [
            (got.a, want.a),
            (got.b, want.b),
            (got.c, want.c),
            (got.d, want.d),
            (got.tx, want.tx),
            (got.ty, want.ty),
        ] {
            assert!((g - w).abs() < EPS, "{got:?} != {want:?}");
        }
    }

    /// Shear follows Spine: `shear.x` rotates the X axis, `shear.y` the Y axis.
    ///
    /// Regression — these were swapped, so an italic skew (`shear.y`) came out
    /// as a near-flip of the Y axis instead, and a rig authored in Spine
    /// rendered wrong.
    #[test]
    fn shear_axes_follow_the_spine_convention() {
        // shear.y = 20°: X axis untouched, Y axis 110° from +X.
        let t = Transform {
            shear: Vec2::new(0.0, 20.0_f32.to_radians()),
            ..Transform::default()
        };
        let m = Affine2::compose(&t);
        let x_angle = m.b.atan2(m.a).to_degrees();
        let y_angle = m.d.atan2(m.c).to_degrees();
        assert!(x_angle.abs() < 0.01, "shear.y must not tilt X: {x_angle}");
        assert!((y_angle - 110.0).abs() < 0.01, "Y axis at 110°: {y_angle}");

        // shear.x = 20°: X axis tilts, Y axis stays at 90°.
        let t = Transform {
            shear: Vec2::new(20.0_f32.to_radians(), 0.0),
            ..Transform::default()
        };
        let m = Affine2::compose(&t);
        let x_angle = m.b.atan2(m.a).to_degrees();
        let y_angle = m.d.atan2(m.c).to_degrees();
        assert!((x_angle - 20.0).abs() < 0.01, "X axis at 20°: {x_angle}");
        assert!(
            (y_angle - 90.0).abs() < 0.01,
            "Y axis stays at 90°: {y_angle}"
        );
    }

    /// The reported case: shear (-175.667°, 2.659°) must produce the same axes
    /// Spine produces — a near-mirrored X axis and an almost-upright Y axis.
    #[test]
    fn reported_shear_matches_spine_axes() {
        let t = Transform {
            shear: Vec2::new((-175.667_f32).to_radians(), 2.659_f32.to_radians()),
            ..Transform::default()
        };
        let m = Affine2::compose(&t);
        let x_angle = m.b.atan2(m.a).to_degrees();
        let y_angle = m.d.atan2(m.c).to_degrees();
        assert!((x_angle + 175.667).abs() < 0.01, "X axis: {x_angle}");
        assert!((y_angle - 92.659).abs() < 0.01, "Y axis: {y_angle}");
    }

    /// Deterministic pseudo-random sequence — `core` must not use global state
    /// or `std::time` (PLAN §2.6), so no `rand`.
    fn lcg(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1 << 24) as f32 // [0, 1)
    }

    #[test]
    fn identity_compose_and_transform() {
        let t = Transform::default();
        assert_affine_eq(&Affine2::compose(&t), &Affine2::IDENTITY);
        let p = Vec2::new(3.0, -7.0);
        assert!((Affine2::IDENTITY.transform_point(p) - p).length() < EPS);
    }

    #[test]
    fn compose_decompose_roundtrip_is_stable() {
        // Property test over non-uniform scale + rotation + shear (PLAN R2).
        let mut seed = 0xC0FF_EE01_u32;
        for _ in 0..500 {
            let t = Transform {
                position: Vec2::new(
                    lcg(&mut seed) * 200.0 - 100.0,
                    lcg(&mut seed) * 200.0 - 100.0,
                ),
                rotation: lcg(&mut seed) * std::f32::consts::TAU - std::f32::consts::PI,
                // Keep scales away from 0 so the affine stays non-degenerate.
                scale: Vec2::new(lcg(&mut seed) * 3.0 + 0.25, lcg(&mut seed) * 3.0 + 0.25),
                // Shear below ±80° so the axes never become parallel.
                shear: Vec2::new(lcg(&mut seed) * 2.8 - 1.4, 0.0),
            };

            let m1 = Affine2::compose(&t);
            let d = m1.decompose();
            let m2 = Affine2::compose(&d);
            // compose∘decompose∘compose must be stable even where `t` itself is
            // not the canonical representative (shear.y folds into rotation).
            assert_affine_eq(&m2, &m1);
            assert_transform_eq(&m2.decompose(), &d);
        }
    }

    #[test]
    fn decompose_recovers_non_uniform_scale_and_rotation() {
        let t = Transform {
            position: Vec2::new(10.0, -4.0),
            rotation: FRAC_PI_2,
            scale: Vec2::new(2.0, 0.5),
            shear: Vec2::ZERO,
        };
        assert_transform_eq(&Affine2::compose(&t).decompose(), &t);
    }

    #[test]
    fn shear_x_folds_into_rotation() {
        // shear.x rotates the X axis only, which is indistinguishable from
        // `rotation` in an affine; the canonical decomposition reports it as
        // rotation with a compensating shear.y (Spine does the same).
        let t = Transform {
            position: Vec2::ZERO,
            rotation: 0.0,
            scale: Vec2::ONE,
            shear: Vec2::new(0.3, 0.0),
        };
        let d = Affine2::compose(&t).decompose();
        assert!((d.rotation - 0.3).abs() < EPS, "rotation {}", d.rotation);
        assert!((d.shear.y - -0.3).abs() < EPS, "shear.y {}", d.shear.y);
        assert_affine_eq(&Affine2::compose(&d), &Affine2::compose(&t));
    }

    #[test]
    fn child_under_non_uniformly_scaled_rotated_parent() {
        // The Mat4-decompose bug (D2): parent scaled (2,1) and rotated 90°,
        // child translated (10,0) locally.
        let parent = Transform {
            position: Vec2::new(5.0, 5.0),
            rotation: FRAC_PI_2,
            scale: Vec2::new(2.0, 1.0),
            shear: Vec2::ZERO,
        };
        let child_local = Transform {
            position: Vec2::new(10.0, 0.0),
            ..Default::default()
        };

        let world = Affine2::compose(&parent).mul(&Affine2::compose(&child_local));

        // Hand computation: parent X axis is (0,2) after 90° rot × 2 scale, so a
        // local +10 X offset lands 20 units up from the parent origin.
        let pos = world.transform_point(Vec2::ZERO);
        assert!(
            (pos - Vec2::new(5.0, 25.0)).length() < EPS,
            "child world pos {pos:?}"
        );

        // The child's X axis is stretched by the parent's X scale only.
        let d = world.decompose();
        assert!((d.scale.x - 2.0).abs() < EPS, "scale.x {}", d.scale.x);
        assert!((d.scale.y - 1.0).abs() < EPS, "scale.y {}", d.scale.y);
    }

    #[test]
    fn invert_roundtrips_points() {
        let t = Transform {
            position: Vec2::new(-3.0, 8.0),
            rotation: 0.7,
            scale: Vec2::new(1.5, 0.4),
            shear: Vec2::new(0.2, 0.0),
        };
        let m = Affine2::compose(&t);
        let inv = m.invert().expect("non-degenerate");
        let p = Vec2::new(12.0, -5.0);
        let back = inv.transform_point(m.transform_point(p));
        assert!((back - p).length() < EPS, "{back:?}");
        assert_affine_eq(&m.mul(&inv), &Affine2::IDENTITY);
    }

    #[test]
    fn invert_rejects_singular() {
        let t = Transform {
            scale: Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        assert!(Affine2::compose(&t).invert().is_none());
    }

    #[test]
    fn inherit_scale_false_strips_parent_scale() {
        let parent = Transform {
            position: Vec2::new(0.0, 0.0),
            rotation: 0.0,
            scale: Vec2::new(3.0, 3.0),
            shear: Vec2::ZERO,
        };
        let local = Transform {
            position: Vec2::new(10.0, 0.0),
            ..Default::default()
        };
        let inherit = Inherit {
            rotation: true,
            scale: false,
            reflect: true,
        };

        let world = Affine2::compose_child(&Affine2::compose(&parent), &local, &inherit);
        let d = world.decompose();
        // Origin still follows the parent's scaled space...
        assert!((d.position - Vec2::new(30.0, 0.0)).length() < EPS, "{d:?}");
        // ...but the bone itself is not stretched.
        assert!((d.scale - Vec2::ONE).length() < EPS, "{:?}", d.scale);
    }

    #[test]
    fn inherit_rotation_false_keeps_world_rotation() {
        let parent = Transform {
            rotation: FRAC_PI_2,
            ..Default::default()
        };
        let local = Transform {
            position: Vec2::new(10.0, 0.0),
            rotation: 0.0,
            ..Default::default()
        };
        let inherit = Inherit {
            rotation: false,
            scale: true,
            reflect: true,
        };

        let world = Affine2::compose_child(&Affine2::compose(&parent), &local, &inherit);
        let d = world.decompose();
        // Position follows the rotated parent, rotation does not.
        assert!((d.position - Vec2::new(0.0, 10.0)).length() < EPS, "{d:?}");
        assert!(
            wrap_angle(d.rotation).abs() < EPS,
            "rotation {}",
            d.rotation
        );
    }

    #[test]
    fn inherit_reflect_false_unflips_mirrored_parent() {
        let parent = Transform {
            scale: Vec2::new(1.0, -1.0),
            ..Default::default()
        };
        let local = Transform::default();
        let mirrored = Affine2::compose_child(
            &Affine2::compose(&parent),
            &local,
            &Inherit {
                rotation: false,
                scale: true,
                reflect: true,
            },
        );
        let unmirrored = Affine2::compose_child(
            &Affine2::compose(&parent),
            &local,
            &Inherit {
                rotation: false,
                scale: true,
                reflect: false,
            },
        );
        assert!(mirrored.determinant() < 0.0);
        assert!(unmirrored.determinant() > 0.0);
    }

    #[test]
    fn wrap_angle_normalizes() {
        use std::f32::consts::{PI, TAU};
        assert!((wrap_angle(0.0) - 0.0).abs() < EPS);
        assert!((wrap_angle(TAU) - 0.0).abs() < EPS);
        assert!((wrap_angle(PI + 0.1) - (-PI + 0.1)).abs() < EPS);
        assert!((wrap_angle(-PI - 0.1) - (PI - 0.1)).abs() < EPS);
        // ±π maps to +π, not −π.
        assert!((wrap_angle(-PI) - PI).abs() < EPS);
    }
}
