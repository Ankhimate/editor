//! Conversion between the in-memory core model and the on-disk schema.
//!
//! Two things happen at this boundary and nowhere else:
//!
//! * **Ids ↔ names.** Core uses slotmap keys; disk uses names (ADR 0004). Saving
//!   resolves keys to names, loading builds fresh keys and resolves names back.
//! * **Radians ↔ degrees.** Core math is radians; the file is degrees (PLAN §2.7,
//!   ADR 0002).
//!
//! A name that fails to resolve on load is reported in [`LoadReport`] rather than
//! failing the whole load: a project with one dangling reference should still
//! open, minus that reference.

use crate::schema;
use ankhimate_core::animation as anim;
use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_core::attachment as att;
use ankhimate_core::constraints::{Constraint, IkConstraint, TransformConstraint};
use ankhimate_core::ids::{AnimationId, BoneId, ConstraintId, SkinId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::skin::Skin;
use ankhimate_core::slot::{BlendMode, Slot};
use ankhimate_core::slotmap::SlotMap;
use ankhimate_core::transforms::Inherit;
use std::collections::HashMap;

/// Everything needed to write a project, borrowed from the caller's document.
///
/// A struct rather than a five-argument function: the list grows with every
/// document-level feature (assets here, constraints and events later), and
/// positional `&str, u32` tails are exactly how a name and an fps end up
/// swapped.
pub struct ProjectRef<'a> {
    pub skeleton: &'a Skeleton,
    pub animations: &'a SlotMap<AnimationId, anim::Animation>,
    pub assets: &'a AssetDb,
    pub name: &'a str,
    pub fps: u32,
}

/// A loaded document, plus anything that could not be resolved.
pub struct Loaded {
    pub skeleton: Skeleton,
    pub animations: SlotMap<AnimationId, anim::Animation>,
    /// Assets with their metadata. Bytes are empty until the container binds
    /// them ([`crate::load`]) — `project.json` holds the index, not the pixels.
    pub assets: AssetDb,
    pub name: String,
    pub fps: u32,
    pub report: LoadReport,
}

/// Non-fatal problems encountered while loading.
#[derive(Debug, Default, PartialEq)]
pub struct LoadReport {
    /// References to entities that were not in the file (`what`, `name`).
    pub dangling: Vec<(&'static str, String)>,
}

impl LoadReport {
    fn dangling(&mut self, what: &'static str, name: &str) {
        self.dangling.push((what, name.to_string()));
    }

    pub fn is_clean(&self) -> bool {
        self.dangling.is_empty()
    }
}

fn blend_mode_name(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "normal",
        BlendMode::Additive => "additive",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
    }
}

fn blend_mode_from(name: &str) -> BlendMode {
    match name {
        "additive" => BlendMode::Additive,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        // Unknown or empty: the sane default rather than a hard failure.
        _ => BlendMode::Normal,
    }
}

fn flatten_vec2(values: &[glam::Vec2]) -> Vec<f32> {
    values.iter().flat_map(|v| [v.x, v.y]).collect()
}

fn unflatten_vec2(values: &[f32]) -> Vec<glam::Vec2> {
    values
        .chunks_exact(2)
        .map(|c| glam::vec2(c[0], c[1]))
        .collect()
}

fn interp_to_schema(interp: anim::Interp) -> schema::Interp {
    match interp {
        anim::Interp::Linear => schema::Interp::Linear,
        anim::Interp::Stepped => schema::Interp::Stepped,
        anim::Interp::Bezier {
            out_handle,
            in_handle,
        } => schema::Interp::Bezier {
            handles: [out_handle.x, out_handle.y, in_handle.x, in_handle.y],
        },
    }
}

fn interp_from_schema(interp: schema::Interp) -> anim::Interp {
    match interp {
        schema::Interp::Linear => anim::Interp::Linear,
        schema::Interp::Stepped => anim::Interp::Stepped,
        schema::Interp::Bezier { handles } => anim::Interp::Bezier {
            out_handle: glam::vec2(handles[0], handles[1]),
            in_handle: glam::vec2(handles[2], handles[3]),
        },
    }
}

// ── Save: core → schema ─────────────────────────────────────────────────────

/// The container-relative file name for an asset's bytes.
///
/// The extension is sniffed from the magic bytes rather than stored: the format
/// only matters to whoever decodes the file, and a wrong extension on a
/// correctly-encoded file is a lie a future importer would trip over.
pub fn asset_file_name(asset: &ImageAsset) -> String {
    let ext = if asset.bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if asset.bytes.starts_with(&[0xFF, 0xD8]) {
        "jpg"
    } else if asset.bytes.len() > 12
        && &asset.bytes[0..4] == b"RIFF"
        && &asset.bytes[8..12] == b"WEBP"
    {
        "webp"
    } else {
        "bin"
    };
    format!("{}.{ext}", asset.name)
}

