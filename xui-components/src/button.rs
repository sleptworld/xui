use std::time::Duration;

use xui::prelude::*;

const ACTIVATE_BUTTON: CommandId = CommandId("xui.button.activate");

pub type ButtonClickCallback = Callback<()>;

/// The semantic emphasis of a `button`.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Outline,
    Ghost,
    Danger,
}

/// The density of a `button`. All sizes retain a practical pointer target.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    #[default]
    Medium,
    Large,
}

fn size_style(size: ButtonSize) -> Style {
    match size {
        ButtonSize::Small => Style::new()
            .min_height(32.0)
            .padding(EdgeInsets::symmetric(12.0, 6.0))
            .gap(6.0)
            .font_size(FontSizeToken::Sm),
        ButtonSize::Medium => Style::new()
            .min_height(40.0)
            .padding(EdgeInsets::symmetric(16.0, 9.0))
            .gap(8.0)
            .font_size(FontSizeToken::Md),
        ButtonSize::Large => Style::new()
            .min_height(48.0)
            .padding(EdgeInsets::symmetric(20.0, 12.0))
            .gap(10.0)
            .font_size(FontSizeToken::Lg),
    }
}

fn variant_style(variant: ButtonVariant, interactive: bool) -> Style {
    let focus_ring = Color::rgba(0.24, 0.52, 1.0, 0.42);
    let mut style = match variant {
        ButtonVariant::Primary => Style::new()
            .background(ColorToken::Primary)
            .color(ColorToken::InverseText)
            .border_color(Color::TRANSPARENT),
        ButtonVariant::Secondary => Style::new()
            .background(ColorToken::Surface)
            .color(ColorToken::Text)
            .border_color(ColorToken::Border),
        ButtonVariant::Outline => Style::new()
            .background(Color::TRANSPARENT)
            .color(ColorToken::Primary)
            .border_color(ColorToken::Primary),
        ButtonVariant::Ghost => Style::new()
            .background(Color::TRANSPARENT)
            .color(ColorToken::Text)
            .border_color(Color::TRANSPARENT),
        ButtonVariant::Danger => Style::new()
            .background(Color::hex("#dc2626"))
            .color(Color::WHITE)
            .border_color(Color::TRANSPARENT),
    };

    if !interactive {
        return match variant {
            ButtonVariant::Primary | ButtonVariant::Danger => style
                .background(Color::hex("#a3a3a3"))
                .color(Color::rgba(1.0, 1.0, 1.0, 0.82)),
            ButtonVariant::Secondary => style
                .background(Color::hex("#e5e5e5"))
                .color(Color::rgba(0.0, 0.0, 0.0, 0.42))
                .border_color(Color::hex("#d4d4d4")),
            ButtonVariant::Outline | ButtonVariant::Ghost => style
                .color(Color::rgba(0.0, 0.0, 0.0, 0.36))
                .border_color(if variant == ButtonVariant::Outline {
                    Color::rgba(0.0, 0.0, 0.0, 0.22)
                } else {
                    Color::TRANSPARENT
                }),
        };
    }

    style = match variant {
        ButtonVariant::Primary => style
            .when(WidgetState::HOVERED, |s| {
                s.background(Color::hex("#245ac8"))
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(Color::hex("#1e4ca9"))
            }),
        ButtonVariant::Secondary => style
            .when(WidgetState::HOVERED, |s| {
                s.background(ColorToken::MutedSurface)
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(Color::hex("#a3a3a3"))
            }),
        ButtonVariant::Outline => style
            .when(WidgetState::HOVERED, |s| {
                s.background(Color::rgba(0.18, 0.42, 0.88, 0.10))
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(Color::rgba(0.18, 0.42, 0.88, 0.18))
            }),
        ButtonVariant::Ghost => style
            .when(WidgetState::HOVERED, |s| {
                s.background(Color::rgba(0.0, 0.0, 0.0, 0.06))
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(Color::rgba(0.0, 0.0, 0.0, 0.12))
            }),
        ButtonVariant::Danger => style
            .when(WidgetState::HOVERED, |s| {
                s.background(Color::hex("#b91c1c"))
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(Color::hex("#991b1b"))
            }),
    };

    style.when(WidgetState::FOCUSED, |s| {
        s.shadow(ShadowStyle::new().color(focus_ring).blur(0.0).spread(3.0))
    })
}

