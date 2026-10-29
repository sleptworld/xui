//! Text: resolving typefaces, building and caching text blobs, and drawing
//! glyph runs and their decorations.

use skia_safe::{
    Canvas, ClipOp, Font, FontMgr, FontStyle as SkFontStyle, GlyphId as SkGlyphId,
    Point as SkPoint, TextBlob, TextBlobBuilder, Typeface as SkTypeface,
};
use std::ops::Range;
use xui_core::text::{TextHost, TextLayoutHandle};

use xui_interface::{
    Affine, Color, FontCollection, FontData, FontWeight, NodeId, PaintRuns, ParagraphLayout, Rect,
    TextBackend, TextPaintProps, TextVerticalAlign,
};

use super::{
    SkiaBackend,
    convert::{sk_bounds, sk_matrix, sk_rect},
    paint::{alpha_color, solid_paint},
};
use crate::{SkiaBackendError, text::sk_font_style};

pub(super) struct CachedTextBlob {
    /// Every glyph, for text without paint runs.
    blob: Option<TextBlob>,
    /// The glyphs grouped by color, one blob per color, with the paint they
    /// were grouped by: the runs and the default color.
    colored: Option<(PaintRuns, Color, Vec<(Color, TextBlob)>)>,
    font_epoch: u64,
    pub(super) owner: NodeId,
    pub(super) last_used_frame: u64,
}

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn draw_text(
        &mut self,
        canvas: &Canvas,
        primitive: &xui_core::render::TextPrimitive,
        transform: Affine,
        opacity: f32,
        text: &mut TextHost<T>,
    ) -> Result<(), SkiaBackendError> {
        let Some(handle) = text.active_layout(primitive.node_id, primitive.unit) else {
            return Err(SkiaBackendError::InvalidFrame(
                "text primitive has no active layout".into(),
            ));
        };
        let Some(layout) = text.layout(handle) else {
            return Err(SkiaBackendError::InvalidFrame(
                "text primitive layout is not resident".into(),
            ));
        };
        let save = canvas.save();
        canvas.concat(&sk_matrix(transform));
        canvas.clip_rect(sk_bounds(primitive.bounds), ClipOp::Intersect, true);
        let y_offset = match primitive.vertical_align {
            TextVerticalAlign::Top | TextVerticalAlign::Baseline => 0.0,
            TextVerticalAlign::Middle => {
                ((primitive.bounds.height() - layout.size().height) * 0.5).max(0.0)
            }
            TextVerticalAlign::Bottom => {
                (primitive.bounds.height() - layout.size().height).max(0.0)
            }
        };
        let origin =
            xui_interface::Point::new(primitive.bounds.x(), primitive.bounds.y() + y_offset);

        if let Some(selection) = primitive.paint.selection
            && let Some(query) = text.query(handle)
        {
            let mut paint = solid_paint(alpha_color(selection.color, opacity));
            paint.set_anti_alias(false);
            for rect in query.selection_rects(selection.range) {
                canvas.draw_rect(sk_rect(rect.translate(origin)), &paint);
            }
        }

        if primitive.paint.runs.is_empty() {
            let color = alpha_color(primitive.paint.style.color, opacity);
            if let Some(blob) =
                self.text_blob_for_layout(text.backend(), handle, primitive.node_id, &layout)?
            {
                canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
            }
        } else {
            let blobs = self.colored_text_blobs(
                text.backend(),
                handle,
                primitive.node_id,
                &layout,
                &primitive.paint,
            )?;
            for (color, blob) in blobs {
                let paint = solid_paint(alpha_color(color, opacity));
                canvas.draw_text_blob(blob, (origin.x, origin.y), &paint);
            }
        }

        if let Some(caret) = primitive.paint.caret {
            let rect = text
                .query(handle)
                .and_then(|query| query.caret_rect(caret.char_index))
                .unwrap_or(Rect::new(
                    layout.size().width,
                    0.0,
                    caret.width,
                    primitive.paint.style.font_size * 1.2,
                ));
            let mut paint = solid_paint(alpha_color(caret.color, opacity));
            paint.set_stroke_width(caret.width.max(1.0));
            canvas.draw_line(
                (origin.x + rect.x, origin.y + rect.y),
                (origin.x + rect.x, origin.y + rect.y + rect.height),
                &paint,
            );
        }
        if let Some(ime) = primitive.paint.ime
            && let Some(query) = text.query(handle)
        {
            let mut paint = solid_paint(alpha_color(ime.underline_color, opacity));
            paint.set_stroke_width(ime.underline_width.max(1.0));
            for rect in query.selection_rects(ime.range) {
                let y = origin.y + rect.y + rect.height;
                canvas.draw_line(
                    (origin.x + rect.x, y),
                    (origin.x + rect.x + rect.width, y),
                    &paint,
                );
            }
        }
        draw_text_decorations(canvas, &layout, primitive, origin, opacity);
        canvas.restore_to_count(save);
        Ok(())
    }

    fn load_font_from_bytes(&mut self, bytes: &[u8], index: u32) -> Option<SkTypeface> {
        self.font_mgr().new_from_data(bytes, Some(index as usize))
    }

    fn system_typeface(
        &mut self,
        family: &str,
        postscript_name: &str,
        style: SkFontStyle,
    ) -> Option<SkTypeface> {
        let mut styles = self.font_mgr().match_family(family);
        for index in 0..styles.count() {
            let Some(typeface) = styles.new_typeface(index) else {
                continue;
            };
            if typeface
                .post_script_name()
                .is_some_and(|name| name == postscript_name)
            {
                return Some(typeface);
            }
        }
        self.font_mgr().match_family_style(family, style)
    }

    /// The process-wide font manager, built on first use.
    ///
    /// `FontMgr::new()` enumerates CoreText on macOS and costs tens of
    /// milliseconds; building one per typeface cache miss put 300 ms into the
    /// first frame of a text-heavy window.
    pub(super) fn font_mgr(&mut self) -> &FontMgr {
        self.font_mgr.get_or_insert_with(FontMgr::new)
    }

    fn typeface_for_font(
        &mut self,
        backend: &T,
        font_id: T::FontId,
        font_weight: FontWeight,
    ) -> Result<SkTypeface, SkiaBackendError> {
        let fonts = backend.fonts();
        let epoch = fonts.epoch();
        if self.font_cache_epoch != Some(epoch) {
            self.font_cache.clear();
            self.font_cache_epoch = Some(epoch);
        }
        let cache_key = (font_id, font_weight);
        if let Some(typeface) = self.font_cache.get(&cache_key) {
            return Ok(typeface.clone());
        }

        let font = fonts.font(font_id).ok_or_else(|| {
            SkiaBackendError::FontDataError("the shaper did not expose data for a run font".into())
        })?;
        let index = font.index();
        let typeface = match font.system() {
            // The native font manager knows installed faces best (and some
            // system collections cannot be rebuilt from their bytes).
            Some(system) => self
                .system_typeface(
                    system.family,
                    system.postscript_name,
                    sk_font_style(font_weight, system.stretch, system.style),
                )
                .or_else(|| self.load_font_from_bytes(font.bytes(), index))
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not resolve system font {} ({}) at collection index {index}",
                        system.family, system.postscript_name
                    ))
                })?,
            None => self
                .load_font_from_bytes(font.bytes(), index)
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not load font bytes at collection index {index}"
                    ))
                })?,
        };
        self.font_cache.insert(cache_key, typeface.clone());
        Ok(typeface)
    }

    fn build_text_blob(
        &mut self,
        backend: &T,
        layout: &ParagraphLayout<T::FontId>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let pieces = layout
            .runs
            .iter()
            .enumerate()
            .map(|(index, run)| (index, run.glyph_range.clone()));
        self.build_text_blob_from(backend, layout, pieces)
    }

    /// A blob of `pieces`: each is a glyph run, by index, and the range of
    /// its glyphs to draw.
    fn build_text_blob_from(
        &mut self,
        backend: &T,
        layout: &ParagraphLayout<T::FontId>,
        pieces: impl IntoIterator<Item = (usize, Range<usize>)>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let mut builder = TextBlobBuilder::new();
        let mut has_glyphs = false;
        for (run_index, glyph_range) in pieces {
            let run = &layout.runs[run_index];
            let glyphs = layout.glyphs.get(glyph_range.clone()).ok_or_else(|| {
                SkiaBackendError::InvalidFrame(format!(
                    "text run glyph range {:?} exceeds {} glyphs",
                    glyph_range,
                    layout.glyphs.len()
                ))
            })?;
            if glyphs.is_empty() {
                continue;
            }

            let typeface = self.typeface_for_font(backend, run.font_id, run.font_weight)?;
            let mut font = Font::from_typeface(typeface, Some(run.font_size.max(1.0)));
            font.set_subpixel(true);
            font.set_edging(skia_safe::font::Edging::SubpixelAntiAlias);
            let (glyph_ids, positions) = builder.alloc_run_pos(&font, glyphs.len(), None);
            for ((glyph_id, position), glyph) in
                glyph_ids.iter_mut().zip(positions.iter_mut()).zip(glyphs)
            {
                *glyph_id = SkGlyphId::try_from(glyph.glyph_id).map_err(|_| {
                    SkiaBackendError::InvalidFrame(format!(
                        "glyph id {} cannot be represented by Skia",
                        glyph.glyph_id
                    ))
                })?;
                *position = SkPoint::new(glyph.draw_pos.x, glyph.draw_pos.y);
            }
            has_glyphs = true;
        }
        Ok(has_glyphs.then(|| builder.make()).flatten())
    }

    fn text_blob_for_layout(
        &mut self,
        backend: &T,
        handle: TextLayoutHandle,
        owner: NodeId,
        layout: &ParagraphLayout<T::FontId>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let font_epoch = backend.fonts().epoch();
        if let Some(cached) = self.text_blob_cache.get_mut(&handle)
            && cached.font_epoch == font_epoch
        {
            cached.last_used_frame = self.frame_index;
            return Ok(cached.blob.clone());
        }
        let blob = self.build_text_blob(backend, layout)?;
        self.text_blob_cache.insert(
            handle,
            CachedTextBlob {
                blob: blob.clone(),
                colored: None,
                font_epoch,
                owner,
                last_used_frame: self.frame_index,
            },
        );
        Ok(blob)
    }

    /// `layout`'s glyphs as one blob per color of `paint`, cached with the
    /// layout until the layout, the fonts or the paint runs change.
    fn colored_text_blobs(
        &mut self,
        backend: &T,
        handle: TextLayoutHandle,
        owner: NodeId,
        layout: &ParagraphLayout<T::FontId>,
        paint: &TextPaintProps,
    ) -> Result<Vec<(Color, TextBlob)>, SkiaBackendError> {
        let font_epoch = backend.fonts().epoch();
        if let Some(cached) = self.text_blob_cache.get_mut(&handle)
            && cached.font_epoch == font_epoch
            && let Some((runs, default, blobs)) = &cached.colored
            && *runs == paint.runs
            && *default == paint.style.color
        {
            cached.last_used_frame = self.frame_index;
            return Ok(blobs.clone());
        }
        let mut groups: Vec<(Color, Vec<(usize, Range<usize>)>)> = Vec::new();
        for span in paint.glyph_spans(layout) {
            match groups.iter_mut().find(|(color, _)| *color == span.color) {
                Some((_, pieces)) => pieces.push((span.run, span.glyphs)),
                None => groups.push((span.color, vec![(span.run, span.glyphs)])),
            }
        }
        let mut blobs = Vec::with_capacity(groups.len());
        for (color, pieces) in groups {
            if let Some(blob) = self.build_text_blob_from(backend, layout, pieces)? {
                blobs.push((color, blob));
            }
        }
        let cached = self
            .text_blob_cache
            .entry(handle)
            .or_insert(CachedTextBlob {
                blob: None,
                colored: None,
                font_epoch,
                owner,
                last_used_frame: self.frame_index,
            });
        if cached.font_epoch != font_epoch {
            cached.blob = None;
            cached.font_epoch = font_epoch;
        }
        cached.colored = Some((paint.runs.clone(), paint.style.color, blobs.clone()));
        cached.last_used_frame = self.frame_index;
        Ok(blobs)
    }

    #[cfg(test)]
    fn draw_glyphs(
        &mut self,
        backend: &T,
        canvas: &Canvas,
        layout: &ParagraphLayout<T::FontId>,
        origin: xui_interface::Point,
        color: Color,
    ) -> Result<(), SkiaBackendError> {
        if let Some(blob) = self.build_text_blob(backend, layout)? {
            canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
        }
        Ok(())
    }
}

