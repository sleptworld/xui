//! Full-frame cost with real text shaping and real rasterization.
//!
//! Run with:
//!
//! ```text
//! cargo test -p xui-skia --release --test raster_budget -- --ignored --nocapture
//! ```
//!
//! Complements `xui`'s `frame_bench`, which stops at the built frame. Two
//! caveats on the numbers:
//!
//! - headless Skia rasterizes on the CPU, while the shipping macOS path runs on
//!   Metal, so this is not a prediction of on-device raster cost;
//! - the CPU path honours the damage tracker, whereas `SkiaBackend::submit`
//!   forces full-viewport damage whenever a GPU context is present. The gap
//!   between the idle/animating columns here therefore shows what damage
//!   tracking buys, which is a benefit the GPU path does not currently take.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use xui::prelude::*;
use xui::text::TextHost;
use xui_interface::events::{
    Modifiers, PointerButtons, PointerKind, RawEvent, RawPointerMove, XuiPointerId,
};
use xui_skia::{SkiaBackend, SkiaBackendOptions};
use xui_text_engine::CosmicEngine;

const ROW_COUNTS: [usize; 4] = [25, 100, 400, 1600];
const VIEWPORT: Size<f32> = Size {
    width: 900.0,
    height: 700.0,
};
const FRAME_DELTA: Duration = Duration::from_micros(8333);

static ROWS: AtomicUsize = AtomicUsize::new(25);

fn pointer_move(position: Point) -> RawEvent {
    RawEvent::PointerMove(RawPointerMove {
        position,
        pointer_id: XuiPointerId::new(0),
        device_id: None,
        kind: PointerKind::Mouse,
        button: None,
        buttons: PointerButtons::default(),
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
    })
}

/// Long enough that every sampled frame is still interpolating.
fn long_transition() -> Transition {
    Transition::new(Duration::from_secs(4)).ease(Easing::CubicOut)
}

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

fn dashboard(_cx: &mut HookContext<'_>) -> ElementDesc {
    let rows: Vec<ElementDesc> = (0..ROWS.load(Ordering::Relaxed)).map(row).collect();
    container()
        .style(
            Style::new()
                .size(Size::fill())
                .background(Color::hex("#0a0a0a"))
                .scroll_vertical(),
        )
        .into_element_desc(rows)
}

fn median(samples: &mut Vec<Duration>) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2].as_secs_f64() * 1e3
}

#[test]
#[ignore = "benchmark; run with --release --ignored"]
fn raster_budget() {
    println!("rows,idle_ms,animating_ms");
    for rows in ROW_COUNTS {
        ROWS.store(rows, Ordering::Relaxed);
        let mut app = App::new(dashboard);
        app.resize(VIEWPORT);
        let mut text = TextHost::new(CosmicEngine::new(1.0));
        let mut backend = SkiaBackend::<CosmicEngine>::headless(1.0, SkiaBackendOptions::default());

        for _ in 0..5 {
            app.render(&mut backend, &mut text).expect("frame renders");
        }

        let mut idle = Vec::new();
        for _ in 0..60 {
            let started = Instant::now();
            app.render(&mut backend, &mut text).expect("frame renders");
            idle.push(started.elapsed());
        }

        // Hover the first row through the normal event path so one row starts
        // transitioning its background.
        app.dispatch_event(pointer_move(Point::new(60.0, 18.0)), &mut text);
        app.render(&mut backend, &mut text).expect("frame renders");
        assert!(
            app.has_running_style_animations(),
            "hover did not start a transition, so the samples measure idle frames"
        );

        let mut animating = Vec::new();
        for _ in 0..120 {
            let started = Instant::now();
            app.tick_style_animations(FRAME_DELTA);
            app.render(&mut backend, &mut text).expect("frame renders");
            animating.push(started.elapsed());
        }

        println!(
            "{rows},{:.4},{:.4}",
            median(&mut idle),
            median(&mut animating)
        );
    }
}
