//! Session state — what the user is *doing*, not what they have *made*
//! (PLAN §3.2).
//!
//! Never undoable, never saved. Selecting a different bone or panning the camera
//! must not land on the undo stack, and reopening a project should not restore
//! someone else's scroll position.

use ankhimate_core::ids::{AnimationId, AssetId, BoneId, ConstraintId, SkinId, SlotId};
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

/// What the inspector is looking at (T-708).
///
/// The tree used to select only bones and slots, so an attachment or a
/// constraint could be *seen* but never inspected — you had to infer which one
/// the panel meant from which slot happened to be active. Naming the focused
/// thing makes the inspector's contents unambiguous and gives the breadcrumb
/// something to render.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Bone(BoneId),
    Slot(SlotId),
    Attachment { slot: SlotId, name: String },
    Constraint(ConstraintId),
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
    /// The weight the brush drives toward — a target, not an amount added.
    pub strength: f32,
    /// Fraction of the radius spent on the gradient. 0 is a hard stamp.
    pub feather: f32,
    pub mode: crate::commands::weight_cmds::BrushMode,
    /// Set weights from the slider instead of by painting.
    ///
    /// An alternative to the brush modes rather than a separate feature: either
    /// the pointer decides which vertices change or the selection does, and
    /// presenting them as one choice is what keeps the panel to one column.
    pub direct: bool,
    /// Bones whose weight the brush must not disturb.
    ///
    /// Per session rather than per mesh: a lock is a statement about what you
    /// are doing right now, and carrying it in the document would mean saving a
    /// file that quietly refuses edits when it is reopened.
    pub locked: Vec<ankhimate_core::ids::BoneId>,
    /// Ceiling on influences per vertex, for the Prune action.
    pub max_bones: usize,
    /// Weights below this are dropped by Prune.
    pub prune_threshold: f32,

    // ── What the canvas draws ────────────────────────────────────────────
    /// Shade the mesh by the active bone's influence.
    pub show_overlay: bool,
    /// Draw each vertex's whole influence split as a pie, rather than only its
    /// share of the active bone.
    ///
    /// The overlay answers "how much of *this* bone", the pies answer "what is
    /// on this vertex at all" — which is the question when a vertex is being
    /// pulled by something you did not expect.
    pub show_pies: bool,
    /// Restrict the vertex markers to the current mesh-edit selection.
    pub show_selected_only: bool,
}

impl Default for WeightPaintSettings {
    fn default() -> Self {
        Self {
            radius: 50.0,
            // Full: strength is the weight painted toward, so the default is
            // "bind what I paint over to this bone" — the common case. It was
            // 0.1 back when strength was an increment.
            strength: 1.0,
            // A soft edge by default. A hard-edged weight boundary creases when
            // the joint bends, and that crease is the most common weighting
            // complaint there is.
            feather: 0.7,
            mode: crate::commands::weight_cmds::BrushMode::Add,
            direct: false,
            locked: Vec::new(),
            max_bones: crate::commands::weight_cmds::DEFAULT_MAX_BONES,
            prune_threshold: crate::commands::weight_cmds::DEFAULT_PRUNE_THRESHOLD,
            show_overlay: true,
            // Off by default: a pie at every vertex on a dense mesh is more ink
            // than art, and the overlay answers the common question already.
            show_pies: false,
            show_selected_only: false,
        }
    }
}

/// UI and interaction state for one open editor window.
pub struct Session {
    pub camera: crate::ui::canvas::camera::Camera2D,

