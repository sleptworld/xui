//! Optional, typed style overrides shared by component props.
//!
//! [`ComponentLength`], [`ComponentSizing`], [`ComponentInsets`], and [`ComponentColor`]
//! distinguish an omitted override from an explicit value. Their `Auto` variant leaves
//! the component default unchanged; `Value` applies the supplied override.

use xui_core::prelude::*;

/// An optional length override; `Auto` retains the component default.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentLength {
    #[default]
    Auto,
    Value(LengthValue),
}

impl ComponentLength {
    pub(crate) fn apply(self, style: Style, f: impl FnOnce(Style, LengthValue) -> Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => f(style, value),
        }
    }
}

impl From<LengthValue> for ComponentLength {
    fn from(value: LengthValue) -> Self {
        Self::Value(value)
    }
}

impl From<f32> for ComponentLength {
    fn from(value: f32) -> Self {
        Self::Value(value.into())
    }
}

impl From<u32> for ComponentLength {
    fn from(value: u32) -> Self {
        Self::Value((value as f32).into())
    }
}

impl From<SpacingToken> for ComponentLength {
    fn from(value: SpacingToken) -> Self {
        Self::Value(value.into())
    }
}

impl From<RadiusToken> for ComponentLength {
    fn from(value: RadiusToken) -> Self {
        Self::Value(value.into())
    }
}

/// An optional sizing override for fixed, filling, or content-sized dimensions.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentSizing {
    #[default]
    Auto,
    Value(Sizing),
}

impl ComponentSizing {
    pub(crate) fn apply(self, style: Style, f: impl FnOnce(Style, Sizing) -> Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => f(style, value),
        }
    }
}

impl From<Sizing> for ComponentSizing {
    fn from(value: Sizing) -> Self {
        Self::Value(value)
    }
}

impl From<f32> for ComponentSizing {
    fn from(value: f32) -> Self {
        Self::Value(value.into())
    }
}

impl From<u32> for ComponentSizing {
    fn from(value: u32) -> Self {
        Self::Value(value.into())
    }
}

/// An optional padding or edge-inset override.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentInsets {
    #[default]
    Auto,
    Value(EdgeInsets),
}

impl ComponentInsets {
    pub(crate) fn apply(self, style: Style, f: impl FnOnce(Style, EdgeInsets) -> Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => f(style, value),
        }
    }
}

impl From<EdgeInsets> for ComponentInsets {
    fn from(value: EdgeInsets) -> Self {
        Self::Value(value)
    }
}

impl From<f32> for ComponentInsets {
    fn from(value: f32) -> Self {
        Self::Value(EdgeInsets::all(value))
    }
}

/// An optional semantic or literal color override.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentColor {
    #[default]
    Auto,
    Value(ColorStyle),
}

impl ComponentColor {
    pub(crate) fn apply(self, style: Style, f: impl FnOnce(Style, ColorStyle) -> Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => f(style, value),
        }
    }
}

impl From<ColorStyle> for ComponentColor {
    fn from(value: ColorStyle) -> Self {
        Self::Value(value)
    }
}

impl From<ColorValue> for ComponentColor {
    fn from(value: ColorValue) -> Self {
        Self::Value(value.into())
    }
}

impl From<Color> for ComponentColor {
    fn from(value: Color) -> Self {
        Self::Value(value.into())
    }
}

impl From<ColorToken> for ComponentColor {
    fn from(value: ColorToken) -> Self {
        Self::Value(value.into())
    }
}

/// Shared interaction chrome. State rules change paint only, keeping pointer
/// targets and text layout stable while hovering or focusing.
pub(crate) fn control_style(disabled: bool) -> Style {
    let style = Style::new().color(ColorToken::Foreground).min_width(0.0);
    if disabled {
        return style.color(ColorToken::MutedForeground);
    }
    style.when(WidgetState::FOCUS_VISIBLE, |s| {
        s.border_color(ColorToken::Ring).shadow(
            ShadowStyle::new()
                .color(ColorToken::Ring.alpha(0.5))
                .spread(3.0)
                .blur(0.0),
        )
    })
}

pub(crate) trait MergeClone {
    fn merge_clone(self, custom: &Style) -> Style;
}
impl MergeClone for Style {
    fn merge_clone(mut self, custom: &Style) -> Style {
        self.merge(custom);
        self
    }
}
