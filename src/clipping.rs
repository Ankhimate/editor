//! Polygon clipping for clipping attachments (T-405).
//!
//! # Why geometry, not a stencil buffer
//!
//! The obvious implementation is a stencil pass: draw the clip polygon into the
//! stencil buffer, then draw the masked slots with a stencil test. The editor
//! cannot do that — it renders inside egui's own render pass, which has no
//! depth-stencil attachment, and taking one would mean rendering the whole
//! viewport to a private texture first.
//!
//! Clipping the triangles instead costs nothing at draw time, is exact rather
//! than sampled, and — the deciding reason — is what a runtime has to do anyway:
//! `ankhimate-runtime` emits triangle batches to whatever renderer the game
//! brought, and cannot assume a stencil buffer exists there either. One
//! implementation, in `core`, and the editor and the runtime cannot disagree
//! about where a mask's edge falls.
//!
//! The clip polygon may be concave, so it is decomposed into triangles and each
//! source triangle is clipped against each of them. Sutherland-Hodgman needs a
//! convex clip region; a triangle always is one.

use glam::Vec2;

/// A vertex being clipped: position plus the attributes to interpolate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipVertex {
    pub position: Vec2,
    pub uv: Vec2,
}

/// Ear-clipping triangulation of a simple polygon, concave included.
///
/// Returns index triples into `polygon`. An empty result means the polygon was
/// degenerate — fewer than three points, or all of them collinear — which the
/// caller should read as "masks nothing" rather than as an error.
pub fn triangulate_polygon(polygon: &[Vec2]) -> Vec<[usize; 3]> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    // Work counter-clockwise so "convex" has one meaning below.
    let mut remaining: Vec<usize> = (0..polygon.len()).collect();
    if signed_area(polygon) < 0.0 {
        remaining.reverse();
    }

    let mut triangles = Vec::with_capacity(polygon.len().saturating_sub(2));
    // Each pass must remove at least one ear; the guard stops a malformed
    // polygon (self-intersecting, duplicate points) from looping forever.
    let mut guard = remaining.len() * remaining.len();
    while remaining.len() > 3 && guard > 0 {
        guard -= 1;
        let count = remaining.len();
        let mut clipped = false;
        for i in 0..count {
            let (a, b, c) = (
                remaining[(i + count - 1) % count],
                remaining[i],
                remaining[(i + 1) % count],
            );
            let (pa, pb, pc) = (polygon[a], polygon[b], polygon[c]);
            // Convex corner?
            if cross(pb - pa, pc - pb) <= 0.0 {
                continue;
            }
            // An ear may not contain any other vertex of the polygon.
            let contains = remaining
                .iter()
                .filter(|&&v| v != a && v != b && v != c)
                .any(|&v| point_in_triangle(polygon[v], pa, pb, pc));
            if contains {
                continue;
            }
            triangles.push([a, b, c]);
            remaining.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            // No ear found: the polygon is not simple. Stop with what is built
            // rather than spin — a partial mask is visible and fixable.
            break;
        }
    }
    if remaining.len() == 3 {
        triangles.push([remaining[0], remaining[1], remaining[2]]);
    }
    triangles
}

/// Clip a convex polygon (given as a vertex ring) against a convex clip
/// triangle, interpolating UVs at every cut.
///
/// Sutherland-Hodgman. Returns a ring of 0..=7 vertices; fewer than three means
/// the source lay entirely outside.
pub fn clip_to_triangle(subject: &[ClipVertex], clip: [Vec2; 3]) -> Vec<ClipVertex> {
    // Counter-clockwise, so "inside" is consistently to the left of each edge.
    let clip = if cross(clip[1] - clip[0], clip[2] - clip[1]) < 0.0 {
        [clip[0], clip[2], clip[1]]
    } else {
        clip
    };

    let mut output = subject.to_vec();
    for i in 0..3 {
        if output.is_empty() {
            break;
        }
        let (edge_a, edge_b) = (clip[i], clip[(i + 1) % 3]);
        let input = std::mem::take(&mut output);
        let inside = |p: Vec2| cross(edge_b - edge_a, p - edge_a) >= 0.0;

        for (index, current) in input.iter().enumerate() {
            let previous = input[(index + input.len() - 1) % input.len()];
            let (current_in, previous_in) = (inside(current.position), inside(previous.position));
            if current_in {
                if !previous_in {
                    output.push(intersect(previous, *current, edge_a, edge_b));
                }
                output.push(*current);
            } else if previous_in {
                output.push(intersect(previous, *current, edge_a, edge_b));
            }
        }
    }
    output
}