/// Build the on-disk project from the in-memory document.
pub fn to_schema(project: &ProjectRef<'_>) -> schema::Project {
    let ProjectRef {
        skeleton,
        animations,
        assets,
        name,
        fps,
    } = *project;

    let bone_name = |id: BoneId| {
        skeleton
            .bones
            .get(id)
            .map(|b| b.name.clone())
            .unwrap_or_default()
    };
    let slot_name = |id: SlotId| {
        skeleton
            .slots
            .get(id)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    };
    let constraint_name = |id: ConstraintId| {
        skeleton
            .constraints
            .get(id)
            .map(|c| c.name().to_string())
            .unwrap_or_default()
    };

    // Bones in `update_order` so a load can insert parents before children.
    let bones = skeleton
        .update_order
        .iter()
        .filter_map(|&id| skeleton.bones.get(id).map(|b| (id, b)))
        .map(|(_, b)| schema::Bone {
            name: b.name.clone(),
            parent: b.parent.map(bone_name).unwrap_or_default(),
            length: b.length,
            tx: b.local_transform.position.x,
            ty: b.local_transform.position.y,
            rotation: b.local_transform.rotation.to_degrees(),
            sx: b.local_transform.scale.x,
            sy: b.local_transform.scale.y,
            shear_x: b.local_transform.shear.x.to_degrees(),
            shear_y: b.local_transform.shear.y.to_degrees(),
            inherit_rotation: b.inherit.rotation,
            inherit_scale: b.inherit.scale,
            inherit_reflect: b.inherit.reflect,
            color: Some(b.color),
            extra: Default::default(),
        })
        .collect();

    let slots = skeleton
        .slots
        .iter()
        .map(|(_, s)| schema::Slot {
            name: s.name.clone(),
            bone: bone_name(s.bone),
            attachment: s.attachment.clone(),
            color: s.color,
            dark_color: s.dark_color,
            blend_mode: blend_mode_name(s.blend_mode).to_string(),
            extra: Default::default(),
        })
        .collect();

    let skins = skeleton
        .skins
        .iter()
        .map(|(_, skin)| {
            // Sort entries so the written file is byte-stable across runs
            // (`HashMap` iteration order is not).
            let mut entries: Vec<schema::SkinEntry> = skin
                .entries
                .iter()
                .map(|((slot, att_name), attachment)| schema::SkinEntry {
                    slot: slot_name(*slot),
                    name: att_name.clone(),
                    attachment: attachment_to_schema(skeleton, attachment),
                })
                .collect();
            entries.sort_by(|a, b| (&a.slot, &a.name).cmp(&(&b.slot, &b.name)));
            schema::Skin {
                name: skin.name.clone(),
                entries,
                bones: skin.bones.iter().map(|b| bone_name(*b)).collect(),
                constraints: skin
                    .constraints
                    .iter()
                    .map(|c| constraint_name(*c))
                    .collect(),
                extra: Default::default(),
            }
        })
        .collect();

    let constraints = skeleton
        .constraints
        .iter()
        .map(|(_, c)| match c {
            Constraint::Ik(ik) => schema::Constraint {
                name: ik.name.clone(),
                kind: "ik".to_string(),
                target: bone_name(ik.target),
                bones: ik.bones.iter().copied().map(bone_name).collect(),
                bend_direction: ik.bend_direction,
                mix: ik.mix,
                softness: ik.softness,
                stretch: ik.stretch,
                stretch_limit: ik.stretch_limit,
                stiffness: ik.stiffness,
                mixes: None,
                offsets: None,
                local: false,
                relative: false,
                physics: None,
                forces: None,
                channels: None,
                slot: None,
                path: None,
                extra: Default::default(),
            },
            Constraint::Transform(tc) => schema::Constraint {
                name: tc.name.clone(),
                kind: "transform".to_string(),
                target: bone_name(tc.target),
                bones: tc.bones.iter().copied().map(bone_name).collect(),
                bend_direction: 1.0,
                mix: 1.0,
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
                stiffness: 0.0,
                mixes: Some([tc.mix_rotate, tc.mix_translate, tc.mix_scale, tc.mix_shear]),
                // Angles are degrees at the document boundary (ADR 0002), the
                // same as every rotation key.
                offsets: Some([
                    tc.offsets.position.x,
                    tc.offsets.position.y,
                    tc.offsets.rotation.to_degrees(),
                    tc.offsets.scale.x,
                    tc.offsets.scale.y,
                    tc.offsets.shear.x.to_degrees(),
                    tc.offsets.shear.y.to_degrees(),
                ]),
                local: tc.local,
                relative: tc.relative,
                physics: None,
                forces: None,
                channels: None,
                slot: None,
                path: None,
                extra: Default::default(),
            },
            Constraint::Physics(p) => schema::Constraint {
                name: p.name.clone(),
                kind: "physics".to_string(),
                // A physics constraint has no separate target; the bone it
                // simulates goes in `target` so the schema shape stays uniform
                // and `bones` stays the "chain" field.
                target: bone_name(p.bone),
                bones: Vec::new(),
                bend_direction: 1.0,
                mix: p.mix,
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
                stiffness: 0.0,
                mixes: None,
                offsets: None,
                local: false,
                relative: false,
                physics: Some([p.inertia, p.strength, p.damping, p.mass]),
                forces: Some([p.wind.x, p.wind.y, p.gravity.x, p.gravity.y]),
                channels: Some([p.rotate, p.translate]),
                slot: None,
                path: None,
                extra: Default::default(),
            },
            Constraint::Path(p) => schema::Constraint {
                name: p.name.clone(),
                kind: "path".to_string(),
                // A path constraint has no target bone; its source is a slot.
                target: String::new(),
                bones: p.bones.iter().copied().map(bone_name).collect(),
                bend_direction: 1.0,
                mix: 1.0,
                softness: 0.0,
                stretch: false,
                stretch_limit: 1.1,
                stiffness: 0.0,
                mixes: None,
                offsets: None,
                local: false,
                relative: false,
                physics: None,
                forces: None,
                channels: None,
                slot: Some(slot_name(p.slot)),
                path: Some([p.position, p.spacing, p.mix_rotate, p.mix_translate]),
                extra: Default::default(),
            },
        })
        .collect();

    let animations = animations
        .iter()
        .map(|(_, a)| schema::Animation {
            name: a.name.clone(),
            duration: a.duration,
            looping: a.looping,
            timelines: a
                .timelines
                .iter()
                .map(|t| timeline_to_schema(t, &bone_name, &slot_name, &constraint_name))
                .collect(),
            events: a
                .events
                .iter()
                .map(|e| schema::Event {
                    time: e.time,
                    name: e.name.clone(),
                    int_value: e.int_value,
                    float_value: e.float_value,
                    string_value: e.string_value.clone(),
                    audio: e.audio.clone(),
                    volume: e.volume,
                    balance: e.balance,
                })
                .collect(),
            markers: a
                .markers
                .iter()
                .map(|m| schema::Marker {
                    time: m.time,
                    name: m.name.clone(),
                    color: m.color,
                })
                .collect(),
            extra: Default::default(),
        })
        .collect();

    // Sorted by name so the written file is byte-stable regardless of slotmap
    // insertion order.
    let mut assets: Vec<schema::Asset> = assets
        .images
        .iter()
        .map(|(_, a)| schema::Asset {
            name: a.name.clone(),
            file: asset_file_name(a),
            width: a.width,
            height: a.height,
            source_path: a.source_path.clone(),
            extra: Default::default(),
        })
        .collect();
    assets.sort_by(|a, b| a.name.cmp(&b.name));

    schema::Project {
        version: schema::CURRENT_VERSION,
        name: name.to_string(),
        fps,
        assets,
        bones,
        slots,
        draw_order: skeleton.draw_order.iter().copied().map(slot_name).collect(),
        skins,
        default_skin: skeleton
            .skins
            .get(skeleton.default_skin)
            .map(|s| s.name.clone())
            .unwrap_or_default(),
        constraints,
        constraint_order: skeleton
            .constraint_order
            .iter()
            .copied()
            .map(constraint_name)
            .collect(),
        animations,
        selection_sets: skeleton
            .selection_sets
            .iter()
            .map(|set| schema::SelectionSet {
                name: set.name.clone(),
                bones: set.bones.iter().copied().map(bone_name).collect(),
            })
            .collect(),
        extra: Default::default(),
    }
}

