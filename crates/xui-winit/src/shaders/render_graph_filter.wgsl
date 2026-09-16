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

@group(0) @binding(0) var source0: texture_2d<f32>;
@group(0) @binding(1) var source1: texture_2d<f32>;
@group(0) @binding(2) var source2: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var<uniform> uniforms: PassUniform;

struct VertexOutput {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2f, 3>(
        vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0)
    );
    var output: VertexOutput;
    output.position = vec4f(positions[index], 0.0, 1.0);
    return output;
}

fn logical_position(position: vec2f) -> vec2f {
    return uniforms.output.xy + position / uniforms.output.z;
}

fn sample_texture(texture: texture_2d<f32>, info: vec4f, logical: vec2f) -> vec4f {
    let pixel = (logical - info.xy) * uniforms.output.z;
    if any(pixel < vec2f(0.0)) || any(pixel >= info.zw) {
        return vec4f(0.0);
    }
    let allocation_extent = vec2f(textureDimensions(texture));
    return textureSampleLevel(texture, linear_sampler, pixel / allocation_extent, 0.0);
}

fn sample0(logical: vec2f) -> vec4f {
    return sample_texture(source0, uniforms.input0, logical);
}

// Adjacent Gaussian taps are paired into one bilinear sample. This halves the
// number of texture fetches while retaining the discrete Gaussian weights.
fn gaussian(logical: vec2f, axis: vec2f) -> vec4f {
    let sigma = max(uniforms.params0.x, 0.0001);
    let radius = min(i32(ceil(sigma * uniforms.params0.y)), 128);
    var result = sample0(logical);
    var total = 1.0;
    var i = 1;
    loop {
        if i > radius { break; }
        let x0 = f32(i);
        let x1 = min(f32(i + 1), f32(radius));
        let w0 = exp(-0.5 * x0 * x0 / (sigma * sigma));
        let w1 = select(0.0, exp(-0.5 * x1 * x1 / (sigma * sigma)), i + 1 <= radius);
        let pair_weight = w0 + w1;
        let offset = (x0 * w0 + x1 * w1) / max(pair_weight, 0.000001);
        let logical_offset = axis * offset / uniforms.output.z;
        result += (sample0(logical + logical_offset) + sample0(logical - logical_offset)) * pair_weight;
        total += 2.0 * pair_weight;
        i += 2;
    }
    return result / total;
}

fn over(front: vec4f, back: vec4f) -> vec4f {
    let af = clamp(front.a, 0.0, 1.0);
    let ab = clamp(back.a, 0.0, 1.0);
    return vec4f(front.rgb + back.rgb * (1.0 - af), af + ab * (1.0 - af));
}

fn unpremultiply(value: vec4f) -> vec4f {
    return select(vec4f(0.0), vec4f(value.rgb / value.a, value.a), value.a > 0.000001);
}

fn premultiply(value: vec4f) -> vec4f {
    return vec4f(value.rgb * value.a, value.a);
}

@fragment
fn fs_main(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let logical = logical_position(position.xy);
    let kind = uniforms.op.x;
    if kind == 0u { return sample0(logical); }
    if kind == 1u { return gaussian(logical, vec2f(1.0, 0.0)); }
    if kind == 2u { return gaussian(logical, vec2f(0.0, 1.0)); }
    if kind == 3u {
        let value = unpremultiply(sample0(logical));
        let transformed = clamp(vec4f(
            dot(uniforms.matrix[0], value), dot(uniforms.matrix[1], value),
            dot(uniforms.matrix[2], value), dot(uniforms.matrix[3], value)
        ) + uniforms.matrix[4], vec4f(0.0), vec4f(1.0));
        return premultiply(transformed);
    }
    if kind == 4u {
        let block = max(uniforms.params0.xy, vec2f(1.0)) / uniforms.output.z;
        let snapped = (floor(logical / block) + vec2f(0.5)) * block;
        return sample0(snapped);
    }
    if kind == 5u {
        let center = uniforms.params1.xy;
        let delta = logical - center;
        let distance = max(length(delta), 0.0001);
        let direction = delta / distance;
        let displacement = direction * uniforms.params0.x * exp(-distance * 0.02) / uniforms.output.z;
        let chroma = direction * uniforms.params0.y / uniforms.output.z;
        let r = sample0(logical + displacement + chroma).r;
        let g = sample0(logical + displacement).g;
        let b = sample0(logical + displacement - chroma).b;
        let a = sample0(logical + displacement).a;
        return vec4f(r, g, b, a);
    }
    if kind == 6u {
        let offset = uniforms.params0.xy / uniforms.output.z;
        let center = sample0(logical);
        return vec4f(sample0(logical + offset).r, center.g, sample0(logical - offset).b, center.a);
    }
    if kind == 7u {
        let alpha = sample0(logical).a;
        return vec4f(alpha, alpha, alpha, alpha);
    }
    if kind == 8u || kind == 9u {
        let radius = min(i32(ceil(uniforms.params0.x)), 128);
        let axis = select(vec2f(1.0, 0.0), vec2f(0.0, 1.0), kind == 9u);
        var alpha = 0.0;
        for (var i = -radius; i <= radius; i += 1) {
            alpha = max(alpha, sample0(logical + axis * f32(i) / uniforms.output.z).a);
        }
        return vec4f(alpha, alpha, alpha, alpha);
    }
    if kind == 10u {
        let original = sample0(logical);
        let alpha = sample_texture(source1, uniforms.input1, logical - uniforms.params0.xy / uniforms.output.z).a;
        let shadow_alpha = clamp(uniforms.color.a * alpha, 0.0, 1.0);
        return over(original, vec4f(uniforms.color.rgb * shadow_alpha, shadow_alpha));
    }
    let local = vec2f(
        uniforms.inverse0.x * logical.x + uniforms.inverse0.z * logical.y + uniforms.inverse1.x,
        uniforms.inverse0.y * logical.x + uniforms.inverse0.w * logical.y + uniforms.inverse1.y
    );
    let mask_alpha = select(
        0.0,
        textureSampleLevel(source1, linear_sampler, local, 0.0).a,
        all(local >= vec2f(0.0)) && all(local <= vec2f(1.0))
    );
    let color = sample0(logical);
    return color * mask_alpha;
}
