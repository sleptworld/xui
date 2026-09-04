//! Cost of a list with and without virtualization, at sizes where the
//! difference stops being academic.
//!
//! ```text
//! cargo test -p xui-components --release --test virtual_list_bench -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use xui::prelude::*;
use xui::text::TextHost;
#[allow(unused_imports)]
use xui_components::*;
use xui_cosmic::CosmicEngine;

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

fn row(index: usize) -> ElementDesc {
    ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::Fill)
                .height(ITEM_HEIGHT)
                .background(Color::hex("#141414")),
        )
        .into_element_desc(vec![
            TextWidget::new(format!("row {index}"))
                .style(Style::new().width(120.0).height(16.0))
                .into_element_desc(),
            ContainerWidget::new()
                .style(
                    Style::new()
                        .width(16.0)
                        .height(16.0)
                        .background(Color::hex("#3b82f6")),
                )
                .into_element_desc(Vec::new()),
        ])
}

/// Every row mounted, which is what the framework did before virtualization.
fn plain_list(_cx: &mut HookContext<'_>) -> ElementDesc {
    let rows: Vec<ElementDesc> = (0..ROWS.load(Ordering::Relaxed)).map(row).collect();
    ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::Fill)
                .height(VIEWPORT.height)
                .scroll_vertical(),
        )
        .into_element_desc(rows)
}

fn virtual_list_root(cx: &mut HookContext<'_>) -> ElementDesc {
    let render_item: Callback<usize, ElementDesc> = cx.use_callback((), row);
    let count = ROWS.load(Ordering::Relaxed);
    let style = Style::new().width(Sizing::Fill).height(VIEWPORT.height);
    xui! {
        <virtual_list
            item_count={count}
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
    fn mount(root: fn(&mut HookContext<'_>) -> ElementDesc, rows: usize) -> (Self, Duration) {
        ROWS.store(rows, Ordering::Relaxed);
        let mut app = App::new(root);
        app.resize(VIEWPORT);
        let mut harness = Self {
            app,
            text: TextHost::new(CosmicEngine::new(1.0)),
            backend: MockRenderBackend::default(),
        };
        let started = Instant::now();
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
                plain_list as fn(&mut HookContext<'_>) -> ElementDesc,
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
