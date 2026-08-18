//! Vertex weight painting as undoable commands (T-403).
//!
//! A stroke is one undo step: `PaintWeights` snapshots the whole weight table on
//! its first apply and merges every later frame of the same stroke into itself.
//! Weights are small (a few influences per vertex) and a stroke is one gesture,
//! so the snapshot is cheaper than reconstructing per-vertex deltas — and it
//! cannot drift out of sync with normalization the way a delta would.

use super::EditCommand;
use crate::WorkMode;
use crate::doc::Document;
use ankhimate_core::attachment::{Attachment, MeshAttachment, VertexWeight};
use ankhimate_core::ids::{BoneId, SkinId, SlotId};

/// How a brush dab combines with what is already on a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrushMode {
    /// Raise the painted bone's influence toward `strength`, never past it.
    #[default]
    Add,
    /// Lower it toward zero. `strength` is the *rate* here rather than a target,
    /// since "remove down to 0.7" is not a thing anyone wants — without a rate
    /// the brush would erase completely on contact and be unusable for shaping.
    Subtract,
    /// Drive it to `strength` from either side. The only mode that can lower a
    /// weight *and* land on an exact value, so it is what you reach for to say
    /// "this vertex is 40% forearm" rather than nudging until it looks right.
    Replace,
    /// Pull each vertex toward the average of its neighbours — the tool that
    /// turns a blotchy hand-painted falloff into a smooth one.
    Smooth,
}

impl BrushMode {
    pub fn label(self) -> &'static str {
        match self {
            BrushMode::Add => "Add",
            BrushMode::Subtract => "Subtract",
            BrushMode::Replace => "Replace",
            BrushMode::Smooth => "Smooth",
        }
    }

    /// All four, in the order the toolbar lists them.
    pub const ALL: [BrushMode; 4] = [
        BrushMode::Add,
        BrushMode::Subtract,
        BrushMode::Replace,
        BrushMode::Smooth,
    ];
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

/// A whole-mesh weight operation that is not a brush stroke.
///
/// Prune, swap, direct entry and paste all rewrite the table wholesale, and all
/// want their own undo label. Sharing `PaintWeights` would work but would call
/// every one of them "Paint Weights" in the history, which is exactly the thing
/// an undo stack is for telling apart.
pub struct SetWeights {
    skin: SkinId,
    slot: SlotId,
    name: String,
    weights: Vec<Vec<VertexWeight>>,
    before: Option<Vec<Vec<VertexWeight>>>,
    label: &'static str,
}

impl SetWeights {
    pub fn new(
        skin: SkinId,
        slot: SlotId,
        name: impl Into<String>,
        weights: Vec<Vec<VertexWeight>>,
        label: &'static str,
    ) -> Self {
        Self {
            skin,
            slot,
            name: name.into(),
            weights,
            before: None,
            label,
        }
    }
}

