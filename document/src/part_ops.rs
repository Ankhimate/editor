//! Attachment and event verbs.
//!
//! Two gaps, and the second shaped its own API.
//!
//! **Attachments** could be created and never touched again: no rename, no
//! duplicate, no remove, and no way to set a region's transform after the fact.
//! An importer that placed art wrongly had to be right the first time.
//!
//! **Events** are stored as an ordered list and every command addresses one by
//! index. A script would have to count, and a count is wrong the moment anything
//! else inserts an event — so these verbs name an event by `(name, time)` and
//! find the index themselves. The commands keep their indices; this is the
//! naming layer's job, exactly as resolving a bone name is.

use crate::args::{Args, Resolver};
use crate::commands::{attachment_cmds, event_cmds};
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use ankhimate_core::ids::{AnimationId, SkinId, SlotId};
use serde_json::json;

/// Times within this many seconds are the same event.
///
/// A script that read a time out of the document and passed it back must find
/// the same event, and a float that survived a JSON round trip is not bit-equal.
const SAME_TIME: f32 = 1e-4;

/// The attachment a verb was pointed at: which skin, which slot, what name.
fn attachment_of(edit: &Edit, args: &Args) -> Result<(SkinId, SlotId, String), OpError> {
    let resolver = Resolver::new(&edit.doc);
    let skin = resolver.skin_or_default(args, "skin")?;
    let slot = resolver.slot(args, "slot")?;
    let name = args.str("attachment")?.to_string();

    if edit.doc.skeleton.skins[skin].get(slot, &name).is_none() {
        return Err(OpError::Args(crate::args::ArgError::Unresolved {
            key: "attachment".into(),
            kind: "attachment",
            name,
        }));
    }
    Ok((skin, slot, name))
}

/// Find an event by name and time rather than by position in the list.
fn event_index(edit: &Edit, anim: AnimationId, name: &str, time: f32) -> Result<usize, OpError> {
    edit.doc
        .animations
        .get(anim)
        .and_then(|a| {
            a.events
                .iter()
                .position(|e| e.name == name && (e.time - time).abs() < SAME_TIME)
        })
        .ok_or_else(|| {
            OpError::Args(crate::args::ArgError::Unresolved {
                key: "event".into(),
                kind: "event",
                name: format!("{name} at {time}s"),
            })
        })
}

/// Rename an attachment within one skin.
pub struct RenameAttachment;

impl DocOperator for RenameAttachment {
    fn id(&self) -> &'static str {
        "attachment.rename"
    }

    fn label(&self) -> &str {
        "Rename Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "attachment", "to"],
            "properties": {
                "slot": { "type": "string" },
                "attachment": { "type": "string" },
                "to": { "type": "string" },
                "skin": { "type": "string", "description": "Defaults to the base skin" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot, name) = attachment_of(edit, args)?;
        let to = args.str("to")?.to_string();
        edit.dispatch(Box::new(attachment_cmds::RenameAttachment::new(
            skin, slot, name, to,
        )))?;
        Ok(())
    }
}

/// Copy an attachment beside itself, under a fresh name.
pub struct DuplicateAttachment;

impl DocOperator for DuplicateAttachment {
    fn id(&self) -> &'static str {
        "attachment.duplicate"
    }

    fn label(&self) -> &str {
        "Duplicate Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "attachment"],
            "properties": {
                "slot": { "type": "string" },
                "attachment": { "type": "string" },
                "skin": { "type": "string" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot, name) = attachment_of(edit, args)?;
        edit.dispatch(Box::new(attachment_cmds::DuplicateAttachment::new(
            skin, slot, name,
        )))?;
        Ok(())
    }
}

/// Remove an attachment from one skin.
pub struct RemoveAttachment;

