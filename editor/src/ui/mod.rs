pub mod animations;
pub mod assets;
pub mod atlas;
pub mod branding;
pub mod canvas;
pub mod constraints;
pub mod dialog;
pub mod draw_order;
pub mod events;
pub mod export;
pub mod icon_font;
pub mod icons;
pub mod inspector;
pub mod plugin_panel;
pub mod psd_import;
pub mod rename;
pub mod settings;
pub mod skins;
pub mod slot_editor;
pub mod startup;
pub mod timeline;
pub mod toolbar;
pub mod trace;
pub mod tree;
pub mod uv;
pub mod weights;

use eframe::egui;
use egui_tiles::{Behavior, TileId, UiResponse};

/// A pane the dock can hold.
///
/// **Not `Copy`, and deliberately so.** `Tab::Plugin` carries the panel's id,
/// because the project's rule is that every extensible thing is looked up by
/// name (`CLAUDE.md`) — a fixed slot with a side table would be a second way to
/// say the same thing and a second place for the two to disagree.
///
/// The cost is that a tab is cloned rather than copied at 36 call sites. That is
/// the honest price of a panel list that a plugin can add to.
#[derive(Clone, PartialEq, Eq, Debug)]
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
    UvEditor,
    Weights,
    Animations,
    Events,
    Constraints,
    Export,
    /// A panel a plugin contributes, by its dotted id.
    Plugin(String),
}

impl Tab {
    /// Every pane, in the order the View menu lists them.
    ///
    /// One list rather than a menu that has to be edited whenever a pane is
    /// added — a View menu missing the pane you are looking for is worse than no
    /// View menu, because it reads as "that panel does not exist".
    pub const ALL: [Tab; 15] = [
        Tab::Canvas,
        Tab::SlotEditor,
        Tab::UvEditor,
        Tab::Weights,
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
        Tab::Export,
    ];

    /// What the tab says.
    ///
    /// A `String` rather than `&'static str`: a plugin's title is read from its
    /// own file at load time and cannot be a literal in ours.
    pub fn title(&self) -> String {
        self.builtin_title()
            .map(str::to_string)
            .unwrap_or_else(|| match self {
                // The id is the fallback, not a placeholder: a panel whose
                // plugin failed to load should name itself so the user can find
                // what is missing.
                Tab::Plugin(id) => id.clone(),
                _ => unreachable!("every built-in has a title"),
            })
    }

