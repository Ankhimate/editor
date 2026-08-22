//! Name the thing under the cursor (T-913).
//!
//! A dense rig is a field of near-identical sticks and overlapping quads. The
//! editor already knows which one the pointer is over — hover has been tracked
//! since T-708 for outlines and gizmo highlighting — but until now it spent that
//! knowledge on a colour change and nothing else. Colour says "this one"; it
//! does not say *which* one, and answering that meant clicking, reading the
//! hierarchy, and losing your place.
//!
//! So: a label at the cursor, naming what is under it.
//!
//! # What it says
//!
//! A **breadcrumb trail**, outermost first, the thing under the cursor last:
//!
//! ```text
//! 🦴 hip › 🦴 front-thigh › ⭕ front-foot › 🖼 front-foot
//! ```
//!
//! Each crumb carries the icon the hierarchy uses for that kind, so the two
//! panels read as one vocabulary and the kind is legible without a word spent
//! naming it. That matters more than it sounds: in `samples/spineboy.ankh` a
//! bone, a slot and a mesh all answer to `front-foot`, and the trail above
//! distinguishes all three at a glance.
//!
//! A trail rather than a stack of "child of X" / "in slot Y" sentences, which is
//! what this drew first. The prose said the same thing in more words, and in a
//! different shape for each kind — so nothing lined up between one hover and the
//! next, and finding the answer meant reading rather than looking. One shape for
//! every kind puts the answer in the same place every time.
//!
//! Two extras below the trail earn their space by answering a question the
//! viewport otherwise cannot:
//!
//! * **Constraints driving the hovered bone.** A bone that will not move when
//!   you drag it is nearly always a bone something else is driving, and there is
//!   no other cheap way to discover that.
//! * **A vertex's influences.** Bone name and weight for each, in the per-mesh
//!   rank colours the weight overlay uses, so "why is this vertex moving" is
//!   answered where it is asked instead of in a panel.
//!
//! # What keeps it from becoming noise
//!
//! A label that follows every mouse movement is worse than no label: it strobes
//! while you pass over a rig, and it covers the thing you are trying to look at.
//! Four rules, each fixing one of those:
//!
//! * a delay before it appears, reset whenever the target changes, so sweeping
//!   across a rig shows nothing;
//! * offset from the cursor and flipped near a viewport edge, so it never covers
//!   its own subject and never leaves the screen;
//! * suppressed entirely during a drag — the cursor is busy, and a label that
//!   still names what it was over before the drag started is actively lying;
//! * a config toggle, plus a modifier that summons it immediately for users who
//!   would rather ask than be told.

use crate::app_state::AppState;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::BoneId;
use eframe::egui;

/// How long the pointer must rest on one thing before it is named.
///
/// Long enough that crossing a rig on the way somewhere else stays silent, short
/// enough that stopping to ask feels answered rather than waited on.
const DELAY: f32 = 0.35;

/// Gap between the cursor and the label's nearest corner.
///
/// The pointer is drawn from its tip, so a label placed at the raw cursor
/// position sits under the arrow. This clears it.
const CURSOR_OFFSET: egui::Vec2 = egui::vec2(16.0, 18.0);

const PADDING: egui::Vec2 = egui::vec2(7.0, 5.0);
const LINE_GAP: f32 = 2.0;
const TITLE_SIZE: f32 = 12.0;
const DETAIL_SIZE: f32 = 10.5;
/// Side of the colour chip drawn beside an influence row.
const SWATCH: f32 = 7.0;

/// Most influences to list on a vertex before summarising the rest.
///
/// A vertex over the influence budget is a fault the mesh overlay already rings;
/// the label's job is to stay readable, not to print all fourteen of them.
const MAX_INFLUENCES: usize = 6;

/// Most crumbs to show before eliding the far end of the trail.
///
/// Four fits a deep rig's useful context — bone, its parent, its slot, the
/// attachment — without the label growing wider than the art it sits on.
const MAX_TRAIL: usize = 4;

/// Drawn between crumbs.
const SEPARATOR: &str = "›";

/// Space between the pieces of the trail — icon to name, name to separator.
const TRAIL_GAP: f32 = 4.0;

