use crate::app_state::AppState;
use crate::theme;
use crate::ui::{AppBehavior, Tab};
use eframe::egui;
use egui_tiles::{Tiles, Tree};

pub struct AnkhimateApp {
    tree: Tree<Tab>,
    theme: theme::Theme,
    state: AppState,
    available_themes: Vec<theme::Theme>,
    /// Path of the currently open `.ankh`, or `None` for an unsaved document.
    current_path: Option<std::path::PathBuf>,
    /// Transient message shown after a file op (save/open result).
    status: Option<String>,
    /// Preferences that outlive the session: recent files, startup behavior.
    config: crate::config::Config,
    /// Is the startup window up? (T-304)
    show_startup: bool,
    /// Is the settings window up? (T-701)
    show_settings: bool,
    /// Is the bulk-rename dialog up? (T-901)
    show_rename: bool,
    /// Its settings, kept while it is open.
    rename: crate::ui::rename::RenameState,
    /// In-progress name for a new selection set (T-904).
    naming_set: Option<String>,
    /// The program mark. Re-rasterises itself from vector art when the size it
    /// is drawn at changes, so a UI-scale change stays sharp.
    logo: crate::ui::branding::Logo,
}

/// One window control. Returns whether it was clicked.
fn window_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, danger: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 26.0), egui::Sense::click());
    if response.hovered() {
        let fill = if danger {
            ui.visuals().error_fg_color.gamma_multiply(0.75)
        } else {
            ui.visuals().faint_bg_color
        };
        ui.painter().rect_filled(rect, 5, fill);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(14.0),
        if response.hovered() && danger {
            egui::Color32::WHITE
        } else {
            ui.visuals().weak_text_color()
        },
    );
    response.on_hover_text(tooltip).clicked()
}

impl AnkhimateApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_file(cc, None)
    }

    /// Start up, optionally opening a project immediately.
    pub fn with_file(cc: &eframe::CreationContext<'_>, open: Option<std::path::PathBuf>) -> Self {
        // Lucide: one stroke weight on one 24px grid, so a column of glyphs reads
        // as a family rather than as clip art at differing optical weights. The
        // vocabulary lives in `ui::icons`, not scattered across the panels.
        //
        // Appended to the proportional family rather than replacing it: egui
        // falls through to the next font for anything the first cannot draw, so
        // ordinary text keeps its own face and only icon codepoints reach here.
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "lucide".into(),
            egui::FontData::from_static(crate::ui::icon_font::FONT).into(),
        );
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.push("lucide".into());
        }
        cc.egui_ctx.set_fonts(fonts);

        // Text and icons are rasterised at `size × zoom × pixels_per_point`, so
        // the UI scale is the only thing that makes them genuinely sharper rather
        // than bigger-and-still-blocky. Restored before the first frame: changing
        // it later re-rasterises every glyph, which is a visible hitch.
        cc.egui_ctx
            .set_zoom_factor(crate::config::Config::load().ui_scale.clamp(0.5, 3.0));

        // User themes join the built-ins here rather than being loaded lazily:
        // the saved choice may well be one of them, and starting in the wrong
        // theme for a frame is a visible flash.
        let available_themes = theme::Theme::load_all_with_user();
        let saved = crate::config::Config::load();
        let default_theme = saved
            .theme_name
            .as_deref()
            .and_then(|name| available_themes.iter().find(|t| t.label() == name))
            .or_else(|| available_themes.first())
            .cloned()
            .unwrap_or_default();

        default_theme.apply(&cc.egui_ctx);

        if let Some(wgpu_state) = cc.wgpu_render_state.as_ref() {
            let device = &wgpu_state.device;
            let format = wgpu_state.target_format;
            let renderer = crate::renderer::CustomRenderer::new(device, format);
            wgpu_state
                .renderer
                .write()
                .callback_resources
                .insert(renderer);
        }

        let config = crate::config::Config::load();
        let show_startup = !config.skip_startup;

        let mut app = Self {
            theme: default_theme,
            available_themes,
            config,
            show_startup,
            ..Default::default()
        };

        if let Some(path) = open {
            match crate::fileops::open_path(&mut app.state, &path) {
                crate::fileops::FileOutcome::Opened(path) => {
                    app.status = Some(format!("Opened {}", path.display()));
                    app.config.touch_recent(&path);
                    app.current_path = Some(path);
                    // A file was asked for by name; the startup window would
                    // only be in the way.
                    app.show_startup = false;
                }
                crate::fileops::FileOutcome::Error(e) => app.status = Some(e),
                _ => {}
            }
        }
        app
    }
}

