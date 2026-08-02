//! Session state — what the user is *doing*, not what they have *made*
//! (PLAN §3.2).
//!
//! Never undoable, never saved. Selecting a different bone or panning the camera
//! must not land on the undo stack, and reopening a project should not restore
//! someone else's scroll position.

use ankhimate_core::ids::{AnimationId, AssetId, BoneId, SkinId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_core::slotmap::SecondaryMap;
use std::collections::HashSet;

/// What the user is authoring right now (T-207, ADR 0006).
///
/// The same gesture means different things in each mode, and this is the single
/// switch that decides which:
///
/// * **Setup** — define the rig. Edits mutate the [`Skeleton`] setup data, the
///   viewport always shows the setup pose (no animation applied, whatever the
///   playhead says), and structural edits are allowed.
/// * **Animate** — animate the rig. The same edits become keys on
///   [`Session::active_animation`] at [`Session::playhead`]; the setup data is
///   read-only and structural commands are refused.
///
/// [`Skeleton`]: ankhimate_core::skeleton::Skeleton
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkMode {
    #[default]
    Setup,
    Animate,
}

impl WorkMode {
    pub fn label(self) -> &'static str {
        match self {
            WorkMode::Setup => "SETUP",
            WorkMode::Animate => "ANIMATE",
        }
    }
}

/// The active canvas tool. Orthogonal to [`WorkMode`] — a tool says *how* you
/// point, the mode says *what the pointing writes to*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    CreateBone,
    WeightPaint,
}

