use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn padded() {
    // A named argument and a modifier are the same setter.
    view! { container(padding: EdgeInsets::all(4.0)).padding(EdgeInsets::all(8.0)) }
}
