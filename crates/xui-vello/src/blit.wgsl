// Copies the Vello output texture onto the swapchain image, pixel for pixel.
//
// Vello writes sRGB-encoded values into an `Rgba8Unorm` storage texture, and the
// backend picks a non-sRGB surface format, so the copy is a plain load with no
// colour conversion in either direction.

@group(0) @binding(0) var source_texture: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -3.0),
        vec2f(3.0, 1.0),
        vec2f(-1.0, 1.0),
    );
    return vec4f(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    return textureLoad(source_texture, vec2i(position.xy), 0);
}
