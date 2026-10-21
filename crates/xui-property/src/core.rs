//! Values, theme tokens, inheritance environment and lightweight dirty flags.
//! These are reduced stand-ins for the existing XUI value types.

use std::ops::{BitOr, BitOrAssign};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleValue<T> {
    /// No declaration in this layer. Not the CSS keyword `unset`.
    Unset,
    Inherit,
    Initial,
    Value(T),
}

impl<T> Default for StyleValue<T> {
    fn default() -> Self { Self::Unset }
}

/// Layer composition ignores an absent declaration, unlike Property::replace.
pub fn overlay_value<T: Clone>(dest: &mut StyleValue<T>, from: &StyleValue<T>) {
    if !matches!(from, StyleValue::Unset) {
        *dest = from.clone();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u32); // 0xAARRGGBB; stand-in for XUI's actual Color

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorToken { Foreground, Primary, Accent }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorValue { Literal(Color), Token(ColorToken) }

impl From<Color> for ColorValue {
    fn from(value: Color) -> Self { Self::Literal(value) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpacingToken { Small, Medium, Large }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LengthValue { Px(f32), Spacing(SpacingToken) }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sizing { Hug, Fill, Fixed(f32) }

impl From<f32> for Sizing {
    fn from(value: f32) -> Self { Self::Fixed(value) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Color,
    pub primary: Color,
    pub accent: Color,
    pub font_size: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            foreground: Color(0xFF222222),
            primary: Color(0xFF007ACC),
            accent: Color(0xFFAA5500),
            font_size: 16.0,
        }
    }
}

impl Theme {
    pub fn color(&self, value: ColorValue) -> Color {
        match value {
            ColorValue::Literal(color) => color,
            ColorValue::Token(ColorToken::Foreground) => self.foreground,
            ColorValue::Token(ColorToken::Primary) => self.primary,
            ColorValue::Token(ColorToken::Accent) => self.accent,
        }
    }

    pub fn length(&self, value: LengthValue) -> f32 {
        match value {
            LengthValue::Px(x) => x,
            LengthValue::Spacing(SpacingToken::Small) => 4.0,
            LengthValue::Spacing(SpacingToken::Medium) => 8.0,
            LengthValue::Spacing(SpacingToken::Large) => 16.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParentStyle {
    // Intrinsically inherited values always pass through intermediate nodes.
    pub color: Color,
    pub font_size: f32,
    // The immediate parent's non-inherited computed values. For explicit Inherit.
    pub width: Option<Sizing>,
    pub background: Option<Color>,
}

impl ParentStyle {
    pub fn root(theme: &Theme) -> Self {
        Self {
            color: theme.foreground,
            font_size: theme.font_size,
            width: None,
            background: None,
        }
    }

    /// Start a child's environment. Non-inherited values must not leak through
    /// an intermediate host that doesn't expose that property.
    pub fn for_child(self) -> Self {
        Self { width: None, background: None, ..self }
    }
}

pub fn resolve_style_value<T: Clone>(
    specified: &StyleValue<T>,
    initial: T,
    parent: Option<&T>,
    inherited_by_default: bool,
) -> T {
    match specified {
        StyleValue::Value(value) => value.clone(),
        StyleValue::Initial => initial,
        StyleValue::Inherit => parent.cloned().unwrap_or(initial),
        StyleValue::Unset if inherited_by_default => parent.cloned().unwrap_or(initial),
        StyleValue::Unset => initial,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetState(pub u8);

impl WidgetState {
    pub const HOVERED: u8 = 0b0000_0001;
    pub const PRESSED: u8 = 0b0000_0010;
    pub const DISABLED: u8 = 0b0000_0100;
    pub const fn new(bits: u8) -> Self { Self(bits) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateMatcher {
    pub required: u8,
    pub forbidden: u8,
}

impl StateMatcher {
    pub const fn hovered() -> Self {
        Self { required: WidgetState::HOVERED, forbidden: 0 }
    }
    pub fn matches(&self, state: WidgetState) -> bool {
        state.0 & self.required == self.required && state.0 & self.forbidden == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dirty(pub u32);

impl Dirty {
    pub const NONE: Self = Self(0);
    pub const STYLE_INPUT: Self = Self(1 << 0);
    pub const LAYOUT: Self = Self(1 << 1);
    pub const TEXT: Self = Self(1 << 2);
    pub const PAINT: Self = Self(1 << 3);
    pub const EVENT: Self = Self(1 << 4);
    pub const COMPONENT_PROPS: Self = Self(1 << 5);
    pub const INHERITED: Self = Self(1 << 6);
    pub const FOCUS: Self = Self(1 << 7);

    pub fn contains(self, other: Self) -> bool { (self.0 & other.0) == other.0 }
    pub fn is_empty(self) -> bool { self.0 == 0 }
}

impl BitOr for Dirty {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
}
impl BitOrAssign for Dirty {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
