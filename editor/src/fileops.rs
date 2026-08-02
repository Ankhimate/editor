//! File▸New / Open / Save / Save As glue (T-108).
//!
//! Bridges the editor's [`AppState`] to `ankhimate-formats`. The `.ankh` path is
//! held on the app so Save can write in place while Save As always prompts.
//!
//! Since T-301 the document owns its images, so save bundles them and open
//! restores them, byte-for-byte — `formats` does the binding, this layer only
//! moves the whole `Document` across.

use crate::app_state::AppState;
use crate::doc::{Document, DocumentMeta};
use std::path::{Path, PathBuf};

const EXT: &str = "ankh";
const FILTER_NAME: &str = "Ankhimate project";

/// Outcome of a file operation, for surfacing a status line or error toast.
pub enum FileOutcome {
    Saved(PathBuf),
    Opened(PathBuf),
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
    use crate::commands::bone_cmds::{CreateBone, SetBoneTransform};
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

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
        use crate::commands::asset_cmds::ImportImage;
        use ankhimate_core::assets::ImageAsset;

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
}
