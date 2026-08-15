//! Reading Spine JSON (`.json` + `.atlas` + page images) into our model.
//!
//! # Import is not the mirror of export
//!
//! Export writes a foreign format from data we own: every field has a value,
//! and the only question is where to put it. Import runs the other way and has
//! to *decide* — a source may carry a concept this model does not have, or
//! carry one it has at a different resolution. Those decisions belong in
//! [`LoadReport::lossy`], not in silence: a rig that plays back subtly wrong is
//! far more expensive to diagnose than one that arrives with a list of what it
//! could not keep.
//!
//! # What this reads
//!
//! Spine 3.8 through 4.3. The two differ in where constraints live (one tagged
//! array in 4.x, four typed arrays before that) and in a handful of field
//! names; both shapes are accepted, because a user with a 3.8 export has no way
//! to produce a 4.x one without buying the newer editor.
//!
//! Clean-room (PLAN §0): written against the published JSON format and against
//! files, never against Spine's runtime source, which is proprietary.

use crate::convert::{LoadReport, Loaded};
use ankhimate_core::animation::Interp;
use ankhimate_core::animation::{Animation, EventKey, Key, Timeline};
use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_core::attachment::{
    Attachment, BoundingBoxAttachment, ClippingAttachment, MeshAttachment, PointAttachment, Rect,
    RegionAttachment,
};
use ankhimate_core::constraints::{Constraint, IkConstraint, TransformConstraint};
use ankhimate_core::ids::{AnimationId, BoneId, ConstraintId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slot::Slot;
use ankhimate_core::slotmap::SlotMap;
use ankhimate_core::transforms::Inherit;
use serde_json::Value;
use std::collections::HashMap;

/// What went wrong badly enough that there is no rig to return.
#[derive(Debug)]
pub enum Error {
    /// The skeleton JSON did not parse.
    Json(String),
    /// The file parsed but carries no `bones` array, so it is not a skeleton.
    NotASkeleton,
    /// An `.atlas` was referenced but could not be read.
    Atlas(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Json(why) => write!(f, "the skeleton JSON did not parse: {why}"),
            Error::NotASkeleton => write!(f, "no `bones` array — this is not a Spine skeleton"),
            Error::Atlas(why) => write!(f, "the atlas could not be read: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The images an import draws attachments from.
///
/// Spine rigs ship either as a packed atlas plus its page images, or as loose
/// PNGs in an `images/` directory. Both are common; a rig exported for a runtime
/// has an atlas, one exported for re-editing usually does not.
pub enum Images<'a> {
    /// A parsed `.atlas` and its decoded pages, keyed by page filename.
    Atlas {
        text: &'a str,
        pages: &'a dyn Fn(&str) -> Option<image::RgbaImage>,
    },
    /// Loose images, looked up by attachment path.
    Loose(&'a dyn Fn(&str) -> Option<image::RgbaImage>),
    /// No images available: geometry imports, attachments reference names that
    /// resolve to nothing, and every one is reported as dangling.
    None,
}

/// Read a Spine skeleton into our model.
///
/// `name` is what the rig will be called — usually the file stem, since Spine
/// does not store a project name.
pub fn read(json: &str, images: Images<'_>, name: &str) -> Result<Loaded, Error> {
    let doc: Value = serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))?;
    if !doc.get("bones").is_some_and(|b| b.is_array()) {
        return Err(Error::NotASkeleton);
    }
    let mut report = LoadReport::default();
    Ok(convert(&doc, images, name, &mut report))
}

/// The version string a file declares, when it has one.
///
/// Only for reporting: the reader accepts what it recognises regardless, since
/// a version it has never heard of is more likely a newer minor release than an
/// incompatible format.
pub fn declared_version(json: &str) -> Option<String> {
    let doc: Value = serde_json::from_str(json).ok()?;
    doc.get("skeleton")?
        .get("spine")?
        .as_str()
        .map(str::to_string)
}

/// Spine's easing for one segment, as our normalized handles.
///
/// Two shifts of frame. Spine hangs a curve on the key that *starts* a segment
/// and we hang it on the key that *ends* one, so a source key's curve becomes
/// the following key's `interp`. And Spine 4.x writes control points in
/// absolute time/value while ours are fractions of the span — hence the
/// divides.
///
/// `channel` picks the pair for a multi-value timeline: translate writes eight
/// numbers, x's four then y's four.
///
/// The two axes are bounded differently, and only one of them is clamped.
///
/// **Value passes through.** A Spine curve may swing past its own endpoints —
/// the wind-up before a punch, the settle after it — and this model represents
/// that: `ease()` feeds the value handle to a plain cubic, and every consumer
/// lerps unclamped, so a fraction outside 0..1 extrapolates. Clamping it here
/// flattened every anticipation in an imported rig.
///
/// **Time is clamped to the segment.** `solve_bezier_x` inverts `x(t)` by
/// bisecting `0..1` and assumes `x(t)` is monotonic; a time handle outside the
/// span makes the curve double back, which is not a function of `t` and cannot
/// be sampled at all. The editor's own handle drag clamps time for the same
/// reason.
///
/// The returned flag says a **time** handle was clamped, so the caller can
/// report the one loss that is real.
pub(crate) fn curve_interp(
    curve: Option<&Value>,
    t0: f32,
    v0: f32,
    t1: f32,
    v1: f32,
    channel: usize,
) -> (Interp, bool) {
    match curve {
        Some(Value::String(kind)) if kind == "stepped" => (Interp::Stepped, false),
        Some(Value::Array(handles)) => {
            let at = |i: usize| {
                handles
                    .get(channel * 4 + i)
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.0) as f32
            };
            let (dt, dv) = (t1 - t0, v1 - v0);
            // A segment that holds one value has no vertical extent to
            // normalize against; its value handles are meaningless either way.
            let nx = |x: f32| if dt.abs() < 1e-6 { 0.0 } else { (x - t0) / dt };
            let ny = |y: f32| if dv.abs() < 1e-6 { 0.0 } else { (y - v0) / dv };
            let (out_t, in_t) = (nx(at(0)), nx(at(2)));
            let (out_v, in_v) = (ny(at(1)), ny(at(3)));
            let (out_x, in_x) = (out_t.clamp(0.0, 1.0), in_t.clamp(0.0, 1.0));
            let lost = (out_t - out_x).abs() > 1e-4 || (in_t - in_x).abs() > 1e-4;
            // A non-finite control point would otherwise reach the sampler and
            // take the whole track with it. `clamp` used to absorb these; with
            // the value axis free, nothing else would.
            let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
            (
                Interp::Bezier {
                    out_handle: glam::vec2(out_x, finite(out_v)),
                    in_handle: glam::vec2(in_x, finite(in_v)),
                },
                lost,
            )
        }
        _ => (Interp::Linear, false),
    }
}

/// Build a timeline's keys, deriving each key's easing from the frame before it.
///
/// `value` reads the key's value; `scalar` reads the one number the curve is
/// normalized against — for a two-channel timeline, whichever channel `channel`
/// selects.
///
/// `where_` names the track for the report, in the source's own terms
/// (`"walk/hip/translate"`), so a clamped handle can be found again.
fn keys_with_curves<T>(
    frames: &[Value],
    channel: usize,
    value: impl Fn(&Value) -> T,
    scalar: impl Fn(&Value) -> f32,
    where_: &str,
    report: &mut LoadReport,
) -> Vec<Key<T>> {
    keys_with_curves_on(frames, channel, value, scalar, None, where_, report)
}

/// How to read a two-axis timeline's other channel: its index, and a reader.
type AltChannel<'a> = (usize, &'a dyn Fn(&Value) -> f32);

