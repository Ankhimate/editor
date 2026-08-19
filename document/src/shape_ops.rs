//! Non-drawing attachments, animation management, and the rest of the bone verbs.
//!
//! The last batch that a script can meaningfully drive. What is left out is left
//! out on purpose: painting weights is a brush stroke, editing a mesh is a
//! vertex drag, and a viewport group gizmo has no meaning without a viewport.
//! Those stay editor-only, which is the rule `doc_ops` already states — the
//! editor's own operators cover what needs a selection or a tool.
//!
//! **Polygons are set outright, not nudged.** The commands underneath take
//! per-index deltas because that is what a drag produces; a script has a shape
//! in mind and wants to state it. So these verbs take the whole polygon and turn
//! it into whatever sequence of inserts, moves and removes gets there — the
//! naming layer's job, the same way resolving a bone name is.

use crate::args::{Args, Resolver};
use crate::commands::{asset_cmds, bone_cmds, clip_cmds, key_cmds};
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use ankhimate_core::ids::{SkinId, SlotId};
use serde_json::json;

/// Read `[[x, y], …]` into local-space points.
fn polygon(args: &Args, key: &str) -> Result<Vec<glam::Vec2>, OpError> {
    let value = args
        .as_json()
        .get(key)
        .ok_or_else(|| OpError::Args(crate::args::ArgError::Missing(key.into())))?;
    let rows = value.as_array().ok_or_else(|| {
        OpError::Args(crate::args::ArgError::WrongType {
            key: key.into(),
            wanted: "an array of [x, y] pairs",
            got: "something else",
        })
    })?;

    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let pair = row.as_array().filter(|p| p.len() == 2).ok_or_else(|| {
            OpError::Args(crate::args::ArgError::WrongType {
                key: key.into(),
                wanted: "an array of [x, y] pairs",
                got: "a row that is not a pair",
            })
        })?;
        let read = |v: &serde_json::Value| {
            v.as_f64().map(|n| n as f32).ok_or_else(|| {
                OpError::Args(crate::args::ArgError::WrongType {
                    key: key.into(),
                    wanted: "numbers",
                    got: "something else",
                })
            })
        };
        points.push(glam::vec2(read(&pair[0])?, read(&pair[1])?));
    }

    // Three points is the least that encloses anything. Two is a line, which
    // would import as a shape that clips or hits nothing — a rig that looks
    // finished and does not work.
    if points.len() < 3 {
        return Err(OpError::Args(crate::args::ArgError::WrongType {
            key: key.into(),
            wanted: "at least three points",
            got: "fewer",
        }));
    }
    Ok(points)
}

/// Which skin and slot a shape verb was pointed at.
fn target(edit: &Edit, args: &Args) -> Result<(SkinId, SlotId), OpError> {
    let resolver = Resolver::new(&edit.doc);
    Ok((
        resolver.skin_or_default(args, "skin")?,
        resolver.slot(args, "slot")?,
    ))
}

/// Replace a polygon's vertices with the ones given.
///
/// The commands take per-index edits; a caller has a shape. Insert or remove to
/// reach the right count, then move every vertex into place.
fn reshape(
    edit: &mut Edit,
    skin: SkinId,
    slot: SlotId,
    name: &str,
    points: &[glam::Vec2],
    is_box: bool,
) -> Result<(), OpError> {
    use ankhimate_core::attachment::Attachment;

    let current = match edit.doc.skeleton.skins[skin].get(slot, name) {
        Some(Attachment::Clipping(clip)) => clip.vertices.len(),
        Some(Attachment::BoundingBox(bb)) => bb.vertices.len(),
        _ => return Ok(()),
    };

    let dispatch = |edit: &mut Edit, e: clip_cmds::ClipEdit| -> Result<(), OpError> {
        if is_box {
            edit.dispatch(Box::new(clip_cmds::EditBoundingBox::new(
                skin, slot, name, e,
            )))?;
        } else {
            edit.dispatch(Box::new(clip_cmds::EditClip::new(skin, slot, name, e)))?;
        }
        Ok(())
    };

    // The order these are listed in does not matter: `RemoveVertices` sorts
    // descending itself, which is where that hazard is handled. Said here
    // because the obvious defensive `.rev()` looks load-bearing and is not —
    // a comment claiming to prevent a bug it cannot is worse than none.
    if current > points.len() {
        let doomed: Vec<usize> = (points.len()..current).collect();
        dispatch(edit, clip_cmds::ClipEdit::RemoveVertices(doomed))?;
    }
    for index in current..points.len() {
        dispatch(
            edit,
            clip_cmds::ClipEdit::InsertVertex(index, points[index]),
        )?;
    }
    let moves: Vec<(usize, glam::Vec2)> = points.iter().copied().enumerate().collect();
    dispatch(edit, clip_cmds::ClipEdit::MoveVertices(moves))?;
    Ok(())
}

