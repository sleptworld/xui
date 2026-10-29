use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn mismatched() {
    xui! { <container><text>{"hi"}</text></grid> }
}
