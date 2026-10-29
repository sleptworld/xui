//! Reactive components mounted into a real `UiRuntime`.
use xui_interface::NodeId;
use xui_macros::component;

use super::{Callback, Children, Component, ComponentDef, FlushReport, Lane, Runtime};
use crate::ui_runtime::UiRuntime;
use crate::widgets::WidgetType;
use crate::widgets::container::{column, container, row};
use crate::widgets::text::text;

/// The hosts under `id`, as markup: `<text>` with its content, anything
/// else by its widget type.
fn markup(ui: &UiRuntime, id: NodeId) -> String {
    let node = ui.node(id).unwrap();
    if node.node_type == WidgetType::Text {
        let text = node.widget.text().unwrap();
        return format!("<text>{}</text>", text.as_str());
    }
    let tag = format!("{:?}", node.node_type).to_lowercase();
    let children: String = ui.children(id).map(|child| markup(ui, child)).collect();
    format!("<{tag}>{children}</{tag}>")
}

/// What `runtime` mounted under the root, as markup.
fn shown(ui: &UiRuntime, runtime: &Runtime, id: super::InstanceId) -> String {
    runtime.hosts(id).into_iter().map(|host| markup(ui, host)).collect()
}

fn analysis<C: Component>() -> Vec<(&'static str, Lane, Vec<&'static str>)> {
    let t = &<C::Instance as ComponentDef>::TEMPLATE;
    t.effects.iter().map(|e| (e.label, e.lane, e.deps.iter().map(|&d| t.sources[d as usize]).collect())).collect()
}

#[component]
fn counter(step: &i32) {
    let count: i32 = state!(0);
    let doubled: i32 = memo!(count * 2);
    view! {
        column {
            text(format!("count {count}"))
            text(format!("doubled {doubled}"))
            text("static")
        }
    }
}

#[test]
fn mounts_into_the_host_tree_and_updates_what_changed() {
    assert_eq!(
        analysis::<Counter>(),
        [
            ("memo doubled", Lane::Memo, vec!["count"]),
            ("text 1.content", Lane::Render, vec!["count"]),
            ("text 2.content", Lane::Render, vec!["doubled"]),
        ]
    );
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Counter>(&mut ui, root, CounterProps { step: 2 });
    assert_eq!(
        shown(&ui, &runtime, id),
        "<container><text>count 0</text><text>doubled 0</text><text>static</text></container>"
    );

    let count = runtime.instance::<Counter>(id).unwrap().count.clone();
    let step = runtime.instance::<Counter>(id).unwrap().step;
    let report = runtime.batch(&mut ui, |cx| count.update(cx, |c| *c += step));
    assert_eq!(report, FlushReport { memos: 1, renders: 2, ..Default::default() });
    assert_eq!(
        shown(&ui, &runtime, id),
        "<container><text>count 2</text><text>doubled 4</text><text>static</text></container>"
    );

    runtime.unmount(&mut ui, id);
    assert!(runtime.hosts(id).is_empty());
    assert_eq!(ui.children(root).count(), 1, "only the overlayer is left");
    assert!(runtime.graph().is_empty());
}

#[component]
fn toggle(#[source] on: &bool, #[source] items: &Vec<u32>) {
    view! {
        row {
            text("first")
            if *on {
                text("on")
                text("and more")
            } else {
                text("off")
            }
            for item in items.clone() {
                text(format!("item {item}")).key(*item)
            }
            text("last")
        }
    }
}

