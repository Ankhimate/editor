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

fn hex_to_color(hex: &str) -> Color32 {
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
