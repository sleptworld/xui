use std::sync::Arc;

use xui_interface::{
    FontCollection, GlyphImage, GlyphRequest, ParagraphLayout, ParagraphStyle, Shaper, TextBackend,
    TextBoxStyle, TextLayoutConstraints, TextLayoutInput, TextStyle,
};

use super::*;

#[test]
fn shader_is_valid_wgsl() {
    let module = naga::front::wgsl::parse_str(include_str!("text.wgsl")).expect("text.wgsl");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .expect("text.wgsl validates");
}

/// Glyph 1 is a solid 4×6 mask, glyph 2 a 2×2 opaque blue bitmap; anything
/// else has no ink.
struct Blocks;

impl GlyphRasterizer for Blocks {
    type FontId = u32;

    fn rasterize(&self, request: &GlyphRequest<u32>) -> Option<GlyphImage> {
        match request.glyph {
            1 => Some(GlyphImage {
                format: RasterizedGlyphFormat::Mask,
                width: 4,
                height: 6,
                left: 0,
                top: 6,
                pixels: Arc::from(vec![255u8; 24]),
            }),
            2 => Some(GlyphImage {
                format: RasterizedGlyphFormat::Color,
                width: 2,
                height: 2,
                left: 0,
                top: 2,
                pixels: Arc::from([0u8, 0, 255, 255].repeat(4)),
            }),
            _ => None,
        }
    }
}

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).ok()?;
    Some(Gpu { device, queue })
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Gpu {
    /// Draws `batches` into a transparent `extent` target and reads it back
    /// as RGBA rows.
    fn draw<F: Copy + Eq + Hash>(
        &self,
        renderer: &mut TextRenderer<F>,
        extent: (u32, u32),
        batches: &[TextBatch],
    ) -> Vec<u8> {
        self.draw_into(renderer, extent, batches, FORMAT, wgpu::Color::TRANSPARENT)
    }

    /// Draws `batches` into an `extent` target of `format` cleared to `clear`
    /// and reads its texels back, rows tightly packed.
    fn draw_into<F: Copy + Eq + Hash>(
        &self,
        renderer: &mut TextRenderer<F>,
        extent: (u32, u32),
        batches: &[TextBatch],
        format: wgpu::TextureFormat,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        renderer.upload(&self.device, &self.queue);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for batch in batches {
                renderer.render(&mut pass, batch, format);
            }
        }

        let unpadded = extent.0 * format.block_copy_size(None).expect("a colour format");
        let padded = unpadded.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: padded as u64 * extent.1 as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(extent.1),
                },
            },
            wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("poll");
        receiver.recv().unwrap().expect("map");
        let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
        let mut pixels = Vec::with_capacity((unpadded * extent.1) as usize);
        for row in mapped.chunks_exact(padded as usize) {
            pixels.extend_from_slice(&row[..unpadded as usize]);
        }
        pixels
    }
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * width + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

fn glyph(id: u32, x: f32, y: f32) -> PositionedGlyph {
    PositionedGlyph { id, x, y }
}

#[test]
fn draws_masks_and_colour_bitmaps_where_their_pen_puts_them() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (32, 16),
    };
    renderer.begin_frame(0);
    let batch = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[
            TextDraw::Glyphs(GlyphRunDraw {
                font: 0,
                ppem: 16.0,
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
                glyphs: &[glyph(1, 4.0, 10.0), glyph(0, 9.0, 10.0)],
            }),
            TextDraw::Glyphs(GlyphRunDraw {
                font: 0,
                ppem: 16.0,
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
                glyphs: &[glyph(2, 20.0, 10.0)],
            }),
        ],
    );
    let pixels = gpu.draw(&mut renderer, target.extent, &[batch]);
    let width = target.extent.0;

    // The mask covers x 4..8 and y 4..10 (top 6 above the baseline at 10).
    assert_eq!(pixel(&pixels, width, 4, 4), [255; 4]);
    assert_eq!(pixel(&pixels, width, 7, 9), [255; 4]);
    assert_eq!(pixel(&pixels, width, 3, 4), [0; 4]);
    assert_eq!(pixel(&pixels, width, 8, 9), [0; 4]);
    assert_eq!(pixel(&pixels, width, 4, 10), [0; 4]);
    // The colour bitmap keeps its own colour: x 20..22, y 8..10.
    assert_eq!(pixel(&pixels, width, 20, 8), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, width, 21, 9), [0, 0, 255, 255]);
    assert_eq!(pixel(&pixels, width, 22, 9), [0; 4]);
}

