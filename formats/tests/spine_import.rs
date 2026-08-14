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

/// A handle reaching outside its span is clamped, and the clamp is reported.
///
/// Spine's control points are absolute and unconstrained: an easing that
/// overshoots puts one behind the key it starts from. Ours are fractions of the
/// span, so that curve cannot be held — and a clamp nobody mentions is how an
/// import looks faithful and plays back wrong.
#[test]
fn an_overshooting_curve_is_clamped_and_said_so() {
    // The value control point sits at -20, below the segment's start of 0.
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }],
      "animations": { "walk": { "bones": { "root": { "rotate": [
        { "value": 0, "curve": [0.1, -20, 0.2, 30] },
        { "time": 0.5, "value": 30 }
      ] } } } }
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
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
