//! Repainting only what changed.
//!
//! The one rule that matters: drawing a frame as a patch over the previous one
//! must give exactly the pixels drawing it from scratch gives. Each scene here
//! changes a small thing between two frames, then the result is compared with a
//! fresh backend that only ever saw the final state.

mod common;

use common::*;
use xui_core::{prelude::*, text::TextHost, widgets::CanvasController};
use xui_cosmic::CosmicEngine;
use xui_interface::{Effect, FlexDirectionStyle, PositionStyle};
use xui_macros::component;
use xui_vello::{FrameStats, VelloBackend, VelloBackendOptions};

/// Mounts a scene that draws version `step` of itself.
type Scene = fn(&mut App, &Signal<u32>);

/// An app showing version `step` of `scene`, and the signal that picks it.
fn app_at(scene: Scene, step: u32) -> (App, Signal<u32>) {
    let mut app = App::new(|_| {});
    let step = app.signal(step);
    scene(&mut app, &step);
    app.resize(Size::new(SIZE, SIZE));
    (app, step)
}

fn backend() -> Option<VelloBackend<CosmicEngine>> {
    match VelloBackend::headless(1.0, VelloBackendOptions::default()) {
        Ok(backend) => Some(backend),
        Err(error) => {
            eprintln!("skipping: no wgpu adapter ({error})");
            None
        }
    }
}

fn render_once(
    backend: &mut VelloBackend<CosmicEngine>,
    app: &mut App,
    text: &mut TextHost<CosmicEngine>,
) {
    app.render(backend, text).expect("render");
}

/// Frame one at step 0, frame two at step 1, on one backend; and the same final
/// state on a backend that never saw step 0.
///
/// Returns the patched pixels, the from-scratch pixels, and the stats after the
/// second frame.
fn patched_and_fresh(scene: Scene) -> Option<(Vec<u8>, Vec<u8>, FrameStats)> {
    let mut patched = backend()?;
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, step) = app_at(scene, 0);
    render_once(&mut patched, &mut app, &mut text);
    app.update(|cx| step.set(cx, 1));
    render_once(&mut patched, &mut app, &mut text);
    let stats = patched.frame_stats();
    let patched_pixels = patched.read_pixels_rgba8().expect("pixels");

    let mut fresh = backend()?;
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, _) = app_at(scene, 1);
    render_once(&mut fresh, &mut app, &mut text);
    let fresh_pixels = fresh.read_pixels_rgba8().expect("pixels");
    Some((patched_pixels, fresh_pixels, stats))
}

