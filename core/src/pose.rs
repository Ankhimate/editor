//! Derived pose state — the entire runtime contract (PLAN §2.6).
//!
//! A [`Pose`] is the output of [`evaluate`]: everything needed to draw a
//! skeleton at one instant, and *nothing* that belongs in the document. It is
//! never serialized. The editor viewport, the exporters, and the shipping game
//! runtime all call the same `evaluate`, so what you see in the editor is what
//! ships.
//!
//! # Determinism
//!
//! Identical inputs must produce a bit-identical `Pose`: no `std::time`, no
//! global state, no iteration over a `HashMap` where order can leak into the
//! result (PLAN §2.6). Bone traversal always follows
//! [`Skeleton::update_order`](crate::skeleton::Skeleton::update_order).

use crate::animation::{self, Animation, Timeline};
use crate::constraints::{
    Constraint, IkConstraint, soften_target, solve_aim, solve_fabrik, solve_two_bone_ik,
    stretch_factor,
};
use crate::ids::{BoneId, ConstraintId, SlotId};
use crate::math::Transform;
use crate::skeleton::Skeleton;
use crate::transforms::{Affine2, wrap_angle};
use slotmap::SecondaryMap;
use std::collections::HashMap;

/// One evaluated instant of a skeleton. Derived state — never serialized.
#[derive(Debug, Clone, Default)]
pub struct Pose {
    /// Setup local transform ⊕ animation, per bone.
    pub locals: SecondaryMap<BoneId, Transform>,
    /// World affine per bone, composed along `update_order`.
    pub worlds: SecondaryMap<BoneId, Affine2>,
    pub slot_colors: SecondaryMap<SlotId, [f32; 4]>,
    pub slot_attachments: SecondaryMap<SlotId, Option<String>>,
    /// Draw order after animation offsets are applied.
    pub draw_order: Vec<SlotId>,
    /// FFD vertex offsets, keyed by (slot, attachment name).
    pub deforms: HashMap<(SlotId, String), Vec<glam::Vec2>>,
    /// Animated IK mix overrides. Absent means "use the constraint's own `mix`".
    pub ik_mix: SecondaryMap<ConstraintId, f32>,
    /// Animated transform-constraint mixes, `[rotate, translate, scale, shear]`
    /// (T-501). Absent means "use the constraint's own values".
    pub transform_mix: SecondaryMap<ConstraintId, [f32; 4]>,
    /// Animated IK bend direction and softness overrides (T-504).
    pub ik_bend_direction: SecondaryMap<ConstraintId, f32>,
    pub ik_softness: SecondaryMap<ConstraintId, f32>,
}

impl Pose {
    pub fn new() -> Self {
        Self::default()
    }

    /// World affine of a bone, or identity when the bone is not in this pose.
    pub fn world(&self, bone: BoneId) -> Affine2 {
        self.worlds.get(bone).copied().unwrap_or(Affine2::IDENTITY)
    }

    /// World-space origin of a bone.
    pub fn world_position(&self, bone: BoneId) -> glam::Vec2 {
        self.world(bone).transform_point(glam::Vec2::ZERO)
    }

    /// World-space tip of a bone — the local point `(length, 0)` pushed through
    /// the world affine, so non-uniform scale and shear are handled correctly.
    pub fn world_tip(&self, skel: &Skeleton, bone: BoneId) -> glam::Vec2 {
        let length = skel.bones.get(bone).map(|b| b.length).unwrap_or(0.0);
        self.world(bone)
            .transform_point(glam::Vec2::new(length, 0.0))
    }

    /// World-space length of a bone along its own X axis.
    pub fn world_length(&self, skel: &Skeleton, bone: BoneId) -> f32 {
        (self.world_tip(skel, bone) - self.world_position(bone)).length()
    }

    /// Decomposed world transform of a bone — for editor gizmos and
    /// world→local conversions only, never for composing children (ADR 0002).
    pub fn world_decomposed(&self, bone: BoneId) -> Transform {
        self.world(bone).decompose()
    }

    /// Drop all contents, keeping allocated capacity so a per-frame
    /// `evaluate` does not churn the allocator.
    fn reset(&mut self) {
        self.locals.clear();
        self.worlds.clear();
        self.slot_colors.clear();
        self.slot_attachments.clear();
        self.draw_order.clear();
        self.deforms.clear();
        self.ik_mix.clear();
        self.transform_mix.clear();
        self.ik_bend_direction.clear();
        self.ik_softness.clear();
    }
}

/// Evaluate a skeleton (plus any animations mixed by `alpha`) into `out`.
///
/// Fixed pipeline order (PLAN §2.6):
/// 1. copy the setup pose into the `Pose`;
/// 2. apply animation timelines, mixed by `alpha`;
/// 3. apply constraints in order;
/// 4. compose world affines along `update_order`.
///
/// `out` is reused across calls: it is reset on entry, so callers can keep one
/// `Pose` per viewport and avoid reallocating every frame.
pub fn evaluate(skel: &Skeleton, anims: &[(&Animation, f32, f32)], out: &mut Pose) {
    out.reset();

    // ── Stage 1: setup pose ──────────────────────────────────────────────
    for (id, bone) in skel.bones.iter() {
        out.locals.insert(id, bone.local_transform);
    }
    for (id, slot) in skel.slots.iter() {
        out.slot_colors.insert(id, slot.color);
        out.slot_attachments.insert(id, slot.attachment.clone());
    }
    out.draw_order.extend_from_slice(&skel.draw_order);
    // Slots missing from `draw_order` (e.g. created but never ordered) still
    // need to be drawable; append them in slotmap order for determinism.
    for (id, _) in skel.slots.iter() {
        if !out.draw_order.contains(&id) {
            out.draw_order.push(id);
        }
    }

    // ── Stage 2: animation ───────────────────────────────────────────────
    apply_animations(skel, anims, out);

    // ── Stage 3 & 4: constraints, then world affines ─────────────────────
    // `update_worlds` composes the FK chain; constraints need world state to
    // solve against, so they run inside it per chain (see T-104).
    update_worlds(skel, out);
    apply_constraints(skel, out);
}

