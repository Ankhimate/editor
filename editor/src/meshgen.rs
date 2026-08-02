//! Mesh triangulation (T-401).
//!
//! Lives in the editor rather than core: `core` is the runtime contract and
//! stays dependency-light (PLAN §3.1), while triangulation only ever runs when
//! someone edits a mesh. The result — a triangle list — is what gets stored, so
//! the runtime never needs this code.
//!
//! Delaunay via `spade`, covering the convex hull. Concave silhouettes need an
//! explicit outline to trim against, which arrives with the tracer (T-402) —
//! see [`retriangulate`] for why guessing one from vertex order does not work.

use ankhimate_core::attachment::MeshAttachment;
use glam::Vec2;
use spade::{DelaunayTriangulation, Point2, Triangulation};

/// Re-triangulate a mesh's current vertices (Delaunay, covering the hull).
///
/// # Why there is no concavity handling here
///
/// A first pass tried to read the vertex list as an outline and drop triangles
/// whose centroid fell outside it. That is unsound: nothing distinguishes a
/// perimeter point from an interior one. Add a vertex in the middle of a quad
/// and the list becomes a *valid* pentagon with a notch — so the filter
/// correctly, and uselessly, carved away a triangle the user wanted.
///
/// A mesh needs an explicit outline for this to work, and the only thing that
/// knows one is the tracer (T-402), whose marching-squares pass produces a real
/// contour. Until then the hull is the honest answer: over-covering a concave
/// silhouette is visible and fixable by deleting a triangle; silently deleting
/// wanted geometry is neither.
pub fn retriangulate(mesh: &mut MeshAttachment) {
    let points = &mesh.setup_vertices;
    if points.len() < 3 {
        mesh.triangles.clear();
        return;
    }

    let mut triangulation: DelaunayTriangulation<Point2<f64>> = DelaunayTriangulation::new();
    // Map spade's handles back to our indices: insertion order is not preserved
    // by the triangulation, and duplicate points collapse.
    let mut handles = Vec::with_capacity(points.len());
    for p in points {
        match triangulation.insert(Point2::new(p.x as f64, p.y as f64)) {
            Ok(handle) => handles.push(Some(handle)),
            // A duplicate or non-finite point cannot be part of the mesh; it is
            // dropped from the topology but left in the vertex list so indices
            // (and any weights keyed to them) stay stable.
            Err(_) => handles.push(None),
        }
    }

    // Handle → index as a map, not a linear scan. The scan made triangulation
    // O(vertices × triangles), which is invisible on a quad and a freeze on a
    // traced mesh with a few thousand points.
    let index_of: std::collections::HashMap<_, u32> = handles
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.map(|h| (h, i as u32)))
        .collect();
    let index_of = |handle: spade::handles::FixedVertexHandle| index_of.get(&handle).copied();

    mesh.triangles = triangulation
        .inner_faces()
        .filter_map(|face| {
            let [a, b, c] = face.vertices();
            Some([index_of(a.fix())?, index_of(b.fix())?, index_of(c.fix())?])
        })
        .collect();
}

// ── Tracing an image's silhouette (T-402) ────────────────────────────────────

/// Knobs for [`trace`] and [`refine`].
///
/// Expressed the way a rigger thinks about them — 0–100 dials, not pixel
/// tolerances. The earlier version took a raw Douglas-Peucker tolerance and an
/// interior spacing **in pixels**, which meant the same settings gave a sensible
/// mesh on a 256px sprite and a million-point one on a 2048px sheet. Everything
/// here is relative to the shape's own size, so a value that works on one image
/// works on all of them.
#[derive(Debug, Clone, Copy)]
pub struct TraceOptions {
    /// How closely the outline follows the silhouette. 0 is a loose shell, 100
    /// hugs every pixel step.
    pub detail: f32,
    /// How much extra effort goes into concave areas — the notches, armpits and
    /// gaps that a uniform simplification flattens first, and that a deforming
    /// mesh most needs vertices in.
    pub concavity: f32,
    /// How evenly spaced the outline vertices end up. 0 keeps them where the
    /// silhouette put them; 1 spaces them regularly, which deforms more
    /// predictably at the cost of following detail.
    pub uniform: f32,
    /// How much work goes into placing the outline vertices well — Spine's
    /// "time and effort to spend finding an optimal solution". It does **not**
    /// change how many vertices there are (that is `detail`); it moves the ones
    /// there are to where they describe the silhouette best.
    pub refinement: f32,
    /// Interior vertex density for [`refine`]. Interior points are what let a
    /// mesh bend in the middle rather than only at its edges.
    ///
    /// Spine has no equivalent dial — its trace produces an outline and interior
    /// points come from elsewhere. Ours needs one, and it used to be spelled
    /// `refinement`, which collided with Spine's meaning of that word.
    pub interior: f32,
    /// Alpha at or above which a pixel counts as part of the shape.
    pub alpha_threshold: u8,
    /// Push the outline outward by this many pixels, so edge texels are not
    /// clipped by the triangle boundary.
    pub padding: f32,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self {
            detail: 50.0,
            concavity: 50.0,
            uniform: 0.3,
            refinement: 50.0,
            interior: 50.0,
            // Well below half: anti-aliased edges fade gradually, and a high
            // threshold eats the soft rim that makes art look un-jagged.
            alpha_threshold: 8,
            padding: 0.0,
        }
    }
}