impl AnkhimateApp {
    /// Hide the animation panes while building the rig, and bring them back when
    /// animating.
    ///
    /// A card whose tabs are *all* animation panes is hidden whole, rather than
    /// left as an empty frame with no tabs in it. One that mixes them — because
    /// the user docked the dopesheet next to the assets — keeps its card and
    /// loses only those tabs.
    fn apply_mode_visibility(&mut self) {
        use egui_tiles::{Container, Tile};

        let animating = self.state.session.is_animating();

        let panes: Vec<(egui_tiles::TileId, bool)> = self
            .tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                Tile::Pane(pane) => Some((*id, pane.is_animation())),
                _ => None,
            })
            .collect();
        for (id, is_animation) in &panes {
            self.tree.set_visible(*id, animating || !is_animation);
        }

        let cards: Vec<(egui_tiles::TileId, Vec<egui_tiles::TileId>)> = self
            .tree
            .tiles
            .iter()
            .filter_map(|(id, tile)| match tile {
                Tile::Container(Container::Tabs(tabs)) => Some((*id, tabs.children.clone())),
                _ => None,
            })
            .collect();
        for (card, children) in cards {
            let all_animation = !children.is_empty()
                && children.iter().all(|child| {
                    matches!(self.tree.tiles.get(*child), Some(Tile::Pane(p)) if p.is_animation())
                });
            self.tree.set_visible(card, animating || !all_animation);
        }
    }

    /// Tabs that should show their icon alone, keyed by tab tile id.
    ///
    /// egui_tiles' answer to a crowded tab bar is a pair of hardcoded scroll
    /// arrows — not stylable, not overridable, and a poor trade: two clicks to
    /// reach a tab that would have fitted as an icon. Collapsing the labels
    /// keeps every tab reachable in one click, and the name is on hover.
    fn compact_tabs(&self, ctx: &egui::Context) -> std::collections::HashSet<egui_tiles::TileId> {
        use egui_tiles::{Container, Tile};

        let font = egui::TextStyle::Button.resolve(&ctx.global_style());
        let label_width = |text: &str| {
            ctx.fonts_mut(|f| {
                f.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            })
        };
        // Per tab: the label, its side margins, and the leading inset once.
        let padding = 2.0 * crate::ui::TAB_TITLE_SPACING;

        let mut compact = std::collections::HashSet::new();
        for (container_id, tile) in self.tree.tiles.iter() {
            let Tile::Container(Container::Tabs(tabs)) = tile else {
                continue;
            };
            let Some(rect) = self.tree.tiles.rect(*container_id) else {
                continue;
            };
            let needed: f32 = tabs
                .children
                .iter()
                .filter_map(|child| match self.tree.tiles.get(*child) {
                    // The icon rides with the label, so it counts toward whether
                    // the labels fit at all.
                    Some(Tile::Pane(pane)) => {
                        Some(label_width(&format!("{}  {}", pane.icon(), pane.title())) + padding)
                    }
                    _ => None,
                })
                .sum();
            // Plus the close button, which only the active tab shows — but which
            // tab is active changes as you click, and a strip that fits until you
            // select the widest tab and then reflows is worse than one that
            // compacts a little early.
            if needed + crate::ui::TAB_START_PAD + crate::ui::TAB_CLOSE_WIDTH > rect.width() {
                compact.extend(tabs.children.iter().copied());
            }
        }
        compact
    }

    /// The tile id of a pane, if it is still in the tree at all.
    fn find_pane(&self, tab: crate::ui::Tab) -> Option<egui_tiles::TileId> {
        self.tree.tiles.iter().find_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(pane) if *pane == tab => Some(*id),
            _ => None,
        })
    }

    /// Put a pane back into the layout.
    ///
    /// Dropped next to whatever is at the root rather than restored to its
    /// original neighbour: the tree has been rearranged by hand since, and
    /// guessing where it "should" go would fight the arrangement the user made.
    fn add_pane(&mut self, tab: crate::ui::Tab) {
        let pane = self.tree.tiles.insert_pane(tab);
        match self.tree.root() {
            Some(root) => {
                let tab_tile = self.tree.tiles.insert_tab_tile(vec![pane]);
                let new_root = self.tree.tiles.insert_horizontal_tile(vec![root, tab_tile]);
                self.tree.root = Some(new_root);
            }
            None => self.tree.root = Some(pane),
        }
    }

    /// The starting arrangement, also used by View → Reset Layout.
    fn default_layout() -> Tree<Tab> {
        let mut tiles = Tiles::default();

        let canvas = tiles.insert_pane(Tab::Canvas);
        let inspector = tiles.insert_pane(Tab::Inspector);
        let draw_order = tiles.insert_pane(Tab::DrawOrder);
        let assets = tiles.insert_pane(Tab::Assets);
        let skins = tiles.insert_pane(Tab::Skins);
        let tree = tiles.insert_pane(Tab::Hierarchy);
        let timeline = tiles.insert_pane(Tab::Timeline);
        let graph = tiles.insert_pane(Tab::Graph);
        let animations = tiles.insert_pane(Tab::Animations);
        let events = tiles.insert_pane(Tab::Events);
        let constraints = tiles.insert_pane(Tab::Constraints);

        // The slot editor tabs with the viewport, so opening a piece replaces the
        // rig on screen the way a smart object replaces the document. The UV
        // editor joins them: it is the same idea one level down — one attachment,
        // opened on its own to be worked on — and it wants the same room.
        let slot_editor = tiles.insert_pane(Tab::SlotEditor);
        let uv_editor = tiles.insert_pane(Tab::UvEditor);
        let canvas_tab = tiles.insert_tab_tile(vec![canvas, slot_editor, uv_editor]);
        // Properties gets its own tile rather than sharing tabs with Assets:
        // the transform controls are used constantly, and hiding them behind a
        // tab every time the image library is opened is the wrong trade.
        let weights = tiles.insert_pane(Tab::Weights);
        let inspector_tab = tiles.insert_tab_tile(vec![inspector, weights]);
        // Assets and draw order are both "what is in the rig" browsers, so they
        // can share.
        let library_tab = tiles.insert_tab_tile(vec![assets, draw_order, skins, constraints]);
        // Animations and events share a tile with the timeline: all three answer
        // "what is in this clip", and the timeline is where you already are when
        // that question comes up.
        let timeline_group = vec![timeline, graph, animations, events];
        let tree_tab = tiles.insert_tab_tile(vec![tree]);
        let timeline_tab = tiles.insert_tab_tile(timeline_group);

        let right = tiles.insert_vertical_tile(vec![tree_tab, inspector_tab, library_tab]);
        let center_row = tiles.insert_horizontal_tile(vec![canvas_tab, right]);
        let root = tiles.insert_vertical_tile(vec![center_row, timeline_tab]);

        Tree::new("ankhimate_tree", root, tiles)
    }
}

impl Default for AnkhimateApp {
    fn default() -> Self {
        Self {
            tree: Self::default_layout(),
            theme: theme::Theme::default(),
            available_themes: vec![theme::Theme::default()],
            state: AppState::default(),
            current_path: None,
            status: None,
            config: crate::config::Config::default(),
            show_startup: false,
            show_settings: false,
            show_rename: false,
            rename: Default::default(),
            naming_set: None,
            logo: crate::ui::branding::Logo::default(),
        }
    }
}

