use eframe::egui::{Color32, Context, Visuals};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub primary: String,
    pub on_primary: String,
    pub panel_fill: String,
    pub window_fill: String,
    pub faint_bg_color: String,
    pub extreme_bg_color: String,
    pub grid_color_even: String,
    pub grid_color_odd: String,
    pub origin_color: String,

    // ── Mesh editing (T-401) ────────────────────────────────────────────
    // Defaulted so a theme file written before these existed still loads —
    // a missing colour should not stop a theme from being usable.
    /// Wireframe edges.
    #[serde(default = "default_mesh_edge")]
    pub mesh_edge: String,
    /// An unselected vertex handle.
    #[serde(default = "default_mesh_vertex")]
    pub mesh_vertex: String,
    /// A selected vertex handle.
    #[serde(default = "default_mesh_vertex_selected")]
    pub mesh_vertex_selected: String,

    // ── Non-drawing attachments ─────────────────────────────────────────
    // A hitbox and a point marker have to read as *not artwork* at a glance,
    // which is why they get their own hues rather than borrowing the mesh
    // wireframe's.
    /// Outline of a bounding-box attachment.
    #[serde(default = "default_hitbox_outline")]
    pub hitbox_outline: String,
    /// Fill of a bounding-box attachment. Deliberately faint: a hitbox covers
    /// the art it guards, so an opaque one hides the thing you are aiming at.
    #[serde(default = "default_hitbox_fill")]
    pub hitbox_fill: String,
    /// Cross and heading tick of a point attachment.
    #[serde(default = "default_point_marker")]
    pub point_marker: String,

    // ── Artwork outlines (T-708) ────────────────────────────────────────
    /// Silhouette of the artwork under the cursor.
    #[serde(default = "default_outline_hover")]
    pub outline_hover: String,
    /// Silhouette of the selected artwork. Must read as *chosen* next to the
    /// hover colour, not merely brighter, or the two are indistinguishable while
    /// the cursor is still on the piece you just clicked.
    #[serde(default = "default_outline_selected")]
    pub outline_selected: String,

    // ── Animation channels ──────────────────────────────────────────────
    // One colour per property, shared by the timeline row icon, the graph curve
    // and its key dots. Colouring the graph by *axis* instead meant a green
    // curve was "the y one" in one row and "the rotate one" in the next, so the
    // colour carried no meaning across a panel.
    #[serde(default = "default_channel_translate")]
    pub channel_translate: String,
    #[serde(default = "default_channel_rotate")]
    pub channel_rotate: String,
    #[serde(default = "default_channel_scale")]
    pub channel_scale: String,
    #[serde(default = "default_channel_shear")]
    pub channel_shear: String,
    /// Event markers, in the timeline lane and the event pane.
    #[serde(default = "default_event_marker")]
    pub event_marker: String,
}

fn default_channel_translate() -> String {
    "#6ea0e6".into()
}

fn default_channel_rotate() -> String {
    "#6ec86e".into()
}

fn default_channel_scale() -> String {
    "#e0c25a".into()
}

fn default_channel_shear() -> String {
    "#e05a5a".into()
}

fn default_event_marker() -> String {
    "#e6aa46".into()
}

fn default_outline_hover() -> String {
    "#ffffffb4".into()
}

fn default_outline_selected() -> String {
    "#ffc83cff".into()
}

fn default_hitbox_outline() -> String {
    "#ff8c69c8".into()
}

fn default_hitbox_fill() -> String {
    "#ff8c6922".into()
}

fn default_point_marker() -> String {
    "#7ce38b".into()
}

fn default_mesh_edge() -> String {
    "#78dcffb4".into()
}

fn default_mesh_vertex() -> String {
    "#9ec8e6".into()
}

fn default_mesh_vertex_selected() -> String {
    "#ffc83c".into()
}

pub fn hex_to_color(hex: &str) -> Color32 {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 || hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        let a = if hex.len() == 8 {
            u8::from_str_radix(&hex[6..8], 16).unwrap_or(255)
        } else {
            255
        };
        Color32::from_rgba_unmultiplied(r, g, b, a)
    } else {
        Color32::BLACK
    }
}

