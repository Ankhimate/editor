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
    /// Bones in the chain, root first. **Any length.**
    ///
    /// 1 is an aim constraint and 2 has a closed-form solution, so those two get
    /// their own constructors; 3 and beyond go through FABRIK and are just as
    /// supported. Stated explicitly because every other 2D editor caps this at
    /// two and riggers arrive assuming the cap is universal — a tentacle or a
    /// tail is one constraint here, not a hand-chained stack of them.
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
    /// How much a 3+ bone chain resists redistributing its bend. `0` spreads the
    /// curl evenly; `1` keeps the pose it is already in.
    ///
    /// The one genuine preference in a long-chain solve, and the reason it is a
    /// field rather than a constant. FABRIK converges to the solution nearest
    /// where it starts, so *where it starts* is the whole control:
    ///
    /// * At `0` the chain is seeded from a circular arc — constant curvature, so
    ///   the bend lands spread over every joint. What a tentacle, a tail or a
    ///   rope wants.
    /// * At `1` it is seeded from the pose as it stands, so the chain keeps the
    ///   shape the animator posed and only the joints that must move do. What a
    ///   hand-posed spine or a chain with authored keys wants, because a solver
    ///   that re-spreads the curl every frame quietly discards that work.
    ///
    /// Between the two it interpolates the seed, so a mostly-arc chain with a
    /// little memory of its pose is expressible.
    ///
    /// Ignored for 1- and 2-bone chains: those have exact solutions with no seed
    /// and nothing to prefer.
    ///
    /// Defaults to `0`. A rig authored before this existed was solved from an
    /// arc, so that is what keeps it looking the same.
    #[serde(default)]
    pub stiffness: f32,
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
            stiffness: 0.0,
        }
    }

    /// An IK constraint over a chain of any length.
    ///
    /// The general case, and the one worth reaching for first. `two_bone` and
    /// `aim` are the two lengths that have their own names because they have
    /// their own closed-form solvers; every other length goes through FABRIK and
    /// needs no special constructor. A chain of three or more is the case a
    /// two-bone limit cannot express at all — a tentacle, a tail, a spine that
    /// reaches — and nothing here restricts it.
    ///
    /// `bend_direction` is what makes a long chain controllable: three or more
    /// bones have infinitely many solutions for a given target, and the bend
    /// picks the side to converge from. See [`solve_fabrik`].
    pub fn chain(name: impl Into<String>, target: BoneId, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            target,
            bones,
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: default_stretch_limit(),
            stiffness: 0.0,
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
            stiffness: 0.0,
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
    /// How much of the target each channel contributes.
    pub mix: TransformMix,
    /// Compare local transforms instead of world ones.
    #[serde(default)]
    pub local: bool,
    /// Add the target's transform to the bone's own rather than replacing it.
    #[serde(default)]
    pub relative: bool,
}

/// How much of the target's transform each channel of a constraint contributes.
///
/// **Every channel mixes per axis where it has axes.** Rotation is one angle so
/// it stays a scalar; translate, scale and shear each have two, so each gets
/// two. A constraint can follow its target horizontally and ignore it
/// vertically, or take its x-scale alone.
///
/// The uniform rule is the point. Spine mixes rotate with one number, translate
/// and scale with two, and shear with one — an asymmetry that has to be
/// memorised rather than derived, and one that leaves shear's second axis with
/// no mix at all. `Transform::shear` is a `Vec2` here, so the axis exists and
/// now so does its mix.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TransformMix {
    /// Rotation has one axis, so one number.
    pub rotate: f32,
    pub translate: glam::Vec2,
    pub scale: glam::Vec2,
    pub shear: glam::Vec2,
}

impl TransformMix {
    /// Nothing contributed on any channel — the constraint is inert.
    pub const NONE: Self = Self {
        rotate: 0.0,
        translate: glam::Vec2::ZERO,
        scale: glam::Vec2::ZERO,
        shear: glam::Vec2::ZERO,
    };

    /// Rotation fully, nothing else — the common "look at" case.
    pub const ROTATION_ONLY: Self = Self {
        rotate: 1.0,
        ..Self::NONE
    };

    /// The same amount on every channel and both axes.
    ///
    /// What a single-mix model meant, so it is also what a file written before
    /// the axes were split migrates to.
    pub fn uniform(amount: f32) -> Self {
        Self {
            rotate: amount,
            translate: glam::Vec2::splat(amount),
            scale: glam::Vec2::splat(amount),
            shear: glam::Vec2::splat(amount),
        }
    }

