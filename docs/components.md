# Component library

`xui-components` is the production component layer above the host widgets in
`xui`. The public surface is organized by behavior rather than by visual demo.
Interactive controls and selectable items expose disabled state, keyboard
activation through semantic events, native accessibility metadata, style
overrides, and callbacks.
Stateful controls use the same contract throughout: an optional controlled
value, a `default_*` value for uncontrolled use, and an `on_*` callback.

## Implemented catalog

| Area | Components |
| --- | --- |
| Actions | `button` with five variants and three sizes |
| Text and choice inputs | `input`, `text_area`, `checkbox`, `radio_group`, `switch`, `slider`, `drop_down` (`select` and `combo_box` aliases) |
| Overlay and transient UI | `dialog`, `drawer` (`sheet` alias), `popover`, `tooltip`, `menu`, `context_menu`, `toast`, `command_palette` |
| Navigation and disclosure | `tabs`, `collapsible`, `accordion`, `breadcrumb`, `pagination` |
| Date | `calendar`, `date_picker`, `CalendarDate`, `CalendarMonth` |
| Data | `data_table`, sortable/selectable `data_grid`, `tree_view`, `virtual_list` |
| Feedback | `progress`, `spinner`, `skeleton`, `callout`, form validation messages |
| Content and structure | `image`, `avatar`, `badge`, `card`, `separator`, `form_field` |

Layout primitives (`ContainerWidget`, `GridWidget`, flex direction, scroll
styles, and portals) remain in `xui`; wrapping each one in a second component
would create duplicate APIs without adding behavior.

For product-scale data surfaces, the separate `xui-table` crate provides
`advanced_table`. It adds ordered start/end columns, multi-column sorting,
column filters, client or server pagination, single/multiple controlled
selection, custom header/cell/row visual slots, pointer and keyboard column
resizing, loading/error/empty states, keyboard grid navigation, and
viewport-bounded row rendering. Its pure `TableModel` can be tested without a
window or renderer. Start/end pinning currently controls column order; sticky
horizontal pinning is intentionally not claimed.

## Engineering contract

- Controlled and uncontrolled state never compete. Controlled values are the
  source of truth; callbacks only request changes.
- Disabled items are removed from focus navigation and ignore activation.
- Selection widgets skip disabled items, support arrow/Home/End navigation
  where applicable, and use buffered case-insensitive typeahead.
- Modal overlays trap and restore focus, close through semantic dismiss events,
  and render in a portal above clipped ancestors.
- Anchored overlays perform viewport collision handling in the runtime.
- Text input supports IME, read-only/disabled state, single-line and multiline
  editing. `TextController` remains the durable editing model.
- Tables, trees, dialogs, listboxes, tabs, controls, ranges, and live feedback
  are exported to the platform accessibility tree by `xui-winit`/AccessKit.
- Large collections use `virtual_list`; rendering cost is bounded by viewport
  height plus overscan rather than total row count.

## Verification

Run the component and platform suites with:

```sh
cargo test -p xui-components --lib --tests
cargo test -p xui-table
cargo test -p xui-winit
cargo test -p xui
```

The example application is the interactive Component Gallery. It requires its
generated asset bootstrap and should be built with `cargo xui`, not a plain
workspace-wide Cargo command.
