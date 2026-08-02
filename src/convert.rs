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
use ankhimate_core::constraints::{Constraint, IkConstraint};
use ankhimate_core::ids::{AnimationId, BoneId, ConstraintId, SlotId};
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
        extra: Default::default(),
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
            extra: Default::default(),
        }),
        att::Attachment::Clipping(c) => schema::Attachment::Clipping(schema::Clipping {
            vertices: flatten_vec2(&c.vertices),
            end_slot: c.end_slot.clone(),
            extra: Default::default(),
        }),
        att::Attachment::Mesh(m) => schema::Attachment::Mesh(schema::Mesh {
            texture: m.texture.clone(),
            vertices: flatten_vec2(&m.setup_vertices),
            uvs: flatten_vec2(&m.uvs),
            triangles: m.triangles.iter().flat_map(|t| *t).collect(),
            weights: m
                .weights
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
                .collect(),
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
        let id = skeleton.skins.insert(skin);
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
        let Some(&target) = bone_ids.get(&c.target) else {
            report.dangling("constraint target", &c.target);
            continue;
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
        let id = skeleton.constraints.insert(Constraint::Ik(IkConstraint {
            name: c.name.clone(),
            target,
            bones: chain,
            bend_direction: c.bend_direction,
            mix: c.mix,
            softness: c.softness,
            stretch: c.stretch,
        }));
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
            events: Vec::new(),
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
        }),
        schema::Attachment::Clipping(c) => att::Attachment::Clipping(att::ClippingAttachment {
            vertices: unflatten_vec2(&c.vertices),
            end_slot: c.end_slot.clone(),
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
            weights: m
                .weights
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
                .collect(),
            ffd_keyframes: Vec::new(),
            inverse_bind_matrices: Default::default(),
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
