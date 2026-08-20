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
    /// What the last import could not carry across, while its report is up.
    ///
    /// On the app rather than the session: `replace_document` reseats the
    /// session, so a report stored there would be wiped by the very import that
    /// produced it.
    import_report: Option<ImportReport>,
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
    /// In-progress name for a folder: `(existing group, name)`.
    naming_group: Option<(Option<ankhimate_core::ids::GroupId>, String)>,
    /// Panes drawn in their own OS window (T-910).
    torn_off: Vec<Tab>,
    /// Plugins read at startup, and what they declared.
    plugins: crate::plugins::Plugins,
    /// The program mark. Re-rasterises itself from vector art when the size it
    /// is drawn at changes, so a UI-scale change stays sharp.
    logo: crate::ui::branding::Logo,
    /// Named verbs, keyed by id. Every built-in registers into it the same way a
    /// plugin will; key handling below resolves through it rather than calling
    /// `AppState` methods directly, so a rebound key and a plugin-shadowed
    /// operator both take effect without touching this file.
    operators: crate::registry::Registry,
    /// Timer and dirty-tracking for crash recovery (T-701).
    autosave: crate::autosave::Autosave,
    /// An autosave newer than its project, found at startup and not yet
    /// answered. `Some` means the recovery prompt is up.
    recovery: Option<crate::autosave::Recovery>,
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

        let mut config = crate::config::Config::load();
        // A config written before a binding existed has no row for it, and the
        // whole table is serialized, so the new key would be live on fresh
        // installs and dead everywhere else (T-701).
        let adopted = config.keymap.merge_new_defaults();
        if adopted > 0 {
            log::info!("adopted {adopted} new default key binding(s)");
        }
        let show_startup = !config.skip_startup;
        // Panes that were on a second monitor last session go back there
        // (T-910). A name the build no longer has is simply not torn off, rather
        // than a startup failure.
        let torn_off: Vec<Tab> = config
            .torn_off
            .iter()
            .filter_map(|name| Tab::from_saved(name))
            .filter(|t| t.can_tear_off())
            .collect();

        // Plugins, once, at startup. A file changing under a running editor is
        // a thing to opt into rather than a surprise: a reload discards
        // whatever a panel was showing.
        let plugins = crate::plugins::Plugins::directory()
            .map(|dir| crate::plugins::Plugins::load(&dir))
            .unwrap_or_default();

        let autosave = crate::autosave::Autosave::new(config.autosave_secs);

        let mut app = Self {
            plugins,
            theme: default_theme,
            available_themes,
            config,
            show_startup,
            torn_off,
            autosave,
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

        // Offer recovery *after* the open, so the check compares against the
        // project that actually loaded. Offered, not applied — see the module
        // note: only the user knows whether the last session ended badly.
        app.recovery = crate::autosave::check(app.current_path.as_deref());

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
        // A torn-off pane is hidden in the dock (T-910), or it would draw twice —
        // once in each window, both live, both editing the same document. Two
        // copies of a panel fighting over one scroll position is not two views,
        // it is a bug that looks like one.
        let torn_off: Vec<Tab> = self.torn_off.clone();
        // Resolved before the loop: `set_visible` takes `&mut self.tree`, so the
        // lookup cannot still be borrowing it.
        let out_of_dock: Vec<bool> = panes
            .iter()
            .map(|(id, _)| match self.tree.tiles.get(*id) {
                Some(Tile::Pane(pane)) => torn_off.contains(pane),
                _ => false,
            })
            .collect();
        for ((id, is_animation), out) in panes.iter().zip(out_of_dock) {
            self.tree
                .set_visible(*id, !out && (animating || !is_animation));
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
    fn find_pane(&self, tab: &crate::ui::Tab) -> Option<egui_tiles::TileId> {
        self.tree.tiles.iter().find_map(|(id, tile)| match tile {
            egui_tiles::Tile::Pane(pane) if pane == tab => Some(*id),
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
        let export = tiles.insert_pane(Tab::Export);

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
        // Export joins them: it is the same kind of question — "what is in the
        // rig, and what leaves it" — and it is opened deliberately rather than
        // watched, so a tab is the right cost.
        let library_tab =
            tiles.insert_tab_tile(vec![assets, draw_order, skins, constraints, export]);
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
            // No plugins in the default app: `Default` is for tests, which
            // should not read whatever happens to be in the developer's config
            // directory.
            plugins: crate::plugins::Plugins::default(),
            tree: Self::default_layout(),
            theme: theme::Theme::default(),
            available_themes: vec![theme::Theme::default()],
            state: AppState::default(),
            current_path: None,
            status: None,
            import_report: None,
            config: crate::config::Config::default(),
            show_startup: false,
            show_settings: false,
            show_rename: false,
            rename: Default::default(),
            naming_group: None,
            torn_off: Vec::new(),
            logo: crate::ui::branding::Logo::default(),
            operators: crate::registry::Registry::with_builtins(),
            autosave: crate::autosave::Autosave::default(),
            recovery: None,
        }
    }
}

/// Draw a menu entry for the operator `id` names, and report a click.
///
/// Label, enabled state and the shortcut all come from the operator and the
/// keymap rather than from the call site. Every one of those was previously
/// written out again at each menu, and they had drifted: Redo advertised
/// `Ctrl+Y` when the first binding is `Ctrl+Shift+Z`, and Copy, Copy Pose and
/// Duplicate had no enable rule at all, so they stayed clickable with an empty
/// selection while their key bindings correctly declined.
///
/// A free function rather than a method: the caller holds `&mut self` for the
/// egui closure, so this borrows only the three pieces it reads.
///
/// `suffix` appends to the operator's own label — the Edit menu shows
/// "Undo Move Bone", where "Move Bone" is the command on top of the stack.
fn operator_button(
    ui: &mut egui::Ui,
    operators: &crate::registry::Registry,
    keymap: &crate::keymap::Keymap,
    state: &AppState,
    id: &str,
    suffix: Option<&str>,
) -> bool {
    let Some(op) = operators.get(id) else {
        // An id with no operator draws nothing rather than a dead entry. This
        // is reachable once plugins can register menu items and then be
        // uninstalled.
        return false;
    };
    let label = match suffix {
        Some(extra) => format!("{} {extra}", op.label()),
        None => op.label().to_string(),
    };
    let mut button = egui::Button::new(label);
    if let Some(chord) = keymap.chord_for(id) {
        button = button.shortcut_text(chord.label());
    }
    let clicked = ui.add_enabled(op.enabled(state), button).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

impl AnkhimateApp {
    /// Run the operator `id` names, and honour whatever chrome it asked for.
    ///
    /// The single place an operator's [`UiRequest`] turns into an open window.
    /// Operators cannot reach these flags themselves — that boundary is what
    /// keeps a future plugin out of the frame loop and the egui context.
    ///
    /// [`UiRequest`]: crate::registry::UiRequest
    fn run_operator(&mut self, id: &str) -> bool {
        use crate::registry::UiRequest;

        let Some(result) = self.operators.invoke(id, &mut self.state) else {
            // Unknown id, or the operator declined as inapplicable. Both are
            // silent: a key bound to something that does not apply right now
            // should feel like the key does nothing, not like an error.
            return false;
        };
        match result.ui {
            Some(UiRequest::Settings) => self.show_settings = !self.show_settings,
            Some(UiRequest::Rename) => self.show_rename = true,
            Some(UiRequest::Startup) => self.show_startup = true,
            None => {}
        }
        true
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

        // Undo and redo are no longer read here — they are keymap bindings like
        // everything else. The flags remain because the Edit menu and the
        // toolbar still raise them, and `resolve_frame` routes all three to the
        // same operator.
        let mut trigger_undo = false;
        let mut trigger_redo = false;
        let mut file_action: Option<FileAction> = None;
        // An operator a menu entry asked for. Deferred rather than run in place
        // because drawing the menu borrows `self` immutably and `run_operator`
        // needs it mutably.
        let mut menu_operator: Option<&str> = None;
        if ctx.input(|i| i.modifiers.ctrl) {
            // File shortcuts: Ctrl+N/O/S, Ctrl+Shift+S. Still inline because a
            // file action is not an operator — it opens a native dialog and can
            // fail with a message, which `OpResult` has no room for. Folding
            // these in belongs with the format registry.
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
            // Clipboard (T-209) moved to the keymap: Ctrl+C/V/D and their
            // Shift variants are ordinary bindings now.
        }

        // ── Autosave (T-701) ─────────────────────────────────────────────
        // Driven off the frame clock, and only when the revision has moved. The
        // write is silent unless it fails: the user did not ask for it, so it
        // must not take over the status line they are reading.
        self.autosave.interval_secs = self.config.autosave_secs;
        let dt = ctx.input(|i| i.stable_dt);
        if self.autosave.tick(dt, self.state.revision) {
            let path = self.current_path.clone();
            if let Some(written) = self.autosave.write(&self.state, path.as_deref()) {
                log::debug!("autosaved to {}", written.display());
            }
        }

        // ── Keymap (T-701) ───────────────────────────────────────────────
        // One table pass replaces what used to be twenty-odd `key_pressed`
        // arms. Which bindings survive a focused text field is the binding's
        // own business, not this site's: `Ctrl+Z` opts in, bare letters do not.
        let typing = ctx.memory(|m| m.focused().is_some());
        // A settings row waiting for a chord swallows the whole frame's input:
        // the key you press to *become* a binding must not also fire whatever it
        // is bound to today. Rebinding undo to Ctrl+U would otherwise undo on
        // the way past.
        let capturing = crate::ui::settings::capturing(ctx);
        let fired: Vec<String> = if capturing {
            Vec::new()
        } else {
            ctx.input(|i| {
                self.config
                    .keymap
                    .resolve(i, typing)
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
        };
        for id in fired {
            self.run_operator(&id);
        }

        // ── Playback shortcuts (T-202) ───────────────────────────────────
        // Suppressed while a text field has focus so typing a name does not
        // scrub the timeline.
        if !typing {
            // Shift+H isolates the viewport to the selection, or leaves
            // isolation when there is nothing selected or it is already on
            // (T-903). Still inline: it toggles two session fields and writes a
            // status line rather than naming one verb, so it wants splitting
            // into `view.isolate` / `view.show_all` before it becomes a binding.
            let (h, ctrl, shift) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::H),
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                )
            });
            {
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
            // Arrows nudge the selection when there is one, and step the timeline
            // when there is not. The same key doing two things is a real cost;
            // the alternative was a chord on the commoner action, and a selected
            // bone is a clear enough signal of which you meant.
            if !self.state.session.selected_bones.is_empty() && self.nudge_selection(ctx) {
                // Handled as a transform; the timeline keeps its playhead.
            } else if left {
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
                                // Import replaces the document, so it sits with
                                // Open rather than with the asset-level imports
                                // in the library panel.
                                ui.menu_button("Import", |ui| {
                                    // One entry per registered importer, so a
                                    // new format is a registration rather than
                                    // an edit here — which is the whole point
                                    // of the registry.
                                    let importers =
                                        crate::fileops::importers_with(&self.plugins);
                                    for importer in importers.iter() {
                                        if ui
                                            .add(egui::Button::new(format!(
                                                "{}…",
                                                importer.label()
                                            )))
                                            .on_hover_text(format!(
                                                "Read a {} rig and its images as a                                                  new document",
                                                importer.label()
                                            ))
                                            .clicked()
                                        {
                                            file_action =
                                                Some(FileAction::Import(importer.id().to_string()));
                                            ui.close();
                                        }
                                    }
                                });
                                ui.separator();
                                // Recent files (T-304) — the same list the
                                // startup window shows, reachable mid-session.
                                ui.menu_button("Open Recent", |ui| {
                                    if self.config.recent_files.is_empty() {
                                        ui.label(egui::RichText::new("Nothing yet").weak().small());
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
                                if operator_button(
                                    ui,
                                    &self.operators,
                                    &self.config.keymap,
                                    &self.state,
                                    "app.settings",
                                    None,
                                ) {
                                    menu_operator = Some("app.settings");
                                }
                                if ui.button("Quit").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            });
                            ui.menu_button("Edit", |ui| {
                                // Every entry's label, enabled state and shortcut
                                // come from the operator and the keymap. The
                                // clicked id is collected and run after the
                                // closure, since drawing holds `&self`.
                                let ops = &self.operators;
                                let keymap = &self.config.keymap;
                                let state = &self.state;

                                // `Undo <label>` per T-107 — the name of the
                                // command on top of the stack, not a static word.
                                if operator_button(
                                    ui,
                                    ops,
                                    keymap,
                                    state,
                                    "edit.undo",
                                    state.history.undo_label(),
                                ) {
                                    trigger_undo = true;
                                }
                                if operator_button(
                                    ui,
                                    ops,
                                    keymap,
                                    state,
                                    "edit.redo",
                                    state.history.redo_label(),
                                ) {
                                    trigger_redo = true;
                                }

                                // ── Clipboard (T-209) ────────────────────
                                ui.separator();
                                for id in [
                                    "edit.copy",
                                    "edit.copy_pose",
                                    "edit.paste",
                                    "edit.paste_mirrored",
                                    "edit.duplicate",
                                ] {
                                    if operator_button(ui, ops, keymap, state, id, None) {
                                        menu_operator = Some(id);
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
                                for tab in crate::ui::Tab::ALL.iter() {
                                    let found = self.find_pane(tab);
                                    let mut on = found.is_some_and(|id| self.tree.is_visible(id));
                                    let label = format!("{}  {}", tab.icon(), tab.title());
                                    // An animation pane is hidden by the mode, not
                                    // by choice; offering a tick that the next
                                    // frame undoes would be a control that does
                                    // nothing.
                                    if tab.is_animation() && !animating {
                                        ui.add_enabled(false, egui::Checkbox::new(&mut on, label))
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
                                            None => self.add_pane(tab.clone()),
                                        }
                                    }
                                }

                                // Plugin panels, under the built-ins. Listed
                                // here rather than in a menu of their own: a
                                // panel is a panel, and a user looking for one
                                // should not have to know which came from a
                                // file they dropped in a folder.
                                let plugin_panels = self.plugins.panels();
                                if !plugin_panels.is_empty() {
                                    ui.separator();
                                    for (id, title) in plugin_panels {
                                        let tab = Tab::Plugin(id);
                                        let found = self.find_pane(&tab);
                                        let mut on =
                                            found.is_some_and(|tile| self.tree.is_visible(tile));
                                        let label =
                                            format!("{}  {title}", crate::ui::icons::PLUGIN);
                                        if ui.checkbox(&mut on, label).clicked() {
                                            match found {
                                                Some(tile) => self.tree.set_visible(tile, on),
                                                None => self.add_pane(tab),
                                            }
                                        }
                                    }
                                }

                                ui.separator();
                                // Layer toggles live here too: they are "what is
                                // drawn", the same question the panel list asks.
                                ui.checkbox(
                                    &mut self.state.session.show_artwork,
                                    format!("{}  Artwork", crate::ui::icons::IMAGE),
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
                                // Tear-off (T-910). A submenu rather than a flat
                                // list: fourteen panes would bury the rest of
                                // the View menu.
                                ui.menu_button("Open in a window", |ui| {
                                    for tab in Tab::ALL.iter().cloned() {
                                        if !tab.can_tear_off() {
                                            continue;
                                        }
                                        let out = self.torn_off.contains(&tab);
                                        if ui
                                            .selectable_label(
                                                out,
                                                format!("{}  {}", tab.icon(), tab.title()),
                                            )
                                            .clicked()
                                        {
                                            if out {
                                                self.torn_off.retain(|t| *t != tab);
                                            } else {
                                                self.torn_off.push(tab);
                                            }
                                            self.save_torn_off();
                                            ui.close();
                                        }
                                    }
                                    if Tab::ALL.iter().any(|t| !t.can_tear_off()) {
                                        ui.separator();
                                        ui.label(
                                            egui::RichText::new(
                                                "The viewport stays docked — it draws\n\
                                                 through the shared GPU pass.",
                                            )
                                            .size(10.0)
                                            .color(ui.visuals().weak_text_color()),
                                        );
                                    }
                                });
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
                                    .on_hover_text("Bake what is on screen into the setup skeleton")
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
            if self.find_pane(&tab).is_none() {
                self.add_pane(tab.clone());
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
            self.resolve_frame(file_action, trigger_undo, trigger_redo, menu_operator);
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
                    plugins: &self.plugins,
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

        // ── Import report (T-6xx) ────────────────────────────────────────
        // Shown after an import that lost something, and only then. An import
        // invents data the source did not have in the shape we hold it, so what
        // it could not carry across belongs in front of the person who still has
        // the original file.
        if self.import_report.is_some() {
            let mut open = true;
            // Taken out so the closure can borrow it without holding `self`.
            let report = self.import_report.take().expect("checked just above");
            egui::Window::new(format!("Imported {}", report.file))
                .open(&mut open)
                .resizable(true)
                .default_width(460.0)
                .max_height(420.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "The rig is loaded. These are the parts that could not \
                             be carried across exactly.",
                        )
                        .weak(),
                    );
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if !report.dangling.is_empty() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Unresolved ({})",
                                    report.dangling.len()
                                ))
                                .strong()
                                .color(ui.visuals().warn_fg_color),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Named by the file but not found in it. Usually a \
                                     missing image.",
                                )
                                .weak()
                                .small(),
                            );
                            for (what, name) in report.dangling.iter().take(40) {
                                ui.label(format!("   {what}: {name}"));
                            }
                            if report.dangling.len() > 40 {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "   … and {} more",
                                        report.dangling.len() - 40
                                    ))
                                    .weak(),
                                );
                            }
                            ui.add_space(8.0);
                        }

                        // Grouped by kind: a rig with one approximated curve and
                        // one with four hundred are different situations, and a
                        // flat list of the first forty hides which you have.
                        let mut by_kind: std::collections::BTreeMap<&str, Vec<_>> =
                            Default::default();
                        for l in &report.lossy {
                            by_kind.entry(l.what).or_default().push(l);
                        }
                        for (what, items) in by_kind {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Approximated — {what} ({})",
                                    items.len()
                                ))
                                .strong(),
                            );
                            // The detail repeats across a kind, so one reading of
                            // it plus the places it happened is the whole story.
                            if let Some(first) = items.first() {
                                ui.label(egui::RichText::new(&first.detail).weak().small());
                            }
                            for l in items.iter().take(8) {
                                ui.label(format!("   {}", l.where_));
                            }
                            if items.len() > 8 {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "   … and {} more",
                                        items.len() - 8
                                    ))
                                    .weak(),
                                );
                            }
                            ui.add_space(8.0);
                        }
                    });
                });
            if open {
                self.import_report = Some(report);
            }
        }

        // ── Settings (T-701) ─────────────────────────────────────────────
        // Applied live, so the theme is re-applied every frame it is open. Doing
        // it unconditionally would fight anything else that touches the style.
        // ── Crash recovery (T-701) ───────────────────────────────────────
        // An autosave newer than its project. Offered rather than applied: only
        // the user knows whether the last session ended badly, and opening a
        // different file than the one they double-clicked is not a favour.
        if let Some(recovery) = self.recovery.clone() {
            let chrome = self.theme.clone();
            let mut restore = false;
            let mut dismiss = false;
            let dialog = crate::ui::dialog::Dialog::new("recovery", "Recover unsaved work?")
                .width(460.0)
                .show(ctx, &chrome, |ui| {
                    ui.label(match &recovery.project {
                        Some(project) => format!(
                            "An autosave of {} is newer than the file itself — the last \
                             session may have ended before saving.",
                            project
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("this project")
                        ),
                        None => "An autosave of an unsaved document was left behind.".to_string(),
                    });
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(recovery.autosave.display().to_string())
                            .weak()
                            .small(),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Recover").clicked() {
                            restore = true;
                        }
                        if ui
                            .button("Ignore")
                            .on_hover_text(
                                "Keeps the autosave file — it is only offered once per launch",
                            )
                            .clicked()
                        {
                            dismiss = true;
                        }
                    });
                });

            if restore {
                match crate::fileops::open_path(&mut self.state, &recovery.autosave) {
                    crate::fileops::FileOutcome::Opened(_) => {
                        // `current_path` stays on the *project*, not the
                        // autosave: the next Save must write the real file, not
                        // a `.ankh.autosave` the user would then have to notice
                        // and rename.
                        self.current_path = recovery.project.clone();
                        self.autosave.reset();
                        self.status = Some("Recovered from autosave — save to keep it".to_string());
                        self.show_startup = false;
                    }
                    crate::fileops::FileOutcome::Error(e) => {
                        self.status = Some(format!("Could not recover: {e}"))
                    }
                    _ => {}
                }
            }
            if restore || dismiss || dialog.closed {
                self.recovery = None;
            }
        }

        if self.show_settings {
            let mut open = true;
            crate::ui::settings::ui(
                ctx,
                &mut self.state,
                &mut self.config,
                &mut self.theme,
                &mut self.available_themes,
                &self.operators,
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

        // ── Folder edits from the hierarchy ──────────────────────────────
        // Row-level actions arrive as requests because the tree cannot reach the
        // dialogs, which live here.
        if let Some(request) = self.state.session.group_request.take() {
            use crate::ui::tree::GroupRequest;
            use ankhimate_document::commands::group_cmds::{EditGroup, GroupEdit};
            match request {
                GroupRequest::Rename(id, current) => {
                    self.naming_group = Some((Some(id), current));
                }
                GroupRequest::Remove(id) => {
                    self.state
                        .dispatch(Box::new(EditGroup::new(id, GroupEdit::Ungroup)));
                }
                GroupRequest::SelectForTransform(id) => {
                    let targets = self.state.doc.skeleton.group_transform_targets(id);
                    if let Some(&last) = targets.last() {
                        self.state.session.selection = Some(crate::session::Selection::Bone(last));
                    }
                    self.state.session.selected_bones = targets;
                }
                GroupRequest::AddSelected(id) => {
                    let members: Vec<_> = self
                        .state
                        .session
                        .selected_bones
                        .iter()
                        .map(|b| ankhimate_core::skeleton::GroupMember::Bone(*b))
                        .collect();
                    self.state
                        .dispatch(Box::new(EditGroup::new(id, GroupEdit::Add(members))));
                }
            }
        }
        if std::mem::take(&mut self.state.session.request_new_group) {
            self.naming_group = Some((None, String::new()));
        }
        if let Some((target, name)) = &mut self.naming_group {
            let target = *target;
            let mut close = false;
            let mut commit: Option<String> = None;
            let response = crate::ui::dialog::Dialog::new(
                "name_group",
                if target.is_some() {
                    "Rename group"
                } else {
                    "New group"
                },
            )
            .icon(crate::ui::icons::FOLDER)
            .width(320.0)
            .show(ctx, &self.theme, |ui| {
                if target.is_none() {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} bone(s) selected",
                            self.state.session.selected_bones.len()
                        ))
                        .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(6.0);
                }
                let field = ui.add(
                    egui::TextEdit::singleline(name)
                        .desired_width(280.0)
                        .hint_text("front leg"),
                );
                field.request_focus();
                ui.add_space(8.0);
                let named = !name.trim().is_empty();
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(named, egui::Button::new("OK")).clicked() || entered) && named {
                    commit = Some(name.trim().to_string());
                }
            });
            if let Some(name) = commit {
                use ankhimate_document::commands::group_cmds::{CreateGroup, EditGroup, GroupEdit};
                match target {
                    Some(id) => self
                        .state
                        .dispatch(Box::new(EditGroup::new(id, GroupEdit::Rename(name)))),
                    None => {
                        let members: Vec<_> = self
                            .state
                            .session
                            .selected_bones
                            .iter()
                            .map(|b| ankhimate_core::skeleton::GroupMember::Bone(*b))
                            .collect();
                        self.state
                            .dispatch(Box::new(CreateGroup::new(name, members)))
                    }
                };
                close = true;
            }
            if close || response.closed {
                self.naming_group = None;
            }
        }

        self.draw_torn_off_windows(ctx, &compact_tabs);

        self.resolve_frame(file_action, trigger_undo, trigger_redo, menu_operator);
    }

    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}
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

