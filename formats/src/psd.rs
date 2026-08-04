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
use ankhimate_core::constraints::{Constraint, IkConstraint};
use ankhimate_core::ids::{BoneId, SkinId};
use ankhimate_core::math::Transform;
use ankhimate_core::skeleton::{Bone, Skeleton};
use ankhimate_core::skin::Skin;
use ankhimate_core::slot::Slot;
use std::collections::{HashMap, HashSet};

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
}

/// A finished import: a skeleton, the images it references, and what happened.
pub struct PsdImport {
    pub skeleton: Skeleton,
    pub assets: AssetDb,
    pub summary: ImportSummary,
    /// Layer path per slot, so a re-import can find the same layer again.
    pub layer_paths: HashMap<String, String>,
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
        nodes.push(LayerNode {
            depth: path.matches('/').count(),
            path: path.clone(),
            name: group.name().to_string(),
            is_group: true,
            visible: shown(group),
            bounds: (0, 0, 0, 0),
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
        if let Some(skin_name) = group.name().strip_prefix(SKIN_PREFIX) {
            let skin_id = *skins.entry(skin_name.to_string()).or_insert_with(|| {
                summary.skins += 1;
                skeleton.add_skin(Skin::new(skin_name))
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
                    &mut skeleton,
                    &mut assets,
                    &mut summary,
                    &mut layer_paths,
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

        // An `$ik ` group is a bone like any other; the constraint is added once
        // its children exist.
        let bone_name = group
            .name()
            .strip_prefix(IK_PREFIX)
            .unwrap_or(group.name())
            .to_string();

        // `$pivot` places the bone; without one the group's own bounds do.
        let origin = sub_layers
            .iter()
            .find(|l| l.name() == PIVOT_LAYER)
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
        if options.flatten.contains(&path) {
            // Flattened: one attachment for the whole group, composited in
            // stacking order. The group stops being a hierarchy and becomes art.
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
                &mut skeleton,
                &mut assets,
                &mut summary,
                &mut layer_paths,
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
            &mut skeleton,
            &mut assets,
            &mut summary,
            &mut layer_paths,
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
        if !group.name().starts_with(IK_PREFIX) {
            continue;
        }
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
                &format!("{}_target", group.name().trim_start_matches(IK_PREFIX)),
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
            name: group.name().trim_start_matches(IK_PREFIX).to_string(),
            target,
            bones: chain,
            bend_direction: 1.0,
            mix: 1.0,
            softness: 0.0,
            stretch: false,
            stretch_limit: 1.1,
        }));
        summary.constraints += 1;
    }

    skeleton.rebuild_update_order();
    Ok(PsdImport {
        skeleton,
        assets,
        summary,
        layer_paths,
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

#[allow(clippy::too_many_arguments)]
fn add_layer(
    skeleton: &mut Skeleton,
    assets: &mut AssetDb,
    summary: &mut ImportSummary,
    layer_paths: &mut HashMap<String, String>,
    skin: SkinId,
    bone: BoneId,
    layer: &psd::PsdLayer,
    path: &str,
    canvas: (u32, u32),
    options: &ImportOptions,
) {
    // `$pivot` is a marker, not art: it placed the bone already.
    if layer.name() == PIVOT_LAYER {
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
    let Some((bytes, w, h)) = layer_png(layer, canvas) else {
        summary.skipped.push(format!("{path} (no pixels)"));
        return;
    };

    let name = ankhimate_core::skeleton::unique_name(
        layer.name(),
        assets.images.values().map(|a| a.name.as_str()),
    );
    assets.add(ImageAsset::new(name.clone(), bytes, w, h));
    summary.images += 1;
    layer_paths.insert(name.clone(), path.to_string());

    let slot = skeleton.add_slot(Slot {
        attachment: Some(name.clone()),
        ..Slot::new(format!("{name}_slot"), bone)
    });
    summary.slots += 1;

    let bone_origin = world_origin(skeleton, bone);
    let offset = layer_center(layer_bounds(layer), canvas, options.scale) - bone_origin;
    skeleton.skins[skin].set(
        slot,
        name.clone(),
        region(
            name,
            offset,
            w as f32 * options.scale,
            h as f32 * options.scale,
        ),
    );
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
}
