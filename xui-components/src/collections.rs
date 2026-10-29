//! Disclosure, navigation, tabular data, and hierarchical collections.
//!
//! [`collapsible`] and [`accordion`] expose disclosure state, [`breadcrumb`] and
//! [`pagination`] report navigation requests, and [`data_table`]/[`data_grid`] render
//! supplied data. [`tree_view`] keeps selection and expansion independent. These data
//! surfaces materialize visible supplied rows; use [`crate::virtual_list::virtual_list`]
//! when the collection requires virtualization.
//!
//! # Components
//!
//! | Component | Purpose |
//! | --- | --- |
//! | [`collapsible`](fn@self::collapsible) | A disclosure section showing content when its trigger is activated. |
//! | [`accordion`](fn@self::accordion) | A single-expanded-item accordion with optional collapse behavior. |
//! | [`breadcrumb`](fn@self::breadcrumb) | A breadcrumb trail with callback-driven navigation and a configurable separator. |
//! | [`pagination`](fn@self::pagination) | A page-navigation row with endpoint buttons and visible sibling pages. |
//! | [`data_table`](fn@self::data_table) | A presentational table of arbitrary element cells with optional headers. |
//! | [`data_grid`](fn@self::data_grid) | A tabular string-data surface with sortable headers and selectable rows. |
//! | [`tree_view`](fn@self::tree_view) | A tree with independent controlled selection and expansion. |

use std::collections::HashSet;

use xui_core::prelude::*;
use xui_macros::component;

use crate::controls::commit;

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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `trigger_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { collapsible(trigger: view! { text("Details") }) { text("More information") } }
/// }
/// ```
#[component]
#[defaults(
    open = None::<bool>,
    default_open = false,
    style = StyleOf::new(),
    trigger_style = StyleOf::new(),
)]
pub fn collapsible(
    trigger: &Children,
    children: &Children,
    #[source] open: &Option<bool>,
    default_open: &bool,
    on_open_change: &Callback<bool>,
    style: &StyleOf<Container>,
    trigger_style: &StyleOf<Container>,
) {
    let own: bool = state!(*default_open);
    let shown: bool = memo!(open.unwrap_or(*own));
    view! {
        container(
            style: style.clone(),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            container(
                style: crate::layout::control_style(false).merged(trigger_style),
                focusable: true,
                tab_index: 0,
                accessibility_role: AccessibilityRole::Button,
                accessibility_expanded: *shown,
                on_click: |cx, _event, event_cx| {
                    commit(cx, !shown.get(), open, own, on_open_change);
                    event_cx.request_focus();
                    EventResult::Consumed
                },
            ) {
                slot(trigger)
            }
            if *shown {
                slot(children)
            }
        }
    }
}

/// An identified disclosure section with title, content, and disabled state.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionItem {
    pub id: String,
    pub title: Children,
    pub content: Children,
    pub disabled: bool,
}