/// Weights out: bone ids become names, which is what makes a file survive a
/// re-import (ADR 0004).
fn weights_to_schema(
    skeleton: &Skeleton,
    weights: &[Vec<att::VertexWeight>],
) -> Vec<Vec<(String, f32)>> {
    weights
        .iter()
        .map(|vertex| {
            vertex
                .iter()
                .map(|w| {
                    let name = skeleton
                        .bones
                        .get(w.bone)
                        .map(|b| b.name.clone())
                        .unwrap_or_default();
                    (name, w.weight)
                })
                .collect()
        })
        .collect()
}

/// Weights in. A weight naming a bone that is not in the file is dropped rather
/// than defaulted onto the root: a silently reparented influence is far harder to
/// spot than a missing one.
fn weights_from_schema(
    weights: &[Vec<(String, f32)>],
    bone_ids: &HashMap<String, BoneId>,
) -> Vec<Vec<att::VertexWeight>> {
    weights
        .iter()
        .map(|vertex| {
            vertex
                .iter()
                .filter_map(|(name, weight)| {
                    bone_ids.get(name).map(|&bone| att::VertexWeight {
                        bone,
                        weight: *weight,
                    })
                })
                .collect()
        })
        .collect()
}

fn sequence_to_schema(sequence: &att::Sequence) -> schema::Sequence {
    schema::Sequence {
        frames: sequence.frames.clone(),
        fps: sequence.fps,
        mode: match sequence.mode {
            att::SequenceMode::Hold => "hold",
            att::SequenceMode::Once => "once",
            att::SequenceMode::Loop => "loop",
            att::SequenceMode::PingPong => "ping_pong",
            att::SequenceMode::OnceReverse => "once_reverse",
            att::SequenceMode::LoopReverse => "loop_reverse",
            att::SequenceMode::PingPongReverse => "ping_pong_reverse",
        }
        .to_string(),
        setup_index: sequence.setup_index,
    }
}

fn sequence_from_schema(sequence: &schema::Sequence) -> att::Sequence {
    att::Sequence {
        frames: sequence.frames.clone(),
        fps: sequence.fps,
        // An unknown mode holds rather than erroring: a newer file should still
        // open, showing one frame instead of nothing.
        mode: match sequence.mode.as_str() {
            "once" => att::SequenceMode::Once,
            "loop" => att::SequenceMode::Loop,
            "ping_pong" => att::SequenceMode::PingPong,
            "once_reverse" => att::SequenceMode::OnceReverse,
            "loop_reverse" => att::SequenceMode::LoopReverse,
            "ping_pong_reverse" => att::SequenceMode::PingPongReverse,
            _ => att::SequenceMode::Hold,
        },
        setup_index: sequence.setup_index,
    }
}

