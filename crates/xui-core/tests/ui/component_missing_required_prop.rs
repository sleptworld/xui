use xui_core::prelude::*;
use xui_macros::{component, xui};

fn main() {}

#[component]
fn badge(label: &String, tone: &String) {
    xui! { <text>{format!("{label}/{tone}")}</text> }
}

fn missing() -> ElementDesc {
    xui! { <badge label={String::from("hi")} /> }
}
