//! Text drawn from glyph bitmaps (`xui-wgpu-text`), against text drawn as
//! Vello outlines.

mod common;

use common::*;
use xui_core::{app::ComponentFn, prelude::*, widgets::TextWidget};
use xui_interface::Effect;
use xui_vello::{AtlasOptions, ResourceStats, VelloBackendOptions};

fn options(bitmap_text: bool) -> VelloBackendOptions {
    VelloBackendOptions {
        clear_color: CLEAR,
        bitmap_text,
        ..VelloBackendOptions::default()
    }
}

fn draw(root: ComponentFn, scale: f32, bitmap_text: bool) -> Option<(Frame, ResourceStats)> {
    render_options(root, scale, options(bitmap_text), |_| {})
}

/// Fully inked red pixels.
fn ink(frame: &Frame) -> usize {
    count(frame, |pixel| is_close(pixel, [255, 0, 0, 255]))
}

/// How much red text a pixel shows over the white root: 0 to 255.
fn redness([r, g, _, _]: [u8; 4]) -> u32 {
    u32::from(r.saturating_sub(g))
}

/// The red text drawn, in pixels' worth of full coverage.
fn coverage(frame: &Frame) -> f32 {
    frame
        .pixels
        .chunks_exact(4)
        .map(|pixel| redness([pixel[0], pixel[1], pixel[2], pixel[3]]))
        .sum::<u32>() as f32
        / 255.0
}

/// The smallest rectangle holding every pixel the text visibly reddens:
/// x0, y0, x1, y1.
fn ink_bounds(frame: &Frame) -> [usize; 4] {
    let height = frame.pixels.len() / 4 / frame.width;
    let mut bounds = [usize::MAX, usize::MAX, 0, 0];
    for y in 0..height {
        for x in 0..frame.width {
            if redness(frame.at(x, y)) > 64 {
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x + 1),
                    bounds[3].max(y + 1),
                ];
            }
        }
    }
    bounds
}

#[test]
fn text_is_drawn_from_glyph_bitmaps() {
    let Some((frame, stats)) = draw(text_root, 1.0, true) else {
        return;
    };
    assert!(stats.glyph_atlas_entries > 0, "{stats:?}");
    assert!(ink(&frame) > 100, "{} inked pixels", ink(&frame));

    let Some((frame, stats)) = draw(text_root, 1.0, false) else {
        return;
    };
    assert_eq!(stats.glyph_atlas_entries, 0, "outlines need no atlas");
    assert!(ink(&frame) > 100);
}

#[test]
fn bitmap_text_lands_where_outline_text_does() {
    let (Some((bitmap, _)), Some((outline, _))) =
        (draw(text_root, 1.0, true), draw(text_root, 1.0, false))
    else {
        return;
    };
    let (a, b) = (ink_bounds(&bitmap), ink_bounds(&outline));
    for edge in 0..4 {
        assert!(
            a[edge].abs_diff(b[edge]) <= 2,
            "bitmap {a:?}, outlines {b:?}"
        );
    }
    let (a, b) = (coverage(&bitmap), coverage(&outline));
    assert!(
        (a / b - 1.0).abs() < 0.15,
        "bitmap {a} pixels of ink, outlines {b}"
    );
}

#[test]
fn hidpi_text_is_rendered_at_the_physical_size() {
    let (Some((one, _)), Some((two, _))) = (draw(text_root, 1.0, true), draw(text_root, 2.0, true))
    else {
        return;
    };
    let ratio = coverage(&two) / coverage(&one);
    assert!(
        (3.6..4.4).contains(&ratio),
        "{ratio}: twice the size is four times the ink"
    );
}

/// Red text, and a blue box painted after it over its left half.
fn covered_text(_: &mut HookContext<'_>) -> ElementDesc {
    block(
        Style::new().width(SIZE).height(SIZE),
        vec![
            TextWidget::new("Hello xui")
                .style(
                    Style::new()
                        .width(SIZE)
                        .height(SIZE)
                        .color(RED)
                        .font_size(32.0),
                )
                .into_element_desc(),
            block(
                Style::new()
                    .absolute()
                    .inset(EdgeInsets::all(0.0))
                    .width(50.0)
                    .height(SIZE)
                    .background(BLUE),
                vec![],
            ),
        ],
    )
}

