//! Shipped presets (T-603c, T-603e).
//!
//! Every one of these is authored **as a template**, not as Rust — including
//! Ankhimate's own runtime format. That is the point: if our own format needed a
//! feature the template engine lacks, we would have found out by discovering the
//! engine was too weak, rather than by a user discovering it after us. It also
//! means each shipped preset doubles as a worked example to clone.
//!
//! # Clean-room (PLAN §0)
//!
//! A preset targeting another engine is written from **that engine's own public
//! documentation**, never by reading another editor's exporter — several are
//! GPL-3.0, and an exporter is precisely the artefact where copying is both
//! tempting and detectable. Where a format is a third party's proprietary
//! schema, we do not ship a preset for it. A user may author one themselves,
//! which is the whole reason the feature is user-authorable.

use crate::preset::Preset;

/// Ankhimate's own runtime format — the reference template.
pub const ANKHIMATE_RUNTIME: &str = include_str!("ankhimate_runtime.json");

/// A flat, engine-neutral JSON dump. The easiest starting point to modify.
pub const GENERIC_JSON: &str = include_str!("generic_json.json");

/// A Phaser 3 texture atlas, JSON Hash form.
///
/// Written from Phaser's published texture documentation and the field list its
/// own loader accepts. Atlas only: Phaser has no skeletal-animation format, so
/// this exports regions and nothing else, which is the honest scope rather than
/// a bone format invented on Phaser's behalf.
pub const PHASER_ATLAS: &str = include_str!("phaser_atlas.json");

/// Spine JSON, targeting the 4.3-shaped schema.
///
/// Written from Esoteric Software's published format documentation plus the
/// **observed shape of a 4.3 export** — the same clean-room footing as every
/// other preset here (PLAN §0). No Spine runtime source and no other editor's
/// Spine exporter was consulted; several of the latter are GPL-3.0 and this is
/// exactly the artefact where copying would be both tempting and detectable.
///
/// What 4.3 changed that this template depends on:
///
/// - constraints live in **one `constraints` array** tagged by `"type"`, not in
///   separate `ik`/`transform`/`path` blocks;
/// - bones carry `inherit`, not `transform`;
/// - slot colour timelines are `rgba`, and bone rotate keys use `value`;
/// - a bezier key's `curve` is four (or eight, for two-axis channels) numbers in
///   **absolute** time/value space. Ankhimate stores handles normalized across
///   the span to the next key, so the context computes the absolute points —
///   `context::control_points`. A template cannot: it cannot see the next key.
///
/// Defaults are omitted aggressively, which is what makes the output diffable
/// against a hand-authored file rather than a wall of zeroes.
///
/// The export is **partial by design** — see `docs/export-spine.md` for what
/// does not cross and why. A format that silently drops half a rig is worse
/// than one that says which half.
pub const SPINE_JSON: &str = include_str!("spine_json.json");

/// Every preset shipped with the editor.
///
/// # Why there is no Godot preset here
///
/// Godot's `SpriteFrames` on-disk `.tres` layout is not published: the class
/// reference documents a scripting API of methods, and the only official
/// tutorial covering it is entirely GUI-driven. Shipping one would mean guessing
/// at a private serialization and calling it support — a preset that half-works
/// is worse than none, because the user cannot tell which half.
///
/// The path to adding it is empirical and clean-room: have Godot itself save a
/// `.tres`, observe the output, and write a template from that. Until someone
/// does that, a user targeting Godot has the same tool we would have used.
pub fn builtin() -> Vec<Preset> {
    [ANKHIMATE_RUNTIME, GENERIC_JSON, PHASER_ATLAS, SPINE_JSON]
        .iter()
        .filter_map(|text| Preset::from_json(text).ok())
        .collect()
}

/// The preset a "new export" starts from.
pub fn default_preset() -> Preset {
    Preset::from_json(ANKHIMATE_RUNTIME).expect("the shipped runtime preset parses")
}
