//! Importers written in JavaScript.
//!
//! The founding requirement: the list of rig formats does not close, so the
//! deliverable is the engine for writing readers rather than the readers
//! (`docs/plugin-plan.md`). A Rust plugin could already register an
//! [`Importer`](ankhimate_formats::Importer); this is the same door for a `.js`
//! file.
//!
//! # A JS importer builds a rig by calling verbs
//!
//! It does not construct a [`Loaded`](ankhimate_formats::Loaded). That holds
//! `core` types keyed by slotmap ids, which a script has no way to make and no
//! business making — ids are not stable across sessions (ADR 0004), and handing
//! a plugin raw entity keys is the mistake the argument layer exists to avoid.
//!
//! Instead a JS importer is a function from file text to verb calls:
//!
//! ```js
//! ankhimate.registerImporter({
//!   id: "import.mine",
//!   label: "My Format",
//!   extensions: ["mine"],
//!   read(text) {
//!     const rig = JSON.parse(text);
//!     for (const b of rig.bones) {
//!       ops.invoke("bone.create", { name: b.name, parent: b.parent });
//!     }
//!   },
//! });
//! ```
//!
//! That reuses everything: the verbs are the documented ones, the edits are
//! commands, and an import is undoable — which the Rust importers are not,
//! since they replace the document wholesale.
//!
//! # Sidecars, and the sandbox
//!
//! A real importer needs the files beside the one it was given: Spine's
//! `.atlas`, DragonBones' `_tex.json`. So `read` is handed a
//! [`Sidecars`](crate::importer::Sidecars) that can open **only** files in the
//! imported file's own directory.
//!
//! Not a filesystem. `sidecar("../../.ssh/id_rsa")` resolves to nothing, and
//! the test that says so is the one that keeps this honest as bindings are
//! added.

use ankhimate_document::Edit;
use std::path::{Path, PathBuf};

/// The files a JS importer may open: those beside the one being imported.
///
/// Scoped by construction rather than by checking, so there is no path a caller
/// can spell that reaches outside. The directory is fixed at construction and
/// only the file *name* comes from the script.
pub struct Sidecars {
    dir: PathBuf,
}

impl Sidecars {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Read a file beside the imported one, by name.
    ///
    /// A name with a separator or a parent segment in it returns `None` rather
    /// than escaping: the whole point is that a plugin reaches its own sidecars
    /// and nothing else, and rejecting is cheaper to reason about than
    /// canonicalising and comparing.
    pub fn read(&self, name: &str) -> Option<String> {
        if !is_plain_name(name) {
            return None;
        }
        std::fs::read_to_string(self.dir.join(name)).ok()
    }

    /// Read a binary sidecar, base64-encoded for the script.
    ///
    /// An atlas page is a PNG, and a plugin needs the bytes to hand to
    /// `asset.add_image`. Encoded rather than passed as a typed array because
    /// the failure mode of a byte channel is a silently truncated image, and
    /// base64 either decodes or does not.
    pub fn read_bytes(&self, name: &str) -> Option<String> {
        if !is_plain_name(name) {
            return None;
        }
        let bytes = std::fs::read(self.dir.join(name)).ok()?;
        Some(encode_base64(&bytes))
    }

    /// The directory, for a binding that needs its own copy.
    pub(crate) fn clone_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    /// The names sitting beside the imported file.
    ///
    /// So an importer can find `whatever_tex.json` without being told, the way
    /// the Rust readers do.
    pub fn list(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect()
    }
}

/// Is `name` a bare file name, with no way out of its directory?
///
/// Rejects separators, parent segments, absolute paths and Windows drive
/// prefixes. A plugin naming `sub/thing.png` is refused rather than resolved —
/// no shipped format needs it, and allowing it would mean reasoning about
/// symlinks.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name != ".."
        && name != "."
}

/// A rig format described by a script.
pub struct JsImporter {
    pub id: String,
    pub label: String,
    pub extensions: Vec<String>,
    /// The script that registered it, kept so `read` can run it again.
    ///
    /// Held as source rather than as a compiled function: a QuickJS value
    /// cannot outlive its runtime, and the host deliberately builds a fresh
    /// runtime per run so a plugin cannot hold state across an undo.
    pub source: String,
}

impl JsImporter {
    /// Run this importer over `path`, building the rig by verb calls.
    pub fn read(&self, path: &Path) -> Result<Edit, crate::PluginError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| crate::PluginError::Script(format!("could not read the file: {e}")))?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let host = crate::Host::new().with_sidecars(Sidecars::new(dir));
        let mut edit = Edit::default();

        // The registration runs again here so the same script defines the
        // importer and performs the read; a plugin file is therefore one thing
        // rather than a registration and a separate body to keep in step.
        let script = format!(
            "{}\n__ankhimate_run_import({}, {});",
            self.source,
            serde_json::Value::String(text),
            serde_json::Value::String(file_name),
        );
        host.run(&script, &mut edit)?;
        Ok(edit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sidecar_reads_a_file_beside_the_import() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rig_tex.json"), "{}").unwrap();

        let sidecars = Sidecars::new(dir.path());
        assert_eq!(sidecars.read("rig_tex.json").as_deref(), Some("{}"));
        assert!(sidecars.list().iter().any(|n| n == "rig_tex.json"));
    }

    #[test]
    fn a_sidecar_cannot_climb_out_of_its_directory() {
        // The property that keeps `a_plugin_has_no_filesystem` true once an
        // importer needs to open files at all. Rejected by shape, so there is
        // no path a script can spell that reaches outside.
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("secret.txt");
        std::fs::write(&outside, "not yours").ok();

        let sidecars = Sidecars::new(dir.path());
        for attempt in [
            "../secret.txt",
            "..\\secret.txt",
            "sub/thing.png",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "..",
        ] {
            assert!(
                sidecars.read(attempt).is_none(),
                "`{attempt}` should not resolve"
            );
        }
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn a_missing_sidecar_is_none_rather_than_an_error() {
        // An importer asking whether an atlas is there should be able to ask.
        let dir = tempfile::tempdir().unwrap();
        assert!(Sidecars::new(dir.path()).read("nothing.json").is_none());
    }
}

/// Encode bytes as standard base64.
///
/// Public because base64 is the channel every binary crosses into a script —
/// a host handing a plugin a PSD to read needs the same encoder the sidecar
/// reader uses, and a second one would be a second chance to differ.
///
/// Paired with the decoder in `document/src/import_ops.rs`; both are short
/// enough to write, and a crate for sixty lines of table lookup is a
/// supply-chain surface nobody asked for.
pub fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
