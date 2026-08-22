//! One open rig, and the rules about writing it.
//!
//! # Never in place
//!
//! `docs/plugin-plan.md` states the rule and this is where it is kept: a caller
//! **names every output path**, and saving over the file that was opened is
//! refused.
//!
//! The reason is not caution for its own sake. There is no undo here and no
//! editor to inspect the damage in, so a model that mangles a rig has mangled
//! the file — and the artist whose morning it was has no version of it left.
//! Export already follows this rule for the same reason; so does this.
//!
//! # Why a session and not a stateless server
//!
//! A rig is megabytes of images and a model works on one across many calls.
//! Re-reading it per tool call would be slow and, worse, would silently discard
//! everything the previous call did — so the document is held open between
//! calls, exactly as the editor holds one.

use ankhimate_document::{Document, Edit};
use std::path::{Path, PathBuf};

/// What went wrong.
#[derive(Debug)]
pub enum Error {
    /// No rig is open and the tool needs one.
    NothingOpen,
    /// The file could not be read or written.
    Io(String),
    /// The rig could not be parsed.
    Format(String),
    /// A script failed.
    Script(String),
    /// An export could not be planned or written.
    Export(String),
    /// A headless preview could not be rendered.
    Render(String),
    /// The caller asked for something the rules refuse.
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NothingOpen => write!(f, "no rig is open — call `open_rig` or `new_rig` first"),
            Error::Io(why) => write!(f, "{why}"),
            Error::Format(why) => write!(f, "{why}"),
            Error::Script(why) => write!(f, "{why}"),
            Error::Export(why) | Error::Render(why) => write!(f, "{why}"),
            Error::Refused(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The rig a model is working on.
#[derive(Default)]
pub struct Session {
    open: Option<Open>,
}

struct Open {
    doc: Document,
    /// Where it was read from, or `None` for a rig built from nothing.
    ///
    /// Kept so saving over the source can be refused. A model that overwrote
    /// the file it opened would have destroyed the only copy.
    source: Option<PathBuf>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is a rig open?
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The open document, or an error naming what to do about it.
    pub fn doc(&self) -> Result<&Document, Error> {
        self.open.as_ref().map(|o| &o.doc).ok_or(Error::NothingOpen)
    }

    /// Where the open rig came from.
    pub fn source(&self) -> Option<&Path> {
        self.open.as_ref().and_then(|o| o.source.as_deref())
    }

    /// Start from nothing.
    pub fn new_rig(&mut self, name: &str) {
        let mut doc = Document::new();
        doc.meta.name = name.to_string();
        self.open = Some(Open { doc, source: None });
    }

    /// Read a rig, replacing whatever was open.
    ///
    /// Any importer will do — `.ankh`, Spine, DragonBones, PSD — because the
    /// registry answers by extension and a model naming a file should not have
    /// to know which reader we happen to have.
    pub fn open_rig(&mut self, path: &Path) -> Result<(), Error> {
        let importers = ankhimate_formats::Importers::builtin();
        let (_, loaded) = importers
            .read_any(path)
            .map_err(|e| Error::Format(format!("{}: {e}", path.display())))?;

        self.open = Some(Open {
            doc: Document {
                skeleton: loaded.skeleton,
                animations: loaded.animations,
                assets: loaded.assets,
                meta: ankhimate_document::doc::DocumentMeta {
                    name: loaded.name,
                    fps: loaded.fps,
                },
                export_presets: Vec::new(),
                psd_layer_paths: loaded.psd_layer_paths,
            },
            source: Some(path.to_path_buf()),
        });
        Ok(())
    }

    /// Write the open rig to `path`.
    ///
    /// **Refuses to write over the file it was opened from.** There is no undo
    /// here and no editor to inspect the damage in, so a model that mangled a
    /// rig must have mangled a new file rather than the artist's.
    pub fn save_rig(&self, path: &Path) -> Result<(), Error> {
        let open = self.open.as_ref().ok_or(Error::NothingOpen)?;

        if let Some(source) = &open.source
            && same_file(source, path)
        {
            return Err(Error::Refused(format!(
                "`{}` is the file this rig was opened from. Name a different \
                 destination — there is no undo here, so a mistake would leave \
                 nothing to go back to.",
                path.display()
            )));
        }

        // `save` builds the image blobs from the asset database itself, so
        // there is nothing extra to hand it — the argument is for images a
        // caller has that the document does not.
        let project = open.doc.as_project_ref();
        ankhimate_formats::save(path, &project, &[])
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
    }

    /// Export the open rig with a saved or built-in template preset.
    pub fn export_rig(
        &self,
        output_dir: &Path,
        preset_name: Option<&str>,
    ) -> Result<ankhimate_export::run::Survey, Error> {
        let doc = self.doc()?;
        let mut presets = doc.presets();
        presets.extend(ankhimate_export::presets::builtin());

        let preset = match preset_name {
            Some(name) => presets
                .iter()
                .find(|preset| preset.name == name)
                .ok_or_else(|| {
                    Error::Refused(format!(
                        "no export preset named `{name}`. Available: {}",
                        presets
                            .iter()
                            .map(|preset| preset.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?,
            None => presets
                .iter()
                .find(|preset| preset.name == "Ankhimate runtime")
                .or_else(|| presets.first())
                .ok_or_else(|| Error::Refused("no export presets are available".into()))?,
        };

        let project = ankhimate_formats::convert::to_schema(&doc.as_project_ref());
        let plan = ankhimate_export::run::plan(&project, &doc.assets, preset)
            .map_err(|error| Error::Export(error.to_string()))?;
        let survey = plan.survey(output_dir);
        ankhimate_export::run::write(&plan, output_dir)
            .map_err(|error| Error::Export(error.to_string()))?;
        Ok(survey)
    }

    /// Run a script against the open rig, over the full verb surface.
    ///
    /// The escape hatch the coarse tool list is built around: a model writing
    /// twenty lines of JavaScript beats twenty round trips, and the plugin host
    /// already sandboxes it — no filesystem, no network, no clock.
    ///
    /// Returns whatever the script logged.
    pub fn run_script(&mut self, script: &str) -> Result<Vec<String>, Error> {
        let open = self.open.as_mut().ok_or(Error::NothingOpen)?;

        // Moved in and moved back, the same way every other host of this API
        // does it: `Document` is not `Clone`, and the document must come back
        // whether the script succeeded or threw.
        let mut edit = Edit::new(std::mem::take(&mut open.doc));
        let outcome = ankhimate_plugins::Host::new().run(script, &mut edit);
        open.doc = edit.doc;

        outcome.map_err(|e| Error::Script(e.to_string()))
    }

    /// A short description of what is open, for a tool result.
    pub fn summary(&self) -> Result<String, Error> {
        let doc = self.doc()?;
        Ok(format!(
            "`{}` — {} bones, {} slots, {} animations, {} images, {} fps",
            doc.meta.name,
            doc.skeleton.bones.len(),
            doc.skeleton.slots.len(),
            doc.animations.len(),
            doc.assets.images.len(),
            doc.meta.fps,
        ))
    }
}

/// Are these two paths the same file?
///
/// Compared after canonicalisation where possible, so `./rig.ankh` and
/// `/home/a/rig.ankh` are recognised as one — a rule that only catches the
/// literal same string is a rule a model steps around by accident.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        // The destination not existing is the common case and means they differ.
        // Falling back to the raw comparison still catches the obvious one.
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_rig() -> Session {
        let mut session = Session::new();
        session.new_rig("hero");
        session
            .run_script(r#"ops.invoke("bone.create", { name: "root" });"#)
            .expect("the script runs");
        session
    }

    #[test]
    fn a_tool_with_nothing_open_says_what_to_do_about_it() {
        // "No document" leaves a model guessing. Naming the tool that fixes it
        // is the difference between one wasted call and several.
        let session = Session::new();
        let message = session.summary().unwrap_err().to_string();
        assert!(message.contains("open_rig"), "{message}");
    }

    #[test]
    fn a_script_edits_the_open_rig() {
        let session = a_rig();
        assert_eq!(session.doc().unwrap().skeleton.bones.len(), 1);
    }

    #[test]
    fn the_rig_survives_a_script_that_throws() {
        // The document is moved into the host and must come back either way. A
        // model with a typo in its script must not lose the rig it was working
        // on.
        let mut session = a_rig();
        let error = session
            .run_script("throw new Error('oops');")
            .expect_err("the script threw");
        assert!(error.to_string().contains("oops"));
        assert_eq!(
            session.doc().unwrap().skeleton.bones.len(),
            1,
            "the rig came back whole"
        );
    }

    #[test]
    fn edits_accumulate_across_calls() {
        // The reason a session exists rather than a stateless server: a model
        // works on one rig across many calls, and re-reading per call would
        // discard everything the last one did.
        let mut session = a_rig();
        session
            .run_script(r#"ops.invoke("bone.create", { name: "spine", parent: "root" });"#)
            .expect("the second script runs");
        assert_eq!(session.doc().unwrap().skeleton.bones.len(), 2);
    }

    #[test]
    fn saving_over_the_source_is_refused_with_the_reason() {
        // The rule this whole module exists for. There is no undo here and no
        // editor to inspect the damage in.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.ankh");

        let session = a_rig();
        session.save_rig(&path).expect("a fresh path is fine");

        let mut opened = Session::new();
        opened.open_rig(&path).expect("it reads back");
        let error = opened.save_rig(&path).unwrap_err().to_string();
        assert!(
            error.contains("opened from") && error.contains("no undo"),
            "the reason has to say why, not just no: {error}"
        );
    }

    #[test]
    fn the_same_file_under_another_spelling_is_still_the_same_file() {
        // A rule that only catches the literal same string is one a model steps
        // around by accident.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.ankh");
        a_rig().save_rig(&path).expect("written");

        let mut opened = Session::new();
        opened.open_rig(&path).expect("it reads back");

        let roundabout = dir.path().join(".").join("hero.ankh");
        assert!(
            opened.save_rig(&roundabout).is_err(),
            "`./hero.ankh` is the same file as `hero.ankh`"
        );
    }

    #[test]
    fn a_rig_built_from_nothing_can_be_saved_anywhere() {
        // The refusal is about not destroying a source, and a rig with no
        // source has none to destroy.
        let dir = tempfile::tempdir().unwrap();
        let session = a_rig();
        assert!(session.source().is_none());
        assert!(session.save_rig(&dir.path().join("out.ankh")).is_ok());
    }

    #[test]
    fn a_saved_rig_reads_back_with_what_was_built() {
        // The round trip, because a save that writes an empty file is a save
        // that looks like it worked.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hero.ankh");
        a_rig().save_rig(&path).expect("written");

        let mut opened = Session::new();
        opened.open_rig(&path).expect("read back");
        assert_eq!(opened.doc().unwrap().skeleton.bones.len(), 1);
        assert!(opened.summary().unwrap().contains("1 bones"));
    }
}
