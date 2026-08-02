//! The document — the only undoable, savable state (PLAN §3.2).
//!
//! Everything here is authored data. Derived state (`Pose`) and UI state
//! (selection, camera, tool) live elsewhere: `Pose` is recomputed per frame from
//! this, and [`Session`](crate::session::Session) holds what the user is *doing*
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
#[derive(Debug, Default)]
pub struct Document {
    pub skeleton: Skeleton,
    pub animations: SlotMap<AnimationId, Animation>,
    /// Imported images (T-301). Attachments reference these by name, so the
    /// library outlives any one attachment that samples it.
    pub assets: AssetDb,
    pub meta: DocumentMeta,
}

impl Document {
    /// A new empty document whose skeleton already has its default skin.
    pub fn new() -> Self {
        Self {
            skeleton: Skeleton::new(),
            animations: SlotMap::with_key(),
            assets: AssetDb::new(),
            meta: DocumentMeta::default(),
        }
    }

    /// Borrow the whole document for serialization.
    pub fn as_project_ref(&self) -> ankhimate_formats::ProjectRef<'_> {
        ankhimate_formats::ProjectRef {
            skeleton: &self.skeleton,
            animations: &self.animations,
            assets: &self.assets,
            name: &self.meta.name,
            fps: self.meta.fps,
        }
    }
}
