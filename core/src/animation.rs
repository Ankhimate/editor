//! Animation model + sampling (PLAN §2.7).
//!
//! # Key value semantics (Spine convention)
//!
//! Bone timeline keys are **offsets from the setup pose**, not absolute values:
//!
//! * `BoneTranslate` / `BoneShear` — added to the setup value.
//! * `BoneScale` — **multiplied** by the setup value (a key of `1.0` means "as
//!   authored"; additive would make `0.0` mean "unscaled", which is unusable).
//! * `BoneRotate` — added to the setup rotation, shortest-arc between keys.
//!
//! This is what makes an animation reusable across skeletons whose setup poses
//! differ, and it is why two animations can be mixed by `alpha` at all.
//!
//! # Angle unit
//!
//! `BoneRotate` keys are **degrees** at the document level (PLAN §2.7); they are
//! converted to radians when written into a `Pose`, because everything inside
//! `core`'s math is radians (see the `transforms` module docs).
//!
//! # Bezier evaluation
//!
//! `Interp::Bezier` stores two handles in normalized 0..1 time/value space. To
//! sample, the curve's `x(t) = time` must be inverted to find `t`, then `y(t)`
//! gives the eased fraction. **Chosen method: Newton-Raphson with a bisection
//! fallback** (see `solve_bezier_x`) rather than a fixed-step LUT:
//!
//! * exact to ~1e-6 in a handful of iterations, versus a LUT's quantization
//!   error at segment boundaries;
//! * no per-key table to build, store, or invalidate when a handle moves — which
//!   matters because the editor drags handles interactively (T-507);
//! * deterministic: a fixed iteration cap, no early-exit on wall-clock.

use crate::ids::{BoneId, ConstraintId, SlotId};
use serde::{Deserialize, Serialize};

/// How the value approaching a key is interpolated from the previous key.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Interp {
    #[default]
    Linear,
    /// Hold the previous key's value until this key's time.
    Stepped,
    /// Cubic bezier in normalized 0..1 time/value space. `out_handle` belongs to
    /// the previous key, `in_handle` to this one.
    Bezier {
        out_handle: glam::Vec2,
        in_handle: glam::Vec2,
    },
}

/// One keyframe: a value at a time, plus how to get there from the previous key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Key<T> {
    /// Seconds from the start of the animation.
    pub time: f32,
    pub value: T,
    #[serde(default)]
    pub interp: Interp,
}

impl<T> Key<T> {
    pub fn linear(time: f32, value: T) -> Self {
        Self {
            time,
            value,
            interp: Interp::Linear,
        }
    }

    pub fn stepped(time: f32, value: T) -> Self {
        Self {
            time,
            value,
            interp: Interp::Stepped,
        }
    }
}

/// A named trigger for runtimes. Post-v1; carried so files round-trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventKey {
    pub time: f32,
    pub name: String,
    #[serde(default)]
    pub int_value: i32,
    #[serde(default)]
    pub float_value: f32,
    #[serde(default)]
    pub string_value: String,
}

/// One animated property track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Timeline {
    /// Offset added to the bone's setup translation.
    BoneTranslate {
        bone: BoneId,
        keys: Vec<Key<glam::Vec2>>,
    },
    /// Offset in **degrees** added to the bone's setup rotation, shortest-arc.
    BoneRotate { bone: BoneId, keys: Vec<Key<f32>> },
    /// Factor **multiplied** into the bone's setup scale.
    BoneScale {
        bone: BoneId,
        keys: Vec<Key<glam::Vec2>>,
    },
    /// Offset in **degrees** added to the bone's setup shear — same unit as
    /// [`Timeline::BoneRotate`], so every angle key on disk and in memory reads
    /// in degrees.
    BoneShear {
        bone: BoneId,
        keys: Vec<Key<glam::Vec2>>,
    },
    /// Absolute RGBA, replacing the slot's setup color.
    SlotColor {
        slot: SlotId,
        keys: Vec<Key<[f32; 4]>>,
    },
    /// Absolute attachment name. Stepped by construction — names do not blend.
    SlotAttachment {
        slot: SlotId,
        keys: Vec<Key<Option<String>>>,
    },
    /// Draw-order **offsets** from the setup order (`slot moved +2 / −1`).
    /// Stepped by construction.
    DrawOrder { keys: Vec<Key<Vec<(SlotId, i32)>>> },
    /// Absolute mix for an IK constraint.
    IkMix {
        constraint: ConstraintId,
        keys: Vec<Key<f32>>,
    },
    /// Bend direction for an IK constraint: `+1` or `-1`, stepped (T-504).
    ///
    /// Stepped by construction — a bend direction interpolated through zero
    /// would pass through "no preference" and let the chain flip either way,
    /// which is exactly the artifact this timeline exists to control.
    IkBendDirection {
        constraint: ConstraintId,
        keys: Vec<Key<f32>>,
    },
    /// Softness for an IK constraint, in world units (T-504).
    IkSoftness {
        constraint: ConstraintId,
        keys: Vec<Key<f32>>,
    },
    /// Absolute per-channel mixes for a transform constraint (T-501), in the
    /// order `[rotate, translate, scale, shear]`.
    ///
    /// One timeline rather than four because the channels are almost always
    /// keyed together — "the constraint fades in" means all of it — and four
    /// dopesheet rows per constraint would bury the bone rows that matter.
    TransformConstraintMix {
        constraint: ConstraintId,
        keys: Vec<Key<[f32; 4]>>,
    },
    /// Per-vertex offsets from the attachment's setup vertices.
    Deform {
        slot: SlotId,
        attachment: String,
        keys: Vec<Key<Vec<glam::Vec2>>>,
    },
}