impl TraceOptions {
    /// Simplification tolerance in pixels, scaled to the image so the dial means
    /// the same thing at any resolution.
    fn tolerance(&self, image_size: f32) -> f32 {
        // 0 → ~2% of the image, 100 → ~0.1%.
        let fraction = lerp(0.02, 0.001, self.detail.clamp(0.0, 100.0) / 100.0);
        (image_size * fraction).max(0.25)
    }

    /// Target spacing between interior points, again relative to the shape.
    ///
    /// Every bound here is a *fraction* of the shape. An absolute floor would
    /// mean nothing in the normalized space contours live in — which is exactly
    /// how the first version ended up asking for a spacing wider than the whole
    /// image and producing no interior points at all.
    fn interior_spacing(&self, shape_size: f32) -> f32 {
        let fraction = lerp(0.35, 0.06, self.interior.clamp(0.0, 100.0) / 100.0);
        (shape_size * fraction).max(shape_size * 0.02)
    }

    /// How many relocation passes [`optimize_placement`] may run.
    ///
    /// Capped low: the pass converges quickly and stops early when nothing
    /// moves, so a huge budget buys nothing but a slower dialog.
    fn optimization_passes(&self) -> usize {
        (self.refinement.clamp(0.0, 100.0) / 100.0 * 8.0).round() as usize
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Most points a traced mesh may have. Past this the mesh is unusable by hand
/// and the triangulation is slow enough to read as a freeze.
pub const MAX_MESH_POINTS: usize = 4000;

/// A traced silhouette in **normalized image space** — `(0,0)` top-left,
/// `(1,1)` bottom-right, matching UV coordinates.
pub struct Traced {
    /// Outer boundary first, then any holes.
    pub contours: Vec<Vec<Vec2>>,
    pub interior: Vec<Vec2>,
}

/// Trace an image's opaque silhouette.
///
/// Returns `None` when nothing clears the alpha threshold — a fully transparent
/// image has no shape, and producing an empty mesh would just move the failure
/// somewhere less obvious.
pub fn trace(image: &image::RgbaImage, options: TraceOptions) -> Option<Traced> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let mask: Vec<bool> = image
        .pixels()
        .map(|p| p.0[3] >= options.alpha_threshold)
        .collect();
    if !mask.iter().any(|solid| *solid) {
        return None;
    }

    let mut contours = find_contours(&mask, width, height);
    if contours.is_empty() {
        return None;
    }
    // Biggest first: downstream code treats index 0 as the outer boundary and
    // the rest as holes.
    contours.sort_by(|a, b| area(b).abs().total_cmp(&area(a).abs()));

    let image_size = (width.max(height)) as f32;
    let tolerance = options.tolerance(image_size);
    let contours: Vec<Vec<Vec2>> = contours
        .into_iter()
        .map(|c| {
            let simplified = simplify_with_concavity(&c, tolerance, options.concavity);
            // Refinement runs here, between choosing the vertices and evening
            // them out: it changes where they sit, never how many there are.
            let optimized = optimize_placement(&c, &simplified, options.optimization_passes());
            let spaced = make_uniform(&optimized, options.uniform);
            // Padding pushes the outer boundary out and holes in, both away from
            // the artwork, so no edge texel is clipped.
            let outward = if area(&spaced) >= 0.0 { 1.0 } else { -1.0 };
            offset_polygon(&spaced, options.padding * outward)
        })
        .filter(|c| c.len() >= 3)
        .collect();
    if contours.is_empty() {
        return None;
    }

    // Tracing produces the outline only; interior points come from `refine`, the
    // way Spine splits the two. Keeping them separate means adjusting interior
    // density does not re-walk the silhouette and lose hand edits to it.
    let interior = Vec::new();

    // Hard ceiling: a mesh nobody can hand-edit is not useful, and a
    // triangulation that large reads as a hang.
    let total = contours.iter().map(|c| c.len()).sum::<usize>() + interior.len();
    if total > MAX_MESH_POINTS {
        return None;
    }

    // Normalize last, so every step above works in pixels — the unit the
    // options are expressed in.
    let to_uv = |p: Vec2| Vec2::new(p.x / width as f32, p.y / height as f32);
    Some(Traced {
        contours: contours
            .into_iter()
            .map(|c| c.into_iter().map(to_uv).collect())
            .collect(),
        interior: interior.into_iter().map(to_uv).collect(),
    })
}

/// Build a mesh from a traced silhouette, in the local space `bounds` describes.
///
/// Unlike [`retriangulate`], this **can** trim to the silhouette: the contours
/// are known, so a triangle's centroid can be tested against them properly. A
/// hole stays a hole.
pub fn mesh_from_trace(
    traced: &Traced,
    bounds: (Vec2, Vec2),
) -> (Vec<Vec2>, Vec<Vec2>, Vec<[u32; 3]>) {
    use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

    let (min, max) = bounds;
    let size = max - min;
    // v runs downward in image space, y upward in local space.
    let to_local = |uv: Vec2| Vec2::new(min.x + uv.x * size.x, max.y - uv.y * size.y);

    let mut uvs: Vec<Vec2> = Vec::new();
    let mut cdt: ConstrainedDelaunayTriangulation<Point2<f64>> =
        ConstrainedDelaunayTriangulation::new();
    let mut handles: Vec<Option<spade::handles::FixedVertexHandle>> = Vec::new();

    // Contour points first, with their edges added as constraints so the
    // triangulation cannot cut across a boundary.
    for contour in &traced.contours {
        let start = uvs.len();
        for uv in contour {
            uvs.push(*uv);
            handles.push(cdt.insert(Point2::new(uv.x as f64, uv.y as f64)).ok());
        }
        let end = uvs.len();
        for i in start..end {
            let j = if i + 1 == end { start } else { i + 1 };
            if let (Some(a), Some(b)) = (handles[i], handles[j])
                && a != b
                && cdt.can_add_constraint(a, b)
            {
                cdt.add_constraint(a, b);
            }
        }
    }
    for uv in &traced.interior {
        uvs.push(*uv);
        handles.push(cdt.insert(Point2::new(uv.x as f64, uv.y as f64)).ok());
    }

    let index_of: std::collections::HashMap<_, u32> = handles
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.map(|h| (h, i as u32)))
        .collect();
    let index_of = |handle: spade::handles::FixedVertexHandle| index_of.get(&handle).copied();
    let triangles: Vec<[u32; 3]> = cdt
        .inner_faces()
        .filter_map(|face| {
            let [a, b, c] = face.vertices();
            let tri = [index_of(a.fix())?, index_of(b.fix())?, index_of(c.fix())?];
            let centroid =
                (uvs[tri[0] as usize] + uvs[tri[1] as usize] + uvs[tri[2] as usize]) / 3.0;
            // Even-odd across every contour: inside the outer boundary and
            // outside each hole.
            let crossings = traced
                .contours
                .iter()
                .filter(|contour| contains(contour, centroid))
                .count();
            (crossings % 2 == 1).then_some(tri)
        })
        .collect();