/// A hitbox: a polygon that answers "was this hit?" and draws nothing.
pub struct CreateBoundingBox;

impl DocOperator for CreateBoundingBox {
    fn id(&self) -> &'static str {
        "attachment.create_box"
    }

    fn label(&self) -> &str {
        "Create Bounding Box"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "name"],
            "properties": {
                "slot": { "type": "string" },
                "name": { "type": "string" },
                "skin": { "type": "string" },
                "points": {
                    "type": "array",
                    "items": { "type": "array", "items": { "type": "number" } },
                    "description": "[[x, y], …] in the bone's local space; at least three"
                },
                "size": {
                    "type": "number",
                    "description": "Side of the default square, when no points are given",
                    "default": 40
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot) = target(edit, args)?;
        let name = args.str("name")?.to_string();
        // Read before anything is created: a shape half-built by a bad polygon
        // is worse than none, because the script cannot tell it happened.
        let points = match args.as_json().get("points") {
            None => None,
            Some(_) => Some(polygon(args, "points")?),
        };
        let size = args.f32_or("size", 40.0)?;

        edit.dispatch(Box::new(clip_cmds::AddBoundingBox::new(
            skin,
            slot,
            name.clone(),
            size,
        )))?;
        if let Some(points) = points {
            reshape(edit, skin, slot, &name, &points, true)?;
        }
        Ok(())
    }
}

/// A clipping polygon: everything drawn after it is masked to this shape.
pub struct CreateClipping;

impl DocOperator for CreateClipping {
    fn id(&self) -> &'static str {
        "attachment.create_clip"
    }

    fn label(&self) -> &str {
        "Create Clipping Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "name"],
            "properties": {
                "slot": { "type": "string" },
                "name": { "type": "string" },
                "skin": { "type": "string" },
                "points": {
                    "type": "array",
                    "items": { "type": "array", "items": { "type": "number" } },
                    "description": "[[x, y], …]; at least three"
                },
                "size": { "type": "number", "default": 40 },
                "end_slot": {
                    "type": "string",
                    "description": "Stop clipping after this slot; omit to clip to the end"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot) = target(edit, args)?;
        let name = args.str("name")?.to_string();
        let points = match args.as_json().get("points") {
            None => None,
            Some(_) => Some(polygon(args, "points")?),
        };
        let size = args.f32_or("size", 40.0)?;
        let end_slot = args.opt_str("end_slot")?.map(str::to_string);

        edit.dispatch(Box::new(clip_cmds::AddClipping::new(
            skin,
            slot,
            name.clone(),
            size,
        )))?;
        if let Some(points) = points {
            reshape(edit, skin, slot, &name, &points, false)?;
        }
        if end_slot.is_some() {
            edit.dispatch(Box::new(clip_cmds::EditClip::new(
                skin,
                slot,
                name,
                clip_cmds::ClipEdit::SetEndSlot(end_slot),
            )))?;
        }
        Ok(())
    }
}

/// A point: a position and a direction, for hanging effects off.
pub struct CreatePoint;

