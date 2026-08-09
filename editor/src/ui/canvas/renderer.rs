use crate::app_state::AppState;
use crate::renderer::{CustomCallback, MeshVertex, SpriteDraw, SpriteUpload};
use ankhimate_core::attachment::{Attachment, RegionAttachment};
use ankhimate_core::ids::SlotId;
use eframe::egui;

pub const BONE_WIDTH_RATIO: f32 = 0.015;

// ── Colors ──────────────────────────────────────────────────────────────

/// Alpha multiplier for artwork outside an isolation (T-903).
///
/// Low enough that the isolated limb is unmistakably the subject, high enough
/// that the rest of the character still reads as a silhouette to place it
/// against.
pub const ISOLATION_GHOST: f32 = 0.18;

pub const COLOR_BONE_NORMAL: [f32; 4] = [0.0, 0.78, 0.78, 0.90];
pub const COLOR_BONE_SELECTED: [f32; 4] = [1.0, 0.65, 0.0, 0.95];
pub const COLOR_BONE_HOVERED: [f32; 4] = [0.2, 0.95, 0.95, 0.95];
pub const COLOR_BONE_PREVIEW: [f32; 4] = [0.5, 1.0, 1.0, 0.85]; // Ghost preview

/// A bone's group colour, inherited from the nearest ancestor that set one
/// (T-505).
///
/// Inheritance rather than per-bone assignment: a group is a *limb*, and
/// colouring one means colouring the shoulder and having the arm follow.
/// A bone that has been given its own colour keeps it and passes that down.
///
/// A bone nobody has coloured — and no ancestor either — falls back to one
/// derived from its name rather than to the shared default. Sixty bones in one
/// teal is sixty bones you have to read the name of; distinct hues make the
/// hierarchy, the viewport and the weight list all answer "which bone is this"
/// at a glance. Setting a colour anywhere up the chain still overrides it, so
/// grouping a limb by hand works exactly as before.
pub fn group_color(
    skeleton: &ankhimate_core::skeleton::Skeleton,
    bone: ankhimate_core::ids::BoneId,
) -> [f32; 4] {
    let default = ankhimate_core::skeleton::Bone::default_color();
    let mut current = Some(bone);
    // Bounded by the hierarchy depth; a cycle is impossible by construction
    // (`update_order` is topologically sorted), but the counter makes that
    // assumption cheap to hold rather than load-bearing.
    for _ in 0..64 {
        let Some(id) = current else { break };
        let Some(b) = skeleton.bones.get(id) else {
            break;
        };
        if b.color != default {
            return b.color;
        }
        current = b.parent;
    }
    // Position in `update_order` — the rig's own stable ordering, parents before
    // children, so a limb's bones come out as a run of neighbouring hues rather
    // than scattered across the wheel.
    skeleton
        .update_order
        .iter()
        .position(|id| *id == bone)
        .map(ankhimate_core::skeleton::Bone::auto_color)
        .unwrap_or(default)
}

/// The fixed palette weight colours are drawn from, by rank.
///
/// Fixed, not derived. A generated hue sequence gives every rank a different
/// colour but never the *same* colour twice across sessions or meshes, so
/// "the sky-blue one is the first bone" is never learnable. These are constant:
/// rank 0 is always sky, rank 1 always pink, and reading a pie stops needing the
/// list open beside it.
///
/// Chosen to stay apart at the size these are actually read at — a weight pie a
/// dozen pixels across. Adjacent entries differ in hue *and* in lightness, so
/// neighbouring wedges separate even where the hue difference is small.
const RANK_PALETTE: [[f32; 4]; 12] = [
    [0.36, 0.72, 1.00, 1.0], // sky
    [1.00, 0.44, 0.72, 1.0], // pink
    [0.55, 0.87, 0.35, 1.0], // green
    [1.00, 0.71, 0.24, 1.0], // amber
    [0.69, 0.55, 1.00, 1.0], // violet
    [0.25, 0.84, 0.78, 1.0], // teal
    [1.00, 0.42, 0.36, 1.0], // coral
    [0.85, 0.82, 0.32, 1.0], // olive
    [0.45, 0.58, 0.95, 1.0], // indigo
    [0.95, 0.55, 0.95, 1.0], // orchid
    [0.40, 0.78, 0.58, 1.0], // jade
    [0.92, 0.60, 0.45, 1.0], // clay
];

/// The colour a rank from [`MeshAttachment::bound_bones`] maps to, if it has one.
///
/// `None` for a bone the mesh does not use. Deliberately not a fallback colour:
/// an unbound bone drawn in *any* colour reads as bound, and "which bones hold
/// this mesh" is the question the whole overlay exists to answer.
///
/// Past the palette's length the entries repeat. A mesh with more than twelve
/// influences is already past what a colour can disambiguate, and cycling keeps
/// the low ranks — the ones carrying the weight — on their fixed colours.
///
/// Takes a rank rather than a bone so a caller drawing hundreds of vertices can
/// rank the mesh's bones once and colour from that, instead of re-ranking per
/// lookup — `pie` runs per vertex and the quadratic version was visible as frame
/// time.
pub fn color_for_rank(rank: Option<usize>) -> Option<[f32; 4]> {
    rank.map(|r| RANK_PALETTE[r % RANK_PALETTE.len()])
}

pub fn bone_gizmo_vertices(
    origin: glam::Vec2,
    angle: f32,
    length: f32,
    zoom: f32,
) -> [glam::Vec2; 4] {
    let dir = glam::Vec2::new(angle.cos(), angle.sin());
    let perp = glam::Vec2::new(-dir.y, dir.x);

    let screen_length = length * zoom;
    // Visual width tapers with length but is clamped so tiny/huge bones stay legible.
    let screen_width = (screen_length * 0.15).clamp(4.0, 14.0);
    let screen_waist = (screen_length * 0.12).min(screen_width); // how far along the bone the "shoulders" sit

    let world_width = screen_width / zoom;
    let world_waist = screen_waist / zoom;

    let tip = origin + dir * length;
    let waist = origin + dir * world_waist;
    let left = waist + perp * (world_width * 0.5);
    let right = waist - perp * (world_width * 0.5);

    // Draw order matters for a convex-looking kite: origin -> left -> tip -> right
    [origin, left, tip, right]
}

// ── Textured attachments (T-301) ─────────────────────────────────────────────

/// Decode any asset the GPU cache does not hold yet.
///
/// Textures are keyed by a **content hash**, not by `AssetId`: slotmap keys are
/// reused after a document is closed, so an id-keyed cache would happily draw
/// the previous project's arm on this project's leg. Hashing also means two
/// copies of the same image share one upload.
///
/// Called before rendering because it needs `&mut` (to memoize the hash), while
/// the render pass reads an immutable state.
pub fn prepare_textures(state: &mut AppState) -> Vec<SpriteUpload> {
    use std::hash::{Hash, Hasher};

    let mut uploads = Vec::new();
    let ids: Vec<_> = state.doc.assets.images.keys().collect();

    for id in ids {
        if state.session.texture_keys.contains_key(id) {
            continue;
        }
        let Some(asset) = state.doc.assets.get(id) else {
            continue;
        };
        if asset.bytes.is_empty() {
            continue;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        asset.bytes.hash(&mut hasher);
        asset.width.hash(&mut hasher);
        asset.height.hash(&mut hasher);
        let key = hasher.finish();
        state.session.texture_keys.insert(id, key);

        if state.session.uploaded_textures.contains(&key) {
            continue;
        }

        match image::load_from_memory(&asset.bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                uploads.push(SpriteUpload {
                    key,
                    width: w,
                    height: h,
                    rgba: rgba.into_raw(),
                });
                state.session.uploaded_textures.insert(key);
            }
            Err(e) => {
                // A file we cannot decode is a user-visible problem, not a crash:
                // the attachment simply does not draw.
                state
                    .session
                    .set_status(format!("Could not decode '{}': {e}", asset.name));
            }
        }
    }
    uploads
}

/// The four world-space corners of a region attachment, in TL, BL, BR, TR order.
///
/// Pivot-aware placement lives in core (`RegionAttachment::local_corners`) so
/// the viewport, the exporter and the runtime cannot disagree about where an
/// image sits.
/// World position of one weighted vertex.
///
/// A bounding box skins the same way a mesh does but has no bind matrices of its
/// own: its vertices are authored in each influencing bone's setup frame already,
/// which is the same convention weighted mesh vertices use once bound.
fn skinned_vertex(
    weights: &[ankhimate_core::attachment::VertexWeight],
    local: glam::Vec2,
    state: &AppState,
) -> glam::Vec2 {
    // The maths lives in core (`Pose::skinned_vertex`) so the viewport and the
    // runtime cannot drift apart about where a weighted vertex belongs.
    state.pose.skinned_vertex(weights, local)
}

fn region_corners(
    region: &RegionAttachment,
    bone_world: &ankhimate_core::transforms::Affine2,
) -> [glam::Vec2; 4] {
    region
        .local_corners()
        .map(|corner| bone_world.transform_point(corner))
}

