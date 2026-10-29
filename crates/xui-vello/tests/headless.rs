//! End to end without a window: a real `App` renders through `VelloBackend`
//! into its offscreen texture, and the pixels are read back.
//!
//! Skipped (with a note on stderr) when the machine has no wgpu adapter at all.
#![allow(unused_imports)]

mod common;

use common::*;

use xui_core::{
    prelude::*,
    text::TextHost,
    widgets::{CANVAS_GPU_FORMAT, CANVAS_GPU_USAGES, CanvasController, CanvasGpuContext},
};
use xui_cosmic::CosmicEngine;
use xui_interface::{
    ColorStyle, Effect, FilterQuality, FlexDirectionStyle, ImageData, ImageFit, Point, Sampling, ShadowStyle,
};
use xui_macros::component;
use xui_vello::{Unsupported, VelloBackend, VelloBackendOptions};

#[test]
fn a_filled_container_reaches_the_pixels() {
    let Some(frame) = render(solid_root) else {
        return;
    };
    assert!(is_close(frame.at(50, 50), [255, 0, 0, 255]), "{:?}", frame.at(50, 50));
    assert!(is_close(frame.at(2, 2), [255, 0, 0, 255]));
    assert!(is_close(frame.at(97, 97), [255, 0, 0, 255]));
}

#[test]
fn a_rounded_clip_cuts_the_corner_but_not_the_middle() {
    let Some(frame) = render(rounded_clip_root) else {
        return;
    };
    // The corner lies outside a 40px radius, so neither the parent's red nor
    // the child's blue is there: what shows is the app root's own surface.
    let corner = frame.at(1, 1);
    assert!(!is_close(corner, [255, 0, 0, 255]) && !is_close(corner, [0, 0, 255, 255]), "{corner:?}");
    // The edge midpoints are inside the shape, so the cut is a curve, not a box.
    assert!(is_close(frame.at(50, 1), [0, 0, 255, 255]), "{:?}", frame.at(50, 1));
    // The middle shows the clipped child on top of its parent.
    assert!(is_close(frame.at(50, 50), [0, 0, 255, 255]), "{:?}", frame.at(50, 50));
}

#[test]
fn a_blur_effect_softens_the_layer_edge() {
    let Some(frame) = render(blur_root) else {
        return;
    };
    // Box spans x 30..70. Its middle is untouched, and the pixel just outside
    // the left edge, pure white if nothing were blurred, picks up red.
    assert!(is_close(frame.at(50, 50), [255, 0, 0, 255]), "{:?}", frame.at(50, 50));
    let outside = frame.at(28, 50);
    assert!(outside[1] < 235 && outside[1] > 60, "edge not softened: {outside:?}");
    assert!(is_close(frame.at(5, 5), [255, 255, 255, 255]));
    assert!(!frame.unsupported.layer_effect);
}

#[test]
fn a_colour_matrix_effect_swaps_channels() {
    let Some(frame) = render(swap_root) else {
        return;
    };
    assert!(is_close(frame.at(50, 50), [0, 0, 255, 255]), "{:?}", frame.at(50, 50));
    assert!(is_close(frame.at(5, 5), [255, 255, 255, 255]));
}

#[test]
fn a_drop_shadow_effect_falls_below_the_layer() {
    let Some(frame) = render(shadow_effect_root) else {
        return;
    };
    // Box y 30..70, shadow 25px lower: y 55..95. Below the box it darkens the
    // white surface; above the box nothing changes.
    let below = frame.at(50, 85);
    assert!(luma(below) < 3 * 120, "no shadow below: {below:?}");
    assert!(is_close(frame.at(50, 10), [255, 255, 255, 255]));
    // The box itself is still on top.
    assert!(is_close(frame.at(50, 50), [255, 0, 0, 255]), "{:?}", frame.at(50, 50));
}

