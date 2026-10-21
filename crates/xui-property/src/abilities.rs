//! Handwritten hypothetical #[capability(style)] expansion.
//! StyleCapability is opt-in: Interaction and Focus are NOT StyleCapabilities.
use std::marker::PhantomData;
use crate::capability::{Capability, StyleCapability, StyleOverlay};
use crate::core::{
    Color, ColorValue, Dirty, LengthValue, ParentStyle, Sizing, StyleValue,
    Theme, overlay_value,
};
use crate::property::{Property, Scope, SlotProperty};

pub struct CapScope<C>(PhantomData<fn() -> C>);
impl<C: Capability> Scope for CapScope<C> { type Storage = C::Config; }

pub trait StyleProperty: Property {
    type Computed: Clone + PartialEq;
    const INHERITED: bool;
    const POTENTIAL_DIRTY: Dirty;

    fn compute(
        declared: &Self::Value,
        parent: Option<&Self::Computed>,
        theme: &Theme,
    ) -> Self::Computed;
}

/// Inherit is *explicit* even for a normally non-inherited property.
/// Unset is absence of a declaration (not the CSS `unset` keyword).
fn resolve_declared<T, C: Clone>(
    value: &StyleValue<T>,
    parent: Option<&C>,
    inherited_by_default: bool,
    initial: impl Fn() -> C,
    compute: impl Fn(&T) -> C,
) -> C {
    match value {
        StyleValue::Value(v) => compute(v),
        StyleValue::Initial => initial(),
        StyleValue::Inherit => parent.cloned().unwrap_or_else(initial),
        StyleValue::Unset if inherited_by_default => parent.cloned().unwrap_or_else(initial),
        StyleValue::Unset => initial(),
    }
}

pub struct BoxLayout;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoxLayoutConfig {
    pub width: StyleValue<Sizing>,
    pub height: StyleValue<Sizing>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct BoxLayoutComputed { pub width: Sizing, pub height: Sizing }
impl StyleOverlay for BoxLayoutConfig {
    fn overlay(&mut self, from: &Self) {
        overlay_value(&mut self.width, &from.width);
        overlay_value(&mut self.height, &from.height);
    }
}
impl Capability for BoxLayout { type Config = BoxLayoutConfig; }
impl StyleCapability for BoxLayout {
    type Computed = BoxLayoutComputed;
    fn resolve(d: &Self::Config, parent: &ParentStyle, theme: &Theme) -> Self::Computed {
        Self::Computed {
            width: Width::compute(&d.width, parent.width.as_ref(), theme),
            height: Height::compute(&d.height, None, theme),
        }
    }
    fn propagate(value: &Self::Computed, child: &mut ParentStyle) {
        child.width = Some(value.width);
    }
}

pub struct Typography;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TypographyConfig {
    pub color: StyleValue<ColorValue>,
    pub font_size: StyleValue<LengthValue>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TypographyComputed { pub color: Color, pub font_size: f32 }
impl StyleOverlay for TypographyConfig {
    fn overlay(&mut self, from: &Self) {
        overlay_value(&mut self.color, &from.color);
        overlay_value(&mut self.font_size, &from.font_size);
    }
}
impl Capability for Typography { type Config = TypographyConfig; }
impl StyleCapability for Typography {
    type Computed = TypographyComputed;
    fn resolve(d: &Self::Config, parent: &ParentStyle, theme: &Theme) -> Self::Computed {
        Self::Computed {
            color: TextColor::compute(&d.color, Some(&parent.color), theme),
            font_size: FontSize::compute(&d.font_size, Some(&parent.font_size), theme),
        }
    }
    fn propagate(value: &Self::Computed, child: &mut ParentStyle) {
        child.color = value.color;
        child.font_size = value.font_size;
    }
}

pub struct Decoration;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecorationConfig {
    pub background: StyleValue<ColorValue>,
    pub border_radius: StyleValue<LengthValue>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DecorationComputed { pub background: Color, pub border_radius: f32 }
impl StyleOverlay for DecorationConfig {
    fn overlay(&mut self, from: &Self) {
        overlay_value(&mut self.background, &from.background);
        overlay_value(&mut self.border_radius, &from.border_radius);
    }
}
impl Capability for Decoration { type Config = DecorationConfig; }
impl StyleCapability for Decoration {
    type Computed = DecorationComputed;
    fn resolve(d: &Self::Config, parent: &ParentStyle, theme: &Theme) -> Self::Computed {
        Self::Computed {
            background: Background::compute(&d.background, parent.background.as_ref(), theme),
            border_radius: BorderRadius::compute(&d.border_radius, None, theme),
        }
    }
    fn propagate(value: &Self::Computed, child: &mut ParentStyle) {
        child.background = Some(value.background);
    }
}

pub struct Width;
impl Property for Width {
    type Scope = CapScope<BoxLayout>;
    type Value = StyleValue<Sizing>;
    fn get(d: &BoxLayoutConfig) -> &Self::Value { &d.width }
    fn replace(d: &mut BoxLayoutConfig, v: Self::Value) { d.width = v; }
}
impl SlotProperty for Width {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for Width {
    type Computed = Sizing;
    const INHERITED: bool = false;
    const POTENTIAL_DIRTY: Dirty = Dirty::LAYOUT;
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, _: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || Sizing::Hug, |v| *v)
    }
}

pub struct Height;
impl Property for Height {
    type Scope = CapScope<BoxLayout>;
    type Value = StyleValue<Sizing>;
    fn get(d: &BoxLayoutConfig) -> &Self::Value { &d.height }
    fn replace(d: &mut BoxLayoutConfig, v: Self::Value) { d.height = v; }
}
impl SlotProperty for Height {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for Height {
    type Computed = Sizing;
    const INHERITED: bool = false;
    const POTENTIAL_DIRTY: Dirty = Dirty::LAYOUT;
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, _: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || Sizing::Hug, |v| *v)
    }
}

pub struct TextColor;
impl Property for TextColor {
    type Scope = CapScope<Typography>;
    type Value = StyleValue<ColorValue>;
    fn get(d: &TypographyConfig) -> &Self::Value { &d.color }
    fn replace(d: &mut TypographyConfig, v: Self::Value) { d.color = v; }
}
impl SlotProperty for TextColor {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for TextColor {
    type Computed = Color;
    const INHERITED: bool = true;
    const POTENTIAL_DIRTY: Dirty = Dirty::PAINT;
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, t: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || t.foreground, |v| t.color(*v))
    }
}

