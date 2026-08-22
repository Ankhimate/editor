//! The built-in operators.
//!
//! Every verb here was previously a hardcoded key handler in `App::update` —
//! bare `if ctx.input(|i| i.key_pressed(..))` blocks, each with its
//! preconditions written inline at the one place it was bound. Naming them does
//! three things: a keymap can rebind them (Phase 7), a menu can grey them out
//! from `enabled` rather than repeating the check, and a plugin can shadow one.
//!
//! None of them contain logic. Each wraps a method that already exists on
//! [`AppState`] or [`Session`](crate::session::Session), so this module is a
//! naming layer and not a second implementation to keep in sync.
//!
//! # Ids
//!
//! Dotted, `domain.verb`, and **stable**: they are what a user's keymap file and
//! a plugin's `shadow` call refer to, neither of which the compiler checks.
//! Treat a rename here the way `docs/export-context.md` treats a field rename.

use crate::app_state::AppState;
use crate::registry::{OpResult, Operator, Registry, UiRequest};
use crate::session::{Tool, TransformTool, WorkMode};

/// Declare an operator that calls one method and needs no arguments.
///
/// The repetitive part of an operator is its four accessors; the interesting
/// part is `invoke` and occasionally `enabled`. Spelling out ~20 near-identical
/// `impl` blocks buries the difference between them in boilerplate, so the macro
/// carries the uniform half and each entry states only what makes it itself.
macro_rules! operator {
    (
        $name:ident, $id:literal, $label:literal
        $(, mode: $mode:expr)?
        $(, enabled: |$es:ident| $enabled:expr)?
        , run: |$state:ident| $body:expr
    ) => {
        pub struct $name;

        impl Operator for $name {
            fn id(&self) -> &'static str {
                $id
            }

            fn label(&self) -> &str {
                $label
            }

            $(
                fn requires_mode(&self) -> Option<WorkMode> {
                    Some($mode)
                }
            )?

            $(
                fn enabled(&self, $es: &AppState) -> bool {
                    $enabled
                }
            )?

            fn invoke(
                &self,
                $state: &mut AppState,
                _args: &ankhimate_document::Args,
            ) -> Result<OpResult, ankhimate_document::ArgError> {
                Ok($body)
            }
        }
    };
}

// ── Edit ─────────────────────────────────────────────────────────────────────

operator!(Undo, "edit.undo", "Undo",
    enabled: |s| s.history.can_undo(),
    run: |s| { s.undo(); OpResult::done() });

operator!(Redo, "edit.redo", "Redo",
    enabled: |s| s.history.can_redo(),
    run: |s| { s.redo(); OpResult::done() });

operator!(CopySelection, "edit.copy", "Copy",
    enabled: |s| !s.session.selected_bones.is_empty(),
    run: |s| { s.copy_selection(); OpResult::done() });

operator!(CopyPose, "edit.copy_pose", "Copy Pose",
    enabled: |s| !s.session.selected_bones.is_empty(),
    run: |s| { s.copy_pose(); OpResult::done() });

operator!(Paste, "edit.paste", "Paste",
    run: |s| { s.paste(false); OpResult::done() });

operator!(PasteMirrored, "edit.paste_mirrored", "Paste Mirrored",
    run: |s| { s.paste(true); OpResult::done() });

operator!(DuplicateSelection, "edit.duplicate", "Duplicate",
    enabled: |s| !s.session.selected_bones.is_empty(),
    run: |s| { s.duplicate_selection(); OpResult::done() });

// Rename opens a dialog rather than editing, so it reports the request and lets
// the app own the window. Guarded on a selection: an empty rename dialog is a
// dead end, which is why the F2 binding checked for one inline.
operator!(RenameSelection, "edit.rename", "Rename",
    enabled: |s| !s.session.selected_bones.is_empty(),
    run: |_s| OpResult::ui(UiRequest::Rename));

// ── Mode and keying ──────────────────────────────────────────────────────────

operator!(ToggleWorkMode, "mode.toggle", "Toggle Setup/Animate",
    run: |s| { s.toggle_work_mode(); OpResult::done() });

// A no-op in Setup mode by construction, but `enabled` states it so a menu can
// grey out rather than accepting a click that does nothing.
operator!(KeyPendingPose, "anim.key_pose", "Key Pose",
    mode: WorkMode::Animate,
    enabled: |s| s.session.is_animating(),
    run: |s| { s.key_pending_pose(); OpResult::done() });

/// Drop a marker at the playhead (T-906).
///
/// The one built-in that builds an `EditCommand` itself rather than calling an
/// `AppState` method, because the binding in `App::update` did the same inline.
pub struct AddMarkerAtPlayhead;