/// The animated deform offsets for a slot's attachment, if the clip has any.
fn mesh_deform(state: &AppState, slot: SlotId) -> Option<&Vec<glam::Vec2>> {
    let name = state.pose.attachment_name(&state.doc.skeleton, slot)?;
    state.pose.deforms.get(&(slot, name.to_string()))
}

/// Build the textured draw for a slot — a quad for a region, a triangle list for
/// a mesh — or `None` if anything it needs is missing.
fn sprite_for_slot(state: &AppState, slot_id: SlotId) -> Option<SpriteDraw> {
    if !state.session.show_artwork {
        return None;
    }
    let slot = state.doc.skeleton.slots.get(slot_id)?;
    // Through the pose, not the slot: an attachment timeline writes the name it
    // shows into the pose, and reading the slot directly draws the setup pose for
    // the whole clip.
    let attachment =
        state
            .doc
            .skeleton
            .resolve_posed(&state.session.skin_stack(), &state.pose, slot_id)?;

    // Hidden slots draw nothing at all (T-505) — distinct from alpha 0, which
    // still costs a draw call and still blends.
    if state.pose.slot_visible.get(slot_id) == Some(&false) {
        return None;
    }
    // Hidden from the tree's visibility column: a way of looking, not a property
    // of the rig, so it is checked here rather than in the pose.
    if state.session.hidden_slots.contains(&slot_id) {
        return None;
    }

    let bone_world = state.pose.worlds.get(slot.bone)?;
    let mut color = state
        .pose
        .slot_colors
        .get(slot_id)
        .copied()
        .unwrap_or(slot.color);
    // Art outside an isolation dims rather than disappears (T-903), the opposite
    // of what the bones do. A bone outside the isolation is clutter, but the art
    // is the silhouette — posing a limb against a character that has vanished is
    // guesswork, and the ghost is what makes the limb's placement legible.
    if !state.session.is_isolated_in(slot.bone) {
        color[3] *= ISOLATION_GHOST;
    }
    // `dark.a` is the amount, so an absent two-color tint is all zeroes and the
    // shader's second term vanishes without a branch.
    let dark = state
        .pose
        .slot_dark_colors
        .get(slot_id)
        .copied()
        .or(slot.dark_color)
        .unwrap_or([0.0; 4]);

    // A sequence replaces the attachment's own texture with the frame the pose
    // says is showing. Resolved here rather than in the model so a frame the
    // asset database has since lost falls back to the attachment's texture
    // instead of drawing nothing.
    let sequence_frame = |sequence: &Option<ankhimate_core::attachment::Sequence>| {
        let sequence = sequence.as_ref()?;
        let index = state.pose.slot_sequence_frames.get(slot_id).copied()?;
        sequence.frame(index).cloned()
    };

    let region = match attachment {
        Attachment::Region(region) => region,
        // The non-artwork attachments: clips mask other slots (T-405), paths
        // drive bones along themselves (T-502), hitboxes and points carry no
        // pixels at all. Each is drawn as an overlay, not as a sprite.
        Attachment::Clipping(_)
        | Attachment::Path(_)
        | Attachment::BoundingBox(_)
        | Attachment::Point(_) => return None,
        // A mesh draws its own triangles. Vertices are in the bone's local
        // space, so the bone affine is all that is needed — weight skinning
        // (T-403) and deform offsets (T-404) slot in here later.
        Attachment::Mesh(mesh) => {
            // A linked mesh draws the source's geometry under its own texture.
            let geometry = state
                .doc
                .skeleton
                .resolve_linked_mesh(&state.session.skin_stack(), mesh);
            let texture = sequence_frame(&mesh.sequence).unwrap_or_else(|| mesh.texture.clone());
            let mesh = geometry;
            let asset_id = state.doc.assets.by_name(&texture)?;
            let key = *state.session.texture_keys.get(asset_id)?;
            if mesh.triangles.is_empty() {
                return None;
            }
            // A weighted vertex follows its bones (T-403); an unweighted one
            // rides its slot's bone rigidly. Both paths land in the same buffer.
            // Deform offsets are applied to the setup vertex *before* skinning
            // (T-404): the shape is authored in local space, then the bones move
            // it. Skinning first would rotate the offsets with the bone.
            //
            // No rigid/skinned branch here: `skin_vertex_with_ffd` falls back to
            // `bone_world` per vertex, so a mesh with no weights and a vertex
            // with no weights are handled by the same path.
            let deform = mesh_deform(state, slot_id);
            let vertices = (0..mesh.setup_vertices.len())
                .map(|i| {
                    let offset = deform.and_then(|d| d.get(i).copied()).unwrap_or_default();
                    let world = mesh.skin_vertex_with_ffd(i, offset, &state.pose, bone_world);
                    let uv = mesh.uvs.get(i).copied().unwrap_or(glam::Vec2::ZERO);
                    MeshVertex {
                        position: [world.x, world.y],
                        uv: [uv.x, uv.y],
                        color,
                        dark,
                    }
                })
                .collect();
            let indices = mesh.triangles.iter().flat_map(|t| *t).collect();
            return Some(SpriteDraw {
                key,
                vertices,
                indices,
                blend: slot.blend_mode,
            });
        }
    };

    let texture = sequence_frame(&region.sequence).unwrap_or_else(|| region.texture.clone());
    let asset_id = state.doc.assets.by_name(&texture)?;
    let key = *state.session.texture_keys.get(asset_id)?;
    let corners = region_corners(region, bone_world);

    // Texture space runs downward, world space upward, so the top-left corner
    // takes the *minimum* v.
    let uv = &region.uv_rect;
    let uvs = [
        [uv.x, uv.y],
        [uv.x, uv.y + uv.h],
        [uv.x + uv.w, uv.y + uv.h],
        [uv.x + uv.w, uv.y],
    ];

    let vertices = std::array::from_fn(|i| MeshVertex {
        position: [corners[i].x, corners[i].y],
        uv: uvs[i],
        color,
        dark,
    });
    Some(SpriteDraw::quad(key, vertices, slot.blend_mode))
}

// ── Shear gizmo geometry ─────────────────────────────────────────────────────

/// Screen distance from the bone origin to a shear handle.
pub const SHEAR_HANDLE_LEN: f32 = 42.0;
/// Click/hover radius of a shear handle dot.
pub const SHEAR_HANDLE_RADIUS: f32 = 7.0;

/// The shear gizmo's on-screen frame: origin plus the *actual* (sheared) screen
/// directions of the bone's X and Y axes.
///
/// Taken from the world affine's columns rather than from `decompose`, because
/// decompose canonicalizes `shear.x` into `rotation` — the handles must sit on
/// the axes the artwork is actually drawn along, or dragging them fights what
/// the user sees.
///
/// Shared with the hit-test in `tools::select` so the dot you click is the dot
/// you see; two copies of this maths would drift apart in a week.
pub struct ShearFrame {
    /// Origin in canvas-local screen space (relative to the viewport rect).
    pub origin: glam::Vec2,
    pub dir_x: glam::Vec2,
    pub dir_y: glam::Vec2,
    /// Where the axes would point with `shear = 0` — the wedge each sector is
    /// measured from, so the fill shows *how much* shear is applied.
    pub base_x: glam::Vec2,
    pub base_y: glam::Vec2,
}

impl ShearFrame {
    pub fn for_bone(
        state: &AppState,
        bone: ankhimate_core::ids::BoneId,
        viewport_size: glam::Vec2,
    ) -> Option<Self> {
        let world = state.pose.worlds.get(bone)?;
        let origin_world = state.pose.world_position(bone);
        let origin = state
            .session
            .camera
            .world_to_screen(origin_world, viewport_size);

        // Project a unit step along each axis, then normalize in screen space:
        // this survives camera zoom and any parent scale.
        let axis_screen = |axis: glam::Vec2| {
            let tip = state
                .session
                .camera
                .world_to_screen(origin_world + axis.normalize_or_zero(), viewport_size);
            (tip - origin).normalize_or_zero()
        };

        // The same bone with shear removed, to measure the sectors against.
        // Rebuilt from the parent's world affine rather than from `decompose`,
        // which would have folded shear.x into rotation already.
        let unsheared = {
            let bone_data = state.doc.skeleton.bones.get(bone)?;
            let parent_world = bone_data
                .parent
                .and_then(|p| state.pose.worlds.get(p).copied())
                .unwrap_or(ankhimate_core::transforms::Affine2::IDENTITY);
            let mut local = state
                .pose
                .locals
                .get(bone)
                .copied()
                .unwrap_or(bone_data.local_transform);
            local.shear = glam::Vec2::ZERO;
            parent_world.mul(&ankhimate_core::transforms::Affine2::compose(&local))
        };

        Some(Self {
            origin,
            dir_x: axis_screen(glam::vec2(world.a, world.b)),
            dir_y: axis_screen(glam::vec2(world.c, world.d)),
            base_x: axis_screen(glam::vec2(unsheared.a, unsheared.b)),
            base_y: axis_screen(glam::vec2(unsheared.c, unsheared.d)),
        })
    }

