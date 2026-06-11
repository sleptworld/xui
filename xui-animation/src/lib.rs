use std::time::Duration;

use easer::functions::{Cubic, Easing as EaserFunction, Linear, Quad, Quart, Quint, Sine};
use palette::{LinSrgba, Mix};
use xui_interface::{
    AnimationEasing, AnimationTransition, Color, ColorStyle, ColorValue, EdgeInsets, LengthValue,
    LinearGradientStyle, Point, RadialGradientStyle, ScrollbarStyle, ShadowStyle, Size, Sizing,
    StrokeStyle, Style, StyleValue,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
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

impl Default for Easing {
    fn default() -> Self {
        Self::Linear
    }
}

impl Easing {
    pub fn sample(self, progress: f32) -> f32 {
        let progress = clamp_unit(progress);
        let eased = match self {
            Self::Linear => Linear::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuadIn => Quad::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuadOut => Quad::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuadInOut => Quad::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::CubicIn => Cubic::ease_in(progress, 0.0, 1.0, 1.0),
            Self::CubicOut => Cubic::ease_out(progress, 0.0, 1.0, 1.0),
            Self::CubicInOut => Cubic::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::QuartIn => Quart::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuartOut => Quart::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuartInOut => Quart::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::QuintIn => Quint::ease_in(progress, 0.0, 1.0, 1.0),
            Self::QuintOut => Quint::ease_out(progress, 0.0, 1.0, 1.0),
            Self::QuintInOut => Quint::ease_in_out(progress, 0.0, 1.0, 1.0),
            Self::SineIn => Sine::ease_in(progress, 0.0, 1.0, 1.0),
            Self::SineOut => Sine::ease_out(progress, 0.0, 1.0, 1.0),
            Self::SineInOut => Sine::ease_in_out(progress, 0.0, 1.0, 1.0),
        };
        clamp_unit(eased)
    }
}

impl From<AnimationEasing> for Easing {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transition {
    pub duration: Duration,
    pub delay: Duration,
    pub easing: Easing,
}

impl Transition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            delay: Duration::ZERO,
            easing: Easing::default(),
        }
    }

    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    pub fn progress_at(self, elapsed: Duration) -> AnimationProgress {
        let raw = if elapsed <= self.delay {
            0.0
        } else if self.duration.is_zero() {
            1.0
        } else {
            elapsed.saturating_sub(self.delay).as_secs_f32() / self.duration.as_secs_f32()
        };
        let raw = clamp_unit(raw);
        AnimationProgress {
            raw,
            eased: self.easing.sample(raw),
            completed: elapsed >= self.delay.saturating_add(self.duration),
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}

impl From<AnimationTransition> for Transition {
    fn from(value: AnimationTransition) -> Self {
        Self {
            duration: value.duration,
            delay: value.delay,
            easing: value.easing.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationProgress {
    pub raw: f32,
    pub eased: f32,
    pub completed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationClock {
    elapsed: Duration,
}

impl AnimationClock {
    pub fn new() -> Self {
        Self {
            elapsed: Duration::ZERO,
        }
    }

    pub fn elapsed(self) -> Duration {
        self.elapsed
    }

    pub fn set_elapsed(&mut self, elapsed: Duration) {
        self.elapsed = elapsed;
    }

    pub fn tick(&mut self, delta: Duration) -> Duration {
        self.elapsed = self.elapsed.saturating_add(delta);
        self.elapsed
    }

    pub fn reset(&mut self) {
        self.elapsed = Duration::ZERO;
    }
}

impl Default for AnimationClock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timeline {
    transition: Transition,
    clock: AnimationClock,
}

impl Timeline {
    pub fn new(transition: Transition) -> Self {
        Self {
            transition,
            clock: AnimationClock::new(),
        }
    }

    pub fn transition(self) -> Transition {
        self.transition
    }

    pub fn elapsed(self) -> Duration {
        self.clock.elapsed()
    }

    pub fn progress(self) -> AnimationProgress {
        self.transition.progress_at(self.clock.elapsed())
    }

    pub fn progress_at(self, elapsed: Duration) -> AnimationProgress {
        self.transition.progress_at(elapsed)
    }

    pub fn tick(&mut self, delta: Duration) -> AnimationProgress {
        self.clock.tick(delta);
        self.progress()
    }

    pub fn set_elapsed(&mut self, elapsed: Duration) -> AnimationProgress {
        self.clock.set_elapsed(elapsed);
        self.progress()
    }

    pub fn reset(&mut self) {
        self.clock.reset();
    }
}

pub trait Animatable: Sized {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tween<T> {
    pub from: T,
    pub to: T,
    pub transition: Transition,
}

impl<T> Tween<T> {
    pub fn new(from: T, to: T, transition: Transition) -> Self {
        Self {
            from,
            to,
            transition,
        }
    }
}

impl<T: Animatable> Tween<T> {
    pub fn sample(&self, elapsed: Duration) -> T {
        let progress = self.transition.progress_at(elapsed).eased;
        T::interpolate(&self.from, &self.to, progress)
    }

    pub fn sample_progress(&self, progress: f32) -> T {
        T::interpolate(
            &self.from,
            &self.to,
            self.transition.easing.sample(progress),
        )
    }
}

pub type PropertyAnimation<T> = Tween<T>;

#[derive(Debug, Clone, PartialEq)]
pub struct AnimatedStyle {
    tween: Tween<Style>,
}

impl AnimatedStyle {
    pub fn new(from: Style, to: Style, transition: Transition) -> Self {
        Self {
            tween: Tween::new(from, to, transition),
        }
    }

    pub fn sample(&self, elapsed: Duration) -> Style {
        self.tween.sample(elapsed)
    }

    pub fn sample_progress(&self, progress: f32) -> Style {
        self.tween.sample_progress(progress)
    }

    pub fn transition(&self) -> Transition {
        self.tween.transition
    }
}

impl Animatable for f32 {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        lerp_f32(*from, *to, progress)
    }
}

impl Animatable for Point {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self::new(
            f32::interpolate(&from.x, &to.x, progress),
            f32::interpolate(&from.y, &to.y, progress),
        )
    }
}

impl Animatable for Size<f32> {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self::new(
            f32::interpolate(&from.width, &to.width, progress),
            f32::interpolate(&from.height, &to.height, progress),
        )
    }
}

impl Animatable for EdgeInsets {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            left: f32::interpolate(&from.left, &to.left, progress),
            right: f32::interpolate(&from.right, &to.right, progress),
            top: f32::interpolate(&from.top, &to.top, progress),
            bottom: f32::interpolate(&from.bottom, &to.bottom, progress),
        }
    }
}

impl Animatable for Color {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        let from = LinSrgba::new(from.r, from.g, from.b, from.a);
        let to = LinSrgba::new(to.r, to.g, to.b, to.a);
        let mixed = from.mix(to, clamp_unit(progress));
        Color::rgba(mixed.red, mixed.green, mixed.blue, mixed.alpha)
    }
}

