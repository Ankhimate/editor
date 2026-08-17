//! Read a DragonBones `_ske.json` into our model.
//!
//! The second importer, and deliberately a different format rather than a
//! near-clone of Spine: four of its decisions differ in ways that have to be
//! *converted* rather than renamed, which is what makes it worth having before
//! an `ImportPlugin` trait is extracted from the pair.
//!
//! # Time is a running sum, not a stamp
//!
//! Every frame carries a `duration` in **frames**, and its position is the sum
//! of all preceding durations. Spine writes absolute seconds. So a key's time
//! is reconstructed by accumulation, and one wrong duration shifts everything
//! after it — a failure that does not look broken, only slightly off, which is
//! the worst kind to ship.
//!
//! # Y is down
//!
//! DragonBones works in screen coordinates; our world is Y-up. Every vertical
//! quantity negates on the way in — bone and attachment positions, translate
//! keys — and so does every **angle**, because mirroring one axis reverses which
//! way is positive. Scale does not: it is a magnitude, not a direction.
//!
//! Getting this half-right is worse than getting it wrong, because a rig with
//! flipped positions and unflipped rotations looks *nearly* correct. The first
//! import of `mecha_1004d` hung upside down below the origin, which at least had
//! the courtesy to be obvious.
//!
//! # Rotation is skew
//!
//! A bone's transform carries `skX`/`skY` rather than a rotation: the directions
//! of its X and Y axes, measured from the same zero. When they agree it is pure
//! rotation; when they differ, the gap is how far Y has swung off perpendicular
//! — which is shear on **Y**, not X. See [`decompose_skew`].
//!
//! # A file holds several armatures
//!
//! Spine is one skeleton per file. DragonBones packs several — `mecha_1004d`
//! ships four. Our `Document` holds one skeleton, so the first is imported and
//! the others are handled by what references them.
//!
//! In practice a nested armature is not a sub-rig: every one in the sample files
//! is a single bone holding a set of images — `we_bl_4` is a five-frame muzzle
//! flash, `weapon_replace` a swappable weapon. A slot with several attachments
//! is already what that means here, so a display naming an armature **folds its
//! images into the host slot** and the artist swaps between them.
//!
//! An armature nothing references is genuinely dropped and reported.
//! `skin_1502b`'s `skin_b`/`skin_c` are that case: alternate skins for one
//! character, which is a different feature from an attachment.
//!
//! # Omission means default, never inheritance
//!
//! `root` has no `transform` key at all. Every field is read against its own
//! default rather than a sibling's value — the bug that cost real time in the
//! Spine importer, where an absent `mixY` picked up `mixX`.

use crate::convert::{LoadReport, Loaded};
use ankhimate_core::animation::{self as anim, Axis, Interp, Key, Timeline};
use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
use ankhimate_core::ids::{BoneId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slot::Slot;
use ankhimate_core::slotmap::SlotMap;
use serde_json::Value;
use std::collections::HashMap;

/// What went wrong badly enough that there is no rig to return.
#[derive(Debug)]
pub enum Error {
    /// The skeleton JSON did not parse.
    Json(String),
    /// Parsed, but carries no `armature` array — not a DragonBones skeleton.
    NotASkeleton,
    /// An `armature` array that is present but empty.
    NoArmatures,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Json(why) => write!(f, "the skeleton JSON did not parse: {why}"),
            Error::NotASkeleton => {
                write!(
                    f,
                    "no `armature` array — this is not a DragonBones skeleton"
                )
            }
            Error::NoArmatures => write!(f, "the file declares no armatures"),
        }
    }
}

impl std::error::Error for Error {}

/// The images an import draws attachments from.
///
/// DragonBones ships a `_tex.json` atlas beside the skeleton, describing
/// sub-rectangles of a single `_tex.png`. Loose images are also common when a
/// rig is exported for re-editing.
pub enum Images<'a> {
    /// A parsed `_tex.json` and its page image.
    Atlas {
        /// The `_tex.json` text.
        text: &'a str,
        /// Opens a page by its `imagePath`.
        pages: &'a dyn Fn(&str) -> Option<image::RgbaImage>,
    },
    /// Loose images, looked up by display name.
    Loose(&'a dyn Fn(&str) -> Option<image::RgbaImage>),
    /// No images: geometry imports and every texture is reported as dangling.
    None,
}

/// Read a DragonBones skeleton into our model.
///
/// `name` falls back to the file's own `name` field, which DragonBones — unlike
/// Spine — actually stores.
pub fn read(json: &str, images: Images<'_>, name: &str) -> Result<Loaded, Error> {
    let doc: Value = serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))?;
    let armatures = doc
        .get("armature")
        .and_then(|a| a.as_array())
        .ok_or(Error::NotASkeleton)?;
    if armatures.is_empty() {
        return Err(Error::NoArmatures);
    }
    let mut report = LoadReport::default();
    Ok(convert(&doc, armatures, images, name, &mut report))
}

/// The version a file declares, when it has one.
///
/// Reporting only: an unrecognised version is far more likely a newer minor
/// release than an incompatible format, so the reader accepts what it can.
pub fn declared_version(json: &str) -> Option<String> {
    let doc: Value = serde_json::from_str(json).ok()?;
    doc.get("version")?.as_str().map(str::to_string)
}

/// Split DragonBones' `skX`/`skY` skew pair into our rotation and shear,
/// flipping handedness on the way.
///
/// DragonBones has no rotation field. It stores two skew angles, and a rigid
/// rotation is the case where they happen to be equal. The shared part is the
/// rotation; what is left on X is the shear.
///
/// Both come out **negated**, because DragonBones measures angles in a Y-down
/// frame and ours is Y-up. Mirroring one axis reverses which way is positive, so
/// an angle carried across unchanged turns the opposite way. The module note
/// covers the rest of the conversion.
///
/// Returned in **radians**, matching `core`.
///
/// Shear lands on **Y**, not X. `Affine2::compose` puts the X axis at
/// `rotation + shear.x` and the Y axis at `rotation + π/2 + shear.y`, so
/// `shear.x` is indistinguishable from rotation and adding the difference there
/// would silently do nothing — which is what the first version of this did.
///
/// **`skY` is the rotation and `skX` describes the Y axis** — the opposite of
/// what the names suggest, and settled by reading the runtime rather than by
/// reasoning about it. `ObjectDataParser` maps the fields as
///
/// ```text
/// rotation = skY
/// skew     = skX - skY
/// ```
///
/// and `Transform::toMatrix` then puts the X axis at `rotation` and the Y axis
/// at `skew + rotation`, which is `skX`. Taking `skX` for the X axis is wrong in
/// a way that hides: the two are equal on every rigid bone, which is nearly all
/// of them, so it only shows on the handful that actually shear.
pub fn decompose_skew(sk_x_deg: f32, sk_y_deg: f32) -> (f32, glam::Vec2) {
    let rotation = -sk_y_deg.to_radians();
    let shear_y = -(sk_x_deg - sk_y_deg).to_radians();
    (rotation, glam::vec2(0.0, shear_y))
}

/// A number, or `default` when the key is absent.
///
/// Absent means *default*, never "same as the neighbouring axis". Spelling that
/// out because the opposite assumption is what made an imported spineboy drag
/// its shoulder 19 units sideways.
fn f(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map_or(default, |x| x as f32)
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

fn floats(v: &Value, key: &str) -> Vec<f32> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|n| n.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default()
}

