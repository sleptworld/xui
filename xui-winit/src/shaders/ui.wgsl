struct UiUniforms {
    viewport_size: vec4f,
    scale_factor: vec4f,
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
    let ndc = pixel_position * ui.scale_factor.xy / ui.viewport_size.xy * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0);

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

fn erf_approx(x: vec2f) -> vec2f {
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.278393 * a + 0.230389 * a * a + 0.000972 * a * a * a + 0.078108 * a * a * a * a);
    return s * (1.0 - t * t * t * t);
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    let sigma2 = sigma * sigma;
    return exp(-(x * x) / (2.0 * sigma2)) / sqrt(6.28318530718 * sigma2);
}

fn box_shadow(lower: vec2f, upper: vec2f, point: vec2f, sigma: f32) -> f32 {
    let denom = 1.41421356237 * sigma;
    let integral_lower = 0.5 + 0.5 * erf_approx((point - lower) / denom);
    let integral_upper = 0.5 + 0.5 * erf_approx((point - upper) / denom);
    let integral = integral_lower - integral_upper;
    return integral.x * integral.y;
}

fn rounded_box_shadow_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2f) -> f32 {
    let delta = min(half_size.y - corner - abs(y), 0.0);
    let curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    let integral = 0.5 + 0.5 * erf_approx((x + vec2f(-curved, curved)) / (1.41421356237 * sigma));
    return integral.y - integral.x;
}

fn rounded_box_shadow(lower: vec2f, upper: vec2f, point: vec2f, sigma: f32, corner: f32) -> f32 {
    let center = (lower + upper) * 0.5;
    let half_size = (upper - lower) * 0.5;
    let local_point = point - center;
    let low = -half_size.y;
    let high = half_size.y;
    let start = clamp(local_point.y - 3.0 * sigma, low, high);
    let end = clamp(local_point.y + 3.0 * sigma, low, high);
    let step_size = (end - start) * 0.25;

    var value = 0.0;
    for (var i = 0; i < 4; i = i + 1) {
        let y = start + (f32(i) + 0.5) * step_size;
        value += rounded_box_shadow_x(local_point.x, y, sigma, corner, half_size) * gaussian(local_point.y - y, sigma) * step_size;
    }
    return value;
}

fn color_style_color(kind: f32, start_color: vec4f, end_color: vec4f, geometry: vec4f, p: vec2f) -> vec4f {
    if (kind < 0.5) {
        return start_color;
    }

    if (kind < 1.5) {
        let start = geometry.xy;
        let end = geometry.zw;
        let axis = end - start;
        let denom = max(dot(axis, axis), 0.0001);
        let t = clamp(dot(p - start, axis) / denom, 0.0, 1.0);
        return mix(start_color, end_color, t);
    }

    let radius = max(geometry.z, 0.0001);
    let t = clamp(length(p - geometry.xy) / radius, 0.0, 1.0);
    return mix(start_color, end_color, t);
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
    let color_style_kind = input.params.z;
    let projection_enabled = input.params.w;
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
    if (projection_enabled > 0.5 && input.projection_color.a > 0.0 && kind < 1.5) {
        let projection_center = shape_center + projection_offset;
        let projection_half_size = max(shape_half_size + vec2f(projection_spread), vec2f(0.0));
        let projection_radius = min(max(radius + projection_spread, 0.0), min(projection_half_size.x, projection_half_size.y));
        var projection_distance = 0.0;

        if (kind < 0.5) {
            projection_distance = sdf_rect(input.pixel_position - projection_center, projection_half_size);
        } else {
            projection_distance = sdf_rounded_rect(input.pixel_position - projection_center, projection_half_size, projection_radius);
        }

        if (projection_blur <= 0.0) {
            projection_alpha = input.projection_color.a * sdf_fill_alpha(projection_distance);
        } else {
            let projection_lower = projection_center - projection_half_size;
            let projection_upper = projection_center + projection_half_size;
            var projection_coverage = 0.0;
            if (kind < 0.5) {
                projection_coverage = box_shadow(projection_lower, projection_upper, input.pixel_position, projection_blur);
            } else {
                projection_coverage = rounded_box_shadow(projection_lower, projection_upper, input.pixel_position, projection_blur, projection_radius);
            }
            projection_alpha = input.projection_color.a * clamp(projection_coverage, 0.0, 1.0);
        }
    }

    let styled_color = color_style_color(
        color_style_kind,
        input.fill_color + input.stroke_color,
        input.projection_color,
        input.extra,
        input.pixel_position,
    );
    let gradient_fill_active = color_style_kind > 0.5 && input.stroke_color.a <= 0.0 && stroke_width <= 0.0 && projection_enabled <= 0.5;
    let gradient_stroke_active = color_style_kind > 0.5 && stroke_width > 0.0;
    let fill_alpha = select(0.0, styled_color.a, input.fill_color.a > 0.0 || gradient_fill_active) * sdf_fill_alpha(distance);
    let stroke_alpha = select(0.0, styled_color.a, input.stroke_color.a > 0.0 || gradient_stroke_active) * sdf_stroke_alpha(distance, stroke_width, stroke_direction);

    let projection = vec4f(input.projection_color.rgb, projection_alpha);
    let fill = vec4f(styled_color.rgb, fill_alpha);
    let stroke = vec4f(styled_color.rgb, stroke_alpha);

    output.color = blend_over(stroke, blend_over(fill, projection));
    return output;
}