#[test]
fn blocks_place_their_rows_between_their_siblings() {
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let (on, items) = (runtime.signal(false), runtime.signal(vec![1, 2]));
    let root = ui.root();
    let id = runtime.mount::<Toggle>(&mut ui, root, ToggleProps { on: on.read_only(), items: items.read_only() });
    let row = |inner: &str| format!("<container><text>first</text>{inner}<text>last</text></container>");
    assert_eq!(shown(&ui, &runtime, id), row("<text>off</text><text>item 1</text><text>item 2</text>"));

    runtime.batch(&mut ui, |cx| on.set(cx, true));
    assert_eq!(
        shown(&ui, &runtime, id),
        row("<text>on</text><text>and more</text><text>item 1</text><text>item 2</text>")
    );

    // Reordered, inserted and removed rows land between the `if` and `last`.
    runtime.batch(&mut ui, |cx| items.set(cx, vec![3, 2]));
    assert_eq!(
        shown(&ui, &runtime, id),
        row("<text>on</text><text>and more</text><text>item 3</text><text>item 2</text>")
    );
    runtime.batch(&mut ui, |cx| items.set(cx, vec![2, 3, 1]));
    assert_eq!(
        shown(&ui, &runtime, id),
        row("<text>on</text><text>and more</text><text>item 2</text><text>item 3</text><text>item 1</text>")
    );

    runtime.batch(&mut ui, |cx| on.set(cx, false));
    assert_eq!(
        shown(&ui, &runtime, id),
        row("<text>off</text><text>item 2</text><text>item 3</text><text>item 1</text>")
    );
}

#[test]
fn rows_already_in_order_are_not_moved() {
    use crate::ui_runtime::state::HostWorkFlags;
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let (on, items) = (runtime.signal(false), runtime.signal(vec![1, 2, 3]));
    let root = ui.root();
    let id = runtime.mount::<Toggle>(&mut ui, root, ToggleProps { on: on.read_only(), items: items.read_only() });
    let container = runtime.hosts(id)[0];
    ui.take_work(container);

    // The block reruns and keeps every row where it stands.
    let report = runtime.batch(&mut ui, |cx| items.set(cx, vec![1, 2, 3]));
    assert_eq!(report.blocks, 1);
    assert!(!ui.take_work(container).intersects(HostWorkFlags::CHILDREN_CHANGED));

    runtime.batch(&mut ui, |cx| items.set(cx, vec![3, 1, 2]));
    assert!(ui.take_work(container).intersects(HostWorkFlags::CHILDREN_CHANGED));
}

#[component]
fn button_row() {
    let clicks: i32 = state!(0);
    view! {
        row {
            container(on_click: |cx| clicks.update(cx, |c| *c += 1)) {
                text("+")
            }
            text(format!("clicks {clicks}"))
        }
    }
}

#[test]
fn a_child_component_places_its_hosts_where_it_stands() {
    #[component]
    fn outer() {
        view! {
            column {
                text("before")
                button_row()
                text("after")
            }
        }
    }
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Outer>(&mut ui, root, OuterProps {});
    assert_eq!(
        shown(&ui, &runtime, id),
        "<container><text>before</text><container><container><text>+</text></container>\
         <text>clicks 0</text></container><text>after</text></container>"
    );
}

#[component]
fn clicker() {
    let clicks: i32 = state!(0);
    view! {
        column {
            container(width: 40.0, height: 40.0, on_click: |cx| clicks.update(cx, |c| *c += 1))
            text(format!("clicks {clicks}"))
        }
    }
}

#[test]
fn a_handler_writes_and_the_flush_shows_it() {
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Clicker>(&mut ui, root, ClickerProps {});
    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 200.0), &mut measurer);

    let column = runtime.hosts(id)[0];
    let button = ui.children(column).next().unwrap();
    crate::ui_runtime::tests::click(&mut ui, &measurer, button);
    // The handler only recorded the write.
    assert!(runtime.is_dirty());
    assert_eq!(runtime.flush(&mut ui), FlushReport { renders: 1, ..Default::default() });
    assert_eq!(shown(&ui, &runtime, id), "<container><container></container><text>clicks 1</text></container>");
}