impl AccordionItem {
    pub fn new(id: impl Into<String>, title: impl Into<Children>, content: impl Into<Children>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            content: content.into(),
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `trigger_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `content_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { accordion(items: vec![AccordionItem::new("details", "Details", "Information")]) }
/// }
/// ```
#[component]
#[defaults(
    expanded = None::<Option<usize>>,
    default_expanded = None::<usize>,
    collapsible = true,
    style = StyleOf::new(),
    trigger_style = StyleOf::new(),
    content_style = StyleOf::new(),
)]
pub fn accordion(
    #[source] items: &Vec<AccordionItem>,
    #[source] expanded: &Option<Option<usize>>,
    default_expanded: &Option<usize>,
    collapsible: &bool,
    on_change: &Callback<Option<usize>>,
    style: &StyleOf<Container>,
    trigger_style: &StyleOf<Container>,
    content_style: &StyleOf<Container>,
) {
    let own: Option<usize> = state!(*default_expanded);
    let active: Option<usize> = memo!(expanded.unwrap_or(*own));
    view! {
        container(
            style: style.clone(),
            flex_direction: FlexDirectionStyle::Column,
        ) {
            for (index, item) in items.iter().cloned().enumerate() {
                container(flex_direction: FlexDirectionStyle::Column) {
                    container(
                        style: StyleOf::new()
                            .with::<container::padding>(EdgeInsets::symmetric(12.0, 10.0))
                            .with::<container::border_color>(ColorToken::Border)
                            .with::<container::border_width>(1.0)
                            .merged(&crate::layout::control_style(item.disabled))
                            .merged(trigger_style),
                        focusable: !item.disabled,
                        tab_index: if item.disabled { -1 } else { 0 },
                        accessibility_role: AccessibilityRole::Button,
                        accessibility_expanded: *active == Some(*index),
                        accessibility_disabled: item.disabled,
                        on_click: |cx, _event, event_cx| {
                            if item.disabled {
                                return EventResult::Ignored;
                            }
                            let shown = active.get() == Some(*index);
                            let next = if shown && *collapsible { None } else { Some(*index) };
                            commit(cx, next, expanded, own, on_change);
                            event_cx.request_focus();
                            EventResult::Consumed
                        },
                    ) {
                        slot(item.title)
                    }
                    if *active == Some(*index) {
                        container(
                            style: StyleOf::new()
                                .with::<container::padding>(EdgeInsets::all(12.0))
                                .merged(content_style),
                        ) {
                            slot(item.content)
                        }
                    }
                }
                .key(item.id.clone())
            }
        }
    }
}

/// A breadcrumb label with optional disabled navigation.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `item_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { breadcrumb(items: vec![BreadcrumbItem::new("Home"), BreadcrumbItem::new("Settings")]) }
/// }
/// ```
#[component]
#[defaults(
    separator = "/".to_string(),
    style = StyleOf::new(),
    item_style = StyleOf::new(),
)]
pub fn breadcrumb(
    #[source] items: &Vec<BreadcrumbItem>,
    on_navigate: &Callback<usize>,
    #[source] separator: &String,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
) {
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(8.0).merged(style),
            flex_direction: FlexDirectionStyle::Row,
            align: AlignStyle::Center,
        ) {
            for (index, item) in items.iter().cloned().enumerate() {
                if *index > 0 {
                    text(separator.clone())
                }
                container(
                    style: crate::layout::control_style(item.disabled || *index + 1 == items.len())
                        .merged(item_style),
                    focusable: !(item.disabled || *index + 1 == items.len()),
                    tab_index: if item.disabled || *index + 1 == items.len() { -1 } else { 0 },
                    accessibility_role: if *index + 1 == items.len() {
                        AccessibilityRole::Text
                    } else {
                        AccessibilityRole::Link
                    },
                    accessibility_label: item.label.clone(),
                    accessibility_disabled: item.disabled || *index + 1 == items.len(),
                    on_click: |cx, _event, event_cx| {
                        if item.disabled || *index + 1 == items.read().len() {
                            return EventResult::Ignored;
                        }
                        event_cx.request_focus();
                        on_navigate.call(cx, *index);
                        EventResult::Consumed
                    },
                ) {
                    text(item.label.clone())
                }
            }
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `item_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { pagination(page_count: 10usize, page: 2usize) }
/// }
/// ```
#[component]
#[defaults(
    page = 0,
    sibling_count = 1,
    style = StyleOf::new(),
    item_style = StyleOf::new(),
)]
pub fn pagination(
    #[source] page_count: &usize,
    #[source] page: &usize,
    #[source] sibling_count: &usize,
    on_change: &Callback<usize>,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
) {
    let current: usize = memo!((*page).min(page_count.saturating_sub(1)));
    let pages: Vec<usize> = memo!({
        let start = current.saturating_sub(*sibling_count);
        let end = current.saturating_add(*sibling_count).saturating_add(1).min(*page_count);
        (start..end).collect()
    });
    let last: usize = memo!(page_count.saturating_sub(1));
    view! {
        container(
            style: StyleOf::new().with::<container::gap>(4.0).merged(style),
            flex_direction: FlexDirectionStyle::Row,
            accessibility_hidden: *page_count == 0,
        ) {
            if *page_count > 0 {
                page_button(label: "First page".to_string(), target: 0, disabled: *current == 0, on_change: on_change, item_style: item_style.clone())
                page_button(
                    label: "Previous page".to_string(),
                    target: current.saturating_sub(1),
                    disabled: *current == 0,
                    on_change: on_change,
                    item_style: item_style.clone(),
                )
                for index in pages.clone() {
                    container(
                        style: StyleOf::new()
                            .with::<container::size>(Size::fix(36.0, 36.0))
                            .with::<container::align>(AlignStyle::Center)
                            .with::<container::justify>(JustifyStyle::Center)
                            .with::<container::background>(if *index == *current {
                                ColorStyle::from(ColorToken::Primary)
                            } else {
                                ColorStyle::from(Color::TRANSPARENT)
                            })
                            .with::<container::border_radius>(RadiusToken::Sm),
                        focusable: true,
                        tab_index: 0,
                        accessibility_role: AccessibilityRole::Button,
                        accessibility_label: format!("Page {}", *index + 1),
                        accessibility_selected: *index == *current,
                        on_click: |cx, _event, event_cx| {
                            event_cx.request_focus();
                            on_change.call(cx, *index);
                            EventResult::Consumed
                        },
                    ) {
                        text((*index + 1).to_string())
                    }
                    .key(*index)
                }
                page_button(
                    label: "Next page".to_string(),
                    target: (*current + 1).min(*last),
                    disabled: *current + 1 >= *page_count,
                    on_change: on_change,
                    item_style: item_style.clone(),
                )
                page_button(
                    label: "Last page".to_string(),
                    target: *last,
                    disabled: *current + 1 >= *page_count,
                    on_change: on_change,
                    item_style: item_style.clone(),
                )
            }
        }
    }
}

