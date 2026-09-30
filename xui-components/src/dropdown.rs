use xui_core::prelude::*;
use xui_macros::component;

pub type DropDownChangeCallback = Callback<usize>;

#[derive(Clone, Debug, Hash)]
pub struct DropDownItem {
    pub id: String,
    pub label: String,
    pub disabled: bool,
}

impl DropDownItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Debug, Hash)]
pub struct DropDownStyle {
    pub root: Style,
    pub trigger: Style,
    pub trigger_open: Style,
    pub menu: Style,
    pub option: Style,
    pub selected_option: Style,
    pub disabled_option: Style,
}

impl Default for DropDownStyle {
    fn default() -> Self {
        let focus_ring = ShadowStyle::new()
            .color(ColorToken::Ring.alpha(0.5))
            .blur(0.0)
            .spread(3.0);
        let option = Style::new()
            .width(Sizing::fill())
            .gap(8.0)
            .padding(EdgeInsets::new(8.0, 8.0, 6.0, 6.0))
            .align(AlignStyle::Center)
            .justify(JustifyStyle::SpaceBetween)
            .border_radius(RadiusToken::Sm)
            .font_size(FontSizeToken::Md)
            .color(ColorToken::PopoverForeground)
            .when(WidgetState::HOVERED, |style| {
                style
                    .background(ColorToken::Accent)
                    .color(ColorToken::AccentForeground)
            })
            .when(WidgetState::FOCUS_VISIBLE, |style| {
                style
                    .background(ColorToken::Accent)
                    .color(ColorToken::AccentForeground)
            });
        Self {
            root: Style::new().min_width(180.0),
            trigger: Style::new()
                .height(36.0)
                .gap(8.0)
                .padding(EdgeInsets::symmetric(12.0, 8.0))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::SpaceBetween)
                .background(ColorToken::Background)
                .color(ColorToken::Foreground)
                .font_size(FontSizeToken::Md)
                .border_color(ColorToken::Input)
                .border_width(1.0)
                .border_radius(RadiusToken::Md)
                .shadow(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
                        .offset(Point::new(0.0, 1.0))
                        .blur(2.0),
                )
                .when(WidgetState::FOCUS_VISIBLE, |style| {
                    style.border_color(ColorToken::Ring).shadow(focus_ring)
                }),
            // Like the focus ring, shown only for keyboard focus, not because
            // the menu is open.
            trigger_open: Style::new(),
            menu: Style::new()
                .padding(EdgeInsets::all(4.0))
                .background(ColorToken::Popover)
                .color(ColorToken::PopoverForeground)
                .border_color(ColorToken::Border)
                .border_width(1.0)
                .border_radius(RadiusToken::Md)
                .shadow(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.1))
                        .offset(Point::new(0.0, 4.0))
                        .blur(6.0)
                        .spread(-1.0),
                )
                .max_height(280.0)
                .scroll_vertical(),
            selected_option: option.clone(),
            disabled_option: Style::new()
                .width(Sizing::fill())
                .gap(8.0)
                .padding(EdgeInsets::new(8.0, 8.0, 6.0, 6.0))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::SpaceBetween)
                .border_radius(RadiusToken::Sm)
                .font_size(FontSizeToken::Md)
                .color(ColorToken::PopoverForeground.alpha(0.5)),
            option,
        }
    }
}

/// Lucide `chevron-down`.
fn chevron_down_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        IconData::from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#,
        )
        .expect("embedded chevron-down icon must be valid")
    })
    .clone()
}

/// Lucide `check`.
fn check_icon() -> IconData {
    static ICON: std::sync::OnceLock<IconData> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        IconData::from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#,
        )
        .expect("embedded check icon must be valid")
    })
    .clone()
}

fn indicator(data: IconData, style: Style) -> ElementDesc {
    icon()
        .from_icon_data(data)
        .size(16.0)
        .style(style)
        .into_element_desc()
}

fn normalized_selection(items: &[DropDownItem], requested: usize) -> Option<usize> {
    items
        .get(requested)
        .filter(|item| !item.disabled)
        .map(|_| requested)
        .or_else(|| items.iter().position(|item| !item.disabled))
}

fn select_item(
    index: usize,
    controlled: bool,
    selection: xui_core::state::State<usize>,
    open: xui_core::state::State<bool>,
    on_change: &Option<DropDownChangeCallback>,
) {
    if !controlled {
        selection.set(index);
    }
    open.set(false);
    if let Some(on_change) = on_change {
        on_change.call(index);
    }
}

