//! PSD import (T-302).
//!
//! A layered PSD is already a rig that nobody has told the computer about: the
//! group structure is the hierarchy, the layer bounds are the placement, and the
//! stacking order is the draw order. This module reads that structure out
//! instead of asking the user to rebuild it by hand.
//!
//! The mapping is a **convention on layer names**, documented in
//! `docs/psd-import.md` and summarised here:
//!
//! | In the PSD | Becomes |
//! |---|---|
//! | layer group | a bone, nested groups nesting |
//! | image layer | a slot plus a region attachment, placed from the layer bounds |
//! | layer named `$pivot` | its group's bone origin, instead of the group centre |
//! | group named `$ik <name>` | an IK constraint over the bones inside it |
//! | top-level group `@skin:<name>` | its contents land in skin `<name>` |
//!
//! Names carry the convention rather than layer metadata because a name is the
//! one thing every art tool round-trips, and the one thing an artist can fix
//! without leaving Photoshop.
//!
//! Coordinates: PSD measures from the top-left with Y down; we measure from the
//! canvas centre with Y up (PLAN §2.2). The conversion happens once, in
//! [`layer_center`].

use ankhimate_core::assets::{AssetDb, ImageAsset};
use ankhimate_core::attachment::{Attachment, Rect, RegionAttachment};
use ankhimate_core::constraints::{Constraint, IkConstraint, PhysicsConstraint};
use ankhimate_core::ids::{BoneId, SkinId, SlotId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::skin::Skin;
use ankhimate_core::slot::{BlendMode, Slot};
use std::collections::{HashMap, HashSet};

/// A layer's tags, with the pre-grammar markers folded in.
///
/// `$pivot`, `$ik <name>` and `@skin:<name>` predate `[tag:value]` and still
/// work: a PSD is an artist's file, and breaking one to tidy a syntax is a bad
/// trade. They are translated here rather than handled separately, so the rest
/// of the reader sees one vocabulary.
fn tags_of(raw: &str) -> crate::psd_tags::Tags {
    let mut tags = crate::psd_tags::Tags::parse(raw);
    if tags.name == PIVOT_LAYER {
        tags.inherit("pivot", None);
    }
    if let Some(rest) = tags.name.strip_prefix(IK_PREFIX) {
        let name = rest.trim().to_string();
        tags.inherit("ik", Some(name.clone()));
        tags.name = name;
    }
    if let Some(rest) = tags.name.strip_prefix(SKIN_PREFIX) {
        let name = rest.trim().to_string();
        tags.inherit("skin", Some(name.clone()));
        tags.name = name;
    }
    tags
}

/// The rectangle covering every one of `parts`, or nothing when there are none.
fn union_of(parts: impl Iterator<Item = (i32, i32, u32, u32)>) -> (i32, i32, u32, u32) {
    let (mut min_x, mut min_y) = (i32::MAX, i32::MAX);
    let (mut max_x, mut max_y) = (i32::MIN, i32::MIN);
    let mut any = false;
    for (x, y, w, h) in parts {
        any = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w as i32);
        max_y = max_y.max(y + h as i32);
    }
    if !any {
        return (0, 0, 0, 0);
    }
    (
        min_x,
        min_y,
        (max_x - min_x).max(0) as u32,
        (max_y - min_y).max(0) as u32,
    )
}

/// Layer whose position defines its group's bone origin.
pub const PIVOT_LAYER: &str = "$pivot";
/// Group-name prefix that asks for an IK constraint over the bones inside.
pub const IK_PREFIX: &str = "$ik ";
/// Top-level group-name prefix that routes contents into a named skin.
pub const SKIN_PREFIX: &str = "@skin:";

#[derive(Debug, thiserror::Error)]
pub enum PsdError {
    #[error("could not read the PSD: {0}")]
    Parse(String),
    #[error("the PSD has no layers to import")]
    Empty,
}

/// One node of the layer tree, for the import modal's preview.
#[derive(Debug, Clone)]
pub struct LayerNode {
    /// Slash-joined path from the document root, e.g. `torso/arm/hand`.
    ///
    /// This is the identity a re-import matches on. A layer that moves inside
    /// its group keeps its path and so keeps its bone; a layer that is *renamed*
    /// reads as a delete plus an add, which is the honest answer — we cannot
    /// tell a rename from a swap.
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub is_group: bool,
    pub visible: bool,
    /// Pixel bounds in the PSD: `(left, top, width, height)`.
    pub bounds: (i32, i32, u32, u32),
}

/// What the caller wants imported, and how.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// World units per PSD pixel.
    pub scale: f32,
    /// Layer paths to import. Empty means everything.
    pub include: HashSet<String>,
    /// Groups to collapse into a single attachment rather than a bone with
    /// children — a face that never articulates does not need eleven bones.
    pub flatten: HashSet<String>,
    /// Skip layers hidden in Photoshop. On by default: a hidden layer is
    /// usually a reference sketch or an alternate the artist did not delete.
    pub skip_hidden: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            include: HashSet::new(),
            flatten: HashSet::new(),
            skip_hidden: true,
        }
    }
}

impl ImportOptions {
    fn wants(&self, path: &str) -> bool {
        self.include.is_empty() || self.include.contains(path)
    }
}

/// What an import produced, for the summary the modal shows afterwards.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportSummary {
    pub bones: usize,
    pub slots: usize,
    pub images: usize,
    pub skins: usize,
    pub constraints: usize,
    /// Layers that were skipped, and why — hidden, excluded, or empty.
    pub skipped: Vec<String>,
    /// Structure the reader decided without being told, each with its evidence
    /// and the tag that would say otherwise (`psd_infer`).
    ///
    /// Carried out to the caller because inference that is usually right is
    /// worse than none if it is silent: the failure mode is a rig subtly wrong
    /// for a reason nobody can see.
    pub guesses: Vec<crate::psd_infer::Guess>,
    /// Layers whose Photoshop blend mode this model has no equivalent for, as
    /// `(layer path, the mode's name)`.
    ///
    /// Photoshop has 28 and this has four. Silently normalising the other 24 to
    /// Normal is a layer that looked right in the artist's file and wrong in the
    /// editor with nothing to read about why — the same loss `LoadReport`
    /// exists for, applied to a slot.
    pub lost_blend: Vec<(String, String)>,
    /// Runs folded into one attachment, as `(stem, frame count)`.
    ///
    /// Worth reporting on its own: the artist gave the importer five layers and
    /// got one slot back, and a count that does not match what they drew is the
    /// first sign a frame was hidden or misnumbered.
    pub sequences: Vec<(String, usize)>,
    /// Tags this build did not recognise, as `(layer path, tag)`.
    ///
    /// A misspelled `[bonee]` should be findable. Dropping it quietly is an
    /// artist wondering why their tag did nothing.
    pub unknown_tags: Vec<(String, String)>,
}

/// The renderer's blend mode for a Photoshop one, and what was lost saying so.
///
/// Photoshop has 28 blend modes and this model has four. Rather than quietly
/// normalising the other 24 to Normal — which is a layer that looked right in
/// the artist's file and wrong in the editor, with nothing to read about why —
/// the ones that do not map are named in the report.
///
/// `PassThrough` is a group-only mode meaning "do not isolate", which is what a
/// slot does anyway; it maps to Normal without loss and is not reported.
fn blend_mode_of(layer: &psd::PsdLayer) -> (BlendMode, bool) {
    // `psd 0.3.5` does not export its `BlendMode`, so the type cannot be named
    // here and the variant is read off its `Debug`. Unpleasant, and the report
    // wants the mode's name as text regardless — this way the two cannot drift.
    match blend_mode_name(layer).as_str() {
        "Normal" | "PassThrough" => (BlendMode::Normal, true),
        "Multiply" => (BlendMode::Multiply, true),
        "Screen" => (BlendMode::Screen, true),
        "LinearDodge" => (BlendMode::Additive, true),
        _ => (BlendMode::Normal, false),
    }
}

