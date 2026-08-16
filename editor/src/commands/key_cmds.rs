//! Keyframe mutations as undoable commands (T-201, PLAN §3.2, §2.7).
//!
//! # Undo strategy: snapshot the timelines vector
//!
//! Bone timelines hold at most a few dozen keys in practice, so every key command
//! snapshots the target animation's whole `Vec<Timeline>` before mutating and
//! restores it on revert. This trades a little memory for one uniform revert path
//! across all nine timeline variants — far less error-prone than nine bespoke
//! surgical inverses, and the size claim holds for any hand-authored rig. If a
//! generated animation ever makes this cost show up in a profile, narrow the
//! snapshot to the single edited timeline.
//!
//! # Addressing
//!
//! A key is edited via [`TimelineAddr`] (which timeline, by target + property)
//! plus a key index. `AddKey` samples the current pose so "set a key" captures
//! what the viewport shows, per the auto-key contract (T-202, PLAN §7.3).

use super::EditCommand;
use crate::doc::Document;
use ankhimate_core::animation::{Animation, Axis, Interp, Key, Timeline};
use ankhimate_core::ids::{AnimationId, BoneId, SlotId};

/// Which bone transform property a timeline drives. The dopesheet groups rows by
/// this; `AddKey` uses it to pick the timeline and the value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoneProperty {
    Translate,
    Rotate,
    Scale,
    Shear,
}

impl BoneProperty {
    pub fn label(self) -> &'static str {
        match self {
            BoneProperty::Translate => "translate",
            BoneProperty::Rotate => "rotate",
            BoneProperty::Scale => "scale",
            BoneProperty::Shear => "shear",
        }
    }
}

/// Names a single timeline within an animation by its driven target + property.
///
/// Timelines are stored in a `Vec`, but their *identity* is the target they
/// drive, not their index — an index shifts when another timeline is inserted.
#[derive(Debug, Clone, PartialEq)]
pub enum TimelineAddr {
    Bone {
        bone: BoneId,
        property: BoneProperty,
        /// Which axis, for a two-axis property. `None` for rotation, which has
        /// one.
        ///
        /// An address names one *track*, and translate/scale/shear are two
        /// tracks each — so the axis is part of the identity, not a detail of
        /// the value. Leaving it out is how a y key ends up written to the x
        /// track.
        axis: Option<Axis>,
    },
    SlotColor {
        slot: SlotId,
    },
    SlotAttachment {
        slot: SlotId,
    },
    /// Stepped visibility (T-505).
    SlotVisible {
        slot: SlotId,
    },
}

impl TimelineAddr {
    /// A stable, collision-free id for this address, for egui widget ids in the
    /// timeline. Derived from the target's slotmap key + a property tag — **not**
    /// from a per-frame pointer, which would clash and trip egui's overlap debug
    /// paint (the red boxes).
    pub fn stable_id(&self) -> u64 {
        use ankhimate_core::slotmap::Key;
        match self {
            TimelineAddr::Bone {
                bone,
                property,
                axis,
            } => {
                // Property in the low nibble, axis above it: two tracks of one
                // property must not collide, or egui hands them one widget id
                // and the second one's clicks vanish.
                let p = match property {
                    BoneProperty::Translate => 1,
                    BoneProperty::Rotate => 2,
                    BoneProperty::Scale => 3,
                    BoneProperty::Shear => 4,
                };
                let a = match axis {
                    None => 0,
                    Some(Axis::X) => 16,
                    Some(Axis::Y) => 32,
                };
                bone.data().as_ffi().wrapping_mul(64).wrapping_add(p + a)
            }
            TimelineAddr::SlotColor { slot } => slot.data().as_ffi() ^ 0xC010_0000_0000_0000,
            TimelineAddr::SlotAttachment { slot } => slot.data().as_ffi() ^ 0xA77A_0000_0000_0000,
            TimelineAddr::SlotVisible { slot } => slot.data().as_ffi() ^ 0x7157_0000_0000_0000,
        }
    }

    /// Does `timeline` drive the target this address names? Public so the
    /// auto-key path can check for an existing timeline before creating one.
    pub fn matches_timeline(&self, timeline: &Timeline) -> bool {
        self.matches(timeline)
    }

    /// Does `timeline` drive the target this address names?
    fn matches(&self, timeline: &Timeline) -> bool {
        match (self, timeline) {
            (
                TimelineAddr::Bone {
                    bone,
                    property: BoneProperty::Translate,
                    axis,
                },
                Timeline::BoneTranslate {
                    bone: b, axis: ax, ..
                },
            ) => bone == b && *axis == Some(*ax),
            (
                TimelineAddr::Bone {
                    bone,
                    property: BoneProperty::Rotate,
                    ..
                },
                Timeline::BoneRotate { bone: b, .. },
            ) => bone == b,
            (
                TimelineAddr::Bone {
                    bone,
                    property: BoneProperty::Scale,
                    axis,
                },
                Timeline::BoneScale {
                    bone: b, axis: ax, ..
                },
            ) => bone == b && *axis == Some(*ax),
            (
                TimelineAddr::Bone {
                    bone,
                    property: BoneProperty::Shear,
                    axis,
                },
                Timeline::BoneShear {
                    bone: b, axis: ax, ..
                },
            ) => bone == b && *axis == Some(*ax),
            (TimelineAddr::SlotColor { slot }, Timeline::SlotColor { slot: s, .. }) => slot == s,
            (TimelineAddr::SlotAttachment { slot }, Timeline::SlotAttachment { slot: s, .. }) => {
                slot == s
            }
            (TimelineAddr::SlotVisible { slot }, Timeline::SlotVisible { slot: s, .. }) => {
                slot == s
            }
            _ => false,
        }
    }

