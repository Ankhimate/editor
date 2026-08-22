//! What a plugin, a script or an MCP client can *ask* about a rig.
//!
//! The counterpart to `doc_ops`: verbs change the document, this reads it.
//!
//! # It is the template context, not a second vocabulary
//!
//! `docs/export-context.md` already documents a complete, versioned, JSON-shaped
//! view of a rig — named references, degrees for angles — and calls itself a
//! public contract for exactly the reason this needs one: a rename breaks
//! templates people have written, silently, with no compiler on that side.
//!
//! A plugin API with its own field names would be a second such contract to keep
//! in step with the first, and the two would drift the way the Edit menu drifted
//! from the keymap. So this *is* that context, reached without a template.
//! Someone who has written an exporter already knows this API.
//!
//! # What it is not
//!
//! Not a write surface. Nothing here hands out a `&mut Document`, because every
//! mutation is a command (`CLAUDE.md`) and a plugin that edited the tree
//! directly would bypass undo. Read here, change through `doc_ops`.
//!
//! Not the live pose, either. This is setup data plus timelines — what the rig
//! *is*, not where it happens to be posed. Sampling a pose needs
//! `core::pose::evaluate`, which is a different question with a different cost.

use crate::doc::Document;

/// A rig as JSON, in the shape `docs/export-context.md` documents.
///
/// `version` is [`ankhimate_export::context::CONTEXT_VERSION`]: additions are
/// free, renames are breaking, and a consumer can check before trusting a field.
pub fn describe(doc: &Document) -> serde_json::Value {
    let project = ankhimate_formats::convert::to_schema(&doc.as_project_ref());
    let ctx = ankhimate_export::context::Context::build(
        &project,
        None,
        ankhimate_export::context::ExportInfo::default(),
    );
    serde_json::to_value(&ctx).unwrap_or(serde_json::Value::Null)
}

/// The names a caller can use as arguments, by kind.
///
/// Cheaper than [`describe`] and the thing most callers actually want first: an
/// operator names its target, so "what is there to name?" is the question that
/// precedes every edit. Answering it without building the whole context matters
/// when the rig is large and the caller is deciding what to do next.
pub fn names(doc: &Document) -> Names {
    let mut images: Vec<String> = doc
        .assets
        .images
        .values()
        .map(|image| image.name.clone())
        .collect();
    images.sort();

    let attachments = doc
        .skeleton
        .slots
        .iter()
        .filter_map(|(slot_id, slot)| {
            let mut available: Vec<String> = doc
                .skeleton
                .skins
                .values()
                .flat_map(|skin| skin.names_for_slot(slot_id).map(str::to_owned))
                .collect();
            available.sort();
            available.dedup();
            if available.is_empty() && slot.attachment.is_none() {
                None
            } else {
                Some(SlotAttachments {
                    slot: slot.name.clone(),
                    current: slot.attachment.clone(),
                    available,
                })
            }
        })
        .collect();

    Names {
        bones: doc
            .skeleton
            .bones
            .values()
            .map(|b| b.name.clone())
            .collect(),
        slots: doc
            .skeleton
            .slots
            .values()
            .map(|s| s.name.clone())
            .collect(),
        skins: doc
            .skeleton
            .skins
            .values()
            .map(|s| s.name.clone())
            .collect(),
        animations: doc.animations.values().map(|a| a.name.clone()).collect(),
        images,
        attachments,
    }
}

/// What is in a rig, by name.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Names {
    pub bones: Vec<String>,
    pub slots: Vec<String>,
    pub skins: Vec<String>,
    pub animations: Vec<String>,
    pub images: Vec<String>,
    pub attachments: Vec<SlotAttachments>,
}

