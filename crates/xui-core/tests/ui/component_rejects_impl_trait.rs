use xui_macros::component;

fn main() {}

#[component]
fn shown(value: &impl std::fmt::Display) {
    view! { text(value.to_string()) }
}
