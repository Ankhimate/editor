//! The template context (T-603b) — what a template can see.
//!
//! # This is a public contract
//!
//! Once users have written templates, renaming a field here breaks their work
//! silently: there is no compiler on the other side, only a template that starts
//! producing the wrong file. Treat this module the way `formats::schema` is
//! treated (ADR 0004) — additive changes are free, renames are breaking and need
//! a [`CONTEXT_VERSION`] bump.
//!
//! The context is a projection of the **disk** schema, not of `core`: named
//! references, degrees for angles (PLAN §2.7). That is deliberate — an exporter
//! and a save file want exactly the same shape, so there is no third
//! representation to keep in sync.
//!
//! Two things are added that the schema does not store, because a logic-less
//! template provably cannot derive them:
//!
//! - `bone.index` and `bone.parent_index` — engines that store bones as a flat
//!   array with integer parent references need an ordering, and a template
//!   cannot topologically sort.
//! - `bone.children` — Handlebars cannot invert a parent pointer.
//!
//! And one thing is deliberately withheld: **no timestamp**. A timestamp makes
//! every export differ from the last, which destroys diffability and turns "did
//! the rig actually change?" into an unanswerable question in version control.

use crate::atlas::Atlas;
use ankhimate_core::pose::{self, Pose};
use ankhimate_core::transforms::Affine2;
use ankhimate_formats::schema::{self, Project};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Bumped when a field is renamed or removed. Additions do not bump it.
pub const CONTEXT_VERSION: u32 = 1;

/// Re-expresses a point `(x, y)` from a slot's bone space in another bone's.
///
/// Threaded through attachment building so weighted vertices can be written per
/// influence; see [`to_bone_space`].
type BoneSpace<'a> = dyn Fn(&str, &str, f32, f32) -> (f32, f32) + 'a;

/// Everything a template can address, as a JSON tree.
#[derive(Debug, Clone, Serialize)]
pub struct Context {
    pub context_version: u32,
    pub project: Value,
    pub skeleton: Value,
    pub animations: Vec<Value>,
    /// Every distinct event name across every clip, sorted.
    ///
    /// Formats that declare events once at the top level need this, and a
    /// template cannot compute it: deduplicating across a nested loop is beyond
    /// what Handlebars expresses.
    pub event_names: Vec<String>,
    /// Absent when the preset bakes no atlas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atlas: Option<Value>,
    pub export: Value,
}

impl Context {
    /// Build the context for one export.
    pub fn build(project: &Project, atlas: Option<&Atlas>, export: ExportInfo) -> Self {
        Self {
            context_version: CONTEXT_VERSION,
            project: json!({
                "name": project.name,
                "fps": project.fps,
                "version": project.version,
            }),
            skeleton: skeleton(project),
            animations: project
                .animations
                .iter()
                .map(|a| animation(a, project))
                .collect(),
            event_names: event_names(project),
            atlas: atlas.map(|a| atlas_value(a, &export.atlas_stem, art_scale(project))),
            export: json!({
                "output_dir": export.output_dir,
                "preset_name": export.preset_name,
                "template_name": export.template_name,
            }),
        }
    }

    /// The same context with `animation` bound to one clip, for a
    /// `per: animation` template.
    pub fn with_animation(&self, index: usize) -> Value {
        let mut root = serde_json::to_value(self).expect("context serializes");
        if let (Some(map), Some(anim)) = (root.as_object_mut(), self.animations.get(index)) {
            map.insert("animation".into(), anim.clone());
        }
        root
    }

    /// The context as a plain JSON value, with no single animation bound.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("context serializes")
    }
}

/// Which export this is, for templates that name their own output.
#[derive(Debug, Clone, Default)]
pub struct ExportInfo {
    pub output_dir: String,
    pub preset_name: String,
    pub template_name: String,
    /// Filename stem the atlas pages were written under.
    ///
    /// A template naming the atlas in a second file — an `.atlas` index beside
    /// the `.png` — has to agree with what was actually written, and the stem is
    /// a preset setting the context would otherwise not see.
    pub atlas_stem: String,
}

/// How much larger the rig's coordinates are than its source art.
///
/// A rig authored against half-resolution images records each attachment at
/// twice its pixel size. Several atlas formats carry that ratio in a header so
/// the consumer can scale regions back up; without it every sprite draws at half
/// size. Computed as the median of per-region ratios so one odd attachment — a
/// deliberately stretched region — cannot skew it, and 1.0 when nothing can be
/// compared.
fn art_scale(project: &Project) -> f32 {
    let mut ratios: Vec<f32> = Vec::new();
    for skin in &project.skins {
        for entry in &skin.entries {
            // Regions only: a region's `width`/`height` are a draw size to
            // compare against the file, while a mesh's are implied by its
            // vertices and carry no such ratio.
            let schema::Attachment::Region(r) = &entry.attachment else {
                continue;
            };
            let Some(asset) = project.assets.iter().find(|a| a.name == r.texture) else {
                continue;
            };
            if asset.width > 0 && r.width > 0.0 {
                ratios.push(r.width / asset.width as f32);
            }
            if asset.height > 0 && r.height > 0.0 {
                ratios.push(r.height / asset.height as f32);
            }
        }
    }
    if ratios.is_empty() {
        return 1.0;
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).expect("a size ratio is never NaN"));
    let median = ratios[ratios.len() / 2];
    if median <= 0.0 {
        return 1.0;
    }
    // Snap a near-integer ratio to the integer. Odd pixel dimensions make a
    // genuinely half-scale rig report 1.9778 on one attachment and 2.0 on the
    // next; consumers divide by this, so an unsnapped 1.9899 becomes a header
    // reading "0.50254" where "0.5" was meant — noise in every diff, and a
    // half-pixel drift across a large region.
    let rounded = median.round();
    if rounded >= 1.0 && (median - rounded).abs() < 0.05 {
        rounded
    } else {
        median
    }
}

