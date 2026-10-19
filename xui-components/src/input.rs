//! Single-line text editing with durable controllers and validation chrome.
//!
//! The [`Input`](fn@self::Input) component wraps the host text-input widget. Use a shared
//! [`xui_core::widgets::TextController`] for external editing state or let the component
//! retain an internal controller.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`Input`](fn@self::Input) | A single-line text field with IME, validation styling, and durable editing state. |

use std::time::Duration;

use xui_core::prelude::*;
use xui_core::widgets::text_input;
use xui_macros::{component, view};

use crate::layout::{ComponentColor, ComponentInsets, ComponentLength, ComponentSizing};

/// A single-line text field with IME, validation styling, and durable editing state.
///
/// Omit `controller` to retain an internal editing model for the component lifetime, or
/// supply a shared [`TextController`] to manage text externally. `read_only` permits
/// selection and copying but blocks edits; `disabled` also removes focusability. `style`
/// decorates the box and `input_style` styles the editing area. Required/invalid flags
/// provide accessibility metadata; validation remains the caller's responsibility.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `controller` | `Option<TextController>` | `None` |
/// | `disabled` | `bool` | `false` |
/// | `read_only` | `bool` | `false` |
/// | `invalid` | `bool` | `false` |
/// | `required` | `bool` | `false` |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `input_style` | `Style` | `Style::new()` |
/// | `padding` | `ComponentInsets` | `ComponentInsets::Value(EdgeInsets::symmetric(12.0, 4.0))` |
/// | `width` | `ComponentSizing` | `ComponentSizing::Value(Sizing::fill())` |
/// | `height` | `ComponentSizing` | `ComponentSizing::Value(Sizing::fix(36.0))` |
/// | `min_width` | `ComponentSizing` | `ComponentSizing::Auto` |
/// | `min_height` | `ComponentSizing` | `ComponentSizing::Auto` |
/// | `max_width` | `ComponentSizing` | `ComponentSizing::Auto` |
/// | `max_height` | `ComponentSizing` | `ComponentSizing::Auto` |
/// | `background` | `ComponentColor` | `ComponentColor::Auto` |
/// | `border_color` | `ComponentColor` | `ComponentColor::Auto` |
/// | `border_width` | `ComponentLength` | `ComponentLength::Auto` |
/// | `border_radius` | `ComponentLength` | `ComponentLength::Auto` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { xui_components::Input(accessibility_label: Some("Email".to_owned()), required: true) };
/// ```
#[component]
#[defaults(
    controller = None,
    disabled = false,
    read_only = false,
    invalid = false,
    required = false,
    accessibility_label = None,
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
    controller: &Option<TextController>,
    disabled: &bool,
    read_only: &bool,
    invalid: &bool,
    required: &bool,
    accessibility_label: &Option<String>,
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
    let internal_controller = cx.use_memo(TextController::new);
    let controller = controller.as_ref().unwrap_or(internal_controller.get());
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
    if *invalid {
        container_style = container_style.border_color(ColorToken::Destructive);
    }
    if *disabled {
        container_style = container_style.color(ColorToken::MutedForeground);
    }
    container_style.merge(style);

    // The text input fills everything inside the border and carries the
    // padding itself, so pressing the padding focuses it rather than landing on
    // the non-focusable container (which would clear focus). It centers its
    // line vertically. `input_style` can still override this for a
    // differently sized editing area.
    let mut text_input_style = padding.apply(Style::new().size(Size::fill()), |style, value| {
        style.padding(value)
    });
    text_input_style.merge(input_style);

    view! {
        container(
            style: container_style,
            focusable: false,
            on_focus_in: move |_, _| {
                focused.set(true);
                EventResult::Ignored
            },
            on_focus_out: move |_, _| {
                focused.set(false);
                EventResult::Ignored
            },
        ) {
            text_input(
                controller: controller.clone(),
                style: text_input_style,
                disabled: *disabled,
                read_only: *read_only,
                tab_index: if *disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::TextField,
                accessibility_label: accessibility_label.clone().unwrap_or_default(),
                accessibility_required: *required,
                accessibility_invalid: *invalid,
            )
        }
    }
}
