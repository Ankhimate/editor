//! Hitboxes, clips, points, animation management and the rest of the bones.
//!
//! The last batch a script can meaningfully drive. What is left out is left out
//! on purpose — painting weights is a brush stroke and editing a mesh is a
//! vertex drag — so this is also the test that says the surface is finished
//! rather than merely large.

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

const RIG: &str = r#"
    ops.invoke("bone.create", { name: "root", length: 40 });
    ops.invoke("bone.create", { name: "arm", parent: "root", x: 40, length: 30 });
    ops.invoke("slot.create", { name: "body", bone: "root" });
"#;

fn attachment_of<'a>(
    edit: &'a ankhimate_document::Edit,
    name: &str,
) -> &'a ankhimate_core::attachment::Attachment {
    let skin = edit.doc.skeleton.default_skin;
    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(id, _)| id)
        .expect("the slot");
    edit.doc.skeleton.skins[skin]
        .get(slot, name)
        .expect("the attachment")
}

#[test]
fn a_hitbox_takes_the_polygon_it_is_given() {
    // The commands underneath take per-index deltas, because that is what a
    // drag produces. A script has a shape in mind and states it.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("attachment.create_box", {{
            slot: "body", name: "hurt",
            points: [[-10, -10], [10, -10], [10, 10], [-10, 10], [0, 20]]
        }});
        "#
    ));

    let ankhimate_core::attachment::Attachment::BoundingBox(bb) = attachment_of(&edit, "hurt")
    else {
        panic!("not a bounding box");
    };
    assert_eq!(bb.vertices.len(), 5, "five points in, five out");
    assert_eq!((bb.vertices[4].x, bb.vertices[4].y), (0.0, 20.0));
}

#[test]
fn a_polygon_smaller_than_what_is_there_loses_its_extra_vertices() {
    // Two removals rather than one, so the count is exercised rather than a
    // single off-by-one.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("attachment.create_box", {{
            slot: "body", name: "tri",
            points: [[0, 0], [10, 0], [10, 10], [5, 15], [0, 10]]
        }});
        ops.invoke("attachment.create_box", {{
            slot: "body", name: "tri2",
            points: [[0, 0], [10, 0], [5, 10]]
        }});
        "#
    ));

    let ankhimate_core::attachment::Attachment::BoundingBox(bb) = attachment_of(&edit, "tri2")
    else {
        panic!("not a bounding box");
    };
    assert_eq!(bb.vertices.len(), 3);
    assert_eq!((bb.vertices[2].x, bb.vertices[2].y), (5.0, 10.0));
}

#[test]
fn shrinking_a_polygon_keeps_the_points_that_were_asked_for() {
    // A five-point shape reshaped to three must end up with the three points
    // given, not three of the old ones moved.
    //
    // This was written to pin removal *order* and does not: `RemoveVertices`
    // sorts descending itself, so listing the indices either way gives the same
    // answer. Kept for what it does check — that shrinking works at all — and
    // labelled, rather than left looking stronger than it is.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("attachment.create_box", {{
            slot: "body", name: "five",
            points: [[0, 0], [10, 0], [10, 10], [5, 15], [0, 10]]
        }});
        ops.invoke("attachment.create_box", {{
            slot: "body", name: "five",
            points: [[1, 1], [2, 2], [3, 3]]
        }});
        "#
    ));

    let ankhimate_core::attachment::Attachment::BoundingBox(bb) = attachment_of(&edit, "five")
    else {
        panic!("not a bounding box");
    };
    assert_eq!(bb.vertices.len(), 3, "five points became three");
    let got: Vec<(f32, f32)> = bb.vertices.iter().map(|v| (v.x, v.y)).collect();
    assert_eq!(got, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
}

#[test]
fn a_two_point_polygon_is_refused() {
    // Two points is a line, which would import as a shape that hits nothing —
    // a rig that looks finished and does not work.
    let message = fails(&format!(
        r#"{RIG}
        ops.invoke("attachment.create_box",
                   {{ slot: "body", name: "line", points: [[0, 0], [10, 0]] }});
        "#
    ));
    assert!(message.contains("three"), "{message}");
}

#[test]
fn a_bad_polygon_creates_nothing_at_all() {
    // Read before anything is made: a shape half-built by a bad argument is
    // worse than none, because the script cannot tell it happened.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let _ = host.run(
        &format!(
            r#"{RIG}
            ops.invoke("attachment.create_box",
                       {{ slot: "body", name: "bad", points: [[0, 0], [1, "x"], [2, 2]] }});
            "#
        ),
        &mut edit,
    );

    let skin = edit.doc.skeleton.default_skin;
    let slot = edit
        .doc
        .skeleton
        .slots
        .iter()
        .find(|(_, s)| s.name == "body")
        .map(|(id, _)| id)
        .expect("the slot");
    assert!(
        edit.doc.skeleton.skins[skin].get(slot, "bad").is_none(),
        "nothing was created on the way to failing"
    );
}

#[test]
fn a_point_reads_its_rotation_in_degrees() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("attachment.create_point",
                   {{ slot: "body", name: "muzzle", x: 12, y: 3, rotation: 90 }});
        "#
    ));

    let ankhimate_core::attachment::Attachment::Point(point) = attachment_of(&edit, "muzzle")
    else {
        panic!("not a point");
    };
    assert_eq!((point.position.x, point.position.y), (12.0, 3.0));
    assert!(
        (point.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "90 degrees became {} radians",
        point.rotation
    );
}

