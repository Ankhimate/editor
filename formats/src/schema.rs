//! The on-disk `project.json` schema (PLAN §6.1, ADR 0004).
//!
//! Every entity is keyed by **name**, never by slotmap key: keys are not stable
//! across sessions or crate versions, and a serialized slotmap embeds internal
//! state that breaks forward compatibility (defect D8).
//!
//! Angle units on disk are **degrees** (PLAN §2.7) — human-editable and matching
//! the animation key convention. Conversion to core's radians happens in
//! [`crate::convert`].
//!
//! Unknown fields are captured into `extra` maps so a file written by a newer
//! version survives a round-trip through an older one rather than silently losing
//! data.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Catch-all for fields this version does not know about.
pub type Extra = BTreeMap<String, serde_json::Value>;

/// The current schema version. Bump on any breaking change and add a migration.
pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub fps: u32,
    /// Image library (T-301). The pixels live in the container under
    /// `images/<file>`; this is the index that names them.
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub bones: Vec<Bone>,
    #[serde(default)]
    pub slots: Vec<Slot>,
    /// Setup draw order, as slot names.
    #[serde(default)]
    pub draw_order: Vec<String>,
    #[serde(default)]
    pub skins: Vec<Skin>,
    /// Name of the default skin. Empty means "the first skin".
    #[serde(default)]
    pub default_skin: String,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// Constraint application order, as constraint names.
    #[serde(default)]
    pub constraint_order: Vec<String>,
    #[serde(default)]
    pub animations: Vec<Animation>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// One entry in the image library. `Region.texture` / `Mesh.texture` reference
