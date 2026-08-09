//! Runtime playback behaviour (T-604).
//!
//! The load-bearing test here is [`the_runtime_pose_matches_a_direct_evaluate`]:
//! if the runtime and the editor can disagree about a pose, everything else this
//! crate does is worthless, because a rig would animate correctly in the tool and
//! wrongly in the game.

use ankhimate_core::animation::{Animation, EventKey, Key, Timeline};
use ankhimate_core::math::Transform;
use ankhimate_core::pose::Pose;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slotmap::SlotMap;
use ankhimate_core::transforms::Inherit;
use ankhimate_runtime::{AnimationState, Rig};

fn rig() -> Rig {
    let mut skeleton = Skeleton::new();
    let root = skeleton.add_bone(Bone {
        name: "root".into(),
        parent: None,
        length: 20.0,
        local_transform: Transform::default(),
        inherit: Inherit::default(),
        color: Bone::default_color(),
    });
    skeleton.add_bone(Bone {
        name: "spine".into(),
        parent: Some(root),
        length: 20.0,
        local_transform: Transform {
            position: glam::vec2(20.0, 0.0),
            ..Transform::default()
        },
        inherit: Inherit::default(),
        color: Bone::default_color(),
    });

    let spine = skeleton
        .bones
        .iter()
        .find(|(_, b)| b.name == "spine")
        .map(|(id, _)| id)
        .unwrap();

    let mut animations = SlotMap::with_key();
    animations.insert(Animation {
        name: "walk".into(),
        duration: 1.0,
        looping: true,
        timelines: vec![Timeline::BoneRotate {
            bone: spine,
            keys: vec![
                Key::linear(0.0, 0.0),
                Key::linear(1.0, std::f32::consts::FRAC_PI_2),
            ],
        }],
        events: vec![EventKey {
            time: 0.5,
            name: "footstep".into(),
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            audio: String::new(),
            volume: 1.0,
            balance: 0.0,
        }],
        markers: Vec::new(),
        bone_offsets: Vec::new(),
    });
    animations.insert(Animation {
        name: "idle".into(),
        duration: 1.0,
        looping: true,
        timelines: vec![Timeline::BoneRotate {
            bone: spine,
            keys: vec![Key::linear(0.0, -std::f32::consts::FRAC_PI_2)],
        }],
        events: Vec::new(),
        markers: Vec::new(),
        bone_offsets: Vec::new(),
    });

    Rig {
        skeleton,
        animations,
        assets: ankhimate_core::assets::AssetDb::new(),
        name: "test".into(),
        fps: 30,
    }
}

fn spine_of(rig: &Rig) -> ankhimate_core::ids::BoneId {
    rig.skeleton
        .bones
        .iter()
        .find(|(_, b)| b.name == "spine")
        .map(|(id, _)| id)
        .unwrap()
}

/// The whole justification for this crate being thin. If these ever diverge, a
/// rig animates one way in the editor and another in the game.
#[test]
fn the_runtime_pose_matches_a_direct_evaluate() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let spine = spine_of(&rig);

    let mut state = AnimationState::new();
    state.play(walk, true);
    let mut runtime_pose = Pose::new();
    state.update(&rig, 0.25, &mut runtime_pose);

    // What the editor would compute for the same clip at the same time.
    let mut direct = Pose::new();
    let anim = &rig.animations[walk];
    ankhimate_core::pose::evaluate(&rig.skeleton, &[(anim, 0.25, 1.0)], &mut direct);

    // The tip, not the origin: rotating a bone never moves its own origin, so
    // an origin comparison would pass even if the rotation were dropped
    // entirely.
    let a = runtime_pose.world_tip(&rig.skeleton, spine);
    let b = direct.world_tip(&rig.skeleton, spine);
    assert!(
        (a - b).length() < 1e-5,
        "runtime pose {a:?} differs from a direct evaluate {b:?}"
    );
}

#[test]
fn playing_advances_the_clock() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let mut state = AnimationState::new();
    state.play(walk, true);

    let mut pose = Pose::new();
    state.update(&rig, 0.3, &mut pose);
    assert!((state.current().unwrap().time - 0.3).abs() < 1e-6);
}

#[test]
fn a_looping_clip_wraps_rather_than_running_off_the_end() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let mut state = AnimationState::new();
    state.play(walk, true);

    let mut pose = Pose::new();
    state.update(&rig, 1.25, &mut pose);
    let time = state.current().unwrap().time;
    assert!(
        (0.0..1.0).contains(&time),
        "a looping clip should wrap into its duration, got {time}"
    );
}

#[test]
fn a_one_shot_clip_stops_at_its_end() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let mut state = AnimationState::new();
    state.play(walk, false);

    let mut pose = Pose::new();
    state.update(&rig, 5.0, &mut pose);
    assert!((state.current().unwrap().time - 1.0).abs() < 1e-6);
}

/// A footstep at 0.95 of a 1s clip must not vanish because a frame straddled the
/// loop point — the case a naive `time > last_time` check drops every lap.
#[test]
fn an_event_fires_across_a_loop_boundary() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let mut state = AnimationState::new();
    state.play(walk, true);

    let mut pose = Pose::new();
    // Step to 0.4 — before the event at 0.5.
    state.update(&rig, 0.4, &mut pose);
    assert!(state.events().is_empty(), "nothing fires before 0.5");

    // Step across it.
    state.update(&rig, 0.2, &mut pose);
    assert_eq!(state.events().len(), 1, "the footstep fires once");
    assert_eq!(state.events()[0].name, "footstep");

    // A full lap fires it exactly once more.
    state.update(&rig, 1.0, &mut pose);
    assert_eq!(
        state.events().len(),
        1,
        "a full lap fires the event once, not zero or twice"
    );
}