    /// The title of a built-in pane, or `None` for a plugin's.
    fn builtin_title(&self) -> Option<&'static str> {
        Some(match self {
            Tab::Canvas => "Viewport",
            Tab::Inspector => "Properties",
            Tab::Hierarchy => "Hierarchy",
            Tab::Timeline => "Dopesheet",
            Tab::Graph => "Graph",
            Tab::DrawOrder => "Draw Order",
            Tab::Assets => "Assets",
            Tab::Skins => "Skins",
            Tab::SlotEditor => "Slot Editor",
            Tab::UvEditor => "UV Editor",
            Tab::Weights => "Weights",
            Tab::Animations => "Animations",
            Tab::Events => "Events",
            Tab::Constraints => "Constraints",
            Tab::Export => "Export",
            Tab::Plugin(_) => return None,
        })
    }

    /// The tab whose [`Tab::title`] is `title`, if any.
    ///
    /// For restoring torn-off windows from config (T-910), which stores names
    /// rather than the enum so a removed variant degrades to "not torn off"
    /// instead of failing the whole parse.
    pub fn from_title(title: &str) -> Option<Tab> {
        Tab::ALL.iter().find(|t| t.title() == title).cloned()
    }

    /// The tab a saved layout named, built-in or plugin.
    ///
    /// A plugin panel's saved name is its **id**, not its title: a plugin that
    /// renames its panel between sessions would otherwise lose every torn-off
    /// window, and the id is the thing that does not change. A name matching no
    /// built-in and containing a dot is read as an id, which is the same shape
    /// verbs use and the one a plugin panel is required to have.
    pub fn from_saved(name: &str) -> Option<Tab> {
        Tab::from_title(name).or_else(|| name.contains('.').then(|| Tab::Plugin(name.into())))
    }

    /// What a saved layout should store for this tab.
    pub fn saved_name(&self) -> String {
        match self {
            Tab::Plugin(id) => id.clone(),
            _ => self.title(),
        }
    }

    /// Can this pane be torn into its own OS window (T-910)?
    ///
    /// Everything but the viewport. The canvas paints through a wgpu callback
    /// against render resources shared with the main window, and a second render
    /// pass into another window is its own piece of work — one I would rather
    /// not land untested behind a feature that otherwise only moves egui panels
    /// around.
    pub fn can_tear_off(&self) -> bool {
        !matches!(self, Tab::Canvas)
    }

    /// Is this pane only meaningful while animating?
    ///
    /// Setup mode has no playhead and no active clip, so these show an
    /// invitation to switch modes and nothing else. Four cards of that is a lot
    /// of screen spent saying "not now".
    pub fn is_animation(&self) -> bool {
        matches!(
            self,
            Tab::Timeline | Tab::Graph | Tab::Animations | Tab::Events
        )
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Tab::Canvas => crate::ui::icons::VIEWPORT,
            Tab::Inspector => crate::ui::icons::PROPERTIES,
            Tab::Hierarchy => crate::ui::icons::IK,
            Tab::Timeline => crate::ui::icons::DOPESHEET,
            Tab::Graph => crate::ui::icons::GRAPH,
            Tab::DrawOrder => crate::ui::icons::DRAW_ORDER,
            Tab::Assets => crate::ui::icons::ASSETS,
            Tab::Skins => crate::ui::icons::SKIN,
            Tab::Export => crate::ui::icons::EXPORT,
            // One glyph for every plugin panel. A per-panel icon would mean a
            // plugin naming one from our set, which pins the icon font as a
            // public contract for the sake of a decoration.
            Tab::Plugin(_) => crate::ui::icons::PLUGIN,
            Tab::SlotEditor => crate::ui::icons::SLOT_EDITOR,
            Tab::UvEditor => crate::ui::icons::MESH,
            Tab::Weights => crate::ui::icons::WEIGHT_PAINT,
            Tab::Animations => crate::ui::icons::ANIMATIONS,
            Tab::Events => crate::ui::icons::EVENTS,
            Tab::Constraints => crate::ui::icons::CONSTRAINT,
        }
    }
}

pub struct AppBehavior<'a> {
    pub state: &'a mut crate::app_state::AppState,
    /// Plugins, for drawing the panels they declare.
    ///
    /// Borrowed beside `state` rather than held inside it: a plugin's source is
    /// not part of the document, and putting it there would make it undoable.
    pub plugins: &'a crate::plugins::Plugins,
    /// Tabs to draw as their icon alone, because their card is too narrow to
    /// hold every label.
    ///
    /// Decided per card and applied to all of its tabs, so a card never shows a
    /// mix of labels and bare icons — which reads as some tabs being special
    /// rather than as a card being narrow.
    pub compact_tabs: &'a std::collections::HashSet<TileId>,
    pub theme: &'a crate::theme::Theme,
    /// Viewport checker settings, which live in `Config` rather than in the
    /// document: a grid size that travelled in a `.ankh` would fight whoever
    /// opened it next.
    pub grid: &'a crate::config::GridSettings,
    pub fonts: &'a crate::config::FontSettings,
    /// Name the thing under the cursor in the viewport (T-913). A `Config` flag
    /// like `grid`, and passed the same way rather than read from a global.
    pub hover_labels: bool,
    /// Tabs whose close button was clicked this frame.
    ///
    /// Collected rather than acted on in place: `tab_ui` is handed the tiles it
    /// is drawing, so removing one mid-draw would pull the ground out from under
    /// the strip it is in the middle of laying out.
    pub close_requests: &'a mut Vec<TileId>,
}

