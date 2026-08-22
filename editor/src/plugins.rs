//! Loading plugins, and what the editor does with them.
//!
//! Until this existed nothing in the workspace depended on `ankhimate-plugins`:
//! importers, exporters, the PSD read surface, forty-nine verbs and the panel
//! vocabulary were all tested and none of them reachable from the running app.
//!
//! # Where plugins live
//!
//! `<config>/plugins/*.js` and `<config>/plugins/<name>/plugin.js`, beside
//! `config.json`. Packages may carry resources beside `plugin.js`; the
//! sandbox exposes those bytes but still exposes no arbitrary filesystem.
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

pub use ankhimate_plugins::discovery::Plugin;
use std::path::{Path, PathBuf};

/// Every plugin the editor knows about.
#[derive(Default)]
pub struct Plugins {
    pub loaded: Vec<Plugin>,
}

/// An opt-in package shipped as source, installed into the same directory as
/// any plugin the user writes or downloads. It is not registered until its JS
/// file is installed and discovered.
pub struct CommunityPlugin {
    pub id: &'static str,
    pub label: &'static str,
    pub importer_id: &'static str,
}

pub const COMMUNITY_PLUGINS: &[CommunityPlugin] = &[
    CommunityPlugin {
        id: "spine",
        label: "Spine JSON",
        importer_id: "import.spine",
    },
    CommunityPlugin {
        id: "dragonbones",
        label: "DragonBones JSON",
        importer_id: "import.dragonbones",
    },
];

impl Plugins {
    /// Read every flat `*.js` plugin and `<name>/plugin.js` package.
    ///
    /// A missing directory is not an error: most users have no plugins, and
    /// creating the folder just to leave it empty is noise in their config
    /// directory.
    pub fn load(dir: &Path) -> Self {
        Self {
            loaded: ankhimate_plugins::discovery::load(dir),
        }
    }

