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
}

impl AnkhimateApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let available_themes = theme::Theme::load_all();
        let default_theme = available_themes.first().cloned().unwrap_or_default();

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

        Self {
            theme: default_theme,
            available_themes,
            config,
            show_startup,
            ..Default::default()
        }
    }
}

impl Default for AnkhimateApp {
    fn default() -> Self {
        let mut tiles = Tiles::default();

        let canvas = tiles.insert_pane(Tab::Canvas);
        let inspector = tiles.insert_pane(Tab::Inspector);
        let draw_order = tiles.insert_pane(Tab::DrawOrder);
        let assets = tiles.insert_pane(Tab::Assets);
        let tree = tiles.insert_pane(Tab::Hierarchy);
        let timeline = tiles.insert_pane(Tab::Timeline);

        let canvas_tab = tiles.insert_tab_tile(vec![canvas]);
        // Properties gets its own tile rather than sharing tabs with Assets:
        // the transform controls are used constantly, and hiding them behind a
        // tab every time the image library is opened is the wrong trade.
        let inspector_tab = tiles.insert_tab_tile(vec![inspector]);
        // Assets and draw order are both "what is in the rig" browsers, so they
        // can share.
        let library_tab = tiles.insert_tab_tile(vec![assets, draw_order]);
        let tree_tab = tiles.insert_tab_tile(vec![tree]);
        let timeline_tab = tiles.insert_tab_tile(vec![timeline]);

        let right = tiles.insert_vertical_tile(vec![tree_tab, inspector_tab, library_tab]);
        let center_row = tiles.insert_horizontal_tile(vec![canvas_tab, right]);
        let root = tiles.insert_vertical_tile(vec![center_row, timeline_tab]);

        let tree = Tree::new("ankhimate_tree", root, tiles);

        Self {
            tree,
            theme: theme::Theme::default(),
            available_themes: vec![theme::Theme::default()],
            state: AppState::default(),
            current_path: None,
            status: None,
            config: crate::config::Config::default(),
            show_startup: false,
        }
    }
}

impl eframe::App for AnkhimateApp {
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
                let (t, r, s, h, ctrl) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::T),
                        i.key_pressed(egui::Key::R),
                        i.key_pressed(egui::Key::S),
                        i.key_pressed(egui::Key::H),
                        i.modifiers.ctrl,
                    )
                });
                if !ctrl {
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
            .frame(egui::Frame::NONE.fill(ctx.global_style().visuals.panel_fill))
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

                // Left: menus only (no theme selector here anymore)
                ui.allocate_ui_at_rect(bar_rect, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
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

                // Center: the most recent thing the editor wants to say — a
                // refused edit (session status) outranks a file-op result,
                // because it is feedback on what the user just tried to do.
                let center_text = match (&self.state.session.status, &self.status) {
                    (Some(msg), _) => msg.as_str(),
                    (None, Some(msg)) => msg.as_str(),
                    _ => "Ankhimate",
                };
                ui.painter().text(
                    bar_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    center_text,
                    egui::FontId::proportional(13.0),
                    ctx.global_style().visuals.weak_text_color(),
                );

                // Right: window controls
                ui.allocate_ui_at_rect(bar_rect, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        if ui.button("🗙").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("🗖").clicked() {
                            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                        }
                        if ui.button("🗕").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                    });
                });
            });

        // ── Toolbar ──────────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(ctx.global_style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ctx, |ui| {
                crate::ui::toolbar::ui(
                    ui,
                    &mut self.state,
                    &mut self.theme,
                    &self.available_themes,
                    &mut trigger_undo,
                    &mut trigger_redo,
                );
                // Re-apply if theme changed inside toolbar
                self.theme.apply(ctx);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut behavior = AppBehavior {
                state: &mut self.state,
                theme: &self.theme,
            };
            self.tree.ui(&mut behavior, ui);
        });

        // ── Mesh trace window (T-402) ────────────────────────────────────
        crate::ui::trace::ui(ctx, &mut self.state, &self.theme);
        crate::ui::uv::ui(ctx, &mut self.state, &self.theme);

        // ── Spritesheet slicer (T-305) ───────────────────────────────────
        crate::ui::atlas::ui(ctx, &mut self.state);

        // ── Import summary (T-303) ───────────────────────────────────────
        // A conversion that quietly drops half a rig is worse than one that
        // says what it left behind, so this is a dialog, not a status line.
        if let Some(notes) = self.state.session.import_summary.clone() {
            let mut open = true;
            egui::Window::new("Import summary")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.set_max_width(460.0);
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
            if !open {
                self.state.session.import_summary = None;
            }
        }

        // ── Startup window (T-304) ───────────────────────────────────────
        // Drawn last so it sits over the editor, which stays usable behind it.
        if self.show_startup {
            match crate::ui::startup::ui(ctx, &mut self.config) {
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
                crate::ui::startup::StartupChoice::Dismiss => self.show_startup = false,
            }
        }

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
