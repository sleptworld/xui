use xui_core::prelude::*;
use xui_macros::xui;

fn main() {}

fn mismatched() -> ElementDesc {
    xui! { <container><text>{"hi"}</text></grid> }
}