impl eframe::App for AnkhimateApp {
    /// What the window is cleared to before anything is drawn.
    ///
    /// eframe defaults this to `panel_fill`, which is the *card* colour — so
    /// every pixel no panel covered came out the same shade as the panels, and
    /// the gaps between cards read as no gap at all. It is the deep background
    /// here, which is what a gap is meant to show.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.theme.window_background();
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }

    // TODO(editor): migrate the custom title bar / toolbar chrome off the
    // deprecated `TopBottomPanel` + `allocate_ui_at_rect` APIs to `Panel::top` +
    // `allocate_new_ui`. That is a layout change, not a mechanical rename, so it
    // is deliberately deferred out of the core-remediation tasks (T-10x).
    #[allow(deprecated)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.theme.apply(ctx);

        let mut trigger_undo = false;
        let mut trigger_redo = false;
        let mut file_action: Option<FileAction> = None;
        if ctx.input(|i| i.modifiers.ctrl) {
            if ctx.input(|i| i.key_pressed(egui::Key::Z)) {
                if ctx.input(|i| i.modifiers.shift) {
                    trigger_redo = true;
                } else {
                    trigger_undo = true;
                }
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Y)) {
                trigger_redo = true;
            }
            // File shortcuts: Ctrl+N/O/S, Ctrl+Shift+S.
            if ctx.input(|i| i.key_pressed(egui::Key::N)) {
                file_action = Some(FileAction::New);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::O)) {
                file_action = Some(FileAction::Open);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::S)) {
                file_action = Some(if ctx.input(|i| i.modifiers.shift) {
                    FileAction::SaveAs
                } else {
                    FileAction::Save
                });
            }

            // ── Clipboard (T-209) ────────────────────────────────────────
            // Shift picks the pose variants: Ctrl+C copies bones, Ctrl+Shift+C
            // copies the pose; Ctrl+Shift+V pastes a pose mirrored.
            let typing_now = ctx.memory(|m| m.focused().is_some());
            if !typing_now {
                let (c, v, d, shift) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::C),
                        i.key_pressed(egui::Key::V),
                        i.key_pressed(egui::Key::D),
                        i.modifiers.shift,
                    )
                });
                if c {
                    if shift {
                        self.state.copy_pose();
                    } else {
                        self.state.copy_selection();
                    }
                }
                if v {
                    self.state.paste(shift);
                }
                if d {
                    self.state.duplicate_selection();
                }
            }
        }

        // ── Playback shortcuts (T-202) ───────────────────────────────────
        // Suppressed while a text field has focus so typing a name does not
        // scrub the timeline.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing {
            // ── Mode, tools, keying (T-207) ──────────────────────────────
            let (tab, key_k, v, b, w) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Tab),
                    i.key_pressed(egui::Key::K),
                    i.key_pressed(egui::Key::V),
                    i.key_pressed(egui::Key::B),
                    i.key_pressed(egui::Key::W),
                )
            });
            // Visibility filters. Bare digits rather than a modifier: these get
            // flipped constantly while rigging, and a chord is a chord too many.
            let (hide_art, hide_bones) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Num1),
                    i.key_pressed(egui::Key::Num2),
                )
            });
            if hide_art {
                self.state.session.show_artwork = !self.state.session.show_artwork;
            }
            if hide_bones {
                self.state.session.show_bones = !self.state.session.show_bones;
            }
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Comma)) {
                self.show_settings = !self.show_settings;
            }
            // F2 renames the selection — the key every file manager and 3D tool
            // uses for it, so it needs no discovering. Only with something
            // selected: an empty rename dialog would be a dead end.
            if ctx.input(|i| i.key_pressed(egui::Key::F2))
                && !self.state.session.selected_bones.is_empty()
            {
                self.show_rename = true;
            }
            // M drops a marker at the playhead (T-906) — the same gesture K uses
            // for a key, on the strip above it. Named after the frame it lands
            // on, because an animator marking a pose knows which pose it is and
            // a dialog mid-scrub would break the rhythm; rename is on its
            // right-click menu.
            if ctx.input(|i| i.key_pressed(egui::Key::M) && !i.modifiers.any())
                && let Some(anim) = self.state.session.active_animation
            {
                let fps = self.state.doc.meta.fps.max(1) as f32;
                let frame = (self.state.session.playhead * fps).round() as i64;
                let playhead = self.state.session.playhead;
                self.state
                    .dispatch(Box::new(crate::commands::marker_cmds::AddMarker::new(
                        anim,
                        format!("f{frame}"),
                        playhead,
                    )));
            }
            if tab {
                self.state.toggle_work_mode();
            }
            if key_k {
                // Commits any posed-but-unkeyed bone; a no-op in Setup mode.
                self.state.key_pending_pose();
            }
            {
                use crate::session::Tool;
                let setup = self.state.session.can_edit_structure();
                if v {
                    self.state.session.tool = Tool::Select;
                }
                if b && setup {
                    self.state.session.tool = Tool::CreateBone;
                }
                if w && setup {
                    self.state.session.tool = Tool::WeightPaint;
                }
            }

            // Transform mode for the Select tool's gizmo: T/R/S/H. Guarded on
            // `!ctrl` so Ctrl+S stays Save.
            {
                use crate::session::TransformTool;
                let (t, r, s, h, ctrl, shift) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::T),
                        i.key_pressed(egui::Key::R),
                        i.key_pressed(egui::Key::S),
                        i.key_pressed(egui::Key::H),
                        i.modifiers.ctrl,
                        i.modifiers.shift,
                    )
                });
                // Shift excluded as well as Ctrl: Shift+H is isolation (T-903),
                // and without this the bare-key match would fire Shear too.
                if !ctrl && !shift {
                    if t {
                        self.state.session.active_transform_tool = TransformTool::Translate;
                    }
                    if r {
                        self.state.session.active_transform_tool = TransformTool::Rotate;
                    }
                    if s {
                        self.state.session.active_transform_tool = TransformTool::Scale;
                    }
                    if h {
                        self.state.session.active_transform_tool = TransformTool::Shear;
                    }
                }
                // Shift+H isolates the viewport to the selection, or leaves
                // isolation when there is nothing selected or it is already on
                // (T-903). One key both ways: there is nothing to remember, and
                // no way to end up isolated with no idea which key gets you out.
                if h && shift && !ctrl {
                    if self.state.session.is_isolating() {
                        self.state.session.clear_isolation();
                        self.state.session.set_status("Showing the whole rig");
                    } else {
                        let bones = self.state.session.selected_bones.clone();
                        self.state.session.isolate(&self.state.doc.skeleton, &bones);
                        let n = self.state.session.isolated_bones.len();
                        self.state.session.set_status(if n == 0 {
                            "Select a bone to isolate".to_string()
                        } else {
                            format!("Isolated {n} bone(s) — Shift+H to exit")
                        });
                    }
                }
            }

            let (space, left, right, ctrl) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                    i.modifiers.ctrl,
                )
            });
            if space {
                crate::ui::timeline::toggle_play(&mut self.state);
            }
            if left {
                if ctrl {
                    self.state.jump_key(false);
                } else {
                    self.state.step_frames(-1);
                }
            }
            if right {
                if ctrl {
                    self.state.jump_key(true);
                } else {
                    self.state.step_frames(1);
                }
            }
        }

        // Advance playback by real elapsed time and keep animating while playing.
        let dt = ctx.input(|i| i.stable_dt);
        if self.state.advance_playback(dt) {
            ctx.request_repaint();
        }

        // ── Title bar ────────────────────────────────────────────────────
        egui::TopBottomPanel::top("top_bar")
            // The desk colour, with no rule beneath it: the title bar is part of
            // the window frame, not a card, and a line under it would draw a
            // fourth horizontal edge into a layout that already has the cards'
            // own outlines doing that job.
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(self.theme.window_background()))
            .show(ctx, |ui| {
                let h = 32.0;
                let bar_rect = {
                    let mut r = ui.max_rect();
                    r.max.y = r.min.y + h;
                    r
                };
                ui.allocate_rect(bar_rect, egui::Sense::hover());

                let drag = ui.interact(
                    bar_rect,
                    ui.id().with("drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // Left: the mark, then the menus
                ui.allocate_ui_at_rect(bar_rect, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        // First thing in the window, as in VS Code and every
                        // other app that owns its title bar.
                        //
                        // It takes the whole width of the tool rail and centres
                        // itself in it, so the mark sits directly over the column
                        // of tools rather than starting a second left edge a few
                        // pixels off from theirs. That also puts the menus flush
                        // with the cards, which begin where the rail ends.
                        let logo_h = crate::ui::branding::TITLE_BAR_HEIGHT;
                        let rail = crate::ui::toolbar::RAIL_WIDTH;
                        let (column, _) =
                            ui.allocate_exact_size(egui::vec2(rail, logo_h), egui::Sense::hover());
                        if let Some(logo) = self.logo.texture(ctx, logo_h) {
                            let size = logo.size_vec2();
                            // Hover-only, and painted rather than allocated: the
                            // bar's own drag interaction is behind this, and a
                            // click sense would punch a dead spot in the window
                            // drag area.
                            ui.painter().image(
                                logo.id(),
                                egui::Rect::from_center_size(
                                    column.center(),
                                    egui::vec2(size.x / size.y * logo_h, logo_h),
                                ),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        }
                        ui.add_space(8.0);
                        egui::MenuBar::new().ui(ui, |ui| {
                            ui.menu_button("File", |ui| {
                                if ui
                                    .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
                                    .clicked()
                                {
                                    file_action = Some(FileAction::New);
                                    ui.close();
                                }
                                if ui
                                    .add(egui::Button::new("Open…").shortcut_text("Ctrl+O"))
                                    .clicked()
                                {
                                    file_action = Some(FileAction::Open);
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
                                    .clicked()
                                {
                                    file_action = Some(FileAction::Save);
                                    ui.close();
                                }
                                if ui
                                    .add(
                                        egui::Button::new("Save As…").shortcut_text("Ctrl+Shift+S"),
                                    )
                                    .clicked()
                                {
                                    file_action = Some(FileAction::SaveAs);
                                    ui.close();
                                }
                                ui.separator();
                                // Recent files (T-304) — the same list the
                                // startup window shows, reachable mid-session.
                                ui.menu_button("Open Recent", |ui| {
                                    if self.config.recent_files.is_empty() {
                                        ui.label(
                                            egui::RichText::new("Nothing yet")
                                                .weak()
                                                .small(),
                                        );
                                    }
                                    for path in self.config.recent_files.clone() {
                                        let name = path
                                            .file_stem()
                                            .and_then(|s| s.to_str())
                                            .unwrap_or("project")
                                            .to_string();
                                        if ui
                                            .add_enabled(path.exists(), egui::Button::new(name))
                                            .on_hover_text(path.display().to_string())
                                            .clicked()
                                        {
                                            file_action = Some(FileAction::OpenPath(path));
                                            ui.close();
                                        }
                                    }
                                });
                                ui.separator();
                                if ui.button("Startup Window").clicked() {
                                    self.show_startup = true;
                                    ui.close();
                                }
                                if ui
                                    .add(
                                        egui::Button::new("Settings…")
                                            .shortcut_text("Ctrl+,"),
                                    )
                                    .clicked()
                                {
                                    self.show_settings = true;
                                    ui.close();
                                }
                                if ui.button("Quit").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                            ui.menu_button("Edit", |ui| {
                                let mut clipboard_action: Option<ClipboardAction> = None;
                                // `Undo <label>` per T-107.
                                let undo_text = match self.state.history.undo_label() {
                                    Some(label) => format!("Undo {label}"),
                                    None => "Undo".to_string(),
                                };
                                if ui
                                    .add_enabled(
                                        self.state.history.can_undo(),
                                        egui::Button::new(undo_text).shortcut_text("Ctrl+Z"),
                                    )
                                    .clicked()
                                {
                                    trigger_undo = true;
                                    ui.close();
                                }
                                let redo_text = match self.state.history.redo_label() {
                                    Some(label) => format!("Redo {label}"),
                                    None => "Redo".to_string(),
                                };
                                if ui
                                    .add_enabled(
                                        self.state.history.can_redo(),
                                        egui::Button::new(redo_text).shortcut_text("Ctrl+Y"),
                                    )
                                    .clicked()
                                {
                                    trigger_redo = true;
                                    ui.close();
                                }

                                // ── Clipboard (T-209) ────────────────────
                                ui.separator();
                                let held = self.state.session.clipboard.describe();
                                for (label, shortcut, action) in [
                                    ("Copy Bones", "Ctrl+C", ClipboardAction::CopyBones),
                                    ("Copy Pose", "Ctrl+Shift+C", ClipboardAction::CopyPose),
                                ] {
                                    if ui
                                        .add(egui::Button::new(label).shortcut_text(shortcut))
                                        .clicked()
                                    {
                                        clipboard_action = Some(action);
                                        ui.close();
                                    }
                                }
                                let can_paste = !self.state.session.clipboard.is_empty();
                                if ui
                                    .add_enabled(
                                        can_paste,
                                        egui::Button::new("Paste").shortcut_text("Ctrl+V"),
                                    )
                                    .on_hover_text(format!("Holding {held}"))
                                    .clicked()
                                {
                                    clipboard_action = Some(ClipboardAction::Paste);
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(
                                        can_paste,
                                        egui::Button::new("Paste Mirrored")
                                            .shortcut_text("Ctrl+Shift+V"),
                                    )
                                    .on_hover_text("Flips X translation and rotation — the other half of a walk cycle")
                                    .clicked()
                                {
                                    clipboard_action = Some(ClipboardAction::PasteMirrored);
                                    ui.close();
                                }
                                if ui
                                    .add(egui::Button::new("Duplicate").shortcut_text("Ctrl+D"))
                                    .clicked()
                                {
                                    clipboard_action = Some(ClipboardAction::Duplicate);
                                    ui.close();
                                }

                                if let Some(action) = clipboard_action {
                                    match action {
                                        ClipboardAction::CopyBones => self.state.copy_selection(),
                                        ClipboardAction::CopyPose => self.state.copy_pose(),
                                        ClipboardAction::Paste => self.state.paste(false),
                                        ClipboardAction::PasteMirrored => self.state.paste(true),
                                        ClipboardAction::Duplicate => {
                                            self.state.duplicate_selection()
                                        }
                                    }
                                }
                            });

                            // ── View menu ────────────────────────────────
                            // Panels are dockable, which means they are also
                            // closable, which means there has to be a way back.
                            // Before this, closing a panel lost it until the
                            // layout was reset by hand.
                            ui.menu_button("View", |ui| {
                                ui.set_min_width(210.0);
                                let animating = self.state.session.is_animating();
                                for tab in crate::ui::Tab::ALL {
                                    let found = self.find_pane(tab);
                                    let mut on = found.is_some_and(|id| self.tree.is_visible(id));
                                    let label = format!("{}  {}", tab.icon(), tab.title());
                                    // An animation pane is hidden by the mode, not
                                    // by choice; offering a tick that the next
                                    // frame undoes would be a control that does
                                    // nothing.
                                    if tab.is_animation() && !animating {
                                        ui.add_enabled(
                                            false,
                                            egui::Checkbox::new(&mut on, label),
                                        )
                                        .on_disabled_hover_text(
                                            "Switch to Animate (Tab) to use this panel",
                                        );
                                        continue;
                                    }
                                    if ui.checkbox(&mut on, label).clicked() {
                                        match found {
                                            Some(id) => self.tree.set_visible(id, on),
                                            // The pane was removed from the tree
                                            // entirely rather than hidden, so
                                            // ticking it has to put one back.
                                            None => self.add_pane(tab),
                                        }
                                    }
                                }
                                ui.separator();
                                // Layer toggles live here too: they are "what is
                                // drawn", the same question the panel list asks.
                                ui.checkbox(
                                    &mut self.state.session.show_artwork,
                                    format!(
                                        "{}  Artwork",
                                        crate::ui::icons::IMAGE
                                    ),
                                );
                                ui.checkbox(
                                    &mut self.state.session.show_bones,
                                    format!("{}  Bones", crate::ui::icons::BONE),
                                );
                                if !self.state.session.hidden_slots.is_empty() {
                                    let hidden = self.state.session.hidden_slots.len();
                                    if ui
                                        .button(format!("Show {hidden} hidden slot(s)"))
                                        .on_hover_text(
                                            "Clear every per-slot hide set in the hierarchy",
                                        )
                                        .clicked()
                                    {
                                        self.state.session.hidden_slots.clear();
                                        ui.close();
                                    }
                                }
                                if self.state.session.is_isolating() {
                                    let n = self.state.session.isolated_bones.len();
                                    if ui
                                        .button(format!("Exit isolation ({n} bone(s))"))
                                        .on_hover_text("Show the whole rig again — Shift+H")
                                        .clicked()
                                    {
                                        self.state.session.clear_isolation();
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui
                                    .button("Reset Layout")
                                    .on_hover_text("Put every panel back where it started")
                                    .clicked()
                                {
                                    self.tree = Self::default_layout();
                                    ui.close();
                                }
                            });

                            // ── Pose menu (T-211) ────────────────────────
                            // Acts on the selection, or the whole rig when
                            // nothing is selected.
                            ui.menu_button("Pose", |ui| {
                                let mut action: Option<PoseAction> = None;
                                let setup = self.state.session.can_edit_structure();
                                ui.set_min_width(230.0);

                                if ui
                                    .add_enabled(setup, egui::Button::new("Set Pose As Setup"))
                                    .on_hover_text(
                                        "Bake what is on screen into the setup skeleton",
                                    )
                                    .clicked()
                                {
                                    action = Some(PoseAction::SetAsSetup);
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(setup, egui::Button::new("Reset Rotation/Scale"))
                                    .on_hover_text("Positions are kept — they are rig structure")
                                    .clicked()
                                {
                                    action = Some(PoseAction::Reset);
                                    ui.close();
                                }

                                ui.separator();
                                let animating = self.state.session.is_animating();
                                if ui
                                    .add_enabled(animating, egui::Button::new("Clear Animation"))
                                    .on_hover_text("Drop this clip's keys for the selected bones")
                                    .clicked()
                                {
                                    action = Some(PoseAction::ClearAnimation);
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(animating, egui::Button::new("Half Speed"))
                                    .clicked()
                                {
                                    action = Some(PoseAction::Scale(2.0));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(animating, egui::Button::new("Double Speed"))
                                    .clicked()
                                {
                                    action = Some(PoseAction::Scale(0.5));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(animating, egui::Button::new("Shift Keys +1"))
                                    .clicked()
                                {
                                    action = Some(PoseAction::Offset(1));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(animating, egui::Button::new("Shift Keys −1"))
                                    .clicked()
                                {
                                    action = Some(PoseAction::Offset(-1));
                                    ui.close();
                                }

                                if let Some(action) = action {
                                    match action {
                                        PoseAction::SetAsSetup => self.state.set_pose_as_setup(),
                                        PoseAction::Reset => self.state.reset_bones(),
                                        PoseAction::ClearAnimation => {
                                            self.state.clear_bone_animation()
                                        }
                                        PoseAction::Scale(f) => {
                                            self.state.scale_animation_timing(f)
                                        }
                                        PoseAction::Offset(frames) => {
                                            self.state.offset_animation_keys(frames)
                                        }
                                    }
                                }
                            });
                        });
                    });
                });

                // Centre: what is open, in a pill — the same place a browser puts
                // the address, and for the same reason. A status message takes
                // it over while there is one, because feedback on what you just
                // tried to do outranks a path you already know.
                let (center_text, is_status) = match (&self.state.session.status, &self.status) {
                    (Some(msg), _) => (msg.clone(), true),
                    (None, Some(msg)) => (msg.clone(), true),
                    (None, None) => (
                        self.current_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "Untitled".to_string()),
                        false,
                    ),
                };
                let visuals = ctx.global_style().visuals.clone();
                let font = egui::FontId::proportional(12.5);
                let galley = ui.painter().layout_no_wrap(
                    center_text,
                    font.clone(),
                    if is_status {
                        visuals.warn_fg_color
                    } else {
                        visuals.weak_text_color()
                    },
                );
                let pill = egui::Rect::from_center_size(
                    bar_rect.center(),
                    egui::vec2(galley.size().x + 28.0, 26.0),
                )
                // Clamped so a long path cannot slide under the menus or the
                // window buttons and steal their clicks.
                .intersect(bar_rect.shrink2(egui::vec2(240.0, 0.0)));
                ui.painter().rect(
                    pill,
                    13,
                    visuals.extreme_bg_color,
                    egui::Stroke::new(1.0, self.theme.card_border()),
                    egui::StrokeKind::Inside,
                );
                ui.painter().with_clip_rect(pill.shrink(6.0)).galley(
                    egui::pos2(
                        pill.center().x - galley.size().x * 0.5,
                        pill.center().y - galley.size().y * 0.5,
                    ),
                    galley,
                    visuals.weak_text_color(),
                );

                // Right: window controls
                ui.allocate_ui_at_rect(bar_rect, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        // Close is the destructive one, so it is the only button
                        // that colours on hover — the others stay quiet.
                        if window_button(ui, crate::ui::icons::CLOSE, "Close", true) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if window_button(ui, crate::ui::icons::FIT, "Maximise", false) {
                            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                        }
                        if window_button(ui, crate::ui::icons::MINIMISE, "Minimise", false) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });

        // ── Toolbar ──────────────────────────────────────────────────────
        // Text sizes, re-applied each frame so a slider drag is visible while it
        // is being dragged. `set_style` on unchanged values is a clone and a
        // pointer swap, not a relayout.
        self.config.fonts.apply(ctx);

        // A tool asked for a pane to be brought forward — double-clicking art
        // opening the slot editor, say. Consumed here, once.
        if let Some(tab) = self.state.session.focus_tab.take() {
            if self.find_pane(tab).is_none() {
                self.add_pane(tab);
            }
            self.tree.make_active(
                |_, tile| matches!(tile, egui_tiles::Tile::Pane(pane) if *pane == tab),
            );
        }

        // ── Startup page (T-304) ─────────────────────────────────────────
        // Drawn *instead of* the workspace, not over it. A launcher floating on
        // a fully-drawn editor showed a new user a busy tool they had not asked
        // for yet, half-visible around the dialog's edges. Taking the whole
        // central area states the truth: nothing is open, and this is where you
        // choose. The title bar stays so the window controls do.
        if self.show_startup {
            let mut choice = crate::ui::startup::StartupChoice::None;
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(self.theme.window_background())
                        .inner_margin(egui::Margin::same(24)),
                )
                .show(ctx, |ui| {
                    choice = crate::ui::startup::ui(ui, &mut self.config);
                });
            match choice {
                crate::ui::startup::StartupChoice::None => {}
                crate::ui::startup::StartupChoice::NewProject => {
                    file_action = Some(FileAction::New);
                    self.show_startup = false;
                }
                crate::ui::startup::StartupChoice::OpenDialog => {
                    file_action = Some(FileAction::Open);
                    self.show_startup = false;
                }
                crate::ui::startup::StartupChoice::Open(path) => {
                    file_action = Some(FileAction::OpenPath(path));
                    self.show_startup = false;
                }
            }
            // The workspace does not run at all this frame — no tool rail, no
            // tiles, no panes. Anything queued above still resolves below.
            self.resolve_frame(file_action, trigger_undo, trigger_redo);
            return;
        }

        // The tool rail. Vertical and on the left because tools are chosen with
        // the pointer already over the viewport — a horizontal strip at the top
        // is the longest trip from where the hand is.
        egui::SidePanel::left("tool_rail")
            .resizable(false)
            .show_separator_line(false)
            .exact_width(crate::ui::toolbar::RAIL_WIDTH)
            .frame(
                egui::Frame::NONE
                    .fill(self.theme.window_background())
                    .inner_margin(egui::Margin::symmetric(4, 6)),
            )
            .show(ctx, |ui| {
                crate::ui::toolbar::ui(ui, &mut self.state, &mut trigger_undo, &mut trigger_redo);
            });

        // Animation panes are hidden in Setup mode.
        //
        // Not disabled — hidden. A dopesheet with no playhead, a graph with no
        // curves and an event table with no clip are four cards saying "not
        // now", and the rig you are actually building gets what is left.
        self.apply_mode_visibility();

        // Which cards are too narrow to show every tab label.
        //
        // Measured here, before the tree runs, because the decision belongs to a
        // whole card and `tab_ui` only ever sees one tab at a time. It uses last
        // frame's rects, which is exact except on the frame a splitter is being
        // dragged — and one frame of a label popping mid-drag is invisible.
        let compact_tabs = self.compact_tabs(ctx);
        let mut close_requests: Vec<egui_tiles::TileId> = Vec::new();

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(self.theme.window_background())
                    // The window edge gets the same gap the cards have between
                    // them, so nothing sits flush against the frame.
                    .inner_margin(egui::Margin::same(crate::ui::CARD_GAP as i8)),
            )
            .show(ctx, |ui| {
                let mut behavior = AppBehavior {
                    state: &mut self.state,
                    theme: &self.theme,
                    grid: &self.config.grid,
                    fonts: &self.config.fonts,
                    hover_labels: self.config.hover_labels,
                    compact_tabs: &compact_tabs,
                    close_requests: &mut close_requests,
                };
                self.tree.ui(&mut behavior, ui);
            });

        // Tabs closed by their own button. Applied after the tree is drawn: the
        // strip is mid-layout while `tab_ui` runs, so removing a tile there would
        // pull the ground out from under it.
        for tile_id in close_requests {
            if let Some(egui_tiles::Tile::Pane(Tab::UvEditor)) = self.tree.tiles.get(tile_id) {
                // The pane's target is session state, not layout state, so
                // closing the tab has to drop it — otherwise reopening comes back
                // showing the mesh from last time.
                crate::ui::uv::clear(&mut self.state);
            }
            self.tree.tiles.remove(tile_id);
        }

        // ── Mesh trace window (T-402) ────────────────────────────────────
        crate::ui::trace::ui(ctx, &mut self.state, &self.theme);

        // ── Spritesheet slicer (T-305) ───────────────────────────────────
        crate::ui::atlas::ui(ctx, &mut self.state, &self.theme);
        crate::ui::psd_import::ui(ctx, &mut self.state, &self.theme);

        // ── Import summary (T-303) ───────────────────────────────────────
        // A conversion that quietly drops half a rig is worse than one that
        // says what it left behind, so this is a dialog, not a status line.
        if let Some(notes) = self.state.session.import_summary.clone() {
            let mut open = true;
            let dialog = crate::ui::dialog::Dialog::new("import_summary", "Import summary")
                .icon(crate::ui::icons::IMPORT_PSD)
                .width(460.0)
                .max_height(320.0)
                .show(ctx, &self.theme, |ui| {
                    for note in &notes {
                        ui.label(note);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            open = false;
                        }
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(notes.join("\n"));
                        }
                    });
                });
            if !open || dialog.closed {
                self.state.session.import_summary = None;
            }
        }

        // ── Settings (T-701) ─────────────────────────────────────────────
        // Applied live, so the theme is re-applied every frame it is open. Doing
        // it unconditionally would fight anything else that touches the style.
        if self.show_settings {
            let mut open = true;
            crate::ui::settings::ui(
                ctx,
                &mut self.state,
                &mut self.config,
                &mut self.theme,
                &mut self.available_themes,
                &mut open,
            );
            self.theme.apply(ctx);
            self.show_settings = open;
        }

        // ── Bulk rename (T-901) ──────────────────────────────────────────
        // A panel can ask for it — the hierarchy's context menu does — because
        // the dialog is owned here and the panels cannot reach it.
        if std::mem::take(&mut self.state.session.request_bulk_rename) {
            self.show_rename = true;
        }
        if self.show_rename
            && crate::ui::rename::ui(ctx, &mut self.state, &mut self.rename, &self.theme)
        {
            self.show_rename = false;
        }

        // ── Name a selection set (T-904) ─────────────────────────────────
        if std::mem::take(&mut self.state.session.request_save_selection_set) {
            self.naming_set = Some(String::new());
        }
        if let Some(name) = &mut self.naming_set {
            let mut close = false;
            let mut save: Option<String> = None;
            let response = crate::ui::dialog::Dialog::new("name_selection_set", "Save selection")
                .icon(crate::ui::icons::BONE)
                .width(320.0)
                .show(ctx, &self.theme, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} bone(s) selected",
                            self.state.session.selected_bones.len()
                        ))
                        .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(6.0);
                    let field = ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(280.0)
                            .hint_text("left arm"),
                    );
                    field.request_focus();
                    ui.add_space(8.0);
                    let named = !name.trim().is_empty();
                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.add_enabled(named, egui::Button::new("Save")).clicked() || entered)
                        && named
                    {
                        save = Some(name.trim().to_string());
                    }
                });
            if let Some(name) = save {
                let bones = self.state.session.selected_bones.clone();
                self.state.dispatch(Box::new(
                    crate::commands::selection_set_cmds::SaveSelectionSet::new(name, bones),
                ));
                close = true;
            }
            if close || response.closed {
                self.naming_set = None;
            }
        }

        self.resolve_frame(file_action, trigger_undo, trigger_redo);
    }

    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}
}

