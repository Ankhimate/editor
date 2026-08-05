//! Artwork outlines for hover and selection (T-708).
//!
//! Two lines, saying two different things:
//!
//! * a **solid** line around what the pixels actually cover — the alpha
//!   silhouette, holes included;
//! * a **dashed** line around the mesh, when the attachment has one.
//!
//! Both matter for picking. A slot's quad is nearly always bigger than the art
//! inside it, so a bounding box tells you almost nothing about whether the thing
//! you are about to click is the arm or the sleeve behind it. The silhouette is
//! the shape you can see, which is the shape you think you are clicking.
//!
//! The outline follows deformation: for a mesh it is mapped through the
//! triangles, so a bent limb has a bent outline rather than a rectangle drawn
//! around where the art used to be.

use crate::app_state::AppState;
use ankhimate_core::attachment::Attachment;
use ankhimate_core::ids::SlotId;
use eframe::egui;

/// Alpha at or above this counts as covered. Matches the auto-tracer's default
/// so an outline and a traced mesh agree about where the art ends.
const ALPHA_THRESHOLD: u8 = 8;

/// Dash and gap length in screen pixels.
const DASH: f32 = 5.0;
const GAP: f32 = 4.0;

/// Draw the hover and selection outlines. Selection wins when both name the same
/// attachment — a highlight that fights itself is worse than either alone.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    state: &AppState,
    theme: &crate::theme::Theme,
) {
    let selected = match &state.session.selection {
        Some(crate::session::Selection::Attachment { slot, name }) => Some((*slot, name.clone())),
        _ => None,
    };
    let hovered = state
        .session
        .hovered_attachment
        .clone()
        .filter(|h| selected.as_ref() != Some(h));

    if let Some((slot, name)) = hovered {
        outline_one(painter, rect, state, slot, &name, theme.outline_hover());
    }
    if let Some((slot, name)) = selected {
        outline_one(painter, rect, state, slot, &name, theme.outline_selected());
    }
}

/// Decode and trace whatever the cursor and the selection are pointing at.
///
/// Runs from the tool pass, which owns `&mut`, so painting stays a read. Tracing
/// decodes a PNG and walks every boundary in it — once per asset, never per
/// frame, and never for art nobody is looking at.
pub fn warm_cache(state: &mut AppState) {
    let mut wanted: Vec<(SlotId, String)> = Vec::new();
    if let Some(hovered) = state.session.hovered_attachment.clone() {
        wanted.push(hovered);
    }
    if let Some(crate::session::Selection::Attachment { slot, name }) = &state.session.selection {
        wanted.push((*slot, name.clone()));
    }
    let skins = state.session.skin_stack();
    let textures: Vec<String> = wanted
        .into_iter()
        .filter_map(
            |(slot, name)| match state.doc.skeleton.resolve_many(&skins, slot, &name)? {
                Attachment::Region(r) => Some(r.texture.clone()),
                Attachment::Mesh(m) => Some(
                    state
                        .doc
                        .skeleton
                        .resolve_linked_mesh(&skins, m)
                        .texture
                        .clone(),
                ),
                _ => None,
            },
        )
        .collect();
    for texture in textures {
        if state.session.silhouettes.contains_key(&texture) {
            continue;
        }
        let contours = state
            .doc
            .assets
            .by_name(&texture)
            .and_then(|id| state.doc.assets.get(id))
            .and_then(|asset| image::load_from_memory(&asset.bytes).ok())
            .map(|image| crate::meshgen::silhouette(&image.to_rgba8(), ALPHA_THRESHOLD))
            // An asset that will not decode gets an empty outline rather than a
            // retry every frame for the rest of the session.
            .unwrap_or_default();
        state.session.silhouettes.insert(texture, contours);
    }
}

