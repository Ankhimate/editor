//! The template context (T-603b) — what a template can see.
//!
//! # This is a public contract
//!
//! Once users have written templates, renaming a field here breaks their work
//! silently: there is no compiler on the other side, only a template that starts
//! producing the wrong file. Treat this module the way `formats::schema` is
//! treated (ADR 0004) — additive changes are free, renames are breaking and need
//! a [`CONTEXT_VERSION`] bump.
//!
//! The context is a projection of the **disk** schema, not of `core`: named
//! references, degrees for angles (PLAN §2.7). That is deliberate — an exporter
//! and a save file want exactly the same shape, so there is no third
//! representation to keep in sync.
//!
//! Two things are added that the schema does not store, because a logic-less
//! template provably cannot derive them:
//!
//! - `bone.index` and `bone.parent_index` — engines that store bones as a flat
//!   array with integer parent references need an ordering, and a template
//!   cannot topologically sort.
//! - `bone.children` — Handlebars cannot invert a parent pointer.
//!
//! And one thing is deliberately withheld: **no timestamp**. A timestamp makes
//! every export differ from the last, which destroys diffability and turns "did
//! the rig actually change?" into an unanswerable question in version control.

use crate::atlas::Atlas;
use ankhimate_formats::schema::{self, Project};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// Bumped when a field is renamed or removed. Additions do not bump it.
pub const CONTEXT_VERSION: u32 = 1;

/// Everything a template can address, as a JSON tree.
#[derive(Debug, Clone, Serialize)]
pub struct Context {
    pub context_version: u32,
    pub project: Value,
    pub skeleton: Value,
    pub animations: Vec<Value>,
    /// Absent when the preset bakes no atlas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atlas: Option<Value>,
    pub export: Value,
}

impl Context {
    /// Build the context for one export.
    pub fn build(project: &Project, atlas: Option<&Atlas>, export: ExportInfo) -> Self {
        Self {
            context_version: CONTEXT_VERSION,
            project: json!({
                "name": project.name,
                "fps": project.fps,
                "version": project.version,
            }),
            skeleton: skeleton(project),
            animations: project.animations.iter().map(animation).collect(),
            atlas: atlas.map(atlas_value),
            export: json!({
                "output_dir": export.output_dir,
                "preset_name": export.preset_name,
                "template_name": export.template_name,
            }),
        }
    }

    /// The same context with `animation` bound to one clip, for a
    /// `per: animation` template.
    pub fn with_animation(&self, index: usize) -> Value {
        let mut root = serde_json::to_value(self).expect("context serializes");
        if let (Some(map), Some(anim)) = (root.as_object_mut(), self.animations.get(index)) {
            map.insert("animation".into(), anim.clone());
        }
        root
    }

    /// The context as a plain JSON value, with no single animation bound.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("context serializes")
    }
}

/// Which export this is, for templates that name their own output.
#[derive(Debug, Clone, Default)]
pub struct ExportInfo {
    pub output_dir: String,
    pub preset_name: String,
    pub template_name: String,
}

fn skeleton(project: &Project) -> Value {
    let order = bone_order(project);
    let index_of = |name: &str| order.iter().position(|n| n == name);

    let bones: Vec<Value> = order
        .iter()
        .filter_map(|name| project.bones.iter().find(|b| &b.name == name))
        .map(|b| {
            let children: Vec<&str> = order
                .iter()
                .filter(|n| {
                    project
                        .bones
                        .iter()
                        .any(|c| &&c.name == n && c.parent == b.name)
                })
                .map(|n| n.as_str())
                .collect();
            json!({
                "name": b.name,
                "parent": b.parent,
                "index": index_of(&b.name),
                // -1 rather than null: a template writing a flat array needs a
                // number in the field either way, and `{{parent_index}}` must
                // not render "null" into a numeric slot.
                "parent_index": if b.parent.is_empty() { -1 } else {
                    index_of(&b.parent).map(|i| i as i64).unwrap_or(-1)
                },
                "length": b.length,
                "x": b.tx,
                "y": b.ty,
                "rotation": b.rotation,
                "scale_x": b.sx,
                "scale_y": b.sy,
                "shear_x": b.shear_x,
                "shear_y": b.shear_y,
                "inherit_rotation": b.inherit_rotation,
                "inherit_scale": b.inherit_scale,
                "inherit_reflect": b.inherit_reflect,
                "color": b.color,
                "children": children,
            })
        })
        .collect();

    let slots: Vec<Value> = project
        .slots
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "bone": s.bone,
                "attachment": s.attachment,
                "color": s.color,
                "dark_color": s.dark_color,
                "blend": if s.blend_mode.is_empty() { "normal" } else { &s.blend_mode },
            })
        })
        .collect();

    let skins: Vec<Value> = project
        .skins
        .iter()
        .map(|skin| {
            let entries: Vec<Value> = skin
                .entries
                .iter()
                .map(|e| {
                    json!({
                        "slot": e.slot,
                        "name": e.name,
                        "attachment": attachment(&e.attachment),
                    })
                })
                .collect();
            json!({
                "name": skin.name,
                "entries": entries,
                "bones": skin.bones,
                "constraints": skin.constraints,
            })
        })
        .collect();

    json!({
        "bones": bones,
        "slots": slots,
        "draw_order": project.draw_order,
        "skins": skins,
        "default_skin": project.default_skin,
        "constraints": project.constraints.iter().map(constraint).collect::<Vec<_>>(),
        "constraint_order": project.constraint_order,
    })
}

