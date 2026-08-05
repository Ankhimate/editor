use crate::ids::BoneId;
use crate::transforms::Affine2;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type TextureRef = String;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionAttachment {
    pub texture: TextureRef,
    /// Where the attachment's **pivot** sits, in the bone's local space.
    pub local_offset: Vec2,
    /// Rotation about the pivot.
    pub local_rotation: f32,
    /// Scale about the pivot.
    pub local_scale: Vec2,
    pub width: f32,
    pub height: f32,
    pub uv_rect: Rect,
    /// The point the image turns and scales around, in normalized image
    /// coordinates: `(0,0)` bottom-left, `(1,1)` top-right, `(0.5,0.5)` centre.
    ///
    /// Without this every sprite can only rotate about its own middle, so a
    /// swinging arm has to be authored as a bone offset instead of a pivot at
    /// the shoulder. Defaulting to the centre keeps pre-pivot files identical.
    #[serde(default = "center_pivot")]
    pub pivot: Vec2,
    /// Frames this attachment cycles through instead of `texture`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
}

fn center_pivot() -> Vec2 {
    Vec2::splat(0.5)
}

impl RegionAttachment {
    /// The quad's corners in the bone's local space, in TL, BL, BR, TR order:
    /// pivot-relative, then scaled, rotated, and moved to `local_offset`.
    ///
    /// Lives in core so the editor viewport, the exporter and the runtime all
    /// derive the same four points — a second copy of this is a second chance to
    /// disagree about what a pivot means.
    pub fn local_corners(&self) -> [Vec2; 4] {
        let size = Vec2::new(self.width, self.height) * self.local_scale;
        // Distances from the pivot to each edge. World Y is up (PLAN §2.2), so
        // `top` is positive.
        let left = -self.pivot.x * size.x;
        let right = (1.0 - self.pivot.x) * size.x;
        let bottom = -self.pivot.y * size.y;
        let top = (1.0 - self.pivot.y) * size.y;

        let (sin, cos) = self.local_rotation.sin_cos();
        let place =
            |v: Vec2| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos) + self.local_offset;
        [
            place(Vec2::new(left, top)),
            place(Vec2::new(left, bottom)),
            place(Vec2::new(right, bottom)),
            place(Vec2::new(right, top)),
        ]
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VertexWeight {
    pub bone: BoneId,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfdKeyframe {
    pub time: f32,
    pub vertex_offsets: Vec<Vec2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MeshAttachment {
    pub texture: TextureRef,
    pub setup_vertices: Vec<Vec2>,
    pub uvs: Vec<Vec2>,
    pub triangles: Vec<[u32; 3]>,
    pub weights: Vec<Vec<VertexWeight>>,
    pub ffd_keyframes: Vec<FfdKeyframe>,

    /// Vertex pairs the triangulation must keep as edges (T-401).
    ///
    /// Delaunay picks the edges that maximise the smallest angle, which is the
    /// right default and the wrong answer for a concave silhouette: it happily
    /// bridges a notch because that triangle is nicely shaped. An edge listed
    /// here is a constraint the retriangulation honours, so a user who knows
    /// where the fold belongs can say so once instead of fighting the solver.
    ///
    /// Defaulted, so a mesh saved before this existed still loads.
    #[serde(default)]
    pub edges: Vec<[u32; 2]>,

    /// Inverse bind affine per influencing bone, captured at bind time.
    /// `Affine2`, not `Mat4` — ADR 0002.
    #[serde(skip)]
    pub inverse_bind_matrices: HashMap<BoneId, Affine2>,

    /// Geometry borrowed from another mesh. When set, this mesh's own
    /// `setup_vertices`, `uvs`, `triangles` and `weights` are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked: Option<LinkedMesh>,

    /// Frames this mesh cycles through instead of `texture`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Sequence>,
}

impl MeshAttachment {
    /// Turn a region into an equivalent 4-vertex mesh (T-401).
    ///
    /// The quad starts exactly where the region drew — same corners, same UVs —
    /// so converting is invisible until the user moves a vertex. Anything else
    /// would make "convert to mesh" feel like it broke the placement.
    ///
    /// Vertices are in the bone's local space, which is where
    /// [`RegionAttachment::local_corners`] leaves them, so the pivot, offset,
    /// rotation and scale are all baked in and the mesh needs none of them.
    pub fn from_region(region: &RegionAttachment) -> Self {
        let corners = region.local_corners();
        let uv = &region.uv_rect;
        // `local_corners` is TL, BL, BR, TR; UVs follow the same order, with v
        // increasing downward (texture space) against y increasing upward.
        let uvs = vec![
            Vec2::new(uv.x, uv.y),
            Vec2::new(uv.x, uv.y + uv.h),
            Vec2::new(uv.x + uv.w, uv.y + uv.h),
            Vec2::new(uv.x + uv.w, uv.y),
        ];
        Self {
            texture: region.texture.clone(),
            setup_vertices: corners.to_vec(),
            uvs,
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            edges: Vec::new(),
            weights: Vec::new(),
            ffd_keyframes: Vec::new(),
            inverse_bind_matrices: HashMap::new(),
            linked: None,
            sequence: None,
        }
    }

    /// The mesh's bounding box in local space, for UV mapping and framing.
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for v in &self.setup_vertices {
            min = min.min(*v);
            max = max.max(*v);
        }
        if self.setup_vertices.is_empty() {
            (Vec2::ZERO, Vec2::ZERO)
        } else {
            (min, max)
        }
    }

    /// Guess a UV for a point by where it sits in the mesh's bounds.
    ///
    /// Used when a vertex is added: the new point has no UV of its own, and
    /// interpolating the bounds keeps the texture continuous for the flat quads
    /// and near-planar shapes this is used on.
    pub fn uv_for_local(&self, point: Vec2) -> Vec2 {
        let (min, max) = self.bounds();
        let size = (max - min).max(Vec2::splat(1e-6));
        let t = (point - min) / size;
        // Find the UV rect the existing vertices span, so this works whether the
        // mesh samples a whole texture or a sub-rect of an atlas.
        let mut uv_min = Vec2::splat(f32::MAX);
        let mut uv_max = Vec2::splat(f32::MIN);
        for uv in &self.uvs {
            uv_min = uv_min.min(*uv);
            uv_max = uv_max.max(*uv);
        }
        if self.uvs.is_empty() {
            return t;
        }
        // v runs opposite to y: the top of the shape is the minimum v.
        Vec2::new(
            uv_min.x + t.x * (uv_max.x - uv_min.x),
            uv_max.y - t.y * (uv_max.y - uv_min.y),
        )
    }

    /// Capture inverse bind affines from the **setup pose** — the pose obtained
    /// by evaluating the skeleton with no animations applied.
    /// `mesh_space` is the world affine of the bone the mesh's vertices are
    /// expressed in — its slot's bone. Without it the bind is wrong by exactly
    /// that transform: `inverse_bind` maps *world* into a bone's frame, while
    /// `setup_vertices` are local, so skinning fed local coordinates to a
    /// world-space matrix. A mesh whose slot bone sat at the origin hid it;
    /// anything else displaced the mesh, which is what a rig with a hip bone at
    /// y=247 showed as a character standing on its head.
    pub fn bind_to_pose(&mut self, setup_pose: &crate::pose::Pose, mesh_space: Affine2) {
        self.inverse_bind_matrices.clear();
        for vertex_weights in &self.weights {
            for vw in vertex_weights {
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    self.inverse_bind_matrices.entry(vw.bone)
                {
                    // A bone with a zero-scale axis has no invertible bind
                    // affine; skip it so its weights fall back to setup.
                    // Bone⁻¹ ∘ mesh-space: takes a vertex from the frame it is
                    // stored in, through the world, into the bone's frame. At
                    // the setup pose the two cancel and skinning reproduces the
                    // rigid result exactly, which is the property that makes a
                    // rebind invisible.
                    if let Some(inv) = setup_pose
                        .worlds
                        .get(vw.bone)
                        .and_then(|world| world.invert())
                    {
                        slot.insert(inv.mul(&mesh_space));
                    }
                }
            }
        }
    }

    /// Skin one vertex against an evaluated [`Pose`](crate::pose::Pose).
    ///
    /// `mesh_space` is the world affine of the bone the vertices are stored in —
    /// the slot's bone — and is what a vertex with no usable influence falls
    /// back to.
    ///
    /// That fallback is the whole reason this takes the argument. A mesh is
    /// skinned as a unit: the moment *one* vertex has weights, every vertex goes
    /// through here. Unweighted ones used to fall back to `setup_pos`, which is a
    /// **local** coordinate being returned as a world one — so painting part of a
    /// mesh left the painted vertices in place and collapsed the rest toward the
    /// origin, tearing the artwork apart. Rigid placement is the correct
    /// fallback, and it is exactly what the mesh drew a moment before it gained
    /// its first weight.
    pub fn skin_vertex_with_ffd(
        &self,
        vertex_idx: usize,
        ffd_offset: Vec2,
        pose: &crate::pose::Pose,
        mesh_space: &Affine2,
    ) -> Vec2 {
        let setup_pos = self.setup_vertices[vertex_idx] + ffd_offset;
        let rigid = mesh_space.transform_point(setup_pos);

        let Some(vertex_weights) = self
            .weights
            .get(vertex_idx)
            .filter(|weights| !weights.is_empty())
        else {
            return rigid;
        };

        let mut final_pos = Vec2::ZERO;
        let mut total_weight = 0.0;
        for vw in vertex_weights {
            if let (Some(inv_bind), Some(world)) = (
                self.inverse_bind_matrices.get(&vw.bone),
                pose.worlds.get(vw.bone),
            ) {
                let skin = world.mul(inv_bind);
                final_pos += skin.transform_point(setup_pos) * vw.weight;
                total_weight += vw.weight;
            }
        }
        // Every influence named a bone with no bind — a bone deleted since, or
        // one whose setup affine will not invert. Rigid, not local.
        if total_weight <= 0.0 {
            return rigid;
        }
        final_pos / total_weight
    }
}

/// A polygon that masks the slots drawn after it (T-405).
///
/// Clipping runs over a **range** of slots, not one: masking a character behind
/// a window means "everything from here until the end slot", which is how the
/// draw order already reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClippingAttachment {
    /// Polygon in the bone's local space, in perimeter order.
    pub vertices: Vec<Vec2>,
    /// Name of the slot the clip stops at, inclusive. `None` clips everything
    /// drawn after it.
    pub end_slot: Option<String>,
}

impl ClippingAttachment {
    /// Is `point` (already in the same local space) inside the polygon?
    ///
    /// Even-odd, so a self-intersecting clip behaves predictably rather than
    /// erroring: the user can see the result and fix the shape.
    pub fn contains(&self, point: Vec2) -> bool {
        if self.vertices.len() < 3 {
            return true; // A degenerate clip masks nothing.
        }
        polygon_contains(&self.vertices, point)
    }

