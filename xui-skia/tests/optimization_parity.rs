//! The frame-drawing fast paths must not change the picture.
//!
//! `SkiaBackend` skips items outside the repainted region, draws plainly
//! composited layers straight into their parent, and pools offscreen surfaces.
//! Each of those is only safe if the frame it produces is the frame the
//! reference path would have produced, so every case here renders the same
//! scene twice and compares pixels:
//!
//! - `SkiaOptimizations::default()` against `SkiaOptimizations::NONE`, which
//!   catches a fast path that draws the wrong thing;
//! - an incrementally-repainted backend against one invalidated every frame,
//!   which catches damage that fails to cover a change — the assumption every
//!   cull rests on.
//!
//! Run with `cargo test -p xui-skia --test optimization_parity`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use xui::prelude::*;
use xui::text::TextHost;
use xui_interface::Translation;
use xui_interface::events::{
    Modifiers, PointerButtons, PointerKind, RawEvent, RawPointerMove, RawWheel, ScrollDelta,
    XuiPointerId,
};
use xui_skia::{SkiaBackend, SkiaBackendOptions, SkiaFrameStats, SkiaOptimizations};
use xui_text_engine::CosmicEngine;

const VIEWPORT: Size<f32> = Size {
    width: 480.0,
    height: 360.0,
};
const ROWS: usize = 40;

static SCROLL: AtomicUsize = AtomicUsize::new(0);

// --------------------------------------------------------------- scenes --

/// Exercises every branch the fast paths care about: plain nested containers
/// (inlinable), a rounded clip, a transformed subtree, a blurred layer and a
/// backdrop layer (neither inlinable), a gradient, text, and a scroll container
/// whose rows run well past the viewport (cullable).
fn scene(_cx: &mut HookContext<'_>) -> ElementDesc {
    let rows: Vec<ElementDesc> = (0..ROWS).map(row).collect();
    container()
        .style(
            Style::new()
                .size(Size::fill())
                .background(Color::hex("#0a0a0a"))
                .clip(true),
        )
        .into_element_desc(vec![
            container()
                .style(
                    Style::new()
                        .size(Size::fill())
                        .scroll_vertical()
                        .clip(true)
                        .padding(EdgeInsets::all(8.0)),
                )
                .into_element_desc(rows),
            // A backdrop layer over the scrolling content: reads the
            // destination, so it must never be inlined or culled away.
            container()
                .style(
                    Style::new()
                        .absolute()
                        .inset(EdgeInsets::new(40.0, 0.0, 0.0, 24.0))
                        .width(200.0)
                        .height(80.0)
                        .border_radius(16.0)
                        .background(Color::rgba(1.0, 1.0, 1.0, 0.10))
                        .backdrop_blur(12.0)
                        .clip(true),
                )
                .into_element_desc(vec![
                    TextWidget::new("backdrop")
                        .style(Style::new().width(160.0).height(20.0))
                        .into_element_desc(),
                ]),
        ])
}

fn row(index: usize) -> ElementDesc {
    let mut style = Style::new()
        .width(Sizing::fill())
        .height(30.0)
        .padding(EdgeInsets::all(4.0))
        .gap(6.0)
        .background(Color::hex("#141414"))
        .border_radius(6.0)
        .clip(true)
        // Hovering has to change something, or the frames after a pointer move
        // are identical and the damage tracker rightly repaints nothing.
        .when(WidgetState::HOVERED, |patch| {
            patch.background(Color::hex("#2a2a2a"))
        });

    // Sprinkle the non-inlinable shapes through the list so both paths run in
    // the same frame.
    style = match index % 5 {
        0 => style.background(ColorStyle::linear_gradient(
            Point::new(0.0, 0.0),
            Point::new(1.0, 1.0),
            Color::hex("#1d4ed8"),
            Color::hex("#9333ea"),
        )),
        1 => style.effects(vec![Effect::blur(2.0)]),
        2 => style.translate_x(6.0),
        3 => style.box_shadow(
            Color::rgba(0.0, 0.0, 0.0, 0.5),
            Point::new(2.0, 2.0),
            4.0,
            0.0,
        ),
        _ => style,
    };

    container().style(style).into_element_desc(vec![
        TextWidget::new(if index % 3 == 0 { "row label" } else { "item" })
            .style(Style::new().width(120.0).height(20.0))
            .into_element_desc(),
        container()
            .style(
                Style::new()
                    .width(16.0)
                    .height(16.0)
                    .border_radius(8.0)
                    .background(Color::hex("#3b82f6")),
            )
            .into_element_desc(Vec::new()),
    ])
}

