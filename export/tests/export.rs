//! End-to-end export behaviour (T-603b, T-603c).
//!
//! The safety properties here are the ones worth the most: rendering is pure,
//! but writing is not, and a bug in this path destroys work that was never ours
//! to lose.

use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_export::context::{CONTEXT_VERSION, Context, ExportInfo};
use ankhimate_export::preset::{Cadence, Preset, Template};
use ankhimate_export::presets;
use ankhimate_export::run::{self, ExportError};
use ankhimate_formats::schema::{self, Project};
use image::{ImageFormat, Rgba, RgbaImage};

fn image_bytes(size: u32) -> Vec<u8> {
    let mut img = RgbaImage::new(size, size);
    for y in 2..(size - 2) {
        for x in 2..(size - 2) {
            img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
        }
    }
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .unwrap();
    out
}

fn assets() -> AssetDb {
    let mut db = AssetDb::new();
    db.add(ImageAsset::new("torso", image_bytes(32), 32, 32));
    db.add(ImageAsset::new("head", image_bytes(24), 24, 24));
    db
}

fn bone(name: &str, parent: &str, rotation: f32) -> schema::Bone {
    schema::Bone {
        name: name.into(),
        parent: parent.into(),
        length: 20.0,
        tx: 5.0,
        ty: 0.0,
        rotation,
        sx: 1.0,
        sy: 1.0,
        shear_x: 0.0,
        shear_y: 0.0,
        inherit_rotation: true,
        inherit_scale: true,
        inherit_reflect: true,
        color: None,
        extra: Default::default(),
    }
}

/// A rig with a hierarchy, a slot, a skin, an IK constraint and two clips —
/// enough that a template exercising every collection has something to walk.
fn fixture() -> Project {
    Project {
        version: schema::CURRENT_VERSION,
        name: "walker".into(),
        fps: 30,
        assets: Vec::new(),
        bones: vec![
            bone("root", "", 0.0),
            bone("spine", "root", 90.0),
            bone("head", "spine", -10.0),
        ],
        slots: vec![schema::Slot {
            name: "torso".into(),
            bone: "spine".into(),
            attachment: Some("torso".into()),
            color: [1.0, 1.0, 1.0, 1.0],
            dark_color: None,
            blend_mode: String::new(),
            extra: Default::default(),
        }],
        draw_order: vec!["torso".into()],
        skins: vec![schema::Skin {
            name: "default".into(),
            entries: vec![schema::SkinEntry {
                slot: "torso".into(),
                name: "torso".into(),
                attachment: schema::Attachment::Region(schema::Region {
                    texture: "torso".into(),
                    offset_x: 0.0,
                    offset_y: 0.0,
                    rotation: 0.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    width: 32.0,
                    height: 32.0,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    pivot_x: 0.5,
                    pivot_y: 0.5,
                    sequence: None,
                    extra: Default::default(),
                }),
            }],
            bones: Vec::new(),
            constraints: Vec::new(),
            extra: Default::default(),
        }],
        default_skin: "default".into(),
        constraints: vec![schema::Constraint {
            name: "arm-ik".into(),
            kind: "ik".into(),
            target: "head".into(),
            bones: vec!["spine".into()],
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: 1.1,
            stiffness: 0.0,
            mixes: None,
            offsets: None,
            local: false,
            relative: false,
            physics: None,
            forces: None,
            channels: None,
            slot: None,
            path: None,
            extra: Default::default(),
        }],
        constraint_order: vec!["arm-ik".into()],
        animations: vec![
            schema::Animation {
                name: "walk".into(),
                duration: 1.0,
                looping: true,
                timelines: vec![schema::Timeline::BoneRotate {
                    bone: "spine".into(),
                    keys: vec![
                        schema::ScalarKey {
                            time: 0.0,
                            value: 0.0,
                            interp: schema::Interp::Linear,
                        },
                        schema::ScalarKey {
                            time: 0.5,
                            value: 30.0,
                            interp: schema::Interp::Bezier {
                                handles: [0.25, 0.0, 0.75, 1.0],
                            },
                        },
                    ],
                }],
                events: vec![schema::Event {
                    time: 0.25,
                    name: "footstep".into(),
                    int_value: 0,
                    float_value: 0.0,
                    string_value: String::new(),
                    audio: String::new(),
                    volume: 1.0,
                    balance: 0.0,
                }],
                markers: vec![schema::Marker {
                    time: 0.5,
                    name: "contact".into(),
                    color: [1.0, 1.0, 1.0, 1.0],
                }],
                bone_offsets: Vec::new(),
                extra: Default::default(),
            },
            schema::Animation {
                name: "idle".into(),
                duration: 2.0,
                looping: true,
                timelines: Vec::new(),
                events: Vec::new(),
                markers: Vec::new(),
                bone_offsets: Vec::new(),
                extra: Default::default(),
            },
        ],
        groups: Vec::new(),
        export_presets: Vec::new(),
        extra: Default::default(),
    }
}

