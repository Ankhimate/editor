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

use super::registry::{OpResult, Operator, Registry, UiRequest};
use crate::app_state::AppState;
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

            fn invoke(&self, $state: &mut AppState) -> OpResult {
                $body
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

    fn invoke(&self, state: &mut AppState) -> OpResult {
        // `enabled` is advisory — a caller may invoke directly — so the target
        // is re-checked here rather than unwrapped.
        let Some(anim) = state.session.active_animation else {
            return OpResult::done();
        };
        // Named after the frame it lands on: an animator marking a pose knows
        // which pose it is, and a dialog mid-scrub would break the rhythm.
        let fps = state.doc.meta.fps.max(1) as f32;
        let playhead = state.session.playhead;
        let frame = (playhead * fps).round() as i64;
        state.dispatch(Box::new(super::marker_cmds::AddMarker::new(
            anim,
            format!("f{frame}"),
            playhead,
        )));
        OpResult::done()
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
pub fn register_builtins(registry: &mut Registry) {
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
    use crate::commands::registry::Registry;

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
    fn settings_asks_the_app_rather_than_opening_anything() {
        // Operators reach AppState only; chrome above it is a request the app
        // honours. This is what keeps a future plugin out of the frame loop.
        let mut state = AppState::default();
        let result = OpenSettings.invoke(&mut state);
        assert_eq!(result.ui, Some(UiRequest::Settings));
    }
}
