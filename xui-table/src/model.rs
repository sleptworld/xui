use std::cmp::Ordering;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<String>,
    pub disabled: bool,
}

impl TableRow {
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

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub const fn toggled(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct SortRule {
    pub column_id: String,
    pub direction: SortDirection,
}

impl SortRule {
    pub fn ascending(column_id: impl Into<String>) -> Self {
        Self {
            column_id: column_id.into(),
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct FilterRule {
    pub column_id: String,
    pub query: String,
}

impl FilterRule {
    pub fn contains(column_id: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            column_id: column_id.into(),
            query: query.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct Pagination {
    pub page: usize,
    pub page_size: usize,
}

impl Pagination {
    pub const fn new(page: usize, page_size: usize) -> Self {
        Self { page, page_size }
    }
}

#[derive(Clone, Debug, Default, Hash, PartialEq, Eq)]
pub struct TableQuery {
    pub sort: Vec<SortRule>,
    pub filters: Vec<FilterRule>,
    pub pagination: Option<Pagination>,
    pub manual_sorting: bool,
    pub manual_filtering: bool,
    pub manual_pagination: bool,
    /// Total rows available on the server when pagination is externally managed.
    pub total_row_count: Option<usize>,
}

pub trait ColumnAccess {
    fn id(&self) -> &str;
    fn source_index(&self) -> usize;
    fn hidden(&self) -> bool;
    fn pin_order(&self) -> u8;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableModel {
    visible_columns: Vec<usize>,
    visible_rows: Vec<usize>,
    filtered_row_count: usize,
    page_count: usize,
}

impl TableModel {
    pub fn build<C: ColumnAccess>(columns: &[C], rows: &[TableRow], query: &TableQuery) -> Self {
        let mut visible_columns = columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| (!column.hidden()).then_some(index))
            .collect::<Vec<_>>();
        visible_columns.sort_by_key(|index| (columns[*index].pin_order(), *index));

        let mut visible_rows = (0..rows.len()).collect::<Vec<_>>();
        if !query.manual_filtering {
            visible_rows.retain(|row_index| {
                query.filters.iter().all(|filter| {
                    let Some(column) = columns
                        .iter()
                        .find(|column| column.id() == filter.column_id)
                    else {
                        return true;
                    };
                    let needle = filter.query.trim().to_lowercase();
                    needle.is_empty()
                        || rows[*row_index]
                            .cells
                            .get(column.source_index())
                            .is_some_and(|value| value.to_lowercase().contains(&needle))
                })
            });
        }
        let filtered_row_count = if query.manual_pagination {
            query.total_row_count.unwrap_or(visible_rows.len())
        } else {
            visible_rows.len()
        };

        if !query.manual_sorting && !query.sort.is_empty() {
            visible_rows.sort_by(|left, right| {
                for rule in &query.sort {
                    let Some(column) = columns.iter().find(|column| column.id() == rule.column_id)
                    else {
                        continue;
                    };
                    let left = rows[*left]
                        .cells
                        .get(column.source_index())
                        .map(String::as_str)
                        .unwrap_or_default();
                    let right = rows[*right]
                        .cells
                        .get(column.source_index())
                        .map(String::as_str)
                        .unwrap_or_default();
                    let ordering = compare_cell_values(left, right);
                    let ordering = match rule.direction {
                        SortDirection::Ascending => ordering,
                        SortDirection::Descending => ordering.reverse(),
                    };
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                rows[*left].id.cmp(&rows[*right].id)
            });
        }

        let page_count = query.pagination.map_or(1, |pagination| {
            if pagination.page_size == 0 {
                1
            } else {
                filtered_row_count.div_ceil(pagination.page_size).max(1)
            }
        });
        if !query.manual_pagination
            && let Some(pagination) = query.pagination
            && pagination.page_size > 0
        {
            let page = pagination.page.min(page_count - 1);
            let start = page
                .saturating_mul(pagination.page_size)
                .min(visible_rows.len());
            let end = start
                .saturating_add(pagination.page_size)
                .min(visible_rows.len());
            visible_rows = visible_rows[start..end].to_vec();
        }

        Self {
            visible_columns,
            visible_rows,
            filtered_row_count,
            page_count,
        }
    }

    pub fn visible_columns(&self) -> &[usize] {
        &self.visible_columns
    }

    pub fn visible_rows(&self) -> &[usize] {
        &self.visible_rows
    }

    pub const fn filtered_row_count(&self) -> usize {
        self.filtered_row_count
    }

    pub const fn page_count(&self) -> usize {
        self.page_count
    }
}

fn compare_cell_values(left: &str, right: &str) -> Ordering {
    match (left.trim().parse::<f64>(), right.trim().parse::<f64>()) {
        (Ok(left), Ok(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        _ => left.to_lowercase().cmp(&right.to_lowercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Column(&'static str, usize, bool, u8);

    impl ColumnAccess for Column {
        fn id(&self) -> &str {
            self.0
        }
        fn source_index(&self) -> usize {
            self.1
        }
        fn hidden(&self) -> bool {
            self.2
        }
        fn pin_order(&self) -> u8 {
            self.3
        }
    }

    fn columns() -> Vec<Column> {
        vec![Column("name", 0, false, 1), Column("score", 1, false, 0)]
    }

    fn rows() -> Vec<TableRow> {
        vec![
            TableRow::new("a", ["Ada", "9"]),
            TableRow::new("b", ["Grace", "10"]),
            TableRow::new("c", ["Linus", "8"]),
        ]
    }

    #[test]
    fn filters_sorts_numerically_paginates_and_orders_pins() {
        let query = TableQuery {
            sort: vec![SortRule::ascending("score")],
            filters: vec![FilterRule::contains("name", "a")],
            pagination: Some(Pagination::new(0, 1)),
            ..Default::default()
        };
        let model = TableModel::build(&columns(), &rows(), &query);
        assert_eq!(model.visible_columns(), &[1, 0]);
        assert_eq!(model.visible_rows(), &[0]);
        assert_eq!(model.filtered_row_count(), 2);
        assert_eq!(model.page_count(), 2);
    }

    #[test]
    fn manual_modes_leave_server_order_and_count_untouched() {
        let query = TableQuery {
            sort: vec![SortRule::ascending("score")],
            filters: vec![FilterRule::contains("name", "missing")],
            pagination: Some(Pagination::new(4, 1)),
            manual_sorting: true,
            manual_filtering: true,
            manual_pagination: true,
            total_row_count: Some(42),
        };
        let model = TableModel::build(&columns(), &rows(), &query);
        assert_eq!(model.visible_rows(), &[0, 1, 2]);
        assert_eq!(model.filtered_row_count(), 42);
        assert_eq!(model.page_count(), 42);
    }
}
