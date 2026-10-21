use xui_core::prelude::*;
use xui_macros::{component, xui};

fn main() {}

mod shadow {
    use super::*;

    #[component]
    pub fn text(label: &String) {
        xui! { <container /> }
    }
}

use shadow::text;

fn shadowed() -> ElementDesc {
    xui! { <text label={String::from("x")} /> }
}