impl Animatable for LengthValue {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        match (*from, *to) {
            (Self::Px(from), Self::Px(to)) => Self::Px(f32::interpolate(&from, &to, progress)),
            _ => discrete(from, to, progress),
        }
    }
}

impl Animatable for Size<Sizing> {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Size::<Sizing>::new(
            interpolate_sizing(from.width, to.width, progress),
            interpolate_sizing(from.height, to.height, progress),
        )
    }
}

impl Animatable for ColorValue {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        match (*from, *to) {
            (Self::Color(from), Self::Color(to)) => {
                Self::Color(Color::interpolate(&from, &to, progress))
            }
            _ => discrete(from, to, progress),
        }
    }
}

impl Animatable for ColorStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        match (*from, *to) {
            (Self::Solid(from), Self::Solid(to)) => {
                Self::Solid(ColorValue::interpolate(&from, &to, progress))
            }
            (Self::LinearGradient(from), Self::LinearGradient(to)) => {
                Self::LinearGradient(LinearGradientStyle::interpolate(&from, &to, progress))
            }
            (Self::RadialGradient(from), Self::RadialGradient(to)) => {
                Self::RadialGradient(RadialGradientStyle::interpolate(&from, &to, progress))
            }
            _ => discrete(from, to, progress),
        }
    }
}

