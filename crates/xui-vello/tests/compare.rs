//! The same scenes through Skia and through Vello, compared pixel by pixel.
//!
//! Skia is the reference. Nothing here expects the two to be identical:
//! antialiasing, gradient dithering and text rasterization differ by design.
//! The point is to see *where* they differ and by how much, and to catch a
//! scene that drifts far from the reference.
//!
//! `XUI_COMPARE_DIR=<dir> cargo test -p xui-vello --test compare -- --nocapture`
//! also writes `<scene>.skia.png`, `.vello.png` and `.diff.png` there.
#![allow(unused_imports)]

mod common;

use common::*;
use xui_core::{app::AppRoot, prelude::*, text::TextHost};
use xui_cosmic::CosmicEngine;
use xui_skia::{SkiaBackend, SkiaBackendOptions};

fn render_skia(root: AppRoot, scale: f32) -> Frame {
    let mut backend = SkiaBackend::<CosmicEngine>::headless(
        scale,
        SkiaBackendOptions {
            clear_color: Color::rgb(0.0, 0.0, 0.0),
            ..SkiaBackendOptions::default()
        },
    );
    let mut app = App::new(root);
    app.set_scale_factor(scale);
    app.resize(Size::new(SIZE, SIZE));
    let mut text = TextHost::new(CosmicEngine::new(scale));
    app.render(&mut backend, &mut text).expect("skia render");
    let size = backend.frame_size_px();
    Frame {
        unsupported: Default::default(),
        pixels: backend.read_pixels_rgba8().expect("skia pixels"),
        width: size.width as usize,
    }
}

#[derive(Debug)]
struct Diff {
    /// Mean absolute difference over every channel, 0..=255.
    mean: f64,
    max: u8,
    /// Pixels whose worst channel differs by more than 16 / 64.
    over_16: usize,
    over_64: usize,
    pixels: usize,
}

fn diff(a: &Frame, b: &Frame) -> Diff {
    assert_eq!(a.pixels.len(), b.pixels.len(), "frames differ in size");
    let mut total = 0u64;
    let (mut max, mut over_16, mut over_64) = (0u8, 0, 0);
    for (p, q) in a.pixels.chunks_exact(4).zip(b.pixels.chunks_exact(4)) {
        let worst = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .inspect(|d| total += *d as u64)
            .max()
            .unwrap();
        max = max.max(worst);
        over_16 += (worst > 16) as usize;
        over_64 += (worst > 64) as usize;
    }
    Diff {
        mean: total as f64 / a.pixels.len() as f64,
        max,
        over_16,
        over_64,
        pixels: a.pixels.len() / 4,
    }
}

fn write_png(dir: &std::path::Path, name: &str, frame: &Frame) {
    let file = std::fs::File::create(dir.join(name)).expect("create png");
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        frame.width as u32,
        (frame.pixels.len() / 4 / frame.width) as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&frame.pixels))
        .expect("write png");
}

/// Skia, Vello, and their difference scaled up so small drift is visible.
fn dump(dir: &std::path::Path, name: &str, skia: &Frame, vello: &Frame) {
    std::fs::create_dir_all(dir).expect("create dir");
    write_png(dir, &format!("{name}.skia.png"), skia);
    write_png(dir, &format!("{name}.vello.png"), vello);
    let pixels = skia
        .pixels
        .chunks_exact(4)
        .zip(vello.pixels.chunks_exact(4))
        .flat_map(|(p, q)| {
            let d = |i: usize| p[i].abs_diff(q[i]).saturating_mul(4);
            [d(0), d(1), d(2), 255]
        })
        .collect();
    write_png(
        dir,
        &format!("{name}.diff.png"),
        &Frame {
            unsupported: Default::default(),
            pixels,
            width: skia.width,
        },
    );
}

/// `(name, scene, worst acceptable mean difference at 1x, at 2x)`.
///
/// The bounds sit a little above what was measured, so they catch a scene
/// drifting from Skia without pretending the two are the same renderer. Text is
/// the loose one: Skia rasterizes through the platform (CoreText on macOS),
/// which positions and weights glyphs differently from an outline fill.
///
/// `backdrop_blur`, `layer_blur` and `layer_drop_shadow` are loose because
/// Vello composes in linear light and Skia on the encoded values: half of red
/// over white is 188 against 128 in the green and blue. A backdrop blur mixes
/// two colours; a layer blur or a shadow is one colour whose soft edge is
/// composited over another, which is the same mix. Inside the shapes the two
/// agree. The bounds go back to 1 when Skia is linear too.
///
/// `box_shadow` is loose at 2x for a reason that is Skia's: its shape shadow
/// uses `MaskFilter::blur(.., respect_ctm = false)`, so the blur radius is in
/// device pixels and does not grow with the scale factor (it measures 10px wide
/// at both 1x and 2x), while every other blur in the Skia backend, and Vello's
/// shadow, scales. Vello is the consistent one; it is not made to copy that.
const SCENES: &[(&str, AppRoot, f64, f64)] = &[
    ("solid", solid_root, 0.01, 0.01),
    ("rounded_clip", rounded_clip_root, 0.5, 0.5),
    ("box_shadow", shadow_root, 1.5, 4.0),
    ("linear_gradient", linear_gradient_root, 1.5, 1.5),
    ("radial_gradient", radial_gradient_root, 1.5, 1.5),
    ("image_fill", image_root, 0.01, 0.01),
    ("image_contain", contained_image_root, 0.01, 0.01),
    ("text", text_root, 6.0, 8.0),
    ("layer_blur", blur_root, 6.0, 6.0),
    ("layer_colour_matrix", swap_root, 0.01, 0.01),
    ("layer_drop_shadow", shadow_effect_root, 4.5, 4.5),
    ("backdrop_blur", glass_root, 9.0, 9.0),
    ("backdrop_with_content", glass_with_content_root, 0.01, 0.01),
];

fn compare_at(scale: f32) {
    let dir = std::env::var_os("XUI_COMPARE_DIR").map(std::path::PathBuf::from);
    let mut rows = Vec::new();
    for (name, root, bound_1x, bound_2x) in SCENES {
        let bound = if scale > 1.5 { bound_2x } else { bound_1x };
        let Some(vello) = render_scaled(*root, scale, |_| {}) else {
            return;
        };
        let skia = render_skia(*root, scale);
        if let Some(dir) = &dir {
            dump(dir, &format!("{name}@{scale}x"), &skia, &vello);
        }
        rows.push((*name, *bound, diff(&skia, &vello)));
    }
    eprintln!(
        "\nscale {scale}\n{:<24} {:>8} {:>5} {:>8} {:>8}",
        "scene", "mean", "max", ">16", ">64"
    );
    for (name, _, d) in &rows {
        eprintln!(
            "{name:<24} {:>8.3} {:>5} {:>7.1}% {:>7.1}%",
            d.mean,
            d.max,
            100.0 * d.over_16 as f64 / d.pixels as f64,
            100.0 * d.over_64 as f64 / d.pixels as f64,
        );
    }
    let drifted: Vec<_> = rows
        .iter()
        .filter(|(_, bound, d)| d.mean > *bound)
        .map(|(name, bound, d)| format!("{name}: mean {:.3} > {bound}", d.mean))
        .collect();
    assert!(drifted.is_empty(), "drifted from Skia: {drifted:?}");
}

#[test]
fn skia_and_vello_agree_scene_by_scene() {
    compare_at(1.0);
}

#[test]
fn skia_and_vello_agree_on_a_2x_display() {
    compare_at(2.0);
}