impl Timeline {
    /// Time of the last key, or `0.0` for an empty timeline.
    pub fn last_key_time(&self) -> f32 {
        macro_rules! last {
            ($keys:expr) => {
                $keys.last().map(|k| k.time).unwrap_or(0.0)
            };
        }
        match self {
            Timeline::BoneTranslate { keys, .. } => last!(keys),
            Timeline::BoneRotate { keys, .. } => last!(keys),
            Timeline::BoneScale { keys, .. } => last!(keys),
            Timeline::BoneShear { keys, .. } => last!(keys),
            Timeline::SlotColor { keys, .. } => last!(keys),
            Timeline::SlotAttachment { keys, .. } => last!(keys),
            Timeline::DrawOrder { keys } => last!(keys),
            Timeline::IkMix { keys, .. } => last!(keys),
            Timeline::IkBendDirection { keys, .. } => last!(keys),
            Timeline::IkSoftness { keys, .. } => last!(keys),
            Timeline::TransformConstraintMix { keys, .. } => last!(keys),
            Timeline::Deform { keys, .. } => last!(keys),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Timeline::BoneTranslate { keys, .. } => keys.is_empty(),
            Timeline::BoneRotate { keys, .. } => keys.is_empty(),
            Timeline::BoneScale { keys, .. } => keys.is_empty(),
            Timeline::BoneShear { keys, .. } => keys.is_empty(),
            Timeline::SlotColor { keys, .. } => keys.is_empty(),
            Timeline::SlotAttachment { keys, .. } => keys.is_empty(),
            Timeline::DrawOrder { keys } => keys.is_empty(),
            Timeline::IkMix { keys, .. } => keys.is_empty(),
            Timeline::IkBendDirection { keys, .. } => keys.is_empty(),
            Timeline::IkSoftness { keys, .. } => keys.is_empty(),
            Timeline::TransformConstraintMix { keys, .. } => keys.is_empty(),
            Timeline::Deform { keys, .. } => keys.is_empty(),
        }
    }
}

/// A named animation: a duration plus the timelines that drive it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Animation {
    pub name: String,
    /// Seconds. The editor displays frames at the project FPS.
    pub duration: f32,
    pub timelines: Vec<Timeline>,
    #[serde(default)]
    pub events: Vec<EventKey>,
    /// Whether this clip is meant to loop — authoring intent, carried to the
    /// runtime (T-604) rather than enforced here. `evaluate` samples whatever
    /// time it is handed; wrapping is the player's job.
    #[serde(default = "yes")]
    pub looping: bool,
}

fn yes() -> bool {
    true
}

impl Animation {
    pub fn new(name: impl Into<String>, duration: f32) -> Self {
        Self {
            name: name.into(),
            duration,
            timelines: Vec::new(),
            events: Vec::new(),
            looping: true,
        }
    }

    /// Longest key time across all timelines — the minimum `duration` for the
    /// animation to play out fully.
    pub fn content_duration(&self) -> f32 {
        self.timelines
            .iter()
            .map(|t| t.last_key_time())
            .fold(0.0, f32::max)
    }
}

// ── Sampling ────────────────────────────────────────────────────────────────