fn assert_same(patched: &[u8], fresh: &[u8]) {
    assert_eq!(patched.len(), fresh.len());
    let differing: Vec<usize> = patched
        .chunks_exact(4)
        .zip(fresh.chunks_exact(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    assert!(
        differing.is_empty(),
        "{} pixels differ from a fresh render, first at ({}, {})",
        differing.len(),
        differing[0] % SIZE as usize,
        differing[0] / SIZE as usize
    );
}

const FULL: u64 = (SIZE as u64) * (SIZE as u64);

/// Dock previews cover text and move without rebuilding the panes underneath.
fn dock_preview(app: &mut App, step: &Signal<u32>) {
    app.mount::<DockPreviewScene>(DockPreviewSceneProps { step: step.into(), moving: true });
}

#[component]
fn dock_preview_scene(#[source] step: &u32, moving: &bool) {
    view! {
        container(width: SIZE, height: SIZE, background: Color::rgb(0.08, 0.08, 0.1)) {
            text("Pane title", font_size: 14.0, color: Color::WHITE)
            if !*moving {
                container(
                    position: PositionStyle::Absolute,
                    inset: EdgeInsets::new(25.0, 0.0, 50.0, 0.0),
                    width: 10.0,
                    height: 10.0,
                    background: if *step == 0 { Color::WHITE } else { Color::BLACK },
                )
            }
            container(
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(if !*moving || *step == 0 { 10.0 } else { 45.0 }, 0.0, 8.0, 0.0),
                width: 40.0,
                height: 70.0,
                background: Color::rgba(0.1, 0.4, 1.0, 0.3),
            )
        }
    }
}

#[test]
fn moving_translucent_dock_preview_matches_fresh_render() {
    let Some((patched, fresh, _)) = patched_and_fresh(dock_preview) else {
        return;
    };
    assert_same(&patched, &fresh);
}

fn changing_under_dock_preview(app: &mut App, step: &Signal<u32>) {
    // Change a small item away from the title while the blue overlay stays put.
    // The overlay cuts the text band in a full frame; the patch culls the title.
    app.mount::<DockPreviewScene>(DockPreviewSceneProps { step: step.into(), moving: false });
}

#[test]
fn patch_under_translucent_dock_preview_matches_fresh_render() {
    let Some((patched, fresh, stats)) = patched_and_fresh(changing_under_dock_preview) else {
        return;
    };
    assert!(stats.last_pixels > 0 && stats.last_pixels < FULL / 4, "{stats:?}");
    // A separate band quantizes the translucent source to RGBA8 before blending;
    // a single Vello band quantizes after blending. Allow that one-code rounding.
    let max_delta = patched
        .iter()
        .zip(&fresh)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(max_delta <= 1, "largest channel difference: {max_delta}");
}

/// Blue at step 0, green after.
fn corner_color(step: u32) -> Color {
    if step == 0 { Color::rgb(0.0, 0.0, 1.0) } else { Color::rgb(0.0, 0.8, 0.0) }
}

/// A static red field with a 20x20 box in the corner that changes colour.
fn corner_box(app: &mut App, step: &Signal<u32>) {
    app.mount::<CornerBoxScene>(CornerBoxSceneProps { step: step.into() });
}

#[component]
fn corner_box_scene(#[source] step: &u32) {
    view! {
        container(width: SIZE, height: SIZE, background: Color::rgb(1.0, 0.0, 0.0)) {
            // Left alone.
            container(width: 30.0, height: 30.0, background: Color::rgb(0.9, 0.9, 0.0))
            container(
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(75.0, 0.0, 75.0, 0.0),
                width: 20.0,
                height: 20.0,
                background: corner_color(*step),
            )
        }
    }
}

#[test]
fn a_patch_gives_the_pixels_of_a_fresh_render() {
    let Some((patched, fresh, _)) = patched_and_fresh(corner_box) else {
        return;
    };
    assert_same(&patched, &fresh);
    // And the change really happened: the box is now green.
    let at = (85 * SIZE as usize + 85) * 4;
    assert_eq!(&patched[at..at + 4], &[0, 204, 0, 255]);
}

#[test]
fn a_small_change_draws_a_small_region() {
    let Some((_, _, stats)) = patched_and_fresh(corner_box) else {
        return;
    };
    assert_eq!(stats.frames, 2);
    assert!(stats.last_pixels > 0, "{stats:?}");
    // The 20x20 box plus a margin, not the 100x100 frame.
    assert!(stats.last_pixels < FULL / 4, "{stats:?}");
    // The first frame drew everything, so the total is one frame plus the patch.
    assert!(stats.total_pixels < FULL + FULL / 4, "{stats:?}");
}

#[test]
fn the_first_frame_is_drawn_in_full() {
    let Some(mut backend) = backend() else {
        return;
    };
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, _) = app_at(corner_box, 0);
    render_once(&mut backend, &mut app, &mut text);
    let stats = backend.frame_stats();
    assert_eq!(stats.last_pixels, FULL, "{stats:?}");
    assert_eq!(stats.last_region, [0, 0, SIZE as i32, SIZE as i32]);
}

/// A changing box over a blurred layer: the patch lands inside a filtered one.
fn over_a_blurred_layer(app: &mut App, step: &Signal<u32>) {
    app.mount::<OverABlurredLayerScene>(OverABlurredLayerSceneProps { step: step.into() });
}

#[component]
fn over_a_blurred_layer_scene(#[source] step: &u32) {
    view! {
        container(width: SIZE, height: SIZE) {
            container(
                width: 60.0,
                height: 60.0,
                background: Color::rgb(1.0, 0.0, 0.0),
                effects: vec![Effect::blur(3.0)],
            )
            container(
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(70.0, 0.0, 70.0, 0.0),
                width: 20.0,
                height: 20.0,
                background: corner_color(*step),
            )
        }
    }
}

#[test]
fn a_patch_beside_a_filtered_layer_matches() {
    let Some((patched, fresh, stats)) = patched_and_fresh(over_a_blurred_layer) else {
        return;
    };
    assert_same(&patched, &fresh);
    assert!(stats.last_pixels < FULL, "{stats:?}");
}

/// The seam between a red and a blue half is blurred inside a glass panel. The
/// box that changes sits *under* the panel, so the panel's result is stale.
fn under_glass(app: &mut App, step: &Signal<u32>) {
    app.mount::<UnderGlassScene>(UnderGlassSceneProps { step: step.into() });
}

#[component]
fn under_glass_scene(#[source] step: &u32) {
    let color: Color = memo!(if *step == 0 { Color::rgb(0.0, 0.0, 1.0) } else { Color::rgb(0.0, 1.0, 1.0) });
    view! {
        container(width: SIZE, height: SIZE, flex_direction: FlexDirectionStyle::Row) {
            container(width: 50.0, height: SIZE, background: Color::rgb(1.0, 0.0, 0.0))
            container(width: 50.0, height: SIZE, background: *color)
            container(
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(40.0, 0.0, 40.0, 0.0),
                width: 10.0,
                height: 10.0,
                background: *color,
            )
            container(
                width: 50.0,
                height: SIZE,
                position: PositionStyle::Absolute,
                inset: EdgeInsets::new(25.0, 0.0, 0.0, 0.0),
                backdrop_blur: 8.0,
            )
        }
    }
}

#[test]
fn a_change_under_glass_repaints_the_glass() {
    let Some((patched, fresh, stats)) = patched_and_fresh(under_glass) else {
        return;
    };
    assert_same(&patched, &fresh);
    // The panel (x 25..75) is repainted whole, not only where the change was.
    assert!(
        stats.last_region[0] <= 25 && stats.last_region[2] >= 75,
        "{stats:?}"
    );
}

/// Text that changes: its old and new extents are both damaged.
fn changing_text(app: &mut App, step: &Signal<u32>) {
    app.mount::<ChangingTextScene>(ChangingTextSceneProps { step: step.into() });
}

#[component]
fn changing_text_scene(#[source] step: &u32) {
    view! {
        container(width: SIZE, height: SIZE) {
            text(if *step == 0 { "one" } else { "three" }, width: SIZE, height: 20.0, font_size: 14.0)
        }
    }
}

#[test]
fn changed_text_leaves_no_trace_of_the_old() {
    let Some((patched, fresh, stats)) = patched_and_fresh(changing_text) else {
        return;
    };
    assert_same(&patched, &fresh);
    assert!(stats.last_pixels < FULL, "{stats:?}");
}

#[test]
fn a_second_identical_frame_draws_nothing() {
    let Some(mut backend) = backend() else {
        return;
    };
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, step) = app_at(corner_box, 0);
    render_once(&mut backend, &mut app, &mut text);
    let before = backend.read_pixels_rgba8().expect("pixels");
    // Writing the value it already has changes nothing, so no region is damaged.
    app.update(|cx| step.set(cx, 0));
    app.mark_needs_rebuild();
    render_once(&mut backend, &mut app, &mut text);
    let stats = backend.frame_stats();
    assert_eq!(backend.read_pixels_rgba8().expect("pixels"), before);
    assert!(stats.last_pixels == 0 || stats.frames == 1, "{stats:?}");
}