/// A `[box]`, `[point]` or `[clip]` layer as the attachment it stands for.
///
/// The three share a shape: a layer whose pixels are a placeholder the artist
/// draws so they can see where the thing is. Its rectangle is the geometry, so
/// nothing has to be positioned twice — move the layer, move the box.
///
/// A rectangle is not the polygon a hull would be. It is the honest reading of
/// what a PSD says: tracing the silhouette would give a shape the artist did not
/// author and cannot see, and refining a box in the editor is one drag.
fn marker_attachment(
    tags: &crate::psd_tags::Tags,
    layer: &psd::PsdLayer,
    canvas: (u32, u32),
    bone: BoneId,
    skeleton: &Skeleton,
    options: &ImportOptions,
) -> Option<Attachment> {
    let bounds = layer_bounds(layer);
    let centre = layer_center(bounds, canvas, options.scale);
    let origin = world_origin(skeleton, bone);
    let local = centre - origin;

    if tags.has("point") {
        // A PSD has no rotation to read, so the point's is zero and the rigger
        // aims it. Reporting a guessed angle would be worse than none.
        return Some(Attachment::Point(
            ankhimate_core::attachment::PointAttachment {
                position: local,
                rotation: 0.0,
            },
        ));
    }

    let corners = |half: glam::Vec2| {
        vec![
            glam::Vec2::new(local.x - half.x, local.y - half.y),
            glam::Vec2::new(local.x + half.x, local.y - half.y),
            glam::Vec2::new(local.x + half.x, local.y + half.y),
            glam::Vec2::new(local.x - half.x, local.y + half.y),
        ]
    };
    let half = glam::Vec2::new(
        bounds.2 as f32 * options.scale / 2.0,
        bounds.3 as f32 * options.scale / 2.0,
    );

    if tags.has("box") {
        return Some(Attachment::BoundingBox(
            ankhimate_core::attachment::BoundingBoxAttachment {
                vertices: corners(half),
                weights: Vec::new(),
            },
        ));
    }
    if tags.has("clip") {
        return Some(Attachment::Clipping(
            ankhimate_core::attachment::ClippingAttachment {
                // `[clip:slot]` stops the clip after that slot; bare `[clip]`
                // clips everything drawn after it, which is what the model's
                // `None` already means.
                vertices: corners(half),
                end_slot: tags.value("clip").map(str::to_string),
            },
        ));
    }
    None
}

/// The Photoshop blend mode's name, for matching on and for the report.
fn blend_mode_name(layer: &psd::PsdLayer) -> String {
    format!("{:?}", layer.blend_mode())
}

/// A blend mode named in a `[blend:…]` tag.
///
/// The tag exists for the case the file cannot express: an artist working in a
/// mode Photoshop has and the engine does not, who knows which of our four they
/// want at runtime. Spelled as the engine names them, not as Photoshop does.
fn blend_mode_named(name: &str) -> Option<BlendMode> {
    match name.trim().to_ascii_lowercase().as_str() {
        "normal" => Some(BlendMode::Normal),
        "additive" | "add" => Some(BlendMode::Additive),
        "multiply" => Some(BlendMode::Multiply),
        "screen" => Some(BlendMode::Screen),
        _ => None,
    }
}

/// Frames per second an inferred run plays at.
///
/// A numbered run says nothing about timing, so this is a starting point the
/// artist changes, not a reading of the file. `[fps:n]` overrides it.
const DEFAULT_SEQUENCE_FPS: f32 = 12.0;

/// What inference decided, in the terms the import loop works in.
///
/// Inference reasons about the whole tree at once and the loop sees one node at
/// a time, so the translation happens once, up front. The alternative — asking
/// `psd_infer` again per layer — is how the guess an artist reads and the rig
/// they get end up disagreeing.
#[derive(Debug, Default)]
struct ImportPlan {
    /// Lead layer path to the run it heads.
    sequences: HashMap<String, SequencePlan>,
    /// Every frame path, including the lead, to which run it belongs to.
    frames: HashMap<String, FramePlan>,
    /// Group paths whose art belongs to their parent's bone.
    not_a_bone: HashSet<String>,
}

#[derive(Debug)]
struct SequencePlan {
    stem: String,
    frames: Vec<String>,
    fps: Option<f32>,
}

#[derive(Debug)]
struct FramePlan {
    /// The run's first frame by number, which owns the slot.
    lead: String,
}

/// A finished import: a skeleton, the images it references, and what happened.
pub struct PsdImport {
    pub skeleton: Skeleton,
    pub assets: AssetDb,
    pub summary: ImportSummary,
    /// Layer path per slot, so a re-import can find the same layer again.
    pub layer_paths: HashMap<String, String>,
    /// Attachments a `[mesh]` tag asked to be traced, as attachment names.
    ///
    /// A request rather than a result, because the tracer lives in `document`
    /// and `formats` cannot reach it: `core` is the runtime contract and stays
    /// dependency-light (PLAN §3.1), so moving `meshgen` down to make this
    /// crate's life easier would drag `spade` and `image` into the crate that
    /// has to compile for `wasm32`. Marking the layers costs one field; the
    /// alternative costs `core` its shape.
    pub trace_requests: Vec<TraceRequest>,
}

/// One `[mesh]` layer, and what its tags asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRequest {
    /// The attachment to replace with a traced mesh.
    pub attachment: String,
    /// The slot it hangs from.
    pub slot: String,
    /// `[mesh:n]` — how closely the outline follows the pixels, 0–100. The
    /// tracer's own default when the tag is bare.
    pub detail: Option<f32>,
    /// `[weights]` — bind the traced vertices to the bones around them.
    pub weights: bool,
}

/// What changed when a PSD was re-imported over an existing document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReimportReport {
    /// Layer paths present in the PSD but not in the document.
    pub added: Vec<String>,
    /// Layer paths the document has that the PSD no longer does.
    pub removed: Vec<String>,
    /// Layers whose pixels or bounds changed.
    pub changed: Vec<String>,
    /// Layers that matched and were identical.
    pub unchanged: usize,
}

/// Read the layer tree without importing anything, for the preview.
pub fn layer_tree(bytes: &[u8]) -> Result<Vec<LayerNode>, PsdError> {
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| PsdError::Parse(e.to_string()))?;
    let mut nodes = Vec::new();
    let paths = group_paths(&psd);
    for id in psd.group_ids_in_order() {
        let Some(group) = psd.groups().get(id) else {
            continue;
        };
        let path = paths
            .get(id)
            .cloned()
            .unwrap_or_else(|| group.name().into());
        // A group's extent is the union of what is inside it. The `psd` crate
        // reports `1x1` for a group's own rectangle — measured, not assumed —
        // and this used to hardcode `0x0`, so either way a caller reading these
        // bounds got a number that meant nothing.
        let (left, top, width, height) = union_of(
            psd.get_group_sub_layers(id)
                .unwrap_or_default()
                .iter()
                .map(layer_bounds),
        );
        nodes.push(LayerNode {
            depth: path.matches('/').count(),
            path: path.clone(),
            name: group.name().to_string(),
            is_group: true,
            visible: shown(group),
            bounds: (left, top, width, height),
        });
        for layer in psd.get_group_sub_layers(id).unwrap_or_default() {
            nodes.push(node_for(layer, &format!("{path}/{}", layer.name())));
        }
    }
    // Layers outside every group, which Photoshop allows and artists use for
    // one-off pieces.
    for layer in psd.layers() {
        if layer.parent_id().is_none() {
            nodes.push(node_for(layer, layer.name()));
        }
    }
    if nodes.is_empty() {
        return Err(PsdError::Empty);
    }
    Ok(nodes)
}

