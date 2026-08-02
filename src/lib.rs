//! `.ankh` file format: read and write Ankhimate projects (T-108, ADR 0004).
//!
//! The stack, outermost to innermost:
//!
//! ```text
//! save(path, ..)                          load(path)
//!   convert::to_schema  (core → schema)     container::read   (zip → json)
//!   serde_json          (schema → json)     serde_json        (json → schema)
//!   container::write    (json → zip)        migrate           (→ current ver)
//!                                           convert::from_schema (schema → core)
//! ```
//!
//! * [`schema`] is the on-disk shape — name-keyed, degrees, unknown-field-tolerant.
//! * [`convert`] is the only place ids↔names and radians↔degrees cross.
//! * [`migrate`] steps an old file forward before it reaches the core model.
//! * [`container`] is the zip wrapper (`project.json` + `images/`).

pub mod container;
pub mod convert;
pub mod migrate;
pub mod schema;

use std::path::Path;

pub use container::ImageBlob;
pub use convert::{LoadReport, Loaded, ProjectRef};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Container(#[from] container::ContainerError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Migrate(#[from] migrate::MigrateError),
}

/// Serialize a document to `project.json` bytes (pretty-printed).
///
/// Split out from [`save`] so callers with their own container strategy — tests,
/// exporters — can reach the JSON without touching the filesystem. Asset *bytes*
/// are not in here: the JSON is the index, the container carries the pixels.
pub fn to_json(project: &ProjectRef<'_>) -> Result<String, Error> {
    let schema = convert::to_schema(project);
    Ok(serde_json::to_string_pretty(&schema)?)
}

/// Parse `project.json` bytes into a document, migrating older versions forward.
pub fn from_json(json: &str) -> Result<Loaded, Error> {
    let project: schema::Project = serde_json::from_str(json)?;
    let project = migrate::migrate(project)?;
    Ok(convert::from_schema(&project))
}

/// Write a document to an `.ankh` container at `path`.
///
/// Asset bytes are written verbatim — never re-encoded — so reopening a project
/// returns the user's own pixels (T-301). `extra_images` is for blobs that are
/// not assets (none today; kept so an exporter can ride along).
pub fn save(
    path: &Path,
    project: &ProjectRef<'_>,
    extra_images: &[ImageBlob],
) -> Result<(), Error> {
    let json = to_json(project)?;
    let mut images: Vec<ImageBlob> = project
        .assets
        .images
        .iter()
        .filter(|(_, a)| !a.bytes.is_empty())
        .map(|(_, a)| ImageBlob {
            rel_path: convert::asset_file_name(a),
            bytes: a.bytes.clone(),
        })
        .collect();
    for blob in extra_images {
        images.push(ImageBlob {
            rel_path: blob.rel_path.clone(),
            bytes: blob.bytes.clone(),
        });
    }
    container::write(path, &json, &images)?;
    Ok(())
}

