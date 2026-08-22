//! Constraints from JavaScript.
//!
//! The gap: a plugin could build bones, slots and artwork but not a single
//! constraint, so a plugin importer could not match the built-in one. The PSD
//! reader creates IK from `[ik]` and physics from `[physics:cloth]`, and a
//! JavaScript importer of any other layered format had no way to say the same
//! thing.
//!
//! Driven through the real host rather than by calling the operators directly:
//! a verb that compiles proves nothing about whether its arguments survive JSON
//! in the shape the schema advertises.

use ankhimate_plugins::Host;

fn run(script: &str) -> ankhimate_document::Edit {
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    host.run(script, &mut edit).expect("the script runs");
    edit
}

/// A two-bone chain, which is the least IK will accept.
const ARM: &str = r#"
    ops.invoke("bone.create", { name: "upper", x: 0, y: 0, length: 40 });
    ops.invoke("bone.create", { name: "fore", parent: "upper", x: 40, y: 0, length: 40 });
"#;

#[test]
fn a_script_can_build_an_ik_chain() {
    let edit = run(&format!(
        r#"{ARM}
        ops.invoke("constraint.create_ik", {{ name: "arm_ik", bones: ["upper", "fore"] }});
        "#
    ));

    let constraint = edit
        .doc
        .skeleton
        .constraints
        .iter()
        .map(|(_, c)| c)
        .next()
        .expect("one constraint");
    assert_eq!(constraint.name(), "arm_ik");

    // The target is created, not named: it is the handle an animator drags, and
    // a chain with no handle is an IK constraint nobody can pose.
    assert!(
        edit.doc
            .skeleton
            .bones
            .iter()
            .any(|(_, b)| b.name.contains("arm_ik")),
        "a target bone was made for the chain"
    );
}

#[test]
fn a_one_bone_chain_is_refused_with_a_reason() {
    // IK over one bone has nothing to bend between. Refusing is right; refusing
    // silently would leave a script wondering why its rig has no constraint.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .run(
            &format!(
                r#"{ARM}
                ops.invoke("constraint.create_ik", {{ name: "no", bones: ["upper"] }});
                "#
            ),
            &mut edit,
        )
        .expect_err("one bone is not a chain");

    let message = error.to_string();
    assert!(
        message.contains("two bones"),
        "the reason names what was wrong: {message}"
    );
}

#[test]
fn tuning_a_constraint_leaves_what_it_acts_on_alone() {
    // The reason create and configure are separate verbs. A `set` that also
    // took `bones` would let a typo rebuild the constraint rather than fail,
    // and rebuilding is not what "set the mix" means.
    let edit = run(&format!(
        r#"{ARM}
        ops.invoke("constraint.create_ik", {{ name: "arm_ik", bones: ["upper", "fore"] }});
        ops.invoke("constraint.set_ik", {{ name: "arm_ik", mix: 0.5, bend_direction: -1 }});
        "#
    ));

    let ik = edit
        .doc
        .skeleton
        .constraints
        .iter()
        .find_map(|(_, c)| match c {
            ankhimate_core::constraints::Constraint::Ik(ik) => Some(ik),
            _ => None,
        })
        .expect("the IK constraint");

    assert_eq!(ik.mix, 0.5);
    assert_eq!(ik.bend_direction, -1.0);
    assert_eq!(ik.bones.len(), 2, "the chain is untouched");
}

#[test]
fn an_argument_left_out_keeps_its_current_value() {
    // "Leave it" and "reset it to a default the caller never saw" are different
    // answers, and only one of them is what a partial edit means.
    let edit = run(&format!(
        r#"{ARM}
        ops.invoke("constraint.create_ik", {{ name: "arm_ik", bones: ["upper", "fore"] }});
        ops.invoke("constraint.set_ik", {{ name: "arm_ik", mix: 0.25 }});
        ops.invoke("constraint.set_ik", {{ name: "arm_ik", softness: 3 }});
        "#
    ));

    let ik = edit
        .doc
        .skeleton
        .constraints
        .iter()
        .find_map(|(_, c)| match c {
            ankhimate_core::constraints::Constraint::Ik(ik) => Some(ik),
            _ => None,
        })
        .expect("the IK constraint");

    assert_eq!(ik.softness, 3.0, "the second edit landed");
    assert_eq!(ik.mix, 0.25, "and the first one survived it");
}

#[test]
fn a_script_can_hang_physics_on_a_bone() {
    let edit = run(&format!(
        r#"{ARM}
        ops.invoke("constraint.create_physics", {{ name: "cape", bone: "fore" }});
        ops.invoke("constraint.set_physics", {{ name: "cape", mass: 2.5, gravity_y: -4 }});
        "#
    ));

    let physics = edit
        .doc
        .skeleton
        .constraints
        .iter()
        .find_map(|(_, c)| match c {
            ankhimate_core::constraints::Constraint::Physics(p) => Some(p),
            _ => None,
        })
        .expect("the physics constraint");

    assert_eq!(physics.mass, 2.5);
    assert_eq!(physics.gravity.y, -4.0);
}

#[test]
fn a_transform_constraint_reads_its_offset_in_degrees() {
    // Degrees at the boundary, radians inside `core` (PLAN §2.7). A verb that
    // passed the number through would give a rig off by a factor of 57.
    let edit = run(&format!(
        r#"{ARM}
        ops.invoke("bone.create", {{ name: "driver" }});
        ops.invoke("constraint.create_transform",
                   {{ name: "follow", target: "driver", bones: ["fore"] }});
        ops.invoke("constraint.set_transform",
                   {{ name: "follow", mix_rotate: 1, offset_rotation: 90 }});
        "#
    ));

    let transform = edit
        .doc
        .skeleton
        .constraints
        .iter()
        .find_map(|(_, c)| match c {
            ankhimate_core::constraints::Constraint::Transform(t) => Some(t),
            _ => None,
        })
        .expect("the transform constraint");

    assert_eq!(transform.mix.rotate, 1.0);
    assert!(
        (transform.offsets.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "90 degrees became {} radians",
        transform.offsets.rotation
    );
}

#[test]
fn naming_the_wrong_kind_of_constraint_fails_rather_than_doing_nothing() {
    // `constraint.set_ik` on a physics constraint is a mistake, and a verb that
    // returned quietly would leave the script believing its edit landed.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let error = host
        .run(
            &format!(
                r#"{ARM}
                ops.invoke("constraint.create_physics", {{ name: "cape", bone: "fore" }});
                ops.invoke("constraint.set_ik", {{ name: "cape", mix: 0.5 }});
                "#
            ),
            &mut edit,
        )
        .expect_err("a physics constraint is not an IK one");

    assert!(
        error.to_string().contains("IK"),
        "the reason says which kind was wanted: {error}"
    );
}

#[test]
fn every_constraint_verb_is_listed_with_a_schema() {
    // The listing is how a plugin author finds a verb at all, and a schema is
    // how they find its arguments. A verb registered but absent from either is
    // one nobody can call without reading our source.
    let host = Host::new();
    let mut edit = ankhimate_document::Edit::default();
    let log = host
        .run(
            r#"
            const ids = ops.list().filter(id => id.startsWith("constraint."));
            console.log(JSON.stringify(ids.sort()));
            console.log(String(ids.every(id => ops.schema(id) !== null)));
            "#,
            &mut edit,
        )
        .expect("the script runs");

    assert_eq!(
        log[0],
        r#"["constraint.create_ik","constraint.create_physics","constraint.create_transform","constraint.delete","constraint.set_ik","constraint.set_physics","constraint.set_transform"]"#
    );
    assert_eq!(log[1], "true", "every one describes its arguments");
}
