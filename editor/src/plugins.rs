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
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const MARKETPLACE_INDEX_URL: &str =
    "https://raw.githubusercontent.com/Ankhimate/community-plugins/main/marketplace.json";
const MARKETPLACE_FILES_URL: &str =
    "https://raw.githubusercontent.com/Ankhimate/community-plugins/main";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Every plugin the editor knows about.
#[derive(Default)]
pub struct Plugins {
    pub loaded: Vec<Plugin>,
}

/// Remote catalog and one in-flight installation. Network work stays off the
/// UI thread; completed installs are reported to `AnkhimateApp`, which reloads
/// the ordinary on-disk plugin registry.
pub struct Marketplace {
    catalog: CatalogState,
    catalog_rx: Option<std::sync::mpsc::Receiver<Result<Vec<MarketplacePlugin>, String>>>,
    install_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    installing: Option<String>,
}

enum CatalogState {
    Loading,
    Ready(Vec<MarketplacePlugin>),
    Failed(String),
}

impl Default for Marketplace {
    fn default() -> Self {
        Self {
            catalog: CatalogState::Ready(Vec::new()),
            catalog_rx: None,
            install_rx: None,
            installing: None,
        }
    }
}

impl Marketplace {
    pub fn fetch(ctx: &eframe::egui::Context) -> Self {
        let mut marketplace = Self::default();
        marketplace.refresh(ctx);
        marketplace
    }

    pub fn refresh(&mut self, ctx: &eframe::egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_marketplace());
            repaint.request_repaint();
        });
        self.catalog = CatalogState::Loading;
        self.catalog_rx = Some(rx);
    }

    pub fn begin_install(
        &mut self,
        plugin: MarketplacePlugin,
        ctx: &eframe::egui::Context,
    ) -> Result<(), String> {
        if self.install_rx.is_some() {
            return Err("another plugin installation is already running".into());
        }
        let id = plugin.id.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Plugins::install_marketplace(&plugin));
            repaint.request_repaint();
        });
        self.installing = Some(id);
        self.install_rx = Some(rx);
        Ok(())
    }

    /// Advance background work and return a completed installation, if any.
    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        if let Some(rx) = &self.catalog_rx {
            match rx.try_recv() {
                Ok(Ok(plugins)) => {
                    self.catalog = CatalogState::Ready(plugins);
                    self.catalog_rx = None;
                }
                Ok(Err(error)) => {
                    self.catalog = CatalogState::Failed(error);
                    self.catalog_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.catalog = CatalogState::Failed("marketplace worker stopped".into());
                    self.catalog_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let rx = self.install_rx.as_ref()?;
        match rx.try_recv() {
            Ok(result) => {
                self.install_rx = None;
                self.installing = None;
                Some(result)
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.install_rx = None;
                self.installing = None;
                Some(Err("plugin installation worker stopped".into()))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        }
    }

    pub fn plugins(&self) -> Option<&[MarketplacePlugin]> {
        match &self.catalog {
            CatalogState::Ready(plugins) => Some(plugins),
            _ => None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match &self.catalog {
            CatalogState::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.catalog, CatalogState::Loading)
    }

    pub fn installing(&self) -> Option<&str> {
        self.installing.as_deref()
    }
}

/// One remotely published package. The repository owns this metadata; the
/// editor has no compiled-in list of plugin ids or formats.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MarketplacePlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub files: Vec<String>,
}

#[derive(Deserialize)]
struct MarketplaceIndex {
    schema: u32,
    plugins: Vec<MarketplacePlugin>,
}

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

    /// Download one marketplace package without overwriting anything.
    pub fn install_marketplace(plugin: &MarketplacePlugin) -> Result<PathBuf, String> {
        let root =
            Self::directory().ok_or_else(|| "plugin directory is unavailable".to_string())?;
        install_marketplace_into(&root, plugin)
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

/// Fetch and validate the catalog published by `community-plugins`.
pub fn fetch_marketplace() -> Result<Vec<MarketplacePlugin>, String> {
    let bytes = fetch_bytes(MARKETPLACE_INDEX_URL, MAX_INDEX_BYTES)?;
    parse_marketplace(&bytes)
}

fn parse_marketplace(bytes: &[u8]) -> Result<Vec<MarketplacePlugin>, String> {
    let index: MarketplaceIndex = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid marketplace index: {error}"))?;
    if index.schema != 1 {
        return Err(format!("unsupported marketplace schema {}", index.schema));
    }
    let mut ids = HashSet::new();
    for plugin in &index.plugins {
        validate_plugin(plugin)?;
        if !ids.insert(plugin.id.as_str()) {
            return Err(format!("duplicate marketplace plugin `{}`", plugin.id));
        }
    }
    Ok(index.plugins)
}

fn validate_plugin(plugin: &MarketplacePlugin) -> Result<(), String> {
    if plugin.id.is_empty()
        || !plugin
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("invalid marketplace plugin id `{}`", plugin.id));
    }
    if plugin.name.trim().is_empty() || plugin.version.trim().is_empty() {
        return Err(format!(
            "marketplace plugin `{}` has incomplete metadata",
            plugin.id
        ));
    }
    if !plugin.files.iter().any(|file| file == "plugin.js") {
        return Err(format!(
            "marketplace plugin `{}` has no plugin.js",
            plugin.id
        ));
    }
    if plugin.files.len() > 128 {
        return Err(format!(
            "marketplace plugin `{}` declares too many files",
            plugin.id
        ));
    }
    let mut files = HashSet::new();
    for file in &plugin.files {
        let path = Path::new(file);
        if file.is_empty()
            || path.components().any(|component| match component {
                Component::Normal(name) => name.to_str().is_none_or(|name| {
                    name.is_empty()
                        || !name.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                        })
                }),
                _ => true,
            })
        {
            return Err(format!(
                "marketplace plugin `{}` has unsafe file `{file}`",
                plugin.id
            ));
        }
        if !files.insert(file) {
            return Err(format!(
                "marketplace plugin `{}` repeats file `{file}`",
                plugin.id
            ));
        }
    }
    Ok(())
}

