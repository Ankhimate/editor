//! The Model Context Protocol server for Ankhimate rigs.
//!
//! "Make me an animation without opening the editor." Not a second road: this is
//! another consumer of the plugin API, alongside JavaScript and the editor's own
//! menus. A plugin that registers `import.dragonbones` is reachable from the
//! File menu, from a script, *and* from here, with nobody writing MCP support
//! for it.
//!
//! # Why not forty-nine tools
//!
//! `docs/plugin-plan.md` says it plainly: a faithful mirror of every command is
//! a worse tool surface than a deliberate coarse one. "Move the bone a bit left"
//! is not expressible in a tool call; "mirror the limbs" is.
//!
//! So the tools are task-shaped — open, describe, save, export and render —
//! plus one escape hatch, `run_script`, which takes JavaScript over the whole
//! verb surface. A model writing twenty lines beats twenty round trips, and the
//! plugin host already sandboxes it: no filesystem, no network, no clock.
//!
//! # Never writes in place
//!
//! There is no undo here and no editor to inspect the damage in. See
//! [`session`], which is where that rule is kept.

pub mod server;
pub mod session;
pub mod tools;
