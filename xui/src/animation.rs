use std::sync::Arc;

use xui_animation::Animatable;
use xui_interface::{
    Affine, Color, ColorMatrix, ComputedBackdropFilter, ComputedBackdropMask,
    ComputedBackdropStyle, ComputedColorStyle, ComputedEffect, ComputedLinearGradientStyle,
    ComputedMaskShape, ComputedRadialGradientStyle, ComputedShadowStyle, ComputedStrokeStyle,
    ComputedStyle, EdgeInsets, FontWeight, LineHeight, Point, Size, Sizing,
};

macro_rules! interpolate_fields {
    ($sampled:expr, $from:expr, $to:expr, $progress:expr, $interpolate:ident; $($field:ident),+ $(,)?) => {
        $(
            $sampled.$field = $interpolate($from.$field, $to.$field, $progress);
        )+
    };
}

macro_rules! collect_changed_fields {
    ($from_values:expr, $to_values:expr, $from:expr, $to:expr; $($field:ident),+ $(,)?) => {
        $(
            if $from.$field != $to.$field {
                $from_values.$field = Some($from.$field);
                $to_values.$field = Some($to.$field);
            }
        )+
    };
}

/// Legacy paint-only animation value. Runtime style transitions now sample a
/// complete [`ComputedStyle`], but this type remains available for callers
/// that use the former manual animation API.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnimableStyle {
    pub paint: AnimablePaintStyle,
}

impl AnimableStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.paint.is_empty()
    }

    pub fn has_properties(&self) -> bool {
        !self.is_empty()
    }

    pub fn diff(from: &ComputedStyle, to: &ComputedStyle) -> (Self, Self) {
        let mut from_value = Self::default();
        let mut to_value = Self::default();
        collect_changed_fields!(
            from_value.paint,
            to_value.paint,
            from.paint,
            to.paint;
            background,
            border_color,
            border_width,
            border_radius,
        );
        if from.paint.stroke != to.paint.stroke {
            let (from, to) = match (from.paint.stroke, to.paint.stroke) {
                (Some(from), Some(to)) => (from, to),
                (None, Some(to)) => (neutral_stroke(to), to),
                (Some(from), None) => (from, neutral_stroke(from)),
                (None, None) => unreachable!(),
            };
            from_value.paint.stroke = Some(from);
            to_value.paint.stroke = Some(to);
        }
        if from.paint.shadow != to.paint.shadow {
            let (from, to) = match (from.paint.shadow, to.paint.shadow) {
                (Some(from), Some(to)) => (from, to),
                (None, Some(to)) => (neutral_shadow(to), to),
                (Some(from), None) => (from, neutral_shadow(from)),
                (None, None) => unreachable!(),
            };
            from_value.paint.shadow = Some(from);
            to_value.paint.shadow = Some(to);
        }
        (from_value, to_value)
    }

    pub fn capture(style: &ComputedStyle, mask: &Self) -> Self {
        Self {
            paint: AnimablePaintStyle {
                background: mask.paint.background.map(|_| style.paint.background),
                border_color: mask.paint.border_color.map(|_| style.paint.border_color),
                border_width: mask.paint.border_width.map(|_| style.paint.border_width),
                border_radius: mask.paint.border_radius.map(|_| style.paint.border_radius),
                stroke: mask.paint.stroke.and(style.paint.stroke),
                shadow: mask.paint.shadow.and(style.paint.shadow),
            },
        }
    }

    pub fn apply_to_computed(&self, style: &mut ComputedStyle) {
        if let Some(value) = self.paint.background {
            style.paint.background = value;
        }
        if let Some(value) = self.paint.border_color {
            style.paint.border_color = value;
        }
        if let Some(value) = self.paint.border_width {
            style.paint.border_width = value;
        }
        if let Some(value) = self.paint.border_radius {
            style.paint.border_radius = value;
        }
        if let Some(value) = self.paint.stroke {
            style.paint.stroke = Some(value);
        }
        if let Some(value) = self.paint.shadow {
            style.paint.shadow = Some(value);
        }
    }
}