/// A selectable menu whose option list is Portal-mounted into the runtime's
/// root overlayer, keeping it above clipped and scrolling ancestors.
#[component]
#[defaults(
    selected = None,
    on_change = None,
    placeholder = "Select…".to_string(),
    disabled = false,
    style = DropDownStyle::default(),
    id_prefix = "dropdown".to_string(),
    z_index = 1000,
)]
pub fn drop_down(
    items: &Vec<DropDownItem>,
    selected: &Option<usize>,
    on_change: &Option<DropDownChangeCallback>,
    placeholder: &String,
    disabled: &bool,
    style: &DropDownStyle,
    id_prefix: &String,
    z_index: &i32,
) {
    let initial = normalized_selection(items, selected.unwrap_or(0)).unwrap_or(0);
    let internal_selection = cx.use_state(|| initial);
    let open = cx.use_state(|| false);
    let controlled = selected.is_some();
    let active = normalized_selection(items, selected.unwrap_or(*internal_selection.get()));

    let selected_label = active
        .and_then(|index| items.get(index))
        .map(|item| item.label.clone());
    let has_selection = selected_label.is_some();
    let label = selected_label.unwrap_or_else(|| placeholder.clone());

    let mut trigger_style = style.trigger.clone();
    if *disabled {
        trigger_style = trigger_style
            .color(ColorToken::Foreground.alpha(0.5))
            .border_color(ColorToken::Input.alpha(0.5));
    }
    if *open.get() {
        trigger_style.merge(&style.trigger_open);
    }

    let trigger = ContainerWidget::new()
        .key(format!("{id_prefix}-trigger"))
        .style(trigger_style)
        .flex_direction(FlexDirectionStyle::Row)
        .focusable(!*disabled)
        .tab_index(if *disabled { -1 } else { 0 })
        .accessibility_role(AccessibilityRole::Button)
        .accessibility_id(format!("{id_prefix}-trigger"))
        .accessibility_label(label.clone())
        .accessibility_disabled(*disabled)
        .accessibility_controls(format!("{id_prefix}-menu"))
        .on_click({
            let disabled = *disabled;
            move |_, event_cx| {
                if disabled {
                    return EventResult::Ignored;
                }
                open.update(|open| *open = !*open);
                event_cx.request_focus();
                EventResult::Consumed
            }
        });
    let mut trigger_children = vec![
        TextWidget::new(label)
            .style(if has_selection {
                Style::new()
            } else {
                Style::new().color(ColorToken::MutedForeground)
            })
            .into_element_desc(),
        indicator(
            chevron_down_icon(),
            Style::new().color(ColorToken::MutedForeground),
        ),
    ];
    if *open.get() {
        let mut options = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let is_selected = active == Some(index);
            let option_style = if item.disabled {
                style.disabled_option.clone()
            } else if is_selected {
                style.selected_option.clone()
            } else {
                style.option.clone()
            };
            let disabled = item.disabled;
            let selection = internal_selection;
            let open_state = open;
            let callback = on_change.clone();
            options.push(
                ContainerWidget::new()
                    .key(format!("{id_prefix}-option-{}", item.id))
                    .style(option_style)
                    .flex_direction(FlexDirectionStyle::Row)
                    .focusable(!disabled)
                    .tab_index(if disabled { -1 } else { 0 })
                    .accessibility_role(AccessibilityRole::Button)
                    .accessibility_label(item.label.clone())
                    .accessibility_selected(is_selected)
                    .accessibility_disabled(disabled)
                    .on_click(move |_, _| {
                        if disabled {
                            return EventResult::Consumed;
                        }
                        select_item(index, controlled, selection, open_state, &callback);
                        EventResult::Consumed
                    })
                    .into_element_desc(vec![
                        TextWidget::new(item.label.clone()).into_element_desc(),
                        // Keep the slot when unselected so labels don't shift.
                        indicator(
                            check_icon(),
                            if is_selected {
                                Style::new()
                            } else {
                                Style::new().color(Color::TRANSPARENT)
                            },
                        ),
                    ]),
            );
        }

        let menu = ContainerWidget::new()
            .key(format!("{id_prefix}-menu"))
            .style(style.menu.clone())
            .flex_direction(FlexDirectionStyle::Column)
            .accessibility_role(AccessibilityRole::List)
            .accessibility_id(format!("{id_prefix}-menu"))
            .into_element_desc(options);

        // Written inside the trigger, so the trigger owns the portal and the
        // menu follows it through resizes, layout changes and scrolling. Modal:
        // a press outside reaches nothing but the dismiss handler, which is
        // also what keeps the trigger from reopening it.
        trigger_children.push(
            portal(vec![menu])
                .key(format!("{id_prefix}-portal"))
                .z_index(*z_index)
                .modal(true)
                .anchor(
                    AnchorPlacement::new(AnchorSide::Bottom)
                        .offset(4.0)
                        .match_width(true),
                )
                .on_dismiss(move |_| open.set(false))
                .into(),
        );
    }

    ContainerWidget::new()
        .style(style.root.clone())
        .into_element_desc(vec![trigger.into_element_desc(trigger_children)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_skips_disabled_and_out_of_range_items() {
        let items = vec![
            DropDownItem::new("a", "A").disabled(true),
            DropDownItem::new("b", "B"),
            DropDownItem::new("c", "C"),
        ];
        assert_eq!(normalized_selection(&items, 0), Some(1));
        assert_eq!(normalized_selection(&items, 99), Some(1));
        assert_eq!(normalized_selection(&items, 2), Some(2));
    }
}
