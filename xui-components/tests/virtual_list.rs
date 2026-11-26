//! End-to-end checks that `virtual_list` mounts only the rows near the
//! viewport and still reports the full scrollable height.

use std::cell::{Cell, RefCell};
use std::time::Instant;
use xui_components::VirtualItemRenderer;
#[allow(unused_imports)]
use xui_components::*;
use xui_core::prelude::*;
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_interface::Translation;
use xui_interface::events::{Modifiers, RawEvent, RawWheel, ScrollDelta};
use xui_macros::component;

const ITEM_COUNT: usize = 100_000;
const ITEM_HEIGHT: f32 = 20.0;
const VIEWPORT: Size<f32> = Size {
    width: 400.0,
    height: 200.0,
};

thread_local! {
    /// Indices the list actually asked for, most recent render last.
    static RENDERED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// Total item-renderer invocations. Thread-local (like `RENDERED`) so
    /// tests running in parallel threads cannot disturb each other's counts.
    static RENDER_CALLS: Cell<usize> = const { Cell::new(0) };
}

/// A row: its index, at the row's height.
#[component]
fn list_row(index: &usize) {
    view! {
        text(
            format!("row {index}"),
            style: StyleOf::new().with::<container::width>(Sizing::Fill).with::<container::height>(ITEM_HEIGHT),
        )
    }
}

#[component]
fn list() {
    let render_item: VirtualItemRenderer = VirtualItemRenderer::new(|index: usize| {
        RENDER_CALLS.with(|calls| calls.set(calls.get() + 1));
        RENDERED.with(|slot| slot.borrow_mut().push(index));
        Children::component::<ListRow>(ListRowProps { index })
    });
    let style: StyleOf<Container> = StyleOf::new()
        .with::<container::width>(Sizing::Fill)
        .with::<container::height>(VIEWPORT.height);
    xui! {
        <virtual_list
            item_count={ITEM_COUNT}
            item_size={ItemSize::Fixed(ITEM_HEIGHT)}
            viewport_height={VIEWPORT.height}
            render_item={render_item.clone()}
            style={style.clone()}
        />
    }
}

fn list_root(app: &mut App) {
    app.mount::<List>(ListProps {});
}

struct Harness {
    app: App,
    text: TextHost<CosmicEngine>,
    backend: MockRenderBackend,
}

impl Harness {
    fn new() -> Self {
        Self::with_root(list_root)
    }

    fn with_root(root: AppRoot) -> Self {
        let mut app = App::new(root);
        app.resize(app.main_window(), VIEWPORT);
        let mut harness = Self {
            app,
            text: TextHost::new(CosmicEngine::new(1.0)),
            backend: MockRenderBackend::default(),
        };
        harness.render();
        harness
    }

    fn render(&mut self) {
        self.app
            .render(self.app.main_window(), &mut self.backend, &mut self.text)
            .expect("mock backend cannot fail");
    }

    /// `(scroll container, spacer)`; the root also owns the overlay layer, so
    /// the list is the first child rather than the only one.
    fn list_nodes(&self) -> (NodeId, NodeId) {
        let runtime = self.app.ui_runtime();
        let scroller = runtime
            .children(runtime.root())
            .next()
            .expect("the list mounted");
        let spacer = runtime
            .children(scroller)
            .next()
            .expect("the spacer mounted");
        (scroller, spacer)
    }

    fn row_count(&self) -> usize {
        let (_, spacer) = self.list_nodes();
        self.app.ui_runtime().children(spacer).count()
    }

    fn total_hosts(&self) -> usize {
        fn walk(runtime: &UiRuntime, id: NodeId) -> usize {
            1 + runtime
                .children(id)
                .map(|c| walk(runtime, c))
                .sum::<usize>()
        }
        let runtime = self.app.ui_runtime();
        walk(runtime, runtime.root())
    }