impl Operator for AddMarkerAtPlayhead {
    fn id(&self) -> &'static str {
        "anim.add_marker"
    }

    fn label(&self) -> &str {
        "Add Marker"
    }

    fn enabled(&self, state: &AppState) -> bool {
        state.session.active_animation.is_some()
    }

    fn invoke(
        &self,
        state: &mut AppState,
        _args: &ankhimate_document::Args,
    ) -> Result<OpResult, ankhimate_document::ArgError> {
        // `enabled` is advisory — a caller may invoke directly — so the target
        // is re-checked here rather than unwrapped.
        let Some(anim) = state.session.active_animation else {
            return Ok(OpResult::done());
        };
        // Named after the frame it lands on: an animator marking a pose knows
        // which pose it is, and a dialog mid-scrub would break the rhythm.
        let fps = state.doc.meta.fps.max(1) as f32;
        let playhead = state.session.playhead;
        let frame = (playhead * fps).round() as i64;
        state.dispatch(Box::new(
            ankhimate_document::commands::marker_cmds::AddMarker::new(
                anim,
                format!("f{frame}"),
                playhead,
            ),
        ));
        Ok(OpResult::done())
    }
}

// ── Tools ────────────────────────────────────────────────────────────────────

operator!(SelectTool, "tool.select", "Select Tool",
    run: |s| { s.session.tool = Tool::Select; OpResult::done() });

operator!(CreateBoneTool, "tool.create_bone", "Create Bone Tool",
    mode: WorkMode::Setup,
    enabled: |s| s.session.can_edit_structure(),
    run: |s| { s.session.tool = Tool::CreateBone; OpResult::done() });

operator!(WeightPaintTool, "tool.weight_paint", "Weight Paint Tool",
    mode: WorkMode::Setup,
    enabled: |s| s.session.can_edit_structure(),
    run: |s| { s.session.tool = Tool::WeightPaint; OpResult::done() });

// ── Transform gizmo mode ─────────────────────────────────────────────────────

operator!(TranslateGizmo, "gizmo.translate", "Translate",
run: |s| {
    s.session.active_transform_tool = TransformTool::Translate;
    OpResult::done()
});

operator!(RotateGizmo, "gizmo.rotate", "Rotate",
run: |s| {
    s.session.active_transform_tool = TransformTool::Rotate;
    OpResult::done()
});

operator!(ScaleGizmo, "gizmo.scale", "Scale",
run: |s| {
    s.session.active_transform_tool = TransformTool::Scale;
    OpResult::done()
});

operator!(ShearGizmo, "gizmo.shear", "Shear",
run: |s| {
    s.session.active_transform_tool = TransformTool::Shear;
    OpResult::done()
});

// ── View ─────────────────────────────────────────────────────────────────────

operator!(ToggleArtwork, "view.toggle_artwork", "Show Artwork",
run: |s| {
    s.session.show_artwork = !s.session.show_artwork;
    OpResult::done()
});

operator!(ToggleBones, "view.toggle_bones", "Show Bones",
run: |s| {
    s.session.show_bones = !s.session.show_bones;
    OpResult::done()
});

operator!(OpenSettings, "app.settings", "Settings",
    run: |_s| OpResult::ui(UiRequest::Settings));

/// Register every built-in.
///
/// Order is irrelevant to lookup — ids are unique, and `builtin_ids_are_unique`
/// in `registry.rs` fails the build if that ever stops being true.
/// Wraps a document verb so the editor's registry can hold it.
///
/// The two traits differ only in what they are handed — `AppState` versus
/// `Edit` — so this is a shim rather than a reimplementation. It exists so one
/// id resolves to one verb from a menu, a keybinding, a plugin and a script
/// alike; two registries with overlapping ids would drift exactly as the Edit
/// menu drifted from the keymap before the registry existed.
struct Adopted(Box<dyn ankhimate_document::DocOperator>);

impl Operator for Adopted {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn label(&self) -> &str {
        self.0.label()
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        self.0.requires_mode()
    }

    fn schema(&self) -> serde_json::Value {
        self.0.schema()
    }

    fn enabled(&self, state: &AppState) -> bool {
        // The mode check a document verb cannot make for itself: it declares
        // which mode it needs, and only the editor knows which one is current.
        match self.0.requires_mode() {
            Some(required) => state.session.work_mode == required,
            None => true,
        }
    }