impl Animatable for AnimableStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            paint: AnimablePaintStyle::interpolate(&from.paint, &to.paint, progress),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AnimablePaintStyle {
    pub background: Option<ComputedColorStyle>,
    pub border_color: Option<ComputedColorStyle>,
    pub border_width: Option<f32>,
    pub border_radius: Option<f32>,
    pub stroke: Option<ComputedStrokeStyle>,
    pub shadow: Option<ComputedShadowStyle>,
}

impl AnimablePaintStyle {
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.border_color.is_none()
            && self.border_width.is_none()
            && self.border_radius.is_none()
            && self.stroke.is_none()
            && self.shadow.is_none()
    }
}

impl Animatable for AnimablePaintStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            background: interpolate_option(from.background, to.background, progress, color),
            border_color: interpolate_option(from.border_color, to.border_color, progress, color),
            border_width: interpolate_option(from.border_width, to.border_width, progress, lerp),
            border_radius: interpolate_option(from.border_radius, to.border_radius, progress, lerp),
            stroke: interpolate_option(from.stroke, to.stroke, progress, stroke),
            shadow: interpolate_option(from.shadow, to.shadow, progress, shadow),
        }
    }
}

fn interpolate_option<T: Copy>(
    from: Option<T>,
    to: Option<T>,
    progress: f32,
    interpolate: impl FnOnce(T, T, f32) -> T,
) -> Option<T> {
    match (from, to) {
        (Some(from), Some(to)) => Some(interpolate(from, to, progress)),
        _ => None,
    }
}

/// Whether the styles differ in at least one property supported by XUI's
/// transition sampler. Incompatible color representations use an end-of-
/// timeline discrete switch and therefore also count as transitionable.
pub(crate) fn has_animatable_difference(from: &ComputedStyle, to: &ComputedStyle) -> bool {
    interpolate_style(from, to, 0.5) != *to
}

/// Samples all animatable properties. Most enum/resource and incompatible
/// value kinds apply immediately; incompatible colors switch on completion.
pub(crate) fn interpolate_style(
    from: &ComputedStyle,
    to: &ComputedStyle,
    progress: f32,
) -> ComputedStyle {
    let progress = progress.clamp(0.0, 1.0);
    if progress >= 1.0 {
        return to.clone();
    }
    let mut sampled = to.clone();

    interpolate_fields!(sampled.text, from.text, to.text, progress, rgba; color);
    interpolate_fields!(
        sampled.text,
        from.text,
        to.text,
        progress,
        lerp;
        font_size,
        letter_spacing,
    );
    if from.text.font_weight != to.text.font_weight {
        sampled.text.font_weight =
            font_weight(from.text.font_weight, to.text.font_weight, progress);
    }
    sampled.text.line_height = line_height(from.text.line_height, to.text.line_height, progress);

    interpolate_fields!(sampled.layout, from.layout, to.layout, progress, lerp; gap);
    interpolate_fields!(
        sampled.layout,
        from.layout,
        to.layout,
        progress,
        sizing;
        width,
        height,
    );
    interpolate_fields!(
        sampled.layout,
        from.layout,
        to.layout,
        progress,
        optional_sizing;
        min_width,
        min_height,
        max_width,
        max_height,
    );
    interpolate_fields!(
        sampled.layout,
        from.layout,
        to.layout,
        progress,
        insets;
        margin,
        padding,
    );
    sampled.layout.inset = optional_insets(from.layout.inset, to.layout.inset, progress);

    interpolate_fields!(
        sampled.paint,
        from.paint,
        to.paint,
        progress,
        color;
        background,
        border_color,
    );
    interpolate_fields!(
        sampled.paint,
        from.paint,
        to.paint,
        progress,
        lerp;
        border_width,
        border_radius,
    );
    sampled.paint.stroke = optional_stroke(from.paint.stroke, to.paint.stroke, progress);
    sampled.paint.shadow = optional_shadow(from.paint.shadow, to.paint.shadow, progress);

    sampled.effect.backdrop =
        optional_backdrop(&from.effect.backdrop, &to.effect.backdrop, progress);
    sampled.effect.effects = effect_list(&from.effect.effects, &to.effect.effects, progress)
        .unwrap_or_else(|| to.effect.effects.clone());

    interpolate_fields!(
        sampled.scroll.scrollbar,
        from.scroll.scrollbar,
        to.scroll.scrollbar,
        progress,
        lerp;
        width,
        radius,
    );
    interpolate_fields!(
        sampled.scroll.scrollbar,
        from.scroll.scrollbar,
        to.scroll.scrollbar,
        progress,
        color;
        track_color,
        thumb_color,
    );
    sampled
}

