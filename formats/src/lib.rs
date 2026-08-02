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
}