#[test]
fn events_are_cleared_between_updates() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let mut state = AnimationState::new();
    state.play(walk, true);

    let mut pose = Pose::new();
    state.update(&rig, 0.6, &mut pose);
    assert_eq!(state.events().len(), 1);
    state.update(&rig, 0.1, &mut pose);
    assert!(
        state.events().is_empty(),
        "last frame's events must not linger"
    );
}

// ── Crossfade ───────────────────────────────────────────────────────────

/// Mid-fade the pose must sit *between* the two clips. Blending two finished
/// poses would give a different answer than weighting the inputs, and the editor
/// weights the inputs.
#[test]
fn a_crossfade_lands_between_the_two_clips() {
    let rig = rig();
    let (walk, idle) = (
        rig.animation("walk").unwrap(),
        rig.animation("idle").unwrap(),
    );
    let spine = spine_of(&rig);

    let mut state = AnimationState::new();
    state.play(walk, true);
    let mut pose = Pose::new();
    state.update(&rig, 0.0, &mut pose);
    let walk_only = pose.world_tip(&rig.skeleton, spine);

    state.crossfade(idle, 1.0, true);
    // Halfway through the fade. Measured at the bone's *tip*: rotating a bone
    // never moves its own origin, so comparing origins would pass no matter what
    // the fade did.
    state.update(&rig, 0.5, &mut pose);
    let mixed = pose.world_tip(&rig.skeleton, spine);

    let mut idle_pose = Pose::new();
    let idle_anim = &rig.animations[idle];
    ankhimate_core::pose::evaluate(&rig.skeleton, &[(idle_anim, 0.5, 1.0)], &mut idle_pose);
    let idle_only = idle_pose.world_tip(&rig.skeleton, spine);

    let to_walk = (mixed - walk_only).length();
    let to_idle = (mixed - idle_only).length();
    assert!(
        to_walk > 1e-3 && to_idle > 1e-3,
        "a half-done fade should match neither end exactly: {mixed:?}"
    );
}

#[test]
fn a_finished_crossfade_drops_the_outgoing_clip() {
    let rig = rig();
    let (walk, idle) = (
        rig.animation("walk").unwrap(),
        rig.animation("idle").unwrap(),
    );

    let mut state = AnimationState::new();
    state.play(walk, true);
    state.crossfade(idle, 0.5, true);

    let mut pose = Pose::new();
    state.update(&rig, 0.6, &mut pose);
    assert_eq!(
        state.current().unwrap().animation,
        idle,
        "the incoming clip owns the pose once the fade completes"
    );
}

/// A game calling `crossfade` every frame in its update loop must not restart
/// the fade each time — that would freeze the rig at alpha 0 forever.
#[test]
fn crossfading_to_what_is_already_fading_in_does_not_restart_it() {
    let rig = rig();
    let (walk, idle) = (
        rig.animation("walk").unwrap(),
        rig.animation("idle").unwrap(),
    );

    let mut state = AnimationState::new();
    state.play(walk, true);
    let mut pose = Pose::new();

    for _ in 0..10 {
        state.crossfade(idle, 0.5, true);
        state.update(&rig, 0.1, &mut pose);
    }
    assert_eq!(
        state.current().unwrap().animation,
        idle,
        "the fade completed"
    );
}

#[test]
fn a_zero_length_crossfade_is_an_immediate_cut() {
    let rig = rig();
    let (walk, idle) = (
        rig.animation("walk").unwrap(),
        rig.animation("idle").unwrap(),
    );

    let mut state = AnimationState::new();
    state.play(walk, true);
    state.crossfade(idle, 0.0, true);

    let mut pose = Pose::new();
    state.update(&rig, 0.0, &mut pose);
    assert_eq!(state.current().unwrap().animation, idle);
}

// ── Determinism ─────────────────────────────────────────────────────────

/// `evaluate()` is deterministic (PLAN §2.6) and the runtime must not undo that
/// by reading a clock or iterating a hash map.
#[test]
fn the_same_dt_sequence_gives_the_same_pose() {
    let rig = rig();
    let walk = rig.animation("walk").unwrap();
    let spine = spine_of(&rig);

    let run = || {
        let mut state = AnimationState::new();
        state.play(walk, true);
        let mut pose = Pose::new();
        for _ in 0..20 {
            state.update(&rig, 1.0 / 60.0, &mut pose);
        }
        pose.world_tip(&rig.skeleton, spine)
    };

    assert_eq!(run(), run());
}

#[test]
fn an_empty_state_leaves_the_setup_pose() {
    let rig = rig();
    let spine = spine_of(&rig);
    let mut state = AnimationState::new();
    let mut pose = Pose::new();
    state.update(&rig, 0.1, &mut pose);

    let mut setup = Pose::new();
    ankhimate_core::pose::evaluate(&rig.skeleton, &[], &mut setup);
    assert_eq!(pose.world_position(spine), setup.world_position(spine));
}

#[test]
fn animation_names_are_listed_sorted() {
    let rig = rig();
    assert_eq!(rig.animation_names(), vec!["idle", "walk"]);
}

#[test]
fn a_missing_animation_is_none_rather_than_a_panic() {
    let rig = rig();
    assert!(rig.animation("does-not-exist").is_none());
}
