//! The editor's icon vocabulary, in one place.
//!
//! Every glyph the UI draws is named here by **what it means**, not by what the
//! icon font happens to call it. Panels referred to the font crate directly,
//! which meant the icon set was spread across sixteen files and changing it was
//! sixteen edits and a guess at whether the new set had a matching name. Here it
//! is one file, and a set missing a glyph fails to compile instead of silently
//! drawing the wrong thing.
//!
//! The set is **Lucide** (ISC), vendored at `editor/assets/lucide.ttf` with its
//! licence beside it. Chosen over Phosphor and Remix for one property: a single
//! stroke weight on a single 24px grid, so a column of glyphs reads as one
//! family rather than as a pile of clip art at differing optical weights. That
//! matters more here than icon count — a rig panel is sixty rows of small
//! glyphs, and inconsistency there reads as noise.
//!
//! Codepoints live in [`super::icon_font`], which is generated. This file is the
//! part written by hand, and the only part worth reading.

use super::icon_font as f;

// ── Rig structure ───────────────────────────────────────────────────────────
pub const BONE: &str = f::BONE;
pub const SLOT: &str = f::CIRCLE_DASHED;
pub const IMAGE: &str = f::IMAGE;
pub const MESH: &str = f::VECTOR_SQUARE;
pub const CLIP: &str = f::SCISSORS;
pub const PATH: &str = f::SPLINE;
pub const HITBOX: &str = f::BOX_SELECT;
pub const POINT: &str = f::CROSSHAIR;
pub const SKIN: &str = f::SHIRT;

// ── Constraints ─────────────────────────────────────────────────────────────
pub const CONSTRAINT: &str = f::LINK;
pub const IK: &str = f::WAYPOINTS;
pub const TRANSFORM_CONSTRAINT: &str = f::MOVE_HORIZONTAL;
pub const PHYSICS: &str = f::WIND;

// ── Animation channels ──────────────────────────────────────────────────────
pub const TRANSLATE: &str = f::MOVE;
pub const ROTATE: &str = f::ROTATE_CW;
pub const SCALE: &str = f::SCALING;
pub const SHEAR: &str = f::ITALIC;
pub const COLOR: &str = f::PALETTE;
pub const READ_ONLY: &str = f::LOCK;

// ── Panels ──────────────────────────────────────────────────────────────────
pub const VIEWPORT: &str = f::FRAME;
pub const HIERARCHY: &str = f::LIST_TREE;
pub const PROPERTIES: &str = f::SLIDERS_HORIZONTAL;
pub const DOPESHEET: &str = f::FILM;
pub const GRAPH: &str = f::CHART_SPLINE;
pub const ANIMATIONS: &str = f::CLAPPERBOARD;
pub const EVENTS: &str = f::FLAG;
pub const ASSETS: &str = f::IMAGES;
/// The export panel (T-603) — a rig packaged for an engine.
pub const EXPORT: &str = f::PACKAGE;
pub const DRAW_ORDER: &str = f::LAYERS;
pub const SLOT_EDITOR: &str = f::CROP;

// ── Tools ───────────────────────────────────────────────────────────────────
pub const SELECT: &str = f::MOUSE_POINTER_2;
pub const CREATE_BONE: &str = f::BONE;
pub const WEIGHT_PAINT: &str = f::PAINTBRUSH;
pub const TOOL_TRANSLATE: &str = f::MOVE;
pub const TOOL_ROTATE: &str = f::ROTATE_CW;
pub const TOOL_SCALE: &str = f::SCALING;
pub const TOOL_SHEAR: &str = f::ITALIC;