    /// Axis-aligned bounds, which is what a scissor-rect renderer can use.
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        if self.vertices.is_empty() {
            return None;
        }
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for v in &self.vertices {
            min = min.min(*v);
            max = max.max(*v);
        }
        Some((min, max))
    }
}

/// A curve bones can be driven along (T-502).
///
/// Authored as a polyline with the same vertex tools a mesh uses. The stored
/// shape *is* the polyline: a bezier nobody can measure is no use to a
/// constraint, and flattening at author time means the editor and the runtime
/// walk identical geometry rather than two different subdivisions of one curve.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathAttachment {
    /// Vertices in the bone's local space, in order.
    pub vertices: Vec<Vec2>,
    /// Whether the last vertex connects back to the first.
    #[serde(default)]
    pub closed: bool,
    /// Space bones by distance rather than by vertex index.
    ///
    /// On by default because the alternative bunches a chain wherever the curve
    /// is tight, which is never what anyone wants and is hard to diagnose.
    #[serde(default = "yes")]
    pub constant_speed: bool,
}

fn yes() -> bool {
    true
}

impl PathAttachment {
    /// Flatten into world space for sampling.
    pub fn sample(&self, world: &crate::transforms::Affine2) -> crate::path::SampledPath {
        crate::path::SampledPath::new(
            self.vertices
                .iter()
                .map(|v| world.transform_point(*v))
                .collect(),
            self.closed,
        )
    }
}

