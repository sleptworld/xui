//! Cost of a list with and without virtualization, at sizes where the
//! difference stops being academic.
//!
//! ```text
//! cargo test -p xui-components --release --test virtual_list_bench -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
#[allow(unused_imports)]
use xui_components::*;
use xui_core::prelude::*;
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_macros::component;

const ITEM_HEIGHT: f32 = 20.0;
const VIEWPORT: Size<f32> = Size {
    width: 400.0,
    height: 600.0,
};

static ROWS: AtomicUsize = AtomicUsize::new(100);
static BENCH_LOCK: Mutex<()> = Mutex::new(());

fn exclusive() -> MutexGuard<'static, ()> {
    BENCH_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One row: a label and a swatch.
#[component]
fn bench_row(index: &usize) {
    view! {
        container(
            style: StyleOf::new()
                .with::<container::width>(Sizing::Fill)
                .with::<container::height>(ITEM_HEIGHT)
                .with::<container::background>(Color::hex("#141414")),
        ) {
            text(
                format!("row {index}"),
                style: StyleOf::new().with::<container::width>(120.0).with::<container::height>(16.0),
            )
            container(
                style: StyleOf::new()
                    .with::<container::width>(16.0)
                    .with::<container::height>(16.0)
                    .with::<container::background>(Color::hex("#3b82f6")),
            )
        }
    }
}

/// Every row mounted, which is what the framework did before virtualization.
#[component]
fn plain_rows() {
    let rows: usize = ROWS.load(Ordering::Relaxed);
    view! {
        container(
            style: StyleOf::new()
                .with::<container::width>(Sizing::Fill)
                .with::<container::height>(VIEWPORT.height)
                .with::<container::scroll_direction>(ScrollDirectionStyle::Vertical),
        ) {
            for index in 0..*rows {
                bench_row(index: *index)
            }
        }
    }
}

#[component]
fn virtual_rows() {
    let count: usize = ROWS.load(Ordering::Relaxed);
    let render_item: VirtualItemRenderer =
        VirtualItemRenderer::new(|index: usize| Children::component::<BenchRow>(BenchRowProps { index }));
    view! {
        virtual_list(
            item_count: *count,
            item_height: ITEM_HEIGHT,
            viewport_height: VIEWPORT.height,
            render_item: render_item.clone(),
            style: StyleOf::new().with::<container::width>(Sizing::Fill).with::<container::height>(VIEWPORT.height),
        )
    }
}

fn plain_list(app: &mut App) {
    app.mount::<PlainRows>(PlainRowsProps {});
}

fn virtual_list_root(app: &mut App) {
    app.mount::<VirtualRows>(VirtualRowsProps {});
}

struct Harness {
    app: App,
    text: TextHost<CosmicEngine>,
    backend: MockRenderBackend,
}

impl Harness {
    fn mount(root: AppRoot, rows: usize) -> (Self, Duration) {
        ROWS.store(rows, Ordering::Relaxed);
        let text = TextHost::new(CosmicEngine::new(1.0));
        // Components mount as the app is built, so that is part of the mount.
        let started = Instant::now();
        let mut app = App::new(root);
        app.resize(VIEWPORT);
        let mut harness = Self {
            app,
            text,
            backend: MockRenderBackend::default(),
        };
        harness.render();
        while harness.app.is_dirty() {
            harness.render();
        }
        let mount = started.elapsed();
        (harness, mount)
    }

    fn render(&mut self) {
        self.app
            .render(&mut self.backend, &mut self.text)
            .expect("mock backend cannot fail");
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

    /// A frame with a layout-affecting change, which is the expensive kind.
    fn resize_frame(&mut self, width: f32) -> Duration {
        self.app.resize(Size::new(width, VIEWPORT.height));
        let started = Instant::now();
        self.render();
        started.elapsed()
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn virtualized_versus_plain() {
    let _exclusive = exclusive();
    println!("rows,mode,hosts,mount_ms,resize_frame_ms");

    for rows in [100usize, 400, 1_600] {
        for (label, root) in [
            (
                "plain",
                plain_list as AppRoot,
            ),
            ("virtual", virtual_list_root),
        ] {
            let (mut harness, mount) = Harness::mount(root, rows);
            let hosts = harness.total_hosts();
            let mut samples: Vec<Duration> = (0..20)
                .map(|i| harness.resize_frame(VIEWPORT.width - (i % 2) as f32))
                .collect();
            samples.sort_unstable();
            println!(
                "{rows},{label},{hosts},{:.3},{:.3}",
                ms(mount),
                ms(samples[samples.len() / 2])
            );
        }
    }

    // A size no plain list can reach.
    for rows in [100_000usize, 1_000_000] {
        let (mut harness, mount) = Harness::mount(virtual_list_root, rows);
        let hosts = harness.total_hosts();
        let mut samples: Vec<Duration> = (0..20)
            .map(|i| harness.resize_frame(VIEWPORT.width - (i % 2) as f32))
            .collect();
        samples.sort_unstable();
        println!(
            "{rows},virtual,{hosts},{:.3},{:.3}",
            ms(mount),
            ms(samples[samples.len() / 2])
        );
    }
}