fn draw_text_decorations<F>(
    canvas: &Canvas,
    layout: &ParagraphLayout<F>,
    primitive: &xui_core::render::TextPrimitive,
    origin: xui_interface::Point,
    opacity: f32,
) {
    let width = (primitive.paint.style.font_size / 16.0).max(1.0);
    for span in primitive.paint.decoration_spans(layout) {
        let line = &layout.lines[span.line];
        let mut paint = solid_paint(alpha_color(span.color, opacity));
        paint.set_stroke_width(width);
        let (x0, x1) = (origin.x + span.x.start, origin.x + span.x.end);
        if span.decoration.underline {
            let y = origin.y + line.baseline + width;
            canvas.draw_line((x0, y), (x1, y), &paint);
        }
        if span.decoration.line_through {
            let y = origin.y + line.y + line.height * 0.5;
            canvas.draw_line((x0, y), (x1, y), &paint);
        }
    }
}

#[cfg(test)]
mod text_draw_tests {
    use skia_safe::{AlphaType, ColorSpace, ColorType, ImageInfo};
    use xui_cosmic::CosmicEngine;
    use xui_interface::{
        FontWeight, ParagraphStyle, Shaper, TextBoxStyle, TextContent, TextLayoutConstraints,
        TextLayoutInput, TextStyle,
    };

