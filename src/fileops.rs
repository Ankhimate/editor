//! File▸New / Open / Save / Save As glue (T-108).
//!
//! Bridges the editor's [`AppState`] to `ankhimate-formats`. The `.ankh` path is
//! held on the app so Save can write in place while Save As always prompts.
//!
//! Since T-301 the document owns its images, so save bundles them and open
//! restores them, byte-for-byte — `formats` does the binding, this layer only
//! moves the whole `Document` across.

use crate::app_state::AppState;
use ankhimate_document::doc::{Document, DocumentMeta};
use std::path::{Path, PathBuf};

const EXT: &str = "ankh";
const FILTER_NAME: &str = "Ankhimate project";

/// Outcome of a file operation, for surfacing a status line or error toast.
pub enum FileOutcome {
    Saved(PathBuf),
    Opened(PathBuf),
    /// A foreign rig was read in. Carries what the conversion could not keep, so
    /// the caller can show it: unlike opening an `.ankh`, importing *invents*
    /// data, and a status line is too small to admit to hundreds of
    /// approximations.
    Imported {
        path: PathBuf,
        report: ankhimate_formats::convert::LoadReport,
    },
    /// User dismissed the native dialog.
    Cancelled,
    Error(String),
}

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter(FILTER_NAME, &[EXT])
}

/// File▸New — replace the document with an empty one. No dialog, no path.
pub fn new_document(state: &mut AppState) {
    state.replace_document(Document::new());
}

/// File▸Save — write to `current_path` if set, else fall through to Save As.
pub fn save(state: &AppState, current_path: &Option<PathBuf>) -> FileOutcome {
    match current_path {
        Some(path) => write_to(state, path),
        None => save_as(state),
    }
}

/// File▸Save As — always prompt for a destination.
pub fn save_as(state: &AppState) -> FileOutcome {
    let Some(mut path) = dialog()
        .set_file_name(format!("{}.{EXT}", state.doc.meta.name))
        .save_file()
    else {
        return FileOutcome::Cancelled;
    };
    // rfd honors the filter but a user can still type a bare name; make sure the
    // extension is present so the file is recognizable.
    if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
        path.set_extension(EXT);
    }
    write_to(state, &path)
}

/// File▸Open — prompt, then hand off to [`open_path`].
pub fn open(state: &mut AppState) -> FileOutcome {
    let Some(path) = dialog().pick_file() else {
        return FileOutcome::Cancelled;
    };
    open_path(state, &path)
}

/// Load `path` and swap the document in. The dialog-free seam `open` calls once
/// the user has picked a file — and the one a headless test can drive.
pub fn open_path(state: &mut AppState, path: &Path) -> FileOutcome {
    match ankhimate_formats::load(path) {
        Ok((loaded, _images)) => {
            let report = loaded.report;
            state.replace_document(Document {
                skeleton: loaded.skeleton,
                animations: loaded.animations,
                assets: loaded.assets,
                meta: DocumentMeta {
                    name: loaded.name,
                    fps: loaded.fps,
                },
                // Layer provenance is per-import, not per-file: opening a
                // project cannot know which PSD its images came from.
                psd_layer_paths: Default::default(),
                export_presets: loaded.export_presets,
            });
            // A dangling reference is not a failed open — the project is usable
            // minus that one thing — but the user should hear about it.
            if !report.is_clean() {
                let what: Vec<String> = report
                    .dangling
                    .iter()
                    .map(|(kind, name)| format!("{kind} '{name}'"))
                    .collect();
                state
                    .session
                    .set_status(format!("Opened with unresolved: {}", what.join(", ")));
            }
            FileOutcome::Opened(path.to_path_buf())
        }
        Err(e) => FileOutcome::Error(format!("open failed: {e}")),
    }
}

/// Every importer the build knows about.
///
/// Built once per call rather than held on the app: registration is cheap, and
/// a plugin host will want to add to it per session rather than at startup.
pub fn importers() -> ankhimate_formats::Importers {
    ankhimate_formats::Importers::builtin()
}

/// The built-ins plus whatever the loaded plugins add.
///
/// A plugin that registers a format reaches the File▸Import menu, a dropped
/// file and an id lookup through this one call — none of those three know a
/// plugin exists, which is the duplication the registry was built to remove.
pub fn importers_with(plugins: &crate::plugins::Plugins) -> ankhimate_formats::Importers {
    let mut importers = importers();
    plugins.register_importers(&mut importers);
    importers
}

