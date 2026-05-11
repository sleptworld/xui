struct UniformCommonTools {
    model_matrix: mat4x4f,
    view_matrix: mat4x4f,
    proj_matrix: mat4x4f,

    camera_position: vec3f,
    camera_front: vec3f,
    camera_up: vec3f,
    ca: mat3x3f,

    projection_para: vec3f,
    half_tan_fov: f32,
    resolution: vec2f,

    time: f32,

}

;

@group(0) @binding(0)
var<uniform> common_tools: UniformCommonTools;

struct FullScreenQuadVertex {
    @invariant @builtin(position) position: vec4f,
    @location(0) texcoord: vec2f,
    @location(1) pixel_position: vec2f
}

var<private> positions: array<vec2f, 4> = array<vec2f, 4>(vec2f(- 1.0, - 1.0), vec2f(1.0, - 1.0), vec2f(- 1.0, 1.0), vec2f(1.0, 1.0));

fn sdf_rect(p: vec2<f32>, half_size: vec2<f32>) -> f32 {
    let d = abs(p) - half_size;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

fn sdf_rounded_rect(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sdf_circle(p: vec2<f32>, radius: f32) -> f32 {
    return length(p) - radius;
}

fn sdf_fill_alpha(distance: f32) -> f32 {
    let width = max(fwidth(distance), 0.0001);
    return 1.0 - smoothstep(- width, width, distance);
}

fn sdf_stroke_alpha(distance: f32, stroke_width: f32) -> f32 {
    let half_width = stroke_width * 0.5;
    let width = max(fwidth(distance), 0.0001);
    return 1.0 - smoothstep(half_width - width, half_width + width, abs(distance));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> FullScreenQuadVertex {
    var output: FullScreenQuadVertex;

    let ndc = positions[vertex_index];
    output.position = vec4f(ndc, 0.0, 1.0);
    output.texcoord = ndc * 0.5 + 0.5;
    output.pixel_position = (ndc + 1.0) * 0.5 * common_tools.resolution;
    return output;
}

@fragment
fn fs_main(input: FullScreenQuadVertex) -> FragmentOutput {

    var output: FragmentOutput;
    return output;
}