impl Animatable for LinearGradientStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            start: Point::interpolate(&from.start, &to.start, progress),
            end: Point::interpolate(&from.end, &to.end, progress),
            from: ColorValue::interpolate(&from.from, &to.from, progress),
            to: ColorValue::interpolate(&from.to, &to.to, progress),
        }
    }
}

impl Animatable for RadialGradientStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            center: Point::interpolate(&from.center, &to.center, progress),
            radius: LengthValue::interpolate(&from.radius, &to.radius, progress),
            from: ColorValue::interpolate(&from.from, &to.from, progress),
            to: ColorValue::interpolate(&from.to, &to.to, progress),
        }
    }
}

impl Animatable for ShadowStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            color: ColorValue::interpolate(&from.color, &to.color, progress),
            offset: Point::interpolate(&from.offset, &to.offset, progress),
            blur: LengthValue::interpolate(&from.blur, &to.blur, progress),
            spread: LengthValue::interpolate(&from.spread, &to.spread, progress),
        }
    }
}

impl Animatable for StrokeStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            color: ColorStyle::interpolate(&from.color, &to.color, progress),
            width: LengthValue::interpolate(&from.width, &to.width, progress),
            line_style: discrete(&from.line_style, &to.line_style, progress),
        }
    }
}

impl Animatable for ScrollbarStyle {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        Self {
            width: LengthValue::interpolate(&from.width, &to.width, progress),
            track_color: ColorStyle::interpolate(&from.track_color, &to.track_color, progress),
            thumb_color: ColorStyle::interpolate(&from.thumb_color, &to.thumb_color, progress),
            radius: LengthValue::interpolate(&from.radius, &to.radius, progress),
            visibility: discrete(&from.visibility, &to.visibility, progress),
        }
    }
}

impl Animatable for Style {
    fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
        let mut style = Style::new();

        style.text.color = animate_target_value(&from.text.color, &to.text.color, progress);
        style.text.font_family = to.text.font_family.clone();
        style.text.font_size =
            animate_target_value(&from.text.font_size, &to.text.font_size, progress);
        style.text.font_weight = to.text.font_weight;
        style.text.font_style = to.text.font_style;
        style.text.line_height = to.text.line_height;
        style.text.letter_spacing =
            animate_target_value(&from.text.letter_spacing, &to.text.letter_spacing, progress);
        style.text.decoration = to.text.decoration;

        style.layout.gap = animate_target_value(&from.layout.gap, &to.layout.gap, progress);
        style.layout.size = animate_target_value(&from.layout.size, &to.layout.size, progress);
        style.layout.min_size =
            animate_target_value(&from.layout.min_size, &to.layout.min_size, progress);
        style.layout.max_size =
            animate_target_value(&from.layout.max_size, &to.layout.max_size, progress);
        style.layout.margin =
            animate_target_value(&from.layout.margin, &to.layout.margin, progress);
        style.layout.padding =
            animate_target_value(&from.layout.padding, &to.layout.padding, progress);
        style.layout.align = to.layout.align;
        style.layout.justify = to.layout.justify;

        style.paint.background =
            animate_target_value(&from.paint.background, &to.paint.background, progress);
        style.paint.border_color =
            animate_target_value(&from.paint.border_color, &to.paint.border_color, progress);
        style.paint.border_width =
            animate_target_value(&from.paint.border_width, &to.paint.border_width, progress);
        style.paint.border_radius =
            animate_target_value(&from.paint.border_radius, &to.paint.border_radius, progress);
        style.paint.stroke = animate_target_value(&from.paint.stroke, &to.paint.stroke, progress);
        style.paint.shadow = animate_target_value(&from.paint.shadow, &to.paint.shadow, progress);
        style.paint.clip = to.paint.clip;

