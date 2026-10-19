//! Loading placeholders, progress activity, and contextual messages.
//!
//! [`Skeleton`] supplies static decoration, [`Spinner`] animates a cached icon transform,
//! and [`Callout`] combines arbitrary content with an optional title and [`CalloutTone`].
//! Determinate progress is provided by [`crate::controls::Progress`].
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`Skeleton`](fn@self::Skeleton) | A static, decorative placeholder for content that has not loaded. |
//! | [`Spinner`](fn@self::Spinner) | An indeterminate loading indicator rendered as a rotating cached vector icon. |
//! | [`Callout`](fn@self::Callout) | A bordered message panel with an optional title and semantic tone. |

use std::time::Duration;

use crate::layout::MergeClone;
use xui_core::prelude::*;
use xui_macros::{component, view};

/// A static, decorative placeholder for content that has not loaded.
///
/// `width` and `height` set the placeholder dimensions, and `style` overrides the muted
/// fill and rounded corners. The placeholder is hidden from accessibility. It schedules
/// no animation frames.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `width` | `Sizing` | `Sizing::Fill` |
/// | `height` | `Sizing` | `Sizing::fix(16.0)` |
/// | `style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Skeleton(width: Sizing::fix(160.0), height: Sizing::fix(20.0)) };
/// ```
#[component]
#[defaults(
    width = Sizing::Fill,
    height = Sizing::fix(16.0),
    style = Style::new(),
)]
pub fn skeleton(width: &Sizing, height: &Sizing, style: &Style) {
    let mut root_style = Style::new()
        .width(*width)
        .height(*height)
        .background(ColorToken::MutedSurface)
        .border_radius(RadiusToken::Sm);
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            accessibility_hidden: true,
        )
    }
}

/// One turn of a [`Spinner`].
const SPINNER_PERIOD: Duration = Duration::from_millis(800);

/// An indeterminate loading indicator rendered as a rotating cached vector icon.
///
/// `period` sets the turn duration and `paused` stops rotation. The animation changes the
/// icon transform instead of rebuilding the component each frame, and the runtime stops
/// requesting frames while the window is hidden. `label` supplies accessibility text;
/// `style` sets the root, including the inherited icon color.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `label` | `String` | `"Loading".to_string()` |
/// | `size` | `f32` | `24.0` |
/// | `style` | `Style` | `Style::new()` |
/// | `period` | `Duration` | `SPINNER_PERIOD` |
/// | `paused` | `bool` | `false` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Spinner(size: 20.0, label: "Loading results") };
/// ```
#[component]
#[defaults(
    label = "Loading".to_string(),
    size = 24.0,
    style = Style::new(),
    period = SPINNER_PERIOD,
    paused = false,
)]
pub fn spinner(label: &String, size: &f32, style: &Style, period: &Duration, paused: &bool) {
    let size = &if size.is_finite() {
        size.max(0.0)
    } else {
        24.0
    };
    let mut root_style = Style::new()
        .size(Size::fix(*size, *size))
        .align(AlignStyle::Center)
        .justify(JustifyStyle::Center);
    root_style.merge(style);
    let icon_style = if *paused {
        Style::new()
    } else {
        Style::new().animation(
            Animation::new(
                StylePatch::new().rotate(0.0),
                StylePatch::new().rotate(std::f32::consts::TAU),
                (*period).max(Duration::from_millis(1)),
            )
            .infinite(),
        )
    };
    view! {
        container(
            style: root_style,
            accessibility_label: label.clone(),
        ) {
            icon(
                from_icon_data: loader_circle_icon(),
                size: *size,
                style: icon_style,
            )
        }
    }
}

/// Lucide `loader-circle`.
fn loader_circle_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        IconData::from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-6.219-8.56"/></svg>"#,
        )
        .expect("embedded loader-circle icon must be valid")
    })
    .clone()
}

/// The accent emphasis of a contextual message panel.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum CalloutTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

/// A bordered message panel with an optional title and semantic tone.
///
/// `content` accepts arbitrary elements. `tone` chooses the accent border, `title_style`
/// customizes the title, and `style` overrides the panel. The content inherits the theme
/// foreground and background.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `content` | `ElementDesc` | **Required** |
/// | `title` | `Option<String>` | `None` |
/// | `tone` | `CalloutTone` | `CalloutTone::Info` |
/// | `style` | `Style` | `Style::new()` |
/// | `title_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Callout(content: view! { text("Your settings were saved.") }, title: Some("Saved".to_owned())) };
/// ```
#[component]
#[defaults(
    title = None,
    tone = CalloutTone::Info,
    style = Style::new(),
    title_style = Style::new(),
)]
pub fn callout(
    content: &ElementDesc,
    title: &Option<String>,
    tone: &CalloutTone,
    style: &Style,
    title_style: &Style,
) {
    let color = match tone {
        CalloutTone::Info => Color::hex("#1d4ed8"),
        CalloutTone::Success => Color::hex("#15803d"),
        CalloutTone::Warning => Color::hex("#a16207"),
        CalloutTone::Danger => Color::hex("#b91c1c"),
    };
    let mut children = Vec::with_capacity(2);
    if let Some(title) = title {
        children.push(view! {
            xui_core::widgets::text(
                title.clone(),
                style: Style::new().font_weight(FontWeight::Bold).merge_clone(title_style),
            )
        });
    }
    children.push(content.clone());
    let mut root_style = Style::new()
        .background(ColorToken::Background)
        .color(ColorToken::Foreground)
        .gap(6.0)
        .padding(EdgeInsets::all(12.0))
        .border_width(1.0)
        .border_color(color)
        .border_radius(RadiusToken::Md);
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
        ) {
            ..children
        }
    }
}