fn skeleton(project: &Project) -> Value {
    let order = bone_order(project);
    let index_of = |name: &str| order.iter().position(|n| n == name);
    let asset_size = |name: &str| {
        project
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| (a.width as f32, a.height as f32))
    };
    let scale = art_scale(project);
    // Weighted vertices are stored once, in the space of the bone their slot
    // hangs from, but exported per influence in each influence's own frame.
    let worlds = setup_worlds(project);
    let host_of = |slot: &str| {
        project
            .slots
            .iter()
            .find(|s| s.name == slot)
            .and_then(|s| worlds.get(&s.bone))
    };
    let bone_space = |slot: &str, bone: &str, x: f32, y: f32| {
        match (host_of(slot), worlds.get(bone)) {
            (Some(host), Some(target)) => to_bone_space(host, target, x, y),
            // An unresolved name is reported at load, not here; leaving the
            // point alone misplaces one influence rather than all of them.
            _ => (x, y),
        }
    };

    let bones: Vec<Value> = order
        .iter()
        .filter_map(|name| project.bones.iter().find(|b| &b.name == name))
        .map(|b| {
            let children: Vec<&str> = order
                .iter()
                .filter(|n| {
                    project
                        .bones
                        .iter()
                        .any(|c| &&c.name == n && c.parent == b.name)
                })
                .map(|n| n.as_str())
                .collect();
            json!({
                "name": b.name,
                "parent": b.parent,
                "index": index_of(&b.name),
                // -1 rather than null: a template writing a flat array needs a
                // number in the field either way, and `{{parent_index}}` must
                // not render "null" into a numeric slot.
                "parent_index": if b.parent.is_empty() { -1 } else {
                    index_of(&b.parent).map(|i| i as i64).unwrap_or(-1)
                },
                "length": b.length,
                "x": b.tx,
                "y": b.ty,
                "rotation": b.rotation,
                "scale_x": b.sx,
                "scale_y": b.sy,
                "shear_x": b.shear_x,
                "shear_y": b.shear_y,
                "inherit_rotation": b.inherit_rotation,
                "inherit_scale": b.inherit_scale,
                "inherit_reflect": b.inherit_reflect,
                "color": b.color,
                "children": children,
            })
        })
        .collect();

    let slots: Vec<Value> = project
        .slots
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "bone": s.bone,
                "attachment": s.attachment,
                "color": s.color,
                "dark_color": s.dark_color,
                "blend": if s.blend_mode.is_empty() { "normal" } else { &s.blend_mode },
            })
        })
        .collect();

    let skins: Vec<Value> = project
        .skins
        .iter()
        .map(|skin| {
            let entries: Vec<Value> = skin
                .entries
                .iter()
                .map(|e| {
                    json!({
                        "slot": e.slot,
                        "name": e.name,
                        "attachment": attachment(&e.attachment, &e.slot, &index_of, &asset_size, scale, &bone_space),
                    })
                })
                .collect();
            // The same entries grouped by slot. A format nesting attachments as
            // `slot: { name: {…} }` needs one key per slot, and a slot with two
            // attachments rendered from the flat list emits its key twice —
            // valid JSON that silently loses an attachment on parse. Handlebars
            // cannot group, so the grouping ships.
            let mut by_slot: Vec<Value> = Vec::new();
            for entry in &skin.entries {
                let found = by_slot
                    .iter_mut()
                    .find(|s| s["slot"] == json!(entry.slot))
                    .and_then(|s| s.get_mut("attachments"))
                    .and_then(|a| a.as_array_mut());
                let value = json!({
                    "name": entry.name,
                    "attachment": attachment(&entry.attachment, &entry.slot, &index_of, &asset_size, scale, &bone_space),
                });
                match found {
                    Some(list) => list.push(value),
                    None => by_slot.push(json!({
                        "slot": entry.slot,
                        "attachments": [value],
                    })),
                }
            }

            json!({
                "name": skin.name,
                "entries": entries,
                "slots": by_slot,
                "bones": skin.bones,
                "constraints": skin.constraints,
            })
        })
        .collect();

    let constraints: Vec<Value> = project.constraints.iter().map(constraint).collect();
    // Also split by kind. Most published formats put each constraint type in its
    // own top-level block, and a template cannot emit one: filtering inside
    // `{{#each}}` still visits every item, so `@last` marks the last *constraint*
    // rather than the last IK constraint, and the commas come out wrong. Pre-split
    // lists are the difference between those formats being writable and not.
    let of_kind = |kind: &str| -> Vec<Value> {
        constraints
            .iter()
            .filter(|c| c["type"] == kind)
            .cloned()
            .collect()
    };

    json!({
        "bones": bones,
        "slots": slots,
        "draw_order": project.draw_order,
        "skins": skins,
        "default_skin": project.default_skin,
        "constraints": constraints,
        "ik": of_kind("ik"),
        "transform": of_kind("transform"),
        "path_constraints": of_kind("path"),
        "physics": of_kind("physics"),
        "constraint_order": project.constraint_order,
    })
}

