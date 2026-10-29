//! Action buttons with semantic emphasis and density settings.
//!
//! Use [`button`](fn@self::button) for keyboard-operable actions. [`ButtonVariant`]
//! chooses emphasis and [`ButtonSize`] chooses density; arbitrary leading/trailing
//! elements support compound labels.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`button`](fn@self::button) | A focusable action button with semantic variants, loading state, and keyboard activation. |

use std::time::Duration;

use xui_core::prelude::*;

use xui_macros::component;

const ACTIVATE_BUTTON: CommandId = CommandId("xui.button.activate");

/// Callback invoked when an enabled button is activated.
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

fn size_style(size: ButtonSize) -> StyleOf<Container> {
    match size {
        ButtonSize::Small => StyleOf::new()
            .with::<container::min_height>(32.0)
            .with::<container::padding>(EdgeInsets::symmetric(12.0, 6.0))
            .with::<container::gap>(6.0)
            .with::<container::border_radius>(RadiusToken::Md),
        ButtonSize::Medium => StyleOf::new()
            .with::<container::min_height>(36.0)
            .with::<container::padding>(EdgeInsets::symmetric(16.0, 8.0))
            .with::<container::gap>(8.0)
            .with::<container::border_radius>(RadiusToken::Md),
        ButtonSize::Large => StyleOf::new()
            .with::<container::min_height>(40.0)
            .with::<container::padding>(EdgeInsets::symmetric(24.0, 8.0))
            .with::<container::gap>(8.0)
            .with::<container::border_radius>(RadiusToken::Md),
        ButtonSize::Icon => StyleOf::new()
            .with::<container::size>(Size::fix(36.0, 36.0))
            .with::<container::min_width>(36.0)
            .with::<container::border_radius>(RadiusToken::Md),
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

fn variant_style(variant: ButtonVariant, interactive: bool) -> StyleOf<Container> {
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
    let mut style = StyleOf::new()
        .with::<container::background>(background)
        .with::<container::color>(foreground)
        .with::<container::border_color>(border);
    if matches!(
        variant,
        ButtonVariant::Primary | ButtonVariant::Danger | ButtonVariant::Outline
    ) {
        style = style.with::<container::shadow>(shadow_xs());
    }

    if !interactive {
        // shadcn dims a disabled button to 50% opacity; halving each paint
        // alpha gives the same result without an offscreen layer.
        let dim = |value: ColorValue| match value {
            ColorValue::Token(token) => token.alpha(0.5),
            ColorValue::TokenAlpha(token, alpha) => token.alpha(alpha * 0.5),
            ColorValue::Color(color) => color.alpha(color.a * 0.5).into(),
        };
        return StyleOf::new()
            .with::<container::background>(dim(background))
            .with::<container::color>(dim(foreground))
            .with::<container::border_color>(dim(border));
    }

    style = match variant {
        ButtonVariant::Primary => style
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Primary.alpha(0.9)),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::PRESSED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Primary.alpha(0.8)),
            ),
        ButtonVariant::Secondary => style
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Secondary.alpha(0.8)),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::PRESSED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Secondary.alpha(0.65)),
            ),
        ButtonVariant::Outline | ButtonVariant::Ghost => style
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Accent)
                    .with::<container::color>(ColorToken::AccentForeground),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::PRESSED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Accent.alpha(0.8))
                    .with::<container::color>(ColorToken::AccentForeground),
            ),
        ButtonVariant::Danger => style
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Destructive.alpha(0.9)),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::PRESSED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Destructive.alpha(0.8)),
            ),
        ButtonVariant::Link => style.when_style(
            WidgetStateMatcher::all(WidgetState::HOVERED),
            StyleOf::<Container>::new().with::<container::decoration>(TextDecoration {
                underline: true,
                line_through: false,
            }),
        ),
    };

    style.when_style(
        WidgetStateMatcher::all(WidgetState::FOCUS_VISIBLE),
        StyleOf::<Container>::new()
            .with::<container::border_color>(ColorToken::Ring)
            .with::<container::shadow>(focus_ring()),
    )
}

fn resolved_style(
    variant: ButtonVariant,
    size: ButtonSize,
    interactive: bool,
    full_width: bool,
    custom: &StyleOf<Container>,
) -> StyleOf<Container> {
    let mut style = StyleOf::new()
        .with::<container::min_width>(44.0)
        .with::<container::align>(AlignStyle::Center)
        .with::<container::justify>(JustifyStyle::Center)
        .with::<container::border_width>(1.0)
        .with::<container::font_size>(FontSizeToken::Md)
        .with::<container::font_weight>(FontWeight::Medium)
        .with::<container::line_height>(LineHeight::Normal);
    style.merge(&size_style(size));
    style.merge(&variant_style(variant, interactive));
    if full_width {
        style = style.with::<container::width>(Sizing::fill());
    }
    // User styles deliberately come last so design-system wrappers can replace
    // any visual or layout decision without reimplementing button behavior.
    style.merge(custom);
    style
}

