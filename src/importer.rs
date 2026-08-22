//! Importers by name, so the list is not closeable.
//!
//! Ankhimate cannot know which format a rig arrives in and the list does not
//! end — that is the same argument `docs/export-plan.md` makes for export being
//! a format editor. This is the import half: a registry a built-in and a plugin
//! register into through the same door.
//!
//! # Extracted from two, not guessed from one
//!
//! The Spine and DragonBones readers were written months apart and converged
//! independently on the same signature: `read(json, images, name) -> Loaded`,
//! the same three-way [`Images`], the same `declared_version`. A trait fitted to
//! one of them would have encoded that one's accidents; this one describes what
//! both already did.
//!
//! What they *disagree* on is the interesting part, and it is why this trait
//! owns file discovery rather than only parsing. Spine scans a directory for
//! `*.atlas`; DragonBones pairs `walk_ske.json` with `walk_tex.json`, and
//! cannot scan for an extension because both of its files are `.json`. An
//! importer knows how its own format is laid out on disk, and nothing above it
//! does.

use crate::convert::Loaded;
use std::path::Path;

/// The application's own portable project container.
///
/// Native projects still go through the registry: callers that accept any rig
/// format should not need a special branch for the format Ankhimate writes.
pub struct AnkhImporter;

impl Importer for AnkhImporter {
    fn id(&self) -> &str {
        "import.ankh"
    }

    fn label(&self) -> &str {
        "Ankhimate"
    }

    fn extensions(&self) -> Vec<&str> {
        vec!["ankh"]
    }

    fn read(&self, path: &Path) -> Result<Loaded, ImportError> {
        crate::load(path)
            .map(|(loaded, _)| loaded)
            .map_err(|error| match error {
                crate::Error::Container(crate::container::ContainerError::Io(error)) => {
                    ImportError::Io(error.to_string())
                }
                other => ImportError::Malformed(other.to_string()),
            })
    }
}

/// The images an import draws attachments from.
///
/// Both shipped readers need exactly these three cases: a packed atlas, loose
/// files, or nothing. A rig exported for a runtime has an atlas; one exported
/// for re-editing usually does not; and geometry still imports when neither is
/// there, with every attachment named in the report.
pub enum Images<'a> {
    /// A parsed atlas description and a way to open its pages.
    Atlas {
        text: &'a str,
        pages: &'a dyn Fn(&str) -> Option<image::RgbaImage>,
    },
    /// Loose images, looked up by the name an attachment uses.
    Loose(&'a dyn Fn(&str) -> Option<image::RgbaImage>),
    /// None available. The rig still imports; textures land in the report.
    None,
}