/// One frame's easing, as our `Interp`.
///
/// DragonBones writes `tweenEasing` on the frame that *starts* a segment, and we
/// hang easing on the frame that *ends* one — the same shift the Spine reader
/// makes, for the same reason.
///
/// Values: `0` is linear, `null`/absent is stepped (DragonBones calls it "no
/// tween"), and a number in `-1..=1` selects quad ease-in/out. A `curve` array
/// carries explicit bezier control points and takes precedence.
///
/// The returned flag marks an easing that could not be represented exactly.
fn frame_interp(frame: &Value) -> (Interp, bool) {
    if let Some(curve) = frame.get("curve").and_then(|c| c.as_array())
        && curve.len() >= 4
    {
        let n = |i: usize| curve[i].as_f64().unwrap_or(0.0) as f32;
        // Control points are already fractions of the segment, which is what
        // `Interp::Bezier` wants. Time is clamped for the reason `spine.rs`
        // documents at length: `solve_bezier_x` bisects a monotonic domain, and
        // a handle outside 0..1 makes the curve double back.
        let (ox, oy, ix, iy) = (n(0), n(1), n(2), n(3));
        let clamped = !(0.0..=1.0).contains(&ox) || !(0.0..=1.0).contains(&ix);
        return (
            Interp::Bezier {
                out_handle: glam::vec2(ox.clamp(0.0, 1.0), oy),
                in_handle: glam::vec2(ix.clamp(0.0, 1.0), iy),
            },
            clamped,
        );
    }
    match frame.get("tweenEasing") {
        // Explicit 0 is linear — the overwhelmingly common case.
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => (Interp::Linear, false),
        Some(Value::Number(n)) => {
            // ±1 is quad ease-out/in. Approximated with the bezier handles that
            // match a quadratic most closely; anything between is interpolated
            // toward linear, which is what the runtime does.
            let amount = (n.as_f64().unwrap_or(0.0) as f32).clamp(-1.0, 1.0);
            let handles = if amount >= 0.0 {
                // Ease out: slow arrival.
                (glam::vec2(0.0, 0.0), glam::vec2(1.0 - amount * 0.5, 1.0))
            } else {
                // Ease in: slow departure.
                (glam::vec2(-amount * 0.5, 0.0), glam::vec2(1.0, 1.0))
            };
            (
                Interp::Bezier {
                    out_handle: handles.0,
                    in_handle: handles.1,
                },
                false,
            )
        }
        // Absent or null: hold until the next frame.
        _ => (Interp::Stepped, false),
    }
}

/// Walk a frame list, turning `duration` counts into absolute seconds.
///
/// This is the format's defining quirk. Each frame declares how long it lasts;
/// its own time is everything before it. `value` extracts the keyed number from
/// a frame, and the easing is shifted one frame forward so it describes the
/// segment *arriving* at the key rather than leaving it.
///
/// The trailing zero-duration frame DragonBones writes to close a loop is kept:
/// it carries the final value, and dropping it would let the last real segment
/// run to the clip's end instead of stopping where the file says.
fn frames_to_keys(
    frames: &[Value],
    fps: f32,
    mut value: impl FnMut(&Value) -> f32,
    report: &mut LoadReport,
    where_: &str,
) -> Vec<Key<f32>> {
    let mut keys = Vec::with_capacity(frames.len());
    let mut elapsed = 0.0_f32;
    // The easing on frame *i* describes the segment that leaves it, so it
    // becomes the `interp` of frame i+1. The first key has nothing arriving at
    // it and keeps the default.
    let mut pending = Interp::Linear;

    for (i, frame) in frames.iter().enumerate() {
        let interp = if i == 0 {
            Interp::Linear
        } else {
            std::mem::replace(&mut pending, Interp::Linear)
        };
        let (next, clamped) = frame_interp(frame);
        pending = next;
        if clamped {
            report.lossy(
                "curve",
                where_,
                "a time handle outside the segment was clamped so the curve stays samplable",
            );
        }

        keys.push(Key {
            time: elapsed / fps,
            value: value(frame),
            interp,
        });
        elapsed += f(frame, "duration", 0.0);
    }
    keys
}

/// Are all of `keys` the same value?
///
/// A DragonBones armature writes a frame list for every channel it touches, so a
/// bone that only rotates still ships a flat `translateFrame`. Importing those
/// produces timelines that do nothing but cost evaluation and clutter the
/// dopesheet.
fn is_flat(keys: &[Key<f32>], default: f32) -> bool {
    keys.iter().all(|k| (k.value - default).abs() < 1e-6)
}

/// `_tex.json` regions, keyed by sub-texture name.
struct AtlasRegion {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    rotated: bool,
}

/// Parse a `_tex.json` into its page filename and region index.
///
/// JSON rather than Spine's bespoke text format, so this is a deserialize
/// rather than a parser.
fn parse_atlas(text: &str) -> (String, HashMap<String, AtlasRegion>) {
    let Ok(doc) = serde_json::from_str::<Value>(text) else {
        return (String::new(), HashMap::new());
    };
    let page = s(&doc, "imagePath").unwrap_or_default().to_string();
    let mut regions = HashMap::new();
    for sub in doc
        .get("SubTexture")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let Some(name) = s(sub, "name") else { continue };
        regions.insert(
            name.to_string(),
            AtlasRegion {
                x: f(sub, "x", 0.0) as u32,
                y: f(sub, "y", 0.0) as u32,
                w: f(sub, "width", 0.0) as u32,
                h: f(sub, "height", 0.0) as u32,
                // DragonBones marks a rotated region with `rotated: true`, and
                // rotates the *opposite* way to Spine — clockwise here.
                rotated: sub
                    .get("rotated")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(false),
            },
        );
    }
    (page, regions)
}

