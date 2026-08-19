//! The verbs a real importer needs, exercised the way one would use them.
//!
//! `docs/export-plan.md` requires our own runtime format to be a template, so
//! that a format we cannot express is found before a user finds it. This is the
//! import side of that rule: a rig with artwork, a weighted mesh and animation,
//! built entirely by verb calls.

use ankhimate_document::{Args, DocOps, Edit};
use serde_json::json;

/// The smallest valid PNG — 1×1, transparent.
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk\
                       YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn run(ops: &DocOps, edit: &mut Edit, id: &str, args: serde_json::Value) {
    ops.invoke(id, edit, &Args::from_json(args))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// A rig with a bone, a slot and an image in the library.
fn base() -> (DocOps, Edit) {
    let ops = DocOps::builtin();
    let mut edit = Edit::default();
    run(&ops, &mut edit, "bone.create", json!({ "name": "root" }));
    run(
        &ops,
        &mut edit,
        "slot.create",
        json!({ "name": "body", "bone": "root" }),
    );
    run(
        &ops,
        &mut edit,
        "asset.add_image",
        json!({ "name": "torso", "bytes_base64": PNG_1X1 }),
    );
    (ops, edit)
}

#[test]
fn an_image_arrives_with_the_size_read_from_its_pixels() {
    // Taken from the file rather than on trust: an attachment sized from a lie
    // draws at the wrong scale, and the PNG already knows the answer.
    let (_, edit) = base();
    let asset = edit.doc.assets.images.values().next().expect("one image");
    assert_eq!(asset.name, "torso");
    assert_eq!((asset.width, asset.height), (1, 1));
    assert!(!asset.bytes.is_empty(), "the pixels came across");
}

#[test]
fn a_region_defaults_to_the_size_of_its_image() {
    // A region at zero size draws nothing, which is an afternoon of wondering
    // where the artwork went — so an importer that says nothing about extents
    // still gets something visible.
    use ankhimate_core::attachment::Attachment;

    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "attachment.create_region",
        json!({ "slot": "body", "name": "torso", "texture": "torso" }),
    );

    let (slot_id, slot) = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .expect("body");
    assert_eq!(
        slot.attachment.as_deref(),
        Some("torso"),
        "and it is what the slot shows"
    );

    let skin = edit.doc.skeleton.default_skin;
    match edit.doc.skeleton.skins[skin].get(slot_id, "torso") {
        Some(Attachment::Region(r)) => {
            assert_eq!((r.width, r.height), (1.0, 1.0), "from the image");
        }
        other => panic!("expected a region, got {other:?}"),
    }
}

#[test]
fn a_weighted_mesh_names_its_bones_rather_than_indexing_them() {
    // Every rig format on disk addresses a mesh's bones by index into its own
    // array. A plugin cannot hold ids, so the verb takes names — which is also
    // what makes a mesh survive a bone being deleted and undone.
    use ankhimate_core::attachment::Attachment;

    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "bone.create",
        json!({ "name": "spine", "parent": "root", "y": 20.0 }),
    );
    run(
        &ops,
        &mut edit,
        "attachment.create_mesh",
        json!({
            "slot": "body", "name": "cloth", "texture": "torso",
            "vertices": [0.0, 0.0, 10.0, 0.0, 10.0, 10.0],
            "uvs": [0.0, 0.0, 1.0, 0.0, 1.0, 1.0],
            "triangles": [0, 1, 2],
            "weights": [
                [{ "bone": "root", "weight": 1.0 }],
                [{ "bone": "root", "weight": 0.5 }, { "bone": "spine", "weight": 0.5 }],
                [{ "bone": "spine", "weight": 1.0 }]
            ]
        }),
    );

    let slot_id = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(id, _)| id)
        .unwrap();
    let skin = edit.doc.skeleton.default_skin;
    match edit.doc.skeleton.skins[skin].get(slot_id, "cloth") {
        Some(Attachment::Mesh(m)) => {
            assert_eq!(m.setup_vertices.len(), 3);
            assert_eq!(m.triangles, [[0, 1, 2]]);
            assert_eq!(m.weights.len(), 3);
            assert_eq!(m.weights[1].len(), 2, "the shared vertex has two bones");
        }
        other => panic!("expected a mesh, got {other:?}"),
    }
}

#[test]
fn a_mesh_whose_uvs_do_not_match_its_vertices_is_refused() {
    // Caught at the call rather than left to draw wrongly: the count mismatch
    // is a bug in the importer, and naming it here names the attachment.
    let (ops, mut edit) = base();
    let err = ops
        .invoke(
            "attachment.create_mesh",
            &mut edit,
            &Args::from_json(json!({
                "slot": "body", "name": "bad", "texture": "torso",
                "vertices": [0.0, 0.0, 10.0, 0.0],
                "uvs": [0.0, 0.0],
                "triangles": [0, 1, 0]
            })),
        )
        .expect_err("a uv per vertex is required");
    assert!(format!("{err}").contains("uvs"), "{err}");
}

