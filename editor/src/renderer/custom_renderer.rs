use ankhimate_core::slot::BlendMode;
use eframe::egui_wgpu;

/// Vertex for Meshes
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Two-color tint (T-505); `[0,0,0,0]` for slots without one.
    pub dark: [f32; 4],
}

/// Vertex and index budget for textured draws (regions and meshes together).
///
/// Sized for a few thousand quads or a handful of dense meshes. Draws past the
/// budget are dropped whole rather than clipped mid-triangle — worth surfacing
/// as a diagnostic if a real rig ever reaches it (T-702).
pub const SPRITE_VERTEX_CAPACITY: usize = 64 * 1024;
pub const SPRITE_INDEX_CAPACITY: usize = 192 * 1024;

pub struct CustomRenderer {
    pub camera_buffer: wgpu::Buffer,
    pub camera_bind_group: wgpu::BindGroup,

    // Mesh Pipeline
    pub mesh_pipeline: wgpu::RenderPipeline,
    pub mesh_vertex_buffer: wgpu::Buffer,
    pub mesh_index_buffer: wgpu::Buffer,

    // Sprite (region attachment) pipeline — T-301
    /// One pipeline per blend mode, indexed by `blend_index` (T-505).
    pub sprite_pipelines: [wgpu::RenderPipeline; 4],
    pub sprite_vertex_buffer: wgpu::Buffer,
    pub sprite_index_buffer: wgpu::Buffer,
    pub sprite_texture_layout: wgpu::BindGroupLayout,
    pub sprite_sampler: wgpu::Sampler,
    /// Colour space to upload sprite pixels in — see [`sprite_texture_format`].
    pub sprite_texture_format: wgpu::TextureFormat,
    /// GPU textures keyed by asset id. Uploaded once and reused across frames;
    /// re-uploading a 2K sprite every frame is the difference between a rig that
    /// scrubs smoothly and one that stutters.
    pub textures: std::collections::HashMap<u64, wgpu::BindGroup>,
}

impl CustomRenderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        // 1. Camera Uniform Buffer
        let camera_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Uniform Buffer"),
            size: 64, // 4x4 f32 matrix
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: Some("Camera Bind Group Layout"),
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("Camera Bind Group"),
        });

        // The bone gizmo used to be an instanced quad through its own pipeline
        // and shader. It is drawn in egui now, with the same function that
        // previews a bone while you drag it out — two drawings of one thing is
        // one too many, and they had drifted into different shapes.

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mesh Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh_shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Canvas Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mesh Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, // position
                        1 => Float32x2, // uv
                        2 => Float32x4  // color
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Pre-allocate large dynamic buffers for meshes
        let mesh_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Vertex Buffer"),
            size: std::mem::size_of::<MeshVertex>() as u64 * 1024 * 10, // 10k vertices
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mesh_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Index Buffer"),
            size: std::mem::size_of::<u32>() as u64 * 1024 * 30, // 30k indices
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ====================================================
        // Sprite Pipeline (textured region attachments, T-301)
        // ====================================================
        let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sprite Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite_shader.wgsl").into()),
        });

        let sprite_texture_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Sprite Texture Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let sprite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Sprite Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let sprite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Sprite Pipeline Layout"),
                bind_group_layouts: &[Some(&bind_group_layout), Some(&sprite_texture_layout)],
                immediate_size: 0,
            });

        // One pipeline per blend mode. Blend state is baked into a pipeline in
        // wgpu, so the alternative is a pipeline switch per draw either way —
        // building all four up front just moves the cost off the frame.
        let sprite_pipelines = BLEND_MODES.map(|mode| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Sprite Render Pipeline"),
                layout: Some(&sprite_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &sprite_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x2, // position
                            1 => Float32x2, // uv
                            2 => Float32x4, // tint
                            3 => Float32x4  // dark tint
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend_state(mode)),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        });

        let sprite_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sprite Vertex Buffer"),
            size: (std::mem::size_of::<MeshVertex>() * SPRITE_VERTEX_CAPACITY) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Indices are uploaded per frame now that a draw can be an arbitrary
        // mesh, not just a quad with a fixed winding.
        let sprite_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sprite Index Buffer"),
            size: (std::mem::size_of::<u32>() * SPRITE_INDEX_CAPACITY) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            camera_buffer,
            camera_bind_group,
            mesh_pipeline,
            mesh_vertex_buffer,
            mesh_index_buffer,
            sprite_pipelines,
            sprite_vertex_buffer,
            sprite_index_buffer,
            sprite_texture_layout,
            sprite_sampler,
            sprite_texture_format: sprite_texture_format(format),
            textures: std::collections::HashMap::new(),
        }
    }

    /// Upload one decoded RGBA8 image and cache its bind group under `key`.
    pub fn upload_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        upload: &SpriteUpload,
    ) {
        let size = wgpu::Extent3d {
            width: upload.width.max(1),
            height: upload.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Sprite Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.sprite_texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width),
                rows_per_image: Some(size.height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sprite Texture Bind Group"),
            layout: &self.sprite_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sprite_sampler),
                },
            ],
        });
        self.textures.insert(upload.key, bind_group);
    }
}

