//! Frame-cost instruments for the host-update and scene-compile pipelines.
//!
//! Run with:
//!
//! ```text
//! cargo test -p xui --release --lib frame_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Every case is `#[ignore]`d: a debug build is far too slow for the larger
//! tree sizes, and wall-clock numbers are meaningless under `--test-threads`
//! contention.
//!
//! The printed timings are for a human to diff across a change. The properties
//! that must hold on *any* machine are asserted instead, so an algorithmic
//! regression fails the test even when the timings look fine on a fast box:
//!
//! - an idle frame must touch no nodes at all,
//! - repeated structural edits must not leak or over-collect compiled entities.
//!
//! Nothing here reaches into private state beyond what the crate already
//! exposes, so the production path carries no profiling hooks.

use crate::app::ComponentFn;
use crate::fiber::{ComponentRender, ComponentType};
use crate::prelude::*;
use crate::render::{BuiltFrame, RenderBackend};
use crate::state::State;
use crate::text::{TextHost, testing::ZeroTextBackend};
use crate::widgets::{WidgetI, container, text};
use core::convert::Infallible;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use xui_interface::{Color, NodeId, Size, Style, WidgetState};

/// Tree sizes to sweep. Each row expands to 4 hosts and ~21 scene nodes, so
/// 1600 rows is ~6.4k hosts / ~34k scene nodes.
const ROW_COUNTS: [usize; 4] = [25, 100, 400, 1600];
const VIEWPORT: Size<f32> = Size {
    width: 900.0,
    height: 700.0,
};
/// 120Hz frame delta.
const FRAME_DELTA: Duration = Duration::from_micros(8333);

static ROWS: AtomicUsize = AtomicUsize::new(25);
/// The fixture is configured through process-wide state, so the cases have to
/// run one at a time no matter what `--test-threads` says.
static BENCH_LOCK: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    BENCH_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
thread_local! {
    static TOGGLE: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
}

/// Discards frames instead of rasterizing them: these cases measure the
/// backend-independent cost. Raster cost lives in `xui-skia/tests`.
#[derive(Default)]
struct NullBackend;

impl<T> RenderBackend<T> for NullBackend {
    type Error = Infallible;

    fn begin_frame(&mut self, _size: Size<f32>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn submit(&mut self, _frame: &BuiltFrame, _text: &mut T) -> Result<(), Self::Error> {
        Ok(())
    }

    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Long enough that every sampled frame is still interpolating; a short
/// transition would leave the median measuring idle frames.
fn long_transition() -> Transition {
    Transition::new(Duration::from_secs(4)).ease(Easing::CubicOut)
}

/// One list row: a hoverable card with a label and two swatches (4 hosts).
fn row(index: usize) -> ElementDesc {
    let style = Style::new()
        .width(Sizing::fill())
        .height(28.0)
        .padding(EdgeInsets::all(4.0))
        .background(Color::hex("#141414"))
        .when(WidgetState::HOVERED, |patch| {
            patch.background(Color::hex("#2a2a2a"))
        })
        .transition(long_transition());

    container().style(style).into_element_desc(vec![
        TextWidget::new(if index % 3 == 0 { "row label" } else { "item" })
            .style(Style::new().width(120.0).height(20.0))
            .into_element_desc(),
        container()
            .style(
                Style::new()
                    .width(16.0)
                    .height(16.0)
                    .background(Color::hex("#3b82f6")),
            )
            .into_element_desc(Vec::new()),
        container()
            .style(
                Style::new()
                    .width(16.0)
                    .height(16.0)
                    .background(Color::hex("#22c55e")),
            )
            .into_element_desc(Vec::new()),
    ])
}

/// Scrollable list root. Flipping `TOGGLE` drops one row, which is the cheapest
/// edit that still forces a structural scene rebuild.
fn dashboard(cx: &mut HookContext<'_>) -> ElementDesc {
    let toggle = cx.use_state(|| false);
    TOGGLE.with(|slot| *slot.borrow_mut() = Some(toggle));
    let dropped = usize::from(*toggle.get());
    let rows: Vec<ElementDesc> = (0..ROWS.load(Ordering::Relaxed) - dropped)
        .map(row)
        .collect();
    container()
        .style(
            Style::new()
                .size(Size::fill())
                .background(Color::hex("#0a0a0a"))
                .scroll_vertical(),
        )
        .into_element_desc(rows)
}

/// The row list as a component, so the reconciler can bail out on it. Host
/// subtrees have no equivalent: they are always walked.
fn row_list(_cx: &mut HookContext<'_>, props: Option<ErasedPropsRef<'_>>) -> ElementDesc {
    let count = props
        .and_then(|props| props.downcast_ref::<usize>())
        .copied()
        .expect("row count props");
    let rows: Vec<ElementDesc> = (0..count).map(row).collect();
    container()
        .style(Style::new().size(Size::fill()).scroll_vertical())
        .into_element_desc(rows)
}

fn row_list_render() -> ComponentRender {
    ComponentRender::new(ComponentType::new("frame_bench::row_list"), row_list)
}

/// Rebuilds the list element on every render, so its props are a fresh
/// allocation and the child always re-renders.
fn rebuilding_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let toggle = cx.use_state(|| false);
    TOGGLE.with(|slot| *slot.borrow_mut() = Some(toggle));
    let count = ROWS.load(Ordering::Relaxed);
    crate::widgets::component(row_list_render())
        .props(count)
        .into()
}

/// Holds the list element in a memo, so every render hands the child the same
/// props allocation and the reconciler can skip the subtree.
fn memoized_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let toggle = cx.use_state(|| false);
    TOGGLE.with(|slot| *slot.borrow_mut() = Some(toggle));
    let count = ROWS.load(Ordering::Relaxed);
    let element = cx.use_memo(move || {
        let desc: ElementDesc = crate::widgets::component(row_list_render())
            .props(count)
            .into();
        desc
    });
    element.get().clone()
}

