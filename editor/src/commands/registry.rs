//! Named operators: the verbs of the editor, addressable by string id.
//!
//! [`EditCommand`](super::EditCommand) is an *instance* — "move bone `b` from
//! here to there", carrying the data needed to reverse itself. That is the wrong
//! granularity for a keymap or a plugin, which need to name the **verb**
//! ("bone.create") without knowing which bone or where.
//!
//! An [`Operator`] is that verb. It has a stable id, knows whether it is
//! currently applicable, and when invoked reads live state to build and dispatch
//! whatever `EditCommand` the situation calls for. Undo, drag-merge and the
//! Setup/Animate rule (T-207) stay where they are — an operator that dispatches
//! gets them for free, and one that forgets cannot bypass them.
//!
//! # Why a registry and not a match
//!
//! Everything a plugin may extend has to be *looked up*, never called directly.
//! Blender's addons can shadow `mesh.subdivide` for exactly one reason: the
//! built-in registered under that name through the same door an addon uses. A
//! `match` on an enum closes the set at compile time and no amount of plugin
//! host can reopen it.
//!
//! # Shadowing is a chain, not a replacement
//!
//! [`Registry::register`] pushes onto a stack per id rather than overwriting.
//! The last registration wins, and [`Registry::shadowed`] still reaches the one
//! beneath it, so a plugin can wrap a built-in instead of only replacing it.
//!
//! Last-wins-and-forget is the cheaper implementation and it is what Blender
//! does; it is also a known sharp edge there, where two addons claiming one
//! idname leave the loser silently dead. Storing the stack costs a `Vec` now and
//! cannot be retrofitted once plugins depend on the semantics, so it goes in
//! before the first plugin exists rather than after.

use crate::app_state::AppState;
use crate::session::WorkMode;
use std::collections::BTreeMap;

/// A window the operator wants opened. Operators reach [`AppState`], but three
/// pieces of chrome live on `App` itself, above it.
///
/// Returning a request rather than taking `&mut App` keeps the operator surface
/// to the document and session. An operator — and later, a plugin — can ask for
/// the rename dialog; it cannot reach into the frame loop or the egui context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRequest {
    Settings,
    Rename,
    Startup,
}

/// What an invocation did.
#[derive(Debug, Default)]
pub struct OpResult {
    /// Chrome to open, if any.
    pub ui: Option<UiRequest>,
}

impl OpResult {
    /// The operator did its work and needs nothing from the app.
    pub fn done() -> Self {
        Self::default()
    }

    /// The operator wants a window opened.
    pub fn ui(request: UiRequest) -> Self {
        Self { ui: Some(request) }
    }
}

/// A named, invocable editor action.
///
/// Implementors are stateless descriptors — one instance lives in the registry
/// for the life of the app and is shared by menu, keymap and plugin callers.
pub trait Operator {
    /// Stable dotted id: `"bone.create"`, `"edit.undo"`. This is the name a
    /// keybinding, a menu entry and a plugin all refer to, so it is a public
    /// contract in the same sense `docs/export-context.md` is — renaming one
    /// breaks user configuration silently, with no compiler on that side.
    fn id(&self) -> &'static str;

    /// Human-readable name for menus and the keymap editor.
    fn label(&self) -> &str;

    /// The work mode this operator may run in, or `None` when it is legal in
    /// both (T-207).
    ///
    /// This mirrors [`EditCommand::requires_mode`](super::EditCommand::requires_mode)
    /// but is not a substitute for it: the command remains the enforcement
    /// point. Declaring it here lets a menu grey the entry out *before* the user
    /// commits, rather than refusing afterwards with a status message.
    fn requires_mode(&self) -> Option<WorkMode> {
        None
    }

    /// Whether invoking right now would do anything.
    ///
    /// Menus grey out on `false` and keybindings quietly no-op. Preconditions
    /// were previously inline at the call site — F2 checked for a selection
    /// where it was bound — which meant a second caller of the same verb had to
    /// remember the same check.
    fn enabled(&self, _state: &AppState) -> bool {
        true
    }

    /// Do the thing. Reads live selection and playhead from `state`, and
    /// dispatches any document edit through
    /// [`AppState::dispatch`](crate::app_state::AppState::dispatch).
    fn invoke(&self, state: &mut AppState) -> OpResult;
}

