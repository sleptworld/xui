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
| Actions | `button` with six variants and four sizes |
| Text and choice inputs | `input`, `checkbox`, `radio_group`, `switch`, `slider`, `drop_down` (`select` and `combo_box` aliases) |
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

## Declarative API and configuration

All 36 component implementations use `#[component]` and `view!`. The controls,
forms, overlays, collections, calendar, command, and selection modules are
exported at the crate root and participate in compilation and tests.
`select`/`combo_box` are aliases for the same non-editable dropdown;
`sheet` is an alias for `drawer`.

```rust
use xui_core::prelude::*;
use xui_components::*;
use xui_macros::{component, view};

#[component]
fn account_actions() {
    view! {
        column(gap: 12.0) {
            xui_components::input(accessibility_label: Some("Email".to_owned()),
                required: true, width: 320.0)
            button(text: "Save", variant: ButtonVariant::Primary,
                size: ButtonSize::Large)
        }
    }
}
```

A component forwards a prop it received with `.share()` instead of `.clone()`:
`popover(trigger: trigger.share(), ..)`. The child then reads the value through
the parent's props allocation, however many levels it is passed down; anything
that is not one of the rendering component's own props is cloned as before.

Every component accepts a `style` override. Named visual slots are merged after
their defaults, so callers can change decoration without replacing behavior.
Virtual row position and height remain owned by the virtualization algorithm.
The image component also accepts `style`, separate from pixel fitting via
`image_style`.

| Components | Configuration and customization |
| --- | --- |
| `button` | Variant, size, loading, disabled, full width, leading/trailing/loading elements, label style, transition duration, accessibility label |
| `input` | Optional durable controller, disabled, read-only, invalid/required, accessibility label, root/input styles, padding, dimensions and bounds, border/background |
| `drop_down`, `select`, `combo_box` | Controlled/default selection and open state, open/change callbacks, disabled options, placeholder, placement/collision behavior, z-index, independent style slots, custom ID prefix |
| `tabs` | Controlled/default selection, horizontal/vertical orientation, automatic/manual activation, wrapping, disabled items, list/tab/panel styles, custom ID prefix |
| `checkbox`, `switch`, `radio_group` | Controlled/default values, disabled, labels and callbacks; indicator/item styles or switch track/thumb styles; radio orientation |
| `slider`, `progress` | Range/value settings, accessible value labels, track/fill/thumb or indicator styles; slider disabled and stepped keyboard interaction |
| `badge`, `avatar`, `card`, `separator` | Badge variants, avatar size/image/fallback/fallback style, card content, separator orientation; root styles |
| `skeleton`, `spinner`, `callout` | Skeleton dimensions; spinner size/period/pause and label; callout tone/title/content/title style; root styles |
| `form_field` | Required/disabled/status, description/message, label/description/message styles, accessibility feedback |
| `dialog`, `drawer`, `sheet` | Controlled/default open state, callbacks, panel/backdrop styles, drawer side; dialog title style and separate Escape/outside dismissal switches |
| `popover`, `tooltip` | Trigger/content styles and anchor placement; popover controlled/default open state, disabled, modal and z-index; tooltip pointer/focus visibility and z-index |
| `menu`, `context_menu`, `command_palette` | Disabled items, selection/open callbacks, item styles for menus, arrow/Home/End navigation; command filtering by externally supplied query |
| `collapsible`, `accordion` | Controlled/default disclosure, callbacks, trigger/content styles; accordion collapsibility and disabled items |
| `breadcrumb`, `pagination` | Navigation callbacks, item styles, breadcrumb separator, page/sibling settings |
| `data_table`, `data_grid` | Explicit column tracks, header/cell/row styles; grid sort requests and row-selection callbacks. Sorting is performed by the caller. |
| `tree_view` | Independent controlled/default selection and expansion, expansion/selection callbacks, item styles |
| `calendar`, `date_picker` | Month/date selection callbacks, disabled, calendar header/day/selected-day styles; picker controlled/default open state and placement |
| `image` | Asset/URL/path source, alt text, dimensions, opacity, root/image styles, fitting/alignment/repeat/sampling, loading/fallback elements |
| `virtual_list` | Fixed row height, count, viewport hint, overscan, renderer and item style; actual viewport size follows layout observations |