// ── Actions ─────────────────────────────────────────────────────────────────
pub const ADD: &str = f::PLUS;
pub const DELETE: &str = f::TRASH_2;
pub const DUPLICATE: &str = f::COPY;
pub const PASTE: &str = f::CLIPBOARD;
pub const UNDO: &str = f::UNDO_2;
pub const REDO: &str = f::REDO_2;
pub const CLOSE: &str = f::X;
pub const MINIMISE: &str = f::MINUS;
pub const CLEAR: &str = f::CIRCLE_X;
pub const REFRESH: &str = f::REFRESH_CW;
pub const RESET: &str = f::ROTATE_CCW;
pub const EDIT: &str = f::PENCIL;
pub const RELINK: &str = f::LINK_2;
pub const APPLY: &str = f::ARROW_RIGHT;
pub const SAVE: &str = f::SAVE;
pub const DOWNLOAD: &str = f::DOWNLOAD;
pub const SEARCH: &str = f::SEARCH;
/// The "click something" empty state.
pub const NOTHING_SELECTED: &str = f::MOUSE_POINTER_CLICK;
/// A group in an import preview.
pub const FOLDER: &str = f::FOLDER;
pub const FIT: &str = f::MAXIMIZE;
pub const ZOOM_IN: &str = f::ZOOM_IN;
pub const ZOOM_OUT: &str = f::ZOOM_OUT;
pub const IMPORT_SHEET: &str = f::GRID_3X3;
pub const IMPORT_PSD: &str = f::LAYERS;

// ── What an import decided for you ──────────────────────────────────────────
//
// An import reads structure the file did not spell out, and the artist has to
// be able to tell what they wrote from what was guessed. These are the glyphs
// that carry that distinction.

/// A decision inference made, which the artist can overrule.
pub const INFERRED: &str = f::WAND_SPARKLES;
/// A tag read off a layer name — what the artist said, not what was guessed.
pub const TAG: &str = f::TAG;
/// A run of layers folded into one flipbook attachment.
pub const SEQUENCE: &str = f::FILM;
/// Something the import could not carry across.
pub const LOSSY: &str = f::ALERT_TRIANGLE;
/// A panel a plugin contributes.
pub const PLUGIN: &str = f::PUZZLE;
/// A group collapsed into one attachment instead of a bone with children.
pub const MERGE_GROUP: &str = f::COMBINE;
pub const LOOP: &str = f::REPEAT;
pub const AUDIO: &str = f::VOLUME_2;

// ── Inspector fields ────────────────────────────────────────────────────────
pub const WORLD: &str = f::GLOBE;
pub const ATTACHMENT: &str = f::FILE_IMAGE;
pub const TIME: &str = f::CLOCK;
pub const INTEGER: &str = f::HASH;
pub const FLOAT: &str = f::PERCENT;
pub const STRING: &str = f::TYPE;
pub const BALANCE: &str = f::MOVE_HORIZONTAL;
pub const FONT: &str = f::A_LARGE_SMALL;
pub const GRID: &str = f::GRID_3X3;
pub const PALETTE: &str = f::PALETTE;

// ── Transport ───────────────────────────────────────────────────────────────
pub const SKIP_START: &str = f::SKIP_BACK;
pub const SKIP_END: &str = f::SKIP_FORWARD;
pub const PREV_KEY: &str = f::CHEVRON_LEFT;
pub const NEXT_KEY: &str = f::CHEVRON_RIGHT;
pub const STEP_BACK: &str = f::REWIND;
pub const STEP_FORWARD: &str = f::FAST_FORWARD;
pub const PLAY: &str = f::PLAY;
pub const PAUSE: &str = f::PAUSE;
/// Auto-key: a record dot, the one metaphor everybody already reads as "what I
/// do now is being written down".
pub const RECORD: &str = f::CIRCLE_DOT;
pub const KEY: &str = f::DIAMOND;
pub const ONION_SKIN: &str = f::GHOST;

// ── Chevrons ────────────────────────────────────────────────────────────────
pub const CARET_DOWN: &str = f::CHEVRON_DOWN;
pub const CARET_RIGHT: &str = f::CHEVRON_RIGHT;
pub const CARET_UP: &str = f::CHEVRON_UP;

// ── State ───────────────────────────────────────────────────────────────────
pub const LOCKED: &str = f::LOCK;
pub const UNLOCKED: &str = f::LOCK_OPEN;
/// Shown when a row is visible or soloed.
///
/// Lucide is a single-weight outline set, so there is no filled twin to lean on:
/// on and off are told apart by *which* circle — a dot inside a ring versus a
/// bare ring — rather than by fill.
pub const DOT_ON: &str = f::CIRCLE_DOT;
/// Shown when it is not. A ring, not an empty cell: a blank column reads as
/// "nothing to toggle here", which is exactly wrong for a row you just hid.
pub const DOT_OFF: &str = f::CIRCLE;
