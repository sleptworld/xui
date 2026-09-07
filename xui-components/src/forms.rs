use xui::prelude::*;

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum FieldStatus {
    #[default]
    Normal,
    Success,
    Warning,
    Error,
}

#[component]
#[defaults(
    description = None,
    message = None,
    required = false,
    disabled = false,
    status = FieldStatus::Normal,
    style = Style::new(),
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
) {
    let mut children = vec![
        ContainerWidget::new()
            .style(Style::new().gap(4.0).color(if *disabled {
                ColorValue::from(Color::rgba(0.5, 0.5, 0.5, 1.0))
            } else {
                ColorValue::from(ColorToken::Text)
            }))
            .flex_direction(FlexDirectionStyle::Row)
            .accessibility_role(AccessibilityRole::Label)
            .into_element_desc(vec![
                TextWidget::new(label.clone()).into_element_desc(),
                TextWidget::new(if *required { "*" } else { "" }).into_element_desc(),
            ]),
    ];
    if let Some(description) = description {
        children.push(
            TextWidget::new(description.clone())
                .style(
                    Style::new()
                        .font_size(FontSizeToken::Sm)
                        .color(Color::rgba(0.55, 0.57, 0.62, 1.0)),
                )
                .into_element_desc(),
        );
    }
    children.push(control.clone());
    if let Some(message) = message {
        let (color, live) = match status {
            FieldStatus::Normal => (Color::rgba(0.55, 0.57, 0.62, 1.0), None),
            FieldStatus::Success => (Color::hex("#15803d"), Some(AccessibilityLiveRegion::Polite)),
            FieldStatus::Warning => (Color::hex("#a16207"), Some(AccessibilityLiveRegion::Polite)),
            FieldStatus::Error => (
                Color::hex("#b91c1c"),
                Some(AccessibilityLiveRegion::Assertive),
            ),
        };
        let mut feedback = ContainerWidget::new()
            .style(Style::new().font_size(FontSizeToken::Sm).color(color))
            .accessibility_label(message.clone());
        if let Some(live) = live {
            feedback = feedback.accessibility_live_region(live);
        }
        children.push(
            feedback.into_element_desc(vec![TextWidget::new(message.clone()).into_element_desc()]),
        );
    }
    let mut root_style = Style::new().gap(6.0).width(Sizing::Fill);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Group)
        .accessibility_label(label.clone())
        .accessibility_description(
            message
                .clone()
                .or_else(|| description.clone())
                .unwrap_or_default(),
        )
        .accessibility_disabled(*disabled)
        .accessibility_required(*required)
        .accessibility_invalid(matches!(status, FieldStatus::Error))
        .into_element_desc(children)
}