/// Result of locating `time` within a key list.
enum Span {
    /// The list is empty.
    Empty,
    /// `time` is at or before the first key, at or after the last, or the list
    /// has one key: hold key `.0`.
    Hold(usize),
    /// `time` falls between `from` and `from + 1`, `fraction` of the way across
    /// (already eased by the target key's `Interp`).
    Between { from: usize, fraction: f32 },
}

/// Locate `time` in a sorted key list via binary search.
///
/// `hint` is a per-timeline cache of the last span index, checked first so
/// sequential playback is O(1) per frame instead of O(log n). A stale hint is
/// detected and falls back to the binary search, so it can never change the
/// result — only the cost.
fn locate<T>(keys: &[Key<T>], time: f32, hint: &mut usize) -> Span {
    if keys.is_empty() {
        return Span::Empty;
    }
    if keys.len() == 1 || time <= keys[0].time {
        return Span::Hold(0);
    }
    let last = keys.len() - 1;
    if time >= keys[last].time {
        return Span::Hold(last);
    }

    // Sequential-playback fast path: is `time` still inside the cached span, or
    // the one right after it?
    let mut from = usize::MAX;
    if *hint < last && keys[*hint].time <= time && time < keys[*hint + 1].time {
        from = *hint;
    } else if *hint + 2 <= last && keys[*hint + 1].time <= time && time < keys[*hint + 2].time {
        from = *hint + 1;
    }

    if from == usize::MAX {
        // `partition_point` counts keys at or before `time`; the span starts at
        // the one before that. Both bounds are in range thanks to the early
        // returns above.
        from = keys.partition_point(|k| k.time <= time).saturating_sub(1);
    }
    *hint = from;

    let (a, b) = (&keys[from], &keys[from + 1]);
    let raw = if b.time > a.time {
        (time - a.time) / (b.time - a.time)
    } else {
        // Coincident keys: jump straight to the later value.
        1.0
    };

    Span::Between {
        from,
        fraction: ease(raw, b.interp),
    }
}

/// Apply a key's interpolation curve to a normalized 0..1 span fraction.
fn ease(t: f32, interp: Interp) -> f32 {
    match interp {
        Interp::Linear => t,
        Interp::Stepped => 0.0, // hold the previous value until the next key
        Interp::Bezier {
            out_handle,
            in_handle,
        } => {
            let u = solve_bezier_x(t, out_handle.x, in_handle.x);
            bezier_axis(u, out_handle.y, in_handle.y)
        }
    }
}

/// Cubic bezier on one axis with endpoints fixed at 0 and 1:
/// `3(1-t)²t·p1 + 3(1-t)t²·p2 + t³`.
fn bezier_axis(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * p1 + 3.0 * inv * t * t * p2 + t * t * t
}

/// Derivative of [`bezier_axis`] with respect to `t`.
fn bezier_axis_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * p1 + 6.0 * inv * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Invert `x(t) = target` for a cubic bezier whose x control points are
/// `x1`, `x2`. See the module docs for why Newton-Raphson over a LUT.
fn solve_bezier_x(target: f32, x1: f32, x2: f32) -> f32 {
    const NEWTON_ITERS: usize = 8;
    const BISECT_ITERS: usize = 24;
    const EPSILON: f32 = 1e-6;

    if target <= 0.0 {
        return 0.0;
    }
    if target >= 1.0 {
        return 1.0;
    }

    // Newton-Raphson from the linear guess.
    let mut t = target;
    for _ in 0..NEWTON_ITERS {
        let error = bezier_axis(t, x1, x2) - target;
        if error.abs() < EPSILON {
            return t;
        }
        let slope = bezier_axis_derivative(t, x1, x2);
        // A (near-)flat slope would make Newton diverge; hand over to bisection.
        if slope.abs() < 1e-6 {
            break;
        }
        // Handles outside 0..1 can throw the iterate out of the valid domain.
        t = (t - error / slope).clamp(0.0, 1.0);
    }

    // Bisection fallback — always converges for a monotonic x(t).
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    let mut t = target;
    for _ in 0..BISECT_ITERS {
        let x = bezier_axis(t, x1, x2);
        if (x - target).abs() < EPSILON {
            break;
        }
        if x < target {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) * 0.5;
    }
    t
}

