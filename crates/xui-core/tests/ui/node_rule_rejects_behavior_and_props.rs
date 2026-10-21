//! A state rule is a style layer: it cannot reach a prop or a behaviour
//! capability.

#[path = "../node_fixture/mod.rs"]
mod common;

use common::container;
use xui_interface::WidgetState;
use xui_interface::style::WidgetStateMatcher;

fn main() {
    let _ = common::container().when(WidgetStateMatcher::all(WidgetState::HOVERED), |layer| {
        layer.set::<container::tab_index>(1);
        layer.set::<container::flex_wrap>(true);
    });
}
