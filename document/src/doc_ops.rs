//! The built-in document verbs.
//!
//! Each wraps a command that already exists — this is a naming layer, not a
//! second implementation. What it adds is the argument shape: a caller with no
//! selection has to name its target, and names resolve here rather than in
//! every caller.
//!
//! Deliberately small. These are the verbs a script needs to *build* a rig; the
//! editor's own operators cover what needs a selection or a tool.

use crate::args::{Args, Resolver};
use crate::commands::{bone_cmds, key_cmds, slot_cmds};
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::Bone;
use serde_json::json;

/// Create a bone.
pub struct CreateBone;

impl DocOperator for CreateBone {
    fn id(&self) -> &'static str {
        "bone.create"
    }

    fn label(&self) -> &str {
        "Create Bone"
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
                    "description": "Name of the parent bone; omit for a root"
                },
                "x": { "type": "number", "default": 0 },
                "y": { "type": "number", "default": 0 },
                "rotation": { "type": "number", "description": "Degrees", "default": 0 },
                "length": { "type": "number", "default": 30 }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        // Every argument is read before the document is touched: a half-built
        // bone left behind by a bad `parent` is an edit nobody asked for.
        let resolver = Resolver::new(&edit.doc);
        let name = args.str("name")?.to_string();
        let parent = resolver.opt_bone(args, "parent")?;
        let x = args.f32_or("x", 0.0)?;
        let y = args.f32_or("y", 0.0)?;
        let rotation = args.f32_or("rotation", 0.0)?;
        let length = args.f32_or("length", 30.0)?;

        edit.dispatch(Box::new(bone_cmds::CreateBone::new(Bone {
            name,
            parent,
            length: length.max(1.0),
            local_transform: Transform {
                position: glam::vec2(x, y),
                // Degrees at the boundary, radians inside `core`.
                rotation: rotation.to_radians(),
                ..Default::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        })))?;
        Ok(())
    }
}

/// Move, turn or scale a bone's setup transform.
pub struct SetBoneTransform;

impl DocOperator for SetBoneTransform {
    fn id(&self) -> &'static str {
        "bone.set_transform"
    }

    fn label(&self) -> &str {
        "Set Bone Transform"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["bone"],
            "properties": {
                "bone": { "type": "string" },
                "x": { "type": "number" },
                "y": { "type": "number" },
                "rotation": { "type": "number", "description": "Degrees" },
                "scale_x": { "type": "number" },
                "scale_y": { "type": "number" }
            },
            "description": "Omitted fields keep their current value, so one axis \
                            can be edited without restating the rest"
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let id = resolver.bone(args, "bone")?;
        let current = edit.doc.skeleton.bones[id].local_transform;

        // Absent means "leave it", not "zero it". A caller nudging one axis
        // should not have to restate the other five to avoid flattening them.
        let after = Transform {
            position: glam::vec2(
                args.f32_or("x", current.position.x)?,
                args.f32_or("y", current.position.y)?,
            ),
            rotation: args
                .f32_or("rotation", current.rotation.to_degrees())?
                .to_radians(),
            scale: glam::vec2(
                args.f32_or("scale_x", current.scale.x)?,
                args.f32_or("scale_y", current.scale.y)?,
            ),
            shear: current.shear,
        };

        edit.dispatch(Box::new(bone_cmds::SetBoneTransform::new(id, after)))?;
        Ok(())
    }
}

/// Rename a bone.
pub struct RenameBone;

impl DocOperator for RenameBone {
    fn id(&self) -> &'static str {
        "bone.rename"
    }

    fn label(&self) -> &str {
        "Rename Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["bone", "name"],
            "properties": {
                "bone": { "type": "string", "description": "Current name" },
                "name": { "type": "string", "description": "New name" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let id = resolver.bone(args, "bone")?;
        let name = args.str("name")?.to_string();
        edit.dispatch(Box::new(bone_cmds::RenameBone::new(id, name)))?;
        Ok(())
    }
}

/// Delete a bone and everything under it.
pub struct DeleteBone;

impl DocOperator for DeleteBone {
    fn id(&self) -> &'static str {
        "bone.delete"
    }

    fn label(&self) -> &str {
        "Delete Bone"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["bone"],
            "properties": { "bone": { "type": "string" } },
            "description": "Deletes the subtree, as the editor's own delete does"
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let id = resolver.bone(args, "bone")?;
        edit.dispatch(Box::new(bone_cmds::DeleteBone::new(id)))?;
        Ok(())
    }
}

/// Create a slot on a bone.
pub struct CreateSlot;

impl DocOperator for CreateSlot {
    fn id(&self) -> &'static str {
        "slot.create"
    }

    fn label(&self) -> &str {
        "Create Slot"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "bone"],
            "properties": {
                "name": { "type": "string" },
                "bone": { "type": "string", "description": "Bone the slot hangs from" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let bone = resolver.bone(args, "bone")?;
        let name = args.str("name")?.to_string();
        edit.dispatch(Box::new(slot_cmds::CreateSlot::new(name, bone)))?;
        Ok(())
    }
}

/// Create an animation clip.
pub struct CreateAnimation;

impl DocOperator for CreateAnimation {
    fn id(&self) -> &'static str {
        "anim.create"
    }

    fn label(&self) -> &str {
        "Create Animation"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "duration": { "type": "number", "description": "Seconds", "default": 1 }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let duration = args.f32_or("duration", 1.0)?;
        edit.dispatch(Box::new(key_cmds::CreateAnimation::new(name, duration)))?;
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(CreateBone));
    ops.register(Box::new(SetBoneTransform));
    ops.register(Box::new(RenameBone));
    ops.register(Box::new(DeleteBone));
    ops.register(Box::new(CreateSlot));
    ops.register(Box::new(CreateAnimation));
}
