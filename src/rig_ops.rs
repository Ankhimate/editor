//! Skin and slot verbs.
//!
//! What a plugin could not reach before: skins at all, and everything about a
//! slot except creating one. An importer bringing a rig across with two outfits
//! had nowhere to put the second, and one bringing draw order across could not
//! set it — so the art arrived in whatever order the slots happened to be made,
//! which is the one thing a viewer notices immediately.
//!
//! Colours are `[r, g, b, a]` in 0..1, matching `core` rather than the 0..255 a
//! file format might use: the conversion belongs at the format boundary, and
//! two conventions in one API is how a tint ends up 255 times too bright.

use crate::args::{Args, Resolver};
use crate::commands::{skin_cmds, slot_cmds};
use crate::edit::Edit;
use crate::ops::{DocOperator, DocOps, OpError};
use crate::work_mode::WorkMode;
use serde_json::json;

/// Read a colour argument, falling back to what is already there.
fn color_or(args: &Args, key: &str, current: [f32; 4]) -> Result<[f32; 4], OpError> {
    let Some(value) = args.as_json().get(key) else {
        return Ok(current);
    };
    if value.is_null() {
        return Ok(current);
    }
    let list = args.f32_list(key)?;
    // Three components means "leave alpha alone", which is what a caller
    // writing `[1, 0, 0]` means — not "make it transparent".
    match list.len() {
        3 => Ok([list[0], list[1], list[2], current[3]]),
        4 => Ok([list[0], list[1], list[2], list[3]]),
        _ => Err(OpError::Args(crate::args::ArgError::WrongType {
            key: key.into(),
            wanted: "three or four numbers, 0..1",
            got: "a list of another length",
        })),
    }
}

/// Add a skin, optionally copying another's artwork into it.
pub struct CreateSkin;

impl DocOperator for CreateSkin {
    fn id(&self) -> &'static str {
        "skin.create"
    }

    fn label(&self) -> &str {
        "Create Skin"
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
                "copy_from": {
                    "type": "string",
                    "description": "Start from this skin's attachments rather than empty"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let name = args.str("name")?.to_string();
        // Resolved before the skin is made, so a bad `copy_from` leaves no
        // half-built skin behind.
        let copy_from = match args.opt_str("copy_from")? {
            None => None,
            Some(_) => Some(resolver.skin(args, "copy_from")?),
        };

        edit.dispatch(Box::new(skin_cmds::AddSkin::new(name.clone())))?;
        if let Some(from) = copy_from {
            let to = edit
                .doc
                .skeleton
                .skins
                .iter()
                .find(|(_, s)| s.name == name)
                .map(|(id, _)| id);
            if let Some(to) = to {
                edit.dispatch(Box::new(skin_cmds::CopyAttachments::new(from, to)))?;
            }
        }
        Ok(())
    }
}

/// Rename a skin.
pub struct RenameSkin;

impl DocOperator for RenameSkin {
    fn id(&self) -> &'static str {
        "skin.rename"
    }

    fn label(&self) -> &str {
        "Rename Skin"
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
        let resolver = Resolver::new(&edit.doc);
        let id = resolver.skin(args, "name")?;
        let to = args.str("to")?.to_string();
        edit.dispatch(Box::new(skin_cmds::RenameSkin::new(id, to)))?;
        Ok(())
    }
}

/// Delete a skin and the artwork routed into it.
pub struct DeleteSkin;

impl DocOperator for DeleteSkin {
    fn id(&self) -> &'static str {
        "skin.delete"
    }

    fn label(&self) -> &str {
        "Delete Skin"
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
        let resolver = Resolver::new(&edit.doc);
        let id = resolver.skin(args, "name")?;
        if id == edit.doc.skeleton.default_skin {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "name".into(),
                wanted: "a skin other than the default",
                got: "the default skin, which every rig needs",
            }));
        }
        edit.dispatch(Box::new(skin_cmds::RemoveSkin::new(id)))?;
        Ok(())
    }
}

/// Copy one skin's artwork into another.
pub struct CopySkin;

impl DocOperator for CopySkin {
    fn id(&self) -> &'static str {
        "skin.copy_attachments"
    }

    fn label(&self) -> &str {
        "Copy Skin Attachments"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["from", "to"],
            "properties": {
                "from": { "type": "string" },
                "to": { "type": "string" }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let from = resolver.skin(args, "from")?;
        let to = resolver.skin(args, "to")?;
        edit.dispatch(Box::new(skin_cmds::CopyAttachments::new(from, to)))?;
        Ok(())
    }
}

/// Delete a slot, its artwork in every skin, and its place in the draw order.
pub struct DeleteSlot;

impl DocOperator for DeleteSlot {
    fn id(&self) -> &'static str {
        "slot.delete"
    }

    fn label(&self) -> &str {
        "Delete Slot"
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
        let resolver = Resolver::new(&edit.doc);
        let slot = resolver.slot(args, "name")?;
        edit.dispatch(Box::new(slot_cmds::DeleteSlot::new(slot)))?;
        Ok(())
    }
}

/// Choose which attachment a slot shows in setup.
pub struct SetSlotAttachment;