/// A filename-safe form of a theme name.
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore && !out.is_empty() {
            out.push('_');
            last_underscore = true;
        }
    }
    let trimmed = out.trim_end_matches('_').to_string();
    if trimmed.is_empty() {
        "theme".to_string()
    } else {
        trimmed
    }
}

impl Default for Theme {
    fn default() -> Self {
        serde_json::from_str(include_str!("themes/default.json")).unwrap()
    }
}

impl Theme {
    pub fn load_all() -> Vec<Theme> {
        vec![
            Theme::default(),
            serde_json::from_str(include_str!("themes/nord.json")).unwrap(),
            serde_json::from_str(include_str!("themes/solarized_dark.json")).unwrap(),
            serde_json::from_str(include_str!("themes/catppuccin.json")).unwrap(),
        ]
    }

    pub fn label(&self) -> &str {
        &self.name
    }

    /// Every colour the settings window can edit, as `(label, field)`.
    ///
    /// Hand-listed rather than reflected: the order is the order they are shown
    /// in, grouped by what they affect, which no derive knows. A field missing
    /// here is uneditable, so adding one to `Theme` means adding it here too.
    pub fn editable_colors(&mut self) -> Vec<(&'static str, &mut String)> {
        vec![
            ("Accent", &mut self.primary),
            ("On accent", &mut self.on_primary),
            ("Panel", &mut self.panel_fill),
            ("Window", &mut self.window_fill),
            ("Faint fill", &mut self.faint_bg_color),
            ("Deep fill", &mut self.extreme_bg_color),
            ("Grid A", &mut self.grid_color_even),
            ("Grid B", &mut self.grid_color_odd),
            ("Origin axes", &mut self.origin_color),
            ("Mesh edge", &mut self.mesh_edge),
            ("Mesh vertex", &mut self.mesh_vertex),
            ("Mesh vertex (selected)", &mut self.mesh_vertex_selected),
            ("Hitbox outline", &mut self.hitbox_outline),
            ("Hitbox fill", &mut self.hitbox_fill),
            ("Point marker", &mut self.point_marker),
            ("Outline (hover)", &mut self.outline_hover),
            ("Outline (selected)", &mut self.outline_selected),
            ("Channel · translate", &mut self.channel_translate),
            ("Channel · rotate", &mut self.channel_rotate),
            ("Channel · scale", &mut self.channel_scale),
            ("Channel · shear", &mut self.channel_shear),
            ("Event marker", &mut self.event_marker),
        ]
    }