fn preset_with(templates: Vec<Template>) -> Preset {
    let mut preset = Preset::new("test");
    preset.atlas.enabled = false;
    preset.templates = templates;
    preset
}

fn simple(name: &str, path: &str, body: &str, per: Cadence) -> Template {
    Template {
        name: name.into(),
        output_path: path.into(),
        per,
        body: body.into(),
    }
}

// ── The shipped presets ──────────────────────────────────────────────────

/// T-603c's gate. If Ankhimate's own runtime format cannot be expressed as a
/// template, the engine is too weak — and we would rather find that out here
/// than have a user find it after us.
#[test]
fn the_native_runtime_format_renders_as_a_template() {
    let preset = presets::default_preset();
    let plan = run::plan(&fixture(), &assets(), &preset).expect("the runtime preset renders");

    let skeleton = plan
        .files
        .iter()
        .find(|f| f.path == "skeleton.json")
        .expect("the preset writes skeleton.json");

    let parsed: serde_json::Value = serde_json::from_str(&skeleton.contents).unwrap_or_else(|e| {
        panic!(
            "the runtime template emitted invalid JSON: {e}\n{}",
            skeleton.contents
        )
    });

    assert_eq!(parsed["name"], "walker");
    assert_eq!(parsed["fps"], 30);
    assert_eq!(parsed["bones"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["slots"].as_array().unwrap().len(), 1);
    assert!(parsed["animations"]["walk"].is_object());
    assert!(parsed["animations"]["idle"].is_object());
}

/// The Phaser preset is the one shipped format aimed at a third-party engine,
/// so its shape is pinned against what that engine's loader documents it reads.
#[test]
fn the_phaser_preset_matches_the_documented_atlas_shape() {
    let preset = presets::builtin()
        .into_iter()
        .find(|p| p.name.contains("Phaser"))
        .expect("the Phaser preset ships");

    let plan = run::plan(&fixture(), &assets(), &preset).expect("it renders");
    let atlas = plan
        .files
        .iter()
        .find(|f| f.path == "atlas.json")
        .expect("it writes atlas.json");
    let parsed: serde_json::Value = serde_json::from_str(&atlas.contents)
        .unwrap_or_else(|e| panic!("invalid JSON: {e}\n{}", atlas.contents));

    // JSON Hash form: `frames` is an object keyed by frame name.
    let frames = parsed["frames"].as_object().expect("frames is an object");
    let torso = &frames["torso"];
    for key in [
        "frame",
        "rotated",
        "trimmed",
        "spriteSourceSize",
        "sourceSize",
    ] {
        assert!(!torso[key].is_null(), "a frame is missing '{key}'");
    }
    for key in ["x", "y", "w", "h"] {
        assert!(
            torso["frame"][key].is_number(),
            "frame.{key} must be a number"
        );
    }
    // The trimmed sprite's original size has to survive, or a runtime cannot put
    // a trimmed region back where the untrimmed one sat.
    assert_eq!(torso["sourceSize"]["w"], 32);
    assert_eq!(torso["sourceSize"]["h"], 32);

    let meta = &parsed["meta"];
    assert_eq!(meta["image"], "atlas.png");
    assert!(
        meta["size"]["w"].is_number(),
        "meta.size.w must be a number"
    );
}

#[test]
fn every_shipped_preset_parses_and_renders() {
    let project = fixture();
    let assets = assets();
    for preset in presets::builtin() {
        let plan = run::plan(&project, &assets, &preset)
            .unwrap_or_else(|e| panic!("preset '{}' failed: {e}", preset.name));
        assert!(
            !plan.files.is_empty(),
            "preset '{}' wrote no files",
            preset.name
        );
        for file in &plan.files {
            if file.path.ends_with(".json") {
                serde_json::from_str::<serde_json::Value>(&file.contents).unwrap_or_else(|e| {
                    panic!(
                        "preset '{}' wrote invalid JSON at {}: {e}",
                        preset.name, file.path
                    )
                });
            }
        }
    }
}

/// The bone array is flat with integer parent references, so a runtime applying
/// transforms in order needs parents to precede children.
#[test]
fn exported_bones_are_ordered_parents_first() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let bones = ctx.skeleton["bones"].as_array().unwrap();
    for (i, bone) in bones.iter().enumerate() {
        let parent = bone["parent_index"].as_i64().unwrap();
        assert!(
            parent < i as i64,
            "bone '{}' at {i} has parent index {parent}",
            bone["name"]
        );
    }
}

