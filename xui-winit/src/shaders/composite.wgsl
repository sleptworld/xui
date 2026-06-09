@group(0) @binding(0)
var scene_texture: texture_2d<f32>;
@group(0) @binding(1)
var scene_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -3.0),
        vec2f(3.0, 1.0),
        vec2f(-1.0, 1.0),
    );

    let position = positions[vertex_index];

    var output: VertexOutput;
    output.position = vec4f(position, 0.0, 1.0);
    // output.uv = position * 0.5 + vec2f(0.5);

    output.uv = vec2f(
        position.x * 0.5 + 0.5,
        0.5 - position.y * 0.5
    );
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    return textureSample(scene_texture, scene_sampler, input.uv);
}
