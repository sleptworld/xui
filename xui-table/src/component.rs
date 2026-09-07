use xui::prelude::*;

use crate::{
    ColumnAccess, FilterRule, Pagination, SortDirection, SortRule, TableModel, TableQuery, TableRow,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ColumnWidth {
    Fixed(f32),
    #[default]
    Flexible,
    FlexibleMin(f32),
}

impl ColumnWidth {
    fn track(self, override_width: Option<f32>) -> GridTrackSize {
        if let Some(width) = override_width {
            return GridTrackSize::fixed(width.max(24.0));
        }
        match self {
            Self::Fixed(width) => GridTrackSize::fixed(width.max(24.0)),
            Self::Flexible => GridTrackSize::flexible(),
            Self::FlexibleMin(min) => GridTrackSize::flexible_min(min.max(0.0)),
        }
    }

    fn resize_base(self) -> f32 {
        match self {
            Self::Fixed(width) | Self::FlexibleMin(width) => width,
            Self::Flexible => 160.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum ColumnPin {
    Start,
    #[default]
    None,
    End,
}

#[derive(Clone, Debug)]
pub struct HeaderRenderContext {
    pub column_id: String,
    pub label: String,
    pub visible_index: usize,
    pub sort: Option<SortDirection>,
}

#[derive(Clone, Debug)]
pub struct CellRenderContext {
    pub row_id: String,
    pub row_index: usize,
    pub column_id: String,
    pub column_index: usize,
    pub value: String,
    pub selected: bool,
}

#[derive(Clone, Debug)]
pub struct RowStyleContext {
    pub row_id: String,
    pub row_index: usize,
    pub selected: bool,
    pub disabled: bool,
}

pub type HeaderRenderer = Callback<HeaderRenderContext, ElementDesc>;
pub type CellRenderer = Callback<CellRenderContext, ElementDesc>;
pub type RowStyleResolver = Callback<RowStyleContext, Style>;
pub type SelectionChangeCallback = Callback<Vec<String>>;
pub type SortChangeCallback = Callback<Vec<SortRule>>;
pub type PageChangeCallback = Callback<usize>;
pub type ColumnResizeCallback = Callback<(String, f32)>;

#[derive(Clone, Debug)]
pub struct TableColumn {
    pub id: String,
    pub label: String,
    pub source_index: usize,
    pub width: ColumnWidth,
    pub min_width: f32,
    pub hidden: bool,
    pub sortable: bool,
    pub resizable: bool,
    pub pin: ColumnPin,
    pub header_renderer: Option<HeaderRenderer>,
    pub cell_renderer: Option<CellRenderer>,
}

impl TableColumn {
    pub fn new(id: impl Into<String>, label: impl Into<String>, source_index: usize) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            source_index,
            width: ColumnWidth::Flexible,
            min_width: 64.0,
            hidden: false,
            sortable: false,
            resizable: false,
            pin: ColumnPin::None,
            header_renderer: None,
            cell_renderer: None,
        }
    }

    pub fn width(mut self, width: ColumnWidth) -> Self {
        self.width = width;
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = min_width.max(24.0);
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    pub fn pinned(mut self, pin: ColumnPin) -> Self {
        self.pin = pin;
        self
    }

    pub fn header_renderer(mut self, renderer: HeaderRenderer) -> Self {
        self.header_renderer = Some(renderer);
        self
    }

    pub fn cell_renderer(mut self, renderer: CellRenderer) -> Self {
        self.cell_renderer = Some(renderer);
        self
    }
}

impl ColumnAccess for TableColumn {
    fn id(&self) -> &str {
        &self.id
    }

    fn source_index(&self) -> usize {
        self.source_index
    }

    fn hidden(&self) -> bool {
        self.hidden
    }

    fn pin_order(&self) -> u8 {
        match self.pin {
            ColumnPin::Start => 0,
            ColumnPin::None => 1,
            ColumnPin::End => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SelectionMode {
    #[default]
    None,
    Single,
    Multiple,
}

#[derive(Clone, Debug, Hash)]
pub struct TableStyle {
    pub root: Style,
    pub header: Style,
    pub header_cell: Style,
    pub body: Style,
    pub row: Style,
    pub selected_row: Style,
    pub disabled_row: Style,
    pub cell: Style,
    pub status: Style,
    pub footer: Style,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            root: Style::new()
                .width(Sizing::Fill)
                .border_width(1.0)
                .border_color(ColorToken::Border)
                .border_radius(RadiusToken::Md)
                .clip(true),
            header: Style::new()
                .width(Sizing::Fill)
                .background(ColorToken::MutedSurface)
                .border_width(1.0)
                .border_color(ColorToken::Border),
            header_cell: Style::new()
                .height(Sizing::Fill)
                .padding(EdgeInsets::symmetric(12.0, 8.0))
                .font_weight(FontWeight::Bold),
            body: Style::new().width(Sizing::Fill),
            row: Style::new()
                .width(Sizing::Fill)
                .border_width(1.0)
                .border_color(ColorToken::Border),
            selected_row: Style::new().background(ColorToken::MutedSurface),
            disabled_row: Style::new().background(ColorToken::MutedSurface),
            cell: Style::new()
                .height(Sizing::Fill)
                .padding(EdgeInsets::symmetric(12.0, 8.0)),
            status: Style::new()
                .width(Sizing::Fill)
                .padding(EdgeInsets::all(24.0))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::Center),
            footer: Style::new()
                .width(Sizing::Fill)
                .padding(EdgeInsets::all(8.0))
                .align(AlignStyle::Center)
                .justify(JustifyStyle::SpaceBetween),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VirtualRange {
    start: usize,
    end: usize,
}

fn visible_range(
    scroll_top: f32,
    viewport_height: f32,
    row_height: f32,
    count: usize,
    overscan: usize,
) -> VirtualRange {
    if count == 0 || viewport_height <= 0.0 || row_height <= 0.0 {
        return VirtualRange { start: 0, end: 0 };
    }
    let first = (scroll_top.max(0.0) / row_height).floor() as usize;
    let span = (viewport_height / row_height).ceil() as usize + 2;
    let start = first.saturating_sub(overscan).min(count);
    let end = first
        .saturating_add(span)
        .saturating_add(overscan)
        .min(count);
    VirtualRange { start, end }
}

fn next_sort(current: &[SortRule], column: &TableColumn, additive: bool) -> Vec<SortRule> {
    let direction = current
        .iter()
        .find(|rule| rule.column_id == column.id)
        .map_or(SortDirection::Ascending, |rule| rule.direction.toggled());
    let next = SortRule {
        column_id: column.id.clone(),
        direction,
    };
    if !additive {
        return vec![next];
    }
    let mut rules = current.to_vec();
    if let Some(existing) = rules.iter_mut().find(|rule| rule.column_id == column.id) {
        *existing = next;
    } else {
        rules.push(next);
    }
    rules
}

fn next_selection(current: &[String], id: &str, mode: SelectionMode) -> Vec<String> {
    match mode {
        SelectionMode::None => current.to_vec(),
        SelectionMode::Single => vec![id.to_owned()],
        SelectionMode::Multiple => {
            let mut next = current.to_vec();
            if let Some(index) = next.iter().position(|selected| selected == id) {
                next.remove(index);
            } else {
                next.push(id.to_owned());
            }
            next
        }
    }
}

#[component]
#[defaults(
    sort = None,
    default_sort = Vec::new(),
    filters = Vec::new(),
    pagination = None,
    manual_sorting = false,
    manual_filtering = false,
    manual_pagination = false,
    total_row_count = None,
    selection_mode = SelectionMode::None,
    selection = None,
    default_selection = Vec::new(),
    on_sort_change = None,
    on_page_change = None,
    on_selection_change = None,
    on_column_resize = None,
    row_style = None,
    loading = false,
    error = None,
    empty_text = "No rows".to_string(),
    loading_text = "Loading".to_string(),
    viewport_height = 420.0,
    row_height = 44.0,
    header_height = 44.0,
    overscan = 4,
    style = TableStyle::default(),
)]
pub fn advanced_table(
    columns: &Vec<TableColumn>,
    rows: &Vec<TableRow>,
    sort: &Option<Vec<SortRule>>,
    default_sort: &Vec<SortRule>,
    filters: &Vec<FilterRule>,
    pagination: &Option<Pagination>,
    manual_sorting: &bool,
    manual_filtering: &bool,
    manual_pagination: &bool,
    total_row_count: &Option<usize>,
    selection_mode: &SelectionMode,
    selection: &Option<Vec<String>>,
    default_selection: &Vec<String>,
    on_sort_change: &Option<SortChangeCallback>,
    on_page_change: &Option<PageChangeCallback>,
    on_selection_change: &Option<SelectionChangeCallback>,
    on_column_resize: &Option<ColumnResizeCallback>,
    row_style: &Option<RowStyleResolver>,
    loading: &bool,
    error: &Option<String>,
    empty_text: &String,
    loading_text: &String,
    viewport_height: &f32,
    row_height: &f32,
    header_height: &f32,
    overscan: &usize,
    style: &TableStyle,
) {
    let internal_sort = cx.use_state(|| default_sort.clone());
    let internal_selection = cx.use_state(|| default_selection.clone());
    let column_widths = cx.use_state(Vec::<(String, f32)>::new);
    let resize_origins = cx.use_ref(Vec::<(String, f32)>::new);
    let scroll_top = cx.use_state(|| 0.0_f32);
    let measured_height = cx.use_state(|| 0.0_f32);
    let active_cell = cx.use_state(|| 0_usize);
    let controlled_sort = sort.is_some();
    let controlled_selection = selection.is_some();
    let effective_sort = sort.clone().unwrap_or_else(|| internal_sort.get().clone());
    let effective_selection = selection
        .clone()
        .unwrap_or_else(|| internal_selection.get().clone());
    let query = TableQuery {
        sort: effective_sort.clone(),
        filters: filters.clone(),
        pagination: *pagination,
        manual_sorting: *manual_sorting,
        manual_filtering: *manual_filtering,
        manual_pagination: *manual_pagination,
        total_row_count: *total_row_count,
    };
    let model = TableModel::build(columns, rows, &query);
    let visible_columns = model.visible_columns().to_vec();
    let visible_rows = model.visible_rows().to_vec();
    let column_count = visible_columns.len();
    let visible_row_count = visible_rows.len();
    let row_height_value = *row_height;
    let overscan_value = *overscan;
    let viewport = if *measured_height.get() > 0.0 {
        *measured_height.get()
    } else {
        *viewport_height
    };
    let range = visible_range(
        *scroll_top.get(),
        viewport,
        row_height_value,
        visible_row_count,
        overscan_value,
    );
    let tracks = GridTracks::Explicit(
        visible_columns
            .iter()
            .map(|index| {
                let column = &columns[*index];
                let resized = column_widths
                    .get()
                    .iter()
                    .find(|(id, _)| id == &column.id)
                    .map(|(_, width)| *width);
                column.width.track(resized)
            })
            .collect(),
    );

    let focus_deps = (
        visible_rows[range.start..range.end]
            .iter()
            .map(|index| rows[*index].id.clone())
            .collect::<Vec<_>>(),
        visible_columns
            .iter()
            .map(|index| columns[*index].id.clone())
            .collect::<Vec<_>>(),
    );
    let cell_handles = cx.use_memo_with(focus_deps, || {
        (0..range
            .end
            .saturating_sub(range.start)
            .saturating_mul(column_count))
            .map(|_| FocusHandle::new())
            .collect::<Vec<_>>()
    });

    let header_cells = visible_columns
        .iter()
        .enumerate()
        .map(|(visible_index, column_index)| {
            let column = &columns[*column_index];
            let sort_direction = effective_sort
                .iter()
                .find(|rule| rule.column_id == column.id)
                .map(|rule| rule.direction);
            let content = column.header_renderer.as_ref().map_or_else(
                || {
                    let indicator = match sort_direction {
                        Some(SortDirection::Ascending) => " ↑",
                        Some(SortDirection::Descending) => " ↓",
                        None => "",
                    };
                    TextWidget::new(format!("{}{indicator}", column.label)).into_element_desc()
                },
                |renderer| {
                    renderer.call(HeaderRenderContext {
                        column_id: column.id.clone(),
                        label: column.label.clone(),
                        visible_index,
                        sort: sort_direction,
                    })
                },
            );
            let mut children = vec![content];
            if column.resizable {
                let id = column.id.clone();
                let start_id = id.clone();
                let move_id = id.clone();
                let end_id = id.clone();
                let min_width = column.min_width;
                let base = column_widths
                    .get()
                    .iter()
                    .find(|(column_id, _)| column_id == &id)
                    .map_or_else(|| column.width.resize_base(), |(_, width)| *width);
                let drag_resized = on_column_resize.clone();
                let keyboard_resized = on_column_resize.clone();
                children.push(
                    ContainerWidget::new()
                        .style(
                            Style::new()
                                .width(8.0)
                                .height(Sizing::Fill)
                                .background(Color::TRANSPARENT),
                        )
                        .accessibility_role(AccessibilityRole::Slider)
                        .accessibility_label(format!("Resize {} column", column.label))
                        .accessibility_numeric_value(base as f64)
                        .accessibility_value_range(min_width as f64, 4096.0, Some(1.0))
                        .focusable(true)
                        .tab_index(0)
                        .on_drag_start(move |_, _| {
                            resize_origins.update(|origins| {
                                origins.retain(|(column_id, _)| column_id != &start_id);
                                origins.push((start_id.clone(), base));
                            });
                            EventResult::Consumed
                        })
                        .on_drag_move(move |event, _| {
                            let origin = resize_origins
                                .get()
                                .iter()
                                .find(|(column_id, _)| column_id == &move_id)
                                .map_or(base, |(_, width)| *width);
                            let width = (origin + event.total_delta.x).max(min_width);
                            let resize_id = move_id.clone();
                            column_widths.update(move |widths| {
                                if let Some((_, value)) = widths
                                    .iter_mut()
                                    .find(|(column_id, _)| column_id == &resize_id)
                                {
                                    *value = width;
                                } else {
                                    widths.push((resize_id, width));
                                }
                            });
                            if let Some(callback) = &drag_resized {
                                callback.call((move_id.clone(), width));
                            }
                            EventResult::Consumed
                        })
                        .on_drag_end(move |_, _| {
                            resize_origins.update(|origins| {
                                origins.retain(|(column_id, _)| column_id != &end_id);
                            });
                            EventResult::Consumed
                        })
                        .on_key_down(move |event, _| {
                            let delta = match event.named_key {
                                Some(NamedKey::ArrowLeft) => -8.0,
                                Some(NamedKey::ArrowRight) => 8.0,
                                _ => return EventResult::Ignored,
                            };
                            let width = (base + delta).max(min_width);
                            let resize_id = id.clone();
                            column_widths.update(move |widths| {
                                if let Some((_, value)) = widths
                                    .iter_mut()
                                    .find(|(column_id, _)| column_id == &resize_id)
                                {
                                    *value = width;
                                } else {
                                    widths.push((resize_id, width));
                                }
                            });
                            if let Some(callback) = &keyboard_resized {
                                callback.call((id.clone(), width));
                            }
                            EventResult::Consumed
                        })
                        .into_element_desc(vec![]),
                );
            }
            let mut cell = ContainerWidget::new()
                .style(
                    style
                        .header_cell
                        .clone()
                        .justify(JustifyStyle::SpaceBetween),
                )
                .flex_direction(FlexDirectionStyle::Row)
                .focusable(column.sortable)
                .tab_index(if column.sortable { 0 } else { -1 })
                .accessibility_role(AccessibilityRole::ColumnHeader)
                .accessibility_label(column.label.clone());
            if column.sortable {
                let callback = on_sort_change.clone();
                let column = column.clone();
                let current = effective_sort.clone();
                cell = cell.on_click(move |event, event_cx| {
                    let next = next_sort(&current, &column, event.meta.modifiers.shift);
                    if !controlled_sort {
                        internal_sort.set(next.clone());
                    }
                    if let Some(callback) = &callback {
                        callback.call(next);
                    }
                    event_cx.request_focus();
                    EventResult::Consumed
                });
            }
            cell.into_element_desc(children)
        })
        .collect();
    let header = GridWidget::new()
        .style(style.header.clone().height(*header_height))
        .columns(tracks.clone())
        .accessibility_role(AccessibilityRole::Row)
        .into_element_desc(header_cells);

    let mut rendered_rows = Vec::with_capacity(range.end.saturating_sub(range.start));
    for visible_row_index in range.start..range.end {
        let source_row_index = visible_rows[visible_row_index];
        let row = &rows[source_row_index];
        let selected = effective_selection.iter().any(|id| id == &row.id);
        let mut row_visual = style.row.clone();
        if selected {
            row_visual.merge(&style.selected_row);
        }
        if row.disabled {
            row_visual.merge(&style.disabled_row);
        }
        if let Some(resolver) = row_style {
            row_visual.merge(&resolver.call(RowStyleContext {
                row_id: row.id.clone(),
                row_index: source_row_index,
                selected,
                disabled: row.disabled,
            }));
        }
        row_visual = row_visual
            .absolute()
            .inset(EdgeInsets::new(
                0.0,
                0.0,
                visible_row_index as f32 * row_height_value,
                0.0,
            ))
            .height(row_height_value);
        let cells = visible_columns
            .iter()
            .enumerate()
            .map(|(visible_column_index, source_column_index)| {
                let column = &columns[*source_column_index];
                let value = row
                    .cells
                    .get(column.source_index)
                    .cloned()
                    .unwrap_or_default();
                let content = column.cell_renderer.as_ref().map_or_else(
                    || TextWidget::new(value.clone()).into_element_desc(),
                    |renderer| {
                        renderer.call(CellRenderContext {
                            row_id: row.id.clone(),
                            row_index: source_row_index,
                            column_id: column.id.clone(),
                            column_index: *source_column_index,
                            value: value.clone(),
                            selected,
                        })
                    },
                );
                let focus_index = visible_row_index * column_count + visible_column_index;
                let local_focus_index =
                    (visible_row_index - range.start) * column_count + visible_column_index;
                let handles = cell_handles.get().clone();
                let current_selection = effective_selection.clone();
                let row_id = row.id.clone();
                let selection_callback = on_selection_change.clone();
                let disabled = row.disabled;
                let mode = *selection_mode;
                let mut cell = ContainerWidget::new()
                    .style(style.cell.clone())
                    .focusable(!disabled)
                    .tab_index(if !disabled && focus_index == *active_cell.get() {
                        0
                    } else {
                        -1
                    })
                    .focus_handle(handles[local_focus_index].clone())
                    .accessibility_role(AccessibilityRole::Cell)
                    .accessibility_label(format!("{}: {}", column.label, value))
                    .accessibility_selected(selected)
                    .accessibility_disabled(disabled);
                if !disabled {
                    cell = cell
                        .on_click(move |_, event_cx| {
                            active_cell.set(focus_index);
                            event_cx.request_focus();
                            if mode != SelectionMode::None {
                                let next = next_selection(&current_selection, &row_id, mode);
                                if !controlled_selection {
                                    internal_selection.set(next.clone());
                                }
                                if let Some(callback) = &selection_callback {
                                    callback.call(next);
                                }
                            }
                            EventResult::Consumed
                        })
                        .on_key_down(move |event, event_cx| {
                            let row_count = visible_row_count;
                            let next = match event.named_key {
                                Some(NamedKey::ArrowLeft) => focus_index
                                    .checked_sub(1)
                                    .filter(|_| focus_index % column_count > 0),
                                Some(NamedKey::ArrowRight) => (visible_column_index + 1
                                    < column_count)
                                    .then_some(focus_index + 1),
                                Some(NamedKey::ArrowUp) => focus_index.checked_sub(column_count),
                                Some(NamedKey::ArrowDown) => (visible_row_index + 1 < row_count)
                                    .then_some(focus_index + column_count),
                                Some(NamedKey::Home) => Some(visible_row_index * column_count),
                                Some(NamedKey::End) => {
                                    Some(visible_row_index * column_count + column_count - 1)
                                }
                                _ => None,
                            };
                            let Some(next) = next.filter(|index| {
                                let target_row = *index / column_count;
                                target_row >= range.start && target_row < range.end
                            }) else {
                                return EventResult::Ignored;
                            };
                            active_cell.set(next);
                            let local_next = next - range.start * column_count;
                            handles[local_next].request_focus(event_cx);
                            EventResult::Consumed
                        });
                }
                cell.into_element_desc(vec![content])
            })
            .collect();
        rendered_rows.push(
            GridWidget::new()
                .key(row.id.clone())
                .style(row_visual)
                .columns(tracks.clone())
                .accessibility_role(AccessibilityRole::Row)
                .accessibility_label(row.id.clone())
                .accessibility_selected(selected)
                .accessibility_disabled(row.disabled)
                .into_element_desc(cells),
        );
    }

    let status = if *loading {
        Some((loading_text.clone(), AccessibilityLiveRegion::Polite))
    } else if let Some(error) = error {
        Some((error.clone(), AccessibilityLiveRegion::Assertive))
    } else if visible_rows.is_empty() || visible_columns.is_empty() {
        Some((empty_text.clone(), AccessibilityLiveRegion::Polite))
    } else {
        None
    };
    let body_content = if let Some((message, live)) = status {
        ContainerWidget::new()
            .style(style.status.clone().height(*viewport_height))
            .accessibility_live_region(live)
            .accessibility_label(message.clone())
            .into_element_desc(vec![TextWidget::new(message).into_element_desc()])
    } else {
        let content = ContainerWidget::new()
            .style(
                Style::new()
                    .relative()
                    .width(Sizing::Fill)
                    .height(row_height_value * visible_row_count as f32),
            )
            .into_element_desc(rendered_rows);
        ContainerWidget::new()
            .style(
                style
                    .body
                    .clone()
                    .height(*viewport_height)
                    .scroll_vertical(),
            )
            .on_scroll(move |event, event_cx| {
                let Some(offset) = event.offset_after.map(|offset| offset.y) else {
                    return EventResult::Ignored;
                };
                let height = event_cx.node_ref.layout.height();
                let next = visible_range(
                    offset,
                    height,
                    row_height_value,
                    visible_row_count,
                    overscan_value,
                );
                if next != range {
                    scroll_top.set(offset);
                    if height > 0.0 && (height - *measured_height.get()).abs() > 0.5 {
                        measured_height.set(height);
                    }
                }
                EventResult::Ignored
            })
            .into_element_desc(vec![content])
    };

    let mut children = vec![header, body_content];
    if let Some(pagination) = pagination {
        let page = pagination.page.min(model.page_count().saturating_sub(1));
        let previous = on_page_change.clone();
        let next = on_page_change.clone();
        let previous_disabled = page == 0;
        let next_disabled = page + 1 >= model.page_count();
        let make_button = |label: &'static str,
                           target: usize,
                           disabled: bool,
                           callback: Option<PageChangeCallback>| {
            let mut button = ContainerWidget::new()
                .style(Style::new().padding(EdgeInsets::symmetric(10.0, 6.0)))
                .focusable(!disabled)
                .tab_index(if disabled { -1 } else { 0 })
                .accessibility_role(AccessibilityRole::Button)
                .accessibility_label(label)
                .accessibility_disabled(disabled);
            if !disabled {
                button = button.on_click(move |_, _| {
                    if let Some(callback) = &callback {
                        callback.call(target);
                    }
                    EventResult::Consumed
                });
            }
            button.into_element_desc(vec![TextWidget::new(label).into_element_desc()])
        };
        children.push(
            ContainerWidget::new()
                .style(style.footer.clone())
                .flex_direction(FlexDirectionStyle::Row)
                .into_element_desc(vec![
                    make_button(
                        "Previous page",
                        page.saturating_sub(1),
                        previous_disabled,
                        previous,
                    ),
                    TextWidget::new(format!(
                        "Page {} of {} · {} rows",
                        page + 1,
                        model.page_count(),
                        model.filtered_row_count()
                    ))
                    .into_element_desc(),
                    make_button(
                        "Next page",
                        (page + 1).min(model.page_count().saturating_sub(1)),
                        next_disabled,
                        next,
                    ),
                ]),
        );
    }
    ContainerWidget::new()
        .style(style.root.clone())
        .flex_direction(FlexDirectionStyle::Column)
        .accessibility_role(AccessibilityRole::Table)
        .accessibility_label("Data table")
        .into_element_desc(children)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_bounded_by_viewport_and_overscan() {
        let range = visible_range(4_000.0, 400.0, 40.0, 100_000, 3);
        assert_eq!(range.start, 97);
        assert_eq!(range.end, 115);
    }

    #[test]
    fn selection_modes_are_deterministic() {
        assert_eq!(next_selection(&[], "a", SelectionMode::Single), ["a"]);
        assert_eq!(
            next_selection(&["a".into()], "a", SelectionMode::Multiple),
            Vec::<String>::new()
        );
    }
}