#[test]
fn an_app_flushes_what_its_handlers_wrote_before_the_next_frame() {
    use crate::app::App;
    use crate::core::{Point, Size};
    use crate::render::MockRenderBackend;
    use crate::runtime::{GuiRuntime, RuntimeEvent};
    use crate::text::testing::ZeroTextBackend;
    use crate::ui_runtime::tests::{pointer_at, pointer_move};
    use xui_interface::events::RawEvent;

    let mut app = App::new(|_| {});
    let id = app.mount::<Clicker>(ClickerProps {});
    let mut gui = GuiRuntime::new(app, MockRenderBackend::default(), ZeroTextBackend);
    gui.handle_event(RuntimeEvent::Resize(Size::new(200.0, 200.0)));
    gui.frame().unwrap();

    let ui = gui.app().ui_runtime();
    let column = gui.app().reactive().hosts(id)[0];
    let button = ui.children(column).next().unwrap();
    let bounds = ui.node(button).unwrap().layout;
    let point = Point::new((bounds.min.x + bounds.max.x) * 0.5, (bounds.min.y + bounds.max.y) * 0.5);
    gui.handle_event(RuntimeEvent::Input(pointer_move(point)));
    gui.handle_event(RuntimeEvent::Input(RawEvent::PointerDown(pointer_at(point))));
    gui.handle_event(RuntimeEvent::Input(RawEvent::PointerUp(pointer_at(point))));
    assert!(gui.app().is_dirty());
    gui.frame().unwrap();

    let app = gui.app();
    assert_eq!(
        shown(app.ui_runtime(), app.reactive(), id),
        "<container><container></container><text>clicks 1</text></container>"
    );
}

#[component]
#[defaults(prefix = "n")]
fn badge(#[source] value: &i32, label: &String, prefix: &String) {
    view! { text(format!("{prefix} {label} {value}")) }
}

#[component]
fn stepper(#[state] count: &i32) {
    view! { container(width: 10.0, height: 10.0, on_click: |cx| count.update(cx, |c| *c += 1)) }
}

#[component]
fn dashboard() {
    let count: i32 = state!(1);
    let label: String = "fixed".to_string();
    view! {
        column {
            badge(value: count * 10, label: format!("at {count}"))
            badge(value: count, label: label, prefix: "p")
            stepper(count: count)
        }
    }
}

#[test]
fn props_are_delivered_as_the_child_declares() {
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Dashboard>(&mut ui, root, DashboardProps {});
    let column = |inner: &str| format!("<container>{inner}<container></container></container>");
    assert_eq!(shown(&ui, &runtime, id), column("<text>n at 1 10</text><text>p fixed 1</text>"));
    // `count` itself and the one source derived for `count * 10`; passed by
    // name, `count` is shared, not copied.
    assert_eq!(runtime.graph().len(), 2);

    let count = runtime.instance::<Dashboard>(id).unwrap().count.clone();
    let report = runtime.batch(&mut ui, |cx| count.set(cx, 2));
    // The derived source, then both badges' texts. `label` was a snapshot.
    assert_eq!(report, FlushReport { memos: 1, renders: 2, ..Default::default() });
    assert_eq!(shown(&ui, &runtime, id), column("<text>n at 1 20</text><text>p fixed 2</text>"));

    runtime.unmount(&mut ui, id);
    assert!(runtime.graph().is_empty());
}

#[test]
fn a_state_prop_writes_the_parents_signal() {
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Dashboard>(&mut ui, root, DashboardProps {});
    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let column = runtime.hosts(id)[0];
    let button = ui.children(column).nth(2).unwrap();
    crate::ui_runtime::tests::click(&mut ui, &measurer, button);
    runtime.flush(&mut ui);
    assert_eq!(runtime.instance::<Dashboard>(id).unwrap().count.get(), 2);
    assert!(shown(&ui, &runtime, id).contains("<text>p fixed 2</text>"));
}

#[component]
fn picker(#[source] value: &i32, on_pick: &Callback<i32>) {
    view! {
        container(width: 10.0, height: 10.0, on_click: |cx, event| {
            let _: &xui_interface::events::semantic::ClickEvent = event;
            on_pick.call(cx, value.get() + 1)
        })
    }
}

