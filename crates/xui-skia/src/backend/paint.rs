//! Turning styles into Skia paints: solid colors, cached gradient shaders,
//! blend-mode mapping and the small color filters the plan passes need.

use skia_safe::{
    BlendMode as SkBlendMode, Color4f, ImageFilter, Paint, Shader, TileMode,
    gradient::{self, Colors, Gradient, Interpolation},
};
use xui_interface::{
    Bounds, Color, ComputedColorStyle, ComputedLinearGradientStyle, ComputedRadialGradientStyle,
};
use xui_render_graph::{BlendMode, CompositeOperator};

use super::{convert::srgb, lru::LocalLru};

pub(super) fn color_matrix_filter(
    matrix: [f32; 20],
    input: Option<ImageFilter>,
) -> Option<ImageFilter> {
    let matrix = skia_safe::ColorMatrix::new(
        matrix[0], matrix[1], matrix[2], matrix[3], matrix[4], matrix[5], matrix[6], matrix[7],
        matrix[8], matrix[9], matrix[10], matrix[11], matrix[12], matrix[13], matrix[14],
        matrix[15], matrix[16], matrix[17], matrix[18], matrix[19],
    );
    let filter = skia_safe::color_filters::matrix(&matrix, None);
    skia_safe::image_filters::color_filter(filter, input, None)
}

pub(super) fn extract_alpha_filter() -> ImageFilter {
    color_matrix_filter(
        [
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
        ],
        None,
    )
    .expect("a finite alpha color matrix is supported")
}

pub(super) fn shadow_color_filter(color: Color) -> Option<skia_safe::ColorFilter> {
    let matrix = skia_safe::ColorMatrix::new(
        0.0, 0.0, 0.0, color.r, 0.0, 0.0, 0.0, 0.0, color.g, 0.0, 0.0, 0.0, 0.0, color.b, 0.0, 0.0,
        0.0, 0.0, color.a, 0.0,
    );
    Some(skia_safe::color_filters::matrix(&matrix, None))
}

pub(super) fn sk_blend_mode(value: BlendMode) -> SkBlendMode {
    match value {
        BlendMode::Normal => SkBlendMode::SrcOver,
        BlendMode::Multiply => SkBlendMode::Multiply,
        BlendMode::Screen => SkBlendMode::Screen,
        BlendMode::Overlay => SkBlendMode::Overlay,
        BlendMode::Darken => SkBlendMode::Darken,
        BlendMode::Lighten => SkBlendMode::Lighten,
        BlendMode::ColorDodge => SkBlendMode::ColorDodge,
        BlendMode::ColorBurn => SkBlendMode::ColorBurn,
        BlendMode::HardLight => SkBlendMode::HardLight,
        BlendMode::SoftLight => SkBlendMode::SoftLight,
        BlendMode::Difference => SkBlendMode::Difference,
        BlendMode::Exclusion => SkBlendMode::Exclusion,
        BlendMode::Hue => SkBlendMode::Hue,
        BlendMode::Saturation => SkBlendMode::Saturation,
        BlendMode::Color => SkBlendMode::Color,
        BlendMode::Luminosity => SkBlendMode::Luminosity,
    }
}

pub(super) fn composite_blend_mode(blend: BlendMode, operator: CompositeOperator) -> SkBlendMode {
    if blend != BlendMode::Normal {
        return sk_blend_mode(blend);
    }
    match operator {
        CompositeOperator::SrcOver => SkBlendMode::SrcOver,
        CompositeOperator::Src => SkBlendMode::Src,
        CompositeOperator::DstOver => SkBlendMode::DstOver,
    }
}

