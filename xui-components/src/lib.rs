//! The built-in declarative component library for XUI.
//!
//! All 36 components are defined with `#[component]` and composed in `view!`. They
//! share themed styles, typed props, and semantic event handling. Props that callers
//! bind to changing state (`disabled`, values, items) are `#[source]` props: pass a
//! source by name and the component follows it. Callbacks take closures
//! (`on_change: |cx, value| ..`), and element props take `view! { .. }` or a trailing
//! `{ .. }` block.
//!
//! # Quick start
//!
//! ```rust
//! use xui_core::prelude::*;
//! use xui_components::*;
//! use xui_macros::component;
//!
//! #[component]
//! fn sign_in() {
//!     let saving: bool = state!(false);
//!     view! {
//!         column(gap: 12.0) {
//!             xui_components::input(accessibility_label: Some("Email".to_owned()), required: true)
//!             button(
//!                 text: "Save",
//!                 variant: ButtonVariant::Primary,
//!                 loading: saving,
//!                 on_click: |cx| saving.set(cx, true),
//!             )
//!         }
//!     }
//! }
//! ```
//!
//! # Component catalog
//!
//! | Module | Component | Purpose |
//! | --- | --- | --- |
//! | [`button`](mod@crate::button) | [`button`](fn@crate::button::button) | A focusable action button with semantic variants, loading state, and keyboard activation. |
//! | [`calendar`](mod@crate::calendar) | [`calendar`](fn@crate::calendar::calendar) | A month grid with selectable dates and previous/next navigation requests. |
//! | [`calendar`](mod@crate::calendar) | [`date_picker`](fn@crate::calendar::date_picker) | A date-selection popover combining a supplied trigger and month calendar. |
//! | [`collections`](mod@crate::collections) | [`accordion`](fn@crate::collections::accordion) | A single-expanded-item accordion with optional collapse behavior. |
//! | [`collections`](mod@crate::collections) | [`breadcrumb`](fn@crate::collections::breadcrumb) | A breadcrumb trail with callback-driven navigation and a configurable separator. |
//! | [`collections`](mod@crate::collections) | [`collapsible`](fn@crate::collections::collapsible) | A disclosure section showing content when its trigger is activated. |
//! | [`collections`](mod@crate::collections) | [`data_grid`](fn@crate::collections::data_grid) | A tabular string-data surface with sortable headers and selectable rows. |
//! | [`collections`](mod@crate::collections) | [`data_table`](fn@crate::collections::data_table) | A presentational table of arbitrary element cells with optional headers. |
//! | [`collections`](mod@crate::collections) | [`pagination`](fn@crate::collections::pagination) | A page-navigation row with endpoint buttons and visible sibling pages. |
//! | [`collections`](mod@crate::collections) | [`tree_view`](fn@crate::collections::tree_view) | A tree with independent controlled selection and expansion. |
//! | [`command`](mod@crate::command) | [`command_palette`](fn@crate::command::command_palette) | A dialog listing commands filtered by a caller-supplied query. |
//! | [`controls`](mod@crate::controls) | [`avatar`](fn@crate::controls::avatar) | A circular avatar showing a supplied image element or text fallback. |
//! | [`controls`](mod@crate::controls) | [`badge`](fn@crate::controls::badge) | A compact text badge with semantic color variants. |
//! | [`controls`](mod@crate::controls) | [`card`](fn@crate::controls::card) | A themed, bordered content panel with rounded corners. |
//! | [`controls`](mod@crate::controls) | [`checkbox`](fn@crate::controls::checkbox) | A labeled checkbox supporting controlled, uncontrolled, and indeterminate states. |
//! | [`controls`](mod@crate::controls) | [`progress`](fn@crate::controls::progress) | A determinate progress bar with accessible numeric value metadata. |
//! | [`controls`](mod@crate::controls) | [`radio_group`](fn@crate::controls::radio_group) | A group of mutually exclusive radio choices with roving keyboard focus. |
//! | [`controls`](mod@crate::controls) | [`separator`](fn@crate::controls::separator) | A decorative horizontal or vertical divider. |
//! | [`controls`](mod@crate::controls) | [`slider`](fn@crate::controls::slider) | A numeric slider with stepped pointer and keyboard adjustment. |
//! | [`controls`](mod@crate::controls) | [`switch`](fn@crate::controls::switch) | A boolean switch with a sliding thumb and optional label. |
//! | [`dropdown`](mod@crate::dropdown) | [`drop_down`](fn@crate::dropdown::drop_down) | An accessible option picker with a portal-mounted dropdown list. |
//! | [`feedback`](mod@crate::feedback) | [`callout`](fn@crate::feedback::callout) | A bordered message panel with an optional title and semantic tone. |
//! | [`feedback`](mod@crate::feedback) | [`skeleton`](fn@crate::feedback::skeleton) | A static, decorative placeholder for content that has not loaded. |
//! | [`feedback`](mod@crate::feedback) | [`spinner`](fn@crate::feedback::spinner) | An indeterminate loading indicator rendered as a rotating cached vector icon. |
//! | [`forms`](mod@crate::forms) | [`form_field`](fn@crate::forms::form_field) | A labeled form-control wrapper with description and validation feedback. |
//! | [`image`](mod@crate::image) | [`image`](fn@crate::image::image) | An asynchronous image loaded from a packaged asset, URL, or local path. |
//! | [`input`](mod@crate::input) | [`input`](fn@crate::input::input) | A single-line text field with IME, validation styling, and durable editing state. |
//! | [`overlays`](mod@crate::overlays) | [`context_menu`](fn@crate::overlays::context_menu) | A menu opened by semantic context-menu events at a pointer or keyboard-requested position. |
//! | [`overlays`](mod@crate::overlays) | [`dialog`](fn@crate::overlays::dialog) | A modal portal containing a titled dialog panel and backdrop. |
//! | [`overlays`](mod@crate::overlays) | [`drawer`](fn@crate::overlays::drawer) | An edge-mounted modal sheet with a configurable side and backdrop. |
//! | [`overlays`](mod@crate::overlays) | [`menu`](fn@crate::overlays::menu) | A trigger-based menu with disabled actions and keyboard focus navigation. |
//! | [`overlays`](mod@crate::overlays) | [`popover`](fn@crate::overlays::popover) | An anchored portal opened by a focusable trigger. |
//! | [`overlays`](mod@crate::overlays) | [`toast`](fn@crate::overlays::toast) | A portal-mounted transient message with a semantic tone and live announcement. |
//! | [`overlays`](mod@crate::overlays) | [`tooltip`](fn@crate::overlays::tooltip) | A non-interactive anchored hint shown on pointer hover or descendant focus. |
//! | [`tabs`](mod@crate::tabs) | [`tabs`](fn@crate::tabs::tabs) | An accessible tab list with one mounted content panel and configurable keyboard activation. |
//! | [`virtual_list`](mod@crate::virtual_list) | [`virtual_list`](fn@crate::virtual_list::virtual_list) | A fixed-row-height scrolling list that materializes only the visible window and overscan. |
//!
//! # Aliases
//!
//! | Alias | Implementation | Behavior |
//! | --- | --- | --- |
//! | [`select`](fn@crate::select) | [`drop_down`](fn@crate::drop_down) | Non-editable option picker. |
//! | [`combo_box`](fn@crate::combo_box) | [`drop_down`](fn@crate::drop_down) | The same non-editable picker; no search editor is added. |
//! | [`sheet`](fn@crate::sheet) | [`drawer`](fn@crate::drawer) | The same edge-mounted modal panel. |
//!
//! # Shared contracts
//!
//! - Optional controlled values are the source of truth. `default_*` props seed
//!   uncontrolled state; callbacks report requested changes. Pure presentations
//!   such as calendars, tables, and toasts keep their data in the caller.
//! - Every component accepts `style`. Individual `*_style` props override defaults;
//!   [`TabsStyle`] and [`DropDownStyle`] provide complete visual-slot styles.
//! - Disabled interactive controls ignore activation and leave sequential focus
//!   navigation. Keyboard behavior is described on each component.
//! - Input uses a durable [`TextController`](xui_core::widgets::TextController). An omitted controller is retained
//!   internally; a supplied controller owns external text and selection state.
//! - Anchored portals follow their trigger and handle viewport collisions.
//!   Modal portals block background pointer input; automatic focus trapping and
//!   restoration are not part of the current portal API.
//! - [`virtual_list`](fn@crate::virtual_list) bounds row construction by viewport
//!   height and overscan. Presentational tables materialize all supplied rows.
//!
//! # Supporting modules
//!
//! | Module | Types and responsibilities |
//! | --- | --- |
//! | [`layout`](mod@crate::layout) | Optional length, sizing, inset, and color overrides. |
//! | [`selection`](mod@crate::selection) | Enabled-item navigation, activation modes, and buffered typeahead matching. |
//! | [`calendar`](mod@crate::calendar) | Gregorian date validation and normalized month arithmetic. |
//! | [`image`](mod@crate::image) | Image sources, sizing, opacity, and load errors. |
//!
//! # Image imports
//!
//! The image component is available as [`image::image`](fn@crate::image::image) rather
//! than a crate-root function, to distinguish it from the host image widget. Use
//! `use xui_components::image::image;` for an unqualified `view! { image(...) }`.
//! `ImageSrc` and `ImageError` are also re-exported at the crate root.

// Ported to `#[component]` one module at a time; see `docs/reactive.md`.
pub mod button;
pub mod calendar;
pub mod collections;
pub mod command;
pub mod controls;
pub mod dropdown;
pub mod feedback;
pub mod image;
pub mod forms;
pub mod input;
pub mod overlays;
pub mod layout;
pub mod selection;
pub mod tabs;
pub mod virtual_list;
pub use button::*;
pub use calendar::*;
pub use collections::*;
pub use command::*;
pub use controls::*;
pub use dropdown::*;
pub use feedback::*;
pub use forms::*;
pub use image::ImageError;
pub use image::ImageSrc;
pub use input::*;
pub use overlays::*;
/// The drawer component under a sheet-style name.
pub use overlays::{Drawer as Sheet, drawer as sheet};
pub use layout::*;
pub use selection::*;
pub use tabs::*;
pub use virtual_list::*;

// An alias is a tag and its marker, since a parent finds the marker by the
// tag's name (`select(..)` mounts through `Select`).
/// The dropdown component under a familiar native-control name.
pub use dropdown::{DropDown as Select, drop_down as select};
/// The dropdown component; accepts the same props as [`select`].
pub use dropdown::{DropDown as ComboBox, drop_down as combo_box};
