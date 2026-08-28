//! Built-in widget library for `xui`.
//!
//! Each widget is a `#[component]` with typed props and a builder, so call sites
//! use them through the `xui!` macro just like user components:
//!
//! - `button` — `ButtonVariant` (Primary/Secondary/Outline), `on_click`, label.
//! - `input` — wraps `xui::widgets::text_input`; `TextController`, sizing.
//! - `dropdown` — `DropDownItem` list, `on_change`.
//! - `tabs` — `TabItem` with label and content, keyboard navigation.
//! - `image` — loads from `AssetId` or URL/path; decodes jpeg/png.
//! - `virtual_list` — only materializes rows near the viewport.
//! - `layout` — shared sizing/color helpers (`ComponentLength`,
//!   `ComponentSizing`, `ComponentInsets`, `ComponentColor`).
//!
//! Most modules are re-exported at the crate root. `image` is not: its
//! component shares a name with the `xui::image` host widget, so importing it
//! explicitly (`use xui_components::image::image;`) is what disambiguates the
//! `<image>` tag — an explicit import wins over both globs.

pub mod button;
pub mod dropdown;
pub mod image;
pub mod input;
pub mod layout;
pub mod tabs;
pub mod virtual_list;

pub use button::*;
pub use dropdown::*;
pub use input::*;

// `ImageError` and `ImageSrc` are defined identically in both `icon` and
// `image`; pin the canonical crate-root re-exports to `image` so the glob
// re-exports are not ambiguous.
pub use image::ImageError;
pub use image::ImageSrc;
pub use layout::*;
pub use tabs::*;
pub use virtual_list::*;
