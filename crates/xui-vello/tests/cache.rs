//! What the backend holds on to between frames: images survive being off
//! screen, caches stay inside their budgets, and filter textures do not pile up
//! as a window is resized.
//!
//! Frames are drawn one after another into a single backend, each from a fresh
//! `App`, which is how a scene changes under a cache without any input.

mod common;

use std::sync::OnceLock;

use common::*;
use xui_core::{
    app::ComponentFn,
    prelude::*,
    text::TextHost,
    widgets::{CanvasController, CanvasWidget, ContainerWidget, ImageWidget},
};
use xui_cosmic::CosmicEngine;
use xui_interface::{
    Effect, ImageData, PathBuilder, PathFill, Point, Sampling, Sizing,
};
use xui_vello::{ResourceStats, VelloBackend, VelloBackendOptions};

fn backend(options: VelloBackendOptions) -> Option<VelloBackend<CosmicEngine>> {
    match VelloBackend::headless(1.0, options) {
        Ok(backend) => Some(backend),
        Err(error) => {
            eprintln!("skipping: no wgpu adapter ({error})");
            None
        }
    }
}

/// One frame of `root`, `size` logical pixels square, into `backend`.
fn draw(backend: &mut VelloBackend<CosmicEngine>, root: ComponentFn, size: f32) -> ResourceStats {
    let mut app = App::new(root);
    app.resize(Size::new(size, size));
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    app.render(backend, &mut text).expect("render");
    backend.resource_stats()
}

/// 32x32 opaque pixels, `shade` apart so each is a distinct image.
fn picture(shade: u8) -> ImageData {
    ImageData::rgba8(Size::new(32, 32), vec![shade; 32 * 32 * 4])
}

fn shared(slot: &'static OnceLock<ImageData>, shade: u8) -> ImageData {
    slot.get_or_init(|| picture(shade)).clone()
}

fn image_widget(key: &str, data: ImageData) -> ElementDesc {
    ImageWidget::new()
        .image_data(key, data)
        .sampling(Sampling::Nearest)
        .style(Style::new().width(32.0).height(32.0))
        .into_element_desc(vec![])
}

static A: OnceLock<ImageData> = OnceLock::new();
static B: OnceLock<ImageData> = OnceLock::new();
static C: OnceLock<ImageData> = OnceLock::new();

fn show_a(_: &mut HookContext<'_>) -> ElementDesc {
    image_widget("a", shared(&A, 10))
}
fn show_b(_: &mut HookContext<'_>) -> ElementDesc {
    image_widget("b", shared(&B, 20))
}
fn show_c(_: &mut HookContext<'_>) -> ElementDesc {
    image_widget("c", shared(&C, 30))
}
fn show_nothing(_: &mut HookContext<'_>) -> ElementDesc {
    ContainerWidget::new()
        .style(Style::new().width(10.0).height(10.0))
        .into_element_desc(vec![])
}

#[test]
fn an_image_keeps_its_upload_while_it_is_off_screen() {
    let Some(mut backend) = backend(VelloBackendOptions::default()) else {
        return;
    };
    let first = draw(&mut backend, show_a, 64.0);
    assert_eq!((first.image_entries, first.image_builds), (1, 1));

    // Gone for several frames: the old per-frame sweep dropped it here.
    for _ in 0..6 {
        draw(&mut backend, show_nothing, 64.0);
    }
    let back = draw(&mut backend, show_a, 64.0);
    assert_eq!(back.image_builds, 1, "the image was rebuilt after it came back");
    assert_eq!(back.image_entries, 1);
}

#[test]
fn drawing_an_image_again_does_not_rebuild_it() {
    let Some(mut backend) = backend(VelloBackendOptions::default()) else {
        return;
    };
    for _ in 0..5 {
        draw(&mut backend, show_a, 64.0);
    }
    assert_eq!(backend.resource_stats().image_builds, 1);
}

#[test]
fn the_image_cache_stays_inside_its_budget() {
    // Two 4 KiB images fit; a third cannot.
    let Some(mut backend) = backend(VelloBackendOptions {
        image_cache_budget_bytes: 8 * 1024,
        ..VelloBackendOptions::default()
    }) else {
        return;
    };
    for root in [show_a, show_b, show_c, show_a, show_b, show_c] {
        let stats = draw(&mut backend, root, 64.0);
        assert!(
            stats.image_bytes <= 8 * 1024,
            "image cache holds {} bytes over a budget of 8192",
            stats.image_bytes
        );
    }
    assert!(backend.resource_stats().image_entries <= 2);
}

fn blurred_layer(_: &mut HookContext<'_>) -> ElementDesc {
    // A layer as large as the window, so its filter textures track the window.
    ContainerWidget::new()
        .style(
            Style::new()
                .width(Sizing::Fill)
                .height(Sizing::Fill)
                .background(Color::rgb(1.0, 0.0, 0.0))
                .effects(vec![Effect::blur(2.0)]),
        )
        .into_element_desc(vec![])
}