    /// The directory plugins are read from, beside `config.json`.
    pub fn directory() -> Option<PathBuf> {
        ankhimate_plugins::discovery::directory()
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

    pub fn has_importer(&self, id: &str) -> bool {
        self.loaded
            .iter()
            .filter(|plugin| plugin.is_loaded())
            .flat_map(|plugin| &plugin.importers)
            .any(|importer| ankhimate_formats::Importer::id(importer) == id)
    }

    /// Install one shipped community package without overwriting anything.
    /// Resources are written first and `plugin.js` last, so a failed partial
    /// install is ignored by discovery rather than loaded without its files.
    pub fn install_community(id: &str) -> Result<PathBuf, String> {
        let root =
            Self::directory().ok_or_else(|| "plugin directory is unavailable".to_string())?;
        install_community_into(&root, id)
    }

    /// Every exporter any loaded plugin contributes, as `(id, label)`.
    ///
    /// For the export panel's format list, which offers these beside the
    /// template presets: a plugin exporter is a format like any other, and a
    /// separate menu for "plugin formats" would be the duplication the registry
    /// exists to remove.
    pub fn exporters(&self) -> Vec<(String, String)> {
        self.loaded
            .iter()
            .filter(|p| p.is_loaded())
            .flat_map(|p| p.exporters.iter())
            .map(|e| (e.id.clone(), e.label.clone()))
            .collect()
    }

    /// The exporter whose *label* is `label`.
    ///
    /// Looked up by label rather than id because that is what a preset carries
    /// and what the panel shows — a user picking "Toy Format" from a list has
    /// named the thing they can see.
    pub fn exporter_named(&self, label: &str) -> Option<&ankhimate_plugins::exporter::JsExporter> {
        self.loaded
            .iter()
            .filter(|p| p.is_loaded())
            .flat_map(|p| p.exporters.iter())
            .find(|e| e.label == label)
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

fn install_community_into(root: &Path, id: &str) -> Result<PathBuf, String> {
    let (script, resources): (&[u8], &[(&str, &[u8])]) = match id {
        "spine" => (
            include_bytes!("../../community-plugins/spine/plugin.js"),
            &[(
                "spine_json.json",
                include_bytes!("../../community-plugins/spine/spine_json.json"),
            )],
        ),
        "dragonbones" => (
            include_bytes!("../../community-plugins/dragonbones/plugin.js"),
            &[],
        ),
        _ => return Err(format!("unknown community plugin `{id}`")),
    };
    let target = root.join(id);
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    std::fs::create_dir_all(root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    std::fs::create_dir(&target)
        .map_err(|error| format!("could not create {}: {error}", target.display()))?;

    for (name, bytes) in resources {
        std::fs::write(target.join(name), bytes)
            .map_err(|error| format!("could not install {name}: {error}"))?;
    }
    std::fs::write(target.join("plugin.js"), script)
        .map_err(|error| format!("could not install {id}: {error}"))?;
    Ok(target)
}

/// Read one plugin file and collect what it declares.
///
/// Declarations are collected in separate runs because the host collects one
/// kind at a time — a cost of three script evaluations per plugin at startup,
/// which is nothing against reading the file at all, and the alternative is a
/// combined entry point whose only caller is this function.
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
    fn installing_a_community_importer_makes_it_discoverable() {
        let dir = tempfile::tempdir().unwrap();
        let installed = install_community_into(dir.path(), "spine").unwrap();
        assert!(installed.join("plugin.js").is_file());
        assert!(installed.join("spine_json.json").is_file());

        let plugins = Plugins::load(dir.path());
        assert!(plugins.has_importer("import.spine"));
        assert!(
            plugins
                .exporters()
                .iter()
                .any(|(id, _)| id == "export.spine")
        );
    }

    #[test]
    fn installing_never_overwrites_an_existing_package() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dragonbones");
        std::fs::create_dir(&target).unwrap();
        write(&target, "plugin.js", "keep me");

        assert!(install_community_into(dir.path(), "dragonbones").is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("plugin.js")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn an_unknown_community_package_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install_community_into(dir.path(), "mystery").is_err());
        assert!(!dir.path().join("mystery").exists());
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

    #[test]
    fn a_plugin_package_can_read_its_own_resources() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("format");
        std::fs::create_dir(&package).unwrap();
        write(
            &package,
            "plugin.js",
            r#"if (ankhimate.resource("preset.json") !== "{\"ok\":true}")
                 throw new Error("resource missing");"#,
        );
        write(&package, "preset.json", r#"{"ok":true}"#);

        let plugins = Plugins::load(dir.path());
        assert_eq!(plugins.loaded.len(), 1);
        assert_eq!(plugins.loaded[0].name, "format");
        assert!(
            plugins.loaded[0].is_loaded(),
            "{:?}",
            plugins.loaded[0].error
        );
    }

    #[test]
    fn the_summary_says_what_is_worth_saying() {
        // A user whose plugin did not appear needs to tell "no plugins found"
        // from "one plugin, and it failed" — the two look identical in a menu
        // that only lists what worked.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Plugins::load(dir.path()).summary(), "No plugins");

        write(dir.path(), "good.js", "");
        assert_eq!(Plugins::load(dir.path()).summary(), "1 plugin(s)");

        write(dir.path(), "bad.js", "throw new Error('nope');");
        assert_eq!(
            Plugins::load(dir.path()).summary(),
            "2 plugin(s), 1 failed to load"
        );
    }

    #[test]
    fn the_plugin_directory_sits_beside_the_config_file() {
        // Where this is wrong the user has no way to find out: a plugin in the
        // wrong folder is indistinguishable from one that threw. Pinned because
        // I put a file one level up from here and lost a restart to it.
        let Some(dir) = Plugins::directory() else {
            return;
        };
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("plugins"));
        assert!(
            dir.parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .is_some(),
            "it has a parent to sit beside"
        );
    }

    #[test]
    fn a_plugin_exporter_is_found_by_the_label_the_user_sees() {
        // The export panel lists formats by label and a preset carries one, so
        // that is what the lookup takes. An id lookup would mean the panel
        // holding a second identifier it never shows.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "toy.js",
            r#"ankhimate.registerExporter({ id: "export.toy", label: "Toy Format",
                                            write() {} });"#,
        );

        let plugins = Plugins::load(dir.path());
        assert_eq!(
            plugins.exporters(),
            [("export.toy".to_string(), "Toy Format".to_string())]
        );
        assert!(plugins.exporter_named("Toy Format").is_some());
        assert!(
            plugins.exporter_named("export.toy").is_none(),
            "the id is not what the panel shows, so it is not what it looks up"
        );
    }

    #[test]
    fn a_plugin_that_registers_all_three_gives_all_three() {
        // One file, one plugin: an importer, an exporter and a panel from the
        // same script all arrive, rather than the first kind found winning.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "everything.js",
            r#"
            ankhimate.registerPanel({ id: "p.one", title: "One", build: () => [] });
            ankhimate.registerImporter({ id: "i.one", label: "In",
                                         extensions: ["one"], read() {} });
            ankhimate.registerExporter({ id: "e.one", label: "Out", write() {} });
            "#,
        );

        let plugins = Plugins::load(dir.path());
        assert_eq!(plugins.panels().len(), 1);
        assert_eq!(plugins.loaded[0].importers.len(), 1);
        assert_eq!(plugins.exporters().len(), 1);
    }
}
