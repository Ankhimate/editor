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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Inverse bind affine per influencing bone, captured at bind time.
    /// `Affine2`, not `Mat4` — ADR 0002.
    #[serde(skip)]
    pub inverse_bind_matrices: HashMap<BoneId, Affine2>,
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
            weights: Vec::new(),
            ffd_keyframes: Vec::new(),
            inverse_bind_matrices: HashMap::new(),
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
    pub fn bind_to_pose(&mut self, setup_pose: &crate::pose::Pose) {
        self.inverse_bind_matrices.clear();
        for vertex_weights in &self.weights {
            for vw in vertex_weights {
                if let std::collections::hash_map::Entry::Vacant(slot) =
                    self.inverse_bind_matrices.entry(vw.bone)
                {
                    // A bone with a zero-scale axis has no invertible bind
                    // affine; skip it so its weights fall back to setup.
                    if let Some(inv) = setup_pose
                        .worlds
                        .get(vw.bone)
                        .and_then(|world| world.invert())
                    {
                        slot.insert(inv);
                    }
                }
            }
        }
    }

    /// Skin one vertex against an evaluated [`Pose`](crate::pose::Pose).
    pub fn skin_vertex_with_ffd(
        &self,
        vertex_idx: usize,
        ffd_offset: Vec2,
        pose: &crate::pose::Pose,
    ) -> Vec2 {
        let setup_pos = self.setup_vertices[vertex_idx] + ffd_offset;

        let mut final_pos = Vec2::ZERO;
        let mut total_weight = 0.0;

        if vertex_idx < self.weights.len() && !self.weights[vertex_idx].is_empty() {
            for vw in &self.weights[vertex_idx] {
                if let (Some(inv_bind), Some(world)) = (
                    self.inverse_bind_matrices.get(&vw.bone),
                    pose.worlds.get(vw.bone),
                ) {
                    let skin = world.mul(inv_bind);
                    final_pos += skin.transform_point(setup_pos) * vw.weight;
                    total_weight += vw.weight;
                }
            }
            if total_weight > 0.0 {
                final_pos /= total_weight;
            } else {
                final_pos = setup_pos;
            }
        } else {
            final_pos = setup_pos;
        }

        final_pos
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
        let mut inside = false;
        let mut j = self.vertices.len() - 1;
        for i in 0..self.vertices.len() {
            let (a, b) = (self.vertices[i], self.vertices[j]);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Attachment {
    Region(RegionAttachment),
    Mesh(MeshAttachment),
    Clipping(ClippingAttachment),
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
}
