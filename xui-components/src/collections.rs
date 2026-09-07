use xui::prelude::*;

use crate::{BoolChangeCallback, IndexChangeCallback};

#[component]
#[defaults(open = None, default_open = false, on_open_change = None, style = Style::new())]
pub fn collapsible(
    trigger: &ElementDesc,
    content: &ElementDesc,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let callback = on_open_change.clone();
    let trigger = ContainerWidget::new()
        .focusable(true)
        .tab_index(0)
        .accessibility_role(AccessibilityRole::Button)
        .accessibility_expanded(shown)
        .on_click(move |_, event_cx| {
            let next = !shown;
            if !controlled {
                state.set(next);
            }
            if let Some(callback) = &callback {
                callback.call(next);
            }
            event_cx.request_focus();
            EventResult::Consumed
        })
        .into_element_desc(vec![trigger.clone()]);
    let mut children = vec![trigger];
    if shown {
        children.push(content.clone());
    }
    let mut root_style = Style::new();
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

#[derive(Clone, Debug)]
pub struct AccordionItem {
    pub id: String,
    pub title: ElementDesc,
    pub content: ElementDesc,
    pub disabled: bool,
}

impl AccordionItem {
    pub fn new(id: impl Into<String>, title: ElementDesc, content: ElementDesc) -> Self {
        Self {
            id: id.into(),
            title,
            content,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

pub type OptionalIndexChangeCallback = Callback<Option<usize>>;

#[component]
#[defaults(
    expanded = None,
    default_expanded = None,
    collapsible = true,
    on_change = None,
    style = Style::new(),
)]
pub fn accordion(
    items: &Vec<AccordionItem>,
    expanded: &Option<Option<usize>>,
    default_expanded: &Option<usize>,
    collapsible: &bool,
    on_change: &Option<OptionalIndexChangeCallback>,
    style: &Style,
) {
    let state = cx.use_state(|| *default_expanded);
    let controlled = expanded.is_some();
    let active = expanded.unwrap_or(*state.get());
    let mut children = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let shown = active == Some(index);
        let disabled = item.disabled;
        let callback = on_change.clone();
        let can_collapse = *collapsible;
        let mut header = ContainerWidget::new()
            .style(
                Style::new()
                    .padding(EdgeInsets::symmetric(12.0, 10.0))
                    .border_color(ColorToken::Border)
                    .border_width(1.0),
            )
            .focusable(!disabled)
            .tab_index(if disabled { -1 } else { 0 })
            .accessibility_role(AccessibilityRole::Button)
            .accessibility_expanded(shown)
            .accessibility_disabled(disabled);
        if !disabled {
            header = header.on_click(move |_, event_cx| {
                let next = if shown && can_collapse {
                    None
                } else {
                    Some(index)
                };
                if !controlled {
                    state.set(next);
                }
                if let Some(callback) = &callback {
                    callback.call(next);
                }
                event_cx.request_focus();
                EventResult::Consumed
            });
        }
        let mut section = vec![header.into_element_desc(vec![item.title.clone()])];
        if shown {
            section.push(
                ContainerWidget::new()
                    .style(Style::new().padding(EdgeInsets::all(12.0)))
                    .into_element_desc(vec![item.content.clone()]),
            );
        }
        children.push(
            ContainerWidget::new()
                .key(item.id.clone())
                .flex_direction(FlexDirectionStyle::Column)
                .into_element_desc(section),
        );
    }
    let mut root_style = Style::new();
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .into_element_desc(children)
}

#[derive(Clone, Debug, Hash)]
pub struct BreadcrumbItem {
    pub label: String,
    pub disabled: bool,
}

impl BreadcrumbItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            disabled: false,
        }
    }
}

#[component]
#[defaults(on_navigate = None, separator = "/".to_string(), style = Style::new())]
pub fn breadcrumb(
    items: &Vec<BreadcrumbItem>,
    on_navigate: &Option<IndexChangeCallback>,
    separator: &String,
    style: &Style,
) {
    let mut children = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            children.push(TextWidget::new(separator.clone()).into_element_desc());
        }
        let current = index + 1 == items.len();
        let callback = on_navigate.clone();
        let disabled = item.disabled || current;
        let mut node = ContainerWidget::new()
            .focusable(!disabled)
            .tab_index(if disabled { -1 } else { 0 })
            .accessibility_role(if current {
                AccessibilityRole::Text
            } else {
                AccessibilityRole::Link
            })
            .accessibility_label(item.label.clone())
            .accessibility_disabled(disabled);
        if !disabled {
            node = node.on_click(move |_, event_cx| {
                event_cx.request_focus();
                if let Some(callback) = &callback {
                    callback.call(index);
                }
                EventResult::Consumed
            });
        }
        children.push(node.into_element_desc(vec![
            TextWidget::new(item.label.clone()).into_element_desc(),
        ]));
    }
    let mut root_style = Style::new().gap(8.0);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Row)
        .align(AlignStyle::Center)
        .into_element_desc(children)
}