    fn scroll_by(&mut self, dy: f32) {
        let event = RawEvent::Wheel(RawWheel {
            position: Point::new(10.0, 10.0),
            delta: ScrollDelta::Pixels(Translation::new(0.0, -dy)),
            device_id: None,
            pointer_id: None,
            modifiers: Modifiers::default(),
            timestamp: Instant::now(),
            is_inertial: false,
        });
        self.app
            .dispatch_event(self.app.main_window(), event, &mut self.text);
        self.render();
    }
}

fn rendered_indices() -> Vec<usize> {
    RENDERED.with(|slot| slot.borrow().clone())
}

fn render_calls() -> usize {
    RENDER_CALLS.with(Cell::get)
}

fn reset_tracking() {
    RENDERED.with(|slot| slot.borrow_mut().clear());
    RENDER_CALLS.with(|calls| calls.set(0));
}

#[test]
fn a_hundred_thousand_rows_mount_only_the_visible_ones() {
    reset_tracking();
    let harness = Harness::new();

    let rows = harness.row_count();
    assert!(
        (10..=20).contains(&rows),
        "expected roughly a viewport's worth of rows, mounted {rows}"
    );

    // The decisive property: cost is independent of list length. A
    // non-virtualized list of this size would mount hundreds of thousands.
    assert!(
        harness.total_hosts() < 100,
        "the whole tree should stay small, has {} hosts",
        harness.total_hosts()
    );

    let indices = rendered_indices();
    assert_eq!(indices.first().copied(), Some(0));
    assert!(
        indices.iter().all(|index| *index < 40),
        "rows far outside the viewport were built: {indices:?}"
    );
}

#[test]
fn the_spacer_reports_the_full_scrollable_height() {
    reset_tracking();
    let harness = Harness::new();
    let (_, spacer) = harness.list_nodes();

    let height = harness
        .app
        .ui_runtime()
        .node(spacer)
        .expect("spacer exists")
        .layout
        .height();

    // Without this the scrollbar and the scroll clamp would both be wrong.
    assert_eq!(height, ITEM_COUNT as f32 * ITEM_HEIGHT);
}

#[test]
fn rows_sit_at_their_index_offset() {
    reset_tracking();
    let mut harness = Harness::new();
    assert_row_positions(&harness, 0.0);

    // The same has to hold once the window has slid away from the start.
    harness.scroll_by(4_000.0);
    assert_row_positions(&harness, 4_000.0);
}

/// Rows must sit at `index * ITEM_HEIGHT` inside the spacer, which is what
/// makes the absolute placement line up with the scroll offset.
fn assert_row_positions(harness: &Harness, scroll_top: f32) {
    let (_, spacer) = harness.list_nodes();
    let runtime = harness.app.ui_runtime();
    let spacer_origin = runtime.node(spacer).expect("spacer exists").world_origin.y;

    let positions: Vec<f32> = runtime
        .children(spacer)
        .map(|row| runtime.node(row).expect("row exists").world_origin.y - spacer_origin)
        .collect();
    assert!(!positions.is_empty());

    for pair in positions.windows(2) {
        assert_eq!(
            pair[1] - pair[0],
            ITEM_HEIGHT,
            "rows are not contiguous at scroll {scroll_top}: {positions:?}"
        );
    }
    assert_eq!(
        positions[0] % ITEM_HEIGHT,
        0.0,
        "the window does not start on a row boundary at scroll {scroll_top}"
    );
}

#[test]
fn scrolling_slides_the_window_without_growing_it() {
    reset_tracking();
    let mut harness = Harness::new();
    let before = harness.row_count();

    harness.scroll_by(4_000.0);

    // The window is bounded, not constant: at the very top and bottom it is
    // clipped by the ends of the list, so mid-list it can be a little larger.
    let after = harness.row_count();
    assert!(
        after <= before + 2 * 3,
        "the mounted window grew beyond overscan: {before} -> {after}"
    );

    let indices = rendered_indices();
    let latest = *indices.last().expect("rows were rendered");
    assert!(
        latest > 150,
        "scrolling did not move the window; last index built was {latest}"
    );
    assert!(
        harness.total_hosts() < 100,
        "the tree grew while scrolling: {} hosts",
        harness.total_hosts()
    );
}

