use std::hash::{Hash, Hasher};

use crate::{
    Color, EdgeInsets, FontFamily, FontStyle, FontWeight, LineHeight, Size, TextContent,
    TextDecoration, text::TextStyle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StyleValue<T> {
    Unset,
    Inherit,
    Initial,
    Value(T),
}

impl<T> Default for StyleValue<T> {
    fn default() -> Self {
        Self::Unset
    }
}

impl<T> StyleValue<T> {
    pub fn value(value: T) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorToken {
    Text,
    InverseText,
    Background,
    Surface,
    MutedSurface,
    Border,
    Primary,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorValue {
    Color(Color),
    Token(ColorToken),
}

impl From<Color> for ColorValue {
    fn from(value: Color) -> Self {
        Self::Color(value)
    }
}

impl From<ColorToken> for ColorValue {
    fn from(value: ColorToken) -> Self {
        Self::Token(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpacingToken {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RadiusToken {
    Sm,
    Md,
    Lg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSizeToken {
    Sm,
    Md,
    Lg,
    Xl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthValue {
    Px(f32),
    Spacing(SpacingToken),
    Radius(RadiusToken),
    FontSize(FontSizeToken),
}

impl From<f32> for LengthValue {
    fn from(value: f32) -> Self {
        Self::Px(value)
    }
}

impl From<SpacingToken> for LengthValue {
    fn from(value: SpacingToken) -> Self {
        Self::Spacing(value)
    }
}

impl From<RadiusToken> for LengthValue {
    fn from(value: RadiusToken) -> Self {
        Self::Radius(value)
    }
}

impl From<FontSizeToken> for LengthValue {
    fn from(value: FontSizeToken) -> Self {
        Self::FontSize(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayStyle {
    Flex,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDirectionStyle {
    Row,
    Column,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStylePatch {
    pub color: StyleValue<ColorValue>,
    pub font_family: StyleValue<FontFamily>,
    pub font_size: StyleValue<LengthValue>,
    pub font_weight: StyleValue<FontWeight>,
    pub font_style: StyleValue<FontStyle>,
    pub line_height: StyleValue<LineHeight>,
    pub letter_spacing: StyleValue<LengthValue>,
    pub decoration: StyleValue<TextDecoration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LayoutStylePatch {
    pub display: StyleValue<DisplayStyle>,
    pub flex_direction: StyleValue<FlexDirectionStyle>,
    pub gap: StyleValue<LengthValue>,
    pub size: StyleValue<Size>,
    pub min_size: StyleValue<Size>,
    pub max_size: StyleValue<Size>,
    pub margin: StyleValue<EdgeInsets>,
    pub padding: StyleValue<EdgeInsets>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaintStylePatch {
    pub background: StyleValue<ColorValue>,
    pub border_color: StyleValue<ColorValue>,
    pub border_width: StyleValue<LengthValue>,
    pub border_radius: StyleValue<LengthValue>,
    pub clip: StyleValue<bool>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Style {
    pub text: TextStylePatch,
    pub layout: LayoutStylePatch,
    pub paint: PaintStylePatch,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: impl Into<ColorValue>) -> Self {
        self.text.color = StyleValue::Value(color.into());
        self
    }

    pub fn font_family(mut self, font_family: FontFamily) -> Self {
        self.text.font_family = StyleValue::Value(font_family);
        self
    }

    pub fn font_size(mut self, font_size: impl Into<LengthValue>) -> Self {
        self.text.font_size = StyleValue::Value(font_size.into());
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.text.font_weight = StyleValue::Value(font_weight);
        self
    }

    pub fn font_style(mut self, font_style: FontStyle) -> Self {
        self.text.font_style = StyleValue::Value(font_style);
        self
    }

    pub fn line_height(mut self, line_height: LineHeight) -> Self {
        self.text.line_height = StyleValue::Value(line_height);
        self
    }

    pub fn letter_spacing(mut self, letter_spacing: impl Into<LengthValue>) -> Self {
        self.text.letter_spacing = StyleValue::Value(letter_spacing.into());
        self
    }

    pub fn decoration(mut self, decoration: TextDecoration) -> Self {
        self.text.decoration = StyleValue::Value(decoration);
        self
    }

    pub fn display(mut self, display: DisplayStyle) -> Self {
        self.layout.display = StyleValue::Value(display);
        self
    }

    pub fn flex_direction(mut self, flex_direction: FlexDirectionStyle) -> Self {
        self.layout.flex_direction = StyleValue::Value(flex_direction);
        self
    }

    pub fn gap(mut self, gap: impl Into<LengthValue>) -> Self {
        self.layout.gap = StyleValue::Value(gap.into());
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.layout.size = StyleValue::Value(size);
        self
    }

    pub fn min_size(mut self, size: Size) -> Self {
        self.layout.min_size = StyleValue::Value(size);
        self
    }

    pub fn max_size(mut self, size: Size) -> Self {
        self.layout.max_size = StyleValue::Value(size);
        self
    }

    pub fn margin(mut self, margin: EdgeInsets) -> Self {
        self.layout.margin = StyleValue::Value(margin);
        self
    }

    pub fn padding(mut self, padding: EdgeInsets) -> Self {
        self.layout.padding = StyleValue::Value(padding);
        self
    }

    pub fn background(mut self, color: impl Into<ColorValue>) -> Self {
        self.paint.background = StyleValue::Value(color.into());
        self
    }

    pub fn border_color(mut self, color: impl Into<ColorValue>) -> Self {
        self.paint.border_color = StyleValue::Value(color.into());
        self
    }

    pub fn border_width(mut self, width: impl Into<LengthValue>) -> Self {
        self.paint.border_width = StyleValue::Value(width.into());
        self
    }

    pub fn border_radius(mut self, radius: impl Into<LengthValue>) -> Self {
        self.paint.border_radius = StyleValue::Value(radius.into());
        self
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.paint.clip = StyleValue::Value(clip);
        self
    }

    pub fn merge(&mut self, other: &Style) {
        merge_text(&mut self.text, &other.text);
        merge_layout(&mut self.layout, &other.layout);
        merge_paint(&mut self.paint, &other.paint);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WidgetState {
    pub hovered: bool,
    pub pressed: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTextStyle {
    pub color: Color,
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub decoration: TextDecoration,
}

impl From<TextStyle> for ComputedTextStyle {
    fn from(style: TextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family,
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

impl From<&TextStyle> for ComputedTextStyle {
    fn from(style: &TextStyle) -> Self {
        Self {
            color: style.color,
            font_family: style.font_family.clone(),
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            line_height: style.line_height,
            letter_spacing: style.letter_spacing,
            decoration: style.decoration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedLayoutStyle {
    pub display: DisplayStyle,
    pub flex_direction: FlexDirectionStyle,
    pub gap: f32,
    pub size: Option<Size>,
    pub min_size: Option<Size>,
    pub max_size: Option<Size>,
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedPaintStyle {
    pub background: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub border_radius: f32,
    pub clip: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    pub text: ComputedTextStyle,
    pub layout: ComputedLayoutStyle,
    pub paint: ComputedPaintStyle,
}

impl ComputedStyle {
    pub fn initial(theme: &Theme) -> Self {
        Self {
            text: ComputedTextStyle {
                color: theme.color(ColorToken::Text),
                font_family: FontFamily::System,
                font_size: theme.font_size(FontSizeToken::Md),
                font_weight: FontWeight::Normal,
                font_style: FontStyle::Normal,
                line_height: LineHeight::Normal,
                letter_spacing: 0.0,
                decoration: TextDecoration::default(),
            },
            layout: ComputedLayoutStyle {
                display: DisplayStyle::Flex,
                flex_direction: FlexDirectionStyle::Column,
                gap: 0.0,
                size: None,
                min_size: None,
                max_size: None,
                margin: EdgeInsets::ZERO,
                padding: EdgeInsets::ZERO,
            },
            paint: ComputedPaintStyle {
                background: Color::TRANSPARENT,
                border_color: Color::TRANSPARENT,
                border_width: 0.0,
                border_radius: 0.0,
                clip: false,
            },
        }
    }

    pub fn apply(&mut self, parent: &ComputedStyle, patch: &Style, theme: &Theme) {
        apply_text(&mut self.text, &parent.text, &patch.text, theme);
        apply_layout(&mut self.layout, &patch.layout, theme);
        apply_paint(&mut self.paint, &patch.paint, theme);
    }

    pub fn inherited_from(&self, theme: &Theme) -> Self {
        let mut next = Self::initial(theme);
        next.text = self.text.clone();
        next
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub text: Color,
    pub inverse_text: Color,
    pub background: Color,
    pub surface: Color,
    pub muted_surface: Color,
    pub border: Color,
    pub primary: Color,
    pub spacing_xs: f32,
    pub spacing_sm: f32,
    pub spacing_md: f32,
    pub spacing_lg: f32,
    pub spacing_xl: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub font_size_sm: f32,
    pub font_size_md: f32,
    pub font_size_lg: f32,
    pub font_size_xl: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: Color::BLACK,
            inverse_text: Color::WHITE,
            background: Color::WHITE,
            surface: Color::GRAY_100,
            muted_surface: Color::GRAY_300,
            border: Color::GRAY_300,
            primary: Color::BLUE_500,
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            radius_sm: 2.0,
            radius_md: 4.0,
            radius_lg: 8.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
        }
    }
}

impl Theme {
    pub fn color(&self, token: ColorToken) -> Color {
        match token {
            ColorToken::Text => self.text,
            ColorToken::InverseText => self.inverse_text,
            ColorToken::Background => self.background,
            ColorToken::Surface => self.surface,
            ColorToken::MutedSurface => self.muted_surface,
            ColorToken::Border => self.border,
            ColorToken::Primary => self.primary,
        }
    }

    pub fn spacing(&self, token: SpacingToken) -> f32 {
        match token {
            SpacingToken::Xs => self.spacing_xs,
            SpacingToken::Sm => self.spacing_sm,
            SpacingToken::Md => self.spacing_md,
            SpacingToken::Lg => self.spacing_lg,
            SpacingToken::Xl => self.spacing_xl,
        }
    }

    pub fn radius(&self, token: RadiusToken) -> f32 {
        match token {
            RadiusToken::Sm => self.radius_sm,
            RadiusToken::Md => self.radius_md,
            RadiusToken::Lg => self.radius_lg,
        }
    }

    pub fn font_size(&self, token: FontSizeToken) -> f32 {
        match token {
            FontSizeToken::Sm => self.font_size_sm,
            FontSizeToken::Md => self.font_size_md,
            FontSizeToken::Lg => self.font_size_lg,
            FontSizeToken::Xl => self.font_size_xl,
        }
    }
}

fn merge_text(target: &mut TextStylePatch, other: &TextStylePatch) {
    merge_value(&mut target.color, &other.color);
    merge_value(&mut target.font_family, &other.font_family);
    merge_value(&mut target.font_size, &other.font_size);
    merge_value(&mut target.font_weight, &other.font_weight);
    merge_value(&mut target.font_style, &other.font_style);
    merge_value(&mut target.line_height, &other.line_height);
    merge_value(&mut target.letter_spacing, &other.letter_spacing);
    merge_value(&mut target.decoration, &other.decoration);
}

fn merge_layout(target: &mut LayoutStylePatch, other: &LayoutStylePatch) {
    merge_value(&mut target.display, &other.display);
    merge_value(&mut target.flex_direction, &other.flex_direction);
    merge_value(&mut target.gap, &other.gap);
    merge_value(&mut target.size, &other.size);
    merge_value(&mut target.min_size, &other.min_size);
    merge_value(&mut target.max_size, &other.max_size);
    merge_value(&mut target.margin, &other.margin);
    merge_value(&mut target.padding, &other.padding);
}

fn merge_paint(target: &mut PaintStylePatch, other: &PaintStylePatch) {
    merge_value(&mut target.background, &other.background);
    merge_value(&mut target.border_color, &other.border_color);
    merge_value(&mut target.border_width, &other.border_width);
    merge_value(&mut target.border_radius, &other.border_radius);
    merge_value(&mut target.clip, &other.clip);
}

fn merge_value<T: Clone>(target: &mut StyleValue<T>, other: &StyleValue<T>) {
    if !matches!(other, StyleValue::Unset) {
        *target = other.clone();
    }
}

fn apply_text(
    target: &mut ComputedTextStyle,
    parent: &ComputedTextStyle,
    patch: &TextStylePatch,
    theme: &Theme,
) {
    target.color = resolve_color(
        patch.color,
        target.color,
        parent.color,
        theme.color(ColorToken::Text),
        theme,
    );
    target.font_family = resolve_clone(
        &patch.font_family,
        &target.font_family,
        &parent.font_family,
        &FontFamily::System,
    );
    target.font_size = resolve_length(
        patch.font_size,
        target.font_size,
        parent.font_size,
        theme.font_size(FontSizeToken::Md),
        theme,
    );
    target.font_weight = resolve_copy(
        patch.font_weight,
        target.font_weight,
        parent.font_weight,
        FontWeight::Normal,
    );
    target.font_style = resolve_copy(
        patch.font_style,
        target.font_style,
        parent.font_style,
        FontStyle::Normal,
    );
    target.line_height = resolve_copy(
        patch.line_height,
        target.line_height,
        parent.line_height,
        LineHeight::Normal,
    );
    target.letter_spacing = resolve_length(
        patch.letter_spacing,
        target.letter_spacing,
        parent.letter_spacing,
        0.0,
        theme,
    );
    target.decoration = resolve_copy(
        patch.decoration,
        target.decoration,
        parent.decoration,
        TextDecoration::default(),
    );
}

fn apply_layout(target: &mut ComputedLayoutStyle, patch: &LayoutStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).layout;
    target.display = resolve_copy_no_inherit(patch.display, target.display, initial.display);
    target.flex_direction = resolve_copy_no_inherit(
        patch.flex_direction,
        target.flex_direction,
        initial.flex_direction,
    );
    target.gap = resolve_length_no_inherit(patch.gap, target.gap, initial.gap, theme);
    target.size = resolve_optional_size_no_inherit(patch.size, target.size, initial.size);
    target.min_size =
        resolve_optional_size_no_inherit(patch.min_size, target.min_size, initial.min_size);
    target.max_size =
        resolve_optional_size_no_inherit(patch.max_size, target.max_size, initial.max_size);
    target.margin = resolve_copy_no_inherit(patch.margin, target.margin, initial.margin);
    target.padding = resolve_copy_no_inherit(patch.padding, target.padding, initial.padding);
}

fn apply_paint(target: &mut ComputedPaintStyle, patch: &PaintStylePatch, theme: &Theme) {
    let initial = ComputedStyle::initial(theme).paint;
    target.background = resolve_color_no_inherit(
        patch.background,
        target.background,
        initial.background,
        theme,
    );
    target.border_color = resolve_color_no_inherit(
        patch.border_color,
        target.border_color,
        initial.border_color,
        theme,
    );
    target.border_width = resolve_length_no_inherit(
        patch.border_width,
        target.border_width,
        initial.border_width,
        theme,
    );
    target.border_radius = resolve_length_no_inherit(
        patch.border_radius,
        target.border_radius,
        initial.border_radius,
        theme,
    );
    target.clip = resolve_copy_no_inherit(patch.clip, target.clip, initial.clip);
}

fn resolve_color(
    value: StyleValue<ColorValue>,
    current: Color,
    inherited: Color,
    initial: Color,
    theme: &Theme,
) -> Color {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_value(value, theme),
    }
}

fn resolve_color_no_inherit(
    value: StyleValue<ColorValue>,
    current: Color,
    initial: Color,
    theme: &Theme,
) -> Color {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => color_value(value, theme),
    }
}

fn color_value(value: ColorValue, theme: &Theme) -> Color {
    match value {
        ColorValue::Color(color) => color,
        ColorValue::Token(token) => theme.color(token),
    }
}

fn resolve_length(
    value: StyleValue<LengthValue>,
    current: f32,
    inherited: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn resolve_length_no_inherit(
    value: StyleValue<LengthValue>,
    current: f32,
    initial: f32,
    theme: &Theme,
) -> f32 {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => length_value(value, theme),
    }
}

fn length_value(value: LengthValue, theme: &Theme) -> f32 {
    match value {
        LengthValue::Px(value) => value,
        LengthValue::Spacing(token) => theme.spacing(token),
        LengthValue::Radius(token) => theme.radius(token),
        LengthValue::FontSize(token) => theme.font_size(token),
    }
}

fn resolve_copy<T: Copy>(value: StyleValue<T>, current: T, inherited: T, initial: T) -> T {
    match value {
        StyleValue::Unset => current,
        StyleValue::Inherit => inherited,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_clone<T: Clone>(value: &StyleValue<T>, current: &T, inherited: &T, initial: &T) -> T {
    match value {
        StyleValue::Unset => current.clone(),
        StyleValue::Inherit => inherited.clone(),
        StyleValue::Initial => initial.clone(),
        StyleValue::Value(value) => value.clone(),
    }
}

fn resolve_copy_no_inherit<T: Copy>(value: StyleValue<T>, current: T, initial: T) -> T {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => value,
    }
}

fn resolve_optional_size_no_inherit(
    value: StyleValue<Size>,
    current: Option<Size>,
    initial: Option<Size>,
) -> Option<Size> {
    match value {
        StyleValue::Unset | StyleValue::Inherit => current,
        StyleValue::Initial => initial,
        StyleValue::Value(value) => Some(value),
    }
}

fn hash_color<H: Hasher>(color: Color, state: &mut H) {
    color.r.to_bits().hash(state);
    color.g.to_bits().hash(state);
    color.b.to_bits().hash(state);
    color.a.to_bits().hash(state);
}

fn hash_edge_insets<H: Hasher>(value: EdgeInsets, state: &mut H) {
    value.left.to_bits().hash(state);
    value.right.to_bits().hash(state);
    value.top.to_bits().hash(state);
    value.bottom.to_bits().hash(state);
}

fn hash_size<H: Hasher>(value: Size, state: &mut H) {
    value.width.to_bits().hash(state);
    value.height.to_bits().hash(state);
}

impl Hash for ColorValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Color(color) => hash_color(*color, state),
            Self::Token(token) => token.hash(state),
        }
    }
}

impl Hash for LengthValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
        match self {
            Self::Px(value) => value.to_bits().hash(state),
            Self::Spacing(token) => token.hash(state),
            Self::Radius(token) => token.hash(state),
            Self::FontSize(token) => token.hash(state),
        }
    }
}

impl Hash for Style {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.layout.hash(state);
        self.paint.hash(state);
    }
}

impl Hash for TextStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        self.font_family.hash(state);
        self.font_size.hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.line_height.hash(state);
        self.letter_spacing.hash(state);
        self.decoration.hash(state);
    }
}

impl Hash for LayoutStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.display.hash(state);
        self.flex_direction.hash(state);
        self.gap.hash(state);
        hash_style_value_size(&self.size, state);
        hash_style_value_size(&self.min_size, state);
        hash_style_value_size(&self.max_size, state);
        hash_style_value_edge_insets(&self.margin, state);
        hash_style_value_edge_insets(&self.padding, state);
    }
}

impl Hash for PaintStylePatch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.background.hash(state);
        self.border_color.hash(state);
        self.border_width.hash(state);
        self.border_radius.hash(state);
        self.clip.hash(state);
    }
}

fn hash_style_value_size<H: Hasher>(value: &StyleValue<Size>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_size(*value, state);
    }
}

fn hash_style_value_edge_insets<H: Hasher>(value: &StyleValue<EdgeInsets>, state: &mut H) {
    core::mem::discriminant(value).hash(state);
    if let StyleValue::Value(value) = value {
        hash_edge_insets(*value, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_text_properties_flow_from_parent() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &Style::new().color(Color::BLUE_500).font_size(20.0),
            &theme,
        );

        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &Style::new(), &theme);

        assert_eq!(child.text.color, Color::BLUE_500);
        assert_eq!(child.text.font_size, 20.0);
        assert_eq!(child.paint.background, Color::TRANSPARENT);
        assert_eq!(child.layout.padding, EdgeInsets::ZERO);
    }

    #[test]
    fn initial_resets_inheritable_values() {
        let theme = Theme::default();
        let mut parent = ComputedStyle::initial(&theme);
        parent.apply(
            &ComputedStyle::initial(&theme),
            &Style::new().font_size(20.0),
            &theme,
        );

        let mut patch = Style::new();
        patch.text.font_size = StyleValue::Initial;
        let mut child = parent.inherited_from(&theme);
        child.apply(&parent, &patch, &theme);

        assert_eq!(child.text.font_size, theme.font_size(FontSizeToken::Md));
    }

    #[test]
    fn tokens_resolve_from_theme() {
        let mut theme = Theme::default();
        theme.primary = Color::rgb(0.2, 0.3, 0.4);
        theme.spacing_lg = 18.0;

        let initial = ComputedStyle::initial(&theme);
        let mut computed = initial.inherited_from(&theme);
        computed.apply(
            &initial,
            &Style::new()
                .background(ColorToken::Primary)
                .gap(SpacingToken::Lg),
            &theme,
        );

        assert_eq!(computed.paint.background, Color::rgb(0.2, 0.3, 0.4));
        assert_eq!(computed.layout.gap, 18.0);
    }
}