    let vertices = uvs.iter().map(|uv| to_local(*uv)).collect();
    (vertices, uvs, triangles)
}

/// Marching-squares contour following over a binary mask.
///
/// Walks every boundary — the outer silhouette and each hole — by stepping
/// between cells and turning according to the four-corner pattern, which is what
/// makes holes fall out for free rather than needing a separate pass.
fn find_contours(mask: &[bool], width: usize, height: usize) -> Vec<Vec<Vec2>> {
    let solid = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
            return false;
        }
        mask[y as usize * width + x as usize]
    };
    // Corner case index for the cell whose top-left corner is (x, y).
    let case_at = |x: isize, y: isize| -> u8 {
        (solid(x - 1, y - 1) as u8)
            | ((solid(x, y - 1) as u8) << 1)
            | ((solid(x - 1, y) as u8) << 2)
            | ((solid(x, y) as u8) << 3)
    };

    let mut visited = vec![false; (width + 1) * (height + 1)];
    let mut contours = Vec::new();

    for start_y in 0..=height as isize {
        for start_x in 0..=width as isize {
            let case = case_at(start_x, start_y);
            if case == 0 || case == 15 {
                continue;
            }
            let key = start_y as usize * (width + 1) + start_x as usize;
            if visited[key] {
                continue;
            }

            let mut contour = Vec::new();
            let (mut x, mut y) = (start_x, start_y);
            let (mut dx, mut dy) = (0isize, 0isize);
            // A contour cannot be longer than the perimeter of every cell.
            let limit = (width + 1) * (height + 1) * 4;
            for _ in 0..limit {
                let index = y as usize * (width + 1) + x as usize;
                if x >= 0 && y >= 0 && x <= width as isize && y <= height as isize {
                    visited[index] = true;
                }
                contour.push(Vec2::new(x as f32, y as f32));

                let case = case_at(x, y);
                let (nx, ny) = match case {
                    1 | 5 | 13 => (0, -1),
                    2 | 3 | 7 => (1, 0),
                    4 | 12 | 14 => (-1, 0),
                    8 | 10 | 11 => (0, 1),
                    // Saddles: keep turning the same way so the walk is
                    // consistent and terminates.
                    6 => {
                        if dx == 0 && dy == -1 {
                            (-1, 0)
                        } else {
                            (1, 0)
                        }
                    }
                    9 => {
                        if dx == 1 && dy == 0 {
                            (0, -1)
                        } else {
                            (0, 1)
                        }
                    }
                    _ => break,
                };
                dx = nx;
                dy = ny;
                x += dx;
                y += dy;
                if x == start_x && y == start_y {
                    break;
                }
            }

            if contour.len() >= 3 {
                contours.push(contour);
            }
        }
    }
    contours
}

