//! Constraints applied after the FK pass, in an explicit order (PLAN §2.5).
//!
//! The v1 set is IK only, but the [`Constraint`] enum and the ordered
//! `constraint_order` application exist now so transform / path / physics
//! constraints are additive later rather than a refactor.
//!
//! # Blending (defect D3)
//!
//! The solver reports **world** angles. Applying those directly to world
//! transforms is what broke before: children of a solved bone did not follow,
//! and a `mix` across the ±π boundary flipped the bone the long way round.
//! Instead each solved angle is converted to a **local** rotation delta and
//! blended shortest-arc:
//!
//! ```text
//! local_rot += wrap_angle(solved_world - current_world) * mix
//! ```
//!
//! then the chain and all its descendants are re-run through FK.

use crate::ids::BoneId;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// An inverse-kinematics constraint over a 1- or 2-bone chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IkConstraint {
    pub name: String,
    /// The bone whose world position the chain reaches for.
    pub target: BoneId,
    /// Bones in the chain, root first. Length 1 = aim, length 2 = two-bone IK.
    pub bones: Vec<BoneId>,
    /// `1.0` for positive bend, `-1.0` for negative bend.
    pub bend_direction: f32,
    /// `0.0` (pure FK) to `1.0` (pure IK).
    pub mix: f32,
    /// Distance over which the chain eases off as it approaches full extension,
    /// to avoid the sudden "snap straight" at the reach limit.
    ///
    /// Serialized so files round-trip; **not yet implemented** in the solver.
    #[serde(default)]
    pub softness: f32,
    /// Allow the chain to scale beyond its natural length to reach the target.
    ///
    /// Serialized so files round-trip; **not yet implemented** in the solver.
    #[serde(default)]
    pub stretch: bool,
}

impl IkConstraint {
    /// A two-bone constraint with default softness/stretch.
    pub fn two_bone(name: impl Into<String>, target: BoneId, chain: [BoneId; 2]) -> Self {
        Self {
            name: name.into(),
            target,
            bones: chain.to_vec(),
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
        }
    }

    /// A single-bone "aim at target" constraint.
    pub fn aim(name: impl Into<String>, target: BoneId, bone: BoneId) -> Self {
        Self {
            name: name.into(),
            target,
            bones: vec![bone],
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
        }
    }
}

/// One constraint of any kind. Post-v1 variants (transform, path, physics) slot
/// in here without touching the application order or the `Pose` contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Constraint {
    Ik(IkConstraint),
}

impl Constraint {
    pub fn name(&self) -> &str {
        match self {
            Constraint::Ik(ik) => &ik.name,
        }
    }

    /// Bones whose local transform this constraint may write to.
    pub fn affected_bones(&self) -> &[BoneId] {
        match self {
            Constraint::Ik(ik) => &ik.bones,
        }
    }

    /// `true` when the constraint has no effect and can be skipped entirely.
    pub fn is_inert(&self) -> bool {
        match self {
            Constraint::Ik(ik) => ik.mix <= 0.0 || ik.bones.is_empty(),
        }
    }
}

/// Aim a single bone at a target.
///
/// Returns the world angle the bone should have so its X axis points from
/// `root_pos` at `target_pos`, or `None` when the target coincides with the
/// root (no defined direction).
pub fn solve_aim(root_pos: Vec2, target_pos: Vec2) -> Option<f32> {
    let diff = target_pos - root_pos;
    if diff.length_squared() < 1e-12 {
        return None;
    }
    Some(diff.y.atan2(diff.x))
}