#[test]
fn text_is_drawn_from_system_fonts() {
    let Some(frame) = render(text_root) else {
        return;
    };
    // Red glyph cores over the white root surface. Antialiased edges are
    // ignored: only fully inked pixels count.
    let ink = count(&frame, |p| is_close(p, [255, 0, 0, 255]));
    assert!(ink > 100, "expected glyph pixels, found {ink}");
    assert!(!frame.unsupported.missing_font, "a run's font failed to load");
    // 32px text sits in the top band of a 100px box, not the bottom.
    let bottom_ink: usize = (70..100)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .filter(|&(x, y)| is_close(frame.at(x, y), [255, 0, 0, 255]))
        .count();
    assert_eq!(bottom_ink, 0, "text drawn far below its line");
}

#[test]
fn an_image_is_stretched_over_its_box() {
    let Some(frame) = render(image_root) else {
        return;
    };
    assert!(is_close(frame.at(25, 25), [255, 0, 0, 255]), "{:?}", frame.at(25, 25));
    assert!(is_close(frame.at(75, 25), [0, 255, 0, 255]), "{:?}", frame.at(75, 25));
    assert!(is_close(frame.at(25, 75), [0, 0, 255, 255]), "{:?}", frame.at(25, 75));
    assert!(is_close(frame.at(75, 75), [255, 255, 255, 255]), "{:?}", frame.at(75, 75));
}

#[test]
fn contain_leaves_the_box_visible_beside_the_image() {
    let Some(frame) = render(contained_image_root) else {
        return;
    };
    // Box is 100x50; the image is a centred 50x50, spanning x = 25..75.
    assert!(is_close(frame.at(5, 25), [0, 0, 255, 255]), "{:?}", frame.at(5, 25));
    assert!(is_close(frame.at(35, 12), [255, 0, 0, 255]), "{:?}", frame.at(35, 12));
    assert!(is_close(frame.at(65, 12), [0, 255, 0, 255]), "{:?}", frame.at(65, 12));
}

#[test]
fn a_shadow_is_blurred_and_offset() {
    let Some(frame) = render(shadow_root) else {
        return;
    };
    // Box occupies x 20..60, y 20..60; the shadow covers y 40..80.
    // Below the box, inside the shadow's body, the white surface is darkened.
    let below = frame.at(40, 70);
    assert!(luma(below) < 3 * 120, "no shadow below the box: {below:?}");
    // The shadow is offset down, so the strip above the box is untouched.
    let above = frame.at(40, 10);
    assert!(is_close(above, [255, 255, 255, 255]), "{above:?}");
    // The edge is soft: the falloff passes through intermediate greys rather
    // than jumping from shadow to white.
    let ramp: Vec<u32> = (76..92).map(|y| luma(frame.at(40, y))).collect();
    assert!(ramp.windows(2).all(|w| w[0] <= w[1]), "not monotonic: {ramp:?}");
    assert!(
        ramp.iter().any(|v| *v > 3 * 40 && *v < 3 * 215),
        "edge is hard, not blurred: {ramp:?}"
    );
}

#[test]
fn a_linear_gradient_runs_from_one_stop_to_the_other() {
    let Some(frame) = render(linear_gradient_root) else {
        return;
    };
    let left = frame.at(2, 50);
    let middle = frame.at(50, 50);
    let right = frame.at(97, 50);
    assert!(left[0] > 230 && left[2] < 25, "left should be red: {left:?}");
    assert!(right[2] > 230 && right[0] < 25, "right should be blue: {right:?}");
    // The midpoint is an even mix, not either end.
    assert!(
        (100..160).contains(&middle[0]) && (100..160).contains(&middle[2]),
        "middle should blend both: {middle:?}"
    );
    // Constant down the box: the gradient axis is horizontal.
    assert!(is_close(frame.at(50, 5), frame.at(50, 95)));
}

#[test]
fn a_radial_gradient_runs_from_the_centre_outwards() {
    let Some(frame) = render(radial_gradient_root) else {
        return;
    };
    let centre = frame.at(50, 50);
    let corner = frame.at(2, 2);
    assert!(centre[0] > 230 && centre[2] < 25, "centre should be red: {centre:?}");
    // The radius is half the short side, so the corner is past it and clamps.
    assert!(corner[2] > 230 && corner[0] < 25, "corner should be blue: {corner:?}");
}

