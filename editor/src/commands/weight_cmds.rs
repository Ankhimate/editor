//! Vertex weight painting as undoable commands (T-403).
//!
//! A stroke is one undo step: `PaintWeights` snapshots the whole weight table on
//! its first apply and merges every later frame of the same stroke into itself.
//! Weights are small (a few influences per vertex) and a stroke is one gesture,
//! so the snapshot is cheaper than reconstructing per-vertex deltas — and it
//! cannot drift out of sync with normalization the way a delta would.

use super::EditCommand;
use crate::doc::Document;
use crate::session::WorkMode;
use ankhimate_core::attachment::{Attachment, MeshAttachment, VertexWeight};
use ankhimate_core::ids::{BoneId, SkinId, SlotId};

/// How a brush dab combines with what is already on a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrushMode {
    #[default]
    Add,
    Subtract,
    /// Pull each vertex toward the average of its neighbours — the tool that
    /// turns a blotchy hand-painted falloff into a smooth one.
    Smooth,
}

impl BrushMode {
    pub fn label(self) -> &'static str {
        match self {
            BrushMode::Add => "Add",
            BrushMode::Subtract => "Subtract",
            BrushMode::Smooth => "Smooth",
        }
    }
}

fn mesh_mut<'a>(
    doc: &'a mut Document,
    skin: SkinId,
    slot: SlotId,
    name: &str,
) -> Option<&'a mut MeshAttachment> {
    match doc
        .skeleton
        .skins
        .get_mut(skin)?
        .entries
        .get_mut(&(slot, name.to_string()))?
    {
        Attachment::Mesh(mesh) => Some(mesh),
        _ => None,
    }
}

/// Set the weight table for a mesh — the result of a paint stroke.
pub struct PaintWeights {
    skin: SkinId,
    slot: SlotId,
    name: String,
    weights: Vec<Vec<VertexWeight>>,
    before: Option<Vec<Vec<VertexWeight>>>,
}

impl PaintWeights {
    pub fn new(
        skin: SkinId,
        slot: SlotId,
        name: impl Into<String>,
        weights: Vec<Vec<VertexWeight>>,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            weights,
            before: None,
        }
    }
}

impl EditCommand for PaintWeights {
    fn apply(&mut self, doc: &mut Document) {
        let capture = self.before.is_none();
        let Some(mesh) = mesh_mut(doc, self.skin, self.slot, &self.name) else {
            return;
        };
        if capture {
            self.before = Some(mesh.weights.clone());
        }
        mesh.weights = self.weights.clone();
        // Binds are rebuilt from the setup pose after this lands
        // (`AppState::rebind_meshes`); clearing them here means a stale bind can
        // never outlive the influences it was captured for.
        mesh.inverse_bind_matrices.clear();
    }

    fn revert(&mut self, doc: &mut Document) {
        if let (Some(before), Some(mesh)) = (
            self.before.take(),
            mesh_mut(doc, self.skin, self.slot, &self.name),
        ) {
            mesh.weights = before;
            mesh.inverse_bind_matrices.clear();
        }
    }

    fn merge(&mut self, next: &dyn EditCommand) -> bool {
        match next.as_any().downcast_ref::<PaintWeights>() {
            Some(other)
                if other.skin == self.skin
                    && other.slot == self.slot
                    && other.name == self.name =>
            {
                self.weights = other.weights.clone();
                true
            }
            _ => false,
        }
    }

