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

fn spine_preset() -> Preset {
    Preset::from_json(include_str!("fixtures/spine_json.json"))
        .expect("the community Spine preset parses")
}

#[test]
fn spine_is_not_a_first_party_export_preset() {
    assert!(
        presets::builtin()
            .iter()
            .all(|preset| !preset.name.starts_with("Spine JSON"))
    );
}

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
        // The index a real project always carries. Attachment sizes are compared
        // against it to find the rig's art scale, so an empty list is not a
        // realistic fixture.
        assets: vec![
            schema::Asset {
                name: "torso".into(),
                file: "torso.png".into(),
                width: 32,
                height: 32,
                source_path: None,
                extra: Default::default(),
            },
            schema::Asset {
                name: "head".into(),
                file: "head.png".into(),
                width: 24,
                height: 24,
                source_path: None,
                extra: Default::default(),
            },
        ],
        bones: vec![
            bone("root", "", 0.0),
            bone("spine", "root", 90.0),
            bone("head", "spine", -10.0),
        ],
        slots: vec![schema::Slot {
            name: "torso".into(),
            bone: "spine".into(),
            attachment: Some("shirt".into()),
            color: [1.0, 1.0, 1.0, 1.0],
            dark_color: None,
            blend_mode: String::new(),
            extra: Default::default(),
        }],
        draw_order: vec!["torso".into()],
        skins: vec![schema::Skin {
            name: "default".into(),
            entries: vec![
                schema::SkinEntry {
                    slot: "torso".into(),
                    name: "shirt".into(),
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
                },
                // A weighted mesh with an explicit edge list, so the packed-vertex
                // and edge encodings are exercised rather than assumed.
                schema::SkinEntry {
                    slot: "torso".into(),
                    name: "cape".into(),
                    attachment: schema::Attachment::Mesh(schema::Mesh {
                        texture: "head".into(),
                        vertices: vec![0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0],
                        uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
                        triangles: vec![0, 1, 2, 0, 2, 3],
                        edges: vec![0, 1, 1, 2, 2, 3, 3, 0],
                        weights: vec![
                            vec![("spine".into(), 1.0)],
                            vec![("spine".into(), 0.5), ("head".into(), 0.5)],
                            vec![("head".into(), 1.0)],
                            vec![("spine".into(), 1.0)],
                        ],
                        linked: None,
                        sequence: None,
                        extra: Default::default(),
                    }),
                },
            ],
            bones: Vec::new(),
            constraints: Vec::new(),
            extra: Default::default(),
        }],
        default_skin: "default".into(),
        constraints: vec![
            schema::Constraint {
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
                transform_mix: None,
                offsets: None,
                local: false,
                relative: false,
                physics: None,
                forces: None,
                channels: None,
                slot: None,
                path: None,
                extra: Default::default(),
            },
            schema::Constraint {
                name: "head-follow".into(),
                kind: "transform".into(),
                target: "spine".into(),
                bones: vec!["head".into()],
                bend_direction: 1.0,
                mix: 1.0,
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
                stiffness: 0.0,
                // Translate mixed on x alone — the shape a mirror uses, and the
                // one a single mix per channel could not hold.
                transform_mix: Some(schema::TransformMix {
                    rotate: 1.0,
                    translate_x: 0.5,
                    ..Default::default()
                }),
                offsets: Some([2.0, 3.0, 15.0, 0.0, 0.0, 0.0, 0.0]),
                local: false,
                relative: false,
                physics: None,
                forces: None,
                channels: None,
                slot: None,
                path: None,
                extra: Default::default(),
            },
        ],
        constraint_order: vec!["arm-ik".into(), "head-follow".into()],
        animations: vec![
            schema::Animation {
                name: "walk".into(),
                duration: 1.0,
                looping: true,
                timelines: vec![
                    schema::Timeline::BoneRotate {
                        bone: "spine".into(),
                        keys: vec![
                            schema::ScalarKey {
                                time: 0.0,
                                value: 0.0,
                                interp: schema::Interp::Linear,
                            },
                            // A bezier in the *middle* of the list: the last key
                            // has no following key, so a curve encoding is only
                            // exercised when one sits before another.
                            schema::ScalarKey {
                                time: 0.5,
                                value: 30.0,
                                interp: schema::Interp::Bezier {
                                    handles: [0.25, 0.0, 0.75, 1.0],
                                },
                            },
                            schema::ScalarKey {
                                time: 1.0,
                                value: 10.0,
                                interp: schema::Interp::Stepped,
                            },
                        ],
                    },
                    // The axes are independent tracks: x keys at 0 and 1, y
                    // keys at 0 and 0.5 with its own easing. A paired
                    // representation cannot hold this, which is the point.
                    schema::Timeline::BoneTranslate {
                        bone: "head".into(),
                        axis: schema::Axis::X,
                        keys: vec![
                            schema::ScalarKey {
                                time: 0.0,
                                value: 0.0,
                                interp: schema::Interp::Linear,
                            },
                            // The bezier sits on the key that is *arrived at*,
                            // which is how the schema stores easing. It
                            // describes the segment from t=0, so an exporter
                            // whose format hangs curves on the starting key
                            // emits it there.
                            schema::ScalarKey {
                                time: 1.0,
                                value: 4.0,
                                interp: schema::Interp::Bezier {
                                    handles: [0.25, 0.1, 0.75, 0.9],
                                },
                            },
                        ],
                        extra: Default::default(),
                    },
                    schema::Timeline::BoneTranslate {
                        bone: "head".into(),
                        axis: schema::Axis::Y,
                        keys: vec![
                            schema::ScalarKey {
                                time: 0.0,
                                value: 0.0,
                                interp: schema::Interp::Linear,
                            },
                            schema::ScalarKey {
                                time: 0.5,
                                value: -6.0,
                                interp: schema::Interp::Linear,
                            },
                        ],
                        extra: Default::default(),
                    },
                    schema::Timeline::Deform {
                        slot: "torso".into(),
                        attachment: "cape".into(),
                        keys: vec![schema::DeformKey {
                            time: 0.25,
                            offsets: vec![0.0, 0.0, 1.5, -0.5, 0.0, 0.0, 0.0, 0.0],
                            interp: schema::Interp::Linear,
                        }],
                    },
                    // Two channels of one constraint, so the merge by constraint
                    // has something to merge.
                    schema::Timeline::IkMix {
                        constraint: "arm-ik".into(),
                        keys: vec![
                            schema::ScalarKey {
                                time: 0.0,
                                value: 1.0,
                                interp: schema::Interp::Linear,
                            },
                            schema::ScalarKey {
                                time: 0.5,
                                value: 0.0,
                                interp: schema::Interp::Linear,
                            },
                        ],
                    },
                    // Softness is keyed at the same times as mix and carries a
                    // bezier of its own. Both facts matter: a curve belongs to
                    // the channel it was authored on, and a format writing one
                    // curve per key needs both channels' points, not whichever
                    // was merged last.
                    schema::Timeline::IkSoftness {
                        constraint: "arm-ik".into(),
                        keys: vec![
                            schema::ScalarKey {
                                time: 0.0,
                                value: 4.0,
                                interp: schema::Interp::Linear,
                            },
                            // Easing is stored on the key it leads *into*, so
                            // this bezier describes the segment from t=0.
                            schema::ScalarKey {
                                time: 0.5,
                                value: 12.0,
                                interp: schema::Interp::Bezier {
                                    handles: [0.3, 0.1, 0.7, 0.9],
                                },
                            },
                        ],
                    },
                    // Keyed at one time only, so the merged list has a key where
                    // this channel has nothing — the case that proves an absent
                    // channel stays absent rather than being defaulted.
                    schema::Timeline::IkBendDirection {
                        constraint: "arm-ik".into(),
                        keys: vec![schema::ScalarKey {
                            time: 0.5,
                            value: -1.0,
                            interp: schema::Interp::Stepped,
                        }],
                    },
                    schema::Timeline::TransformConstraintMix {
                        constraint: "head-follow".into(),
                        keys: vec![schema::MixKey {
                            time: 0.0,
                            value: schema::TransformMix {
                                rotate: 1.0,
                                translate_x: 0.5,
                                ..Default::default()
                            },
                            interp: schema::Interp::Linear,
                            extra: Default::default(),
                        }],
                    },
                ],
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
        psd_layer_paths: Default::default(),
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

/// The Spine preset's output is 4.3-shaped, not merely valid JSON.
///
/// `every_shipped_preset_parses_and_renders` only proves the bytes parse, and a
/// preset emitting the 3.8 layout, or normalized curve handles, passes it
/// happily. This one reads the fields a 4.3 consumer actually branches on.
#[test]
fn the_spine_preset_emits_the_43_layout() {
    let preset = spine_preset();
    let plan = run::plan(&fixture(), &assets(), &preset).expect("the Spine preset renders");
    let file = plan
        .files
        .iter()
        .find(|f| f.path.ends_with(".json"))
        .expect("a skeleton file is written");
    let out: serde_json::Value = serde_json::from_str(&file.contents).expect("valid JSON");

    assert!(
        out["skeleton"]["spine"]
            .as_str()
            .is_some_and(|v| v.starts_with("4.")),
        "declares a 4.x version: {}",
        out["skeleton"]["spine"]
    );

    // 4.3 puts every constraint in one array tagged by type; 3.8 used separate
    // `ik` / `transform` blocks at the top level.
    assert!(out.get("ik").is_none(), "3.8's top-level ik block is gone");
    let kinds: Vec<&str> = out["constraints"]
        .as_array()
        .expect("one tagged constraints array")
        .iter()
        .filter_map(|c| c["type"].as_str())
        .collect();
    assert!(
        kinds.contains(&"ik") && kinds.contains(&"transform"),
        "{kinds:?}"
    );

    let rotate = &out["animations"]["walk"]["bones"]["spine"]["rotate"];
    // 4.x names a rotate key's field `value`; 3.8 called it `angle`.
    assert!(rotate[0]["value"].is_number(), "rotate keys use `value`");
    assert!(rotate[0]["angle"].is_null(), "and not 3.8's `angle`");

    // The curve is absolute time/value, and sits on the key the segment starts
    // from — the schema stores easing on the key it arrives at, so the bezier
    // authored at t=0.5 describes 0.0 -> 0.5 and is emitted on key 0.
    // Normalized handles would all sit in 0..1; the second control point's value
    // here is 30 (the segment's end value), so this fails loudly if the
    // normalized form ever comes back.
    let curve = rotate[0]["curve"].as_array().expect("a bezier key");
    assert_eq!(curve.len(), 4);
    assert!(
        (curve[3].as_f64().unwrap() - 30.0).abs() < 1e-3,
        "control points are absolute, not normalized: {curve:?}"
    );

    // Slot colour timelines are `rgba` in 4.x, and IK channels arrive merged.
    let ik = &out["animations"]["walk"]["ik"]["arm-ik"];
    assert_eq!(ik[1]["softness"], 12.0, "merged IK channels: {ik}");

    // Deform lives under `attachments`, keyed by skin.
    assert!(
        out["animations"]["walk"]["attachments"]["default"]["torso"]["cape"]["deform"].is_array(),
        "deform is written under attachments/<skin>"
    );

    // Both of the torso slot's attachments survive. Rendered from the flat entry
    // list, the slot key was emitted twice — valid JSON that a parser collapses
    // to whichever came last, silently losing an attachment. serde_json does the
    // same collapsing, so this is checked on the raw text as well.
    let torso = &out["skins"][0]["attachments"]["torso"];
    assert!(
        torso["shirt"].is_object() && torso["cape"].is_object(),
        "{torso}"
    );
    let skins_text = {
        let start = file.contents.find("\"skins\"").expect("a skins block");
        let end = file.contents[start..]
            .find("\"animations\"")
            .expect("animations follow the skins");
        &file.contents[start..start + end]
    };
    assert_eq!(
        skins_text.matches("\"torso\": {").count(),
        1,
        "a slot with two attachments emits its key once, not twice"
    );

    // No curve on the last key of any channel. A curve describes the
    // interpolation *towards the next key*, so one written on the final key
    // sends the reader looking for a frame that does not exist — Spine answers
    // with `[error] Invalid curve` and then a null-frame NPE. The fixture's
    // spine rotate track deliberately ends on a stepped key, which is precisely
    // the case that shipped broken.
    let rotate = out["animations"]["walk"]["bones"]["spine"]["rotate"]
        .as_array()
        .unwrap();
    assert!(
        rotate.last().unwrap()["curve"].is_null(),
        "the last key carries no curve: {:?}",
        rotate.last().unwrap()
    );
    assert!(
        rotate[0]["curve"].is_array(),
        "but a key whose outgoing segment eases still does"
    );
    // And a key whose *next* key is stepped says so, rather than carrying its
    // own easing: the curve describes the segment leaving the key.
    assert_eq!(rotate[1]["curve"], "stepped");

    let translate = out["animations"]["walk"]["bones"]["head"]["translate"]
        .as_array()
        .unwrap();
    assert!(translate.last().unwrap()["curve"].is_null());

    let ik = out["animations"]["walk"]["ik"]["arm-ik"]
        .as_array()
        .unwrap();
    assert!(ik.last().unwrap()["curve"].is_null());
}

/// The Spine preset ships the artwork the Spine editor can actually open.
///
/// Spine's editor builds atlases; it does not consume one. Importing a skeleton
/// that names regions in a packed page leaves it with correct bones and no
/// artwork, and — because nothing failed — no error to explain why. So this
/// preset writes **loose images** at the path the skeleton declares.
#[test]
fn the_spine_preset_writes_loose_images_where_the_skeleton_looks() {
    let preset = spine_preset();
    assert!(
        preset.copy_images && !preset.atlas.enabled,
        "the editor wants loose files, not a packed page"
    );

    let plan = run::plan(&fixture(), &assets(), &preset).expect("renders");
    let skeleton = plan
        .files
        .iter()
        .find(|f| f.path.ends_with(".json"))
        .expect("a skeleton is written");
    let out: serde_json::Value = serde_json::from_str(&skeleton.contents).expect("valid JSON");

    // Every image lands under the folder the skeleton points at, or the editor
    // silently finds nothing.
    let images = out["skeleton"]["images"].as_str().expect("an images path");
    let dir = images.trim_start_matches("./").trim_end_matches('/');
    assert!(!plan.binaries.is_empty(), "artwork is written at all");
    for (path, _) in &plan.binaries {
        assert!(
            path.starts_with(dir),
            "'{path}' is not under the declared images path '{images}'"
        );
    }

    // One file per asset, named as the attachments reference it.
    for name in ["torso", "head"] {
        assert!(
            plan.binaries
                .iter()
                .any(|(p, _)| p == &format!("{dir}/{name}.png")),
            "no image for '{name}': {:?}",
            plan.paths()
        );
    }
}

/// A region exports the size the **rig** authored, not the image file's.
///
/// A region's `width`/`height` are already a draw size in rig space, and its
/// `scale_x`/`scale_y` are the artist's own scaling. Spine wants exactly those
/// two, passed through.
///
/// This once derived a scale from the ratio of rig size to file size and
/// declared the *file's* dimensions instead. On a rig authored against
/// half-resolution art that produced a fractional 1.9778 rather than 2.0, so
/// every region landed slightly off — and it overwrote genuine authored scales
/// with the derived one. The rig is the truth here; the file size is not
/// evidence about it.
#[test]
fn a_region_exports_the_size_the_rig_authored() {
    let mut project = fixture();
    // The torso asset is 32x32; the rig draws it at 63x63 with its own scale.
    // An odd number matters: a ratio-derived scale would report 1.96875 here.
    project.assets.push(schema::Asset {
        name: "torso".into(),
        file: "torso.png".into(),
        width: 32,
        height: 32,
        source_path: None,
        extra: Default::default(),
    });
    for entry in &mut project.skins[0].entries {
        if let schema::Attachment::Region(r) = &mut entry.attachment {
            r.width = 63.0;
            r.height = 63.0;
            r.scale_x = 0.5;
            r.scale_y = 1.25;
        }
    }

    let rendered = render_spine(&project);
    let region = &rendered["skins"][0]["attachments"]["torso"]["shirt"];
    assert_eq!(region["width"], 63.0, "the rig's size, not the file's 32");
    assert_eq!(
        region["scaleX"], 0.5,
        "the artist's scale, not a derived one"
    );
    assert_eq!(region["scaleY"], 1.25);

    // A scale of 1.0 is the common case and is left out entirely rather than
    // written as a redundant field.
    let rendered = render_spine(&fixture());
    let region = &rendered["skins"][0]["attachments"]["torso"]["shirt"];
    assert!(region.get("scaleX").is_none(), "got {region}");
}

/// A transform constraint names only the channels it actually drives.
///
/// A mix of 0 contributes nothing, so a channel left at 0 is one the rig does
/// not have. Spine's `properties` block says what a constraint is *allowed* to
/// affect, and naming a channel there switches it on: a constraint that suddenly
/// copies its target's scale and shear stretches every bone it governs while
/// leaving every attachment provably correct — which is what makes it such a
/// convincing red herring.
///
/// This exporter declared all six properties on every transform constraint and
/// did exactly that to a real rig.
#[test]
fn a_transform_constraint_drives_only_its_live_channels() {
    // The fixture mixes rotate 1.0 and translate 0.5, leaving scale and shear
    // at 0.
    let rendered = render_spine(&fixture());
    let constraint = rendered["constraints"]
        .as_array()
        .expect("constraints ship as an array")
        .iter()
        .find(|c| c["name"] == "head-follow")
        .expect("the fixture's transform constraint is exported")
        .clone();

    let properties = constraint["properties"]
        .as_object()
        .expect("a live constraint declares its properties");
    let mut named: Vec<&str> = properties.keys().map(String::as_str).collect();
    named.sort_unstable();
    assert_eq!(
        named,
        ["rotate", "x", "y"],
        "only the driven channels, and never scaleX/scaleY/shearY"
    );

    assert_eq!(constraint["mixRotate"], 1.0);
    assert_eq!(constraint["mixX"], 0.5);
    assert!(
        constraint.get("mixScaleX").is_none(),
        "a dead channel carries no mix either, got {constraint}"
    );

    // A constraint with every mix at 0 does nothing, and says so by naming
    // nothing — rather than naming every channel and driving them all to 0.
    let mut project = fixture();
    for c in &mut project.constraints {
        if c.kind == "transform" {
            c.transform_mix = Some(schema::TransformMix::default());
        }
    }
    let rendered = render_spine(&project);
    let constraint = rendered["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "head-follow")
        .unwrap()
        .clone();
    assert!(
        constraint.get("properties").is_none(),
        "an inert constraint claims no channels, got {constraint}"
    );
}

/// A weighted vertex is written in each influence's **own** bone space.
///
/// A rig stores one position per vertex, in the space of the bone its slot hangs
/// from, because `core` skins by transforming that single position through each
/// bound bone. A runtime storing weights per influence wants the opposite: the
/// vertex already expressed in each bone's frame, so skinning is a weighted sum.
///
/// This exporter wrote the shared position for every influence. A vertex bound
/// to one bone survives that; one bound to two bones far apart does not, and
/// every weighted mesh on a real rig came out scattered. Nothing caught it,
/// because the two positions are only *equal* when the bones coincide — which is
/// exactly the case a small fixture has.
#[test]
fn a_weighted_vertex_is_written_in_each_bones_own_space() {
    // The fixture's `cape` binds vertex 1 to `spine` and `head` at 0.5 each.
    // Those bones are 20 units apart and rotated, so a correct export gives the
    // two influences *different* coordinates.
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let att = mesh_attachment(&ctx);
    let packed = att["weights"][1]["bones"]
        .as_array()
        .expect("the second vertex has two influences")
        .clone();
    assert_eq!(packed.len(), 2, "got {packed:?}");

    let first = (
        packed[0]["x"].as_f64().unwrap(),
        packed[0]["y"].as_f64().unwrap(),
    );
    let second = (
        packed[1]["x"].as_f64().unwrap(),
        packed[1]["y"].as_f64().unwrap(),
    );
    assert!(
        (first.0 - second.0).abs() > 1.0 || (first.1 - second.1).abs() > 1.0,
        "two distinct bones must not share one offset: {first:?} and {second:?}"
    );

    // The flat encoding — what the Spine preset actually writes — has to agree
    // with the packed one, or a template picks whichever is wrong.
    let flat = att["flat_vertices"]
        .as_array()
        .expect("flat_vertices ships")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>();
    // vertex 0 is `[1, bone, x, y, weight]`, so vertex 1 starts at index 5.
    assert_eq!(flat[5], 2.0, "the second vertex has two influences");
    let flat_first = (flat[7], flat[8]);
    let flat_second = (flat[11], flat[12]);
    assert!(
        (flat_first.0 - flat_second.0).abs() > 1.0 || (flat_first.1 - flat_second.1).abs() > 1.0,
        "the flat encoding repeats one offset: {flat_first:?} and {flat_second:?}"
    );
}

/// Weighted vertices are measured against the **constrained** setup pose.
///
/// `evaluate()` applies constraints before composing world transforms (PLAN
/// §2.6), so an IK-driven bone rests somewhere its local transform alone does
/// not predict. A vertex bound to such a bone has to be expressed against where
/// the bone actually *is*.
///
/// This composed the FK chain by hand instead. Every bone whose placement no
/// constraint touched agreed, so the fixture and most of a real rig looked
/// correct — while the tip of every IK chain sat a few degrees out, and with it
/// every vertex weighted to one. On spineboy the feet were 48 and 56 units
/// adrift while all ten other meshes matched exactly.
#[test]
fn a_weighted_vertex_follows_its_bone_through_constraints() {
    // Checked against `core` itself rather than a hand-built expectation: the
    // bug was precisely that the exporter derived worlds its own way, and any
    // expectation written here by hand would have to re-derive them again.
    //
    // A rotated bone is the discriminator. `head` rests at 80° from the mesh's
    // host bone, so a vertex's offset in `head`'s frame is a rotation away from
    // its offset in the host's — an exporter that skipped the transform, or
    // applied the FK chain without constraints, lands somewhere else.
    let project = fixture();
    let loaded = ankhimate_formats::convert::from_schema(&project);
    let mut pose = ankhimate_core::pose::Pose::default();
    ankhimate_core::pose::evaluate(&loaded.skeleton, &[], &mut pose);

    let world_of = |name: &str| {
        loaded
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == name)
            .and_then(|(id, _)| pose.worlds.get(id).copied())
            .unwrap_or_else(|| panic!("core places '{name}'"))
    };

    // The cape hangs from the `torso` slot, which is on `spine`.
    let host = world_of("spine");
    let head = world_of("head");
    // Vertex 2 of the fixture's cape, in mesh space.
    let (mx, my) = (10.0_f32, 10.0_f32);
    let world = host.transform_point(glam::Vec2::new(mx, my));
    let det = head.a * head.d - head.b * head.c;
    let (dx, dy) = (world.x - head.tx, world.y - head.ty);
    let expected = (
        (head.d * dx - head.c * dy) / det,
        (head.a * dy - head.b * dx) / det,
    );

    let ctx = Context::build(&project, None, ExportInfo::default());
    let att = mesh_attachment(&ctx);
    let bones = att["weights"][2]["bones"].as_array().unwrap().clone();
    let got = (
        bones[0]["x"].as_f64().unwrap() as f32,
        bones[0]["y"].as_f64().unwrap() as f32,
    );

    // Against absolute numbers from `core`, not against a second derivation
    // sharing the same inputs: an error common to both sides cancels, and a
    // check written that way passes on a pose that has been shifted wholesale.
    assert!(
        (got.0 - expected.0).abs() < 0.01 && (got.1 - expected.1).abs() < 0.01,
        "the export must agree with core's pose: got {got:?}, core says {expected:?}"
    );
    // `head` rests at 80° from `spine` with a 5-unit offset, so the two frames
    // genuinely disagree — without this the check above would pass on an
    // exporter that applied no transform at all.
    assert!(
        (expected.0 - mx).abs() > 1.0 || (expected.1 - my).abs() > 1.0,
        "this fixture cannot tell the two apart; pick a more rotated bone"
    );
    // Pin what the export actually emits, as a literal.
    //
    // The comparison above is necessary but not sufficient: it recomputes its
    // expectation from the same `evaluate()` call the exporter uses, so an
    // error common to both cancels and the assert still passes. Only a constant
    // that came from a verified export can catch that.
    //
    // This moved from (8.643, 9.072) when the fixture's constraint went from
    // mixing translate on both axes at 0.5 to mixing x alone: y is no longer
    // pulled toward the target, so it moved four times as far as x did. A
    // change here that is *not* explained by the fixture is a regression.
    assert!(
        (got.0 - 8.384).abs() < 0.01 && (got.1 - 8.106).abs() < 0.01,
        "the emitted offset moved: {got:?}"
    );
}

/// Every IK key carries the constraint's bend direction.
///
/// Ankhimate has no bend-direction *timeline*: which way a chain bends is a
/// property of the constraint. Formats that read it per key default it to
/// "positive" when a key omits it, so a rig whose knees bend backwards
/// straightens them for the whole animation while its setup pose stays correct —
/// the export looks right until it moves.
#[test]
fn an_ik_key_carries_the_constraints_bend_direction() {
    let mut project = fixture();
    for c in &mut project.constraints {
        if c.kind == "ik" {
            c.bend_direction = -1.0;
        }
    }

    let ctx = Context::build(&project, None, ExportInfo::default());
    let merged = ctx.animations[0]["ik_by_constraint"]
        .as_array()
        .expect("the fixture animates an IK constraint")
        .clone();
    let keys = merged[0]["keys"].as_array().expect("keys ship").clone();
    assert!(!keys.is_empty(), "the fixture has IK keys to check");
    for key in &keys {
        assert_eq!(
            key["bend_direction"], -1.0,
            "every key repeats the setup value, got {key}"
        );
    }

    // And it follows the constraint rather than being hardcoded.
    let mut positive = fixture();
    for c in &mut positive.constraints {
        if c.kind == "ik" {
            c.bend_direction = 1.0;
        }
    }
    let ctx = Context::build(&positive, None, ExportInfo::default());
    let merged = ctx.animations[0]["ik_by_constraint"].as_array().unwrap();
    let keys = merged[0]["keys"].as_array().unwrap();
    assert_eq!(keys[0]["bend_direction"], 1.0);
}

/// A key's curve describes the segment **leaving** it, not arriving at it.
///
/// The schema stores a key's `interp` as how that key is *reached* — the easing
/// belongs to the key ending a segment. Spine, and most published formats, hang
/// it on the key that starts one. The exporter shifts frames by one; reading
/// `k.interp` for the segment `k -> k+1` instead left the first key of every
/// track linear and moved every other curve one key late.
///
/// Nothing caught that: keyframe poses matched exactly, because a curve only
/// affects the values *between* keys. Only the frames in between drifted, which
/// reads as a subtly wrong animation rather than an off-by-one.
#[test]
fn a_curve_describes_the_segment_leaving_its_key() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let spine = ctx.animations[0]["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "spine")
        .expect("the spine bone is keyed")
        .clone();
    let rotate = spine["rotate"].as_array().expect("rotate keys").clone();

    // The fixture: key0 t=0 linear, key1 t=0.5 bezier, key2 t=1.0 stepped.
    // The bezier on key1 is how key1 is *arrived at*, so it eases 0.0 -> 0.5.
    assert!(
        rotate[0]["is_bezier"].as_bool().unwrap(),
        "the segment leaving key0 is the bezier authored on key1"
    );
    assert!(
        !rotate[1]["is_bezier"].as_bool().unwrap(),
        "key1's own handles describe the segment before it, not after"
    );
    assert_eq!(
        rotate[1]["curve"], "stepped",
        "key2 is stepped, so the segment leaving key1 is"
    );

    // The first key of a track is never left without a curve just because
    // nothing precedes it — that was the visible half of the bug.
    assert!(
        !rotate[0]["points"].as_array().unwrap().is_empty(),
        "the first key carries the easing of its outgoing segment"
    );
}

/// Every constraint kind renders through the shipped preset.
///
/// The fixture and all four sample rigs carry only IK and transform
/// constraints, so the template's `path` and `physics` branches had never run
/// against data. A branch that no rig reaches is a branch that breaks silently
/// the first time a user's rig does reach it — and in strict mode a missing
/// field is a hard render error, so the failure is a refused export rather than
/// a wrong number.
#[test]
fn a_rig_with_every_constraint_kind_still_renders() {
    let mut project = fixture();
    project.slots.push(schema::Slot {
        name: "trail".into(),
        bone: "spine".into(),
        attachment: Some("trail-path".into()),
        color: [1.0, 1.0, 1.0, 1.0],
        dark_color: None,
        blend_mode: String::new(),
        extra: Default::default(),
    });
    project.skins[0].entries.push(schema::SkinEntry {
        slot: "trail".into(),
        name: "trail-path".into(),
        attachment: schema::Attachment::Path(schema::Path {
            vertices: vec![0.0, 0.0, 10.0, 0.0, 20.0, 0.0],
            closed: false,
            constant_speed: true,
            extra: Default::default(),
        }),
    });

    let base = project.constraints[0].clone();
    project.constraints.push(schema::Constraint {
        name: "trail-follow".into(),
        kind: "path".into(),
        bones: vec!["head".into()],
        slot: Some("trail".into()),
        path: Some([0.25, 0.5, 1.0, 1.0]),
        ..base.clone()
    });
    project.constraints.push(schema::Constraint {
        name: "hair-physics".into(),
        kind: "physics".into(),
        bones: vec!["head".into()],
        physics: Some([0.9, 0.4, 0.6, 1.2]),
        forces: Some([0.0, 0.0, 0.0, -9.8]),
        channels: Some([true, false]),
        ..base
    });
    project
        .constraint_order
        .extend(["trail-follow".into(), "hair-physics".into()]);

    // Strict mode: this errors rather than rendering an empty string if any
    // branch addresses a field the context does not carry.
    let rendered = render_spine(&project);
    let kinds: Vec<&str> = rendered["constraints"]
        .as_array()
        .expect("constraints ship")
        .iter()
        .filter_map(|c| c["type"].as_str())
        .collect();
    assert!(kinds.contains(&"path"), "{kinds:?}");
}

/// Renders the shipped Spine preset and parses the result as a consumer would.
///
/// Asserting on the *context* is what let a broken region size ship: the fields
/// were present and self-consistent, and the file Spine read was still wrong.
fn render_spine(project: &schema::Project) -> serde_json::Value {
    let preset = spine_preset();
    let plan = run::plan(project, &Default::default(), &preset).expect("the preset renders");
    let file = plan
        .files
        .iter()
        .find(|f| f.path.ends_with(".json"))
        .expect("the preset writes a skeleton");
    serde_json::from_str(&file.contents).expect("the preset emits valid JSON")
}

/// A mesh reports its image size in **rig space**, not file pixels.
///
/// A mesh's vertices are already in the rig's coordinates, so a format declaring
/// a mesh's dimensions alongside them needs the rig-space size — the two have to
/// agree or the mesh scales against the wrong extent. This is the opposite of a
/// region, whose `width`/`height` are a *draw* size for a whole image and so
/// pair with the file's own dimensions plus a scale.
///
/// Omitting them entirely, as this preset briefly did, makes a consumer read 0
/// and the mesh explodes across the skeleton.
#[test]
fn a_mesh_reports_its_size_in_rig_space() {
    // The cape's texture is the 24x24 `head` asset. At the rig's own scale the
    // rig-space size is the file size.
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let att = mesh_attachment(&ctx);
    assert_eq!(att["source_width"], 24.0, "the file's own pixels");
    assert_eq!(att["scaled_width"], 24.0, "and the rig agrees with it");

    // Double every region, so the rig is authored at 2x its art.
    let mut project = fixture();
    for entry in &mut project.skins[0].entries {
        if let schema::Attachment::Region(r) = &mut entry.attachment {
            r.width = 64.0;
            r.height = 64.0;
        }
    }
    let ctx = Context::build(&project, None, ExportInfo::default());
    let att = mesh_attachment(&ctx);
    assert_eq!(att["source_width"], 24.0, "the file has not changed");
    assert_eq!(att["scaled_width"], 48.0, "but rig space is twice it");
}

fn mesh_attachment(ctx: &Context) -> serde_json::Value {
    ctx.skeleton["skins"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "cape")
        .unwrap()["attachment"]
        .clone()
}

/// Art authored at a different resolution than the rig reports its scale.
///
/// A rig built against half-resolution images records each attachment at twice
/// its pixel size. Without a scale header every sprite draws at half size, which
/// looks like a broken exporter rather than a resolution mismatch.
#[test]
fn a_rig_authored_against_smaller_art_reports_its_scale() {
    let mut project = fixture();
    // The fixture's torso asset is 32x32; claim the attachment is twice that.
    for entry in &mut project.skins[0].entries {
        if let schema::Attachment::Region(r) = &mut entry.attachment {
            r.width = 64.0;
            r.height = 64.0;
        }
    }

    let ctx = Context::build(&project, None, ExportInfo::default());
    let att = mesh_attachment(&ctx);
    assert_eq!(
        att["scaled_width"], 48.0,
        "the rig is authored at twice the art's size"
    );

    // Art at the rig's own scale is the usual case and reports 1.0.
    let plain = Context::build(&fixture(), None, ExportInfo::default());
    let att = mesh_attachment(&plain);
    assert_eq!(att["scaled_width"], 24.0, "art at the rig's own scale");
}

/// A bezier key's control points are **absolute** time/value, not the normalized
/// handles the schema stores.
///
/// This is the bug that shipped: a preset printed `curve.handles` — numbers in
/// 0..1 — into a slot the consuming format reads as absolute coordinates. The
/// file parsed, imported, and animated wrongly, which is the worst failure mode
/// available. Asserting the *values* rather than merely that a `curve` key
/// exists is the whole point; the broken version had one too.
#[test]
fn bezier_control_points_are_absolute_not_normalized() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let walk = &ctx.animations[0];
    let spine = walk["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "spine")
        .expect("the spine bone is keyed");

    // A key's `interp` is how it is *arrived at*, so the bezier stored on the
    // t=0.5 key describes the segment 0.0 -> 0.5 and is emitted on key 0. The
    // segment runs value 0 -> 30 with handles [.25, 0, .75, 1]:
    // absolute t = 0 + h*0.5, v = 0 + h*30.
    let key = &spine["rotate"][0];
    let points: Vec<f64> = key["points"]
        .as_array()
        .expect("the key starting a bezier segment carries control points")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(
        points.len(),
        4,
        "a scalar channel has one control-point pair"
    );
    assert!((points[0] - 0.125).abs() < 1e-5, "out_x: {points:?}");
    assert!((points[1] - 0.0).abs() < 1e-5, "out_y: {points:?}");
    assert!((points[2] - 0.375).abs() < 1e-5, "in_x: {points:?}");
    assert!((points[3] - 30.0).abs() < 1e-5, "in_y: {points:?}");

    // And the key holding those handles describes the segment *leaving* it,
    // which the next key marks as stepped.
    assert_eq!(spine["rotate"][1]["curve"], "stepped");

    // The final key has nothing to interpolate towards, so it has no points at
    // all rather than points computed against itself.
    assert!(spine["rotate"][2]["points"].is_null());

    // A two-axis property is two tracks, each a scalar channel with its own
    // control points — four numbers, not eight. The axes have their own key
    // times and their own easing now, so there is no pair to concatenate.
    let head = walk["bones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "head")
        .unwrap();
    let x_points = head["translate_x"][0]["points"].as_array().unwrap();
    assert_eq!(x_points.len(), 4, "one axis, one control-point pair");
    // And the y track is genuinely its own: the fixture keys it at a different
    // time, which a paired representation could not express at all.
    let y_times: Vec<f64> = head["translate_y"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["time"].as_f64().unwrap())
        .collect();
    assert_eq!(y_times, vec![0.0, 0.5], "y keys where x does not");
}

/// `has_next` marks the keys that may carry a curve.
///
/// `points` already goes absent on a last key, but `curve` is per-key in the
/// schema regardless of position — a stepped or linear last key still reports
/// its interpolation. A template branching on that string has no other way to
/// know it must stay silent.
#[test]
fn the_last_key_of_a_channel_is_marked_as_having_no_successor() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let walk = &ctx.animations[0];

    for (bone, channel) in [("spine", "rotate"), ("head", "translate")] {
        let keys = walk["bones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == bone)
            .unwrap()[channel]
            .as_array()
            .unwrap();
        let (last, rest) = keys.split_last().expect("the channel has keys");
        assert_eq!(last["has_next"], false, "{bone}.{channel} last key");
        assert!(last["points"].is_null(), "{bone}.{channel} last key");
        for key in rest {
            assert_eq!(key["has_next"], true, "{bone}.{channel} interior key");
        }
    }

    // The merged IK view marks position against *its own* list: a time can be
    // last for one channel and not for the merged track.
    let keys = walk["ik_by_constraint"][0]["keys"].as_array().unwrap();
    let (last, rest) = keys.split_last().unwrap();
    assert_eq!(last["has_next"], false);
    for key in rest {
        assert_eq!(key["has_next"], true);
    }
}

/// A merged IK key's curve covers **every** channel, not whichever merged last.
///
/// The bug this pins shipped and reached Spine: the merge let the last channel
/// written own a single `curve`, so a softness ramp's control points described a
/// key whose primary value was `mix`. Spine read four numbers where it wanted
/// eight, rejected them, and then dereferenced the frame that was not there —
/// `[error] Invalid curve` followed by an NPE.
///
/// A channel that is merely linear still contributes its pair. A short array is
/// read positionally, so a gap misassigns every number after it.
#[test]
fn a_merged_ik_curve_covers_every_channel() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let keys = ctx.animations[0]["ik_by_constraint"][0]["keys"]
        .as_array()
        .unwrap();

    // t=0: mix is linear, softness is a bezier. The joined curve must still
    // carry both pairs — mix's first, softness's second.
    let points = keys[0]["points"]
        .as_array()
        .expect("a key with any bezier channel carries a joined curve");
    assert_eq!(points.len(), 8, "two channels, two pairs each: {points:?}");

    // Softness runs 4 -> 12 with handles [0.3, 0.1, 0.7, 0.9] over t 0 -> 0.5.
    let softness = &points[4..];
    assert!(
        (softness[0].as_f64().unwrap() - 0.15).abs() < 1e-4,
        "{points:?}"
    );
    assert!(
        (softness[1].as_f64().unwrap() - 4.8).abs() < 1e-4,
        "{points:?}"
    );
    assert!(
        (softness[3].as_f64().unwrap() - 11.2).abs() < 1e-4,
        "{points:?}"
    );

    // Mix runs 1 -> 0 linearly, so its pair is the straight line's thirds.
    let mix = &points[..4];
    assert!(
        (mix[1].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-4,
        "{points:?}"
    );

    // The last key has nothing to interpolate towards, joined or otherwise.
    assert!(keys.last().unwrap()["points"].is_null());
}

/// IK channels are also offered merged by constraint, because most formats write
/// one key list per constraint rather than one per channel.
#[test]
fn ik_channels_merge_into_one_key_list_per_constraint() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let merged = ctx.animations[0]["ik_by_constraint"].as_array().unwrap();
    assert_eq!(merged.len(), 1, "one entry per constraint, not per channel");
    assert_eq!(merged[0]["constraint"], "arm-ik");

    let keys = merged[0]["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2, "times unioned across channels: 0.0 and 0.5");
    // t=0.0 has mix and softness keys; bend direction is not keyed there, so it
    // reports the constraint's own value rather than nothing — see
    // `an_ik_key_carries_the_constraints_bend_direction` for why absent is
    // wrong.
    assert_eq!(keys[0]["mix"], 1.0);
    assert_eq!(keys[0]["softness"], 4.0);
    assert_eq!(
        keys[0]["bend_direction"], 1.0,
        "the constraint's setup value"
    );
    // t=0.5 has all three, merged into one key — and the keyed bend direction
    // wins over the setup value it seeds.
    assert_eq!(keys[1]["mix"], 0.0);
    assert_eq!(keys[1]["softness"], 12.0);
    assert_eq!(keys[1]["bend_direction"], -1.0);
}

/// Mesh edges are offered in the doubled form formats that index a flat vertex
/// array need, alongside the plain vertex-index pairs.
#[test]
fn mesh_edges_are_offered_as_component_offsets_too() {
    let ctx = Context::build(&fixture(), None, ExportInfo::default());
    let mesh = ctx.skeleton["skins"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "cape")
        .expect("the cape mesh is in the default skin")["attachment"]
        .clone();

    assert_eq!(mesh["edges"], serde_json::json!([0, 1, 1, 2, 2, 3, 3, 0]));
    assert_eq!(
        mesh["edges_x2"],
        serde_json::json!([0, 2, 2, 4, 4, 6, 6, 0])
    );
    // Every vertex of a quad is on its perimeter.
    assert_eq!(mesh["hull"], 4);
}

/// A mesh with no authored edges still exports its boundary and a real hull.
///
/// Vertex *order* cannot say which points are on the perimeter — `meshgen.rs` is
/// right to refuse that — but the triangulation can: an edge in exactly one
/// triangle is a boundary edge. Meshes imported from a format that dropped their
/// edge lists would otherwise export as "every vertex is hull, no edges", and
/// Spine answers that with "mesh internal edges lost" and rebuilds its own.
#[test]
fn a_mesh_without_authored_edges_derives_its_boundary_and_hull() {
    // A quad with one vertex in the middle: four perimeter vertices first, then
    // the interior one, which is the order the tracer produces.
    let att = cape_with(
        Vec::new(),
        vec![0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0, 5.0, 5.0],
        vec![0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4],
    );

    // The centre vertex touches every triangle twice, so it is interior; each
    // rim edge belongs to exactly one triangle.
    assert_eq!(
        att["edges"],
        serde_json::json!([0, 1, 0, 3, 1, 2, 2, 3]),
        "the boundary, and only the boundary"
    );
    assert_eq!(att["hull"], 4, "four of five vertices are on the rim");
    assert_eq!(att["vertex_count"], 5);
}

/// A hull is only claimed when the boundary really is the leading run.
///
/// Formats read `hull` as "the first N vertices are the outline". If the
/// boundary is scattered through the array, a count would slice the mesh in the
/// wrong place — worse than admitting no outline is known, which merely costs
/// the consumer its interior edges.
#[test]
fn a_scattered_boundary_claims_no_hull() {
    // The same fan with the interior vertex stored *first*, so the boundary is
    // 1..=4 rather than 0..=3.
    let att = cape_with(
        Vec::new(),
        vec![5.0, 5.0, 0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0],
        vec![1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1, 0],
    );

    assert_eq!(att["hull"], 5, "no prefix, so no outline is claimed");
    // The edges are still right — only the hull claim is withheld.
    assert_eq!(att["edges"], serde_json::json!([1, 2, 1, 4, 2, 3, 3, 4]));
}

/// Build the context for the fixture with the cape mesh's geometry replaced.
fn cape_with(edges: Vec<u32>, vertices: Vec<f32>, triangles: Vec<u32>) -> serde_json::Value {
    let mut project = fixture();
    let entry = project.skins[0]
        .entries
        .iter_mut()
        .find(|e| e.name == "cape")
        .expect("the fixture has a cape mesh");
    let schema::Attachment::Mesh(mesh) = &mut entry.attachment else {
        panic!("the cape is a mesh");
    };
    let count = vertices.len() / 2;
    mesh.edges = edges;
    mesh.uvs = vec![0.0; vertices.len()];
    mesh.vertices = vertices;
    mesh.triangles = triangles;
    mesh.weights = vec![vec![("spine".into(), 1.0)]; count];

    Context::build(&project, None, ExportInfo::default()).skeleton["skins"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "cape")
        .unwrap()["attachment"]
        .clone()
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
    assert_eq!(
        times,
        vec![0.25, 0.75, 1.25],
        "keys shift by the bone's offset"
    );
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

/// A preview plan skips PNG compression but not the bake, so the text it shows
/// is what a real export writes.
///
/// The editor's preview runs on every document change; encoding atlas pages it
/// then drops made the panel unusable. Skipping the *bake* instead would have
/// been the wrong economy — regions and UVs feed the context, so the rendered
/// text would silently disagree with the export, which is worse than no preview
/// at all.
#[test]
fn a_named_plan_matches_an_encoded_one_except_for_image_bytes() {
    let mut preset = preset_with(vec![simple(
        "atlas",
        "atlas.json",
        "{{#each atlas.regions}}{{name}}:{{x}},{{y}},{{width}},{{height}};{{/each}}",
        Cadence::Once,
    )]);
    preset.atlas.enabled = true;

    let encoded = run::plan(&fixture(), &assets(), &preset).unwrap();
    let named = run::plan_with(&fixture(), &assets(), &preset, run::Images::Named).unwrap();

    assert_eq!(
        encoded.files, named.files,
        "the rendered text must not depend on whether pages were compressed"
    );
    assert_eq!(
        encoded.paths(),
        named.paths(),
        "both plans write the same set of paths"
    );
    assert!(
        !encoded.binaries.is_empty() && encoded.binaries.iter().all(|(_, b)| !b.is_empty()),
        "the encoded plan carries real PNG bytes"
    );
    assert!(
        named.binaries.iter().all(|(_, b)| b.is_empty()),
        "the named plan carries none"
    );
}

/// Writing a preview plan is refused rather than producing empty images.
#[test]
fn a_preview_plan_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut preset = preset_with(vec![simple("skel", "skeleton.json", "{}", Cadence::Once)]);
    preset.atlas.enabled = true;

    let named = run::plan_with(&fixture(), &assets(), &preset, run::Images::Named).unwrap();
    let err = run::write(&named, dir.path()).expect_err("a byte-less plan must not be written");
    assert!(
        matches!(&err, ExportError::Io { reason, .. } if reason.contains("preview")),
        "{err}"
    );
    // Refused *before* anything landed, not halfway through.
    assert!(
        !dir.path().join("skeleton.json").exists(),
        "nothing is written when the plan is rejected"
    );
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
