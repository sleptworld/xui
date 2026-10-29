//! Option selection with independently controlled value and disclosure.
//!
//! [`drop_down`] renders its option list in an anchored portal. [`DropDownItem`] supplies
//! identity/disabled state and [`DropDownStyle`] defines complete visual slots.
//! Crate-root `select` and `combo_box` are aliases for the same non-editable picker.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`drop_down`](fn@self::drop_down) | An accessible option picker with a portal-mounted dropdown list. |

use crate::controls::commit;
use crate::{SelectionItem, SelectionModel};
use xui_core::prelude::*;
use xui_macros::component;
const PREVIOUS_OPTION: CommandId = CommandId("xui.dropdown.previous");
const NEXT_OPTION: CommandId = CommandId("xui.dropdown.next");
const FIRST_OPTION: CommandId = CommandId("xui.dropdown.first");
const LAST_OPTION: CommandId = CommandId("xui.dropdown.last");

/// Callback requesting a zero-based dropdown option index.
pub type DropDownChangeCallback = Callback<usize>;

/// An identified dropdown option with display text and disabled state.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
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

/// Complete styles for the dropdown root, trigger, menu, and option states.
#[derive(Clone, Debug)]
pub struct DropDownStyle {
    pub root: StyleOf<Container>,
    pub trigger: StyleOf<Container>,
    pub trigger_open: StyleOf<Container>,
    pub menu: StyleOf<Container>,
    pub option: StyleOf<Container>,
    pub selected_option: StyleOf<Container>,
    pub disabled_option: StyleOf<Container>,
}