/// A File-menu / shortcut request, resolved after the frame's UI is built so the
/// native dialog does not run mid-layout.
/// An Edit-menu clipboard request, resolved after the menu closes so the
/// borrow of `self.state` inside the closure has ended.
#[derive(Clone, Copy)]
enum ClipboardAction {
    CopyBones,
    CopyPose,
    Paste,
    PasteMirrored,
    Duplicate,
}

/// A Pose-menu request (T-211), resolved after the menu closes.
#[derive(Clone, Copy)]
enum PoseAction {
    SetAsSetup,
    Reset,
    ClearAnimation,
    /// Multiply every key time by this factor — 2.0 halves the speed.
    Scale(f32),
    Offset(i32),
}

#[derive(Clone)]
enum FileAction {
    New,
    Open,
    /// Open a known path — the startup window's recent files and samples.
    OpenPath(std::path::PathBuf),
    Save,
    SaveAs,
}

impl AnkhimateApp {
    /// Everything a frame defers until its UI is built.
    ///
    /// Undo, redo and file actions all mutate state that the layout borrowed, or
    /// open a native dialog that must not run mid-layout. Factored out because
    /// the startup page returns early and still has to resolve its own choice —
    /// two copies of this tail is one chance for the launcher's "Open…" to
    /// silently do nothing.
    fn resolve_frame(
        &mut self,
        file_action: Option<FileAction>,
        trigger_undo: bool,
        trigger_redo: bool,
    ) {
        if trigger_undo {
            self.state.undo();
        }
        if trigger_redo {
            self.state.redo();
        }
        if let Some(action) = file_action {
            self.run_file_action(action);
        }
    }

