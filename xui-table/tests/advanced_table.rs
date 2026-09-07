use xui::prelude::*;
use xui::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_table::*;

const ROW_COUNT: usize = 100_000;

fn table_root(_: &mut HookContext<'_>) -> ElementDesc {
    let columns = vec![
        TableColumn::new("id", "ID", 0)
            .width(ColumnWidth::Fixed(96.0))
            .sortable(true)
            .resizable(true)
            .pinned(ColumnPin::Start),
        TableColumn::new("name", "Name", 1)
            .width(ColumnWidth::FlexibleMin(160.0))
            .sortable(true),
        TableColumn::new("score", "Score", 2)
            .width(ColumnWidth::Fixed(100.0))
            .sortable(true),
    ];
    let rows = (0..ROW_COUNT)
        .map(|index| {
            TableRow::new(
                format!("row-{index}"),
                [
                    index.to_string(),
                    format!("Person {index}"),
                    (index % 100).to_string(),
                ],
            )
        })
        .collect::<Vec<_>>();
    xui! {
        <advanced_table
            columns={columns}
            rows={rows}
            selection_mode={SelectionMode::Multiple}
            viewport_height={240.0}
            row_height={40.0}
            overscan={3_usize}
        />
    }
}

fn total_hosts(runtime: &UiRuntime, id: NodeId) -> usize {
    1 + runtime
        .children(id)
        .map(|child| total_hosts(runtime, child))
        .sum::<usize>()
}

#[test]
fn a_hundred_thousand_rows_keep_the_host_and_accessibility_trees_bounded() {
    let mut app = App::new(table_root);
    app.resize(Size::new(900.0, 500.0));
    let mut backend = MockRenderBackend::default();
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    app.render(&mut backend, &mut text)
        .expect("advanced table should render");

    let runtime = app.ui_runtime();
    assert!(
        total_hosts(runtime, runtime.root()) < 100,
        "virtualized table mounted too many hosts"
    );

    let roles = runtime
        .accessibility_tree()
        .nodes
        .into_iter()
        .filter_map(|node| node.properties.role)
        .collect::<Vec<_>>();
    assert!(roles.contains(&AccessibilityRole::Table));
    assert_eq!(
        roles
            .iter()
            .filter(|role| **role == AccessibilityRole::ColumnHeader)
            .count(),
        3
    );
    assert!(
        roles
            .iter()
            .filter(|role| **role == AccessibilityRole::Row)
            .count()
            < 20
    );
}
