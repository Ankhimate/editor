//! Export pipeline (Phase 6, T-603).
//!
//! Export is **user-authored**. Ankhimate does not know which engine a rig is
//! headed for, and the list of engines is not closeable, so the deliverable is
//! not a set of exporters but an engine for writing them: a text template over a
//! documented context, plus a baked atlas the template can reference.
//!
//! The crate is headless — no egui, no wgpu — which is what lets it run in CI
//! and, later, in a CLI exporter with no display.
//!
//! See `docs/export-plan.md` for the reasoning, and `docs/export-context.md` for
//! the field-by-field contract a template author works against.

pub mod atlas;
pub mod context;
pub mod preset;
pub mod presets;
pub mod run;
pub mod template;
