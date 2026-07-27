use xui::prelude::*;
use xui::widgets::text_input;

use crate::layout::{ComponentColor, ComponentInsets, ComponentLength, ComponentSizing};

#[component]
#[defaults(
    controller = TextController::new(),
    style = Style::new(),
    input_style = Style::new(),
    padding = ComponentInsets::Value(EdgeInsets::symmetric(8.0, 2.0)),
    width = ComponentSizing::Auto,
    height = ComponentSizing::Auto,
    min_width = ComponentSizing::Auto,
    min_height = ComponentSizing::Auto,
    max_width = ComponentSizing::Auto,
    max_height = ComponentSizing::Auto,
    background = ComponentColor::Auto,
    border_color = ComponentColor::Auto,
    border_width = ComponentLength::Auto,
    border_radius = ComponentLength::Auto,
)]
pub fn input(
    controller: &TextController,
    style: &Style,
    input_style: &Style,
    padding: &ComponentInsets,
    width: &ComponentSizing,
    height: &ComponentSizing,
    min_width: &ComponentSizing,
    min_height: &ComponentSizing,
    max_width: &ComponentSizing,
    max_height: &ComponentSizing,
    background: &ComponentColor,
    border_color: &ComponentColor,
    border_width: &ComponentLength,
    border_radius: &ComponentLength,
) {
    // The container owns the component box and all of its decoration. The text
    // input only owns the editable content area, but it still needs an explicit
    // size so layout, hit testing, clipping, and IME positioning all agree on
    // the same rectangle.
    let mut container_style = style.clone().line_height(LineHeight::Normal);
    container_style = padding.apply(container_style, |style, value| style.padding(value));
    container_style = width.apply(container_style, |style, value| style.width(value));
    container_style = height.apply(container_style, |style, value| style.height(value));
    container_style = min_width.apply(container_style, |style, value| style.min_width(value));
    container_style = min_height.apply(container_style, |style, value| style.min_height(value));
    container_style = max_width.apply(container_style, |style, value| style.max_width(value));
    container_style = max_height.apply(container_style, |style, value| style.max_height(value));
    container_style = background.apply(container_style, |style, value| style.background(value));
    container_style = border_color.apply(container_style, |style, value| style.border_color(value));
    container_style = border_width.apply(container_style, |style, value| style.border_width(value));
    container_style =
        border_radius.apply(container_style, |style, value| style.border_radius(value));

    // Start from a concrete fill size, then let `input_style` override it when
    // callers intentionally want a differently sized editing area.
    let mut text_input_style = Style::new().size(Size::fill());
    text_input_style.merge(input_style);

    ContainerWidget::new()
        .style(container_style)
        .into_element_desc(vec![
            text_input()
                .controller(controller.clone())
                .style(text_input_style)
                .into_element_desc(),
        ])
}
