use xui::prelude::*;

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
        .accessibility_hidden(true)
        .into_element_desc(vec![])
}

#[component]
#[defaults(label = "Loading".to_string(), size = 24.0, style = Style::new())]
pub fn spinner(label: &String, size: &f32, style: &Style) {
    let mut root_style = Style::new()
        .size(Size::fix(*size, *size))
        .align(AlignStyle::Center)
        .justify(JustifyStyle::Center)
        .font_size(*size);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .accessibility_role(AccessibilityRole::ProgressIndicator)
        .accessibility_label(label.clone())
        .accessibility_live_region(AccessibilityLiveRegion::Polite)
        .into_element_desc(vec![TextWidget::new("◌").into_element_desc()])
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
        .accessibility_live_region(match tone {
            CalloutTone::Danger => AccessibilityLiveRegion::Assertive,
            _ => AccessibilityLiveRegion::Polite,
        })
        .into_element_desc(children)
}