#[test]
fn clips_cut_glyphs_and_targets_offset_them() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    // Texel (0, 0) of the target is at (100, 50) of the space glyphs are in.
    let target = TextTarget {
        origin_px: (100, 50),
        extent: (16, 16),
    };
    renderer.begin_frame(0);
    let half_red = [1.0, 0.0, 0.0, 0.5];
    let batch = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[TextDraw::Glyphs(GlyphRunDraw {
            font: 0,
            ppem: 16.0,
            color: half_red,
            clip: Some([100.0, 50.0, 116.0, 56.0]),
            glyphs: &[glyph(1, 102.0, 60.0)],
        })],
    );
    let pixels = gpu.draw(&mut renderer, target.extent, &[batch]);
    let width = target.extent.0;

    // The quad is x 2..6, y 4..10; the clip keeps rows above 6.
    assert_eq!(pixel(&pixels, width, 2, 4), [128, 0, 0, 128]);
    assert_eq!(pixel(&pixels, width, 5, 5), [128, 0, 0, 128]);
    assert_eq!(pixel(&pixels, width, 2, 6), [0; 4]);
    assert_eq!(pixel(&pixels, width, 2, 9), [0; 4]);
}

#[test]
fn batches_draw_in_the_order_they_are_rendered() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (16, 16),
    };
    renderer.begin_frame(0);
    let run = |color| {
        TextDraw::Glyphs(GlyphRunDraw {
            font: 0,
            ppem: 16.0,
            color,
            clip: None,
            glyphs: &[PositionedGlyph {
                id: 1,
                x: 2.0,
                y: 8.0,
            }],
        })
    };
    let red = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[run([1.0, 0.0, 0.0, 1.0])],
    );
    let green = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[run([0.0, 1.0, 0.0, 1.0])],
    );
    assert_eq!(renderer.atlas().len(), 1, "one bitmap serves both batches");

    let pixels = gpu.draw(&mut renderer, target.extent, &[red.clone(), green.clone()]);
    assert_eq!(pixel(&pixels, 16, 3, 5), [0, 255, 0, 255]);
    let pixels = gpu.draw(&mut renderer, target.extent, &[green, red]);
    assert_eq!(pixel(&pixels, 16, 3, 5), [255, 0, 0, 255]);
}

#[test]
fn draws_text_shaped_and_rasterized_by_a_real_backend() {
    let Some(gpu) = gpu() else { return };
    let mut backend = xui_f::FBackend::new();
    if backend.face_count() == 0 {
        return;
    }
    let mut state = backend.create_state();
    let layout: ParagraphLayout<xui_f::FFontId> = backend.layout_paragraph(
        &mut state,
        TextLayoutInput::new(
            "Hi 中".into(),
            TextLayoutConstraints::max_width(200.0),
            TextStyle::default().into(),
            ParagraphStyle::default(),
            TextBoxStyle::default(),
            0,
        ),
    );
    let rasterizer = backend.rasterizer().unwrap().clone();
    let scale = 2.0;
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (128, 64),
    };
    let glyphs: Vec<Vec<PositionedGlyph>> = layout
        .runs
        .iter()
        .map(|run| {
            layout.glyphs[run.glyph_range.clone()]
                .iter()
                .map(|glyph| PositionedGlyph {
                    id: glyph.glyph_id,
                    x: (4.0 + glyph.draw_pos.x) * scale,
                    y: (4.0 + glyph.draw_pos.y) * scale,
                })
                .collect()
        })
        .collect();
    let runs: Vec<_> = layout
        .runs
        .iter()
        .zip(&glyphs)
        .map(|(run, glyphs)| {
            TextDraw::Glyphs(GlyphRunDraw {
                font: run.font_id,
                ppem: run.font_size * scale,
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
                glyphs,
            })
        })
        .collect();

    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    renderer.begin_frame(backend.fonts().epoch());
    let batch = renderer.prepare(&gpu.device, &gpu.queue, &*rasterizer, target, &runs);
    assert!(!batch.is_empty());
    let pixels = gpu.draw(&mut renderer, target.extent, &[batch]);
    let inked = pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
    assert!(inked > 100, "{inked} pixels have ink");
}