/// Stage 2 — apply animation timelines into the `Pose`.
///
/// Bone keys are **offsets from the setup pose** (see the `animation` module
/// docs): translate/shear add, scale multiplies, rotate adds shortest-arc. Each
/// animation's contribution is scaled by its `alpha`, so several animations
/// crossfade by construction.
///
/// Non-blendable timelines (`SlotAttachment`, `DrawOrder`) cannot be averaged, so
/// the highest-alpha animation that has an opinion wins.
fn apply_animations(skel: &Skeleton, anims: &[(&Animation, f32, f32)], out: &mut Pose) {
    // Winner-takes-all bookkeeping for the non-blendable timelines.
    let mut attachment_winner: SecondaryMap<SlotId, f32> = SecondaryMap::new();
    let mut draw_order_winner: Option<f32> = None;

    for &(anim, time, alpha) in anims {
        if alpha <= 0.0 {
            continue;
        }
        // A sampling hint per timeline would have to live across calls to pay off;
        // `evaluate` is stateless by contract (determinism, PLAN §2.6), so the
        // hint starts cold here and the binary search carries the cost. Playback
        // callers that want the sequential fast path should hold their own
        // per-timeline cursors and sample directly.
        let mut hint = 0usize;

        for timeline in &anim.timelines {
            match timeline {
                Timeline::BoneTranslate { bone, keys } => {
                    if let Some(offset) = animation::sample(keys, time, &mut hint)
                        && let Some(local) = out.locals.get_mut(*bone)
                    {
                        local.position += offset * alpha;
                    }
                }
                Timeline::BoneRotate { bone, keys } => {
                    if let Some(degrees) = animation::sample_angle_degrees(keys, time, &mut hint)
                        && let Some(local) = out.locals.get_mut(*bone)
                    {
                        // Keys are degrees at the document level; core math is
                        // radians (ADR 0002).
                        local.rotation = wrap_angle(local.rotation + degrees.to_radians() * alpha);
                    }
                }
                Timeline::BoneScale { bone, keys } => {
                    if let Some(factor) = animation::sample(keys, time, &mut hint)
                        && let Some(local) = out.locals.get_mut(*bone)
                    {
                        // Scale is multiplicative, so `alpha` interpolates between
                        // "no effect" (1.0) and the sampled factor.
                        let blended = glam::Vec2::ONE.lerp(factor, alpha);
                        local.scale *= blended;
                    }
                }
                Timeline::BoneShear { bone, keys } => {
                    if let Some(offset) = animation::sample(keys, time, &mut hint)
                        && let Some(local) = out.locals.get_mut(*bone)
                    {
                        // Keys are degrees at the document level, like rotation;
                        // core math is radians (ADR 0002).
                        local.shear +=
                            glam::vec2(offset.x.to_radians(), offset.y.to_radians()) * alpha;
                    }
                }
                Timeline::SlotColor { slot, keys } => {
                    if let Some(color) = animation::sample(keys, time, &mut hint)
                        && let Some(current) = out.slot_colors.get_mut(*slot)
                    {
                        // Absolute value, blended toward by alpha.
                        for i in 0..4 {
                            current[i] += (color[i] - current[i]) * alpha;
                        }
                    }
                }
                Timeline::SlotAttachment { slot, keys } => {
                    if let Some(name) = animation::sample_stepped(keys, time, &mut hint) {
                        let wins = attachment_winner
                            .get(*slot)
                            .is_none_or(|&best| alpha >= best);
                        if wins && out.slot_attachments.contains_key(*slot) {
                            attachment_winner.insert(*slot, alpha);
                            out.slot_attachments.insert(*slot, name);
                        }
                    }
                }
                Timeline::DrawOrder { keys } => {
                    if let Some(offsets) = animation::sample_stepped(keys, time, &mut hint)
                        && draw_order_winner.is_none_or(|best| alpha >= best)
                    {
                        draw_order_winner = Some(alpha);
                        apply_draw_order_offsets(&mut out.draw_order, &offsets);
                    }
                }
                Timeline::IkMix { constraint, keys } => {
                    if let Some(mix) = animation::sample(keys, time, &mut hint) {
                        let current = out
                            .ik_mix
                            .get(*constraint)
                            .copied()
                            .or_else(|| {
                                skel.constraints.get(*constraint).and_then(|c| match c {
                                    Constraint::Ik(ik) => Some(ik.mix),
                                    // An `IkMix` key pointed at a non-IK
                                    // constraint: nothing to blend from.
                                    Constraint::Transform(_) => None,
                                })
                            })
                            .unwrap_or(0.0);
                        out.ik_mix
                            .insert(*constraint, current + (mix - current) * alpha);
                    }
                }
                Timeline::IkBendDirection { constraint, keys } => {
                    // Stepped: a direction interpolated through zero would let
                    // the chain flip either way mid-blend.
                    if let Some(dir) = animation::sample_stepped(keys, time, &mut hint) {
                        out.ik_bend_direction.insert(*constraint, dir.signum());
                    }
                }
                Timeline::IkSoftness { constraint, keys } => {
                    if let Some(softness) = animation::sample(keys, time, &mut hint) {
                        let current = out
                            .ik_softness
                            .get(*constraint)
                            .copied()
                            .or_else(|| {
                                skel.constraints.get(*constraint).and_then(|c| match c {
                                    Constraint::Ik(ik) => Some(ik.softness),
                                    Constraint::Transform(_) => None,
                                })
                            })
                            .unwrap_or(0.0);
                        out.ik_softness
                            .insert(*constraint, current + (softness - current) * alpha);
                    }
                }
                Timeline::TransformConstraintMix { constraint, keys } => {
                    if let Some(mixes) = animation::sample(keys, time, &mut hint) {
                        // Same crossfade shape as `IkMix`: blend from whatever
                        // is already there — an earlier animation's contribution
                        // or the constraint's authored value — toward this
                        // animation's opinion, weighted by its alpha.
                        let current = out
                            .transform_mix
                            .get(*constraint)
                            .copied()
                            .or_else(|| {
                                skel.constraints.get(*constraint).and_then(|c| match c {
                                    Constraint::Transform(tc) => Some([
                                        tc.mix_rotate,
                                        tc.mix_translate,
                                        tc.mix_scale,
                                        tc.mix_shear,
                                    ]),
                                    Constraint::Ik(_) => None,
                                })
                            })
                            .unwrap_or([0.0; 4]);
                        let blended =
                            std::array::from_fn(|i| current[i] + (mixes[i] - current[i]) * alpha);
                        out.transform_mix.insert(*constraint, blended);
                    }
                }
                Timeline::Deform {
                    slot,
                    attachment,
                    keys,
                } => {
                    if let Some(offsets) = animation::sample(keys, time, &mut hint) {
                        let key = (*slot, attachment.clone());
                        match out.deforms.get_mut(&key) {
                            Some(current) => {
                                for (i, dst) in current.iter_mut().enumerate() {
                                    if let Some(v) = offsets.get(i) {
                                        *dst += (*v - *dst) * alpha;
                                    }
                                }
                            }
                            None => {
                                out.deforms
                                    .insert(key, offsets.iter().map(|v| *v * alpha).collect());
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Apply Spine-style draw-order offsets: each `(slot, delta)` moves that slot
/// `delta` places from where it sits in the setup order.
///
/// Offsets rather than an absolute permutation so the data stays valid when slots
/// are added later (PLAN §2.3).
fn apply_draw_order_offsets(order: &mut [SlotId], offsets: &[(SlotId, i32)]) {
    if offsets.is_empty() {
        return;
    }
    // Resolve every offset against the *setup* index, then sort by the resulting
    // target position. Doing it pairwise against a mutating vector would make the
    // result depend on the offset list's own order.
    let mut targets: Vec<(f32, SlotId)> = order
        .iter()
        .enumerate()
        .map(|(i, &slot)| {
            let delta = offsets
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, d)| *d)
                .unwrap_or(0);
            // Bias in the direction of travel so a moved slot settles past the
            // resident at its target index rather than tying with it: moving
            // later lands after, moving earlier lands before. Ties would
            // otherwise resolve by whichever the sort visited first.
            let bias = match delta.signum() {
                1 => 0.5,
                -1 => -0.5,
                _ => 0.0,
            };
            ((i as i32 + delta) as f32 + bias, slot)
        })
        .collect();

    targets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (dst, (_, slot)) in order.iter_mut().zip(targets) {
        *dst = slot;
    }
}

/// Recompose every world affine from the pose's current `locals`.
///
/// For callers that mutate `Pose.locals` after `evaluate` — the editor overlays
/// live drag values this way so a drag looks correct without touching the
/// document (PLAN §3.2, defect D7). Constraints are **not** re-run: the override
/// is the user's direct intent and should win.
pub fn recompose_worlds(skel: &Skeleton, out: &mut Pose) {
    update_worlds(skel, out);
}

/// Stage 4 — compose world affines for every bone along `update_order`.
fn update_worlds(skel: &Skeleton, out: &mut Pose) {
    for &id in &skel.update_order {
        update_world_of(skel, out, id);
    }
}

/// Compose one bone's world affine from its parent's, honoring `inherit`.
/// Pure `Affine2` math — no `Mat4`, no `Quat` (ADR 0002).
fn update_world_of(skel: &Skeleton, out: &mut Pose, id: BoneId) {
    let Some(bone) = skel.bones.get(id) else {
        return;
    };
    let local = out.locals.get(id).copied().unwrap_or(bone.local_transform);

    let world = match bone.parent.and_then(|p| out.worlds.get(p).copied()) {
        Some(parent_world) => Affine2::compose_child(&parent_world, &local, &bone.inherit),
        None => Affine2::compose(&local),
    };
    out.worlds.insert(id, world);
}

/// Stage 3 — apply constraints in `constraint_order`, re-running FK for each
/// affected subtree so later constraints see the earlier ones' results.
fn apply_constraints(skel: &Skeleton, out: &mut Pose) {
    let ordered: Vec<(ConstraintId, &Constraint)> = skel.ordered_constraints().collect();
    for (id, constraint) in ordered {
        match constraint {
            Constraint::Ik(ik) => {
                // An `IkMix` timeline overrides the constraint's authored mix.
                let mix = out.ik_mix.get(id).copied().unwrap_or(ik.mix);
                if mix <= 0.0 || ik.bones.is_empty() {
                    continue;
                }
                // Animated bend direction and softness override the authored
                // values, the same way `IkMix` overrides `mix` (T-504).
                let bend = out.ik_bend_direction.get(id).copied();
                let softness = out.ik_softness.get(id).copied();
                let effective;
                let ik = if bend.is_some() || softness.is_some() {
                    effective = IkConstraint {
                        bend_direction: bend.unwrap_or(ik.bend_direction),
                        softness: softness.unwrap_or(ik.softness),
                        ..ik.clone()
                    };
                    &effective
                } else {
                    ik
                };
                apply_ik(skel, out, ik, mix);
            }
            Constraint::Transform(tc) => {
                // Per-channel mix overrides from timelines, falling back to the
                // authored values.
                let mixes = out.transform_mix.get(id).copied().unwrap_or([
                    tc.mix_rotate,
                    tc.mix_translate,
                    tc.mix_scale,
                    tc.mix_shear,
                ]);
                if mixes.iter().all(|m| *m == 0.0) || tc.bones.is_empty() {
                    continue;
                }
                apply_transform_constraint(skel, out, tc, mixes);
            }
        }
    }
}

/// Drive each constrained bone toward the target's transform, channel by
/// channel (T-501).
///
/// Writes **local** transforms, like the IK pass and for the same reason
/// (defect D3): a world write does not propagate to children, and the next
/// constraint in the order would read a world that no longer matches the locals
/// it is about to modify.
fn apply_transform_constraint(
    skel: &Skeleton,
    out: &mut Pose,
    tc: &crate::constraints::TransformConstraint,
    [mix_rotate, mix_translate, mix_scale, mix_shear]: [f32; 4],
) {
    if skel.bones.get(tc.target).is_none() {
        return;
    }
    // What the target contributes. In world mode it is the target's world
    // transform decomposed; in local mode its own local transform.
    let source = if tc.local {
        out.locals
            .get(tc.target)
            .copied()
            .unwrap_or_else(Transform::default)
    } else {
        out.world_decomposed(tc.target)
    };

    for &bone in &tc.bones {
        // A bone driving itself would read its own output — the constraint
        // would converge on nothing meaningful and the result would depend on
        // constraint order in a way nobody could reason about.
        if bone == tc.target || skel.bones.get(bone).is_none() {
            continue;
        }

        // The goal for this bone, expressed in the same space as `source`.
        let current = if tc.local {
            out.locals.get(bone).copied().unwrap_or_default()
        } else {
            out.world_decomposed(bone)
        };
        let goal = if tc.relative {
            Transform {
                position: current.position + source.position + tc.offsets.position,
                rotation: current.rotation + source.rotation + tc.offsets.rotation,
                scale: current.scale * source.scale * tc.offsets.scale,
                shear: current.shear + source.shear + tc.offsets.shear,
            }
        } else {
            Transform {
                position: source.position + tc.offsets.position,
                rotation: source.rotation + tc.offsets.rotation,
                scale: source.scale * tc.offsets.scale,
                shear: source.shear + tc.offsets.shear,
            }
        };

        // Blend each channel by its own mix, then write the result as a local
        // delta so children follow.
        let (Some(local), Some(_)) = (out.locals.get(bone).copied(), skel.bones.get(bone)) else {
            continue;
        };
        let mut next = local;

        if mix_rotate != 0.0 {
            // Shortest-arc, so a constraint across the ±π boundary turns the
            // short way — the same defect that broke IK blending.
            let delta = wrap_angle(goal.rotation - current.rotation) * mix_rotate;
            next.rotation = wrap_angle(next.rotation + delta);
        }
        if mix_translate != 0.0 {
            let delta = (goal.position - current.position) * mix_translate;
            // A world-space delta has to be rotated into the parent's frame
            // before it can be added to a local position, or a constrained bone
            // under a rotated parent slides off at an angle.
            let delta = if tc.local {
                delta
            } else {
                parent_inverse_direction(skel, out, bone, delta)
            };
            next.position += delta;
        }
        if mix_scale != 0.0 {
            let delta = goal.scale - current.scale;
            next.scale += delta * mix_scale;
        }
        if mix_shear != 0.0 {
            let delta = glam::Vec2::new(
                wrap_angle(goal.shear.x - current.shear.x),
                wrap_angle(goal.shear.y - current.shear.y),
            ) * mix_shear;
            next.shear += delta;
        }

        if let Some(slot) = out.locals.get_mut(bone) {
            *slot = next;
        }
        update_world_of(skel, out, bone);
        update_subtree(skel, out, bone);
    }
}

/// Rotate a world-space direction into a bone's parent space.
///
/// Only the linear part is applied — a direction has no origin, so the parent's
/// translation must not be added to it.
fn parent_inverse_direction(
    skel: &Skeleton,
    out: &Pose,
    bone: BoneId,
    world_delta: glam::Vec2,
) -> glam::Vec2 {
    let Some(parent) = skel.bones.get(bone).and_then(|b| b.parent) else {
        return world_delta;
    };
    let Some(inverse) = out.world(parent).invert() else {
        // A zero-scaled parent has no invertible frame; leaving the delta in
        // world space is wrong but finite, and the alternative is a NaN that
        // spreads through the whole subtree.
        return world_delta;
    };
    inverse.transform_vector(world_delta)
}

/// Rotate one bone toward `target_world_rot`, shortest-arc, scaled by `mix`.
/// Writes a **local** delta (defect D3) and re-derives that bone's world affine.
fn blend_bone_rotation(
    skel: &Skeleton,
    out: &mut Pose,
    bone: BoneId,
    target_world_rot: f32,
    mix: f32,
) {
    let current = out.world_decomposed(bone).rotation;
    let delta = wrap_angle(target_world_rot - current) * mix;
    if let Some(local) = out.locals.get_mut(bone) {
        local.rotation = wrap_angle(local.rotation + delta);
    }
    update_world_of(skel, out, bone);
}

/// Re-run FK for every descendant of `root`, in topological order.
fn update_subtree(skel: &Skeleton, out: &mut Pose, root: BoneId) {
    for &id in &skel.update_order {
        if id != root && skel.is_descendant(id, root) {
            update_world_of(skel, out, id);
        }
    }
}

fn apply_ik(skel: &Skeleton, out: &mut Pose, ik: &IkConstraint, mix: f32) {
    // Every chain bone must exist, and the target must not be part of the chain
    // (that would make the constraint chase its own output).
    if ik.bones.iter().any(|b| skel.bones.get(*b).is_none())
        || skel.bones.get(ik.target).is_none()
        || ik.bones.contains(&ik.target)
    {
        return;
    }

    let raw_target = out.world_position(ik.target);
    let root_pos = out.world_position(ik.bones[0]);
    // Natural reach: the chain fully extended, measured in world units so a
    // scaled bone counts for what it actually spans.
    let reach: f32 = ik
        .bones
        .iter()
        .map(|b| out.world_length(skel, *b))
        .sum::<f32>();

    // Softness eases the approach to full extension (T-504); stretch lets the
    // chain grow past it. They compose: soften first so the eased target is
    // what stretch measures against, or a soft chain would still snap.
    let target_pos = soften_target(root_pos, raw_target, reach, ik.softness);
    let stretch = if ik.stretch {
        stretch_factor(root_pos, target_pos, reach, ik.stretch_limit)
    } else {
        1.0
    };
    if stretch > 1.0 {
        apply_stretch(skel, out, ik, stretch, mix);
    }

    match ik.bones.len() {
        // Aim: point a single bone's X axis at the target.
        1 => {
            let bone = ik.bones[0];
            let root_pos = out.world_position(bone);
            if let Some(angle) = solve_aim(root_pos, target_pos) {
                blend_bone_rotation(skel, out, bone, angle, mix);
                update_subtree(skel, out, bone);
            }
        }
        // Two-bone IK: an exact solution, so it is worth keeping rather than
        // letting FABRIK iterate toward what trigonometry already knows.
        2 => {
            let (parent, child) = (ik.bones[0], ik.bones[1]);
            let root_pos = out.world_position(parent);
            let l1 = out.world_length(skel, parent);
            let l2 = out.world_length(skel, child);

            let (p_angle, c_angle) =
                solve_two_bone_ik(root_pos, target_pos, l1, l2, ik.bend_direction);

            blend_bone_rotation(skel, out, parent, p_angle, mix);
            // The child's world rotation changed with its parent, so re-derive it
            // before measuring the child's own delta.
            update_world_of(skel, out, child);
            blend_bone_rotation(skel, out, child, c_angle, mix);

            // Descendants of the chain root follow the whole solved chain.
            update_subtree(skel, out, parent);
        }
        // Three or more: FABRIK (T-504).
        _ => apply_fabrik(skel, out, ik, target_pos, mix),
    }
}

/// Scale the chain's bones so it can reach past its natural length (T-504).
///
/// Writes `local.scale.x` — the axis a bone's length runs along — so the stretch
/// inherits down the chain the way every other transform does, and so a stretched
/// chain's attachments stretch with it rather than detaching.
fn apply_stretch(skel: &Skeleton, out: &mut Pose, ik: &IkConstraint, factor: f32, mix: f32) {
    // Mixed like everything else: a half-mixed IK should be half-stretched, or
    // fading a constraint out would leave the bones long.
    let scaled = 1.0 + (factor - 1.0) * mix;
    for &bone in &ik.bones {
        // Scale is inherited, so a child whose parent is also in the chain has
        // *already* been stretched by that parent. Scaling it again compounds:
        // a two-bone chain at 1.5 came out 1.875 times its length, because the
        // child got 1.5 from its parent and 1.5 of its own.
        let inherits_from_chain = skel
            .bones
            .get(bone)
            .and_then(|b| b.parent.map(|p| (p, b.inherit.scale)))
            .is_some_and(|(parent, inherits)| inherits && ik.bones.contains(&parent));
        if inherits_from_chain {
            continue;
        }
        if let Some(local) = out.locals.get_mut(bone) {
            local.scale.x *= scaled;
        }
        update_world_of(skel, out, bone);
    }
    update_subtree(skel, out, ik.bones[0]);
}

/// Solve a chain of three or more bones with FABRIK, then convert the solved
/// joint positions back into bone rotations (T-504).
///
/// FABRIK works in positions; a skeleton stores angles. The conversion is the
/// interesting half: each bone is rotated to point at the next solved joint,
/// blended shortest-arc through the same local-delta path as every other
/// constraint (defect D3), so children follow and a partial mix is meaningful.
fn apply_fabrik(
    skel: &Skeleton,
    out: &mut Pose,
    ik: &IkConstraint,
    target_pos: glam::Vec2,
    mix: f32,
) {
    // Joint positions: each bone's origin, plus the tip of the last one.
    let mut joints: Vec<glam::Vec2> = ik.bones.iter().map(|b| out.world_position(*b)).collect();
    joints.push(out.world_tip(skel, *ik.bones.last().expect("non-empty chain")));

    let lengths: Vec<f32> = ik
        .bones
        .iter()
        .map(|b| out.world_length(skel, *b))
        .collect();
    if lengths.iter().any(|l| *l <= 1e-6) {
        // A zero-length bone has no direction to solve for, and normalizing its
        // segment would produce NaN that spreads through the whole chain.
        return;
    }

    let solved = solve_fabrik(&joints, &lengths, target_pos);

    // Rotate each bone toward its solved segment, in order, re-deriving world
    // transforms as we go so each bone measures its delta against a parent that
    // has already moved.
    for (i, &bone) in ik.bones.iter().enumerate() {
        let (from, to) = (solved[i], solved[i + 1]);
        let Some(angle) = solve_aim(from, to) else {
            continue;
        };
        update_world_of(skel, out, bone);
        blend_bone_rotation(skel, out, bone, angle, mix);
    }
    update_subtree(skel, out, ik.bones[0]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Key;
    use crate::constraints::{IkConstraint, TransformConstraint};
    use crate::skeleton::Bone;

    const EPS: f32 = 1e-4;

    fn bone(name: &str, parent: Option<BoneId>) -> Bone {
        Bone {
            name: name.to_string(),
            parent,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn evaluate_is_deterministic() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let mid = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                rotation: 0.4,
                ..Transform::default()
            },
            ..bone("mid", Some(root))
        });
        let tip = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("tip", Some(mid))
        });
        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 0.5,
            bend_direction: 1.0,
            ..IkConstraint::two_bone("ik", tip, [root, mid])
        }));

        let mut a = Pose::new();
        let mut b = Pose::new();
        evaluate(&skel, &[], &mut a);
        evaluate(&skel, &[], &mut b);

        for &id in &skel.update_order {
            assert_eq!(a.world(id), b.world(id), "world differs for {id:?}");
            assert_eq!(
                a.locals.get(id).copied(),
                b.locals.get(id).copied(),
                "local differs for {id:?}"
            );
        }
    }

    #[test]
    fn evaluate_does_not_mutate_the_document() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let mid = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("mid", Some(root))
        });
        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 1.0,
            bend_direction: 1.0,
            ..IkConstraint::two_bone("ik", mid, [root, mid])
        }));

        let before: Vec<Transform> = skel.bones.iter().map(|(_, b)| b.local_transform).collect();
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        let after: Vec<Transform> = skel.bones.iter().map(|(_, b)| b.local_transform).collect();

        // `evaluate` takes `&Skeleton`, so this is enforced by the type system —
        // the test documents the intent and guards against a future `&mut`.
        assert_eq!(before, after);
    }

    #[test]
    fn child_under_scaled_rotated_parent_lands_at_hand_computed_position() {
        // Regression for defect D2, now asserted through the Pose pipeline.
        let mut skel = Skeleton::new();
        let root = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(5.0, 5.0),
                rotation: std::f32::consts::FRAC_PI_2,
                scale: glam::vec2(2.0, 1.0),
                shear: glam::Vec2::ZERO,
            },
            ..bone("root", None)
        });
        let child = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("child", Some(root))
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let pos = pose.world_position(child);
        assert!((pos - glam::vec2(5.0, 25.0)).length() < 1e-4, "{pos:?}");
    }

    #[test]
    fn reused_pose_does_not_leak_stale_bones() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let doomed = skel.add_bone(bone("doomed", Some(root)));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        assert!(pose.worlds.get(doomed).is_some());

        skel.remove_bone(doomed);
        evaluate(&skel, &[], &mut pose);
        assert!(
            pose.worlds.get(doomed).is_none(),
            "removed bone survived into a reused Pose"
        );
    }

    #[test]
    fn draw_order_includes_unordered_slots() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let ordered = skel.slots.insert(crate::slot::Slot::new("a".into(), root));
        let unordered = skel.slots.insert(crate::slot::Slot::new("b".into(), root));
        skel.draw_order.push(ordered);

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        assert_eq!(pose.draw_order.len(), 2);
        assert_eq!(pose.draw_order[0], ordered);
        assert!(pose.draw_order.contains(&unordered));
    }

    #[test]
    fn ik_mix_across_pi_boundary_does_not_flip() {
        // A bone pointing just below −π blended halfway toward a target just
        // above +π must take the short way round.
        let mut skel = Skeleton::new();
        let root = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: std::f32::consts::PI - 0.1,
                ..Transform::default()
            },
            ..bone("root", None)
        });
        let mid = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("mid", Some(root))
        });
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(-19.0, -1.0),
                ..Transform::default()
            },
            ..bone("target", None)
        });
        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 0.5,
            bend_direction: 1.0,
            ..IkConstraint::two_bone("ik", target, [root, mid])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        // Short-arc blend keeps the bone near ±π; a naive lerp would swing it
        // through 0 and end up near zero rotation.
        let rot = pose.world_decomposed(root).rotation;
        assert!(
            rot.abs() > std::f32::consts::FRAC_PI_2,
            "flipped through zero: {rot}"
        );
    }

    // ── Stage 2: animation ───────────────────────────────────────────────

    #[test]
    fn bone_keys_are_offsets_from_the_setup_pose() {
        let mut skel = Skeleton::new();
        // Setup pose is deliberately non-identity: keys must offset it, not
        // replace it.
        let b = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(100.0, 5.0),
                rotation: 30.0_f32.to_radians(),
                scale: glam::vec2(2.0, 2.0),
                shear: glam::Vec2::ZERO,
            },
            ..bone("b", None)
        });

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneTranslate {
            bone: b,
            keys: vec![Key::linear(0.0, glam::vec2(10.0, 0.0))],
        });
        anim.timelines.push(Timeline::BoneRotate {
            bone: b,
            keys: vec![Key::linear(0.0, 60.0)],
        });
        anim.timelines.push(Timeline::BoneScale {
            bone: b,
            keys: vec![Key::linear(0.0, glam::vec2(3.0, 1.0))],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);
        let local = pose.locals[b];

        // Translate adds.
        assert!(
            (local.position - glam::vec2(110.0, 5.0)).length() < EPS,
            "{local:?}"
        );
        // Rotate adds (30° setup + 60° key).
        assert!(
            (local.rotation - 90.0_f32.to_radians()).abs() < EPS,
            "{}",
            local.rotation.to_degrees()
        );
        // Scale multiplies (2×3, 2×1).
        assert!(
            (local.scale - glam::vec2(6.0, 2.0)).length() < EPS,
            "{:?}",
            local.scale
        );
    }

    #[test]
    fn alpha_scales_an_animations_contribution() {
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("b", None));

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneTranslate {
            bone: b,
            keys: vec![Key::linear(0.0, glam::vec2(100.0, 0.0))],
        });
        anim.timelines.push(Timeline::BoneScale {
            bone: b,
            keys: vec![Key::linear(0.0, glam::vec2(3.0, 3.0))],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 0.5)], &mut pose);
        let local = pose.locals[b];

        assert!(
            (local.position.x - 50.0).abs() < EPS,
            "{}",
            local.position.x
        );
        // Multiplicative: halfway between 1.0 (no effect) and 3.0.
        assert!((local.scale.x - 2.0).abs() < EPS, "{}", local.scale.x);
    }

    #[test]
    fn zero_alpha_animation_is_ignored() {
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("b", None));

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneTranslate {
            bone: b,
            keys: vec![Key::linear(0.0, glam::vec2(100.0, 0.0))],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 0.0)], &mut pose);
        assert!(pose.locals[b].position.length() < EPS);
    }

    #[test]
    fn mixing_two_animations_blends_rotation_across_pi() {
        // Acceptance case: two animations at α=0.5 whose rotations straddle ±π.
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("b", None));

        let mut a = Animation::new("a", 1.0);
        a.timelines.push(Timeline::BoneRotate {
            bone: b,
            keys: vec![Key::linear(0.0, 170.0)],
        });
        let mut c = Animation::new("c", 1.0);
        c.timelines.push(Timeline::BoneRotate {
            bone: b,
            keys: vec![Key::linear(0.0, -170.0)],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&a, 0.0, 0.5), (&c, 0.0, 0.5)], &mut pose);

        // Each contributes half its offset, and `wrap_angle` keeps the running
        // total on the short side of the circle: 85° then −85° nets back to 0.
        let deg = pose.locals[b].rotation.to_degrees();
        assert!(deg.abs() < 0.01, "expected ~0°, got {deg}");
    }

    #[test]
    fn rotation_offsets_accumulate_shortest_arc() {
        // A single animation stepping past ±π must not wrap the long way.
        let mut skel = Skeleton::new();
        let b = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: 170.0_f32.to_radians(),
                ..Transform::default()
            },
            ..bone("b", None)
        });

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneRotate {
            bone: b,
            keys: vec![Key::linear(0.0, 20.0)],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);

        // 170 + 20 = 190, wrapped to -170.
        let deg = pose.locals[b].rotation.to_degrees();
        assert!((deg + 170.0).abs() < 0.01, "expected ~-170°, got {deg}");
    }

    #[test]
    fn slot_color_blends_toward_the_keyed_value() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(crate::slot::Slot::new("s".into(), root));
        skel.draw_order.push(slot);

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::SlotColor {
            slot,
            keys: vec![Key::linear(0.0, [0.0, 0.0, 0.0, 0.0])],
        });

        let mut pose = Pose::new();
        // Setup color is opaque white; α=0.5 lands halfway to transparent black.
        evaluate(&skel, &[(&anim, 0.0, 0.5)], &mut pose);
        assert_eq!(pose.slot_colors[slot], [0.5, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn draw_order_offsets_match_the_plan_example() {
        // PLAN §2.3: offsets are "slot moved +2 / −1 from setup".
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slots: Vec<_> = (0..4)
            .map(|i| {
                let s = skel
                    .slots
                    .insert(crate::slot::Slot::new(format!("s{i}"), root));
                skel.draw_order.push(s);
                s
            })
            .collect();
        // Setup order: [s0, s1, s2, s3]

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::DrawOrder {
            keys: vec![Key::stepped(0.0, vec![(slots[0], 2), (slots[3], -1)])],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);

        // s0 targets index 2, s3 targets index 2 as well but from the other side;
        // the unmoved s1/s2 keep their relative order around them.
        assert_eq!(pose.draw_order.len(), 4);
        let index_of = |s| pose.draw_order.iter().position(|x| *x == s).unwrap();
        assert!(index_of(slots[0]) > index_of(slots[1]), "s0 moved later");
        assert!(index_of(slots[3]) < 3, "s3 moved earlier");
        // Every slot still present exactly once.
        for s in &slots {
            assert_eq!(pose.draw_order.iter().filter(|x| *x == s).count(), 1);
        }
    }

    #[test]
    fn draw_order_animation_never_touches_the_setup_order() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let a = skel.slots.insert(crate::slot::Slot::new("a".into(), root));
        let b = skel.slots.insert(crate::slot::Slot::new("b".into(), root));
        skel.draw_order = vec![a, b];
        let setup_before = skel.draw_order.clone();

        let mut anim = Animation::new("anim", 1.0);
        anim.timelines.push(Timeline::DrawOrder {
            keys: vec![Key::stepped(0.0, vec![(a, 1)])],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);

        assert_eq!(pose.draw_order, vec![b, a], "animated order swapped");
        assert_eq!(skel.draw_order, setup_before, "setup order must be intact");
    }

    #[test]
    fn slot_attachment_is_stepped_and_highest_alpha_wins() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel
            .slots
            .insert(crate::slot::Slot::new("mouth".into(), root));
        skel.slots[slot].attachment = Some("closed".into());
        skel.draw_order.push(slot);

        let mut low = Animation::new("low", 1.0);
        low.timelines.push(Timeline::SlotAttachment {
            slot,
            keys: vec![Key::stepped(0.0, Some("low_wins".into()))],
        });
        let mut high = Animation::new("high", 1.0);
        high.timelines.push(Timeline::SlotAttachment {
            slot,
            keys: vec![Key::stepped(0.0, Some("high_wins".into()))],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&high, 0.0, 0.9), (&low, 0.0, 0.2)], &mut pose);
        assert_eq!(
            pose.slot_attachments[slot],
            Some("high_wins".to_string()),
            "attachment names cannot blend; highest alpha must win"
        );

        // The slot's own stored name is untouched (PLAN §2.4).
        assert_eq!(skel.slots[slot].attachment, Some("closed".to_string()));
    }

    #[test]
    fn ik_mix_timeline_overrides_the_authored_mix() {
        let mut skel = Skeleton::new();
        let aimer = skel.add_bone(bone("a_aimer", None));
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(0.0, 25.0),
                ..Transform::default()
            },
            ..bone("b_target", None)
        });
        // Authored mix is full-on.
        let cid = skel.add_constraint(Constraint::Ik(IkConstraint::aim("look", target, aimer)));

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::IkMix {
            constraint: cid,
            keys: vec![Key::linear(0.0, 0.0)],
        });

        let mut pose = Pose::new();
        // The timeline drives the mix to 0, so the aim must not fire at all.
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);
        assert!(
            pose.world_decomposed(aimer).rotation.abs() < EPS,
            "IkMix=0 should disable the constraint"
        );

        // Without the timeline the same skeleton solves fully.
        let mut solved = Pose::new();
        evaluate(&skel, &[], &mut solved);
        assert!(
            (solved.world_decomposed(aimer).rotation - std::f32::consts::FRAC_PI_2).abs() < EPS
        );
    }

    #[test]
    fn deform_offsets_land_in_the_pose() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root", None));
        let slot = skel.slots.insert(crate::slot::Slot::new("s".into(), root));

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::Deform {
            slot,
            attachment: "mesh".into(),
            keys: vec![Key::linear(
                0.0,
                vec![glam::vec2(4.0, 0.0), glam::vec2(0.0, 8.0)],
            )],
        });

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 0.5)], &mut pose);

        let got = pose
            .deforms
            .get(&(slot, "mesh".to_string()))
            .expect("deform");
        assert!((got[0] - glam::vec2(2.0, 0.0)).length() < EPS, "{got:?}");
        assert!((got[1] - glam::vec2(0.0, 4.0)).length() < EPS, "{got:?}");
    }

    #[test]
    fn animation_sampling_is_time_dependent() {
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("b", None));

        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneTranslate {
            bone: b,
            keys: vec![
                Key::linear(0.0, glam::Vec2::ZERO),
                Key::linear(1.0, glam::vec2(100.0, 0.0)),
            ],
        });

        let mut pose = Pose::new();
        for (time, want_x) in [(0.0, 0.0), (0.25, 25.0), (0.5, 50.0), (1.0, 100.0)] {
            evaluate(&skel, &[(&anim, time, 1.0)], &mut pose);
            let got = pose.locals[b].position.x;
            assert!((got - want_x).abs() < EPS, "at {time}: {got} != {want_x}");
        }
    }

    #[test]
    fn animated_evaluate_is_still_deterministic() {
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("b", None));
        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneRotate {
            bone: b,
            keys: vec![Key::linear(0.0, 0.0), Key::linear(1.0, 90.0)],
        });

        let mut first = Pose::new();
        let mut second = Pose::new();
        evaluate(&skel, &[(&anim, 0.37, 0.8)], &mut first);
        evaluate(&skel, &[(&anim, 0.37, 0.8)], &mut second);
        assert_eq!(first.world(b), second.world(b));

        // A reused pose must give the same answer as a fresh one.
        evaluate(&skel, &[(&anim, 0.9, 1.0)], &mut first);
        evaluate(&skel, &[(&anim, 0.37, 0.8)], &mut first);
        assert_eq!(first.world(b), second.world(b));
    }

    #[test]
    fn timelines_pointing_at_removed_entities_are_ignored() {
        let mut skel = Skeleton::new();
        let doomed = skel.add_bone(bone("doomed", None));
        let mut anim = Animation::new("a", 1.0);
        anim.timelines.push(Timeline::BoneTranslate {
            bone: doomed,
            keys: vec![Key::linear(0.0, glam::vec2(10.0, 0.0))],
        });

        skel.remove_bone(doomed);

        // Must not panic on the dangling id.
        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);
        assert!(pose.locals.get(doomed).is_none());
    }

    // ── Stage 3: constraints ─────────────────────────────────────────────

    #[test]
    fn aim_constraint_points_bone_at_target() {
        let mut skel = Skeleton::new();
        let aimer = skel.add_bone(bone("a_aimer", None));
        // Target straight above the aimer's origin.
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(0.0, 25.0),
                ..Transform::default()
            },
            ..bone("b_target", None)
        });
        skel.add_constraint(Constraint::Ik(IkConstraint::aim("look", target, aimer)));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let rot = pose.world_decomposed(aimer).rotation;
        assert!(
            (rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "aimer should point +Y: {rot}"
        );
        // And its tip should land on the way to the target.
        let tip = pose.world_tip(&skel, aimer);
        assert!((tip - glam::vec2(0.0, 10.0)).length() < 1e-4, "{tip:?}");
    }

    #[test]
    fn aim_constraint_respects_mix() {
        let mut skel = Skeleton::new();
        let aimer = skel.add_bone(bone("a_aimer", None));
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(0.0, 25.0),
                ..Transform::default()
            },
            ..bone("b_target", None)
        });
        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 0.5,
            ..IkConstraint::aim("look", target, aimer)
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        // Half of the 90° arc from the bone's rest rotation of 0.
        let rot = pose.world_decomposed(aimer).rotation;
        assert!(
            (rot - std::f32::consts::FRAC_PI_4).abs() < 1e-4,
            "half-mixed aim should be 45°: {rot}"
        );
    }

    #[test]
    fn inert_constraint_leaves_pose_untouched() {
        let mut skel = Skeleton::new();
        let aimer = skel.add_bone(bone("a_aimer", None));
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(0.0, 25.0),
                ..Transform::default()
            },
            ..bone("b_target", None)
        });

        let mut unconstrained = Pose::new();
        evaluate(&skel, &[], &mut unconstrained);

        // mix = 0 must be a no-op, not a partial solve.
        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 0.0,
            ..IkConstraint::aim("look", target, aimer)
        }));
        let mut constrained = Pose::new();
        evaluate(&skel, &[], &mut constrained);

        assert_eq!(unconstrained.world(aimer), constrained.world(aimer));
    }

    #[test]
    fn constraint_targeting_its_own_chain_is_skipped() {
        // A constraint whose target is inside its own chain would chase its own
        // output; it must be ignored rather than produce a garbage pose.
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a", None));
        let b = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("b", Some(a))
        });

        let mut before = Pose::new();
        evaluate(&skel, &[], &mut before);

        skel.add_constraint(Constraint::Ik(IkConstraint::two_bone("bad", b, [a, b])));
        let mut after = Pose::new();
        evaluate(&skel, &[], &mut after);

        assert_eq!(before.world(a), after.world(a));
        assert_eq!(before.world(b), after.world(b));
    }

    #[test]
    fn constraints_apply_in_constraint_order() {
        // Two aim constraints on one bone, pointing at opposite targets. The
        // last one in `constraint_order` wins, so swapping the order swaps the
        // resulting rotation.
        let mut skel = Skeleton::new();
        let aimer = skel.add_bone(bone("a_aimer", None));
        let up = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(0.0, 25.0),
                ..Transform::default()
            },
            ..bone("b_up", None)
        });
        let right = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(25.0, 0.0),
                ..Transform::default()
            },
            ..bone("c_right", None)
        });

        let aim_up = skel.add_constraint(Constraint::Ik(IkConstraint::aim("up", up, aimer)));
        let aim_right =
            skel.add_constraint(Constraint::Ik(IkConstraint::aim("right", right, aimer)));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        // `right` applied last → bone points +X.
        assert!(pose.world_decomposed(aimer).rotation.abs() < 1e-4);

        skel.constraint_order = vec![aim_right, aim_up];
        evaluate(&skel, &[], &mut pose);
        // `up` applied last → bone points +Y.
        let rot = pose.world_decomposed(aimer).rotation;
        assert!((rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{rot}");
    }

    #[test]
    fn ik_descendants_follow_the_chain() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("a_root", None));
        let mid = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("b_mid", Some(root))
        });
        // Child of the IK-driven chain: must move when the chain solves.
        let leaf = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("c_leaf", Some(mid))
        });
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(5.0, 12.0),
                ..Transform::default()
            },
            ..bone("d_target", None)
        });

        let mut unconstrained = Pose::new();
        evaluate(&skel, &[], &mut unconstrained);
        let leaf_before = unconstrained.world_position(leaf);

        skel.add_constraint(Constraint::Ik(IkConstraint {
            mix: 1.0,
            bend_direction: 1.0,
            ..IkConstraint::two_bone("ik", target, [root, mid])
        }));

        let mut solved = Pose::new();
        evaluate(&skel, &[], &mut solved);
        let leaf_after = solved.world_position(leaf);

        assert!(
            (leaf_after - leaf_before).length() > 1.0,
            "leaf did not follow the IK chain: {leaf_before:?} -> {leaf_after:?}"
        );
    }
    // ── Transform constraints (T-501) ────────────────────────────────────

    /// A skeleton with an unparented target and a bone to drive from it.
    fn target_and_driven() -> (Skeleton, BoneId, BoneId) {
        let mut skel = Skeleton::new();
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(100.0, 50.0),
                rotation: std::f32::consts::FRAC_PI_2, // 90°
                scale: glam::vec2(2.0, 2.0),
                ..Transform::default()
            },
            ..bone("target", None)
        });
        let driven = skel.add_bone(bone("driven", None));
        (skel, target, driven)
    }

    /// The acceptance case: mix 0.5 lands exactly halfway, by hand-computed
    /// angle rather than by "looks about right".
    #[test]
    fn a_rotation_constraint_at_half_mix_lands_halfway() {
        let (mut skel, target, driven) = target_and_driven();
        skel.add_constraint(Constraint::Transform(TransformConstraint {
            mix_rotate: 0.5,
            ..TransformConstraint::rotation_only("look", target, vec![driven])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        // Driven starts at 0°, target is at 90°, so halfway is 45°.
        let angle = pose.world_decomposed(driven).rotation;
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < EPS,
            "expected 45°, got {}°",
            angle.to_degrees()
        );
    }

    #[test]
    fn mix_zero_changes_nothing_and_mix_one_matches_the_target() {
        for (mix, expected) in [(0.0, 0.0), (1.0, std::f32::consts::FRAC_PI_2)] {
            let (mut skel, target, driven) = target_and_driven();
            skel.add_constraint(Constraint::Transform(TransformConstraint {
                mix_rotate: mix,
                ..TransformConstraint::rotation_only("look", target, vec![driven])
            }));
            let mut pose = Pose::new();
            evaluate(&skel, &[], &mut pose);
            let angle = pose.world_decomposed(driven).rotation;
            assert!(
                (angle - expected).abs() < EPS,
                "mix {mix}: expected {}°, got {}°",
                expected.to_degrees(),
                angle.to_degrees()
            );
        }
    }

    /// Each channel is independent: a rotation-only constraint must not drag
    /// position or scale along with it.
    #[test]
    fn channels_are_independent() {
        let (mut skel, target, driven) = target_and_driven();
        skel.add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
            "look",
            target,
            vec![driven],
        )));
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let world = pose.world_decomposed(driven);
        assert!(
            world.position.length() < EPS,
            "translate was untouched: {:?}",
            world.position
        );
        assert!(
            (world.scale.x - 1.0).abs() < EPS && (world.scale.y - 1.0).abs() < EPS,
            "scale was untouched: {:?}",
            world.scale
        );
    }

    /// The offset is what makes "track that, but stay 10° off it" possible.
    #[test]
    fn the_offset_is_added_to_the_targets_transform() {
        let (mut skel, target, driven) = target_and_driven();
        skel.add_constraint(Constraint::Transform(TransformConstraint {
            offsets: Transform {
                rotation: 10.0_f32.to_radians(),
                ..Transform::default()
            },
            ..TransformConstraint::rotation_only("look", target, vec![driven])
        }));
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let angle = pose.world_decomposed(driven).rotation.to_degrees();
        assert!((angle - 100.0).abs() < 1e-2, "expected 100°, got {angle}°");
    }

    /// Relative mode adds to what the bone already has instead of replacing it,
    /// which is what lets a constraint layer on top of an animation.
    #[test]
    fn relative_mode_adds_rather_than_replaces() {
        let (mut skel, target, _) = target_and_driven();
        let driven = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: 20.0_f32.to_radians(),
                ..Transform::default()
            },
            ..bone("posed", None)
        });
        skel.add_constraint(Constraint::Transform(TransformConstraint {
            relative: true,
            ..TransformConstraint::rotation_only("add", target, vec![driven])
        }));
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        // 20° of its own plus 90° from the target.
        let angle = pose.world_decomposed(driven).rotation.to_degrees();
        assert!((angle - 110.0).abs() < 1e-2, "expected 110°, got {angle}°");
    }

    /// A driven bone's children have to follow, which only happens because the
    /// constraint writes a *local* transform and re-runs FK (defect D3).
    #[test]
    fn children_of_a_constrained_bone_follow_it() {
        let (mut skel, target, driven) = target_and_driven();
        let child = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(10.0, 0.0),
                ..Transform::default()
            },
            ..bone("child", Some(driven))
        });

        let mut before = Pose::new();
        evaluate(&skel, &[], &mut before);
        let child_before = before.world_position(child);

        skel.add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
            "look",
            target,
            vec![driven],
        )));
        let mut after = Pose::new();
        evaluate(&skel, &[], &mut after);
        let child_after = after.world_position(child);

        // The child sat at +10 on X; a 90° turn swings it to +10 on Y.
        assert!(
            (child_before - glam::vec2(10.0, 0.0)).length() < EPS,
            "child started on the X axis: {child_before:?}"
        );
        assert!(
            (child_after - glam::vec2(0.0, 10.0)).length() < 1e-2,
            "child swung with its parent: {child_after:?}"
        );
    }

    /// Constraint order is the whole reason the list is ordered: two
    /// constraints writing one bone resolve last-wins, deterministically.
    #[test]
    fn constraint_order_decides_the_result() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: 30.0_f32.to_radians(),
                ..Transform::default()
            },
            ..bone("a", None)
        });
        let b = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: 60.0_f32.to_radians(),
                ..Transform::default()
            },
            ..bone("b", None)
        });
        let driven = skel.add_bone(bone("driven", None));
        let first = skel.add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
            "from_a",
            a,
            vec![driven],
        )));
        let second = skel.add_constraint(Constraint::Transform(
            TransformConstraint::rotation_only("from_b", b, vec![driven]),
        ));

        skel.constraint_order = vec![first, second];
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        let b_last = pose.world_decomposed(driven).rotation.to_degrees();

        skel.constraint_order = vec![second, first];
        evaluate(&skel, &[], &mut pose);
        let a_last = pose.world_decomposed(driven).rotation.to_degrees();

        assert!((b_last - 60.0).abs() < 1e-2, "b ran last: {b_last}°");
        assert!((a_last - 30.0).abs() < 1e-2, "a ran last: {a_last}°");
    }

    /// A constraint pointed at its own driven bone would read its own output.
    #[test]
    fn a_bone_cannot_be_driven_by_itself() {
        let mut skel = Skeleton::new();
        let solo = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: 0.3,
                ..Transform::default()
            },
            ..bone("solo", None)
        });
        skel.add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
            "self",
            solo,
            vec![solo],
        )));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        assert!(
            (pose.world_decomposed(solo).rotation - 0.3).abs() < EPS,
            "the self-reference was skipped, not applied"
        );
    }

    /// A `TransformConstraintMix` key overrides the authored mixes, so a
    /// constraint can fade in over an animation.
    #[test]
    fn a_mix_timeline_overrides_the_authored_mixes() {
        let (mut skel, target, driven) = target_and_driven();
        let cid = skel.add_constraint(Constraint::Transform(TransformConstraint::rotation_only(
            "look",
            target,
            vec![driven],
        )));

        let anim = Animation {
            name: "fade".into(),
            duration: 1.0,
            looping: false,
            events: Vec::new(),
            timelines: vec![Timeline::TransformConstraintMix {
                constraint: cid,
                keys: vec![
                    Key {
                        time: 0.0,
                        value: [0.0; 4],
                        interp: crate::animation::Interp::Linear,
                    },
                    Key {
                        time: 1.0,
                        value: [1.0, 0.0, 0.0, 0.0],
                        interp: crate::animation::Interp::Linear,
                    },
                ],
            }],
        };

        let mut pose = Pose::new();
        evaluate(&skel, &[(&anim, 0.0, 1.0)], &mut pose);
        assert!(
            pose.world_decomposed(driven).rotation.abs() < EPS,
            "mix 0 at t=0 leaves the bone alone"
        );

        evaluate(&skel, &[(&anim, 0.5, 1.0)], &mut pose);
        let half = pose.world_decomposed(driven).rotation.to_degrees();
        assert!((half - 45.0).abs() < 1e-2, "mix 0.5 is halfway: {half}°");

        evaluate(&skel, &[(&anim, 1.0, 1.0)], &mut pose);
        let full = pose.world_decomposed(driven).rotation.to_degrees();
        assert!(
            (full - 90.0).abs() < 1e-2,
            "mix 1 matches the target: {full}°"
        );
    }

    /// Translation is blended in world space but written locally, so a
    /// constrained bone under a rotated parent must not slide off at an angle.
    #[test]
    fn a_translate_constraint_under_a_rotated_parent_lands_on_the_target() {
        let mut skel = Skeleton::new();
        let target = skel.add_bone(Bone {
            local_transform: Transform {
                position: glam::vec2(50.0, 0.0),
                ..Transform::default()
            },
            ..bone("target", None)
        });
        // The parent is turned 90°, so a naive local write sends the child along
        // the wrong axis.
        let parent = skel.add_bone(Bone {
            local_transform: Transform {
                rotation: std::f32::consts::FRAC_PI_2,
                ..Transform::default()
            },
            ..bone("parent", None)
        });
        let driven = skel.add_bone(bone("driven", Some(parent)));
        skel.add_constraint(Constraint::Transform(TransformConstraint {
            mix_rotate: 0.0,
            mix_translate: 1.0,
            ..TransformConstraint::rotation_only("follow", target, vec![driven])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        let world = pose.world_position(driven);
        assert!(
            (world - glam::vec2(50.0, 0.0)).length() < 1e-2,
            "driven should sit on the target, got {world:?}"
        );
    }
    // ── IK completeness (T-504) ──────────────────────────────────────────

    /// A chain of `n` bones, each 10 long, laid end to end along +X, plus a
    /// free target bone. Returns the chain and the target.
    fn ik_chain(n: usize) -> (Skeleton, Vec<BoneId>, BoneId) {
        let mut skel = Skeleton::new();
        let mut chain = Vec::new();
        let mut parent = None;
        for i in 0..n {
            let id = skel.add_bone(Bone {
                local_transform: Transform {
                    // The first bone sits at the origin; each next one starts at
                    // its parent's tip.
                    position: if i == 0 {
                        glam::Vec2::ZERO
                    } else {
                        glam::vec2(10.0, 0.0)
                    },
                    ..Transform::default()
                },
                ..bone(&format!("b{i}"), parent)
            });
            chain.push(id);
            parent = Some(id);
        }
        let target = skel.add_bone(bone("target", None));
        (skel, chain, target)
    }

    /// The acceptance case: a 3-bone chain reaches a reachable target within
    /// tolerance, which two-bone trigonometry cannot do at all.
    #[test]
    fn a_three_bone_chain_reaches_its_target() {
        let (mut skel, chain, target) = ik_chain(3);
        // Well inside the 30-unit reach, and off-axis so every bone has to move.
        skel.bones[target].local_transform.position = glam::vec2(12.0, 14.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let tip = pose.world_tip(&skel, *chain.last().unwrap());
        let goal = pose.world_position(target);
        assert!(
            (tip - goal).length() <= crate::constraints::FABRIK_TOLERANCE * 4.0,
            "tip {tip:?} did not reach {goal:?}"
        );
    }

    /// Bone lengths are invariants of the rig: FABRIK may move joints, but a
    /// solved chain that changed length has torn the skeleton apart.
    #[test]
    fn fabrik_preserves_bone_lengths() {
        let (mut skel, chain, target) = ik_chain(4);
        skel.bones[target].local_transform.position = glam::vec2(-5.0, 18.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        for &b in &chain {
            let length = pose.world_length(&skel, b);
            assert!(
                (length - 10.0).abs() < 1e-2,
                "bone stretched to {length} without `stretch` set"
            );
        }
    }

    /// Out of reach, a chain should point straight at the target rather than
    /// curling: every joint on the line from root to target.
    #[test]
    fn an_unreachable_target_extends_the_chain_straight() {
        let (mut skel, chain, target) = ik_chain(3);
        skel.bones[target].local_transform.position = glam::vec2(0.0, 100.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        // Straight up: every bone's world rotation is 90°.
        for &b in &chain {
            let angle = pose.world_decomposed(b).rotation.to_degrees();
            assert!(
                (angle - 90.0).abs() < 1.0,
                "bone points at {angle}°, expected 90°"
            );
        }
    }

    /// Stretch lets a chain reach past its natural length — but only up to the
    /// configured limit, or an out-of-range target becomes a rubber band.
    #[test]
    fn stretch_extends_the_chain_only_up_to_its_limit() {
        let (mut skel, chain, target) = ik_chain(2);
        // 40 units away with a 20-unit reach: it cannot get there even at the
        // limit, so the limit is what is being measured.
        skel.bones[target].local_transform.position = glam::vec2(40.0, 0.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            stretch: true,
            stretch_limit: 1.5,
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let reach: f32 = chain.iter().map(|b| pose.world_length(&skel, *b)).sum();
        assert!(
            (reach - 30.0).abs() < 0.1,
            "20 units of bone at the 1.5 limit is 30, got {reach}"
        );
    }

    #[test]
    fn stretch_does_nothing_when_the_target_is_in_reach() {
        let (mut skel, chain, target) = ik_chain(2);
        skel.bones[target].local_transform.position = glam::vec2(12.0, 0.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            stretch: true,
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);

        let reach: f32 = chain.iter().map(|b| pose.world_length(&skel, *b)).sum();
        assert!(
            (reach - 20.0).abs() < 0.01,
            "a reachable target needs no stretch, got {reach}"
        );
    }

    /// Softness trades exactness near full extension for a chain that does not
    /// visibly snap. Inside the soft zone the tip should fall *short* of a
    /// target it would otherwise hit exactly.
    #[test]
    fn softness_eases_the_approach_to_full_extension() {
        let solve = |softness: f32| {
            let (mut skel, chain, target) = ik_chain(2);
            // 19 of a possible 20: just inside reach, inside a 5-unit soft zone.
            skel.bones[target].local_transform.position = glam::vec2(19.0, 0.0);
            skel.add_constraint(Constraint::Ik(IkConstraint {
                softness,
                bones: chain.clone(),
                ..IkConstraint::aim("reach", target, chain[0])
            }));
            let mut pose = Pose::new();
            evaluate(&skel, &[], &mut pose);
            let tip = pose.world_tip(&skel, chain[1]);
            (tip - pose.world_position(target)).length()
        };

        let hard = solve(0.0);
        let soft = solve(5.0);
        assert!(hard < 0.1, "without softness the chain reaches: {hard}");
        assert!(
            soft > hard + 0.1,
            "softness should hold the chain back: hard {hard}, soft {soft}"
        );
    }

    /// Bend direction is stepped so a chain never interpolates through "no
    /// preference", which is where a flip becomes possible.
    #[test]
    fn an_animated_bend_direction_is_stepped_and_flip_free() {
        let (mut skel, chain, target) = ik_chain(2);
        skel.bones[target].local_transform.position = glam::vec2(14.0, 0.0);
        let cid = skel.add_constraint(Constraint::Ik(IkConstraint {
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let anim = Animation {
            name: "flip".into(),
            duration: 1.0,
            looping: false,
            events: Vec::new(),
            timelines: vec![Timeline::IkBendDirection {
                constraint: cid,
                keys: vec![
                    Key {
                        time: 0.0,
                        value: 1.0,
                        interp: crate::animation::Interp::Linear,
                    },
                    Key {
                        time: 1.0,
                        value: -1.0,
                        interp: crate::animation::Interp::Linear,
                    },
                ],
            }],
        };

        let mut pose = Pose::new();
        // The elbow's sign is the bend; it must never sit near zero, which is
        // what a linearly interpolated direction would produce mid-clip.
        let mut signs = Vec::new();
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            evaluate(&skel, &[(&anim, t, 1.0)], &mut pose);
            let elbow_y = pose.world_position(chain[1]).y;
            assert!(
                elbow_y.abs() > 0.5 || t == 0.0,
                "the elbow flattened at t={t} (y={elbow_y}) — the direction interpolated"
            );
            signs.push(elbow_y.signum());
        }
        assert!(
            signs.first() != signs.last(),
            "the bend direction key had no effect"
        );
    }

    /// A zero-length bone has no direction to solve for; normalizing its
    /// segment would spread NaN through the chain.
    #[test]
    fn a_zero_length_bone_is_refused_rather_than_producing_nan() {
        let (mut skel, chain, target) = ik_chain(3);
        skel.bones[chain[1]].length = 0.0;
        skel.bones[target].local_transform.position = glam::vec2(5.0, 5.0);
        skel.add_constraint(Constraint::Ik(IkConstraint {
            bones: chain.clone(),
            ..IkConstraint::aim("reach", target, chain[0])
        }));

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        for &b in &chain {
            let p = pose.world_position(b);
            assert!(p.x.is_finite() && p.y.is_finite(), "NaN at {b:?}: {p:?}");
        }
    }
}
