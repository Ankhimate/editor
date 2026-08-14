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
//! `Interp::Bezier` stores two handles as fractions of the span between the two
//! keys. To sample, the curve's `x(t) = time` must be inverted to find `t`, then
//! `y(t)` gives the eased fraction.
//!
//! **The two axes are bounded differently.** A handle's `x` must stay in 0..1:
//! `solve_bezier_x` bisects that domain assuming `x(t)` is monotonic, and a
//! control point outside the span makes the curve double back in time, which is
//! not a function of `t` and cannot be sampled at all. A handle's `y` is
//! **unbounded** — a value outside 0..1 makes `ease` return a fraction outside
//! 0..1, which `Sampleable::lerp` extrapolates through rather than clamping.
//! That is an overshoot, and it is how anticipation and follow-through are
//! authored: a bone winds back before it swings, and settles past its target
//! before returning.
//!
//! **Chosen method: Newton-Raphson with a bisection fallback** (see
//! `solve_bezier_x`) rather than a fixed-step LUT:
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
    /// Cubic bezier, handles as fractions of the span between the two keys.
    /// `out_handle` belongs to the previous key, `in_handle` to this one.
    ///
    /// `x` is in 0..1; `y` is unbounded, and a value outside 0..1 overshoots.
    /// See the module docs for why only one axis is bounded.
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

/// A named trigger for runtimes: footsteps, hit frames, sound cues (T-506).
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

    /// Sound to play with the event, as an asset name. Empty means silent.
    ///
    /// An event that says "footstep" and an event that *plays* a footstep are the
    /// same authoring act; splitting them into two timelines means keeping two
    /// things in sync forever.
    #[serde(default)]
    pub audio: String,
    /// Playback gain, `1.0` being the sample as recorded.
    #[serde(default = "unit")]
    pub volume: f32,
    /// Stereo placement: `-1` hard left, `0` centre, `1` hard right.
    #[serde(default)]
    pub balance: f32,
}

fn unit() -> f32 {
    1.0
}