/// Bones parents-before-children.
///
/// A runtime that applies transforms in array order needs this; the saved order
/// is authoring order and carries no such guarantee. Ties break by name so the
/// result is deterministic.
fn bone_order(project: &Project) -> Vec<String> {
    let mut roots: Vec<&str> = project
        .bones
        .iter()
        .filter(|b| b.parent.is_empty())
        .map(|b| b.name.as_str())
        .collect();
    roots.sort_unstable();

    let mut out: Vec<String> = Vec::with_capacity(project.bones.len());
    let mut stack: Vec<&str> = roots.into_iter().rev().collect();
    while let Some(name) = stack.pop() {
        if out.iter().any(|n| n == name) {
            continue; // A cycle, or a name used twice. Do not spin.
        }
        out.push(name.to_string());
        let mut children: Vec<&str> = project
            .bones
            .iter()
            .filter(|b| b.parent == name)
            .map(|b| b.name.as_str())
            .collect();
        children.sort_unstable();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    // A bone whose parent does not exist is orphaned rather than dropped: losing
    // it silently would be worse than exporting it as a root.
    for b in &project.bones {
        if !out.iter().any(|n| n == &b.name) {
            out.push(b.name.clone());
        }
    }
    out
}

fn attachment(att: &schema::Attachment) -> Value {
    match att {
        schema::Attachment::Region(r) => json!({
            "type": "region",
            "texture": r.texture,
            "x": r.offset_x,
            "y": r.offset_y,
            "rotation": r.rotation,
            "scale_x": r.scale_x,
            "scale_y": r.scale_y,
            "width": r.width,
            "height": r.height,
            "uv": r.uv,
            "pivot_x": r.pivot_x,
            "pivot_y": r.pivot_y,
            "sequence": r.sequence.as_ref().map(sequence),
        }),
        schema::Attachment::Mesh(m) => json!({
            "type": "mesh",
            "texture": m.texture,
            "vertices": m.vertices,
            "uvs": m.uvs,
            "triangles": m.triangles,
            "edges": m.edges,
            "weights": packed_weights(&m.weights, &m.vertices),
            "weighted": !m.weights.is_empty(),
            "vertex_count": m.vertices.len() / 2,
            "linked": m.linked.as_ref().map(|l| json!({
                "skin": l.skin,
                "slot": l.slot,
                "attachment": l.attachment,
                "inherit_deform": l.inherit_deform,
            })),
            "sequence": m.sequence.as_ref().map(sequence),
        }),
        schema::Attachment::Clipping(c) => json!({
            "type": "clipping",
            "vertices": c.vertices,
            "vertex_count": c.vertices.len() / 2,
            "end_slot": c.end_slot,
        }),
        schema::Attachment::Path(p) => json!({
            "type": "path",
            "vertices": p.vertices,
            "vertex_count": p.vertices.len() / 2,
            "closed": p.closed,
            "constant_speed": p.constant_speed,
        }),
        schema::Attachment::BoundingBox(b) => json!({
            "type": "bounding_box",
            "vertices": b.vertices,
            "vertex_count": b.vertices.len() / 2,
            "weights": packed_weights(&b.weights, &b.vertices),
            "weighted": !b.weights.is_empty(),
        }),
        schema::Attachment::Point(p) => json!({
            "type": "point",
            "x": p.x,
            "y": p.y,
            "rotation": p.rotation,
        }),
    }
}

/// Weights in the standard packed form: per vertex, a count followed by that
/// many `{bone, x, y, weight}` entries.
///
/// Restructuring nested arrays is exactly what a logic-less template cannot do,
/// and every runtime format wants some variant of this shape. Doing it once here
/// beats every preset author reimplementing it — badly, or not at all.
///
/// `x`/`y` repeat the vertex position per influence, matching the convention
/// where a weighted vertex's position is expressed once per bound bone.
fn packed_weights(weights: &[Vec<(String, f32)>], vertices: &[f32]) -> Vec<Value> {
    weights
        .iter()
        .enumerate()
        .map(|(i, per_vertex)| {
            let (x, y) = (
                vertices.get(i * 2).copied().unwrap_or(0.0),
                vertices.get(i * 2 + 1).copied().unwrap_or(0.0),
            );
            let bones: Vec<Value> = per_vertex
                .iter()
                .map(|(bone, weight)| json!({ "bone": bone, "x": x, "y": y, "weight": weight }))
                .collect();
            json!({ "count": bones.len(), "bones": bones })
        })
        .collect()
}

fn sequence(s: &schema::Sequence) -> Value {
    json!({
        "frames": s.frames,
        "fps": s.fps,
        "mode": s.mode,
        "setup_index": s.setup_index,
    })
}

fn constraint(c: &schema::Constraint) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), json!(c.name));
    map.insert("type".into(), json!(c.kind));
    map.insert("target".into(), json!(c.target));
    map.insert("bones".into(), json!(c.bones));
    map.insert("mix".into(), json!(c.mix));

    // Kind-specific blocks are present only for their own kind, so a template
    // that walks `constraints` can branch on `type` and trust what it finds.
    match c.kind.as_str() {
        "ik" => {
            map.insert("bend_direction".into(), json!(c.bend_direction));
            map.insert("softness".into(), json!(c.softness));
            map.insert("stretch".into(), json!(c.stretch));
            map.insert("stretch_limit".into(), json!(c.stretch_limit));
            map.insert("stiffness".into(), json!(c.stiffness));
        }
        "transform" => {
            if let Some(m) = c.mixes {
                map.insert(
                    "mixes".into(),
                    json!({ "rotate": m[0], "translate": m[1], "scale": m[2], "shear": m[3] }),
                );
            }
            if let Some(o) = c.offsets {
                map.insert(
                    "offsets".into(),
                    json!({
                        "x": o[0], "y": o[1], "rotation": o[2],
                        "scale_x": o[3], "scale_y": o[4],
                        "shear_x": o[5], "shear_y": o[6],
                    }),
                );
            }
            map.insert("local".into(), json!(c.local));
            map.insert("relative".into(), json!(c.relative));
        }
        "physics" => {
            if let Some(p) = c.physics {
                map.insert(
                    "physics".into(),
                    json!({ "inertia": p[0], "strength": p[1], "damping": p[2], "mass": p[3] }),
                );
            }
            if let Some(f) = c.forces {
                map.insert(
                    "forces".into(),
                    json!({ "wind_x": f[0], "wind_y": f[1], "gravity_x": f[2], "gravity_y": f[3] }),
                );
            }
            if let Some(ch) = c.channels {
                map.insert(
                    "channels".into(),
                    json!({ "rotate": ch[0], "translate": ch[1] }),
                );
            }
        }
        "path" => {
            if let Some(slot) = &c.slot {
                map.insert("slot".into(), json!(slot));
            }
            if let Some(p) = c.path {
                map.insert(
                    "path".into(),
                    json!({
                        "position": p[0], "spacing": p[1],
                        "mix_rotate": p[2], "mix_translate": p[3],
                    }),
                );
            }
        }
        _ => {}
    }
    Value::Object(map)
}

