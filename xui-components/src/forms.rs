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

use crate::layout::MergeClone;
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
/// | `style` | `Style` | `Style::new()` |
/// | `label_style` | `Style` | `Style::new()` |
/// | `description_style` | `Style` | `Style::new()` |
/// | `message_style` | `Style` | `Style::new()` |
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
    style = Style::new(),
    label_style = Style::new(),
    description_style = Style::new(),
    message_style = Style::new(),
)]
pub fn form_field(
    label: &String,
    control: &ElementDesc,
    description: &Option<String>,
    message: &Option<String>,
    required: &bool,
    disabled: &bool,
    status: &FieldStatus,
    style: &Style,
    label_style: &Style,
    description_style: &Style,
    message_style: &Style,
) {
    let mut children = vec![view! {
        container(
            style: Style::new().gap(4.0).color(if *disabled {
                    ColorValue::from(ColorToken::MutedForeground)
                } else {
                    ColorValue::from(ColorToken::Foreground)
            }).merge_clone(label_style),
            flex_direction: FlexDirectionStyle::Row,
            accessibility_role: AccessibilityRole::Label,
        ) {
            xui_core::widgets::text(
                label.clone(),
            )
            xui_core::widgets::text(
                if *required { "*" } else { "" },
            )
        }
    }];
    if let Some(description) = description {
        children.push(
            view! {
                xui_core::widgets::text(
                    description.clone(),
                    style: Style::new()
                        .font_size(FontSizeToken::Sm)
                        .color(ColorValue::from(ColorToken::MutedForeground)).merge_clone(description_style),
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
                    style: Style::new().font_size(FontSizeToken::Sm).color(color).merge_clone(message_style),
                    accessibility_label: message.clone(),
                    accessibility: AccessibilityProperties { live_region: live, label: Some(message.clone()), ..Default::default() },
                ) {
                    xui_core::widgets::text(
                        message.clone(),
                    )
                }
            },
        );
    }
    let mut root_style = Style::new().gap(6.0).width(Sizing::Fill);
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
