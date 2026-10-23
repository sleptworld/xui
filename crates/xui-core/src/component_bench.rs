//! Cost of the component layer: rendering, reconciling and committing a tree
//! of `#[component]`s written with `xui!`, as an application writes it.
//!
//! Run with:
//!
//! ```text
//! cargo test -p xui-core --release --lib component_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `frame_bench` builds its rows from hand-written drafts; this one goes
//! through the element macros and the component runtime, which is where
//! templates, slots and props are paid for. Each case prints the time spent
//! in the component flush (render, reconcile, commit to hosts) apart from the
//! rest of the frame (style, layout, scene).
//!
//! - `leaf_state_change`: one row's own state changes. Only that row has to
//!   render; everything else is the cost of finding it.
//! - `root_state_change`: the list's owner re-renders and hands every row
//!   new props, so every row renders again.
//! - `branch_toggle`: every row holds an `if` between static elements, and
//!   the root flips it for all of them.
//! - `root_state_change_memo`: the same re-render, with `#[component(memo)]`
//!   sections handed props equal to the last ones, so none renders again.
//! - `root_state_change_same_values`: the same sections without `memo`, so
//!   every section and row renders again, into templates whose values are
//!   all what they were.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use core::convert::Infallible;
use xui_interface::{Color, Size};
use xui_macros::{component, xui};

use crate::app::{App, ComponentFn};
use crate::render::{BuiltFrame, RenderBackend};
use crate::state::{HookContext, State};
use crate::text::{TextHost, testing::ZeroTextBackend};
#[allow(unused_imports)]
use crate::test_prelude::*;
use crate::ElementDesc;

const ROW_COUNTS: [usize; 4] = [25, 100, 400, 1600];
const SAMPLES: usize = 30;
const VIEWPORT: Size<f32> = Size {
    width: 900.0,
    height: 700.0,
};

static ROWS: AtomicUsize = AtomicUsize::new(25);
static BENCH_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static ROOT: RefCell<Option<State<u32>>> = const { RefCell::new(None) };
    /// The state of the row in the middle of the list.
    static MIDDLE: RefCell<Option<State<bool>>> = const { RefCell::new(None) };
}

fn exclusive() -> MutexGuard<'static, ()> {
    BENCH_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set<T: 'static>(slot: &'static std::thread::LocalKey<RefCell<Option<State<T>>>>, value: T) {
    let state = slot.with(|slot| slot.borrow().expect("rendered"));
    slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(value));
}

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

/// A row: its own state, a label, and a swatch whose width follows the
/// state. Three hosts in one template.
#[component]
fn bench_row(index: &usize, label: &String) {
    let wide = cx.use_state(|| false);
    if *index == ROWS.load(Ordering::Relaxed) / 2 {
        MIDDLE.with(|slot| *slot.borrow_mut() = Some(wide));
    }
    let width = if *wide.get() { 24.0 } else { 16.0 };
    xui! {
        <row height={20.0} gap={4.0} background={Color::BLACK}>
            <text>{label.clone()}</text>
            <container width={width} height={16.0} background={Color::WHITE} />
        </row>
    }
}

/// A section of ten rows, so the tree has depth as well as breadth.
#[component]
fn bench_section(start: &usize, generation: &u32) {
    let rows = (*start..*start + 10)
        .map(|index| {
            xui! { <bench_row key={index.to_string()} index={index} label={format!("row {index} / {generation}")} /> }
        })
        .collect::<Vec<_>>();
    xui! { <column gap={2.0}>{rows}</column> }
}

fn bench_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let generation = cx.use_state(|| 0u32);
    ROOT.with(|slot| *slot.borrow_mut() = Some(generation));
    let generation = *generation.get();
    let sections = (0..ROWS.load(Ordering::Relaxed) / 10)
        .map(|section| {
            let start = section * 10;
            xui! { <bench_section key={section.to_string()} start={start} generation={generation} /> }
        })
        .collect::<Vec<_>>();
    xui! {
        <column width={900.0}>
            <text>{"header"}</text>
            <column>{sections}</column>
        </column>
    }
}

/// A section that renders again only when `start` changes.
#[component(memo)]
fn bench_memo_section(start: &usize) {
    let rows = (*start..*start + 10)
        .map(|index| {
            xui! { <bench_row key={index.to_string()} index={index} label={format!("row {index}")} /> }
        })
        .collect::<Vec<_>>();
    xui! { <column gap={2.0}>{rows}</column> }
}

fn bench_memo_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let generation = cx.use_state(|| 0u32);
    ROOT.with(|slot| *slot.borrow_mut() = Some(generation));
    let generation = *generation.get();
    let sections = (0..ROWS.load(Ordering::Relaxed) / 10)
        .map(|section| {
            let start = section * 10;
            xui! { <bench_memo_section key={section.to_string()} start={start} /> }
        })
        .collect::<Vec<_>>();
    xui! {
        <column width={900.0}>
            <text>{format!("generation {generation}")}</text>
            <column>{sections}</column>
        </column>
    }
}

