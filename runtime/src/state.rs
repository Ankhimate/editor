//! Playback state (T-604): what is playing, and how far in.
//!
//! Crossfade here is **weighting**, not interpolation of two finished poses.
//! `evaluate()` already accepts `(animation, time, alpha)` triples and mixes
//! them (PLAN §2.6), so a fade hands core two entries with weights that sum to
//! one and core does the blend. Blending two output poses instead would give
//! different — and wrong — results the moment a bone is keyed in one clip and
//! not the other, and it would disagree with what the editor shows.

use crate::Rig;
use ankhimate_core::animation::Animation;
use ankhimate_core::ids::AnimationId;
use ankhimate_core::physics::PhysicsState;
use ankhimate_core::pose::Pose;

/// One playing clip.
#[derive(Debug, Clone)]
pub struct TrackEntry {
    pub animation: AnimationId,
    /// Seconds into the clip.
    pub time: f32,
    pub looping: bool,
    /// Playback rate. 1.0 is authored speed; negative plays backwards.
    pub speed: f32,
    /// Contribution to the final pose, 0..=1.
    pub alpha: f32,
    /// How long the fade in still has to run, in seconds. Zero means "at full
    /// weight".
    pub fade_remaining: f32,
    pub fade_total: f32,
    /// Set when this entry is fading *out* and should be dropped at zero.
    pub fading_out: bool,
}

impl TrackEntry {
    fn new(animation: AnimationId, looping: bool) -> Self {
        Self {
            animation,
            time: 0.0,
            looping,
            speed: 1.0,
            alpha: 1.0,
            fade_remaining: 0.0,
            fade_total: 0.0,
            fading_out: false,
        }
    }
}

/// An event that fired during [`AnimationState::update`].
#[derive(Debug, Clone, PartialEq)]
pub struct FiredEvent {
    pub name: String,
    /// Seconds into the clip the event sits at.
    pub time: f32,
    pub int_value: i32,
    pub float_value: f32,
    pub string_value: String,
    pub audio: String,
    pub volume: f32,
    pub balance: f32,
}

/// What is playing on one track.
///
/// One track is enough for the common case (walk fading into idle). Layering —
/// an upper-body aim over a lower-body run — is a second track, which this is
/// shaped for but does not yet implement; see the note on [`AnimationState`].
#[derive(Default)]
pub struct AnimationState {
    /// Entries, oldest first. The last is the one being faded *to*.
    entries: Vec<TrackEntry>,
    /// Physics carries its own accumulator so the sim is framerate-independent
    /// and deterministic (PLAN §2.6, ADR 0007). Owned here, never in the
    /// document.
    physics: PhysicsState,
    fired: Vec<FiredEvent>,
}

impl AnimationState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Play `animation` immediately, dropping anything already playing.
    pub fn play(&mut self, animation: AnimationId, looping: bool) {
        self.entries.clear();
        self.entries.push(TrackEntry::new(animation, looping));
    }

    /// Fade from what is playing into `animation` over `duration` seconds.
    ///
    /// A zero or negative duration is an immediate cut rather than an error:
    /// `crossfade(next, 0.0)` is a reasonable thing for a game to write, and
    /// making it panic would be hostile.
    pub fn crossfade(&mut self, animation: AnimationId, duration: f32, looping: bool) {
        if duration <= 0.0 || self.entries.is_empty() {
            self.play(animation, looping);
            return;
        }
        // Already fading to this clip: let it finish rather than restarting the
        // fade every frame a game calls this in an update loop.
        if let Some(last) = self.entries.last()
            && last.animation == animation
            && !last.fading_out
        {
            return;
        }
        for entry in &mut self.entries {
            entry.fading_out = true;
        }
        let mut next = TrackEntry::new(animation, looping);
        next.alpha = 0.0;
        next.fade_remaining = duration;
        next.fade_total = duration;
        self.entries.push(next);
    }

    /// Stop everything. The rig holds its setup pose.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry driving the pose most strongly, for a game asking "what is it
    /// doing right now".
    pub fn current(&self) -> Option<&TrackEntry> {
        self.entries
            .iter()
            .max_by(|a, b| a.alpha.total_cmp(&b.alpha))
    }

    pub fn set_speed(&mut self, speed: f32) {
        for entry in &mut self.entries {
            entry.speed = speed;
        }
    }

    /// Events that fired during the last [`update`](Self::update).
    pub fn events(&self) -> &[FiredEvent] {
        &self.fired
    }

    /// Advance by `dt` seconds and write the resulting pose into `out`.
    ///
    /// Deterministic for a given `(dt sequence, starting state)`: no wall clock
    /// is read here, and physics keeps its own fixed-step accumulator.
    pub fn update(&mut self, rig: &Rig, dt: f32, out: &mut Pose) {
        self.fired.clear();
        self.advance(rig, dt);

        // Collect the weighted list core wants. Weights are normalised so a
        // mid-fade pose is not quietly scaled down: two entries at 0.5 must mean
        // "half of each", not "half a pose".
        let total: f32 = self.entries.iter().map(|e| e.alpha).sum();
        let scale = if total > 0.0 { 1.0 / total } else { 0.0 };

        let weighted: Vec<(&Animation, f32, f32)> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let anim = rig.animations.get(entry.animation)?;
                Some((anim, entry.time, entry.alpha * scale))
            })
            .collect();

        ankhimate_core::pose::evaluate_with(&rig.skeleton, &weighted, &mut self.physics, dt, out);
    }

    fn advance(&mut self, rig: &Rig, dt: f32) {
        for entry in &mut self.entries {
            let Some(anim) = rig.animations.get(entry.animation) else {
                continue;
            };
            let before = entry.time;
            entry.time += dt * entry.speed;
            let duration = anim.duration.max(f32::EPSILON);

            // Event windowing lives in core (`events_in_window`), which already
            // handles the two cases a hand-rolled version gets wrong: a step that
            // straddles the loop point, and a `dt` larger than the whole clip.
            // Reimplementing it here would be a second answer to "did the
            // footstep fire", and the editor's would be the one users trust.
            self.fired.extend(
                ankhimate_core::animation::events_in_window(
                    anim,
                    before,
                    entry.time,
                    entry.looping,
                )
                .into_iter()
                .map(FiredEvent::from),
            );

            if entry.looping {
                entry.time = entry.time.rem_euclid(duration);
            } else if entry.time > duration {
                entry.time = duration;
            }

            if entry.fade_remaining > 0.0 {
                entry.fade_remaining = (entry.fade_remaining - dt).max(0.0);
                let progress = if entry.fade_total > 0.0 {
                    1.0 - entry.fade_remaining / entry.fade_total
                } else {
                    1.0
                };
                entry.alpha = progress.clamp(0.0, 1.0);
            }
        }

        // Fading-out entries lose exactly what the incoming one gains, so the
        // pair always sums to 1.
        if let Some(incoming) = self.entries.last().map(|e| e.alpha) {
            let count = self.entries.len();
            for entry in self.entries.iter_mut().take(count.saturating_sub(1)) {
                if entry.fading_out {
                    entry.alpha = 1.0 - incoming;
                }
            }
        }

        self.entries
            .retain(|e| !(e.fading_out && e.alpha <= f32::EPSILON));
    }
}

impl From<ankhimate_core::animation::EventKey> for FiredEvent {
    fn from(e: ankhimate_core::animation::EventKey) -> Self {
        Self {
            name: e.name,
            time: e.time,
            int_value: e.int_value,
            float_value: e.float_value,
            string_value: e.string_value,
            audio: e.audio,
            volume: e.volume,
            balance: e.balance,
        }
    }
}