#[test]
fn a_gpu_painter_canvas_is_composited() {
    let Some(frame) = render(gpu_canvas_root) else {
        return;
    };
    assert!(is_close(frame.at(50, 50), [0, 0, 255, 255]), "{:?}", frame.at(50, 50));
    assert!(is_close(frame.at(2, 2), [0, 0, 255, 255]), "{:?}", frame.at(2, 2));
}

#[test]
fn a_registered_texture_is_drawn_where_the_canvas_places_it() {
    let Some(frame) = render_with(registered_texture_root, |backend| {
        let context = backend.wgpu_context();
        let texture = context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("registered"),
            size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CANVAS_GPU_FORMAT,
            usage: CANVAS_GPU_USAGES,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        clear(
            context.device(),
            context.queue(),
            &view,
            wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
        );
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    assert!(is_close(frame.at(25, 50), [0, 255, 0, 255]), "{:?}", frame.at(25, 50));
    // Right of the placed bounds the white root surface is untouched.
    assert!(is_close(frame.at(75, 50), [255, 255, 255, 255]), "{:?}", frame.at(75, 50));
}

#[test]
fn an_unregistered_texture_draws_nothing() {
    let Some(frame) = render(registered_texture_root) else {
        return;
    };
    assert!(is_close(frame.at(25, 50), [255, 255, 255, 255]), "{:?}", frame.at(25, 50));
}

#[test]
fn a_backdrop_blur_softens_what_is_behind_the_layer() {
    let Some(frame) = render(glass_root) else {
        return;
    };
    // Outside the panel (x 25..75) nothing is touched.
    assert!(is_close(frame.at(10, 50), [255, 0, 0, 255]), "{:?}", frame.at(10, 50));
    assert!(is_close(frame.at(90, 50), [0, 0, 255, 255]), "{:?}", frame.at(90, 50));
    // Just inside it, five pixels from the seam, the two colours have mixed.
    // The blur is in linear light: 27% blue there is 146 once encoded, where
    // blurring the encoded values would have given about 69.
    let near = frame.at(45, 50);
    assert!(near[0] > 110 && near[0] < 240, "red should have faded: {near:?}");
    assert!(near[2] > 20 && near[2] < 175, "blue should have bled in: {near:?}");
    // The same pixel is monotonic across the seam.
    let ramp: Vec<u8> = (30..70).map(|x| frame.at(x, 50)[0]).collect();
    assert!(ramp.windows(2).all(|w| w[0] >= w[1]), "not monotonic: {ramp:?}");
    // Deep inside the panel, far from the seam, the blur is nearly nothing: a
    // 0.3% blue tail, which the steep part of the sRGB curve encodes as about 8.
    let far = frame.at(28, 50);
    assert!(far[0] == 255 && far[1] == 0 && far[2] < 16, "{far:?}");
    assert_eq!(frame.unsupported, Default::default());
}

#[test]
fn what_a_glass_layer_draws_is_on_top_of_its_backdrop() {
    let Some(frame) = render(glass_with_content_root) else {
        return;
    };
    // The pane's own child sits at its top-left corner, x 25..35, y 0..10.
    assert!(is_close(frame.at(30, 5), [0, 255, 0, 255]), "{:?}", frame.at(30, 5));
    // The backdrop around it is still the red behind the panel.
    assert!(is_close(frame.at(60, 50), [255, 0, 0, 255]), "{:?}", frame.at(60, 50));
}

const GREEN: wgpu::Color = wgpu::Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 };

/// Red everywhere; a green texture over the middle; a blue square over the
/// texture. Paint order is red, texture, blue.
fn texture_between_shapes(app: &mut App) {
    let controller = CanvasController::with_painter(|painter| {
        painter.rect(
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            Color::rgb(1.0, 0.0, 0.0),
        );
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((20.0, 20.0), (60.0, 60.0)),
        );
        painter.rect(
            xui_interface::Bounds::from_origin_size((40.0, 40.0), (20.0, 20.0)),
            Color::rgb(0.0, 0.0, 1.0),
        );
    });
    mount_canvas(app, controller);
}

