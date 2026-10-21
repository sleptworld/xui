//! A property the node did not select is not even nameable through its tag.

#[path = "../node_fixture/mod.rs"]
mod common;

use common::text;
use xui_core::node::Node;

fn main() {
    let mut label = text("a");
    label.set::<common::text::width>(10.0);
}
