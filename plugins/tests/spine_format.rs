//! Reading Spine JSON through the community JavaScript package.
//!
//! These pin the decisions an importer makes that a caller cannot see: which
//! field name a version uses, what happens to a curve this model cannot hold,
//! and — most importantly — that a thing which could not be carried across is
//! *reported* rather than dropped. A silent drop is the expensive kind: the rig
//! opens, looks right, and plays back wrong.

mod spine {
    use ankhimate_formats::Importer;

    pub enum Images {
        None,
    }

    pub fn read(
        json: &str,
        _images: Images,
        name: &str,
    ) -> Result<ankhimate_formats::Loaded, String> {
        let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = dir.path().join(format!("{name}.json"));
        std::fs::write(&path, json).map_err(|error| error.to_string())?;
        let source = include_str!("../../community-plugins/spine/plugin.js");
        let resources = std::collections::BTreeMap::from([(
            "spine_json.json".to_string(),
            include_bytes!("../../community-plugins/spine/spine_json.json").to_vec(),
        )]);
        let importer = ankhimate_plugins::Host::new()
            .with_resources(resources)
            .importers(source)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|importer| importer.id == "import.spine")
            .ok_or_else(|| "Spine importer was not registered".to_string())?;
        Importer::read(&importer, &path).map_err(|error| error.to_string())
    }

    pub fn declared_version(json: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(json)
            .ok()?
            .get("skeleton")?
            .get("spine")?
            .as_str()
            .map(str::to_string)
    }
}

use spine::Images;

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
                    Some(t.mix)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no constraint named {name}"))
    };

    // Every channel off: the constraint does nothing, which is what the file says.
    assert_eq!(of("aim"), ankhimate_core::constraints::TransformMix::NONE);

    // Only the axis it names is live. `mixX: -1` mirrors horizontally and
    // leaves y alone — held exactly, because each axis mixes on its own.
    let shoulder = of("shoulder");
    assert_eq!(shoulder.translate, glam::vec2(-1.0, 0.0));
    assert_eq!(shoulder.rotate, 0.0);
    assert_eq!(shoulder.scale, glam::Vec2::ZERO);

    // And nothing is reported, because nothing was lost. A model that had to
    // pick one axis for both would have to say so here.
    assert!(
        !loaded.report.lossy.iter().any(|l| l.where_ == "shoulder"),
        "a per-axis mix is held exactly now: {:?}",
        loaded.report.lossy
    );
}

/// Spine's per-axis mixes all survive, including the two it has no field for.
///
/// Spine writes `mixScaleY` but the importer only read `mixScaleX`, and Spine
/// has no `mixShearX` at all — shear's second axis has no mix there. Both are
/// channels this model has and that one does not, so both are readable and the
/// first is a real value that used to be dropped in silence.
#[test]
fn every_per_axis_mix_is_read() {
    let doc = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }, { "name": "arm", "parent": "root" }],
      "constraints": [
        { "type": "transform", "name": "full", "source": "root", "bones": ["arm"],
          "mixRotate": 0.1, "mixX": 0.2, "mixY": 0.3,
          "mixScaleX": 0.4, "mixScaleY": 0.5, "mixShearY": 0.6 }
      ]
    }"#;
    let loaded = spine::read(doc, Images::None, "rig").expect("reads");
    let mix = loaded
        .skeleton
        .constraints
        .values()
        .find_map(|c| match c {
            ankhimate_core::constraints::Constraint::Transform(t) => Some(t.mix),
            _ => None,
        })
        .expect("the constraint imported");

    assert!((mix.rotate - 0.1).abs() < 1e-6);
    assert_eq!(mix.translate, glam::vec2(0.2, 0.3));
    // `mixScaleY` was dropped before; it is a distinct axis now.
    assert_eq!(mix.scale, glam::vec2(0.4, 0.5));
    // Spine names no `mixShearX`, so it reads as "not driven".
    assert_eq!(mix.shear, glam::vec2(0.0, 0.6));
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

/// Each axis keeps its **own** curve.
///
/// This is the whole point of splitting the tracks. Spine gives translate two
/// curves — four control points for x, four for y — and a paired keyframe holds
/// one easing for both, so one axis always inherited the other's shape.
///
/// The fixture eases y and leaves x flat, which is the case that used to need a
/// fallback ("follow y where x has no span") and a report when the two
/// disagreed. Neither exists now: x reads x's curve and y reads y's.
#[test]
fn each_axis_keeps_its_own_curve() {
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

    let track = |want: ankhimate_core::animation::Axis| {
        anim.timelines
            .iter()
            .find_map(|t| match t {
                ankhimate_core::animation::Timeline::BoneTranslate { axis, keys, .. }
                    if *axis == want =>
                {
                    Some(keys.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("a {want:?} track"))
    };

    // The easing belongs to the key it arrives at, so key 1 holds it.
    // Y moves 0 -> 8 with its control point at 8, so its handle reaches the top.
    match track(ankhimate_core::animation::Axis::Y)[1].interp {
        ankhimate_core::animation::Interp::Bezier { in_handle, .. } => assert!(
            in_handle.y > 0.5,
            "y keeps its own handle, got {in_handle:?}"
        ),
        other => panic!("expected a bezier on y, got {other:?}"),
    }

    // X never moves, so its own curve is flat — and that is now *x's* answer
    // rather than something y is forced to inherit.
    match track(ankhimate_core::animation::Axis::X)[1].interp {
        ankhimate_core::animation::Interp::Bezier { in_handle, .. } => assert!(
            in_handle.y.abs() < 1e-3,
            "x has no span of its own, got {in_handle:?}"
        ),
        other => panic!("expected a bezier on x, got {other:?}"),
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
        psd_layer_paths: &Default::default(),
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
