use xui::prelude::*;

#[component]
fn callback_component_prop(label: &String, on_select: &Callback<usize>) {
    let _ = on_select;
    xui! {
        <container accessibility_role={AccessibilityRole::Tab}>
            <text>{label}</text>
        </container>
    }
}

#[test]
fn callback_is_valid_in_component_props() {
    let _ = callback_component_prop_component_render();
}

struct NotHash(u32);

#[component]
fn non_hash_component_prop(value: &NotHash, suffix: &String) {
    xui! {
        <text>{format!("{}{}", value.0, suffix)}</text>
    }
}

#[test]
fn component_props_do_not_need_to_implement_hash() {
    let element = xui! {
        <non_hash_component_prop value={NotHash(7)} suffix={String::from("!")} />
    };
    assert!(matches!(element, ElementDesc::Component(_)));
}