    /// Does any channel contribute anything?
    pub fn is_any(&self) -> bool {
        self.rotate != 0.0
            || self.translate != glam::Vec2::ZERO
            || self.scale != glam::Vec2::ZERO
            || self.shear != glam::Vec2::ZERO
    }

    /// Blend toward `other` by `alpha`, for crossfading animated mixes.
    pub fn lerp(&self, other: &Self, alpha: f32) -> Self {
        Self {
            rotate: self.rotate + (other.rotate - self.rotate) * alpha,
            translate: self.translate.lerp(other.translate, alpha),
            scale: self.scale.lerp(other.scale, alpha),
            shear: self.shear.lerp(other.shear, alpha),
        }
    }
}

impl Default for TransformMix {
    fn default() -> Self {
        Self::NONE
    }
}

impl TransformConstraint {
    /// A constraint that copies rotation only — the common "look at" case.
    pub fn rotation_only(name: impl Into<String>, target: BoneId, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            target,
            bones,
            offsets: crate::math::Transform::default(),
            mix: TransformMix::ROTATION_ONLY,
            local: false,
            relative: false,
        }
    }

    /// Does any channel do anything?
    pub fn has_effect(&self) -> bool {
        !self.bones.is_empty() && self.mix.is_any()
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

/// Drive a chain of bones along a path attachment (T-502).
///
/// Tails, treads, belts, a train of carriages, vines: anything whose bones
/// should follow a curve rather than each other. Strictly more general than
/// binding mesh vertices to a spline, because the bones stay bones — they can
/// carry art, be keyed, and have their own children.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathConstraint {
    pub name: String,
    /// The slot whose attachment is the path.
    pub slot: crate::ids::SlotId,
    /// Bones driven along it, in order from the path's start.
    pub bones: Vec<BoneId>,
    /// Where the chain starts, as a fraction of the path's length.
    pub position: f32,
    /// Scales the gap between bones; `1` spreads them over the whole path.
    pub spacing: f32,
    /// How much of the path's direction each bone takes.
    pub mix_rotate: f32,
    /// How much of the path's position each bone takes.
    pub mix_translate: f32,
}

