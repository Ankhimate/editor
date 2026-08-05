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
pub const CARD_RADIUS: u8 = 8;
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
        // Round the card, by carving rather than by filling.
        //
        // Neither end of the card is ours to fill. egui_tiles paints the tab
        // strip as a plain rectangle before this pane runs, and the panels paint
        // their own backgrounds across the body afterwards — the viewport's
        // checkerboard, the dopesheet's sheet. A rounded fill from here is
        // squared off at the top by the first and at the bottom by the second.
        //
        // So each corner is cut back after everything else has drawn: the desk
        // colour over the corner square, then a quarter-disc of card colour put
        // back inside it. Clipping a full circle to the square gives the quarter,
        // and the result does not care what painted underneath.
        let radius = CARD_RADIUS as f32;
        let card =
            egui::Rect::from_min_max(egui::pos2(body.min.x, body.min.y - TAB_BAR_H), body.max);

        // A layer painter, not `ui.painter()`.
        //
        // `Painter::with_clip_rect` *intersects* with the clip it already has,
        // and a pane's clip is its body — which stops below the tab strip. Every
        // shape aimed at the top of the card was therefore clipped to nothing,
        // which is why the bottom corners rounded and the top two never did, and
        // why the outline only ever appeared along three sides.
        let painter = ui.ctx().layer_painter(ui.layer_id());

        for (corner, centre) in [
            (card.left_top(), egui::vec2(radius, radius)),
            (
                egui::pos2(card.right() - radius, card.top()),
                egui::vec2(0.0, radius),
            ),
            (
                egui::pos2(card.left(), card.bottom() - radius),
                egui::vec2(radius, 0.0),
            ),
            (
                egui::pos2(card.right() - radius, card.bottom() - radius),
                egui::vec2(0.0, 0.0),
            ),
        ] {
            let square = egui::Rect::from_min_size(corner, egui::Vec2::splat(radius));
            let painter = painter.with_clip_rect(square);
            painter.rect_filled(square, 0, self.theme.window_background());
            painter.circle_filled(corner + centre, radius, ui.visuals().panel_fill);
        }

        // The card's outline, drawn *after* the panel's content and reaching back
        // up over the tab strip so strip and body enclose as one object.
        //
        // After, because the viewport paints its checkerboard across its whole
        // rect and the dopesheet paints its sheet — an outline drawn first is
        // painted over by the very panels that need it most, which is why the
        // cards looked stuck together with a 10px gap between them.
        painter.with_clip_rect(card.expand(2.0)).rect_stroke(
            card,
            CARD_RADIUS,
            egui::Stroke::new(1.0, self.theme.card_border()),
            egui::StrokeKind::Inside,
        );

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

    /// What fills the gap between two cards.
    ///
    /// egui_tiles paints the resize separator as a stroke *as wide as the gap*,
    /// and its default colour is `tab_bar_color` — which this behaviour sets to
    /// the panel colour so a tab strip reads as the top of a card. The two
    /// together painted every gap in card colour, so cards 10px apart looked
    /// welded together with no gap at all.
    ///
    /// Idle fills the gap with the desk behind the cards, which is what a gap is.
    /// Hovering and dragging keep the accent, because a separator you are about
    /// to drag has to announce itself.
    fn resize_stroke(&self, style: &egui::Style, state: egui_tiles::ResizeState) -> egui::Stroke {
        match state {
            egui_tiles::ResizeState::Idle => {
                egui::Stroke::new(self.gap_width(style), self.theme.window_background())
            }
            egui_tiles::ResizeState::Hovering => {
                egui::Stroke::new(self.gap_width(style), self.theme.card_border())
            }
            egui_tiles::ResizeState::Dragging => {
                egui::Stroke::new(self.gap_width(style), style.visuals.selection.bg_fill)
            }
        }
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

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// The gap between cards is laid out, not painted — `gap_width` shrinks each
    /// child's rect, and egui_tiles re-runs layout every frame. Asserted here
    /// because "the panels look stuck together" and "there is no gap" are two
    /// different bugs with two different fixes, and a screenshot cannot tell them
    /// apart when both sides of the gap are the same near-black.
    #[test]
    #[allow(deprecated)]
    fn sibling_cards_are_separated_by_the_gap() {
        use crate::app_state::AppState;
        use egui_tiles::{Tile, Tiles, Tree};

        let mut tiles = Tiles::default();
        let a = tiles.insert_pane(Tab::Canvas);
        let b = tiles.insert_pane(Tab::Hierarchy);
        let left = tiles.insert_tab_tile(vec![a]);
        let right = tiles.insert_tab_tile(vec![b]);
        let root = tiles.insert_horizontal_tile(vec![left, right]);
        let mut tree = Tree::new("test", root, tiles);

        let mut state = AppState::default();
        let theme = crate::theme::Theme::default();
        let grid = crate::config::GridSettings::default();
        let fonts = crate::config::FontSettings::default();

        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut behavior = AppBehavior {
                    state: &mut state,
                    theme: &theme,
                    grid: &grid,
                    fonts: &fonts,
                };
                tree.ui(&mut behavior, ui);
            });
        });

        let (left_rect, right_rect) = (
            tree.tiles.rect(left).expect("left laid out"),
            tree.tiles.rect(right).expect("right laid out"),
        );
        let gap = right_rect.min.x - left_rect.max.x;
        assert!(
            (gap - CARD_GAP).abs() < 0.5,
            "cards are {gap}px apart, expected {CARD_GAP}"
        );
        assert!(matches!(tree.tiles.get(left), Some(Tile::Container(_))));
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    /// `Painter::with_clip_rect` intersects; it does not replace.
    ///
    /// Pinned because it cost four rounds of "still squared". A pane's clip stops
    /// at its body, so anything aimed at the tab strip above — the top corners,
    /// the top edge of the card outline — clipped to nothing and simply never
    /// appeared. The card chrome uses a layer painter for exactly this reason,
    /// and a future refactor that "simplifies" it back to `ui.painter()` would
    /// silently lose the top of every card again.
    #[test]
    fn a_rect_above_the_pane_body_clips_to_nothing() {
        let body = egui::Rect::from_min_max(egui::pos2(0.0, 100.0), egui::pos2(500.0, 400.0));
        let top_corner = egui::Rect::from_min_size(
            egui::pos2(body.min.x, body.min.y - TAB_BAR_H),
            egui::Vec2::splat(CARD_RADIUS as f32),
        );
        assert!(
            !top_corner.intersects(body),
            "the top corner sits in the tab strip, outside the pane"
        );
        assert!(
            top_corner.intersect(body).is_negative(),
            "so intersecting with the pane's clip leaves nothing to paint"
        );
    }
}