/// What the cursor is over, resolved to something worth printing.
struct Target {
    /// Where the thing sits, outermost first, the thing itself last.
    ///
    /// A trail rather than a stack of "child of X" / "in slot Y" sentences. The
    /// prose said the same thing in more words and in a different shape for each
    /// kind, so nothing lined up between one hover and the next; a trail is one
    /// shape for every kind, and the eye finds the last crumb — the thing under
    /// the cursor — in the same place every time.
    trail: Vec<Crumb>,
    /// Lines under the trail: constraints, influences. Only things that are not
    /// ancestry — anything that *is* ancestry belongs in the trail.
    details: Vec<Detail>,
    /// Identity for the hover timer. Two targets with the same key are the same
    /// thing, and the delay must not restart between frames.
    key: String,
}

/// One step of the trail: an icon and a name.
struct Crumb {
    icon: &'static str,
    text: String,
}

impl Crumb {
    fn new(icon: &'static str, text: impl Into<String>) -> Self {
        Self {
            icon,
            text: text.into(),
        }
    }
}

/// One line under the trail, optionally with a colour chip.
struct Detail {
    text: String,
    swatch: Option<egui::Color32>,
}

impl Detail {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            swatch: None,
        }
    }
}

/// Draw the label, if anything is under the cursor and nothing forbids it.
///
/// Called last in the canvas pass so the label sits above the artwork, the
/// gizmos and the weight overlay. It is the one thing on the canvas that should
/// never be occluded — a tooltip behind a bone is not a tooltip.
pub fn draw(ui: &mut egui::Ui, rect: egui::Rect, state: &AppState, enabled: bool) {
    if !enabled && !summoned(ui) {
        return;
    }
    // A drag owns the cursor. Whatever is under the pointer mid-drag is not what
    // the drag is about, so naming it is worse than saying nothing.
    if is_dragging(state) || ui.ctx().input(|i| i.pointer.any_down()) {
        return;
    }
    let Some(cursor) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    if !rect.contains(cursor) {
        return;
    }
    let Some(target) = resolve(state) else {
        return;
    };

    // The delay is per-target: moving from one bone to the next restarts it, so
    // sweeping a rig never flickers a trail of labels. Held in egui's own
    // temporary store rather than in Session, because it is pure view state that
    // nothing else reads and no undo should ever see.
    let id = egui::Id::new("hover_label_timer");
    let now = ui.ctx().input(|i| i.time) as f32;
    let (since, key) = ui
        .ctx()
        .data(|d| d.get_temp::<(f32, String)>(id))
        .unwrap_or((now, String::new()));
    let rested = if key == target.key {
        now - since
    } else {
        ui.ctx()
            .data_mut(|d| d.insert_temp(id, (now, target.key.clone())));
        0.0
    };
    if rested < DELAY && !summoned(ui) {
        // Keep the frame coming so the label appears when the delay elapses,
        // rather than waiting for the next unrelated repaint.
        ui.ctx().request_repaint();
        return;
    }

    paint(ui, rect, cursor, &target);
}

/// The modifier that shows a label now, without waiting out the delay.
///
/// Alt, because it is the one modifier no canvas tool has claimed — Ctrl and
/// Shift already mean things to selection and constrained dragging.
fn summoned(ui: &egui::Ui) -> bool {
    ui.ctx().input(|i| i.modifiers.alt)
}

/// Is some drag in flight?
///
/// Checked field by field rather than by a single flag because the session has
/// no single flag: each tool tracks its own drag, and any of them means the
/// cursor is committed to something.
fn is_dragging(state: &AppState) -> bool {
    state.session.dragging_vertex.is_some()
        || state.session.drag_start_world_pos.is_some()
        || state.session.preview_bone.is_some()
        || state.session.dragging_gizmo != crate::session::GizmoInteraction::None
}

/// Work out what the cursor is over, in priority order.
///
/// Vertex first, then attachment, then bone. That is smallest-to-largest, which
/// is the order that matches intent: a vertex sits on top of the art it belongs
/// to, which sits on top of the bone that moves it, and hovering the small thing
/// is always the more specific request.
fn resolve(state: &AppState) -> Option<Target> {
    vertex_target(state)
        .or_else(|| attachment_target(state))
        .or_else(|| bone_target(state))
}

