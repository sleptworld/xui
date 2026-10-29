use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn misspelled() {
    view! { container().paddin(EdgeInsets::all(8.0)) }
}
