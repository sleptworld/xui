//! Naming the property by its full path still fails: `Text` does not select
//! `BoxSize`, so it has no storage for it.

#[path = "../node_fixture/mod.rs"]
mod common;

use xui_core::caps::style::box_size::props::width;
use xui_core::node::Node;

fn main() {
    let mut label = common::text("a");
    label.set::<width>(10.0);
}