/// Fill a traced silhouette with interior points (the "Refine" step).
///
/// Separate from [`trace`] because they answer different questions: tracing
/// decides the *shape*, refining decides how finely it can bend. Re-running the
/// trace to change interior density would throw the outline away with it.
pub fn refine(traced: &Traced, options: TraceOptions) -> Traced {
    // Work in the shape's own scale: contours are normalized, so a spacing
    // expressed against the bounding box behaves the same on any image.
    let (min, max) = contour_bounds(&traced.contours);
    let extent = (max - min).max_element().max(1e-3);
    let spacing = options.interior_spacing(extent);

    let interior = interior_points(&traced.contours, min, max, spacing);
    Traced {
        contours: traced.contours.clone(),
        interior,
    }
}

fn contour_bounds(contours: &[Vec<Vec2>]) -> (Vec2, Vec2) {
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for contour in contours {
        for p in contour {
            min = min.min(*p);
            max = max.max(*p);
        }
    }
    if min.x > max.x {
        (Vec2::ZERO, Vec2::ONE)
    } else {
        (min, max)
    }
}

/// Douglas-Peucker where concave corners are held to a tighter tolerance.
///
/// A plain simplification flattens notches first — they are exactly the points
/// whose deviation is easiest to write off — and those are the places a
/// deforming mesh most needs geometry. `concavity` scales how much harder a
/// concave point has to argue to be removed.
fn simplify_with_concavity(points: &[Vec2], tolerance: f32, concavity: f32) -> Vec<Vec2> {
    if points.len() < 4 {
        return points.to_vec();
    }
    let simplified = simplify(points, tolerance);
    let strength = (concavity.clamp(0.0, 100.0) / 100.0) * 0.9;
    if strength <= 0.01 {
        return simplified;
    }

    // Re-admit dropped concave points whose deviation clears the reduced bar.
    let orientation = area(points).signum();
    let kept: std::collections::HashSet<(i32, i32)> = simplified
        .iter()
        .map(|p| ((p.x * 100.0) as i32, (p.y * 100.0) as i32))
        .collect();

    let reduced = tolerance * (1.0 - strength);
    let mut out: Vec<Vec2> = Vec::with_capacity(simplified.len());
    let mut cursor = 0usize;
    for (i, p) in points.iter().enumerate() {
        let key = ((p.x * 100.0) as i32, (p.y * 100.0) as i32);
        if kept.contains(&key) {
            out.push(*p);
            cursor = i;
            continue;
        }
        // Concave here means the outline turns *against* its overall winding.
        let prev = points[(i + points.len() - 1) % points.len()];
        let next = points[(i + 1) % points.len()];
        let turn = (*p - prev).perp_dot(next - *p);
        let concave = turn.signum() != orientation && turn.abs() > 1e-6;
        if concave && distance_to_segment(*p, points[cursor], next) > reduced {
            out.push(*p);
            cursor = i;
        }
    }
    if out.len() >= 3 { out } else { simplified }
}