        style.scroll.direction = to.scroll.direction;
        style.scroll.scrollbar =
            animate_target_value(&from.scroll.scrollbar, &to.scroll.scrollbar, progress);

        style
    }
}

fn animate_target_value<T>(from: &StyleValue<T>, to: &StyleValue<T>, progress: f32) -> StyleValue<T>
where
    T: Animatable + Clone,
{
    match to {
        StyleValue::Unset => StyleValue::Unset,
        StyleValue::Inherit => StyleValue::Inherit,
        StyleValue::Initial => StyleValue::Initial,
        StyleValue::Value(to_value) => match from {
            StyleValue::Value(from_value) => {
                StyleValue::Value(T::interpolate(from_value, to_value, progress))
            }
            _ => StyleValue::Value(to_value.clone()),
        },
    }
}

fn interpolate_sizing(from: Sizing, to: Sizing, progress: f32) -> Sizing {
    match (from, to) {
        (Sizing::Fix(from), Sizing::Fix(to)) => Sizing::fix(f32::interpolate(
            &from.into_inner(),
            &to.into_inner(),
            progress,
        )),
        (Sizing::Percent(from), Sizing::Percent(to)) => Sizing::percent(f32::interpolate(
            &from.into_inner(),
            &to.into_inner(),
            progress,
        )),
        _ => discrete(&from, &to, progress),
    }
}

fn discrete<T: Clone>(from: &T, to: &T, progress: f32) -> T {
    if clamp_unit(progress) >= 1.0 {
        to.clone()
    } else {
        from.clone()
    }
}

fn lerp_f32(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * clamp_unit(progress)
}

fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{
        ColorToken, FontSizeToken, ScrollbarVisibilityStyle, SpacingToken, StrokeLineStyle,
    };

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
    }

    #[test]
    fn easing_output_is_clamped() {
        assert_eq!(Easing::CubicOut.sample(-1.0), 0.0);
        assert_eq!(Easing::CubicOut.sample(2.0), 1.0);
    }

    #[test]
    fn transition_respects_delay_and_completion() {
        let transition = Transition::new(Duration::from_millis(100))
            .delay(Duration::from_millis(50))
            .ease(Easing::Linear);

        let before = transition.progress_at(Duration::from_millis(25));
        assert_eq!(before.raw, 0.0);
        assert_eq!(before.eased, 0.0);
        assert!(!before.completed);

        let middle = transition.progress_at(Duration::from_millis(100));
        assert_near(middle.raw, 0.5);
        assert_near(middle.eased, 0.5);
        assert!(!middle.completed);

        let after = transition.progress_at(Duration::from_millis(150));
        assert_eq!(after.raw, 1.0);
        assert_eq!(after.eased, 1.0);
        assert!(after.completed);
    }

    #[test]
    fn tween_samples_animatable_values() {
        let tween = Tween::new(
            0.0,
            10.0,
            Transition::new(Duration::from_millis(100)).ease(Easing::Linear),
        );

        assert_eq!(tween.sample(Duration::ZERO), 0.0);
        assert_eq!(tween.sample(Duration::from_millis(50)), 5.0);
        assert_eq!(tween.sample(Duration::from_millis(100)), 10.0);
    }

    #[test]
    fn geometry_and_color_interpolate() {
        let point = Point::interpolate(&Point::new(0.0, 10.0), &Point::new(10.0, 30.0), 0.5);
        assert_eq!(point, Point::new(5.0, 20.0));

        let insets = EdgeInsets::interpolate(&EdgeInsets::all(0.0), &EdgeInsets::all(8.0), 0.25);
        assert_eq!(insets, EdgeInsets::all(2.0));

        let color = Color::interpolate(
            &Color::rgba(0.0, 0.0, 0.0, 0.2),
            &Color::rgba(1.0, 1.0, 1.0, 0.8),
            0.5,
        );
        assert_near(color.r, 0.5);
        assert_near(color.g, 0.5);
        assert_near(color.b, 0.5);
        assert_near(color.a, 0.5);
    }

    #[test]
    fn shadow_stroke_and_scrollbar_interpolate() {
        let shadow = ShadowStyle::interpolate(
            &ShadowStyle::new()
                .color(Color::rgba(0.0, 0.0, 0.0, 0.0))
                .offset(Point::new(0.0, 2.0))
                .blur(2.0)
                .spread(0.0),
            &ShadowStyle::new()
                .color(Color::rgba(1.0, 1.0, 1.0, 1.0))
                .offset(Point::new(10.0, 12.0))
                .blur(6.0)
                .spread(4.0),
            0.5,
        );
        assert_eq!(shadow.offset, Point::new(5.0, 7.0));
        assert_eq!(shadow.blur, LengthValue::Px(4.0));
        assert_eq!(shadow.spread, LengthValue::Px(2.0));

        let stroke = StrokeStyle::interpolate(
            &StrokeStyle::new(Color::BLACK, 1.0).dashed(),
            &StrokeStyle::new(Color::WHITE, 5.0).dotted(),
            0.5,
        );
        assert_eq!(stroke.width, LengthValue::Px(3.0));
        assert_eq!(stroke.line_style, StrokeLineStyle::Dashed);

        let scrollbar = ScrollbarStyle::interpolate(
            &ScrollbarStyle::new().width(4.0),
            &ScrollbarStyle::new()
                .width(12.0)
                .visibility(ScrollbarVisibilityStyle::Hidden),
            1.0,
        );
        assert_eq!(scrollbar.width, LengthValue::Px(12.0));
        assert_eq!(scrollbar.visibility, ScrollbarVisibilityStyle::Hidden);
    }

    #[test]
    fn length_and_color_style_discrete_switch_for_different_variants() {
        let length = LengthValue::interpolate(
            &LengthValue::Spacing(SpacingToken::Sm),
            &LengthValue::FontSize(FontSizeToken::Lg),
            0.5,
        );
        assert_eq!(length, LengthValue::Spacing(SpacingToken::Sm));
        assert_eq!(
            LengthValue::interpolate(
                &LengthValue::Spacing(SpacingToken::Sm),
                &LengthValue::FontSize(FontSizeToken::Lg),
                1.0,
            ),
            LengthValue::FontSize(FontSizeToken::Lg)
        );

        let color_style = ColorStyle::interpolate(
            &ColorStyle::solid(ColorToken::Primary),
            &Color::WHITE.into(),
            0.5,
        );
        assert_eq!(color_style, ColorStyle::solid(ColorToken::Primary));
    }

    #[test]
    fn animated_style_only_outputs_targeted_properties() {
        let from = Style::new()
            .background(Color::BLACK)
            .border_radius(2.0)
            .padding(EdgeInsets::all(4.0));
        let to = Style::new()
            .background(Color::WHITE)
            .border_radius(10.0)
            .font_size(20.0);
        let animation = AnimatedStyle::new(
            from,
            to,
            Transition::new(Duration::from_millis(100)).ease(Easing::Linear),
        );

        let sampled = animation.sample(Duration::from_millis(50));
        assert_eq!(
            sampled.paint.border_radius,
            StyleValue::Value(LengthValue::Px(6.0))
        );
        assert_eq!(
            sampled.text.font_size,
            StyleValue::Value(LengthValue::Px(20.0))
        );
        assert_eq!(sampled.layout.padding, StyleValue::Unset);
        assert_eq!(sampled.paint.border_width, StyleValue::Unset);
    }

    #[test]
    fn timeline_ticks_by_delta() {
        let mut timeline =
            Timeline::new(Transition::new(Duration::from_millis(100)).ease(Easing::Linear));

        let first = timeline.tick(Duration::from_millis(25));
        assert_near(first.eased, 0.25);
        let second = timeline.tick(Duration::from_millis(25));
        assert_near(second.eased, 0.5);
        assert_eq!(timeline.elapsed(), Duration::from_millis(50));
    }
}
