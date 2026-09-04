//! Text: resolving typefaces, building and caching text blobs, and drawing
//! glyph runs and their decorations.

use skia_safe::{
    Canvas, ClipOp, Font, FontMgr, FontStyle as SkFontStyle, GlyphId as SkGlyphId,
    Point as SkPoint, TextBlob, TextBlobBuilder, Typeface as SkTypeface,
};
use std::fs::File;
use xui::text::{TextHost, TextLayoutHandle};
use xui_interface::{
    Affine, FontDataRef, FontDatabase, FontWeight, NodeId, ParagraphLayout, Rect, Shaper,
    TextBackend, TextVerticalAlign,
};

use super::{
    SkiaBackend,
    convert::{sk_bounds, sk_matrix, sk_rect},
    paint::{alpha_color, solid_paint},
};
use crate::{SkiaBackendError, text::sk_font_style};

/// Only `draw_glyphs`, which is itself test-only, needs this.
#[cfg(test)]
use xui_interface::Color;

pub(super) struct CachedTextBlob {
    blob: Option<TextBlob>,
    font_epoch: u64,
    pub(super) owner: NodeId,
    pub(super) last_used_frame: u64,
}

impl<T: TextBackend> SkiaBackend<T> {
    pub(super) fn draw_text(
        &mut self,
        canvas: &Canvas,
        primitive: &xui::render::TextPrimitive,
        transform: Affine,
        opacity: f32,
        text: &mut TextHost<T>,
    ) -> Result<(), SkiaBackendError> {
        let Some(handle) = text.active_slot(primitive.node_id, primitive.slot) else {
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

        let color = alpha_color(primitive.paint.style.color, opacity);
        if let Some(blob) =
            self.text_blob_for_layout(text.backend(), handle, primitive.node_id, &layout)?
        {
            canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
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
        draw_text_decorations(canvas, &layout.lines, primitive, origin, opacity);
        canvas.restore_to_count(save);
        Ok(())
    }

    fn load_font_from_path(
        &mut self,
        path: &std::path::Path,
        index: u32,
    ) -> std::io::Result<Option<SkTypeface>> {
        let file = File::open(path)?;
        let f = unsafe { memmap2::Mmap::map(&file) }?;
        Ok(self.load_font_from_bytes(&f, index))
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
        font_id: <T as FontDatabase>::FontId,
        font_weight: FontWeight,
    ) -> Result<SkTypeface, SkiaBackendError> {
        let epoch = backend.epoch();
        if self.font_cache_epoch != Some(epoch) {
            self.font_cache.clear();
            self.font_cache_epoch = Some(epoch);
        }
        let cache_key = (font_id, font_weight);
        if let Some(typeface) = self.font_cache.get(&cache_key) {
            return Ok(typeface.clone());
        }

        let font_data = backend.font_data(font_id).ok_or_else(|| {
            SkiaBackendError::FontDataError("the shaper did not expose data for a run font".into())
        })?;
        let typeface = match font_data {
            FontDataRef::Bytes { bytes, index } => self
                .load_font_from_bytes(bytes, index)
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not load font bytes at collection index {index}"
                    ))
                })?,
            FontDataRef::SystemMemory {
                bytes,
                index,
                family,
                postscript_name,
                style,
                stretch,
                ..
            } => self
                .system_typeface(
                    family,
                    postscript_name,
                    sk_font_style(font_weight, stretch, style),
                )
                .or_else(|| self.load_font_from_bytes(bytes, index))
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not resolve system font {family} ({postscript_name}) from in-memory collection index {index}"
                    ))
                })?,
            FontDataRef::System {
                path,
                index,
                family,
                postscript_name,
                style,
                stretch,
                ..
            } => self
                .system_typeface(
                    family,
                    postscript_name,
                    sk_font_style(font_weight, stretch, style),
                )
                .or_else(|| self.load_font_from_path(path, index).ok().flatten())
                .ok_or_else(|| {
                    SkiaBackendError::FontDataError(format!(
                        "Skia could not resolve system font {family} ({postscript_name}) from {} at collection index {index}",
                        path.display()
                    ))
                })?,
        };
        self.font_cache.insert(cache_key, typeface.clone());
        Ok(typeface)
    }

    fn build_text_blob(
        &mut self,
        backend: &T,
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let mut builder = TextBlobBuilder::new();
        let mut has_glyphs = false;
        for run in &layout.runs {
            let glyphs = layout.glyphs.get(run.glyph_range.clone()).ok_or_else(|| {
                SkiaBackendError::InvalidFrame(format!(
                    "text run glyph range {:?} exceeds {} glyphs",
                    run.glyph_range,
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
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
    ) -> Result<Option<TextBlob>, SkiaBackendError> {
        let font_epoch = backend.epoch();
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
                font_epoch,
                owner,
                last_used_frame: self.frame_index,
            },
        );
        Ok(blob)
    }

    #[cfg(test)]
    fn draw_glyphs(
        &mut self,
        backend: &T,
        canvas: &Canvas,
        layout: &ParagraphLayout<<T as Shaper>::FontId, <T as Shaper>::GlyphKey>,
        origin: xui_interface::Point,
        color: Color,
    ) -> Result<(), SkiaBackendError> {
        if let Some(blob) = self.build_text_blob(backend, layout)? {
            canvas.draw_text_blob(blob, (origin.x, origin.y), &solid_paint(color));
        }
        Ok(())
    }
}

fn draw_text_decorations(
    canvas: &Canvas,
    lines: &[xui_interface::LineLayout],
    primitive: &xui::render::TextPrimitive,
    origin: xui_interface::Point,
    opacity: f32,
) {
    let decoration = primitive.paint.style.decoration;
    if !decoration.underline && !decoration.line_through {
        return;
    }
    let mut paint = solid_paint(alpha_color(primitive.paint.style.color, opacity));
    paint.set_stroke_width((primitive.paint.style.font_size / 16.0).max(1.0));
    for line in lines {
        if decoration.underline {
            let y = origin.y + line.baseline + paint.stroke_width();
            canvas.draw_line(
                (origin.x + line.x, y),
                (origin.x + line.x + line.width, y),
                &paint,
            );
        }
        if decoration.line_through {
            let y = origin.y + line.y + line.height * 0.5;
            canvas.draw_line(
                (origin.x + line.x, y),
                (origin.x + line.x + line.width, y),
                &paint,
            );
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
}
