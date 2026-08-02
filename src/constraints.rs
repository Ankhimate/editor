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

fn default_stretch_limit() -> f32 {
    1.1
}

/// An inverse-kinematics constraint over a chain of any length.
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
    /// so it does not snap straight the instant the target leaves its range.
    /// World units; `0` disables it (T-504).
    #[serde(default)]
    pub softness: f32,
    /// Allow the chain to lengthen to reach a target beyond its natural reach.
    #[serde(default)]
    pub stretch: bool,
    /// Most a stretching chain may grow, as a factor of its natural length.
    ///
    /// Uncapped stretch turns an out-of-range target into a rubber band, so
    /// this defaults to a tenth rather than to infinity (T-504).
    #[serde(default = "default_stretch_limit")]
    pub stretch_limit: f32,
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
            stretch_limit: default_stretch_limit(),
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
            stretch_limit: default_stretch_limit(),
        }
    }
}

/// Drive a set of bones from another bone's transform (T-501).
///
/// The general "follow that" constraint: a head that tracks a look-at bone, a
/// wheel that mirrors a drive shaft, a group of bones that inherit a master's
/// scale. IK asks *where should this chain point*; this asks *what should this
/// bone's transform be*, channel by channel, each with its own mix.
///
/// # Absolute vs relative
///
/// `relative: false` — the constrained bone is driven **toward** the target's
/// transform plus `offsets`. Mix 1 means "become the target".
///
/// `relative: true` — the target's transform is **added to** whatever the bone
/// already has. Mix 1 means "move by as much as the target moved". This is what
/// makes a constraint composable with an animation instead of overwriting it.
///
/// # Local vs world
///
/// `local: false` compares world transforms, which is what "point at the same
/// direction as that bone" means when the two live under different parents.
/// `local: true` compares the bones' own local transforms, so a constrained bone
/// copies the target's *relationship to its parent* rather than its absolute
/// placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformConstraint {
    pub name: String,
    /// The bone being followed.
    pub target: BoneId,
    /// Bones driven by it.
    pub bones: Vec<BoneId>,
    /// Added to the target's transform before mixing — the "offset" that lets a
    /// head track a target while staying tilted 10° from it.
    pub offsets: crate::math::Transform,
    pub mix_rotate: f32,
    pub mix_translate: f32,
    pub mix_scale: f32,
    pub mix_shear: f32,
    /// Compare local transforms instead of world ones.
    #[serde(default)]
    pub local: bool,
    /// Add the target's transform to the bone's own rather than replacing it.
    #[serde(default)]
    pub relative: bool,
}

impl TransformConstraint {
    /// A constraint that copies rotation only — the common "look at" case.
    pub fn rotation_only(name: impl Into<String>, target: BoneId, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            target,
            bones,
            offsets: crate::math::Transform::default(),
            mix_rotate: 1.0,
            mix_translate: 0.0,
            mix_scale: 0.0,
            mix_shear: 0.0,
            local: false,
            relative: false,
        }
    }

    /// Does any channel do anything?
    pub fn has_effect(&self) -> bool {
        !self.bones.is_empty()
            && (self.mix_rotate != 0.0
                || self.mix_translate != 0.0
                || self.mix_scale != 0.0
                || self.mix_shear != 0.0)
    }
}

/// Sway, bounce and jiggle for a bone that should follow its parent loosely
/// (T-503): hair, tails, cloth, chains, antennae.
///
/// Unlike every other constraint this one is **stateful** — where the bone lands
/// depends on where it was last frame — so the simulation lives in a
/// caller-owned [`crate::physics::PhysicsState`] rather than in the document or
/// in `evaluate`. ADR 0007 has the reasoning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicsConstraint {
    pub name: String,
    /// The bone that sways.
    pub bone: BoneId,
    /// How much the bone resists following its parent, `0`..`1`. Higher lags
    /// more, which is what reads as weight.
    pub inertia: f32,
    /// How hard it is pulled back toward its rest pose.
    pub strength: f32,
    /// How quickly motion bleeds off, `0`..`1`. At `0` it never settles.
    pub damping: f32,
    /// Scales the whole response; heavier bones move less for the same push.
    pub mass: f32,
    /// Constant world-space push, for a breeze.
    pub wind: Vec2,
    /// Constant world-space pull. Negative Y is down in a Y-up world.
    pub gravity: Vec2,
    /// `0` (constraint off) to `1` (fully simulated).
    pub mix: f32,
    /// Which channels the simulation drives.
    #[serde(default = "yes")]
    pub rotate: bool,
    #[serde(default)]
    pub translate: bool,
}

fn yes() -> bool {
    true
}

impl PhysicsConstraint {
    /// A sway with sensible defaults: rotation only, settles in about a second.
    pub fn sway(name: impl Into<String>, bone: BoneId) -> Self {
        Self {
            name: name.into(),
            bone,
            inertia: 0.5,
            strength: 40.0,
            damping: 0.5,
            mass: 1.0,
            wind: Vec2::ZERO,
            gravity: Vec2::ZERO,
            mix: 1.0,
            rotate: true,
            translate: false,
        }
    }
}

/// One constraint of any kind. Post-v1 variants (path, physics) slot in here
/// without touching the application order or the `Pose` contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Constraint {
    Ik(IkConstraint),
    Transform(TransformConstraint),
    Physics(PhysicsConstraint),
}

impl Constraint {
    pub fn name(&self) -> &str {
        match self {
            Constraint::Ik(ik) => &ik.name,
            Constraint::Transform(tc) => &tc.name,
            Constraint::Physics(p) => &p.name,
        }
    }