/// One of `pagination`'s first/previous/next/last buttons.
#[component]
fn page_button(
    #[source] label: &String,
    #[source] target: &usize,
    #[source] disabled: &bool,
    on_change: &Callback<usize>,
    item_style: &StyleOf<Container>,
) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::size>(Size::fix(36.0, 36.0))
                .with::<container::align>(AlignStyle::Center)
                .with::<container::justify>(JustifyStyle::Center)
                .merged(&crate::layout::control_style(*disabled))
                .merged(item_style),
            focusable: !*disabled,
            tab_index: if *disabled { -1 } else { 0 },
            accessibility_role: AccessibilityRole::Button,
            accessibility_label: label.clone(),
            accessibility_disabled: *disabled,
            on_click: |cx, _event, event_cx| {
                if disabled.get() {
                    return EventResult::Ignored;
                }
                event_cx.request_focus();
                on_change.call(cx, target.get());
                EventResult::Consumed
            },
        ) {
            text(label.clone())
        }
    }
}

/// An identified table row containing arbitrary element cells.
#[derive(Clone, Debug, PartialEq)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<Children>,
}

impl TableRow {
    pub fn new(id: impl Into<String>, cells: impl IntoIterator<Item = impl Into<Children>>) -> Self {
        Self {
            id: id.into(),
            cells: cells.into_iter().map(Into::into).collect(),
        }
    }
}