const TEXTURE: xui_interface::ExternalTextureId = xui_interface::ExternalTextureId(3);

/// A red field with a registered texture over its left half. The texture's
/// revision follows the step, as a canvas that redrew into it would bump it.
fn textured(app: &mut App, step: &Signal<u32>) {
    app.mount::<TexturedScene>(TexturedSceneProps { step: step.into() });
}

fn texture_painter(step: u32) -> CanvasController {
    let revision = step as u64 + 1;
    CanvasController::with_painter(move |painter| {
        painter.rect(
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            Color::rgb(1.0, 0.0, 0.0),
        );
        painter.texture(
            TEXTURE,
            revision,
            xui_interface::Bounds::from_origin_size((10.0, 10.0), (40.0, 40.0)),
        );
    })
}

#[component]
fn textured_scene(#[source] step: &u32) {
    view! { canvas(controller: texture_painter(*step), width: SIZE, height: SIZE) }
}

fn paint(backend: &VelloBackend<CosmicEngine>, texture: &wgpu::Texture, color: wgpu::Color) {
    let context = backend.wgpu_context();
    let view = texture.create_view(&Default::default());
    clear(context.device(), context.queue(), &view, color);
}

#[test]
fn redrawing_a_texture_repaints_only_where_it_is() {
    let blue = wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
    let green = wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };
    let Some(mut patched) = backend() else {
        return;
    };
    let texture = texture_of(&patched, 4, green);
    patched.set_wgpu_texture(TEXTURE, &texture);
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, step) = app_at(textured, 0);
    render_once(&mut patched, &mut app, &mut text);

    // The application draws into its texture again and says so.
    paint(&patched, &texture, blue);
    app.update(|cx| step.set(cx, 1));
    render_once(&mut patched, &mut app, &mut text);
    let stats = patched.frame_stats();
    let patched_pixels = patched.read_pixels_rgba8().expect("pixels");

    let mut fresh = backend().expect("backend");
    let texture = texture_of(&fresh, 4, blue);
    fresh.set_wgpu_texture(TEXTURE, &texture);
    let mut text = TextHost::new(CosmicEngine::new(1.0));
    let (mut app, _) = app_at(textured, 1);
    render_once(&mut fresh, &mut app, &mut text);
    assert_same(&patched_pixels, &fresh.read_pixels_rgba8().expect("pixels"));

    // And it was a patch: the 40x40 texture, not the frame.
    assert!(stats.last_pixels > 0 && stats.last_pixels < FULL / 2, "{stats:?}");
}
