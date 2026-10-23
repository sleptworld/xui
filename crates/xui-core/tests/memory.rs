//! How much heap a mounted tree holds, per row and per host, and whether it
//! stays put while the tree updates.
//!
//! A benchmark, not a gate:
//!
//! ```text
//! cargo test -p xui-core --release --test memory -- --ignored --nocapture
//! ```
//!
//! This test binary counts every allocation through its own global
//! allocator, so the numbers are the runtime's live heap: hosts, columns,
//! layout, style, scene, fibers, hooks and the text cache, with text on
//! xui-f, the backend the native hosts use. Fonts are loaded once and
//! reported apart.
//!
//! `heap_attribution` mounts the same rows with and without text, and with
//! and without components, and breaks the heap down by allocation size;
//! `XUI_TRACE_ALLOC=<bytes>` also attributes every allocation of at least
//! that size to its call site (allocated, not net of frees).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use xui_core::prelude::*;
use xui_core::render::backend::MockRenderBackend;
use xui_core::state::State;
use xui_core::text::TextHost;
use xui_f::FBackend;
use xui_interface::{Color, Size};
use xui_macros::{component, xui};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
/// Allocation calls, reallocations included.
static CALLS: AtomicUsize = AtomicUsize::new(0);
/// Live bytes by allocation size: bucket `i` holds allocations of
/// `2^(i-1)..2^i` bytes.
static BY_SIZE: [AtomicUsize; 40] = [const { AtomicUsize::new(0) }; 40];

fn bucket(size: usize) -> usize {
    (usize::BITS - size.leading_zeros()) as usize
}

fn track(size: usize, add: bool) {
    let (total, by_size) = (&LIVE, &BY_SIZE[bucket(size).min(39)]);
    if add {
        CALLS.fetch_add(1, Ordering::Relaxed);
        total.fetch_add(size, Ordering::Relaxed);
        by_size.fetch_add(size, Ordering::Relaxed);
    } else {
        total.fetch_sub(size, Ordering::Relaxed);
        by_size.fetch_sub(size, Ordering::Relaxed);
    }
}

