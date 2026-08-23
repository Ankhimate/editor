//! Editing a document without an editor.
//!
//! `AppState::dispatch` is the editor's only sanctioned mutation path, and it
//! is the right one — it enforces undo and the Setup/Animate rule so no panel
//! has to remember them. But it needs a `Session` for the current mode and a
//! status line for refusals, and a headless caller has neither.
//!
//! [`Edit`] is the same guarantees without the UI: a document, its history, and
//! the mode to judge commands against. The editor keeps its own `dispatch`
//! because it has more to do afterwards — prune the selection, refresh the pose
//! — but both go through [`History::push_in_mode`], so a plugin's edit is
//! undoable and mode-checked exactly as a menu's is.

use crate::commands::{EditCommand, History};
use crate::doc::Document;
use crate::work_mode::WorkMode;

/// A document, its undo history, and the mode edits are judged against.
/// One thing an import could not carry across.
///
/// The same shape `formats::LoadReport` uses, kept here so a plugin importer
/// has the honesty property a Rust one has: an import that drops half a file
/// quietly is worse than one that refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approximation {
    /// The kind of thing — `"curve"`, `"attachment"`, `"timeline"`.
    pub what: String,
    /// Where, in the source's own names.
    pub where_: String,
    /// What was done instead.
    pub detail: String,
}

pub struct Edit {
    pub doc: Document,
    pub history: History,
    /// Name references that did not resolve while importing (`what`, `name`).
    /// Like [`Self::report`], this is diagnostic session state, not authored data.
    pub dangling: Vec<(String, String)>,
    /// What an importer could not represent exactly.
    ///
    /// Not undoable: a report is not part of the rig, and undoing an import's
    /// last bone should not un-say what that import could not do.
    pub report: Vec<Approximation>,
    /// Which half of the editor's two modes a headless caller is standing in.
    ///
    /// `Setup` by default: a script that builds a rig is doing structural work,
    /// and defaulting to `Animate` would refuse every command it issues with no
    /// UI to explain why.
    pub mode: WorkMode,
}

/// Why an edit did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The command is Setup-only and the caller is in Animate, or the reverse
    /// (T-207). Carries the mode the command wanted.
    WrongMode(WorkMode),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::WrongMode(WorkMode::Setup) => {
                write!(f, "this edit changes the rig, so it needs Setup mode")
            }
            Refused::WrongMode(WorkMode::Animate) => {
                write!(f, "this edit writes keys, so it needs Animate mode")
            }
        }
    }
}

impl std::error::Error for Refused {}

impl Default for Edit {
    fn default() -> Self {
        Self::new(Document::new())
    }
}

impl Edit {
    pub fn new(doc: Document) -> Self {
        Self {
            doc,
            history: History::default(),
            dangling: Vec::new(),
            report: Vec::new(),
            mode: WorkMode::Setup,
        }
    }

    /// Apply `cmd`, recording it for undo.
    ///
    /// Returns the mode the command wanted when it is refused, rather than a
    /// bare `false`. The editor turns that into a status line; a script turns it
    /// into an error its author can act on, which a boolean cannot support.
    pub fn dispatch(&mut self, cmd: Box<dyn EditCommand>) -> Result<(), Refused> {
        let required = cmd.requires_mode();
        if self.history.push_in_mode(cmd, &mut self.doc, self.mode) {
            // Binds are derived and not serialized, so a weighted mesh edited
            // headlessly is broken without this. It is document integrity, not
            // editor convenience — see `docs/plugin-plan.md`.
            self.rebind_meshes();
            Ok(())
        } else {
            Err(Refused::WrongMode(required.unwrap_or(WorkMode::Setup)))
        }
    }

    pub fn undo(&mut self) -> bool {
        let undone = self.history.undo(&mut self.doc);
        if undone {
            self.rebind_meshes();
        }
        undone
    }

    pub fn redo(&mut self) -> bool {
        let redone = self.history.redo(&mut self.doc);
        if redone {
            self.rebind_meshes();
        }
        redone
    }

    /// Give any weighted mesh missing its inverse binds a fresh set.
    ///
    /// Cheap when nothing needs it: the scan short-circuits unless a mesh is
    /// actually unbound.
    fn rebind_meshes(&mut self) {
        crate::rebind::rebind_meshes(&mut self.doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bone_cmds::CreateBone;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.into(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn a_script_builds_and_undoes_a_rig() {
        let mut edit = Edit::default();
        edit.dispatch(Box::new(CreateBone::new(bone("root"))))
            .expect("Setup by default");
        assert_eq!(edit.doc.skeleton.bones.len(), 1);

        assert!(edit.undo());
        assert_eq!(edit.doc.skeleton.bones.len(), 0);
    }

    /// The reason `rebind_meshes` moved down here.
    ///
    /// Inverse binds are `#[serde(skip)]` derived state, so a weighted mesh
    /// without them is broken — and a script that paints weights has no editor
    /// to run the pass for it. If this regresses, headless mesh edits produce a
    /// rig that loads and deforms wrongly, which is the worst kind of silent.
    #[test]
    fn a_headless_mesh_edit_gets_its_binds() {
        use ankhimate_core::attachment::{Attachment, MeshAttachment, VertexWeight};
        use ankhimate_core::slot::Slot;

        let mut edit = Edit::default();
        let root = edit.doc.skeleton.add_bone(bone("root"));
        let slot = edit
            .doc
            .skeleton
            .add_slot(Slot::new("body".to_string(), root));

        let skin = edit.doc.skeleton.default_skin;
        edit.doc.skeleton.skins[skin].set(
            slot,
            "mesh".to_string(),
            Attachment::Mesh(MeshAttachment {
                texture: "img".to_string(),
                setup_vertices: vec![glam::Vec2::ZERO, glam::vec2(10.0, 0.0)],
                uvs: vec![glam::Vec2::ZERO, glam::vec2(1.0, 0.0)],
                triangles: Vec::new(),
                weights: vec![
                    vec![VertexWeight {
                        bone: root,
                        weight: 1.0,
                    }],
                    vec![VertexWeight {
                        bone: root,
                        weight: 1.0,
                    }],
                ],
                ffd_keyframes: Vec::new(),
                edges: Vec::new(),
                inverse_bind_matrices: Default::default(),
                linked: None,
                sequence: None,
            }),
        );

        // Unbound as inserted — nothing has run the pass yet.
        let bound = |edit: &Edit| match edit.doc.skeleton.skins[skin].get(slot, "mesh") {
            Some(Attachment::Mesh(m)) => !m.inverse_bind_matrices.is_empty(),
            _ => panic!("the mesh is there"),
        };
        assert!(!bound(&edit), "no binds before an edit");

        edit.dispatch(Box::new(CreateBone::new(bone("spine"))))
            .expect("Setup by default");
        assert!(bound(&edit), "dispatching rebinds, with no editor involved");
    }

    #[test]
    fn the_mode_rule_holds_without_a_session_to_read_it_from() {
        // T-207 is a property of the command, so it applies to a script exactly
        // as to a panel — and the refusal says which mode was wanted, which a
        // boolean could not.
        let mut edit = Edit::default();
        edit.mode = WorkMode::Animate;

        let refused = edit
            .dispatch(Box::new(CreateBone::new(bone("root"))))
            .expect_err("structural edits are Setup-only");
        assert_eq!(refused, Refused::WrongMode(WorkMode::Setup));
        assert_eq!(edit.doc.skeleton.bones.len(), 0, "nothing was applied");
    }
}
