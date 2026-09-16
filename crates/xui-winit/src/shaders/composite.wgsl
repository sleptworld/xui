@group(0) @binding(0)
var tile_texture: texture_2d<f32>;
@group(0) @binding(1)
var tile_sampler: sampler;

struct TileUniform {
    frame_extent: vec2f,
    tile_origin: vec2f,
    valid_extent: vec2f,
    allocation_extent: vec2f,
}

@group(0) @binding(2)
var<uniform> tile: TileUniform;

struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var corners = array<vec2f, 3>(
        vec2f(0.0, 0.0),
        vec2f(2.0, 0.0),
        vec2f(0.0, 2.0),
    );
    let corner = corners[vertex_index];
    let pixel = tile.tile_origin + corner * tile.valid_extent;
    var output: VertexOutput;
    output.position = vec4f(
        pixel.x / tile.frame_extent.x * 2.0 - 1.0,
        1.0 - pixel.y / tile.frame_extent.y * 2.0,
        0.0,
        1.0,
    );
    output.uv = corner * tile.valid_extent / tile.allocation_extent;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    return textureSample(tile_texture, tile_sampler, input.uv);
}