Default dropdown and tab accessibility IDs are unique per component instance
and stable across rerenders. Supply `id_prefix` when external references need
an application-defined ID.

## Performance behavior

- Virtual lists memoize their visible rows. Scrolling inside the same row window
  does not invoke the row renderer; resizing updates the window without waiting
  for a wheel event. Shrinking a source clamps old scroll offsets.
- Tabs and radio groups share focus-handle storage across item callbacks instead
  of cloning the complete handle array for every item. Tab navigation avoids
  scanning disabled items for each disabled tab. Keys follow item IDs through
  reorder operations.
- Tree expansion lookup uses a shared hash set. Closed branches are not built;
  very large visible collections should still use `virtual_list`.
- Menus do not build their option elements while closed. Command matching
  normalizes the query once per pass; selection models cache folded labels.
- Network images reuse an HTTP client and retain the resource across rerenders.
  URL/file decoding runs on a blocking worker rather than the async executor,
  and downloaded bytes are decoded without an extra buffer copy.
- Spinners animate the icon transform, reuse embedded icon data, and can be
  paused. Ordinary focus/hover styles change paint without changing layout.
- Slider range/step normalization, finite geometry checks, and saturating
  pagination arithmetic handle reversed, non-finite, and extreme inputs.

## Theme

Component styles are written against semantic `ColorToken`s modeled on
shadcn/ui (`Background`/`Foreground`, `Primary`, `Secondary`, `Muted`,
`Accent`, `Destructive`, each with a `*Foreground`, plus `Border`, `Input`, and
`Ring`), so they follow the runtime theme instead of hard-coding colors.
`Theme::light()` (the default) and `Theme::dark()` are the shadcn neutral
palettes; switch with `App::set_theme`. `ColorToken::Primary.alpha(0.9)`
expresses Tailwind-style `bg-primary/90` hover states.

`cargo run -p xui-example-app`, the Component Gallery, shows the styled
controls (`XUI_THEME=dark` for the dark palette).

## Engineering contract

- Controlled and uncontrolled state never compete. Controlled values are the
  source of truth; callbacks only request changes.
- Disabled items are removed from focus navigation and ignore activation.
- Selection widgets skip disabled items, support arrow/Home/End navigation
  where applicable. `SelectionModel` and `TypeaheadState` provide reusable
  case-insensitive buffered matching.
- Overlays render in portals above clipped ancestors. Modal portals block
  background pointer input; Escape and pointer-outside dismissal request state
  changes through callbacks. Automatic focus trapping/restoration is not part
  of the current portal API.
- Anchored overlays perform viewport collision handling in the runtime.
- `input` supports single-line editing, IME, and disabled/read-only behavior.
  An omitted controller is retained for the component lifetime; an explicit
  `TextController` remains the external editing model.
- Tables, trees, dialogs, listboxes, tabs, controls, ranges, and live feedback
  expose platform-neutral accessibility roles and state for host adapters.
- Large collections use `virtual_list`; rendering cost is bounded by viewport
  height plus overscan rather than total row count.

## Verification

Run the component, runtime, and gallery suites with:

```sh
cargo test -p xui-components --lib --tests
cargo test -p xui-core --lib --tests
cargo test -p xui-example-app --bin xui-example-app gallery::tests
```

The catalog test covers every component in light/dark themes at a narrow
viewport; interaction tests cover controlled values, disabled/read-only input,
keyboard selection, menu navigation, tree expansion, and viewport resizing.

The example application is the interactive Component Gallery:
`cargo run -p xui-example-app`. Its test renders every page headlessly in both
palettes; set `XUI_GALLERY_SNAPSHOT_DIR` to write the frames as PNGs. The
aircraft icing dashboard demo is `cargo run -p xui-example-app --bin flight-icing`.
