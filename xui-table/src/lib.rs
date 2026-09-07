//! Advanced, model-driven tables for XUI.
//!
//! `xui-table` separates deterministic row processing from rendering. The
//! [`TableModel`] can be tested without a UI, while [`advanced_table`] adds
//! custom cells, sorting, filtering, pagination, selection, keyboard grid
//! navigation, column resizing callbacks, and viewport-bounded row rendering.

mod component;
mod model;

pub use component::*;
pub use model::*;