/// [`keys_with_curves`], with a fallback channel for a two-axis timeline.
///
/// One `Key` holds one easing, so a two-axis timeline has to pick an axis to
/// normalize against. X is the natural choice — an animator reaching for a
/// curve usually shapes the dominant axis — but a track where x never moves has
/// no span to normalize against, and the handles collapse to zero. The track
/// then imports as linear on *both* axes while the source eased on y.
///
/// `alt` names the other channel and how to read it. It is used per segment,
/// only where the primary axis is flat, so a normal track is unaffected.
#[allow(clippy::too_many_arguments)]
fn keys_with_curves_on<T>(
    frames: &[Value],
    channel: usize,
    value: impl Fn(&Value) -> T,
    scalar: impl Fn(&Value) -> f32,
    alt: Option<AltChannel<'_>>,
    where_: &str,
    report: &mut LoadReport,
) -> Vec<Key<T>> {
    let mut clamped = 0usize;
    let mut split = 0usize;
    let keys: Vec<Key<T>> = frames
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let time = f(k, "time", 0.0);
            let interp = match i.checked_sub(1).map(|j| &frames[j]) {
                None => Interp::Linear,
                Some(prev) => {
                    // Follow the axis that actually moves over this segment.
                    let (ch, from, to) = match alt {
                        Some((other, read)) if (scalar(k) - scalar(prev)).abs() < 1e-6 => {
                            (other, read(prev), read(k))
                        }
                        _ => (channel, scalar(prev), scalar(k)),
                    };
                    // Both axes moving means the source's two curves cannot
                    // both survive; note it when they actually differ.
                    if let Some((other, read)) = alt
                        && (scalar(k) - scalar(prev)).abs() > 1e-6
                        && (read(k) - read(prev)).abs() > 1e-6
                        && let Some(Value::Array(h)) = prev.get("curve")
                        && h.len() > other * 4 + 3
                    {
                        let a = &h[channel * 4..channel * 4 + 4];
                        let b = &h[other * 4..other * 4 + 4];
                        // Compare the *time* control points: identical timing
                        // means one easing describes both axes exactly.
                        let differs = [0usize, 2].iter().any(|&i| {
                            let (x, y) =
                                (a[i].as_f64().unwrap_or(0.0), b[i].as_f64().unwrap_or(0.0));
                            (x - y).abs() > 1e-4
                        });
                        split += usize::from(differs);
                    }
                    let (interp, lost) =
                        curve_interp(prev.get("curve"), f(prev, "time", 0.0), from, time, to, ch);
                    clamped += usize::from(lost);
                    interp
                }
            };
            Key {
                time,
                value: value(k),
                interp,
            }
        })
        .collect();
    if clamped > 0 {
        report.lossy(
            "curve",
            where_.to_string(),
            format!(
                "{clamped} handle(s) reached outside the segment in time and were \
                 clamped to it; an easing that doubles back in time cannot be sampled"
            ),
        );
    }
    if split > 0 {
        report.lossy(
            "curve",
            where_.to_string(),
            format!(
                "{split} segment(s) eased each axis differently; a key holds one \
                 easing here, so both axes follow the first axis's curve"
            ),
        );
    }
    keys
}

/// One packed region in a `.atlas` page.
struct AtlasRegion {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// The page stores it turned 90°, so the crop has to be turned back.
    rotated: bool,
}

