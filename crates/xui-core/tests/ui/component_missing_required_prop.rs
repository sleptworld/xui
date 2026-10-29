use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn badge(label: &String, tone: &String) {
    view! { text(format!("{label}/{tone}")) }
}

#[component]
fn missing() {
    view! { column { badge(label: String::from("hi")) } }
}