/// File▸Import▸<format> — prompt with that format's extensions, then read.
///
/// The user has already said which format, so the named importer is used rather
/// than guessed at: obeying beats guessing when the caller knows.
pub fn import_with(
    state: &mut AppState,
    plugins: &crate::plugins::Plugins,
    id: &str,
) -> FileOutcome {
    let importers = importers_with(plugins);
    let Some(importer) = importers.get(id) else {
        return FileOutcome::Error(format!("no importer named `{id}`"));
    };
    let Some(path) = rfd::FileDialog::new()
        .add_filter(importer.label(), &importer.extensions())
        .set_title(format!("Import {}", importer.label()))
        .pick_file()
    else {
        return FileOutcome::Cancelled;
    };
    import_path_with(state, plugins, id, &path)
}

/// Read `path` with the importer `id` names. The dialog-free seam.
pub fn import_path_with(
    state: &mut AppState,
    plugins: &crate::plugins::Plugins,
    id: &str,
    path: &Path,
) -> FileOutcome {
    let importers = importers_with(plugins);
    let Some(importer) = importers.get(id) else {
        return FileOutcome::Error(format!("no importer named `{id}`"));
    };
    match importer.read(path) {
        Ok(loaded) => adopt(state, loaded, path),
        Err(e) => FileOutcome::Error(format!("{}: {e}", path.display())),
    }
}

/// Swap an imported rig in and hand back its report.
///
/// Shared by every importer: what a foreign format becomes is a `Loaded`, and
/// from there the editor does not care which reader produced it. Keeping this in
/// one place is what stops a new importer quietly forgetting to carry the report
/// across — the part of an import the user most needs to see.
fn adopt(state: &mut AppState, loaded: ankhimate_formats::Loaded, path: &Path) -> FileOutcome {
    let report = loaded.report;
    state.replace_document(Document {
        skeleton: loaded.skeleton,
        animations: loaded.animations,
        assets: loaded.assets,
        meta: DocumentMeta {
            name: loaded.name,
            fps: loaded.fps,
        },
        // Layer provenance is per-import, not per-file.
        psd_layer_paths: Default::default(),
        export_presets: loaded.export_presets,
    });
    FileOutcome::Imported {
        path: path.to_path_buf(),
        report,
    }
}

