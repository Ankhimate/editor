//! `ankhimate-runtime` (T-604) — playing an exported rig in a game.
//!
//! # Why this is thin
//!
//! Almost nothing here is animation logic. `ankhimate-core` already owns the
//! model, the constraint solvers and `evaluate()`, and it is framework-free and
//! `wasm32`-clean precisely so a game can link it (PLAN §3.1). Reimplementing
//! any of that here would create a second answer to "what pose is this rig in",
//! and the two would drift — the editor would show one thing and the game
//! another, which is the single worst failure this project can have.
//!
//! So this crate is three things core deliberately does not do:
//!
//! - **Load** an exported file back into a `Skeleton` (via `ankhimate-formats`).
//! - **Track playback**: what is playing, how far in, what is fading into what.
//! - **Emit draw batches**: geometry a renderer can hand to a GPU.
//!
//! Crossfade is `evaluate()`'s existing alpha mixing, not new code:
//! [`AnimationState`] hands core a weighted list and core blends it. That is why
//! a crossfade in the game looks like a crossfade in the editor.
//!
//! No wgpu, no window, no filesystem assumptions beyond reading bytes. Builds
//! for `wasm32`.

#![forbid(unsafe_code)]

pub mod batch;
pub mod state;

pub use batch::{Blend, DrawBatch, Vertex, build_batches};
pub use state::{AnimationState, TrackEntry};

use ankhimate_core::animation::Animation;
use ankhimate_core::assets::AssetDb;
use ankhimate_core::ids::AnimationId;
use ankhimate_core::skeleton::Skeleton;
use ankhimate_core::slotmap::SlotMap;

/// A rig loaded and ready to play.
pub struct Rig {
    pub skeleton: Skeleton,
    pub animations: SlotMap<AnimationId, Animation>,
    /// Image metadata. Pixels are the host's problem — a runtime that decoded
    /// images would need an image crate and a texture API, and every engine
    /// already has both.
    pub assets: AssetDb,
    pub name: String,
    pub fps: u32,
}

#[derive(Debug)]
pub enum LoadError {
    Parse(String),
    /// The file loaded, but references inside it did not resolve. Carries the
    /// names, because "something is missing" is not actionable.
    Dangling(Vec<String>),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(e) => write!(f, "could not parse the rig: {e}"),
            LoadError::Dangling(names) => {
                write!(f, "unresolved references: {}", names.join(", "))
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl Rig {
    /// Load from a `project.json`-shaped document.
    ///
    /// Dangling references are reported but do not fail the load — the same rule
    /// the editor follows (ADR 0004). A rig missing one attachment should still
    /// play; refusing to load it helps nobody at runtime, where there is no user
    /// to fix the file.
    pub fn from_json(text: &str) -> Result<(Self, Vec<String>), LoadError> {
        let project =
            ankhimate_formats::from_json(text).map_err(|e| LoadError::Parse(e.to_string()))?;

        let dangling: Vec<String> = project
            .report
            .dangling
            .iter()
            .map(|d| format!("{d:?}"))
            .collect();

        Ok((
            Self {
                skeleton: project.skeleton,
                animations: project.animations,
                assets: project.assets,
                name: project.name,
                fps: project.fps,
            },
            dangling,
        ))
    }

    /// The animation named `name`, if the rig has one.
    pub fn animation(&self, name: &str) -> Option<AnimationId> {
        self.animations
            .iter()
            .find(|(_, a)| a.name == name)
            .map(|(id, _)| id)
    }

    /// Every animation name, sorted — for a game that wants to list them.
    pub fn animation_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .animations
            .iter()
            .map(|(_, a)| a.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }
}
