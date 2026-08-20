//! Loading plugins, and what the editor does with them.
//!
//! Until this existed nothing in the workspace depended on `ankhimate-plugins`:
//! importers, exporters, the PSD read surface, forty-nine verbs and the panel
//! vocabulary were all tested and none of them reachable from the running app.
//!
//! # Where plugins live
//!
//! `<config>/plugins/*.js`, beside `config.json`. One file per plugin, read at
//! startup. A directory rather than a manifest because a plugin is one file and
//! a manifest would be a second thing to keep in step with it; a flat listing
//! rather than a recursive walk because a plugin that needs a folder of its own
//! needs more than this loader gives it.
//!
//! # When they are read
//!
//! Once, at startup, and again only when the user asks. A plugin file changing
//! under a running editor is a thing to opt into rather than a surprise — a
//! reload discards whatever a panel was showing, and doing that because an
//! editor was left open while a file was saved is worse than a menu item.
//!
//! # What failure looks like
//!
//! A plugin that throws on load is **listed with its error**, not dropped. A
//! plugin silently missing is somebody's afternoon; the same plugin listed as
//! "failed: unexpected token at line 4" is a fix.

use ankhimate_plugins::Host;
use ankhimate_plugins::panel::PanelSpec;
use std::path::{Path, PathBuf};

/// One plugin file, and what it declared.
pub struct Plugin {
    /// The file it was read from.
    pub path: PathBuf,
    /// Its file stem, for the UI.
    pub name: String,
    /// The script, kept so its importers, exporters and panels can run.
    pub source: String,
    /// Panels it contributes.
    pub panels: Vec<PanelSpec>,
    /// Importers it contributes.
    pub importers: Vec<ankhimate_plugins::importer::JsImporter>,
    /// Why it did not load, if it did not. `Some` means the rest is empty.
    pub error: Option<String>,
}

impl Plugin {
    /// Did this one load?
    pub fn is_loaded(&self) -> bool {
        self.error.is_none()
    }
}

/// Every plugin the editor knows about.
#[derive(Default)]
pub struct Plugins {
    pub loaded: Vec<Plugin>,
}

impl Plugins {
    /// Read every `*.js` in the plugin directory.
    ///
    /// A missing directory is not an error: most users have no plugins, and
    /// creating the folder just to leave it empty is noise in their config
    /// directory.
    pub fn load(dir: &Path) -> Self {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self::default();
        };

        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("js"))
            .collect();
        // Sorted, so a shadowing plugin resolves the same way on every machine.
        // Directory order is not a contract and differs between filesystems.
        files.sort();

        Self {
            loaded: files.iter().map(|path| read_one(path)).collect(),
        }
    }

    /// The directory plugins are read from, beside `config.json`.
    pub fn directory() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "ankhimate")
            .map(|dirs| dirs.config_dir().join("plugins"))
    }

    /// Every panel any loaded plugin contributes, as `(id, title)`.
    pub fn panels(&self) -> Vec<(String, String)> {
        self.loaded
            .iter()
            .filter(|p| p.is_loaded())
            .flat_map(|p| p.panels.iter())
            .map(|spec| (spec.id.clone(), spec.title.clone()))
            .collect()
    }

    /// The script that declares the panel `id`.
    pub fn source_for_panel(&self, id: &str) -> Option<&str> {
        self.loaded
            .iter()
            .find(|p| p.panels.iter().any(|spec| spec.id == id))
            .map(|p| p.source.as_str())
    }

    /// Register every plugin importer into `importers`.
    ///
    /// Called where the built-ins register, so a plugin format reaches the
    /// File▸Import menu, a dropped file and an id lookup without any of those
    /// three knowing a plugin exists.
    pub fn register_importers(&self, importers: &mut ankhimate_formats::Importers) {
        for plugin in self.loaded.iter().filter(|p| p.is_loaded()) {
            for importer in &plugin.importers {
                importers.register(Box::new(importer.clone()));
            }
        }
    }

    /// A line per plugin, for the status bar or an about box.
    pub fn summary(&self) -> String {
        let failed = self.loaded.iter().filter(|p| !p.is_loaded()).count();
        match (self.loaded.len(), failed) {
            (0, _) => "No plugins".to_string(),
            (n, 0) => format!("{n} plugin(s)"),
            (n, f) => format!("{n} plugin(s), {f} failed to load"),
        }
    }
}