fn attachment_to_schema(skeleton: &Skeleton, attachment: &att::Attachment) -> schema::Attachment {
    match attachment {
        att::Attachment::Region(r) => schema::Attachment::Region(schema::Region {
            texture: r.texture.clone(),
            offset_x: r.local_offset.x,
            offset_y: r.local_offset.y,
            rotation: r.local_rotation.to_degrees(),
            scale_x: r.local_scale.x,
            scale_y: r.local_scale.y,
            width: r.width,
            height: r.height,
            uv: [r.uv_rect.x, r.uv_rect.y, r.uv_rect.w, r.uv_rect.h],
            pivot_x: r.pivot.x,
            pivot_y: r.pivot.y,
            sequence: r.sequence.as_ref().map(sequence_to_schema),
            extra: Default::default(),
        }),
        att::Attachment::Clipping(c) => schema::Attachment::Clipping(schema::Clipping {
            vertices: flatten_vec2(&c.vertices),
            end_slot: c.end_slot.clone(),
            extra: Default::default(),
        }),
        att::Attachment::Path(p) => schema::Attachment::Path(schema::Path {
            vertices: flatten_vec2(&p.vertices),
            closed: p.closed,
            constant_speed: p.constant_speed,
            extra: Default::default(),
        }),
        att::Attachment::Mesh(m) => schema::Attachment::Mesh(schema::Mesh {
            texture: m.texture.clone(),
            vertices: flatten_vec2(&m.setup_vertices),
            uvs: flatten_vec2(&m.uvs),
            triangles: m.triangles.iter().flat_map(|t| *t).collect(),
            edges: m.edges.iter().flat_map(|e| *e).collect(),
            weights: weights_to_schema(skeleton, &m.weights),
            linked: m.linked.as_ref().map(|l| schema::LinkedMesh {
                skin: l.skin.clone(),
                slot: l.slot.clone(),
                attachment: l.attachment.clone(),
                inherit_deform: l.inherit_deform,
            }),
            sequence: m.sequence.as_ref().map(sequence_to_schema),
            extra: Default::default(),
        }),
        att::Attachment::BoundingBox(b) => schema::Attachment::BoundingBox(schema::BoundingBox {
            vertices: flatten_vec2(&b.vertices),
            weights: weights_to_schema(skeleton, &b.weights),
            extra: Default::default(),
        }),
        att::Attachment::Point(p) => schema::Attachment::Point(schema::Point {
            x: p.position.x,
            y: p.position.y,
            rotation: p.rotation.to_degrees(),
            extra: Default::default(),
        }),
    }
}

fn timeline_to_schema(
    timeline: &anim::Timeline,
    bone_name: &impl Fn(BoneId) -> String,
    slot_name: &impl Fn(SlotId) -> String,
    constraint_name: &impl Fn(ConstraintId) -> String,
) -> schema::Timeline {
    let vec2_keys = |keys: &[anim::Key<glam::Vec2>], to_degrees: bool| -> Vec<schema::Vec2Key> {
        keys.iter()
            .map(|k| schema::Vec2Key {
                time: k.time,
                x: if to_degrees {
                    k.value.x.to_degrees()
                } else {
                    k.value.x
                },
                y: if to_degrees {
                    k.value.y.to_degrees()
                } else {
                    k.value.y
                },
                interp: interp_to_schema(k.interp),
            })
            .collect()
    };
    let scalar_keys = |keys: &[anim::Key<f32>]| -> Vec<schema::ScalarKey> {
        keys.iter()
            .map(|k| schema::ScalarKey {
                time: k.time,
                value: k.value,
                interp: interp_to_schema(k.interp),
            })
            .collect()
    };

    match timeline {
        anim::Timeline::BoneTranslate { bone, keys } => schema::Timeline::BoneTranslate {
            bone: bone_name(*bone),
            keys: vec2_keys(keys, false),
        },
        // Rotation keys are already degrees in core (PLAN §2.7).
        anim::Timeline::BoneRotate { bone, keys } => schema::Timeline::BoneRotate {
            bone: bone_name(*bone),
            keys: scalar_keys(keys),
        },
        anim::Timeline::BoneScale { bone, keys } => schema::Timeline::BoneScale {
            bone: bone_name(*bone),
            keys: vec2_keys(keys, false),
        },
        // Shear keys are degrees on both sides — no conversion, same as rotate.
        anim::Timeline::BoneShear { bone, keys } => schema::Timeline::BoneShear {
            bone: bone_name(*bone),
            keys: vec2_keys(keys, false),
        },
        anim::Timeline::SlotColor { slot, keys } => schema::Timeline::SlotColor {
            slot: slot_name(*slot),
            keys: keys
                .iter()
                .map(|k| schema::ColorKey {
                    time: k.time,
                    value: k.value,
                    interp: interp_to_schema(k.interp),
                })
                .collect(),
        },
        anim::Timeline::SlotVisible { slot, keys } => schema::Timeline::SlotVisible {
            slot: slot_name(*slot),
            keys: keys
                .iter()
                .map(|k| schema::VisibleKey {
                    time: k.time,
                    value: k.value,
                })
                .collect(),
        },
        anim::Timeline::SlotAttachment { slot, keys } => schema::Timeline::SlotAttachment {
            slot: slot_name(*slot),
            keys: keys
                .iter()
                .map(|k| schema::AttachmentKey {
                    time: k.time,
                    value: k.value.clone(),
                })
                .collect(),
        },
        anim::Timeline::DrawOrder { keys } => schema::Timeline::DrawOrder {
            keys: keys
                .iter()
                .map(|k| schema::DrawOrderKey {
                    time: k.time,
                    offsets: k
                        .value
                        .iter()
                        .map(|(slot, delta)| (slot_name(*slot), *delta))
                        .collect(),
                })
                .collect(),
        },
        anim::Timeline::IkMix { constraint, keys } => schema::Timeline::IkMix {
            constraint: constraint_name(*constraint),
            keys: scalar_keys(keys),
        },
        anim::Timeline::IkBendDirection { constraint, keys } => schema::Timeline::IkBendDirection {
            constraint: constraint_name(*constraint),
            keys: scalar_keys(keys),
        },
        anim::Timeline::IkSoftness { constraint, keys } => schema::Timeline::IkSoftness {
            constraint: constraint_name(*constraint),
            keys: scalar_keys(keys),
        },
        anim::Timeline::TransformConstraintMix { constraint, keys } => {
            schema::Timeline::TransformConstraintMix {
                constraint: constraint_name(*constraint),
                keys: keys
                    .iter()
                    .map(|k| schema::ColorKey {
                        time: k.time,
                        value: k.value,
                        interp: interp_to_schema(k.interp),
                    })
                    .collect(),
            }
        }
        anim::Timeline::Deform {
            slot,
            attachment,
            keys,
        } => schema::Timeline::Deform {
            slot: slot_name(*slot),
            attachment: attachment.clone(),
            keys: keys
                .iter()
                .map(|k| schema::DeformKey {
                    time: k.time,
                    offsets: flatten_vec2(&k.value),
                    interp: interp_to_schema(k.interp),
                })
                .collect(),
        },
    }
}

