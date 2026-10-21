//! Field labels, descriptions, and accessible validation feedback.
//!
//! [`form_field`] wraps an element supplied by the caller and adds required/disabled
//! metadata and status messages. [`FieldStatus`] determines feedback emphasis and
//! live-announcement priority; behavioral flags should also be passed to the nested
//! control.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`form_field`](fn@self::form_field) | A labeled form-control wrapper with description and validation feedback. |

use xui_core::prelude::*;
use xui_macros::{component, view};

/// The validation emphasis and live-announcement priority of a form field.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum FieldStatus {
    #[default]
    Normal,
    Success,
    Warning,
    Error,
}

/// A labeled form-control wrapper with description and validation feedback.
///
/// `control` is supplied by the caller. Required, invalid, and disabled flags describe
/// the field; pass corresponding behavioral flags to the nested control as well.
/// `message` and `status` configure validation feedback and live-region priority. Label,
/// description, message, and root styles can be customized independently.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `label` | `String` | **Required** |
/// | `control` | `ElementDesc` | **Required** |
/// | `description` | `Option<String>` | `None` |
/// | `message` | `Option<String>` | `None` |
/// | `required` | `bool` | `false` |
/// | `disabled` | `bool` | `false` |
/// | `status` | `FieldStatus` | `FieldStatus::Normal` |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `label_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `description_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `message_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { form_field(label: "Email", control: view! { xui_components::input() }, required: true) };
/// ```
#[component]
#[defaults(
    description = None,
    message = None,
    required = false,
    disabled = false,
    status = FieldStatus::Normal,
    style = StyleOf::new(),
    label_style = StyleOf::new(),
    description_style = StyleOf::new(),
    message_style = StyleOf::new(),
)]
pub fn form_field(
    label: &String,
    control: &ElementDesc,
    description: &Option<String>,
    message: &Option<String>,
    required: &bool,
    disabled: &bool,
    status: &FieldStatus,
    style: &StyleOf<Container>,
    label_style: &StyleOf<Container>,
    description_style: &StyleOf<Text>,
    message_style: &StyleOf<Container>,
) {
    let mut children = vec![view! {
        container(
            style: StyleOf::new().with::<container::gap>(4.0).with::<container::color>(if *disabled {
                    ColorValue::from(ColorToken::MutedForeground)
                } else {
                    ColorValue::from(ColorToken::Foreground)
            }).merged(label_style),
            flex_direction: FlexDirectionStyle::Row,
            accessibility_role: AccessibilityRole::Label,
        ) {
            xui_core::prelude::text(
                label.clone(),
            )
            xui_core::prelude::text(
                if *required { "*" } else { "" },
            )
        }
    }];
    if let Some(description) = description {
        children.push(
            view! {
                xui_core::prelude::text(
                    description.clone(),
                    style: StyleOf::new()
                        .with::<container::font_size>(FontSizeToken::Sm)
                        .with::<container::color>(ColorValue::from(ColorToken::MutedForeground)).merged(description_style),
                )
            },
        );
    }
    children.push(control.clone());
    if let Some(message) = message {
        let (color, live) = match status {
            FieldStatus::Normal => (ColorValue::from(ColorToken::MutedForeground), None),
            FieldStatus::Success => (
                ColorValue::from(Color::hex("#15803d")),
                Some(AccessibilityLiveRegion::Polite),
            ),
            FieldStatus::Warning => (
                ColorValue::from(Color::hex("#a16207")),
                Some(AccessibilityLiveRegion::Polite),
            ),
            FieldStatus::Error => (
                ColorValue::from(ColorToken::Destructive),
                Some(AccessibilityLiveRegion::Assertive),
            ),
        };

        children.push(
            view! {
                container(
                    style: StyleOf::new().with::<container::font_size>(FontSizeToken::Sm).with::<container::color>(color).merged(message_style),
                    accessibility_label: message.clone(),
                    accessibility: AccessibilityProperties { live_region: live, label: Some(message.clone()), ..Default::default() },
                ) {
                    xui_core::prelude::text(
                        message.clone(),
                    )
                }
            },
        );
    }
    let mut root_style = StyleOf::new()
        .with::<container::gap>(6.0)
        .with::<container::width>(Sizing::Fill);
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Group,
            accessibility_label: label.clone(),
            accessibility_description: message
                .clone()
                .or_else(|| description.clone())
                .unwrap_or_default(),
            accessibility_disabled: *disabled,
            accessibility_required: *required,
            accessibility_invalid: matches!(status, FieldStatus::Error),
        ) {
            ..children
        }
    }
}
