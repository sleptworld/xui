//! Tab navigation, panel mounting, and keyboard activation modes.
//!
//! [`TabItem`] combines a label, content panel, identity, and disabled state.
//! [`tabs`](fn@self::tabs) mounts only the active panel, while [`TabsStyle`] exposes
//! independently replaceable styles.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`tabs`](fn@self::tabs) | An accessible tab list with one mounted content panel and configurable keyboard activation. |

use std::time::Duration;

use crate::{SelectionActivationMode, SelectionOrientation};
use std::rc::Rc;
use xui_core::prelude::*;
use xui_core::state::State;
use xui_macros::{component, view};

const PREVIOUS_TAB: CommandId = CommandId("xui.tabs.previous");
const NEXT_TAB: CommandId = CommandId("xui.tabs.next");
const FIRST_TAB: CommandId = CommandId("xui.tabs.first");
const LAST_TAB: CommandId = CommandId("xui.tabs.last");

/// Callback requesting a zero-based active tab index.
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
#[derive(Clone, Debug)]
pub struct TabsStyle {
    pub root: StyleOf<Container>,
    pub list: StyleOf<Container>,
    pub tab: StyleOf<Container>,
    pub selected_tab: StyleOf<Container>,
    pub disabled_tab: StyleOf<Container>,
    pub panel: StyleOf<Container>,
}

