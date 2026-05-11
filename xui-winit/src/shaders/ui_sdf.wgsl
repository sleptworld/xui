struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );

    let pos = positions[vertex_index];
    var out: VertexOut;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + vec2<f32>(0.5);
    return out;
}

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
    return 1.0 - smoothstep(-width, width, distance);
}

fn sdf_stroke_alpha(distance: f32, stroke_width: f32) -> f32 {
    let half_width = stroke_width * 0.5;
    let width = max(fwidth(distance), 0.0001);
    return 1.0 - smoothstep(half_width - width, half_width + width, abs(distance));
}

fn premultiply(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(color.rgb * color.a, color.a);
}

fn blend_over(src: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    let src_pm = premultiply(src);
    let dst_pm = premultiply(dst);
    let out_a = src_pm.a + dst_pm.a * (1.0 - src_pm.a);
    let out_rgb = src_pm.rgb + dst_pm.rgb * (1.0 - src_pm.a);
    if (out_a <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(out_rgb / out_a, out_a);
}

fn draw_demo_ui(uv: vec2<f32>) -> vec4<f32> {
    let p = (uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(640.0, 360.0);

    let panel_distance = sdf_rounded_rect(
        p - vec2<f32>(0.0, 0.0),
        vec2<f32>(190.0, 88.0),
        18.0,
    );
    let panel = vec4<f32>(0.94, 0.96, 1.0, sdf_fill_alpha(panel_distance));

    let border = vec4<f32>(0.18, 0.42, 0.88, sdf_stroke_alpha(panel_distance, 2.0) * 0.9);

    let dot_distance = sdf_circle(p - vec2<f32>(-145.0, -45.0), 14.0);
    let dot = vec4<f32>(0.18, 0.42, 0.88, sdf_fill_alpha(dot_distance));

    return blend_over(dot, blend_over(border, panel));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return draw_demo_ui(in.uv);
}
