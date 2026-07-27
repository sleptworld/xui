use std::time::Duration;

pub use xui_interface::{
    AlignStyle, ColorStyle, ColorToken, ColorValue, ComputedColorStyle, ComputedLayoutStyle,
    ComputedLinearGradientStyle, ComputedPaintStyle, ComputedRadialGradientStyle,
    ComputedScrollStyle, ComputedScrollbarStyle, ComputedShadowStyle, ComputedStrokeStyle,
    ComputedStyle, ComputedTextStyle, FlexDirectionStyle, FontSizeToken, JustifyStyle,
    LayoutStylePatch, LengthValue, LinearGradientStyle, PaintStylePatch, PositionStyle,
    RadialGradientStyle, RadiusToken, ScrollDirectionStyle, ScrollStylePatch, ScrollbarStyle,
    ScrollbarStylePatch, ScrollbarVisibilityStyle, ShadowStyle, SpacingToken, StateStyleRule,
    Stroke, StrokeLineStyle, StrokeStyle, Style, StyleMerge, StylePatch, StyleValue,
    TextStylePatch, Theme, WidgetState, WidgetStateMatcher,
};

use crate::animation::AnimableStyle;
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

            let (from, to) = AnimableStyle::diff(current, &next);
            if to.has_properties() {
                self.handler = Some(TransitionStyleHandler::new(
                    Timeline::new(transition),
                    from,
                    to,
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
    from: AnimableStyle,
    to: AnimableStyle,
    current_style: ComputedStyle,
}

impl TransitionStyleHandler {
    fn new(
        timeline: Timeline,
        from: AnimableStyle,
        to: AnimableStyle,
        start: ComputedStyle,
    ) -> Self {
        Self {
            from,
            to,
            timeline,
            current_style: start,
        }
    }

    fn current(&mut self, delta: Duration, theme: &Theme) -> &ComputedStyle {
        let progress = self.timeline.tick(delta);
        if progress.completed {
            return &self.current_style;
        }
        let interpolated = AnimableStyle::interpolate(&self.from, &self.to, progress.eased);
        interpolated.apply_to_computed(&mut self.current_style, theme);
        &self.current_style
    }
}