pub(super) fn blend_index(value: BlendMode) -> u32 {
    match value {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::Darken => 4,
        BlendMode::Lighten => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::HardLight => 8,
        BlendMode::SoftLight => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

pub(super) fn operator_index(value: CompositeOperator) -> u32 {
    match value {
        CompositeOperator::SrcOver => 0,
        CompositeOperator::Src => 1,
        CompositeOperator::DstOver => 2,
    }
}

pub(super) fn sk_color(color: Color) -> skia_safe::Color {
    skia_safe::Color::from_argb(
        (color.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn sk_color4f(color: Color) -> Color4f {
    Color4f::new(color.r, color.g, color.b, color.a)
}

pub(super) fn alpha_color(mut color: Color, opacity: f32) -> Color {
    color.a *= opacity.clamp(0.0, 1.0);
    color
}

pub(super) fn solid_paint(color: Color) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color4f(sk_color4f(color), srgb().as_ref());
    paint
}

/// Identifies a gradient shader by every input that shapes it.
///
/// The fields are destructured rather than read through accessors so that
/// adding one to `ComputedColorStyle` fails to compile here instead of
/// silently producing a key that two different gradients share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct GradientKey([u32; 18]);

impl GradientKey {
    pub(super) fn new(style: ComputedColorStyle, rect: Bounds, opacity: f32) -> Option<Self> {
        // 1 discriminant + 8 geometry/colour slots + 4 rect + 1 opacity, with
        // both arms padded to the same 13 before the rect.
        let mut bits = [0u32; 18];
        let mut at = 0;
        let mut push = |value: f32| {
            bits[at] = value.to_bits();
            at += 1;
        };
        match style {
            ComputedColorStyle::Solid(_) => return None,
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start,
                end,
                from,
                to,
            }) => {
                push(0.0);
                push(start.x);
                push(start.y);
                push(end.x);
                push(end.y);
                for color in [from, to] {
                    push(color.r);
                    push(color.g);
                    push(color.b);
                    push(color.a);
                }
            }
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center,
                radius,
                from,
                to,
            }) => {
                push(1.0);
                push(center.x);
                push(center.y);
                push(radius);
                for color in [from, to] {
                    push(color.r);
                    push(color.g);
                    push(color.b);
                    push(color.a);
                }
                push(0.0);
            }
        }
        push(rect.min.x);
        push(rect.min.y);
        push(rect.max.x);
        push(rect.max.y);
        push(opacity);
        debug_assert_eq!(at, bits.len(), "gradient key left a slot unwritten");
        Some(Self(bits))
    }
}

fn gradient_shader(style: ComputedColorStyle, rect: Bounds, opacity: f32) -> Option<Shader> {
    let (from, to) = match style {
        ComputedColorStyle::Solid(_) => return None,
        ComputedColorStyle::LinearGradient(value) => (value.from, value.to),
        ComputedColorStyle::RadialGradient(value) => (value.from, value.to),
    };
    let colors = [
        sk_color4f(alpha_color(from, opacity)),
        sk_color4f(alpha_color(to, opacity)),
    ];
    let colors = Colors::new_evenly_spaced(&colors, TileMode::Clamp, srgb());
    let gradient = Gradient::new(colors, Interpolation::default());
    match style {
        ComputedColorStyle::Solid(_) => None,
        ComputedColorStyle::LinearGradient(value) => {
            let start = (
                rect.x() + rect.width() * value.start.x,
                rect.y() + rect.height() * value.start.y,
            );
            let end = (
                rect.x() + rect.width() * value.end.x,
                rect.y() + rect.height() * value.end.y,
            );
            gradient::shaders::linear_gradient((start, end), &gradient, None)
        }
        ComputedColorStyle::RadialGradient(value) => {
            let center = (
                rect.x() + rect.width() * value.center.x,
                rect.y() + rect.height() * value.center.y,
            );
            let radius = value.radius * rect.width().min(rect.height());
            gradient::shaders::radial_gradient((center, radius.max(0.001)), &gradient, None)
        }
    }
}

/// Builds the paint for a fill or stroke, taking the gradient shader from
/// `cache` when one was already built for the same gradient and geometry.
pub(super) fn style_paint(
    cache: &mut LocalLru<GradientKey, Shader>,
    style: ComputedColorStyle,
    rect: Bounds,
    opacity: f32,
) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    if let ComputedColorStyle::Solid(color) = style {
        paint.set_color4f(sk_color4f(alpha_color(color, opacity)), srgb().as_ref());
        return paint;
    }
    let key = GradientKey::new(style, rect, opacity);
    if let Some(key) = key
        && let Some(shader) = cache.get(&key)
    {
        paint.set_shader(shader);
        return paint;
    }
    if let Some(shader) = gradient_shader(style, rect, opacity) {
        if let Some(key) = key {
            cache.insert(key, shader.clone());
        }
        paint.set_shader(shader);
    }
    paint
}
