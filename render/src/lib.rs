//! Deterministic, transport-free headless rendering.
//!
//! This crate is shared infrastructure for rendered exports and MCP previews. It
//! owns no files, windows, GPU device, or protocol objects; callers provide a
//! document and receive encoded PNG bytes.

use ankhimate_core::{
    attachment::Attachment,
    clipping::{ClipVertex, clip_triangles},
    ids::{BoneId, SlotId},
    pose::{Pose, evaluate},
    slot::BlendMode,
};
use ankhimate_document::Document;
use glam::Vec2;
use image::{ImageEncoder, RgbaImage, codecs::png::PngEncoder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const DEFAULT_PADDING: f32 = 0.08;
const LABEL_HEIGHT: u32 = 13;

#[derive(Debug)]
pub enum Error {
    Invalid(String),
    Image(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) | Self::Image(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FocusMode {
    #[default]
    Dim,
    Isolate,
    SkeletonOnly,
    ArtOnly,
}

fn default_other_opacity() -> f32 {
    0.12
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    pub bones: Vec<String>,
    pub include_descendants: bool,
    pub mode: FocusMode,
    pub other_opacity: f32,
    pub show_bone_names: bool,
    pub show_joint_points: bool,
    pub show_constraint_targets: bool,
    pub motion_trails: Vec<String>,
}

impl Default for Focus {
    fn default() -> Self {
        Self {
            bones: Vec::new(),
            include_descendants: false,
            mode: FocusMode::Dim,
            other_opacity: default_other_opacity(),
            show_bone_names: false,
            show_joint_points: false,
            show_constraint_targets: false,
            motion_trails: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    /// World-space point at the center. Omit with `zoom` for automatic fitting.
    pub center: Option<[f32; 2]>,
    /// Screen pixels per world unit. Omit with `center` for automatic fitting.
    pub zoom: Option<f32>,
    /// Fraction of each viewport reserved around automatically fitted content.
    pub padding: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: None,
            zoom: None,
            padding: DEFAULT_PADDING,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedCamera {
    pub center: Vec2,
    pub zoom: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameRequest {
    pub animation: Option<String>,
    pub time: f32,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 4],
    pub camera: Camera,
    pub focus: Option<Focus>,
}

impl Default for FrameRequest {
    fn default() -> Self {
        Self {
            animation: None,
            time: 0.0,
            width: 512,
            height: 512,
            background: [30, 30, 34, 255],
            camera: Camera::default(),
            focus: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactSheetRequest {
    pub animation: Option<String>,
    /// Explicit times. When empty, `frame_count` evenly spans the animation.
    pub times: Vec<f32>,
    pub frame_count: u32,
    pub columns: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 4],
    pub camera: Camera,
    pub focus: Option<Focus>,
}

impl Default for ContactSheetRequest {
    fn default() -> Self {
        Self {
            animation: None,
            times: Vec::new(),
            frame_count: 6,
            columns: None,
            width: 1024,
            height: 768,
            background: [30, 30, 34, 255],
            camera: Camera::default(),
            focus: None,
        }
    }
}

pub struct RenderedPng {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn render_frame(doc: &Document, request: &FrameRequest) -> Result<RenderedPng, Error> {
    validate_size(request.width, request.height)?;
    let pose = pose_at(doc, request.animation.as_deref(), request.time)?;
    let focus = ResolvedFocus::new(doc, request.focus.as_ref())?;
    let draws = build_draws(doc, &pose, &focus)?;
    let bounds = bounds_for(doc, &pose, &draws, &focus);
    let camera = resolve_camera(&request.camera, bounds, request.width, request.height)?;
    let mut canvas = Canvas::new(request.width, request.height, request.background);
    let textures = decode_textures(doc);
    let trail_poses = [(request.time, &pose)];
    let scene = PaintContext {
        doc,
        focus: &focus,
        camera,
        trail_poses: &trail_poses,
        textures: &textures,
    };
    paint_scene(
        &mut canvas,
        Viewport::new(0, 0, request.width, request.height),
        &pose,
        &draws,
        &scene,
    );
    Ok(RenderedPng {
        bytes: canvas.png()?,
        width: request.width,
        height: request.height,
    })
}

pub fn render_contact_sheet(
    doc: &Document,
    request: &ContactSheetRequest,
) -> Result<RenderedPng, Error> {
    validate_size(request.width, request.height)?;
    let times = contact_times(doc, request)?;
    let columns = request
        .columns
        .unwrap_or_else(|| (times.len() as f32).sqrt().ceil() as u32)
        .clamp(1, times.len() as u32);
    let rows = (times.len() as u32).div_ceil(columns);
    let cell_width = request.width / columns;
    let cell_height = request.height / rows;
    if cell_width == 0 || cell_height <= LABEL_HEIGHT {
        return Err(Error::Invalid("contact-sheet cells are too small".into()));
    }

    let focus = ResolvedFocus::new(doc, request.focus.as_ref())?;
    let poses: Vec<Pose> = times
        .iter()
        .map(|time| pose_at(doc, request.animation.as_deref(), *time))
        .collect::<Result<_, _>>()?;
    let draws: Vec<Vec<Draw>> = poses
        .iter()
        .map(|pose| build_draws(doc, pose, &focus))
        .collect::<Result<_, _>>()?;

    // One union, one camera, every cell. Never fit cells independently: doing
    // so makes translation look like scale and defeats a contact sheet.
    let mut union = Bounds::empty();
    for (pose, scene) in poses.iter().zip(&draws) {
        union.include_bounds(bounds_for(doc, pose, scene, &focus));
    }
    let camera = resolve_camera(
        &request.camera,
        union,
        cell_width,
        cell_height - LABEL_HEIGHT,
    )?;
    let trail_poses: Vec<(f32, &Pose)> = times.iter().copied().zip(poses.iter()).collect();
    let textures = decode_textures(doc);
    let paint = PaintContext {
        doc,
        focus: &focus,
        camera,
        trail_poses: &trail_poses,
        textures: &textures,
    };
    let mut canvas = Canvas::new(request.width, request.height, request.background);
    for (index, ((time, pose), scene)) in times.iter().zip(&poses).zip(&draws).enumerate() {
        let col = index as u32 % columns;
        let row = index as u32 / columns;
        let x = col * cell_width;
        let y = row * cell_height;
        let viewport = Viewport::new(x, y + LABEL_HEIGHT, cell_width, cell_height - LABEL_HEIGHT);
        paint_scene(&mut canvas, viewport, pose, scene, &paint);
        canvas.text(x + 3, y + 3, &format!("{time:.3}s"), [235, 235, 235, 255]);
        canvas.rect_outline(x, y, cell_width, cell_height, [90, 90, 96, 255]);
    }
    Ok(RenderedPng {
        bytes: canvas.png()?,
        width: request.width,
        height: request.height,
    })
}

fn validate_size(width: u32, height: u32) -> Result<(), Error> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(Error::Invalid(
            "width and height must be between 1 and 8192".into(),
        ));
    }
    Ok(())
}

fn animation<'a>(
    doc: &'a Document,
    name: Option<&str>,
) -> Result<Option<&'a ankhimate_core::animation::Animation>, Error> {
    name.map(|name| {
        doc.animations
            .values()
            .find(|animation| animation.name == name)
            .ok_or_else(|| Error::Invalid(format!("no animation named `{name}`")))
    })
    .transpose()
}

fn pose_at(doc: &Document, animation_name: Option<&str>, time: f32) -> Result<Pose, Error> {
    if !time.is_finite() {
        return Err(Error::Invalid("time must be finite".into()));
    }
    let animation = animation(doc, animation_name)?;
    let mut pose = Pose::new();
    match animation {
        Some(animation) => evaluate(&doc.skeleton, &[(animation, time, 1.0)], &mut pose),
        None => evaluate(&doc.skeleton, &[], &mut pose),
    }
    Ok(pose)
}

fn contact_times(doc: &Document, request: &ContactSheetRequest) -> Result<Vec<f32>, Error> {
    if !request.times.is_empty() {
        if request.times.iter().any(|time| !time.is_finite()) {
            return Err(Error::Invalid(
                "every contact-sheet time must be finite".into(),
            ));
        }
        return Ok(request.times.clone());
    }
    let count = request.frame_count.clamp(1, 100) as usize;
    let duration = animation(doc, request.animation.as_deref())?
        .map(|animation| animation.duration)
        .unwrap_or(0.0);
    if count == 1 {
        return Ok(vec![0.0]);
    }
    Ok((0..count)
        .map(|index| duration * index as f32 / (count - 1) as f32)
        .collect())
}

struct ResolvedFocus {
    settings: Option<Focus>,
    bones: HashSet<BoneId>,
    trails: Vec<BoneId>,
}

impl ResolvedFocus {
    fn new(doc: &Document, settings: Option<&Focus>) -> Result<Self, Error> {
        let Some(settings) = settings else {
            return Ok(Self {
                settings: None,
                bones: HashSet::new(),
                trails: Vec::new(),
            });
        };
        let find = |name: &str| {
            doc.skeleton
                .bones
                .iter()
                .find(|(_, bone)| bone.name == name)
                .map(|(id, _)| id)
                .ok_or_else(|| Error::Invalid(format!("no bone named `{name}`")))
        };
        let mut bones: HashSet<BoneId> = settings
            .bones
            .iter()
            .map(|name| find(name))
            .collect::<Result<_, _>>()?;
        if settings.include_descendants {
            let selected = bones.clone();
            for (id, _) in doc.skeleton.bones.iter() {
                let mut parent = doc.skeleton.bones[id].parent;
                while let Some(candidate) = parent {
                    if selected.contains(&candidate) {
                        bones.insert(id);
                        break;
                    }
                    parent = doc
                        .skeleton
                        .bones
                        .get(candidate)
                        .and_then(|bone| bone.parent);
                }
            }
        }
        let trails: Vec<BoneId> = settings
            .motion_trails
            .iter()
            .map(|name| find(name))
            .collect::<Result<_, _>>()?;
        if let Some(trail) = trails.iter().find(|bone| !bones.contains(bone)) {
            let name = &doc.skeleton.bones[*trail].name;
            return Err(Error::Invalid(format!(
                "motion trail `{name}` is not among the focused bones"
            )));
        }
        Ok(Self {
            settings: Some(settings.clone()),
            bones,
            trails,
        })
    }

    fn art_alpha(&self, bone: BoneId) -> Option<f32> {
        let Some(settings) = &self.settings else {
            return Some(1.0);
        };
        if settings.mode == FocusMode::SkeletonOnly {
            return None;
        }
        if settings.bones.is_empty() {
            return Some(1.0);
        }
        if self.bones.contains(&bone) {
            return Some(1.0);
        }
        match settings.mode {
            FocusMode::Dim => Some(settings.other_opacity.clamp(0.0, 1.0)),
            FocusMode::Isolate | FocusMode::SkeletonOnly | FocusMode::ArtOnly => None,
        }
    }

    fn show_bone(&self, bone: BoneId) -> bool {
        self.settings.as_ref().is_some_and(|settings| {
            settings.mode != FocusMode::ArtOnly && self.bones.contains(&bone)
        })
    }
}

#[derive(Clone)]
struct Vertex {
    world: Vec2,
    uv: Vec2,
    color: [f32; 4],
    dark: [f32; 4],
}

struct Texture {
    image: RgbaImage,
}

struct Draw {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    texture: String,
    blend: BlendMode,
}

fn build_draws(doc: &Document, pose: &Pose, focus: &ResolvedFocus) -> Result<Vec<Draw>, Error> {
    let skins = [doc.skeleton.default_skin];
    let mut clip: Option<(Vec<Vec2>, Option<SlotId>)> = None;
    let mut draws = Vec::new();
    for &slot_id in &pose.draw_order {
        let Some(slot) = doc.skeleton.slots.get(slot_id) else {
            continue;
        };
        let attachment = doc.skeleton.resolve_posed(&skins, pose, slot_id);
        if let Some(Attachment::Clipping(clipping)) = attachment {
            let end = clipping.end_slot.as_ref().and_then(|name| {
                doc.skeleton
                    .slots
                    .iter()
                    .find(|(_, slot)| &slot.name == name)
                    .map(|(id, _)| id)
            });
            let world = pose.world(slot.bone);
            clip = Some((
                clipping
                    .vertices
                    .iter()
                    .map(|point| world.transform_point(*point))
                    .collect(),
                end,
            ));
            continue;
        }

        let alpha = focus.art_alpha(slot.bone);
        if alpha.is_some()
            && pose.slot_visible.get(slot_id) != Some(&false)
            && let Some(attachment) = attachment
            && let Some(mut draw) =
                attachment_draw(doc, pose, slot_id, slot, attachment, alpha.unwrap())?
        {
            if let Some((polygon, _)) = &clip
                && polygon.len() >= 3
            {
                let subject: Vec<ClipVertex> = draw
                    .vertices
                    .iter()
                    .map(|vertex| ClipVertex {
                        position: vertex.world,
                        uv: vertex.uv,
                    })
                    .collect();
                let (vertices, indices) = clip_triangles(&subject, &draw.indices, polygon);
                if !indices.is_empty() {
                    let template = draw.vertices[0].clone();
                    draw.vertices = vertices
                        .into_iter()
                        .map(|vertex| Vertex {
                            world: vertex.position,
                            uv: vertex.uv,
                            ..template.clone()
                        })
                        .collect();
                    draw.indices = indices;
                    draws.push(draw);
                }
            } else {
                draws.push(draw);
            }
        }
        if clip.as_ref().and_then(|(_, end)| *end) == Some(slot_id) {
            clip = None;
        }
    }
    Ok(draws)
}

fn attachment_draw(
    doc: &Document,
    pose: &Pose,
    slot_id: SlotId,
    slot: &ankhimate_core::slot::Slot,
    attachment: &Attachment,
    alpha: f32,
) -> Result<Option<Draw>, Error> {
    let mut color = pose.slot_colors.get(slot_id).copied().unwrap_or(slot.color);
    color[3] *= alpha;
    let dark = pose
        .slot_dark_colors
        .get(slot_id)
        .copied()
        .or(slot.dark_color)
        .unwrap_or([0.0; 4]);
    let sequence_texture = |sequence: &Option<ankhimate_core::attachment::Sequence>| {
        let sequence = sequence.as_ref()?;
        let frame = pose.slot_sequence_frames.get(slot_id).copied()?;
        sequence.frame(frame).cloned()
    };
    match attachment {
        Attachment::Region(region) => {
            let texture =
                sequence_texture(&region.sequence).unwrap_or_else(|| region.texture.clone());
            if doc.assets.by_name(&texture).is_none() {
                return Ok(None);
            }
            let world = pose.world(slot.bone);
            let corners = region
                .local_corners()
                .map(|point| world.transform_point(point));
            let uv = &region.uv_rect;
            let uvs = [
                Vec2::new(uv.x, uv.y),
                Vec2::new(uv.x, uv.y + uv.h),
                Vec2::new(uv.x + uv.w, uv.y + uv.h),
                Vec2::new(uv.x + uv.w, uv.y),
            ];
            Ok(Some(Draw {
                vertices: (0..4)
                    .map(|i| Vertex {
                        world: corners[i],
                        uv: uvs[i],
                        color,
                        dark,
                    })
                    .collect(),
                indices: vec![0, 1, 2, 0, 2, 3],
                texture,
                blend: slot.blend_mode,
            }))
        }
        Attachment::Mesh(mesh) => {
            let geometry = doc
                .skeleton
                .resolve_linked_mesh(&[doc.skeleton.default_skin], mesh);
            if geometry.triangles.is_empty() {
                return Ok(None);
            }
            let texture = sequence_texture(&mesh.sequence).unwrap_or_else(|| mesh.texture.clone());
            if doc.assets.by_name(&texture).is_none() {
                return Ok(None);
            }
            let deform = pose
                .attachment_name(&doc.skeleton, slot_id)
                .and_then(|name| pose.deforms.get(&(slot_id, name.to_string())));
            let weighted = !geometry.weights.is_empty();
            let bone_world = pose.world(slot.bone);
            let vertices = (0..geometry.setup_vertices.len())
                .map(|index| {
                    let offsets: Vec<Vec2> = match deform {
                        Some(values) if weighted => values
                            .get(geometry.influence_range(index))
                            .unwrap_or_default()
                            .to_vec(),
                        Some(values) => vec![values.get(index).copied().unwrap_or_default()],
                        None => Vec::new(),
                    };
                    Vertex {
                        world: geometry.skin_vertex_with_deform(index, &offsets, pose, &bone_world),
                        uv: geometry.uvs.get(index).copied().unwrap_or_default(),
                        color,
                        dark,
                    }
                })
                .collect();
            Ok(Some(Draw {
                vertices,
                indices: geometry
                    .triangles
                    .iter()
                    .flat_map(|triangle| *triangle)
                    .collect(),
                texture,
                blend: slot.blend_mode,
            }))
        }
        Attachment::Clipping(_)
        | Attachment::Path(_)
        | Attachment::BoundingBox(_)
        | Attachment::Point(_) => Ok(None),
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec2,
    max: Vec2,
    any: bool,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            min: Vec2::splat(f32::MAX),
            max: Vec2::splat(f32::MIN),
            any: false,
        }
    }
    fn include(&mut self, point: Vec2) {
        if point.is_finite() {
            self.min = self.min.min(point);
            self.max = self.max.max(point);
            self.any = true;
        }
    }
    fn include_bounds(&mut self, other: Self) {
        if other.any {
            self.include(other.min);
            self.include(other.max);
        }
    }
}

fn bounds_for(doc: &Document, pose: &Pose, draws: &[Draw], focus: &ResolvedFocus) -> Bounds {
    let mut bounds = Bounds::empty();
    for draw in draws {
        for vertex in &draw.vertices {
            bounds.include(vertex.world);
        }
    }
    for &bone in &doc.skeleton.update_order {
        if draws.is_empty() || focus.show_bone(bone) {
            bounds.include(pose.world_position(bone));
            bounds.include(pose.world_tip(&doc.skeleton, bone));
        }
    }
    if !bounds.any {
        bounds.include(Vec2::ZERO);
        bounds.include(Vec2::ONE);
    }
    bounds
}

fn resolve_camera(
    spec: &Camera,
    bounds: Bounds,
    width: u32,
    height: u32,
) -> Result<ResolvedCamera, Error> {
    match (spec.center, spec.zoom) {
        (Some(center), Some(zoom)) if zoom.is_finite() && zoom > 0.0 => Ok(ResolvedCamera {
            center: Vec2::from(center),
            zoom,
        }),
        (None, None) => {
            let center = (bounds.min + bounds.max) * 0.5;
            let size = (bounds.max - bounds.min).max(Vec2::splat(1.0));
            let padding = spec.padding.clamp(0.0, 0.45);
            let usable = Vec2::new(width as f32, height as f32) * (1.0 - padding * 2.0);
            Ok(ResolvedCamera {
                center,
                zoom: (usable.x / size.x).min(usable.y / size.y).max(0.0001),
            })
        }
        _ => Err(Error::Invalid(
            "camera.center and camera.zoom must be supplied together".into(),
        )),
    }
}

#[derive(Clone, Copy)]
struct Viewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}
impl Viewport {
    fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

fn screen(point: Vec2, camera: ResolvedCamera, viewport: Viewport) -> Vec2 {
    let offset = (point - camera.center) * camera.zoom;
    Vec2::new(
        viewport.x as f32 + viewport.width as f32 * 0.5 + offset.x,
        viewport.y as f32 + viewport.height as f32 * 0.5 - offset.y,
    )
}

struct PaintContext<'a, 'pose> {
    doc: &'a Document,
    focus: &'a ResolvedFocus,
    camera: ResolvedCamera,
    trail_poses: &'a [(f32, &'pose Pose)],
    textures: &'a HashMap<String, Texture>,
}

fn decode_textures(doc: &Document) -> HashMap<String, Texture> {
    doc.assets
        .images
        .values()
        .filter_map(|asset| {
            image::load_from_memory(&asset.bytes).ok().map(|image| {
                (
                    asset.name.clone(),
                    Texture {
                        image: image.to_rgba8(),
                    },
                )
            })
        })
        .collect()
}

fn paint_scene(
    canvas: &mut Canvas,
    viewport: Viewport,
    pose: &Pose,
    draws: &[Draw],
    context: &PaintContext<'_, '_>,
) {
    for draw in draws {
        let Some(texture) = context.textures.get(&draw.texture) else {
            continue;
        };
        for triangle in draw.indices.chunks_exact(3) {
            let vertices = [
                &draw.vertices[triangle[0] as usize],
                &draw.vertices[triangle[1] as usize],
                &draw.vertices[triangle[2] as usize],
            ];
            canvas.textured_triangle(
                viewport,
                vertices.map(|vertex| screen(vertex.world, context.camera, viewport)),
                vertices,
                texture,
                draw.blend,
            );
        }
    }
    let Some(settings) = &context.focus.settings else {
        return;
    };
    if settings.mode != FocusMode::ArtOnly {
        for &bone in &context.doc.skeleton.update_order {
            if !context.focus.show_bone(bone) {
                continue;
            }
            let origin = screen(pose.world_position(bone), context.camera, viewport);
            let tip = screen(
                pose.world_tip(&context.doc.skeleton, bone),
                context.camera,
                viewport,
            );
            canvas.line(origin, tip, 2.0, [255, 174, 30, 240]);
            if settings.show_joint_points {
                canvas.circle(origin, 3.0, [255, 225, 120, 255]);
            }
            if settings.show_bone_names
                && let Some(data) = context.doc.skeleton.bones.get(bone)
            {
                canvas.text(
                    (origin.x + 4.0).max(0.0) as u32,
                    (origin.y - 8.0).max(0.0) as u32,
                    &data.name,
                    [255, 236, 180, 255],
                );
            }
        }
        if settings.show_constraint_targets {
            for constraint in context.doc.skeleton.constraints.values() {
                if let Some(target) = constraint.target()
                    && (context.focus.bones.contains(&target)
                        || constraint
                            .affected_bones()
                            .iter()
                            .any(|bone| context.focus.bones.contains(bone)))
                {
                    let point = screen(pose.world_position(target), context.camera, viewport);
                    canvas.cross(point, 5.0, [255, 70, 100, 255]);
                }
            }
        }
        for &bone in &context.focus.trails {
            let points: Vec<Vec2> = context
                .trail_poses
                .iter()
                .map(|(_, trail_pose)| {
                    screen(
                        trail_pose.world_tip(&context.doc.skeleton, bone),
                        context.camera,
                        viewport,
                    )
                })
                .collect();
            for pair in points.windows(2) {
                canvas.line(pair[0], pair[1], 1.5, [80, 220, 255, 210]);
            }
            for point in points {
                canvas.circle(point, 2.0, [120, 235, 255, 240]);
            }
        }
    }
}

struct Canvas {
    image: RgbaImage,
}

impl Canvas {
    fn new(width: u32, height: u32, background: [u8; 4]) -> Self {
        Self {
            image: RgbaImage::from_pixel(width, height, image::Rgba(background)),
        }
    }
    fn png(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                self.image.as_raw(),
                self.image.width(),
                self.image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| Error::Image(error.to_string()))?;
        Ok(bytes)
    }
    fn textured_triangle(
        &mut self,
        viewport: Viewport,
        points: [Vec2; 3],
        vertices: [&Vertex; 3],
        texture: &Texture,
        blend: BlendMode,
    ) {
        let min = points
            .iter()
            .fold(Vec2::splat(f32::MAX), |value, point| value.min(*point))
            .floor();
        let max = points
            .iter()
            .fold(Vec2::splat(f32::MIN), |value, point| value.max(*point))
            .ceil();
        let x0 = (min.x.max(viewport.x as f32).max(0.0)) as u32;
        let y0 = (min.y.max(viewport.y as f32).max(0.0)) as u32;
        let x1 = (max
            .x
            .min((viewport.x + viewport.width).saturating_sub(1) as f32)
            .min(self.image.width().saturating_sub(1) as f32)) as u32;
        let y1 = (max
            .y
            .min((viewport.y + viewport.height).saturating_sub(1) as f32)
            .min(self.image.height().saturating_sub(1) as f32)) as u32;
        let area = edge(points[0], points[1], points[2]);
        if area.abs() < 1e-6 || x0 > x1 || y0 > y1 {
            return;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let point = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let weights = [
                    edge(points[1], points[2], point) / area,
                    edge(points[2], points[0], point) / area,
                    edge(points[0], points[1], point) / area,
                ];
                if weights.iter().any(|weight| *weight < -1e-5) {
                    continue;
                }
                let uv = vertices
                    .iter()
                    .zip(weights)
                    .fold(Vec2::ZERO, |sum, (vertex, weight)| sum + vertex.uv * weight);
                let color = mix4(vertices.map(|vertex| vertex.color), weights);
                let dark = mix4(vertices.map(|vertex| vertex.dark), weights);
                let texel = sample(&texture.image, uv);
                let source = [
                    texel[0] * color[0] + (1.0 - texel[0]) * dark[0] * dark[3],
                    texel[1] * color[1] + (1.0 - texel[1]) * dark[1] * dark[3],
                    texel[2] * color[2] + (1.0 - texel[2]) * dark[2] * dark[3],
                    texel[3] * color[3],
                ];
                self.blend(x, y, source, blend);
            }
        }
    }
    fn blend(&mut self, x: u32, y: u32, source: [f32; 4], mode: BlendMode) {
        let destination = self
            .image
            .get_pixel(x, y)
            .0
            .map(|channel| channel as f32 / 255.0);
        let sa = source[3].clamp(0.0, 1.0);
        let mut output = [0.0; 4];
        match mode {
            BlendMode::Normal => {
                for i in 0..3 {
                    output[i] = source[i] * sa + destination[i] * (1.0 - sa);
                }
            }
            BlendMode::Additive => {
                for i in 0..3 {
                    output[i] = source[i] * sa + destination[i];
                }
            }
            BlendMode::Multiply => {
                for i in 0..3 {
                    output[i] = source[i] * destination[i] + destination[i] * (1.0 - sa);
                }
            }
            BlendMode::Screen => {
                for i in 0..3 {
                    output[i] = source[i] + destination[i] * (1.0 - source[i]);
                }
            }
        }
        output[3] = match mode {
            BlendMode::Normal => sa + destination[3] * (1.0 - sa),
            BlendMode::Additive => destination[3],
            BlendMode::Multiply => destination[3],
            BlendMode::Screen => sa + destination[3] * (1.0 - sa),
        };
        *self.image.get_pixel_mut(x, y) =
            image::Rgba(output.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8));
    }
    fn line(&mut self, from: Vec2, to: Vec2, width: f32, color: [u8; 4]) {
        let distance = (to - from).length();
        let steps = distance.ceil().max(1.0) as u32;
        for step in 0..=steps {
            self.circle(
                from.lerp(to, step as f32 / steps as f32),
                width * 0.5,
                color,
            );
        }
    }
    fn circle(&mut self, center: Vec2, radius: f32, color: [u8; 4]) {
        let min_x = (center.x - radius).floor().max(0.0) as u32;
        let max_x = (center.x + radius)
            .ceil()
            .min(self.image.width().saturating_sub(1) as f32) as u32;
        let min_y = (center.y - radius).floor().max(0.0) as u32;
        let max_y = (center.y + radius)
            .ceil()
            .min(self.image.height().saturating_sub(1) as f32) as u32;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                if (Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - center).length_squared()
                    <= radius * radius
                {
                    self.blend(x, y, color.map(|v| v as f32 / 255.0), BlendMode::Normal);
                }
            }
        }
    }
    fn cross(&mut self, center: Vec2, radius: f32, color: [u8; 4]) {
        self.line(
            center - Vec2::splat(radius),
            center + Vec2::splat(radius),
            1.5,
            color,
        );
        self.line(
            center + Vec2::new(-radius, radius),
            center + Vec2::new(radius, -radius),
            1.5,
            color,
        );
    }
    fn rect_outline(&mut self, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
        if width == 0 || height == 0 {
            return;
        }
        self.line(
            Vec2::new(x as f32, y as f32),
            Vec2::new((x + width - 1) as f32, y as f32),
            1.0,
            color,
        );
        self.line(
            Vec2::new(x as f32, (y + height - 1) as f32),
            Vec2::new((x + width - 1) as f32, (y + height - 1) as f32),
            1.0,
            color,
        );
    }
    fn text(&mut self, x: u32, y: u32, text: &str, color: [u8; 4]) {
        let mut cursor = x;
        for character in text.chars() {
            draw_glyph(self, cursor, y, character, color);
            cursor += 4;
        }
    }
}

