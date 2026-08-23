//! Setup vs Animate (T-207, ADR 0006).
//!
//! In the document crate rather than the editor's session because every
//! command declares `requires_mode` with it, and commands have to compile
//! without a UI. The mode itself is interaction state — which is why the
//! editor owns the *value* while this owns the *type*.

/// Which half of the editor's two modes is active.
///
/// The same gesture means different things in each mode, and this is the single
/// switch that decides which:
///
/// * **Setup** — define the rig. Edits mutate the [`Skeleton`] setup data, the
///   viewport always shows the setup pose (no animation applied, whatever the
///   playhead says), and structural edits are allowed.
/// * **Animate** — animate the rig. The same edits become keys on
///   the editor session's active animation at its playhead; the setup data is
///   read-only and structural commands are refused.
///
/// [`Skeleton`]: ankhimate_core::skeleton::Skeleton
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkMode {
    #[default]
    Setup,
    Animate,
}

impl WorkMode {
    pub fn label(self) -> &'static str {
        match self {
            WorkMode::Setup => "SETUP",
            WorkMode::Animate => "ANIMATE",
        }
    }
}
