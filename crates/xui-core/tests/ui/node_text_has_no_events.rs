//! Handlers exist only on a node that selects the `events` capability.

#[path = "../node_fixture/mod.rs"]
mod common;

use xui_core::caps::behavior::events::props::on_click;
use xui_core::node::Node;

fn main() {
    let mut label = common::text("a");
    label.on::<on_click, _>(|_, _| {});
}