fn outline_one(
    painter: &egui::Painter,
    rect: egui::Rect,
    state: &AppState,
    slot_id: SlotId,
    name: &str,
    color: egui::Color32,
) {
    let skins = state.session.skin_stack();
    let Some(attachment) = state
        .doc
        .skeleton
        .resolve_many(&skins, slot_id, name)
        .cloned()
    else {
        return;
    };
    let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
        return;
    };
    let Some(bone_world) = state.pose.worlds.get(slot.bone).copied() else {
        return;
    };
    let to_screen = |v: glam::Vec2| crate::ui::canvas::camera::world_to_screen(v, rect, state);

    match &attachment {
        Attachment::Region(region) => {
            let Some(contours) = silhouette_for(state, &region.texture) else {
                return;
            };
            let corners = region.local_corners();
            let (w, h) = image_size(state, &region.texture).unwrap_or((1.0, 1.0));
            // The quad's corners are TL, BL, BR, TR; a pixel at (u, v) with v
            // running down maps by bilinear interpolation between them.
            let map = |p: glam::Vec2| {
                let (u, v) = (p.x / w, p.y / h);
                let top = corners[0].lerp(corners[3], u);
                let bottom = corners[1].lerp(corners[2], u);
                bone_world.transform_point(top.lerp(bottom, v))
            };
            for contour in contours {
                let points: Vec<egui::Pos2> = contour.iter().map(|p| to_screen(map(*p))).collect();
                stroke_loop(painter, &points, color, false);
            }
        }
        Attachment::Mesh(mesh) => {
            let geometry = state.doc.skeleton.resolve_linked_mesh(&skins, mesh).clone();
            let world = mesh_world_vertices(state, slot_id, &geometry, &bone_world);

            // The mesh outline: every edge belonging to exactly one triangle.
            // Dashed, because it is authoring structure rather than a thing the
            // player will ever see.
            let boundary = boundary_edges(&geometry.triangles);
            for (a, b) in boundary {
                let (Some(a), Some(b)) = (world.get(a as usize), world.get(b as usize)) else {
                    continue;
                };
                dashed_segment(painter, to_screen(*a), to_screen(*b), color);
            }

            // The pixel silhouette, carried through the triangles so it deforms
            // with the mesh.
            let Some(contours) = silhouette_for(state, &geometry.texture) else {
                return;
            };
            let (w, h) = image_size(state, &geometry.texture).unwrap_or((1.0, 1.0));
            for contour in contours {
                let mut run: Vec<egui::Pos2> = Vec::new();
                for p in contour.iter() {
                    let uv = glam::vec2(p.x / w, p.y / h);
                    match map_through_mesh(&geometry, &world, uv) {
                        Some(point) => run.push(to_screen(point)),
                        // A silhouette point outside every triangle means the
                        // mesh does not cover that part of the image. Break the
                        // run rather than bridge the gap with a false edge.
                        None => {
                            stroke_loop(painter, &run, color, true);
                            run.clear();
                        }
                    }
                }
                stroke_loop(painter, &run, color, true);
            }
        }
        _ => {}
    }
}

/// Contours for an asset, traced by [`warm_cache`] before painting.
fn silhouette_for<'a>(state: &'a AppState, texture: &str) -> Option<&'a Vec<Vec<glam::Vec2>>> {
    state.session.silhouettes.get(texture)
}

fn image_size(state: &AppState, texture: &str) -> Option<(f32, f32)> {
    let id = state.doc.assets.by_name(texture)?;
    state
        .doc
        .assets
        .get(id)
        .map(|a| (a.width as f32, a.height as f32))
}

/// Where each mesh vertex ends up, deform and skinning included.
fn mesh_world_vertices(
    state: &AppState,
    slot_id: SlotId,
    mesh: &ankhimate_core::attachment::MeshAttachment,
    bone_world: &ankhimate_core::transforms::Affine2,
) -> Vec<glam::Vec2> {
    let name = state.pose.attachment_name(&state.doc.skeleton, slot_id);
    let deform = name.and_then(|n| state.pose.deforms.get(&(slot_id, n.to_string())));
    // No `skinned` branch: `skin_vertex_with_ffd` falls back to rigid placement
    // through `bone_world` for any vertex without usable influences, so an
    // unweighted mesh and an unweighted vertex both come out right.
    (0..mesh.setup_vertices.len())
        .map(|i| {
            let offset = deform.and_then(|d| d.get(i).copied()).unwrap_or_default();
            mesh.skin_vertex_with_ffd(i, offset, &state.pose, bone_world)
        })
        .collect()
}

/// Edges used by exactly one triangle — the mesh's own boundary.
fn boundary_edges(triangles: &[[u32; 3]]) -> Vec<(u32, u32)> {
    use std::collections::HashMap;
    let mut counts: HashMap<(u32, u32), (usize, (u32, u32))> = HashMap::new();
    for t in triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            // Keyed on the sorted pair so the two windings of a shared edge meet.
            let key = if a < b { (a, b) } else { (b, a) };
            let entry = counts.entry(key).or_insert((0, (a, b)));
            entry.0 += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, (count, _))| *count == 1)
        .map(|(_, (_, edge))| edge)
        .collect()
}