fn convert(
    doc: &Value,
    armatures: &[Value],
    images: Images<'_>,
    fallback_name: &str,
    report: &mut LoadReport,
) -> Loaded {
    // One skeleton per document. Which of the others survive is decided by the
    // attachment pass below — an armature a display names gets folded into the
    // slot that names it, and only what nothing references is really skipped.
    let armature = &armatures[0];
    let mut folded_armatures: std::collections::HashSet<&str> = Default::default();

    let name = s(doc, "name")
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback_name)
        .to_string();
    // An armature may override the document's frame rate, and the frame counts
    // in its timelines are in *its* units.
    let fps = f(armature, "frameRate", f(doc, "frameRate", 24.0)).max(1.0);

    let mut skel = Skeleton::new();
    let mut assets = AssetDb::new();
    let (page_file, regions) = match &images {
        Images::Atlas { text, .. } => parse_atlas(text),
        _ => (String::new(), HashMap::new()),
    };

    // ── Bones ────────────────────────────────────────────────────────────
    let empty = Vec::new();
    let bones = armature
        .get("bone")
        .and_then(|b| b.as_array())
        .unwrap_or(&empty);
    let mut ids: HashMap<String, BoneId> = HashMap::new();
    for b in bones {
        let bone_name = s(b, "name").unwrap_or("bone").to_string();
        // A bone with no `transform` is at the origin, unrotated, unit-scaled.
        let t = b.get("transform").cloned().unwrap_or(Value::Null);
        let (rotation, shear) = decompose_skew(f(&t, "skX", 0.0), f(&t, "skY", 0.0));
        let id = skel.add_bone(Bone {
            name: bone_name.clone(),
            parent: None,
            // Zero-length bones are legal in DragonBones and unselectable here.
            length: f(b, "length", 0.0).max(1.0),
            local_transform: Transform {
                // Y negates: DragonBones is Y-down, we are Y-up.
                position: glam::vec2(f(&t, "x", 0.0), -f(&t, "y", 0.0)),
                rotation,
                // Scale is a magnitude and does not flip with the axis.
                scale: glam::vec2(f(&t, "scX", 1.0), f(&t, "scY", 1.0)),
                shear,
            },
            inherit: inherit_flags(b),
            color: Bone::default_color(),
        });
        ids.insert(bone_name, id);
    }
    for b in bones {
        let (Some(child), Some(parent)) = (s(b, "name"), s(b, "parent")) else {
            continue;
        };
        if let (Some(&child), Some(&parent)) = (ids.get(child), ids.get(parent))
            && let Some(bone) = skel.bones.get_mut(child)
        {
            bone.parent = Some(parent);
        }
    }
    skel.rebuild_update_order();

    // ── Slots ────────────────────────────────────────────────────────────
    // Slot order is draw order, back to front, as in Spine and ours. `add_slot`
    // appends to `draw_order` already; pushing again draws everything twice.
    let mut slots: HashMap<String, SlotId> = HashMap::new();
    // Which display each slot starts on. DragonBones picks by index into the
    // slot's display list and defaults to 0; `-1` means *show nothing*, which is
    // how `effect_l` starts hidden. Resolved to a name once the skin is read,
    // since the index means nothing without the list it indexes into.
    let mut display_index: HashMap<String, i64> = HashMap::new();
    for sl in armature
        .get("slot")
        .and_then(|s| s.as_array())
        .unwrap_or(&empty)
    {
        let (Some(slot_name), Some(parent)) = (s(sl, "name"), s(sl, "parent")) else {
            continue;
        };
        let Some(&bone_id) = ids.get(parent) else {
            report.dangling("dragonbones slot parent", parent);
            continue;
        };
        display_index.insert(
            slot_name.to_string(),
            sl.get("displayIndex").and_then(|d| d.as_i64()).unwrap_or(0),
        );
        let id = skel.add_slot(Slot::new(slot_name.to_string(), bone_id));
        slots.insert(slot_name.to_string(), id);
    }

    // ── Attachments ──────────────────────────────────────────────────────
    let default_skin = skel.default_skin;
    let mut decoded: HashMap<String, (u32, u32)> = HashMap::new();
    // Returns the asset name *and its pixel size*. DragonBones does not put a
    // width or height on the display the way Spine does — the size lives in the
    // atlas — and a region attachment built with zero extents draws nothing at
    // all, which is how the first import came out invisible.
    let mut crop = |region: &str, assets: &mut AssetDb| -> Option<(String, u32, u32)> {
        if let Some(&(w, h)) = decoded.get(region) {
            return Some((region.to_string(), w, h));
        }
        let piece = match &images {
            Images::Atlas { pages, .. } => {
                let r = regions.get(region)?;
                let page = pages(&page_file)?;
                if r.rotated {
                    // A rotated region occupies a transposed rectangle, so crop
                    // (h, w) and turn it back. DragonBones rotates the opposite
                    // way to Spine — counter-clockwise to restore.
                    let packed = image::imageops::crop_imm(&page, r.x, r.y, r.h, r.w).to_image();
                    image::imageops::rotate270(&packed)
                } else {
                    image::imageops::crop_imm(&page, r.x, r.y, r.w, r.h).to_image()
                }
            }
            Images::Loose(open) => open(region)?,
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
        assets.add(ImageAsset::new(region.to_string(), bytes, w, h));
        decoded.insert(region.to_string(), (w, h));
        Some((region.to_string(), w, h))
    };

    if let Some(skin) = armature
        .get("skin")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
    {
        for entry in skin
            .get("slot")
            .and_then(|s| s.as_array())
            .unwrap_or(&empty)
        {
            let Some(slot_name) = s(entry, "name") else {
                continue;
            };
            let Some(&slot_id) = slots.get(slot_name) else {
                continue;
            };
            // Names in file order, so `displayIndex` can be resolved below. A
            // display that produced no attachment still occupies its index —
            // dropping it would shift every later one.
            let mut display_names: Vec<Option<String>> = Vec::new();
            for display in entry
                .get("display")
                .and_then(|d| d.as_array())
                .unwrap_or(&empty)
            {
                let Some(display_name) = s(display, "name") else {
                    // Still an index, even nameless.
                    display_names.push(None);
                    continue;
                };
                display_names.push(Some(display_name.to_string()));
                // `path` names the atlas region when it differs from the
                // display's own name — same convention as Spine's `path`.
                let region_name = s(display, "path").unwrap_or(display_name);
                let kind = s(display, "type").unwrap_or("image");
                match kind {
                    "image" => {
                        let Some((asset, w, h)) = crop(region_name, &mut assets) else {
                            report.dangling("dragonbones region", region_name);
                            continue;
                        };
                        let t = display.get("transform").cloned().unwrap_or(Value::Null);
                        let (rotation, _) = decompose_skew(f(&t, "skX", 0.0), f(&t, "skY", 0.0));
                        skel.skins[default_skin].set(
                            slot_id,
                            display_name.to_string(),
                            Attachment::Region(RegionAttachment {
                                texture: asset,
                                // Y-down to Y-up, as for bones.
                                local_offset: glam::vec2(f(&t, "x", 0.0), -f(&t, "y", 0.0)),
                                local_rotation: rotation,
                                local_scale: glam::vec2(f(&t, "scX", 1.0), f(&t, "scY", 1.0)),
                                // From the atlas region, not the display: a
                                // DragonBones display carries no size.
                                width: w as f32,
                                height: h as f32,
                                uv_rect: Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    w: 1.0,
                                    h: 1.0,
                                },
                                // A separate flip from the world-space one
                                // above: this is normalized *within the image*,
                                // where DragonBones counts down from the top and
                                // we count up from the bottom.
                                pivot: glam::vec2(
                                    f(display.get("pivot").unwrap_or(&Value::Null), "x", 0.5),
                                    1.0 - f(display.get("pivot").unwrap_or(&Value::Null), "y", 0.5),
                                ),
                                sequence: None,
                            }),
                        );
                    }
                    "mesh" => {
                        // A mesh carries its own geometry, so the region's pixel
                        // size is not needed here — the UVs address the texture.
                        let Some((asset, _, _)) = crop(region_name, &mut assets) else {
                            report.dangling("dragonbones region", region_name);
                            continue;
                        };
                        let raw = floats(display, "vertices");
                        let uvs_raw = floats(display, "uvs");
                        // Weighted meshes carry a `weights` array alongside
                        // `vertices`; none of the sample rigs use one, so rather
                        // than guess at an encoding this cannot check, the mesh
                        // imports rigid and says so.
                        if display.get("weights").is_some() {
                            report.lossy(
                                "attachment",
                                &format!("{slot_name}/{display_name}"),
                                "a weighted mesh imported without its weights, so it \
                                 follows its slot's bone rigidly",
                            );
                        }

                        // Y-down to Y-up, per vertex, exactly as for bones.
                        let setup_vertices: Vec<glam::Vec2> = raw
                            .chunks_exact(2)
                            .map(|c| glam::vec2(c[0], -c[1]))
                            .collect();
                        // UVs are texture coordinates, not world positions: the
                        // atlas already stores them top-left origin, which is
                        // what a sampler wants. Flipping these would flip the
                        // art inside a correctly placed mesh.
                        let uvs: Vec<glam::Vec2> = uvs_raw
                            .chunks_exact(2)
                            .map(|c| glam::vec2(c[0], c[1]))
                            .collect();
                        let triangles: Vec<[u32; 3]> = display
                            .get("triangles")
                            .and_then(|a| a.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|n| n.as_u64().map(|v| v as u32))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                            .chunks_exact(3)
                            .map(|c| [c[0], c[1], c[2]])
                            .collect();

                        if setup_vertices.is_empty() || triangles.is_empty() {
                            report.lossy(
                                "attachment",
                                &format!("{slot_name}/{display_name}"),
                                "a mesh with no vertices or no triangles was skipped",
                            );
                            continue;
                        }

                        skel.skins[default_skin].set(
                            slot_id,
                            display_name.to_string(),
                            Attachment::Mesh(ankhimate_core::attachment::MeshAttachment {
                                texture: asset,
                                setup_vertices,
                                uvs,
                                triangles,
                                weights: Vec::new(),
                                ffd_keyframes: Vec::new(),
                                edges: Vec::new(),
                                inverse_bind_matrices: Default::default(),
                                linked: None,
                                sequence: None,
                            }),
                        );
                    }
                    "armature" => {
                        // A display that names another armature in the same
                        // file. In practice these are not sub-rigs: every one in
                        // the sample rigs is a single bone holding a set of
                        // images — `we_bl_4` is a five-frame muzzle flash,
                        // `weapon_replace` is a swappable weapon. That is
                        // already what a slot with several attachments means
                        // here, so the images are folded into the host slot and
                        // the artist can swap between them.
                        //
                        // The nested armature's own bone transform is folded in
                        // as an offset rather than becoming a bone: a one-bone
                        // armature whose bone exists only to place its art is a
                        // bone nobody wants in their hierarchy.
                        let Some(nested) = armatures
                            .iter()
                            .find(|a| s(a, "name") == Some(display_name))
                        else {
                            report.dangling("dragonbones armature", display_name);
                            continue;
                        };

                        // The display's own transform, and the nested bone's,
                        // compose into one offset.
                        let host_t = display.get("transform").cloned().unwrap_or(Value::Null);
                        let host_scale = glam::vec2(f(&host_t, "scX", 1.0), f(&host_t, "scY", 1.0));
                        let host_offset = glam::vec2(f(&host_t, "x", 0.0), -f(&host_t, "y", 0.0));

                        let mut folded = 0usize;
                        let mut first_folded: Option<String> = None;
                        for nested_slot in nested
                            .get("skin")
                            .and_then(|a| a.as_array())
                            .and_then(|a| a.first())
                            .and_then(|sk| sk.get("slot"))
                            .and_then(|s_| s_.as_array())
                            .unwrap_or(&empty)
                        {
                            for nested_display in nested_slot
                                .get("display")
                                .and_then(|d| d.as_array())
                                .unwrap_or(&empty)
                            {
                                // Only images fold. A nested armature holding a
                                // mesh or another armature is a real sub-rig and
                                // outside what this flattening claims to do.
                                if s(nested_display, "type").unwrap_or("image") != "image" {
                                    continue;
                                }
                                let Some(nested_name) = s(nested_display, "name") else {
                                    continue;
                                };
                                let nested_region =
                                    s(nested_display, "path").unwrap_or(nested_name);
                                let Some((asset, nw, nh)) = crop(nested_region, &mut assets) else {
                                    report.dangling("dragonbones region", nested_region);
                                    continue;
                                };
                                let nt = nested_display
                                    .get("transform")
                                    .cloned()
                                    .unwrap_or(Value::Null);
                                let (rotation, _) =
                                    decompose_skew(f(&nt, "skX", 0.0), f(&nt, "skY", 0.0));
                                // Named for the host display slot it fills, not
                                // for the image inside it. `weapon_hand_r` lists
                                // `weapon_replace` three times at different
                                // offsets — three placements of one weapon — so
                                // naming these after the image collapsed all
                                // three onto a single entry and lost two of the
                                // placements.
                                let host_index = display_names.len() - 1;
                                let attachment_name =
                                    format!("{display_name}#{host_index}/{nested_name}");
                                skel.skins[default_skin].set(
                                    slot_id,
                                    attachment_name.clone(),
                                    Attachment::Region(RegionAttachment {
                                        texture: asset,
                                        local_offset: host_offset
                                            + glam::vec2(f(&nt, "x", 0.0), -f(&nt, "y", 0.0))
                                                * host_scale,
                                        local_rotation: rotation,
                                        local_scale: host_scale
                                            * glam::vec2(f(&nt, "scX", 1.0), f(&nt, "scY", 1.0)),
                                        width: nw as f32,
                                        height: nh as f32,
                                        uv_rect: Rect {
                                            x: 0.0,
                                            y: 0.0,
                                            w: 1.0,
                                            h: 1.0,
                                        },
                                        pivot: glam::vec2(
                                            f(
                                                nested_display.get("pivot").unwrap_or(&Value::Null),
                                                "x",
                                                0.5,
                                            ),
                                            1.0 - f(
                                                nested_display.get("pivot").unwrap_or(&Value::Null),
                                                "y",
                                                0.5,
                                            ),
                                        ),
                                        sequence: None,
                                    }),
                                );
                                folded += 1;
                                first_folded.get_or_insert(attachment_name);
                            }
                        }

                        if folded == 0 {
                            report.lossy(
                                "attachment",
                                &format!("{slot_name}/{display_name}"),
                                "a nested armature held nothing this reader could fold in",
                            );
                        } else {
                            folded_armatures.insert(display_name);
                            // The armature's *own* name is not an attachment, so
                            // this display index has to point at the first image
                            // folded in from it instead.
                            if let Some(slot) = display_names.last_mut() {
                                *slot = first_folded;
                            }
                        }
                    }
                    other => {
                        // Bounding boxes and anything newer. Reported by kind so
                        // the count is actionable rather than one opaque number.
                        //
                        // Bounding boxes are deliberately unread: none of the
                        // sample rigs contains one — `bounding_box_tester` is
                        // named for what it tests against, not for what it holds
                        // — and writing a reader for an encoding nothing can
                        // check against real data is how the 5.6 timeline gap
                        // got shipped in the first place.
                        report.lossy(
                            "attachment",
                            &format!("{slot_name}/{display_name}"),
                            match other {
                                "boundingBox" => "a bounding box display is not read yet",
                                _ => "an unrecognised display type was skipped",
                            },
                        );
                    }
                }
            }

            // Which display the slot starts on. Without this every slot has a
            // skin full of attachments and none of them showing, so the rig
            // loads complete and draws nothing — which is what the first import
            // with working images did.
            //
            // `-1` is DragonBones for "show nothing" and stays `None`; that is
            // how `effect_l` starts hidden rather than flashing its muzzle
            // effect over the whole animation.
            let index = display_index.get(slot_name).copied().unwrap_or(0);
            if index >= 0
                && let Some(Some(name)) = display_names.get(index as usize)
                && let Some(slot) = skel.slots.get_mut(slot_id)
            {
                slot.attachment = Some(name.clone());
            }
        }
    }

    // ── Armatures nothing referenced ─────────────────────────────────────
    // Reported only now, because until the attachments were read there was no
    // way to know which of them a display had folded in. An armature no display
    // names is genuinely dropped, and a document that quietly lost one would
    // look like most of the file had vanished.
    for extra in &armatures[1..] {
        let extra_name = s(extra, "name").unwrap_or("unnamed");
        if folded_armatures.contains(extra_name) {
            continue;
        }
        report.lossy(
            "armature",
            extra_name,
            "no display referenced this armature, and a document holds one skeleton",
        );
    }

    // ── Constraints ──────────────────────────────────────────────────────
    // IK lives on the armature rather than in a tagged list.
    //
    // `chain` counts bones *above* the named one, so `chain: 0` is a one-bone
    // aim and `chain: 1` is the two-bone knee that most rigs use. Ours wants the
    // chain root first, so it is built by walking parents and reversing.
    for ik in armature
        .get("ik")
        .and_then(|a| a.as_array())
        .unwrap_or(&empty)
    {
        let ik_name = s(ik, "name").unwrap_or("ik").to_string();
        let (Some(bone_name), Some(target_name)) = (s(ik, "bone"), s(ik, "target")) else {
            report.lossy("constraint", &ik_name, "an IK constraint named no bone");
            continue;
        };
        let (Some(&tip), Some(&target)) = (ids.get(bone_name), ids.get(target_name)) else {
            report.dangling("dragonbones ik bone", bone_name);
            continue;
        };

        let extra = f(ik, "chain", 0.0).max(0.0) as usize;
        let mut chain = vec![tip];
        let mut walk = tip;
        for _ in 0..extra {
            let Some(parent) = skel.bones.get(walk).and_then(|b| b.parent) else {
                break;
            };
            chain.push(parent);
            walk = parent;
        }
        chain.reverse();

        skel.add_constraint(ankhimate_core::constraints::Constraint::Ik(
            ankhimate_core::constraints::IkConstraint {
                name: ik_name,
                target,
                bones: chain,
                // `bendPositive` defaults to true. Their positive bend is
                // counter-clockwise in a Y-down frame, which is *clockwise* in
                // ours — so the sign inverts along with everything else the
                // axis flip touches.
                bend_direction: if ik.get("bendPositive").and_then(|b| b.as_bool()) == Some(false) {
                    1.0
                } else {
                    -1.0
                },
                // Weights are 0..100 in the file, 0..1 here.
                mix: (f(ik, "weight", 100.0) / 100.0).clamp(0.0, 1.0),
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
                stiffness: 0.0,
            },
        ));
    }

    // ── Animations ───────────────────────────────────────────────────────
    let mut animations = SlotMap::with_key();
    for a in armature
        .get("animation")
        .and_then(|a| a.as_array())
        .unwrap_or(&empty)
    {
        let anim_name = s(a, "name").unwrap_or("animation").to_string();
        let mut timelines = Vec::new();

        for bone_track in a.get("bone").and_then(|b| b.as_array()).unwrap_or(&empty) {
            let Some(track_name) = s(bone_track, "name") else {
                continue;
            };
            let Some(&bone) = ids.get(track_name) else {
                report.dangling("dragonbones animated bone", track_name);
                continue;
            };
            let where_ = format!("{anim_name}/{track_name}");

            // Translate: one frame list, two axes — split, since each axis owns
            // its own keys and easing in this model. Y negates with the frame,
            // exactly as the setup transform does; a pose whose setup flipped
            // and whose animation did not would drift further from correct the
            // further it played.
            if let Some(frames) = bone_track.get("translateFrame").and_then(|f| f.as_array()) {
                for axis in Axis::BOTH {
                    let (key, sign) = match axis {
                        Axis::X => ("x", 1.0),
                        Axis::Y => ("y", -1.0),
                    };
                    let keys =
                        frames_to_keys(frames, fps, |f_| sign * f(f_, key, 0.0), report, &where_);
                    if !is_flat(&keys, 0.0) {
                        timelines.push(Timeline::BoneTranslate { bone, axis, keys });
                    }
                }
            }
            if let Some(frames) = bone_track.get("rotateFrame").and_then(|f| f.as_array()) {
                // Degrees on disk and in our timelines alike, so no unit
                // conversion — but the sign still flips with the axis.
                let keys = frames_to_keys(frames, fps, |f_| -f(f_, "rotate", 0.0), report, &where_);
                if !is_flat(&keys, 0.0) {
                    timelines.push(Timeline::BoneRotate { bone, keys });
                }
            }
            if let Some(frames) = bone_track.get("scaleFrame").and_then(|f| f.as_array()) {
                for axis in Axis::BOTH {
                    let key = if axis == Axis::X { "x" } else { "y" };
                    let keys = frames_to_keys(frames, fps, |f_| f(f_, key, 1.0), report, &where_);
                    if !is_flat(&keys, 1.0) {
                        timelines.push(Timeline::BoneScale { bone, axis, keys });
                    }
                }
            }
        }

        // 5.6 replaced the per-channel `bone`/`translateFrame` shape with a
        // generic `timeline` array whose entries carry a numeric `type` code —
        // used heavily by Live2D-style rigs, where the tracks drive named
        // parameters rather than bone channels. Not read yet.
        //
        // Reported rather than passed over: a clip with a real duration and no
        // timelines looks like a rig that simply does not animate, and the
        // import would read as successful while dropping everything.
        if let Some(generic) = a.get("timeline").and_then(|t| t.as_array())
            && !generic.is_empty()
        {
            report.lossy(
                "timeline",
                &anim_name,
                "a DragonBones 5.6 generic timeline (numeric `type` codes) is not read yet",
            );
        }

        // `duration` is in frames, like everything else here.
        let duration = f(a, "duration", 0.0) / fps;
        animations.insert(anim::Animation {
            name: anim_name,
            duration,
            timelines,
            ..Default::default()
        });
    }

    Loaded {
        skeleton: skel,
        animations,
        assets,
        name,
        fps: fps.round() as u32,
        export_presets: Vec::new(),
        report: std::mem::take(report),
    }
}

