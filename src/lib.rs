//! The undoable document, the commands that edit it, and nothing that needs a
//! screen.
//!
//! Split out of `editor` so a headless consumer — an exporter, an MCP server, a
//! plugin host — can build and edit a rig without linking egui or wgpu
//! (`docs/plugin-plan.md`). The editor depends on this crate; nothing here
//! depends on the editor.
//!
//! # What is here, and what is deliberately not
//!
//! Here: [`Document`], every [`EditCommand`], the undo [`History`], the
//! clipboard, and [`WorkMode`]. All of it is a function of saved state.
//!
//! Not here: selection, tools, gizmo modes, the playhead, the derived pose.
//! Those are interaction state, they live in the editor's `Session`, and a
//! headless caller has no use for them. The line is the one `CLAUDE.md` already
//! draws — "if it would be wrong to find it in a teammate's file, it belongs in
//! the session".
//!
//! # The rule this crate exists to keep
//!
//! **Every document edit is an undoable command.** Mutating [`Document`]
//! directly bypasses undo, and the editor's `dispatch` is not reachable from
//! here — so a consumer of this crate has to go through [`History`], which is
//! the point.

#![forbid(unsafe_code)]

pub mod args;
pub mod clipboard;
pub mod commands;
pub mod constraint_ops;
pub mod doc;
pub mod doc_ops;
pub mod edit;
pub mod import_ops;
/// Retriangulation for meshes — geometry, so it travels with the commands that
/// call it rather than with the viewport that displays the result.
pub mod meshgen;
pub mod ops;
pub mod part_ops;
pub mod psd_mesh;
pub mod read;
pub mod rebind;
pub mod rig_ops;
pub mod shape_ops;
mod work_mode;

pub use args::{ArgError, Args, Resolver};
pub use commands::{EditCommand, History, IdRemap};
pub use doc::{Document, DocumentMeta};
pub use edit::{Approximation, Edit, Refused};
pub use ops::{DocOperator, DocOps, OpError};
pub use read::{Names, describe, names};
pub use work_mode::WorkMode;

#[cfg(test)]
mod tests {
    use super::*;
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

    /// The property this crate exists to have.
    ///
    /// A rig is built, edited and undone with no `Session`, no `AppState` and no
    /// egui in scope — which is what lets an exporter, an MCP server or a plugin
    /// host do the same. If this stops compiling, something UI-shaped has leaked
    /// into the document layer.
    #[test]
    fn a_rig_is_built_and_undone_without_a_session() {
        use commands::bone_cmds::{CreateBone, SetBoneTransform};

        let mut doc = Document::new();
        let mut history = History::default();

        history.push(Box::new(CreateBone::new(bone("root"))), &mut doc);
        assert_eq!(doc.skeleton.bones.len(), 1);

        let id = doc.skeleton.update_order[0];
        let moved = Transform {
            position: glam::vec2(30.0, 0.0),
            ..Default::default()
        };
        history.push(Box::new(SetBoneTransform::new(id, moved)), &mut doc);
        assert_eq!(doc.skeleton.bones[id].local_transform.position.x, 30.0);

        history.undo(&mut doc);
        assert_eq!(doc.skeleton.bones[id].local_transform.position.x, 0.0);
    }

    /// Setup-only commands still declare their mode with no session to read it
    /// from. `WorkMode` moved down here for exactly that: `requires_mode` is a
    /// property of the command, and the editor holds the current *value*.
    #[test]
    fn a_structural_command_declares_its_mode_here() {
        use commands::bone_cmds::CreateBone;

        let cmd = CreateBone::new(bone("b"));
        assert_eq!(cmd.requires_mode(), Some(WorkMode::Setup));
    }
}