/// [`bench_memo_section`] without `memo`: it renders again whenever its
/// parent does, and so do its rows.
#[component]
fn bench_plain_section(start: &usize) {
    let rows = (*start..*start + 10)
        .map(|index| {
            xui! { <bench_row key={index.to_string()} index={index} label={format!("row {index}")} /> }
        })
        .collect::<Vec<_>>();
    xui! { <column gap={2.0}>{rows}</column> }
}

fn bench_same_values_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let generation = cx.use_state(|| 0u32);
    ROOT.with(|slot| *slot.borrow_mut() = Some(generation));
    let _ = *generation.get();
    let sections = (0..ROWS.load(Ordering::Relaxed) / 10)
        .map(|section| {
            let start = section * 10;
            xui! { <bench_plain_section key={section.to_string()} start={start} /> }
        })
        .collect::<Vec<_>>();
    xui! {
        <column width={900.0}>
            <text>{"header"}</text>
            <column>{sections}</column>
        </column>
    }
}

/// A row whose middle element comes and goes, between static siblings.
#[component]
fn bench_branch_row(index: &usize, open: &bool) {
    let index = *index;
    let open = *open;
    xui_macros::view! {
        row(height: 20.0, gap: 4.0) {
            text("label")
            if open {
                text(format!("row {index}"))
            }
            container(width: 16.0, height: 16.0, background: Color::WHITE)
            text("end")
        }
    }
}

fn bench_branch_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let generation = cx.use_state(|| 0u32);
    ROOT.with(|slot| *slot.borrow_mut() = Some(generation));
    let open = *generation.get() % 2 == 1;
    let rows = (0..ROWS.load(Ordering::Relaxed))
        .map(|index| xui! { <bench_branch_row key={index.to_string()} index={index} open={open} /> })
        .collect::<Vec<_>>();
    xui! { <column width={900.0}>{rows}</column> }
}

struct Harness {
    app: App,
    text: TextHost<ZeroTextBackend>,
    backend: NullBackend,
}

impl Harness {
    fn new(rows: usize, root: ComponentFn) -> Self {
        ROWS.store(rows, Ordering::Relaxed);
        let mut app = App::new(root);
        app.resize(VIEWPORT);
        let mut harness = Self {
            app,
            text: TextHost::new(ZeroTextBackend),
            backend: NullBackend,
        };
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

    /// One frame after a state change: the component flush, the tree
    /// update (style and layout), then the rest of the frame.
    fn frame(&mut self) -> [Duration; 3] {
        let started = Instant::now();
        self.app.flush_components();
        let flushed = started.elapsed();
        let started = Instant::now();
        self.app.ui_runtime_mut().update_tree(VIEWPORT, &mut self.text);
        let updated = started.elapsed();
        let started = Instant::now();
        self.render();
        [flushed, updated, started.elapsed()]
    }
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// `XUI_BENCH_ROWS` narrows the sweep to one size and `XUI_BENCH_SAMPLES`
/// lengthens it, for running under a profiler.
fn run(name: &str, change: impl Fn(usize)) {
    run_with(name, bench_root, change);
}

fn run_with(name: &str, root: ComponentFn, change: impl Fn(usize)) {
    let env = |name: &str| std::env::var(name).ok().and_then(|value| value.parse().ok());
    let row_counts = env("XUI_BENCH_ROWS").map_or(ROW_COUNTS.to_vec(), |rows| vec![rows]);
    let iterations = env("XUI_BENCH_SAMPLES").unwrap_or(SAMPLES);
    println!("{name}\nrows,components_ms,tree_ms,draw_ms");
    for rows in row_counts {
        let mut harness = Harness::new(rows, root);
        let mut samples: [Vec<Duration>; 3] = Default::default();
        for iteration in 0..iterations {
            change(iteration);
            for (phase, duration) in harness.frame().into_iter().enumerate() {
                samples[phase].push(duration);
            }
            assert!(!harness.app.is_dirty(), "a frame left work behind");
        }
        let [components, tree, draw] = samples.map(|mut phase| ms(median(&mut phase)));
        println!("{rows},{components:.4},{tree:.4},{draw:.4}");
    }
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn leaf_state_change() {
    let _exclusive = exclusive();
    run("leaf_state_change", |iteration| {
        set(&MIDDLE, iteration % 2 == 0)
    });
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn root_state_change() {
    let _exclusive = exclusive();
    run("root_state_change", |iteration| {
        set(&ROOT, iteration as u32 + 1)
    });
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn root_state_change_memo() {
    let _exclusive = exclusive();
    run_with("root_state_change_memo", bench_memo_root, |iteration| {
        set(&ROOT, iteration as u32 + 1)
    });
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn root_state_change_same_values() {
    let _exclusive = exclusive();
    run_with(
        "root_state_change_same_values",
        bench_same_values_root,
        |iteration| set(&ROOT, iteration as u32 + 1),
    );
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn branch_toggle() {
    let _exclusive = exclusive();
    run_with("branch_toggle", bench_branch_root, |iteration| {
        set(&ROOT, iteration as u32 + 1)
    });
}