    // ── Selection (multi) ────────────────────────────────────────────────
    /// Selected bones, in click order. The **last** entry is the active one that
    /// single-target panels (inspector, gizmos) operate on.
    /// The focused item, which decides what the inspector shows (T-708).
    /// Kept alongside the bone/slot lists rather than replacing them: multi-bone
    /// selection drives posing, while this drives inspection.
    pub selection: Option<Selection>,
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
    /// Where a bone box-select began, in canvas-local screen space.
    ///
    /// Dragging from empty canvas sweeps a rectangle over the bones, which is
    /// how a limb gets selected without clicking eight sticks in turn.
    pub bone_box_start: Option<glam::Vec2>,
    /// Where a mesh box-select began, in canvas-local screen space (T-401).
    pub vertex_box_start: Option<glam::Vec2>,
    /// Vertex being dragged, if any.
    pub dragging_vertex: Option<usize>,
    /// The mesh vertex under the cursor, by index into setup_vertices (T-913).
    ///
    /// The renderer worked this out for its own highlight and threw it away each
    /// frame. Kept here so the hover label can name the vertex and list what
    /// holds it without repeating the search — and so anything else that wants
    /// "what is the pointer on" has one place to ask.
    pub hovered_vertex: Option<usize>,

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
    /// Weights copied off a mesh, waiting to be pasted onto another.
    ///
    /// Session state: a clipboard that survived a save would paste weights from
    /// a rig nobody has open.
    pub weight_clipboard: Option<Vec<Vec<ankhimate_core::attachment::VertexWeight>>>,

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
    /// A pane to bring to the front, consumed by the next frame.
    ///
    /// The panel tree belongs to the app, not to the session, so a tool that
    /// wants a tab focused leaves a request here rather than reaching across.
    pub focus_tab: Option<crate::ui::Tab>,
    /// A panel asked for the bulk-rename dialog (T-901). Consumed by the app,
    /// which owns the dialog — the tree cannot reach it directly.
    pub request_bulk_rename: bool,
    /// A panel asked to name and save the current selection (T-904).
    /// A panel asked to create a folder from the selection.
    pub request_new_group: bool,
    /// A folder edit a tree row asked for, resolved by the app.
    pub group_request: Option<crate::ui::tree::GroupRequest>,
    /// The attachment open in the slot-space editor, if any.
    pub slot_edit: Option<crate::ui::slot_editor::SlotEdit>,
    /// Which handle that pane is dragging, as a small code.
    ///
    /// A code rather than the pane's own enum: the drag has to survive between
    /// frames, and `Session` should not have to know what a corner handle is.
    pub slot_edit_grab_kind: Option<usize>,
    /// Which event the event pane is editing, as an index into the active
    /// animation's list.
    ///
    /// An index, not an id, because events have none — they are a plain vector
    /// that re-sorts whenever one is retimed. The pane clamps it every frame
    /// rather than pretending it is an identity.
    pub selected_event: Option<usize>,
    /// Slots hidden from the viewport by the tree's visibility column.
    ///
    /// A way of looking, not an edit: it never touches the document, never lands
    /// on the undo stack, and is not saved. Distinct from `Slot::attachment` being
    /// `None` (the slot shows nothing) and from a `SlotVisible` key (the
    /// animation hides it) — both of those are things the rig actually does.
    pub hidden_slots: std::collections::HashSet<SlotId>,
    /// Bones the viewport is isolated to, with their descendants (T-903).
    ///
    /// Empty means not isolated. When it is not empty, everything outside the
    /// set — bones, and the art hanging off them — dims or disappears, so one
    /// limb can be worked on without the other fifty bones drawn over it.
    ///
    /// Kept apart from `hidden_slots` on purpose, even though both end in
    /// "do not draw this". Hiding is a per-item decision the user made and
    /// expects to persist; isolation is a temporary lens on a *selection*, and
    /// leaving it entangled with hiding would mean exiting isolation had to
    /// guess which slots the user had hidden by hand beforehand.
    ///
    /// Like hiding, it is a way of looking: never touches the document, never
    /// lands on the undo stack, never serialized. That last one matters more
    /// than it sounds — a rig saved mid-isolation that reopened with most of
    /// itself missing would read as data loss.
    pub isolated_bones: std::collections::HashSet<BoneId>,
    /// Substring the hierarchy filters rows by. Empty shows everything.
    ///
    /// A 67-bone rig is not browsable by scrolling. Matching is on the name only
    /// and case-insensitive; a bone whose *descendant* matches is kept too, or
    /// filtering would orphan every match under a non-matching parent.
    pub tree_filter: String,
    /// Scroll the tree to the selected row once, then forget.
    ///
    /// Revealing on every frame pinned the panel: scrolling away snapped
    /// straight back, so a 67-bone rig could only ever be browsed near whatever
    /// happened to be selected. Set when the selection is made *elsewhere* —
    /// clicking art on the canvas, following a breadcrumb — and consumed by the
    /// first paint that finds the row.
    pub reveal_selection: bool,
    /// Draw the artwork. Off leaves the rig on its own, which is how you check
    /// a pose you cannot see for the art covering it.
    pub show_artwork: bool,
    /// Draw bones and their handles. Off is for judging the art without a
    /// skeleton drawn across it.
    ///
    /// Both also gate picking: something you cannot see must not be clickable, or
    /// hiding it only half works and the half that remains is the confusing one.
    pub show_bones: bool,
    /// The attachment under the cursor, if any: `(slot, attachment name)`.
    ///
    /// Hover has to be its own state rather than recomputed at paint time — the
    /// pick walks the draw order and tests every triangle, and doing that twice a
    /// frame to draw one outline is work for nothing.
    pub hovered_attachment: Option<(SlotId, String)>,
    /// Artwork outlines in pixel space, cached per asset name.
    ///
    /// Contour extraction decodes the image and walks every boundary, which is
    /// far too much to redo per frame. Keyed by asset rather than by attachment
    /// so two attachments sharing art share the work.
    pub silhouettes: std::collections::HashMap<String, Vec<Vec<glam::Vec2>>>,
    /// What a drag on the spritesheet preview is doing (T-305).
    pub atlas_drag: Option<crate::ui::atlas::AtlasDrag>,
    /// A PSD staged for import (T-302).
    pub pending_psd: Option<crate::ui::psd_import::PendingPsd>,
    /// Seconds for the next physics step, set by the frame loop and consumed by
    /// `refresh_pose` (T-503). `None` means "still frame": evaluate the rest
    /// pose, so scrubbing stays reproducible.
    pub physics_dt: Option<f32>,
    /// Simulate physics in Setup mode, so values can be tuned without a clip.
    pub simulate_in_setup: bool,
    /// Index of the event marker being dragged in the timeline lane (T-506).
    pub dragging_event: Option<usize>,
    /// Index of the ruler marker being dragged (T-906). Separate from
    /// `dragging_event` because the two live on different strips and a drag on
    /// one must not light up the other.
    pub dragging_marker: Option<usize>,
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
            selection: None,
            selected_bones: Vec::new(),
            selected_slots: Vec::new(),
            hovered_bone: None,
            work_mode: WorkMode::Setup,
            tool: Tool::Select,
            trace_options: crate::meshgen::TraceOptions::default(),
            mesh_edit: false,
            selected_vertices: Vec::new(),
            dragging_vertex: None,
            hovered_vertex: None,
            bone_box_start: None,
            vertex_box_start: None,
            edit_target: EditTarget::Bone,
            active_transform_tool: TransformTool::Translate,
            hovered_gizmo: GizmoInteraction::None,
            dragging_gizmo: GizmoInteraction::None,
            drag_start_world_pos: None,
            preview_bone: None,
            preview_locals: SecondaryMap::new(),
            weight_paint_settings: WeightPaintSettings::default(),
            weight_clipboard: None,
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
            focus_tab: None,
            request_bulk_rename: false,
            request_new_group: false,
            group_request: None,
            slot_edit: None,
            slot_edit_grab_kind: None,
            selected_event: None,
            hidden_slots: std::collections::HashSet::new(),
            isolated_bones: std::collections::HashSet::new(),
            tree_filter: String::new(),
            reveal_selection: false,
            show_artwork: true,
            show_bones: true,
            hovered_attachment: None,
            silhouettes: std::collections::HashMap::new(),
            atlas_drag: None,
            pending_psd: None,
            physics_dt: None,
            simulate_in_setup: false,
            dragging_event: None,
            dragging_marker: None,
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

