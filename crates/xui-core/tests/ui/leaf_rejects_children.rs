use xui_core::prelude::*;
use xui_macros::xui;

fn main() {}

fn canvas_with_body(controller: CanvasController) -> ElementDesc {
    xui! { <canvas controller={controller}>{"canvas takes no body"}</canvas> }
}
