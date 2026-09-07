use xui::prelude::*;

use crate::{BoolChangeCallback, MenuSelectCallback, dialog};

#[derive(Clone, Debug, Hash)]
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
    let query = query.trim().to_lowercase();
    query.is_empty()
        || item.label.to_lowercase().contains(&query)
        || item
            .keywords
            .iter()
            .any(|keyword| keyword.to_lowercase().contains(&query))
}

#[component]
#[defaults(
    query = String::new(),
    open = false,
    on_open_change = None,
    on_select = None,
    empty_text = "No commands found".to_string(),
    style = Style::new(),
)]
pub fn command_palette(
    items: &Vec<CommandItem>,
    query: &String,
    open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    on_select: &Option<MenuSelectCallback>,
    empty_text: &String,
    style: &Style,
) {
    let visible = items
        .iter()
        .enumerate()
        .filter(|(_, item)| command_matches(item, query))
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(visible.len().max(1) + 1);
    rows.push(
        TextWidget::new(if query.is_empty() {
            "Type to filter commands".to_owned()
        } else {
            format!("Results for “{query}”")
        })
        .style(
            Style::new()
                .padding(EdgeInsets::all(10.0))
                .font_size(FontSizeToken::Sm),
        )
        .into_element_desc(),
    );
    if visible.is_empty() {
        rows.push(
            ContainerWidget::new()
                .style(Style::new().padding(EdgeInsets::all(16.0)))
                .accessibility_live_region(AccessibilityLiveRegion::Polite)
                .into_element_desc(vec![
                    TextWidget::new(empty_text.clone()).into_element_desc(),
                ]),
        );
    } else {
        for (index, item) in visible {
            let selected = on_select.clone();
            let close = on_open_change.clone();
            let disabled = item.disabled;
            let mut children = vec![TextWidget::new(item.label.clone()).into_element_desc()];
            if let Some(shortcut) = &item.shortcut {
                children.push(
                    TextWidget::new(shortcut.clone())
                        .style(Style::new().font_size(FontSizeToken::Sm))
                        .into_element_desc(),
                );
            }
            let mut row = ContainerWidget::new()
                .style(
                    Style::new()
                        .padding(EdgeInsets::all(10.0))
                        .justify(JustifyStyle::SpaceBetween)
                        .border_radius(RadiusToken::Sm)
                        .when(WidgetState::HOVERED, |style| {
                            style.background(ColorToken::MutedSurface)
                        }),
                )
                .flex_direction(FlexDirectionStyle::Row)
                .focusable(!disabled)
                .tab_index(if disabled { -1 } else { 0 })
                .accessibility_role(AccessibilityRole::Option)
                .accessibility_label(item.label.clone())
                .accessibility_disabled(disabled);
            if !disabled {
                row = row.on_click(move |_, _| {
                    if let Some(callback) = &selected {
                        callback.call(index);
                    }
                    if let Some(callback) = &close {
                        callback.call(false);
                    }
                    EventResult::Consumed
                });
            }
            rows.push(row.into_element_desc(children));
        }
    }
    let mut content_style = Style::new()
        .min_width(420.0)
        .max_height(480.0)
        .scroll_vertical();
    content_style.merge(style);
    let content = ContainerWidget::new()
        .style(content_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::ListBox)
        .accessibility_label("Commands")
        .into_element_desc(rows);
    xui! {
        <dialog
            content={content}
            open={Some(*open)}
            title={Some("Command palette".to_owned())}
            on_open_change={on_open_change.clone()}
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_filter_checks_labels_and_keywords_case_insensitively() {
        let item = CommandItem::new("open", "Open File").keywords(["document", "load"]);
        assert!(command_matches(&item, "FILE"));
        assert!(command_matches(&item, "doc"));
        assert!(!command_matches(&item, "save"));
    }
}