/// Read an `.ankh` container from `path`.
///
/// Asset bytes are bound back onto [`Loaded::assets`] by matching each blob's
/// path against the index in `project.json`; blobs that match no asset are
/// returned as-is rather than dropped, so a newer writer's extras survive.
pub fn load(path: &Path) -> Result<(Loaded, Vec<ImageBlob>), Error> {
    let contents = container::read(path)?;
    let mut loaded = from_json(&contents.project_json)?;

    // name → file, from the index we just parsed.
    let index: std::collections::HashMap<String, String> =
        match serde_json::from_str::<schema::Project>(&contents.project_json) {
            Ok(p) => p.assets.into_iter().map(|a| (a.file, a.name)).collect(),
            Err(_) => Default::default(),
        };

    let mut unclaimed = Vec::new();
    for blob in contents.images {
        match index
            .get(&blob.rel_path)
            .and_then(|name| loaded.assets.by_name(name))
        {
            Some(id) => {
                if let Some(asset) = loaded.assets.images.get_mut(id) {
                    asset.bytes = blob.bytes;
                }
            }
            None => unclaimed.push(blob),
        }
    }

    // An asset the container did not carry is a broken reference, not a crash.
    let missing: Vec<String> = loaded
        .assets
        .images
        .iter()
        .filter(|(_, a)| a.bytes.is_empty())
        .map(|(_, a)| a.name.clone())
        .collect();
    for name in missing {
        loaded.report.dangling.push(("asset image", name));
    }

    Ok((loaded, unclaimed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::animation::Animation;
    use ankhimate_core::assets::{AssetDb, ImageAsset};
    use ankhimate_core::ids::AnimationId;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::{Bone, Skeleton};
    use ankhimate_core::slotmap::SlotMap;
    use ankhimate_core::transforms::Inherit;

    /// A `ProjectRef` over the pieces a test happens to care about.
    fn project<'a>(
        skeleton: &'a Skeleton,
        animations: &'a SlotMap<AnimationId, Animation>,
        assets: &'a AssetDb,
        name: &'a str,
        fps: u32,
    ) -> ProjectRef<'a> {
        ProjectRef {
            skeleton,
            animations,
            assets,
            name,
            fps,
        }
    }

    fn sample_skeleton() -> Skeleton {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 40.0,
            local_transform: Transform::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        skel.add_bone(Bone {
            name: "arm".into(),
            parent: Some(root),
            length: 30.0,
            local_transform: Transform {
                position: glam::vec2(40.0, 0.0),
                rotation: 15.0_f32.to_radians(),
                scale: glam::vec2(1.0, 1.0),
                shear: glam::vec2(0.0, 0.0),
            },
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        skel
    }

    #[test]
    fn json_round_trip_preserves_bones() {
        let skel = sample_skeleton();
        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();

        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean());
        assert_eq!(loaded.name, "hero");
        assert_eq!(loaded.fps, 30);
        assert_eq!(loaded.skeleton.bones.len(), 2);

        // Re-serialize and compare structurally: same document, same JSON.
        let json2 = to_json(&project(
            &loaded.skeleton,
            &loaded.animations,
            &loaded.assets,
            &loaded.name,
            loaded.fps,
        ))
        .unwrap();
        assert_eq!(json, json2, "second round trip is a fixed point");
    }

    /// T-301 acceptance: asset pixels survive a save/reopen byte-for-byte, and
    /// come back attached to the right asset.
    #[test]
    fn container_round_trip_matches_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.ankh");
        let skel = sample_skeleton();
        let anims = SlotMap::with_key();

        // A one-pixel PNG, so the magic-byte sniff picks the right extension.
        let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
        let mut assets = AssetDb::new();
        assets.add(ImageAsset::new("arm", png.clone(), 16, 32));

        save(&path, &project(&skel, &anims, &assets, "hero", 24), &[]).unwrap();
        let (loaded, extras) = load(&path).unwrap();

        assert_eq!(loaded.name, "hero");
        assert_eq!(loaded.fps, 24);
        assert_eq!(loaded.skeleton.bones.len(), 2);
        assert!(extras.is_empty(), "every blob was claimed by an asset");

        let id = loaded.assets.by_name("arm").expect("asset came back");
        let asset = loaded.assets.get(id).unwrap();
        assert_eq!(asset.bytes, png, "bytes preserved, not re-encoded");
        assert_eq!((asset.width, asset.height), (16, 32));
        assert!(loaded.report.is_clean());

        // Rotation survived the radians→degrees→radians trip within tolerance.
        let arm = loaded
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "arm")
            .map(|(_, b)| b)
            .unwrap();
        assert!((arm.local_transform.rotation - 15.0_f32.to_radians()).abs() < 1e-4);
    }

    /// The checked-in golden. Regenerate with the `gen_minimal` example.
    const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/minimal.ankh");

    #[test]
    fn golden_load_save_load_is_value_equal() {
        // load → save → load must reproduce the document byte-for-byte in JSON.
        let (first, _) = load(std::path::Path::new(GOLDEN)).unwrap();
        assert!(first.report.is_clean(), "golden loads cleanly");

        // Acceptance shape: 2 bones, 1 slot, 1 skin entry, 1 animation.
        assert_eq!(first.skeleton.bones.len(), 2);
        assert_eq!(first.skeleton.slots.len(), 1);
        let skin = &first.skeleton.skins[first.skeleton.default_skin];
        assert_eq!(skin.entries.len(), 1);
        assert_eq!(first.animations.len(), 1);

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("resave.ankh");
        save(
            &out,
            &project(
                &first.skeleton,
                &first.animations,
                &first.assets,
                &first.name,
                first.fps,
            ),
            &[],
        )
        .unwrap();

        let (second, _) = load(&out).unwrap();
        let json1 = to_json(&project(
            &first.skeleton,
            &first.animations,
            &first.assets,
            &first.name,
            first.fps,
        ))
        .unwrap();
        let json2 = to_json(&project(
            &second.skeleton,
            &second.animations,
            &second.assets,
            &second.name,
            second.fps,
        ))
        .unwrap();
        assert_eq!(json1, json2, "round trip is a fixed point");
    }

    #[test]
    fn unknown_top_level_field_survives_round_trip() {
        // The golden carries a hand-added `editor_note` the schema does not know.
        let contents = container::read(std::path::Path::new(GOLDEN)).unwrap();
        let project: schema::Project = serde_json::from_str(&contents.project_json).unwrap();
        assert!(
            project.extra.contains_key("editor_note"),
            "unknown field captured into `extra`"
        );

        // Re-serialize the parsed schema; the unknown field must still be there.
        let reserialized = serde_json::to_string(&project).unwrap();
        assert!(
            reserialized.contains("editor_note"),
            "unknown field written back out"
        );
    }
    /// T-401/T-405: the two pieces of geometry that are *not* vertices — a
    /// mesh's pinned edges and a clip's polygon — have to survive a save. Both
    /// are new fields on old structures, which is exactly where a serializer
    /// silently drops something.
    #[test]
    fn pinned_edges_and_clips_survive_a_round_trip() {
        use ankhimate_core::attachment::{Attachment, ClippingAttachment, MeshAttachment};
        use ankhimate_core::slot::Slot;

        let mut skel = sample_skeleton();
        let bone = skel.bones.keys().next().unwrap();
        let art = skel.add_slot(Slot {
            attachment: Some("art".into()),
            ..Slot::new("art_slot".to_string(), bone)
        });
        let mask = skel.add_slot(Slot {
            attachment: Some("mask".into()),
            ..Slot::new("mask_slot".to_string(), bone)
        });

        let mesh = MeshAttachment {
            texture: "arm".into(),
            setup_vertices: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(10.0, 0.0),
                glam::vec2(10.0, 10.0),
                glam::vec2(0.0, 10.0),
            ],
            uvs: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(1.0, 0.0),
                glam::vec2(1.0, 1.0),
                glam::vec2(0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            edges: vec![[0, 2]],
            ..Default::default()
        };
        let clip = ClippingAttachment {
            vertices: vec![
                glam::vec2(-5.0, -5.0),
                glam::vec2(5.0, -5.0),
                glam::vec2(0.0, 5.0),
            ],
            end_slot: Some("art_slot".into()),
        };
        let skin = skel.default_skin;
        skel.skins[skin].set(art, "art".to_string(), Attachment::Mesh(mesh));
        skel.skins[skin].set(mask, "mask".to_string(), Attachment::Clipping(clip));

        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let skin = loaded.skeleton.default_skin;
        let Some(Attachment::Mesh(mesh)) = loaded.skeleton.skins[skin].get(art, "art") else {
            panic!("the mesh came back");
        };
        assert_eq!(mesh.edges, vec![[0, 2]], "pinned edges survived");

        let Some(Attachment::Clipping(clip)) = loaded.skeleton.skins[skin].get(mask, "mask") else {
            panic!("the clip came back");
        };
        assert_eq!(clip.vertices.len(), 3);
        assert_eq!(
            clip.end_slot.as_deref(),
            Some("art_slot"),
            "range preserved"
        );
    }
    /// T-501: a transform constraint and its mix timeline must survive a save.
    /// The constraint schema was IK-shaped, so this is the round trip most
    /// likely to quietly drop half a constraint.
    #[test]
    fn transform_constraints_survive_a_round_trip() {
        use ankhimate_core::animation::{Interp, Key, Timeline};
        use ankhimate_core::constraints::{Constraint, TransformConstraint};

        let mut skel = sample_skeleton();
        let mut ids = skel.bones.keys();
        let target = ids.next().unwrap();
        let driven = ids.next().unwrap();

        let cid = skel.add_constraint(Constraint::Transform(TransformConstraint {
            offsets: Transform {
                position: glam::vec2(3.0, -4.0),
                rotation: 15.0_f32.to_radians(),
                scale: glam::vec2(1.5, 0.5),
                shear: glam::vec2(5.0_f32.to_radians(), 0.0),
            },
            mix_rotate: 0.75,
            mix_translate: 0.25,
            mix_scale: 0.5,
            mix_shear: 0.1,
            local: true,
            relative: true,
            ..TransformConstraint::rotation_only("look", target, vec![driven])
        }));

        let mut anims = SlotMap::with_key();
        let anim: AnimationId = anims.insert(Animation {
            name: "fade".into(),
            duration: 1.0,
            looping: false,
            events: Vec::new(),
            timelines: vec![Timeline::TransformConstraintMix {
                constraint: cid,
                keys: vec![Key {
                    time: 0.5,
                    value: [1.0, 0.0, 0.0, 0.0],
                    interp: Interp::Linear,
                }],
            }],
        });
        let _ = anim;

        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let Some(Constraint::Transform(tc)) = loaded.skeleton.constraints.values().next() else {
            panic!("the constraint came back as a transform constraint");
        };
        assert_eq!(tc.name, "look");
        assert!((tc.mix_rotate - 0.75).abs() < 1e-6);
        assert!((tc.mix_translate - 0.25).abs() < 1e-6);
        assert!((tc.mix_scale - 0.5).abs() < 1e-6);
        assert!((tc.mix_shear - 0.1).abs() < 1e-6);
        assert!(tc.local && tc.relative);
        // Degrees on disk, radians in memory (ADR 0002) — the conversion has to
        // survive both directions.
        assert!(
            (tc.offsets.rotation - 15.0_f32.to_radians()).abs() < 1e-5,
            "offset rotation: {}",
            tc.offsets.rotation.to_degrees()
        );
        assert!((tc.offsets.position - glam::vec2(3.0, -4.0)).length() < 1e-5);
        assert!((tc.offsets.scale - glam::vec2(1.5, 0.5)).length() < 1e-5);

        let mix_keys = loaded
            .animations
            .values()
            .flat_map(|a| &a.timelines)
            .find_map(|t| match t {
                Timeline::TransformConstraintMix { keys, .. } => Some(keys.len()),
                _ => None,
            });
        assert_eq!(mix_keys, Some(1), "the mix timeline came back");
    }
    /// T-504: softness, stretch and the stretch limit were serialized long
    /// before they were implemented, and the two new IK timelines are new
    /// schema. All of it has to come back.
    #[test]
    fn ik_completeness_fields_survive_a_round_trip() {
        use ankhimate_core::animation::{Interp, Key, Timeline};
        use ankhimate_core::constraints::{Constraint, IkConstraint};

        let mut skel = sample_skeleton();
        let mut ids = skel.bones.keys();
        let root = ids.next().unwrap();
        let target = ids.next().unwrap();

        let cid = skel.add_constraint(Constraint::Ik(IkConstraint {
            softness: 7.5,
            stretch: true,
            stretch_limit: 1.35,
            bend_direction: -1.0,
            ..IkConstraint::aim("reach", target, root)
        }));

        let mut anims = SlotMap::with_key();
        anims.insert(Animation {
            name: "clip".into(),
            duration: 1.0,
            looping: false,
            events: Vec::new(),
            timelines: vec![
                Timeline::IkBendDirection {
                    constraint: cid,
                    keys: vec![Key {
                        time: 0.0,
                        value: -1.0,
                        interp: Interp::Stepped,
                    }],
                },
                Timeline::IkSoftness {
                    constraint: cid,
                    keys: vec![Key {
                        time: 0.25,
                        value: 3.0,
                        interp: Interp::Linear,
                    }],
                },
            ],
        });

        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let Some(Constraint::Ik(ik)) = loaded.skeleton.constraints.values().next() else {
            panic!("the IK constraint came back");
        };
        assert!((ik.softness - 7.5).abs() < 1e-6);
        assert!(ik.stretch);
        assert!((ik.stretch_limit - 1.35).abs() < 1e-6);
        assert!((ik.bend_direction + 1.0).abs() < 1e-6);

        let kinds: Vec<&str> = loaded
            .animations
            .values()
            .flat_map(|a| &a.timelines)
            .map(|t| match t {
                Timeline::IkBendDirection { .. } => "bend",
                Timeline::IkSoftness { .. } => "softness",
                _ => "other",
            })
            .collect();
        assert!(
            kinds.contains(&"bend") && kinds.contains(&"softness"),
            "{kinds:?}"
        );
    }
}
