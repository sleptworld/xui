use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn leaf(label: &String) {
    view! { text(label.clone()) }
}

#[component]
fn with_body() {
    view! { column { leaf(label: String::from("hi")) { text("unexpected") } } }
}
