use xui_core::prelude::*;
use xui_macros::component;

fn main() {}

mod shadow {
    use super::*;

    /// Reached by its path; a bare `text` is the host.
    #[component]
    pub fn text(label: &String) {
        view! { container() }
    }
}

use shadow::text;

#[component]
fn shadowed() {
    view! { column { text(label: String::from("x")) } }
}