/// A presentational table of arbitrary element cells with optional headers.
///
/// `rows` and `headers` provide content; `column_tracks` overrides the default
/// equal-width tracks. Header, cell, row, and root styles can be customized. This
/// component materializes all rows; use [`crate::virtual_list::virtual_list`] for large
/// visible datasets.
///
/// # Props
///
/// | Prop | Type | Default |
/// | --- | --- | --- |
/// | `rows` | `Vec<TableRow>` | **Required** |
/// | `headers` | `Vec<ElementDesc>` | `Vec::new()` |
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `row_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `header_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `cell_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `column_tracks` | `Option<GridTracks>` | `None` |
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
///     view! { data_table(rows: vec![TableRow::new("row-1", ["Alpha"])]) }
/// }
/// ```
#[component]
#[defaults(
    headers = Vec::<Children>::new(),
    style = StyleOf::new(),
    row_style = StyleOf::new(),
    header_style = StyleOf::new(),
    cell_style = StyleOf::new(),
    column_tracks = None::<GridTracks>,
)]
pub fn data_table(
    #[source] rows: &Vec<TableRow>,
    #[source] headers: &Vec<Children>,
    style: &StyleOf<Container>,
    row_style: &StyleOf<Grid>,
    header_style: &StyleOf<Grid>,
    cell_style: &StyleOf<Container>,
    column_tracks: &Option<GridTracks>,
) {
    let columns: usize = memo!(headers.len().max(rows.iter().map(|row| row.cells.len()).max().unwrap_or(1)));
    let line: StyleOf<Grid> = StyleOf::new()
        .with::<container::width>(Sizing::Fill)
        .with::<container::border_color>(ColorToken::Border)
        .with::<container::border_width>(1.0);
    let cell: StyleOf<Container> = StyleOf::new()
        .with::<container::padding>(EdgeInsets::all(10.0))
        .with::<container::width>(Sizing::Fill)
        .merged(cell_style);
    view! {
        container(
            style: StyleOf::new().with::<container::width>(Sizing::Fill).merged(style),
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Table,
        ) {
            if !headers.is_empty() {
                grid(
                    style: line.clone().with::<container::background>(ColorToken::MutedSurface).merged(header_style),
                    columns: column_tracks.clone().unwrap_or_else(|| GridTracks::repeat(*columns, GridTrackSize::flexible())),
                    accessibility_role: AccessibilityRole::Row,
                ) {
                    for header in headers.clone() {
                        container(style: cell.clone(), accessibility_role: AccessibilityRole::ColumnHeader) {
                            slot(header)
                        }
                    }
                }
            }
            for row in rows.clone() {
                grid(
                    style: line.clone().merged(row_style),
                    columns: column_tracks.clone().unwrap_or_else(|| GridTracks::repeat(*columns, GridTrackSize::flexible())),
                    accessibility_role: AccessibilityRole::Row,
                ) {
                    for value in row.cells.clone() {
                        container(style: cell.clone(), accessibility_role: AccessibilityRole::Cell) {
                            slot(value)
                        }
                    }
                }
                .key(row.id.clone())
            }
        }
    }
}

