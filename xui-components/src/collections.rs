//! Disclosure, navigation, tabular data, and hierarchical collections.
//!
//! [`Collapsible`] and [`Accordion`] expose disclosure state, [`Breadcrumb`] and
//! [`Pagination`] report navigation requests, and [`DataTable`]/[`DataGrid`] render
//! supplied data. [`TreeView`] keeps selection and expansion independent. These data
//! surfaces materialize visible supplied rows; use [`crate::virtual_list::VirtualList`]
//! when the collection requires virtualization.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`Collapsible`](fn@self::Collapsible) | A disclosure section showing content when its trigger is activated. |
//! | [`Accordion`](fn@self::Accordion) | A single-expanded-item accordion with optional collapse behavior. |
//! | [`Breadcrumb`](fn@self::Breadcrumb) | A breadcrumb trail with callback-driven navigation and a configurable separator. |
//! | [`Pagination`](fn@self::Pagination) | A page-navigation row with endpoint buttons and visible sibling pages. |
//! | [`DataTable`](fn@self::DataTable) | A presentational table of arbitrary element cells with optional headers. |
//! | [`DataGrid`](fn@self::DataGrid) | A tabular string-data surface with sortable headers and selectable rows. |
//! | [`TreeView`](fn@self::TreeView) | A tree with independent controlled selection and expansion. |

use crate::layout::MergeClone;
use xui_core::prelude::*;
use xui_macros::{component, view};

use crate::{BoolChangeCallback, IndexChangeCallback};

/// A disclosure section showing content when its trigger is activated.
///
/// `open` controls expansion or `default_open` seeds internal state. `on_open_change`
/// reports requested changes. Closed content is not mounted. `trigger_style` customizes
/// the focusable trigger wrapper and `style` customizes the section.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `trigger` | `ElementDesc` | **Required** |
/// | `content` | `ElementDesc` | **Required** |
/// | `open` | `Option<bool>` | `None` |
/// | `default_open` | `bool` | `false` |
/// | `on_open_change` | `Option<BoolChangeCallback>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `trigger_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Collapsible(trigger: view! { text("Details") }, content: view! { text("More information") }) };
/// ```
#[component]
#[defaults(
    open = None,
    default_open = false,
    on_open_change = None,
    style = Style::new(),
    trigger_style = Style::new(),
)]
pub fn collapsible(
    trigger: &ElementDesc,
    content: &ElementDesc,
    open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Option<BoolChangeCallback>,
    style: &Style,
    trigger_style: &Style,
) {
    let state = cx.use_state(|| *default_open);
    let controlled = open.is_some();
    let shown = open.unwrap_or(*state.get());
    let callback = on_open_change.clone();
    let trigger = view! {
        container(
            style: crate::layout::control_style(false).merge_clone(trigger_style),
            focusable: true,
            tab_index: 0,
            accessibility_role: AccessibilityRole::Button,
            accessibility_expanded: shown,
            on_click: move |_, event_cx| {
                let next = !shown;
                if !controlled {
                    state.set(next);
                }
                if let Some(callback) = &callback {
                    callback.call(next);
                }
                event_cx.request_focus();
                EventResult::Consumed
            },
        ) {
            ..trigger.clone()
        }
    };
    let mut children = vec![trigger];
    if shown {
        children.push(content.clone());
    }
    let mut root_style = Style::new();
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
        ) {
            ..children
        }
    }
}

/// An identified disclosure section with title, content, and disabled state.
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

/// Callback requesting an item index or closure with `None`.
pub type OptionalIndexChangeCallback = Callback<Option<usize>>;