#[test]
fn a_root_bone_reports_minus_one_rather_than_null() {
    // A template writing a flat array needs a number in the field either way;
    // "null" in a numeric slot is invalid in most target formats.
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let bones = ctx.skeleton["bones"].as_array().unwrap();
    assert_eq!(bones[0]["parent_index"], -1);
}

/// Markers are editor furniture (`schema::Marker` says so). Exporting them would
/// invite presets to write an animator's notes into a runtime file.
#[test]
fn editor_only_markers_never_reach_the_context() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let json = serde_json::to_string(&ctx.animations).unwrap();
    assert!(
        !json.contains("contact"),
        "the marker named 'contact' leaked into the context"
    );
}

/// `{{#each}}` over an absent key is a strict-mode error, so a collection must
/// always be present — empty, never missing — or a template that walks it breaks
/// on the one rig that happens to lack that feature.
#[test]
fn collections_are_present_even_when_empty() {
    let mut bare = fixture();
    bare.animations[0].timelines.clear();
    bare.animations[0].events.clear();
    bare.constraints.clear();
    bare.skins.clear();

    let ctx = Context::build(&bare, None, ExportInfo::default());
    for key in ["bones", "slots", "skins", "constraints"] {
        assert!(
            ctx.skeleton[key].is_array(),
            "skeleton.{key} must be an array"
        );
    }
    let anim = &ctx.animations[0];
    for key in [
        "bones",
        "slots",
        "deform",
        "draw_order",
        "ik",
        "transform",
        "events",
    ] {
        assert!(anim[key].is_array(), "animation.{key} must be an array");
    }
}

#[test]
fn the_context_version_is_exposed_to_templates() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    assert_eq!(ctx.context_version, CONTEXT_VERSION);
}

/// Track offsets (T-905) are runtime data with no equivalent in any target
/// format, so they are folded into key times here rather than left for a
/// template to apply — which every preset would get wrong.
#[test]
fn per_bone_track_offsets_are_baked_into_key_times() {
    let mut project = fixture();
    project.animations[0].bone_offsets = vec![schema::BoneOffset {
        bone: "spine".into(),
        offset: 0.25,
    }];

    let ctx = Context::build(&project, None, ExportInfo::default());
    let spine = ctx.animations[0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "spine")
        .expect("spine has a timeline");
    let times: Vec<f64> = spine["rotate"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["time"].as_f64().unwrap())
        .collect();
    assert_eq!(times, vec![0.25, 0.75], "keys shift by the bone's offset");
}

