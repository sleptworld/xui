//! Loading placeholders, progress activity, and contextual messages.
//!
//! [`skeleton`] supplies static decoration, [`spinner`] animates a cached icon transform,
//! and [`callout`] combines arbitrary content with an optional title and [`CalloutTone`].
//! Determinate progress is provided by [`crate::controls::progress`].
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`skeleton`](fn@self::skeleton) | A static, decorative placeholder for content that has not loaded. |
//! | [`spinner`](fn@self::spinner) | An indeterminate loading indicator rendered as a rotating cached vector icon. |
//! | [`callout`](fn@self::callout) | A bordered message panel with an optional title and semantic tone. |

use std::time::Duration;

use xui_core::prelude::*;
use xui_macros::component;

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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { skeleton(width: Sizing::fix(160.0), height: Sizing::fix(20.0)) }
/// }
/// ```
#[component]
#[defaults(
    width = Sizing::Fill,
    height = Sizing::fix(16.0),
    style = StyleOf::new(),
)]
pub fn skeleton(#[source] width: &Sizing, #[source] height: &Sizing, style: &StyleOf<Container>) {
    let root_style: StyleOf<Container> = memo!({
        let mut root_style = StyleOf::new()
            .with::<container::width>(*width)
            .with::<container::height>(*height)
            .with::<container::background>(ColorToken::MutedSurface)
            .with::<container::border_radius>(RadiusToken::Sm);
        root_style.merge(style);
        root_style
    });
    view! {
        container(
            style: root_style.clone(),
            accessibility_hidden: true,
        )
    }
}

/// One turn of a [`spinner`].
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `period` | `Duration` | `SPINNER_PERIOD` |
/// | `paused` | `bool` | `false` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { spinner(size: 20.0, label: "Loading results") }
/// }
/// ```
#[component]
#[defaults(
    label = "Loading".to_string(),
    size = 24.0,
    style = StyleOf::new(),
    period = SPINNER_PERIOD,
    paused = false,
)]
pub fn spinner(
    #[source] label: &String,
    #[source] size: &f32,
    style: &StyleOf<Container>,
    #[source] period: &Duration,
    #[source] paused: &bool,
) {
    let side: f32 = memo!(if size.is_finite() { size.max(0.0) } else { 24.0 });
    let root_style: StyleOf<Container> = memo!({
        let mut root_style = StyleOf::new()
            .with::<container::size>(Size::fix(*side, *side))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::Center);
        root_style.merge(style);
        root_style
    });
    view! {
        container(
            style: root_style.clone(),
            accessibility_label: label.clone(),
        ) {
            icon(
                data: loader_circle_icon(),
                size: Size::fix(*side, *side),
                draft: |indicator| if *paused {
                    indicator
                } else {
                    indicator.animate(
                        Animation::new((*period).max(Duration::from_millis(1))).infinite(),
                        |from| from.set::<icon::rotate>(0.0),
                        |to| to.set::<icon::rotate>(std::f32::consts::TAU),
                    )
                },
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `title_style` | `StyleOf<Container>` | `StyleOf::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { callout(title: Some("Saved".to_owned())) { text("Your settings were saved.") } }
/// }
/// ```
#[component]
#[defaults(
    title = None::<String>,
    tone = CalloutTone::Info,
    style = StyleOf::new(),
    title_style = StyleOf::new(),
)]
pub fn callout(
    children: &Children,
    #[source] title: &Option<String>,
    #[source] tone: &CalloutTone,
    style: &StyleOf<Container>,
    title_style: &StyleOf<Text>,
) {
    let heading_style: StyleOf<Text> =
        StyleOf::new().with::<container::font_weight>(FontWeight::Bold).merged(title_style);
    let root_style: StyleOf<Container> = memo!({
        let color = match tone {
            CalloutTone::Info => Color::hex("#1d4ed8"),
            CalloutTone::Success => Color::hex("#15803d"),
            CalloutTone::Warning => Color::hex("#a16207"),
            CalloutTone::Danger => Color::hex("#b91c1c"),
        };
        let mut root_style = StyleOf::new()
            .with::<container::background>(ColorToken::Background)
            .with::<container::color>(ColorToken::Foreground)
            .with::<container::gap>(6.0)
            .with::<container::padding>(EdgeInsets::all(12.0))
            .with::<container::border_width>(1.0)
            .with::<container::border_color>(color)
            .with::<container::border_radius>(RadiusToken::Md);
        root_style.merge(style);
        root_style
    });
    view! {
        container(
            style: root_style.clone(),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            if let Some(title) = title.clone() {
                text(title.clone(), style: heading_style.clone())
            }
            slot(children)
        }
    }
}