    /// The four handle positions: X positive/negative, then Y positive/negative.
    pub fn handles(&self) -> [(glam::Vec2, crate::session::GizmoInteraction); 4] {
        use crate::session::GizmoInteraction as G;
        [
            (self.origin + self.dir_x * SHEAR_HANDLE_LEN, G::ShearX),
            (self.origin - self.dir_x * SHEAR_HANDLE_LEN, G::ShearX),
            (self.origin + self.dir_y * SHEAR_HANDLE_LEN, G::ShearY),
            (self.origin - self.dir_y * SHEAR_HANDLE_LEN, G::ShearY),
        ]
    }

    /// Which handle (if any) is under a canvas-local screen point.
    pub fn hit(&self, point: glam::Vec2) -> crate::session::GizmoInteraction {
        use crate::session::GizmoInteraction as G;
        // Grab radius is generous relative to the drawn dot — a 5px target is
        // unusable on a trackpad.
        let grab = SHEAR_HANDLE_RADIUS + 4.0;
        for (pos, kind) in self.handles() {
            if (point - pos).length() <= grab {
                return kind;
            }
        }
        G::None
    }
}

/// Fill the sector swept going from `from` to `to` around `center`.
///
/// Drawn as a triangle fan so it stays correct past 90°: egui's convex-polygon
/// fill would misrender a reflex wedge, and shear angles routinely exceed 180°.
/// One vertex's whole influence split, drawn as a pie in the bones' own colours.
///
/// The heat overlay answers "how much of the *selected* bone is here". This
/// answers the other question — "what is holding this vertex at all" — which is
/// the one you have when a vertex moves and you do not know what moved it.
/// Bone colours rather than a heat ramp, so a wedge is traceable back to a row
/// in the weight list without a legend.
///
/// `ranks` maps a bone to its position in this mesh's influence ranking, built
/// once by the caller — see [`color_for_rank`].
fn pie(
    painter: &egui::Painter,
    center: egui::Pos2,
    weights: Option<&Vec<ankhimate_core::attachment::VertexWeight>>,
    ranks: &std::collections::HashMap<ankhimate_core::ids::BoneId, usize>,
) {
    const RADIUS: f32 = 6.0;
    let Some(weights) = weights.filter(|w| !w.is_empty()) else {
        // An unweighted vertex is a hollow ring, not an absent one: a gap in the
        // pies reads as "no vertex here", which is the wrong thing to conclude.
        painter.circle_stroke(
            center,
            RADIUS * 0.6,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(120)),
        );
        return;
    };

    let total: f32 = weights.iter().map(|w| w.weight).sum();
    if total <= 1e-6 {
        return;
    }
    painter.circle_filled(center, RADIUS + 1.0, egui::Color32::from_black_alpha(160));

    let mut start = -std::f32::consts::FRAC_PI_2;
    for w in weights {
        let sweep = w.weight / total * std::f32::consts::TAU;
        // Unreachable in practice — `ranks` is built from these same weights —
        // but the wedge is skipped rather than given a stand-in colour, which
        // would claim a rank this bone does not hold.
        let Some(rgba) = color_for_rank(ranks.get(&w.bone).copied()) else {
            start += sweep;
            continue;
        };
        let color = egui::Color32::from_rgb(
            (rgba[0] * 255.0) as u8,
            (rgba[1] * 255.0) as u8,
            (rgba[2] * 255.0) as u8,
        );

        // Fanned as a triangle strip from the centre. Steps scale with the
        // wedge so a thin slice is not spent on twelve segments it cannot show.
        let steps = ((sweep / std::f32::consts::TAU) * 24.0).ceil().max(2.0) as usize;
        let mut fan = egui::Mesh::default();
        fan.colored_vertex(center, color);
        for step in 0..=steps {
            let angle = start + sweep * (step as f32 / steps as f32);
            fan.colored_vertex(
                center + egui::vec2(angle.cos(), angle.sin()) * RADIUS,
                color,
            );
        }
        for step in 1..=steps as u32 {
            fan.indices.extend_from_slice(&[0, step, step + 1]);
        }
        painter.add(egui::Shape::mesh(fan));

        // A spoke at each slice's leading edge. Without one a single-influence
        // vertex — which is most of them on a finished rig — draws as a plain
        // filled disc, and a plain disc reads as "a dot", not as "one bone holds
        // all of this". The spoke is what makes it legible as a chart.
        let edge = egui::vec2(start.cos(), start.sin()) * RADIUS;
        painter.line_segment(
            [center, center + edge],
            egui::Stroke::new(1.0, egui::Color32::from_black_alpha(200)),
        );
        start += sweep;
    }

    painter.circle_stroke(
        center,
        RADIUS,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(160)),
    );
}

/// One vertex's influences mixed into a single colour for the surface overlay.
///
/// *Every* bone on the vertex, not just the active one. Shading the active
/// bone's weight from black to full was the earlier version, and it made a
/// correctly weighted mesh look broken: where a neighbouring bone takes over,
/// the active bone's weight is near zero, so a perfectly good foot faded into a
/// black wedge with nothing to say what was holding it. Mixing means the same
/// area comes out in the neighbour's colour instead — which is the actual
/// answer, and the one the pies and the weight list already give.
///
/// Weighted mean in the palette's colours, so a vertex split evenly between the
/// sky and pink bones lands halfway between them, and the gradient across a
/// joint reads as one colour handing over to the other.
///
/// Fully opaque. Translucency was tried and it made the one thing this overlay
/// is for — reading *how much* sits where — a guess: a strong weight over dark
/// artwork and a weak one over light artwork came out the same on screen,
/// because half of what you were looking at was the drawing underneath. Hiding
/// the artwork is the point while weighting; the art is one keypress away.
///
/// Black is kept for one case only, and it now means what it looks like: a
/// vertex nothing holds. Unweighted geometry does not deform, so it is worth
/// showing as starkly as an error.
fn heat(
    weights: Option<&Vec<ankhimate_core::attachment::VertexWeight>>,
    ranks: &std::collections::HashMap<ankhimate_core::ids::BoneId, usize>,
) -> egui::Color32 {
    let Some(weights) = weights.filter(|w| !w.is_empty()) else {
        return egui::Color32::BLACK;
    };
    let mut total = 0.0;
    let mut mixed = [0.0f32; 3];
    for w in weights {
        let Some(rgba) = color_for_rank(ranks.get(&w.bone).copied()) else {
            continue;
        };
        for channel in 0..3 {
            mixed[channel] += rgba[channel] * w.weight;
        }
        total += w.weight;
    }
    if total <= 1e-6 {
        return egui::Color32::BLACK;
    }
    // Normalised by the total rather than assuming it is 1. Weights are not
    // required to sum to one while they are being painted, and an un-normalised
    // mix would read as "darker" purely because the vertex is under-weighted —
    // which is a different fault, and one the over-influence ring already flags.
    egui::Color32::from_rgb(
        (mixed[0] / total * 255.0) as u8,
        (mixed[1] / total * 255.0) as u8,
        (mixed[2] / total * 255.0) as u8,
    )
}

fn filled_wedge(
    painter: &egui::Painter,
    center: egui::Pos2,
    from: glam::Vec2,
    to: glam::Vec2,
    radius: f32,
    color: egui::Color32,
) {
    let start = from.y.atan2(from.x);
    // Shortest signed sweep between the two directions.
    let sweep = {
        let raw = to.y.atan2(to.x) - start;
        ankhimate_core::transforms::wrap_angle(raw)
    };
    if sweep.abs() < 0.01 {
        return;
    }

    // ~4° per triangle: smooth enough at gizmo size, cheap at any angle.
    let steps = ((sweep.abs() / 0.07).ceil() as usize).clamp(1, 96);
    let step = sweep / steps as f32;
    let point_at = |angle: f32| center + egui::vec2(angle.cos(), angle.sin()) * radius;

    for i in 0..steps {
        let a0 = start + step * i as f32;
        let a1 = start + step * (i + 1) as f32;
        painter.add(egui::Shape::convex_polygon(
            vec![center, point_at(a0), point_at(a1)],
            color,
            egui::Stroke::NONE,
        ));
    }
}