/// Carry a point in UV space through the mesh to where it is drawn.
///
/// Finds the triangle whose UVs contain the point, then applies the same
/// barycentric weights to that triangle's world vertices. Linear per triangle,
/// which is exactly how the texture is sampled, so the outline lands on the same
/// pixels the renderer does.
fn map_through_mesh(
    mesh: &ankhimate_core::attachment::MeshAttachment,
    world: &[glam::Vec2],
    uv: glam::Vec2,
) -> Option<glam::Vec2> {
    for t in &mesh.triangles {
        let (ia, ib, ic) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let (Some(a), Some(b), Some(c)) = (mesh.uvs.get(ia), mesh.uvs.get(ib), mesh.uvs.get(ic))
        else {
            continue;
        };
        let area = (*b - *a).perp_dot(*c - *a);
        if area.abs() < 1e-9 {
            continue;
        }
        let w0 = (*b - uv).perp_dot(*c - uv) / area;
        let w1 = (*c - uv).perp_dot(*a - uv) / area;
        let w2 = 1.0 - w0 - w1;
        // A small negative tolerance keeps a point sitting exactly on a shared
        // edge from falling through both triangles.
        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
            continue;
        }
        let (Some(wa), Some(wb), Some(wc)) = (world.get(ia), world.get(ib), world.get(ic)) else {
            continue;
        };
        return Some(*wa * w0 + *wb * w1 + *wc * w2);
    }
    None
}

/// Stroke a polyline, optionally leaving it open.
fn stroke_loop(painter: &egui::Painter, points: &[egui::Pos2], color: egui::Color32, open: bool) {
    if points.len() < 2 {
        return;
    }
    let stroke = egui::Stroke::new(1.5, color);
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
    if !open {
        painter.line_segment([points[points.len() - 1], points[0]], stroke);
    }
}

/// One dashed line. egui strokes solid, so the dashes are cut here.
fn dashed_segment(painter: &egui::Painter, from: egui::Pos2, to: egui::Pos2, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.0, color.gamma_multiply(0.85));
    let span = to - from;
    let length = span.length();
    if length < 1e-3 {
        return;
    }
    let step = DASH + GAP;
    let direction = span / length;
    let mut travelled = 0.0;
    // Bounded by the segment length; a zero step would have been caught above.
    while travelled < length {
        let end = (travelled + DASH).min(length);
        painter.line_segment(
            [from + direction * travelled, from + direction * end],
            stroke,
        );
        travelled += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::attachment::MeshAttachment;

    #[test]
    fn a_quad_has_four_boundary_edges_and_no_diagonal() {
        // Two triangles sharing the 0-2 diagonal: the diagonal is interior, so
        // outlining it would draw a line across the middle of the art.
        let edges = boundary_edges(&[[0, 1, 2], [0, 2, 3]]);
        assert_eq!(edges.len(), 4);
        let sorted: Vec<(u32, u32)> = edges
            .iter()
            .map(|(a, b)| if a < b { (*a, *b) } else { (*b, *a) })
            .collect();
        assert!(!sorted.contains(&(0, 2)), "the shared diagonal is interior");
    }

    #[test]
    fn uv_maps_through_the_triangle_it_lands_in() {
        let mesh = MeshAttachment {
            uvs: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(1.0, 0.0),
                glam::vec2(1.0, 1.0),
            ],
            triangles: vec![[0, 1, 2]],
            ..Default::default()
        };
        // The same triangle, moved and doubled in world space.
        let world = vec![
            glam::vec2(10.0, 10.0),
            glam::vec2(12.0, 10.0),
            glam::vec2(12.0, 12.0),
        ];
        let centre = map_through_mesh(&mesh, &world, glam::vec2(0.66, 0.33)).unwrap();
        let expected = (world[0] + world[1] + world[2]) / 3.0;
        assert!((centre - expected).length() < 0.05, "{centre:?}");
    }

    #[test]
    fn a_uv_outside_every_triangle_has_nowhere_to_go() {
        let mesh = MeshAttachment {
            uvs: vec![
                glam::vec2(0.0, 0.0),
                glam::vec2(0.2, 0.0),
                glam::vec2(0.2, 0.2),
            ],
            triangles: vec![[0, 1, 2]],
            ..Default::default()
        };
        let world = vec![glam::Vec2::ZERO, glam::Vec2::X, glam::Vec2::ONE];
        assert!(map_through_mesh(&mesh, &world, glam::vec2(0.9, 0.9)).is_none());
    }
}
