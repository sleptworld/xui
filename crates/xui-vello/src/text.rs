//! Text: the shaper decides which glyphs go where, Vello draws their outlines.
//!
//! Layout and shaping stay with the `TextBackend` (cosmic-text, or xui-f's
//! HarfRust), so system fonts are found exactly as for the other backends. This
//! module only turns a laid-out paragraph into `Scene::draw_glyphs` runs, plus
//! the selection, caret, IME underline and decoration rectangles around them.
//!
//! Glyph outlines are rasterized by Vello itself (through skrifa), not by the
//! platform's text stack, so there is no LCD subpixel rendering and glyphs
//! differ a little from what CoreText or DirectWrite would draw. Colour fonts
//! (COLR, bitmap strikes) are handled by Vello.

use std::{hash::Hash, sync::Arc};

use vello::{
    Glyph, Scene,
    kurbo::{Affine as KurboAffine, Line, Rect as KurboRect, Stroke},
    peniko::{Blob, Fill, FontData, color::{AlphaColor, Srgb}},
};
use xui_core::{
    render::TextPrimitive,
    text::TextHost,
};
use crate::lru::ByteLru;
use xui_interface::{
    Color, FontDataRef, FontDatabase, LineLayout, Point, Rect, TextBackend, TextVerticalAlign,
};

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

    fn resolve<D: FontDatabase<FontId = I>>(&mut self, database: &D, id: I) -> Option<FontData> {
        let epoch = database.epoch();
        if self.epoch != Some(epoch) {
            self.fonts.clear();
            self.epoch = Some(epoch);
        }
        self.fonts.get_or_insert_with(id, || {
            let font = load_font(database, id);
            let bytes = font.as_ref().map_or(1, |font| font.data.len() as u64);
            (font, bytes)
        })
    }
}

fn load_font<D: FontDatabase>(database: &D, id: D::FontId) -> Option<FontData> {
    let (bytes, index) = match database.font_data(id)? {
        FontDataRef::Bytes { bytes, index } | FontDataRef::SystemMemory { bytes, index, .. } => {
            (bytes.to_vec(), index)
        }
        FontDataRef::System { path, index, .. } => match std::fs::read(path) {
            Ok(bytes) => (bytes, index),
            Err(error) => {
                log::warn!("xui-vello: cannot read font {}: {error}", path.display());
                return None;
            }
        },
    };
    Some(FontData::new(Blob::new(Arc::new(bytes)), index))
}

/// What the text path could not do, folded into [`crate::Unsupported`].
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TextGaps {
    /// A run's font could not be loaded; its glyphs were skipped.
    pub missing_font: bool,
}

pub(crate) fn encode_text<T: TextBackend>(
    scene: &mut Scene,
    world: KurboAffine,
    primitive: &TextPrimitive,
    text: &TextHost<T>,
    fonts: &mut FontCache<<T as FontDatabase>::FontId>,
) -> TextGaps {
    let mut gaps = TextGaps::default();
    let Some(handle) = text.active_slot(primitive.node_id, primitive.slot) else {
        return gaps;
    };
    let Some(layout) = text.layout(handle) else {
        return gaps;
    };
    let bounds = primitive.bounds;
    scene.push_clip_layer(
        Fill::NonZero,
        world,
        &KurboRect::new(
            bounds.min.x as f64,
            bounds.min.y as f64,
            bounds.max.x as f64,
            bounds.max.y as f64,
        ),
    );

    let y_offset = match primitive.vertical_align {
        TextVerticalAlign::Top | TextVerticalAlign::Baseline => 0.0,
        TextVerticalAlign::Middle => ((bounds.height() - layout.size().height) * 0.5).max(0.0),
        TextVerticalAlign::Bottom => (bounds.height() - layout.size().height).max(0.0),
    };
    let origin = Point::new(bounds.x(), bounds.y() + y_offset);
    let query = text.query(handle);
    let paint = &primitive.paint;

    if let (Some(selection), Some(query)) = (paint.selection, query) {
        for rect in query.selection_rects(selection.range) {
            fill_rect(scene, world, rect, origin, selection.color);
        }
    }

    if paint.style.color.a > 0.0 {
        let brush = color(paint.style.color);
        for run in &layout.runs {
            let Some(font) = fonts.resolve(text.backend(), run.font_id) else {
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
        stroke_line(
            scene,
            world,
            (x, origin.y + rect.y),
            (x, origin.y + rect.y + rect.height),
            caret.width.max(1.0),
            caret.color,
        );
    }
    if let (Some(ime), Some(query)) = (paint.ime, query) {
        for rect in query.selection_rects(ime.range) {
            let y = origin.y + rect.y + rect.height;
            stroke_line(
                scene,
                world,
                (origin.x + rect.x, y),
                (origin.x + rect.x + rect.width, y),
                ime.underline_width.max(1.0),
                ime.underline_color,
            );
        }
    }
    decorations(scene, world, &layout.lines, primitive, origin);

    scene.pop_layer();
    gaps
}

fn decorations(
    scene: &mut Scene,
    world: KurboAffine,
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
            stroke_line(scene, world, (x0, y), (x1, y), width, ink);
        }
        if decoration.line_through {
            let y = origin.y + line.y + line.height * 0.5;
            stroke_line(scene, world, (x0, y), (x1, y), width, ink);
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
    AlphaColor::new([value.r, value.g, value.b, value.a])
}
