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

use crate::{SelectionItem, SelectionModel};
use std::rc::Rc;
use xui_core::prelude::*;
use xui_macros::{component, view};
const PREVIOUS_OPTION: CommandId = CommandId("xui.dropdown.previous");
const NEXT_OPTION: CommandId = CommandId("xui.dropdown.next");
const FIRST_OPTION: CommandId = CommandId("xui.dropdown.first");
const LAST_OPTION: CommandId = CommandId("xui.dropdown.last");

/// Callback requesting a zero-based dropdown option index.
pub type DropDownChangeCallback = Callback<usize>;

/// An identified dropdown option with display text and disabled state.
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

/// Complete styles for the dropdown root, trigger, menu, and option states.
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
    view! {
        icon(
            from_icon_data: data,
            size: 16.0,
            style: style,
        )
    }
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
/// use xui_macros::view;
///
/// let _ = view! { drop_down(items: vec![DropDownItem::new("a", "Alpha"), DropDownItem::new("b", "Beta")]) };
/// ```
#[component]
#[defaults(
    selected = None,
    default_selected = 0,
    open = None,
    default_open = false,
    on_open_change = None,
    placement = AnchorPlacement::default().match_width(true),
    on_change = None,
    placeholder = "Select…".to_string(),
    disabled = false,
    style = DropDownStyle::default(),
    id_prefix = String::new(),
    z_index = 1000,
)]
pub fn drop_down(
    items: &Vec<DropDownItem>,
    selected: &Option<usize>,
    default_selected: &usize,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<crate::BoolChangeCallback>,
    placement: &AnchorPlacement,
    on_change: &Option<DropDownChangeCallback>,
    placeholder: &String,
    disabled: &bool,
    style: &DropDownStyle,
    id_prefix: &String,
    z_index: &i32,
) {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generated_id = cx.use_memo(|| {
        format!(
            "dropdown-{}",
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )
    });
    let id_prefix = if id_prefix.is_empty() {
        generated_id.get()
    } else {
        id_prefix
    };

    let initial = normalized_selection(items, selected.unwrap_or(*default_selected)).unwrap_or(0);
    let internal_selection = cx.use_state(|| initial);
    let internal_open = cx.use_state(|| *default_open);
    let open_controlled = open.is_some();
    let shown = !*disabled && open.unwrap_or(*internal_open.get());
    let set_open = cx.use_callback((open_controlled, internal_open, on_open_change.clone()), {
        let callback = on_open_change.clone();
        move |next| {
            if !open_controlled {
                internal_open.set(next);
            }
            if let Some(callback) = &callback {
                callback.call(next);
            }
        }
    });
    let model = Rc::new(SelectionModel::new(items.iter().map(|item| {
        SelectionItem::new(item.id.clone(), item.label.clone(), item.disabled)
    })));
    let nav_model = model.clone();
    let nav_callback = on_change.clone();
    let nav_disabled = *disabled;

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
    if shown {
        trigger_style.merge(&style.trigger_open);
    }

    let mut trigger_children = vec![
        view! {
            xui_core::widgets::text(
                label.clone(),
                style: if has_selection {
                    Style::new()
                } else {
                    Style::new().color(ColorToken::MutedForeground)
                },
            )
        },
        indicator(
            chevron_down_icon(),
            Style::new().color(ColorToken::MutedForeground),
        ),
    ];
    if shown {
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
            let close = set_open.clone();
            let callback = on_change.clone();
            options.push(view! {
                container(
                    key: format!("{id_prefix}-option-{}", item.id),
                    style: option_style,
                    flex_direction: FlexDirectionStyle::Row,
                    focusable: !disabled,
                    tab_index: if disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::Option,
                    accessibility_label: item.label.clone(),
                    accessibility_selected: is_selected,
                    accessibility_disabled: disabled,
                    on_click: move |_, _| {
                        if disabled {
                            return EventResult::Consumed;
                        }
                        if !controlled { selection.set(index); }
                        close.call(false);
                        if let Some(callback) = &callback { callback.call(index); }
                        EventResult::Consumed
                    },
                ) {
                    xui_core::widgets::text(
                        item.label.clone(),
                    )
                    ..// Keep the slot when unselected so labels don't shift.
                    indicator(
                        check_icon(),
                        if is_selected {
                            Style::new()
                        } else {
                            Style::new().color(Color::TRANSPARENT)
                        },
                    )
                }
            });
        }

        let menu = view! {
            container(
                key: format!("{id_prefix}-menu"),
                style: style.menu.clone(),
                flex_direction: FlexDirectionStyle::Column,
                accessibility_role: AccessibilityRole::ListBox,
                accessibility_id: format!("{id_prefix}-menu"),
            ) {
                ..options
            }
        };

        // Written inside the trigger, so the trigger owns the portal and the
        // menu follows it through resizes, layout changes and scrolling. Modal:
        // a press outside reaches nothing but the dismiss handler, which is
        // also what keeps the trigger from reopening it.
        trigger_children.push(view! {
            xui_core::element::portal_view(
                key: format!("{id_prefix}-portal"),
                z_index: *z_index,
                modal: true,
                anchor: *placement,
                on_dismiss: { let set_open = set_open.clone(); move |_| set_open.call(false) },
            ) {
                ..menu
            }
        });
    }

    view! {
        container(
            style: style.root.clone(),
        ) {
            container(
                key: format!("{id_prefix}-trigger"),
                style: trigger_style,
                flex_direction: FlexDirectionStyle::Row,
                focusable: !*disabled,
                tab_index: if *disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::ComboBox,
                accessibility_expanded: shown,
                accessibility_id: format!("{id_prefix}-trigger"),
                accessibility_label: label.clone(),
                accessibility_disabled: *disabled,
                accessibility_controls: format!("{id_prefix}-menu"),
                shortcuts: [
                    (Shortcut::named(NamedKey::ArrowUp).allow_repeat(true), PREVIOUS_OPTION),
                    (Shortcut::named(NamedKey::ArrowDown).allow_repeat(true), NEXT_OPTION),
                    (Shortcut::named(NamedKey::Home), FIRST_OPTION), (Shortcut::named(NamedKey::End), LAST_OPTION),
                ],
                on_command: move |event, _| {
                    if nav_disabled { return EventResult::Ignored; }
                    let target = match event.command {
                        PREVIOUS_OPTION => nav_model.adjacent(active.unwrap_or(0), -1),
                        NEXT_OPTION => nav_model.adjacent(active.unwrap_or(0), 1),
                        FIRST_OPTION => nav_model.first(), LAST_OPTION => nav_model.last(),
                        _ => return EventResult::Ignored,
                    };
                    if let Some(target) = target {
                        if !controlled { internal_selection.set(target); }
                        if let Some(callback) = &nav_callback { callback.call(target); }
                    }
                    EventResult::Consumed
                },
                on_click: {
                    let disabled = *disabled;
                    move |_, event_cx| {
                        if disabled {
                            return EventResult::Ignored;
                        }
                        set_open.call(!shown);
                        event_cx.request_focus();
                        EventResult::Consumed
                    }
                },
            ) {
                ..trigger_children
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
