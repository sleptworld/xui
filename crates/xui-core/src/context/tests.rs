//! The fixture: `root` renders `provider` and `outside`, side by side;
//! `provider` renders `middle`, which renders `reader` in a shape picked by
//! [`MIDDLE`]. None of them take props, so each one bails out unless it has
//! work of its own -- whatever reaches `reader` past `middle` came through the
//! context. Tests run on their own threads, so the thread-locals are theirs.

use std::any::TypeId;
use std::cell::{Cell, RefCell};

use rustc_hash::FxHashMap;

use super::*;
use crate::component::ComponentRuntime;
use crate::dsl::{Content, ElementBody};
use crate::element::portal;
use crate::state::State;
use crate::ui_runtime::UiRuntime;
use crate::widgets::{component, container};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Level(u32);

#[derive(Clone, Copy, PartialEq)]
enum Middle {
    Reader,
    /// `reader`, plus `inner` under a `<context_provider>` of `Level(100)`.
    Nested,
    /// `reader` through a portal.
    Portal,
}

thread_local! {
    static MIDDLE: Cell<Middle> = const { Cell::new(Middle::Reader) };
    /// Whether `root` renders `provider` at all.
    static SHOW_PROVIDER: Cell<bool> = const { Cell::new(true) };
    /// `provider` also reads `Level` itself.
    static PROVIDER_READS: Cell<bool> = const { Cell::new(false) };
    static PROVIDE_TWICE: Cell<bool> = const { Cell::new(false) };
    /// Seeds `provider`'s state; 0 means it provides nothing.
    static INITIAL: Cell<u32> = const { Cell::new(1) };
    static PROVIDER_STATE: RefCell<Option<State<u32>>> = const { RefCell::new(None) };
    static RENDERS: RefCell<FxHashMap<&'static str, usize>> = RefCell::new(FxHashMap::default());
    static SEEN: RefCell<FxHashMap<&'static str, Option<Level>>> =
        RefCell::new(FxHashMap::default());
}

fn rendered(name: &'static str) {
    RENDERS.with(|renders| *renders.borrow_mut().entry(name).or_default() += 1);
}

fn renders(name: &str) -> usize {
    RENDERS.with(|renders| renders.borrow().get(name).copied().unwrap_or(0))
}

fn seen(name: &str) -> Option<Level> {
    SEEN.with(|seen| *seen.borrow().get(name).expect("never rendered"))
}

fn read(cx: &mut HookContext<'_>, name: &'static str) {
    rendered(name);
    let level = cx.use_context::<Level>();
    SEEN.with(|seen| seen.borrow_mut().insert(name, level));
}

fn render(name: &'static str, call: crate::fiber::ComponentCall) -> ComponentRender {
    ComponentRender::new(ComponentType::new(name, TypeId::of::<()>()), call)
}

fn leaf() -> ElementDesc {
    container().into_element_desc(Vec::new())
}

fn reader(cx: &mut HookContext<'_>, _: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    read(cx, "reader");
    leaf()
}

fn inner(cx: &mut HookContext<'_>, _: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    read(cx, "inner");
    leaf()
}

fn outside(cx: &mut HookContext<'_>, _: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    read(cx, "outside");
    leaf()
}

fn middle(_cx: &mut HookContext<'_>, _: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    rendered("middle");
    let reader = ElementDesc::from(component(render("reader", reader)));
    match MIDDLE.with(Cell::get) {
        Middle::Reader => reader,
        Middle::Nested => container().into_element_desc(vec![
            reader,
            Content(component(render("inner", inner))).build(context_provider().value(Level(100))),
        ]),
        Middle::Portal => container().into_element_desc(vec![portal(vec![reader]).into()]),
    }
}

fn provider(cx: &mut HookContext<'_>, _: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    let state = cx.use_state(|| INITIAL.with(Cell::get));
    PROVIDER_STATE.with(|slot| *slot.borrow_mut() = Some(state));
    if PROVIDER_READS.with(Cell::get) {
        read(cx, "provider");
    } else {
        rendered("provider");
    }
    let value = *state.get();
    if value != 0 {
        cx.provide_context(Level(value));
        if PROVIDE_TWICE.with(Cell::get) {
            cx.provide_context(Level(value));
        }
    }
    component(render("middle", middle)).into()
}

fn root(_cx: &mut HookContext<'_>) -> ElementDesc {
    let mut children = Vec::new();
    if SHOW_PROVIDER.with(Cell::get) {
        children.push(component(render("provider", provider)).into());
    }
    children.push(component(render("outside", outside)).into());
    container().into_element_desc(children)
}

struct Fixture {
    ui: UiRuntime,
    components: ComponentRuntime,
}

impl Fixture {
    fn mount() -> Self {
        let mut ui = UiRuntime::new();
        let mut components = ComponentRuntime::new(ui.root(), Scheduler::default(), root);
        components.flush_sync(&mut ui);
        Self { ui, components }
    }

    fn set_provider(&mut self, value: u32) {
        PROVIDER_STATE.with(|slot| slot.borrow().expect("provider mounted").set(value));
        self.flush();
    }

    fn flush(&mut self) {
        self.components.flush_sync(&mut self.ui);
    }
}