/// A focusable action button with semantic variants, loading state, and keyboard
/// activation.
///
/// `disabled` and `loading` prevent activation and remove the button from sequential
/// focus navigation. Enter and Space activate an enabled button. `leading`, `trailing`,
/// and `loading_indicator` accept custom elements; `label_style` styles the text and
/// `style` overrides the root after defaults.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `text` | `String` | **Required** |
/// | `variant` | `ButtonVariant` | `ButtonVariant::Secondary` |
/// | `size` | `ButtonSize` | `ButtonSize::Medium` |
/// | `disabled` | `bool` | `false` |
/// | `loading` | `bool` | `false` |
/// | `full_width` | `bool` | `false` |
/// | `leading` | `Children` | none |
/// | `trailing` | `Children` | none |
/// | `loading_indicator` | `Children` | a spinner |
/// | `on_click` | `Callback<()>` | none |
/// | `accessibility_label` | `Option<String>` | `None` |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `label_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `transition_duration` | `Duration` | `Duration::from_millis(150)` |
///
/// # Example
///
/// ```ignore
/// view! { button(text: "Save", variant: ButtonVariant::Primary, on_click: |cx| saved.set(cx, true)) }
/// ```
#[component]
#[defaults(
    variant = ButtonVariant::Secondary,
    size = ButtonSize::Medium,
    disabled = false,
    loading = false,
    full_width = false,
    accessibility_label = None::<String>,
    style = StyleOf::new(),
    label_style = StyleOf::new(),
    transition_duration = Duration::from_millis(150),
)]
pub fn button(
    #[source] text: &String,
    #[source] variant: &ButtonVariant,
    #[source] size: &ButtonSize,
    #[source] disabled: &bool,
    #[source] loading: &bool,
    #[source] full_width: &bool,
    leading: &Children,
    trailing: &Children,
    loading_indicator: &Children,
    on_click: &Callback<()>,
    #[source] accessibility_label: &Option<String>,
    #[source] style: &StyleOf<Container>,
    label_style: &StyleOf<Text>,
    transition_duration: &Duration,
) {
    let interactive: bool = memo!(!*disabled && !*loading);
    let root_style: StyleOf<Container> = memo!({
        let mut root_style = resolved_style(*variant, *size, *interactive, *full_width, &StyleOf::new());
        root_style.merge(style);
        root_style
    });
    let label: String = memo!(accessibility_label.clone().unwrap_or_else(|| text.clone()));

    view! {
        container(
            style: root_style.clone(),
            transition: Transition::new(*transition_duration),
            flex_direction: FlexDirectionStyle::Row,
            focusable: *interactive,
            tab_index: if *interactive { 0 } else { -1 },
            accessibility_role: AccessibilityRole::Button,
            accessibility_label: label.clone(),
            accessibility_disabled: !*interactive,
            accessibility_description: if *loading { "Loading" } else { "" },
            shortcuts: [(Shortcut::named(NamedKey::Enter), ACTIVATE_BUTTON), (Shortcut::named(NamedKey::Space), ACTIVATE_BUTTON)]
                .map(|(shortcut, command)| ShortcutBinding { shortcut, command })
                .to_vec(),
            on_click: |cx, event, event_cx| {
                if !interactive.get() || event.button.is_some_and(|button| button != PointerButton::Primary) {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                on_click.call(cx, ());
                EventResult::Consumed
            },
            on_command: |cx, event| {
                if !interactive.get() || event.command != ACTIVATE_BUTTON {
                    return EventResult::Ignored;
                }
                on_click.call(cx, ());
                EventResult::Consumed
            },
        ) {
            if *loading {
                if loading_indicator.is_empty() {
                    crate::spinner(size: 16.0, label: "Loading".to_string())
                } else {
                    slot(loading_indicator)
                }
            } else {
                slot(leading)
            }
            xui_core::prelude::text(text.clone(), style: label_style.clone())
            if !*loading {
                slot(trailing)
            }
        }
    }
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
        assert!(disabled.rules.is_empty());
    }

    #[test]
    fn custom_style_is_applied_last() {
        let custom = StyleOf::new()
            .with::<container::min_height>(72.0)
            .with::<container::border_radius>(18.0);
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
                &StyleOf::new(),
            );
            expected.merge(&custom);
            expected
        };
        assert_eq!(resolved, expected);
    }
}