#[component]
#[defaults(page = 0, sibling_count = 1, on_change = None, style = Style::new())]
pub fn pagination(
    page_count: &usize,
    page: &usize,
    sibling_count: &usize,
    on_change: &Option<IndexChangeCallback>,
    style: &Style,
) {
    let page_count = *page_count;
    if page_count == 0 {
        return ContainerWidget::new()
            .accessibility_hidden(true)
            .into_element_desc(vec![]);
    }
    let current = (*page).min(page_count.saturating_sub(1));
    let start = current.saturating_sub(*sibling_count);
    let end = (current + *sibling_count + 1).min(page_count);
    let navigation_button = |label: String, target: usize, disabled: bool| {
        let callback = on_change.clone();
        let mut button = ContainerWidget::new()
            .style(
                Style::new()
                    .size(Size::fix(36.0, 36.0))
                    .align(AlignStyle::Center)
                    .justify(JustifyStyle::Center),
            )
            .focusable(!disabled)
            .tab_index(if disabled { -1 } else { 0 })
            .accessibility_role(AccessibilityRole::Button)
            .accessibility_label(label.clone())
            .accessibility_disabled(disabled);
        if !disabled {
            button = button.on_click(move |_, event_cx| {
                event_cx.request_focus();
                if let Some(callback) = &callback {
                    callback.call(target);
                }
                EventResult::Consumed
            });
        }
        button.into_element_desc(vec![TextWidget::new(label).into_element_desc()])
    };
    let mut children = vec![
        navigation_button("First page".to_owned(), 0, current == 0),
        navigation_button(
            "Previous page".to_owned(),
            current.saturating_sub(1),
            current == 0,
        ),
    ];
    for index in start..end {
        let callback = on_change.clone();
        let selected = index == current;
        children.push(
            ContainerWidget::new()
                .style(
                    Style::new()
                        .size(Size::fix(36.0, 36.0))
                        .align(AlignStyle::Center)
                        .justify(JustifyStyle::Center)
                        .background(if selected {
                            ColorStyle::from(ColorToken::Primary)
                        } else {
                            ColorStyle::from(Color::TRANSPARENT)
                        })
                        .border_radius(RadiusToken::Sm),
                )
                .focusable(true)
                .tab_index(0)
                .accessibility_role(AccessibilityRole::Button)
                .accessibility_label(format!("Page {}", index + 1))
                .accessibility_selected(selected)
                .on_click(move |_, event_cx| {
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(index);
                    }
                    EventResult::Consumed
                })
                .into_element_desc(vec![
                    TextWidget::new((index + 1).to_string()).into_element_desc(),
                ]),
        );
    }
    children.push(navigation_button(
        "Next page".to_owned(),
        (current + 1).min(page_count - 1),
        current + 1 >= page_count,
    ));
    children.push(navigation_button(
        "Last page".to_owned(),
        page_count - 1,
        current + 1 >= page_count,
    ));
    let mut root_style = Style::new().gap(4.0);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Row)
        .into_element_desc(children)
}

#[derive(Clone, Debug)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<ElementDesc>,
}

impl TableRow {
    pub fn new(id: impl Into<String>, cells: Vec<ElementDesc>) -> Self {
        Self {
            id: id.into(),
            cells,
        }
    }
}

