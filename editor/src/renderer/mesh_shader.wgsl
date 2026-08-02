// Camera View-Projection Matrix
@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = model.color;
    out.uv = model.uv;

    let world_position = vec4<f32>(model.position, 0.0, 1.0);
    out.clip_position = view_proj * world_position;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Basic vertex color output (heatmap).
    // In the future we would sample a texture here using in.uv
    return in.color;
}
