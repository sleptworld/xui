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
use xui_core::prelude::*;
use xui_macros::component;

const PREVIOUS_TAB: CommandId = CommandId("xui.tabs.previous");
const NEXT_TAB: CommandId = CommandId("xui.tabs.next");
const FIRST_TAB: CommandId = CommandId("xui.tabs.first");
const LAST_TAB: CommandId = CommandId("xui.tabs.last");

/// Callback requesting a zero-based active tab index.
pub type TabChangeCallback = Callback<usize>;

/// One selectable tab and the content displayed while it is active.
#[derive(Clone, Debug, PartialEq)]
pub struct TabItem {
    pub id: String,
    pub label: Children,
    pub content: Children,
    pub disabled: bool,
}

impl TabItem {
    /// `content` is text, or a component: `Children::component::<Panel>(PanelProps { .. })`.
    pub fn new(id: impl Into<String>, label: impl Into<Children>, content: impl Into<Children>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            content: content.into(),
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
    cx: &mut Cx<'_>,
    index: usize,
    selected: &ReadSignal<Option<usize>>,
    own: &Signal<usize>,
    on_change: &Callback<usize>,
) {
    if selected.with(Option::is_none) {
        own.set(cx, index);
    }
    on_change.call(cx, index);
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
/// use xui_macros::component;
///
/// #[component]
/// fn example() {
///     view! { tabs(items: vec![TabItem::new("account", "Account", "Account settings")]) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<usize>,
    default_selected = 0,
    orientation = SelectionOrientation::Horizontal,
    activation = SelectionActivationMode::Automatic,
    wrap = true,
    style = TabsStyle::default(),
    id_prefix = String::new(),
)]
pub fn tabs(
    #[source] items: &Vec<TabItem>,
    #[source] selected: &Option<usize>,
    default_selected: &usize,
    orientation: &SelectionOrientation,
    activation: &SelectionActivationMode,
    wrap: &bool,
    on_change: &Callback<usize>,
    style: &TabsStyle,
    id_prefix: &String,
) {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id_base: String = if id_prefix.is_empty() {
        format!("tabs-{}", NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    } else {
        id_prefix.clone()
    };
    let own: usize = state!(
        normalized_selection(&items.read(), selected.get().unwrap_or(*default_selected)).unwrap_or(0)
    );
    let active: Option<usize> = memo!(normalized_selection(items, selected.unwrap_or(*own)));
    let handles: Vec<FocusHandle> = memo!((0..items.len()).map(|_| FocusHandle::new()).collect());
    // The active item, keyed below so a different tab mounts its own panel.
    let panel: Option<TabItem> = memo!(active.and_then(|index| items.get(index).cloned()));
    let horizontal: bool = *orientation == SelectionOrientation::Horizontal;
    let shortcuts: Vec<ShortcutBinding> = [
        (Shortcut::named(if horizontal { NamedKey::ArrowLeft } else { NamedKey::ArrowUp }).allow_repeat(true), PREVIOUS_TAB),
        (Shortcut::named(if horizontal { NamedKey::ArrowRight } else { NamedKey::ArrowDown }).allow_repeat(true), NEXT_TAB),
        (Shortcut::named(NamedKey::Home), FIRST_TAB),
        (Shortcut::named(NamedKey::End), LAST_TAB),
    ]
    .map(|(shortcut, command)| ShortcutBinding { shortcut, command })
    .to_vec();
    let list_style: StyleOf<Container> = if horizontal {
        style.list.clone()
    } else {
        style.list.clone().with::<container::height>(Sizing::Hug)
    };
    view! {
        container(
            style: style.root.clone(),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            container(
                style: list_style.clone(),
                flex_direction: if *horizontal { FlexDirectionStyle::Row } else { FlexDirectionStyle::Column },
                accessibility_role: AccessibilityRole::TabList,
            ) {
                for (index, item) in items.iter().cloned().enumerate() {
                    container(
                        style: if item.disabled {
                            style.disabled_tab.clone()
                        } else if *active == Some(*index) {
                            style.selected_tab.clone()
                        } else {
                            style.tab.clone()
                        },
                        transition: Transition::new(Duration::from_millis(150)),
                        flex_direction: FlexDirectionStyle::Row,
                        focusable: !item.disabled,
                        tab_index: if *active == Some(*index) && !item.disabled { 0 } else { -1 },
                        focus_handle: handles[*index].clone(),
                        accessibility_role: AccessibilityRole::Tab,
                        accessibility_id: format!("{id_base}-tab-{}", item.id),
                        accessibility_selected: *active == Some(*index),
                        accessibility_disabled: item.disabled,
                        accessibility_controls: format!("{id_base}-panel-{}", item.id),
                        shortcuts: shortcuts.clone(),
                        on_click: |cx, _event, event_cx| {
                            if item.disabled {
                                return EventResult::Ignored;
                            }
                            event_cx.request_focus();
                            activate_tab(cx, *index, selected, own, on_change);
                            EventResult::Consumed
                        },
                        on_command: |cx, event, event_cx| {
                            if item.disabled {
                                return EventResult::Ignored;
                            }
                            let target = {
                                let items = items.read();
                                let first = items.iter().position(|item| !item.disabled);
                                let last = items.iter().rposition(|item| !item.disabled);
                                match event.command {
                                    PREVIOUS_TAB if !*wrap && Some(*index) == first => None,
                                    PREVIOUS_TAB => adjacent_enabled(&items, *index, -1),
                                    NEXT_TAB if !*wrap && Some(*index) == last => None,
                                    NEXT_TAB => adjacent_enabled(&items, *index, 1),
                                    FIRST_TAB => first,
                                    LAST_TAB => last,
                                    _ => return EventResult::Ignored,
                                }
                            };
                            let Some(target) = target else {
                                return EventResult::Ignored;
                            };
                            handles.read()[target].request_focus(event_cx);
                            if *activation == SelectionActivationMode::Automatic {
                                activate_tab(cx, target, selected, own, on_change);
                            }
                            EventResult::Consumed
                        },
                    ) {
                        slot(item.label)
                    }
                    .key(item.id.clone())
                }
            }
            for item in panel.clone() {
                container(
                    style: style.panel.clone(),
                    accessibility_role: AccessibilityRole::TabPanel,
                    accessibility_id: format!("{id_base}-panel-{}", item.id),
                    accessibility_labelled_by: format!("{id_base}-tab-{}", item.id),
                ) {
                    slot(item.content)
                }
                .key(item.id.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, disabled: bool) -> TabItem {
        TabItem::new(id, id.to_string(), id.to_string()).disabled(disabled)
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