/// How a sequence of frames advances over time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum SequenceMode {
    /// Stay on the setup frame until a timeline says otherwise.
    #[default]
    Hold,
    Once,
    Loop,
    PingPong,
    OnceReverse,
    LoopReverse,
    PingPongReverse,
}

/// A list of textures one attachment cycles through.
///
/// Frames are named explicitly rather than derived from a numeric suffix. The
/// asset database already keys images by name, and a convention that breaks the
/// moment someone renames `fire_09` to `fire_9` is a support burden we can
/// decline.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Sequence {
    pub frames: Vec<TextureRef>,
    pub fps: f32,
    pub mode: SequenceMode,
    /// Frame shown in setup, and the one playback starts from.
    pub setup_index: u32,
}

impl Sequence {
    /// Which frame is showing `time` seconds into playback.
    ///
    /// Returns an index into [`Self::frames`], already wrapped or clamped per
    /// [`Self::mode`], so callers never have to bounds-check.
    pub fn index_at(&self, time: f32) -> u32 {
        let count = self.frames.len() as i64;
        if count == 0 {
            return 0;
        }
        let start = (self.setup_index as i64).clamp(0, count - 1);
        if self.fps <= 0.0 || matches!(self.mode, SequenceMode::Hold) {
            return start as u32;
        }
        let elapsed = (time * self.fps).floor() as i64;
        // One shared forward walk; the reverse modes are that walk negated, which
        // keeps "does ping-pong bounce on the last frame or past it" a question
        // with exactly one answer.
        let reverse = matches!(
            self.mode,
            SequenceMode::OnceReverse | SequenceMode::LoopReverse | SequenceMode::PingPongReverse
        );
        let walked = start + if reverse { -elapsed } else { elapsed };
        let index = match self.mode {
            SequenceMode::Hold => start,
            SequenceMode::Once | SequenceMode::OnceReverse => walked.clamp(0, count - 1),
            SequenceMode::Loop | SequenceMode::LoopReverse => walked.rem_euclid(count),
            SequenceMode::PingPong | SequenceMode::PingPongReverse => {
                // A full bounce is 2n-2 frames: forward over n, back over the n-2
                // interior ones, so neither end is held for two frames running.
                let span = (2 * count - 2).max(1);
                let phase = walked.rem_euclid(span);
                if phase < count { phase } else { span - phase }
            }
        };
        index.clamp(0, count - 1) as u32
    }