/// The first glyph of "Hello" shaped by a real backend: (font, glyph, ppem).
fn real_glyph(backend: &mut xui_f::FBackend) -> (xui_f::FFontId, u32, f32) {
    let mut state = backend.create_state();
    let layout = backend.layout_paragraph(
        &mut state,
        TextLayoutInput::new(
            "Hello".into(),
            TextLayoutConstraints::max_width(400.0),
            TextStyle::default().into(),
            ParagraphStyle::default(),
            TextBoxStyle::default(),
            0,
        ),
    );
    let run = &layout.runs[0];
    (
        run.font_id,
        layout.glyphs[run.glyph_range.start].glyph_id,
        run.font_size,
    )
}

#[test]
fn atlas_places_and_caches_real_glyphs() {
    let mut backend = xui_f::FBackend::new();
    if backend.face_count() == 0 {
        return;
    }
    let (font, glyph, ppem) = real_glyph(&mut backend);
    let rasterizer = backend.rasterizer().unwrap().clone();
    let mut atlas = GlyphAtlas::new(AtlasOptions::default());

    atlas.begin_frame(backend.fonts().epoch());
    let first = atlas
        .glyph(&*rasterizer, font, glyph, ppem, 10.0, 20.0)
        .unwrap()
        .expect("'H' has ink");
    let placed = first.placed;
    assert_eq!(placed.x, 10 + placed.glyph.left);
    assert_eq!(placed.y, 20 - placed.glyph.top);
    assert!(placed.y < 20, "'H' is drawn above its baseline");
    assert_eq!(
        atlas.page(placed.glyph.page).unwrap().format(),
        AtlasFormat::Alpha8
    );
    let image = first.upload.expect("a new glyph's bitmap is handed over");
    assert_eq!(
        (image.width, image.height),
        (placed.glyph.width, placed.glyph.height)
    );
    assert!(image.pixels.iter().any(|value| *value > 0));

    // Same glyph, another whole-pixel position: nothing new to write.
    let moved = atlas
        .glyph(&*rasterizer, font, glyph, ppem, 50.0, 20.0)
        .unwrap()
        .unwrap();
    assert_eq!(moved.placed.glyph, placed.glyph);
    assert!(moved.upload.is_none());

    // A quarter-pixel offset is its own bitmap.
    let shifted = atlas
        .glyph(&*rasterizer, font, glyph, ppem, 50.25, 20.0)
        .unwrap()
        .unwrap();
    assert_ne!(shifted.placed.glyph, placed.glyph);
    assert!(shifted.upload.is_some());

    atlas.set_raster_options(xui_interface::RasterOptions {
        subpixel_antialiasing: true,
        ..Default::default()
    });
    let lcd = atlas
        .glyph(&*rasterizer, font, glyph, ppem, 10.0, 20.0)
        .unwrap()
        .unwrap();
    assert_eq!(lcd.placed.glyph.format, RasterizedGlyphFormat::SubpixelMask);
    assert_eq!(
        atlas.page(lcd.placed.glyph.page).unwrap().format(),
        AtlasFormat::Rgba8
    );
}

/// Glyph 3 fills a whole 16px page; glyph 4 is an 8px checkerboard.
struct Pages;

impl GlyphRasterizer for Pages {
    type FontId = u32;

    fn rasterize(&self, request: &GlyphRequest<u32>) -> Option<GlyphImage> {
        let side = match request.glyph {
            3 => 16,
            4 => 8,
            _ => return None,
        };
        let pixels = (0..side * side)
            .map(|index| {
                let (x, y) = (index % side, index / side);
                if request.glyph == 3 || (x + y) % 2 == 0 {
                    255
                } else {
                    0
                }
            })
            .collect::<Vec<u8>>();
        Some(GlyphImage {
            format: RasterizedGlyphFormat::Mask,
            width: side,
            height: side,
            left: 0,
            top: side as i32,
            pixels: Arc::from(pixels),
        })
    }
}

