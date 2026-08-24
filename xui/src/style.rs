use std::time::Duration;

pub use xui_interface::{
    AlignStyle, BackdropFilter, BackdropMask, BackdropStyle, BlendMode, ColorMatrix, ColorStyle,
    ColorToken, ColorValue, ComputedBackdropFilter, ComputedBackdropMask, ComputedBackdropStyle,
    ComputedColorStyle, ComputedEffect, ComputedEffectStyle, ComputedLayoutStyle,
    ComputedLinearGradientStyle, ComputedMaskShape, ComputedPaintStyle,
    ComputedRadialGradientStyle, ComputedScrollStyle, ComputedScrollbarStyle, ComputedShadowStyle,
    ComputedStrokeStyle, ComputedStyle, ComputedTextStyle, Effect, EffectStylePatch, FilterQuality,
    FlexDirectionStyle, FontSizeToken, JustifyStyle, LayoutStylePatch, LengthValue,
    LinearGradientStyle, MaskShape, PaintStylePatch, PositionStyle, RadialGradientStyle,
    RadiusToken, ScrollDirectionStyle, ScrollStylePatch, ScrollbarStyle, ScrollbarStylePatch,
    ScrollbarVisibilityStyle, ShadowStyle, SpacingToken, StateStyleRule, Stroke, StrokeLineStyle,
    StrokeStyle, Style, StyleMerge, StylePatch, StyleValue, TextStylePatch, Theme, TransformStyle,
    TransformStylePatch, WidgetState, WidgetStateMatcher,
};

use crate::animation::{has_animatable_difference, interpolate_style};
use xui_animation::*;

pub struct XStyle {
    pub target: ComputedStyle,
    transition: Option<Transition>,
    handler: Option<TransitionStyleHandler>,
}

impl XStyle {
    pub fn new(style: ComputedStyle) -> Self {
        Self {
            target: style,
            transition: None,
            handler: None,
        }
    }

    pub fn with_transition(mut self, transition: Option<Transition>) -> Self {
        self.transition = transition;
        self
    }

    pub fn update_style(&mut self, next: ComputedStyle) -> bool {
        if let Some(transition) = self.transition {
            let current = if let Some(handler) = self.handler.as_ref() {
                &handler.current_style
            } else {
                &self.target
            };

            if has_animatable_difference(current, &next) {
                self.handler = Some(TransitionStyleHandler::new(
                    Timeline::new(transition),
                    current.clone(),
                    next.clone(),
                ));
                self.target = next;
                true
            } else {
                self.handler = None;
                self.target = next;
                false
            }
        } else {
            self.target = next;
            self.handler = None;
            false
        }
    }

    pub fn style(&mut self, delta: Duration, theme: &Theme) -> &ComputedStyle {
        if let Some(handler) = self.handler.as_mut() {
            handler.current(delta, theme)
        } else {
            &self.target
        }
    }

    pub fn target_style(&self) -> &ComputedStyle {
        &self.target
    }
}

struct TransitionStyleHandler {
    timeline: Timeline,
    from: ComputedStyle,
    target: ComputedStyle,
    current_style: ComputedStyle,
}

impl TransitionStyleHandler {
    fn new(timeline: Timeline, from: ComputedStyle, target: ComputedStyle) -> Self {
        let current_style = interpolate_style(&from, &target, 0.0);
        Self {
            from,
            target,
            timeline,
            current_style,
        }
    }

    fn current(&mut self, delta: Duration, _theme: &Theme) -> &ComputedStyle {
        let progress = self.timeline.tick(delta);
        self.current_style = interpolate_style(&self.from, &self.target, progress.eased);
        &self.current_style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use xui_interface::Color;

    #[test]
    fn effect_chain_and_paint_properties_transition_together() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let next = ComputedStyle::compute(
            &initial,
            &StylePatch::new()
                .background(Color::BLACK)
                .effects(Arc::from([Effect::blur(6.0)])),
            &theme,
        );
        let mut style = XStyle::new(initial.clone())
            .with_transition(Some(Transition::new(Duration::from_millis(100))));

        assert!(style.update_style(next.clone()));
        let current = style.style(Duration::ZERO, &theme);
        assert_ne!(current.effect.effects, next.effect.effects);
        assert_eq!(current.paint.background, initial.paint.background);
    }

    #[test]
    fn compatible_effect_change_creates_an_animation_handler() {
        let theme = Theme::default();
        let initial = ComputedStyle::initial(&theme);
        let next = ComputedStyle::compute(
            &initial,
            &StylePatch::new().effects(Arc::from([Effect::blur(6.0)])),
            &theme,
        );
        let mut style =
            XStyle::new(initial).with_transition(Some(Transition::new(Duration::from_millis(100))));

        assert!(style.update_style(next.clone()));
        assert_ne!(style.style(Duration::ZERO, &theme), &next);
    }
}
