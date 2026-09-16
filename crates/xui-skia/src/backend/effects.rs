//! The SkSL sources this backend ships, and the per-backend cache that
//! compiles each one at most once.

use skia_safe::{Data, ImageFilter, RuntimeEffect, runtime_effect::RuntimeShaderBuilder};
use xui_interface::TextBackend;
use xui_render_graph::{BlendMode, CompositeOperator};

use super::{
    SkiaBackend,
    paint::{blend_index, operator_index},
};
use crate::SkiaBackendError;

pub(super) const PIXELATE_SKSL: &str = r#"
uniform shader source;
uniform float2 block;
half4 main(float2 p) {
    float2 size = max(block, float2(1.0));
    float2 snapped = (floor(p / size) + float2(0.5)) * size;
    return source.eval(snapped);
}
"#;

pub(super) const REFRACTION_SKSL: &str = r#"
uniform shader source;
uniform float2 center;
uniform float2 amount;
half4 main(float2 p) {
    float2 delta = p - center;
    float distance = max(length(delta), 0.0001);
    float2 direction = delta / distance;
    float2 displacement = direction * amount.x * exp(-distance * 0.02);
    float2 chroma = direction * amount.y;
    half4 middle = source.eval(p + displacement);
    return half4(source.eval(p + displacement + chroma).r,
                 middle.g,
                 source.eval(p + displacement - chroma).b,
                 middle.a);
}
"#;

pub(super) const CHROMATIC_ABERRATION_SKSL: &str = r#"
uniform shader source;
uniform float2 offset;
half4 main(float2 p) {
    half4 middle = source.eval(p);
    return half4(source.eval(p + offset).r,
                 middle.g,
                 source.eval(p - offset).b,
                 middle.a);
}
"#;

pub(super) const COMPOSITE_SKSL: &str = r#"
uniform int blend_mode;
uniform int composite_op;

float lum(float3 c) { return dot(c, float3(0.3, 0.59, 0.11)); }
float sat(float3 c) { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
float3 clip_color(float3 c) {
    float l = lum(c), n = min(c.r, min(c.g, c.b)), x = max(c.r, max(c.g, c.b));
    if (n < 0.0) c = float3(l) + (c - float3(l)) * l / (l - n);
    if (x > 1.0) c = float3(l) + (c - float3(l)) * (1.0 - l) / (x - l);
    return c;
}
float3 set_lum(float3 c, float l) { return clip_color(c + float3(l - lum(c))); }
float3 set_sat(float3 c, float s) {
    float lo = min(c.r, min(c.g, c.b)), hi = max(c.r, max(c.g, c.b));
    return hi <= lo ? float3(0.0) : (c - float3(lo)) * s / (hi - lo);
}
float soft_light(float b, float s) {
    if (s <= 0.5) return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    float d = b <= 0.25 ? ((16.0 * b - 12.0) * b + 4.0) * b : sqrt(b);
    return b + (2.0 * s - 1.0) * (d - b);
}
float3 blend(float3 b, float3 s) {
    if (blend_mode == 0) return s;
    if (blend_mode == 1) return b * s;
    if (blend_mode == 2) return b + s - b * s;
    if (blend_mode == 3) return mix(2.0*b*s, 1.0-2.0*(1.0-b)*(1.0-s), step(float3(0.5), b));
    if (blend_mode == 4) return min(b, s);
    if (blend_mode == 5) return max(b, s);
    if (blend_mode == 6) return min(float3(1.0), b / max(float3(0.00001), 1.0-s));
    if (blend_mode == 7) return 1.0-min(float3(1.0), (1.0-b)/max(s,float3(0.00001)));
    if (blend_mode == 8) return mix(2.0*b*s, 1.0-2.0*(1.0-b)*(1.0-s), step(float3(0.5), s));
    if (blend_mode == 9) return float3(soft_light(b.r,s.r),soft_light(b.g,s.g),soft_light(b.b,s.b));
    if (blend_mode == 10) return abs(b-s);
    if (blend_mode == 11) return b+s-2.0*b*s;
    if (blend_mode == 12) return set_lum(set_sat(s,sat(b)),lum(b));
    if (blend_mode == 13) return set_lum(set_sat(b,sat(s)),lum(b));
    if (blend_mode == 14) return set_lum(b,lum(s));
    return set_lum(s,lum(b));
}
half4 main(half4 source, half4 destination) {
    float sa = clamp(float(source.a),0.0,1.0), da = clamp(float(destination.a),0.0,1.0);
    float3 sc = sa > 0.000001 ? float3(source.rgb)/sa : float3(0.0);
    float3 dc = da > 0.000001 ? float3(destination.rgb)/da : float3(0.0);
    float3 mixed = (1.0-da)*sc + da*blend(dc,sc);
    float fa = 1.0, fb = 1.0-sa;
    if (composite_op == 1) fb = 0.0;
    if (composite_op == 2) { fa = 1.0-da; fb = 1.0; }
    float alpha = sa*fa + da*fb;
    return half4(sa*fa*mixed + da*fb*dc, alpha);
}
"#;

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn runtime_filter(
        &mut self,
        name: &'static str,
        source: &'static str,
        uniforms: &[(&str, &[f32])],
    ) -> Result<ImageFilter, SkiaBackendError> {
        let effect = self.runtime_effect(name, source, false)?;
        let mut builder = RuntimeShaderBuilder::new(effect);
        for (uniform, value) in uniforms {
            builder.set_uniform_float(uniform, value).map_err(|error| {
                SkiaBackendError::RuntimeUniform {
                    effect: name,
                    message: error.to_string(),
                }
            })?;
        }
        skia_safe::image_filters::runtime_shader(&builder, "source", None)
            .ok_or(SkiaBackendError::RuntimeShader(name))
    }

    fn runtime_effect(
        &mut self,
        name: &'static str,
        source: &'static str,
        blender: bool,
    ) -> Result<RuntimeEffect, SkiaBackendError> {
        if let Some(effect) = self.runtime_effects.get(name) {
            return Ok(effect.clone());
        }
        let effect = if blender {
            RuntimeEffect::make_for_blender(source, None)
        } else {
            RuntimeEffect::make_for_shader(source, None)
        }
        .map_err(|message| SkiaBackendError::RuntimeEffect {
            effect: name,
            message,
        })?;
        self.runtime_effects.insert(name, effect.clone());
        Ok(effect)
    }

    pub(super) fn runtime_blender(
        &mut self,
        blend: BlendMode,
        operator: CompositeOperator,
    ) -> Result<skia_safe::Blender, SkiaBackendError> {
        let effect = self.runtime_effect("composite", COMPOSITE_SKSL, true)?;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&(blend_index(blend) as i32).to_ne_bytes());
        bytes.extend_from_slice(&(operator_index(operator) as i32).to_ne_bytes());
        effect
            .make_blender(Data::new_copy(&bytes), None)
            .ok_or(SkiaBackendError::RuntimeShader("composite"))
    }
}
