//! The verbs an importer needs, beyond the ones that build a skeleton.
//!
//! `doc_ops` covers rig structure — bones, slots, clips. That is enough for a
//! format that carries only a hierarchy, and not enough for any real one.
//!
//! `docs/export-plan.md` states the rule this exists to satisfy, on the export
//! side: our own runtime format is a template, so if it cannot be expressed the
//! engine is too weak and we find out before users do. The import side needs
//! the same guarantee, and the way to get it is to make a shipped importer
//! writable as a plugin.
//!
//! These are the verbs that gap turned out to be: an image, an attachment, a
//! mesh, a keyframe, and a way to say what could not be carried across.

use crate::args::{Args, Resolver};
use crate::commands::{create_attachment_cmds, key_cmds};
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use ankhimate_core::attachment::{Attachment, MeshAttachment, Rect, RegionAttachment};
use serde_json::json;

/// Add an image to the asset library.
///
/// Bytes arrive base64-encoded, which is what a script can produce from a
/// sidecar without a binary channel. Ugly and honest: the alternative is a
/// typed-array binding whose failure mode is a silently truncated PNG.
pub struct AddImage;

impl DocOperator for AddImage {
    fn id(&self) -> &'static str {
        "asset.add_image"
    }

    fn label(&self) -> &str {
        "Add Image"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "bytes_base64"],
            "properties": {
                "name": { "type": "string", "description": "What attachments will reference" },
                "bytes_base64": { "type": "string", "description": "The encoded image file" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let encoded = args.str("bytes_base64")?;
        let bytes = decode_base64(encoded).ok_or_else(|| {
            OpError::Args(crate::args::ArgError::WrongType {
                key: "bytes_base64".into(),
                wanted: "base64",
                got: "something else",
            })
        })?;

        // The size is read from the pixels rather than taken on trust: an
        // attachment sized from a lie draws at the wrong scale, and the file
        // already knows the answer.
        let (width, height) = image::load_from_memory(&bytes)
            .map(|img| (img.width(), img.height()))
            .unwrap_or((0, 0));

        edit.dispatch(Box::new(crate::commands::asset_cmds::AddAsset::new(
            ankhimate_core::assets::ImageAsset::new(name, bytes, width, height),
        )))?;
        Ok(())
    }
}

/// Put a region attachment in a slot.
pub struct CreateRegion;

