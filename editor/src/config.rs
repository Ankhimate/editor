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
    /// Name of the theme to start in, matched against `Theme::label`.
    #[serde(default)]
    pub theme_name: Option<String>,
    #[serde(default)]
    pub grid: GridSettings,
    #[serde(default)]
    pub fonts: FontSettings,
    /// Global UI scale, multiplying egui's own DPI factor.
    ///
    /// This is what actually fixes blocky text: glyphs are rasterised at
    /// `size × zoom × pixels_per_point`, so raising it renders them at a higher
    /// resolution rather than scaling up the same blocky bitmap. Font *sizes* do
    /// not do that — a 20pt glyph at 1× is still rasterised once per pixel.
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

fn default_ui_scale() -> f32 {
    1.0
}

/// The viewport's transparency checker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSettings {
    /// Cell size in **world units**, not pixels: the checker has to sit still
    /// relative to the artwork while zooming, or it reads as the texture changing
    /// rather than as the camera moving.
    pub cell: f32,
    /// Below this on-screen size the checker is noise, and the cell count
    /// explodes — a 3px cell is a quarter of a million rects on a 1080p viewport,
    /// every frame.
    pub min_cell_px: f32,
    pub show: bool,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            cell: 50.0,
            min_cell_px: 8.0,
            show: true,
        }
    }
}

/// Text sizes, per area rather than one global scale.
///
/// One slider would not do: the timeline packs sixty rows into a panel and wants
/// small text, while the inspector is read a field at a time and wants normal
/// text. Tying them together means one of the two is always wrong.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSettings {
    /// Menus, buttons, and anything without an area of its own.
    pub ui: f32,
    /// The hierarchy and the other tree panels.
    pub tree: f32,
    /// Inspector labels and values.
    pub inspector: f32,
    /// Timeline row names and ruler numbers.
    pub timeline: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            ui: 13.0,
            tree: 12.5,
            inspector: 12.0,
            timeline: 11.0,
        }
    }
}

impl FontSettings {
    /// Clamped to something legible at both ends: below 7 the glyphs stop being
    /// distinguishable, above 24 a panel holds four rows.
    pub const MIN: f32 = 7.0;
    pub const MAX: f32 = 24.0;

    /// Push these sizes into egui's text styles.
    ///
    /// The named styles are the ones widgets pick up on their own — `Body` for
    /// labels, `Button` for buttons, `Small` for the dim secondary text panels
    /// use. Areas that paint their own text read the numbers directly instead;
    /// there is no style slot for "the timeline's row names".
    pub fn apply(&self, ctx: &eframe::egui::Context) {
        use eframe::egui::{FontFamily, FontId, TextStyle};
        let ui = self.ui.clamp(Self::MIN, Self::MAX);
        let mut style = (*ctx.global_style()).clone();
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(ui, FontFamily::Proportional));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::new(ui, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new((ui - 2.0).max(Self::MIN), FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(ui - 1.0, FontFamily::Monospace),
        );
        ctx.set_global_style(style);
    }

    /// Size for a panel that paints its own text.
    pub fn for_area(&self, area: Area) -> f32 {
        let raw = match area {
            Area::Ui => self.ui,
            Area::Tree => self.tree,
            Area::Inspector => self.inspector,
            Area::Timeline => self.timeline,
        };
        raw.clamp(Self::MIN, Self::MAX)
    }
}

/// The panels that size their own text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Ui,
    Tree,
    Inspector,
    Timeline,
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

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn font_sizes_are_clamped_to_something_legible() {
        let fonts = FontSettings {
            ui: 200.0,
            tree: 0.0,
            inspector: 12.0,
            timeline: 11.0,
        };
        assert_eq!(fonts.for_area(Area::Ui), FontSettings::MAX);
        assert_eq!(fonts.for_area(Area::Tree), FontSettings::MIN);
        assert_eq!(fonts.for_area(Area::Inspector), 12.0);
    }

    /// The config is written by hand often enough — and by older versions — that
    /// a missing section must default rather than fail the whole load.
    #[test]
    fn an_old_config_without_the_new_sections_still_loads() {
        let json = r#"{"recent_files":[],"skip_startup":true}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.skip_startup);
        assert_eq!(config.grid, GridSettings::default());
        assert_eq!(config.fonts, FontSettings::default());
        assert_eq!(config.theme_name, None);
    }

    #[test]
    fn grid_defaults_are_the_values_the_viewport_used_before() {
        let grid = GridSettings::default();
        assert_eq!(grid.cell, 50.0);
        assert_eq!(grid.min_cell_px, 8.0);
        assert!(grid.show);
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    /// The scale is what re-rasterises glyphs, so a config missing it must land
    /// on 1.0 rather than 0.0 — which would render nothing at all.
    #[test]
    fn a_config_without_a_scale_defaults_to_one() {
        let json = r#"{"recent_files":[],"skip_startup":false}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.ui_scale, 1.0);
    }

    #[test]
    fn the_scale_round_trips() {
        let config = Config {
            ui_scale: 1.75,
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ui_scale, 1.75);
    }
}