/// A single-expanded-item accordion with optional collapse behavior.
///
/// `expanded` distinguishes uncontrolled state (`None`), controlled closure
/// (`Some(None)`), and a controlled index (`Some(Some(index))`). `default_expanded` seeds
/// uncontrolled state. Disabled items ignore activation, and `collapsible` controls
/// whether an active item may close. Trigger, content, and root styles are independent.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<AccordionItem>` | **Required** |
/// | `expanded` | `Option<Option<usize>>` | `None` |
/// | `default_expanded` | `Option<usize>` | `None` |
/// | `collapsible` | `bool` | `true` |
/// | `on_change` | `Option<OptionalIndexChangeCallback>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `trigger_style` | `Style` | `Style::new()` |
/// | `content_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Accordion(items: vec![AccordionItem::new("details", view! { text("Details") }, view! { text("Information") })]) };
/// ```
#[component]
#[defaults(
    expanded = None,
    default_expanded = None,
    collapsible = true,
    on_change = None,
    style = Style::new(),
    trigger_style = Style::new(),
    content_style = Style::new(),
)]
pub fn accordion(
    items: &Vec<AccordionItem>,
    expanded: &Option<Option<usize>>,
    default_expanded: &Option<usize>,
    collapsible: &bool,
    on_change: &Option<OptionalIndexChangeCallback>,
    style: &Style,
    trigger_style: &Style,
    content_style: &Style,
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
        let interactive = !disabled;

        let mut section = vec![view! {
            container(
                style: Style::new()
                    .padding(EdgeInsets::symmetric(12.0, 10.0))
                    .border_color(ColorToken::Border)
                    .border_width(1.0).merge_clone(&crate::layout::control_style(disabled)).merge_clone(trigger_style),
                focusable: !disabled,
                tab_index: if disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::Button,
                accessibility_expanded: shown,
                accessibility_disabled: disabled,
                on_click: move |_, event_cx| { if !interactive { return EventResult::Ignored; }
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
                },
            ) {
                ..item.title.clone()
            }
        }];
        if shown {
            section.push(view! {
                container(
                    style: Style::new().padding(EdgeInsets::all(12.0)).merge_clone(content_style),
                ) {
                    ..item.content.clone()
                }
            });
        }
        children.push(view! {
            container(
                key: item.id.clone(),
                flex_direction: FlexDirectionStyle::Column,
            ) {
                ..section
            }
        });
    }
    let mut root_style = Style::new();
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
        ) {
            ..children
        }
    }
}

/// A breadcrumb label with optional disabled navigation.
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

/// A breadcrumb trail with callback-driven navigation and a configurable separator.
///
/// Enabled non-current entries notify `on_navigate` with their index. The last entry
/// identifies the current location and is not actionable. `item_style` customizes entries
/// and `style` customizes the trail.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `items` | `Vec<BreadcrumbItem>` | **Required** |
/// | `on_navigate` | `Option<IndexChangeCallback>` | `None` |
/// | `separator` | `String` | `"/".to_string()` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Breadcrumb(items: vec![BreadcrumbItem::new("Home"), BreadcrumbItem::new("Settings")]) };
/// ```
#[component]
#[defaults(
    on_navigate = None,
    separator = "/".to_string(),
    style = Style::new(),
    item_style = Style::new(),
)]
pub fn breadcrumb(
    items: &Vec<BreadcrumbItem>,
    on_navigate: &Option<IndexChangeCallback>,
    separator: &String,
    style: &Style,
    item_style: &Style,
) {
    let mut children = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            children.push(view! {
                xui_core::widgets::text(
                    separator.clone(),
                )
            });
        }
        let current = index + 1 == items.len();
        let callback = on_navigate.clone();
        let disabled = item.disabled || current;
        let navigation_enabled = !disabled;

        children.push(view! {
            container(
                style: crate::layout::control_style(disabled).merge_clone(item_style),
                focusable: !disabled,
                tab_index: if disabled { -1 } else { 0 },
                accessibility_role: if current {
                    AccessibilityRole::Text
                } else {
                    AccessibilityRole::Link
                },
                accessibility_label: item.label.clone(),
                accessibility_disabled: disabled,
                on_click: move |_, event_cx| { if !navigation_enabled { return EventResult::Ignored; }
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(index);
                    }
                    EventResult::Consumed
                },
            ) {
                xui_core::widgets::text(
                    item.label.clone(),
                )
            }
        });
    }
    let mut root_style = Style::new().gap(8.0);
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Row,
            align: AlignStyle::Center,
        ) {
            ..children
        }
    }
}

