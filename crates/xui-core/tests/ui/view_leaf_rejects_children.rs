use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn canvas_with_body(controller: &CanvasController) {
    view! { canvas(controller: controller.clone()) { text("canvas takes no body") } }
}