#[test]
fn a_texture_sits_where_it_was_painted_in_the_order() {
    let Some(frame) = render_with(texture_between_shapes, |backend| {
        let texture = texture_of(backend, 4, GREEN);
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    // Below the texture: the red field.
    assert!(is_close(frame.at(10, 10), [255, 0, 0, 255]), "{:?}", frame.at(10, 10));
    // The texture itself.
    assert!(is_close(frame.at(30, 30), [0, 255, 0, 255]), "{:?}", frame.at(30, 30));
    // Above the texture: the blue square, not the texture through it.
    assert!(is_close(frame.at(50, 50), [0, 0, 255, 255]), "{:?}", frame.at(50, 50));
    assert_eq!(frame.unsupported, Default::default());
}

fn half_transparent_texture(app: &mut App) {
    let controller = CanvasController::with_painter(|painter| {
        painter.texture_with(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            0.5,
            Sampling::Linear,
        );
    });
    mount_canvas(app, controller);
}

#[test]
fn a_texture_takes_the_opacity_of_its_command() {
    let Some(frame) = render_with(half_transparent_texture, |backend| {
        let texture = texture_of(backend, 4, GREEN);
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    // Half green over the white surface behind it.
    let mixed = frame.at(50, 50);
    // Half of the colour, in linear light: the red and blue of 0.5 linear are
    // 188 encoded, not 128.
    assert!(is_close(mixed, [188, 255, 188, 255]), "{mixed:?}");
}

#[test]
fn an_srgb_texture_keeps_its_colour() {
    // 0.5 written to an sRGB target is stored as ~188, and must come out so.
    let Some(frame) = render_with(registered_texture_root, |backend| {
        let texture = texture_of(
            backend,
            4,
            wgpu::Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 },
        );
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    assert!(is_close(frame.at(25, 50), [188, 188, 188, 255]), "{:?}", frame.at(25, 50));
}

fn clipped_texture(app: &mut App) {
    app.mount::<ClippedTextureScene>(ClippedTextureSceneProps {});
}

#[component]
fn clipped_texture_scene() {
    let controller: CanvasController = CanvasController::with_painter(|painter| {
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
        );
    });
    view! {
        container(width: SIZE, height: SIZE, border_radius: 40.0, clip: true) {
            canvas(controller: controller.clone(), width: SIZE, height: SIZE)
        }
    }
}

#[test]
fn a_texture_is_cut_by_a_rounded_clip() {
    let Some(frame) = render_with(clipped_texture, |backend| {
        let texture = texture_of(backend, 4, GREEN);
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    assert!(is_close(frame.at(50, 50), [0, 255, 0, 255]), "{:?}", frame.at(50, 50));
    // The corner is outside a 40px radius: the texture must not reach it.
    let corner = frame.at(1, 1);
    assert!(!is_close(corner, [0, 255, 0, 255]), "{corner:?}");
    // The edge midpoint is inside the shape.
    assert!(is_close(frame.at(50, 1), [0, 255, 0, 255]), "{:?}", frame.at(50, 1));
}

fn texture_in_a_blurred_layer(app: &mut App) {
    app.mount::<TextureInABlurredLayerScene>(TextureInABlurredLayerSceneProps {});
}

#[component]
fn texture_in_a_blurred_layer_scene() {
    let controller: CanvasController = CanvasController::with_painter(|painter| {
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (60.0, 60.0)),
        );
    });
    view! {
        container(width: SIZE, height: SIZE) {
            container(width: 60.0, height: 60.0, effects: vec![Effect::blur(2.0)]) {
                canvas(controller: controller.clone(), width: 60.0, height: 60.0)
            }
        }
    }
}

#[test]
fn a_texture_inside_a_filtered_layer_is_composed_there() {
    let Some(frame) = render_with(texture_in_a_blurred_layer, |backend| {
        let texture = texture_of(backend, 4, GREEN);
        backend.set_wgpu_texture(REGISTERED, &texture);
    }) else {
        return;
    };
    assert!(is_close(frame.at(30, 30), [0, 255, 0, 255]), "{:?}", frame.at(30, 30));
    assert!(is_close(frame.at(90, 90), [255, 255, 255, 255]));
    assert_eq!(frame.unsupported, Default::default());
}

fn texture_at_an_odd_offset(app: &mut App) {
    let controller = CanvasController::with_painter(|painter| {
        painter.rect(
            xui_interface::Bounds::from_origin_size((0.0, 0.0), (100.0, 100.0)),
            Color::rgb(1.0, 0.0, 0.0),
        );
        // Odd offsets, so the texture's edges fall inside 2x2 pixel quads.
        painter.texture(
            REGISTERED,
            1,
            xui_interface::Bounds::from_origin_size((11.0, 13.0), (41.0, 37.0)),
        );
    });
    mount_canvas(app, controller);
}

#[test]
fn a_texture_edge_is_clean_where_it_splits_a_pixel_quad() {
    // The compositor once read undefined derivatives along exactly such edges,
    // and the wrong pixels differed from run to run, so draw it a few times.
    for _ in 0..6 {
        let Some(frame) = render_with(texture_at_an_odd_offset, |backend| {
            let texture = texture_of(backend, 4, GREEN);
            backend.set_wgpu_texture(REGISTERED, &texture);
        }) else {
            return;
        };
        // Every pixel on the texture's outline is fully green, none blended.
        for x in 11..52 {
            for y in [13usize, 49] {
                assert!(is_close(frame.at(x, y), [0, 255, 0, 255]), "({x}, {y}) {:?}", frame.at(x, y));
            }
        }
        for y in 13..50 {
            for x in [11usize, 51] {
                assert!(is_close(frame.at(x, y), [0, 255, 0, 255]), "({x}, {y}) {:?}", frame.at(x, y));
            }
        }
        // And just outside it nothing of the texture leaks.
        assert!(is_close(frame.at(10, 30), [255, 0, 0, 255]), "{:?}", frame.at(10, 30));
        assert!(is_close(frame.at(30, 12), [255, 0, 0, 255]), "{:?}", frame.at(30, 12));
    }
}

#[test]
fn a_backdrop_blur_mixes_colours_in_linear_light() {
    let Some(frame) = render(glass_root) else {
        return;
    };
    // On the seam itself red and blue are half and half. Half of each in linear
    // light is 0.5, which sRGB encodes as 188; in the encoded values it would be
    // 128, a visibly darker purple.
    let seam = frame.at(50, 50);
    assert!(
        (seam[0] as i32 - 188).abs() <= 6 && (seam[2] as i32 - 188).abs() <= 6 && seam[1] < 4,
        "{seam:?}"
    );
}

fn dim_gray_root(app: &mut App) {
    app.mount::<DimGray>(DimGrayProps {});
}

#[component]
fn dim_gray() {
    // A mid-grey box (188 encoded is 0.5 linear), halved by a colour matrix.
    let halve: [f32; 20] = [
        0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0,
        1.0, 0.0,
    ];
    view! {
        container(width: SIZE, height: SIZE) {
            container(
                width: 60.0,
                height: 60.0,
                background: Color::rgb(188.0 / 255.0, 188.0 / 255.0, 188.0 / 255.0),
                effects: vec![Effect::ColorMatrix(*halve)],
            )
        }
    }
}

#[test]
fn a_colour_matrix_acts_on_linear_light() {
    let Some(frame) = render(dim_gray_root) else {
        return;
    };
    // 0.5 linear halved is 0.25 linear, which sRGB encodes as 137. On the
    // encoded values it would be 94.
    let inside = frame.at(30, 30);
    assert!(
        (inside[0] as i32 - 137).abs() <= 3 && inside[0] == inside[1] && inside[1] == inside[2],
        "{inside:?}"
    );
}
