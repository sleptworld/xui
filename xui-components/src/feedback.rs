use std::time::Duration;

use xui_core::prelude::*;
use xui_macros::{component, defaults};

#[component]
#[defaults(width = Sizing::Fill, height = Sizing::fix(16.0), style = Style::new())]
pub fn skeleton(width: &Sizing, height: &Sizing, style: &Style) {
    let mut root_style = Style::new()
        .width(*width)
        .height(*height)
        .background(ColorToken::MutedSurface)
        .border_radius(RadiusToken::Sm);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .into_element_desc(vec![])
}

/// One turn of a [`spinner`].
const SPINNER_PERIOD: Duration = Duration::from_millis(800);

/// An indeterminate progress indicator: Lucide's `loader-circle`, turning.
///
/// The turn is a looping style animation of the icon's rotation, so it costs a
/// transform per frame -- the icon is drawn once and the renderer moves the
/// picture -- and stops asking for frames while the window is off screen. It
/// takes the text color, like text does; set `color` in `style` to change it.
#[component]
#[defaults(label = "Loading".to_string(), size = 24.0, style = Style::new())]
pub fn spinner(label: &String, size: &f32, style: &Style) {
    let mut root_style = Style::new()
        .size(Size::fix(*size, *size))
        .align(AlignStyle::Center)
        .justify(JustifyStyle::Center);
    root_style.merge(style);
    let turn = Animation::new(
        StylePatch::new().rotate(0.0),
        StylePatch::new().rotate(std::f32::consts::TAU),
        SPINNER_PERIOD,
    )
    .infinite();
    ContainerWidget::new()
        .style(root_style)
        .accessibility_label(label.clone())
        .into_element_desc(vec![
            icon()
                .from_icon_data(loader_circle_icon())
                .size(*size)
                .style(Style::new().animation(turn))
                .into_element_desc(),
        ])
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

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum CalloutTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

#[component]
#[defaults(title = None, tone = CalloutTone::Info, style = Style::new())]
pub fn callout(content: &ElementDesc, title: &Option<String>, tone: &CalloutTone, style: &Style) {
    let color = match tone {
        CalloutTone::Info => Color::hex("#1d4ed8"),
        CalloutTone::Success => Color::hex("#15803d"),
        CalloutTone::Warning => Color::hex("#a16207"),
        CalloutTone::Danger => Color::hex("#b91c1c"),
    };
    let mut children = Vec::with_capacity(2);
    if let Some(title) = title {
        children.push(
            TextWidget::new(title.clone())
                .style(Style::new().font_weight(FontWeight::Bold))
                .into_element_desc(),
        );
    }
    children.push(content.clone());
    let mut root_style = Style::new()
        .gap(6.0)
        .padding(EdgeInsets::all(12.0))
        .border_width(1.0)
        .border_color(color)
        .border_radius(RadiusToken::Md);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}
