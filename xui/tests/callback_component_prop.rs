use xui::prelude::*;

#[component]
fn callback_component_prop(label: &String, on_select: &Callback<fn(usize)>) {
    let _ = on_select;
    xui! {
        <container accessibility_role={AccessibilityRole::Tab}>
            <text>{label}</text>
        </container>
    }
}

#[test]
fn callback_is_valid_in_hash_derived_component_props() {
    let _ = callback_component_prop_component_render();
}
