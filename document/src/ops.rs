//! Document verbs: named, argument-taking, and reachable without an editor.
//!
//! The editor's `Operator` is the same idea over `AppState` — it can read the
//! selection, switch tools and open windows, none of which exist here. This is
//! the half a script, a plugin or an MCP client can reach: verbs whose whole
//! input is their arguments and whose whole effect is an undoable edit.
//!
//! # Why two traits rather than one with a mode flag
//!
//! A single trait would have to hand every operator an `AppState`, which is
//! what forces the editor into the dependency graph of anything that wants to
//! run a verb. Splitting them means a document verb *cannot* reach the session
//! by construction, and the compiler says so rather than a convention.
//!
//! The editor adopts these into its own registry (see `editor/src/operators.rs`),
//! so a menu, a keybinding and a plugin all still resolve one id to one verb.

use crate::args::{ArgError, Args};
use crate::edit::Edit;
use crate::work_mode::WorkMode;

/// A named edit that needs nothing but its arguments.
pub trait DocOperator: Send + Sync {
    /// Stable dotted id — `"bone.create"`. A public contract in the same sense
    /// `docs/export-context.md` is: a user's keymap and a plugin's `shadow` both
    /// name it, and neither has a compiler.
    fn id(&self) -> &'static str;

    fn label(&self) -> &str;

    /// The mode this verb runs in, or `None` for both (T-207).
    ///
    /// Advisory here, enforced by the command — see [`Edit::dispatch`].
    /// Declaring it lets a caller ask before committing rather than discovering
    /// the refusal afterwards.
    fn requires_mode(&self) -> Option<WorkMode> {
        None
    }

    /// What arguments this takes, as JSON Schema.
    ///
    /// For description, not validation: `invoke` reports what is missing with
    /// better context than a schema walk, and an MCP client lists tools from
    /// this.
    fn schema(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Do the thing.
    fn invoke(&self, edit: &mut Edit, args: &Args) -> Result<(), OpError>;
}

/// Why a document verb did not run.
#[derive(Debug, Clone, PartialEq)]
pub enum OpError {
    /// The arguments could not be read — missing, wrong type, or naming
    /// something the rig does not have.
    Args(ArgError),
    /// The edit was refused by the mode rule.
    Refused(crate::edit::Refused),
    /// No verb answers to that id.
    Unknown(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Args(e) => write!(f, "{e}"),
            OpError::Refused(e) => write!(f, "{e}"),
            OpError::Unknown(id) => write!(f, "no such operator: `{id}`"),
        }
    }
}

impl std::error::Error for OpError {}

impl From<ArgError> for OpError {
    fn from(e: ArgError) -> Self {
        OpError::Args(e)
    }
}

impl From<crate::edit::Refused> for OpError {
    fn from(e: crate::edit::Refused) -> Self {
        OpError::Refused(e)
    }
}

/// Document verbs by id.
///
/// Deliberately not the editor's `Registry`: this one has no shadowing, because
/// shadowing is a plugin concern and the plugin host lives above both. The
/// editor's registry adopts these and provides the chain.
#[derive(Default)]
pub struct DocOps {
    ops: std::collections::BTreeMap<&'static str, Box<dyn DocOperator>>,
}

impl DocOps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every built-in document verb.
    pub fn builtin() -> Self {
        let mut ops = Self::new();
        crate::doc_ops::register(&mut ops);
        crate::import_ops::register(&mut ops);
        crate::constraint_ops::register(&mut ops);
        crate::rig_ops::register(&mut ops);
        crate::part_ops::register(&mut ops);
        ops
    }

    pub fn register(&mut self, op: Box<dyn DocOperator>) {
        self.ops.insert(op.id(), op);
    }

    pub fn get(&self, id: &str) -> Option<&dyn DocOperator> {
        self.ops.get(id).map(|b| &**b)
    }

    pub fn ids(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.ops.keys().copied()
    }

    /// Hand the verbs over, for a host that keeps its own registry.
    ///
    /// The editor adopts these into a registry that also holds session verbs and
    /// supports plugin shadowing; taking them by value avoids a second
    /// construction path that could register a different set.
    pub fn into_ops(self) -> impl Iterator<Item = Box<dyn DocOperator>> {
        self.ops.into_values()
    }

    /// Run `id` against `edit`.
    pub fn invoke(&self, id: &str, edit: &mut Edit, args: &Args) -> Result<(), OpError> {
        let op = self
            .get(id)
            .ok_or_else(|| OpError::Unknown(id.to_string()))?;
        op.invoke(edit, args)
    }
}
