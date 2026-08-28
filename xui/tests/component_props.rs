//! `#[component]` derives `{Name}Props` from the parameters after `cx`, for any
//! number of them. A one-parameter component used to be read as "this argument
//! *is* the props struct", which left `{Name}Props` undefined and made the
//! component unusable from `xui!`.

use xui::prelude::*;

#[component]
fn one_prop(label: &String) {
    TextWidget::new(label.clone()).into_element_desc()
}

#[component]
#[defaults(label = "fallback".to_string())]
fn one_prop_with_default(label: &String) {
    TextWidget::new(label.clone()).into_element_desc()
}

#[component]
fn two_props(label: &String, size: &f32) {
    TextWidget::new(format!("{label}:{size}")).into_element_desc()
}

#[component]
fn no_props() {
    container().into_element_desc(Vec::new())
}

#[component]
fn one_prop_forwarding_children(children: &Vec<ElementDesc>) {
    xui! {
        <container>
            {children}
        </container>
    }
}

#[test]
fn a_single_prop_component_is_usable_from_the_macro() {
    let _: ElementDesc = xui! { <one_prop label={"hello".to_string()} /> };
}

#[test]
fn a_single_prop_component_accepts_defaults() {
    let _: ElementDesc = xui! { <one_prop_with_default /> };
    let _: ElementDesc = xui! { <one_prop_with_default label={"given".to_string()} /> };
}

#[test]
fn multiple_and_zero_prop_components_still_work() {
    let _: ElementDesc = xui! { <two_props label={"a".to_string()} size={2.0} /> };
    let _: ElementDesc = xui! { <no_props /> };
}

/// The generated props struct is what `xui!` builds, so it has to exist by name
/// and take the parameter as a field.
#[test]
fn the_props_struct_is_named_after_the_component() {
    let props = OnePropProps::builder().label("direct".to_string()).build();
    assert_eq!(props.label, "direct");

    let defaulted = OnePropWithDefaultProps::builder().build();
    assert_eq!(defaulted.label, "fallback");
}

/// The macro puts its support items in a per-component module and re-exports
/// only what call sites name: the props struct, the render function, and the
/// handle. Everything else -- the builder, the typestate markers, the erased
/// call adapter -- stays reachable but out of the way, so a `pub use foo::*`
/// re-export brings in a fixed handful of names instead of one set per prop.
#[test]
fn support_items_live_behind_a_per_component_module() {
    // Reachable through the module, absent from module scope.
    let _ = __xui_one_prop::one_prop_component_type();
    let _: __xui_one_prop::OnePropPropsLabelSet = __xui_one_prop::OnePropPropsLabelSet;

    // The facade.
    let _ = one_prop_component_render();
    let _ = OnePropProps::builder().label("x".to_string()).build();
}

/// A single-prop component must also accept children, which routes through
/// `WithChildren` on the generated props rather than a plain builder call.
#[test]
fn a_single_prop_component_accepts_children() {
    let element = xui! {
        <one_prop_forwarding_children>
            <container key={"child-a"} />
            <container key={"child-b"} />
        </one_prop_forwarding_children>
    };
    assert!(matches!(element, ElementDesc::Component(_)));
}