pub struct FontSize;
impl Property for FontSize {
    type Scope = CapScope<Typography>;
    type Value = StyleValue<LengthValue>;
    fn get(d: &TypographyConfig) -> &Self::Value { &d.font_size }
    fn replace(d: &mut TypographyConfig, v: Self::Value) { d.font_size = v; }
}
impl SlotProperty for FontSize {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for FontSize {
    type Computed = f32;
    const INHERITED: bool = true;
    const POTENTIAL_DIRTY: Dirty = Dirty(Dirty::TEXT.0 | Dirty::LAYOUT.0);
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, t: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || t.font_size, |v| t.length(*v))
    }
}

pub struct Background;
impl Property for Background {
    type Scope = CapScope<Decoration>;
    type Value = StyleValue<ColorValue>;
    fn get(d: &DecorationConfig) -> &Self::Value { &d.background }
    fn replace(d: &mut DecorationConfig, v: Self::Value) { d.background = v; }
}
impl SlotProperty for Background {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for Background {
    type Computed = Color;
    const INHERITED: bool = false;
    const POTENTIAL_DIRTY: Dirty = Dirty::PAINT;
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, t: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || Color(0), |v| t.color(*v))
    }
}

pub struct BorderRadius;
impl Property for BorderRadius {
    type Scope = CapScope<Decoration>;
    type Value = StyleValue<LengthValue>;
    fn get(d: &DecorationConfig) -> &Self::Value { &d.border_radius }
    fn replace(d: &mut DecorationConfig, v: Self::Value) { d.border_radius = v; }
}
impl SlotProperty for BorderRadius {
    const HINT: Dirty = Dirty::STYLE_INPUT;
    fn same(a: &Self::Value, b: &Self::Value) -> bool { a == b }
}
impl StyleProperty for BorderRadius {
    type Computed = f32;
    const INHERITED: bool = false;
    const POTENTIAL_DIRTY: Dirty = Dirty::PAINT;
    fn compute(d: &Self::Value, p: Option<&Self::Computed>, t: &Theme) -> Self::Computed {
        resolve_declared(d, p, Self::INHERITED, || 0.0, |v| t.length(*v))
    }
}
