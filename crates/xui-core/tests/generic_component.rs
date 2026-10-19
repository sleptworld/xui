//! `#[component]` on generic functions: the component's parameters are carried
//! by its props type, inferred from the props that name them, and give every
//! monomorphization its own identity in the fiber tree.

use std::cell::RefCell;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

use xui_core::prelude::*;
use xui_core::render::backend::MockRenderBackend;
use xui_core::state::State;
use xui_core::text::TextHost;
use xui_cosmic::{CosmicEngine, FontSet};
use xui_interface::Size;
use xui_macros::{component, xui};

#[component]
fn labeled<T: Display>(value: &T) {
    TextWidget::new(value.to_string()).into_element_desc()
}

#[component]
#[defaults(items = Vec::new())]
fn listing<T>(items: &Vec<T>, children: &Vec<ElementDesc>)
where
    T: Display,
{
    let _ = items;
    xui! { <container>{children}</container> }
}

/// `F` appears only behind the bound's own parentheses and arrow, which the
/// signature scanner must not mistake for the parameter list.
#[component]
fn rendered<F: Fn(i32) -> String>(render: &F, value: &i32) {
    TextWidget::new(render(*value)).into_element_desc()
}

#[component]
fn fixed<const N: usize>(values: &[f32; N]) {
    TextWidget::new(format!("{N}:{}", values.iter().sum::<f32>())).into_element_desc()
}

/// No field names `T`, so it can only be given with a turbofish.
#[component]
fn typed<T: Default + Display>(label: &String) {
    TextWidget::new(format!("{label}={}", T::default())).into_element_desc()
}

#[component]
fn marker<T>() {
    let _ = PhantomData::<T>;
    container().into_element_desc(Vec::new())
}

#[test]
fn type_parameters_are_inferred_from_the_props_that_name_them() {
    let _: ElementDesc = xui! { <Labeled value={3u8} /> };
    let _: ElementDesc = xui! { <Labeled value={"text"} /> };
    let _: ElementDesc = xui! { <Rendered render={|v: i32| v.to_string()} value={2} /> };
    let _: ElementDesc = xui! { <Fixed values={[1.0, 2.0, 3.0]} /> };
    let _: ElementDesc = xui! {
        <Listing items={vec![1, 2]}>
            <container />
        </Listing>
    };

    let props = LabeledProps::builder().value(7i64).build();
    assert_eq!(props.value, 7);
}

#[test]
fn parameters_no_prop_names_take_a_turbofish() {
    let _: ElementDesc = xui! { <Typed::<u8> label={"n".to_string()} /> };
    let _: ElementDesc = xui! { <Marker::<String> /> };

    let props = TypedProps::<u8>::builder().label("n").build();
    assert_eq!(props.label, "n");
}

#[test]
fn each_monomorphization_is_its_own_component() {
    assert_eq!(
        labeled_component_render::<u8>(),
        labeled_component_render::<u8>()
    );
    assert_ne!(
        labeled_component_render::<u8>(),
        labeled_component_render::<u16>()
    );
    assert_ne!(
        marker_component_render::<u8>(),
        marker_component_render::<u16>()
    );
    // Same name: the type is what tells them apart.
    assert_eq!(
        labeled_component_render::<u8>().name,
        labeled_component_render::<u16>().name
    );
}

thread_local! {
    static WIDE: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
}
static PROBE_MOUNTS: AtomicUsize = AtomicUsize::new(0);

/// Holds a hook typed by `T`, so a fiber reused across `T`s would read the
/// other instantiation's slot.
#[component]
fn probe<T: Default + Clone>(tag: &String) {
    let _ = tag;
    let value = cx.use_state(|| {
        PROBE_MOUNTS.fetch_add(1, Ordering::SeqCst);
        T::default()
    });
    let _: T = value.get().clone();
    container().into_element_desc(Vec::new())
}

fn swapping_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let wide = cx.use_state(|| false);
    WIDE.with(|slot| *slot.borrow_mut() = Some(wide));
    if *wide.get() {
        xui! { <Probe::<u16> key={"probe"} tag={"p".to_string()} /> }
    } else {
        xui! { <Probe::<u8> key={"probe"} tag={"p".to_string()} /> }
    }
}

#[test]
fn swapping_type_arguments_in_place_remounts_the_component() {
    let mut app = App::new(swapping_root);
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
    assert_eq!(PROBE_MOUNTS.load(Ordering::SeqCst), 1);

    WIDE.with(|slot| slot.borrow().as_ref().expect("root mounted").set(true));
    app.render(&mut backend, &mut text).expect("mock backend");
    assert_eq!(PROBE_MOUNTS.load(Ordering::SeqCst), 2);
}