#[component]
fn card(title: &String, children: &Children) {
    let open: bool = state!(true);
    view! {
        column {
            text(title.clone())
            if *open {
                slot(children)
            }
            text("end")
        }
    }
}

#[component]
fn page() {
    let picked: i32 = state!(0);
    view! {
        column {
            picker(value: picked, on_pick: |cx, v| picked.set(cx, v))
            card(title: "Card".to_string()) {
                text(format!("picked {picked}"))
                text("second")
            }
        }
    }
}

#[test]
fn callbacks_and_children_run_in_the_parents_scope() {
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Page>(&mut ui, root, PageProps {});
    let page = |picked: i32| {
        format!(
            "<container><container></container><container><text>Card</text>\
             <text>picked {picked}</text><text>second</text><text>end</text></container></container>"
        )
    };
    assert_eq!(shown(&ui, &runtime, id), page(0));

    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let column = runtime.hosts(id)[0];
    let button = ui.children(column).next().unwrap();
    crate::ui_runtime::tests::click(&mut ui, &measurer, button);
    // The callback wrote `picked`; the slot's text, a binding of the page,
    // reruns, and nothing of the card does.
    assert_eq!(runtime.flush(&mut ui), FlushReport { renders: 1, ..Default::default() });
    assert_eq!(shown(&ui, &runtime, id), page(1));

    runtime.unmount(&mut ui, id);
    assert!(runtime.graph().is_empty());
}

#[component]
fn popup() {
    let open: bool = state!(false);
    let dismissed: i32 = state!(0);
    view! {
        column {
            container(width: 10.0, height: 10.0, on_click: |cx| open.set(cx, true))
            if *open {
                portal(z_index: 5, on_dismiss: |cx, _reason| {
                    open.set(cx, false);
                    dismissed.update(cx, |d| *d += 1)
                }) {
                    column {
                        text(format!("dismissed {dismissed}"))
                    }
                }
            }
            text("below")
        }
    }
}

#[test]
fn a_portal_mounts_its_content_under_the_overlayer() {
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let overlayer = ui.root_overlayer();
    let id = runtime.mount::<Popup>(&mut ui, root, PopupProps {});
    assert_eq!(ui.children(overlayer).count(), 0);

    let open = runtime.instance::<Popup>(id).unwrap().open.clone();
    runtime.batch(&mut ui, |cx| open.set(cx, true));
    // In the overlayer, not between the button and `below`.
    assert_eq!(shown(&ui, &runtime, id), "<container><container></container><text>below</text></container>");
    let layer: Vec<_> = ui.children(overlayer).collect();
    assert_eq!(layer.len(), 1);
    assert_eq!(markup(&ui, layer[0]), "<container><text>dismissed 0</text></container>");

    runtime.batch(&mut ui, |cx| open.set(cx, false));
    assert_eq!(ui.children(overlayer).count(), 0);
    assert!(!ui.contains(layer[0]), "the content went with the portal");

    runtime.batch(&mut ui, |cx| open.set(cx, true));
    runtime.unmount(&mut ui, id);
    assert_eq!(ui.children(overlayer).count(), 0);
    assert_eq!(ui.children(root).count(), 1, "only the overlayer is left");
}