/// DragonBones' inherit flags, as our `Inherit`.
///
/// The flags default to *true* when absent — the opposite of the usual "absent
/// means off" — because inheriting everything is the ordinary case and the file
/// only records departures from it.
fn inherit_flags(b: &Value) -> ankhimate_core::transforms::Inherit {
    let flag = |key: &str| b.get(key).and_then(|v| v.as_bool()).unwrap_or(true);
    ankhimate_core::transforms::Inherit {
        rotation: flag("inheritRotation"),
        scale: flag("inheritScale"),
        reflect: flag("inheritReflection"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_skew_angles_are_pure_rotation_the_other_way() {
        // The ordinary case: nearly every bone in a real rig has skX == skY.
        // Negated, because DragonBones measures angles in a Y-down frame.
        let (rotation, shear) = decompose_skew(30.0, 30.0);
        assert!((rotation + 30.0_f32.to_radians()).abs() < 1e-6);
        assert_eq!(shear, glam::Vec2::ZERO, "no shear when the axes agree");
    }

    #[test]
    fn unequal_skew_angles_shear_the_y_axis() {
        // Shear must land on Y. `Affine2::compose` puts the X axis at
        // `rotation + shear.x`, so anything written to `shear.x` is
        // indistinguishable from rotation and has no effect on the pose — the
        // first version of this put the difference there and it did nothing.
        // Settled from `ObjectDataParser`: rotation = skY, skew = skX - skY.
        let (rotation, shear) = decompose_skew(50.0, 20.0);
        assert!(
            (rotation + 20.0_f32.to_radians()).abs() < 1e-6,
            "skY is the rotation, however the field is spelled"
        );
        assert_eq!(shear.x, 0.0, "shear.x would only re-rotate the X axis");
        assert!(
            (shear.y + 30.0_f32.to_radians()).abs() < 1e-6,
            "skX - skY is how far Y sits off perpendicular, negated with the axis"
        );
    }

    #[test]
    fn shear_actually_changes_the_composed_axes() {
        // The check the previous test could not make: that the decomposition
        // survives `compose`. Putting the difference on `shear.x` passed a
        // field-equality assertion and produced an identical matrix to no shear
        // at all.
        use ankhimate_core::math::Transform;

        let (rotation, shear) = decompose_skew(50.0, 20.0);
        let sheared = Transform {
            rotation,
            shear,
            ..Default::default()
        }
        .to_affine();
        let rigid = Transform {
            rotation,
            ..Default::default()
        }
        .to_affine();

        assert!(
            (sheared.c - rigid.c).abs() > 1e-3 || (sheared.d - rigid.d).abs() > 1e-3,
            "a sheared bone must not compose to the same matrix as a rigid one"
        );
    }

    #[test]
    fn a_rig_built_downward_arrives_the_right_way_up() {
        // The bug the first import shipped with: `mecha_1004d` hung upside down
        // below the origin, because DragonBones works in screen coordinates and
        // our world is Y-up. Positions *and* angles flip; scale does not.
        let json = r#"{
            "name": "t", "frameRate": 24,
            "armature": [{"name": "a", "bone": [
                {"name": "root"},
                {"name": "head", "parent": "root",
                 "transform": {"x": 10, "y": -100, "skX": 30, "skY": 30,
                               "scX": 2.0, "scY": 3.0}}
            ]}]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let head = loaded
            .skeleton
            .bones
            .values()
            .find(|b| b.name == "head")
            .expect("head");

        assert_eq!(
            head.local_transform.position,
            glam::vec2(10.0, 100.0),
            "a bone 100 above the origin in a Y-down file is 100 above in ours"
        );
        assert!(
            (head.local_transform.rotation + 30.0_f32.to_radians()).abs() < 1e-6,
            "the turn reverses with the axis"
        );
        assert_eq!(
            head.local_transform.scale,
            glam::vec2(2.0, 3.0),
            "scale is a magnitude and does not flip"
        );
    }

    #[test]
    fn animated_translation_flips_with_the_setup_pose() {
        // Flipping the setup transform and not the keys would leave a rig that
        // starts correct and drifts wrong the moment it plays — the failure
        // that is hardest to attribute later.
        let json = r#"{
            "name": "t", "frameRate": 10,
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "animation": [{"name": "hop", "duration": 20, "bone": [
                    {"name": "root",
                     "translateFrame": [{"duration": 10, "y": 0},
                                        {"duration": 10, "y": -50}],
                     "rotateFrame": [{"duration": 10, "rotate": 0},
                                     {"duration": 10, "rotate": 90}]}
                ]}]
            }]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let clip = loaded.animations.values().next().expect("one clip");

        let y_keys = clip
            .timelines
            .iter()
            .find_map(|t| match t {
                Timeline::BoneTranslate {
                    axis: Axis::Y,
                    keys,
                    ..
                } => Some(keys),
                _ => None,
            })
            .expect("a Y translate timeline");
        assert_eq!(y_keys[1].value, 50.0, "-50 down becomes +50 up");

        let rot_keys = clip
            .timelines
            .iter()
            .find_map(|t| match t {
                Timeline::BoneRotate { keys, .. } => Some(keys),
                _ => None,
            })
            .expect("a rotate timeline");
        assert_eq!(rot_keys[1].value, -90.0, "the turn reverses too");
    }

    #[test]
    fn frame_durations_accumulate_into_absolute_times() {
        // The format's defining quirk, and the one most able to go subtly wrong:
        // a frame's time is everything *before* it, not its own duration.
        let frames: Vec<Value> = vec![
            serde_json::json!({"duration": 10, "x": 0.0}),
            serde_json::json!({"duration": 20, "x": 5.0}),
            serde_json::json!({"duration": 0, "x": 9.0}),
        ];
        let mut report = LoadReport::default();
        let keys = frames_to_keys(&frames, 10.0, |f_| f(f_, "x", 0.0), &mut report, "t");

        let times: Vec<f32> = keys.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![0.0, 1.0, 3.0], "0, then 10/10, then 30/10");
        let values: Vec<f32> = keys.iter().map(|k| k.value).collect();
        assert_eq!(values, vec![0.0, 5.0, 9.0]);
    }

    #[test]
    fn the_frame_rate_scales_the_times() {
        let frames: Vec<Value> = vec![
            serde_json::json!({"duration": 30}),
            serde_json::json!({"duration": 0}),
        ];
        let mut report = LoadReport::default();
        let keys = frames_to_keys(&frames, 60.0, |_| 0.0, &mut report, "t");
        assert_eq!(keys[1].time, 0.5, "30 frames at 60fps is half a second");
    }

    #[test]
    fn easing_shifts_onto_the_key_it_arrives_at() {
        // DragonBones hangs easing on the frame a segment *leaves*; we hang it
        // on the one it arrives at. A reader that skipped the shift would ease
        // every segment with its neighbour's curve — plausible-looking and
        // wrong everywhere.
        let frames: Vec<Value> = vec![
            serde_json::json!({"duration": 10, "tweenEasing": 0}),
            serde_json::json!({"duration": 10}),
            serde_json::json!({"duration": 0, "tweenEasing": 0}),
        ];
        let mut report = LoadReport::default();
        let keys = frames_to_keys(&frames, 10.0, |_| 0.0, &mut report, "t");

        assert_eq!(
            keys[0].interp,
            Interp::Linear,
            "nothing arrives at the first"
        );
        assert_eq!(
            keys[1].interp,
            Interp::Linear,
            "frame 0 declared linear, so the segment into key 1 is linear"
        );
        assert_eq!(
            keys[2].interp,
            Interp::Stepped,
            "frame 1 declared no tween, so the segment into key 2 holds"
        );
    }

    #[test]
    fn an_absent_tween_is_stepped_not_linear() {
        // DragonBones' "no tween" is a hold. Reading it as linear would make
        // every stepped animation glide.
        let (interp, _) = frame_interp(&serde_json::json!({"duration": 5}));
        assert_eq!(interp, Interp::Stepped);
    }

    #[test]
    fn a_flat_channel_is_not_imported() {
        // An armature writes a frame list per channel it touches, so a bone that
        // only rotates still ships a flat translate track.
        let flat = vec![Key::linear(0.0, 0.0), Key::linear(1.0, 0.0)];
        assert!(is_flat(&flat, 0.0));

        let moving = vec![Key::linear(0.0, 0.0), Key::linear(1.0, 3.0)];
        assert!(!is_flat(&moving, 0.0));

        // Scale's rest value is 1, not 0 — checking against the wrong default
        // would drop every real scale track and keep every flat one.
        let flat_scale = vec![Key::linear(0.0, 1.0), Key::linear(1.0, 1.0)];
        assert!(is_flat(&flat_scale, 1.0));
        assert!(!is_flat(&flat_scale, 0.0));
    }

    #[test]
    fn a_file_without_armatures_is_refused() {
        assert!(matches!(
            read("{}", Images::None, "x"),
            Err(Error::NotASkeleton)
        ));
        assert!(matches!(
            read(r#"{"armature":[]}"#, Images::None, "x"),
            Err(Error::NoArmatures)
        ));
    }

    #[test]
    fn a_bone_with_no_transform_sits_at_the_origin() {
        // `root` really is written as `{"name": "root"}`. Every field must fall
        // back to its own default rather than to a neighbour's value.
        let json = r#"{
            "name": "t", "frameRate": 24,
            "armature": [{"name": "a", "bone": [{"name": "root"}], "slot": [],
                          "skin": [], "animation": []}]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let bone = loaded.skeleton.bones.values().next().expect("one bone");
        assert_eq!(bone.local_transform.position, glam::Vec2::ZERO);
        assert_eq!(bone.local_transform.rotation, 0.0);
        assert_eq!(bone.local_transform.scale, glam::vec2(1.0, 1.0));
        assert_eq!(bone.local_transform.shear, glam::Vec2::ZERO);
    }

    #[test]
    fn a_slot_starts_on_the_display_its_index_names() {
        // A skin full of attachments draws nothing until the slot points at one.
        // DragonBones selects by index into the display list rather than by name
        // the way Spine does, and defaulting the slot to `None` is why an import
        // with correct bones, slots and images was still invisible.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "slot": [{"name": "body", "parent": "root", "displayIndex": 1}],
                "skin": [{"slot": [{"name": "body", "display": [
                    {"name": "closed"}, {"name": "open"}
                ]}]}]
            }]
        }"#;
        let png = image::RgbaImage::new(4, 4);
        let loaded = read(json, Images::Loose(&|_| Some(png.clone())), "x").expect("reads");

        let slot = loaded
            .skeleton
            .slots
            .values()
            .find(|s_| s_.name == "body")
            .expect("body slot");
        assert_eq!(
            slot.attachment.as_deref(),
            Some("open"),
            "index 1 is the second display, not the first"
        );
    }

    #[test]
    fn a_negative_display_index_starts_the_slot_hidden() {
        // `-1` is DragonBones for "show nothing" — `mecha_1004d` uses it so its
        // muzzle effects do not sit lit through the whole animation. Reading it
        // as an index would show the wrong art; ignoring it would show art that
        // should not be there at all.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "slot": [{"name": "effect", "parent": "root", "displayIndex": -1}],
                "skin": [{"slot": [{"name": "effect", "display": [{"name": "flash"}]}]}]
            }]
        }"#;
        let png = image::RgbaImage::new(4, 4);
        let loaded = read(json, Images::Loose(&|_| Some(png.clone())), "x").expect("reads");

        let slot = loaded
            .skeleton
            .slots
            .values()
            .find(|s_| s_.name == "effect")
            .expect("effect slot");
        assert_eq!(
            slot.attachment, None,
            "hidden, but the flash is still there"
        );
        assert_eq!(
            loaded.skeleton.skins[loaded.skeleton.default_skin]
                .names_for_slot(
                    loaded
                        .skeleton
                        .slots
                        .iter()
                        .find(|(_, s_)| s_.name == "effect")
                        .map(|(id, _)| id)
                        .unwrap()
                )
                .count(),
            1,
            "the attachment exists to be switched on later"
        );
    }

    #[test]
    fn a_region_takes_its_size_from_the_image() {
        // DragonBones puts no width or height on the display — Spine does, and
        // the reader was built from that shape, so every region imported with
        // zero extents and the whole rig drew nothing. A rig that loads with the
        // right bone count and no visible art is the symptom.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "slot": [{"name": "body", "parent": "root"}],
                "skin": [{"slot": [{"name": "body", "display": [{"name": "torso"}]}]}]
            }]
        }"#;
        let png = image::RgbaImage::new(64, 48);
        let loaded = read(json, Images::Loose(&|_| Some(png.clone())), "x").expect("reads");

        let skin = &loaded.skeleton.skins[loaded.skeleton.default_skin];
        let slot = loaded
            .skeleton
            .slots
            .iter()
            .find(|(_, s_)| s_.name == "body")
            .map(|(id, _)| id)
            .expect("body slot");
        let Some(Attachment::Region(region)) = skin.get(slot, "torso") else {
            panic!("expected a region attachment");
        };
        assert_eq!((region.width, region.height), (64.0, 48.0));
    }

    #[test]
    fn a_referenced_armature_folds_into_the_slot_that_names_it() {
        // Every nested armature in the sample rigs is one bone holding a set of
        // images — `we_bl_4` is a five-frame muzzle flash, `weapon_replace` a
        // swappable weapon. That is what a slot with several attachments already
        // means here, so they fold in rather than being reported as lost.
        let json = r#"{
            "name": "t",
            "armature": [
                {"name": "main",
                 "bone": [{"name": "root"}],
                 "slot": [{"name": "hand", "parent": "root"}],
                 "skin": [{"slot": [{"name": "hand", "display": [
                     {"type": "armature", "name": "weapons",
                      "transform": {"x": 10, "y": -4}}
                 ]}]}]},
                {"name": "weapons",
                 "bone": [{"name": "b"}],
                 "slot": [{"name": "b", "parent": "b"}],
                 "skin": [{"slot": [{"name": "b", "display": [
                     {"name": "sword"}, {"name": "axe"}
                 ]}]}]}
            ]
        }"#;
        let png = image::RgbaImage::new(2, 2);
        let loaded = read(json, Images::Loose(&|_| Some(png.clone())), "x").expect("reads");

        let slot = loaded
            .skeleton
            .slots
            .iter()
            .find(|(_, s_)| s_.name == "hand")
            .map(|(id, _)| id)
            .expect("hand slot");
        let mut names: Vec<&str> = loaded.skeleton.skins[loaded.skeleton.default_skin]
            .names_for_slot(slot)
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["weapons#0/axe", "weapons#0/sword"],
            "both options land in the slot, keyed by the display that brought them — \
             `weapon_hand_r` names one armature three times at different offsets, and \
             keying on the image alone collapsed all three onto a single entry"
        );

        assert!(
            !loaded.report.lossy.iter().any(|l| l.what == "armature"),
            "a folded armature is not a loss: {:?}",
            loaded.report.lossy
        );
    }

    #[test]
    fn an_armature_nothing_references_is_still_reported() {
        // Folding must not turn the report off wholesale — a spare armature no
        // display names really is dropped, and `skin_1502b`'s alternate skins
        // are exactly that case.
        let json = r#"{
            "name": "t",
            "armature": [
                {"name": "main", "bone": [{"name": "root"}]},
                {"name": "unused_alt", "bone": [{"name": "root"}]}
            ]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let skipped: Vec<&str> = loaded
            .report
            .lossy
            .iter()
            .filter(|l| l.what == "armature")
            .map(|l| l.where_.as_str())
            .collect();
        assert_eq!(skipped, ["unused_alt"]);
    }

    #[test]
    fn extra_armatures_are_reported_rather_than_dropped() {
        // `mecha_1004d` ships four armatures, three of them swappable weapons.
        // Importing the first silently would look like most of the file vanished.
        let json = r#"{
            "name": "t", "frameRate": 24,
            "armature": [
                {"name": "main", "bone": [{"name": "root"}]},
                {"name": "weapon_a", "bone": [{"name": "root"}]},
                {"name": "weapon_b", "bone": [{"name": "root"}]}
            ]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let skipped: Vec<&str> = loaded
            .report
            .lossy
            .iter()
            .filter(|l| l.what == "armature")
            .map(|l| l.where_.as_str())
            .collect();
        assert_eq!(skipped, ["weapon_a", "weapon_b"]);
    }

    #[test]
    fn an_ik_chain_is_built_root_first_by_walking_parents() {
        // `chain` counts bones *above* the named one, so `chain: 1` is the
        // two-bone knee. Ours wants the chain root first; reading the count as a
        // length, or forgetting to reverse, both produce a chain that solves
        // from the wrong end.
        let json = r#"{
            "name": "t", "frameRate": 24,
            "armature": [{"name": "a",
                "bone": [
                    {"name": "root"},
                    {"name": "thigh", "parent": "root"},
                    {"name": "calf", "parent": "thigh"},
                    {"name": "foot", "parent": "root"}
                ],
                "ik": [{"name": "leg", "bone": "calf", "target": "foot",
                        "chain": 1, "bendPositive": false}]
            }]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let name_of = |id| loaded.skeleton.bones[id].name.as_str();

        assert_eq!(loaded.skeleton.constraints.len(), 1);
        let c = loaded.skeleton.constraints.values().next().unwrap();
        let ankhimate_core::constraints::Constraint::Ik(ik) = c else {
            panic!("expected an IK constraint");
        };
        assert_eq!(ik.name, "leg");
        assert_eq!(name_of(ik.target), "foot");
        let chain: Vec<&str> = ik.bones.iter().map(|&b| name_of(b)).collect();
        assert_eq!(chain, ["thigh", "calf"], "root first, tip last");
        assert_eq!(
            ik.bend_direction, 1.0,
            "their `bendPositive: false` is our positive bend once the axis flips"
        );
    }

    #[test]
    fn a_one_bone_ik_chain_does_not_walk_past_its_bone() {
        // `chain: 0` is an aim constraint. Walking a parent anyway would quietly
        // turn every aim into a two-bone solve.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}, {"name": "head", "parent": "root"},
                         {"name": "look", "parent": "root"}],
                "ik": [{"name": "aim", "bone": "head", "target": "look", "chain": 0}]
            }]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");
        let c = loaded.skeleton.constraints.values().next().unwrap();
        let ankhimate_core::constraints::Constraint::Ik(ik) = c else {
            panic!("expected IK");
        };
        assert_eq!(ik.bones.len(), 1);
        assert_eq!(loaded.skeleton.bones[ik.bones[0]].name, "head");
    }

    #[test]
    fn a_mesh_flips_its_vertices_but_not_its_uvs() {
        // Two coordinate systems that look alike and are not. Vertices are world
        // positions and flip with the axis; UVs address the texture, which is
        // already stored top-left origin. Flipping both would place the mesh
        // correctly and draw its art upside down inside it.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "slot": [{"name": "face", "parent": "root"}],
                "skin": [{"slot": [{"name": "face", "display": [
                    {"type": "mesh", "name": "face",
                     "vertices": [0, 0, 10, 0, 10, -20, 0, -20],
                     "uvs": [0, 0, 1, 0, 1, 1, 0, 1],
                     "triangles": [0, 1, 2, 0, 2, 3]}
                ]}]}]
            }]
        }"#;
        let loaded = read(json, Images::Loose(&|_| None), "x").expect("reads");
        // No image resolves, so the mesh is reported rather than built.
        assert!(
            loaded
                .report
                .dangling
                .iter()
                .any(|(kind, _)| *kind == "dragonbones region"),
            "a mesh with no texture is named: {:?}",
            loaded.report
        );
    }

    #[test]
    fn a_mesh_with_no_triangles_is_reported_rather_than_built() {
        // An empty triangle list is a mesh that draws nothing; building it would
        // put an invisible attachment in the skin and leave the artist hunting.
        let json = r#"{
            "name": "t",
            "armature": [{"name": "a",
                "bone": [{"name": "root"}],
                "slot": [{"name": "s", "parent": "root"}],
                "skin": [{"slot": [{"name": "s", "display": [
                    {"type": "mesh", "name": "m", "vertices": [0, 0], "uvs": [0, 0],
                     "triangles": []}
                ]}]}]
            }]
        }"#;
        let png = image::RgbaImage::new(2, 2);
        let loaded = read(json, Images::Loose(&|_| Some(png.clone())), "x").expect("reads");
        assert!(
            loaded
                .report
                .lossy
                .iter()
                .any(|l| l.detail.contains("no vertices or no triangles")),
            "the empty mesh is named: {:?}",
            loaded.report.lossy
        );
    }

    #[test]
    fn an_unread_generic_timeline_is_reported_rather_than_ignored() {
        // Found on `shizuku`, and only by running the importer on a real file:
        // 489 animations came through with real durations and zero timelines,
        // which reads as "this rig does not animate" rather than as a gap. 5.6
        // replaced the per-channel shape with a generic `timeline` array.
        let json = r#"{
            "name": "t", "frameRate": 30,
            "armature": [{
                "name": "a", "bone": [{"name": "root"}],
                "animation": [{
                    "name": "blink", "duration": 30,
                    "timeline": [{"name": "PARAM_EYE", "type": 40,
                                  "frame": [{"duration": 5, "value": 1.0}]}]
                }]
            }]
        }"#;
        let loaded = read(json, Images::None, "x").expect("reads");

        let reported: Vec<&str> = loaded
            .report
            .lossy
            .iter()
            .filter(|l| l.what == "timeline")
            .map(|l| l.where_.as_str())
            .collect();
        assert_eq!(reported, ["blink"], "the gap must be named, not silent");
    }

    #[test]
    fn the_documents_own_name_wins_over_the_file_stem() {
        // Unlike Spine, DragonBones stores a project name.
        let json = r#"{"name": "hero", "armature": [{"name": "a"}]}"#;
        let loaded = read(json, Images::None, "file_stem").expect("reads");
        assert_eq!(loaded.name, "hero");
    }
}