/// The shear gizmo: a red X-axis bar and a green Y-axis bar with grab dots at
/// both ends, over sector fills that show how far each axis has been swung.
///
/// Dragging a red dot swings the X axis (`shear.x`), a green dot the Y axis
/// (`shear.y`) — so the handle that moves is the axis that changes, which is the
/// whole reason the two colours exist.
fn draw_shear_gizmo(
    painter: &egui::Painter,
    rect: egui::Rect,
    state: &AppState,
    bone: ankhimate_core::ids::BoneId,
    viewport_size: glam::Vec2,
) {
    use crate::session::GizmoInteraction as G;

    let Some(frame) = ShearFrame::for_bone(state, bone, viewport_size) else {
        return;
    };
    let to_pos = |v: glam::Vec2| egui::pos2(rect.min.x + v.x, rect.min.y + v.y);

    let active =
        |kind: G| state.session.hovered_gizmo == kind || state.session.dragging_gizmo == kind;
    let red = if active(G::ShearX) {
        egui::Color32::from_rgb(255, 120, 120)
    } else {
        egui::Color32::from_rgb(225, 45, 45)
    };
    let green = if active(G::ShearY) {
        egui::Color32::from_rgb(140, 255, 140)
    } else {
        egui::Color32::from_rgb(45, 205, 45)
    };

    let origin = to_pos(frame.origin);
    let dir_x = egui::vec2(frame.dir_x.x, frame.dir_x.y);
    let dir_y = egui::vec2(frame.dir_y.x, frame.dir_y.y);

    // Sectors, not discs: each wedge sweeps from where its axis would sit with
    // no shear to where it sits now, so the fill *is* the shear amount. Two full
    // circles would just be a muddy overlap that says nothing.
    let disc = SHEAR_HANDLE_LEN * 0.55;
    painter.circle_stroke(
        origin,
        disc,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(28)),
    );
    // Each sector is drawn on both halves — a bowtie, not a single slice. The
    // axis bar runs through the origin in both directions, so the swept angle is
    // just as true on the far side; filling only one half reads as lopsided
    // against a symmetric handle.
    for (base, dir, color) in [
        (frame.base_x, frame.dir_x, red),
        (frame.base_y, frame.dir_y, green),
    ] {
        let fill = color.gamma_multiply(0.35);
        filled_wedge(painter, origin, base, dir, disc, fill);
        filled_wedge(painter, origin, -base, -dir, disc, fill);
    }

    // Reference axes: where each axis would sit with no shear. Spine draws these
    // alongside the live ones, and without them the gizmo is ambiguous — you can
    // see where an axis *is* but not how far it has been swung, which is the
    // whole point of the tool.
    for (base, color) in [(frame.base_x, red), (frame.base_y, green)] {
        let dir = egui::vec2(base.x, base.y);
        let end = origin + dir * SHEAR_HANDLE_LEN;
        painter.line_segment(
            [origin, end],
            egui::Stroke::new(1.5, color.gamma_multiply(0.55)),
        );
        painter.circle_stroke(end, 3.0, egui::Stroke::new(1.5, color.gamma_multiply(0.55)));
    }

    // Axis bars, each with a dark 1px offset copy so they stay legible over
    // light artwork.
    for (dir, color) in [(dir_x, red), (dir_y, green)] {
        let a = origin - dir * SHEAR_HANDLE_LEN;
        let b = origin + dir * SHEAR_HANDLE_LEN;
        let shadow = egui::vec2(1.0, 1.0);
        painter.line_segment(
            [a + shadow, b + shadow],
            egui::Stroke::new(3.0, egui::Color32::from_black_alpha(140)),
        );
        painter.line_segment([a, b], egui::Stroke::new(2.5, color));
    }

    // Grab dots at all four ends.
    for (pos, kind) in frame.handles() {
        let p = to_pos(pos);
        let color = match kind {
            G::ShearX => red,
            _ => green,
        };
        painter.circle_filled(
            p + egui::vec2(1.0, 1.0),
            SHEAR_HANDLE_RADIUS,
            egui::Color32::from_black_alpha(140),
        );
        painter.circle_filled(p, SHEAR_HANDLE_RADIUS, color);
        if active(kind) {
            painter.circle_stroke(
                p,
                SHEAR_HANDLE_RADIUS + 2.0,
                egui::Stroke::new(1.5, egui::Color32::WHITE),
            );
        }
    }

    // Origin pin, matching the other tools' centre marker.
    painter.circle_filled(origin, 5.0, egui::Color32::from_rgb(20, 20, 20));
    painter.circle_stroke(
        origin,
        4.0,
        egui::Stroke::new(2.0, egui::Color32::from_rgb(200, 200, 200)),
    );
}

