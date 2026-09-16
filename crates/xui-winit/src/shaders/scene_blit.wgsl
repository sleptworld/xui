struct SceneBlitUniform {
    target_info: vec4f,
    source_info: vec4f,
    allocation: vec4f,
    map0: vec4f,
    map1: vec4f,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: SceneBlitUniform;

struct VertexOutput { @builtin(position) position: vec4f }

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -3.0),
        vec2f(3.0, 1.0),
        vec2f(-1.0, 1.0),
    );
    var output: VertexOutput;
    output.position = vec4f(positions[index], 0.0, 1.0);
    return output;
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let target_extent = max(uniforms.target_info.zw, vec2f(0.00001));
    let target_logical = uniforms.target_info.xy
        + position.xy / max(uniforms.allocation.zw, vec2f(1.0)) * target_extent;
    let source_logical = vec2f(
        uniforms.map0.x * target_logical.x + uniforms.map0.z * target_logical.y + uniforms.map1.x,
        uniforms.map0.y * target_logical.x + uniforms.map0.w * target_logical.y + uniforms.map1.y,
    );
    let logical_to_pixel = uniforms.allocation.zw / target_extent;
    let source_extent = uniforms.source_info.zw * logical_to_pixel;
    let pixel = (source_logical - uniforms.source_info.xy) * logical_to_pixel;
    if any(pixel < vec2f(0.0)) || any(pixel >= source_extent) {
        return vec4f(0.0);
    }
    return textureSampleLevel(
        source_texture,
        source_sampler,
        pixel / max(uniforms.allocation.xy, vec2f(1.0)),
        0.0,
    );
}
