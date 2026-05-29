struct Uniforms {
    viewport_size: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> ui_uniforms: Uniforms;
@group(1) @binding(0)
var atlas_texture: texture_3d<f32>;
@group(1) @binding(1)
var f_sampler: sampler;

const gamma_lut: array<f32, 256> = array<f32, 256>(0.000, 0.058, 0.117, 0.175, 0.234, 0.293, 0.353, 0.413, 0.473, 0.534, 0.597, 0.661, 0.727, 0.797, 0.876, 1.000, 0.000, 0.021, 0.082, 0.143, 0.203, 0.264, 0.325, 0.386, 0.448, 0.510, 0.572, 0.635, 0.700, 0.766, 0.836, 1.000, 0.000, 0.000, 0.034, 0.098, 0.161, 0.224, 0.287, 0.350, 0.413, 0.475, 0.538, 0.601, 0.665, 0.729, 0.793, 1.000, 0.000, 0.000, 0.000, 0.033, 0.099, 0.165, 0.231, 0.296, 0.360, 0.425, 0.489, 0.552, 0.616, 0.679, 0.741, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000, 0.000, 0.092, 0.180, 0.264, 0.345, 0.422, 0.496, 0.566, 0.633, 0.696, 0.756, 0.812, 0.864, 0.913, 0.958, 1.000);

var<private> positions: array<vec2f, 4> = array<vec2f, 4>(
    vec2f(0.0, 0.0),
    vec2f(1.0, 0.0),
    vec2f(0.0, 1.0),
    vec2f(1.0, 1.0),
);

struct GlyphInstance {
    @location(0) bound: vec4<f32>,
    @location(1) layer: f32,
    @location(2) padding: vec3<f32>,
    @location(3) uv: vec4<f32>,
    @location(4) color: vec4<f32>,
}

;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec3<f32>,
    @location(1) color: vec4<f32>,
}

;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: GlyphInstance) -> VertexOutput {
    var output: VertexOutput;
    let bound = instance.bound;
    let corner = positions[vertex_index];
    let current_position = corner * bound.zw + bound.xy;
    let ndc = current_position / ui_uniforms.viewport_size.xy * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);

    output.position = vec4f(ndc, 0.0, 1.0);
    output.uv = vec3f(corner * instance.uv.zw + instance.uv.xy, instance.layer);
    output.color = instance.color;
    return output;
}

fn luma(color: vec4f) -> f32 {
    return color.x * 0.25 + color.y * 0.72 + color.z * 0.075;
}

fn gamma_correct(luma: f32, alpha: f32, gamma: f32, contrast: f32) -> f32 {
    let inverse_luma = 1.0 - luma;
    let inverse_alpha = 1.0 - alpha;
    let g = pow(luma * alpha + inverse_luma * inverse_alpha, gamma);
    var a = (g - inverse_luma) / (luma - inverse_luma);
    a = a + ((1.0 - a) * contrast * a);
    return clamp(a, 0.0, 1.0);
}

fn gamma_correct_subpx(color: vec4f, mask: vec4f) -> vec4f {
    let l = luma(color);
    let inverse_luma = 1.0 - l;
    let gamma = mix(1.0 / 1.2, 1.0 / 2.4, inverse_luma);
    let contrast = mix(0.1, 0.8, inverse_luma);
    return vec4f(gamma_correct(l, mask.x * color.a, gamma, contrast), gamma_correct(l, mask.y * color.a, gamma, contrast), gamma_correct(l, mask.z * color.a, gamma, contrast), 1.0);
}

@fragment
fn fs_main_red(input: VertexOutput) -> @location(0) vec4f {
    let mask = textureSample(atlas_texture, f_sampler, input.uv);
    let coverage = gamma_correct_subpx(input.color, mask);
    return vec4f(input.color.rgb, coverage.r);
}

@fragment
fn fs_main_green(input: VertexOutput) -> @location(0) vec4f {
    let mask = textureSample(atlas_texture, f_sampler, input.uv);
    let coverage = gamma_correct_subpx(input.color, mask);
    return vec4f(input.color.rgb, coverage.g);
}

@fragment
fn fs_main_blue(input: VertexOutput) -> @location(0) vec4f {
    let mask = textureSample(atlas_texture, f_sampler, input.uv);
    let coverage = gamma_correct_subpx(input.color, mask);
    return vec4f(input.color.rgb, coverage.b);
}