#[derive(Clone, PartialEq, Debug)]
struct Density(&'static str);

#[derive(Clone, PartialEq, Debug)]
struct Accent(&'static str);

#[component]
fn themed() {
    let density: Density = context!();
    let accent: Accent = context!();
    view! { text(format!("{} {}", density.0, accent.0)) }
}

#[component]
fn provider() {
    let density: Density = state!(Density("compact"));
    provide!(density);
    view! {
        column {
            themed()
            container(width: 10.0, height: 10.0, on_click: |cx| density.set(cx, Density("cozy")))
        }
    }
}

#[test]
fn context_comes_from_the_nearest_provider_or_the_app() {
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    runtime.provide_context(Density("app"));
    runtime.provide_context(Accent("blue"));
    let root = ui.root();
    let id = runtime.mount::<Provider>(&mut ui, root, ProviderProps {});
    assert_eq!(shown(&ui, &runtime, id), "<container><text>compact blue</text><container></container></container>");

    let density = runtime.instance::<Provider>(id).unwrap().density.clone();
    runtime.batch(&mut ui, |cx| density.set(cx, Density("cozy")));
    runtime.provide_context(Accent("red"));
    runtime.flush(&mut ui);
    assert_eq!(shown(&ui, &runtime, id), "<container><text>cozy red</text><container></container></container>");

    // Without a provider in between, the app's.
    let alone = runtime.mount::<Themed>(&mut ui, root, ThemedProps {});
    assert_eq!(shown(&ui, &runtime, alone), "<text>app red</text>");
}

#[component]
fn measured() {
    let size: Option<crate::core::Size<f32>> = size!();
    let frames: u32 = state!(0);
    let spin: crate::ticker::Ticker = ticker!(|cx, _frame| {
        frames.update(cx, |f| *f += 1);
        if frames.get() >= 3 { crate::ticker::Tick::Stop } else { crate::ticker::Tick::Continue }
    });
    view! {
        column {
            container(width: 30.0, height: 20.0, size_observer: size)
            text(format!("{:?} {frames}", size.map(|s| s.width)))
        }
    }
}

#[test]
fn layout_writes_size_and_tickers_run_per_frame() {
    use crate::clock::FrameTime;
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Measured>(&mut ui, root, MeasuredProps {});
    let text = |ui: &UiRuntime, runtime: &Runtime| {
        let column = runtime.hosts(id)[0];
        markup(ui, ui.children(column).nth(1).unwrap())
    };
    assert_eq!(text(&ui, &runtime), "<text>None 0</text>");

    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 200.0), &mut measurer);
    assert!(runtime.is_dirty(), "layout wrote the size");
    runtime.flush(&mut ui);
    assert_eq!(text(&ui, &runtime), "<text>Some(30.0) 0</text>");

    let tickers = ui.tickers();
    for _ in 0..5 {
        tickers.tick(FrameTime::ZERO);
        runtime.flush(&mut ui);
    }
    // It stopped itself after three frames.
    assert_eq!(text(&ui, &runtime), "<text>Some(30.0) 3</text>");
    assert!(!runtime.instance::<Measured>(id).unwrap().spin.is_running());
    runtime.instance::<Measured>(id).unwrap().spin.start();
    tickers.tick(FrameTime::ZERO);
    runtime.flush(&mut ui);
    assert_eq!(text(&ui, &runtime), "<text>Some(30.0) 4</text>");

    runtime.unmount(&mut ui, id);
    assert!(!tickers.is_running(), "unmounting unregisters it");
}

#[component]
fn fetched() {
    let n: i32 = state!(2);
    let doubled: super::AsyncValue<i32, String> = resource!({
        let n = *n;
        async move { if n < 0 { Err("negative".to_string()) } else { Ok(n * 2) } }
    });
    view! {
        text(match &*doubled {
            super::AsyncValue::Pending => "pending".to_string(),
            super::AsyncValue::Ready(v) => format!("ready {v}"),
            super::AsyncValue::Error(e) => format!("error {e}"),
        })
    }
}

