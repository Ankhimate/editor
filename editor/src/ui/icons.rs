//! The editor's icon vocabulary, in one place.
//!
//! Every glyph the UI draws is named here by **what it means**, not by what the
//! icon font happens to call it. Panels referred to the font crate directly,
//! which meant the icon set was spread across fifteen files and
//! changing it was fifteen edits and a guess at whether the new set had a
//! matching name. Here it is one file, and a set without some glyph fails to
//! compile instead of silently drawing the wrong thing.
//!
//! The set is **Remix Icon**, line weight. Two reasons over the previous
//! Phosphor: it draws on a 24px grid with a consistent stroke, so a column of
//! them lines up rather than each glyph sitting at its own optical weight; and
//! it has both line and fill cuts of nearly everything, so a filled variant is
//! available where one is genuinely wanted.
//!
//! Line rather than fill for the general case, reversing the earlier call. Fill
//! was chosen because outline glyphs at 12px are mostly gaps — but that is a
//! rendering-resolution problem, fixed properly by the UI scale in Settings, and
//! a panel of solid blobs loses the shape differences that make the icons worth
//! having at all.

use egui_remixicon::icons;

// ── Rig structure ───────────────────────────────────────────────────────────
pub const BONE: &str = icons::BODY_SCAN_LINE;
pub const SLOT: &str = icons::CHECKBOX_BLANK_CIRCLE_LINE;
pub const IMAGE: &str = icons::IMAGE_LINE;
pub const MESH: &str = icons::SHAPE_LINE;
pub const CLIP: &str = icons::SCISSORS_LINE;
pub const PATH: &str = icons::PEN_NIB_LINE;
pub const HITBOX: &str = icons::CROP_LINE;
pub const POINT: &str = icons::CROSSHAIR_2_LINE;
pub const SKIN: &str = icons::T_SHIRT_LINE;

// ── Constraints ─────────────────────────────────────────────────────────────
pub const CONSTRAINT: &str = icons::LINKS_LINE;
pub const IK: &str = icons::GIT_BRANCH_LINE;
pub const TRANSFORM_CONSTRAINT: &str = icons::ARROW_LEFT_RIGHT_LINE;
pub const PHYSICS: &str = icons::WINDY_LINE;

// ── Animation channels ──────────────────────────────────────────────────────
pub const TRANSLATE: &str = icons::DRAG_MOVE_2_LINE;
pub const ROTATE: &str = icons::CLOCKWISE_LINE;
pub const SCALE: &str = icons::EXPAND_DIAGONAL_LINE;
pub const SHEAR: &str = icons::PARENTHESES_LINE;
pub const COLOR: &str = icons::PALETTE_LINE;
pub const READ_ONLY: &str = icons::LOCK_LINE;

// ── Panels ──────────────────────────────────────────────────────────────────
pub const VIEWPORT: &str = icons::LAYOUT_4_LINE;
pub const HIERARCHY: &str = icons::NODE_TREE;
pub const PROPERTIES: &str = icons::EQUALIZER_LINE;
pub const DOPESHEET: &str = icons::FILM_LINE;
pub const GRAPH: &str = icons::LINE_CHART_LINE;
pub const ANIMATIONS: &str = icons::CLAPPERBOARD_LINE;
pub const EVENTS: &str = icons::FLAG_LINE;
pub const ASSETS: &str = icons::GALLERY_LINE;
pub const DRAW_ORDER: &str = icons::STACK_LINE;
pub const SLOT_EDITOR: &str = icons::CROP_2_LINE;

// ── Tools ───────────────────────────────────────────────────────────────────
pub const SELECT: &str = icons::CURSOR_LINE;
pub const CREATE_BONE: &str = icons::BODY_SCAN_LINE;
pub const WEIGHT_PAINT: &str = icons::BRUSH_LINE;
pub const TOOL_TRANSLATE: &str = icons::DRAG_MOVE_2_LINE;
pub const TOOL_ROTATE: &str = icons::CLOCKWISE_LINE;
pub const TOOL_SCALE: &str = icons::EXPAND_DIAGONAL_LINE;
pub const TOOL_SHEAR: &str = icons::PARENTHESES_LINE;

