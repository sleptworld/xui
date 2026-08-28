//! End-to-end checks that `virtual_list` mounts only the rows near the
//! viewport and still reports the full scrollable height.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use xui::prelude::*;
use xui::text::TextHost;
use xui_components::VirtualItemRenderer;
#[allow(unused_imports)]
use xui_components::*;
use xui_interface::Translation;
use xui_interface::events::{Modifiers, RawEvent, RawWheel, ScrollDelta};
use xui_text_engine::CosmicEngine;

const ITEM_COUNT: usize = 100_000;
const ITEM_HEIGHT: f32 = 20.0;
const VIEWPORT: Size<f32> = Size {
    width: 400.0,
    height: 200.0,
};

thread_local! {
    /// Indices the list actually asked for, most recent render last.
    static RENDERED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}
static RENDER_CALLS: AtomicUsize = AtomicUsize::new(0);

fn list_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let render_item: VirtualItemRenderer = cx.use_callback((), |index: usize| {
        RENDER_CALLS.fetch_add(1, Ordering::Relaxed);
        RENDERED.with(|slot| slot.borrow_mut().push(index));
        TextWidget::new(format!("row {index}"))
            .style(Style::new().width(Sizing::Fill).height(ITEM_HEIGHT))
            .into_element_desc()
    });

    let style = Style::new().width(Sizing::Fill).height(VIEWPORT.height);
    xui! {
        <virtual_list
            item_count={ITEM_COUNT}
            item_height={ITEM_HEIGHT}
            viewport_height={VIEWPORT.height}
            render_item={render_item}
            style={style}
        />
    }
}

struct Harness {
    app: App,
    text: TextHost<CosmicEngine>,
    backend: MockRenderBackend,
}

impl Harness {
    fn new() -> Self {
        let mut app = App::new(list_root);
        app.resize(VIEWPORT);
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
            .render(&mut self.backend, &mut self.text)
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
        self.app.dispatch_event(event, &mut self.text);
        self.render();
    }
}

fn rendered_indices() -> Vec<usize> {
    RENDERED.with(|slot| slot.borrow().clone())
}

fn reset_tracking() {
    RENDERED.with(|slot| slot.borrow_mut().clear());
    RENDER_CALLS.store(0, Ordering::Relaxed);
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
    let calls_after_mount = RENDER_CALLS.load(Ordering::Relaxed);

    // One pixel cannot bring a new row into range given the overscan, so the
    // runtime should move the content without asking the component again.
    harness.scroll_by(1.0);

    assert_eq!(
        RENDER_CALLS.load(Ordering::Relaxed),
        calls_after_mount,
        "a sub-row scroll rebuilt the list"
    );
}
