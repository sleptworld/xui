//! A component prop is a `Property` but not a `HostProperty`.

#[path = "../node_fixture/mod.rs"]
mod common;

use xui_core::property::Slot;

fn main() {
    let mut node = common::counter();
    Slot::<common::counter::count>::new().update(&mut node, 1);
}