// ---------------------------------------------------------------- driver --

struct Harness {
    app: App,
    text: TextHost<CosmicEngine>,
    backend: SkiaBackend<CosmicEngine>,
}

impl Harness {
    fn new(optimizations: SkiaOptimizations) -> Self {
        let mut app = App::new(scene);
        app.resize(VIEWPORT);
        Self {
            app,
            text: TextHost::new(CosmicEngine::new(1.0)),
            backend: SkiaBackend::<CosmicEngine>::headless(
                1.0,
                SkiaBackendOptions {
                    optimizations,
                    ..SkiaBackendOptions::default()
                },
            ),
        }
    }

    fn render(&mut self) {
        self.app
            .render(&mut self.backend, &mut self.text)
            .expect("frame renders");
    }

    fn event(&mut self, event: RawEvent) {
        self.app.dispatch_event(event, &mut self.text);
    }

    fn pixels(&mut self) -> Vec<u8> {
        self.backend.read_pixels_rgba8().expect("pixels read back")
    }

    fn stats(&self) -> SkiaFrameStats {
        self.backend.frame_stats()
    }
}

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

fn scroll(delta: f32) -> RawEvent {
    RawEvent::Wheel(RawWheel {
        position: Point::new(200.0, 180.0),
        delta: ScrollDelta::Pixels(Translation::new(0.0, delta)),
        pointer_id: Some(XuiPointerId::new(0)),
        device_id: None,
        modifiers: Modifiers::default(),
        timestamp: Instant::now(),
        is_inertial: false,
    })
}

/// The interaction script both sides of a comparison replay.
///
/// Hover, scroll and a settled animation between them, so the frames under
/// comparison have gone through partial repaints rather than one clean paint.
fn drive(harness: &mut Harness, invalidate_each_frame: bool) {
    for step in 0..12 {
        if invalidate_each_frame {
            harness.backend.invalidate();
        }
        match step {
            2 => harness.event(pointer_move(Point::new(400.0, 120.0))),
            5 => harness.event(scroll(-90.0)),
            8 => harness.event(pointer_move(Point::new(240.0, 200.0))),
            9 => harness.event(scroll(-30.0)),
            _ => {}
        }
        harness.app.tick_style_animations(Duration::from_millis(16));
        harness.render();
    }
    // Let any transition finish so the final frame is not a moving target.
    for _ in 0..40 {
        if invalidate_each_frame {
            harness.backend.invalidate();
        }
        harness.app.tick_style_animations(Duration::from_millis(50));
        harness.render();
    }
}

// ------------------------------------------------------------ comparison --

struct Diff {
    differing: usize,
    total: usize,
    max_channel: u8,
}

fn compare(a: &[u8], b: &[u8]) -> Diff {
    assert_eq!(a.len(), b.len(), "frames differ in size");
    let mut differing = 0;
    let mut max_channel = 0;
    for (left, right) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let delta = left
            .iter()
            .zip(right)
            .map(|(l, r)| l.abs_diff(*r))
            .max()
            .unwrap_or(0);
        if delta > 0 {
            differing += 1;
            max_channel = max_channel.max(delta);
        }
    }
    Diff {
        differing,
        total: a.len() / 4,
        max_channel,
    }
}