    /// The texture for a frame index, clamped to the list.
    pub fn frame(&self, index: u32) -> Option<&TextureRef> {
        if self.frames.is_empty() {
            return None;
        }
        Some(&self.frames[(index as usize).min(self.frames.len() - 1)])
    }
}

/// A mesh that borrows another mesh's geometry.
///
/// Two skins sharing a silhouette — the same jacket in three colours — should
/// share one set of vertices, weights and triangles. Editing the source then
/// updates every copy, and a deform keyed once plays on all of them, which is
/// the whole reason the link exists rather than a duplicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedMesh {
    /// Skin holding the source, or `None` for the default skin.
    pub skin: Option<String>,
    /// Slot the source attachment lives under.
    pub slot: String,
    /// The source attachment's name.
    pub attachment: String,
    /// Follow the source's deform timelines as well as its geometry.
    #[serde(default = "yes")]
    pub inherit_deform: bool,
}

/// A polygon used for hit tests, triggers and spawn regions.
///
/// Skinnable like a mesh: a hitbox that does not follow the pose it belongs to
/// is worse than no hitbox, because it looks like it works.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BoundingBoxAttachment {
    /// Polygon in the bone's local space, in perimeter order.
    pub vertices: Vec<Vec2>,
    /// Per-vertex bone influences. Empty means the polygon is rigid to its slot's
    /// bone, exactly as a mesh with no weights is.
    #[serde(default)]
    pub weights: Vec<Vec<VertexWeight>>,
}

impl BoundingBoxAttachment {
    /// Is `point` (in the same space as `vertices`) inside the polygon?
    pub fn contains(&self, point: Vec2) -> bool {
        polygon_contains(&self.vertices, point)
    }
}

