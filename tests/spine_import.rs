//! Reading Spine JSON (T-6xx).
//!
//! These pin the decisions an importer makes that a caller cannot see: which
//! field name a version uses, what happens to a curve this model cannot hold,
//! and — most importantly — that a thing which could not be carried across is
//! *reported* rather than dropped. A silent drop is the expensive kind: the rig
//! opens, looks right, and plays back wrong.

use ankhimate_formats::spine::{self, Images};

/// A minimal 4.x skeleton: two bones, one transform constraint, one animation.
fn skeleton_4x() -> String {
    r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [
        { "name": "root" },
        { "name": "spine", "parent": "root", "x": 10, "rotation": 90 }
      ],
      "slots": [{ "name": "torso", "bone": "spine" }],
      "constraints": [
        { "type": "transform", "name": "follow", "source": "root",
          "bones": ["spine"], "mixRotate": 0.5 }
      ],
      "animations": {
        "walk": {
          "bones": {
            "spine": {
              "rotate": [
                { "value": 0, "curve": [0.1, 0, 0.2, 30] },
                { "time": 0.5, "value": 30 }
              ]
            }
          }
        }
      }
    }"#
    .to_string()
}

#[test]
fn a_4x_skeleton_reads_its_bones_slots_and_animations() {
    let loaded = spine::read(&skeleton_4x(), Images::None, "rig").expect("a 4.x file reads");
    assert_eq!(loaded.skeleton.bones.len(), 2);
    assert_eq!(loaded.skeleton.slots.len(), 1);
    assert_eq!(loaded.animations.len(), 1);
}

/// 4.x renamed a transform constraint's followed bone from `target` to `source`.
///
/// Reading only 3.8's name made the lookup fail, and the `continue` that
/// followed dropped the constraint in silence — every one of spineboy's seven,
/// leaving a rig that posed almost right with nothing to point at.
#[test]
fn a_transform_constraint_reads_either_versions_source_field() {
    let loaded = spine::read(&skeleton_4x(), Images::None, "rig").expect("reads");
    assert_eq!(
        loaded.skeleton.constraints.len(),
        1,
        "4.x names it `source`: {:?}",
        loaded.report.dangling
    );

    // 3.8 spelled the same field `target`.
    let old = skeleton_4x().replace("\"source\": \"root\"", "\"target\": \"root\"");
    let loaded = spine::read(&old, Images::None, "rig").expect("reads");
    assert_eq!(
        loaded.skeleton.constraints.len(),
        1,
        "3.8 names it `target`"
    );
}

/// 3.8 kept each constraint kind in its own array; 4.x uses one tagged array.
#[test]
fn both_constraint_layouts_are_read() {
    let old = r#"{
      "skeleton": { "spine": "3.8.99" },
      "bones": [{ "name": "root" }, { "name": "arm", "parent": "root" }],
      "ik": [{ "name": "reach", "target": "arm", "bones": ["root"] }],
      "transform": [{ "name": "follow", "target": "root", "bones": ["arm"] }]
    }"#;
    let loaded = spine::read(old, Images::None, "rig").expect("a 3.8 file reads");
    assert_eq!(loaded.skeleton.constraints.len(), 2);
}