/// Read one plugin file and collect what it declares.
///
/// Declarations are collected in separate runs because the host collects one
/// kind at a time — a cost of three script evaluations per plugin at startup,
/// which is nothing against reading the file at all, and the alternative is a
/// combined entry point whose only caller is this function.
fn read_one(path: &Path) -> Plugin {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("plugin")
        .to_string();

    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            return Plugin {
                path: path.to_path_buf(),
                name,
                source: String::new(),
                panels: Vec::new(),
                importers: Vec::new(),
                error: Some(format!("could not be read: {e}")),
            };
        }
    };

    let host = Host::new();
    let panels = host.panels(&source);
    let importers = host.importers(&source);

    // One failure fails the file. A plugin whose panel registration threw has
    // not necessarily broken its importer, but running half a plugin is how a
    // user ends up with a format that reads and a panel that does not, with
    // nothing saying the two came from the same broken file.
    let error = panels
        .as_ref()
        .err()
        .or(importers.as_ref().err())
        .map(|e| e.to_string());

    Plugin {
        path: path.to_path_buf(),
        name,
        source,
        panels: panels.unwrap_or_default(),
        importers: importers.unwrap_or_default(),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, source: &str) {
        std::fs::write(dir.join(name), source).unwrap();
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        // Most users have no plugins, and creating the folder to leave it empty
        // is noise in their config directory.
        let plugins = Plugins::load(Path::new("no/such/place"));
        assert!(plugins.loaded.is_empty());
        assert_eq!(plugins.summary(), "No plugins");
    }

    #[test]
    fn a_plugin_that_throws_is_listed_with_its_reason() {
        // A plugin silently missing is somebody's afternoon. The same plugin
        // listed as "failed: …" is a fix.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "broken.js", "throw new Error('no rig here');");

        let plugins = Plugins::load(dir.path());
        assert_eq!(plugins.loaded.len(), 1, "it was not dropped");
        assert!(!plugins.loaded[0].is_loaded());
        assert!(
            plugins.loaded[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("no rig here"),
            "the author's own message survived: {:?}",
            plugins.loaded[0].error
        );
        assert_eq!(plugins.summary(), "1 plugin(s), 1 failed to load");
    }

    #[test]
    fn a_panel_and_an_importer_from_one_file_both_arrive() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "tools.js",
            r#"
            ankhimate.registerPanel({ id: "tools.mirror", title: "Mirror",
                                      build: () => [{ text: "hi" }] });
            ankhimate.registerImporter({ id: "import.toy", label: "Toy",
                                         extensions: ["toy"], read() {} });
            "#,
        );

        let plugins = Plugins::load(dir.path());
        assert_eq!(plugins.panels(), [("tools.mirror".into(), "Mirror".into())]);
        assert_eq!(plugins.loaded[0].importers.len(), 1);
    }

    #[test]
    fn a_plugin_importer_reaches_the_registry() {
        // The join that makes the whole surface reachable: a plugin format
        // appears in File▸Import with nobody writing menu code for it.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "toy.js",
            r#"ankhimate.registerImporter({ id: "import.toy", label: "Toy",
                                            extensions: ["toy"], read() {} });"#,
        );

        let plugins = Plugins::load(dir.path());
        let mut importers = ankhimate_formats::Importers::builtin();
        let before = importers.ids().count();
        plugins.register_importers(&mut importers);

        assert_eq!(importers.ids().count(), before + 1);
        assert!(
            importers.get("import.toy").is_some(),
            "and it is reachable by the id the plugin gave"
        );
    }

    #[test]
    fn files_are_read_in_a_stable_order() {
        // Directory order is not a contract and differs between filesystems. A
        // plugin that shadows another must resolve the same way on every
        // machine or a bug report cannot be reproduced.
        let dir = tempfile::tempdir().unwrap();
        for name in ["zulu.js", "alpha.js", "mike.js"] {
            write(dir.path(), name, "");
        }

        let plugins = Plugins::load(dir.path());
        let names: Vec<&str> = plugins.loaded.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mike", "zulu"]);
    }

    #[test]
    fn a_file_that_is_not_javascript_is_ignored() {
        // A README beside the plugins is a README, not a plugin that failed.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "README.md", "# my plugins");
        write(dir.path(), "real.js", "");

        let plugins = Plugins::load(dir.path());
        assert_eq!(plugins.loaded.len(), 1);
        assert_eq!(plugins.loaded[0].name, "real");
    }
}