/// Slide each chosen vertex along the source contour to where it describes the
/// silhouette best — the "Refinement" dial (T-402).
///
/// Douglas-Peucker is greedy: it keeps whichever point deviated most from the
/// span it was splitting, which is not the point that leaves the *least* error
/// once its neighbours are also fixed. This walks each vertex over the source
/// points between its neighbours and takes the position with the lowest total
/// deviation, repeating until nothing moves. Vertex count never changes — that
/// is `detail`'s job — so more refinement costs time, not geometry.
///
/// Returns `chosen` untouched when the budget is zero or the vertices cannot be
/// traced back to the source (they can't be after `make_uniform` invents points,
/// which is why this runs first).
fn optimize_placement(original: &[Vec2], chosen: &[Vec2], passes: usize) -> Vec<Vec2> {
    if passes == 0 || chosen.len() < 3 || original.len() <= chosen.len() {
        return chosen.to_vec();
    }
    let key = |p: &Vec2| (p.x.to_bits(), p.y.to_bits());
    let lookup: std::collections::HashMap<(u32, u32), usize> = original
        .iter()
        .enumerate()
        .map(|(i, p)| (key(p), i))
        .collect();
    let Some(mut indices) = chosen
        .iter()
        .map(|p| lookup.get(&key(p)).copied())
        .collect::<Option<Vec<usize>>>()
    else {
        return chosen.to_vec();
    };
    // Douglas-Peucker walks the ring in order, so the indices should already
    // ascend; anything else means the caller changed and the neighbour spans
    // below would be nonsense.
    if indices.windows(2).any(|w| w[0] >= w[1]) {
        return chosen.to_vec();
    }

    // Cost of routing the source points from `from` to `to` through `mid`: how
    // far the real outline strays from the two segments that replace it.
    let span_cost = |from: usize, mid: usize, to: usize| -> f32 {
        let (a, b, c) = (original[from], original[mid], original[to]);
        let step = ((to - from) / 64).max(1); // Sample long spans; exact is not worth the wait.
        let mut cost = 0.0;
        let mut i = from + 1;
        while i < to {
            let p = original[i];
            let d = if i <= mid {
                distance_to_segment(p, a, b)
            } else {
                distance_to_segment(p, b, c)
            };
            cost += d * d;
            i += step;
        }
        cost
    };

    for _ in 0..passes {
        let mut moved = false;
        // The first and last vertices are left alone: their spans wrap the seam,
        // and a wrapped relocation can reorder the ring. One pinned vertex costs
        // nothing visible; a self-crossing outline costs the whole trace.
        for k in 1..indices.len() - 1 {
            let (prev, next) = (indices[k - 1], indices[k + 1]);
            if next - prev < 3 {
                continue;
            }
            let step = ((next - prev) / 64).max(1);
            let mut best = (indices[k], span_cost(prev, indices[k], next));
            let mut candidate = prev + 1;
            while candidate < next {
                let cost = span_cost(prev, candidate, next);
                if cost < best.1 {
                    best = (candidate, cost);
                }
                candidate += step;
            }
            if best.0 != indices[k] {
                indices[k] = best.0;
                moved = true;
            }
        }
        // Converged: another pass would repeat this one exactly.
        if !moved {
            break;
        }
    }

    indices.iter().map(|&i| original[i]).collect()
}

/// Even out the spacing of a closed ring, blended by `amount` (0 = untouched).
///
/// Regular spacing deforms more predictably — a long edge between two vertices
/// is a hinge that cannot bend — at the cost of following fine detail, which is
/// why it is a dial rather than a rule.
fn make_uniform(points: &[Vec2], amount: f32) -> Vec<Vec2> {
    let amount = amount.clamp(0.0, 1.0);
    if amount <= 0.01 || points.len() < 4 {
        return points.to_vec();
    }
    let perimeter: f32 = (0..points.len())
        .map(|i| (points[(i + 1) % points.len()] - points[i]).length())
        .sum();
    if perimeter < 1e-4 {
        return points.to_vec();
    }
    let target = perimeter / points.len() as f32;

    let mut out = Vec::with_capacity(points.len());
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        out.push(a);
        // Split edges that are much longer than the average; the blend keeps
        // some of the original irregularity at low `amount`.
        let length = (b - a).length();
        if length > target * 1.6 {
            let splits = ((length / target) as usize).min(8);
            for s in 1..splits {
                let t = s as f32 / splits as f32;
                out.push(a.lerp(b, t * amount + t * (1.0 - amount)));
            }
        }
    }
    out.dedup_by(|a, b| (*a - *b).length() < 1e-4);
    out
}

/// Douglas-Peucker, applied to a closed ring.
fn simplify(points: &[Vec2], tolerance: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    fn recurse(points: &[Vec2], tolerance: f32, out: &mut Vec<Vec2>) {
        let (first, last) = (points[0], points[points.len() - 1]);
        let mut worst = (0usize, 0.0f32);
        for (i, p) in points.iter().enumerate().skip(1).take(points.len() - 2) {
            let d = distance_to_segment(*p, first, last);
            if d > worst.1 {
                worst = (i, d);
            }
        }
        if worst.1 > tolerance {
            recurse(&points[..=worst.0], tolerance, out);
            out.pop();
            recurse(&points[worst.0..], tolerance, out);
        } else {
            out.push(first);
            out.push(last);
        }
    }
    let mut out = Vec::new();
    recurse(points, tolerance, &mut out);
    out.dedup_by(|a, b| (*a - *b).length() < 1e-4);
    out
}

/// Signed area — positive for counter-clockwise, which tells a hole from the
/// outer boundary.
fn area(points: &[Vec2]) -> f32 {
    let mut sum = 0.0;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        sum += a.perp_dot(b);
    }
    sum * 0.5
}