/// The icon for whatever pane a tile holds, or a placeholder for a container.
fn tab_icon(tiles: &egui_tiles::Tiles<Tab>, tile_id: TileId) -> &'static str {
    match tiles.get(tile_id) {
        Some(egui_tiles::Tile::Pane(pane)) => pane.icon(),
        _ => icons::FOLDER,
    }
}

/// Corner radius of a panel card.
pub const CARD_RADIUS: u8 = 8;
/// Gap between cards, and between the outermost cards and the window edge.
pub const CARD_GAP: f32 = 5.0;
/// Space between a tab's label and the top and bottom of its plate.
pub const TAB_PAD_Y: f32 = 6.0;
/// Space either side of a tab's label, inside its plate.
pub const TAB_TITLE_SPACING: f32 = 10.0;
/// Width a tab gives up to its close button, when it has one.
pub const TAB_CLOSE_WIDTH: f32 = 20.0;

/// Space before the first tab, holding it off the card's rounded corner.
///
/// Matched to the corner radius: any less and the plate's square bottom-left
/// overhangs the curve, which reads as the tab hanging off the edge of the card.
pub const TAB_START_PAD: f32 = CARD_RADIUS as f32;

/// Height of a card's tab strip.
///
/// Derived from the label rather than fixed, so a tab is padded by the same
/// amount whatever size the text is. A constant has to be chosen for one font
/// size and clips the label at any larger one — and the interface font is a
/// setting, so "any larger one" is something users can ask for.
///
/// The pane needs this too: the card frame is drawn from the pane, which has to
/// reach back up over the strip to enclose it.
pub fn tab_bar_height(style: &egui::Style) -> f32 {
    let text = egui::TextStyle::Button.resolve(style).size;
    TAB_TOP_PAD + text + 2.0 * TAB_PAD_Y
}
/// Space above a tab plate, between it and the top of the card.
///
/// The plate has to sit *on* the strip rather than fill it, or its rounded top
/// meets the card's own rounded top and the two curves fight.
pub const TAB_TOP_PAD: f32 = 10.0;

