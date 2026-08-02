//! Simulation state for physics constraints (T-503, ADR 0007).
//!
//! Kept out of the document and out of `evaluate` because it is the one piece of
//! posing that is not a pure function of the playhead: where a hair bone lands
//! now depends on where it was a moment ago. The caller owns one of these per
//! thing being animated — one per viewport in the editor, one per instance in a
//! game — so two views of the same rig cannot interfere and an export cannot
//! inherit whatever the editor had been doing.

use crate::ids::{BoneId, ConstraintId};
use glam::Vec2;
use std::collections::HashMap;

/// Fixed integration step, in seconds.
///
/// The simulation always advances in whole steps of this size, whatever `dt` the
/// caller passes. That is what makes a 30fps export and a 144Hz viewport follow
/// the same trajectory — a variable step would make the result depend on the
/// frame rate, and an export would not match what was authored.
pub const PHYSICS_STEP: f32 = 1.0 / 120.0;

/// Most steps one `advance` may run.
///
/// A long stall — a breakpoint, a dialog, a loading hitch — would otherwise ask
/// for thousands of steps at once. Capping means a hitch under-simulates rather
/// than freezing the app; the alternative reads as a crash.
const MAX_STEPS: usize = 8;

/// Velocity decay rate at `damping = 1`, per second.
///
/// `damping` is a 0..1 dial and decay needs a rate. The number is chosen so
/// `damping = 1` is roughly *critical* for the default strength (ω ≈ 6.3 rad/s,
/// critical damping ≈ 2ω): the bone returns to rest as fast as it can without
/// overshooting, and higher values would only make the dial's top half feel
/// dead. A damped oscillator's envelope decays at half the damping coefficient,
/// so `damping = 0.5` settles as `e^(-3t)` — under a hundredth of its
/// disturbance in two seconds.
const DAMPING_RATE: f32 = 12.0;

/// One simulated bone.
#[derive(Debug, Clone, Copy, Default)]
struct BoneSim {
    /// Rotation offset from the rest pose, radians.
    rotation: f32,
    /// Its rate of change, radians per second.
    rotation_velocity: f32,
    /// Translation offset from the rest pose, local units.
    position: Vec2,
    position_velocity: Vec2,
}

/// Caller-owned physics state (ADR 0007).
#[derive(Debug, Clone, Default)]
pub struct PhysicsState {
    bones: HashMap<(ConstraintId, BoneId), BoneSim>,
    /// Where each bone's parent was last frame, so the pass can measure how far
    /// it moved without the document having to remember.
    anchors: HashMap<(ConstraintId, BoneId), Vec2>,
    /// Time left over from the last `advance`, carried so the fixed step does
    /// not drift against the wall clock.
    remainder: f32,
    /// Set while the caller wants motion frozen but state kept.
    pub paused: bool,
}