// ── Cadence ─────────────────────────────────────────────────────────────

#[test]
fn a_per_animation_template_writes_one_file_per_animation() {
    let preset = preset_with(vec![simple(
        "anim",
        "anim/{{animation.name}}.json",
        r#"{"name":"{{animation.name}}","duration":{{animation.duration}}}"#,
        Cadence::Animation,
    )]);
    let plan = run::plan(&fixture(), &assets(), &preset).unwrap();

    let mut paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, vec!["anim/idle.json", "anim/walk.json"]);
}

#[test]
fn a_rig_with_no_animations_still_exports() {
    let mut project = fixture();
    project.animations.clear();
    let preset = preset_with(vec![
        simple(
            "skel",
            "skeleton.json",
            r#"{"name":"{{project.name}}"}"#,
            Cadence::Once,
        ),
        simple(
            "anim",
            "anim/{{animation.name}}.json",
            "{}",
            Cadence::Animation,
        ),
    ]);
    let plan = run::plan(&project, &assets(), &preset).unwrap();
    assert_eq!(plan.files.len(), 1, "only the once-template runs");
}

// ── Writing: the dangerous part ─────────────────────────────────────────

/// Rule 2 of `run`: all or nothing. A template failing on the ninth animation
/// must not leave eight files from this run beside four from the last.
#[test]
fn a_template_that_fails_writes_nothing_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let preset = preset_with(vec![
        simple("good", "good.json", r#"{"ok":true}"#, Cadence::Once),
        simple("bad", "bad.json", "{{missing_field}}", Cadence::Once),
    ]);

    let err = run::export(&fixture(), &assets(), &preset, dir.path()).unwrap_err();
    assert!(matches!(err, ExportError::Template(_)));

    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "a failed export left files behind"
    );
}

#[test]
fn an_escaping_path_aborts_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let preset = preset_with(vec![
        simple("good", "good.json", "{}", Cadence::Once),
        simple("evil", "../escaped.json", "{}", Cadence::Once),
    ]);

    let err = run::export(&fixture(), &assets(), &preset, dir.path()).unwrap_err();
    assert!(matches!(err, ExportError::Template(_)), "got {err:?}");
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    assert!(!dir.path().parent().unwrap().join("escaped.json").exists());
}

/// Two templates writing one path means the second silently overwrites the
/// first, and the user gets a file set quietly missing a member.
#[test]
fn two_templates_claiming_one_path_is_an_error() {
    let preset = preset_with(vec![
        simple("first", "out.json", "{}", Cadence::Once),
        simple("second", "out.json", "{}", Cadence::Once),
    ]);
    match run::plan(&fixture(), &assets(), &preset) {
        Err(ExportError::DuplicatePath { first, second, .. }) => {
            assert_eq!(first, "first");
            assert_eq!(second, "second");
        }
        other => panic!("expected DuplicatePath, got {other:?}"),
    }
}

#[test]
fn a_successful_export_writes_every_planned_file() {
    let dir = tempfile::tempdir().unwrap();
    let preset = preset_with(vec![
        simple(
            "skel",
            "skeleton.json",
            r#"{"n":"{{project.name}}"}"#,
            Cadence::Once,
        ),
        simple(
            "anim",
            "anim/{{animation.name}}.json",
            "{}",
            Cadence::Animation,
        ),
    ]);

    let plan = run::export(&fixture(), &assets(), &preset, dir.path()).unwrap();
    for path in plan.paths() {
        assert!(dir.path().join(path).exists(), "{path} was not written");
    }
    let text = std::fs::read_to_string(dir.path().join("skeleton.json")).unwrap();
    assert_eq!(text, r#"{"n":"walker"}"#);
}

/// Rule 3: never delete. A renamed animation leaves its old file behind, and
/// only the user knows whether that matters — `output_dir` can be a source tree.
#[test]
fn an_export_reports_orphans_rather_than_removing_them() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("stale.json"), "old").unwrap();

    let preset = preset_with(vec![simple("skel", "skeleton.json", "{}", Cadence::Once)]);
    let plan = run::plan(&fixture(), &assets(), &preset).unwrap();
    let survey = plan.survey(dir.path());

    assert_eq!(survey.orphans, vec!["stale.json"]);
    assert_eq!(survey.created, vec!["skeleton.json"]);
    assert!(survey.replaced.is_empty());

    run::write(&plan, dir.path()).unwrap();
    assert!(
        dir.path().join("stale.json").exists(),
        "the export deleted a file it did not write"
    );
}