    fn label(&self) -> &str {
        "Paint Weights"
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Apply one brush dab to a weight table, returning the new table.
///
/// Pure so it can be tested without a document, a camera or a pointer — the
/// falloff maths is the part worth pinning down.
///
/// `distances` is the world-space distance from the brush centre to each vertex,
/// parallel to `mesh.setup_vertices`.
pub fn brush(
    mesh: &MeshAttachment,
    bone: BoneId,
    mode: BrushMode,
    distances: &[f32],
    radius: f32,
    strength: f32,
) -> Vec<Vec<VertexWeight>> {
    let mut weights = mesh.weights.clone();
    weights.resize(mesh.setup_vertices.len(), Vec::new());

    for (index, distance) in distances.iter().enumerate() {
        if index >= weights.len() || *distance > radius {
            continue;
        }
        // Linear falloff: predictable, and the shape a user expects from a
        // radius they can see.
        let falloff = 1.0 - (distance / radius.max(1e-6));
        let amount = strength * falloff;

        match mode {
            BrushMode::Add | BrushMode::Subtract => {
                let signed = if mode == BrushMode::Add {
                    amount
                } else {
                    -amount
                };
                let entry = weights[index].iter_mut().find(|w| w.bone == bone);
                match entry {
                    Some(w) => w.weight = (w.weight + signed).clamp(0.0, 1.0),
                    None if signed > 0.0 => weights[index].push(VertexWeight {
                        bone,
                        weight: signed.clamp(0.0, 1.0),
                    }),
                    None => {}
                }
            }
            BrushMode::Smooth => {
                // Toward the mean of this vertex's neighbours for this bone.
                let target = neighbour_average(mesh, index, bone);
                let entry = weights[index].iter_mut().find(|w| w.bone == bone);
                match entry {
                    Some(w) => w.weight += (target - w.weight) * amount,
                    None if target > 0.0 => weights[index].push(VertexWeight {
                        bone,
                        weight: target * amount,
                    }),
                    None => {}
                }
            }
        }
        normalize(&mut weights[index]);
    }
    weights
}

/// Mean weight of `bone` across the vertices sharing a triangle with `index`.
fn neighbour_average(mesh: &MeshAttachment, index: usize, bone: BoneId) -> f32 {
    let mut total = 0.0;
    let mut count = 0;
    for tri in &mesh.triangles {
        if !tri.contains(&(index as u32)) {
            continue;
        }
        for other in tri {
            let other = *other as usize;
            if other == index {
                continue;
            }
            total += mesh
                .weights
                .get(other)
                .and_then(|w| w.iter().find(|w| w.bone == bone))
                .map(|w| w.weight)
                .unwrap_or(0.0);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// Drop empty influences and rescale the rest to sum to 1.
///
/// An unnormalized vertex is not wrong so much as unpredictable: the skinning
/// divides by the total, so two vertices with the same *relative* weights but
/// different sums deform identically, and the numbers stop meaning anything to
/// the person reading them.
pub fn normalize(weights: &mut Vec<VertexWeight>) {
    weights.retain(|w| w.weight > 1e-4);
    let total: f32 = weights.iter().map(|w| w.weight).sum();
    if total > 1e-6 {
        for w in weights.iter_mut() {
            w.weight /= total;
        }
    }
}

/// Bind every vertex to its nearest bone in `bones`, weighted by inverse
/// distance — a usable starting point that beats painting from nothing.
pub fn auto_weight(
    mesh: &MeshAttachment,
    bones: &[(BoneId, glam::Vec2, glam::Vec2)],
    falloff: f32,
) -> Vec<Vec<VertexWeight>> {
    mesh.setup_vertices
        .iter()
        .map(|vertex| {
            let mut influences: Vec<VertexWeight> = bones
                .iter()
                .map(|(bone, start, end)| {
                    let distance = distance_to_segment(*vertex, *start, *end).max(1e-3);
                    VertexWeight {
                        bone: *bone,
                        // Inverse distance raised to `falloff`: higher values
                        // make the binding tighter around each bone.
                        weight: 1.0 / distance.powf(falloff.max(0.1)),
                    }
                })
                .collect();
            // Keep the strongest few; a vertex influenced by every bone in the
            // rig is both slow and mushy.
            influences.sort_by(|a, b| b.weight.total_cmp(&a.weight));
            influences.truncate(4);
            normalize(&mut influences);
            influences
        })
        .collect()
}

fn distance_to_segment(p: glam::Vec2, a: glam::Vec2, b: glam::Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-9 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::attachment::{Rect, RegionAttachment};
    use ankhimate_core::slotmap::KeyData;

    fn bone_id(n: u64) -> BoneId {
        BoneId::from(KeyData::from_ffi(n))
    }

    fn quad() -> MeshAttachment {
        MeshAttachment::from_region(&RegionAttachment {
            texture: "img".into(),
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
            sequence: None,
        })
    }

    #[test]
    fn a_dab_falls_off_with_distance_and_normalizes() {
        let mesh = quad();
        let bone = bone_id(1);
        // Vertex 0 at the centre of the brush, vertex 1 at the edge.
        let distances = vec![0.0, 40.0, 999.0, 999.0];
        let weights = brush(&mesh, bone, BrushMode::Add, &distances, 50.0, 1.0);

        assert_eq!(weights[0].len(), 1);
        assert!(
            (weights[0][0].weight - 1.0).abs() < 1e-4,
            "a single influence normalizes to 1"
        );
        assert!(!weights[1].is_empty(), "the edge vertex got some weight");
        assert!(weights[2].is_empty(), "out of range, untouched");
    }

    #[test]
    fn subtract_removes_influence_and_prunes_it() {
        let mesh = quad();
        let bone = bone_id(1);
        let painted = brush(
            &mesh,
            bone,
            BrushMode::Add,
            &[0.0, 999.0, 999.0, 999.0],
            50.0,
            1.0,
        );

        let mut mesh = mesh;
        mesh.weights = painted;
        let erased = brush(
            &mesh,
            bone,
            BrushMode::Subtract,
            &[0.0, 999.0, 999.0, 999.0],
            50.0,
            1.0,
        );
        assert!(
            erased[0].is_empty(),
            "a zeroed influence is dropped, not kept at 0: {:?}",
            erased[0]
        );
    }

    #[test]
    fn two_bones_share_a_vertex_and_sum_to_one() {
        let mesh = quad();
        let (a, b) = (bone_id(1), bone_id(2));
        let mut mesh = mesh;
        mesh.weights = brush(&mesh, a, BrushMode::Add, &[0.0; 4], 50.0, 1.0);
        mesh.weights = brush(&mesh, b, BrushMode::Add, &[0.0; 4], 50.0, 0.5);

        let total: f32 = mesh.weights[0].iter().map(|w| w.weight).sum();
        assert!((total - 1.0).abs() < 1e-4, "normalized: {total}");
        assert_eq!(mesh.weights[0].len(), 2, "both bones influence it");
    }

    #[test]
    fn smoothing_pulls_toward_the_neighbours() {
        let mut mesh = quad();
        let bone = bone_id(1);
        // One corner fully bound, the rest at zero.
        mesh.weights = vec![
            vec![VertexWeight { bone, weight: 1.0 }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        // Smooth vertex 1, which shares a triangle with the weighted corner.
        let smoothed = brush(
            &mesh,
            bone,
            BrushMode::Smooth,
            &[999.0, 0.0, 999.0, 999.0],
            50.0,
            1.0,
        );
        assert!(
            !smoothed[1].is_empty(),
            "the neighbour's weight bled into it"
        );
    }

    #[test]
    fn auto_weight_binds_each_vertex_to_its_nearest_bone() {
        let mesh = quad();
        let (left, right) = (bone_id(1), bone_id(2));
        let bones = vec![
            (left, glam::vec2(-50.0, 0.0), glam::vec2(-50.0, 10.0)),
            (right, glam::vec2(50.0, 0.0), glam::vec2(50.0, 10.0)),
        ];
        let weights = auto_weight(&mesh, &bones, 2.0);

        // Vertex 0 is the top-left corner: the left bone must dominate.
        let strongest = weights[0]
            .iter()
            .max_by(|a, b| a.weight.total_cmp(&b.weight))
            .unwrap();
        assert_eq!(strongest.bone, left);
        for vertex in &weights {
            let total: f32 = vertex.iter().map(|w| w.weight).sum();
            assert!((total - 1.0).abs() < 1e-3, "normalized: {total}");
        }
    }
}
