use std::hash::{Hash, Hasher};
use xui::prelude::*;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentLength {
    #[default]
    Auto,
    Value(LengthValue),
}

impl ComponentLength {
    fn apply(self, style: Style, f: impl FnOnce(Style, LengthValue) -> Style) -> Style {
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
    fn apply(self, style: Style, f: impl FnOnce(Style, Sizing) -> Style) -> Style {
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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ComponentSize {
    #[default]
    Auto,
    Value(Size<Sizing>),
}

impl ComponentSize {
    fn apply(self, style: Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => style.size(value),
        }
    }
}

impl Hash for ComponentSize {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        if let Self::Value(value) = self {
            value.width.hash(state);
            value.height.hash(state);
        }
    }
}

impl From<Size<Sizing>> for ComponentSize {
    fn from(value: Size<Sizing>) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ComponentInsets {
    #[default]
    Auto,
    Value(EdgeInsets),
}

impl ComponentInsets {
    fn apply(self, style: Style, f: impl FnOnce(Style, EdgeInsets) -> Style) -> Style {
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
    fn apply(self, style: Style, f: impl FnOnce(Style, ColorStyle) -> Style) -> Style {
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
pub enum ComponentAlign {
    #[default]
    Auto,
    Value(AlignStyle),
}

impl ComponentAlign {
    fn apply(self, style: Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => style.align(value),
        }
    }
}

impl From<AlignStyle> for ComponentAlign {
    fn from(value: AlignStyle) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
pub enum ComponentJustify {
    #[default]
    Auto,
    Value(JustifyStyle),
}

impl ComponentJustify {
    fn apply(self, style: Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => style.justify(value),
        }
    }
}

impl From<JustifyStyle> for ComponentJustify {
    fn from(value: JustifyStyle) -> Self {
        Self::Value(value)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
pub enum ComponentScrollDirection {
    #[default]
    Auto,
    Value(ScrollDirectionStyle),
}

impl ComponentScrollDirection {
    fn apply(self, style: Style) -> Style {
        match self {
            Self::Auto => style,
            Self::Value(value) => style.scroll_direction(value),
        }
    }
}

impl From<ScrollDirectionStyle> for ComponentScrollDirection {
    fn from(value: ScrollDirectionStyle) -> Self {
        Self::Value(value)
    }
}

#[component]
#[defaults(
    children = Vec::new(),
    style = Style::new(),
    gap = ComponentLength::Auto,
    padding = ComponentInsets::Auto,
    margin = ComponentInsets::Auto,
    size = ComponentSize::Auto,
    width = ComponentSizing::Auto,
    height = ComponentSizing::Auto,
    min_width = ComponentSizing::Auto,
    min_height = ComponentSizing::Auto,
    max_width = ComponentSizing::Auto,
    max_height = ComponentSizing::Auto,
    align = ComponentAlign::Auto,
    justify = ComponentJustify::Auto,
    background = ComponentColor::Auto,
    border_color = ComponentColor::Auto,
    border_width = ComponentLength::Auto,
    border_radius = ComponentLength::Auto,
    scroll_direction = ComponentScrollDirection::Auto,
)]
pub fn column(
    children: &Vec<ElementDesc>,
    style: &Style,
    gap: &ComponentLength,
    padding: &ComponentInsets,
    margin: &ComponentInsets,
    size: &ComponentSize,
    width: &ComponentSizing,
    height: &ComponentSizing,
    min_width: &ComponentSizing,
    min_height: &ComponentSizing,
    max_width: &ComponentSizing,
    max_height: &ComponentSizing,
    align: &ComponentAlign,
    justify: &ComponentJustify,
    background: &ComponentColor,
    border_color: &ComponentColor,
    border_width: &ComponentLength,
    border_radius: &ComponentLength,
    scroll_direction: &ComponentScrollDirection,
) {
    stack(
        FlexDirectionStyle::Column,
        children,
        style,
        *gap,
        *padding,
        *margin,
        *size,
        *width,
        *height,
        *min_width,
        *min_height,
        *max_width,
        *max_height,
        *align,
        *justify,
        *background,
        *border_color,
        *border_width,
        *border_radius,
        *scroll_direction,
    )
}

#[component]
#[defaults(
    children = Vec::new(),
    style = Style::new(),
    gap = ComponentLength::Auto,
    padding = ComponentInsets::Auto,
    margin = ComponentInsets::Auto,
    size = ComponentSize::Auto,
    width = ComponentSizing::Auto,
    height = ComponentSizing::Auto,
    min_width = ComponentSizing::Auto,
    min_height = ComponentSizing::Auto,
    max_width = ComponentSizing::Auto,
    max_height = ComponentSizing::Auto,
    align = ComponentAlign::Auto,
    justify = ComponentJustify::Auto,
    background = ComponentColor::Auto,
    border_color = ComponentColor::Auto,
    border_width = ComponentLength::Auto,
    border_radius = ComponentLength::Auto,
    scroll_direction = ComponentScrollDirection::Auto,
)]
pub fn row(
    children: &Vec<ElementDesc>,
    style: &Style,
    gap: &ComponentLength,
    padding: &ComponentInsets,
    margin: &ComponentInsets,
    size: &ComponentSize,
    width: &ComponentSizing,
    height: &ComponentSizing,
    min_width: &ComponentSizing,
    min_height: &ComponentSizing,
    max_width: &ComponentSizing,
    max_height: &ComponentSizing,
    align: &ComponentAlign,
    justify: &ComponentJustify,
    background: &ComponentColor,
    border_color: &ComponentColor,
    border_width: &ComponentLength,
    border_radius: &ComponentLength,
    scroll_direction: &ComponentScrollDirection,
) {
    stack(
        FlexDirectionStyle::Row,
        children,
        style,
        *gap,
        *padding,
        *margin,
        *size,
        *width,
        *height,
        *min_width,
        *min_height,
        *max_width,
        *max_height,
        *align,
        *justify,
        *background,
        *border_color,
        *border_width,
        *border_radius,
        *scroll_direction,
    )
}

fn stack(
    direction: FlexDirectionStyle,
    children: &[ElementDesc],
    style: &Style,
    gap: ComponentLength,
    padding: ComponentInsets,
    margin: ComponentInsets,
    size: ComponentSize,
    width: ComponentSizing,
    height: ComponentSizing,
    min_width: ComponentSizing,
    min_height: ComponentSizing,
    max_width: ComponentSizing,
    max_height: ComponentSizing,
    align: ComponentAlign,
    justify: ComponentJustify,
    background: ComponentColor,
    border_color: ComponentColor,
    border_width: ComponentLength,
    border_radius: ComponentLength,
    scroll_direction: ComponentScrollDirection,
) -> ElementDesc {
    let mut style = style.clone();
    style = gap.apply(style, |style, value| style.gap(value));
    style = padding.apply(style, |style, value| style.padding(value));
    style = margin.apply(style, |style, value| style.margin(value));
    style = size.apply(style);
    style = width.apply(style, |style, value| style.width(value));
    style = height.apply(style, |style, value| style.height(value));
    style = min_width.apply(style, |style, value| style.min_width(value));
    style = min_height.apply(style, |style, value| style.min_height(value));
    style = max_width.apply(style, |style, value| style.max_width(value));
    style = max_height.apply(style, |style, value| style.max_height(value));
    style = align.apply(style);
    style = justify.apply(style);
    style = background.apply(style, |style, value| style.background(value));
    style = border_color.apply(style, |style, value| style.border_color(value));
    style = border_width.apply(style, |style, value| style.border_width(value));
    style = border_radius.apply(style, |style, value| style.border_radius(value));
    style = scroll_direction.apply(style);

    ContainerWidget::new()
        .flex_direction(direction)
        .style(style)
        .into_element_desc(children.to_vec())
}