/// A File-menu / shortcut request, resolved after the frame's UI is built so the
/// native dialog does not run mid-layout.
#[derive(Clone)]
enum FileAction {
    New,
    Open,
    /// Open a known path — the startup window's recent files and samples.
    OpenPath(std::path::PathBuf),
    Save,
    SaveAs,
    /// Read a foreign rig, replacing the document (T-6xx). Carries the
    /// importer's id rather than a variant per format: the set is open, and a
    /// plugin's importer has to reach this without a new enum arm.
    Import(String),
}

/// What an import could not carry across, held while the report is shown.
///
/// Opening an `.ankh` is lossless; importing is not. A source format may carry
/// a concept this model has at a different resolution, or not at all, and the
/// conversion has to choose. Those choices belong in front of the person who
/// still has the original file — not in a status line that scrolls away, and not
/// discovered weeks later as an animation that drifts.
struct ImportReport {
    file: String,
    /// References that did not resolve at all.
    dangling: Vec<(&'static str, String)>,
    /// Approximations, counted by kind, with a few examples of each.
    lossy: Vec<ankhimate_formats::convert::Lossy>,
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
        menu_operator: Option<&str>,
    ) {
        // Through the registry rather than `AppState` directly, so the keyboard,
        // the Edit menu and the toolbar — all three of which set these flags —
        // resolve to the same operator, and a plugin shadowing `edit.undo`
        // reaches every one of them.
        if trigger_undo {
            self.run_operator("edit.undo");
        }
        if trigger_redo {
            self.run_operator("edit.redo");
        }
        if let Some(id) = menu_operator {
            self.run_operator(id);
        }
        if let Some(action) = file_action {
            self.run_file_action(action);
        }
    }

