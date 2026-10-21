//! A property names the inputs it accepts; anything else is rejected with
//! the property's own message.

#[path = "../node_fixture/mod.rs"]
mod common;

use common::{container, text};
use xui_core::node::Node;

fn main() {
    let mut node = common::container();
    node.set::<container::flex_wrap>("yes");
    node.set::<container::width>("wide");

    let mut label = common::text("a");
    label.set::<text::content>(3_u8);
}
