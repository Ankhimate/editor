//! Attachment and event editing from JavaScript.
//!
//! Attachments could be created and never touched again — an importer that
//! placed art wrongly had to be right the first time. Events could not be
//! reached at all, and the commands behind them address an event by its index
//! in a list, which a script has no business counting.

use ankhimate_plugins::Host;

/// A 1×1 transparent PNG, so `asset.add_image` has real bytes to decode.
const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk\
                       YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

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

/// A bone, a slot, an image and a region attachment on it.
fn rig() -> String {
    format!(
        r#"
        ops.invoke("bone.create", {{ name: "root" }});
        ops.invoke("slot.create", {{ name: "body", bone: "root" }});
        ops.invoke("asset.add_image", {{ name: "torso", bytes_base64: "{PNG_1X1}" }});
        ops.invoke("attachment.create_region",
                   {{ slot: "body", name: "torso", texture: "torso",
                      width: 40, height: 60 }});
        "#
    )
}

fn region_of<'a>(
    edit: &'a ankhimate_document::Edit,
    name: &str,
) -> &'a ankhimate_core::attachment::RegionAttachment {
    use ankhimate_core::attachment::Attachment;
    let skin = edit.doc.skeleton.default_skin;
    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(id, _)| id)
        .expect("the slot");
    match edit.doc.skeleton.skins[skin]
        .get(slot, name)
        .expect("the attachment")
    {
        Attachment::Region(region) => region,
        _ => panic!("not a region"),
    }
}

#[test]
fn an_attachment_can_be_renamed_after_it_is_made() {
    let edit = run(&format!(
        r#"{}
        ops.invoke("attachment.rename",
                   {{ slot: "body", attachment: "torso", to: "chest" }});
        "#,
        rig()
    ));

    let region = region_of(&edit, "chest");
    assert_eq!(
        region.texture, "torso",
        "the attachment's name changed; the image it draws did not"
    );
}

#[test]
fn a_region_transform_can_be_set_after_the_fact() {
    // The gap this closes: an importer that placed art wrongly had to be right
    // the first time.
    let edit = run(&format!(
        r#"{}
        ops.invoke("attachment.set_region",
                   {{ slot: "body", attachment: "torso",
                      x: 5, y: -3, rotation: 90, scale_x: 2 }});
        "#,
        rig()
    ));

    let region = region_of(&edit, "torso");
    assert_eq!((region.local_offset.x, region.local_offset.y), (5.0, -3.0));
    assert!(
        (region.local_rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "90 degrees became {} radians",
        region.local_rotation
    );
    assert_eq!(region.local_scale.x, 2.0);
    assert_eq!(region.local_scale.y, 1.0, "the axis it did not mention");
}

#[test]
fn moving_the_pivot_leaves_the_art_where_it_was() {
    // An importer setting a shoulder pivot means "turn about here", not "and
    // also jump half a sprite to the left". The offset compensates, which is
    // what the command's own helper is for — assigning the field directly is
    // the bug this pins.
    let edit = run(&format!(
        r#"{}
        ops.invoke("attachment.set_region",
                   {{ slot: "body", attachment: "torso", pivot_x: 0, pivot_y: 0 }});
        "#,
        rig()
    ));

    let region = region_of(&edit, "torso");
    assert_eq!(
        (region.pivot.x, region.pivot.y),
        (0.0, 0.0),
        "the pivot moved"
    );

    // The quad's corners are where they were: a 40×60 sprite centred on the
    // bone still spans -20..20 by -30..30 whatever its pivot is.
    let corners = region.local_corners();
    let min_x = corners.iter().map(|c| c.x).fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|c| c.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (min_x + 20.0).abs() < 1e-3 && (max_y - 30.0).abs() < 1e-3,
        "the art moved with the pivot: corners span from {min_x} and up to {max_y}"
    );
}

#[test]
fn an_attachment_of_the_wrong_kind_is_refused_by_name() {
    let message = fails(&format!(
        r#"{}
        ops.invoke("attachment.set_region",
                   {{ slot: "body", attachment: "nope", x: 1 }});
        "#,
        rig()
    ));
    assert!(
        message.contains("nope"),
        "the reason names what could not be found: {message}"
    );
}