#[test]
fn a_survey_separates_created_from_replaced() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("skeleton.json"), "previous").unwrap();

    let preset = preset_with(vec![
        simple("skel", "skeleton.json", "{}", Cadence::Once),
        simple("other", "other.json", "{}", Cadence::Once),
    ]);
    let survey = run::plan(&fixture(), &assets(), &preset)
        .unwrap()
        .survey(dir.path());

    assert_eq!(survey.replaced, vec!["skeleton.json"]);
    assert_eq!(survey.created, vec!["other.json"]);
}

// ── Atlas integration ───────────────────────────────────────────────────

#[test]
fn an_atlas_preset_writes_pages_and_exposes_regions() {
    let dir = tempfile::tempdir().unwrap();
    let mut preset = preset_with(vec![simple(
        "regions",
        "regions.json",
        "[{{#each atlas.regions}}\"{{name}}\"{{#unless @last}},{{/unless}}{{/each}}]",
        Cadence::Once,
    )]);
    preset.atlas.enabled = true;

    let plan = run::export(&fixture(), &assets(), &preset, dir.path()).unwrap();
    assert!(
        dir.path().join("atlas.png").exists(),
        "the atlas page was not written"
    );

    let text = std::fs::read_to_string(dir.path().join("regions.json")).unwrap();
    let names: Vec<String> = serde_json::from_str(&text).unwrap();
    assert_eq!(
        names,
        vec!["head", "torso"],
        "regions come through in name order"
    );
    let _ = plan;
}

#[test]
fn a_preset_without_an_atlas_omits_it_from_the_context() {
    let preset = preset_with(vec![simple(
        "t",
        "out.json",
        "{{#if atlas}}has{{else}}none{{/if}}",
        Cadence::Once,
    )]);
    let plan = run::plan(&fixture(), &assets(), &preset).unwrap();
    assert_eq!(plan.files[0].contents, "none");
}

// ── Determinism ─────────────────────────────────────────────────────────

/// An export that differs run-to-run makes "did the rig actually change?"
/// unanswerable in version control.
#[test]
fn exporting_the_same_project_twice_is_byte_identical() {
    let project = fixture();
    let assets = assets();
    let preset = presets::default_preset();

    let first = run::plan(&project, &assets, &preset).unwrap();
    let second = run::plan(&project, &assets, &preset).unwrap();

    assert_eq!(first.files, second.files);
    assert_eq!(first.binaries.len(), second.binaries.len());
    for (a, b) in first.binaries.iter().zip(second.binaries.iter()) {
        assert_eq!(a.0, b.0);
        assert_eq!(a.1, b.1, "page {} differs between exports", a.0);
    }
}

#[test]
fn no_timestamp_leaks_into_the_output() {
    // A timestamp would make every export differ from the last, destroying
    // diffability. The context deliberately carries none.
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let json = serde_json::to_string(&ctx.export).unwrap();
    for word in ["timestamp", "date", "time\"", "generated"] {
        assert!(!json.contains(word), "'{word}' appears in the export block");
    }
}

// ── Presets as data ─────────────────────────────────────────────────────

#[test]
fn a_preset_survives_a_json_round_trip() {
    let preset = presets::default_preset();
    let text = preset.to_json();
    let back = Preset::from_json(&text).expect("a written preset parses");
    assert_eq!(preset, back);
}