    /// Draw every torn-off pane in its own OS window (T-910).
    ///
    /// `show_viewport_immediate` rather than the deferred form, and that is the
    /// whole design: the immediate callback is `FnMut`, so it borrows `self`
    /// directly and draws from the *same* `AppState` the main window just drew
    /// from. There is one document, one undo stack, and no synchronisation to
    /// get wrong — "two windows must never diverge" is not enforced here, it is
    /// impossible here.
    ///
    /// The deferred form would have needed the state behind an `Arc<Mutex<_>>`
    /// and a frame of lag between the windows, which is exactly the divergence
    /// this is meant to avoid.
    fn draw_torn_off_windows(
        &mut self,
        ctx: &egui::Context,
        compact_tabs: &std::collections::HashSet<egui_tiles::TileId>,
    ) {
        if self.torn_off.is_empty() {
            return;
        }
        let mut closed: Vec<Tab> = Vec::new();
        // Cloned so the loop does not hold a borrow of `self` while the callback
        // below takes one.
        for tab in self.torn_off.clone() {
            let id = egui::ViewportId::from_hash_of(("torn_off", tab.title()));
            ctx.show_viewport_immediate(
                id,
                egui::ViewportBuilder::default()
                    .with_title(format!("Ankhimate — {}", tab.title()))
                    .with_inner_size([520.0, 640.0]),
                |ctx, _class| {
                    egui::Area::new(egui::Id::new(("torn_off_area", tab.title())))
                        .fixed_pos(egui::Pos2::ZERO)
                        .show(ctx, |ui| {
                            let screen = ctx.content_rect();
                            ui.painter()
                                .rect_filled(screen, 0.0, self.theme.window_background());
                            ui.set_max_size(screen.size());
                            ui.scope_builder(
                                egui::UiBuilder::new().max_rect(screen.shrink(4.0)),
                                |ui| {
                                    let mut close_requests = Vec::new();
                                    let mut behavior = AppBehavior {
                                        state: &mut self.state,
                                        plugins: &self.plugins,
                                        theme: &self.theme,
                                        grid: &self.config.grid,
                                        fonts: &self.config.fonts,
                                        hover_labels: self.config.hover_labels,
                                        compact_tabs,
                                        close_requests: &mut close_requests,
                                    };
                                    // The same call the docked tile makes, so a
                                    // panel cannot behave differently for being
                                    // in another window.
                                    behavior.pane_contents(ui, &tab);
                                },
                            );
                        });
                    // The window's own ✕ docks the pane again rather than
                    // destroying it: a panel is not a document, and losing one
                    // to a misclick should cost a menu item, not the layout.
                    if ctx.input(|i| i.viewport().close_requested()) {
                        closed.push(tab.clone());
                    }
                },
            );
        }
        if !closed.is_empty() {
            self.torn_off.retain(|t| !closed.contains(t));
            self.save_torn_off();
        }
    }

