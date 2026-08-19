//! Constraint verbs.
//!
//! The gap these close is a specific one: a plugin could build bones, slots and
//! artwork but not a single constraint, so a plugin importer could not match the
//! built-in one. The PSD reader creates IK from `[ik]` and physics from
//! `[physics:cloth]`, and a JavaScript importer of any other layered format had
//! no way to say the same thing.
//!
//! Four kinds, and their properties. Creating and configuring are separate
//! verbs because they are separate decisions — an importer creates, a rigging
//! script tunes, and a caller that only wants to change a mix should not have to
//! restate the chain.
//!
//! Like the rest of the verb surface, angles are **degrees** here and radians
//! inside `core` (PLAN §2.7).

use crate::args::{Args, Resolver};
use crate::commands::constraint_cmds;
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use ankhimate_core::ids::ConstraintId;
use serde_json::json;

/// Find a constraint by name, for the verbs that edit rather than create.
fn constraint_named(edit: &Edit, name: &str) -> Result<ConstraintId, OpError> {
    edit.doc
        .skeleton
        .constraints
        .iter()
        .find(|(_, c)| c.name() == name)
        .map(|(id, _)| id)
        .ok_or_else(|| {
            OpError::Args(crate::args::ArgError::Unresolved {
                key: "name".into(),
                kind: "constraint",
                name: name.to_string(),
            })
        })
}

/// A constraint that exists but is the wrong kind for this verb.
///
/// Reported as an argument error rather than a refusal: `Refused` means the
/// mode rule said no, and folding a different failure into it would make the
/// two indistinguishable to a caller trying to recover.
fn wrong_kind(name: &str, wanted: &'static str) -> OpError {
    // The name goes in the key rather than being dropped: "a constraint of
    // another kind" without saying which one is a message that sends the
    // reader back to their script to guess.
    OpError::Args(crate::args::ArgError::WrongType {
        key: format!("name (`{name}`)"),
        wanted,
        got: "a constraint of another kind",
    })
}

/// IK over a chain of bones, with a target bone created at the chain's tip.
///
/// The target is created rather than named because it is the handle an animator
/// drags, and a chain with no handle is an IK constraint nobody can pose.
pub struct CreateIk;

impl DocOperator for CreateIk {
    fn id(&self) -> &'static str {
        "constraint.create_ik"
    }

    fn label(&self) -> &str {
        "Create IK Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "bones"],
            "properties": {
                "name": { "type": "string" },
                "bones": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The chain, root first and tip last. Order is the argument."
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let name = args.str("name")?.to_string();
        let chain = resolver.bone_list(args, "bones")?;
        if chain.len() < 2 {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "bones".into(),
                wanted: "at least two bones, for IK to bend between",
                got: "fewer",
            }));
        }

        // The command has no pose, so the caller resolves where the target
        // goes. The tip's world position is the only answer that puts the
        // handle where the animator expects to grab it.
        let mut pose = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&edit.doc.skeleton, &[], &mut pose);
        let tip = *chain.last().expect("checked non-empty");
        let position = pose
            .worlds
            .get(tip)
            .map(|world| world.transform_point(glam::Vec2::ZERO))
            .unwrap_or_default();

        edit.dispatch(Box::new(constraint_cmds::CreateIkTarget::new(
            chain, name, position,
        )))?;
        Ok(())
    }
}

/// Retune an IK constraint.
pub struct SetIk;

