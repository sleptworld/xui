use xui::prelude::*;
use xui::xui;

fn main() {}

fn mismatched() -> ElementDesc {
    xui! { <container><text>{"hi"}</text></grid> }
}
