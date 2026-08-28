use xui::prelude::*;

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
    pub backdrop: Style,
    pub menu: Style,
    pub option: Style,
    pub selected_option: Style,
    pub disabled_option: Style,
}

impl Default for DropDownStyle {
    fn default() -> Self {
        let border = Color::rgba(1.0, 1.0, 1.0, 0.14);
        let focus = Color::rgba(0.42, 0.65, 1.0, 0.92);
        Self {
            root: Style::new().min_width(180.0),
            trigger: Style::new()
                .padding(EdgeInsets::symmetric(12.0, 8.0))
                .background(Color::rgba(1.0, 1.0, 1.0, 0.07))
                .border_color(border)
                .border_width(1.0)
                .border_radius(7.0)
                .when(WidgetState::HOVERED, |style| {
                    style.background(Color::rgba(1.0, 1.0, 1.0, 0.11))
                })
                .when(WidgetState::FOCUSED, |style| style.border_color(focus)),
            trigger_open: Style::new().border_color(focus),
            backdrop: Style::new(),
            menu: Style::new()
                .padding(EdgeInsets::all(4.0))
                .background(Color::rgba(0.10, 0.11, 0.14, 0.98))
                .border_color(border)
                .border_width(1.0)
                .border_radius(8.0)
                .max_height(280.0)
                .scroll_vertical(),
            option: Style::new()
                .padding(EdgeInsets::symmetric(10.0, 7.0))
                .border_radius(5.0)
                .color(Color::rgba(1.0, 1.0, 1.0, 0.82))
                .when(WidgetState::HOVERED, |style| {
                    style.background(Color::rgba(1.0, 1.0, 1.0, 0.09))
                })
                .when(WidgetState::FOCUSED, |style| style.border_color(focus)),
            selected_option: Style::new()
                .padding(EdgeInsets::symmetric(10.0, 7.0))
                .background(Color::BLUE_500)
                .border_radius(5.0)
                .color(Color::WHITE)
                .font_weight(FontWeight::Medium),
            disabled_option: Style::new()
                .padding(EdgeInsets::symmetric(10.0, 7.0))
                .border_radius(5.0)
                .color(Color::rgba(1.0, 1.0, 1.0, 0.3)),
        }
    }
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
    selection: xui::state::State<usize>,
    open: xui::state::State<bool>,
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
    let anchor = cx.use_state(|| None::<(Point, Size<f32>)>);
    let controlled = selected.is_some();
    let active = normalized_selection(items, selected.unwrap_or(*internal_selection.get()));

    let label = active
        .and_then(|index| items.get(index))
        .map(|item| item.label.clone())
        .unwrap_or_else(|| placeholder.clone());

    let mut trigger_style = style.trigger.clone();
    if *open.get() {
        trigger_style.merge(&style.trigger_open);
    }

    let trigger = ContainerWidget::new()
        .key(format!("{id_prefix}-trigger"))
        .style(trigger_style)
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
                anchor.set(Some((
                    event_cx.node_ref.world_origin,
                    event_cx.node_ref.layout.size(),
                )));
                open.update(|open| *open = !*open);
                event_cx.request_focus();
                EventResult::Consumed
            }
        })
        .into_element_desc(vec![TextWidget::new(label).into_element_desc()]);

    let mut children = vec![trigger];
    if *open.get()
        && let Some((origin, trigger_size)) = *anchor.get()
    {
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
                    .into_element_desc(vec![TextWidget::new(item.label.clone()).into_element_desc()]),
            );
        }

        let menu_style = style
            .menu
            .clone()
            .absolute()
            .inset(EdgeInsets::new(
                origin.x,
                0.0,
                origin.y + trigger_size.height + 4.0,
                0.0,
            ))
            .width(trigger_size.width.max(1.0));
        let menu = ContainerWidget::new()
            .key(format!("{id_prefix}-menu"))
            .style(menu_style)
            .flex_direction(FlexDirectionStyle::Column)
            .accessibility_role(AccessibilityRole::List)
            .accessibility_id(format!("{id_prefix}-menu"))
            .on_click(|_, _| EventResult::Consumed)
            .into_element_desc(options);
        let backdrop = ContainerWidget::new()
            .key(format!("{id_prefix}-backdrop"))
            .style(
                style
                    .backdrop
                    .clone()
                    .absolute()
                    .inset(EdgeInsets::zero())
                    .size(Size::fill()),
            )
            .on_click(move |_, _| {
                open.set(false);
                EventResult::Consumed
            })
            .into_element_desc(vec![menu]);

        children.push(
            portal(vec![backdrop])
                .key(format!("{id_prefix}-portal"))
                .z_index(*z_index)
                .modal(true)
                .into(),
        );
    }

    ContainerWidget::new()
        .style(style.root.clone())
        .into_element_desc(children)
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
