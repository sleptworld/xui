//! `#[component]` derives `{Name}Props` from the parameters after `cx`, for any
//! number of them. A one-parameter component used to be read as "this argument
//! *is* the props struct", which left `{Name}Props` undefined and made the
//! component unusable from `xui!`.

use xui_core::prelude::*;

use xui_macros::{component, xui};

#[component]
fn one_prop(label: &String) {
    text().with::<text::content>(label.clone()).into_element_desc(Vec::new())
}

#[component]
#[defaults(label = "fallback".to_string())]
fn one_prop_with_default(label: &String) {
    text().with::<text::content>(label.clone()).into_element_desc(Vec::new())
}

#[component]
fn two_props(label: &String, size: &f32) {
    text().with::<text::content>(format!("{label}:{size}")).into_element_desc(Vec::new())
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

/// The generated props struct is what the render function reads, so it has to
/// exist by name and take the parameter as a field; the tag's draft holds each
/// prop until the element is built.
#[test]
fn the_props_struct_is_named_after_the_component() {
    let props = OnePropProps {
        label: Prop::new("direct".to_string()),
    };
    assert_eq!(*props.label, "direct");

    let draft = one_prop().with::<__xui_one_prop::label>("direct");
    assert_eq!(draft.storage.props.label.as_deref().map(String::as_str), Some("direct"));
    // A default is applied when the element is built, not stored in the draft.
    assert!(one_prop_with_default().storage.props.label.is_none());
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
    let _: xui_core::node::Draft<__xui_one_prop::OneProp, _> = one_prop();

    // The facade.
    let _ = one_prop_component_render();
    let _ = OnePropProps {
        label: Prop::new("x".to_string()),
    };
}

/// A single-prop component must also accept children, which its draft takes as
/// the `children` prop.
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