/// Clip a triangle list against a clipping polygon.
///
/// Returns the surviving geometry as a new triangle list. A source triangle may
/// come back as several triangles, or as none.
///
/// The polygon is triangulated once and each source triangle clipped against
/// every piece. Pieces share edges but do not overlap, so a triangle spanning
/// two of them comes back as two parts that meet exactly — no double-blending
/// along the seam.
pub fn clip_triangles(
    vertices: &[ClipVertex],
    indices: &[u32],
    polygon: &[Vec2],
) -> (Vec<ClipVertex>, Vec<u32>) {
    let pieces = triangulate_polygon(polygon);
    if pieces.is_empty() {
        // A degenerate clip masks nothing. Refusing to draw would make a
        // half-built polygon look like a broken rig.
        return (vertices.to_vec(), indices.to_vec());
    }

    let mut out_vertices: Vec<ClipVertex> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();

    for triangle in indices.chunks_exact(3) {
        let (Some(&a), Some(&b), Some(&c)) = (
            vertices.get(triangle[0] as usize),
            vertices.get(triangle[1] as usize),
            vertices.get(triangle[2] as usize),
        ) else {
            continue;
        };
        let subject = [a, b, c];
        for piece in &pieces {
            let clip = [polygon[piece[0]], polygon[piece[1]], polygon[piece[2]]];
            let ring = clip_to_triangle(&subject, clip);
            if ring.len() < 3 {
                continue;
            }
            // Fan the clipped ring: it is convex by construction.
            let base = out_vertices.len() as u32;
            out_vertices.extend_from_slice(&ring);
            for i in 1..ring.len() as u32 - 1 {
                out_indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
    }
    (out_vertices, out_indices)
}

fn intersect(from: ClipVertex, to: ClipVertex, edge_a: Vec2, edge_b: Vec2) -> ClipVertex {
    let direction = to.position - from.position;
    let edge = edge_b - edge_a;
    let denominator = cross(edge, direction);
    // Parallel: the segment does not actually cross this edge, so the endpoint
    // is as good an answer as any and keeps the ring closed.
    if denominator.abs() < 1e-9 {
        return to;
    }
    let t = cross(edge, edge_a - from.position) / denominator;
    let t = t.clamp(0.0, 1.0);
    ClipVertex {
        position: from.position + direction * t,
        uv: from.uv + (to.uv - from.uv) * t,
    }
}

fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn signed_area(polygon: &[Vec2]) -> f32 {
    let mut area = 0.0;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        area += a.x * b.y - b.x * a.y;
    }
    area * 0.5
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(size: f32) -> (Vec<ClipVertex>, Vec<u32>) {
        let v = |x: f32, y: f32, u: f32, w: f32| ClipVertex {
            position: Vec2::new(x, y),
            uv: Vec2::new(u, w),
        };
        (
            vec![
                v(-size, -size, 0.0, 1.0),
                v(size, -size, 1.0, 1.0),
                v(size, size, 1.0, 0.0),
                v(-size, size, 0.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn area(vertices: &[ClipVertex], indices: &[u32]) -> f32 {
        indices
            .chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (
                    vertices[t[0] as usize].position,
                    vertices[t[1] as usize].position,
                    vertices[t[2] as usize].position,
                );
                cross(b - a, c - a).abs() * 0.5
            })
            .sum()
    }

    #[test]
    fn a_square_clipped_by_its_own_half_keeps_half_the_area() {
        let (vertices, indices) = quad(10.0);
        // Right half of the 20×20 quad.
        let polygon = vec![
            Vec2::new(0.0, -10.0),
            Vec2::new(10.0, -10.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let (out_v, out_i) = clip_triangles(&vertices, &indices, &polygon);
        assert!((area(&out_v, &out_i) - 200.0).abs() < 1e-2, "half of 400");
        for v in &out_v {
            assert!(v.position.x >= -1e-3, "nothing left of the cut: {v:?}");
        }
    }

    #[test]
    fn geometry_entirely_outside_the_clip_disappears() {
        let (vertices, indices) = quad(1.0);
        let polygon = vec![
            Vec2::new(50.0, 50.0),
            Vec2::new(60.0, 50.0),
            Vec2::new(60.0, 60.0),
        ];
        let (_, out_i) = clip_triangles(&vertices, &indices, &polygon);
        assert!(out_i.is_empty(), "nothing survives a disjoint clip");
    }

    #[test]
    fn a_clip_larger_than_the_art_keeps_all_of_it() {
        let (vertices, indices) = quad(5.0);
        let polygon = vec![
            Vec2::new(-100.0, -100.0),
            Vec2::new(100.0, -100.0),
            Vec2::new(100.0, 100.0),
            Vec2::new(-100.0, 100.0),
        ];
        let (out_v, out_i) = clip_triangles(&vertices, &indices, &polygon);
        assert!((area(&out_v, &out_i) - 100.0).abs() < 1e-2);
    }

    /// The reason this is geometry clipping and not a scissor rect: a concave
    /// mask has to actually cut in, not just bound.
    #[test]
    fn a_concave_clip_removes_its_notch() {
        let (vertices, indices) = quad(10.0);
        // A 20×20 square with a notch bitten out of the top middle.
        let polygon = vec![
            Vec2::new(-10.0, -10.0),
            Vec2::new(10.0, -10.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(2.0, 10.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(-2.0, 10.0),
            Vec2::new(-10.0, 10.0),
        ];
        let (out_v, out_i) = clip_triangles(&vertices, &indices, &polygon);
        let clipped = area(&out_v, &out_i);
        assert!(clipped < 400.0, "the notch came out: {clipped}");
        assert!(clipped > 350.0, "only the notch came out: {clipped}");
    }

    #[test]
    fn uvs_are_interpolated_at_the_cut() {
        // One triangle spanning u 0→1, cut down the middle.
        let vertices = vec![
            ClipVertex {
                position: Vec2::new(0.0, 0.0),
                uv: Vec2::new(0.0, 0.0),
            },
            ClipVertex {
                position: Vec2::new(10.0, 0.0),
                uv: Vec2::new(1.0, 0.0),
            },
            ClipVertex {
                position: Vec2::new(0.0, 10.0),
                uv: Vec2::new(0.0, 1.0),
            },
        ];
        let polygon = vec![
            Vec2::new(-1.0, -1.0),
            Vec2::new(5.0, -1.0),
            Vec2::new(5.0, 11.0),
            Vec2::new(-1.0, 11.0),
        ];
        let (out_v, _) = clip_triangles(&vertices, &[0, 1, 2], &polygon);
        // The vertex created at x = 5 must carry u = 0.5, not u = 0 or 1.
        let cut = out_v
            .iter()
            .find(|v| (v.position.x - 5.0).abs() < 1e-3 && v.position.y.abs() < 1e-3)
            .expect("a vertex on the cut");
        assert!(
            (cut.uv.x - 0.5).abs() < 1e-3,
            "u should be halfway: {:?}",
            cut.uv
        );
    }

    #[test]
    fn a_degenerate_polygon_masks_nothing() {
        let (vertices, indices) = quad(1.0);
        let (out_v, out_i) = clip_triangles(&vertices, &indices, &[Vec2::ZERO, Vec2::ONE]);
        assert_eq!(out_v.len(), vertices.len());
        assert_eq!(out_i, indices);
    }

    #[test]
    fn ear_clipping_covers_a_concave_polygon() {
        let polygon = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(2.0, 1.0), // the reflex vertex
            Vec2::new(0.0, 4.0),
        ];
        let triangles = triangulate_polygon(&polygon);
        assert_eq!(triangles.len(), 3, "n-2 triangles for n=5");

        let covered: f32 = triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (polygon[t[0]], polygon[t[1]], polygon[t[2]]);
                cross(b - a, c - a).abs() * 0.5
            })
            .sum();
        assert!(
            (covered - signed_area(&polygon).abs()).abs() < 1e-3,
            "the pieces tile the polygon: {covered}"
        );
    }
}