#[test]
fn a_clip_can_name_the_slot_it_stops_after() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("slot.create", {{ name: "hair", bone: "root" }});
        ops.invoke("attachment.create_clip",
                   {{ slot: "body", name: "mask", end_slot: "hair" }});
        "#
    ));

    let ankhimate_core::attachment::Attachment::Clipping(clip) = attachment_of(&edit, "mask")
    else {
        panic!("not a clip");
    };
    assert_eq!(clip.end_slot.as_deref(), Some("hair"));
}

#[test]
fn reparenting_a_bone_does_not_move_it() {
    // T-206: a bone whose parent changed has not moved. A verb that let it jump
    // would make reparenting an edit nobody could use without fixing the pose
    // afterwards.
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("bone.create", {{ name: "torso", x: 10, y: 5 }});
        ops.invoke("bone.set_parent", {{ name: "arm", parent: "torso" }});
        "#
    ));

    let mut pose = ankhimate_core::pose::Pose::new();
    ankhimate_core::pose::evaluate(&edit.doc.skeleton, &[], &mut pose);
    let arm = edit
        .doc
        .skeleton
        .bones
        .iter()
        .find(|(_, b)| b.name == "arm")
        .map(|(id, _)| id)
        .expect("the bone");
    let world = pose.worlds[arm].transform_point(Default::default());

    assert!(
        (world.x - 40.0).abs() < 1e-3 && world.y.abs() < 1e-3,
        "the arm was at (40, 0) and is now at ({}, {})",
        world.x,
        world.y
    );
}

#[test]
fn an_animation_can_be_renamed_duplicated_and_deleted() {
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("anim.rename", { animation: "walk", to: "stride" });
        ops.invoke("anim.duplicate", { animation: "stride" });
        ops.invoke("anim.create", { name: "spare" });
        ops.invoke("anim.delete", { animation: "spare" });
        "#);

    let names: Vec<&str> = edit
        .doc
        .animations
        .iter()
        .map(|(_, a)| a.name.as_str())
        .collect();
    assert!(names.contains(&"stride"), "{names:?}");
    assert_eq!(
        names.len(),
        2,
        "the rename, the copy, and no spare: {names:?}"
    );
    assert!(!names.contains(&"walk"));
    assert!(!names.contains(&"spare"));
}

#[test]
fn retiming_scales_the_duration() {
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("anim.set_meta", { animation: "walk", duration: 2, looping: true });
        ops.invoke("anim.retime", { animation: "walk", scale: 0.5 });
        "#);

    let anim = edit.doc.animations.iter().next().map(|(_, a)| a).unwrap();
    assert!((anim.duration - 1.0).abs() < 1e-4, "{}", anim.duration);
    assert!(anim.looping, "retiming did not un-loop it");
}

#[test]
fn a_zero_retime_is_refused() {
    // It would collapse every key onto one moment, which is not a slower
    // animation — it is a destroyed one.
    let message = fails(
        r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("anim.retime", { animation: "walk", scale: 0 });
        "#,
    );
    assert!(message.contains("positive"), "{message}");
}

#[test]
fn setting_only_the_duration_leaves_looping_alone() {
    let edit = run(r#"
        ops.invoke("anim.create", { name: "walk" });
        ops.invoke("anim.set_meta", { animation: "walk", looping: true });
        ops.invoke("anim.set_meta", { animation: "walk", duration: 3 });
        "#);

    let anim = edit.doc.animations.iter().next().map(|(_, a)| a).unwrap();
    assert!((anim.duration - 3.0).abs() < 1e-4);
    assert!(anim.looping, "the flag the second call did not mention");
}

#[test]
fn an_image_can_be_renamed_and_the_attachment_follows() {
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk\
                       YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("asset.add_image", {{ name: "torso", bytes_base64: "{PNG}" }});
        ops.invoke("attachment.create_region",
                   {{ slot: "body", name: "torso", texture: "torso" }});
        ops.invoke("asset.rename", {{ name: "torso", to: "chest" }});
        "#
    ));

    let names: Vec<&str> = edit
        .doc
        .assets
        .images
        .values()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["chest"]);

    let ankhimate_core::attachment::Attachment::Region(region) = attachment_of(&edit, "torso")
    else {
        panic!("not a region");
    };
    assert_eq!(
        region.texture, "chest",
        "the attachment points at the image's new name"
    );
}

#[test]
fn a_bone_colour_leaves_alpha_alone_like_every_other_colour() {
    let edit = run(&format!(
        r#"{RIG}
        ops.invoke("bone.set_color", {{ name: "arm", color: [1, 0, 0, 0.25] }});
        ops.invoke("bone.set_color", {{ name: "arm", color: [0, 0, 1] }});
        "#
    ));

    let bone = edit
        .doc
        .skeleton
        .bones
        .iter()
        .find(|(_, b)| b.name == "arm")
        .map(|(_, b)| b)
        .expect("the bone");
    assert_eq!(bone.color[2], 1.0);
    assert_eq!(bone.color[3], 0.25, "the alpha it did not mention survived");
}

#[test]
fn the_verb_surface_lists_and_describes_every_verb() {
    // The listing is how a plugin author finds a verb at all, and a schema is
    // how they find its arguments. This is also the check that the surface is
    // *finished* rather than merely large — a verb registered but absent from
    // either is one nobody can call without reading our source.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let log = host
        .run(
            r#"
            const ids = ops.list();
            console.log(String(ids.length));
            console.log(String(ids.every(id => ops.schema(id) !== null)));
            console.log(String(ids.every(id => id.includes("."))));
            "#,
            &mut edit,
        )
        .expect("the script runs");

    let count: usize = log[0].parse().expect("a number");
    assert!(count >= 49, "the surface shrank: {count} verbs");
    assert_eq!(log[1], "true", "every verb describes its arguments");
    assert_eq!(
        log[2], "true",
        "and every id is dotted, as the contract says"
    );
}