fn fetch_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .set("User-Agent", "ankhimate-editor")
        .call()
        .map_err(|error| format!("could not fetch {url}: {error}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {url}: {error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("download from {url} exceeds {limit} bytes"));
    }
    Ok(bytes)
}

fn install_marketplace_into(root: &Path, plugin: &MarketplacePlugin) -> Result<PathBuf, String> {
    install_marketplace_into_with(root, plugin, |file| {
        fetch_bytes(&marketplace_file_url(plugin, file), MAX_PLUGIN_FILE_BYTES)
    })
}

fn marketplace_file_url(plugin: &MarketplacePlugin, file: &str) -> String {
    // The package path points at `main`, so its URL otherwise never changes and
    // a CDN may serve the previous package after the marketplace index has
    // already announced a new version. The catalog version is a cache key, not
    // an API argument; raw.githubusercontent.com ignores it when serving bytes.
    format!(
        "{MARKETPLACE_FILES_URL}/{}/{file}?version={}",
        plugin.id, plugin.version
    )
}

fn install_marketplace_into_with(
    root: &Path,
    plugin: &MarketplacePlugin,
    mut download: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    validate_plugin(plugin)?;
    let target = root.join(&plugin.id);
    if target.exists() {
        return Err(format!("{} already exists", target.display()));
    }
    std::fs::create_dir_all(root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;
    let staging = root.join(format!(".{}.installing", plugin.id));
    if staging.exists() {
        return Err(format!(
            "incomplete installation exists at {}",
            staging.display()
        ));
    }
    std::fs::create_dir(&staging)
        .map_err(|error| format!("could not create {}: {error}", staging.display()))?;

    let result = (|| {
        for file in &plugin.files {
            let bytes = download(file)?;
            let destination = staging.join(file);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }
            std::fs::write(&destination, bytes)
                .map_err(|error| format!("could not install {file}: {error}"))?;
        }
        std::fs::rename(&staging, &target)
            .map_err(|error| format!("could not publish {}: {error}", target.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
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
    fn marketplace_install_makes_a_downloaded_plugin_discoverable() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = MarketplacePlugin {
            id: "remote-format".into(),
            name: "Remote Format".into(),
            version: "1.0.0".into(),
            description: "test".into(),
            files: vec!["plugin.js".into(), "preset.json".into()],
        };
        let installed = install_marketplace_into_with(dir.path(), &plugin, |file| {
            Ok(match file {
                "plugin.js" => br#"ankhimate.registerImporter({ id: "import.remote", label: "Remote", extensions: ["remote"], read() {} });"#.to_vec(),
                "preset.json" => b"{}".to_vec(),
                _ => unreachable!(),
            })
        })
        .unwrap();
        assert!(installed.join("plugin.js").is_file());
        assert!(installed.join("preset.json").is_file());

        let plugins = Plugins::load(dir.path());
        assert!(plugins.has_importer("import.remote"));
    }

    #[test]
    fn marketplace_file_urls_change_with_the_package_version() {
        let mut plugin = MarketplacePlugin {
            id: "dragonbones".into(),
            name: "DragonBones".into(),
            version: "0.2.0".into(),
            description: String::new(),
            files: vec!["plugin.js".into()],
        };
        let old = marketplace_file_url(&plugin, "plugin.js");
        plugin.version = "0.2.1".into();
        let new = marketplace_file_url(&plugin, "plugin.js");
        assert_ne!(old, new);
        assert!(new.ends_with("?version=0.2.1"));
    }

    #[test]
    fn installing_never_overwrites_an_existing_package() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = MarketplacePlugin {
            id: "existing".into(),
            name: "Existing".into(),
            version: "1".into(),
            description: String::new(),
            files: vec!["plugin.js".into()],
        };
        let target = dir.path().join("existing");
        std::fs::create_dir(&target).unwrap();
        write(&target, "plugin.js", "keep me");

        assert!(
            install_marketplace_into_with(dir.path(), &plugin, |_| { Ok(b"replace me".to_vec()) })
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(target.join("plugin.js")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn the_repository_catalog_defines_the_available_plugins() {
        let plugins = parse_marketplace(
            br#"{"schema":1,"plugins":[{"id":"not-compiled-in","name":"New", "version":"2", "description":"remote", "files":["plugin.js"]}]}"#,
        )
        .unwrap();
        assert_eq!(plugins[0].id, "not-compiled-in");
    }

    #[test]
    fn unsafe_paths_are_rejected_before_any_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = MarketplacePlugin {
            id: "escape".into(),
            name: "Escape".into(),
            version: "1".into(),
            description: String::new(),
            files: vec!["plugin.js".into(), "../outside".into()],
        };
        assert!(install_marketplace_into_with(dir.path(), &plugin, |_| Ok(Vec::new())).is_err());
        assert!(!dir.path().join("escape").exists());
    }

    #[test]
    fn a_failed_download_leaves_no_visible_or_partial_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = MarketplacePlugin {
            id: "partial".into(),
            name: "Partial".into(),
            version: "1".into(),
            description: String::new(),
            files: vec!["plugin.js".into(), "resource.json".into()],
        };
        let result = install_marketplace_into_with(dir.path(), &plugin, |file| {
            if file == "plugin.js" {
                Ok(b"valid first file".to_vec())
            } else {
                Err("network stopped".into())
            }
        });
        assert!(result.is_err());
        assert!(!dir.path().join("partial").exists());
        assert!(!dir.path().join(".partial.installing").exists());
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