    /// Is the viewport currently isolated to part of the rig (T-903)?
    pub fn is_isolating(&self) -> bool {
        !self.isolated_bones.is_empty()
    }

    /// Isolate the viewport to `bones` and everything under them (T-903).
    ///
    /// Descendants are included because a limb is what people mean by "this
    /// bone": isolating a shoulder and losing the arm would be a worse answer
    /// than not isolating at all.
    ///
    /// Called with an empty selection this clears isolation rather than
    /// isolating to nothing — an empty viewport is never what the user wanted,
    /// and making the keystroke a toggle costs no extra binding.
    pub fn isolate(&mut self, skeleton: &ankhimate_core::skeleton::Skeleton, bones: &[BoneId]) {
        self.isolated_bones.clear();
        for &root in bones {
            // `update_order` is topologically sorted parents-first, so one pass
            // catches a whole subtree: by the time a bone is reached its parent
            // has already been added if it belonged.
            self.isolated_bones.insert(root);
        }
        if self.isolated_bones.is_empty() {
            return;
        }
        for &id in &skeleton.update_order {
            if let Some(bone) = skeleton.bones.get(id)
                && let Some(parent) = bone.parent
                && self.isolated_bones.contains(&parent)
            {
                self.isolated_bones.insert(id);
            }
        }
    }

