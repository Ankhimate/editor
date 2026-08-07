//! Regenerate the checked-in `samples/tentacle.ankh` (T-908).
//!
//! Run with `cargo run -p ankhimate-formats --example gen_tentacle`. The sample
//! is committed; this exists so it can be rebuilt deterministically if the schema
//! changes, not so it is generated at test time.
//!
//! # Why this sample exists
//!
//! Every other 2D skeletal editor caps an IK chain at two bones, and the ones
//! that document the cap describe it as deliberate — three or more bones have
//! infinitely many solutions for a given target, which is real, and they answer
//! it by refusing. We answer it with FABRIK plus `bend_direction` to choose the
//! side, so a chain here can be any length.
//!
//! That is the single largest rigging-capability difference in our favour and it
//! is invisible: a rigger arriving from another tool assumes two is the ceiling
//! and never tries three. `samples/spineboy.ankh` cannot demonstrate it — it was
//! authored elsewhere, so its longest chain is two by construction. Hence a
//! sample whose whole point is a chain that could not exist there.
//!
//! Eight bones, one IK constraint, one target. Drag the target and the whole
//! tentacle curls. In a two-bone editor this is seven constraints hand-chained
//! together, and it does not behave the same.

use ankhimate_core::animation::{Animation, Key, Timeline};
use ankhimate_core::constraints::{Constraint, IkConstraint};
use ankhimate_core::ids::BoneId;
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slotmap::SlotMap;

/// Bones in the tentacle, not counting the root it hangs from.
const SEGMENTS: usize = 8;
/// Length of the first segment; each one after tapers.
const BASE_LENGTH: f32 = 46.0;
/// How much of the previous segment's length each one keeps.
const TAPER: f32 = 0.9;

fn main() {
    let (skeleton, animations) = build();

    let assets = ankhimate_core::assets::AssetDb::new();
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &skeleton,
        animations: &animations,
        assets: &assets,
        name: "tentacle",
        fps: 30,
    })
    .unwrap();

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/tentacle.ankh");
    ankhimate_formats::container::write(std::path::Path::new(path), &json, &[]).unwrap();
    println!("wrote {path}");
}

fn build() -> (
    Skeleton,
    SlotMap<ankhimate_core::ids::AnimationId, Animation>,
) {
    let mut skel = Skeleton::new();

    let root = skel.add_bone(Bone {
        name: "root".into(),
        parent: None,
        length: 30.0,
        local_transform: Transform::default(),
        inherit: Default::default(),
        color: Bone::default_color(),
    });

    // The chain, each segment shorter than the last. A taper is not decoration:
    // an even chain bends into a circular arc, which hides whether the solver is
    // distributing the bend or just rotating the base.
    let mut chain: Vec<BoneId> = Vec::with_capacity(SEGMENTS);
    let mut parent = root;
    let mut length = BASE_LENGTH;
    for i in 0..SEGMENTS {
        let bone = skel.add_bone(Bone {
            name: format!("tentacle{}", i + 1),
            parent: Some(parent),
            length,
            local_transform: Transform {
                // Each segment starts at its parent's tip. The first hangs off
                // the root's origin instead, so the chain begins where the rig
                // does rather than a bone-length away from it.
                position: glam::vec2(if i == 0 { 0.0 } else { length / TAPER }, 0.0),
                // A slight setup-pose curl. A dead-straight chain is the one
                // starting shape FABRIK has to nudge off before it can pick a
                // side, so the sample opens in a state that has already chosen.
                rotation: if i == 0 { 0.0 } else { 6.0_f32.to_radians() },
                scale: glam::vec2(1.0, 1.0),
                shear: glam::vec2(0.0, 0.0),
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        chain.push(bone);
        parent = bone;
        length *= TAPER;
    }

    // The target sits outside the hierarchy, as IK targets do: it is a handle
    // the animator moves, and a target that inherited the chain's motion would
    // chase itself.
    let reach: f32 = (0..SEGMENTS)
        .map(|i| BASE_LENGTH * TAPER.powi(i as i32))
        .sum();
    let target = skel.add_bone(Bone {
        name: "tentacle-target".into(),
        parent: None,
        length: 0.0,
        local_transform: Transform {
            // Placed at about two thirds of full reach, so the chain opens
            // curled rather than stretched straight — a straight chain looks
            // like no IK is running at all.
            position: glam::vec2(reach * 0.62, reach * 0.30),
            ..Default::default()
        },
        inherit: Default::default(),
        color: Bone::default_color(),
    });

    // One constraint over all eight. This is the line that no two-bone editor
    // can express.
    skel.add_constraint(Constraint::Ik(IkConstraint::chain(
        "tentacle-ik",
        target,
        chain,
    )));

    // A curl animation driving the *target*, not the bones: the point of the
    // sample is that eight bones follow one handle, and keying the bones
    // directly would demonstrate the opposite.
    let mut animations = SlotMap::with_key();
    animations.insert(Animation {
        name: "curl".into(),
        duration: 2.0,
        // Offsets from the setup pose, not absolute positions — `BoneTranslate`
        // adds to the setup translation, so keying `(0,0)` at either end returns
        // the target exactly where the rig opens.
        timelines: vec![Timeline::BoneTranslate {
            bone: target,
            keys: vec![
                Key::linear(0.0, glam::Vec2::ZERO),
                Key::linear(1.0, glam::vec2(-reach * 0.32, -reach * 0.65)),
                Key::linear(2.0, glam::Vec2::ZERO),
            ],
        }],
        events: Vec::new(),
        looping: true,
    });

    (skel, animations)
}