/// The bone's ancestry as crumbs, root first, `bone` itself last.
///
/// Capped, because the trail is a label and not a hierarchy panel: a rig can be
/// twenty bones deep, and a crumb per level would be wider than the viewport.
/// The near ancestors are the ones that place a bone; the far ones are always
/// `root`, and dropping them for an ellipsis loses nothing.
fn bone_trail(state: &AppState, bone: BoneId) -> Vec<Crumb> {
    let mut chain = Vec::new();
    let mut current = Some(bone);
    // Bounded like every other walk of this hierarchy: a cycle cannot be built
    // through the editor, but a bounded loop costs nothing and cannot hang.
    for _ in 0..64 {
        let Some(id) = current else { break };
        let Some(b) = state.doc.skeleton.bones.get(id) else {
            break;
        };
        chain.push(Crumb::new(crate::ui::icons::BONE, b.name.clone()));
        current = b.parent;
    }
    chain.reverse();
    if chain.len() > MAX_TRAIL {
        // Keep the tail: the bones nearest the thing being hovered are the ones
        // that say where it is.
        chain = chain.split_off(chain.len() - MAX_TRAIL);
        chain.insert(0, Crumb::new("", "…".to_string()));
    }
    chain
}

/// A mesh vertex, with the bones that hold it.
fn vertex_target(state: &AppState) -> Option<Target> {
    let index = state.session.hovered_vertex?;
    let slot_id = state.session.active_slot()?;
    let slot = state.doc.skeleton.slots.get(slot_id)?;
    let Attachment::Mesh(mesh) = state
        .doc
        .skeleton
        .resolve_slot_many(&state.session.skin_stack(), slot_id)?
    else {
        return None;
    };

    // Ranked exactly as the weight overlay ranks it, so a row here and a wedge
    // out there are the same colour for the same bone.
    let ranks: std::collections::HashMap<BoneId, usize> = mesh
        .bound_bones()
        .iter()
        .enumerate()
        .map(|(rank, (bone, _))| (*bone, rank))
        .collect();

    let mut details = Vec::new();
    match mesh.weights.get(index).filter(|w| !w.is_empty()) {
        None => details.push(Detail::plain("Unweighted — follows its slot's bone")),
        Some(weights) => {
            let mut sorted: Vec<_> = weights.iter().collect();
            sorted.sort_by(|a, b| b.weight.total_cmp(&a.weight));
            for w in sorted.iter().take(MAX_INFLUENCES) {
                let name = state
                    .doc
                    .skeleton
                    .bones
                    .get(w.bone)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| "<missing bone>".into());
                let swatch =
                    super::renderer::color_for_rank(ranks.get(&w.bone).copied()).map(|rgba| {
                        egui::Color32::from_rgb(
                            (rgba[0] * 255.0) as u8,
                            (rgba[1] * 255.0) as u8,
                            (rgba[2] * 255.0) as u8,
                        )
                    });
                details.push(Detail {
                    text: format!("{name}  {:.0}%", w.weight * 100.0),
                    swatch,
                });
            }
            if sorted.len() > MAX_INFLUENCES {
                details.push(Detail::plain(format!(
                    "+{} more",
                    sorted.len() - MAX_INFLUENCES
                )));
            }
        }
    }

    // Bone › slot › vertex. The bone the *slot* hangs on, not the bones weighting
    // the vertex — those are the influence rows below, and they are a different
    // relationship: one is where the geometry lives, the other is what pulls it.
    let mut trail = bone_trail(state, slot.bone);
    trail.push(Crumb::new(crate::ui::icons::SLOT, slot.name.clone()));
    trail.push(Crumb::new(
        crate::ui::icons::MESH,
        format!("vertex {index}"),
    ));

    Some(Target {
        trail,
        details,
        key: format!("vertex:{slot_id:?}:{index}"),
    })
}