/// A page-navigation row with endpoint buttons and visible sibling pages.
///
/// `page` is a zero-based caller-controlled index, clamped to `page_count`.
/// `sibling_count` chooses the visible neighborhood; `on_change` reports navigation
/// requests. No controls are rendered when the count is zero. `item_style` customizes
/// endpoint controls and `style` customizes the row.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `page_count` | `usize` | **Required** |
/// | `page` | `usize` | `0` |
/// | `sibling_count` | `usize` | `1` |
/// | `on_change` | `Option<IndexChangeCallback>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { Pagination(page_count: 10usize, page: 2usize) };
/// ```
#[component]
#[defaults(
    page = 0,
    sibling_count = 1,
    on_change = None,
    style = Style::new(),
    item_style = Style::new(),
)]
pub fn pagination(
    page_count: &usize,
    page: &usize,
    sibling_count: &usize,
    on_change: &Option<IndexChangeCallback>,
    style: &Style,
    item_style: &Style,
) {
    let page_count = *page_count;
    if page_count == 0 {
        return view! {
            container(
                accessibility_hidden: true,
            )
        };
    }
    let current = (*page).min(page_count.saturating_sub(1));
    let start = current.saturating_sub(*sibling_count);
    let end = current
        .saturating_add(*sibling_count)
        .saturating_add(1)
        .min(page_count);
    let navigation_button = |label: String, target: usize, disabled: bool| {
        let callback = on_change.clone();
        let navigation_enabled = !disabled;

        view! {
            container(
                style: Style::new()
                    .size(Size::fix(36.0, 36.0))
                    .align(AlignStyle::Center)
                    .justify(JustifyStyle::Center).merge_clone(&crate::layout::control_style(disabled)).merge_clone(item_style),
                focusable: !disabled,
                tab_index: if disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::Button,
                accessibility_label: label.clone(),
                accessibility_disabled: disabled,
                on_click: move |_, event_cx| { if !navigation_enabled { return EventResult::Ignored; }
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(target);
                    }
                    EventResult::Consumed
                },
            ) {
                xui_core::widgets::text(
                    label.clone(),
                )
            }
        }
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
        children.push(view! {
            container(
                style: Style::new()
                    .size(Size::fix(36.0, 36.0))
                    .align(AlignStyle::Center)
                    .justify(JustifyStyle::Center)
                    .background(if selected {
                        ColorStyle::from(ColorToken::Primary)
                    } else {
                        ColorStyle::from(Color::TRANSPARENT)
                })
                    .border_radius(RadiusToken::Sm),
                focusable: true,
                tab_index: 0,
                accessibility_role: AccessibilityRole::Button,
                accessibility_label: format!("Page {}", index + 1),
                accessibility_selected: selected,
                on_click: move |_, event_cx| {
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(index);
                    }
                    EventResult::Consumed
                },
            ) {
                xui_core::widgets::text(
                    (index + 1).to_string(),
                )
            }
        });
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
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Row,
        ) {
            ..children
        }
    }
}

/// An identified table row containing arbitrary element cells.
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

/// A presentational table of arbitrary element cells with optional headers.
///
/// `rows` and `headers` provide content; `column_tracks` overrides the default
/// equal-width tracks. Header, cell, row, and root styles can be customized. This
/// component materializes all rows; use [`crate::virtual_list::VirtualList`] for large
/// visible datasets.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `rows` | `Vec<TableRow>` | **Required** |
/// | `headers` | `Vec<ElementDesc>` | `Vec::new()` |
/// | `style` | `Style` | `Style::new()` |
/// | `row_style` | `Style` | `Style::new()` |
/// | `header_style` | `Style` | `Style::new()` |
/// | `cell_style` | `Style` | `Style::new()` |
/// | `column_tracks` | `Option<GridTracks>` | `None` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { DataTable(rows: vec![TableRow::new("row-1", vec![view! { text("Alpha") }])]) };
/// ```
#[component]
#[defaults(
    headers = Vec::new(),
    style = Style::new(),
    row_style = Style::new(),
    header_style = Style::new(),
    cell_style = Style::new(),
    column_tracks = None,
)]
pub fn data_table(
    rows: &Vec<TableRow>,
    headers: &Vec<ElementDesc>,
    style: &Style,
    row_style: &Style,
    header_style: &Style,
    cell_style: &Style,
    column_tracks: &Option<GridTracks>,
) {
    let columns = headers
        .len()
        .max(rows.iter().map(|row| row.cells.len()).max().unwrap_or(1));
    let make_row = |key: String, cells: Vec<ElementDesc>, header: bool| {
        let cells: Vec<ElementDesc> = cells
            .into_iter()
            .map(|cell| {
                view! {
                    container(
                        style: Style::new()
                            .padding(EdgeInsets::all(10.0))
                            .width(Sizing::Fill).merge_clone(cell_style),
                        accessibility_role: if header {
                            AccessibilityRole::ColumnHeader
                        } else {
                            AccessibilityRole::Cell
                        },
                    ) {
                        ..cell
                    }
                }
            })
            .collect();
        let mut style = Style::new()
            .width(Sizing::Fill)
            .border_color(ColorToken::Border)
            .border_width(1.0);
        if header {
            style = style
                .background(ColorToken::MutedSurface)
                .merge_clone(header_style);
        } else {
            style.merge(row_style);
        }
        view! {
            grid(
                key: key,
                style: style,
                columns: column_tracks.clone().unwrap_or_else(|| GridTracks::repeat(columns, GridTrackSize::flexible())),
                accessibility_role: AccessibilityRole::Row,
            ) {
                ..cells
            }
        }
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
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Table,
        ) {
            ..children
        }
    }
}