/// Move every point of a ring along its outward normal.
fn offset_polygon(points: &[Vec2], amount: f32) -> Vec<Vec2> {
    if amount.abs() < 1e-4 || points.len() < 3 {
        return points.to_vec();
    }
    let centre = points.iter().copied().sum::<Vec2>() / points.len() as f32;
    points
        .iter()
        .map(|p| {
            let direction = (*p - centre).normalize_or_zero();
            *p + direction * amount
        })
        .collect()
}

/// A grid of points inside the silhouette, for meshes that must bend in the
/// middle.
/// A grid of points inside the silhouette, spanning `min..max`.
fn interior_points(contours: &[Vec<Vec2>], min: Vec2, max: Vec2, spacing: f32) -> Vec<Vec2> {
    // Guard the grid size regardless of what was asked for: a fine spacing on a
    // large area is a million candidate points, each tested against every
    // contour, which reads as a hang.
    let extent = max - min;
    let spacing = spacing.max(1e-4);
    let cells = (extent.x / spacing) * (extent.y / spacing);
    let spacing = if cells > MAX_MESH_POINTS as f32 {
        spacing * (cells / MAX_MESH_POINTS as f32).sqrt()
    } else {
        spacing
    };

    let mut points = Vec::new();
    let mut y = min.y + spacing;
    while y < max.y {
        let mut x = min.x + spacing;
        while x < max.x {
            let p = Vec2::new(x, y);
            let inside = contours.iter().filter(|c| contains(c, p)).count() % 2 == 1;
            // Keep clear of the boundary: a point sitting on a constraint edge
            // makes for slivers.
            if inside
                && contours
                    .iter()
                    .all(|c| distance_to_ring(c, p) > spacing * 0.5)
            {
                points.push(p);
            }
            x += spacing;
        }
        y += spacing;
    }
    points
}