    fn run_file_action(&mut self, action: FileAction) {
        use crate::fileops::{self, FileOutcome};
        let outcome = match action {
            FileAction::New => {
                fileops::new_document(&mut self.state);
                self.current_path = None;
                self.status = Some("New document".to_string());
                return;
            }
            FileAction::Open => fileops::open(&mut self.state),
            FileAction::OpenPath(path) => fileops::open_path(&mut self.state, &path),
            FileAction::Save => fileops::save(&self.state, &self.current_path),
            FileAction::SaveAs => fileops::save_as(&self.state),
        };
        match outcome {
            FileOutcome::Saved(path) => {
                self.status = Some(format!("Saved {}", path.display()));
                // Save-As gives a project a new home; the recents list should
                // point at where it actually lives now.
                self.config.touch_recent(&path);
                self.current_path = Some(path);
            }
            FileOutcome::Opened(path) => {
                self.status = Some(format!("Opened {}", path.display()));
                self.config.touch_recent(&path);
                self.current_path = Some(path);
            }
            FileOutcome::Cancelled => {}
            FileOutcome::Error(msg) => self.status = Some(msg),
        }
    }
}

#[cfg(test)]
mod view_menu_tests {
    use super::*;
    use crate::ui::Tab;

    #[test]
    fn the_default_layout_holds_every_pane() {
        let app = AnkhimateApp::default();
        for tab in Tab::ALL {
            assert!(
                app.find_pane(tab).is_some(),
                "{tab:?} is missing from the default layout, so View could not show it"
            );
        }
    }

