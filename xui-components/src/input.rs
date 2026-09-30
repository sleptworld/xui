use std::time::Duration;

use xui_core::prelude::*;
use xui_macros::component;
use xui_core::widgets::text_input;

use crate::layout::{ComponentColor, ComponentInsets, ComponentLength, ComponentSizing};

#[component]
#[defaults(
    controller = TextController::new(),
    style = Style::new(),
    input_style = Style::new(),
    padding = ComponentInsets::Value(EdgeInsets::symmetric(12.0, 4.0)),
    width = ComponentSizing::Value(Sizing::fill()),
    height = ComponentSizing::Value(Sizing::fix(36.0)),
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
    // The container owns the component box and its decoration (background,
    // border, shadow, ring). The text input owns everything inside the border,
    // padding included, and needs an explicit size so layout, hit testing,
    // clipping, and IME positioning all agree on the same rectangle.
    //
    // Focus lands on the inner text input, so the container follows it through
    // the bubbling FocusIn/FocusOut events to draw the ring (focus-within).
    let focused = cx.use_state(|| false);
    let mut container_style = Style::new()
        .min_width(0.0)
        .justify(JustifyStyle::Center)
        // Opaque, not transparent: shadows paint beneath the whole box, so a
        // see-through fill would show the shadow tint.
        .background(ColorToken::Background)
        .color(ColorToken::Foreground)
        .border_color(ColorToken::Input)
        .border_width(1.0)
        .border_radius(RadiusToken::Md)
        .font_size(FontSizeToken::Md)
        .shadow(
            ShadowStyle::new()
                .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
                .offset(Point::new(0.0, 1.0))
                .blur(2.0),
        )
        .transition(Transition::new(Duration::from_millis(150)));
    if *focused.get() {
        container_style = container_style.border_color(ColorToken::Ring).shadow(
            ShadowStyle::new()
                .color(ColorToken::Ring.alpha(0.5))
                .blur(0.0)
                .spread(3.0),
        );
    }
    container_style.merge(style);
    container_style = container_style.line_height(LineHeight::Normal);
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

    // The text input fills everything inside the border and carries the
    // padding itself, so pressing the padding focuses it rather than landing on
    // the non-focusable container (which would clear focus). It centers its
    // line vertically. `input_style` can still override this for a
    // differently sized editing area.
    let mut text_input_style = padding.apply(Style::new().size(Size::fill()), |style, value| {
        style.padding(value)
    });
    text_input_style.merge(input_style);

    ContainerWidget::new()
        .style(container_style)
        .focusable(false)
        .on_focus_in(move |_, _| {
            focused.set(true);
            EventResult::Ignored
        })
        .on_focus_out(move |_, _| {
            focused.set(false);
            EventResult::Ignored
        })
        .into_element_desc(vec![
            text_input()
                .controller(controller.clone())
                .style(text_input_style)
                .into_element_desc(),
        ])
}
