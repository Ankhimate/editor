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
//! A bone's transform carries `skX`/`skY` rather than a rotation. When they are
//! equal it is pure rotation; when they differ, the difference *is* shear. See
//! [`decompose_skew`], which also applies the negation above.
//!
//! # A file holds several armatures
//!
//! Spine is one skeleton per file. DragonBones packs several — `mecha_1004d`
//! ships four, three of them swappable weapons. Our `Document` holds one
//! skeleton, so the first is imported and the rest are reported: dropping them
//! silently would lose most of a file that looked like it loaded.
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
/// Shear lands entirely on X because that is where the asymmetry lives:
/// `skY - skX` is the angle by which the axes stop being perpendicular, and
/// `Transform::shear.y` measured against the same rotation would double-count
/// it. A rig with `skX == skY` — every rigid bone, which is nearly all of them —
/// comes through with zero shear either way.
pub fn decompose_skew(sk_x_deg: f32, sk_y_deg: f32) -> (f32, glam::Vec2) {
    let rotation = -sk_y_deg.to_radians();
    let shear_x = -(sk_x_deg - sk_y_deg).to_radians();
    (rotation, glam::vec2(shear_x, 0.0))
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
    // One skeleton per document. The rest are named in the report rather than
    // dropped quietly — in `mecha_1004d` three of four armatures are swappable
    // weapons, and a silent import would look like most of the file vanished.
    let armature = &armatures[0];
    for extra in &armatures[1..] {
        report.lossy(
            "armature",
            s(extra, "name").unwrap_or("unnamed"),
            "only the first armature in a file is imported; this one was skipped",
        );
    }

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
        let id = skel.add_slot(Slot::new(slot_name.to_string(), bone_id));
        slots.insert(slot_name.to_string(), id);
    }

    // ── Attachments ──────────────────────────────────────────────────────
    let default_skin = skel.default_skin;
    let mut decoded: HashMap<String, (u32, u32)> = HashMap::new();
    let mut crop = |region: &str, assets: &mut AssetDb| -> Option<String> {
        if decoded.contains_key(region) {
            return Some(region.to_string());
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
        Some(region.to_string())
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
            for display in entry
                .get("display")
                .and_then(|d| d.as_array())
                .unwrap_or(&empty)
            {
                let Some(display_name) = s(display, "name") else {
                    continue;
                };
                // `path` names the atlas region when it differs from the
                // display's own name — same convention as Spine's `path`.
                let region_name = s(display, "path").unwrap_or(display_name);
                let kind = s(display, "type").unwrap_or("image");
                match kind {
                    "image" => {
                        let Some(asset) = crop(region_name, &mut assets) else {
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
                                width: 0.0,
                                height: 0.0,
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
                    other => {
                        // Meshes, bounding boxes and sub-armatures. Reported by
                        // kind so the count in the import report is actionable
                        // rather than a single opaque number.
                        report.lossy(
                            "attachment",
                            &format!("{slot_name}/{display_name}"),
                            match other {
                                "mesh" => "a mesh display is not read yet",
                                "boundingBox" => "a bounding box display is not read yet",
                                "armature" => "a nested armature display is not read yet",
                                _ => "an unrecognised display type was skipped",
                            },
                        );
                    }
                }
            }
        }
    }

    // ── Constraints ──────────────────────────────────────────────────────
    // IK lives on the armature rather than in a tagged list. Not built yet, but
    // named individually: "2 constraints skipped" is a number, "calf_l and
    // calf_r skipped" is something the file's author can act on.
    for ik in armature
        .get("ik")
        .and_then(|a| a.as_array())
        .unwrap_or(&empty)
    {
        report.lossy(
            "constraint",
            s(ik, "name").unwrap_or("unnamed"),
            "an IK constraint is not read yet",
        );
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
    fn unequal_skew_angles_become_rotation_plus_shear() {
        // The difference *is* the shear; discarding it would quietly square up
        // every deliberately skewed part. Both halves negate with the axis.
        let (rotation, shear) = decompose_skew(50.0, 20.0);
        assert!((rotation + 20.0_f32.to_radians()).abs() < 1e-6);
        assert!((shear.x + 30.0_f32.to_radians()).abs() < 1e-6);
        assert_eq!(shear.y, 0.0);
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