fn sizing(from: Sizing, to: Sizing, progress: f32) -> Sizing {
    match (from, to) {
        (Sizing::Fix(from), Sizing::Fix(to)) => {
            Sizing::fix(lerp(from.into_inner(), to.into_inner(), progress))
        }
        (Sizing::Percent(from), Sizing::Percent(to)) => {
            Sizing::percent(lerp(from.into_inner(), to.into_inner(), progress))
        }
        _ => to,
    }
}

fn compatible_sizing(from: Sizing, to: Sizing) -> bool {
    matches!(
        (from, to),
        (Sizing::Fix(_), Sizing::Fix(_)) | (Sizing::Percent(_), Sizing::Percent(_))
    )
}

fn optional_sizing(from: Option<Sizing>, to: Option<Sizing>, progress: f32) -> Option<Sizing> {
    match (from, to) {
        (Some(from), Some(to)) if compatible_sizing(from, to) => Some(sizing(from, to, progress)),
        _ => to,
    }
}

fn insets(from: EdgeInsets, to: EdgeInsets, progress: f32) -> EdgeInsets {
    EdgeInsets::new(
        lerp(from.left(), to.left(), progress),
        lerp(from.top(), to.top(), progress),
        lerp(from.right(), to.right(), progress),
        lerp(from.bottom(), to.bottom(), progress),
    )
}

fn optional_insets(
    from: Option<EdgeInsets>,
    to: Option<EdgeInsets>,
    progress: f32,
) -> Option<EdgeInsets> {
    match (from, to) {
        (Some(from), Some(to)) => Some(insets(from, to, progress)),
        _ => to,
    }
}

fn font_weight(from: FontWeight, to: FontWeight, progress: f32) -> FontWeight {
    FontWeight::Number(
        lerp(
            weight_number(from) as f32,
            weight_number(to) as f32,
            progress,
        )
        .round() as u16,
    )
}

fn weight_number(weight: FontWeight) -> u16 {
    match weight {
        FontWeight::Thin => 100,
        FontWeight::ExtraLight => 200,
        FontWeight::Light => 300,
        FontWeight::Normal => 400,
        FontWeight::Medium => 500,
        FontWeight::SemiBold => 600,
        FontWeight::Bold => 700,
        FontWeight::ExtraBold => 800,
        FontWeight::Black => 900,
        FontWeight::Number(value) => value,
    }
}

fn line_height(from: LineHeight, to: LineHeight, progress: f32) -> LineHeight {
    match (from, to) {
        (LineHeight::Px(from), LineHeight::Px(to)) => LineHeight::Px(lerp(from, to, progress)),
        (LineHeight::Em(from), LineHeight::Em(to)) => LineHeight::Em(lerp(from, to, progress)),
        _ => to,
    }
}

fn color(from: ComputedColorStyle, to: ComputedColorStyle, progress: f32) -> ComputedColorStyle {
    match (from, to) {
        (ComputedColorStyle::Solid(from), ComputedColorStyle::Solid(to)) => {
            ComputedColorStyle::Solid(rgba(from, to, progress))
        }
        (ComputedColorStyle::LinearGradient(from), ComputedColorStyle::LinearGradient(to)) => {
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: point(from.start, to.start, progress),
                end: point(from.end, to.end, progress),
                from: rgba(from.from, to.from, progress),
                to: rgba(from.to, to.to, progress),
            })
        }
        (ComputedColorStyle::RadialGradient(from), ComputedColorStyle::RadialGradient(to)) => {
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center: point(from.center, to.center, progress),
                radius: lerp(from.radius, to.radius, progress),
                from: rgba(from.from, to.from, progress),
                to: rgba(from.to, to.to, progress),
            })
        }
        (from, _) => from,
    }
}

