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
pub mod psd;
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

    /// Every attachment kind, a linked mesh, a sequence, a skin that owns bones
    /// and constraints, and an event with audio — through JSON and back.
    #[test]
    fn json_round_trip_preserves_the_whole_model() {
        use ankhimate_core::animation::EventKey;
        use ankhimate_core::attachment::{
            Attachment, BoundingBoxAttachment, LinkedMesh, MeshAttachment, PointAttachment,
            Sequence, SequenceMode, VertexWeight,
        };
        use ankhimate_core::constraints::{Constraint, IkConstraint};
        use ankhimate_core::skin::Skin;
        use ankhimate_core::slot::Slot;

        let mut skel = sample_skeleton();
        let arm = skel.bones.iter().find(|(_, b)| b.name == "arm").unwrap().0;
        let slot = skel.add_slot(Slot::new("arm_slot".to_string(), arm));
        let default_skin = skel.default_skin;

        let source = MeshAttachment {
            texture: "arm".into(),
            setup_vertices: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(1.0, 0.0),
                glam::vec2(0.0, 1.0),
            ],
            uvs: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(1.0, 0.0),
                glam::vec2(0.0, 1.0),
            ],
            triangles: vec![[0, 1, 2]],
            weights: vec![
                vec![VertexWeight {
                    bone: arm,
                    weight: 1.0,
                }],
                vec![VertexWeight {
                    bone: arm,
                    weight: 1.0,
                }],
                vec![VertexWeight {
                    bone: arm,
                    weight: 1.0,
                }],
            ],
            ..Default::default()
        };
        skel.skins[default_skin].set(slot, "source_mesh", Attachment::Mesh(source));
        skel.skins[default_skin].set(
            slot,
            "linked_mesh",
            Attachment::Mesh(MeshAttachment {
                texture: "arm_recolour".into(),
                linked: Some(LinkedMesh {
                    skin: None,
                    slot: "arm_slot".into(),
                    attachment: "source_mesh".into(),
                    inherit_deform: true,
                }),
                ..Default::default()
            }),
        );
        skel.skins[default_skin].set(
            slot,
            "hitbox",
            Attachment::BoundingBox(BoundingBoxAttachment {
                vertices: vec![
                    glam::vec2(0.0, 0.0),
                    glam::vec2(4.0, 0.0),
                    glam::vec2(4.0, 4.0),
                ],
                weights: vec![
                    vec![VertexWeight {
                        bone: arm,
                        weight: 1.0,
                    }],
                    vec![VertexWeight {
                        bone: arm,
                        weight: 1.0,
                    }],
                    vec![VertexWeight {
                        bone: arm,
                        weight: 1.0,
                    }],
                ],
            }),
        );
        skel.skins[default_skin].set(
            slot,
            "muzzle",
            Attachment::Point(PointAttachment {
                position: glam::vec2(7.0, -3.0),
                rotation: std::f32::consts::FRAC_PI_2,
            }),
        );
        skel.skins[default_skin].set(
            slot,
            "flash",
            Attachment::Region(ankhimate_core::attachment::RegionAttachment {
                texture: "flash_0".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 8.0,
                height: 8.0,
                uv_rect: Default::default(),
                pivot: glam::Vec2::splat(0.5),
                sequence: Some(Sequence {
                    frames: vec!["flash_0".into(), "flash_1".into(), "flash_2".into()],
                    fps: 24.0,
                    mode: SequenceMode::PingPong,
                    setup_index: 1,
                }),
            }),
        );

        let ik = skel.add_constraint(Constraint::Ik(IkConstraint {
            name: "cape_ik".into(),
            target: arm,
            bones: vec![arm],
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: 1.1,
            stiffness: 0.0,
        }));
        let mut cape = Skin::new("cape");
        cape.bones.push(arm);
        cape.constraints.push(ik);
        skel.add_skin(cape);

        let mut anims: SlotMap<AnimationId, Animation> = SlotMap::with_key();
        let mut anim = Animation::new("shoot", 1.0);
        anim.events.push(EventKey {
            time: 0.25,
            name: "footstep".into(),
            int_value: 3,
            float_value: 0.5,
            string_value: "left".into(),
            audio: "step_gravel".into(),
            volume: 0.8,
            balance: -0.25,
        });
        anims.insert(anim);

        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        // Re-serializing has to produce the same document, which is the only
        // check that catches a field written but never read back.
        let json2 = to_json(&project(
            &loaded.skeleton,
            &loaded.animations,
            &loaded.assets,
            &loaded.name,
            loaded.fps,
        ))
        .unwrap();
        assert_eq!(json, json2, "round trip is not a fixed point");

        let skin = &loaded.skeleton.skins[loaded.skeleton.default_skin];
        let slot = loaded
            .skeleton
            .slots
            .iter()
            .find(|(_, s)| s.name == "arm_slot")
            .unwrap()
            .0;
        assert!(matches!(
            skin.get(slot, "hitbox"),
            Some(Attachment::BoundingBox(b)) if b.weights.len() == 3
        ));
        assert!(matches!(
            skin.get(slot, "muzzle"),
            Some(Attachment::Point(p)) if (p.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-4
        ));
        assert!(matches!(
            skin.get(slot, "linked_mesh"),
            Some(Attachment::Mesh(m)) if m.linked.as_ref().unwrap().attachment == "source_mesh"
        ));
        assert!(matches!(
            skin.get(slot, "flash"),
            Some(Attachment::Region(r))
                if r.sequence.as_ref().unwrap().mode == SequenceMode::PingPong
        ));

        let cape = loaded
            .skeleton
            .skins
            .iter()
            .find(|(_, s)| s.name == "cape")
            .unwrap()
            .1;
        assert_eq!(cape.bones.len(), 1, "skin kept its bone");
        assert_eq!(cape.constraints.len(), 1, "skin kept its constraint");

        let event = &loaded.animations.values().next().unwrap().events[0];
        assert_eq!(event.audio, "step_gravel");
        assert!((event.volume - 0.8).abs() < 1e-6);
        assert!((event.balance + 0.25).abs() < 1e-6);
    }

    /// A linked mesh draws the source's geometry, and editing the source moves
    /// every copy — the entire reason a link beats a duplicate.
    #[test]
    fn a_linked_mesh_follows_its_source() {
        use ankhimate_core::attachment::{Attachment, LinkedMesh, MeshAttachment};
        use ankhimate_core::slot::Slot;

        let mut skel = sample_skeleton();
        let arm = skel.bones.iter().find(|(_, b)| b.name == "arm").unwrap().0;
        let slot = skel.add_slot(Slot::new("arm_slot".to_string(), arm));
        let skin = skel.default_skin;
        skel.skins[skin].set(
            slot,
            "source",
            Attachment::Mesh(MeshAttachment {
                setup_vertices: vec![glam::vec2(5.0, 5.0)],
                ..Default::default()
            }),
        );
        let link = MeshAttachment {
            texture: "other".into(),
            linked: Some(LinkedMesh {
                skin: None,
                slot: "arm_slot".into(),
                attachment: "source".into(),
                inherit_deform: true,
            }),
            ..Default::default()
        };
        assert_eq!(
            skel.resolve_linked_mesh(&[], &link).setup_vertices,
            vec![glam::vec2(5.0, 5.0)]
        );

        // A link that points nowhere resolves to itself, so a broken reference
        // draws the mesh rather than a hole.
        let dangling = MeshAttachment {
            linked: Some(LinkedMesh {
                skin: None,
                slot: "nope".into(),
                attachment: "nope".into(),
                inherit_deform: true,
            }),
            ..Default::default()
        };
        assert!(
            skel.resolve_linked_mesh(&[], &dangling)
                .setup_vertices
                .is_empty()
        );
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

    /// The N-bone IK sample. Regenerate with the `gen_tentacle` example.
    const TENTACLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/tentacle.ankh");

    /// The shipped sample really does carry a chain longer than two bones, and
    /// the chain really does reach (T-908).
    ///
    /// Both halves matter. The sample exists to show a capability every other
    /// editor lacks, so a version of it that loads but does not solve — or that
    /// quietly lost bones off its chain in a schema change — would advertise the
    /// opposite of what it is for.
    #[test]
    fn the_tentacle_sample_ships_a_long_ik_chain_that_reaches() {
        let (doc, _) = load(std::path::Path::new(TENTACLE)).unwrap();
        assert!(doc.report.is_clean(), "sample loads cleanly");

        let ik = doc
            .skeleton
            .constraints
            .values()
            .find_map(|c| match c {
                ankhimate_core::constraints::Constraint::Ik(ik) => Some(ik),
                _ => None,
            })
            .expect("sample has an IK constraint");
        assert!(
            ik.bones.len() > 2,
            "the whole point is a chain no two-bone solver can express, got {}",
            ik.bones.len()
        );

        let mut pose = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&doc.skeleton, &[], &mut pose);

        // The target is inside the chain's reach by construction, so the tip
        // should arrive at it rather than merely point that way.
        let tip = pose.world_tip(&doc.skeleton, *ik.bones.last().unwrap());
        let goal = pose.world_position(ik.target);
        assert!(
            (tip - goal).length() < 1.0,
            "tip {tip:?} should reach target {goal:?}"
        );
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
            markers: Vec::new(),
            bone_offsets: Vec::new(),
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
            markers: Vec::new(),
            bone_offsets: Vec::new(),
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
    /// T-506: events were modelled in core but silently dropped on save — the
    /// conversion built an empty list in both directions.
    #[test]
    fn animation_events_survive_a_round_trip() {
        use ankhimate_core::animation::EventKey;

        let skel = sample_skeleton();
        let arm_bone = skel.bones.iter().find(|(_, b)| b.name == "arm").unwrap().0;
        let root_bone = skel.bones.iter().find(|(_, b)| b.name == "root").unwrap().0;
        let mut anims = SlotMap::with_key();
        anims.insert(Animation {
            name: "walk".into(),
            duration: 1.0,
            looping: true,
            timelines: Vec::new(),
            events: vec![
                EventKey {
                    time: 0.25,
                    name: "footstep".into(),
                    int_value: 3,
                    float_value: 0.8,
                    string_value: "left".into(),
                    audio: String::new(),
                    volume: 1.0,
                    balance: 0.0,
                },
                EventKey {
                    time: 0.75,
                    name: "footstep".into(),
                    int_value: 4,
                    float_value: 0.8,
                    string_value: "right".into(),
                    audio: String::new(),
                    volume: 1.0,
                    balance: 0.0,
                },
            ],
            // Deliberately out of order, and deliberately sharing times with the
            // events above: markers and events are separate lists that happen to
            // sit on the same ruler, and a round trip must not merge or reorder
            // one into the other (T-906).
            markers: vec![
                ankhimate_core::animation::Marker::new(0.75, "up"),
                ankhimate_core::animation::Marker::new(0.0, "contact"),
            ],
            // A negative offset as well as a positive one: leading is the half
            // of T-905 that needs evaluation to tolerate negative sample times,
            // so a round trip that only carried trailing would miss it.
            bone_offsets: vec![(arm_bone, 0.125), (root_bone, -0.05)],
        });

        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let clip = loaded.animations.values().next().expect("the clip");
        // Offsets come back keyed to the same bones, by name.
        assert_eq!(clip.bone_offsets.len(), 2);
        for (bone, offset) in &clip.bone_offsets {
            let name = loaded.skeleton.bones[*bone].name.as_str();
            let expected = if name == "arm" { 0.125 } else { -0.05 };
            assert!(
                (*offset - expected).abs() < 1e-6,
                "{name} offset {offset} != {expected}"
            );
        }
        let events = &clip.events;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].name, "footstep");
        assert_eq!(events[0].int_value, 3);
        assert!((events[0].float_value - 0.8).abs() < 1e-6);
        assert_eq!(events[1].string_value, "right");

        // Markers survive, and arrive sorted however they were written.
        let names: Vec<&str> = clip.markers.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["contact", "up"], "markers sorted on load");
        assert_eq!(clip.events.len(), 2, "markers did not leak into events");
    }
    /// Selection sets survive a round trip, by bone *name* (T-904).
    ///
    /// The point of a set being document state is that a rigger builds it once
    /// and whoever opens the file next has it — which is only true if it is
    /// actually written and read back.
    #[test]
    fn selection_sets_survive_a_round_trip() {
        use ankhimate_core::skeleton::SelectionSet;

        let mut skel = sample_skeleton();
        let bones: Vec<_> = skel.bones.keys().collect();
        skel.selection_sets.push(SelectionSet {
            name: "left arm".into(),
            bones: bones.clone(),
        });

        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        assert_eq!(loaded.skeleton.selection_sets.len(), 1);
        let set = &loaded.skeleton.selection_sets[0];
        assert_eq!(set.name, "left arm");
        assert_eq!(set.bones.len(), bones.len());
        // Ids are per-load, so identity is checked through the names they resolve
        // to — which is also what the file actually stores.
        let names: Vec<&str> = set
            .bones
            .iter()
            .map(|b| loaded.skeleton.bones[*b].name.as_str())
            .collect();
        assert!(names.contains(&"root"));
        assert!(names.contains(&"arm"));
    }

    /// A set naming a bone the rig no longer has is reported, not silently
    /// shrunk — a set that selects three of the eight bones it names is worse
    /// than one that says it lost five.
    #[test]
    fn a_selection_set_naming_a_missing_bone_is_reported() {
        let skel = sample_skeleton();
        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();

        // Splice in a set referencing a bone that does not exist.
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["selection_sets"] = serde_json::json!([{
            "name": "ghosts",
            "bones": ["root", "no_such_bone"],
        }]);
        let json = serde_json::to_string(&value).unwrap();

        let loaded = from_json(&json).unwrap();
        assert!(
            !loaded.report.is_clean(),
            "a dangling bone name must be reported"
        );
        // The rest of the set still loads: losing one name should not lose the
        // whole group.
        assert_eq!(loaded.skeleton.selection_sets.len(), 1);
        assert_eq!(loaded.skeleton.selection_sets[0].bones.len(), 1);
    }

    /// T-505: blend mode and the two-color tint are slot fields the schema
    /// already had but nothing wrote; visibility keys are new.
    #[test]
    fn slot_presentation_and_visibility_survive_a_round_trip() {
        use ankhimate_core::animation::{Key, Timeline};
        use ankhimate_core::slot::{BlendMode, Slot};

        let mut skel = sample_skeleton();
        let bone = skel.bones.keys().next().unwrap();
        let slot = skel.add_slot(Slot {
            blend_mode: BlendMode::Additive,
            dark_color: Some([0.1, 0.2, 0.3, 1.0]),
            ..Slot::new("flash".into(), bone)
        });

        let mut anims = SlotMap::with_key();
        anims.insert(Animation {
            name: "blink".into(),
            duration: 1.0,
            looping: false,
            events: Vec::new(),
            markers: Vec::new(),
            bone_offsets: Vec::new(),
            timelines: vec![Timeline::SlotVisible {
                slot,
                keys: vec![Key::stepped(0.3, false), Key::stepped(0.6, true)],
            }],
        });

        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let slot = loaded
            .skeleton
            .slots
            .values()
            .find(|s| s.name == "flash")
            .expect("the slot came back");
        assert_eq!(slot.blend_mode, BlendMode::Additive);
        assert_eq!(slot.dark_color, Some([0.1, 0.2, 0.3, 1.0]));

        let keys = loaded
            .animations
            .values()
            .flat_map(|a| &a.timelines)
            .find_map(|t| match t {
                Timeline::SlotVisible { keys, .. } => Some(keys.clone()),
                _ => None,
            })
            .expect("the visibility timeline came back");
        assert_eq!(keys.len(), 2);
        assert!(!keys[0].value && keys[1].value);
    }
    /// T-503: a physics constraint's dials must round-trip. Its bone lives in
    /// the schema's `target` field so the constraint shape stays uniform, which
    /// is exactly the sort of mapping that silently loses data.
    #[test]
    fn physics_constraints_survive_a_round_trip() {
        use ankhimate_core::constraints::{Constraint, PhysicsConstraint};

        let mut skel = sample_skeleton();
        let bone = skel.bones.keys().next().unwrap();
        skel.add_constraint(Constraint::Physics(PhysicsConstraint {
            inertia: 0.8,
            strength: 55.0,
            damping: 0.25,
            mass: 2.5,
            wind: glam::vec2(3.0, 0.0),
            gravity: glam::vec2(0.0, -9.8),
            mix: 0.75,
            rotate: true,
            translate: true,
            ..PhysicsConstraint::sway("hair", bone)
        }));

        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let Some(Constraint::Physics(p)) = loaded.skeleton.constraints.values().next() else {
            panic!("the physics constraint came back");
        };
        assert_eq!(p.name, "hair");
        assert!((p.inertia - 0.8).abs() < 1e-6);
        assert!((p.strength - 55.0).abs() < 1e-6);
        assert!((p.damping - 0.25).abs() < 1e-6);
        assert!((p.mass - 2.5).abs() < 1e-6);
        assert!((p.wind - glam::vec2(3.0, 0.0)).length() < 1e-6);
        assert!((p.gravity - glam::vec2(0.0, -9.8)).length() < 1e-5);
        assert!((p.mix - 0.75).abs() < 1e-6);
        assert!(p.rotate && p.translate);
        assert_eq!(
            loaded.skeleton.bones[p.bone].name, skel.bones[bone].name,
            "it points at the same bone"
        );
    }
    /// T-502: a path attachment and the constraint that drives bones along it.
    /// The constraint is the only one whose source is a *slot*, which is
    /// exactly the mapping most likely to be dropped.
    #[test]
    fn paths_and_path_constraints_survive_a_round_trip() {
        use ankhimate_core::attachment::{Attachment, PathAttachment};
        use ankhimate_core::constraints::{Constraint, PathConstraint};
        use ankhimate_core::slot::Slot;

        let mut skel = sample_skeleton();
        let bone = skel.bones.keys().next().unwrap();
        let slot = skel.add_slot(Slot {
            attachment: Some("curve".into()),
            ..Slot::new("path_slot".into(), bone)
        });
        let default = skel.default_skin;
        skel.skins[default].set(
            slot,
            "curve".to_string(),
            Attachment::Path(PathAttachment {
                vertices: vec![glam::vec2(0.0, 0.0), glam::vec2(10.0, 5.0)],
                closed: true,
                constant_speed: false,
            }),
        );
        skel.add_constraint(Constraint::Path(PathConstraint {
            position: 0.3,
            spacing: 0.7,
            mix_rotate: 0.9,
            mix_translate: 0.4,
            ..PathConstraint::new("tail", slot, vec![bone])
        }));

        let anims = SlotMap::with_key();
        let assets = AssetDb::new();
        let json = to_json(&project(&skel, &anims, &assets, "hero", 30)).unwrap();
        let loaded = from_json(&json).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);

        let path = loaded
            .skeleton
            .skins
            .values()
            .find_map(|s| {
                s.entries.values().find_map(|a| match a {
                    Attachment::Path(p) => Some(p.clone()),
                    _ => None,
                })
            })
            .expect("the path attachment came back");
        assert_eq!(path.vertices.len(), 2);
        assert!(path.closed);
        assert!(!path.constant_speed, "the flag is not defaulted back on");

        let Some(Constraint::Path(c)) = loaded.skeleton.constraints.values().next() else {
            panic!("the path constraint came back");
        };
        assert_eq!(c.name, "tail");
        assert!((c.position - 0.3).abs() < 1e-6);
        assert!((c.spacing - 0.7).abs() < 1e-6);
        assert!((c.mix_rotate - 0.9).abs() < 1e-6);
        assert!((c.mix_translate - 0.4).abs() < 1e-6);
        assert_eq!(
            loaded.skeleton.slots[c.slot].name, "path_slot",
            "it points at the same slot"
        );
    }
    /// The checked-in sample rig must load, resolve every reference, and pose.
    ///
    /// It is the only artefact that exercises bones, slots, skins, draw order,
    /// keyframes, an IK constraint and events together, so a break here is a
    /// break in the pipeline rather than in one module's tests.
    #[test]
    fn the_sample_rig_loads_and_poses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("samples")
            .join("walker.ankh");
        if !path.exists() {
            eprintln!("skipping: run `cargo run -p ankhimate-formats --example make_sample`");
            return;
        }

        let (loaded, _) = load(&path).unwrap();
        assert!(loaded.report.is_clean(), "{:?}", loaded.report);
        assert_eq!(loaded.skeleton.bones.len(), 12);
        assert_eq!(loaded.skeleton.slots.len(), 10);
        assert_eq!(loaded.assets.images.len(), 10, "art came back with the rig");

        let anim = loaded
            .animations
            .values()
            .find(|a| a.name == "walk")
            .expect("the walk cycle");
        assert_eq!(anim.events.len(), 2, "two footsteps");

        // The IK clip has to actually bend the leg, which is the whole reason
        // the sample ships a constraint: a demo that needs a slider found first
        // is not a demo.
        let ik_clip = loaded
            .animations
            .values()
            .find(|a| a.name == "leg_ik")
            .expect("the IK clip");
        let shin = loaded
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "shin_l")
            .map(|(id, _)| id)
            .expect("a near shin");
        let mut ik_pose = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&loaded.skeleton, &[(ik_clip, 0.0, 1.0)], &mut ik_pose);
        let knee_start = ik_pose.world_decomposed(shin).rotation;
        ankhimate_core::pose::evaluate(&loaded.skeleton, &[(ik_clip, 0.25, 1.0)], &mut ik_pose);
        let knee_mid = ik_pose.world_decomposed(shin).rotation;
        assert!(
            (knee_start - knee_mid).abs() > 0.1,
            "the knee solved to a different angle as the target moved:              {knee_start} vs {knee_mid}"
        );

        // It has to actually move: the same rig at two times must differ.
        let mut pose = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&loaded.skeleton, &[(anim, 0.0, 1.0)], &mut pose);
        let head = loaded
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "head")
            .map(|(id, _)| id)
            .expect("a head");
        let start = pose.world_position(head);
        ankhimate_core::pose::evaluate(&loaded.skeleton, &[(anim, 0.25, 1.0)], &mut pose);
        let quarter = pose.world_position(head);
        assert!(
            (start - quarter).length() > 1.0,
            "the walk cycle moves the head: {start:?} vs {quarter:?}"
        );
    }
}
