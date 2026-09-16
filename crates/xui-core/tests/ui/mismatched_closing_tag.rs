use xui_core::prelude::*;
use xui_core::xui;

fn main() {}

fn mismatched() -> ElementDesc {
    xui! { <container><text>{"hi"}</text></grid> }
}
