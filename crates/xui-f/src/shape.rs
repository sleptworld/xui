use std::{ops::Range, sync::Arc};

use harfrust::Direction;
use smallvec::SmallVec;
use unicode_script::{Script, UnicodeScript as _};
use unicode_segmentation::UnicodeSegmentation;
use xui_interface::{ComputedTextStyle, FontQuery, FontStretch, GlyphFlags, Point};

use crate::{
    FFontId, bidi,
    font::{FaceMetrics, FontStore, ShapeResources, shape_plan},
};

#[derive(Debug, Clone)]
pub(crate) struct ShapedGlyph {
    pub glyph_id: u32,
    pub offset: Point,
    pub advance: f32,
    pub flags: GlyphFlags,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedCluster {
    pub text_range: Range<usize>,
    pub glyph_range: Range<usize>,
    pub advance: f32,
    pub whitespace: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedRun {
    pub text_range: Range<usize>,
    pub font_id: FFontId,
    /// Logical pixels per em.
    pub font_size: f32,
    pub bidi_level: u8,
    pub metrics: FaceMetrics,
    pub glyphs: Vec<ShapedGlyph>,
    pub clusters: Vec<ShapedCluster>,
    pub advance: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ShapedLine {
    pub runs: Vec<ShapedRun>,
    pub width: f32,
    pub base_rtl: bool,
}

#[derive(Debug, Clone)]
struct FontSpan {
    range: Range<usize>,
    font_id: FFontId,
    fallback: bool,
    script: Script,
}

/// The font query a text style asks for and the faces it found, looked up
/// once for every line of a paragraph.
pub(crate) struct LineFonts {
    query: FontQuery,
    candidates: Arc<[FFontId]>,
}

impl LineFonts {
    pub fn new(fonts: &mut FontStore, style: &ComputedTextStyle) -> Self {
        let query = FontQuery {
            families: vec![style.font_family.clone()],
            weight: style.font_weight,
            style: style.font_style,
            stretch: FontStretch::Normal,
        };
        let candidates = fonts.candidates_for(&query);
        Self { query, candidates }
    }

    pub fn candidates(&self) -> &[FFontId] {
        &self.candidates
    }
}

impl ShapedLine {
    /// How many runs, glyphs and clusters the line holds.
    pub fn counts(&self) -> (usize, usize, usize) {
        self.runs
            .iter()
            .fold((0, 0, 0), |(runs, glyphs, clusters), run| {
                (
                    runs + 1,
                    glyphs + run.glyphs.len(),
                    clusters + run.clusters.len(),
                )
            })
    }
}

pub(crate) fn shape_line(
    fonts: &mut FontStore,
    line_fonts: &LineFonts,
    text: &str,
    range: Range<usize>,
    style: &ComputedTextStyle,
    resolver: &bidi::Resolver,
) -> ShapedLine {
    let order = resolver.resolve(range);
    let mut output = ShapedLine {
        runs: Vec::new(),
        width: 0.0,
        base_rtl: order.base_rtl,
    };

    for bidi_run in order.runs {
        let mut spans = font_spans(
            fonts,
            text,
            bidi_run.range,
            &line_fonts.query,
            &line_fonts.candidates,
        );
        if bidi_run.level % 2 == 1 {
            spans.reverse();
        }
        output.runs.reserve(spans.len());
        for span in spans {
            if let Some(run) = shape_span(fonts, text, span, bidi_run.level, style) {
                output.width += run.advance;
                output.runs.push(run);
            }
        }
    }
    output
}

fn font_spans(
    fonts: &mut FontStore,
    text: &str,
    range: Range<usize>,
    query: &FontQuery,
    candidates: &[FFontId],
) -> SmallVec<[FontSpan; 2]> {
    let mut spans = SmallVec::new();
    let Some(slice) = text.get(range.clone()) else {
        return spans;
    };
    // The first grapheme with a strong script is the one holding the first
    // character with one.
    let mut current_script = strong_script(slice).unwrap_or(Script::Common);
    for (offset, grapheme) in slice.grapheme_indices(true) {
        if let Some(script) = strong_script(grapheme) {
            current_script = script;
        }
        let start = range.start + offset;
        let end = start + grapheme.len();
        let Some((font_id, fallback)) =
            fonts.font_for_grapheme(query, candidates, grapheme, current_script)
        else {
            continue;
        };
        if let Some(last) = spans.last_mut()
            && last.font_id == font_id
            && last.fallback == fallback
            && last.script == current_script
            && last.range.end == start
        {
            last.range.end = end;
        } else {
            spans.push(FontSpan {
                range: start..end,
                font_id,
                fallback,
                script: current_script,
            });
        }
    }
    spans
}

fn shape_span(
    fonts: &mut FontStore,
    text: &str,
    span: FontSpan,
    bidi_level: u8,
    style: &ComputedTextStyle,
) -> Option<ShapedRun> {
    let metrics = fonts.face(span.font_id)?.metrics;
    let slice = text.get(span.range.clone())?;
    let mut buffer = fonts.take_buffer();
    buffer.push_str(slice);
    buffer.set_direction(if bidi_level.is_multiple_of(2) {
        Direction::LeftToRight
    } else {
        Direction::RightToLeft
    });
    let tag = harfrust::Tag::new(&span.script.as_iso15924_tag().to_be_bytes());
    if let Some(script) = harfrust::Script::from_iso15924_tag(tag) {
        buffer.set_script(script);
    }
    buffer.guess_segment_properties();
    let (shaped, units_per_em) = {
        let ShapeResources { font, data, plans } = fonts.shape_resources(span.font_id)?;
        let shaper = data.shaper(&font).build();
        let plan = shape_plan(
            plans,
            &shaper,
            buffer.direction(),
            buffer.script(),
            buffer.language(),
        );
        (
            shaper.shape_with_plan(plan, buffer, &[]),
            shaper.units_per_em(),
        )
    };
    let infos = shaped.glyph_infos();
    let positions = shaped.glyph_positions();
    let scale = style.font_size.max(0.0) / units_per_em.max(1) as f32;
    let mut boundaries = fonts.take_boundaries();
    boundaries.extend(
        infos
            .iter()
            .map(|info| info.cluster as usize)
            .chain(std::iter::once(slice.len())),
    );
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut glyphs = Vec::with_capacity(infos.len());
    // One cluster per glyph unless glyphs share one, so this rarely grows.
    let mut clusters = Vec::with_capacity(infos.len());
    let mut cursor = 0;
    let mut advance = 0.0;
    while cursor < infos.len() {
        let cluster_start = infos[cursor].cluster as usize;
        let group_start = cursor;
        while cursor < infos.len() && infos[cursor].cluster as usize == cluster_start {
            cursor += 1;
        }
        let group_end = cursor;
        let cluster_end = boundaries
            .get(boundaries.partition_point(|boundary| *boundary <= cluster_start))
            .copied()
            .unwrap_or(slice.len());
        let cluster_text = slice.get(cluster_start..cluster_end).unwrap_or("");
        let whitespace = cluster_text.chars().all(char::is_whitespace);
        let tab = cluster_text == "\t";
        let invisible = cluster_text.chars().all(char::is_control);
        let glyph_start = glyphs.len();
        let cluster_advance_start = advance;

        for index in group_start..group_end {
            let info = infos[index];
            let position = positions[index];
            let mut flags = GlyphFlags::empty();
            if whitespace {
                flags |= GlyphFlags::WHITESPACE;
            }
            if tab {
                flags |= GlyphFlags::TAB;
            }
            if invisible {
                flags |= GlyphFlags::INVISIBLE;
            }
            if span.fallback {
                flags |= GlyphFlags::FALLBACK_FONT;
            }
            if info.glyph_id == 0 {
                flags |= GlyphFlags::MISSING;
            }
            if cluster_text.chars().count() > 1 && group_end - group_start == 1 {
                flags |= GlyphFlags::LIGATURE;
            }
            let glyph_advance = position.x_advance as f32 * scale;
            glyphs.push(ShapedGlyph {
                glyph_id: info.glyph_id,
                offset: Point::new(
                    advance + position.x_offset as f32 * scale,
                    -(position.y_offset as f32 * scale),
                ),
                advance: glyph_advance,
                flags,
            });
            advance += glyph_advance;
        }
        if let Some(last) = glyphs.last_mut() {
            last.advance += style.letter_spacing;
            advance += style.letter_spacing;
            if tab {
                let tab_advance = style.font_size.max(1.0) * 2.0;
                let shaped_advance = advance - cluster_advance_start;
                last.advance += tab_advance - shaped_advance;
                advance += tab_advance - shaped_advance;
            }
        }
        clusters.push(ShapedCluster {
            text_range: (span.range.start + cluster_start)..(span.range.start + cluster_end),
            glyph_range: glyph_start..glyphs.len(),
            advance: (advance - cluster_advance_start).max(0.0),
            whitespace,
        });
    }

    fonts.return_buffer(shaped.clear());
    fonts.return_boundaries(boundaries);
    Some(ShapedRun {
        text_range: span.range,
        font_id: span.font_id,
        font_size: style.font_size,
        bidi_level,
        metrics,
        glyphs,
        clusters,
        advance,
    })
}

fn strong_script(text: &str) -> Option<Script> {
    text.chars()
        .map(|character| character.script())
        .find(|script| !matches!(script, Script::Common | Script::Inherited | Script::Unknown))
}