/// Shortest-arc interpolation between two angles in **degrees**.
pub fn lerp_angle_degrees(a: f32, b: f32, t: f32) -> f32 {
    let mut delta = (b - a) % 360.0;
    if delta > 180.0 {
        delta -= 360.0;
    } else if delta < -180.0 {
        delta += 360.0;
    }
    a + delta * t
}

/// A value that can be interpolated between keys.
pub trait Sampleable: Clone {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self;
}

impl Sampleable for f32 {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        a + (b - a) * t
    }
}

impl Sampleable for glam::Vec2 {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        *a + (*b - *a) * t
    }
}

impl Sampleable for [f32; 4] {
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ]
    }
}

impl Sampleable for Vec<glam::Vec2> {
    /// Per-vertex lerp. Mismatched lengths interpolate the common prefix and keep
    /// `a`'s tail, so a re-meshed attachment degrades instead of panicking.
    fn lerp(a: &Self, b: &Self, t: f32) -> Self {
        let mut out = a.clone();
        for (i, dst) in out.iter_mut().enumerate() {
            if let Some(bv) = b.get(i) {
                *dst += (*bv - *dst) * t;
            }
        }
        out
    }
}

/// Sample an interpolatable timeline at `time`.
///
/// Returns `None` only when the timeline has no keys.
pub fn sample<T: Sampleable>(keys: &[Key<T>], time: f32, hint: &mut usize) -> Option<T> {
    match locate(keys, time, hint) {
        Span::Empty => None,
        Span::Hold(i) => Some(keys[i].value.clone()),
        Span::Between { from, fraction } => {
            Some(T::lerp(&keys[from].value, &keys[from + 1].value, fraction))
        }
    }
}

/// Sample a rotation timeline (degrees) at `time`, interpolating shortest-arc.
pub fn sample_angle_degrees(keys: &[Key<f32>], time: f32, hint: &mut usize) -> Option<f32> {
    match locate(keys, time, hint) {
        Span::Empty => None,
        Span::Hold(i) => Some(keys[i].value),
        Span::Between { from, fraction } => Some(lerp_angle_degrees(
            keys[from].value,
            keys[from + 1].value,
            fraction,
        )),
    }
}

