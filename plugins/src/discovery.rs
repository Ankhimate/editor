//! Filesystem discovery for ordinary JavaScript plugin files and packages.

use crate::{Host, PluginError, exporter::JsExporter, importer::JsImporter, panel::PanelSpec};
use std::path::{Path, PathBuf};

/// One discovered script and the declarations it contributed.
pub struct Plugin {
    pub path: PathBuf,
    pub name: String,
    pub source: String,
    pub panels: Vec<PanelSpec>,
    pub importers: Vec<JsImporter>,
    pub exporters: Vec<JsExporter>,
    pub error: Option<String>,
}

impl Plugin {
    pub fn is_loaded(&self) -> bool {
        self.error.is_none()
    }
}

/// Platform configuration directory shared by the editor and MCP server.
pub fn directory() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ankhimate")
        .map(|dirs| dirs.config_dir().join("plugins"))
}

/// Load flat `*.js` plugins and `<name>/plugin.js` packages in stable order.
pub fn load(dir: &Path) -> Vec<Plugin> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            if path.extension().and_then(|ext| ext.to_str()) == Some("js") {
                Some(path)
            } else if path.is_dir() && path.join("plugin.js").is_file() {
                Some(path.join("plugin.js"))
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files.iter().map(|path| read_one(path)).collect()
}

fn read_one(path: &Path) -> Plugin {
    let name = if path.file_name().and_then(|name| name.to_str()) == Some("plugin.js") {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("plugin")
            .to_string()
    } else {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("plugin")
            .to_string()
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => return failed(path, name, format!("could not be read: {error}")),
    };
    let host = Host::new().with_resources(read_resources(path));
    let panels = host.panels(&source);
    let importers = host.importers(&source);
    let exporters = host.exporters(&source);
    let error = first_error(&panels, &importers, &exporters);
    Plugin {
        path: path.to_path_buf(),
        name,
        source,
        panels: panels.unwrap_or_default(),
        importers: importers.unwrap_or_default(),
        exporters: exporters.unwrap_or_default(),
        error,
    }
}

fn first_error(
    panels: &Result<Vec<PanelSpec>, PluginError>,
    importers: &Result<Vec<JsImporter>, PluginError>,
    exporters: &Result<Vec<JsExporter>, PluginError>,
) -> Option<String> {
    panels
        .as_ref()
        .err()
        .or(importers.as_ref().err())
        .or(exporters.as_ref().err())
        .map(ToString::to_string)
}

fn failed(path: &Path, name: String, error: String) -> Plugin {
    Plugin {
        path: path.to_path_buf(),
        name,
        source: String::new(),
        panels: Vec::new(),
        importers: Vec::new(),
        exporters: Vec::new(),
        error: Some(error),
    }
}

fn read_resources(script: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    if script.file_name().and_then(|name| name.to_str()) != Some("plugin.js") {
        return Default::default();
    }
    let Some(root) = script.parent() else {
        return Default::default();
    };
    let mut resources = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path != script
                && let Ok(relative) = path.strip_prefix(root)
                && let Ok(bytes) = std::fs::read(&path)
            {
                resources.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
            }
        }
    }
    resources
}
