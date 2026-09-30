use xui_core::prelude::*;
use xui_macros::view;

fn main() {}

fn padded() -> ElementDesc {
    // A named argument and a modifier are the same setter.
    view! { container(padding: EdgeInsets::all(4.0)).padding(EdgeInsets::all(8.0)) }
}