impl Default for TabsStyle {
    fn default() -> Self {
        let trigger = StyleOf::new()
            .with::<container::height>(29.0)
            .with::<container::gap>(6.0)
            .with::<container::padding>(EdgeInsets::symmetric(8.0, 4.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::Center)
            .with::<container::font_size>(FontSizeToken::Md)
            .with::<container::font_weight>(FontWeight::Medium)
            .with::<container::background>(Color::TRANSPARENT)
            .with::<container::border_color>(Color::TRANSPARENT)
            .with::<container::border_width>(1.0)
            .with::<container::border_radius>(RadiusToken::Md);
        let focused = |style: StyleOf<Container>| {
            style
                .with::<container::border_color>(ColorToken::Ring)
                .with::<container::shadow>(
                    ShadowStyle::new()
                        .color(ColorToken::Ring.alpha(0.5))
                        .blur(0.0)
                        .spread(3.0),
                )
        };
        Self {
            root: StyleOf::new()
                .with::<container::gap>(8.0)
                .with::<container::align>(AlignStyle::Start),
            list: StyleOf::new()
                .with::<container::height>(36.0)
                .with::<container::padding>(EdgeInsets::all(3.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::background>(ColorToken::Muted)
                .with::<container::color>(ColorToken::MutedForeground)
                .with::<container::border_radius>(RadiusToken::Lg),
            tab: trigger
                .clone()
                .with::<container::color>(ColorToken::MutedForeground)
                .when_style(
                    WidgetStateMatcher::all(WidgetState::HOVERED),
                    StyleOf::<Container>::new().with::<container::color>(ColorToken::Foreground),
                )
                .when_style(
                    WidgetStateMatcher::all(WidgetState::FOCUS_VISIBLE),
                    focused(StyleOf::new()),
                ),
            selected_tab: trigger
                .clone()
                .with::<container::background>(ColorToken::Background)
                .with::<container::color>(ColorToken::Foreground)
                .with::<container::shadow>(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.1))
                        .offset(Point::new(0.0, 1.0))
                        .blur(3.0),
                )
                .when_style(
                    WidgetStateMatcher::all(WidgetState::FOCUS_VISIBLE),
                    focused(StyleOf::new()),
                ),
            disabled_tab: trigger.with::<container::color>(ColorToken::MutedForeground.alpha(0.5)),
            panel: StyleOf::new().with::<container::width>(Sizing::fill()),
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

fn new_focus_handles(count: usize) -> Rc<[FocusHandle]> {
    (0..count)
        .map(|_| FocusHandle::new())
        .collect::<Vec<_>>()
        .into()
}

/// An accessible tab list with one mounted content panel and configurable keyboard
/// activation.
///
/// `selected` controls the active index; otherwise `default_selected` seeds internal
/// state. Arrows follow `orientation`, skip disabled tabs, and optionally wrap; Home/End
/// choose the first/last enabled tab. Automatic activation selects the focused
/// destination; manual activation moves focus until Enter/Space activates it.
/// [`TabsStyle`] supplies complete styles for the root, list, tabs, and panel. Keys
/// follow item IDs, and the default accessibility ID prefix is unique per instance.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<TabItem>` | **Required** |
/// | `selected` | `Option<usize>` | `None` |
/// | `default_selected` | `usize` | `0` |
/// | `orientation` | `SelectionOrientation` | `SelectionOrientation::Horizontal` |
/// | `activation` | `SelectionActivationMode` | `SelectionActivationMode::Automatic` |
/// | `wrap` | `bool` | `true` |
/// | `on_change` | `Option<TabChangeCallback>` | `None` |
/// | `style` | `TabsStyle` | `TabsStyle::default()` |
/// | `id_prefix` | `String` | `String::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { tabs(items: vec![TabItem::new("account", "Account", view! { text("Account settings") })]) };
/// ```
#[component]
#[defaults(
    selected = None,
    default_selected = 0,
    orientation = SelectionOrientation::Horizontal,
    activation = SelectionActivationMode::Automatic,
    wrap = true,
    on_change = None,
    style = TabsStyle::default(),
    id_prefix = String::new(),
)]
pub fn tabs(
    items: &Vec<TabItem>,
    selected: &Option<usize>,
    default_selected: &usize,
    orientation: &SelectionOrientation,
    activation: &SelectionActivationMode,
    wrap: &bool,
    on_change: &Option<TabChangeCallback>,
    style: &TabsStyle,
    id_prefix: &String,
) {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generated_id = cx.use_memo(|| {
        format!(
            "tabs-{}",
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    });
    let id_prefix = if id_prefix.is_empty() {
        generated_id.get()
    } else {
        id_prefix
    };

    let initial_selection =
        normalized_selection(items, selected.unwrap_or(*default_selected)).unwrap_or(0);
    let internal_selection = cx.use_state(|| initial_selection);
    let requested = selected.unwrap_or(*internal_selection.get());
    let active = normalized_selection(items, requested);
    let controlled = selected.is_some();

    let handle_dependencies = tab_ids(items);
    let handle_count = items.len();
    let focus_handles =
        cx.use_memo_with(handle_dependencies, move || new_focus_handles(handle_count));

    let orientation = *orientation;
    let automatic = *activation == SelectionActivationMode::Automatic;
    let wrap = *wrap;
    let first_enabled = items.iter().position(|item| !item.disabled);
    let last_enabled = items.iter().rposition(|item| !item.disabled);
    let mut tab_elements = Vec::with_capacity(items.len());

    for (index, item) in items.iter().enumerate() {
        let is_active = active == Some(index);
        let tab_id = format!("{id_prefix}-tab-{}", item.id);
        let panel_id = format!("{id_prefix}-panel-{}", item.id);
        let previous = if item.disabled || (!wrap && Some(index) == first_enabled) {
            None
        } else {
            adjacent_enabled(items, index, -1)
        };
        let next = if item.disabled || (!wrap && Some(index) == last_enabled) {
            None
        } else {
            adjacent_enabled(items, index, 1)
        };
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

        let tab = view! {
            container(
                key: format!("tab-{}", item.id),
                style: tab_style,
                transition: Transition::new(Duration::from_millis(150)),
                flex_direction: FlexDirectionStyle::Row,
                focusable: !disabled,
                tab_index: if is_active && !disabled { 0 } else { -1 },
                focus_handle: focus_handles.get()[index].clone(),
                accessibility_role: AccessibilityRole::Tab,
                accessibility_id: tab_id,
                accessibility_selected: is_active,
                accessibility_disabled: disabled,
                accessibility_controls: panel_id,
                shortcuts: [(
                        Shortcut::named(if orientation == SelectionOrientation::Horizontal { NamedKey::ArrowLeft } else { NamedKey::ArrowUp }).allow_repeat(true),
                        PREVIOUS_TAB,
                    ),(
                        Shortcut::named(if orientation == SelectionOrientation::Horizontal { NamedKey::ArrowRight } else { NamedKey::ArrowDown }).allow_repeat(true),
                        NEXT_TAB,
                    ),(Shortcut::named(NamedKey::Home), FIRST_TAB),(Shortcut::named(NamedKey::End), LAST_TAB)].map(|(shortcut, command)| ShortcutBinding { shortcut, command }).to_vec(),
                on_click: move |_, event_cx| {
                    if disabled {
                        return EventResult::Ignored;
                    }
                    event_cx.request_focus();
                    activate_tab(index, controlled, state, &callback);
                    EventResult::Consumed
                },
                on_command: {
                    let callback = on_change.clone();
                    move |event, event_cx| {
                        if disabled { return EventResult::Ignored; }
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
                        if automatic { activate_tab(target, controlled, state, &callback); }
                        EventResult::Consumed
                    }
                },
            ) {
                ..item.label.clone()
            }
        };

        tab_elements.push(tab);
    }

    let list_style = if orientation == SelectionOrientation::Vertical {
        style.list.clone().with::<container::height>(Sizing::Hug)
    } else {
        style.list.clone()
    };
    let tab_list = view! {
        container(
            style: list_style,
            flex_direction: if orientation == SelectionOrientation::Horizontal { FlexDirectionStyle::Row } else { FlexDirectionStyle::Column },
            accessibility_role: AccessibilityRole::TabList,
        ) {
            ..tab_elements
        }
    };

    let mut children = vec![tab_list];
    if let Some(active) = active {
        let item = &items[active];
        children.push(view! {
            container(
                key: format!("panel-{}", item.id),
                style: style.panel.clone(),
                accessibility_role: AccessibilityRole::TabPanel,
                accessibility_id: format!("{id_prefix}-panel-{}", item.id),
                accessibility_labelled_by: format!("{id_prefix}-tab-{}", item.id),
            ) {
                ..item.content.clone()
            }
        });
    }

    view! {
        container(
            style: style.root.clone(),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            ..children
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, disabled: bool) -> TabItem {
        TabItem::new(
            id,
            id.to_string(),
            view! {
                xui_core::prelude::text(
                    id.to_string(),
                )
            },
        )
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