/// Write `state`'s document to `path`. The dialog-free seam `save`/`save_as`
/// funnel through — and the one a headless test can drive.
pub fn write_to(state: &AppState, path: &Path) -> FileOutcome {
    match ankhimate_formats::save(path, &state.doc.as_project_ref(), &[]) {
        Ok(()) => FileOutcome::Saved(path.to_path_buf()),
        Err(e) => FileOutcome::Error(format!("save failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_document::commands::bone_cmds::{CreateBone, SetBoneTransform};

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.to_string(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    fn community_plugins() -> crate::plugins::Plugins {
        crate::plugins::Plugins::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("community-plugins"),
        )
    }

    /// Drives the exact code path the File menu triggers — command → document →
    /// `write_to` → `open_path` into a fresh state — minus the OS dialog. Proves
    /// the editor's save/load wiring, not just the `formats` crate in isolation.
    #[test]
    fn author_save_reopen_preserves_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("smoke.ankh");

        // Author a document the way the canvas tools do: dispatched commands.
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        let posed = Transform {
            position: glam::vec2(12.0, -7.0),
            rotation: 0.4,
            ..Default::default()
        };
        state.dispatch(Box::new(SetBoneTransform::new(root, posed)));
        state.doc.meta.name = "smoke".into();

        // Save through the same function the menu calls.
        assert!(matches!(write_to(&state, &path), FileOutcome::Saved(_)));
        assert!(path.exists(), "a file landed on disk");

        // Open into a brand-new editor state, as File▸Open would.
        let mut reopened = AppState::default();
        assert!(matches!(
            open_path(&mut reopened, &path),
            FileOutcome::Opened(_)
        ));

        assert_eq!(reopened.doc.meta.name, "smoke");
        assert_eq!(reopened.doc.skeleton.bones.len(), 1);
        let (_, b) = reopened.doc.skeleton.bones.iter().next().unwrap();
        assert_eq!(b.name, "root");
        assert!((b.local_transform.position.x - 12.0).abs() < 1e-4);
        assert!((b.local_transform.rotation - 0.4).abs() < 1e-4);

        // Opening resets history and pose; the reloaded bone shows in the pose.
        assert_eq!(reopened.pose.worlds.len(), 1);
        assert!(!reopened.history.can_undo(), "load clears undo");
    }

    /// T-301 acceptance: importing three images produces three drawable slots,
    /// and a save/reopen returns the *same bytes* — the project is the pixels,
    /// not a set of paths that go stale.
    #[test]
    fn imported_images_round_trip_with_their_pixels() {
        use ankhimate_core::assets::ImageAsset;
        use ankhimate_document::commands::asset_cmds::ImportImage;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.ankh");

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();

        // Distinct byte patterns so a mix-up on load is visible.
        let png = |tag: u8| {
            let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
            bytes.extend_from_slice(&[tag; 16]);
            bytes
        };
        for (i, name) in ["head", "torso", "arm"].iter().enumerate() {
            state.dispatch(Box::new(ImportImage::new(
                ImageAsset::new(*name, png(i as u8), 32 + i as u32, 16),
                root,
                glam::vec2(i as f32 * 10.0, 0.0),
            )));
        }

        assert_eq!(state.doc.assets.len(), 3);
        assert_eq!(
            state.doc.skeleton.draw_order.len(),
            3,
            "three drawable slots"
        );

        assert!(matches!(write_to(&state, &path), FileOutcome::Saved(_)));

        let mut reopened = AppState::default();
        assert!(matches!(
            open_path(&mut reopened, &path),
            FileOutcome::Opened(_)
        ));

        assert_eq!(reopened.doc.assets.len(), 3);
        assert_eq!(reopened.doc.skeleton.draw_order.len(), 3);
        for (i, name) in ["head", "torso", "arm"].iter().enumerate() {
            let id = reopened
                .doc
                .assets
                .by_name(name)
                .unwrap_or_else(|| panic!("{name} came back"));
            let asset = reopened.doc.assets.get(id).unwrap();
            assert_eq!(asset.bytes, png(i as u8), "{name} kept its exact bytes");
            assert_eq!(asset.width, 32 + i as u32);
        }

        // Every slot still resolves to a region attachment pointing at an asset
        // that exists — the shape the renderer needs to draw anything.
        for &slot in &reopened.doc.skeleton.draw_order {
            let att = reopened
                .doc
                .skeleton
                .resolve_slot(reopened.session.active_skin, slot)
                .expect("attachment resolves through the default skin");
            match att {
                ankhimate_core::attachment::Attachment::Region(r) => {
                    assert!(
                        reopened.doc.assets.by_name(&r.texture).is_some(),
                        "region references a live asset"
                    );
                    assert!(r.width > 0.0 && r.height > 0.0);
                }
                _ => panic!("expected a region attachment"),
            }
        }
        assert!(
            !reopened
                .session
                .status
                .as_deref()
                .unwrap_or("")
                .contains("unresolved")
        );
    }

    /// A Spine skeleton with one bone, one slot, and a curve that overshoots.
    const SPINE_RIG: &str = r#"{
      "skeleton": { "spine": "4.3.23" },
      "bones": [{ "name": "root" }, { "name": "arm", "parent": "root", "x": 12 }],
      "slots": [{ "name": "hand", "bone": "arm" }],
      "animations": { "wave": { "bones": { "arm": { "rotate": [
        { "value": 0, "curve": [0.1, -20, 0.2, 30] },
        { "time": 0.5, "value": 30 }
      ] } } } }
    }"#;

    /// Importing replaces the document with the foreign rig.
    ///
    /// Drives the seam the File▸Import menu item calls, past the native dialog.
    #[test]
    fn importing_a_spine_rig_replaces_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.json");
        std::fs::write(&path, SPINE_RIG).unwrap();

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("leftover"))));
        let before = state.revision;

        let outcome = import_path_with(&mut state, &community_plugins(), "import.spine", &path);
        assert!(
            matches!(outcome, FileOutcome::Imported { .. }),
            "expected an import outcome"
        );
        // The previous document is gone, not merged into.
        assert!(state.doc.skeleton.bones.values().any(|b| b.name == "arm"));
        assert!(
            !state
                .doc
                .skeleton
                .bones
                .values()
                .any(|b| b.name == "leftover")
        );
        assert_eq!(state.doc.meta.name, "hero");
        assert_eq!(state.doc.animations.len(), 1);
        assert_ne!(state.revision, before, "a new rig is a document change");
    }

    /// A rig with no images still imports, and says what it could not find.
    ///
    /// Geometry is the expensive part to rebuild by hand; refusing the whole
    /// file because its art is elsewhere would throw that away. The missing
    /// pieces are named instead.
    #[test]
    fn importing_without_images_reports_rather_than_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.json");
        // A region attachment whose image is nowhere beside the skeleton.
        let rig = SPINE_RIG.replace(
            r#""animations""#,
            r#""skins": [{ "name": "default", "attachments": {
                 "hand": { "glove": { "width": 10, "height": 10 } } } }],
               "animations""#,
        );
        std::fs::write(&path, rig).unwrap();

        let mut state = AppState::default();
        let FileOutcome::Imported { report, .. } =
            import_path_with(&mut state, &community_plugins(), "import.spine", &path)
        else {
            panic!("a rig without images still imports");
        };
        assert!(state.doc.skeleton.bones.values().any(|b| b.name == "arm"));
        assert!(
            !report.dangling.is_empty(),
            "the missing image is named: {report:?}"
        );
    }

    /// A minimal DragonBones rig: one armature, two bones, one clip.
    ///
    /// `root` is written with no `transform` at all, which is how real files do
    /// it and the case an importer is most likely to get wrong by reaching for
    /// a neighbouring value.
    const DRAGONBONES_RIG: &str = r#"{
        "name": "golem", "frameRate": 24, "version": "5.5",
        "armature": [{
            "name": "golem", "frameRate": 24,
            "bone": [
                {"name": "root"},
                {"name": "arm", "parent": "root", "length": 30,
                 "transform": {"x": 10, "y": 5, "skX": 20, "skY": 20}}
            ],
            "slot": [{"name": "hand", "parent": "arm"}],
            "skin": [{"slot": []}],
            "animation": [{
                "name": "wave", "duration": 24,
                "bone": [{"name": "arm", "rotateFrame": [
                    {"duration": 12, "tweenEasing": 0},
                    {"duration": 12, "tweenEasing": 0, "rotate": 45},
                    {"duration": 0}
                ]}]
            }]
        }]
    }"#;

    #[test]
    fn importing_a_dragonbones_rig_replaces_the_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("golem_ske.json");
        std::fs::write(&path, DRAGONBONES_RIG).unwrap();

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("leftover"))));
        let before = state.revision;

        let outcome = import_path_with(
            &mut state,
            &community_plugins(),
            "import.dragonbones",
            &path,
        );
        assert!(
            matches!(outcome, FileOutcome::Imported { .. }),
            "expected an import outcome"
        );
        assert!(state.doc.skeleton.bones.values().any(|b| b.name == "arm"));
        assert!(
            !state
                .doc
                .skeleton
                .bones
                .values()
                .any(|b| b.name == "leftover"),
            "the previous document is replaced, not merged into"
        );
        // The file's own name wins over the file stem — unlike Spine, a
        // DragonBones document stores one.
        assert_eq!(state.doc.meta.name, "golem");
        assert_eq!(state.doc.meta.fps, 24);
        assert_eq!(state.doc.animations.len(), 1);
        assert_ne!(state.revision, before, "a new rig is a document change");
    }

    /// Frame counts become seconds at the armature's own rate.
    ///
    /// The format's defining quirk, checked through the editor's seam rather
    /// than only in `formats`: a clip whose duration survives the reader but is
    /// mis-scaled here would play at the wrong speed with nothing to point at.
    #[test]
    fn a_dragonbones_clip_arrives_in_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("golem_ske.json");
        std::fs::write(&path, DRAGONBONES_RIG).unwrap();

        let mut state = AppState::default();
        import_path_with(
            &mut state,
            &community_plugins(),
            "import.dragonbones",
            &path,
        );

        let clip = state.doc.animations.values().next().expect("one clip");
        assert_eq!(clip.name, "wave");
        assert_eq!(clip.duration, 1.0, "24 frames at 24fps is one second");
    }

    /// A `.json` that is not a DragonBones skeleton fails without touching the
    /// document — and in particular does not fall back to the Spine reader.
    #[test]
    fn a_bad_dragonbones_import_leaves_the_document_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        std::fs::write(&path, r#"{"hello": "world"}"#).unwrap();

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("keep-me"))));

        let outcome = import_path_with(
            &mut state,
            &community_plugins(),
            "import.dragonbones",
            &path,
        );
        assert!(matches!(outcome, FileOutcome::Error(_)));
        assert!(
            state
                .doc
                .skeleton
                .bones
                .values()
                .any(|b| b.name == "keep-me")
        );
    }

    /// A file that is not a Spine skeleton fails without touching the document.
    #[test]
    fn a_bad_import_leaves_the_open_document_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.json");
        std::fs::write(&path, r#"{"hello": "world"}"#).unwrap();

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("keep-me"))));
        let before = state.revision;

        let outcome = import_path_with(&mut state, &community_plugins(), "import.spine", &path);
        assert!(matches!(outcome, FileOutcome::Error(_)));
        assert!(
            state
                .doc
                .skeleton
                .bones
                .values()
                .any(|b| b.name == "keep-me"),
            "a failed import must not replace what was open"
        );
        assert_eq!(state.revision, before);
    }
}