    /// An empty timeline of the right variant for this address.
    fn empty_timeline(&self) -> Timeline {
        match self {
            TimelineAddr::Bone {
                bone,
                property,
                axis,
            } => {
                // A two-axis property with no axis named would be ambiguous;
                // x is the one a caller means when it does not say.
                let axis = axis.unwrap_or(Axis::X);
                match property {
                    BoneProperty::Translate => Timeline::BoneTranslate {
                        bone: *bone,
                        axis,
                        keys: Vec::new(),
                    },
                    BoneProperty::Rotate => Timeline::BoneRotate {
                        bone: *bone,
                        keys: Vec::new(),
                    },
                    BoneProperty::Scale => Timeline::BoneScale {
                        bone: *bone,
                        axis,
                        keys: Vec::new(),
                    },
                    BoneProperty::Shear => Timeline::BoneShear {
                        bone: *bone,
                        axis,
                        keys: Vec::new(),
                    },
                }
            }
            TimelineAddr::SlotColor { slot } => Timeline::SlotColor {
                slot: *slot,
                keys: Vec::new(),
            },
            TimelineAddr::SlotAttachment { slot } => Timeline::SlotAttachment {
                slot: *slot,
                keys: Vec::new(),
            },
            TimelineAddr::SlotVisible { slot } => Timeline::SlotVisible {
                slot: *slot,
                keys: Vec::new(),
            },
        }
    }
}

/// Interpolation presets for the key context menu (T-203).
///
/// Presets are pure UI sugar: each returns a plain [`Interp`], and storage stays
/// `Interp` — nothing here is a new key type. The bezier handles are the standard
/// CSS-easing control points in normalized 0..1 time/value space, matching what
/// designers expect from other tools.
pub mod presets {
    use ankhimate_core::animation::Interp;

    fn bezier(ox: f32, oy: f32, ix: f32, iy: f32) -> Interp {
        Interp::Bezier {
            out_handle: glam::vec2(ox, oy),
            in_handle: glam::vec2(ix, iy),
        }
    }

    /// `(label, interp)` for every preset, in menu order.
    pub fn all() -> Vec<(&'static str, Interp)> {
        vec![
            ("Linear", Interp::Linear),
            ("Stepped", Interp::Stepped),
            ("Ease In", bezier(0.42, 0.0, 1.0, 1.0)),
            ("Ease Out", bezier(0.0, 0.0, 0.58, 1.0)),
            ("Ease In-Out", bezier(0.42, 0.0, 0.58, 1.0)),
            ("Sine In-Out", bezier(0.37, 0.0, 0.63, 1.0)),
            // "Snap" is a designer-facing alias for Stepped.
            ("Snap", Interp::Stepped),
        ]
    }
}

/// The value to key for a property, in the timeline's storage units — every
/// angle channel (rotate and shear alike) is **degrees**, per PLAN §2.7.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyValue {
    /// Every bone track drives one number now — an axis of a two-axis property,
    /// or rotation. The `Vec2` variant this replaced carried both axes of a
    /// paired key, which no longer exists.
    Scalar(f32),
    Color([f32; 4]),
    Visible(bool),
}

// ── Shared snapshot machinery ────────────────────────────────────────────────

/// Restore `anim`'s timelines from a snapshot, if the animation still exists.
fn restore(doc: &mut Document, anim: AnimationId, snapshot: &Option<Vec<Timeline>>) {
    if let (Some(a), Some(snap)) = (doc.animations.get_mut(anim), snapshot) {
        a.timelines = snap.clone();
    }
}

/// Snapshot `anim`'s timelines, or `None` if it is gone.
fn snapshot(doc: &Document, anim: AnimationId) -> Option<Vec<Timeline>> {
    doc.animations.get(anim).map(|a| a.timelines.clone())
}

/// Find (or create) the timeline `addr` names and return its index.
fn timeline_index(anim: &mut Animation, addr: &TimelineAddr) -> usize {
    if let Some(i) = anim.timelines.iter().position(|t| addr.matches(t)) {
        return i;
    }
    anim.timelines.push(addr.empty_timeline());
    anim.timelines.len() - 1
}

/// Insert `key` into a key list at its sorted time, replacing an existing key at
/// the same time. Returns the index it landed at.
fn insert_key<T>(keys: &mut Vec<Key<T>>, key: Key<T>) -> usize {
    match keys.binary_search_by(|k| k.time.total_cmp(&key.time)) {
        Ok(i) => {
            keys[i] = key;
            i
        }
        Err(i) => {
            keys.insert(i, key);
            i
        }
    }
}

// ── AddKey ───────────────────────────────────────────────────────────────────

/// Insert or overwrite a key at `time` on the timeline `addr` names.
///
/// If the timeline does not exist yet it is created. A key already at `time` is
/// replaced (re-keying the current frame), matching the auto-key contract.
pub struct AddKey {
    anim: AnimationId,
    addr: TimelineAddr,
    time: f32,
    value: KeyValue,
    interp: Interp,
    before: Option<Vec<Timeline>>,
}

impl AddKey {
    pub fn new(
        anim: AnimationId,
        addr: TimelineAddr,
        time: f32,
        value: KeyValue,
        interp: Interp,
    ) -> Self {
        Self {
            anim,
            addr,
            time,
            value,
            interp,
            before: None,
        }
    }
}