/// An identified grid column with a label and optional sort interaction.
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

/// An identified grid row containing string cells and optional disabled selection.
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

/// The requested ordering direction for a data-grid column.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// A requested sort direction paired with a column ID; sorting is performed by the
/// caller.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct GridSort {
    pub column_id: String,
    pub direction: SortDirection,
}

/// Callback requesting a change to the active data-grid sort.
pub type GridSortCallback = Callback<GridSort>;

/// A tabular string-data surface with sortable headers and selectable rows.
///
/// `sort` and `selected_row` describe caller-controlled state. `on_sort` and
/// `on_row_select` report requests; the component does not reorder the data.
/// `column_tracks` controls widths, and header/cell/row/root styles customize decoration.
/// Disabled rows ignore selection. All supplied rows are materialized.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `columns` | `Vec<DataGridColumn>` | **Required** |
/// | `rows` | `Vec<DataGridRow>` | **Required** |
/// | `selected_row` | `Option<String>` | `None` |
/// | `sort` | `Option<GridSort>` | `None` |
/// | `on_row_select` | `Option<StringSelectCallback>` | `None` |
/// | `on_sort` | `Option<GridSortCallback>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `header_style` | `Style` | `Style::new()` |
/// | `row_style` | `Style` | `Style::new()` |
/// | `cell_style` | `Style` | `Style::new()` |
/// | `column_tracks` | `Option<GridTracks>` | `None` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { DataGrid(columns: vec![DataGridColumn::new("name", "Name")], rows: vec![DataGridRow::new("row-1", vec!["Alpha".to_owned()])]) };
/// ```
#[component]
#[defaults(
    selected_row = None,
    sort = None,
    on_row_select = None,
    on_sort = None,
    style = Style::new(),
    header_style = Style::new(),
    row_style = Style::new(),
    cell_style = Style::new(),
    column_tracks = None,
)]
pub fn data_grid(
    columns: &Vec<DataGridColumn>,
    rows: &Vec<DataGridRow>,
    selected_row: &Option<String>,
    sort: &Option<GridSort>,
    on_row_select: &Option<StringSelectCallback>,
    on_sort: &Option<GridSortCallback>,
    style: &Style,
    header_style: &Style,
    row_style: &Style,
    cell_style: &Style,
    column_tracks: &Option<GridTracks>,
) {
    let count = columns.len().max(1);
    let tracks = column_tracks
        .clone()
        .unwrap_or_else(|| GridTracks::repeat(count, GridTrackSize::flexible()));
    let headers: Vec<ElementDesc> = columns
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
            let sortable = column.sortable;

            view! {
                container(
                    style: Style::new()
                        .padding(EdgeInsets::all(10.0))
                        .background(ColorToken::MutedSurface)
                        .font_weight(FontWeight::Bold).merge_clone(&crate::layout::control_style(!column.sortable)).merge_clone(header_style),
                    focusable: column.sortable,
                    tab_index: if column.sortable { 0 } else { -1 },
                    accessibility_role: AccessibilityRole::ColumnHeader,
                    accessibility_label: column.label.clone(),
                    on_click: move |_, event_cx| { if !sortable { return EventResult::Ignored; }
                        event_cx.request_focus();
                        if let Some(callback) = &callback {
                            callback.call(next.clone());
                        }
                        EventResult::Consumed
                    },
                ) {
                    xui_core::widgets::text(
                        format!("{}{indicator}", column.label),
                    )
                }
            }
        })
        .collect();
    let header = view! {
        grid(
            columns: tracks.clone(),
            accessibility_role: AccessibilityRole::Row,
        ) {
            ..headers
        }
    };
    let mut children = vec![header];
    for row in rows {
        let selected = selected_row.as_ref() == Some(&row.id);
        let callback = on_row_select.clone();
        let id = row.id.clone();
        let disabled = row.disabled;
        let cells: Vec<ElementDesc> = (0..count)
            .map(|index| {
                view! {
                    container(
                        style: Style::new().padding(EdgeInsets::all(10.0)).merge_clone(cell_style),
                        accessibility_role: AccessibilityRole::Cell,
                    ) {
                        xui_core::widgets::text(
                            row.cells.get(index).cloned().unwrap_or_default(),
                        )
                    }
                }
            })
            .collect();
        let selectable = !disabled;

        children.push(view! {
            grid(
                key: row.id.clone(),
                style: Style::new()
                    .background(if selected {
                        ColorStyle::from(ColorToken::MutedSurface)
                    } else {
                        ColorStyle::from(Color::TRANSPARENT)
                })
                    .border_color(ColorToken::Border)
                    .border_width(1.0).merge_clone(&crate::layout::control_style(disabled)).merge_clone(row_style),
                columns: tracks.clone(),
                focusable: !disabled,
                tab_index: if disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::Row,
                accessibility_label: id.clone(),
                accessibility_selected: selected,
                accessibility_disabled: disabled,
                on_click: move |_, event_cx| { if !selectable { return EventResult::Ignored; }
                    event_cx.request_focus();
                    if let Some(callback) = &callback {
                        callback.call(id.clone());
                    }
                    EventResult::Consumed
                },
            ) {
                ..cells
            }
        });
    }
    let mut root_style = Style::new().width(Sizing::Fill);
    root_style.merge(style);
    view! {
        container(
            style: root_style,
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Table,
        ) {
            ..children
        }
    }
}

