//! Where an edit goes (T-207, ADR 0006).
//!
//! Panels never construct a setup command or a key command directly. They state
//! *what the user did* as an [`EditIntent`] and this module decides what that
//! means in the current [`WorkMode`]:
//!
//! ```text
//! Setup    → mutate the Skeleton's setup data (SetBoneTransform, SetSlotColor, …)
//! Animate  → write keys on the active animation at the playhead (AddKey, …)
//! ```
//!
//! Centralizing the branch is the point. The pre-T-207 editor repeated an
//! `auto_key && active_animation && playhead > 0.0` test in four places, each
//! subtly different — the draw-order panel and the pose commit disagreed about
//! what `t = 0` meant. One router, one rule, and new intents inherit the baseline
//! and locking behavior for free.
//!
//! Key values are **offsets from the setup pose** for bone timelines (PLAN §2.7)
//! and absolute values for slot timelines.

use crate::session::Session;
use ankhimate_core::animation::{Axis, Interp, Timeline};
use ankhimate_core::ids::{AnimationId, BoneId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_document::commands::EditCommand;
use ankhimate_document::commands::bone_cmds::SetBoneTransform;
use ankhimate_document::commands::key_cmds::{
    AddAttachmentKey, AddDrawOrderKey, AddKey, BoneProperty, KeyValue, TimelineAddr,
};
use ankhimate_document::commands::slot_cmds::{SetDrawOrder, SetSlotAttachment, SetSlotColor};
use ankhimate_document::doc::Document;

/// A user edit, stated in terms of the value they produced — not the command it
/// should become.
pub enum EditIntent {
    /// A posed bone: the local transform the user dragged or typed.
    BoneLocal {
        bone: BoneId,
        value: Transform,
    },
    SlotColor {
        slot: SlotId,
        value: [f32; 4],
    },
    SlotAttachment {
        slot: SlotId,
        value: Option<String>,
    },
    /// A full back-to-front slot order.
    DrawOrder {
        order: Vec<SlotId>,
    },
}

/// What the router decided.
pub enum Routed {
    /// Dispatch these, in order. Empty means "nothing to do" (a no-op edit).
    Commands(Vec<Box<dyn EditCommand>>),
    /// Animate mode with auto-key off: hold the value as a viewport preview
    /// until the user presses `K`. Nothing is committed.
    Pending,
    /// Refused, with a reason for the status bar.
    Refused(&'static str),
}

/// Route an edit through the current mode.
pub fn route(intent: EditIntent, doc: &Document, session: &Session) -> Routed {
    route_inner(intent, doc, session, false)
}

/// Route an edit as if auto-key were on — the explicit "key this now" path (`K`).
pub fn route_forced(intent: EditIntent, doc: &Document, session: &Session) -> Routed {
    route_inner(intent, doc, session, true)
}

fn route_inner(intent: EditIntent, doc: &Document, session: &Session, force: bool) -> Routed {
    if !session.is_animating() {
        return Routed::Commands(setup_commands(intent));
    }

    let Some(anim) = session.active_animation else {
        // Animate mode is entered with a clip selected (`AppState::set_work_mode`),
        // so this is a bug rather than a user error — refuse loudly instead of
        // silently editing the setup pose the user cannot see.
        return Routed::Refused("No animation selected — pick or create one first");
    };

    if !(session.auto_key || force) {
        return Routed::Pending;
    }

    Routed::Commands(key_commands(intent, doc, session, anim))
}

// ── Setup mode ───────────────────────────────────────────────────────────────

fn setup_commands(intent: EditIntent) -> Vec<Box<dyn EditCommand>> {
    match intent {
        EditIntent::BoneLocal { bone, value } => vec![Box::new(SetBoneTransform::new(bone, value))],
        EditIntent::SlotColor { slot, value } => vec![Box::new(SetSlotColor::new(slot, value))],
        EditIntent::SlotAttachment { slot, value } => {
            vec![Box::new(SetSlotAttachment::new(slot, value))]
        }
        EditIntent::DrawOrder { order } => vec![Box::new(SetDrawOrder::new(order))],
    }
}

// ── Animate mode ─────────────────────────────────────────────────────────────

fn key_commands(
    intent: EditIntent,
    doc: &Document,
    session: &Session,
    anim: AnimationId,
) -> Vec<Box<dyn EditCommand>> {
    let time = session.playhead.max(0.0);
    match intent {
        EditIntent::BoneLocal { bone, value } => bone_keys(doc, anim, bone, value, time),
        EditIntent::SlotColor { slot, value } => {
            vec![Box::new(AddKey::new(
                anim,
                TimelineAddr::SlotColor { slot },
                time,
                KeyValue::Color(value),
                Interp::Linear,
            ))]
        }
        EditIntent::SlotAttachment { slot, value } => {
            vec![Box::new(AddAttachmentKey::new(anim, slot, time, value))]
        }
        EditIntent::DrawOrder { order } => draw_order_keys(doc, anim, order, time),
    }
}

/// Bone channels that actually changed, as offsets from the setup pose.
///
/// A newly created timeline gets a `t = 0` baseline key holding the setup value
/// first, so the change eases in from the setup pose instead of being constant
/// across the whole clip (PLAN §7.3).
fn bone_keys(
    doc: &Document,
    anim: AnimationId,
    bone: BoneId,
    local: Transform,
    time: f32,
) -> Vec<Box<dyn EditCommand>> {
    let Some(setup) = doc.skeleton.bones.get(bone).map(|b| b.local_transform) else {
        return Vec::new();
    };

    let rot_delta = ankhimate_core::transforms::wrap_angle(local.rotation - setup.rotation);
    // One entry per **track**: each axis of translate/scale/shear, plus
    // rotation. Tested per axis, so nudging a bone along x keys x alone — the
    // pairing this replaced keyed both and left a redundant y key behind on
    // every horizontal move.
    let d_pos = local.position - setup.position;
    let d_shear = local.shear - setup.shear;
    let channels: [(BoneProperty, Option<Axis>, f32, f32, bool); 7] = [
        (
            BoneProperty::Translate,
            Some(Axis::X),
            d_pos.x,
            0.0,
            d_pos.x.abs() > 1e-4,
        ),
        (
            BoneProperty::Translate,
            Some(Axis::Y),
            d_pos.y,
            0.0,
            d_pos.y.abs() > 1e-4,
        ),
        (
            BoneProperty::Rotate,
            None,
            rot_delta.to_degrees(),
            0.0,
            rot_delta.abs() > 1e-4,
        ),
        (
            BoneProperty::Scale,
            Some(Axis::X),
            ratio(local.scale.x, setup.scale.x),
            1.0,
            (local.scale.x - setup.scale.x).abs() > 1e-4,
        ),
        (
            BoneProperty::Scale,
            Some(Axis::Y),
            ratio(local.scale.y, setup.scale.y),
            1.0,
            (local.scale.y - setup.scale.y).abs() > 1e-4,
        ),
        // Degrees, like the rotate channel above.
        (
            BoneProperty::Shear,
            Some(Axis::X),
            d_shear.x.to_degrees(),
            0.0,
            d_shear.x.abs() > 1e-4,
        ),
        (
            BoneProperty::Shear,
            Some(Axis::Y),
            d_shear.y.to_degrees(),
            0.0,
            d_shear.y.abs() > 1e-4,
        ),
    ];

    let mut cmds: Vec<Box<dyn EditCommand>> = Vec::new();
    for (property, axis, value, rest, changed) in channels {
        if !changed {
            continue;
        }
        let addr = TimelineAddr::Bone {
            bone,
            property,
            axis,
        };
        if time > 0.0 && !timeline_exists(doc, anim, &addr) {
            cmds.push(Box::new(AddKey::new(
                anim,
                addr.clone(),
                0.0,
                KeyValue::Scalar(rest),
                Interp::Linear,
            )));
        }
        cmds.push(Box::new(AddKey::new(
            anim,
            addr,
            time,
            KeyValue::Scalar(value),
            Interp::Linear,
        )));
    }
    cmds
}

/// Draw-order keys store offsets from the setup order, so the first key on a
/// clip is preceded by a `t = 0` baseline holding the setup stack — otherwise the
/// stepped sampler would hold this key's value backward to the start of the clip.
fn draw_order_keys(
    doc: &Document,
    anim: AnimationId,
    order: Vec<SlotId>,
    time: f32,
) -> Vec<Box<dyn EditCommand>> {
    let setup = doc.skeleton.draw_order.clone();
    let has_timeline = doc
        .animations
        .get(anim)
        .map(|a| {
            a.timelines
                .iter()
                .any(|t| matches!(t, Timeline::DrawOrder { .. }))
        })
        .unwrap_or(false);

    let mut cmds: Vec<Box<dyn EditCommand>> = Vec::new();
    if time > 0.0 && !has_timeline {
        cmds.push(Box::new(AddDrawOrderKey::new(
            anim,
            0.0,
            setup.clone(),
            setup.clone(),
        )));
    }
    cmds.push(Box::new(AddDrawOrderKey::new(anim, time, order, setup)));
    cmds
}

fn timeline_exists(doc: &Document, anim: AnimationId, addr: &TimelineAddr) -> bool {
    doc.animations
        .get(anim)
        .map(|a| a.timelines.iter().any(|t| addr.matches_timeline(t)))
        .unwrap_or(false)
}

// ── Key state, for the inspector's affordances (T-210) ───────────────────────

/// What the animation says about one property, at the playhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// Nothing animates this property in this clip.
    NoTimeline,
    /// A timeline exists, but there is no key at this exact time.
    Unkeyed,
    /// Keyed here — the index is what a delete would remove.
    Keyed(usize),
    /// Posed but not committed: an unkeyed edit is pending (auto-key off).
    Modified,
}

/// Two key times are the same key when they land on the same displayed frame;
/// float times never compare exactly after a drag.
const KEY_EPS: f32 = 1e-4;

/// Inspect one property's key state at the playhead.
pub fn key_state(doc: &Document, session: &Session, addr: &TimelineAddr) -> KeyState {
    let Some(anim) = session
        .active_animation
        .and_then(|id| doc.animations.get(id))
    else {
        return KeyState::NoTimeline;
    };
    let Some(timeline) = anim.timelines.iter().find(|t| addr.matches_timeline(t)) else {
        return KeyState::NoTimeline;
    };
    match key_index_at(timeline, session.playhead) {
        Some(i) => KeyState::Keyed(i),
        None => KeyState::Unkeyed,
    }
}

/// Index of the key at `time` on `timeline`, if any.
fn key_index_at(timeline: &Timeline, time: f32) -> Option<usize> {
    macro_rules! find {
        ($keys:expr) => {
            $keys.iter().position(|k| (k.time - time).abs() < KEY_EPS)
        };
    }
    match timeline {
        Timeline::BoneTranslate { keys, .. } => find!(keys),
        Timeline::BoneRotate { keys, .. } => find!(keys),
        Timeline::BoneScale { keys, .. } => find!(keys),
        Timeline::BoneShear { keys, .. } => find!(keys),
        Timeline::SlotVisible { keys, .. } => find!(keys),
        Timeline::SlotColor { keys, .. } => find!(keys),
        Timeline::SlotAttachment { keys, .. } => find!(keys),
        Timeline::DrawOrder { keys } => find!(keys),
        Timeline::IkMix { keys, .. } => find!(keys),
        Timeline::IkBendDirection { keys, .. } => find!(keys),
        Timeline::IkSoftness { keys, .. } => find!(keys),
        Timeline::TransformConstraintMix { keys, .. } => find!(keys),
        Timeline::Deform { keys, .. } => find!(keys),
    }
}

/// The value a key on this bone property would capture right now — the posed
/// value as an offset from setup, in the timeline's storage units.
///
/// Shared by the inspector dots and the timeline's "set key" so the two can
/// never disagree about what "key this" means.
pub fn bone_key_value(
    doc: &Document,
    pose: &ankhimate_core::pose::Pose,
    bone: BoneId,
    property: BoneProperty,
    axis: Option<Axis>,
) -> Option<KeyValue> {
    let setup = doc.skeleton.bones.get(bone)?.local_transform;
    let local = pose.locals.get(bone).copied()?;
    // A two-axis property addresses one axis per track; `None` means x, which
    // is what a caller that has not chosen means.
    let i = axis.unwrap_or(Axis::X).index();
    Some(KeyValue::Scalar(match property {
        BoneProperty::Translate => local.position[i] - setup.position[i],
        BoneProperty::Rotate => {
            ankhimate_core::transforms::wrap_angle(local.rotation - setup.rotation).to_degrees()
        }
        BoneProperty::Scale => ratio(local.scale[i], setup.scale[i]),
        BoneProperty::Shear => (local.shear[i] - setup.shear[i]).to_degrees(),
    }))
}

/// Divide, guarding a near-zero denominator (scale keys multiply).
fn ratio(a: f32, b: f32) -> f32 {
    if b.abs() < 1e-6 { 1.0 } else { a / b }
}
