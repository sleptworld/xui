# xui-table

`xui-table` is the advanced data-table layer for XUI. It keeps row processing
in a deterministic `TableModel` and rendering in the `advanced_table`
component, so query behavior can be tested without a window.

## Capabilities

- Fixed, flexible and minimum-flex column sizing
- Start/end column ordering and hidden columns
- Single and Shift-assisted multi-column sorting
- Case-insensitive per-column filtering with numeric-aware sorting
- Client-side or server-managed sorting, filtering and pagination
- Controlled or uncontrolled single/multiple row selection
- Pointer and keyboard column resizing with change callbacks
- Custom header, cell and row style renderers
- Arrow/Home/End keyboard cell navigation
- Loading, error and empty states with live-region semantics
- Viewport-bounded row and focus-handle virtualization
- Table, row, column-header and cell accessibility roles

Start/end pinning currently determines column order. Sticky columns during
horizontal scrolling are not part of the current contract.

## Example

```rust
use xui::prelude::*;
use xui_table::*;

let columns = vec![
    TableColumn::new("name", "Name", 0)
        .sortable(true)
        .resizable(true),
    TableColumn::new("score", "Score", 1)
        .width(ColumnWidth::Fixed(100.0))
        .sortable(true),
];
let rows = vec![
    TableRow::new("ada", ["Ada", "98"]),
    TableRow::new("grace", ["Grace", "100"]),
];

xui! {
    <advanced_table
        columns={columns}
        rows={rows}
        selection_mode={SelectionMode::Multiple}
        pagination={Some(Pagination::new(0, 25))}
    />
}
```

Run `cargo test -p xui-table` for model, virtualization and accessibility-tree
coverage. The interactive catalog lives in `xui-example-app` and runs with
`cargo xui run` from that package directory.
