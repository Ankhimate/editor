# ADR 0006 — Setup mode and Animate mode

**Status:** accepted · **Task:** T-207 · **Refs:** [PLAN §2.6](../ARCHITECTURE_PLAN.md), §3.2, §7

## Context

Before T-207 the editor had one editing context and a boolean, `Session.auto_key`,
that decided whether a pose became setup data or a key. The test
`auto_key && active_animation.is_some() && playhead > 0.0` was repeated in four
places and the copies disagreed: the pose commit refused to key at `t = 0` (so
posing on frame 0 silently overwrote the setup pose), while the draw-order panel
seeded a `t = 0` baseline and keyed anyway. The viewport gave no signal about
which of the two a drag would do, and nothing stopped a structural edit — create,
delete, reparent — from landing while an animation was displayed, leaving the rig
changed under keys that referenced the old structure.

Spine and Spriter both solve this with an explicit mode. It is not chrome: it is
the rule that makes every gesture unambiguous.

## Decision

`Session.work_mode: WorkMode { Setup, Animate }` is the editor's primary state.

1. **Setup** evaluates `evaluate(skeleton, &[], …)` — no animation applied, at any
   playhead. Edits mutate setup data. Structural edits are allowed.
2. **Animate** evaluates the active animation at the playhead. The same edits
   become keys at that time. Structural edits are refused.
3. All edits enter through `editor/src/edit_router.rs`: a panel states an
   `EditIntent` and the router returns `Routed::Commands`, `Routed::Pending`
   (Animate + auto-key off — hold as a preview until `K`), or `Routed::Refused`.
   Panels never choose between a setup command and a key command.
4. `EditCommand::requires_mode() -> Option<WorkMode>` makes the structural rule a
   property of the command. `History::push_in_mode` refuses a mismatch without
   touching the document, so the invariant is unit-testable rather than a
   convention the next panel can forget.
5. Entering Animate guarantees a clip (select the first, or create one). A mode
   in which every edit vanishes is worse than a mode that made a decision for you.
6. `Tool` (Select / Create Bone / Weight Paint) is renamed out of `EditorMode` and
   is orthogonal: a tool says how you point, the mode says what the pointing
   writes to. Setup-only tools reset to Select on entering Animate.

## Consequences

- Keying at `t = 0` now works: Animate mode always keys, and the `t = 0` baseline
  is only inserted when the edit is at `t > 0` on a fresh timeline.
- `Session.auto_key` survives as an Animate-mode sub-toggle, default on. Off means
  "let me look before I commit": the pose shows as a preview, the viewport says
  `unkeyed pose — press K`, and moving the playhead discards it.
- Undo is unaffected — `revert` never consults the mode, so history recorded in
  one mode unwinds correctly in the other.
- Every future editing surface (mesh topology vs. deform keys, T-401/T-404;
  constraint setup vs. mix keys, T-501…T-504) declares its behavior by adding an
  `EditIntent` variant and, if structural, a `requires_mode`.

## Alternatives considered

- **Keep auto-key only.** Rejected: it cannot express "structural edits are
  illegal right now", and it leaves the viewport ambiguous.
- **Mode as a property of the Document.** Rejected: it is what the *user* is
  doing, not what the project *is*, so it belongs in `Session` (PLAN §3.2) and
  must not be saved or undone.
- **Separate Setup and Animate windows.** Rejected: the rig and the animation are
  the same object; two views of it would double the state and the bugs.
