pub mod animations;
pub mod assets;
pub mod atlas;
pub mod canvas;
pub mod constraints;
pub mod draw_order;
pub mod events;
pub mod icon_font;
pub mod icons;
pub mod inspector;
pub mod psd_import;
pub mod settings;
pub mod skins;
pub mod slot_editor;
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
    Graph,
    DrawOrder,
    Assets,
    Skins,
    SlotEditor,
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
    pub const ALL: [Tab; 12] = [
        Tab::Canvas,
        Tab::SlotEditor,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Timeline,
        Tab::Graph,
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
            Tab::Timeline => "Dopesheet",
            Tab::Graph => "Graph",
            Tab::DrawOrder => "Draw Order",
            Tab::Assets => "Assets",
            Tab::Skins => "Skins",
            Tab::SlotEditor => "Slot Editor",
            Tab::Animations => "Animations",
            Tab::Events => "Events",
            Tab::Constraints => "Constraints",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tab::Canvas => crate::ui::icons::VIEWPORT,
            Tab::Inspector => crate::ui::icons::PROPERTIES,
            Tab::Hierarchy => crate::ui::icons::IK,
            Tab::Timeline => crate::ui::icons::DOPESHEET,
            Tab::Graph => crate::ui::icons::GRAPH,
            Tab::DrawOrder => crate::ui::icons::DRAW_ORDER,
            Tab::Assets => crate::ui::icons::ASSETS,
            Tab::Skins => crate::ui::icons::SKIN,
            Tab::SlotEditor => crate::ui::icons::SLOT_EDITOR,
            Tab::Animations => crate::ui::icons::ANIMATIONS,
            Tab::Events => crate::ui::icons::EVENTS,
            Tab::Constraints => crate::ui::icons::CONSTRAINT,
        }
    }
}

pub struct AppBehavior<'a> {
    pub state: &'a mut crate::app_state::AppState,
    pub theme: &'a crate::theme::Theme,
    /// Viewport checker settings, which live in `Config` rather than in the
    /// document: a grid size that travelled in a `.ankh` would fight whoever
    /// opened it next.
    pub grid: &'a crate::config::GridSettings,
    pub fonts: &'a crate::config::FontSettings,
}

/// Corner radius of a panel card.
///
/// Deliberately small. A pane paints its own background over its whole rect —
/// the viewport's checkerboard reaches every corner — so the card's rounding can
/// only be drawn as an outline on top, and any radius large enough to notice
/// leaves a square of panel colour outside the curve. Four pixels reads as a
/// softened corner without exposing that.
pub const CARD_RADIUS: u8 = 4;
/// Gap between cards, and between the outermost cards and the window edge.
pub const CARD_GAP: f32 = 10.0;
/// Height of a card's tab strip. Pinned rather than left to the default because
/// the card frame is drawn from the pane, which has to reach back up over the
/// strip to enclose it.
pub const TAB_BAR_H: f32 = 26.0;

impl<'a> Behavior<Tab> for AppBehavior<'a> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: TileId, pane: &mut Tab) -> UiResponse {
        // The card body. The window behind is the deep background, so anything
        // not painted here reads as a gap between cards — which is exactly what
        // the gaps are.
        let body = ui.max_rect();
        ui.painter().rect_filled(
            body,
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: CARD_RADIUS,
                se: CARD_RADIUS,
            },
            ui.visuals().panel_fill,
        );
        // The card's outline, reaching back up over the tab strip so the strip
        // and the body read as one object. Drawn here rather than from
        // `paint_on_top_of_tile` because a pane always runs, and because two of
        // these panes paint the window's own colour across their bodies — the
        // viewport's checkerboard, the dopesheet's sheet — so without an edge
        // there is nothing to tell card from gap.
        let card =
            egui::Rect::from_min_max(egui::pos2(body.min.x, body.min.y - TAB_BAR_H), body.max);
        ui.painter().with_clip_rect(card.expand(2.0)).rect_stroke(
            card,
            CARD_RADIUS,
            egui::Stroke::new(1.0, self.theme.card_border()),
            egui::StrokeKind::Inside,
        );
        // Consistent inner margin for all non-canvas panels
        let margin = egui::Margin::same(8);
        match pane {
            Tab::Canvas => {
                canvas::ui(ui, self.state, self.theme, self.grid);
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
                            tree::ui(ui, self.state, self.fonts);
                        });
                    });
            }
            Tab::Timeline => {
                // The dopesheet manages its own 2D layout and scrolling, so it is
                // not wrapped in a ScrollArea like the other panels.
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        timeline::dopesheet(ui, self.state, self.theme, self.fonts);
                    });
            }
            Tab::Graph => {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        timeline::graph_view(ui, self.state, self.theme, self.fonts);
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
            Tab::SlotEditor => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    slot_editor::ui(ui, self.state);
                });
            }
            Tab::Animations => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    animations::ui(ui, self.state);
                });
            }
            Tab::Events => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    events::ui(ui, self.state, self.theme);
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

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        CARD_GAP
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        TAB_BAR_H
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        // The tab strip is the top of the card, not a separate bar, so it takes
        // the panel colour rather than the window's.
        visuals.panel_fill
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

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        // No rule under the tabs: the card's own outline already separates it
        // from what is around it, and a second line inside reads as clutter.
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}