fn animation(anim: &schema::Animation) -> Value {
    let offset_of = |bone: &str| {
        anim.bone_offsets
            .iter()
            .find(|o| o.bone == bone)
            .map(|o| o.offset)
            .unwrap_or(0.0)
    };

    let mut bones: Vec<Value> = Vec::new();
    let mut slots: Vec<Value> = Vec::new();
    let mut deform: Vec<Value> = Vec::new();
    let mut draw_order: Vec<Value> = Vec::new();
    let mut ik: Vec<Value> = Vec::new();
    let mut transform: Vec<Value> = Vec::new();

    // Grouped by target rather than left as a flat timeline list: every runtime
    // format writes "this bone, these channels", and regrouping a flat list is
    // beyond a logic-less template.
    let mut bone_entry = |name: &str, channel: &str, keys: Value| {
        let shift = offset_of(name);
        if let Some(existing) = bones
            .iter_mut()
            .find(|b| b["name"].as_str() == Some(name))
            .and_then(|b| b.as_object_mut())
        {
            existing.insert(channel.into(), keys);
            return;
        }
        let mut map = Map::new();
        map.insert("name".into(), json!(name));
        map.insert("offset".into(), json!(shift));
        map.insert(channel.into(), keys);
        bones.push(Value::Object(map));
    };

    for timeline in &anim.timelines {
        match timeline {
            schema::Timeline::BoneTranslate { bone, keys } => {
                bone_entry(bone, "translate", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneRotate { bone, keys } => {
                bone_entry(bone, "rotate", scalar_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneScale { bone, keys } => {
                bone_entry(bone, "scale", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneShear { bone, keys } => {
                bone_entry(bone, "shear", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::SlotColor { slot, keys } => slots.push(json!({
                "name": slot, "channel": "color",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "color": k.value, "curve": interp(&k.interp),
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::SlotVisible { slot, keys } => slots.push(json!({
                "name": slot, "channel": "visible",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "value": k.value, "curve": "stepped",
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::SlotAttachment { slot, keys } => slots.push(json!({
                "name": slot, "channel": "attachment",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "name": k.value, "curve": "stepped",
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::DrawOrder { keys } => {
                draw_order = keys
                    .iter()
                    .map(|k| {
                        json!({
                            "time": k.time,
                            "offsets": k.offsets.iter().map(|(slot, off)| json!({
                                "slot": slot, "offset": off,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect()
            }
            schema::Timeline::IkMix { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "mix",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::IkBendDirection { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "bend_direction",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::IkSoftness { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "softness",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::TransformConstraintMix { constraint, keys } => {
                transform.push(json!({
                    "constraint": constraint,
                    "keys": keys.iter().map(|k| json!({
                        "time": k.time,
                        "rotate": k.value[0], "translate": k.value[1],
                        "scale": k.value[2], "shear": k.value[3],
                        "curve": interp(&k.interp),
                    })).collect::<Vec<_>>(),
                }))
            }
            schema::Timeline::Deform {
                slot,
                attachment,
                keys,
            } => deform.push(json!({
                "slot": slot, "attachment": attachment,
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "offsets": k.offsets, "curve": interp(&k.interp),
                })).collect::<Vec<_>>(),
            })),
        }
    }

    json!({
        "name": anim.name,
        "duration": anim.duration,
        "looping": anim.looping,
        "bones": bones,
        "slots": slots,
        "deform": deform,
        "draw_order": draw_order,
        "ik": ik,
        "transform": transform,
        "events": anim.events.iter().map(|e| json!({
            "time": e.time,
            "name": e.name,
            "int": e.int_value,
            "float": e.float_value,
            "string": e.string_value,
            "audio": e.audio,
            "volume": e.volume,
            "balance": e.balance,
        })).collect::<Vec<_>>(),
        // `markers` is absent by design: it is editor furniture (schema::Marker
        // says so) and exporting it would invite presets to write notes into
        // runtime files.
    })
}

/// Per-bone track offsets (T-905) are folded into key times here.
///
/// No runtime format has a concept for them, so leaving them for a template to
/// apply guarantees every preset gets it wrong. Bake, do not export — the
/// authored offset still reaches the game, as shifted keys.
fn shift(time: f32, offset: f32) -> f32 {
    time + offset
}

fn scalar_keys(keys: &[schema::ScalarKey], offset: f32) -> Value {
    json!(
        keys.iter()
            .map(|k| json!({
                "time": shift(k.time, offset),
                "value": k.value,
                "curve": interp(&k.interp),
            }))
            .collect::<Vec<_>>()
    )
}

fn vec2_keys(keys: &[schema::Vec2Key], offset: f32) -> Value {
    json!(
        keys.iter()
            .map(|k| json!({
                "time": shift(k.time, offset),
                "x": k.x,
                "y": k.y,
                "curve": interp(&k.interp),
            }))
            .collect::<Vec<_>>()
    )
}

/// A key's interpolation, as a value a template can both print and branch on.
fn interp(i: &schema::Interp) -> Value {
    match i {
        schema::Interp::Linear => json!("linear"),
        schema::Interp::Stepped => json!("stepped"),
        // An object where the simple cases are strings would force every
        // template to type-check before printing. Bezier keeps the string in
        // `curve` and puts its handles alongside, so `{{curve}}` always prints.
        schema::Interp::Bezier { handles } => json!({
            "type": "bezier",
            "handles": handles,
            "out_x": handles[0], "out_y": handles[1],
            "in_x": handles[2], "in_y": handles[3],
        }),
    }
}

fn atlas_value(atlas: &Atlas) -> Value {
    json!({
        "pages": atlas.pages.iter().map(|p| json!({
            "index": p.index,
            "width": p.width,
            "height": p.height,
            "file": crate::atlas::page_filename("atlas", p.index),
        })).collect::<Vec<_>>(),
        "regions": atlas.regions.iter().map(|r| json!({
            "name": r.name,
            "page": r.page,
            "x": r.x,
            "y": r.y,
            "width": r.width,
            "height": r.height,
            "offset_x": r.offset_x,
            "offset_y": r.offset_y,
            "original_width": r.original_width,
            "original_height": r.original_height,
            "rotated": r.rotated,
        })).collect::<Vec<_>>(),
    })
}