pub fn render_bones(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &AppState,
    theme: &crate::theme::Theme,
    sprite_uploads: Vec<SpriteUpload>,
) {
    // The wgpu callback carrying the artwork is only assembled at the *end* of
    // this function, but egui paints a layer in submission order — so gizmos
    // drawn beforehand would end up buried under the art.
    //
    // The fix is to reserve the callback's slot now and fill it in last, rather
    // than to promote the gizmos to a foreground layer: a foreground layer beats
    // every window too, so the handles floated on top of open dialogs.
    let canvas_painter = ui.painter_at(rect);
    let artwork_slot = canvas_painter.add(egui::Shape::Noop);
    // The weight surface is opaque, so anything drawn before it is simply gone —
    // and the bones were, which left weight painting with no way to see the
    // thing being painted *with*. Reserved here for the same reason as the
    // artwork: it is built much further down, once the mesh has been resolved,
    // but it belongs underneath every gizmo.
    let weight_surface_slot = canvas_painter.add(egui::Shape::Noop);
    let painter = canvas_painter.clone();
    let viewport_size = glam::Vec2::new(rect.width(), rect.height());

    // The flat-colour mesh pipeline has no producer since the weight heat map
    // moved to the egui painter, which can scope it to one mesh and shade the
    // triangles by vertex. The pipeline is left in place rather than torn out
    // with it: removing a wgpu pass is its own change with its own risk, and
    // this is the seam any future GPU-side mesh overlay would plug into.
    let mesh_draws = Vec::new();

    // Which bones hold the mesh being weighted, and in what order — empty unless
    // the weight tool is up with a mesh under it.
    //
    // Computed here rather than inside the weight overlay below because the bone
    // gizmos need it too: while weighting, a bone is drawn in the colour it has
    // *on this mesh*, so that the stick on the canvas, its wedge in a vertex pie
    // and its row in the weight list are all the same colour. Ranked once, since
    // both readers would otherwise re-walk every weight in the mesh.
    let weight_ranks: std::collections::HashMap<ankhimate_core::ids::BoneId, usize> =
        if state.session.tool == crate::session::Tool::WeightPaint
            && let Some(slot_id) = state.session.active_slot()
            && let Some(Attachment::Mesh(mesh)) = state
                .doc
                .skeleton
                .resolve_slot_many(&state.session.skin_stack(), slot_id)
        {
            mesh.bound_bones()
                .iter()
                .enumerate()
                .map(|(rank, (bone, _))| (*bone, rank))
                .collect()
        } else {
            std::collections::HashMap::new()
        };

    for (bone_id, bone) in state.doc.skeleton.bones.iter() {
        if !state.session.show_bones {
            break;
        }
        // Isolation hides rather than dims (T-903). Dimming a sixty-bone rig
        // leaves sixty bones drawn over the limb being worked on, which is the
        // problem isolation was reached for.
        if !state.session.is_isolated_in(bone_id) {
            continue;
        }
        let is_selected = state.session.is_bone_selected(bone_id);
        let is_hovered = state.session.hovered_bone == Some(bone_id);

        let world = state.pose.world_decomposed(bone_id);
        let origin = state.pose.world_position(bone_id);
        let angle = world.rotation;

        // Colors
        let (fill_color, stroke_color, joint_color) = if is_selected {
            (
                egui::Color32::from_rgba_unmultiplied(255, 165, 0, 150),
                egui::Color32::from_rgb(255, 200, 0),
                egui::Color32::from_rgb(255, 255, 0),
            )
        } else if is_hovered {
            (
                egui::Color32::from_rgba_unmultiplied(50, 220, 220, 150),
                egui::Color32::from_rgb(100, 255, 255),
                egui::Color32::from_rgb(150, 255, 255),
            )
        } else {
            // While weighting, the bone's colour *on the mesh being painted*;
            // otherwise its group colour (T-505), inherited from the nearest
            // ancestor that set one. Selection and hover still win — knowing
            // what is selected matters more than knowing which group it is in.
            //
            // The swap matters because a group colour says "this is the left
            // arm", which is not the question in front of you while weighting.
            // There the question is "which of these sticks is the pink in the
            // mesh", and only the rank colour answers it. A bone with no weight
            // on this mesh has no rank and keeps its group colour, which is also
            // what marks it out as not yet bound.
            let ranked = color_for_rank(weight_ranks.get(&bone_id).copied());
            let [r, g, b, a] = ranked.unwrap_or_else(|| group_color(&state.doc.skeleton, bone_id));
            let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0) as u8;
            // Opaque over the weight surface. The usual 40% fill is right over
            // artwork, where a see-through stick keeps the art readable, and
            // wrong over the overlay: a sky-coloured bone at 40% sitting on the
            // sky-coloured region it owns is invisible in exactly the place you
            // most need to find it. The dark outline below is what separates the
            // two.
            let fill_alpha = if ranked.is_some() { a } else { a * 0.4 };
            (
                egui::Color32::from_rgba_unmultiplied(
                    to_u8(r),
                    to_u8(g),
                    to_u8(b),
                    to_u8(fill_alpha),
                ),
                if ranked.is_some() {
                    // Near-black rather than a darker shade of the fill: the
                    // shade separates a bone from artwork, but not from the
                    // overlay region painted in that same colour, which is the
                    // one case that matters here.
                    egui::Color32::from_rgb(20, 20, 24)
                } else {
                    egui::Color32::from_rgba_unmultiplied(
                        to_u8(r * 0.7),
                        to_u8(g * 0.7),
                        to_u8(b * 0.7),
                        255,
                    )
                },
                egui::Color32::from_rgba_unmultiplied(to_u8(r), to_u8(g), to_u8(b), 255),
            )
        };

        // A target bone is drawn as a marker rather than a bone: it is a
        // handle for a constraint, not part of the skeleton's silhouette.
        let is_ik_target = state.doc.skeleton.constraints.values().any(|c| match c {
            ankhimate_core::constraints::Constraint::Ik(ik) => ik.target == bone_id,
            ankhimate_core::constraints::Constraint::Transform(tc) => tc.target == bone_id,
            // A physics bone is an ordinary bone that happens to wobble; it has
            // no target handle to draw.
            ankhimate_core::constraints::Constraint::Physics(_)
            | ankhimate_core::constraints::Constraint::Path(_) => false,
        });

        // The same kite the create-bone tool previews while you drag, drawn by
        // the same function. It used to be a quad through a separate wgpu
        // pipeline, so a bone changed shape the moment you released the mouse —
        // two drawings of one thing that could, and did, disagree.
        if !is_ik_target {
            let points: Vec<egui::Pos2> = bone_gizmo_vertices(
                origin,
                world.rotation,
                bone.length,
                state.session.camera.zoom,
            )
            .iter()
            .map(|v| {
                let screen = state.session.camera.world_to_screen(*v, viewport_size);
                egui::pos2(rect.min.x + screen.x, rect.min.y + screen.y)
            })
            .collect();
            painter.add(egui::Shape::convex_polygon(
                points.clone(),
                fill_color,
                egui::Stroke::new(1.0, stroke_color),
            ));
            painter.circle_filled(points[0], 3.5, joint_color);
        } else {
            let screen_radius = 8.0;
            let world_radius = screen_radius / state.session.camera.zoom;
            let world_vertices = [
                origin + glam::Vec2::new(0.0, world_radius),
                origin + glam::Vec2::new(world_radius, 0.0),
                origin + glam::Vec2::new(0.0, -world_radius),
                origin + glam::Vec2::new(-world_radius, 0.0),
            ];

            let points: Vec<egui::Pos2> = world_vertices
                .iter()
                .map(|v| {
                    let screen = state.session.camera.world_to_screen(*v, viewport_size);
                    egui::pos2(rect.min.x + screen.x, rect.min.y + screen.y)
                })
                .collect();

            painter.add(egui::Shape::convex_polygon(
                points,
                fill_color,
                egui::Stroke::new(1.0, stroke_color),
            ));
        }

        // Draw joint circle on top
        if !is_ik_target {
            let center = state.session.camera.world_to_screen(origin, viewport_size);
            let point = egui::pos2(rect.min.x + center.x, rect.min.y + center.y);
            painter.circle_filled(point, 4.0, joint_color);
            painter.circle_stroke(point, 4.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
        } else {
            let world_vertices =
                bone_gizmo_vertices(origin, angle, bone.length, state.session.camera.zoom);

            let v0 = state
                .session
                .camera
                .world_to_screen(world_vertices[0], viewport_size);
            let v1 = state
                .session
                .camera
                .world_to_screen(world_vertices[1], viewport_size);
            let v2 = state
                .session
                .camera
                .world_to_screen(world_vertices[2], viewport_size);
            let v3 = state
                .session
                .camera
                .world_to_screen(world_vertices[3], viewport_size);

            let points = vec![
                egui::pos2(rect.min.x + v0.x, rect.min.y + v0.y),
                egui::pos2(rect.min.x + v1.x, rect.min.y + v1.y),
                egui::pos2(rect.min.x + v2.x, rect.min.y + v2.y),
                egui::pos2(rect.min.x + v3.x, rect.min.y + v3.y),
            ];

            painter.add(egui::Shape::convex_polygon(
                points.clone(),
                fill_color,
                egui::Stroke::new(1.0, stroke_color),
            ));

            painter.circle_filled(points[0], 4.0, joint_color);
            painter.circle_stroke(points[0], 4.0, egui::Stroke::new(1.0, stroke_color));
        }
    }

    // Preview bone (while dragging to create)
    if let Some((start, end)) = state.session.preview_bone {
        let delta = end - start;
        let length = delta.length().max(0.01);
        let angle = f32::atan2(delta.y, delta.x);

        let world_vertices = bone_gizmo_vertices(start, angle, length, state.session.camera.zoom);

        let points: Vec<egui::Pos2> = world_vertices
            .iter()
            .map(|v| {
                let screen = state.session.camera.world_to_screen(*v, viewport_size);
                egui::pos2(rect.min.x + screen.x, rect.min.y + screen.y)
            })
            .collect();

        painter.add(egui::Shape::convex_polygon(
            points.clone(),
            egui::Color32::from_rgba_unmultiplied(128, 255, 255, 80),
            egui::Stroke::new(1.0, egui::Color32::from_rgb(128, 255, 255)),
        ));
        painter.circle_filled(points[0], 4.0, egui::Color32::from_rgb(128, 255, 255));
    }

    // Pivot marker for the selected slot's artwork: the point it turns and
    // scales around. Invisible geometry is unauthorable — without this the
    // pivot is a pair of numbers with no relation to what is on screen.
    if let Some(slot_id) = state.session.active_slot()
        && let Some(slot) = state.doc.skeleton.slots.get(slot_id)
        && let Some(ankhimate_core::attachment::Attachment::Region(region)) = state
            .doc
            .skeleton
            .resolve_slot_many(&state.session.skin_stack(), slot_id)
        && let Some(bone_world) = state.pose.worlds.get(slot.bone)
    {
        let pivot_screen = state.session.camera.world_to_screen(
            bone_world.transform_point(region.local_offset),
            viewport_size,
        );
        let p = egui::pos2(rect.min.x + pivot_screen.x, rect.min.y + pivot_screen.y);
        // Bigger and brighter once it is a live handle, so "this is draggable
        // now" needs no explanation.
        let active = state.session.editing_attachment();
        let arm = if active { 11.0 } else { 6.0 };
        let color = if active {
            egui::Color32::from_rgb(255, 225, 140)
        } else {
            egui::Color32::from_rgb(255, 205, 90)
        };
        for (a, b) in [
            (egui::vec2(-arm, 0.0), egui::vec2(arm, 0.0)),
            (egui::vec2(0.0, -arm), egui::vec2(0.0, arm)),
        ] {
            painter.line_segment(
                [p + a, p + b],
                egui::Stroke::new(1.5, egui::Color32::from_black_alpha(150)),
            );
        }
        for (a, b) in [
            (egui::vec2(-arm, 0.0), egui::vec2(arm, 0.0)),
            (egui::vec2(0.0, -arm), egui::vec2(0.0, arm)),
        ] {
            painter.line_segment([p + a, p + b], egui::Stroke::new(1.0, color));
        }
        painter.circle_stroke(
            p,
            if active { 5.0 } else { 3.0 },
            egui::Stroke::new(1.0, color),
        );
        if active {
            // The quad outline, so it is obvious which art the handle drives.
            let corners = region.local_corners().map(|c| {
                let s = state
                    .session
                    .camera
                    .world_to_screen(bone_world.transform_point(c), viewport_size);
                egui::pos2(rect.min.x + s.x, rect.min.y + s.y)
            });
            for i in 0..4 {
                painter.line_segment(
                    [corners[i], corners[(i + 1) % 4]],
                    egui::Stroke::new(1.0, color.gamma_multiply(0.7)),
                );
            }
        }
    }

    // Weight heat map (T-403): how strongly the selected bone holds each vertex
    // of the selected mesh. Weights are invisible without it, and "paint until
    // it looks right" needs something to look at.
    if state.session.tool == crate::session::Tool::WeightPaint
        && let Some(slot_id) = state.session.active_slot()
        && let Some(bone) = state.session.active_bone()
        && let Some(slot) = state.doc.skeleton.slots.get(slot_id)
        && let Some(Attachment::Mesh(mesh)) = state
            .doc
            .skeleton
            .resolve_slot_many(&state.session.skin_stack(), slot_id)
        && let Some(bone_world) = state.pose.worlds.get(slot.bone)
    {
        let points: Vec<egui::Pos2> = (0..mesh.setup_vertices.len())
            .map(|index| {
                let world =
                    mesh.skin_vertex_with_ffd(index, glam::Vec2::ZERO, &state.pose, bone_world);
                let screen = state.session.camera.world_to_screen(world, viewport_size);
                egui::pos2(rect.min.x + screen.x, rect.min.y + screen.y)
            })
            .collect();
        // The same ranking the bone gizmos were drawn from, so a stick and the
        // wedges it owns cannot come out different colours.
        let ranks = &weight_ranks;
        // The active bone's colour, for the brush ring. The surface itself is
        // mixed from every bone, so the ring is the only thing left saying which
        // one a stroke will land on.
        //
        // A bone with no rank holds nothing on this mesh yet, which is the
        // ordinary state a moment before the first stroke. It has no palette
        // colour to borrow — taking one would put it in another rank's place —
        // so the brush goes white until the first stroke gives it a rank.
        let bone_color = color_for_rank(ranks.get(&bone).copied())
            .map(|rgba| {
                egui::Color32::from_rgb(
                    (rgba[0] * 255.0) as u8,
                    (rgba[1] * 255.0) as u8,
                    (rgba[2] * 255.0) as u8,
                )
            })
            .unwrap_or(egui::Color32::WHITE);

        let settings = &state.session.weight_paint_settings;

        // The surface, not just the vertices. Dots alone say what each vertex
        // holds but nothing about the gradient between them — and the gradient
        // *is* the thing being painted, since that is what decides whether a
        // joint creases or goes rubbery.
        if settings.show_overlay {
            let mut fill = egui::Mesh::default();
            for (index, p) in points.iter().enumerate() {
                fill.vertices.push(egui::epaint::Vertex {
                    pos: *p,
                    uv: egui::epaint::WHITE_UV,
                    color: heat(mesh.weights.get(index), ranks),
                });
            }
            for tri in &mesh.triangles {
                if tri.iter().all(|i| (*i as usize) < points.len()) {
                    fill.indices.extend_from_slice(&[tri[0], tri[1], tri[2]]);
                }
            }
            if !fill.indices.is_empty() {
                // Into the slot reserved before the gizmos, not appended here:
                // the fill is opaque and would otherwise bury every bone on the
                // canvas — including the ones it is colour-coded to match.
                painter.set(weight_surface_slot, egui::Shape::mesh(fill));
            }
        }

        let shown = |index: usize| {
            !settings.show_selected_only || state.session.selected_vertices.contains(&index)
        };

        // Vertices on top of the surface fill: the fill interpolates between
        // vertices, so the vertex itself is the only place the mix is exact, and
        // that is what you aim a stroke at.
        for (index, p) in points.iter().enumerate() {
            if !shown(index) {
                continue;
            }
            if settings.show_pies {
                pie(&painter, *p, mesh.weights.get(index), ranks);
            } else {
                let color = heat(mesh.weights.get(index), ranks);
                painter.circle_filled(*p, 4.0, egui::Color32::from_black_alpha(120));
                painter.circle_filled(*p, 3.0, color);
            }

            // Over-influenced: more bones on this vertex than the rig budgets
            // for. Runtimes take a fixed number of influences, so a mesh that
            // quietly exceeds it exports fine and deforms differently in the
            // game than in the editor. A ring is the only warning there is.
            if mesh.weights.get(index).is_some_and(|w| {
                crate::commands::weight_cmds::over_influenced(w, settings.max_bones)
            }) {
                painter.circle_stroke(*p, 7.0, egui::Stroke::new(1.5, ui.visuals().warn_fg_color));
            }
        }

        // The brush itself. Painting with an invisible cursor footprint means
        // guessing what a stroke will touch, and the feathered part of the
        // radius behaves differently enough from the solid core to be worth
        // drawing separately.
        //
        // In the bone's colour, like everything else here: the cursor is what
        // you are looking at while painting, so it is the cheapest place to say
        // which bone the stroke will land on.
        if let Some(cursor) = ui.input(|i| i.pointer.hover_pos())
            && rect.contains(cursor)
        {
            let settings = &state.session.weight_paint_settings;
            let outer = settings.radius * state.session.camera.zoom;
            let solid = outer * (1.0 - settings.feather.clamp(0.0, 1.0));
            painter.circle_stroke(
                cursor,
                outer,
                egui::Stroke::new(1.0, bone_color.gamma_multiply(0.9)),
            );
            if solid > 1.0 {
                painter.circle_stroke(
                    cursor,
                    solid,
                    egui::Stroke::new(1.0, bone_color.gamma_multiply(0.45)),
                );
            }
        }
    }

    // Mesh edit overlay (T-401): wireframe plus grab handles. Without it the
    // vertices are invisible and the mode is unusable.
    if let Some(target) = crate::ui::canvas::tools::mesh_edit::target(state) {
        let positions = crate::ui::canvas::tools::mesh_edit::vertex_screen_positions(
            &target,
            state,
            viewport_size,
        );
        let to_pos = |v: glam::Vec2| egui::pos2(rect.min.x + v.x, rect.min.y + v.y);
        // Every colour here comes from the theme: the overlay sits directly on
        // the artwork, so a fixed palette fights whatever the user picked.
        let wire = theme.mesh_edge();

        for tri in &target.mesh.triangles {
            for k in 0..3 {
                let (Some(&a), Some(&b)) = (
                    positions.get(tri[k] as usize),
                    positions.get(tri[(k + 1) % 3] as usize),
                ) else {
                    continue;
                };
                painter.line_segment([to_pos(a), to_pos(b)], egui::Stroke::new(1.0, wire));
            }
        }

        // Pinned edges (T-401) drawn thicker and in the selection colour: a
        // constraint the user placed by hand has to be distinguishable from one
        // the triangulation happened to pick, or there is no way to find it
        // again to release it.
        for [a, b] in &target.mesh.edges {
            let (Some(&from), Some(&to)) = (positions.get(*a as usize), positions.get(*b as usize))
            else {
                continue;
            };
            painter.line_segment(
                [to_pos(from), to_pos(to)],
                egui::Stroke::new(2.5, theme.mesh_vertex_selected()),
            );
        }

        // Box-select in progress.
        if let Some(start) = state.session.vertex_box_start
            && let Some(cursor) = ui.ctx().pointer_latest_pos()
        {
            let a = to_pos(start);
            let b = egui::pos2(cursor.x, cursor.y);
            let band = egui::Rect::from_two_pos(a, b);
            painter.rect_filled(band, 0.0, wire.gamma_multiply(0.15));
            painter.rect_stroke(
                band,
                0.0,
                egui::Stroke::new(1.0, wire),
                egui::StrokeKind::Inside,
            );
        }

        // Whichever vertex a click would take, so the handle lights up before
        // the grab rather than after it. Found by the mesh-edit tool, which runs
        // before painting and owns `&mut` — this used to repeat the search here,
        // and once the hover label wanted the same answer (T-913) a single
        // source became worth more than the duplicated loop.
        let hovered = state.session.hovered_vertex;

        for (index, position) in positions.iter().enumerate() {
            let selected = state.session.selected_vertices.contains(&index);
            let p = to_pos(*position);
            // Size, not a light halo, carries the state — a ring of pale pixels
            // around every dot reads as haze over the art in a dense mesh.
            let (radius, color) = match (selected, hovered == Some(index)) {
                (true, _) => (5.0, theme.mesh_vertex_selected()),
                (false, true) => (4.5, theme.mesh_vertex_hovered()),
                (false, false) => (3.0, theme.mesh_vertex()),
            };
            painter.circle_filled(p, radius, color);
        }
    }

    // Clip polygon overlay (T-405). A clip draws no artwork of its own, so
    // without this the only evidence it exists is the hole it cuts in something
    // else — which is exactly the situation where you need to see its shape.
    if let Some(target) = crate::ui::canvas::tools::clip_edit::target(state) {
        let positions = crate::ui::canvas::tools::clip_edit::vertex_screen_positions(
            &target,
            state,
            viewport_size,
        );
        let to_pos = |v: glam::Vec2| egui::pos2(rect.min.x + v.x, rect.min.y + v.y);
        let count = positions.len();

        // A path is open; a clip and a bounding box close back to their first
        // vertex.
        let edges = if !target.kind.closed() {
            count.saturating_sub(1)
        } else {
            count
        };
        for i in 0..edges {
            painter.line_segment(
                [to_pos(positions[i]), to_pos(positions[(i + 1) % count])],
                egui::Stroke::new(1.5, theme.mesh_edge()),
            );
        }
        if let Some(start) = state.session.vertex_box_start
            && let Some(cursor) = ui.ctx().pointer_latest_pos()
        {
            let band = egui::Rect::from_two_pos(to_pos(start), cursor);
            painter.rect_filled(band, 0.0, theme.mesh_edge().gamma_multiply(0.15));
            painter.rect_stroke(
                band,
                0.0,
                egui::Stroke::new(1.0, theme.mesh_edge()),
                egui::StrokeKind::Inside,
            );
        }
        for (index, position) in positions.iter().enumerate() {
            let selected = state.session.selected_vertices.contains(&index);
            painter.circle_filled(
                to_pos(*position),
                if selected { 5.0 } else { 3.0 },
                if selected {
                    theme.mesh_vertex_selected()
                } else {
                    theme.mesh_vertex()
                },
            );
        }
    }

    // Hitbox and point overlays. Neither draws artwork, so without this the only
    // evidence they exist is a row in the tree — and a hitbox you cannot see is
    // a hitbox you cannot place. Always drawn in setup mode, and while animating
    // only for the selected item, so they do not fog the artwork being posed.
    {
        let selected_attachment = match &state.session.selection {
            Some(crate::session::Selection::Attachment { slot, name }) => {
                Some((*slot, name.clone()))
            }
            _ => None,
        };
        let always = !state.session.is_animating();
        let skins = state.session.skin_stack();
        for &slot_id in &state.pose.draw_order {
            let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
                continue;
            };
            let Some(name) = state
                .pose
                .slot_attachments
                .get(slot_id)
                .cloned()
                .flatten()
                .or_else(|| slot.attachment.clone())
            else {
                continue;
            };
            let focused = selected_attachment
                .as_ref()
                .is_some_and(|(s, n)| *s == slot_id && *n == name);
            if !always && !focused {
                continue;
            }
            let Some(attachment) = state.doc.skeleton.resolve_many(&skins, slot_id, &name) else {
                continue;
            };
            let Some(bone_world) = state.pose.worlds.get(slot.bone) else {
                continue;
            };
            let to_screen =
                |v: glam::Vec2| crate::ui::canvas::camera::world_to_screen(v, rect, state);
            match attachment {
                Attachment::BoundingBox(b) if b.vertices.len() >= 2 => {
                    // Skinned exactly like a mesh, and drawn from the same
                    // vertices, so what you see is what a hit test would use.
                    let skinned = !b.weights.is_empty();
                    let points: Vec<egui::Pos2> = (0..b.vertices.len())
                        .map(|i| {
                            let world = if skinned {
                                skinned_vertex(&b.weights[i], b.vertices[i], state)
                            } else {
                                bone_world.transform_point(b.vertices[i])
                            };
                            to_screen(world)
                        })
                        .collect();
                    let stroke =
                        egui::Stroke::new(if focused { 2.0 } else { 1.0 }, theme.hitbox_outline());
                    painter.add(egui::Shape::convex_polygon(
                        points.clone(),
                        theme.hitbox_fill(),
                        stroke,
                    ));
                    if focused {
                        for p in &points {
                            painter.circle_filled(*p, 3.0, theme.mesh_vertex());
                        }
                    }
                }
                Attachment::Point(point) => {
                    let (world, rotation) = point.world(bone_world);
                    let centre = to_screen(world);
                    let color = theme.point_marker();
                    // A cross plus a short heading tick: the orientation is the
                    // half of a point attachment that a plain dot throws away.
                    let arm = if focused { 9.0 } else { 6.0 };
                    painter.line_segment(
                        [centre - egui::vec2(arm, 0.0), centre + egui::vec2(arm, 0.0)],
                        egui::Stroke::new(1.5, color),
                    );
                    painter.line_segment(
                        [centre - egui::vec2(0.0, arm), centre + egui::vec2(0.0, arm)],
                        egui::Stroke::new(1.5, color),
                    );
                    // Screen Y runs down while world Y runs up, so the heading
                    // negates its sine (PLAN §2.2).
                    let heading = egui::vec2(rotation.cos(), -rotation.sin()) * (arm * 2.0);
                    painter.line_segment([centre, centre + heading], egui::Stroke::new(2.0, color));
                    painter.circle_filled(centre, 2.5, color);
                }
                _ => {}
            }
        }
    }

    // Artwork outlines, under the gizmos so a handle is never hidden by a line.
    if state.session.show_artwork {
        crate::ui::canvas::outline::draw(&painter, rect, state, theme);
    }

    // Transform gizmos for the selected bone.
    //
    // Only under Select. The other tools select a bone as part of doing
    // something else — weight painting clicks one to aim the brush — and the
    // gizmo then lands on top of the exact area being worked on, with handles
    // that swallow the drag meant for the mesh. A gizmo for a transform the
    // current tool cannot perform is decoration in the way.
    if state.session.tool == crate::session::Tool::Select
        && let Some(selected_id) = state.session.active_bone()
        && state.doc.skeleton.bones.contains_key(selected_id)
    {
        let selected_world = state.pose.world_decomposed(selected_id);
        if state.session.active_transform_tool == crate::session::TransformTool::Translate {
            let origin_world = state.pose.world_position(selected_id);
            let origin_screen = state
                .session
                .camera
                .world_to_screen(origin_world, viewport_size);
            let origin_pos = egui::pos2(rect.min.x + origin_screen.x, rect.min.y + origin_screen.y);

            let angle = selected_world.rotation;
            let dir_x_world = glam::Vec2::new(angle.cos(), angle.sin());
            let dir_y_world = glam::Vec2::new(-angle.sin(), angle.cos());

            let x_p1_world = origin_world + dir_x_world;
            let x_p1_screen = state
                .session
                .camera
                .world_to_screen(x_p1_world, viewport_size);
            let screen_dir_x = (x_p1_screen - origin_screen).normalize_or_zero();
            let dir_x = egui::vec2(screen_dir_x.x, screen_dir_x.y);

            let y_p1_world = origin_world + dir_y_world;
            let y_p1_screen = state
                .session
                .camera
                .world_to_screen(y_p1_world, viewport_size);
            let screen_dir_y = (y_p1_screen - origin_screen).normalize_or_zero();
            let dir_y = egui::vec2(screen_dir_y.x, screen_dir_y.y);

            painter.line_segment(
                [origin_pos - dir_x * 12.0, origin_pos + dir_x * 12.0],
                egui::Stroke::new(3.0, egui::Color32::from_rgb(0, 255, 255)),
            );
            painter.line_segment(
                [origin_pos - dir_y * 12.0, origin_pos + dir_y * 12.0],
                egui::Stroke::new(3.0, egui::Color32::from_rgb(0, 255, 255)),
            );

            let draw_arrow_gizmo =
                |painter: &egui::Painter, dir: egui::Vec2, col: egui::Color32| {
                    let length = 50.0;
                    let tip = origin_pos + dir * length;
                    let right = egui::vec2(-dir.y, dir.x);
                    let base_right = tip - dir * 15.0 + right * 7.0;
                    let inner_base = tip - dir * 10.0;
                    let base_left = tip - dir * 15.0 - right * 7.0;

                    let draw_shape = |offset: egui::Vec2, c: egui::Color32| {
                        painter.line_segment(
                            [origin_pos + offset, inner_base + offset],
                            egui::Stroke::new(3.0, c),
                        );
                        painter.add(egui::Shape::convex_polygon(
                            vec![tip + offset, base_right + offset, inner_base + offset],
                            c,
                            egui::Stroke::NONE,
                        ));
                        painter.add(egui::Shape::convex_polygon(
                            vec![tip + offset, inner_base + offset, base_left + offset],
                            c,
                            egui::Stroke::NONE,
                        ));
                    };

                    draw_shape(egui::vec2(1.0, 1.0), egui::Color32::from_black_alpha(150));
                    draw_shape(egui::vec2(0.0, 0.0), col);
                };

            let x_active = state.session.hovered_gizmo
                == crate::session::GizmoInteraction::TranslateX
                || state.session.dragging_gizmo == crate::session::GizmoInteraction::TranslateX;
            let x_color = if x_active {
                egui::Color32::from_rgb(255, 100, 100)
            } else {
                egui::Color32::RED
            };
            draw_arrow_gizmo(&painter, dir_x, x_color);

            let y_active = state.session.hovered_gizmo
                == crate::session::GizmoInteraction::TranslateY
                || state.session.dragging_gizmo == crate::session::GizmoInteraction::TranslateY;
            let y_color = if y_active {
                egui::Color32::from_rgb(100, 255, 100)
            } else {
                egui::Color32::GREEN
            };
            draw_arrow_gizmo(&painter, dir_y, y_color);

            let center_active = state.session.hovered_gizmo
                == crate::session::GizmoInteraction::TranslateFree
                || state.session.dragging_gizmo == crate::session::GizmoInteraction::TranslateFree;
            painter.circle_filled(origin_pos, 8.0, egui::Color32::from_rgb(20, 20, 20));
            let ring_col = if center_active {
                egui::Color32::from_rgb(255, 255, 255)
            } else {
                egui::Color32::from_rgb(200, 255, 255)
            };
            painter.circle_stroke(origin_pos, 6.0, egui::Stroke::new(3.0, ring_col));
        } else if state.session.active_transform_tool == crate::session::TransformTool::Rotate {
            let origin_world = state.pose.world_position(selected_id);
            let origin_screen = state
                .session
                .camera
                .world_to_screen(origin_world, viewport_size);
            let origin_pos = egui::pos2(rect.min.x + origin_screen.x, rect.min.y + origin_screen.y);

            let angle = selected_world.rotation;
            let dir_x_world = glam::Vec2::new(angle.cos(), angle.sin());
            let dir_y_world = glam::Vec2::new(-angle.sin(), angle.cos());

            let x_p1_world = origin_world + dir_x_world;
            let x_p1_screen = state
                .session
                .camera
                .world_to_screen(x_p1_world, viewport_size);
            let screen_dir_x = (x_p1_screen - origin_screen).normalize_or_zero();
            let dir_x = egui::vec2(screen_dir_x.x, screen_dir_x.y);

            let y_p1_world = origin_world + dir_y_world;
            let y_p1_screen = state
                .session
                .camera
                .world_to_screen(y_p1_world, viewport_size);
            let screen_dir_y = (y_p1_screen - origin_screen).normalize_or_zero();
            let dir_y = egui::vec2(screen_dir_y.x, screen_dir_y.y);

            painter.line_segment(
                [origin_pos - dir_x * 40.0, origin_pos + dir_x * 40.0],
                egui::Stroke::new(3.0, egui::Color32::from_rgb(0, 255, 255)),
            );
            painter.line_segment(
                [origin_pos - dir_y * 40.0, origin_pos + dir_y * 40.0],
                egui::Stroke::new(3.0, egui::Color32::from_rgb(0, 255, 255)),
            );

            let is_hovered =
                state.session.hovered_gizmo == crate::session::GizmoInteraction::Rotate;
            let is_dragging =
                state.session.dragging_gizmo == crate::session::GizmoInteraction::Rotate;
            let red_color = if is_hovered || is_dragging {
                egui::Color32::from_rgb(255, 100, 100)
            } else {
                egui::Color32::RED
            };

            painter.circle_stroke(
                origin_pos + egui::vec2(1.0, 1.0),
                30.0,
                egui::Stroke::new(4.0, egui::Color32::from_black_alpha(150)),
            );
            painter.circle_stroke(origin_pos, 30.0, egui::Stroke::new(4.0, red_color));

            let diamond_center = origin_pos + dir_x * 30.0;
            let right = egui::vec2(-dir_x.y, dir_x.x);
            let p1 = diamond_center + dir_x * 6.0;
            let p2 = diamond_center + right * 4.0;
            let p3 = diamond_center - dir_x * 6.0;
            let p4 = diamond_center - right * 4.0;

            painter.add(egui::Shape::convex_polygon(
                vec![
                    p1 + egui::vec2(1.0, 1.0),
                    p2 + egui::vec2(1.0, 1.0),
                    p3 + egui::vec2(1.0, 1.0),
                    p4 + egui::vec2(1.0, 1.0),
                ],
                egui::Color32::from_black_alpha(150),
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![p1, p2, p3, p4],
                red_color,
                egui::Stroke::NONE,
            ));

            let center_active = state.session.hovered_gizmo
                == crate::session::GizmoInteraction::TranslateFree
                || state.session.dragging_gizmo == crate::session::GizmoInteraction::TranslateFree;
            painter.circle_filled(origin_pos, 8.0, egui::Color32::from_rgb(20, 20, 20));
            let ring_col = if center_active {
                egui::Color32::from_rgb(255, 255, 255)
            } else {
                egui::Color32::from_rgb(200, 255, 255)
            };
            painter.circle_stroke(origin_pos, 6.0, egui::Stroke::new(3.0, ring_col));
        } else if state.session.active_transform_tool == crate::session::TransformTool::Shear {
            draw_shear_gizmo(&painter, rect, state, selected_id, viewport_size);
        }
    }

    // Slots with nothing to show get a placeholder dot; a textured attachment
    // is its own affordance (T-301).
    //
    // This loop used to also paint every mesh in the rig as a flat-coloured
    // weight heat map, through the wgpu pass, whenever the weight tool was
    // active. It ran over the whole draw order rather than the selected mesh, so
    // picking up the brush turned the entire character blue — every unweighted
    // mesh in the rig shaded as "0% of the selected bone", which is true and
    // useless. The overlay above draws the one mesh being worked on.
    for &slot_id in &state.doc.skeleton.draw_order {
        let Some(slot) = state.doc.skeleton.slots.get(slot_id) else {
            continue;
        };
        if !state.doc.skeleton.bones.contains_key(slot.bone)
            || sprite_for_slot(state, slot_id).is_some()
            || state
                .doc
                .skeleton
                .resolve_slot_many(&state.session.skin_stack(), slot_id)
                .is_some()
        {
            continue;
        }
        let pos = state.pose.world_position(slot.bone);
        let screen_pos = state.session.camera.world_to_screen(pos, viewport_size);
        let point = egui::pos2(rect.min.x + screen_pos.x, rect.min.y + screen_pos.y);

        let color = egui::Color32::from_rgba_unmultiplied(
            (slot.color[0] * 255.0) as u8,
            (slot.color[1] * 255.0) as u8,
            (slot.color[2] * 255.0) as u8,
            (slot.color[3] * 255.0) as u8,
        );
        let radius = if state.session.active_slot() == Some(slot_id) {
            8.0
        } else {
            5.0
        };
        painter.circle_filled(point, radius, color);
        painter.circle_stroke(point, radius, egui::Stroke::new(1.0, egui::Color32::BLACK));
    }

    // Textured attachments, in the pose's draw order (back to front) — the
    // animated order, so a draw-order key reorders the artwork live.
    //
    // Clipping (T-405) cuts the geometry rather than using a stencil buffer: the
    // egui render pass this callback runs inside has no stencil attachment, and
    // taking one would mean rendering the viewport to a private texture first.
    // `core::clipping` does the cut, so the runtime — which cannot assume a
    // stencil buffer exists in the host engine either — masks identically.
    //
    // The polygon is carried in **world** space. It is authored in its own
    // bone's local space, and the slots it masks are on other bones; converting
    // once here is the only place both spaces are in hand.
    let mut clip: Option<(Vec<glam::Vec2>, Option<SlotId>)> = None;
    let mut sprite_draws: Vec<SpriteDraw> = Vec::new();
    for &slot_id in &state.pose.draw_order {
        // A clip starts here and runs until its end slot.
        if let Some(Attachment::Clipping(c)) =
            state
                .doc
                .skeleton
                .resolve_posed(&state.session.skin_stack(), &state.pose, slot_id)
        {
            let end = c.end_slot.as_ref().and_then(|name| {
                state
                    .doc
                    .skeleton
                    .slots
                    .iter()
                    .find(|(_, s)| &s.name == name)
                    .map(|(id, _)| id)
            });
            let world = state
                .doc
                .skeleton
                .slots
                .get(slot_id)
                .and_then(|s| state.pose.worlds.get(s.bone));
            if let Some(world) = world {
                let polygon: Vec<glam::Vec2> = c
                    .vertices
                    .iter()
                    .map(|v| world.transform_point(*v))
                    .collect();
                clip = Some((polygon, end));
            }
            continue;
        }

        if let Some(mut draw) = sprite_for_slot(state, slot_id) {
            if let Some((polygon, _)) = &clip
                && polygon.len() >= 3
            {
                let subject: Vec<ankhimate_core::clipping::ClipVertex> = draw
                    .vertices
                    .iter()
                    .map(|v| ankhimate_core::clipping::ClipVertex {
                        position: glam::vec2(v.position[0], v.position[1]),
                        uv: glam::vec2(v.uv[0], v.uv[1]),
                    })
                    .collect();
                // Colour is per-slot here, so the first vertex's is every
                // vertex's; a clipped triangle inherits it unchanged.
                let color = draw.vertices.first().map(|v| v.color).unwrap_or([1.0; 4]);
                let dark = draw.vertices.first().map(|v| v.dark).unwrap_or([0.0; 4]);
                let (clipped, indices) =
                    ankhimate_core::clipping::clip_triangles(&subject, &draw.indices, polygon);
                if indices.is_empty() {
                    // Entirely masked: skip the draw call rather than submit an
                    // empty one.
                    continue;
                }
                draw.vertices = clipped
                    .into_iter()
                    .map(|v| MeshVertex {
                        position: [v.position.x, v.position.y],
                        uv: [v.uv.x, v.uv.y],
                        color,
                        dark,
                    })
                    .collect();
                draw.indices = indices;
            }
            sprite_draws.push(draw);
        }

        // Past the end slot, the clip stops applying.
        if let Some((_, Some(end))) = &clip
            && *end == slot_id
        {
            clip = None;
        }
    }

    let custom_callback = CustomCallback {
        view_proj: state.session.camera.view_proj_matrix(viewport_size),
        mesh_draws,
        sprite_draws,
        sprite_uploads,
    };

    // Into the slot reserved before anything was drawn, so the artwork lands
    // under every gizmo while staying in the canvas's own layer.
    canvas_painter.set(
        artwork_slot,
        eframe::egui_wgpu::Callback::new_paint_callback(rect, custom_callback),
    );
}