/// A named point with an orientation, carried by a bone.
///
/// What a muzzle flash, a footstep spark or a "hold the sword here" marker
/// attaches to. It draws nothing, so it costs a slot and no draw call.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PointAttachment {
    /// Position in the bone's local space.
    pub position: Vec2,
    /// Rotation relative to the bone, in radians.
    pub rotation: f32,
}

impl PointAttachment {
    /// World position and world rotation, given the owning bone's world affine.
    pub fn world(&self, bone_world: &Affine2) -> (Vec2, f32) {
        let position = bone_world.transform_point(self.position);
        let axis = bone_world.transform_vector(Vec2::from_angle(self.rotation));
        (position, axis.to_angle())
    }
}

/// Even-odd point-in-polygon, shared by clipping and bounding boxes so the two
/// never disagree about a shape that touches its own edge.
fn polygon_contains(vertices: &[Vec2], point: Vec2) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = vertices.len() - 1;
    for i in 0..vertices.len() {
        let (a, b) = (vertices[i], vertices[j]);
        if (a.y > point.y) != (b.y > point.y) {
            let t = (point.y - a.y) / (b.y - a.y);
            if point.x < a.x + t * (b.x - a.x) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

// A mesh is much larger than a point, and boxing the big variants would put an
// indirection on the draw path for every sprite to save bytes on the rare
// marker. Skins hold one of these per entry, not per frame.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Attachment {
    Region(RegionAttachment),
    Mesh(MeshAttachment),
    Clipping(ClippingAttachment),
    Path(PathAttachment),
    BoundingBox(BoundingBoxAttachment),
    Point(PointAttachment),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region() -> RegionAttachment {
        RegionAttachment {
            texture: "img".into(),
            local_offset: Vec2::ZERO,
            local_rotation: 0.0,
            local_scale: Vec2::ONE,
            width: 100.0,
            height: 50.0,
            uv_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            pivot: Vec2::splat(0.5),
            sequence: None,
        }
    }

    #[test]
    fn clipping_contains_only_inside_the_polygon() {
        let clip = ClippingAttachment {
            vertices: vec![
                Vec2::new(-10.0, -10.0),
                Vec2::new(10.0, -10.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(-10.0, 10.0),
            ],
            end_slot: None,
        };
        assert!(clip.contains(Vec2::ZERO));
        assert!(!clip.contains(Vec2::new(20.0, 0.0)));
        assert_eq!(clip.bounds(), Some((Vec2::splat(-10.0), Vec2::splat(10.0))));
    }

    /// A clip with too few points masks nothing, rather than hiding the art
    /// behind it — an unfinished polygon should not blank the character.
    #[test]
    fn a_degenerate_clip_masks_nothing() {
        let clip = ClippingAttachment {
            vertices: vec![Vec2::ZERO, Vec2::new(1.0, 0.0)],
            end_slot: None,
        };
        assert!(clip.contains(Vec2::new(500.0, 500.0)));
    }

    #[test]
    fn centre_pivot_centres_the_quad() {
        let corners = region().local_corners();
        assert_eq!(corners[0], Vec2::new(-50.0, 25.0), "top-left");
        assert_eq!(corners[2], Vec2::new(50.0, -25.0), "bottom-right");
    }

    #[test]
    fn pivot_moves_the_quad_not_the_anchor() {
        // Bottom-left pivot: the quad extends up and right from the offset.
        let mut r = region();
        r.pivot = Vec2::ZERO;
        let corners = r.local_corners();
        assert_eq!(corners[1], Vec2::ZERO, "bottom-left sits on the offset");
        assert_eq!(corners[3], Vec2::new(100.0, 50.0));
    }

    #[test]
    fn rotation_turns_about_the_pivot() {
        // Pivot at the bottom-centre, rotated 90°: the sprite swings up from its
        // base like a limb, instead of spinning about its middle.
        let mut r = region();
        r.pivot = Vec2::new(0.5, 0.0);
        r.local_rotation = std::f32::consts::FRAC_PI_2;
        let corners = r.local_corners();
        // The pivot itself does not move.
        let base_mid = (corners[1] + corners[2]) * 0.5;
        assert!(base_mid.length() < 1e-4, "pivot stayed put: {base_mid:?}");
        // The far edge is now 50 units along -X (rotated from +Y).
        let far_mid = (corners[0] + corners[3]) * 0.5;
        assert!(
            (far_mid - Vec2::new(-50.0, 0.0)).length() < 1e-4,
            "{far_mid:?}"
        );
    }
    /// Binding at the setup pose must be invisible: skinning has to reproduce
    /// exactly what rigid attachment produced, or every weighted mesh jumps the
    /// moment it gains its first weight.
    ///
    /// The bind matrix maps *world* into a bone's frame, but `setup_vertices`
    /// are in the slot bone's frame — so the bind has to carry that frame too.
    /// A rig whose slot bone sits at the origin hides the difference entirely,
    /// which is why this went unnoticed until a rig with a hip bone at y=247
    /// drew its character upside down.
    #[test]
    fn binding_at_setup_reproduces_the_rigid_pose() {
        use crate::ids::BoneId;
        use crate::math::Transform;
        use crate::pose::{Pose, evaluate};
        use crate::skeleton::{Bone, Skeleton};

        let mut skel = Skeleton::new();
        let bone = |name: &str, parent: Option<BoneId>, pos: Vec2, deg: f32| Bone {
            name: name.into(),
            parent,
            length: 40.0,
            local_transform: Transform {
                position: pos,
                rotation: deg.to_radians(),
                ..Transform::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        };
        // A hip well away from the origin, which is what exposes the bug.
        let hip = skel.add_bone(bone("hip", None, Vec2::new(0.0, 247.0), 0.0));
        let torso = skel.add_bone(bone("torso", Some(hip), Vec2::new(0.0, 60.0), 15.0));

        let mut mesh = MeshAttachment {
            texture: "art".into(),
            // In the slot bone's frame, as our meshes always are.
            setup_vertices: vec![
                Vec2::new(-20.0, 0.0),
                Vec2::new(20.0, 0.0),
                Vec2::new(0.0, 50.0),
            ],
            uvs: vec![Vec2::ZERO; 3],
            triangles: vec![[0, 1, 2]],
            weights: vec![
                vec![VertexWeight {
                    bone: hip,
                    weight: 1.0,
                }],
                vec![
                    VertexWeight {
                        bone: hip,
                        weight: 0.5,
                    },
                    VertexWeight {
                        bone: torso,
                        weight: 0.5,
                    },
                ],
                vec![VertexWeight {
                    bone: torso,
                    weight: 1.0,
                }],
            ],
            ..MeshAttachment::default()
        };

        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        // The mesh hangs off the hip, so that is the frame its vertices are in.
        mesh.bind_to_pose(&pose, pose.world(hip));

        for (i, v) in mesh.setup_vertices.iter().enumerate() {
            let rigid = pose.world(hip).transform_point(*v);
            let skinned = mesh.skin_vertex_with_ffd(i, Vec2::ZERO, &pose, &pose.world(hip));
            assert!(
                (rigid - skinned).length() < 1e-3,
                "vertex {i}: rigid {rigid:?} but skinned {skinned:?}"
            );
        }
    }

    /// And once a bone moves, the skinned vertices follow it — otherwise the
    /// test above would pass with skinning that does nothing at all.
    #[test]
    fn skinned_vertices_follow_their_bones() {
        use crate::math::Transform;
        use crate::pose::{Pose, evaluate};
        use crate::skeleton::{Bone, Skeleton};

        let mut skel = Skeleton::new();
        let hip = skel.add_bone(Bone {
            name: "hip".into(),
            parent: None,
            length: 40.0,
            local_transform: Transform {
                position: Vec2::new(0.0, 100.0),
                ..Transform::default()
            },
            inherit: Default::default(),
            color: Bone::default_color(),
        });
        let mut mesh = MeshAttachment {
            setup_vertices: vec![Vec2::new(10.0, 0.0)],
            uvs: vec![Vec2::ZERO],
            weights: vec![vec![VertexWeight {
                bone: hip,
                weight: 1.0,
            }]],
            ..MeshAttachment::default()
        };
        let mut pose = Pose::new();
        evaluate(&skel, &[], &mut pose);
        mesh.bind_to_pose(&pose, pose.world(hip));
        let before = mesh.skin_vertex_with_ffd(0, Vec2::ZERO, &pose, &pose.world(hip));

        // Turn the hip a quarter turn and re-evaluate.
        skel.bones[hip].local_transform.rotation = std::f32::consts::FRAC_PI_2;
        evaluate(&skel, &[], &mut pose);
        let after = mesh.skin_vertex_with_ffd(0, Vec2::ZERO, &pose, &pose.world(hip));

        assert!(
            (before - Vec2::new(10.0, 100.0)).length() < 1e-3,
            "started beside the hip: {before:?}"
        );
        assert!(
            (after - Vec2::new(0.0, 110.0)).length() < 1e-3,
            "swung with it: {after:?}"
        );
    }
}

#[cfg(test)]
mod new_attachment_tests {
    use super::*;

    fn seq(mode: SequenceMode, count: usize) -> Sequence {
        Sequence {
            frames: (0..count).map(|i| format!("f{i}")).collect(),
            fps: 10.0,
            mode,
            setup_index: 0,
        }
    }

    #[test]
    fn hold_ignores_time() {
        let mut s = seq(SequenceMode::Hold, 4);
        s.setup_index = 2;
        assert_eq!(s.index_at(0.0), 2);
        assert_eq!(s.index_at(9.9), 2);
    }

    #[test]
    fn once_clamps_at_the_last_frame() {
        let s = seq(SequenceMode::Once, 4);
        assert_eq!(s.index_at(0.0), 0);
        assert_eq!(s.index_at(0.25), 2);
        assert_eq!(s.index_at(5.0), 3);
    }

    #[test]
    fn loop_wraps() {
        let s = seq(SequenceMode::Loop, 4);
        assert_eq!(s.index_at(0.4), 0);
        assert_eq!(s.index_at(0.5), 1);
    }

    #[test]
    fn ping_pong_holds_no_end_frame_twice() {
        let s = seq(SequenceMode::PingPong, 4);
        // 0 1 2 3 2 1 | 0 1 2 3 ...
        let walked: Vec<u32> = (0..8).map(|i| s.index_at(i as f32 / 10.0)).collect();
        assert_eq!(walked, vec![0, 1, 2, 3, 2, 1, 0, 1]);
    }

    #[test]
    fn reverse_runs_the_same_walk_backwards() {
        let mut s = seq(SequenceMode::LoopReverse, 4);
        s.setup_index = 3;
        let walked: Vec<u32> = (0..5).map(|i| s.index_at(i as f32 / 10.0)).collect();
        assert_eq!(walked, vec![3, 2, 1, 0, 3]);
    }

    #[test]
    fn a_sequence_without_frames_is_harmless() {
        let s = Sequence {
            frames: Vec::new(),
            fps: 24.0,
            mode: SequenceMode::Loop,
            setup_index: 7,
        };
        assert_eq!(s.index_at(1.0), 0);
        assert!(s.frame(0).is_none());
    }

    #[test]
    fn bounding_box_hit_test_matches_its_polygon() {
        let bb = BoundingBoxAttachment {
            vertices: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(10.0, 0.0),
                Vec2::new(10.0, 10.0),
                Vec2::new(0.0, 10.0),
            ],
            weights: Vec::new(),
        };
        assert!(bb.contains(Vec2::new(5.0, 5.0)));
        assert!(!bb.contains(Vec2::new(15.0, 5.0)));
    }

    #[test]
    fn a_degenerate_bounding_box_contains_nothing() {
        // The opposite of clipping, on purpose: an empty clip masks nothing, but
        // an empty hitbox must not swallow every hit test in the scene.
        let bb = BoundingBoxAttachment::default();
        assert!(!bb.contains(Vec2::ZERO));
    }

    #[test]
    fn a_point_rides_its_bone() {
        let bone = Affine2::compose(&crate::math::Transform {
            position: Vec2::new(100.0, 0.0),
            rotation: std::f32::consts::FRAC_PI_2,
            scale: Vec2::ONE,
            shear: Vec2::ZERO,
        });
        let point = PointAttachment {
            position: Vec2::new(10.0, 0.0),
            rotation: 0.0,
        };
        let (pos, rot) = point.world(&bone);
        assert!((pos - Vec2::new(100.0, 10.0)).length() < 1e-4, "{pos:?}");
        assert!((rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }
}