/// The attachment under the cursor, named by its kind.
fn attachment_target(state: &AppState) -> Option<Target> {
    let (slot_id, name) = state.session.hovered_attachment.clone()?;
    let slot = state.doc.skeleton.slots.get(slot_id)?;
    let attachment = state
        .doc
        .skeleton
        .resolve_many(&state.session.skin_stack(), slot_id, &name);

    let icon = match attachment {
        Some(Attachment::Mesh(_)) => crate::ui::icons::MESH,
        Some(Attachment::Clipping(_)) => crate::ui::icons::CLIP,
        Some(Attachment::Path(_)) => crate::ui::icons::PATH,
        Some(Attachment::BoundingBox(_)) => crate::ui::icons::MESH,
        Some(Attachment::Point(_)) => crate::ui::icons::POINT,
        Some(Attachment::Region(_)) | None => crate::ui::icons::IMAGE,
    };

    // Bone › slot › attachment. The slot crumb is not redundant with the
    // attachment's own name even when the two read alike — they are separate
    // things that share a name often enough that collapsing them would hide the
    // exact confusion this label exists to clear up.
    let mut trail = bone_trail(state, slot.bone);
    trail.push(Crumb::new(crate::ui::icons::SLOT, slot.name.clone()));
    trail.push(Crumb::new(icon, name.clone()));

    Some(Target {
        trail,
        details: Vec::new(),
        key: format!("attachment:{slot_id:?}:{name}"),
    })
}

/// The bone under the cursor, with anything driving it.
fn bone_target(state: &AppState) -> Option<Target> {
    let bone_id = state.session.hovered_bone?;
    // Existence check: a stale hover id would otherwise build a trail that ends
    // in a bone the document no longer has.
    state.doc.skeleton.bones.get(bone_id)?;

    let mut details = Vec::new();

    // Constraints that write to this bone. The reason this is worth the space:
    // a constrained bone ignores the pose you drag it into, and without a name
    // for the thing overriding you, that reads as the editor being broken.
    for constraint in state.doc.skeleton.constraints.values() {
        if !constraint.affected_bones().contains(&bone_id) {
            continue;
        }
        let (icon, kind) = match constraint {
            ankhimate_core::constraints::Constraint::Ik(_) => (crate::ui::icons::IK, "IK"),
            ankhimate_core::constraints::Constraint::Transform(_) => {
                (crate::ui::icons::CONSTRAINT, "Transform")
            }
            ankhimate_core::constraints::Constraint::Physics(_) => {
                (crate::ui::icons::CONSTRAINT, "Physics")
            }
            ankhimate_core::constraints::Constraint::Path(_) => (crate::ui::icons::PATH, "Path"),
        };
        // Inert constraints are named too, and marked: "there is an IK here but
        // it is doing nothing" is a different and equally useful answer to "this
        // bone will not move".
        let inert = if constraint.is_inert() {
            "  (inert)"
        } else {
            ""
        };
        // Not a crumb: a constraint is not an ancestor. It sits beside the bone
        // in the rig rather than above it, and putting it in the trail would
        // claim a containment that does not exist.
        details.push(Detail::plain(format!(
            "{icon}  {kind}: {}{inert}",
            constraint.name()
        )));
    }

    Some(Target {
        trail: bone_trail(state, bone_id),
        details,
        key: format!("bone:{bone_id:?}"),
    })
}

