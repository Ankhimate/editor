//! First-party export presets (T-603c, T-603e).
//!
//! Every format here is authored as a strict template rather than Rust. Foreign
//! formats belong in ordinary JavaScript community packages; their packaged
//! resources can reuse the same engine through `emitPreset`.

use crate::preset::Preset;

/// Ankhimate's own runtime format — the reference template.
pub const ANKHIMATE_RUNTIME: &str = include_str!("ankhimate_runtime.json");

/// A flat, engine-neutral JSON dump and the easiest starting point to modify.
pub const GENERIC_JSON: &str = include_str!("generic_json.json");

/// A Phaser 3 texture atlas, JSON Hash form.
///
/// Phaser has no skeletal-animation format, so this exports atlas regions only.
pub const PHASER_ATLAS: &str = include_str!("phaser_atlas.json");

/// Every first-party preset shipped with the editor.
pub fn builtin() -> Vec<Preset> {
    [ANKHIMATE_RUNTIME, GENERIC_JSON, PHASER_ATLAS]
        .iter()
        .filter_map(|text| Preset::from_json(text).ok())
        .collect()
}

/// The preset a new export starts from.
pub fn default_preset() -> Preset {
    Preset::from_json(ANKHIMATE_RUNTIME).expect("the shipped runtime preset parses")
}
