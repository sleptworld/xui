//! `share()` through the real runtime: a prop forwarded down several
//! components is read from the allocation the first one received.

use std::cell::RefCell;

use xui_core::prelude::*;
use xui_core::render::backend::MockRenderBackend;
use xui_core::text::TextHost;
use xui_cosmic::{CosmicEngine, FontSet};
use xui_interface::Size;
use xui_macros::{component, xui};

thread_local! {
    /// `(component, address of its items' buffer, address of its label)`.
    static SEEN: RefCell<Vec<(String, usize, usize)>> = const { RefCell::new(Vec::new()) };
}

fn record(name: &str, items: &[u32], label: &str) {
    SEEN.with(|seen| {
        seen.borrow_mut()
            .push((name.to_string(), items.as_ptr() as usize, label.as_ptr() as usize))
    });
}

#[component]
fn outer(items: &Vec<u32>, label: &str) {
    record("outer", items, label);
    let local = vec![7, 8];
    xui! {
        <container>
            <middle items={items.share()} label={label.share()} />
            <generic_leaf name={"copied"} items={local.share()} />
        </container>
    }
}

#[component]
fn middle(items: &Vec<u32>, label: &str) {
    record("middle", items, label);
    xui! {
        <container>
            <leaf items={items.share()} label={label.share()} />
            <generic_leaf name={"generic"} items={items.share()} />
        </container>
    }
}

#[component]
fn leaf(items: &Vec<u32>, label: &str) {
    record("leaf", items, label);
    container().into_element_desc(Vec::new())
}

#[component]
fn generic_leaf<T: Clone>(name: &str, items: &Vec<T>) {
    record(name, &[], "");
    SEEN.with(|seen| seen.borrow_mut().last_mut().unwrap().1 = items.as_ptr() as usize);
    container().into_element_desc(Vec::new())
}

fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
    xui! { <outer items={vec![1, 2, 3]} label={"shared"} /> }
}

#[test]
fn a_shared_prop_is_never_copied_on_the_way_down() {
    let mut app = App::new(root);
    app.resize(Size::new(200.0, 200.0));
    let mut backend = MockRenderBackend::default();
    let mut text = TextHost::new(CosmicEngine::with_fonts(
        1.0,
        FontSet::Only {
            sources: Vec::new(),
            sans_serif_family: None,
        },
    ));
    app.render(&mut backend, &mut text).expect("mock backend");

    let seen = SEEN.with(|seen| seen.borrow_mut().drain(..).collect::<Vec<_>>());
    let find = |name: &str| seen.iter().find(|(seen, ..)| seen == name).unwrap().clone();
    let (_, items, label) = find("outer");

    for name in ["middle", "leaf"] {
        assert_eq!(find(name), (name.to_string(), items, label), "{name} copied its props");
    }
    assert_eq!(find("generic").1, items, "a generic prop copied its value");
    // A local is not a prop: sharing it clones it into the child.
    assert_ne!(find("copied").1, items);
}