    use super::*;
    use crate::SkiaBackendOptions;

    #[test]
    fn draws_shaper_output_without_backend_rasterization() {
        let mut text_backend = CosmicEngine::new(1.0);
        let mut state = text_backend.create_state();
        let layout = text_backend.layout_paragraph(
            &mut state,
            TextLayoutInput::new(
                TextContent::from_static("Skia draw_glyphs 啊，是关中王来啦"),
                TextLayoutConstraints::max_width(240.0),
                TextStyle {
                    font_weight: FontWeight::Thin,
                    ..TextStyle::default()
                }
                .into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                0,
            ),
        );
        assert!(!layout.runs.is_empty());

        let mut renderer =
            SkiaBackend::<CosmicEngine>::headless(1.0, SkiaBackendOptions::default());
        let mut surface = skia_safe::surfaces::raster_n32_premul((240, 80)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        renderer
            .draw_glyphs(
                &text_backend,
                surface.canvas(),
                &layout,
                xui_interface::Point::new(0.0, 0.0),
                Color::WHITE,
            )
            .unwrap();

        let mut pixels = vec![0; 240 * 80 * 4];
        let info = ImageInfo::new(
            (240, 80),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        assert!(surface.read_pixels(&info, &mut pixels, 240 * 4, (0, 0)));
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
        assert!(!renderer.font_cache.is_empty());
    }

    #[test]
    fn paint_runs_color_their_glyphs_and_nothing_else() {
        use xui_core::render::TextPrimitive;
        use xui_core::text::TextLayoutSlot;
        use xui_interface::{
            Bounds, NodeId, PaintRun, PaintRuns, TextDecoration, TextPaintProps, TextPaintStyle,
        };

        let (width, height) = (240, 60);
        let blue = Color::rgba(0.0, 0.0, 1.0, 1.0);
        let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
        let mut host = TextHost::new(CosmicEngine::new(1.0));
        let node = NodeId::default();
        let text = "MMMM MMMM";
        let style = TextStyle {
            font_size: 24.0,
            font_weight: FontWeight::Bold,
            ..TextStyle::default()
        };
        let font_context = host.font_epoch();
        host.activate_slot(
            node,
            TextLayoutSlot::PRIMARY,
            TextLayoutInput::new(
                TextContent::from_static(text),
                TextLayoutConstraints::max_width(width as f32),
                style.clone().into(),
                ParagraphStyle::default(),
                TextBoxStyle::default(),
                font_context,
            ),
        );
        let primitive = TextPrimitive {
            bounds: Bounds::from_origin_size((0.0, 0.0), (width as f32, height as f32)),
            node_id: node,
            unit: TextLayoutSlot::PRIMARY.into(),
            layout_revision: 0,
            vertical_align: TextVerticalAlign::Top,
            paint: TextPaintProps::new(TextPaintStyle {
                color: blue,
                font_size: style.font_size,
                line_height: style.line_height,
                decoration: TextDecoration::default(),
            })
            .with_runs(PaintRuns::new(vec![PaintRun {
                range: 0..4,
                color: red,
                decoration: TextDecoration::default(),
            }])),
        };

        let mut renderer =
            SkiaBackend::<CosmicEngine>::headless(1.0, SkiaBackendOptions::default());
        let mut surface = skia_safe::surfaces::raster_n32_premul((width, height)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        renderer
            .draw_text(
                surface.canvas(),
                &primitive,
                Affine::IDENTITY,
                1.0,
                &mut host,
            )
            .unwrap();

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            ColorSpace::new_srgb(),
        );
        assert!(surface.read_pixels(&info, &mut pixels, width as usize * 4, (0, 0)));

        // Where each word ends and starts, from the layout.
        let layout = host
            .layout(host.active_slot(node, TextLayoutSlot::PRIMARY).unwrap())
            .unwrap();
        let first_end = layout
            .clusters
            .iter()
            .filter(|cluster| cluster.text_range.start.offset() < 4)
            .map(|cluster| cluster.hitbox.x + cluster.hitbox.width)
            .fold(0.0, f32::max);
        let second_start = layout
            .clusters
            .iter()
            .filter(|cluster| cluster.text_range.start.offset() > 4)
            .map(|cluster| cluster.hitbox.x)
            .fold(f32::INFINITY, f32::min);
        assert!(first_end < second_start, "{first_end} vs {second_start}");
        let (mut left, mut right) = ([0u32; 2], [0u32; 2]);
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            if pixel[3] < 128 {
                continue;
            }
            let x = (index % width as usize) as f32;
            let side = if x < first_end {
                &mut left
            } else if x >= second_start {
                &mut right
            } else {
                continue;
            };
            side[usize::from(pixel[2] > pixel[0])] += 1;
        }
        // [red-ish, blue-ish] ink on each side of the space.
        assert!(left[0] > 0 && left[1] == 0, "left word: {left:?}");
        assert!(right[1] > 0 && right[0] == 0, "right word: {right:?}");
    }
}
