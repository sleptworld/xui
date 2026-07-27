struct UiUniforms {
    viewport_size: vec4f,
    scale_factor: vec4f,
}

@group(0) @binding(0)
var<uniform> ui: UiUniforms;
@group(1) @binding(0)
var tile_texture: texture_2d<f32>;
@group(1) @binding(1)
var tile_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) uv: vec2f,
    @location(2) opacity: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) opacity: f32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let ndc = input.position * ui.scale_factor.xy / ui.viewport_size.xy
        * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
    var output: VertexOutput;
    output.position = vec4f(ndc, 0.0, 1.0);
    output.uv = input.uv;
    output.opacity = input.opacity;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    let color = textureSample(tile_texture, tile_sampler, input.uv);
    return vec4f(color.rgb, color.a * input.opacity);
}
