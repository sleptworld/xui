//! Text: the shaper decides which glyphs go where; glyphs are drawn as bitmaps
//! by `xui-wgpu-text`, or as outlines by Vello.
//!
//! Layout and shaping stay with the `TextBackend` (cosmic-text, or xui-f's
//! HarfRust), so system fonts are found exactly as for the other backends.
//!
//! When the backend has a glyph rasterizer, text whose transform is a
//! translation and a uniform scale, cut by nothing but rectangles, is drawn
//! from its bitmaps: [`bitmap_parts`] lays its glyphs, and the caret, IME
//! underline and decorations in front of them, out in target pixels for
//! `xui-wgpu-text`, while its selection, behind them, stays a Vello draw.
//! Everything else goes through [`encode_text`]: the same paragraph as
//! `Scene::draw_glyphs` runs, rasterized by Vello itself (through skrifa).

use std::{hash::Hash, sync::Arc};

use vello::{
    Glyph, Scene,
    kurbo::{Affine as KurboAffine, Line, Rect as KurboRect, Stroke},
    peniko::{Blob, Fill, FontData, color::{AlphaColor, Srgb}},
};
use xui_core::{
    render::TextPrimitive,
    text::{TextHost, TextLayoutHandle},
};
use crate::lru::ByteLru;
use xui_interface::{
    Color, FontCollection, FontData as XuiFontData, LineLayout, ParagraphLayout, Point, Rect,
    TextBackend, TextVerticalAlign,
};
use xui_wgpu_text::{GlyphRunDraw, PositionedGlyph, RectDraw, TextDraw};

/// Font bytes, shared with Vello once per font.
///
/// Vello keys its glyph caches on the blob's identity, so a font must keep the
/// same `FontData` for as long as it is in use; rebuilding it per frame would
/// re-upload every outline. A font is the whole file (a CJK face is tens of
/// megabytes), so the cache is bounded by those bytes, least recently used out
/// first. A font that failed to load is remembered as `None`, so it is not
/// retried every frame.
pub(crate) struct FontCache<I> {
    epoch: Option<u64>,
    fonts: ByteLru<I, Option<FontData>>,
}

impl<I: Copy + Eq + Hash> FontCache<I> {
    pub(crate) fn new(budget_bytes: u64) -> Self {
        Self {
            epoch: None,
            fonts: ByteLru::new(budget_bytes),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.fonts.len()
    }

    pub(crate) fn bytes(&self) -> u64 {
        self.fonts.bytes()
    }

    fn resolve<C: FontCollection<FontId = I>>(&mut self, fonts: &C, id: I) -> Option<FontData> {
        let epoch = fonts.epoch();
        if self.epoch != Some(epoch) {
            self.fonts.clear();
            self.epoch = Some(epoch);
        }
        self.fonts.get_or_insert_with(id, || {
            let font = fonts.font(id).map(|data| {
                let index = data.index();
                FontData::new(Blob::new(Arc::new(SharedBytes(data))), index)
            });
            let bytes = font.as_ref().map_or(1, |font| font.data.len() as u64);
            (font, bytes)
        })
    }
}

/// The backend's font handle as a peniko blob, so the file is shared rather
/// than copied.
struct SharedBytes<D>(D);

impl<D: XuiFontData> AsRef<[u8]> for SharedBytes<D> {
    fn as_ref(&self) -> &[u8] {
        self.0.bytes()
    }
}

/// What the text path could not do, folded into [`crate::Unsupported`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TextGaps {
    /// A run's font could not be loaded; its glyphs were skipped.
    pub missing_font: bool,
}

/// Physical pixels per em past which glyphs are drawn as outlines: a bitmap
/// that large would take most of an atlas page.
pub(crate) const MAX_BITMAP_PPEM: f32 = 512.0;

/// A text primitive's laid-out paragraph and where it sits, in the
/// primitive's logical space.
pub(crate) struct Paragraph<F> {
    handle: TextLayoutHandle,
    pub layout: Arc<ParagraphLayout<F>>,
    origin: Point,
}

pub(crate) fn paragraph<T: TextBackend>(
    text: &TextHost<T>,
    primitive: &TextPrimitive,
) -> Option<Paragraph<T::FontId>> {
    let handle = text.active_slot(primitive.node_id, primitive.slot)?;
    let layout = text.layout(handle)?;
    let bounds = primitive.bounds;
    let y_offset = match primitive.vertical_align {
        TextVerticalAlign::Top | TextVerticalAlign::Baseline => 0.0,
        TextVerticalAlign::Middle => ((bounds.height() - layout.size().height) * 0.5).max(0.0),
        TextVerticalAlign::Bottom => (bounds.height() - layout.size().height).max(0.0),
    };
    Some(Paragraph {
        handle,
        origin: Point::new(bounds.x(), bounds.y() + y_offset),
        layout,
    })
}