#[component]
#[defaults(headers = Vec::new(), style = Style::new(), row_style = Style::new())]
pub fn data_table(
    rows: &Vec<TableRow>,
    headers: &Vec<ElementDesc>,
    style: &Style,
    row_style: &Style,
) {
    let columns = headers
        .len()
        .max(rows.iter().map(|row| row.cells.len()).max().unwrap_or(1));
    let make_row = |key: String, cells: Vec<ElementDesc>, header: bool| {
        let cells = cells
            .into_iter()
            .map(|cell| {
                ContainerWidget::new()
                    .style(
                        Style::new()
                            .padding(EdgeInsets::all(10.0))
                            .width(Sizing::Fill),
                    )
                    .accessibility_role(if header {
                        AccessibilityRole::ColumnHeader
                    } else {
                        AccessibilityRole::Cell
                    })
                    .into_element_desc(vec![cell])
            })
            .collect();
        let mut style = Style::new()
            .width(Sizing::Fill)
            .border_color(ColorToken::Border)
            .border_width(1.0);
        if header {
            style = style.background(ColorToken::MutedSurface);
        } else {
            style.merge(row_style);
        }
        GridWidget::new()
            .key(key)
            .style(style)
            .columns(GridTracks::repeat(columns, GridTrackSize::flexible()))
            .accessibility_role(AccessibilityRole::Row)
            .into_element_desc(cells)
    };
    let mut children = Vec::with_capacity(rows.len() + 1);
    if !headers.is_empty() {
        children.push(make_row("header".to_owned(), headers.clone(), true));
    }
    children.extend(
        rows.iter()
            .map(|row| make_row(row.id.clone(), row.cells.clone(), false)),
    );
    let mut root_style = Style::new().width(Sizing::Fill);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Table)
        .into_element_desc(children)
}

#[derive(Clone, Debug, Hash)]
pub struct DataGridColumn {
    pub id: String,
    pub label: String,
    pub sortable: bool,
}

impl DataGridColumn {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            sortable: false,
        }
    }

    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }
}

#[derive(Clone, Debug, Hash)]
pub struct DataGridRow {
    pub id: String,
    pub cells: Vec<String>,
    pub disabled: bool,
}

impl DataGridRow {
    pub fn new(id: impl Into<String>, cells: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            id: id.into(),
            cells: cells.into_iter().map(Into::into).collect(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct GridSort {
    pub column_id: String,
    pub direction: SortDirection,
}

pub type GridSortCallback = Callback<GridSort>;

#[component]
#[defaults(
    selected_row = None,
    sort = None,
    on_row_select = None,
    on_sort = None,
    style = Style::new(),
)]
pub fn data_grid(
    columns: &Vec<DataGridColumn>,
    rows: &Vec<DataGridRow>,
    selected_row: &Option<String>,
    sort: &Option<GridSort>,
    on_row_select: &Option<StringSelectCallback>,
    on_sort: &Option<GridSortCallback>,
    style: &Style,
) {
    let count = columns.len().max(1);
    let tracks = GridTracks::repeat(count, GridTrackSize::flexible());
    let headers = columns
        .iter()
        .map(|column| {
            let callback = on_sort.clone();
            let current = sort.as_ref().filter(|sort| sort.column_id == column.id);
            let indicator = match current.map(|sort| sort.direction) {
                Some(SortDirection::Ascending) => " ↑",
                Some(SortDirection::Descending) => " ↓",
                None => "",
            };
            let next = GridSort {
                column_id: column.id.clone(),
                direction: match current.map(|sort| sort.direction) {
                    Some(SortDirection::Ascending) => SortDirection::Descending,
                    _ => SortDirection::Ascending,
                },
            };
            let mut cell = ContainerWidget::new()
                .style(
                    Style::new()
                        .padding(EdgeInsets::all(10.0))
                        .background(ColorToken::MutedSurface)
                        .font_weight(FontWeight::Bold),
                )
                .focusable(column.sortable)
                .tab_index(if column.sortable { 0 } else { -1 })
                .accessibility_role(AccessibilityRole::ColumnHeader)
                .accessibility_label(column.label.clone());
            if column.sortable {
                cell = cell.on_click(move |_, event_cx| {
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(next.clone());
                    }
                    EventResult::Consumed
                });
            }
            cell.into_element_desc(vec![
                TextWidget::new(format!("{}{indicator}", column.label)).into_element_desc(),
            ])
        })
        .collect();
    let header = GridWidget::new()
        .columns(tracks.clone())
        .accessibility_role(AccessibilityRole::Row)
        .into_element_desc(headers);
    let mut children = vec![header];
    for row in rows {
        let selected = selected_row.as_ref() == Some(&row.id);
        let callback = on_row_select.clone();
        let id = row.id.clone();
        let disabled = row.disabled;
        let cells = (0..count)
            .map(|index| {
                ContainerWidget::new()
                    .style(Style::new().padding(EdgeInsets::all(10.0)))
                    .accessibility_role(AccessibilityRole::Cell)
                    .into_element_desc(vec![
                        TextWidget::new(row.cells.get(index).cloned().unwrap_or_default())
                            .into_element_desc(),
                    ])
            })
            .collect();
        let mut grid = GridWidget::new()
            .key(row.id.clone())
            .style(
                Style::new()
                    .background(if selected {
                        ColorStyle::from(ColorToken::MutedSurface)
                    } else {
                        ColorStyle::from(Color::TRANSPARENT)
                    })
                    .border_color(ColorToken::Border)
                    .border_width(1.0),
            )
            .columns(tracks.clone())
            .focusable(!disabled)
            .tab_index(if disabled { -1 } else { 0 })
            .accessibility_role(AccessibilityRole::Row)
            .accessibility_label(id.clone())
            .accessibility_selected(selected)
            .accessibility_disabled(disabled);
        if !disabled {
            grid = grid.on_click(move |_, event_cx| {
                event_cx.request_focus();
                if let Some(callback) = &callback {
                    callback.call(id.clone());
                }
                EventResult::Consumed
            });
        }
        children.push(grid.into_element_desc(cells));
    }
    let mut root_style = Style::new().width(Sizing::Fill);
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Table)
        .into_element_desc(children)
}