/// An identified leaf or branch with children and optional disabled interaction.
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

/// Callback requesting selection of an item by its string ID.
pub type StringSelectCallback = Callback<String>;

/// Callback requesting the complete set of expanded tree-node IDs.
pub type TreeExpansionCallback = Callback<Vec<String>>;

fn tree_nodes(
    nodes: &[TreeNode],
    depth: usize,
    expanded: &std::rc::Rc<std::collections::HashSet<String>>,
    expansion_state: xui_core::state::State<Vec<String>>,
    expansion_controlled: bool,
    on_expanded_change: &Option<TreeExpansionCallback>,
    selected: &Option<String>,
    selection_state: xui_core::state::State<Option<String>>,
    selection_controlled: bool,
    on_select: &Option<StringSelectCallback>,
    item_style: &Style,
) -> Vec<ElementDesc> {
    let mut output = Vec::with_capacity(nodes.len());
    for node in nodes {
        let is_expanded = expanded.contains(&node.id);
        let has_children = !node.children.is_empty();
        let id = node.id.clone();
        let selected_now = selected.as_ref() == Some(&node.id);
        let callback = on_select.clone();
        let expansion_callback = on_expanded_change.clone();
        let expansion = if has_children {
            Some(expanded.clone())
        } else {
            None
        };
        let disabled = node.disabled;
        let row_style = crate::layout::control_style(disabled)
            .padding(EdgeInsets::new(8.0 + depth as f32 * 16.0, 8.0, 6.0, 6.0))
            .background(if selected_now {
                ColorStyle::from(ColorToken::Accent)
            } else {
                ColorStyle::from(Color::TRANSPARENT)
            })
            .when(WidgetState::HOVERED, |s| s.background(ColorToken::Muted))
            .merge_clone(item_style);
        output.push(view! {
            row(style: row_style, key: node.id.clone(), gap: 6.0,
                focusable: !disabled, tab_index: if disabled { -1 } else { 0 },
                accessibility_role: AccessibilityRole::TreeItem,
                accessibility_label: node.label.clone(),
                accessibility_expanded: is_expanded, accessibility_selected: selected_now,
                accessibility_disabled: disabled,
                on_click: move |_, event_cx| {
                    if disabled { return EventResult::Ignored; }
                    if let Some(expansion) = &expansion {
                        let mut next = (**expansion).clone();
                        if is_expanded { next.remove(&id); } else { next.insert(id.clone()); }
                        let mut next: Vec<String> = next.into_iter().collect();
                        next.sort_unstable();
                        if !expansion_controlled { expansion_state.set(next.clone()); }
                        if let Some(callback) = &expansion_callback { callback.call(next); }
                    }
                    if !selection_controlled { selection_state.set(Some(id.clone())); }
                    if let Some(callback) = &callback { callback.call(id.clone()); }
                    event_cx.request_focus();
                    EventResult::Consumed
            }) {
                xui_core::widgets::text(if has_children { if is_expanded { "▾" } else { "▸" } } else { "" })
                xui_core::widgets::text(node.label.clone())
            }
        });
        if is_expanded {
            output.extend(tree_nodes(
                &node.children,
                depth + 1,
                expanded,
                expansion_state,
                expansion_controlled,
                on_expanded_change,
                selected,
                selection_state,
                selection_controlled,
                on_select,
                item_style,
            ));
        }
    }
    output
}