#[test]
fn a_bone_channel_is_keyed_per_axis() {
    // An address names one track, and translate is two. Defaulting a missing
    // axis would put a y key on the x track, which is the kind of wrong that
    // plays back plausibly.
    use ankhimate_core::animation::{Axis, Timeline};

    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "anim.create",
        json!({ "name": "walk", "duration": 1.0 }),
    );
    run(
        &ops,
        &mut edit,
        "anim.key_bone",
        json!({
            "animation": "walk", "bone": "root", "property": "translate",
            "axis": "y", "time": 0.5, "value": 30.0
        }),
    );
    run(
        &ops,
        &mut edit,
        "anim.key_bone",
        json!({
            "animation": "walk", "bone": "root", "property": "rotate",
            "time": 0.5, "value": 90.0
        }),
    );

    let clip = edit.doc.animations.values().next().expect("walk");
    let y_track = clip
        .timelines
        .iter()
        .find_map(|t| match t {
            Timeline::BoneTranslate {
                axis: Axis::Y,
                keys,
                ..
            } => Some(keys),
            _ => None,
        })
        .expect("a Y translate track");
    assert_eq!(y_track[0].value, 30.0);

    assert!(
        clip.timelines
            .iter()
            .any(|t| matches!(t, Timeline::BoneRotate { .. })),
        "rotate has one track and needs no axis"
    );
}

#[test]
fn keying_a_two_axis_property_without_an_axis_is_refused() {
    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "anim.create",
        json!({ "name": "walk", "duration": 1.0 }),
    );
    let err = ops
        .invoke(
            "anim.key_bone",
            &mut edit,
            &Args::from_json(json!({
                "animation": "walk", "bone": "root", "property": "scale",
                "time": 0.0, "value": 2.0
            })),
        )
        .expect_err("scale is two tracks");
    assert!(format!("{err}").contains("axis"), "{err}");
}

#[test]
fn an_attachment_key_can_hide_a_slot() {
    // Absent means hidden, which is a real value rather than a missing one —
    // it is how an effect is switched off partway through a clip.
    use ankhimate_core::animation::Timeline;

    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "attachment.create_region",
        json!({ "slot": "body", "name": "torso", "texture": "torso" }),
    );
    run(
        &ops,
        &mut edit,
        "anim.create",
        json!({ "name": "blink", "duration": 1.0 }),
    );
    run(
        &ops,
        &mut edit,
        "anim.key_attachment",
        json!({ "animation": "blink", "slot": "body", "time": 0.0, "attachment": "torso" }),
    );
    run(
        &ops,
        &mut edit,
        "anim.key_attachment",
        json!({ "animation": "blink", "slot": "body", "time": 0.5 }),
    );

    let clip = edit.doc.animations.values().next().expect("blink");
    let keys = clip
        .timelines
        .iter()
        .find_map(|t| match t {
            Timeline::SlotAttachment { keys, .. } => Some(keys),
            _ => None,
        })
        .expect("an attachment track");
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].value.as_deref(), Some("torso"));
    assert_eq!(keys[1].value, None, "omitted means hidden");
}

#[test]
fn an_importer_can_say_what_it_could_not_carry() {
    // The honesty property the Rust readers have. An import that drops half a
    // file quietly is worse than one that refuses.
    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "import.report",
        json!({
            "what": "curve",
            "where": "walk/hip/translate",
            "detail": "a time handle outside the segment was clamped"
        }),
    );

    assert_eq!(edit.report.len(), 1);
    assert_eq!(edit.report[0].what, "curve");
    assert_eq!(edit.report[0].where_, "walk/hip/translate");
}

#[test]
fn a_report_is_not_undone_with_the_rig() {
    // A report is not part of the rig. Undoing an import's last bone should not
    // un-say what that import could not do.
    let (ops, mut edit) = base();
    run(
        &ops,
        &mut edit,
        "import.report",
        json!({ "what": "mesh", "where": "cloth", "detail": "imported rigid" }),
    );
    run(&ops, &mut edit, "bone.create", json!({ "name": "extra" }));

    edit.undo();
    assert_eq!(edit.report.len(), 1, "the report survived the undo");
}

#[test]
fn every_import_verb_describes_its_arguments() {
    // What a plugin author reads instead of the source, and what an MCP client
    // lists tools from.
    let ops = DocOps::builtin();
    for id in [
        "asset.add_image",
        "attachment.create_region",
        "attachment.create_mesh",
        "anim.key_bone",
        "anim.key_attachment",
        "import.report",
    ] {
        let op = ops.get(id).unwrap_or_else(|| panic!("{id} is registered"));
        let schema = op.schema();
        assert!(schema["properties"].is_object(), "{id} describes nothing");
        assert!(schema["required"].is_array(), "{id} names no required args");
    }
}
