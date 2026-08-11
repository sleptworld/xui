struct PassUniform {
    op: vec4u,
    output: vec4f,
    input0: vec4f,
    input1: vec4f,
    input2: vec4f,
    params0: vec4f,
    params1: vec4f,
    color: vec4f,
    inverse0: vec4f,
    inverse1: vec4f,
    matrix: array<vec4f, 5>,
}

@group(0) @binding(0) var foreground_texture: texture_2d<f32>;
@group(0) @binding(1) var backdrop_texture: texture_2d<f32>;
@group(0) @binding(2) var mask_texture: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var<uniform> uniforms: PassUniform;

struct VertexOutput { @builtin(position) position: vec4f }
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));
    var output: VertexOutput;
    output.position = vec4f(positions[index], 0.0, 1.0);
    return output;
}

fn sample_at(texture: texture_2d<f32>, info: vec4f, logical: vec2f) -> vec4f {
    let pixel = (logical - info.xy) * uniforms.output.z;
    if any(pixel < vec2f(0.0)) || any(pixel >= info.zw) { return vec4f(0.0); }
    let allocation_extent = vec2f(textureDimensions(texture));
    return textureSampleLevel(texture, linear_sampler, pixel / allocation_extent, 0.0);
}

fn lum(c: vec3f) -> f32 { return dot(c, vec3f(0.3, 0.59, 0.11)); }
fn sat(c: vec3f) -> f32 { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
fn clip_color(c0: vec3f) -> vec3f {
    let l = lum(c0); let n = min(c0.r, min(c0.g, c0.b)); let x = max(c0.r, max(c0.g, c0.b));
    var c = c0;
    if n < 0.0 { c = vec3f(l) + (c - vec3f(l)) * l / (l - n); }
    if x > 1.0 { c = vec3f(l) + (c - vec3f(l)) * (1.0 - l) / (x - l); }
    return c;
}
fn set_lum(c: vec3f, l: f32) -> vec3f { return clip_color(c + vec3f(l - lum(c))); }
fn set_sat(c: vec3f, s: f32) -> vec3f {
    let lo = min(c.r, min(c.g, c.b)); let hi = max(c.r, max(c.g, c.b));
    if hi <= lo { return vec3f(0.0); }
    return (c - vec3f(lo)) * s / (hi - lo);
}
fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
    let d = select(sqrt(b), ((16.0 * b - 12.0) * b + 4.0) * b, b <= 0.25);
    return b + (2.0 * s - 1.0) * (d - b);
}
fn blend(mode: u32, b: vec3f, s: vec3f) -> vec3f {
    if mode == 0u { return s; }
    if mode == 1u { return b * s; }
    if mode == 2u { return b + s - b * s; }
    if mode == 3u { return select(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), b > vec3f(0.5)); }
    if mode == 4u { return min(b, s); }
    if mode == 5u { return max(b, s); }
    if mode == 6u { return select(min(vec3f(1.0), b / max(vec3f(0.00001), 1.0 - s)), vec3f(1.0), s >= vec3f(1.0)); }
    if mode == 7u { return select(1.0 - min(vec3f(1.0), (1.0 - b) / max(s, vec3f(0.00001))), vec3f(0.0), s <= vec3f(0.0)); }
    if mode == 8u { return select(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), s > vec3f(0.5)); }
    if mode == 9u { return vec3f(soft_light(b.r,s.r), soft_light(b.g,s.g), soft_light(b.b,s.b)); }
    if mode == 10u { return abs(b - s); }
    if mode == 11u { return b + s - 2.0 * b * s; }
    if mode == 12u { return set_lum(set_sat(s, sat(b)), lum(b)); }
    if mode == 13u { return set_lum(set_sat(b, sat(s)), lum(b)); }
    if mode == 14u { return set_lum(b, lum(s)); }
    return set_lum(s, lum(b));
}

fn inverse_point(p: vec2f) -> vec2f {
    return vec2f(uniforms.inverse0.x*p.x + uniforms.inverse0.z*p.y + uniforms.inverse1.x,
                 uniforms.inverse0.y*p.x + uniforms.inverse0.w*p.y + uniforms.inverse1.y);
}

// Shape masks are evaluated in their canonical local space. Derivative-based
// coverage keeps their edges stable under scale, rotation, and skew without an
// extra mask texture or a supersampling pass.
fn shape_coverage(signed_distance: f32) -> f32 {
    let antialias_width = max(fwidth(signed_distance), 0.00001);
    return 1.0 - smoothstep(-antialias_width, antialias_width, signed_distance);
}

fn mask_alpha(logical: vec2f) -> f32 {
    let kind = uniforms.op.w;
    if kind == 0u { return 1.0; }
    let p = inverse_point(logical);
    if kind == 1u {
        let q = abs(p - vec2f(0.5)) - vec2f(0.5);
        let distance = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0);
        return shape_coverage(distance);
    }
    if kind == 2u {
        let radius = clamp(uniforms.params1.x, 0.0, 0.5);
        let q = abs(p - vec2f(0.5)) - vec2f(0.5 - radius);
        let distance = length(max(q, vec2f(0.0))) + min(max(q.x,q.y),0.0) - radius;
        return shape_coverage(distance);
    }
    if kind == 3u || kind == 4u {
        return shape_coverage(length((p - vec2f(0.5)) * 2.0) - 1.0);
    }
    if kind == 5u {
        let a = uniforms.params1.xy; let b = uniforms.color.xy; let ab = b-a;
        let t = clamp(dot(p-a,ab)/max(dot(ab,ab),0.00001),0.0,1.0);
        return shape_coverage(length(p-(a+ab*t)) - max(uniforms.params1.z,0.5));
    }
    return select(
        0.0,
        textureSampleLevel(mask_texture, linear_sampler, p, 0.0).a,
        all(p >= vec2f(0.0)) && all(p <= vec2f(1.0))
    );
}

fn composite(source: vec4f, destination: vec4f, mode: u32, op: u32) -> vec4f {
    let source_alpha = clamp(source.a,0.0,1.0); let destination_alpha = clamp(destination.a,0.0,1.0);
    let source_color = select(vec3f(0.0), source.rgb / source_alpha, source_alpha > 0.000001);
    let destination_color = select(vec3f(0.0), destination.rgb / destination_alpha, destination_alpha > 0.000001);
    let blended = (1.0-destination_alpha)*source_color
        + destination_alpha*blend(mode,destination_color,source_color);
    var fa = 1.0; var fb = 1.0-source_alpha;
    if op == 1u { fb = 0.0; }
    if op == 2u { fa = 1.0-destination_alpha; fb = 1.0; }
    let alpha = source_alpha*fa + destination_alpha*fb;
    let premul = source_alpha*fa*blended + destination_alpha*fb*destination_color;
    return vec4f(premul,alpha);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let logical = uniforms.output.xy + position.xy / uniforms.output.z;
    var source_logical = logical;
    if uniforms.op.x == 1u || uniforms.op.x == 3u { source_logical = inverse_point(logical); }
    var source = sample_at(foreground_texture, uniforms.input0, source_logical);
    source *= clamp(uniforms.params0.x,0.0,1.0) * mask_alpha(logical);
    if uniforms.op.x == 2u || uniforms.op.x == 3u { return source; }
    let destination = sample_at(backdrop_texture, uniforms.input1, logical);
    return composite(source,destination,uniforms.op.y,uniforms.op.z);
}