fn edge(a: Vec2, b: Vec2, point: Vec2) -> f32 {
    (point.x - a.x) * (b.y - a.y) - (point.y - a.y) * (b.x - a.x)
}
fn mix4(values: [[f32; 4]; 3], weights: [f32; 3]) -> [f32; 4] {
    std::array::from_fn(|channel| {
        values[0][channel] * weights[0]
            + values[1][channel] * weights[1]
            + values[2][channel] * weights[2]
    })
}
fn sample(image: &RgbaImage, uv: Vec2) -> [f32; 4] {
    let position = Vec2::new(
        uv.x.clamp(0.0, 1.0) * image.width().saturating_sub(1) as f32,
        uv.y.clamp(0.0, 1.0) * image.height().saturating_sub(1) as f32,
    );
    let x0 = position.x.floor() as u32;
    let y0 = position.y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let tx = position.x - x0 as f32;
    let ty = position.y - y0 as f32;
    let pixel = |x, y| {
        image
            .get_pixel(x, y)
            .0
            .map(|channel| channel as f32 / 255.0)
    };
    let top = mix_pixel(pixel(x0, y0), pixel(x1, y0), tx);
    let bottom = mix_pixel(pixel(x0, y1), pixel(x1, y1), tx);
    mix_pixel(top, bottom, ty)
}
fn mix_pixel(a: [f32; 4], b: [f32; 4], amount: f32) -> [f32; 4] {
    std::array::from_fn(|channel| a[channel] + (b[channel] - a[channel]) * amount)
}