    /// Leave isolation, showing the whole rig again.
    pub fn clear_isolation(&mut self) {
        self.isolated_bones.clear();
    }

    /// Should this bone be drawn at full strength?
    ///
    /// True when not isolating at all, so callers can ask unconditionally rather
    /// than branching on the mode first.
    pub fn is_isolated_in(&self, bone: BoneId) -> bool {
        self.isolated_bones.is_empty() || self.isolated_bones.contains(&bone)
    }

    /// Replace the selection with a single bone (a plain click).
    /// Focus an attachment, and its slot with it — the inspector's attachment
    /// section reads the slot, and the canvas gizmos follow the slot's bone.
    /// Focus one attachment, and *only* that attachment.
    ///
    /// Deliberately clears the bone selection and switches the transform gizmo to
    /// attachment editing. Selecting a piece of art used to select its bone as
    /// well, which meant every drag moved the bone and everything else hanging
    /// off it — there was no way to nudge one image without disturbing its
    /// siblings. `bone` is still taken because the caller has it in hand and the
    /// breadcrumb walks up from the slot.
    pub fn select_attachment(&mut self, slot: SlotId, name: impl Into<String>, _bone: BoneId) {
        self.reveal_selection = true;
        self.selected_slots.clear();
        self.selected_slots.push(slot);
        self.selected_bones.clear();
        self.edit_target = EditTarget::Attachment;
        self.selection = Some(Selection::Attachment {
            slot,
            name: name.into(),
        });
    }

    /// Focus a constraint.
    pub fn select_constraint(&mut self, constraint: ConstraintId) {
        self.reveal_selection = true;
        self.selection = Some(Selection::Constraint(constraint));
    }