#[test]
fn a_shape_painted_after_text_covers_it() {
    let Some((frame, stats)) = draw(covered_text, 1.0, true) else {
        return;
    };
    assert!(stats.glyph_atlas_entries > 0);
    for y in 0..SIZE as usize {
        for x in 0..50 {
            assert_eq!(
                frame.at(x, y),
                [0, 0, 255, 255],
                "text over the box at ({x}, {y})"
            );
        }
    }
    assert!(
        ink(&frame) > 20,
        "the right half of the text is still there"
    );
}

/// [`text_root`] in a layer of its own: an identity colour matrix isolates
/// it without changing what it draws, so the text is composed in linear
/// light, over transparent, and the layer over the root.
fn layered_text(_: &mut HookContext<'_>) -> ElementDesc {
    #[rustfmt::skip]
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 1.0, 0.0,
    ];
    block(
        Style::new()
            .width(SIZE)
            .height(SIZE)
            .effects(vec![Effect::ColorMatrix(identity)]),
        vec![
            TextWidget::new("Hello xui")
                .style(
                    Style::new()
                        .width(SIZE)
                        .height(SIZE)
                        .color(RED)
                        .font_size(32.0),
                )
                .into_element_desc(),
        ],
    )
}

#[test]
fn text_in_an_isolated_layer_is_drawn_from_bitmaps_too() {
    // In the layer, text is composed in linear light, as Vello's own outlines
    // are there: its edges differ from text drawn straight over the root,
    // for bitmaps and outlines alike, so it is held against outlines in the
    // same layer.
    let (Some((bitmap, stats)), Some((outline, _))) = (
        draw(layered_text, 1.0, true),
        draw(layered_text, 1.0, false),
    ) else {
        return;
    };
    assert!(stats.glyph_atlas_entries > 0);
    let (a, b) = (ink_bounds(&bitmap), ink_bounds(&outline));
    for edge in 0..4 {
        assert!(
            a[edge].abs_diff(b[edge]) <= 2,
            "bitmap {a:?}, outlines {b:?}"
        );
    }
    let (a, b) = (coverage(&bitmap), coverage(&outline));
    assert!(
        (a / b - 1.0).abs() < 0.15,
        "bitmap {a} pixels of ink, outlines {b}"
    );
}

fn rotated_text(_: &mut HookContext<'_>) -> ElementDesc {
    TextWidget::new("Hello xui")
        .style(
            Style::new()
                .width(SIZE)
                .height(SIZE)
                .color(RED)
                .font_size(32.0)
                .rotate(0.3),
        )
        .into_element_desc()
}

#[test]
fn rotated_text_is_drawn_as_outlines() {
    let Some((frame, stats)) = draw(rotated_text, 1.0, true) else {
        return;
    };
    assert_eq!(stats.glyph_atlas_entries, 0, "a bitmap cannot be rotated");
    assert!(ink(&frame) > 100);
}

#[test]
fn text_the_atlas_cannot_hold_is_drawn_as_outlines() {
    // One 16-texel page: no 32px glyph fits, at any number of evictions.
    let tiny = VelloBackendOptions {
        glyph_atlas: AtlasOptions {
            page_size: 16,
            budget_pages: 1,
            max_pages: 1,
            ..AtlasOptions::default()
        },
        ..options(true)
    };
    let (Some((full, _)), Some((outline, _))) = (
        render_options(text_root, 1.0, tiny, |_| {}),
        draw(text_root, 1.0, false),
    ) else {
        return;
    };
    assert!(ink(&full) > 100, "the text is drawn, not left with holes");
    assert_eq!(full.pixels, outline.pixels, "as Vello outlines, whole");
}