/// an asset by `name`; `file` says where its bytes sit inside the container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    /// Path relative to `images/` inside the `.ankh` zip, e.g. `arm.png`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    /// Absolute path this asset was imported from, when known — for "reload from
    /// source" (T-306). Machine-specific, so it is advisory only.
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    /// Parent bone name; empty for a root bone.
    #[serde(default)]
    pub parent: String,
    pub length: f32,
    #[serde(default)]
    pub tx: f32,
    #[serde(default)]
    pub ty: f32,
    /// Degrees, CCW positive.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub sx: f32,
    #[serde(default = "one")]
    pub sy: f32,
    /// Shear in degrees.
    #[serde(default)]
    pub shear_x: f32,
    #[serde(default)]
    pub shear_y: f32,
    #[serde(default = "yes")]
    pub inherit_rotation: bool,
    #[serde(default = "yes")]
    pub inherit_scale: bool,
    #[serde(default = "yes")]
    pub inherit_reflect: bool,
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    /// Bone name this slot hangs from.
    pub bone: String,
    /// Attachment **name** the slot shows, resolved through the active skin.
    #[serde(default)]
    pub attachment: Option<String>,
    #[serde(default = "white")]
    pub color: [f32; 4],
    #[serde(default)]
    pub dark_color: Option<[f32; 4]>,
    #[serde(default)]
    pub blend_mode: String,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skin {
    pub name: String,
    #[serde(default)]
    pub entries: Vec<SkinEntry>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkinEntry {
    /// Slot name this attachment belongs to.
    pub slot: String,
    /// Attachment name a slot or timeline refers to.
    pub name: String,
    pub attachment: Attachment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Attachment {
    Region(Region),
    Mesh(Mesh),
    Clipping(Clipping),
}

/// A masking polygon (T-405).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clipping {
    /// Flattened `[x, y, x, y, …]`, like `Mesh::vertices`.
    #[serde(default)]
    pub vertices: Vec<f32>,
    /// Slot name the clip stops at, inclusive; absent clips to the end.
    #[serde(default)]
    pub end_slot: Option<String>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub texture: String,
    #[serde(default)]
    pub offset_x: f32,
    #[serde(default)]
    pub offset_y: f32,
    /// Degrees.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub scale_x: f32,
    #[serde(default = "one")]
    pub scale_y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub uv: [f32; 4],
    /// Pivot in normalized image coordinates, `(0,0)` bottom-left. Defaults to
    /// the centre so files written before pivots load unchanged.
    #[serde(default = "half")]
    pub pivot_x: f32,
    #[serde(default = "half")]
    pub pivot_y: f32,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub texture: String,
    /// Flattened `[x, y, x, y, …]` — compact and stable in JSON.
    #[serde(default)]
    pub vertices: Vec<f32>,
    #[serde(default)]
    pub uvs: Vec<f32>,
    #[serde(default)]
    pub triangles: Vec<u32>,
    /// Flattened vertex-index pairs the triangulation must preserve (T-401).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<u32>,
    /// Per-vertex weights: `[(bone_name, weight), …]` per vertex.
    #[serde(default)]
    pub weights: Vec<Vec<(String, f32)>>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// Target bone name.
    pub target: String,
    /// Chain bone names, root first.
    #[serde(default)]
    pub bones: Vec<String>,
    #[serde(default = "one")]
    pub bend_direction: f32,
    #[serde(default = "one")]
    pub mix: f32,
    #[serde(default)]
    pub softness: f32,
    #[serde(default)]
    pub stretch: bool,
    /// Most a stretching chain may grow, as a factor of its natural length.
    #[serde(default = "stretch_limit_default")]
    pub stretch_limit: f32,

    // ── Transform constraints (T-501) ────────────────────────────────────
    // Defaulted and skipped when empty, so an IK constraint's JSON is
    // unchanged by their existence.
    /// `[rotate, translate, scale, shear]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixes: Option<[f32; 4]>,
    /// `[x, y, rotation_degrees, scale_x, scale_y, shear_x_deg, shear_y_deg]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offsets: Option<[f32; 7]>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub relative: bool,

    // ── Physics constraints (T-503) ──────────────────────────────────────
    /// `[inertia, strength, damping, mass]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physics: Option<[f32; 4]>,
    /// `[wind_x, wind_y, gravity_x, gravity_y]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forces: Option<[f32; 4]>,
    /// `[rotate, translate]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<[bool; 2]>,

    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animation {
    pub name: String,
    /// Seconds.
    pub duration: f32,
    /// Authoring intent for the runtime; defaults to looping so pre-T-208 files
    /// keep the behavior the editor already gave them.
    #[serde(default = "yes")]
    pub looping: bool,
    #[serde(default)]
    pub timelines: Vec<Timeline>,
    /// Named triggers for the runtime (T-506).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// A named trigger at a point in a clip (T-506).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub time: f32,
    pub name: String,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub int_value: i32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub float_value: f32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub string_value: String,
}

fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

/// A timeline, tagged by kind with its target named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Timeline {
    BoneTranslate {
        bone: String,
        keys: Vec<Vec2Key>,
    },
    /// Degrees.
    BoneRotate {
        bone: String,
        keys: Vec<ScalarKey>,
    },
    BoneScale {
        bone: String,
        keys: Vec<Vec2Key>,
    },
    /// Degrees.
    BoneShear {
        bone: String,
        keys: Vec<Vec2Key>,
    },
    SlotColor {
        slot: String,
        keys: Vec<ColorKey>,
    },
    /// Stepped visibility (T-505).
    SlotVisible {
        slot: String,
        keys: Vec<VisibleKey>,
    },
    SlotAttachment {
        slot: String,
        keys: Vec<AttachmentKey>,
    },
    DrawOrder {
        keys: Vec<DrawOrderKey>,
    },
    IkMix {
        constraint: String,
        keys: Vec<ScalarKey>,
    },
    /// `+1` / `-1`, stepped (T-504).
    IkBendDirection {
        constraint: String,
        keys: Vec<ScalarKey>,
    },
    /// World units (T-504).
    IkSoftness {
        constraint: String,
        keys: Vec<ScalarKey>,
    },
    /// `[rotate, translate, scale, shear]` per key (T-501).
    TransformConstraintMix {
        constraint: String,
        keys: Vec<ColorKey>,
    },
    Deform {
        slot: String,
        attachment: String,
        keys: Vec<DeformKey>,
    },
}

/// How a key is approached from the previous one.
///
/// Serialized as a tagged enum so `bezier` can carry its handles and the two
/// simple cases stay one short string.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "curve", rename_all = "lowercase")]
pub enum Interp {
    #[default]
    Linear,
    Stepped,
    Bezier {
        /// `[out_x, out_y, in_x, in_y]` in normalized 0..1 time/value space.
        handles: [f32; 4],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScalarKey {
    pub time: f32,
    pub value: f32,
    #[serde(default, flatten)]
    pub interp: Interp,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2Key {
    pub time: f32,
    pub x: f32,
    pub y: f32,
    #[serde(default, flatten)]
    pub interp: Interp,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorKey {
    pub time: f32,
    pub value: [f32; 4],
    #[serde(default, flatten)]
    pub interp: Interp,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VisibleKey {
    pub time: f32,
    pub value: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentKey {
    pub time: f32,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawOrderKey {
    pub time: f32,
    /// `(slot_name, offset_from_setup)`.
    pub offsets: Vec<(String, i32)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeformKey {
    pub time: f32,
    /// Flattened `[x, y, x, y, …]` vertex offsets.
    pub offsets: Vec<f32>,
    #[serde(default, flatten)]
    pub interp: Interp,
}

fn stretch_limit_default() -> f32 {
    1.1
}

fn one() -> f32 {
    1.0
}

fn half() -> f32 {
    0.5
}

fn yes() -> bool {
    true
}

fn white() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