/// Events fired by advancing a clip from `from` to `to` (T-506).
///
/// This is the whole contract a runtime needs: "I advanced the playhead by `dt`,
/// what fired?" — and it is fiddlier than it looks, which is why it lives in
/// `core` next to `evaluate` rather than in each runtime.
///
/// # The window is half-open
///
/// `(from, to]`. An event exactly at `from` has already fired on the previous
/// step; one exactly at `to` fires now. Closing both ends double-fires every
/// event on a frame boundary, and closing neither drops events that land exactly
/// on one — which, at 60fps with times authored in whole frames, is most of them.
///
/// # Looping and overshoot
///
/// When `looping`, a step that crosses the end wraps: the window becomes
/// `(from, duration]` plus `(0, to']`. A `dt` larger than the whole clip fires
/// every event once per full lap, in time order, so a frame hitch cannot silently
/// swallow a footstep. That bound is the reason this returns owned copies rather
/// than borrowing: a lap's worth of events may repeat.
///
/// Events are returned in the order they fire, which for multiple laps means the
/// clip's order repeated — a runtime that plays sounds in sequence needs that,
/// and sorting by time alone would interleave laps.
pub fn events_in_window(anim: &Animation, from: f32, to: f32, looping: bool) -> Vec<EventKey> {
    if anim.events.is_empty() || anim.duration <= 0.0 {
        return Vec::new();
    }
    // Events are authored in any order; firing order is time order.
    let mut sorted: Vec<&EventKey> = anim.events.iter().collect();
    sorted.sort_by(|a, b| a.time.total_cmp(&b.time));

    let collect = |out: &mut Vec<EventKey>, lo: f32, hi: f32| {
        for e in &sorted {
            if e.time > lo && e.time <= hi {
                out.push((*e).clone());
            }
        }
    };

    let mut fired = Vec::new();
    if !looping {
        collect(&mut fired, from, to);
        return fired;
    }

    let duration = anim.duration;
    let advance = to - from;
    if advance <= 0.0 {
        // A backwards or stationary step fires nothing. Scrubbing a timeline
        // backwards should not replay a footstep.
        return fired;
    }

    // Finish the current lap.
    let start = from.rem_euclid(duration);
    let mut remaining = advance;
    let to_end = duration - start;
    if remaining < to_end {
        collect(&mut fired, start, start + remaining);
        return fired;
    }
    collect(&mut fired, start, duration);
    remaining -= to_end;

    // Whole laps in between: every event, once each.
    let laps = (remaining / duration).floor() as usize;
    for _ in 0..laps {
        collect(&mut fired, 0.0, duration);
    }
    remaining -= laps as f32 * duration;

    // The partial lap that lands on `to`. `0.0` is exclusive here for the same
    // reason the window is half-open: an event at time 0 fired as part of the
    // wrap above.
    collect(&mut fired, 0.0, remaining);
    fired
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
    /// Whether the slot draws at all (T-505). Stepped: a half-visible slot is
    /// what `SlotColor`'s alpha is for, and interpolating a boolean would make
    /// "hidden at frame 10, back at 20" fade instead of cut.
    SlotVisible { slot: SlotId, keys: Vec<Key<bool>> },
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
    /// The bone this timeline drives, if it drives one.
    ///
    /// `None` for slot, draw-order and constraint timelines — they are keyed on
    /// something other than a bone, and a caller that wants "which bone" has to
    /// handle their absence rather than be handed a wrong answer.
    pub fn bone(&self) -> Option<BoneId> {
        match self {
            Timeline::BoneTranslate { bone, .. }
            | Timeline::BoneRotate { bone, .. }
            | Timeline::BoneScale { bone, .. }
            | Timeline::BoneShear { bone, .. } => Some(*bone),
            _ => None,
        }
    }

    /// Re-point a bone timeline at a different bone.
    ///
    /// For transferring rigging between skeletons (T-909): a copied timeline
    /// holds ids from the document it came from, and pasting has to aim it at
    /// the bones the paste just created. A no-op on timelines that drive
    /// something other than a bone.
    pub fn set_bone(&mut self, to: BoneId) {
        match self {
            Timeline::BoneTranslate { bone, .. }
            | Timeline::BoneRotate { bone, .. }
            | Timeline::BoneScale { bone, .. }
            | Timeline::BoneShear { bone, .. } => *bone = to,
            _ => {}
        }
    }

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
            Timeline::SlotVisible { keys, .. } => last!(keys),
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
            Timeline::SlotVisible { keys, .. } => keys.is_empty(),
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

/// A label on the timeline ruler (T-906).
///
/// A note to whoever is animating — "contact", "down", "passing", "up" on a walk
/// cycle — so the structure of a clip is written down rather than counted out
/// each time it is opened.
///
/// **Not an event.** The distinction is the whole reason this is a separate
/// type: an [`EventKey`] fires into the running game and belongs to the runtime,
/// while a marker never leaves the editor and is exported by nothing. Folding
/// them together would mean either shipping the animator's notes to the game or
/// making every note a thing gameplay might react to, and both are wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    /// Seconds, like every other time in this module.
    pub time: f32,
    pub name: String,
    /// RGBA, so a set of markers can be grouped by eye on a busy ruler.
    #[serde(default = "default_marker_color")]
    pub color: [f32; 4],
}

fn default_marker_color() -> [f32; 4] {
    // A muted amber: legible on the dark ruler without competing with the
    // playhead, which is the one thing on that strip that must stay loudest.
    [0.95, 0.72, 0.30, 1.0]
}

