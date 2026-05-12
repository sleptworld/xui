struct UiUniforms {
    viewport_size: vec4f,
}

@group(0) @binding(0)
var<uniform> ui: UiUniforms;

struct QuadVertex {
    @invariant @builtin(position) position: vec4f,
    @location(0) pixel_position: vec2f,
    @location(1) shape: vec4f,
    @location(2) clip: vec4f,
    @location(3) fill_color: vec4f,
    @location(4) stroke_color: vec4f,
    @location(5) params: vec4f,
    @location(6) stroke_params: vec4f,
    @location(7) projection_color: vec4f,
    @location(8) projection_params: vec4f,
    @location(9) extra: vec4f,
}

struct FragmentOutput {
    @location(0) color: vec4f,
}

struct UiInstance {
    @location(0) bounds: vec4f,
    @location(1) shape: vec4f,
    @location(2) clip: vec4f,
    @location(3) fill_color: vec4f,
    @location(4) stroke_color: vec4f,
    @location(5) params: vec4f,
    @location(6) stroke_params: vec4f,
    @location(7) projection_color: vec4f,
    @location(8) projection_params: vec4f,
    @location(9) extra: vec4f,
}

var<private> positions: array<vec2f, 6> = array<vec2f, 6>(
    vec2f(0.0, 0.0),
    vec2f(1.0, 0.0),
    vec2f(0.0, 1.0),
    vec2f(0.0, 1.0),
    vec2f(1.0, 0.0),
    vec2f(1.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: UiInstance) -> QuadVertex {
    let corner = positions[vertex_index];
    let pixel_position = instance.bounds.xy + corner * instance.bounds.zw;
    let ndc = pixel_position / ui.viewport_size.xy * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);

    var output: QuadVertex;
    output.position = vec4f(ndc, 0.0, 1.0);
    output.pixel_position = pixel_position;
    output.shape = instance.shape;
    output.clip = instance.clip;
    output.fill_color = instance.fill_color;
    output.stroke_color = instance.stroke_color;
    output.params = instance.params;
    output.stroke_params = instance.stroke_params;
    output.projection_color = instance.projection_color;
    output.projection_params = instance.projection_params;
    output.extra = instance.extra;
    return output;
}

fn sdf_rect(p: vec2f, half_size: vec2f) -> f32 {
    let d = abs(p) - half_size;
    return length(max(d, vec2f(0.0))) + min(max(d.x, d.y), 0.0);
}

fn sdf_rounded_rect(p: vec2f, half_size: vec2f, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2f(r);
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sdf_line_segment(p: vec2f, a: vec2f, b: vec2f, width: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let denom = max(dot(ba, ba), 0.0001);
    let h = clamp(dot(pa, ba) / denom, 0.0, 1.0);
    return length(pa - ba * h) - width * 0.5;
}

fn sdf_fill_alpha(distance: f32) -> f32 {
    let width = max(fwidth(distance), 0.0001);
    return 1.0 - smoothstep(-width, width, distance);
}

fn sdf_stroke_alpha(distance: f32, stroke_width: f32, stroke_direction: f32) -> f32 {
    let half_width = stroke_width * 0.5;
    let stroke_center = clamp(stroke_direction, -1.0, 1.0) * half_width;
    let width = max(fwidth(distance), 0.0001);
    return 1.0 - smoothstep(half_width - width, half_width + width, abs(distance - stroke_center));
}

fn sdf_projection_alpha(distance: f32, blur: f32) -> f32 {
    let softness = max(blur, max(fwidth(distance), 0.0001));
    return 1.0 - smoothstep(-softness, softness, distance);
}

fn premultiply(color: vec4f) -> vec4f {
    return vec4f(color.rgb * color.a, color.a);
}

fn unpremultiply(color: vec4f) -> vec4f {
    if (color.a <= 0.0) {
        return vec4f(0.0);
    }
    return vec4f(color.rgb / color.a, color.a);
}

fn blend_over(src: vec4f, dst: vec4f) -> vec4f {
    let s = premultiply(src);
    let d = premultiply(dst);
    return unpremultiply(vec4f(s.rgb + d.rgb * (1.0 - s.a), s.a + d.a * (1.0 - s.a)));
}

fn is_outside_clip(p: vec2f, clip: vec4f) -> bool {
    return p.x < clip.x || p.y < clip.y || p.x >= clip.x + clip.z || p.y >= clip.y + clip.w;
}

@fragment
fn fs_main(input: QuadVertex) -> FragmentOutput {
    var output: FragmentOutput;

    if (is_outside_clip(input.pixel_position, input.clip)) {
        output.color = vec4f(0.0);
        return output;
    }

    let kind = input.params.x;
    let radius = input.params.y;
    let stroke_width = input.stroke_params.x;
    let stroke_direction = input.stroke_params.y;
    let projection_offset = input.projection_params.xy;
    let projection_blur = input.projection_params.z;
    let projection_spread = input.projection_params.w;
    let shape_center = input.shape.xy + input.shape.zw * 0.5;
    let shape_half_size = input.shape.zw * 0.5;

    var distance = 0.0;
    if (kind < 0.5) {
        distance = sdf_rect(input.pixel_position - shape_center, shape_half_size);
    } else if (kind < 1.5) {
        distance = sdf_rounded_rect(input.pixel_position - shape_center, shape_half_size, radius);
    } else {
        distance = sdf_line_segment(input.pixel_position, input.extra.xy, input.extra.zw, stroke_width);
    }

    var projection_alpha = 0.0;
    if (input.projection_color.a > 0.0 && kind < 1.5) {
        let projection_center = shape_center + projection_offset;
        let projection_half_size = max(shape_half_size + vec2f(projection_spread), vec2f(0.0));
        let projection_radius = max(radius + projection_spread, 0.0);
        var projection_distance = 0.0;

        if (kind < 0.5) {
            projection_distance = sdf_rect(input.pixel_position - projection_center, projection_half_size);
        } else {
            projection_distance = sdf_rounded_rect(input.pixel_position - projection_center, projection_half_size, projection_radius);
        }

        projection_alpha = input.projection_color.a * sdf_projection_alpha(projection_distance, projection_blur);
    }

    let fill_alpha = input.fill_color.a * sdf_fill_alpha(distance);
    let stroke_alpha = input.stroke_color.a * sdf_stroke_alpha(distance, stroke_width, stroke_direction);

    let projection = vec4f(input.projection_color.rgb, projection_alpha);
    let fill = vec4f(input.fill_color.rgb, fill_alpha);
    let stroke = vec4f(input.stroke_color.rgb, stroke_alpha);

    output.color = blend_over(stroke, blend_over(fill, projection));
    return output;
}