#[test]
fn a_reader_sees_the_nearest_provider_and_none_without_one() {
    let _fixture = Fixture::mount();
    assert_eq!(seen("reader"), Some(Level(1)));
    assert_eq!(seen("outside"), None, "a sibling is not below the provider");
}

#[test]
fn a_changed_value_reaches_a_reader_behind_a_bailed_out_component() {
    let mut fixture = Fixture::mount();
    fixture.set_provider(2);

    assert_eq!(seen("reader"), Some(Level(2)));
    assert_eq!(renders("reader"), 2);
    assert_eq!(renders("middle"), 1, "middle does not read the context");
    assert_eq!(renders("outside"), 1);
}

#[test]
fn an_equal_value_does_not_re_render_readers() {
    let mut fixture = Fixture::mount();
    fixture.set_provider(1);

    assert_eq!(renders("provider"), 2);
    assert_eq!(renders("reader"), 1);
}

#[test]
fn readers_follow_a_provider_that_stops_and_starts_providing() {
    let mut fixture = Fixture::mount();
    fixture.components.provide_context(Level(9));
    fixture.flush();
    assert_eq!(seen("outside"), Some(Level(9)));
    assert_eq!(
        renders("reader"),
        1,
        "the reader's provider still shadows the app"
    );

    fixture.set_provider(0);
    assert_eq!(
        seen("reader"),
        Some(Level(9)),
        "fell back to the application"
    );

    let outside = renders("outside");
    fixture.set_provider(3);
    assert_eq!(seen("reader"), Some(Level(3)), "taken over again");
    assert_eq!(
        renders("outside"),
        outside,
        "a reader that is not below the new provider keeps its value"
    );
}

#[test]
fn a_provider_mounted_without_a_value_can_start_providing() {
    INITIAL.with(|initial| initial.set(0));
    let mut fixture = Fixture::mount();
    assert_eq!(seen("reader"), None);

    fixture.set_provider(4);
    assert_eq!(seen("reader"), Some(Level(4)));
    assert_eq!(renders("outside"), 1);
}

#[test]
fn the_application_value_reaches_readers_that_found_nothing() {
    let mut fixture = Fixture::mount();
    fixture.components.provide_context(Level(5));
    fixture.flush();
    assert_eq!(seen("outside"), Some(Level(5)));
    assert_eq!(fixture.components.context::<Level>(), Some(Level(5)));
    assert_eq!(renders("reader"), 1);

    fixture.components.provide_context(Level(5));
    fixture.flush();
    assert_eq!(renders("outside"), 2, "an equal application value is free");

    fixture.components.remove_context::<Level>();
    fixture.flush();
    assert_eq!(seen("outside"), None);
    assert_eq!(fixture.components.context::<Level>(), None);
}

#[test]
fn an_application_value_set_before_mounting_is_seen_on_the_first_render() {
    SHOW_PROVIDER.with(|show| show.set(false));
    let mut ui = UiRuntime::new();
    let mut components = ComponentRuntime::new(ui.root(), Scheduler::default(), root);
    components.provide_context(Level(7));
    components.flush_sync(&mut ui);
    assert_eq!(seen("outside"), Some(Level(7)));
}

#[test]
fn a_nested_provider_shadows_the_outer_one() {
    MIDDLE.with(|middle| middle.set(Middle::Nested));
    let mut fixture = Fixture::mount();
    assert_eq!(seen("reader"), Some(Level(1)));
    assert_eq!(seen("inner"), Some(Level(100)));

    fixture.set_provider(2);
    assert_eq!(seen("reader"), Some(Level(2)));
    assert_eq!(
        renders("inner"),
        1,
        "the inner provider's value did not change"
    );
}

#[test]
fn a_reader_in_a_portal_sees_the_context_where_the_portal_is_written() {
    MIDDLE.with(|middle| middle.set(Middle::Portal));
    let mut fixture = Fixture::mount();
    assert_eq!(seen("reader"), Some(Level(1)));

    fixture.set_provider(6);
    assert_eq!(seen("reader"), Some(Level(6)));
}

#[test]
fn a_component_does_not_read_its_own_context() {
    PROVIDER_READS.with(|reads| reads.set(true));
    let mut fixture = Fixture::mount();
    assert_eq!(seen("provider"), None);

    fixture.components.provide_context(Level(8));
    fixture.flush();
    assert_eq!(seen("provider"), Some(Level(8)));
    assert_eq!(seen("reader"), Some(Level(1)));
}

#[test]
fn unmounting_drops_every_subscription() {
    let mut fixture = Fixture::mount();
    assert_eq!(
        fixture.components.context_registry().subscription_count(),
        2
    );

    SHOW_PROVIDER.with(|show| show.set(false));
    fixture.components.mark_root_dirty();
    fixture.flush();
    assert_eq!(
        fixture.components.context_registry().subscription_count(),
        1,
        "only `outside` is left"
    );
}

#[test]
#[should_panic(expected = "was called twice in one render")]
fn providing_a_type_twice_in_one_render_panics() {
    PROVIDE_TWICE.with(|twice| twice.set(true));
    let mut fixture = Fixture::mount();
    fixture.set_provider(2);
}