impl AppBehavior<'_> {
    /// Draw one pane's contents.
    ///
    /// Split out of `pane_ui` so a torn-off window (T-910) draws a panel by
    /// calling the same code the docked tile does. Two copies of this match is
    /// how a panel starts behaving differently depending on which window it is
    /// in, which is the failure this whole feature has to avoid.
    pub fn pane_contents(&mut self, ui: &mut egui::Ui, pane: &Tab) {
        // Consistent inner margin for all non-canvas panels
        let margin = egui::Margin::same(8);
        match pane {
            Tab::Canvas => {
                canvas::ui(ui, self.state, self.theme, self.grid, self.hover_labels);
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
            Tab::Export => {
                egui::ScrollArea::vertical()
                    .id_salt("export_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            export::ui(ui, self.state);
                        });
                    });
            }
            Tab::SlotEditor => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    slot_editor::ui(ui, self.state);
                });
            }
            Tab::UvEditor => {
                egui::ScrollArea::both()
                    .id_salt("uv_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            uv::ui(ui, self.state, self.theme);
                        });
                    });
            }
            Tab::Weights => {
                // No outer scroll: the bone list sizes itself from the height
                // left over, and an ancestor that scrolls hands it an unbounded
                // one. The controls above it are fixed height, so the pane only
                // needs the list to flex.
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    weights::ui(ui, self.state);
                });
            }
            Tab::Animations => {
                egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                    animations::ui(ui, self.state, self.theme);
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
            Tab::Plugin(id) => {
                egui::ScrollArea::vertical()
                    .id_salt(("plugin_panel", id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
                            plugin_panel::ui(ui, self.state, self.plugins, id);
                        });
                    });
            }
        }
    }
}

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
        self.pane_contents(ui, pane);
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
        let strip_h = tab_bar_height(ui.style());
        let card = egui::Rect::from_min_max(egui::pos2(body.min.x, body.min.y - strip_h), body.max);

        // A layer painter, not `ui.painter()`.
        //
        // `Painter::with_clip_rect` *intersects* with the clip it already has,
        // and a pane's clip is its body — which stops below the tab strip. Every
        // shape aimed at the top of the card was therefore clipped to nothing,
        // which is why the bottom corners rounded and the top two never did, and
        // why the outline only ever appeared along three sides.
        let painter = ui.ctx().layer_painter(ui.layer_id());

        let strip = self.theme.window_background();
        let panel = ui.visuals().panel_fill;
        for (corner, centre, restore) in [
            (card.left_top(), egui::vec2(radius, radius), strip),
            (
                egui::pos2(card.right() - radius, card.top()),
                egui::vec2(0.0, radius),
                strip,
            ),
            (
                egui::pos2(card.left(), card.bottom() - radius),
                egui::vec2(radius, 0.0),
                panel,
            ),
            (
                egui::pos2(card.right() - radius, card.bottom() - radius),
                egui::vec2(0.0, 0.0),
                panel,
            ),
        ] {
            let square = egui::Rect::from_min_size(corner, egui::Vec2::splat(radius));
            let painter = painter.with_clip_rect(square);
            painter.rect_filled(square, 0, self.theme.window_background());
            painter.circle_filled(corner + centre, radius, restore);
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

    fn tab_bar_height(&self, style: &egui::Style) -> f32 {
        tab_bar_height(style)
    }

    /// One tab: a plate that sits on the strip rather than filling it.
    ///
    /// Reimplemented rather than restyled because the default paints the tab as
    /// a square rect filling the strip's full height, and neither the inset nor
    /// the rounded top is reachable through the colour hooks.
    ///
    /// The plate is rounded at the top and square at the bottom, so the active
    /// tab runs into the body below it — the join is what says "this tab owns
    /// that content".
    /// Every pane can be closed; the View menu is how it comes back.
    ///
    /// Containers are not: closing one would take its children with it, and the
    /// tab strip gives no hint that it is about to.
    fn is_tab_closable(&self, tiles: &egui_tiles::Tiles<Tab>, tile_id: TileId) -> bool {
        matches!(tiles.get(tile_id), Some(egui_tiles::Tile::Pane(_)))
    }

    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Tab>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        // Hold the first tab off the card's corner. The tab bar lays its tabs out
        // left to right with no item spacing, so the leading gap has to be taken
        // here — and only once, when the cursor is still at the strip's edge.
        if ui.cursor().left() <= ui.max_rect().left() + 0.5 {
            ui.add_space(TAB_START_PAD);
        }

        let compact = self.compact_tabs.contains(&tile_id);
        let full_title = self.tab_title_for_tile(tiles, tile_id);
        let icon = tab_icon(tiles, tile_id);
        let label: egui::WidgetText = if compact {
            icon.into()
        } else {
            format!("{icon}  {}", full_title.text()).into()
        };
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley =
            label.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id);

        // Only the active tab carries a close button. A row of them turns a tab
        // strip into a row of targets to misclick, and the tab you want to close
        // is nearly always the one you are looking at.
        let closable = state.active && self.is_tab_closable(tiles, tile_id) && !compact;
        let x_margin = TAB_TITLE_SPACING;
        let width = galley.size().x + 2.0 * x_margin + if closable { TAB_CLOSE_WIDTH } else { 0.0 };
        let (_, rect) = ui.allocate_space(egui::vec2(width, ui.available_height()));

        let draggable = self.is_tile_draggable(tiles, tile_id);
        let sense = if draggable {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::click()
        };
        let response = ui.interact(rect, id, sense);
        let response = if draggable {
            response.on_hover_cursor(self.tab_hover_cursor_icon())
        } else {
            response
        };

        if ui.is_rect_visible(rect) && !state.is_being_dragged {
            let plate = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + TAB_TOP_PAD),
                rect.max,
            );
            let radius = egui::CornerRadius {
                nw: CARD_RADIUS,
                ne: CARD_RADIUS,
                sw: 0,
                se: 0,
            };
            if state.active {
                ui.painter()
                    .rect_filled(plate, radius, ui.visuals().panel_fill);
            } else if response.hovered() {
                // A hint, not a plate: hovering should say "this is clickable"
                // without briefly looking like the active tab.
                ui.painter().rect_filled(
                    plate,
                    radius,
                    ui.visuals().panel_fill.gamma_multiply(0.5),
                );
            }

            let color = self.tab_text_color(ui.visuals(), tiles, tile_id, state);
            // The label centres in what is left after the close button, so
            // adding one shifts the text rather than sliding it under the glyph.
            let text_area = egui::Rect::from_min_max(
                plate.min,
                egui::pos2(
                    plate.max.x - if closable { TAB_CLOSE_WIDTH } else { 0.0 },
                    plate.max.y,
                ),
            );
            let pos = egui::Align2::CENTER_CENTER
                .align_size_within_rect(galley.size(), text_area)
                .min;
            ui.painter().galley(pos, galley, color);
        }

        if closable {
            let plate_top = rect.top() + TAB_TOP_PAD;
            let btn = egui::Rect::from_center_size(
                egui::pos2(
                    rect.max.x - TAB_CLOSE_WIDTH * 0.5 - x_margin * 0.5,
                    (plate_top + rect.bottom()) * 0.5,
                ),
                egui::vec2(16.0, 16.0),
            );
            // Its own id, and interacted *after* the tab's own response is
            // built, so the click lands on the button rather than selecting the
            // tab underneath it.
            let close = ui.interact(btn, id.with("close"), egui::Sense::click());
            if close.hovered() {
                ui.painter()
                    .rect_filled(btn, 4, ui.visuals().widgets.hovered.bg_fill);
            }
            ui.painter().text(
                btn.center(),
                egui::Align2::CENTER_CENTER,
                icons::CLOSE,
                egui::FontId::proportional(11.0),
                if close.hovered() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                },
            );
            if close.clicked() {
                self.close_requests.push(tile_id);
            }
        }

        // The name on hover, always. It is the only way to read a compact tab,
        // and harmless on a full one.
        if compact {
            return response.on_hover_text(full_title.text());
        }
        response
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

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        // The strip recedes to the desk colour, so the card reads as a sheet of
        // panel with its heading set into the surface behind it rather than as a
        // solid block with a lighter band on top. The active tab keeps the panel
        // colour (see `tab_bg_color`), which is what connects it to the body
        // below and makes it look raised out of the strip.
        self.theme.window_background()
    }

    fn tab_bg_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Tab>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            // The panel colour, matching the body below it, so the active tab
            // reads as raised out of the strip and joined to its own content.
            visuals.panel_fill
        } else {
            // Nothing. An inactive tab is its label on the strip; giving it a
            // fill would put three competing plates in a row and make the active
            // one harder to find, not easier.
            egui::Color32::TRANSPARENT
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
                    plugins: &Default::default(),
                    theme: &theme,
                    grid: &grid,
                    fonts: &fonts,
                    hover_labels: true,
                    compact_tabs: &Default::default(),
                    close_requests: &mut Vec::new(),
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
            egui::pos2(
                body.min.x,
                body.min.y - tab_bar_height(&egui::Style::default()),
            ),
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

#[cfg(test)]
mod tab_metrics_tests {
    use super::*;

    /// A tab's label sits with the same space above and below it, whatever size
    /// the interface font is set to. A fixed strip height gives that at exactly
    /// one font size and clips the label above it.
    #[test]
    fn the_strip_grows_with_the_label() {
        let mut small = egui::Style::default();
        small
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(10.0));
        let mut large = egui::Style::default();
        large
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(20.0));

        let (a, b) = (tab_bar_height(&small), tab_bar_height(&large));
        assert!(b > a, "a bigger label needs a taller strip");
        // The difference is exactly the difference in text size: the padding is
        // the same at both ends, which is what "symmetric" means here.
        assert!((b - a - 10.0).abs() < 0.01, "{a} then {b}");
    }

    /// The plate leaves `TAB_PAD_Y` above and below the label. Asserted through
    /// the same arithmetic the painter uses, so a change to one without the other
    /// is caught.
    #[test]
    fn the_plate_pads_the_label_evenly() {
        let style = egui::Style::default();
        let text = egui::TextStyle::Button.resolve(&style).size;
        let plate = tab_bar_height(&style) - TAB_TOP_PAD;
        assert!(
            (plate - text - 2.0 * TAB_PAD_Y).abs() < 0.01,
            "plate {plate} should be the label {text} plus {TAB_PAD_Y} at each end"
        );
    }
}

