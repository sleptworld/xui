//! The built-in declarative component library for XUI.
//!
//! All 36 component implementations are defined with `#[component]` and composed with
//! `view!`. They share themed styles, typed props, and semantic event handling. The
//! component entry points are documented below; macro-generated render functions,
//! handles, builders, and typestate markers are hidden from the rendered API.
//!
//! # Quick start
//!
//! ```rust
//! use xui_core::prelude::*;
//! use xui_components::*;
//! use xui_macros::view;
//!
//! let content = view! {
//!     column(gap: 12.0) {
//!         xui_components::Input(accessibility_label: Some("Email".to_owned()), required: true)
//!         Button(text: "Save", variant: ButtonVariant::Primary)
//!     }
//! };
//! ```
//!
//! # Component catalog
//!
//! | Module | Component | Purpose |
//! | --- | --- | --- |
//! | [`button`](mod@crate::button) | [`Button`](fn@crate::button::Button) | A focusable action button with semantic variants, loading state, and keyboard activation. |
//! | [`calendar`](mod@crate::calendar) | [`Calendar`](fn@crate::calendar::Calendar) | A month grid with selectable dates and previous/next navigation requests. |
//! | [`calendar`](mod@crate::calendar) | [`DatePicker`](fn@crate::calendar::DatePicker) | A date-selection popover combining a supplied trigger and month calendar. |
//! | [`collections`](mod@crate::collections) | [`Accordion`](fn@crate::collections::Accordion) | A single-expanded-item accordion with optional collapse behavior. |
//! | [`collections`](mod@crate::collections) | [`Breadcrumb`](fn@crate::collections::Breadcrumb) | A breadcrumb trail with callback-driven navigation and a configurable separator. |
//! | [`collections`](mod@crate::collections) | [`Collapsible`](fn@crate::collections::Collapsible) | A disclosure section showing content when its trigger is activated. |
//! | [`collections`](mod@crate::collections) | [`DataGrid`](fn@crate::collections::DataGrid) | A tabular string-data surface with sortable headers and selectable rows. |
//! | [`collections`](mod@crate::collections) | [`DataTable`](fn@crate::collections::DataTable) | A presentational table of arbitrary element cells with optional headers. |
//! | [`collections`](mod@crate::collections) | [`Pagination`](fn@crate::collections::Pagination) | A page-navigation row with endpoint buttons and visible sibling pages. |
//! | [`collections`](mod@crate::collections) | [`TreeView`](fn@crate::collections::TreeView) | A tree with independent controlled selection and expansion. |
//! | [`command`](mod@crate::command) | [`CommandPalette`](fn@crate::command::CommandPalette) | A dialog listing commands filtered by a caller-supplied query. |
//! | [`controls`](mod@crate::controls) | [`Avatar`](fn@crate::controls::Avatar) | A circular avatar showing a supplied image element or text fallback. |
//! | [`controls`](mod@crate::controls) | [`Badge`](fn@crate::controls::Badge) | A compact text badge with semantic color variants. |
//! | [`controls`](mod@crate::controls) | [`Card`](fn@crate::controls::Card) | A themed, bordered content panel with rounded corners. |
//! | [`controls`](mod@crate::controls) | [`Checkbox`](fn@crate::controls::Checkbox) | A labeled checkbox supporting controlled, uncontrolled, and indeterminate states. |
//! | [`controls`](mod@crate::controls) | [`Progress`](fn@crate::controls::Progress) | A determinate progress bar with accessible numeric value metadata. |
//! | [`controls`](mod@crate::controls) | [`RadioGroup`](fn@crate::controls::RadioGroup) | A group of mutually exclusive radio choices with roving keyboard focus. |
//! | [`controls`](mod@crate::controls) | [`Separator`](fn@crate::controls::Separator) | A decorative horizontal or vertical divider. |
//! | [`controls`](mod@crate::controls) | [`Slider`](fn@crate::controls::Slider) | A numeric slider with stepped pointer and keyboard adjustment. |
//! | [`controls`](mod@crate::controls) | [`Switch`](fn@crate::controls::Switch) | A boolean switch with a sliding thumb and optional label. |
//! | [`dropdown`](mod@crate::dropdown) | [`DropDown`](fn@crate::dropdown::DropDown) | An accessible option picker with a portal-mounted dropdown list. |
//! | [`feedback`](mod@crate::feedback) | [`Callout`](fn@crate::feedback::Callout) | A bordered message panel with an optional title and semantic tone. |
//! | [`feedback`](mod@crate::feedback) | [`Skeleton`](fn@crate::feedback::Skeleton) | A static, decorative placeholder for content that has not loaded. |
//! | [`feedback`](mod@crate::feedback) | [`Spinner`](fn@crate::feedback::Spinner) | An indeterminate loading indicator rendered as a rotating cached vector icon. |
//! | [`forms`](mod@crate::forms) | [`FormField`](fn@crate::forms::FormField) | A labeled form-control wrapper with description and validation feedback. |
//! | [`image`](mod@crate::image) | [`Image`](fn@crate::image::Image) | An asynchronous image loaded from a packaged asset, URL, or local path. |
//! | [`input`](mod@crate::input) | [`Input`](fn@crate::input::Input) | A single-line text field with IME, validation styling, and durable editing state. |
//! | [`overlays`](mod@crate::overlays) | [`ContextMenu`](fn@crate::overlays::ContextMenu) | A menu opened by semantic context-menu events at a pointer or keyboard-requested position. |
//! | [`overlays`](mod@crate::overlays) | [`Dialog`](fn@crate::overlays::Dialog) | A modal portal containing a titled dialog panel and backdrop. |
//! | [`overlays`](mod@crate::overlays) | [`Drawer`](fn@crate::overlays::Drawer) | An edge-mounted modal sheet with a configurable side and backdrop. |
//! | [`overlays`](mod@crate::overlays) | [`Menu`](fn@crate::overlays::Menu) | A trigger-based menu with disabled actions and keyboard focus navigation. |
//! | [`overlays`](mod@crate::overlays) | [`Popover`](fn@crate::overlays::Popover) | An anchored portal opened by a focusable trigger. |
//! | [`overlays`](mod@crate::overlays) | [`Toast`](fn@crate::overlays::Toast) | A portal-mounted transient message with a semantic tone and live announcement. |
//! | [`overlays`](mod@crate::overlays) | [`Tooltip`](fn@crate::overlays::Tooltip) | A non-interactive anchored hint shown on pointer hover or descendant focus. |
//! | [`tabs`](mod@crate::tabs) | [`Tabs`](fn@crate::tabs::Tabs) | An accessible tab list with one mounted content panel and configurable keyboard activation. |
//! | [`virtual_list`](mod@crate::virtual_list) | [`VirtualList`](fn@crate::virtual_list::VirtualList) | A fixed-row-height scrolling list that materializes only the visible window and overscan. |
//!
//! # Aliases
//!
//! | Alias | Implementation | Behavior |
//! | --- | --- | --- |
//! | [`Select`](fn@crate::Select) | [`DropDown`](fn@crate::DropDown) | Non-editable option picker. |
//! | [`ComboBox`](fn@crate::ComboBox) | [`DropDown`](fn@crate::DropDown) | The same non-editable picker; no search editor is added. |
//! | [`Sheet`](fn@crate::Sheet) | [`Drawer`](fn@crate::Drawer) | The same edge-mounted modal panel. |
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
//! - [`VirtualList`](fn@crate::VirtualList) bounds row construction by viewport
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
//! The image component is available as [`image::Image`](fn@crate::image::Image) rather
//! than a crate-root function, to distinguish it from the host image widget. Use
//! `use xui_components::image::Image;` for an unqualified `view! { Image(...) }`.
//! `ImageSrc` and `ImageError` are also re-exported at the crate root.

pub mod button;
pub mod dropdown;
pub mod feedback;
pub mod image;
pub mod input;
pub mod layout;
pub mod tabs;
pub mod virtual_list;

pub use button::*;
pub use dropdown::*;
pub use feedback::*;
pub use input::*;

pub use image::ImageError;
pub use image::ImageSrc;
pub use layout::*;
pub use tabs::*;
pub use virtual_list::*;

pub mod selection;
pub use selection::*;
pub mod controls;
pub use controls::*;
pub mod forms;
pub use forms::*;
pub mod overlays;
pub use overlays::*;
pub mod collections;
pub use collections::*;
pub mod calendar;
pub use calendar::*;
pub mod command;
pub use command::*;

/// The dropdown component under a familiar native-control name.
pub use dropdown::DropDown as Select;
/// The dropdown component; accepts the same props as [`Select`].
pub use dropdown::DropDown as ComboBox;
/// The drawer component under a sheet-style name.
pub use overlays::Drawer as Sheet;
