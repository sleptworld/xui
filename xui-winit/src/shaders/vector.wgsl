struct UiUniforms {
    viewport_size: vec4f,
    scale_factor: vec4f,
}

@group(0) @binding(0)
var<uniform> ui: UiUniforms;
@group(1) @binding(0)
var vector_texture: texture_2d<f32>;
@group(1) @binding(1)
var vector_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2f,
    @location(1) uv: vec2f,
}

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let ndc = input.position * ui.scale_factor.xy / ui.viewport_size.xy
        * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);
    var output: VertexOutput;
    output.position = vec4f(ndc, 0.0, 1.0);
    output.uv = input.uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    let color = textureSample(vector_texture, vector_sampler, input.uv);
    let cutoff = color.rgb / vec3f(12.92);
    let power = pow((color.rgb + vec3f(0.055)) / vec3f(1.055), vec3f(2.4));
    let linear = select(power, cutoff, color.rgb <= vec3f(0.04045));
    return vec4f(linear, color.a);
}