impl PathConstraint {
    pub fn new(name: impl Into<String>, slot: crate::ids::SlotId, bones: Vec<BoneId>) -> Self {
        Self {
            name: name.into(),
            slot,
            bones,
            position: 0.0,
            spacing: 1.0,
            mix_rotate: 1.0,
            mix_translate: 1.0,
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
    Path(PathConstraint),
}

impl Constraint {
    pub fn name(&self) -> &str {
        match self {
            Constraint::Ik(ik) => &ik.name,
            Constraint::Transform(tc) => &tc.name,
            Constraint::Physics(p) => &p.name,
            Constraint::Path(p) => &p.name,
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
            Constraint::Path(p) => &p.bones,
        }
    }

    /// The bone this constraint reaches for, when it has one.
    ///
    /// `None` for physics and path constraints, which are driven by a
    /// simulation and a path rather than by another bone. Needed wherever a
    /// constraint's full set of bone references matters — copying a subtree
    /// (T-909) has to know whether the target came along with it.
    pub fn target(&self) -> Option<BoneId> {
        match self {
            Constraint::Ik(ik) => Some(ik.target),
            Constraint::Transform(tc) => Some(tc.target),
            Constraint::Physics(_) | Constraint::Path(_) => None,
        }
    }

    /// `true` when the constraint has no effect and can be skipped entirely.
    pub fn is_inert(&self) -> bool {
        match self {
            Constraint::Ik(ik) => ik.mix <= 0.0 || ik.bones.is_empty(),
            Constraint::Transform(tc) => !tc.has_effect(),
            Constraint::Physics(p) => p.mix <= 0.0 || (!p.rotate && !p.translate),
            Constraint::Path(p) => {
                p.bones.is_empty() || (p.mix_rotate == 0.0 && p.mix_translate == 0.0)
            }
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

/// Enforce the requested bend side on a solved chain.
///
/// The arc seed in [`solve_fabrik`] commits the chain to a side before the first
/// iteration, so this rarely fires — it is the backstop for a chain that was
/// already bent the wrong way hard enough to converge back there. It checks the
/// side actually landed on and mirrors the **interior** joints across the
/// root→tip axis if they disagree. Reflection preserves every segment length
/// exactly, so the mirrored chain still reaches, and the result is deterministic
/// rather than dependent on how the iteration happened to go.
///
/// The endpoints are excluded rather than reflected along with everything else.
/// They define the mirror, so reflecting them is a no-op *in exact arithmetic* —
/// but FABRIK stops on a tolerance, not on equality, so the tip sits a hair off
/// the axis it helped define, and mirroring moved it by twice that. On a chain
/// solved to 0.01 that was a visible sub-unit error; the tip is now left exactly
/// where the solver put it.
pub fn enforce_bend(points: &mut [Vec2], bend_dir: f32) {
    if bend_dir == 0.0 || points.len() < 3 {
        return;
    }
    let last = points.len() - 1;
    let (root, tip) = (points[0], points[last]);
    let axis = tip - root;
    if axis.length_squared() < 1e-9 {
        return;
    }
    // The **first** joint decides the side, not the sum of all of them. A long
    // chain can have interior joints on either side of the chord while still
    // reading as bending one way, and summing them lets a later joint outvote
    // the elbow — which is the joint a rigger means by "bend this way".
    let side = axis.perp_dot(points[1] - root);
    if side == 0.0 || side.signum() == bend_dir.signum() {
        return;
    }
    let unit = axis.normalize_or_zero();
    for point in &mut points[1..last] {
        let offset = *point - root;
        let along = offset.dot(unit);
        // Reflect across the chord: keep the component along it, flip the rest.
        *point = root + unit * along - (offset - unit * along);
    }
}

/// Iterations FABRIK may run before giving up on a chain.
///
/// FABRIK converges quickly — a 3-bone chain is usually within tolerance in
/// under five passes — and the loop also exits early once it is close enough.
/// The cap only bounds the pathological case (a target the chain cannot reach
/// while its root is pinned), where extra passes buy nothing.
///
/// Raised from 12 when the arc seed landed: seeding from a distributed curve
/// rather than from the setup pose starts a near-extended chain further from its
/// answer, and the last few hundredths of a unit take passes that a straight
/// start did not need. Measured worst case over the shipped tentacle is inside
/// this with room to spare.
pub const FABRIK_ITERATIONS: usize = 48;
/// How close to the target counts as solved, in world units.
pub const FABRIK_TOLERANCE: f32 = 0.01;

/// Lay `points` along a circular arc from the root to the target.
///
/// The seed FABRIK iterates from — see the call site for why the seed decides
/// the result's shape. An arc is chosen because its curvature is constant, so
/// the bend is spread evenly across every joint rather than pooled in whichever
/// ones happen to sit near the effector.
///
/// # The arc
///
/// One arc passes through both endpoints with a given arc length: the one whose
/// half-angle `h` satisfies `sin(h)/h = chord/total`. That has no closed form,
/// so it is bisected — `sin(h)/h` decreases monotonically on `(0, π)`, which
/// makes bisection both correct and quick. Forty rounds takes `h` well below
/// what `f32` can represent, so the result is exact to the type.
///
/// A chain longer than the straight-line distance bulges; one barely longer is
/// nearly straight. Both fall out of the same solve.
fn seed_arc(points: &mut [Vec2], root: Vec2, target: Vec2, lengths: &[f32], total: f32, bend: f32) {
    let chord = (target - root).length();
    // Degenerate: target on the root, or a chain with no length. Leaving the
    // seed alone is right — there is no arc, and the caller's straight-line
    // fallbacks handle it.
    if chord < 1e-6 || total < 1e-6 {
        return;
    }
    let ratio = (chord / total).clamp(0.0, 1.0);
    let (mut lo, mut hi) = (1e-4f32, std::f32::consts::PI - 1e-4);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if mid.sin() / mid > ratio {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let half_angle = 0.5 * (lo + hi);
    let radius = total / (2.0 * half_angle);

    let direction = (target - root) / chord;
    let normal = Vec2::new(-direction.y, direction.x) * bend.signum();
    // The centre sits off the chord's midpoint by the triangle's other leg.
    // Clamped at zero because `radius` and `chord/2` can cross by a rounding
    // error on an almost-straight chain, and a NaN here would poison the pose.
    let midpoint = root + direction * (chord * 0.5);
    let offset = (radius * radius - chord * chord * 0.25).max(0.0).sqrt();
    let centre = midpoint - normal * offset;

    let start = (root - centre).y.atan2((root - centre).x);
    let sweep_sign = bend.signum();
    let mut angle = start;
    for (i, &length) in lengths.iter().enumerate() {
        // Step by the angle whose **chord** is this segment's length, not by its
        // share of the arc. Those differ — a chord is always shorter than the arc
        // it subtends — and stepping by arc length lays the joints too far apart,
        // so the seed's segments come out longer than the bones. FABRIK then
        // spends its iterations reaching the target and converges with that
        // length error still in the chain: the tip lands, and the rig is a
        // fraction of a unit longer than it should be. Chord placement seeds a
        // chain whose segments are already exactly right.
        //
        // `length / (2 * radius)` can exceed 1 for a segment longer than the
        // circle's diameter, which only happens when the arc is nearly straight;
        // clamping falls back to a half-turn there, and the iteration corrects
        // the rest.
        let step = 2.0 * (length / (2.0 * radius)).clamp(-1.0, 1.0).asin();
        angle += step * sweep_sign;
        points[i + 1] = centre + Vec2::new(angle.cos(), angle.sin()) * radius;
    }
}

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
/// As [`solve_fabrik`], with control over how much of the chain's current pose
/// the seed keeps — see [`IkConstraint::stiffness`].
///
/// `0` seeds from a pure arc (bend spread evenly), `1` seeds from the pose as
/// given (bend stays where the animator put it), and between them the two seeds
/// are mixed per joint.
pub fn solve_fabrik_stiff(
    joints: &[Vec2],
    lengths: &[f32],
    target: Vec2,
    bend_dir: f32,
    stiffness: f32,
) -> Vec<Vec2> {
    solve_inner(joints, lengths, target, bend_dir, stiffness)
}

pub fn solve_fabrik(joints: &[Vec2], lengths: &[f32], target: Vec2, bend_dir: f32) -> Vec<Vec2> {
    solve_inner(joints, lengths, target, bend_dir, 0.0)
}

fn solve_inner(
    joints: &[Vec2],
    lengths: &[f32],
    target: Vec2,
    bend_dir: f32,
    stiffness: f32,
) -> Vec<Vec2> {
    let mut points = joints.to_vec();
    if points.len() < 2 || lengths.len() + 1 != points.len() {
        return points;
    }
    let root = points[0];
    let total: f32 = lengths.iter().sum();

    // Where the chain *starts* decides the shape it ends in, because FABRIK
    // converges to the solution nearest its seed. That is the whole ballgame for
    // a long chain, and getting it wrong is not a wrong answer — it is a valid
    // one that looks broken.
    //
    // Two failures come from seeding with the pose as authored:
    //
    // * A flat, fully-extended chain (every rotation zero, which is how rigs are
    //   built) sits exactly on the boundary between bending up and bending down,
    //   so the elbow picks a side by floating-point noise and a leg can bend
    //   backwards.
    // * With slack in the chain — the target well inside reach — the backward
    //   pass satisfies the constraint using the joints nearest the effector and
    //   leaves the rest straight. An eight-bone tentacle came out as seven
    //   collinear bones and one 96° kink. Every length is right, the tip is on
    //   the target, and it looks nothing like a tentacle.
    //
    // Seeding from a circular arc that spans root to target fixes both. The arc
    // is the shape whose bend is spread evenly over its whole length, so the
    // nearest solution to it is a distributed one; and it commits to a side, so
    // there is no tie for noise to break. `bend_dir` picks which side.
    // Stiffness decides how much of the arc is used. At 1 the pose is the seed
    // untouched, so the arc is not computed at all.
    let blend = 1.0 - stiffness.clamp(0.0, 1.0);
    if bend_dir != 0.0 && points.len() > 2 && blend > 0.0 {
        let mut arc = points.clone();
        seed_arc(&mut arc, root, target, lengths, total, bend_dir);
        // Mixed per joint rather than choosing one seed or the other, so the
        // slider is continuous. The result is not itself a valid chain — a
        // lerp between two length-correct chains generally is not — but it is
        // only a starting point, and FABRIK's first forward pass restores every
        // length before anything reads it.
        for (point, arc_point) in points.iter_mut().zip(arc.iter()) {
            *point = point.lerp(*arc_point, blend);
        }
    }

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
