//! The editor's top-level state: document + session + history + derived pose.
//!
//! This is the document/session/derived split from PLAN §3.2. `AppState` itself
//! owns no data — it is the seam that holds the four pieces together and the one
//! place that knows the frame flow:
//!
//! ```text
//! input → intent → edit_router → command → Document → evaluate(...) → Pose → renderer
//! ```
//!
//! Panels take `&mut AppState`, read from `pose`/`session`, and mutate the
//! document only via [`AppState::dispatch`] or one of the `commit_*` helpers,
//! which run the edit through [`crate::edit_router`] so Setup and Animate mode
//! stay honest (T-207).

use crate::commands::{EditCommand, History};
use crate::doc::Document;
use crate::edit_router::{self, EditIntent, Routed};
use crate::session::{Session, Tool, WorkMode};
use ankhimate_core::ids::{BoneId, SlotId};
use ankhimate_core::math::Transform;

/// Every key time on a timeline, for prev/next-key navigation.
fn key_times(timeline: &ankhimate_core::animation::Timeline) -> Vec<f32> {
    use ankhimate_core::animation::Timeline as T;
    macro_rules! times {
        ($keys:expr) => {
            $keys.iter().map(|k| k.time).collect()
        };
    }
    match timeline {
        T::BoneTranslate { keys, .. } => times!(keys),
        T::BoneRotate { keys, .. } => times!(keys),
        T::BoneScale { keys, .. } => times!(keys),
        T::BoneShear { keys, .. } => times!(keys),
        T::SlotVisible { keys, .. } => times!(keys),
        T::SlotColor { keys, .. } => times!(keys),
        T::SlotAttachment { keys, .. } => times!(keys),
        T::DrawOrder { keys } => times!(keys),
        T::IkMix { keys, .. } => times!(keys),
        T::IkBendDirection { keys, .. } => times!(keys),
        T::IkSoftness { keys, .. } => times!(keys),
        T::TransformConstraintMix { keys, .. } => times!(keys),
        T::Deform { keys, .. } => times!(keys),
    }
}

pub struct AppState {
    /// Physics simulation for the viewport (T-503, ADR 0007).
    ///
    /// Owned here rather than in the document: it is derived state that depends
    /// on elapsed wall time, so it must not be saved, undone, or shared with an
    /// exporter.
    pub physics: ankhimate_core::physics::PhysicsState,
    /// The only undoable, savable state.
    pub doc: Document,
    /// UI/interaction state — never undoable, never saved.
    pub session: Session,
    /// Undo/redo stacks of commands (not JSON snapshots).
    pub history: History,
    /// Derived pose for the current frame. Recomputed from the document by
    /// [`Self::refresh_pose`]; the viewport reads only from here.
    pub pose: ankhimate_core::pose::Pose,
}

impl Default for AppState {
    fn default() -> Self {
        let doc = Document::new();
        let session = Session::new(doc.skeleton.default_skin);
        let mut state = Self {
            doc,
            session,
            history: History::default(),
            pose: ankhimate_core::pose::Pose::new(),
            physics: ankhimate_core::physics::PhysicsState::new(),
        };
        state.refresh_pose();
        state
    }
}

impl AppState {
    /// Apply a command, record it for undo, and refresh the derived pose.
    ///
    /// The **only** sanctioned way to mutate the document. A command that
    /// declares a `requires_mode` incompatible with the current work mode is
    /// refused here (T-207) and reported in the status bar — the document is not
    /// touched and nothing lands on the undo stack.
    pub fn dispatch(&mut self, cmd: Box<dyn EditCommand>) -> bool {
        let required = cmd.requires_mode();
        if !self
            .history
            .push_in_mode(cmd, &mut self.doc, self.session.work_mode)
        {
            let hint = match required {
                Some(WorkMode::Setup) => "Switch to Setup mode to edit the rig (Tab)",
                _ => "Not available in this mode",
            };
            self.session.set_status(hint);
            return false;
        }
        self.after_document_change();
        true
    }

    /// Dispatch a batch as separate undo steps, stopping at the first refusal.
    fn dispatch_all(&mut self, cmds: Vec<Box<dyn EditCommand>>) -> bool {
        let mut ok = true;
        for cmd in cmds {
            ok &= self.dispatch(cmd);
            if !ok {
                break;
            }
        }
        ok
    }

