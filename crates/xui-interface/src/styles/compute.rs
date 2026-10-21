//! How a declared style unit resolves against the theme: the computed form
//! of each [`units`](super::units) type, and the default resolution of one
//! declared field into its computed field.
//!
//! Capabilities (in `xui-core`) decide which computed field a declaration
//! writes; this module only knows values.

use std::sync::Arc;

use super::computed::{
    ComputedBackdropStyle, ComputedColorStyle, ComputedEffect, ComputedShadowStyle,
    ComputedStrokeStyle,
};
use super::units::{
    BackdropStyle, ColorStyle, ColorValue, Effect, LengthValue, ShadowStyle, StrokeStyle,
    StyleValue,
};
use crate::Color;
use crate::style::{
    Theme, color_style, color_value, compute_backdrop_style, compute_effect, length_value,
};

/// Resolves a declared unit against the theme into its computed form `C`.
pub trait StyleCompute<C> {
    fn compute(&self, theme: &Theme) -> C;
}

impl<T: Clone> StyleCompute<T> for T {
    fn compute(&self, _: &Theme) -> T {
        self.clone()
    }
}

/// A computed field that is `None` until declared (`min_width`, `inset`,
/// `cursor`, ..).
impl<T: Clone> StyleCompute<Option<T>> for T {
    fn compute(&self, _: &Theme) -> Option<T> {
        Some(self.clone())
    }
}

impl StyleCompute<f32> for LengthValue {
    fn compute(&self, theme: &Theme) -> f32 {
        length_value(*self, theme)
    }
}

impl StyleCompute<Color> for ColorValue {
    fn compute(&self, theme: &Theme) -> Color {
        color_value(*self, theme)
    }
}

impl StyleCompute<ComputedColorStyle> for ColorStyle {
    fn compute(&self, theme: &Theme) -> ComputedColorStyle {
        color_style(*self, theme)
    }
}

impl StyleCompute<Option<ComputedShadowStyle>> for ShadowStyle {
    fn compute(&self, theme: &Theme) -> Option<ComputedShadowStyle> {
        Some(ComputedShadowStyle {
            color: color_value(self.color, theme),
            offset: self.offset,
            blur: length_value(self.blur, theme),
            spread: length_value(self.spread, theme),
        })
    }
}

/// An invisible stroke (zero width or fully transparent) computes to none.
impl StyleCompute<Option<ComputedStrokeStyle>> for StrokeStyle {
    fn compute(&self, theme: &Theme) -> Option<ComputedStrokeStyle> {
        let color = color_style(self.color, theme);
        let width = length_value(self.width, theme);
        (width > 0.0 && color.is_visible()).then_some(ComputedStrokeStyle {
            color,
            width,
            line_style: self.line_style,
        })
    }
}

impl StyleCompute<Option<ComputedBackdropStyle>> for BackdropStyle {
    fn compute(&self, theme: &Theme) -> Option<ComputedBackdropStyle> {
        Some(compute_backdrop_style(self, theme))
    }
}

impl StyleCompute<Arc<[ComputedEffect]>> for Arc<[Effect]> {
    fn compute(&self, theme: &Theme) -> Arc<[ComputedEffect]> {
        self.iter()
            .map(|effect| compute_effect(effect, theme))
            .collect()
    }
}

/// The default resolution of one field.
///
/// `Unset` keeps what the node inherited; `Inherit` takes the parent's value
/// for an inherited property and is the same as `Unset` otherwise; `Initial`
/// takes the theme's initial value.
#[doc(hidden)]
pub fn resolve_field<D: StyleCompute<C>, C: Clone>(
    value: &StyleValue<D>,
    inherited: bool,
    target: &mut C,
    parent: &C,
    initial: &C,
    theme: &Theme,
) {
    match value {
        StyleValue::Unset => {}
        StyleValue::Inherit => {
            if inherited {
                *target = parent.clone();
            }
        }
        StyleValue::Initial => *target = initial.clone(),
        StyleValue::Value(value) => *target = value.compute(theme),
    }
}

#[doc(hidden)]
pub fn leaves_to_parent<T>(value: &StyleValue<T>) -> bool {
    matches!(value, StyleValue::Unset | StyleValue::Inherit)
}
