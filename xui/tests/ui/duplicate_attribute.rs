use xui::prelude::*;
use xui::xui;

fn main() {}

fn duplicated() -> ElementDesc {
    xui! { <container width={10.0} height={4.0} width={20.0} /> }
}