// ── Load: schema → core ─────────────────────────────────────────────────────

/// Rebuild the in-memory document from a parsed project.
pub fn from_schema(project: &schema::Project) -> Loaded {
    let mut report = LoadReport::default();
    let mut skeleton = Skeleton::default();

    // Assets first: attachments reference them by name, and the bytes are bound
    // afterwards by the container reader (`crate::load`).
    let mut assets = AssetDb::new();
    for a in &project.assets {
        assets.images.insert(ImageAsset {
            name: a.name.clone(),
            bytes: Vec::new(),
            width: a.width,
            height: a.height,
            source_path: a.source_path.clone(),
        });
    }

    // Bones, in file order. The writer emits `update_order`, so parents normally
    // precede children — but a hand-edited file might not, so parent links are
    // resolved in a second pass.
    let mut bone_ids: HashMap<String, BoneId> = HashMap::new();
    for b in &project.bones {
        let id = skeleton.bones.insert(Bone {
            name: b.name.clone(),
            parent: None,
            length: b.length,
            local_transform: Transform {
                position: glam::vec2(b.tx, b.ty),
                rotation: b.rotation.to_radians(),
                scale: glam::vec2(b.sx, b.sy),
                shear: glam::vec2(b.shear_x.to_radians(), b.shear_y.to_radians()),
            },
            inherit: Inherit {
                rotation: b.inherit_rotation,
                scale: b.inherit_scale,
                reflect: b.inherit_reflect,
            },
            color: b.color.unwrap_or_else(Bone::default_color),
        });
        bone_ids.insert(b.name.clone(), id);
    }
    for b in &project.bones {
        if b.parent.is_empty() {
            continue;
        }
        let Some(&child) = bone_ids.get(&b.name) else {
            continue;
        };
        match bone_ids.get(&b.parent) {
            Some(&parent) => {
                if let Some(bone) = skeleton.bones.get_mut(child) {
                    bone.parent = Some(parent);
                }
            }
            None => report.dangling("bone parent", &b.parent),
        }
    }
    skeleton.rebuild_update_order();

    // Slots.
    let mut slot_ids: HashMap<String, SlotId> = HashMap::new();
    for s in &project.slots {
        let Some(&bone) = bone_ids.get(&s.bone) else {
            report.dangling("slot bone", &s.bone);
            continue;
        };
        let id = skeleton.slots.insert(Slot {
            name: s.name.clone(),
            bone,
            attachment: s.attachment.clone(),
            color: s.color,
            dark_color: s.dark_color,
            blend_mode: blend_mode_from(&s.blend_mode),
        });
        slot_ids.insert(s.name.clone(), id);
    }

    // Draw order; any slot the file omitted is appended so it stays drawable.
    for name in &project.draw_order {
        match slot_ids.get(name) {
            Some(&id) => skeleton.draw_order.push(id),
            None => report.dangling("draw order slot", name),
        }
    }
    for (id, _) in skeleton.slots.iter() {
        if !skeleton.draw_order.contains(&id) {
            skeleton.draw_order.push(id);
        }
    }

    // Skins.
    // A skin's constraint list is resolved after the constraints themselves are
    // read, since a name is only an id once its constraint exists.
    let mut skin_constraint_names: Vec<(SkinId, Vec<String>)> = Vec::new();
    for s in &project.skins {
        let mut skin = Skin::new(&s.name);
        for entry in &s.entries {
            let Some(&slot) = slot_ids.get(&entry.slot) else {
                report.dangling("skin entry slot", &entry.slot);
                continue;
            };
            skin.set(
                slot,
                entry.name.clone(),
                attachment_from_schema(&entry.attachment, &bone_ids),
            );
        }
        for name in &s.bones {
            match bone_ids.get(name) {
                Some(&id) => skin.bones.push(id),
                None => report.dangling("skin bone", name),
            }
        }
        let id = skeleton.skins.insert(skin);
        skin_constraint_names.push((id, s.constraints.clone()));
        if s.name == project.default_skin {
            skeleton.default_skin = id;
        }
    }
    // Every skeleton needs a default skin (T-105); synthesize one if the file
    // had none or named one that is missing.
    if skeleton.skins.get(skeleton.default_skin).is_none() {
        skeleton.default_skin = match skeleton.skins.iter().next() {
            Some((id, _)) => id,
            None => skeleton.skins.insert(Skin::new("default")),
        };
    }

    // Constraints.
    let mut constraint_ids: HashMap<String, ConstraintId> = HashMap::new();
    for c in &project.constraints {
        // A path constraint's source is a slot, not a bone, so it is the one
        // kind with no target to resolve.
        let target = match bone_ids.get(&c.target) {
            Some(&id) => id,
            None if c.kind == "path" => Default::default(),
            None => {
                report.dangling("constraint target", &c.target);
                continue;
            }
        };
        let mut chain = Vec::with_capacity(c.bones.len());
        let mut chain_ok = true;
        for name in &c.bones {
            match bone_ids.get(name) {
                Some(&id) => chain.push(id),
                None => {
                    report.dangling("constraint bone", name);
                    chain_ok = false;
                }
            }
        }
        if !chain_ok {
            continue;
        }
        // The `type` field decides the variant. An unknown kind is reported
        // rather than guessed at: silently importing a constraint we do not
        // understand as an IK one would move bones the author never asked to
        // move.
        let constraint = match c.kind.as_str() {
            "transform" => {
                let mixes = c.mixes.unwrap_or([1.0, 0.0, 0.0, 0.0]);
                let o = c.offsets.unwrap_or([0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0]);
                Constraint::Transform(TransformConstraint {
                    name: c.name.clone(),
                    target,
                    bones: chain,
                    offsets: ankhimate_core::math::Transform {
                        position: glam::vec2(o[0], o[1]),
                        rotation: o[2].to_radians(),
                        scale: glam::vec2(o[3], o[4]),
                        shear: glam::vec2(o[5].to_radians(), o[6].to_radians()),
                    },
                    mix_rotate: mixes[0],
                    mix_translate: mixes[1],
                    mix_scale: mixes[2],
                    mix_shear: mixes[3],
                    local: c.local,
                    relative: c.relative,
                })
            }
            "physics" => {
                let [inertia, strength, damping, mass] = c.physics.unwrap_or([0.5, 40.0, 0.5, 1.0]);
                let [wx, wy, gx, gy] = c.forces.unwrap_or([0.0; 4]);
                let [rotate, translate] = c.channels.unwrap_or([true, false]);
                Constraint::Physics(ankhimate_core::constraints::PhysicsConstraint {
                    name: c.name.clone(),
                    bone: target,
                    inertia,
                    strength,
                    damping,
                    mass,
                    wind: glam::vec2(wx, wy),
                    gravity: glam::vec2(gx, gy),
                    mix: c.mix,
                    rotate,
                    translate,
                })
            }
            "path" => {
                let Some(slot_name) = c.slot.as_deref() else {
                    report.dangling("path constraint slot", &c.name);
                    continue;
                };
                let Some(&slot) = slot_ids.get(slot_name) else {
                    report.dangling("path constraint slot", slot_name);
                    continue;
                };
                let [position, spacing, mix_rotate, mix_translate] =
                    c.path.unwrap_or([0.0, 1.0, 1.0, 1.0]);
                Constraint::Path(ankhimate_core::constraints::PathConstraint {
                    name: c.name.clone(),
                    slot,
                    bones: chain,
                    position,
                    spacing,
                    mix_rotate,
                    mix_translate,
                })
            }
            "ik" => Constraint::Ik(IkConstraint {
                name: c.name.clone(),
                target,
                bones: chain,
                bend_direction: c.bend_direction,
                mix: c.mix,
                softness: c.softness,
                stretch: c.stretch,
                stretch_limit: c.stretch_limit,
                stiffness: c.stiffness,
            }),
            other => {
                report.dangling("constraint type", other);
                continue;
            }
        };
        let id = skeleton.constraints.insert(constraint);
        constraint_ids.insert(c.name.clone(), id);
    }
    for name in &project.constraint_order {
        match constraint_ids.get(name) {
            Some(&id) => skeleton.constraint_order.push(id),
            None => report.dangling("constraint order", name),
        }
    }
    for (id, _) in skeleton.constraints.iter() {
        if !skeleton.constraint_order.contains(&id) {
            skeleton.constraint_order.push(id);
        }
    }

    // Selection sets (T-904). A name the rig no longer has is reported rather
    // than dropped in silence: a set that quietly selects three bones instead of
    // the eight it names is worse than one that says it lost five.
    for set in &project.selection_sets {
        let mut bones = Vec::new();
        for name in &set.bones {
            match bone_ids.get(name) {
                Some(&id) => bones.push(id),
                None => report.dangling("selection set bone", name),
            }
        }
        // An empty set is a row that does nothing; if every bone in it is gone,
        // the set is gone with them.
        if !bones.is_empty() {
            skeleton
                .selection_sets
                .push(ankhimate_core::skeleton::SelectionSet {
                    name: set.name.clone(),
                    bones,
                });
        }
    }

    // Now that constraints have ids, hand each skin the ones it owns.
    for (skin, names) in skin_constraint_names {
        for name in &names {
            match constraint_ids.get(name) {
                Some(&id) => {
                    if let Some(skin) = skeleton.skins.get_mut(skin) {
                        skin.constraints.push(id);
                    }
                }
                None => report.dangling("skin constraint", name),
            }
        }
    }

    // Animations.
    let mut animations = SlotMap::with_key();
    for a in &project.animations {
        let timelines = a
            .timelines
            .iter()
            .filter_map(|t| {
                timeline_from_schema(t, &bone_ids, &slot_ids, &constraint_ids, &mut report)
            })
            .collect();
        animations.insert(anim::Animation {
            name: a.name.clone(),
            duration: a.duration,
            timelines,
            events: a
                .events
                .iter()
                .map(|e| ankhimate_core::animation::EventKey {
                    time: e.time,
                    name: e.name.clone(),
                    int_value: e.int_value,
                    float_value: e.float_value,
                    string_value: e.string_value.clone(),
                    audio: e.audio.clone(),
                    volume: e.volume,
                    balance: e.balance,
                })
                .collect(),
            // Sorted on the way in rather than trusted: the invariant everything
            // downstream relies on is "ordered by time", and a hand-edited or
            // third-party file has made no such promise.
            markers: {
                let mut markers: Vec<ankhimate_core::animation::Marker> = a
                    .markers
                    .iter()
                    .map(|m| ankhimate_core::animation::Marker {
                        time: m.time,
                        name: m.name.clone(),
                        color: m.color,
                    })
                    .collect();
                markers.sort_by(|x, y| x.time.total_cmp(&y.time));
                markers
            },
            looping: a.looping,
        });
    }

    Loaded {
        skeleton,
        animations,
        assets,
        name: project.name.clone(),
        fps: project.fps,
        report,
    }
}