impl DocOperator for SetIk {
    fn id(&self) -> &'static str {
        "constraint.set_ik"
    }

    fn label(&self) -> &str {
        "Edit IK Constraint"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "mix": { "type": "number", "description": "0 is off, 1 is fully solved" },
                "bend_direction": {
                    "type": "number",
                    "description": "1 or -1 — which way the elbow points"
                },
                "softness": { "type": "number" },
                "stretch": { "type": "boolean" },
                "stretch_limit": { "type": "number" },
                "stiffness": { "type": "number" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let id = constraint_named(edit, &name)?;
        // Read from what is there, so an argument left out means "leave it" and
        // not "reset it to a default the caller never saw".
        let current = match edit.doc.skeleton.constraints.get(id) {
            Some(ankhimate_core::constraints::Constraint::Ik(ik)) => ik.clone(),
            _ => return Err(wrong_kind(&name, "an IK constraint")),
        };

        // From the constraint's own constructor, then overridden per argument.
        // The chain and target are not editable here: what a constraint acts on
        // is a different decision from how strongly it acts, and a verb that
        // took both would let a typo in `bones` rebuild it rather than fail.
        let mut props = constraint_cmds::IkProps::from_constraint(&current);
        props.mix = args.f32_or("mix", current.mix)?;
        props.bend_direction = args.f32_or("bend_direction", current.bend_direction)?;
        props.softness = args.f32_or("softness", current.softness)?;
        props.stretch = args.bool_or("stretch", current.stretch)?;
        props.stretch_limit = args.f32_or("stretch_limit", current.stretch_limit)?;
        props.stiffness = args.f32_or("stiffness", current.stiffness)?;

        edit.dispatch(Box::new(constraint_cmds::SetIkProps::new(id, props)))?;
        Ok(())
    }
}

/// A transform constraint: one bone drives others.
pub struct CreateTransform;

impl DocOperator for CreateTransform {
    fn id(&self) -> &'static str {
        "constraint.create_transform"
    }

    fn label(&self) -> &str {
        "Create Transform Constraint"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "target", "bones"],
            "properties": {
                "name": { "type": "string" },
                "target": { "type": "string", "description": "The bone that drives" },
                "bones": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The bones that follow"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let name = args.str("name")?.to_string();
        let target = resolver.bone(args, "target")?;
        let bones = resolver.bone_list(args, "bones")?;
        if bones.is_empty() {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "bones".into(),
                wanted: "at least one bone to drive",
                got: "an empty list",
            }));
        }
        edit.dispatch(Box::new(constraint_cmds::AddTransformConstraint::new(
            name, target, bones,
        )))?;
        Ok(())
    }
}

/// Retune a transform constraint's per-channel mixes.
pub struct SetTransform;

impl DocOperator for SetTransform {
    fn id(&self) -> &'static str {
        "constraint.set_transform"
    }

    fn label(&self) -> &str {
        "Edit Transform Constraint"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "mix_rotate": { "type": "number" },
                "mix_translate_x": { "type": "number" },
                "mix_translate_y": { "type": "number" },
                "mix_scale_x": { "type": "number" },
                "mix_scale_y": { "type": "number" },
                "mix_shear_x": { "type": "number" },
                "mix_shear_y": { "type": "number" },
                "offset_rotation": { "type": "number", "description": "Degrees" },
                "offset_x": { "type": "number" },
                "offset_y": { "type": "number" },
                "relative": { "type": "boolean" },
                "local": { "type": "boolean" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let id = constraint_named(edit, &name)?;
        let current = match edit.doc.skeleton.constraints.get(id) {
            Some(ankhimate_core::constraints::Constraint::Transform(t)) => t.clone(),
            _ => {
                return Err(wrong_kind(&name, "a transform constraint"));
            }
        };

        // Built from what is there, so an argument left out means "leave it"
        // rather than "reset it to a default the caller never saw". The
        // constraint's own constructor is the source: reconstructing the struct
        // field by field here is how a new field ends up silently zeroed.
        let mut props = constraint_cmds::TransformProps::from_constraint(&current);

        props.mix = ankhimate_core::constraints::TransformMix {
            rotate: args.f32_or("mix_rotate", current.mix.rotate)?,
            translate: glam::vec2(
                args.f32_or("mix_translate_x", current.mix.translate.x)?,
                args.f32_or("mix_translate_y", current.mix.translate.y)?,
            ),
            scale: glam::vec2(
                args.f32_or("mix_scale_x", current.mix.scale.x)?,
                args.f32_or("mix_scale_y", current.mix.scale.y)?,
            ),
            shear: glam::vec2(
                args.f32_or("mix_shear_x", current.mix.shear.x)?,
                args.f32_or("mix_shear_y", current.mix.shear.y)?,
            ),
        };
        // Degrees at the boundary, radians inside `core`.
        props.offsets.rotation = args
            .f32_or("offset_rotation", current.offsets.rotation.to_degrees())?
            .to_radians();
        props.offsets.position = glam::vec2(
            args.f32_or("offset_x", current.offsets.position.x)?,
            args.f32_or("offset_y", current.offsets.position.y)?,
        );
        props.relative = args.bool_or("relative", current.relative)?;
        props.local = args.bool_or("local", current.local)?;

        edit.dispatch(Box::new(constraint_cmds::SetTransformProps::new(id, props)))?;
        Ok(())
    }
}

