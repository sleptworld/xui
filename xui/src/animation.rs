use std::{
    hash::{Hash, Hasher},
    time::Duration,
};
use xui_animation::{Animatable, Timeline};
use xui_interface::{
    Color, ColorStyle, ColorValue, ComputedColorStyle, ComputedLinearGradientStyle,
    ComputedRadialGradientStyle, ComputedStrokeStyle, ComputedStyle, EventTrigger,
    LinearGradientStyle, Point, RadialGradientStyle, StrokeLineStyle, Style, Theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnimationEasing {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    QuartIn,
    QuartOut,
    QuartInOut,
    QuintIn,
    QuintOut,
    QuintInOut,
    SineIn,
    SineOut,
    SineInOut,
}

impl From<AnimationEasing> for xui_animation::Easing {
    fn from(value: AnimationEasing) -> Self {
        match value {
            AnimationEasing::Linear => Self::Linear,
            AnimationEasing::QuadIn => Self::QuadIn,
            AnimationEasing::QuadOut => Self::QuadOut,
            AnimationEasing::QuadInOut => Self::QuadInOut,
            AnimationEasing::CubicIn => Self::CubicIn,
            AnimationEasing::CubicOut => Self::CubicOut,
            AnimationEasing::CubicInOut => Self::CubicInOut,
            AnimationEasing::QuartIn => Self::QuartIn,
            AnimationEasing::QuartOut => Self::QuartOut,
            AnimationEasing::QuartInOut => Self::QuartInOut,
            AnimationEasing::QuintIn => Self::QuintIn,
            AnimationEasing::QuintOut => Self::QuintOut,
            AnimationEasing::QuintInOut => Self::QuintInOut,
            AnimationEasing::SineIn => Self::SineIn,
            AnimationEasing::SineOut => Self::SineOut,
            AnimationEasing::SineInOut => Self::SineInOut,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationTransition {
    pub duration: Duration,
    pub delay: Duration,
    pub easing: AnimationEasing,
}

impl AnimationTransition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: AnimationEasing::default(),
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn ease(mut self, easing: AnimationEasing) -> Self {
        self.easing = easing;
        self
    }
}

impl Default for AnimationTransition {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

impl From<AnimationTransition> for xui_animation::Transition {
    fn from(value: AnimationTransition) -> Self {
        Self {
            duration: value.duration,
            delay: value.delay,
            easing: value.easing.into(),
        }
    }
}

pub fn default_style_transition() -> AnimationTransition {
    AnimationTransition::new(Duration::from_millis(120)).ease(AnimationEasing::CubicOut)
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct StyleAnimation {
    pub trigger: EventTrigger,
    pub style: AnimableStyle,
    pub transition: AnimationTransition,
}

impl StyleAnimation {
    pub fn new(
        trigger: EventTrigger,
        style: AnimableStyle,
        transition: AnimationTransition,
    ) -> Self {
        Self {
            trigger,
            style,
            transition,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Hash)]
pub struct AnimatedStyle {
    pub base: Style,
    pub animations: Vec<StyleAnimation>,
}

impl AnimatedStyle {
    pub fn new(base: Style) -> Self {
        Self {
            base,
            animations: Vec::new(),
        }
    }

    pub fn animation(
        mut self,
        trigger: EventTrigger,
        style: AnimableStyle,
        transition: AnimationTransition,
    ) -> Self {
        self.animations
            .push(StyleAnimation::new(trigger, style, transition));
        self
    }

    pub fn on_hover(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHover, style, transition)
    }

    pub fn on_hover_start(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHoverStart, style, transition)
    }

    pub fn on_hover_end(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnHoverEnd, style, transition)
    }

    pub fn on_press(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnPress, style, transition)
    }

    pub fn on_focus(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnFocus, style, transition)
    }

    pub fn on_click(self, style: AnimableStyle, transition: AnimationTransition) -> Self {
        self.animation(EventTrigger::OnClick, style, transition)
    }
}

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct StyleAnimationRule {
    pub trigger: EventTrigger,
    pub style: Option<AnimableStyle>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.text.color = Some(ColorStyle::solid(color));
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.text.font_size = Some(font_size);
        self
    }

    pub fn background(mut self, background: impl Into<ColorStyle>) -> Self {
        self.paint.background = Some(background.into());
        self
    }

    pub fn border_color(mut self, color: impl Into<ColorStyle>) -> Self {
        self.paint.border_color = Some(color.into());
        self
    }

    pub fn border_width(mut self, width: f32) -> Self {
        self.paint.border_width = Some(width);
        self
    }

    pub fn border_radius(mut self, radius: f32) -> Self {
        self.paint.border_radius = Some(radius);
        self
    }

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

impl Hash for AnimableStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.paint.hash(state);
    }
}

impl Hash for AnimableTextStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        hash_f32_option(self.font_size, state);
    }
}

impl Hash for AnimablePaintStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.background.hash(state);
        self.border_color.hash(state);
        hash_f32_option(self.border_width, state);
        hash_f32_option(self.border_radius, state);
    }
}

impl Hash for AnimableShadowStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_color_option(self.color, state);
        match self.offset {
            Some(point) => {
                true.hash(state);
                point.x.to_bits().hash(state);
                point.y.to_bits().hash(state);
            }
            None => false.hash(state),
        }
        hash_f32_option(self.blur, state);
        hash_f32_option(self.spread, state);
    }
}

impl Hash for AnimableStrokeStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        hash_f32_option(self.width, state);
    }
}

fn hash_f32_option<H: Hasher>(value: Option<f32>, state: &mut H) {
    match value {
        Some(value) => {
            true.hash(state);
            value.to_bits().hash(state);
        }
        None => false.hash(state),
    }
}

fn hash_color_option<H: Hasher>(value: Option<Color>, state: &mut H) {
    match value {
        Some(color) => {
            true.hash(state);
            color.r.to_bits().hash(state);
            color.g.to_bits().hash(state);
            color.b.to_bits().hash(state);
            color.a.to_bits().hash(state);
        }
        None => false.hash(state),
    }
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

    #[test]
    fn animated_style_records_event_triggered_animable_style_animation() {
        let transition = AnimationTransition::new(Duration::from_millis(120))
            .delay(Duration::from_millis(20))
            .ease(AnimationEasing::CubicOut);
        let animated = AnimatedStyle::new(Style::new().background(Color::BLACK))
            .on_hover(AnimableStyle::new().background(Color::WHITE), transition);

        assert_eq!(
            animated.base.paint.background,
            xui_interface::StyleValue::Value(ColorStyle::Solid(ColorValue::Color(Color::BLACK)))
        );
        assert_eq!(animated.animations.len(), 1);
        assert_eq!(animated.animations[0].trigger, EventTrigger::OnHover);
        assert_eq!(animated.animations[0].transition, transition);
        assert_eq!(
            animated.animations[0].style.paint.background,
            Some(ColorStyle::Solid(ColorValue::Color(Color::WHITE)))
        );
    }
}