fn attachment_from_schema(
    attachment: &schema::Attachment,
    bone_ids: &HashMap<String, BoneId>,
) -> att::Attachment {
    match attachment {
        schema::Attachment::Region(r) => att::Attachment::Region(att::RegionAttachment {
            texture: r.texture.clone(),
            local_offset: glam::vec2(r.offset_x, r.offset_y),
            local_rotation: r.rotation.to_radians(),
            local_scale: glam::vec2(r.scale_x, r.scale_y),
            width: r.width,
            height: r.height,
            uv_rect: att::Rect {
                x: r.uv[0],
                y: r.uv[1],
                w: r.uv[2],
                h: r.uv[3],
            },
            pivot: glam::vec2(r.pivot_x, r.pivot_y),
            sequence: r.sequence.as_ref().map(sequence_from_schema),
        }),
        schema::Attachment::Clipping(c) => att::Attachment::Clipping(att::ClippingAttachment {
            vertices: unflatten_vec2(&c.vertices),
            end_slot: c.end_slot.clone(),
        }),
        schema::Attachment::Path(p) => att::Attachment::Path(att::PathAttachment {
            vertices: unflatten_vec2(&p.vertices),
            closed: p.closed,
            constant_speed: p.constant_speed,
        }),
        schema::Attachment::Mesh(m) => att::Attachment::Mesh(att::MeshAttachment {
            texture: m.texture.clone(),
            setup_vertices: unflatten_vec2(&m.vertices),
            uvs: unflatten_vec2(&m.uvs),
            triangles: m
                .triangles
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
            edges: m.edges.chunks_exact(2).map(|c| [c[0], c[1]]).collect(),
            weights: weights_from_schema(&m.weights, bone_ids),
            ffd_keyframes: Vec::new(),
            inverse_bind_matrices: Default::default(),
            linked: m.linked.as_ref().map(|l| att::LinkedMesh {
                skin: l.skin.clone(),
                slot: l.slot.clone(),
                attachment: l.attachment.clone(),
                inherit_deform: l.inherit_deform,
            }),
            sequence: m.sequence.as_ref().map(sequence_from_schema),
        }),
        schema::Attachment::BoundingBox(b) => {
            att::Attachment::BoundingBox(att::BoundingBoxAttachment {
                vertices: unflatten_vec2(&b.vertices),
                weights: weights_from_schema(&b.weights, bone_ids),
            })
        }
        schema::Attachment::Point(p) => att::Attachment::Point(att::PointAttachment {
            position: glam::vec2(p.x, p.y),
            rotation: p.rotation.to_radians(),
        }),
    }
}