impl Marker {
    pub fn new(time: f32, name: impl Into<String>) -> Self {
        Self {
            time,
            name: name.into(),
            color: default_marker_color(),
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
    /// Editor-only labels on the ruler (T-906). Kept sorted by time.
    ///
    /// Defaulted and skipped when empty, so a clip without markers serialises
    /// exactly as it did before they existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
    /// Seconds to shift a bone's timelines by when sampling, without moving a
    /// single key (T-905).
    ///
    /// The use this exists for is secondary motion: ten strands of hair, a tail,
    /// a scarf — every one wanting the same curve a few frames apart. Authoring
    /// that today means copying the keys ten times and dragging nine copies,
    /// after which changing the motion means redoing all of it.
    ///
    /// An offset is **not keyable**, deliberately. An animatable offset on top
    /// of animated tracks is a second time dimension: the value at a frame would
    /// depend on a time that itself depends on time, and a rig that misbehaves
    /// becomes impossible to reason about. It is authored once and shown on the
    /// track header.
    ///
    /// Negative offsets work, which is the underlying ask — a strand that leads
    /// rather than trails. Evaluation tolerates negative sample times by holding
    /// the first key, exactly as it holds the last past the end.
    ///
    /// Bones only. Slot, draw-order and constraint timelines have no offset
    /// because the motion this is for is bone motion, and a hidden time shift on
    /// an attachment swap would be a debugging nightmare for no gain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bone_offsets: Vec<(BoneId, f32)>,
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
            markers: Vec::new(),
            bone_offsets: Vec::new(),
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

    /// How far this bone's timelines are shifted when sampling (T-905).
    ///
    /// A `Vec` of pairs rather than a map because slotmap keys are not JSON
    /// object keys; the list is one entry per *offset* bone, which is a handful
    /// even on a rig with sixty, so the linear scan is cheaper than the map.
    pub fn bone_offset(&self, bone: BoneId) -> f32 {
        self.bone_offsets
            .iter()
            .find(|(id, _)| *id == bone)
            .map(|(_, offset)| *offset)
            .unwrap_or(0.0)
    }

    /// Set a bone's sampling offset, or clear it when zero.
    ///
    /// Zero is stored as absence rather than as an entry: "no offset" and "an
    /// offset of nothing" are the same state, and keeping both would let a file
    /// carry rows that do nothing.
    pub fn set_bone_offset(&mut self, bone: BoneId, offset: f32) {
        self.bone_offsets.retain(|(id, _)| *id != bone);
        if offset != 0.0 {
            self.bone_offsets.push((bone, offset));
        }
    }

    /// Add a marker, keeping the list ordered by time (T-906).
    ///
    /// Sorted on insert rather than on read because everything that consumes
    /// markers — drawing the ruler, snapping the playhead, stepping to the next
    /// one — wants them in order, and sorting once beats sorting at every call.
    pub fn add_marker(&mut self, marker: Marker) {
        let at = self.markers.partition_point(|m| m.time <= marker.time);
        self.markers.insert(at, marker);
    }

    /// The marker nearest `time`, within `tolerance` seconds.
    ///
    /// What "snap the playhead to a marker" and "which one did I just click"
    /// both need. `None` when nothing is close enough, so a caller can fall
    /// through to its own behaviour rather than being handed a distant marker.
    pub fn marker_near(&self, time: f32, tolerance: f32) -> Option<&Marker> {
        self.markers
            .iter()
            .filter(|m| (m.time - time).abs() <= tolerance)
            .min_by(|a, b| (a.time - time).abs().total_cmp(&(b.time - time).abs()))
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
///
/// The **input** is 0..1; the **return** need not be. A bezier whose value
/// handles reach outside the span returns a fraction outside 0..1, and callers
/// extrapolate through it — that is an overshoot, and clamping here would
/// silently flatten every bounce in the document.
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
    // Before the first key there is nothing to hold yet, so the setup value
    // stands. `None` says so; the caller has already seeded setup.
    //
    // This is where a stepped timeline differs from a blended one. Holding the
    // first key backwards would mean a mouth keyed to change at 0.9s wears that
    // shape from frame zero, and a draw-order key at 2s reorders the whole clip
    // before it.
    if keys.first().is_some_and(|k| time < k.time) {
        return None;
    }
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

    /// An offset shifts *when* a bone's curve is read, and moves no keys
    /// (T-905).
    ///
    /// The property that makes it worth having: the authored data is
    /// byte-identical before and after, so ten strands of hair can share one
    /// curve and nine offsets rather than ten copies of the keys.
    #[test]
    fn a_bone_offset_shifts_sampling_without_touching_the_keys() {
        use crate::skeleton::{Bone, Skeleton};

        let mut skel = Skeleton::new();
        let bone = skel.add_bone(Bone {
            name: "strand".into(),
            parent: None,
            length: 10.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });

        let mut anim = Animation::new("sway", 1.0);
        anim.timelines.push(Timeline::BoneRotate {
            bone,
            keys: keys_f32(&[(0.0, 0.0), (0.5, 90.0), (1.0, 0.0)]),
        });
        let keys_before = anim.timelines.clone();

        let sample_at = |anim: &Animation, time: f32| {
            let mut pose = crate::pose::Pose::new();
            crate::pose::evaluate(&skel, &[(anim, time, 1.0)], &mut pose);
            pose.locals[bone].rotation.to_degrees()
        };

        // Unshifted, the peak is at 0.5.
        assert!((sample_at(&anim, 0.5) - 90.0).abs() < EPS);

        // Trailing by a quarter second moves the peak to 0.75 — the curve is
        // read a quarter second earlier than the playhead says.
        anim.set_bone_offset(bone, 0.25);
        assert!((sample_at(&anim, 0.75) - 90.0).abs() < EPS);
        assert!(sample_at(&anim, 0.5) < 90.0, "the peak left 0.5");

        // Leading: the curve is read a quarter second *later*, so the peak
        // arrives early, at 0.25.
        anim.set_bone_offset(bone, -0.25);
        assert!((sample_at(&anim, 0.25) - 90.0).abs() < EPS);

        // A trailing strand asked for before its curve begins samples at a
        // negative time. That must hold the first key rather than extrapolate
        // or panic — it is the state every trailing strand is in for its first
        // few frames.
        anim.set_bone_offset(bone, 0.25);
        assert!(
            (sample_at(&anim, 0.0) - 0.0).abs() < EPS,
            "a negative sample time holds the first key"
        );

        // And through all of it the keys never moved.
        assert_eq!(anim.timelines, keys_before, "no key was touched");

        // Zero is stored as absence, so a cleared offset leaves no row behind.
        anim.set_bone_offset(bone, 0.0);
        assert!(anim.bone_offsets.is_empty());
        assert!((sample_at(&anim, 0.5) - 90.0).abs() < EPS);
    }

    /// Markers stay ordered however they are added (T-906).
    ///
    /// Everything downstream — drawing the ruler, snapping, stepping to the next
    /// one — assumes order, so it is established on insert rather than trusted.
    #[test]
    fn markers_stay_sorted_by_time() {
        let mut anim = Animation::new("walk", 1.0);
        for (time, name) in [
            (0.5, "passing"),
            (0.0, "contact"),
            (0.75, "up"),
            (0.25, "down"),
        ] {
            anim.add_marker(Marker::new(time, name));
        }
        let times: Vec<f32> = anim.markers.iter().map(|m| m.time).collect();
        assert_eq!(times, vec![0.0, 0.25, 0.5, 0.75]);
        let names: Vec<&str> = anim.markers.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["contact", "down", "passing", "up"]);
    }

    /// `marker_near` finds the closest inside the tolerance and nothing outside
    /// it — a snap that reached for a distant marker would move the playhead
    /// somewhere the user did not point.
    #[test]
    fn marker_near_picks_the_closest_within_tolerance() {
        let mut anim = Animation::new("walk", 1.0);
        anim.add_marker(Marker::new(0.20, "down"));
        anim.add_marker(Marker::new(0.30, "passing"));

        assert_eq!(
            anim.marker_near(0.22, 0.05).map(|m| m.name.as_str()),
            Some("down")
        );
        assert_eq!(
            anim.marker_near(0.28, 0.05).map(|m| m.name.as_str()),
            Some("passing")
        );
        // Equidistant is fine either way; what matters is that far is `None`.
        assert!(anim.marker_near(0.60, 0.05).is_none(), "too far to snap");
        assert!(
            anim.marker_near(0.25, 0.0).is_none(),
            "zero tolerance snaps to nothing"
        );
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

    /// A value handle outside 0..1 overshoots, and that is the point.
    ///
    /// Anticipation and follow-through are made this way: a bone winds back
    /// before it swings, and settles past its target before returning. `ease`
    /// returns a fraction outside 0..1 for such a handle and `Sampleable::lerp`
    /// extrapolates through it — neither clamps, deliberately.
    ///
    /// The Spine importer used to flatten these on the way in, on the grounds
    /// that handles were "normalized 0..1". Nothing enforced that, and this is
    /// what it was throwing away.
    #[test]
    fn a_value_handle_outside_the_span_overshoots() {
        let keys = vec![
            Key::linear(0.0, 0.0),
            Key {
                time: 1.0,
                value: 100.0,
                interp: Interp::Bezier {
                    // Below the start, then past the end.
                    out_handle: glam::vec2(0.3, -0.5),
                    in_handle: glam::vec2(0.7, 1.5),
                },
            },
        ];
        let mut hint = 0;
        let samples: Vec<f32> = (0..=100)
            .map(|i| sample(&keys, i as f32 / 100.0, &mut hint).unwrap())
            .collect();

        let lowest = samples.iter().cloned().fold(f32::INFINITY, f32::min);
        let highest = samples.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(lowest < -1.0, "should dip below the first key: {lowest}");
        assert!(highest > 101.0, "should pass the last key: {highest}");
    }

    /// Overshooting in the middle does not move where the curve starts and ends.
    ///
    /// A handle only shapes the approach; the keys themselves are the contract.
    /// An implementation that clamped the eased fraction to fix the overshoot
    /// would keep these endpoints and still be wrong, so this pins the half that
    /// must not change alongside the half that must.
    #[test]
    fn bezier_endpoints_hold_despite_wild_value_handles() {
        let keys = vec![
            Key::linear(0.0, 0.0),
            Key {
                time: 1.0,
                value: 100.0,
                interp: Interp::Bezier {
                    out_handle: glam::vec2(0.3, -0.5),
                    in_handle: glam::vec2(0.7, 1.5),
                },
            },
        ];
        let mut hint = 0;
        assert!((sample(&keys, 0.0, &mut hint).unwrap() - 0.0).abs() < EPS);
        assert!((sample(&keys, 1.0, &mut hint).unwrap() - 100.0).abs() < EPS);
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

    // ── Events (T-506) ───────────────────────────────────────────────────

    fn clip_with_events(times: &[f32], duration: f32) -> Animation {
        Animation {
            name: "walk".into(),
            duration,
            looping: true,
            timelines: Vec::new(),
            events: times
                .iter()
                .enumerate()
                .map(|(i, t)| EventKey {
                    time: *t,
                    name: format!("step{i}"),
                    int_value: 0,
                    float_value: 0.0,
                    string_value: String::new(),
                    audio: String::new(),
                    volume: 1.0,
                    balance: 0.0,
                })
                .collect(),
            markers: Vec::new(),
            bone_offsets: Vec::new(),
        }
    }

    fn names(fired: &[EventKey]) -> Vec<&str> {
        fired.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn an_event_fires_once_in_the_step_that_crosses_it() {
        let anim = clip_with_events(&[0.5], 1.0);
        assert_eq!(names(&events_in_window(&anim, 0.4, 0.6, true)), ["step0"]);
        // The next step must not fire it again.
        assert!(events_in_window(&anim, 0.6, 0.8, true).is_empty());
    }

    /// The half-open window is the whole reason this is not a one-liner: an
    /// event landing exactly on a frame boundary must fire once, not twice and
    /// not never.
    #[test]
    fn an_event_on_a_frame_boundary_fires_exactly_once() {
        let anim = clip_with_events(&[0.5], 1.0);
        let first = events_in_window(&anim, 0.25, 0.5, true);
        let second = events_in_window(&anim, 0.5, 0.75, true);
        assert_eq!(
            names(&first),
            ["step0"],
            "fires in the step that reaches it"
        );
        assert!(second.is_empty(), "and not again in the next step");
    }

    /// The acceptance case: a footstep fires exactly once per loop, including
    /// when the step crosses the loop boundary.
    #[test]
    fn a_looping_clip_fires_each_event_once_per_lap() {
        let anim = clip_with_events(&[0.2], 1.0);
        // Walk the clip in 0.1 steps for three laps.
        let mut count = 0;
        let mut t = 0.0;
        for _ in 0..30 {
            count += events_in_window(&anim, t, t + 0.1, true).len();
            t += 0.1;
        }
        assert_eq!(count, 3, "one footstep per lap over three laps");
    }

    /// A frame hitch must not swallow events: a `dt` longer than the clip fires
    /// every event once per lap it covers.
    #[test]
    fn an_overshooting_step_fires_every_lap_it_crossed() {
        let anim = clip_with_events(&[0.2, 0.7], 1.0);
        // 2.5 seconds of a 1-second clip, starting at 0.
        let fired = events_in_window(&anim, 0.0, 2.5, true);
        assert_eq!(
            names(&fired),
            ["step0", "step1", "step0", "step1", "step0"],
            "two full laps plus the 0.5 remainder, in firing order"
        );
    }

    #[test]
    fn a_non_looping_clip_does_not_wrap() {
        let anim = Animation {
            looping: false,
            ..clip_with_events(&[0.2], 1.0)
        };
        assert_eq!(names(&events_in_window(&anim, 0.0, 0.5, false)), ["step0"]);
        // Past the end there is nothing more to fire.
        assert!(events_in_window(&anim, 1.0, 3.0, false).is_empty());
    }

    /// Scrubbing a timeline backwards should not replay sounds.
    #[test]
    fn a_backwards_step_fires_nothing() {
        let anim = clip_with_events(&[0.5], 1.0);
        assert!(events_in_window(&anim, 0.8, 0.2, true).is_empty());
    }

    #[test]
    fn events_fire_in_time_order_however_they_were_authored() {
        let mut anim = clip_with_events(&[0.8, 0.2, 0.5], 1.0);
        // Authored out of order on purpose.
        anim.events.sort_by(|a, b| b.time.total_cmp(&a.time));
        let fired = events_in_window(&anim, 0.0, 1.0, true);
        let times: Vec<f32> = fired.iter().map(|e| e.time).collect();
        assert_eq!(times, vec![0.2, 0.5, 0.8]);
    }

    #[test]
    fn a_clip_with_no_events_or_no_duration_fires_nothing() {
        let empty = clip_with_events(&[], 1.0);
        assert!(events_in_window(&empty, 0.0, 5.0, true).is_empty());
        let zero = clip_with_events(&[0.5], 0.0);
        assert!(events_in_window(&zero, 0.0, 5.0, true).is_empty());
    }
}

#[cfg(test)]
mod stepped_tests {
    use super::*;

    #[test]
    fn a_stepped_timeline_holds_setup_before_its_first_key() {
        let keys = vec![
            Key::stepped(0.9, "grind".to_string()),
            Key::stepped(2.2, "smile".to_string()),
        ];
        let mut hint = 0;
        assert_eq!(
            sample_stepped(&keys, 0.0, &mut hint),
            None,
            "before the first key the setup attachment stands"
        );
        assert_eq!(sample_stepped(&keys, 0.89, &mut hint), None);
        assert_eq!(
            sample_stepped(&keys, 0.9, &mut hint).as_deref(),
            Some("grind")
        );
        assert_eq!(
            sample_stepped(&keys, 3.0, &mut hint).as_deref(),
            Some("smile"),
            "after the last key it holds"
        );
    }

    #[test]
    fn a_key_at_zero_applies_from_the_first_frame() {
        let keys = vec![Key::stepped(0.0, 7u32)];
        let mut hint = 0;
        assert_eq!(sample_stepped(&keys, 0.0, &mut hint), Some(7));
    }
}
