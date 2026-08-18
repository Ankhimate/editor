//! Building a rig with nothing but verb names and JSON.
//!
//! This is the shape a plugin, an MCP client or a script sees, exercised end to
//! end: no `AppState`, no `Session`, no egui — a `DocOps` registry, an `Edit`,
//! and arguments that name their targets. If this stops compiling, the headless
//! surface has grown a dependency on the editor and `docs/plugin-plan.md`'s
//! whole ordering is wrong.

use ankhimate_document::{Args, DocOps, Edit, OpError, WorkMode};
use serde_json::json;

fn run(ops: &DocOps, edit: &mut Edit, id: &str, args: serde_json::Value) -> Result<(), OpError> {
    ops.invoke(id, edit, &Args::from_json(args))
}

#[test]
fn a_script_builds_a_two_bone_rig_and_undoes_it() {
    let ops = DocOps::builtin();
    let mut edit = Edit::default();

    run(&ops, &mut edit, "bone.create", json!({ "name": "root" })).expect("root");
    run(
        &ops,
        &mut edit,
        "bone.create",
        json!({ "name": "spine", "parent": "root", "y": 40.0, "rotation": 90.0 }),
    )
    .expect("spine");
    run(
        &ops,
        &mut edit,
        "slot.create",
        json!({ "name": "body", "bone": "spine" }),
    )
    .expect("slot");
    run(
        &ops,
        &mut edit,
        "anim.create",
        json!({ "name": "walk", "duration": 2.0 }),
    )
    .expect("clip");

    assert_eq!(edit.doc.skeleton.bones.len(), 2);
    assert_eq!(edit.doc.skeleton.slots.len(), 1);
    assert_eq!(edit.doc.animations.len(), 1);

    let spine = edit
        .doc
        .skeleton
        .bones
        .values()
        .find(|b| b.name == "spine")
        .expect("spine exists");
    assert!(spine.parent.is_some(), "parented by name");
    assert_eq!(spine.local_transform.position.y, 40.0);
    assert!(
        (spine.local_transform.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "degrees in, radians stored"
    );

    // Every one of those was a command, so every one undoes.
    assert!(edit.undo(), "the clip");
    assert!(edit.undo(), "the slot");
    assert!(edit.undo(), "the spine");
    assert_eq!(edit.doc.skeleton.bones.len(), 1);
}

#[test]
fn an_edit_names_its_target_and_leaves_the_rest_alone() {
    // `bone.set_transform` reads absent fields as "keep", which is what lets a
    // caller nudge one axis. Zeroing the others would be a quiet way to flatten
    // a rig one call at a time.
    let ops = DocOps::builtin();
    let mut edit = Edit::default();

    run(
        &ops,
        &mut edit,
        "bone.create",
        json!({ "name": "arm", "x": 10.0, "y": 20.0, "rotation": 45.0 }),
    )
    .expect("arm");

    run(
        &ops,
        &mut edit,
        "bone.set_transform",
        json!({ "bone": "arm", "x": 99.0 }),
    )
    .expect("moved");

    let arm = edit
        .doc
        .skeleton
        .bones
        .values()
        .find(|b| b.name == "arm")
        .expect("arm exists");
    assert_eq!(arm.local_transform.position.x, 99.0, "x moved");
    assert_eq!(arm.local_transform.position.y, 20.0, "y kept");
    assert!(
        (arm.local_transform.rotation - 45.0_f32.to_radians()).abs() < 1e-5,
        "rotation kept"
    );
    assert_eq!(arm.local_transform.scale, glam::Vec2::ONE, "scale kept");
}

#[test]
fn a_name_the_rig_lacks_stops_the_edit_rather_than_half_doing_it() {
    let ops = DocOps::builtin();
    let mut edit = Edit::default();
    run(&ops, &mut edit, "bone.create", json!({ "name": "root" })).expect("root");

    let err = run(
        &ops,
        &mut edit,
        "bone.create",
        json!({ "name": "hand", "parent": "nope" }),
    )
    .expect_err("an unresolvable parent is an error");

    assert!(matches!(err, OpError::Args(_)), "{err}");
    assert_eq!(
        edit.doc.skeleton.bones.len(),
        1,
        "the document is untouched, not half-edited"
    );
}

#[test]
fn the_mode_rule_reaches_a_script_too() {
    // T-207 is a property of the command, so a caller with no UI is held to it
    // exactly as a panel is — and hears which mode was wanted rather than
    // nothing at all.
    let ops = DocOps::builtin();
    let mut edit = Edit::default();
    edit.mode = WorkMode::Animate;

    let err = run(&ops, &mut edit, "bone.create", json!({ "name": "root" }))
        .expect_err("structural edits are Setup-only");

    assert!(matches!(err, OpError::Refused(_)), "{err}");
    assert_eq!(edit.doc.skeleton.bones.len(), 0);
}

#[test]
fn an_unknown_verb_says_so() {
    let ops = DocOps::builtin();
    let mut edit = Edit::default();
    let err = run(&ops, &mut edit, "bone.explode", json!({})).expect_err("no such verb");
    assert_eq!(err, OpError::Unknown("bone.explode".into()));
}

#[test]
fn every_verb_describes_its_arguments() {
    // What an MCP client lists tools from. A verb taking arguments with no
    // schema is one nobody can call without reading its source.
    let ops = DocOps::builtin();
    for id in ops.ids() {
        let op = ops.get(id).expect("registered");
        let schema = op.schema();
        assert!(
            schema.is_object(),
            "{id} takes arguments but describes none"
        );
        assert!(
            schema["properties"].is_object(),
            "{id} has no properties in its schema"
        );
    }
}

#[test]
fn ids_are_dotted_and_unique() {
    let ops = DocOps::builtin();
    let ids: Vec<&str> = ops.ids().collect();
    assert!(!ids.is_empty());
    for id in &ids {
        assert!(id.contains('.'), "{id} is not domain.verb");
    }
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "an id is registered twice");
}