#[test]
fn a_resource_starts_with_its_sources_and_writes_its_result() {
    use crate::app::App;

    let mut app = App::new(|_| {});
    let id = app.mount::<Fetched>(FetchedProps {});
    let shown = |app: &App| markup(app.ui_runtime(), app.reactive().hosts(id)[0]);
    // Renders (which drains finished futures and flushes) until `expected` shows.
    let settle = |app: &mut App, expected: &str| {
        let mut text = crate::text::TextHost::new(crate::text::testing::ZeroTextBackend);
        let mut backend = crate::render::MockRenderBackend::default();
        for _ in 0..400 {
            app.render(&mut backend, &mut text).unwrap();
            if shown(app) == expected {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("never showed {expected}, still {}", shown(app));
    };
    assert_eq!(shown(&app), "<text>pending</text>");
    settle(&mut app, "<text>ready 4</text>");

    let n = app.reactive().instance::<Fetched>(id).unwrap().n.clone();
    app.update(|cx| n.set(cx, -1));
    settle(&mut app, "<text>error negative</text>");
}

#[test]
fn a_resource_keeps_only_its_latest_result() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let started: Rc<RefCell<Vec<super::tasks::BoxFuture>>> = Rc::default();
    let (sender, finished) = std::sync::mpsc::channel();
    let queue = started.clone();
    runtime.tasks_mut().connect(Box::new(move |future| queue.borrow_mut().push(future)), sender);
    let root = ui.root();
    let id = runtime.mount::<Fetched>(&mut ui, root, FetchedProps {});
    let n = runtime.instance::<Fetched>(id).unwrap().n.clone();
    runtime.batch(&mut ui, |cx| n.set(cx, 5));

    // The newer future finishes first; the older one's result comes last
    // and must not overwrite it.
    let mut futures = std::mem::take(&mut *started.borrow_mut());
    assert_eq!(futures.len(), 2);
    for future in futures.iter_mut().rev() {
        assert!(future.as_mut().poll(&mut Context::from_waker(Waker::noop())) == Poll::Ready(()));
    }
    while let Ok(result) = finished.try_recv() {
        runtime.finish_task(result);
    }
    runtime.flush(&mut ui);
    assert_eq!(shown(&ui, &runtime, id), "<text>ready 10</text>");
}

#[component]
fn labelled(leading: &Children, label: &String) {
    view! { row { slot(leading) text(label.clone()) } }
}

#[component]
fn uses_labelled() {
    let count: i32 = state!(3);
    view! {
        column {
            labelled(label: "items".to_string(), leading: view! { text(format!("{count}")) text("·") })
            container(width: 10.0, height: 10.0, on_click: |cx, _event, ecx| {
                ecx.request_focus();
                count.set(cx, 4)
            })
        }
    }
}

#[test]
fn a_children_prop_takes_elements_and_a_handler_its_event_context() {
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<UsesLabelled>(&mut ui, root, UsesLabelledProps {});
    let row = "<container><text>3</text><text>·</text><text>items</text></container>";
    assert_eq!(shown(&ui, &runtime, id), format!("<container>{row}<container></container></container>"));

    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 200.0), &mut measurer);
    let column = runtime.hosts(id)[0];
    let button = ui.children(column).nth(1).unwrap();
    crate::ui_runtime::tests::click(&mut ui, &measurer, button);
    runtime.flush(&mut ui);
    assert!(shown(&ui, &runtime, id).contains("<text>4</text>"));
}

// ---- generic components ----------------------------------------------------

#[component]
fn labeled<T: std::fmt::Display>(value: &T) {
    view! { text(value.to_string()) }
}

#[component]
#[defaults(items = Vec::new())]
fn listing<T>(items: &Vec<T>, children: &Children)
where
    T: std::fmt::Display,
{
    let joined: String = items.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
    view! {
        column {
            text(joined.clone())
            slot(children)
        }
    }
}

/// `F` appears only in a bound's own parentheses and arrow.
#[component]
fn rendered<F: Fn(i32) -> String>(render: &F, value: &i32) {
    view! { text(render(*value)) }
}

#[component]
fn fixed<const N: usize>(values: &[f32; N]) {
    view! { text(format!("{N}:{}", values.iter().sum::<f32>())) }
}

/// No prop names `T`: it takes a turbofish.
#[component]
fn typed<T: Default + std::fmt::Display>(label: &String) {
    view! { text(format!("{label}={}", T::default())) }
}