/// What went wrong badly enough that there is no rig.
#[derive(Debug)]
pub enum ImportError {
    /// The file could not be read.
    Io(String),
    /// It parsed, but is not this format.
    ///
    /// Distinct from a parse failure because it is what a caller trying several
    /// importers in turn needs to hear: "not mine, try the next one".
    NotThisFormat,
    /// It is this format, and it is broken.
    Malformed(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Io(why) => write!(f, "{why}"),
            ImportError::NotThisFormat => write!(f, "this file is not that format"),
            ImportError::Malformed(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// One rig format that can be read in.
pub trait Importer: Send + Sync {
    /// Stable dotted id — `"import.spine"`. What a menu entry, a plugin and an
    /// MCP tool all name, so it is a public contract like an operator id.
    /// The dotted id a caller names this importer by.
    ///
    /// `&str`, not `&'static str`: a plugin's importer reads its id out of its
    /// own file at load time, and a vocabulary that only built-ins can join is
    /// the closed list this registry exists to replace.
    fn id(&self) -> &str;

    /// Human-readable, for the File▸Import menu.
    fn label(&self) -> &str;

    /// Extensions a file dialog should offer, without the dot.
    fn extensions(&self) -> Vec<&str>;

    /// Could this importer plausibly read `path`?
    ///
    /// Cheap and allowed to be wrong in the permissive direction — [`read`] is
    /// what actually decides, and returns [`ImportError::NotThisFormat`] when it
    /// disagrees. This exists so a caller with a file and no idea which format
    /// it is can narrow the candidates rather than parse it six times.
    ///
    /// [`read`]: Self::read
    fn claims(&self, path: &Path) -> bool {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        self.extensions()
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext))
    }

    /// What options this importer accepts, as JSON Schema.
    ///
    /// Most take none. The ones that do — PSD's layer selection and scale, a
    /// sprite sheet's grid — are *parameterised*, not conversational: every
    /// option has a sensible default, so an unattended caller gets a usable
    /// result and the editor's panel refines rather than supplies.
    ///
    /// That distinction is why these fit the registry at all. An importer that
    /// genuinely needed a conversation could not be reached from a script, and
    /// would have to stay panel-only.
    fn options_schema(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Read `path` with options, when the caller has any.
    ///
    /// Defaults to ignoring them, so an importer with no options implements one
    /// method rather than two that must agree.
    fn read_with(&self, path: &Path, _options: &serde_json::Value) -> Result<Loaded, ImportError> {
        self.read(path)
    }

    /// Read `path` and whatever sits beside it.
    ///
    /// The importer finds its own sidecars: only it knows that a Spine rig keeps
    /// its atlas in any `*.atlas` in the directory while a DragonBones rig pairs
    /// `<stem>_ske.json` with `<stem>_tex.json`. Hoisting that into the caller
    /// is what makes a registry impossible, because the caller would need a
    /// branch per format — which is the thing being removed.
    fn read(&self, path: &Path) -> Result<Loaded, ImportError>;

    /// The version the file declares, when it has one. For reporting only.
    fn declared_version(&self, _path: &Path) -> Option<String> {
        None
    }
}

/// Importers by id.
///
/// No shadowing here, unlike the editor's operator registry: shadowing is a
/// plugin concern and the plugin host sits above this. What this provides is the
/// lookup, so a menu, a file dialog and an MCP tool all resolve one id to one
/// reader.
#[derive(Default)]
pub struct Importers {
    /// Keyed by an owned id, because a plugin's importer names itself out of
    /// its own file and cannot hand out a `&'static str`.
    by_id: std::collections::BTreeMap<String, Box<dyn Importer>>,
}

impl Importers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every built-in importer, registered through the same door a plugin uses.
    pub fn builtin() -> Self {
        let mut importers = Self::new();
        importers.register(Box::new(AnkhImporter));
        importers.register(Box::new(crate::psd::PsdImporter));
        importers
    }

    pub fn register(&mut self, importer: Box<dyn Importer>) {
        self.by_id.insert(importer.id().to_string(), importer);
    }

    pub fn get(&self, id: &str) -> Option<&dyn Importer> {
        self.by_id.get(id).map(|b| &**b)
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> + '_ {
        self.by_id.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Importer> + '_ {
        self.by_id.values().map(|b| &**b)
    }

    /// Importers that might read `path`, in registration order.
    ///
    /// Several may claim one file — both shipped readers accept `.json` — so
    /// this narrows rather than decides. A caller tries them until one does not
    /// answer [`ImportError::NotThisFormat`].
    pub fn claiming<'a>(&'a self, path: &'a Path) -> impl Iterator<Item = &'a dyn Importer> + 'a {
        self.iter().filter(move |i| i.claims(path))
    }

    /// Read `path` with whichever importer accepts it.
    ///
    /// Returns the importer's id alongside the rig, since a caller that guessed
    /// wants to know what it actually got.
    pub fn read_any(&self, path: &Path) -> Result<(String, Loaded), ImportError> {
        let mut last = None;
        for importer in self.claiming(path) {
            match importer.read(path) {
                Ok(loaded) => return Ok((importer.id().to_string(), loaded)),
                // Not this one — keep looking. A malformed file of a format that
                // *did* claim it is worth remembering, though: reporting "no
                // importer accepted this" for a Spine rig with a typo in it
                // would send the user hunting the wrong problem.
                Err(ImportError::NotThisFormat) => {}
                Err(other) => last = Some(other),
            }
        }
        Err(last.unwrap_or(ImportError::NotThisFormat))
    }
}

#[cfg(test)]
mod tests {
    use super::Importers;

    #[test]
    fn external_json_formats_are_not_built_in() {
        let importers = Importers::builtin();
        assert!(importers.get("import.ankh").is_some());
        assert!(importers.get("import.psd").is_some());
        assert!(importers.get("import.spine").is_none());
        assert!(importers.get("import.dragonbones").is_none());
    }
}