impl EditCommand for AddKey {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        let idx = timeline_index(anim, &self.addr);
        let time = self.time;
        let interp = self.interp;
        match (&mut anim.timelines[idx], self.value) {
            (Timeline::BoneTranslate { keys, .. }, KeyValue::Scalar(v))
            | (Timeline::BoneScale { keys, .. }, KeyValue::Scalar(v))
            | (Timeline::BoneShear { keys, .. }, KeyValue::Scalar(v))
            | (Timeline::BoneRotate { keys, .. }, KeyValue::Scalar(v)) => {
                insert_key(
                    keys,
                    Key {
                        time,
                        value: v,
                        interp,
                    },
                );
            }
            (Timeline::SlotVisible { keys, .. }, KeyValue::Visible(v)) => {
                insert_key(
                    keys,
                    Key {
                        time,
                        value: v,
                        // Always stepped: there is no halfway between shown and
                        // hidden, and interpolating one would fade instead of cut.
                        interp: ankhimate_core::animation::Interp::Stepped,
                    },
                );
            }
            (Timeline::SlotColor { keys, .. }, KeyValue::Color(v)) => {
                insert_key(
                    keys,
                    Key {
                        time,
                        value: v,
                        interp,
                    },
                );
            }
            _ => {
                // Value/variant mismatch: leave the timeline untouched. The UI
                // only ever pairs matching addr+value, so this is unreachable in
                // practice; not panicking keeps a misuse from taking the app down.
            }
        }
        // Growing content must not exceed the clip length, or the new key would be
        // unreachable by the playhead.
        if let Some(anim) = doc.animations.get_mut(self.anim) {
            anim.duration = anim.duration.max(anim.content_duration());
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn label(&self) -> &str {
        "Add Key"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Generic key selection edits (move / delete / interp) ─────────────────────

/// A key identified by the timeline it lives on and its position in that
/// timeline's key list.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRef {
    pub addr: TimelineAddr,
    pub index: usize,
}

/// A mutable view of a key's time and interp, the two fields generic edits touch.
struct KeyTimeInterp<'a> {
    time: &'a mut f32,
    interp: &'a mut Interp,
}

macro_rules! with_key_arm {
    ($keys:expr, $index:expr, $f:expr) => {
        if let Some(k) = $keys.get_mut($index) {
            $f(&mut KeyTimeInterp {
                time: &mut k.time,
                interp: &mut k.interp,
            });
        }
    };
}

fn with_key(timeline: &mut Timeline, index: usize, f: &mut impl FnMut(&mut KeyTimeInterp)) {
    match timeline {
        Timeline::BoneTranslate { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::BoneRotate { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::BoneScale { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::BoneShear { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::SlotVisible { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::SlotColor { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::SlotAttachment { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::DrawOrder { keys } => with_key_arm!(keys, index, f),
        Timeline::IkMix { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::IkBendDirection { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::IkSoftness { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::TransformConstraintMix { keys, .. } => with_key_arm!(keys, index, f),
        Timeline::Deform { keys, .. } => with_key_arm!(keys, index, f),
    }
}

macro_rules! sort_arm {
    ($keys:expr) => {
        $keys.sort_by(|a, b| a.time.total_cmp(&b.time))
    };
}

fn sort_timeline(timeline: &mut Timeline) {
    match timeline {
        Timeline::BoneTranslate { keys, .. } => sort_arm!(keys),
        Timeline::BoneRotate { keys, .. } => sort_arm!(keys),
        Timeline::BoneScale { keys, .. } => sort_arm!(keys),
        Timeline::BoneShear { keys, .. } => sort_arm!(keys),
        Timeline::SlotVisible { keys, .. } => sort_arm!(keys),
        Timeline::SlotColor { keys, .. } => sort_arm!(keys),
        Timeline::SlotAttachment { keys, .. } => sort_arm!(keys),
        Timeline::DrawOrder { keys } => sort_arm!(keys),
        Timeline::IkMix { keys, .. } => sort_arm!(keys),
        Timeline::IkBendDirection { keys, .. } => sort_arm!(keys),
        Timeline::IkSoftness { keys, .. } => sort_arm!(keys),
        Timeline::TransformConstraintMix { keys, .. } => sort_arm!(keys),
        Timeline::Deform { keys, .. } => sort_arm!(keys),
    }
}

/// Apply a per-key mutation to every referenced key, then re-sort each touched
/// timeline so key lists stay time-ordered.
fn edit_keys(anim: &mut Animation, refs: &[KeyRef], mut f: impl FnMut(&mut KeyTimeInterp)) {
    use std::collections::BTreeSet;
    let mut touched: BTreeSet<usize> = BTreeSet::new();
    for r in refs {
        if let Some(idx) = anim.timelines.iter().position(|t| r.addr.matches(t)) {
            with_key(&mut anim.timelines[idx], r.index, &mut f);
            touched.insert(idx);
        }
    }
    for idx in touched {
        sort_timeline(&mut anim.timelines[idx]);
    }
}

/// Shift a set of keys in time by `delta` seconds (a dopesheet horizontal drag).
///
/// Coalesces with successive `MoveKeys` on the same selection so a drag is one
/// undo step (PLAN §3.2 drag coalescing).
pub struct MoveKeys {
    anim: AnimationId,
    refs: Vec<KeyRef>,
    delta: f32,
    before: Option<Vec<Timeline>>,
}

impl MoveKeys {
    pub fn new(anim: AnimationId, refs: Vec<KeyRef>, delta: f32) -> Self {
        Self {
            anim,
            refs,
            delta,
            before: None,
        }
    }
}

impl EditCommand for MoveKeys {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        let delta = self.delta;
        edit_keys(anim, &self.refs, |k| *k.time = (*k.time + delta).max(0.0));
        anim.duration = anim.duration.max(anim.content_duration());
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        // Fold a continuing drag on the same keys into this step. The snapshot in
        // `self.before` already predates the whole drag, so only the delta grows.
        if let Some(other) = next.as_any().downcast_ref::<MoveKeys>()
            && other.anim == self.anim
            && other.refs == self.refs
        {
            self.delta += other.delta;
            return true;
        }
        false
    }

    fn label(&self) -> &str {
        "Move Keys"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete a set of keys (drag off-sheet, or context-menu delete). Empty
/// timelines left behind are removed so the dopesheet does not show dead rows.
pub struct DeleteKeys {
    anim: AnimationId,
    refs: Vec<KeyRef>,
    before: Option<Vec<Timeline>>,
}

impl DeleteKeys {
    pub fn new(anim: AnimationId, refs: Vec<KeyRef>) -> Self {
        Self {
            anim,
            refs,
            before: None,
        }
    }
}

macro_rules! remove_arm {
    ($keys:expr, $indices:expr) => {{
        // Remove from the back so earlier indices stay valid.
        for &i in $indices.iter().rev() {
            if i < $keys.len() {
                $keys.remove(i);
            }
        }
    }};
}

impl EditCommand for DeleteKeys {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        // Group indices per timeline so removals within one list are done together.
        use std::collections::BTreeMap;
        let mut per_timeline: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for r in &self.refs {
            if let Some(idx) = anim.timelines.iter().position(|t| r.addr.matches(t)) {
                per_timeline.entry(idx).or_default().push(r.index);
            }
        }
        for (idx, mut indices) in per_timeline {
            indices.sort_unstable();
            match &mut anim.timelines[idx] {
                Timeline::BoneTranslate { keys, .. } => remove_arm!(keys, indices),
                Timeline::BoneRotate { keys, .. } => remove_arm!(keys, indices),
                Timeline::BoneScale { keys, .. } => remove_arm!(keys, indices),
                Timeline::BoneShear { keys, .. } => remove_arm!(keys, indices),
                Timeline::SlotVisible { keys, .. } => remove_arm!(keys, indices),
                Timeline::SlotColor { keys, .. } => remove_arm!(keys, indices),
                Timeline::SlotAttachment { keys, .. } => remove_arm!(keys, indices),
                Timeline::DrawOrder { keys } => remove_arm!(keys, indices),
                Timeline::IkMix { keys, .. } => remove_arm!(keys, indices),
                Timeline::IkBendDirection { keys, .. } => remove_arm!(keys, indices),
                Timeline::IkSoftness { keys, .. } => remove_arm!(keys, indices),
                Timeline::TransformConstraintMix { keys, .. } => remove_arm!(keys, indices),
                Timeline::Deform { keys, .. } => remove_arm!(keys, indices),
            }
        }
        anim.timelines.retain(|t| !t.is_empty());
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn label(&self) -> &str {
        "Delete Keys"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set the interpolation of a set of keys (context-menu preset, T-203).
pub struct SetInterp {
    anim: AnimationId,
    refs: Vec<KeyRef>,
    interp: Interp,
    before: Option<Vec<Timeline>>,
}

impl SetInterp {
    pub fn new(anim: AnimationId, refs: Vec<KeyRef>, interp: Interp) -> Self {
        Self {
            anim,
            refs,
            interp,
            before: None,
        }
    }
}

impl EditCommand for SetInterp {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        let interp = self.interp;
        edit_keys(anim, &self.refs, |k| *k.interp = interp);
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn label(&self) -> &str {
        "Set Interpolation"
    }

    /// Merge consecutive edits to the same keys, so dragging a bezier handle in
    /// the graph is one undo step rather than one per pixel of mouse travel.
    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        let Some(other) = next.as_any().downcast_ref::<SetInterp>() else {
            return false;
        };
        if other.anim != self.anim || other.refs != self.refs {
            return false;
        }
        self.interp = other.interp;
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Draw order key (T-204) ───────────────────────────────────────────────────

/// Write a `DrawOrder` key at `time` capturing `target_order` as per-slot offsets
/// from the setup order (PLAN §2.3). Only slots whose position changed get an
/// entry, so the key is minimal and reads as "this slot moved +2".
pub struct AddDrawOrderKey {
    anim: AnimationId,
    time: f32,
    /// The desired full slot order at this time.
    target_order: Vec<ankhimate_core::ids::SlotId>,
    /// The setup order to diff against.
    setup_order: Vec<ankhimate_core::ids::SlotId>,
    before: Option<Vec<Timeline>>,
}

impl AddDrawOrderKey {
    pub fn new(
        anim: AnimationId,
        time: f32,
        target_order: Vec<ankhimate_core::ids::SlotId>,
        setup_order: Vec<ankhimate_core::ids::SlotId>,
    ) -> Self {
        Self {
            anim,
            time,
            target_order,
            setup_order,
            before: None,
        }
    }

    /// `(slot, offset)` for every slot whose index differs from setup.
    fn offsets(&self) -> Vec<(ankhimate_core::ids::SlotId, i32)> {
        self.target_order
            .iter()
            .enumerate()
            .filter_map(|(target_idx, slot)| {
                let setup_idx = self.setup_order.iter().position(|s| s == slot)?;
                let delta = target_idx as i32 - setup_idx as i32;
                (delta != 0).then_some((*slot, delta))
            })
            .collect()
    }
}

impl EditCommand for AddDrawOrderKey {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let offsets = self.offsets();
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        // Find or create the single DrawOrder timeline.
        let idx = match anim
            .timelines
            .iter()
            .position(|t| matches!(t, Timeline::DrawOrder { .. }))
        {
            Some(i) => i,
            None => {
                anim.timelines
                    .push(Timeline::DrawOrder { keys: Vec::new() });
                anim.timelines.len() - 1
            }
        };
        if let Timeline::DrawOrder { keys } = &mut anim.timelines[idx] {
            insert_key(
                keys,
                Key {
                    time: self.time,
                    value: offsets,
                    // Draw order is not blendable; stepped by construction.
                    interp: Interp::Stepped,
                },
            );
        }
        anim.duration = anim.duration.max(anim.content_duration());
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn label(&self) -> &str {
        "Key Draw Order"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Slot attachment key (T-205) ──────────────────────────────────────────────

/// Write a `SlotAttachment` key at `time`: which attachment **name** the slot
/// shows from this time on. Attachment names cannot blend, so the key is stepped
/// by construction (PLAN §2.7).
pub struct AddAttachmentKey {
    anim: AnimationId,
    slot: SlotId,
    time: f32,
    value: Option<String>,
    before: Option<Vec<Timeline>>,
}

impl AddAttachmentKey {
    pub fn new(anim: AnimationId, slot: SlotId, time: f32, value: Option<String>) -> Self {
        Self {
            anim,
            slot,
            time,
            value,
            before: None,
        }
    }
}

impl EditCommand for AddAttachmentKey {
    fn apply(&mut self, doc: &mut Document) {
        self.before = snapshot(doc, self.anim);
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        let idx =
            match anim.timelines.iter().position(
                |t| matches!(t, Timeline::SlotAttachment { slot, .. } if *slot == self.slot),
            ) {
                Some(i) => i,
                None => {
                    anim.timelines.push(Timeline::SlotAttachment {
                        slot: self.slot,
                        keys: Vec::new(),
                    });
                    anim.timelines.len() - 1
                }
            };
        if let Timeline::SlotAttachment { keys, .. } = &mut anim.timelines[idx] {
            insert_key(
                keys,
                Key {
                    time: self.time,
                    value: self.value.clone(),
                    interp: Interp::Stepped,
                },
            );
        }
        anim.duration = anim.duration.max(anim.content_duration());
    }

    fn revert(&mut self, doc: &mut Document) {
        restore(doc, self.anim, &self.before);
    }

    fn label(&self) -> &str {
        "Key Attachment"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Key a mesh's deformed shape at a time (T-404).
///
/// Values are **offsets from the setup vertices**, like every other bone
/// channel: the mesh keeps one authored shape and animations say how far each
/// vertex strays from it, so re-posing the setup mesh moves every clip with it.
///
/// Its own command because the payload is a vertex list, not a scalar or a
/// `Vec2` — `KeyValue` would have to grow a variant that only this uses.
pub struct AddDeformKey {
    anim: AnimationId,
    slot: SlotId,
    attachment: String,
    time: f32,
    offsets: Vec<glam::Vec2>,
    before: Option<Vec<Timeline>>,
}

impl AddDeformKey {
    pub fn new(
        anim: AnimationId,
        slot: SlotId,
        attachment: impl Into<String>,
        time: f32,
        offsets: Vec<glam::Vec2>,
    ) -> Self {
        Self {
            anim,
            slot,
            attachment: attachment.into(),
            time,
            offsets,
            before: None,
        }
    }
}

impl EditCommand for AddDeformKey {
    fn apply(&mut self, doc: &mut Document) {
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(anim.timelines.clone());
        }

        let existing = anim.timelines.iter_mut().find(|t| {
            matches!(
                t,
                Timeline::Deform { slot, attachment, .. }
                    if *slot == self.slot && *attachment == self.attachment
            )
        });
        let timeline = match existing {
            Some(timeline) => timeline,
            None => {
                anim.timelines.push(Timeline::Deform {
                    slot: self.slot,
                    attachment: self.attachment.clone(),
                    keys: Vec::new(),
                });
                anim.timelines.last_mut().expect("just pushed")
            }
        };
        let Timeline::Deform { keys, .. } = timeline else {
            return;
        };

        match keys.iter_mut().find(|k| (k.time - self.time).abs() < 1e-4) {
            // Re-keying the same frame replaces it, rather than stacking two
            // keys the sampler would pick between arbitrarily.
            Some(key) => key.value = self.offsets.clone(),
            None => keys.push(Key::linear(self.time, self.offsets.clone())),
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(anim)) = (self.before.take(), doc.animations.get_mut(self.anim))
        {
            anim.timelines = before;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<AddDeformKey>() {
            // A vertex drag re-keys the same frame every mouse-move.
            Some(other)
                if other.anim == self.anim
                    && other.slot == self.slot
                    && other.attachment == self.attachment
                    && (other.time - self.time).abs() < 1e-4 =>
            {
                self.offsets = other.offsets.clone();
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Key Deform"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Whole-clip pose tools (T-211) ───────────────────────────────────────────

/// Every key time in a clip, transformed by `f`, with the duration following.
///
/// Retiming and offsetting are the same operation with a different function, and
/// both need the identical nine-variant walk — writing it twice would be writing
/// the second one wrong.
fn map_key_times(anim: &mut Animation, f: impl Fn(f32) -> f32) {
    macro_rules! remap {
        ($keys:expr) => {
            for key in $keys.iter_mut() {
                key.time = f(key.time).max(0.0);
            }
        };
    }
    for timeline in &mut anim.timelines {
        match timeline {
            Timeline::BoneTranslate { keys, .. } => remap!(keys),
            Timeline::BoneRotate { keys, .. } => remap!(keys),
            Timeline::BoneScale { keys, .. } => remap!(keys),
            Timeline::BoneShear { keys, .. } => remap!(keys),
            Timeline::SlotVisible { keys, .. } => remap!(keys),
            Timeline::SlotColor { keys, .. } => remap!(keys),
            Timeline::SlotAttachment { keys, .. } => remap!(keys),
            Timeline::DrawOrder { keys } => remap!(keys),
            Timeline::IkMix { keys, .. } => remap!(keys),
            Timeline::IkBendDirection { keys, .. } => remap!(keys),
            Timeline::IkSoftness { keys, .. } => remap!(keys),
            Timeline::TransformConstraintMix { keys, .. } => remap!(keys),
            Timeline::Deform { keys, .. } => remap!(keys),
        }
    }
    anim.duration = f(anim.duration).max(0.0);
}

/// Scale a clip's timing, or shift every key by a fixed offset (T-211).
///
/// Snapshots the timelines rather than inverting the mapping: a key clamped at
/// zero cannot be un-clamped, so an arithmetic inverse would quietly lose the
/// keys a negative offset had pushed off the start.
pub struct RetimeAnimation {
    anim: AnimationId,
    factor: f32,
    offset: f32,
    before: Option<(Vec<Timeline>, f32)>,
    label: &'static str,
}

impl RetimeAnimation {
    /// Multiply every key time (and the duration) by `factor`.
    pub fn scaled(anim: AnimationId, factor: f32) -> Self {
        Self {
            anim,
            factor: factor.max(0.01),
            offset: 0.0,
            before: None,
            label: "Scale Timing",
        }
    }

    /// Shift every key by `offset` seconds; the duration is unchanged.
    pub fn offset(anim: AnimationId, offset: f32) -> Self {
        Self {
            anim,
            factor: 1.0,
            offset,
            before: None,
            label: "Offset Keys",
        }
    }
}

impl EditCommand for RetimeAnimation {
    fn apply(&mut self, doc: &mut Document) {
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some((anim.timelines.clone(), anim.duration));
        }
        let (factor, offset) = (self.factor, self.offset);
        let duration = anim.duration;
        map_key_times(anim, |t| t * factor + offset);
        if offset != 0.0 {
            // A shift moves the content, not the clip's length.
            anim.duration = duration;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some((timelines, duration)), Some(anim)) =
            (self.before.take(), doc.animations.get_mut(self.anim))
        {
            anim.timelines = timelines;
            anim.duration = duration;
        }
    }

    fn label(&self) -> &str {
        self.label
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Drop every timeline driving the given bones (T-211).
///
/// The "start this limb over" tool. Slot timelines are left alone: a slot is not
/// a bone, and taking its colour animation away because its bone was cleared
/// would be a surprise.
pub struct ClearBoneAnimation {
    anim: AnimationId,
    bones: Vec<BoneId>,
    before: Option<Vec<Timeline>>,
}

impl ClearBoneAnimation {
    pub fn new(anim: AnimationId, bones: Vec<BoneId>) -> Self {
        Self {
            anim,
            bones,
            before: None,
        }
    }
}

impl EditCommand for ClearBoneAnimation {
    fn apply(&mut self, doc: &mut Document) {
        let Some(anim) = doc.animations.get_mut(self.anim) else {
            return;
        };
        if self.before.is_none() {
            self.before = Some(anim.timelines.clone());
        }
        let bones = &self.bones;
        anim.timelines.retain(|t| match t {
            Timeline::BoneTranslate { bone, .. }
            | Timeline::BoneRotate { bone, .. }
            | Timeline::BoneScale { bone, .. }
            | Timeline::BoneShear { bone, .. } => !bones.contains(bone),
            _ => true,
        });
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(timelines), Some(anim)) =
            (self.before.take(), doc.animations.get_mut(self.anim))
        {
            anim.timelines = timelines;
        }
    }

    fn label(&self) -> &str {
        "Clear Bone Animation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ── Animation clip create/delete (kept from the T-107 stub) ──────────────────

/// Create an animation clip.
pub struct CreateAnimation {
    animation: Animation,
    created: Option<AnimationId>,
}

impl CreateAnimation {
    pub fn new(name: impl Into<String>, duration: f32) -> Self {
        Self {
            animation: Animation::new(name, duration),
            created: None,
        }
    }

    pub fn created_id(&self) -> Option<AnimationId> {
        self.created
    }
}

impl EditCommand for CreateAnimation {
    fn apply(&mut self, doc: &mut Document) {
        self.created = Some(doc.animations.insert(self.animation.clone()));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.animations.remove(id);
        }
    }

    fn label(&self) -> &str {
        "Create Animation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Rename an animation clip.
///
/// Names are the clip's identity on disk (ADR 0004), so a collision would make
/// one of the two unreachable after a round trip — it is refused instead.
pub struct RenameAnimation {
    target: AnimationId,
    new_name: String,
    before: Option<String>,
}

impl RenameAnimation {
    pub fn new(target: AnimationId, new_name: impl Into<String>) -> Self {
        Self {
            target,
            new_name: new_name.into(),
            before: None,
        }
    }

    fn taken(doc: &Document, target: AnimationId, name: &str) -> bool {
        doc.animations
            .iter()
            .any(|(id, a)| id != target && a.name == name)
    }
}

impl EditCommand for RenameAnimation {
    fn apply(&mut self, doc: &mut Document) {
        if Self::taken(doc, self.target, &self.new_name) {
            return;
        }
        if let Some(anim) = doc.animations.get_mut(self.target) {
            if self.before.is_none() {
                self.before = Some(anim.name.clone());
            }
            anim.name = self.new_name.clone();
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(anim)) =
            (self.before.take(), doc.animations.get_mut(self.target))
        {
            anim.name = before;
        }
    }

    fn label(&self) -> &str {
        "Rename Animation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Deep-copy an animation, timelines and all.
pub struct DuplicateAnimation {
    source: AnimationId,
    created: Option<AnimationId>,
}

impl DuplicateAnimation {
    pub fn new(source: AnimationId) -> Self {
        Self {
            source,
            created: None,
        }
    }

    pub fn created_id(&self) -> Option<AnimationId> {
        self.created
    }
}

impl EditCommand for DuplicateAnimation {
    fn apply(&mut self, doc: &mut Document) {
        let Some(source) = doc.animations.get(self.source).cloned() else {
            return;
        };
        let mut copy = source;
        let mut n = 2;
        copy.name = loop {
            let candidate = format!("{}_{n}", copy.name);
            if !doc.animations.values().any(|a| a.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        self.created = Some(doc.animations.insert(copy));
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(id) = self.created.take() {
            doc.animations.remove(id);
        }
    }

    fn label(&self) -> &str {
        "Duplicate Animation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Set a clip's duration and loop flag.
///
/// Shortening is **non-destructive**: keys past the new end are kept, not
/// trimmed. Losing work to a mistyped number would be unforgivable; the
/// diagnostics pass (T-702) flags out-of-range keys instead.
pub struct SetAnimationMeta {
    target: AnimationId,
    duration: f32,
    looping: bool,
    before: Option<(f32, bool)>,
}

impl SetAnimationMeta {
    pub fn new(target: AnimationId, duration: f32, looping: bool) -> Self {
        Self {
            target,
            duration: duration.max(0.0),
            looping,
            before: None,
        }
    }
}

impl EditCommand for SetAnimationMeta {
    fn apply(&mut self, doc: &mut Document) {
        if let Some(anim) = doc.animations.get_mut(self.target) {
            if self.before.is_none() {
                self.before = Some((anim.duration, anim.looping));
            }
            anim.duration = self.duration;
            anim.looping = self.looping;
        }
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some((duration, looping)), Some(anim)) =
            (self.before.take(), doc.animations.get_mut(self.target))
        {
            anim.duration = duration;
            anim.looping = looping;
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<SetAnimationMeta>() {
            Some(other) if other.target == self.target => {
                self.duration = other.duration;
                self.looping = other.looping;
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Animation Settings"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Delete an animation clip.
pub struct DeleteAnimation {
    target: AnimationId,
    removed: Option<Animation>,
    restored: Option<AnimationId>,
}

impl DeleteAnimation {
    pub fn new(target: AnimationId) -> Self {
        Self {
            target,
            removed: None,
            restored: None,
        }
    }
}

impl EditCommand for DeleteAnimation {
    fn apply(&mut self, doc: &mut Document) {
        let id = self.restored.take().unwrap_or(self.target);
        self.removed = doc.animations.remove(id);
    }

    fn revert(&mut self, doc: &mut Document) {
        if let Some(anim) = self.removed.take() {
            self.restored = Some(doc.animations.insert(anim));
        }
    }

    fn label(&self) -> &str {
        "Delete Animation"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::History;

    fn rotate_addr(bone: BoneId) -> TimelineAddr {
        TimelineAddr::Bone {
            bone,
            property: BoneProperty::Rotate,
            axis: None,
        }
    }

    fn anim_with_clip(doc: &mut Document) -> AnimationId {
        doc.animations.insert(Animation::new("clip", 1.0))
    }

    // ── Animation manager (T-208) ────────────────────────────────────────

    /// T-208 acceptance: duplicating deep-copies, so editing the copy leaves the
    /// original untouched.
    #[test]
    fn duplicate_is_a_deep_copy() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                0.0,
                KeyValue::Scalar(10.0),
                Interp::Linear,
            )),
            &mut doc,
        );

        let mut cmd = DuplicateAnimation::new(anim);
        cmd.apply(&mut doc);
        let copy = cmd.created_id().expect("copy created");
        assert_eq!(doc.animations[copy].name, "clip_2");
        assert_eq!(doc.animations[copy].timelines.len(), 1);

        // Edit the copy; the original must not move.
        history.push(
            Box::new(AddKey::new(
                copy,
                rotate_addr(bone),
                0.5,
                KeyValue::Scalar(90.0),
                Interp::Linear,
            )),
            &mut doc,
        );
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert_eq!(keys.len(), 1, "original untouched by edits to the copy")
            }
            other => panic!("expected a rotate timeline, got {other:?}"),
        }

        cmd.revert(&mut doc);
        assert!(doc.animations.get(copy).is_none());
    }

    #[test]
    fn rename_refuses_a_taken_name_and_undoes() {
        let mut doc = Document::new();
        let a = doc.animations.insert(Animation::new("walk", 1.0));
        doc.animations.insert(Animation::new("idle", 1.0));
        let mut history = History::default();

        history.push(Box::new(RenameAnimation::new(a, "idle")), &mut doc);
        assert_eq!(doc.animations[a].name, "walk", "collision refused");

        history.push(Box::new(RenameAnimation::new(a, "run")), &mut doc);
        assert_eq!(doc.animations[a].name, "run");
        history.undo(&mut doc);
        assert_eq!(doc.animations[a].name, "walk");
    }

    /// Shortening a clip keeps out-of-range keys — losing authored work to a
    /// mistyped duration would be unforgivable.
    #[test]
    fn shortening_a_clip_keeps_its_keys() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                0.9,
                KeyValue::Scalar(45.0),
                Interp::Linear,
            )),
            &mut doc,
        );

        history.push(Box::new(SetAnimationMeta::new(anim, 0.5, false)), &mut doc);
        assert_eq!(doc.animations[anim].duration, 0.5);
        assert!(!doc.animations[anim].looping);
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert_eq!(keys.len(), 1, "the key past the new end survives");
                assert_eq!(keys[0].time, 0.9);
            }
            other => panic!("expected a rotate timeline, got {other:?}"),
        }

        history.undo(&mut doc);
        assert_eq!(doc.animations[anim].duration, 1.0);
        assert!(doc.animations[anim].looping);
    }

    #[test]
    fn duration_edits_merge_into_one_step() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let mut history = History::default();
        let before = history.undo_depth();

        for frames in 1..=5 {
            history.push(
                Box::new(SetAnimationMeta::new(anim, frames as f32 / 30.0, true)),
                &mut doc,
            );
        }
        assert_eq!(history.undo_depth(), before + 1);
        history.undo(&mut doc);
        assert_eq!(doc.animations[anim].duration, 1.0, "back to the original");
    }

    #[test]
    fn add_key_creates_timeline_and_key() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();

        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                0.5,
                KeyValue::Scalar(30.0),
                Interp::Linear,
            )),
            &mut doc,
        );

        let a = &doc.animations[anim];
        assert_eq!(a.timelines.len(), 1);
        match &a.timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0].time, 0.5);
                assert_eq!(keys[0].value, 30.0);
            }
            _ => panic!("wrong timeline variant"),
        }

        history.undo(&mut doc);
        assert!(doc.animations[anim].timelines.is_empty(), "undo removed it");
    }

    #[test]
    fn add_key_overwrites_same_time() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();

        for v in [10.0, 20.0] {
            history.push(
                Box::new(AddKey::new(
                    anim,
                    rotate_addr(bone),
                    0.0,
                    KeyValue::Scalar(v),
                    Interp::Linear,
                )),
                &mut doc,
            );
        }
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert_eq!(keys.len(), 1, "same time replaced, not appended");
                assert_eq!(keys[0].value, 20.0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn add_key_grows_duration() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                2.5,
                KeyValue::Scalar(0.0),
                Interp::Linear,
            )),
            &mut doc,
        );
        assert!(doc.animations[anim].duration >= 2.5);
    }

    #[test]
    fn move_keys_shifts_time_and_merges() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        for t in [0.0, 0.5] {
            history.push(
                Box::new(AddKey::new(
                    anim,
                    rotate_addr(bone),
                    t,
                    KeyValue::Scalar(0.0),
                    Interp::Linear,
                )),
                &mut doc,
            );
        }
        let refs = vec![KeyRef {
            addr: rotate_addr(bone),
            index: 1,
        }];
        let before = history.undo_depth();
        // A drag: two MoveKeys on the same ref, +0.1 then +0.1.
        history.push(Box::new(MoveKeys::new(anim, refs.clone(), 0.1)), &mut doc);
        history.push(Box::new(MoveKeys::new(anim, refs, 0.1)), &mut doc);
        assert_eq!(history.undo_depth(), before + 1, "drag merged to one step");
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert!((keys[1].time - 0.7).abs() < 1e-5);
            }
            _ => panic!(),
        }
        history.undo(&mut doc);
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => assert_eq!(keys[1].time, 0.5),
            _ => panic!(),
        }
    }

    #[test]
    fn delete_keys_removes_and_prunes_empty_timeline() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                0.0,
                KeyValue::Scalar(0.0),
                Interp::Linear,
            )),
            &mut doc,
        );
        history.push(
            Box::new(DeleteKeys::new(
                anim,
                vec![KeyRef {
                    addr: rotate_addr(bone),
                    index: 0,
                }],
            )),
            &mut doc,
        );
        assert!(
            doc.animations[anim].timelines.is_empty(),
            "empty row pruned"
        );
        history.undo(&mut doc);
        assert_eq!(doc.animations[anim].timelines.len(), 1, "key came back");
    }

    #[test]
    fn set_interp_changes_and_reverts() {
        let mut doc = Document::new();
        let anim = anim_with_clip(&mut doc);
        let bone = BoneId::default();
        let mut history = History::default();
        history.push(
            Box::new(AddKey::new(
                anim,
                rotate_addr(bone),
                0.0,
                KeyValue::Scalar(0.0),
                Interp::Linear,
            )),
            &mut doc,
        );
        let refs = vec![KeyRef {
            addr: rotate_addr(bone),
            index: 0,
        }];
        history.push(
            Box::new(SetInterp::new(anim, refs, Interp::Stepped)),
            &mut doc,
        );
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => assert_eq!(keys[0].interp, Interp::Stepped),
            _ => panic!(),
        }
        history.undo(&mut doc);
        match &doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => assert_eq!(keys[0].interp, Interp::Linear),
            _ => panic!(),
        }
    }

    #[test]
    fn create_animation_roundtrips() {
        let mut doc = Document::new();
        let mut history = History::default();
        history.push(Box::new(CreateAnimation::new("walk", 1.0)), &mut doc);
        assert_eq!(doc.animations.len(), 1);
        history.undo(&mut doc);
        assert_eq!(doc.animations.len(), 0);
        history.redo(&mut doc);
        assert_eq!(doc.animations.len(), 1);
    }

    #[test]
    fn delete_animation_preserves_timelines_through_undo() {
        let mut doc = Document::new();
        let mut anim = Animation::new("walk", 2.0);
        anim.timelines.push(Timeline::BoneRotate {
            bone: Default::default(),
            keys: vec![Key::linear(0.0, 45.0)],
        });
        let id = doc.animations.insert(anim);

        let mut history = History::default();
        history.push(Box::new(DeleteAnimation::new(id)), &mut doc);
        assert_eq!(doc.animations.len(), 0);
        history.undo(&mut doc);
        let restored = doc.animations.values().next().expect("restored");
        assert_eq!(restored.name, "walk");
        assert_eq!(restored.timelines.len(), 1);
    }
}