impl DocOperator for RemoveAttachment {
    fn id(&self) -> &'static str {
        "attachment.remove"
    }

    fn label(&self) -> &str {
        "Remove Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "attachment"],
            "properties": {
                "slot": { "type": "string" },
                "attachment": { "type": "string" },
                "skin": { "type": "string" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let (skin, slot, name) = attachment_of(edit, args)?;
        edit.dispatch(Box::new(attachment_cmds::RemoveAttachment::new(
            skin, slot, name,
        )))?;
        Ok(())
    }
}

/// Where a region sits in its slot: offset, rotation, scale, size, pivot.
pub struct SetRegion;

impl DocOperator for SetRegion {
    fn id(&self) -> &'static str {
        "attachment.set_region"
    }

    fn label(&self) -> &str {
        "Edit Region Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "attachment"],
            "properties": {
                "slot": { "type": "string" },
                "attachment": { "type": "string" },
                "skin": { "type": "string" },
                "x": { "type": "number" },
                "y": { "type": "number" },
                "rotation": { "type": "number", "description": "Degrees" },
                "scale_x": { "type": "number" },
                "scale_y": { "type": "number" },
                "width": { "type": "number" },
                "height": { "type": "number" },
                "pivot_x": {
                    "type": "number",
                    "description": "0 is the left edge, 1 the right, 0.5 the centre"
                },
                "pivot_y": { "type": "number", "description": "0 is the bottom edge" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        use ankhimate_core::attachment::Attachment;
        let (skin, slot, name) = attachment_of(edit, args)?;

        let region = match edit.doc.skeleton.skins[skin].get(slot, &name) {
            Some(Attachment::Region(region)) => region.clone(),
            _ => {
                return Err(OpError::Args(crate::args::ArgError::WrongType {
                    key: "attachment".into(),
                    wanted: "a region attachment",
                    got: "an attachment of another kind",
                }));
            }
        };

        // From the command's own constructor, then overridden per argument, so
        // a field added later cannot be silently zeroed here.
        let mut props = attachment_cmds::RegionProps::from_region(&region);
        props.offset = glam::vec2(
            args.f32_or("x", region.local_offset.x)?,
            args.f32_or("y", region.local_offset.y)?,
        );
        // Degrees at the boundary, radians inside `core` (PLAN §2.7).
        props.rotation = args
            .f32_or("rotation", region.local_rotation.to_degrees())?
            .to_radians();
        props.scale = glam::vec2(
            args.f32_or("scale_x", region.local_scale.x)?,
            args.f32_or("scale_y", region.local_scale.y)?,
        );
        props.width = args.f32_or("width", region.width)?;
        props.height = args.f32_or("height", region.height)?;

        // The pivot moves the art unless the offset compensates, so this uses
        // the command's own helper rather than assigning the field: an importer
        // setting a shoulder pivot means "turn about here", not "and also jump
        // half a sprite to the left".
        let pivot = glam::vec2(
            args.f32_or("pivot_x", region.pivot.x)?,
            args.f32_or("pivot_y", region.pivot.y)?,
        );
        let props = if pivot != region.pivot {
            props.with_pivot_keeping_position(pivot)
        } else {
            props
        };

        edit.dispatch(Box::new(attachment_cmds::SetRegionProps::new(
            skin, slot, name, props,
        )))?;
        Ok(())
    }
}

/// Add an event to an animation.
pub struct AddEvent;

impl DocOperator for AddEvent {
    fn id(&self) -> &'static str {
        "anim.add_event"
    }

    fn label(&self) -> &str {
        "Add Event"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "name", "time"],
            "properties": {
                "animation": { "type": "string" },
                "name": { "type": "string" },
                "time": { "type": "number", "description": "Seconds" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let name = args.str("name")?.to_string();
        let time = args.f32("time")?;
        edit.dispatch(Box::new(event_cmds::AddEvent::new(anim, name, time)))?;
        Ok(())
    }
}

/// Move an event, rename it, or set what it carries.
pub struct SetEvent;

impl DocOperator for SetEvent {
    fn id(&self) -> &'static str {
        "anim.set_event"
    }

    fn label(&self) -> &str {
        "Edit Event"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "name", "time"],
            "properties": {
                "animation": { "type": "string" },
                "name": {
                    "type": "string",
                    "description": "Which event — with `time`, this identifies it"
                },
                "time": { "type": "number", "description": "Which event, in seconds" },
                "to_name": { "type": "string", "description": "Rename it" },
                "to_time": { "type": "number", "description": "Move it" },
                "int_value": { "type": "integer" },
                "float_value": { "type": "number" },
                "string_value": { "type": "string" },
                "audio": { "type": "string", "description": "Asset name; empty is silent" },
                "volume": { "type": "number" },
                "balance": { "type": "number" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let name = args.str("name")?.to_string();
        let time = args.f32("time")?;
        let index = event_index(edit, anim, &name, time)?;

        let current = edit.doc.animations[anim].events[index].clone();

        // Payload, rename and move are three separate commands, so the caller's
        // one call becomes up to three. Ordered so the identifying pair stays
        // valid: payload and rename first, the move last, because moving an
        // event is what changes the time this verb found it by.
        let touches_payload = [
            "int_value",
            "float_value",
            "string_value",
            "audio",
            "volume",
            "balance",
        ]
        .iter()
        .any(|key| args.as_json().get(key).is_some());
        if touches_payload {
            edit.dispatch(Box::new(event_cmds::EditEvent::new(
                anim,
                index,
                event_cmds::EventEdit::SetPayload {
                    int_value: args.f32_or("int_value", current.int_value as f32)? as i32,
                    float_value: args.f32_or("float_value", current.float_value)?,
                    string_value: args
                        .opt_str("string_value")?
                        .unwrap_or(&current.string_value)
                        .to_string(),
                    audio: args.opt_str("audio")?.unwrap_or(&current.audio).to_string(),
                    volume: args.f32_or("volume", current.volume)?,
                    balance: args.f32_or("balance", current.balance)?,
                },
            )))?;
        }
        if let Some(to_name) = args.opt_str("to_name")? {
            let to_name = to_name.to_string();
            edit.dispatch(Box::new(event_cmds::EditEvent::new(
                anim,
                index,
                event_cmds::EventEdit::Rename(to_name),
            )))?;
        }
        if let Some(to_time) = args.as_json().get("to_time") {
            let _ = to_time;
            let to_time = args.f32("to_time")?;
            edit.dispatch(Box::new(event_cmds::EditEvent::new(
                anim,
                index,
                event_cmds::EventEdit::SetTime(to_time),
            )))?;
        }
        Ok(())
    }
}

/// Remove an event.
pub struct DeleteEvent;

impl DocOperator for DeleteEvent {
    fn id(&self) -> &'static str {
        "anim.delete_event"
    }

    fn label(&self) -> &str {
        "Delete Event"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["animation", "name", "time"],
            "properties": {
                "animation": { "type": "string" },
                "name": { "type": "string" },
                "time": { "type": "number", "description": "Seconds" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let anim = resolver.animation(args, "animation")?;
        let name = args.str("name")?.to_string();
        let time = args.f32("time")?;
        let index = event_index(edit, anim, &name, time)?;
        edit.dispatch(Box::new(event_cmds::EditEvent::new(
            anim,
            index,
            event_cmds::EventEdit::Remove,
        )))?;
        Ok(())
    }
}

/// Switch between Setup and Animate.
///
/// Every other verb declares which mode it needs (T-207) and a script had no
/// way to be in the other one — so the event verbs, which write keys and are
/// therefore Animate-only, were unreachable from JavaScript entirely. Found by
/// a test that assumed otherwise.
///
/// This is not an editor session setting leaking in: mode decides what an edit
/// *means*, so it belongs to the document a script is editing rather than to
/// the window it is not looking at.
pub struct SetMode;

impl DocOperator for SetMode {
    fn id(&self) -> &'static str {
        "doc.set_mode"
    }

    fn label(&self) -> &str {
        "Set Work Mode"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["mode"],
            "properties": {
                "mode": { "type": "string", "enum": ["setup", "animate"] }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        edit.mode = match args.str("mode")?.trim().to_ascii_lowercase().as_str() {
            "setup" => WorkMode::Setup,
            "animate" => WorkMode::Animate,
            _ => {
                return Err(OpError::Args(crate::args::ArgError::WrongType {
                    key: "mode".into(),
                    wanted: "setup or animate",
                    got: "another name",
                }));
            }
        };
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(SetMode));
    ops.register(Box::new(RenameAttachment));
    ops.register(Box::new(DuplicateAttachment));
    ops.register(Box::new(RemoveAttachment));
    ops.register(Box::new(SetRegion));
    ops.register(Box::new(AddEvent));
    ops.register(Box::new(SetEvent));
    ops.register(Box::new(DeleteEvent));
}