struct Harness {
    app: App,
    text: TextHost<ZeroTextBackend>,
    backend: NullBackend,
}

impl Harness {
    fn new(rows: usize) -> Self {
        Self::with_root(rows, dashboard)
    }

    fn with_root(rows: usize, root: ComponentFn) -> Self {
        ROWS.store(rows, Ordering::Relaxed);
        let mut app = App::new(root);
        app.resize(VIEWPORT);
        let mut harness = Self {
            app,
            text: TextHost::new(ZeroTextBackend),
            backend: NullBackend,
        };
        // Mount and reach a steady state before anything is sampled.
        for _ in 0..3 {
            harness.render();
        }
        harness
    }

    fn render(&mut self) {
        self.app
            .render(&mut self.backend, &mut self.text)
            .expect("null backend cannot fail");
    }

    fn host_count(&self) -> usize {
        fn walk(runtime: &UiRuntime, id: NodeId) -> usize {
            1 + runtime
                .children(id)
                .map(|c| walk(runtime, c))
                .sum::<usize>()
        }
        walk(self.app.ui_runtime(), self.app.ui_runtime().root())
    }

    /// Direct children of the list, i.e. one node per row.
    fn row_ids(&self) -> Vec<NodeId> {
        let runtime = self.app.ui_runtime();
        let list = runtime
            .children(runtime.root())
            .next()
            .expect("list root exists");
        let ids: Vec<_> = runtime.children(list).collect();
        assert!(!ids.is_empty(), "the list mounted no rows");
        ids
    }

    /// Live compiled entities: `(pictures, primitives, spatials, clips)`.
    fn compiled_entities(&self) -> (usize, usize, usize, usize) {
        let scene = self
            .app
            .ui_runtime()
            .compiled_scene()
            .expect("a frame has been built");
        (
            scene.pictures.len(),
            scene.primitives.len(),
            scene.spatial_nodes.len(),
            scene.clips.len(),
        )
    }
}