#[test]
fn filter_textures_are_released_once_nothing_uses_them() {
    let Some(mut backend) = backend(VelloBackendOptions::default()) else {
        return;
    };
    let busy = draw(&mut backend, blurred_layer, 64.0);
    assert!(busy.filter_textures > 0, "the layer made no filter textures");

    // A pooled texture idle for a few frames goes.
    for _ in 0..12 {
        draw(&mut backend, show_nothing, 64.0);
    }
    let idle = backend.resource_stats();
    assert_eq!(idle.filter_textures, 0, "{idle:?}");
    assert_eq!(idle.filter_texture_bytes, 0);
}

#[test]
fn resizing_does_not_pile_up_filter_textures() {
    let Some(mut backend) = backend(VelloBackendOptions::default()) else {
        return;
    };
    // The textures one size of the layer needs, once the pool has settled.
    for _ in 0..3 {
        draw(&mut backend, blurred_layer, 60.0);
    }
    let per_size = backend.resource_stats().filter_textures;
    assert!(per_size > 0);

    // Thirty different window sizes, one frame each: every one needs textures
    // of a size no other frame used. Keeping them all would come to thirty sets.
    for step in 0..30 {
        draw(&mut backend, blurred_layer, 63.0 + step as f32 * 3.0);
    }
    let settled = backend.resource_stats();
    eprintln!("per size: {per_size}; after 30 sizes: {settled:?}");

    // Only sizes seen within the idle window (eight frames) can still be held.
    assert!(
        settled.filter_textures <= per_size * 9,
        "{} textures after 30 sizes, {per_size} per size ({settled:?})",
        settled.filter_textures
    );
    assert!(settled.filter_textures < per_size * 30);
}

#[test]
fn the_filter_pool_stays_inside_its_byte_budget() {
    let Some(mut backend) = backend(VelloBackendOptions {
        filter_texture_budget_bytes: 40 * 1024,
        ..VelloBackendOptions::default()
    }) else {
        return;
    };
    // One 64x64 texture is 16 KiB; a frame's worth is more than the budget.
    draw(&mut backend, blurred_layer, 64.0);
    // The budget is applied as the next frame starts.
    let after = draw(&mut backend, blurred_layer, 64.0);
    assert!(
        after.filter_texture_bytes <= 4 * 16 * 1024,
        "{} bytes pooled",
        after.filter_texture_bytes
    );
    let trimmed = {
        let mut backend = backend;
        draw(&mut backend, show_nothing, 64.0);
        backend.resource_stats()
    };
    assert!(trimmed.filter_texture_bytes <= 40 * 1024, "{trimmed:?}");
}

fn path_root(_: &mut HookContext<'_>) -> ElementDesc {
    static PATH: OnceLock<xui_interface::PathData> = OnceLock::new();
    let path = PATH
        .get_or_init(|| {
            let mut builder = PathBuilder::new();
            builder
                .move_to(Point::new(0.0, 0.0))
                .line_to(Point::new(40.0, 0.0))
                .line_to(Point::new(20.0, 30.0))
                .close();
            builder.build()
        })
        .clone();
    let controller = CanvasController::with_painter(move |painter| {
        painter.fill_path(path.clone(), PathFill::new(Color::rgb(0.0, 0.0, 1.0)));
    });
    CanvasWidget::new(controller)
        .style(Style::new().width(64.0).height(64.0))
        .into_element_desc()
}

#[test]
fn a_path_is_converted_once() {
    let Some(mut backend) = backend(VelloBackendOptions::default()) else {
        return;
    };
    for _ in 0..5 {
        draw(&mut backend, path_root, 64.0);
    }
    let stats = backend.resource_stats();
    assert_eq!(stats.path_entries, 1, "{stats:?}");
    assert_eq!(stats.path_segments, 4, "{stats:?}");
}

fn some_text(_: &mut HookContext<'_>) -> ElementDesc {
    xui_core::widgets::TextWidget::new("cache")
        .style(Style::new().width(64.0).height(32.0))
        .into_element_desc()
}

#[test]
fn a_font_is_loaded_once_and_counted() {
    // Fonts are loaded for Vello's outlines; bitmap text needs none of them.
    let Some(mut backend) = backend(VelloBackendOptions {
        bitmap_text: false,
        ..VelloBackendOptions::default()
    }) else {
        return;
    };
    let first = draw(&mut backend, some_text, 64.0);
    assert!(first.font_entries >= 1 && first.font_bytes > 0, "{first:?}");
    for _ in 0..4 {
        draw(&mut backend, some_text, 64.0);
    }
    let again = backend.resource_stats();
    assert_eq!(again.font_entries, first.font_entries);
    assert_eq!(again.font_bytes, first.font_bytes);
}
