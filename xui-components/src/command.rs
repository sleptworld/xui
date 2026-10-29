//! Externally filtered command discovery in a dialog.
//!
//! [`CommandItem`] defines identity, label, keywords, display shortcut, and disabled
//! state. [`command_palette`] filters a caller-supplied query and reports selection by
//! original item index; it does not include an editable search field.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`command_palette`](fn@self::command_palette) | A dialog listing commands filtered by a caller-supplied query. |

use xui_core::prelude::*;
use xui_macros::component;

use crate::dialog;

/// An identified command with searchable keywords, a display shortcut, and disabled
/// state.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct CommandItem {
    pub id: String,
    pub label: String,
    pub keywords: Vec<String>,
    pub shortcut: Option<String>,
    pub disabled: bool,
}

impl CommandItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            keywords: Vec::new(),
            shortcut: None,
            disabled: false,
        }
    }

    pub fn keywords(mut self, keywords: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keywords = keywords.into_iter().map(Into::into).collect();
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

fn command_matches(item: &CommandItem, query: &str) -> bool {
    query.is_empty()
        || item.label.to_lowercase().contains(query)
        || item
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(query))
}

/// A dialog listing commands filtered by a caller-supplied query.
///
/// `open` and `query` are caller-controlled. Matching searches labels and keywords
/// without case sensitivity; the component does not render an editable search field.
/// Arrow/Home/End navigation skips disabled and filtered items, and selecting a command
/// notifies `on_select` with its original index before requesting closure. `style`
/// customizes the result list.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<CommandItem>` | **Required** |
/// | `query` | `String` | `String::new()` |
/// | `open` | `bool` | `false` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `on_select` | `Option<MenuSelectCallback>` | `None` |
/// | `empty_text` | `String` | `"No commands found".to_string()` |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { command_palette(items: vec![CommandItem::new("open", "Open file").keywords(["document"])], query: "file", open: true) }
/// }
/// ```
#[component]
#[defaults(
    query = String::new(),
    open = false,
    empty_text = "No commands found".to_string(),
    style = StyleOf::new(),
)]
pub fn command_palette(
    #[source] items: &Vec<CommandItem>,
    #[source] query: &String,
    #[source] open: &bool,
    on_open_change: &Callback<bool>,
    on_select: &Callback<usize>,
    #[source] empty_text: &String,
    style: &StyleOf<Container>,
) {
    let normalized_query: String = memo!(query.trim().to_lowercase());
    let handles: Vec<FocusHandle> = memo!((0..items.len()).map(|_| FocusHandle::new()).collect());
    let model: crate::SelectionModel = memo!(crate::SelectionModel::new(items.iter().map(|item| {
        crate::SelectionItem::new(
            item.id.clone(),
            item.label.clone(),
            item.disabled || !command_matches(item, normalized_query),
        )
    })));
    // The commands the query matches, with their index among all of them.
    let visible: Vec<(usize, CommandItem)> = memo!(
        items
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, item)| command_matches(item, normalized_query))
            .collect()
    );
    let mut content_style: StyleOf<Container> = StyleOf::new()
        .with::<container::min_width>(420.0)
        .with::<container::max_height>(480.0)
        .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical);
    content_style.merge(style);
    view! {
        container() {
            dialog(
                open: Some(*open),
                title: Some("Command palette".to_owned()),
                on_open_change: on_open_change,
            ) {
                container(
                    style: content_style.clone(),
                    flex_direction: FlexDirectionStyle::Column,
                    accessibility_role: AccessibilityRole::ListBox,
                    accessibility_label: "Commands",
                ) {
                    text(
                        if query.is_empty() {
                            "Type to filter commands".to_owned()
                        } else {
                            format!("Results for “{query}”")
                        },
                        style: StyleOf::new()
                            .with::<container::padding>(EdgeInsets::all(10.0))
                            .with::<container::font_size>(FontSizeToken::Sm),
                    )
                    if visible.is_empty() {
                        container(
                            style: StyleOf::new().with::<container::padding>(EdgeInsets::all(16.0)),
                            accessibility_live_region: AccessibilityLiveRegion::Polite,
                        ) {
                            text(empty_text.clone())
                        }
                    }
                    for (index, item) in visible.clone() {
                        container(
                            style: StyleOf::new()
                                .with::<container::padding>(EdgeInsets::all(10.0))
                                .with::<container::justify>(JustifyStyle::SpaceBetween)
                                .with::<container::border_radius>(RadiusToken::Sm)
                                .when_style(
                                    WidgetStateMatcher::all(WidgetState::HOVERED),
                                    StyleOf::<Container>::new().with::<container::background>(ColorToken::MutedSurface),
                                ),
                            flex_direction: FlexDirectionStyle::Row,
                            focus_handle: handles[*index].clone(),
                            shortcuts: crate::selection::selection_shortcuts()
                                .map(|(shortcut, command)| ShortcutBinding { shortcut, command })
                                .to_vec(),
                            on_command: |_cx, event, event_cx| {
                                if item.disabled {
                                    return EventResult::Ignored;
                                }
                                let Some(target) = crate::selection::selection_target(&model.read(), *index, event.command) else {
                                    return EventResult::Ignored;
                                };
                                handles.read()[target].request_focus(event_cx);
                                EventResult::Consumed
                            },
                            focusable: !item.disabled,
                            tab_index: if item.disabled { -1 } else { 0 },
                            accessibility_role: AccessibilityRole::Option,
                            accessibility_label: item.label.clone(),
                            accessibility_disabled: item.disabled,
                            on_click: |cx| {
                                if item.disabled {
                                    return EventResult::Ignored;
                                }
                                on_select.call(cx, *index);
                                on_open_change.call(cx, false);
                                EventResult::Consumed
                            },
                        ) {
                            text(item.label.clone())
                            if let Some(shortcut) = item.shortcut.clone() {
                                text(shortcut.clone(), style: StyleOf::new().with::<container::font_size>(FontSizeToken::Sm))
                            }
                        }
                        .key(item.id.clone())
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
    fn command_filter_checks_labels_and_keywords_case_insensitively() {
        let item = CommandItem::new("open", "Open File").keywords(["document", "load"]);
        assert!(command_matches(&item, "file"));
        assert!(command_matches(&item, "doc"));
        assert!(!command_matches(&item, "save"));
    }
}