    /// Closing the UV tab has to drop what it was editing, not just remove the
    /// tile. The target lives in session state, so a tile removed without it
    /// leaves the pane pointing at a mesh nobody can see — and reopening comes
    /// back showing that mesh instead of a clean pane.
    #[test]
    fn closing_the_uv_tab_drops_its_target() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(Tab::UvEditor).unwrap();
        app.state.session.uv_pane = Some(crate::ui::uv::UvPane::new(
            app.state.doc.skeleton.default_skin,
            app.state
                .doc
                .skeleton
                .slots
                .keys()
                .next()
                .unwrap_or_default(),
            "art".to_string(),
        ));

        // What the close button's tile id ends up doing, minus the click.
        if let Some(egui_tiles::Tile::Pane(Tab::UvEditor)) = app.tree.tiles.get(id) {
            crate::ui::uv::clear(&mut app.state);
        }
        app.tree.tiles.remove(id);

        assert!(
            app.state.session.uv_pane.is_none(),
            "target outlived the tab"
        );
        assert!(
            app.find_pane(Tab::UvEditor).is_none(),
            "the tile is still in the tree"
        );
    }

    #[test]
    fn hiding_a_pane_keeps_its_place_in_the_tree() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(Tab::Assets).unwrap();
        app.tree.set_visible(id, false);
        assert!(!app.tree.is_visible(id));
        // Still findable, so ticking the box again shows the same pane rather
        // than grafting a second copy on.
        assert_eq!(app.find_pane(Tab::Assets), Some(id));
        app.tree.set_visible(id, true);
        assert!(app.tree.is_visible(id));
    }

    /// A pane dragged out of the tree entirely has no tile to un-hide, so the
    /// menu has to be able to build a new one.
    #[test]
    fn a_removed_pane_can_be_added_back() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(Tab::Skins).unwrap();
        app.tree.remove_recursively(id);
        assert_eq!(app.find_pane(Tab::Skins), None);

        app.add_pane(Tab::Skins);
        let restored = app.find_pane(Tab::Skins).expect("pane came back");
        assert!(app.tree.is_visible(restored));
    }

    #[test]
    fn reset_layout_restores_everything() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(Tab::Timeline).unwrap();
        app.tree.remove_recursively(id);
        app.tree = AnkhimateApp::default_layout();
        for tab in Tab::ALL {
            assert!(app.find_pane(tab).is_some(), "{tab:?} did not come back");
        }
    }
}

