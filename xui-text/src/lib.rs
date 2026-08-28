//! `swash`/`fontique` text stack for `xui`.
//!
//! One of two text backends (the other is `xui-text-engine`, backed by
//! `cosmic-text`). Provides glyph shaping, scaling, atlas bin-packing,
//! bidirectional layout, line breaking, and a rich-text document model. `xui`
//! re-exports `xui_text::engine::Engine` as `xui::text::Engine`.
//!
//! Modules: `atlas`, `bidi`, `doc`, `engine`, `fontique_library`, `layout`,
//! `line_breaker`, `par`, `span`, `typ`.
//!
//! Application code rarely touches this crate directly; the `TextHost` in
//! `xui::app::App` selects a text backend.

pub mod atlas;
pub mod bidi;
pub mod doc;
pub mod engine;
pub mod fontique_library;
pub mod layout;
pub mod line_breaker;
pub mod par;
pub mod span;
pub mod typ;