impl DocOperator for CreateRegion {
    fn id(&self) -> &'static str {
        "attachment.create_region"
    }

    fn label(&self) -> &str {
        "Create Region Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "name", "texture"],
            "properties": {
                "slot": { "type": "string" },
                "name": { "type": "string", "description": "Attachment name within the slot" },
                "texture": { "type": "string", "description": "An image in the asset library" },
                "x": { "type": "number", "default": 0 },
                "y": { "type": "number", "default": 0 },
                "rotation": { "type": "number", "description": "Degrees", "default": 0 },
                "scale_x": { "type": "number", "default": 1 },
                "scale_y": { "type": "number", "default": 1 },
                "width": { "type": "number", "description": "Defaults to the image's own" },
                "height": { "type": "number" },
                "pivot_x": { "type": "number", "default": 0.5 },
                "pivot_y": { "type": "number", "default": 0.5, "description": "0 is the bottom" },
                "show": {
                    "type": "boolean", "default": true,
                    "description": "Make this the slot's visible attachment"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let slot = resolver.slot(args, "slot")?;
        let name = args.str("name")?.to_string();
        let texture = args.str("texture")?.to_string();

        // The image's own size unless told otherwise, so an importer that knows
        // nothing about extents still produces something that draws — a region
        // at zero size draws nothing at all, which is a whole afternoon of
        // wondering where the artwork went.
        let (image_w, image_h) = edit
            .doc
            .assets
            .by_name(&texture)
            .and_then(|id| edit.doc.assets.images.get(id))
            .map(|a| (a.width as f32, a.height as f32))
            .unwrap_or((0.0, 0.0));

        let attachment = Attachment::Region(RegionAttachment {
            texture,
            local_offset: glam::vec2(args.f32_or("x", 0.0)?, args.f32_or("y", 0.0)?),
            local_rotation: args.f32_or("rotation", 0.0)?.to_radians(),
            local_scale: glam::vec2(args.f32_or("scale_x", 1.0)?, args.f32_or("scale_y", 1.0)?),
            width: args.f32_or("width", image_w)?,
            height: args.f32_or("height", image_h)?,
            uv_rect: Rect::default(),
            pivot: glam::vec2(args.f32_or("pivot_x", 0.5)?, args.f32_or("pivot_y", 0.5)?),
            sequence: None,
        });

        edit.dispatch(Box::new(create_attachment_cmds::CreateAttachment::new(
            None,
            slot,
            name.clone(),
            attachment,
        )))?;

        if args.bool_or("show", true)? {
            edit.dispatch(Box::new(
                crate::commands::slot_cmds::SetSlotAttachment::new(slot, Some(name)),
            ))?;
        }
        Ok(())
    }
}

/// Put a mesh attachment in a slot.
pub struct CreateMesh;

impl DocOperator for CreateMesh {
    fn id(&self) -> &'static str {
        "attachment.create_mesh"
    }

    fn label(&self) -> &str {
        "Create Mesh Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "name", "texture", "vertices", "uvs", "triangles"],
            "properties": {
                "slot": { "type": "string" },
                "name": { "type": "string" },
                "texture": { "type": "string" },
                "vertices": {
                    "type": "array", "items": { "type": "number" },
                    "description": "Flat [x, y, x, y, ...] in the slot bone's space"
                },
                "uvs": {
                    "type": "array", "items": { "type": "number" },
                    "description": "Flat [u, v, ...], one pair per vertex"
                },
                "triangles": {
                    "type": "array", "items": { "type": "integer" },
                    "description": "Flat vertex indices, three per triangle"
                },
                "weights": {
                    "type": "array",
                    "description": "Per vertex, a list of {bone, weight}. Omit for a rigid mesh",
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "bone": { "type": "string" },
                                "weight": { "type": "number" }
                            }
                        }
                    }
                },
                "show": { "type": "boolean", "default": true }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let slot = resolver.slot(args, "slot")?;
        let name = args.str("name")?.to_string();
        let texture = args.str("texture")?.to_string();

        let vertices = args.f32_list("vertices")?;
        let uvs = args.f32_list("uvs")?;
        let triangles = args.u32_list("triangles")?;

        // Checked here rather than left to draw wrongly: a mesh whose uvs and
        // vertices disagree in length is a bug in the importer, and reporting it
        // at the call site names the attachment.
        if vertices.len() != uvs.len() {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "uvs".into(),
                wanted: "one uv pair per vertex",
                got: "a different count",
            }));
        }
        if triangles.len() % 3 != 0 {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "triangles".into(),
                wanted: "a multiple of three",
                got: "something else",
            }));
        }

        let weights = read_weights(edit, args)?;

        let attachment = Attachment::Mesh(MeshAttachment {
            texture,
            setup_vertices: vertices
                .chunks_exact(2)
                .map(|c| glam::vec2(c[0], c[1]))
                .collect(),
            uvs: uvs
                .chunks_exact(2)
                .map(|c| glam::vec2(c[0], c[1]))
                .collect(),
            triangles: triangles
                .chunks_exact(3)
                .map(|c| [c[0], c[1], c[2]])
                .collect(),
            weights,
            ffd_keyframes: Vec::new(),
            edges: Vec::new(),
            inverse_bind_matrices: Default::default(),
            linked: None,
            sequence: None,
        });

        edit.dispatch(Box::new(create_attachment_cmds::CreateAttachment::new(
            None,
            slot,
            name.clone(),
            attachment,
        )))?;

        if args.bool_or("show", true)? {
            edit.dispatch(Box::new(
                crate::commands::slot_cmds::SetSlotAttachment::new(slot, Some(name)),
            ))?;
        }
        Ok(())
    }
}

/// Per-vertex bone influences, with every bone named rather than indexed.
fn read_weights(
    edit: &Edit,
    args: &Args,
) -> Result<Vec<Vec<ankhimate_core::attachment::VertexWeight>>, OpError> {
    let Some(list) = args.as_json().get("weights").and_then(|w| w.as_array()) else {
        return Ok(Vec::new());
    };
    let resolver = Resolver::new(&edit.doc);
    let mut out = Vec::with_capacity(list.len());
    for per_vertex in list {
        let mut influences = Vec::new();
        for entry in per_vertex.as_array().into_iter().flatten() {
            let single = Args::from_json(entry.clone());
            let bone = resolver.bone(&single, "bone")?;
            let weight = single.f32_or("weight", 1.0)?;
            influences.push(ankhimate_core::attachment::VertexWeight { bone, weight });
        }
        out.push(influences);
    }
    Ok(out)
}

/// Key a bone channel.
pub struct KeyBone;