impl DocOperator for CreatePoint {
    fn id(&self) -> &'static str {
        "attachment.create_point"
    }

    fn label(&self) -> &str {
        "Create Point Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "name"],
            "properties": {
                "slot": { "type": "string" },
                "name": { "type": "string" },
                "skin": { "type": "string" },
                "x": { "type": "number", "default": 0 },
                "y": { "type": "number", "default": 0 },
                "rotation": { "type": "number", "description": "Degrees", "default": 0 }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot) = target(edit, args)?;
        let name = args.str("name")?.to_string();
        let x = args.f32_or("x", 0.0)?;
        let y = args.f32_or("y", 0.0)?;
        let rotation = args.f32_or("rotation", 0.0)?;

        edit.dispatch(Box::new(clip_cmds::AddPoint::new(skin, slot, name.clone())))?;
        edit.dispatch(Box::new(clip_cmds::SetPoint::new(
            skin,
            slot,
            name,
            ankhimate_core::attachment::PointAttachment {
                position: glam::vec2(x, y),
                // Degrees at the boundary, radians inside `core` (PLAN §2.7).
                rotation: rotation.to_radians(),
            },
        )))?;
        Ok(())
    }
}

/// Rename an animation.
pub struct RenameAnimation;

impl DocOperator for RenameAnimation {
    fn id(&self) -> &'static str {
        "anim.rename"
    }

    fn label(&self) -> &str {
        "Rename Animation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "to"],
            "properties": {
                "animation": { "type": "string" },
                "to": { "type": "string" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let to = args.str("to")?.to_string();
        edit.dispatch(Box::new(key_cmds::RenameAnimation::new(anim, to)))?;
        Ok(())
    }
}

/// Copy an animation, keys and all.
pub struct DuplicateAnimation;

impl DocOperator for DuplicateAnimation {
    fn id(&self) -> &'static str {
        "anim.duplicate"
    }

    fn label(&self) -> &str {
        "Duplicate Animation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation"],
            "properties": { "animation": { "type": "string" } }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        edit.dispatch(Box::new(key_cmds::DuplicateAnimation::new(anim)))?;
        Ok(())
    }
}

/// Delete an animation.
pub struct DeleteAnimation;

impl DocOperator for DeleteAnimation {
    fn id(&self) -> &'static str {
        "anim.delete"
    }

    fn label(&self) -> &str {
        "Delete Animation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation"],
            "properties": { "animation": { "type": "string" } }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        edit.dispatch(Box::new(key_cmds::DeleteAnimation::new(anim)))?;
        Ok(())
    }
}

/// An animation's length and whether it loops.
pub struct SetAnimation;

impl DocOperator for SetAnimation {
    fn id(&self) -> &'static str {
        "anim.set_meta"
    }

    fn label(&self) -> &str {
        "Set Animation Length"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation"],
            "properties": {
                "animation": { "type": "string" },
                "duration": { "type": "number", "description": "Seconds" },
                "looping": { "type": "boolean" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let current = &edit.doc.animations[anim];
        let duration = args.f32_or("duration", current.duration)?;
        let looping = args.bool_or("looping", current.looping)?;
        edit.dispatch(Box::new(key_cmds::SetAnimationMeta::new(
            anim, duration, looping,
        )))?;
        Ok(())
    }
}

/// Scale every key's time, so a clip plays faster or slower.
pub struct RetimeAnimation;

impl DocOperator for RetimeAnimation {
    fn id(&self) -> &'static str {
        "anim.retime"
    }

    fn label(&self) -> &str {
        "Retime Animation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "scale"],
            "properties": {
                "animation": { "type": "string" },
                "scale": {
                    "type": "number",
                    "description": "2 makes it twice as long, 0.5 half"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let scale = args.f32("scale")?;
        if scale <= 0.0 {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "scale".into(),
                wanted: "a positive number",
                got: "zero or less, which would collapse every key onto one moment",
            }));
        }
        edit.dispatch(Box::new(key_cmds::RetimeAnimation::scaled(anim, scale)))?;
        Ok(())
    }
}

/// Move a bone under a different parent.
pub struct SetBoneParent;

impl DocOperator for SetBoneParent {
    fn id(&self) -> &'static str {
        "bone.set_parent"
    }

    fn label(&self) -> &str {
        "Reparent Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "parent": {
                    "type": "string",
                    "description": "Omit to make it a root"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let bone = resolver.bone(args, "name")?;
        let parent = resolver.opt_bone(args, "parent")?;
        // `keeping_world`, not `new`: a bone whose parent changed has not moved
        // (T-206), and a verb that let it jump would make reparenting an edit
        // nobody could use without fixing the pose afterwards.
        edit.dispatch(Box::new(bone_cmds::SetBoneParent::keeping_world(
            &edit.doc.skeleton,
            bone,
            parent,
        )))?;
        Ok(())
    }
}