    /// Bones whose local transform this constraint may write to.
    pub fn affected_bones(&self) -> &[BoneId] {
        match self {
            Constraint::Ik(ik) => &ik.bones,
            Constraint::Transform(tc) => &tc.bones,
            // One bone, but the signature is a slice: borrowing a field as a
            // one-element slice needs the field to *be* one, so it is stored as
            // an array.
            Constraint::Physics(p) => std::slice::from_ref(&p.bone),
        }
    }

    /// `true` when the constraint has no effect and can be skipped entirely.
    pub fn is_inert(&self) -> bool {
        match self {
            Constraint::Ik(ik) => ik.mix <= 0.0 || ik.bones.is_empty(),
            Constraint::Transform(tc) => !tc.has_effect(),
            Constraint::Physics(p) => p.mix <= 0.0 || (!p.rotate && !p.translate),
        }
    }
}

/// How far a chain may be pulled toward a target before `softness` stops easing.
///
/// Softness eases the *last* stretch of reach so a chain does not snap straight
/// the instant the target leaves its range. Expressed as a distance in world
/// units, subtracted from the effective reach and eased back in.
pub fn soften_target(root_pos: Vec2, target_pos: Vec2, reach: f32, softness: f32) -> Vec2 {
    if softness <= 0.0 {
        return target_pos;
    }
    let diff = target_pos - root_pos;
    let distance = diff.length();
    // Only the approach to full extension is softened; a target comfortably
    // inside the chain's range is reached exactly.
    let soft_start = (reach - softness).max(0.0);
    if distance <= soft_start || distance <= 1e-6 {
        return target_pos;
    }
    // Beyond `soft_start` the remaining distance is compressed asymptotically
    // toward `reach`: an exponential approach, so the chain never quite locks
    // out and the derivative stays continuous at the handover.
    let over = distance - soft_start;
    let remaining = (reach - soft_start).max(1e-6);
    let eased = remaining * (1.0 - (-over / remaining).exp());
    root_pos + diff / distance * (soft_start + eased)
}

/// How far a chain must stretch to reach a target beyond its natural length.
///
/// Returns a factor to scale bone lengths by — `1.0` when the target is within
/// reach. Capped at `limit` (e.g. `1.1` for "10% longer at most"), because an
/// uncapped stretch turns an out-of-range target into a rubber band.
pub fn stretch_factor(root_pos: Vec2, target_pos: Vec2, reach: f32, limit: f32) -> f32 {
    if reach <= 1e-6 {
        return 1.0;
    }
    let distance = (target_pos - root_pos).length();
    if distance <= reach {
        return 1.0;
    }
    (distance / reach).min(limit.max(1.0))
}

/// Iterations FABRIK may run before giving up on a chain.
///
/// FABRIK converges quickly — a 3-bone chain is usually within tolerance in
/// under five passes — and the loop also exits early once it is close enough.
/// The cap only bounds the pathological case (a target the chain cannot reach
/// while its root is pinned), where extra passes buy nothing.
pub const FABRIK_ITERATIONS: usize = 12;
/// How close to the target counts as solved, in world units.
pub const FABRIK_TOLERANCE: f32 = 0.01;

/// Solve an N-bone chain with FABRIK (Forward And Backward Reaching Inverse
/// Kinematics).
///
/// `joints` are the chain's joint positions, root first, with one more entry
/// than there are bones — the last is the end effector. `lengths[i]` is the
/// distance from `joints[i]` to `joints[i + 1]`.
///
/// Returns the solved joint positions. The root is pinned: a chain whose base
/// wanders is not a skeleton.
///
/// # Why FABRIK and not CCD or Jacobian
///
/// FABRIK is positional rather than angular: each pass just moves points along
/// lines, so there are no trigonometric singularities, no matrix inversion, and
/// no gimbal-adjacent failure at full extension — the cases where a CCD chain
/// visibly judders. It is also deterministic and allocation-free here, which
/// `evaluate`'s contract requires (PLAN §2.6).
pub fn solve_fabrik(joints: &[Vec2], lengths: &[f32], target: Vec2) -> Vec<Vec2> {
    let mut points = joints.to_vec();
    if points.len() < 2 || lengths.len() + 1 != points.len() {
        return points;
    }
    let root = points[0];
    let total: f32 = lengths.iter().sum();

    // Out of reach: there is one answer and it is exact — point straight at the
    // target. Iterating would only approach it from below.
    if (target - root).length() >= total {
        let direction = (target - root).normalize_or_zero();
        for i in 1..points.len() {
            points[i] = points[i - 1] + direction * lengths[i - 1];
        }
        return points;
    }

    for _ in 0..FABRIK_ITERATIONS {
        if (points[points.len() - 1] - target).length() <= FABRIK_TOLERANCE {
            break;
        }
        // Backward: pull the end effector onto the target, then walk to the root
        // keeping every bone its own length.
        let last = points.len() - 1;
        points[last] = target;
        for i in (0..last).rev() {
            let direction = (points[i] - points[i + 1]).normalize_or_zero();
            points[i] = points[i + 1] + direction * lengths[i];
        }
        // Forward: put the root back where it belongs and walk out again.
        points[0] = root;
        for i in 1..points.len() {
            let direction = (points[i] - points[i - 1]).normalize_or_zero();
            points[i] = points[i - 1] + direction * lengths[i - 1];
        }
    }
    points
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