    fn invoke(
        &self,
        state: &mut AppState,
        args: &ankhimate_document::Args,
    ) -> Result<OpResult, ankhimate_document::ArgError> {
        // `Edit` owns its document and history, and `AppState` owns the editor's
        // — so the two are swapped in for the call and swapped back. Cheaper
        // than it looks: both are moves, and the alternative is a second
        // dispatch path that can drift from `AppState::dispatch`.
        let mut edit = ankhimate_document::Edit {
            doc: std::mem::take(&mut state.doc),
            history: std::mem::take(&mut state.history),
            // The editor has no importer running, so nothing reports into this.
            dangling: Vec::new(),
            report: Vec::new(),
            mode: state.session.work_mode,
        };
        let outcome = self.0.invoke(&mut edit, args);
        state.doc = edit.doc;
        state.history = edit.history;

        match outcome {
            Ok(()) => {
                // The editor's own bookkeeping, which `Edit` does not do:
                // selection pruning, the pose, the revision counter.
                state.after_document_change();
                Ok(OpResult::done())
            }
            // A refusal is not an argument error — the caller asked correctly
            // and the mode said no. `enabled` should have caught it, so this is
            // the direct-invoke path.
            Err(ankhimate_document::OpError::Refused(_)) => Ok(OpResult::done()),
            Err(ankhimate_document::OpError::Unknown(_)) => Ok(OpResult::done()),
            Err(ankhimate_document::OpError::Args(e)) => Err(e),
        }
    }
}

