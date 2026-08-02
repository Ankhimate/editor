# Contributing to Ankhimate

Thanks for your interest! This file is a quick orientation; the normative
references are [`docs/ARCHITECTURE_PLAN.md`](docs/ARCHITECTURE_PLAN.md) and
[`docs/TASKS.md`](docs/TASKS.md).

## Clean-room policy (read first)

Ankhimate is licensed **MIT OR Apache-2.0** so that anyone can embed the runtime
in any game. Keeping that promise means the codebase must stay free of code
derived from copyleft-licensed projects.

**Do not copy, translate, or closely paraphrase source code from any other
animation editor.** Implement observed *behavior* — what a feature does, how it
should feel — never ported code. If you have read the source of a GPL-licensed
editor, do not contribute to the matching Ankhimate feature; pick a different
task.

Reading a **file format** is different from reading source: a format is
reverse-engineered from sample data files, and the findings are written up as a
document before any code is written. That document, not the importer, is then the
record of what was observed.

Full policy: [ADR 005](docs/adr/0005-license-and-clean-room-policy.md).

## Getting started

```bash
git clone https://github.com/Ankhimate/editor.git
cd editor
cargo test --workspace
cargo run -p ankhimate-editor
```

Needs a recent stable Rust toolchain (`rust-toolchain.toml` pins it) and a GPU
with Vulkan, Metal, DX12, or GL support.

## Picking something to work on

Work is broken into PR-sized tasks in [`docs/TASKS.md`](docs/TASKS.md), each with
dependencies and acceptance criteria. Tasks marked ∥ can be done in parallel with
their siblings. Comment on the issue (or open one) before starting anything large
so two people do not build the same thing.

Small fixes need no ceremony — open the PR.

## House rules

These are what the review will actually check:

- **`core/` stays framework-free.** No egui, no wgpu, no editor types. It is the
  contract games compile against.
- **World transforms are always computed from local**, never stored.
- **Every user action is a command** with `apply` and `revert`, pushed to the
  history stack. If it changes the document, it must undo.
- **Setup vs Animate**: structural edits are Setup-only and must be refused with
  a hint in Animate, not silently allowed (see
  [ADR 006](docs/adr/0006-work-modes.md)).
- **Comments explain why, not what.** A comment restating the code is noise; a
  comment recording why an obvious approach was rejected is worth its lines.
- **New behavior comes with a test.** Bug fixes come with the test that would
  have caught the bug.

## Before opening a PR

All of these must pass:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Do not break the editor build, even if your task is core-only. If your change is
visual, say what you verified on screen — the render path is not covered by
tests.

Reference the task ID in the PR title where there is one (e.g.
`T-405: clipping attachments`), and describe what you *did not* do as well as
what you did.

## Decisions and ADRs

Significant decisions are recorded as Architecture Decision Records in
[`docs/adr/`](docs/adr/). A change that deviates from the plan needs a new ADR
(`docs/adr/NNN-*.md`) in the same PR — not a follow-up.

## Reporting bugs

Include the OS, GPU/backend, and what you did to trigger it. For rendering bugs
a screenshot is worth more than a paragraph. If a rig is involved and you can
share it, attach the `.ankh`.

## License of contributions

Unless you state otherwise, any contribution you intentionally submit for
inclusion shall be dual-licensed as MIT OR Apache-2.0, with no additional terms.
