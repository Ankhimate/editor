//! The document — the only undoable, savable state (PLAN §3.2).
//!
//! Everything here is authored data. Derived state (`Pose`) and UI state
//! (selection, camera, tool) live elsewhere: `Pose` is recomputed per frame from
//! this, and the editor's `Session` holds what the user is *doing*
//! rather than what they have *made*.
//!
//! Mutating a `Document` outside an [`EditCommand`](crate::commands::EditCommand)
//! bypasses undo. Panels should build a command and dispatch it.

use ankhimate_core::animation::Animation;
use ankhimate_core::assets::AssetDb;
use ankhimate_core::ids::AnimationId;
use ankhimate_core::skeleton::Skeleton;
use ankhimate_core::slotmap::SlotMap;

/// Project-level settings that are neither skeleton nor animation.
#[derive(Debug, Clone)]
pub struct DocumentMeta {
    pub name: String,
    /// Frames per second the timeline displays and playback advances at.
    pub fps: u32,
}

impl Default for DocumentMeta {
    fn default() -> Self {
        Self {
            name: "untitled".to_string(),
            fps: 30,
        }
    }
}

/// The undoable state of an open project.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub skeleton: Skeleton,
    pub animations: SlotMap<AnimationId, Animation>,
    /// Imported images (T-301). Attachments reference these by name, so the
    /// library outlives any one attachment that samples it.
    pub assets: AssetDb,
    pub meta: DocumentMeta,
    /// Asset name → the PSD layer path it came from (T-302).
    ///
    /// Kept so a re-import can tell "this is the same arm, redrawn" from "this
    /// is a new layer". Undoable state, because an import writes it and undo has
    /// to take it back.
    pub psd_layer_paths: std::collections::HashMap<String, String>,
    /// Export presets (T-603), kept as JSON.
    ///
    /// Document state, not session state: a rig's export settings belong to the
    /// rig and have to survive a reopen, and editing one is undoable like any
    /// other document edit.
    ///
    /// Stored serialized rather than as `Preset` values because
    /// [`Self::as_project_ref`] hands out borrows and cannot serialize on the
    /// way past. It also means a preset written by a newer editor survives a
    /// round trip through an older one untouched, which a typed field would
    /// silently truncate.
    pub export_presets: Vec<serde_json::Value>,
}

impl Document {
    /// A new empty document whose skeleton already has its default skin.
    pub fn new() -> Self {
        Self {
            skeleton: Skeleton::new(),
            animations: SlotMap::with_key(),
            assets: AssetDb::new(),
            meta: DocumentMeta::default(),
            psd_layer_paths: std::collections::HashMap::new(),
            export_presets: Vec::new(),
        }
    }

    /// Build document state from the format layer's name-resolved result.
    ///
    /// Importers use this at the serialization boundary, then replace the open
    /// document through an [`EditCommand`](crate::commands::EditCommand). Keeping
    /// the field mapping here avoids every headless consumer inventing its own.
    pub fn from_loaded(loaded: ankhimate_formats::Loaded) -> Self {
        Self {
            skeleton: loaded.skeleton,
            animations: loaded.animations,
            assets: loaded.assets,
            meta: DocumentMeta {
                name: loaded.name,
                fps: loaded.fps,
            },
            psd_layer_paths: loaded.psd_layer_paths,
            export_presets: loaded.export_presets,
        }
    }

    /// The presets, parsed. Anything that no longer deserializes is skipped
    /// rather than failing the document — a preset from a newer editor is a
    /// reason to leave it alone, not to refuse to open the rig.
    pub fn presets(&self) -> Vec<ankhimate_export::preset::Preset> {
        self.export_presets
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect()
    }

    /// Borrow the whole document for serialization.
    pub fn as_project_ref(&self) -> ankhimate_formats::ProjectRef<'_> {
        ankhimate_formats::ProjectRef {
            skeleton: &self.skeleton,
            animations: &self.animations,
            assets: &self.assets,
            name: &self.meta.name,
            fps: self.meta.fps,
            export_presets: &self.export_presets,
            // Carried so a re-import can tell a redrawn layer from a new one.
            psd_layer_paths: &self.psd_layer_paths,
        }
    }
}