/// A bone's length, which is what the gizmo draws and what IK measures.
pub struct SetBoneLength;

impl DocOperator for SetBoneLength {
    fn id(&self) -> &'static str {
        "bone.set_length"
    }

    fn label(&self) -> &str {
        "Set Bone Length"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "length"],
            "properties": {
                "name": { "type": "string" },
                "length": { "type": "number" },
                "carry_children": {
                    "type": "boolean",
                    "description": "Move children sitting at the tip along with it",
                    "default": true
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let bone = resolver.bone(args, "name")?;
        let length = args.f32("length")?;
        // Children at the tip follow by default: lengthening a forearm should
        // take the hand with it, which is what an artist dragging the tip sees.
        let carry_children = args.bool_or("carry_children", true)?;
        edit.dispatch(Box::new(bone_cmds::SetBoneLength::new(
            bone,
            length,
            carry_children,
        )))?;
        Ok(())
    }
}

/// A bone's colour in the hierarchy and the viewport.
pub struct SetBoneColor;

impl DocOperator for SetBoneColor {
    fn id(&self) -> &'static str {
        "bone.set_color"
    }

    fn label(&self) -> &str {
        "Set Bone Color"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "color"],
            "properties": {
                "name": { "type": "string" },
                "color": {
                    "type": "array",
                    "items": { "type": "number" },
                    "description": "[r, g, b] or [r, g, b, a], each 0..1"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let bone = resolver.bone(args, "name")?;
        let current = edit
            .doc
            .skeleton
            .bones
            .get(bone)
            .map(|b| b.color)
            .unwrap_or([1.0; 4]);
        let list = args.f32_list("color")?;
        // Three components leaves alpha alone, as everywhere else in this API.
        let color = match list.len() {
            3 => [list[0], list[1], list[2], current[3]],
            4 => [list[0], list[1], list[2], list[3]],
            _ => {
                return Err(OpError::Args(crate::args::ArgError::WrongType {
                    key: "color".into(),
                    wanted: "three or four numbers, 0..1",
                    got: "a list of another length",
                }));
            }
        };
        edit.dispatch(Box::new(bone_cmds::SetBoneColor::new(bone, color)))?;
        Ok(())
    }
}

/// Rename an image in the asset library.
pub struct RenameAsset;

impl DocOperator for RenameAsset {
    fn id(&self) -> &'static str {
        "asset.rename"
    }

    fn label(&self) -> &str {
        "Rename Image"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "to"],
            "properties": {
                "name": { "type": "string" },
                "to": { "type": "string" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let id = edit.doc.assets.by_name(&name).ok_or_else(|| {
            OpError::Args(crate::args::ArgError::Unresolved {
                key: "name".into(),
                kind: "image",
                name: name.clone(),
            })
        })?;
        let to = args.str("to")?.to_string();
        edit.dispatch(Box::new(asset_cmds::RenameAsset::new(id, to)))?;
        Ok(())
    }
}

/// Remove an image from the asset library.
pub struct DeleteAsset;

impl DocOperator for DeleteAsset {
    fn id(&self) -> &'static str {
        "asset.delete"
    }

    fn label(&self) -> &str {
        "Delete Image"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" } }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let id = edit.doc.assets.by_name(&name).ok_or_else(|| {
            OpError::Args(crate::args::ArgError::Unresolved {
                key: "name".into(),
                kind: "image",
                name: name.clone(),
            })
        })?;
        edit.dispatch(Box::new(asset_cmds::DeleteAsset::new(id)))?;
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(CreateBoundingBox));
    ops.register(Box::new(CreateClipping));
    ops.register(Box::new(CreatePoint));
    ops.register(Box::new(RenameAnimation));
    ops.register(Box::new(DuplicateAnimation));
    ops.register(Box::new(DeleteAnimation));
    ops.register(Box::new(SetAnimation));
    ops.register(Box::new(RetimeAnimation));
    ops.register(Box::new(SetBoneParent));
    ops.register(Box::new(SetBoneLength));
    ops.register(Box::new(SetBoneColor));
    ops.register(Box::new(RenameAsset));
    ops.register(Box::new(DeleteAsset));
}