fn stroke(
    from: ComputedStrokeStyle,
    to: ComputedStrokeStyle,
    progress: f32,
) -> ComputedStrokeStyle {
    ComputedStrokeStyle {
        color: color(from.color, to.color, progress),
        width: lerp(from.width, to.width, progress),
        line_style: from.line_style,
    }
}

fn optional_stroke(
    from: Option<ComputedStrokeStyle>,
    to: Option<ComputedStrokeStyle>,
    progress: f32,
) -> Option<ComputedStrokeStyle> {
    match (from, to) {
        (Some(from), Some(to)) => Some(stroke(from, to, progress)),
        (None, Some(to)) => Some(stroke(neutral_stroke(to), to, progress)),
        (Some(from), None) => {
            (progress < 1.0).then(|| stroke(from, neutral_stroke(from), progress))
        }
        (None, None) => None,
    }
}

fn neutral_stroke(reference: ComputedStrokeStyle) -> ComputedStrokeStyle {
    ComputedStrokeStyle {
        color: transparent_color(reference.color),
        width: 0.0,
        line_style: reference.line_style,
    }
}

fn shadow(
    from: ComputedShadowStyle,
    to: ComputedShadowStyle,
    progress: f32,
) -> ComputedShadowStyle {
    ComputedShadowStyle {
        color: rgba(from.color, to.color, progress),
        offset: point(from.offset, to.offset, progress),
        blur: lerp(from.blur, to.blur, progress),
        spread: lerp(from.spread, to.spread, progress),
    }
}

fn optional_shadow(
    from: Option<ComputedShadowStyle>,
    to: Option<ComputedShadowStyle>,
    progress: f32,
) -> Option<ComputedShadowStyle> {
    match (from, to) {
        (Some(from), Some(to)) => Some(shadow(from, to, progress)),
        (None, Some(to)) => Some(shadow(neutral_shadow(to), to, progress)),
        (Some(from), None) => {
            (progress < 1.0).then(|| shadow(from, neutral_shadow(from), progress))
        }
        (None, None) => None,
    }
}

fn neutral_shadow(reference: ComputedShadowStyle) -> ComputedShadowStyle {
    ComputedShadowStyle {
        color: Color {
            a: 0.0,
            ..reference.color
        },
        blur: 0.0,
        spread: 0.0,
        ..reference
    }
}

fn optional_backdrop(
    from: &Option<ComputedBackdropStyle>,
    to: &Option<ComputedBackdropStyle>,
    progress: f32,
) -> Option<ComputedBackdropStyle> {
    match (from, to) {
        (Some(from), Some(to)) => backdrop(from, to, progress).or_else(|| Some(to.clone())),
        (None, Some(to)) => {
            backdrop(&neutral_backdrop(to), to, progress).or_else(|| Some(to.clone()))
        }
        (Some(from), None) => {
            backdrop(from, &neutral_backdrop(from), progress).filter(|_| progress < 1.0)
        }
        (None, None) => None,
    }
}

fn backdrop(
    from: &ComputedBackdropStyle,
    to: &ComputedBackdropStyle,
    progress: f32,
) -> Option<ComputedBackdropStyle> {
    Some(ComputedBackdropStyle {
        filters: backdrop_filters(&from.filters, &to.filters, progress),
        opacity: lerp(from.opacity, to.opacity, progress),
        blend_mode: to.blend_mode,
        mask: backdrop_mask(&from.mask, &to.mask, progress),
    })
}

fn neutral_backdrop(reference: &ComputedBackdropStyle) -> ComputedBackdropStyle {
    ComputedBackdropStyle {
        filters: reference
            .filters
            .iter()
            .map(neutral_backdrop_filter)
            .collect::<Vec<_>>()
            .into(),
        opacity: 0.0,
        blend_mode: reference.blend_mode,
        mask: reference.mask.clone(),
    }
}