/// A source prop typed by the parameter (`T: PartialEq` is implied), and
/// state of that type.
#[component]
fn echo<T: Clone + std::fmt::Debug>(#[source] value: &T) {
    let first: T = state!(value.get());
    view! { text(format!("{value:?} from {first:?}")) }
}

/// A callback typed by the parameter.
#[component]
fn chooser<T: Clone>(items: &Vec<T>, on_pick: &Callback<T>) {
    view! {
        container(width: 10.0, height: 10.0, on_click: |cx| {
            if let Some(first) = items.first() {
                on_pick.call(cx, first.clone());
            }
        })
    }
}

fn twice(value: i32) -> String {
    format!("{value}{value}")
}

#[component]
fn generic_uses() {
    let count: i32 = state!(1);
    let picked: String = state!(String::new());
    view! {
        column {
            labeled(value: 3u8)
            labeled(value: "text")
            labeled(value: count * 2)
            listing(items: vec![1, 2]) { text("inside") }
            listing::<u8>()
            rendered(render: twice, value: 4)
            rendered(render: (|v: i32| format!("<{v}>")), value: count)
            fixed(values: [1.0, 2.0, 3.0])
            typed::<u8>(label: "n")
            echo(value: count)
            chooser(items: vec!["a".to_string()], on_pick: |cx, item| picked.set(cx, item))
            text(format!("picked {picked}"))
        }
    }
}

#[test]
fn a_generic_component_infers_its_parameters_from_its_props() {
    use crate::core::Size;
    use crate::text::{TextHost, testing::ZeroTextBackend};

    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<GenericUses>(&mut ui, root, GenericUsesProps {});
    // `labeled` and `rendered` take snapshots, made at mount; `echo` reads
    // its prop as a source.
    let expected = |count: i32, picked: &str| {
        format!(
            "<container><text>3</text><text>text</text><text>2</text>\
             <container><text>1,2</text><text>inside</text></container>\
             <container><text></text></container>\
             <text>44</text><text><1></text><text>3:6</text><text>n=0</text>\
             <text>{count} from 1</text><container></container><text>picked {picked}</text></container>"
        )
    };
    assert_eq!(shown(&ui, &runtime, id), expected(1, ""));

    let count = runtime.instance::<GenericUses>(id).unwrap().count.clone();
    runtime.batch(&mut ui, |cx| count.set(cx, 5));
    assert_eq!(shown(&ui, &runtime, id), expected(5, ""));

    let mut measurer = TextHost::new(ZeroTextBackend);
    ui.update_tree(Size::new(200.0, 400.0), &mut measurer);
    let column = runtime.hosts(id)[0];
    let picker = ui.children(column).nth(10).unwrap();
    crate::ui_runtime::tests::click(&mut ui, &measurer, picker);
    runtime.flush(&mut ui);
    assert_eq!(shown(&ui, &runtime, id), expected(5, "a"));
}

#[test]
fn a_generic_component_mounts_at_the_root_and_as_children() {
    let mut ui = UiRuntime::new();
    let mut runtime = Runtime::new();
    let root = ui.root();
    let id = runtime.mount::<Labeled<f32>>(&mut ui, root, LabeledProps { value: 1.5 });
    assert_eq!(shown(&ui, &runtime, id), "<text>1.5</text>");
    // A parameter no prop names is carried by the props as phantom data.
    let id = runtime.mount::<Typed<i64>>(&mut ui, root, TypedProps { label: "k".into(), __phantom: Default::default() });
    assert_eq!(shown(&ui, &runtime, id), "<text>k=0</text>");
    let children = Children::component::<Labeled<char>>(LabeledProps { value: 'x' });
    let id = runtime.mount::<Listing<u8>>(&mut ui, root, ListingProps { items: vec![7], children });
    assert_eq!(shown(&ui, &runtime, id), "<container><text>7</text><text>x</text></container>");
}