pub fn register_builtins(registry: &mut Registry) {
    // Document verbs first, so a session verb registered later shadows one
    // deliberately rather than by accident of ordering.
    for op in ankhimate_document::DocOps::builtin().into_ops() {
        registry.register(Box::new(Adopted(op)));
    }

    registry.register(Box::new(Undo));
    registry.register(Box::new(Redo));
    registry.register(Box::new(CopySelection));
    registry.register(Box::new(CopyPose));
    registry.register(Box::new(Paste));
    registry.register(Box::new(PasteMirrored));
    registry.register(Box::new(DuplicateSelection));
    registry.register(Box::new(RenameSelection));

    registry.register(Box::new(ToggleWorkMode));
    registry.register(Box::new(KeyPendingPose));
    registry.register(Box::new(AddMarkerAtPlayhead));

    registry.register(Box::new(SelectTool));
    registry.register(Box::new(CreateBoneTool));
    registry.register(Box::new(WeightPaintTool));

    registry.register(Box::new(TranslateGizmo));
    registry.register(Box::new(RotateGizmo));
    registry.register(Box::new(ScaleGizmo));
    registry.register(Box::new(ShearGizmo));

    registry.register(Box::new(ToggleArtwork));
    registry.register(Box::new(ToggleBones));
    registry.register(Box::new(OpenSettings));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    fn registry() -> Registry {
        Registry::with_builtins()
    }

    #[test]
    fn every_builtin_id_is_dotted() {
        // The id space is a public contract; a bare word would collide with a
        // future domain and read as a typo in a keymap file.
        for id in registry().ids() {
            assert!(
                id.contains('.') && !id.starts_with('.') && !id.ends_with('.'),
                "{id} is not domain.verb"
            );
        }
    }

    #[test]
    fn a_structural_tool_is_disabled_in_animate() {
        // T-207: the Create Bone tool is Setup-only. Previously the `b` key
        // checked `can_edit_structure()` at the binding; now the operator owns
        // it, so any caller gets the same answer.
        let mut state = AppState::default();

        state.session.work_mode = WorkMode::Setup;
        assert!(CreateBoneTool.enabled(&state));

        state.session.work_mode = WorkMode::Animate;
        assert!(!CreateBoneTool.enabled(&state));
    }

    #[test]
    fn a_disabled_tool_operator_does_not_change_the_tool() {
        // The registry refuses it, so the session keeps whatever tool it had.
        let registry = registry();
        let mut state = AppState::default();
        state.session.work_mode = WorkMode::Animate;
        state.session.tool = Tool::Select;

        assert!(registry.invoke("tool.create_bone", &mut state).is_none());
        assert_eq!(state.session.tool, Tool::Select, "tool untouched");
    }

    #[test]
    fn the_select_tool_is_available_in_both_modes() {
        // Selection is not a structural edit; gating it on Setup would strand an
        // animator with whatever tool was last active.
        let mut state = AppState::default();
        state.session.work_mode = WorkMode::Animate;
        assert!(SelectTool.enabled(&state));
        assert!(SelectTool.requires_mode().is_none());
    }

    #[test]
    fn rename_needs_a_selection() {
        let registry = registry();
        let mut state = AppState::default();

        assert!(
            registry.invoke("edit.rename", &mut state).is_none(),
            "no selection, no dialog"
        );
    }

    #[test]
    fn gizmo_operators_set_their_own_mode() {
        let registry = registry();
        let mut state = AppState::default();

        for (id, expected) in [
            ("gizmo.translate", TransformTool::Translate),
            ("gizmo.rotate", TransformTool::Rotate),
            ("gizmo.scale", TransformTool::Scale),
            ("gizmo.shear", TransformTool::Shear),
        ] {
            registry.invoke(id, &mut state).expect("registered");
            assert_eq!(state.session.active_transform_tool, expected, "{id}");
        }
    }

    #[test]
    fn view_toggles_flip_and_flip_back() {
        let registry = registry();
        let mut state = AppState::default();
        let before = state.session.show_bones;

        registry.invoke("view.toggle_bones", &mut state).unwrap();
        assert_eq!(state.session.show_bones, !before);
        registry.invoke("view.toggle_bones", &mut state).unwrap();
        assert_eq!(state.session.show_bones, before);
    }

    #[test]
    fn undo_is_disabled_with_an_empty_history() {
        let registry = registry();
        let mut state = AppState::default();
        assert!(registry.invoke("edit.undo", &mut state).is_none());
    }

    #[test]
    fn a_marker_needs_an_animation() {
        let registry = registry();
        let mut state = AppState::default();
        state.session.active_animation = None;
        assert!(registry.invoke("anim.add_marker", &mut state).is_none());
    }

    #[test]
    fn a_bone_is_created_by_name_and_is_undoable() {
        // What a plugin or an MCP client does: name the verb, name the target,
        // never hold an id. The edit still goes through `dispatch`, so undo
        // works exactly as it does for a menu.
        use ankhimate_document::Args;

        let registry = registry();
        let mut state = AppState::default();
        state.session.work_mode = WorkMode::Setup;

        registry
            .try_invoke(
                "bone.create",
                &mut state,
                &Args::from_json(serde_json::json!({ "name": "root" })),
            )
            .expect("arguments read")
            .expect("not declined");

        let root = state
            .doc
            .skeleton
            .bones
            .values()
            .find(|b| b.name == "root")
            .expect("root exists");
        assert_eq!(root.parent, None);

        // A second bone under the first, placed and turned.
        registry
            .try_invoke(
                "bone.create",
                &mut state,
                &Args::from_json(serde_json::json!({
                    "name": "spine", "parent": "root", "y": 40.0, "rotation": 90.0
                })),
            )
            .expect("arguments read")
            .expect("not declined");

        let (spine_id, spine) = state
            .doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "spine")
            .expect("spine exists");
        assert!(spine.parent.is_some(), "parented by name");
        assert_eq!(spine.local_transform.position.y, 40.0);
        assert!(
            (spine.local_transform.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "degrees at the boundary, radians inside core"
        );

        state.undo();
        assert!(
            state.doc.skeleton.bones.get(spine_id).is_none(),
            "a plugin's edit undoes like any other"
        );
    }

    #[test]
    fn a_bad_argument_is_reported_and_changes_nothing() {
        // The failure a keybinding may swallow and a script may not. Naming a
        // parent the rig does not have is a bug in the caller, and the document
        // must not be half-edited by the time it is found.
        use ankhimate_document::{ArgError, Args};

        let registry = registry();
        let mut state = AppState::default();
        state.session.work_mode = WorkMode::Setup;
        let before = state.doc.skeleton.bones.len();

        let err = registry
            .try_invoke(
                "bone.create",
                &mut state,
                &Args::from_json(serde_json::json!({ "name": "arm", "parent": "nope" })),
            )
            .expect_err("an unresolvable parent is an error");

        assert!(matches!(err, ArgError::Unresolved { kind: "bone", .. }));
        assert_eq!(
            state.doc.skeleton.bones.len(),
            before,
            "nothing was created"
        );
    }

    #[test]
    fn a_keybinding_still_invokes_without_arguments() {
        // The quiet path stays quiet: `invoke` swallows the same failure that
        // `try_invoke` reports, because a key bound to something inapplicable
        // should do nothing rather than interrupt.
        let registry = registry();
        let mut state = AppState::default();
        state.session.work_mode = WorkMode::Setup;

        assert!(
            registry.invoke("bone.create", &mut state).is_none(),
            "no name given, so nothing happens"
        );
        assert_eq!(state.doc.skeleton.bones.len(), 0);
    }

    #[test]
    fn an_operator_that_takes_arguments_describes_them() {
        // What an MCP client lists tools from, and what a plugin author reads
        // instead of the source.
        // Through the registry, because the verb itself now lives in the
        // document crate — what matters here is that adoption carries the
        // schema across rather than flattening it to null.
        let registry = registry();
        let schema = registry.get("bone.create").expect("adopted").schema();
        assert_eq!(schema["required"][0], "name");
        assert!(schema["properties"]["parent"].is_object());
    }

    #[test]
    fn settings_asks_the_app_rather_than_opening_anything() {
        // Operators reach AppState only; chrome above it is a request the app
        // honours. This is what keeps a future plugin out of the frame loop.
        let mut state = AppState::default();
        let result = OpenSettings
            .invoke(&mut state, &ankhimate_document::Args::none())
            .expect("takes no arguments");
        assert_eq!(result.ui, Some(UiRequest::Settings));
    }
}