impl Tool {
    /// Tools that author the rig itself, and so only make sense in Setup mode.
    pub fn is_setup_only(self) -> bool {
        matches!(self, Tool::CreateBone | Tool::WeightPaint)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformTool {
    Rotate,
    Translate,
    Scale,
    Shear,
}

/// What the canvas transform tools act on (T-307).
///
/// Moving the art inside its slot and moving the bone are different intents that
/// look identical as a drag, so which one is meant has to be stated rather than
/// inferred from the selection — inferring it means every stray slot selection
/// silently changes what a drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditTarget {
    #[default]
    Bone,
    Attachment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoInteraction {
    None,
    Rotate,
    TranslateFree,
    TranslateX,
    TranslateY,
    /// Dragging the red handles: rotates the bone's X axis (`shear.x`).
    ShearX,
    /// Dragging the green handles: rotates the bone's Y axis (`shear.y`).
    ShearY,
}

#[derive(Debug, Clone)]
pub struct WeightPaintSettings {
    pub radius: f32,
    pub strength: f32,
    pub mode: crate::commands::weight_cmds::BrushMode,
}

impl Default for WeightPaintSettings {
    fn default() -> Self {
        Self {
            radius: 50.0,
            // Low: weight painting is built up in passes, and a strong default
            // makes every first stroke something to undo.
            strength: 0.1,
            mode: crate::commands::weight_cmds::BrushMode::Add,
        }
    }
}

/// UI and interaction state for one open editor window.
pub struct Session {
    pub camera: crate::ui::canvas::camera::Camera2D,

    // ── Selection (multi) ────────────────────────────────────────────────
    /// Selected bones, in click order. The **last** entry is the active one that
    /// single-target panels (inspector, gizmos) operate on.
    pub selected_bones: Vec<BoneId>,
    pub selected_slots: Vec<SlotId>,
    pub hovered_bone: Option<BoneId>,

    // ── Mode & tools ─────────────────────────────────────────────────────
    /// Setup or Animate (T-207). Read it through [`Session::is_animating`]
    /// rather than comparing the field everywhere.
    pub work_mode: WorkMode,
    pub tool: Tool,
    /// Tracing knobs, kept across traces so tuning one mesh carries to the
    /// next (T-402).
    pub trace_options: crate::meshgen::TraceOptions,

    /// Mesh vertex editing is on for the selected slot's attachment (T-401).
    pub mesh_edit: bool,
    /// Vertices picked in mesh edit mode, by index into setup_vertices.
    pub selected_vertices: Vec<usize>,
    /// Where a mesh box-select began, in canvas-local screen space (T-401).
    pub vertex_box_start: Option<glam::Vec2>,
    /// Vertex being dragged, if any.
    pub dragging_vertex: Option<usize>,

    /// Whether canvas transforms drive the selected bone or the selected slot's
    /// artwork (T-307). Setup-mode concept: attachment placement is rig data.
    pub edit_target: EditTarget,
    pub active_transform_tool: TransformTool,
    pub hovered_gizmo: GizmoInteraction,
    pub dragging_gizmo: GizmoInteraction,
    pub drag_start_world_pos: Option<glam::Vec2>,
    /// In-flight bone creation: `(start_world, current_world)`.
    pub preview_bone: Option<(glam::Vec2, glam::Vec2)>,
    /// Live drag overrides, applied on top of the evaluated pose so a drag never
    /// touches the `Document` until mouse-up (PLAN §3.2, defect D7).
    pub preview_locals: SecondaryMap<BoneId, Transform>,
    pub weight_paint_settings: WeightPaintSettings,

    // ── Playback / animation ─────────────────────────────────────────────
    pub active_animation: Option<AnimationId>,
    /// Playhead position in seconds.
    pub playhead: f32,
    pub playing: bool,
    pub looping: bool,
    /// Write keys when posing (T-202). Only consulted in Animate mode, where it
    /// defaults to on; turning it off holds edits as a *pending pose* until the
    /// user presses `K`.
    pub auto_key: bool,
    /// Bones whose preview transform is an unkeyed edit waiting for `K`
    /// (Animate mode with auto-key off). Empty otherwise.
    pub pending_pose: Vec<BoneId>,
    /// Bones the user has locked against viewport edits and auto-key (T-206).
    /// Absent or `false` means editable.
    pub locked_bones: SecondaryMap<BoneId, bool>,

    // ── Rendering ────────────────────────────────────────────────────────
    /// Skin attachments resolve through (T-105).
    /// The skin edits are written to, and the highest-priority one for display.
    pub active_skin: SkinId,
    /// Extra skins layered under `active_skin` for display only (T-507).
    ///
    /// Composition is a *viewing* state, not a document one: which outfits are
    /// worn together is a question the game asks at runtime, and baking it into
    /// the rig would mean re-authoring to see a different combination. Edits
    /// always go to `active_skin` so "where did that attachment land" has one
    /// answer.
    pub layered_skins: Vec<SkinId>,
    /// Content hash per asset — the GPU texture cache key (T-301). Not the
    /// `AssetId`: slotmap keys are recycled across documents, and an id-keyed
    /// cache would draw the previous project's pixels.
    pub texture_keys: SecondaryMap<AssetId, u64>,
    /// Hashes already uploaded to the renderer this session.
    pub uploaded_textures: HashSet<u64>,
    /// Asset-panel thumbnails, keyed by name+size (see `ui::assets`).
    pub thumbnails: std::collections::HashMap<String, eframe::egui::TextureHandle>,
    /// Assets whose source file differs from what we hold (T-306). `true` means
    /// the file changed, `false` means it is gone. Only populated by an explicit
    /// check — an absent entry means "not looked at", not "fine".
    pub stale_assets: SecondaryMap<AssetId, bool>,
    /// Bone widget width in screen pixels, so it reads the same at every zoom.
    pub bone_width_pixels: f32,
    pub toolbar_horizontal: bool,

    /// Transient one-line feedback (e.g. a refused structural edit). Shown in the
    /// title bar and cleared by the next successful action.
    pub status: Option<String>,

    /// Summary from the last import (T-303): what came across and what did
    /// not. Shown as a dialog until dismissed.
    pub import_summary: Option<Vec<String>>,

    /// Has Refine been pressed for the trace in progress? Interior points
    /// should not reappear on their own after the outline is re-cut (T-402).
    pub trace_refined: bool,

    /// A mesh trace being set up in its window (T-402).
    pub pending_trace: Option<crate::ui::trace::PendingTrace>,

    /// A spritesheet waiting to be sliced (T-305). Cancelling drops it.
    pub pending_atlas: Option<crate::ui::atlas::PendingAtlas>,
    /// Seconds for the next physics step, set by the frame loop and consumed by
    /// `refresh_pose` (T-503). `None` means "still frame": evaluate the rest
    /// pose, so scrubbing stays reproducible.
    pub physics_dt: Option<f32>,
    /// Simulate physics in Setup mode, so values can be tuned without a clip.
    pub simulate_in_setup: bool,
    /// Index of the event marker being dragged in the timeline lane (T-506).
    pub dragging_event: Option<usize>,
    /// The open UV editing pane, if any (T-401).
    pub uv_pane: Option<crate::ui::uv::UvPane>,

    /// Copy buffer (T-209). Session state: copying is not an edit, so it never
    /// touches undo or a save file.
    pub clipboard: crate::clipboard::Clipboard,
}

impl Session {
    /// Skins to resolve against, in priority order: the active one first, then
    /// any layered under it. The default skin is `resolve_many`'s own fallback.
    pub fn skin_stack(&self) -> Vec<SkinId> {
        let mut stack = vec![self.active_skin];
        stack.extend(
            self.layered_skins
                .iter()
                .copied()
                .filter(|s| *s != self.active_skin),
        );
        stack
    }

    pub fn new(active_skin: SkinId) -> Self {
        Self {
            camera: crate::ui::canvas::camera::Camera2D::default(),
            selected_bones: Vec::new(),
            selected_slots: Vec::new(),
            hovered_bone: None,
            work_mode: WorkMode::Setup,
            tool: Tool::Select,
            trace_options: crate::meshgen::TraceOptions::default(),
            mesh_edit: false,
            selected_vertices: Vec::new(),
            dragging_vertex: None,
            vertex_box_start: None,
            edit_target: EditTarget::Bone,
            active_transform_tool: TransformTool::Translate,
            hovered_gizmo: GizmoInteraction::None,
            dragging_gizmo: GizmoInteraction::None,
            drag_start_world_pos: None,
            preview_bone: None,
            preview_locals: SecondaryMap::new(),
            weight_paint_settings: WeightPaintSettings::default(),
            active_animation: None,
            playhead: 0.0,
            playing: false,
            looping: true,
            auto_key: true,
            pending_pose: Vec::new(),
            locked_bones: SecondaryMap::new(),
            active_skin,
            layered_skins: Vec::new(),
            texture_keys: SecondaryMap::new(),
            uploaded_textures: HashSet::new(),
            thumbnails: std::collections::HashMap::new(),
            stale_assets: SecondaryMap::new(),
            bone_width_pixels: 7.0,
            toolbar_horizontal: false,
            status: None,
            import_summary: None,
            trace_refined: false,
            pending_trace: None,
            pending_atlas: None,
            physics_dt: None,
            simulate_in_setup: false,
            dragging_event: None,
            uv_pane: None,
            clipboard: crate::clipboard::Clipboard::Empty,
        }
    }

    /// Is the editor writing keys rather than setup data?
    pub fn is_animating(&self) -> bool {
        self.work_mode == WorkMode::Animate
    }

    /// May structural commands (create/delete/reparent/rename) run right now?
    pub fn can_edit_structure(&self) -> bool {
        self.work_mode == WorkMode::Setup
    }

    /// Are canvas transforms currently aimed at the selected slot's artwork?
    ///
    /// Only in Setup: an attachment's placement is rig data, and animating that
    /// geometry is what `Deform` timelines are for (T-404).
    pub fn editing_attachment(&self) -> bool {
        self.edit_target == EditTarget::Attachment
            && self.work_mode == WorkMode::Setup
            && self.active_slot().is_some()
    }

    /// Are there posed-but-unkeyed edits waiting for `K`?
    pub fn has_pending_pose(&self) -> bool {
        !self.pending_pose.is_empty()
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// The bone single-target UI acts on — the most recently selected.
    pub fn active_bone(&self) -> Option<BoneId> {
        self.selected_bones.last().copied()
    }

    /// The slot single-target UI acts on.
    pub fn active_slot(&self) -> Option<SlotId> {
        self.selected_slots.last().copied()
    }

    pub fn is_bone_selected(&self, bone: BoneId) -> bool {
        self.selected_bones.contains(&bone)
    }

    /// Is this bone locked against viewport edits / auto-key (T-206)?
    pub fn is_bone_locked(&self, bone: BoneId) -> bool {
        self.locked_bones.get(bone).copied().unwrap_or(false)
    }

    /// Replace the selection with a single bone (a plain click).
    pub fn select_bone(&mut self, bone: Option<BoneId>) {
        self.selected_bones.clear();
        if let Some(bone) = bone {
            self.selected_bones.push(bone);
        }
    }

    /// Toggle a bone in the selection (ctrl-click).
    pub fn toggle_bone(&mut self, bone: BoneId) {
        if let Some(i) = self.selected_bones.iter().position(|&b| b == bone) {
            self.selected_bones.remove(i);
        } else {
            self.selected_bones.push(bone);
        }
    }

    pub fn select_slot(&mut self, slot: Option<SlotId>) {
        self.selected_slots.clear();
        if let Some(slot) = slot {
            self.selected_slots.push(slot);
        }
    }

    /// Drop selection entries whose entity no longer exists. Call after any
    /// command that can delete bones or slots — a stale id would make the
    /// inspector and gizmos operate on nothing.
    pub fn prune_selection(&mut self, skeleton: &ankhimate_core::skeleton::Skeleton) {
        self.selected_bones
            .retain(|&b| skeleton.bones.contains_key(b));
        self.selected_slots
            .retain(|&s| skeleton.slots.contains_key(s));
        if let Some(h) = self.hovered_bone
            && !skeleton.bones.contains_key(h)
        {
            self.hovered_bone = None;
        }
    }

    /// Stage a live drag value for a bone without touching the document.
    pub fn set_preview_local(&mut self, bone: BoneId, local: Transform) {
        self.preview_locals.insert(bone, local);
    }

    pub fn clear_previews(&mut self) {
        self.preview_locals.clear();
        self.preview_bone = None;
        self.pending_pose.clear();
    }

    pub fn has_previews(&self) -> bool {
        !self.preview_locals.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::skeleton::{Bone, Skeleton};
    use ankhimate_core::transforms::Inherit;

    fn session() -> Session {
        Session::new(SkinId::default())
    }

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.to_string(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        }
    }

    #[test]
    fn active_bone_is_the_most_recent() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a"));
        let b = skel.add_bone(bone("b"));

        let mut s = session();
        assert!(s.active_bone().is_none());

        s.select_bone(Some(a));
        assert_eq!(s.active_bone(), Some(a));

        s.toggle_bone(b);
        assert_eq!(s.active_bone(), Some(b), "last clicked wins");
        assert_eq!(s.selected_bones.len(), 2);
    }

    #[test]
    fn select_replaces_and_toggle_accumulates() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a"));
        let b = skel.add_bone(bone("b"));

        let mut s = session();
        s.toggle_bone(a);
        s.toggle_bone(b);
        assert_eq!(s.selected_bones, vec![a, b]);

        // Toggling a selected bone deselects it.
        s.toggle_bone(a);
        assert_eq!(s.selected_bones, vec![b]);

        // A plain click clears everything else.
        s.select_bone(Some(a));
        assert_eq!(s.selected_bones, vec![a]);

        s.select_bone(None);
        assert!(s.selected_bones.is_empty());
    }

    #[test]
    fn prune_selection_drops_deleted_entities() {
        let mut skel = Skeleton::new();
        let keep = skel.add_bone(bone("keep"));
        let doomed = skel.add_bone(bone("doomed"));

        let mut s = session();
        s.toggle_bone(keep);
        s.toggle_bone(doomed);
        s.hovered_bone = Some(doomed);

        skel.remove_bone(doomed);
        s.prune_selection(&skel);

        assert_eq!(s.selected_bones, vec![keep]);
        assert!(s.hovered_bone.is_none(), "stale hover must clear");
    }

    /// Attachment editing needs all three conditions; any one missing and a drag
    /// must fall back to moving the bone rather than silently doing nothing.
    #[test]
    fn attachment_mode_requires_setup_and_a_slot() {
        use ankhimate_core::slot::Slot;
        let mut skel = Skeleton::new();
        let b = skel.add_bone(bone("a"));
        let slot = skel.add_slot(Slot::new("art".to_string(), b));

        let mut s = session();
        s.edit_target = EditTarget::Attachment;
        assert!(!s.editing_attachment(), "no slot selected yet");

        s.select_slot(Some(slot));
        assert!(s.editing_attachment());

        s.work_mode = WorkMode::Animate;
        assert!(
            !s.editing_attachment(),
            "placement is rig data — Deform keys animate the geometry instead"
        );

        s.work_mode = WorkMode::Setup;
        s.edit_target = EditTarget::Bone;
        assert!(!s.editing_attachment());
    }

    #[test]
    fn previews_are_transient() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a"));

        let mut s = session();
        assert!(!s.has_previews());
        s.set_preview_local(a, Transform::default());
        assert!(s.has_previews());
        s.clear_previews();
        assert!(!s.has_previews());
    }
}