fn resolved_style(
    variant: ButtonVariant,
    size: ButtonSize,
    interactive: bool,
    full_width: bool,
    custom: &Style,
) -> Style {
    let mut style = Style::new()
        .min_width(44.0)
        .align(AlignStyle::Center)
        .justify(JustifyStyle::Center)
        .border_width(1.0)
        .border_radius(RadiusToken::Md)
        .font_weight(FontWeight::Medium)
        .line_height(LineHeight::Normal);
    style.merge(&size_style(size));
    style.merge(&variant_style(variant, interactive));
    if full_width {
        style = style.width(Sizing::fill());
    }
    // User styles deliberately come last so design-system wrappers can replace
    // any visual or layout decision without reimplementing button behavior.
    style.merge(custom);
    style
}

fn invoke(callback: &Option<ButtonClickCallback>) {
    if let Some(callback) = callback {
        callback.call(());
    }
}

/// A focusable, keyboard-operable button with visual variants and async states.
///
/// `loading` is intentionally treated as disabled to prevent duplicate submits.
/// `leading` and `trailing` accept arbitrary elements, making icon buttons and
/// compound labels possible without weakening the button's semantics.
#[component]
#[defaults(
    variant = ButtonVariant::Secondary,
    size = ButtonSize::Medium,
    disabled = false,
    loading = false,
    full_width = false,
    leading = None,
    trailing = None,
    loading_indicator = None,
    on_click = None,
    accessibility_label = None,
    style = Style::new(),
)]
pub fn button(
    text: &String,
    variant: &ButtonVariant,
    size: &ButtonSize,
    disabled: &bool,
    loading: &bool,
    full_width: &bool,
    leading: &Option<ElementDesc>,
    trailing: &Option<ElementDesc>,
    loading_indicator: &Option<ElementDesc>,
    on_click: &Option<ButtonClickCallback>,
    accessibility_label: &Option<String>,
    style: &Style,
) {
    let interactive = !*disabled && !*loading;
    let root_style = resolved_style(*variant, *size, interactive, *full_width, style);
    let label = accessibility_label.clone().unwrap_or_else(|| text.clone());

    let mut children = Vec::with_capacity(3);
    if *loading {
        children.push(
            loading_indicator
                .clone()
                .unwrap_or_else(|| xui::widgets::TextWidget::new("…").into_element_desc()),
        );
    } else if let Some(leading) = leading {
        children.push(leading.clone());
    }
    children.push(xui::widgets::TextWidget::new(text.clone()).into_element_desc());
    if !*loading && let Some(trailing) = trailing {
        children.push(trailing.clone());
    }

    let mut root = ContainerWidget::new()
        .style(root_style.transition(Transition::new(Duration::from_millis(120))))
        .flex_direction(FlexDirectionStyle::Row)
        .focusable(interactive)
        .tab_index(if interactive { 0 } else { -1 })
        .accessibility_role(AccessibilityRole::Button)
        .accessibility_label(label)
        .accessibility_disabled(!interactive);

    if *loading {
        root = root.accessibility_description("Loading");
    }

    if interactive {
        let click_callback = on_click.clone();
        let command_callback = on_click.clone();
        root = root
            .shortcut(Shortcut::named(NamedKey::Enter), ACTIVATE_BUTTON)
            .shortcut(Shortcut::named(NamedKey::Space), ACTIVATE_BUTTON)
            .on_click(move |event, event_cx| {
                if event
                    .button
                    .is_some_and(|button| button != PointerButton::Primary)
                {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                invoke(&click_callback);
                EventResult::Consumed
            })
            .on_command(move |event, _| {
                if event.command != ACTIVATE_BUTTON {
                    return EventResult::Ignored;
                }
                invoke(&command_callback);
                EventResult::Consumed
            });
    }

    root.into_element_desc(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_distinct_style() {
        let variants = [
            ButtonVariant::Primary,
            ButtonVariant::Secondary,
            ButtonVariant::Outline,
            ButtonVariant::Ghost,
            ButtonVariant::Danger,
        ];
        for (index, variant) in variants.iter().enumerate() {
            for other in &variants[index + 1..] {
                assert_ne!(variant_style(*variant, true), variant_style(*other, true));
            }
        }
    }

    #[test]
    fn disabled_style_has_no_interaction_state_dependencies() {
        let disabled = variant_style(ButtonVariant::Primary, false);
        assert!(disabled.state_deps().is_empty());
    }

    #[test]
    fn custom_style_is_applied_last() {
        let custom = Style::new().min_height(72.0).border_radius(18.0);
        let resolved = resolved_style(
            ButtonVariant::Primary,
            ButtonSize::Small,
            true,
            false,
            &custom,
        );
        let expected = {
            let mut expected = resolved_style(
                ButtonVariant::Primary,
                ButtonSize::Small,
                true,
                false,
                &Style::new(),
            );
            expected.merge(&custom);
            expected
        };
        assert_eq!(resolved, expected);
    }
}
