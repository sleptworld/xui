use xui_animation::Animatable;
use xui_interface::{
    Color, ComputedColorStyle, ComputedLinearGradientStyle, ComputedRadialGradientStyle,
    ComputedShadowStyle, ComputedStrokeStyle, ComputedStyle, Point,
};

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
        let mut from_anim = Self::default();
        let mut to_anim = Self::default();

        if from.paint.background != to.paint.background {
            from_anim.paint.background = Some(from.paint.background);
            to_anim.paint.background = Some(to.paint.background);
        }
        if from.paint.border_color != to.paint.border_color {
            from_anim.paint.border_color = Some(from.paint.border_color);
            to_anim.paint.border_color = Some(to.paint.border_color);
        }
        if from.paint.border_width != to.paint.border_width {
            from_anim.paint.border_width = Some(from.paint.border_width);
            to_anim.paint.border_width = Some(to.paint.border_width);
        }
        if from.paint.border_radius != to.paint.border_radius {
            from_anim.paint.border_radius = Some(from.paint.border_radius);
            to_anim.paint.border_radius = Some(to.paint.border_radius);
        }
        if from.paint.stroke != to.paint.stroke {
            let (from_stroke, to_stroke) = stroke_endpoints(from.paint.stroke, to.paint.stroke);
            from_anim.paint.stroke = Some(from_stroke);
            to_anim.paint.stroke = Some(to_stroke);
        }
        if from.paint.shadow != to.paint.shadow {
            let (from_shadow, to_shadow) = shadow_endpoints(from.paint.shadow, to.paint.shadow);
            from_anim.paint.shadow = Some(from_shadow);
            to_anim.paint.shadow = Some(to_shadow);
        }

        (from_anim, to_anim)
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
        if let Some(background) = self.paint.background {
            style.paint.background = background;
        }
        if let Some(border_color) = self.paint.border_color {
            style.paint.border_color = border_color;
        }
        if let Some(border_width) = self.paint.border_width {
            style.paint.border_width = border_width;
        }
        if let Some(border_radius) = self.paint.border_radius {
            style.paint.border_radius = border_radius;
        }
        if let Some(stroke) = self.paint.stroke {
            style.paint.stroke = Some(stroke);
        }
        if let Some(shadow) = self.paint.shadow {
            style.paint.shadow = Some(shadow);
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
            background: interpolate_option(
                from.background,
                to.background,
                progress,
                interpolate_color,
            ),
            border_color: interpolate_option(
                from.border_color,
                to.border_color,
                progress,
                interpolate_color,
            ),
            border_width: interpolate_option(from.border_width, to.border_width, progress, lerp),
            border_radius: interpolate_option(from.border_radius, to.border_radius, progress, lerp),
            stroke: interpolate_option(from.stroke, to.stroke, progress, interpolate_stroke),
            shadow: interpolate_option(from.shadow, to.shadow, progress, interpolate_shadow),
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

fn interpolate_color(
    from: ComputedColorStyle,
    to: ComputedColorStyle,
    progress: f32,
) -> ComputedColorStyle {
    match (from, to) {
        (ComputedColorStyle::Solid(from), ComputedColorStyle::Solid(to)) => {
            ComputedColorStyle::Solid(interpolate_rgba(from, to, progress))
        }
        (ComputedColorStyle::LinearGradient(from), ComputedColorStyle::LinearGradient(to)) => {
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: interpolate_point(from.start, to.start, progress),
                end: interpolate_point(from.end, to.end, progress),
                from: interpolate_rgba(from.from, to.from, progress),
                to: interpolate_rgba(from.to, to.to, progress),
            })
        }
        (ComputedColorStyle::RadialGradient(from), ComputedColorStyle::RadialGradient(to)) => {
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center: interpolate_point(from.center, to.center, progress),
                radius: lerp(from.radius, to.radius, progress),
                from: interpolate_rgba(from.from, to.from, progress),
                to: interpolate_rgba(from.to, to.to, progress),
            })
        }
        (from, to) => discrete(from, to, progress),
    }
}

fn interpolate_stroke(
    from: ComputedStrokeStyle,
    to: ComputedStrokeStyle,
    progress: f32,
) -> ComputedStrokeStyle {
    ComputedStrokeStyle {
        color: interpolate_color(from.color, to.color, progress),
        width: lerp(from.width, to.width, progress),
        line_style: discrete(from.line_style, to.line_style, progress),
    }
}