impl DocOperator for KeyBone {
    fn id(&self) -> &'static str {
        "anim.key_bone"
    }

    fn label(&self) -> &str {
        "Key Bone"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "bone", "property", "time", "value"],
            "properties": {
                "animation": { "type": "string" },
                "bone": { "type": "string" },
                "property": {
                    "type": "string",
                    "enum": ["translate", "rotate", "scale", "shear"]
                },
                "axis": {
                    "type": "string", "enum": ["x", "y"],
                    "description": "Required for every property but rotate, which has one track"
                },
                "time": { "type": "number", "description": "Seconds" },
                "value": {
                    "type": "number",
                    "description": "Degrees for rotate and shear; a factor for scale"
                },
                "interp": {
                    "type": "string", "enum": ["linear", "stepped"], "default": "linear",
                    "description": "How the segment *arriving* at this key eases"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        use key_cmds::{BoneProperty, KeyValue, TimelineAddr};

        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let bone = resolver.bone(args, "bone")?;
        let time = args.f32("time")?;
        let value = args.f32("value")?;

        let property = match args.str("property")? {
            "translate" => BoneProperty::Translate,
            "rotate" => BoneProperty::Rotate,
            "scale" => BoneProperty::Scale,
            "shear" => BoneProperty::Shear,
            other => {
                return Err(OpError::Args(crate::args::ArgError::WrongType {
                    key: "property".into(),
                    wanted: "translate, rotate, scale or shear",
                    got: Box::leak(other.to_string().into_boxed_str()),
                }));
            }
        };

        // An address names one *track*, and translate/scale/shear are two each.
        // Defaulting a missing axis to X is how a y key ends up on the x track,
        // so it is required where it matters and refused where it does not.
        let axis = match property {
            BoneProperty::Rotate => None,
            _ => Some(match args.str("axis")? {
                "x" => ankhimate_core::animation::Axis::X,
                "y" => ankhimate_core::animation::Axis::Y,
                other => {
                    return Err(OpError::Args(crate::args::ArgError::WrongType {
                        key: "axis".into(),
                        wanted: "x or y",
                        got: Box::leak(other.to_string().into_boxed_str()),
                    }));
                }
            }),
        };

        let interp = match args.opt_str("interp")? {
            Some("stepped") => ankhimate_core::animation::Interp::Stepped,
            _ => ankhimate_core::animation::Interp::Linear,
        };

        edit.dispatch(Box::new(key_cmds::AddKey::new(
            anim,
            TimelineAddr::Bone {
                bone,
                property,
                axis,
            },
            time,
            KeyValue::Scalar(value),
            interp,
        )))?;
        Ok(())
    }
}

/// Key which attachment a slot shows.
pub struct KeySlotAttachment;

impl DocOperator for KeySlotAttachment {
    fn id(&self) -> &'static str {
        "anim.key_attachment"
    }

    fn label(&self) -> &str {
        "Key Slot Attachment"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "slot", "time"],
            "properties": {
                "animation": { "type": "string" },
                "slot": { "type": "string" },
                "time": { "type": "number", "description": "Seconds" },
                "attachment": {
                    "type": "string",
                    "description": "Omit or null to hide the slot from here"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let slot = resolver.slot(args, "slot")?;
        let time = args.f32("time")?;
        // Absent means hidden, which is a real value here rather than a missing
        // one — it is how an effect is switched off partway through a clip.
        let attachment = args.opt_str("attachment")?.map(str::to_string);

        edit.dispatch(Box::new(key_cmds::AddAttachmentKey::new(
            anim, slot, time, attachment,
        )))?;
        Ok(())
    }
}

/// Say what an import could not carry across.
///
/// The honesty property the Rust readers have and a plugin could not: an import
/// that quietly drops half a file is worse than one that refuses, and the user
/// needs to hear which half.
pub struct ReportLossy;

impl DocOperator for ReportLossy {
    fn id(&self) -> &'static str {
        "import.report"
    }

    fn label(&self) -> &str {
        "Report Approximation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["what", "where", "detail"],
            "properties": {
                "what": {
                    "type": "string",
                    "description": "The kind of thing — \"curve\", \"attachment\", \"timeline\""
                },
                "where": { "type": "string", "description": "In the source's own names" },
                "detail": { "type": "string", "description": "What was done instead" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let what = args.str("what")?.to_string();
        let where_ = args.str("where")?.to_string();
        let detail = args.str("detail")?.to_string();
        // Not an `EditCommand`: a report is not part of the rig and must not
        // land on the undo stack. Undoing an import's last bone should not
        // un-say what that import could not do.
        edit.report.push(crate::edit::Approximation {
            what,
            where_,
            detail,
        });
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(AddImage));
    ops.register(Box::new(CreateRegion));
    ops.register(Box::new(CreateMesh));
    ops.register(Box::new(KeyBone));
    ops.register(Box::new(KeySlotAttachment));
    ops.register(Box::new(ReportLossy));
}

/// Decode standard base64, without a dependency.
///
/// Small enough to write, and a crate for it would be a supply-chain surface
/// for sixty lines of table lookup.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    const INVALID: u8 = 255;
    let mut table = [INVALID; 256];
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (i, &c) in alphabet.iter().enumerate() {
        table[c as usize] = i as u8;
    }

    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = table[byte as usize];
        if value == INVALID {
            return None;
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_a_png_header() {
        // The eight bytes every PNG starts with, so a decode that drops or
        // shifts one shows up as an image the decoder rejects rather than as a
        // subtly wrong picture.
        let decoded = decode_base64("iVBORw0KGgo=").expect("decodes");
        assert_eq!(decoded, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn base64_refuses_what_is_not_base64() {
        assert!(decode_base64("not base64!").is_none());
    }

    #[test]
    fn base64_ignores_the_line_breaks_an_encoder_adds() {
        let decoded = decode_base64("iVBO\nRw0K\nGgo=").expect("decodes");
        assert_eq!(decoded.len(), 8);
    }
}