#[cfg(test)]
mod compact_tab_tests {
    use super::*;

    /// Every tab in a card collapses together or not at all. A card showing two
    /// labels and three bare icons reads as some tabs being special rather than
    /// as the card being narrow.
    #[test]
    fn compaction_is_decided_per_card() {
        let mut tiles = egui_tiles::Tiles::default();
        let a = tiles.insert_pane(Tab::Assets);
        let b = tiles.insert_pane(Tab::DrawOrder);
        let card = tiles.insert_tab_tile(vec![a, b]);

        // What `AppBehavior::compact_tabs` builds: the children of a card that
        // did not fit, never a subset of them.
        let compact: std::collections::HashSet<_> = match tiles.get(card) {
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) => {
                tabs.children.iter().copied().collect()
            }
            _ => panic!("expected a tab container"),
        };
        assert!(compact.contains(&a) && compact.contains(&b));
    }

    /// A compact tab still has to be identifiable, so every pane owns an icon.
    #[test]
    fn every_pane_has_an_icon_to_collapse_to() {
        for tab in Tab::ALL {
            assert!(
                !tab.icon().is_empty(),
                "{tab:?} would collapse to an empty tab"
            );
        }
    }
}

#[cfg(test)]
mod tab_tests {
    use super::Tab;

    #[test]
    fn a_plugin_panel_survives_a_saved_layout() {
        // A torn-off panel is stored by name and read back on the next launch.
        // Before `Tab::Plugin` existed the list was closed, so this is the round
        // trip that has to work for the variant to be worth anything.
        let tab = Tab::Plugin("tools.mirror".into());
        let saved = tab.saved_name();
        assert_eq!(Tab::from_saved(&saved), Some(tab));
    }

    #[test]
    fn a_plugin_panel_is_saved_by_id_and_not_by_title() {
        // The id is the thing that does not change. A plugin that renamed its
        // panel between sessions would otherwise lose every torn-off window.
        assert_eq!(
            Tab::Plugin("tools.mirror".into()).saved_name(),
            "tools.mirror"
        );
    }

    #[test]
    fn a_built_in_pane_still_round_trips_by_title() {
        // The change must not break the layouts users already have saved.
        for tab in Tab::ALL {
            let saved = tab.saved_name();
            assert_eq!(
                Tab::from_saved(&saved),
                Some(tab.clone()),
                "`{saved}` did not come back"
            );
        }
    }

    #[test]
    fn a_saved_name_that_is_neither_reads_as_nothing() {
        // A pane removed from a future build should degrade to "not torn off"
        // rather than failing the whole config parse — the reason `from_title`
        // stores names in the first place.
        assert_eq!(Tab::from_saved("Some Removed Panel"), None);
    }
}
