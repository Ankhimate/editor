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
use crate::constraints::{Constraint, IkConstraint, solve_aim, solve_two_bone_ik};
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
                                skel.constraints
                                    .get(*constraint)
                                    .map(|Constraint::Ik(ik)| ik.mix)
                            })
                            .unwrap_or(0.0);
                        out.ik_mix
                            .insert(*constraint, current + (mix - current) * alpha);
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
                apply_ik(skel, out, ik, mix);
            }
        }
    }
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

    let target_pos = out.world_position(ik.target);

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
        // Two-bone IK.
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
        // Longer chains (FABRIK et al.) are post-v1; ignore rather than
        // half-solve and produce a silently wrong pose.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Key;
    use crate::constraints::IkConstraint;
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
}