/// A transform channel the file does not mention is **off**, not full.
///
/// A Spine file writes only the channels its constraint drives. Defaulting the
/// rest to 1 turns a constraint the artist switched off into one that drags its
/// bones along every axis it never mentioned.
///
/// Spineboy's four `aim-*` constraints say `mixRotate: 0` and nothing else. Read
/// with 1.0 defaults they pulled the torso, head and gun arm toward a crosshair
/// parked 645 units away — in every animation, on a rig whose every stored key
/// matched the source exactly.
#[test]
fn an_unmentioned_transform_mix_is_off() {
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }, { "name": "arm", "parent": "root" }],
      "constraints": [
        { "type": "transform", "name": "aim", "source": "root",
          "bones": ["arm"], "mixRotate": 0 },
        { "type": "transform", "name": "shoulder", "source": "root",
          "bones": ["arm"], "mixX": -1 }
      ]
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    let of = |name: &str| {
        loaded
            .skeleton
            .constraints
            .values()
            .find_map(|c| match c {
                ankhimate_core::constraints::Constraint::Transform(t) if t.name == name => {
                    Some((t.mix_rotate, t.mix_translate, t.mix_scale, t.mix_shear))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no constraint named {name}"))
    };

    // Every channel off: the constraint does nothing, which is what the file says.
    assert_eq!(of("aim"), (0.0, 0.0, 0.0, 0.0));

    // Only the channel it names is live — and `mixY` inherits `mixX` rather
    // than falling back, so a shoulder mirroring on X does not also move on Y.
    assert_eq!(of("shoulder"), (0.0, -1.0, 0.0, 0.0));
}

/// A constraint kind this model has no equivalent for is named, not ignored.
#[test]
fn an_unsupported_constraint_is_reported_rather_than_dropped() {
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "constraints": [
        { "type": "path", "name": "rail", "bones": ["root"] },
        { "type": "physics", "name": "wobble", "bone": "root" }
      ]
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    assert_eq!(loaded.skeleton.constraints.len(), 0);
    let named: Vec<&str> = loaded
        .report
        .lossy
        .iter()
        .map(|l| l.where_.as_str())
        .collect();
    assert!(named.contains(&"rail"), "{named:?}");
    assert!(named.contains(&"wobble"), "{named:?}");
}

/// A value control point outside the keys survives import untouched.
///
/// Spine's control points are absolute and unconstrained, so a curve can swing
/// past its own endpoints — the wind-up before a punch. This model represents
/// that: `ease()` runs the value handle through a plain cubic and consumers lerp
/// unclamped, so a fraction outside 0..1 extrapolates.
///
/// The importer used to clamp all four components. On Esoteric's spineboy that
/// flattened 211 curves, most of them the snap in `death`.
#[test]
fn a_value_overshoot_survives_import_unclamped() {
    // The value control point sits at -20, below the segment's start of 0 —
    // normalized against the 0..30 span, that is -0.667.
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "animations": { "walk": { "bones": { "root": { "rotate": [
        { "value": 0, "curve": [0.1, -20, 0.2, 30] },
        { "time": 0.5, "value": 30 }
      ] } } } }
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    let anim = loaded.animations.values().next().expect("one animation");
    let keys = match anim.timelines.first().expect("a timeline") {
        ankhimate_core::animation::Timeline::BoneRotate { keys, .. } => keys,
        other => panic!("expected a rotate timeline, got {other:?}"),
    };
    // The easing belongs to the key it arrives at, so key 1 holds it.
    match keys[1].interp {
        ankhimate_core::animation::Interp::Bezier { out_handle, .. } => assert!(
            (out_handle.y + 0.667).abs() < 1e-3,
            "the overshoot is kept, not clamped to 0: {out_handle:?}"
        ),
        other => panic!("expected a bezier, got {other:?}"),
    }

    // And nothing is reported as lost, because nothing was: reporting a loss we
    // no longer take would train the reader to ignore the report.
    let curves: Vec<&str> = loaded
        .report
        .lossy
        .iter()
        .filter(|l| l.what == "curve")
        .map(|l| l.where_.as_str())
        .collect();
    assert!(curves.is_empty(), "{:?}", loaded.report.lossy);
}

/// A control point outside the segment **in time** is clamped, and reported.
///
/// `solve_bezier_x` inverts `x(t)` by bisecting 0..1 and assumes `x(t)` is
/// monotonic. A time handle outside the span makes the curve double back, which
/// is not a function of `t` and cannot be sampled at all — so unlike an
/// overshoot in value, this one genuinely cannot be kept.
#[test]
fn a_time_overshoot_is_clamped_and_said_so() {
    // The first control point sits at t = -0.3, before the segment starts.
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "animations": { "walk": { "bones": { "root": { "rotate": [
        { "value": 0, "curve": [-0.3, 5, 0.4, 25] },
        { "time": 0.5, "value": 30 }
      ] } } } }
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    let anim = loaded.animations.values().next().expect("one animation");
    let keys = match anim.timelines.first().expect("a timeline") {
        ankhimate_core::animation::Timeline::BoneRotate { keys, .. } => keys,
        other => panic!("expected a rotate timeline, got {other:?}"),
    };
    match keys[1].interp {
        ankhimate_core::animation::Interp::Bezier { out_handle, .. } => {
            assert_eq!(out_handle.x, 0.0, "time is pulled back into the segment");
        }
        other => panic!("expected a bezier, got {other:?}"),
    }

    let clamped: Vec<&str> = loaded
        .report
        .lossy
        .iter()
        .filter(|l| l.what == "curve")
        .map(|l| l.where_.as_str())
        .collect();
    assert_eq!(clamped, ["walk/root/rotate"], "{:?}", loaded.report.lossy);
}

/// A curve is normalized against the axis that actually moves.
///
/// A `Vec2Key` holds one easing for both axes, so a two-axis timeline has to
/// pick one to measure against. X is the usual choice — but a track where x
/// never moves has no span, and the handles collapse to zero. The track then
/// imports as linear on *both* axes while the source eased on y.
#[test]
fn a_two_axis_curve_follows_the_axis_that_moves() {
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "animations": { "walk": { "bones": { "root": { "translate": [
        { "x": 0, "y": 0, "curve": [0.1, 0, 0.2, 0, 0.1, 0, 0.25, 8] },
        { "time": 0.5, "x": 0, "y": 8 }
      ] } } } }
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    let anim = loaded.animations.values().next().expect("one animation");
    let keys = anim
        .timelines
        .iter()
        .find_map(|t| match t {
            ankhimate_core::animation::Timeline::BoneTranslate { keys, .. } => Some(keys),
            _ => None,
        })
        .expect("a translate timeline");
    // The easing belongs to the key it arrives at, so key 1 holds it.
    match keys[1].interp {
        ankhimate_core::animation::Interp::Bezier { in_handle, .. } => assert!(
            in_handle.y > 0.5,
            "y's own handle should survive, got {in_handle:?}"
        ),
        other => panic!("expected a bezier, got {other:?}"),
    }
}

/// A file that is not a skeleton fails rather than producing an empty rig.
#[test]
fn a_file_without_bones_is_refused() {
    assert!(spine::read(r#"{"skeleton":{}}"#, Images::None, "rig").is_err());
    assert!(spine::read("not json at all", Images::None, "rig").is_err());
}

#[test]
fn the_declared_version_is_readable_without_a_full_parse() {
    assert_eq!(
        spine::declared_version(&skeleton_4x()).as_deref(),
        Some("4.3.23")
    );
    assert_eq!(spine::declared_version("{}"), None);
}

/// An overshooting handle survives being written to an `.ankh` and read back.
///
/// The schema stores handles as a plain `[f32; 4]` and the conversions are
/// range-agnostic, so this passes today. It exists to fail if someone later
/// "tidies" the schema by clamping on the way in or out — that loss would be
/// silent, permanent, and visible only as an animation that stopped snapping.
#[test]
fn an_overshooting_handle_survives_an_ankh_round_trip() {
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "animations": { "walk": { "bones": { "root": { "rotate": [
        { "value": 0, "curve": [0.1, -20, 0.2, 45] },
        { "time": 0.5, "value": 30 }
      ] } } } }
    }"#;
    let imported = spine::read(doc, Images::None, "rig").expect("reads");
    let before = first_rotate_interp(&imported);
    match before {
        ankhimate_core::animation::Interp::Bezier { out_handle, .. } => assert!(
            out_handle.y < 0.0,
            "the fixture must overshoot or this proves nothing: {out_handle:?}"
        ),
        other => panic!("expected a bezier, got {other:?}"),
    }

    let json = ankhimate_formats::to_json(&ankhimate_formats::convert::ProjectRef {
        skeleton: &imported.skeleton,
        animations: &imported.animations,
        assets: &imported.assets,
        name: &imported.name,
        fps: imported.fps,
        export_presets: &[],
    })
    .expect("a rig with an overshoot serializes");
    let reloaded = ankhimate_formats::from_json(&json).expect("and reads back");

    assert_eq!(
        before,
        first_rotate_interp(&reloaded),
        "the handles came back changed"
    );
}

/// The easing on the second key of the first rotate track.
fn first_rotate_interp(
    loaded: &ankhimate_formats::convert::Loaded,
) -> ankhimate_core::animation::Interp {
    let anim = loaded.animations.values().next().expect("one animation");
    let keys = anim
        .timelines
        .iter()
        .find_map(|t| match t {
            ankhimate_core::animation::Timeline::BoneRotate { keys, .. } => Some(keys),
            _ => None,
        })
        .expect("a rotate timeline");
    keys[1].interp
}
