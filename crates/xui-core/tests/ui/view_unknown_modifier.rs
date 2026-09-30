use xui_core::prelude::*;
use xui_macros::view;

fn main() {}

fn misspelled() -> ElementDesc {
    view! { container().paddin(EdgeInsets::all(8.0)) }
}
