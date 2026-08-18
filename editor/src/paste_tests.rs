//! Copy/paste round-trip tests (T-209, T-909).
//!
//! In the editor rather than beside `PasteBones`, because the copy half is
//! `AppState::copy_selection` — it reads the selection, which is session state.
//! The paste half is a command and lives in the document crate; these exercise
//! the pair, which is the part that has actually broken before.

#[cfg(test)]
mod tests {

    use ankhimate_core::ids::BoneId;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;
    use ankhimate_document::clipboard::Clipboard;
    use ankhimate_document::commands::History;
    use ankhimate_document::commands::bone_cmds::PasteBones;
    use ankhimate_document::doc::Document;

    fn bone(name: &str, parent: Option<BoneId>) -> Bone {
        Bone {
            name: name.to_string(),
            parent,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn pasting_a_subtree_brings_its_constraints_and_keys() {
        use ankhimate_core::animation::{Animation, Key, Timeline};
        use ankhimate_core::constraints::{Constraint, IkConstraint};

        let mut doc = Document::new();
        let shoulder = doc.skeleton.add_bone(bone("shoulder", None));
        let elbow = doc.skeleton.add_bone(bone("elbow", Some(shoulder)));
        let target = doc.skeleton.add_bone(bone("hand_target", Some(shoulder)));
        doc.skeleton
            .add_constraint(Constraint::Ik(IkConstraint::chain(
                "arm_ik",
                target,
                vec![shoulder, elbow],
            )));
        let mut walk = Animation::new("walk", 1.0);
        walk.timelines.push(Timeline::BoneRotate {
            bone: elbow,
            keys: vec![Key::linear(0.0, 0.0), Key::linear(0.5, 45.0)],
        });
        doc.animations.insert(walk);

        let mut state = crate::app_state::AppState {
            doc,
            ..Default::default()
        };
        state.session.selected_bones = vec![shoulder];
        state.copy_selection();

        let Clipboard::Bones(clip) = state.session.clipboard.clone() else {
            panic!("expected a bone clip");
        };
        assert_eq!(clip.bones.len(), 3, "the whole subtree");
        assert_eq!(clip.constraints.len(), 1, "the IK came along");
        assert_eq!(clip.animations.len(), 1, "so did the keys");

        // Paste it as a second, unparented limb.
        let mut history = History::default();
        let before_bones = state.doc.skeleton.bones.len();
        history.push(Box::new(PasteBones::new(clip, None)), &mut state.doc);

        assert_eq!(state.doc.skeleton.bones.len(), before_bones + 3);
        assert_eq!(
            state.doc.skeleton.constraints.len(),
            2,
            "the pasted limb has its own IK"
        );
        // The pasted constraint points at the *pasted* bones, not the originals.
        let pasted_ik = state
            .doc
            .skeleton
            .constraints
            .iter()
            .filter_map(|(_, c)| match c {
                Constraint::Ik(ik) => Some(ik),
                _ => None,
            })
            .find(|ik| !ik.bones.contains(&shoulder))
            .expect("a second IK over new bones");
        assert!(
            !pasted_ik.bones.contains(&elbow) && pasted_ik.target != target,
            "the copy must not reach back into the original limb"
        );

        // The keys landed in the existing `walk` rather than a second clip.
        assert_eq!(state.doc.animations.len(), 1, "no duplicate walk");
        let (_, walk) = state.doc.animations.iter().next().unwrap();
        assert_eq!(walk.timelines.len(), 2, "the original plus the pasted one");

        history.undo(&mut state.doc);
        assert_eq!(state.doc.skeleton.bones.len(), before_bones);
        assert_eq!(state.doc.skeleton.constraints.len(), 1);
        let (_, walk) = state.doc.animations.iter().next().unwrap();
        assert_eq!(
            walk.timelines.len(),
            1,
            "undo removed only the timelines the paste added"
        );
    }

    /// A constraint reaching outside the copy is reported, not pasted
    /// half-wired (T-909).
    #[test]
    fn a_constraint_reaching_outside_the_selection_is_left_behind() {
        use ankhimate_core::constraints::{Constraint, IkConstraint};

        let mut doc = Document::new();
        let arm = doc.skeleton.add_bone(bone("arm", None));
        let elsewhere = doc.skeleton.add_bone(bone("elsewhere", None));
        // The target lives outside the subtree being copied.
        doc.skeleton
            .add_constraint(Constraint::Ik(IkConstraint::chain(
                "reaching",
                elsewhere,
                vec![arm],
            )));

        let mut state = crate::app_state::AppState {
            doc,
            ..Default::default()
        };
        state.session.selected_bones = vec![arm];
        state.copy_selection();

        let Clipboard::Bones(clip) = state.session.clipboard.clone() else {
            panic!("expected a bone clip");
        };
        assert!(clip.constraints.is_empty(), "not rebuilt from nothing");
        assert_eq!(
            clip.dropped_constraints,
            vec!["reaching".to_string()],
            "and the user is told which one"
        );
    }
}