#[derive(Clone, Debug)]
pub struct TreeNode {
    pub id: String,
    pub label: String,
    pub children: Vec<TreeNode>,
    pub disabled: bool,
}

impl TreeNode {
    pub fn leaf(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children: Vec::new(),
            disabled: false,
        }
    }

    pub fn branch(
        id: impl Into<String>,
        label: impl Into<String>,
        children: Vec<TreeNode>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children,
            disabled: false,
        }
    }
}

pub type StringSelectCallback = Callback<String>;

fn tree_nodes(
    nodes: &[TreeNode],
    depth: usize,
    expanded: xui::state::State<Vec<String>>,
    selected: &Option<String>,
    on_select: &Option<StringSelectCallback>,
) -> Vec<ElementDesc> {
    let mut output = Vec::new();
    for node in nodes {
        let is_expanded = expanded.get().contains(&node.id);
        let has_children = !node.children.is_empty();
        let id = node.id.clone();
        let callback = on_select.clone();
        let disabled = node.disabled;
        let row = ContainerWidget::new()
            .style(
                Style::new()
                    .padding(EdgeInsets::new(8.0 + depth as f32 * 16.0, 8.0, 6.0, 6.0))
                    .background(if selected.as_ref() == Some(&node.id) {
                        ColorStyle::from(ColorToken::MutedSurface)
                    } else {
                        ColorStyle::from(Color::TRANSPARENT)
                    }),
            )
            .flex_direction(FlexDirectionStyle::Row)
            .gap(6.0)
            .focusable(!disabled)
            .tab_index(if disabled { -1 } else { 0 })
            .accessibility_role(AccessibilityRole::TreeItem)
            .accessibility_label(node.label.clone())
            .accessibility_expanded(is_expanded)
            .accessibility_disabled(disabled)
            .on_click(move |_, event_cx| {
                if disabled {
                    return EventResult::Ignored;
                }
                if has_children {
                    let toggle_id = id.clone();
                    expanded.update(move |items| {
                        if let Some(index) = items.iter().position(|item| item == &toggle_id) {
                            items.remove(index);
                        } else {
                            items.push(toggle_id);
                        }
                    });
                }
                if let Some(callback) = &callback {
                    callback.call(id.clone());
                }
                event_cx.request_focus();
                EventResult::Consumed
            })
            .into_element_desc(vec![
                TextWidget::new(if has_children {
                    if is_expanded { "▾" } else { "▸" }
                } else {
                    ""
                })
                .into_element_desc(),
                TextWidget::new(node.label.clone()).into_element_desc(),
            ]);
        let mut section = vec![row];
        if is_expanded {
            section.extend(tree_nodes(
                &node.children,
                depth + 1,
                expanded,
                selected,
                on_select,
            ));
        }
        output.extend(section);
    }
    output
}

#[component]
#[defaults(selected = None, default_expanded = Vec::new(), on_select = None, style = Style::new())]
pub fn tree_view(
    nodes: &Vec<TreeNode>,
    selected: &Option<String>,
    default_expanded: &Vec<String>,
    on_select: &Option<StringSelectCallback>,
    style: &Style,
) {
    let expanded = cx.use_state(|| default_expanded.clone());
    let mut root_style = Style::new();
    root_style.merge(style);
    ContainerWidget::new()
        .style(root_style)
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Tree)
        .into_element_desc(tree_nodes(nodes, 0, expanded, selected, on_select))
}