/// Every distinct event name in the project, sorted for a stable export.
fn event_names(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = project
        .animations
        .iter()
        .flat_map(|a| a.events.iter().map(|e| e.name.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Bones parents-before-children.
///
/// A runtime that applies transforms in array order needs this; the saved order
/// is authoring order and carries no such guarantee. Ties break by name so the
/// result is deterministic.
fn bone_order(project: &Project) -> Vec<String> {
    let mut roots: Vec<&str> = project
        .bones
        .iter()
        .filter(|b| b.parent.is_empty())
        .map(|b| b.name.as_str())
        .collect();
    roots.sort_unstable();

    let mut out: Vec<String> = Vec::with_capacity(project.bones.len());
    let mut stack: Vec<&str> = roots.into_iter().rev().collect();
    while let Some(name) = stack.pop() {
        if out.iter().any(|n| n == name) {
            continue; // A cycle, or a name used twice. Do not spin.
        }
        out.push(name.to_string());
        let mut children: Vec<&str> = project
            .bones
            .iter()
            .filter(|b| b.parent == name)
            .map(|b| b.name.as_str())
            .collect();
        children.sort_unstable();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }

    // A bone whose parent does not exist is orphaned rather than dropped: losing
    // it silently would be worse than exporting it as a root.
    for b in &project.bones {
        if !out.iter().any(|n| n == &b.name) {
            out.push(b.name.clone());
        }
    }
    out
}

/// A mesh's edges, falling back to the outline implied by its triangulation.
///
/// Authored edges (T-401) win when present. When the mesh has none — every mesh
/// imported from a format that did not carry them — the **boundary** is still
/// recoverable: an edge shared by exactly one triangle is on the perimeter, and
/// one shared by two is interior. That is a property of the triangulation, not a
/// guess about vertex order, which is what `editor/src/meshgen.rs` rightly
/// refuses to do.
///
/// Consumers treat a mesh with no edges as one whose edge structure was dropped
/// on the way out — Spine says "mesh internal edges lost" and rebuilds its own —
/// so emitting the real boundary is strictly better than emitting nothing.
///
/// Returned as flat vertex-index pairs, matching `schema::Mesh::edges`.
fn mesh_edges(m: &schema::Mesh) -> Vec<u32> {
    if !m.edges.is_empty() {
        return m.edges.clone();
    }

    // Count how many triangles each undirected edge belongs to.
    let mut counts: std::collections::BTreeMap<(u32, u32), usize> = Default::default();
    for tri in m.triangles.chunks_exact(3) {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            let key = if a <= b { (a, b) } else { (b, a) };
            *counts.entry(key).or_default() += 1;
        }
    }

    // `BTreeMap` so the result is ordered by vertex index and therefore stable:
    // an export that reshuffles its own edges every run is undiffable.
    counts
        .into_iter()
        .filter(|(_, n)| *n == 1)
        .flat_map(|((a, b), _)| [a, b])
        .collect()
}

/// How many leading vertices form the mesh's outline.
///
/// Formats that store an outline require its vertices **first** in the array,
/// with `hull` counting them; everything after is interior, and that split is
/// what lets a consuming editor drag a silhouette or retriangulate.
///
/// `editor/src/meshgen.rs` is right that vertex *order* cannot reveal which
/// points are on the perimeter — "a valid pentagon with a notch" is
/// indistinguishable from a quad with a centre vertex. But the **triangulation**
/// can: an edge belonging to one triangle is a boundary edge. So the boundary is
/// computed, and then checked to be exactly the leading run `0..n`. Ankhimate's
/// tracer builds meshes contour-first (`meshgen.rs`, "Contour points first"), so
/// it is — and when it is not, this falls back to reporting every vertex as
/// hull, which renders correctly and merely costs the consumer its interior
/// edges. Never a hull that would slice a mesh in the wrong place.
fn mesh_hull(m: &schema::Mesh) -> usize {
    let count = m.vertices.len() / 2;
    let boundary: std::collections::BTreeSet<u32> = mesh_edges(m).into_iter().collect();
    if boundary.is_empty() {
        return count;
    }
    // A prefix iff the largest boundary index is one below the count of them.
    match boundary.iter().next_back() {
        Some(&max) if max as usize + 1 == boundary.len() => boundary.len(),
        _ => count,
    }
}

fn attachment(
    att: &schema::Attachment,
    slot: &str,
    bone_index: &dyn Fn(&str) -> Option<usize>,
    asset_size: &dyn Fn(&str) -> Option<(f32, f32)>,
    art_scale: f32,
    bone_space: &BoneSpace,
) -> Value {
    // Both a region and a mesh need their image's size. A region draws the whole
    // file at a declared size; a mesh's UVs are normalized 0..1 and a consumer
    // multiplies them by the image dimensions to find the pixels. Omit them from
    // a mesh and the UVs scale against nothing — Spine reads the missing field
    // as 0 and the mesh explodes across the skeleton.
    let source = match att {
        schema::Attachment::Region(r) => asset_size(&r.texture),
        schema::Attachment::Mesh(m) => asset_size(&m.texture),
        _ => None,
    }
    .map_or((None, None), |(w, h)| (Some(w), Some(h)));
    match att {
        schema::Attachment::Region(r) => json!({
            "type": "region",
            "texture": r.texture,
            "x": r.offset_x,
            "y": r.offset_y,
            "rotation": r.rotation,
            "scale_x": r.scale_x,
            "scale_y": r.scale_y,
            "width": r.width,
            "height": r.height,
            // The source image's own pixel size.
            //
            // A rig authored against half-resolution art records `width`/`height`
            // at twice the file's size, so these two disagree. Offered for a
            // format that addresses the file rather than the rig; a format
            // declaring a *draw* size wants `width`/`height` and the attachment's
            // own `scale_x`/`scale_y`, which are already the rig's truth.
            // Falls back to the declared size when the asset is unknown, so a
            // template can address these unconditionally: a rig referencing a
            // missing image should export a wrong-sized region, not fail to
            // render at all.
            "source_width": source.0.unwrap_or(r.width),
            "source_height": source.1.unwrap_or(r.height),
            "uv": r.uv,
            "pivot_x": r.pivot_x,
            "pivot_y": r.pivot_y,
            "sequence": r.sequence.as_ref().map(sequence),
        }),
        schema::Attachment::Mesh(m) => json!({
            "type": "mesh",
            "texture": m.texture,
            // The image's own pixel size. A mesh's `uvs` are normalized 0..1, so
            // a consumer multiplies by these to reach the texture; without them
            // the mesh has no scale at all. Zero when the asset is unknown,
            // which is visible rather than silently wrong.
            "source_width": source.0.unwrap_or(0.0),
            "source_height": source.1.unwrap_or(0.0),
            // The same size in the **rig's** coordinate space.
            //
            // A mesh's vertices are already rig-space, so a format declaring a
            // mesh's dimensions alongside them wants this, not the file size —
            // they must agree or the mesh scales against the wrong extent. On a
            // rig authored at the art's own scale the two are equal; on one
            // authored against half-resolution art these are twice the file.
            "scaled_width": source.0.unwrap_or(0.0) * art_scale,
            "scaled_height": source.1.unwrap_or(0.0) * art_scale,
            "vertices": m.vertices,
            "uvs": m.uvs,
            "triangles": m.triangles,
            "edges": mesh_edges(m),
            // The same edges with each index doubled. Several formats address a
            // flat `[x, y, x, y, …]` vertex array and so store edge endpoints as
            // *component* offsets, not vertex indices. A template cannot map an
            // arithmetic operation over an array, so the doubled form is emitted
            // alongside rather than left as an exercise.
            "edges_x2": mesh_edges(m).iter().map(|i| i * 2).collect::<Vec<_>>(),
            "hull": mesh_hull(m),
            "weights": packed_weights(&m.weights, &m.vertices, slot, bone_space),
            "flat_vertices": flat_vertices(&m.weights, &m.vertices, bone_index, slot, bone_space),
            "weighted": !m.weights.is_empty(),
            "vertex_count": m.vertices.len() / 2,
            "linked": m.linked.as_ref().map(|l| json!({
                "skin": l.skin,
                "slot": l.slot,
                "attachment": l.attachment,
                "inherit_deform": l.inherit_deform,
            })),
            "sequence": m.sequence.as_ref().map(sequence),
        }),
        schema::Attachment::Clipping(c) => json!({
            "type": "clipping",
            "vertices": c.vertices,
            "vertex_count": c.vertices.len() / 2,
            "end_slot": c.end_slot,
        }),
        schema::Attachment::Path(p) => json!({
            "type": "path",
            "vertices": p.vertices,
            "vertex_count": p.vertices.len() / 2,
            "closed": p.closed,
            "constant_speed": p.constant_speed,
        }),
        schema::Attachment::BoundingBox(b) => json!({
            "type": "bounding_box",
            "vertices": b.vertices,
            "vertex_count": b.vertices.len() / 2,
            "weights": packed_weights(&b.weights, &b.vertices, slot, bone_space),
            "flat_vertices": flat_vertices(&b.weights, &b.vertices, bone_index, slot, bone_space),
            "weighted": !b.weights.is_empty(),
        }),
        schema::Attachment::Point(p) => json!({
            "type": "point",
            "x": p.x,
            "y": p.y,
            "rotation": p.rotation,
        }),
    }
}

/// Weights in the standard packed form: per vertex, a count followed by that
/// many `{bone, x, y, weight}` entries.
///
/// Restructuring nested arrays is exactly what a logic-less template cannot do,
/// and every runtime format wants some variant of this shape. Doing it once here
/// beats every preset author reimplementing it — badly, or not at all.
///
/// `x`/`y` are the vertex in **that influence's bone space**, not the mesh's.
///
/// A rig stores one position per vertex, shared by every influence, because
/// `core` skins by transforming that one position through each bone. A runtime
/// that stores weights per influence expects the opposite: the vertex already
/// expressed in each bone's local frame, so skinning is a weighted sum with no
/// inverse left to apply.
///
/// This once wrote the shared mesh-space position for every influence. A vertex
/// bound to one bone near the origin survives that; one bound to two bones far
/// from it — a head bound to hair and a head control — flies apart. Every
/// weighted mesh on a real rig was wrong, and no field-by-field diff against a
/// reference file showed it, because both files were structurally identical.
fn packed_weights(
    weights: &[Vec<(String, f32)>],
    vertices: &[f32],
    slot: &str,
    bone_space: &BoneSpace,
) -> Vec<Value> {
    weights
        .iter()
        .enumerate()
        .map(|(i, per_vertex)| {
            let (x, y) = (
                vertices.get(i * 2).copied().unwrap_or(0.0),
                vertices.get(i * 2 + 1).copied().unwrap_or(0.0),
            );
            let bones: Vec<Value> = per_vertex
                .iter()
                .map(|(bone, weight)| {
                    let (bx, by) = bone_space(slot, bone, x, y);
                    json!({ "bone": bone, "x": bx, "y": by, "weight": weight })
                })
                .collect();
            json!({ "count": bones.len(), "bones": bones })
        })
        .collect()
}

/// Setup-pose world transform per bone, by name.
///
/// Weighted vertices are expressed relative to each bone they bind to, which
/// needs the bone's world placement — and that is `core`'s answer, not a
/// re-derivation. `evaluate()` applies constraints *before* composing worlds
/// (PLAN §2.6), so an IK-driven bone rests somewhere its local transform alone
/// does not predict.
///
/// Composing the FK chain here instead put every tip bone of an IK chain a few
/// degrees off, and with it every vertex weighted to one. The editor draws
/// `evaluate()`; an export that disagrees with it is wrong by definition.
fn setup_worlds(project: &Project) -> BTreeMap<String, Affine2> {
    let loaded = ankhimate_formats::convert::from_schema(project);
    let mut pose = Pose::default();
    pose::evaluate(&loaded.skeleton, &[], &mut pose);

    loaded
        .skeleton
        .bones
        .iter()
        .filter_map(|(id, bone)| pose.worlds.get(id).map(|w| (bone.name.clone(), *w)))
        .collect()
}

/// Re-expresses a point from one bone's space in another's.
///
/// A mesh's vertices are stored in the space of the bone its slot hangs from.
/// An influence needs the same point relative to *its* bone, which is the host's
/// world transform followed by the inverse of the influence's: lift into world
/// space, then drop back down.
///
/// A singular matrix — a bone scaled to zero on an axis — has no inverse; the
/// point passes through unchanged rather than becoming NaN and poisoning the
/// whole vertex array.
fn to_bone_space(host: &Affine2, bone: &Affine2, x: f32, y: f32) -> (f32, f32) {
    let world = host.transform_point(glam::Vec2::new(x, y));
    let det = bone.a * bone.d - bone.b * bone.c;
    if det.abs() < f32::EPSILON {
        return (world.x, world.y);
    }
    let (dx, dy) = (world.x - bone.tx, world.y - bone.ty);
    (
        (bone.d * dx - bone.c * dy) / det,
        (bone.a * dy - bone.b * dx) / det,
    )
}

/// The same weights as one **flat number array**, bones addressed by index.
///
/// Per vertex: either an `x, y` pair when unweighted, or a count followed by
/// `bone_index, x, y, weight` for each influence. Several published formats
/// specify exactly this encoding, and a template cannot produce it — it needs
/// to flatten nested arrays *and* resolve names to positions, neither of which
/// Handlebars can do. Emitting it here is the difference between those formats
/// being writable and not.
///
/// A vertex with no influences falls back to its plain `x, y` pair, which is
/// what an unweighted mesh is.
fn flat_vertices(
    weights: &[Vec<(String, f32)>],
    vertices: &[f32],
    bone_index: &dyn Fn(&str) -> Option<usize>,
    slot: &str,
    bone_space: &BoneSpace,
) -> Vec<f32> {
    if weights.is_empty() {
        return vertices.to_vec();
    }

    let mut out = Vec::new();
    for (i, per_vertex) in weights.iter().enumerate() {
        let (x, y) = (
            vertices.get(i * 2).copied().unwrap_or(0.0),
            vertices.get(i * 2 + 1).copied().unwrap_or(0.0),
        );
        // A weight naming a bone that does not exist is dropped rather than
        // written as index -1: a negative index is an out-of-bounds read in
        // every consumer, and a missing influence merely shifts one vertex.
        let resolved: Vec<(usize, f32, (f32, f32))> = per_vertex
            .iter()
            .filter_map(|(bone, weight)| {
                bone_index(bone).map(|i| (i, *weight, bone_space(slot, bone, x, y)))
            })
            .collect();

        if resolved.is_empty() {
            out.push(x);
            out.push(y);
            continue;
        }
        out.push(resolved.len() as f32);
        for (index, weight, (bx, by)) in resolved {
            out.push(index as f32);
            // In the bone's own space, not the mesh's — see `packed_weights`.
            out.push(bx);
            out.push(by);
            out.push(weight);
        }
    }
    out
}

fn sequence(s: &schema::Sequence) -> Value {
    json!({
        "frames": s.frames,
        "fps": s.fps,
        "mode": s.mode,
        "setup_index": s.setup_index,
    })
}

fn constraint(c: &schema::Constraint) -> Value {
    let mut map = Map::new();
    map.insert("name".into(), json!(c.name));
    map.insert("type".into(), json!(c.kind));
    map.insert("target".into(), json!(c.target));
    map.insert("bones".into(), json!(c.bones));
    map.insert("mix".into(), json!(c.mix));

    // Kind-specific blocks are present only for their own kind, so a template
    // that walks `constraints` can branch on `type` and trust what it finds.
    match c.kind.as_str() {
        "ik" => {
            map.insert("bend_direction".into(), json!(c.bend_direction));
            map.insert("softness".into(), json!(c.softness));
            map.insert("stretch".into(), json!(c.stretch));
            map.insert("stretch_limit".into(), json!(c.stretch_limit));
            map.insert("stiffness".into(), json!(c.stiffness));
        }
        "transform" => {
            if let Some(m) = c.mixes {
                map.insert(
                    "mixes".into(),
                    json!({ "rotate": m[0], "translate": m[1], "scale": m[2], "shear": m[3] }),
                );
                // Which channels the constraint actually drives.
                //
                // A mix of 0 contributes nothing, so these say what the
                // constraint *does*. Several formats — Spine among them — name
                // the driven channels separately from the mix amounts, and
                // declaring one the artist left at 0 switches on a channel the
                // rig never had: a transform constraint that suddenly copies its
                // target's scale and shear stretches every bone it governs. This
                // exporter declared all four unconditionally and did exactly
                // that.
                //
                // A template cannot build a conditional object from four floats,
                // so the decision ships as data.
                map.insert(
                    "drives".into(),
                    json!({
                        "rotate": m[0] != 0.0,
                        "translate": m[1] != 0.0,
                        "scale": m[2] != 0.0,
                        "shear": m[3] != 0.0,
                        "any": m.iter().any(|v| *v != 0.0),
                    }),
                );
            }
            if let Some(o) = c.offsets {
                map.insert(
                    "offsets".into(),
                    json!({
                        "x": o[0], "y": o[1], "rotation": o[2],
                        "scale_x": o[3], "scale_y": o[4],
                        "shear_x": o[5], "shear_y": o[6],
                    }),
                );
            }
            map.insert("local".into(), json!(c.local));
            map.insert("relative".into(), json!(c.relative));
        }
        "physics" => {
            if let Some(p) = c.physics {
                map.insert(
                    "physics".into(),
                    json!({ "inertia": p[0], "strength": p[1], "damping": p[2], "mass": p[3] }),
                );
            }
            if let Some(f) = c.forces {
                map.insert(
                    "forces".into(),
                    json!({ "wind_x": f[0], "wind_y": f[1], "gravity_x": f[2], "gravity_y": f[3] }),
                );
            }
            if let Some(ch) = c.channels {
                map.insert(
                    "channels".into(),
                    json!({ "rotate": ch[0], "translate": ch[1] }),
                );
            }
        }
        "path" => {
            if let Some(slot) = &c.slot {
                map.insert("slot".into(), json!(slot));
            }
            if let Some(p) = c.path {
                map.insert(
                    "path".into(),
                    json!({
                        "position": p[0], "spacing": p[1],
                        "mix_rotate": p[2], "mix_translate": p[3],
                    }),
                );
            }
        }
        _ => {}
    }
    Value::Object(map)
}

fn animation(anim: &schema::Animation, project: &Project) -> Value {
    let offset_of = |bone: &str| {
        anim.bone_offsets
            .iter()
            .find(|o| o.bone == bone)
            .map(|o| o.offset)
            .unwrap_or(0.0)
    };

    let mut bones: Vec<Value> = Vec::new();
    let mut slots: Vec<Value> = Vec::new();
    let mut deform: Vec<Value> = Vec::new();
    let mut draw_order: Vec<Value> = Vec::new();
    let mut ik: Vec<Value> = Vec::new();
    let mut transform: Vec<Value> = Vec::new();

    // Grouped by target rather than left as a flat timeline list: every runtime
    // format writes "this bone, these channels", and regrouping a flat list is
    // beyond a logic-less template.
    let mut bone_entry = |name: &str, channel: &str, keys: Value| {
        let shift = offset_of(name);
        if let Some(existing) = bones
            .iter_mut()
            .find(|b| b["name"].as_str() == Some(name))
            .and_then(|b| b.as_object_mut())
        {
            existing.insert(channel.into(), keys);
            return;
        }
        let mut map = Map::new();
        map.insert("name".into(), json!(name));
        map.insert("offset".into(), json!(shift));
        map.insert(channel.into(), keys);
        bones.push(Value::Object(map));
    };

    for timeline in &anim.timelines {
        match timeline {
            schema::Timeline::BoneTranslate { bone, keys } => {
                bone_entry(bone, "translate", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneRotate { bone, keys } => {
                bone_entry(bone, "rotate", scalar_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneScale { bone, keys } => {
                bone_entry(bone, "scale", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::BoneShear { bone, keys } => {
                bone_entry(bone, "shear", vec2_keys(keys, offset_of(bone)))
            }
            schema::Timeline::SlotColor { slot, keys } => slots.push(json!({
                "name": slot, "channel": "color",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "color": k.value, "curve": interp(&k.interp),
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::SlotVisible { slot, keys } => slots.push(json!({
                "name": slot, "channel": "visible",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "value": k.value, "curve": "stepped",
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::SlotAttachment { slot, keys } => slots.push(json!({
                "name": slot, "channel": "attachment",
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "name": k.value, "curve": "stepped",
                })).collect::<Vec<_>>(),
            })),
            schema::Timeline::DrawOrder { keys } => {
                draw_order = keys
                    .iter()
                    .map(|k| {
                        json!({
                            "time": k.time,
                            "offsets": k.offsets.iter().map(|(slot, off)| json!({
                                "slot": slot, "offset": off,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect()
            }
            schema::Timeline::IkMix { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "mix",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::IkBendDirection { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "bend_direction",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::IkSoftness { constraint, keys } => ik.push(json!({
                "constraint": constraint, "channel": "softness",
                "keys": scalar_keys(keys, 0.0),
            })),
            schema::Timeline::TransformConstraintMix { constraint, keys } => {
                transform.push(json!({
                    "constraint": constraint,
                    "keys": keys.iter().map(|k| json!({
                        "time": k.time,
                        "rotate": k.value[0], "translate": k.value[1],
                        "scale": k.value[2], "shear": k.value[3],
                        "curve": interp(&k.interp),
                    })).collect::<Vec<_>>(),
                }))
            }
            schema::Timeline::Deform {
                slot,
                attachment,
                keys,
            } => deform.push(json!({
                "slot": slot, "attachment": attachment,
                "keys": keys.iter().map(|k| json!({
                    "time": k.time, "offsets": k.offsets, "curve": interp(&k.interp),
                })).collect::<Vec<_>>(),
            })),
        }
    }

    json!({
        "name": anim.name,
        "duration": anim.duration,
        "looping": anim.looping,
        "bones": bones,
        // `ik` above is one entry per *channel*, which is the shape a format
        // with separate mix/softness tracks wants. Most published formats
        // instead write one key list per constraint with the channels as fields
        // of each key, and merging by constraint is beyond a template — so both
        // views ship. See `ik_by_constraint` in `docs/export-context.md`.
        "ik_by_constraint": ik_by_constraint(&ik, project),
        "slots": slots,
        "deform": deform,
        "draw_order": draw_order,
        "ik": ik,
        "transform": transform,
        "events": anim.events.iter().map(|e| json!({
            "time": e.time,
            "name": e.name,
            "int": e.int_value,
            "float": e.float_value,
            "string": e.string_value,
            "audio": e.audio,
            "volume": e.volume,
            "balance": e.balance,
        })).collect::<Vec<_>>(),
        // `markers` is absent by design: it is editor furniture (schema::Marker
        // says so) and exporting it would invite presets to write notes into
        // runtime files.
    })
}

/// The per-channel IK timelines regrouped as one key list per constraint.
///
/// Keys from different channels at the same time merge into one key carrying
/// every channel's value; a channel with no key at that time is simply absent,
/// which is what a format that omits unchanged fields wants. Times are collected
/// across all channels and sorted, so the result is stable regardless of which
/// channel was authored first.
fn ik_by_constraint(per_channel: &[Value], project: &Project) -> Vec<Value> {
    // Sorted, not authoring-ordered: `dedup` only collapses *adjacent* repeats,
    // and one constraint's channels are not adjacent in the timeline list.
    let mut names: Vec<&str> = per_channel
        .iter()
        .filter_map(|t| t["constraint"].as_str())
        .collect();
    names.sort_unstable();
    names.dedup();

    let mut out = Vec::new();
    for name in names {
        let tracks: Vec<&Value> = per_channel
            .iter()
            .filter(|t| t["constraint"].as_str() == Some(name))
            .collect();

        // Falls back to 1.0 for a constraint that no longer exists: an
        // unresolved name is a load-time report, not an export failure.
        let setup_bend = project
            .constraints
            .iter()
            .find(|c| c.name == name)
            .map_or(1.0, |c| c.bend_direction);

        // f32 has no Ord, so times are gathered as bits-comparable strings of
        // their own value; sorting by the float itself via partial_cmp is fine
        // here because a key time is never NaN.
        let mut times: Vec<f64> = tracks
            .iter()
            .filter_map(|t| t["keys"].as_array())
            .flatten()
            .filter_map(|k| k["time"].as_f64())
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).expect("key times are never NaN"));
        times.dedup();

        let keys: Vec<Value> = times
            .iter()
            .enumerate()
            .map(|(i, &time)| {
                let mut key = Map::new();
                key.insert("time".into(), json!(time));
                // Against the *merged* list, not the source channel's: a key can
                // be last for `softness` and not for `mix`, and what a consumer
                // reads is this list.
                key.insert("has_next".into(), json!(i + 1 < times.len()));
                // The constraint's own bend direction, repeated on every key.
                //
                // Ankhimate has no bend-direction *timeline* — it is a property
                // of the constraint. Several formats read it per key instead,
                // and default it to "positive" when a key omits it: a rig whose
                // knees bend backwards then straightens them for the length of
                // the animation while its setup pose stays correct. Carrying the
                // setup value on each key is what makes the two agree.
                key.insert("bend_direction".into(), json!(setup_bend));
                // Each channel keeps its **own** control points. An earlier
                // version let the last channel written win and put its points in
                // a single `curve`: a softness ramp's numbers then described a
                // key whose value was `mix`, and Spine rejected the file with
                // "Invalid curve". A curve belongs to the channel it was
                // authored on, and nothing else can be assumed.
                let mut curve = Value::Null;
                let mut any_bezier = false;
                for track in &tracks {
                    let Some(channel) = track["channel"].as_str() else {
                        continue;
                    };
                    let found = track["keys"]
                        .as_array()
                        .and_then(|ks| ks.iter().find(|k| k["time"].as_f64() == Some(time)));
                    if let Some(k) = found {
                        key.insert(channel.into(), k["value"].clone());
                        key.insert(format!("{channel}_points"), k["points"].clone());
                        key.insert(format!("{channel}_line"), k["line_points"].clone());
                        any_bezier |= k["is_bezier"].as_bool().unwrap_or(false);
                        // `curve` stays a plain string for the linear/stepped
                        // cases every template branches on. It is only ever the
                        // *kind*; the numbers live per channel.
                        if k["curve"].is_string() {
                            curve = k["curve"].clone();
                        }
                    } else {
                        key.insert(format!("{channel}_points"), Value::Null);
                        key.insert(format!("{channel}_line"), Value::Null);
                    }
                }
                key.insert("curve".into(), curve);

                // A format writing one curve for the whole key wants the
                // channels concatenated in a fixed order — `mix` then
                // `softness`, matching how a two-axis bone channel writes x then
                // y. Every channel contributes a pair whenever *any* of them is
                // a bezier: a short array is read positionally and misassigns
                // every number after the gap, which is the "Invalid curve" this
                // whole path exists to avoid. Channels that are merely linear
                // contribute their straight line.
                let joined: Option<Vec<Value>> = if any_bezier {
                    ["mix_line", "softness_line"]
                        .iter()
                        .map(|k| key.get(*k).and_then(|v| v.as_array()).cloned())
                        .collect::<Option<Vec<_>>>()
                        .map(|parts| parts.into_iter().flatten().collect())
                } else {
                    None
                };
                key.insert("points".into(), json!(joined));

                Value::Object(key)
            })
            .collect();

        out.push(json!({ "constraint": name, "keys": keys }));
    }
    out
}

/// Per-bone track offsets (T-905) are folded into key times here.
///
/// No runtime format has a concept for them, so leaving them for a template to
/// apply guarantees every preset gets it wrong. Bake, do not export — the
/// authored offset still reaches the game, as shifted keys.
fn shift(time: f32, offset: f32) -> f32 {
    time + offset
}

fn scalar_keys(keys: &[schema::ScalarKey], offset: f32) -> Value {
    json!(
        keys.iter()
            .enumerate()
            .map(|(i, k)| {
                let time = shift(k.time, offset);
                let next = keys.get(i + 1);
                // The easing of the segment *leaving* this key.
                //
                // Ankhimate stores a key's `interp` as how it is arrived at, so
                // the segment from `i` to `i+1` is described by `keys[i + 1]`.
                // Formats that hang the curve on the key starting the segment —
                // Spine among them — want that one, not this key's own.
                //
                // Reading `k.interp` here shifted every curve one key late and
                // left the first key linear: keyframe poses matched exactly and
                // everything between them drifted, which looks like a subtly
                // wrong animation rather than an off-by-one.
                let leaving = next.map(|n| &n.interp);
                json!({
                    "time": time,
                    "value": k.value,
                    "curve": leaving.map_or_else(|| json!("linear"), interp),
                    "has_next": next.is_some(),
                    "points": next.map(|n| control_points(
                        &n.interp,
                        (time, k.value),
                        (shift(n.time, offset), n.value),
                    )),
                    // Padded to a straight line when this segment is not a
                    // bezier, for a consumer concatenating several channels into
                    // one curve array. See `joined_points`.
                    "line_points": next.map(|n| joined_points(
                        &n.interp,
                        (time, k.value),
                        (shift(n.time, offset), n.value),
                    )),
                    "is_bezier": leaving
                        .is_some_and(|i| matches!(i, schema::Interp::Bezier { .. })),
                })
            })
            .collect::<Vec<_>>()
    )
}

fn vec2_keys(keys: &[schema::Vec2Key], offset: f32) -> Value {
    json!(
        keys.iter()
            .enumerate()
            .map(|(i, k)| {
                let time = shift(k.time, offset);
                let next = keys.get(i + 1);
                // Two axes, so two control-point pairs: x's first, then y's.
                // The easing of the segment *leaving* this key lives on the next
                // key — see `scalar_keys`.
                let points = next.map(|n| {
                    let nt = shift(n.time, offset);
                    let mut p = control_points(&n.interp, (time, k.x), (nt, n.x));
                    p.extend(control_points(&n.interp, (time, k.y), (nt, n.y)));
                    p
                });
                json!({
                    "time": time,
                    "x": k.x,
                    "y": k.y,
                    "curve": next.map_or_else(|| json!("linear"), |n| interp(&n.interp)),
                    "has_next": next.is_some(),
                    "points": points,
                })
            })
            .collect::<Vec<_>>()
    )
}

/// Whether a key has another key after it on the same channel — `has_next`.
///
/// A curve describes the interpolation *towards the next key*, so the last key
/// of a channel has nothing to describe. Formats that read a curve then fetch
/// the following frame crash on one written there; Spine reports
/// `[error] Invalid curve` followed by a null-frame NPE. `points` is already
/// absent on a last key, but `curve` is per-key in the schema regardless of
/// position, so a template branching on `"stepped"` needs this to know when to
/// stay silent. Guard every curve with `{{#if has_next}}`.
///
/// A bezier key's two control points in **absolute** time/value space.
///
/// Ankhimate stores handles normalized 0..1 across the span to the next key
/// (`schema::Interp::Bezier`); several published formats store the same curve as
/// four absolute numbers. Converting needs the *next* key, and a template cannot
/// look ahead in an `{{#each}}` — so a preset that printed the normalized
/// handles into an absolute slot produced a file that parses and animates
/// wrongly. This is why the conversion lives here.
///
/// Empty for linear and stepped keys, which carry no control points.
fn control_points(interp: &schema::Interp, from: (f32, f32), to: (f32, f32)) -> Vec<f32> {
    let schema::Interp::Bezier { handles } = interp else {
        return Vec::new();
    };
    bezier_points(handles, from, to)
}

/// [`control_points`], but a non-bezier key yields the control points of the
/// straight line instead of nothing.
///
/// For a format that writes **one curve array covering several channels** — an
/// IK key's `mix` and `softness`, a bone's x and y. If one channel is a bezier
/// and another linear, the array still needs both pairs: a short array is read
/// positionally and misassigns every number after the gap. Handles at thirds
/// reproduce a straight line exactly, so a linear channel padded this way
/// interpolates identically to one with no curve at all.
fn joined_points(interp: &schema::Interp, from: (f32, f32), to: (f32, f32)) -> Vec<f32> {
    let handles = match interp {
        schema::Interp::Bezier { handles } => *handles,
        _ => [1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0],
    };
    bezier_points(&handles, from, to)
}

fn bezier_points(handles: &[f32; 4], from: (f32, f32), to: (f32, f32)) -> Vec<f32> {
    let (t0, v0) = from;
    let (t1, v1) = to;
    let (dt, dv) = (t1 - t0, v1 - v0);
    vec![
        t0 + handles[0] * dt,
        v0 + handles[1] * dv,
        t0 + handles[2] * dt,
        v0 + handles[3] * dv,
    ]
}

/// A key's interpolation, as a value a template can both print and branch on.
fn interp(i: &schema::Interp) -> Value {
    match i {
        schema::Interp::Linear => json!("linear"),
        schema::Interp::Stepped => json!("stepped"),
        // An object where the simple cases are strings would force every
        // template to type-check before printing. Bezier keeps the string in
        // `curve` and puts its handles alongside, so `{{curve}}` always prints.
        schema::Interp::Bezier { handles } => json!({
            "type": "bezier",
            "handles": handles,
            "out_x": handles[0], "out_y": handles[1],
            "in_x": handles[2], "in_y": handles[3],
        }),
    }
}

fn atlas_value(atlas: &Atlas, stem: &str, art_scale: f32) -> Value {
    json!({
        // How much larger the rig's coordinates are than the source art.
        //
        // A rig authored against half-resolution images records attachments at
        // twice their pixel size; several atlas formats carry exactly this as a
        // header so a consumer can scale the regions back up. 1.0 when the art
        // is at the rig's own scale, which is the usual case.
        "art_scale": art_scale,
        "pages": atlas.pages.iter().map(|p| json!({
            "index": p.index,
            "width": p.width,
            "height": p.height,
            "file": crate::atlas::page_filename(stem, p.index),
        })).collect::<Vec<_>>(),
        "regions": atlas.regions.iter().map(|r| json!({
            "name": r.name,
            "page": r.page,
            "x": r.x,
            "y": r.y,
            "width": r.width,
            "height": r.height,
            "offset_x": r.offset_x,
            "offset_y": r.offset_y,
            "original_width": r.original_width,
            "original_height": r.original_height,
            "rotated": r.rotated,
        })).collect::<Vec<_>>(),
    })
}