/// Operators by id, with shadowing.
#[derive(Default)]
pub struct Registry {
    ops: BTreeMap<&'static str, Vec<Box<dyn Operator>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The registry every built-in registers into. Built-ins go through
    /// [`register`](Self::register) exactly as a plugin will.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        super::operators::register_builtins(&mut registry);
        registry
    }

    /// Add `op` under its own id, shadowing any operator already there.
    pub fn register(&mut self, op: Box<dyn Operator>) {
        self.ops.entry(op.id()).or_default().push(op);
    }

    /// The operator currently answering to `id`, or `None`.
    pub fn get(&self, id: &str) -> Option<&dyn Operator> {
        self.ops
            .get(id)
            .and_then(|stack| stack.last())
            .map(|b| &**b)
    }

    /// The operator `id` would resolve to if the current one were removed —
    /// what a plugin calls to defer to the built-in it wrapped.
    pub fn shadowed(&self, id: &str) -> Option<&dyn Operator> {
        self.ops
            .get(id)
            .filter(|stack| stack.len() >= 2)
            .map(|stack| &*stack[stack.len() - 2])
    }

    /// Invoke `id` if it exists and is enabled.
    ///
    /// Returns `None` when the id is unknown or the operator is disabled — the
    /// caller cannot distinguish, and does not need to: a keybinding for a
    /// misspelled id and one for an inapplicable operator should both do
    /// nothing quietly.
    pub fn invoke(&self, id: &str, state: &mut AppState) -> Option<OpResult> {
        let op = self.get(id)?;
        if !op.enabled(state) {
            return None;
        }
        Some(op.invoke(state))
    }

    /// Every registered id, sorted. For the keymap editor and diagnostics.
    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.ops.keys().copied()
    }

    /// How many operators are registered under `id`, shadowed ones included.
    pub fn depth(&self, id: &str) -> usize {
        self.ops.get(id).map_or(0, |stack| stack.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        id: &'static str,
        label: &'static str,
        enabled: bool,
    }

    impl Stub {
        fn new(id: &'static str) -> Self {
            Self {
                id,
                label: "Stub",
                enabled: true,
            }
        }
    }

    impl Operator for Stub {
        fn id(&self) -> &'static str {
            self.id
        }
        fn label(&self) -> &str {
            self.label
        }
        fn enabled(&self, _state: &AppState) -> bool {
            self.enabled
        }
        fn invoke(&self, state: &mut AppState) -> OpResult {
            state.session.set_status(self.label);
            OpResult::done()
        }
    }

    #[test]
    fn an_unknown_id_resolves_to_nothing() {
        let registry = Registry::new();
        assert!(registry.get("no.such.op").is_none());
    }

    #[test]
    fn registering_twice_shadows_rather_than_replaces() {
        let mut registry = Registry::new();
        registry.register(Box::new(Stub {
            label: "first",
            ..Stub::new("test.op")
        }));
        registry.register(Box::new(Stub {
            label: "second",
            ..Stub::new("test.op")
        }));

        assert_eq!(registry.depth("test.op"), 2, "the first is still there");
        assert_eq!(registry.get("test.op").unwrap().label(), "second");
        assert_eq!(
            registry.shadowed("test.op").unwrap().label(),
            "first",
            "a shadowing operator can still reach the one it covered"
        );
    }

    #[test]
    fn the_only_registration_shadows_nothing() {
        let mut registry = Registry::new();
        registry.register(Box::new(Stub::new("test.op")));
        assert!(registry.shadowed("test.op").is_none());
    }

    #[test]
    fn a_disabled_operator_is_not_invoked() {
        // Not merely "invoke returns None": the operator's body must not run.
        // `Stub::invoke` sets the status line, so an empty status proves it was
        // never entered — a test that only checked the return value would pass
        // with the `enabled` guard deleted.
        let mut registry = Registry::new();
        registry.register(Box::new(Stub {
            enabled: false,
            ..Stub::new("test.op")
        }));

        let mut state = AppState::default();
        state.session.status = None;
        assert!(registry.invoke("test.op", &mut state).is_none());
        assert_eq!(state.session.status, None, "invoke body never ran");
    }

    #[test]
    fn an_enabled_operator_runs() {
        let mut registry = Registry::new();
        registry.register(Box::new(Stub::new("test.op")));

        let mut state = AppState::default();
        state.session.status = None;
        assert!(registry.invoke("test.op", &mut state).is_some());
        assert_eq!(state.session.status.as_deref(), Some("Stub"));
    }

    #[test]
    fn builtin_ids_are_unique() {
        // Two built-ins sharing an id would silently shadow one another, and the
        // loser would be unreachable from every menu and keybinding.
        let registry = Registry::with_builtins();
        let clashes: Vec<_> = registry.ids().filter(|id| registry.depth(id) > 1).collect();
        assert!(clashes.is_empty(), "duplicate built-in ids: {clashes:?}");
    }
}