/// The colour space to store sprite pixels in, given the render target's format.
///
/// Getting this wrong is why imported art looked darker than the source file. A
/// PNG's bytes are sRGB-encoded. Declaring the texture `…UnormSrgb` makes the
/// sampler decode them to linear — correct only if something re-encodes on the
/// way out, which happens when the target is itself sRGB. egui's target is
/// usually plain `Bgra8Unorm` (it does its own gamma work), so the linear values
/// would be written out as if they were already sRGB: every mid-tone lands too
/// dark. Matching the texture to the target keeps the pixels untouched from file
/// to framebuffer.
fn sprite_texture_format(target: wgpu::TextureFormat) -> wgpu::TextureFormat {
    if target.is_srgb() {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

/// A decoded image waiting for the GPU. Produced by the canvas layer only for
/// assets the renderer does not already hold.
pub struct SpriteUpload {
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// One textured shape, already in world space, in paint order.
///
/// Holds an index list rather than assuming a quad: a region attachment is two
/// triangles, a mesh is however many it has, and both take the same path to the
/// screen (T-401).
pub struct SpriteDraw {
    pub key: u64,
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    /// How this slot composites (T-505).
    pub blend: BlendMode,
}

impl SpriteDraw {
    /// The common case: a quad, wound as two triangles.
    pub fn quad(key: u64, vertices: [MeshVertex; 4], blend: BlendMode) -> Self {
        Self {
            key,
            vertices: vertices.to_vec(),
            indices: vec![0, 1, 2, 0, 2, 3],
            blend,
        }
    }
}

/// The blend state each slot blend mode composites with (T-505).
///
/// Sources are straight-alpha, so every mode multiplies the source by its alpha
/// rather than assuming premultiplied input.
fn blend_state(mode: BlendMode) -> wgpu::BlendState {
    use wgpu::{BlendComponent, BlendFactor, BlendOperation};
    match mode {
        BlendMode::Normal => wgpu::BlendState::ALPHA_BLENDING,
        // Additive: light emitted, not surface shown — flashes, sparks, glows.
        // Alpha still gates it, or a transparent texel would brighten the frame.
        BlendMode::Additive => wgpu::BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        },
        // Multiply darkens by what is already there — shadows, stains.
        BlendMode::Multiply => wgpu::BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::Dst,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::DstAlpha,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
        },
        // Screen is the inverse of multiply: it lightens without blowing out.
        BlendMode::Screen => wgpu::BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrc,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrc,
                operation: BlendOperation::Add,
            },
        },
    }
}

/// Index of a blend mode's pipeline in [`CustomRenderer::sprite_pipelines`].
fn blend_index(mode: BlendMode) -> usize {
    match mode {
        BlendMode::Normal => 0,
        BlendMode::Additive => 1,
        BlendMode::Multiply => 2,
        BlendMode::Screen => 3,
    }
}

/// Every blend mode, in `blend_index` order.
const BLEND_MODES: [BlendMode; 4] = [
    BlendMode::Normal,
    BlendMode::Additive,
    BlendMode::Multiply,
    BlendMode::Screen,
];

pub struct MeshDrawCall {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
}

pub struct CustomCallback {
    pub view_proj: glam::Mat4,
    pub mesh_draws: Vec<MeshDrawCall>,
    /// Textured attachments, back-to-front (T-301).
    pub sprite_draws: Vec<SpriteDraw>,
    /// Images the canvas found were not in the texture cache yet.
    pub sprite_uploads: Vec<SpriteUpload>,
    /// GPU-resident content hashes, acknowledged only after upload succeeds.
    pub uploaded_textures: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<u64>>>,
}

