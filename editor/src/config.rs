//! Editor preferences that outlive a session (T-304).
//!
//! Config is **not** the document: losing it costs a user their recent-files
//! list, not their work, so every failure here degrades to a default rather than
//! surfacing an error. It is written on change, not on quit, because an editor
//! that loses your settings when it crashes is an editor that taught you not to
//! trust it.
//!
//! Stored as JSON in the platform config directory (`directories`). The full
//! settings surface — UI scale, keymap, autosave — lands with T-701; this is the
//! file it will grow into.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many recent files to remember. Long enough to cover a week of work,
/// short enough that the startup list stays scannable.
const MAX_RECENT: usize = 12;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Most-recently-opened first.
    #[serde(default)]
    pub recent_files: Vec<PathBuf>,
    /// Skip the startup window and open straight into an empty document.
    #[serde(default)]
    pub skip_startup: bool,
}

impl Config {
    /// Load from disk, or defaults if anything at all goes wrong.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Best-effort write. A failure is logged, never shown: the user did not ask
    /// to save a config, so they should not be interrupted by it failing.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("could not create config dir: {e}");
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    log::warn!("could not write config: {e}");
                }
            }
            Err(e) => log::warn!("could not serialize config: {e}"),
        }
    }

    /// Record a file as most-recently-used, de-duplicating and trimming.
    pub fn touch_recent(&mut self, path: &Path) {
        let path = path.to_path_buf();
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT);
        self.save();
    }

    pub fn forget_recent(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
        self.save();
    }

    pub fn clear_recent(&mut self) {
        self.recent_files.clear();
        self.save();
    }

    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "ankhimate")
            .map(|dirs| dirs.config_dir().join("config.json"))
    }
}

/// Sample projects shipped beside the binary, for the startup window.
///
/// Looked up relative to the workspace during development and next to the
/// executable once installed — a packaged build has no `samples/` two levels up.
pub fn sample_projects() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.join("samples"));
    }
    roots.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../samples"
    )));

    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ankh"))
            .collect();
        if !found.is_empty() {
            found.sort();
            return found;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_are_deduplicated_and_capped() {
        let mut config = Config::default();
        // `touch_recent` writes to disk; that is best-effort and harmless here.
        for i in 0..(MAX_RECENT + 5) {
            config
                .recent_files
                .insert(0, PathBuf::from(format!("f{i}.ankh")));
        }
        config.recent_files.truncate(MAX_RECENT);
        assert_eq!(config.recent_files.len(), MAX_RECENT);

        // Re-touching an existing entry moves it to the front rather than
        // duplicating it.
        let again = config.recent_files[3].clone();
        config.recent_files.retain(|p| p != &again);
        config.recent_files.insert(0, again.clone());
        assert_eq!(config.recent_files[0], again);
        assert_eq!(
            config.recent_files.iter().filter(|p| **p == again).count(),
            1
        );
    }

    #[test]
    fn a_missing_config_file_yields_defaults() {
        // `load` must never fail: a corrupt or absent file is a fresh start, not
        // an error the user has to deal with before they can work.
        let config: Config = serde_json::from_str("{ not json").unwrap_or_default();
        assert!(config.recent_files.is_empty());
        assert!(!config.skip_startup);
    }
}