impl EditCommand for SetWeights {
    fn apply(&mut self, doc: &mut Document) {
        let capture = self.before.is_none();
        let Some(mesh) = mesh_mut(doc, self.skin, self.slot, &self.name) else {
            return;
        };
        if capture {
            self.before = Some(mesh.weights.clone());
        }
        mesh.weights = self.weights.clone();
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

    /// Never merges: each of these is one deliberate action, so two in a row are
    /// two undo steps.
    fn merge(&mut self, _next: &dyn EditCommand) -> bool {
        false
    }

    fn label(&self) -> &str {
        self.label
    }

    fn requires_mode(&self) -> Option<WorkMode> {
        Some(WorkMode::Setup)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Exchange two bones' influence everywhere on a mesh.
///
/// Weighting the wrong side of a symmetric rig is a mistake that is otherwise
/// only fixable by repainting both limbs.
pub fn swap_bones(weights: &[Vec<VertexWeight>], a: BoneId, b: BoneId) -> Vec<Vec<VertexWeight>> {
    weights
        .iter()
        .map(|vertex| {
            vertex
                .iter()
                .map(|w| VertexWeight {
                    bone: if w.bone == a {
                        b
                    } else if w.bone == b {
                        a
                    } else {
                        w.bone
                    },
                    weight: w.weight,
                })
                .collect()
        })
        .collect()
}

/// Make one mesh's weights match another's, wherever their vertices coincide.
///
/// Two meshes that meet at a seam — a torso and a hip, a sleeve and a forearm —
/// each own their own copy of the vertices along it. Weight them separately and
/// the seam splits open the moment the joint bends, because the two edges are
/// pulled by slightly different mixes of bone.
///
/// The **source** mesh is authority and is never modified. An average would have
/// been the symmetric thing to do and is the wrong tool: it changes the mesh you
/// already got right in order to meet the one you have not, so welding a good
/// torso to a rough sleeve damages the torso.
///
/// Positions are in a **common space**; `epsilon` is how close counts as the same
/// point. Returns the target's new weight table.
pub fn weld_to_source(
    source: (&[glam::Vec2], &[Vec<VertexWeight>]),
    target: (&[glam::Vec2], &[Vec<VertexWeight>]),
    epsilon: f32,
) -> Vec<Vec<VertexWeight>> {
    let (source_positions, source_weights) = source;
    let (target_positions, target_weights) = target;

    let mut out: Vec<Vec<VertexWeight>> = target_weights.to_vec();
    out.resize(target_positions.len(), Vec::new());

    for (index, position) in target_positions.iter().enumerate() {
        // Nearest within epsilon, not merely the first inside it: two source
        // vertices can both be in range at a dense seam, and taking whichever
        // came first in the list would pick by authoring order.
        let nearest = source_positions
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (*p - *position).length()))
            .filter(|(_, d)| *d <= epsilon)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((source_index, _)) = nearest
            && let Some(weights) = source_weights.get(source_index)
        {
            out[index] = weights.clone();
        }
    }
    out
}

/// Drop one bone's influence from a mesh entirely, sharing it out.
pub fn remove_bone(weights: &[Vec<VertexWeight>], bone: BoneId) -> Vec<Vec<VertexWeight>> {
    weights
        .iter()
        .map(|vertex| {
            let mut kept: Vec<VertexWeight> =
                vertex.iter().filter(|w| w.bone != bone).copied().collect();
            // Renormalized, not left short: a vertex that summed to 1 and lost
            // half its influence would otherwise deform toward the origin.
            normalize(&mut kept);
            kept
        })
        .collect()
}

/// Shape of the brush's falloff across its radius.
///
/// `feather` is the fraction of the radius spent on the gradient: 0 is a hard
/// stamp with no edge, 1 fades all the way from the centre. Returns 0 outside
/// the brush.
pub fn falloff(distance: f32, radius: f32, feather: f32) -> f32 {
    if distance > radius {
        return 0.0;
    }
    let t = (distance / radius.max(1e-6)).clamp(0.0, 1.0);
    let feather = feather.clamp(0.0, 1.0);
    if feather <= 1e-6 {
        return 1.0;
    }
    let solid = 1.0 - feather;
    if t <= solid {
        1.0
    } else {
        1.0 - (t - solid) / feather
    }
}

/// Apply one brush dab to a weight table, returning the new table.
///
/// Pure so it can be tested without a document, a camera or a pointer — the
/// falloff maths is the part worth pinning down.
///
/// `distances` is the world-space distance from the brush centre to each vertex,
/// parallel to `mesh.setup_vertices`.
///
/// **`strength` is the weight the brush drives toward, not an amount added.**
/// The earlier version added `strength * falloff` and then renormalized the
/// whole vertex, which scaled the addition back down by its own size: two bones
/// at 0.5/0.5 painted at full strength went 0.667, 0.75, 0.8, 0.833 … — the
/// series n/(n+1), which approaches 1 and never arrives. Fully binding a vertex
/// to one bone was not possible at any strength, with any number of strokes.
/// Now a dab at the centre of a full-strength brush lands on exactly 1.0.
#[allow(clippy::too_many_arguments)]
pub fn brush(
    mesh: &MeshAttachment,
    bone: BoneId,
    mode: BrushMode,
    distances: &[f32],
    radius: f32,
    strength: f32,
    feather: f32,
    locked: &[BoneId],
) -> Vec<Vec<VertexWeight>> {
    let mut weights = mesh.weights.clone();
    weights.resize(mesh.setup_vertices.len(), Vec::new());

    for (index, distance) in distances.iter().enumerate() {
        if index >= weights.len() {
            continue;
        }
        let rate = falloff(*distance, radius, feather);
        if rate <= 0.0 {
            continue;
        }

        let current = weights[index]
            .iter()
            .find(|w| w.bone == bone)
            .map_or(0.0, |w| w.weight);

        // Every mode is "move toward a target at `rate`". Only the target and
        // whether it may move down differ, which keeps the four consistent: a
        // dab at the brush centre always lands exactly on the target.
        let new = match mode {
            // Never lowers: painting Add over an area already stronger than the
            // brush should leave it alone, not drag it down to the strength.
            BrushMode::Add => current + (strength - current).max(0.0) * rate,
            BrushMode::Subtract => current * (1.0 - rate * strength.clamp(0.0, 1.0)),
            BrushMode::Replace => current + (strength - current) * rate,
            BrushMode::Smooth => {
                let target = neighbour_average(mesh, index, bone);
                current + (target - current) * rate * strength
            }
        };
        set_weight(&mut weights[index], bone, new.clamp(0.0, 1.0), locked);
    }
    weights
}

/// Set one bone's influence on a vertex, giving the rest room proportionally.
///
/// This is the rule that makes painting converge. The painted bone lands on the
/// value asked for; whatever is left of the vertex's 100% is shared among the
/// other bones in the ratio they already had. Locked bones keep their weight and
/// are taken off the top, so "hold this bone at 0.3 while I paint the others"
/// works — which is the whole point of a lock.
pub fn set_weight(weights: &mut Vec<VertexWeight>, bone: BoneId, value: f32, locked: &[BoneId]) {
    let value = value.clamp(0.0, 1.0);
    let is_locked = |b: BoneId| locked.contains(&b);

    // Locking the bone being painted means the dab is a no-op, which is what
    // the user asked for by locking it.
    if is_locked(bone) {
        return;
    }

    let locked_total: f32 = weights
        .iter()
        .filter(|w| is_locked(w.bone))
        .map(|w| w.weight)
        .sum();
    // Locks can only reserve what there is; past that the painted bone gets
    // nothing rather than the vertex summing over 1.
    let available = (1.0 - locked_total).max(0.0);
    let value = value.min(available);

    match weights.iter_mut().find(|w| w.bone == bone) {
        Some(entry) => entry.weight = value,
        None if value > 0.0 => weights.push(VertexWeight {
            bone,
            weight: value,
        }),
        None => return,
    }

    // Share what is left among the other unlocked bones, keeping their ratios.
    let rest: f32 = weights
        .iter()
        .filter(|w| w.bone != bone && !is_locked(w.bone))
        .map(|w| w.weight)
        .sum();
    let room = (available - value).max(0.0);
    for w in weights.iter_mut() {
        if w.bone == bone || is_locked(w.bone) {
            continue;
        }
        w.weight = if rest > 1e-6 {
            w.weight / rest * room
        } else {
            // Nothing to scale: the painted bone is alone on this vertex, so it
            // takes everything available rather than leaving the vertex short.
            0.0
        };
    }
    // Alone on the vertex, so it takes everything available rather than leaving
    // the vertex summing to less than 1 — but only if it is being given weight
    // at all. Subtracting the last influence down to zero must leave the vertex
    // unweighted, not snap it back to fully bound.
    if rest <= 1e-6
        && value > 1e-4
        && let Some(entry) = weights.iter_mut().find(|w| w.bone == bone)
    {
        entry.weight = available;
    }

    weights.retain(|w| w.weight > 1e-4);
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
/// Scale runs before the cull, not after. The inputs are not always in the 0..1
/// the name suggests — `auto_weight` hands over raw inverse distances, and a
/// vertex far along a limb legitimately scores 5e-5 against every bone. Culling
/// first threw that vertex's whole influence list away and left it unweighted.
pub fn normalize(weights: &mut Vec<VertexWeight>) {
    let total: f32 = weights.iter().map(|w| w.weight).sum();
    if total > 1e-12 {
        for w in weights.iter_mut() {
            w.weight /= total;
        }
    }
    let before = weights.len();
    weights.retain(|w| w.weight > 1e-4);
    if weights.len() != before {
        let total: f32 = weights.iter().map(|w| w.weight).sum();
        if total > 1e-12 {
            for w in weights.iter_mut() {
                w.weight /= total;
            }
        }
    }
}

/// Default ceiling on influences per vertex.
///
/// Four is what the common runtimes budget for, and past it the returns are
/// slight: a fifth influence at a few percent moves a vertex less than the
/// rounding in the atlas.
pub const DEFAULT_MAX_BONES: usize = 4;
/// Weights below this contribute nothing visible but still cost a matrix
/// multiply per vertex per frame.
pub const DEFAULT_PRUNE_THRESHOLD: f32 = 0.01;

/// Is this vertex influenced by more bones than the rig budgets for?
///
/// A diagnostic, not a rule — nothing stops it happening, and it is only a
/// *problem* against a number the user chose. Runtimes budget a fixed number of
/// influences per vertex, so a mesh that quietly exceeds it exports fine and
/// deforms differently in the game than in the editor. Surfacing it in the
/// viewport is the only way that gets noticed before then.
pub fn over_influenced(weights: &[VertexWeight], limit: usize) -> bool {
    weights.len() > limit
}

/// Drop the weakest influences from a vertex, then renormalize.
///
/// Two knobs because they catch different things: `threshold` removes the dregs
/// a long painting session leaves behind — a bone at 0.003 that nobody meant to
/// touch — while `max_bones` caps the count for vertices that picked up a little
/// of everything and would otherwise deform mushily.
pub fn prune(weights: &mut Vec<VertexWeight>, max_bones: usize, threshold: f32) {
    weights.retain(|w| w.weight >= threshold);
    if weights.len() > max_bones {
        weights.sort_by(|a, b| b.weight.total_cmp(&a.weight));
        weights.truncate(max_bones);
    }
    normalize(weights);
}

/// Bind every vertex to nearby bones, weighted by inverse distance **across the
/// mesh surface**.
///
/// Straight-line distance is the obvious implementation and it is wrong in a
/// specific, visible way: it does not know the mesh has a shape. Two fingers, or
/// a character's two legs, pass within a few pixels of each other while being
/// far apart across the surface — so the left thigh bone picks up the right
/// thigh's vertices, and bending one leg drags the other. Measuring along the
/// mesh's own edges cannot make that mistake, because there is no path between
/// them that does not go all the way up and back down.
///
/// The distances are geodesic; the falloff on top of them is unchanged.
/// `vertices`, when non-empty, restricts the recompute to those indices —
/// everything else keeps the weights it has. Recomputing a whole mesh to fix one
/// badly-bound corner throws away every hand edit on it, which is why the
/// selection has to be honoured rather than treated as a hint.
pub fn auto_weight(
    mesh: &MeshAttachment,
    bones: &[(BoneId, glam::Vec2, glam::Vec2)],
    falloff: f32,
    vertices: &[usize],
) -> Vec<Vec<VertexWeight>> {
    let adjacency = adjacency(mesh);
    let per_bone: Vec<Vec<f32>> = bones
        .iter()
        .map(|(_, start, end)| geodesic(mesh, &adjacency, *start, *end))
        .collect();

    let mut existing = mesh.weights.clone();
    existing.resize(mesh.setup_vertices.len(), Vec::new());

    (0..mesh.setup_vertices.len())
        .map(|index| {
            if !vertices.is_empty() && !vertices.contains(&index) {
                return std::mem::take(&mut existing[index]);
            }
            let mut influences: Vec<VertexWeight> = bones
                .iter()
                .enumerate()
                .filter_map(|(b, (bone, _, _))| {
                    let distance = per_bone[b][index];
                    // Unreachable across the surface: a disconnected island, or
                    // the far side of a gap. No influence at all, rather than a
                    // small one that reintroduces the bug.
                    if !distance.is_finite() {
                        return None;
                    }
                    Some(VertexWeight {
                        bone: *bone,
                        // Inverse distance raised to `falloff`: higher values
                        // make the binding tighter around each bone.
                        weight: 1.0 / distance.max(1e-3).powf(falloff.max(0.1)),
                    })
                })
                .collect();
            prune(&mut influences, DEFAULT_MAX_BONES, 0.0);
            influences
        })
        .collect()
}

/// Which vertices share a triangle edge with which, with the edge lengths.
fn adjacency(mesh: &MeshAttachment) -> Vec<Vec<(usize, f32)>> {
    let mut adjacency = vec![Vec::new(); mesh.setup_vertices.len()];
    let mut push = |a: usize, b: usize| {
        let (Some(pa), Some(pb)) = (
            mesh.setup_vertices.get(a).copied(),
            mesh.setup_vertices.get(b).copied(),
        ) else {
            return;
        };
        let length = (pa - pb).length();
        let list: &mut Vec<(usize, f32)> = &mut adjacency[a];
        if !list.iter().any(|(other, _)| *other == b) {
            list.push((b, length));
        }
    };
    for tri in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
            if a < mesh.setup_vertices.len() && b < mesh.setup_vertices.len() {
                push(a, b);
                push(b, a);
            }
        }
    }
    adjacency
}