    /// Record a command whose effect is already in the document, then refresh.
    /// For interactions that computed their result from live state.
    pub fn dispatch_applied(&mut self, cmd: Box<dyn EditCommand>) {
        self.history.push_applied(cmd);
        self.after_document_change();
    }

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.doc) {
            self.after_document_change();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.doc) {
            self.after_document_change();
        }
    }

    /// Replace the whole document — for File▸New and File▸Open.
    ///
    /// Loading is not an undoable edit: it swaps the document wholesale and
    /// clears the history, session selections, and any in-flight preview. The
    /// session is reseated on the new skeleton's default skin, back in Setup
    /// mode — a freshly opened rig is something you look at before you animate.
    pub fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.session = Session::new(self.doc.skeleton.default_skin);
        self.history = History::default();
        // Binds are derived and not serialized, so a freshly loaded weighted
        // mesh has none until this runs.
        self.rebind_meshes();
        self.refresh_pose();
    }

    // ── Work mode (T-207) ────────────────────────────────────────────────

    /// Switch between Setup and Animate.
    ///
    /// Entering Animate needs a clip: the first existing animation is selected,
    /// or one is created, rather than dropping the user into a mode where every
    /// edit would silently go nowhere. Setup-only tools fall back to Select.
    pub fn set_work_mode(&mut self, mode: WorkMode) {
        if self.session.work_mode == mode {
            return;
        }
        // Anything staged mid-gesture belongs to the old mode's semantics.
        self.session.clear_previews();

        if mode == WorkMode::Animate {
            if self.session.active_animation.is_none() {
                let existing = self.doc.animations.iter().next().map(|(id, _)| id);
                match existing {
                    Some(id) => self.session.active_animation = Some(id),
                    None => {
                        // Created while still in Setup so the command's own mode
                        // check cannot bite; CreateAnimation is mode-agnostic
                        // anyway, but the ordering keeps that a free choice.
                        self.create_animation();
                    }
                }
            }
            if self.session.tool.is_setup_only() {
                self.session.tool = Tool::Select;
            }
        } else {
            self.session.playing = false;
        }

        self.session.work_mode = mode;
        self.session.status = None;
        self.refresh_pose();
    }

    pub fn toggle_work_mode(&mut self) {
        let next = match self.session.work_mode {
            WorkMode::Setup => WorkMode::Animate,
            WorkMode::Animate => WorkMode::Setup,
        };
        self.set_work_mode(next);
    }

    /// Create a clip named `animation_N`, select it, and rewind the playhead.
    pub fn create_animation(&mut self) {
        let n = self.doc.animations.len() + 1;
        let cmd = crate::commands::key_cmds::CreateAnimation::new(format!("animation_{n}"), 1.0);
        self.dispatch(Box::new(cmd));
        if let Some((id, _)) = self.doc.animations.iter().last() {
            self.session.active_animation = Some(id);
            self.session.playhead = 0.0;
            self.refresh_pose();
        }
    }

    // ── Edit commits (routed through the mode, T-207) ────────────────────

    /// Commit a posed bone transform.
    ///
    /// Locked bones (T-206) are ignored entirely — no setup edit, no key. In
    /// Animate mode with auto-key off the value is held as a viewport preview
    /// until `K` ([`Self::key_pending_pose`]).
    pub fn commit_bone_pose(&mut self, bone: BoneId, local: Transform) {
        if self.session.is_bone_locked(bone) {
            return;
        }
        self.commit(EditIntent::BoneLocal { bone, value: local }, Some(bone));
    }

    pub fn commit_slot_color(&mut self, slot: SlotId, color: [f32; 4]) {
        self.commit(EditIntent::SlotColor { slot, value: color }, None);
    }

    pub fn commit_slot_attachment(&mut self, slot: SlotId, name: Option<String>) {
        self.commit(EditIntent::SlotAttachment { slot, value: name }, None);
    }

    /// Commit a full back-to-front slot order (draw-order panel, T-204).
    pub fn commit_draw_order(&mut self, order: Vec<SlotId>) {
        self.commit(EditIntent::DrawOrder { order }, None);
    }

    /// Run an intent through the router and act on the verdict.
    ///
    /// `pending_bone` is the bone whose preview should stay on screen if the
    /// router defers the edit (auto-key off).
    fn commit(&mut self, intent: EditIntent, pending_bone: Option<BoneId>) {
        // Keep the value around for the Pending branch before the intent moves.
        let pending_value = match (&intent, pending_bone) {
            (EditIntent::BoneLocal { value, .. }, Some(_)) => Some(*value),
            _ => None,
        };

        match edit_router::route(intent, &self.doc, &self.session) {
            Routed::Commands(cmds) => {
                self.dispatch_all(cmds);
            }
            Routed::Pending => {
                if let (Some(bone), Some(value)) = (pending_bone, pending_value) {
                    self.session.set_preview_local(bone, value);
                    if !self.session.pending_pose.contains(&bone) {
                        self.session.pending_pose.push(bone);
                    }
                    self.session
                        .set_status("Unkeyed pose — press K to key it, or enable auto-key");
                    self.refresh_pose();
                }
            }
            Routed::Refused(reason) => self.session.set_status(reason),
        }
    }

    /// Key every pending (posed-but-unkeyed) bone at the playhead — the `K`
    /// shortcut. No-op outside Animate mode or when nothing is pending.
    pub fn key_pending_pose(&mut self) {
        if !self.session.is_animating() || !self.session.has_pending_pose() {
            return;
        }
        let pending: Vec<(BoneId, Transform)> = self
            .session
            .pending_pose
            .iter()
            .filter_map(|&bone| self.session.preview_locals.get(bone).map(|t| (bone, *t)))
            .collect();

        self.session.clear_previews();
        for (bone, local) in pending {
            if self.session.is_bone_locked(bone) {
                continue;
            }
            match edit_router::route_forced(
                EditIntent::BoneLocal { bone, value: local },
                &self.doc,
                &self.session,
            ) {
                Routed::Commands(cmds) => {
                    self.dispatch_all(cmds);
                }
                Routed::Pending => {}
                Routed::Refused(reason) => self.session.set_status(reason),
            }
        }
        self.refresh_pose();
    }

    // ── Clipboard (T-209) ────────────────────────────────────────────────

    /// Copy the selected bones — the whole subtree under each, with their slots
    /// and artwork.
    ///
    /// Copying a bone without its children would produce a paste that silently
    /// loses half the limb, so the subtree is the unit whether or not the user
    /// selected every part of it.
    pub fn copy_selection(&mut self) {
        use crate::clipboard::{BoneClip, ClipBone, ClipSlot, Clipboard};

        let roots = self.selection_roots();
        if roots.is_empty() {
            self.session.set_status("Nothing selected to copy");
            return;
        }

        let mut clip = BoneClip::default();
        // Bone id → index in the clip, so children can point at their parent.
        let mut index_of: std::collections::HashMap<BoneId, usize> =
            std::collections::HashMap::new();

        for root in roots {
            for id in self.subtree(root) {
                let Some(bone) = self.doc.skeleton.bones.get(id) else {
                    continue;
                };
                let parent = bone.parent.and_then(|p| index_of.get(&p).copied());
                index_of.insert(id, clip.bones.len());
                clip.bones.push(ClipBone {
                    bone: bone.clone(),
                    parent,
                });
            }
        }

        for (slot_id, slot) in self.doc.skeleton.slots.iter() {
            let Some(&bone_index) = index_of.get(&slot.bone) else {
                continue;
            };
            let entries = self
                .doc
                .skeleton
                .skins
                .iter()
                .flat_map(|(_, skin)| {
                    let skin_name = skin.name.clone();
                    skin.names_for_slot(slot_id)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .filter_map(move |name| {
                            skin.get(slot_id, &name)
                                .map(|a| (skin_name.clone(), name, a.clone()))
                        })
                })
                .collect();
            clip.slots.push(ClipSlot {
                slot: slot.clone(),
                bone: bone_index,
                entries,
            });
        }

        let summary = format!("Copied {} bone(s)", clip.bones.len());
        self.session.clipboard = Clipboard::Bones(clip);
        self.session.set_status(summary);
    }

    /// Copy the current pose of the selected bones (or the whole rig when
    /// nothing is selected) — the walk-cycle workhorse.
    pub fn copy_pose(&mut self) {
        use crate::clipboard::{Clipboard, PoseClip};

        let bones: Vec<BoneId> = if self.session.selected_bones.is_empty() {
            self.doc.skeleton.update_order.clone()
        } else {
            self.session.selected_bones.clone()
        };

        let entries = bones
            .into_iter()
            .filter_map(|id| {
                let name = self.doc.skeleton.bones.get(id)?.name.clone();
                let local = self.pose.locals.get(id).copied()?;
                Some((name, local))
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            self.session.set_status("No pose to copy");
            return;
        }
        let summary = format!("Copied a pose ({} bones)", entries.len());
        self.session.clipboard = Clipboard::Pose(PoseClip { entries });
        self.session.set_status(summary);
    }

    /// Paste whatever is on the clipboard. `mirrored` only affects a pose.
    pub fn paste(&mut self, mirrored: bool) {
        use crate::clipboard::Clipboard;

        match self.session.clipboard.clone() {
            Clipboard::Empty => self.session.set_status("Clipboard is empty"),
            Clipboard::Bones(clip) => {
                // Paste under the active bone so the gesture reads as "another
                // one of these, here"; with nothing selected they become roots.
                let parent = self.session.active_bone();
                let cmd = crate::commands::bone_cmds::PasteBones::new(clip, parent);
                if self.dispatch(Box::new(cmd)) {
                    // Select what was just pasted: it is what the user will edit.
                    if let Some(id) = self.doc.skeleton.update_order.last().copied() {
                        self.session.select_bone(Some(id));
                    }
                    self.session.set_status("Pasted bones");
                }
            }
            Clipboard::Pose(clip) => {
                let clip = if mirrored { clip.mirrored() } else { clip };
                let mut applied = 0;
                for (name, local) in clip.entries {
                    // Resolved by name every paste, so undo/redo id churn cannot
                    // land a pose on the wrong bone.
                    let Some((id, _)) =
                        self.doc.skeleton.bones.iter().find(|(_, b)| b.name == name)
                    else {
                        continue;
                    };
                    // Routed: setup edit in Setup, keys at the playhead in
                    // Animate — pasting a pose onto a frame *is* how you key it.
                    self.commit_bone_pose(id, local);
                    applied += 1;
                }
                let what = if mirrored { "mirrored pose" } else { "pose" };
                self.session
                    .set_status(format!("Pasted {what} onto {applied} bone(s)"));
            }
        }
    }

    /// Duplicate the selected subtree in place (Ctrl+D).
    pub fn duplicate_selection(&mut self) {
        let previous = std::mem::take(&mut self.session.clipboard);
        self.copy_selection();
        if !self.session.clipboard.is_empty() {
            // Paste as a sibling, not a child: duplicating a limb should not
            // nest it inside itself.
            let parent = self
                .session
                .active_bone()
                .and_then(|id| self.doc.skeleton.bones.get(id))
                .and_then(|b| b.parent);
            if let crate::clipboard::Clipboard::Bones(clip) = self.session.clipboard.clone() {
                let cmd = crate::commands::bone_cmds::PasteBones::new(clip, parent);
                if self.dispatch(Box::new(cmd))
                    && let Some(id) = self.doc.skeleton.update_order.last().copied()
                {
                    self.session.select_bone(Some(id));
                }
                self.session.set_status("Duplicated");
            }
        }
        self.session.clipboard = previous;
    }

    /// Selected bones with any that are descendants of another selected bone
    /// dropped — copying a parent already carries its children.
    fn selection_roots(&self) -> Vec<BoneId> {
        let selected = &self.session.selected_bones;
        selected
            .iter()
            .copied()
            .filter(|&id| {
                !selected
                    .iter()
                    .any(|&other| other != id && self.doc.skeleton.is_descendant(id, other))
            })
            .collect()
    }

    /// `root` and every descendant, parents before children.
    fn subtree(&self, root: BoneId) -> Vec<BoneId> {
        self.doc
            .skeleton
            .update_order
            .iter()
            .copied()
            .filter(|&id| id == root || self.doc.skeleton.is_descendant(id, root))
            .collect()
    }

    // ── Pose tools (T-211) ───────────────────────────────────────────────

    /// Bake the displayed pose into the setup skeleton (selection, or all bones).
    pub fn set_pose_as_setup(&mut self) {
        let bones = self.target_bones();
        let targets: Vec<(BoneId, Transform)> = bones
            .into_iter()
            .filter_map(|id| self.pose.locals.get(id).map(|t| (id, *t)))
            .collect();
        if targets.is_empty() {
            self.session.set_status("No pose to bake");
            return;
        }
        let count = targets.len();
        if self.dispatch(Box::new(crate::commands::bone_cmds::SetPoseAsSetup::new(
            targets,
        ))) {
            self.session
                .set_status(format!("Set the pose of {count} bone(s) as setup"));
        }
    }

    /// Clear rotation, scale and shear on the selection (positions are kept —
    /// they are rig structure, not pose).
    pub fn reset_bones(&mut self) {
        let bones = self.target_bones();
        if bones.is_empty() {
            self.session.set_status("Nothing to reset");
            return;
        }
        let count = bones.len();
        if self.dispatch(Box::new(crate::commands::bone_cmds::ResetBones::new(bones))) {
            self.session.set_status(format!("Reset {count} bone(s)"));
        }
    }

    /// Remove the active clip's animation for the selected bones.
    pub fn clear_bone_animation(&mut self) {
        let Some(anim) = self.session.active_animation else {
            self.session.set_status("No animation selected");
            return;
        };
        let bones = self.target_bones();
        if bones.is_empty() {
            self.session.set_status("Nothing to clear");
            return;
        }
        let count = bones.len();
        if self.dispatch(Box::new(
            crate::commands::key_cmds::ClearBoneAnimation::new(anim, bones),
        )) {
            self.session
                .set_status(format!("Cleared animation on {count} bone(s)"));
        }
    }

    /// Multiply every key time in the active clip (and its duration).
    pub fn scale_animation_timing(&mut self, factor: f32) {
        let Some(anim) = self.session.active_animation else {
            return;
        };
        if self.dispatch(Box::new(
            crate::commands::key_cmds::RetimeAnimation::scaled(anim, factor),
        )) {
            self.session
                .set_status(format!("Scaled timing by {factor:.2}×"));
        }
    }

    /// Shift every key in the active clip by `frames`.
    pub fn offset_animation_keys(&mut self, frames: i32) {
        let Some(anim) = self.session.active_animation else {
            return;
        };
        let seconds = frames as f32 / self.doc.meta.fps.max(1) as f32;
        if self.dispatch(Box::new(
            crate::commands::key_cmds::RetimeAnimation::offset(anim, seconds),
        )) {
            self.session
                .set_status(format!("Offset keys by {frames} frame(s)"));
        }
    }

    /// The bones a pose tool acts on: the selection, or the whole rig when
    /// nothing is selected — "do this to everything" is the common case, and
    /// requiring a select-all first would be busywork.
    fn target_bones(&self) -> Vec<BoneId> {
        if self.session.selected_bones.is_empty() {
            self.doc.skeleton.update_order.clone()
        } else {
            self.session.selected_bones.clone()
        }
    }

    // ── Playback ─────────────────────────────────────────────────────────

    /// Advance the playhead by `dt` seconds of wall-clock time while playing.
    ///
    /// Playback lives entirely in the editor: core `evaluate` stays a pure
    /// function of `(skeleton, animations, time)` (PLAN §2.6 determinism), so the
    /// only state that moves is `Session.playhead`. Returns `true` if the pose
    /// changed and the viewport needs a repaint.
    pub fn advance_playback(&mut self, dt: f32) -> bool {
        // Physics runs while playing, and — when asked — while tuning values in
        // Setup with no clip to play (T-503). Both go through the same field so
        // `refresh_pose` has one rule.
        let simulating = self.session.playing
            || (self.session.simulate_in_setup && !self.session.is_animating());
        if simulating && !self.physics.paused {
            self.session.physics_dt = Some(dt);
        }
        if simulating && !self.session.playing {
            // Nothing else advances in Setup, so the repaint has to come from
            // here or the simulation would only step when something else
            // happened to redraw.
            self.refresh_pose();
            return true;
        }
        if !self.session.playing {
            return false;
        }
        let Some(duration) = self
            .session
            .active_animation
            .and_then(|id| self.doc.animations.get(id))
            .map(|a| a.duration.max(0.0))
        else {
            self.session.playing = false;
            return false;
        };
        if duration <= 0.0 {
            return false;
        }

        let mut t = self.session.playhead + dt;
        if t >= duration {
            if self.session.looping {
                // Wrap, preserving the overshoot so playback speed is steady.
                t %= duration;
            } else {
                t = duration;
                self.session.playing = false;
            }
        }
        self.set_playhead(t);
        true
    }

    /// Move the playhead, dropping any unkeyed pose (it belonged to the frame
    /// the user has just left).
    pub fn set_playhead(&mut self, time: f32) {
        if self.session.has_pending_pose() {
            self.session.clear_previews();
        }
        self.session.playhead = time.max(0.0);
        self.refresh_pose();
    }

    /// Step the playhead by `frames` (negative = back), clamped to the clip.
    pub fn step_frames(&mut self, frames: i32) {
        let fps = self.doc.meta.fps.max(1) as f32;
        let duration = self
            .session
            .active_animation
            .and_then(|id| self.doc.animations.get(id))
            .map(|a| a.duration.max(0.0))
            .unwrap_or(0.0);
        let t = self.session.playhead + frames as f32 / fps;
        self.session.playing = false;
        self.set_playhead(t.clamp(0.0, duration));
    }

    /// Jump to the previous/next key across all timelines of the active clip.
    pub fn jump_key(&mut self, forward: bool) {
        let Some(anim) = self
            .session
            .active_animation
            .and_then(|id| self.doc.animations.get(id))
        else {
            return;
        };
        let now = self.session.playhead;
        let mut times: Vec<f32> = anim.timelines.iter().flat_map(key_times).collect();
        times.sort_by(f32::total_cmp);
        let target = if forward {
            times.into_iter().find(|&t| t > now + 1e-5)
        } else {
            times.into_iter().rev().find(|&t| t < now - 1e-5)
        };
        if let Some(t) = target {
            self.session.playing = false;
            self.set_playhead(t);
        }
    }

    // ── Mesh binds (T-403) ───────────────────────────────────────────────

    /// Capture inverse bind matrices for any weighted mesh that lacks them.
    ///
    /// Binds are `#[serde(skip)]` derived state — they come from the **setup**
    /// pose, so they can always be recomputed and must be, after a load or any
    /// change to a mesh's influences. Capturing at setup is also what stops a
    /// newly painted bone from yanking the mesh: at the setup pose every bone's
    /// `world × inverse_bind` is the identity, so adding an influence moves
    /// nothing until the rig is actually posed.
    pub fn rebind_meshes(&mut self) {
        use ankhimate_core::attachment::Attachment;

        let needs_binds = self.doc.skeleton.skins.iter().any(|(_, skin)| {
            skin.entries.values().any(|a| match a {
                Attachment::Mesh(mesh) => {
                    !mesh.weights.is_empty() && mesh.inverse_bind_matrices.is_empty()
                }
                // Clips carry no weights, so they never need binds.
                Attachment::Region(_) | Attachment::Clipping(_) => false,
            })
        });
        if !needs_binds {
            return;
        }

        // The setup pose, not the current one: binds must not depend on where
        // the playhead happens to be.
        let mut setup = ankhimate_core::pose::Pose::new();
        ankhimate_core::pose::evaluate(&self.doc.skeleton, &[], &mut setup);

        for (_, skin) in self.doc.skeleton.skins.iter_mut() {
            for attachment in skin.entries.values_mut() {
                if let Attachment::Mesh(mesh) = attachment
                    && !mesh.weights.is_empty()
                    && mesh.inverse_bind_matrices.is_empty()
                {
                    mesh.bind_to_pose(&setup);
                }
            }
        }
    }

    // ── Derived pose ─────────────────────────────────────────────────────

    /// Post-mutation housekeeping: drop selections pointing at deleted entities,
    /// then recompute the pose.
    fn after_document_change(&mut self) {
        self.session.prune_selection(&self.doc.skeleton);
        // Cheap when nothing needs it: the scan short-circuits unless a weighted
        // mesh is missing its binds.
        self.rebind_meshes();
        self.refresh_pose();
    }

    /// Recompute [`Self::pose`] from the document, then lay any in-flight drag
    /// previews over the top.
    ///
    /// In **Setup** mode no animation is applied at all, whatever the playhead
    /// says: the setup pose is what you are authoring, and a rig that drifted
    /// with the timeline would be unauthorable (T-207).
    ///
    /// Previews are applied to `Pose.locals` and the world pass re-run, so a drag
    /// looks live without the document being touched until mouse-up (defect D7).
    pub fn refresh_pose(&mut self) {
        // `update_order` is `#[serde(skip)]`, so a deserialized skeleton arrives
        // with it empty; `evaluate` walks it directly and would produce an empty
        // pose.
        if self.doc.skeleton.update_order.len() != self.doc.skeleton.bones.len() {
            self.doc.skeleton.rebuild_update_order();
        }

        // Borrow the document immutably and the pose mutably at the same time by
        // splitting the fields explicitly.
        let Self {
            doc,
            session,
            pose,
            physics,
            ..
        } = self;

        // `(animation, time, alpha)` for the current playhead — Animate mode
        // only; Setup mode always evaluates the bare setup pose.
        let animations: Vec<(&ankhimate_core::animation::Animation, f32, f32)> =
            match session.work_mode {
                WorkMode::Animate => match session
                    .active_animation
                    .and_then(|id| doc.animations.get(id))
                {
                    Some(anim) => vec![(anim, session.playhead, 1.0)],
                    None => Vec::new(),
                },
                WorkMode::Setup => Vec::new(),
            };
        // Physics advances only while something is actually moving: during
        // playback, or when the rigger has asked to simulate while tuning
        // values in Setup (T-503). A still frame evaluates to the rest pose, so
        // scrubbing the timeline is reproducible.
        let dt = session.physics_dt.take().unwrap_or(0.0);
        if dt > 0.0 {
            ankhimate_core::pose::evaluate_with(&doc.skeleton, &animations, physics, dt, pose);
        } else {
            ankhimate_core::pose::evaluate(&doc.skeleton, &animations, pose);
        }

        if self.session.has_previews() {
            self.apply_previews();
        }
    }

    /// Overlay `Session::preview_locals` onto the evaluated pose and re-run the
    /// world pass for the affected bones and their descendants.
    fn apply_previews(&mut self) {
        for (bone, local) in self.session.preview_locals.iter() {
            if self.pose.locals.contains_key(bone) {
                self.pose.locals.insert(bone, *local);
            }
        }
        // Cheapest correct thing: recompose every world affine along the
        // topological order. A 500-bone rig is T-706's benchmark target; if this
        // shows up there, narrow it to the previewed subtrees.
        ankhimate_core::pose::recompose_worlds(&self.doc.skeleton, &mut self.pose);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::bone_cmds::{CreateBone, DeleteBone, SetBoneTransform};
    use crate::commands::key_cmds::{
        AddKey, BoneProperty, CreateAnimation, KeyValue, TimelineAddr,
    };
    use ankhimate_core::animation::Interp;
    use ankhimate_core::math::Transform;
    use ankhimate_core::skeleton::Bone;

    fn bone(name: &str) -> Bone {
        Bone {
            name: name.to_string(),
            parent: None,
            length: 10.0,
            local_transform: Transform::default(),
            inherit: Default::default(),
            color: Bone::default_color(),
        }
    }

    /// A one-bone rig with a one-second clip selected, in Animate mode.
    fn animating_state() -> (AppState, BoneId) {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        state.dispatch(Box::new(CreateAnimation::new("clip", 1.0)));
        state.set_work_mode(WorkMode::Animate);
        (state, root)
    }

    // ── T-207 acceptance ─────────────────────────────────────────────────

    /// The same gesture is a setup edit in Setup mode…
    #[test]
    fn setup_mode_routes_a_pose_to_the_skeleton() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        assert_eq!(state.session.work_mode, WorkMode::Setup, "default is Setup");

        let posed = Transform {
            position: glam::vec2(12.0, -3.0),
            ..Default::default()
        };
        state.commit_bone_pose(root, posed);

        assert_eq!(state.doc.skeleton.bones[root].local_transform, posed);
        assert_eq!(state.history.undo_label(), Some("Transform Bone"));
    }

    /// …and a key in Animate mode, leaving the setup skeleton byte-identical.
    #[test]
    fn animate_mode_routes_a_pose_to_keys_and_leaves_setup_untouched() {
        use ankhimate_core::animation::Timeline;
        let (mut state, root) = animating_state();
        let setup = state.doc.skeleton.bones[root].local_transform;
        let anim = state.session.active_animation.unwrap();

        state.set_playhead(0.4);
        state.commit_bone_pose(
            root,
            Transform {
                rotation: setup.rotation + std::f32::consts::FRAC_PI_2,
                ..setup
            },
        );

        assert_eq!(
            state.doc.skeleton.bones[root].local_transform, setup,
            "animating must never touch the setup pose"
        );
        let rotate = state.doc.animations[anim]
            .timelines
            .iter()
            .find_map(|t| match t {
                Timeline::BoneRotate { bone, keys } if *bone == root => Some(keys),
                _ => None,
            })
            .expect("rotate timeline created");
        assert_eq!(rotate.len(), 2, "t=0 baseline + the posed key");
        assert!((rotate[1].time - 0.4).abs() < 1e-4);
        assert!((rotate[1].value - 90.0).abs() < 0.5);
    }

    /// A structural command in Animate mode is refused outright — the document
    /// is unchanged and nothing lands on the undo stack.
    #[test]
    fn structural_commands_are_refused_while_animating() {
        let (mut state, root) = animating_state();
        let bones_before = state.doc.skeleton.bones.len();
        let depth_before = state.history.undo_depth();

        assert!(!state.dispatch(Box::new(CreateBone::new(bone("late")))));
        assert!(!state.dispatch(Box::new(DeleteBone::new(root))));

        assert_eq!(state.doc.skeleton.bones.len(), bones_before);
        assert_eq!(state.history.undo_depth(), depth_before);
        assert!(state.session.status.is_some(), "user was told why");
    }

    /// Setup mode shows the setup pose regardless of where the playhead is.
    #[test]
    fn setup_mode_ignores_the_playhead() {
        let (mut state, root) = animating_state();
        let setup_rot = state.doc.skeleton.bones[root].local_transform.rotation;
        let anim = state.session.active_animation.unwrap();
        let addr = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Rotate,
        };
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.0,
            KeyValue::Scalar(0.0),
            Interp::Linear,
        )));
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr,
            1.0,
            KeyValue::Scalar(90.0),
            Interp::Linear,
        )));

        state.set_playhead(1.0);
        let animated = state.pose.locals[root].rotation;
        assert!(
            (animated - setup_rot).to_degrees().abs() > 45.0,
            "animating"
        );

        state.set_work_mode(WorkMode::Setup);
        assert!(
            (state.pose.locals[root].rotation - setup_rot).abs() < 1e-5,
            "Setup mode evaluates the setup pose even at playhead 1.0"
        );
    }

    /// Entering Animate with no clip creates and selects one rather than
    /// dropping the user somewhere every edit would vanish.
    #[test]
    fn entering_animate_mode_guarantees_a_clip() {
        let mut state = AppState::default();
        assert!(state.doc.animations.is_empty());

        state.set_work_mode(WorkMode::Animate);
        assert!(state.session.active_animation.is_some());
        assert_eq!(state.doc.animations.len(), 1);

        // A second round trip reuses the existing clip.
        state.set_work_mode(WorkMode::Setup);
        state.set_work_mode(WorkMode::Animate);
        assert_eq!(state.doc.animations.len(), 1);
    }

    /// T-210: the dot state tracks what the clip actually holds at the playhead.
    #[test]
    fn key_state_reports_timeline_and_key_presence() {
        use crate::commands::key_cmds::{BoneProperty, TimelineAddr};
        use crate::edit_router::{KeyState, key_state};

        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();
        let addr = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Rotate,
        };

        // Nothing animates rotation yet.
        assert_eq!(
            key_state(&state.doc, &state.session, &addr),
            KeyState::NoTimeline
        );

        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.0,
            KeyValue::Scalar(0.0),
            Interp::Linear,
        )));
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.5,
            KeyValue::Scalar(90.0),
            Interp::Linear,
        )));

        state.set_playhead(0.5);
        assert_eq!(
            key_state(&state.doc, &state.session, &addr),
            KeyState::Keyed(1),
            "keyed at the playhead, and the index is the one a delete removes"
        );

        state.set_playhead(0.25);
        assert_eq!(
            key_state(&state.doc, &state.session, &addr),
            KeyState::Unkeyed,
            "between keys: the timeline exists but this frame is not keyed"
        );

        // A different property is still untouched.
        let translate = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Translate,
        };
        assert_eq!(
            key_state(&state.doc, &state.session, &translate),
            KeyState::NoTimeline
        );
    }

    /// The value a dot-click keys is the posed value as an offset from setup —
    /// the same thing the timeline's "set key" captures.
    #[test]
    fn bone_key_value_is_the_offset_from_setup() {
        use crate::commands::key_cmds::BoneProperty;
        use crate::edit_router::bone_key_value;

        let (mut state, root) = animating_state();
        let setup = state.doc.skeleton.bones[root].local_transform;
        state.session.auto_key = false;
        state.set_playhead(0.5);
        state.commit_bone_pose(
            root,
            Transform {
                rotation: setup.rotation + std::f32::consts::FRAC_PI_2,
                ..setup
            },
        );

        match bone_key_value(&state.doc, &state.pose, root, BoneProperty::Rotate) {
            Some(KeyValue::Scalar(degrees)) => {
                assert!((degrees - 90.0).abs() < 0.5, "got {degrees}")
            }
            other => panic!("expected a scalar rotation key, got {other:?}"),
        }
    }

    // ── T-404 deform timelines ───────────────────────────────────────────

    /// A deform key moves the drawn shape without touching the authored mesh,
    /// and survives a save/load round trip.
    #[test]
    fn deform_keys_animate_the_mesh_and_round_trip() {
        use crate::commands::key_cmds::AddDeformKey;
        use crate::commands::mesh_cmds::ConvertToMesh;
        use crate::commands::slot_cmds::CreateSlot;
        use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = state.doc.skeleton.update_order[0];
        state.dispatch(Box::new(CreateSlot::new("flag", root)));
        let slot = state.doc.skeleton.draw_order[0];
        let skin = state.doc.skeleton.default_skin;

        state.doc.skeleton.skins[skin].set(
            slot,
            "flag",
            Attachment::Region(RegionAttachment {
                texture: "flag".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 100.0,
                height: 100.0,
                uv_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                pivot: glam::Vec2::splat(0.5),
            }),
        );
        if let Some(s) = state.doc.skeleton.slots.get_mut(slot) {
            s.attachment = Some("flag".into());
        }
        state.dispatch(Box::new(ConvertToMesh::new(skin, slot, "flag")));
        let setup_vertices = match state.doc.skeleton.skins[skin].get(slot, "flag") {
            Some(Attachment::Mesh(mesh)) => mesh.setup_vertices.clone(),
            other => panic!("expected a mesh, got {other:?}"),
        };

        state.dispatch(Box::new(CreateAnimation::new("wave", 1.0)));
        state.set_work_mode(WorkMode::Animate);
        let anim = state.session.active_animation.unwrap();

        // Flat at t=0, one corner pushed out at t=0.5 — the waving-flag shape.
        state.dispatch(Box::new(AddDeformKey::new(
            anim,
            slot,
            "flag",
            0.0,
            vec![glam::Vec2::ZERO; 4],
        )));
        state.dispatch(Box::new(AddDeformKey::new(
            anim,
            slot,
            "flag",
            0.5,
            vec![
                glam::vec2(0.0, 20.0),
                glam::Vec2::ZERO,
                glam::Vec2::ZERO,
                glam::Vec2::ZERO,
            ],
        )));

        state.set_playhead(0.5);
        let deform = state
            .pose
            .deforms
            .get(&(slot, "flag".to_string()))
            .expect("the clip deforms this attachment");
        assert_eq!(deform[0], glam::vec2(0.0, 20.0));

        // Halfway between the keys the offset interpolates.
        state.set_playhead(0.25);
        let half = state.pose.deforms.get(&(slot, "flag".to_string())).unwrap()[0];
        assert!((half.y - 10.0).abs() < 0.5, "interpolated: {half:?}");

        // The authored mesh is untouched — the clip only says how far it strays.
        match state.doc.skeleton.skins[skin].get(slot, "flag") {
            Some(Attachment::Mesh(mesh)) => {
                assert_eq!(mesh.setup_vertices, setup_vertices, "setup shape preserved")
            }
            other => panic!("expected a mesh, got {other:?}"),
        }

        // Round trip.
        let json = ankhimate_formats::to_json(&state.doc.as_project_ref()).expect("serialize");
        let loaded = ankhimate_formats::from_json(&json).expect("deserialize");
        let mut reopened = AppState::default();
        reopened.replace_document(Document {
            skeleton: loaded.skeleton,
            animations: loaded.animations,
            assets: loaded.assets,
            meta: crate::doc::DocumentMeta {
                name: loaded.name,
                fps: loaded.fps,
            },
        });
        reopened.set_work_mode(WorkMode::Animate);
        reopened.set_playhead(0.5);

        let slot2 = reopened.doc.skeleton.draw_order[0];
        let deform = reopened
            .pose
            .deforms
            .get(&(slot2, "flag".to_string()))
            .expect("deform survived the round trip");
        assert_eq!(deform[0], glam::vec2(0.0, 20.0));
    }

    // ── T-403 weights ────────────────────────────────────────────────────

    /// The acceptance property: binding a vertex to a bone must not move it.
    ///
    /// Binds are captured from the **setup** pose, so at setup every
    /// `world × inverse_bind` is the identity and a new influence contributes
    /// exactly the vertex's own position. Capturing from the *current* pose
    /// instead is the classic way to make a mesh lurch the moment it is painted.
    #[test]
    fn painting_weights_does_not_move_the_mesh() {
        use crate::commands::mesh_cmds::ConvertToMesh;
        use crate::commands::slot_cmds::CreateSlot;
        use crate::commands::weight_cmds::{PaintWeights, brush};
        use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = state.doc.skeleton.update_order[0];
        state.dispatch(Box::new(CreateSlot::new("art", root)));
        let slot = state.doc.skeleton.draw_order[0];

        let skin = state.doc.skeleton.default_skin;
        state.doc.skeleton.skins[skin].set(
            slot,
            "art",
            Attachment::Region(RegionAttachment {
                texture: "art".into(),
                local_offset: glam::Vec2::ZERO,
                local_rotation: 0.0,
                local_scale: glam::Vec2::ONE,
                width: 100.0,
                height: 100.0,
                uv_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                },
                pivot: glam::Vec2::splat(0.5),
            }),
        );
        if let Some(s) = state.doc.skeleton.slots.get_mut(slot) {
            s.attachment = Some("art".into());
        }
        state.dispatch(Box::new(ConvertToMesh::new(skin, slot, "art")));

        let vertex_before = {
            let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot, "art")
            else {
                panic!("expected a mesh");
            };
            mesh.setup_vertices[0]
        };

        // Paint the root bone fully onto every vertex.
        let weights = {
            let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot, "art")
            else {
                panic!("expected a mesh");
            };
            brush(mesh, root, Default::default(), &[0.0; 4], 50.0, 1.0)
        };
        state.dispatch(Box::new(PaintWeights::new(skin, slot, "art", weights)));

        // Binds were recaptured by `after_document_change`.
        let Some(Attachment::Mesh(mesh)) = state.doc.skeleton.skins[skin].get(slot, "art") else {
            panic!("expected a mesh");
        };
        assert!(
            !mesh.inverse_bind_matrices.is_empty(),
            "binds were captured after painting"
        );

        let skinned = mesh.skin_vertex_with_ffd(0, glam::Vec2::ZERO, &state.pose);
        assert!(
            (skinned - vertex_before).length() < 1e-3,
            "the vertex must not move when it gains an influence: {skinned:?} vs {vertex_before:?}"
        );
    }

    // ── T-211 pose tools ─────────────────────────────────────────────────

    /// Baking writes whatever the viewport is showing into the setup skeleton —
    /// including an in-flight preview, which is the whole point: pose until it
    /// looks right, then make that the rest pose.
    #[test]
    fn set_pose_as_setup_bakes_what_is_on_screen() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("arm"))));
        let arm = state.doc.skeleton.update_order[0];
        let authored = state.doc.skeleton.bones[arm].local_transform;

        // A drag in progress: the pose shows it, the document does not.
        let dragged = Transform {
            rotation: 0.8,
            position: glam::vec2(12.0, 0.0),
            ..authored
        };
        state.session.set_preview_local(arm, dragged);
        state.refresh_pose();
        assert_eq!(state.doc.skeleton.bones[arm].local_transform, authored);

        state.session.select_bone(Some(arm));
        state.set_pose_as_setup();

        let baked = state.doc.skeleton.bones[arm].local_transform;
        assert!(
            (baked.rotation - 0.8).abs() < 1e-5,
            "got {}",
            baked.rotation
        );
        assert_eq!(baked.position, glam::vec2(12.0, 0.0));

        state.undo();
        assert_eq!(state.doc.skeleton.bones[arm].local_transform, authored);
    }

    /// Reset clears rotation/scale/shear but keeps the bone where it is: the
    /// offset from its parent is structure, and collapsing the rig onto the
    /// origin is never what "reset" means.
    #[test]
    fn reset_keeps_position() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("arm"))));
        let arm = state.doc.skeleton.update_order[0];
        state.commit_bone_pose(
            arm,
            Transform {
                position: glam::vec2(30.0, 4.0),
                rotation: 0.7,
                scale: glam::vec2(2.0, 2.0),
                ..Default::default()
            },
        );

        state.session.select_bone(Some(arm));
        state.reset_bones();

        let t = state.doc.skeleton.bones[arm].local_transform;
        assert_eq!(t.position, glam::vec2(30.0, 4.0), "position kept");
        assert_eq!(t.rotation, 0.0);
        assert_eq!(t.scale, glam::Vec2::ONE);

        state.undo();
        assert!((state.doc.skeleton.bones[arm].local_transform.rotation - 0.7).abs() < 1e-5);
    }

    /// Retiming scales every key and the duration together.
    #[test]
    fn scaling_timing_moves_keys_and_duration() {
        use ankhimate_core::animation::Timeline;

        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();
        let addr = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Rotate,
        };
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.0,
            KeyValue::Scalar(0.0),
            Interp::Linear,
        )));
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr,
            0.5,
            KeyValue::Scalar(90.0),
            Interp::Linear,
        )));

        state.scale_animation_timing(2.0);

        assert!((state.doc.animations[anim].duration - 2.0).abs() < 1e-4);
        match &state.doc.animations[anim].timelines[0] {
            Timeline::BoneRotate { keys, .. } => {
                assert!((keys[1].time - 1.0).abs() < 1e-4, "key moved with the clip")
            }
            other => panic!("expected a rotate timeline, got {other:?}"),
        }

        state.undo();
        assert!((state.doc.animations[anim].duration - 1.0).abs() < 1e-4);
    }

    /// Clearing drops the selected bones' timelines and leaves the rest alone.
    #[test]
    fn clearing_animation_spares_other_bones() {
        use ankhimate_core::animation::Timeline;

        let (mut state, root) = animating_state();
        state.set_work_mode(WorkMode::Setup);
        state.dispatch(Box::new(CreateBone::new(bone("other"))));
        let other = state
            .doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "other")
            .map(|(id, _)| id)
            .unwrap();
        state.set_work_mode(WorkMode::Animate);
        let anim = state.session.active_animation.unwrap();

        for bone in [root, other] {
            state.dispatch(Box::new(AddKey::new(
                anim,
                TimelineAddr::Bone {
                    bone,
                    property: BoneProperty::Rotate,
                },
                0.0,
                KeyValue::Scalar(10.0),
                Interp::Linear,
            )));
        }
        assert_eq!(state.doc.animations[anim].timelines.len(), 2);

        state.session.select_bone(Some(root));
        state.clear_bone_animation();

        let remaining = &state.doc.animations[anim].timelines;
        assert_eq!(remaining.len(), 1, "only the selected bone was cleared");
        match &remaining[0] {
            Timeline::BoneRotate { bone, .. } => assert_eq!(*bone, other),
            other => panic!("expected a rotate timeline, got {other:?}"),
        }

        state.undo();
        assert_eq!(state.doc.animations[anim].timelines.len(), 2);
    }

    // ── T-209 clipboard ──────────────────────────────────────────────────

    /// Copying a bone takes its whole subtree — pasting half a limb would be
    /// worse than refusing.
    #[test]
    fn copy_paste_carries_the_whole_subtree() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a_root"))));
        let root = state.doc.skeleton.update_order[0];
        let mut child = bone("b_child");
        child.parent = Some(root);
        state.dispatch(Box::new(CreateBone::new(child)));

        state.session.select_bone(Some(root));
        state.copy_selection();
        state.paste(false);

        // 2 original + 2 pasted, and the copy's names were uniquified.
        assert_eq!(state.doc.skeleton.bones.len(), 4);
        let names: Vec<String> = state
            .doc
            .skeleton
            .bones
            .iter()
            .map(|(_, b)| b.name.clone())
            .collect();
        assert!(names.iter().any(|n| n == "a_root_2"), "got {names:?}");
        assert!(names.iter().any(|n| n == "b_child_2"), "got {names:?}");

        // The pasted subtree keeps its own shape: the copied child hangs off the
        // copied root, not off the original.
        let pasted_root = state
            .doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "a_root_2")
            .map(|(id, _)| id)
            .unwrap();
        let pasted_child = state
            .doc
            .skeleton
            .bones
            .iter()
            .find(|(_, b)| b.name == "b_child_2")
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(
            state.doc.skeleton.bones[pasted_child].parent,
            Some(pasted_root)
        );

        state.undo();
        assert_eq!(
            state.doc.skeleton.bones.len(),
            2,
            "one undo removes the paste"
        );
    }

    /// Pasting a pose in Animate mode keys it — that is the whole point of
    /// copying a pose from one frame to another.
    #[test]
    fn pasting_a_pose_while_animating_writes_keys() {
        use ankhimate_core::animation::Timeline;

        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();
        let setup = state.doc.skeleton.bones[root].local_transform;

        // Pose and copy at frame 0.
        state.set_playhead(0.0);
        state.commit_bone_pose(
            root,
            Transform {
                position: glam::vec2(25.0, 0.0),
                ..setup
            },
        );
        state.session.select_bone(Some(root));
        state.copy_pose();

        // Paste it later in the clip.
        state.set_playhead(0.75);
        state.paste(false);

        let translate = state.doc.animations[anim]
            .timelines
            .iter()
            .find_map(|t| match t {
                Timeline::BoneTranslate { bone, keys } if *bone == root => Some(keys),
                _ => None,
            })
            .expect("translate timeline");
        assert!(
            translate.iter().any(|k| (k.time - 0.75).abs() < 1e-4),
            "the paste landed as a key at the playhead"
        );
        assert_eq!(
            state.doc.skeleton.bones[root].local_transform, setup,
            "and the setup pose was not touched"
        );
    }

    /// Paste-mirrored flips X translation and rotation — the other half of a
    /// walk cycle, without re-posing by hand.
    #[test]
    fn paste_mirrored_flips_x_and_rotation() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("arm"))));
        let arm = state.doc.skeleton.update_order[0];

        state.commit_bone_pose(
            arm,
            Transform {
                position: glam::vec2(20.0, 5.0),
                rotation: 0.4,
                ..Default::default()
            },
        );
        state.session.select_bone(Some(arm));
        state.copy_pose();
        state.paste(true);

        let posed = state.doc.skeleton.bones[arm].local_transform;
        assert_eq!(posed.position, glam::vec2(-20.0, 5.0));
        assert!((posed.rotation + 0.4).abs() < 1e-5);
    }

    /// Copying with nothing selected says so rather than silently doing nothing.
    #[test]
    fn copying_an_empty_selection_reports_it() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        state.session.select_bone(None);

        state.copy_selection();
        assert!(state.session.clipboard.is_empty());
        assert!(state.session.status.is_some());
    }

    /// Setup-only tools do not survive the switch into Animate.
    #[test]
    fn setup_only_tools_fall_back_to_select() {
        let mut state = AppState::default();
        state.session.tool = Tool::CreateBone;
        state.set_work_mode(WorkMode::Animate);
        assert_eq!(state.session.tool, Tool::Select);
    }

    /// Auto-key off holds the pose as a preview; `K` commits it as keys.
    #[test]
    fn auto_key_off_defers_the_pose_until_k() {
        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();
        state.session.auto_key = false;
        state.set_playhead(0.5);

        let posed = Transform {
            position: glam::vec2(40.0, 0.0),
            ..Default::default()
        };
        state.commit_bone_pose(root, posed);

        assert!(state.session.has_pending_pose(), "held as pending");
        assert!(
            state.doc.animations[anim].timelines.is_empty(),
            "nothing keyed yet"
        );
        // ...but the viewport shows it.
        assert_eq!(state.pose.world_position(root).x, 40.0);

        state.key_pending_pose();
        assert!(!state.session.has_pending_pose());
        assert!(
            !state.doc.animations[anim].timelines.is_empty(),
            "K wrote the keys"
        );
    }

    /// Moving the playhead abandons an unkeyed pose — it belonged to the frame
    /// the user just left.
    #[test]
    fn scrubbing_discards_a_pending_pose() {
        let (mut state, root) = animating_state();
        state.session.auto_key = false;
        state.set_playhead(0.5);
        state.commit_bone_pose(
            root,
            Transform {
                position: glam::vec2(40.0, 0.0),
                ..Default::default()
            },
        );
        assert!(state.session.has_pending_pose());

        state.set_playhead(0.6);
        assert!(!state.session.has_pending_pose());
        assert_eq!(state.pose.world_position(root).x, 0.0);
    }

    /// The draw-order panel's two behaviors, now selected by mode instead of by
    /// the auto-key flag (T-204 amendment).
    #[test]
    fn draw_order_routes_by_mode() {
        use crate::commands::slot_cmds::CreateSlot;

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        state.dispatch(Box::new(CreateSlot::new("back", root)));
        state.dispatch(Box::new(CreateSlot::new("front", root)));
        let order = state.doc.skeleton.draw_order.clone();
        let (back, front) = (order[0], order[1]);

        // Setup: edits the setup order.
        state.commit_draw_order(vec![front, back]);
        assert_eq!(state.doc.skeleton.draw_order, vec![front, back]);
        state.undo();
        assert_eq!(state.doc.skeleton.draw_order, vec![back, front]);

        // Animate: writes a key, setup order untouched.
        state.dispatch(Box::new(CreateAnimation::new("swap", 1.0)));
        state.set_work_mode(WorkMode::Animate);
        state.set_playhead(0.5);
        state.commit_draw_order(vec![front, back]);

        assert_eq!(
            state.doc.skeleton.draw_order,
            vec![back, front],
            "setup order untouched by the key"
        );
        assert_eq!(state.pose.draw_order, vec![front, back], "swapped at time");
        state.set_playhead(0.0);
        assert_eq!(
            state.pose.draw_order,
            vec![back, front],
            "baseline key holds the setup stack before the swap"
        );
    }

    // ── Pre-existing behavior (T-201 … T-206) ────────────────────────────

    /// T-201 acceptance: an animation authored via commands drives the viewport
    /// pose when the playhead scrubs across it. A rotation timeline 0°→90° over
    /// one second must read 45° at the midpoint and 90° at the end.
    #[test]
    fn scrubbing_a_keyed_animation_moves_the_pose() {
        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();

        let addr = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Rotate,
        };
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.0,
            KeyValue::Scalar(0.0),
            Interp::Linear,
        )));
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr,
            1.0,
            KeyValue::Scalar(90.0),
            Interp::Linear,
        )));

        state.set_playhead(0.5);
        let mid = state.pose.locals[root].rotation.to_degrees();
        assert!((mid - 45.0).abs() < 0.5, "midpoint ~45°, got {mid}");

        state.set_playhead(1.0);
        let end = state.pose.locals[root].rotation.to_degrees();
        assert!((end - 90.0).abs() < 0.5, "end ~90°, got {end}");
    }

    /// T-202 acceptance (headless): posing at t>0 while animating writes keys
    /// (plus a t=0 baseline) instead of editing setup; playback interpolates.
    #[test]
    fn auto_key_records_a_pose_and_playback_interpolates() {
        let (mut state, root) = animating_state();
        let setup = state.doc.skeleton.bones[root].local_transform;

        state.set_playhead(0.5);
        state.commit_bone_pose(
            root,
            Transform {
                rotation: setup.rotation + std::f32::consts::FRAC_PI_2,
                ..setup
            },
        );

        state.set_playhead(0.25);
        let mid = ankhimate_core::transforms::wrap_angle(
            state.pose.locals[root].rotation - setup.rotation,
        )
        .to_degrees();
        assert!((mid - 45.0).abs() < 1.0, "interpolated ~45°, got {mid}");
    }

    /// T-203 acceptance: applying the Ease In-Out preset to a rotation key
    /// visibly changes the motion, and undo restores the linear timing.
    #[test]
    fn ease_in_out_changes_motion_and_undo_reverts() {
        use crate::commands::key_cmds::{SetInterp, presets};
        let (mut state, root) = animating_state();
        let anim = state.session.active_animation.unwrap();
        let setup_rot = state.doc.skeleton.bones[root].local_transform.rotation;

        let addr = TimelineAddr::Bone {
            bone: root,
            property: BoneProperty::Rotate,
        };
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            0.0,
            KeyValue::Scalar(0.0),
            Interp::Linear,
        )));
        state.dispatch(Box::new(AddKey::new(
            anim,
            addr.clone(),
            1.0,
            KeyValue::Scalar(90.0),
            Interp::Linear,
        )));

        state.set_playhead(0.25);
        let linear = (state.pose.locals[root].rotation - setup_rot).to_degrees();

        let ease = presets::all()
            .into_iter()
            .find(|(l, _)| *l == "Ease In-Out")
            .map(|(_, i)| i)
            .unwrap();
        let refs = vec![crate::commands::key_cmds::KeyRef { addr, index: 1 }];
        state.dispatch(Box::new(SetInterp::new(anim, refs, ease)));
        state.refresh_pose();
        let eased = (state.pose.locals[root].rotation - setup_rot).to_degrees();

        assert!(
            (eased - linear).abs() > 2.0,
            "ease-in-out must differ from linear at t=0.25 (linear={linear}, eased={eased})"
        );

        state.undo();
        let reverted = (state.pose.locals[root].rotation - setup_rot).to_degrees();
        assert!(
            (reverted - linear).abs() < 0.5,
            "undo restored linear timing"
        );
    }

    /// T-205 acceptance: a blink (slot alpha keys) and a mouth-swap (attachment
    /// keys) both replay correctly after a save/load round-trip.
    #[test]
    fn slot_color_and_attachment_animate_and_survive_round_trip() {
        use crate::commands::key_cmds::{AddAttachmentKey, AddKey};
        use crate::commands::slot_cmds::CreateSlot;

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        state.dispatch(Box::new(CreateSlot::new("mouth", root)));
        let slot = state.doc.skeleton.draw_order[0];

        state.dispatch(Box::new(CreateAnimation::new("talk", 1.0)));
        state.set_work_mode(WorkMode::Animate);
        let anim = state.session.active_animation.unwrap();

        // Blink: alpha 1 → 0 → 1 over the clip.
        for (t, a) in [(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)] {
            state.dispatch(Box::new(AddKey::new(
                anim,
                TimelineAddr::SlotColor { slot },
                t,
                KeyValue::Color([1.0, 1.0, 1.0, a]),
                Interp::Linear,
            )));
        }
        // Mouth swap: "open" then "shut".
        state.dispatch(Box::new(AddAttachmentKey::new(
            anim,
            slot,
            0.0,
            Some("open".into()),
        )));
        state.dispatch(Box::new(AddAttachmentKey::new(
            anim,
            slot,
            0.5,
            Some("shut".into()),
        )));

        // Round-trip through the on-disk format.
        let json = ankhimate_formats::to_json(&state.doc.as_project_ref()).expect("serialize");
        let loaded = ankhimate_formats::from_json(&json).expect("deserialize");

        let mut reopened = AppState::default();
        reopened.replace_document(Document {
            skeleton: loaded.skeleton,
            animations: loaded.animations,
            assets: loaded.assets,
            meta: crate::doc::DocumentMeta {
                name: loaded.name,
                fps: loaded.fps,
            },
        });
        reopened.set_work_mode(WorkMode::Animate);
        let slot2 = reopened.doc.skeleton.draw_order[0];

        // Blink midpoint: alpha ~0.
        reopened.set_playhead(0.5);
        let alpha = reopened.pose.slot_colors[slot2][3];
        assert!(alpha < 0.05, "blink closed at t=0.5, alpha={alpha}");

        // Attachment stepped: "open" before 0.5, "shut" at/after.
        reopened.set_playhead(0.25);
        assert_eq!(
            reopened.pose.slot_attachments[slot2].as_deref(),
            Some("open")
        );
        reopened.set_playhead(0.75);
        assert_eq!(
            reopened.pose.slot_attachments[slot2].as_deref(),
            Some("shut")
        );
    }

    /// Playback advances the playhead and wraps when looping.
    #[test]
    fn playback_advances_and_loops() {
        let (mut state, _root) = animating_state();
        state.session.looping = true;
        state.session.playing = true;
        state.session.playhead = 0.9;

        assert!(state.advance_playback(0.05));
        assert!((state.session.playhead - 0.95).abs() < 1e-4);

        // Cross the end: 0.95 + 0.1 = 1.05 → wraps to 0.05.
        state.advance_playback(0.1);
        assert!((state.session.playhead - 0.05).abs() < 1e-4, "looped");
        assert!(state.session.playing, "still playing after a loop");

        // Non-looping stops at the end.
        state.session.looping = false;
        state.session.playhead = 0.95;
        state.advance_playback(0.2);
        assert!((state.session.playhead - 1.0).abs() < 1e-4);
        assert!(!state.session.playing, "stopped at the end");
    }

    /// T-206 acceptance: reparenting a posed bone via `keeping_world` leaves its
    /// rendered (world) pose identical.
    #[test]
    fn reparent_keeps_world_pose_identical() {
        use crate::commands::bone_cmds::SetBoneParent;

        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        state.dispatch(Box::new(CreateBone::new(bone("b"))));
        let order = state.doc.skeleton.update_order.clone();
        let (a, b) = (order[0], order[1]);

        state.dispatch(Box::new(SetBoneTransform::new(
            a,
            Transform {
                position: glam::vec2(30.0, 10.0),
                rotation: 0.6,
                ..Default::default()
            },
        )));
        state.dispatch(Box::new(SetBoneTransform::new(
            b,
            Transform {
                position: glam::vec2(-20.0, 40.0),
                rotation: -0.3,
                scale: glam::vec2(1.5, 1.5),
                ..Default::default()
            },
        )));

        let world_before = state.pose.world(b);

        state.dispatch(Box::new(SetBoneParent::keeping_world(
            &state.doc.skeleton,
            b,
            Some(a),
        )));
        assert_eq!(state.doc.skeleton.bones[b].parent, Some(a), "reparented");

        let world_after = state.pose.world(b);
        for (x, y) in [
            (world_before.a, world_after.a),
            (world_before.b, world_after.b),
            (world_before.c, world_after.c),
            (world_before.d, world_after.d),
            (world_before.tx, world_after.tx),
            (world_before.ty, world_after.ty),
        ] {
            assert!((x - y).abs() < 1e-3, "world affine changed: {x} vs {y}");
        }
    }

    /// A locked bone ignores viewport commits entirely (T-206).
    #[test]
    fn locked_bone_ignores_pose_commits() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("root"))));
        let root = *state.doc.skeleton.update_order.first().unwrap();
        state.session.locked_bones.insert(root, true);

        let before = state.doc.skeleton.bones[root].local_transform;
        state.commit_bone_pose(
            root,
            Transform {
                position: glam::vec2(99.0, 0.0),
                ..before
            },
        );
        assert_eq!(
            state.doc.skeleton.bones[root].local_transform, before,
            "locked bone must not move"
        );
    }

    #[test]
    fn dispatch_refreshes_the_pose() {
        let mut state = AppState::default();
        assert_eq!(state.pose.worlds.len(), 0);

        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        assert_eq!(state.pose.worlds.len(), 1, "pose picked up the new bone");
    }

    #[test]
    fn undo_refreshes_the_pose_and_prunes_selection() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        let id = *state.doc.skeleton.update_order.first().unwrap();
        state.session.select_bone(Some(id));

        state.undo();
        assert_eq!(state.pose.worlds.len(), 0, "pose no longer has the bone");
        assert!(
            state.session.selected_bones.is_empty(),
            "selection must not point at a deleted bone"
        );
    }

    #[test]
    fn deleting_a_selected_bone_clears_the_selection() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        let id = *state.doc.skeleton.update_order.first().unwrap();
        state.session.select_bone(Some(id));

        state.dispatch(Box::new(DeleteBone::new(id)));
        assert!(state.session.selected_bones.is_empty());
        assert!(state.session.active_bone().is_none());
    }

    #[test]
    fn previews_move_the_pose_without_touching_the_document() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        let id = *state.doc.skeleton.update_order.first().unwrap();

        let authored = state.doc.skeleton.bones[id].local_transform;
        state.session.set_preview_local(
            id,
            Transform {
                position: glam::vec2(50.0, 0.0),
                ..authored
            },
        );
        state.refresh_pose();

        assert_eq!(state.pose.world_position(id).x, 50.0);
        assert_eq!(state.doc.skeleton.bones[id].local_transform, authored);
        assert_eq!(state.history.undo_depth(), 1, "only the create");
    }

    #[test]
    fn clearing_previews_returns_the_pose_to_the_document() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        let id = *state.doc.skeleton.update_order.first().unwrap();

        state.session.set_preview_local(
            id,
            Transform {
                position: glam::vec2(50.0, 0.0),
                ..Default::default()
            },
        );
        state.refresh_pose();
        assert_eq!(state.pose.world_position(id).x, 50.0);

        state.session.clear_previews();
        state.refresh_pose();
        assert_eq!(state.pose.world_position(id).x, 0.0);
    }

    #[test]
    fn preview_propagates_to_child_bones() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a_parent"))));
        let parent = *state.doc.skeleton.update_order.first().unwrap();

        let mut child = bone("b_child");
        child.parent = Some(parent);
        child.local_transform.position = glam::vec2(10.0, 0.0);
        state.dispatch(Box::new(CreateBone::new(child)));
        let child_id = state.doc.skeleton.update_order[1];

        assert_eq!(state.pose.world_position(child_id).x, 10.0);

        state.session.set_preview_local(
            parent,
            Transform {
                position: glam::vec2(100.0, 0.0),
                ..Default::default()
            },
        );
        state.refresh_pose();
        assert_eq!(state.pose.world_position(child_id).x, 110.0);
    }

    #[test]
    fn drag_then_commit_is_one_undo_step() {
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        let id = *state.doc.skeleton.update_order.first().unwrap();
        let before = state.history.undo_depth();

        for x in 1..=10 {
            state.session.set_preview_local(
                id,
                Transform {
                    position: glam::vec2(x as f32, 0.0),
                    ..Default::default()
                },
            );
            state.refresh_pose();
        }
        assert_eq!(state.history.undo_depth(), before, "drag pushed nothing");

        let final_local = state.session.preview_locals[id];
        state.session.clear_previews();
        state.commit_bone_pose(id, final_local);
        assert_eq!(state.history.undo_depth(), before + 1);
        assert_eq!(
            state.doc.skeleton.bones[id].local_transform.position.x,
            10.0
        );
    }

    #[test]
    fn acceptance_create_three_pose_delete_then_undo_four_times() {
        // T-107 acceptance script: create 3 bones, pose one, delete one,
        // undo x4 restores exactly.
        let mut state = AppState::default();

        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        state.dispatch(Box::new(CreateBone::new(bone("b"))));
        state.dispatch(Box::new(CreateBone::new(bone("c"))));
        assert_eq!(state.doc.skeleton.bones.len(), 3);

        let order = state.doc.skeleton.update_order.clone();
        let (a, b) = (order[0], order[1]);

        let posed = Transform {
            position: glam::vec2(25.0, -5.0),
            rotation: 0.5,
            ..Default::default()
        };
        state.dispatch(Box::new(SetBoneTransform::new(a, posed)));
        assert_eq!(state.doc.skeleton.bones[a].local_transform, posed);

        state.dispatch(Box::new(DeleteBone::new(b)));
        assert_eq!(state.doc.skeleton.bones.len(), 2);
        assert_eq!(state.history.undo_depth(), 5);

        state.undo();
        assert_eq!(state.doc.skeleton.bones.len(), 3);
        state.undo();
        assert_eq!(
            state.doc.skeleton.bones[a].local_transform,
            Transform::default()
        );
        state.undo();
        assert_eq!(state.doc.skeleton.bones.len(), 2);
        state.undo();
        assert_eq!(state.doc.skeleton.bones.len(), 1);

        state.undo();
        assert_eq!(state.doc.skeleton.bones.len(), 0);
        assert!(!state.history.can_undo());
        assert_eq!(state.pose.worlds.len(), 0, "pose tracked every step");
    }

    #[test]
    fn history_holds_commands_not_json() {
        // Guards the memory claim in the T-107 acceptance criteria.
        let mut state = AppState::default();
        state.dispatch(Box::new(CreateBone::new(bone("a"))));
        assert_eq!(state.history.undo_depth(), 1);
    }
}
