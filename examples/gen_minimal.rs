//! Regenerate the checked-in golden `samples/minimal.ankh`.
//!
//! Run with `cargo run -p ankhimate-formats --example gen_minimal`. The golden is
//! committed; this exists so the file can be rebuilt deterministically if the
//! schema changes, not so it is generated at test time.
//!
//! The document matches the T-108 acceptance shape: 2 bones, 1 slot, 1 skin (with
//! a region attachment), 1 animation, plus an unknown top-level JSON field that a
//! round-trip must preserve.

use ankhimate_core::animation::{Animation, Key, Timeline};
use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::slot::Slot;
use ankhimate_core::slotmap::SlotMap;

fn main() {
    let (skeleton, animations) = build();

    // Serialize through the normal path, then splice an unknown top-level field
    // in so the golden exercises forward-compat preservation.
    let assets = ankhimate_core::assets::AssetDb::new();
    let json = ankhimate_formats::to_json(&ankhimate_formats::ProjectRef {
        skeleton: &skeleton,
        animations: &animations,
        assets: &assets,
        name: "minimal",
        fps: 30,
    })
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["editor_note"] = serde_json::json!("hand-added unknown field; must survive round-trips");
    let json = serde_json::to_string_pretty(&value).unwrap();

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/minimal.ankh");
    ankhimate_formats::container::write(std::path::Path::new(path), &json, &[]).unwrap();
    println!("wrote {path}");
}

fn build() -> (
    Skeleton,
    SlotMap<ankhimate_core::ids::AnimationId, Animation>,
) {
    let mut skel = Skeleton::new();

    let root = skel.add_bone(Bone {
        name: "root".into(),
        parent: None,
        length: 40.0,
        local_transform: Transform::default(),
        inherit: Default::default(),
        color: Bone::default_color(),
    });
    let arm = skel.add_bone(Bone {
        name: "arm".into(),
        parent: Some(root),
        length: 30.0,
        local_transform: Transform {
            position: glam::vec2(40.0, 0.0),
            rotation: 15.0_f32.to_radians(),
            scale: glam::vec2(1.0, 1.0),
            shear: glam::vec2(0.0, 0.0),
        },
        inherit: Default::default(),
        color: Bone::default_color(),
    });

    let slot = skel.add_slot(Slot {
        attachment: Some("arm_img".into()),
        ..Slot::new("arm_slot".into(), arm)
    });

    // Put a region attachment in the skeleton's existing default skin.
    let default_skin = skel.default_skin;
    skel.skins[default_skin].set(
        slot,
        "arm_img",
        Attachment::Region(RegionAttachment {
            texture: "arm.png".into(),
            local_offset: glam::vec2(0.0, 0.0),
            local_rotation: 0.0,
            local_scale: glam::vec2(1.0, 1.0),
            width: 64.0,
            height: 32.0,
            uv_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            pivot: glam::Vec2::splat(0.5),
            sequence: None,
        }),
    );

    let mut animations = SlotMap::with_key();
    animations.insert(Animation {
        name: "wave".into(),
        duration: 1.0,
        timelines: vec![Timeline::BoneRotate {
            bone: arm,
            keys: vec![
                Key::linear(0.0, 0.0),
                Key::linear(0.5, 30.0),
                Key::linear(1.0, 0.0),
            ],
        }],
        events: Vec::new(),
        looping: true,
    });

    (skel, animations)
}
