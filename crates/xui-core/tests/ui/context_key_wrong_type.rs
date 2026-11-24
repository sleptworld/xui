use xui_core::prelude::*;
use xui_core::system::window;
use xui_macros::component;

fn main() {}

#[component]
fn title_bar() {
    // `window::Maximized` holds a `bool`.
    let maximized: f32 = context!(window::Maximized);
    view! { text(format!("{maximized}")) }
}