/// A line stroked in front of the glyphs: the caret, an IME underline, a
/// decoration.
struct ForegroundLine {
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    ink: Color,
}

/// Whether `primitive` has a selection to draw behind its glyphs.
pub(crate) fn has_selection(primitive: &TextPrimitive) -> bool {
    primitive.paint.selection.is_some()
}

/// The selection behind the glyphs, clipped to the primitive's bounds.
pub(crate) fn encode_selection<T: TextBackend>(
    scene: &mut Scene,
    world: KurboAffine,
    primitive: &TextPrimitive,
    text: &TextHost<T>,
    paragraph: &Paragraph<T::FontId>,
) {
    let (Some(selection), Some(query)) = (primitive.paint.selection, text.query(paragraph.handle))
    else {
        return;
    };
    scene.push_clip_layer(Fill::NonZero, world, &bounds_rect(primitive));
    for rect in query.selection_rects(selection.range) {
        fill_rect(scene, world, rect, paragraph.origin, selection.color);
    }
    scene.pop_layer();
}

/// The caret, IME underline and decorations, in the order they are drawn.
fn foreground<T: TextBackend>(
    primitive: &TextPrimitive,
    text: &TextHost<T>,
    paragraph: &Paragraph<T::FontId>,
) -> Vec<ForegroundLine> {
    let mut strokes = Vec::new();
    let (paint, origin, layout) = (&primitive.paint, paragraph.origin, &paragraph.layout);
    let query = text.query(paragraph.handle);
    if let Some(caret) = paint.caret {
        let rect = query
            .and_then(|query| query.caret_rect(caret.char_index))
            .unwrap_or(Rect::new(
                layout.size().width,
                0.0,
                caret.width,
                paint.style.font_size * 1.2,
            ));
        let x = origin.x + rect.x;
        strokes.push(ForegroundLine {
            from: (x, origin.y + rect.y),
            to: (x, origin.y + rect.y + rect.height),
            width: caret.width.max(1.0),
            ink: caret.color,
        });
    }
    if let (Some(ime), Some(query)) = (paint.ime, query) {
        for rect in query.selection_rects(ime.range) {
            let y = origin.y + rect.y + rect.height;
            strokes.push(ForegroundLine {
                from: (origin.x + rect.x, y),
                to: (origin.x + rect.x + rect.width, y),
                width: ime.underline_width.max(1.0),
                ink: ime.underline_color,
            });
        }
    }
    decorations(&mut strokes, &layout.lines, primitive, origin);
    strokes
}

/// Everything `xui-wgpu-text` draws of a text primitive, in front of its
/// selection, in target pixels. `world` takes the primitive's logical space
/// to target pixels and is a translation and a uniform scale; `clip` is in
/// target pixels.
pub(crate) struct BitmapParts<F> {
    runs: Vec<(F, f32, Vec<PositionedGlyph>)>,
    color: [f32; 4],
    rects: Vec<RectDraw>,
    clip: [f32; 4],
}

impl<F: Copy> BitmapParts<F> {
    pub fn draws(&self) -> Vec<TextDraw<'_, F>> {
        let glyphs = self.runs.iter().map(|(font, ppem, glyphs)| {
            TextDraw::Glyphs(GlyphRunDraw {
                font: *font,
                ppem: *ppem,
                color: self.color,
                clip: Some(self.clip),
                glyphs,
            })
        });
        glyphs
            .chain(self.rects.iter().copied().map(TextDraw::Rect))
            .collect()
    }
}

pub(crate) fn bitmap_parts<T: TextBackend>(
    world: KurboAffine,
    primitive: &TextPrimitive,
    text: &TextHost<T>,
    paragraph: &Paragraph<T::FontId>,
    clip: [f32; 4],
) -> BitmapParts<T::FontId> {
    let [scale, _, _, _, dx, dy] = world.as_coeffs().map(|value| value as f32);
    let px = |x: f32, y: f32| (x * scale + dx, y * scale + dy);
    let paint = &primitive.paint;
    let (layout, origin) = (&paragraph.layout, paragraph.origin);
    let runs = if paint.style.color.a > 0.0 {
        layout
            .runs
            .iter()
            .map(|run| {
                let glyphs = layout.glyphs[run.glyph_range.clone()]
                    .iter()
                    .map(|glyph| {
                        let (x, y) = px(origin.x + glyph.draw_pos.x, origin.y + glyph.draw_pos.y);
                        PositionedGlyph {
                            id: glyph.glyph_id,
                            x,
                            y,
                        }
                    })
                    .collect();
                (run.font_id, run.font_size * scale, glyphs)
            })
            .collect()
    } else {
        Vec::new()
    };
    let rects = foreground(primitive, text, paragraph)
        .into_iter()
        .map(|stroke| {
            // A stroke along an axis is a rectangle `width` across it.
            let half = stroke.width * 0.5;
            let (x0, x1) = (stroke.from.0.min(stroke.to.0), stroke.from.0.max(stroke.to.0));
            let (y0, y1) = (stroke.from.1.min(stroke.to.1), stroke.from.1.max(stroke.to.1));
            let [x0, y0, x1, y1] = if y0 == y1 {
                [x0, y0 - half, x1, y1 + half]
            } else {
                [x0 - half, y0, x1 + half, y1]
            };
            let (x0, y0) = px(x0, y0);
            let (x1, y1) = px(x1, y1);
            RectDraw {
                rect: [x0, y0, x1, y1],
                color: color_array(stroke.ink),
                clip: Some(clip),
            }
        })
        .collect();
    BitmapParts {
        runs,
        color: color_array(paint.style.color),
        rects,
        clip,
    }
}