#[test]
fn an_evicted_slot_is_reused_without_showing_what_was_there() {
    let Some(gpu) = gpu() else { return };
    let options = AtlasOptions {
        page_size: 16,
        budget_pages: 1,
        max_pages: 1,
        ..AtlasOptions::default()
    };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], options);
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (32, 32),
    };
    let full_page = [glyph(3, 0.0, 16.0)];
    let checkerboard = [glyph(4, 4.0, 20.0)];
    let draw = |glyphs| {
        TextDraw::Glyphs(GlyphRunDraw {
            font: 0,
            ppem: 16.0,
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
            glyphs,
        })
    };

    renderer.begin_frame(0);
    let full = renderer.prepare(&gpu.device, &gpu.queue, &Pages, target, &[draw(&full_page)]);
    gpu.draw(&mut renderer, target.extent, &[full]);

    // The page is full of a glyph this frame no longer uses: the checkerboard
    // takes its place, over texels that are all still white.
    renderer.begin_frame(0);
    let checker = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Pages,
        target,
        &[draw(&checkerboard)],
    );
    assert!(!checker.is_empty());
    assert_eq!(renderer.atlas().len(), 1);
    let pixels = gpu.draw(&mut renderer, target.extent, &[checker]);

    // Quad x 4..12, y 12..20.
    assert_eq!(pixel(&pixels, 32, 4, 12), [255; 4]);
    assert_eq!(pixel(&pixels, 32, 5, 12), [0; 4]);
    assert_eq!(pixel(&pixels, 32, 11, 19), [255; 4]);
    assert_eq!(pixel(&pixels, 32, 12, 12), [0; 4]);
    assert_eq!(pixel(&pixels, 32, 4, 20), [0; 4]);
    let inked = pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0).count();
    assert_eq!(inked, 32, "only the checkerboard's own texels are drawn");
}

#[test]
fn rectangles_cover_the_area_of_each_pixel_they_touch() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::<u32>::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (8, 8),
    };
    renderer.begin_frame(0);
    let batch = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[TextDraw::Rect(RectDraw {
            rect: [2.5, 2.0, 6.0, 3.0],
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
        })],
    );
    let pixels = gpu.draw(&mut renderer, target.extent, &[batch]);
    assert_eq!(
        pixel(&pixels, 8, 2, 2),
        [128; 4],
        "half of the pixel is covered"
    );
    assert_eq!(pixel(&pixels, 8, 3, 2), [255; 4]);
    assert_eq!(pixel(&pixels, 8, 5, 2), [255; 4]);
    assert_eq!(pixel(&pixels, 8, 6, 2), [0; 4]);
    assert_eq!(pixel(&pixels, 8, 3, 3), [0; 4]);
}

/// Glyph 5 is a 1×1 mask of half coverage.
struct Half;

impl GlyphRasterizer for Half {
    type FontId = u32;

    fn rasterize(&self, _request: &GlyphRequest<u32>) -> Option<GlyphImage> {
        Some(GlyphImage {
            format: RasterizedGlyphFormat::Mask,
            width: 1,
            height: 1,
            left: 0,
            top: 1,
            pixels: Arc::from([128u8]),
        })
    }
}

