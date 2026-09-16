struct Uniforms {
    viewport_size: vec4<f32>,
    scale_factor: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> ui_uniforms: Uniforms;
@group(1) @binding(0)
var image_texture: texture_2d<f32>;
@group(1) @binding(1)
var image_sampler: sampler;

var<private> positions: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
);

struct ImageInstance {
    @location(0) bounds: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) tile: vec4<f32>,
    @location(4) repeat: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) logical_position: vec2<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) opacity: f32,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: ImageInstance) -> VertexOutput {
    let corner = positions[vertex_index];
    let current_position = corner * instance.bounds.zw + instance.bounds.xy;
    let ndc = current_position * ui_uniforms.scale_factor.xy / ui_uniforms.viewport_size.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);

    var output: VertexOutput;
    output.position = vec4<f32>(ndc, 0.0, 1.0);
    var uv = (current_position - instance.tile.xy) / instance.tile.zw;
    if instance.repeat.x > 0.5 {
        uv.x = fract(uv.x);
    }
    if instance.repeat.y > 0.5 {
        uv.y = fract(uv.y);
    }
    if instance.params.y > 0.5 {
        uv.x = 1.0 - uv.x;
    }
    if instance.params.z > 0.5 {
        uv.y = 1.0 - uv.y;
    }
    let rotation = u32(instance.params.w + 0.5) % 4u;
    if rotation == 1u {
        uv = vec2<f32>(uv.y, 1.0 - uv.x);
    } else if rotation == 2u {
        uv = vec2<f32>(1.0 - uv.x, 1.0 - uv.y);
    } else if rotation == 3u {
        uv = vec2<f32>(1.0 - uv.y, uv.x);
    }
    output.uv = uv;
    output.logical_position = current_position;
    output.clip = instance.clip;
    output.opacity = instance.params.x;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    if input.logical_position.x < input.clip.x ||
        input.logical_position.y < input.clip.y ||
        input.logical_position.x > input.clip.x + input.clip.z ||
        input.logical_position.y > input.clip.y + input.clip.w {
        discard;
    }

    let color = textureSample(image_texture, image_sampler, input.uv);
    return vec4<f32>(color.rgb, color.a * input.opacity);
}
