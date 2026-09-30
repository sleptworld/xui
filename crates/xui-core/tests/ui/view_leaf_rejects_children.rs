use xui_core::prelude::*;
use xui_macros::view;

fn main() {}

fn canvas_with_body(controller: CanvasController) -> ElementDesc {
    view! { canvas(controller: controller) { text("canvas takes no body") } }
}