fn interpolate_shadow(
    from: ComputedShadowStyle,
    to: ComputedShadowStyle,
    progress: f32,
) -> ComputedShadowStyle {
    ComputedShadowStyle {
        color: interpolate_rgba(from.color, to.color, progress),
        offset: interpolate_point(from.offset, to.offset, progress),
        blur: lerp(from.blur, to.blur, progress),
        spread: lerp(from.spread, to.spread, progress),
    }
}

fn stroke_endpoints(
    from: Option<ComputedStrokeStyle>,
    to: Option<ComputedStrokeStyle>,
) -> (ComputedStrokeStyle, ComputedStrokeStyle) {
    match (from, to) {
        (Some(from), Some(to)) => (from, to),
        (None, Some(to)) => (neutral_stroke(to), to),
        (Some(from), None) => (from, neutral_stroke(from)),
        (None, None) => unreachable!("equal strokes do not create transition endpoints"),
    }
}

fn neutral_stroke(reference: ComputedStrokeStyle) -> ComputedStrokeStyle {
    ComputedStrokeStyle {
        color: transparent_color_style(reference.color),
        width: 0.0,
        line_style: reference.line_style,
    }
}

fn shadow_endpoints(
    from: Option<ComputedShadowStyle>,
    to: Option<ComputedShadowStyle>,
) -> (ComputedShadowStyle, ComputedShadowStyle) {
    match (from, to) {
        (Some(from), Some(to)) => (from, to),
        (None, Some(to)) => (neutral_shadow(to), to),
        (Some(from), None) => (from, neutral_shadow(from)),
        (None, None) => unreachable!("equal shadows do not create transition endpoints"),
    }
}

fn neutral_shadow(reference: ComputedShadowStyle) -> ComputedShadowStyle {
    ComputedShadowStyle {
        color: Color {
            a: 0.0,
            ..reference.color
        },
        ..reference
    }
}

fn transparent_color_style(value: ComputedColorStyle) -> ComputedColorStyle {
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

fn interpolate_rgba(from: Color, to: Color, progress: f32) -> Color {
    Color {
        r: lerp(from.r, to.r, progress),
        g: lerp(from.g, to.g, progress),
        b: lerp(from.b, to.b, progress),
        a: lerp(from.a, to.a, progress),
    }
}

fn interpolate_point(from: Point, to: Point, progress: f32) -> Point {
    Point::new(lerp(from.x, to.x, progress), lerp(from.y, to.y, progress))
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress.clamp(0.0, 1.0)
}

fn discrete<T: Copy>(from: T, to: T, progress: f32) -> T {
    if progress >= 1.0 { to } else { from }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{StrokeLineStyle, Theme};

    fn assert_near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn diff_only_contains_paint_properties() {
        let theme = Theme::default();
        let from = ComputedStyle::initial(&theme);
        let mut to = from.clone();
        to.text.color = Color::WHITE;
        to.text.font_size = 48.0;

        let (_, to_anim) = AnimableStyle::diff(&from, &to);
        assert!(to_anim.is_empty());
    }

    #[test]
    fn paint_diff_samples_background_and_radius() {
        let theme = Theme::default();
        let mut from = ComputedStyle::initial(&theme);
        from.paint.background = ComputedColorStyle::Solid(Color::BLACK);
        let mut to = from.clone();
        to.paint.background = ComputedColorStyle::Solid(Color::WHITE);
        to.paint.border_radius = 10.0;

        let (from_anim, to_anim) = AnimableStyle::diff(&from, &to);
        let sampled = AnimableStyle::interpolate(&from_anim, &to_anim, 0.5);
        let mut effective = to.clone();
        sampled.apply_to_computed(&mut effective);

        let ComputedColorStyle::Solid(color) = effective.paint.background else {
            panic!("expected solid background")
        };
        assert_near(color.r, 0.5);
        assert_near(effective.paint.border_radius, 5.0);
    }

    #[test]
    fn optional_stroke_and_shadow_use_neutral_endpoints() {
        let theme = Theme::default();
        let from = ComputedStyle::initial(&theme);
        let mut to = from.clone();
        to.paint.stroke = Some(ComputedStrokeStyle {
            color: ComputedColorStyle::Solid(Color::WHITE),
            width: 4.0,
            line_style: StrokeLineStyle::Solid,
        });
        to.paint.shadow = Some(ComputedShadowStyle {
            color: Color::BLACK,
            offset: Point::new(2.0, 4.0),
            blur: 8.0,
            spread: 2.0,
        });

        let (from_anim, to_anim) = AnimableStyle::diff(&from, &to);
        let sampled = AnimableStyle::interpolate(&from_anim, &to_anim, 0.5);
        assert_near(sampled.paint.stroke.unwrap().width, 2.0);
        assert_near(sampled.paint.shadow.unwrap().color.a, 0.5);
    }
}