impl Default for DropDownStyle {
    fn default() -> Self {
        let focus_ring = ShadowStyle::new()
            .color(ColorToken::Ring.alpha(0.5))
            .blur(0.0)
            .spread(3.0);
        let option = StyleOf::new()
            .with::<container::width>(Sizing::fill())
            .with::<container::gap>(8.0)
            .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 6.0, 6.0))
            .with::<container::align>(AlignStyle::Center)
            .with::<container::justify>(JustifyStyle::SpaceBetween)
            .with::<container::border_radius>(RadiusToken::Sm)
            .with::<container::font_size>(FontSizeToken::Md)
            .with::<container::color>(ColorToken::PopoverForeground)
            .when_style(
                WidgetStateMatcher::all(WidgetState::HOVERED),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Accent)
                    .with::<container::color>(ColorToken::AccentForeground),
            )
            .when_style(
                WidgetStateMatcher::all(WidgetState::FOCUS_VISIBLE),
                StyleOf::<Container>::new()
                    .with::<container::background>(ColorToken::Accent)
                    .with::<container::color>(ColorToken::AccentForeground),
            );
        Self {
            root: StyleOf::new().with::<container::min_width>(180.0),
            trigger: StyleOf::new()
                .with::<container::height>(36.0)
                .with::<container::gap>(8.0)
                .with::<container::padding>(EdgeInsets::symmetric(12.0, 8.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween)
                .with::<container::background>(ColorToken::Background)
                .with::<container::color>(ColorToken::Foreground)
                .with::<container::font_size>(FontSizeToken::Md)
                .with::<container::border_color>(ColorToken::Input)
                .with::<container::border_width>(1.0)
                .with::<container::border_radius>(RadiusToken::Md)
                .with::<container::shadow>(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.05))
                        .offset(Point::new(0.0, 1.0))
                        .blur(2.0),
                )
                .when_style(
                    WidgetStateMatcher::all(WidgetState::FOCUS_VISIBLE),
                    StyleOf::<Container>::new()
                        .with::<container::border_color>(ColorToken::Ring)
                        .with::<container::shadow>(focus_ring),
                ),
            // Like the focus ring, shown only for keyboard focus, not because
            // the menu is open.
            trigger_open: StyleOf::new(),
            menu: StyleOf::new()
                .with::<container::padding>(EdgeInsets::all(4.0))
                .with::<container::background>(ColorToken::Popover)
                .with::<container::color>(ColorToken::PopoverForeground)
                .with::<container::border_color>(ColorToken::Border)
                .with::<container::border_width>(1.0)
                .with::<container::border_radius>(RadiusToken::Md)
                .with::<container::shadow>(
                    ShadowStyle::new()
                        .color(Color::rgba(0.0, 0.0, 0.0, 0.1))
                        .offset(Point::new(0.0, 4.0))
                        .blur(6.0)
                        .spread(-1.0),
                )
                .with::<container::max_height>(280.0)
                .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
            selected_option: option.clone(),
            disabled_option: StyleOf::new()
                .with::<container::width>(Sizing::fill())
                .with::<container::gap>(8.0)
                .with::<container::padding>(EdgeInsets::new(8.0, 8.0, 6.0, 6.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::SpaceBetween)
                .with::<container::border_radius>(RadiusToken::Sm)
                .with::<container::font_size>(FontSizeToken::Md)
                .with::<container::color>(ColorToken::PopoverForeground.alpha(0.5)),
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



fn normalized_selection(items: &[DropDownItem], requested: usize) -> Option<usize> {
    items
        .get(requested)
        .filter(|item| !item.disabled)
        .map(|_| requested)
        .or_else(|| items.iter().position(|item| !item.disabled))
}

/// An accessible option picker with a portal-mounted dropdown list.
///
/// `selected` and `open` opt into controlled state independently; otherwise
/// `default_selected` and `default_open` seed internal state. Change callbacks report
/// requested values in either mode. Disabled options are skipped by arrow/Home/End
/// navigation. `placement` controls anchoring, width matching, and collision avoidance;
/// [`DropDownStyle`] contains complete, independently replaceable visual slots. The
/// default accessibility ID prefix is unique and stable per instance.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<DropDownItem>` | **Required** |
/// | `selected` | `Option<usize>` | `None` |
/// | `default_selected` | `usize` | `0` |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<crate::BoolChangeCallback>` | `None` |
/// | `placement` | `AnchorPlacement` | `AnchorPlacement::default().match_width(true)` |
/// | `on_change` | `Option<DropDownChangeCallback>` | `None` |
/// | `placeholder` | `String` | `"Select…".to_string()` |
/// | `disabled` | `bool` | `false` |
/// | `style` | `DropDownStyle` | `DropDownStyle::default()` |
/// | `id_prefix` | `String` | `String::new()` |
/// | `z_index` | `i32` | `1000` |
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
///     view! { drop_down(items: vec![DropDownItem::new("a", "Alpha"), DropDownItem::new("b", "Beta")]) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<usize>,
    default_selected = 0,
    open = None::<bool>,
    default_open = false,
    placement = AnchorPlacement::default().match_width(true),
    placeholder = "Select…".to_string(),
    disabled = false,
    style = DropDownStyle::default(),
    id_prefix = String::new(),
    z_index = 1000,
)]
pub fn drop_down(
    #[source] items: &Vec<DropDownItem>,
    #[source] selected: &Option<usize>,
    default_selected: &usize,
    #[source] open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Callback<bool>,
    placement: &AnchorPlacement,
    on_change: &Callback<usize>,
    #[source] placeholder: &String,
    #[source] disabled: &bool,
    style: &DropDownStyle,
    id_prefix: &String,
    z_index: &i32,
) {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id_base: String = if id_prefix.is_empty() {
        format!("dropdown-{}", NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    } else {
        id_prefix.clone()
    };
    let own: usize = state!(
        normalized_selection(&items.read(), selected.get().unwrap_or(*default_selected)).unwrap_or(0)
    );
    let own_open: bool = state!(*default_open);
    let shown: bool = memo!(!*disabled && open.unwrap_or(*own_open));
    let model: SelectionModel = memo!(SelectionModel::new(
        items.iter().map(|item| SelectionItem::new(item.id.clone(), item.label.clone(), item.disabled))
    ));
    let active: Option<usize> = memo!(normalized_selection(items, selected.unwrap_or(*own)));
    let selected_label: Option<String> = memo!(active.and_then(|index| items.get(index)).map(|item| item.label.clone()));
    let trigger_style: StyleOf<Container> = memo!({
        let mut trigger_style = style.trigger.clone();
        if *disabled {
            trigger_style = trigger_style
                .with::<container::color>(ColorToken::Foreground.alpha(0.5))
                .with::<container::border_color>(ColorToken::Input.alpha(0.5));
        }
        if *shown {
            trigger_style.merge(&style.trigger_open);
        }
        trigger_style
    });
    view! {
        container(style: style.root.clone()) {
            container(
                style: trigger_style.clone(),
                flex_direction: FlexDirectionStyle::Row,
                focusable: !*disabled,
                tab_index: if *disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::ComboBox,
                accessibility_expanded: *shown,
                accessibility_id: format!("{id_base}-trigger"),
                accessibility_label: selected_label.clone().unwrap_or_else(|| placeholder.clone()),
                accessibility_disabled: *disabled,
                accessibility_controls: format!("{id_base}-menu"),
                shortcuts: [
                    (Shortcut::named(NamedKey::ArrowUp).allow_repeat(true), PREVIOUS_OPTION),
                    (Shortcut::named(NamedKey::ArrowDown).allow_repeat(true), NEXT_OPTION),
                    (Shortcut::named(NamedKey::Home), FIRST_OPTION),
                    (Shortcut::named(NamedKey::End), LAST_OPTION),
                ]
                .map(|(shortcut, command)| ShortcutBinding { shortcut, command })
                .to_vec(),
                on_command: |cx, event| {
                    if disabled.get() {
                        return EventResult::Ignored;
                    }
                    let (model, current) = (model.get(), active.get().unwrap_or(0));
                    let target = match event.command {
                        PREVIOUS_OPTION => model.adjacent(current, -1),
                        NEXT_OPTION => model.adjacent(current, 1),
                        FIRST_OPTION => model.first(),
                        LAST_OPTION => model.last(),
                        _ => return EventResult::Ignored,
                    };
                    if let Some(target) = target {
                        commit(cx, target, selected, own, on_change);
                    }
                    EventResult::Consumed
                },
                on_click: |cx, _event, event_cx| {
                    if disabled.get() {
                        return EventResult::Ignored;
                    }
                    commit(cx, !shown.get(), open, own_open, on_open_change);
                    event_cx.request_focus();
                    EventResult::Consumed
                },
            ) {
                text(
                    selected_label.clone().unwrap_or_else(|| placeholder.clone()),
                    style: if selected_label.is_some() {
                        StyleOf::new()
                    } else {
                        StyleOf::new().with::<container::color>(ColorToken::MutedForeground)
                    },
                )
                container(style: StyleOf::new().with::<container::color>(ColorToken::MutedForeground)) {
                    icon(data: chevron_down_icon(), size: Size::fix(16.0, 16.0))
                }
                // Written inside the trigger, so the trigger owns the portal and
                // the menu follows it through resizes, layout changes and
                // scrolling. Modal: a press outside reaches nothing but the
                // dismiss handler, which is also what keeps the trigger from
                // reopening it.
                if *shown {
                    portal(
                        z_index: *z_index,
                        modal: true,
                        anchor: *placement,
                        on_dismiss: |cx, _reason| commit(cx, false, open, own_open, on_open_change),
                    ) {
                        container(
                            style: style.menu.clone(),
                            flex_direction: FlexDirectionStyle::Column,
                            accessibility_role: AccessibilityRole::ListBox,
                            accessibility_id: format!("{id_base}-menu"),
                        ) {
                            for (index, item) in items.iter().cloned().enumerate() {
                                container(
                                    style: if item.disabled {
                                        style.disabled_option.clone()
                                    } else if *active == Some(*index) {
                                        style.selected_option.clone()
                                    } else {
                                        style.option.clone()
                                    },
                                    flex_direction: FlexDirectionStyle::Row,
                                    focusable: !item.disabled,
                                    tab_index: if item.disabled { -1 } else { 0 },
                                    accessibility_role: AccessibilityRole::Option,
                                    accessibility_label: item.label.clone(),
                                    accessibility_selected: *active == Some(*index),
                                    accessibility_disabled: item.disabled,
                                    on_click: |cx| {
                                        if !item.disabled {
                                            commit(cx, *index, selected, own, on_change);
                                            commit(cx, false, open, own_open, on_open_change);
                                        }
                                        EventResult::Consumed
                                    },
                                ) {
                                    text(item.label.clone())
                                    // Kept when unselected so labels don't shift.
                                    container(
                                        style: if *active == Some(*index) {
                                            StyleOf::new()
                                        } else {
                                            StyleOf::new().with::<container::color>(Color::TRANSPARENT)
                                        },
                                    ) {
                                        icon(data: check_icon(), size: Size::fix(16.0, 16.0))
                                    }
                                }
                                .key(item.id.clone())
                            }
                        }
                    }
                }
            }
        }
    }
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