fn backdrop_filters(
    from: &[ComputedBackdropFilter],
    to: &[ComputedBackdropFilter],
    progress: f32,
) -> Arc<[ComputedBackdropFilter]> {
    if from.len() != to.len() {
        return to.to_vec().into();
    }
    let filters = from
        .iter()
        .zip(to)
        .map(|(from, to)| backdrop_filter(*from, *to, progress).unwrap_or(*to))
        .collect::<Vec<_>>();
    filters.into()
}

fn backdrop_filter(
    from: ComputedBackdropFilter,
    to: ComputedBackdropFilter,
    progress: f32,
) -> Option<ComputedBackdropFilter> {
    use ComputedBackdropFilter as F;
    Some(match (from, to) {
        (
            F::Blur {
                sigma_x: fx,
                sigma_y: fy,
                ..
            },
            F::Blur {
                sigma_x: tx,
                sigma_y: ty,
                quality,
            },
        ) => F::Blur {
            sigma_x: lerp(fx, tx, progress),
            sigma_y: lerp(fy, ty, progress),
            quality,
        },
        (F::Saturate(from), F::Saturate(to)) => F::Saturate(lerp(from, to, progress)),
        (F::Brightness(from), F::Brightness(to)) => F::Brightness(lerp(from, to, progress)),
        (F::Contrast(from), F::Contrast(to)) => F::Contrast(lerp(from, to, progress)),
        (F::Grayscale(from), F::Grayscale(to)) => F::Grayscale(lerp(from, to, progress)),
        (F::Sepia(from), F::Sepia(to)) => F::Sepia(lerp(from, to, progress)),
        (F::HueRotate(from), F::HueRotate(to)) => F::HueRotate(lerp(from, to, progress)),
        (F::Invert(from), F::Invert(to)) => F::Invert(lerp(from, to, progress)),
        (F::ColorMatrix(from), F::ColorMatrix(to)) => {
            F::ColorMatrix(color_matrix(from, to, progress))
        }
        (F::Pixelate { size: from }, F::Pixelate { size: to }) => F::Pixelate {
            size: size(from, to, progress),
        },
        (
            F::Refraction {
                strength: fs,
                chromatic_aberration: fc,
            },
            F::Refraction {
                strength: ts,
                chromatic_aberration: tc,
            },
        ) => F::Refraction {
            strength: lerp(fs, ts, progress),
            chromatic_aberration: lerp(fc, tc, progress),
        },
        (F::ChromaticAberration { offset: from }, F::ChromaticAberration { offset: to }) => {
            F::ChromaticAberration {
                offset: [
                    lerp(from[0], to[0], progress),
                    lerp(from[1], to[1], progress),
                ],
            }
        }
        _ => return None,
    })
}

fn neutral_backdrop_filter(filter: &ComputedBackdropFilter) -> ComputedBackdropFilter {
    use ComputedBackdropFilter as F;
    match *filter {
        F::Blur { quality, .. } => F::Blur {
            sigma_x: 0.0,
            sigma_y: 0.0,
            quality,
        },
        F::Saturate(_) => F::Saturate(1.0),
        F::Brightness(_) => F::Brightness(1.0),
        F::Contrast(_) => F::Contrast(1.0),
        F::Grayscale(_) => F::Grayscale(0.0),
        F::Sepia(_) => F::Sepia(0.0),
        F::HueRotate(_) => F::HueRotate(0.0),
        F::Invert(_) => F::Invert(0.0),
        F::ColorMatrix(_) => F::ColorMatrix(identity_matrix()),
        F::Pixelate { .. } => F::Pixelate {
            size: Size::new(1.0, 1.0),
        },
        F::Refraction { .. } => F::Refraction {
            strength: 0.0,
            chromatic_aberration: 0.0,
        },
        F::ChromaticAberration { .. } => F::ChromaticAberration { offset: [0.0; 2] },
    }
}