fn timeline_from_schema(
    timeline: &schema::Timeline,
    bone_ids: &HashMap<String, BoneId>,
    slot_ids: &HashMap<String, SlotId>,
    constraint_ids: &HashMap<String, ConstraintId>,
    report: &mut LoadReport,
) -> Option<anim::Timeline> {
    let vec2_keys = |keys: &[schema::Vec2Key], from_degrees: bool| {
        keys.iter()
            .map(|k| anim::Key {
                time: k.time,
                value: if from_degrees {
                    glam::vec2(k.x.to_radians(), k.y.to_radians())
                } else {
                    glam::vec2(k.x, k.y)
                },
                interp: interp_from_schema(k.interp),
            })
            .collect()
    };
    let scalar_keys = |keys: &[schema::ScalarKey]| {
        keys.iter()
            .map(|k| anim::Key {
                time: k.time,
                value: k.value,
                interp: interp_from_schema(k.interp),
            })
            .collect()
    };

    // A timeline whose target vanished is dropped, not silently retargeted.
    macro_rules! bone {
        ($name:expr) => {
            match bone_ids.get($name) {
                Some(&id) => id,
                None => {
                    report.dangling("timeline bone", $name);
                    return None;
                }
            }
        };
    }
    macro_rules! slot {
        ($name:expr) => {
            match slot_ids.get($name) {
                Some(&id) => id,
                None => {
                    report.dangling("timeline slot", $name);
                    return None;
                }
            }
        };
    }

    Some(match timeline {
        schema::Timeline::BoneTranslate { bone, keys } => anim::Timeline::BoneTranslate {
            bone: bone!(bone),
            keys: vec2_keys(keys, false),
        },
        schema::Timeline::BoneRotate { bone, keys } => anim::Timeline::BoneRotate {
            bone: bone!(bone),
            keys: scalar_keys(keys),
        },
        schema::Timeline::BoneScale { bone, keys } => anim::Timeline::BoneScale {
            bone: bone!(bone),
            keys: vec2_keys(keys, false),
        },
        // Degrees on both sides (see the writer) — no conversion.
        schema::Timeline::BoneShear { bone, keys } => anim::Timeline::BoneShear {
            bone: bone!(bone),
            keys: vec2_keys(keys, false),
        },
        schema::Timeline::SlotColor { slot, keys } => anim::Timeline::SlotColor {
            slot: slot!(slot),
            keys: keys
                .iter()
                .map(|k| anim::Key {
                    time: k.time,
                    value: k.value,
                    interp: interp_from_schema(k.interp),
                })
                .collect(),
        },
        schema::Timeline::SlotVisible { slot, keys } => anim::Timeline::SlotVisible {
            slot: slot!(slot),
            keys: keys
                .iter()
                .map(|k| anim::Key {
                    time: k.time,
                    value: k.value,
                    // Stepped by construction; the schema does not carry a curve
                    // for a boolean because there is no curve to carry.
                    interp: anim::Interp::Stepped,
                })
                .collect(),
        },
        schema::Timeline::SlotAttachment { slot, keys } => anim::Timeline::SlotAttachment {
            slot: slot!(slot),
            keys: keys
                .iter()
                .map(|k| anim::Key {
                    time: k.time,
                    value: k.value.clone(),
                    interp: anim::Interp::Stepped,
                })
                .collect(),
        },
        schema::Timeline::DrawOrder { keys } => anim::Timeline::DrawOrder {
            keys: keys
                .iter()
                .map(|k| anim::Key {
                    time: k.time,
                    value: k
                        .offsets
                        .iter()
                        .filter_map(|(name, delta)| slot_ids.get(name).map(|&id| (id, *delta)))
                        .collect(),
                    interp: anim::Interp::Stepped,
                })
                .collect(),
        },
        schema::Timeline::IkMix { constraint, keys } => {
            let id = match constraint_ids.get(constraint) {
                Some(&id) => id,
                None => {
                    report.dangling("timeline constraint", constraint);
                    return None;
                }
            };
            anim::Timeline::IkMix {
                constraint: id,
                keys: scalar_keys(keys),
            }
        }
        schema::Timeline::IkBendDirection { constraint, keys } => {
            let Some(&id) = constraint_ids.get(constraint) else {
                report.dangling("timeline constraint", constraint);
                return None;
            };
            anim::Timeline::IkBendDirection {
                constraint: id,
                keys: scalar_keys(keys),
            }
        }
        schema::Timeline::IkSoftness { constraint, keys } => {
            let Some(&id) = constraint_ids.get(constraint) else {
                report.dangling("timeline constraint", constraint);
                return None;
            };
            anim::Timeline::IkSoftness {
                constraint: id,
                keys: scalar_keys(keys),
            }
        }
        schema::Timeline::TransformConstraintMix { constraint, keys } => {
            let id = match constraint_ids.get(constraint) {
                Some(&id) => id,
                None => {
                    report.dangling("timeline constraint", constraint);
                    return None;
                }
            };
            anim::Timeline::TransformConstraintMix {
                constraint: id,
                keys: keys
                    .iter()
                    .map(|k| anim::Key {
                        time: k.time,
                        value: k.value,
                        interp: interp_from_schema(k.interp),
                    })
                    .collect(),
            }
        }
        schema::Timeline::Deform {
            slot,
            attachment,
            keys,
        } => anim::Timeline::Deform {
            slot: slot!(slot),
            attachment: attachment.clone(),
            keys: keys
                .iter()
                .map(|k| anim::Key {
                    time: k.time,
                    value: unflatten_vec2(&k.offsets),
                    interp: interp_from_schema(k.interp),
                })
                .collect(),
        },
    })
}