/// Parse the `.atlas` text format: a page name, indented page properties, then
/// region names each followed by their own indented properties.
fn parse_atlas(text: &str) -> (String, HashMap<String, AtlasRegion>) {
    let mut page = String::new();
    let mut regions = HashMap::new();
    let mut current: Option<String> = None;
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    let mut rotated = false;

    let flush = |regions: &mut HashMap<String, AtlasRegion>,
                 name: &Option<String>,
                 bounds: &Option<(u32, u32, u32, u32)>,
                 rotated: bool| {
        if let (Some(name), Some((x, y, w, h))) = (name, bounds) {
            regions.insert(
                name.clone(),
                AtlasRegion {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    rotated,
                },
            );
        }
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indented = line.starts_with('\t') || line.starts_with("  ");
        if !indented {
            // A bare line is either the page image or a region name.
            if page.is_empty() && trimmed.ends_with(".png") {
                page = trimmed.to_string();
                continue;
            }
            flush(&mut regions, &current, &bounds, rotated);
            current = Some(trimmed.to_string());
            bounds = None;
            rotated = false;
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let nums: Vec<i64> = value
            .split(',')
            .filter_map(|v| v.trim().parse::<i64>().ok())
            .collect();
        match key.trim() {
            "bounds" if nums.len() == 4 => {
                bounds = Some((
                    nums[0] as u32,
                    nums[1] as u32,
                    nums[2] as u32,
                    nums[3] as u32,
                ))
            }
            // 4.x writes `rotate: 90`; older files wrote `rotate: true`.
            "rotate" => rotated = value.trim() == "90" || value.trim() == "true",
            _ => {}
        }
    }
    flush(&mut regions, &current, &bounds, rotated);
    (page, regions)
}

/// Spine's bone colours are `RRGGBBAA` hex.
fn hex_rgba(hex: &str) -> Option<[f32; 4]> {
    if hex.len() != 8 {
        return None;
    }
    let byte = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?])
}

/// How a bone takes its parent's transform. Dropping this left the toe bones
/// inheriting the foot's rotation, which dragged the boot tips a good 45 units
/// through the floor.
fn inherit_mode(mode: Option<&str>) -> Inherit {
    match mode.unwrap_or("normal") {
        "onlyTranslation" => Inherit {
            rotation: false,
            scale: false,
            reflect: false,
        },
        "noRotationOrReflection" => Inherit {
            rotation: false,
            scale: true,
            reflect: false,
        },
        "noScale" => Inherit {
            rotation: true,
            scale: false,
            reflect: true,
        },
        "noScaleOrReflection" => Inherit {
            rotation: true,
            scale: false,
            reflect: false,
        },
        _ => Inherit::default(),
    }
}

/// Spine writes slot colours as `RRGGBBAA` hex.
fn hex_color(hex: &str) -> [f32; 4] {
    hex_rgba(hex).unwrap_or([1.0, 1.0, 1.0, 1.0])
}

fn f(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key)
        .and_then(|x| x.as_f64())
        .unwrap_or(default as f64) as f32
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

fn floats(v: &Value, key: &str) -> Vec<f32> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_f64())
                .map(|n| n as f32)
                .collect()
        })
        .unwrap_or_default()
}