impl DocOperator for SetSlotAttachment {
    fn id(&self) -> &'static str {
        "slot.set_attachment"
    }

    fn label(&self) -> &str {
        "Set Slot Attachment"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot"],
            "properties": {
                "slot": { "type": "string" },
                "attachment": {
                    "type": "string",
                    "description": "Omit or pass null to show nothing"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let slot = resolver.slot(args, "slot")?;
        // "Show nothing" is a real answer, not a missing argument — an empty
        // slot is how a rig hides a part in setup.
        let attachment = args.opt_str("attachment")?.map(str::to_string);
        edit.dispatch(Box::new(slot_cmds::SetSlotAttachment::new(
            slot, attachment,
        )))?;
        Ok(())
    }
}

/// A slot's tint.
pub struct SetSlotColor;

impl DocOperator for SetSlotColor {
    fn id(&self) -> &'static str {
        "slot.set_color"
    }

    fn label(&self) -> &str {
        "Set Slot Color"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot", "color"],
            "properties": {
                "slot": { "type": "string" },
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
        let slot = resolver.slot(args, "slot")?;
        let current = edit
            .doc
            .skeleton
            .slots
            .get(slot)
            .map(|s| s.color)
            .unwrap_or([1.0; 4]);
        let color = color_or(args, "color", current)?;
        edit.dispatch(Box::new(slot_cmds::SetSlotColor::new(slot, color)))?;
        Ok(())
    }
}

/// How a slot composites: blend mode and the dark half of a two-colour tint.
pub struct SetSlotPresentation;

impl DocOperator for SetSlotPresentation {
    fn id(&self) -> &'static str {
        "slot.set_presentation"
    }

    fn label(&self) -> &str {
        "Set Slot Presentation"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slot"],
            "properties": {
                "slot": { "type": "string" },
                "blend_mode": {
                    "type": "string",
                    "enum": ["normal", "additive", "multiply", "screen"]
                },
                "dark_color": {
                    "type": "array",
                    "items": { "type": "number" },
                    "description": "[r, g, b] or [r, g, b, a]; null clears it"
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        use ankhimate_core::slot::BlendMode;
        let resolver = Resolver::new(&edit.doc);
        let slot = resolver.slot(args, "slot")?;
        let current = edit.doc.skeleton.slots.get(slot).ok_or_else(|| {
            OpError::Args(crate::args::ArgError::Unresolved {
                key: "slot".into(),
                kind: "slot",
                name: args.str("slot").unwrap_or_default().to_string(),
            })
        })?;
        let (current_blend, current_dark) = (current.blend_mode, current.dark_color);

        let blend_mode = match args.opt_str("blend_mode")? {
            None => current_blend,
            Some(name) => match name.trim().to_ascii_lowercase().as_str() {
                "normal" => BlendMode::Normal,
                "additive" | "add" => BlendMode::Additive,
                "multiply" => BlendMode::Multiply,
                "screen" => BlendMode::Screen,
                _ => {
                    return Err(OpError::Args(crate::args::ArgError::WrongType {
                        key: "blend_mode".into(),
                        wanted: "normal, additive, multiply or screen",
                        got: "another name",
                    }));
                }
            },
        };

        // Three states, not two: absent leaves it, null clears it, a list sets
        // it. A two-colour tint that could only be set and never removed would
        // be a decision a script could not take back.
        let dark_color = match args.as_json().get("dark_color") {
            None => current_dark,
            Some(serde_json::Value::Null) => None,
            Some(_) => Some(color_or(
                args,
                "dark_color",
                current_dark.unwrap_or([0.0, 0.0, 0.0, 1.0]),
            )?),
        };

        edit.dispatch(Box::new(slot_cmds::SetSlotPresentation::new(
            slot,
            slot_cmds::SlotPresentation {
                blend_mode,
                dark_color,
            },
        )))?;
        Ok(())
    }
}

/// The order slots draw in, back to front.
pub struct SetDrawOrder;

impl DocOperator for SetDrawOrder {
    fn id(&self) -> &'static str {
        "slot.set_draw_order"
    }

    fn label(&self) -> &str {
        "Set Draw Order"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["slots"],
            "properties": {
                "slots": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Every slot, back to front. Naming only some is refused."
                }
            }
        })
    }

    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let resolver = Resolver::new(&edit.doc);
        let order = resolver.slot_list(args, "slots")?;

        // A partial order is refused rather than completed. There is no honest
        // rule for where the unnamed slots go — appending them changes what
        // draws on top, and so does prepending — and an importer that lost a
        // slot from its list should hear about it rather than get a rig whose
        // layering is subtly wrong.
        let total = edit.doc.skeleton.slots.len();
        if order.len() != total {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "slots".into(),
                wanted: "every slot in the rig, back to front",
                got: "a partial list",
            }));
        }
        let mut seen = order.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != order.len() {
            return Err(OpError::Args(crate::args::ArgError::WrongType {
                key: "slots".into(),
                wanted: "each slot once",
                got: "a list naming one twice",
            }));
        }

        edit.dispatch(Box::new(slot_cmds::SetDrawOrder::new(order)))?;
        Ok(())
    }
}

pub fn register(ops: &mut DocOps) {
    ops.register(Box::new(CreateSkin));
    ops.register(Box::new(RenameSkin));
    ops.register(Box::new(DeleteSkin));
    ops.register(Box::new(CopySkin));
    ops.register(Box::new(DeleteSlot));
    ops.register(Box::new(SetSlotAttachment));
    ops.register(Box::new(SetSlotColor));
    ops.register(Box::new(SetSlotPresentation));
    ops.register(Box::new(SetDrawOrder));
}
