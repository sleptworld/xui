use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

#[component]
fn duplicated() {
    xui! { <container width={10.0} height={4.0} width={20.0} /> }
}
