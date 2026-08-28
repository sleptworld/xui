use xui::prelude::*;
use xui::{component, xui};

fn main() {}

#[component]
fn leaf(label: &String) {
    xui! { <text>{label.clone()}</text> }
}

fn with_body() -> ElementDesc {
    xui! { <leaf label={String::from("hi")}>{"unexpected"}</leaf> }
}