    /// Where user themes live: beside the config, one JSON file each.
    pub fn user_dir() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("", "", "ankhimate")
            .map(|dirs| dirs.config_dir().join("themes"))
    }

    /// Write this theme to the user theme directory, returning where it landed.
    ///
    /// The filename comes from the name, lowercased with runs of non-alphanumerics
    /// collapsed to underscores — so "My Theme 2" and "my-theme-2" are the same
    /// file rather than two entries that look identical in the picker.
    pub fn save_to_disk(&self) -> std::io::Result<std::path::PathBuf> {
        let dir = Self::user_dir()
            .ok_or_else(|| std::io::Error::other("no config directory on this platform"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", slug(&self.name)));
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Built-in themes plus whatever the user has saved.
    ///
    /// A user theme with a built-in's name replaces it: editing "Nord" and saving
    /// should give you your Nord, not two of them.
    pub fn load_all_with_user() -> Vec<Theme> {
        let mut all = Self::load_all();
        let Some(dir) = Self::user_dir() else {
            return all;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return all;
        };
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // A malformed theme file is skipped rather than fatal: a stray file in
            // a config directory must not stop the editor from starting.
            let Some(theme) = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| serde_json::from_str::<Theme>(&text).ok())
            else {
                continue;
            };
            all.retain(|t| t.label() != theme.label());
            all.push(theme);
        }
        all
    }

    pub fn primary(&self) -> Color32 {
        hex_to_color(&self.primary)
    }

    pub fn on_primary(&self) -> Color32 {
        hex_to_color(&self.on_primary)
    }

    pub fn grid_color_even(&self) -> Color32 {
        hex_to_color(&self.grid_color_even)
    }

    pub fn grid_color_odd(&self) -> Color32 {
        hex_to_color(&self.grid_color_odd)
    }

    pub fn origin_color(&self) -> Color32 {
        hex_to_color(&self.origin_color)
    }

    pub fn mesh_edge(&self) -> Color32 {
        hex_to_color(&self.mesh_edge)
    }

    pub fn mesh_vertex(&self) -> Color32 {
        hex_to_color(&self.mesh_vertex)
    }

    pub fn mesh_vertex_selected(&self) -> Color32 {
        hex_to_color(&self.mesh_vertex_selected)
    }

    pub fn hitbox_outline(&self) -> Color32 {
        hex_to_color(&self.hitbox_outline)
    }

    pub fn hitbox_fill(&self) -> Color32 {
        hex_to_color(&self.hitbox_fill)
    }

    pub fn point_marker(&self) -> Color32 {
        hex_to_color(&self.point_marker)
    }

    pub fn outline_hover(&self) -> Color32 {
        hex_to_color(&self.outline_hover)
    }

    pub fn outline_selected(&self) -> Color32 {
        hex_to_color(&self.outline_selected)
    }

    pub fn event_marker(&self) -> Color32 {
        hex_to_color(&self.event_marker)
    }

    /// The colour for an animation channel, by the property's row label.
    ///
    /// Keyed on the label because that is what both the tree and the graph
    /// already have in hand; a shared enum would have to be threaded through the
    /// model for no gain.
    pub fn channel_color(&self, property: &str) -> Color32 {
        match property {
            "translate" => hex_to_color(&self.channel_translate),
            "rotate" => hex_to_color(&self.channel_rotate),
            "scale" => hex_to_color(&self.channel_scale),
            "shear" => hex_to_color(&self.channel_shear),
            // Anything else — colour, attachment, the read-only rows — keeps the
            // panel's text colour rather than borrowing a channel's meaning.
            _ => Color32::GRAY,
        }
    }

    /// A vertex under the cursor: the selected colour, lifted.
    pub fn mesh_vertex_hovered(&self) -> Color32 {
        let c = self.mesh_vertex_selected();
        Color32::from_rgb(
            c.r().saturating_add(40),
            c.g().saturating_add(40),
            c.b().saturating_add(40),
        )
    }

    pub fn apply(&self, ctx: &Context) {
        let mut visuals = Visuals::dark();

        let panel_fill = hex_to_color(&self.panel_fill);
        let window_fill = hex_to_color(&self.window_fill);
        let faint_bg = hex_to_color(&self.faint_bg_color);
        let extreme_bg = hex_to_color(&self.extreme_bg_color);
        let primary = self.primary();
        let on_primary = self.on_primary();

        visuals.panel_fill = panel_fill;
        visuals.window_fill = window_fill;
        visuals.faint_bg_color = faint_bg;
        visuals.extreme_bg_color = extreme_bg;
        visuals.selection.bg_fill = primary;
        visuals.selection.stroke.color = on_primary;

        // Widget fills derived from theme — makes buttons/inputs/dropdowns theme-aware
        // inactive: slightly lighter than the panel so controls are visible
        let widget_bg = faint_bg;
        let widget_bg_hovered = primary.linear_multiply(0.18);
        let widget_bg_active = primary.linear_multiply(0.30);

        visuals.widgets.noninteractive.bg_fill = panel_fill;
        visuals.widgets.noninteractive.weak_bg_fill = panel_fill;
        visuals.widgets.inactive.bg_fill = widget_bg;
        visuals.widgets.inactive.weak_bg_fill = widget_bg;
        visuals.widgets.hovered.bg_fill = widget_bg_hovered;
        visuals.widgets.hovered.weak_bg_fill = widget_bg_hovered;
        visuals.widgets.active.bg_fill = widget_bg_active;
        visuals.widgets.active.weak_bg_fill = widget_bg_active;
        visuals.widgets.open.bg_fill = widget_bg_hovered;
        visuals.widgets.open.weak_bg_fill = widget_bg_hovered;

        // Input text area bg (DragValue, TextEdit inner box)
        visuals.extreme_bg_color = extreme_bg;

        // Borders
        let border_dim = eframe::egui::Stroke::new(1.0, faint_bg);
        let border_accent = eframe::egui::Stroke::new(1.0, primary);
        visuals.widgets.noninteractive.bg_stroke = border_dim;
        visuals.widgets.inactive.bg_stroke = border_dim;
        visuals.widgets.hovered.bg_stroke = border_accent;
        visuals.widgets.active.bg_stroke = border_accent;
        visuals.window_stroke = border_dim;

        // Consistent rounding
        let r = eframe::egui::epaint::CornerRadius::same(4);
        visuals.widgets.noninteractive.corner_radius = r;
        visuals.widgets.inactive.corner_radius = r;
        visuals.widgets.hovered.corner_radius = r;
        visuals.widgets.active.corner_radius = r;
        visuals.widgets.open.corner_radius = r;

        ctx.set_visuals(visuals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled theme must carry mesh colours. A missing entry falls back
    /// to the serde default, which is off-palette — cheaper to catch here than
    /// to notice by eye months later.
    #[test]
    fn bundled_themes_define_their_own_mesh_colors() {
        for theme in Theme::load_all() {
            assert_ne!(theme.mesh_edge, default_mesh_edge(), "{}", theme.name);
            assert_ne!(theme.mesh_vertex, default_mesh_vertex(), "{}", theme.name);
            assert_ne!(
                theme.mesh_vertex_selected,
                default_mesh_vertex_selected(),
                "{}",
                theme.name
            );
        }
    }

    /// A theme written before mesh colours existed must still load.
    #[test]
    fn a_theme_without_mesh_colors_still_loads() {
        // `##` delimiters: hex colours contain `"#`, which would close a plain
        // raw string mid-literal.
        let json = r##"{"name":"old","primary":"#ffffff","on_primary":"#000000",
            "panel_fill":"#111111","window_fill":"#222222","faint_bg_color":"#333333",
            "extreme_bg_color":"#000000","grid_color_even":"#111111","grid_color_odd":"#222222",
            "origin_color":"#00ff00"}"##;
        let theme: Theme = serde_json::from_str(json).unwrap();
        assert_ne!(theme.mesh_vertex(), Color32::BLACK);
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    /// Two names that differ only in punctuation must land on one file, or the
    /// picker fills with entries that look identical.
    #[test]
    fn slugs_collapse_punctuation() {
        assert_eq!(slug("My Theme 2"), "my_theme_2");
        assert_eq!(slug("my-theme-2"), "my_theme_2");
        assert_eq!(slug("  Nord  "), "nord");
    }

    #[test]
    fn a_nameless_theme_still_gets_a_filename() {
        assert_eq!(slug("***"), "theme");
        assert_eq!(slug(""), "theme");
    }

    /// Every editable colour has to survive the round trip through the swatch,
    /// which reads hex and writes hex.
    #[test]
    fn editable_colours_are_all_parseable_hex() {
        let mut theme = Theme::default();
        for (label, hex) in theme.editable_colors() {
            let color = hex_to_color(hex);
            assert!(
                color != Color32::BLACK || hex.starts_with("#00000"),
                "{label} did not parse: {hex}"
            );
        }
    }

    /// A theme saved by the settings window has to load back as itself.
    #[test]
    fn a_theme_round_trips_through_json() {
        let theme = Theme {
            name: "Test".into(),
            primary: "#123456ff".into(),
            ..Default::default()
        };
        let json = serde_json::to_string(&theme).unwrap();
        let back: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(back.label(), "Test");
        assert_eq!(back.primary, "#123456ff");
    }
}
