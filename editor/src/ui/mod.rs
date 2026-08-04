pub mod animations;
pub mod assets;
pub mod atlas;
pub mod canvas;
pub mod constraints;
pub mod draw_order;
pub mod events;
pub mod inspector;
pub mod psd_import;
pub mod skins;
pub mod startup;
pub mod timeline;
pub mod toolbar;
pub mod trace;
pub mod tree;
pub mod uv;

use eframe::egui;
use egui_tiles::{Behavior, TileId, UiResponse};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Canvas,
    Inspector,
    Hierarchy,
    Timeline,
    DrawOrder,
    Assets,
    Skins,
    Animations,
    Events,
    Constraints,
}

impl Tab {
    /// Every pane, in the order the View menu lists them.
    ///
    /// One list rather than a menu that has to be edited whenever a pane is
    /// added — a View menu missing the pane you are looking for is worse than no
    /// View menu, because it reads as "that panel does not exist".
    pub const ALL: [Tab; 10] = [
        Tab::Canvas,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Timeline,
        Tab::Animations,
        Tab::Events,
        Tab::Constraints,
        Tab::Assets,
        Tab::DrawOrder,
        Tab::Skins,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Canvas => "Viewport",
            Tab::Inspector => "Properties",
            Tab::Hierarchy => "Hierarchy",
            Tab::Timeline => "Timeline",
            Tab::DrawOrder => "Draw Order",
            Tab::Assets => "Assets",
            Tab::Skins => "Skins",
            Tab::Animations => "Animations",
            Tab::Events => "Events",
            Tab::Constraints => "Constraints",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::Canvas => egui_phosphor::fill::MONITOR,
            Tab::Inspector => egui_phosphor::fill::SLIDERS,
            Tab::Hierarchy => egui_phosphor::fill::TREE_STRUCTURE,
            Tab::Timeline => egui_phosphor::fill::FILM_STRIP,
            Tab::DrawOrder => egui_phosphor::fill::STACK,
            Tab::Assets => egui_phosphor::fill::IMAGES,
            Tab::Skins => egui_phosphor::fill::T_SHIRT,
            Tab::Animations => egui_phosphor::fill::FILM_SLATE,
            Tab::Events => egui_phosphor::fill::FLAG,
            Tab::Constraints => egui_phosphor::fill::LINK,
        }
    }
}

pub struct AppBehavior<'a> {
    pub state: &'a mut crate::app_state::AppState,
    pub theme: &'a crate::theme::Theme,
}

impl<'a> Behavior<Tab> for AppBehavior<'a> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut Tab) -> UiResponse {
        // Consistent inner margin for all non-canvas panels
        let margin = egui::Margin::same(8);
        match pane {
            Tab::Canvas => {
                canvas::ui(ui, self.state, self.theme);
            }
            Tab::Inspector => {
                egui::ScrollArea::vertical()
                    .id_salt("inspector_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            inspector::ui(ui, self.state);
                        });
                    });
            }
            Tab::Hierarchy => {
                egui::ScrollArea::vertical()
                    .id_salt("hierarchy_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            tree::ui(ui, self.state);
                        });
                    });
            }
            Tab::Timeline => {
                // The dopesheet manages its own 2D layout and scrolling, so it is
                // not wrapped in a ScrollArea like the other panels.
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        timeline::ui(ui, self.state);
                    });
            }
            Tab::DrawOrder => {
                egui::ScrollArea::vertical()
                    .id_salt("draw_order_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            draw_order::ui(ui, self.state);
                        });
                    });
            }
            Tab::Assets => {
                egui::ScrollArea::vertical()
                    .id_salt("assets_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            assets::ui(ui, self.state);
                        });
                    });
            }
            Tab::Skins => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    skins::ui(ui, self.state);
                });
            }
            Tab::Animations => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    animations::ui(ui, self.state);
                });
            }
            Tab::Events => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    events::ui(ui, self.state);
                });
            }
            Tab::Constraints => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    constraints::ui(ui, self.state);
                });
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &Tab) -> egui::WidgetText {
        pane.title().into()
    }

    fn drag_preview_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.selection.bg_fill.linear_multiply(0.5)
    }

    fn drag_preview_stroke(&self, visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::new(2.0, visuals.selection.bg_fill)
    }

    fn paint_drag_preview(
        &self,
        visuals: &egui::Visuals,
        painter: &egui::Painter,
        _parent_rect: Option<egui::Rect>,
        preview_rect: egui::Rect,
    ) {
        painter.rect_filled(preview_rect, 6.0, self.drag_preview_color(visuals));
        painter.rect_stroke(
            preview_rect,
            6.0,
            self.drag_preview_stroke(visuals),
            egui::StrokeKind::Inside,
        );
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.extreme_bg_color
    }

    fn tab_bg_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Tab>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            visuals.panel_fill
        } else {
            visuals.extreme_bg_color
        }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Tab>,
        _tile_id: TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn tab_text_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Tab>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            visuals.selection.bg_fill
        } else {
            visuals.text_color().linear_multiply(0.6)
        }
    }

    fn tab_bar_hline_stroke(&self, visuals: &egui::Visuals) -> egui::Stroke {
        visuals.widgets.noninteractive.bg_stroke
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}
