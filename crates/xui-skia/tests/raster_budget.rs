//! Full-frame cost with real text shaping and real rasterization.
//!
//! Run with:
//!
//! ```text
//! cargo test -p xui-skia --release --test raster_budget -- --ignored --nocapture
//! ```
//!
//! Complements `xui`'s `frame_bench`, which stops at the built frame.
//!
//! One caveat on the numbers: headless Skia rasterizes on the CPU, while the
//! shipping macOS path runs on Metal, so this is not a prediction of on-device
//! raster cost. Both paths do now honour the damage tracker — the GPU path
//! draws the scene into a persistent compositor surface and blits that to the
//! swapchain image — so the gap between the idle and animating columns is the
//! same benefit either one gets.

use std::time::{Duration, Instant};
use xui_core::prelude::*;
use xui_core::text::TextHost;
use xui_cosmic::CosmicEngine;
use xui_interface::events::{
    Modifiers, PointerButtons, PointerKind, RawEvent, RawPointerMove, XuiPointerId,
};
use xui_macros::component;
use xui_skia::{SkiaBackend, SkiaBackendOptions};

const ROW_COUNTS: [usize; 4] = [25, 100, 400, 1600];
const VIEWPORT: Size<f32> = Size {
    width: 900.0,
    height: 700.0,
};
const FRAME_DELTA: Duration = Duration::from_micros(8333);

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

fn row_style() -> StyleOf<Container> {
    StyleOf::<Container>::new()
        .with::<container::width>(Sizing::fill())
        .with::<container::height>(28.0)
        .with::<container::padding>(EdgeInsets::all(4.0))
        .with::<container::background>(Color::hex("#141414"))
        .when_style(
            WidgetStateMatcher::all(WidgetState::HOVERED),
            StyleOf::new().with::<container::background>(Color::hex("#2a2a2a")),
        )
}

#[component]
fn list_row(index: &usize) {
    view! {
        container(style: row_style(), transition: long_transition()) {
            text(if *index % 3 == 0 { "row label" } else { "item" }, width: 120.0, height: 20.0)
            container(width: 16.0, height: 16.0, background: Color::hex("#3b82f6"))
            container(width: 16.0, height: 16.0, background: Color::hex("#22c55e"))
        }
    }
}

#[component]
fn dashboard(rows: &usize) {
    view! {
        container(
            size: Size::fill(),
            background: Color::hex("#0a0a0a"),
            scroll_direction: ScrollDirectionStyle::Vertical,
        ) {
            for index in 0..*rows {
                list_row(index: *index)
            }
        }
    }
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
        let mut app = App::new(|_| {});
        app.mount::<Dashboard>(DashboardProps { rows });
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
