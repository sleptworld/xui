use std::time::Duration;

use xui_core::prelude::*;

use xui_macros::component;

const ACTIVATE_BUTTON: CommandId = CommandId("xui.button.activate");

pub type ButtonClickCallback = Callback<()>;

/// The semantic emphasis of a `button`, after shadcn/ui's button variants.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Solid `Primary` fill; the main call to action.
    Primary,
    #[default]
    Secondary,
    /// Background-colored with a border; highlights on hover.
    Outline,
    /// No chrome until hovered.
    Ghost,
    Danger,
    /// Text-only, underlined on hover.
    Link,
}

/// The density of a `button`. All sizes retain a practical pointer target.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    #[default]
    Medium,
    Large,
    /// A square button sized for a single icon.
    Icon,
}

fn size_style(size: ButtonSize) -> Style {
    match size {
        ButtonSize::Small => Style::new()
            .min_height(32.0)
            .padding(EdgeInsets::symmetric(12.0, 6.0))
            .gap(6.0)
            .border_radius(RadiusToken::Md),
        ButtonSize::Medium => Style::new()
            .min_height(36.0)
            .padding(EdgeInsets::symmetric(16.0, 8.0))
            .gap(8.0)
            .border_radius(RadiusToken::Md),
        ButtonSize::Large => Style::new()
            .min_height(40.0)
            .padding(EdgeInsets::symmetric(24.0, 8.0))
            .gap(8.0)
            .border_radius(RadiusToken::Md),
        ButtonSize::Icon => Style::new()
            .size(Size::fix(36.0, 36.0))
            .min_width(36.0)
            .border_radius(RadiusToken::Md),
    }
}

/// shadcn's `shadow-xs`.
fn shadow_xs() -> ShadowStyle {
    ShadowStyle::new()
        .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
        .offset(Point::new(0.0, 1.0))
        .blur(2.0)
}

/// shadcn's `focus-visible:ring-[3px] ring-ring/50`.
fn focus_ring() -> ShadowStyle {
    ShadowStyle::new()
        .color(ColorToken::Ring.alpha(0.5))
        .blur(0.0)
        .spread(3.0)
}

fn variant_style(variant: ButtonVariant, interactive: bool) -> Style {
    let (background, foreground, border): (ColorValue, ColorValue, ColorValue) = match variant {
        ButtonVariant::Primary => (
            ColorToken::Primary.into(),
            ColorToken::PrimaryForeground.into(),
            Color::TRANSPARENT.into(),
        ),
        ButtonVariant::Secondary => (
            ColorToken::Secondary.into(),
            ColorToken::SecondaryForeground.into(),
            Color::TRANSPARENT.into(),
        ),
        ButtonVariant::Outline => (
            ColorToken::Background.into(),
            ColorToken::Foreground.into(),
            ColorToken::Border.into(),
        ),
        ButtonVariant::Ghost => (
            Color::TRANSPARENT.into(),
            ColorToken::Foreground.into(),
            Color::TRANSPARENT.into(),
        ),
        ButtonVariant::Danger => (
            ColorToken::Destructive.into(),
            ColorToken::DestructiveForeground.into(),
            Color::TRANSPARENT.into(),
        ),
        ButtonVariant::Link => (
            Color::TRANSPARENT.into(),
            ColorToken::Primary.into(),
            Color::TRANSPARENT.into(),
        ),
    };
    let mut style = Style::new()
        .background(background)
        .color(foreground)
        .border_color(border);
    if matches!(
        variant,
        ButtonVariant::Primary | ButtonVariant::Danger | ButtonVariant::Outline
    ) {
        style = style.shadow(shadow_xs());
    }

    if !interactive {
        // shadcn dims a disabled button to 50% opacity; halving each paint
        // alpha gives the same result without an offscreen layer.
        let dim = |value: ColorValue| match value {
            ColorValue::Token(token) => token.alpha(0.5),
            ColorValue::TokenAlpha(token, alpha) => token.alpha(alpha * 0.5),
            ColorValue::Color(color) => color.alpha(color.a * 0.5).into(),
        };
        return Style::new()
            .background(dim(background))
            .color(dim(foreground))
            .border_color(dim(border));
    }

    style = match variant {
        ButtonVariant::Primary => style
            .when(WidgetState::HOVERED, |s| s.background(ColorToken::Primary.alpha(0.9)))
            .when(WidgetState::PRESSED, |s| s.background(ColorToken::Primary.alpha(0.8))),
        ButtonVariant::Secondary => style
            .when(WidgetState::HOVERED, |s| s.background(ColorToken::Secondary.alpha(0.8)))
            .when(WidgetState::PRESSED, |s| s.background(ColorToken::Secondary.alpha(0.65))),
        ButtonVariant::Outline | ButtonVariant::Ghost => style
            .when(WidgetState::HOVERED, |s| {
                s.background(ColorToken::Accent)
                    .color(ColorToken::AccentForeground)
            })
            .when(WidgetState::PRESSED, |s| {
                s.background(ColorToken::Accent.alpha(0.8))
                    .color(ColorToken::AccentForeground)
            }),
        ButtonVariant::Danger => style
            .when(WidgetState::HOVERED, |s| s.background(ColorToken::Destructive.alpha(0.9)))
            .when(WidgetState::PRESSED, |s| s.background(ColorToken::Destructive.alpha(0.8))),
        ButtonVariant::Link => style.when(WidgetState::HOVERED, |s| {
            s.decoration(TextDecoration {
                underline: true,
                line_through: false,
            })
        }),
    };

    style.when(WidgetState::FOCUS_VISIBLE, |s| {
        s.border_color(ColorToken::Ring).shadow(focus_ring())
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
        .font_size(FontSizeToken::Md)
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
                .unwrap_or_else(|| xui_core::widgets::TextWidget::new("…").into_element_desc()),
        );
    } else if let Some(leading) = leading {
        children.push(leading.clone());
    }
    children.push(xui_core::widgets::TextWidget::new(text.clone()).into_element_desc());
    if !*loading && let Some(trailing) = trailing {
        children.push(trailing.clone());
    }

    let mut root = ContainerWidget::new()
        .style(root_style.transition(Transition::new(Duration::from_millis(150))))
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
            ButtonVariant::Link,
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