thread_local! {
    static TRACING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static TRACE_ABOVE: AtomicUsize = AtomicUsize::new(usize::MAX);
static TRACED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Bytes allocated (not net of frees) by call site, while tracing.
    static SITES: std::cell::RefCell<std::collections::HashMap<String, (usize, usize)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Prints where an allocation of `size` bytes comes from, when it is at least
/// [`TRACE_ABOVE`]: the runtime's frames only.
fn trace(size: usize) {
    if size < TRACE_ABOVE.load(Ordering::Relaxed) || TRACING.with(|t| t.replace(true)) {
        return;
    }
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let frames: Vec<String> = backtrace
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("at "))
        .map(|line| line.split_once(": ").map_or(line, |(_, name)| name))
        .filter(|name| {
            const SKIP: [&str; 12] = [
                "alloc::", "<alloc::", "core::", "<core::", "std::", "<std::", "__rustc",
                "hashbrown", "<hashbrown", "memory::", "backtrace", "smallvec",
            ];
            !SKIP.iter().any(|skip| name.starts_with(skip))
        })
        .take(3)
        .map(str::to_string)
        .collect();
    TRACED.fetch_add(1, Ordering::Relaxed);
    let site = frames.join(" <- ");
    SITES.with(|sites| {
        let mut sites = sites.borrow_mut();
        let entry = sites.entry(site).or_insert((0, 0));
        entry.0 += size;
        entry.1 += 1;
    });
    TRACING.with(|t| t.set(false));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        track(layout.size(), true);
        trace(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        track(layout.size(), false);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        track(new_size, true);
        trace(new_size);
        track(layout.size(), false);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn by_size() -> [usize; 40] {
    std::array::from_fn(|index| BY_SIZE[index].load(Ordering::Relaxed))
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

thread_local! {
    static ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ROOT: std::cell::RefCell<Option<State<u32>>> = const { std::cell::RefCell::new(None) };
}

/// One list row: three hosts, one state hook.
#[component]
fn memory_row(index: &usize, generation: &u32) {
    let wide = cx.use_state(|| false);
    let width = if *wide.get() { 24.0 } else { 16.0 };
    xui! {
        <row height={20.0} gap={4.0} background={Color::BLACK}>
            <text>{format!("row {index} / {generation}")}</text>
            <container width={width} height={16.0} background={Color::WHITE} />
        </row>
    }
}

fn root(cx: &mut HookContext<'_>) -> ElementDesc {
    let generation = cx.use_state(|| 0u32);
    ROOT.with(|slot| *slot.borrow_mut() = Some(generation));
    let generation = *generation.get();
    let rows = (0..ROWS.with(std::cell::Cell::get))
        .map(|index| {
            xui! { <memory_row key={index.to_string()} index={index} generation={generation} /> }
        })
        .collect::<Vec<_>>();
    xui! { <column width={900.0}>{rows}</column> }
}

/// The text backend the native hosts run on, over the platform's fonts, so
/// that text is shaped and its layouts cached as an application's would be.
fn text_backend() -> FBackend {
    FBackend::new()
}

fn kib(bytes: usize) -> f64 {
    bytes as f64 / 1024.0
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn heap_per_row() {
    let before_fonts = live();
    let mut text = TextHost::new(text_backend());
    println!("fonts: {:.1} KiB", kib(live() - before_fonts));

    println!("rows,hosts,mounted_kib,bytes_per_row,bytes_per_host,after_updates_kib");
    for rows in [100usize, 400, 1600, 6400] {
        ROWS.with(|cell| cell.set(rows));
        let baseline = live();
        let mut app = App::new(root);
        app.resize(Size::new(900.0, 700.0));
        let mut backend = MockRenderBackend::default();
        for _ in 0..3 {
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        let mounted = live() - baseline;
        let stats = text.stats();
        println!(
            "  text cache at mount: {} units, {} layouts, {:.1} KiB estimated layout bytes",
            stats.units,
            stats.layouts,
            kib(stats.resident_bytes as usize)
        );

        // Every row renders again with a new label, fifty times over: what is
        // held afterwards should be what was held before.
        for generation in 1..=50 {
            let state = ROOT.with(|slot| slot.borrow().expect("rendered"));
            slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(generation));
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        let after = live() - baseline;

        let hosts = rows * 2 + 1;
        println!(
            "{rows},{hosts},{:.1},{},{},{:.1}",
            kib(mounted),
            mounted / rows,
            mounted / hosts,
            kib(after),
        );
        drop(app);
    }
}

thread_local! {
    static MODE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

#[component]
fn bare_row(index: &usize) {
    let _ = index;
    xui! {
        <row height={20.0} gap={4.0} background={Color::BLACK}>
            <container width={16.0} height={16.0} background={Color::WHITE} />
            <container width={16.0} height={16.0} background={Color::WHITE} />
        </row>
    }
}

fn variant_root(_: &mut HookContext<'_>) -> ElementDesc {
    let rows = (0..ROWS.with(std::cell::Cell::get))
        .map(|index| match MODE.with(std::cell::Cell::get) {
            // Components, no text.
            0 => xui! { <bare_row key={index.to_string()} index={index} /> },
            // Hosts alone, no component.
            _ => xui! {
                <row height={20.0} gap={4.0} background={Color::BLACK}>
                    <container width={16.0} height={16.0} background={Color::WHITE} />
                    <container width={16.0} height={16.0} background={Color::WHITE} />
                </row>
            },
        })
        .collect::<Vec<_>>();
    xui! { <column width={900.0}>{rows}</column> }
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn heap_attribution() {
    let mut text = TextHost::new(text_backend());
    let rows = 1600;
    ROWS.with(|cell| cell.set(rows));
    // `XUI_TRACE_ALLOC=<bytes>`: every allocation of at least that size is
    // attributed to its call site, and the largest sites are printed.
    let trace_above = std::env::var("XUI_TRACE_ALLOC")
        .ok()
        .and_then(|bytes| bytes.parse().ok());
    let mut measure = |label: &str, root: fn(&mut HookContext<'_>) -> ElementDesc| {
        let baseline = live();
        let sizes_before = by_size();
        if let Some(bytes) = trace_above {
            TRACE_ABOVE.store(bytes, Ordering::Relaxed);
        }
        let mut app = App::new(root);
        app.resize(Size::new(900.0, 700.0));
        let mut backend = MockRenderBackend::default();
        for _ in 0..3 {
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        TRACE_ABOVE.store(usize::MAX, Ordering::Relaxed);
        let mounted = live() - baseline;
        let sizes = by_size();
        let classes: Vec<String> = (0..40)
            .filter_map(|index| {
                let bytes = sizes[index].saturating_sub(sizes_before[index]);
                (bytes > 64 * 1024).then(|| format!("<{}B:{:.0}K", 1usize << index, kib(bytes)))
            })
            .collect();
        println!(
            "{label}: {:.1} KiB, {} B/row  [{}]",
            kib(mounted),
            mounted / rows,
            classes.join(" ")
        );
        TRACING.with(|t| t.set(true));
        let mut sites: Vec<_> = SITES.with(|sites| sites.borrow_mut().drain().collect());
        sites.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        for (site, (bytes, _)) in sites.iter().take(15) {
            println!("  {:>8.0} KiB  {site}", kib(*bytes));
        }
        TRACING.with(|t| t.set(false));
        drop(app);
    };
    measure("full rows (component, text, state)", root);
    MODE.with(|mode| mode.set(0));
    measure("component rows, no text", variant_root);
    MODE.with(|mode| mode.set(1));
    measure("host rows, no component", variant_root);
}

/// What one render of every row allocates: a root state change hands every
/// row new props, so each row's component and template are built again.
#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn allocations_per_render() {
    let mut text = TextHost::new(text_backend());
    let rows = 1600;
    ROWS.with(|cell| cell.set(rows));
    let mut app = App::new(root);
    app.resize(Size::new(900.0, 700.0));
    let mut backend = MockRenderBackend::default();
    for _ in 0..3 {
        app.render(&mut backend, &mut text).expect("mock backend");
    }
    let mut renders = Vec::new();
    for generation in 1..=10 {
        let state = ROOT.with(|slot| slot.borrow().expect("rendered"));
        slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(generation));
        let calls = CALLS.load(Ordering::Relaxed);
        app.render(&mut backend, &mut text).expect("mock backend");
        while app.is_dirty() {
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        renders.push(CALLS.load(Ordering::Relaxed) - calls);
    }
    renders.sort_unstable();
    let calls = renders[renders.len() / 2];
    println!("allocations per render: {calls}, per row: {:.1}", calls as f64 / rows as f64);

    // `XUI_TRACE_ALLOC=1`: one more render, every allocation attributed.
    if std::env::var("XUI_TRACE_ALLOC").is_ok() {
        let state = ROOT.with(|slot| slot.borrow().expect("rendered"));
        slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(100));
        TRACE_ABOVE.store(1, Ordering::Relaxed);
        // Slower than a frame's budget while tracing: the render is sliced.
        app.render(&mut backend, &mut text).expect("mock backend");
        while app.is_dirty() {
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        TRACE_ABOVE.store(usize::MAX, Ordering::Relaxed);
        TRACING.with(|t| t.set(true));
        println!("traced: {}", TRACED.load(Ordering::Relaxed));
        let mut sites: Vec<_> = SITES.with(|sites| sites.borrow_mut().drain().collect());
        sites.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        for (site, (_, count)) in sites.iter().take(30) {
            println!("  {:>6.1}/row  {site}", *count as f64 / rows as f64);
        }
        TRACING.with(|t| t.set(false));
    }
}

/// One frame of the same render, timed: every row's label changes, so every
/// paragraph is shaped and laid out again on the real backend.
#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn frame_time_with_real_text() {
    let mut text = TextHost::new(text_backend());
    println!("rows,frame_ms");
    for rows in [100usize, 400, 1600] {
        ROWS.with(|cell| cell.set(rows));
        let mut app = App::new(root);
        app.resize(Size::new(900.0, 700.0));
        let mut backend = MockRenderBackend::default();
        for _ in 0..3 {
            app.render(&mut backend, &mut text).expect("mock backend");
        }
        let mut samples = Vec::new();
        for generation in 1..=30 {
            let state = ROOT.with(|slot| slot.borrow().expect("rendered"));
            slot::Runtime::with_phase(slot::RenderPhase::Event, || state.set(generation));
            let started = std::time::Instant::now();
            app.render(&mut backend, &mut text).expect("mock backend");
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        println!("{rows},{:.3}", samples[samples.len() / 2].as_secs_f64() * 1000.0);
    }
}