/// Physics on a bone: it sways, lags and settles.
pub struct CreatePhysics;

impl DocOperator for CreatePhysics {
    fn id(&self) -> &'static str {
        "constraint.create_physics"
    }

    fn label(&self) -> &str {
        "Create Physics Constraint"
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
                "bone": { "type": "string", "description": "The bone that sways" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let name = args.str("name")?.to_string();
        let bone = resolver.bone(args, "bone")?;
        edit.dispatch(Box::new(constraint_cmds::AddPhysics::new(bone, name)))?;
        Ok(())
    }
}

/// Retune a physics constraint.
pub struct SetPhysics;

impl DocOperator for SetPhysics {
    fn id(&self) -> &'static str {
        "constraint.set_physics"
    }

    fn label(&self) -> &str {
        "Edit Physics Constraint"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "inertia": { "type": "number", "description": "0..1 — higher lags more" },
                "strength": { "type": "number" },
                "damping": { "type": "number", "description": "0..1 — at 0 it never settles" },
                "mass": { "type": "number" },
                "wind_x": { "type": "number" },
                "wind_y": { "type": "number" },
                "gravity_x": { "type": "number" },
                "gravity_y": { "type": "number", "description": "Negative is down" },
                "mix": { "type": "number" },
                "rotate": { "type": "boolean" },
                "translate": { "type": "boolean" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let name = args.str("name")?.to_string();
        let id = constraint_named(edit, &name)?;
        let current = match edit.doc.skeleton.constraints.get(id) {
            Some(ankhimate_core::constraints::Constraint::Physics(p)) => p.clone(),
            _ => {
                return Err(wrong_kind(&name, "a physics constraint"));
            }
        };

        let mut props = constraint_cmds::PhysicsProps::from_constraint(&current);
        props.inertia = args.f32_or("inertia", current.inertia)?;
        props.strength = args.f32_or("strength", current.strength)?;
        props.damping = args.f32_or("damping", current.damping)?;
        props.mass = args.f32_or("mass", current.mass)?;
        props.wind = glam::vec2(
            args.f32_or("wind_x", current.wind.x)?,
            args.f32_or("wind_y", current.wind.y)?,
        );
        props.gravity = glam::vec2(
            args.f32_or("gravity_x", current.gravity.x)?,
            args.f32_or("gravity_y", current.gravity.y)?,
        );
        props.mix = args.f32_or("mix", current.mix)?;
        props.rotate = args.bool_or("rotate", current.rotate)?;
        props.translate = args.bool_or("translate", current.translate)?;

        edit.dispatch(Box::new(constraint_cmds::SetPhysicsProps::new(id, props)))?;
        Ok(())
    }
}

/// Remove a constraint by name.
pub struct DeleteConstraint;

impl DocOperator for DeleteConstraint {
    fn id(&self) -> &'static str {
        "constraint.delete"
    }

    fn label(&self) -> &str {
        "Delete Constraint"
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
        let id = constraint_named(edit, &name)?;
        edit.dispatch(Box::new(constraint_cmds::RemoveConstraint::new(id)))?;
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(CreateIk));
    ops.register(Box::new(SetIk));
    ops.register(Box::new(CreateTransform));
    ops.register(Box::new(SetTransform));
    ops.register(Box::new(CreatePhysics));
    ops.register(Box::new(SetPhysics));
    ops.register(Box::new(DeleteConstraint));
}
