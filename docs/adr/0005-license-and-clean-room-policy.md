# ADR 005: Dual MIT/Apache-2.0 license and clean-room policy

- **Status:** Accepted
- **Date:** 2026-07-30
- **Decision owner:** Ankhimate core team
- **Records:** PLAN §0, §3.3

## Context

The established tools in this space are either proprietary or **GPL-3.0**. If
Ankhimate incorporated GPL-derived code it would itself have to be GPL-3.0 —
which would prevent embedding the runtime in games that are not GPL-compatible.
That undercuts a key competitive advantage: letting any game integrate
`ankhimate-runtime` freely.

We therefore need both a license choice and a contribution policy that keeps the
codebase free of copyleft-derived code.

## Decision

1. **License:** `ankhimate-core`, `ankhimate-runtime`, and the editor are all
   dual-licensed under **MIT OR Apache-2.0**. Games can pick either. This is a
   deliberate advantage over GPL-only alternatives.
2. **Clean-room policy:** Ankhimate replicates the *feature set* of existing
   animation editors but must not copy, translate, or closely paraphrase source
   code from any of them. Contributors describe *behavior to implement*, never
   *code to port*. If a foreign file format ever has to be read, it is
   reverse-engineered from sample **data files** and the findings written up as a
   document first; that document, not the source of the tool that wrote the
   file, is what an implementation may follow.
3. **FFmpeg** (used later for video export) is shelled out to an external
   binary — never statically linked — to avoid (L)GPL linkage questions.

## Alternatives considered

- **GPL-3.0 for everything:** maximally free, but blocks the "embed the runtime
  in any game" promise. Rejected.
- **GPL for the editor, MIT/Apache for core/runtime:** a coherent split, but
  adds license-complexity and offers no real benefit for a desktop tool.

## Consequences

- `LICENSE-MIT` + `LICENSE-APACHE` at the repo root; `license = "MIT OR Apache-2.0"`
  in every `Cargo.toml`.
- `CONTRIBUTING.md` documents the clean-room rule; a contributor who has read a
  GPL editor's source must not work on the matching Ankhimate feature.
- `cargo-deny` (deny.toml) restricts dependency licenses to permissive sets so
  the runtime embedding promise holds transitively.
- Third-party sample rigs carry licenses of their own and are **not** committed;
  tests that would use them skip when they are absent.