#[test]
fn a_scroll_too_small_to_change_the_window_does_not_rebuild_rows() {
    reset_tracking();
    let mut harness = Harness::new();
    // Into the middle of a row, so both edges straddle one.
    harness.scroll_by(5.0);
    let calls_after_mount = render_calls();

    // One pixel more cannot bring a new row into range, so the runtime
    // should move the content without asking the component again.
    harness.scroll_by(1.0);

    assert_eq!(
        render_calls(),
        calls_after_mount,
        "a sub-row scroll rebuilt the list"
    );
}

// ---- measured rows ------------------------------------------------------

/// Heights cycle 20, 30, 40: none of them the estimate's 20 on average.
fn tall_height(index: usize) -> f32 {
    20.0 + (index % 3) as f32 * 10.0
}

#[component]
fn tall_row(index: &usize) {
    view! {
        text(
            format!("row {index}"),
            style: StyleOf::new().with::<container::width>(Sizing::Fill).with::<container::height>(tall_height(*index)),
        )
    }
}

#[component]
fn measured_list() {
    let render_item: VirtualItemRenderer = VirtualItemRenderer::new(|index: usize| {
        Children::component::<TallRow>(TallRowProps { index })
    });
    let style: StyleOf<Container> = StyleOf::new()
        .with::<container::width>(Sizing::Fill)
        .with::<container::height>(VIEWPORT.height);
    view! {
        virtual_list(
            item_count: ITEM_COUNT,
            item_size: ItemSize::Estimated(ITEM_HEIGHT),
            viewport_height: VIEWPORT.height,
            render_item: render_item.clone(),
            style: style.clone(),
        )
    }
}

fn measured_root(app: &mut App) {
    app.mount::<MeasuredList>(MeasuredListProps {});
}

impl Harness {
    /// Measurements land after a layout and move rows in the next one.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.render();
        }
    }

    /// Each mounted row's offset from the content's origin, and its height.
    fn row_geometry(&self) -> Vec<(f32, f32)> {
        let (_, content) = self.list_nodes();
        let runtime = self.app.ui_runtime();
        let origin = runtime.node(content).expect("content").world_origin.y;
        runtime
            .children(content)
            .map(|row| {
                let node = runtime.node(row).expect("row");
                (node.world_origin.y - origin, node.layout.height())
            })
            .collect()
    }

    fn content_height(&self) -> f32 {
        let (_, content) = self.list_nodes();
        self.app.ui_runtime().node(content).expect("content").layout.height()
    }

    fn scroll_offset(&self) -> f32 {
        let (scroller, _) = self.list_nodes();
        self.app.ui_runtime().node(scroller).expect("scroller").scroll_offset.y
    }
}

#[test]
fn measured_rows_stack_at_their_own_heights() {
    let mut harness = Harness::with_root(measured_root);
    harness.settle();

    let rows = harness.row_geometry();
    assert!(!rows.is_empty());
    for (index, (_, height)) in rows.iter().enumerate() {
        assert_eq!(*height, tall_height(index), "row {index} is not its content's height");
    }
    for pair in rows.windows(2) {
        assert_eq!(pair[1].0, pair[0].0 + pair[0].1, "rows overlap or gap: {rows:?}");
    }

    // Measured rows replace the estimate in the scrollable height, including
    // rows measured in an earlier, larger window that are no longer mounted.
    let estimated = ITEM_COUNT as f32 * ITEM_HEIGHT;
    let mounted: f32 = (0..rows.len()).map(|index| tall_height(index) - ITEM_HEIGHT).sum();
    let height = harness.content_height();
    assert!(height >= estimated + mounted, "{height} leaves out measured rows");
    assert!(height <= estimated + 13.0 * 20.0, "{height} counts rows never mounted");

    // Taller rows mean fewer of them fill the viewport: at most 10 at 20px
    // and three of overscan, as estimated before measuring.
    assert!(rows.len() <= 13, "measuring did not shrink the window: {}", rows.len());
}