/// Sample a stepped (non-interpolatable) timeline at `time`: the value of the
/// last key at or before `time`.
pub fn sample_stepped<T: Clone>(keys: &[Key<T>], time: f32, hint: &mut usize) -> Option<T> {
    match locate(keys, time, hint) {
        Span::Empty => None,
        Span::Hold(i) => Some(keys[i].value.clone()),
        // Hold `from` until `from + 1` is actually reached; `locate` only yields a
        // fraction of 1.0 here for coincident keys.
        Span::Between { from, fraction } => {
            let i = if fraction >= 1.0 { from + 1 } else { from };
            Some(keys[i].value.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn keys_f32(pairs: &[(f32, f32)]) -> Vec<Key<f32>> {
        pairs.iter().map(|&(t, v)| Key::linear(t, v)).collect()
    }

    #[test]
    fn empty_timeline_samples_to_none() {
        let keys: Vec<Key<f32>> = Vec::new();
        let mut hint = 0;
        assert!(sample(&keys, 0.5, &mut hint).is_none());
    }

    #[test]
    fn single_key_holds_everywhere() {
        let keys = keys_f32(&[(1.0, 42.0)]);
        let mut hint = 0;
        for t in [-5.0, 0.0, 1.0, 99.0] {
            assert_eq!(sample(&keys, t, &mut hint), Some(42.0));
        }
    }

    #[test]
    fn clamps_before_first_and_after_last_key() {
        let keys = keys_f32(&[(1.0, 10.0), (2.0, 20.0)]);
        let mut hint = 0;
        assert_eq!(sample(&keys, 0.0, &mut hint), Some(10.0));
        assert_eq!(sample(&keys, 5.0, &mut hint), Some(20.0));
    }

    #[test]
    fn linear_interpolation_golden_values() {
        let keys = keys_f32(&[(0.0, 0.0), (1.0, 100.0)]);
        let mut hint = 0;
        for (time, want) in [(0.0, 0.0), (0.25, 25.0), (0.5, 50.0), (1.0, 100.0)] {
            let got = sample(&keys, time, &mut hint).unwrap();
            assert!((got - want).abs() < EPS, "at {time}: {got} != {want}");
        }
    }

    #[test]
    fn stepped_holds_until_the_next_key() {
        let keys = vec![Key::linear(0.0, 0.0), Key::stepped(1.0, 100.0)];
        let mut hint = 0;
        // The *target* key's interp decides the approach, so the whole span holds.
        assert_eq!(sample(&keys, 0.0, &mut hint), Some(0.0));
        assert_eq!(sample(&keys, 0.99, &mut hint), Some(0.0));
        assert_eq!(sample(&keys, 1.0, &mut hint), Some(100.0));
    }

    #[test]
    fn bezier_ease_in_out_golden_values() {
        // The classic ease-in-out handles.
        let interp = Interp::Bezier {
            out_handle: glam::vec2(0.42, 0.0),
            in_handle: glam::vec2(0.58, 1.0),
        };
        let keys = vec![
            Key::linear(0.0, 0.0),
            Key {
                time: 1.0,
                value: 100.0,
                interp,
            },
        ];
        let mut hint = 0;

        // Endpoints are exact.
        assert!((sample(&keys, 0.0, &mut hint).unwrap() - 0.0).abs() < EPS);
        assert!((sample(&keys, 1.0, &mut hint).unwrap() - 100.0).abs() < EPS);
        // Symmetric curve passes through the midpoint.
        let mid = sample(&keys, 0.5, &mut hint).unwrap();
        assert!((mid - 50.0).abs() < 0.5, "midpoint {mid}");
        // Ease-in: slower than linear early, faster late.
        let quarter = sample(&keys, 0.25, &mut hint).unwrap();
        assert!(quarter < 25.0, "should lag linear early: {quarter}");
        let three_q = sample(&keys, 0.75, &mut hint).unwrap();
        assert!(three_q > 75.0, "should lead linear late: {three_q}");
    }

    #[test]
    fn bezier_is_monotonic_for_sane_handles() {
        let interp = Interp::Bezier {
            out_handle: glam::vec2(0.42, 0.0),
            in_handle: glam::vec2(0.58, 1.0),
        };
        let keys = vec![
            Key::linear(0.0, 0.0),
            Key {
                time: 1.0,
                value: 1.0,
                interp,
            },
        ];
        let mut hint = 0;
        let mut previous = f32::NEG_INFINITY;
        for i in 0..=100 {
            let v = sample(&keys, i as f32 / 100.0, &mut hint).unwrap();
            assert!(
                v >= previous - EPS,
                "not monotonic at {i}: {v} < {previous}"
            );
            previous = v;
        }
    }

    #[test]
    fn bezier_solve_inverts_x_exactly() {
        // Round-trip: x(solve(x)) == x for a range of handle shapes.
        for (x1, x2) in [
            (0.42, 0.58),
            (0.1, 0.9),
            (0.9, 0.1),
            (0.0, 1.0),
            (0.25, 0.25),
        ] {
            for i in 0..=20 {
                let target = i as f32 / 20.0;
                let t = solve_bezier_x(target, x1, x2);
                let back = bezier_axis(t, x1, x2);
                assert!(
                    (back - target).abs() < 1e-3,
                    "handles ({x1},{x2}) target {target}: got {back}"
                );
            }
        }
    }

    #[test]
    fn rotation_interpolates_shortest_arc_across_the_seam() {
        // 170° → -170° is a 20° step forward through 180, not 340° backwards.
        let keys = keys_f32(&[(0.0, 170.0), (1.0, -170.0)]);
        let mut hint = 0;
        let mid = sample_angle_degrees(&keys, 0.5, &mut hint).unwrap();
        assert!(
            (mid - 180.0).abs() < EPS || (mid + 180.0).abs() < EPS,
            "midpoint {mid}"
        );

        // A plain lerp would have gone the long way round through 0.
        let naive = sample(&keys, 0.5, &mut hint).unwrap();
        assert!((naive - 0.0).abs() < EPS, "plain lerp sanity: {naive}");
    }

    #[test]
    fn rotation_shortest_arc_the_other_direction() {
        let keys = keys_f32(&[(0.0, -170.0), (1.0, 170.0)]);
        let mut hint = 0;
        let mid = sample_angle_degrees(&keys, 0.5, &mut hint).unwrap();
        assert!(
            (mid - 180.0).abs() < EPS || (mid + 180.0).abs() < EPS,
            "midpoint {mid}"
        );
    }

    #[test]
    fn stepped_sampler_never_interpolates() {
        let keys = vec![
            Key::stepped(0.0, Some("a".to_string())),
            Key::stepped(1.0, Some("b".to_string())),
        ];
        let mut hint = 0;
        assert_eq!(
            sample_stepped(&keys, 0.5, &mut hint),
            Some(Some("a".to_string()))
        );
        assert_eq!(
            sample_stepped(&keys, 1.0, &mut hint),
            Some(Some("b".to_string()))
        );
    }

    #[test]
    fn vec2_and_color_interpolate_componentwise() {
        let keys = vec![
            Key::linear(0.0, glam::vec2(0.0, 10.0)),
            Key::linear(1.0, glam::vec2(10.0, 0.0)),
        ];
        let mut hint = 0;
        let mid = sample(&keys, 0.5, &mut hint).unwrap();
        assert!((mid - glam::vec2(5.0, 5.0)).length() < EPS, "{mid:?}");

        let keys = vec![
            Key::linear(0.0, [0.0, 0.0, 0.0, 1.0]),
            Key::linear(1.0, [1.0, 1.0, 1.0, 0.0]),
        ];
        let mut hint = 0;
        let mid = sample(&keys, 0.5, &mut hint).unwrap();
        assert_eq!(mid, [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn deform_lerp_tolerates_mismatched_vertex_counts() {
        let a = vec![glam::vec2(0.0, 0.0), glam::vec2(0.0, 0.0)];
        let b = vec![glam::vec2(10.0, 10.0)];
        let out = Vec::<glam::Vec2>::lerp(&a, &b, 0.5);
        assert_eq!(out.len(), 2);
        assert!((out[0] - glam::vec2(5.0, 5.0)).length() < EPS);
        // Missing counterpart keeps `a`'s value.
        assert!((out[1] - glam::vec2(0.0, 0.0)).length() < EPS);
    }

    #[test]
    fn hint_cache_matches_cold_binary_search() {
        // Sequential playback reuses the hint; results must be identical to a
        // fresh search at every sample point.
        let keys = keys_f32(&[
            (0.0, 0.0),
            (1.0, 10.0),
            (2.0, 20.0),
            (3.0, 30.0),
            (4.0, 40.0),
        ]);
        let mut warm = 0;
        for i in 0..=80 {
            let time = i as f32 * 0.05;
            let mut cold = 0;
            let a = sample(&keys, time, &mut warm).unwrap();
            let b = sample(&keys, time, &mut cold).unwrap();
            assert!((a - b).abs() < EPS, "at {time}: warm {a} != cold {b}");
        }
    }

    #[test]
    fn hint_cache_survives_random_access() {
        let keys = keys_f32(&[(0.0, 0.0), (1.0, 10.0), (2.0, 20.0), (3.0, 30.0)]);
        let mut hint = 0;
        // Jump around; a stale hint must not produce a wrong span.
        for &time in &[2.5, 0.1, 3.0, 1.5, 0.0, 2.9] {
            let mut cold = 0;
            let a = sample(&keys, time, &mut hint).unwrap();
            let b = sample(&keys, time, &mut cold).unwrap();
            assert!((a - b).abs() < EPS, "at {time}: {a} != {b}");
        }
    }

    #[test]
    fn coincident_keys_take_the_later_value() {
        let keys = keys_f32(&[(0.0, 0.0), (1.0, 10.0), (1.0, 99.0), (2.0, 20.0)]);
        let mut hint = 0;
        // Landing exactly on the duplicated time resolves to the last of them.
        let got = sample(&keys, 1.0, &mut hint).unwrap();
        assert!((got - 99.0).abs() < EPS, "{got}");
    }

    #[test]
    fn content_duration_is_the_last_key_time() {
        let mut anim = Animation::new("walk", 1.0);
        assert_eq!(anim.content_duration(), 0.0);

        anim.timelines.push(Timeline::BoneRotate {
            bone: BoneId::default(),
            keys: keys_f32(&[(0.0, 0.0), (2.5, 90.0)]),
        });
        anim.timelines.push(Timeline::BoneTranslate {
            bone: BoneId::default(),
            keys: vec![Key::linear(0.0, glam::Vec2::ZERO)],
        });
        assert!((anim.content_duration() - 2.5).abs() < EPS);
    }

    #[test]
    fn timeline_is_empty_reports_per_variant() {
        let empty = Timeline::DrawOrder { keys: Vec::new() };
        assert!(empty.is_empty());
        let filled = Timeline::DrawOrder {
            keys: vec![Key::stepped(0.0, Vec::new())],
        };
        assert!(!filled.is_empty());
    }
}