/// A tree with independent controlled selection and expansion.
///
/// `expanded` controls expanded IDs or `default_expanded` seeds internal state.
/// `selected` controls selection or `default_selected` seeds it. Expansion and selection
/// callbacks report requested changes. Expanded-ID lookup uses a shared hash set and
/// closed branches are not materialized. `item_style` customizes visible rows and `style`
/// customizes the tree.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `nodes` | `Vec<TreeNode>` | **Required** |
/// | `selected` | `Option<String>` | `None` |
/// | `default_selected` | `Option<String>` | `None` |
/// | `expanded` | `Option<Vec<String>>` | `None` |
/// | `default_expanded` | `Vec<String>` | `Vec::new()` |
/// | `on_expanded_change` | `Option<TreeExpansionCallback>` | `None` |
/// | `on_select` | `Option<StringSelectCallback>` | `None` |
/// | `style` | `Style` | `Style::new()` |
/// | `item_style` | `Style` | `Style::new()` |
///
/// # Example
///
/// ```rust
/// use xui_core::prelude::*;
/// use xui_components::*;
/// use xui_macros::view;
///
/// let _ = view! { TreeView(nodes: vec![TreeNode::branch("folder", "Folder", vec![TreeNode::leaf("file", "File")])]) };
/// ```
#[component]
#[defaults(
    selected = None,
    default_selected = None,
    expanded = None,
    default_expanded = Vec::new(),
    on_expanded_change = None,
    on_select = None,
    style = Style::new(),
    item_style = Style::new(),
)]
pub fn tree_view(
    nodes: &Vec<TreeNode>,
    selected: &Option<String>,
    default_selected: &Option<String>,
    expanded: &Option<Vec<String>>,
    default_expanded: &Vec<String>,
    on_expanded_change: &Option<TreeExpansionCallback>,
    on_select: &Option<StringSelectCallback>,
    style: &Style,
    item_style: &Style,
) {
    let expansion = cx.use_state(|| default_expanded.clone());
    let selection = cx.use_state(|| default_selected.clone());
    let expanded_ids = std::rc::Rc::new(
        expanded
            .as_ref()
            .unwrap_or(expansion.get())
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<String>>(),
    );
    let selected_id = selected.clone().or_else(|| selection.get().clone());
    let children = tree_nodes(
        nodes,
        0,
        &expanded_ids,
        expansion,
        expanded.is_some(),
        on_expanded_change,
        &selected_id,
        selection,
        selected.is_some(),
        on_select,
        item_style,
    );
    view! {
        column(children, style: style.clone(), accessibility_role: AccessibilityRole::Tree)
    }
}