/// Lay the label out and paint it.
fn paint(ui: &egui::Ui, rect: egui::Rect, cursor: egui::Pos2, target: &Target) {
    let painter = ui.painter_at(rect);

    // Measured before placement, because where the label goes depends on how big
    // it is: a box that would overflow the right edge has to flip to the other
    // side of the cursor, and that cannot be decided until the width is known.
    let font = egui::FontId::proportional(TITLE_SIZE);
    let detail_font = egui::FontId::proportional(DETAIL_SIZE);

    // The trail laid out as a flat run of galleys — icon, name, separator, icon,
    // name, … — rather than one string per crumb. Each piece is coloured
    // separately (icons dimmer than names, the last crumb brighter than its
    // ancestors), and a galley is the smallest thing egui will colour on its own.
    let last = target.trail.len().saturating_sub(1);
    let mut trail_parts: Vec<(std::sync::Arc<egui::Galley>, egui::Color32)> = Vec::new();
    for (i, crumb) in target.trail.iter().enumerate() {
        if i > 0 {
            trail_parts.push((
                painter.layout_no_wrap(
                    SEPARATOR.to_string(),
                    detail_font.clone(),
                    egui::Color32::PLACEHOLDER,
                ),
                egui::Color32::from_gray(105),
            ));
        }
        // The thing under the cursor is the last crumb, and it is what the user
        // asked about; the ones before it are context. Dimming the context is
        // what lets the answer be found without reading the whole line.
        let (icon_color, text_color) = if i == last {
            (egui::Color32::from_gray(190), egui::Color32::WHITE)
        } else {
            (egui::Color32::from_gray(120), egui::Color32::from_gray(165))
        };
        if !crumb.icon.is_empty() {
            trail_parts.push((
                painter.layout_no_wrap(
                    crumb.icon.to_string(),
                    detail_font.clone(),
                    egui::Color32::PLACEHOLDER,
                ),
                icon_color,
            ));
        }
        trail_parts.push((
            painter.layout_no_wrap(crumb.text.clone(), font.clone(), egui::Color32::PLACEHOLDER),
            text_color,
        ));
    }

    let detail_galleys: Vec<_> = target
        .details
        .iter()
        .map(|d| {
            (
                painter.layout_no_wrap(
                    d.text.clone(),
                    detail_font.clone(),
                    egui::Color32::PLACEHOLDER,
                ),
                d.swatch,
            )
        })
        .collect();

    let trail_width = trail_parts.iter().map(|(g, _)| g.size().x).sum::<f32>()
        + TRAIL_GAP * trail_parts.len() as f32;
    let trail_height = trail_parts
        .iter()
        .map(|(g, _)| g.size().y)
        .fold(0.0f32, f32::max);
    let widest_detail = detail_galleys
        .iter()
        .map(|(g, swatch)| g.size().x + if swatch.is_some() { SWATCH + 5.0 } else { 0.0 })
        .fold(0.0f32, f32::max);
    let width = trail_width.max(widest_detail) + PADDING.x * 2.0;
    let height = trail_height
        + detail_galleys
            .iter()
            .map(|(g, _)| g.size().y + LINE_GAP)
            .sum::<f32>()
        + PADDING.y * 2.0;

    // Below-right of the cursor by default, flipped on either axis when that
    // would put the box outside the viewport. Flipping rather than clamping: a
    // clamped label slides under the pointer, which is the thing the offset
    // exists to prevent.
    let mut origin = cursor + CURSOR_OFFSET;
    if origin.x + width > rect.max.x {
        origin.x = cursor.x - CURSOR_OFFSET.x - width;
    }
    if origin.y + height > rect.max.y {
        origin.y = cursor.y - CURSOR_OFFSET.y - height;
    }
    // If it does not fit on either side — a viewport narrower than the label —
    // clamping is all that is left, and a covered cursor beats an off-screen
    // label.
    origin.x = origin.x.max(rect.min.x + 2.0);
    origin.y = origin.y.max(rect.min.y + 2.0);

    let box_rect = egui::Rect::from_min_size(origin, egui::vec2(width, height));

    // Its own background, not just a text colour: this sits on whatever artwork
    // the user imported, and text alone is unreadable over half of it.
    painter.rect_filled(
        box_rect.expand(1.0),
        4.0,
        egui::Color32::from_black_alpha(210),
    );
    painter.rect_stroke(
        box_rect.expand(1.0),
        4.0,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)),
        egui::StrokeKind::Middle,
    );

    let mut y = origin.y + PADDING.y;
    let mut x = origin.x + PADDING.x;
    for (galley, color) in trail_parts {
        // Vertically centred on the row: the icon font and the text font have
        // different heights, and sitting them both on a shared baseline leaves
        // the icons visibly high.
        let dy = (trail_height - galley.size().y) * 0.5;
        let advance = galley.size().x;
        painter.galley(egui::pos2(x, y + dy), galley, color);
        x += advance + TRAIL_GAP;
    }
    y += trail_height + LINE_GAP;

    for (galley, swatch) in detail_galleys {
        let mut x = origin.x + PADDING.x;
        if let Some(color) = swatch {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(x + SWATCH * 0.5, y + galley.size().y * 0.5),
                    egui::vec2(SWATCH, SWATCH),
                ),
                1.0,
                color,
            );
            x += SWATCH + 5.0;
        }
        let height = galley.size().y;
        painter.galley(egui::pos2(x, y), galley, egui::Color32::from_gray(190));
        y += height + LINE_GAP;
    }
}