fn backdrop_mask(
    from: &ComputedBackdropMask,
    to: &ComputedBackdropMask,
    progress: f32,
) -> ComputedBackdropMask {
    use ComputedBackdropMask as M;
    match (from, to) {
        (
            M::Shape {
                shape: fs,
                transform: ft,
            },
            M::Shape {
                shape: ts,
                transform: tt,
            },
        ) => M::Shape {
            shape: mask_shape(*fs, *ts, progress),
            transform: affine(*ft, *tt, progress),
        },
        (
            M::AlphaTexture {
                texture: fi,
                transform: ft,
            },
            M::AlphaTexture {
                texture: ti,
                transform: tt,
            },
        ) if fi == ti => M::AlphaTexture {
            texture: ti.clone(),
            transform: affine(*ft, *tt, progress),
        },
        _ => to.clone(),
    }
}

fn mask_shape(from: ComputedMaskShape, to: ComputedMaskShape, progress: f32) -> ComputedMaskShape {
    use ComputedMaskShape as S;
    match (from, to) {
        (S::RoundedRect(from), S::RoundedRect(to)) => S::RoundedRect(lerp(from, to, progress)),
        (S::Line { from: ff, to: ft }, S::Line { from: tf, to: tt }) => S::Line {
            from: point(ff, tf, progress),
            to: point(ft, tt, progress),
        },
        (S::Rect, S::Rect) => S::Rect,
        (S::Circle, S::Circle) => S::Circle,
        (S::Ellipse, S::Ellipse) => S::Ellipse,
        _ => to,
    }
}

fn effect_list(
    from: &[ComputedEffect],
    to: &[ComputedEffect],
    progress: f32,
) -> Option<Arc<[ComputedEffect]>> {
    let (from, to) = match (from.is_empty(), to.is_empty()) {
        (false, false) if from.len() == to.len() => (from.to_vec(), to.to_vec()),
        (true, false) => (
            to.iter().map(neutral_effect).collect::<Option<Vec<_>>>()?,
            to.to_vec(),
        ),
        (false, true) => (
            from.to_vec(),
            from.iter()
                .map(neutral_effect)
                .collect::<Option<Vec<_>>>()?,
        ),
        (true, true) => return Some(Arc::from([])),
        _ => return None,
    };
    let effects = from
        .iter()
        .zip(&to)
        .map(|(from, to)| effect(from, to, progress).unwrap_or_else(|| to.clone()))
        .collect::<Vec<_>>();
    Some(effects.into())
}

fn effect(from: &ComputedEffect, to: &ComputedEffect, progress: f32) -> Option<ComputedEffect> {
    use ComputedEffect as E;
    Some(match (from, to) {
        (
            E::Blur {
                sigma_x: fx,
                sigma_y: fy,
                ..
            },
            E::Blur {
                sigma_x: tx,
                sigma_y: ty,
                quality,
            },
        ) => E::Blur {
            sigma_x: lerp(*fx, *tx, progress),
            sigma_y: lerp(*fy, *ty, progress),
            quality: *quality,
        },
        (
            E::DropShadow {
                color: fc,
                offset: fo,
                sigma_x: fsx,
                sigma_y: fsy,
                spread: fs,
                ..
            },
            E::DropShadow {
                color: tc,
                offset: to,
                sigma_x: tsx,
                sigma_y: tsy,
                spread: ts,
                quality,
            },
        ) => E::DropShadow {
            color: rgba(*fc, *tc, progress),
            offset: point(*fo, *to, progress),
            sigma_x: lerp(*fsx, *tsx, progress),
            sigma_y: lerp(*fsy, *tsy, progress),
            spread: lerp(*fs, *ts, progress),
            quality: *quality,
        },
        (E::ColorMatrix(from), E::ColorMatrix(to)) => {
            E::ColorMatrix(color_matrix(*from, *to, progress))
        }
        _ => return None,
    })
}

fn neutral_effect(effect: &ComputedEffect) -> Option<ComputedEffect> {
    use ComputedEffect as E;
    Some(match effect {
        E::Blur { quality, .. } => E::Blur {
            sigma_x: 0.0,
            sigma_y: 0.0,
            quality: *quality,
        },
        E::DropShadow {
            color,
            offset,
            quality,
            ..
        } => E::DropShadow {
            color: Color { a: 0.0, ..*color },
            offset: *offset,
            sigma_x: 0.0,
            sigma_y: 0.0,
            spread: 0.0,
            quality: *quality,
        },
        E::ColorMatrix(_) => E::ColorMatrix(identity_matrix()),
        E::ImageMask { .. } => return None,
    })
}