/// A text primitive as Vello draws it: selection, glyph outlines, and the
/// strokes in front of them.
pub(crate) fn encode_text<T: TextBackend>(
    scene: &mut Scene,
    world: KurboAffine,
    primitive: &TextPrimitive,
    text: &TextHost<T>,
    fonts: &mut FontCache<T::FontId>,
) -> TextGaps {
    let mut gaps = TextGaps::default();
    let Some(paragraph) = paragraph(text, primitive) else {
        return gaps;
    };
    encode_selection(scene, world, primitive, text, &paragraph);
    scene.push_clip_layer(Fill::NonZero, world, &bounds_rect(primitive));

    let (paint, origin, layout) = (&primitive.paint, paragraph.origin, &paragraph.layout);
    if paint.style.color.a > 0.0 {
        let brush = color(paint.style.color);
        for run in &layout.runs {
            let Some(font) = fonts.resolve(&**text.backend().fonts(), run.font_id) else {
                gaps.missing_font = true;
                continue;
            };
            let glyphs = layout.glyphs[run.glyph_range.clone()].iter().map(|glyph| Glyph {
                id: glyph.glyph_id,
                x: origin.x + glyph.draw_pos.x,
                y: origin.y + glyph.draw_pos.y,
            });
            // Positions come from the shaper already, so hinting only snaps
            // outlines. Measured against Skia it is the closer of the two.
            scene
                .draw_glyphs(&font)
                .font_size(run.font_size)
                .transform(world)
                .hint(true)
                .brush(brush)
                .draw(Fill::NonZero, glyphs);
        }
    }
    for stroke in foreground(primitive, text, &paragraph) {
        stroke_line(scene, world, stroke.from, stroke.to, stroke.width, stroke.ink);
    }

    scene.pop_layer();
    gaps
}

fn bounds_rect(primitive: &TextPrimitive) -> KurboRect {
    let bounds = primitive.bounds;
    KurboRect::new(
        bounds.min.x as f64,
        bounds.min.y as f64,
        bounds.max.x as f64,
        bounds.max.y as f64,
    )
}

fn decorations(
    strokes: &mut Vec<ForegroundLine>,
    lines: &[LineLayout],
    primitive: &TextPrimitive,
    origin: Point,
) {
    let decoration = primitive.paint.style.decoration;
    if !decoration.underline && !decoration.line_through {
        return;
    }
    let width = (primitive.paint.style.font_size / 16.0).max(1.0);
    let ink = primitive.paint.style.color;
    for line in lines {
        let (x0, x1) = (origin.x + line.x, origin.x + line.x + line.width);
        if decoration.underline {
            let y = origin.y + line.baseline + width;
            strokes.push(ForegroundLine {
                from: (x0, y),
                to: (x1, y),
                width,
                ink,
            });
        }
        if decoration.line_through {
            let y = origin.y + line.y + line.height * 0.5;
            strokes.push(ForegroundLine {
                from: (x0, y),
                to: (x1, y),
                width,
                ink,
            });
        }
    }
}

fn fill_rect(scene: &mut Scene, world: KurboAffine, rect: Rect, origin: Point, ink: Color) {
    let rect = KurboRect::new(
        (origin.x + rect.x) as f64,
        (origin.y + rect.y) as f64,
        (origin.x + rect.x + rect.width) as f64,
        (origin.y + rect.y + rect.height) as f64,
    );
    scene.fill(Fill::NonZero, world, color(ink), None, &rect);
}

fn stroke_line(
    scene: &mut Scene,
    world: KurboAffine,
    from: (f32, f32),
    to: (f32, f32),
    width: f32,
    ink: Color,
) {
    scene.stroke(
        &Stroke::new(width as f64),
        world,
        color(ink),
        None,
        &Line::new((from.0 as f64, from.1 as f64), (to.0 as f64, to.1 as f64)),
    );
}

fn color(value: Color) -> AlphaColor<Srgb> {
    AlphaColor::new(color_array(value))
}

fn color_array(value: Color) -> [f32; 4] {
    [value.r, value.g, value.b, value.a]
}
