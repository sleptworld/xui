use xui::prelude::*;
use xui::state::State;

const PREVIOUS_TAB: CommandId = CommandId("xui.tabs.previous");
const NEXT_TAB: CommandId = CommandId("xui.tabs.next");
const FIRST_TAB: CommandId = CommandId("xui.tabs.first");
const LAST_TAB: CommandId = CommandId("xui.tabs.last");

pub type TabChangeCallback = Callback<usize>;

/// One selectable tab and the content displayed while it is active.
#[derive(Clone, Debug)]
pub struct TabItem {
    pub id: String,
    pub label: ElementDesc,
    pub content: ElementDesc,
    pub disabled: bool,
}

impl TabItem {
    pub fn new(id: impl Into<String>, label: impl Into<Component>, content: ElementDesc) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            content,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// Visual slots for `tabs`. Each slot is a complete style and can be
/// replaced independently.
#[derive(Clone, Debug, Hash)]
pub struct TabsStyle {
    pub root: Style,
    pub list: Style,
    pub tab: Style,
    pub selected_tab: Style,
    pub disabled_tab: Style,
    pub panel: Style,
}

impl Default for TabsStyle {
    fn default() -> Self {
        let focus_ring = Color::rgba(0.42, 0.65, 1.0, 0.92);
        Self {
            root: Style::new().gap(8.0),
            list: Style::new()
                .gap(4.0)
                .padding(EdgeInsets::all(4.0))
                .background(Color::rgba(1.0, 1.0, 1.0, 0.07))
                .border_radius(10.0),
            tab: Style::new()
                .padding(EdgeInsets::symmetric(14.0, 8.0))
                .color(Color::rgba(1.0, 1.0, 1.0, 0.72))
                .border_color(Color::rgba(1.0, 1.0, 1.0, 0.0))
                .border_width(1.0)
                .border_radius(7.0)
                .when(WidgetState::HOVERED, |style| {
                    style
                        .background(Color::rgba(1.0, 1.0, 1.0, 0.08))
                        .color(Color::WHITE)
                })
                .when(WidgetState::FOCUSED, |style| style.border_color(focus_ring)),
            selected_tab: Style::new()
                .padding(EdgeInsets::symmetric(14.0, 8.0))
                .background(Color::BLUE_500)
                .color(Color::WHITE)
                .font_weight(FontWeight::Medium)
                .border_color(Color::rgba(0.42, 0.65, 1.0, 0.0))
                .border_width(1.0)
                .border_radius(7.0)
                .when(WidgetState::FOCUSED, |style| {
                    style.border_color(Color::WHITE)
                }),
            disabled_tab: Style::new()
                .padding(EdgeInsets::symmetric(14.0, 8.0))
                .color(Color::rgba(1.0, 1.0, 1.0, 0.3))
                .border_color(Color::rgba(1.0, 1.0, 1.0, 0.0))
                .border_width(1.0)
                .border_radius(7.0),
            panel: Style::new()
                .min_height(72.0)
                .padding(EdgeInsets::all(16.0))
                .background(Color::rgba(1.0, 1.0, 1.0, 0.045))
                .border_color(Color::rgba(1.0, 1.0, 1.0, 0.1))
                .border_width(1.0)
                .border_radius(10.0),
        }
    }
}

fn normalized_selection(items: &[TabItem], requested: usize) -> Option<usize> {
    items
        .get(requested)
        .filter(|item| !item.disabled)
        .map(|_| requested)
        .or_else(|| items.iter().position(|item| !item.disabled))
}

fn adjacent_enabled(items: &[TabItem], current: usize, direction: isize) -> Option<usize> {
    if items.is_empty() {
        return None;
    }

    let mut candidate = current;
    for _ in 0..items.len() {
        candidate = if direction < 0 {
            candidate.checked_sub(1).unwrap_or(items.len() - 1)
        } else {
            (candidate + 1) % items.len()
        };
        if !items[candidate].disabled {
            return Some(candidate);
        }
    }
    None
}

fn activate_tab(
    index: usize,
    controlled: bool,
    internal_selection: State<usize>,
    on_change: &Option<TabChangeCallback>,
) {
    if !controlled {
        internal_selection.set(index);
    }
    if let Some(on_change) = on_change {
        on_change.call(index);
    }
}

fn tab_ids(items: &[TabItem]) -> Vec<String> {
    items.iter().map(|item| item.id.clone()).collect()
}

fn new_focus_handles(count: usize) -> Vec<FocusHandle> {
    (0..count).map(|_| FocusHandle::new()).collect()
}

/// A horizontal, accessible tabs component.
///
/// Pass `selected` for controlled mode. When it is `None`, the component owns
/// the active index; `on_change` is invoked in both modes. Arrow keys wrap and
/// skip disabled tabs, while Home and End jump to the first and last enabled
/// tab. Keyboard focus automatically activates its destination.
#[component]
#[defaults(
    selected = None,
    on_change = None,
    style = TabsStyle::default(),
    id_prefix = "tabs".to_string(),
)]
pub fn tabs(
    items: &Vec<TabItem>,
    selected: &Option<usize>,
    on_change: &Option<TabChangeCallback>,
    style: &TabsStyle,
    id_prefix: &String,
) {
    let initial_selection = normalized_selection(items, selected.unwrap_or(0)).unwrap_or(0);
    let internal_selection = cx.use_state(|| initial_selection);
    let requested = selected.unwrap_or(*internal_selection.get());
    let active = normalized_selection(items, requested);
    let controlled = selected.is_some();

    let handle_dependencies = tab_ids(items);
    let handle_count = items.len();
    let focus_handles =
        cx.use_memo_with(handle_dependencies, move || new_focus_handles(handle_count));

    let first_enabled = items.iter().position(|item| !item.disabled);
    let last_enabled = items.iter().rposition(|item| !item.disabled);
    let mut tab_elements = Vec::with_capacity(items.len());

    for (index, item) in items.iter().enumerate() {
        let is_active = active == Some(index);
        let tab_id = format!("{id_prefix}-tab-{}", item.id);
        let panel_id = format!("{id_prefix}-panel-{}", item.id);
        let previous = adjacent_enabled(items, index, -1);
        let next = adjacent_enabled(items, index, 1);
        let handles = focus_handles.get().clone();
        let callback = on_change.clone();
        let state = internal_selection;
        let disabled = item.disabled;

        let tab_style = if disabled {
            style.disabled_tab.clone()
        } else if is_active {
            style.selected_tab.clone()
        } else {
            style.tab.clone()
        };

        let tab = ContainerWidget::new()
            .key(format!("tab-{}-{index}", item.id))
            .style(tab_style)
            .focusable(!disabled)
            .tab_index(if is_active && !disabled { 0 } else { -1 })
            .focus_handle(focus_handles.get()[index].clone())
            .accessibility_role(AccessibilityRole::Tab)
            .accessibility_id(tab_id)
            // .accessibility_label(item.label.clone())
            .accessibility_selected(is_active)
            .accessibility_disabled(disabled)
            .accessibility_controls(panel_id)
            .shortcut(
                Shortcut::named(NamedKey::ArrowLeft).allow_repeat(true),
                PREVIOUS_TAB,
            )
            .shortcut(
                Shortcut::named(NamedKey::ArrowRight).allow_repeat(true),
                NEXT_TAB,
            )
            .shortcut(Shortcut::named(NamedKey::Home), FIRST_TAB)
            .shortcut(Shortcut::named(NamedKey::End), LAST_TAB)
            .on_click(move |_, event_cx| {
                if disabled {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                activate_tab(index, controlled, state, &callback);
                EventResult::Consumed
            })
            .on_command({
                let callback = on_change.clone();
                move |event, event_cx| {
                    let target = match event.command {
                        PREVIOUS_TAB => previous,
                        NEXT_TAB => next,
                        FIRST_TAB => first_enabled,
                        LAST_TAB => last_enabled,
                        _ => return EventResult::Ignored,
                    };
                    let Some(target) = target else {
                        return EventResult::Ignored;
                    };
                    handles[target].request_focus(event_cx);
                    activate_tab(target, controlled, state, &callback);
                    EventResult::Consumed
                }
            })
            .into_element_desc(vec![item.label.clone()]);

        tab_elements.push(tab);
    }

    let tab_list = ContainerWidget::new()
        .style(style.list.clone())
        .flex_direction(FlexDirectionStyle::Row)
        .accessibility_role(AccessibilityRole::TabList)
        .into_element_desc(tab_elements);

    let mut children = vec![tab_list];
    if let Some(active) = active {
        let item = &items[active];
        children.push(
            ContainerWidget::new()
                .key(format!("panel-{}-{active}", item.id))
                .style(style.panel.clone())
                .accessibility_role(AccessibilityRole::TabPanel)
                .accessibility_id(format!("{id_prefix}-panel-{}", item.id))
                .accessibility_labelled_by(format!("{id_prefix}-tab-{}", item.id))
                .into_element_desc(vec![item.content.clone()]),
        );
    }

    ContainerWidget::new()
        .style(style.root.clone())
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, disabled: bool) -> TabItem {
        TabItem::new(id, id.to_string(), TextWidget::new(id.to_string()).into_element_desc())
            .disabled(disabled)
    }

    #[test]
    fn selection_falls_back_to_first_enabled_tab() {
        let items = vec![item("one", true), item("two", false), item("three", false)];
        assert_eq!(normalized_selection(&items, 0), Some(1));
        assert_eq!(normalized_selection(&items, 99), Some(1));
        assert_eq!(normalized_selection(&items, 2), Some(2));
    }

    #[test]
    fn adjacent_navigation_wraps_and_skips_disabled_tabs() {
        let items = vec![item("one", false), item("two", true), item("three", false)];
        assert_eq!(adjacent_enabled(&items, 0, 1), Some(2));
        assert_eq!(adjacent_enabled(&items, 2, 1), Some(0));
        assert_eq!(adjacent_enabled(&items, 0, -1), Some(2));
    }

    #[test]
    fn all_disabled_tabs_have_no_selection_or_navigation_target() {
        let items = vec![item("one", true), item("two", true)];
        assert_eq!(normalized_selection(&items, 0), None);
        assert_eq!(adjacent_enabled(&items, 0, 1), None);
    }
}