fn affine(from: Affine, to: Affine, progress: f32) -> Affine {
    Affine::new(
        lerp(from.xx, to.xx, progress),
        lerp(from.yx, to.yx, progress),
        lerp(from.xy, to.xy, progress),
        lerp(from.yy, to.yy, progress),
        lerp(from.dx, to.dx, progress),
        lerp(from.dy, to.dy, progress),
    )
}

fn size(from: Size<f32>, to: Size<f32>, progress: f32) -> Size<f32> {
    Size::new(
        lerp(from.width, to.width, progress),
        lerp(from.height, to.height, progress),
    )
}

fn color_matrix(from: ColorMatrix, to: ColorMatrix, progress: f32) -> ColorMatrix {
    std::array::from_fn(|index| lerp(from[index], to[index], progress))
}

fn identity_matrix() -> ColorMatrix {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ]
}

fn transparent_color(value: ComputedColorStyle) -> ComputedColorStyle {
    match value {
        ComputedColorStyle::Solid(color) => ComputedColorStyle::Solid(Color { a: 0.0, ..color }),
        ComputedColorStyle::LinearGradient(mut gradient) => {
            gradient.from.a = 0.0;
            gradient.to.a = 0.0;
            ComputedColorStyle::LinearGradient(gradient)
        }
        ComputedColorStyle::RadialGradient(mut gradient) => {
            gradient.from.a = 0.0;
            gradient.to.a = 0.0;
            ComputedColorStyle::RadialGradient(gradient)
        }
    }
}

fn rgba(from: Color, to: Color, progress: f32) -> Color {
    Color {
        r: lerp(from.r, to.r, progress),
        g: lerp(from.g, to.g, progress),
        b: lerp(from.b, to.b, progress),
        a: lerp(from.a, to.a, progress),
    }
}

fn point(from: Point, to: Point, progress: f32) -> Point {
    Point::new(lerp(from.x, to.x, progress), lerp(from.y, to.y, progress))
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{StylePatch, Theme};

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn samples_text_layout_paint_and_scroll() {
        let theme = Theme::default();
        let mut from = ComputedStyle::initial(&theme);
        from.layout.width = Sizing::from(100.0);
        let to = ComputedStyle::compute(
            &from,
            &StylePatch::new()
                .color(Color::WHITE)
                .font_size(24.0)
                .width(200.0)
                .padding(EdgeInsets::all(20.0))
                .background(Color::WHITE)
                .scrollbar_width(16.0),
            &theme,
        );
        let sampled = interpolate_style(&from, &to, 0.5);
        let Sizing::Fix(width) = sampled.layout.width else {
            panic!("fixed width")
        };
        near(width.into_inner(), 150.0);
        near(sampled.layout.padding.left(), 10.0);
        near(sampled.scroll.scrollbar.width, 12.0);
        assert!(has_animatable_difference(&from, &to));
    }

    #[test]
    fn incompatible_values_and_clip_are_discrete() {
        let theme = Theme::default();
        let from = ComputedStyle::initial(&theme);
        let mut to = from.clone();
        to.layout.width = Sizing::Fill;
        to.paint.clip = true;
        assert!(!has_animatable_difference(&from, &to));
        let sampled = interpolate_style(&from, &to, 0.25);
        assert_eq!(sampled.layout.width, Sizing::Fill);
        assert!(sampled.paint.clip);
    }

    #[test]
    fn incompatible_color_representation_switches_at_completion() {
        let theme = Theme::default();
        let mut from = ComputedStyle::initial(&theme);
        from.paint.background = ComputedColorStyle::Solid(Color::BLACK);
        let mut to = from.clone();
        to.paint.background = ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
            start: Point::zero(),
            end: Point::new(1.0, 1.0),
            from: Color::BLACK,
            to: Color::WHITE,
        });

        assert!(has_animatable_difference(&from, &to));
        assert_eq!(
            interpolate_style(&from, &to, 0.5).paint.background,
            from.paint.background
        );
        assert_eq!(interpolate_style(&from, &to, 1.0), to);
    }
}
