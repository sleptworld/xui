//! The box most layouts are made of: `container`, `row`, `column`, `center`.

use xui_interface::FlexDirectionStyle;
use xui_interface::core::Sizing;
use xui_interface::style::{AlignStyle, JustifyStyle};
use xui_macros::widget;

use crate::node::{Draft, WidgetT};

/// A box that lays its children out as a block, or along a flex line.
///
/// It selects every capability: whatever a box can be — sized, padded,
/// decorated, scrolled, focused, listened to — a container can be, and the
/// text style it declares flows to its children.
#[widget(
    style(
        box_size,
        margin,
        padding,
        flex_layout,
        position,
        display,
        typography,
        background,
        corner_radius,
        border,
        box_shadow,
        clip,
        visibility,
        transform,
        effects,
        scroll,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        scroll_binding,
        size_observation,
        events
    )
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct Container {
    /// `None` lays the children out as a block; a direction, as a flex line.
    #[default]
    #[dirty(LAYOUT_INPUT)]
    pub flex_direction: Option<FlexDirectionStyle>,
    /// Wraps overflowing children onto new lines, like CSS
    /// `flex-wrap: wrap`; `gap` then spaces the lines too. Ignored without a
    /// flex direction.
    #[default]
    #[dirty(LAYOUT_INPUT)]
    pub flex_wrap: bool,
}

impl WidgetT for Container {
    type State = ();

    fn create_state(_: &ContainerProps) {}
}

/// A block container.
pub fn container() -> Draft<Container> {
    Draft::new(ContainerProps::builder().build())
}

/// Children side by side.
pub fn row() -> Draft<Container> {
    container().with::<container::flex_direction>(FlexDirectionStyle::Row)
}

/// Children one under the other.
pub fn column() -> Draft<Container> {
    container().with::<container::flex_direction>(FlexDirectionStyle::Column)
}

/// Fills its parent and centres its children in it.
pub fn center() -> Draft<Container> {
    column()
        .with::<container::width>(Sizing::Fill)
        .with::<container::height>(Sizing::Fill)
        .with::<container::justify>(JustifyStyle::Center)
        .with::<container::align>(AlignStyle::Center)
}