/// Solve a two-bone IK chain.
///
/// Returns the **world-space** angles (radians) for the parent and child bones.
/// `l1` / `l2` are the world-space lengths of the two bones.
pub fn solve_two_bone_ik(
    root_pos: Vec2,
    target_pos: Vec2,
    l1: f32,
    l2: f32,
    bend_dir: f32,
) -> (f32, f32) {
    let diff = target_pos - root_pos;
    let dist_sq = diff.length_squared();
    let l1_sq = l1 * l1;
    let l2_sq = l2 * l2;

    let total_len = l1 + l2;
    if dist_sq >= total_len * total_len - 1e-5 {
        // Unreachable: fully extend towards the target.
        let angle = diff.y.atan2(diff.x);
        return (angle, angle);
    }

    let min_len = (l1 - l2).abs();
    if dist_sq <= min_len * min_len + 1e-5 {
        // Target is too close: fold the child bone completely backwards.
        let angle = diff.y.atan2(diff.x);
        return (angle, angle + std::f32::consts::PI);
    }

    let dist = dist_sq.sqrt();

    // Angle between l1 and the target vector.
    let cos_alpha = (l1_sq + dist_sq - l2_sq) / (2.0 * l1 * dist);
    let alpha = cos_alpha.clamp(-1.0, 1.0).acos();

    // Interior angle between l1 and l2.
    let cos_beta = (dist_sq - l1_sq - l2_sq) / (2.0 * l1 * l2);
    let beta = cos_beta.clamp(-1.0, 1.0).acos();

    let gamma = diff.y.atan2(diff.x);

    let parent_angle = gamma - bend_dir * alpha;
    let child_angle = parent_angle + bend_dir * beta;

    (parent_angle, child_angle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn two_bone_reachable() {
        // Root at (0,0), target at (1,1), both bones length 1.
        let (p, c) = solve_two_bone_ik(Vec2::ZERO, Vec2::new(1.0, 1.0), 1.0, 1.0, 1.0);
        assert!((p - 0.0).abs() < 1e-4, "parent {p}");
        assert!((c - FRAC_PI_2).abs() < 1e-4, "child {c}");

        // Negative bend mirrors the elbow.
        let (p, c) = solve_two_bone_ik(Vec2::ZERO, Vec2::new(1.0, 1.0), 1.0, 1.0, -1.0);
        assert!((p - FRAC_PI_2).abs() < 1e-4, "parent {p}");
        assert!((c - 0.0).abs() < 1e-4, "child {c}");
    }

    #[test]
    fn two_bone_unreachable_extends_straight() {
        let (p, c) = solve_two_bone_ik(Vec2::ZERO, Vec2::new(0.0, 3.0), 1.0, 1.0, 1.0);
        assert!((p - FRAC_PI_2).abs() < 1e-4, "parent {p}");
        assert!((c - FRAC_PI_2).abs() < 1e-4, "child {c}");
    }

    #[test]
    fn two_bone_too_close_folds() {
        // Target well inside the minimum reach of a (2, 1) chain.
        let (p, c) = solve_two_bone_ik(Vec2::ZERO, Vec2::new(0.2, 0.0), 2.0, 1.0, 1.0);
        assert!(p.abs() < 1e-4, "parent {p}");
        // Child folds back on itself.
        assert!(
            (crate::transforms::wrap_angle(c - p).abs() - PI).abs() < 1e-4,
            "child should fold: {c}"
        );
    }

    #[test]
    fn aim_points_at_target() {
        assert!((solve_aim(Vec2::ZERO, Vec2::new(1.0, 0.0)).unwrap() - 0.0).abs() < 1e-4);
        assert!((solve_aim(Vec2::ZERO, Vec2::new(0.0, 1.0)).unwrap() - FRAC_PI_2).abs() < 1e-4);
        assert!((solve_aim(Vec2::new(5.0, 5.0), Vec2::new(6.0, 5.0)).unwrap() - 0.0).abs() < 1e-4);
    }

    #[test]
    fn aim_rejects_degenerate_target() {
        assert!(solve_aim(Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0)).is_none());
    }

    #[test]
    fn inert_constraints_are_detected() {
        let b = BoneId::default();
        let mut ik = IkConstraint::two_bone("ik", b, [b, b]);
        assert!(!Constraint::Ik(ik.clone()).is_inert());
        ik.mix = 0.0;
        assert!(Constraint::Ik(ik.clone()).is_inert());
        ik.mix = 1.0;
        ik.bones.clear();
        assert!(Constraint::Ik(ik).is_inert());
    }

    #[test]
    fn softness_and_stretch_default_to_off() {
        let b = BoneId::default();
        let ik = IkConstraint::two_bone("ik", b, [b, b]);
        assert_eq!(ik.softness, 0.0);
        assert!(!ik.stretch);
    }
}
