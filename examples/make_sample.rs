//! Generate `samples/walker.ankh` — a small rigged character with a walk cycle.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p ankhimate-formats --example make_sample
//! ```
//!
//! Everything, art included, is produced here rather than checked in: the point
//! of the sample is to be *readable*, and a generated rig can be read as code.
//! It is also the only rig in the repo that exercises bones, slots, skins,
//! draw order, keyframes, an IK constraint and an event together, which makes it
//! a decent smoke test for the whole pipeline.

use ankhimate_core::animation::{Animation, EventKey, Interp, Key, Timeline};
use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
use ankhimate_core::constraints::{Constraint, IkConstraint};
use ankhimate_core::ids::{AnimationId, BoneId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slot::Slot;
use ankhimate_core::slotmap::SlotMap;
use ankhimate_core::transforms::Inherit;
use ankhimate_formats::convert::ProjectRef;

/// A flat-coloured rounded rectangle, as PNG bytes.
///
/// Deliberately plain: the sample is about the rig, and detailed art would make
/// it harder to see which part is which while posing.
fn part(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let radius = (width.min(height) as f32 * 0.35) as i32;
    let mut img = image::RgbaImage::new(width, height);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let (x, y) = (x as i32, y as i32);
        let (w, h) = (width as i32, height as i32);
        // Distance outside the rounded-rect's inner box, for a cheap corner cut.
        let dx = (radius - x).max(x - (w - 1 - radius)).max(0);
        let dy = (radius - y).max(y - (h - 1 - radius)).max(0);
        let outside = ((dx * dx + dy * dy) as f32).sqrt() - radius as f32;
        // One pixel of feather, so edges are not stair-stepped.
        let alpha = (1.0 - outside).clamp(0.0, 1.0);
        *pixel = image::Rgba([rgb[0], rgb[1], rgb[2], (alpha * 255.0) as u8]);
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encoding a generated image cannot fail");
    bytes
}

/// Add a bone, returning its id.
fn bone(
    skel: &mut Skeleton,
    name: &str,
    parent: Option<BoneId>,
    position: glam::Vec2,
    degrees: f32,
    length: f32,
) -> BoneId {
    skel.add_bone(Bone {
        name: name.to_string(),
        parent,
        length,
        local_transform: Transform {
            position,
            rotation: degrees.to_radians(),
            ..Transform::default()
        },
        inherit: Inherit::default(),
        color: Bone::default_color(),
    })
}

/// Attach a generated image to a bone through a slot, centred on the bone with
/// its pivot at the base — the end that should stay put when the bone turns.
#[allow(clippy::too_many_arguments)]
fn limb(
    skel: &mut Skeleton,
    assets: &mut AssetDb,
    name: &str,
    attached_to: BoneId,
    size: (f32, f32),
    rgb: [u8; 3],
    pivot: glam::Vec2,
    offset: glam::Vec2,
) {
    let (w, h) = size;
    assets.add(ImageAsset::new(
        name,
        part(w as u32, h as u32, rgb),
        w as u32,
        h as u32,
    ));
    let slot = skel.add_slot(Slot {
        attachment: Some(name.to_string()),
        ..Slot::new(format!("{name}_slot"), attached_to)
    });
    let skin = skel.default_skin;
    skel.skins[skin].set(
        slot,
        name.to_string(),
        Attachment::Region(RegionAttachment {
            texture: name.to_string(),
            local_offset: offset,
            local_rotation: 0.0,
            local_scale: glam::Vec2::ONE,
            width: w,
            height: h,
            uv_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            pivot,
        }),
    );
}

/// Rotation keys, in degrees, as `(time, angle)` pairs.
fn rotate(bone: BoneId, keys: &[(f32, f32)]) -> Timeline {
    Timeline::BoneRotate {
        bone,
        keys: keys
            .iter()
            .map(|(time, degrees)| Key {
                time: *time,
                value: *degrees,
                interp: Interp::Linear,
            })
            .collect(),
    }
}

fn main() {
    let mut skel = Skeleton::new();
    let mut assets = AssetDb::new();

    // ── Skeleton ─────────────────────────────────────────────────────────
    // Y is up. The root sits at the hips; everything hangs off it.
    let root = bone(&mut skel, "root", None, glam::vec2(0.0, 0.0), 0.0, 40.0);
    let torso = bone(
        &mut skel,
        "torso",
        Some(root),
        glam::vec2(0.0, 0.0),
        90.0,
        90.0,
    );
    let head = bone(
        &mut skel,
        "head",
        Some(torso),
        glam::vec2(90.0, 0.0),
        0.0,
        50.0,
    );

    // Arms hang from the top of the torso. Their local X runs down the limb, so
    // -90° from the torso's frame points them at the floor.
    let arm_l = bone(
        &mut skel,
        "arm_l",
        Some(torso),
        glam::vec2(80.0, 0.0),
        -100.0,
        60.0,
    );
    let forearm_l = bone(
        &mut skel,
        "forearm_l",
        Some(arm_l),
        glam::vec2(60.0, 0.0),
        10.0,
        55.0,
    );
    let arm_r = bone(
        &mut skel,
        "arm_r",
        Some(torso),
        glam::vec2(80.0, 0.0),
        -80.0,
        60.0,
    );
    let forearm_r = bone(
        &mut skel,
        "forearm_r",
        Some(arm_r),
        glam::vec2(60.0, 0.0),
        10.0,
        55.0,
    );

    let thigh_l = bone(
        &mut skel,
        "thigh_l",
        Some(root),
        glam::vec2(0.0, 0.0),
        -80.0,
        70.0,
    );
    let shin_l = bone(
        &mut skel,
        "shin_l",
        Some(thigh_l),
        glam::vec2(70.0, 0.0),
        5.0,
        65.0,
    );
    let thigh_r = bone(
        &mut skel,
        "thigh_r",
        Some(root),
        glam::vec2(0.0, 0.0),
        -100.0,
        70.0,
    );
    let shin_r = bone(
        &mut skel,
        "shin_r",
        Some(thigh_r),
        glam::vec2(70.0, 0.0),
        5.0,
        65.0,
    );

    // ── Art ──────────────────────────────────────────────────────────────
    // Back limbs first: draw order is creation order, and the far arm and leg
    // belong behind the body.
    let skin_tone = [232, 190, 160];
    let cloth = [70, 110, 180];
    let dark_cloth = [50, 80, 140];

    let base = glam::vec2(0.0, 0.5); // pivot at the limb's top edge, centred
    limb(
        &mut skel,
        &mut assets,
        "arm_far",
        arm_r,
        (26.0, 62.0),
        dark_cloth,
        base,
        glam::vec2(30.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "forearm_far",
        forearm_r,
        (24.0, 58.0),
        dark_cloth,
        base,
        glam::vec2(27.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "thigh_far",
        thigh_r,
        (30.0, 72.0),
        dark_cloth,
        base,
        glam::vec2(35.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "shin_far",
        shin_r,
        (26.0, 68.0),
        dark_cloth,
        base,
        glam::vec2(32.0, 0.0),
    );

    limb(
        &mut skel,
        &mut assets,
        "torso",
        torso,
        (70.0, 95.0),
        cloth,
        glam::vec2(0.5, 0.1),
        glam::vec2(45.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "head",
        head,
        (64.0, 64.0),
        skin_tone,
        glam::vec2(0.5, 0.15),
        glam::vec2(30.0, 0.0),
    );

    limb(
        &mut skel,
        &mut assets,
        "thigh_near",
        thigh_l,
        (32.0, 72.0),
        cloth,
        base,
        glam::vec2(35.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "shin_near",
        shin_l,
        (28.0, 68.0),
        cloth,
        base,
        glam::vec2(32.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "arm_near",
        arm_l,
        (28.0, 62.0),
        cloth,
        base,
        glam::vec2(30.0, 0.0),
    );
    limb(
        &mut skel,
        &mut assets,
        "forearm_near",
        forearm_l,
        (26.0, 58.0),
        skin_tone,
        base,
        glam::vec2(27.0, 0.0),
    );

    // ── An IK target for the near leg ────────────────────────────────────
    // Unparented and at the foot, so switching it on does not move the rig.
    // Drag this bone in the editor and the whole leg follows.
    let foot_target = bone(
        &mut skel,
        "foot_target_l",
        None,
        glam::vec2(20.0, -125.0),
        0.0,
        0.0,
    );
    // Live, so dragging the target in Setup bends the leg immediately — an IK
    // rig that does nothing until you find a slider is not a demonstration.
    let leg_ik = skel.add_constraint(Constraint::Ik(IkConstraint {
        bones: vec![thigh_l, shin_l],
        mix: 1.0,
        ..IkConstraint::two_bone("leg_ik_l", foot_target, [thigh_l, shin_l])
    }));

    // ── A walk cycle ─────────────────────────────────────────────────────
    // Keys are *offsets from the setup pose*, so 0 means "as rigged".
    let mut animations: SlotMap<AnimationId, Animation> = SlotMap::with_key();
    animations.insert(Animation {
        name: "walk".into(),
        duration: 1.0,
        looping: true,
        events: vec![
            EventKey {
                time: 0.0,
                name: "footstep".into(),
                int_value: 0,
                float_value: 1.0,
                string_value: "left".into(),
            },
            EventKey {
                time: 0.5,
                name: "footstep".into(),
                int_value: 1,
                float_value: 1.0,
                string_value: "right".into(),
            },
        ],
        timelines: vec![
            // Legs, half a cycle apart.
            rotate(thigh_l, &[(0.0, 25.0), (0.5, -25.0), (1.0, 25.0)]),
            rotate(
                shin_l,
                &[(0.0, -10.0), (0.25, -35.0), (0.5, 0.0), (1.0, -10.0)],
            ),
            rotate(thigh_r, &[(0.0, -25.0), (0.5, 25.0), (1.0, -25.0)]),
            rotate(
                shin_r,
                &[(0.0, 0.0), (0.5, -10.0), (0.75, -35.0), (1.0, 0.0)],
            ),
            // Arms swing opposite the legs.
            rotate(arm_l, &[(0.0, -20.0), (0.5, 20.0), (1.0, -20.0)]),
            rotate(forearm_l, &[(0.0, -15.0), (0.5, -5.0), (1.0, -15.0)]),
            rotate(arm_r, &[(0.0, 20.0), (0.5, -20.0), (1.0, 20.0)]),
            rotate(forearm_r, &[(0.0, -5.0), (0.5, -15.0), (1.0, -5.0)]),
            // The body bobs twice per cycle — once per footfall.
            Timeline::BoneTranslate {
                bone: root,
                keys: [
                    (0.0, 0.0),
                    (0.25, -6.0),
                    (0.5, 0.0),
                    (0.75, -6.0),
                    (1.0, 0.0),
                ]
                .iter()
                .map(|(time, y)| Key {
                    time: *time,
                    value: glam::vec2(0.0, *y),
                    interp: Interp::Linear,
                })
                .collect(),
            },
            rotate(torso, &[(0.0, -3.0), (0.5, 3.0), (1.0, -3.0)]),
            rotate(head, &[(0.0, 3.0), (0.5, -3.0), (1.0, 3.0)]),
            // The walk is hand-keyed FK, so the constraint stands down for it.
            // Keying the mix rather than authoring it off is the point: the same
            // rig does IK when posed and FK when played.
            Timeline::IkMix {
                constraint: leg_ik,
                keys: vec![Key {
                    time: 0.0,
                    value: 0.0,
                    interp: Interp::Stepped,
                }],
            },
        ],
    });

    // A second clip that *is* driven by IK: the foot target traces a step and
    // the leg works out its own angles. Compare the two dopesheets — this one
    // keys one bone where the walk keys four.
    animations.insert(Animation {
        name: "leg_ik".into(),
        duration: 1.0,
        looping: true,
        events: Vec::new(),
        timelines: vec![Timeline::BoneTranslate {
            bone: foot_target,
            keys: [
                (0.0, glam::vec2(0.0, 0.0)),
                (0.25, glam::vec2(45.0, 35.0)),
                (0.5, glam::vec2(80.0, 0.0)),
                (0.75, glam::vec2(40.0, -6.0)),
                (1.0, glam::vec2(0.0, 0.0)),
            ]
            .iter()
            .map(|(time, offset)| Key {
                time: *time,
                value: *offset,
                interp: Interp::Linear,
            })
            .collect(),
        }],
    });

    // ── Write it ─────────────────────────────────────────────────────────
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("samples")
        .join("walker.ankh");
    ankhimate_formats::save(
        &path,
        &ProjectRef {
            skeleton: &skel,
            animations: &animations,
            assets: &assets,
            name: "walker",
            fps: 30,
        },
        &[],
    )
    .expect("writing the sample");

    println!("wrote {}", path.display());
    println!(
        "  {} bones, {} slots, {} images, {} animations",
        skel.bones.len(),
        skel.slots.len(),
        assets.images.len(),
        animations.len()
    );
}
