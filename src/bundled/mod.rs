//! Import plugins shipped with Ankhimate.
//!
//! Foreign formats live here rather than in `ankhimate-formats`: the latter
//! owns the native `.ankh` contract and the importer interface, while this
//! crate owns optional extensions of that interface.

#[cfg(feature = "import-dragonbones")]
pub mod dragonbones;
#[cfg(feature = "import-spine")]
pub mod spine;

/// Register every foreign-format importer shipped with the application.
///
/// Kept separate from `ankhimate_formats::Importers::builtin()` so a headless
/// consumer that only wants native formats does not acquire external readers
/// implicitly. The editor and MCP opt in at their composition roots.
pub fn register_importers(_importers: &mut ankhimate_formats::Importers) {
    #[cfg(feature = "import-spine")]
    _importers.register(Box::new(spine::SpineImporter));
    #[cfg(feature = "import-dragonbones")]
    _importers.register(Box::new(dragonbones::DragonBonesImporter));
}
