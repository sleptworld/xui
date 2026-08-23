use xui::prelude::*;

#[component]
fn forwarding_component(children: &Vec<ElementDesc>) {
    xui! {
        <container>
            {children}
        </container>
    }
}

fn keyed(key: &'static str) -> ElementDesc {
    container().key(key).into_element_desc(Vec::new())
}

#[test]
fn braced_children_expand_single_and_collection_values_in_order() {
    let borrowed = vec![keyed("borrowed-a"), keyed("borrowed-b")];
    let optional = Some(keyed("optional"));
    let absent: Option<ElementDesc> = None;
    let array = [keyed("array-a"), keyed("array-b")];

    let element = xui! {
        <container>
            {keyed("single")}
            {&borrowed}
            {optional}
            {absent}
            {array}
        </container>
    };

    let ElementDesc::Host(host) = element else {
        panic!("container must expand to a host element");
    };
    let keys = host
        .children
        .iter()
        .map(ElementDesc::key)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            Some("single".into()),
            Some("borrowed-a".into()),
            Some("borrowed-b".into()),
            Some("optional".into()),
            Some("array-a".into()),
            Some("array-b".into()),
        ]
    );
}

#[test]
fn component_children_can_be_forwarded_with_a_braced_expression() {
    let _ = forwarding_component_component_render();
}