// ── Actions ─────────────────────────────────────────────────────────────────
pub const ADD: &str = icons::ADD_LINE;
pub const DELETE: &str = icons::DELETE_BIN_LINE;
pub const DUPLICATE: &str = icons::FILE_COPY_LINE;
pub const UNDO: &str = icons::ARROW_GO_BACK_LINE;
pub const REDO: &str = icons::ARROW_GO_FORWARD_LINE;
pub const CLOSE: &str = icons::CLOSE_LINE;
pub const CLEAR: &str = icons::CLOSE_CIRCLE_LINE;
pub const REFRESH: &str = icons::REFRESH_LINE;
pub const SEARCH: &str = icons::SEARCH_LINE;
/// The "click something" empty state.
pub const NOTHING_SELECTED: &str = icons::CURSOR_LINE;
/// A group in an import preview.
pub const FOLDER: &str = icons::FOLDER_LINE;
pub const FIT: &str = icons::FULLSCREEN_LINE;
pub const ZOOM_IN: &str = icons::ZOOM_IN_LINE;
pub const ZOOM_OUT: &str = icons::ZOOM_OUT_LINE;
pub const IMPORT_SHEET: &str = icons::GRID_LINE;
pub const IMPORT_PSD: &str = icons::STACK_LINE;
pub const LOOP: &str = icons::REPEAT_LINE;
pub const AUDIO: &str = icons::VOLUME_UP_LINE;

// ── Inspector fields ────────────────────────────────────────────────────────
pub const WORLD: &str = icons::GLOBE_LINE;
pub const ATTACHMENT: &str = icons::IMAGE_LINE;
pub const TIME: &str = icons::TIME_LINE;
pub const INTEGER: &str = icons::HASHTAG;
pub const FLOAT: &str = icons::PERCENT_LINE;
pub const STRING: &str = icons::TEXT;
pub const BALANCE: &str = icons::ARROW_LEFT_RIGHT_LINE;
pub const FONT: &str = icons::FONT_SIZE;
pub const GRID: &str = icons::GRID_LINE;
pub const PALETTE: &str = icons::PALETTE_LINE;

// ── Transport ───────────────────────────────────────────────────────────────
pub const SKIP_START: &str = icons::SKIP_BACK_LINE;
pub const SKIP_END: &str = icons::SKIP_FORWARD_LINE;
pub const PREV_KEY: &str = icons::ARROW_LEFT_S_LINE;
pub const NEXT_KEY: &str = icons::ARROW_RIGHT_S_LINE;
pub const STEP_BACK: &str = icons::REWIND_LINE;
pub const STEP_FORWARD: &str = icons::SPEED_LINE;
pub const PLAY: &str = icons::PLAY_LINE;
pub const PAUSE: &str = icons::PAUSE_LINE;
/// Auto-key: a record dot, which is the one metaphor everybody already reads as
/// "what I do now is being written down".
pub const RECORD: &str = icons::RECORD_CIRCLE_LINE;
pub const KEY: &str = icons::KEY_LINE;

// ── Chevrons ────────────────────────────────────────────────────────────────
pub const CARET_DOWN: &str = icons::ARROW_DOWN_S_LINE;
pub const CARET_RIGHT: &str = icons::ARROW_RIGHT_S_LINE;
pub const CARET_UP: &str = icons::ARROW_UP_S_LINE;

// ── State ───────────────────────────────────────────────────────────────────
pub const LOCKED: &str = icons::LOCK_LINE;
pub const UNLOCKED: &str = icons::LOCK_UNLOCK_LINE;
/// Shown when a row is visible or soloed.
pub const DOT_ON: &str = icons::CHECKBOX_BLANK_CIRCLE_FILL;
/// Shown when it is not — a ring, not an empty cell: a blank column reads as
/// "nothing to toggle here", which is exactly wrong for a row you just hid.
pub const DOT_OFF: &str = icons::CHECKBOX_BLANK_CIRCLE_LINE;
