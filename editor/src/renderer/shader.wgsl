// Camera View-Projection Matrix
@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(6) bary: vec3<f32>,
}

// Instance Data: Bone Model Matrix + Color
struct InstanceInput {
    @location(1) model_matrix_0: vec4<f32>,
    @location(2) model_matrix_1: vec4<f32>,
    @location(3) model_matrix_2: vec4<f32>,
    @location(4) model_matrix_3: vec4<f32>,
    @location(5) bone_color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) bary: vec3<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = instance.bone_color;
    out.uv = model.position;
    out.bary = model.bary;

    // Reconstruct the 4x4 model matrix from the instance input columns
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    let world_position = model_matrix * vec4<f32>(model.position, 0.0, 1.0);
    out.clip_position = view_proj * world_position;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Edge detection via barycentric coordinates
    let bary = in.bary;
    let edge_min = min(bary.x, min(bary.y, bary.z));

    // Use a fixed relative width for the outline so it scales perfectly
    // with the bone's screen size (and camera zoom)
    let relative_edge_width = 0.08; // 8% of the triangle width
    // Anti-aliasing transition range (1 pixel wide in barycentric space)
    let aa_width = fwidth(edge_min);
    
    // Smoothstep from the relative edge width down to 0, using aa_width for smoothness
    let edge_factor = smoothstep(relative_edge_width - aa_width, relative_edge_width + aa_width, edge_min);

    // Fill: translucent (alpha ~0.22), Edge: more opaque (alpha ~0.9)
    let fill_alpha = in.color.a * 0.25;
    let edge_alpha = in.color.a * 1.0;
    let final_alpha = mix(edge_alpha, fill_alpha, edge_factor);

    return vec4<f32>(in.color.rgb, final_alpha);
}