fn median(samples: &mut Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn idle_frame_is_free() {
    let _exclusive = exclusive();
    println!("rows,hosts,idle_ms");
    for rows in ROW_COUNTS {
        let mut harness = Harness::new(rows);
        let hosts = harness.host_count();

        let mut samples = Vec::new();
        for _ in 0..120 {
            let started = Instant::now();
            harness.render();
            samples.push(started.elapsed());
        }

        // The whole frame must bail out: no style, state, or shape recompute.
        // This is the property that keeps idle cost independent of tree size.
        assert_eq!(
            harness.app.ui_runtime().update_visits,
            0,
            "an idle frame recomputed nodes at {rows} rows"
        );
        println!("{rows},{hosts},{:.4}", ms(median(&mut samples)));
    }
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn interaction_frame_cost() {
    let _exclusive = exclusive();
    println!("rows,hosts,hover_ms");
    for rows in ROW_COUNTS {
        let mut harness = Harness::new(rows);
        let hosts = harness.host_count();
        let ids = harness.row_ids();

        // Walk the hover down the list, one row per frame.
        let mut samples = Vec::new();
        let mut previous: Option<NodeId> = None;
        for index in 0..120 {
            let target = ids[index % ids.len()];
            let runtime = harness.app.ui_runtime_mut();
            if let Some(previous) = previous {
                runtime.set_widget_state_flag(previous, WidgetState::HOVERED, false);
            }
            runtime.set_widget_state_flag(target, WidgetState::HOVERED, true);
            previous = Some(target);

            let started = Instant::now();
            harness.render();
            samples.push(started.elapsed());
        }
        println!("{rows},{hosts},{:.4}", ms(median(&mut samples)));
    }
}

/// Splits an animating frame into the four phases `App::render` runs, using the
/// same public entry points, so a regression can be attributed without adding
/// profiling hooks to the production path.
#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn animation_frame_phase_split() {
    let _exclusive = exclusive();
    println!("rows,hosts,tick_ms,update_tree_ms,build_frame_ms,finish_frame_ms,total_ms");
    for rows in ROW_COUNTS {
        let mut harness = Harness::new(rows);
        let hosts = harness.host_count();

        // One row transitions its background; nothing else changes.
        let first_row = harness.row_ids()[0];
        harness
            .app
            .ui_runtime_mut()
            .set_widget_state_flag(first_row, WidgetState::HOVERED, true);
        harness.render();

        let (mut tick, mut update, mut build, mut finish) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..120 {
            let started = Instant::now();
            harness.app.tick_style_animations(FRAME_DELTA);
            tick.push(started.elapsed());

            let started = Instant::now();
            harness
                .app
                .ui_runtime_mut()
                .update_tree(VIEWPORT, &mut harness.text);
            update.push(started.elapsed());

            let started = Instant::now();
            let frame = harness
                .app
                .ui_runtime_mut()
                .build_render_frame()
                .expect("scene compiles");
            build.push(started.elapsed());

            let started = Instant::now();
            if let Some(frame) = frame {
                harness.app.ui_runtime_mut().finish_render_frame(&frame);
            }
            finish.push(started.elapsed());
        }

        assert!(
            harness.app.has_running_style_animations(),
            "the transition ended early, so the samples measured idle frames"
        );

        let (tick, update, build, finish) = (
            median(&mut tick),
            median(&mut update),
            median(&mut build),
            median(&mut finish),
        );
        println!(
            "{rows},{hosts},{:.4},{:.4},{:.4},{:.4},{:.4}",
            ms(tick),
            ms(update),
            ms(build),
            ms(finish),
            ms(tick + update + build + finish),
        );
    }
}

/// Adding and removing a single host subtree, driven straight against the
/// runtime so the component layer stays out of the measurement. This is the
/// path that still forces a full `rebuild_structure`, because a plain container
/// tree has no isolated-layer ancestor to scope the rebuild to.
#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn structural_frame_cost() {
    let _exclusive = exclusive();
    println!("rows,hosts,update_tree_ms,build_frame_ms,finish_frame_ms,total_ms");
    for rows in ROW_COUNTS {
        let mut harness = Harness::new(rows);
        let hosts = harness.host_count();
        let list = harness
            .app
            .ui_runtime()
            .children(harness.app.ui_runtime().root())
            .next()
            .expect("list root exists");
        let baseline = harness.compiled_entities();

        let (mut update, mut build, mut finish) = (Vec::new(), Vec::new(), Vec::new());
        for iteration in 0..40 {
            let runtime = harness.app.ui_runtime_mut();
            let added = if iteration % 2 == 0 {
                let widget = WidgetI::new(container().style(Style::new().width(16.0).height(16.0)));
                let key = widget.key();
                let props_hash = widget.props_hash();
                let interaction = widget.take_host_interaction();
                let id = runtime.create_node(key, props_hash, widget, interaction);
                runtime.append_child(list, id);
                Some(id)
            } else {
                None
            };

            let started = Instant::now();
            harness
                .app
                .ui_runtime_mut()
                .update_tree(VIEWPORT, &mut harness.text);
            update.push(started.elapsed());

            let started = Instant::now();
            let frame = harness
                .app
                .ui_runtime_mut()
                .build_render_frame()
                .expect("scene compiles");
            build.push(started.elapsed());

            let started = Instant::now();
            if let Some(frame) = frame {
                harness.app.ui_runtime_mut().finish_render_frame(&frame);
            }
            finish.push(started.elapsed());

            if let Some(added) = added {
                harness.app.ui_runtime_mut().remove_subtree(added);
            }
        }

        // Every added node was removed again, so the compiled scene must hold
        // exactly the entities it started with: drift either way means the
        // mark-and-sweep leaked or collected something still reachable.
        harness
            .app
            .ui_runtime_mut()
            .update_tree(VIEWPORT, &mut harness.text);
        if let Some(frame) = harness
            .app
            .ui_runtime_mut()
            .build_render_frame()
            .expect("scene compiles")
        {
            harness.app.ui_runtime_mut().finish_render_frame(&frame);
        }
        assert_eq!(
            harness.compiled_entities(),
            baseline,
            "compiled entity counts drifted after 40 add/remove cycles at {rows} rows"
        );

        let (update, build, finish) =
            (median(&mut update), median(&mut build), median(&mut finish));
        println!(
            "{rows},{hosts},{:.4},{:.4},{:.4},{:.4}",
            ms(update),
            ms(build),
            ms(finish),
            ms(update + build + finish),
        );
    }
}

