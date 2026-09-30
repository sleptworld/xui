use xui_core::prelude::*;
use xui_macros::view;

fn main() {}

fn bare_path(rows: Vec<ElementDesc>) -> ElementDesc {
    view! { column { rows } }
}

fn braced_splice(rows: Vec<ElementDesc>) -> ElementDesc {
    view! { column { text("a") {rows} } }
}

fn content_and_body() -> ElementDesc {
    view! { column("x") { text("a") } }
}

fn content_after_named() -> ElementDesc {
    view! { text(color: Color::BLACK, "x") }
}

fn two_roots() -> ElementDesc {
    view! { text("a") text("b") }
}

fn empty_modifier() -> ElementDesc {
    view! { container().clip() }
}