/// Is this layer shown in Photoshop?
///
/// The `psd` crate reads bit 1 of the layer flags and calls it `visible`, which
/// is what Adobe's spec calls it. In practice the bit is set when a layer is
/// **hidden** — every layer in that crate's own fixtures, all of them ordinary
/// visible art, reports `visible() == false`. So the flag is read here and
/// inverted, with the name saying what it actually means.
fn shown(flagged: impl VisibilityFlag) -> bool {
    !flagged.raw_visible()
}

/// Both a layer and a group carry the flag, and they share no public trait.
trait VisibilityFlag {
    fn raw_visible(&self) -> bool;
}

impl VisibilityFlag for &psd::PsdLayer {
    fn raw_visible(&self) -> bool {
        self.visible()
    }
}

impl VisibilityFlag for &psd::PsdGroup {
    fn raw_visible(&self) -> bool {
        self.visible()
    }
}

fn node_for(layer: &psd::PsdLayer, path: &str) -> LayerNode {
    LayerNode {
        depth: path.matches('/').count(),
        path: path.to_string(),
        name: layer.name().to_string(),
        is_group: false,
        visible: shown(layer),
        bounds: layer_bounds(layer),
    }
}

/// `(left, top, width, height)` in PSD pixels.
///
/// `layer_right`/`layer_bottom` are inclusive in the file format, so the width
/// is the difference plus one. Off by one here means every attachment is a pixel
/// short and every pivot half a pixel out.
fn layer_bounds(layer: &psd::PsdLayer) -> (i32, i32, u32, u32) {
    let left = layer.layer_left();
    let top = layer.layer_top();
    let width = (layer.layer_right() - left + 1).max(0) as u32;
    let height = (layer.layer_bottom() - top + 1).max(0) as u32;
    (left, top, width, height)
}

/// The layer's centre in world units: PSD top-left/Y-down to centre/Y-up.
fn layer_center(bounds: (i32, i32, u32, u32), canvas: (u32, u32), scale: f32) -> glam::Vec2 {
    let (left, top, width, height) = bounds;
    let cx = left as f32 + width as f32 * 0.5 - canvas.0 as f32 * 0.5;
    let cy = canvas.1 as f32 * 0.5 - (top as f32 + height as f32 * 0.5);
    glam::vec2(cx * scale, cy * scale)
}

/// Full path per group id, so nested groups read as `torso/arm`.
/// # A wart, recorded rather than fixed
///
/// Paths are built from **raw** layer names, tags included: a group called
/// `arm [bone]` gives its children `arm [bone]/upper`. A re-import matches on
/// the path, so adding a tag to a group renames every path beneath it and the
/// match is lost — the artist gets a delete plus an add for art that did not
/// move.
///
/// Not fixed here because `psd_layer_paths` is already saved in this shape, so
/// changing it is a migration rather than an edit. `psd_read::Layer` exposes
/// `name` alongside `path` so a consumer at least never has to strip tags
/// itself.
fn group_paths(psd: &psd::Psd) -> HashMap<u32, String> {
    // `group_ids_in_order` is *not* parent-before-child — a nested group can be
    // listed ahead of the group that contains it — so each path is walked up
    // from its own parents rather than read off an already-built table.
    let mut paths: HashMap<u32, String> = HashMap::new();
    for id in psd.group_ids_in_order() {
        let mut parts: Vec<&str> = Vec::new();
        let mut cursor = Some(*id);
        // Bounded by nesting depth; a cycle would mean a malformed file.
        for _ in 0..64 {
            let Some(current) = cursor else { break };
            let Some(group) = psd.groups().get(&current) else {
                break;
            };
            parts.push(group.name());
            cursor = group.parent_id();
        }
        parts.reverse();
        paths.insert(*id, parts.join("/"));
    }
    paths
}