/// Attachment choices for one slot, without the geometry in the full read surface.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SlotAttachments {
    pub slot: String,
    pub current: Option<String>,
    pub available: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Args, DocOps, Edit};
    use ankhimate_core::{
        assets::ImageAsset,
        attachment::{Attachment, PointAttachment},
        math::Transform,
        skeleton::Bone,
        slot::Slot,
    };
    use serde_json::json;

    fn rig() -> Edit {
        let ops = DocOps::builtin();
        let mut edit = Edit::default();
        for args in [
            json!({ "name": "root" }),
            json!({ "name": "spine", "parent": "root", "y": 40.0 }),
        ] {
            ops.invoke("bone.create", &mut edit, &Args::from_json(args))
                .expect("built");
        }
        ops.invoke(
            "slot.create",
            &mut edit,
            &Args::from_json(json!({ "name": "body", "bone": "spine" })),
        )
        .expect("slot");
        ops.invoke(
            "anim.create",
            &mut edit,
            &Args::from_json(json!({ "name": "walk", "duration": 2.0 })),
        )
        .expect("clip");
        edit
    }

    #[test]
    fn names_answer_what_an_operator_can_be_given() {
        // The question that precedes every edit: a verb names its target, so a
        // caller needs to know what names exist before it can name one.
        let edit = rig();
        let names = names(&edit.doc);

        assert!(names.bones.contains(&"root".to_string()));
        assert!(names.bones.contains(&"spine".to_string()));
        assert_eq!(names.slots, ["body"]);
        assert_eq!(names.animations, ["walk"]);
    }

    #[test]
    fn names_include_compact_asset_and_attachment_choices() {
        let mut doc = Document::new();
        let bone = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 0.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let mut hand = Slot::new("hand".into(), bone);
        hand.attachment = Some("closed".into());
        let hand = doc.skeleton.add_slot(hand);
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(hand, "open", Attachment::Point(PointAttachment::default()));
        doc.skeleton.skins[skin].set(
            hand,
            "closed",
            Attachment::Point(PointAttachment::default()),
        );
        doc.assets
            .add(ImageAsset::new("hand.png", Vec::new(), 1, 1));

        let names = names(&doc);
        assert_eq!(names.images, ["hand.png"]);
        assert_eq!(
            names.attachments,
            [SlotAttachments {
                slot: "hand".into(),
                current: Some("closed".into()),
                available: vec!["closed".into(), "open".into()],
            }]
        );
    }

    #[test]
    fn a_described_rig_is_the_template_context() {
        // Not a second vocabulary: someone who has written an exporter already
        // knows these field names, and there is one contract to keep rather
        // than two that can drift.
        let edit = rig();
        let described = describe(&edit.doc);

        assert!(
            described["context_version"].is_number(),
            "carries the version a consumer checks"
        );
        assert_eq!(described["project"]["name"], "untitled");

        let bones = described["skeleton"]["bones"]
            .as_array()
            .expect("bones is a list");
        assert_eq!(bones.len(), 2);
        assert!(bones.iter().any(|b| b["name"] == "spine"));
    }

    #[test]
    fn angles_are_degrees_as_the_contract_says() {
        // `core` works in radians and the contract is degrees (PLAN §2.7). A
        // reader handing out radians would be wrong in a way that looks right
        // until someone's exported rig is off by a factor of 57.
        let ops = DocOps::builtin();
        let mut edit = Edit::default();
        ops.invoke(
            "bone.create",
            &mut edit,
            &Args::from_json(json!({ "name": "arm", "rotation": 90.0 })),
        )
        .expect("built");

        let described = describe(&edit.doc);
        let arm = described["skeleton"]["bones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == "arm")
            .expect("arm");
        assert!(
            (arm["rotation"].as_f64().unwrap() - 90.0).abs() < 1e-3,
            "degrees out, as given in"
        );
    }

    #[test]
    fn describing_an_empty_rig_does_not_fail() {
        // A caller asking about a document before anything is in it should get
        // an empty answer rather than an error — the first thing a script does
        // is look.
        let described = describe(&Document::new());
        assert!(
            described["skeleton"]["bones"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
