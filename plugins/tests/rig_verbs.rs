//! Skins and slot editing from JavaScript.
//!
//! What a plugin could not reach before: skins at all, and everything about a
//! slot except creating one. An importer bringing a rig across with two outfits
//! had nowhere to put the second; one bringing draw order across could not set
//! it, so art arrived in whatever order the slots happened to be made — the one
//! thing a viewer notices immediately.

use ankhimate_plugins::Host;

fn run(script: &str) -> ankhimate_document::Edit {
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    host.run(script, &mut edit).expect("the script runs");
    edit
}

fn fails(script: &str) -> String {
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    host.run(script, &mut edit)
        .expect_err("this should have been refused")
        .to_string()
}

/// A root bone and three slots on it.
const RIG: &str = r#"
    ops.invoke("bone.create", { name: "root" });
    ops.invoke("slot.create", { name: "back", bone: "root" });
    ops.invoke("slot.create", { name: "body", bone: "root" });
    ops.invoke("slot.create", { name: "front", bone: "root" });
"#;

#[test]
fn a_script_can_add_a_skin_and_copy_art_into_it() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("skin.create", {{ name: "winter" }});
        ops.invoke("skin.create", {{ name: "summer", copy_from: "winter" }});
        "#
    ));

    let names: Vec<&str> = edit
        .doc
        .skeleton
        .skins
        .iter()
        .map(|(_, s)| s.name.as_str())
        .collect();
    assert!(names.contains(&"winter"), "{names:?}");
    assert!(names.contains(&"summer"), "{names:?}");
}

#[test]
fn copying_from_a_skin_that_is_not_there_leaves_no_half_built_skin() {
    // Resolved before anything is created. A verb that made the skin first and
    // then failed would leave a rig with an empty outfit nobody asked for, and
    // the script has no way to know it needs cleaning up.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let before = edit.doc.skeleton.skins.len();
    let _ = host.run(
        r#"ops.invoke("skin.create", { name: "summer", copy_from: "nope" });"#,
        &mut edit,
    );

    assert_eq!(
        edit.doc.skeleton.skins.len(),
        before,
        "nothing was created on the way to failing"
    );
}

#[test]
fn the_default_skin_cannot_be_deleted() {
    // Every rig needs one, and a script that removed it would leave a document
    // that cannot resolve any attachment at all.
    let message = fails(&format!(
        r#"{RIG}
        const base = names().skins[0];
        ops.invoke("skin.delete", {{ name: base }});
        "#
    ));
    assert!(
        message.contains("default"),
        "the reason says which skin and why: {message}"
    );
}

#[test]
fn draw_order_is_set_back_to_front() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_draw_order", {{ slots: ["front", "body", "back"] }});
        "#
    ));

    let order: Vec<&str> = edit
        .doc
        .skeleton
        .draw_order
        .iter()
        .filter_map(|id| edit.doc.skeleton.slots.get(*id))
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(order, ["front", "body", "back"]);
}

#[test]
fn a_partial_draw_order_is_refused_rather_than_completed() {
    // There is no honest rule for where the unnamed slots go: appending them
    // changes what draws on top and so does prepending. An importer that lost a
    // slot from its list should hear about it rather than get a rig whose
    // layering is subtly wrong.
    let message = fails(&format!(
        r#"{RIG}
        ops.invoke("slot.set_draw_order", {{ slots: ["front", "back"] }});
        "#
    ));
    assert!(
        message.contains("every slot"),
        "the reason says what was wanted: {message}"
    );
}

#[test]
fn naming_one_slot_twice_in_a_draw_order_is_refused() {
    // The other way a list can be wrong while being the right length.
    let message = fails(&format!(
        r#"{RIG}
        ops.invoke("slot.set_draw_order", {{ slots: ["front", "front", "back"] }});
        "#
    ));
    assert!(message.contains("once"), "{message}");
}

#[test]
fn a_three_component_colour_leaves_alpha_alone() {
    // `[1, 0, 0]` means "make it red", not "make it transparent". A verb that
    // defaulted the fourth component to zero would hide the slot instead.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_color", {{ slot: "body", color: [1, 0, 0, 0.5] }});
        ops.invoke("slot.set_color", {{ slot: "body", color: [0, 1, 0] }});
        "#
    ));

    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(_, s)| s)
        .expect("the slot");
    assert_eq!(slot.color[1], 1.0, "the new colour landed");
    assert_eq!(
        slot.color[3], 0.5,
        "and the alpha it did not mention survived"
    );
}

#[test]
fn a_blend_mode_is_named_and_an_unknown_one_is_refused() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_presentation", {{ slot: "body", blend_mode: "additive" }});
        "#
    ));
    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(_, s)| s)
        .expect("the slot");
    assert_eq!(slot.blend_mode, ankhimate_core::slot::BlendMode::Additive);

    let message = fails(&format!(
        r#"{RIG}
        ops.invoke("slot.set_presentation", {{ slot: "body", blend_mode: "overlay" }});
        "#
    ));
    assert!(
        message.contains("additive"),
        "the reason lists what is available: {message}"
    );
}

#[test]
fn a_dark_colour_can_be_set_and_then_cleared() {
    // Three states, not two: absent leaves it, null clears it, a list sets it.
    // A two-colour tint that could only be set would be a decision a script
    // could not take back.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_presentation", {{ slot: "body", dark_color: [0.2, 0, 0] }});
        ops.invoke("slot.set_presentation", {{ slot: "body", blend_mode: "screen" }});
        "#
    ));
    let dark = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .and_then(|(_, s)| s.dark_color);
    assert!(
        dark.is_some(),
        "an edit that did not mention the dark colour left it alone"
    );

    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_presentation", {{ slot: "body", dark_color: [0.2, 0, 0] }});
        ops.invoke("slot.set_presentation", {{ slot: "body", dark_color: null }});
        "#
    ));
    let dark = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .and_then(|(_, s)| s.dark_color);
    assert!(dark.is_none(), "null cleared it");
}

#[test]
fn a_slot_can_be_emptied_as_well_as_filled() {
    // "Show nothing" is a real answer rather than a missing argument: an empty
    // slot is how a rig hides a part in setup.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.set_attachment", {{ slot: "body", attachment: "torso" }});
        ops.invoke("slot.set_attachment", {{ slot: "body" }});
        "#
    ));

    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(_, s)| s)
        .expect("the slot");
    assert_eq!(slot.attachment, None);
}

#[test]
fn deleting_a_slot_takes_it_out_of_the_draw_order_too() {
    // A draw order naming a slot that is gone is a rig that cannot be drawn.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.delete", {{ name: "body" }});
        "#
    ));

    assert_eq!(edit.doc.skeleton.slots.len(), 2);
    assert_eq!(
        edit.doc.skeleton.draw_order.len(),
        2,
        "the draw order shrank with it"
    );
}