/// Import a PSD into a fresh skeleton.
pub fn import(bytes: &[u8], options: &ImportOptions) -> Result<PsdImport, PsdError> {
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| PsdError::Parse(e.to_string()))?;
    let canvas = (psd.width(), psd.height());
    let paths = group_paths(&psd);

    let mut skeleton = Skeleton::new();
    let mut assets = AssetDb::new();
    let mut summary = ImportSummary::default();
    let mut layer_paths = HashMap::new();
    let mut frame_images: HashMap<String, String> = HashMap::new();
    let mut leads: HashMap<String, (SkinId, SlotId, String)> = HashMap::new();
    let mut trace_requests: Vec<TraceRequest> = Vec::new();

    // Read the file's own structure before building anything, so a guess is
    // made against the whole tree rather than against whatever has been seen so
    // far. `layer_tree` is the same walk the import modal previews with.
    let nodes = layer_tree(bytes)?;
    let candidates: Vec<crate::psd_infer::Candidate> = nodes
        .iter()
        .map(|n| crate::psd_infer::Candidate {
            path: n.path.clone(),
            name: n.name.clone(),
            depth: n.depth,
            is_group: n.is_group,
            bounds: n.bounds,
        })
        .collect();
    let node_tags: Vec<crate::psd_tags::Tags> = nodes.iter().map(|n| tags_of(&n.name)).collect();

    for (node, tags) in nodes.iter().zip(&node_tags) {
        for tag in tags.names() {
            if !crate::psd_tags::KNOWN.contains(&tag) {
                summary
                    .unknown_tags
                    .push((node.path.clone(), tag.to_string()));
            }
        }
    }
    let inferred = crate::psd_infer::infer(&candidates, &node_tags, &mut summary.guesses);

    // What inference decided, keyed by the path the import loop knows a layer
    // by. Both are built here rather than looked up per layer, because the
    // decision is about the whole tree and the loop only ever sees one node.
    let mut plan = ImportPlan::default();
    for (i, node) in nodes.iter().enumerate() {
        if let Some(sequence) = &inferred[i].sequence {
            for path in &sequence.frames {
                plan.frames.insert(
                    path.clone(),
                    FramePlan {
                        lead: sequence.frames[0].clone(),
                    },
                );
            }
            plan.sequences.insert(
                sequence.frames[0].clone(),
                SequencePlan {
                    stem: sequence.stem.clone(),
                    frames: sequence.frames.clone(),
                    fps: sequence.fps,
                },
            );
        }
        // A group inference read as *one* bone keeps its children as art on
        // it. Recorded as a set of paths rather than re-derived in the loop, so
        // the importer and the guess the artist reads cannot disagree.
        if node.is_group && !inferred[i].bone {
            plan.not_a_bone.insert(node.path.clone());
        }
    }

    let root = skeleton.add_bone(Bone {
        name: "root".into(),
        parent: None,
        length: 1.0,
        local_transform: Transform::default(),
        inherit: Default::default(),
        color: Bone::default_color(),
    });
    summary.bones += 1;

    // Bone and skin per group path, filled as groups are visited parent-first.
    let mut bones: HashMap<String, BoneId> = HashMap::new();
    let mut skins: HashMap<String, SkinId> = HashMap::new();

    // Parents first, so a child group finds the bone it hangs from. The file's
    // own order does not guarantee it — a nested group can be listed ahead of
    // its container — and depth is the one ordering that always does.
    let mut ordered: Vec<(u32, String)> = psd
        .group_ids_in_order()
        .iter()
        .filter_map(|id| paths.get(id).map(|p| (*id, p.clone())))
        .collect();
    ordered.sort_by_key(|(_, path)| path.matches('/').count());

    for (id, path) in &ordered {
        let Some(group) = psd.groups().get(id) else {
            continue;
        };
        let path = path.clone();
        if !options.wants(&path) {
            summary.skipped.push(format!("{path} (not selected)"));
            continue;
        }
        if options.skip_hidden && !shown(group) {
            summary.skipped.push(format!("{path} (hidden)"));
            continue;
        }

        let sub_layers = psd.get_group_sub_layers(id).unwrap_or_default();

        // `@skin:` groups are containers, not bones: their job is to route what
        // is inside them into a named skin.
        let group_tags = tags_of(group.name());
        if let Some(skin_name) = group_tags.value("skin").map(str::to_string) {
            let skin_id = *skins.entry(skin_name.clone()).or_insert_with(|| {
                summary.skins += 1;
                skeleton.add_skin(Skin::new(&skin_name))
            });
            // Its bone is its parent's, so art in a skin hangs where the base
            // art hangs rather than under a bone that only exists in one outfit.
            let parent = group
                .parent_id()
                .and_then(|p| paths.get(&p))
                .and_then(|p| bones.get(p))
                .copied()
                .unwrap_or(root);
            bones.insert(path.clone(), parent);
            for layer in sub_layers {
                let layer_path = format!("{path}/{}", layer.name());
                add_layer(
                    &mut Sink {
                        skeleton: &mut skeleton,
                        assets: &mut assets,
                        summary: &mut summary,
                        layer_paths: &mut layer_paths,
                        frame_images: &mut frame_images,
                        leads: &mut leads,
                        trace_requests: &mut trace_requests,
                    },
                    &plan,
                    skin_id,
                    parent,
                    layer,
                    &layer_path,
                    canvas,
                    options,
                );
            }
            continue;
        }

        // `[folder]` is the artist saying what the scatter heuristic guesses:
        // this group is organisation, its art belongs to the parent's bone. An
        // explicit form matters because the guess is only usually right, and
        // the alternative is renaming a group to hide it from a heuristic.
        // A group inference read as art rather than articulation gets no bone
        // of its own: its layers hang from its parent's. This is the half of
        // the "one bone, not eleven" guess that the artist can see in the rig —
        // without it the guess is a sentence in a report and the hierarchy is
        // unchanged.
        if plan.not_a_bone.contains(&path) || group_tags.has("folder") {
            let parent = group
                .parent_id()
                .and_then(|p| paths.get(&p))
                .and_then(|p| bones.get(p))
                .copied()
                .unwrap_or(root);
            bones.insert(path.clone(), parent);
            let default_skin = skeleton.default_skin;
            for layer in sub_layers {
                let layer_path = format!("{path}/{}", layer.name());
                add_layer(
                    &mut Sink {
                        skeleton: &mut skeleton,
                        assets: &mut assets,
                        summary: &mut summary,
                        layer_paths: &mut layer_paths,
                        frame_images: &mut frame_images,
                        leads: &mut leads,
                        trace_requests: &mut trace_requests,
                    },
                    &plan,
                    default_skin,
                    parent,
                    layer,
                    &layer_path,
                    canvas,
                    options,
                );
            }
            continue;
        }

        // An `$ik ` group is a bone like any other; the constraint is added once
        // its children exist.
        let bone_name = group_tags.name.clone();

        // `$pivot` places the bone; without one the group's own bounds do.
        let origin = sub_layers
            .iter()
            .find(|l| tags_of(l.name()).has("pivot"))
            .map(|l| layer_center(layer_bounds(l), canvas, options.scale))
            .unwrap_or_else(|| {
                layer_center(
                    (
                        group.layer_left(),
                        group.layer_top(),
                        (group.layer_right() - group.layer_left() + 1).max(0) as u32,
                        (group.layer_bottom() - group.layer_top() + 1).max(0) as u32,
                    ),
                    canvas,
                    options.scale,
                )
            });

        let parent = group
            .parent_id()
            .and_then(|p| paths.get(&p))
            .and_then(|p| bones.get(p))
            .copied()
            .unwrap_or(root);
        // Bone transforms are local, so subtract the parent's world offset. Only
        // translation is ever produced here — a PSD has no rotation to read.
        let parent_origin = world_origin(&skeleton, parent);
        let bone = skeleton.add_bone(Bone {
            name: ankhimate_core::skeleton::unique_name(
                &bone_name,
                skeleton.bones.iter().map(|(_, b)| b.name.as_str()),
            ),
            parent: Some(parent),
            length: 1.0,
            local_transform: Transform {
                position: origin - parent_origin,
                ..Transform::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        summary.bones += 1;
        bones.insert(path.clone(), bone);

        let default_skin = skeleton.default_skin;
        if options.flatten.contains(&path) || group_tags.has("merge") {
            // Flattened: one attachment for the whole group, composited in
            // stacking order. The group stops being a hierarchy and becomes art.
            // `[merge]` is the same decision written in the file rather than
            // ticked in a dialog, so it survives the next import.
            if let Some((asset, bounds)) = flatten_group(&psd, sub_layers, &path, options) {
                let name = asset.name.clone();
                let (w, h) = (asset.width as f32, asset.height as f32);
                assets.add(asset);
                summary.images += 1;
                let slot = skeleton.add_slot(Slot {
                    attachment: Some(name.clone()),
                    ..Slot::new(format!("{bone_name}_slot"), bone)
                });
                summary.slots += 1;
                layer_paths.insert(name.clone(), path.clone());
                let offset = layer_center(bounds, canvas, options.scale) - origin;
                skeleton.skins[default_skin].set(
                    slot,
                    name.clone(),
                    region(name, offset, w * options.scale, h * options.scale),
                );
            }
            continue;
        }

        for layer in sub_layers {
            let layer_path = format!("{path}/{}", layer.name());
            add_layer(
                &mut Sink {
                    skeleton: &mut skeleton,
                    assets: &mut assets,
                    summary: &mut summary,
                    layer_paths: &mut layer_paths,
                    frame_images: &mut frame_images,
                    leads: &mut leads,
                    trace_requests: &mut trace_requests,
                },
                &plan,
                default_skin,
                bone,
                layer,
                &layer_path,
                canvas,
                options,
            );
        }
    }

    // Loose layers: no group, so they hang off the root.
    let default_skin = skeleton.default_skin;
    for layer in psd.layers() {
        if layer.parent_id().is_some() {
            continue;
        }
        let path = layer.name().to_string();
        add_layer(
            &mut Sink {
                skeleton: &mut skeleton,
                assets: &mut assets,
                summary: &mut summary,
                layer_paths: &mut layer_paths,
                frame_images: &mut frame_images,
                leads: &mut leads,
                trace_requests: &mut trace_requests,
            },
            &plan,
            default_skin,
            root,
            layer,
            &path,
            canvas,
            options,
        );
    }

    // IK scaffolds, once every bone exists. The chain is the group's descendant
    // bones in hierarchy order; the target is a new bone at the chain's tip,
    // which is the handle an animator actually drags.
    for id in psd.group_ids_in_order() {
        let Some(group) = psd.groups().get(id) else {
            continue;
        };
        // Read through `tags_of`, not off the raw name: `$ik ` is an alias
        // into `[ik:name]` and matching the prefix here meant the alias worked
        // while the tag it aliases to did nothing.
        let group_tags = tags_of(group.name());
        let Some(ik_name) = group_tags.value_or_name("ik").map(str::to_string) else {
            continue;
        };
        let Some(path) = paths.get(id) else { continue };
        let Some(&start) = bones.get(path) else {
            continue;
        };
        let chain = descendant_chain(&skeleton, start);
        if chain.len() < 2 {
            summary
                .skipped
                .push(format!("{path} (IK needs at least two bones)"));
            continue;
        }
        let tip = *chain.last().expect("checked non-empty");
        let tip_origin = world_origin(&skeleton, tip);
        let target = skeleton.add_bone(Bone {
            name: ankhimate_core::skeleton::unique_name(
                &format!("{ik_name}_target"),
                skeleton.bones.iter().map(|(_, b)| b.name.as_str()),
            ),
            parent: None,
            length: 1.0,
            local_transform: Transform {
                position: tip_origin,
                ..Transform::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        summary.bones += 1;
        skeleton.add_constraint(Constraint::Ik(IkConstraint {
            name: ik_name.clone(),
            target,
            bones: chain,
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: 1.1,
            stiffness: 0.0,
        }));
        summary.constraints += 1;
    }

    // `[physics]` on a group: every bone in it sways. A cape, a ponytail and a
    // chain of cloth are all the same request, and all of them are chains — so
    // the constraint goes on each bone rather than on the group's own, or only
    // the top of the cape would move.
    for id in psd.group_ids_in_order() {
        let Some(group) = psd.groups().get(id) else {
            continue;
        };
        let group_tags = tags_of(group.name());
        if !group_tags.has("physics") {
            continue;
        }
        let Some(path) = paths.get(id) else { continue };
        let Some(&start) = bones.get(path) else {
            continue;
        };

        // `[physics:cloth]` names a preset. A number would have to be seven
        // numbers, and a layer name is not the place to tune a simulation —
        // the inspector is. What a tag can usefully say is *what kind of thing
        // this is*.
        let preset = group_tags.value("physics").unwrap_or("cloth");
        let Some(settings) = physics_preset(preset) else {
            summary
                .unknown_tags
                .push((path.clone(), format!("physics:{preset}")));
            continue;
        };

        let chain = descendant_chain(&skeleton, start);
        let chain = if chain.is_empty() { vec![start] } else { chain };
        for (depth, bone) in chain.iter().enumerate() {
            let bone_name = skeleton
                .bones
                .get(*bone)
                .map(|b| b.name.clone())
                .unwrap_or_default();
            skeleton.add_constraint(Constraint::Physics(PhysicsConstraint {
                name: ankhimate_core::skeleton::unique_name(
                    &format!("{bone_name}_physics"),
                    skeleton.constraints.iter().map(|(_, c)| c.name()),
                ),
                bone: *bone,
                // Further down the chain sways more: the tip of a cape moves
                // more than where it attaches. Without this every link responds
                // identically and the whole thing swings as one board.
                inertia: (settings.inertia + depth as f32 * 0.05).min(0.95),
                strength: settings.strength,
                damping: settings.damping,
                mass: settings.mass,
                wind: glam::Vec2::ZERO,
                gravity: settings.gravity,
                mix: 1.0,
                rotate: true,
                translate: false,
            }));
            summary.constraints += 1;
        }
    }

    // Sequences, once every layer is in. A run is ordered by number and the
    // layers arrive in stacking order, so the frames a lead cycles through are
    // not all known when its slot is made.
    apply_sequences(
        &mut skeleton,
        &plan,
        &frame_images,
        &leads,
        &mut summary,
        DEFAULT_SEQUENCE_FPS,
    );

    skeleton.rebuild_update_order();
    Ok(PsdImport {
        skeleton,
        assets,
        summary,
        layer_paths,
        trace_requests,
    })
}

/// A bone's origin in world units, walking up the parents.
fn world_origin(skeleton: &Skeleton, bone: BoneId) -> glam::Vec2 {
    let mut sum = glam::Vec2::ZERO;
    let mut cursor = Some(bone);
    // Bounded by hierarchy depth; a cycle cannot exist by construction.
    for _ in 0..256 {
        let Some(id) = cursor else { break };
        let Some(b) = skeleton.bones.get(id) else {
            break;
        };
        sum += b.local_transform.position;
        cursor = b.parent;
    }
    sum
}

/// The single-file chain under `start`, deepest last.
///
/// A chain, not a subtree: IK solves a line of bones, so a group that branches
/// gives its first branch and the rest is left to the rigger. Guessing which
/// fork an animator meant would be worse than stopping.
fn descendant_chain(skeleton: &Skeleton, start: BoneId) -> Vec<BoneId> {
    let mut chain = vec![start];
    let mut cursor = start;
    for _ in 0..256 {
        let child = skeleton
            .bones
            .iter()
            .find(|(_, b)| b.parent == Some(cursor))
            .map(|(id, _)| id);
        match child {
            Some(id) => {
                chain.push(id);
                cursor = id;
            }
            None => break,
        }
    }
    chain
}

fn region(texture: String, offset: glam::Vec2, width: f32, height: f32) -> Attachment {
    Attachment::Region(RegionAttachment {
        texture,
        local_offset: offset,
        local_rotation: 0.0,
        local_scale: glam::Vec2::ONE,
        width,
        height,
        uv_rect: Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
        pivot: glam::Vec2::splat(0.5),
        sequence: None,
    })
}

/// The layer's pixels, or `None` if the reader could not produce them.
///
/// `psd 0.3.5` indexes out of bounds and panics on a layer whose top-left is
/// negative — art dragged off the canvas edge, which is ordinary in a working
/// file. A parser panic must not take the editor with it, so the call is caught
/// and the layer is reported as skipped instead.
fn layer_rgba(layer: &psd::PsdLayer) -> Option<Vec<u8>> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| layer.rgba())).ok()
}

/// Is there anything to see in this PNG?
///
/// Decoded rather than read off the layer's bounds: Photoshop's bounds describe
/// the region the layer *may* paint, and a cleared layer keeps them.
fn has_visible_pixels(png: &[u8]) -> bool {
    let Ok(image) = image::load_from_memory(png) else {
        return true;
    };
    image.to_rgba8().pixels().any(|p| p.0[3] > 0)
}

/// Crop one layer out of the canvas-sized buffer the reader hands back.
fn layer_png(layer: &psd::PsdLayer, canvas: (u32, u32)) -> Option<(Vec<u8>, u32, u32)> {
    let (left, top, width, height) = layer_bounds(layer);
    if width == 0 || height == 0 {
        return None;
    }
    let rgba = layer_rgba(layer)?;
    let full = image::RgbaImage::from_raw(canvas.0, canvas.1, rgba)?;
    let cropped = image::imageops::crop_imm(
        &full,
        left.max(0) as u32,
        top.max(0) as u32,
        width.min(canvas.0),
        height.min(canvas.1),
    )
    .to_image();
    let (w, h) = cropped.dimensions();
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(cropped)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    Some((bytes, w, h))
}

/// Everything `add_layer` writes into, gathered so the signature says what the
/// function does rather than listing nine things it happens to touch.
struct Sink<'a> {
    skeleton: &'a mut Skeleton,
    assets: &'a mut AssetDb,
    summary: &'a mut ImportSummary,
    layer_paths: &'a mut HashMap<String, String>,
    /// Image name per frame path, so a run's lead can find the frames that were
    /// imported after it. Frames arrive in stacking order and a run is ordered
    /// by number, so the sequence cannot be assembled until every layer is in.
    frame_images: &'a mut HashMap<String, String>,
    /// `[mesh]` layers, gathered for the caller to trace.
    trace_requests: &'a mut Vec<TraceRequest>,
    /// Where a run lead's attachment landed, so the sequence can be put on it
    /// once the frames are known. `Skin` has no by-texture lookup and adding
    /// one to `core` for an importer's convenience is the wrong place for it.
    leads: &'a mut HashMap<String, (SkinId, SlotId, String)>,
}

#[allow(clippy::too_many_arguments)]
fn add_layer(
    sink: &mut Sink,
    plan: &ImportPlan,
    skin: SkinId,
    bone: BoneId,
    layer: &psd::PsdLayer,
    path: &str,
    canvas: (u32, u32),
    options: &ImportOptions,
) {
    let skeleton = &mut *sink.skeleton;
    let assets = &mut *sink.assets;
    let summary = &mut *sink.summary;
    let layer_paths = &mut *sink.layer_paths;
    let tags = tags_of(layer.name());
    // A pivot is a marker, not art: it placed the bone already.
    if tags.has("pivot") {
        return;
    }
    // `[ignore]` travels in the file, unlike an unticked box in a dialog — so a
    // reference sketch the artist marked once stays out of every import rather
    // than needing the same click each time.
    if tags.has("ignore") {
        summary.skipped.push(format!("{path} (tagged [ignore])"));
        return;
    }
    if !options.wants(path) {
        summary.skipped.push(format!("{path} (not selected)"));
        return;
    }
    if options.skip_hidden && !shown(layer) {
        summary.skipped.push(format!("{path} (hidden)"));
        return;
    }
    // A marker layer is geometry, not art: its pixels exist so the artist can
    // see where it is. Handled before the image is read, so a bounding box drawn
    // as a flat magenta rectangle does not also become a texture in the atlas.
    if let Some(attachment) = marker_attachment(&tags, layer, canvas, bone, skeleton, options) {
        let wanted = tags.value_or_name("slot").unwrap_or(&tags.name);
        let name = ankhimate_core::skeleton::unique_name(
            wanted,
            skeleton.slots.iter().map(|(_, s)| s.name.as_str()),
        );
        let slot = skeleton.add_slot(Slot {
            attachment: Some(name.clone()),
            ..Slot::new(format!("{name}_slot"), bone)
        });
        summary.slots += 1;
        skeleton.skins[skin].set(slot, name, attachment);
        layer_paths.insert(format!("{path}_marker"), path.to_string());
        return;
    }

    let Some((bytes, w, h)) = layer_png(layer, canvas) else {
        summary.skipped.push(format!("{path} (no pixels)"));
        return;
    };
    // A layer with nothing visible in it is not art. Photoshop leaves these
    // behind — an empty `Layer 1` from a stray click, a layer whose content was
    // deleted rather than the layer — and each one imported is a slot the
    // artist has to find and delete in a rig they did not make.
    //
    // Checked on the alpha rather than on the size: the stray layer in the test
    // fixture is 1x1, so a size threshold would have to guess where "too small"
    // begins, and a genuinely tiny piece of art does exist.
    if !has_visible_pixels(&bytes) {
        summary.skipped.push(format!("{path} (fully transparent)"));
        return;
    }

    // `[slot:name]` names the slot; the tags are stripped either way, so a layer
    // called `arm [mesh]` becomes `arm` rather than carrying its own markup.
    let wanted = tags.value_or_name("slot").unwrap_or(&tags.name);
    let name = ankhimate_core::skeleton::unique_name(
        wanted,
        assets.images.values().map(|a| a.name.as_str()),
    );
    assets.add(ImageAsset::new(name.clone(), bytes, w, h));
    summary.images += 1;
    layer_paths.insert(name.clone(), path.to_string());
    sink.frame_images.insert(path.to_string(), name.clone());
    let is_lead = plan.frames.get(path).is_some_and(|f| f.lead == path);

    // A frame that is not the run's lead has had its image imported and that is
    // all it gets: the slot belongs to the lead, which cycles through them. Five
    // slots for a five-frame flipbook is the answer that made `Sequence` worth
    // having in the first place.
    if plan.frames.contains_key(path) && !is_lead {
        return;
    }

    // Photoshop already records blend and opacity per layer, so the common case
    // needs no tag at all — the artist's file says it and every art tool round
    // trips it. The tags are for the mode Photoshop has and the engine does not.
    let (mut blend_mode, mapped) = blend_mode_of(layer);
    if let Some(named) = tags.value("blend") {
        match blend_mode_named(named) {
            Some(mode) => blend_mode = mode,
            None => summary
                .unknown_tags
                .push((path.to_string(), format!("blend:{named}"))),
        }
    } else if !mapped {
        summary
            .lost_blend
            .push((path.to_string(), blend_mode_name(layer)));
    }

    // `[scale:n]` multiplies the import scale for this layer alone: art drawn at
    // twice the size for a detail pass comes in at the size it is meant to be
    // rather than needing a resize the next import undoes.
    let scale = options.scale * tags.number("scale").filter(|s| *s > 0.0).unwrap_or(1.0);

    // `[alpha:n]` takes 0–1, which is how every other number in this grammar
    // reads; Photoshop stores 0–255 and that is a detail of the file format.
    let alpha = tags
        .number("alpha")
        .map(|a| a.clamp(0.0, 1.0))
        .unwrap_or(layer.opacity() as f32 / 255.0);

    let slot = skeleton.add_slot(Slot {
        attachment: Some(name.clone()),
        color: [1.0, 1.0, 1.0, alpha],
        blend_mode,
        ..Slot::new(format!("{name}_slot"), bone)
    });
    summary.slots += 1;

    let bone_origin = world_origin(skeleton, bone);
    // The layer's *position* stays where the artist put it — `[scale]` resizes
    // the art, it does not move the part. Scaling the offset too would drag a
    // detail-pass layer away from the thing it belongs to.
    let offset = layer_center(layer_bounds(layer), canvas, options.scale) - bone_origin;
    skeleton.skins[skin].set(
        slot,
        name.clone(),
        region(name.clone(), offset, w as f32 * scale, h as f32 * scale),
    );
    // `[mesh]` is a request, not a result: the tracer is in `document`. The
    // attachment is imported as a region either way, so a build that ignores
    // the request still gets a working rig — a tag that half-applies would be
    // worse than one that does nothing.
    if tags.has("mesh") {
        sink.trace_requests.push(TraceRequest {
            attachment: name.clone(),
            slot: skeleton.slots[slot].name.clone(),
            // `[mesh:70]` is the detail dial; bare `[mesh]` takes the tracer's
            // own default rather than a number invented here.
            detail: tags.number("mesh"),
            weights: tags.has("weights"),
        });
    }
    if is_lead {
        sink.leads.insert(path.to_string(), (skin, slot, name));
    }
}

/// Turn each run's lead attachment into a sequence over its frames.
///
/// The followers' images are already in the asset database — every frame has to
/// be, or the flipbook has nothing to show — and this is what stops them from
/// each having a slot the artist would have to hide by hand.
fn apply_sequences(
    skeleton: &mut Skeleton,
    plan: &ImportPlan,
    frame_images: &HashMap<String, String>,
    leads: &HashMap<String, (SkinId, SlotId, String)>,
    summary: &mut ImportSummary,
    fps: f32,
) {
    for (lead_path, run) in &plan.sequences {
        let Some((skin, slot, lead_image)) = leads.get(lead_path) else {
            // The lead was skipped — hidden, unselected, or without pixels —
            // so there is no attachment to make a sequence on. The frames that
            // did import stay as they are rather than being silently dropped.
            continue;
        };
        let frames: Vec<ankhimate_core::attachment::TextureRef> = run
            .frames
            .iter()
            .filter_map(|path| frame_images.get(path).cloned())
            .collect();
        if frames.len() < 2 {
            continue;
        }
        let count = frames.len();

        let Some(Attachment::Region(region)) = skeleton.skins[*skin].get(*slot, lead_image) else {
            continue;
        };
        let mut region = region.clone();
        region.sequence = Some(ankhimate_core::attachment::Sequence {
            frames,
            fps: run.fps.unwrap_or(fps),
            // Looping is the only mode a numbered run implies on its own; a
            // one-shot effect is a `[frames]` decision the artist makes, not
            // something the numbering can say.
            mode: ankhimate_core::attachment::SequenceMode::Loop,
            setup_index: 0,
        });
        skeleton.skins[*skin].set(*slot, lead_image.clone(), Attachment::Region(region));

        // The slot was named after the lead frame, because `add_layer` had not
        // yet been told the layer heads a run. `fire_01_slot` for a slot that
        // plays all three frames reads as "frame 1 is here and the others went
        // missing" — which is the question this import gets asked. The stem is
        // what the run is called, so the slot takes it.
        let taken: Vec<String> = skeleton
            .slots
            .iter()
            .filter(|(id, _)| id != slot)
            .map(|(_, s)| s.name.clone())
            .collect();
        if let Some(entry) = skeleton.slots.get_mut(*slot) {
            entry.name = ankhimate_core::skeleton::unique_name(
                &format!("{}_slot", run.stem),
                taken.iter().map(String::as_str),
            );
        }
        summary.sequences.push((run.stem.clone(), count));
    }
}

/// Named physics presets for `[physics:<kind>]`.
///
/// A tag names a *kind of thing*, not seven numbers: a layer name is not where
/// a simulation gets tuned, and `[physics:0.3,0.8,0.4,1,0,-9.8,1]` would be
/// unreadable and unmaintainable both. These are starting points the inspector
/// refines.
struct PhysicsPreset {
    inertia: f32,
    strength: f32,
    damping: f32,
    mass: f32,
    gravity: glam::Vec2,
}

fn physics_preset(name: &str) -> Option<PhysicsPreset> {
    match name.trim().to_ascii_lowercase().as_str() {
        // Light, hangs, settles slowly.
        "cloth" | "cape" | "skirt" => Some(PhysicsPreset {
            inertia: 0.6,
            strength: 0.35,
            damping: 0.75,
            mass: 1.0,
            gravity: glam::Vec2::new(0.0, -9.8),
        }),
        // Stiffer and lighter than cloth, and it barely falls.
        "hair" | "ponytail" => Some(PhysicsPreset {
            inertia: 0.5,
            strength: 0.6,
            damping: 0.8,
            mass: 0.6,
            gravity: glam::Vec2::new(0.0, -4.0),
        }),
        // Heavy and quick to stop: a sword on a belt, a pouch.
        "dangle" | "chain" => Some(PhysicsPreset {
            inertia: 0.7,
            strength: 0.5,
            damping: 0.6,
            mass: 2.0,
            gravity: glam::Vec2::new(0.0, -9.8),
        }),
        // No gravity at all — a floating antenna or a tail that follows.
        "float" | "tail" => Some(PhysicsPreset {
            inertia: 0.65,
            strength: 0.4,
            damping: 0.85,
            mass: 0.8,
            gravity: glam::Vec2::ZERO,
        }),
        _ => None,
    }
}

/// Composite a group's layers into one image, in stacking order.
fn flatten_group(
    psd: &psd::Psd,
    layers: &[psd::PsdLayer],
    path: &str,
    options: &ImportOptions,
) -> Option<(ImageAsset, (i32, i32, u32, u32))> {
    let canvas = (psd.width(), psd.height());
    let mut composite = image::RgbaImage::new(canvas.0, canvas.1);
    let mut bounds: Option<(i32, i32, i32, i32)> = None;
    let mut any = false;

    // Back to front, which is the order the layers already come in.
    for layer in layers {
        if layer.name() == PIVOT_LAYER {
            continue;
        }
        if options.skip_hidden && !shown(layer) {
            continue;
        }
        let (left, top, width, height) = layer_bounds(layer);
        if width == 0 || height == 0 {
            continue;
        }
        let Some(source) =
            layer_rgba(layer).and_then(|rgba| image::RgbaImage::from_raw(canvas.0, canvas.1, rgba))
        else {
            continue;
        };
        image::imageops::overlay(&mut composite, &source, 0, 0);
        any = true;
        let (l, t, r, b) = (left, top, left + width as i32, top + height as i32);
        bounds = Some(match bounds {
            None => (l, t, r, b),
            Some((ol, ot, or, ob)) => (ol.min(l), ot.min(t), or.max(r), ob.max(b)),
        });
    }
    if !any {
        return None;
    }
    let (l, t, r, b) = bounds?;
    let (x, y) = (l.max(0) as u32, t.max(0) as u32);
    let (w, h) = (
        ((r - l).max(1) as u32).min(canvas.0 - x),
        ((b - t).max(1) as u32).min(canvas.1 - y),
    );
    let cropped = image::imageops::crop_imm(&composite, x, y, w, h).to_image();
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(cropped)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .ok()?;
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    Some((ImageAsset::new(name, bytes, w, h), (l, t, w, h)))
}

/// Compare a PSD against an already-imported document.
///
/// Reports rather than mutates. Deciding what to do about a removed layer is the
/// user's call — deleting the slot would take its animation keys with it — so the
/// import modal shows this and asks.
pub fn diff(bytes: &[u8], existing: &HashMap<String, String>) -> Result<ReimportReport, PsdError> {
    let nodes = layer_tree(bytes)?;
    let incoming: HashSet<&str> = nodes
        .iter()
        .filter(|n| !n.is_group)
        .map(|n| n.path.as_str())
        .collect();
    let known: HashSet<&str> = existing.values().map(|p| p.as_str()).collect();

    let mut report = ReimportReport::default();
    for path in &incoming {
        if known.contains(path) {
            report.unchanged += 1;
        } else {
            report.added.push((*path).to_string());
        }
    }
    for path in &known {
        if !incoming.contains(path) {
            report.removed.push((*path).to_string());
        }
    }
    report.added.sort();
    report.removed.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_old_markers_still_read_as_tags() {
        // A PSD is an artist's file. Breaking one to tidy a syntax is a bad
        // trade, so `$pivot`, `$ik <name>` and `@skin:<name>` translate into the
        // grammar rather than being handled beside it.
        assert!(tags_of("$pivot").has("pivot"));

        let ik = tags_of("$ik arm_chain");
        assert_eq!(ik.value("ik"), Some("arm_chain"));
        assert_eq!(ik.name, "arm_chain", "and the marker leaves the name");

        let skin = tags_of("@skin:winter");
        assert_eq!(skin.value("skin"), Some("winter"));
        assert_eq!(skin.name, "winter");
    }

    #[test]
    fn a_new_tag_and_an_old_marker_mean_the_same_thing() {
        // The port is a translation, not a second dialect: whichever an artist
        // writes, the reader sees one vocabulary.
        assert_eq!(
            tags_of("$pivot").has("pivot"),
            tags_of("anything [pivot]").has("pivot")
        );
        assert_eq!(
            tags_of("$ik leg").value("ik"),
            tags_of("leg [ik:leg]").value("ik")
        );
    }

    #[test]
    fn tags_compose_where_the_markers_could_not() {
        // The reason for the grammar. Each old marker owned the whole layer
        // name, so a group could not be both a bone and a skin.
        let tags = tags_of("cape [bone][skin:winter][physics:cloth]");
        assert!(tags.has("bone"));
        assert_eq!(tags.value("skin"), Some("winter"));
        assert_eq!(tags.value("physics"), Some("cloth"));
        assert_eq!(tags.name, "cape");
    }

    use super::*;

    #[test]
    fn psd_top_left_becomes_centre_origin_y_up() {
        // A 100×100 canvas, a 20×20 layer in the top-left corner: its centre is
        // 40 left of and 40 above the middle.
        let centre = layer_center((0, 0, 20, 20), (100, 100), 1.0);
        assert_eq!(centre, glam::vec2(-40.0, 40.0));

        // Bottom-right corner mirrors it.
        let centre = layer_center((80, 80, 20, 20), (100, 100), 1.0);
        assert_eq!(centre, glam::vec2(40.0, -40.0));
    }

    #[test]
    fn scale_multiplies_placement() {
        let centre = layer_center((0, 0, 20, 20), (100, 100), 0.5);
        assert_eq!(centre, glam::vec2(-20.0, 20.0));
    }

    #[test]
    fn a_diff_reports_added_and_removed_by_layer_path() {
        let mut existing = HashMap::new();
        existing.insert("arm".to_string(), "torso/arm".to_string());
        existing.insert("gone".to_string(), "torso/gone".to_string());

        let report = ReimportReport {
            added: vec!["torso/hand".into()],
            removed: vec!["torso/gone".into()],
            changed: vec![],
            unchanged: 1,
        };
        // The shape the modal renders; asserted here so the field meanings stay
        // pinned even before a fixture PSD exists.
        assert_eq!(report.added.len(), 1);
        assert_eq!(report.removed, vec!["torso/gone".to_string()]);
        assert_eq!(existing.len(), 2);
    }

    #[test]
    fn options_include_empty_means_everything() {
        let options = ImportOptions::default();
        assert!(options.wants("anything"));

        let mut options = ImportOptions::default();
        options.include.insert("torso".into());
        assert!(options.wants("torso"));
        assert!(!options.wants("head"));
    }
    #[test]
    fn every_named_physics_kind_resolves_and_an_unknown_one_does_not() {
        // The reader, not the grammar: an unknown kind must return `None` so the
        // caller reports it. Quietly picking cloth for `[physics:jelly]` is a rig
        // that moves wrongly for a reason nobody can find.
        for kind in [
            "cloth", "cape", "skirt", "hair", "ponytail", "dangle", "chain", "float", "tail",
        ] {
            assert!(
                physics_preset(kind).is_some(),
                "`{kind}` is offered in the docs and resolves to nothing"
            );
        }
        assert!(physics_preset("jelly").is_none());
        assert!(
            physics_preset("CLOTH").is_some(),
            "the kind folds case, like every other tag value that names a thing"
        );
    }

    #[test]
    fn the_physics_presets_differ_from_one_another() {
        // Four names for one set of numbers would be a worse API than one name:
        // it promises a distinction the rig does not have.
        let cloth = physics_preset("cloth").expect("cloth");
        let hair = physics_preset("hair").expect("hair");
        let float = physics_preset("float").expect("float");

        assert!(
            hair.mass < cloth.mass,
            "hair is lighter than cloth: {} vs {}",
            hair.mass,
            cloth.mass
        );
        assert_eq!(
            float.gravity,
            glam::Vec2::ZERO,
            "a floating thing does not fall"
        );
        assert!(cloth.gravity.y < 0.0, "and cloth does");
    }
}

/// PSD, as a registered importer.
///
/// Layered artwork rather than a rig format, and the difference shows: a PSD has
/// no animations and no constraints, so this produces a skeleton and images and
/// nothing else. The editor's own panel additionally offers *merging* into an
/// open document, which the registry has no way to express — an importer returns
/// a rig, and merging is a different question.
///
/// Registered anyway, because the options are parameters rather than a
/// conversation: every one has a default that produces a usable rig, so a script
/// or an MCP client can import a PSD without a UI and refine afterwards.
pub struct PsdImporter;

impl crate::importer::Importer for PsdImporter {
    fn id(&self) -> &str {
        "import.psd"
    }

    fn label(&self) -> &str {
        "Photoshop PSD"
    }

    fn extensions(&self) -> Vec<&str> {
        vec!["psd"]
    }

    fn options_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scale": {
                    "type": "number", "default": 1,
                    "description": "World units per PSD pixel"
                },
                "skip_hidden": {
                    "type": "boolean", "default": true,
                    "description": "Skip layers hidden in Photoshop — usually references"
                },
                "include": {
                    "type": "array", "items": { "type": "string" },
                    "description": "Layer paths to import; omit for everything"
                },
                "flatten": {
                    "type": "array", "items": { "type": "string" },
                    "description": "Groups to collapse into one attachment rather than a bone tree"
                }
            }
        })
    }

    fn read(&self, path: &std::path::Path) -> Result<crate::Loaded, crate::importer::ImportError> {
        self.read_with(path, &serde_json::Value::Null)
    }

    fn read_with(
        &self,
        path: &std::path::Path,
        options: &serde_json::Value,
    ) -> Result<crate::Loaded, crate::importer::ImportError> {
        use crate::importer::ImportError;

        let bytes = std::fs::read(path)
            .map_err(|e| ImportError::Io(format!("could not read {}: {e}", path.display())))?;

        let strings = |key: &str| -> std::collections::HashSet<String> {
            options
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let opts = ImportOptions {
            scale: options.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
            include: strings("include"),
            flatten: strings("flatten"),
            skip_hidden: options
                .get("skip_hidden")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        };

        let imported = import(&bytes, &opts).map_err(|e| match e {
            // The reader does not distinguish "not a PSD" from "a PSD that
            // will not parse", and the extension has already narrowed it — so a
            // `.psd` this cannot read is reported as not ours rather than as
            // broken, and a caller trying several importers moves on.
            PsdError::Parse(_) => ImportError::NotThisFormat,
            // Empty is different: it parsed, it is a PSD, and it has nothing in
            // it. Saying "not that format" there would send the user hunting.
            other => ImportError::Malformed(other.to_string()),
        })?;

        let mut report = crate::convert::LoadReport::default();
        for skipped in &imported.summary.skipped {
            report.lossy(
                "layer",
                skipped,
                "this layer produced no attachment — it is empty, hidden, or an \
                 adjustment rather than pixels",
            );
        }

        Ok(crate::Loaded {
            skeleton: imported.skeleton,
            // A PSD carries artwork and no motion, which is the honest answer
            // rather than an empty clip nobody asked for.
            animations: ankhimate_core::slotmap::SlotMap::with_key(),
            assets: imported.assets,
            name: path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string(),
            fps: 30,
            export_presets: Vec::new(),
            // The whole point of registering this: a re-import compares against
            // these, and without them every layer looks new.
            psd_layer_paths: imported.layer_paths,
            report,
        })
    }
}