#[test]
fn measuring_rows_above_the_viewport_keeps_the_visible_row_in_place() {
    let mut harness = Harness::with_root(measured_root);
    harness.settle();
    // Far enough that the rows mounted there have never been measured.
    harness.scroll_by(4_000.0);
    let before = harness.scroll_offset();
    harness.settle();

    let rows = harness.row_geometry();
    let after = harness.scroll_offset();
    // Three rows of overscan sit above the first visible one. They grew
    // from the estimate once measured, and the offset grew with them.
    let grown: f32 = rows[..3].iter().map(|(_, height)| height - ITEM_HEIGHT).sum();
    assert!(grown > 0.0, "the rows above did not change: {rows:?}");
    assert_eq!(after - before, grown, "the offset did not follow the rows above it");
    let (top, height) = rows[3];
    assert!(
        top <= after && after < top + height,
        "the first visible row moved off the top of the viewport: {after} in {rows:?}"
    );
}

// ---- a component of its own on a Virtualizer -----------------------------

thread_local! {
    static CUSTOM: RefCell<Option<Signal<Virtualizer>>> = const { RefCell::new(None) };
}

/// What `virtual_list` does, written against `Virtualizer` directly: the
/// shape any component with its own row layout takes.
#[component]
fn custom_list() {
    let list: Virtualizer = state!(
        Virtualizer::new(ITEM_COUNT, ItemSize::Fixed(ITEM_HEIGHT), 2).with_viewport(VIEWPORT.height)
    );
    CUSTOM.with(|slot| *slot.borrow_mut() = Some(list.clone()));
    view! {
        container(
            style: StyleOf::new()
                .with::<container::width>(Sizing::Fill)
                .with::<container::height>(VIEWPORT.height)
                .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
            scroll_controller: list.controller(),
            on_scroll: |cx, event, event_cx| list.on_scroll(cx, event, event_cx),
        ) {
            container(
                style: StyleOf::new()
                    .with::<container::position>(PositionStyle::Relative)
                    .with::<container::width>(Sizing::Fill)
                    .with::<container::height>(list.total_size()),
            ) {
                for index in list.range().iter() {
                    text(
                        format!("row {index}"),
                        style: StyleOf::new()
                            .with::<container::position>(PositionStyle::Absolute)
                            .with::<container::inset>(EdgeInsets::new(0.0, 0.0, list.offset_of(*index), 0.0))
                            .with::<container::width>(Sizing::Fill)
                            .with::<container::height>(ITEM_HEIGHT),
                    )
                    .key(*index)
                }
            }
        }
    }
}

fn custom_root(app: &mut App) {
    app.mount::<CustomList>(CustomListProps {});
}

#[test]
fn a_custom_component_scrolls_an_index_into_view() {
    let mut harness = Harness::with_root(custom_root);
    harness.settle();
    assert_eq!(harness.row_count(), 12, "ten visible rows and two of overscan");

    let list = CUSTOM.with(|slot| slot.borrow().clone()).expect("mounted");
    assert!(list.with(|list| list.scroll_to_index(5_000, ScrollAlign::Start)));
    harness.settle();

    assert_eq!(harness.scroll_offset(), 5_000.0 * ITEM_HEIGHT);
    assert_eq!(list.with(Virtualizer::range).start, 4_998);
    let (_, content) = harness.list_nodes();
    let runtime = harness.app.ui_runtime();
    let origin = runtime.node(content).expect("content").world_origin.y;
    let tops: Vec<f32> = runtime
        .children(content)
        .map(|row| runtime.node(row).expect("row").world_origin.y - origin)
        .collect();
    assert_eq!(tops.first().copied(), Some(4_998.0 * ITEM_HEIGHT));
    assert_eq!(harness.row_count(), 14, "overscan on both sides once away from the top");
}