/// An identified grid column with a label and optional sort interaction.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
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
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `header_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `row_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `cell_style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `column_tracks` | `Option<GridTracks>` | `None` |
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
///     view! { data_grid(columns: vec![DataGridColumn::new("name", "Name")], rows: vec![DataGridRow::new("row-1", vec!["Alpha".to_owned()])]) }
/// }
/// ```
#[component]
#[defaults(
    selected_row = None::<String>,
    sort = None::<GridSort>,
    style = StyleOf::new(),
    header_style = StyleOf::new(),
    row_style = StyleOf::new(),
    cell_style = StyleOf::new(),
    column_tracks = None::<GridTracks>,
)]
pub fn data_grid(
    #[source] columns: &Vec<DataGridColumn>,
    #[source] rows: &Vec<DataGridRow>,
    #[source] selected_row: &Option<String>,
    #[source] sort: &Option<GridSort>,
    on_row_select: &Callback<String>,
    on_sort: &Callback<GridSort>,
    style: &StyleOf<Container>,
    header_style: &StyleOf<Container>,
    row_style: &StyleOf<Grid>,
    cell_style: &StyleOf<Container>,
    column_tracks: &Option<GridTracks>,
) {
    let count: usize = memo!(columns.len().max(1));
    view! {
        container(
            style: StyleOf::new().with::<container::width>(Sizing::Fill).merged(style),
            flex_direction: FlexDirectionStyle::Column,
            accessibility_role: AccessibilityRole::Table,
        ) {
            grid(
                columns: column_tracks.clone().unwrap_or_else(|| GridTracks::repeat(*count, GridTrackSize::flexible())),
                accessibility_role: AccessibilityRole::Row,
            ) {
                for column in columns.clone() {
                    container(
                        style: StyleOf::new()
                            .with::<container::padding>(EdgeInsets::all(10.0))
                            .with::<container::background>(ColorToken::MutedSurface)
                            .with::<container::font_weight>(FontWeight::Bold)
                            .merged(&crate::layout::control_style(!column.sortable))
                            .merged(header_style),
                        focusable: column.sortable,
                        tab_index: if column.sortable { 0 } else { -1 },
                        accessibility_role: AccessibilityRole::ColumnHeader,
                        accessibility_label: column.label.clone(),
                        on_click: |cx, _event, event_cx| {
                            if !column.sortable {
                                return EventResult::Ignored;
                            }
                            event_cx.request_focus();
                            let next = next_sort(&sort.read(), &column.id);
                            on_sort.call(cx, next);
                            EventResult::Consumed
                        },
                    ) {
                        text(format!("{}{}", column.label, sort_indicator(sort, &column.id)))
                    }
                    .key(column.id.clone())
                }
            }
            for row in rows.clone() {
                grid(
                    style: StyleOf::new()
                        .with::<container::background>(if selected_row.as_ref() == Some(&row.id) {
                            ColorStyle::from(ColorToken::MutedSurface)
                        } else {
                            ColorStyle::from(Color::TRANSPARENT)
                        })
                        .with::<container::border_color>(ColorToken::Border)
                        .with::<container::border_width>(1.0)
                        .merged(&crate::layout::control_style(row.disabled))
                        .merged(row_style),
                    columns: column_tracks.clone().unwrap_or_else(|| GridTracks::repeat(*count, GridTrackSize::flexible())),
                    focusable: !row.disabled,
                    tab_index: if row.disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::Row,
                    accessibility_label: row.id.clone(),
                    accessibility_selected: selected_row.as_ref() == Some(&row.id),
                    accessibility_disabled: row.disabled,
                    on_click: |cx, _event, event_cx| {
                        if row.disabled {
                            return EventResult::Ignored;
                        }
                        event_cx.request_focus();
                        on_row_select.call(cx, row.id.clone());
                        EventResult::Consumed
                    },
                ) {
                    for index in 0..*count {
                        container(
                            style: StyleOf::new().with::<container::padding>(EdgeInsets::all(10.0)).merged(cell_style),
                            accessibility_role: AccessibilityRole::Cell,
                        ) {
                            text(row.cells.get(*index).cloned().unwrap_or_default())
                        }
                    }
                }
                .key(row.id.clone())
            }
        }
    }
}

/// The arrow after a column's label: which way it sorts, if it does.
fn sort_indicator(sort: &Option<GridSort>, column: &str) -> &'static str {
    match sort.as_ref().filter(|sort| sort.column_id == column).map(|sort| sort.direction) {
        Some(SortDirection::Ascending) => " ↑",
        Some(SortDirection::Descending) => " ↓",
        None => "",
    }
}

/// The sort a press on `column` asks for: ascending, or the reverse of its own.
fn next_sort(sort: &Option<GridSort>, column: &str) -> GridSort {
    let current = sort.as_ref().filter(|sort| sort.column_id == column).map(|sort| sort.direction);
    GridSort {
        column_id: column.to_owned(),
        direction: match current {
            Some(SortDirection::Ascending) => SortDirection::Descending,
            _ => SortDirection::Ascending,
        },
    }
}

/// An identified leaf or branch with children and optional disabled interaction.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// A node shown in a tree view: its place in the flattened, expanded tree.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VisibleNode {
    id: String,
    label: String,
    depth: usize,
    has_children: bool,
    expanded: bool,
    disabled: bool,
}