/// A root-component state change: the whole element tree is re-rendered and
/// reconciled. Compare against `ComponentRuntime`'s rebuild budget.
/// What the pointer-equality bailout is worth: the same root state change,
/// once with the child's element rebuilt each render and once with it held in
/// a memo. Only the second can bail out.
#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn memoized_subtree_bailout() {
    let _exclusive = exclusive();
    println!("rows,mode,state_change_ms");
    for rows in ROW_COUNTS {
        for (label, root) in [
            ("rebuilt", rebuilding_root as ComponentFn),
            ("memoized", memoized_root as ComponentFn),
        ] {
            let mut harness = Harness::with_root(rows, root);
            let mut samples = Vec::new();
            for iteration in 0..20 {
                TOGGLE.with(|slot| {
                    slot.borrow()
                        .as_ref()
                        .expect("root component has mounted")
                        .set(iteration % 2 == 0)
                });
                let started = Instant::now();
                harness.render();
                samples.push(started.elapsed());
                while harness.app.is_dirty() {
                    harness.render();
                }
            }
            println!("{rows},{label},{:.4}", ms(median(&mut samples)));
        }
    }
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn component_rebuild_cost() {
    let _exclusive = exclusive();
    println!("rows,hosts,first_frame_ms,frames_to_settle");
    for rows in ROW_COUNTS {
        let mut harness = Harness::new(rows);
        let hosts = harness.host_count();

        let mut samples = Vec::new();
        let mut settle = Vec::new();
        for iteration in 0..20 {
            TOGGLE.with(|slot| {
                slot.borrow()
                    .as_ref()
                    .expect("root component has mounted")
                    .set(iteration % 2 == 0)
            });
            let started = Instant::now();
            harness.render();
            samples.push(started.elapsed());

            let mut frames = 1usize;
            while harness.app.is_dirty() {
                harness.render();
                frames += 1;
                assert!(
                    frames < 512,
                    "component rebuild never settled at {rows} rows"
                );
            }
            settle.push(frames);
        }

        settle.sort_unstable();
        println!(
            "{rows},{hosts},{:.4},{}",
            ms(median(&mut samples)),
            settle[settle.len() / 2]
        );
    }
}