/// Distance from a bone to every vertex, measured along the mesh's edges.
///
/// Dijkstra from a seed band rather than from a single nearest vertex: a bone is
/// a segment, and collapsing it to a point would make a long bone's far end
/// artificially distant from the art beside it. The band is every vertex within
/// a quarter of the bone's length of the closest one, which covers the vertices
/// genuinely lying along the bone without reaching across a gap to the next limb.
fn geodesic(
    mesh: &MeshAttachment,
    adjacency: &[Vec<(usize, f32)>],
    start: glam::Vec2,
    end: glam::Vec2,
) -> Vec<f32> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    let count = mesh.setup_vertices.len();
    let straight: Vec<f32> = mesh
        .setup_vertices
        .iter()
        .map(|v| distance_to_segment(*v, start, end))
        .collect();
    let mut dist = vec![f32::INFINITY; count];
    let Some(nearest) = straight.iter().copied().fold(None, |acc: Option<f32>, d| {
        Some(acc.map_or(d, |a| a.min(d)))
    }) else {
        return dist;
    };

    let band = nearest + (end - start).length() * 0.25;
    // f32 is not Ord, so the heap carries fixed-point keys. Micrometres of a
    // texel: far finer than any distance that matters here.
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
    for (index, d) in straight.iter().enumerate() {
        if *d <= band {
            dist[index] = *d;
            heap.push(Reverse(((*d * 1000.0) as u64, index)));
        }
    }

    while let Some(Reverse((key, index))) = heap.pop() {
        // A stale entry from before this vertex was relaxed lower.
        if (key as f32) / 1000.0 > dist[index] + 1e-3 {
            continue;
        }
        for (next, length) in &adjacency[index] {
            let candidate = dist[index] + length;
            if candidate < dist[*next] {
                dist[*next] = candidate;
                heap.push(Reverse(((candidate * 1000.0) as u64, *next)));
            }
        }
    }
    dist
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

    /// Paint one bone at `strength` over the vertex at the brush centre.
    fn dab(
        mesh: &MeshAttachment,
        bone: BoneId,
        mode: BrushMode,
        strength: f32,
    ) -> Vec<Vec<VertexWeight>> {
        brush(
            mesh,
            bone,
            mode,
            &[0.0, 999.0, 999.0, 999.0],
            50.0,
            strength,
            1.0,
            &[],
        )
    }

    #[test]
    fn a_dab_falls_off_with_distance_and_normalizes() {
        let mesh = quad();
        let bone = bone_id(1);
        // Vertex 0 at the centre of the brush, vertex 1 at the edge.
        let distances = vec![0.0, 40.0, 999.0, 999.0];
        let weights = brush(&mesh, bone, BrushMode::Add, &distances, 50.0, 1.0, 1.0, &[]);

        assert_eq!(weights[0].len(), 1);
        assert!(
            (weights[0][0].weight - 1.0).abs() < 1e-4,
            "a single influence normalizes to 1"
        );
        assert!(!weights[1].is_empty(), "the edge vertex got some weight");
        assert!(weights[2].is_empty(), "out of range, untouched");
    }

    /// The bug this system was rebuilt around.
    ///
    /// Adding `strength * falloff` and renormalizing the whole vertex scaled the
    /// addition back down by its own size, so two bones at 0.5/0.5 went 0.667,
    /// 0.75, 0.8, 0.833 … — n/(n+1), which approaches 1 and never arrives. No
    /// number of strokes at any strength could fully bind a vertex to one bone.
    #[test]
    fn one_full_strength_dab_fully_binds_a_vertex() {
        let mut mesh = quad();
        let (a, b) = (bone_id(1), bone_id(2));
        mesh.weights = vec![
            vec![
                VertexWeight {
                    bone: a,
                    weight: 0.5,
                },
                VertexWeight {
                    bone: b,
                    weight: 0.5,
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];

        let painted = dab(&mesh, a, BrushMode::Add, 1.0);
        let wa = painted[0]
            .iter()
            .find(|w| w.bone == a)
            .map_or(0.0, |w| w.weight);
        assert!(
            (wa - 1.0).abs() < 1e-4,
            "one dab at full strength must reach 1.0, got {wa}"
        );
        assert!(
            painted[0].iter().all(|w| w.bone == a),
            "the other bone gave way entirely"
        );
    }

    /// Strength is the weight the brush drives *toward*, so a half-strength
    /// brush parks on 0.5 however long it is held there.
    #[test]
    fn strength_is_a_target_not_an_increment() {
        let mut mesh = quad();
        let bone = bone_id(1);
        for _ in 0..10 {
            mesh.weights = dab(&mesh, bone, BrushMode::Add, 0.5);
        }
        // Alone on the vertex it normalizes to 1 — with nothing to share with,
        // 0.5 of one bone *is* all of the influence.
        assert_eq!(mesh.weights[0].len(), 1);

        let other = bone_id(2);
        mesh.weights = dab(&mesh, other, BrushMode::Add, 1.0);
        for _ in 0..10 {
            mesh.weights = dab(&mesh, bone, BrushMode::Add, 0.5);
        }
        let w = mesh.weights[0]
            .iter()
            .find(|w| w.bone == bone)
            .map_or(0.0, |w| w.weight);
        assert!(
            (w - 0.5).abs() < 1e-3,
            "ten dabs at strength 0.5 should sit on 0.5, got {w}"
        );
    }

    #[test]
    fn add_never_lowers_a_stronger_weight() {
        let mut mesh = quad();
        let bone = bone_id(1);
        mesh.weights = dab(&mesh, bone, BrushMode::Add, 1.0);
        let after = dab(&mesh, bone, BrushMode::Add, 0.3);
        let w = after[0]
            .iter()
            .find(|w| w.bone == bone)
            .map_or(0.0, |w| w.weight);
        assert!((w - 1.0).abs() < 1e-4, "Add dragged a weight down to {w}");
    }

    /// Replace is the only mode that can land on an exact value from either
    /// side — the reason it exists.
    #[test]
    fn replace_lowers_as_well_as_raises() {
        let mut mesh = quad();
        let (a, b) = (bone_id(1), bone_id(2));
        mesh.weights = dab(&mesh, a, BrushMode::Add, 1.0);
        mesh.weights = dab(&mesh, b, BrushMode::Replace, 0.25);

        let wa = mesh.weights[0].iter().find(|w| w.bone == a).unwrap().weight;
        let wb = mesh.weights[0].iter().find(|w| w.bone == b).unwrap().weight;
        assert!(
            (wb - 0.25).abs() < 1e-4,
            "b should be exactly 0.25, got {wb}"
        );
        assert!(
            (wa - 0.75).abs() < 1e-4,
            "a should give way to 0.75, got {wa}"
        );
    }

    #[test]
    fn subtract_removes_influence_and_prunes_it() {
        let mut mesh = quad();
        let bone = bone_id(1);
        mesh.weights = dab(&mesh, bone, BrushMode::Add, 1.0);
        let erased = dab(&mesh, bone, BrushMode::Subtract, 1.0);
        assert!(
            erased[0].is_empty(),
            "a zeroed influence is dropped, not kept at 0: {:?}",
            erased[0]
        );
    }

    #[test]
    fn a_locked_bone_keeps_its_weight_while_others_are_painted() {
        let mut mesh = quad();
        let (a, b, c) = (bone_id(1), bone_id(2), bone_id(3));
        mesh.weights = vec![
            vec![
                VertexWeight {
                    bone: a,
                    weight: 0.4,
                },
                VertexWeight {
                    bone: b,
                    weight: 0.6,
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        // Paint c hard with a locked at 0.4.
        let painted = brush(
            &mesh,
            c,
            BrushMode::Add,
            &[0.0, 999.0, 999.0, 999.0],
            50.0,
            1.0,
            1.0,
            &[a],
        );
        let wa = painted[0].iter().find(|w| w.bone == a).unwrap().weight;
        let wc = painted[0].iter().find(|w| w.bone == c).unwrap().weight;
        assert!((wa - 0.4).abs() < 1e-4, "the lock did not hold: {wa}");
        assert!(
            (wc - 0.6).abs() < 1e-4,
            "c should take everything the lock left, got {wc}"
        );
    }

    #[test]
    fn two_bones_share_a_vertex_and_sum_to_one() {
        let mut mesh = quad();
        let (a, b) = (bone_id(1), bone_id(2));
        mesh.weights = dab(&mesh, a, BrushMode::Add, 1.0);
        mesh.weights = dab(&mesh, b, BrushMode::Replace, 0.5);

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
            1.0,
            &[],
        );
        assert!(
            !smoothed[1].is_empty(),
            "the neighbour's weight bled into it"
        );
    }

    #[test]
    fn feather_shapes_the_falloff() {
        // No feather: a hard stamp, full strength right to the rim.
        assert!((falloff(49.0, 50.0, 0.0) - 1.0).abs() < 1e-4);
        assert_eq!(falloff(51.0, 50.0, 0.0), 0.0, "still nothing outside");

        // Full feather: linear from the centre, the old fixed behaviour.
        assert!((falloff(25.0, 50.0, 1.0) - 0.5).abs() < 1e-4);

        // Half: solid to the halfway mark, then a ramp.
        assert!((falloff(24.0, 50.0, 0.5) - 1.0).abs() < 1e-4);
        assert!((falloff(37.5, 50.0, 0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn prune_caps_the_count_and_drops_the_dregs() {
        let mut weights = vec![
            VertexWeight {
                bone: bone_id(1),
                weight: 0.50,
            },
            VertexWeight {
                bone: bone_id(2),
                weight: 0.30,
            },
            VertexWeight {
                bone: bone_id(3),
                weight: 0.15,
            },
            VertexWeight {
                bone: bone_id(4),
                weight: 0.04,
            },
            VertexWeight {
                bone: bone_id(5),
                weight: 0.01,
            },
        ];
        prune(&mut weights, 3, 0.02);

        assert_eq!(weights.len(), 3, "capped at three");
        assert!(
            !weights.iter().any(|w| w.bone == bone_id(5)),
            "the sub-threshold influence went"
        );
        let total: f32 = weights.iter().map(|w| w.weight).sum();
        assert!((total - 1.0).abs() < 1e-4, "renormalized after pruning");
    }

    #[test]
    fn welding_copies_the_source_onto_coincident_vertices() {
        let (a, b) = (bone_id(1), bone_id(2));
        // Source seam at the origin, fully bound to a.
        let source_pos = vec![glam::vec2(0.0, 0.0), glam::vec2(10.0, 0.0)];
        let source_w = vec![
            vec![VertexWeight {
                bone: a,
                weight: 1.0,
            }],
            Vec::new(),
        ];
        // The target's seam vertex is a hair off, which is what an authored
        // seam actually looks like.
        let target_pos = vec![glam::vec2(0.001, 0.0), glam::vec2(-10.0, 0.0)];
        let target_w = vec![
            vec![VertexWeight {
                bone: b,
                weight: 1.0,
            }],
            Vec::new(),
        ];

        let welded = weld_to_source((&source_pos, &source_w), (&target_pos, &target_w), 0.01);

        assert_eq!(welded[0].len(), 1, "the seam vertex took one influence");
        assert_eq!(
            welded[0][0].bone, a,
            "it took the source's bone, not its own"
        );
        assert!((welded[0][0].weight - 1.0).abs() < 1e-6);
        // The far vertex matched nothing, so it keeps what it had.
        assert!(welded[1].is_empty());
    }

    /// The source is authority and must come back untouched — an average would
    /// damage the mesh you already got right in order to meet the rough one.
    #[test]
    fn welding_leaves_vertices_that_match_nothing_alone() {
        let a = bone_id(1);
        let source_pos = vec![glam::vec2(0.0, 0.0)];
        let source_w = vec![vec![VertexWeight {
            bone: a,
            weight: 1.0,
        }]];
        let target_pos = vec![glam::vec2(50.0, 0.0)];
        let target_w = vec![vec![VertexWeight {
            bone: bone_id(9),
            weight: 1.0,
        }]];

        let welded = weld_to_source((&source_pos, &source_w), (&target_pos, &target_w), 0.01);
        assert_eq!(welded[0].len(), 1);
        assert_eq!(
            welded[0][0].bone,
            bone_id(9),
            "an unmatched vertex kept its own"
        );
    }

    #[test]
    fn swapping_two_bones_exchanges_their_influence() {
        let (a, b) = (bone_id(1), bone_id(2));
        let weights = vec![vec![
            VertexWeight {
                bone: a,
                weight: 0.7,
            },
            VertexWeight {
                bone: b,
                weight: 0.3,
            },
        ]];
        let swapped = swap_bones(&weights, a, b);
        assert_eq!(swapped[0].iter().find(|w| w.bone == a).unwrap().weight, 0.3);
        assert_eq!(swapped[0].iter().find(|w| w.bone == b).unwrap().weight, 0.7);
    }

    #[test]
    fn removing_a_bone_shares_its_weight_out() {
        let (a, b) = (bone_id(1), bone_id(2));
        let weights = vec![vec![
            VertexWeight {
                bone: a,
                weight: 0.5,
            },
            VertexWeight {
                bone: b,
                weight: 0.5,
            },
        ]];
        let after = remove_bone(&weights, a);
        assert_eq!(after[0].len(), 1);
        assert!(
            (after[0][0].weight - 1.0).abs() < 1e-4,
            "the survivor takes the whole vertex"
        );
    }

    /// Two limbs that pass close together but are only joined at the top — the
    /// shape that breaks straight-line auto-weighting.
    ///
    /// ```text
    ///   0───1        vertices 0,1 are the waist
    ///   │   │        2,4 run down the left leg
    ///   2   3        3,5 run down the right
    ///   │   │        the legs are 2 units apart, but 12 apart across the mesh
    ///   4   5
    /// ```
    fn two_legs() -> MeshAttachment {
        let mut mesh = quad();
        mesh.setup_vertices = vec![
            glam::vec2(-1.0, 10.0),
            glam::vec2(1.0, 10.0),
            glam::vec2(-1.0, 5.0),
            glam::vec2(1.0, 5.0),
            glam::vec2(-1.0, 0.0),
            glam::vec2(1.0, 0.0),
        ];
        // Joined across the waist only; no triangle spans the gap lower down.
        mesh.triangles = vec![[0, 1, 2], [1, 3, 2], [2, 4, 2], [3, 5, 3]];
        mesh.weights = Vec::new();
        mesh
    }

    #[test]
    fn auto_weight_does_not_reach_across_a_gap() {
        let mesh = two_legs();
        let (left, right) = (bone_id(1), bone_id(2));
        let bones = vec![
            // A bone down each leg.
            (left, glam::vec2(-1.0, 5.0), glam::vec2(-1.0, 0.0)),
            (right, glam::vec2(1.0, 5.0), glam::vec2(1.0, 0.0)),
        ];
        let weights = auto_weight(&mesh, &bones, 2.0, &[]);

        // Vertex 4 is the bottom of the left leg. It is 2 units from the right
        // leg's bone in a straight line — closer than the top of its own leg —
        // but the only path there runs up over the waist and back down.
        let strongest = weights[4]
            .iter()
            .max_by(|a, b| a.weight.total_cmp(&b.weight))
            .unwrap();
        assert_eq!(
            strongest.bone, left,
            "the far leg's bone captured a vertex through the gap"
        );

        let leaked = weights[4]
            .iter()
            .find(|w| w.bone == right)
            .map_or(0.0, |w| w.weight);
        assert!(
            leaked < 0.2,
            "the far leg still leaked {leaked} into this vertex"
        );
    }

    #[test]
    fn auto_weight_only_touches_the_selected_vertices() {
        let mesh = quad();
        let (left, right) = (bone_id(1), bone_id(2));
        let bones = vec![
            (left, glam::vec2(-50.0, 0.0), glam::vec2(-50.0, 10.0)),
            (right, glam::vec2(50.0, 0.0), glam::vec2(50.0, 10.0)),
        ];

        // Vertex 3 is hand-weighted to something auto-weighting would never
        // produce, then left out of the selection.
        let mut mesh = mesh;
        let sentinel = bone_id(99);
        mesh.weights = vec![
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![VertexWeight {
                bone: sentinel,
                weight: 1.0,
            }],
        ];

        let weights = auto_weight(&mesh, &bones, 2.0, &[0, 1]);
        assert!(!weights[0].is_empty(), "selected vertex was recomputed");
        assert!(
            weights[2].is_empty(),
            "unselected, unweighted vertex untouched"
        );
        assert_eq!(
            weights[3].first().map(|w| w.bone),
            Some(sentinel),
            "recomputing the whole mesh threw away a hand edit"
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
        let weights = auto_weight(&mesh, &bones, 2.0, &[]);

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