fn distance_to_ring(ring: &[Vec2], p: Vec2) -> f32 {
    let mut best = f32::MAX;
    for i in 0..ring.len() {
        let a = ring[i];
        let b = ring[(i + 1) % ring.len()];
        best = best.min(distance_to_segment(p, a, b));
    }
    best
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq < 1e-9 {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// Even-odd point-in-polygon test.
fn contains(polygon: &[Vec2], point: Vec2) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[j]);
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

/// Index of the edge (as a vertex-pair position) nearest to `point`, and the
/// closest point on it — for inserting a vertex by clicking an edge.
pub fn nearest_edge(mesh: &MeshAttachment, point: Vec2) -> Option<(usize, usize, Vec2, f32)> {
    let mut best: Option<(usize, usize, Vec2, f32)> = None;
    for tri in &mesh.triangles {
        for k in 0..3 {
            let (i, j) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
            let (Some(&a), Some(&b)) = (mesh.setup_vertices.get(i), mesh.setup_vertices.get(j))
            else {
                continue;
            };
            let ab = b - a;
            let len_sq = ab.length_squared();
            let t = if len_sq < 1e-9 {
                0.0
            } else {
                ((point - a).dot(ab) / len_sq).clamp(0.0, 1.0)
            };
            let closest = a + ab * t;
            let distance = (point - closest).length();
            if best.is_none_or(|(_, _, _, d)| distance < d) {
                best = Some((i, j, closest, distance));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::attachment::{Rect, RegionAttachment};

    fn quad_mesh() -> MeshAttachment {
        let region = RegionAttachment {
            texture: "img".into(),
            local_offset: Vec2::ZERO,
            local_rotation: 0.0,
            local_scale: Vec2::ONE,
            width: 100.0,
            height: 100.0,
            uv_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            pivot: Vec2::splat(0.5),
        };
        MeshAttachment::from_region(&region)
    }

    #[test]
    fn a_converted_quad_keeps_its_corners_and_uvs() {
        let mesh = quad_mesh();
        assert_eq!(mesh.setup_vertices.len(), 4);
        assert_eq!(mesh.triangles.len(), 2);
        // Corners span the region's size, centred on its pivot.
        let (min, max) = mesh.bounds();
        assert_eq!(min, Vec2::new(-50.0, -50.0));
        assert_eq!(max, Vec2::new(50.0, 50.0));
        // The top-left corner samples the top-left of the texture.
        assert_eq!(mesh.uvs[0], Vec2::new(0.0, 0.0));
    }

    #[test]
    fn retriangulating_a_quad_gives_two_triangles() {
        let mut mesh = quad_mesh();
        mesh.triangles.clear();
        retriangulate(&mut mesh);
        assert_eq!(mesh.triangles.len(), 2);
    }

    /// Documented limitation: with no outline to consult, a concave silhouette
    /// is triangulated across its opening. Visible and fixable by deleting a
    /// triangle; T-402's tracer will supply a real contour.
    #[test]
    fn a_concave_outline_is_filled_to_the_hull() {
        let mut mesh = quad_mesh();
        // An arrowhead: the notch at the bottom gets covered.
        mesh.setup_vertices = vec![
            Vec2::new(0.0, 50.0),
            Vec2::new(-50.0, -50.0),
            Vec2::new(0.0, -10.0),
            Vec2::new(50.0, -50.0),
        ];
        mesh.uvs = vec![Vec2::ZERO; 4];
        retriangulate(&mut mesh);
        // The notch point sits inside the hull of the other three, so Delaunay
        // fans around it: three triangles, one of them covering the notch.
        assert_eq!(mesh.triangles.len(), 3, "{:?}", mesh.triangles);
    }

    /// The case that killed the outline heuristic: a vertex added inside a quad
    /// must fan into four triangles, not lose one to a phantom notch.
    #[test]
    fn an_interior_vertex_does_not_break_the_fan() {
        let mut mesh = quad_mesh();
        mesh.setup_vertices.push(Vec2::ZERO);
        mesh.uvs.push(Vec2::splat(0.5));
        retriangulate(&mut mesh);
        assert_eq!(
            mesh.triangles.len(),
            4,
            "quad + centre = four triangles: {:?}",
            mesh.triangles
        );
    }

    #[test]
    fn too_few_vertices_yields_no_triangles() {
        let mut mesh = quad_mesh();
        mesh.setup_vertices.truncate(2);
        retriangulate(&mut mesh);
        assert!(mesh.triangles.is_empty());
    }

    // ── Tracing (T-402) ──────────────────────────────────────────────────

    /// A filled disc, or a ring when `inner` is non-zero. Generated rather than
    /// checked in: the shape's exact geometry is what the assertions reason
    /// about, and a binary asset would hide it.
    fn donut(size: u32, outer: f32, inner: f32) -> image::RgbaImage {
        let centre = size as f32 / 2.0;
        image::RgbaImage::from_fn(size, size, |x, y| {
            let d = ((x as f32 + 0.5 - centre).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
            let solid = d <= outer && d >= inner;
            image::Rgba([255, 255, 255, if solid { 255 } else { 0 }])
        })
    }

    #[test]
    fn tracing_a_disc_finds_one_contour() {
        let image = donut(64, 28.0, 0.0);
        let traced = trace(&image, TraceOptions::default()).expect("a shape");
        assert_eq!(traced.contours.len(), 1, "just the silhouette");
        assert!(
            traced.contours[0].len() >= 8,
            "a circle needs more than a few points: {}",
            traced.contours[0].len()
        );
        // Normalized: every point inside the unit square.
        for p in &traced.contours[0] {
            assert!(
                (-0.1..=1.1).contains(&p.x) && (-0.1..=1.1).contains(&p.y),
                "{p:?}"
            );
        }
    }

    /// The acceptance case: a ring must keep its hole, which is what separates a
    /// real tracer from a bounding-box one.
    #[test]
    fn tracing_a_donut_preserves_the_hole() {
        let image = donut(96, 44.0, 18.0);
        let traced = trace(&image, TraceOptions::default()).expect("a shape");
        assert_eq!(
            traced.contours.len(),
            2,
            "outer boundary plus one hole, got {}",
            traced.contours.len()
        );

        let (vertices, uvs, triangles) =
            mesh_from_trace(&traced, (Vec2::splat(-50.0), Vec2::splat(50.0)));
        assert_eq!(vertices.len(), uvs.len(), "one UV per vertex");
        assert!(!triangles.is_empty(), "the ring triangulated");

        // No triangle may cover the middle of the hole.
        let hole_centre = Vec2::splat(0.5);
        for tri in &triangles {
            let a = uvs[tri[0] as usize];
            let b = uvs[tri[1] as usize];
            let c = uvs[tri[2] as usize];
            let centroid = (a + b + c) / 3.0;
            assert!(
                (centroid - hole_centre).length() > 0.10,
                "triangle {tri:?} sits in the hole at {centroid:?}"
            );
        }
    }

    /// Regression: the settings used to be raw pixel values, so a fine spacing
    /// on a large image asked for ~a million points and read as a hang. Now the
    /// dials are relative to the shape and the grid is capped besides.
    #[test]
    fn maximum_settings_on_a_big_image_stay_bounded() {
        let image = donut(512, 240.0, 0.0);
        let options = TraceOptions {
            detail: 100.0,
            concavity: 100.0,
            refinement: 100.0,
            interior: 100.0,
            ..TraceOptions::default()
        };
        let traced = refine(&trace(&image, options).expect("a shape"), options);

        let total: usize =
            traced.contours.iter().map(|c| c.len()).sum::<usize>() + traced.interior.len();
        assert!(
            total <= MAX_MESH_POINTS,
            "traced {total} points, budget is {MAX_MESH_POINTS}"
        );
        assert!(!traced.interior.is_empty(), "still got interior points");

        // And it triangulates in reasonable time rather than crawling.
        let (vertices, _, triangles) =
            mesh_from_trace(&traced, (Vec2::splat(-50.0), Vec2::splat(50.0)));
        assert_eq!(vertices.len(), total);
        assert!(!triangles.is_empty());
    }

    /// Refinement is Spine's "time and effort to spend finding an optimal
    /// solution": it moves vertices, it does not add them. It used to control
    /// interior density, which is a different dial entirely (now `interior`).
    #[test]
    fn refinement_improves_placement_without_changing_the_vertex_count() {
        let image = donut(96, 40.0, 0.0);
        let coarse = TraceOptions {
            refinement: 0.0,
            uniform: 0.0,
            ..TraceOptions::default()
        };
        let fine = TraceOptions {
            refinement: 100.0,
            ..coarse
        };
        let a = trace(&image, coarse).expect("a shape");
        let b = trace(&image, fine).expect("a shape");

        assert_eq!(
            a.contours[0].len(),
            b.contours[0].len(),
            "refinement must not change how many vertices surround the shape"
        );
        assert!(
            a.interior.is_empty() && b.interior.is_empty(),
            "outline only"
        );

        // A polygon inscribed in a disc always under-covers it, so the better
        // placement is simply the one that loses less area.
        let covered = |t: &Traced| area(&t.contours[0]).abs();
        assert!(
            covered(&b) >= covered(&a),
            "refined outline covers {:.5}, coarse covers {:.5}",
            covered(&b),
            covered(&a)
        );
    }

    /// The renamed dial still drives interior density, and only that.
    #[test]
    fn interior_density_controls_the_refine_step() {
        let image = donut(96, 40.0, 0.0);
        let sparse = TraceOptions {
            interior: 0.0,
            ..TraceOptions::default()
        };
        let dense = TraceOptions {
            interior: 100.0,
            ..TraceOptions::default()
        };
        let outline = trace(&image, sparse).expect("a shape");
        let few = refine(&outline, sparse);
        let many = refine(&outline, dense);

        assert!(
            many.interior.len() > few.interior.len(),
            "{} interior points at full density vs {} at none",
            many.interior.len(),
            few.interior.len()
        );
        assert_eq!(
            few.contours[0].len(),
            many.contours[0].len(),
            "refining must leave the outline alone"
        );
    }

    #[test]
    fn a_fully_transparent_image_traces_to_nothing() {
        let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 0]));
        assert!(trace(&image, TraceOptions::default()).is_none());
    }

    #[test]
    fn simplify_collapses_a_straight_run() {
        let line: Vec<Vec2> = (0..20).map(|i| Vec2::new(i as f32, 0.0)).collect();
        let simplified = simplify(&line, 0.5);
        assert_eq!(simplified.len(), 2, "a straight line is two points");
    }

    /// Tracing gives the outline; refining fills it. Keeping them apart is what
    /// lets interior density change without re-cutting the silhouette.
    #[test]
    fn tracing_gives_an_outline_and_refining_fills_it() {
        let image = donut(64, 30.0, 0.0);
        let options = TraceOptions::default();
        let outline = trace(&image, options).expect("a shape");
        assert!(
            outline.interior.is_empty(),
            "a trace is the outline only, got {} interior points",
            outline.interior.len()
        );

        let traced = refine(&outline, options);
        assert_eq!(
            traced.contours.len(),
            outline.contours.len(),
            "refining leaves the outline alone"
        );
        assert!(!traced.interior.is_empty(), "the disc got interior points");
        // Both sides normalized: `traced` is entirely in UV space by this point.
        for p in &traced.interior {
            let inside = traced.contours.iter().filter(|c| contains(c, *p)).count() % 2 == 1;
            assert!(inside, "interior point {p:?} escaped the silhouette");
        }
    }

    #[test]
    fn nearest_edge_finds_the_side_under_the_cursor() {
        let mesh = quad_mesh();
        // Just outside the left edge, halfway up.
        let (i, j, closest, distance) =
            nearest_edge(&mesh, Vec2::new(-52.0, 0.0)).expect("an edge");
        assert!(distance < 3.0, "distance {distance}");
        assert!((closest.x + 50.0).abs() < 1e-3, "closest {closest:?}");
        let ends = [mesh.setup_vertices[i], mesh.setup_vertices[j]];
        assert!(
            ends.iter().all(|v| (v.x + 50.0).abs() < 1e-3),
            "both ends on the left edge: {ends:?}"
        );
    }
}