fn half_float(bits: u16) -> f32 {
    let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
    let exponent = ((bits >> 10) & 0x1f) as i32;
    let fraction = (bits & 0x3ff) as f32;
    sign * match exponent {
        0 => fraction / 1024.0 * 2f32.powi(-14),
        _ => (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
    }
}

#[test]
fn linear_targets_get_decoded_colour_blended_in_linear_light() {
    let Some(gpu) = gpu() else { return };
    const LINEAR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    let mut renderer = TextRenderer::<u32>::new(&gpu.device, &[LINEAR], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (4, 4),
    };
    // The red value a half-covered pixel holds, in linear light.
    let mut linear = |color: [f32; 4], background: f64| {
        renderer.begin_frame(0);
        let batch = renderer.prepare(
            &gpu.device,
            &gpu.queue,
            &Half,
            target,
            &[TextDraw::Glyphs(GlyphRunDraw {
                font: 0,
                ppem: 16.0,
                color,
                clip: None,
                glyphs: &[glyph(5, 1.0, 2.0)],
            })],
        );
        let clear = wgpu::Color {
            r: background,
            g: background,
            b: background,
            a: 1.0,
        };
        let texels = gpu.draw_into(&mut renderer, target.extent, &[batch], LINEAR, clear);
        // Texel (1, 1): eight bytes a texel, red first.
        let at = (4 + 1) * 8;
        half_float(u16::from_le_bytes([texels[at], texels[at + 1]]))
    };
    let coverage = 128.0 / 255.0;
    let black_on_white = linear([0.0, 0.0, 0.0, 1.0], 1.0);
    assert!(
        (black_on_white - (1.0 - coverage)).abs() < 0.005,
        "{black_on_white}"
    );
    // Mid grey (encoded 0.5) is decoded before it is blended.
    let grey_on_black = linear([0.5, 0.5, 0.5, 1.0], 0.0);
    assert!(
        (grey_on_black - 0.214 * coverage).abs() < 0.005,
        "{grey_on_black}"
    );
}

#[test]
fn a_batch_with_glyphs_the_atlas_cannot_hold_says_so() {
    let Some(gpu) = gpu() else { return };
    // One 16-texel page, which one full-page glyph fills.
    let options = AtlasOptions {
        page_size: 16,
        budget_pages: 1,
        max_pages: 1,
        subpixel_max_ppem: 0.0,
        ..AtlasOptions::default()
    };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], options);
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (32, 32),
    };
    let run = |glyphs| {
        TextDraw::Glyphs(GlyphRunDraw {
            font: 0,
            ppem: 16.0,
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
            glyphs,
        })
    };
    let (full_page, checkerboard) = ([glyph(3, 0.0, 16.0)], [glyph(4, 0.0, 30.0)]);
    renderer.begin_frame(0);
    let fits = renderer.prepare(&gpu.device, &gpu.queue, &Pages, target, &[run(&full_page)]);
    assert!(!fits.incomplete());
    // The page is full of a glyph this frame uses: no room for another.
    let full = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Pages,
        target,
        &[run(&checkerboard)],
    );
    assert!(full.incomplete());
    let mut both = fits.clone();
    both.append(full);
    assert!(both.incomplete());

    // Next frame the old glyph is stale and gives way.
    renderer.begin_frame(0);
    let next = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Pages,
        target,
        &[run(&checkerboard)],
    );
    assert!(!next.incomplete() && !next.is_empty());
}

/// 3,500 CJK glyphs shaped by a real backend: (font, glyph) pairs.
fn cjk_glyphs(backend: &mut xui_f::FBackend) -> Vec<(xui_f::FFontId, u32)> {
    let text: String = (0x4E00u32..0x4E00 + 3500)
        .filter_map(char::from_u32)
        .collect();
    let mut state = backend.create_state();
    let layout = backend.layout_paragraph(
        &mut state,
        TextLayoutInput::new(
            text.into(),
            TextLayoutConstraints::Unbound,
            TextStyle::default().into(),
            ParagraphStyle::default(),
            TextBoxStyle::default(),
            0,
        ),
    );
    layout
        .runs
        .iter()
        .flat_map(|run| {
            layout.glyphs[run.glyph_range.clone()]
                .iter()
                .map(move |glyph| (run.font_id, glyph.glyph_id))
        })
        .collect()
}