impl egui_wgpu::CallbackTrait for CustomCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Texture uploads first: `paint` looks bind groups up by key and skips
        // anything missing, so a sprite must be resident before its draw runs.
        if !self.sprite_uploads.is_empty()
            && let Some(renderer) = resources.get_mut::<CustomRenderer>()
        {
            for upload in &self.sprite_uploads {
                renderer.upload_texture(device, queue, upload);
                self.uploaded_textures
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(upload.key);
            }
        }

        let renderer = resources.get::<CustomRenderer>().unwrap();

        // Sprites and meshes share one buffer pair; each draw's indices are
        // rebased as they are concatenated, and `paint` walks the same ranges.
        if !self.sprite_draws.is_empty() {
            let mut verts: Vec<MeshVertex> = Vec::new();
            let mut indices: Vec<u32> = Vec::new();
            for draw in &self.sprite_draws {
                let base = verts.len() as u32;
                verts.extend_from_slice(&draw.vertices);
                indices.extend(draw.indices.iter().map(|i| i + base));
            }
            verts.truncate(SPRITE_VERTEX_CAPACITY);
            indices.truncate(SPRITE_INDEX_CAPACITY);
            queue.write_buffer(
                &renderer.sprite_vertex_buffer,
                0,
                bytemuck::cast_slice(&verts),
            );
            queue.write_buffer(
                &renderer.sprite_index_buffer,
                0,
                bytemuck::cast_slice(&indices),
            );
        }

        // Write the MVP matrix to the Camera Uniform
        queue.write_buffer(
            &renderer.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.view_proj.to_cols_array()]),
        );

        // Write the Mesh data (concat all draws)
        let mut all_vertices = Vec::new();
        let mut all_indices = Vec::new();
        let mut index_offset = 0;

        for draw in &self.mesh_draws {
            all_vertices.extend_from_slice(&draw.vertices);
            for &idx in &draw.indices {
                all_indices.push(idx + index_offset);
            }
            index_offset += draw.vertices.len() as u32;
        }

        if !all_vertices.is_empty() {
            queue.write_buffer(
                &renderer.mesh_vertex_buffer,
                0,
                bytemuck::cast_slice(&all_vertices),
            );
            queue.write_buffer(
                &renderer.mesh_index_buffer,
                0,
                bytemuck::cast_slice(&all_indices),
            );
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &eframe::egui_wgpu::CallbackResources,
    ) {
        let renderer = resources.get::<CustomRenderer>().unwrap();

        let px_per_point = info.pixels_per_point;

        // Fix for High-DPI and UI Panel offsets:
        // We MUST map the wgpu NDC strictly to our canvas UI rect.
        //
        // Clamped to the target. A viewport larger than the surface is a
        // validation error that aborts the process, and egui can hand us one:
        // on the first frames the pane's rect is whatever the layout guessed
        // before the window reported its size. Clamping turns a frame that would
        // have killed the editor into a frame drawn slightly wrong, which is the
        // right trade for something that resolves itself on the next pass.
        let [max_w, max_h] = info.screen_size_px;
        let x = (info.viewport.min.x * px_per_point).clamp(0.0, max_w as f32);
        let y = (info.viewport.min.y * px_per_point).clamp(0.0, max_h as f32);
        let w = (info.viewport.width() * px_per_point).clamp(0.0, max_w as f32 - x);
        let h = (info.viewport.height() * px_per_point).clamp(0.0, max_h as f32 - y);
        if w <= 0.0 || h <= 0.0 {
            // Nothing to draw into — a collapsed or off-screen pane.
            return;
        }
        render_pass.set_viewport(x, y, w, h, 0.0, 1.0);

        // Sprites first: they are the artwork, everything else is an overlay on
        // top of it. Within the batch, submission order *is* draw order, so the
        // canvas hands them over already sorted by `Pose.draw_order`.
        if !self.sprite_draws.is_empty() {
            render_pass.set_bind_group(0, &renderer.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, renderer.sprite_vertex_buffer.slice(..));
            render_pass.set_index_buffer(
                renderer.sprite_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            // Draw order is submission order and must not be reordered to batch
            // by pipeline: an additive flash drawn out of order composites
            // against the wrong background. So the pipeline is switched only
            // when the mode actually changes, which for a normal rig is once.
            let mut current: Option<usize> = None;
            let mut start = 0u32;
            for draw in &self.sprite_draws {
                let count = draw.indices.len() as u32;
                if start + count > SPRITE_INDEX_CAPACITY as u32 {
                    break;
                }
                // A texture that failed to decode or arrived this frame after the
                // upload budget simply does not draw — never a panic.
                if let Some(bind_group) = renderer.textures.get(&draw.key) {
                    let index = blend_index(draw.blend);
                    if current != Some(index) {
                        render_pass.set_pipeline(&renderer.sprite_pipelines[index]);
                        current = Some(index);
                    }
                    render_pass.set_bind_group(1, bind_group, &[]);
                    render_pass.draw_indexed(start..start + count, 0, 0..1);
                }
                start += count;
            }
        }

        // Then meshes (so bones overlay on top)
        if !self.mesh_draws.is_empty() {
            render_pass.set_pipeline(&renderer.mesh_pipeline);
            render_pass.set_bind_group(0, &renderer.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, renderer.mesh_vertex_buffer.slice(..));

            // Calculate total indices
            let total_indices: u32 = self.mesh_draws.iter().map(|d| d.indices.len() as u32).sum();

            render_pass.set_index_buffer(
                renderer.mesh_index_buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(0..total_indices, 0, 0..1);
        }
    }
}
