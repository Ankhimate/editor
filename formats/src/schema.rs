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
    /// Folders the hierarchy is filed into. Organisation, not rig structure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    /// Export presets (T-603) — a rig's export settings belong to the rig.
    ///
    /// Held as opaque JSON rather than a typed field: the preset type lives in
    /// `ankhimate-export`, which depends on this crate, so naming it here would
    /// be a dependency cycle. Nothing in `formats` needs to understand a preset
    /// to round-trip one, and leaving it unparsed means a preset written by a
    /// newer editor survives an older one intact — the rule `Extra` exists for,
    /// applied to a field that has earned a name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub export_presets: Vec<serde_json::Value>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// A folder in the hierarchy, by name.
///
/// Members are written as `kind:name` — `bone:front-shin`, `slot:boot` — one
/// flat list rather than a list per kind. A group is a folder, and a folder does
/// not sort its contents by type; splitting them would also make "is this thing
/// grouped" two lookups instead of one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default = "group_color_default")]
    pub color: [f32; 4],
    #[serde(default)]
    pub members: Vec<String>,
    /// Name of the enclosing folder, if any.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent: String,
}

fn group_color_default() -> [f32; 4] {
    [0.55, 0.58, 0.65, 1.0]
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
    /// Bone names this skin brings with it; skipped while the skin is off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bones: Vec<String>,
    /// Constraint names that only apply while this skin is worn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
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
    Path(Path),
    BoundingBox(BoundingBox),
    Point(Point),
}

/// A hit-test polygon, optionally skinned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// Flattened `[x, y, x, y, …]`, like `Mesh::vertices`.
    #[serde(default)]
    pub vertices: Vec<f32>,
    /// Per-vertex weights: `[(bone_name, weight), …]` per vertex. Empty means
    /// the polygon is rigid to its slot's bone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<Vec<(String, f32)>>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// A named anchor with an orientation. Draws nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    /// Degrees, like every other authored angle in the file.
    #[serde(default)]
    pub rotation: f32,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// Frames an attachment cycles through, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Sequence {
    /// Texture names, in play order.
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default)]
    pub fps: f32,
    /// `hold`, `once`, `loop`, `ping_pong`, or any of those with `_reverse`.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub setup_index: u32,
}

/// Where a linked mesh borrows its geometry from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkedMesh {
    /// Skin holding the source; absent means the default skin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<String>,
    pub slot: String,
    pub attachment: String,
    #[serde(default = "yes")]
    pub inherit_deform: bool,
}

/// A curve bones can be driven along (T-502).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Path {
    /// Flattened `[x, y, x, y, …]`, like `Mesh::vertices`.
    #[serde(default)]
    pub vertices: Vec<f32>,
    #[serde(default)]
    pub closed: bool,
    #[serde(default = "yes")]
    pub constant_speed: bool,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
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
    /// Geometry borrowed from another mesh; its own vertices are then ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked: Option<LinkedMesh>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
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
    /// How much a 3+ bone chain keeps its current pose instead of redistributing
    /// its bend. Defaults to 0, which is how every rig written before this field
    /// existed was solved, so an older file still looks the same.
    #[serde(default)]
    pub stiffness: f32,

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

    // ── Path constraints (T-502) ─────────────────────────────────────────
    /// Slot name whose attachment is the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// `[position, spacing, mix_rotate, mix_translate]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<[f32; 4]>,

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
    /// Editor-only ruler labels (T-906). Written but never exported to a
    /// runtime — see [`Marker`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
    /// Per-bone sampling offsets in seconds (T-905), by bone name.
    ///
    /// Runtime data, unlike markers: a clip whose scarf trails four frames
    /// behind has to trail in the game too, or the export does not match what
    /// was authored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bone_offsets: Vec<BoneOffset>,
    #[serde(flatten)]
    #[serde(default)]
    pub extra: Extra,
}

/// One bone's sampling offset within a clip (T-905).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneOffset {
    pub bone: String,
    /// Seconds. Positive trails, negative leads.
    pub offset: f32,
}

/// A label on the timeline ruler (T-906).
///
/// Editor furniture, not runtime data: it is saved into the project so an
/// animator's notes survive a reopen, and it is exported by nothing. The
/// distinction from [`Event`] is deliberate — one fires into the game, the other
/// never leaves the tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub time: f32,
    pub name: String,
    /// RGBA. Defaulted so a marker written without one still loads.
    #[serde(default = "marker_color_default")]
    pub color: [f32; 4],
}

fn marker_color_default() -> [f32; 4] {
    [0.95, 0.72, 0.30, 1.0]
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
    /// Asset name of a sound to play with the event.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub audio: String,
    #[serde(default = "one", skip_serializing_if = "is_one_f32")]
    pub volume: f32,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub balance: f32,
}

fn is_one_f32(v: &f32) -> bool {
    *v == 1.0
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
        /// `[out_x, out_y, in_x, in_y]` as fractions of the span to the next
        /// key. The `x` pair is in 0..1; the `y` pair is unbounded, and a value
        /// outside 0..1 is an overshoot the sampler reproduces deliberately.
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
