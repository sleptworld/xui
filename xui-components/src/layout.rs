use xui::prelude::*;

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