#[test]
fn an_event_is_named_rather_than_counted() {
    // The commands behind events address one by its index in a list. A script
    // has no business counting, and a count is wrong the moment anything else
    // inserts an event — so the verb takes `(name, time)` and finds the index.
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 0.5 });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 1.0 });
        ops.invoke("anim.set_event",
                   { animation: "walk", name: "step", time: 1.0, string_value: "right" });
        "#);

    let anim = edit.doc.animations.iter().next().map(|(_, a)| a).unwrap();
    assert_eq!(anim.events.len(), 2);
    let late = anim
        .events
        .iter()
        .find(|e| (e.time - 1.0).abs() < 1e-4)
        .expect("the second step");
    assert_eq!(late.string_value, "right");

    let early = anim
        .events
        .iter()
        .find(|e| (e.time - 0.5).abs() < 1e-4)
        .expect("the first step");
    assert_eq!(
        early.string_value, "",
        "the event with the same name at another time was left alone"
    );
}

#[test]
fn an_event_that_is_not_there_is_reported_with_its_time() {
    // "No event called step" would be wrong: there is one, at another time.
    let message = fails(
        r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 0.5 });
        ops.invoke("anim.set_event", { animation: "walk", name: "step", time: 9 });
        "#,
    );
    assert!(
        message.contains('9'),
        "the reason says which moment was looked for: {message}"
    );
}

#[test]
fn an_event_can_be_moved_and_renamed_in_one_call() {
    // Three commands behind one verb. The move goes last, because moving an
    // event changes the time this verb found it by.
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 0.5 });
        ops.invoke("anim.set_event",
                   { animation: "walk", name: "step", time: 0.5,
                     to_name: "footfall", to_time: 0.75, float_value: 2 });
        "#);

    let event = edit
        .doc
        .animations
        .iter()
        .next()
        .map(|(_, a)| &a.events[0])
        .expect("the event");
    assert_eq!(event.name, "footfall");
    assert!((event.time - 0.75).abs() < 1e-4, "moved to {}", event.time);
    assert_eq!(event.float_value, 2.0, "and the payload landed too");
}

#[test]
fn deleting_an_event_leaves_the_others() {
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "a", time: 0.1 });
        ops.invoke("anim.add_event", { animation: "walk", name: "b", time: 0.2 });
        ops.invoke("anim.delete_event", { animation: "walk", name: "a", time: 0.1 });
        "#);

    let anim = edit.doc.animations.iter().next().map(|(_, a)| a).unwrap();
    assert_eq!(anim.events.len(), 1);
    assert_eq!(anim.events[0].name, "b");
}

#[test]
fn a_payload_field_left_out_keeps_its_value() {
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "hit", time: 0.5 });
        ops.invoke("anim.set_event",
                   { animation: "walk", name: "hit", time: 0.5, int_value: 7, audio: "thud" });
        ops.invoke("anim.set_event",
                   { animation: "walk", name: "hit", time: 0.5, volume: 0.5 });
        "#);

    let event = edit
        .doc
        .animations
        .iter()
        .next()
        .map(|(_, a)| &a.events[0])
        .expect("the event");
    assert_eq!(event.volume, 0.5, "the second edit landed");
    assert_eq!(event.int_value, 7, "and the first one survived it");
    assert_eq!(event.audio, "thud");
}

#[test]
fn a_script_can_reach_animate_mode() {
    // Found by writing the event tests: every verb declares which mode it needs
    // (T-207) and nothing could put a script in the other one, so the event
    // verbs — which write keys, and are therefore Animate-only — were
    // unreachable from JavaScript entirely.
    let message = fails(
        r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 0.5 });
        "#,
    );
    assert!(
        message.contains("Animate"),
        "without the mode verb this is what a script hits: {message}"
    );

    // And with it, the same script works.
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("doc.set_mode", { mode: "animate" });
        ops.invoke("anim.add_event", { animation: "walk", name: "step", time: 0.5 });
        "#);
    assert_eq!(edit.doc.animations.iter().next().unwrap().1.events.len(), 1);
}
