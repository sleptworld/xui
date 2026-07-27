struct EffectUniform {
    params0: vec4f,
    params1: vec4f,
    color: vec4f,
    texture_size: vec4f,
    target_origin_scale: vec4f,
    mask_bounds: vec4f,
    matrix: array<vec4f, 5>,
}

@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var mask_texture: texture_2d<f32>;
@group(0) @binding(2) var effect_sampler: sampler;
@group(0) @binding(3) var<uniform> effect: EffectUniform;

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
    output.uv = vec2f(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return output;
}

fn sample_source(uv: vec2f) -> vec4f {
    if any(uv < vec2f(0.0)) || any(uv > vec2f(1.0)) {
        return vec4f(0.0);
    }
    return textureSample(source_texture, effect_sampler, uv);
}

fn blur(uv: vec2f) -> vec4f {
    let sigma = max(effect.params0.y, 0.01);
    let direction = effect.params0.zw / effect.texture_size.xy;
    var color = sample_source(uv) * 0.227027;
    color += sample_source(uv + direction * sigma * 1.384615) * 0.316216;
    color += sample_source(uv - direction * sigma * 1.384615) * 0.316216;
    color += sample_source(uv + direction * sigma * 3.230769) * 0.070270;
    color += sample_source(uv - direction * sigma * 3.230769) * 0.070270;
    return color;
}

fn over(source: vec4f, destination: vec4f) -> vec4f {
    let source_pm = vec4f(source.rgb * source.a, source.a);
    let destination_pm = vec4f(destination.rgb * destination.a, destination.a);
    let alpha = source_pm.a + destination_pm.a * (1.0 - source_pm.a);
    let rgb = source_pm.rgb + destination_pm.rgb * (1.0 - source_pm.a);
    if alpha <= 0.00001 {
        return vec4f(0.0);
    }
    return vec4f(rgb / alpha, alpha);
}

fn drop_shadow(uv: vec2f) -> vec4f {
    let offset = effect.params1.xy / effect.texture_size.xy;
    let sigma = max(effect.params0.y, 0.5);
    let step_uv = sigma / effect.texture_size.xy;
    var alpha = 0.0;
    var weight = 0.0;
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let distance = f32(x * x + y * y);
            let w = exp(-distance * 0.5);
            alpha += sample_source(uv - offset + vec2f(f32(x), f32(y)) * step_uv).a * w;
            weight += w;
        }
    }
    alpha = clamp(alpha / max(weight, 0.0001), 0.0, 1.0);
    let spread = max(effect.params1.z, 0.0);
    alpha = clamp(alpha + spread / max(sigma * 3.0, 1.0), 0.0, 1.0);
    let shadow = vec4f(effect.color.rgb, effect.color.a * alpha);
    return over(sample_source(uv), shadow);
}

fn color_matrix(color: vec4f) -> vec4f {
    let value = vec4f(
        dot(effect.matrix[0], color),
        dot(effect.matrix[1], color),
        dot(effect.matrix[2], color),
        dot(effect.matrix[3], color),
    ) + effect.matrix[4];
    return clamp(value, vec4f(0.0), vec4f(1.0));
}

fn apply_mask(uv: vec2f) -> vec4f {
    let pixel = uv * effect.texture_size.xy;
    let logical = effect.target_origin_scale.xy + pixel / effect.target_origin_scale.z;
    let relative = (logical - effect.mask_bounds.xy) / effect.mask_bounds.zw;
    var alpha = 0.0;
    if all(relative >= vec2f(0.0)) && all(relative <= vec2f(1.0)) {
        alpha = textureSample(mask_texture, effect_sampler, relative).a;
    }
    let color = sample_source(uv);
    return vec4f(color.rgb, color.a * alpha);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4f {
    let kind = u32(effect.params0.x + 0.5);
    if kind == 0u {
        return blur(input.uv);
    }
    if kind == 1u {
        return drop_shadow(input.uv);
    }
    if kind == 2u {
        return color_matrix(sample_source(input.uv));
    }
    return apply_mask(input.uv);
}