impl PhysicsState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget every simulated bone. The next frame starts from rest.
    pub fn reset(&mut self) {
        self.bones.clear();
        self.anchors.clear();
        self.remainder = 0.0;
    }

    /// Where this bone's parent was when the simulation last ran.
    pub fn last_anchor(&self, constraint: ConstraintId, bone: BoneId) -> Option<Vec2> {
        self.anchors.get(&(constraint, bone)).copied()
    }

    pub fn set_anchor(&mut self, constraint: ConstraintId, bone: BoneId, anchor: Vec2) {
        self.anchors.insert((constraint, bone), anchor);
    }

    /// The offsets to apply for one constrained bone this frame.
    pub fn offsets(&self, constraint: ConstraintId, bone: BoneId) -> (f32, Vec2) {
        self.bones
            .get(&(constraint, bone))
            .map(|s| (s.rotation, s.position))
            .unwrap_or((0.0, Vec2::ZERO))
    }

    /// Advance one constrained bone by `dt`, given the acceleration acting on it.
    ///
    /// `rest_delta` is how far the bone has been displaced from where the
    /// simulation last saw it — the parent's motion, which is what the bone is
    /// meant to lag behind. `push` is world-space force (wind plus gravity)
    /// already converted into the bone's local frame.
    ///
    /// Returns the offsets to apply.
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        constraint: ConstraintId,
        bone: BoneId,
        dt: f32,
        rest_delta: Vec2,
        push: Vec2,
        inertia: f32,
        strength: f32,
        damping: f32,
        mass: f32,
    ) -> (f32, Vec2) {
        if self.paused || dt <= 0.0 {
            return self.offsets(constraint, bone);
        }
        let mass = mass.max(0.01);
        let inertia = inertia.clamp(0.0, 1.0);
        let damping = damping.clamp(0.0, 1.0);

        // Whole fixed steps only; the remainder is carried to the next call so
        // the simulation neither drifts nor depends on the frame rate.
        let total = dt + self.remainder;
        let steps = ((total / PHYSICS_STEP).floor() as usize).min(MAX_STEPS);
        self.remainder = (total - steps as f32 * PHYSICS_STEP).max(0.0);

        if steps == 0 {
            return self.offsets(constraint, bone);
        }

        // `rest_delta` is how far the parent moved *this frame*, so it is an
        // impulse for the frame, not a per-step force: spread it across the
        // steps or a 30fps caller would push four times as hard as a 120fps one
        // over the same second.
        let impulse = -rest_delta * inertia / steps as f32;
        let decay = (-damping * DAMPING_RATE * PHYSICS_STEP).exp();
        let spring = strength / mass;

        let sim = self.bones.entry((constraint, bone)).or_default();
        for _ in 0..steps {
            let force = push / mass + impulse * spring;

            // Semi-implicit Euler: velocity first, then position from the new
            // velocity. It is stable for springs at these step sizes, where
            // explicit Euler gains energy and eventually throws the bone off.
            sim.position_velocity += (force - sim.position * spring) * PHYSICS_STEP;
            sim.position_velocity *= decay;
            sim.position += sim.position_velocity * PHYSICS_STEP;

            // Rotation swings on the same spring, driven by the sideways part of
            // the displacement — a bone dragged sideways rotates.
            let torque = push.x / mass + impulse.x * spring;
            sim.rotation_velocity += (torque - sim.rotation * spring) * PHYSICS_STEP;
            sim.rotation_velocity *= decay;
            sim.rotation += sim.rotation_velocity * PHYSICS_STEP;

            // A simulation that has gone non-finite cannot recover, and its NaN
            // would spread through every child. Reset instead.
            if !sim.rotation.is_finite() || !sim.position.is_finite() {
                *sim = BoneSim::default();
                break;
            }
        }
        (sim.rotation, sim.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{BoneId, ConstraintId};
    use slotmap::KeyData;

    fn ids() -> (ConstraintId, BoneId) {
        (
            ConstraintId::from(KeyData::from_ffi(1)),
            BoneId::from(KeyData::from_ffi(1)),
        )
    }

    /// The determinism requirement from ADR 0007: the same dt sequence must
    /// produce the same trajectory, every run.
    #[test]
    fn the_same_dt_sequence_produces_the_same_trajectory() {
        let (c, b) = ids();
        let run = || {
            let mut state = PhysicsState::new();
            let mut samples = Vec::new();
            for i in 0..60 {
                let displacement = Vec2::new(if i < 10 { 5.0 } else { 0.0 }, 0.0);
                let (rot, pos) = state.advance(
                    c,
                    b,
                    1.0 / 60.0,
                    displacement,
                    Vec2::ZERO,
                    0.5,
                    40.0,
                    0.5,
                    1.0,
                );
                samples.push((rot, pos.x, pos.y));
            }
            samples
        };
        assert_eq!(
            run(),
            run(),
            "physics is deterministic given the same input"
        );
    }

    /// The acceptance case: a disturbed bone settles back to rest, and quickly
    /// enough that a rigger does not think it is broken.
    #[test]
    fn a_disturbed_bone_settles_to_rest_within_two_seconds() {
        let (c, b) = ids();
        let mut state = PhysicsState::new();
        // A shove for the first tenth of a second, then nothing.
        for i in 0..120 {
            let displacement = if i < 6 {
                Vec2::new(10.0, 0.0)
            } else {
                Vec2::ZERO
            };
            state.advance(
                c,
                b,
                1.0 / 60.0,
                displacement,
                Vec2::ZERO,
                0.5,
                40.0,
                0.5,
                1.0,
            );
        }
        let (rotation, position) = state.offsets(c, b);
        assert!(
            rotation.abs() < 0.01,
            "rotation settled: {rotation} rad after 2s"
        );
        assert!(
            position.length() < 0.05,
            "position settled: {position:?} after 2s"
        );
    }

    /// The fixed step is what makes an export match the editor: the same elapsed
    /// time in different-sized frames must land in the same place.
    #[test]
    fn the_trajectory_does_not_depend_on_frame_size() {
        let (c, b) = ids();
        // The parent moves at a constant *velocity*, so the per-frame
        // displacement scales with the frame: 60 units/second either way.
        let sample = |frames: usize, dt: f32| {
            let mut state = PhysicsState::new();
            for _ in 0..frames {
                let moved = Vec2::new(60.0 * dt, 0.0);
                state.advance(c, b, dt, moved, Vec2::ZERO, 0.5, 40.0, 0.5, 1.0);
            }
            state.offsets(c, b)
        };
        // One second at 30fps against one second at 120fps.
        let (slow_rot, slow_pos) = sample(30, 1.0 / 30.0);
        let (fast_rot, fast_pos) = sample(120, 1.0 / 120.0);
        assert!(
            (slow_rot - fast_rot).abs() < 1e-3,
            "30fps {slow_rot} vs 120fps {fast_rot}"
        );
        assert!(
            (slow_pos - fast_pos).length() < 1e-3,
            "30fps {slow_pos:?} vs 120fps {fast_pos:?}"
        );
    }

    /// A stall must under-simulate rather than freeze: a huge dt is capped.
    #[test]
    fn an_enormous_step_is_capped_rather_than_simulated_in_full() {
        let (c, b) = ids();
        let mut state = PhysicsState::new();
        // Ten seconds in one frame — a breakpoint, a loading hitch.
        let (rotation, position) = state.advance(
            c,
            b,
            10.0,
            Vec2::new(5.0, 0.0),
            Vec2::ZERO,
            0.5,
            40.0,
            0.5,
            1.0,
        );
        assert!(rotation.is_finite() && position.is_finite());
        assert!(
            rotation.abs() < 10.0,
            "the cap kept it sane: {rotation} rad"
        );
    }

    #[test]
    fn pausing_freezes_the_simulation_without_losing_it() {
        let (c, b) = ids();
        let mut state = PhysicsState::new();
        for _ in 0..30 {
            state.advance(
                c,
                b,
                1.0 / 60.0,
                Vec2::new(5.0, 0.0),
                Vec2::ZERO,
                0.5,
                40.0,
                0.5,
                1.0,
            );
        }
        let before = state.offsets(c, b);
        state.paused = true;
        for _ in 0..30 {
            state.advance(
                c,
                b,
                1.0 / 60.0,
                Vec2::new(5.0, 0.0),
                Vec2::ZERO,
                0.5,
                40.0,
                0.5,
                1.0,
            );
        }
        assert_eq!(state.offsets(c, b), before, "paused means paused");

        state.reset();
        assert_eq!(
            state.offsets(c, b),
            (0.0, Vec2::ZERO),
            "reset returns to rest"
        );
    }
}