#[test]
fn a_screen_of_cjk_text_fits_the_default_atlas() {
    let mut backend = xui_f::FBackend::new();
    if backend.face_count() == 0 {
        return;
    }
    let glyphs = cjk_glyphs(&mut backend);
    let rasterizer = backend.rasterizer().unwrap().clone();
    let budget = AtlasOptions::default().budget_pages;

    // Body text on a 2x display, at quarter-pixel positions where they are
    // used: every glyph is placed, within the budget.
    for ppem in [28.0, 32.0, 40.0] {
        let mut atlas = GlyphAtlas::new(AtlasOptions::default());
        atlas.begin_frame(0);
        for (index, &(font, glyph)) in glyphs.iter().enumerate() {
            let x = index as f32 * 0.37;
            assert!(
                atlas.glyph(&*rasterizer, font, glyph, ppem, x, 0.0).is_ok(),
                "glyph {index} at {ppem} ppem has no room"
            );
        }
        assert!(
            atlas.page_count() <= budget,
            "{} pages at {ppem} ppem",
            atlas.page_count()
        );
    }

    // Headings this large need more than the budget in one frame: the atlas
    // grows for it, drops nothing, and gives the pages back once they idle.
    let mut atlas = GlyphAtlas::new(AtlasOptions::default());
    atlas.begin_frame(0);
    for &(font, glyph) in &glyphs {
        assert!(
            atlas
                .glyph(&*rasterizer, font, glyph, 72.0, 0.0, 0.0)
                .is_ok()
        );
    }
    let grown = atlas.page_count();
    assert!(grown > budget, "{grown} pages");
    atlas.begin_frame(0);
    atlas.begin_frame(0);
    assert_eq!(atlas.page_count(), budget);
}

#[test]
fn masks_and_colour_glyphs_mixed_in_a_run_are_one_draw() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (64, 16),
    };
    let glyphs = [
        glyph(1, 2.0, 10.0),
        glyph(2, 10.0, 10.0),
        glyph(1, 14.0, 10.0),
        glyph(2, 22.0, 10.0),
    ];
    renderer.begin_frame(0);
    let batch = renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &Blocks,
        target,
        &[
            TextDraw::Glyphs(GlyphRunDraw {
                font: 0,
                ppem: 16.0,
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
                glyphs: &glyphs,
            }),
            TextDraw::Rect(RectDraw {
                rect: [0.0, 12.0, 30.0, 13.0],
                color: [1.0, 1.0, 1.0, 1.0],
                clip: None,
            }),
        ],
    );
    assert_eq!(batch.draws.len(), 1, "{:?}", batch.draws);
    let pixels = gpu.draw(&mut renderer, target.extent, &[batch]);
    assert_eq!(pixel(&pixels, 64, 2, 5), [255; 4], "mask");
    assert_eq!(pixel(&pixels, 64, 10, 9), [0, 0, 255, 255], "colour glyph");
    assert_eq!(pixel(&pixels, 64, 14, 5), [255; 4], "mask again");
    assert_eq!(
        pixel(&pixels, 64, 22, 9),
        [0, 0, 255, 255],
        "colour glyph again"
    );
    assert_eq!(pixel(&pixels, 64, 5, 12), [255; 4], "rectangle");
}

#[test]
fn appended_batches_join_only_into_pairs_already_bound() {
    let Some(gpu) = gpu() else { return };
    let mut renderer = TextRenderer::new(&gpu.device, &[FORMAT], AtlasOptions::default());
    let target = TextTarget {
        origin_px: (0, 0),
        extent: (32, 16),
    };
    let run = |glyphs| {
        TextDraw::Glyphs(GlyphRunDraw {
            font: 0,
            ppem: 16.0,
            color: [1.0, 1.0, 1.0, 1.0],
            clip: None,
            glyphs,
        })
    };
    let (mask, colour) = ([glyph(1, 2.0, 10.0)], [glyph(2, 20.0, 10.0)]);
    renderer.begin_frame(0);
    let mut masks = renderer.prepare(&gpu.device, &gpu.queue, &Blocks, target, &[run(&mask)]);
    let more_masks = renderer.prepare(&gpu.device, &gpu.queue, &Blocks, target, &[run(&mask)]);
    let colours = renderer.prepare(&gpu.device, &gpu.queue, &Blocks, target, &[run(&colour)]);
    masks.append(more_masks);
    assert_eq!(masks.draws.len(), 1, "the same pages");
    // A coverage page alone, then a colour page alone: binding both is a
    // pair no prepared draw has, so they stay two.
    masks.append(colours);
    assert_eq!(masks.draws.len(), 2);
    let pixels = gpu.draw(&mut renderer, target.extent, &[masks]);
    assert_eq!(pixel(&pixels, 32, 2, 5), [255; 4]);
    assert_eq!(pixel(&pixels, 32, 20, 9), [0, 0, 255, 255]);
}
