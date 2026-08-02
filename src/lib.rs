//! Export pipeline (Phase 4): PNG sequence, video, runtime formats.
//!
//! Project serialization lives in `ankhimate-formats`, not here — the bincode
//! `ProjectData` path was removed under ADR 0004 / T-108 in favor of the `.ankh`
//! zip container. This crate is reserved for render-to-media exporters, which
//! land in Phase 4.
