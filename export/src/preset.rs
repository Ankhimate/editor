//! Export presets (T-603b) — a user-authored format, as data.
//!
//! A preset is saved **in the project**, because a rig's export settings belong
//! to the rig, and is also importable/exportable standalone so a studio can
//! share one across projects.
//!
//! Template bodies are stored **inline, not as file paths**. A path makes the
//! project non-portable: send the `.ankh` to a colleague and their export
//! silently produces nothing, or worse, something stale. Inline costs some
//! duplication between projects, which is exactly what the standalone preset
//! file is for.

use crate::atlas::AtlasSettings;
use serde::{Deserialize, Serialize};

/// Bumped when a field is renamed or removed — presets are user data that
/// outlives the version that wrote them.
pub const PRESET_VERSION: u32 = 1;

/// How often a template runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    /// One output file for the whole export.
    #[default]
    Once,
    /// One output file per animation, with `{{animation}}` bound to that clip.
    Animation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// Display name, and the name errors are reported against.
    pub name: String,
    /// Output path, itself a template — this is what makes per-animation file
    /// naming (`anim/{{animation.name}}.json`) work with no extra machinery.
    pub output_path: String,
    #[serde(default)]
    pub per: Cadence,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default = "current_preset_version")]
    pub preset_version: u32,
    pub name: String,
    /// Relative to the `.ankh`, or absolute.
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
    pub atlas: AtlasOptions,
    #[serde(default)]
    pub templates: Vec<Template>,
    /// Write the source images alongside, for presets that use no atlas.
    #[serde(default)]
    pub copy_images: bool,
    /// A note to the preset's users — which engine, which version, which spec.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn current_preset_version() -> u32 {
    PRESET_VERSION
}

/// Atlas settings plus whether to bake one at all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasOptions {
    pub enabled: bool,
    /// Filename stem: `atlas.png`, `atlas_2.png`, …
    pub stem: String,
    #[serde(flatten)]
    pub settings: AtlasSettings,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            stem: "atlas".into(),
            settings: AtlasSettings::default(),
        }
    }
}

impl Preset {
    /// An empty preset, as the editor's "new preset" starting point.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            preset_version: PRESET_VERSION,
            name: name.into(),
            output_dir: "export".into(),
            atlas: AtlasOptions::default(),
            templates: Vec::new(),
            copy_images: false,
            description: String::new(),
        }
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("preset serializes")
    }
}