    pub fn select_bone(&mut self, bone: Option<BoneId>) {
        self.reveal_selection = true;
        self.selection = bone.map(Selection::Bone);
        self.selected_bones.clear();
        if let Some(bone) = bone {
            self.selected_bones.push(bone);
            // Picking a bone means you want to move the bone. The counterpart of
            // `select_attachment` switching the other way, so the gizmo always
            // acts on the thing you just clicked.
            self.edit_target = EditTarget::Bone;
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
        self.reveal_selection = true;
        self.selection = slot.map(Selection::Slot);
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
        // Deleting the last isolated bone leaves isolation on with nothing in
        // it, which draws an empty viewport and a badge that cannot be reasoned
        // about. Retaining rather than clearing outright: deleting one bone of
        // an isolated limb should not throw away the isolation.
        self.isolated_bones
            .retain(|&b| skeleton.bones.contains_key(b));
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

    /// Isolating a bone takes its whole subtree (T-903).
    ///
    /// A limb is what people mean by "this bone": isolating a shoulder and
    /// losing the arm would be a worse answer than not isolating at all.
    #[test]
    fn isolation_takes_the_whole_subtree() {
        let mut skel = Skeleton::new();
        let root = skel.add_bone(bone("root"));
        let shoulder = skel.add_bone(Bone {
            parent: Some(root),
            ..bone("shoulder")
        });
        let elbow = skel.add_bone(Bone {
            parent: Some(shoulder),
            ..bone("elbow")
        });
        let wrist = skel.add_bone(Bone {
            parent: Some(elbow),
            ..bone("wrist")
        });
        let other = skel.add_bone(Bone {
            parent: Some(root),
            ..bone("leg")
        });

        let mut s = session();
        assert!(!s.is_isolating(), "starts off");
        assert!(
            s.is_isolated_in(other),
            "not isolating means everything shows"
        );

        s.isolate(&skel, &[shoulder]);
        assert!(s.is_isolating());
        for b in [shoulder, elbow, wrist] {
            assert!(s.is_isolated_in(b), "the whole arm is in");
        }
        assert!(!s.is_isolated_in(other), "the leg is out");
        assert!(!s.is_isolated_in(root), "so is the parent");

        // Isolating nothing leaves isolation rather than isolating to an empty
        // viewport, which is never what was wanted.
        s.isolate(&skel, &[]);
        assert!(!s.is_isolating());
        assert!(s.is_isolated_in(other));
    }

    /// Isolation is a lens, never a document edit: it must not survive into a
    /// saved file, and a deleted bone must not leave it stuck on nothing.
    #[test]
    fn isolation_drops_deleted_bones() {
        let mut skel = Skeleton::new();
        let a = skel.add_bone(bone("a"));
        let b = skel.add_bone(bone("b"));

        let mut s = session();
        s.isolate(&skel, &[a, b]);
        assert_eq!(s.isolated_bones.len(), 2);

        skel.remove_bone(b);
        s.prune_selection(&skel);
        assert_eq!(s.isolated_bones.len(), 1, "stale id dropped");
        assert!(s.is_isolating(), "the surviving bone keeps it on");

        skel.remove_bone(a);
        s.prune_selection(&skel);
        assert!(
            !s.is_isolating(),
            "isolating nothing at all must switch itself off"
        );
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

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn both_layers_start_visible() {
        let skel = ankhimate_core::skeleton::Skeleton::new();
        let s = Session::new(skel.default_skin);
        assert!(s.show_artwork);
        assert!(s.show_bones);
    }

    /// Selecting art must not drag its bone along, or every transform moves the
    /// bone and everything else hanging off it.
    #[test]
    fn selecting_an_attachment_leaves_the_bone_alone() {
        use ankhimate_core::skeleton::{Bone, Skeleton};
        use ankhimate_core::slot::Slot;

        let mut skel = Skeleton::new();
        let bone = skel.add_bone(Bone {
            name: "arm".into(),
            parent: None,
            length: 10.0,
            local_transform: Default::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let slot = skel.add_slot(Slot::new("art".to_string(), bone));

        let mut s = Session::new(skel.default_skin);
        s.select_bone(Some(bone));
        assert_eq!(s.edit_target, EditTarget::Bone);

        s.select_attachment(slot, "art", bone);
        assert!(s.selected_bones.is_empty(), "the bone came along uninvited");
        assert_eq!(s.edit_target, EditTarget::Attachment);
        assert_eq!(s.active_slot(), Some(slot));

        // And back: clicking a bone returns the gizmo to the bone.
        s.select_bone(Some(bone));
        assert_eq!(s.edit_target, EditTarget::Bone);
    }
}