// Compact deterministic 3x5 font. Names are uppercased; unsupported glyphs
// become a box. It avoids a platform font dependency in headless/server builds.
fn draw_glyph(canvas: &mut Canvas, x: u32, y: u32, character: char, color: [u8; 4]) {
    let rows = glyph(character.to_ascii_uppercase());
    for (dy, row) in rows.iter().enumerate() {
        for dx in 0..3 {
            if row & (1 << (2 - dx)) != 0 {
                let px = x + dx;
                let py = y + dy as u32;
                if px < canvas.image.width() && py < canvas.image.height() {
                    canvas.blend(px, py, color.map(|v| v as f32 / 255.0), BlendMode::Normal);
                }
            }
        }
    }
}
fn glyph(c: char) -> [u8; 5] {
    match c {
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [5, 7, 7, 7, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 3, 1],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [6, 1, 7, 4, 7],
        '3' => [6, 1, 3, 1, 6],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 6, 1, 6],
        '6' => [3, 4, 7, 5, 7],
        '7' => [7, 1, 2, 2, 2],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 6],
        '.' => [0, 0, 0, 0, 2],
        '-' => [0, 0, 7, 0, 0],
        ':' => [0, 2, 0, 2, 0],
        '_' => [0, 0, 0, 0, 7],
        ' ' => [0; 5],
        _ => [7, 5, 5, 5, 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankhimate_core::{
        animation::{Animation, Axis, Key, Timeline},
        assets::ImageAsset,
        attachment::{MeshAttachment, Rect, RegionAttachment, VertexWeight},
        skeleton::Bone,
        slot::Slot,
        transforms::Inherit,
    };

    fn png_asset(name: &str) -> ImageAsset {
        let image = RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255]));
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(image.as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        ImageAsset::new(name, bytes, 4, 4)
    }

    fn region(texture: &str) -> Attachment {
        Attachment::Region(RegionAttachment {
            texture: texture.into(),
            local_offset: Vec2::ZERO,
            local_rotation: 0.0,
            local_scale: Vec2::ONE,
            width: 16.0,
            height: 16.0,
            uv_rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            pivot: Vec2::splat(0.5),
            sequence: None,
        })
    }

    fn rig() -> Document {
        let mut doc = Document::new();
        doc.assets.add(png_asset("white"));
        let root = doc.skeleton.add_bone(Bone {
            name: "root".into(),
            parent: None,
            length: 20.0,
            local_transform: Default::default(),
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        let child_transform = ankhimate_core::math::Transform {
            position: Vec2::new(28.0, 0.0),
            ..Default::default()
        };
        let child = doc.skeleton.add_bone(Bone {
            name: "child".into(),
            parent: Some(root),
            length: 15.0,
            local_transform: child_transform,
            inherit: Inherit::default(),
            color: Bone::default_color(),
        });
        let mut root_slot = Slot::new("root_art".into(), root);
        root_slot.attachment = Some("root_region".into());
        root_slot.color = [1.0, 0.1, 0.1, 1.0];
        let root_slot = doc.skeleton.add_slot(root_slot);
        let mut child_slot = Slot::new("child_art".into(), child);
        child_slot.attachment = Some("child_region".into());
        child_slot.color = [0.1, 1.0, 0.1, 1.0];
        let child_slot = doc.skeleton.add_slot(child_slot);
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(root_slot, "root_region", region("white"));
        doc.skeleton.skins[skin].set(child_slot, "child_region", region("white"));
        let mut animation = Animation::new("move", 1.0);
        animation.timelines.push(Timeline::BoneTranslate {
            bone: child,
            axis: Axis::Y,
            keys: vec![Key::linear(0.0, 0.0), Key::linear(1.0, 25.0)],
        });
        doc.animations.insert(animation);
        doc
    }

    fn fixed_frame(time: f32) -> FrameRequest {
        FrameRequest {
            animation: Some("move".into()),
            time,
            width: 160,
            height: 120,
            camera: Camera {
                center: Some([15.0, 10.0]),
                zoom: Some(2.0),
                padding: 0.0,
            },
            ..FrameRequest::default()
        }
    }

    #[test]
    fn animation_changes_pixels_and_identical_inputs_are_byte_identical() {
        let doc = rig();
        let first = render_frame(&doc, &fixed_frame(0.0)).unwrap();
        let repeated = render_frame(&doc, &fixed_frame(0.0)).unwrap();
        let later = render_frame(&doc, &fixed_frame(1.0)).unwrap();
        assert_eq!(first.bytes, repeated.bytes, "PNG encoding is deterministic");
        assert_ne!(
            first.bytes, later.bytes,
            "animation time moved rendered pixels"
        );
        assert!(first.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn focus_only_filters_visuals_and_descendants_are_opt_in() {
        let doc = rig();
        let before = ankhimate_document::read::describe(&doc);
        let pose_before = pose_at(&doc, Some("move"), 0.5).unwrap();
        let mut without = fixed_frame(0.5);
        without.focus = Some(Focus {
            bones: vec!["root".into()],
            mode: FocusMode::Isolate,
            ..Focus::default()
        });
        let without = render_frame(&doc, &without).unwrap();
        let mut with = fixed_frame(0.5);
        with.focus = Some(Focus {
            bones: vec!["root".into()],
            include_descendants: true,
            mode: FocusMode::Isolate,
            ..Focus::default()
        });
        let with = render_frame(&doc, &with).unwrap();
        assert_ne!(
            without.bytes, with.bytes,
            "including the child reveals its artwork"
        );
        assert_eq!(
            before,
            ankhimate_document::read::describe(&doc),
            "rendering never mutates the document"
        );
        let pose_after = pose_at(&doc, Some("move"), 0.5).unwrap();
        for &bone in &doc.skeleton.update_order {
            assert_eq!(
                pose_before.world(bone),
                pose_after.world(bone),
                "visual focus cannot affect evaluation"
            );
        }
    }

    #[test]
    fn dim_and_isolate_change_pixels() {
        let doc = rig();
        let mut dim = fixed_frame(0.0);
        dim.focus = Some(Focus {
            bones: vec!["root".into()],
            mode: FocusMode::Dim,
            other_opacity: 0.12,
            ..Focus::default()
        });
        let mut isolate = dim.clone();
        isolate.focus.as_mut().unwrap().mode = FocusMode::Isolate;
        assert_ne!(
            render_frame(&doc, &dim).unwrap().bytes,
            render_frame(&doc, &isolate).unwrap().bytes
        );
    }

    #[test]
    fn contact_sheet_uses_one_camera_for_every_cell() {
        let doc = rig();
        let request = ContactSheetRequest {
            animation: Some("move".into()),
            times: vec![0.0, 1.0],
            columns: Some(2),
            width: 320,
            height: 140,
            ..ContactSheetRequest::default()
        };
        let png = render_contact_sheet(&doc, &request).unwrap();
        let image = image::load_from_memory(&png.bytes).unwrap().to_rgba8();
        let green_centroid = |start_x: u32, end_x: u32| {
            let mut sum_y = 0_u64;
            let mut count = 0_u64;
            for y in LABEL_HEIGHT..image.height() {
                for x in start_x..end_x {
                    let pixel = image.get_pixel(x, y).0;
                    if pixel[1] > 160 && pixel[1] > pixel[0] * 2 {
                        sum_y += y as u64;
                        count += 1;
                    }
                }
            }
            sum_y as f32 / count as f32
        };
        let early = green_centroid(0, 160);
        let late = green_centroid(160, 320);
        assert!(
            early - late > 10.0,
            "a shared camera preserves the visible upward motion: {early} -> {late}"
        );
    }

    #[test]
    fn a_weighted_mesh_is_filtered_as_one_attachment() {
        let mut doc = rig();
        let root = doc
            .skeleton
            .bones
            .iter()
            .find(|(_, bone)| bone.name == "root")
            .unwrap()
            .0;
        let child = doc
            .skeleton
            .bones
            .iter()
            .find(|(_, bone)| bone.name == "child")
            .unwrap()
            .0;
        let mut slot = Slot::new("weighted".into(), root);
        slot.attachment = Some("weighted".into());
        slot.color = [0.1, 0.4, 1.0, 1.0];
        let slot = doc.skeleton.add_slot(slot);
        let mesh = MeshAttachment {
            texture: "white".into(),
            setup_vertices: vec![
                Vec2::new(-8.0, -8.0),
                Vec2::new(8.0, -8.0),
                Vec2::new(8.0, 8.0),
                Vec2::new(-8.0, 8.0),
            ],
            uvs: vec![
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 0.0),
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            weights: vec![
                vec![
                    VertexWeight {
                        bone: root,
                        weight: 0.5
                    },
                    VertexWeight {
                        bone: child,
                        weight: 0.5
                    }
                ];
                4
            ],
            ..MeshAttachment::default()
        };
        let skin = doc.skeleton.default_skin;
        doc.skeleton.skins[skin].set(slot, "weighted", Attachment::Mesh(mesh));
        let base = render_frame(&doc, &fixed_frame(0.0)).unwrap();
        let mut focused = fixed_frame(0.0);
        focused.focus = Some(Focus {
            bones: vec!["root".into()],
            mode: FocusMode::ArtOnly,
            ..Focus::default()
        });
        let focused = render_frame(&doc, &focused).unwrap();
        let blue_pixels = |bytes: &[u8]| {
            image::load_from_memory(bytes)
                .unwrap()
                .to_rgba8()
                .pixels()
                .filter(|pixel| pixel[2] > 150 && pixel[2] > pixel[0] * 2)
                .count()
        };
        assert_eq!(
            blue_pixels(&base.bytes),
            blue_pixels(&focused.bytes),
            "focus keeps or drops the weighted mesh as a whole"
        );
    }
}
