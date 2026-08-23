---
title: Contribute to Ankhimate
description: Set up the workspace, run its quality gates, follow clean-room rules, and update documentation safely.
---

# Contribute to Ankhimate

Ankhimate accepts focused changes under MIT OR Apache-2.0. Before implementation,
read the repository `AGENTS.md`, the acceptance notes in `TASKS.md`, and
[ADR 0005](https://github.com/Ankhimate/editor/blob/main/docs/adr/0005-license-and-clean-room-policy.md).
Implement features from observed behavior and public specifications; never copy,
translate, or closely paraphrase another editor's source.

## Workspace checks

Use the pinned Rust toolchain and keep changes within the crate that owns the behavior.
The separate [Rust API reference](/editor/api/) documents code-level interfaces and
algorithms.

```console
cargo fmt --all --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets
cargo check -p ankhimate-core --target wasm32-unknown-unknown
```

Close the running editor before `cargo build` on Windows; it holds its binary open.
`cargo check` remains usable. Add behavioral tests that would fail if the reported
problem returned. Keep unrelated working-tree changes out of the commit.

## Documentation changes

Use the five status labels consistently and do not present roadmap work as current.
Keep language-neutral user and integration contracts in this site; put Rust items and
implementation algorithms in rustdoc. Update generated references and build the site:

```console
cargo run -p xtask -- docs-sync
cargo run -p xtask -- docs-check
bun install --frozen-lockfile
bun run check
bun run build
```

Screenshots must use shipped samples, the default theme, a fixed window size, no
personal paths, and meaningful alternative text. Prefer durable SVG diagrams for
language-neutral architecture. Public names such as verb IDs, schema fields, export
context fields, and plugin globals are compatibility contracts.