/// The nodes shown, in order: every root, and the children of every expanded node.
fn visible_nodes(nodes: &[TreeNode], expanded: &HashSet<String>, depth: usize, out: &mut Vec<VisibleNode>) {
    for node in nodes {
        let is_expanded = expanded.contains(&node.id);
        out.push(VisibleNode {
            id: node.id.clone(),
            label: node.label.clone(),
            depth,
            has_children: !node.children.is_empty(),
            expanded: is_expanded,
            disabled: node.disabled,
        });
        if is_expanded {
            visible_nodes(&node.children, expanded, depth + 1, out);
        }
    }
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
/// | `style` | `StyleOf<Container>` | `StyleOf::new()` |
/// | `item_style` | `StyleOf<Container>` | `StyleOf::new()` |
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
///     view! { tree_view(nodes: vec![TreeNode::branch("folder", "Folder", vec![TreeNode::leaf("file", "File")])]) }
/// }
/// ```
#[component]
#[defaults(
    selected = None::<Option<String>>,
    default_selected = None::<String>,
    expanded = None::<Vec<String>>,
    default_expanded = Vec::<String>::new(),
    style = StyleOf::new(),
    item_style = StyleOf::new(),
)]
pub fn tree_view(
    #[source] nodes: &Vec<TreeNode>,
    #[source] selected: &Option<Option<String>>,
    default_selected: &Option<String>,
    #[source] expanded: &Option<Vec<String>>,
    default_expanded: &Vec<String>,
    on_expanded_change: &Callback<Vec<String>>,
    on_select: &Callback<Option<String>>,
    style: &StyleOf<Container>,
    item_style: &StyleOf<Container>,
) {
    let own_expanded: Vec<String> = state!(default_expanded.clone());
    let own_selected: Option<String> = state!(default_selected.clone());
    let expanded_ids: Vec<String> = memo!(expanded.clone().unwrap_or_else(|| own_expanded.clone()));
    let selected_id: Option<String> = memo!(selected.clone().unwrap_or_else(|| own_selected.clone()));
    let visible: Vec<VisibleNode> = memo!({
        let mut out = Vec::new();
        visible_nodes(nodes, &expanded_ids.iter().cloned().collect(), 0, &mut out);
        out
    });
    view! {
        column(style: style.clone(), accessibility_role: AccessibilityRole::Tree) {
            for node in visible.clone() {
                row(
                    style: crate::layout::control_style(node.disabled)
                        .with::<container::padding>(EdgeInsets::new(8.0 + node.depth as f32 * 16.0, 8.0, 6.0, 6.0))
                        .with::<container::background>(if selected_id.as_ref() == Some(&node.id) {
                            ColorStyle::from(ColorToken::Accent)
                        } else {
                            ColorStyle::from(Color::TRANSPARENT)
                        })
                        .when_style(
                            WidgetStateMatcher::all(WidgetState::HOVERED),
                            StyleOf::<Container>::new().with::<container::background>(ColorToken::Muted),
                        )
                        .merged(item_style),
                    gap: 6.0,
                    focusable: !node.disabled,
                    tab_index: if node.disabled { -1 } else { 0 },
                    accessibility_role: AccessibilityRole::TreeItem,
                    accessibility_label: node.label.clone(),
                    accessibility_expanded: node.expanded,
                    accessibility_selected: selected_id.as_ref() == Some(&node.id),
                    accessibility_disabled: node.disabled,
                    on_click: |cx, _event, event_cx| {
                        if node.disabled {
                            return EventResult::Ignored;
                        }
                        if node.has_children {
                            let mut next: Vec<String> = expanded_ids.get();
                            if node.expanded {
                                next.retain(|id| *id != node.id);
                            } else {
                                next.push(node.id.clone());
                            }
                            next.sort_unstable();
                            next.dedup();
                            commit(cx, next, expanded, own_expanded, on_expanded_change);
                        }
                        commit(cx, Some(node.id.clone()), selected, own_selected, on_select);
                        event_cx.request_focus();
                        EventResult::Consumed
                    },
                ) {
                    text(if node.has_children { if node.expanded { "▾" } else { "▸" } } else { "" })
                    text(node.label.clone())
                }
                .key(node.id.clone())
            }
        }
    }
}