#[cfg(test)]
mod mode_visibility_tests {
    use super::*;
    use crate::ui::Tab;

    fn is_shown(app: &AnkhimateApp, tab: Tab) -> bool {
        app.find_pane(tab).is_some_and(|id| app.tree.is_visible(id))
    }

    #[test]
    fn setup_hides_the_animation_panes_and_animate_brings_them_back() {
        let mut app = AnkhimateApp::default();

        app.state.session.work_mode = crate::session::WorkMode::Setup;
        app.apply_mode_visibility();
        for tab in Tab::ALL {
            assert_eq!(is_shown(&app, tab), !tab.is_animation(), "{tab:?} in Setup");
        }

        app.state.session.work_mode = crate::session::WorkMode::Animate;
        app.apply_mode_visibility();
        for tab in Tab::ALL {
            assert!(is_shown(&app, tab), "{tab:?} in Animate");
        }
    }

    /// A card whose tabs are *all* animation panes is hidden whole. Left visible
    /// it would be an empty frame with no tabs — which looks broken rather than
    /// deliberate.
    #[test]
    fn a_card_of_only_animation_panes_is_hidden_whole() {
        use egui_tiles::{Container, Tile};

        let mut app = AnkhimateApp::default();
        app.state.session.work_mode = crate::session::WorkMode::Setup;
        app.apply_mode_visibility();

        for (id, tile) in app.tree.tiles.iter() {
            let Tile::Container(Container::Tabs(tabs)) = tile else {
                continue;
            };
            let all_animation = !tabs.children.is_empty()
                && tabs.children.iter().all(
                    |c| matches!(app.tree.tiles.get(*c), Some(Tile::Pane(p)) if p.is_animation()),
                );
            if all_animation {
                assert!(!app.tree.is_visible(*id), "an all-animation card stayed up");
            }
        }
    }
}