    /// Nudge the selected bones with the arrow keys, per the active transform
    /// tool. Returns whether an arrow was consumed.
    ///
    /// Which axis each arrow drives is the tool's business, not one convention
    /// forced onto four different operations:
    ///
    /// * **Translate** — arrows move on their own axes, the obvious mapping.
    /// * **Rotate** — left/right turn, and up/down turn too, so the hand already
    ///   on the arrows does not have to find the right pair.
    /// * **Scale** — up/down scale both axes together; left/right scale x alone,
    ///   which is the one people reach for when a limb is the wrong length.
    /// * **Shear** — left/right shear x, up/down shear y, matching the gizmo's
    ///   own two handles.
    ///
    /// Shift multiplies the step by ten. Small enough by default to place a bone
    /// exactly, and a held Shift covers the distance that would otherwise be
    /// forty presses.
    ///
    /// A group is nudged through `TransformGroup` when the selection *is* a
    /// group's members, so a folder moves as one undo step rather than one per
    /// bone.
    fn nudge_selection(&mut self, ctx: &egui::Context) -> bool {
        use crate::session::TransformTool;

        let (left, right, up, down, shift) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.modifiers.shift,
            )
        });
        if !(left || right || up || down) {
            return false;
        }
        // Structural edits only in Setup; while animating the same nudge should
        // be keyed, which `commit_bone_pose` already routes.
        let x = (right as i32 - left as i32) as f32;
        // Screen up is world down — the viewport's Y runs the other way.
        let y = (up as i32 - down as i32) as f32;
        let step = if shift { 10.0 } else { 1.0 };

        let mut delta = ankhimate_document::commands::group_cmds::GroupDelta::default();
        match self.state.session.active_transform_tool {
            TransformTool::Translate => delta.translate = glam::vec2(x, y) * step,
            TransformTool::Rotate => {
                // Both pairs turn, so either hand position works.
                let turn = if x != 0.0 { x } else { y };
                delta.rotate = turn * step * std::f32::consts::PI / 180.0;
            }
            TransformTool::Scale => {
                let factor = 1.0 + 0.01 * step;
                if y != 0.0 {
                    let s = if y > 0.0 { factor } else { 1.0 / factor };
                    delta.scale = glam::vec2(s, s);
                } else if x != 0.0 {
                    let s = if x > 0.0 { factor } else { 1.0 / factor };
                    delta.scale = glam::vec2(s, 1.0);
                }
            }
            TransformTool::Shear => {
                delta.shear = glam::vec2(x, y) * step * std::f32::consts::PI / 180.0;
            }
        }

        let bones = self.state.session.selected_bones.clone();
        if bones.len() > 1 {
            // A multi-selection turns and scales about the box drawn round it,
            // however it was selected — box-swept, ctrl-clicked, or a folder.
            // One rule, and the pivot is the thing on screen, so there is no
            // guessing which behaviour you got.
            let pivot = ankhimate_core::pose::selection_bounds(
                &self.state.doc.skeleton,
                &self.state.pose,
                &bones,
            )
            .map(|(min, max)| (min + max) * 0.5);
            self.state.dispatch(Box::new(
                ankhimate_document::commands::group_cmds::TransformGroup::new(bones, pivot, delta),
            ));
        } else {
            for bone in bones {
                let Some(b) = self.state.doc.skeleton.bones.get(bone) else {
                    continue;
                };
                let mut local = b.local_transform;
                local.position += delta.translate;
                local.rotation += delta.rotate;
                local.scale *= delta.scale;
                local.shear += delta.shear;
                // Through `commit_bone_pose`, so a single-bone nudge becomes a
                // setup edit or a key depending on mode, exactly as a drag does.
                self.state.commit_bone_pose(bone, local);
            }
            self.state.refresh_pose();
        }
        true
    }

    /// Persist which panes are torn off, so a second monitor stays set up.
    fn save_torn_off(&mut self) {
        self.config.torn_off = self.torn_off.iter().map(|t| t.saved_name()).collect();
        self.config.save();
    }

    fn run_file_action(&mut self, action: FileAction) {
        use crate::fileops::{self, FileOutcome};
        let outcome = match action {
            FileAction::New => {
                fileops::new_document(&mut self.state);
                self.current_path = None;
                self.autosave.reset();
                self.status = Some("New document".to_string());
                return;
            }
            FileAction::Open => fileops::open(&mut self.state),
            FileAction::OpenPath(path) => fileops::open_path(&mut self.state, &path),
            FileAction::Save => fileops::save(&self.state, &self.current_path),
            FileAction::SaveAs => fileops::save_as(&self.state),
            FileAction::Import(id) => fileops::import_with(&mut self.state, &self.plugins, &id),
        };
        match outcome {
            FileOutcome::Saved(path) => {
                self.status = Some(format!("Saved {}", path.display()));
                // The real save supersedes any autosave, so drop it. Keyed on
                // the path we were editing *before* this, not the new one: a
                // Save As leaves the old project's autosave behind otherwise,
                // and it would be offered on the next launch as if the session
                // had crashed.
                self.autosave.discard(self.current_path.as_deref());
                if self.current_path.as_deref() != Some(path.as_path()) {
                    self.autosave.discard(Some(&path));
                }
                self.autosave.reset();
                // Save-As gives a project a new home; the recents list should
                // point at where it actually lives now.
                self.config.touch_recent(&path);
                self.current_path = Some(path);
            }
            FileOutcome::Opened(path) => {
                self.status = Some(format!("Opened {}", path.display()));
                self.config.touch_recent(&path);
                self.current_path = Some(path);
                // A different document shares nothing with the last one; keeping
                // its saved revision would make the first tick believe this one
                // was already written.
                self.autosave.reset();
            }
            FileOutcome::Imported { path, report } => {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("rig")
                    .to_string();
                self.status = Some(format!("Imported {name}"));
                // Deliberately *not* `current_path` and not a recent file: the
                // document is now an unsaved `.ankh`, and pointing Save at the
                // source `.json` would write our format over their skeleton.
                self.current_path = None;
                if !report.is_clean() {
                    self.import_report = Some(ImportReport {
                        file: name,
                        dangling: report.dangling,
                        lossy: report.lossy,
                    });
                }
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
        for tab in Tab::ALL.iter().cloned() {
            assert!(
                app.find_pane(&tab).is_some(),
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
        let id = app.find_pane(&Tab::UvEditor).unwrap();
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
            app.find_pane(&Tab::UvEditor).is_none(),
            "the tile is still in the tree"
        );
    }

    #[test]
    fn hiding_a_pane_keeps_its_place_in_the_tree() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(&Tab::Assets).unwrap();
        app.tree.set_visible(id, false);
        assert!(!app.tree.is_visible(id));
        // Still findable, so ticking the box again shows the same pane rather
        // than grafting a second copy on.
        assert_eq!(app.find_pane(&Tab::Assets), Some(id));
        app.tree.set_visible(id, true);
        assert!(app.tree.is_visible(id));
    }

    /// A pane dragged out of the tree entirely has no tile to un-hide, so the
    /// menu has to be able to build a new one.
    #[test]
    fn a_removed_pane_can_be_added_back() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(&Tab::Skins).unwrap();
        app.tree.remove_recursively(id);
        assert_eq!(app.find_pane(&Tab::Skins), None);

        app.add_pane(Tab::Skins);
        let restored = app.find_pane(&Tab::Skins).expect("pane came back");
        assert!(app.tree.is_visible(restored));
    }

    #[test]
    fn reset_layout_restores_everything() {
        let mut app = AnkhimateApp::default();
        let id = app.find_pane(&Tab::Timeline).unwrap();
        app.tree.remove_recursively(id);
        app.tree = AnkhimateApp::default_layout();
        for tab in Tab::ALL.iter().cloned() {
            assert!(app.find_pane(&tab).is_some(), "{tab:?} did not come back");
        }
    }
}

#[cfg(test)]
mod mode_visibility_tests {
    use super::*;
    use crate::ui::Tab;

    fn is_shown(app: &AnkhimateApp, tab: Tab) -> bool {
        app.find_pane(&tab)
            .is_some_and(|id| app.tree.is_visible(id))
    }

    #[test]
    fn setup_hides_the_animation_panes_and_animate_brings_them_back() {
        let mut app = AnkhimateApp::default();

        app.state.session.work_mode = crate::session::WorkMode::Setup;
        app.apply_mode_visibility();
        for tab in Tab::ALL.iter().cloned() {
            let shown = is_shown(&app, tab.clone());
            assert_eq!(shown, !tab.is_animation(), "{tab:?} in Setup");
        }

        app.state.session.work_mode = crate::session::WorkMode::Animate;
        app.apply_mode_visibility();
        for tab in Tab::ALL.iter().cloned() {
            assert!(is_shown(&app, tab.clone()), "{tab:?} in Animate");
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
