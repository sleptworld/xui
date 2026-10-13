//! Contexts as an application writes them: `#[component]`s, `xui!`, the
//! `<context_provider>` element, and a value set on the `App`.

use std::cell::RefCell;

use xui_core::prelude::*;
use xui_core::render::backend::MockRenderBackend;
use xui_core::text::TextHost;
use xui_cosmic::{CosmicEngine, FontSet};
use xui_interface::Size;
use xui_macros::{component, xui};

#[derive(Clone, Debug, PartialEq)]
struct Locale(&'static str);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Compact(bool);

thread_local! {
    static SEEN: RefCell<Vec<(String, Option<Locale>, Compact)>> = const { RefCell::new(Vec::new()) };
}

#[component]
fn label(name: &String) {
    let locale = cx.use_context::<Locale>();
    let compact = cx.use_context::<Compact>().unwrap_or(Compact(false));
    SEEN.with(|seen| seen.borrow_mut().push((name.clone(), locale, compact)));
    container().into_element_desc(Vec::new())
}

#[component]
fn compact_scope(children: &Vec<ElementDesc>) {
    cx.provide_context(Compact(true));
    container().into_element_desc(children.clone())
}

fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
    xui! {
        <container>
            <label name={"plain".to_string()} />
            <compact_scope>
                <label name={"compact".to_string()} />
                <context_provider value={Locale("fr")}>
                    <label name={"french".to_string()} />
                </context_provider>
            </compact_scope>
        </container>
    }
}

fn seen() -> Vec<(String, Option<Locale>, Compact)> {
    SEEN.with(|seen| seen.borrow_mut().drain(..).collect())
}

#[test]
fn components_read_what_the_app_and_their_ancestors_provide() {
    let mut app = App::new(root);
    app.provide_context(Locale("en"));
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
    assert_eq!(
        seen(),
        vec![
            ("plain".to_string(), Some(Locale("en")), Compact(false)),
            ("compact".to_string(), Some(Locale("en")), Compact(true)),
            ("french".to_string(), Some(Locale("fr")), Compact(true)),
        ]
    );

    // Only the readers of the application's value render again; the one
    // under its own `Locale` provider does not.
    app.provide_context(Locale("de"));
    assert!(app.is_dirty());
    app.render(&mut backend, &mut text).expect("mock backend");
    assert_eq!(
        seen(),
        vec![
            ("plain".to_string(), Some(Locale("de")), Compact(false)),
            ("compact".to_string(), Some(Locale("de")), Compact(true)),
        ]
    );
    assert_eq!(app.context::<Locale>(), Some(Locale("de")));
}
