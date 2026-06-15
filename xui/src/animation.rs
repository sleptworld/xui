use std::time::Duration;
use xui_animation::{Animatable, Timeline};
use xui_interface::{
    AnimationEasing, AnimationTransition, Color, ColorStyle, ColorValue, ComputedColorStyle,
    ComputedLinearGradientStyle, ComputedRadialGradientStyle, ComputedStrokeStyle, ComputedStyle,
    EventTrigger, LinearGradientStyle, Point, RadialGradientStyle, StrokeLineStyle, Style,
    StyleAnimation, Theme,
};

pub fn default_style_transition() -> AnimationTransition {
    AnimationTransition::new(Duration::from_millis(120)).ease(AnimationEasing::CubicOut)
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct StyleAnimationRule {
    pub trigger: EventTrigger,
    pub style: Option<Style>,
    pub transition: AnimationTransition,
}

impl StyleAnimationRule {
    pub fn new(trigger: EventTrigger, transition: AnimationTransition) -> Self {
        Self {
            trigger,
            style: None,
            transition,
        }
    }

    pub fn from_style_animation(animation: &StyleAnimation) -> Self {
        Self {
            trigger: animation.trigger,
            style: Some(animation.style.clone()),
            transition: animation.transition,
        }
    }

    pub fn reverse(trigger: EventTrigger, transition: AnimationTransition) -> Self {
        Self::new(trigger, transition)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActiveAnimation<A: Animatable> {
    pub trigger: EventTrigger,
    pub from_style: A,
    pub to_style: A,
    pub timeline: Timeline,
    completed: bool,
}

impl<A: Animatable> ActiveAnimation<A> {
    pub fn new(
        trigger: EventTrigger,
        from_style: A,
        to_style: A,
        transition: AnimationTransition,
    ) -> Self {
        Self {
            trigger,
            from_style,
            to_style,
            timeline: Timeline::new(transition.into()),
            completed: false,
        }
    }

    pub fn sample(&self) -> A {
        let progress = self.timeline.progress().eased;
        A::interpolate(&self.from_style, &self.to_style, progress)
    }

    pub fn tick(&mut self, delta: Duration) -> bool {
        if self.completed {
            return false;
        }

        let progress = self.timeline.tick(delta);
        self.completed = progress.completed;
        true
    }

    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn is_running(&self) -> bool {
        !self.completed
    }
}

#[derive(Debug, Clone, PartialEq, Animatable, Default)]
pub struct AnimableStyle {
    pub text: AnimableTextStyle,
    pub paint: AnimablePaintStyle,
}

impl AnimableStyle {
    pub fn is_empty(&self) -> bool {
        self.text.color.is_none()
            && self.text.font_size.is_none()
            && self.paint.background.is_none()
            && self.paint.border_color.is_none()
            && self.paint.border_width.is_none()
            && self.paint.border_radius.is_none()
    }

    pub fn merge(&mut self, other: &Self) {
        if other.text.color.is_some() {
            self.text.color = other.text.color;
        }
        if other.text.font_size.is_some() {
            self.text.font_size = other.text.font_size;
        }
        if other.paint.background.is_some() {
            self.paint.background = other.paint.background;
        }
        if other.paint.border_color.is_some() {
            self.paint.border_color = other.paint.border_color;
        }
        if other.paint.border_width.is_some() {
            self.paint.border_width = other.paint.border_width;
        }
        if other.paint.border_radius.is_some() {
            self.paint.border_radius = other.paint.border_radius;
        }
    }

    pub fn from_computed(style: &ComputedStyle) -> Self {
        Self {
            text: AnimableTextStyle {
                color: Some(ColorStyle::solid(style.text.color)),
                font_size: Some(style.text.font_size),
            },
            paint: AnimablePaintStyle {
                background: Some(color_style_from_computed(style.paint.background)),
                border_color: Some(color_style_from_computed(style.paint.border_color)),
                border_width: Some(style.paint.border_width),
                border_radius: Some(style.paint.border_radius),
            },
        }
    }

    pub fn diff(from: &ComputedStyle, to: &ComputedStyle) -> (Self, Self) {
        let mut from_anim = Self::default();
        let mut to_anim = Self::default();

        if from.text.color != to.text.color {
            from_anim.text.color = Some(ColorStyle::solid(from.text.color));
            to_anim.text.color = Some(ColorStyle::solid(to.text.color));
        }
        if from.text.font_size != to.text.font_size {
            from_anim.text.font_size = Some(from.text.font_size);
            to_anim.text.font_size = Some(to.text.font_size);
        }
        if from.paint.background != to.paint.background {
            from_anim.paint.background = Some(color_style_from_computed(from.paint.background));
            to_anim.paint.background = Some(color_style_from_computed(to.paint.background));
        }
        if from.paint.border_color != to.paint.border_color {
            from_anim.paint.border_color = Some(color_style_from_computed(from.paint.border_color));
            to_anim.paint.border_color = Some(color_style_from_computed(to.paint.border_color));
        }
        if from.paint.border_width != to.paint.border_width {
            from_anim.paint.border_width = Some(from.paint.border_width);
            to_anim.paint.border_width = Some(to.paint.border_width);
        }
        if from.paint.border_radius != to.paint.border_radius {
            from_anim.paint.border_radius = Some(from.paint.border_radius);
            to_anim.paint.border_radius = Some(to.paint.border_radius);
        }

        (from_anim, to_anim)
    }

    pub fn apply_to_computed(&self, style: &mut ComputedStyle, theme: &Theme) {
        if let Some(color) = self.text.color {
            if let ComputedColorStyle::Solid(color) = resolve_color_style(color, theme) {
                style.text.color = color;
            }
        }
        if let Some(font_size) = self.text.font_size {
            style.text.font_size = font_size;
        }
        if let Some(background) = self.paint.background {
            style.paint.background = resolve_color_style(background, theme);
        }
        let mut border_changed = false;
        if let Some(border_color) = self.paint.border_color {
            style.paint.border_color = resolve_color_style(border_color, theme);
            border_changed = true;
        }
        if let Some(border_width) = self.paint.border_width {
            style.paint.border_width = border_width;
            border_changed = true;
        }
        if let Some(border_radius) = self.paint.border_radius {
            style.paint.border_radius = border_radius;
        }
        if border_changed {
            sync_border_stroke(style);
        }
    }
}

#[derive(Debug, Animatable, Clone, Copy, Default, PartialEq)]
pub struct AnimableTextStyle {
    pub color: Option<ColorStyle>,
    pub font_size: Option<f32>,
}

#[derive(Debug, Animatable, Clone, Copy, Default, PartialEq)]
pub struct AnimablePaintStyle {
    pub background: Option<ColorStyle>,
    pub border_color: Option<ColorStyle>,
    pub border_width: Option<f32>,
    pub border_radius: Option<f32>,
}

#[derive(Debug, Animatable, Clone, Copy, Default, PartialEq)]
pub struct AnimableShadowStyle {
    pub color: Option<Color>,
    pub offset: Option<Point>,
    pub blur: Option<f32>,
    pub spread: Option<f32>,
}

#[derive(Debug, Animatable, Clone, Copy, Default, PartialEq)]
pub struct AnimableStrokeStyle {
    pub color: Option<ColorStyle>,
    pub width: Option<f32>,
}

fn color_style_from_computed(value: ComputedColorStyle) -> ColorStyle {
    match value {
        ComputedColorStyle::Solid(color) => ColorStyle::solid(color),
        ComputedColorStyle::LinearGradient(gradient) => {
            ColorStyle::LinearGradient(LinearGradientStyle {
                start: gradient.start,
                end: gradient.end,
                from: ColorValue::Color(gradient.from),
                to: ColorValue::Color(gradient.to),
            })
        }
        ComputedColorStyle::RadialGradient(gradient) => {
            ColorStyle::RadialGradient(RadialGradientStyle {
                center: gradient.center,
                radius: gradient.radius.into(),
                from: ColorValue::Color(gradient.from),
                to: ColorValue::Color(gradient.to),
            })
        }
    }
}

fn resolve_color_style(value: ColorStyle, theme: &Theme) -> ComputedColorStyle {
    match value {
        ColorStyle::Solid(value) => ComputedColorStyle::Solid(resolve_color_value(value, theme)),
        ColorStyle::LinearGradient(gradient) => {
            ComputedColorStyle::LinearGradient(ComputedLinearGradientStyle {
                start: gradient.start,
                end: gradient.end,
                from: resolve_color_value(gradient.from, theme),
                to: resolve_color_value(gradient.to, theme),
            })
        }
        ColorStyle::RadialGradient(gradient) => {
            ComputedColorStyle::RadialGradient(ComputedRadialGradientStyle {
                center: gradient.center,
                radius: match gradient.radius {
                    xui_interface::LengthValue::Px(value) => value,
                    xui_interface::LengthValue::Spacing(token) => theme.spacing(token),
                    xui_interface::LengthValue::Radius(token) => theme.radius(token),
                    xui_interface::LengthValue::FontSize(token) => theme.font_size(token),
                },
                from: resolve_color_value(gradient.from, theme),
                to: resolve_color_value(gradient.to, theme),
            })
        }
    }
}

fn resolve_color_value(value: ColorValue, theme: &Theme) -> Color {
    match value {
        ColorValue::Color(color) => color,
        ColorValue::Token(token) => theme.color(token),
    }
}

fn sync_border_stroke(style: &mut ComputedStyle) {
    if style.paint.border_width > 0.0 && style.paint.border_color.is_visible() {
        let line_style = style
            .paint
            .stroke
            .map(|stroke| stroke.line_style)
            .unwrap_or(StrokeLineStyle::Solid);
        style.paint.stroke = Some(ComputedStrokeStyle {
            color: style.paint.border_color,
            width: style.paint.border_width,
            line_style,
        });
    } else {
        style.paint.stroke = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
    }

    #[test]
    fn animable_style_diff_samples_and_applies_to_computed_style() {
        let theme = Theme::default();
        let mut from = ComputedStyle::initial(&theme);
        from.paint.background = ComputedColorStyle::Solid(Color::BLACK);
        from.paint.border_radius = 0.0;

        let mut to = from.clone();
        to.paint.background = ComputedColorStyle::Solid(Color::WHITE);
        to.paint.border_radius = 10.0;

        let (from_anim, to_anim) = AnimableStyle::diff(&from, &to);
        assert!(!to_anim.is_empty());

        let sampled = AnimableStyle::interpolate(&from_anim, &to_anim, 0.5);
        let mut effective = to.clone();
        sampled.apply_to_computed(&mut effective, &theme);

        let ComputedColorStyle::Solid(color) = effective.paint.background else {
            panic!("expected solid background");
        };
        assert_near(color.r, 0.5);
        assert_near(color.g, 0.5);
        assert_near(color.b, 0.5);
        assert_near(effective.paint.border_radius, 5.0);
    }
}
