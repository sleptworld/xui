use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn bare_path(rows: &u32) {
    view! { column { rows } }
}

#[component]
fn content_and_body() {
    view! { column("x") { text("a") } }
}

#[component]
fn content_after_named() {
    view! { text(color: Color::BLACK, "x") }
}

#[component]
fn two_roots() {
    view! { text("a") text("b") }
}

#[component]
fn empty_modifier() {
    view! { container().clip() }
}