fn assert_identical(a: &[u8], b: &[u8], what: &str) {
    let diff = compare(a, b);
    assert_eq!(
        diff.differing, 0,
        "{what}: {} of {} pixels differ, largest channel delta {}",
        diff.differing, diff.total, diff.max_channel
    );
}

// ----------------------------------------------------------------- tests --

#[test]
fn fast_paths_render_the_reference_frame() {
    SCROLL.store(0, Ordering::Relaxed);
    let mut fast = Harness::new(SkiaOptimizations::default());
    let mut reference = Harness::new(SkiaOptimizations::NONE);
    drive(&mut fast, false);
    drive(&mut reference, false);
    assert_identical(&fast.pixels(), &reference.pixels(), "fast vs reference");
}

#[test]
fn incremental_repaints_match_full_repaints() {
    let mut incremental = Harness::new(SkiaOptimizations::default());
    let mut full = Harness::new(SkiaOptimizations::default());
    drive(&mut incremental, false);
    drive(&mut full, true);
    assert_identical(
        &incremental.pixels(),
        &full.pixels(),
        "incremental vs full repaint",
    );
}

/// A partial repaint is where culling pays: the frame builder already drops
/// what falls outside the viewport, so the items a cull removes are the ones
/// inside the viewport but outside the damaged region.
#[test]
fn a_partial_repaint_culls_the_untouched_items() {
    let mut fast = Harness::new(SkiaOptimizations::default());
    let mut reference = Harness::new(SkiaOptimizations::NONE);
    for harness in [&mut fast, &mut reference] {
        // Settle, so the next frame repaints only what the hover changed.
        for _ in 0..3 {
            harness.render();
        }
        harness.event(pointer_move(Point::new(400.0, 300.0)));
        harness.render();
    }
    let (fast_stats, reference_stats) = (fast.stats(), reference.stats());
    assert!(
        fast_stats.root_damage_area_sum < VIEWPORT.width * VIEWPORT.height,
        "the hover repainted the whole viewport, so this measures nothing: {fast_stats:?}"
    );
    assert!(
        fast_stats.items_culled > 0,
        "nothing was culled on a partial repaint: {fast_stats:?}"
    );
    assert!(
        fast_stats.primitive_draws < reference_stats.primitive_draws,
        "culling drew as many primitives as the reference: {fast_stats:?} vs {reference_stats:?}"
    );
    assert_identical(
        &fast.pixels(),
        &reference.pixels(),
        "culled partial repaint vs reference",
    );
}

/// Offscreen surfaces should come back from the pool on a steady-state frame
/// rather than being allocated again.
#[test]
fn steady_state_frames_reuse_pooled_surfaces() {
    let mut fast = Harness::new(SkiaOptimizations::default());
    drive(&mut fast, false);
    fast.app.mark_needs_rebuild();
    fast.render();
    let stats = fast.stats();
    assert!(
        stats.pooled_surface_reuses > 0,
        "no offscreen surface came from the pool: {stats:?}"
    );
    assert!(
        stats.offscreen_surface_allocations < stats.pooled_surface_reuses,
        "the pool served fewer surfaces than it allocated: {stats:?}"
    );
}

/// The scene has to actually exercise the layer machinery, or the parity tests
/// above are comparing two trivial frames.
#[test]
fn the_scene_exercises_layers_and_backdrops() {
    let mut fast = Harness::new(SkiaOptimizations::default());
    fast.render();
    fast.app.mark_needs_rebuild();
    fast.render();
    let stats = fast.stats();
    assert!(
        stats.render_plans > 0 && stats.render_passes > stats.render_plans,
        "no multi-pass layer program ran: {stats:?}"
    );
    assert!(
        stats.backdrop_materializations > 0,
        "the backdrop layer did not materialize: {stats:?}"
    );
}
