// Textured attachments (T-301): world-space quads sampled from an atlas-free
// per-asset texture, tinted by the slot color from `Pose.slot_colors`.

@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

@group(1) @binding(0)
var sprite_texture: texture_2d<f32>;
@group(1) @binding(1)
var sprite_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // Two-color tint (T-505): `dark.a` doubles as the enable flag, so a slot
    // without one costs no branch and no second pipeline.
    @location(3) dark: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) dark: vec4<f32>,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = model.color;
    out.dark = model.dark;
    out.uv = model.uv;
    out.clip_position = view_proj * vec4<f32>(model.position, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(sprite_texture, sprite_sampler, in.uv);
    // Two-color tint: the light color multiplies as usual, the dark color fills
    // what the texture leaves dark. It is what lets one sprite read as both lit
    // and shadowed without a second texture. `dark.a` is the amount.
    let lit = texel.rgb * in.color.rgb;
    let shadowed = (1.0 - texel.rgb) * in.dark.rgb * in.dark.a;
    // Straight-alpha source; the pipeline's blend state does the premultiply.
    return vec4<f32>(lit + shadowed, texel.a * in.color.a);
}