/// The conversion proper: a parsed document and an image source in, a rig out.
///
/// Split from [`read`] so the parse-and-validate step stays readable, and so a
/// caller that already holds a `Value` — a plugin host, a test — does not pay
/// to re-serialize it.
fn convert(doc: &Value, images: Images<'_>, name: &str, report: &mut LoadReport) -> Loaded {
    let mut skel = Skeleton::new();
    let mut assets = AssetDb::new();

    // Only the atlas case has a region index; the others look images up by
    // name, so an empty map is the right answer rather than a special case
    // threaded through every attachment.
    let (page_file, regions) = match &images {
        Images::Atlas { text, .. } => parse_atlas(text),
        _ => (String::new(), HashMap::new()),
    };

    // ── Bones ────────────────────────────────────────────────────────────
    // Spine's model is the one ours was designed against, so this is a direct
    // mapping: degrees to radians, and y-up in both.
    let mut ids: HashMap<String, BoneId> = HashMap::new();
    let empty = Vec::new();
    let bones = doc["bones"].as_array().unwrap_or(&empty);
    for b in bones {
        let name = s(b, "name").unwrap_or("bone").to_string();
        let id = skel.add_bone(Bone {
            name: name.clone(),
            parent: None,
            length: f(b, "length", 0.0).max(1.0),
            local_transform: Transform {
                position: glam::vec2(f(b, "x", 0.0), f(b, "y", 0.0)),
                rotation: f(b, "rotation", 0.0).to_radians(),
                scale: glam::vec2(f(b, "scaleX", 1.0), f(b, "scaleY", 1.0)),
                shear: glam::vec2(
                    f(b, "shearX", 0.0).to_radians(),
                    f(b, "shearY", 0.0).to_radians(),
                ),
            },
            inherit: inherit_mode(s(b, "inherit")),
            color: s(b, "color")
                .and_then(hex_rgba)
                .unwrap_or_else(Bone::default_color),
        });
        ids.insert(name, id);
    }
    for b in bones {
        let (Some(name), Some(parent)) = (s(b, "name"), s(b, "parent")) else {
            continue;
        };
        if let (Some(&child), Some(&parent)) = (ids.get(name), ids.get(parent))
            && let Some(bone) = skel.bones.get_mut(child)
        {
            bone.parent = Some(parent);
        }
    }
    skel.rebuild_update_order();

    // ── Bone colours ────────────────────────────────────────────────────
    // Most bones carry a colour in the file and it already reads as one hue per
    // limb. The rest are default grey, which leaves the tree a wall of identical
    // glyphs, so colour the root of each remaining group and let inheritance
    // carry it down.
    {
        let groups: &[(&str, [f32; 4])] = &[
            ("hip", [0.75, 0.55, 0.95, 0.9]),
            ("torso", [0.35, 0.65, 1.0, 0.9]),
            ("neck", [0.55, 0.80, 1.0, 0.9]),
            ("gun", [0.85, 0.75, 0.30, 0.9]),
            ("hoverboard-controller", [0.60, 0.60, 0.70, 0.9]),
        ];
        for (name, color) in groups {
            if let Some(&id) = ids.get(*name)
                && let Some(bone) = skel.bones.get_mut(id)
                && bone.color == Bone::default_color()
            {
                bone.color = *color;
            }
        }
    }

    // ── Slots ────────────────────────────────────────────────────────────
    // Slot order *is* draw order in Spine, back to front — same as ours.
    let mut slots: HashMap<String, SlotId> = HashMap::new();
    for s_ in doc["slots"].as_array().unwrap_or(&empty) {
        let (Some(name), Some(bone)) = (s(s_, "name"), s(s_, "bone")) else {
            continue;
        };
        let Some(&bone_id) = ids.get(bone) else {
            continue;
        };
        let id = skel.add_slot(Slot {
            attachment: s(s_, "attachment").map(str::to_string),
            ..Slot::new(name.to_string(), bone_id)
        });
        // `add_slot` already appends to `draw_order`, and Spine's slot order is
        // draw order, so inserting again drew every piece twice — visible as
        // doubled anti-aliased edges rather than as anything obviously wrong.
        slots.insert(name.to_string(), id);
    }

    // ── Attachments ──────────────────────────────────────────────────────
    // Weighted meshes address bones by their index in the file's bone array,
    // and their vertices are expressed in each bone's own setup frame.
    let bone_order: Vec<BoneId> = bones
        .iter()
        .filter_map(|b| s(b, "name").and_then(|n| ids.get(n).copied()))
        .collect();
    let setup = {
        let mut pose = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&skel, &[], &mut pose);
        pose
    };
    let default_skin = skel.default_skin;
    // Name -> asset, decoding from whichever image source the caller gave.
    // The atlas case crops a page; the loose case takes a whole file. Both end
    // as an `ImageAsset`, so nothing downstream knows which it was.
    let mut decoded: HashMap<String, (u32, u32)> = HashMap::new();
    let mut crop = |name: &str, assets: &mut AssetDb| -> Option<(String, u32, u32)> {
        if let Some(&(w, h)) = decoded.get(name) {
            return Some((name.to_string(), w, h));
        }
        let piece = match &images {
            Images::Atlas { pages, .. } => {
                let r = regions.get(name)?;
                let page = pages(&page_file)?;
                // `bounds` is the region's *unrotated* size, so a rotated
                // region occupies a transposed rectangle in the page: crop
                // (h, w), then turn it back *clockwise*. Turning it the other
                // way stands the art up too, so the shape looks plausible while
                // the art sits 180 degrees out.
                if r.rotated {
                    let packed = image::imageops::crop_imm(&page, r.x, r.y, r.h, r.w).to_image();
                    image::imageops::rotate90(&packed)
                } else {
                    image::imageops::crop_imm(&page, r.x, r.y, r.w, r.h).to_image()
                }
            }
            Images::Loose(open) => open(name)?,
            Images::None => return None,
        };
        let (w, h) = (piece.width(), piece.height());
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(piece)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .ok()?;
        assets.add(ImageAsset::new(name.to_string(), bytes, w, h));
        decoded.insert(name.to_string(), (w, h));
        Some((name.to_string(), w, h))
    };

    if let Some(skin) = doc["skins"].as_array().and_then(|a| a.first())
        && let Some(entries) = skin["attachments"].as_object()
    {
        for (slot_name, attachments) in entries {
            let Some(&slot_id) = slots.get(slot_name) else {
                continue;
            };
            for (att_name, att) in attachments.as_object().into_iter().flatten() {
                // `path` names the atlas region when it differs from the
                // attachment's own name.
                let region_name = s(att, "path").unwrap_or(att_name);
                match att.get("type").and_then(|t| t.as_str()).unwrap_or("region") {
                    "region" => {
                        let Some((asset, _, _)) = crop(region_name, &mut assets) else {
                            report.dangling("spine region", region_name);
                            continue;
                        };
                        skel.skins[default_skin].set(
                            slot_id,
                            att_name.clone(),
                            Attachment::Region(RegionAttachment {
                                texture: asset,
                                local_offset: glam::vec2(f(att, "x", 0.0), f(att, "y", 0.0)),
                                local_rotation: f(att, "rotation", 0.0).to_radians(),
                                local_scale: glam::vec2(
                                    f(att, "scaleX", 1.0),
                                    f(att, "scaleY", 1.0),
                                ),
                                width: f(att, "width", 0.0),
                                height: f(att, "height", 0.0),
                                uv_rect: Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    w: 1.0,
                                    h: 1.0,
                                },
                                // Spine draws an attachment centred on its own
                                // origin, which is what a 0.5 pivot means here.
                                pivot: glam::Vec2::splat(0.5),
                                sequence: None,
                            }),
                        );
                    }
                    "mesh" => {
                        let raw = floats(att, "vertices");
                        let uvs = floats(att, "uvs");
                        // Two encodings share one field. Unweighted is a flat
                        // `[x, y, …]` in the slot bone's space. Weighted is, per
                        // vertex, `[bone_count, (bone, x, y, weight) × count]`
                        // with each position in *that bone's* space — so a
                        // vertex is the weighted sum of its bones' placements.
                        let weighted = raw.len() != uvs.len();
                        let (verts, weights) = if weighted {
                            let mut positions = Vec::new();
                            let mut per_vertex = Vec::new();
                            let mut i = 0usize;
                            while i < raw.len() {
                                let count = raw[i] as usize;
                                i += 1;
                                let mut point = glam::Vec2::ZERO;
                                let mut influences = Vec::new();
                                for _ in 0..count {
                                    if i + 3 > raw.len() {
                                        break;
                                    }
                                    let bone_index = raw[i] as usize;
                                    let local = glam::vec2(raw[i + 1], raw[i + 2]);
                                    let weight = raw[i + 3];
                                    i += 4;
                                    let Some(&bone) = bone_order.get(bone_index) else {
                                        continue;
                                    };
                                    // Bring the offset into world space through
                                    // that bone, which is what the weights mean.
                                    point += setup.world(bone).transform_point(local) * weight;
                                    influences.push(ankhimate_core::attachment::VertexWeight {
                                        bone,
                                        weight,
                                    });
                                }
                                positions.push(point);
                                per_vertex.push(influences);
                            }
                            (positions, per_vertex)
                        } else {
                            (
                                raw.chunks_exact(2)
                                    .map(|c| glam::vec2(c[0], c[1]))
                                    .collect(),
                                Vec::new(),
                            )
                        };
                        // A weighted mesh's vertices are world-space above; ours
                        // are in the slot bone's frame, so bring them back.
                        let verts: Vec<glam::Vec2> = if weighted {
                            let inverse = setup.world(skel.slots[slot_id].bone).invert();
                            verts
                                .into_iter()
                                .map(|v| inverse.map(|i| i.transform_point(v)).unwrap_or(v))
                                .collect()
                        } else {
                            verts
                        };
                        let Some((asset, _, _)) = crop(region_name, &mut assets) else {
                            report.dangling("spine region", region_name);
                            continue;
                        };
                        let triangles: Vec<[u32; 3]> = att["triangles"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|n| n.as_u64().map(|v| v as u32))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                            .chunks_exact(3)
                            .map(|c| [c[0], c[1], c[2]])
                            .collect();
                        skel.skins[default_skin].set(
                            slot_id,
                            att_name.clone(),
                            Attachment::Mesh(MeshAttachment {
                                texture: asset,
                                setup_vertices: verts,
                                uvs: uvs
                                    .chunks_exact(2)
                                    .map(|c| glam::vec2(c[0], c[1]))
                                    .collect(),
                                triangles,
                                weights,
                                ..MeshAttachment::default()
                            }),
                        );
                    }
                    "clipping" => {
                        skel.skins[default_skin].set(
                            slot_id,
                            att_name.clone(),
                            Attachment::Clipping(ClippingAttachment {
                                vertices: floats(att, "vertices")
                                    .chunks_exact(2)
                                    .map(|c| glam::vec2(c[0], c[1]))
                                    .collect(),
                                end_slot: s(att, "end").map(str::to_string),
                            }),
                        );
                    }
                    "boundingbox" => {
                        let flat = floats(att, "vertices");
                        let count =
                            att.get("vertexCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        // Same two layouts a mesh uses: `2 * vertexCount` floats
                        // means rigid, anything longer is weighted.
                        let vertices: Vec<glam::Vec2> = if flat.len() == count * 2 {
                            flat.chunks_exact(2)
                                .map(|c| glam::vec2(c[0], c[1]))
                                .collect()
                        } else {
                            // A weighted hitbox is flattened to its setup shape in
                            // the slot bone's frame. Our weighted vertices are
                            // authored in one bone's space and skinned from there;
                            // the file authors them per influence. Rather than
                            // guess a conversion, it comes in rigid and says so.
                            report.lossy(
                                "attachment",
                                att_name.clone(),
                                "weights on a bounding box are not kept",
                            );
                            let inverse = setup.world(skel.slots[slot_id].bone).invert();
                            let mut out = Vec::new();
                            let mut i = 0usize;
                            for _ in 0..count {
                                if i >= flat.len() {
                                    break;
                                }
                                let n = flat[i] as usize;
                                i += 1;
                                let mut point = glam::Vec2::ZERO;
                                for _ in 0..n {
                                    if i + 3 > flat.len() {
                                        break;
                                    }
                                    let bone_index = flat[i] as usize;
                                    let local = glam::vec2(flat[i + 1], flat[i + 2]);
                                    let weight = flat[i + 3];
                                    i += 4;
                                    if let Some(&bone) = bone_order.get(bone_index) {
                                        point += setup.world(bone).transform_point(local) * weight;
                                    }
                                }
                                out.push(
                                    inverse
                                        .map(|inv| inv.transform_point(point))
                                        .unwrap_or(point),
                                );
                            }
                            out
                        };
                        let weights = Vec::new();
                        skel.skins[default_skin].set(
                            slot_id,
                            att_name.clone(),
                            Attachment::BoundingBox(BoundingBoxAttachment { vertices, weights }),
                        );
                    }
                    "point" => {
                        skel.skins[default_skin].set(
                            slot_id,
                            att_name.clone(),
                            Attachment::Point(PointAttachment {
                                position: glam::vec2(f(att, "x", 0.0), f(att, "y", 0.0)),
                                rotation: f(att, "rotation", 0.0).to_radians(),
                            }),
                        );
                    }
                    other => report.lossy(
                        "attachment",
                        att_name.clone(),
                        format!("`{other}` attachments are not read"),
                    ),
                }
            }
        }
    }

    // ── Constraints ──────────────────────────────────────────────────────
    // Spine gives every constraint an explicit `order` that interleaves IK and
    // transform constraints, and it matters: the leg IK runs at 4-5 and the foot
    // IK at 6-7, so the feet are re-aimed *after* the legs that carry them.
    // Importing all the IK first and all the transforms second solved each foot
    // against a leg that had not moved yet, which showed up as boots at the
    // wrong angle. `constraint_order` is ours to fill, so fill it in their order.
    enum Pending<'a> {
        Ik(&'a Value),
        Transform(&'a Value),
    }
    let mut pending: Vec<(i64, Pending)> = Vec::new();
    let order_of = |v: &Value| v.get("order").and_then(|o| o.as_i64()).unwrap_or(0);

    // 4.x puts every constraint in one array tagged by `type`; 3.8 gave each
    // kind its own. Both are read, because a user with a 3.8 export cannot
    // produce a 4.x one without buying the newer editor — and a file that
    // silently imports with no constraints looks like a rig that never had any.
    for c in doc["constraints"].as_array().unwrap_or(&empty) {
        match c.get("type").and_then(|t| t.as_str()) {
            Some("ik") => pending.push((order_of(c), Pending::Ik(c))),
            Some("transform") => pending.push((order_of(c), Pending::Transform(c))),
            // Path and physics constraints have no equivalent here yet. Naming
            // them is the difference between "this rig had none" and "this rig
            // had some and they are gone".
            Some(other) => report.lossy(
                "constraint",
                s(c, "name").unwrap_or("?").to_string(),
                format!("`{other}` constraints are not read yet"),
            ),
            None => {}
        }
    }
    for ik in doc["ik"].as_array().unwrap_or(&empty) {
        pending.push((order_of(ik), Pending::Ik(ik)));
    }
    for tc in doc["transform"].as_array().unwrap_or(&empty) {
        pending.push((order_of(tc), Pending::Transform(tc)));
    }
    for kind in ["path", "physics"] {
        for c in doc[kind].as_array().unwrap_or(&empty) {
            report.lossy(
                "constraint",
                s(c, "name").unwrap_or("?").to_string(),
                format!("`{kind}` constraints are not read yet"),
            );
        }
    }
    // A stable sort, so two constraints sharing an `order` keep the order the
    // file listed them in. Constraints solve in sequence and the result differs.
    pending.sort_by_key(|(order, _)| *order);

    let bone_list = |v: &Value| -> Vec<BoneId> {
        v["bones"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| b.as_str())
                    .filter_map(|b| ids.get(b).copied())
                    .collect()
            })
            .unwrap_or_default()
    };

    let mut constraint_ids: HashMap<String, ConstraintId> = HashMap::new();
    for (_, item) in pending {
        match item {
            Pending::Ik(ik) => {
                let Some(&target) = s(ik, "target").and_then(|t| ids.get(t)) else {
                    continue;
                };
                let chain = bone_list(ik);
                if chain.is_empty() {
                    continue;
                }
                let ik_name = s(ik, "name").unwrap_or("ik").to_string();
                let cid = skel.add_constraint(Constraint::Ik(IkConstraint {
                    name: ik_name.clone(),
                    target,
                    bones: chain,
                    // `bendPositive` defaults to true, and their positive bend is
                    // counter-clockwise — the same sign our solver uses.
                    bend_direction: if ik.get("bendPositive").and_then(|b| b.as_bool())
                        == Some(false)
                    {
                        -1.0
                    } else {
                        1.0
                    },
                    mix: f(ik, "mix", 1.0),
                    softness: f(ik, "softness", 0.0),
                    stretch: ik.get("stretch").and_then(|b| b.as_bool()).unwrap_or(false),
                    stretch_limit: 1.1,
                    stiffness: 0.0,
                }));
                constraint_ids.insert(ik_name, cid);
            }
            Pending::Transform(tc) => {
                // 4.x names the followed bone `source`; 3.8 called it `target`.
                let named = s(tc, "source").or_else(|| s(tc, "target"));
                let Some(&target) = named.and_then(|t| ids.get(t)) else {
                    // Reported, not skipped in silence: a constraint that
                    // vanishes takes its effect on the pose with it, and the rig
                    // looks subtly wrong with nothing to point at. Reading 4.x
                    // files with only 3.8's field name dropped all seven of
                    // spineboy's transform constraints exactly this way.
                    report.dangling("transform constraint source", named.unwrap_or("(none)"));
                    continue;
                };
                let driven = bone_list(tc);
                if driven.is_empty() {
                    report.dangling("transform constraint bones", s(tc, "name").unwrap_or("?"));
                    continue;
                }
                // An omitted mix is **off**, not full.
                //
                // A file writes only the channels its constraint drives, so
                // defaulting the rest to 1 turns a constraint the artist
                // switched off into one that drags its bones along every axis
                // it never mentioned. Spineboy's four `aim-*` constraints say
                // `mixRotate: 0` and nothing else; read with 1.0 defaults they
                // pulled the torso, head and gun arm toward a crosshair parked
                // 645 units away, in every animation.
                //
                // Each axis is its own channel with its own default — `mixY`
                // does not inherit `mixX`. Spineboy's `shoulder` mirrors with
                // `mixX: -1` and no `mixY`, and reading that as "both axes at
                // -1" put a 19.5-unit vertical shift on the bone carrying the
                // rear arm and gun.
                //
                // Spine writes no `mixShearX`: shear's second axis has no mix
                // there. It reads as 0, which is what "not driven" means.
                let mix = ankhimate_core::constraints::TransformMix {
                    rotate: f(tc, "mixRotate", 0.0),
                    translate: glam::vec2(f(tc, "mixX", 0.0), f(tc, "mixY", 0.0)),
                    scale: glam::vec2(f(tc, "mixScaleX", 0.0), f(tc, "mixScaleY", 0.0)),
                    shear: glam::vec2(f(tc, "mixShearX", 0.0), f(tc, "mixShearY", 0.0)),
                };
                let tc_name = s(tc, "name").unwrap_or("transform").to_string();
                let cid = skel.add_constraint(Constraint::Transform(TransformConstraint {
                    name: tc_name.clone(),
                    target,
                    bones: driven,
                    offsets: Transform {
                        position: glam::vec2(f(tc, "x", 0.0), f(tc, "y", 0.0)),
                        rotation: f(tc, "rotation", 0.0).to_radians(),
                        scale: glam::vec2(1.0 + f(tc, "scaleX", 0.0), 1.0 + f(tc, "scaleY", 0.0)),
                        shear: glam::vec2(0.0, f(tc, "shearY", 0.0).to_radians()),
                    },
                    mix,
                    local: tc.get("local").and_then(|b| b.as_bool()).unwrap_or(false),
                    relative: tc
                        .get("relative")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false),
                }));
                constraint_ids.insert(tc_name, cid);
            }
        }
    }

    // ── Animations ───────────────────────────────────────────────────────
    // Spine keys bone properties as offsets from setup, exactly like ours, so
    // the values carry across untouched.
    let mut animations: SlotMap<AnimationId, Animation> = SlotMap::with_key();
    for (name, anim) in doc["animations"].as_object().into_iter().flatten() {
        let mut timelines = Vec::new();
        let mut duration: f32 = 0.0;
        let note = |t: f32, duration: &mut f32| *duration = duration.max(t);

        for (bone_name, tracks) in anim["bones"].as_object().into_iter().flatten() {
            let Some(&bone) = ids.get(bone_name) else {
                continue;
            };
            for (kind, frames) in tracks.as_object().into_iter().flatten() {
                let Some(frames) = frames.as_array() else {
                    continue;
                };
                let track = format!("{name}/{bone_name}/{kind}");
                let times: Vec<f32> = frames.iter().map(|k| f(k, "time", 0.0)).collect();
                for t in &times {
                    note(*t, &mut duration);
                }
                match kind.as_str() {
                    "rotate" => timelines.push(Timeline::BoneRotate {
                        bone,
                        keys: keys_with_curves(
                            frames,
                            0,
                            |k| f(k, "value", 0.0),
                            |k| f(k, "value", 0.0),
                            &track,
                            report,
                        ),
                    }),
                    // A two-channel timeline carries a curve per channel, but
                    // one `Key` holds one easing. X is the one an animator
                    // shapes; Y follows it.
                    "translate" => timelines.push(Timeline::BoneTranslate {
                        bone,
                        keys: keys_with_curves_on(
                            frames,
                            0,
                            |k| glam::vec2(f(k, "x", 0.0), f(k, "y", 0.0)),
                            |k| f(k, "x", 0.0),
                            Some((1, &|k| f(k, "y", 0.0))),
                            &track,
                            report,
                        ),
                    }),
                    "scale" => timelines.push(Timeline::BoneScale {
                        bone,
                        keys: keys_with_curves_on(
                            frames,
                            0,
                            |k| glam::vec2(f(k, "x", 1.0), f(k, "y", 1.0)),
                            |k| f(k, "x", 1.0),
                            Some((1, &|k| f(k, "y", 1.0))),
                            &track,
                            report,
                        ),
                    }),
                    "shear" => timelines.push(Timeline::BoneShear {
                        bone,
                        keys: keys_with_curves_on(
                            frames,
                            0,
                            |k| glam::vec2(f(k, "x", 0.0), f(k, "y", 0.0)),
                            |k| f(k, "x", 0.0),
                            Some((1, &|k| f(k, "y", 0.0))),
                            &track,
                            report,
                        ),
                    }),
                    _ => {}
                }
            }
        }

        // ── Slot timelines ───────────────────────────────────────────────
        for (slot_name, tracks) in anim["slots"].as_object().into_iter().flatten() {
            let Some(&slot) = slots.get(slot_name) else {
                continue;
            };
            for (kind, frames) in tracks.as_object().into_iter().flatten() {
                let Some(frames) = frames.as_array() else {
                    continue;
                };
                let track = format!("{name}/{slot_name}/{kind}");
                for k in frames {
                    note(f(k, "time", 0.0), &mut duration);
                }
                match kind.as_str() {
                    // A missing `name` means "show nothing from here", which is
                    // how Spine hides a slot without a visibility track.
                    "attachment" => timelines.push(Timeline::SlotAttachment {
                        slot,
                        keys: frames
                            .iter()
                            .map(|k| Key {
                                time: f(k, "time", 0.0),
                                value: s(k, "name").map(str::to_string),
                                interp: Interp::Stepped,
                            })
                            .collect(),
                    }),
                    "rgba" | "rgb" => timelines.push(Timeline::SlotColor {
                        slot,
                        keys: keys_with_curves(
                            frames,
                            0,
                            |k| s(k, "color").map(hex_color).unwrap_or([1.0; 4]),
                            // Spine curves each channel separately; alpha is the
                            // one that carries a fade, so normalize against it.
                            |k| s(k, "color").map(hex_color).unwrap_or([1.0; 4])[3],
                            &track,
                            report,
                        ),
                    }),
                    _ => {}
                }
            }
        }

        // ── Mesh deform ──────────────────────────────────────────────────
        // Spine writes a sparse run: `offset` floats into the vertex stream are
        // unchanged, then the listed pairs. For a rigid mesh that stream is one
        // (x, y) per vertex, which is our layout too.
        //
        // A weighted mesh instead streams one pair *per influence*, in bone-local
        // space, and our `Deform` keys one offset per vertex — there is nowhere
        // to put a per-influence value, so those are reported and dropped.
        for (_, slots_of_skin) in anim["attachments"].as_object().into_iter().flatten() {
            for (slot_name, atts) in slots_of_skin.as_object().into_iter().flatten() {
                let Some(&slot) = slots.get(slot_name) else {
                    continue;
                };
                for (att_name, tracks) in atts.as_object().into_iter().flatten() {
                    let Some(frames) = tracks["deform"].as_array() else {
                        continue;
                    };
                    let Some(Attachment::Mesh(mesh)) = skel.skins[default_skin]
                        .entries
                        .get(&(slot, att_name.clone()))
                    else {
                        continue;
                    };
                    let count = mesh.setup_vertices.len();
                    // One weight list per vertex, so the influence stream can be
                    // walked in step with the vertices it belongs to.
                    let influences: Vec<Vec<f32>> = mesh
                        .weights
                        .iter()
                        .map(|v| v.iter().map(|w| w.weight).collect())
                        .collect();
                    let weighted = !influences.is_empty();
                    if weighted {
                        report.lossy(
                            "deform",
                            format!("{slot_name}/{att_name}"),
                            "a weighted mesh's deform is approximated in mesh space",
                        );
                    }
                    for k in frames {
                        note(f(k, "time", 0.0), &mut duration);
                    }
                    timelines.push(Timeline::Deform {
                        slot,
                        attachment: att_name.clone(),
                        keys: keys_with_curves(
                            frames,
                            0,
                            |k| {
                                let offset =
                                    k.get("offset").and_then(|o| o.as_u64()).unwrap_or(0) as usize;
                                let run = floats(k, "vertices");
                                if !weighted {
                                    let mut out = vec![glam::Vec2::ZERO; count];
                                    for (i, pair) in run.chunks_exact(2).enumerate() {
                                        let v = offset / 2 + i;
                                        if v < count {
                                            out[v] = glam::vec2(pair[0], pair[1]);
                                        }
                                    }
                                    return out;
                                }
                                // Rebuild the dense influence stream, then collapse
                                // each vertex's influences by weight. Ours is one
                                // offset per vertex in the mesh's own frame; theirs
                                // is one per influence in each bone's. The weighted
                                // mean lands in the right place while a vertex's
                                // bones point roughly the same way — the usual case
                                // for a deform — and drifts when they do not.
                                let total: usize =
                                    influences.iter().map(|w| w.len()).sum::<usize>() * 2;
                                let mut flat = vec![0.0f32; total];
                                for (i, v) in run.iter().enumerate() {
                                    if offset + i < flat.len() {
                                        flat[offset + i] = *v;
                                    }
                                }
                                let mut out = Vec::with_capacity(count);
                                let mut cursor = 0usize;
                                for weights in &influences {
                                    let mut sum = glam::Vec2::ZERO;
                                    let mut mass = 0.0;
                                    for (i, weight) in weights.iter().enumerate() {
                                        let base = (cursor + i) * 2;
                                        if base + 1 >= flat.len() {
                                            break;
                                        }
                                        sum += glam::vec2(flat[base], flat[base + 1]) * *weight;
                                        mass += *weight;
                                    }
                                    cursor += weights.len();
                                    out.push(if mass > 0.0 { sum / mass } else { sum });
                                }
                                out
                            },
                            // Nothing scalar to normalize a curve against, so the
                            // handles stay in time only.
                            |_| 0.0,
                            &format!("{name}/{slot_name}/{att_name}/deform"),
                            report,
                        ),
                    });
                }
            }
        }

        // ── Constraint timelines ─────────────────────────────────────────
        for (cname, frames) in anim["ik"].as_object().into_iter().flatten() {
            let (Some(&constraint), Some(frames)) = (constraint_ids.get(cname), frames.as_array())
            else {
                continue;
            };
            for k in frames {
                note(f(k, "time", 0.0), &mut duration);
            }
            let track = format!("{name}/{cname}/ik");
            timelines.push(Timeline::IkMix {
                constraint,
                keys: keys_with_curves(
                    frames,
                    0,
                    |k| f(k, "mix", 1.0),
                    |k| f(k, "mix", 1.0),
                    &track,
                    report,
                ),
            });
            if frames.iter().any(|k| k.get("softness").is_some()) {
                timelines.push(Timeline::IkSoftness {
                    constraint,
                    keys: keys_with_curves(
                        frames,
                        1,
                        |k| f(k, "softness", 0.0),
                        |k| f(k, "softness", 0.0),
                        &track,
                        report,
                    ),
                });
            }
            // Only worth a track if it actually flips; otherwise it is a row of
            // identical keys cluttering the dopesheet.
            let bend = |k: &Value| {
                if k.get("bendPositive").and_then(|b| b.as_bool()) == Some(false) {
                    -1.0
                } else {
                    1.0
                }
            };
            if frames.windows(2).any(|w| bend(&w[0]) != bend(&w[1])) {
                timelines.push(Timeline::IkBendDirection {
                    constraint,
                    keys: frames
                        .iter()
                        .map(|k| Key {
                            time: f(k, "time", 0.0),
                            value: bend(k),
                            interp: Interp::Stepped,
                        })
                        .collect(),
                });
            }
        }

        for (cname, frames) in anim["transform"].as_object().into_iter().flatten() {
            let (Some(&constraint), Some(frames)) = (constraint_ids.get(cname), frames.as_array())
            else {
                continue;
            };
            for k in frames {
                note(f(k, "time", 0.0), &mut duration);
            }
            let track = format!("{name}/{cname}/transform");
            timelines.push(Timeline::TransformConstraintMix {
                constraint,
                keys: keys_with_curves(
                    frames,
                    0,
                    |k| {
                        // Same rule as the setup values: a channel a key does
                        // not mention is off, and each axis is its own channel.
                        ankhimate_core::constraints::TransformMix {
                            rotate: f(k, "mixRotate", 0.0),
                            translate: glam::vec2(f(k, "mixX", 0.0), f(k, "mixY", 0.0)),
                            scale: glam::vec2(f(k, "mixScaleX", 0.0), f(k, "mixScaleY", 0.0)),
                            shear: glam::vec2(f(k, "mixShearX", 0.0), f(k, "mixShearY", 0.0)),
                        }
                    },
                    |k| f(k, "mixRotate", 1.0),
                    &track,
                    report,
                ),
            });
        }

        let events: Vec<EventKey> = anim["events"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| EventKey {
                        time: f(e, "time", 0.0),
                        name: s(e, "name").unwrap_or("event").to_string(),
                        int_value: e.get("int").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        float_value: f(e, "float", 0.0),
                        string_value: s(e, "string").unwrap_or("").to_string(),
                        audio: String::new(),
                        volume: 1.0,
                        balance: 0.0,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for e in &events {
            note(e.time, &mut duration);
        }

        animations.insert(Animation {
            name: name.clone(),
            duration: duration.max(0.1),
            looping: true,
            events,
            timelines,
            // Spine has no equivalent, so an import brings none.
            markers: Vec::new(),
            bone_offsets: Vec::new(),
        });
    }
    Loaded {
        skeleton: skel,
        animations,
        assets,
        name: name.to_string(),
        // Spine stores no authoring frame rate in the skeleton; 30 is its own
        // editor default, and every time in the file is in seconds regardless.
        fps: 30,
        export_presets: Vec::new(),
        report: std::mem::take(report),
    }
}
